use num_complex::Complex;
use crate::dsp::utils::ms_to_coeff;
use super::{SpectralEngine, BinParams};

#[derive(Default)]
pub struct SpectralContrastEngine {
    /// Temporally-smoothed local spatial mean magnitude per bin.
    contrast_env: Vec<f32>,
    /// Raw contrast GR per bin in dB (positive = boost, negative = cut).
    gr_db: Vec<f32>,
    /// Frequency-smoothed GR mask (anti-warbling, same prefix-sum as compressor Pass 2).
    smooth_buf: Vec<f32>,
    /// Long-term average GR for auto-makeup (~1000 ms smoothing).
    auto_makeup_db: Vec<f32>,
    /// Prefix-sum scratch buffer for spatial mean computation (length num_bins + 1).
    mag_prefix: Vec<f32>,
    /// Prefix-sum scratch buffer for GR mask smoothing (length num_bins + 1).
    gr_prefix: Vec<f32>,
    num_bins:    usize,
    sample_rate: f32,
    hop_size:    usize,
    fft_size:    usize,
}

impl SpectralContrastEngine {
    pub fn new() -> Self { Self::default() }

    /// Contrast gain computer with soft knee.
    ///
    /// `deviation_db`: how far the bin is above (+) or below (-) its local mean.
    /// `ratio`: 1.0 = no effect; >1.0 = expand deviations (enhance contrast);
    ///          <1.0 = compress deviations (flatten spectrum toward local mean).
    /// Returns GR in dB (positive = boost, negative = cut).
    #[inline]
    fn contrast_gain(deviation_db: f32, ratio: f32, knee_db: f32) -> f32 {
        // gr = deviation * (ratio - 1): at ratio=2, a +6 dB peak becomes +12 dB.
        let gr = deviation_db * (ratio - 1.0);
        if knee_db < 0.001 {
            return gr;
        }
        // Soft knee: smoothly ramp up the effect within ±knee_db/2 of zero deviation.
        // At |deviation| = 0   → gr = 0 (no discontinuity at the mean).
        // At |deviation| ≥ knee → full gr.
        let half_knee = knee_db / 2.0;
        if deviation_db.abs() <= half_knee {
            deviation_db * (ratio - 1.0) * (deviation_db.abs() / knee_db)
        } else {
            gr
        }
    }


}

impl SpectralEngine for SpectralContrastEngine {
    fn reset(&mut self, sample_rate: f32, fft_size: usize) {
        self.sample_rate = sample_rate;
        self.fft_size    = fft_size;
        self.hop_size    = fft_size / 4;
        self.num_bins    = fft_size / 2 + 1;
        self.contrast_env  = vec![0.0f32; self.num_bins];
        self.gr_db         = vec![0.0f32; self.num_bins];
        self.smooth_buf    = vec![0.0f32; self.num_bins];
        self.auto_makeup_db = vec![0.0f32; self.num_bins];
        self.mag_prefix    = vec![0.0f32; self.num_bins + 1];
        self.gr_prefix     = vec![0.0f32; self.num_bins + 1];
    }

