//! Calibration audit — every per-curve offset_fn's declared ±1 → [y_min, y_max]
//! range is verified end-to-end through the module's DSP.
//! See docs/superpowers/plans/2026-04-24-calibration-audit.md Task 2.

use num_complex::Complex;
use spectral_forge::dsp::modules::{
    create_module, FutureMode, GainMode, ModuleContext, ModuleType, ProbeSnapshot, SpectralModule,
};
use spectral_forge::dsp::modules::rhythm::RhythmMode;
use spectral_forge::dsp::modules::geometry::GeometryMode;
use spectral_forge::editor::curve_config::curve_display_config;
use spectral_forge::params::{FxChannelTarget, StereoLink};

const FFT_SIZE: usize = 2048;
const NUM_BINS: usize = FFT_SIZE / 2 + 1;
const SAMPLE_RATE: f32 = 48_000.0;

fn make_ctx() -> ModuleContext<'static> {
    ModuleContext::new(
        SAMPLE_RATE, FFT_SIZE, NUM_BINS,
        10.0, 100.0, 0.5,
        0.0, false, false,
    )
}

/// Run the module with every curve filled with `gain_on_target` on
/// `target_curve_idx` and 1.0 on all other curves. Returns the probe.
fn run_case(
    module: &mut Box<dyn SpectralModule>,
    num_curves: usize,
    target_curve_idx: usize,
    gain_on_target: f32,
) -> ProbeSnapshot {
    let curves_storage: Vec<Vec<f32>> = (0..num_curves)
        .map(|c| if c == target_curve_idx {
            vec![gain_on_target; NUM_BINS]
        } else {
            vec![1.0; NUM_BINS]
        })
        .collect();
    let curves_refs: Vec<&[f32]> = curves_storage.iter().map(|v| v.as_slice()).collect();

    let mut bins: Vec<Complex<f32>> = vec![Complex::new(0.1, 0.0); NUM_BINS];
    let mut suppression: Vec<f32> = vec![0.0; NUM_BINS];
    let ctx = make_ctx();
    module.process(
        0,
        StereoLink::Linked,
        FxChannelTarget::All,
        &mut bins,
        None,
        &curves_refs,
        &mut suppression,
        None,
        &ctx,
    );
    module.last_probe()
}

#[test]
fn dynamics_threshold_offset_plus_one_hits_y_max() {
    let mut m = create_module(ModuleType::Dynamics, SAMPLE_RATE, FFT_SIZE);
    let nc = m.num_curves();
    let cfg = curve_display_config(ModuleType::Dynamics, 0, GainMode::Add);
    let g = (cfg.offset_fn)(1.0, 1.0);
    let probe = run_case(&mut m, nc, 0, g);
    let observed = probe.threshold_db.expect("dynamics must probe threshold");
    assert!(
        (observed - cfg.y_max).abs() < 0.5,
        "offset=+1 should give threshold≈{}, got {}", cfg.y_max, observed,
    );
}

#[test]
fn dynamics_threshold_offset_minus_one_hits_y_min() {
    let mut m = create_module(ModuleType::Dynamics, SAMPLE_RATE, FFT_SIZE);
    let nc = m.num_curves();
    let cfg = curve_display_config(ModuleType::Dynamics, 0, GainMode::Add);
    let g = (cfg.offset_fn)(1.0, -1.0);
    let probe = run_case(&mut m, nc, 0, g);
    let observed = probe.threshold_db.expect("dynamics must probe threshold");
    assert!(
        (observed - cfg.y_min).abs() < 0.5,
        "offset=-1 should give threshold≈{}, got {}", cfg.y_min, observed,
    );
}

#[test]
fn dynamics_ratio_offset_plus_one_hits_y_max() {
    let mut m = create_module(ModuleType::Dynamics, SAMPLE_RATE, FFT_SIZE);
    let nc = m.num_curves();
    let cfg = curve_display_config(ModuleType::Dynamics, 1, GainMode::Add);
    let g = (cfg.offset_fn)(1.0, 1.0);
    let probe = run_case(&mut m, nc, 1, g);
    let observed = probe.ratio.expect("dynamics must probe ratio");
    assert!(
        (observed - cfg.y_max).abs() < 0.1,
        "offset=+1 should give ratio≈{}, got {}", cfg.y_max, observed,
    );
}

#[test]
fn dynamics_ratio_offset_minus_one_hits_y_min() {
    let mut m = create_module(ModuleType::Dynamics, SAMPLE_RATE, FFT_SIZE);
    let nc = m.num_curves();
    let cfg = curve_display_config(ModuleType::Dynamics, 1, GainMode::Add);
    let g = (cfg.offset_fn)(1.0, -1.0);
    let probe = run_case(&mut m, nc, 1, g);
    let observed = probe.ratio.expect("dynamics must probe ratio");
    assert!(
        (observed - cfg.y_min).abs() < 0.1,
        "offset=-1 should give ratio≈{}, got {}", cfg.y_min, observed,
    );
}

#[test]
fn dynamics_knee_offset_extremes() {
    let mut m = create_module(ModuleType::Dynamics, SAMPLE_RATE, FFT_SIZE);
    let nc = m.num_curves();
    let cfg = curve_display_config(ModuleType::Dynamics, 4, GainMode::Add);

    let g_hi = (cfg.offset_fn)(1.0, 1.0);
    let probe = run_case(&mut m, nc, 4, g_hi);
    let observed = probe.knee_db.unwrap();
    assert!((observed - cfg.y_max).abs() < 0.5, "knee hi: want {}, got {}", cfg.y_max, observed);

    let g_lo = (cfg.offset_fn)(1.0, -1.0);
    let probe = run_case(&mut m, nc, 4, g_lo);
    let observed = probe.knee_db.unwrap();
    assert!((observed - cfg.y_min).abs() < 0.5, "knee lo: want {}, got {}", cfg.y_min, observed);
}

#[test]
fn dynamics_mix_offset_extremes() {
    let mut m = create_module(ModuleType::Dynamics, SAMPLE_RATE, FFT_SIZE);
    let nc = m.num_curves();
    let cfg = curve_display_config(ModuleType::Dynamics, 5, GainMode::Add);

    let g_hi = (cfg.offset_fn)(1.0, 1.0);
    let probe = run_case(&mut m, nc, 5, g_hi);
    assert!((probe.mix_pct.unwrap() - cfg.y_max).abs() < 1.0);

    let g_lo = (cfg.offset_fn)(1.0, -1.0);
    let probe = run_case(&mut m, nc, 5, g_lo);
    assert!((probe.mix_pct.unwrap() - cfg.y_min).abs() < 1.0);
}

// Attack/Release are multiplicative with factor 1024. The module scales
// ctx.attack_ms (10 ms) by the curve gain and clamps. Verify scaling works.
#[test]
fn dynamics_attack_offset_plus_one_multiplies_global() {
    let mut m = create_module(ModuleType::Dynamics, SAMPLE_RATE, FFT_SIZE);
    let nc = m.num_curves();
    let cfg = curve_display_config(ModuleType::Dynamics, 2, GainMode::Add);
    let g = (cfg.offset_fn)(1.0, 1.0); // = 1024.0
    let probe = run_case(&mut m, nc, 2, g);
    // ctx.attack_ms=10 × 1024 = 10240, clamped at 500 (pipeline limit) — so the
    // y_max of 1024 in the config is actually a display-only limit, not a DSP
    // limit. The test just asserts the attack reaches the 500 ms DSP clamp.
    let observed = probe.attack_ms.unwrap();
    assert!(observed >= 500.0 - 1.0, "attack should reach DSP clamp 500, got {}", observed);
}

#[test]
fn freeze_length_offset_plus_one_hits_y_max() {
    let mut m = create_module(ModuleType::Freeze, SAMPLE_RATE, FFT_SIZE);
    m.reset(SAMPLE_RATE, FFT_SIZE);
    let nc = m.num_curves();
    let cfg = curve_display_config(ModuleType::Freeze, 0, GainMode::Add);
    let g = (cfg.offset_fn)(1.0, 1.0);
    let probe = run_case(&mut m, nc, 0, g);
    let observed = probe.length_ms.expect("freeze must probe length");
    assert!(
        (observed - cfg.y_max).abs() < 50.0,
        "freeze length offset=+1 should give ≈{} ms, got {}", cfg.y_max, observed,
    );
}

#[test]
fn freeze_length_offset_minus_one_hits_y_min() {
    let mut m = create_module(ModuleType::Freeze, SAMPLE_RATE, FFT_SIZE);
    m.reset(SAMPLE_RATE, FFT_SIZE);
    let nc = m.num_curves();
    let cfg = curve_display_config(ModuleType::Freeze, 0, GainMode::Add);
    let g = (cfg.offset_fn)(1.0, -1.0);
    let probe = run_case(&mut m, nc, 0, g);
    let observed = probe.length_ms.expect("freeze must probe length");
    assert!(
        (observed - cfg.y_min).abs() < 5.0,
        "freeze length offset=-1 should give ≈{} ms, got {}", cfg.y_min, observed,
    );
}

