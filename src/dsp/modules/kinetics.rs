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
use crate::dsp::physics_helpers::{
    apply_energy_rise_hysteresis,
    clamp_damping_floor,
    clamp_for_cfl,
    smooth_curve_one_pole,
};
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
/// Sidechain envelope smoother time constant in hops.
/// `alpha = 1 - exp(-1 / SC_ENVELOPE_TAU_HOPS)`, so at 1.0 hops the envelope
/// follows the sidechain within ~1 hop (very fast). The rate-of-change derived
/// from this envelope is divided by `dt`, so the effective rate scales with
/// sample rate / hop size; `SC_MASS_RATE_SCALE` was tuned for the default
/// hop dt (fft=2048, sr=48k → dt ≈ 10.7 ms). If hop changes substantially,
/// the scale may need re-tuning to keep the audible response consistent.
const SC_ENVELOPE_TAU_HOPS: f32 = 1.0;
/// Sidechain rate-of-change → mass multiplier scale.
/// `mass = (1.0 + SC_MASS_RATE_SCALE * rate) * MASS_curve[k]`. Tuned for the
/// default hop dt; if hop changes substantially this may need re-tuning.
const SC_MASS_RATE_SCALE: f32 = 5.0;
const TUNING_FORK_MIN_SEP: usize = 4;
const MAX_PEAKS: usize = 16;
const ORBITAL_SAT_HALF_WINDOW: usize = 16;
/// OrbitalPhase peak detection: a bin counts as a "peak" only if its magnitude
/// exceeds this factor times the local-window mean (window half-width =
/// `ORBITAL_SAT_HALF_WINDOW`). Higher values reject more micro-peaks.
const ORBITAL_PEAK_THRESHOLD_FACTOR: f32 = 2.0;
/// Strength curve must exceed this baseline to register as a static gravity well.
const STATIC_WELL_BASELINE: f32 = 1.05;
/// Sidechain peak must reach this fraction of the per-hop max to register as a well.
const SC_WELL_THRESHOLD_FRAC: f32 = 0.4;

// ── Ferromagnetism kernel constants ──────────────────────────────────────────

/// Peak detection window half-width for Ferromagnetism mode.
/// A bin counts as a peak only if its magnitude exceeds `ORBITAL_PEAK_THRESHOLD_FACTOR`
/// times the local-window mean over this many bins on each side.
const FERRO_PEAK_WINDOW_HALF: usize = 8;
/// Per-hop alpha multiplier on STRENGTH: `alpha = STRENGTH * FERRO_ALPHA_SCALE`.
/// Keeps per-hop phase pull in the range [0, 0.6] for STRENGTH in [0, 2].
const FERRO_ALPHA_SCALE: f32 = 0.3;
/// Maximum per-hop pull fraction — clamps `alpha * weight / (1 + resistance)`.
const FERRO_PULL_CAP: f32 = 0.95;
/// REACH curve → bin distance multiplier: `reach_bins = REACH * FERRO_REACH_SCALE`.
const FERRO_REACH_SCALE: f32 = 16.0;

// ── ThermalExpansion kernel constants ─────────────────────────────────────────

/// STRENGTH curve clamp upper bound for heat input.
/// `heat_in = STRENGTH.clamp(0, THERMAL_HEAT_STRENGTH_CLAMP_HI) * mag² * dt`.
const THERMAL_HEAT_STRENGTH_CLAMP_HI: f32 = 4.0;
/// DAMPING curve clamp upper bound for cooling rate.
/// `cool_rate = DAMPING.clamp(0, THERMAL_DAMPING_CLAMP_HI) * THERMAL_COOL_RATE_SCALE`.
const THERMAL_DAMPING_CLAMP_HI: f32 = 4.0;
/// Multiplier from DAMPING to cool_rate: `cool_rate = DAMPING * THERMAL_COOL_RATE_SCALE`.
const THERMAL_COOL_RATE_SCALE: f32 = 2.0;
/// Maximum temperature per bin. Prevents unbounded accumulation on loud sustained signals.
const THERMAL_TEMP_CEILING: f32 = 10.0;
/// Frequency detune per unit temperature: `detune_hz = THERMAL_DETUNE_HZ_PER_TEMP * temp`.
/// At temp=1.0 and default fft (2048/48k), produces ~5 Hz of phase-rotation detune per hop.
const THERMAL_DETUNE_HZ_PER_TEMP: f32 = 5.0;
/// 50/50 blend weight when mirroring local temperature into `BinPhysics.temperature`.
/// `p.temperature[k] = (1 - BLEND) * p.temperature[k] + BLEND * temp[k]`.
const THERMAL_PHYSICS_BLEND: f32 = 0.5;

// ── TuningFork kernel constants ───────────────────────────────────────────────

