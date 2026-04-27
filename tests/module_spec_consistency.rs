use spectral_forge::dsp::modules::{ModuleType, module_spec, create_module};

const ALL_TYPES: &[ModuleType] = &[
    ModuleType::Empty, ModuleType::Dynamics, ModuleType::Freeze,
    ModuleType::PhaseSmear, ModuleType::Contrast, ModuleType::Gain,
    ModuleType::MidSide, ModuleType::TransientSustainedSplit,
    ModuleType::Harmonic, ModuleType::Master,
];

#[test]
fn module_spec_num_curves_matches_module_num_curves() {
    for &t in ALL_TYPES {
        let spec = module_spec(t);
        let module = create_module(t, 48000.0, 2048);
        assert_eq!(
            spec.num_curves, module.num_curves(),
            "ModuleSpec disagrees with module for {:?}", t,
        );
    }
}

#[test]
fn module_spec_wants_sidechain_default_false_for_non_sc_modules() {
    // Modules that take no sidechain input must not request sidechain by default.
    assert!(!module_spec(ModuleType::MidSide).wants_sidechain);
    assert!(!module_spec(ModuleType::Contrast).wants_sidechain);
    assert!(!module_spec(ModuleType::Harmonic).wants_sidechain);
    assert!(!module_spec(ModuleType::Master).wants_sidechain);
    assert!(!module_spec(ModuleType::Empty).wants_sidechain);
    // Sidechain-capable modules also default to false: opt-in is intentional
    // so existing presets don't auto-route a fresh slot to a stale aux.
    assert!(!module_spec(ModuleType::Dynamics).wants_sidechain);
}