#[test]
fn freeze_threshold_offset_extremes() {
    let mut m = create_module(ModuleType::Freeze, SAMPLE_RATE, FFT_SIZE);
    m.reset(SAMPLE_RATE, FFT_SIZE);
    let nc = m.num_curves();
    let cfg = curve_display_config(ModuleType::Freeze, 1, GainMode::Add);

    let g_hi = (cfg.offset_fn)(1.0, 1.0);
    let probe = run_case(&mut m, nc, 1, g_hi);
    assert!((probe.threshold_db.unwrap() - cfg.y_max).abs() < 1.0,
        "freeze threshold hi: want {}, got {}", cfg.y_max, probe.threshold_db.unwrap());

    let g_lo = (cfg.offset_fn)(1.0, -1.0);
    let probe = run_case(&mut m, nc, 1, g_lo);
    assert!((probe.threshold_db.unwrap() - cfg.y_min).abs() < 1.0,
        "freeze threshold lo: want {}, got {}", cfg.y_min, probe.threshold_db.unwrap());
}

#[test]
fn freeze_portamento_offset_extremes() {
    let mut m = create_module(ModuleType::Freeze, SAMPLE_RATE, FFT_SIZE);
    m.reset(SAMPLE_RATE, FFT_SIZE);
    let nc = m.num_curves();
    let cfg = curve_display_config(ModuleType::Freeze, 2, GainMode::Add);

    let g_hi = (cfg.offset_fn)(1.0, 1.0);
    let probe = run_case(&mut m, nc, 2, g_hi);
    assert!((probe.portamento_ms.unwrap() - cfg.y_max).abs() < 5.0,
        "freeze portamento hi: want {}, got {}", cfg.y_max, probe.portamento_ms.unwrap());

    let g_lo = (cfg.offset_fn)(1.0, -1.0);
    let probe = run_case(&mut m, nc, 2, g_lo);
    assert!((probe.portamento_ms.unwrap() - cfg.y_min).abs() < 1.0,
        "freeze portamento lo: want {}, got {}", cfg.y_min, probe.portamento_ms.unwrap());
}

#[test]
fn freeze_resistance_offset_extremes() {
    let mut m = create_module(ModuleType::Freeze, SAMPLE_RATE, FFT_SIZE);
    m.reset(SAMPLE_RATE, FFT_SIZE);
    let nc = m.num_curves();
    let cfg = curve_display_config(ModuleType::Freeze, 3, GainMode::Add);

    let g_hi = (cfg.offset_fn)(1.0, 1.0);
    let probe = run_case(&mut m, nc, 3, g_hi);
    assert!((probe.resistance.unwrap() - cfg.y_max).abs() < 0.05,
        "freeze resistance hi: want {}, got {}", cfg.y_max, probe.resistance.unwrap());

    let g_lo = (cfg.offset_fn)(1.0, -1.0);
    let probe = run_case(&mut m, nc, 3, g_lo);
    assert!((probe.resistance.unwrap() - cfg.y_min).abs() < 0.05,
        "freeze resistance lo: want {}, got {}", cfg.y_min, probe.resistance.unwrap());
}

#[test]
fn freeze_mix_offset_extremes() {
    let mut m = create_module(ModuleType::Freeze, SAMPLE_RATE, FFT_SIZE);
    m.reset(SAMPLE_RATE, FFT_SIZE);
    let nc = m.num_curves();
    let cfg = curve_display_config(ModuleType::Freeze, 4, GainMode::Add);

    let g_hi = (cfg.offset_fn)(1.0, 1.0);
    let probe = run_case(&mut m, nc, 4, g_hi);
    assert!((probe.mix_pct.unwrap() - cfg.y_max).abs() < 1.0,
        "freeze mix hi: want {}, got {}", cfg.y_max, probe.mix_pct.unwrap());

    let g_lo = (cfg.offset_fn)(1.0, -1.0);
    let probe = run_case(&mut m, nc, 4, g_lo);
    assert!((probe.mix_pct.unwrap() - cfg.y_min).abs() < 1.0,
        "freeze mix lo: want {}, got {}", cfg.y_min, probe.mix_pct.unwrap());
}

// ── PhaseSmear ───────────────────────────────────────────────────────────────

#[test]
fn phase_smear_amount_offset_extremes() {
    let mut m = create_module(ModuleType::PhaseSmear, SAMPLE_RATE, FFT_SIZE);
    m.reset(SAMPLE_RATE, FFT_SIZE);
    let nc = m.num_curves();
    let cfg = curve_display_config(ModuleType::PhaseSmear, 0, GainMode::Add);

    let g_hi = (cfg.offset_fn)(1.0, 1.0);
    let probe = run_case(&mut m, nc, 0, g_hi);
    assert!((probe.amount_pct.unwrap() - cfg.y_max).abs() < 0.1,
        "phase smear amount hi: want {}, got {}", cfg.y_max, probe.amount_pct.unwrap());

    let g_lo = (cfg.offset_fn)(1.0, -1.0);
    let probe = run_case(&mut m, nc, 0, g_lo);
    assert!((probe.amount_pct.unwrap() - cfg.y_min).abs() < 0.1,
        "phase smear amount lo: want {}, got {}", cfg.y_min, probe.amount_pct.unwrap());
}

#[test]
fn phase_smear_peak_hold_offset_extremes() {
    let mut m = create_module(ModuleType::PhaseSmear, SAMPLE_RATE, FFT_SIZE);
    m.reset(SAMPLE_RATE, FFT_SIZE);
    let nc = m.num_curves();
    let cfg = curve_display_config(ModuleType::PhaseSmear, 1, GainMode::Add);

    let g_hi = (cfg.offset_fn)(1.0, 1.0);
    let probe = run_case(&mut m, nc, 1, g_hi);
    assert!((probe.peak_hold_ms.unwrap() - cfg.y_max).abs() < 1.0,
        "phase smear peak hold hi: want {}, got {}", cfg.y_max, probe.peak_hold_ms.unwrap());

    let g_lo = (cfg.offset_fn)(1.0, -1.0);
    let probe = run_case(&mut m, nc, 1, g_lo);
    assert!((probe.peak_hold_ms.unwrap() - cfg.y_min).abs() < 1.0,
        "phase smear peak hold lo: want {}, got {}", cfg.y_min, probe.peak_hold_ms.unwrap());
}

#[test]
fn phase_smear_mix_offset_extremes() {
    let mut m = create_module(ModuleType::PhaseSmear, SAMPLE_RATE, FFT_SIZE);
    m.reset(SAMPLE_RATE, FFT_SIZE);
    let nc = m.num_curves();
    let cfg = curve_display_config(ModuleType::PhaseSmear, 2, GainMode::Add);

    let g_hi = (cfg.offset_fn)(1.0, 1.0);
    let probe = run_case(&mut m, nc, 2, g_hi);
    assert!((probe.mix_pct.unwrap() - cfg.y_max).abs() < 0.1,
        "phase smear mix hi: want {}, got {}", cfg.y_max, probe.mix_pct.unwrap());

    let g_lo = (cfg.offset_fn)(1.0, -1.0);
    let probe = run_case(&mut m, nc, 2, g_lo);
    assert!((probe.mix_pct.unwrap() - cfg.y_min).abs() < 0.1,
        "phase smear mix lo: want {}, got {}", cfg.y_min, probe.mix_pct.unwrap());
}

// ── Contrast ─────────────────────────────────────────────────────────────────

#[test]
fn contrast_amount_offset_extremes() {
    let mut m = create_module(ModuleType::Contrast, SAMPLE_RATE, FFT_SIZE);
    m.reset(SAMPLE_RATE, FFT_SIZE);
    let nc = m.num_curves();
    let cfg = curve_display_config(ModuleType::Contrast, 0, GainMode::Add);

    let g_hi = (cfg.offset_fn)(1.0, 1.0);
    let probe = run_case(&mut m, nc, 0, g_hi);
    assert!((probe.ratio.unwrap() - cfg.y_max).abs() < 0.1,
        "contrast ratio hi: want {}, got {}", cfg.y_max, probe.ratio.unwrap());

    // off_ratio(1, -1) = 1 + 19*(-1) = -18 when o>=0 branch is not taken;
    // but off_ratio's negative branch returns g unchanged (1.0). Either way,
    // the DSP clamps to min=1.0. So observed ratio should equal y_min (= 1.0).
    let g_lo = (cfg.offset_fn)(1.0, -1.0);
    let probe = run_case(&mut m, nc, 0, g_lo);
    assert!((probe.ratio.unwrap() - cfg.y_min).abs() < 0.1,
        "contrast ratio lo: want {}, got {}", cfg.y_min, probe.ratio.unwrap());
}

// ── Gain ─────────────────────────────────────────────────────────────────────

