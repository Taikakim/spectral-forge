use spectral_forge::dsp::engines::{
    BinParams, EngineSelection, SpectralEngine, create_engine,
};
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
    };
    engine.process_bins(&mut bins, Some(&sidechain_mag), &params, 44100.0, &mut suppression);
    for &s in &suppression {
        assert!(s.is_finite() && s >= 0.0, "suppression must be finite and non-negative with sidechain");
    }
}
