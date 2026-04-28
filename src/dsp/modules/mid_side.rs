use num_complex::Complex;
use crate::params::{FxChannelTarget, StereoLink};
use crate::dsp::utils::xorshift64;
use super::{ModuleContext, ModuleType, SpectralModule};

pub struct MidSideModule {
    /// xorshift64 state for phase decorrelation. Must never be zero.
    rng_state: u64,
    num_bins:  usize,
    /// Phase 4.3d — per-module PLPV peak-aligned mid/side decode enable.
    /// Mirrors `params.plpv_midside_enable`; written each audio block by the
    /// Pipeline via `FxMatrix::set_plpv_midside_enable`. Default `true` matches
    /// the param default so a freshly-constructed module behaves identically
    /// to one with the param applied.
    plpv_enabled: bool,
    #[cfg(any(test, feature = "probe"))]
    last_probe: crate::dsp::modules::ProbeSnapshot,
}

impl MidSideModule {
    pub fn new() -> Self {
        Self {
            rng_state: 0xdeadbeefcafebabe,
            num_bins: 0,
            plpv_enabled: true,
            #[cfg(any(test, feature = "probe"))]
            last_probe: Default::default(),
        }
    }


}

impl Default for MidSideModule {
    fn default() -> Self { Self::new() }
}

impl SpectralModule for MidSideModule {
    fn reset(&mut self, _sr: f32, fft_size: usize) {
        self.num_bins = fft_size / 2 + 1;
        self.rng_state = 0xdeadbeefcafebabe;
    }

    fn set_plpv_midside_enabled(&mut self, enabled: bool) {
        self.plpv_enabled = enabled;
    }

