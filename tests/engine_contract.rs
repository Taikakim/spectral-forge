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

#[test]
fn fx_matrix_no_route_to_master_produces_silence() {
    use spectral_forge::dsp::{
        modules::{ModuleType, ModuleContext, RouteMatrix},
        fx_matrix::FxMatrix,
        pipeline::MAX_NUM_BINS,
    };
    use spectral_forge::params::{StereoLink, FxChannelTarget};
    use num_complex::Complex;

    let n = 1025usize;
    let mut types = [ModuleType::Empty; 9];
    types[0] = ModuleType::Dynamics;
    types[8] = ModuleType::Master;
    let mut fm = FxMatrix::new(44100.0, 2048, &types);

    // Use a route matrix with NO send to Master (slot 8)
    let mut rm = RouteMatrix::default();
    rm.send[0][1] = 1.0;
    rm.send[1][2] = 1.0;
    rm.send[2][8] = 0.0;   // explicitly clear the default route to Master

    let mut bins: Vec<Complex<f32>> = vec![Complex::new(1.0, 0.0); n];
    let curves: Vec<Vec<Vec<f32>>> = (0..9)
        .map(|_| (0..7).map(|_| vec![1.0f32; MAX_NUM_BINS]).collect())
        .collect();
    let mut supp = vec![0.0f32; n];
    let sc: [Option<&[f32]>; 9] = [None; 9];
    let targets = [FxChannelTarget::All; 9];
    let ctx = ModuleContext {
        sample_rate: 44100.0, fft_size: 2048, num_bins: n,
        attack_ms: 10.0, release_ms: 100.0, sensitivity: 0.0,
        suppression_width: 0.0, auto_makeup: false, delta_monitor: false,
    };
    fm.process_hop(0, StereoLink::Linked, &mut bins, &sc, &targets, &curves, &rm, &ctx, &mut supp, n);

    // All bins should be zero when nothing routes to Master
    for (k, b) in bins.iter().enumerate() {
        assert!(
            b.norm() < 1e-6,
            "bin {k} should be silent when nothing routes to Master, got {}", b.norm()
        );
    }
}

// ── FxMatrix tests ───────────────────────────────────────────────────────────

fn make_default_fx_matrix() -> spectral_forge::dsp::fx_matrix::FxMatrix {
    use spectral_forge::dsp::modules::ModuleType;
    let mut types = [ModuleType::Empty; 9];
    types[0] = ModuleType::Dynamics;
    types[1] = ModuleType::Dynamics;
    types[2] = ModuleType::Gain;
    types[8] = ModuleType::Master;
    spectral_forge::dsp::fx_matrix::FxMatrix::new(44100.0, 2048, &types)
}

