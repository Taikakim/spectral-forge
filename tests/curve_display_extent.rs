//! UI regression: response-curve polyline extends to Nyquist at the displayed
//! sample rate, not a hardcoded upper frequency.

use spectral_forge::editor::curve::{compute_curve_response, CurveNode};

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
