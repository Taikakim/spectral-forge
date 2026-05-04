//! Foundation regression tests for the per-mode `CurveLayout` infrastructure
//! introduced by the Past UX overhaul. See
//! docs/superpowers/specs/2026-04-23-ui-parameter-spec-design.md §8.

use spectral_forge::dsp::modules::{module_spec, CurveLayout, ModuleType};

/// CurveLayout struct fields exist with the documented types.
#[test]
fn curve_layout_has_expected_fields() {
    fn empty_help(_: u8) -> &'static str { "" }
    let layout = CurveLayout {
        active:          &[0u8, 2u8, 4u8],
        label_overrides: &[(1u8, "Age"), (1u8, "Delay")],
        help_for:        empty_help,
        mode_overview:   Some("test"),
    };
    assert_eq!(layout.active, &[0, 2, 4]);
    assert_eq!(layout.label_overrides.len(), 2);
    assert_eq!((layout.help_for)(0), "");
    assert_eq!(layout.mode_overview, Some("test"));
}

/// Every existing ModuleSpec defaults `active_layout` to `None`.
/// Modules without modes (Dynamics, Freeze, etc.) must keep the legacy "render
/// all curve_labels" behaviour. Only modules that have explicitly opted in
/// should return `Some`.
#[test]
fn default_module_specs_have_active_layout_none() {
    let mode_bearing = [
        ModuleType::Past, ModuleType::Geometry, ModuleType::Circuit,
        ModuleType::Life, ModuleType::Kinetics, ModuleType::Harmony,
        ModuleType::Modulate, ModuleType::Rhythm,
    ];
    for ty in [
        ModuleType::Dynamics, ModuleType::Freeze, ModuleType::PhaseSmear,
        ModuleType::Contrast, ModuleType::Gain, ModuleType::MidSide,
        ModuleType::TransientSustainedSplit, ModuleType::Future,
        ModuleType::Punch, ModuleType::Harmonic, ModuleType::Master,
        ModuleType::Empty,
    ] {
        if mode_bearing.contains(&ty) { continue; }
        assert!(
            module_spec(ty).active_layout.is_none(),
            "Non-mode-bearing module {:?} unexpectedly has active_layout = Some",
            ty,
        );
    }
}
