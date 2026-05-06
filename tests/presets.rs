#[test]
fn preset_default_has_correct_types() {
    let s = spectral_forge::presets::preset_default();
    use spectral_forge::dsp::modules::ModuleType;
    assert_eq!(s.slot_module_types[0], ModuleType::Dynamics);
    assert_eq!(s.slot_module_types[1], ModuleType::Gain);
    assert_eq!(s.slot_module_types[8], ModuleType::Master);
    for i in 2..8 {
        assert_eq!(s.slot_module_types[i], ModuleType::Empty);
    }
}

#[test]
fn preset_roundtrips_through_json() {
    let s = spectral_forge::presets::preset_default();
    let json = serde_json::to_string(&s).expect("serialize");
    let s2: spectral_forge::presets::PluginState = serde_json::from_str(&json)
        .expect("deserialize");
    assert_eq!(s.slot_module_types, s2.slot_module_types);
}

#[test]
fn all_presets_compile_and_serialize() {
    use spectral_forge::presets::*;
    let builders: &[fn() -> PluginState] = &[
        preset_default,
        preset_transient_sculptor,
        preset_spectral_width,
        preset_phase_sculptor,
        preset_freeze_pad,
    ];
    for build in builders {
        let state = build();
        let json = serde_json::to_string(&state).expect("serialize failed");
        assert!(!json.is_empty());
    }
}
