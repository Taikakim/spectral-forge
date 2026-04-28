//! Engine-level unit tests for Phase 4.3a — Dynamics PLPV peak-locked ducking
//! and Phase 4.3b — PhaseSmear PLPV unwrapped-phase randomization wiring.
//!
//! 4.3a tests exercise `SpectralCompressorEngine::process_bins` directly with the
//! new `BinParams.peaks` / `BinParams.plpv_dynamics_enabled` fields.
//!
//! 4.3b tests exercise `PhaseSmearModule::process` directly to prove the routing
//! decision: PLPV-on writes `ctx.unwrapped_phase` only (Pipeline rewrap stage
//! propagates to bins); PLPV-off writes `bins[k]` only (legacy complex-space mix).
//! End-to-end audible correctness is verified at Phase 4.9.

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

// ── Phase 4.3b — PhaseSmear PLPV routing wiring ───────────────────────────────
//
// Two engine-level tests prove the PLPV-on vs PLPV-off branching writes to the
// correct destination. The Pipeline-level smoothness test from the plan (boundary
// RMS measurement) is deferred to Phase 4.9.

use std::cell::Cell;
use spectral_forge::dsp::modules::{ModuleContext, FreezeModule, PhaseSmearModule, SpectralModule};
use spectral_forge::params::{FxChannelTarget, StereoLink};

/// Build the curves PhaseSmear consumes: amount, peak hold, mix.
/// `amount = 1.0 → ±π scale`, `mix = 1.0 → fully wet`.
fn phase_smear_curves(num_bins: usize) -> (Vec<f32>, Vec<f32>, Vec<f32>) {
    (
        vec![1.0f32; num_bins],   // AMOUNT — full ±π scale
        vec![1.0f32; num_bins],   // PEAK HOLD — neutral
        vec![1.0f32; num_bins],   // MIX — fully wet
    )
}

/// Synthesize an unwrapped-phase trajectory for tests: `[k * 0.1; n]`.
/// Returned as a `Vec<Cell<f32>>` so the slice form `&[Cell<f32>]` matches the
/// `ModuleContext.unwrapped_phase` field type.
fn make_unwrapped(num_bins: usize) -> Vec<Cell<f32>> {
    (0..num_bins).map(|k| Cell::new(k as f32 * 0.1)).collect()
}

#[test]
fn phase_smear_plpv_off_writes_to_bins_only() {
    let num_bins = 1025usize;
    let mut m = PhaseSmearModule::new();
    m.reset(48000.0, 2048);
    m.set_plpv_phase_smear_enabled(false);

    // Bins: a non-trivial spectrum so a phase change is detectable. Real-only
    // initial bins → arg() == 0 for k != Nyquist; any phase write shows up as
    // a non-zero imaginary component.
    let initial_bins: Vec<Complex<f32>> = (0..num_bins)
        .map(|_| Complex::new(1.0, 0.0)).collect();
    let mut bins = initial_bins.clone();

    // Unwrapped-phase trajectory exposed through ctx — but since PLPV is OFF,
    // the module must not touch it.
    let unwrapped = make_unwrapped(num_bins);
    let initial_unwrapped: Vec<f32> = unwrapped.iter().map(|c| c.get()).collect();

    let (am, pk, mx) = phase_smear_curves(num_bins);
    let curves_vec: Vec<&[f32]> = vec![&am, &pk, &mx];

    let mut supp = vec![0.0f32; num_bins];

    let mut ctx = ModuleContext::new(
        48000.0, 2048, num_bins,
        10.0, 80.0, 0.5, 0.0, false, false,
    );
    ctx.unwrapped_phase = Some(&unwrapped);

    m.process(0, StereoLink::Linked, FxChannelTarget::All,
              &mut bins, None, &curves_vec, &mut supp, None, &ctx);

    // PLPV OFF → unwrapped[] must be byte-identical to initial values.
    for k in 0..num_bins {
        assert_eq!(unwrapped[k].get(), initial_unwrapped[k],
            "PLPV-off must not touch unwrapped[{k}]: got {} expected {}",
            unwrapped[k].get(), initial_unwrapped[k]);
    }

    // PLPV OFF → bins must show some phase change (non-zero imaginary part on
    // at least some bin, since the input was real and amount=1.0). The DC and
    // Nyquist bins are skipped, so check the interior.
    let any_changed = (1..num_bins - 1).any(|k| (bins[k].im - initial_bins[k].im).abs() > 1e-6);
    assert!(any_changed, "PLPV-off must apply random phase to bins[]");
}

