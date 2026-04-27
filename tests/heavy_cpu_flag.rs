use spectral_forge::dsp::modules::{ModuleType, create_module};

#[test]
fn heavy_cpu_flag_defaults_to_module_spec_value() {
    let m = create_module(ModuleType::Dynamics, 48000.0, 2048);
    // Dynamics is light; default heavy_cpu_for_mode() must be false.
    assert!(!m.heavy_cpu_for_mode());
}

#[test]
fn empty_and_master_are_never_heavy() {
    for ty in [ModuleType::Empty, ModuleType::Master] {
        let m = create_module(ty, 48000.0, 2048);
        assert!(!m.heavy_cpu_for_mode());
    }
}
