use spectral_forge::dsp::modules::{module_spec, ModuleType};

#[test]
fn circuit_module_spec_present() {
    let spec = module_spec(ModuleType::Circuit);
    assert_eq!(spec.display_name, "Circuit");
    assert_eq!(spec.num_curves, 4);
    assert_eq!(spec.curve_labels.len(), 4);
    assert_eq!(spec.curve_labels, &["AMOUNT", "THRESH", "RELEASE", "MIX"]);
    assert!(!spec.supports_sidechain, "Circuit v1 has no sidechain modes");
    assert!(!spec.wants_sidechain);
}

#[test]
fn circuit_module_constructs_and_passes_through() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::{create_module, ModuleContext, ModuleType};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let mut module = create_module(ModuleType::Circuit, 48_000.0, 2048);
    assert_eq!(module.module_type(), ModuleType::Circuit);
    assert_eq!(module.num_curves(), 4);

    let num_bins = 1025;
    let mut bins: Vec<Complex<f32>> =
        (0..num_bins).map(|k| Complex::new((k as f32 * 0.01).sin(), 0.0)).collect();
    let dry: Vec<Complex<f32>> = bins.clone();

    // AMOUNT=0, MIX=0 → passthrough.
    let zeros = vec![0.0_f32; num_bins];
    let neutral = vec![1.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&zeros, &neutral, &neutral, &zeros];

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
fn circuit_mode_default_is_crossover_distortion() {
    use spectral_forge::dsp::modules::circuit::CircuitMode;
    assert_eq!(CircuitMode::default(), CircuitMode::CrossoverDistortion);
}

#[test]
fn circuit_set_mode_round_trip() {
    use spectral_forge::dsp::modules::circuit::{CircuitMode, CircuitModule};
    let mut m = CircuitModule::new();
    assert_eq!(m.current_mode(), CircuitMode::CrossoverDistortion);
    m.set_mode(CircuitMode::BbdBins);
    assert_eq!(m.current_mode(), CircuitMode::BbdBins);
    m.set_mode(CircuitMode::SpectralSchmitt);
    assert_eq!(m.current_mode(), CircuitMode::SpectralSchmitt);
}
