use spectral_forge::dsp::modules::{module_spec, ModuleContext, ModuleType};

fn circuit_test_ctx(num_bins: usize) -> ModuleContext<'static> {
    ModuleContext::new(
        48_000.0, 2048, num_bins,
        10.0, 100.0, 1.0,
        0.5, false, false,
    )
}



#[test]
fn circuit_module_spec_present() {
    let spec = module_spec(ModuleType::Circuit);
    assert_eq!(spec.display_name, "Circuit");
    assert_eq!(spec.num_curves, 5);
    assert_eq!(spec.curve_labels.len(), 5);
    assert_eq!(spec.curve_labels, &["AMOUNT", "THRESH", "SPREAD", "RELEASE", "MIX"]);
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
    assert_eq!(module.num_curves(), 5);

    let num_bins = 1025;
    let mut bins: Vec<Complex<f32>> =
        (0..num_bins).map(|k| Complex::new((k as f32 * 0.01).sin(), 0.0)).collect();
    let dry: Vec<Complex<f32>> = bins.clone();

    // AMOUNT=0, THRESH=1, SPREAD=1, RELEASE=1, MIX=0 → passthrough.
    let zeros = vec![0.0_f32; num_bins];
    let neutral = vec![1.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&zeros, &neutral, &neutral, &neutral, &zeros];

    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = ModuleContext::new(
        48_000.0, 2048, num_bins,
        10.0, 100.0, 1.0,
        0.5, false, false,
    );

    module.process(0, StereoLink::Linked, FxChannelTarget::All,
                   &mut bins, None, &curves, &mut suppression, None, &ctx);

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

    // AMOUNT=2 (max stage-3 gain), THRESHOLD=1 (mild dither), SPREAD=1 (neutral),
    // RELEASE=1 (mid LP), MIX=2 (full wet)
    let amount = vec![2.0_f32; num_bins];
    let thresh = vec![1.0_f32; num_bins];
    let spread = vec![1.0_f32; num_bins];
    let release = vec![1.0_f32; num_bins];
    let mix = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &thresh, &spread, &release, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = ModuleContext::new(
        48_000.0, 2048, num_bins,
        10.0, 100.0, 1.0,
        0.5, false, false,
    );

    // Hop 1: input enters stage 0; output (stage 3) is still small.
    module.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, None, &curves, &mut suppression, None, &ctx);
    let after_hop_1 = bins[100].norm();
    assert!(after_hop_1 < 4.0, "BBD must delay (bin 100 still at {} after hop 1)", after_hop_1);

    // Drive zero-input hops so the previously-injected energy propagates through stages.
    for _ in 0..4 {
        for b in bins.iter_mut() { *b = Complex::new(0.0, 0.0); }
        module.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, None, &curves, &mut suppression, None, &ctx);
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

    // AMOUNT=2 (full attenuation when OFF), THRESHOLD=1 (high=1.0), SPREAD=1 (neutral),
    // RELEASE=1 (gap=0.5 → low=0.5), MIX=2 (full wet).
    let amount = vec![2.0_f32; num_bins];
    let thresh = vec![1.0_f32; num_bins];
    let spread = vec![1.0_f32; num_bins];
    let release = vec![1.0_f32; num_bins];
    let mix = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &thresh, &spread, &release, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = ModuleContext::new(
        48_000.0, 2048, num_bins,
        10.0, 100.0, 1.0,
        0.5, false, false,
    );

    module.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, None, &curves, &mut suppression, None, &ctx);

    assert!((bins[100].norm() - 2.0).abs() < 0.1, "bin 100 should latch ON (got {})", bins[100].norm());
    assert!(bins[101].norm() < 0.04, "bin 101 should latch OFF (got {})", bins[101].norm());

    // Drop bin 100 to 0.6 — inside hysteresis band [0.5, 1.0]. Should hold ON.
    bins[100] = Complex::new(0.6, 0.0);
    module.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, None, &curves, &mut suppression, None, &ctx);
    assert!(bins[100].norm() > 0.5, "bin 100 should hold ON in hysteresis band (got {})", bins[100].norm());

    // Drop bin 100 to 0.3 — below low (0.5). Should latch OFF.
    bins[100] = Complex::new(0.3, 0.0);
    module.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, None, &curves, &mut suppression, None, &ctx);
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

    // AMOUNT=1 → dz_width = 0.1, MIX=2 → full wet. THRESH/SPREAD/RELEASE unused.
    let amount = vec![1.0_f32; num_bins];
    let thresh = vec![1.0_f32; num_bins];
    let spread = vec![1.0_f32; num_bins];
    let release = vec![1.0_f32; num_bins];
    let mix = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &thresh, &spread, &release, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = ModuleContext::new(
        48_000.0, 2048, num_bins,
        10.0, 100.0, 1.0,
        0.5, false, false,
    );

    module.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, None, &curves, &mut suppression, None, &ctx);

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
        // [AMOUNT, THRESH, SPREAD, RELEASE, MIX]
        let curves: Vec<&[f32]> = vec![&amount, &mid, &mid, &mid, &mix];

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
                               bins, None, &curves, &mut suppression, None, &ctx);
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

