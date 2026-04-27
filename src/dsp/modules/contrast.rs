use num_complex::Complex;
use crate::dsp::engines::{BinParams, SpectralEngine, create_engine, EngineSelection};
use crate::params::{FxChannelTarget, StereoLink};
use super::{ModuleContext, ModuleType, SpectralModule};

pub struct ContrastModule {
    engine:       Box<dyn SpectralEngine>,
    bp_threshold: Vec<f32>,
    bp_ratio:     Vec<f32>,
    bp_attack:    Vec<f32>,
    bp_release:   Vec<f32>,
    bp_knee:      Vec<f32>,
    bp_makeup:    Vec<f32>,
    bp_mix:       Vec<f32>,
    num_bins:     usize,
    sample_rate:  f32,
    #[cfg(any(test, feature = "probe"))]
    last_probe: crate::dsp::modules::ProbeSnapshot,
}

impl ContrastModule {
    pub fn new() -> Self {
        Self {
            engine:       create_engine(EngineSelection::SpectralContrast),
            bp_threshold: Vec::new(),
            bp_ratio:     Vec::new(),
            bp_attack:    Vec::new(),
            bp_release:   Vec::new(),
            bp_knee:      Vec::new(),
            bp_makeup:    Vec::new(),
            bp_mix:       Vec::new(),
            num_bins:     0,
            sample_rate:  44100.0,
            #[cfg(any(test, feature = "probe"))]
            last_probe: Default::default(),
        }
    }
}

impl Default for ContrastModule {
    fn default() -> Self { Self::new() }
}

impl SpectralModule for ContrastModule {
    fn reset(&mut self, sample_rate: f32, fft_size: usize) {
        self.sample_rate = sample_rate;
        self.num_bins    = fft_size / 2 + 1;
        self.engine.reset(sample_rate, fft_size);
        let n = self.num_bins;
        self.bp_threshold = vec![-20.0f32; n];
        self.bp_ratio     = vec![2.0f32;   n];
        self.bp_attack    = vec![10.0f32;  n];
        self.bp_release   = vec![100.0f32; n];
        self.bp_knee      = vec![6.0f32;   n];
        self.bp_makeup    = vec![0.0f32;   n];
        self.bp_mix       = vec![1.0f32;   n];
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
        ctx: &ModuleContext<'_>,
    ) {
        let n = self.num_bins;
        for k in 0..n {
            let amount = curves.get(0).and_then(|c| c.get(k)).copied().unwrap_or(1.0);
            self.bp_ratio[k]     = amount.max(1.0).min(20.0);
            self.bp_threshold[k] = -20.0;
            self.bp_attack[k]    = ctx.attack_ms.clamp(0.1, 500.0);
            self.bp_release[k]   = ctx.release_ms.clamp(1.0, 2000.0);
            self.bp_knee[k]      = 6.0;
            self.bp_mix[k]       = 1.0;
        }
        let params = BinParams {
            threshold_db:        &self.bp_threshold,
            ratio:               &self.bp_ratio,
            attack_ms:           &self.bp_attack,
            release_ms:          &self.bp_release,
            knee_db:             &self.bp_knee,
            makeup_db:           &self.bp_makeup,
            mix:                 &self.bp_mix,
            sensitivity:         ctx.sensitivity,
            auto_makeup:         false,
            smoothing_semitones: ctx.suppression_width,
        };
        self.engine.process_bins(bins, sidechain, &params, self.sample_rate, suppression_out);

        #[cfg(any(test, feature = "probe"))]
        {
            let k = self.num_bins / 2;
            self.last_probe = crate::dsp::modules::ProbeSnapshot {
                ratio: Some(self.bp_ratio[k]),
                ..Default::default()
            };
        }
    }

    fn clear_state(&mut self) {
        self.engine.clear_state();
        self.bp_threshold.fill(-20.0);
        self.bp_ratio.fill(2.0);
        self.bp_attack.fill(10.0);
        self.bp_release.fill(100.0);
        self.bp_knee.fill(6.0);
        self.bp_makeup.fill(0.0);
        self.bp_mix.fill(1.0);
    }

    fn module_type(&self) -> ModuleType { ModuleType::Contrast }
    fn num_curves(&self) -> usize { 1 }

    #[cfg(any(test, feature = "probe"))]
    fn last_probe(&self) -> crate::dsp::modules::ProbeSnapshot { self.last_probe }
}
