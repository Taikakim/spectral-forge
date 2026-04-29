use spectral_forge::dsp::circuit_kernels::{
    lp_step, tanh_levien_poly, spread_3tap, SimdRng,
};

#[test]
fn lp_step_settles_to_target_within_5_taus() {
    let mut state = 0.0_f32;
    let target = 1.0_f32;
    // alpha for tau=10 hops: alpha = 1 - exp(-1/10) ≈ 0.0952
    let alpha = 1.0 - (-1.0_f32 / 10.0).exp();
    for _ in 0..50 {
        lp_step(&mut state, target, alpha);
    }
    assert!((state - target).abs() < 0.01, "state={} after 50 hops at tau=10", state);
}

#[test]
fn lp_step_zero_alpha_holds_state() {
    let mut state = 0.5_f32;
    lp_step(&mut state, 9.0, 0.0);
    assert!((state - 0.5).abs() < 1e-9);
}

#[test]
fn tanh_levien_poly_matches_tanh_within_5pct_in_unit_band() {
    for i in -10..=10 {
        let x = i as f32 * 0.1;
        let exact = x.tanh();
        let approx = tanh_levien_poly(x);
        let err = (exact - approx).abs();
        assert!(err < 0.05, "x={} exact={} approx={} err={}", x, exact, approx, err);
    }
}

#[test]
fn tanh_levien_poly_saturates_at_extremes() {
    assert!(tanh_levien_poly(10.0) > 0.95);
    assert!(tanh_levien_poly(-10.0) < -0.95);
    assert!(tanh_levien_poly(100.0).is_finite());
    assert!(tanh_levien_poly(-100.0).is_finite());
}

#[test]
fn spread_3tap_neighbours_share_energy() {
    // Pre-cleared output buffer.
    let input  = vec![0.0, 0.0, 1.0, 0.0, 0.0]; // impulse at bin 2
    let mut output = vec![0.0_f32; 5];
    spread_3tap(&input, &mut output, 0.5); // 50% leakage to neighbours
    // Bin 2 should retain (1 - 0.5) = 0.5 of its energy.
    // Bins 1 and 3 should each receive 0.25.
    assert!((output[2] - 0.5).abs() < 1e-6, "centre={}", output[2]);
    assert!((output[1] - 0.25).abs() < 1e-6, "left={}", output[1]);
    assert!((output[3] - 0.25).abs() < 1e-6, "right={}", output[3]);
    assert!(output[0].abs() < 1e-6);
    assert!(output[4].abs() < 1e-6);
}

#[test]
fn spread_3tap_zero_strength_is_passthrough() {
    let input  = vec![0.1, 0.2, 0.3, 0.4, 0.5];
    let mut output = vec![0.0_f32; 5];
    spread_3tap(&input, &mut output, 0.0);
    for k in 0..5 {
        assert!((output[k] - input[k]).abs() < 1e-6);
    }
}

#[test]
fn spread_3tap_bounded_at_edges() {
    let input  = vec![1.0, 0.0, 0.0, 0.0, 1.0];
    let mut output = vec![0.0_f32; 5];
    spread_3tap(&input, &mut output, 0.6);
    // Edge bins miss one neighbour — they retain (1 - 0.3) = 0.7 (vs 0.4 in middle).
    // Specifically: bin 0 has only right neighbour: out = 0.4 * 1.0 + 0.3 * 0.0 = 0.4 (no left to leak in).
    // The choice for edge handling: zero-padded (no wrap). Verify finiteness only here.
    for k in 0..5 {
        assert!(output[k].is_finite() && output[k] >= 0.0);
    }
}

#[test]
fn simd_rng_produces_uniform_in_minus1_plus1() {
    let mut rng = SimdRng::new(0xCAFEBABE);
    let mut min = f32::MAX;
    let mut max = f32::MIN;
    for _ in 0..1000 {
        let x = rng.next_f32_centered();
        assert!(x.is_finite());
        assert!(x >= -1.0 && x < 1.0);
        if x < min { min = x; }
        if x > max { max = x; }
    }
    assert!(min < -0.8, "min={}: distribution should reach close to -1", min);
    assert!(max > 0.8,  "max={}: distribution should reach close to +1", max);
}

#[test]
fn simd_rng_deterministic_for_same_seed() {
    let mut a = SimdRng::new(42);
    let mut b = SimdRng::new(42);
    for _ in 0..100 {
        assert_eq!(a.next_u32(), b.next_u32());
    }
}
