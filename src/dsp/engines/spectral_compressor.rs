use num_complex::Complex;
use super::{SpectralEngine, BinParams};

#[derive(Default)]
pub struct SpectralCompressorEngine;

impl SpectralCompressorEngine {
    pub fn new() -> Self { Self::default() }
}

impl SpectralEngine for SpectralCompressorEngine {
    fn reset(&mut self, _sample_rate: f32, _fft_size: usize) {}

    fn process_bins(
        &mut self,
        _bins: &mut [Complex<f32>],
        _sidechain: Option<&[f32]>,
        _params: &BinParams<'_>,
        _sample_rate: f32,
        suppression_out: &mut [f32],
    ) {
        suppression_out.fill(0.0);
    }

    fn name(&self) -> &'static str { "Spectral Compressor" }
}
