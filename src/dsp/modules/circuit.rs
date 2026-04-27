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

use num_complex::Complex;
use serde::{Deserialize, Serialize};

use crate::dsp::modules::{
    FxChannelTarget, ModuleContext, ModuleType, SpectralModule,
};
use crate::params::StereoLink;

// ── Constants ──────────────────────────────────────────────────────────────

pub const BBD_STAGES: usize = 4;

// ── BBD helpers ────────────────────────────────────────────────────────────

/// Xorshift32 PRNG step — returns a value in `[-1, 1)`.
fn xorshift32_step(state: &mut u32) -> f32 {
    let mut s = *state;
    s ^= s << 13;
    s ^= s >> 17;
    s ^= s << 5;
    *state = s;
    (s as i32 as f32) / (i32::MAX as f32)
}

/// 4-stage bucket-brigade delay on per-bin magnitudes.
/// Curves: `[AMOUNT, THRESH, RELEASE, MIX]`.
fn apply_bbd(
    bins: &mut [Complex<f32>],
    bbd_mag: &mut [Vec<f32>; BBD_STAGES],
    rng_state: &mut u32,
    curves: &[&[f32]],
) {
    let amount_c = curves[0];
    let thresh_c = curves[1];
    let release_c = curves[2];
    let mix_c = curves[3];

    let num_bins = bins.len();

    for k in 0..num_bins {
        let amount = amount_c[k].clamp(0.0, 2.0) * 0.5; // 0..1 stage-3 output gain
        let dither_amt = thresh_c[k].clamp(0.0, 2.0) * 0.005; // very small noise
        let lp_alpha = (release_c[k].clamp(0.01, 2.0) * 0.4).clamp(0.05, 0.9);
        let mix = mix_c[k].clamp(0.0, 2.0) * 0.5;

        let dry = bins[k];
        let in_mag = dry.norm();

        // Push input into stage 0 (with LP smoothing toward target).
        let target_0 = bbd_mag[0][k] + (in_mag - bbd_mag[0][k]) * lp_alpha;
        let dither_0 = xorshift32_step(rng_state) * dither_amt;
        bbd_mag[0][k] = (target_0 + dither_0).max(0.0);

        // Cascade: each stage LP-smooths toward the previous stage's value.
        // Read s0 from the just-written stage 0 (intentional — see plan §note),
        // then read old stages 1/2/3 before overwriting them.
        let s0_prev = bbd_mag[0][k]; // intentionally the NEW stage-0 value
        let s1_prev = bbd_mag[1][k];
        let s2_prev = bbd_mag[2][k];
        let s3_prev = bbd_mag[3][k];

        bbd_mag[3][k] = s3_prev + (s2_prev - s3_prev) * lp_alpha + xorshift32_step(rng_state) * dither_amt;
        bbd_mag[2][k] = s2_prev + (s1_prev - s2_prev) * lp_alpha + xorshift32_step(rng_state) * dither_amt;
        bbd_mag[1][k] = s1_prev + (s0_prev - s1_prev) * lp_alpha + xorshift32_step(rng_state) * dither_amt;

        // Output: stage 3 (most-delayed) magnitude, scaled by amount.
        // Phase is preserved when there is a live carrier (in_mag > 1e-9); for silent
        // input bins we emit the delayed magnitude as real-positive (arbitrary unit phase).
        let out_mag = bbd_mag[3][k].max(0.0) * amount;
        let wet = if in_mag > 1e-9 {
            dry * (out_mag / in_mag)
        } else {
            Complex::new(out_mag, 0.0)
        };
        bins[k] = dry * (1.0 - mix) + wet * mix;
    }
}

// ── Schmitt helpers ────────────────────────────────────────────────────────

