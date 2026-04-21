use num_complex::Complex;
use crate::params::{FxChannelTarget, StereoLink};
use super::{GainMode, ModuleContext, ModuleType, SpectralModule};
use crate::dsp::pipeline::{MAX_NUM_BINS, OVERLAP};

pub struct GainModule {
    pub(crate) mode: GainMode,
    /// Per-bin peak-hold envelope state. Only written in Pull mode.
    peak_env: Vec<f32>,
    sample_rate: f32,
    fft_size: usize,
}

impl GainModule {
    pub fn new() -> Self {
        Self {
            mode: GainMode::Add,
            peak_env: vec![0.0f32; MAX_NUM_BINS],
            sample_rate: 44100.0,
            fft_size: 2048,
        }
    }

    /// Pub-for-test inspector: current peak-hold envelope at bin `k`.
    #[doc(hidden)]
    pub fn peak_env_at(&self, k: usize) -> f32 {
        self.peak_env.get(k).copied().unwrap_or(0.0)
    }

    /// Map PEAK HOLD curve gain (linear, 1.0 = neutral) to hold time in ms.
    /// Log-scaled; range [1.0, 500.0] ms; curve=1.0 → 50 ms.
    #[inline]
    fn curve_to_hold_ms(curve: f32) -> f32 {
        let c = curve.clamp(0.0, 2.0);
        let log_min = 1.0f32.ln();
        let log_mid = 50.0f32.ln();
        let log_max = 500.0f32.ln();
        let log_t = if c <= 1.0 {
            log_min + (log_mid - log_min) * c
        } else {
            log_mid + (log_max - log_mid) * (c - 1.0)
        };
        log_t.exp()
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
        _ctx: &ModuleContext,
    ) {
        let n = bins.len();
        match self.mode {
            GainMode::Add => {
                for k in 0..n {
                    let g  = curves.get(0).and_then(|c| c.get(k)).copied().unwrap_or(1.0);
                    let sc = sidechain.and_then(|s| s.get(k)).copied().unwrap_or(0.0).max(0.0);
                    bins[k] *= g + sc;
                }
            }
            GainMode::Subtract => {
                for k in 0..n {
                    let g  = curves.get(0).and_then(|c| c.get(k)).copied().unwrap_or(1.0);
                    let sc = sidechain.and_then(|s| s.get(k)).copied().unwrap_or(0.0).max(0.0);
                    bins[k] *= (g - sc).max(0.0);
                }
            }
            GainMode::Pull => {
                let hop_ms = self.fft_size as f32 / (OVERLAP as f32 * self.sample_rate) * 1000.0;
                for k in 0..n {
                    let g = curves.get(0).and_then(|c| c.get(k)).copied()
                            .unwrap_or(1.0).clamp(0.0, 1.0);
                    let sc_mag_raw = sidechain.and_then(|s| s.get(k)).copied().unwrap_or(0.0).max(0.0);

                    let hold_curve = curves.get(1).and_then(|c| c.get(k)).copied().unwrap_or(1.0);
                    let hold_ms = Self::curve_to_hold_ms(hold_curve);
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
                }
            }
        }
        suppression_out.fill(0.0);
    }

    fn set_gain_mode(&mut self, mode: GainMode) { self.mode = mode; }

    fn module_type(&self) -> ModuleType { ModuleType::Gain }
    fn num_curves(&self) -> usize { 2 }
}
