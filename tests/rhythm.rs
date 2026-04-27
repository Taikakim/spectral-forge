use spectral_forge::dsp::modules::{ModuleType, module_spec};

#[test]
fn rhythm_module_spec_basic() {
    let spec = module_spec(ModuleType::Rhythm);
    assert_eq!(spec.num_curves, 5);
    assert_eq!(spec.curve_labels, &["AMOUNT", "DIVISION", "ATTACK_FADE", "TARGET_PHASE", "MIX"]);
    assert!(!spec.supports_sidechain);
    assert!(!spec.wants_sidechain);
    assert_eq!(spec.display_name, "Rhythm");
}
