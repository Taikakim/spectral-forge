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

// Test for the new `off_amount_norm` helper introduced by the Past UX
// overhaul. See docs/superpowers/specs/2026-05-04-past-module-ux-design.md §5.

#[test]
fn off_amount_norm_clamps_and_passes_zero() {
    use spectral_forge::editor::curve_config::off_amount_norm;
    let approx = |a: f32, b: f32| (a - b).abs() < 1e-6;
    // Identity at o=0
    assert_eq!(off_amount_norm(0.5, 0.0),  0.5);
    assert_eq!(off_amount_norm(0.0, 0.0),  0.0);
    assert_eq!(off_amount_norm(1.0, 0.0),  1.0);
    // Linear add (use approx — f32 addition isn't exact)
    assert!(approx(off_amount_norm(0.3, 0.4),  0.7));
    assert!(approx(off_amount_norm(0.5, -0.3), 0.2));
    // Clamps at 0 and 1
    assert_eq!(off_amount_norm(0.5,  0.7), 1.0);
    assert_eq!(off_amount_norm(0.5, -0.7), 0.0);
    // Beyond range still clamps
    assert_eq!(off_amount_norm(2.0,  0.5), 1.0);
    assert_eq!(off_amount_norm(-1.0, 0.0), 0.0);
}

#[test]
fn gain_to_display_index_13_returns_history_relative_seconds() {
    use spectral_forge::editor::curve::gain_to_display;
    // gain * total_history_seconds, clamped to [0, total]
    let v = gain_to_display(13, 0.5, 0.0, 0.0, 0.0, 0.0, /* total */ 4.0);
    assert!((v - 2.0).abs() < 1e-6, "expected 2.0, got {v}");
    let v = gain_to_display(13, 0.0, 0.0, 0.0, 0.0, 0.0, 4.0);
    assert_eq!(v, 0.0);
    let v = gain_to_display(13, 1.0, 0.0, 0.0, 0.0, 0.0, 4.0);
    assert!((v - 4.0).abs() < 1e-6);
    // Clamp above total
    let v = gain_to_display(13, 2.0, 0.0, 0.0, 0.0, 0.0, 4.0);
    assert_eq!(v, 4.0);
    // Clamp below zero
    let v = gain_to_display(13, -1.0, 0.0, 0.0, 0.0, 0.0, 4.0);
    assert_eq!(v, 0.0);
}

#[test]
fn display_curve_idx_routes_past_curves_to_specific_scales() {
    use spectral_forge::editor::curve::display_curve_idx;
    use spectral_forge::dsp::modules::{ModuleType, GainMode};
    // Past has 5 curves; routing per spec §5.
    assert_eq!(display_curve_idx(ModuleType::Past, 0, GainMode::Add),  6,  "AMOUNT → %");
    assert_eq!(display_curve_idx(ModuleType::Past, 1, GainMode::Add),  13, "TIME → seconds-history");
    assert_eq!(display_curve_idx(ModuleType::Past, 2, GainMode::Add),  9,  "THRESHOLD → dBFS");
    assert_eq!(display_curve_idx(ModuleType::Past, 3, GainMode::Add),  6,  "SPREAD/Smear → %");
    assert_eq!(display_curve_idx(ModuleType::Past, 4, GainMode::Add),  6,  "MIX → %");
}
