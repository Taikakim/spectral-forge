use num_complex::Complex;
use crate::params::{FxChannelTarget, StereoLink};
use crate::dsp::utils::xorshift64;
use super::{ModuleContext, ModuleType, SpectralModule};

/// Onset ratio (mid magnitude / slow average) at which a bin counts as fully transient.
const TRANSIENT_FULL_RATIO: f32 = 3.0;
/// Slow-average coefficient for the transient detector (matches T/S Split's neutral).
const TRANSIENT_SLOW_COEFF: f32 = 0.98;
/// Per-hop decay of the transient envelope, so the side gain doesn't flutter.
const TRANSIENT_ENV_DECAY: f32 = 0.7;

/// Bipolar curve value: gain 1.0 → 0 (neutral), 0 → −1, 2 → +1.
#[inline]
fn bipolar(g: f32) -> f32 { (g - 1.0).clamp(-1.0, 1.0) }

pub struct MidSideModule {
    /// xorshift64 state for phase decorrelation. Must never be zero.
    rng_state: u64,
    num_bins:  usize,
    /// Mid bins after this slot's balance stage, captured on channel 0 so PAN
    /// can mix them into the side on channel 1 of the same hop.
    mid_out: Vec<Complex<f32>>,
    /// Slow per-bin average of mid magnitude (transient detector).
    mid_avg: Vec<f32>,
    /// Per-bin transient envelope in [0, 1], updated on channel 0.
    transient_env: Vec<f32>,
    /// Set on channel 0, cleared on channel 1: the snapshot belongs to this hop.
    mid_valid: bool,
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
            mid_out: Vec::new(),
            mid_avg: Vec::new(),
            transient_env: Vec::new(),
            mid_valid: false,
            plpv_enabled: true,
            #[cfg(any(test, feature = "probe"))]
            last_probe: Default::default(),
        }
    }

    /// Update the per-bin transient envelope from the incoming mid. A bin is
    /// fully transient when its magnitude reaches `TRANSIENT_FULL_RATIO` × its
    /// slow average; the envelope then decays by `TRANSIENT_ENV_DECAY` per hop.
    fn update_transient_env(&mut self, bins: &[Complex<f32>]) {
        let n = bins.len().min(self.mid_avg.len());
        for k in 0..n {
            let mag = bins[k].norm();
            let avg = self.mid_avg[k];
            let onset = if avg > 1e-12 {
                ((mag / avg - 1.0) / (TRANSIENT_FULL_RATIO - 1.0)).clamp(0.0, 1.0)
            } else {
                0.0
            };
            self.transient_env[k] = onset.max(self.transient_env[k] * TRANSIENT_ENV_DECAY);
            self.mid_avg[k] = TRANSIENT_SLOW_COEFF * avg + (1.0 - TRANSIENT_SLOW_COEFF) * mag;
        }
    }

    /// Side-channel TRANSIENT and PAN. TRANSIENT scales the side by
    /// `1 + t·env` (t ∈ [−1, 1]: −1 mutes the side on onsets → mono, +1 doubles
    /// it). PAN completes the constant-power law started on channel 0:
    /// S' = S − sin(p·π/4)·M, which with M' = cos(p·π/4)·M gives a centred mono
    /// source L ∝ cos−sin, R ∝ cos+sin (p = +1 → hard right).
    fn apply_transient_and_pan(&self, bins: &mut [Complex<f32>], transient: &[f32], pan: &[f32]) {
        let n = bins.len().min(self.mid_out.len());
        for k in 0..n {
            let t = bipolar(transient.get(k).copied().unwrap_or(1.0));
            if t != 0.0 {
                bins[k] *= 1.0 + t * self.transient_env[k];
            }
            let p = bipolar(pan.get(k).copied().unwrap_or(1.0));
            if p != 0.0 {
                bins[k] -= self.mid_out[k] * (p * std::f32::consts::FRAC_PI_4).sin();
            }
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
        self.mid_out.clear();
        self.mid_out.resize(self.num_bins, Complex::new(0.0, 0.0));
        self.mid_avg.clear();
        self.mid_avg.resize(self.num_bins, 0.0);
        self.transient_env.clear();
        self.transient_env.resize(self.num_bins, 0.0);
        self.mid_valid = false;
    }

    fn clear_state(&mut self) {
        self.mid_out.fill(Complex::new(0.0, 0.0));
        self.mid_avg.fill(0.0);
        self.transient_env.fill(0.0);
        self.mid_valid = false;
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
        // 0 = BALANCE, 1 = EXPANSION, 2 = DECORREL, 3 = TRANSIENT, 4 = PAN
        // TRANSIENT and PAN are bipolar: gain 1.0 = neutral (see `bipolar`).
        let balance   = curves.get(0).copied().unwrap_or(&[] as &[f32]);
        let expansion = curves.get(1).copied().unwrap_or(&[] as &[f32]);
        let decorrel  = curves.get(2).copied().unwrap_or(&[] as &[f32]);
        let transient = curves.get(3).copied().unwrap_or(&[] as &[f32]);
        let pan       = curves.get(4).copied().unwrap_or(&[] as &[f32]);

        // ── Probe: populate all 5 declared-curve fields regardless of which
        // channel the DSP path actually consumes them on. TRANSIENT and PAN
        // report the bipolar value as −100 … +100 %.
        #[cfg(any(test, feature = "probe"))]
        let probe_k = if n == 0 { 0 } else { n / 2 };
        #[cfg(any(test, feature = "probe"))]
        if n > 0 {
            let bal_raw = balance.get(probe_k).copied().unwrap_or(1.0).clamp(0.0, 2.0);
            let exp_raw = expansion.get(probe_k).copied().unwrap_or(1.0).max(0.0);
            let dec_raw = decorrel.get(probe_k).copied().unwrap_or(0.0).clamp(0.0, 2.0);
            let trans_raw = bipolar(transient.get(probe_k).copied().unwrap_or(1.0));
            let pan_raw   = bipolar(pan.get(probe_k).copied().unwrap_or(1.0));
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
                // Transient detector runs on the incoming mid (pre-balance).
                self.update_transient_env(bins);

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

                // Snapshot the balanced mid for channel 1's PAN, then apply the
                // mid half of the constant-power pan law.
                let m = n.min(self.mid_out.len());
                self.mid_out[..m].copy_from_slice(&bins[..m]);
                self.mid_valid = m == n;
                for k in 0..m {
                    let p = bipolar(pan.get(k).copied().unwrap_or(1.0));
                    if p != 0.0 {
                        bins[k] *= (p * std::f32::consts::FRAC_PI_4).cos();
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

                // TRANSIENT + side half of PAN need this hop's mid from channel 0.
                if self.mid_valid {
                    self.apply_transient_and_pan(bins, transient, pan);
                }
                self.mid_valid = false;
            }
            _ => {} // No more than 2 channels
        }
    }

    fn module_type(&self) -> ModuleType { ModuleType::MidSide }
    fn num_curves(&self) -> usize { 5 }

    #[cfg(any(test, feature = "probe"))]
    fn last_probe(&self) -> crate::dsp::modules::ProbeSnapshot { self.last_probe }
}
