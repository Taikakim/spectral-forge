use num_complex::Complex;
use crate::params::{FxChannelTarget, StereoLink};
use super::{ModuleContext, ModuleType, SpectralModule};

pub struct TsSplitModule {
    avg_mag:       Vec<f32>,
    transient_out: Vec<Complex<f32>>,
    sustained_out: Vec<Complex<f32>>,
    fft_size:      usize,
    #[cfg(any(test, feature = "probe"))]
    last_probe: crate::dsp::modules::ProbeSnapshot,
}

impl TsSplitModule {
    pub fn new() -> Self {
        Self {
            avg_mag:       Vec::new(),
            transient_out: Vec::new(),
            sustained_out: Vec::new(),
            fft_size:      2048,
            #[cfg(any(test, feature = "probe"))]
            last_probe: Default::default(),
        }
    }

    pub fn transient_bins(&self) -> &[Complex<f32>] { &self.transient_out }
    pub fn sustained_bins(&self) -> &[Complex<f32>] { &self.sustained_out }
}

impl Default for TsSplitModule {
    fn default() -> Self { Self::new() }
}

impl SpectralModule for TsSplitModule {
    fn reset(&mut self, _sample_rate: f32, fft_size: usize) {
        self.fft_size = fft_size;
        let n = fft_size / 2 + 1;
        self.avg_mag       = vec![0.0f32;                n];
        self.transient_out = vec![Complex::new(0.0, 0.0); n];
        self.sustained_out = vec![Complex::new(0.0, 0.0); n];
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
        _ctx: &ModuleContext<'_>,
    ) {
        let n = bins.len();
        let slow_coeff: f32 = 0.98;
        #[cfg(any(test, feature = "probe"))]
        let probe_k = if n == 0 { 0 } else { n / 2 };
        #[cfg(any(test, feature = "probe"))]
        let mut probe_sensitivity_pct: Option<f32> = None;
        for k in 0..n {
            let mag = bins[k].norm();
            self.avg_mag[k] = slow_coeff * self.avg_mag[k] + (1.0 - slow_coeff) * mag;
            let sensitivity = curves.get(0).and_then(|c| c.get(k))
                                     .copied().unwrap_or(1.0).clamp(0.0, 2.0);
            #[cfg(any(test, feature = "probe"))]
            if k == probe_k {
                probe_sensitivity_pct = Some(sensitivity * 100.0);
            }
            if mag > self.avg_mag[k] * (1.0 + sensitivity) {
                self.transient_out[k] = bins[k];
                self.sustained_out[k] = Complex::new(0.0, 0.0);
            } else {
                self.transient_out[k] = Complex::new(0.0, 0.0);
                self.sustained_out[k] = bins[k];
            }
        }
        #[cfg(any(test, feature = "probe"))]
        {
            self.last_probe = crate::dsp::modules::ProbeSnapshot {
                sensitivity_pct: probe_sensitivity_pct,
                ..Default::default()
            };
        }
        suppression_out.fill(0.0);
    }

    fn clear_state(&mut self) {
        self.avg_mag.fill(0.0);
        for b in self.transient_out.iter_mut() { *b = num_complex::Complex::new(0.0, 0.0); }
        for b in self.sustained_out.iter_mut()  { *b = num_complex::Complex::new(0.0, 0.0); }
    }

    fn tail_length(&self) -> u32 { self.fft_size as u32 }
    fn module_type(&self) -> ModuleType { ModuleType::TransientSustainedSplit }
    fn num_curves(&self) -> usize { 1 }
    fn num_outputs(&self) -> Option<usize> { Some(2) }

    fn virtual_outputs(&self) -> Option<[&[Complex<f32>]; 2]> {
        Some([&self.transient_out, &self.sustained_out])
    }

    #[cfg(any(test, feature = "probe"))]
    fn last_probe(&self) -> crate::dsp::modules::ProbeSnapshot { self.last_probe }
}
