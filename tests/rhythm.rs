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
    assert!(spec.panel_widget.is_some(),
        "rhythm needs a panel widget for arpeggiator step grid");
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
        &mut bins, None, &curves, &mut supp, None, &ctx);
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
        &mut bins, None, &curves, &mut supp, None, &ctx);
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
        &mut bins, None, &curves, &mut supp, None, &ctx);

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

// ── Task 5 tests ─────────────────────────────────────────────────────────────

#[test]
fn arpeggiator_advances_at_step_crossing() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::{SpectralModule, ModuleContext};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let mut m = RhythmModule::new();
    m.set_mode(RhythmMode::Arpeggiator);
    let mut g = ArpGrid::default();
    // Voice 0 plays only at step 0.
    g.toggle(0, 0);
    // Voice 1 plays only at step 4.
    g.toggle(1, 4);
    m.set_arp_grid(g);
    m.reset(48000.0, 1024);

    let amount = vec![2.0f32; 513];
    let div    = vec![1.0f32; 513];
    let af     = vec![0.0f32; 513];
    let tphase = vec![1.0f32; 513];
    let mix    = vec![2.0f32; 513];
    let curves: Vec<&[f32]> = vec![&amount, &div, &af, &tphase, &mix];

    // Build input: peaks at bins 50 and 100.
    let mut input = vec![Complex::new(0.1, 0.0); 513];
    input[50]  = Complex::new(1.0, 0.0);
    input[100] = Complex::new(1.0, 0.0);

    let mut supp = vec![0.0f32; 513];

    // At beat_position=0 (step 0), only voice 0 active → only the highest peak (or first picked) plays.
    let mut bins = input.clone();
    let mut ctx = ModuleContext::new(48000.0, 1024, 513, 10.0, 100.0, 0.5, 1.0, false, false);
    ctx.bpm = 120.0;
    ctx.beat_position = 0.0;
    m.process(0, StereoLink::Linked, FxChannelTarget::All,
        &mut bins, None, &curves, &mut supp, None, &ctx);
    // Voice 0 active at step 0 → its peak bin (50) should pass through.
    // Voice 1 inactive at step 0 → its peak bin (100) should be silenced (with mix=1.0).
    // Non-peak bins should be silenced.
    assert!(bins[50].norm() > 0.5,
        "voice 0 active at step 0: peak bin 50 should pass, got {}", bins[50].norm());
    assert!(bins[100].norm() < 0.05,
        "voice 1 inactive at step 0: peak bin 100 should be silenced, got {}", bins[100].norm());
    assert!(bins[200].norm() < 0.05,
        "non-peak bin 200 should be silenced when mix=1.0, got {}", bins[200].norm());
    for (idx, c) in bins.iter().enumerate() {
        assert!(c.re.is_finite() && c.im.is_finite(),
            "bin {} non-finite: {:?}", idx, c);
    }

    // At step 4 (half a bar in at 8 steps): beat_position = 2.0
    let mut bins = input.clone();
    let mut ctx2 = ModuleContext::new(48000.0, 1024, 513, 10.0, 100.0, 0.5, 1.0, false, false);
    ctx2.bpm = 120.0;
    ctx2.beat_position = 2.0;
    m.process(0, StereoLink::Linked, FxChannelTarget::All,
        &mut bins, None, &curves, &mut supp, None, &ctx2);
    // Voice 1 active at step 4 → its peak bin (100) should pass through.
    // Voice 0 inactive at step 4 → its peak bin (50) should be silenced.
    assert!(bins[100].norm() > 0.5,
        "voice 1 active at step 4: peak bin 100 should pass, got {}", bins[100].norm());
    assert!(bins[50].norm() < 0.05,
        "voice 0 inactive at step 4: peak bin 50 should be silenced, got {}", bins[50].norm());
    for (idx, c) in bins.iter().enumerate() {
        assert!(c.re.is_finite() && c.im.is_finite(),
            "bin {} non-finite: {:?}", idx, c);
    }
}

// ── Task 6 tests ─────────────────────────────────────────────────────────────

