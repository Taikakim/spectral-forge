#[test]
fn panel_widget_is_optional_function_pointer() {
    use spectral_forge::dsp::modules::{ModuleType, module_spec, PanelWidgetFn};
    for ty in [ModuleType::Dynamics, ModuleType::Freeze, ModuleType::Empty] {
        let spec = module_spec(ty);
        // Compile-time check: the field is an Option<PanelWidgetFn>.
        let _: Option<PanelWidgetFn> = spec.panel_widget;
    }
}
