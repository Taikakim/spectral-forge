//! Modulate module — spectral modulation / ring-mod / cross-synthesis effects.
//!
//! Five modes ship across Phase 2f tasks:
//! - **PhasePhaser**  — per-bin animated phase rotation driven by a RATE curve.
//! - **BinSwapper**   — displaces bin energy by a REACH offset with wet/dry blend.
//! - **RmFmMatrix**   — ring-mod (magnitude) and frequency-mod (bin-shift) from sidechain.
//! - **DiodeRm**      — amplitude-gated leaky ring mod (AMPGATE curve controls threshold).
//! - **GroundLoop**   — mains-hum injection + sag-gated harmonic spray.
//!
//! Kernel implementations are added in Tasks 3–7 of Phase 2f. This skeleton
//! provides the enum, struct, and stub `process()` that passes audio through
//! unmodified and zeroes suppression_out.

use num_complex::Complex;
use serde::{Deserialize, Serialize};

use crate::dsp::modules::{
    FxChannelTarget, ModuleContext, ModuleType, SpectralModule,
};
use crate::params::StereoLink;

// ── ModulateMode ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModulateMode {
    PhasePhaser,
    BinSwapper,
    RmFmMatrix,
    DiodeRm,
    GroundLoop,
}

impl Default for ModulateMode {
    fn default() -> Self { ModulateMode::PhasePhaser }
}

// ── ModulateModule ─────────────────────────────────────────────────────────

pub struct ModulateModule {
    mode: ModulateMode,
    /// Accumulated hop count per channel (used by phase animation kernels).
    hop_count: [u64; 2],
    /// Per-channel scratch buffer for BinSwapper (length = num_bins after reset).
    swap_scratch: [Vec<f32>; 2],
    /// Per-channel RMS history ring buffers (16 frames each).
    rms_history: [[f32; 16]; 2],
    /// Current write index into rms_history for each channel.
    rms_idx: [usize; 2],
    sample_rate: f32,
    fft_size: usize,
    #[cfg(any(test, feature = "probe"))]
    last_probe: crate::dsp::modules::ProbeSnapshot,
}

impl ModulateModule {
    pub fn new() -> Self {
        Self {
            mode:         ModulateMode::default(),
            hop_count:    [0; 2],
            swap_scratch: [Vec::new(), Vec::new()],
            rms_history:  [[0.0; 16]; 2],
            rms_idx:      [0; 2],
            sample_rate:  48_000.0,
            fft_size:     2048,
            #[cfg(any(test, feature = "probe"))]
            last_probe:   crate::dsp::modules::ProbeSnapshot::default(),
        }
    }

    /// Test/UI helper — update the operating mode and clear transient state.
    pub fn set_mode(&mut self, mode: ModulateMode) {
        if mode != self.mode {
            self.hop_count    = [0; 2];
            self.rms_history  = [[0.0; 16]; 2];
            self.rms_idx      = [0; 2];
            for ch in 0..2 {
                for v in self.swap_scratch[ch].iter_mut() { *v = 0.0; }
            }
            self.mode = mode;
        }
    }

    pub fn current_mode(&self) -> ModulateMode { self.mode }
}

impl SpectralModule for ModulateModule {
    fn process(
        &mut self,
        _channel: usize,
        _stereo_link: StereoLink,
        _target: FxChannelTarget,
        _bins: &mut [Complex<f32>],
        _sidechain: Option<&[f32]>,
        _curves: &[&[f32]],
        suppression_out: &mut [f32],
        _ctx: &ModuleContext<'_>,
    ) {
        // Stub: audio passes through unmodified. Kernels added in Tasks 3–7.
        for s in suppression_out.iter_mut() { *s = 0.0; }

        #[cfg(any(test, feature = "probe"))]
        {
            self.last_probe = crate::dsp::modules::ProbeSnapshot::default();
        }
    }

    fn reset(&mut self, sample_rate: f32, fft_size: usize) {
        self.sample_rate = sample_rate;
        self.fft_size    = fft_size;
        let num_bins = fft_size / 2 + 1;
        for ch in 0..2 {
            self.swap_scratch[ch].clear();
            self.swap_scratch[ch].resize(num_bins, 0.0);
            self.rms_history[ch] = [0.0; 16];
            self.rms_idx[ch]     = 0;
        }
        self.hop_count = [0; 2];
        // self.mode is preserved across reset (user choice survives FFT-size change).
    }

    fn module_type(&self) -> ModuleType { ModuleType::Modulate }
    fn num_curves(&self) -> usize { 6 }

    fn set_modulate_mode(&mut self, mode: ModulateMode) {
        self.set_mode(mode);
    }

    #[cfg(any(test, feature = "probe"))]
    fn last_probe(&self) -> crate::dsp::modules::ProbeSnapshot { self.last_probe }
}
