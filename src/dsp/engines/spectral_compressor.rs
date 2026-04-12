use num_complex::Complex;
use super::{SpectralEngine, BinParams};

#[derive(Default)]
pub struct SpectralCompressorEngine {
    /// Per-bin envelope state in dBFS (smoothed level tracking).
    env_db:      Vec<f32>,
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

    /// Convert milliseconds to one-pole coefficient for hop-rate envelope follower.
    /// Returns coefficient in [0.0, 1.0): higher = slower response.
    #[inline]
    fn ms_to_coeff(ms: f32, sample_rate: f32, hop_size: usize) -> f32 {
        if ms < 0.001 { return 0.0; }
        let hops_per_sec = sample_rate / hop_size as f32;
        let time_hops = ms * 0.001 * hops_per_sec;
        (-1.0_f32 / time_hops).exp()
    }
}

impl SpectralEngine for SpectralCompressorEngine {
    fn reset(&mut self, sample_rate: f32, fft_size: usize) {
        self.sample_rate = sample_rate;
        self.fft_size    = fft_size;
        self.hop_size    = fft_size / 4; // 75% overlap
        self.num_bins    = fft_size / 2 + 1;
        // Initialise envelopes to silence so attack ramps up from nothing
        self.env_db = vec![-96.0f32; self.num_bins];
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
        let n   = bins.len().min(self.num_bins);

        for k in 0..n {
            // 1. Detect level — use sidechain magnitude if provided, else self-keyed
            let level_linear = match sidechain {
                Some(sc) => sc.get(k).copied().unwrap_or(0.0),
                None     => bins[k].norm(),
            };
            let level_db = if level_linear > 1e-10 {
                20.0 * level_linear.log10()
            } else {
                -96.0
            };

            // 2. Envelope follower: one-pole LP at hop rate
            let attack_ms  = params.attack_ms[k].max(0.1);
            let release_ms = params.release_ms[k].max(1.0);
            let coeff = if level_db > self.env_db[k] {
                Self::ms_to_coeff(attack_ms, sample_rate, hop)
            } else {
                Self::ms_to_coeff(release_ms, sample_rate, hop)
            };
            self.env_db[k] = coeff * self.env_db[k] + (1.0 - coeff) * level_db;

            // 3. Gain computer → gain reduction in dB (≤ 0)
            let threshold_db = params.threshold_db[k];
            let ratio        = params.ratio[k].max(1.0);
            let knee_db      = params.knee_db[k].max(0.0);
            let gr_db        = Self::gain_computer(self.env_db[k], threshold_db, ratio, knee_db);

            // 4. Total gain = GR + makeup, converted to linear
            let total_db     = gr_db + params.makeup_db[k];
            let linear_gain  = 10.0f32.powf(total_db / 20.0);

            // 5. Apply gain with per-bin dry/wet mix
            let mix = params.mix[k].clamp(0.0, 1.0);
            bins[k] = bins[k] * (1.0 - mix + mix * linear_gain);

            // 6. Write |gain_reduction_db| to suppression_out for GUI stalactites
            suppression_out[k] = (-gr_db).max(0.0);
        }
    }

    fn name(&self) -> &'static str { "Spectral Compressor" }
}
