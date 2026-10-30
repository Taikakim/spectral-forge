//! Live per-slot monitor: input and output bin levels plus change statistics
//! for one slot (the slot being edited), accumulated over an audio block and
//! published to the GUI through a triple buffer.
//!
//! `FxMatrix::process_hop` calls `capture_pre` just before the monitored
//! slot's `process()` and `observe_post` right after it, for every channel and
//! hop. `Pipeline::process` publishes and clears once per block. Allocation-free
//! after construction.

use num_complex::Complex;

use crate::dsp::pipeline::MAX_NUM_BINS;

/// Published layout: `[in_peak; MAX_NUM_BINS] ++ [out_peak; MAX_NUM_BINS] ++ stats`.
pub const MONITOR_STATS: usize = 8;
pub const MONITOR_LEN: usize = 2 * MAX_NUM_BINS + MONITOR_STATS;

/// Stat offsets within the trailing `MONITOR_STATS` block.
pub const STAT_GAIN_DB: usize = 0;
pub const STAT_DELTA_DB: usize = 1;
pub const STAT_SPREAD_PCT: usize = 2;
pub const STAT_SPILL_DB: usize = 3;
pub const STAT_MAX_BIN_GAIN_DB: usize = 4;
pub const STAT_HOPS: usize = 5;
pub const STAT_SLOT: usize = 6;
pub const STAT_NUM_BINS: usize = 7;

/// Bins below this, relative to the frame's input peak, are "silent" (−60 dB).
const FLOOR_REL: f32 = 1e-3;
/// Only output bins within this of the input peak count for max bin gain (−20 dB).
const LOUD_REL: f32 = 0.1;
/// Complex change counted as "changed" (~1 dB equivalent).
const CHANGED_REL: f32 = 0.12;

pub struct SlotMonitor {
    pre:      Vec<Complex<f32>>,
    in_peak:  Vec<f32>,
    out_peak: Vec<f32>,
    e_in:     f64,
    e_out:    f64,
    e_diff:   f64,
    spill:    f64,
    changed:  u64,
    active:   u64,
    max_bin_gain_db: f32,
    hops:     u32,
    num_bins: usize,
}

impl Default for SlotMonitor {
    fn default() -> Self { Self::new() }
}

impl SlotMonitor {
    pub fn new() -> Self {
        Self {
            pre:      vec![Complex::new(0.0, 0.0); MAX_NUM_BINS],
            in_peak:  vec![0.0; MAX_NUM_BINS],
            out_peak: vec![0.0; MAX_NUM_BINS],
            e_in: 0.0, e_out: 0.0, e_diff: 0.0, spill: 0.0,
            changed: 0, active: 0,
            max_bin_gain_db: -200.0,
            hops: 0,
            num_bins: 0,
        }
    }

    /// Snapshot the slot's input. Call immediately before the module's `process()`.
    #[inline]
    pub fn capture_pre(&mut self, bins: &[Complex<f32>]) {
        let n = bins.len().min(MAX_NUM_BINS);
        self.pre[..n].copy_from_slice(&bins[..n]);
    }

    /// Compare the slot's output with the captured input and accumulate.
    pub fn observe_post(&mut self, out: &[Complex<f32>]) {
        let n = out.len().min(MAX_NUM_BINS);
        self.num_bins = n;
        let in_max = self.pre[..n].iter().fold(0.0f32, |m, c| m.max(c.norm()));
        let floor = (in_max * FLOOR_REL).max(1e-9);
        let loud = in_max * LOUD_REL;
        for k in 0..n {
            let a = self.pre[k];
            let b = out[k];
            let (ma, mb) = (a.norm(), b.norm());
            if !mb.is_finite() { continue; }
            let md = (b - a).norm();
            self.in_peak[k] = self.in_peak[k].max(ma);
            self.out_peak[k] = self.out_peak[k].max(mb);
            self.e_in += (ma as f64) * (ma as f64);
            self.e_out += (mb as f64) * (mb as f64);
            self.e_diff += (md as f64) * (md as f64);
            if ma > floor {
                self.active += 1;
                if md > CHANGED_REL * ma { self.changed += 1; }
                if mb > loud {
                    self.max_bin_gain_db = self.max_bin_gain_db.max(20.0 * (mb / ma).log10());
                }
            } else if mb > floor {
                self.spill += (mb as f64) * (mb as f64);
            }
        }
        self.hops += 1;
    }

    pub fn hops(&self) -> u32 { self.hops }

    /// Write the published layout into `dst` (length `MONITOR_LEN`).
    pub fn publish_into(&self, slot: usize, dst: &mut [f32]) {
        if dst.len() < MONITOR_LEN { return; }
        dst[..MAX_NUM_BINS].copy_from_slice(&self.in_peak);
        dst[MAX_NUM_BINS..2 * MAX_NUM_BINS].copy_from_slice(&self.out_peak);
        let db10 = |x: f64| (10.0 * x.max(1e-30).log10()) as f32;
        let e_in = self.e_in.max(1e-30);
        let stats = &mut dst[2 * MAX_NUM_BINS..];
        stats[STAT_GAIN_DB] = db10(self.e_out / e_in);
        stats[STAT_DELTA_DB] = db10(self.e_diff / e_in);
        stats[STAT_SPREAD_PCT] = if self.active > 0 {
            self.changed as f32 / self.active as f32 * 100.0
        } else { 0.0 };
        stats[STAT_SPILL_DB] = db10(self.spill / e_in);
        stats[STAT_MAX_BIN_GAIN_DB] = self.max_bin_gain_db;
        stats[STAT_HOPS] = self.hops as f32;
        stats[STAT_SLOT] = slot as f32;
        stats[STAT_NUM_BINS] = self.num_bins as f32;
    }

    /// Reset the accumulators for the next block.
    pub fn clear(&mut self) {
        self.in_peak.fill(0.0);
        self.out_peak.fill(0.0);
        self.e_in = 0.0;
        self.e_out = 0.0;
        self.e_diff = 0.0;
        self.spill = 0.0;
        self.changed = 0;
        self.active = 0;
        self.max_bin_gain_db = -200.0;
        self.hops = 0;
    }
}
