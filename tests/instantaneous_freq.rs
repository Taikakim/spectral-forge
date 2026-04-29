use spectral_forge::dsp::instantaneous_freq::principal_argument;
use std::f32::consts::PI;

#[test]
fn principal_argument_wraps_above_pi_to_negative_range() {
    let result = principal_argument(1.5 * PI);
    assert!((result - (-0.5 * PI)).abs() < 1e-5,
        "expected -0.5π, got {}", result);
}

#[test]
fn principal_argument_wraps_below_minus_pi_to_positive_range() {
    let result = principal_argument(-1.5 * PI);
    assert!((result - 0.5 * PI).abs() < 1e-5,
        "expected 0.5π, got {}", result);
}

#[test]
fn principal_argument_passthrough_in_range() {
    for x in [-PI + 0.01, 0.0, 0.5 * PI, PI - 0.01].iter() {
        assert!((principal_argument(*x) - *x).abs() < 1e-6);
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Task 2 tests
// ──────────────────────────────────────────────────────────────────────────────

use spectral_forge::dsp::instantaneous_freq::compute_instantaneous_freq;

#[test]
fn if_pure_tone_at_1khz_resolves_to_1khz() {
    let sample_rate = 48000.0_f32;
    let fft_size    = 2048_usize;
    let hop_size    = fft_size / 4;
    let num_bins    = fft_size / 2 + 1;
    let freq_hz     = 1000.0_f32;
    let bin_freq_hz = sample_rate / fft_size as f32;
    let target_bin  = (freq_hz / bin_freq_hz).round() as usize;
    let frame_n_seconds      = (hop_size as f32) / sample_rate;
    // Phases come from the FFT in (-π, π], so wrap before storing — keeps the test
    // representative of real Pipeline data and ensures the deviation arithmetic is
    // exercised end-to-end (the prior un-wrapped phase only worked by coincidence).
    let prev_phase_at_target = principal_argument(0.0_f32);
    let curr_phase_at_target = principal_argument(2.0 * std::f32::consts::PI * freq_hz * frame_n_seconds);
    let mut prev = vec![0.0_f32; num_bins];
    let mut curr = vec![0.0_f32; num_bins];
    prev[target_bin] = prev_phase_at_target;
    curr[target_bin] = curr_phase_at_target;
    let mut if_out = vec![0.0_f32; num_bins];
    compute_instantaneous_freq(&prev, &curr, &mut if_out, sample_rate, hop_size, fft_size);
    let if_at_target = if_out[target_bin];
    assert!((if_at_target - freq_hz).abs() < 1.0,
        "expected ~1000 Hz at bin {}, got {} Hz", target_bin, if_at_target);
}

#[test]
fn if_at_bin_zero_is_dc() {
    let sample_rate = 48000.0;
    let fft_size = 2048;
    let hop_size = fft_size / 4;
    let num_bins = fft_size / 2 + 1;
    let prev = vec![0.0_f32; num_bins];
    let curr = vec![0.0_f32; num_bins];
    let mut if_out = vec![1.0_f32; num_bins];
    compute_instantaneous_freq(&prev, &curr, &mut if_out, sample_rate, hop_size, fft_size);
    assert_eq!(if_out[0], 0.0, "DC bin must always report 0 Hz");
}

#[test]
fn if_finite_for_all_bins_under_random_phase() {
    use spectral_forge::dsp::utils::xorshift64;
    let sample_rate = 48000.0;
    let fft_size = 2048;
    let hop_size = 512;
    let num_bins = fft_size / 2 + 1;
    let mut state = 0xDEADBEEFu64;
    let mut prev = vec![0.0_f32; num_bins];
    let mut curr = vec![0.0_f32; num_bins];
    for k in 0..num_bins {
        prev[k] = (xorshift64(&mut state) as f32 / u64::MAX as f32) * std::f32::consts::TAU - std::f32::consts::PI;
        curr[k] = (xorshift64(&mut state) as f32 / u64::MAX as f32) * std::f32::consts::TAU - std::f32::consts::PI;
    }
    let mut if_out = vec![0.0_f32; num_bins];
    compute_instantaneous_freq(&prev, &curr, &mut if_out, sample_rate, hop_size, fft_size);
    for (k, v) in if_out.iter().enumerate() {
        assert!(v.is_finite(), "bin {} produced non-finite IF: {}", k, v);
    }
}