#[test]
fn phase_smear_plpv_on_writes_to_unwrapped_only() {
    let num_bins = 1025usize;
    let mut m = PhaseSmearModule::new();
    m.reset(48000.0, 2048);
    m.set_plpv_phase_smear_enabled(true);

    let initial_bins: Vec<Complex<f32>> = (0..num_bins)
        .map(|_| Complex::new(1.0, 0.0)).collect();
    let mut bins = initial_bins.clone();

    let unwrapped = make_unwrapped(num_bins);
    let initial_unwrapped: Vec<f32> = unwrapped.iter().map(|c| c.get()).collect();

    let (am, pk, mx) = phase_smear_curves(num_bins);
    let curves_vec: Vec<&[f32]> = vec![&am, &pk, &mx];

    let mut supp = vec![0.0f32; num_bins];

    let mut ctx = ModuleContext::new(
        48000.0, 2048, num_bins,
        10.0, 80.0, 0.5, 0.0, false, false,
    );
    ctx.unwrapped_phase = Some(&unwrapped);

    m.process(0, StereoLink::Linked, FxChannelTarget::All,
              &mut bins, None, &curves_vec, &mut supp, None, &ctx);

    // PLPV ON → bins must be byte-identical to the input. The Pipeline's rewrap
    // stage (not run in this test) is what produces the audible effect; the
    // module itself must NOT write bins on this path.
    for k in 0..num_bins {
        assert_eq!(bins[k].re, initial_bins[k].re,
            "PLPV-on must not write bins[{k}].re: got {} expected {}",
            bins[k].re, initial_bins[k].re);
        assert_eq!(bins[k].im, initial_bins[k].im,
            "PLPV-on must not write bins[{k}].im: got {} expected {}",
            bins[k].im, initial_bins[k].im);
    }

    // PLPV ON → at least one interior unwrapped[] entry must have changed
    // (DC at k=0 and Nyquist at k=num_bins-1 are skipped; everywhere else
    // sees a non-zero random phase × mix).
    let any_changed = (1..num_bins - 1)
        .any(|k| (unwrapped[k].get() - initial_unwrapped[k]).abs() > 1e-6);
    assert!(any_changed, "PLPV-on must write to unwrapped[]");
}

// ── Phase 4.3c — Freeze PLPV routing wiring ───────────────────────────────────
//
// Two engine-level tests prove the PLPV-on vs PLPV-off branching writes to the
// correct destination. End-to-end audible verification (no zipper across hop
// boundaries, frozen-spectrum stationarity) is deferred to Phase 4.9.

/// Build the curves Freeze consumes: length, threshold, portamento, resistance, mix.
/// Tuned so the freeze path actually fires and mix is fully wet.
fn freeze_curves(num_bins: usize) -> (Vec<f32>, Vec<f32>, Vec<f32>, Vec<f32>, Vec<f32>) {
    (
        vec![1.0f32; num_bins],   // LENGTH      — neutral (≈500 ms)
        vec![1.0f32; num_bins],   // THRESHOLD   — neutral (-20 dBFS)
        vec![1.0f32; num_bins],   // PORTAMENTO  — neutral (≈200 ms)
        vec![1.0f32; num_bins],   // RESISTANCE  — neutral (≈1.0 normalised excess)
        vec![1.0f32; num_bins],   // MIX         — fully wet
    )
}

