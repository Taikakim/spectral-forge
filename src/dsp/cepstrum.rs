//! Cepstrum analysis scratch (log-magnitude → inverse FFT).
//!
//! Per-channel scratch + output. `compute_from_bins` is RT-safe:
//! one magnitude-sq + ln + inverse real-FFT, no heap.

use num_complex::Complex;
use realfft::{RealFftPlanner, ComplexToReal};
use std::sync::Arc;

/// Magnitude-squared epsilon for the log clamp.
/// ε = 1e-10 ⇒ -100 dB log threshold (per research/02-pitch-and-cepstral.md Topic B).
const LOG_EPSILON_SQ: f32 = 1e-10;

/// Per-channel cepstrum scratch + output. One instance per channel in Pipeline.
pub struct CepstrumBuf {
    fft_size:  usize,
    inv_fft:   Arc<dyn ComplexToReal<f32>>,
    /// log_mag[k] = 0.5 * ln(|bins[k]|² + ε), Complex with imag=0
    /// (`ComplexToReal` consumes a Hermitian half-spectrum).
    log_mag:   Vec<Complex<f32>>,
    /// Quefrency-domain output, length = fft_size.
    cepstrum:  Vec<f32>,
    /// realfft scratch.
    scratch:   Vec<Complex<f32>>,
}

impl CepstrumBuf {
    /// Allocate buffers for the given FFT size. May allocate; not RT-safe.
    pub fn new(fft_size: usize) -> Self {
        let mut planner = RealFftPlanner::<f32>::new();
        let inv_fft     = planner.plan_fft_inverse(fft_size);
        let num_bins    = fft_size / 2 + 1;
        let scratch_len = inv_fft.get_scratch_len();
        Self {
            fft_size,
            inv_fft,
            log_mag:  vec![Complex::new(0.0, 0.0); num_bins],
            cepstrum: vec![0.0; fft_size],
            scratch:  vec![Complex::new(0.0, 0.0); scratch_len],
        }
    }

    /// Resize for a new FFT size. May allocate; only call off the audio thread.
    pub fn resize(&mut self, fft_size: usize) {
        if fft_size == self.fft_size { return; }
        *self = Self::new(fft_size);
    }

    /// Compute cepstrum from the half-spectrum slice. Reads `bins[..fft_size/2+1]`,
    /// writes into `self.cepstrum[..fft_size]`. RT-safe: no heap allocation.
    pub fn compute_from_bins(&mut self, bins: &[Complex<f32>]) {
        let num_bins = self.fft_size / 2 + 1;
        debug_assert!(bins.len() >= num_bins);
        for k in 0..num_bins {
            let mag_sq = bins[k].norm_sqr();
            let log_v  = 0.5 * (mag_sq + LOG_EPSILON_SQ).ln();
            self.log_mag[k] = Complex::new(log_v, 0.0);
        }
        let _ = self.inv_fft.process_with_scratch(
            &mut self.log_mag,
            &mut self.cepstrum,
            &mut self.scratch,
        );
    }

    /// Borrow the cepstrum output. Length = `fft_size`.
    #[inline]
    pub fn quefrency(&self) -> &[f32] {
        &self.cepstrum
    }

    /// Current FFT size — used by Pipeline::reset() to detect a size change.
    #[inline]
    pub fn fft_size(&self) -> usize {
        self.fft_size
    }
}
