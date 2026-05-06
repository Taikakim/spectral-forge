//! Master output soft clipper.
//!
//! Threshold-gated soft saturation in the spectral magnitude domain:
//!   - bins with magnitude <= threshold → bit-exact passthrough
//!   - bins above threshold → soft asymptotic approach to a ceiling
//!
//! The curve is `1 - exp(-x)`, applied to the excess above the threshold and
//! scaled to a knee width = (ceiling - threshold). At mag = threshold the
//! output equals threshold (continuity); for mag → ∞ the output asymptotes
//! to ceiling. Derivative at the knee is 1 from both sides → smooth knee.
//!
//! Threshold maps from dBFS (knob: -24..0) to linear magnitude
//! `t_lin = 10^(t_db/20)`. Ceiling = 4× threshold gives a 12 dB headroom
//! window — gentle enough that quiet/normal mixes feel transparent and only
//! peaks get bounded.
//!
//! See docs/superpowers/specs/2026-05-06-stabilization-sweep.md §4.4 for
//! original toggle design; the threshold knob and curve reshape were added
//! after the initial K/(K+|bin|) algorithm proved too colorful.

use num_complex::Complex;

/// Soft-clip magnitudes per-bin with a threshold knee.
///
/// `threshold_db` is in dBFS, expected range -24..0. Bins with magnitude
/// at or below `10^(threshold_db/20)` are unchanged. Above the threshold
/// the magnitude approaches `4 × threshold` asymptotically.
#[inline]
pub fn apply_soft_clip(bins: &mut [Complex<f32>], num_bins: usize, threshold_db: f32) {
    let t_lin = 10f32.powf(threshold_db / 20.0);
    let ceiling = t_lin * 4.0;
    let knee = ceiling - t_lin;
    if knee <= 1e-9 {
        return;
    }
    for k in 0..num_bins.min(bins.len()) {
        let mag = bins[k].norm();
        if mag > t_lin {
            let excess = mag - t_lin;
            let normalized = excess / knee;
            let scaled_excess = knee * (1.0 - (-normalized).exp());
            let new_mag = t_lin + scaled_excess;
            let scale = new_mag / mag;
            bins[k] *= scale;
        }
    }
}
