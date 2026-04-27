use num_complex::Complex;
use crate::params::{FxChannelTarget, StereoLink};
use super::{ModuleContext, ModuleType, SpectralModule};

pub struct MasterModule;
impl SpectralModule for MasterModule {
    fn reset(&mut self, _: f32, _: usize) {}
    fn process(
        &mut self, _: usize, _: StereoLink, _: FxChannelTarget,
        _: &mut [Complex<f32>], _: Option<&[f32]>, _: &[&[f32]],
        suppression_out: &mut [f32], _physics: Option<&mut crate::dsp::bin_physics::BinPhysics>,
        _: &ModuleContext<'_>,
    ) { suppression_out.fill(0.0); }
    fn module_type(&self) -> ModuleType { ModuleType::Master }
    fn num_curves(&self) -> usize { 0 }
}

/// Zero-cost placeholder for an unoccupied slot. Returns `ModuleType::Empty`
/// from `module_type()` so callers can distinguish empty slots from the Master
/// output bus without special-casing a `MasterModule` instance.
pub struct EmptyModule;
impl SpectralModule for EmptyModule {
    fn reset(&mut self, _: f32, _: usize) {}
    fn process(
        &mut self, _: usize, _: StereoLink, _: FxChannelTarget,
        _: &mut [Complex<f32>], _: Option<&[f32]>, _: &[&[f32]],
        suppression_out: &mut [f32], _physics: Option<&mut crate::dsp::bin_physics::BinPhysics>,
        _: &ModuleContext<'_>,
    ) { suppression_out.fill(0.0); }
    fn module_type(&self) -> ModuleType { ModuleType::Empty }
    fn num_curves(&self) -> usize { 0 }
}
