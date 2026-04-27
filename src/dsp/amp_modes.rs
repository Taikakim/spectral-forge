use serde::{Deserialize, Serialize};

/// Per-cell amp mode for the routing matrix. Each cell of `RouteMatrix` carries
/// one of these to select what kind of non-linear processing is applied to the
/// signal as it travels along that send.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum AmpMode {
    #[default]
    Linear,
    Vactrol,
    Schmitt,
    Slew,
    Stiction,
}

impl AmpMode {
    /// Human-readable name; used by the cell popup and any future tooltips.
    pub fn label(self) -> &'static str {
        match self {
            AmpMode::Linear   => "Linear",
            AmpMode::Vactrol  => "Vactrol",
            AmpMode::Schmitt  => "Schmitt",
            AmpMode::Slew     => "Slew",
            AmpMode::Stiction => "Stiction",
        }
    }
}

/// Per-cell numeric parameters shared across all amp modes. Each mode reads only
/// the subset relevant to it (e.g. `Vactrol` ignores `slew_db_per_s`).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct AmpCellParams {
    /// Strength of the amp effect. 0 = bypass, 1 = full, >1 = exaggerated. Range 0..2.
    pub amount: f32,
    /// Schmitt on-threshold (0..1 magnitude); also the Stiction step size.
    pub threshold: f32,
    /// Vactrol release time in milliseconds.
    pub release_ms: f32,
    /// Slew maximum change rate in dB per second.
    pub slew_db_per_s: f32,
}

// Manual `Default` impl rather than `#[derive(Default)]`: every field has a
// non-zero neutral value, and `f32::default()` would silently produce a fully-
// silent cell. Keeping the values inline here also serves as the spec.
impl Default for AmpCellParams {
    fn default() -> Self {
        Self { amount: 1.0, threshold: 0.5, release_ms: 100.0, slew_db_per_s: 60.0 }
    }
}

use num_complex::Complex;

/// Per-cell DSP state. One of these per (row, col) per channel in the matrix.
/// State arrays are sized to `num_bins` at construction; the audio thread later
/// calls `resize` (inside `permit_alloc`) if the FFT size changes.
#[derive(Debug)]
pub enum AmpNodeState {
    Linear,
    Vactrol  { cap: Vec<f32> },
    Schmitt  { latch: Vec<bool> },
    Slew     { current_db: Vec<f32> },
    Stiction { accumulator: Vec<f32>, last_out: Vec<f32> },
}

impl AmpNodeState {
    /// Construct state for `mode`. Allocates per-bin arrays for non-Linear modes.
    /// Caller is responsible for invoking inside `permit_alloc!` if on the audio thread.
    pub fn new(mode: AmpMode, num_bins: usize) -> Self {
        match mode {
            AmpMode::Linear   => AmpNodeState::Linear,
            AmpMode::Vactrol  => AmpNodeState::Vactrol  { cap: vec![0.0; num_bins] },
            AmpMode::Schmitt  => AmpNodeState::Schmitt  { latch: vec![false; num_bins] },
            AmpMode::Slew     => AmpNodeState::Slew     { current_db: vec![-120.0; num_bins] },
            AmpMode::Stiction => AmpNodeState::Stiction {
                accumulator: vec![0.0; num_bins],
                last_out:    vec![0.0; num_bins],
            },
        }
    }

    /// True if this state matches the given mode (used to detect mode changes).
    pub fn matches(&self, mode: AmpMode) -> bool {
        matches!(
            (self, mode),
            (AmpNodeState::Linear,   AmpMode::Linear)
            | (AmpNodeState::Vactrol  { .. }, AmpMode::Vactrol)
            | (AmpNodeState::Schmitt  { .. }, AmpMode::Schmitt)
            | (AmpNodeState::Slew     { .. }, AmpMode::Slew)
            | (AmpNodeState::Stiction { .. }, AmpMode::Stiction)
        )
    }

    /// Reset all internal state arrays to startup values, but keep allocations.
    pub fn clear(&mut self) {
        match self {
            AmpNodeState::Linear => {}
            AmpNodeState::Vactrol  { cap }      => cap.fill(0.0),
            AmpNodeState::Schmitt  { latch }    => latch.fill(false),
            AmpNodeState::Slew     { current_db } => current_db.fill(-120.0),
            AmpNodeState::Stiction { accumulator, last_out } => {
                accumulator.fill(0.0);
                last_out.fill(0.0);
            }
        }
    }

    /// Resize state arrays for a new fft size. Allocates if growing; cheap if same.
    /// Must be called inside `permit_alloc!` if on the audio thread.
    pub fn resize(&mut self, num_bins: usize) {
        match self {
            AmpNodeState::Linear => {}
            AmpNodeState::Vactrol  { cap }        => cap.resize(num_bins, 0.0),
            AmpNodeState::Schmitt  { latch }      => latch.resize(num_bins, false),
            AmpNodeState::Slew     { current_db } => current_db.resize(num_bins, -120.0),
            AmpNodeState::Stiction { accumulator, last_out } => {
                accumulator.resize(num_bins, 0.0);
                last_out.resize(num_bins, 0.0);
            }
        }
    }

