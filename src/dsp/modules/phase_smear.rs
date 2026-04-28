use num_complex::Complex;
use crate::params::{FxChannelTarget, StereoLink};
use crate::dsp::utils::xorshift64;
use crate::dsp::pipeline::{MAX_NUM_BINS, OVERLAP};
use super::{ModuleContext, ModuleType, SpectralModule};

pub struct PhaseSmearModule {
    rng_state: u64,
    peak_env: Vec<f32>,
    sample_rate: f32,
    fft_size: usize,
    /// Phase 4.3b — per-module PLPV unwrapped-phase randomization enable. Mirrors
    /// `params.plpv_phase_smear_enable`; written each audio block by the Pipeline
    /// via `FxMatrix::set_plpv_phase_smear_enable`. Default `true` matches the
    /// param default so a freshly-constructed module behaves identically to one
    /// with the param applied.
    plpv_enabled: bool,
    #[cfg(any(test, feature = "probe"))]
    last_probe: crate::dsp::modules::ProbeSnapshot,
}

impl PhaseSmearModule {
    pub fn new() -> Self {
        Self {
            rng_state: 0x123456789abcdef0,
            peak_env: vec![0.0f32; MAX_NUM_BINS],
            sample_rate: 44100.0,
            fft_size: 2048,
            plpv_enabled: true,
            #[cfg(any(test, feature = "probe"))]
            last_probe: Default::default(),
        }
    }

}

impl Default for PhaseSmearModule {
    fn default() -> Self { Self::new() }
}

impl SpectralModule for PhaseSmearModule {
    fn reset(&mut self, sample_rate: f32, fft_size: usize) {
        self.sample_rate = sample_rate;
        self.fft_size = fft_size;
        for v in &mut self.peak_env { *v = 0.0; }
    }

    fn clear_state(&mut self) {
        self.peak_env.fill(0.0);
    }

