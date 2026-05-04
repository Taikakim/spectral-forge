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
    CurveLayout, GainMode, ModuleContext, ModuleType, SpectralModule,
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

impl TryFrom<u8> for PastMode {
    type Error = ();
    fn try_from(b: u8) -> Result<Self, ()> {
        match b {
            0 => Ok(Self::Granular),
            1 => Ok(Self::DecaySorter),
            2 => Ok(Self::Convolution),
            3 => Ok(Self::Reverse),
            4 => Ok(Self::Stretch),
            _ => Err(()),
        }
    }
}

/// Per-mode `CurveLayout` for Past. See
/// docs/superpowers/specs/2026-05-04-past-module-ux-design.md §1 + §4.
///
/// Wired via `active_layout: Some(past::active_layout)` on the Past
/// `ModuleSpec` literal (Task 7).
pub fn active_layout(mode: u8) -> CurveLayout {
    match PastMode::try_from(mode).unwrap_or(PastMode::Granular) {
        PastMode::Granular => CurveLayout {
            active:          &[0, 1, 2, 3, 4],
            label_overrides: &[(1, "Age"), (3, "Smear")],
            help_for:        granular_help_for,
            mode_overview:   None,
        },
        PastMode::DecaySorter => CurveLayout {
            active:          &[0, 2, 4],
            label_overrides: &[],
            help_for:        decay_sorter_help_for,
            mode_overview:   None,
        },
        PastMode::Convolution => CurveLayout {
            active:          &[0, 1, 2, 4],
            label_overrides: &[(1, "Delay")],
            help_for:        convolution_help_for,
            mode_overview:   None,
        },
        PastMode::Reverse => CurveLayout {
            active:          &[0, 2, 4],
            label_overrides: &[],
            help_for:        reverse_help_for,
            mode_overview:   None,
        },
        PastMode::Stretch => CurveLayout {
            active:          &[0, 4],
            label_overrides: &[],
            help_for:        stretch_help_for,
            mode_overview:   None,
        },
    }
}

fn granular_help_for(curve_idx: u8) -> &'static str {
    match curve_idx {
        0 => "How much of the historical bin replaces the current bin. 0 = current only, 1 = historical only. Adds with upstream BinPhysics `crystallization`.",
        1 => "Per-bin lookback into history. 0 = now, 1 = oldest available frame.",
        2 => "Per-bin gate. Bins whose current magnitude falls below the threshold pass through unchanged.",
        3 => "Toggle (>0.5) per-bin 3-bin frequency smear of the historical read. Smooths bin-leakage across narrow partials.",
        4 => "Per-bin wet/dry.",
        _ => "",
    }
}

fn decay_sorter_help_for(curve_idx: u8) -> &'static str {
    match curve_idx {
        0 => "Per-bin output gain on the rearranged signal.",
        2 => "Per-bin floor — bins below this magnitude are excluded from sorting.",
        4 => "Per-bin wet/dry of sorted output vs. original.",
        _ => "",
    }
}

fn convolution_help_for(curve_idx: u8) -> &'static str {
    match curve_idx {
        0 => "Per-bin convolution strength. Multiplied by upstream BinPhysics `flux` if present (gates by recent change).",
        1 => "Per-bin delay into history. Low bins can sample old, high bins recent, or any other shape.",
        2 => "Per-bin gate on the current frame's magnitude.",
        4 => "Per-bin wet/dry.",
        _ => "",
    }
}

fn reverse_help_for(curve_idx: u8) -> &'static str {
    match curve_idx {
        0 => "Per-bin keep during the reverse read.",
        2 => "Per-bin gate.",
        4 => "Per-bin wet/dry.",
        _ => "",
    }
}

fn stretch_help_for(curve_idx: u8) -> &'static str {
    match curve_idx {
        0 => "Per-bin keep during the stretched read.",
        4 => "Per-bin wet/dry.",
        _ => "",
    }
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
/// Public alias of `MAX_SORT_BINS` so `Pipeline` can clamp `floor_bin` against it
/// without reaching into the private const. Keep in sync with `MAX_SORT_BINS`.
pub const MAX_SORT_BINS_PUB: usize = MAX_SORT_BINS;
const MAX_NUM_BINS_LOCAL: usize = crate::dsp::pipeline::MAX_NUM_BINS;

/// Stretch rate clamps. Keep the FloatParam range (build.rs) and this kernel
/// clamp aligned. The 0.05 floor prevents the read-pointer from freezing;
/// user-facing musical range is 0.25..4.0 (see spec §7.1).
const STRETCH_RATE_MIN: f32 = 0.05;
const STRETCH_RATE_MAX: f32 = 4.0;

