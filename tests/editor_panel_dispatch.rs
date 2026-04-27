use spectral_forge::dsp::modules::{module_spec, ModuleType, PanelWidgetFn};

const ALL_TYPES: &[ModuleType] = &[
    ModuleType::Empty, ModuleType::Dynamics, ModuleType::Freeze,
    ModuleType::PhaseSmear, ModuleType::Contrast, ModuleType::Gain,
    ModuleType::MidSide, ModuleType::TransientSustainedSplit,
    ModuleType::Harmonic, ModuleType::Master,
];

#[test]
fn panel_widget_is_optional_function_pointer() {
    for &ty in ALL_TYPES {
        let spec = module_spec(ty);
        let _: Option<PanelWidgetFn> = spec.panel_widget;
    }
}

#[test]
fn all_shipped_modules_have_panel_widget_none() {
    for &ty in ALL_TYPES {
        assert!(
            module_spec(ty).panel_widget.is_none(),
            "{ty:?} should have panel_widget = None until its panel is implemented",
        );
    }
}
