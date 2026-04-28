//! Kinetics — physical-force spectral module. 8 modes; per-mode kernels in `apply_*`.
//!
//! Velocity-Verlet integrator + CFL clamp + 1-pole curve smoothing + viscous-damping
//! floor + energy-rise hysteresis safety net live in `crate::dsp::physics_helpers`.

// Per-channel scratch fields, mode constants, and RNG helpers are populated
// across Tasks 4–12 of phase 5b.3. The skeleton declares them upfront so the
// integrator and kernels can land in small, focused commits without churning
// struct shape; until then they read as dead code.
#![allow(dead_code)]

use num_complex::Complex;
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;

use crate::dsp::modules::{ModuleContext, ModuleType, SpectralModule};
use crate::params::{FxChannelTarget, StereoLink};

// ── Enums ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum KineticsMode {
    #[default]
    Hooke,
    GravityWell,
    InertialMass,
    OrbitalPhase,
    Ferromagnetism,
    ThermalExpansion,
    TuningFork,
    Diamagnet,
}

/// Source for gravity-well positions (GravityWell mode).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum WellSource {
    /// Wells positioned by the REACH curve directly.
    #[default]
    Static,
    /// Wells positioned at sidechain spectrum peaks (top-N magnitudes).
    Sidechain,
    /// Wells positioned at f_root × harmonics for each held MIDI note.
    /// Degrades to no-op when `ctx.midi_notes` is `None` (Phase 6 plumb).
    MIDI,
}

/// Source for per-bin mass (InertialMass mode).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum MassSource {
    /// MASS curve directly drives per-bin mass.
    #[default]
    Static,
    /// MASS = clamp(rate_of_change(sidechain_envelope) * MASS_curve, 0.01, 1000).
    Sidechain,
}

// ── Constants ──────────────────────────────────────────────────────────────

const MAX_TUNING_FORKS: usize = 16;
const MAX_HARMONIC_SPRINGS: usize = 8;
const SC_ENVELOPE_TAU_HOPS: f32 = 1.0;
const TUNING_FORK_MIN_SEP: usize = 4;
const MAX_PEAKS: usize = 16;
const ORBITAL_SAT_HALF_WINDOW: usize = 16;

// ── State structs ──────────────────────────────────────────────────────────

#[cfg(any(test, feature = "probe"))]
#[derive(Default, Clone, Copy)]
struct ProbeState {
    strength_at_probe:    f32,
    mass_at_probe:        f32,
    displacement_at_probe: f32,
    velocity_at_probe:    f32,
    active_mode_idx:      u8,
    well_count:           u16,
}

// ── Module ─────────────────────────────────────────────────────────────────

pub struct KineticsModule {
    mode:        KineticsMode,
    well_source: WellSource,
    mass_source: MassSource,

    /// Per-channel integrator displacement state.
    displacement: [Vec<f32>; 2],
    /// Per-channel integrator velocity state.
    velocity: [Vec<f32>; 2],
    /// Per-channel temperature accumulator (ThermalExpansion mode).
    temperature_local: [Vec<f32>; 2],
    /// Magnitude at previous hop (Ferromagnetism / Diamagnet scratchpad).
    mag_prev: [Vec<f32>; 2],
    /// Phase at previous hop (OrbitalPhase / Ferromagnetism / ThermalExpansion).
    prev_phase: [Vec<f32>; 2],

    /// Energy-rise hysteresis: previous hop KE+PE per bin.
    prev_kepe: [Vec<f32>; 2],
    /// Energy-rise hysteresis: "doubled last hop" flag per bin.
    kepe_rose_last_hop: [Vec<bool>; 2],

    /// 1-pole-smoothed parameter curves — indexed [channel][curve_idx].
    /// Curve idx: 0=STRENGTH, 1=MASS, 2=REACH, 3=DAMPING, 4=MIX.
    smoothed_curves: [[Vec<f32>; 5]; 2],

    /// Per-channel TuningFork active list: (bin_index, fork_freq_hz).
    tuning_forks: [SmallVec<[(usize, f32); MAX_TUNING_FORKS]>; 2],

    /// Per-channel sidechain envelope smoother (MassSource::Sidechain rate-of-change).
    sc_env_smoothed: [f32; 2],
    sc_env_prev:     [f32; 2],

    /// Per-channel xorshift32 RNG (Diamagnet jitter).
    rng_state: [u32; 2],

    sample_rate: f32,
    fft_size:    usize,

    #[cfg(any(test, feature = "probe"))]
    last_probe_state: ProbeState,
}

// ── Helpers ────────────────────────────────────────────────────────────────

#[inline]
fn xorshift32_step(state: &mut u32) -> u32 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *state = x;
    x
}

