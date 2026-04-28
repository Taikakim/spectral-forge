//! Engine-level unit tests for Phase 4.3a — Dynamics PLPV peak-locked ducking.
//!
//! These exercise `SpectralCompressorEngine::process_bins` directly with the
//! new `BinParams.peaks` / `BinParams.plpv_dynamics_enabled` fields. A full
//! Pipeline-level harness is out of scope (Spectral Forge has no public
//! `Pipeline::run_probe` helper today). Engine-level coverage is sufficient
//! because the lock lives inside the engine — the wiring around it is a
//! single `if let` that BinParams already gates.

use num_complex::Complex;
use spectral_forge::dsp::engines::{
    BinParams, EngineSelection, create_engine,
};
use spectral_forge::dsp::modules::PeakInfo;

/// Build a minimal compressor BinParams with neutral curves except threshold/ratio.
/// Returned tuple owns the per-bin Vecs so the borrow stays valid for the whole test.
fn make_params(n: usize, threshold_db: f32, ratio: f32)
    -> (Vec<f32>, Vec<f32>, Vec<f32>, Vec<f32>, Vec<f32>, Vec<f32>, Vec<f32>)
{
    (
        vec![threshold_db; n], // threshold
        vec![ratio;        n], // ratio
        vec![0.1f32;       n], // attack_ms — fast convergence
        vec![100.0f32;     n], // release_ms
        vec![0.0f32;       n], // knee — hard knee
        vec![0.0f32;       n], // makeup
        vec![1.0f32;       n], // mix — fully wet
    )
}

/// PLPV off must be a *complete* off-switch: providing peaks but with the
/// flag clear must produce identical output to providing no peaks at all.
#[test]
fn dynamics_engine_plpv_off_does_not_change_gr() {
    let n = 1025usize;
    let fft_size = 2048usize;
    let sample_rate = 44100.0f32;

    // Input: flat spectrum at -6 dBFS-ish (raw FFT mag ≈ fft_size/8 = 256).
    let input_mag = 256.0f32;
    let baseline_bins = vec![Complex::new(input_mag, 0.0f32); n];

    let (th, ra, at, re, kn, mk, mx) = make_params(n, -20.0, 4.0);

    // Construct a peak set that *would* be locked if PLPV were on.
    let peaks = vec![
        PeakInfo { k: 100, mag: input_mag, low_k: 95,  high_k: 105 },
        PeakInfo { k: 500, mag: input_mag, low_k: 495, high_k: 505 },
    ];

    // Path A: PLPV off, no peaks.
    let mut engine_a = create_engine(EngineSelection::SpectralCompressor);
    engine_a.reset(sample_rate, fft_size);
    let mut bins_a = baseline_bins.clone();
    let mut supp_a = vec![0.0f32; n];
    let params_a = BinParams {
        threshold_db: &th, ratio: &ra, attack_ms: &at, release_ms: &re,
        knee_db: &kn, makeup_db: &mk, mix: &mx,
        sensitivity: 0.0, auto_makeup: false, smoothing_semitones: 0.0,
        peaks: None,
        plpv_dynamics_enabled: false,
    };
    for _ in 0..200 {
        let mut b = baseline_bins.clone();
        engine_a.process_bins(&mut b, None, &params_a, sample_rate, &mut supp_a);
    }
    engine_a.process_bins(&mut bins_a, None, &params_a, sample_rate, &mut supp_a);

    // Path B: PLPV off, peaks provided. Must be byte-identical to A.
    let mut engine_b = create_engine(EngineSelection::SpectralCompressor);
    engine_b.reset(sample_rate, fft_size);
    let mut bins_b = baseline_bins.clone();
    let mut supp_b = vec![0.0f32; n];
    let params_b = BinParams {
        threshold_db: &th, ratio: &ra, attack_ms: &at, release_ms: &re,
        knee_db: &kn, makeup_db: &mk, mix: &mx,
        sensitivity: 0.0, auto_makeup: false, smoothing_semitones: 0.0,
        peaks: Some(&peaks),
        plpv_dynamics_enabled: false, // OFF
    };
    for _ in 0..200 {
        let mut b = baseline_bins.clone();
        engine_b.process_bins(&mut b, None, &params_b, sample_rate, &mut supp_b);
    }
    engine_b.process_bins(&mut bins_b, None, &params_b, sample_rate, &mut supp_b);

    // Bit-equivalent state evolution → bit-equivalent output.
    // Strict equality, not approx-eq: PLPV-off must be deterministically identical
    // to a stock run, not merely close.
    for k in 0..n {
        assert_eq!(
            bins_a[k].re, bins_b[k].re,
            "bin[{k}].re differs with PLPV off: {} vs {}", bins_a[k].re, bins_b[k].re
        );
        assert_eq!(
            bins_a[k].im, bins_b[k].im,
            "bin[{k}].im differs with PLPV off: {} vs {}", bins_a[k].im, bins_b[k].im
        );
        assert_eq!(supp_a[k], supp_b[k],
            "suppression[{k}] differs with PLPV off: {} vs {}", supp_a[k], supp_b[k]);
    }
}

