//! Master output soft clipper.
//!
//! Threshold-gated soft saturation in the spectral magnitude domain:
//!   - bins with magnitude <= threshold → bit-exact passthrough
//!   - bins above threshold → soft asymptotic approach to a ceiling
//!
//! ## Calibration
//!
//! Bin magnitudes inside the STFT closure are at the un-normalised rfft
//! scale, which is FFT-size-dependent. For a Hann-windowed 0 dBFS sine,
//! the peak bin magnitude is approximately `fft_size / 4`. We anchor
//! `threshold_db = 0` to that level — so 0 dB = "only clip 0-dBFS peaks"
//! and -24 dB = "clip aggressively, even at moderate levels".
//!
//!   t_lin = 10^(threshold_db / 20) * (fft_size / 4)
//!
//! The curve above the knee is `1 - exp(-x)` applied to the excess and
//! scaled to `ceiling - t_lin`, where `ceiling = 4 × t_lin` (12 dB
//! headroom window). At `mag = t_lin` the output equals `t_lin` and the
//! derivative is 1 from both sides → smooth knee.
//!
//! See docs/superpowers/specs/2026-05-06-stabilization-sweep.md §4.4.

use num_complex::Complex;

/// Soft-clip magnitudes per-bin with a threshold knee.
///
/// `threshold_db` is in dBFS, expected range -24..0. `fft_size` is needed
/// because bin magnitudes scale with it. Bins at or below the linear
/// threshold are unchanged; above, they approach `4 × threshold`
/// asymptotically.
#[inline]
pub fn apply_soft_clip(
    bins: &mut [Complex<f32>],
    num_bins: usize,
    threshold_db: f32,
    fft_size: usize,
) {
    let peak_mag_0dbfs = (fft_size as f32) * 0.25;
    let t_lin = 10f32.powf(threshold_db / 20.0) * peak_mag_0dbfs;
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
