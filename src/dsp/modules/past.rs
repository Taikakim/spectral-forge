//! Past module — read-only consumer of `HistoryBuffer` (Phase 5b.1).
//!
//! Five sub-modes:
//! 1. Granular Window — selectively replay a band from history.
//! 2. Decay Sorter — reorder bins by decay/stability/area summary stat.
//! 3. Spectral Convolution — point-wise multiply with a delayed self.
//! 4. Reverse — read history at a backward read position.
//! 5. Stretch — read history at a fractional rate, with phase rotation.
//!
//! All modes accept curves [AMOUNT, TIME, THRESHOLD, SPREAD, MIX] and
//! respect MIX as a per-bin wet/dry blend. State is per-channel.

use num_complex::Complex;
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;

use crate::dsp::history_buffer::HistoryBuffer;
use crate::dsp::modules::{
    GainMode, ModuleContext, ModuleType, SpectralModule,
};
use crate::dsp::phase::PhaseRotator;
use crate::params::{FxChannelTarget, StereoLink};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum PastMode {
    #[default]
    Granular,
    DecaySorter,
    Convolution,
    Reverse,
    Stretch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SortKey {
    #[default]
    Decay,
    Stability,
    Area,
}

/// Decay sorter operates on the lowest 256 bins only. Sorting all
/// MAX_NUM_BINS (8193 at fft=16384) is O(n log n) per block per channel
/// and not worth the cost — perceptually significant decay-time
/// material lives in the lower bands.
const MAX_SORT_BINS: usize = 256;
const MAX_NUM_BINS_LOCAL: usize = crate::dsp::pipeline::MAX_NUM_BINS;

pub struct PastModule {
    mode: PastMode,
    sort_key: SortKey,
    rotator: PhaseRotator,

    /// Per-channel state. Index 0 = L / Mid / mono; 1 = R / Side.
    channels: [PastChannelState; 2],

    /// Current FFT size (used to derive bin-centre frequencies for Stretch).
    fft_size: usize,
    sample_rate: f32,
}

struct PastChannelState {
    /// Reverse mode: cumulative read offset (frames back, increments each hop).
    reverse_read_offset: u32,

    /// Stretch mode: fractional read position (frames back). Updated by
    /// `read_phase += hop_per_hop_rate`.
    stretch_read_phase: f64,

    /// Decay sorter: cached sort order — indices of MAX_SORT_BINS bins
    /// ordered by ascending sort key.
    sort_order: Vec<u32>,

    /// Decay sorter: cached scratch for assigning output bin slots.
    sort_scratch: Vec<Complex<f32>>,

    /// Stretch xorshift32 seed for hash-driven phase dither (avoids comb-filter
    /// "tape head" artifacts at fractional read rates).
    stretch_rng: u32,
}

impl PastChannelState {
    fn new() -> Self {
        Self {
            reverse_read_offset: 0,
            stretch_read_phase: 0.0,
            sort_order: vec![0; MAX_SORT_BINS],
            sort_scratch: vec![Complex::new(0.0, 0.0); MAX_NUM_BINS_LOCAL],
            stretch_rng: 0xA5A5_A5A5,
        }
    }
}

impl PastModule {
    pub fn new(sample_rate: f32, fft_size: usize) -> Self {
        Self {
            mode: PastMode::default(),
            sort_key: SortKey::default(),
            rotator: PhaseRotator::new(),
            channels: [PastChannelState::new(), PastChannelState::new()],
            fft_size,
            sample_rate,
        }
    }

    /// Per-block setter, called from FxMatrix::set_past_modes via lib.rs snapshot.
    pub fn set_mode(&mut self, mode: PastMode) { self.mode = mode; }

    /// Per-block setter, called from FxMatrix::set_past_sort_keys via lib.rs snapshot.
    pub fn set_sort_key(&mut self, key: SortKey) { self.sort_key = key; }
}

impl SpectralModule for PastModule {
    fn process(
        &mut self,
        channel: usize,
        _stereo_link: StereoLink,
        _target: FxChannelTarget,
        bins: &mut [Complex<f32>],
        _sidechain: Option<&[f32]>,
        curves: &[&[f32]],
        suppression_out: &mut [f32],
        _physics: Option<&mut crate::dsp::bin_physics::BinPhysics>,
        ctx: &ModuleContext<'_>,
    ) {
        debug_assert_eq!(curves.len(), 5, "Past expects 5 curves");
        // TODO(5b2.5+): once kernels write to the full read range, decide whether
        // FxChannelTarget::Mid/Side gating is needed here. Current stubs are no-op.
        // Conservative defaults if curves are missing (should never happen).
        let amount    = curves.get(0).map(|c| &c[..]).unwrap_or(&[]);
        let time      = curves.get(1).map(|c| &c[..]).unwrap_or(&[]);
        let threshold = curves.get(2).map(|c| &c[..]).unwrap_or(&[]);
        let spread    = curves.get(3).map(|c| &c[..]).unwrap_or(&[]);
        let mix       = curves.get(4).map(|c| &c[..]).unwrap_or(&[]);

        // Suppression: Past is mostly additive/replacement — clear to 0.0.
        for v in suppression_out.iter_mut() { *v = 0.0; }

        let history = match ctx.history { Some(h) => h, None => return };

        let ch = channel.min(1);
        match self.mode {
            PastMode::Granular   => self.apply_granular(ch, bins, history, amount, time, threshold, spread, mix, ctx),
            PastMode::DecaySorter=> self.apply_decay_sorter(ch, bins, history, amount, threshold, mix, ctx),
            PastMode::Convolution=> self.apply_convolution(ch, bins, history, amount, time, threshold, mix, ctx),
            PastMode::Reverse    => self.apply_reverse(ch, bins, history, amount, time, threshold, mix, ctx),
            PastMode::Stretch    => self.apply_stretch(ch, bins, history, amount, time, spread, mix, ctx),
        }
    }

    fn reset(&mut self, sample_rate: f32, fft_size: usize) {
        self.sample_rate = sample_rate;
        self.fft_size = fft_size;
        for s in &mut self.channels {
            s.reverse_read_offset = 0;
            s.stretch_read_phase = 0.0;
            s.stretch_rng = 0xA5A5_A5A5;
            for v in &mut s.sort_order { *v = 0; }
            for v in &mut s.sort_scratch { *v = Complex::new(0.0, 0.0); }
        }
    }

    fn module_type(&self) -> ModuleType { ModuleType::Past }
    fn num_curves(&self) -> usize { 5 }
    fn tail_length(&self) -> u32 { 0 }
    fn set_gain_mode(&mut self, _: GainMode) {}
}

// ── Mode kernels (stubs — Tasks 5–9 fill them in) ────────────────────────────

impl PastModule {
    fn apply_granular(
        &mut self, ch: usize, bins: &mut [Complex<f32>], hist: &HistoryBuffer,
        amount: &[f32], time: &[f32], threshold: &[f32], spread: &[f32], mix: &[f32],
        ctx: &ModuleContext<'_>,
    ) {
        let n = bins.len().min(ctx.num_bins);
        // TIME maps [0..1] to [0..capacity_frames] historic frames.
        let max_age = hist.capacity_frames() as f32;
        // BinPhysics crystallization (if present) biases AMOUNT toward 1.0 per-bin.
        let cryst = ctx.bin_physics.map(|p| &p.crystallization[..]);
        for k in 0..n {
            let bin_amount = amount.get(k).copied().unwrap_or(0.0);
            let cryst_bias = cryst.and_then(|c| c.get(k).copied()).unwrap_or(0.0);
            let effective_amount = (bin_amount + cryst_bias).clamp(0.0, 1.0);
            let mag_sq = bins[k].norm_sqr();
            let thr = threshold.get(k).copied().unwrap_or(0.0);
            if mag_sq < thr * thr { continue; }
            let age = (time.get(k).copied().unwrap_or(0.0).clamp(0.0, 1.0) * max_age).round() as usize;
            let frame = match hist.read_frame(ch, age) { Some(f) => f, None => continue };
            if k >= frame.len() { continue; }
            let val = if spread.get(k).copied().unwrap_or(0.0) > 0.5 && k > 0 && k + 1 < frame.len() {
                (frame[k - 1] + frame[k] + frame[k + 1]) * (1.0 / 3.0)
            } else {
                frame[k]
            };
            let replacement = val * effective_amount;
            let m_val = mix.get(k).copied().unwrap_or(1.0).clamp(0.0, 1.0);
            bins[k] = bins[k] * (1.0 - m_val) + replacement * m_val;
        }
    }

    fn apply_decay_sorter(
        &mut self, ch: usize, bins: &mut [Complex<f32>], hist: &HistoryBuffer,
        amount: &[f32], threshold: &[f32], mix: &[f32], _ctx: &ModuleContext<'_>,
    ) {
        let n = bins.len();
        // Bin 10 ≈ 230 Hz at fft 2048 / 48 kHz — first musically useful bin.
        let low_k = 10usize.min(n.saturating_sub(1));
        // Pick top MAX_SORT_BINS bins by current magnitude above THRESHOLD.
        // TODO(perf): linear min_by is O(n × MAX_SORT_BINS) — acceptable up to
        // fft=4096 (~330 K cmps/block); revisit before enabling fft=16384.
        let mut candidates: SmallVec<[(u32, f32); MAX_SORT_BINS]> = SmallVec::new();
        for k in 0..n {
            let mag_sq = bins[k].norm_sqr();
            let thr = threshold.get(k).copied().unwrap_or(0.0);
            if mag_sq < thr * thr { continue; }
            if candidates.len() < MAX_SORT_BINS {
                candidates.push((k as u32, mag_sq));
            } else if let Some((min_idx, _)) = candidates.iter().enumerate()
                .min_by(|a, b| a.1.1.partial_cmp(&b.1.1).unwrap_or(std::cmp::Ordering::Equal))
            {
                if candidates[min_idx].1 < mag_sq {
                    candidates[min_idx] = (k as u32, mag_sq);
                }
            }
        }
        // Pull the right summary stat per sort key.
        let key_values_borrow: std::cell::Ref<'_, [f32]> = match self.sort_key {
            SortKey::Decay     => hist.summary_decay_estimate(ch),
            SortKey::Stability => hist.summary_if_stability(ch),
            SortKey::Area      => hist.summary_rms_envelope(ch),
        };
        // Sort descending — highest key value first → lowest output slot.
        // Tie-break: louder bin (higher mag_sq) comes first.
        {
            let key_values: &[f32] = &*key_values_borrow;
            candidates.sort_by(|a, b| {
                let ka = key_values.get(a.0 as usize).copied().unwrap_or(0.0);
                let kb = key_values.get(b.0 as usize).copied().unwrap_or(0.0);
                kb.partial_cmp(&ka)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then(b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal))
            });
        }
        drop(key_values_borrow);
        // Snapshot bins before destructive write.
        let scratch = &mut self.channels[ch].sort_scratch;
        debug_assert!(n <= scratch.len(), "bins.len() exceeds sort_scratch capacity");
        scratch[..n].copy_from_slice(&bins[..n]);
        let max_dest = (low_k + candidates.len()).min(n);
        for k in low_k..max_dest { bins[k] = Complex::new(0.0, 0.0); }
        // Write each ranked complex value into low_k + rank.
        for (rank, (src_k, _)) in candidates.iter().enumerate() {
            let dest = low_k + rank;
            if dest >= n { break; }
            let bin_amount = amount.get(*src_k as usize).copied().unwrap_or(1.0);
            let m_val = mix.get(dest).copied().unwrap_or(1.0).clamp(0.0, 1.0);
            let value = scratch[*src_k as usize] * bin_amount;
            bins[dest] = scratch[dest] * (1.0 - m_val) + value * m_val;
        }
        // Source bins outside the destination range get cleared (their content moved).
        for (src_k, _) in candidates.iter() {
            let kk = *src_k as usize;
            if kk < low_k || kk >= max_dest {
                let m_val = mix.get(kk).copied().unwrap_or(1.0).clamp(0.0, 1.0);
                bins[kk] = scratch[kk] * (1.0 - m_val);
            }
        }
    }

    fn apply_convolution(
        &mut self, ch: usize, bins: &mut [Complex<f32>], hist: &HistoryBuffer,
        amount: &[f32], time: &[f32], threshold: &[f32], mix: &[f32], ctx: &ModuleContext<'_>,
    ) {
        let n = bins.len().min(ctx.num_bins);
        let max_age = hist.capacity_frames() as f32;
        let flux = ctx.bin_physics.map(|p| &p.flux[..]);
        for k in 0..n {
            let mag_sq = bins[k].norm_sqr();
            let thr = threshold.get(k).copied().unwrap_or(0.0);
            if mag_sq < thr * thr { continue; }
            let flux_gate = flux.and_then(|f| f.get(k).copied()).unwrap_or(1.0).clamp(0.0, 1.0);
            let bin_amount = amount.get(k).copied().unwrap_or(0.0) * flux_gate;
            if bin_amount < 1e-6 { continue; }
            let age = (time.get(k).copied().unwrap_or(0.0).clamp(0.0, 1.0) * max_age).round() as usize;
            let frame = match hist.read_frame(ch, age) { Some(f) => f, None => continue };
            if k >= frame.len() { continue; }
            let conv = bins[k] * frame[k] * bin_amount;
            let m_val = mix.get(k).copied().unwrap_or(1.0).clamp(0.0, 1.0);
            bins[k] = bins[k] * (1.0 - m_val) + conv * m_val;
        }
    }

    fn apply_reverse(
        &mut self, ch: usize, bins: &mut [Complex<f32>], hist: &HistoryBuffer,
        amount: &[f32], time: &[f32], threshold: &[f32], mix: &[f32], ctx: &ModuleContext<'_>,
    ) {
        let n = bins.len().min(ctx.num_bins);
        let max_age = hist.capacity_frames() as f32;
        // Median TIME picks the window length — TIME is per-bin but the read
        // pointer is per-channel. Picking the average avoids per-bin pointer drift.
        let window = {
            let t_avg = if n == 0 { 0.0 } else {
                time.iter().take(n).copied().sum::<f32>() / n as f32
            };
            ((t_avg.clamp(0.0, 1.0) * max_age).round() as u32).max(1)
        };
        let st = &mut self.channels[ch];
        let age = (st.reverse_read_offset % window) as usize;

        // Read first; only advance the offset if the read succeeded. Otherwise the
        // offset would skip positions during cold-start while the ring fills up,
        // producing a discontinuity once audio finally starts reading.
        let frame = match hist.read_frame(ch, age) { Some(f) => f, None => return };
        st.reverse_read_offset = (st.reverse_read_offset + 1) % window;
        for k in 0..n {
            let mag_sq = bins[k].norm_sqr();
            let thr = threshold.get(k).copied().unwrap_or(0.0);
            if mag_sq < thr * thr { continue; }
            if k >= frame.len() { continue; }
            let bin_amount = amount.get(k).copied().unwrap_or(0.0).clamp(0.0, 1.0);
            let value = frame[k] * bin_amount;
            let m_val = mix.get(k).copied().unwrap_or(1.0).clamp(0.0, 1.0);
            bins[k] = bins[k] * (1.0 - m_val) + value * m_val;
        }
    }

    fn apply_stretch(
        &mut self, ch: usize, bins: &mut [Complex<f32>], hist: &HistoryBuffer,
        amount: &[f32], time: &[f32], spread: &[f32], mix: &[f32], ctx: &ModuleContext<'_>,
    ) {
        let n = bins.len().min(ctx.num_bins);
        // TIME maps [0..1] log-scale to [0.25×..4×] read rate.
        // 0.0 → 0.25, 0.5 → 1.0, 1.0 → 4.0
        let t_avg = if n == 0 { 0.0 } else {
            time.iter().take(n).copied().sum::<f32>() / n as f32
        };
        let rate = 4.0_f32.powf(2.0 * t_avg.clamp(0.0, 1.0) - 1.0);

        // −2 because read_fractional reads frame[age_floor] AND frame[age_floor+1];
        // both must be in range (< frames_used). Using saturating_sub(1) would let
        // age_floor reach cap-1 and attempt frame cap, which exceeds frames_used.
        let max_age = hist.capacity_frames().saturating_sub(2) as f32;

        let read_age = self.channels[ch].stretch_read_phase as f32;
        // Advance read phase for next hop; wrap at capacity.
        self.channels[ch].stretch_read_phase += rate as f64;
        if self.channels[ch].stretch_read_phase > max_age as f64 {
            self.channels[ch].stretch_read_phase = 0.0;
        }

        // Read the two bracketing frames into the per-channel workspace.
        // Scope the &mut borrow to just this block so the per-bin loop below
        // can reborrow self.channels[ch] independently.
        let ok = {
            let scratch = &mut self.channels[ch].sort_scratch;
            scratch[..n].fill(Complex::new(0.0, 0.0));
            hist.read_fractional(ch, read_age, &mut scratch[..n])
        };
        if !ok { return; }

        let if_offset = ctx.if_offset.unwrap_or(&[]);
        // Need k for parallel indexing into bins / sort_scratch / if_offset; iterator
        // refactor would not improve readability here.
        // TODO(v2): RNG state currently only advances on bins where SPREAD>0; if
        // SPREAD is zero everywhere the per-channel state never updates. Move the
        // single-step advance out of the per-bin branch when wiring v2 (Laroche-
        // Dolson). Dither quality is fine for v1.
        #[allow(clippy::needless_range_loop)]
        for k in 0..n {
            let bin_amount = amount.get(k).copied().unwrap_or(0.0).clamp(0.0, 1.0);
            if bin_amount < 1e-6 { continue; }

            let mut sample = self.channels[ch].sort_scratch[k];

            // Phase rotation: 2π · if_offset · (rate - 1) cycles.
            let if_off = if_offset.get(k).copied().unwrap_or(0.0);
            let rot = if_off * (rate - 1.0);
            sample = self.rotator.rotate(sample, rot, 1.0);

            // Per-bin hash dither based on SPREAD curve (xorshift32).
            let spr = spread.get(k).copied().unwrap_or(0.0).clamp(0.0, 1.0);
            if spr > 1e-6 {
                let s = self.channels[ch].stretch_rng;
                let mut x = s ^ (k as u32).wrapping_mul(0x9E37_79B9);
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                self.channels[ch].stretch_rng = x;
                let dither_phase = ((x as f32 / u32::MAX as f32) - 0.5) * spr * 0.05;
                sample = self.rotator.rotate(sample, dither_phase, 1.0);
            }
            let value = sample * bin_amount;
            let m_val = mix.get(k).copied().unwrap_or(1.0).clamp(0.0, 1.0);
            bins[k] = bins[k] * (1.0 - m_val) + value * m_val;
        }
    }
}
