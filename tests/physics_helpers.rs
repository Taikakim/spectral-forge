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

#[test]
fn wrap_phase_folds_into_minus_pi_to_pi() {
    use std::f32::consts::PI;
    assert!((wrap_phase(0.0) - 0.0).abs() < 1e-6);
    assert!((wrap_phase(PI) - PI).abs() < 1e-6);
    assert!((wrap_phase(-PI) - (-PI)).abs() < 1e-6);
    // 1.5*PI -> -0.5*PI
    assert!((wrap_phase(1.5 * PI) - (-0.5 * PI)).abs() < 1e-5);
    // -1.5*PI -> 0.5*PI
    assert!((wrap_phase(-1.5 * PI) - (0.5 * PI)).abs() < 1e-5);
    // 5*PI -> PI (or -PI; either is fine within 1e-5)
    let w = wrap_phase(5.0 * PI);
    assert!(w.abs() <= PI + 1e-5);
}

#[test]
fn pll_bank_step_locks_to_constant_target() {
    use std::f32::consts::PI;
    // 4 bins, all targets at PI/4. PLL starts at 0, should converge.
    let mut pll_phase = vec![0.0_f32; 4];
    let mut pll_freq = vec![0.0_f32; 4];
    let target = vec![PI / 4.0; 4];
    let mut err = vec![0.0_f32; 4];
    // Butterworth-flat: omega_n = 0.05 cycles/hop, zeta = 0.707.
    // alpha = 2 * zeta * omega_n, beta = omega_n^2.
    let omega_n = 0.05_f32;
    let zeta = 0.707_f32;
    let alpha = 2.0 * zeta * omega_n;
    let beta = omega_n * omega_n;

    // Run 200 hops; final phase error must be near-zero.
    for _ in 0..200 {
        pll_bank_step(&mut pll_phase, &mut pll_freq, &target, alpha, beta, &mut err);
    }
    for k in 0..4 {
        assert!(err[k].abs() < 0.01, "bin {} did not lock: err = {}", k, err[k]);
        // PLL frequency should have settled near 0 (target is constant).
        assert!(pll_freq[k].abs() < 0.01, "bin {} freq drift: {}", k, pll_freq[k]);
    }
}

#[test]
fn pll_bank_step_tracks_constant_velocity_target() {
    // Target advances by 0.1 rad per hop. PLL should match the velocity.
    let mut pll_phase = vec![0.0_f32];
    let mut pll_freq = vec![0.0_f32];
    let mut target = 0.0_f32;
    let mut err = vec![0.0_f32];
    let omega_n = 0.1_f32;
    let zeta = 0.707_f32;
    let alpha = 2.0 * zeta * omega_n;
    let beta = omega_n * omega_n;

    // 500 hops to fully settle.
    for _ in 0..500 {
        target += 0.1;
        let target_v = vec![wrap_phase(target)];
        pll_bank_step(&mut pll_phase, &mut pll_freq, &target_v, alpha, beta, &mut err);
    }
    // Steady-state error for a velocity ramp under a 2nd-order PI loop should
    // approach zero (this loop is type-2, no steady-state ramp error).
    assert!(err[0].abs() < 0.05, "velocity tracking err = {}", err[0]);
    // Freq estimate should be near 0.1 rad/hop.
    assert!((pll_freq[0] - 0.1).abs() < 0.01, "freq estimate = {}", pll_freq[0]);
}

#[test]
fn pll_bank_step_lengths_must_match() {
    // Debug assert; not a panic in release. Skip on release builds.
    if cfg!(debug_assertions) {
        let mut pll_phase = vec![0.0_f32; 3];
        let mut pll_freq = vec![0.0_f32; 3];
        let target = vec![0.0_f32; 4]; // mismatched
        let mut err = vec![0.0_f32; 3];
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            pll_bank_step(&mut pll_phase, &mut pll_freq, &target, 0.05, 0.0025, &mut err);
        }));
        assert!(result.is_err(), "expected debug assert panic on length mismatch");
    }
}
