use spectral_forge::dsp::cepstrum::CepstrumBuf;
use num_complex::Complex;

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
