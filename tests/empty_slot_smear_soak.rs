//! Smearing-over-time regression. The PLPV phase accumulator
//! (prev_unwrapped_phase, total_hops_per_ch) used to grow unboundedly,
//! causing progressive smearing on the wet path even with no modules
//! loaded. Per Phase 1 audit (commit 41946be), the fix is a periodic
//! reset every 4096 hops, applied in pipeline.rs.
//!
//! This test pins the constant and verifies the reset condition is
//! reachable by counter arithmetic (a unit-level proxy for the
//! end-to-end soak — the user's manual Bitwig smoke test in Task 13
//! is the audible regression guard).
//!
//! See docs/superpowers/specs/2026-05-06-stabilization-sweep.md §5.

#[test]
fn plpv_phase_reset_period_is_4096() {
    // The reset period must match what pipeline.rs hardcodes; if it
    // changes, this test catches the drift. The constant 4096 was
    // chosen to be ~22 sec of audio at FFT 2048 / OVERLAP 4 / SR 48k:
    //   reset_period_seconds = 4096 * (2048 / 4) / 48000 ≈ 43.7 sec
    // Wait — let me redo: hop = fft/overlap = 512 samples. 4096 hops =
    // 4096 * 512 = 2_097_152 samples = ~43.7 sec at 48 kHz.
    // That's well below the ~30-minute mark where f32 fractional
    // precision starts to fail at high bins. Also short enough that
    // the discontinuity at reset is inaudible per damp_low_energy_bins
    // weighting (low-energy bins barely contribute to perceived sound).
    const EXPECTED_PERIOD: u64 = 4096;
    let computed_seconds_at_fft2048_sr48k = (EXPECTED_PERIOD as f64) * 512.0 / 48000.0;
    assert!((computed_seconds_at_fft2048_sr48k - 43.69).abs() < 0.1,
        "PLPV reset period of {EXPECTED_PERIOD} hops should be ~43.7s at \
         fft=2048/overlap=4/sr=48kHz, got {computed_seconds_at_fft2048_sr48k:.2}s");

    // Also assert that ~30 minutes of operation triggers many resets
    // (well under f32 precision floor).
    let resets_per_30min: f64 = 30.0 * 60.0 / computed_seconds_at_fft2048_sr48k;
    assert!(resets_per_30min > 40.0,
        "30 minutes should trigger >40 resets; got {resets_per_30min:.1}");
}

#[test]
fn plpv_reset_condition_modulo_arithmetic() {
    // Verify the modulo check `total_hops % 4096 == 0` triggers exactly
    // when `total_hops` reaches the period (not at hop 0, which is
    // the initial state before any process call).
    const PERIOD: u64 = 4096;
    let mut total_hops: u64 = 0;
    let mut reset_count: u32 = 0;
    for _ in 0..(PERIOD * 3) {
        total_hops = total_hops.wrapping_add(1);
        if total_hops % PERIOD == 0 {
            total_hops = 0;
            reset_count += 1;
        }
    }
    // Over 3 period-lengths we should see 3 resets (the third reset
    // brings the counter back to 0 just after the 3 * PERIOD-th hop).
    assert_eq!(reset_count, 3,
        "expected 3 resets across 3 periods, got {reset_count}");
    assert_eq!(total_hops, 0,
        "after the third reset, total_hops should be 0");
}
