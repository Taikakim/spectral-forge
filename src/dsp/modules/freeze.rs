use num_complex::Complex;
use crate::dsp::utils::linear_to_db;
use crate::params::{FxChannelTarget, StereoLink};
use super::{ModuleContext, ModuleType, SpectralModule};

pub struct FreezeModule {
    frozen_bins:      Vec<Complex<f32>>,
    freeze_target:    Vec<Complex<f32>>,
    freeze_port_t:    Vec<f32>,
    freeze_hold_hops: Vec<u32>,
    freeze_accum:     Vec<f32>,
    freeze_captured:  bool,
    fft_size:         usize,
    sample_rate:      f32,
}

impl FreezeModule {
    pub fn new() -> Self {
        Self {
            frozen_bins:      Vec::new(),
            freeze_target:    Vec::new(),
            freeze_port_t:    Vec::new(),
            freeze_hold_hops: Vec::new(),
            freeze_accum:     Vec::new(),
            freeze_captured:  false,
            fft_size:         2048,
            sample_rate:      44100.0,
        }
    }
}

impl Default for FreezeModule {
    fn default() -> Self { Self::new() }
}

impl SpectralModule for FreezeModule {
    fn reset(&mut self, sample_rate: f32, fft_size: usize) {
        self.sample_rate    = sample_rate;
        self.fft_size       = fft_size;
        let n               = fft_size / 2 + 1;
        self.frozen_bins      = vec![Complex::new(0.0, 0.0); n];
        self.freeze_target    = vec![Complex::new(0.0, 0.0); n];
        self.freeze_port_t    = vec![1.0f32; n];
        self.freeze_hold_hops = vec![0u32; n];
        self.freeze_accum     = vec![0.0f32; n];
        self.freeze_captured  = false;
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
        ctx: &ModuleContext,
    ) {
        debug_assert_eq!(bins.len(), self.frozen_bins.len(),
            "FreezeModule: bins/buffer size mismatch — call reset() before process()");

        use crate::dsp::pipeline::OVERLAP;
        let hop_ms = ctx.fft_size as f32 / (OVERLAP as f32 * self.sample_rate) * 1000.0;

        if !self.freeze_captured {
            // First call: capture current frame as initial frozen state.
            self.frozen_bins.copy_from_slice(bins);
            self.freeze_target.copy_from_slice(bins);
            self.freeze_port_t.fill(1.0);
            self.freeze_hold_hops.fill(0);
            self.freeze_accum.fill(0.0);
            self.freeze_captured = true;
        }

        let n = bins.len();
        for k in 0..n {
            let dry = bins[k];

            // Map per-bin curve gains to physical parameter values.
            let length_ms   = (curves.get(0).and_then(|c| c.get(k)).copied().unwrap_or(1.0)
                               * 500.0).clamp(0.0, 2000.0);
            let length_hops = (length_ms / hop_ms).ceil() as u32;

            let thr_gain      = curves.get(1).and_then(|c| c.get(k)).copied().unwrap_or(1.0);
            let thr_db        = linear_to_db(thr_gain);
            let threshold_db  = (-20.0 + thr_db * (60.0 / 18.0)).clamp(-80.0, 0.0);
            let threshold_lin = 10.0f32.powf(threshold_db / 20.0);

            let port_ms   = (curves.get(2).and_then(|c| c.get(k)).copied().unwrap_or(1.0)
                             * 100.0).clamp(0.0, 1000.0);
            let port_hops = (port_ms / hop_ms).max(0.5);

            let resistance = (curves.get(3).and_then(|c| c.get(k)).copied().unwrap_or(1.0)
                              * 1.0).clamp(0.0, 5.0);

            if self.freeze_port_t[k] < 1.0 {
                // Portamento in progress: advance and interpolate.
                self.freeze_port_t[k] = (self.freeze_port_t[k] + 1.0 / port_hops).min(1.0);
                let t = self.freeze_port_t[k];
                self.frozen_bins[k] = Complex::new(
                    self.frozen_bins[k].re * (1.0 - t) + self.freeze_target[k].re * t,
                    self.frozen_bins[k].im * (1.0 - t) + self.freeze_target[k].im * t,
                );
            } else {
                // Settled: hold and accumulate energy toward next transition.
                self.freeze_hold_hops[k] += 1;
                let mag = bins[k].norm();
                if mag > threshold_lin {
                    self.freeze_accum[k] += mag - threshold_lin;
                }
                // Trigger state change when hold duration and resistance both met.
                if self.freeze_hold_hops[k] >= length_hops && self.freeze_accum[k] >= resistance {
                    self.freeze_target[k]    = bins[k];
                    self.freeze_port_t[k]    = 0.0;
                    self.freeze_hold_hops[k] = 0;
                    self.freeze_accum[k]     = 0.0;
                }
            }

            let wet = self.frozen_bins[k];
            let mix = curves.get(4).and_then(|c| c.get(k)).copied().unwrap_or(1.0).clamp(0.0, 1.0);
            bins[k] = Complex::new(
                dry.re * (1.0 - mix) + wet.re * mix,
                dry.im * (1.0 - mix) + wet.im * mix,
            );
        }
        suppression_out.fill(0.0);
    }

    fn tail_length(&self) -> u32 { self.fft_size as u32 }
    fn module_type(&self) -> ModuleType { ModuleType::Freeze }
    fn num_curves(&self) -> usize { 5 }
}
