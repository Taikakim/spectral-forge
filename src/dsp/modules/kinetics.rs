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
/// Strength curve must exceed this baseline to register as a static gravity well.
const STATIC_WELL_BASELINE: f32 = 1.05;
/// Sidechain peak must reach this fraction of the per-hop max to register as a well.
const SC_WELL_THRESHOLD_FRAC: f32 = 0.4;

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