/// PLPV on must apply the peak bin's gain reduction to every bin in its
/// Voronoi skirt, even when those skirt bins would have ducked less (or not
/// at all) under per-bin compression.
#[test]
fn dynamics_engine_plpv_on_locks_skirt_to_peak() {
    let n = 1025usize;
    let fft_size = 2048usize;
    let sample_rate = 44100.0f32;

    // Spectrum: a loud peak at k=100 (raw mag 256.0 ≈ -6 dBFS) surrounded by
    // a quiet floor (raw mag 4.0 ≈ -42 dBFS). Threshold = -20 dBFS so only
    // the peak triggers GR; the skirt sits well below threshold and would be
    // untouched by per-bin compression.
    let peak_mag  = 256.0f32;
    let floor_mag = 4.0f32;
    let mut input_bins = vec![Complex::new(floor_mag, 0.0f32); n];
    input_bins[100] = Complex::new(peak_mag, 0.0);

    let (th, ra, at, re, kn, mk, mx) = make_params(n, -20.0, 4.0);

    let peaks = vec![
        PeakInfo { k: 100, mag: peak_mag, low_k: 95, high_k: 105 },
    ];

    let mut engine = create_engine(EngineSelection::SpectralCompressor);
    engine.reset(sample_rate, fft_size);
    let mut bins = input_bins.clone();
    let mut supp = vec![0.0f32; n];
    let params = BinParams {
        threshold_db: &th, ratio: &ra, attack_ms: &at, release_ms: &re,
        knee_db: &kn, makeup_db: &mk, mix: &mx,
        sensitivity: 0.0, auto_makeup: false, smoothing_semitones: 0.0,
        peaks: Some(&peaks),
        plpv_dynamics_enabled: true,
    };
    // Converge envelope follower. Each hop sees the same input.
    for _ in 0..400 {
        let mut b = input_bins.clone();
        engine.process_bins(&mut b, None, &params, sample_rate, &mut supp);
    }
    engine.process_bins(&mut bins, None, &params, sample_rate, &mut supp);

    // suppression_out[k] = -smooth_buf[k].max(0.0). After Pass 2.5 the skirt
    // shares smooth_buf[100], so suppression_out[k] == suppression_out[100]
    // for every k ∈ [low_k, high_k].
    let peak_supp = supp[100];
    assert!(peak_supp > 0.5,
        "expected non-trivial GR at the peak bin, got {peak_supp} dB");
    for k in 95..=105 {
        let diff = (supp[k] - peak_supp).abs();
        assert!(diff < 1e-4,
            "skirt bin {k} suppression {} should equal peak suppression {} (diff {})",
            supp[k], peak_supp, diff);
    }

    // Bins outside the skirt must NOT match the peak's GR — the lock is local
    // to the skirt. Pick a bin well away from the peak/skirt and check its
    // suppression is much smaller (the floor never trips the threshold).
    let far_supp = supp[800];
    assert!(far_supp < peak_supp * 0.1,
        "out-of-skirt bin should not be locked to peak GR: far={far_supp}, peak={peak_supp}");
}
