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

// ── RM/FM Matrix kernel ────────────────────────────────────────────────────

fn apply_rm_fm_matrix(
    bins: &mut [Complex<f32>],
    sidechain: &[f32],
    curves: &[&[f32]],
) {
    use std::f32::consts::PI;

    let amount_c = curves[0];
    let reach_c  = curves[1];
    let thresh_c = curves[3];
    let mix_c    = curves[5];

    let num_bins = bins.len().min(sidechain.len());

    for k in 0..num_bins {
        let fm_blend = amount_c[k].clamp(0.0, 2.0) * 0.5; // 0=pure RM, 1=pure FM
        let reach    = reach_c[k].clamp(0.0, 4.0);
        let thresh   = thresh_c[k].clamp(0.0, 4.0) * 0.1;
        let mix      = mix_c[k].clamp(0.0, 2.0) * 0.5;

        let sc = sidechain[k].max(0.0);
        if sc <= thresh {
            // At or below threshold: leave bin untouched (passthrough).
            continue;
        }

        let dry = bins[k];

        // Ring-mod output: complex scale by real sidechain magnitude.
        let rm_out = dry * sc * reach;

        // FM output: phase rotation by sc·π radians, magnitude-preserving.
        // The plan body computes  (rotate dry) * dry.norm()  which gives
        // magnitude = dry.norm()² — wrong.  The corrected form is simply
        // rotate dry by the phase angle; since rotation is unitary the
        // magnitude stays exactly dry.norm().
        let phase = sc * PI;
        let cos_p = phase.cos();
        let sin_p = phase.sin();
        let fm_out = Complex::new(
            dry.re * cos_p - dry.im * sin_p,
            dry.re * sin_p + dry.im * cos_p,
        ); // magnitude = dry.norm() (no extra scaling needed)

        let wet = rm_out * (1.0 - fm_blend) + fm_out * fm_blend;
        bins[k] = dry * (1.0 - mix) + wet * mix;
    }
}

// ── Diode RM kernel ───────────────────────────────────────────────────────

fn apply_diode_rm(
    bins: &mut [Complex<f32>],
    sidechain: &[f32],
    curves: &[&[f32]],
) {
    let amount_c = curves[0];
    let reach_c  = curves[1];
    let thresh_c = curves[3];
    let mix_c    = curves[5];

    let num_bins = bins.len().min(sidechain.len());

    for k in 0..num_bins {
        let amount = amount_c[k].clamp(0.0, 2.0) * 0.5; // 0..1
        let reach  = reach_c[k].clamp(0.0, 4.0);
        let thresh = thresh_c[k].clamp(0.01, 4.0) * 0.5; // input level above which diode closes
        let mix    = mix_c[k].clamp(0.0, 2.0) * 0.5;

        let sc        = sidechain[k].max(0.0);
        let dry       = bins[k];
        let input_amp = dry.norm();

        // Mismatch coefficient: 0 = perfect match (no leak), 1 = max leak.
        let mismatch = (1.0 - input_amp / thresh).clamp(0.0, 1.0);

        // RM path: scaled product.
        let rm_path   = dry * sc * reach * amount;
        // Leak path: carrier passes through with phase preserved (real → real).
        let leak_path = Complex::new(sc * mismatch, 0.0);

        let wet   = rm_path + leak_path;
        bins[k]   = dry * (1.0 - mix) + wet * mix;
    }
}

// ── Ground Loop kernel ────────────────────────────────────────────────────

fn apply_ground_loop(
    bins: &mut [Complex<f32>],
    rms_history: &mut [f32; 16],
    rms_idx: &mut usize,
    sample_rate: f32,
    fft_size: usize,
    curves: &[&[f32]],
) {
    let amount_c = curves[0];
    let reach_c  = curves[1];
    let rate_c   = curves[2];
    let thresh_c = curves[3];
    let mix_c    = curves[5];

    let num_bins = bins.len();

    // Step 1 — record RMS.
    let mut sum_sq = 0.0_f32;
    for b in bins.iter() {
        sum_sq += b.norm_sqr();
    }
    let rms = (sum_sq / num_bins as f32).sqrt();
    rms_history[*rms_idx] = rms;
    *rms_idx = (*rms_idx + 1) % 16;

    // Step 2 — sag detection.
    let avg_rms: f32 = rms_history.iter().sum::<f32>() / 16.0;
    let thresh = thresh_c[0].clamp(0.001, 4.0);
    let sag_factor = (avg_rms / thresh).min(2.0);

    if sag_factor < 0.05 {
        return; // Below sag threshold: no hum injection.
    }

    // Step 3 — mains frequency: RATE < 1.0 → 50 Hz, RATE >= 1.0 → 60 Hz.
    let mains_hz = if rate_c[0] >= 1.0 { 60.0_f32 } else { 50.0_f32 };
    let mains_bin = ((mains_hz * fft_size as f32 / sample_rate).round() as usize).max(1);

    // Step 4 — harmonic count (1..5).
    let harmonics = (1.0_f32 + reach_c[0].clamp(0.0, 2.0) * 2.0).round() as usize;
    let harmonics = harmonics.clamp(1, 5);

    // Step 5 — global amount.
    let amount = amount_c[0].clamp(0.0, 2.0);

    // Step 6 — inject hum at mains_bin × h with 1/h falloff.
    for h in 1..=harmonics {
        let target = mains_bin * h;
        if target >= num_bins {
            break;
        }
        let harmonic_amp = amount * sag_factor / h as f32;
        let mix          = mix_c[target].clamp(0.0, 2.0) * 0.5;
        let cur_mag      = bins[target].norm().max(1e-9);
        let new_mag      = cur_mag + harmonic_amp;
        let scale        = new_mag / cur_mag;
        let dry          = bins[target];
        let wet          = bins[target] * scale;
        bins[target]     = dry * (1.0 - mix) + wet * mix;
    }
}

