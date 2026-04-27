use spectral_forge::dsp::amp_modes::{AmpMode, AmpCellParams, AmpNodeState};
use num_complex::Complex;

#[test]
fn amp_mode_default_is_linear() {
    assert_eq!(AmpMode::default(), AmpMode::Linear);
}

#[test]
fn amp_cell_params_default_is_neutral() {
    let p = AmpCellParams::default();
    assert_eq!(p.amount,        1.0);
    assert_eq!(p.threshold,     0.5);
    assert_eq!(p.release_ms,   100.0);
    assert_eq!(p.slew_db_per_s, 60.0);
}

const NB: usize = 16;

fn neutral_input() -> Vec<Complex<f32>> {
    (0..NB).map(|k| Complex::new(0.5, 0.1 * k as f32 / NB as f32)).collect()
}

#[test]
fn linear_passes_through_with_amount_one() {
    let mut state = AmpNodeState::new(AmpMode::Linear, NB);
    let p = AmpCellParams::default();
    let mut buf = neutral_input();
    let original = buf.clone();
    state.apply(&p, &mut buf, 1.0 / 48000.0 * 512.0);
    for (a, b) in buf.iter().zip(original.iter()) {
        assert!((a.re - b.re).abs() < 1e-6);
        assert!((a.im - b.im).abs() < 1e-6);
    }
}

#[test]
fn vactrol_holds_then_releases() {
    let mut state = AmpNodeState::new(AmpMode::Vactrol, NB);
    let p = AmpCellParams { amount: 1.0, release_ms: 200.0, ..Default::default() };
    let mut buf = vec![Complex::new(1.0, 0.0); NB];
    let hop_dt = 1.0 / 48000.0 * 512.0;
    state.apply(&p, &mut buf, hop_dt); // first hit, capacitor charges fast
    let charged_mag = buf[0].norm();
    // 5 ms attack TC + ~10.7 ms hop ≈ exp(-2.13) ≈ 0.118 leftover, so cap ≈ 0.882,
    // gain ≈ 0.882^0.6 ≈ 0.926. The test only verifies "charges quickly" — well
    // above the un-charged 0.0 baseline within one hop.
    assert!(charged_mag > 0.85, "vactrol should charge quickly on first hit, got {charged_mag}");
    for _ in 0..5 {
        let mut zero_buf = vec![Complex::new(0.0, 0.0); NB];
        state.apply(&p, &mut zero_buf, hop_dt);
    }
    // Probe decay with a half-magnitude signal: the release branch leaves cap above the
    // input (0.5), so output = 0.5 * cap.powf(0.6). After silence the cap is lower than
    // immediately post-charge, so this probe should yield strictly less output than a
    // matching probe taken with no silence between charge and probe.
    let mut probe_after_decay = vec![Complex::new(0.5, 0.0); NB];
    state.apply(&p, &mut probe_after_decay, hop_dt);
    for c in &probe_after_decay { assert!(c.re.is_finite() && c.im.is_finite()); }

    // Reset state, charge, then immediately probe (no silence in between).
    let mut state2 = AmpNodeState::new(AmpMode::Vactrol, NB);
    let mut charge = vec![Complex::new(1.0, 0.0); NB];
    state2.apply(&p, &mut charge, hop_dt);
    let mut probe_no_decay = vec![Complex::new(0.5, 0.0); NB];
    state2.apply(&p, &mut probe_no_decay, hop_dt);
    assert!(probe_after_decay[0].norm() < probe_no_decay[0].norm(),
            "vactrol cap should decay during silence: probe_after_decay={} probe_no_decay={}",
            probe_after_decay[0].norm(), probe_no_decay[0].norm());
}

#[test]
fn schmitt_latches() {
    let mut state = AmpNodeState::new(AmpMode::Schmitt, NB);
    let p = AmpCellParams { amount: 1.0, threshold: 0.6, ..Default::default() };
    let mut buf = vec![Complex::new(0.3, 0.0); NB];
    state.apply(&p, &mut buf, 0.01);
    for c in &buf { assert!(c.norm() < 1e-6, "below threshold should be gated"); }
    let mut buf = vec![Complex::new(0.8, 0.0); NB];
    state.apply(&p, &mut buf, 0.01);
    for c in &buf { assert!(c.norm() > 0.5, "above threshold should pass"); }
    let mut buf = vec![Complex::new(0.55, 0.0); NB];
    state.apply(&p, &mut buf, 0.01);
    for c in &buf { assert!(c.norm() > 0.5, "hysteresis: should remain open"); }
}

#[test]
fn slew_limits_change_rate() {
    let mut state = AmpNodeState::new(AmpMode::Slew, NB);
    let p = AmpCellParams { amount: 1.0, slew_db_per_s: 60.0, ..Default::default() };
    let mut buf = vec![Complex::new(1.0, 0.0); NB];
    state.apply(&p, &mut buf, 1.0 / 48000.0 * 512.0);
    for c in &buf { assert!(c.norm() < 0.2, "slew should limit large jumps"); }
}

#[test]
fn stiction_dead_zone() {
    let mut state = AmpNodeState::new(AmpMode::Stiction, NB);
    let p = AmpCellParams { amount: 1.0, threshold: 0.1, ..Default::default() };
    let mut buf = vec![Complex::new(0.05, 0.0); NB];
    state.apply(&p, &mut buf, 0.01);
    for c in &buf { assert!(c.norm() < 1e-6, "below stiction threshold = no movement"); }
    let mut buf = vec![Complex::new(0.5, 0.0); NB];
    state.apply(&p, &mut buf, 0.01);
    for c in &buf { assert!(c.norm() > 0.4, "over threshold should release"); }
}
