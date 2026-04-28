//! Life module — fluid/physical-metaphor spectral processing.
//!
//! Ten modes ship across Phase 5a tasks:
//! - **Viscosity**       — FTCS finite-volume diffusion (Task 3)
//! - **SurfaceTension**  — adjacent peak coalescence (Task 4)
//! - **Crystallization** — sustain-driven phase lock + BinPhysics write (Task 5)
//! - **Archimedes**      — volume-conserving global ducking (Task 6)
//! - **NonNewtonian**    — rate-limit transients (Task 7)
//! - **Stiction**        — static/kinetic friction (Task 8)
//! - **Yield**           — fabric tearing with phase scramble (Task 9)
//! - **Capillary**       — upward harmonic wicking, two-pass (Task 10)
//! - **Sandpaper**       — granular phase friction sparks (Task 11)
//! - **Brownian**        — temperature-driven random walk (Task 12)
//!
//! Task 3 (Viscosity) is implemented; Tasks 4–12 land per-mode kernels via
//! the `match self.mode` dispatch in `process()`.

use num_complex::Complex;
use serde::{Deserialize, Serialize};

use crate::dsp::modules::{
    FxChannelTarget, ModuleContext, ModuleType, SpectralModule, StereoLink,
};

// ── Constants ──────────────────────────────────────────────────────────────

/// FTCS diffusion stability clamp (research finding from ideas/.../11-life.md).
/// Used by the Viscosity kernel (Task 3).
const VISCOSITY_D_MAX: f32 = 0.45;

/// ~50ms time-constant LP alpha at 48k/256-hop.
/// Used by the Capillary (Task 10) and Crystallization (Task 5) kernels.
const SUSTAIN_LP_ALPHA: f32 = 0.05;

/// Surface Tension max steal fraction per hop (5%). Conservative cap so even the
/// max-amount, max-reach case can't drain a neighbour in a single hop.
const SURFACE_TENSION_AMT_MAX: f32 = 0.05;

/// Surface Tension max reach in bins. Curve range maps `[0, 2]` → `[0, 8]`.
const SURFACE_TENSION_REACH_MAX: i32 = 8;

/// Archimedes — minimum residual signal kept after ducking (5%). Even at max
/// overflow × max amount, the wet path keeps at least this fraction of the dry
/// signal, so an out-of-control overflow can never null the bus.
const ARCHIMEDES_DUCK_FLOOR: f32 = 0.05;

/// Archimedes — guard against zero-capacity divide. If `avg_thresh` collapses
/// to 0, capacity floors here (much looser than VISCOSITY's 1e-12 because this
/// guards a divisor in the *transport ratio*, not a magnitude comparison).
const ARCHIMEDES_CAPACITY_FLOOR: f32 = 1e-6;

/// Non-Newtonian — accumulated displacement cap. Bounds growth during long
/// transient streaks so downstream Stiction/Yield see a finite value. Stiction
/// clamps its own input to ~1.0 anyway, but capping here prevents the field
/// from drifting unbounded across hours of audio.
const NON_NEWTONIAN_DISPLACEMENT_CAP: f32 = 10.0;

/// Stiction — minimum hop-to-hop decay of `is_moving`. Even at speed=0 a bin
/// re-locks within ~20 hops after velocity drops below threshold.
const STICTION_DECAY_MIN: f32 = 0.05;

/// Stiction — additional decay range scaled by speed curve. At speed=1 total
/// decay per hop is 0.5, so a bin re-locks in ~2 hops.
const STICTION_DECAY_RANGE: f32 = 0.45;

// ── LifeMode ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LifeMode {
    Viscosity,
    SurfaceTension,
    Crystallization,
    Archimedes,
    NonNewtonian,
    Stiction,
    Yield,
    Capillary,
    Sandpaper,
    Brownian,
}

impl Default for LifeMode {
    fn default() -> Self {
        LifeMode::Viscosity
    }
}

