//! Circuit module — analog circuit-inspired spectral distortion / saturation effects.
//!
//! Three modes ship across Phase 2g tasks:
//! - **BbdBins**             — 4-stage bucket-brigade delay on per-bin magnitudes + LP + dither.
//! - **SpectralSchmitt**     — branch-free hysteresis latch per FFT bin (Schmitt trigger).
//! - **CrossoverDistortion** — C¹-smooth deadzone mimicking BJT crossover artefacts.
//!
//! Kernel implementations are added in Tasks 3–5 of Phase 2g. This skeleton
//! provides the enum, struct, and stub `process()` that passes audio through
//! unmodified and zeroes suppression_out.

#[allow(unused_imports)]
use num_complex::Complex;
use serde::{Deserialize, Serialize};

use crate::dsp::modules::{
    FxChannelTarget, ModuleContext, ModuleType, SpectralModule,
};
#[allow(unused_imports)]
use crate::params::StereoLink;

// ── Constants ──────────────────────────────────────────────────────────────

pub const BBD_STAGES: usize = 4;

// ── CircuitMode ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CircuitMode {
    BbdBins,
    SpectralSchmitt,
    CrossoverDistortion,
}

impl Default for CircuitMode {
    fn default() -> Self {
        CircuitMode::CrossoverDistortion
    }
}

// ── CircuitModule ──────────────────────────────────────────────────────────

pub struct CircuitModule {
    mode: CircuitMode,
    bbd_mag: [[Vec<f32>; BBD_STAGES]; 2],   // bbd_mag[ch][stage][bin]
    schmitt_latched: [Vec<u8>; 2],           // packed bool per bin
    rng_state: [u32; 2],                     // xorshift32 per channel for BBD dither
    sample_rate: f32,
    fft_size: usize,
}

impl CircuitModule {
    pub fn new() -> Self {
        Self {
            mode: CircuitMode::default(),
            bbd_mag: [
                [Vec::new(), Vec::new(), Vec::new(), Vec::new()],
                [Vec::new(), Vec::new(), Vec::new(), Vec::new()],
            ],
            schmitt_latched: [Vec::new(), Vec::new()],
            rng_state: [0xDEAD_BEEFu32, 0xCAFE_BABEu32],
            sample_rate: 48_000.0,
            fft_size: 2048,
        }
    }

    pub fn set_mode(&mut self, mode: CircuitMode) {
        if mode != self.mode {
            self.mode = mode;
            // Reset transient kernel state on mode change so BBD/Schmitt do not leak between modes.
            for ch in 0..2 {
                for stage in 0..BBD_STAGES {
                    for v in self.bbd_mag[ch][stage].iter_mut() {
                        *v = 0.0;
                    }
                }
                for v in self.schmitt_latched[ch].iter_mut() {
                    *v = 0;
                }
            }
        }
    }

    pub fn current_mode(&self) -> CircuitMode {
        self.mode
    }
}

impl SpectralModule for CircuitModule {
    fn process(
        &mut self,
        channel: usize,
        _stereo_link: StereoLink,
        _target: FxChannelTarget,
        _bins: &mut [Complex<f32>],
        _sidechain: Option<&[f32]>,
        _curves: &[&[f32]],
        suppression_out: &mut [f32],
        _ctx: &ModuleContext,
    ) {
        debug_assert!(channel < 2);
        for s in suppression_out.iter_mut() {
            *s = 0.0;
        }
    }

    fn reset(&mut self, sample_rate: f32, fft_size: usize) {
        self.sample_rate = sample_rate;
        self.fft_size = fft_size;
        let num_bins = fft_size / 2 + 1;
        for ch in 0..2 {
            for stage in 0..BBD_STAGES {
                self.bbd_mag[ch][stage].clear();
                self.bbd_mag[ch][stage].resize(num_bins, 0.0);
            }
            self.schmitt_latched[ch].clear();
            self.schmitt_latched[ch].resize(num_bins, 0);
        }
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::Circuit
    }

    fn num_curves(&self) -> usize {
        4
    }
}