#[inline]
fn xorshift32_signed_unit(state: &mut u32) -> f32 {
    let u = xorshift32_step(state);
    (u as f32 / u32::MAX as f32) * 2.0 - 1.0
}

// ── impl KineticsModule ────────────────────────────────────────────────────

impl KineticsModule {
    pub fn new() -> Self {
        Self {
            mode:        KineticsMode::default(),
            well_source: WellSource::default(),
            mass_source: MassSource::default(),
            displacement:       [Vec::new(), Vec::new()],
            velocity:           [Vec::new(), Vec::new()],
            temperature_local:  [Vec::new(), Vec::new()],
            mag_prev:           [Vec::new(), Vec::new()],
            prev_phase:         [Vec::new(), Vec::new()],
            prev_kepe:          [Vec::new(), Vec::new()],
            kepe_rose_last_hop: [Vec::new(), Vec::new()],
            smoothed_curves: [
                [Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new()],
                [Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new()],
            ],
            tuning_forks:    [SmallVec::new(), SmallVec::new()],
            sc_env_smoothed: [0.0, 0.0],
            sc_env_prev:     [0.0, 0.0],
            rng_state:       [0xC0FF_EE01, 0xBADD_CAFE],
            sample_rate:     48_000.0,
            fft_size:        2048,
            #[cfg(any(test, feature = "probe"))]
            last_probe_state: ProbeState::default(),
        }
    }

    pub fn set_mode(&mut self, mode: KineticsMode) { self.mode = mode; }
    pub fn set_well_source(&mut self, src: WellSource) { self.well_source = src; }
    pub fn set_mass_source(&mut self, src: MassSource) { self.mass_source = src; }
}

// ── SpectralModule impl ────────────────────────────────────────────────────

impl SpectralModule for KineticsModule {
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
        // Stub — kernels added in Tasks 4–12. Passthrough for now.
        debug_assert!(channel < 2);
        for s in suppression_out.iter_mut() { *s = 0.0; }

        #[cfg(any(test, feature = "probe"))]
        {
            // Populate probe with zeros — the real data arrives in Task 4+.
            self.last_probe_state = ProbeState {
                active_mode_idx: self.mode as u8,
                ..ProbeState::default()
            };
        }
    }

    fn reset(&mut self, sample_rate: f32, fft_size: usize) {
        self.sample_rate = sample_rate;
        self.fft_size = fft_size;
        let num_bins = fft_size / 2 + 1;
        for ch in 0..2 {
            self.displacement[ch].clear();
            self.displacement[ch].resize(num_bins, 0.0);
            self.velocity[ch].clear();
            self.velocity[ch].resize(num_bins, 0.0);
            self.temperature_local[ch].clear();
            self.temperature_local[ch].resize(num_bins, 0.0);
            self.mag_prev[ch].clear();
            self.mag_prev[ch].resize(num_bins, 0.0);
            self.prev_phase[ch].clear();
            self.prev_phase[ch].resize(num_bins, 0.0);
            self.prev_kepe[ch].clear();
            self.prev_kepe[ch].resize(num_bins, 0.0);
            self.kepe_rose_last_hop[ch].clear();
            self.kepe_rose_last_hop[ch].resize(num_bins, false);
            for c in 0..5 {
                self.smoothed_curves[ch][c].clear();
                self.smoothed_curves[ch][c].resize(num_bins, 1.0);
            }
            self.tuning_forks[ch].clear();
            self.sc_env_smoothed[ch] = 0.0;
            self.sc_env_prev[ch] = 0.0;
        }
        self.rng_state = [0xC0FF_EE01, 0xBADD_CAFE];
    }

    fn module_type(&self) -> ModuleType { ModuleType::Kinetics }
    fn num_curves(&self) -> usize { 5 }

    fn set_kinetics_mode(&mut self, mode: crate::dsp::modules::kinetics::KineticsMode) {
        self.set_mode(mode);
    }
    fn set_kinetics_well_source(&mut self, src: crate::dsp::modules::kinetics::WellSource) {
        self.set_well_source(src);
    }
    fn set_kinetics_mass_source(&mut self, src: crate::dsp::modules::kinetics::MassSource) {
        self.set_mass_source(src);
    }

    #[cfg(any(test, feature = "probe"))]
    fn last_probe(&self) -> crate::dsp::modules::ProbeSnapshot {
        let p = self.last_probe_state;
        crate::dsp::modules::ProbeSnapshot {
            kinetics_strength:        Some(p.strength_at_probe),
            kinetics_mass:            Some(p.mass_at_probe),
            kinetics_displacement:    Some(p.displacement_at_probe),
            kinetics_velocity:        Some(p.velocity_at_probe),
            kinetics_active_mode_idx: Some(p.active_mode_idx),
            kinetics_well_count:      Some(p.well_count),
            ..Default::default()
        }
    }
}
