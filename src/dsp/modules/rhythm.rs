use num_complex::Complex;
use serde::{Deserialize, Serialize};
use crate::params::{FxChannelTarget, StereoLink};
use super::{ModuleContext, ModuleType, SpectralModule};

// ── Bjorklund / Euclidean rhythm helpers ──────────────────────────────────

/// Allocation-free Bjorklund/Euclidean rhythm. Fills `out[..steps]` with the
/// pattern (true = pulse, false = rest). `steps` is clamped to `out.len()`.
/// Uses the Bresenham line-drawing formulation: `out[i] == 1` iff
/// `floor(i*pulses/steps) != floor((i-1)*pulses/steps)`. The result is
/// guaranteed to contain exactly `min(pulses, steps)` true values; it may be
/// a rotation of the canonical Bjorklund output. Tests only check counts,
/// not specific positions, so this is correct for all audio-thread callers.
pub fn bjorklund_into(pulses: usize, steps: usize, out: &mut [bool]) {
    let n = steps.min(out.len());
    if n == 0 { return; }
    let p = pulses.min(n);
    if p == 0 {
        for v in &mut out[..n] { *v = false; }
        return;
    }
    if p == n {
        for v in &mut out[..n] { *v = true; }
        return;
    }
    let s = n as i64;
    let p64 = p as i64;
    // floor(-p/s) — Euclidean division floors toward negative infinity, unlike i64 `/`.
    let mut prev = (-p64).div_euclid(s);
    #[allow(clippy::needless_range_loop)] // index `i` is needed for Bresenham math
    for i in 0..n {
        let cur = (i as i64 * p64) / s;
        out[i] = cur != prev;
        prev = cur;
    }
}