#[test]
fn phase_reset_overwrites_phase_at_step_crossing() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::{SpectralModule, ModuleContext};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let mut m = RhythmModule::new();
    m.set_mode(RhythmMode::PhaseReset);
    m.reset(48000.0, 1024);

    let amount = vec![2.0f32; 513];   // full reset strength
    let div    = vec![1.0f32; 513];   // 8 steps (neutral)
    let af     = vec![0.0f32; 513];   // instant (no fade)
    let tphase = vec![1.0f32; 513];   // neutral = 0 phase target
    let mix    = vec![2.0f32; 513];   // full wet
    let curves: Vec<&[f32]> = vec![&amount, &div, &af, &tphase, &mix];

    // Input: Complex(1,1) = magnitude sqrt(2), phase π/4.
    // After full reset to phase 0: re = sqrt(2), im = 0.
    let mut bins = vec![Complex::new(1.0_f32, 1.0_f32); 513];
    let mut supp = vec![0.0f32; 513];

    let mut ctx = ModuleContext::new(48000.0, 1024, 513, 10.0, 100.0, 0.5, 1.0, false, false);
    ctx.bpm = 120.0;
    ctx.beat_position = 0.0; // step boundary: step_pos=0.0 < 0.05 → reset_env=1.0

    m.process(0, StereoLink::Linked, FxChannelTarget::All,
        &mut bins, None, &curves, &mut supp, None, &ctx);

    let bin = bins[100];
    let original_mag = (1.0_f32 * 1.0 + 1.0 * 1.0).sqrt(); // sqrt(2) ≈ 1.4142
    assert!((bin.re - original_mag).abs() < 1e-3,
        "phase-reset should align bin to mag along real axis; got re={}", bin.re);
    assert!(bin.im.abs() < 1e-3,
        "phase-reset should kill imaginary part; got im={}", bin.im);

    // Verify reset applies uniformly across interior bins (not just bin 100).
    for k in 1..512 {
        assert!((bins[k].re - original_mag).abs() < 1e-3,
            "interior bin {} should be reset to (sqrt(2), 0); re={}", k, bins[k].re);
        assert!(bins[k].im.abs() < 1e-3,
            "interior bin {} should have zero imaginary; im={}", k, bins[k].im);
    }
    // DC and Nyquist must pass through dry (input was Complex(1.0, 1.0)).
    assert!((bins[0].re - 1.0).abs() < 1e-6 && (bins[0].im - 1.0).abs() < 1e-6,
        "DC bin should pass through dry; got {:?}", bins[0]);
    assert!((bins[512].re - 1.0).abs() < 1e-6 && (bins[512].im - 1.0).abs() < 1e-6,
        "Nyquist bin should pass through dry; got {:?}", bins[512]);
    // suppression_out must be zeroed (PhaseReset emits no gain reduction).
    assert!(supp.iter().all(|&x| x == 0.0), "suppression_out must be zeroed");
}

#[test]
fn phase_reset_preserves_dc_and_nyquist_real() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::{SpectralModule, ModuleContext};
    use spectral_forge::dsp::modules::rhythm::{RhythmModule, RhythmMode};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let mut m = RhythmModule::new();
    m.set_mode(RhythmMode::PhaseReset);
    m.reset(48000.0, 1024);

    // Non-neutral tphase = 1.5 → target_phase = +π/2. sin(π/2) = 1.0 → would inject im at every bin
    // including DC/Nyquist if the guard is missing. With the guard, those bins must stay real.
    let amount = vec![2.0f32; 513];   // full strength
    let div    = vec![1.0f32; 513];
    let af     = vec![0.0f32; 513];
    let tphase = vec![1.5f32; 513];   // → target_phase = π/2
    let mix    = vec![2.0f32; 513];   // → mix_global = 1.0
    let curves: Vec<&[f32]> = vec![&amount, &div, &af, &tphase, &mix];

    // Real-only DC/Nyquist input (as a real-IFFT input would have).
    let mut bins = vec![Complex::new(1.0, 0.0); 513];
    let mut supp = vec![0.0f32; 513];

    let mut ctx = ModuleContext::new(48000.0, 1024, 513, 10.0, 100.0, 0.5, 1.0, false, false);
    ctx.bpm = 120.0;
    ctx.beat_position = 0.0;

    m.process(0, StereoLink::Linked, FxChannelTarget::All,
        &mut bins, None, &curves, &mut supp, None, &ctx);

    // DC (k=0) and Nyquist (k=512) MUST stay real (im == 0).
    // Without the guard, sin(π/2) = 1.0 would inject im=1.0 at these bins → realfft panic.
    assert!(bins[0].im.abs() < 1e-6,
        "DC bin must stay real to satisfy IFFT; got im={}", bins[0].im);
    assert!(bins[512].im.abs() < 1e-6,
        "Nyquist bin must stay real to satisfy IFFT; got im={}", bins[512].im);

    // Interior bins should rotate by π/2: input (1, 0) → mag=1, target_phase=π/2 → target=(0, 1).
    // With strength=1, reset_env=1, mix=1: bins[k] = target = (0, 1) for k in 1..512.
    for k in 1..512 {
        assert!(bins[k].re.abs() < 1e-3,
            "interior bin {} re should be ~0 after π/2 rotation; got {}", k, bins[k].re);
        assert!((bins[k].im - 1.0).abs() < 1e-3,
            "interior bin {} im should be ~1 after π/2 rotation; got {}", k, bins[k].im);
    }
}
