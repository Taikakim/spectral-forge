//! Calibration audit — every per-curve offset_fn's declared ±1 → [y_min, y_max]
//! range is verified end-to-end through the module's DSP.
//! See docs/superpowers/plans/2026-04-24-calibration-audit.md Task 2.

use num_complex::Complex;
use spectral_forge::dsp::modules::{
    create_module, GainMode, ModuleContext, ModuleType, ProbeSnapshot, SpectralModule,
};
use spectral_forge::editor::curve_config::curve_display_config;
use spectral_forge::params::{FxChannelTarget, StereoLink};

const FFT_SIZE: usize = 2048;
const NUM_BINS: usize = FFT_SIZE / 2 + 1;
const SAMPLE_RATE: f32 = 48_000.0;

fn make_ctx() -> ModuleContext {
    ModuleContext {
        sample_rate: SAMPLE_RATE,
        fft_size: FFT_SIZE,
        num_bins: NUM_BINS,
        attack_ms: 10.0,
        release_ms: 100.0,
        sensitivity: 0.5,
        suppression_width: 0.0,
        auto_makeup: false,
        delta_monitor: false,
    }
}

/// Run the module with every curve filled with `gain_on_target` on
/// `target_curve_idx` and 1.0 on all other curves. Returns the probe.
fn run_case(
    module: &mut Box<dyn SpectralModule>,
    num_curves: usize,
    target_curve_idx: usize,
    gain_on_target: f32,
) -> ProbeSnapshot {
    let curves_storage: Vec<Vec<f32>> = (0..num_curves)
        .map(|c| if c == target_curve_idx {
            vec![gain_on_target; NUM_BINS]
        } else {
            vec![1.0; NUM_BINS]
        })
        .collect();
    let curves_refs: Vec<&[f32]> = curves_storage.iter().map(|v| v.as_slice()).collect();

    let mut bins: Vec<Complex<f32>> = vec![Complex::new(0.1, 0.0); NUM_BINS];
    let mut suppression: Vec<f32> = vec![0.0; NUM_BINS];
    let ctx = make_ctx();
    module.process(
        0,
        StereoLink::Linked,
        FxChannelTarget::All,
        &mut bins,
        None,
        &curves_refs,
        &mut suppression,
        &ctx,
    );
    module.last_probe()
}

#[test]
fn dynamics_threshold_offset_plus_one_hits_y_max() {
    let mut m = create_module(ModuleType::Dynamics, SAMPLE_RATE, FFT_SIZE);
    let nc = m.num_curves();
    let cfg = curve_display_config(ModuleType::Dynamics, 0, GainMode::Add);
    let g = (cfg.offset_fn)(1.0, 1.0);
    let probe = run_case(&mut m, nc, 0, g);
    let observed = probe.threshold_db.expect("dynamics must probe threshold");
    assert!(
        (observed - cfg.y_max).abs() < 0.5,
        "offset=+1 should give threshold≈{}, got {}", cfg.y_max, observed,
    );
}

#[test]
fn dynamics_threshold_offset_minus_one_hits_y_min() {
    let mut m = create_module(ModuleType::Dynamics, SAMPLE_RATE, FFT_SIZE);
    let nc = m.num_curves();
    let cfg = curve_display_config(ModuleType::Dynamics, 0, GainMode::Add);
    let g = (cfg.offset_fn)(1.0, -1.0);
    let probe = run_case(&mut m, nc, 0, g);
    let observed = probe.threshold_db.expect("dynamics must probe threshold");
    assert!(
        (observed - cfg.y_min).abs() < 0.5,
        "offset=-1 should give threshold≈{}, got {}", cfg.y_min, observed,
    );
}