// ── PRNG helpers ───────────────────────────────────────────────────────────

/// Xorshift32 PRNG step — returns the raw state value.
/// Used by the Yield (Task 9) and Brownian (Task 12) kernels.
#[allow(dead_code)]
#[inline]
fn xorshift32_step(state: &mut u32) -> u32 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *state = x;
    x
}

/// Xorshift32 PRNG step mapped to `[-1.0, 1.0)`.
/// Used by the Yield (Task 9) and Brownian (Task 12) kernels.
#[allow(dead_code)]
#[inline]
fn xorshift32_signed_unit(state: &mut u32) -> f32 {
    let u = xorshift32_step(state);
    (u as f32 / u32::MAX as f32) * 2.0 - 1.0
}

// ── LifeModule ─────────────────────────────────────────────────────────────

pub struct LifeModule {
    mode: LifeMode,
    /// Per-channel per-bin power envelope — Viscosity, Archimedes, NonNewtonian, Stiction, Yield.
    scratch_power: [Vec<f32>; 2],
    /// Per-channel per-bin magnitude scratch — SurfaceTension, Sandpaper.
    scratch_mag: [Vec<f32>; 2],
    /// Per-channel per-bin sustain LP envelope — Capillary + Crystallization.
    sustain_envelope: [Vec<f32>; 2],
    /// Per-channel per-bin capillary carry state — Capillary.
    wick_carry: [Vec<f32>; 2],
    /// Per-channel per-bin yield/tearing state — Yield.
    tear_state: [Vec<f32>; 2],
    /// Per-channel per-bin stiction moving indicator — Stiction.
    is_moving: [Vec<f32>; 2],
    /// Per-channel xorshift32 PRNG state — Yield + Brownian.
    rng_state: [u32; 2],
    sample_rate: f32,
    fft_size: usize,
}

impl LifeModule {
    pub fn new() -> Self {
        Self {
            mode: LifeMode::default(),
            scratch_power:    [Vec::new(), Vec::new()],
            scratch_mag:      [Vec::new(), Vec::new()],
            sustain_envelope: [Vec::new(), Vec::new()],
            wick_carry:       [Vec::new(), Vec::new()],
            tear_state:       [Vec::new(), Vec::new()],
            is_moving:        [Vec::new(), Vec::new()],
            rng_state: [0xCAFE_F00D, 0xDEAD_BEEF],
            sample_rate: 48_000.0,
            fft_size: 2048,
        }
    }

    /// Set mode directly. Used by Task 13 (per-slot persistence) and tests.
    pub fn set_mode(&mut self, mode: LifeMode) {
        self.mode = mode;
    }
}

impl Default for LifeModule {
    fn default() -> Self { Self::new() }
}