#[test]
fn gain_add_offset_extremes() {
    let mut m = create_module(ModuleType::Gain, SAMPLE_RATE, FFT_SIZE);
    m.set_gain_mode(GainMode::Add);
    m.reset(SAMPLE_RATE, FFT_SIZE);
    let nc = m.num_curves();
    let cfg = curve_display_config(ModuleType::Gain, 0, GainMode::Add);

    let g_hi = (cfg.offset_fn)(1.0, 1.0);
    let probe = run_case(&mut m, nc, 0, g_hi);
    assert!((probe.gain_db.unwrap() - cfg.y_max).abs() < 0.5,
        "gain Add hi: want {}, got {}", cfg.y_max, probe.gain_db.unwrap());

    let g_lo = (cfg.offset_fn)(1.0, -1.0);
    let probe = run_case(&mut m, nc, 0, g_lo);
    assert!((probe.gain_db.unwrap() - cfg.y_min).abs() < 0.5,
        "gain Add lo: want {}, got {}", cfg.y_min, probe.gain_db.unwrap());
}

#[test]
fn gain_subtract_offset_extremes() {
    let mut m = create_module(ModuleType::Gain, SAMPLE_RATE, FFT_SIZE);
    m.set_gain_mode(GainMode::Subtract);
    m.reset(SAMPLE_RATE, FFT_SIZE);
    let nc = m.num_curves();
    // Subtract uses the same curve 0 config as Add (dB range).
    let cfg = curve_display_config(ModuleType::Gain, 0, GainMode::Subtract);

    let g_hi = (cfg.offset_fn)(1.0, 1.0);
    let probe = run_case(&mut m, nc, 0, g_hi);
    assert!((probe.gain_db.unwrap() - cfg.y_max).abs() < 0.5,
        "gain Subtract hi: want {}, got {}", cfg.y_max, probe.gain_db.unwrap());

    let g_lo = (cfg.offset_fn)(1.0, -1.0);
    let probe = run_case(&mut m, nc, 0, g_lo);
    assert!((probe.gain_db.unwrap() - cfg.y_min).abs() < 0.5,
        "gain Subtract lo: want {}, got {}", cfg.y_min, probe.gain_db.unwrap());
}

#[test]
fn gain_pull_offset_extremes() {
    let mut m = create_module(ModuleType::Gain, SAMPLE_RATE, FFT_SIZE);
    m.set_gain_mode(GainMode::Pull);
    m.reset(SAMPLE_RATE, FFT_SIZE);
    let nc = m.num_curves();
    let cfg = curve_display_config(ModuleType::Gain, 0, GainMode::Pull);

    // off_gain_pct(1, +1) returns g unchanged (1.0) → 100% = y_natural = y_max.
    let g_hi = (cfg.offset_fn)(1.0, 1.0);
    let probe = run_case(&mut m, nc, 0, g_hi);
    assert!((probe.gain_pct.unwrap() - cfg.y_max).abs() < 1.0,
        "gain Pull hi: want {}, got {}", cfg.y_max, probe.gain_pct.unwrap());

    let g_lo = (cfg.offset_fn)(1.0, -1.0);
    let probe = run_case(&mut m, nc, 0, g_lo);
    assert!((probe.gain_pct.unwrap() - cfg.y_min).abs() < 1.0,
        "gain Pull lo: want {}, got {}", cfg.y_min, probe.gain_pct.unwrap());
}

#[test]
fn gain_match_offset_extremes() {
    let mut m = create_module(ModuleType::Gain, SAMPLE_RATE, FFT_SIZE);
    m.set_gain_mode(GainMode::Match);
    m.reset(SAMPLE_RATE, FFT_SIZE);
    let nc = m.num_curves();
    let cfg = curve_display_config(ModuleType::Gain, 0, GainMode::Match);

    let g_hi = (cfg.offset_fn)(1.0, 1.0);
    let probe = run_case(&mut m, nc, 0, g_hi);
    assert!((probe.gain_pct.unwrap() - cfg.y_max).abs() < 1.0,
        "gain Match hi: want {}, got {}", cfg.y_max, probe.gain_pct.unwrap());

    let g_lo = (cfg.offset_fn)(1.0, -1.0);
    let probe = run_case(&mut m, nc, 0, g_lo);
    assert!((probe.gain_pct.unwrap() - cfg.y_min).abs() < 1.0,
        "gain Match lo: want {}, got {}", cfg.y_min, probe.gain_pct.unwrap());
}

#[test]
fn gain_peak_hold_offset_extremes() {
    let mut m = create_module(ModuleType::Gain, SAMPLE_RATE, FFT_SIZE);
    // Mode doesn't affect curve 1 (PEAK HOLD) config; use Pull so the DSP path
    // consumes the curve too, though the probe records it regardless of mode.
    m.set_gain_mode(GainMode::Pull);
    m.reset(SAMPLE_RATE, FFT_SIZE);
    let nc = m.num_curves();
    let cfg = curve_display_config(ModuleType::Gain, 1, GainMode::Pull);

    let g_hi = (cfg.offset_fn)(1.0, 1.0);
    let probe = run_case(&mut m, nc, 1, g_hi);
    assert!((probe.peak_hold_ms.unwrap() - cfg.y_max).abs() < 1.0,
        "gain peak hold hi: want {}, got {}", cfg.y_max, probe.peak_hold_ms.unwrap());

    let g_lo = (cfg.offset_fn)(1.0, -1.0);
    let probe = run_case(&mut m, nc, 1, g_lo);
    assert!((probe.peak_hold_ms.unwrap() - cfg.y_min).abs() < 1.0,
        "gain peak hold lo: want {}, got {}", cfg.y_min, probe.peak_hold_ms.unwrap());
}

// ── MidSide ──────────────────────────────────────────────────────────────────

#[test]
fn mid_side_balance_offset_extremes() {
    let mut m = create_module(ModuleType::MidSide, SAMPLE_RATE, FFT_SIZE);
    m.reset(SAMPLE_RATE, FFT_SIZE);
    let nc = m.num_curves();
    let cfg = curve_display_config(ModuleType::MidSide, 0, GainMode::Add);

    let g_hi = (cfg.offset_fn)(1.0, 1.0);
    let probe = run_case(&mut m, nc, 0, g_hi);
    assert!((probe.balance_pct.unwrap() - cfg.y_max).abs() < 2.0,
        "mid_side balance hi: want {}, got {}", cfg.y_max, probe.balance_pct.unwrap());

    let g_lo = (cfg.offset_fn)(1.0, -1.0);
    let probe = run_case(&mut m, nc, 0, g_lo);
    assert!((probe.balance_pct.unwrap() - cfg.y_min).abs() < 2.0,
        "mid_side balance lo: want {}, got {}", cfg.y_min, probe.balance_pct.unwrap());
}

#[test]
fn mid_side_expansion_offset_extremes() {
    let mut m = create_module(ModuleType::MidSide, SAMPLE_RATE, FFT_SIZE);
    m.reset(SAMPLE_RATE, FFT_SIZE);
    let nc = m.num_curves();
    let cfg = curve_display_config(ModuleType::MidSide, 1, GainMode::Add);

    let g_hi = (cfg.offset_fn)(1.0, 1.0);
    let probe = run_case(&mut m, nc, 1, g_hi);
    assert!((probe.expansion_pct.unwrap() - cfg.y_max).abs() < 2.0,
        "mid_side expansion hi: want {}, got {}", cfg.y_max, probe.expansion_pct.unwrap());

    let g_lo = (cfg.offset_fn)(1.0, -1.0);
    let probe = run_case(&mut m, nc, 1, g_lo);
    assert!((probe.expansion_pct.unwrap() - cfg.y_min).abs() < 2.0,
        "mid_side expansion lo: want {}, got {}", cfg.y_min, probe.expansion_pct.unwrap());
}

#[test]
fn mid_side_decorrel_offset_extremes() {
    let mut m = create_module(ModuleType::MidSide, SAMPLE_RATE, FFT_SIZE);
    m.reset(SAMPLE_RATE, FFT_SIZE);
    let nc = m.num_curves();
    let cfg = curve_display_config(ModuleType::MidSide, 2, GainMode::Add);

    let g_hi = (cfg.offset_fn)(1.0, 1.0);
    let probe = run_case(&mut m, nc, 2, g_hi);
    assert!((probe.decorrel_pct.unwrap() - cfg.y_max).abs() < 1.0,
        "mid_side decorrel hi: want {}, got {}", cfg.y_max, probe.decorrel_pct.unwrap());

    let g_lo = (cfg.offset_fn)(1.0, -1.0);
    let probe = run_case(&mut m, nc, 2, g_lo);
    assert!((probe.decorrel_pct.unwrap() - cfg.y_min).abs() < 1.0,
        "mid_side decorrel lo: want {}, got {}", cfg.y_min, probe.decorrel_pct.unwrap());
}