#[test]
fn circuit_vactrol_smooths_flux_input_with_release_envelope() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::circuit::{CircuitMode, CircuitModule};
    use spectral_forge::dsp::modules::SpectralModule;
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let mut module = CircuitModule::new();
    module.reset(48_000.0, 2048);
    module.set_circuit_mode(CircuitMode::Vactrol);

    let num_bins = 1025;
    let mut bins: Vec<Complex<f32>> = vec![Complex::new(1.0, 0.0); num_bins];

    let amount  = vec![1.0_f32; num_bins];
    let thresh  = vec![1.0_f32; num_bins];
    let spread  = vec![0.0_f32; num_bins];
    let release = vec![1.0_f32; num_bins];
    let mix     = vec![2.0_f32; num_bins]; // full wet
    let curves: Vec<&[f32]> = vec![&amount, &thresh, &spread, &release, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = circuit_test_ctx(num_bins);

    // Hop 1: vactrol caps empty → strong attenuation.
    module.process(0, StereoLink::Linked, FxChannelTarget::All,
                   &mut bins, None, &curves, &mut suppression, None, &ctx);
    let first_hop_mag = bins[100].norm();
    assert!(first_hop_mag < 0.5, "first hop should be attenuated by empty cap (got {})", first_hop_mag);

    // 200 hops: caps charge → output approaches input.
    for _ in 0..200 {
        for b in bins.iter_mut() { *b = Complex::new(1.0, 0.0); }
        module.process(0, StereoLink::Linked, FxChannelTarget::All,
                       &mut bins, None, &curves, &mut suppression, None, &ctx);
    }
    let charged_mag = bins[100].norm();
    assert!(charged_mag > 0.7, "after charge, output should approach input (got {})", charged_mag);

    // Drop input to zero: slow cap should still hold charge.
    for b in bins.iter_mut() { *b = Complex::new(0.0, 0.0); }
    module.process(0, StereoLink::Linked, FxChannelTarget::All,
                   &mut bins, None, &curves, &mut suppression, None, &ctx);
    #[cfg(any(test, feature = "probe"))]
    {
        let probe = module.probe_state(0);
        assert!(probe.vactrol_slow_avg > 0.1, "slow cap should still hold charge (got {})", probe.vactrol_slow_avg);
    }
}

#[test]
fn circuit_transformer_saturates_high_magnitudes_softly() {
    use spectral_forge::dsp::modules::circuit::{CircuitModule, CircuitMode};
    use spectral_forge::dsp::modules::SpectralModule;
    use spectral_forge::params::{FxChannelTarget, StereoLink};
    use num_complex::Complex;

    let mut module = CircuitModule::new();
    module.reset(48_000.0, 2048);
    module.set_circuit_mode(CircuitMode::TransformerSaturation);

    let num_bins = 1025;
    let mut bins: Vec<Complex<f32>> = vec![Complex::new(0.0, 0.0); num_bins];
    bins[100] = Complex::new(0.5, 0.0);  // sub-knee
    bins[200] = Complex::new(3.0, 0.0);  // above knee — should saturate

    // AMOUNT=2 (max drive), THRESHOLD=1 (knee at unity), SPREAD=0 (test isolation), RELEASE=1, MIX=2 wet.
    let amount  = vec![2.0_f32; num_bins];
    let thresh  = vec![1.0_f32; num_bins];
    let spread  = vec![0.0_f32; num_bins];
    let release = vec![1.0_f32; num_bins];
    let mix     = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &thresh, &spread, &release, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = circuit_test_ctx(num_bins);

    // Several hops to let the magnitude smoother settle.
    for _ in 0..40 {
        bins[100] = Complex::new(0.5, 0.0);
        bins[200] = Complex::new(3.0, 0.0);
        module.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, None, &curves, &mut suppression, None, &ctx);
    }

    // Sub-knee bin: ~unchanged.
    assert!(bins[100].norm() < 0.7 && bins[100].norm() > 0.3, "bin 100 sub-knee got {}", bins[100].norm());
    // Above-knee bin: bounded well below input.
    assert!(bins[200].norm() < 2.0, "bin 200 should saturate (got {})", bins[200].norm());
    assert!(bins[200].norm() > 0.5, "bin 200 should not collapse to 0 (got {})", bins[200].norm());
}