    fn process_bins(
        &mut self,
        bins: &mut [Complex<f32>],
        _sidechain: Option<&[f32]>,  // contrast is always self-referenced
        params: &BinParams<'_>,
        sample_rate: f32,
        suppression_out: &mut [f32],
    ) {
        debug_assert_eq!(bins.len(), self.num_bins);
        let n   = bins.len();
        let hop = self.hop_size;

        // Effective log-frequency neighbourhood width.
        // Minimum 1 semitone so the local mean spans more than the bin itself.
        let width_ratio = 2.0f32.powf(params.smoothing_semitones.max(1.0) / 12.0);

        // Pass 1 — build prefix sum of raw magnitudes for spatial mean computation.
        // All magnitudes come from the unmodified bins of this frame so no bin's
        // gain computation sees already-boosted/cut data from the same frame.
        self.mag_prefix[0] = 0.0;
        for k in 0..n {
            self.mag_prefix[k + 1] = self.mag_prefix[k] + bins[k].norm();
        }

        // Pass 2 — temporal tracking of local spatial mean + contrast gain computer.
        for k in 0..n {
            // Spatial mean in the log-frequency neighbourhood [k/w, k*w].
            let k_lo = ((k as f32 / width_ratio).floor() as usize).min(k);
            let k_hi = ((k as f32 * width_ratio).ceil() as usize).min(n - 1).max(k);
            let count = (k_hi - k_lo + 1) as f32;
            let local_mean = (self.mag_prefix[k_hi + 1] - self.mag_prefix[k_lo]) / count;

            // One-pole LP tracking of the local mean (attack when rising, release when falling).
            let attack_ms  = params.attack_ms[k].max(0.1);
            let release_ms = params.release_ms[k].max(1.0);
            let coeff = if local_mean > self.contrast_env[k] {
                ms_to_coeff(attack_ms,  sample_rate, hop)
            } else {
                ms_to_coeff(release_ms, sample_rate, hop)
            };
            self.contrast_env[k] = coeff * self.contrast_env[k] + (1.0 - coeff) * local_mean;

            // Deviation of this bin from its temporally-smoothed local mean.
            // Using the ratio (mag / env) avoids the dBFS calibration offset that
            // cancels in the difference anyway: deviation = 20*log10(mag/env).
            let mag = bins[k].norm();
            let env = self.contrast_env[k].max(1e-10);
            // Clamp deviation to ±48 dB — prevents startup transient explosion when
            // contrast_env hasn't yet converged from its zero-initialised state.
            let deviation_db = (20.0 * (mag / env).log10()).clamp(-48.0, 48.0);

            let ratio   = params.ratio[k].clamp(0.0, 20.0);
            let knee_db = params.knee_db[k].max(0.0);
            self.gr_db[k] = Self::contrast_gain(deviation_db, ratio, knee_db);
        }

        // Pass 3 — smooth the gain mask in log-frequency (anti-warbling).
        // Prevents abrupt bin-to-bin gain discontinuities (the primary cause of
        // "musical noise" / spectral warbling with hard per-bin contrast).
        if params.smoothing_semitones < 0.01 {
            self.smooth_buf.copy_from_slice(&self.gr_db);
        } else {
            self.gr_prefix[0] = 0.0;
            for k in 0..n {
                self.gr_prefix[k + 1] = self.gr_prefix[k] + self.gr_db[k];
            }
            for k in 0..n {
                let k_lo = ((k as f32 / width_ratio).floor() as usize).min(k);
                let k_hi = ((k as f32 * width_ratio).ceil() as usize).min(n - 1).max(k);
                let count = (k_hi - k_lo + 1) as f32;
                self.smooth_buf[k] = (self.gr_prefix[k_hi + 1] - self.gr_prefix[k_lo]) / count;
            }
        }

        // Update auto-makeup long-term average (~1000 ms, tracks mix-weighted GR).
        let coeff_slow = ms_to_coeff(1000.0, sample_rate, hop);
        for k in 0..n {
            let effective_gr = self.smooth_buf[k] * params.mix[k].clamp(0.0, 1.0);
            self.auto_makeup_db[k] = coeff_slow * self.auto_makeup_db[k]
                + (1.0 - coeff_slow) * effective_gr;
        }

        // Pass 4 — apply smoothed gain + makeup + auto-makeup + mix.
        for k in 0..n {
            let auto_comp   = if params.auto_makeup { -self.auto_makeup_db[k] } else { 0.0 };
            // Clamp total_db to ±40 dB (100× linear gain max) to prevent f32 overflow
            // from unusually large GR values reaching the audio output.
            let total_db    = (self.smooth_buf[k] + params.makeup_db[k] + auto_comp).clamp(-80.0, 40.0);
            let linear_gain = 10.0f32.powf(total_db / 20.0);
            let mix         = params.mix[k].clamp(0.0, 1.0);
            bins[k] = bins[k] * (1.0 - mix + mix * linear_gain);
            // Suppression out: show the magnitude of net cut to the existing display.
            // Boosts (positive smooth_buf) are currently not visualised.
            suppression_out[k] = (-self.smooth_buf[k]).max(0.0);
        }
    }

    fn clear_state(&mut self) {
        self.contrast_env.fill(0.0);
        self.gr_db.fill(0.0);
        self.smooth_buf.fill(0.0);
        self.auto_makeup_db.fill(0.0);
        self.mag_prefix.fill(0.0);
        self.gr_prefix.fill(0.0);
    }

    fn name(&self) -> &'static str { "Spectral Contrast" }
}
