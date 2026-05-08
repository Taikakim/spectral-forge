//! Circuit scalars: default-correctness + plumbing.
use spectral_forge::dsp::modules::circuit::CircuitScalars;

#[test]
fn circuit_safe_default_matches_hardcoded_values() {
    let s = CircuitScalars::safe_default();
    assert_eq!(s.vactrol_fast_ms, 8.0);
    assert_eq!(s.vactrol_slow_ms, 250.0);
}

#[test]
#[cfg(feature = "probe")]
fn circuit_scalars_round_trip_through_fx_matrix() {
    use spectral_forge::dsp::fx_matrix::FxMatrix;
    use spectral_forge::dsp::modules::ModuleType;

    // Slot 0 = Circuit; everything else Empty.
    let slot_types: [ModuleType; 9] = [
        ModuleType::Circuit, ModuleType::Empty, ModuleType::Empty,
        ModuleType::Empty, ModuleType::Empty, ModuleType::Empty,
        ModuleType::Empty, ModuleType::Empty, ModuleType::Empty,
    ];
    let mut fxm = FxMatrix::new(48_000.0, 2048, &slot_types);

    let custom = CircuitScalars {
        vactrol_fast_ms: 4.0,
        vactrol_slow_ms: 500.0,
    };
    let mut arr = [CircuitScalars::safe_default(); 9];
    arr[0] = custom;

    fxm.set_circuit_scalars(&arr);
    let read_back = fxm.test_circuit_scalars(0).expect("slot 0 should hold Circuit");
    assert!((read_back.vactrol_fast_ms - 4.0).abs() < 1e-6);
    assert!((read_back.vactrol_slow_ms - 500.0).abs() < 1e-6);
}
