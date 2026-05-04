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

#[test]
fn past_config_returns_calibrated_display_per_curve() {
    use spectral_forge::editor::curve_config::curve_display_config;
    use spectral_forge::dsp::modules::{ModuleType, GainMode};

    // The plan originally specified `std::ptr::eq(cfg.offset_fn as *const (),
    // helper as *const ())`, but that comparison is unreliable for `#[inline]
    // pub fn` helpers across crate boundaries — the test crate gets its own
    // monomorphised copy with ThinLTO, so the pointer compare always fails
    // even when routing is correct. We instead verify the routing by behaviour:
    // each helper has a unique fingerprint over a few probe inputs, so
    // matching all probes uniquely identifies which helper was wired.
    //
    // Probe table (see helper definitions in src/editor/curve_config.rs):
    //   helper          (0.5,-0.3)  (0.5,+0.3)  (2.0,+0.5)
    //   off_mix          0.2         0.5         2.0
    //   off_amount_norm  0.2         0.8         1.0   (clamped)
    //   off_thresh      -0.1         0.8         2.5
    //   off_identity     0.5         0.5         2.0
    let approx = |a: f32, b: f32| (a - b).abs() < 1e-5;
    let is_mix = |f: fn(f32, f32) -> f32| {
        approx(f(0.5, -0.3), 0.2) && approx(f(0.5, 0.3), 0.5) && approx(f(2.0, 0.5), 2.0)
    };
    let is_amount_norm = |f: fn(f32, f32) -> f32| {
        approx(f(0.5, -0.3), 0.2) && approx(f(0.5, 0.3), 0.8) && approx(f(2.0, 0.5), 1.0)
    };
    let is_thresh = |f: fn(f32, f32) -> f32| {
        approx(f(0.5, -0.3), -0.1) && approx(f(0.5, 0.3), 0.8) && approx(f(2.0, 0.5), 2.5)
    };
    let is_identity = |f: fn(f32, f32) -> f32| {
        approx(f(0.5, -0.3), 0.5) && approx(f(0.5, 0.3), 0.5) && approx(f(2.0, 0.5), 2.0)
    };

    // AMOUNT (curve 0) — % units, neutral at 100, off_mix
    let amount = curve_display_config(ModuleType::Past, 0, GainMode::Add);
    assert_eq!(amount.y_label, "%");
    assert_eq!(amount.y_min, 0.0);
    assert_eq!(amount.y_max, 100.0);
    assert!((amount.y_natural - 100.0).abs() < 1e-6);
    assert!(is_mix(amount.offset_fn), "AMOUNT should route to off_mix");

    // TIME (curve 1) — seconds, neutral at 0.0, off_amount_norm
    let time = curve_display_config(ModuleType::Past, 1, GainMode::Add);
    assert_eq!(time.y_label, "s");
    assert_eq!(time.y_min, 0.0);
    // y_max is set to a placeholder of 1.0 inside curve_display_config and rewritten
    // at paint time using `total_history_seconds` from the live Pipeline. Test the
    // structural identity here.
    assert!(is_amount_norm(time.offset_fn), "TIME should route to off_amount_norm");

    // THRESHOLD (curve 2) — dBFS, neutral -60, off_thresh
    let thresh = curve_display_config(ModuleType::Past, 2, GainMode::Add);
    assert_eq!(thresh.y_label, "dBFS");
    assert_eq!(thresh.y_min, -80.0);
    assert_eq!(thresh.y_max, 0.0);
    assert!((thresh.y_natural - (-60.0)).abs() < 1e-6);
    assert!(is_thresh(thresh.offset_fn), "THRESHOLD should route to off_thresh");

    // SPREAD (Smear in Granular) — % units
    let spread = curve_display_config(ModuleType::Past, 3, GainMode::Add);
    assert_eq!(spread.y_label, "%");
    assert!(is_mix(spread.offset_fn), "SPREAD should route to off_mix");

    // MIX
    let mix = curve_display_config(ModuleType::Past, 4, GainMode::Add);
    assert_eq!(mix.y_label, "%");
    assert!(is_mix(mix.offset_fn), "MIX should route to off_mix");

    // Out-of-range curve_idx falls back to default_config (off_identity)
    let oob = curve_display_config(ModuleType::Past, 99, GainMode::Add);
    assert!(is_identity(oob.offset_fn), "OOB should route to off_identity");
}
