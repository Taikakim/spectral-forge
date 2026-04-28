//! Per-channel rolling complex-spectrum history buffer.
//!
//! Lives in `Pipeline`, written each STFT hop after the analysis FFT and
//! before `FxMatrix::process_hop`. Modules read it via `ctx.history:
//! Option<&HistoryBuffer>`.
//!
//! Layout:
//!   - One ring of `capacity_frames` frames per audio channel.
//!   - Each frame is a `Vec<Complex<f32>>` of length `num_bins`.
//!   - Frames are written at `write_pos` (one shared write head across all
//!     channels — the pipeline writes channel 0 first, then channel 1, then
//!     calls `advance_after_all_channels_written()`).
//!
//! Read API: `read_frame(channel, age_frames)` returns the spectrum that was
//! current `age_frames` hops ago (age 0 = most recent), or `None` if the
//! requested age exceeds `frames_used()`. `read_fractional` interpolates
//! between adjacent frames for sub-hop precision (used by the future
//! Past Stretch sub-effect).
//!
//! Lazy summary stats (decay/RMS/IF stability) are computed on first
//! request per block via interior mutability and cleared at the top of
//! every block by `clear_summary_cache()`. See § "Summary stats" below.

use num_complex::Complex;
use std::cell::RefCell;

pub struct HistoryBuffer {
    /// `frames[channel][frame_idx][bin]`. All channels share `write_pos`.
    frames: Vec<Vec<Vec<Complex<f32>>>>,
    capacity_frames: usize,
    num_bins: usize,
    num_channels: usize,
    write_pos: usize,
    frames_used: usize,
    summary: RefCell<SummaryCache>,
}

#[derive(Default)]
struct SummaryCache {
    decay_estimate_valid: bool,
    rms_envelope_valid:   bool,
    if_stability_valid:   bool,
    decay_estimate: Vec<f32>,
    rms_envelope:   Vec<f32>,
    if_stability:   Vec<f32>,
}

impl HistoryBuffer {
    pub fn new(num_channels: usize, capacity_frames: usize, num_bins: usize) -> Self {
        let frames: Vec<Vec<Vec<Complex<f32>>>> = (0..num_channels)
            .map(|_| (0..capacity_frames)
                .map(|_| vec![Complex::new(0.0, 0.0); num_bins])
                .collect())
            .collect();
        let summary = SummaryCache {
            decay_estimate: vec![0.0; num_bins],
            rms_envelope:   vec![0.0; num_bins],
            if_stability:   vec![0.0; num_bins],
            ..SummaryCache::default()
        };
        Self {
            frames,
            capacity_frames,
            num_bins,
            num_channels,
            write_pos: 0,
            frames_used: 0,
            summary: RefCell::new(summary),
        }
    }

    pub fn num_channels(&self)    -> usize { self.num_channels }
    pub fn capacity_frames(&self) -> usize { self.capacity_frames }
    pub fn num_bins(&self)        -> usize { self.num_bins }
    pub fn frames_used(&self)     -> usize { self.frames_used }

    /// Write one hop's complex spectrum for a channel into the current write slot.
    /// Allocation-free; copies `num_bins` complex floats. Caller MUST call
    /// `advance_after_all_channels_written()` once per hop after writing every
    /// channel, otherwise the next hop overwrites the same slot.
    pub fn write_hop(&mut self, channel: usize, spectrum: &[Complex<f32>]) {
        debug_assert!(channel < self.num_channels, "channel out of range");
        debug_assert_eq!(spectrum.len(), self.num_bins,
            "history write expected {} bins, got {}", self.num_bins, spectrum.len());
        let dst = &mut self.frames[channel][self.write_pos];
        let n = dst.len().min(spectrum.len());
        dst[..n].copy_from_slice(&spectrum[..n]);
    }

    /// Advance the shared write head one frame forward. Call once per hop after
    /// every channel has been written. Wraps at `capacity_frames`.
    pub fn advance_after_all_channels_written(&mut self) {
        self.write_pos = (self.write_pos + 1) % self.capacity_frames;
        if self.frames_used < self.capacity_frames {
            self.frames_used += 1;
        }
        // Any cached summary becomes stale once a new frame lands.
        self.invalidate_summary_cache();
    }

