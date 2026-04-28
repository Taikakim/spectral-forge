//! Peak-Locked Phase Vocoder kernels.
//!
//! Implements per-bin phase unwrapping (Laroche-Dolson 1999), low-energy
//! bin phase damping (lifted from `repos/pvx` PHASINESS_IMPLEMENTATION_PLAN
//! Phase 1), and peak detection with Voronoi (nearest-peak) skirt assignment.
//!
//! References:
//! - Laroche, J. and Dolson, M. (1997). About this Phasiness Business.
//!   Proc. ICMC 1997.
//! - Laroche, J. and Dolson, M. (1999). Improved Phase Vocoder Time-Scale
//!   Modification of Audio. IEEE Trans. on Speech and Audio Processing 7(3).

use std::f32::consts::PI;

/// Wrap a phase to (-π, π] (the "principal value of arg").
#[inline]
pub fn principal_arg(phi: f32) -> f32 {
    let mut p = phi.rem_euclid(2.0 * PI);
    if p > PI { p -= 2.0 * PI; }
    p
}

/// Compute per-bin unwrapped phase trajectory.
///
/// `curr_phase`, `prev_phase`, `prev_unwrapped`, `out_unwrapped` are slices
/// of length `>= num_bins`. After the call, `out_unwrapped[k]` is the
/// continuous-time-like phase trajectory at bin k. `prev_unwrapped` is
/// updated in-place to `out_unwrapped` for the next hop.
///
/// `fft_size` and `hop_size` define the expected per-hop advance.
pub fn unwrap_phase(
    curr_phase:     &[f32],
    prev_phase:     &[f32],
    prev_unwrapped: &mut [f32],
    out_unwrapped:  &mut [f32],
    fft_size:       usize,
    hop_size:       usize,
    num_bins:       usize,
) {
    let n = fft_size as f32;
    let r = hop_size as f32;
    for k in 0..num_bins {
        let expected_advance = 2.0 * PI * (k as f32) * r / n;
        let observed_delta   = curr_phase[k] - prev_phase[k];
        let deviation        = principal_arg(observed_delta - expected_advance);
        let true_advance     = expected_advance + deviation;
        out_unwrapped[k]     = prev_unwrapped[k] + true_advance;
    }
    // Roll prev_unwrapped forward.
    prev_unwrapped[..num_bins].copy_from_slice(&out_unwrapped[..num_bins]);
}

/// Re-wrap an unwrapped phase array back into (-π, π] for iFFT input.
pub fn rewrap_phase(unwrapped: &[f32], wrapped_out: &mut [f32], num_bins: usize) {
    for k in 0..num_bins {
        wrapped_out[k] = principal_arg(unwrapped[k]);
    }
}
