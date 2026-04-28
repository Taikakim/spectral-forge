use num_complex::Complex;
use spectral_forge::dsp::phase::PhaseRotator;
use spectral_forge::dsp::modules::ModuleContext;

#[test]
fn phase_rotator_zero_delta_is_identity() {
    let r = PhaseRotator::new();
    let c = Complex::new(1.0_f32, 0.0);
    let out = r.rotate(c, 0.0, 0.0);
    assert!((out.re - 1.0).abs() < 1e-5);
    assert!((out.im - 0.0).abs() < 1e-5);
}

#[test]
fn phase_rotator_pi_rotation_negates() {
    let r = PhaseRotator::new();
    // Rotation amount = freq_offset * time_delta * 2π. Pick freq_offset=0.5, time_delta=1.0
    // → rotation = π → multiplies by exp(iπ) = -1.
    let c = Complex::new(2.0_f32, 0.0);
    let out = r.rotate(c, 0.5, 1.0);
    assert!((out.re - -2.0).abs() < 1e-3, "expected ~-2.0, got {}", out.re);
    assert!(out.im.abs() < 1e-3, "imaginary should be ~0, got {}", out.im);
}

#[test]
fn phase_rotator_lut_quantum_under_threshold() {
    // 1024-entry LUT covers 2π with floor-truncation, so the worst-case
    // angular offset before a LUT step advances is one full step:
    // 2π/1024 ≈ 0.006135 rad. Max sin/cos error is therefore ≈ 0.006135.
    // Probe at i + 0.5 — the midpoint between LUT entries — to actually
    // exercise truncation quantization. Tolerance 0.007 covers the bound
    // with a small margin.
    let r = PhaseRotator::new();
    let c = Complex::new(1.0_f32, 0.0);
    for i in 0..1024 {
        let theta = (i as f32 + 0.5) / 1024.0;
        let out = r.rotate(c, theta, 1.0);
        let expected = Complex::from_polar(1.0, theta * std::f32::consts::TAU);
        assert!((out.re - expected.re).abs() < 0.007, "entry {} re error", i);
        assert!((out.im - expected.im).abs() < 0.007, "entry {} im error", i);
    }
}

#[test]
fn module_context_has_if_offset_field() {
    // Field exists and is Option<&[f32]>. We don't assert its content here —
    // that's Pipeline's responsibility, exercised in tests/past_pipeline.rs.
    let ctx = ModuleContext::new(
        48000.0, 2048, 1025, 10.0, 100.0, 1.0, 0.5, false, false,
    );
    assert!(ctx.if_offset.is_none());
}

#[test]
fn past_module_constructs_and_reports_curves() {
    use spectral_forge::dsp::modules::{create_module, ModuleType};
    let m = create_module(ModuleType::Past, 48000.0, 2048);
    assert_eq!(m.module_type(), ModuleType::Past);
    assert_eq!(m.num_curves(), 5);
    assert_eq!(m.tail_length(), 0);
}

#[test]
fn granular_replaces_bin_with_history_at_offset_when_amount_high() {
    use spectral_forge::dsp::history_buffer::HistoryBuffer;
    use spectral_forge::dsp::modules::past::{PastModule, PastMode};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    // History has bin 100 = magnitude 5.0 at age 8 frames.
    let mut h = HistoryBuffer::new(1, 64, 256);
    for i in 0..16 {
        let mut frame = vec![Complex::new(0.0, 0.0); 256];
        if i == 7 { frame[100] = Complex::new(5.0, 0.0); }
        h.write_hop(0, &frame);
        h.advance_after_all_channels_written();
    }

    let mut m = PastModule::new(48000.0, 2048);
    m.set_mode(PastMode::Granular);

    let mut bins = vec![Complex::new(1.0, 0.0); 256];
    let amount    = vec![1.0_f32; 256];
    let time      = vec![8.0 / 64.0; 256];     // map → age 8 frames
    let threshold = vec![0.0_f32; 256];
    let spread    = vec![0.0_f32; 256];
    let mix       = vec![1.0_f32; 256];
    let curves: Vec<&[f32]> = vec![&amount, &time, &threshold, &spread, &mix];
    let mut supp = vec![0.0_f32; 256];
    let mut ctx = ModuleContext::new(48000.0, 2048, 256, 10.0, 100.0, 1.0, 0.5, false, false);
    ctx.history = Some(&h);

    // Note: process() now takes physics: Option<&mut BinPhysics> between supp and ctx.
    m.process(0, StereoLink::Linked, FxChannelTarget::All,
              &mut bins, None, &curves, &mut supp, None, &ctx);

    assert!((bins[100].re - 5.0).abs() < 0.5,
        "expected bin[100] re ≈ 5.0, got {}", bins[100].re);
}

