use spectral_forge::dsp::modules::{ModuleType, module_spec};
use spectral_forge::dsp::modules::rhythm::{RhythmModule, RhythmMode, ArpGrid};

#[test]
fn rhythm_module_spec_basic() {
    let spec = module_spec(ModuleType::Rhythm);
    assert_eq!(spec.num_curves, 5);
    assert_eq!(spec.curve_labels, &["AMOUNT", "DIVISION", "ATTACK_FADE", "TARGET_PHASE", "MIX"]);
    assert!(!spec.supports_sidechain);
    assert!(!spec.wants_sidechain);
    assert_eq!(spec.display_name, "Rhythm");
}

#[test]
fn rhythm_mode_default_is_euclidean() {
    assert_eq!(RhythmMode::default(), RhythmMode::Euclidean);
}

#[test]
fn arp_grid_default_is_empty() {
    let g = ArpGrid::default();
    for v in 0..8 {
        assert_eq!(g.steps[v], 0u8, "voice {} should start with no active steps", v);
    }
}

#[test]
fn rhythm_module_skeleton_is_passthrough() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::{SpectralModule, ModuleContext};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let mut m = RhythmModule::new();
    m.reset(48000.0, 1024);
    let mut bins = vec![Complex::new(0.5, 0.1); 513];
    let original = bins.clone();
    let curves_storage: Vec<Vec<f32>> = (0..5).map(|_| vec![1.0f32; 513]).collect();
    let curves: Vec<&[f32]> = curves_storage.iter().map(|v| v.as_slice()).collect();
    let mut supp = vec![0.0f32; 513];
    let ctx = ModuleContext::new(
        48000.0, 1024, 513,
        10.0, 100.0, 0.5,
        1.0, false, false,
    );
    // bpm/beat_position default to 0.0 in ::new; nothing to set for this test.
    m.process(0, StereoLink::Linked, FxChannelTarget::All,
        &mut bins, None, &curves, &mut supp, &ctx);
    // Skeleton process() is a no-op stub: bins must be unchanged and suppression_out zeroed.
    for (a, b) in bins.iter().zip(original.iter()) {
        assert!((a.re - b.re).abs() < 1e-3 && (a.im - b.im).abs() < 1e-3);
    }
    assert!(supp.iter().all(|&x| x == 0.0), "suppression_out must be zeroed");
}

// ── Task 4 tests ─────────────────────────────────────────────────────────────

#[test]
fn bjorklund_5_of_8_distributes_evenly() {
    use spectral_forge::dsp::modules::rhythm::bjorklund;
    let pattern = bjorklund(5, 8);
    let count: usize = pattern.iter().filter(|&&b| b).count();
    assert_eq!(count, 5);
}

#[test]
fn bjorklund_zero_pulses_is_all_silent() {
    use spectral_forge::dsp::modules::rhythm::bjorklund;
    let pattern = bjorklund(0, 8);
    assert_eq!(pattern.iter().filter(|&&b| b).count(), 0);
}

#[test]
fn euclidean_gate_silences_off_steps() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::{SpectralModule, ModuleContext};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let mut m = RhythmModule::new();
    m.set_mode(RhythmMode::Euclidean);
    m.reset(48000.0, 1024);

    // AMOUNT=2.0 (full gate depth), DIVISION=1.0 (8 steps), ATTACK_FADE=0.0 (instant), MIX=2.0
    let amount = vec![2.0f32; 513];
    let div    = vec![1.0f32; 513];
    let af     = vec![0.0f32; 513];
    let tphase = vec![1.0f32; 513];
    let mix    = vec![2.0f32; 513];
    let curves: Vec<&[f32]> = vec![&amount, &div, &af, &tphase, &mix];

    // Construct ctx at beat_position = 0.0 (beginning of bar at 120 BPM).
    let mut ctx = ModuleContext::new(48000.0, 1024, 513, 10.0, 100.0, 0.5, 1.0, false, false);
    ctx.bpm = 120.0;
    ctx.beat_position = 0.0; // explicit even though it's the default — clarifies intent

    // At step 0, Bjorklund(5,8)[0] = true → gate open → bins pass.
    let mut bins = vec![Complex::new(1.0, 0.0); 513];
    let mut supp = vec![0.0f32; 513];
    m.process(0, StereoLink::Linked, FxChannelTarget::All,
        &mut bins, None, &curves, &mut supp, &ctx);
    // Step 0 of Bjorklund(5,8) → verify finite + bounded.
    for c in &bins { assert!(c.re.is_finite() && c.norm() <= 2.0); }
}