    fn process(
        &mut self,
        _channel: usize,
        _stereo_link: StereoLink,
        _target: FxChannelTarget,
        bins: &mut [Complex<f32>],
        sidechain: Option<&[f32]>,
        curves: &[&[f32]],
        suppression_out: &mut [f32],
        _physics: Option<&mut crate::dsp::bin_physics::BinPhysics>,
        ctx: &ModuleContext<'_>,
    ) {
        if bins.is_empty() { suppression_out.fill(0.0); return; }
        let last = bins.len() - 1;
        let hop_ms = self.fft_size as f32 / (OVERLAP as f32 * self.sample_rate) * 1000.0;
        #[cfg(any(test, feature = "probe"))]
        let probe_k = bins.len() / 2;

        #[cfg(any(test, feature = "probe"))]
        let mut probe_amount_pct:  f32 = 0.0;
        #[cfg(any(test, feature = "probe"))]
        let mut probe_peak_hold_ms: f32 = 0.0;
        #[cfg(any(test, feature = "probe"))]
        let mut probe_mix_pct:     f32 = 0.0;

        // Phase 4.3b — PLPV branch: when the unwrapped-phase trajectory is exposed
        // by the Pipeline AND this module's PLPV flag is on, write the random offset
        // directly to the unwrapped phase. The Pipeline's re-wrap stage afterwards
        // recomputes bins[k] = polar(|bins[k]|, principal_arg(unwrapped[k])), so we
        // MUST NOT also write to bins[k] here — that write would be discarded.
        //
        // The mix on this path is implemented as a phase-space lerp:
        //   wet_phase = unwrapped[k] + rand_phase
        //   out_phase = (1 - mix)·unwrapped[k] + mix·wet_phase = unwrapped[k] + mix·rand_phase
        // This differs from the non-PLPV path's complex-space mix (which interpolates
        // (re, im)). The phase-space lerp keeps magnitude untouched and stays inside
        // the unwrapped trajectory so future hops continue from the modified phase.
        if let (Some(unwrapped), true) = (ctx.unwrapped_phase, self.plpv_enabled) {
            for k in 0..bins.len() {
                // Always advance PRNG to keep the sequence independent of skipping.
                let rand = xorshift64(&mut self.rng_state);
                // DC (k=0) and Nyquist (k=last) must stay real for IFFT correctness.
                if k == 0 || k == last { continue; }

                let sc_raw = sidechain.and_then(|s| s.get(k)).copied().unwrap_or(0.0).max(0.0);
                let hold_c = curves.get(1).and_then(|c| c.get(k)).copied().unwrap_or(1.0);
                let hold_ms = super::peak_hold_curve_to_ms(hold_c);
                let rel = (-hop_ms / hold_ms.max(0.1)).exp();
                if sc_raw > self.peak_env[k] {
                    self.peak_env[k] = sc_raw;
                } else {
                    self.peak_env[k] = rel * self.peak_env[k] + (1.0 - rel) * sc_raw;
                }
                let sc_mod = self.peak_env[k].min(1.0);

                let amount_curve = curves.get(0).and_then(|c| c.get(k))
                                   .copied().unwrap_or(1.0).clamp(0.0, 2.0);
                let per_bin = (amount_curve * (1.0 + sc_mod)).clamp(0.0, 2.0);

                let scale      = per_bin * std::f32::consts::PI;
                let rand_phase = (rand as f32 / u64::MAX as f32 * 2.0 - 1.0) * scale;
                let mix = curves.get(2).and_then(|c| c.get(k))
                                .copied().unwrap_or(1.0).clamp(0.0, 1.0);

                // Phase-space lerp; see comment above.
                let prev = unwrapped[k].get();
                unwrapped[k].set(prev + rand_phase * mix);

                #[cfg(any(test, feature = "probe"))]
                if k == probe_k {
                    probe_amount_pct   = amount_curve * 100.0;
                    probe_peak_hold_ms = hold_ms;
                    probe_mix_pct      = mix * 100.0;
                }
            }
        } else {
            // Non-PLPV path — wrapped-phase + complex-space mix. Unchanged from pre-4.3b.
            for k in 0..bins.len() {
                let dry = bins[k];
                // Always advance PRNG to keep the sequence independent of skipping.
                let rand = xorshift64(&mut self.rng_state);
                // DC (k=0) and Nyquist (k=last) must stay real for IFFT correctness.
                if k == 0 || k == last { continue; }

                let sc_raw = sidechain.and_then(|s| s.get(k)).copied().unwrap_or(0.0).max(0.0);
                let hold_c = curves.get(1).and_then(|c| c.get(k)).copied().unwrap_or(1.0);
                let hold_ms = super::peak_hold_curve_to_ms(hold_c);
                let rel = (-hop_ms / hold_ms.max(0.1)).exp();
                if sc_raw > self.peak_env[k] {
                    self.peak_env[k] = sc_raw;
                } else {
                    self.peak_env[k] = rel * self.peak_env[k] + (1.0 - rel) * sc_raw;
                }
                let sc_mod = self.peak_env[k].min(1.0);

                let amount_curve = curves.get(0).and_then(|c| c.get(k))
                                   .copied().unwrap_or(1.0).clamp(0.0, 2.0);
                let per_bin = (amount_curve * (1.0 + sc_mod)).clamp(0.0, 2.0);

                let scale      = per_bin * std::f32::consts::PI;
                let rand_phase = (rand as f32 / u64::MAX as f32 * 2.0 - 1.0) * scale;
                let (mag, phase) = (bins[k].norm(), bins[k].arg());
                let wet = Complex::from_polar(mag, phase + rand_phase);
                let mix = curves.get(2).and_then(|c| c.get(k)).copied().unwrap_or(1.0).clamp(0.0, 1.0);
                bins[k] = Complex::new(
                    dry.re * (1.0 - mix) + wet.re * mix,
                    dry.im * (1.0 - mix) + wet.im * mix,
                );

                #[cfg(any(test, feature = "probe"))]
                if k == probe_k {
                    probe_amount_pct   = amount_curve * 100.0;
                    probe_peak_hold_ms = hold_ms;
                    probe_mix_pct      = mix * 100.0;
                }
            }
        }

        #[cfg(any(test, feature = "probe"))]
        {
            self.last_probe = crate::dsp::modules::ProbeSnapshot {
                amount_pct:    Some(probe_amount_pct),
                peak_hold_ms:  Some(probe_peak_hold_ms),
                mix_pct:       Some(probe_mix_pct),
                ..Default::default()
            };
        }

        suppression_out.fill(0.0);
    }

    fn module_type(&self) -> ModuleType { ModuleType::PhaseSmear }
    fn num_curves(&self) -> usize { 3 }

    /// Phase 4.3b — propagated each block by `FxMatrix::set_plpv_phase_smear_enable`.
    fn set_plpv_phase_smear_enabled(&mut self, enabled: bool) {
        self.plpv_enabled = enabled;
    }

    #[cfg(any(test, feature = "probe"))]
    fn last_probe(&self) -> crate::dsp::modules::ProbeSnapshot { self.last_probe }
}
