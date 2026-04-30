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
    /// Pre-allocated peak buffer for HarmonicGenerator mode (K=5).
    peaks_buf:          [crate::dsp::modules::harmony_helpers::PeakRecord; 5],
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
            peaks_buf: [crate::dsp::modules::harmony_helpers::PeakRecord::default(); 5],
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

impl HarmonyModule {
    fn process_undertone(
        &mut self,
        bins: &mut [Complex<f32>],
        curves: &[&[f32]],
        ctx: &ModuleContext,
    ) {
        use crate::dsp::modules::harmony_helpers::find_top_k_peaks;

        let n = self.num_bins;
        let amount      = curves.get(0).copied().unwrap_or(&[]);
        let threshold   = curves.get(1).copied().unwrap_or(&[]);
        let spread      = curves.get(3).copied().unwrap_or(&[]);
        let coefficient = curves.get(4).copied().unwrap_or(&[]);
        let mix         = curves.get(5).copied().unwrap_or(&[]);

        for k in 0..n { self.scratch_mag[k] = bins[k].norm(); }
        let thr_centre = threshold.get(n / 2).copied().unwrap_or(0.1);
        let n_peaks = find_top_k_peaks(&self.scratch_mag[..n], thr_centre, &mut self.peaks_buf);

        let bin_freq = ctx.sample_rate / ctx.fft_size as f32;
        let nyquist  = ctx.sample_rate * 0.5;

        // COEFFICIENT ∈ [0,2] → hum freq selector:
        //   0.0–0.5 = off, 0.5–1.0 = 50Hz, 1.0–1.5 = 60Hz, 1.5–2.0 = 120Hz.
        // The hum modulates the undertone amplitudes per partial.
        let hum_centre = coefficient.get(n / 2).copied().unwrap_or(0.0).clamp(0.0, 2.0);
        let hum_hz = if      hum_centre < 0.5 { 0.0   }
                     else if hum_centre < 1.0 { 50.0  }
                     else if hum_centre < 1.5 { 60.0  }
                     else                     { 120.0 };

        for p in 0..n_peaks {
            let pk = self.peaks_buf[p];
            let f0 = match ctx.instantaneous_freq {
                Some(if_buf) => if_buf.get(pk.bin).copied().unwrap_or(0.0),
                None => (pk.bin as f32) * bin_freq,
            };
            if f0 <= 0.0 { continue; }

            let amt = amount.get(pk.bin).copied().unwrap_or(0.0).clamp(0.0, 4.0);
            if amt < 1e-9 { continue; }
            let mix_v = mix.get(pk.bin).copied().unwrap_or(1.0).clamp(0.0, 1.0);
            let s = spread.get(pk.bin).copied().unwrap_or(1.0).clamp(0.0, 2.0);
            let decay = 0.95 - 0.275 * s; // same shape as Harmonic Generator.
            let phase = bins[pk.bin].arg();

            // Hum amplitude weight: closer to a hum-multiple → higher weight.
            // For the ground-loop hum, undertones near hum_hz get +30% boost.
            let hum_weight = |freq: f32| -> f32 {
                if hum_hz <= 0.0 || freq <= 0.0 { 1.0 }
                else {
                    let octaves_off = ((freq / hum_hz).log2()).abs().min(2.0);
                    1.0 + 0.3 * (1.0 - octaves_off * 0.5).max(0.0)
                }
            };

            let mut amp = pk.mag;
            for div in 2..=8 {
                amp *= decay;
                let f_under = f0 / div as f32;
                if f_under < 20.0 { break; }
                if f_under >= nyquist { continue; }
                let target_bin = (f_under / bin_freq + 0.5) as usize;
                if target_bin == 0 || target_bin >= n - 1 { break; }
                let w = hum_weight(f_under);
                let added = Complex::from_polar(amp * amt * mix_v * w, phase);
                bins[target_bin] += added;
            }
        }
    }
}

