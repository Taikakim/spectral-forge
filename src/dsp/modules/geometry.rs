//! Geometry module — 2-D-substrate-inspired spectral effects.
//!
//! Two light-CPU modes ship in v1:
//! - **Chladni Plate Nodes** — bins are projected onto a row-major (m,n) eigenmode
//!   grid; energy at antinodes is suppressed and redistributed to nodes (two-pass,
//!   conservative within ~5%).
//! - **Helmholtz Traps** — 8 fixed log-spaced bandpass traps absorb input energy
//!   into per-trap fill levels; on overflow (fill > threshold·capacity) energy
//!   re-injects at the trap centre + 2nd-harmonic overtone with phase-preserving
//!   magnitude scaling.
//!
//! Wavefield + Persistent Homology defer to Phase 7 (need SIMD wave kernel +
//! History Buffer infra respectively).

use num_complex::Complex;
use serde::{Deserialize, Serialize};

use crate::dsp::modules::{
    FxChannelTarget, ModuleContext, ModuleType, SpectralModule,
};
use crate::params::StereoLink;

pub const N_TRAPS:    usize = 8;
pub const GEO_GRID_W: usize = 128;
pub const GEO_GRID_H: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GeometryMode {
    Chladni,
    Helmholtz,
}

impl Default for GeometryMode {
    fn default() -> Self { GeometryMode::Chladni }
}

pub struct GeometryModule {
    mode:         GeometryMode,
    /// Per-channel cached |psi| buffer for Chladni's two-pass kernel.
    plate_phase:  [Vec<f32>; 2],
    /// Per-channel Helmholtz fill-level array.
    fill_level:   [[f32; N_TRAPS]; 2],
    /// Helmholtz trap centre bins (computed at reset).
    trap_centers: [usize; N_TRAPS],
    /// Helmholtz trap bandwidth in bins (computed at reset).
    trap_bw:      usize,
    sample_rate:  f32,
    fft_size:     usize,
}

impl GeometryModule {
    pub fn new() -> Self {
        Self {
            mode:         GeometryMode::default(),
            plate_phase:  [Vec::new(), Vec::new()],
            fill_level:   [[0.0; N_TRAPS]; 2],
            trap_centers: [0; N_TRAPS],
            trap_bw:      1,
            sample_rate:  48_000.0,
            fft_size:     2048,
        }
    }

    /// Test/UI helper.
    pub fn set_mode(&mut self, mode: GeometryMode) {
        if mode != self.mode {
            // Reset transient state on mode change.
            for ch in 0..2 { self.fill_level[ch] = [0.0; N_TRAPS]; }
            self.mode = mode;
        }
    }

    pub fn current_mode(&self) -> GeometryMode { self.mode }
}

impl SpectralModule for GeometryModule {
    fn process(
        &mut self,
        channel: usize,
        _stereo_link: StereoLink,
        _target: FxChannelTarget,
        _bins: &mut [Complex<f32>],
        _sidechain: Option<&[f32]>,
        _curves: &[&[f32]],
        suppression_out: &mut [f32],
        _ctx: &ModuleContext<'_>,
    ) {
        // v1 stub: clear suppression. Kernels arrive in Tasks 2e.3 and 2e.4.
        debug_assert!(channel < 2);
        for s in suppression_out.iter_mut() { *s = 0.0; }
    }

    fn reset(&mut self, sample_rate: f32, fft_size: usize) {
        self.sample_rate = sample_rate;
        self.fft_size    = fft_size;
        let num_bins = fft_size / 2 + 1;
        for ch in 0..2 {
            self.plate_phase[ch].clear();
            self.plate_phase[ch].resize(num_bins, 0.0);
            self.fill_level[ch] = [0.0; N_TRAPS];
        }
        // Log-spaced trap centres in [1, num_bins-1].
        let max = (num_bins - 1) as f32;
        for i in 0..N_TRAPS {
            let t   = (i as f32 + 0.5) / N_TRAPS as f32;
            let bin = max.powf(t).max(1.0);
            self.trap_centers[i] = (bin as usize).min(num_bins - 1);
        }
        self.trap_bw = (num_bins / 32).max(2);
        // self.mode is preserved across reset (user choice survives FFT-size change).
    }

    fn module_type(&self) -> ModuleType { ModuleType::Geometry }
    fn num_curves(&self) -> usize { 5 }
}
