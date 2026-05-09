//! PhaseSmear PHASE_RANGE curve calibration + DSP plumbing.
use spectral_forge::editor::curve_config::curve_display_config;
use spectral_forge::dsp::modules::{ModuleType, GainMode};

#[test]
fn phase_smear_phase_range_curve_config_present() {
    let cfg = curve_display_config(ModuleType::PhaseSmear, 3, GainMode::Add);
    assert_eq!(cfg.y_label, "× π", "PHASE_RANGE should show as multiples of pi");
    assert!((cfg.y_min - 0.0).abs() < 1e-6);
    assert!((cfg.y_max - 2.0).abs() < 1e-6);
    assert!((cfg.y_natural - 1.0).abs() < 1e-6);
}
