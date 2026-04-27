use spectral_forge::dsp::modules::{module_spec, ModuleType};

#[test]
fn modulate_module_spec_present() {
    let spec = module_spec(ModuleType::Modulate);
    assert_eq!(spec.display_name, "Modulate");
    assert_eq!(spec.num_curves, 6);
    assert_eq!(spec.curve_labels.len(), 6);
    assert_eq!(spec.curve_labels, &["AMOUNT", "REACH", "RATE", "THRESH", "AMPGATE", "MIX"]);
    assert!(spec.supports_sidechain, "Modulate must support sidechain (RM/Diode RM modes)");
    assert!(spec.wants_sidechain, "RM/Diode RM modes need sidechain auto-routed");
}

#[test]
fn modulate_module_constructs_and_passes_through() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::{create_module, ModuleContext, ModuleType};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let mut module = create_module(ModuleType::Modulate, 48_000.0, 2048);
    assert_eq!(module.module_type(), ModuleType::Modulate);
    assert_eq!(module.num_curves(), 6);

    let num_bins = 1025;
    let mut bins: Vec<Complex<f32>> =
        (0..num_bins).map(|k| Complex::new((k as f32 * 0.01).sin(), 0.0)).collect();
    let dry: Vec<Complex<f32>> = bins.clone();

    // All curves neutral — kernel stub passthrough.
    let zeros = vec![0.0_f32; num_bins];
    let neutral = vec![1.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&zeros, &neutral, &neutral, &neutral, &zeros, &zeros];

    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = ModuleContext::new(
        48_000.0, 2048, num_bins,
        10.0, 100.0, 1.0,
        0.5, false, false,
    );

    module.process(0, StereoLink::Linked, FxChannelTarget::All,
                   &mut bins, None, &curves, &mut suppression, &ctx);

    for k in 0..num_bins {
        let diff = (bins[k] - dry[k]).norm();
        assert!(diff < 1e-5, "bin {} drifted by {}", k, diff);
    }
    for s in &suppression {
        assert!(s.is_finite() && *s >= 0.0);
    }
}

#[test]
fn modulate_mode_default_is_phase_phaser() {
    use spectral_forge::dsp::modules::modulate::ModulateMode;
    assert_eq!(ModulateMode::default(), ModulateMode::PhasePhaser);
}

#[test]
fn modulate_set_mode_round_trip() {
    use spectral_forge::dsp::modules::modulate::{ModulateMode, ModulateModule};
    use spectral_forge::dsp::modules::SpectralModule;
    let mut m = ModulateModule::new();
    m.reset(48_000.0, 2048);
    assert_eq!(m.current_mode(), ModulateMode::PhasePhaser);
    m.set_mode(ModulateMode::DiodeRm);
    assert_eq!(m.current_mode(), ModulateMode::DiodeRm);
}

#[test]
fn modulate_phase_phaser_rotates_phase_and_preserves_magnitude() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::modulate::{ModulateModule, ModulateMode};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let mut module = ModulateModule::new();
    module.reset(48_000.0, 2048);
    module.set_mode(ModulateMode::PhasePhaser);

    let num_bins = 1025;
    // Pure cosines (phase = 0) at unit magnitude.
    let mut bins: Vec<Complex<f32>> = (0..num_bins).map(|_| Complex::new(1.0, 0.0)).collect();

    // AMOUNT=2 (max rotation), RATE=1, THRESH=1, AMPGATE=0 (gate disabled), MIX=2 (full wet).
    let amount  = vec![2.0_f32; num_bins];
    let reach   = vec![1.0_f32; num_bins];
    let rate    = vec![1.0_f32; num_bins];
    let thresh  = vec![1.0_f32; num_bins];
    let ampgate = vec![0.0_f32; num_bins];
    let mix     = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &reach, &rate, &thresh, &ampgate, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = ModuleContext::new(48_000.0, 2048, num_bins, 10.0, 100.0, 1.0, 0.5, false, false);

    // Run a couple of hops so the animator advances away from hop_count=0
    // (where every rotation goes through 0).
    for _ in 0..3 {
        // Re-seed bins each hop so we test rotation, not accumulated drift.
        for b in bins.iter_mut() { *b = Complex::new(1.0, 0.0); }
        module.process(0, StereoLink::Linked, FxChannelTarget::All,
                       &mut bins, None, &curves, &mut suppression, &ctx);
    }

    // Magnitudes preserved (rotation is unit-modulus).
    for k in 0..num_bins {
        let mag = bins[k].norm();
        assert!((mag - 1.0).abs() < 1e-3, "bin {} magnitude drifted to {}", k, mag);
    }
    // At least some phases must have rotated away from 0.
    let max_im: f32 = bins.iter().map(|b| b.im.abs()).fold(0.0_f32, f32::max);
    assert!(max_im > 0.1, "Phase Phaser did not rotate phase (max im = {})", max_im);
}

