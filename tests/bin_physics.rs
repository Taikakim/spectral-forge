use spectral_forge::dsp::bin_physics::{BinPhysics, MergeRule};

#[test]
fn defaults_match_spec() {
    let p = BinPhysics::new();
    assert_eq!(p.velocity[0], 0.0);
    assert_eq!(p.mass[0], 1.0);             // inertia = 1 (no resistance)
    assert_eq!(p.temperature[0], 0.0);
    assert_eq!(p.flux[0], 0.0);
    assert_eq!(p.displacement[0], 0.0);
    assert_eq!(p.crystallization[0], 0.0);
    assert_eq!(p.phase_momentum[0], 0.0);
    assert_eq!(p.slew[0], 0.0);
    assert_eq!(p.bias[0], 0.0);
    assert_eq!(p.decay_estimate[0], 0.0);
}

#[test]
fn reset_active_seeds_lock_target_freq_to_bin_centre() {
    let mut p = BinPhysics::new();
    let sr = 48_000.0_f32;
    let fft = 2048_usize;
    let num_bins = 1025;
    p.reset_active(num_bins, sr, fft);
    let bin_hz = sr / fft as f32;
    assert!((p.lock_target_freq[0]   - 0.0           ).abs() < 1e-3);
    assert!((p.lock_target_freq[1]   - bin_hz        ).abs() < 1e-3);
    assert!((p.lock_target_freq[100] - 100.0 * bin_hz).abs() < 1e-3);
}

#[test]
fn merge_rule_max_wins_picks_higher() {
    let mut dst = 0.2;
    BinPhysics::merge_one(&mut dst, 0.7, 0.3, MergeRule::Max);
    assert!((dst - 0.7).abs() < 1e-6);
}

#[test]
fn merge_rule_heavier_wins() {
    let mut dst = 1.0;
    BinPhysics::merge_one(&mut dst, 5.0, 0.5, MergeRule::HeavierWins);
    assert!((dst - 5.0).abs() < 1e-6);
    BinPhysics::merge_one(&mut dst, 2.0, 1.0, MergeRule::HeavierWins);
    assert!((dst - 5.0).abs() < 1e-6, "lighter src must not overwrite heavier dst");
}

#[test]
fn merge_rule_weighted_avg_blends() {
    let mut dst = 0.0;
    BinPhysics::merge_one(&mut dst, 1.0, 0.5, MergeRule::WeightedAvg);
    assert!((dst - 0.5).abs() < 1e-6);
    BinPhysics::merge_one(&mut dst, 0.0, 0.5, MergeRule::WeightedAvg);
    // dst now: prior 0.5 weighted by 0.5, plus 0.0 weighted by 0.5 = 0.25
    assert!((dst - 0.25).abs() < 1e-6);
}

#[test]
fn velocity_computed_from_magnitude_delta() {
    use num_complex::Complex;
    let mut p = BinPhysics::new();
    let prev = vec![Complex::new(0.0_f32, 0.0); 4];
    let curr = vec![
        Complex::new(0.5, 0.0),
        Complex::new(0.25, 0.0),
        Complex::new(0.0, 0.0),
        Complex::new(0.1, 0.0),
    ];
    BinPhysics::compute_velocity(&mut p.velocity, &prev, &curr, 4);
    assert!((p.velocity[0] - 0.5 ).abs() < 1e-6);
    assert!((p.velocity[1] - 0.25).abs() < 1e-6);
    assert!((p.velocity[2] - 0.0 ).abs() < 1e-6);
    assert!((p.velocity[3] - 0.1 ).abs() < 1e-6);
}

#[test]
fn mix_from_blends_all_fields_per_rule() {
    let mut dst = BinPhysics::new();
    let mut src = BinPhysics::new();
    let n = 4;
    // Seed src with non-default values so we can detect the merge.
    for k in 0..n {
        src.mass[k] = 5.0;             // dst=1.0, src=5.0 → HeavierWins → 5.0
        src.temperature[k] = 1.0;      // dst=0.0, src=1.0, w=0.5 → 0.5
        src.crystallization[k] = 0.7;  // dst=0.0, src=0.7 → Max → 0.7
    }
    dst.mix_from(&src, 0.5, n);
    for k in 0..n {
        assert!((dst.mass[k] - 5.0).abs() < 1e-6, "mass not heavier-wins");
        assert!((dst.temperature[k] - 0.5).abs() < 1e-6, "temperature not weighted-avg");
        assert!((dst.crystallization[k] - 0.7).abs() < 1e-6, "crystallization not max");
    }
}

#[test]
fn copy_from_replicates_active_region() {
    let mut dst = BinPhysics::new();
    let mut src = BinPhysics::new();
    let n = 8;
    for k in 0..n {
        src.mass[k] = (k as f32) + 2.0;
        src.temperature[k] = -(k as f32);
    }
    dst.copy_from(&src, n);
    for k in 0..n {
        assert!((dst.mass[k] - ((k as f32) + 2.0)).abs() < 1e-6);
        assert!((dst.temperature[k] + k as f32).abs() < 1e-6);
    }
}

#[test]
fn reset_active_does_not_touch_inactive_region() {
    let mut p = BinPhysics::new();
    let n = 4;
    // Mark the bin just past the active region with a sentinel.
    p.mass[n] = 99.0;
    p.temperature[n] = -7.0;
    p.lock_target_freq[n] = 12_345.0;
    p.reset_active(n, 48_000.0, 2048);
    assert!((p.mass[n] - 99.0).abs() < 1e-6,
        "mass past active region should be untouched, got {}", p.mass[n]);
    assert!((p.temperature[n] + 7.0).abs() < 1e-6,
        "temperature past active region should be untouched");
    assert!((p.lock_target_freq[n] - 12_345.0).abs() < 1e-6,
        "lock_target_freq past active region should be untouched");
}

#[test]
fn mix_from_does_not_touch_inactive_region() {
    let mut dst = BinPhysics::new();
    let mut src = BinPhysics::new();
    let n = 4;
    // Sentinels just past the active region in dst.
    dst.mass[n] = 99.0;
    dst.temperature[n] = -7.0;
    // src has different non-default values inside its active region (irrelevant past n).
    for k in 0..n {
        src.mass[k] = 5.0;
        src.temperature[k] = 1.0;
    }
    dst.mix_from(&src, 0.5, n);
    assert!((dst.mass[n] - 99.0).abs() < 1e-6,
        "mass past num_bins must not be merged, got {}", dst.mass[n]);
    assert!((dst.temperature[n] + 7.0).abs() < 1e-6,
        "temperature past num_bins must not be merged");
}

#[test]
fn copy_from_does_not_touch_inactive_region() {
    let mut dst = BinPhysics::new();
    let mut src = BinPhysics::new();
    let n = 4;
    dst.mass[n] = 99.0;
    dst.temperature[n] = -7.0;
    for k in 0..n {
        src.mass[k] = 3.0;
        src.temperature[k] = 1.0;
    }
    dst.copy_from(&src, n);
    assert!((dst.mass[n] - 99.0).abs() < 1e-6,
        "mass past num_bins must not be copied, got {}", dst.mass[n]);
    assert!((dst.temperature[n] + 7.0).abs() < 1e-6,
        "temperature past num_bins must not be copied");
}
