//! Contrast THRESHOLD bypass + mode + scalars regression suite.
use spectral_forge::dsp::engines::spectral_contrast::SpectralContrastEngine;
use spectral_forge::dsp::engines::{BinParams, SpectralEngine};
use num_complex::Complex;

#[test]
fn contrast_threshold_bins_below_floor_bypass() {
    let mut engine = SpectralContrastEngine::new();
    engine.reset(48_000.0, 1024);
    let n = 513;

    // bin[0..256] sit at -60 dBFS, bin[256..] sit at 0 dBFS.
    // THRESHOLD set to -40 dBFS: bins below it should bypass, above should be processed.
    let mut bins: Vec<Complex<f32>> = (0..n)
        .map(|k| if k < 256 { Complex::new(1e-3, 0.0) } else { Complex::new(1.0, 0.0) })
        .collect();
    let original: Vec<Complex<f32>> = bins.clone();
    let threshold: Vec<f32> = vec![-40.0; n];
    let ratio:     Vec<f32> = vec![5.0; n];
    let attack:    Vec<f32> = vec![10.0; n];
    let release:   Vec<f32> = vec![100.0; n];
    let knee:      Vec<f32> = vec![0.0; n];
    let makeup:    Vec<f32> = vec![0.0; n];
    let mix:       Vec<f32> = vec![1.0; n];
    let mut suppression: Vec<f32> = vec![0.0; n];

    let params = BinParams {
        threshold_db: &threshold, ratio: &ratio, attack_ms: &attack, release_ms: &release,
        knee_db: &knee, makeup_db: &makeup, mix: &mix, smoothing_semitones: 1.0,
        sensitivity: 1.0, auto_makeup: false,
        peaks: None, plpv_dynamics_enabled: false,
    };
    engine.process_bins(&mut bins, None, &params, 48_000.0, &mut suppression);

    // Bins below threshold (k < 256) must be untouched.
    for k in 0..256 {
        assert!((bins[k].re - original[k].re).abs() < 1e-6,
            "bin {k}: expected unchanged (below threshold), got {:?}", bins[k]);
    }

    // Above-threshold bins MUST be processed (not just the below-threshold bins
    // bypassed). With ratio=5.0 and mix=1.0, bins at 0 dBFS should differ
    // measurably from input. This catches an inverted-condition regression
    // where bypass would erroneously apply to the wrong half.
    let mut at_least_one_changed = false;
    for k in 256..n {
        if (bins[k].re - original[k].re).abs() > 0.01 {
            at_least_one_changed = true;
            break;
        }
    }
    assert!(at_least_one_changed,
        "above-threshold bins should be processed; if NONE changed, bypass was applied to wrong half");
}

#[test]
fn contrast_temporal_converges_to_unity_on_steady_input() {
    // Temporal mode: each bin compared against its own long-running mean.
    // After enough blocks for the time-constant to converge, current = mean
    // → zero deviation → unity gain. Output magnitudes should match input
    // magnitudes (within tolerance).
    let mut engine = SpectralContrastEngine::new();
    engine.reset(48_000.0, 1024);
    let n = 513;

    let attack:  Vec<f32> = vec![10.0;  n];
    let release: Vec<f32> = vec![100.0; n];
    let knee:    Vec<f32> = vec![0.0;   n];
    let makeup:  Vec<f32> = vec![0.0;   n];
    let mix:     Vec<f32> = vec![1.0;   n];
    let thresh:  Vec<f32> = vec![-200.0; n];  // bypass disabled
    let ratio:   Vec<f32> = vec![5.0;   n];   // strong expand

    let original_input: Vec<Complex<f32>> = (0..n).map(|k| {
        if k == 200 { Complex::new(1.0, 0.0) } else { Complex::new(0.1, 0.0) }
    }).collect();

    let mut suppression: Vec<f32> = vec![0.0; n];
    let mut output_after = original_input.clone();

    for _ in 0..500 {
        let mut bins = original_input.clone();
        let params = BinParams {
            threshold_db: &thresh, ratio: &ratio, attack_ms: &attack, release_ms: &release,
            knee_db: &knee, makeup_db: &makeup, mix: &mix, smoothing_semitones: 1.0,
            sensitivity: 1.0, auto_makeup: false,
            peaks: None, plpv_dynamics_enabled: false,
        };
        engine.process_bins_temporal(&mut bins, &params, 48_000.0, &mut suppression);
        output_after = bins;
    }

    for k in 0..n {
        let in_mag  = original_input[k].norm();
        let out_mag = output_after[k].norm();
        assert!((out_mag - in_mag).abs() < 0.05,
            "bin {k}: in={in_mag:.4} out={out_mag:.4} (Temporal should converge to unity gain)");
    }
}

#[test]
#[cfg(feature = "probe")]
fn contrast_scalars_round_trip_through_fx_matrix() {
    use spectral_forge::dsp::fx_matrix::FxMatrix;
    use spectral_forge::dsp::modules::ModuleType;
    use spectral_forge::dsp::modules::contrast::ContrastScalars;

    let mut fxm = FxMatrix::new(48_000.0, 1024, &[
        ModuleType::Contrast, ModuleType::Empty, ModuleType::Empty,
        ModuleType::Empty,   ModuleType::Empty, ModuleType::Empty,
        ModuleType::Empty,   ModuleType::Empty, ModuleType::Empty,
    ]);

    let custom = ContrastScalars {
        mean_window_st: 6.0,
        tilt_slope_db_per_oct: -3.0,
    };
    let mut arr = [ContrastScalars::safe_default(); 9];
    arr[0] = custom;

    fxm.set_contrast_scalars(&arr);
    let read_back = fxm.test_contrast_scalars(0).expect("slot 0 should hold Contrast");
    assert!((read_back.mean_window_st - 6.0).abs() < 1e-6);
    assert!((read_back.tilt_slope_db_per_oct - (-3.0)).abs() < 1e-6);
}
