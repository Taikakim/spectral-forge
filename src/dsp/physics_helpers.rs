//! Shared helpers for physics-based spectral modules (Kinetics, Modulate retrofit, v2).
//!
//! All functions are real-time-safe: no allocation, no locking, no I/O. The pre-allocated
//! buffer is mutated in place. See `ideas/next-gen-modules/12-kinetics.md` § "Research
//! findings (2026-04-26)" for the numerical-stability rationale.

/// One-pole low-pass smoother applied per-bin. Coefficient is derived from
/// `tau = 4 * dt` (research finding 3 — slow enough to suppress hop-rate Mathieu
/// pumping but fast enough to track user gestures within ~50 ms at 44.1 kHz/hop=512).
///
/// Both slices must have the same length. Mutates `state` in place.
#[inline]
pub fn smooth_curve_one_pole(state: &mut [f32], input: &[f32], dt: f32) {
    debug_assert_eq!(state.len(), input.len());
    // tau = 4*dt -> dt/tau = 0.25 -> alpha = 1 - exp(-0.25) ≈ 0.2212.
    // Compute once instead of per-bin (avoids exp() in tight loop).
    let alpha = 1.0_f32 - (-0.25_f32).exp();
    for k in 0..state.len() {
        let s = state[k];
        state[k] = s + alpha * (input[k] - s);
    }
    // dt is a parameter for forward compatibility — if a caller wants tau != 4*dt
    // in the future we can swap to: alpha = 1 - exp(-dt / tau).
    let _ = dt;
}

/// Clamp a user-facing angular frequency (rad/s) so the Velocity-Verlet integrator
/// stays inside the CFL stability bound `omega * dt < 1.5` (50% safety from the strict
/// `< 2.0` bound). See research finding 2.
///
/// Negative input is treated as 0 (physically meaningless).
#[inline]
pub fn clamp_for_cfl(omega: f32, dt: f32) -> f32 {
    if omega <= 0.0 {
        return 0.0;
    }
    let cap = 1.5_f32 / dt;
    if omega > cap { cap } else { omega }
}

/// Enforce the per-bin viscous-damping floor of 0.05 (research finding 4).
/// Below this, the spring chain provably destabilises under all parameter modulations
/// within the CFL bound.
#[inline]
pub fn clamp_damping_floor(damping: f32) -> f32 {
    if damping < 0.05 { 0.05 } else { damping }
}

/// Energy-rise hysteresis safety net (research finding 5). For each bin, if
/// `KE+PE` doubles in **two consecutive hops** (the hysteresis condition), scale
/// `velocity[k]` by `1/sqrt(2)` to bleed off the runaway energy.
///
/// `rose_last[k]` carries the previous hop's "doubled" flag and is overwritten with
/// this hop's flag. Both `prev_kepe` and `curr_kepe` must be the same length as `velocity`.
#[inline]
pub fn apply_energy_rise_hysteresis(
    velocity: &mut [f32],
    prev_kepe: &[f32],
    curr_kepe: &[f32],
    rose_last: &mut [bool],
) {
    debug_assert_eq!(velocity.len(), prev_kepe.len());
    debug_assert_eq!(velocity.len(), curr_kepe.len());
    debug_assert_eq!(velocity.len(), rose_last.len());
    let inv_sqrt2 = 1.0_f32 / 2.0_f32.sqrt();
    for k in 0..velocity.len() {
        let doubled = curr_kepe[k] > 2.0 * prev_kepe[k];
        if doubled && rose_last[k] {
            velocity[k] *= inv_sqrt2;
        }
        rose_last[k] = doubled;
    }
}