/// FTCS finite-volume diffusion of `|bin|^2` (power) with harmonic-mean face flux.
/// Reflective boundaries (zero flux at k=0 and k=num_bins-1).
/// Phase preserved via complex scaling.
fn apply_viscosity(
    bins: &mut [Complex<f32>],
    scratch_power: &mut [f32],
    scratch_mag: &mut [f32],
    curves: &[&[f32]],
) {
    const EPS: f32 = 1e-12;

    let amount_c = curves[0];
    let mix_c    = curves[4];

    let num_bins = bins.len();

    for k in 0..num_bins {
        let mag = bins[k].norm();
        scratch_mag[k]   = mag;
        scratch_power[k] = mag * mag;
    }

    // Power diffusion reads from `scratch_power` (pre-hop snapshot); `bins` is
    // written only at index k, so in-place phasor scaling is safe.
    for k in 1..num_bins - 1 {
        let d_k     = (amount_c[k]     * 0.5 * VISCOSITY_D_MAX).clamp(0.0, VISCOSITY_D_MAX);
        let d_kp1   = (amount_c[k + 1] * 0.5 * VISCOSITY_D_MAX).clamp(0.0, VISCOSITY_D_MAX);
        let d_km1   = (amount_c[k - 1] * 0.5 * VISCOSITY_D_MAX).clamp(0.0, VISCOSITY_D_MAX);
        let d_face_right = 2.0 * d_k * d_kp1 / (d_k + d_kp1 + EPS);
        let d_face_left  = 2.0 * d_k * d_km1 / (d_k + d_km1 + EPS);
        let p_new = scratch_power[k]
            + d_face_right * (scratch_power[k + 1] - scratch_power[k])
            - d_face_left  * (scratch_power[k]     - scratch_power[k - 1]);

        let p_new   = p_new.max(0.0);
        let mag_new = p_new.sqrt();
        let mix     = (mix_c[k].clamp(0.0, 2.0)) * 0.5;

        let mag_old = scratch_mag[k];
        let dry = bins[k];
        let wet = if mag_old > EPS {
            dry * (mag_new / mag_old)
        } else {
            // Silent bin receiving incoming flux: inject as real-valued (no phase info).
            Complex::new(mag_new, 0.0)
        };
        bins[k] = dry * (1.0 - mix) + wet * mix;
    }
}

/// Adjacent peak attraction. Bins above THRESHOLD steal a tiny fraction of the
/// magnitude of weaker neighbours within ±REACH bins, weighted 1/distance.
/// Approximately conserves total magnitude (transport, not creation).
fn apply_surface_tension(
    bins: &mut [Complex<f32>],
    scratch_mag: &mut [f32],
    curves: &[&[f32]],
) {
    let amount_c = curves[0];
    let thresh_c = curves[1];
    let reach_c  = curves[3];
    let mix_c    = curves[4];

    let num_bins = bins.len();

    for k in 0..num_bins {
        scratch_mag[k] = bins[k].norm();
    }

    // Streaming steal pass: `scratch_mag` is mutated as we iterate. The
    // left-to-right asymmetry is intentional — earlier (lower-k) bins are
    // depleted first by their right neighbours, which then find their left
    // neighbour already weakened and can steal more aggressively. This drives
    // coalescence even when the input is locally uniform.
    for k in 0..num_bins {
        let mag = scratch_mag[k];
        let thresh = (thresh_c[k] * 0.5).clamp(0.0, 1.0);
        if mag <= thresh {
            continue;
        }

        let amt_max = SURFACE_TENSION_AMT_MAX;
        let amt = (amount_c[k] * (amt_max * 0.5)).clamp(0.0, amt_max);
        let reach_max = SURFACE_TENSION_REACH_MAX;
        let reach_bins = ((reach_c[k] * (reach_max as f32 * 0.5)) as i32).clamp(1, reach_max);

        let mut accum = 0.0_f32;
        for d in 1..=reach_bins {
            let kl = k as i32 - d;
            let kr = k as i32 + d;
            let weight = amt / d as f32;
            if kl >= 0 {
                let nb = scratch_mag[kl as usize];
                if nb <= mag {
                    let take = nb * weight;
                    accum += take;
                    scratch_mag[kl as usize] -= take;
                }
            }
            if (kr as usize) < num_bins {
                let nb = scratch_mag[kr as usize];
                if nb <= mag {
                    let take = nb * weight;
                    accum += take;
                    scratch_mag[kr as usize] -= take;
                }
            }
        }

        scratch_mag[k] = mag + accum;
    }

    for k in 0..num_bins {
        let old_mag = bins[k].norm();
        let new_mag = scratch_mag[k].max(0.0);
        let scale_wet = if old_mag > 1e-9 { new_mag / old_mag } else { 0.0 };
        let mix = (mix_c[k].clamp(0.0, 2.0)) * 0.5;
        let dry = bins[k];
        let wet = if old_mag > 1e-9 {
            dry * scale_wet
        } else {
            // Silent bin receiving accumulated mag from neighbour-stealing
            // would otherwise stay zero — inject as real-valued.
            Complex::new(new_mag, 0.0)
        };
        bins[k] = dry * (1.0 - mix) + wet * mix;
    }
}

