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
                for c in frame { *c = Complex::new(0.0, 0.0); }
            }
        }
        self.write_pos = 0;
        self.frames_used = 0;
        self.invalidate_summary_cache();
    }

    /// Called by the pipeline at the top of every audio block. Marks the
    /// summary stats stale so they get re-derived on next read.
    pub fn clear_summary_cache(&self) {
        self.invalidate_summary_cache();
    }

    fn invalidate_summary_cache(&self) {
        let mut s = self.summary.borrow_mut();
        s.decay_estimate_valid = false;
        s.rms_envelope_valid = false;
        s.if_stability_valid = false;
    }
}
