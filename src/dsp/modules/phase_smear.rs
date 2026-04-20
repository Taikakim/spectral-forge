use num_complex::Complex;
use crate::params::{FxChannelTarget, StereoLink};
use crate::dsp::utils::xorshift64;
use super::{ModuleContext, ModuleType, SpectralModule};

pub struct PhaseSmearModule {
    rng_state: u64,
}

impl PhaseSmearModule {
    pub fn new() -> Self { Self { rng_state: 0x123456789abcdef0 } }


}

impl Default for PhaseSmearModule {
    fn default() -> Self { Self::new() }
}

impl SpectralModule for PhaseSmearModule {
    fn reset(&mut self, _sample_rate: f32, _fft_size: usize) {}

    fn process(
        &mut self,
        _channel: usize,
        _stereo_link: StereoLink,
        _target: FxChannelTarget,
        bins: &mut [Complex<f32>],
        _sidechain: Option<&[f32]>,
        curves: &[&[f32]],
        suppression_out: &mut [f32],
        _ctx: &ModuleContext,
    ) {
        if bins.is_empty() { suppression_out.fill(0.0); return; }
        let last = bins.len() - 1;
        for k in 0..bins.len() {
            let dry = bins[k];
            // Always advance PRNG to keep the sequence independent of skipping.
            let rand = xorshift64(&mut self.rng_state);
            // DC (k=0) and Nyquist (k=last) must stay real for IFFT correctness.
            if k == 0 || k == last { continue; }
            let per_bin    = curves.get(0).and_then(|c| c.get(k))
                                   .copied().unwrap_or(1.0).clamp(0.0, 2.0);
            let scale      = per_bin * std::f32::consts::PI;
            let rand_phase = (rand as f32 / u64::MAX as f32 * 2.0 - 1.0) * scale;
            let (mag, phase) = (bins[k].norm(), bins[k].arg());
            let wet = Complex::from_polar(mag, phase + rand_phase);
            let mix = curves.get(2).and_then(|c| c.get(k)).copied().unwrap_or(1.0).clamp(0.0, 1.0);
            bins[k] = Complex::new(
                dry.re * (1.0 - mix) + wet.re * mix,
                dry.im * (1.0 - mix) + wet.im * mix,
            );
        }
        suppression_out.fill(0.0);
    }

    fn module_type(&self) -> ModuleType { ModuleType::PhaseSmear }
    fn num_curves(&self) -> usize { 3 }
}