/// Sustained tonal bins build crystallization. Writes to BinPhysics.crystallization
/// for downstream readers (Freeze). AMOUNT scales the crystallization growth rate;
/// THRESHOLD is the magnitude floor above which a bin counts as "sustained";
/// SPEED scales the LP coefficient (larger = faster build/decay).
fn apply_crystallization(
    bins: &mut [Complex<f32>],
    sustain_envelope: &mut [f32],
    curves: &[&[f32]],
    physics: Option<&mut crate::dsp::bin_physics::BinPhysics>,
    num_bins: usize,
) {
    let amount_c = curves[0];
    let thresh_c = curves[1];
    let speed_c  = curves[2];
    let mix_c    = curves[4];

    for k in 0..num_bins {
        let mag    = bins[k].norm();
        let thresh = (thresh_c[k] * 0.5).clamp(0.0, 1.0);
        let speed  = (speed_c[k]  * 0.5).clamp(0.0, 1.0);
        let alpha  = SUSTAIN_LP_ALPHA * (1.0 + speed * 4.0); // 0.05 .. 0.25

        let sustained = if mag > thresh { 1.0 } else { 0.0 };
        sustain_envelope[k] = sustain_envelope[k] * (1.0 - alpha) + sustained * alpha;

        let amt = (amount_c[k] * 0.5).clamp(0.0, 1.0);
        let crystal_local = (sustain_envelope[k] * amt).clamp(0.0, 1.0);

        // v1 phase-lock target: real axis (frozen phase = 0). Future revision may
        // lock to first-observed-phase per slot; keep simple for now.
        let mix    = (mix_c[k].clamp(0.0, 2.0)) * 0.5;
        let target = Complex::new(mag, 0.0);
        let dry    = bins[k];
        let locked = dry * (1.0 - crystal_local) + target * crystal_local;
        bins[k]    = dry * (1.0 - mix) + locked * mix;
    }

    if let Some(p) = physics {
        // BinPhysics merge rule for `crystallization` is Max (see bin_physics.rs:110).
        // Max-merge only: crystallization accumulates and never decays within a
        // session (reset_active() clears it). Permanent crystallization is the
        // intended v1 semantic — downstream readers (Freeze) treat it as a
        // durable latch.
        for k in 0..num_bins {
            let amt = (amount_c[k] * 0.5).clamp(0.0, 1.0);
            let crystal_local = (sustain_envelope[k] * amt).clamp(0.0, 1.0);
            p.crystallization[k] = p.crystallization[k].max(crystal_local);
        }
    }
}

/// Volume-conserving ducking. Total spectral magnitude is treated as fluid volume;
/// when total exceeds capacity (controlled by THRESHOLD), bins are scaled down
/// proportionally. AMOUNT scales the displacement; THRESHOLD sets the capacity.
/// MIX blends wet/dry. Does not use BinPhysics directly.
fn apply_archimedes(
    bins: &mut [Complex<f32>],
    curves: &[&[f32]],
    num_bins: usize,
) {
    if num_bins == 0 {
        return;
    }

    let amount_c = curves[0];
    let thresh_c = curves[1];
    let mix_c    = curves[4];

    let mut total_mag = 0.0_f32;
    for k in 0..num_bins {
        total_mag += bins[k].norm();
    }

    let mut sum_amt = 0.0_f32;
    let mut sum_thresh = 0.0_f32;
    for k in 0..num_bins {
        sum_amt    += amount_c[k];
        sum_thresh += thresh_c[k];
    }
    let avg_amt    = (sum_amt    / num_bins as f32 * 0.5).clamp(0.0, 1.0);
    let avg_thresh = (sum_thresh / num_bins as f32 * 0.5).clamp(0.0, 1.0);

    let capacity       = (num_bins as f32 * avg_thresh).max(ARCHIMEDES_CAPACITY_FLOOR);
    let overflow_ratio = (total_mag / capacity - 1.0).max(0.0);
    let duck_factor    = 1.0 - (overflow_ratio * avg_amt).min(1.0 - ARCHIMEDES_DUCK_FLOOR);

    for k in 0..num_bins {
        let mix = (mix_c[k].clamp(0.0, 2.0)) * 0.5;
        let dry = bins[k];
        let wet = bins[k] * duck_factor;
        bins[k] = dry * (1.0 - mix) + wet * mix;
    }
}