/// Per-bin hysteresis latch (Schmitt trigger).
/// Curves: `[AMOUNT, THRESH, RELEASE, MIX]`.
fn apply_schmitt(
    bins: &mut [Complex<f32>],
    latched: &mut [u8],
    curves: &[&[f32]],
) {
    let amount_c = curves[0];
    let thresh_c = curves[1];
    let release_c = curves[2];
    let mix_c = curves[3];

    let num_bins = bins.len();

    for k in 0..num_bins {
        let attenuation = amount_c[k].clamp(0.0, 2.0) * 0.5;          // 0..1 attenuation when OFF
        let high = thresh_c[k].clamp(0.01, 4.0);
        let gap = (release_c[k].clamp(0.0, 2.0) * 0.5).clamp(0.05, 0.95);
        let low = high * (1.0 - gap);
        let mix = mix_c[k].clamp(0.0, 2.0) * 0.5;

        let mag = bins[k].norm();
        let was_latched = latched[k] != 0;
        let now_latched = if was_latched { mag > low } else { mag > high };
        latched[k] = if now_latched { 1 } else { 0 };

        let attenuate = if now_latched { 1.0 } else { 1.0 - attenuation };
        let dry = bins[k];
        let wet = dry * attenuate;
        bins[k] = dry * (1.0 - mix) + wet * mix;
    }
}

// ── Crossover helpers ──────────────────────────────────────────────────────

/// C¹-smooth deadzone mimicking BJT class-B crossover artefacts.
/// Bins with magnitude ≤ dz_width are silenced; above the deadzone,
/// output follows `(mag - dz)² / mag`, which is continuous and has a
/// continuous first derivative at the boundary (no audible click).
/// Phase is preserved by scaling the original complex bin.
/// Curves: `[AMOUNT, THRESH(unused), RELEASE(unused), MIX]`.
fn apply_crossover(bins: &mut [Complex<f32>], curves: &[&[f32]]) {
    let amount_c = curves[0];
    let mix_c = curves[3];

    let num_bins = bins.len();

    for k in 0..num_bins {
        let dz_width = amount_c[k].clamp(0.0, 2.0) * 0.1; // up to 0.2 deadzone half-width
        let mix = mix_c[k].clamp(0.0, 2.0) * 0.5;

        let dry = bins[k];
        let mag = dry.norm();

        let new_mag = if mag <= dz_width {
            0.0
        } else {
            let excess = mag - dz_width;
            (excess * excess) / mag
        };

        let scale = if mag > 1e-9 { new_mag / mag } else { 0.0 };
        let wet = dry * scale;
        bins[k] = dry * (1.0 - mix) + wet * mix;
    }
}

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
    #[cfg(any(test, feature = "probe"))]
    last_probe: crate::dsp::modules::ProbeSnapshot,
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
            #[cfg(any(test, feature = "probe"))]
            last_probe: crate::dsp::modules::ProbeSnapshot::default(),
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
        bins: &mut [Complex<f32>],
        _sidechain: Option<&[f32]>,
        curves: &[&[f32]],
        suppression_out: &mut [f32],
        _ctx: &ModuleContext,
    ) {
        debug_assert!(channel < 2);

        // Probe capture: all three kernels share the same mapping for curves[0] and curves[3].
        // curves[0] (AMOUNT): g=1.0 → 50%, g=2.0 → 100%  (g.clamp(0,2) * 50.0)
        // curves[3] (MIX):   g=1.0 → 50%, g=2.0 → 100%  (g.clamp(0,2) * 50.0)
        #[cfg(any(test, feature = "probe"))]
        let probe_amount_pct = curves[0].get(0).copied().unwrap_or(0.0).clamp(0.0, 2.0) * 50.0;
        #[cfg(any(test, feature = "probe"))]
        let probe_mix_pct = curves[3].get(0).copied().unwrap_or(0.0).clamp(0.0, 2.0) * 50.0;

        match self.mode {
            CircuitMode::BbdBins => {
                let bbd = &mut self.bbd_mag[channel];
                let rng = &mut self.rng_state[channel];
                apply_bbd(bins, bbd, rng, curves);
            }
            CircuitMode::SpectralSchmitt => {
                let latched = &mut self.schmitt_latched[channel];
                apply_schmitt(bins, latched, curves);
            }
            CircuitMode::CrossoverDistortion => {
                apply_crossover(bins, curves);
            }
        }

        for s in suppression_out.iter_mut() {
            *s = 0.0;
        }

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

    fn set_circuit_mode(&mut self, mode: CircuitMode) {
        self.set_mode(mode);
    }

    fn num_curves(&self) -> usize {
        4
    }

    #[cfg(any(test, feature = "probe"))]
    fn last_probe(&self) -> crate::dsp::modules::ProbeSnapshot { self.last_probe }
}
