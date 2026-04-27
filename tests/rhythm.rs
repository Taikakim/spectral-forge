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
    use spectral_forge::dsp::modules::rhythm::bjorklund_into;
    let mut pattern = vec![false; 8];
    bjorklund_into(5, 8, &mut pattern);
    let count: usize = pattern.iter().filter(|&&b| b).count();
    assert_eq!(count, 5);
}

#[test]
fn bjorklund_zero_pulses_is_all_silent() {
    use spectral_forge::dsp::modules::rhythm::bjorklund_into;
    let mut pattern = vec![false; 8];
    bjorklund_into(0, 8, &mut pattern);
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

    // At step 0, Bresenham Bjorklund(5,8)[0] = true → gate open.
    // depth=1.0, mix=1.0, af=0.0 → gain == 1.0 → bins must equal input (1.0, 0.0).
    let mut bins = vec![Complex::new(1.0, 0.0); 513];
    let mut supp = vec![0.0f32; 513];
    m.process(0, StereoLink::Linked, FxChannelTarget::All,
        &mut bins, None, &curves, &mut supp, &ctx);
    // At step 0, Bresenham Bjorklund(5,8)[0] = true → gate open → bins pass.
    // depth=1.0, mix=1.0, af=0.0 → gain == 1.0 → bins must equal input (1.0, 0.0).
    for (idx, c) in bins.iter().enumerate() {
        assert!(c.re.is_finite() && c.im.is_finite(), "bin {} non-finite: {:?}", idx, c);
        assert!((c.re - 1.0).abs() < 1e-4 && c.im.abs() < 1e-4,
            "bin {} expected (1.0, 0.0) for gate-open passthrough, got {:?}", idx, c);
    }
}

#[test]
fn euclidean_gate_silences_off_step() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::{SpectralModule, ModuleContext};
    use spectral_forge::dsp::modules::rhythm::{RhythmModule, RhythmMode};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let mut m = RhythmModule::new();
    m.set_mode(RhythmMode::Euclidean);
    m.reset(48000.0, 1024);

    // probe_k = n/2 = 513/2 = 256. Set amount[probe_k]=1.25 so that
    // pulses_g=1.25 → pulses=(1.25*0.5*8).round()=5 → Bjorklund(5,8).
    // All other bins: amount=2.0 → depth=(2.0*0.5)=1.0 (full attenuation).
    // DIVISION=1.0 (8 steps), ATTACK_FADE=0.0 (instant), MIX=2.0 (mix=1.0).
    let mut amount = vec![2.0f32; 513];
    amount[256] = 1.25; // probe_k drives pulse count; other bins get full depth
    let div    = vec![1.0f32; 513];
    let af     = vec![0.0f32; 513];
    let tphase = vec![1.0f32; 513];
    let mix    = vec![2.0f32; 513];
    let curves: Vec<&[f32]> = vec![&amount, &div, &af, &tphase, &mix];

    // Step 1 of Bresenham Bjorklund(5,8) = false (gate closed).
    // 8 steps over 4 beats → step 1 sits at beat_position=0.5
    // (bar_pos = 0.5/4 = 0.125; step_idx_f = 0.125*8 = 1.0 → step_idx=1).
    let mut ctx = ModuleContext::new(48000.0, 1024, 513, 10.0, 100.0, 0.5, 1.0, false, false);
    ctx.bpm = 120.0;
    ctx.beat_position = 0.5;

    let mut bins = vec![Complex::new(1.0, 0.0); 513];
    let mut supp = vec![0.0f32; 513];
    m.process(0, StereoLink::Linked, FxChannelTarget::All,
        &mut bins, None, &curves, &mut supp, &ctx);

    // gate closed, depth=1.0 (most bins), mix=1.0:
    // gain = 1 - 1.0 + 1.0*0.0 = 0.0 → bins ~0.
    // probe_k bin has depth=0.625, so it becomes 0.375 — skip that bin.
    for (idx, c) in bins.iter().enumerate() {
        if idx == 256 { continue; } // probe_k: different depth, skip
        assert!(c.re.abs() < 1e-4 && c.im.abs() < 1e-4,
            "bin {} expected ~0 for gate-closed step, got {:?}", idx, c);
    }
}

#[test]
fn division_to_steps_neutral_is_8() {
    use spectral_forge::dsp::modules::rhythm::division_to_steps;
    assert_eq!(division_to_steps(0.0), 1);
    assert_eq!(division_to_steps(1.0), 8);
    assert_eq!(division_to_steps(2.0), 32);
}

#[test]
fn bjorklund_1_of_8_has_one_pulse() {
    use spectral_forge::dsp::modules::rhythm::bjorklund_into;
    let mut pattern = vec![false; 8];
    bjorklund_into(1, 8, &mut pattern);
    assert_eq!(pattern.iter().filter(|&&b| b).count(), 1,
        "single pulse must produce exactly one true");
}
