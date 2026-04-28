use num_complex::Complex;
use spectral_forge::dsp::phase::PhaseRotator;

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
    // 1024-entry LUT covers 2π. Worst-case quantization is 2π/1024 ≈ 0.006 rad.
    // sin/cos error at that quantum is ≤ 0.003. The test allows 0.005 slack.
    let r = PhaseRotator::new();
    let c = Complex::new(1.0_f32, 0.0);
    for i in 0..1024 {
        let theta = i as f32 / 1024.0; // freq_offset such that 2π·1·θ covers the table
        let out = r.rotate(c, theta, 1.0);
        let expected = Complex::from_polar(1.0, theta * std::f32::consts::TAU);
        assert!((out.re - expected.re).abs() < 0.005, "entry {} re error", i);
        assert!((out.im - expected.im).abs() < 0.005, "entry {} im error", i);
    }
}