#[test]
fn circuit_transformer_spread_leaks_to_neighbours() {
    use spectral_forge::dsp::modules::circuit::{CircuitModule, CircuitMode};
    use spectral_forge::dsp::modules::SpectralModule;
    use spectral_forge::params::{FxChannelTarget, StereoLink};
    use num_complex::Complex;

    let mut module = CircuitModule::new();
    module.reset(48_000.0, 2048);
    module.set_circuit_mode(CircuitMode::TransformerSaturation);

    let num_bins = 1025;
    let mut bins: Vec<Complex<f32>> = vec![Complex::new(0.0, 0.0); num_bins];
    bins[200] = Complex::new(3.0, 0.0);

    // SPREAD = 2 (full leak), drive on, neighbours start at zero.
    let amount  = vec![2.0_f32; num_bins];
    let thresh  = vec![1.0_f32; num_bins];
    let spread  = vec![2.0_f32; num_bins]; // 1.0 leak strength after clamp/scale
    let release = vec![1.0_f32; num_bins];
    let mix     = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &thresh, &spread, &release, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = circuit_test_ctx(num_bins);

    // Settle several hops to let the magnitude smoother + spread reach steady state.
    for _ in 0..20 {
        bins[200] = Complex::new(3.0, 0.0);
        bins[199] = Complex::new(0.0, 0.0);
        bins[201] = Complex::new(0.0, 0.0);
        module.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, None, &curves, &mut suppression, None, &ctx);
    }

    // Neighbours should have *non-zero* magnitude after the leak.
    assert!(bins[199].norm() > 0.05, "bin 199 should receive leak (got {})", bins[199].norm());
    assert!(bins[201].norm() > 0.05, "bin 201 should receive leak (got {})", bins[201].norm());
}

#[test]
fn circuit_power_sag_attenuates_under_high_energy() {
    use spectral_forge::dsp::modules::circuit::{CircuitModule, CircuitMode};
    use spectral_forge::dsp::modules::SpectralModule;
    use spectral_forge::params::{StereoLink, FxChannelTarget};
    use num_complex::Complex;

    let mut module = CircuitModule::new();
    module.reset(48_000.0, 2048);
    module.set_circuit_mode(CircuitMode::PowerSag);

    let num_bins = 1025;

    // AMOUNT=2 (deep sag), THRESHOLD=0.1 (low energy threshold), SPREAD=0, RELEASE=1, MIX=2.
    let amount  = vec![2.0_f32; num_bins];
    let thresh  = vec![0.1_f32; num_bins];
    let spread  = vec![0.0_f32; num_bins];
    let release = vec![1.0_f32; num_bins];
    let mix     = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &thresh, &spread, &release, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = circuit_test_ctx(num_bins);

    // High-energy input: every bin at magnitude 2.0.
    let mut bins: Vec<Complex<f32>> = vec![Complex::new(2.0, 0.0); num_bins];
    let initial_total: f32 = bins.iter().map(|b| b.norm()).sum();

    // Settle 100 hops.
    for _ in 0..100 {
        for b in bins.iter_mut() { *b = Complex::new(2.0, 0.0); }
        module.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, None, &curves, &mut suppression, None, &ctx);
    }

    let final_total: f32 = bins.iter().map(|b| b.norm()).sum();
    assert!(final_total < initial_total * 0.95, "sag should attenuate (initial={}, final={})", initial_total, final_total);
    assert!(final_total > initial_total * 0.05, "sag should not zero out (final={})", final_total);
}

#[test]
fn circuit_power_sag_recovers_when_energy_drops() {
    use spectral_forge::dsp::modules::circuit::{CircuitModule, CircuitMode};
    use spectral_forge::dsp::modules::SpectralModule;
    use spectral_forge::params::{StereoLink, FxChannelTarget};
    use num_complex::Complex;

    let mut module = CircuitModule::new();
    module.reset(48_000.0, 2048);
    module.set_circuit_mode(CircuitMode::PowerSag);

    let num_bins = 1025;
    let amount  = vec![2.0_f32; num_bins];
    let thresh  = vec![0.1_f32; num_bins];
    let spread  = vec![0.0_f32; num_bins];
    let release = vec![1.0_f32; num_bins];
    let mix     = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &thresh, &spread, &release, &mix];
    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = circuit_test_ctx(num_bins);

    let mut bins: Vec<Complex<f32>> = vec![Complex::new(2.0, 0.0); num_bins];
    // High-energy ramp-up.
    for _ in 0..50 {
        for b in bins.iter_mut() { *b = Complex::new(2.0, 0.0); }
        module.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, None, &curves, &mut suppression, None, &ctx);
    }
    let probe_high = module.probe_state(0);
    // Now drop energy to silence.
    for _ in 0..200 {
        for b in bins.iter_mut() { *b = Complex::new(0.0, 0.0); }
        module.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, None, &curves, &mut suppression, None, &ctx);
    }
    let probe_low = module.probe_state(0);
    assert!(probe_low.sag_envelope < probe_high.sag_envelope * 0.5,
        "sag envelope should recover (high={}, low={})", probe_high.sag_envelope, probe_low.sag_envelope);
}

