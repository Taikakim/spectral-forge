//! Modulate scalars: default-correctness + plumbing.
use spectral_forge::dsp::modules::modulate::ModulateScalars;
use std::f32::consts::FRAC_PI_2;

#[test]
fn modulate_safe_default_matches_hardcoded_values() {
    let s = ModulateScalars::safe_default();
    assert!((s.damping - 0.707).abs() < 1e-6);
    assert!((s.tear_angle_rad - FRAC_PI_2).abs() < 1e-6);
}

#[test]
#[cfg(feature = "probe")]
fn modulate_scalars_round_trip_through_fx_matrix() {
    use spectral_forge::dsp::fx_matrix::FxMatrix;
    use spectral_forge::dsp::modules::ModuleType;

    let mut fxm = FxMatrix::new(48_000.0, 2048, &[ModuleType::Modulate, ModuleType::Empty, ModuleType::Empty, ModuleType::Empty, ModuleType::Empty, ModuleType::Empty, ModuleType::Empty, ModuleType::Empty, ModuleType::Empty]);

    let custom = ModulateScalars {
        damping:        1.5,
        tear_angle_rad: 2.0,
    };
    let mut arr = [ModulateScalars::safe_default(); 9];
    arr[0] = custom;

    fxm.set_modulate_scalars(&arr);
    let read_back = fxm.test_modulate_scalars(0).expect("slot 0 should hold Modulate");
    assert!((read_back.damping - 1.5).abs() < 1e-6);
    assert!((read_back.tear_angle_rad - 2.0).abs() < 1e-6);
}