#[test]
fn modulate_phase_phaser_amount_zero_passthrough() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::modulate::{ModulateModule, ModulateMode};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let mut module = ModulateModule::new();
    module.reset(48_000.0, 2048);
    module.set_mode(ModulateMode::PhasePhaser);

    let num_bins = 1025;
    let mut bins: Vec<Complex<f32>> =
        (0..num_bins).map(|k| Complex::new((k as f32 * 0.03).cos(), (k as f32 * 0.03).sin())).collect();
    let dry = bins.clone();

    // AMOUNT=0 → zero rotation regardless of MIX.
    let zeros = vec![0.0_f32; num_bins];
    let neutral = vec![1.0_f32; num_bins];
    let mix = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&zeros, &neutral, &neutral, &neutral, &zeros, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = ModuleContext::new(48_000.0, 2048, num_bins, 10.0, 100.0, 1.0, 0.5, false, false);

    module.process(0, StereoLink::Linked, FxChannelTarget::All,
                   &mut bins, None, &curves, &mut suppression, &ctx);

    for k in 0..num_bins {
        let diff = (bins[k] - dry[k]).norm();
        assert!(diff < 1e-5, "bin {} drifted by {} with AMOUNT=0", k, diff);
    }
}

#[test]
fn modulate_bin_swapper_blends_to_offset_neighbour() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::modulate::{ModulateModule, ModulateMode};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let mut module = ModulateModule::new();
    module.reset(48_000.0, 2048);
    module.set_mode(ModulateMode::BinSwapper);

    let num_bins = 1025;
    // Spike at bin 100, silence elsewhere.
    let mut bins: Vec<Complex<f32>> = vec![Complex::new(0.0, 0.0); num_bins];
    bins[100] = Complex::new(2.0, 0.0);

    // AMOUNT=2 (full swap), REACH=1 (offset = 5 bins), THRESH=0 (no floor), MIX=2 (full wet).
    let amount = vec![2.0_f32; num_bins];
    let reach  = vec![1.0_f32; num_bins];
    let zeros  = vec![0.0_f32; num_bins];
    let thresh = vec![0.0_f32; num_bins];
    let mix    = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &reach, &zeros, &thresh, &zeros, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = ModuleContext::new(48_000.0, 2048, num_bins, 10.0, 100.0, 1.0, 0.5, false, false);

    module.process(0, StereoLink::Linked, FxChannelTarget::All,
                   &mut bins, None, &curves, &mut suppression, &ctx);

    // Bin 100: AMOUNT=1 means it pulls fully from bin 105 which was 0 → bin 100 should be 0.
    assert!(bins[100].norm() < 1.0, "bin 100 still hot ({}) — swap did not pull silence in", bins[100].norm());
    // Bin 95 = bin 95 + offset 5 → reads from bin 100 (which had magnitude 2 in the snapshot) → grows.
    assert!(bins[95].norm() > 0.5, "bin 95 silent ({}) — swap did not land", bins[95].norm());
}

#[test]
fn modulate_bin_swapper_amount_zero_passthrough() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::modulate::{ModulateModule, ModulateMode};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let mut module = ModulateModule::new();
    module.reset(48_000.0, 2048);
    module.set_mode(ModulateMode::BinSwapper);

    let num_bins = 1025;
    let mut bins: Vec<Complex<f32>> =
        (0..num_bins).map(|k| Complex::new((k as f32 * 0.03).cos(), 0.1)).collect();
    let dry = bins.clone();

    // MIX=0 → wet inactive regardless of AMOUNT.
    let amount = vec![2.0_f32; num_bins];
    let reach  = vec![1.0_f32; num_bins];
    let zeros  = vec![0.0_f32; num_bins];
    let thresh = vec![0.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &reach, &zeros, &thresh, &zeros, &zeros];

    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = ModuleContext::new(48_000.0, 2048, num_bins, 10.0, 100.0, 1.0, 0.5, false, false);

    module.process(0, StereoLink::Linked, FxChannelTarget::All,
                   &mut bins, None, &curves, &mut suppression, &ctx);

    for k in 0..num_bins {
        let diff = (bins[k] - dry[k]).norm();
        assert!(diff < 1e-5, "bin {} drifted by {} with MIX=0", k, diff);
    }
}