/// Minimum bin magnitude for a peak candidate to register as a tuning fork.
/// Bins below this threshold are too quiet to act as resonance drivers.
const TUNING_FORK_MIN_MAG: f32 = 0.5;
/// STRENGTH curve must exceed this value at a peak bin for it to register as a fork.
/// At neutral STRENGTH=1.0 forks are suppressed; user must raise STRENGTH above 1.5
/// to activate sympathetic modulation at a given frequency region.
const TUNING_FORK_STRENGTH_THRESHOLD: f32 = 1.5;
/// STRENGTH curve upper clamp for modulation depth computation.
/// `modulation_depth = (STRENGTH.clamp(0, STRENGTH_MAX) - 1.0).max(0) * DEPTH_SCALE`.
const TUNING_FORK_STRENGTH_MAX: f32 = 2.0;
/// REACH curve upper clamp. Maps REACH (neutral=1.0) to a maximum of 4.0 bin-radius-units.
const TUNING_FORK_REACH_CLAMP_HI: f32 = 4.0;
/// Scale from REACH value to reach in bins: `reach_bins = (REACH * REACH_BIN_SCALE).round()`.
/// At neutral REACH=1.0 → 8 bins radius; at max REACH=4.0 → 32 bins radius.
const TUNING_FORK_REACH_BIN_SCALE: f32 = 8.0;
/// Converts (STRENGTH − 1.0) excess to phase modulation depth per unit carrier.
/// `modulation_depth = (strength - 1.0) * DEPTH_SCALE`. Tuned so that at STRENGTH=2.0
/// (maximum) the per-neighbour phase excursion stays within ±0.4 rad per hop.
const TUNING_FORK_DEPTH_SCALE: f32 = 0.4;
/// Minimum fractional phase advance per hop for the fork carrier.
/// When `phase_advance rem_euclid(2π)` is exactly zero (bin at an integer-overlap-multiple
/// frequency), the carrier stalls. Adding this tiny nudge — one quarter of the golden
/// angle divided by MAX_TUNING_FORKS — prevents degeneracy without audibly colouring the
/// modulation rate. Each fork gets a distinct nudge via `fork_idx * NUDGE`.
const TUNING_FORK_CARRIER_NUDGE: f32 = 0.02439_f32; // ≈ π*(√5−1)/2 / MAX_TUNING_FORKS

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
    /// Per-channel scratch shared between two roles:
    /// - Persisted across hops as the previous-hop magnitude (Ferromagnetism / Diamagnet).
    /// - Reused inside `process()` as the KE+PE scratch for the energy-rise hysteresis
    ///   step, then restored to the dry magnitude before the suppression delta runs.
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

    /// Scratch for well count written by apply_gravity_well; read by the probe block in
    /// process() so the ProbeState reflects the correct mode's well count.
    last_well_count: [u16; 2],

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
            last_well_count: [0; 2],
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
    #[allow(clippy::too_many_arguments)]
    fn apply_hooke(
        &mut self,
        channel: usize,
        bins: &mut [Complex<f32>],
        dt: f32,
        num_bins: usize,
        _physics: Option<&BinPhysics>,
    ) {
        // Curves (smoothed): 0=STRENGTH, 1=MASS, 2=REACH, 3=DAMPING, 4=MIX.
        //   STRENGTH (omega in rad/s) : neutral=1 → 50 rad/s; range 0..max-CFL.
        //   MASS                       : neutral=1 → 1.0; clamp to [0.1, 1000].
        //   REACH (harmonic count)     : neutral=1 → 0 harmonic springs; up to 8 harmonics.
        //   DAMPING                    : neutral=1 → 0.2; floored at 0.05.
        //   MIX (wet/dry blend)        : neutral=1 → 0.5; range [0, 1].

        // -- 1. Neighbour spring forces + sympathetic harmonic springs.
        //    Two-pass approach to satisfy the borrow checker:
        //      Pass A: read smoothed_curves + dry_mag_scratch, write displacement + velocity.
        //      Pass B: read displacement + dry_mag_scratch, write bins.

        // Bind disjoint-field borrows so the inner loop sees flat slice references,
        // not three-level indexes through self.
        let s_strength = &self.smoothed_curves[channel][0][..num_bins];
        let s_mass     = &self.smoothed_curves[channel][1][..num_bins];
        let s_reach    = &self.smoothed_curves[channel][2][..num_bins];
        let s_damping  = &self.smoothed_curves[channel][3][..num_bins];
        let s_dry      = &self.dry_mag_scratch[channel][..num_bins];
        let velocity     = &mut self.velocity[channel][..num_bins];
        let displacement = &mut self.displacement[channel][..num_bins];

        // Pass A: Verlet integration — iterate bins, read curves by slice ref, mutate velocity
        // and displacement.
        for k in 1..(num_bins - 1) {
            let omega   = clamp_for_cfl(50.0 * s_strength[k].max(0.0), dt);
            let mass    = s_mass[k].clamp(0.1, 1000.0);
            let damping = clamp_damping_floor(0.2 * s_damping[k]);
            let reach_val = s_reach[k];

            let dry_k   = s_dry[k];
            let dry_km1 = s_dry[k - 1];
            let dry_kp1 = s_dry[k + 1];

            let neighbour_avg = 0.5 * (dry_km1 + dry_kp1);
            let mut f = -omega * omega * (dry_k - neighbour_avg);

            // Sympathetic harmonic springs (cap = MAX_HARMONIC_SPRINGS).
            let h_count = (reach_val.clamp(0.0, 2.0) * 4.0).round() as usize;
            let h_count = h_count.min(MAX_HARMONIC_SPRINGS);
            for h in 2..(2 + h_count) {
                let kh = k.saturating_mul(h);
                if kh >= num_bins - 1 { break; }
                let weight = 1.0 / h as f32;
                f += -omega * omega * weight * (dry_k - s_dry[kh]);
            }

            let accel = (f - damping * velocity[k]) / mass;
            velocity[k]     += accel * dt;
            displacement[k] += velocity[k] * dt;

            // Floor: magnitude cannot go below zero. When displacement would drive
            // mag negative, clamp and zero velocity (absorb rather than reflect).
            let floor = -dry_k;
            if displacement[k] < floor {
                displacement[k] = floor;
                if velocity[k] < 0.0 {
                    velocity[k] = 0.0;
                }
            }
        }

        // -- 2. Translate displacement → magnitude multiplier and blend with dry.
        let max_mag = {
            let mut m = 0.0_f32;
            for k in 0..num_bins {
                let v = self.dry_mag_scratch[channel][k];
                if v > m { m = v; }
            }
            m
        };
        let cap = 4.0 * max_mag.max(1e-6);

        // Pass A skips k=0 (DC) and k=num_bins-1 (Nyquist) — their displacement stays zero,
        // so the wet path here reduces to identity for those bins. Intentional.
        for k in 0..num_bins {
            let mix = self.smoothed_curves[channel][4][k].clamp(0.0, 1.0);
            let dry_k = self.dry_mag_scratch[channel][k];
            let new_mag = (dry_k + self.displacement[channel][k]).clamp(0.0, cap);
            if dry_k > 1e-9 {
                let scale = new_mag / dry_k;
                let wet_re = bins[k].re * scale;
                let wet_im = bins[k].im * scale;
                bins[k].re = bins[k].re * (1.0 - mix) + wet_re * mix;
                bins[k].im = bins[k].im * (1.0 - mix) + wet_im * mix;
            } else if new_mag > 1e-9 {
                // Inject displaced energy into a previously-silent bin as a real-valued tone.
                bins[k].re = bins[k].re * (1.0 - mix) + new_mag * mix;
                bins[k].im = bins[k].im * (1.0 - mix);
            }
        }
    }

    /// Gravity-well attraction toward frequency targets.
    ///
    /// Three well sources:
    /// - `Static`:   local maxima above `STATIC_WELL_BASELINE` in the STRENGTH curve become wells.
    /// - `Sidechain`: peaks ≥ `SC_WELL_THRESHOLD_FRAC` of max sidechain amplitude become wells.
    /// - `MIDI`:     harmonic series per held note; no-op when `ctx.midi_notes` is `None`.
    ///
    /// Per-bin force is Newtonian (`sign(d) * w_amp / d²` summed over wells) driven through
    /// the Velocity-Verlet integrator. MIX curve blends wet/dry.
    #[allow(clippy::too_many_arguments)]
    fn apply_gravity_well(
        &mut self,
        channel: usize,
        bins: &mut [Complex<f32>],
        dt: f32,
        num_bins: usize,
        sidechain: Option<&[f32]>,
        ctx: &ModuleContext<'_>,
        _physics: Option<&BinPhysics>,
    ) {
        // Bind local slice refs for all smoothed curves — avoids triple-indexing through self
        // inside the inner loop and keeps borrows disjoint from the velocity/displacement writes.
        let s_strength = &self.smoothed_curves[channel][0][..num_bins];
        let s_mass     = &self.smoothed_curves[channel][1][..num_bins];
        let s_reach    = &self.smoothed_curves[channel][2][..num_bins];
        let s_damping  = &self.smoothed_curves[channel][3][..num_bins];
        let s_mix      = &self.smoothed_curves[channel][4][..num_bins];

        // -- 1. Determine well positions (bin_index, well_amplitude). --
        //    SmallVec stays stack-allocated up to MAX_PEAKS = 16 entries; no heap allocation.
        let mut wells: SmallVec<[(usize, f32); MAX_PEAKS]> = SmallVec::new();
        match self.well_source {
            WellSource::Static => {
                // Local maxima of the STRENGTH curve above the 1.05 baseline become wells.
                for k in 1..(num_bins - 1) {
                    if s_strength[k] > STATIC_WELL_BASELINE
                        && s_strength[k] > s_strength[k - 1]
                        && s_strength[k] > s_strength[k + 1]
                    {
                        let amp = s_strength[k] - 1.0; // excess above neutral
                        if wells.len() < MAX_PEAKS {
                            wells.push((k, amp));
                        }
                    }
                }
            }
            WellSource::Sidechain => {
                if let Some(sc) = sidechain {
                    let sc_max = sc.iter().fold(0.0_f32, |a, &b| a.max(b));
                    let thresh = sc_max * SC_WELL_THRESHOLD_FRAC;
                    if sc_max > 1e-6 {
                        let sc_len = sc.len().min(num_bins);
                        for k in 1..(sc_len.saturating_sub(1)) {
                            if sc[k] >= thresh && sc[k] > sc[k - 1] && sc[k] > sc[k + 1] {
                                if wells.len() < MAX_PEAKS {
                                    wells.push((k, sc[k]));
                                }
                            }
                        }
                    }
                }
            }
            WellSource::MIDI => {
                // Harmonic series per held note. No-op when ctx.midi_notes is None (Phase 6 pending).
                if let Some(notes) = ctx.midi_notes {
                    let bin_hz = ctx.sample_rate / ctx.fft_size as f32;
                    let harmonic_count = (s_reach[0].clamp(0.0, 2.0) * 4.0).round() as usize;
                    for midi in 0..128_usize {
                        if wells.len() >= MAX_PEAKS { break; }
                        if !notes[midi] { continue; }
                        let f_root = 440.0 * 2f32.powf((midi as f32 - 69.0) / 12.0);
                        for h in 1..=harmonic_count {
                            if wells.len() >= MAX_PEAKS { break; }
                            let f = f_root * h as f32;
                            let k = (f / bin_hz).round() as isize;
                            if k > 0 && (k as usize) < num_bins {
                                let amp = 1.0 / h as f32;
                                if wells.len() < MAX_PEAKS {
                                    wells.push((k as usize, amp));
                                }
                            }
                        }
                    }
                }
                // If ctx.midi_notes is None, wells stays empty → true passthrough.
            }
        }

        // Record well count for the probe block in process() before possible early return.
        self.last_well_count[channel] = wells.len() as u16;

        if wells.is_empty() {
            return; // No wells → no force → passthrough.
        }

        // -- 2. Per-bin force + Verlet integration. --
        //    Read dry mags from the pre-allocated scratch (captured before the kernel call).
        //    The velocity and displacement vecs are mutated in place.
        {
            let velocity     = &mut self.velocity[channel][..num_bins];
            let displacement = &mut self.displacement[channel][..num_bins];

            for k in 0..num_bins {
                let mass    = s_mass[k].clamp(0.1, 1000.0);
                let damping = clamp_damping_floor(0.2 * s_damping[k]);
                let reach   = s_reach[k].clamp(0.1, 4.0);

                let mut force_signed = 0.0_f32;
                for &(wk, w_amp) in wells.iter() {
                    let d = wk as isize - k as isize;
                    if d == 0 {
                        // Well bin itself: inject positive force so energy accumulates at the well.
                        force_signed += w_amp;
                        continue;
                    }
                    // Normalise bin distance by REACH-scaled window (20 bins at reach=1).
                    let d_norm = d as f32 / (reach * 20.0);
                    let denom  = (d_norm * d_norm).max(1e-3);
                    // Pull toward well: sign(d) * w_amp / d^2  (Newtonian-ish).
                    force_signed += (d.signum() as f32) * w_amp / denom;
                }
                // Scale force by STRENGTH² at this bin; 0.001 is empirical to keep displacements small.
                let omega = clamp_for_cfl(50.0 * s_strength[k].max(0.0), dt);
                let f     = omega * omega * force_signed * 0.001;

                let accel     = (f - damping * velocity[k]) / mass;
                velocity[k]     += accel * dt;
                displacement[k] += velocity[k] * dt;

                // Floor: clamp in step 3 via .clamp(0, cap); no velocity zeroing here
                // (gravity well is a weaker pull than Hooke; cap handles it).
            }
        }

        // -- 3. Translate displacement → magnitude bend; blend with dry via MIX. --
        let max_mag = {
            let dry_mag_scratch = &self.dry_mag_scratch[channel][..num_bins];
            dry_mag_scratch.iter().fold(0.0_f32, |a, &b| a.max(b))
        };
        let cap = 4.0 * max_mag.max(1e-6);

        let dry_mag_scratch = &self.dry_mag_scratch[channel][..num_bins];
        let displacement    = &self.displacement[channel][..num_bins];
        for k in 0..num_bins {
            let mix    = s_mix[k].clamp(0.0, 1.0);
            let dry_k  = dry_mag_scratch[k];
            let new_mag = (dry_k + displacement[k]).clamp(0.0, cap);
            if dry_k > 1e-9 {
                let scale  = new_mag / dry_k;
                let wet_re = bins[k].re * scale;
                let wet_im = bins[k].im * scale;
                bins[k].re = bins[k].re * (1.0 - mix) + wet_re * mix;
                bins[k].im = bins[k].im * (1.0 - mix) + wet_im * mix;
            } else if new_mag > 1e-9 {
                // Inject displaced energy into a previously-silent bin as a real-valued tone.
                bins[k].re = bins[k].re * (1.0 - mix) + new_mag * mix;
                bins[k].im = bins[k].im * (1.0 - mix);
            }
        }
    }

    /// Inertial mass — writes `physics.mass` per bin; `bins` are **not** modified.
    ///
    /// Two sources (selected by `self.mass_source`):
    /// - `Static`:   `physics.mass[k]` = MASS_curve[k], MIX-blended with current value.
    /// - `Sidechain`: broadband SC envelope rate-of-change drives mass UP when the
    ///   sidechain is changing fast and DOWN when steady. Formula:
    ///   `mass = clamp((1 + 5 * rate) * MASS_curve[k], 0.01, 1000)`.
    ///
    /// Returns immediately (no-op) when `physics` is `None`.
    #[allow(clippy::too_many_arguments)]
    fn apply_inertial_mass(
        &mut self,
        channel: usize,
        _bins: &mut [Complex<f32>],
        dt: f32,
        num_bins: usize,
        sidechain: Option<&[f32]>,
        _ctx: &ModuleContext<'_>,
        physics: Option<&mut BinPhysics>,
    ) {
        // Bind local slice refs — avoids triple-indexing through self inside the inner
        // loop and keeps borrows disjoint from the physics write below.
        let mass_curve = &self.smoothed_curves[channel][1][..num_bins];
        let mix_curve  = &self.smoothed_curves[channel][4][..num_bins];

        // No physics writer → nothing to do.
        let physics = match physics {
            Some(p) => p,
            None => return,
        };

        match self.mass_source {
            MassSource::Static => {
                // Direct write: physics.mass[k] = MASS_curve[k] (clamped), MIX-blended.
                for k in 0..num_bins {
                    let target = mass_curve[k].clamp(0.01, 1000.0);
                    let mix    = mix_curve[k].clamp(0.0, 1.0);
                    let cur    = physics.mass[k];
                    physics.mass[k] = cur * (1.0 - mix) + target * mix;
                }
            }
            MassSource::Sidechain => {
                // -- Compute broadband SC magnitude for this hop. --
                let sc_now = if let Some(sc) = sidechain {
                    let n = sc.len().min(num_bins);
                    if n == 0 {
                        0.0
                    } else {
                        sc[..n].iter().map(|x| x.abs()).sum::<f32>() / n as f32
                    }
                } else {
                    0.0
                };

                // -- 1-pole envelope smoother (tau = SC_ENVELOPE_TAU_HOPS hops). --
                // alpha = 1 - exp(-1 / tau_hops). Read → compute → write, so the
                // immutable borrow of mass_curve/mix_curve and the mutable physics write
                // below use only local scalars, not self-borrows.
                let alpha_env = 1.0 - (-(1.0 / SC_ENVELOPE_TAU_HOPS)).exp();
                let env_prev  = self.sc_env_smoothed[channel];
                let env       = env_prev + alpha_env * (sc_now - env_prev);
                self.sc_env_smoothed[channel] = env;

                // -- Rate of change (absolute delta per second). --
                let prev = self.sc_env_prev[channel];
                let rate = (env - prev).abs() / dt.max(1e-6);
                self.sc_env_prev[channel] = env;

                // -- Per-bin mass write: high rate → heavier mass. --
                for k in 0..num_bins {
                    // Inner MASS clamp is 100 (not 1000 like Static) so that the rate-multiplied
                    // product can hit the outer 1000 ceiling without immediately saturating: we
                    // reserve dynamic range for the rate term.
                    // `1.0 +` baseline: at zero rate, target = MASS_curve (mass never drops below
                    // MASS_curve); rate × scale lifts it higher when SC is changing fast.
                    let target = ((1.0 + SC_MASS_RATE_SCALE * rate) * mass_curve[k].clamp(0.01, 100.0))
                        .clamp(0.01, 1000.0);
                    let mix    = mix_curve[k].clamp(0.0, 1.0);
                    let cur    = physics.mass[k];
                    physics.mass[k] = cur * (1.0 - mix) + target * mix;
                }
            }
        }
        // bins are not modified — InertialMass only writes physics.mass.
    }

    /// Orbital phase rotation — peak-driven, paired-bin linear phase shift.
    ///
    /// Detects up to `MAX_PEAKS` spectral peaks (local maxima that exceed both neighbours
    /// AND exceed 2× the local-window mean over `ORBITAL_SAT_HALF_WINDOW` bins).
    ///
    /// For each peak at bin `km` with amplitude `m_amp` and each satellite distance
    /// `d ∈ 1..=ORBITAL_SAT_HALF_WINDOW`:
    ///
    /// - **+d satellite** (`kp = km + d`): `Δφ = +α * m_amp / d²`
    /// - **-d satellite** (`kn = km - d`): `Δφ = -α * m_amp / d²`  (opposite sign)
    ///
    /// where `α = 0.5 * STRENGTH[km] * dt`. The master peak bin itself is NOT rotated.
    /// MIX curve blends the rotation amount before applying the complex rotation.
    fn apply_orbital_phase(
        &mut self,
        channel: usize,
        bins: &mut [Complex<f32>],
        dt: f32,
        num_bins: usize,
        _physics: Option<&BinPhysics>,
    ) {
        // Bind local slice refs — avoids triple-indexing through self inside inner loops
        // and keeps borrows disjoint from the bins mutation in Pass B.
        let strength_curve = &self.smoothed_curves[channel][0][..num_bins];
        let mix_curve      = &self.smoothed_curves[channel][4][..num_bins];

        // -- Pass A: Find peaks. --
        // A bin qualifies as a master peak if:
        //   1. magnitude > both immediate neighbours (local maximum)
        //   2. magnitude > 2× the mean over the local ORBITAL_SAT_HALF_WINDOW window
        // SmallVec stays stack-allocated up to MAX_PEAKS = 16 entries; no heap allocation.
        let mut peaks: SmallVec<[(usize, f32); MAX_PEAKS]> = SmallVec::new();
        for k in 1..(num_bins - 1) {
            let m = bins[k].norm();
            if m < 1e-6 { continue; }
            let left  = bins[k - 1].norm();
            let right = bins[k + 1].norm();
            if m > left && m > right {
                // Window-mean check: allocation-free iterator sum, compiles to a plain loop.
                let lo   = k.saturating_sub(ORBITAL_SAT_HALF_WINDOW);
                let hi   = (k + ORBITAL_SAT_HALF_WINDOW).min(num_bins - 1);
                let mean = (lo..=hi).map(|i| bins[i].norm()).sum::<f32>() / (hi - lo + 1) as f32;
                if m > ORBITAL_PEAK_THRESHOLD_FACTOR * mean {
                    if peaks.len() < MAX_PEAKS {
                        peaks.push((k, m));
                    }
                }
            }
        }
        if peaks.is_empty() { return; }

        // -- Pass B: Apply phase rotation to satellites around each peak. --
        // Bins are read+mutated here; the peak list (Pass A) is complete so there is
        // no conflict between the read of bins[k].norm() above and the write below.
        for &(km, m_amp) in peaks.iter() {
            let alpha = 0.5 * strength_curve[km] * dt;
            // Upper bound for d: stay within the array AND respect ORBITAL_SAT_HALF_WINDOW.
            // d_max derivation guarantees kp = km + d ≤ num_bins - 1, so no bounds guard is
            // needed on the +d satellite block.
            let d_max = ORBITAL_SAT_HALF_WINDOW.min(
                num_bins.saturating_sub(km).max(1) - 1
            );
            for d in 1..=d_max {
                // d ≥ 1 in this loop, so d² ≥ 1 — no defensive .max(1.0) needed.
                let denom = (d as f32) * (d as f32);

                // +d satellite: rotate by +Δφ
                let kp = km + d;
                let dphi_pos = alpha * m_amp / denom;
                let mix      = mix_curve[kp].clamp(0.0, 1.0);
                let dphi     = dphi_pos * mix;
                let (c, s)   = (dphi.cos(), dphi.sin());
                let re = bins[kp].re * c - bins[kp].im * s;
                let im = bins[kp].re * s + bins[kp].im * c;
                bins[kp].re = re;
                bins[kp].im = im;

                // -d satellite: rotate by -Δφ (opposite sign)
                if km >= d {
                    let kn       = km - d;
                    let dphi_neg = -alpha * m_amp / denom;
                    let mix      = mix_curve[kn].clamp(0.0, 1.0);
                    let dphi     = dphi_neg * mix;
                    let (c, s)   = (dphi.cos(), dphi.sin());
                    let re = bins[kn].re * c - bins[kn].im * s;
                    let im = bins[kn].re * s + bins[kn].im * c;
                    bins[kn].re = re;
                    bins[kn].im = im;
                }
            }
        }
    }

    /// Ferromagnetism — peak-attracted phase alignment.
    ///
    /// **Pass A** detects up to `MAX_PEAKS` spectral peaks (local maxima that exceed both
    /// immediate neighbours AND exceed `ORBITAL_PEAK_THRESHOLD_FACTOR` × the local-window
    /// mean over `FERRO_PEAK_WINDOW_HALF` bins on each side).
    ///
    /// **Pass B** pulls satellite bins' phases toward the master peak's phase.  For each
    /// peak at bin `km` with phase `target_phase` and each distance `d ∈ 1..=reach_bins`:
    ///
    /// ```text
    /// weight   = exp(-d / reach_bins)            // exponential decay with distance
    /// pull     = clamp(α * weight / (1+resistance), 0, FERRO_PULL_CAP)
    /// new_phase = cur_phase + phase_diff_wrapped(target, cur) * pull * mix
    /// ```
    ///
    /// where `α = STRENGTH[km] * FERRO_ALPHA_SCALE`.  Magnitudes are preserved; only
    /// phase is rotated.  MIX curve blends the pull amount (0 = dry phase, 1 = full pull).
    ///
    /// **Limitation**: When two peaks' satellite spheres overlap, pulls are applied
    /// sequentially in peak-detection order — the second peak reads the magnitudes/
    /// phases the first peak already wrote, so its pull is non-linear and order-
    /// dependent. A proper blend would require a scratch accumulator buffer; deferred
    /// to v2 since dense overlapping peaks are uncommon in practice.
    fn apply_ferromagnetism(
        &mut self,
        channel: usize,
        bins: &mut [Complex<f32>],
        _dt: f32,
        num_bins: usize,
        _physics: Option<&BinPhysics>,
    ) {
        use std::f32::consts::PI;

        // Bind local slice refs — avoids triple-indexing through self inside inner loops
        // and keeps borrows disjoint from the bins mutation in Pass B.
        let strength_curve = &self.smoothed_curves[channel][0][..num_bins];
        let reach_curve    = &self.smoothed_curves[channel][2][..num_bins];
        let damping_curve  = &self.smoothed_curves[channel][3][..num_bins];
        let mix_curve      = &self.smoothed_curves[channel][4][..num_bins];

        // -- Pass A: Find peaks. --
        // A bin qualifies as a master peak if:
        //   1. magnitude > both immediate neighbours (local maximum)
        //   2. magnitude > ORBITAL_PEAK_THRESHOLD_FACTOR × the mean over the local
        //      FERRO_PEAK_WINDOW_HALF window
        // SmallVec stays stack-allocated up to MAX_PEAKS = 16 entries; no heap allocation.
        // Tuple: (bin_index, magnitude, phase)
        let mut peaks: SmallVec<[(usize, f32, f32); MAX_PEAKS]> = SmallVec::new();
        for k in 1..(num_bins - 1) {
            let m = bins[k].norm();
            if m < 1e-6 { continue; }
            let left  = bins[k - 1].norm();
            let right = bins[k + 1].norm();
            if m > left && m > right {
                // Window-mean check: allocation-free iterator sum over the local window.
                let lo   = k.saturating_sub(FERRO_PEAK_WINDOW_HALF);
                let hi   = (k + FERRO_PEAK_WINDOW_HALF).min(num_bins - 1);
                let mean = (lo..=hi).map(|i| bins[i].norm()).sum::<f32>() / (hi - lo + 1) as f32;
                if m > ORBITAL_PEAK_THRESHOLD_FACTOR * mean {
                    if peaks.len() < MAX_PEAKS {
                        peaks.push((k, m, bins[k].arg()));
                    }
                }
            }
        }
        if peaks.is_empty() { return; }

        // -- Pass B: Pull satellite phases toward each peak's phase. --
        // Bins are read+mutated here; the peak list (Pass A) is complete so there is
        // no conflict between the norm/arg reads above and the writes below.
        for &(km, _m_amp, target_phase) in peaks.iter() {
            let reach_bins = (reach_curve[km].clamp(0.1, 4.0) * FERRO_REACH_SCALE).round() as usize;
            let alpha      = strength_curve[km].clamp(0.0, 2.0) * FERRO_ALPHA_SCALE;
            let resistance = damping_curve[km].clamp(0.0, 2.0);

            for d in 1..=reach_bins {
                // Exponential decay: weight = exp(-d / reach_bins).
                // .max(1) is defensive; reach_bins is already ≥ 2 given the clamp(0.1, 4.0) * 16.0 formula above.
                let weight = (-(d as f32) / reach_bins.max(1) as f32).exp();
                let pull   = (alpha * weight / (1.0 + resistance)).min(FERRO_PULL_CAP);

                // +d satellite
                let kp = km + d;
                if kp < num_bins {
                    let cur_mag = bins[kp].norm();
                    let cur_ph  = bins[kp].arg();
                    let mix     = mix_curve[kp].clamp(0.0, 1.0);
                    // Shortest-arc phase difference, wrapped to (-π, π].
                    // .arg() returns phase in (-π, π], so diff ∈ (-2π, 2π]; each while loop runs at most once.
                    let mut diff = target_phase - cur_ph;
                    while diff >  PI { diff -= 2.0 * PI; }
                    while diff < -PI { diff += 2.0 * PI; }
                    let new_ph  = cur_ph + diff * pull * mix;
                    bins[kp].re = cur_mag * new_ph.cos();
                    bins[kp].im = cur_mag * new_ph.sin();
                }

                // -d satellite
                if km >= d {
                    let kn      = km - d;
                    let cur_mag = bins[kn].norm();
                    let cur_ph  = bins[kn].arg();
                    let mix     = mix_curve[kn].clamp(0.0, 1.0);
                    let mut diff = target_phase - cur_ph;
                    while diff >  PI { diff -= 2.0 * PI; }
                    while diff < -PI { diff += 2.0 * PI; }
                    let new_ph  = cur_ph + diff * pull * mix;
                    bins[kn].re = cur_mag * new_ph.cos();
                    bins[kn].im = cur_mag * new_ph.sin();
                }
            }
        }
    }

    /// Thermal expansion — temperature accumulation drives a frequency-detune phase rotation.
    ///
    /// Heat: `temp[k] = ((temp[k] + STRENGTH·|mag|²·dt) · (1 − DAMPING·2·dt).max(0)).min(CEILING)`.
    /// Detune: `Δφ = 2π · DETUNE_HZ_PER_TEMP · temp[k] · dt`, scaled by `MIX.clamp(0,1)`.
    /// If `physics` is `Some`, `p.temperature[k]` gets a 50/50 blend with `temp[k]`.
    #[allow(clippy::too_many_arguments)]
    fn apply_thermal_expansion(
        &mut self,
        channel: usize,
        bins: &mut [Complex<f32>],
        dt: f32,
        num_bins: usize,
        physics: Option<&mut BinPhysics>,
    ) {
        use std::f32::consts::PI;

        let strength_curve = &self.smoothed_curves[channel][0][..num_bins];
        let damping_curve  = &self.smoothed_curves[channel][3][..num_bins];
        let mix_curve      = &self.smoothed_curves[channel][4][..num_bins];
        let dry_mag        = &self.dry_mag_scratch[channel][..num_bins];

        // 1. Heat update. cool_factor is applied AFTER adding heat_in (plan §10.3).
        let temp = &mut self.temperature_local[channel][..num_bins];
        for k in 0..num_bins {
            let heat_in   = strength_curve[k].clamp(0.0, THERMAL_HEAT_STRENGTH_CLAMP_HI)
                            * dry_mag[k] * dry_mag[k] * dt;
            let cool_rate = damping_curve[k].clamp(0.0, THERMAL_DAMPING_CLAMP_HI)
                            * THERMAL_COOL_RATE_SCALE;
            let cool_factor = (1.0 - cool_rate * dt).max(0.0);
            temp[k] = ((temp[k] + heat_in) * cool_factor).min(THERMAL_TEMP_CEILING);
        }

        // 2. Phase rotation. Hoist the per-hop invariant: dphi = dphi_per_unit · temp[k].
        let dphi_per_unit = 2.0 * PI * THERMAL_DETUNE_HZ_PER_TEMP * dt;
        for k in 0..num_bins {
            let dphi_mixed = dphi_per_unit * temp[k] * mix_curve[k].clamp(0.0, 1.0);
            let (c, s)     = (dphi_mixed.cos(), dphi_mixed.sin());
            let re = bins[k].re * c - bins[k].im * s;
            let im = bins[k].re * s + bins[k].im * c;
            bins[k].re = re;
            bins[k].im = im;
        }

        // 3. Mirror local temperature into BinPhysics (50/50 blend with upstream).
        if let Some(p) = physics {
            for k in 0..num_bins {
                p.temperature[k] = (1.0 - THERMAL_PHYSICS_BLEND) * p.temperature[k]
                                  + THERMAL_PHYSICS_BLEND * temp[k];
            }
        }
    }

    /// Tuning fork resonance — sympathetic frequency clusters.
    ///
    /// Each hop:
    /// 1. Re-detect local-magnitude peaks that exceed `TUNING_FORK_MIN_MAG` and have
    ///    STRENGTH-curve value > `TUNING_FORK_STRENGTH_THRESHOLD`, spaced at least
    ///    `TUNING_FORK_MIN_SEP` bins apart, up to `MAX_TUNING_FORKS` (16) total.
    /// 2. For each detected fork at bin `kf` with frequency `freq`, advance a per-bin
    ///    phase carrier in `displacement[side]` by `2π·freq·dt` for every neighbour bin
    ///    `side` within `reach_bins` radius, then apply `sin(carrier)` phase rotation
    ///    scaled by `modulation_depth · (1/distance) · mix`.
    ///
    /// No allocation, no probe write (probe gathered in `process()` Step 6).
    fn apply_tuning_fork(
        &mut self,
        channel: usize,
        bins: &mut [Complex<f32>],
        dt: f32,
        num_bins: usize,
        _physics: Option<&BinPhysics>,
    ) {
        use std::f32::consts::PI;

        // Hoist curve slice refs before any &mut borrows.
        let strength_curve = &self.smoothed_curves[channel][0][..num_bins];
        let reach_curve    = &self.smoothed_curves[channel][2][..num_bins];
        let mix_curve      = &self.smoothed_curves[channel][4][..num_bins];

        // -- 1. Re-detect forks each hop: loud peaks above threshold, with min separation. --
        self.tuning_forks[channel].clear();
        let mut last_pick: isize = -(TUNING_FORK_MIN_SEP as isize) - 1;
        for k in 1..(num_bins - 1) {
            let m = bins[k].norm();
            if m < TUNING_FORK_MIN_MAG { continue; }
            // STRENGTH-curve gating: only bins where STRENGTH is elevated register as forks.
            if strength_curve[k] < TUNING_FORK_STRENGTH_THRESHOLD { continue; }
            if m > bins[k - 1].norm() && m > bins[k + 1].norm()
                && (k as isize - last_pick) >= TUNING_FORK_MIN_SEP as isize
            {
                // Convert bin index to Hz using actual sample rate / fft size.
                let freq = (k as f32) * (self.sample_rate / self.fft_size as f32);
                if self.tuning_forks[channel].len() < MAX_TUNING_FORKS {
                    self.tuning_forks[channel].push((k, freq));
                    last_pick = k as isize;
                }
            }
        }

        if self.tuning_forks[channel].is_empty() { return; }

        // -- 2. For each fork, modulate phase of bins within REACH by sin(carrier). --
        //    One carrier per fork, stored in displacement[kf] and advanced once per hop.
        //    Phase advance = 2π·k/overlap (fractional cycles). When k is an integer
        //    multiple of the overlap (e.g. k=300, overlap=4 → exactly 75 full cycles),
        //    the advance rem_euclid(2π) is zero and the carrier would stall. A small
        //    per-fork nudge (TUNING_FORK_CARRIER_NUDGE × fork_idx) prevents this without
        //    audibly colouring the modulation rate.
        //    SmallVec stack-only clone (capacity = MAX_TUNING_FORKS = 16) — no heap alloc.
        let forks = self.tuning_forks[channel].clone();
        let displacement = &mut self.displacement[channel][..num_bins];

        for (fork_idx, (kf, freq)) in forks.iter().enumerate() {
            let kf = *kf;
            let freq = *freq;
            let reach_bins = (reach_curve[kf].clamp(0.1, TUNING_FORK_REACH_CLAMP_HI)
                              * TUNING_FORK_REACH_BIN_SCALE)
                             .round() as usize;
            let modulation_depth = (strength_curve[kf]
                                        .clamp(0.0, TUNING_FORK_STRENGTH_MAX) - 1.0)
                                   .max(0.0) * TUNING_FORK_DEPTH_SCALE;
            // Carrier phase advance: fractional-cycle part of 2π·freq·dt, plus nudge to
            // prevent stall when freq·dt is an integer (bin at integer-overlap-multiple).
            let raw_advance = (2.0 * PI * freq * dt).rem_euclid(2.0 * PI);
            // Index is 1-based so even the first fork gets a non-zero nudge.
            let nudge = TUNING_FORK_CARRIER_NUDGE * (fork_idx + 1) as f32;
            let phase_advance = raw_advance + nudge;
            // Advance the fork's own carrier (displacement[kf]) once per hop; wrap to (−π, π].
            displacement[kf] = (displacement[kf] + phase_advance).rem_euclid(2.0 * PI) - PI;
            let carrier_sin = displacement[kf].sin();

            for d in 1..=reach_bins {
                let weight = 1.0 / d as f32;
                // Process both sides: bin at kf+d and bin at kf-d (saturating sub guards ≥0).
                let sides = [kf.saturating_add(d), kf.saturating_sub(d)];
                for side in sides {
                    if side >= num_bins { continue; }
                    let mix = mix_curve[side].clamp(0.0, 1.0);
                    let dphi = modulation_depth * weight * carrier_sin * mix;
                    let (c, s) = (dphi.cos(), dphi.sin());
                    let re = bins[side].re * c - bins[side].im * s;
                    let im = bins[side].re * s + bins[side].im * c;
                    bins[side].re = re;
                    bins[side].im = im;
                }
            }
        }
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
            if c >= curves.len() {
                // No curve provided — hold previous smoothed value.
                continue;
            }
            debug_assert!(
                curves[c].len() >= num_bins,
                "kinetics: curve {} length {} < num_bins {}",
                c, curves[c].len(), num_bins
            );
            let src = &curves[c][..num_bins];
            let dst = &mut self.smoothed_curves[channel][c][..num_bins];
            smooth_curve_one_pole(dst, src, dt);
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
            let strength     = &self.smoothed_curves[channel][0][..num_bins];
            let velocity     = &self.velocity[channel][..num_bins];
            let displacement = &self.displacement[channel][..num_bins];
            let curr_kepe    = &mut self.mag_prev[channel][..num_bins];
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
                well_count:            match self.mode {
                    KineticsMode::GravityWell => self.last_well_count[channel],
                    KineticsMode::TuningFork  => self.tuning_forks[channel].len() as u16,
                    _ => 0,
                },
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
                // MIX (curve 4) initialises to 0.0 so a cold-start with MIX curve=0
                // gives exact passthrough immediately. Physical params (0-3) initialise
                // to 1.0 (the neutral value for STRENGTH, MASS, REACH, DAMPING).
                let init = if c == 4 { 0.0 } else { 1.0 };
                self.smoothed_curves[ch][c].resize(num_bins, init);
            }
            self.tuning_forks[ch].clear();
            self.sc_env_smoothed[ch] = 0.0;
            self.sc_env_prev[ch] = 0.0;
            self.dry_mag_scratch[ch].clear();
            self.dry_mag_scratch[ch].resize(num_bins, 0.0);
        }
        self.rng_state = [0xC0FF_EE01, 0xBADD_CAFE];
        self.last_well_count = [0; 2];
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
