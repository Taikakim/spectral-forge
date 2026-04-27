use num_complex::Complex;
use crate::params::{FxChannelTarget, StereoLink};
use super::{ModuleContext, ModuleType, SpectralModule};

/// Map a per-bin threshold curve gain (linear, 1.0 = neutral) to dBFS threshold.
/// Linear in gain space: gain=1.0 → -20 dBFS (neutral); gain=2.0 → 0 dBFS (y_max);
/// gain≤-2.0 → -80 dBFS (y_min, clamped). Matches off_freeze_thresh calibration.
pub fn curve_to_threshold_db(curve_gain: f32) -> f32 {
    (-40.0 + curve_gain * 20.0).clamp(-80.0, 0.0)
}

pub struct FreezeModule {
    frozen_bins:      Vec<Complex<f32>>,
    freeze_target:    Vec<Complex<f32>>,
    freeze_port_t:    Vec<f32>,
    freeze_hold_hops: Vec<u32>,
    freeze_accum:     Vec<f32>,
    freeze_captured:  bool,
    fft_size:         usize,
    sample_rate:      f32,
    #[cfg(any(test, feature = "probe"))]
    last_probe: crate::dsp::modules::ProbeSnapshot,
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
            #[cfg(any(test, feature = "probe"))]
            last_probe: Default::default(),
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
        _physics: Option<&mut crate::dsp::bin_physics::BinPhysics>,
        ctx: &ModuleContext<'_>,
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

        // Calibrate threshold to raw FFT bin magnitudes.
        // Raw bins for a 0 dBFS sine ≈ fft_size/4, matching the Dynamics compressor convention.
        let norm_factor = ctx.fft_size as f32 / 4.0;

        let n = bins.len();

        #[cfg(any(test, feature = "probe"))]
        let mut probe_length_ms:     f32 = 0.0;
        #[cfg(any(test, feature = "probe"))]
        let mut probe_threshold_db:  f32 = 0.0;
        #[cfg(any(test, feature = "probe"))]
        let mut probe_portamento_ms: f32 = 0.0;
        #[cfg(any(test, feature = "probe"))]
        let mut probe_resistance:    f32 = 0.0;
        #[cfg(any(test, feature = "probe"))]
        let mut probe_mix_pct:       f32 = 0.0;

        for k in 0..n {
            let dry = bins[k];

            // Map per-bin curve gains to physical parameter values.
            let length_ms   = (curves.get(0).and_then(|c| c.get(k)).copied().unwrap_or(1.0)
                               * 500.0).clamp(0.0, 4000.0);
            let length_hops = ((length_ms / hop_ms).ceil() as u32).max(1);

            let thr_gain      = curves.get(1).and_then(|c| c.get(k)).copied().unwrap_or(1.0);
            let threshold_db  = curve_to_threshold_db(thr_gain);
            // Multiply by norm_factor so threshold_lin is on the same scale as bins[k].norm().
            let threshold_lin = 10.0f32.powf(threshold_db / 20.0) * norm_factor;

            let port_ms   = (curves.get(2).and_then(|c| c.get(k)).copied().unwrap_or(1.0)
                             * 200.0).clamp(0.0, 1000.0);
            let port_hops = (port_ms / hop_ms).max(0.5);

            // Resistance is a dimensionless relative-excess threshold (0–2).
            // Accumulation uses normalised excess (mag/threshold − 1), so the value
            // is independent of signal level and FFT size.
            let resistance = curves.get(3).and_then(|c| c.get(k)).copied().unwrap_or(1.0)
                             .clamp(0.0, 2.0);

            #[cfg(any(test, feature = "probe"))]
            if k == n / 2 {
                probe_length_ms     = length_ms;
                probe_threshold_db  = threshold_db;
                probe_portamento_ms = port_ms;
                probe_resistance    = resistance;
            }

            if self.freeze_port_t[k] < 1.0 {
                // Portamento in progress: advance and interpolate.
                self.freeze_port_t[k] = (self.freeze_port_t[k] + 1.0 / port_hops).min(1.0);
                let t = self.freeze_port_t[k];
                self.frozen_bins[k] = Complex::new(
                    self.frozen_bins[k].re * (1.0 - t) + self.freeze_target[k].re * t,
                    self.frozen_bins[k].im * (1.0 - t) + self.freeze_target[k].im * t,
                );
            } else {
                // Settled: hold and accumulate normalised excess energy toward next transition.
                self.freeze_hold_hops[k] += 1;
                let mag = bins[k].norm();
                if mag > threshold_lin && threshold_lin > 0.0 {
                    // Relative excess: 0 at threshold, 1 when mag = 2× threshold (+6 dB)
                    self.freeze_accum[k] += mag / threshold_lin - 1.0;
                }
                // Trigger when hold duration AND accumulated excess both met.
                if self.freeze_hold_hops[k] >= length_hops && self.freeze_accum[k] >= resistance {
                    self.freeze_target[k]    = bins[k];
                    self.freeze_port_t[k]    = 0.0;
                    self.freeze_hold_hops[k] = 0;
                    self.freeze_accum[k]     = 0.0;
                }
            }

            let wet = self.frozen_bins[k];
            let mix = curves.get(4).and_then(|c| c.get(k)).copied().unwrap_or(1.0).clamp(0.0, 1.0);

            #[cfg(any(test, feature = "probe"))]
            if k == n / 2 {
                probe_mix_pct = mix * 100.0;
            }

            bins[k] = Complex::new(
                dry.re * (1.0 - mix) + wet.re * mix,
                dry.im * (1.0 - mix) + wet.im * mix,
            );
        }

        #[cfg(any(test, feature = "probe"))]
        {
            self.last_probe = crate::dsp::modules::ProbeSnapshot {
                length_ms:     Some(probe_length_ms),
                threshold_db:  Some(probe_threshold_db),
                portamento_ms: Some(probe_portamento_ms),
                resistance:    Some(probe_resistance),
                mix_pct:       Some(probe_mix_pct),
                ..Default::default()
            };
        }

        suppression_out.fill(0.0);
    }

    fn clear_state(&mut self) {
        // Zero all captured/interpolation state and release any frozen snapshot.
        // The next process() call will re-capture from live audio, as if the module
        // was freshly instantiated.
        for b in self.frozen_bins.iter_mut()  { *b = num_complex::Complex::new(0.0, 0.0); }
        for b in self.freeze_target.iter_mut() { *b = num_complex::Complex::new(0.0, 0.0); }
        self.freeze_port_t.fill(1.0);
        self.freeze_hold_hops.fill(0);
        self.freeze_accum.fill(0.0);
        self.freeze_captured = false;
    }

    fn tail_length(&self) -> u32 { self.fft_size as u32 }
    fn module_type(&self) -> ModuleType { ModuleType::Freeze }
    fn num_curves(&self) -> usize { 5 }

    #[cfg(any(test, feature = "probe"))]
    fn last_probe(&self) -> crate::dsp::modules::ProbeSnapshot { self.last_probe }
}