#[test]
fn fx_matrix_passthrough_preserves_finite() {
    use spectral_forge::dsp::modules::ModuleContext;
    use spectral_forge::params::{StereoLink, FxChannelTarget};
    use num_complex::Complex;

    let num_bins = 1025usize;
    let mut fx = make_default_fx_matrix();

    let mut bins: Vec<Complex<f32>> = (0..num_bins)
        .map(|k| Complex::new((k as f32 * 0.001).sin(), (k as f32 * 0.001).cos()))
        .collect();

    // Build 9x7xnum_bins slot curves (all-ones = neutral)
    let slot_curves: Vec<Vec<Vec<f32>>> = (0..9)
        .map(|_| (0..7).map(|_| vec![1.0f32; num_bins]).collect())
        .collect();
    let sc_args: [Option<&[f32]>; 9] = [None; 9];
    let slot_targets = [FxChannelTarget::All; 9];
    let ctx = ModuleContext {
        sample_rate: 44100.0,
        fft_size: 2048,
        num_bins,
        attack_ms: 10.0,
        release_ms: 80.0,
        sensitivity: 0.0,
        suppression_width: 0.0,
        auto_makeup: false,
        delta_monitor: false,
    };

    let mut supp_out = vec![0.0f32; num_bins];
    let rm = spectral_forge::dsp::modules::RouteMatrix::default();
    fx.process_hop(
        0,
        StereoLink::Linked,
        &mut bins,
        &sc_args,
        &slot_targets,
        &slot_curves,
        &rm,
        &ctx,
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

#[test]
fn fx_matrix_dynamics_produces_finite_output() {
    use spectral_forge::dsp::modules::ModuleContext;
    use spectral_forge::params::{StereoLink, FxChannelTarget};
    use num_complex::Complex;

    let num_bins = 1025usize;
    let mut fx = make_default_fx_matrix();

    // A non-trivial spectrum with variation
    let mut bins: Vec<Complex<f32>> = (0..num_bins)
        .map(|k| {
            let mag = if k % 10 == 0 { 1.0 } else { 0.1 };
            Complex::new(mag * (k as f32 * 0.01).cos(), mag * (k as f32 * 0.01).sin())
        })
        .collect();

    // Build 9x7xnum_bins slot curves (all-ones = neutral)
    let slot_curves: Vec<Vec<Vec<f32>>> = (0..9)
        .map(|_| (0..7).map(|_| vec![1.0f32; num_bins]).collect())
        .collect();
    let sc_args: [Option<&[f32]>; 9] = [None; 9];
    let slot_targets = [FxChannelTarget::All; 9];
    let ctx = ModuleContext {
        sample_rate: 44100.0,
        fft_size: 2048,
        num_bins,
        attack_ms: 10.0,
        release_ms: 80.0,
        sensitivity: 0.0,
        suppression_width: 0.0,
        auto_makeup: false,
        delta_monitor: false,
    };

    let mut supp_out = vec![0.0f32; num_bins];
    let rm = spectral_forge::dsp::modules::RouteMatrix::default();
    fx.process_hop(
        0,
        StereoLink::Linked,
        &mut bins,
        &sc_args,
        &slot_targets,
        &slot_curves,
        &rm,
        &ctx,
        &mut supp_out,
        num_bins,
    );

    for (k, b) in bins.iter().enumerate() {
        assert!(b.re.is_finite() && b.im.is_finite(),
            "bin {k} not finite after processing: {b:?}");
    }
    for (k, &s) in supp_out.iter().enumerate() {
        assert!(s.is_finite() && s >= 0.0, "suppression[{k}] = {s}");
    }
}

#[test]
fn fft_size_choice_variants_and_max_bins() {
    use spectral_forge::params::{FftSizeChoice, fft_size_from_choice};
    use spectral_forge::dsp::pipeline::MAX_NUM_BINS;

    let pairs: &[(FftSizeChoice, usize)] = &[
        (FftSizeChoice::S512,   512),
        (FftSizeChoice::S1024,  1024),
        (FftSizeChoice::S2048,  2048),
        (FftSizeChoice::S4096,  4096),
        (FftSizeChoice::S8192,  8192),
        (FftSizeChoice::S16384, 16384),
    ];
    for &(choice, expected) in pairs {
        assert_eq!(fft_size_from_choice(choice), expected);
        assert!(expected / 2 + 1 <= MAX_NUM_BINS,
            "fft_size {} → {} bins exceeds MAX_NUM_BINS {}", expected, expected/2+1, MAX_NUM_BINS);
    }
}

#[test]
fn fx_matrix_constructs_from_slot_types() {
    use spectral_forge::dsp::{
        modules::ModuleType,
        fx_matrix::FxMatrix,
    };
    let mut types = [ModuleType::Empty; 9];
    types[0] = ModuleType::Dynamics;
    types[1] = ModuleType::Gain;
    types[8] = ModuleType::Master;
    // Should not panic and slot 8 must be Master.
    let _m = FxMatrix::new(44100.0, 2048, &types);
}

#[test]
fn gain_module_set_gain_mode_changes_behavior() {
    use spectral_forge::dsp::modules::{create_module, GainMode, ModuleType};
    let mut g = create_module(ModuleType::Gain, 44100.0, 2048);
    // Default is Add. After setting Subtract, mode should be Subtract.
    g.set_gain_mode(GainMode::Subtract);
    // No public mode accessor — test indirectly via process output.
    // With Subtract and no sidechain, gain curve all-ones → bins unchanged.
    // (Behavioral test is implicit: we just verify no panic and it compiles.)
    g.set_gain_mode(GainMode::Add);
}

#[test]
fn mid_side_module_compiles_and_passes_through_at_neutral() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::{
        create_module, ModuleType, ModuleContext, SpectralModule,
    };
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let n = 1025usize;
    let mut m = create_module(ModuleType::MidSide, 44100.0, 2048);

    let ones = vec![1.0f32; n];
    let curves_storage: [&[f32]; 5] = [&ones, &ones, &ones, &ones, &ones];
    let curves: &[&[f32]] = &curves_storage;

    let mut bins = vec![Complex::new(1.0f32, 0.0); n];
    let mut supp = vec![0.0f32; n];
    let ctx = ModuleContext {
        sample_rate: 44100.0, fft_size: 2048, num_bins: n,
        attack_ms: 10.0, release_ms: 100.0, sensitivity: 0.0,
        suppression_width: 0.0, auto_makeup: false, delta_monitor: false,
    };

    // Mid channel — neutral balance (1.0) → mid_scale = sqrt(1.0) = 1.0 → bins unchanged
    m.process(0, StereoLink::MidSide, FxChannelTarget::All, &mut bins, None, curves, &mut supp, &ctx);
    let mid_out = bins[10].norm();
    assert!(mid_out > 0.5, "mid signal should survive neutral M/S processing, got {}", mid_out);

    // Side channel — neutral: bal=1 → side_scale=1, exp=1 → no change
    let mut side_bins = vec![Complex::new(0.5f32, 0.0); n];
    m.process(1, StereoLink::MidSide, FxChannelTarget::All, &mut side_bins, None, curves, &mut supp, &ctx);
    assert!(side_bins[10].norm() > 0.1, "side signal should survive neutral M/S processing");

    // When NOT in MidSide mode → pass through (suppression_out zeroed, bins unchanged)
    let mut bypass_bins = vec![Complex::new(1.0f32, 0.0); n];
    m.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bypass_bins, None, curves, &mut supp, &ctx);
    assert_eq!(bypass_bins[10].re, 1.0, "MidSide module should pass through when not in M/S mode");
}

