use num_complex::Complex;
use crate::params::{FxChannelTarget, StereoLink};
use super::{GainMode, ModuleContext, ModuleType, SpectralModule};
use crate::dsp::pipeline::{MAX_NUM_BINS, OVERLAP};

pub struct GainModule {
    pub(crate) mode: GainMode,
    /// Per-bin peak-hold envelope state. Used by Pull and Match.
    peak_env: Vec<f32>,
    /// Scratch integral images of ln(mag) for ERB-smoothed envelopes (Match only).
    /// Length `MAX_NUM_BINS + 1`; index 0 is a zero sentinel so the running-sum
    /// formula `cum[hi+1] - cum[lo]` covers an inclusive `[lo..=hi]` range.
    cum_main_log: Vec<f32>,
    cum_sc_log:   Vec<f32>,
    sample_rate: f32,
    fft_size: usize,
    #[cfg(any(test, feature = "probe"))]
    last_probe: crate::dsp::modules::ProbeSnapshot,
}

impl GainModule {
    pub fn new() -> Self {
        Self {
            mode: GainMode::Add,
            peak_env: vec![0.0f32; MAX_NUM_BINS],
            cum_main_log: vec![0.0f32; MAX_NUM_BINS + 1],
            cum_sc_log:   vec![0.0f32; MAX_NUM_BINS + 1],
            sample_rate: 44100.0,
            fft_size: 2048,
            #[cfg(any(test, feature = "probe"))]
            last_probe: Default::default(),
        }
    }

    /// Pub-for-test inspector: current peak-hold envelope at bin `k`.
    #[doc(hidden)]
    pub fn peak_env_at(&self, k: usize) -> f32 {
        self.peak_env.get(k).copied().unwrap_or(0.0)
    }

}

impl Default for GainModule {
    fn default() -> Self { Self::new() }
}

impl SpectralModule for GainModule {
    fn reset(&mut self, sample_rate: f32, fft_size: usize) {
        self.sample_rate = sample_rate;
        self.fft_size = fft_size;
        for v in &mut self.peak_env { *v = 0.0; }
    }

