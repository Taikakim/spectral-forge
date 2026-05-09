use spectral_forge::dsp::modules::{module_spec, ModuleType, PanelWidgetFn};

const ALL_TYPES: &[ModuleType] = &[
    ModuleType::Empty, ModuleType::Dynamics, ModuleType::Freeze,
    ModuleType::PhaseSmear, ModuleType::Contrast, ModuleType::Gain,
    ModuleType::MidSide, ModuleType::TransientSustainedSplit,
    ModuleType::Harmonic, ModuleType::Rhythm, ModuleType::Master,
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
        // Rhythm is the first module to opt in to a panel widget (Phase 2d.7);
        // every other shipped module still defers its panel work.
        if ty == ModuleType::Rhythm { continue; }
        // Contrast gains a dev-build-only mode picker + scalar knobs panel.
        // In release builds panel_widget is still None; in dev-build it is Some.
        #[cfg(not(feature = "dev-build"))]
        assert!(
            module_spec(ty).panel_widget.is_none(),
            "{ty:?} should have panel_widget = None until its panel is implemented",
        );
        // Under dev-build, Contrast is allowed to have Some(panel_widget).
        #[cfg(feature = "dev-build")]
        if ty != ModuleType::Contrast {
            assert!(
                module_spec(ty).panel_widget.is_none(),
                "{ty:?} should have panel_widget = None until its panel is implemented",
            );
        }
    }
}

#[test]
fn rhythm_has_panel_widget_none_post_2026_05_08() {
    // Rhythm's Arpeggiator grid moved into the editor's Dynamics-panel
    // row inline (so it doesn't push the rest of the UI down). The
    // module-spec panel_widget hook is no longer used.
    assert!(
        module_spec(ModuleType::Rhythm).panel_widget.is_none(),
        "Rhythm's Arpeggiator grid is rendered inline now, not via panel_widget",
    );
}

/// Verifies that when a slot's module has `active_layout = Some(...)`, the
/// editor consults the layout to decide which curve tabs are rendered.
/// Logical-level check; UI snapshot deferred to manual visual validation.
#[test]
fn past_active_layout_shapes_visible_curves_per_mode() {
    use spectral_forge::dsp::modules::past::PastMode;

    let layout_fn = module_spec(ModuleType::Past).active_layout
        .expect("Past has active_layout");

    // E-2: SPREAD (curve 3) is now active in EVERY mode, so all counts
    // bumped by 1 except Granular which already had it.
    assert_eq!(layout_fn(PastMode::Granular as u8).active.len(),    5);
    assert_eq!(layout_fn(PastMode::DecaySorter as u8).active.len(), 4);
    assert_eq!(layout_fn(PastMode::Convolution as u8).active.len(), 5);
    assert_eq!(layout_fn(PastMode::Reverse as u8).active.len(),     4);
    assert_eq!(layout_fn(PastMode::Stretch as u8).active.len(),     3);

    // Non-mode-bearing modules return None and the editor falls back to
    // rendering all curve_labels.
    assert!(module_spec(ModuleType::Dynamics).active_layout.is_none());
}

#[test]
fn past_module_spec_advertises_panel_widget() {
    use spectral_forge::dsp::modules::{module_spec, ModuleType};
    let spec = module_spec(ModuleType::Past);
    assert!(
        spec.panel_widget.is_some(),
        "Past must declare a panel_widget for Soft Clip + scalars",
    );
}
