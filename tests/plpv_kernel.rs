use spectral_forge::dsp::plpv::{damp_low_energy_bins, unwrap_phase, principal_arg};
use std::f32::consts::PI;

#[test]
fn principal_arg_wraps_to_pm_pi() {
    assert!((principal_arg(0.0) - 0.0).abs() < 1e-6);
    assert!((principal_arg(PI - 0.001) - (PI - 0.001)).abs() < 1e-6);
    assert!((principal_arg(PI + 0.001) - (-PI + 0.001)).abs() < 1e-4);
    assert!((principal_arg(3.0 * PI) - PI).abs() < 1e-4);
    assert!((principal_arg(-3.0 * PI) - (-PI)).abs() < 1e-4);
}

#[test]
fn unwrap_phase_constant_partial_advances_by_expected() {
    // A pure tone at bin k=10, sample rate 48000, fft 2048, hop 512.
    // Expected per-hop phase advance: 2π · 10 · 512 / 2048 = 5π ≡ π (mod 2π).
    let prev_phase = vec![0.0_f32; 2048 / 2 + 1];
    let curr_phase = {
        let mut v = vec![0.0_f32; 2048 / 2 + 1];
        v[10] = principal_arg(5.0 * PI);  // = π
        v
    };
    let mut prev_unwrapped = vec![0.0_f32; 2048 / 2 + 1];
    let mut out_unwrapped = vec![0.0_f32; 2048 / 2 + 1];

    unwrap_phase(
        &curr_phase, &prev_phase, &mut prev_unwrapped, &mut out_unwrapped,
        2048, 512, 1025,
    );

    // The unwrapped phase at bin 10 should be ~5π — the cumulative true phase.
    assert!((out_unwrapped[10] - 5.0 * PI).abs() < 1e-3,
        "expected ~5π, got {}", out_unwrapped[10]);
}

#[test]
fn unwrap_phase_multi_hop_accumulates_correctly() {
    // Verify that after N hops of a pure tone at bin k, the unwrapped phase
    // equals N * expected_advance_per_hop.
    //
    // fft=2048, hop=512 → expected advance at bin 5 = 2π·5·512/2048 = 5π/2.
    // After 4 hops the tone completes full revolutions and we can assert a
    // specific cumulative value.
    let fft = 2048;
    let hop = 512;
    let num_bins = fft / 2 + 1;
    let k = 5_usize;
    let advance_per_hop = 2.0 * PI * k as f32 * hop as f32 / fft as f32; // 5π/2

    let mut prev_phase     = vec![0.0_f32; num_bins];
    let mut prev_unwrapped = vec![0.0_f32; num_bins];
    let mut out_unwrapped  = vec![0.0_f32; num_bins];

    for hop_idx in 1..=4_u32 {
        // Simulate the current phase of a pure tone that started at phase 0.
        let mut curr_phase = vec![0.0_f32; num_bins];
        curr_phase[k] = principal_arg(advance_per_hop * hop_idx as f32);

        unwrap_phase(
            &curr_phase, &prev_phase, &mut prev_unwrapped, &mut out_unwrapped,
            fft, hop, num_bins,
        );
        prev_phase.copy_from_slice(&curr_phase);
    }

    // After 4 hops, unwrapped phase at bin 5 should be 4 * 5π/2 = 10π.
    let expected = 4.0 * advance_per_hop;
    assert!(
        (out_unwrapped[k] - expected).abs() < 1e-2,
        "after 4 hops: expected {:.4}, got {:.4}", expected, out_unwrapped[k],
    );
}

#[test]
fn damp_low_energy_blends_silent_bins_to_expected() {
    use std::f32::consts::PI;
    let num_bins = 1025;
    let mut unwrapped = vec![0.0_f32; num_bins];
    let mags = vec![0.0_f32; num_bins];  // all silence
    // Pre-fill: bin k=10 has unwrapped phase 99π (way off); rest 0.
    unwrapped[10] = 99.0 * PI;
    let expected = {
        let mut v = vec![0.0_f32; num_bins];
        for k in 0..num_bins { v[k] = PI * k as f32 / 2.0; }
        v
    };

    damp_low_energy_bins(&mut unwrapped, &mags, &expected, -60.0, num_bins);

    // Bin 10 was silent — its unwrapped phase should now be the expected_advance value.
    assert!((unwrapped[10] - expected[10]).abs() < 1e-3,
        "silent bin should be damped to expected; got {}", unwrapped[10]);
}

#[test]
fn damp_low_energy_leaves_loud_bins_alone() {
    use std::f32::consts::PI;
    let num_bins = 1025;
    let mut unwrapped = vec![5.0_f32 * PI; num_bins];
    // Loud signal: 0 dBFS == 1.0 RMS magnitude; well above -60 dB floor.
    let mags = vec![1.0_f32; num_bins];
    let expected = vec![PI / 2.0; num_bins];

    damp_low_energy_bins(&mut unwrapped, &mags, &expected, -60.0, num_bins);

    // Loud bins must not have been touched.
    for k in 0..num_bins {
        assert!((unwrapped[k] - 5.0 * PI).abs() < 1e-3, "loud bin {} was modified", k);
    }
}