#[test]
fn freeze_plpv_off_writes_to_bins_only() {
    let num_bins = 1025usize;
    let mut m = FreezeModule::new();
    m.reset(48000.0, 2048);
    m.set_plpv_freeze_enabled(false);

    // Bins: a non-trivial spectrum where dry differs from a frozen snapshot.
    // First-process call captures the initial bins as the frozen state, so on
    // subsequent calls a *different* live spectrum will produce a non-trivial
    // dry/wet mix. We rotate by k between calls to make `dry != frozen`.
    let initial_bins: Vec<Complex<f32>> = (0..num_bins)
        .map(|k| Complex::new(1.0 + (k as f32) * 0.001, 0.0))
        .collect();

    // Unwrapped-phase trajectory exposed through ctx — but since PLPV is OFF,
    // the module must not touch it.
    let unwrapped = make_unwrapped(num_bins);
    let initial_unwrapped: Vec<f32> = unwrapped.iter().map(|c| c.get()).collect();

    let (lg, th, pt, rs, mx) = freeze_curves(num_bins);
    let curves_vec: Vec<&[f32]> = vec![&lg, &th, &pt, &rs, &mx];

    let mut supp = vec![0.0f32; num_bins];

    let mut ctx = ModuleContext::new(
        48000.0, 2048, num_bins,
        10.0, 80.0, 0.5, 0.0, false, false,
    );
    ctx.unwrapped_phase = Some(&unwrapped);

    // First hop: captures initial bins as frozen state. Mix=1.0, so output ==
    // frozen == input — we need at least one more hop with a DIFFERENT input to
    // observe a change. (If we only ran one hop, output would equal input even
    // on the working path, masking any bug.)
    let mut bins = initial_bins.clone();
    m.process(0, StereoLink::Linked, FxChannelTarget::All,
              &mut bins, None, &curves_vec, &mut supp, None, &ctx);

    // Second hop: feed a *different* input. With mix=1.0, output should equal
    // the frozen snapshot (≈ initial_bins), not the new input.
    let live_bins: Vec<Complex<f32>> = (0..num_bins)
        .map(|k| Complex::new(2.0 + (k as f32) * 0.001, 1.0))
        .collect();
    let mut bins = live_bins.clone();
    m.process(0, StereoLink::Linked, FxChannelTarget::All,
              &mut bins, None, &curves_vec, &mut supp, None, &ctx);

    // PLPV OFF → unwrapped[] must be byte-identical to initial values across
    // both hops. (DC and Nyquist are special-case but they are still untouched.)
    for k in 0..num_bins {
        assert_eq!(unwrapped[k].get(), initial_unwrapped[k],
            "PLPV-off must not touch unwrapped[{k}]: got {} expected {}",
            unwrapped[k].get(), initial_unwrapped[k]);
    }

    // PLPV OFF → bins must show a freeze effect: mix=1.0 means the output
    // tracks the frozen snapshot, not the live input. So at least one bin
    // should differ from the live input.
    let any_changed = (0..num_bins)
        .any(|k| (bins[k].re - live_bins[k].re).abs() > 1e-6
              || (bins[k].im - live_bins[k].im).abs() > 1e-6);
    assert!(any_changed,
        "PLPV-off freeze must write a frozen spectrum into bins[] (different from live input)");
}