    fn process(
        &mut self,
        _channel: usize,
        _stereo_link: StereoLink,
        _target: FxChannelTarget,
        bins: &mut [Complex<f32>],
        sidechain: Option<&[f32]>,
        curves: &[&[f32]],
        suppression_out: &mut [f32],
        _ctx: &ModuleContext<'_>,
    ) {
        let n = bins.len();
        #[cfg(any(test, feature = "probe"))]
        let probe_k = if n == 0 { 0 } else { n / 2 };
        #[cfg(any(test, feature = "probe"))]
        let mut probe_gain_db:      Option<f32> = None;
        #[cfg(any(test, feature = "probe"))]
        let mut probe_gain_pct:     Option<f32> = None;
        #[cfg(any(test, feature = "probe"))]
        let mut probe_peak_hold_ms: Option<f32> = None;

        // PEAK HOLD probe is a pure function of curve 1 at probe_k — compute it
        // here regardless of mode so Add/Subtract tests also populate it if ever
        // needed, and Pull/Match tests see the same value the DSP uses below.
        #[cfg(any(test, feature = "probe"))]
        if n > 0 {
            let hold_curve = curves.get(1).and_then(|c| c.get(probe_k)).copied().unwrap_or(1.0);
            probe_peak_hold_ms = Some(super::peak_hold_curve_to_ms(hold_curve));
        }

        match self.mode {
            GainMode::Add => {
                for k in 0..n {
                    let g  = curves.get(0).and_then(|c| c.get(k)).copied().unwrap_or(1.0);
                    let sc = sidechain.and_then(|s| s.get(k)).copied().unwrap_or(0.0).max(0.0);
                    bins[k] *= g + sc;

                    #[cfg(any(test, feature = "probe"))]
                    if k == probe_k {
                        probe_gain_db = Some(20.0 * g.max(1e-6).log10());
                    }
                }
            }
            GainMode::Subtract => {
                for k in 0..n {
                    let g  = curves.get(0).and_then(|c| c.get(k)).copied().unwrap_or(1.0);
                    let sc = sidechain.and_then(|s| s.get(k)).copied().unwrap_or(0.0).max(0.0);
                    bins[k] *= (g - sc).max(0.0);

                    #[cfg(any(test, feature = "probe"))]
                    if k == probe_k {
                        probe_gain_db = Some(20.0 * g.max(1e-6).log10());
                    }
                }
            }
            GainMode::Pull => {
                let hop_ms = self.fft_size as f32 / (OVERLAP as f32 * self.sample_rate) * 1000.0;
                for k in 0..n {
                    let g = curves.get(0).and_then(|c| c.get(k)).copied()
                            .unwrap_or(1.0).clamp(0.0, 1.0);
                    let sc_mag_raw = sidechain.and_then(|s| s.get(k)).copied().unwrap_or(0.0).max(0.0);

                    let hold_curve = curves.get(1).and_then(|c| c.get(k)).copied().unwrap_or(1.0);
                    let hold_ms = super::peak_hold_curve_to_ms(hold_curve);
                    let release_coeff = (-hop_ms / hold_ms.max(0.1)).exp();
                    if sc_mag_raw > self.peak_env[k] {
                        self.peak_env[k] = sc_mag_raw;
                    } else {
                        self.peak_env[k] = release_coeff * self.peak_env[k]
                            + (1.0 - release_coeff) * sc_mag_raw;
                    }
                    let sc_eff = self.peak_env[k];

                    let cur_mag = bins[k].norm();
                    if cur_mag > 1e-10 {
                        let target_mag = cur_mag * g + sc_eff * (1.0 - g);
                        bins[k] *= target_mag / cur_mag;
                    }

                    #[cfg(any(test, feature = "probe"))]
                    if k == probe_k {
                        probe_gain_pct = Some(g * 100.0);
                    }
                }
            }
            GainMode::Match => {
                // Timbre-match: derive a smooth per-bin EQ multiplier from the ratio of
                // ERB-smoothed SC vs. main log-magnitudes. Preserves main's harmonic
                // structure (the multiplier is a smooth envelope, applied to raw bins)
                // while tilting its broad spectral shape toward the SC's.
                //
                // GAIN curve (0..1) is the wet/dry mix: 1 = keep main, 0 = full match.
                // PEAK HOLD curve controls the per-bin temporal release on SC (same as
                // Pull) — Match still benefits from temporal stability.
                const MAX_BOOST_DB: f32 = 12.0;
                let max_lin = 10f32.powf(MAX_BOOST_DB / 20.0);
                let min_lin = 1.0 / max_lin;
                // Log floor ~ -120 dBFS so silent bins don't drive a -huge multiplier.
                const LOG_FLOOR: f32 = -13.8;

                let hop_ms = self.fft_size as f32 / (OVERLAP as f32 * self.sample_rate) * 1000.0;
                let sr = self.sample_rate;
                let fft_size = self.fft_size;

                // Step 1: update peak-held SC envelope (same rule as Pull).
                for k in 0..n {
                    let sc_raw = sidechain.and_then(|s| s.get(k)).copied().unwrap_or(0.0).max(0.0);
                    let hold_curve = curves.get(1).and_then(|c| c.get(k)).copied().unwrap_or(1.0);
                    let hold_ms = super::peak_hold_curve_to_ms(hold_curve);
                    let release_coeff = (-hop_ms / hold_ms.max(0.1)).exp();
                    if sc_raw > self.peak_env[k] {
                        self.peak_env[k] = sc_raw;
                    } else {
                        self.peak_env[k] = release_coeff * self.peak_env[k]
                            + (1.0 - release_coeff) * sc_raw;
                    }
                }

                // Step 2: build integral images of ln(mag) for main and peak-held SC.
                self.cum_main_log[0] = 0.0;
                self.cum_sc_log[0]   = 0.0;
                for k in 0..n {
                    let main_log = bins[k].norm().max(1e-6).ln().max(LOG_FLOOR);
                    let sc_log   = self.peak_env[k].max(1e-6).ln().max(LOG_FLOOR);
                    self.cum_main_log[k + 1] = self.cum_main_log[k] + main_log;
                    self.cum_sc_log[k + 1]   = self.cum_sc_log[k]   + sc_log;
                }

                // Step 3: per-bin ERB-proportional smoothing + apply matched multiplier,
                // mixed with GAIN curve. ERB(f) ≈ 24.7·(4.37·f/1000 + 1) Hz; half-window
                // in bins = ERB·fft/sr·0.5. At low freq it's ~1 bin; at high freq tens of bins.
                for k in 0..n {
                    let g = curves.get(0).and_then(|c| c.get(k)).copied()
                            .unwrap_or(1.0).clamp(0.0, 1.0);
                    let freq_hz = (k as f32 * sr / fft_size as f32).max(20.0);
                    let erb_hz = 24.7 * (4.37 * freq_hz / 1000.0 + 1.0);
                    let half_w = ((erb_hz * fft_size as f32 / sr) * 0.5).max(1.0) as usize;
                    let lo = k.saturating_sub(half_w);
                    let hi = (k + half_w).min(n - 1);
                    let count = (hi - lo + 1) as f32;
                    let smooth_main = (self.cum_main_log[hi + 1] - self.cum_main_log[lo]) / count;
                    let smooth_sc   = (self.cum_sc_log[hi + 1]   - self.cum_sc_log[lo])   / count;
                    let matched_mul = (smooth_sc - smooth_main).exp().clamp(min_lin, max_lin);
                    let mul = g + (1.0 - g) * matched_mul;
                    bins[k] *= mul;

                    #[cfg(any(test, feature = "probe"))]
                    if k == probe_k {
                        probe_gain_pct = Some(g * 100.0);
                    }
                }
            }
        }

        #[cfg(any(test, feature = "probe"))]
        {
            self.last_probe = crate::dsp::modules::ProbeSnapshot {
                gain_db:      probe_gain_db,
                gain_pct:     probe_gain_pct,
                peak_hold_ms: probe_peak_hold_ms,
                ..Default::default()
            };
        }

        suppression_out.fill(0.0);
    }

    fn set_gain_mode(&mut self, mode: GainMode) { self.mode = mode; }

    fn module_type(&self) -> ModuleType { ModuleType::Gain }
    fn num_curves(&self) -> usize { 2 }

    #[cfg(any(test, feature = "probe"))]
    fn last_probe(&self) -> crate::dsp::modules::ProbeSnapshot { self.last_probe }
}
