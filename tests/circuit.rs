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
