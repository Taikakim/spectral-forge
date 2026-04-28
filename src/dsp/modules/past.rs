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
        &mut self, _ch: usize, _bins: &mut [Complex<f32>], _hist: &HistoryBuffer,
        _amount: &[f32], _time: &[f32], _threshold: &[f32], _spread: &[f32], _mix: &[f32],
        _ctx: &ModuleContext<'_>,
    ) {}

    fn apply_decay_sorter(
        &mut self, _ch: usize, _bins: &mut [Complex<f32>], _hist: &HistoryBuffer,
        _amount: &[f32], _threshold: &[f32], _mix: &[f32], _ctx: &ModuleContext<'_>,
    ) {}

    fn apply_convolution(
        &mut self, _ch: usize, _bins: &mut [Complex<f32>], _hist: &HistoryBuffer,
        _amount: &[f32], _time: &[f32], _threshold: &[f32], _mix: &[f32], _ctx: &ModuleContext<'_>,
    ) {}

    fn apply_reverse(
        &mut self, _ch: usize, _bins: &mut [Complex<f32>], _hist: &HistoryBuffer,
        _amount: &[f32], _time: &[f32], _threshold: &[f32], _mix: &[f32], _ctx: &ModuleContext<'_>,
    ) {}

    fn apply_stretch(
        &mut self, _ch: usize, _bins: &mut [Complex<f32>], _hist: &HistoryBuffer,
        _amount: &[f32], _time: &[f32], _spread: &[f32], _mix: &[f32], _ctx: &ModuleContext<'_>,
    ) {}
}
