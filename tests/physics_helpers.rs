use spectral_forge::dsp::physics_helpers::*;

#[test]
fn smooth_curve_one_pole_settles_to_input() {
    // Coefficient designed for tau = 4*dt; after ~16 steps with constant input,
    // the smoothed value should be within 1% of the input.
    let dt = 1.0 / 86.0; // hop rate at 44100 / 512
    let mut state = vec![0.0_f32; 4];
    let input = vec![0.0_f32, 0.5, 1.0, 2.0];
    for _ in 0..50 {
        smooth_curve_one_pole(&mut state, &input, dt);
    }
    for k in 0..4 {
        let err = (state[k] - input[k]).abs();
        assert!(err < 0.01 * (input[k].abs().max(1e-6)),
            "bin {} not converged: {} vs target {}", k, state[k], input[k]);
    }
}

#[test]
fn smooth_curve_one_pole_responds_within_tau() {
    // tau = 4*dt. After 4*dt, should reach ~63% of step input.
    let dt = 1.0 / 86.0;
    let mut state = vec![0.0_f32; 1];
    let input = vec![1.0_f32];
    // 4 steps = 1 tau.
    for _ in 0..4 {
        smooth_curve_one_pole(&mut state, &input, dt);
    }
    assert!(state[0] > 0.55 && state[0] < 0.75,
        "expected ~0.63 after 1 tau, got {}", state[0]);
}

#[test]
fn smooth_curve_one_pole_does_not_allocate() {
    let dt = 1.0 / 86.0;
    let mut state = vec![0.0_f32; 1024];
    let input = vec![1.0_f32; 1024];
    let cap_before = state.capacity();
    smooth_curve_one_pole(&mut state, &input, dt);
    assert_eq!(state.capacity(), cap_before, "must not realloc");
}

#[test]
fn clamp_for_cfl_caps_at_1_5_over_dt() {
    let dt = 1.0 / 86.0;
    let max_omega = 1.5 / dt;
    // Below the cap — pass through.
    assert_eq!(clamp_for_cfl(max_omega * 0.5, dt), max_omega * 0.5);
    // At the cap — pass through (exact boundary OK).
    let at_cap = clamp_for_cfl(max_omega, dt);
    assert!((at_cap - max_omega).abs() < 1e-3);
    // Above the cap — clamped.
    let above = clamp_for_cfl(max_omega * 10.0, dt);
    assert!((above - max_omega).abs() < 1e-3, "must clamp to 1.5/dt, got {}", above);
}

#[test]
fn clamp_for_cfl_handles_zero_and_negative() {
    let dt = 1.0 / 86.0;
    assert_eq!(clamp_for_cfl(0.0, dt), 0.0);
    // Negative omega is meaningless physically — treat as 0.
    assert_eq!(clamp_for_cfl(-5.0, dt), 0.0);
}

#[test]
fn clamp_damping_floor_enforces_minimum() {
    assert_eq!(clamp_damping_floor(0.0), 0.05);
    assert_eq!(clamp_damping_floor(0.04), 0.05);
    assert_eq!(clamp_damping_floor(0.05), 0.05);
    assert_eq!(clamp_damping_floor(0.5), 0.5);
    assert_eq!(clamp_damping_floor(1.5), 1.5);
}

#[test]
fn apply_energy_rise_hysteresis_scales_doubled_bins() {
    let mut velocity = vec![1.0_f32, 1.0, 1.0, 1.0];
    let prev_kepe = vec![1.0_f32, 1.0, 1.0, 1.0];
    let curr_kepe = vec![3.0_f32, 1.5, 0.5, 2.5];
    let mut rose_last = vec![true, false, true, true];
    apply_energy_rise_hysteresis(&mut velocity, &prev_kepe, &curr_kepe, &mut rose_last);
    // bin 0: doubled (3.0 > 2 * 1.0) AND rose_last -> scale by sqrt(0.5) ≈ 0.707
    assert!((velocity[0] - (1.0_f32 / 2.0_f32.sqrt())).abs() < 1e-5);
    // bin 1: did not double -> unchanged
    assert_eq!(velocity[1], 1.0);
    // bin 2: did not double (0.5 < 2*1.0) -> unchanged
    assert_eq!(velocity[2], 1.0);
    // bin 3: doubled but rose_last was true -> scale (hysteresis fires on 2 in a row)
    assert!((velocity[3] - (1.0_f32 / 2.0_f32.sqrt())).abs() < 1e-5);
    // rose_last updated for next call
    assert!(rose_last[0]);  // current also doubled -> still true
    assert!(!rose_last[1]); // did not double -> false
    assert!(!rose_last[2]); // did not double -> false
    assert!(rose_last[3]);  // doubled -> true
}
