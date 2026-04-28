//! Shared test signal helpers for Phase 4.9 audio-render regression tests.
//!
//! All functions produce deterministic, allocation-friendly vectors suitable for
//! use as synthetic test stimuli. None of these run on the audio thread, so
//! standard Vec allocation is fine here.

use std::f32::consts::PI;
use num_complex::Complex;
use realfft::RealFftPlanner;

/// Synthesize a linear frequency sweep (chirp) from `start_hz` to `end_hz`.
///
/// Uses the linear instantaneous-frequency formula:
///   φ(n) = 2π · (start_hz · n + (end_hz - start_hz) / (2 · num_samples) · n²) / sample_rate
///
/// Linear (not log) because the dominant quality metric here is frame-to-frame
/// spectral centroid stability, not perceptual frequency resolution.
pub fn sine_sweep(
    start_hz: f32,
    end_hz: f32,
    sample_rate: f32,
    num_samples: usize,
) -> Vec<f32> {
    let mut out = Vec::with_capacity(num_samples);
    let k = (end_hz - start_hz) / (2.0 * num_samples as f32);
    for n in 0..num_samples {
        let nf = n as f32;
        let phase = 2.0 * PI * (start_hz * nf + k * nf * nf) / sample_rate;
        out.push(phase.sin());
    }
    out
}

/// Synthesize a sum of pure sines (a chord).
///
/// `freqs`: slice of frequencies in Hz; each partial has unit amplitude.
/// Output is normalised by the number of partials to keep peak level ≤ 1.0.
pub fn chord(freqs: &[f32], sample_rate: f32, num_samples: usize) -> Vec<f32> {
    if freqs.is_empty() {
        return vec![0.0; num_samples];
    }
    let inv = 1.0 / freqs.len() as f32;
    let mut out = vec![0.0f32; num_samples];
    for &f in freqs {
        let omega = 2.0 * PI * f / sample_rate;
        for (n, s) in out.iter_mut().enumerate() {
            *s += (omega * n as f32).sin() * inv;
        }
    }
    out
}

/// Synthesize a synthetic drum loop: kick + snare + hi-hat.
///
/// All components are deterministic (no RNG). Sharp amplitude envelopes are
/// used to stress transient preservation under PLPV.
///
/// Pattern (16ths at ~120 BPM, period = 22050 samples at 44100 Hz):
///   - Kick:  beats 1, 3 — pitched sine with exponential decay (~40 ms)
///   - Snare: beats 2, 4 — noise burst approximated with a sum of co-prime partials
///   - Hi-hat: every 8th at half the period — very short burst (~5 ms)
///
/// Deterministic "noise" for snare: sum of 32 co-prime high-frequency sines.
pub fn drum_loop(sample_rate: f32, num_samples: usize) -> Vec<f32> {
    // 16th-note grid at 120 BPM: period_16th = 60s/(120*4) = 125 ms
    let period_16th = (sample_rate * 0.125) as usize;

    // Exponential decay envelopes (in samples)
    let kick_decay   = (sample_rate * 0.040) as usize; // 40 ms
    let snare_decay  = (sample_rate * 0.025) as usize; // 25 ms
    let hat_decay    = (sample_rate * 0.005) as usize; //  5 ms

    let kick_freq    = 60.0_f32;   // Hz
    let kick_omega   = 2.0 * PI * kick_freq / sample_rate;

    let mut out = vec![0.0f32; num_samples];

    for (n, s) in out.iter_mut().enumerate() {
        let beat = n / period_16th; // which 16th-note we're on
        let off  = n % period_16th; // sample offset within that 16th

        // Kick on beats 0, 8 (i.e., quarter-notes 1 and 3 of a bar)
        let is_kick  = beat % 8 == 0 || beat % 8 == 4;
        // Snare on beats 4, 12 (quarter-notes 2 and 4)
        let is_snare = beat % 8 == 2 || beat % 8 == 6;
        // Hi-hat on every 2nd 16th
        let is_hat   = beat % 2 == 1;

        if is_kick && off < kick_decay {
            let env = (-3.0 * off as f32 / kick_decay as f32).exp();
            *s += (kick_omega * off as f32).sin() * env;
        }

        if is_snare && off < snare_decay {
            let env = (-4.0 * off as f32 / snare_decay as f32).exp();
            // Deterministic "noise": sum 8 inharmonic sines at co-prime frequencies
            let mut noise = 0.0f32;
            for i in 1u32..=8 {
                let f = sample_rate / 4.0 * (2 * i + 1) as f32 / 17.0;
                noise += (2.0 * PI * f / sample_rate * off as f32).sin();
            }
            *s += noise * env * 0.25; // scale down to avoid clipping
        }

        if is_hat && off < hat_decay {
            let env = (-5.0 * off as f32 / hat_decay as f32).exp();
            // Hat: 4 very high frequency sines
            let mut hiss = 0.0f32;
            for i in 1u32..=4 {
                let f = sample_rate * 0.45 * i as f32 / 4.0;
                hiss += (2.0 * PI * f / sample_rate * off as f32).sin();
            }
            *s += hiss * env * 0.15;
        }
    }

    out
}

/// Compute one forward FFT frame from a time-domain slice of length `fft_size`.
///
/// Applies a Hann window before the FFT.
/// Returns `fft_size / 2 + 1` complex bins (positive frequencies only).
///
/// Useful for building a single spectral frame from a time-domain signal
/// without running the full STFT pipeline.
pub fn forward_fft(frame: &[f32]) -> Vec<Complex<f32>> {
    let fft_size = frame.len();
    let num_bins = fft_size / 2 + 1;

    // Hann window
    let mut windowed: Vec<f32> = frame
        .iter()
        .enumerate()
        .map(|(i, &s)| {
            let w = 0.5 * (1.0 - (2.0 * PI * i as f32 / (fft_size - 1) as f32).cos());
            s * w
        })
        .collect();

    let mut planner = RealFftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(fft_size);
    let mut spectrum = fft.make_output_vec();
    fft.process(&mut windowed, &mut spectrum).unwrap();

    // plan_fft_forward returns num_bins bins
    debug_assert_eq!(spectrum.len(), num_bins);
    spectrum
}

/// Spectral centroid in bin units: Σ(k · |X[k]|) / Σ|X[k]|.
///
/// Returns 0.0 if all magnitudes are zero (degenerate case).
pub fn spectral_centroid(bins: &[Complex<f32>]) -> f32 {
    let mut num = 0.0_f32;
    let mut den = 0.0_f32;
    for (k, b) in bins.iter().enumerate() {
        let m = b.norm();
        num += k as f32 * m;
        den += m;
    }
    if den < 1e-12 { 0.0 } else { num / den }
}

/// Variance of a float slice (population variance, no Bessel correction).
/// Returns 0.0 if the slice has fewer than 2 elements.
pub fn variance(xs: &[f32]) -> f32 {
    if xs.len() < 2 {
        return 0.0;
    }
    let mean = xs.iter().sum::<f32>() / xs.len() as f32;
    xs.iter().map(|x| (x - mean) * (x - mean)).sum::<f32>() / xs.len() as f32
}
