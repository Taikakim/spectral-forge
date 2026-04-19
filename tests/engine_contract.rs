use spectral_forge::dsp::engines::{
    BinParams, EngineSelection, SpectralEngine, create_engine,
};

fn make_contrast_params(n: usize, ratio: f32) -> (Vec<f32>, Vec<f32>, Vec<f32>, Vec<f32>, Vec<f32>, Vec<f32>, Vec<f32>) {
    (
        vec![-20.0f32; n],  // threshold_db (unused by contrast engine)
        vec![ratio;    n],  // ratio — contrast depth (1=no effect, 2=expand, 0=flatten)
        vec![10.0f32;  n],  // attack_ms
        vec![100.0f32; n],  // release_ms
        vec![0.0f32;   n],  // knee_db
        vec![0.0f32;   n],  // makeup_db
        vec![1.0f32;   n],  // mix
    )
}
use num_complex::Complex;

fn make_params(n: usize) -> (Vec<f32>, Vec<f32>, Vec<f32>, Vec<f32>, Vec<f32>, Vec<f32>, Vec<f32>) {
    (
        vec![-20.0f32; n],  // threshold_db
        vec![4.0f32; n],    // ratio
        vec![10.0f32; n],   // attack_ms
        vec![100.0f32; n],  // release_ms
        vec![6.0f32; n],    // knee_db
        vec![0.0f32; n],    // makeup_db
        vec![1.0f32; n],    // mix
    )
}

fn run_engine(engine: &mut Box<dyn SpectralEngine>, bins: &mut Vec<Complex<f32>>) {
    let n = bins.len();
    let (th, ra, at, re, kn, mk, mx) = make_params(n);
    let params = BinParams {
        threshold_db: &th, ratio: &ra, attack_ms: &at,
        release_ms: &re, knee_db: &kn, makeup_db: &mk, mix: &mx,
        sensitivity: 0.0,
        auto_makeup: false,
        smoothing_semitones: 0.0,
    };
    // NaN sentinel: if engine forgets to write suppression_out, the assertion
    // in callers will catch it (NaN >= 0.0 is false).
    let mut suppression = vec![f32::NAN; n];
    engine.process_bins(bins, None, &params, 44100.0, &mut suppression);
    for &s in &suppression {
        assert!(s.is_finite() && s >= 0.0, "suppression must be finite and non-negative, got {s}");
    }
}

#[test]
fn all_zero_bins_stay_zero() {
    let mut engine = create_engine(EngineSelection::SpectralCompressor);
    engine.reset(44100.0, 2048);
    let mut bins = vec![Complex::new(0.0f32, 0.0); 1025];
    run_engine(&mut engine, &mut bins);
    for b in &bins {
        assert!(b.norm() < 1e-6, "zero bins should stay zero");
    }
}

#[test]
fn reset_callable_multiple_times() {
    let mut engine = create_engine(EngineSelection::SpectralCompressor);
    engine.reset(44100.0, 2048);
    engine.reset(48000.0, 4096);
    engine.reset(44100.0, 2048);
    // must not panic
}

#[test]
fn suppression_out_filled() {
    let mut engine = create_engine(EngineSelection::SpectralCompressor);
    engine.reset(44100.0, 2048);
    let n = 1025;
    let mut bins = vec![Complex::new(1.0f32, 0.0); n];
    let mut suppression = vec![-1.0f32; n]; // sentinel
    let (th, ra, at, re, kn, mk, mx) = make_params(n);
    let params = BinParams {
        threshold_db: &th, ratio: &ra, attack_ms: &at,
        release_ms: &re, knee_db: &kn, makeup_db: &mk, mix: &mx,
        sensitivity: 0.0,
        auto_makeup: false,
        smoothing_semitones: 0.0,
    };
    engine.process_bins(&mut bins, None, &params, 44100.0, &mut suppression);
    // All values must be >= 0 (gain reduction magnitude)
    for &s in &suppression {
        assert!(s >= 0.0, "suppression must be non-negative");
    }
}