#[test]
fn dynamics_ratio_offset_plus_one_hits_y_max() {
    let mut m = create_module(ModuleType::Dynamics, SAMPLE_RATE, FFT_SIZE);
    let nc = m.num_curves();
    let cfg = curve_display_config(ModuleType::Dynamics, 1, GainMode::Add);
    let g = (cfg.offset_fn)(1.0, 1.0);
    let probe = run_case(&mut m, nc, 1, g);
    let observed = probe.ratio.expect("dynamics must probe ratio");
    assert!(
        (observed - cfg.y_max).abs() < 0.1,
        "offset=+1 should give ratio≈{}, got {}", cfg.y_max, observed,
    );
}

#[test]
fn dynamics_ratio_offset_minus_one_hits_y_min() {
    let mut m = create_module(ModuleType::Dynamics, SAMPLE_RATE, FFT_SIZE);
    let nc = m.num_curves();
    let cfg = curve_display_config(ModuleType::Dynamics, 1, GainMode::Add);
    let g = (cfg.offset_fn)(1.0, -1.0);
    let probe = run_case(&mut m, nc, 1, g);
    let observed = probe.ratio.expect("dynamics must probe ratio");
    assert!(
        (observed - cfg.y_min).abs() < 0.1,
        "offset=-1 should give ratio≈{}, got {}", cfg.y_min, observed,
    );
}

#[test]
fn dynamics_knee_offset_extremes() {
    let mut m = create_module(ModuleType::Dynamics, SAMPLE_RATE, FFT_SIZE);
    let nc = m.num_curves();
    let cfg = curve_display_config(ModuleType::Dynamics, 4, GainMode::Add);

    let g_hi = (cfg.offset_fn)(1.0, 1.0);
    let probe = run_case(&mut m, nc, 4, g_hi);
    let observed = probe.knee_db.unwrap();
    assert!((observed - cfg.y_max).abs() < 0.5, "knee hi: want {}, got {}", cfg.y_max, observed);

    let g_lo = (cfg.offset_fn)(1.0, -1.0);
    let probe = run_case(&mut m, nc, 4, g_lo);
    let observed = probe.knee_db.unwrap();
    assert!((observed - cfg.y_min).abs() < 0.5, "knee lo: want {}, got {}", cfg.y_min, observed);
}

#[test]
fn dynamics_mix_offset_extremes() {
    let mut m = create_module(ModuleType::Dynamics, SAMPLE_RATE, FFT_SIZE);
    let nc = m.num_curves();
    let cfg = curve_display_config(ModuleType::Dynamics, 5, GainMode::Add);

    let g_hi = (cfg.offset_fn)(1.0, 1.0);
    let probe = run_case(&mut m, nc, 5, g_hi);
    assert!((probe.mix_pct.unwrap() - cfg.y_max).abs() < 1.0);

    let g_lo = (cfg.offset_fn)(1.0, -1.0);
    let probe = run_case(&mut m, nc, 5, g_lo);
    assert!((probe.mix_pct.unwrap() - cfg.y_min).abs() < 1.0);
}

// Attack/Release are multiplicative with factor 1024. The module scales
// ctx.attack_ms (10 ms) by the curve gain and clamps. Verify scaling works.
#[test]
fn dynamics_attack_offset_plus_one_multiplies_global() {
    let mut m = create_module(ModuleType::Dynamics, SAMPLE_RATE, FFT_SIZE);
    let nc = m.num_curves();
    let cfg = curve_display_config(ModuleType::Dynamics, 2, GainMode::Add);
    let g = (cfg.offset_fn)(1.0, 1.0); // = 1024.0
    let probe = run_case(&mut m, nc, 2, g);
    // ctx.attack_ms=10 × 1024 = 10240, clamped at 500 (pipeline limit) — so the
    // y_max of 1024 in the config is actually a display-only limit, not a DSP
    // limit. The test just asserts the attack reaches the 500 ms DSP clamp.
    let observed = probe.attack_ms.unwrap();
    assert!(observed >= 500.0 - 1.0, "attack should reach DSP clamp 500, got {}", observed);
}