#[test]
fn decay_sorter_long_ringing_bin_lands_at_low_output_bin() {
    use spectral_forge::dsp::history_buffer::HistoryBuffer;
    use spectral_forge::dsp::modules::past::{PastModule, PastMode, SortKey};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    // Build history where bin 200 = stable 1.0 for 40 frames (long decay)
    // and bin 100 = single spike at frame 0 (fast decay).
    let mut h = HistoryBuffer::new(1, 64, 256);
    for i in 0..40 {
        let mut frame = vec![Complex::new(0.0, 0.0); 256];
        if i == 0 { frame[100] = Complex::new(1.0, 0.0); }
        frame[200] = Complex::new(1.0, 0.0);
        h.write_hop(0, &frame);
        h.advance_after_all_channels_written();
    }

    let mut m = PastModule::new(48000.0, 2048);
    m.set_mode(PastMode::DecaySorter);
    m.set_sort_key(SortKey::Decay);

    let mut bins = vec![Complex::new(0.0, 0.0); 256];
    bins[100] = Complex::new(0.7, 0.0);
    bins[200] = Complex::new(0.9, 0.0);

    let amount    = vec![1.0_f32; 256];
    let time      = vec![0.0_f32; 256];
    let threshold = vec![0.05_f32; 256];
    let spread    = vec![0.0_f32; 256];
    let mix       = vec![1.0_f32; 256];
    let curves: Vec<&[f32]> = vec![&amount, &time, &threshold, &spread, &mix];
    let mut supp = vec![0.0_f32; 256];
    let mut ctx = ModuleContext::new(48000.0, 2048, 256, 10.0, 100.0, 1.0, 0.5, false, false);
    ctx.history = Some(&h);

    // Note: process() takes physics: Option<&mut BinPhysics> between supp and ctx.
    m.process(0, StereoLink::Linked, FxChannelTarget::All,
              &mut bins, None, &curves, &mut supp, None, &ctx);

    assert!(bins[10].norm() > 0.5,
        "lowest output slot should hold long-ringing partial, got {}", bins[10].norm());
    assert!(bins[200].norm() < 0.2,
        "bin 200 should have been moved out, got {}", bins[200].norm());
}

