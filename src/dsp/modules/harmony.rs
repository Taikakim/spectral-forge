use num_complex::Complex;
use crate::params::{FxChannelTarget, StereoLink};
use super::{ModuleContext, ModuleType, SpectralModule};

pub struct HarmonyModule;

impl HarmonyModule {
    pub fn new() -> Self { Self }
}

impl SpectralModule for HarmonyModule {
    fn reset(&mut self, _sample_rate: f32, _fft_size: usize) {}

    fn process(
        &mut self,
        _channel: usize,
        _stereo_link: StereoLink,
        _target: FxChannelTarget,
        _bins: &mut [Complex<f32>],
        _sidechain: Option<&[f32]>,
        _curves: &[&[f32]],
        suppression_out: &mut [f32],
        _physics: Option<&mut crate::dsp::bin_physics::BinPhysics>,
        _ctx: &ModuleContext<'_>,
    ) {
        suppression_out.fill(0.0);
    }

    fn module_type(&self) -> ModuleType { ModuleType::Harmony }
    fn num_curves(&self) -> usize { 6 }
}