    /// Apply this amp's transform to `buf` in place.
    /// `buf.len()` must equal the state's array length.
    /// `hop_dt` is the audio time elapsed per hop in seconds.
    pub fn apply(&mut self, p: &AmpCellParams, buf: &mut [Complex<f32>], hop_dt: f32) {
        match self {
            AmpNodeState::Linear => apply_linear(p, buf),
            AmpNodeState::Vactrol  { cap }            => apply_vactrol(p, buf, cap, hop_dt),
            AmpNodeState::Schmitt  { latch }          => apply_schmitt(p, buf, latch),
            AmpNodeState::Slew     { current_db }     => apply_slew(p, buf, current_db, hop_dt),
            AmpNodeState::Stiction { accumulator, last_out }
                                                      => apply_stiction(p, buf, accumulator, last_out),
        }
    }
}

// ── Kernels ────────────────────────────────────────────────────────────────

fn apply_linear(p: &AmpCellParams, buf: &mut [Complex<f32>]) {
    if (p.amount - 1.0).abs() < 1e-6 { return; }
    for c in buf.iter_mut() { *c *= p.amount; }
}

/// Vactrol: capacitor charges fast (~1 ms time constant) on input, releases slowly.
/// The capacitor level then becomes a gain modulator on the next sample (LDR behaviour).
fn apply_vactrol(p: &AmpCellParams, buf: &mut [Complex<f32>], cap: &mut [f32], hop_dt: f32) {
    const ATTACK_MS: f32 = 1.0;
    let attack_a  = (-hop_dt / (ATTACK_MS * 0.001)).exp();
    let release_a = (-hop_dt / (p.release_ms * 0.001)).exp();
    for (c, cap_k) in buf.iter_mut().zip(cap.iter_mut()) {
        let mag = c.norm();
        if mag > *cap_k {
            *cap_k = attack_a * *cap_k + (1.0 - attack_a) * mag;
        } else {
            *cap_k = release_a * *cap_k + (1.0 - release_a) * mag;
        }
        let gain = (*cap_k).clamp(0.0, 1.0).powf(0.6);
        let blend = 1.0 - p.amount + p.amount * gain;
        *c *= blend;
    }
}

/// Schmitt: per-bin two-threshold latch. ON threshold = `p.threshold`, OFF threshold = 0.7 × ON.
fn apply_schmitt(p: &AmpCellParams, buf: &mut [Complex<f32>], latch: &mut [bool]) {
    let on_th  = p.threshold;
    let off_th = p.threshold * 0.7;
    for (c, l) in buf.iter_mut().zip(latch.iter_mut()) {
        let mag = c.norm();
        if !*l && mag >= on_th  { *l = true; }
        if *l  && mag <  off_th { *l = false; }
        if !*l { *c = Complex::new(0.0, 0.0); }
        else if (p.amount - 1.0).abs() > 1e-6 { *c *= p.amount; }
    }
}

/// Slew: per-bin magnitude can only change at most `slew_db_per_s` per second.
/// Phase is preserved.
fn apply_slew(p: &AmpCellParams, buf: &mut [Complex<f32>], current_db: &mut [f32], hop_dt: f32) {
    let max_step_db = p.slew_db_per_s * hop_dt;
    for (c, cur_db) in buf.iter_mut().zip(current_db.iter_mut()) {
        let mag = c.norm().max(1e-12);
        let target_db = 20.0 * mag.log10();
        let delta = (target_db - *cur_db).clamp(-max_step_db, max_step_db);
        *cur_db = (*cur_db + delta).max(-120.0);
        let new_mag = 10f32.powf(*cur_db * 0.05);
        let new_gain = (new_mag / mag) * p.amount + (1.0 - p.amount);
        *c *= new_gain;
    }
}

/// Stiction: dead-zone. Output stays put until accumulated change exceeds `threshold`,
/// then output snaps to current input and accumulator resets.
fn apply_stiction(p: &AmpCellParams, buf: &mut [Complex<f32>], accumulator: &mut [f32], last_out: &mut [f32]) {
    let th = p.threshold.max(1e-6);
    for (c, (acc, lo)) in buf.iter_mut().zip(accumulator.iter_mut().zip(last_out.iter_mut())) {
        let mag = c.norm();
        let delta = (mag - *lo).abs();
        *acc += delta;
        if *acc >= th {
            *lo  = mag;
            *acc = 0.0;
        }
        let phase_unit = if mag > 1e-12 { *c / mag } else { Complex::new(0.0, 0.0) };
        let out_mag = *lo * p.amount + mag * (1.0 - p.amount);
        *c = phase_unit * out_mag;
    }
}
