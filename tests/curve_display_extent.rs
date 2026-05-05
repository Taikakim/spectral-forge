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