/// Oobleck — solidifies under fast amplitude changes (large velocity), passes
/// slow changes freely. Reads `BinPhysics.velocity` (auto-computed by Pipeline).
/// Writes `BinPhysics.displacement` so downstream Stiction/Yield can react.
fn apply_non_newtonian(
    bins: &mut [Complex<f32>],
    curves: &[&[f32]],
    velocity: Option<&[f32]>,
    physics_out: Option<&mut crate::dsp::bin_physics::BinPhysics>,
    num_bins: usize,
) {
    let amount_c = curves[0];
    let thresh_c = curves[1];
    let mix_c    = curves[4];

    // Single pass: scale wet bins above threshold AND accumulate displacement.
    let mut physics_out = physics_out;
    for k in 0..num_bins {
        let v      = velocity.map(|vs| vs[k]).unwrap_or(0.0);
        let thresh = (thresh_c[k] * 0.5).clamp(0.0, 1.0);
        if v <= thresh {
            continue; // v <= thresh: bin passes through unchanged.
        }

        let amt    = (amount_c[k] * 0.5).clamp(0.0, 1.0);
        let excess = v - thresh;

        let mag_old = bins[k].norm();
        let limit   = (mag_old - excess * amt).max(0.0);
        let scale   = if mag_old > 1e-9 { limit / mag_old } else { 0.0 };
        let mix     = (mix_c[k].clamp(0.0, 2.0)) * 0.5;
        let dry     = bins[k];
        let wet     = bins[k] * scale;
        bins[k]     = dry * (1.0 - mix) + wet * mix;

        if let Some(p) = physics_out.as_deref_mut() {
            p.displacement[k] = (p.displacement[k] + excess).min(NON_NEWTONIAN_DISPLACEMENT_CAP);
        }
    }
}

/// Static + kinetic friction. Bins below THRESHOLD velocity are "stuck" — they
/// decay to zero. Bins above THRESHOLD are "moving" — passthrough. Once moving,
/// they stay moving for SPEED hops before re-locking. Writes displacement to
/// BinPhysics (same cap as Non-Newtonian).
fn apply_stiction(
    bins: &mut [Complex<f32>],
    is_moving: &mut [f32],
    curves: &[&[f32]],
    velocity: Option<&[f32]>,
    physics_out: Option<&mut crate::dsp::bin_physics::BinPhysics>,
    num_bins: usize,
) {
    let amount_c = curves[0];
    let thresh_c = curves[1];
    let speed_c  = curves[2];
    let mix_c    = curves[4];

    // Single pass: update is_moving, scale wet, accumulate displacement.
    let mut physics_out = physics_out;
    for k in 0..num_bins {
        let v      = velocity.map(|vs| vs[k]).unwrap_or(0.0);
        let thresh = (thresh_c[k] * 0.5).clamp(0.0, 1.0);
        let speed  = (speed_c[k]  * 0.5).clamp(0.0, 1.0);

        if v > thresh {
            is_moving[k] = 1.0;
        } else {
            let decay = STICTION_DECAY_MIN + speed * STICTION_DECAY_RANGE;
            is_moving[k] = (is_moving[k] - decay).max(0.0);
        }

        let amt = (amount_c[k] * 0.5).clamp(0.0, 1.0);
        // stuck_factor lerps from `1 - amt` (fully stuck, is_moving=0) to `1`
        // (free, is_moving=1). At amt=1 + is_moving=0 the bin is fully silenced.
        let stuck_factor = 1.0 - (1.0 - is_moving[k]) * amt;

        let mix = (mix_c[k].clamp(0.0, 2.0)) * 0.5;
        let dry = bins[k];
        let wet = bins[k] * stuck_factor;
        bins[k] = dry * (1.0 - mix) + wet * mix;

        if let Some(p) = physics_out.as_deref_mut() {
            // Displacement contribution = `stuck * v`. Stuck-but-low-velocity bins
            // accumulate slowly; momentarily-stuck high-velocity bins barely register
            // because `is_moving` was already 1.0 the moment they crossed threshold.
            // This is the static-friction metaphor: visible displacement only when
            // the bin is being held against a real (small) push.
            let stuck = (1.0 - is_moving[k]).clamp(0.0, 1.0);
            p.displacement[k] =
                (p.displacement[k] + stuck * v).min(NON_NEWTONIAN_DISPLACEMENT_CAP);
        }
    }
}

