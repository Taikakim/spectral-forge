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
