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
