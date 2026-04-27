use num_complex::Complex;
use serde::{Deserialize, Serialize};
use crate::params::{FxChannelTarget, StereoLink};
use super::{ModuleContext, ModuleType, SpectralModule};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum RhythmMode {
    #[default]
    Euclidean,
    Arpeggiator,
    PhaseReset,
}

impl RhythmMode {
    pub fn label(self) -> &'static str {
        match self {
            RhythmMode::Euclidean   => "Euclidean",
            RhythmMode::Arpeggiator => "Arpeggiator",
            RhythmMode::PhaseReset  => "Phase Reset",
        }
    }
}

/// Arpeggiator step grid: 8 voices × 8 steps. Each voice's steps are packed in a `u8`
/// (bit 0 = step 0, bit 7 = step 7). A '1' bit means the voice plays at that step.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct ArpGrid {
    pub steps: [u8; 8],
}

impl ArpGrid {
    pub fn voice_active_at(&self, voice: usize, step: usize) -> bool {
        if voice >= 8 || step >= 8 { return false; }
        (self.steps[voice] >> step) & 1 != 0
    }
    pub fn toggle(&mut self, voice: usize, step: usize) {
        if voice < 8 && step < 8 {
            self.steps[voice] ^= 1 << step;
        }
    }
}

pub struct RhythmModule {
    mode:        RhythmMode,
    fft_size:    usize,
    sample_rate: f32,
    /// Snapshot of last-processed step index — used to detect step crossings.
    last_step_idx: i32,
    /// The arpeggiator grid (set by the GUI via `set_arp_grid`).
    arp_grid:    ArpGrid,
    /// Per-voice peak bin (assigned at step crossings, held for the step duration).
    #[allow(dead_code)] // used by Arpeggiator arm in Task 5
    arp_voice_peak_bin: [u32; 8],
    /// Per-voice envelope state (0..1) for amp ramp-up at each gate-on.
    arp_voice_env: [f32; 8],
    #[cfg(any(test, feature = "probe"))]
    last_probe:  crate::dsp::modules::ProbeSnapshot,
}

impl RhythmModule {
    pub fn new() -> Self {
        Self {
            mode:        RhythmMode::default(),
            fft_size:    2048,
            sample_rate: 44100.0,
            last_step_idx: -1,
            arp_grid:    ArpGrid::default(),
            arp_voice_peak_bin: [0; 8],
            arp_voice_env: [0.0; 8],
            #[cfg(any(test, feature = "probe"))]
            last_probe: Default::default(),
        }
    }

    pub fn set_mode(&mut self, mode: RhythmMode) { self.mode = mode; }
    pub fn mode(&self) -> RhythmMode { self.mode }
    pub fn set_arp_grid(&mut self, g: ArpGrid) { self.arp_grid = g; }
}

impl Default for RhythmModule {
    fn default() -> Self { Self::new() }
}

impl SpectralModule for RhythmModule {
    fn reset(&mut self, sample_rate: f32, fft_size: usize) {
        self.sample_rate   = sample_rate;
        self.fft_size      = fft_size;
        self.last_step_idx = -1;
        self.arp_voice_env = [0.0; 8];
    }

    fn process(
        &mut self,
        _channel: usize,
        _stereo_link: StereoLink,
        _target: FxChannelTarget,
        bins: &mut [Complex<f32>],
        _sidechain: Option<&[f32]>,
        _curves: &[&[f32]],
        suppression_out: &mut [f32],
        _ctx: &ModuleContext<'_>,
    ) {
        // Stub. Tasks 4-6 implement Euclidean, Arpeggiator, Phase Reset.
        suppression_out.fill(0.0);
        let _ = bins;
    }

    fn module_type(&self) -> ModuleType { ModuleType::Rhythm }
    fn num_curves(&self) -> usize { 5 }

    #[cfg(any(test, feature = "probe"))]
    fn last_probe(&self) -> crate::dsp::modules::ProbeSnapshot { self.last_probe }
}