    /// Read the spectrum that was current `age_frames` ago. age=0 = most recent.
    /// Returns `None` if the requested age has not been written yet (cold start
    /// or after a reset).
    pub fn read_frame(&self, channel: usize, age_frames: usize) -> Option<&[Complex<f32>]> {
        if channel >= self.num_channels { return None; }
        if age_frames >= self.frames_used { return None; }
        let cap = self.capacity_frames;
        // write_pos points to the slot to be written NEXT. Most-recent written = write_pos - 1.
        let idx = (self.write_pos + cap - 1 - age_frames) % cap;
        Some(&self.frames[channel][idx])
    }

    /// Linear-interpolate between two adjacent frames at fractional age. Returns
    /// `false` (and leaves `out` unchanged) if `age + 1` exceeds `frames_used`.
    /// Note: this lerps complex bins directly — naive and intentionally cheap.
    /// The Past Stretch consumer adds phase-vocoder rotation on top via the
    /// shared `PhaseRotator` (Phase 5b.2).
    pub fn read_fractional(&self, channel: usize, age: f32, out: &mut [Complex<f32>]) -> bool {
        let age_floor = age.floor() as usize;
        let frac = age - age_floor as f32;
        let frame_a = match self.read_frame(channel, age_floor) {
            Some(f) => f,
            None => return false,
        };
        let frame_b = match self.read_frame(channel, age_floor + 1) {
            Some(f) => f,
            None => return false,
        };
        let n = out.len().min(frame_a.len()).min(frame_b.len());
        for k in 0..n {
            out[k] = frame_a[k] * (1.0 - frac) + frame_b[k] * frac;
        }
        true
    }

    /// Wipe the buffer and reset write head and frame count. Allocation-free.
    pub fn reset(&mut self) {
        for ch in &mut self.frames {
            for frame in ch {
                frame.fill(Complex::new(0.0, 0.0));
            }
        }
        self.write_pos = 0;
        self.frames_used = 0;
        self.invalidate_summary_cache();
    }

    /// Number of recent frames used to derive every summary stat.
    /// 32 frames at the default fft 2048 hop 512 / 48 kHz is ~340 ms.
    pub const ANALYSIS_WINDOW: usize = 32;

    /// Called by the pipeline at the top of every audio block. Marks the
    /// summary stats stale so they get re-derived on next read.
    pub fn clear_summary_cache(&self) {
        self.invalidate_summary_cache();
    }

