use num_complex::Complex;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
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

    // ── Lifter mode — cepstrum-domain envelope/pitch shaping (Phase 6.5 Task 8) ──
    /// Forward real-FFT used by Lifter to round-trip edited cepstrum → log-magnitude.
    /// `None` until `reset()` is called; Arc::clone is ref-count only (RT-safe).
    fwd_fft:        Option<Arc<dyn realfft::RealToComplex<f32>>>,
    /// Real-valued input buffer for the forward FFT (length = fft_size).
    /// Written in-place with the edited cepstrum; realfft reads+scratches it.
    fwd_input:      Vec<f32>,
    /// Complex output of the forward FFT (length = num_bins).
    fwd_output:     Vec<Complex<f32>>,
    /// realfft internal scratch area (length = get_scratch_len()).
    fwd_scratch:    Vec<Complex<f32>>,
    /// Extracted .re values from fwd_output (length = num_bins): the edited log-magnitude.
    log_mag_edited: Vec<f32>,
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
            // Lifter fields — allocated in reset().
            fwd_fft:        None,
            fwd_input:      Vec::new(),
            fwd_output:     Vec::new(),
            fwd_scratch:    Vec::new(),
            log_mag_edited: Vec::new(),
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

impl HarmonyModule {
    fn process_inharmonic(
        &mut self,
        bins: &mut [Complex<f32>],
        curves: &[&[f32]],
        ctx: &ModuleContext,
    ) {
        use crate::dsp::modules::harmony_helpers::{
            find_top_k_peaks, BESSEL_J0_ZEROS, SMALL_PRIMES,
        };

        let n = self.num_bins;
        let amount      = curves.get(0).copied().unwrap_or(&[]);
        let threshold   = curves.get(1).copied().unwrap_or(&[]);
        let coefficient = curves.get(4).copied().unwrap_or(&[]);
        let mix         = curves.get(5).copied().unwrap_or(&[]);

        for k in 0..n { self.scratch_mag[k] = bins[k].norm(); }
        let thr_centre = threshold.get(n / 2).copied().unwrap_or(0.1);
        let n_peaks = find_top_k_peaks(&self.scratch_mag[..n], thr_centre, &mut self.peaks_buf);
        if n_peaks == 0 { return; }

        let bin_freq = ctx.sample_rate / ctx.fft_size as f32;
        let nyquist  = ctx.sample_rate * 0.5;

        // The first peak (loudest) is treated as fundamental; remaining peaks are partials.
        let pk0 = self.peaks_buf[0];
        let f0 = match ctx.instantaneous_freq {
            Some(if_buf) => if_buf.get(pk0.bin).copied().unwrap_or(0.0),
            None => (pk0.bin as f32) * bin_freq,
        };
        if f0 <= 0.0 { return; }

        // COEFFICIENT ∈ [0, 2] in all sub-modes — interpretation depends on submode.
        let coef_centre = coefficient.get(n / 2).copied().unwrap_or(1.0).clamp(0.0, 2.0);

        // Copy submode to a local to avoid borrow-checker conflict with &mut self in closure.
        let submode = self.inharmonic_submode;

        let target_freq_for_n = |n_idx: usize| -> f32 {
            match submode {
                HarmonyInharmonicSubmode::Stiffness => {
                    // B ∈ [0, 0.001] for COEFFICIENT ∈ [0, 2]. Piano-like B ≈ 0.0004.
                    let b = coef_centre * 0.0005;
                    let n_f = n_idx as f32;
                    f0 * n_f * (1.0 + b * n_f * n_f).sqrt()
                }
                HarmonyInharmonicSubmode::Bessel => {
                    // n_idx = 1 → 2nd partial uses BESSEL_J0_ZEROS[1] / [0].
                    let idx = (n_idx - 1).min(BESSEL_J0_ZEROS.len() - 1);
                    f0 * BESSEL_J0_ZEROS[idx] / BESSEL_J0_ZEROS[0]
                }
                HarmonyInharmonicSubmode::Prime => {
                    let idx = (n_idx - 1).min(SMALL_PRIMES.len() - 1);
                    f0 * SMALL_PRIMES[idx] as f32 / SMALL_PRIMES[0] as f32
                }
            }
        };

        // For partials 2..n_peaks: estimate which harmonic n the source peak is,
        // then move it to the target frequency.
        for p in 1..n_peaks {
            let pk = self.peaks_buf[p];
            let amt = amount.get(pk.bin).copied().unwrap_or(0.0).clamp(0.0, 4.0);
            if amt < 1e-9 { continue; }
            let mix_v = mix.get(pk.bin).copied().unwrap_or(1.0).clamp(0.0, 1.0);

            let pk_freq = match ctx.instantaneous_freq {
                Some(if_buf) => if_buf.get(pk.bin).copied().unwrap_or(0.0),
                None => (pk.bin as f32) * bin_freq,
            };
            if pk_freq <= 0.0 { continue; }
            let n_idx = (pk_freq / f0).round().max(1.0) as usize;
            if n_idx <= 1 { continue; }

            let target_freq = target_freq_for_n(n_idx);
            if target_freq <= 0.0 || target_freq >= nyquist { continue; }
            let target_bin = (target_freq / bin_freq + 0.5) as usize;
            if target_bin == 0 || target_bin >= n - 1 { continue; }

            // Move energy: attenuate source by amt, add same amt to target.
            let attn = (1.0 - amt * mix_v).max(0.0);
            let original = bins[pk.bin];
            bins[pk.bin] = original * attn;
            bins[target_bin] += original * (amt * mix_v);
        }
    }
}