#[test]
fn convolution_amplifies_when_history_aligns() {
    use spectral_forge::dsp::history_buffer::HistoryBuffer;
    use spectral_forge::dsp::modules::past::{PastModule, PastMode};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let mut h = HistoryBuffer::new(1, 32, 256);
    for i in 0..16 {
        let mut frame = vec![Complex::new(0.0, 0.0); 256];
        if i == 11 { frame[50] = Complex::new(2.0, 0.0); }
        h.write_hop(0, &frame);
        h.advance_after_all_channels_written();
    }
    let mut m = PastModule::new(48000.0, 2048);
    m.set_mode(PastMode::Convolution);

    let mut bins = vec![Complex::new(0.0, 0.0); 256];
    bins[50] = Complex::new(3.0, 0.0);

    let amount    = vec![1.0_f32; 256];
    let time      = vec![4.0 / 32.0; 256];
    let threshold = vec![0.0_f32; 256];
    let spread    = vec![0.0_f32; 256];
    let mix       = vec![1.0_f32; 256];
    let curves: Vec<&[f32]> = vec![&amount, &time, &threshold, &spread, &mix];
    let mut supp = vec![0.0_f32; 256];
    let mut ctx = ModuleContext::new(48000.0, 2048, 256, 10.0, 100.0, 1.0, 0.5, false, false);
    ctx.history = Some(&h);

    // Note: process() takes physics: Option<&mut BinPhysics> between supp and ctx.
    m.process(0, StereoLink::Linked, FxChannelTarget::All,
              &mut bins, None, &curves, &mut supp, None, &ctx);

    // current 3.0 * historic 2.0 = 6.0 at AMOUNT=1, MIX=1.
    assert!((bins[50].re - 6.0).abs() < 0.5,
        "expected ~6.0, got {}", bins[50].re);
}

#[test]
fn reverse_reads_backward_through_history() {
    use spectral_forge::dsp::history_buffer::HistoryBuffer;
    use spectral_forge::dsp::modules::past::{PastModule, PastMode};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let mut h = HistoryBuffer::new(1, 32, 256);
    for i in 0..16 {
        let mut frame = vec![Complex::new(0.0, 0.0); 256];
        frame[30] = Complex::new(i as f32 + 1.0, 0.0);
        h.write_hop(0, &frame);
        h.advance_after_all_channels_written();
    }

    let mut m = PastModule::new(48000.0, 2048);
    m.set_mode(PastMode::Reverse);

    let amount    = vec![1.0_f32; 256];
    let time      = vec![0.5_f32; 256];
    let threshold = vec![0.0_f32; 256];
    let spread    = vec![0.0_f32; 256];
    let mix       = vec![1.0_f32; 256];
    let curves: Vec<&[f32]> = vec![&amount, &time, &threshold, &spread, &mix];

    let mut readings: Vec<f32> = Vec::new();
    for _ in 0..3 {
        let mut bins = vec![Complex::new(0.0, 0.0); 256];
        let mut supp = vec![0.0_f32; 256];
        let mut ctx = ModuleContext::new(48000.0, 2048, 256, 10.0, 100.0, 1.0, 0.5, false, false);
        ctx.history = Some(&h);
        // Note: process() takes physics: Option<&mut BinPhysics> between supp and ctx.
        m.process(0, StereoLink::Linked, FxChannelTarget::All,
                  &mut bins, None, &curves, &mut supp, None, &ctx);
        readings.push(bins[30].re);
    }
    // Most-recent frame magnitude is 16.0 (i=15, value i+1=16). Backward order:
    // hop 0 reads age 0 (16), hop 1 reads age 1 (15), hop 2 reads age 2 (14).
    assert!((readings[0] - 16.0).abs() < 0.5, "hop 0 = {}", readings[0]);
    assert!((readings[1] - 15.0).abs() < 0.5, "hop 1 = {}", readings[1]);
    assert!((readings[2] - 14.0).abs() < 0.5, "hop 2 = {}", readings[2]);
}

