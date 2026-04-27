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
fn circuit_mode_dispatch_via_trait_setter() {
    use spectral_forge::dsp::modules::circuit::{CircuitMode, CircuitModule};
    use spectral_forge::dsp::modules::SpectralModule;

    let mut m = CircuitModule::new();
    assert_eq!(m.current_mode(), CircuitMode::CrossoverDistortion);

    // Trait setter must update the operating mode.
    m.set_circuit_mode(CircuitMode::BbdBins);
    assert_eq!(m.current_mode(), CircuitMode::BbdBins);

    m.set_circuit_mode(CircuitMode::SpectralSchmitt);
    assert_eq!(m.current_mode(), CircuitMode::SpectralSchmitt);
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

#[test]
fn circuit_bbd_delays_and_lowpasses() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::circuit::{CircuitMode, CircuitModule};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let mut module = CircuitModule::new();
    module.reset(48_000.0, 2048);
    module.set_mode(CircuitMode::BbdBins);

    let num_bins = 1025;
    let mut bins: Vec<Complex<f32>> = vec![Complex::new(0.0, 0.0); num_bins];
    bins[100] = Complex::new(4.0, 0.0); // single-bin impulse

    // AMOUNT=2 (max stage-3 gain), THRESHOLD=1 (mild dither), RELEASE=1 (mid LP), MIX=2 (full wet)
    let amount = vec![2.0_f32; num_bins];
    let thresh = vec![1.0_f32; num_bins];
    let release = vec![1.0_f32; num_bins];
    let mix = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &thresh, &release, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = ModuleContext::new(
        48_000.0, 2048, num_bins,
        10.0, 100.0, 1.0,
        0.5, false, false,
    );

    // Hop 1: input enters stage 0; output (stage 3) is still small.
    module.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, None, &curves, &mut suppression, &ctx);
    let after_hop_1 = bins[100].norm();
    assert!(after_hop_1 < 4.0, "BBD must delay (bin 100 still at {} after hop 1)", after_hop_1);

    // Drive zero-input hops so the previously-injected energy propagates through stages.
    for _ in 0..4 {
        for b in bins.iter_mut() { *b = Complex::new(0.0, 0.0); }
        module.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, None, &curves, &mut suppression, &ctx);
    }
    let final_mag = bins[100].norm();
    assert!(final_mag > 0.05, "BBD did not propagate signal through stages (final={})", final_mag);

    for b in &bins {
        assert!(b.norm().is_finite() && b.norm() < 100.0);
    }
}

#[test]
fn circuit_schmitt_hysteresis_latches_above_threshold() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::circuit::{CircuitMode, CircuitModule};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let mut module = CircuitModule::new();
    module.reset(48_000.0, 2048);
    module.set_mode(CircuitMode::SpectralSchmitt);

    let num_bins = 1025;
    let mut bins: Vec<Complex<f32>> = vec![Complex::new(0.0, 0.0); num_bins];
    bins[100] = Complex::new(2.0, 0.0); // above on-threshold (high = 1.0)
    bins[101] = Complex::new(0.05, 0.0); // far below off-threshold

    // AMOUNT=2 (full attenuation when OFF), THRESHOLD=1 (high=1.0),
    // RELEASE=1 (gap=0.5 → low=0.5), MIX=2 (full wet).
    let amount = vec![2.0_f32; num_bins];
    let thresh = vec![1.0_f32; num_bins];
    let release = vec![1.0_f32; num_bins];
    let mix = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &thresh, &release, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = ModuleContext::new(
        48_000.0, 2048, num_bins,
        10.0, 100.0, 1.0,
        0.5, false, false,
    );

    module.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, None, &curves, &mut suppression, &ctx);

    assert!((bins[100].norm() - 2.0).abs() < 0.1, "bin 100 should latch ON (got {})", bins[100].norm());
    assert!(bins[101].norm() < 0.04, "bin 101 should latch OFF (got {})", bins[101].norm());

    // Drop bin 100 to 0.6 — inside hysteresis band [0.5, 1.0]. Should hold ON.
    bins[100] = Complex::new(0.6, 0.0);
    module.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, None, &curves, &mut suppression, &ctx);
    assert!(bins[100].norm() > 0.5, "bin 100 should hold ON in hysteresis band (got {})", bins[100].norm());

    // Drop bin 100 to 0.3 — below low (0.5). Should latch OFF.
    bins[100] = Complex::new(0.3, 0.0);
    module.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, None, &curves, &mut suppression, &ctx);
    assert!(bins[100].norm() < 0.1, "bin 100 should latch OFF after falling below low (got {})", bins[100].norm());
}

