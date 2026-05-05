//! UI regression: response-curve polyline extends to Nyquist at the displayed
//! sample rate, not a hardcoded upper frequency.

use spectral_forge::editor::curve::{compute_curve_response, gain_to_display, CurveNode};

#[test]
fn curve_response_spans_full_num_bins_at_44_1_khz() {
    // Flat curve: a single unity node. Result is a vec of length num_bins.
    let nodes: [CurveNode; 6] = Default::default();
    let sample_rate = 44_100.0_f32;
    let fft_size = 2048_usize;
    let num_bins = fft_size / 2 + 1;
    let gains = compute_curve_response(&nodes, num_bins, sample_rate, fft_size);
    assert_eq!(gains.len(), num_bins,
        "compute_curve_response must return num_bins samples ({})", num_bins);
    assert!(gains.iter().all(|g| g.is_finite()),
        "all gains must be finite");
}

#[test]
fn gain_to_display_idx9_eq_node_range_spans_full_dbfs_window() {
    // gain ≈ 0.126 corresponds to a -18 dB EQ bell (10^(-18/20)).
    // The display range is -80..0 dBFS; the formula must reach the floor.
    // Pass sentinel values for db_min/db_max; idx=9 owns its own -80..0 dBFS range regardless.

    let g_min = 10f32.powf(-18.0 / 20.0); // 0.1259
    let g_neutral = 1.0f32;
    let g_max = 10f32.powf(18.0 / 20.0);  // 7.943

    let dbfs_min     = gain_to_display(9, g_min,     0.0, 0.0, -60.0, 0.0, 0.0);
    let dbfs_neutral = gain_to_display(9, g_neutral, 0.0, 0.0, -60.0, 0.0, 0.0);
    let dbfs_max     = gain_to_display(9, g_max,     0.0, 0.0, -60.0, 0.0, 0.0);

    // Neutral curve (gain 1.0) → -20 dBFS (the y_natural anchor in freeze_config).
    assert!((dbfs_neutral - (-20.0)).abs() < 1e-3, "expected -20, got {dbfs_neutral}");
    // Bottom EQ node must reach the -80 dBFS floor.
    assert!(dbfs_min <= -79.0, "expected ≤ -79 (close to floor), got {dbfs_min}");
    // Top EQ node must reach the 0 dBFS ceiling.
    assert!(dbfs_max >= -1.0, "expected ≥ -1 (close to ceiling), got {dbfs_max}");
}

#[test]
fn runtime_anchors_substitutes_db_range_for_threshold_idx_0() {
    use spectral_forge::editor::curve::runtime_anchors;
    use spectral_forge::editor::curve_config::{curve_display_config};
    use spectral_forge::dsp::modules::{GainMode, ModuleType};

    let cfg = curve_display_config(ModuleType::Dynamics, 0, GainMode::Add);

    // db_min=-72, db_max=-3 should override cfg.y_min/y_max for display idx 0.
    let (y_min, y_natural, y_max) = runtime_anchors(&cfg, 0, 0.0, -72.0, -3.0, 10.0, 100.0);
    assert!((y_min - -72.0).abs() < 1e-3, "expected -72, got {y_min}");
    assert!((y_max - -3.0).abs() < 1e-3, "expected -3, got {y_max}");
    // y_natural is the config's neutral (-20 dBFS) — not substituted.
    assert!((y_natural - -20.0).abs() < 1e-3, "expected -20, got {y_natural}");

    // Display index 13 still substitutes total_history_seconds, unaffected.
    let past_cfg = curve_display_config(ModuleType::Past, 1, GainMode::Add);
    let (a, b, c) = runtime_anchors(&past_cfg, 13, 4.0, -60.0, 0.0, 10.0, 100.0);
    assert!((c - 4.0).abs() < 1e-3, "history substitution still works, got {c}");
    let _ = (a, b);

    // Other display indices pass through unchanged.
    let phase_cfg = curve_display_config(ModuleType::PhaseSmear, 0, GainMode::Add);
    let (lo, _, hi) = runtime_anchors(&phase_cfg, 7, 0.0, -60.0, 0.0, 10.0, 100.0);
    assert!((lo - 0.0).abs() < 1e-3, "expected lo=0, got {lo}");
    assert!((hi - 200.0).abs() < 1e-3, "expected hi=200, got {hi}");
}

