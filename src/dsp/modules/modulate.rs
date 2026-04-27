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

// ── Phase Phaser kernel ────────────────────────────────────────────────────

fn apply_phase_phaser(
    bins: &mut [Complex<f32>],
    hop_count: u64,
    curves: &[&[f32]],
) {
    use std::f32::consts::PI;

    let amount_c  = curves[0];
    let rate_c    = curves[2];
    let thresh_c  = curves[3];
    let ampgate_c = curves[4];
    let mix_c     = curves[5];

    let num_bins = bins.len();
    let hop_phase_base = hop_count as f32 * 0.01;

    for k in 0..num_bins {
        let amount       = amount_c[k].clamp(0.0, 2.0);
        let rate         = rate_c[k].clamp(0.0, 4.0);
        let thresh       = thresh_c[k].clamp(0.01, 4.0);
        let gate_strength = ampgate_c[k].clamp(0.0, 2.0);
        let mix          = mix_c[k].clamp(0.0, 2.0) * 0.5;

        let mag = bins[k].norm();
        let gate_factor = if gate_strength > 0.001 {
            (mag / thresh).min(1.0) * gate_strength.min(1.0)
        } else {
            1.0
        };
        let rotation = amount * PI * (hop_phase_base * rate + k as f32 * 0.001).sin() * gate_factor;
        let cos_r = rotation.cos();
        let sin_r = rotation.sin();
        let dry = bins[k];
        let wet = Complex::new(
            dry.re * cos_r - dry.im * sin_r,
            dry.re * sin_r + dry.im * cos_r,
        );
        bins[k] = dry * (1.0 - mix) + wet * mix;
    }
}

// ── Bin Swapper kernel ────────────────────────────────────────────────────

fn apply_bin_swapper(
    bins: &mut [Complex<f32>],
    scratch: &mut [Complex<f32>],
    curves: &[&[f32]],
) {
    let amount_c = curves[0];
    let reach_c  = curves[1];
    let thresh_c = curves[3];
    let mix_c    = curves[5];

    let num_bins = bins.len();

    // Snapshot current bins into scratch — needed because swap reads other indices.
    scratch[..num_bins].copy_from_slice(&bins[..num_bins]);

    for k in 0..num_bins {
        let amount = amount_c[k].clamp(0.0, 2.0) * 0.5; // 0..1 blend
        let reach  = reach_c[k].clamp(0.0, 4.0);
        let offset = (reach * 5.0).round() as i32;       // up to 20 bins offset
        let thresh = thresh_c[k].clamp(0.0, 4.0) * 0.1; // magnitude floor
        let mix    = mix_c[k].clamp(0.0, 2.0) * 0.5;    // 0..1

        let cur_mag = scratch[k].norm();
        if cur_mag < thresh {
            // Below threshold: leave bin untouched.
            continue;
        }

        let target_idx = (k as i32 + offset).clamp(0, num_bins as i32 - 1) as usize;
        let dry   = scratch[k];
        let other = scratch[target_idx];
        let wet   = dry * (1.0 - amount) + other * amount;
        bins[k]   = dry * (1.0 - mix) + wet * mix;
    }
}

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
    swap_scratch: [Vec<Complex<f32>>; 2],
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
            swap_scratch: [Vec::<Complex<f32>>::new(), Vec::<Complex<f32>>::new()],
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
                for v in self.swap_scratch[ch].iter_mut() { *v = Complex::new(0.0, 0.0); }
            }
            self.mode = mode;
        }
    }

    pub fn current_mode(&self) -> ModulateMode { self.mode }
}

impl SpectralModule for ModulateModule {
    fn process(
        &mut self,
        channel: usize,
        _stereo_link: StereoLink,
        _target: FxChannelTarget,
        bins: &mut [Complex<f32>],
        sidechain: Option<&[f32]>,
        curves: &[&[f32]],
        suppression_out: &mut [f32],
        _ctx: &ModuleContext<'_>,
    ) {
        debug_assert!(channel < 2);
        let _ = sidechain; // Used by RM modes — silence warnings until Task 5.

        match self.mode {
            ModulateMode::PhasePhaser => {
                apply_phase_phaser(bins, self.hop_count[channel], curves);
                self.hop_count[channel] = self.hop_count[channel].wrapping_add(1);
            }
            ModulateMode::BinSwapper => {
                let scratch = &mut self.swap_scratch[channel];
                apply_bin_swapper(bins, scratch, curves);
            }
            _ => {
                // Other modes filled in subsequent tasks.
            }
        }

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
            self.swap_scratch[ch].resize(num_bins, Complex::new(0.0, 0.0));
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
