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
