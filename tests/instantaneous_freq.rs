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

// ──────────────────────────────────────────────────────────────────────────────
// Task 3 tests
// ──────────────────────────────────────────────────────────────────────────────

use spectral_forge::dsp::modules::{module_spec, ModuleType};

#[test]
fn only_approved_modules_declare_needs_if() {
    use ModuleType::*;
    // Exhaustive match: when a new ModuleType variant lands the compiler will
    // refuse to build this test until the new variant is added below, forcing
    // an explicit decision about whether the new module reads IF.
    // Phase 6.5 Task 5: Harmony is approved to use IF (HarmonicGenerator mode).
    let all: &[ModuleType] = &[
        Empty, Dynamics, Freeze, PhaseSmear, Contrast, Gain, MidSide,
        TransientSustainedSplit, Harmonic, Future, Punch, Rhythm, Geometry,
        Modulate, Circuit, Life, Past, Kinetics, Harmony, Master,
    ];
    for &ty in all {
        let _exhaustive_guard: () = match ty {
            Empty | Dynamics | Freeze | PhaseSmear | Contrast | Gain | MidSide
            | TransientSustainedSplit | Harmonic | Future | Punch | Rhythm
            | Geometry | Modulate | Circuit | Life | Past | Kinetics | Harmony | Master => (),
        };
        let spec = module_spec(ty);
        let approved_for_if = matches!(ty, Harmony | Modulate);
        if approved_for_if {
            assert!(spec.needs_instantaneous_freq,
                "{:?} is approved for IF but spec has needs_instantaneous_freq=false", ty);
        } else {
            assert!(!spec.needs_instantaneous_freq,
                "{:?} should not need IF (add to approved_for_if if intentional)", ty);
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Task 8 tests
// ──────────────────────────────────────────────────────────────────────────────

#[test]
fn if_kernel_independent_per_channel_invocation() {
    // Two simulated channels with different inputs. The kernel is stateless
    // per call; this test pins per-call separation — neither channel's IF
    // should be influenced by the other's data.
    let sample_rate = 48000.0_f32;
    let fft_size    = 2048_usize;
    let hop_size    = fft_size / 4;
    let num_bins    = fft_size / 2 + 1;
    let bin_freq_hz = sample_rate / fft_size as f32;
    // Off-bin-center frequencies — bin_l and bin_r are the *nearest* bins,
    // which makes the test exercise the phase-deviation arithmetic rather
    // than just bin-center alignment.
    let freq_l      = 1000.0_f32;
    let freq_r      = 2000.0_f32;
    let bin_l       = (freq_l / bin_freq_hz).round() as usize;
    let bin_r       = (freq_r / bin_freq_hz).round() as usize;
    let frame_n_seconds = (hop_size as f32) / sample_rate;

    // L: freq_l tone at bin_l; R: freq_r tone at bin_r. Wrap phases to (-π, π]
    // to mirror real FFT output (matches the convention of the other tests
    // in this file).
    let mut prev_l = vec![0.0_f32; num_bins];
    let mut prev_r = vec![0.0_f32; num_bins];
    let mut curr_l = vec![0.0_f32; num_bins];
    let mut curr_r = vec![0.0_f32; num_bins];
    prev_l[bin_l] = principal_argument(0.0);
    prev_r[bin_r] = principal_argument(0.0);
    curr_l[bin_l] = principal_argument(2.0 * std::f32::consts::PI * freq_l * frame_n_seconds);
    curr_r[bin_r] = principal_argument(2.0 * std::f32::consts::PI * freq_r * frame_n_seconds);

    let mut if_l = vec![0.0_f32; num_bins];
    let mut if_r = vec![0.0_f32; num_bins];
    compute_instantaneous_freq(&prev_l, &curr_l, &mut if_l, sample_rate, hop_size, fft_size);
    compute_instantaneous_freq(&prev_r, &curr_r, &mut if_r, sample_rate, hop_size, fft_size);

    // Each channel resolves its own tone correctly.
    assert!((if_l[bin_l] - 1000.0).abs() < 1.0,
        "L channel must report ~1 kHz at bin {}; got {} Hz", bin_l, if_l[bin_l]);
    assert!((if_r[bin_r] - 2000.0).abs() < 1.0,
        "R channel must report ~2 kHz at bin {}; got {} Hz", bin_r, if_r[bin_r]);

    // Cross-channel non-bleed: the L IF array at bin_r (which received zero
    // input on the L channel) must equal the silent-bin baseline, not the
    // R channel's resolved 2 kHz. Compute that baseline independently:
    // IF[k] = k*bin_freq_hz + principal_argument(-TAU*k*hop/fft) * sr/(hop*TAU)
    let hop_per_fft = hop_size as f32 / fft_size as f32;
    let radians_to_hz = sample_rate / (hop_size as f32 * 2.0 * std::f32::consts::PI);
    let silent_bin_if = |k: usize| {
        let dev = principal_argument(-(2.0 * std::f32::consts::PI) * k as f32 * hop_per_fft);
        k as f32 * bin_freq_hz + dev * radians_to_hz
    };
    let expected_l_at_bin_r = silent_bin_if(bin_r);
    let expected_r_at_bin_l = silent_bin_if(bin_l);
    assert!((if_l[bin_r] - expected_l_at_bin_r).abs() < 1e-2,
        "L channel at bin_r must equal silent-bin baseline ({} Hz), not R's tone; got {} Hz",
        expected_l_at_bin_r, if_l[bin_r]);
    assert!((if_r[bin_l] - expected_r_at_bin_l).abs() < 1e-2,
        "R channel at bin_l must equal silent-bin baseline ({} Hz), not L's tone; got {} Hz",
        expected_r_at_bin_l, if_r[bin_l]);
}
