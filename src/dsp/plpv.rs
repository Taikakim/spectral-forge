//! Peak-Locked Phase Vocoder kernels.
//!
//! Phase 4.1 ships per-bin phase unwrapping (Laroche-Dolson 1999) and the
//! matching re-wrap. Low-energy phase damping (Phase 4.1.5) and peak
//! detection / Voronoi skirt assignment (Phase 4.2) will be added in
//! follow-on tasks.
//!
//! References:
//! - Laroche, J. and Dolson, M. (1997). About this Phasiness Business.
//!   Proc. ICMC 1997.
//! - Laroche, J. and Dolson, M. (1999). Improved Phase Vocoder Time-Scale
//!   Modification of Audio. IEEE Trans. on Speech and Audio Processing 7(3).

use std::f32::consts::PI;

/// Wrap a phase to (-π, π] (the "principal value of arg").
///
/// NaN-in / NaN-out: `rem_euclid` propagates NaN, the comparison is false,
/// and the original NaN is returned unchanged. ±∞ collapse to NaN via
/// `rem_euclid`. RT-safe (no allocation, no panics for finite input).
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
///
/// Note: phase is meaningful only where the bin has non-trivial magnitude.
/// The damping stage (Phase 4.1.5) silences low-energy bins before this
/// runs in the Pipeline, so callers should not rely on this function's
/// behavior for bins that are physically silent — at bins where
/// `expected_advance ≡ π (mod 2π)` exactly, the half-open `(-π, π]`
/// convention pulls the deviation to `+π` and the accumulator picks up a
/// spurious `2π` per hop.
pub fn unwrap_phase(
    curr_phase:     &[f32],
    prev_phase:     &[f32],
    prev_unwrapped: &mut [f32],
    out_unwrapped:  &mut [f32],
    fft_size:       usize,
    hop_size:       usize,
    num_bins:       usize,
) {
    debug_assert!(curr_phase.len()     >= num_bins);
    debug_assert!(prev_phase.len()     >= num_bins);
    debug_assert!(prev_unwrapped.len() >= num_bins);
    debug_assert!(out_unwrapped.len()  >= num_bins);

    let two_pi_r_over_n = 2.0 * PI * (hop_size as f32) / (fft_size as f32);
    for k in 0..num_bins {
        let expected_advance = two_pi_r_over_n * (k as f32);
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
    debug_assert!(unwrapped.len()   >= num_bins);
    debug_assert!(wrapped_out.len() >= num_bins);
    for k in 0..num_bins {
        wrapped_out[k] = principal_arg(unwrapped[k]);
    }
}

/// Damp the unwrapped phase of low-energy bins toward their expected-advance
/// value, using a soft-sigmoid blend across a ±6 dB band centred on the
/// noise floor. Avoids letting noise-dominated phase pollute downstream
/// peak-relative math.
///
/// `mags` and `expected_phase` are length `>= num_bins`. The expected phase
/// is the per-bin cumulative `2π · k · hop_total / fft_size` (caller-computed).
/// `noise_floor_db` is the dB FS reference (typically -60.0).
pub fn damp_low_energy_bins(
    unwrapped:      &mut [f32],
    mags:           &[f32],
    expected_phase: &[f32],
    noise_floor_db: f32,
    num_bins:       usize,
) {
    debug_assert!(unwrapped.len()      >= num_bins);
    debug_assert!(mags.len()           >= num_bins);
    debug_assert!(expected_phase.len() >= num_bins);

    let floor_lin    = 10.0_f32.powf(noise_floor_db / 20.0);
    let band_lo      = floor_lin * 0.5_f32; // -6 dB below floor
    let band_hi      = floor_lin * 2.0_f32; // +6 dB above floor
    let band_inv_len = 1.0 / (band_hi - band_lo);

    for k in 0..num_bins {
        let m = mags[k];
        let blend = if m <= band_lo {
            1.0  // fully damped
        } else if m >= band_hi {
            0.0  // untouched
        } else {
            // Smoothstep across the ±6 dB band.
            let t = ((band_hi - m) * band_inv_len).clamp(0.0, 1.0);
            t * t * (3.0 - 2.0 * t)
        };
        if blend > 0.0 {
            unwrapped[k] = unwrapped[k] * (1.0 - blend) + expected_phase[k] * blend;
        }
    }
}
