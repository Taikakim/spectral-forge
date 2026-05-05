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
    let (y_min, y_natural, y_max) = runtime_anchors(&cfg, 0, 0.0, -72.0, -3.0);
    assert!((y_min - -72.0).abs() < 1e-3, "expected -72, got {y_min}");
    assert!((y_max - -3.0).abs() < 1e-3, "expected -3, got {y_max}");
    // y_natural is the config's neutral (-20 dBFS) — not substituted.
    assert!((y_natural - -20.0).abs() < 1e-3, "expected -20, got {y_natural}");

    // Display index 13 still substitutes total_history_seconds, unaffected.
    let past_cfg = curve_display_config(ModuleType::Past, 1, GainMode::Add);
    let (a, b, c) = runtime_anchors(&past_cfg, 13, 4.0, -60.0, 0.0);
    assert!((c - 4.0).abs() < 1e-3, "history substitution still works, got {c}");
    let _ = (a, b);

    // Other display indices pass through unchanged.
    let phase_cfg = curve_display_config(ModuleType::PhaseSmear, 0, GainMode::Add);
    let (lo, _, hi) = runtime_anchors(&phase_cfg, 7, 0.0, -60.0, 0.0);
    assert!((lo - 0.0).abs() < 1e-3, "expected lo=0, got {lo}");
    assert!((hi - 200.0).abs() < 1e-3, "expected hi=200, got {hi}");
}