#[test]
fn sidechain_some_does_not_panic() {
    let mut engine = create_engine(EngineSelection::SpectralCompressor);
    engine.reset(44100.0, 2048);
    let n = 1025;
    let mut bins = vec![Complex::new(0.5f32, 0.0); n];
    let sidechain_mag = vec![0.5f32; n];
    let mut suppression = vec![f32::NAN; n];
    let (th, ra, at, re, kn, mk, mx) = make_params(n);
    let params = BinParams {
        threshold_db: &th, ratio: &ra, attack_ms: &at,
        release_ms: &re, knee_db: &kn, makeup_db: &mk, mix: &mx,
        sensitivity: 0.0,
        auto_makeup: false,
        smoothing_semitones: 0.0,
    };
    engine.process_bins(&mut bins, Some(&sidechain_mag), &params, 44100.0, &mut suppression);
    for &s in &suppression {
        assert!(s.is_finite() && s >= 0.0, "suppression must be finite and non-negative with sidechain");
    }
}

#[test]
fn loud_signal_gets_compressed() {
    let mut engine = create_engine(EngineSelection::SpectralCompressor);
    engine.reset(44100.0, 2048);
    let n = 1025;
    // Raw FFT magnitude: for FFT_SIZE=2048, a 0 dBFS sine → magnitude ≈ FFT_SIZE/4 = 512.
    // Using 256.0 ≈ −6 dBFS in FFT-normalised space (well above the −20 dBFS threshold).
    let input_mag = 256.0f32;

    let threshold = vec![-20.0f32; n];  // -20 dBFS — signal is above threshold
    let ratio     = vec![4.0f32; n];
    let attack    = vec![0.1f32; n];    // very fast attack
    let release   = vec![100.0f32; n];
    let knee      = vec![0.0f32; n];    // hard knee
    let makeup    = vec![0.0f32; n];
    let mix       = vec![1.0f32; n];

    let params = BinParams {
        threshold_db: &threshold, ratio: &ratio,
        attack_ms: &attack, release_ms: &release,
        knee_db: &knee, makeup_db: &makeup, mix: &mix,
        sensitivity: 0.0,
        auto_makeup: false,
        smoothing_semitones: 0.0,
    };
    let mut suppression = vec![0.0f32; n];

    // Run 200 hops to let envelope follower converge
    let mut bins: Vec<Complex<f32>> = vec![Complex::new(input_mag, 0.0); n];
    for _ in 0..200 {
        let mut b = bins.clone();
        engine.process_bins(&mut b, None, &params, 44100.0, &mut suppression);
    }
    // Final measurement
    let mut final_bins = bins.clone();
    engine.process_bins(&mut final_bins, None, &params, 44100.0, &mut suppression);
    let output_mag = final_bins[512].norm();
    assert!(output_mag < input_mag,
        "compression should reduce level: {} >= {}", output_mag, input_mag);
    // Suppression should be positive (gain reduction is happening)
    assert!(suppression[512] > 0.0,
        "suppression should be positive, got {}", suppression[512]);
}

#[test]
fn fx_module_type_dynamics_is_slot_zero() {
    use spectral_forge::params::{FxModuleType, FxChannelTarget, SpectralForgeParams};
    let p = SpectralForgeParams::default();
    let types = p.fx_module_types.lock();
    assert_eq!(types[0], FxModuleType::Dynamics);
    for i in 1..8 {
        assert_eq!(types[i], FxModuleType::Empty, "slot {i} should be Empty by default");
    }
    let targets = p.fx_module_targets.lock();
    assert!(targets.iter().all(|&t| t == FxChannelTarget::All));
    let names = p.fx_module_names.lock();
    assert_eq!(&names[0], "Dynamics");
    assert_eq!(*p.editing_slot.lock(), 0u8);
}

// ── FxMatrix tests ───────────────────────────────────────────────────────────

#[test]
fn fx_matrix_passthrough_preserves_finite() {
    use spectral_forge::dsp::fx_matrix::FxMatrix;
    use spectral_forge::dsp::engines::BinParams;
    use spectral_forge::params::{StereoLink, EffectMode, FxChannelTarget};
    use num_complex::Complex;

    let num_bins = 1025usize;
    let mut fx = FxMatrix::new(44100.0, 2048);

    let mut bins: Vec<Complex<f32>> = (0..num_bins)
        .map(|k| Complex::new((k as f32 * 0.001).sin(), (k as f32 * 0.001).cos()))
        .collect();

    let threshold = vec![-20.0f32; num_bins];
    let ratio     = vec![4.0f32; num_bins];
    let attack    = vec![10.0f32; num_bins];
    let release   = vec![80.0f32; num_bins];
    let knee      = vec![6.0f32; num_bins];
    let makeup    = vec![0.0f32; num_bins];
    let mix       = vec![1.0f32; num_bins];
    let params = BinParams {
        threshold_db: &threshold,
        ratio:        &ratio,
        attack_ms:    &attack,
        release_ms:   &release,
        knee_db:      &knee,
        makeup_db:    &makeup,
        mix:          &mix,
        sensitivity:  0.0,
        auto_makeup:  false,
        smoothing_semitones: 0.0,
    };

    let mut supp_out = vec![0.0f32; num_bins];
    fx.process_hop(
        0,
        StereoLink::Linked,
        &mut bins,
        None,
        &params,
        EffectMode::Bypass,
        FxChannelTarget::All,
        44100.0,
        &mut supp_out,
        num_bins,
    );

    for (k, b) in bins.iter().enumerate() {
        assert!(b.re.is_finite() && b.im.is_finite(), "bin {k} is not finite: {b:?}");
    }
    for (k, &s) in supp_out.iter().enumerate() {
        assert!(s.is_finite() && s >= 0.0, "suppression[{k}] = {s}");
    }
}