impl HarmonyModule {
    fn process_harmonic_generator(
        &mut self,
        bins: &mut [Complex<f32>],
        curves: &[&[f32]],
        ctx: &ModuleContext,
    ) {
        use crate::dsp::modules::harmony_helpers::find_top_k_peaks;

        let n = self.num_bins;
        let amount      = curves.get(0).copied().unwrap_or(&[]);
        let threshold   = curves.get(1).copied().unwrap_or(&[]);
        let spread      = curves.get(3).copied().unwrap_or(&[]);
        let coefficient = curves.get(4).copied().unwrap_or(&[]);
        let mix         = curves.get(5).copied().unwrap_or(&[]);

        // Pre-compute magnitudes into scratch.
        for k in 0..n { self.scratch_mag[k] = bins[k].norm(); }

        // Threshold is sampled at centre-of-spectrum to keep peak detection one-shot.
        let thr_centre = threshold.get(n / 2).copied().unwrap_or(0.1);
        let n_peaks = find_top_k_peaks(&self.scratch_mag[..n], thr_centre, &mut self.peaks_buf);

        // For each detected peak, generate its harmonic series.
        for p in 0..n_peaks {
            let pk = self.peaks_buf[p];
            let f0 = match ctx.instantaneous_freq {
                Some(if_buf) => if_buf.get(pk.bin).copied().unwrap_or(0.0),
                None => (pk.bin as f32) * ctx.sample_rate / ctx.fft_size as f32,
            };
            if f0 <= 0.0 { continue; }

            let amp_root = pk.mag;
            let amt      = amount.get(pk.bin).copied().unwrap_or(0.0).clamp(0.0, 4.0);
            if amt < 1e-9 { continue; }
            let mix_v    = mix.get(pk.bin).copied().unwrap_or(1.0).clamp(0.0, 1.0);
            // SPREAD ∈ [0,2] → decay ∈ [0.95, 0.40] (slow → fast).
            let s = spread.get(pk.bin).copied().unwrap_or(1.0).clamp(0.0, 2.0);
            let decay = 0.95 - 0.275 * s;
            // COEFFICIENT ∈ [0,2] → harmonic count ∈ [2, 32].
            let c = coefficient.get(pk.bin).copied().unwrap_or(1.0).clamp(0.0, 2.0);
            let hcount = 2 + (c * 15.0) as usize; // 2..=32

            let phase = bins[pk.bin].arg();
            let bin_freq = ctx.sample_rate / ctx.fft_size as f32;

            let mut amp = amp_root;
            for h in 2..=hcount {
                amp *= decay;
                let target_freq = f0 * h as f32;
                if target_freq >= ctx.sample_rate * 0.5 { break; }
                let target_bin = (target_freq / bin_freq + 0.5) as usize;
                if target_bin == 0 || target_bin >= n - 1 { break; }
                let added = Complex::from_polar(amp * amt * mix_v, phase);
                bins[target_bin] += added;
            }
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
        self.peaks_buf   = [crate::dsp::modules::harmony_helpers::PeakRecord::default(); 5];
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
        suppression_out.fill(0.0);

        // AMOUNT = 0 ⇒ pure passthrough for every mode.
        // The curve is sampled at the centre bin to avoid expensive per-bin
        // dispatch; per-bin AMOUNT scaling lives inside each mode method.
        let amount_centre = curves.get(0)
            .and_then(|c| c.get(self.num_bins / 2)).copied().unwrap_or(0.0);
        if amount_centre.abs() < 1e-9 { return; }

        match self.mode {
            HarmonyMode::Chordification    => { /* TODO Task 11 */ }
            HarmonyMode::Undertone         => self.process_undertone(bins, curves, ctx),
            HarmonyMode::Companding        => { /* TODO Task 12 */ }
            HarmonyMode::FormantRotation   => { /* TODO Task 9 */ }
            HarmonyMode::Lifter            => { /* TODO Task 10 */ }
            HarmonyMode::Inharmonic        => { /* TODO Task 7 */ }
            HarmonyMode::HarmonicGenerator => self.process_harmonic_generator(bins, curves, ctx),
            HarmonyMode::Shuffler          => self.process_shuffler(bins, curves),
        }
    }

    fn module_type(&self) -> ModuleType { ModuleType::Harmony }
    fn num_curves(&self) -> usize { 6 }
}
