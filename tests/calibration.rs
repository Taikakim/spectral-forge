#[test]
fn history_probe_fills_under_realistic_load() {
    use spectral_forge::dsp::history_buffer::HistoryBuffer;
    // Direct buffer probe (the calibration harness drives modules, not Pipeline
    // construction). We verify the same numbers Pipeline::history_probe would
    // surface, by directly building a buffer with the same shape Pipeline would.
    let mut h = HistoryBuffer::new(2, 50, 1025);
    for hop in 0..200 {
        use num_complex::Complex;
        let mag = (hop as f32 / 200.0).sin().abs();
        let frame: Vec<Complex<f32>> = (0..1025).map(|k| {
            Complex::from_polar(mag * (k as f32 + 1.0).recip(), 0.0)
        }).collect();
        h.write_hop(0, &frame);
        h.write_hop(1, &frame);
        h.advance_after_all_channels_written();
    }
    assert_eq!(h.frames_used(), 50, "probe: frames_used must saturate at capacity");
    let decay_max = {
        let decay = h.summary_decay_estimate(0);
        decay.iter().cloned().fold(0.0f32, f32::max)
    };
    let rms_max = {
        let rms = h.summary_rms_envelope(0);
        rms.iter().cloned().fold(0.0f32, f32::max)
    };
    let stab_max = {
        let stab = h.summary_if_stability(0);
        stab.iter().cloned().fold(0.0f32, f32::max)
    };
    assert!(decay_max.is_finite() && decay_max <= 1000.0);
    assert!(rms_max.is_finite()   && rms_max   <= 10.0);
    assert!(stab_max.is_finite()  && stab_max  <= 1.0 + 1e-6);
}