// ── SpectralContrast engine tests ─────────────────────────────────────────────

#[test]
fn contrast_bypass_at_ratio_one() {
    // ratio=1.0 → no effect: output magnitudes should be unchanged.
    let mut engine = create_engine(EngineSelection::SpectralContrast);
    engine.reset(44100.0, 2048);
    let n = 1025;
    let input_mag = 128.0f32;
    let (th, ra, at, re, kn, mk, mx) = make_contrast_params(n, 1.0);
    let params = BinParams {
        threshold_db: &th, ratio: &ra, attack_ms: &at,
        release_ms: &re, knee_db: &kn, makeup_db: &mk, mix: &mx,
        sensitivity: 0.0, auto_makeup: false, smoothing_semitones: 4.0,
    };
    let mut suppression = vec![0.0f32; n];
    let mut bins = vec![Complex::new(input_mag, 0.0f32); n];
    // Run many hops so the envelope converges.
    for _ in 0..200 {
        let mut b = bins.clone();
        engine.process_bins(&mut b, None, &params, 44100.0, &mut suppression);
    }
    let mut final_bins = bins.clone();
    engine.process_bins(&mut final_bins, None, &params, 44100.0, &mut suppression);
    // With flat spectrum and ratio=1, all bins should be at input_mag (no contrast).
    for b in &final_bins {
        assert!((b.norm() - input_mag).abs() < 1e-3,
            "ratio=1 should pass through unchanged, got {}", b.norm());
    }
    // Suppression must be finite and non-negative.
    for &s in &suppression {
        assert!(s.is_finite() && s >= 0.0, "suppression contract violated: {s}");
    }
}

#[test]
fn contrast_expands_peaked_spectrum() {
    // Single loud bin surrounded by quieter bins: ratio=2 should boost the loud bin.
    let mut engine = create_engine(EngineSelection::SpectralContrast);
    engine.reset(44100.0, 2048);
    let n = 1025;
    let (th, ra, at, re, kn, mk, mx) = make_contrast_params(n, 2.0);
    let params = BinParams {
        threshold_db: &th, ratio: &ra, attack_ms: &at,
        release_ms: &re, knee_db: &kn, makeup_db: &mk, mix: &mx,
        // smoothing_semitones=0: test the core contrast gain with no frequency averaging.
        // Frequency averaging would dilute a single-bin peak into the surrounding floor,
        // masking whether the contrast gain formula actually boosts the peak.
        sensitivity: 0.0, auto_makeup: false, smoothing_semitones: 0.0,
    };
    let mut suppression = vec![0.0f32; n];
    // Flat spectrum with one prominent peak at bin 512.
    let floor_mag = 16.0f32;
    let peak_mag  = 256.0f32;
    let mut bins = vec![Complex::new(floor_mag, 0.0f32); n];
    bins[512] = Complex::new(peak_mag, 0.0);
    // Converge the envelope follower.
    for _ in 0..300 {
        let mut b = bins.clone();
        engine.process_bins(&mut b, None, &params, 44100.0, &mut suppression);
    }
    let mut final_bins = bins.clone();
    engine.process_bins(&mut final_bins, None, &params, 44100.0, &mut suppression);
    // The peak bin should have been boosted (above input peak).
    assert!(final_bins[512].norm() > peak_mag,
        "contrast should boost the peak bin: {} <= {}", final_bins[512].norm(), peak_mag);
    // Suppression contract: non-negative finite values.
    for &s in &suppression {
        assert!(s.is_finite() && s >= 0.0, "suppression contract violated: {s}");
    }
}
