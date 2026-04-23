// UI parameter contract: see docs/superpowers/specs/2026-04-23-ui-parameter-spec-design.md
use spectral_forge::editor::curve_config::curve_display_config;
use spectral_forge::dsp::modules::{ModuleType, GainMode, module_spec};

#[test]
fn all_module_curves_return_valid_config() {
    let types = [
        ModuleType::Dynamics, ModuleType::Freeze, ModuleType::PhaseSmear,
        ModuleType::Contrast, ModuleType::Gain, ModuleType::MidSide,
        ModuleType::TransientSustainedSplit,
    ];
    for ty in types {
        let spec = module_spec(ty);
        for i in 0..spec.num_curves {
            let cfg = curve_display_config(ty, i, GainMode::Add);
            assert!(
                cfg.y_max > cfg.y_min,
                "{:?} curve {}: y_max ({}) must exceed y_min ({})",
                ty, i, cfg.y_max, cfg.y_min
            );
            assert_eq!(
                cfg.grid_lines.len(), 4,
                "{:?} curve {}: expected 4 grid lines, got {}",
                ty, i, cfg.grid_lines.len()
            );
        }
    }
}

#[test]
fn dynamics_threshold_is_linear_dBFS() {
    let cfg = curve_display_config(ModuleType::Dynamics, 0, GainMode::Add);
    assert_eq!(cfg.y_label, "dBFS");
    assert_eq!(cfg.y_min, -60.0);
    assert_eq!(cfg.y_max, 0.0);
    assert!(!cfg.y_log);
}

#[test]
fn dynamics_ratio_is_log() {
    let cfg = curve_display_config(ModuleType::Dynamics, 1, GainMode::Add);
    assert_eq!(cfg.y_label, "ratio");
    assert!(cfg.y_log);
    assert_eq!(cfg.y_min, 1.0);
    assert_eq!(cfg.y_max, 20.0);
}

#[test]
fn gain_add_vs_pull_differ() {
    let add  = curve_display_config(ModuleType::Gain, 0, GainMode::Add);
    let pull = curve_display_config(ModuleType::Gain, 0, GainMode::Pull);
    assert_eq!(add.y_label,  "dB");
    assert_eq!(pull.y_label, "%");
}