#[test]
fn stretch_at_unity_rate_returns_recent_history() {
    use spectral_forge::dsp::history_buffer::HistoryBuffer;
    use spectral_forge::dsp::modules::past::{PastModule, PastMode};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let mut h = HistoryBuffer::new(1, 32, 256);
    for _ in 0..32 {
        let mut frame = vec![Complex::new(0.0, 0.0); 256];
        frame[80] = Complex::new(2.0, 0.0);
        h.write_hop(0, &frame);
        h.advance_after_all_channels_written();
    }

    let mut m = PastModule::new(48000.0, 2048);
    m.set_mode(PastMode::Stretch);

    let mut bins = vec![Complex::new(0.0, 0.0); 256];
    let amount    = vec![1.0_f32; 256];
    let time      = vec![0.5_f32; 256];   // unity rate
    let threshold = vec![0.0_f32; 256];
    let spread    = vec![0.0_f32; 256];
    let mix       = vec![1.0_f32; 256];
    let curves: Vec<&[f32]> = vec![&amount, &time, &threshold, &spread, &mix];
    let mut supp = vec![0.0_f32; 256];
    let mut ctx = ModuleContext::new(48000.0, 2048, 256, 10.0, 100.0, 1.0, 0.5, false, false);
    ctx.history = Some(&h);

    m.process(0, StereoLink::Linked, FxChannelTarget::All,
              &mut bins, None, &curves, &mut supp, None, &ctx);
    assert!((bins[80].norm() - 2.0).abs() < 0.5,
        "expected ~2.0 at unity rate, got {}", bins[80].norm());
}

#[test]
fn stretch_at_half_rate_advances_read_phase_slowly() {
    use spectral_forge::dsp::history_buffer::HistoryBuffer;
    use spectral_forge::dsp::modules::past::{PastModule, PastMode};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let mut h = HistoryBuffer::new(1, 32, 256);
    for i in 0..32 {
        let mut frame = vec![Complex::new(0.0, 0.0); 256];
        frame[80] = Complex::new(i as f32, 0.0);
        h.write_hop(0, &frame);
        h.advance_after_all_channels_written();
    }
    let mut m = PastModule::new(48000.0, 2048);
    m.set_mode(PastMode::Stretch);

    let amount    = vec![1.0_f32; 256];
    let time      = vec![0.25_f32; 256];  // < 0.5 → < 1.0× rate (slower)
    let threshold = vec![0.0_f32; 256];
    let spread    = vec![0.0_f32; 256];
    let mix       = vec![1.0_f32; 256];
    let curves: Vec<&[f32]> = vec![&amount, &time, &threshold, &spread, &mix];

    let mut readings: Vec<f32> = Vec::new();
    for _ in 0..4 {
        let mut bins = vec![Complex::new(0.0, 0.0); 256];
        let mut supp = vec![0.0_f32; 256];
        let mut ctx = ModuleContext::new(48000.0, 2048, 256, 10.0, 100.0, 1.0, 0.5, false, false);
        ctx.history = Some(&h);
        m.process(0, StereoLink::Linked, FxChannelTarget::All,
                  &mut bins, None, &curves, &mut supp, None, &ctx);
        readings.push(bins[80].norm());
    }
    // Sanity: kernel must actually run. With bins seeded to 0 and MIX=1, a no-op
    // implementation would leave readings all zero, vacuously satisfying delta<4.
    assert!(readings[0] > 0.5,
        "kernel did not produce output at hop 0; readings[0] = {}", readings[0]);
    // Slower rate ⇒ adjacent readings should be closer together than at unity rate.
    let delta = (readings[3] - readings[0]).abs();
    assert!(delta < 4.0, "read_phase should advance slowly at half rate, delta {}", delta);
}

#[test]
fn set_past_mode_changes_dispatch() {
    use spectral_forge::dsp::modules::past::{PastModule, PastMode, SortKey};
    use spectral_forge::dsp::modules::SpectralModule;
    let mut m = PastModule::new(48000.0, 2048);
    // Exercise set_past_mode via the trait method — should not panic.
    m.set_past_mode(PastMode::Reverse);
    m.set_past_mode(PastMode::Stretch);
    m.set_past_mode(PastMode::Granular);
    m.set_past_mode(PastMode::DecaySorter);
    m.set_past_mode(PastMode::Convolution);
    // Exercise set_past_sort_key via the trait method — should not panic.
    m.set_past_sort_key(SortKey::Stability);
    m.set_past_sort_key(SortKey::Area);
    m.set_past_sort_key(SortKey::Decay);
}
