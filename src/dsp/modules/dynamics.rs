use num_complex::Complex;
use crate::dsp::engines::{BinParams, SpectralEngine, create_engine, EngineSelection};
use crate::dsp::utils::linear_to_db;
use crate::params::{FxChannelTarget, StereoLink};
use super::{ModuleContext, ModuleType, SpectralModule};

pub struct DynamicsModule {
    engine:       Box<dyn SpectralEngine>,
    engine_r:     Box<dyn SpectralEngine>,
    bp_threshold: Vec<f32>,
    bp_ratio:     Vec<f32>,
    bp_attack:    Vec<f32>,
    bp_release:   Vec<f32>,
    bp_knee:      Vec<f32>,
    bp_makeup:    Vec<f32>,  // always 0.0 — makeup is the Gain module's job
    bp_mix:       Vec<f32>,
    num_bins:     usize,
    sample_rate:  f32,
    #[cfg(any(test, feature = "probe"))]
    last_probe: crate::dsp::modules::ProbeSnapshot,
}

impl DynamicsModule {
    pub fn new() -> Self {
        Self {
            engine:       create_engine(EngineSelection::SpectralCompressor),
            engine_r:     create_engine(EngineSelection::SpectralCompressor),
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

impl Default for DynamicsModule {
    fn default() -> Self { Self::new() }
}

impl SpectralModule for DynamicsModule {
    fn reset(&mut self, sample_rate: f32, fft_size: usize) {
        self.sample_rate = sample_rate;
        self.num_bins    = fft_size / 2 + 1;
        self.engine.reset(sample_rate, fft_size);
        self.engine_r.reset(sample_rate, fft_size);
        let n = self.num_bins;
        self.bp_threshold = vec![-20.0f32; n];
        self.bp_ratio     = vec![1.0f32;   n];
        self.bp_attack    = vec![10.0f32;  n];
        self.bp_release   = vec![100.0f32; n];
        self.bp_knee      = vec![6.0f32;   n];
        self.bp_makeup    = vec![0.0f32;   n];
        self.bp_mix       = vec![1.0f32;   n];
    }

    fn process(
        &mut self,
        channel: usize,
        stereo_link: StereoLink,
        target: FxChannelTarget,
        bins: &mut [Complex<f32>],
        sidechain: Option<&[f32]>,
        curves: &[&[f32]],
        suppression_out: &mut [f32],
        ctx: &ModuleContext<'_>,
    ) {
        // Channel gating: skip if this slot's target doesn't match channel/mode.
        let skip = match (target, stereo_link, channel) {
            (FxChannelTarget::Mid,  StereoLink::MidSide,     1) => true,
            (FxChannelTarget::Side, StereoLink::MidSide,     0) => true,
            (FxChannelTarget::Mid,  StereoLink::Linked,      _) => true,
            (FxChannelTarget::Mid,  StereoLink::Independent, _) => true,
            (FxChannelTarget::Side, StereoLink::Linked,      _) => true,
            (FxChannelTarget::Side, StereoLink::Independent, _) => true,
            _ => false,
        };
        if skip {
            suppression_out.fill(0.0);
            return;
        }

        debug_assert_eq!(bins.len(), self.num_bins,
            "DynamicsModule: bins/buffer size mismatch — call reset() before process()");

        let n   = self.num_bins;
        let atk = ctx.attack_ms;
        let rel = ctx.release_ms;

        for k in 0..n {
            // curves: [0]=threshold, [1]=ratio, [2]=attack_factor, [3]=release_factor,
            //         [4]=knee, [5]=mix
            let t    = curves.get(0).and_then(|c| c.get(k)).copied().unwrap_or(1.0);
            let t_db = linear_to_db(t);
            self.bp_threshold[k] = (-20.0 + t_db * (60.0 / 18.0)).clamp(-60.0, 0.0);

            let r = curves.get(1).and_then(|c| c.get(k)).copied().unwrap_or(1.0);
            self.bp_ratio[k] = r.clamp(1.0, 20.0);

            let af = curves.get(2).and_then(|c| c.get(k)).copied().unwrap_or(1.0).max(0.01);
            self.bp_attack[k] = (atk * af).clamp(0.1, 500.0);

            let rf = curves.get(3).and_then(|c| c.get(k)).copied().unwrap_or(1.0).max(0.01);
            self.bp_release[k] = (rel * rf).clamp(1.0, 2000.0);

            let kn = curves.get(4).and_then(|c| c.get(k)).copied().unwrap_or(1.0);
            self.bp_knee[k] = (kn * 6.0).clamp(0.0, 48.0);

            let mx = curves.get(5).and_then(|c| c.get(k)).copied().unwrap_or(1.0);
            self.bp_mix[k] = mx.clamp(0.0, 1.0);
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
            auto_makeup:         ctx.auto_makeup,
            smoothing_semitones: ctx.suppression_width,
        };

        let eng: &mut Box<dyn SpectralEngine> = match stereo_link {
            StereoLink::Independent if channel == 1 => &mut self.engine_r,
            _ => &mut self.engine,
        };
        eng.process_bins(bins, sidechain, &params, self.sample_rate, suppression_out);

        #[cfg(any(test, feature = "probe"))]
        {
            let k = self.num_bins / 2;
            self.last_probe = crate::dsp::modules::ProbeSnapshot {
                threshold_db: Some(self.bp_threshold[k]),
                ratio:        Some(self.bp_ratio[k]),
                attack_ms:    Some(self.bp_attack[k]),
                release_ms:   Some(self.bp_release[k]),
                knee_db:      Some(self.bp_knee[k]),
                mix_pct:      Some(self.bp_mix[k] * 100.0),
                ..Default::default()
            };
        }
    }

    fn clear_state(&mut self) {
        self.engine.clear_state();
        self.engine_r.clear_state();
        self.bp_threshold.fill(-20.0);
        self.bp_ratio.fill(1.0);
        self.bp_attack.fill(10.0);
        self.bp_release.fill(100.0);
        self.bp_knee.fill(6.0);
        self.bp_makeup.fill(0.0);
        self.bp_mix.fill(1.0);
    }

    fn module_type(&self) -> ModuleType { ModuleType::Dynamics }
    fn num_curves(&self) -> usize { 6 }

    #[cfg(any(test, feature = "probe"))]
    fn last_probe(&self) -> crate::dsp::modules::ProbeSnapshot { self.last_probe }
}