/// Mode-specific scalar controls for Past. Replaces the curve-averaging hacks
/// in Reverse and Stretch with honest per-slot scalars; gates the soft-clip
/// post-pass; carries the DecaySorter floor.
///
/// Note: `Default` returns all-zeros for `..Default::default()` partial-update
/// syntax in tests. Production code (Pipeline wiring in Task 12) should use
/// `safe_default()` as the base and apply param overrides on top, never `Default`.
///
/// See docs/superpowers/specs/2026-05-04-past-module-ux-design.md §2 + §3.
#[derive(Clone, Copy, Debug, Default)]
pub struct PastScalars {
    /// DecaySorter destination-bin floor (DecaySorter only). Pipeline clamps to
    /// `[1, num_bins - MAX_SORT_BINS]` so DC stays untouched. Default 10 ≈ 230 Hz
    /// at fft 2048 / 48 kHz, matching the legacy hardcoded value.
    pub floor_bin:     usize,
    /// Reverse window length in **frames** (Pipeline converts seconds → frames each block).
    pub window_frames: u32,
    /// Stretch read rate. 1.0 = unity. Param range is 0.05..4.0 (the 0.05 floor
    /// prevents pointer freeze; user-facing musical range is 0.25..4.0 — see spec §7.1).
    pub rate:          f32,
    /// Stretch dither amount (0..1, normalised — Pipeline divides %-param by 100).
    pub dither:        f32,
    /// Module-wide soft-clip toggle (default ON).
    pub soft_clip:     bool,
}

impl PastScalars {
    /// Conservative default that's musically inert (rate=1.0 means stretch is no-op,
    /// window=1 frame is the smallest legal value, soft_clip ON).
    pub fn safe_default() -> Self {
        Self {
            floor_bin:     10,
            window_frames: 1,
            rate:          1.0,
            dither:        0.0,
            soft_clip:     true,
        }
    }
}

pub struct PastModule {
    mode: PastMode,
    sort_key: SortKey,
    rotator: PhaseRotator,

    /// Per-channel state. Index 0 = L / Mid / mono; 1 = R / Side.
    channels: [PastChannelState; 2],

    /// Current FFT size (used to derive bin-centre frequencies for Stretch).
    fft_size: usize,
    sample_rate: f32,

    /// Mode-specific scalar controls. Set per-block from Pipeline.
    scalars: PastScalars,

