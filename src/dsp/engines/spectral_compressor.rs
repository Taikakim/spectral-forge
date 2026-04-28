use num_complex::Complex;
use multiversion::multiversion;
use crate::dsp::utils::ms_to_coeff;
use super::{SpectralEngine, BinParams};

/// Apply per-bin gain reduction, makeup, and wet/dry mix to complex bins.
///
/// Extracted so `#[multiversion]` generates AVX2/SSE4.1/scalar variants and selects
/// the best at runtime via CPUID. The loop has no cross-bin dependencies and is purely
/// scalar arithmetic — ideal for auto-vectorisation.
///
/// `auto_scale` is `−1.0` when auto-makeup is active, `0.0` otherwise, hoisting the
/// branch outside the loop so the vectoriser sees a simple multiply-accumulate.
#[multiversion(targets("x86_64+avx2+fma", "x86_64+sse4.1"))]
fn apply_gains(
    bins:           &mut [Complex<f32>],
    smooth_buf:     &[f32],
    makeup_db:      &[f32],
    auto_makeup_db: &[f32],
    mix:            &[f32],
    suppression_out: &mut [f32],
    auto_scale:     f32,
) {
    // Use exp(x * LN_10/20) instead of 10^(x/20): identical result, but LLVM can
    // vectorise exp() to _mm256_exp_ps with SVML whereas powf blocks auto-vec.
    const LN10_OVER_20: f32 = std::f32::consts::LN_10 / 20.0;
    for k in 0..bins.len() {
        let total_db    = smooth_buf[k] + makeup_db[k] + auto_scale * auto_makeup_db[k];
        let linear_gain = (total_db * LN10_OVER_20).exp();
        let mix_k       = mix[k].clamp(0.0, 1.0);
        bins[k]          = bins[k] * (1.0 - mix_k + mix_k * linear_gain);
        suppression_out[k] = (-smooth_buf[k]).max(0.0);
    }
}

#[derive(Default)]
pub struct SpectralCompressorEngine {
    /// Per-bin envelope state in dBFS (smoothed level tracking).
    env_db:      Vec<f32>,
    /// Raw gain reduction per bin (dB, ≤ 0) — computed in pass 1.
    gr_db:       Vec<f32>,
    /// Smoothed gain reduction per bin — reused buffer, no per-call allocation.
    smooth_buf:        Vec<f32>,
    /// Smoothed per-bin median magnitude for relative threshold mode.
    spectral_envelope: Vec<f32>,
    /// Long-term average gain reduction per bin for auto-makeup (~1000ms smoothing).
    auto_makeup_db: Vec<f32>,
    /// Prefix-sum scratch buffer for log-frequency GR smoothing (length num_bins + 1).
    prefix_buf: Vec<f32>,
    num_bins:    usize,
    sample_rate: f32,
    fft_size:    usize,
    hop_size:    usize,
}

impl SpectralCompressorEngine {
    pub fn new() -> Self { Self::default() }

    /// Soft-knee gain computer. Returns gain change in dB (≤ 0 for reduction).
    #[inline]
    fn gain_computer(level_db: f32, threshold_db: f32, ratio: f32, knee_db: f32) -> f32 {
        let overshoot = level_db - threshold_db;
        if knee_db < 0.001 {
            // Hard knee
            if overshoot <= 0.0 { 0.0 }
            else { overshoot * (1.0 / ratio - 1.0) }
        } else {
            // Soft knee (quadratic)
            let half_knee = knee_db / 2.0;
            if overshoot <= -half_knee {
                0.0
            } else if overshoot <= half_knee {
                (overshoot + half_knee).powi(2) / (2.0 * knee_db) * (1.0 / ratio - 1.0)
            } else {
                overshoot * (1.0 / ratio - 1.0)
            }
        }
    }


}

