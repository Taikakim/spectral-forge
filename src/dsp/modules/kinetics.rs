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

use crate::dsp::bin_physics::BinPhysics;
use crate::dsp::modules::{ModuleContext, ModuleType, SpectralModule};
use crate::dsp::physics_helpers::{apply_energy_rise_hysteresis, smooth_curve_one_pole};
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

    /// Pre-allocated per-channel scratch for input bin magnitudes (avoids audio-thread alloc).
    dry_mag_scratch: [Vec<f32>; 2],

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

/// Duration of one STFT hop in seconds. OVERLAP = 4 (75% overlap, hop = fft_size / 4).
#[inline]
fn hop_dt(sample_rate: f32, fft_size: usize) -> f32 {
    (fft_size as f32 / 4.0) / sample_rate
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
            dry_mag_scratch: [Vec::new(), Vec::new()],
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

// ── Mode kernel stubs (bodies added in Tasks 5-12) ─────────────────────────

impl KineticsModule {
    /// Hooke spring restoring force per bin. Reads dry mag from `self.dry_mag_scratch[channel]`.
    /// Implemented in Task 5.
    #[allow(clippy::too_many_arguments)]
    fn apply_hooke(
        &mut self,
        _channel: usize,
        _bins: &mut [Complex<f32>],
        _dt: f32,
        _num_bins: usize,
        _physics: Option<&BinPhysics>,
    ) {
        // Implemented in Task 5.
    }

    /// Gravity-well attraction toward static/sidechain/MIDI well positions. Implemented in Task 6.
    #[allow(clippy::too_many_arguments)]
    fn apply_gravity_well(
        &mut self,
        _channel: usize,
        _bins: &mut [Complex<f32>],
        _dt: f32,
        _num_bins: usize,
        _sidechain: Option<&[f32]>,
        _ctx: &ModuleContext<'_>,
        _physics: Option<&BinPhysics>,
    ) {
        // Implemented in Task 6.
    }

    /// Inertial mass — per-bin mass from static curve or sidechain rate-of-change.
    /// Reads dry mag from `self.dry_mag_scratch[channel]`. Implemented in Task 7.
    #[allow(clippy::too_many_arguments)]
    fn apply_inertial_mass(
        &mut self,
        _channel: usize,
        _bins: &mut [Complex<f32>],
        _dt: f32,
        _num_bins: usize,
        _sidechain: Option<&[f32]>,
        _ctx: &ModuleContext<'_>,
        _physics: Option<&mut BinPhysics>,
    ) {
        // Implemented in Task 7.
    }

    /// Orbital phase rotation of each bin around its neighbour. Implemented in Task 8.
    fn apply_orbital_phase(
        &mut self,
        _channel: usize,
        _bins: &mut [Complex<f32>],
        _dt: f32,
        _num_bins: usize,
        _physics: Option<&BinPhysics>,
    ) {
        // Implemented in Task 8.
    }

    /// Ferromagnetism — bin clusters magnetically align in magnitude. Implemented in Task 9.
    fn apply_ferromagnetism(
        &mut self,
        _channel: usize,
        _bins: &mut [Complex<f32>],
        _dt: f32,
        _num_bins: usize,
        _physics: Option<&BinPhysics>,
    ) {
        // Implemented in Task 9.
    }

    /// Thermal expansion — temperature-driven magnitude swelling.
    /// Reads dry mag from `self.dry_mag_scratch[channel]`. Implemented in Task 10.
    #[allow(clippy::too_many_arguments)]
    fn apply_thermal_expansion(
        &mut self,
        _channel: usize,
        _bins: &mut [Complex<f32>],
        _dt: f32,
        _num_bins: usize,
        _physics: Option<&mut BinPhysics>,
    ) {
        // Implemented in Task 10.
    }

    /// Tuning fork resonance — sympathetic frequency clusters.
    /// Reads dry mag from `self.dry_mag_scratch[channel]`. Implemented in Task 11.
    #[allow(clippy::too_many_arguments)]
    fn apply_tuning_fork(
        &mut self,
        _channel: usize,
        _bins: &mut [Complex<f32>],
        _dt: f32,
        _num_bins: usize,
        _physics: Option<&BinPhysics>,
    ) {
        // Implemented in Task 11.
    }

    /// Diamagnet — repulsion from high-magnitude neighbours + jitter.
    /// Reads dry mag from `self.dry_mag_scratch[channel]`. Implemented in Task 12.
    #[allow(clippy::too_many_arguments)]
    fn apply_diamagnet(
        &mut self,
        _channel: usize,
        _bins: &mut [Complex<f32>],
        _dt: f32,
        _num_bins: usize,
        _physics: Option<&BinPhysics>,
    ) {
        // Implemented in Task 12.
    }
}

// ── SpectralModule impl ────────────────────────────────────────────────────

impl SpectralModule for KineticsModule {
    fn process(
        &mut self,
        channel: usize,
        _stereo_link: StereoLink,
        _target: FxChannelTarget,
        bins: &mut [Complex<f32>],
        sidechain: Option<&[f32]>,
        curves: &[&[f32]],
        suppression_out: &mut [f32],
        mut physics: Option<&mut BinPhysics>,
        ctx: &ModuleContext<'_>,
    ) {
        debug_assert!(channel < 2);
        let num_bins = ctx.num_bins.min(bins.len()).min(suppression_out.len());
        let dt = hop_dt(ctx.sample_rate, ctx.fft_size);

        // -- 1. Smooth all five parameter curves through the 1-pole at this hop. --
        for c in 0..5 {
            if c >= curves.len() { continue; }
            let src = &curves[c][..num_bins.min(curves[c].len())];
            if src.len() < num_bins { continue; }
            smooth_curve_one_pole(
                &mut self.smoothed_curves[channel][c][..num_bins],
                src,
                dt,
            );
        }

        // -- 2. Capture dry magnitudes into pre-allocated scratch (no alloc). --
        // Kernels read dry magnitudes from self.dry_mag_scratch[channel] directly,
        // which avoids holding a &[f32] borrow on self during the &mut self kernel calls.
        for k in 0..num_bins {
            self.dry_mag_scratch[channel][k] = bins[k].norm();
        }

        // -- 3. Apply the active mode's force kernel. --
        // Reborrow physics for each arm via as_deref / as_deref_mut so the
        // borrow checker sees independent borrows per arm (Rust 2021 NLL).
        match self.mode {
            KineticsMode::Hooke => {
                self.apply_hooke(channel, bins, dt, num_bins, physics.as_deref());
            }
            KineticsMode::GravityWell => {
                self.apply_gravity_well(channel, bins, dt, num_bins, sidechain, ctx, physics.as_deref());
            }
            KineticsMode::InertialMass => {
                self.apply_inertial_mass(channel, bins, dt, num_bins, sidechain, ctx, physics.as_deref_mut());
            }
            KineticsMode::OrbitalPhase => {
                self.apply_orbital_phase(channel, bins, dt, num_bins, physics.as_deref());
            }
            KineticsMode::Ferromagnetism => {
                self.apply_ferromagnetism(channel, bins, dt, num_bins, physics.as_deref());
            }
            KineticsMode::ThermalExpansion => {
                self.apply_thermal_expansion(channel, bins, dt, num_bins, physics.as_deref_mut());
            }
            KineticsMode::TuningFork => {
                self.apply_tuning_fork(channel, bins, dt, num_bins, physics.as_deref());
            }
            KineticsMode::Diamagnet => {
                self.apply_diamagnet(channel, bins, dt, num_bins, physics.as_deref());
            }
        }

        // -- 4. Energy-rise hysteresis (after kernel mutated velocity). --
        {
            // Compute KE+PE into mag_prev scratch.
            let strength = &self.smoothed_curves[channel][0];
            let velocity = &self.velocity[channel];
            let displacement = &self.displacement[channel];
            let curr_kepe = &mut self.mag_prev[channel];
            for k in 0..num_bins {
                let v = velocity[k];
                let d = displacement[k];
                let s = strength[k];
                curr_kepe[k] = 0.5 * v * v + 0.5 * s * s * d * d;
            }
        }
        {
            // Apply hysteresis — mutable velocity borrow separated from the above.
            let vel_mut = &mut self.velocity[channel][..num_bins];
            let prev = &self.prev_kepe[channel][..num_bins];
            let curr = &self.mag_prev[channel][..num_bins];
            let rose = &mut self.kepe_rose_last_hop[channel][..num_bins];
            apply_energy_rise_hysteresis(vel_mut, prev, curr, rose);
        }
        // Persist this hop's energy as next-hop's "prev", then restore mag_prev to dry.
        for k in 0..num_bins {
            self.prev_kepe[channel][k] = self.mag_prev[channel][k];
            self.mag_prev[channel][k] = self.dry_mag_scratch[channel][k];
        }

        // -- 5. Suppression delta. --
        for k in 0..num_bins {
            let new_mag = bins[k].norm();
            suppression_out[k] = (new_mag - self.dry_mag_scratch[channel][k]).abs();
        }

        // -- 6. Probe. --
        #[cfg(any(test, feature = "probe"))]
        {
            let probe_bin = (num_bins / 4).min(num_bins.saturating_sub(1));
            self.last_probe_state = ProbeState {
                strength_at_probe:     self.smoothed_curves[channel][0][probe_bin],
                mass_at_probe:         self.smoothed_curves[channel][1][probe_bin],
                displacement_at_probe: self.displacement[channel][probe_bin],
                velocity_at_probe:     self.velocity[channel][probe_bin],
                active_mode_idx:       self.mode as u8,
                well_count:            self.tuning_forks[channel].len() as u16,
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
            self.dry_mag_scratch[ch].clear();
            self.dry_mag_scratch[ch].resize(num_bins, 0.0);
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