#[test]
fn circuit_component_drift_modulates_magnitudes_slowly() {
    use spectral_forge::dsp::modules::circuit::{CircuitModule, CircuitMode};
    use spectral_forge::dsp::modules::SpectralModule;
    use spectral_forge::params::{FxChannelTarget, StereoLink};
    use num_complex::Complex;

    let mut module = CircuitModule::new();
    module.reset(48_000.0, 2048);
    module.set_circuit_mode(CircuitMode::ComponentDrift);

    let num_bins = 1025;
    let amount  = vec![2.0_f32; num_bins]; // max drift
    let thresh  = vec![0.0_f32; num_bins];
    let spread  = vec![0.0_f32; num_bins];
    let release = vec![1.0_f32; num_bins];
    let mix     = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &thresh, &spread, &release, &mix];
    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = circuit_test_ctx(num_bins);

    let baseline = 1.0_f32;
    let mut bins: Vec<Complex<f32>> = vec![Complex::new(baseline, 0.0); num_bins];

    let mut max_dev = 0.0_f32;
    // With release=1: drift_tau=5 s, alpha≈0.00213/hop. Per-bin drift_env grows as a
    // random walk: std ≈ alpha * amount_scale * sqrt(N). After 500 hops across 1025 bins
    // the maximum deviation comfortably exceeds the 0.005 bound (analytical std ≈ 0.0057).
    for _ in 0..500 {
        for b in bins.iter_mut() { *b = Complex::new(baseline, 0.0); }
        module.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, None, &curves, &mut suppression, None, &ctx);
        for b in &bins {
            let dev = (b.norm() - baseline).abs();
            if dev > max_dev { max_dev = dev; }
        }
    }
    // ±1 dB ≈ 12% magnitude swing. Deviation should reach a few percent within 500 hops.
    assert!(max_dev > 0.005, "drift should reach measurable deviation (got {})", max_dev);
    assert!(max_dev < 0.5,   "drift should remain bounded (got {})", max_dev);
}

#[test]
fn circuit_pcb_crosstalk_leaks_to_neighbours() {
    use spectral_forge::dsp::modules::circuit::{CircuitModule, CircuitMode};
    use spectral_forge::dsp::modules::SpectralModule;
    use spectral_forge::params::{StereoLink, FxChannelTarget};
    use num_complex::Complex;

    let mut module = CircuitModule::new();
    module.reset(48_000.0, 2048);
    module.set_circuit_mode(CircuitMode::PcbCrosstalk);

    let num_bins = 1025;
    let mut bins: Vec<Complex<f32>> = vec![Complex::new(0.0, 0.0); num_bins];
    bins[200] = Complex::new(1.0, 0.0);

    // AMOUNT=2 (full wet contribution), THRESH=0, SPREAD=1.0 (50% leak), RELEASE=0, MIX=2.
    let amount  = vec![2.0_f32; num_bins];
    let thresh  = vec![0.0_f32; num_bins];
    let spread  = vec![1.0_f32; num_bins];
    let release = vec![0.0_f32; num_bins];
    let mix     = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &thresh, &spread, &release, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = circuit_test_ctx(num_bins);

    module.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, None, &curves, &mut suppression, None, &ctx);

    // Centre bin retains some energy; neighbours pick up.
    assert!(bins[200].norm() < 1.0, "centre should bleed (got {})", bins[200].norm());
    assert!(bins[199].norm() > 0.05, "left neighbour should pick up (got {})", bins[199].norm());
    assert!(bins[201].norm() > 0.05, "right neighbour should pick up (got {})", bins[201].norm());
    // Distant bins should remain zero.
    assert!(bins[150].norm() < 1e-6);
    assert!(bins[250].norm() < 1e-6);
}

