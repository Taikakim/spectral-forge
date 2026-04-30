// tests/chromagram.rs

use spectral_forge::dsp::chromagram::{compute_chromagram, NUM_PITCH_CLASSES};
use num_complex::Complex;

#[test]
fn chromagram_a440_lights_pitch_class_a() {
    // A440 should land at pitch class 9 (A) when MIDI A4 = 69, 69 % 12 = 9.
    let sample_rate = 48000.0_f32;
    let fft_size    = 2048_usize;
    let num_bins    = fft_size / 2 + 1;

    let bin_freq_hz = sample_rate / fft_size as f32;
    let bin_a       = (440.0_f32 / bin_freq_hz).round() as usize;

    let mut bins = vec![Complex::<f32>::new(0.0, 0.0); num_bins];
    bins[bin_a] = Complex::new(1.0, 0.0);
    let mut ifreq = vec![0.0_f32; num_bins];
    ifreq[bin_a] = 440.0;

    let mut chroma = [0.0_f32; NUM_PITCH_CLASSES];
    compute_chromagram(&bins, Some(&ifreq), sample_rate, fft_size, &mut chroma);

    let max_idx = chroma.iter().enumerate().max_by(|a, b| a.1.partial_cmp(b.1).unwrap()).unwrap().0;
    assert_eq!(max_idx, 9, "A440 should land on pitch class 9 (A); got class {} = {:?}", max_idx, chroma);
}

#[test]
fn chromagram_normalizes_to_unit_sum_when_nonzero() {
    let sample_rate = 48000.0;
    let fft_size    = 2048;
    let num_bins    = fft_size / 2 + 1;
    let bin_freq_hz = sample_rate / fft_size as f32;

    let bin_a = (440.0_f32 / bin_freq_hz).round() as usize;
    let bin_e = (660.0_f32 / bin_freq_hz).round() as usize;
    let mut bins = vec![Complex::<f32>::new(0.0, 0.0); num_bins];
    bins[bin_a] = Complex::new(1.0, 0.0);
    bins[bin_e] = Complex::new(0.5, 0.0);

    let mut chroma = [0.0_f32; NUM_PITCH_CLASSES];
    compute_chromagram(&bins, None, sample_rate, fft_size, &mut chroma);

    let sum: f32 = chroma.iter().sum();
    assert!((sum - 1.0).abs() < 1e-3, "chromagram sum should be ~1.0, got {}", sum);
}

#[test]
fn chromagram_silent_input_is_all_zero() {
    let sample_rate = 48000.0;
    let fft_size    = 2048;
    let num_bins    = fft_size / 2 + 1;
    let bins = vec![Complex::<f32>::new(0.0, 0.0); num_bins];
    let mut chroma = [9.9_f32; NUM_PITCH_CLASSES];
    compute_chromagram(&bins, None, sample_rate, fft_size, &mut chroma);
    assert!(chroma.iter().all(|&c| c == 0.0), "silent input must produce all-zero chromagram, got {:?}", chroma);
}

#[test]
fn chromagram_rejects_subsonic_bins() {
    let sample_rate = 48000.0;
    let fft_size    = 2048;
    let num_bins    = fft_size / 2 + 1;
    let bin_freq_hz = sample_rate / fft_size as f32;
    let bin_low = (50.0_f32 / bin_freq_hz).round() as usize; // below 80 Hz cutoff
    let mut bins = vec![Complex::<f32>::new(0.0, 0.0); num_bins];
    bins[bin_low] = Complex::new(1.0, 0.0);
    let mut chroma = [0.0_f32; NUM_PITCH_CLASSES];
    compute_chromagram(&bins, None, sample_rate, fft_size, &mut chroma);
    assert!(chroma.iter().all(|&c| c == 0.0), "subsonic-only input must produce all-zero chromagram");
}