impl SpectralModule for LifeModule {
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
        ctx: &ModuleContext<'_>,
    ) {
        debug_assert!(channel < 2);
        debug_assert_eq!(bins.len(), ctx.num_bins);

        let scratch_power = &mut self.scratch_power[channel];
        let scratch_mag   = &mut self.scratch_mag[channel];

        match self.mode {
            LifeMode::Viscosity => {
                let _ = physics;
                apply_viscosity(bins, scratch_power, scratch_mag, curves);
            }
            LifeMode::SurfaceTension => {
                let _ = physics;
                apply_surface_tension(bins, scratch_mag, curves);
            }
            LifeMode::Crystallization => {
                let sustain = &mut self.sustain_envelope[channel];
                apply_crystallization(bins, sustain, curves, physics, ctx.num_bins);
            }
            LifeMode::Archimedes => {
                let _ = physics;
                apply_archimedes(bins, curves, ctx.num_bins);
            }
            LifeMode::NonNewtonian => {
                let velocity = ctx.bin_physics.map(|bp| &bp.velocity[..ctx.num_bins]);
                apply_non_newtonian(bins, curves, velocity, physics, ctx.num_bins);
            }
            LifeMode::Stiction => {
                let velocity = ctx.bin_physics.map(|bp| &bp.velocity[..ctx.num_bins]);
                let is_moving = &mut self.is_moving[channel];
                apply_stiction(bins, is_moving, curves, velocity, physics, ctx.num_bins);
            }
            _ => {
                let _ = physics;
                // Filled in Tasks 9–12.
            }
        }

        for s in suppression_out.iter_mut() {
            *s = 0.0;
        }
    }

    fn reset(&mut self, sample_rate: f32, fft_size: usize) {
        self.sample_rate = sample_rate;
        self.fft_size = fft_size;
        let num_bins = fft_size / 2 + 1;
        for ch in 0..2 {
            self.scratch_power[ch].clear();
            self.scratch_power[ch].resize(num_bins, 0.0);
            self.scratch_mag[ch].clear();
            self.scratch_mag[ch].resize(num_bins, 0.0);
            self.sustain_envelope[ch].clear();
            self.sustain_envelope[ch].resize(num_bins, 0.0);
            self.wick_carry[ch].clear();
            self.wick_carry[ch].resize(num_bins, 0.0);
            self.tear_state[ch].clear();
            self.tear_state[ch].resize(num_bins, 0.0);
            self.is_moving[ch].clear();
            self.is_moving[ch].resize(num_bins, 0.0);
        }
        // Deterministic reseed on reset.
        self.rng_state = [0xCAFE_F00D, 0xDEAD_BEEF];
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::Life
    }

    fn num_curves(&self) -> usize {
        5
    }
}
