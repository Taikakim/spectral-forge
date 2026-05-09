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

    /// Spatial kernel — refactored from process_bins to take mean_window_st.
    /// Identical behaviour at mean_window_st=1.0.
    pub fn process_bins_spatial(
        &mut self,
        bins: &mut [Complex<f32>],
        params: &BinParams<'_>,
        sample_rate: f32,
        mean_window_st: f32,
        suppression_out: &mut [f32],
    ) {
        debug_assert_eq!(bins.len(), self.num_bins);
        let n   = bins.len();
        let hop = self.hop_size;

        let width_ratio = 2.0f32.powf(mean_window_st.max(0.1) / 12.0);

        // Pass 1 — prefix sum of raw magnitudes for spatial mean
        self.mag_prefix[0] = 0.0;
        for k in 0..n {
            self.mag_prefix[k + 1] = self.mag_prefix[k] + bins[k].norm();
        }

        // Pass 2 — temporal-tracked spatial mean + contrast gain
        for k in 0..n {
            let k_lo = ((k as f32 / width_ratio).floor() as usize).min(k);
            let k_hi = ((k as f32 * width_ratio).ceil() as usize).min(n - 1).max(k);
            let count = (k_hi - k_lo + 1) as f32;
            let local_mean = (self.mag_prefix[k_hi + 1] - self.mag_prefix[k_lo]) / count;

            let attack_ms  = params.attack_ms[k].max(0.1);
            let release_ms = params.release_ms[k].max(1.0);
            let coeff = if local_mean > self.contrast_env[k] {
                ms_to_coeff(attack_ms,  sample_rate, hop)
            } else {
                ms_to_coeff(release_ms, sample_rate, hop)
            };
            self.contrast_env[k] = coeff * self.contrast_env[k] + (1.0 - coeff) * local_mean;

            let mag = bins[k].norm();
            let env = self.contrast_env[k].max(1e-10);
            let deviation_db = (20.0 * (mag / env).log10()).clamp(-48.0, 48.0);
            let ratio   = params.ratio[k].clamp(0.0, 20.0);
            let knee_db = params.knee_db[k].max(0.0);
            self.gr_db[k] = Self::contrast_gain(deviation_db, ratio, knee_db);
        }

        self.smooth_and_apply(bins, params, sample_rate, mean_window_st, suppression_out);
    }

    /// Temporal kernel — per-bin deviation from each bin's own long-running mean.
    /// Uses ATTACK/RELEASE as the time constants. After convergence on a steady
    /// input, current = mean → zero deviation → unity gain (no contrast applied).
    pub fn process_bins_temporal(
        &mut self,
        bins: &mut [Complex<f32>],
        params: &BinParams<'_>,
        sample_rate: f32,
        suppression_out: &mut [f32],
    ) {
        debug_assert_eq!(bins.len(), self.num_bins);
        let n   = bins.len();
        let hop = self.hop_size;

        for k in 0..n {
            let attack_ms  = params.attack_ms[k].max(0.1);
            let release_ms = params.release_ms[k].max(1.0);
            let mag = bins[k].norm();
            let coeff = if mag > self.contrast_env[k] {
                ms_to_coeff(attack_ms,  sample_rate, hop)
            } else {
                ms_to_coeff(release_ms, sample_rate, hop)
            };
            self.contrast_env[k] = coeff * self.contrast_env[k] + (1.0 - coeff) * mag;

            let env = self.contrast_env[k].max(1e-10);
            let deviation_db = (20.0 * (mag / env).log10()).clamp(-48.0, 48.0);
            let ratio   = params.ratio[k].clamp(0.0, 20.0);
            let knee_db = params.knee_db[k].max(0.0);
            self.gr_db[k] = Self::contrast_gain(deviation_db, ratio, knee_db);
        }
        // Temporal mode uses smooth_and_apply with the spec's default semitone width
        // (mean_window_st=1.0 — engine still smooths the GR mask in log-frequency)
        self.smooth_and_apply(bins, params, sample_rate, 1.0, suppression_out);
    }

    /// Tilt kernel — per-bin deviation from a fitted 1/f^alpha reference slope.
    /// baseline_db is the average dBFS across bins (self-tuning to overall level).
    /// Negative slope = pink reference; positive slope = blue reference.
    pub fn process_bins_tilt(
        &mut self,
        bins: &mut [Complex<f32>],
        params: &BinParams<'_>,
        fft_size: usize,
        sample_rate: f32,
        slope_db_per_oct: f32,
        suppression_out: &mut [f32],
    ) {
        debug_assert_eq!(bins.len(), self.num_bins);
        let n = bins.len();

        // Self-tuning baseline: average dBFS across bins (skip k=0 DC)
        let mut sum_db = 0.0f32;
        let mut count = 0u32;
        for k in 1..n {
            let mag = bins[k].norm().max(1e-10);
            sum_db += 20.0 * mag.log10();
            count += 1;
        }
        let baseline_db = sum_db / count.max(1) as f32;

        for k in 0..n {
            let freq_hz = (k as f32 * sample_rate / fft_size as f32).max(20.0);
            let oct_from_1k = (freq_hz / 1000.0).log2();
            let expected_db = baseline_db + slope_db_per_oct * oct_from_1k;
            let mag_db = 20.0 * bins[k].norm().max(1e-10).log10();
            let deviation_db = (mag_db - expected_db).clamp(-48.0, 48.0);
            let ratio   = params.ratio[k].clamp(0.0, 20.0);
            let knee_db = params.knee_db[k].max(0.0);
            self.gr_db[k] = Self::contrast_gain(deviation_db, ratio, knee_db);
        }
        self.smooth_and_apply(bins, params, sample_rate, 1.0, suppression_out);
    }

    /// Pass 3 + Pass 4 from the original process_bins, factored so all three
    /// kernels can reuse them. Includes THRESHOLD bypass-floor (Task 0 fix).
    fn smooth_and_apply(
        &mut self,
        bins: &mut [Complex<f32>],
        params: &BinParams<'_>,
        sample_rate: f32,
        mean_window_st: f32,
        suppression_out: &mut [f32],
    ) {
        let n   = bins.len();
        let hop = self.hop_size;
        let width_ratio = 2.0f32.powf(mean_window_st.max(0.1) / 12.0);

        // Pass 3 — log-frequency smoothing of GR mask
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

        // Auto-makeup tracker (1-second smoothing of effective GR)
        let coeff_slow = ms_to_coeff(1000.0, sample_rate, hop);
        for k in 0..n {
            let effective_gr = self.smooth_buf[k] * params.mix[k].clamp(0.0, 1.0);
            self.auto_makeup_db[k] = coeff_slow * self.auto_makeup_db[k]
                + (1.0 - coeff_slow) * effective_gr;
        }

        // Pass 4 — apply with THRESHOLD bypass floor (from Task 0)
        for k in 0..n {
            let auto_comp   = if params.auto_makeup { -self.auto_makeup_db[k] } else { 0.0 };
            let total_db    = (self.smooth_buf[k] + params.makeup_db[k] + auto_comp).clamp(-80.0, 40.0);
            let linear_gain = 10.0f32.powf(total_db / 20.0);

            let mag_db   = 20.0 * bins[k].norm().max(1e-10).log10();
            let bypass_t = if mag_db < params.threshold_db[k] { 1.0 } else { 0.0 };
            let mix      = params.mix[k].clamp(0.0, 1.0) * (1.0 - bypass_t);

            bins[k] = bins[k] * (1.0 - mix + mix * linear_gain);
            suppression_out[k] = (-self.smooth_buf[k]).max(0.0);
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
        // Legacy entry point — defaults to Spatial mode with default mean window.
        self.process_bins_spatial(bins, params, sample_rate, 1.0, suppression_out);
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