    fn process(
        &mut self,
        channel:      usize,
        _stereo_link: StereoLink,
        _target:      FxChannelTarget,
        bins:         &mut [Complex<f32>],
        _sidechain:   Option<&[f32]>,
        curves:       &[&[f32]],
        suppression_out: &mut [f32],
        _physics:     Option<&mut crate::dsp::bin_physics::BinPhysics>,
        ctx:          &ModuleContext<'_>,
    ) {
        suppression_out.fill(0.0);

        let n = bins.len();

        // Curve indices (per module_spec order):
        // 0 = BALANCE, 1 = EXPANSION, 2 = DECORREL, 3 = TRANSIENT (stub), 4 = PAN (stub)
        let balance   = curves.get(0).copied().unwrap_or(&[] as &[f32]);
        let expansion = curves.get(1).copied().unwrap_or(&[] as &[f32]);
        let decorrel  = curves.get(2).copied().unwrap_or(&[] as &[f32]);

        // ── Probe: populate all 5 declared-curve fields regardless of which
        // channel the DSP path actually consumes them on. Curves 3 (TRANSIENT)
        // and 4 (PAN) are currently STUBS — the DSP does not consume them;
        // the probe reads them for calibration-contract tests only. Future
        // implementation must preserve the declared [0, 100]% range.
        #[cfg(any(test, feature = "probe"))]
        let probe_k = if n == 0 { 0 } else { n / 2 };
        #[cfg(any(test, feature = "probe"))]
        if n > 0 {
            let bal_raw = balance.get(probe_k).copied().unwrap_or(1.0).clamp(0.0, 2.0);
            let exp_raw = expansion.get(probe_k).copied().unwrap_or(1.0).max(0.0);
            let dec_raw = decorrel.get(probe_k).copied().unwrap_or(0.0).clamp(0.0, 2.0);
            let trans_raw = curves.get(3).and_then(|c| c.get(probe_k))
                                  .copied().unwrap_or(1.0).clamp(0.0, 1.0);
            let pan_raw = curves.get(4).and_then(|c| c.get(probe_k))
                                .copied().unwrap_or(1.0).clamp(0.0, 1.0);
            self.last_probe = crate::dsp::modules::ProbeSnapshot {
                balance_pct:   Some(bal_raw * 100.0),
                expansion_pct: Some(exp_raw * 100.0),
                decorrel_pct:  Some(dec_raw * 100.0),
                transient_pct: Some(trans_raw * 100.0),
                pan_pct:       Some(pan_raw * 100.0),
                ..Default::default()
            };
        }

        // Phase 4.3d — PLPV-on iff toggle enabled AND the Pipeline supplied a
        // peak set. Both branches share `self.rng_state`; PRNG sequence
        // intentionally diverges between the two paths because the iteration
        // shape differs (per-peak vs per-bin). Documented in the plan.
        let plpv = if self.plpv_enabled { ctx.peaks } else { None };
        // Empty peak set → fall through to per-bin path so silent/sub-threshold
        // blocks still apply balance/expansion.
        let plpv = plpv.filter(|p| !p.is_empty());

        // Voronoi invariant: skirts must be disjoint and sorted so that the
        // per-peak broadcast never double-applies. Caught here in debug builds
        // at no release cost.
        if let Some(peaks) = plpv {
            debug_assert!(
                peaks.windows(2).all(|w| w[0].high_k < w[1].low_k),
                "MidSide PLPV: peak skirts must be disjoint and sorted (Voronoi invariant)"
            );
        }

        match channel {
            0 => {
                // Mid channel: apply balance (mid scale)
                // balance curve: 1.0 = neutral, 0.0 = full side (mute mid), 2.0 = double mid
                match plpv {
                    Some(peaks) => {
                        // Per-peak broadcast: sample balance at the peak bin
                        // and apply it across the entire skirt.
                        if n == 0 { return; }
                        let last = n - 1;
                        for p in peaks {
                            let pk = (p.k as usize).min(last);
                            let lo = (p.low_k as usize).min(last);
                            let hi = (p.high_k as usize).min(last);
                            let bal = balance.get(pk).copied().unwrap_or(1.0).clamp(0.0, 2.0);
                            let mid_scale = bal.sqrt().min(std::f32::consts::SQRT_2);
                            for k in lo..=hi {
                                bins[k] *= mid_scale;
                            }
                        }
                    }
                    None => {
                        for k in 0..n {
                            let bal = balance.get(k).copied().unwrap_or(1.0).clamp(0.0, 2.0);
                            let mid_scale = bal.sqrt().min(std::f32::consts::SQRT_2);
                            bins[k] *= mid_scale;
                        }
                    }
                }
            }
            1 => {
                // Side channel: balance (side scale) + expansion + decorrelation
                match plpv {
                    Some(peaks) => {
                        // Per-peak broadcast: sample curves at the peak bin,
                        // draw ONE random rotation per peak, and apply to the
                        // entire skirt. Bins in the same peak share the rotation
                        // — this preserves inter-channel phase coherence
                        // relative to the per-bin random rotation in the off path.
                        if n == 0 { return; }
                        let last = n - 1;
                        for p in peaks {
                            let pk = (p.k as usize).min(last);
                            let lo = (p.low_k as usize).min(last);
                            let hi = (p.high_k as usize).min(last);

                            let bal = balance.get(pk).copied().unwrap_or(1.0).clamp(0.0, 2.0);
                            let side_scale = (2.0 - bal).sqrt().min(std::f32::consts::SQRT_2);
                            let exp = expansion.get(pk).copied().unwrap_or(1.0).max(0.0);
                            let dec_amt = decorrel.get(pk).copied().unwrap_or(0.0).clamp(0.0, 2.0);
                            let scale = side_scale * exp;

                            // Draw one rotation per peak when decorrel is active.
                            let phase_rot = if dec_amt > 0.001 {
                                let rnd = xorshift64(&mut self.rng_state) as f32 / u64::MAX as f32;
                                (rnd - 0.5) * 2.0 * std::f32::consts::PI * dec_amt
                            } else {
                                0.0
                            };
                            let (sin_r, cos_r) = phase_rot.sin_cos();

                            for k in lo..=hi {
                                // DC (k=0) and Nyquist (k=last) are required by realfft's
                                // inverse to have zero imaginary part — never rotate.
                                let is_real_bin = k == 0 || k == last;
                                if is_real_bin || phase_rot == 0.0 {
                                    bins[k] *= scale;
                                } else {
                                    let r = bins[k].re;
                                    let i = bins[k].im;
                                    bins[k] = Complex::new(
                                        (r * cos_r - i * sin_r) * scale,
                                        (r * sin_r + i * cos_r) * scale,
                                    );
                                }
                            }
                        }
                    }
                    None => {
                        for k in 0..n {
                            let bal = balance.get(k).copied().unwrap_or(1.0).clamp(0.0, 2.0);
                            let side_scale = (2.0 - bal).sqrt().min(std::f32::consts::SQRT_2);

                            let exp = expansion.get(k).copied().unwrap_or(1.0).max(0.0);

                            let dec_amt = decorrel.get(k).copied().unwrap_or(0.0).clamp(0.0, 2.0);
                            // DC (k=0) and Nyquist (k=n-1) are required by realfft's inverse to
                            // have zero imaginary part — they represent real-valued components
                            // and have no meaningful phase. Rotating them panics the IFFT.
                            let is_real_bin = k == 0 || k == n - 1;
                            let phase_rot = if dec_amt > 0.001 && !is_real_bin {
                                let rnd = xorshift64(&mut self.rng_state) as f32 / u64::MAX as f32;
                                (rnd - 0.5) * 2.0 * std::f32::consts::PI * dec_amt
                            } else {
                                0.0
                            };

                            let (sin_r, cos_r) = phase_rot.sin_cos();
                            let rotated = Complex::new(
                                bins[k].re * cos_r - bins[k].im * sin_r,
                                bins[k].re * sin_r + bins[k].im * cos_r,
                            );
                            bins[k] = rotated * (side_scale * exp);
                        }
                    }
                }
            }
            _ => {} // No more than 2 channels
        }
    }

    fn module_type(&self) -> ModuleType { ModuleType::MidSide }
    fn num_curves(&self) -> usize { 5 }

    #[cfg(any(test, feature = "probe"))]
    fn last_probe(&self) -> crate::dsp::modules::ProbeSnapshot { self.last_probe }
}
