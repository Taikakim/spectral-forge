use num_complex::Complex;
use crate::dsp::engines::{BinParams, SpectralEngine};
use crate::dsp::engines::spectral_contrast::SpectralContrastEngine;
use crate::params::{FxChannelTarget, StereoLink};
use super::{ModuleContext, ModuleType, SpectralModule};

// ── ContrastMode ──────────────────────────────────────────────────────────

#[derive(Default, Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ContrastMode {
    #[default]
    Spatial,
    Temporal,
    Tilt,
}

// ── ContrastScalars ───────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug)]
pub struct ContrastScalars {
    pub mean_window_st:        f32,  // Spatial only
    pub tilt_slope_db_per_oct: f32,  // Tilt only
}

impl ContrastScalars {
    pub fn safe_default() -> Self {
        Self { mean_window_st: 1.0, tilt_slope_db_per_oct: 0.0 }
    }
}

impl Default for ContrastScalars {
    fn default() -> Self { Self::safe_default() }
}

// ── ContrastModule ────────────────────────────────────────────────────────

pub struct ContrastModule {
    engine:       SpectralContrastEngine,
    mode:         ContrastMode,
    scalars:      ContrastScalars,
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
            engine:       SpectralContrastEngine::new(),
            mode:         ContrastMode::default(),
            scalars:      ContrastScalars::safe_default(),
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

    pub fn set_mode(&mut self, mode: ContrastMode) {
        self.mode = mode;
    }

    pub fn set_contrast_scalars(&mut self, scalars: ContrastScalars) {
        self.scalars = scalars;
    }

    #[cfg(any(test, feature = "probe"))]
    pub fn test_contrast_scalars(&self) -> ContrastScalars {
        self.scalars
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
        _sidechain: Option<&[f32]>,
        curves: &[&[f32]],
        suppression_out: &mut [f32],
        _physics: Option<&mut crate::dsp::bin_physics::BinPhysics>,
        ctx: &ModuleContext<'_>,
    ) {
        // Contrast now mirrors Dynamics' 6-curve layout (2026-05-08): every
        // bin parameter is exposed for prototyping. See `freeze.rs::curve_to_threshold_db`
        // for the canonical dBFS calibration; ATTACK / RELEASE multiply the
        // global Atk/Rel knobs (so curve gain 1.0 hits the global value).
        use super::freeze::curve_to_threshold_db;
        let n = self.num_bins;
        for k in 0..n {
            let thr_g  = curves.get(0).and_then(|c| c.get(k)).copied().unwrap_or(1.0);
            let rat_g  = curves.get(1).and_then(|c| c.get(k)).copied().unwrap_or(1.0);
            let atk_g  = curves.get(2).and_then(|c| c.get(k)).copied().unwrap_or(1.0);
            let rel_g  = curves.get(3).and_then(|c| c.get(k)).copied().unwrap_or(1.0);
            let knee_g = curves.get(4).and_then(|c| c.get(k)).copied().unwrap_or(1.0);
            let mix_g  = curves.get(5).and_then(|c| c.get(k)).copied().unwrap_or(1.0);

            self.bp_threshold[k] = curve_to_threshold_db(thr_g);
            self.bp_ratio[k]     = rat_g.clamp(1.0, 20.0);
            self.bp_attack[k]    = (ctx.attack_ms  * atk_g.max(0.0)).clamp(0.1, 500.0);
            self.bp_release[k]   = (ctx.release_ms * rel_g.max(0.0)).clamp(1.0, 2000.0);
            self.bp_knee[k]      = (knee_g * 6.0).clamp(0.0, 48.0);
            self.bp_mix[k]       = mix_g.clamp(0.0, 1.0);
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
            // SpectralContrastEngine ignores these — set to inert defaults so
            // peak-locked ducking is a Dynamics-only feature.
            peaks:                 None,
            plpv_dynamics_enabled: false,
        };

        // Mode dispatch — Spatial / Temporal / Tilt kernels.
        match self.mode {
            ContrastMode::Spatial => {
                self.engine.process_bins_spatial(
                    bins, &params, self.sample_rate,
                    self.scalars.mean_window_st,
                    suppression_out,
                );
            }
            ContrastMode::Temporal => {
                self.engine.process_bins_temporal(
                    bins, &params, self.sample_rate, suppression_out,
                );
            }
            ContrastMode::Tilt => {
                self.engine.process_bins_tilt(
                    bins, &params, ctx.fft_size, self.sample_rate,
                    self.scalars.tilt_slope_db_per_oct,
                    suppression_out,
                );
            }
        }

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
    fn num_curves(&self) -> usize { 6 }

    fn set_contrast_mode(&mut self, mode: crate::dsp::modules::contrast::ContrastMode) {
        self.set_mode(mode);
    }

    fn set_contrast_scalars(&mut self, scalars: crate::dsp::modules::contrast::ContrastScalars) {
        ContrastModule::set_contrast_scalars(self, scalars);
    }

    #[cfg(any(test, feature = "probe"))]
    fn test_contrast_scalars(&self) -> Option<crate::dsp::modules::contrast::ContrastScalars> {
        Some(ContrastModule::test_contrast_scalars(self))
    }

    #[cfg(any(test, feature = "probe"))]
    fn last_probe(&self) -> crate::dsp::modules::ProbeSnapshot { self.last_probe }
}
