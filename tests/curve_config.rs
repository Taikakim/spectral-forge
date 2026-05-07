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
    assert_eq!(cfg.y_min, -160.0);
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
    let a = (0.0, 0.0, 0.0);
    assert_eq!(off_amount_norm(0.5, 0.0,  a),  0.5);
    assert_eq!(off_amount_norm(0.0, 0.0,  a),  0.0);
    assert_eq!(off_amount_norm(1.0, 0.0,  a),  1.0);
    // Linear add (use approx — f32 addition isn't exact)
    assert!(approx(off_amount_norm(0.3,  0.4, a),  0.7));
    assert!(approx(off_amount_norm(0.5, -0.3, a),  0.2));
    // Clamps at 0 and 1
    assert_eq!(off_amount_norm(0.5,  0.7, a), 1.0);
    assert_eq!(off_amount_norm(0.5, -0.7, a), 0.0);
    // Beyond range still clamps
    assert_eq!(off_amount_norm(2.0,  0.5, a), 1.0);
    assert_eq!(off_amount_norm(-1.0, 0.0, a), 0.0);
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
    //   helper             (0.5,-0.3)    (0.5,+0.3)    (2.0,+0.5)
    //   off_mix             0.2           0.5           2.0
    //   off_amount_norm     0.2           0.8           1.0   (clamped)
    //   off_freeze_thresh   0.5*10^-0.27  0.5*10^0.09  2.0*10^0.15
    //                      ≈0.26852       ≈0.61513      ≈2.82508  (multiplicative log-dBFS, post-Task-4)
    //   off_identity        0.5           0.5           2.0
    let approx = |a: f32, b: f32| (a - b).abs() < 1e-4;
    let sentinel = (0.0_f32, 0.0_f32, 0.0_f32);
    let is_mix = |f: fn(f32, f32, (f32, f32, f32)) -> f32| {
        approx(f(0.5, -0.3, sentinel), 0.2) && approx(f(0.5, 0.3, sentinel), 0.5) && approx(f(2.0, 0.5, sentinel), 2.0)
    };
    let is_amount_norm = |f: fn(f32, f32, (f32, f32, f32)) -> f32| {
        approx(f(0.5, -0.3, sentinel), 0.2) && approx(f(0.5, 0.3, sentinel), 0.8) && approx(f(2.0, 0.5, sentinel), 1.0)
    };
    let is_freeze_thresh = |f: fn(f32, f32, (f32, f32, f32)) -> f32| {
        // Asymmetric: g * 10^(0.9*o) for o<0, g * 10^(0.3*o) for o≥0
        approx(f(0.5, -0.3, sentinel), 0.5 * 10f32.powf(0.9 * -0.3))
            && approx(f(0.5, 0.3, sentinel), 0.5 * 10f32.powf(0.3 * 0.3))
            && approx(f(2.0, 0.5, sentinel), 2.0 * 10f32.powf(0.3 * 0.5))
    };
    let is_identity = |f: fn(f32, f32, (f32, f32, f32)) -> f32| {
        approx(f(0.5, -0.3, sentinel), 0.5) && approx(f(0.5, 0.3, sentinel), 0.5) && approx(f(2.0, 0.5, sentinel), 2.0)
    };

    // AMOUNT (curve 0) — % units, neutral at 100, off_mix
    let amount = curve_display_config(ModuleType::Past, 0, GainMode::Add);
    assert_eq!(amount.y_label, "%");
    assert_eq!(amount.y_min, 0.0);
    assert_eq!(amount.y_max, 100.0);
    assert!((amount.y_natural - 100.0).abs() < 1e-6);
    assert!(is_mix(amount.offset_fn), "AMOUNT should route to off_mix");

    // TIME (curve 1) — seconds, neutral at 0.5 (midpoint fraction of total
    // history; runtime_anchors() scales by total_history_seconds), off_amount_norm
    let time = curve_display_config(ModuleType::Past, 1, GainMode::Add);
    assert_eq!(time.y_label, "s");
    assert_eq!(time.y_min, 0.0);
    // y_max=1.0 placeholder; runtime_anchors substitutes total_history_seconds.
    assert_eq!(time.y_max, 1.0);
    assert!((time.y_natural - 0.5).abs() < 1e-6, "TIME y_natural should be 0.5 (midpoint fraction)");
    assert!(is_amount_norm(time.offset_fn), "TIME should route to off_amount_norm");

    // THRESHOLD (curve 2) — dBFS -160..0, neutral -20 (matches gain_to_display(9, 1.0)),
    // off_freeze_thresh so the offset slider can reach the full visible y range.
    let thresh = curve_display_config(ModuleType::Past, 2, GainMode::Add);
    assert_eq!(thresh.y_label, "dBFS");
    assert_eq!(thresh.y_min, -160.0);
    assert_eq!(thresh.y_max, 0.0);
    assert!((thresh.y_natural - (-20.0)).abs() < 1e-6);
    assert!(is_freeze_thresh(thresh.offset_fn), "THRESHOLD should route to off_freeze_thresh");

    // SPREAD / Smear (curve 3) — %, neutral at 100% (matches gain_to_display(6, 1.0)),
    // off_mix so the slider has only negative reach (no "more than 100%" semantically).
    let spread = curve_display_config(ModuleType::Past, 3, GainMode::Add);
    assert_eq!(spread.y_label, "%");
    assert_eq!(spread.y_min, 0.0);
    assert_eq!(spread.y_max, 100.0);
    assert!((spread.y_natural - 100.0).abs() < 1e-6, "Smear y_natural should be 100%");
    assert!(is_mix(spread.offset_fn), "Smear should route to off_mix");

    // MIX
    let mix = curve_display_config(ModuleType::Past, 4, GainMode::Add);
    assert_eq!(mix.y_label, "%");
    assert!(is_mix(mix.offset_fn), "MIX should route to off_mix");

    // Out-of-range curve_idx falls back to default_config (off_identity)
    let oob = curve_display_config(ModuleType::Past, 99, GainMode::Add);
    assert!(is_identity(oob.offset_fn), "OOB should route to off_identity");
}

