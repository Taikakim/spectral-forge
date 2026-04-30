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
    /// xorshift32 RNG state for Shuffler mode; seeded deterministically.
    rng_state:          u32,
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
            rng_state: 0xC0FFEE_u32,
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

impl HarmonyModule {
    fn process_shuffler(
        &mut self,
        bins: &mut [Complex<f32>],
        curves: &[&[f32]],
    ) {
        let n = self.num_bins;
        let amount    = curves.get(0).copied().unwrap_or(&[]);
        let threshold = curves.get(1).copied().unwrap_or(&[]);
        let spread    = curves.get(3).copied().unwrap_or(&[]);
        let mix       = curves.get(5).copied().unwrap_or(&[]);

        // xorshift32 RNG state lives in self.rng_state.
        let rng = &mut self.rng_state;

        for k in 1..n - 1 {
            let amt = amount.get(k).copied().unwrap_or(0.0).clamp(0.0, 1.0);
            if amt < 1e-9 { continue; }

            let mag = bins[k].norm();
            let thr = threshold.get(k).copied().unwrap_or(0.0);
            if mag < thr { continue; }

            // SPREAD curve in [0,2] → reach in [1, 16].
            let s = spread.get(k).copied().unwrap_or(1.0).clamp(0.0, 2.0);
            let reach = (1.0 + s * 7.5) as usize; // 1..=16
            if k + reach >= n { continue; }

            // xorshift32 rand in [0,1).
            *rng ^= *rng << 13;
            *rng ^= *rng >> 17;
            *rng ^= *rng << 5;
            let r = (*rng as f32 / u32::MAX as f32).clamp(0.0, 1.0);
            if r >= amt { continue; }

            let m = mix.get(k).copied().unwrap_or(1.0).clamp(0.0, 1.0);
            // Linear blend swap: out[k] = (1-m)*bins[k] + m*bins[k+reach];
            let a = bins[k];
            let b = bins[k + reach];
            bins[k] = a * (1.0 - m) + b * m;
            bins[k + reach] = b * (1.0 - m) + a * m;
        }
    }
}

impl SpectralModule for HarmonyModule {
    fn reset(&mut self, sample_rate: f32, fft_size: usize) {
        self.rng_state   = 0xC0FFEE_u32;
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
            HarmonyMode::Shuffler          => self.process_shuffler(bins, curves),
        }
    }

    fn module_type(&self) -> ModuleType { ModuleType::Harmony }
    fn num_curves(&self) -> usize { 6 }
}