#[test]
fn modulate_rm_fm_matrix_amplifies_at_sidechain_spike() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::modulate::{ModulateModule, ModulateMode};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let mut module = ModulateModule::new();
    module.reset(48_000.0, 2048);
    module.set_mode(ModulateMode::RmFmMatrix);

    let num_bins = 1025;
    let mut bins: Vec<Complex<f32>> = vec![Complex::new(1.0, 0.0); num_bins];
    let dry: Vec<Complex<f32>> = bins.clone();

    // Sidechain: spike at bin 200, magnitude 4.
    let mut sc = vec![0.0_f32; num_bins];
    sc[200] = 4.0;

    // AMOUNT=0 (pure RM), REACH=2, RATE=1, THRESH=0, MIX=2 (full wet).
    let amount = vec![0.0_f32; num_bins];
    let reach  = vec![2.0_f32; num_bins];
    let rate   = vec![1.0_f32; num_bins];
    let thresh = vec![0.0_f32; num_bins];
    let zeros  = vec![0.0_f32; num_bins];
    let mix    = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &reach, &rate, &thresh, &zeros, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = ModuleContext::new(48_000.0, 2048, num_bins, 10.0, 100.0, 1.0, 0.5, false, false);

    module.process(0, StereoLink::Linked, FxChannelTarget::All,
                   &mut bins, Some(&sc), &curves, &mut suppression, &ctx);

    // Bin 200: RM = dry × sc × reach = 1 × 4 × 2 = 8. With THRESH=0 and MIX=1.
    assert!(bins[200].norm() > 6.0, "bin 200 = {} (expected ≈ 8 from RM)", bins[200].norm());

    // Bin 50: sc[50] = 0 → THRESH guard skips. With MIX > 0, bin should remain near dry.
    let dist = (bins[50] - dry[50]).norm();
    assert!(dist < 0.1, "bin 50 drifted by {} (sidechain was 0)", dist);
}

#[test]
fn modulate_rm_fm_pure_fm_preserves_magnitude() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::modulate::{ModulateModule, ModulateMode};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let mut module = ModulateModule::new();
    module.reset(48_000.0, 2048);
    module.set_mode(ModulateMode::RmFmMatrix);

    let num_bins = 1025;
    let mut bins: Vec<Complex<f32>> = vec![Complex::new(0.5, 0.0); num_bins];

    // Sidechain at all bins, magnitude 0.5.
    let sc = vec![0.5_f32; num_bins];

    // AMOUNT=2 (pure FM), REACH=1, THRESH=0, MIX=2.
    let amount = vec![2.0_f32; num_bins];
    let reach  = vec![1.0_f32; num_bins];
    let rate   = vec![1.0_f32; num_bins];
    let thresh = vec![0.0_f32; num_bins];
    let zeros  = vec![0.0_f32; num_bins];
    let mix    = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &reach, &rate, &thresh, &zeros, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = ModuleContext::new(48_000.0, 2048, num_bins, 10.0, 100.0, 1.0, 0.5, false, false);

    module.process(0, StereoLink::Linked, FxChannelTarget::All,
                   &mut bins, Some(&sc), &curves, &mut suppression, &ctx);

    // Pure FM rotates phase but magnitude must remain ≈ 0.5.
    for k in 0..num_bins {
        let mag = bins[k].norm();
        assert!((mag - 0.5).abs() < 0.05, "bin {} magnitude {} drifted from 0.5 in pure FM", k, mag);
    }
}

#[test]
fn modulate_rm_fm_no_sidechain_passes_through() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::modulate::{ModulateModule, ModulateMode};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let mut module = ModulateModule::new();
    module.reset(48_000.0, 2048);
    module.set_mode(ModulateMode::RmFmMatrix);

    let num_bins = 1025;
    let mut bins: Vec<Complex<f32>> =
        (0..num_bins).map(|k| Complex::new((k as f32 * 0.01).sin(), 0.2)).collect();
    let dry = bins.clone();

    let amount = vec![1.0_f32; num_bins];
    let neutral = vec![1.0_f32; num_bins];
    let mix = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &neutral, &neutral, &neutral, &neutral, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = ModuleContext::new(48_000.0, 2048, num_bins, 10.0, 100.0, 1.0, 0.5, false, false);

    module.process(0, StereoLink::Linked, FxChannelTarget::All,
                   &mut bins, None, &curves, &mut suppression, &ctx);

    for k in 0..num_bins {
        let diff = (bins[k] - dry[k]).norm();
        assert!(diff < 1e-6, "bin {} drifted by {} with no sidechain", k, diff);
    }
}