#[test]
fn physical_to_y_uses_cfg_y_log_for_axis_choice() {
    use spectral_forge::editor::curve::physical_to_y;
    use spectral_forge::editor::curve_config::{curve_display_config};
    use spectral_forge::dsp::modules::{GainMode, ModuleType};
    use nih_plug_egui::egui::{Pos2, Rect};

    let rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(100.0, 100.0));

    // Dynamics RATIO (idx 1, log axis 1..20). At y_min the pixel is bottom;
    // at y_max it's top; at the geometric midpoint sqrt(20)≈4.47 it's centre.
    let ratio_cfg = curve_display_config(ModuleType::Dynamics, 1, GainMode::Add);
    let anchors_ratio = (ratio_cfg.y_min, ratio_cfg.y_natural, ratio_cfg.y_max);

    let y_bottom = physical_to_y(1.0, &ratio_cfg, anchors_ratio, rect);
    let y_top    = physical_to_y(20.0, &ratio_cfg, anchors_ratio, rect);
    let y_mid    = physical_to_y(20f32.sqrt(), &ratio_cfg, anchors_ratio, rect);

    assert!((y_bottom - rect.bottom()).abs() < 1e-3);
    assert!((y_top    - rect.top()).abs() < 1e-3);
    assert!((y_mid    - rect.center().y).abs() < 1.0);

    // PAST mix (idx 6, linear axis 0..100). 50 maps to centre.
    let mix_cfg = curve_display_config(ModuleType::Past, 4, GainMode::Add);
    let anchors_mix = (mix_cfg.y_min, mix_cfg.y_natural, mix_cfg.y_max);
    let y_50 = physical_to_y(50.0, &mix_cfg, anchors_mix, rect);
    assert!((y_50 - rect.center().y).abs() < 1.0);
}

#[test]
fn screen_y_to_physical_inverts_physical_to_y_for_log_and_linear() {
    use spectral_forge::editor::curve::{physical_to_y, screen_y_to_physical};
    use spectral_forge::editor::curve_config::{curve_display_config};
    use spectral_forge::dsp::modules::{GainMode, ModuleType};
    use nih_plug_egui::egui::{Pos2, Rect};

    let rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(100.0, 100.0));

    // Log axis (Dynamics ratio).
    let ratio_cfg = curve_display_config(ModuleType::Dynamics, 1, GainMode::Add);
    let anchors_ratio = (ratio_cfg.y_min, ratio_cfg.y_natural, ratio_cfg.y_max);
    for &v in &[1.5_f32, 4.0, 10.0] {
        let y    = physical_to_y(v, &ratio_cfg, anchors_ratio, rect);
        let back = screen_y_to_physical(y, &ratio_cfg, anchors_ratio, rect);
        assert!((back - v).abs() < 0.05, "round-trip {v} → {y} → {back}");
    }

    // Linear axis (Mix %).
    let mix_cfg = curve_display_config(ModuleType::Past, 4, GainMode::Add);
    let anchors_mix = (mix_cfg.y_min, mix_cfg.y_natural, mix_cfg.y_max);
    for &v in &[12.5_f32, 33.0, 78.0] {
        let y    = physical_to_y(v, &mix_cfg, anchors_mix, rect);
        let back = screen_y_to_physical(y, &mix_cfg, anchors_mix, rect);
        assert!((back - v).abs() < 0.5, "round-trip {v} → {y} → {back}");
    }
}

#[test]
fn runtime_anchors_substitutes_attack_ms_for_idx_2() {
    use spectral_forge::editor::curve::runtime_anchors;
    use spectral_forge::editor::curve_config::curve_display_config;
    use spectral_forge::dsp::modules::{ModuleType, GainMode};

    let cfg = curve_display_config(ModuleType::Dynamics, 2, GainMode::Add);
    // attack_ms = 10 should substitute y_natural; release_ms ignored for idx 2.
    let (lo, nat, hi) = runtime_anchors(&cfg, 2, 0.0, -60.0, 0.0, 10.0, 100.0);
    assert!((nat - 10.0).abs() < 1e-3, "y_natural should be attack_ms=10, got {nat}");
    assert!((lo  -  1.0).abs() < 1e-3);
    assert!((hi  - 1024.0).abs() < 1e-3);
}

