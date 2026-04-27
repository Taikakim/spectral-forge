use spectral_forge::dsp::modules::{ModuleType, module_spec};

#[test]
fn future_module_spec_has_5_curves() {
    let spec = module_spec(ModuleType::Future);
    assert_eq!(spec.num_curves, 5);
    assert_eq!(spec.curve_labels, &["AMOUNT", "TIME", "THRESHOLD", "SPREAD", "MIX"]);
    assert!(!spec.supports_sidechain);
    assert_eq!(spec.display_name, "Future");
}