    /// Per-bin frames-to-fall-20-dB estimate, derived from the linear-regression
    /// slope of `log10(magnitude)` over the most recent `ANALYSIS_WINDOW` frames.
    /// Higher = longer-ringing bin. Bins whose magnitude is too small or whose
    /// regression slope is non-negative get 0.0.
    ///
    /// Returned slice borrows the cached Vec; valid until the next
    /// `advance_after_all_channels_written()` or `clear_summary_cache()`.
    pub fn summary_decay_estimate(&self, channel: usize) -> std::cell::Ref<'_, [f32]> {
        self.maybe_recompute_decay(channel);
        std::cell::Ref::map(self.summary.borrow(), |s| s.decay_estimate.as_slice())
    }

    pub fn summary_rms_envelope(&self, channel: usize) -> std::cell::Ref<'_, [f32]> {
        self.maybe_recompute_rms(channel);
        std::cell::Ref::map(self.summary.borrow(), |s| s.rms_envelope.as_slice())
    }

    pub fn summary_if_stability(&self, channel: usize) -> std::cell::Ref<'_, [f32]> {
        self.maybe_recompute_if_stability(channel);
        std::cell::Ref::map(self.summary.borrow(), |s| s.if_stability.as_slice())
    }

    fn maybe_recompute_decay(&self, channel: usize) {
        {
            let s = self.summary.borrow();
            if s.decay_estimate_valid { return; }
        }
        let mut s = self.summary.borrow_mut();
        for v in &mut s.decay_estimate { *v = 0.0; }
        if channel >= self.num_channels { s.decay_estimate_valid = true; return; }
        let n = Self::ANALYSIS_WINDOW.min(self.frames_used);
        if n < 4 { s.decay_estimate_valid = true; return; }
        // Linear regression of log10(mag) vs frame index over the most recent n frames.
        // slope < 0 = decaying; we report -1 / slope as a decay-time proxy (larger = longer ring).
        let mean_x: f32 = (n as f32 - 1.0) * 0.5;
        let var_x: f32 = (0..n).map(|i| {
            let dx = i as f32 - mean_x;
            dx * dx
        }).sum::<f32>().max(1.0);
        for k in 0..self.num_bins {
            let mut mean_y = 0.0_f32;
            for i in 0..n {
                if let Some(frame) = self.read_frame(channel, i) {
                    let mag = frame[k].norm().max(1e-9);
                    mean_y += mag.log10();
                }
            }
            mean_y /= n as f32;
            let mut cov = 0.0_f32;
            for i in 0..n {
                if let Some(frame) = self.read_frame(channel, i) {
                    let mag = frame[k].norm().max(1e-9);
                    let dx = i as f32 - mean_x;
                    let dy = mag.log10() - mean_y;
                    cov += dx * dy;
                }
            }
            let slope = cov / var_x;
            // age 0 = most recent, age n-1 = oldest. Older frames are HIGHER index, so a
            // decaying signal (newer is louder) gives a NEGATIVE slope (log_mag decreases
            // with increasing age). Decay-time proxy: -1 / slope, clamped to [0, 1000].
            s.decay_estimate[k] = if slope < -1e-6 {
                (-1.0 / slope).clamp(0.0, 1000.0)
            } else {
                0.0
            };
        }
        s.decay_estimate_valid = true;
    }

    fn maybe_recompute_rms(&self, channel: usize) {
        {
            let s = self.summary.borrow();
            if s.rms_envelope_valid { return; }
        }
        let mut s = self.summary.borrow_mut();
        for v in &mut s.rms_envelope { *v = 0.0; }
        if channel >= self.num_channels { s.rms_envelope_valid = true; return; }
        let n = Self::ANALYSIS_WINDOW.min(self.frames_used);
        if n == 0 { s.rms_envelope_valid = true; return; }
        for k in 0..self.num_bins {
            let mut acc = 0.0_f32;
            for i in 0..n {
                if let Some(frame) = self.read_frame(channel, i) {
                    let mag = frame[k].norm();
                    acc += mag * mag;
                }
            }
            s.rms_envelope[k] = (acc / n as f32).sqrt();
        }
        s.rms_envelope_valid = true;
    }

    fn maybe_recompute_if_stability(&self, channel: usize) {
        {
            let s = self.summary.borrow();
            if s.if_stability_valid { return; }
        }
        let mut s = self.summary.borrow_mut();
        for v in &mut s.if_stability { *v = 0.0; }
        if channel >= self.num_channels { s.if_stability_valid = true; return; }
        let n = Self::ANALYSIS_WINDOW.min(self.frames_used);
        if n < 3 { s.if_stability_valid = true; return; }
        // For each bin, compute hop-to-hop phase-difference variance over n-1
        // adjacent frame pairs. Stable partials → low variance → near-1 score.
        // Unstable / noisy bins → high variance → near-0 score. We map variance v
        // to stability = 1 / (1 + v) so the result is bounded in (0, 1].
        for k in 0..self.num_bins {
            let mut diffs = [0.0_f32; Self::ANALYSIS_WINDOW];
            let mut count = 0_usize;
            for i in 0..(n - 1) {
                let a = self.read_frame(channel, i);
                let b = self.read_frame(channel, i + 1);
                if let (Some(a), Some(b)) = (a, b) {
                    let phase_a = a[k].arg();
                    let phase_b = b[k].arg();
                    let mut d = phase_a - phase_b;
                    while d > std::f32::consts::PI  { d -= std::f32::consts::TAU; }
                    while d < -std::f32::consts::PI { d += std::f32::consts::TAU; }
                    diffs[count] = d;
                    count += 1;
                }
            }
            if count < 2 { continue; }
            let mean: f32 = diffs[..count].iter().sum::<f32>() / count as f32;
            let var: f32 = diffs[..count].iter()
                .map(|&x| (x - mean) * (x - mean))
                .sum::<f32>() / count as f32;
            s.if_stability[k] = 1.0 / (1.0 + var);
        }
        s.if_stability_valid = true;
    }

    fn invalidate_summary_cache(&self) {
        let mut s = self.summary.borrow_mut();
        s.decay_estimate_valid = false;
        s.rms_envelope_valid = false;
        s.if_stability_valid = false;
    }
}
