//! Kinetics scalars: default-correctness + plumbing.
use spectral_forge::dsp::modules::kinetics::KineticsScalars;

#[test]
fn kinetics_safe_default_matches_hardcoded_values() {
    let s = KineticsScalars::safe_default();
    assert_eq!(s.sc_envelope_tau_hops, 1.0);
    assert_eq!(s.sc_mass_rate_scale, 5.0);
    assert_eq!(s.tuning_fork_min_sep, 4.0);
    assert_eq!(s.orbital_sat_half_window, 16.0);
    assert_eq!(s.orbital_peak_threshold_factor, 2.0);
    assert!((s.static_well_baseline - 1.05).abs() < 1e-6);
    assert!((s.sc_well_threshold_frac - 0.4).abs() < 1e-6);
}

#[test]
#[cfg(feature = "probe")]
fn kinetics_scalars_round_trip_through_fx_matrix() {
    use spectral_forge::dsp::fx_matrix::FxMatrix;
    use spectral_forge::dsp::modules::ModuleType;

    // Slot 0 = Kinetics; everything else Empty.
    let slot_types: [ModuleType; 9] = [
        ModuleType::Kinetics, ModuleType::Empty, ModuleType::Empty,
        ModuleType::Empty, ModuleType::Empty, ModuleType::Empty,
        ModuleType::Empty, ModuleType::Empty, ModuleType::Empty,
    ];
    let mut fxm = FxMatrix::new(48_000.0, 2048, &slot_types);

    let custom = KineticsScalars {
        sc_well_threshold_frac: 0.7,
        ..KineticsScalars::safe_default()
    };
    let mut arr = [KineticsScalars::safe_default(); 9];
    arr[0] = custom;

    fxm.set_kinetics_scalars(&arr);
    let read_back = fxm.test_kinetics_scalars(0).expect("slot 0 should hold Kinetics");
    assert!((read_back.sc_well_threshold_frac - 0.7).abs() < 1e-6);
}
