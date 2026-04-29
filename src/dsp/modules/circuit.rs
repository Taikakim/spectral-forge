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
/// Curves: `[AMOUNT, THRESH, SPREAD(unused), RELEASE, MIX]`.
fn apply_bbd(
    bins: &mut [Complex<f32>],
    bbd_mag: &mut [Vec<f32>; BBD_STAGES],
    rng_state: &mut u32,
    curves: &[&[f32]],
) {
    let amount_c = curves[0];
    let thresh_c = curves[1];
    // curves[2] = SPREAD — reserved for Phase 5c.8, unused by v1 BBD kernel.
    let release_c = curves[3];
    let mix_c = curves[4];

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
/// Curves: `[AMOUNT, THRESH, SPREAD(unused), RELEASE, MIX]`.
fn apply_schmitt(
    bins: &mut [Complex<f32>],
    latched: &mut [u8],
    curves: &[&[f32]],
) {
    let amount_c = curves[0];
    let thresh_c = curves[1];
    // curves[2] = SPREAD — reserved for Phase 5c.8, unused by v1 Schmitt kernel.
    let release_c = curves[3];
    let mix_c = curves[4];

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
/// Curves: `[AMOUNT, THRESH(unused), SPREAD(unused), RELEASE(unused), MIX]`.
fn apply_crossover(bins: &mut [Complex<f32>], curves: &[&[f32]]) {
    let amount_c = curves[0];
    // curves[1] = THRESH, curves[3] = RELEASE — unused by v1 Crossover kernel.
    // curves[2] = SPREAD — reserved for Phase 5c.8 (PCB Crosstalk).
    let mix_c = curves[4];

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

// ── Vactrol helpers ────────────────────────────────────────────────────────

/// Nominal time constants for the opto-coupler photocell model (seconds).
const VACTROL_TAU_FAST: f32 = 0.008;  // 8 ms
const VACTROL_TAU_SLOW: f32 = 0.250;  // 250 ms

/// Cascaded 2-pole vactrol-style photocell per-bin.
///
/// Drive charges the fast cap; the fast cap drives the slow cap. Cell gain
/// `g = tanh(slow)` soft-saturates into `[0, 1)` — applied as a multiplicative
/// gain on the bin (passive opto-coupler model: g attenuates, never amplifies).
///
/// `flux` — optional per-bin flux from BinPhysics. When `Some`, each bin's
/// drive is `flux[k] * amount` instead of `in_mag * amount`.
///
/// Curves: `[AMOUNT, THRESH(unused), SPREAD(unused), RELEASE, MIX]`.
fn apply_vactrol(
    bins: &mut [Complex<f32>],
    fast: &mut [f32],
    slow: &mut [f32],
    curves: &[&[f32]],
    hop_dt: f32,
    flux: Option<&[f32]>,
) {
    use crate::dsp::circuit_kernels::lp_step;

    let amount_c  = curves[0];
    // curves[1] = THRESH — unused by Vactrol v1.
    // curves[2] = SPREAD — unused by Vactrol v1.
    let release_c = curves[3];
    let mix_c     = curves[4];

    let num_bins = bins.len();

    for k in 0..num_bins {
        let amount  = amount_c[k].clamp(0.0, 2.0);
        let rel_scl = release_c[k].clamp(0.01, 4.0);   // user scale on both τ
        let mix     = mix_c[k].clamp(0.0, 2.0) * 0.5;  // 0..1

        let tau_fast = VACTROL_TAU_FAST * rel_scl;
        let tau_slow = VACTROL_TAU_SLOW * rel_scl;

        // α = hop_dt / τ, clamped to [0, 1].
        let alpha_fast = (hop_dt / tau_fast).min(1.0);
        let alpha_slow = (hop_dt / tau_slow).min(1.0);

        let dry = bins[k];

        // Drive: flux[k] when upstream BinPhysics is present, else magnitude.
        let raw_drive = match flux {
            Some(f) => f[k].abs(),
            None    => dry.norm(),
        };
        let drive = raw_drive * amount;

        // Charge fast cap toward drive, then charge slow cap toward fast.
        lp_step(&mut fast[k], drive, alpha_fast);
        lp_step(&mut slow[k], fast[k], alpha_slow);

        // Soft-saturating cell gain via tanh: `g ∈ [0, 1)` for non-negative slow cap.
        // tanh(1.0) ≈ 0.762, so a fully-charged cap on a unit-amplitude bin yields
        // ~0.76× passthrough — the photocell is a passive divider, never gains above 1.
        let g = crate::dsp::circuit_kernels::tanh_levien_poly(slow[k]).max(0.0);
        let wet = dry * g;

        bins[k] = dry * (1.0 - mix) + wet * mix;
    }
}

// ── CircuitMode ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CircuitMode {
    BbdBins,
    SpectralSchmitt,
    CrossoverDistortion,
    Vactrol,
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
    // Vactrol state: per-channel, per-bin fast/slow 1-pole caps.
    vactrol_fast: [Vec<f32>; 2],
    vactrol_slow: [Vec<f32>; 2],
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
            vactrol_fast: [Vec::new(), Vec::new()],
            vactrol_slow: [Vec::new(), Vec::new()],
            sample_rate: 48_000.0,
            fft_size: 2048,
            #[cfg(any(test, feature = "probe"))]
            last_probe: crate::dsp::modules::ProbeSnapshot::default(),
        }
    }

    pub fn set_mode(&mut self, mode: CircuitMode) {
        if mode != self.mode {
            self.mode = mode;
            // Reset transient kernel state on mode change so kernels do not leak between modes.
            for ch in 0..2 {
                for stage in 0..BBD_STAGES {
                    for v in self.bbd_mag[ch][stage].iter_mut() {
                        *v = 0.0;
                    }
                }
                for v in self.schmitt_latched[ch].iter_mut() {
                    *v = 0;
                }
                for v in self.vactrol_fast[ch].iter_mut() {
                    *v = 0.0;
                }
                for v in self.vactrol_slow[ch].iter_mut() {
                    *v = 0.0;
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
        physics: Option<&mut crate::dsp::bin_physics::BinPhysics>,
        ctx: &ModuleContext,
    ) {
        debug_assert!(channel < 2);

        // Probe capture: all three kernels share the same mapping for curves[0] and curves[4].
        // curves[0] (AMOUNT): g=1.0 → 50%, g=2.0 → 100%  (g.clamp(0,2) * 50.0)
        // curves[4] (MIX):   g=1.0 → 50%, g=2.0 → 100%  (g.clamp(0,2) * 50.0)
        #[cfg(any(test, feature = "probe"))]
        let probe_amount_pct = curves[0].get(0).copied().unwrap_or(0.0).clamp(0.0, 2.0) * 50.0;
        #[cfg(any(test, feature = "probe"))]
        let probe_mix_pct = curves[4].get(0).copied().unwrap_or(0.0).clamp(0.0, 2.0) * 50.0;

        // Compute hop duration in seconds (variable FFT size).
        let hop_dt = (ctx.fft_size / 4) as f32 / ctx.sample_rate;

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
            CircuitMode::Vactrol => {
                // Read upstream flux via the writer-slot's mixed `physics`, not
                // `ctx.bin_physics`: Circuit declares `writes_bin_physics: true`,
                // so FxMatrix passes `physics = Some(&mut mix_phys)` and leaves
                // `ctx.bin_physics = None`. Vactrol does not write — `physics` is
                // read-only here.
                let flux: Option<&[f32]> = physics.as_ref().and_then(|bp| {
                    let f = &bp.flux[..];
                    if f.len() >= bins.len() { Some(&f[..bins.len()]) } else { None }
                });
                let fast = &mut self.vactrol_fast[channel];
                let slow = &mut self.vactrol_slow[channel];
                apply_vactrol(bins, fast, slow, curves, hop_dt, flux);
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
            self.vactrol_fast[ch].clear();
            self.vactrol_fast[ch].resize(num_bins, 0.0);
            self.vactrol_slow[ch].clear();
            self.vactrol_slow[ch].resize(num_bins, 0.0);
        }
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::Circuit
    }

    fn set_circuit_mode(&mut self, mode: CircuitMode) {
        self.set_mode(mode);
    }

    fn num_curves(&self) -> usize {
        5
    }

    #[cfg(any(test, feature = "probe"))]
    fn last_probe(&self) -> crate::dsp::modules::ProbeSnapshot { self.last_probe }
}

// ── CircuitProbe (test / probe builds only) ────────────────────────────────

/// Per-module probe snapshot for Circuit. Returned by `probe_state()`.
/// Fields are kept minimal for Phase 5c.4; later tasks add more.
#[cfg(any(test, feature = "probe"))]
#[derive(Debug, Clone, Copy)]
pub struct CircuitProbe {
    pub active_mode:       CircuitMode,
    pub vactrol_fast_avg:  f32,
    pub vactrol_slow_avg:  f32,
}

#[cfg(any(test, feature = "probe"))]
impl CircuitModule {
    pub fn probe_state(&self, channel: usize) -> CircuitProbe {
        let ch = channel.min(1);
        let (fa, sa) = if self.mode == CircuitMode::Vactrol && !self.vactrol_slow[ch].is_empty() {
            let n = self.vactrol_slow[ch].len() as f32;
            let fa: f32 = self.vactrol_fast[ch].iter().sum::<f32>() / n;
            let sa: f32 = self.vactrol_slow[ch].iter().sum::<f32>() / n;
            (fa, sa)
        } else {
            (0.0, 0.0)
        };
        CircuitProbe {
            active_mode:      self.mode,
            vactrol_fast_avg: fa,
            vactrol_slow_avg: sa,
        }
    }
}
