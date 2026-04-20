use num_complex::Complex;
use crate::params::{FxChannelTarget, StereoLink};
use super::{GainMode, ModuleContext, ModuleType, SpectralModule};

pub struct GainModule {
    pub(crate) mode: GainMode,
}

impl GainModule {
    pub fn new() -> Self { Self { mode: GainMode::Add } }
}

impl Default for GainModule {
    fn default() -> Self { Self::new() }
}

impl SpectralModule for GainModule {
    fn reset(&mut self, _sample_rate: f32, _fft_size: usize) {}

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
                for k in 0..n {
                    let g      = curves.get(0).and_then(|c| c.get(k)).copied()
                                       .unwrap_or(1.0).clamp(0.0, 1.0);
                    let sc_mag = sidechain.and_then(|s| s.get(k)).copied().unwrap_or(0.0);
                    let cur_mag = bins[k].norm();
                    if cur_mag > 1e-10 {
                        let target_mag = cur_mag * g + sc_mag * (1.0 - g);
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