#[test]
fn freeze_plpv_on_writes_to_unwrapped_only() {
    let num_bins = 1025usize;
    let mut m = FreezeModule::new();
    m.reset(48000.0, 2048);
    m.set_plpv_freeze_enabled(true);

    let initial_bins: Vec<Complex<f32>> = (0..num_bins)
        .map(|k| Complex::new(1.0 + (k as f32) * 0.001, 0.0))
        .collect();

    let unwrapped = make_unwrapped(num_bins);
    let initial_unwrapped: Vec<f32> = unwrapped.iter().map(|c| c.get()).collect();

    let (lg, th, pt, rs, mx) = freeze_curves(num_bins);
    let curves_vec: Vec<&[f32]> = vec![&lg, &th, &pt, &rs, &mx];

    let mut supp = vec![0.0f32; num_bins];

    let mut ctx = ModuleContext::new(
        48000.0, 2048, num_bins,
        10.0, 80.0, 0.5, 0.0, false, false,
    );
    ctx.unwrapped_phase = Some(&unwrapped);

    // First hop: captures the initial frozen state and seeds frozen_unwrapped.
    let mut bins = initial_bins.clone();
    m.process(0, StereoLink::Linked, FxChannelTarget::All,
              &mut bins, None, &curves_vec, &mut supp, None, &ctx);

    // Second hop with a *different* live input. The magnitude path is still
    // active (frozen ≠ live), so bins[] will be modified — the rewrap test
    // below cares only about the PLPV phase write, which lives in `unwrapped`.
    let live_bins: Vec<Complex<f32>> = (0..num_bins)
        .map(|k| Complex::new(2.0 + (k as f32) * 0.001, 1.0))
        .collect();
    let mut bins = live_bins.clone();
    m.process(0, StereoLink::Linked, FxChannelTarget::All,
              &mut bins, None, &curves_vec, &mut supp, None, &ctx);

    // PLPV ON → at least one interior unwrapped[] entry must have changed
    // (DC at k=0 advances by 0 every hop; everywhere else accumulates the
    // 2π·k·hop/N step, then the mix=1.0 lerp replaces dry entirely).
    let any_changed = (1..num_bins - 1)
        .any(|k| (unwrapped[k].get() - initial_unwrapped[k]).abs() > 1e-6);
    assert!(any_changed, "PLPV-on Freeze must write to unwrapped[]");

    // The magnitude path still runs end-to-end. With mix=1.0 and a different
    // live input, bins should differ from the live input (frozen magnitude
    // wins via the complex-space mix below the PLPV write site).
    let any_mag_changed = (0..num_bins)
        .any(|k| (bins[k].norm() - live_bins[k].norm()).abs() > 1e-6);
    assert!(any_mag_changed,
        "PLPV-on Freeze magnitude path must still affect bin magnitudes");
}

// ── Phase 4.3d — MidSide PLPV peak-aligned decode + J probe ──────────────────
//
// Three tests:
//   1. The Laroche-Dolson-style J helper (inter-channel phase drift) is
//      exercised by `midside_plpv_does_not_increase_phase_drift_on_tonal_signal`,
//      which round-trips a synthetic stereo chord through M/S encode → MidSide
//      module → M/S decode and asserts the PLPV-on path produces strictly less
//      drift than the per-bin random PLPV-off path.
//   2. `midside_plpv_off_uses_per_bin_path` verifies the off-path still applies
//      a per-bin random rotation (different output per bin).
//   3. `midside_plpv_on_uses_per_peak_path` verifies the on-path applies the
//      same rotation to every bin in a peak's skirt (skirt bins share an arg).

use spectral_forge::dsp::modules::MidSideModule;

fn inter_channel_phase_drift(
    in_l: &[Complex<f32>], in_r: &[Complex<f32>],
    out_l: &[Complex<f32>], out_r: &[Complex<f32>],
    num_bins: usize,
    noise_floor: f32,
) -> f32 {
    let mut j = 0.0_f32;
    for k in 0..num_bins {
        let dphi_in  = in_l[k].arg()  - in_r[k].arg();
        let dphi_out = out_l[k].arg() - out_r[k].arg();
        if in_l[k].norm() > noise_floor && in_r[k].norm() > noise_floor {
            // Use principal_arg to fold into (-π, π] so that ±2π wraps don't
            // accumulate spurious 2π errors at branch crossings.
            j += spectral_forge::dsp::plpv::principal_arg(dphi_out - dphi_in).abs();
        }
    }
    j
}

