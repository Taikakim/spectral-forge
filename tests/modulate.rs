use spectral_forge::dsp::modules::{module_spec, ModuleType};

#[test]
fn modulate_module_spec_present() {
    let spec = module_spec(ModuleType::Modulate);
    assert_eq!(spec.display_name, "Modulate");
    assert_eq!(spec.num_curves, 6);
    assert_eq!(spec.curve_labels.len(), 6);
    assert_eq!(spec.curve_labels, &["AMOUNT", "REACH", "RATE", "THRESH", "AMPGATE", "MIX"]);
    assert!(spec.supports_sidechain, "Modulate must support sidechain (RM/Diode RM modes)");
    assert!(spec.wants_sidechain, "RM/Diode RM modes need sidechain auto-routed");
}
