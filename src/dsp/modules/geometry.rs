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

fn apply_chladni(
    bins: &mut [Complex<f32>],
    plate_phase: &mut [f32],
    grid_w: usize,
    grid_h: usize,
    curves: &[&[f32]],
) {
    use std::f32::consts::PI;

    let amount_c  = curves[0];   // AMOUNT — settle force
    let mode_c    = curves[1];   // MODE_CAP — picks (m, n) eigenmode
    let damping_c = curves[2];   // DAMP_REL — extra magnitude bleed
    let mix_c     = curves[4];   // MIX — dry/wet blend

    let num_bins = bins.len();
    let lx = grid_w as f32;
    let ly = grid_h as f32;

    // Pass 1: compute |psi| per bin and accumulate suppressed energy + node weight.
    let mut total_suppressed = 0.0_f32;
    let mut total_node_weight = 0.0_f32;
    for k in 0..num_bins {
        let mode_g = mode_c.get(k).copied().unwrap_or(1.0).clamp(0.0, 2.0);
        // Map mode_g ∈ [0, 2] → (m, n) ∈ ([1..6], [1..4]).
        let m = (1.0 + mode_g * 2.5) as usize;
        let n = (1.0 + mode_g * 1.5) as usize;
        let m = m.clamp(1, 6);
        let n = n.clamp(1, 4);

        // Row-major projection of bin index onto a (grid_w × grid_h) plate.
        let x = (k % grid_w) as f32 / lx;
        let y = ((k / grid_w) % grid_h) as f32 / ly;
        let psi = (m as f32 * PI * x).sin().abs() * (n as f32 * PI * y).sin().abs();
        plate_phase[k] = psi;

        let amt = (amount_c.get(k).copied().unwrap_or(0.0) * 0.025).clamp(0.0, 0.05);
        let mag = bins[k].norm();
        total_suppressed  += mag * amt * psi;
        total_node_weight += 1.0 - psi;
    }

    let inv_node = 1.0 / total_node_weight.max(1e-9);

    // Pass 2: suppress at antinodes, redistribute to nodes, blend dry/wet.
    for k in 0..num_bins {
        let psi    = plate_phase[k];
        let amt    = (amount_c.get(k).copied().unwrap_or(0.0) * 0.025).clamp(0.0, 0.05);
        let damp   = (damping_c.get(k).copied().unwrap_or(0.0) * 0.01).clamp(0.0, 0.02);
        let mix    = mix_c.get(k).copied().unwrap_or(0.0).clamp(0.0, 2.0) * 0.5;
        let suppress = amt * psi;
        let inject   = total_suppressed * (1.0 - psi) * inv_node;

        let mag = bins[k].norm();
        let new_mag = (mag * (1.0 - suppress - damp) + inject).max(0.0);
        let scale = new_mag / mag.max(1e-9);
        let dry = bins[k];
        let wet = bins[k] * scale;
        bins[k] = dry * (1.0 - mix) + wet * mix;
    }
}

impl SpectralModule for GeometryModule {
    fn process(
        &mut self,
        channel: usize,
        _stereo_link: StereoLink,
        _target: FxChannelTarget,
        bins: &mut [Complex<f32>],
        _sidechain: Option<&[f32]>,
        curves: &[&[f32]],
        suppression_out: &mut [f32],
        ctx: &ModuleContext<'_>,
    ) {
        debug_assert!(channel < 2);
        debug_assert_eq!(bins.len(), ctx.num_bins);
        debug_assert!(curves.len() >= 5, "Geometry needs 5 curves: AMOUNT/MODE_CAP/DAMP_REL/THRESH/MIX");

        match self.mode {
            GeometryMode::Chladni => {
                let plate_phase = &mut self.plate_phase[channel];
                apply_chladni(bins, plate_phase, GEO_GRID_W, GEO_GRID_H, curves);
            }
            GeometryMode::Helmholtz => {
                // Filled in Task 2e.4.
            }
        }

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
