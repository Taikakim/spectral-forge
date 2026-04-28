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

#[test]
fn past_probe_reports_active_mode_after_dispatch() {
    use spectral_forge::dsp::history_buffer::HistoryBuffer;
    use spectral_forge::dsp::modules::past::{PastModule, PastMode};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule};
    use spectral_forge::params::{FxChannelTarget, StereoLink};
    use num_complex::Complex;

    // 1 channel × capacity 16 frames × 8 bins. Pre-fill 8 frames so that
    // frames_used == 8 (still under capacity), proving the probe reports
    // the live count, not a saturated capacity value.
    let mut h = HistoryBuffer::new(1, 16, 8);
    for _ in 0..8 {
        h.write_hop(0, &vec![Complex::new(0.5, 0.0); 8]);
        h.advance_after_all_channels_written();
    }
    let mut m = PastModule::new(48000.0, 2048);
    m.set_mode(PastMode::Reverse);

    let amount    = vec![0.7_f32; 8];
    let time      = vec![0.3_f32; 8];
    let threshold = vec![0.0_f32; 8];
    let spread    = vec![0.0_f32; 8];
    let mix       = vec![1.0_f32; 8];
    let curves: Vec<&[f32]> = vec![&amount, &time, &threshold, &spread, &mix];

    let mut bins = vec![Complex::new(1.0, 0.0); 8];
    let mut supp = vec![0.0_f32; 8];
    let mut ctx = ModuleContext::new(48000.0, 2048, 8, 10.0, 100.0, 1.0, 0.5, false, false);
    ctx.history = Some(&h);
    m.process(0, StereoLink::Linked, FxChannelTarget::All,
              &mut bins, None, &curves, &mut supp, None, &ctx);

    let snap = m.last_probe();
    assert_eq!(snap.past_active_mode_idx, Some(PastMode::Reverse as u8));
    assert!((snap.past_amount_pct.unwrap() - 70.0).abs() < 1e-3);
    assert_eq!(snap.past_history_frames_used, Some(8));
}