impl HarmonyModule {
    /// Lifter mode: cepstrum-domain envelope/pitch shaping.
    ///
    /// Reads `ctx.cepstrum_buf` (Phase 6.4 lazy infra). Edits the cepstrum
    /// by gating low-quefrency samples with SPREAD (envelope shaping) and
    /// high-quefrency samples with COEFFICIENT (pitch shaping). Forward-FFTs
    /// the edited cepstrum back to log-magnitude, exponentiates, and re-applies
    /// with the original phase. Wet/dry blend by AMOUNT × MIX.
    ///
    /// RT-safe: uses pre-allocated `fwd_input`, `fwd_output`, `fwd_scratch`
    /// buffers. `Arc::clone` on `fwd_fft` only increments a refcount — no heap.
    fn process_lifter(
        &mut self,
        bins: &mut [Complex<f32>],
        curves: &[&[f32]],
        ctx: &ModuleContext,
    ) {
        let n = self.num_bins;
        let cepstrum = match ctx.cepstrum_buf {
            Some(c) if c.len() == self.fft_size => c,
            _ => return, // Phase 6.4 not active or wrong size; passthrough.
        };

        let amount  = curves.get(0).copied().unwrap_or(&[]);
        let spread  = curves.get(3).copied().unwrap_or(&[]); // envelope-quefrency gain
        let coef    = curves.get(4).copied().unwrap_or(&[]); // pitch-quefrency gain
        let mix     = curves.get(5).copied().unwrap_or(&[]);

        let amt_centre = amount.get(n / 2).copied().unwrap_or(0.0).clamp(0.0, 1.0);
        if amt_centre < 1e-9 { return; }

        // Quefrency boundary: low-quefrency = first fft_size/8 samples (envelope),
        // the rest (mirrored) = pitch. This is a coarse v1 split; future tasks can
        // expose env_cutoff as a curve.
        let env_cutoff = self.fft_size / 8;
        // SPREAD and COEFFICIENT sampled at spectrum centre — scalar gains for v1.
        let env_gain   = spread.get(n / 2).copied().unwrap_or(1.0).clamp(0.0, 4.0);
        let pitch_gain = coef.get(n / 2).copied().unwrap_or(1.0).clamp(0.0, 4.0);

        // Edit cepstrum into fwd_input directly — no clone of the Vec.
        // qf is the folded quefrency index (symmetric for real signals).
        for q in 0..self.fft_size {
            let qf = if q <= self.fft_size / 2 { q } else { self.fft_size - q };
            let g = if qf < env_cutoff { env_gain } else { pitch_gain };
            self.fwd_input[q] = cepstrum[q] * g;
        }

        // Forward real-FFT → complex log-magnitude. Arc::clone = refcount bump only.
        let fft = match self.fwd_fft.as_ref() { Some(f) => f.clone(), None => return };
        let _ = fft.process_with_scratch(
            &mut self.fwd_input,
            &mut self.fwd_output,
            &mut self.fwd_scratch,
        );
        // realfft inverse FFT (used by CepstrumBuf) does NOT normalize — it scales
        // by fft_size. The forward FFT here undoes that, so we divide by fft_size to
        // recover the original log-magnitude values.
        let inv_n = 1.0 / self.fft_size as f32;
        for k in 0..n {
            self.log_mag_edited[k] = self.fwd_output[k].re * inv_n;
        }

        // Re-apply: target_mag = exp(log_mag_edited[k]); preserve original phase.
        let mix_centre = mix.get(n / 2).copied().unwrap_or(1.0).clamp(0.0, 1.0);
        let blend = (amt_centre * mix_centre).clamp(0.0, 1.0);
        for k in 0..n {
            let target_mag = self.log_mag_edited[k].exp();
            let phase = bins[k].arg();
            let edited = Complex::from_polar(target_mag, phase);
            bins[k] = edited * blend + bins[k] * (1.0 - blend);
        }
    }
}