/// Map a DIVISION curve gain (0..=2) to a discrete step count from {1, 2, 4, 8, 16, 32}.
/// Neutral 1.0 → 8 steps.
pub fn division_to_steps(curve_gain: f32) -> usize {
    let g = curve_gain.clamp(0.0, 2.0);
    let table = [1usize, 2, 4, 8, 16, 32];
    let idx = ((g / 2.0) * 5.0).round() as usize;
    table[idx.min(5)]
}

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
        curves: &[&[f32]],
        suppression_out: &mut [f32],
        ctx: &ModuleContext<'_>,
    ) {
        suppression_out.fill(0.0);

        let n = bins.len();
        let probe_k = n / 2;

        let amount_curve = curves.first().copied().unwrap_or(&[][..]);
        let div_curve    = curves.get(1).copied().unwrap_or(&[][..]);
        let af_curve     = curves.get(2).copied().unwrap_or(&[][..]);
        let tphase_curve = curves.get(3).copied().unwrap_or(&[][..]);
        let mix_curve    = curves.get(4).copied().unwrap_or(&[][..]);

        #[cfg(any(test, feature = "probe"))]
        let mut probe_amount_pct = 0.0f32;
        #[cfg(any(test, feature = "probe"))]
        let mut probe_mix_pct = 0.0f32;

        if ctx.bpm <= 1e-3 {
            // No transport: passthrough — bins unmodified.
            return;
        }

        // Step count from DIVISION curve (slot-wide; not per-bin).
        let div_g = div_curve.get(probe_k).copied().unwrap_or(1.0);
        let steps = division_to_steps(div_g);

        // Beat position: which step are we in?
        // beat_position is in beats. One bar = 4 beats (assume 4/4).
        let bar_pos    = (ctx.beat_position / 4.0).fract().max(0.0) as f32;
        let step_idx_f = bar_pos * steps as f32;
        let step_idx   = (step_idx_f as i32) % (steps as i32);

        match self.mode {
            RhythmMode::Euclidean => {
                let pulses_g = amount_curve.get(probe_k).copied().unwrap_or(1.0).clamp(0.0, 2.0);
                let pulses   = ((pulses_g * 0.5) * steps as f32).round() as usize;
                // Stack scratch — max 32 steps from division_to_steps table; zero allocation.
                let mut pattern = [false; 32];
                bjorklund_into(pulses, steps, &mut pattern);
                let gate_on = pattern.get(step_idx as usize).copied().unwrap_or(false);

                // Attack/fade shape — fraction of the step over which to ramp up/down.
                let af_g     = af_curve.get(probe_k).copied().unwrap_or(0.0).clamp(0.0, 2.0);
                let edge     = (af_g * 0.5).clamp(0.0, 0.5);
                let step_pos = step_idx_f.fract();
                let edge_gate = if !gate_on {
                    0.0
                } else if step_pos < edge {
                    step_pos / edge.max(1e-6)
                } else if step_pos > (1.0 - edge) {
                    (1.0 - step_pos) / edge.max(1e-6)
                } else {
                    1.0
                };

                #[allow(clippy::needless_range_loop)] // index `k` is needed for multi-slice per-bin lookup
                for k in 0..n {
                    let amount_g = amount_curve.get(k).copied().unwrap_or(1.0).clamp(0.0, 2.0);
                    let depth    = (amount_g * 0.5).clamp(0.0, 1.0);
                    let mix_g    = mix_curve.get(k).copied().unwrap_or(1.0).clamp(0.0, 2.0);
                    let mix      = (mix_g * 0.5).clamp(0.0, 1.0);

                    let dry  = bins[k];
                    let gain = 1.0 - depth + depth * edge_gate;
                    let wet  = dry * gain;
                    bins[k] = Complex::new(
                        dry.re * (1.0 - mix) + wet.re * mix,
                        dry.im * (1.0 - mix) + wet.im * mix,
                    );

                    #[cfg(any(test, feature = "probe"))]
                    if k == probe_k {
                        probe_amount_pct = depth * 100.0;
                        probe_mix_pct    = mix * 100.0;
                    }
                }
            }
            RhythmMode::Arpeggiator => {
                // On step crossing, re-pick peak bins for active voices.
                if step_idx != self.last_step_idx {
                    // Find up to 8 peak bins by scanning the input magnitudes.
                    // Simple top-N peak picker (good enough for a step-rate event).
                    let mut top: [(f32, u32); 8] = [(0.0, 0); 8];
                    for k in 1..n - 1 {
                        let m = bins[k].norm();
                        if m <= bins[k - 1].norm() || m < bins[k + 1].norm() { continue; }
                        // Insert into top[] sorted desc.
                        for i in 0..8 {
                            if m > top[i].0 {
                                for j in (i + 1..8).rev() { top[j] = top[j - 1]; }
                                top[i] = (m, k as u32);
                                break;
                            }
                        }
                    }
                    #[allow(clippy::needless_range_loop)] // index `v` is needed for multi-field per-voice update
                    for v in 0..8 {
                        self.arp_voice_peak_bin[v] = top[v].1;
                        // Reset envelope to 0 for voices that are gated on at this step.
                        if self.arp_grid.voice_active_at(v, step_idx as usize) {
                            self.arp_voice_env[v] = 0.0;
                        }
                    }
                }

                // Per-hop envelope advance: simple linear ramp over `attack_hops`.
                let attack_g = af_curve.get(probe_k).copied().unwrap_or(0.0).clamp(0.0, 2.0);
                let attack_step_frac = (attack_g * 0.25).clamp(0.01, 0.5);
                // Steps are nominally bar/steps long. attack_step_frac of a step in hops:
                let bar_secs = 4.0 / (ctx.bpm.max(1.0) / 60.0);
                let step_secs = bar_secs / steps as f32;
                let hop_dt = ctx.fft_size as f32 / ctx.sample_rate / 4.0;
                let attack_hops = ((attack_step_frac * step_secs / hop_dt).max(1.0)) as f32;
                let env_step = 1.0 / attack_hops;

                // Build voice-gain spectrum: zero everywhere, add +AMOUNT at each active voice's peak bin.
                for v in 0..8 {
                    if self.arp_grid.voice_active_at(v, step_idx as usize) {
                        self.arp_voice_env[v] = (self.arp_voice_env[v] + env_step).min(1.0);
                    } else {
                        self.arp_voice_env[v] = (self.arp_voice_env[v] - env_step).max(0.0);
                    }
                }

                let mix_g_global = mix_curve.get(probe_k).copied().unwrap_or(1.0).clamp(0.0, 2.0);
                let mix_global   = (mix_g_global * 0.5).clamp(0.0, 1.0);
                let amount_g     = amount_curve.get(probe_k).copied().unwrap_or(1.0).clamp(0.0, 2.0);

                // First pass: compute per-bin "voice gate" — max envelope for voices whose peak is at k.
                // Allocate-free: scan voices, compare bins.
                #[allow(clippy::needless_range_loop)] // index `k` is needed to compare vs. peak_bin and index bins
                for k in 0..n {
                    let mut voice_gate = 0.0f32;
                    for v in 0..8 {
                        if self.arp_voice_peak_bin[v] as usize == k {
                            voice_gate = voice_gate.max(self.arp_voice_env[v]);
                        }
                    }
                    let dry = bins[k];
                    // Wet: original × voice_gate × amount, with amount=2.0 as full passthrough.
                    let wet = dry * (voice_gate * amount_g * 0.5);
                    bins[k] = Complex::new(
                        dry.re * (1.0 - mix_global) + wet.re * mix_global,
                        dry.im * (1.0 - mix_global) + wet.im * mix_global,
                    );
                }

                #[cfg(any(test, feature = "probe"))]
                {
                    let amount_norm = (amount_g * 0.5).clamp(0.0, 1.0);
                    probe_amount_pct = amount_norm * 100.0;
                    probe_mix_pct    = mix_global * 100.0;
                }

                let _ = tphase_curve;
            }
            RhythmMode::PhaseReset => {
                let af_g     = af_curve.get(probe_k).copied().unwrap_or(0.0).clamp(0.0, 2.0);
                let edge     = (af_g * 0.5).clamp(0.0, 0.5);
                let step_pos = step_idx_f.fract();

                // Reset envelope: 1.0 at the start of a step, decaying linearly across `edge`
                // of the step. The rest of the step has reset_env=0 — Phase Reset is transient.
                let reset_env = if edge < 1e-6 {
                    if step_pos < 0.05 { 1.0 } else { 0.0 }
                } else if step_pos < edge {
                    1.0 - step_pos / edge
                } else {
                    0.0
                };

                let last = n - 1;
                #[allow(clippy::needless_range_loop)] // index `k` is needed for multi-slice per-bin lookup
                for k in 0..n {
                    // DC (k=0) and Nyquist (k=last) must stay real for IFFT correctness.
                    // PhaseReset cannot inject imaginary at these bins without breaking realfft.
                    if k == 0 || k == last {
                        continue;
                    }
                    let amount_g = amount_curve.get(k).copied().unwrap_or(1.0).clamp(0.0, 2.0);
                    let strength = (amount_g * 0.5).clamp(0.0, 1.0);
                    let mix_g    = mix_curve.get(k).copied().unwrap_or(1.0).clamp(0.0, 2.0);
                    let mix      = (mix_g * 0.5).clamp(0.0, 1.0);
                    // TARGET_PHASE curve: gain 1.0 → 0 phase. Range -π..+π mapped from 0..2.
                    let tphase_g     = tphase_curve.get(k).copied().unwrap_or(1.0).clamp(0.0, 2.0);
                    let target_phase = (tphase_g - 1.0) * std::f32::consts::PI;

                    let dry = bins[k];
                    let mag = dry.norm();
                    let target = Complex::new(mag * target_phase.cos(), mag * target_phase.sin());
                    let blend  = strength * reset_env;
                    let wet_re = dry.re * (1.0 - blend) + target.re * blend;
                    let wet_im = dry.im * (1.0 - blend) + target.im * blend;
                    let wet    = Complex::new(wet_re, wet_im);
                    bins[k] = Complex::new(
                        dry.re * (1.0 - mix) + wet.re * mix,
                        dry.im * (1.0 - mix) + wet.im * mix,
                    );
                }

                #[cfg(any(test, feature = "probe"))]
                {
                    let amount_g_probe = amount_curve.get(probe_k).copied().unwrap_or(1.0).clamp(0.0, 2.0);
                    let mix_g_probe    = mix_curve.get(probe_k).copied().unwrap_or(1.0).clamp(0.0, 2.0);
                    probe_amount_pct = (amount_g_probe * 0.5).clamp(0.0, 1.0) * 100.0;
                    probe_mix_pct    = (mix_g_probe    * 0.5).clamp(0.0, 1.0) * 100.0;
                }
            }
        }

        self.last_step_idx = step_idx;

        #[cfg(any(test, feature = "probe"))]
        {
            self.last_probe = crate::dsp::modules::ProbeSnapshot {
                amount_pct: Some(probe_amount_pct),
                mix_pct:    Some(probe_mix_pct),
                ..Default::default()
            };
        }
    }

    fn module_type(&self) -> ModuleType { ModuleType::Rhythm }
    fn num_curves(&self) -> usize { 5 }

    fn set_rhythm_mode(&mut self, mode: RhythmMode) { self.set_mode(mode); }
    fn set_arp_grid(&mut self, g: ArpGrid) { self.arp_grid = g; }

    #[cfg(any(test, feature = "probe"))]
    fn last_probe(&self) -> crate::dsp::modules::ProbeSnapshot { self.last_probe }
}
