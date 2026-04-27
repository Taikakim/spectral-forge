use num_complex::Complex;
use serde::{Deserialize, Serialize};
use crate::params::{FxChannelTarget, StereoLink};
use super::{ModuleContext, ModuleType, SpectralModule};

pub const MAX_PEAKS:        usize = 32;
pub const MAX_DRIFT_SITES:  usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum PunchMode {
    #[default]
    Direct,
    Inverse,
}

impl PunchMode {
    pub fn label(self) -> &'static str {
        match self {
            PunchMode::Direct  => "Direct",
            PunchMode::Inverse => "Inverse",
        }
    }
}

pub struct PunchModule {
    mode:        PunchMode,
    fft_size:    usize,
    sample_rate: f32,
    /// Smoothed carve depth applied this hop (0 = no carve, 1 = full mute), per channel × per bin.
    /// Allocated in `reset()`. Tasks 2c.4-2c.6 read/write this.
    current_carve_depth: [Vec<f32>; 2],
    /// Sub-bin pitch-drift accumulator (in fractional bins), per channel × per bin.
    /// Allocated in `reset()`. Task 2c.5 populates.
    drift_accum:         [Vec<f32>; 2],
    /// Sidechain peak indices detected this hop (Task 2c.3 fills).
    #[allow(dead_code)] // populated in Task 2c.3 (sidechain peak detection)
    peak_bin:            [u32; MAX_PEAKS],
    peak_count:          usize,
    #[cfg(any(test, feature = "probe"))]
    last_probe:          crate::dsp::modules::ProbeSnapshot,
}

impl PunchModule {
    pub fn new() -> Self {
        Self {
            mode:        PunchMode::default(),
            fft_size:    2048,
            sample_rate: 44100.0,
            current_carve_depth: [Vec::new(), Vec::new()],
            drift_accum:         [Vec::new(), Vec::new()],
            peak_bin:            [0u32; MAX_PEAKS],
            peak_count:          0,
            #[cfg(any(test, feature = "probe"))]
            last_probe:          Default::default(),
        }
    }

    pub fn set_mode(&mut self, mode: PunchMode) { self.mode = mode; }
    pub fn mode(&self) -> PunchMode { self.mode }
}

impl Default for PunchModule {
    fn default() -> Self { Self::new() }
}

impl SpectralModule for PunchModule {
    fn reset(&mut self, sample_rate: f32, fft_size: usize) {
        self.sample_rate = sample_rate;
        self.fft_size    = fft_size;
        let n = fft_size / 2 + 1;
        for ch in 0..2 {
            self.current_carve_depth[ch] = vec![0.0; n];
            self.drift_accum[ch]         = vec![0.0; n];
        }
        self.peak_count = 0;
    }

    fn clear_state(&mut self) {
        for ch in 0..2 {
            self.current_carve_depth[ch].fill(0.0);
            self.drift_accum[ch].fill(0.0);
        }
        self.peak_count = 0;
    }

    fn process(
        &mut self,
        _channel:    usize,
        _stereo_link: StereoLink,
        _target:     FxChannelTarget,
        bins:        &mut [Complex<f32>],
        _sidechain:  Option<&[f32]>,
        _curves:     &[&[f32]],
        suppression_out: &mut [f32],
        _ctx:        &ModuleContext<'_>,
    ) {
        // Stub. Tasks 2c.3-2c.6 implement peak detection, carve, fill, healing.
        // No sidechain → no carve → pass-through.
        suppression_out.fill(0.0);
        let _ = bins;
    }

    fn module_type(&self) -> ModuleType { ModuleType::Punch }
    fn num_curves(&self) -> usize { 6 }

    #[cfg(any(test, feature = "probe"))]
    fn last_probe(&self) -> crate::dsp::modules::ProbeSnapshot { self.last_probe }
}
