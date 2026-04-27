use spectral_forge::dsp::modules::{ModuleType, module_spec};

#[test]
fn punch_module_spec() {
    let spec = module_spec(ModuleType::Punch);
    assert_eq!(spec.num_curves, 6);
    assert_eq!(spec.curve_labels, &["AMOUNT", "WIDTH", "FILL_MODE", "AMP_FILL", "HEAL", "MIX"]);
    assert!(spec.supports_sidechain);
    assert!(spec.wants_sidechain);
    assert_eq!(spec.display_name, "Punch");
}