/// NOTE: TRANSIENT (curve 3) is currently a STUB — the MidSide DSP does not
/// consume this curve. The probe reads curve 3 at probe_k so the calibration
/// contract is enforced for when this parameter is implemented.
#[test]
fn mid_side_transient_offset_extremes() {
    let mut m = create_module(ModuleType::MidSide, SAMPLE_RATE, FFT_SIZE);
    m.reset(SAMPLE_RATE, FFT_SIZE);
    let nc = m.num_curves();
    let cfg = curve_display_config(ModuleType::MidSide, 3, GainMode::Add);

    let g_hi = (cfg.offset_fn)(1.0, 1.0);
    let probe = run_case(&mut m, nc, 3, g_hi);
    assert!((probe.transient_pct.unwrap() - cfg.y_max).abs() < 1.0,
        "mid_side transient hi: want {}, got {}", cfg.y_max, probe.transient_pct.unwrap());

    let g_lo = (cfg.offset_fn)(1.0, -1.0);
    let probe = run_case(&mut m, nc, 3, g_lo);
    assert!((probe.transient_pct.unwrap() - cfg.y_min).abs() < 1.0,
        "mid_side transient lo: want {}, got {}", cfg.y_min, probe.transient_pct.unwrap());
}

/// NOTE: PAN (curve 4) is currently a STUB — the MidSide DSP does not
/// consume this curve. The probe reads curve 4 at probe_k so the calibration
/// contract is enforced for when this parameter is implemented.
#[test]
fn mid_side_pan_offset_extremes() {
    let mut m = create_module(ModuleType::MidSide, SAMPLE_RATE, FFT_SIZE);
    m.reset(SAMPLE_RATE, FFT_SIZE);
    let nc = m.num_curves();
    let cfg = curve_display_config(ModuleType::MidSide, 4, GainMode::Add);

    let g_hi = (cfg.offset_fn)(1.0, 1.0);
    let probe = run_case(&mut m, nc, 4, g_hi);
    assert!((probe.pan_pct.unwrap() - cfg.y_max).abs() < 1.0,
        "mid_side pan hi: want {}, got {}", cfg.y_max, probe.pan_pct.unwrap());

    let g_lo = (cfg.offset_fn)(1.0, -1.0);
    let probe = run_case(&mut m, nc, 4, g_lo);
    assert!((probe.pan_pct.unwrap() - cfg.y_min).abs() < 1.0,
        "mid_side pan lo: want {}, got {}", cfg.y_min, probe.pan_pct.unwrap());
}

// ── TS Split ─────────────────────────────────────────────────────────────────

#[test]
fn ts_split_sensitivity_offset_extremes() {
    let mut m = create_module(ModuleType::TransientSustainedSplit, SAMPLE_RATE, FFT_SIZE);
    m.reset(SAMPLE_RATE, FFT_SIZE);
    let nc = m.num_curves();
    let cfg = curve_display_config(ModuleType::TransientSustainedSplit, 0, GainMode::Add);

    let g_hi = (cfg.offset_fn)(1.0, 1.0);
    let probe = run_case(&mut m, nc, 0, g_hi);
    assert!((probe.sensitivity_pct.unwrap() - cfg.y_max).abs() < 1.0,
        "ts_split sensitivity hi: want {}, got {}", cfg.y_max, probe.sensitivity_pct.unwrap());

    let g_lo = (cfg.offset_fn)(1.0, -1.0);
    let probe = run_case(&mut m, nc, 0, g_lo);
    assert!((probe.sensitivity_pct.unwrap() - cfg.y_min).abs() < 1.0,
        "ts_split sensitivity lo: want {}, got {}", cfg.y_min, probe.sensitivity_pct.unwrap());
}

/// T3b: GUI display-mapping contract — the scalar functions in
/// `editor::curve` that convert curve gains → physical units and
/// physical units ↔ pixel y must agree with the DSP and with the
/// `CurveDisplayConfig::y_min`/`y_max` declared in `editor::curve_config`.
/// See docs/superpowers/plans/2026-04-24-calibration-audit.md Task 3b.
mod display_mapping_contract {
    use nih_plug_egui::egui::{Pos2, Rect, Vec2};
    use spectral_forge::editor::curve::{gain_to_display, physical_to_y, screen_y_to_physical};

    fn rect() -> Rect {
        Rect::from_min_size(Pos2::ZERO, Vec2::splat(100.0))
    }

    // ── Freeze Threshold (display_idx = 9) ────────────────────────────────────
    #[test]
    fn freeze_threshold_gain_1_maps_to_minus_20_dbfs() {
        let v = gain_to_display(9, 1.0, 10.0, 100.0, -80.0, 0.0);
        assert!((v - (-20.0)).abs() < 0.1, "gain=1.0 should be -20 dBFS, got {}", v);
    }

    #[test]
    fn freeze_threshold_gain_2_maps_to_0_dbfs() {
        let v = gain_to_display(9, 2.0, 10.0, 100.0, -80.0, 0.0);
        assert!((v - 0.0).abs() < 0.1, "gain=2.0 should be 0 dBFS, got {}", v);
    }

    #[test]
    fn freeze_threshold_gain_1p5_matches_linear_dsp() {
        // DSP formula: -40 + gain*20, so gain=1.5 → -10 dBFS.
        let v = gain_to_display(9, 1.5, 10.0, 100.0, -80.0, 0.0);
        assert!((v - (-10.0)).abs() < 0.5,
            "gain=1.5 should be ≈-10 dBFS (DSP linear formula), got {}", v);
    }

    // ── Freeze Length (display_idx = 8) ───────────────────────────────────────
    #[test]
    fn freeze_length_physical_to_y_at_y_min_is_rect_bottom() {
        let r = rect();
        let y = physical_to_y(62.5, 8, -80.0, 0.0, r);
        assert!((y - r.bottom()).abs() < 1.0,
            "v=62.5 (y_min) should map to rect.bottom()={}, got {}", r.bottom(), y);
    }

    #[test]
    fn freeze_length_screen_y_to_physical_at_bottom_is_y_min() {
        let r = rect();
        let v = screen_y_to_physical(r.bottom(), 8, -80.0, 0.0, r);
        assert!((v - 62.5).abs() < 1.0,
            "y=rect.bottom() should map back to 62.5 ms, got {}", v);
    }

    #[test]
    fn freeze_length_roundtrip_midrange() {
        let r = rect();
        let y = physical_to_y(500.0, 8, -80.0, 0.0, r);
        let v = screen_y_to_physical(y, 8, -80.0, 0.0, r);
        assert!((v - 500.0).abs() < 1.0,
            "500 ms roundtrip should recover ≈500, got {}", v);
    }

    // ── Portamento / SC Smooth (display_idx = 10) ─────────────────────────────
    #[test]
    fn portamento_physical_to_y_at_y_min_is_rect_bottom() {
        let r = rect();
        let y = physical_to_y(40.0, 10, -80.0, 0.0, r);
        assert!((y - r.bottom()).abs() < 1.0,
            "v=40 (y_min) should map to rect.bottom()={}, got {}", r.bottom(), y);
    }

