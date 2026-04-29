//! Instantaneous-frequency helpers for the phase-vocoder pipeline.
//!
//! Provides `principal_argument` (a re-export of the canonical implementation
//! in [`crate::dsp::plpv`]) and `compute_instantaneous_freq`, which converts
//! two consecutive frames of wrapped per-bin phase into per-bin frequency in Hz
//! using the standard phase-vocoder deviation formula.

// Re-export the canonical (-π, π] wrap from plpv under the plan's public name.
// Option A: alias — zero divergence from the existing impl.
pub use crate::dsp::plpv::principal_arg as principal_argument;

use std::f32::consts::TAU;

/// Compute per-bin instantaneous frequency in Hz from two consecutive frames of
/// wrapped phase using the standard phase-vocoder formula:
///
/// ```text
/// IF[k] = k × bin_freq_hz
///       + principal_argument(curr[k] − prev[k] − expected_phase_inc[k])
///       × sample_rate / (hop_size × 2π)
/// ```
///
/// `if_out[0]` is always set to `0.0` (DC). All slices must be at least
/// `fft_size / 2 + 1` elements long. No allocation is performed.
#[inline]
pub fn compute_instantaneous_freq(
    prev:        &[f32],
    curr:        &[f32],
    if_out:      &mut [f32],
    sample_rate: f32,
    hop_size:    usize,
    fft_size:    usize,
) {
    let num_bins         = fft_size / 2 + 1;
    debug_assert!(prev.len()   >= num_bins);
    debug_assert!(curr.len()   >= num_bins);
    debug_assert!(if_out.len() >= num_bins);

    let bin_freq_hz       = sample_rate / fft_size as f32;
    let hop_per_fft       = hop_size as f32 / fft_size as f32;
    let radians_to_hz_div = sample_rate / (hop_size as f32 * TAU);

    // DC bin: instantaneous frequency is always 0 Hz.
    if_out[0] = 0.0;

    for k in 1..num_bins {
        let expected_phase_inc = TAU * k as f32 * hop_per_fft;
        let raw_delta          = curr[k] - prev[k] - expected_phase_inc;
        let dev                = principal_argument(raw_delta);
        if_out[k] = k as f32 * bin_freq_hz + dev * radians_to_hz_div;
    }
}
