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
//! This skeleton provides the enum, struct, and stub `process()` that passes
//! audio through unmodified and zeroes suppression_out. Kernels land in
//! Tasks 3–12.

use num_complex::Complex;
use serde::{Deserialize, Serialize};

use crate::dsp::modules::{
    FxChannelTarget, ModuleContext, ModuleType, SpectralModule, StereoLink,
};

// ── Constants ──────────────────────────────────────────────────────────────

/// FTCS diffusion stability clamp (research finding from ideas/.../11-life.md).
/// Used by the Viscosity kernel (Task 3).
#[allow(dead_code)]
const VISCOSITY_D_MAX: f32 = 0.45;

/// ~50ms time-constant LP alpha at 48k/256-hop.
/// Used by the Capillary (Task 10) and Crystallization (Task 5) kernels.
#[allow(dead_code)]
const SUSTAIN_LP_ALPHA: f32 = 0.05;

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
    #[allow(dead_code)]
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
    #[allow(dead_code)]
    pub(crate) fn set_mode(&mut self, mode: LifeMode) {
        self.mode = mode;
    }

    /// Test/probe accessor for mode injection.
    #[cfg(any(test, feature = "probe"))]
    pub fn set_mode_for_test(&mut self, mode: LifeMode) {
        self.mode = mode;
    }
}

impl Default for LifeModule {
    fn default() -> Self { Self::new() }
}

impl SpectralModule for LifeModule {
    fn process(
        &mut self,
        channel: usize,
        _stereo_link: StereoLink,
        _target: FxChannelTarget,
        _bins: &mut [Complex<f32>],
        _sidechain: Option<&[f32]>,
        _curves: &[&[f32]],
        suppression_out: &mut [f32],
        _physics: Option<&mut crate::dsp::bin_physics::BinPhysics>,
        _ctx: &ModuleContext<'_>,
    ) {
        debug_assert!(channel < 2);
        // Skeleton: passthrough — bins are untouched, suppression zeroed.
        // Kernels are added in Tasks 3–12.
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
