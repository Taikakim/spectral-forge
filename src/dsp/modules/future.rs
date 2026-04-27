use num_complex::Complex;
use serde::{Deserialize, Serialize};
use crate::params::{FxChannelTarget, StereoLink};
use super::{ModuleContext, ModuleType, SpectralModule};

pub const MAX_ECHO_FRAMES: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum FutureMode {
    #[default]
    PrintThrough,
    PreEcho,
}

impl FutureMode {
    pub fn label(self) -> &'static str {
        match self {
            FutureMode::PrintThrough => "Print-Through",
            FutureMode::PreEcho      => "Pre-Echo",
        }
    }
}

pub struct FutureModule {
    mode:        FutureMode,
    fft_size:    usize,
    sample_rate: f32,
    /// Ring buffer of write-ahead frames per channel. `[channel][frame_idx][bin]`.
    pub ring:    [Vec<Vec<Complex<f32>>>; 2],
    write_pos:   [usize; 2],
}

impl FutureModule {
    pub fn new() -> Self {
        Self {
            mode:        FutureMode::default(),
            fft_size:    2048,
            sample_rate: 44100.0,
            ring:        [Vec::new(), Vec::new()],
            write_pos:   [0; 2],
        }
    }

    pub fn set_mode(&mut self, mode: FutureMode) { self.mode = mode; }
    pub fn mode(&self) -> FutureMode { self.mode }
}

impl Default for FutureModule {
    fn default() -> Self { Self::new() }
}

impl SpectralModule for FutureModule {
    fn reset(&mut self, sample_rate: f32, fft_size: usize) {
        self.sample_rate = sample_rate;
        self.fft_size    = fft_size;
        let n = fft_size / 2 + 1;
        for ch in 0..2 {
            self.ring[ch] = (0..MAX_ECHO_FRAMES)
                .map(|_| vec![Complex::new(0.0, 0.0); n])
                .collect();
            self.write_pos[ch] = 0;
        }
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
        // Stub. Tasks 3 + 4 implement Print-Through and Pre-Echo kernels.
        suppression_out.fill(0.0);
        let _ = bins;
    }

    fn tail_length(&self) -> u32 { (self.fft_size as u32) * (MAX_ECHO_FRAMES as u32) / 4 }
    fn module_type(&self) -> ModuleType { ModuleType::Future }
    fn num_curves(&self) -> usize { 5 }
}