// ── Gravity Phaser kernel ─────────────────────────────────────────────────

fn apply_gravity_phaser(
    bins: &mut [Complex<f32>],
    smoothed: &[&[f32]; 6],
    phase_momentum: &mut [f32],
    repel: bool,
) {
    use std::f32::consts::PI;

    let amount_c  = smoothed[0];
    let reach_c   = smoothed[1];
    let _rate_c   = smoothed[2]; // animation rate consumed by SidechainPositioned (Task 5b4.6)
    let thresh_c  = smoothed[3];
    let ampgate_c = smoothed[4];
    let mix_c     = smoothed[5];

    let num_bins = bins.len();
    debug_assert!(phase_momentum.len() >= num_bins,
        "phase_momentum buffer too short: {} < {}", phase_momentum.len(), num_bins);
    let sign: f32 = if repel { -1.0 } else { 1.0 };

    for k in 0..num_bins {
        let amount  = amount_c[k].clamp(0.0, 2.0);
        let reach   = reach_c[k].clamp(0.0, 4.0);
        let thresh  = thresh_c[k].clamp(0.01, 4.0);
        let ampgate = ampgate_c[k].clamp(0.0, 2.0);
        let mix     = mix_c[k].clamp(0.0, 2.0) * 0.5;

        let mag = bins[k].norm();
        // Amp-gated drive: when ampgate > 0, scale per-bin drive by min(mag/thresh, 1).
        let gate_factor = if ampgate > 0.001 {
            (mag / thresh).min(1.0) * ampgate.min(1.0)
        } else {
            1.0
        };

        // Force = sign * amount * (reach * 0.05) — `reach` widens the per-bin influence.
        // 5%/hop momentum decay prevents unbounded growth.
        let force = sign * amount * reach * 0.05 * gate_factor;
        phase_momentum[k] = phase_momentum[k] * 0.95 + force;

        let rotation = phase_momentum[k] * PI;
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

// ── ModulateMode ───────────────────────────────────────────────────────────

/// Per-mode heavy-CPU markers for ModulateMode. Order MUST match enum declaration.
/// PhasePhaser, BinSwapper, RmFmMatrix, DiodeRm, GroundLoop, GravityPhaser, PllTear.
const MOD_HEAVY: [bool; 7] = [false, false, false, false, false, false, true /* [6] PllTear */];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModulateMode {
    PhasePhaser,
    BinSwapper,
    RmFmMatrix,
    DiodeRm,
    GroundLoop,
    GravityPhaser,
    PllTear,
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
    /// Smoothed copies of the 6 input curves, used by retrofit modes
    /// (GravityPhaser, PllTear) to defend against parametric instability.
    smoothed_curves: [[Vec<f32>; 6]; 2],
    /// Per-channel first-touch flag; primes the smoother with a direct copy
    /// on the first hop after reset (avoids 5-hop ramp-in artefact).
    smoothed_primed: [bool; 2],
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
            smoothed_curves: [
                [Vec::<f32>::new(), Vec::<f32>::new(), Vec::<f32>::new(), Vec::<f32>::new(), Vec::<f32>::new(), Vec::<f32>::new()],
                [Vec::<f32>::new(), Vec::<f32>::new(), Vec::<f32>::new(), Vec::<f32>::new(), Vec::<f32>::new(), Vec::<f32>::new()],
            ],
            smoothed_primed: [false; 2],
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

    /// Test helper — returns the first smoothed-curve buffer's length.
    /// `0` if `reset` has not been called yet.
    pub fn smoothed_curves_len(&self) -> usize {
        self.smoothed_curves[0][0].len()
    }

    /// Test helper — exposes the per-channel first-touch flag so tests can
    /// assert that v1 modes never feed the smoother.
    pub fn smoothed_primed_for_test(&self, channel: usize) -> bool {
        self.smoothed_primed[channel]
    }

    /// Borrow the 6 smoothed curves for `channel` as a fixed-size array.
    /// Used by retrofit-mode kernels.
    fn smoothed_curves_for(&self, channel: usize) -> [&[f32]; 6] {
        debug_assert!(channel < 2, "channel must be 0 or 1, got {}", channel);
        let c = &self.smoothed_curves[channel];
        [&c[0], &c[1], &c[2], &c[3], &c[4], &c[5]]
    }

    /// Refresh `smoothed_curves[channel]` from the raw input curves. Called only
    /// by retrofit modes; v1 modes consume `curves` directly. On the first hop
    /// after reset, the smoother is primed by direct copy (otherwise ~5-hop ramp).
    fn refresh_smoothed(&mut self, channel: usize, curves: &[&[f32]], num_bins: usize) {
        use crate::dsp::physics_helpers::smooth_curve_one_pole;
        debug_assert!(channel < 2, "channel must be 0 or 1, got {}", channel);
        debug_assert!(curves.len() >= 6, "refresh_smoothed expects 6 curves, got {}", curves.len());
        let dt = self.fft_size as f32 / self.sample_rate / 4.0; // hop = fft/4 (75% overlap)
        let primed = self.smoothed_primed[channel];
        let take = curves.len().min(6);
        for c in 0..take {
            let src = &curves[c][..num_bins];
            let dst = &mut self.smoothed_curves[channel][c][..num_bins];
            if !primed {
                dst.copy_from_slice(src);
            } else {
                smooth_curve_one_pole(dst, src, dt);
            }
        }
        self.smoothed_primed[channel] = true;
    }
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
        mut physics: Option<&mut crate::dsp::bin_physics::BinPhysics>,
        _ctx: &ModuleContext<'_>,
    ) {
        debug_assert!(channel < 2);

        // Probe capture: all 5 kernels share the same mapping for curves[0] and curves[5].
        // curves[0] (AMOUNT): g=1.0 → 50%, g=2.0 → 100%  (g.clamp(0,2) * 50.0)
        // curves[5] (MIX):   g=1.0 → 50%, g=2.0 → 100%  (g.clamp(0,2) * 50.0)
        #[cfg(any(test, feature = "probe"))]
        let probe_amount_pct = curves[0].get(0).copied().unwrap_or(0.0).clamp(0.0, 2.0) * 50.0;
        #[cfg(any(test, feature = "probe"))]
        let probe_mix_pct = curves[5].get(0).copied().unwrap_or(0.0).clamp(0.0, 2.0) * 50.0;

        match self.mode {
            ModulateMode::PhasePhaser => {
                apply_phase_phaser(bins, self.hop_count[channel], curves);
                self.hop_count[channel] = self.hop_count[channel].wrapping_add(1);
            }
            ModulateMode::BinSwapper => {
                let scratch = &mut self.swap_scratch[channel];
                apply_bin_swapper(bins, scratch, curves);
            }
            ModulateMode::RmFmMatrix => {
                if let Some(sc) = sidechain {
                    apply_rm_fm_matrix(bins, sc, curves);
                }
                // No sidechain → passthrough (bins unchanged).
            }
            ModulateMode::DiodeRm => {
                if let Some(sc) = sidechain {
                    apply_diode_rm(bins, sc, curves);
                }
                // No sidechain → passthrough (bins unchanged).
            }
            ModulateMode::GroundLoop => {
                let history = &mut self.rms_history[channel];
                let idx     = &mut self.rms_idx[channel];
                apply_ground_loop(bins, history, idx, self.sample_rate, self.fft_size, curves);
            }
            ModulateMode::GravityPhaser => {
                let num_bins = bins.len();
                self.refresh_smoothed(channel, curves, num_bins);
                let smoothed = self.smoothed_curves_for(channel);
                if let Some(p) = physics.as_mut() {
                    let momentum = &mut p.phase_momentum[..num_bins];
                    apply_gravity_phaser(bins, &smoothed, momentum, /* repel */ false);
                } else {
                    debug_assert!(false,
                        "GravityPhaser requires Some(physics) — FxMatrix must supply it for writes_bin_physics modules");
                }
            }
            ModulateMode::PllTear => {
                // Kernel added in Task 5b4.7. Pass through unchanged.
            }
        }

        for s in suppression_out.iter_mut() { *s = 0.0; }

        #[cfg(any(test, feature = "probe"))]
        {
            self.last_probe = crate::dsp::modules::ProbeSnapshot {
                amount_pct: Some(probe_amount_pct),
                mix_pct:    Some(probe_mix_pct),
                ..Default::default()
            };
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
            for c in 0..6 {
                self.smoothed_curves[ch][c].clear();
                self.smoothed_curves[ch][c].resize(num_bins, 0.0);
            }
            self.smoothed_primed[ch] = false;
        }
        self.hop_count = [0; 2];
        // self.mode is preserved across reset (user choice survives FFT-size change).
    }

    fn module_type(&self) -> ModuleType { ModuleType::Modulate }
    fn num_curves(&self) -> usize { 6 }

    fn heavy_cpu_for_mode(&self) -> bool {
        MOD_HEAVY[self.mode as usize]
    }

    fn set_modulate_mode(&mut self, mode: ModulateMode) {
        self.set_mode(mode);
    }

    #[cfg(any(test, feature = "probe"))]
    fn last_probe(&self) -> crate::dsp::modules::ProbeSnapshot { self.last_probe }
}