/// Run the same M/S encode → MidSide.process(0) + .process(1) → M/S decode
/// pipeline once, with PLPV either on or off, and return the J metric.
/// Each invocation constructs a fresh `MidSideModule` so PRNG state starts
/// from the default seed — required so PLPV-on vs PLPV-off are compared
/// against an apples-to-apples PRNG initial condition.
fn ms_pipeline_j(
    in_l: &[Complex<f32>], in_r: &[Complex<f32>],
    num_bins: usize,
    peaks: &[PeakInfo],
    bal: &[f32], exp: &[f32], dec: &[f32], trans: &[f32], pan: &[f32],
    plpv_on: bool,
) -> f32 {
    use std::f32::consts::FRAC_1_SQRT_2;

    let curves: [&[f32]; 5] = [bal, exp, dec, trans, pan];
    let curves_slice: &[&[f32]] = &curves;

    let mut module = MidSideModule::new();
    module.reset(48000.0, (num_bins - 1) * 2);
    module.set_plpv_midside_enabled(plpv_on);

    // L/R → M/S
    let mut mid: Vec<Complex<f32>> = in_l.iter().zip(in_r.iter())
        .map(|(l, r)| (l + r) * FRAC_1_SQRT_2).collect();
    let mut side: Vec<Complex<f32>> = in_l.iter().zip(in_r.iter())
        .map(|(l, r)| (l - r) * FRAC_1_SQRT_2).collect();

    let mut ctx = ModuleContext::new(
        48000.0, (num_bins - 1) * 2, num_bins,
        10.0, 80.0, 0.5, 0.0, false, false,
    );
    if plpv_on {
        ctx.peaks = Some(peaks);
    }

    let mut supp = vec![0.0_f32; num_bins];

    module.process(0, StereoLink::MidSide, FxChannelTarget::All,
                   &mut mid, None, curves_slice, &mut supp, None, &ctx);
    module.process(1, StereoLink::MidSide, FxChannelTarget::All,
                   &mut side, None, curves_slice, &mut supp, None, &ctx);

    // M/S → L/R
    let out_l: Vec<Complex<f32>> = mid.iter().zip(side.iter())
        .map(|(m, s)| (m + s) * FRAC_1_SQRT_2).collect();
    let out_r: Vec<Complex<f32>> = mid.iter().zip(side.iter())
        .map(|(m, s)| (m - s) * FRAC_1_SQRT_2).collect();

    inter_channel_phase_drift(in_l, in_r, &out_l, &out_r, num_bins, 1e-3)
}

#[test]
fn midside_plpv_does_not_increase_phase_drift_on_tonal_signal() {
    let num_bins = 1025usize;

    // Synthesize stereo input: tonal energy at four bins with correlated but
    // slightly different L/R phases. Bins outside the tonal set sit at a tiny
    // noise floor so the J helper's noise gate excludes them.
    let mut in_l = vec![Complex::new(1e-9, 0.0); num_bins];
    let mut in_r = vec![Complex::new(1e-9, 0.0); num_bins];
    for &k in &[50, 100, 200, 400] {
        in_l[k] = Complex::from_polar(1.0, 0.3 * k as f32);
        in_r[k] = Complex::from_polar(1.0, 0.3 * k as f32 + 0.1);
    }

    // Peaks tile [0, num_bins-1] so every bin lands in some skirt under PLPV.
    let last = (num_bins - 1) as u32;
    let peaks = vec![
        PeakInfo { k: 50,  mag: 1.0, low_k: 0,   high_k: 75 },
        PeakInfo { k: 100, mag: 1.0, low_k: 76,  high_k: 150 },
        PeakInfo { k: 200, mag: 1.0, low_k: 151, high_k: 300 },
        PeakInfo { k: 400, mag: 1.0, low_k: 301, high_k: last },
    ];

    // Curves: balance neutral, expansion neutral, decorrel = 0.5 (substantial).
    let bal   = vec![1.0_f32; num_bins];
    let exp   = vec![1.0_f32; num_bins];
    let dec   = vec![0.5_f32; num_bins];
    let trans = vec![1.0_f32; num_bins];
    let pan   = vec![1.0_f32; num_bins];

    // Two fresh modules → identical PRNG initial seed; difference comes only
    // from the iteration shape (per-peak vs per-bin).
    let j_off = ms_pipeline_j(&in_l, &in_r, num_bins, &peaks,
                              &bal, &exp, &dec, &trans, &pan, false);
    let j_on  = ms_pipeline_j(&in_l, &in_r, num_bins, &peaks,
                              &bal, &exp, &dec, &trans, &pan, true);

    // PLPV-on must reduce phase drift relative to PLPV-off — the per-peak
    // broadcast keeps every bin in a peak's skirt phase-aligned with the
    // peak, whereas PLPV-off scrambles each bin independently.
    println!("MidSide J probe: J_off={} J_on={}", j_off, j_on);
    assert!(j_on < j_off,
        "PLPV-on should reduce inter-channel phase drift; got J_on={} J_off={}",
        j_on, j_off);
}