#[test]
fn circuit_crossover_smooth_deadzone() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::circuit::{CircuitMode, CircuitModule};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let mut module = CircuitModule::new();
    module.reset(48_000.0, 2048);
    module.set_mode(CircuitMode::CrossoverDistortion);

    let num_bins = 1025;
    let mut bins: Vec<Complex<f32>> = vec![Complex::new(0.0, 0.0); num_bins];
    bins[10]  = Complex::new(0.05, 0.0); // well below dz=0.1
    bins[50]  = Complex::new(0.15, 0.0); // just above dz (50% above)
    bins[100] = Complex::new(2.0, 0.0);  // well above dz

    // AMOUNT=1 → dz_width = 0.1, MIX=2 → full wet. THRESH/RELEASE unused.
    let amount = vec![1.0_f32; num_bins];
    let thresh = vec![1.0_f32; num_bins];
    let release = vec![1.0_f32; num_bins];
    let mix = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &thresh, &release, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = ModuleContext::new(
        48_000.0, 2048, num_bins,
        10.0, 100.0, 1.0,
        0.5, false, false,
    );

    module.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, None, &curves, &mut suppression, &ctx);

    assert!(bins[10].norm() < 0.005, "bin 10 should be deadzoned (got {})", bins[10].norm());
    assert!(bins[50].norm() > 0.0 && bins[50].norm() < 0.1,
        "bin 50 should re-emerge gently (got {})", bins[50].norm());
    assert!(bins[100].norm() > 1.5, "bin 100 should pass mostly through (got {})", bins[100].norm());

    // C¹ check: at mag=0.15, dz=0.1 → expected = (0.05)^2 / 0.15 ≈ 0.0167.
    let expected_50 = 0.05_f32.powi(2) / 0.15;
    assert!((bins[50].norm() - expected_50).abs() < 0.05,
        "bin 50 = {} not within tolerance of {}", bins[50].norm(), expected_50);
}

#[test]
fn circuit_finite_bounded_all_modes_dual_channel() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::circuit::{CircuitMode, CircuitModule};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let num_bins = 1025;

    for mode in [
        CircuitMode::CrossoverDistortion,
        CircuitMode::SpectralSchmitt,
        CircuitMode::BbdBins,
    ] {
        let mut module = CircuitModule::new();
        module.reset(48_000.0, 2048);
        module.set_mode(mode);

        let mut bins_l: Vec<Complex<f32>> = (0..num_bins).map(|k|
            Complex::new(((k as f32 * 0.07).sin() + 0.1).abs(),
                         (k as f32 * 0.11).cos() * 0.5)
        ).collect();
        let mut bins_r: Vec<Complex<f32>> = bins_l.iter().map(|b| b * 0.6).collect();

        let initial_l = bins_l.clone();
        let initial_r = bins_r.clone();

        let amount = vec![1.5_f32; num_bins];
        let mid = vec![1.0_f32; num_bins];
        let mix = vec![1.0_f32; num_bins];
        let curves: Vec<&[f32]> = vec![&amount, &mid, &mid, &mix];

        let mut suppression = vec![0.0_f32; num_bins];
        let ctx = ModuleContext::new(
            48_000.0, 2048, num_bins,
            10.0, 100.0, 1.0,
            0.5, false, false,
        );

        for hop in 0..200 {
            bins_l.copy_from_slice(&initial_l);
            bins_r.copy_from_slice(&initial_r);
            for ch in 0..2 {
                let bins = if ch == 0 { &mut bins_l } else { &mut bins_r };
                module.process(ch, StereoLink::Independent, FxChannelTarget::All,
                               bins, None, &curves, &mut suppression, &ctx);
                for (i, b) in bins.iter().enumerate() {
                    assert!(b.norm().is_finite(),
                        "mode={:?} hop={} ch={} bin={} norm={}",
                        mode, hop, ch, i, b.norm());
                    assert!(b.norm() < 1e6,
                        "runaway: mode={:?} hop={} ch={} bin={} norm={}",
                        mode, hop, ch, i, b.norm());
                }
                for (i, s) in suppression.iter().enumerate() {
                    assert!(s.is_finite() && *s >= 0.0,
                        "suppression: mode={:?} hop={} ch={} bin={} val={}",
                        mode, hop, ch, i, s);
                }
            }
        }
    }
}