#[test]
fn circuit_pcb_crosstalk_amount_zero_disables_leak() {
    use spectral_forge::dsp::modules::circuit::{CircuitModule, CircuitMode};
    use spectral_forge::dsp::modules::SpectralModule;
    use spectral_forge::params::{StereoLink, FxChannelTarget};
    use num_complex::Complex;

    let mut module = CircuitModule::new();
    module.reset(48_000.0, 2048);
    module.set_circuit_mode(CircuitMode::PcbCrosstalk);

    let num_bins = 1025;
    let mut bins: Vec<Complex<f32>> = vec![Complex::new(0.0, 0.0); num_bins];
    bins[200] = Complex::new(1.0, 0.0);

    // AMOUNT=0 (raw passthrough), SPREAD=1.0 (would otherwise leak), MIX=2 (full wet).
    let amount  = vec![0.0_f32; num_bins];
    let thresh  = vec![0.0_f32; num_bins];
    let spread  = vec![1.0_f32; num_bins];
    let release = vec![0.0_f32; num_bins];
    let mix     = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &thresh, &spread, &release, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = circuit_test_ctx(num_bins);
    module.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, None, &curves, &mut suppression, None, &ctx);

    // out_mag = workspace2 * 0 + in_mag * 1 = in_mag — neighbours stay silent, centre intact.
    assert!((bins[200].norm() - 1.0).abs() < 1e-6, "centre should pass through (got {})", bins[200].norm());
    assert!(bins[199].norm() < 1e-6, "left neighbour should not leak (got {})", bins[199].norm());
    assert!(bins[201].norm() < 1e-6, "right neighbour should not leak (got {})", bins[201].norm());
}

#[test]
fn circuit_pcb_crosstalk_spread_average_scales_leak() {
    use spectral_forge::dsp::modules::circuit::{CircuitModule, CircuitMode};
    use spectral_forge::dsp::modules::SpectralModule;
    use spectral_forge::params::{StereoLink, FxChannelTarget};
    use num_complex::Complex;

    // Two passes with the same module: first SPREAD=1.0 uniform (avg 0.5),
    // second SPREAD=0.5 uniform (avg 0.25). Lower average → smaller leak.
    let amount  = vec![2.0_f32; 1025];
    let thresh  = vec![0.0_f32; 1025];
    let release = vec![0.0_f32; 1025];
    let mix     = vec![2.0_f32; 1025];

    let leak_for_spread = |s: f32| -> f32 {
        let mut module = CircuitModule::new();
        module.reset(48_000.0, 2048);
        module.set_circuit_mode(CircuitMode::PcbCrosstalk);

        let num_bins = 1025;
        let mut bins: Vec<Complex<f32>> = vec![Complex::new(0.0, 0.0); num_bins];
        bins[200] = Complex::new(1.0, 0.0);
        let spread = vec![s; num_bins];
        let curves: Vec<&[f32]> = vec![&amount, &thresh, &spread, &release, &mix];
        let mut suppression = vec![0.0_f32; num_bins];
        let ctx = circuit_test_ctx(num_bins);
        module.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, None, &curves, &mut suppression, None, &ctx);
        bins[199].norm()
    };

    let leak_full = leak_for_spread(1.0);
    let leak_half = leak_for_spread(0.5);

    assert!(leak_full > leak_half, "leak at SPREAD=1.0 ({}) should exceed leak at SPREAD=0.5 ({})", leak_full, leak_half);
    assert!(leak_half > 0.0, "leak should still be positive at SPREAD=0.5 (got {})", leak_half);
}

#[test]
fn circuit_vactrol_finite_after_long_run() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::circuit::{CircuitMode, CircuitModule};
    use spectral_forge::dsp::modules::SpectralModule;
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let mut module = CircuitModule::new();
    module.reset(48_000.0, 2048);
    module.set_circuit_mode(CircuitMode::Vactrol);

    let num_bins = 1025;
    let mut bins: Vec<Complex<f32>> = (0..num_bins)
        .map(|k| Complex::new((k as f32 * 0.05).sin().abs(), 0.0))
        .collect();
    let amount  = vec![1.5_f32; num_bins];
    let thresh  = vec![1.0_f32; num_bins];
    let spread  = vec![0.0_f32; num_bins];
    let release = vec![1.0_f32; num_bins];
    let mix     = vec![1.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &thresh, &spread, &release, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = circuit_test_ctx(num_bins);

    for _ in 0..500 {
        module.process(0, StereoLink::Linked, FxChannelTarget::All,
                       &mut bins, None, &curves, &mut suppression, None, &ctx);
        for b in &bins {
            assert!(b.norm().is_finite() && b.norm() < 100.0);
        }
    }
}
