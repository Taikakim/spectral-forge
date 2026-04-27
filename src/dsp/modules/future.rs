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
    ring:        [Vec<Vec<Complex<f32>>>; 2],
    write_pos:   [usize; 2],
    /// Per-channel scratch for the PrintThrough two-pass spread: pass 1 stores
    /// the complex side-bleed value here so pass 2 can `+=` it to ring neighbours
    /// without losing dry phase. Allocated in `reset()`, contents are transient
    /// per `process()` call.
    spread_scratch: [Vec<Complex<f32>>; 2],
    #[cfg(any(test, feature = "probe"))]
    last_probe: crate::dsp::modules::ProbeSnapshot,
}

impl FutureModule {
    pub fn new() -> Self {
        Self {
            mode:        FutureMode::default(),
            fft_size:    2048,
            sample_rate: 44100.0,
            ring:        [Vec::new(), Vec::new()],
            write_pos:   [0; 2],
            spread_scratch: [Vec::new(), Vec::new()],
            #[cfg(any(test, feature = "probe"))]
            last_probe: Default::default(),
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
            self.spread_scratch[ch] = vec![Complex::new(0.0, 0.0); n];
        }
    }

    fn process(
        &mut self,
        channel: usize,
        _stereo_link: StereoLink,
        _target: FxChannelTarget,
        bins: &mut [Complex<f32>],
        _sidechain: Option<&[f32]>,
        curves: &[&[f32]],
        suppression_out: &mut [f32],
        _ctx: &ModuleContext<'_>,
    ) {
        let ch = channel.min(1);
        let n  = bins.len();
        debug_assert_eq!(self.ring[ch][0].len(), n,
            "FutureModule: bins/ring size mismatch — call reset() before process()");

        let probe_k = n / 2;
        let amount_curve = curves.get(0).copied().unwrap_or(&[][..]);
        let time_curve   = curves.get(1).copied().unwrap_or(&[][..]);
        let spread_curve = curves.get(3).copied().unwrap_or(&[][..]);
        let mix_curve    = curves.get(4).copied().unwrap_or(&[][..]);

        #[cfg(any(test, feature = "probe"))]
        let mut probe_amount_pct = 0.0f32;
        #[cfg(any(test, feature = "probe"))]
        let mut probe_time_hops  = 0u32;
        #[cfg(any(test, feature = "probe"))]
        let mut probe_mix_pct    = 0.0f32;

        match self.mode {
            FutureMode::PrintThrough => {
                // Slot-wide TIME (read once at probe_k).
                let time_gain  = time_curve.get(probe_k).copied().unwrap_or(1.0).clamp(0.0, 2.0);
                let delay_hops = ((time_gain * 8.0).round() as usize).clamp(1, MAX_ECHO_FRAMES - 1);
                let read_pos   = (self.write_pos[ch] + MAX_ECHO_FRAMES - delay_hops) % MAX_ECHO_FRAMES;

                // Pass 1: mix wet→bins; write centre leaked values; store full complex
                // side-bleed in spread_scratch so pass 2 can accumulate to neighbours
                // without losing the original dry phase.
                for k in 0..n {
                    let amount_gain = amount_curve.get(k).copied().unwrap_or(1.0).clamp(0.0, 4.0);
                    let leak_pct    = (amount_gain * 0.05).clamp(0.0, 0.20);
                    let spread_gain = spread_curve.get(k).copied().unwrap_or(0.0).clamp(0.0, 2.0);
                    let spread_pct  = (spread_gain * 0.20).clamp(0.0, 0.50);
                    let mix_gain    = mix_curve.get(k).copied().unwrap_or(1.0).clamp(0.0, 2.0);
                    let mix         = (mix_gain * 0.5).clamp(0.0, 1.0);

                    let dry = bins[k];
                    let wet = self.ring[ch][read_pos][k];
                    bins[k] = Complex::new(
                        dry.re * (1.0 - mix) + wet.re * mix,
                        dry.im * (1.0 - mix) + wet.im * mix,
                    );

                    #[cfg(any(test, feature = "probe"))]
                    if k == probe_k {
                        probe_amount_pct = leak_pct * 100.0;
                        probe_time_hops  = delay_hops as u32;
                        probe_mix_pct    = mix * 100.0;
                    }

                    let dry_norm   = dry.norm();
                    let leaked_mag = dry_norm * leak_pct;
                    let phase_unit = if dry_norm > 1e-12 { dry / dry_norm } else { Complex::new(1.0, 0.0) };
                    // Write centre into ring (ring slot was pre-cleared at end of last hop).
                    self.ring[ch][self.write_pos[ch]][k] = phase_unit * (leaked_mag * (1.0 - 2.0 * spread_pct));
                    // Store full complex side value — preserves dry phase even when centre is zero.
                    self.spread_scratch[ch][k] = phase_unit * (leaked_mag * spread_pct);
                }
                // Pass 2: accumulate side bleeds into neighbours (all centres are now written).
                for k in 0..n {
                    let side = self.spread_scratch[ch][k];
                    if side.norm_sqr() < 1e-48 { continue; }
                    if k > 0     { self.ring[ch][self.write_pos[ch]][k - 1] += side; }
                    if k + 1 < n { self.ring[ch][self.write_pos[ch]][k + 1] += side; }
                }
            }
            FutureMode::PreEcho => {
                // Task 4 implements the Pre-Echo kernel.
                // Stub: copy dry into ring (no echo until kernel lands).
                for k in 0..n {
                    self.ring[ch][self.write_pos[ch]][k] = bins[k];
                }
            }
        }

        // Advance write_pos.
        self.write_pos[ch] = (self.write_pos[ch] + 1) % MAX_ECHO_FRAMES;
        // Pre-clear the next slot so the += spread accumulators on the next hop start from zero.
        let next_pos = self.write_pos[ch];
        for k in 0..n { self.ring[ch][next_pos][k] = Complex::new(0.0, 0.0); }

        suppression_out.fill(0.0);

        #[cfg(any(test, feature = "probe"))]
        {
            let hop_size = (self.fft_size as f32) / 4.0;
            let length_ms = (probe_time_hops as f32) * hop_size / self.sample_rate * 1000.0;
            self.last_probe = crate::dsp::modules::ProbeSnapshot {
                amount_pct: Some(probe_amount_pct),
                length_ms:  Some(length_ms),
                mix_pct:    Some(probe_mix_pct),
                ..Default::default()
            };
        }
    }

    fn clear_state(&mut self) {
        for ch in 0..2 {
            for frame in self.ring[ch].iter_mut() {
                frame.fill(Complex::new(0.0, 0.0));
            }
            self.spread_scratch[ch].fill(Complex::new(0.0, 0.0));
        }
        self.write_pos = [0; 2];
    }

    #[cfg(any(test, feature = "probe"))]
    fn last_probe(&self) -> crate::dsp::modules::ProbeSnapshot { self.last_probe }

    fn tail_length(&self) -> u32 { (self.fft_size as u32) * (MAX_ECHO_FRAMES as u32) / 4 }
    fn module_type(&self) -> ModuleType { ModuleType::Future }
    fn num_curves(&self) -> usize { 5 }
}
