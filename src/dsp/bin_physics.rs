//! Per-bin persistent physics state, transported through FxMatrix between slots.
//!
//! See `docs/superpowers/specs/2026-04-21-bin-physics-infrastructure.md` for the
//! original design and `ideas/next-gen-modules/01-global-infrastructure.md § 1`
//! for the audit additions (slew/bias/decay_estimate/lock_target_freq + per-field
//! merge rules).

use num_complex::Complex;
use crate::dsp::pipeline::MAX_NUM_BINS;

/// Per-field rule used when multiple sends mix into one destination slot's input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MergeRule {
    /// Standard amplitude-weighted average (matches audio-bin mixing).
    /// Use for: temperature, flux, displacement, phase_momentum, slew, bias,
    /// decay_estimate, lock_target_freq.
    WeightedAvg,
    /// Take the higher of the two values. Reasoning: harder to break than to form.
    /// Use for: crystallization.
    Max,
    /// Take the higher mass — heavier parent dominates.
    /// Use for: mass.
    HeavierWins,
}

pub struct BinPhysics {
    pub velocity:        Vec<f32>,   // auto-computed each hop, never stored
    pub mass:            Vec<f32>,   // default 1.0
    pub temperature:     Vec<f32>,
    pub flux:            Vec<f32>,
    pub displacement:    Vec<f32>,
    pub crystallization: Vec<f32>,   // [0, 1]
    pub phase_momentum:  Vec<f32>,
    pub slew:            Vec<f32>,
    pub bias:            Vec<f32>,
    pub decay_estimate:  Vec<f32>,
    pub lock_target_freq: Vec<f32>,
}

impl BinPhysics {
    pub fn new() -> Self {
        Self {
            velocity:        vec![0.0; MAX_NUM_BINS],
            mass:            vec![1.0; MAX_NUM_BINS],
            temperature:     vec![0.0; MAX_NUM_BINS],
            flux:            vec![0.0; MAX_NUM_BINS],
            displacement:    vec![0.0; MAX_NUM_BINS],
            crystallization: vec![0.0; MAX_NUM_BINS],
            phase_momentum:  vec![0.0; MAX_NUM_BINS],
            slew:            vec![0.0; MAX_NUM_BINS],
            bias:            vec![0.0; MAX_NUM_BINS],
            decay_estimate:  vec![0.0; MAX_NUM_BINS],
            lock_target_freq: vec![0.0; MAX_NUM_BINS],
        }
    }

    /// Reset the active region to defaults. `lock_target_freq[k]` becomes the
    /// bin-centre frequency `k * sample_rate / fft_size`.
    pub fn reset_active(&mut self, num_bins: usize, sample_rate: f32, fft_size: usize) {
        self.velocity[..num_bins].fill(0.0);
        self.mass[..num_bins].fill(1.0);
        self.temperature[..num_bins].fill(0.0);
        self.flux[..num_bins].fill(0.0);
        self.displacement[..num_bins].fill(0.0);
        self.crystallization[..num_bins].fill(0.0);
        self.phase_momentum[..num_bins].fill(0.0);
        self.slew[..num_bins].fill(0.0);
        self.bias[..num_bins].fill(0.0);
        self.decay_estimate[..num_bins].fill(0.0);
        let bin_hz = sample_rate / fft_size as f32;
        for k in 0..num_bins {
            self.lock_target_freq[k] = k as f32 * bin_hz;
        }
    }

    /// Apply a single send into a destination value using the merge rule.
    /// `weight` is the send amplitude clamped to [0, 1].
    #[inline]
    pub fn merge_one(dst: &mut f32, src: f32, weight: f32, rule: MergeRule) {
        let w = weight.clamp(0.0, 1.0);
        match rule {
            MergeRule::WeightedAvg => *dst = *dst * (1.0 - w) + src * w,
            MergeRule::Max         => { if w > 0.0 { *dst = dst.max(src); } }
            MergeRule::HeavierWins => { if w > 0.0 && src > *dst { *dst = src; } }
        }
    }

    /// Compute per-bin velocity from the magnitude delta between previous and
    /// current FFT frames. Velocity is the absolute change in magnitude per hop.
    pub fn compute_velocity(
        out_velocity: &mut [f32],
        prev_bins:    &[Complex<f32>],
        curr_bins:    &[Complex<f32>],
        num_bins:     usize,
    ) {
        for k in 0..num_bins {
            let prev_mag = prev_bins[k].norm();
            let curr_mag = curr_bins[k].norm();
            out_velocity[k] = (curr_mag - prev_mag).abs();
        }
    }

    /// Mix `other` into `self` with the given send weight, per per-field rule.
    pub fn mix_from(&mut self, other: &BinPhysics, weight: f32, num_bins: usize) {
        for k in 0..num_bins {
            Self::merge_one(&mut self.mass[k],            other.mass[k],            weight, MergeRule::HeavierWins);
            Self::merge_one(&mut self.temperature[k],     other.temperature[k],     weight, MergeRule::WeightedAvg);
            Self::merge_one(&mut self.flux[k],            other.flux[k],            weight, MergeRule::WeightedAvg);
            Self::merge_one(&mut self.displacement[k],    other.displacement[k],    weight, MergeRule::WeightedAvg);
            Self::merge_one(&mut self.crystallization[k], other.crystallization[k], weight, MergeRule::Max);
            Self::merge_one(&mut self.phase_momentum[k],  other.phase_momentum[k],  weight, MergeRule::WeightedAvg);
            Self::merge_one(&mut self.slew[k],            other.slew[k],            weight, MergeRule::WeightedAvg);
            Self::merge_one(&mut self.bias[k],            other.bias[k],            weight, MergeRule::WeightedAvg);
            Self::merge_one(&mut self.decay_estimate[k],  other.decay_estimate[k],  weight, MergeRule::WeightedAvg);
            Self::merge_one(&mut self.lock_target_freq[k], other.lock_target_freq[k], weight, MergeRule::WeightedAvg);
        }
        // velocity is recomputed downstream — do not mix it here.
    }

    /// Element-wise copy of the active region. Used by FxMatrix to snapshot the
    /// per-slot output state after `module.process()` returns.
    pub fn copy_from(&mut self, src: &BinPhysics, num_bins: usize) {
        self.velocity[..num_bins].copy_from_slice(&src.velocity[..num_bins]);
        self.mass[..num_bins].copy_from_slice(&src.mass[..num_bins]);
        self.temperature[..num_bins].copy_from_slice(&src.temperature[..num_bins]);
        self.flux[..num_bins].copy_from_slice(&src.flux[..num_bins]);
        self.displacement[..num_bins].copy_from_slice(&src.displacement[..num_bins]);
        self.crystallization[..num_bins].copy_from_slice(&src.crystallization[..num_bins]);
        self.phase_momentum[..num_bins].copy_from_slice(&src.phase_momentum[..num_bins]);
        self.slew[..num_bins].copy_from_slice(&src.slew[..num_bins]);
        self.bias[..num_bins].copy_from_slice(&src.bias[..num_bins]);
        self.decay_estimate[..num_bins].copy_from_slice(&src.decay_estimate[..num_bins]);
        self.lock_target_freq[..num_bins].copy_from_slice(&src.lock_target_freq[..num_bins]);
    }
}

impl Default for BinPhysics {
    fn default() -> Self { Self::new() }
}
