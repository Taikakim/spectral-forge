use spectral_forge::dsp::harmonic_groups::{HarmonicGroupBuf, MAX_GROUPS, HARMONIC_TOLERANCE_CENTS};
use num_complex::Complex;

#[test]
fn detects_simple_harmonic_series_at_220hz() {
    // Build a spectrum with peaks at 220, 440, 660, 880, 1100 Hz (5 harmonics of 220).
    let sample_rate = 48000.0_f32;
    let fft_size    = 2048_usize;
    let num_bins    = fft_size / 2 + 1;
    let bin_freq_hz = sample_rate / fft_size as f32;

    let mut bins  = vec![Complex::<f32>::new(0.0, 0.0); num_bins];
    let mut ifreq = vec![0.0_f32; num_bins];
    for n in 1..=5 {
        let f = 220.0 * n as f32;
        let k = (f / bin_freq_hz).round() as usize;
        bins[k]  = Complex::new(1.0 / n as f32, 0.0); // 1/n decay typical for sawtooth
        ifreq[k] = f;
    }

    let mut buf = HarmonicGroupBuf::new();
    buf.detect(&bins, &ifreq, sample_rate, fft_size);
    let groups = buf.groups();
    assert!(!groups.is_empty(), "should detect at least one group");
    let g0 = &groups[0];
    assert!((g0.fundamental_hz - 220.0).abs() < 5.0,
        "fundamental should be ~220 Hz, got {} Hz", g0.fundamental_hz);
    assert!(g0.harmonic_count >= 4,
        "should detect ≥4 harmonics of 220, found {}", g0.harmonic_count);
}

#[test]
fn no_groups_on_silent_input() {
    let sample_rate = 48000.0;
    let fft_size    = 2048;
    let num_bins    = fft_size / 2 + 1;
    let bins  = vec![Complex::<f32>::new(0.0, 0.0); num_bins];
    let ifreq = vec![0.0_f32; num_bins];
    let mut buf = HarmonicGroupBuf::new();
    buf.detect(&bins, &ifreq, sample_rate, fft_size);
    assert!(buf.groups().is_empty(), "silent input should produce no groups");
}

#[test]
fn at_most_max_groups_returned() {
    // Stress test: 10 distinct fundamentals (220, 277, 330, ...) — only the top MAX_GROUPS survive.
    let sample_rate = 48000.0_f32;
    let fft_size    = 2048_usize;
    let num_bins    = fft_size / 2 + 1;
    let bin_freq_hz = sample_rate / fft_size as f32;

    let mut bins  = vec![Complex::<f32>::new(0.0, 0.0); num_bins];
    let mut ifreq = vec![0.0_f32; num_bins];
    let fundamentals = [220.0, 247.0, 277.0, 311.0, 330.0, 370.0, 415.0, 466.0, 523.0, 587.0];
    for f0 in fundamentals.iter() {
        for n in 1..=4 {
            let f = f0 * n as f32;
            if f >= 5000.0 { break; }
            let k = (f / bin_freq_hz).round() as usize;
            bins[k]  = Complex::new(1.0 / n as f32, 0.0);
            ifreq[k] = f;
        }
    }
    let mut buf = HarmonicGroupBuf::new();
    buf.detect(&bins, &ifreq, sample_rate, fft_size);
    assert!(buf.groups().len() <= MAX_GROUPS,
        "should return at most MAX_GROUPS = {}, got {}", MAX_GROUPS, buf.groups().len());
}

#[test]
fn tolerance_constant_in_cents_is_reasonable() {
    // Sanity: HARMONIC_TOLERANCE_CENTS ≤ 50 (a quarter-tone).
    assert!(HARMONIC_TOLERANCE_CENTS <= 50.0);
    assert!(HARMONIC_TOLERANCE_CENTS >= 10.0);
}