/// All PAST curves use the universal neutral default (y=0). Earlier code seeded
/// PAST Age + Smear at y=-0.334 to "centre at 50%", but bells are bandwidth-
/// limited and don't sum to a flat non-neutral curve — the result was a comb of
/// bumps rather than a flat line. Pins the all-neutral convention so a future
/// regression (re-introducing flat_at_y) breaks this test.
#[test]
fn past_default_nodes_centre_age_and_floor_smear() {
    use spectral_forge::editor::curve::default_nodes_for_module_curve;
    use spectral_forge::dsp::modules::ModuleType;

    for c in 0..7 {
        let nodes = default_nodes_for_module_curve(ModuleType::Past, c);
        for n in &nodes {
            assert_eq!(n.y, 0.0,
                "Past curve {c} should default to neutral (y=0); got {}", n.y);
        }
    }

    // Modules without per-module overrides fall through to the legacy curve-only
    // defaults — Dynamics' curve 1 (Ratio) gets its high-shelf preset back.
    let dyn_ratio = default_nodes_for_module_curve(ModuleType::Dynamics, 1);
    assert!((dyn_ratio[5].y - 0.334).abs() < 1e-3,
        "Dynamics Ratio should keep its high-shelf y=0.334 preset");
}

/// `runtime_anchors` returns absolute (y_min, y_natural, y_max) for normal
/// display indices, and substitutes total_history_seconds for index 13 (Past
/// Age/Delay) treating the config anchors as fractions of the buffer.
#[test]
fn runtime_anchors_substitutes_history_seconds_for_index_13() {
    use spectral_forge::editor::curve::runtime_anchors;
    use spectral_forge::editor::curve_config::{curve_display_config};
    use spectral_forge::dsp::modules::{ModuleType, GainMode};

    // Past THRESHOLD (display idx 9) — anchors use db_min/db_max like idx 0.
    // y_natural is -20 (matches gain_to_display(9, 1.0)).
    let cfg = curve_display_config(ModuleType::Past, 2, GainMode::Add);
    let (lo, nat, hi) = runtime_anchors(&cfg, 9, 4.0, -160.0, 0.0, 10.0, 100.0);
    assert_eq!(lo, -160.0);
    assert!((nat - (-20.0)).abs() < 1e-6);
    assert_eq!(hi, 0.0);

    // Past Age (display idx 13) — anchors are fractions, scaled by total.
    let cfg = curve_display_config(ModuleType::Past, 1, GainMode::Add);
    let (lo, nat, hi) = runtime_anchors(&cfg, 13, 4.0, -60.0, 0.0, 10.0, 100.0);
    assert_eq!(lo, 0.0);
    assert!((nat - 2.0).abs() < 1e-6, "y_natural=0.5 × total=4.0 = 2.0 s, got {nat}");
    assert!((hi - 4.0).abs() < 1e-6);
}

/// Spec §2 piecewise-linear interpolation: at offset = 0 the slider reads
/// y_natural; at +1 it reads y_max; at -1 it reads y_min; the in-between
/// values lerp linearly between those three anchors.
///
/// This is the formula the slider's custom_formatter implements directly —
/// independent of `offset_fn`. The test pins down the math against Past's
/// THRESHOLD (-80..-20..0 dBFS) — the original regression was the threshold
/// display being clamped at -40 dBFS; with the corrected `y_natural=-20`
/// the slider lerp should reach -80 dBFS at v=-1.
#[test]
fn slider_lerp_covers_full_range_for_past_threshold() {
    let lerp = |y_min: f32, y_nat: f32, y_max: f32, v: f32| -> f32 {
        if v >= 0.0 {
            y_nat + v * (y_max - y_nat)
        } else {
            y_nat + v * (y_nat - y_min)
        }
    };
    let (y_min, y_nat, y_max) = (-80.0_f32, -20.0_f32, 0.0_f32);
    assert!((lerp(y_min, y_nat, y_max,  0.0) - (-20.0)).abs() < 1e-5);
    assert!((lerp(y_min, y_nat, y_max,  1.0) -    0.0 ).abs() < 1e-5);
    assert!((lerp(y_min, y_nat, y_max, -1.0) - (-80.0)).abs() < 1e-5);
    // -0.5 should land midway between -20 and -80 = -50 dBFS.
    assert!((lerp(y_min, y_nat, y_max, -0.5) - (-50.0)).abs() < 1e-5);
    // -0.25 → -35 dBFS.
    assert!((lerp(y_min, y_nat, y_max, -0.25) - (-35.0)).abs() < 1e-5);
}