impl HarmonyModule {
    fn process_chordification(
        &mut self,
        bins: &mut [Complex<f32>],
        curves: &[&[f32]],
        ctx: &ModuleContext,
    ) {
        use crate::dsp::modules::harmony_helpers::{best_chord_template, CHORD_TEMPLATES_24};

        let n = self.num_bins;
        let chroma = match ctx.chromagram {
            Some(c) => c,
            None => return,
        };
        let (chord_idx, _score) = best_chord_template(chroma);
        let chord = &CHORD_TEMPLATES_24[chord_idx];

        let amount    = curves.get(0).copied().unwrap_or(&[]);
        let threshold = curves.get(1).copied().unwrap_or(&[]);
        let spread    = curves.get(3).copied().unwrap_or(&[]);
        let mix       = curves.get(5).copied().unwrap_or(&[]);

        let bin_freq = ctx.sample_rate / ctx.fft_size as f32;

        for k in 1..n {
            let mag = bins[k].norm();
            let thr = threshold.get(k).copied().unwrap_or(0.0);
            if mag < thr { continue; }
            let amt = amount.get(k).copied().unwrap_or(0.0).clamp(0.0, 1.0);
            if amt < 1e-9 { continue; }

            // Estimate pitch class from bin's IF-refined frequency.
            let f = match ctx.instantaneous_freq {
                Some(if_buf) => if_buf.get(k).copied().unwrap_or(0.0),
                None => k as f32 * bin_freq,
            };
            if f < 27.5 { continue; } // below A0
            let midi = 12.0 * (f / 440.0).log2() + 69.0;
            let pc = ((midi.round() as i32).rem_euclid(12)) as usize;

            // If already in chord, skip.
            if chord[pc] > 0.5 { continue; }

            // Find nearest in-chord PC.
            let mut nearest_d = 12_i32;
            for cpc in 0..12 {
                if chord[cpc] < 0.5 { continue; }
                let d = ((pc as i32 - cpc as i32) + 6).rem_euclid(12) - 6; // signed −6..=5
                if d.abs() < nearest_d.abs() { nearest_d = d; }
            }
            let delta_semitones = -(nearest_d as f32);
            // SPREAD ∈ [0,2] → snap radius ∈ [0.0, 1.0] of full snap.
            let s = spread.get(k).copied().unwrap_or(1.0).clamp(0.0, 2.0) * 0.5;
            let snap = amt * s;

            let new_midi = midi + snap * delta_semitones;
            let new_freq = 440.0 * 2.0_f32.powf((new_midi - 69.0) / 12.0);
            let target_bin = (new_freq / bin_freq + 0.5) as usize;
            if target_bin == 0 || target_bin >= n - 1 { continue; }
            if target_bin == k { continue; }

            let mix_v = mix.get(k).copied().unwrap_or(1.0).clamp(0.0, 1.0);
            let original = bins[k];
            bins[k] = original * (1.0 - mix_v);
            bins[target_bin] += original * mix_v;
        }
    }
}

