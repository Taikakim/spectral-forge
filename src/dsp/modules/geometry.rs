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
    #[cfg(any(test, feature = "probe"))]
    last_probe:   crate::dsp::modules::ProbeSnapshot,
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
            #[cfg(any(test, feature = "probe"))]
            last_probe:   crate::dsp::modules::ProbeSnapshot::default(),
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

fn apply_helmholtz(
    bins: &mut [Complex<f32>],
    fill_level: &mut [f32; N_TRAPS],
    trap_centers: &[usize; N_TRAPS],
    trap_bw: usize,
    curves: &[&[f32]],
) {
    let amount_c    = curves[0];
    let capacity_c  = curves[1];
    let release_c   = curves[2];
    let threshold_c = curves[3];
    let mix_c       = curves[4];

    let num_bins = bins.len();
    let half_bw  = trap_bw / 2;

    for k in 0..N_TRAPS {
        let center = trap_centers[k];
        if center == 0 || center >= num_bins {
            continue;
        }

        let amount = (amount_c[center] * 0.5).clamp(0.0, 1.0);
        if amount < 0.01 {
            // Trap inactive: drain residual fill so it doesn't leak across reactivations.
            fill_level[k] *= 0.95;
            continue;
        }

        let capacity  = capacity_c[center].clamp(0.1, 4.0);
        let release   = (release_c[center] * 0.2).clamp(0.0, 0.5);
        let threshold = (threshold_c[center] * 0.5).clamp(0.1, 1.5);
        let mix       = (mix_c[center].clamp(0.0, 2.0)) * 0.5;

        // Bandwidth window.
        let lo = center.saturating_sub(half_bw);
        let hi = (center + half_bw).min(num_bins - 1);

        // Sum input energy in band.
        let mut input_energy = 0.0_f32;
        for b in lo..=hi {
            input_energy += bins[b].norm();
        }

        // Absorb a fraction into fill_level.
        fill_level[k] += amount * input_energy;

        // Soft notch: attenuate band by (1 - amount * mix).
        let attenuate = 1.0 - amount * mix;
        for b in lo..=hi {
            let mag     = bins[b].norm();
            let new_mag = mag * attenuate;
            let scale   = new_mag / mag.max(1e-9);
            bins[b] *= scale;
        }

        // Overflow check: phase-preserving magnitude scaling at overtone.
        // The resonant cavity re-radiates at the 2nd-harmonic overtone (not the center,
        // which lies inside the absorption band and would just get re-absorbed).
        // If the target bin is near-zero, inject additively as a real component
        // (phase is undefined at zero magnitude, so pure-real is as valid as any choice).
        let trigger = threshold * capacity;
        if fill_level[k] > trigger {
            let overflow   = fill_level[k] - trigger;
            let inject_amt = overflow * release;
            // 2nd-harmonic overtone: phase-preserving magnitude injection.
            let overtone = (center * 2).min(num_bins - 1);
            let cur_o    = bins[overtone].norm();
            if cur_o > 1e-9 {
                bins[overtone] *= (cur_o + inject_amt) / cur_o;
            } else {
                bins[overtone] = Complex::new(inject_amt, 0.0);
            }
            fill_level[k] -= inject_amt;
        } else {
            // Drain when below threshold.
            fill_level[k] *= 1.0 - release;
        }
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

        #[cfg(any(test, feature = "probe"))]
        let mut probe_amount_pct = 0.0_f32;
        #[cfg(any(test, feature = "probe"))]
        let mut probe_mix_pct = 0.0_f32;

        match self.mode {
            GeometryMode::Chladni => {
                let plate_phase = &mut self.plate_phase[channel];
                apply_chladni(bins, plate_phase, GEO_GRID_W, GEO_GRID_H, curves);
                #[cfg(any(test, feature = "probe"))]
                {
                    let amount_g = curves[0].get(0).copied().unwrap_or(0.0);
                    let mix_g    = curves[4].get(0).copied().unwrap_or(0.0);
                    // AMOUNT: (g * 0.025).clamp(0.0, 0.05) → range 0..0.05 → pct = (val/0.05)*100
                    let amt_val = (amount_g * 0.025).clamp(0.0, 0.05);
                    let mix_val = mix_g.clamp(0.0, 2.0) * 0.5;
                    probe_amount_pct = (amt_val / 0.05) * 100.0;
                    probe_mix_pct    = mix_val * 100.0;
                }
            }
            GeometryMode::Helmholtz => {
                let fill_level = &mut self.fill_level[channel];
                apply_helmholtz(
                    bins,
                    fill_level,
                    &self.trap_centers,
                    self.trap_bw,
                    curves,
                );
                #[cfg(any(test, feature = "probe"))]
                {
                    let amount_g = curves[0].get(0).copied().unwrap_or(0.0);
                    let mix_g    = curves[4].get(0).copied().unwrap_or(0.0);
                    // AMOUNT: (g * 0.5).clamp(0.0, 1.0) → range 0..1.0 → pct = val * 100
                    let amt_val = (amount_g * 0.5).clamp(0.0, 1.0);
                    let mix_val = mix_g.clamp(0.0, 2.0) * 0.5;
                    probe_amount_pct = amt_val * 100.0;
                    probe_mix_pct    = mix_val * 100.0;
                }
            }
        }

        for s in suppression_out.iter_mut() { *s = 0.0; }

        #[cfg(any(test, feature = "probe"))]
        {
            self.last_probe = crate::dsp::modules::ProbeSnapshot {
                amount_pct: Some(probe_amount_pct),
                mix_pct:    Some(probe_mix_pct),
                ..Default::default()
            };
        }
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

    fn set_geometry_mode(&mut self, mode: GeometryMode) {
        self.set_mode(mode);
    }

    #[cfg(any(test, feature = "probe"))]
    fn last_probe(&self) -> crate::dsp::modules::ProbeSnapshot { self.last_probe }
}