#[test]
fn modulate_diode_rm_leaks_carrier_when_input_quiet() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::modulate::{ModulateModule, ModulateMode};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let num_bins = 1025;

    let mut module_quiet = ModulateModule::new();
    module_quiet.reset(48_000.0, 2048);
    module_quiet.set_mode(ModulateMode::DiodeRm);

    let mut module_loud = ModulateModule::new();
    module_loud.reset(48_000.0, 2048);
    module_loud.set_mode(ModulateMode::DiodeRm);

    // Same sidechain (carrier) for both: spike at bin 300, magnitude 2.
    let mut sc = vec![0.0_f32; num_bins];
    sc[300] = 2.0;

    // Quiet input: bin 300 magnitude = 0.05 (well below threshold = 0.5).
    let mut bins_quiet: Vec<Complex<f32>> = vec![Complex::new(0.0, 0.0); num_bins];
    bins_quiet[300] = Complex::new(0.05, 0.0);

    // Loud input: bin 300 magnitude = 1.5 (well above threshold).
    let mut bins_loud: Vec<Complex<f32>> = vec![Complex::new(0.0, 0.0); num_bins];
    bins_loud[300] = Complex::new(1.5, 0.0);

    // AMOUNT=2 (max RM), REACH=1, RATE=neutral, THRESH=1 (= 0.5 absolute), AMPGATE=0, MIX=2.
    let amount  = vec![2.0_f32; num_bins];
    let reach   = vec![1.0_f32; num_bins];
    let rate    = vec![1.0_f32; num_bins];
    let thresh  = vec![1.0_f32; num_bins];
    let ampgate = vec![0.0_f32; num_bins];
    let mix     = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &reach, &rate, &thresh, &ampgate, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = ModuleContext::new(48_000.0, 2048, num_bins, 10.0, 100.0, 1.0, 0.5, false, false);

    module_quiet.process(0, StereoLink::Linked, FxChannelTarget::All,
                         &mut bins_quiet, Some(&sc), &curves, &mut suppression, &ctx);
    module_loud.process(0, StereoLink::Linked, FxChannelTarget::All,
                        &mut bins_loud, Some(&sc), &curves, &mut suppression, &ctx);

    let quiet_out = bins_quiet[300].norm();
    let loud_out  = bins_loud[300].norm();

    // Quiet input → diode "open" → carrier leaks through. Output dominated by sc·mismatch.
    // mismatch ≈ 1 - 0.05/0.5 = 0.9. Leak ≈ 2·0.9 = 1.8. RM = 0.05·2·1·1 = 0.1.
    // Total ≈ 1.9. Must be > 1.0.
    assert!(quiet_out > 1.0, "quiet path bin 300 = {} (expected leak-dominant > 1.0)", quiet_out);

    // Loud input → diode "closed" → carrier leak = 0. Output ≈ pure RM = 1.5·2·1·1 = 3.0.
    // Must be > 2.0 (leak-only would give ~0).
    assert!(loud_out > 2.0, "loud path bin 300 = {} (expected RM-dominant > 2.0)", loud_out);
}

#[test]
fn modulate_diode_rm_no_sidechain_passes_through() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::modulate::{ModulateModule, ModulateMode};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let mut module = ModulateModule::new();
    module.reset(48_000.0, 2048);
    module.set_mode(ModulateMode::DiodeRm);

    let num_bins = 1025;
    let mut bins: Vec<Complex<f32>> =
        (0..num_bins).map(|k| Complex::new((k as f32 * 0.02).cos(), 0.1)).collect();
    let dry = bins.clone();

    let amount  = vec![2.0_f32; num_bins];
    let neutral = vec![1.0_f32; num_bins];
    let mix     = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &neutral, &neutral, &neutral, &neutral, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = ModuleContext::new(48_000.0, 2048, num_bins, 10.0, 100.0, 1.0, 0.5, false, false);

    module.process(0, StereoLink::Linked, FxChannelTarget::All,
                   &mut bins, None, &curves, &mut suppression, &ctx);

    for k in 0..num_bins {
        let diff = (bins[k] - dry[k]).norm();
        assert!(diff < 1e-6, "bin {} drifted by {} with no sidechain", k, diff);
    }
}
