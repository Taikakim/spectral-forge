use num_complex::Complex;
use serde::{Deserialize, Serialize};
use crate::params::{FxChannelTarget, StereoLink};
use super::{ModuleContext, ModuleType, SpectralModule};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum HarmonyMode {
    #[default]
    Chordification,
    Undertone,
    Companding,
    FormantRotation,
    Lifter,
    Inharmonic,
    HarmonicGenerator,
    Shuffler,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum HarmonyInharmonicSubmode {
    #[default]
    Stiffness,
    Bessel,
    Prime,
}

pub struct HarmonyModule {
    mode:               HarmonyMode,
    inharmonic_submode: HarmonyInharmonicSubmode,
    sample_rate:        f32,
    fft_size:           usize,
    num_bins:           usize,
    /// Scratch buffer for per-bin magnitude (shared across modes).
    scratch_mag:        Vec<f32>,
    /// Scratch buffer for output bins built additively (shared across modes).
    scratch_out:        Vec<Complex<f32>>,
}

impl HarmonyModule {
    pub fn new() -> Self {
        Self {
            mode: HarmonyMode::default(),
            inharmonic_submode: HarmonyInharmonicSubmode::default(),
            sample_rate: 48_000.0,
            fft_size: 2048,
            num_bins: 1025,
            scratch_mag: Vec::new(),
            scratch_out: Vec::new(),
        }
    }

    pub fn set_mode(&mut self, m: HarmonyMode) { self.mode = m; }
    pub fn set_inharmonic_submode(&mut self, m: HarmonyInharmonicSubmode) {
        self.inharmonic_submode = m;
    }
    pub fn mode(&self) -> HarmonyMode { self.mode }
    pub fn inharmonic_submode(&self) -> HarmonyInharmonicSubmode {
        self.inharmonic_submode
    }
}

impl Default for HarmonyModule {
    fn default() -> Self { Self::new() }
}

impl SpectralModule for HarmonyModule {
    fn reset(&mut self, sample_rate: f32, fft_size: usize) {
        self.sample_rate = sample_rate;
        self.fft_size    = fft_size;
        self.num_bins    = fft_size / 2 + 1;
        self.scratch_mag.resize(self.num_bins, 0.0);
        self.scratch_out.resize(self.num_bins, Complex::new(0.0, 0.0));
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
        suppression_out.fill(0.0);

        // AMOUNT = 0 ⇒ pure passthrough for every mode.
        // The curve is sampled at the centre bin to avoid expensive per-bin
        // dispatch; per-bin AMOUNT scaling lives inside each mode method.
        let amount_centre = curves.get(0)
            .and_then(|c| c.get(self.num_bins / 2)).copied().unwrap_or(0.0);
        if amount_centre.abs() < 1e-9 { return; }

        match self.mode {
            HarmonyMode::Chordification    => { /* TODO Task 11 */ }
            HarmonyMode::Undertone         => { /* TODO Task 6 */ }
            HarmonyMode::Companding        => { /* TODO Task 12 */ }
            HarmonyMode::FormantRotation   => { /* TODO Task 9 */ }
            HarmonyMode::Lifter            => { /* TODO Task 10 */ }
            HarmonyMode::Inharmonic        => { /* TODO Task 7 */ }
            HarmonyMode::HarmonicGenerator => { /* TODO Task 5 */ }
            HarmonyMode::Shuffler          => { /* TODO Task 4 */ }
        }
        let _ = bins;
    }

    fn module_type(&self) -> ModuleType { ModuleType::Harmony }
    fn num_curves(&self) -> usize { 6 }
}