    #[cfg(any(test, feature = "probe"))]
    last_probe: crate::dsp::modules::ProbeSnapshot,
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
            scalars: PastScalars::safe_default(),
            #[cfg(any(test, feature = "probe"))]
            last_probe: crate::dsp::modules::ProbeSnapshot::default(),
        }
    }

    /// Per-block setter for mode-specific scalars (window_frames, rate, dither,
    /// floor_bin, soft_clip). Replaces the curve-averaging hacks documented in
    /// docs/superpowers/specs/2026-05-04-past-module-ux-design.md §1.4.
    pub fn set_scalars(&mut self, scalars: PastScalars) { self.scalars = scalars; }

    /// Accessor for current scalars (used in tests and by GUI for echo).
    pub fn scalars(&self) -> PastScalars { self.scalars }

    /// Per-block setter, called from FxMatrix::set_past_modes via the per-block
    /// snapshot in pipeline.rs. On a real change of mode we clear per-channel
    /// state so cross-mode artefacts (Reverse offset, Stretch read phase, sort
    /// scratch) don't leak into the new mode.
    pub fn set_mode(&mut self, mode: PastMode) {
        if self.mode != mode {
            self.mode = mode;
            self.clear_channel_state();
        }
    }

    /// Per-block setter, called from FxMatrix::set_past_sort_keys via the
    /// per-block snapshot in pipeline.rs.
    pub fn set_sort_key(&mut self, key: SortKey) { self.sort_key = key; }

    fn clear_channel_state(&mut self) {
        for s in &mut self.channels {
            s.reverse_read_offset = 0;
            s.stretch_read_phase = 0.0;
            s.stretch_rng = 0xA5A5_A5A5;
            for v in &mut s.sort_order { *v = 0; }
            for v in &mut s.sort_scratch { *v = Complex::new(0.0, 0.0); }
        }
    }
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

        #[cfg(any(test, feature = "probe"))]
        {
            // TIME is a normalized [0..1] fraction of the buffer's total temporal
            // depth; multiplying by `total_seconds` converts it to audible-units
            // for calibration assertions. AMOUNT is clamped to [0..1] before
            // scaling to 0..100% to mirror the convention used by other modules.
            let hop_size = (ctx.fft_size as f32 / 4.0) / ctx.sample_rate;
            let total_seconds = history.capacity_frames() as f32 * hop_size;
            let amount_at_bin0 = amount.first().copied().unwrap_or(0.0).clamp(0.0, 1.0);
            let mut probe = crate::dsp::modules::ProbeSnapshot {
                past_amount_pct:          Some(amount_at_bin0 * 100.0),
                past_active_mode_idx:     Some(self.mode as u8),
                past_history_frames_used: Some(history.frames_used() as u32),
                past_sort_key_idx:        Some(self.sort_key as u8),
                ..Default::default()
            };
            match self.mode {
                PastMode::Granular | PastMode::Convolution => {
                    // TIME curve still meaningful per-bin; report bin0 reading.
                    let time_at_bin0 = time.first().copied().unwrap_or(0.0);
                    probe.past_time_seconds = Some(time_at_bin0 * total_seconds);
                }
                PastMode::Reverse => {
                    probe.past_reverse_window_s = Some(self.scalars.window_frames as f32 * hop_size);
                }
                PastMode::Stretch => {
                    probe.past_stretch_rate = Some(self.scalars.rate);
                    probe.past_stretch_dither_pct = Some(self.scalars.dither * 100.0);
                }
                PastMode::DecaySorter => { /* no scalar probe */ }
            }
            self.last_probe = probe;
        }

        let ch = channel.min(1);
        match self.mode {
            PastMode::Granular   => self.apply_granular(ch, bins, history, amount, time, threshold, spread, mix, ctx),
            PastMode::DecaySorter=> self.apply_decay_sorter(ch, bins, history, amount, threshold, mix, ctx),
            PastMode::Convolution=> self.apply_convolution(ch, bins, history, amount, time, threshold, mix, ctx),
            PastMode::Reverse    => self.apply_reverse(ch, bins, history, amount, threshold, mix, ctx),
            PastMode::Stretch    => self.apply_stretch(ch, bins, history, amount, mix, ctx),
        }

        if self.scalars.soft_clip {
            apply_soft_clip(bins, ctx.num_bins);
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

    fn set_past_mode(&mut self, mode: crate::dsp::modules::past::PastMode) {
        self.set_mode(mode);
    }
    fn set_past_sort_key(&mut self, key: crate::dsp::modules::past::SortKey) {
        self.set_sort_key(key);
    }
    fn set_past_scalars(&mut self, scalars: crate::dsp::modules::past::PastScalars) {
        self.scalars = scalars;
    }

    #[cfg(any(test, feature = "probe"))]
    fn test_past_scalars(&self) -> Option<crate::dsp::modules::past::PastScalars> {
        Some(self.scalars)
    }

    #[cfg(any(test, feature = "probe"))]
    fn last_probe(&self) -> crate::dsp::modules::ProbeSnapshot { self.last_probe }
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
        // Pipeline converts Hz → bin index (per-slot Floor param). Default ≈ 230 Hz
        // at fft 2048 / 48 kHz → bin 10. Clamped to [1, num_bins - MAX_SORT_BINS] in Pipeline.
        let low_k = self.scalars.floor_bin.min(n.saturating_sub(1));
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
        amount: &[f32], threshold: &[f32], mix: &[f32],
        ctx: &ModuleContext<'_>,
    ) {
        let n = bins.len().min(ctx.num_bins);
        let window = self.scalars.window_frames.max(1);

        let st = &mut self.channels[ch];
        let age = (st.reverse_read_offset % window) as usize;

        // Read first, advance only on success — keeps the offset frozen while the
        // history ring fills (cold-start guard).
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
        amount: &[f32], mix: &[f32],
        ctx: &ModuleContext<'_>,
    ) {
        let n = bins.len().min(ctx.num_bins);
        let rate = self.scalars.rate.clamp(STRETCH_RATE_MIN, STRETCH_RATE_MAX);
        let dither_amt = self.scalars.dither.clamp(0.0, 1.0);

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
        #[allow(clippy::needless_range_loop)]
        for k in 0..n {
            let bin_amount = amount.get(k).copied().unwrap_or(0.0).clamp(0.0, 1.0);
            if bin_amount < 1e-6 { continue; }

            let mut sample = self.channels[ch].sort_scratch[k];

            // Phase rotation: 2π · if_offset · (rate - 1) cycles.
            let if_off = if_offset.get(k).copied().unwrap_or(0.0);
            let rot = if_off * (rate - 1.0);
            sample = self.rotator.rotate(sample, rot, 1.0);

            // Always tick the RNG (xorshift32) per bin so the sequence stays
            // consistent across dither enable/disable cycles. Apply the
            // resulting phase rotation only when `dither_amt > 0`.
            let s = self.channels[ch].stretch_rng;
            let mut x = s ^ (k as u32).wrapping_mul(0x9E37_79B9);
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            self.channels[ch].stretch_rng = x;
            if dither_amt > 0.0 {
                let dither_phase = ((x as f32 / u32::MAX as f32) - 0.5) * dither_amt * 0.05;
                sample = self.rotator.rotate(sample, dither_phase, 1.0);
            }
            let value = sample * bin_amount;
            let m_val = mix.get(k).copied().unwrap_or(1.0).clamp(0.0, 1.0);
            bins[k] = bins[k] * (1.0 - m_val) + value * m_val;
        }
    }
}

/// Per-bin radial soft-clip toward magnitude `K = 4.0` (≈ +12 dBFS).
/// `bins[k] *= K / (K + |bins[k]|)` shrinks magnitudes asymptotically toward
/// `K` while leaving small magnitudes nearly unchanged.
///
/// Module-wide safety net for Past, primarily protecting Convolution's
/// multiplicative output from exploding when fed loud audio × loud history.
/// See spec §3 + §7.1.
pub fn apply_soft_clip(bins: &mut [Complex<f32>], num_bins: usize) {
    const K: f32 = 4.0;
    for k in 0..num_bins.min(bins.len()) {
        let mag = bins[k].norm();
        if mag > 1e-9 {
            let scale = K / (K + mag);
            bins[k] *= scale;
        }
    }
}