#[test]
fn midside_plpv_off_uses_per_bin_path() {
    // With PLPV off, the side path advances PRNG per non-real bin and applies
    // an independent rotation to each. Verify by checking that two adjacent
    // interior bins receive different rotations (statistically certain given
    // 1e-6 tolerance and a uniform [-π, π] distribution).
    let num_bins = 1025usize;
    let mut m = MidSideModule::new();
    m.reset(48000.0, 2048);
    m.set_plpv_midside_enabled(false);

    // Real-valued unit input on every bin: input arg is 0 everywhere
    // (excluding DC/Nyquist sentinels). Any non-zero output arg comes from
    // the side path's rotation.
    let mut bins: Vec<Complex<f32>> = (0..num_bins).map(|_| Complex::new(1.0, 0.0)).collect();

    let bal   = vec![1.0_f32; num_bins];
    let exp   = vec![1.0_f32; num_bins];
    let dec   = vec![1.0_f32; num_bins];   // full ±π decorrelation
    let trans = vec![1.0_f32; num_bins];
    let pan   = vec![1.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&bal, &exp, &dec, &trans, &pan];

    let mut supp = vec![0.0_f32; num_bins];
    let ctx = ModuleContext::new(
        48000.0, 2048, num_bins,
        10.0, 80.0, 0.5, 0.0, false, false,
    );
    // ctx.peaks intentionally None — PLPV is off anyway.

    m.process(1, StereoLink::MidSide, FxChannelTarget::All,
              &mut bins, None, &curves, &mut supp, None, &ctx);

    // Pick a stretch of interior bins. Their rotations must differ — at least
    // one pair must show a measurable arg difference.
    let mut any_diff = false;
    for k in 100..200 {
        if (bins[k].arg() - bins[k + 1].arg()).abs() > 1e-3 {
            any_diff = true;
            break;
        }
    }
    assert!(any_diff,
        "PLPV-off side path must rotate each bin independently — all bins shared a phase");
}

#[test]
fn midside_plpv_on_uses_per_peak_path() {
    // With PLPV on, every bin in a peak's skirt receives the SAME random
    // rotation (one PRNG draw per peak). Verify by constructing a peak whose
    // skirt covers a stretch of interior bins, then asserting all bins in the
    // skirt share the same arg (within float epsilon).
    let num_bins = 1025usize;
    let mut m = MidSideModule::new();
    m.reset(48000.0, 2048);
    m.set_plpv_midside_enabled(true);

    let mut bins: Vec<Complex<f32>> = (0..num_bins).map(|_| Complex::new(1.0, 0.0)).collect();

    let bal   = vec![1.0_f32; num_bins];
    let exp   = vec![1.0_f32; num_bins];
    let dec   = vec![1.0_f32; num_bins];
    let trans = vec![1.0_f32; num_bins];
    let pan   = vec![1.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&bal, &exp, &dec, &trans, &pan];

    // Single peak whose skirt is wholly interior (not touching DC/Nyquist).
    let peaks = vec![
        PeakInfo { k: 150, mag: 1.0, low_k: 100, high_k: 200 },
    ];
    let mut ctx = ModuleContext::new(
        48000.0, 2048, num_bins,
        10.0, 80.0, 0.5, 0.0, false, false,
    );
    ctx.peaks = Some(&peaks);

    let mut supp = vec![0.0_f32; num_bins];

    m.process(1, StereoLink::MidSide, FxChannelTarget::All,
              &mut bins, None, &curves, &mut supp, None, &ctx);

    // Every bin in [100..=200] should share the same arg as bin 100 — the
    // peak's per-skirt rotation was drawn ONCE.
    let arg_ref = bins[100].arg();
    for k in 100..=200 {
        let diff = (bins[k].arg() - arg_ref).abs();
        assert!(diff < 1e-4,
            "PLPV-on per-peak path: bin {k} arg {} should equal peak arg {} (diff {})",
            bins[k].arg(), arg_ref, diff);
    }
}
