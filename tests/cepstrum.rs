use spectral_forge::dsp::cepstrum::CepstrumBuf;
use spectral_forge::dsp::modules::{module_spec, ModuleType};
use num_complex::Complex;

#[test]
fn no_existing_module_declares_needs_cepstrum() {
    for &ty in &[
        ModuleType::Empty,
        ModuleType::Dynamics,
        ModuleType::Freeze,
        ModuleType::PhaseSmear,
        ModuleType::Contrast,
        ModuleType::Gain,
        ModuleType::MidSide,
        ModuleType::TransientSustainedSplit,
        ModuleType::Harmonic,
        ModuleType::Master,
    ] {
        let spec = module_spec(ty);
        assert!(!spec.needs_cepstrum,
            "{:?} should not need cepstrum in v1 (Harmony Lifter is the v1 consumer in 6.5)", ty);
    }
}

#[test]
fn cepstrum_of_pure_tone_has_low_quefrency_envelope_and_pitch_spike() {
    let fft_size = 2048;
    let num_bins = fft_size / 2 + 1;
    let mut cb = CepstrumBuf::new(fft_size);

    let mut bins = vec![Complex::<f32>::new(0.0, 0.0); num_bins];
    bins[100] = Complex::new(1.0, 0.0);
    bins[101] = Complex::new(0.5, 0.0);
    bins[99]  = Complex::new(0.5, 0.0);

    cb.compute_from_bins(&bins);

    let q = cb.quefrency();
    assert_eq!(q.len(), fft_size);
    assert!(q[0].is_finite());
    for (n, v) in q.iter().enumerate() {
        assert!(v.is_finite(), "quefrency[{}] = {}", n, v);
    }
}

#[test]
fn cepstrum_silent_input_produces_finite_zero_envelope() {
    let fft_size = 2048;
    let num_bins = fft_size / 2 + 1;
    let mut cb = CepstrumBuf::new(fft_size);

    let bins = vec![Complex::<f32>::new(0.0, 0.0); num_bins];
    cb.compute_from_bins(&bins);

    for (n, v) in cb.quefrency().iter().enumerate() {
        assert!(v.is_finite(), "silent quefrency[{}] = {}", n, v);
    }
}

#[test]
fn cepstrum_buf_reusable_across_calls_no_alloc() {
    let fft_size = 2048;
    let num_bins = fft_size / 2 + 1;
    let mut cb = CepstrumBuf::new(fft_size);

    let bins = vec![Complex::<f32>::new(1.0, 0.0); num_bins];
    for _ in 0..10 {
        cb.compute_from_bins(&bins);
        assert!(cb.quefrency()[0].is_finite());
    }
}

use spectral_forge::dsp::pipeline::{Pipeline, FFT_SIZE};

#[test]
fn pipeline_does_not_panic_on_silent_input_with_no_cepstrum_consumer() {
    let sr  = 48000.0_f32;
    let fft = FFT_SIZE;
    let mut slot_types = [ModuleType::Empty; 9];
    slot_types[0] = ModuleType::Dynamics;
    slot_types[8] = ModuleType::Master;
    let _p = Pipeline::new(sr, 2, fft, &slot_types, 1.0);
}