#[test]
fn runtime_anchors_substitutes_release_ms_for_idx_3() {
    use spectral_forge::editor::curve::runtime_anchors;
    use spectral_forge::editor::curve_config::curve_display_config;
    use spectral_forge::dsp::modules::{ModuleType, GainMode};

    let cfg = curve_display_config(ModuleType::Dynamics, 3, GainMode::Add);
    let (_, nat, _) = runtime_anchors(&cfg, 3, 0.0, -60.0, 0.0, 10.0, 250.0);
    assert!((nat - 250.0).abs() < 1e-3, "y_natural should be release_ms=250, got {nat}");
}

#[test]
fn axis_aware_lerp_log_geometric_midpoint() {
    use spectral_forge::editor::curve::axis_aware_lerp;
    use spectral_forge::editor::curve_config::curve_display_config;
    use spectral_forge::dsp::modules::{ModuleType, GainMode};

    let cfg = curve_display_config(ModuleType::Dynamics, 1, GainMode::Add); // y_log=true, ratio
    let anchors = (1.0_f32, 1.0, 20.0);
    let mid = axis_aware_lerp(&cfg, anchors, 0.5);
    let expected = 20f32.powf(0.5); // ≈ 4.472
    assert!((mid - expected).abs() < 0.01, "geometric mid expected {expected}, got {mid}");
}

#[test]
fn axis_aware_lerp_linear_arithmetic_midpoint() {
    use spectral_forge::editor::curve::axis_aware_lerp;
    use spectral_forge::editor::curve_config::curve_display_config;
    use spectral_forge::dsp::modules::{ModuleType, GainMode};

    let cfg = curve_display_config(ModuleType::Dynamics, 5, GainMode::Add); // y_log=false, mix
    let anchors = (cfg.y_min, cfg.y_natural, cfg.y_max);
    let mid = axis_aware_lerp(&cfg, anchors, 0.5);
    let expected = cfg.y_natural + 0.5 * (cfg.y_max - cfg.y_natural);
    assert!((mid - expected).abs() < 0.01);
}

#[test]
fn axis_aware_lerp_log_negative_half_reaches_y_min() {
    use spectral_forge::editor::curve::axis_aware_lerp;
    use spectral_forge::editor::curve_config::curve_display_config;
    use spectral_forge::dsp::modules::{ModuleType, GainMode};

    // Use Dynamics ATTACK (idx 2, y_log=true). Test with non-trivial y_min/y_nat
    // (not the static cfg defaults where y_min == y_natural).
    let cfg = curve_display_config(ModuleType::Dynamics, 2, GainMode::Add);
    let anchors = (1.0_f32, 50.0, 1024.0); // y_min=1, y_nat=50, y_max=1024

    // v = -1 must reach y_min exactly.
    let at_minus_one = axis_aware_lerp(&cfg, anchors, -1.0);
    assert!((at_minus_one - 1.0).abs() < 1e-3,
        "v=-1 should reach y_min=1.0, got {at_minus_one}");

    // v = 0 must equal y_natural.
    let at_zero = axis_aware_lerp(&cfg, anchors, 0.0);
    assert!((at_zero - 50.0).abs() < 1e-3,
        "v=0 should equal y_nat=50.0, got {at_zero}");

    // v = -0.5 should be the geometric midpoint between y_min and y_nat.
    // sqrt(1 * 50) = 7.071
    let at_minus_half = axis_aware_lerp(&cfg, anchors, -0.5);
    let expected = (1.0_f32 * 50.0).sqrt();
    assert!((at_minus_half - expected).abs() < 0.01,
        "v=-0.5 should be geometric mid {expected}, got {at_minus_half}");
}