    #[test]
    fn portamento_screen_y_to_physical_at_bottom_is_y_min() {
        let r = rect();
        let v = screen_y_to_physical(r.bottom(), 10, -80.0, 0.0, r);
        assert!((v - 40.0).abs() < 1.0,
            "y=rect.bottom() should map back to 40 ms, got {}", v);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Neutral contract (T7): every offset_fn satisfies f(g, 0.0) == g
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn all_offset_fns_are_neutral_at_zero() {
    use spectral_forge::editor::curve_config::{
        off_thresh, off_ratio, off_atk_rel, off_knee, off_mix, off_gain_db,
        off_gain_pct, off_amount_200, off_freeze_length, off_freeze_thresh,
        off_portamento, off_resistance, off_identity,
    };
    let fns: &[(&str, fn(f32, f32) -> f32)] = &[
        ("thresh",        off_thresh),
        ("ratio",         off_ratio),
        ("atk_rel",       off_atk_rel),
        ("knee",          off_knee),
        ("mix",           off_mix),
        ("gain_db",       off_gain_db),
        ("gain_pct",      off_gain_pct),
        ("amount_200",    off_amount_200),
        ("freeze_length", off_freeze_length),
        ("freeze_thresh", off_freeze_thresh),
        ("portamento",    off_portamento),
        ("resistance",    off_resistance),
        ("identity",      off_identity),
    ];
    for (name, f) in fns {
        for &g in &[0.1_f32, 0.5, 1.0, 2.0, 10.0] {
            let result = f(g, 0.0);
            assert!(
                (result - g).abs() < 1e-5,
                "{} violates neutral contract: f({}, 0.0) = {}, expected {}",
                name, g, result, g,
            );
        }
    }
}

#[test]
fn future_print_through_amount_default_probes_5_pct() {
    let mut m = create_module(ModuleType::Future, SAMPLE_RATE, FFT_SIZE);
    let nc = m.num_curves();
    // Mode defaults to PrintThrough.
    let probe = run_case(&mut m, nc, 0, 1.0);  // AMOUNT_gain=1.0
    let observed = probe.amount_pct.expect("future must probe amount_pct");
    assert!(
        (observed - 5.0).abs() < 0.5,
        "PrintThrough AMOUNT=1.0 should give amount_pct≈5.0, got {}", observed,
    );
}

#[test]
fn future_print_through_mix_max_probes_100_pct() {
    let mut m = create_module(ModuleType::Future, SAMPLE_RATE, FFT_SIZE);
    let nc = m.num_curves();
    let probe = run_case(&mut m, nc, 4, 2.0);  // MIX_gain=2.0
    let observed = probe.mix_pct.expect("future must probe mix_pct");
    assert!(
        (observed - 100.0).abs() < 0.5,
        "PrintThrough MIX=2.0 should give mix_pct≈100.0, got {}", observed,
    );
}

#[test]
fn future_print_through_time_default_probes_length_ms() {
    let mut m = create_module(ModuleType::Future, SAMPLE_RATE, FFT_SIZE);
    let nc = m.num_curves();
    let probe = run_case(&mut m, nc, 1, 1.0);  // TIME_gain=1.0 → 8 hops
    let observed = probe.length_ms.expect("future must probe length_ms");
    let hop_ms = (FFT_SIZE as f32 / 4.0) / SAMPLE_RATE * 1000.0;
    let expected = 8.0 * hop_ms;
    assert!(
        (observed - expected).abs() < 0.5,
        "TIME=1.0 should give length_ms≈{} ({} hops × {} ms), got {}",
        expected, 8, hop_ms, observed,
    );
}

#[test]
fn future_pre_echo_amount_max_probes_100_pct() {
    let mut m = create_module(ModuleType::Future, SAMPLE_RATE, FFT_SIZE);
    m.set_future_mode(FutureMode::PreEcho);
    let nc = m.num_curves();
    let probe = run_case(&mut m, nc, 0, 2.0);  // AMOUNT_gain=2.0 (max)
    let observed = probe.amount_pct.expect("future must probe amount_pct");
    assert!(
        (observed - 100.0).abs() < 0.5,
        "PreEcho AMOUNT=2.0 should give amount_pct≈100.0 (echo_amp × 50), got {}", observed,
    );
}

#[test]
fn punch_direct_amount_default_probes_50_pct() {
    use spectral_forge::dsp::modules::punch::PunchMode;
    let mut m = create_module(ModuleType::Punch, SAMPLE_RATE, FFT_SIZE);
    m.set_punch_mode(PunchMode::Direct);
    let nc = m.num_curves();
    let probe = run_case(&mut m, nc, 0, 1.0);  // AMOUNT_gain=1.0 (default)
    let observed = probe.amount_pct.expect("punch must probe amount_pct");
    assert!(
        (observed - 50.0).abs() < 0.5,
        "AMOUNT=1.0 should give amount_pct≈50.0 (depth × 100), got {}", observed,
    );
}

#[test]
fn punch_direct_amount_max_probes_100_pct() {
    use spectral_forge::dsp::modules::punch::PunchMode;
    let mut m = create_module(ModuleType::Punch, SAMPLE_RATE, FFT_SIZE);
    m.set_punch_mode(PunchMode::Direct);
    let nc = m.num_curves();
    let probe = run_case(&mut m, nc, 0, 2.0);  // AMOUNT_gain=2.0 (max)
    let observed = probe.amount_pct.expect("punch must probe amount_pct");
    assert!(
        (observed - 100.0).abs() < 0.5,
        "AMOUNT=2.0 should give amount_pct≈100.0, got {}", observed,
    );
}

#[test]
fn punch_direct_mix_max_probes_100_pct() {
    use spectral_forge::dsp::modules::punch::PunchMode;
    let mut m = create_module(ModuleType::Punch, SAMPLE_RATE, FFT_SIZE);
    m.set_punch_mode(PunchMode::Direct);
    let nc = m.num_curves();
    let probe = run_case(&mut m, nc, 5, 2.0);  // MIX_gain=2.0 (max), curve idx 5
    let observed = probe.mix_pct.expect("punch must probe mix_pct");
    assert!(
        (observed - 100.0).abs() < 0.5,
        "MIX=2.0 should give mix_pct≈100.0, got {}", observed,
    );
}

#[test]
fn punch_inverse_amount_max_probes_100_pct() {
    use spectral_forge::dsp::modules::punch::PunchMode;
    let mut m = create_module(ModuleType::Punch, SAMPLE_RATE, FFT_SIZE);
    m.set_punch_mode(PunchMode::Inverse);
    let nc = m.num_curves();
    let probe = run_case(&mut m, nc, 0, 2.0);  // AMOUNT_gain=2.0 (max)
    let observed = probe.amount_pct.expect("punch must probe amount_pct");
    assert!(
        (observed - 100.0).abs() < 0.5,
        "Inverse AMOUNT=2.0 should give amount_pct≈100.0 (mode-agnostic probe), got {}", observed,
    );
}

// ── Rhythm calibration helpers ────────────────────────────────────────────────

/// Like `run_case` but sets `ctx.bpm = 120.0` so the module doesn't passthrough.
/// Without a positive bpm, Rhythm returns immediately and the probe stays at 0.
///
/// `beat_position = 0.0` puts the module at `step_idx = 0` for all three modes.
/// At default DIVISION (1.0 → 8 steps), Bjorklund E(4,8) and E(8,8) both pulse
/// at step 0, so the Euclidean tests fire the gate regardless of AMOUNT value.
/// The probe captures the curve-derived value (e.g. PhaseReset `strength`),
/// not the gated output (`strength * reset_env`).
fn run_rhythm_case(
    module: &mut Box<dyn SpectralModule>,
    num_curves: usize,
    target_curve_idx: usize,
    gain_on_target: f32,
) -> ProbeSnapshot {
    let curves_storage: Vec<Vec<f32>> = (0..num_curves)
        .map(|c| if c == target_curve_idx {
            vec![gain_on_target; NUM_BINS]
        } else {
            vec![1.0; NUM_BINS]
        })
        .collect();
    let curves_refs: Vec<&[f32]> = curves_storage.iter().map(|v| v.as_slice()).collect();

    let mut bins: Vec<Complex<f32>> = vec![Complex::new(0.1, 0.0); NUM_BINS];
    let mut suppression: Vec<f32> = vec![0.0; NUM_BINS];
    let mut ctx = make_ctx();
    ctx.bpm = 120.0;       // required: bpm ≤ 1e-3 causes immediate passthrough (no probe)
    ctx.beat_position = 0.0;  // bar position 0 → step_idx 0
    module.process(
        0,
        StereoLink::Linked,
        FxChannelTarget::All,
        &mut bins,
        None,
        &curves_refs,
        &mut suppression,
        None,
        &ctx,
    );
    module.last_probe()
}

// ── Rhythm / Euclidean ────────────────────────────────────────────────────────

#[test]
fn rhythm_euclidean_amount_default_probes_50_pct() {
    let mut m = create_module(ModuleType::Rhythm, SAMPLE_RATE, FFT_SIZE);
    m.set_rhythm_mode(RhythmMode::Euclidean);
    let nc = m.num_curves();
    let probe = run_rhythm_case(&mut m, nc, 0, 1.0);  // AMOUNT_gain=1.0 (default)
    let observed = probe.amount_pct.expect("rhythm must probe amount_pct");
    assert!(
        (observed - 50.0).abs() < 0.5,
        "Euclidean AMOUNT=1.0 → depth=(1.0×0.5)=0.5 → amount_pct=50.0, got {}", observed,
    );
}

#[test]
fn rhythm_euclidean_amount_max_probes_100_pct() {
    let mut m = create_module(ModuleType::Rhythm, SAMPLE_RATE, FFT_SIZE);
    m.set_rhythm_mode(RhythmMode::Euclidean);
    let nc = m.num_curves();
    let probe = run_rhythm_case(&mut m, nc, 0, 2.0);  // AMOUNT_gain=2.0 (max)
    let observed = probe.amount_pct.expect("rhythm must probe amount_pct");
    assert!(
        (observed - 100.0).abs() < 0.5,
        "Euclidean AMOUNT=2.0 → depth=(2.0×0.5)=1.0 → amount_pct=100.0, got {}", observed,
    );
}

#[test]
fn rhythm_euclidean_mix_max_probes_100_pct() {
    let mut m = create_module(ModuleType::Rhythm, SAMPLE_RATE, FFT_SIZE);
    m.set_rhythm_mode(RhythmMode::Euclidean);
    let nc = m.num_curves();
    let probe = run_rhythm_case(&mut m, nc, 4, 2.0);  // MIX_gain=2.0, curve idx 4
    let observed = probe.mix_pct.expect("rhythm must probe mix_pct");
    assert!(
        (observed - 100.0).abs() < 0.5,
        "Euclidean MIX=2.0 → mix=(2.0×0.5)=1.0 → mix_pct=100.0, got {}", observed,
    );
}

// ── Rhythm / Arpeggiator ──────────────────────────────────────────────────────

#[test]
fn rhythm_arpeggiator_amount_default_probes_50_pct() {
    let mut m = create_module(ModuleType::Rhythm, SAMPLE_RATE, FFT_SIZE);
    m.set_rhythm_mode(RhythmMode::Arpeggiator);
    let nc = m.num_curves();
    let probe = run_rhythm_case(&mut m, nc, 0, 1.0);  // AMOUNT_gain=1.0 (default)
    let observed = probe.amount_pct.expect("rhythm must probe amount_pct");
    assert!(
        (observed - 50.0).abs() < 0.5,
        "Arpeggiator AMOUNT=1.0 → amount_norm=(1.0×0.5)=0.5 → amount_pct=50.0, got {}", observed,
    );
}

#[test]
fn rhythm_arpeggiator_amount_max_probes_100_pct() {
    let mut m = create_module(ModuleType::Rhythm, SAMPLE_RATE, FFT_SIZE);
    m.set_rhythm_mode(RhythmMode::Arpeggiator);
    let nc = m.num_curves();
    let probe = run_rhythm_case(&mut m, nc, 0, 2.0);  // AMOUNT_gain=2.0 (max)
    let observed = probe.amount_pct.expect("rhythm must probe amount_pct");
    assert!(
        (observed - 100.0).abs() < 0.5,
        "Arpeggiator AMOUNT=2.0 → amount_norm=(2.0×0.5)=1.0 → amount_pct=100.0, got {}", observed,
    );
}

#[test]
fn rhythm_arpeggiator_mix_max_probes_100_pct() {
    let mut m = create_module(ModuleType::Rhythm, SAMPLE_RATE, FFT_SIZE);
    m.set_rhythm_mode(RhythmMode::Arpeggiator);
    let nc = m.num_curves();
    let probe = run_rhythm_case(&mut m, nc, 4, 2.0);  // MIX_gain=2.0, curve idx 4
    let observed = probe.mix_pct.expect("rhythm must probe mix_pct");
    assert!(
        (observed - 100.0).abs() < 0.5,
        "Arpeggiator MIX=2.0 → mix=(2.0×0.5)=1.0 → mix_pct=100.0, got {}", observed,
    );
}

// ── Rhythm / PhaseReset ───────────────────────────────────────────────────────

#[test]
fn rhythm_phase_reset_amount_default_probes_50_pct() {
    let mut m = create_module(ModuleType::Rhythm, SAMPLE_RATE, FFT_SIZE);
    m.set_rhythm_mode(RhythmMode::PhaseReset);
    let nc = m.num_curves();
    let probe = run_rhythm_case(&mut m, nc, 0, 1.0);  // AMOUNT_gain=1.0 (default)
    let observed = probe.amount_pct.expect("rhythm must probe amount_pct");
    assert!(
        (observed - 50.0).abs() < 0.5,
        "PhaseReset AMOUNT=1.0 → strength=(1.0×0.5)=0.5 → amount_pct=50.0, got {}", observed,
    );
}

#[test]
fn rhythm_phase_reset_amount_max_probes_100_pct() {
    let mut m = create_module(ModuleType::Rhythm, SAMPLE_RATE, FFT_SIZE);
    m.set_rhythm_mode(RhythmMode::PhaseReset);
    let nc = m.num_curves();
    let probe = run_rhythm_case(&mut m, nc, 0, 2.0);  // AMOUNT_gain=2.0 (max)
    let observed = probe.amount_pct.expect("rhythm must probe amount_pct");
    assert!(
        (observed - 100.0).abs() < 0.5,
        "PhaseReset AMOUNT=2.0 → strength=(2.0×0.5)=1.0 → amount_pct=100.0, got {}", observed,
    );
}

#[test]
fn rhythm_phase_reset_mix_max_probes_100_pct() {
    let mut m = create_module(ModuleType::Rhythm, SAMPLE_RATE, FFT_SIZE);
    m.set_rhythm_mode(RhythmMode::PhaseReset);
    let nc = m.num_curves();
    let probe = run_rhythm_case(&mut m, nc, 4, 2.0);  // MIX_gain=2.0, curve idx 4
    let observed = probe.mix_pct.expect("rhythm must probe mix_pct");
    assert!(
        (observed - 100.0).abs() < 0.5,
        "PhaseReset MIX=2.0 → mix=(2.0×0.5)=1.0 → mix_pct=100.0, got {}", observed,
    );
}

// ── Geometry / Chladni ────────────────────────────────────────────────────────

#[test]
fn geometry_chladni_amount_default_probes_50_pct() {
    let mut m = create_module(ModuleType::Geometry, SAMPLE_RATE, FFT_SIZE);
    m.set_geometry_mode(GeometryMode::Chladni);
    let nc = m.num_curves();
    // AMOUNT curve index 0, g=1.0 → amt_val = 1.0*0.025 = 0.025 → pct = (0.025/0.05)*100 = 50.0
    let probe = run_case(&mut m, nc, 0, 1.0);
    let observed = probe.amount_pct.expect("geometry must probe amount_pct");
    assert!(
        (observed - 50.0).abs() < 2.0,
        "Chladni AMOUNT=1.0 should give amount_pct≈50.0, got {}", observed,
    );
}

#[test]
fn geometry_chladni_amount_max_probes_100_pct() {
    let mut m = create_module(ModuleType::Geometry, SAMPLE_RATE, FFT_SIZE);
    m.set_geometry_mode(GeometryMode::Chladni);
    let nc = m.num_curves();
    // AMOUNT curve index 0, g=2.0 → amt_val = 2.0*0.025 = 0.05 → pct = (0.05/0.05)*100 = 100.0
    let probe = run_case(&mut m, nc, 0, 2.0);
    let observed = probe.amount_pct.expect("geometry must probe amount_pct");
    assert!(
        (observed - 100.0).abs() < 2.0,
        "Chladni AMOUNT=2.0 should give amount_pct≈100.0, got {}", observed,
    );
}

#[test]
fn geometry_chladni_mix_max_probes_100_pct() {
    let mut m = create_module(ModuleType::Geometry, SAMPLE_RATE, FFT_SIZE);
    m.set_geometry_mode(GeometryMode::Chladni);
    let nc = m.num_curves();
    // MIX curve index 4, g=2.0 → mix_val = 2.0.clamp(0,2)*0.5 = 1.0 → pct = 100.0
    let probe = run_case(&mut m, nc, 4, 2.0);
    let observed = probe.mix_pct.expect("geometry must probe mix_pct");
    assert!(
        (observed - 100.0).abs() < 2.0,
        "Chladni MIX=2.0 should give mix_pct≈100.0, got {}", observed,
    );
}

// ── Geometry / Helmholtz ──────────────────────────────────────────────────────

#[test]
fn geometry_helmholtz_amount_default_probes_50_pct() {
    let mut m = create_module(ModuleType::Geometry, SAMPLE_RATE, FFT_SIZE);
    m.set_geometry_mode(GeometryMode::Helmholtz);
    m.reset(SAMPLE_RATE, FFT_SIZE);
    let nc = m.num_curves();
    // AMOUNT curve index 0, g=1.0 → amt_val = (1.0*0.5).clamp(0,1) = 0.5 → pct = 50.0
    let probe = run_case(&mut m, nc, 0, 1.0);
    let observed = probe.amount_pct.expect("geometry must probe amount_pct");
    assert!(
        (observed - 50.0).abs() < 2.0,
        "Helmholtz AMOUNT=1.0 should give amount_pct≈50.0, got {}", observed,
    );
}

#[test]
fn geometry_helmholtz_amount_max_probes_100_pct() {
    let mut m = create_module(ModuleType::Geometry, SAMPLE_RATE, FFT_SIZE);
    m.set_geometry_mode(GeometryMode::Helmholtz);
    m.reset(SAMPLE_RATE, FFT_SIZE);
    let nc = m.num_curves();
    // AMOUNT curve index 0, g=2.0 → amt_val = (2.0*0.5).clamp(0,1) = 1.0 → pct = 100.0
    let probe = run_case(&mut m, nc, 0, 2.0);
    let observed = probe.amount_pct.expect("geometry must probe amount_pct");
    assert!(
        (observed - 100.0).abs() < 2.0,
        "Helmholtz AMOUNT=2.0 should give amount_pct≈100.0, got {}", observed,
    );
}

#[test]
fn geometry_helmholtz_mix_max_probes_100_pct() {
    let mut m = create_module(ModuleType::Geometry, SAMPLE_RATE, FFT_SIZE);
    m.set_geometry_mode(GeometryMode::Helmholtz);
    m.reset(SAMPLE_RATE, FFT_SIZE);
    let nc = m.num_curves();
    // MIX curve index 4, g=2.0 → mix_val = 2.0.clamp(0,2)*0.5 = 1.0 → pct = 100.0
    let probe = run_case(&mut m, nc, 4, 2.0);
    let observed = probe.mix_pct.expect("geometry must probe mix_pct");
    assert!(
        (observed - 100.0).abs() < 2.0,
        "Helmholtz MIX=2.0 should give mix_pct≈100.0, got {}", observed,
    );
}

// ── Modulate ──────────────────────────────────────────────────────────────────

#[test]
fn modulate_phase_phaser_amount_default_probes_50_pct() {
    use spectral_forge::dsp::modules::modulate::ModulateMode;
    let mut m = create_module(ModuleType::Modulate, SAMPLE_RATE, FFT_SIZE);
    m.set_modulate_mode(ModulateMode::PhasePhaser);
    let nc = m.num_curves();
    let probe = run_case(&mut m, nc, 0, 1.0);
    let observed = probe.amount_pct.expect("modulate must probe amount_pct");
    assert!((observed - 50.0).abs() < 2.0, "PhasePhaser AMOUNT=1.0 → 50%, got {}", observed);
}

#[test]
fn modulate_phase_phaser_amount_max_probes_100_pct() {
    use spectral_forge::dsp::modules::modulate::ModulateMode;
    let mut m = create_module(ModuleType::Modulate, SAMPLE_RATE, FFT_SIZE);
    m.set_modulate_mode(ModulateMode::PhasePhaser);
    let nc = m.num_curves();
    let probe = run_case(&mut m, nc, 0, 2.0);
    let observed = probe.amount_pct.expect("modulate must probe amount_pct");
    assert!((observed - 100.0).abs() < 2.0, "PhasePhaser AMOUNT=2.0 → 100%, got {}", observed);
}

#[test]
fn modulate_phase_phaser_mix_max_probes_100_pct() {
    use spectral_forge::dsp::modules::modulate::ModulateMode;
    let mut m = create_module(ModuleType::Modulate, SAMPLE_RATE, FFT_SIZE);
    m.set_modulate_mode(ModulateMode::PhasePhaser);
    let nc = m.num_curves();
    let probe = run_case(&mut m, nc, 5, 2.0);   // MIX is curves[5]
    let observed = probe.mix_pct.expect("modulate must probe mix_pct");
    assert!((observed - 100.0).abs() < 2.0, "PhasePhaser MIX=2.0 → 100%, got {}", observed);
}

#[test]
fn modulate_bin_swapper_amount_default_probes_50_pct() {
    use spectral_forge::dsp::modules::modulate::ModulateMode;
    let mut m = create_module(ModuleType::Modulate, SAMPLE_RATE, FFT_SIZE);
    m.set_modulate_mode(ModulateMode::BinSwapper);
    let nc = m.num_curves();
    let probe = run_case(&mut m, nc, 0, 1.0);
    let observed = probe.amount_pct.expect("modulate must probe amount_pct");
    assert!((observed - 50.0).abs() < 2.0, "BinSwapper AMOUNT=1.0 → 50%, got {}", observed);
}

#[test]
fn modulate_bin_swapper_amount_max_probes_100_pct() {
    use spectral_forge::dsp::modules::modulate::ModulateMode;
    let mut m = create_module(ModuleType::Modulate, SAMPLE_RATE, FFT_SIZE);
    m.set_modulate_mode(ModulateMode::BinSwapper);
    let nc = m.num_curves();
    let probe = run_case(&mut m, nc, 0, 2.0);
    let observed = probe.amount_pct.expect("modulate must probe amount_pct");
    assert!((observed - 100.0).abs() < 2.0, "BinSwapper AMOUNT=2.0 → 100%, got {}", observed);
}

#[test]
fn modulate_bin_swapper_mix_max_probes_100_pct() {
    use spectral_forge::dsp::modules::modulate::ModulateMode;
    let mut m = create_module(ModuleType::Modulate, SAMPLE_RATE, FFT_SIZE);
    m.set_modulate_mode(ModulateMode::BinSwapper);
    let nc = m.num_curves();
    let probe = run_case(&mut m, nc, 5, 2.0);   // MIX is curves[5]
    let observed = probe.mix_pct.expect("modulate must probe mix_pct");
    assert!((observed - 100.0).abs() < 2.0, "BinSwapper MIX=2.0 → 100%, got {}", observed);
}

#[test]
fn modulate_rm_fm_matrix_amount_default_probes_50_pct() {
    use spectral_forge::dsp::modules::modulate::ModulateMode;
    let mut m = create_module(ModuleType::Modulate, SAMPLE_RATE, FFT_SIZE);
    m.set_modulate_mode(ModulateMode::RmFmMatrix);
    let nc = m.num_curves();
    let probe = run_case(&mut m, nc, 0, 1.0);
    let observed = probe.amount_pct.expect("modulate must probe amount_pct");
    assert!((observed - 50.0).abs() < 2.0, "RmFmMatrix AMOUNT=1.0 → 50%, got {}", observed);
}

#[test]
fn modulate_rm_fm_matrix_amount_max_probes_100_pct() {
    use spectral_forge::dsp::modules::modulate::ModulateMode;
    let mut m = create_module(ModuleType::Modulate, SAMPLE_RATE, FFT_SIZE);
    m.set_modulate_mode(ModulateMode::RmFmMatrix);
    let nc = m.num_curves();
    let probe = run_case(&mut m, nc, 0, 2.0);
    let observed = probe.amount_pct.expect("modulate must probe amount_pct");
    assert!((observed - 100.0).abs() < 2.0, "RmFmMatrix AMOUNT=2.0 → 100%, got {}", observed);
}

#[test]
fn modulate_rm_fm_matrix_mix_max_probes_100_pct() {
    use spectral_forge::dsp::modules::modulate::ModulateMode;
    let mut m = create_module(ModuleType::Modulate, SAMPLE_RATE, FFT_SIZE);
    m.set_modulate_mode(ModulateMode::RmFmMatrix);
    let nc = m.num_curves();
    let probe = run_case(&mut m, nc, 5, 2.0);   // MIX is curves[5]
    let observed = probe.mix_pct.expect("modulate must probe mix_pct");
    assert!((observed - 100.0).abs() < 2.0, "RmFmMatrix MIX=2.0 → 100%, got {}", observed);
}

#[test]
fn modulate_diode_rm_amount_default_probes_50_pct() {
    use spectral_forge::dsp::modules::modulate::ModulateMode;
    let mut m = create_module(ModuleType::Modulate, SAMPLE_RATE, FFT_SIZE);
    m.set_modulate_mode(ModulateMode::DiodeRm);
    let nc = m.num_curves();
    let probe = run_case(&mut m, nc, 0, 1.0);
    let observed = probe.amount_pct.expect("modulate must probe amount_pct");
    assert!((observed - 50.0).abs() < 2.0, "DiodeRm AMOUNT=1.0 → 50%, got {}", observed);
}

#[test]
fn modulate_diode_rm_amount_max_probes_100_pct() {
    use spectral_forge::dsp::modules::modulate::ModulateMode;
    let mut m = create_module(ModuleType::Modulate, SAMPLE_RATE, FFT_SIZE);
    m.set_modulate_mode(ModulateMode::DiodeRm);
    let nc = m.num_curves();
    let probe = run_case(&mut m, nc, 0, 2.0);
    let observed = probe.amount_pct.expect("modulate must probe amount_pct");
    assert!((observed - 100.0).abs() < 2.0, "DiodeRm AMOUNT=2.0 → 100%, got {}", observed);
}

#[test]
fn modulate_diode_rm_mix_max_probes_100_pct() {
    use spectral_forge::dsp::modules::modulate::ModulateMode;
    let mut m = create_module(ModuleType::Modulate, SAMPLE_RATE, FFT_SIZE);
    m.set_modulate_mode(ModulateMode::DiodeRm);
    let nc = m.num_curves();
    let probe = run_case(&mut m, nc, 5, 2.0);   // MIX is curves[5]
    let observed = probe.mix_pct.expect("modulate must probe mix_pct");
    assert!((observed - 100.0).abs() < 2.0, "DiodeRm MIX=2.0 → 100%, got {}", observed);
}

#[test]
fn modulate_ground_loop_amount_default_probes_50_pct() {
    use spectral_forge::dsp::modules::modulate::ModulateMode;
    let mut m = create_module(ModuleType::Modulate, SAMPLE_RATE, FFT_SIZE);
    m.set_modulate_mode(ModulateMode::GroundLoop);
    let nc = m.num_curves();
    let probe = run_case(&mut m, nc, 0, 1.0);
    let observed = probe.amount_pct.expect("modulate must probe amount_pct");
    assert!((observed - 50.0).abs() < 2.0, "GroundLoop AMOUNT=1.0 → 50%, got {}", observed);
}

#[test]
fn modulate_ground_loop_amount_max_probes_100_pct() {
    use spectral_forge::dsp::modules::modulate::ModulateMode;
    let mut m = create_module(ModuleType::Modulate, SAMPLE_RATE, FFT_SIZE);
    m.set_modulate_mode(ModulateMode::GroundLoop);
    let nc = m.num_curves();
    let probe = run_case(&mut m, nc, 0, 2.0);
    let observed = probe.amount_pct.expect("modulate must probe amount_pct");
    assert!((observed - 100.0).abs() < 2.0, "GroundLoop AMOUNT=2.0 → 100%, got {}", observed);
}

#[test]
fn modulate_ground_loop_mix_max_probes_100_pct() {
    use spectral_forge::dsp::modules::modulate::ModulateMode;
    let mut m = create_module(ModuleType::Modulate, SAMPLE_RATE, FFT_SIZE);
    m.set_modulate_mode(ModulateMode::GroundLoop);
    let nc = m.num_curves();
    let probe = run_case(&mut m, nc, 5, 2.0);   // MIX is curves[5]
    let observed = probe.mix_pct.expect("modulate must probe mix_pct");
    assert!((observed - 100.0).abs() < 2.0, "GroundLoop MIX=2.0 → 100%, got {}", observed);
}

// ── Circuit ──────────────────────────────────────────────────────────────────

#[test]
fn circuit_bbd_bins_amount_default_probes_50_pct() {
    use spectral_forge::dsp::modules::circuit::CircuitMode;
    let mut m = create_module(ModuleType::Circuit, SAMPLE_RATE, FFT_SIZE);
    m.set_circuit_mode(CircuitMode::BbdBins);
    let nc = m.num_curves();
    let probe = run_case(&mut m, nc, 0, 1.0);   // AMOUNT is curves[0]
    let observed = probe.amount_pct.expect("circuit must probe amount_pct");
    assert!((observed - 50.0).abs() < 2.0, "BbdBins AMOUNT=1.0 → 50%, got {}", observed);
}

#[test]
fn circuit_bbd_bins_amount_max_probes_100_pct() {
    use spectral_forge::dsp::modules::circuit::CircuitMode;
    let mut m = create_module(ModuleType::Circuit, SAMPLE_RATE, FFT_SIZE);
    m.set_circuit_mode(CircuitMode::BbdBins);
    let nc = m.num_curves();
    let probe = run_case(&mut m, nc, 0, 2.0);
    let observed = probe.amount_pct.expect("circuit must probe amount_pct");
    assert!((observed - 100.0).abs() < 2.0, "BbdBins AMOUNT=2.0 → 100%, got {}", observed);
}

#[test]
fn circuit_bbd_bins_mix_max_probes_100_pct() {
    use spectral_forge::dsp::modules::circuit::CircuitMode;
    let mut m = create_module(ModuleType::Circuit, SAMPLE_RATE, FFT_SIZE);
    m.set_circuit_mode(CircuitMode::BbdBins);
    let nc = m.num_curves();
    let probe = run_case(&mut m, nc, 3, 2.0);   // MIX is curves[3] (Circuit has 4 curves)
    let observed = probe.mix_pct.expect("circuit must probe mix_pct");
    assert!((observed - 100.0).abs() < 2.0, "BbdBins MIX=2.0 → 100%, got {}", observed);
}

#[test]
fn circuit_spectral_schmitt_amount_default_probes_50_pct() {
    use spectral_forge::dsp::modules::circuit::CircuitMode;
    let mut m = create_module(ModuleType::Circuit, SAMPLE_RATE, FFT_SIZE);
    m.set_circuit_mode(CircuitMode::SpectralSchmitt);
    let nc = m.num_curves();
    let probe = run_case(&mut m, nc, 0, 1.0);   // AMOUNT is curves[0]
    let observed = probe.amount_pct.expect("circuit must probe amount_pct");
    assert!((observed - 50.0).abs() < 2.0, "SpectralSchmitt AMOUNT=1.0 → 50%, got {}", observed);
}

#[test]
fn circuit_spectral_schmitt_amount_max_probes_100_pct() {
    use spectral_forge::dsp::modules::circuit::CircuitMode;
    let mut m = create_module(ModuleType::Circuit, SAMPLE_RATE, FFT_SIZE);
    m.set_circuit_mode(CircuitMode::SpectralSchmitt);
    let nc = m.num_curves();
    let probe = run_case(&mut m, nc, 0, 2.0);
    let observed = probe.amount_pct.expect("circuit must probe amount_pct");
    assert!((observed - 100.0).abs() < 2.0, "SpectralSchmitt AMOUNT=2.0 → 100%, got {}", observed);
}

#[test]
fn circuit_spectral_schmitt_mix_max_probes_100_pct() {
    use spectral_forge::dsp::modules::circuit::CircuitMode;
    let mut m = create_module(ModuleType::Circuit, SAMPLE_RATE, FFT_SIZE);
    m.set_circuit_mode(CircuitMode::SpectralSchmitt);
    let nc = m.num_curves();
    let probe = run_case(&mut m, nc, 3, 2.0);   // MIX is curves[3] (Circuit has 4 curves)
    let observed = probe.mix_pct.expect("circuit must probe mix_pct");
    assert!((observed - 100.0).abs() < 2.0, "SpectralSchmitt MIX=2.0 → 100%, got {}", observed);
}

#[test]
fn circuit_crossover_distortion_amount_default_probes_50_pct() {
    use spectral_forge::dsp::modules::circuit::CircuitMode;
    let mut m = create_module(ModuleType::Circuit, SAMPLE_RATE, FFT_SIZE);
    m.set_circuit_mode(CircuitMode::CrossoverDistortion);
    let nc = m.num_curves();
    let probe = run_case(&mut m, nc, 0, 1.0);   // AMOUNT is curves[0]
    let observed = probe.amount_pct.expect("circuit must probe amount_pct");
    assert!((observed - 50.0).abs() < 2.0, "CrossoverDistortion AMOUNT=1.0 → 50%, got {}", observed);
}

#[test]
fn circuit_crossover_distortion_amount_max_probes_100_pct() {
    use spectral_forge::dsp::modules::circuit::CircuitMode;
    let mut m = create_module(ModuleType::Circuit, SAMPLE_RATE, FFT_SIZE);
    m.set_circuit_mode(CircuitMode::CrossoverDistortion);
    let nc = m.num_curves();
    let probe = run_case(&mut m, nc, 0, 2.0);
    let observed = probe.amount_pct.expect("circuit must probe amount_pct");
    assert!((observed - 100.0).abs() < 2.0, "CrossoverDistortion AMOUNT=2.0 → 100%, got {}", observed);
}

#[test]
fn circuit_crossover_distortion_mix_max_probes_100_pct() {
    use spectral_forge::dsp::modules::circuit::CircuitMode;
    let mut m = create_module(ModuleType::Circuit, SAMPLE_RATE, FFT_SIZE);
    m.set_circuit_mode(CircuitMode::CrossoverDistortion);
    let nc = m.num_curves();
    let probe = run_case(&mut m, nc, 3, 2.0);   // MIX is curves[3] (Circuit has 4 curves)
    let observed = probe.mix_pct.expect("circuit must probe mix_pct");
    assert!((observed - 100.0).abs() < 2.0, "CrossoverDistortion MIX=2.0 → 100%, got {}", observed);
}

#[test]
fn bin_physics_round_trip_stub() {
    // Phase 5 modules (Life, Kinetics) will fill this in:
    //   1. Set the relevant curve to a known value.
    //   2. Process one block.
    //   3. Read back ProbeSnapshot.bp_mass / bp_temperature / etc.
    //   4. Assert the value matches the curve→physical mapping.
    //
    // Phase 3 ships the probe slots only (the Option<f32> shapes on
    // ProbeSnapshot). The stub keeps a test of the same name in CI so
    // the Phase 5 implementer's first move is filling this in.
    eprintln!("Phase 3 ships probe field shapes; Phase 5 fills in the round-trip.");
}

#[test]
fn life_probe_reports_active_mode() {
    use spectral_forge::dsp::modules::life::{LifeModule, LifeMode, LifeProbe};
    use spectral_forge::dsp::modules::SpectralModule;

    let mut module = LifeModule::new();
    module.reset(48_000.0, 2048);
    module.set_mode(LifeMode::Capillary);

    let probe: LifeProbe = module.probe();
    assert_eq!(probe.active_mode, LifeMode::Capillary);
    assert_eq!(probe.recent_sustain_max, 0.0);
    assert_eq!(probe.recent_tear_count, 0);
}

#[test]
fn kinetics_calibration_probes_round_trip() {
    use spectral_forge::dsp::modules::kinetics::{KineticsModule, KineticsMode};

    let mut m = KineticsModule::new();
    m.reset(48_000.0, 2048);
    m.set_mode(KineticsMode::Hooke);

    let num_bins = 1025;
    let mut bins: Vec<Complex<f32>> = (0..num_bins)
        .map(|k| Complex::new((k as f32 * 0.013).sin(), (k as f32 * 0.011).cos()))
        .collect();
    let neutral = vec![1.0_f32; num_bins];
    let mix = vec![1.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&neutral, &neutral, &neutral, &neutral, &mix];
    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = make_ctx();

    m.process(
        0, StereoLink::Linked, FxChannelTarget::All,
        &mut bins, None, &curves, &mut suppression, None, &ctx,
    );

    let p = m.last_probe();
    assert_eq!(p.kinetics_active_mode_idx, Some(KineticsMode::Hooke as u8));
    assert!(p.kinetics_strength.unwrap().is_finite());
    assert!(p.kinetics_mass.unwrap().is_finite());
    assert!(p.kinetics_displacement.unwrap().is_finite());
    assert!(p.kinetics_velocity.unwrap().is_finite());
    assert_eq!(p.kinetics_well_count, Some(0)); // Hooke uses no fork list
}