impl HarmonyModule {
    /// Formant Rotation mode: cepstrum-domain envelope preservation + harmonic shift.
    ///
    /// Extracts the spectral envelope from the low-quefrency portion of the cepstrum,
    /// computes the residual (log_mag - envelope), shifts the residual by COEFFICIENT
    /// ratio (clamped [0.5, 2.0]), then recombines shifted-residual + original envelope.
    /// This preserves formants while shifting harmonics.
    ///
    /// RT-safe: reuses `fwd_input`, `fwd_output`, `fwd_scratch` pre-allocated in Task 8.
    /// `Arc::clone` on `fwd_fft` only increments a refcount — no heap.
    fn process_formant_rotation(
        &mut self,
        bins: &mut [Complex<f32>],
        curves: &[&[f32]],
        ctx: &ModuleContext,
    ) {
        let n = self.num_bins;
        let cepstrum = match ctx.cepstrum_buf {
            Some(c) if c.len() == self.fft_size => c,
            _ => return, // Phase 6.4 not active or wrong size; passthrough.
        };

        let amount      = curves.get(0).copied().unwrap_or(&[]);
        let coefficient = curves.get(4).copied().unwrap_or(&[]);
        let mix         = curves.get(5).copied().unwrap_or(&[]);

        let amt_centre = amount.get(n / 2).copied().unwrap_or(0.0).clamp(0.0, 1.0);
        if amt_centre < 1e-9 { return; }

        // COEFFICIENT curve: 1.0 = identity (no rotation). Clamp to [0.5, 2.0].
        let ratio = coefficient.get(n / 2).copied().unwrap_or(1.0).clamp(0.5, 2.0);
        if (ratio - 1.0).abs() < 1e-3 {
            // Identity rotation; nothing to do.
            return;
        }

        // Step 1: extract envelope from cepstrum (low-quefrency window only).
        // Fold index qf is symmetric for real signals.
        let env_cutoff = self.fft_size / 8;
        for q in 0..self.fft_size {
            let qf = if q <= self.fft_size / 2 { q } else { self.fft_size - q };
            self.fwd_input[q] = if qf < env_cutoff { cepstrum[q] } else { 0.0 };
        }

        // Forward real-FFT → complex log-magnitude. Arc::clone = refcount bump only.
        let fft = match self.fwd_fft.as_ref() { Some(f) => f.clone(), None => return };
        let _ = fft.process_with_scratch(
            &mut self.fwd_input,
            &mut self.fwd_output,
            &mut self.fwd_scratch,
        );
        // realfft inverse FFT (used by CepstrumBuf) does NOT normalize — it scales by
        // fft_size. The forward FFT here undoes that, so divide by fft_size to recover
        // the original log-magnitude values. (Mirrors Lifter mode normalization.)
        let inv_n = 1.0 / self.fft_size as f32;
        for k in 0..n {
            self.log_mag_edited[k] = self.fwd_output[k].re * inv_n;
        }

        // Step 2: residual[k] = log|bins[k]| - log_envelope[k].
        // Step 3: shift residual: residual'[k] = residual[round(k / ratio)].
        // Step 4: log_mag_target[k] = log_envelope[k] + residual'[k].
        let mix_centre = mix.get(n / 2).copied().unwrap_or(1.0).clamp(0.0, 1.0);
        let blend = (amt_centre * mix_centre).clamp(0.0, 1.0);
        for k in 0..n {
            let env = self.log_mag_edited[k];
            let src_k = ((k as f32) / ratio).round() as usize;
            let src_full = if src_k < n {
                bins[src_k].norm().max(1e-10).ln()
            } else {
                env // no source → use envelope only (residual = 0)
            };
            let src_env = if src_k < n { self.log_mag_edited[src_k] } else { env };
            let residual = src_full - src_env;
            let log_target = env + residual;
            let target_mag = log_target.exp();
            let phase = bins[k].arg();
            let edited = Complex::from_polar(target_mag, phase);
            bins[k] = edited * blend + bins[k] * (1.0 - blend);
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

        // Lifter: allocate forward real-FFT + scratch buffers (off the audio thread).
        let mut planner = realfft::RealFftPlanner::<f32>::new();
        let fwd = planner.plan_fft_forward(fft_size);
        let scratch_len = fwd.get_scratch_len();
        self.fwd_input.resize(fft_size, 0.0);
        self.fwd_output.resize(self.num_bins, Complex::new(0.0, 0.0));
        self.fwd_scratch.resize(scratch_len, Complex::new(0.0, 0.0));
        self.log_mag_edited.resize(self.num_bins, 0.0);
        self.fwd_fft = Some(fwd);
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
            HarmonyMode::Chordification    => self.process_chordification(bins, curves, ctx),
            HarmonyMode::Undertone         => self.process_undertone(bins, curves, ctx),
            HarmonyMode::Companding        => { /* TODO Task 12 */ }
            HarmonyMode::FormantRotation   => self.process_formant_rotation(bins, curves, ctx),
            HarmonyMode::Lifter            => self.process_lifter(bins, curves, ctx),
            HarmonyMode::Inharmonic        => self.process_inharmonic(bins, curves, ctx),
            HarmonyMode::HarmonicGenerator => self.process_harmonic_generator(bins, curves, ctx),
            HarmonyMode::Shuffler          => self.process_shuffler(bins, curves),
        }
    }

    fn module_type(&self) -> ModuleType { ModuleType::Harmony }
    fn num_curves(&self) -> usize { 6 }
}