#[test]
fn off_thresh_wysiwyg_at_canonical_db_min() {
    use spectral_forge::editor::curve::{gain_to_display, runtime_anchors, axis_aware_lerp};
    use spectral_forge::editor::curve_config::{curve_display_config, off_thresh};
    use spectral_forge::dsp::modules::{ModuleType, GainMode};

    let cfg = curve_display_config(ModuleType::Dynamics, 0, GainMode::Add);
    let anchors = runtime_anchors(&cfg, 0, 0.0, -60.0, 0.0, 10.0, 100.0);
    for &v in &[-1.0_f32, -0.5, 0.0, 0.5, 1.0] {
        let g_off = off_thresh(1.0, v, anchors);
        let display_actual = gain_to_display(0, g_off, 10.0, 100.0, -60.0, 0.0, 0.0);
        let display_expected = axis_aware_lerp(&cfg, anchors, v);
        assert!((display_actual - display_expected).abs() < 0.5,
            "v={v}: expected {display_expected:.2}, got {display_actual:.2}");
    }
}

#[test]
fn off_freeze_thresh_wysiwyg_at_v_minus_half() {
    use spectral_forge::editor::curve::{gain_to_display, runtime_anchors, axis_aware_lerp};
    use spectral_forge::editor::curve_config::{curve_display_config, off_freeze_thresh};
    use spectral_forge::dsp::modules::{ModuleType, GainMode};

    let cfg = curve_display_config(ModuleType::Freeze, 1, GainMode::Add);
    let anchors = runtime_anchors(&cfg, 9, 0.0, -60.0, 0.0, 10.0, 100.0);
    for &v in &[-1.0_f32, -0.5, 0.0, 0.5, 1.0] {
        let g_off = off_freeze_thresh(1.0, v, anchors);
        let display_actual = gain_to_display(9, g_off, 10.0, 100.0, -80.0, 0.0, 0.0);
        let display_expected = axis_aware_lerp(&cfg, anchors, v);
        assert!((display_actual - display_expected).abs() < 0.5,
            "v={v}: expected {display_expected:.2}, got {display_actual:.2}");
    }
}

#[test]
fn off_atk_rel_wysiwyg_at_attack_ms_10() {
    use spectral_forge::editor::curve::{gain_to_display, runtime_anchors, axis_aware_lerp};
    use spectral_forge::editor::curve_config::{curve_display_config, off_atk_rel};
    use spectral_forge::dsp::modules::{ModuleType, GainMode};

    let cfg = curve_display_config(ModuleType::Dynamics, 2, GainMode::Add);
    let attack_ms = 10.0_f32;
    let anchors = runtime_anchors(&cfg, 2, 0.0, -60.0, 0.0, attack_ms, 100.0);
    for &v in &[-1.0_f32, -0.5, 0.0, 0.5, 1.0] {
        let g_off = off_atk_rel(1.0, v, anchors);
        let display_actual = gain_to_display(2, g_off, attack_ms, 100.0, -60.0, 0.0, 0.0);
        let display_expected = axis_aware_lerp(&cfg, anchors, v);
        assert!((display_actual - display_expected).abs() < 0.5,
            "v={v}: expected {display_expected:.3}, got {display_actual:.3}");
    }
}

#[test]
fn off_atk_rel_wysiwyg_at_release_ms_250() {
    use spectral_forge::editor::curve::{gain_to_display, runtime_anchors, axis_aware_lerp};
    use spectral_forge::editor::curve_config::{curve_display_config, off_atk_rel};
    use spectral_forge::dsp::modules::{ModuleType, GainMode};

    let cfg = curve_display_config(ModuleType::Dynamics, 3, GainMode::Add);
    let release_ms = 250.0_f32;
    let anchors = runtime_anchors(&cfg, 3, 0.0, -60.0, 0.0, 10.0, release_ms);
    for &v in &[-1.0_f32, -0.5, 0.0, 0.5, 1.0] {
        let g_off = off_atk_rel(1.0, v, anchors);
        let display_actual = gain_to_display(3, g_off, 10.0, release_ms, -60.0, 0.0, 0.0);
        let display_expected = axis_aware_lerp(&cfg, anchors, v);
        assert!((display_actual - display_expected).abs() < 0.5,
            "v={v}: expected {display_expected:.3}, got {display_actual:.3}");
    }
}