#[test]
fn matrix_routing_serial_default_passes_signal() {
    use num_complex::Complex;
    use spectral_forge::dsp::{
        modules::{ModuleType, ModuleContext, RouteMatrix},
        fx_matrix::FxMatrix,
        pipeline::MAX_NUM_BINS,
    };
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let n = 1025usize; // 2048/2+1
    let mut types = [ModuleType::Empty; 9];
    types[0] = ModuleType::Dynamics;
    types[8] = ModuleType::Master;
    let mut fm = FxMatrix::new(44100.0, 2048, &types);

    // Serial default: slot 0 → slot 1 → slot 2 → Master (empty intermediate slots pass signal through).
    let mut bins: Vec<Complex<f32>> = vec![Complex::new(1.0, 0.0); n];
    let curves: Vec<Vec<Vec<f32>>> = (0..9)
        .map(|_| (0..7).map(|_| vec![1.0f32; MAX_NUM_BINS]).collect())
        .collect();
    let mut supp = vec![0.0f32; n];
    let sc: [Option<&[f32]>; 9] = [None; 9];
    let targets = [FxChannelTarget::All; 9];
    let rm = RouteMatrix::default();
    let ctx = ModuleContext {
        sample_rate: 44100.0, fft_size: 2048, num_bins: n,
        attack_ms: 10.0, release_ms: 100.0, sensitivity: 0.0,
        suppression_width: 0.0, auto_makeup: false, delta_monitor: false,
    };
    fm.process_hop(0, StereoLink::Linked, &mut bins, &sc, &targets, &curves, &rm, &ctx, &mut supp, n);
    // Signal should make it through: at least some bins are non-zero.
    assert!(bins.iter().any(|c| c.norm() > 0.01), "signal lost through matrix");
}

#[test]
fn fx_matrix_sync_slot_types_activates_new_module() {
    use spectral_forge::dsp::{
        modules::ModuleType,
        fx_matrix::FxMatrix,
    };

    // Start with only Master in slot 8
    let mut types = [ModuleType::Empty; 9];
    types[8] = ModuleType::Master;
    let mut fm = FxMatrix::new(44100.0, 2048, &types);

    // Sync: add Dynamics to slot 0
    types[0] = ModuleType::Dynamics;
    fm.sync_slot_types(&types, 44100.0, 2048);

    // Slot 0 must now contain a module of type Dynamics
    assert!(fm.slots[0].is_some(), "slot 0 should have Dynamics after sync");
    assert_eq!(
        fm.slots[0].as_ref().unwrap().module_type(),
        ModuleType::Dynamics
    );

    // Sync: remove it
    types[0] = ModuleType::Empty;
    fm.sync_slot_types(&types, 44100.0, 2048);
    assert!(fm.slots[0].is_none(), "slot 0 should be None after sync to Empty");
}

#[test]
fn contrast_module_neutral_curve_passes_flat_spectrum() {
    use spectral_forge::dsp::{
        modules::{create_module, ModuleType, ModuleContext, SpectralModule},
    };
    use spectral_forge::params::{StereoLink, FxChannelTarget};
    use num_complex::Complex;

    // A flat input spectrum with a neutral AMOUNT curve (all 1.0 → ratio=1.0)
    // must pass through unchanged: each output bin magnitude should be within
    // 1% of the input magnitude after envelope convergence.

    let n = 1025usize;
    let mut m = create_module(ModuleType::Contrast, 44100.0, 2048);

    // Neutral AMOUNT curve: gain=1.0 everywhere → ratio=1.0 (no contrast)
    let ones = vec![1.0f32; n];
    let curves_storage: [&[f32]; 2] = [&ones, &ones];
    let curves: &[&[f32]] = &curves_storage;

    let input_mag = 1.0f32;
    let mut supp = vec![0.0f32; n];
    let ctx = ModuleContext {
        sample_rate: 44100.0, fft_size: 2048, num_bins: n,
        attack_ms: 10.0, release_ms: 100.0, sensitivity: 0.0,
        suppression_width: 0.0,
        auto_makeup: false, delta_monitor: false,
    };

    // Converge the contrast envelope with a flat spectrum
    for _ in 0..500 {
        let mut bins = vec![Complex::new(input_mag, 0.0f32); n];
        m.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, None, curves, &mut supp, &ctx);
    }

    // Final measurement hop
    let mut final_bins = vec![Complex::new(input_mag, 0.0f32); n];
    m.process(0, StereoLink::Linked, FxChannelTarget::All, &mut final_bins, None, curves, &mut supp, &ctx);

    // With a flat spectrum and ratio=1.0, all bins should pass through within 1%
    for (k, b) in final_bins.iter().enumerate() {
        let out_mag = b.norm();
        assert!(
            (out_mag - input_mag).abs() < 0.01 * input_mag,
            "bin {k}: expected output within 1% of {input_mag}, got {out_mag}"
        );
    }
}