impl SpectralEngine for SpectralCompressorEngine {
    fn reset(&mut self, sample_rate: f32, fft_size: usize) {
        self.sample_rate = sample_rate;
        self.fft_size    = fft_size;
        self.hop_size    = fft_size / 4; // 75% overlap
        self.num_bins    = fft_size / 2 + 1;
        // Initialise envelopes to silence so attack ramps up from nothing
        self.env_db           = vec![-96.0f32; self.num_bins];
        self.gr_db            = vec![0.0f32; self.num_bins];
        self.smooth_buf       = vec![0.0f32; self.num_bins];
        self.spectral_envelope = vec![0.0f32; self.num_bins];
        self.auto_makeup_db   = vec![0.0f32; self.num_bins];
        self.prefix_buf       = vec![0.0f32; self.num_bins + 1];
    }

    fn process_bins(
        &mut self,
        bins: &mut [Complex<f32>],
        sidechain: Option<&[f32]>,
        params: &BinParams<'_>,
        sample_rate: f32,
        suppression_out: &mut [f32],
    ) {
        debug_assert_eq!(bins.len(), self.num_bins,
            "bins.len() ({}) != num_bins ({})", bins.len(), self.num_bins);

        let hop = self.hop_size;
        let n   = bins.len(); // caller guarantees == num_bins (asserted above)

        // Pre-pass — update spectral envelope from main-signal bins.
        // Always computed; sensitivity blends how much it shifts the effective threshold.
        // Uses a 3-tap median of each bin and its immediate neighbours to track the
        // local spectral "floor" — bins that stick out above this floor look like tones
        // or resonances; bins at or below it look like broadband content.
        let env_coeff = ms_to_coeff(50.0, sample_rate, hop);
        for k in 0..n {
            let mag = bins[k].norm();
            let lo  = if k > 0     { bins[k - 1].norm() } else { mag };
            let hi  = if k + 1 < n { bins[k + 1].norm() } else { mag };
            let med = {
                let mut arr = [lo, mag, hi];
                arr.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                arr[1]
            };
            self.spectral_envelope[k] =
                env_coeff * self.spectral_envelope[k] + (1.0 - env_coeff) * med;
        }

        // FFT magnitude calibration offset: raw FFT norm for a 0 dBFS sine ≈ fft_size/4,
        // so subtracting this converts raw dB to dBFS.
        let norm_offset = 20.0 * (self.fft_size as f32 / 4.0).log10();
        let sensitivity  = params.sensitivity.clamp(0.0, 1.0);

        // Pass 1 — envelope follower + gain computer → raw gr per bin
        for k in 0..n {
            // 1. Detect level in calibrated dBFS (same scale as threshold_db).
            let level_linear = match sidechain {
                Some(sc) => sc.get(k).copied().unwrap_or(0.0),
                None     => bins[k].norm(),
            };
            let level_db = if level_linear > 1e-10 {
                20.0 * level_linear.log10() - norm_offset
            } else {
                -120.0
            };

            // 2. Sensitivity — raise the effective threshold toward the local spectral
            //    envelope level so that bins blending into their neighbours are spared.
            //    • 0.0: effective_threshold = threshold_db  (pure absolute compressor)
            //    • 1.0: effective_threshold = max(threshold_db, envelope_db)
            //           → only bins that stick out above the local spectral floor compress
            //    Values in between blend continuously.
            let env_db = if self.spectral_envelope[k] > 1e-10 {
                20.0 * self.spectral_envelope[k].log10() - norm_offset
            } else {
                -120.0
            };
            let threshold_db = params.threshold_db[k];
            let envelope_excess = (env_db - threshold_db).max(0.0); // only raises, never lowers
            let effective_threshold = threshold_db + sensitivity * envelope_excess;

            // 3. Envelope follower: one-pole LP at hop rate
            let attack_ms  = params.attack_ms[k].max(0.1);
            let release_ms = params.release_ms[k].max(1.0);
            let coeff = if level_db > self.env_db[k] {
                ms_to_coeff(attack_ms, sample_rate, hop)
            } else {
                ms_to_coeff(release_ms, sample_rate, hop)
            };
            self.env_db[k] = coeff * self.env_db[k] + (1.0 - coeff) * level_db;

            // 4. Gain computer → raw gain reduction in dB (≤ 0)
            let ratio   = params.ratio[k].max(1.0);
            let knee_db = params.knee_db[k].max(0.0);
            self.gr_db[k] = Self::gain_computer(self.env_db[k], effective_threshold, ratio, knee_db);
        }

        // Pass 2 — log-frequency gain-reduction smoothing.
        // `smoothing_semitones` is the half-width (each side) in semitones; the kernel
        // covers [k / 2^(w/12), k * 2^(w/12)] in bin-index space, which is a constant
        // musical interval regardless of frequency.  Uses a prefix sum for O(n) cost.
        if params.smoothing_semitones < 0.01 {
            // No smoothing — copy gr_db verbatim
            self.smooth_buf.copy_from_slice(&self.gr_db);
        } else {
            // width_ratio: bin range multiplier for the chosen semitone width.
            // e.g. 12 st → ratio = 2^(12/12) = 2  (one octave each side)
            let width_ratio = 2.0f32.powf(params.smoothing_semitones / 12.0);
            // Build prefix sum of gr_db so range queries are O(1)
            self.prefix_buf[0] = 0.0;
            for k in 0..n {
                self.prefix_buf[k + 1] = self.prefix_buf[k] + self.gr_db[k];
            }
            for k in 0..n {
                let k_lo = ((k as f32 / width_ratio).floor() as usize).min(k);
                let k_hi = ((k as f32 * width_ratio).ceil() as usize).min(n - 1).max(k);
                let range = (k_hi - k_lo + 1) as f32;
                self.smooth_buf[k] = (self.prefix_buf[k_hi + 1] - self.prefix_buf[k_lo]) / range;
            }
        }

        // Pass 2.5 — PLPV peak-locked override (Phase 4.3a).
        // For each detected peak, force every bin in its Voronoi skirt to share the
        // peak bin's gain reduction. Preserves partial coherence under ducking — all
        // bins of a single partial duck together. No-op when the per-module flag is
        // off OR the Pipeline didn't populate ctx.peaks (i.e. global PLPV off).
        if params.plpv_dynamics_enabled {
            if let Some(peaks) = params.peaks {
                for p in peaks {
                    let pk = (p.k as usize).min(n - 1);
                    let peak_gr = self.smooth_buf[pk];
                    let lo = (p.low_k as usize).min(n - 1);
                    let hi = (p.high_k as usize).min(n - 1);
                    for k in lo..=hi {
                        self.smooth_buf[k] = peak_gr;
                    }
                }
            }
        }

        // Update auto-makeup long-term average (~1000ms smoothing at hop rate).
        // Tracks the mix-weighted spatially-smoothed GR (smooth_buf * mix) — i.e. the
        // GR actually applied to the audio — so compensation is exact at any mix setting.
        let coeff_slow = ms_to_coeff(1000.0, sample_rate, hop);
        for k in 0..n {
            let effective_gr = self.smooth_buf[k] * params.mix[k].clamp(0.0, 1.0);
            self.auto_makeup_db[k] = coeff_slow * self.auto_makeup_db[k]
                + (1.0 - coeff_slow) * effective_gr;
        }

        // Pass 3 — apply smoothed gain reduction + makeup + mix (SIMD-dispatched)
        let auto_scale = if params.auto_makeup { -1.0f32 } else { 0.0f32 };
        apply_gains(
            bins,
            &self.smooth_buf[..n],
            params.makeup_db,
            &self.auto_makeup_db[..n],
            params.mix,
            suppression_out,
            auto_scale,
        );
    }

    fn clear_state(&mut self) {
        self.env_db.fill(-96.0);
        self.gr_db.fill(0.0);
        self.smooth_buf.fill(0.0);
        self.spectral_envelope.fill(0.0);
        self.auto_makeup_db.fill(0.0);
        self.prefix_buf.fill(0.0);
    }

    fn name(&self) -> &'static str { "Spectral Compressor" }
}
