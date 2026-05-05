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

/// Past opts into the `active_layout` infra (Task 7). The Granular layout
/// covers all 5 curves; verify the wired function returns the expected shape.
#[test]
fn past_module_spec_has_active_layout_some() {
    use spectral_forge::dsp::modules::past::PastMode;
    let spec = module_spec(ModuleType::Past);
    let layout_fn = spec.active_layout
        .expect("Past must opt in to active_layout for the per-mode UI to work");
    let granular = layout_fn(PastMode::Granular as u8);
    assert_eq!(granular.active, &[0u8, 1, 2, 3, 4]);
}

/// Every existing ModuleSpec defaults `active_layout` to `None`.
/// Modules without modes (Dynamics, Freeze, etc.) must keep the legacy "render
/// all curve_labels" behaviour. Only modules that have explicitly opted in
/// should return `Some`.
#[test]
fn default_module_specs_have_active_layout_none() {
    // All multi-mode modules that have opted into active_layout.
    let mode_bearing = [
        ModuleType::Past, ModuleType::Future, ModuleType::Circuit,
        ModuleType::Geometry, ModuleType::Punch, ModuleType::Rhythm,
        ModuleType::Modulate, ModuleType::Harmony, ModuleType::Kinetics,
        ModuleType::Life,
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

/// Future has 2 modes: PrintThrough and PreEcho.
/// PrintThrough reads AMOUNT(0), TIME(1), SPREAD(3), MIX(4) — no THRESHOLD.
/// PreEcho reads all 5: AMOUNT(0), TIME(1), THRESHOLD(2), SPREAD(3), MIX(4).
#[test]
fn future_active_layout_matches_kernel_signatures() {
    use spectral_forge::dsp::modules::future::FutureMode;

    let layout_fn = module_spec(ModuleType::Future).active_layout
        .expect("Future should declare an active_layout");

    let layout_pt = layout_fn(FutureMode::PrintThrough as u8);
    assert_eq!(layout_pt.active, &[0u8, 1, 3, 4],
        "PrintThrough should expose AMOUNT, TIME, SPREAD, MIX (no THRESHOLD)");

    let layout_pe = layout_fn(FutureMode::PreEcho as u8);
    assert_eq!(layout_pe.active, &[0u8, 1, 2, 3, 4],
        "PreEcho should expose all 5 curves including THRESHOLD");
}

/// Kinetics has 8 modes. Curves: 0=STRENGTH, 1=MASS, 2=REACH, 3=DAMPING, 4=MIX.
/// Highly variable per-mode — InertialMass uses only [1,4]; OrbitalPhase only [0,4].
#[test]
fn kinetics_active_layout_matches_kernel_signatures() {
    use spectral_forge::dsp::modules::kinetics::KineticsMode;

    let layout_fn = module_spec(ModuleType::Kinetics).active_layout
        .expect("Kinetics should declare an active_layout");

    let modes_and_active: &[(KineticsMode, &[u8])] = &[
        (KineticsMode::Hooke,            &[0, 1, 2, 3, 4]),
        (KineticsMode::GravityWell,      &[0, 1, 2, 3, 4]),
        (KineticsMode::InertialMass,     &[1, 4]),
        (KineticsMode::OrbitalPhase,     &[0, 4]),
        (KineticsMode::Ferromagnetism,   &[0, 2, 3, 4]),
        (KineticsMode::ThermalExpansion, &[0, 3, 4]),
        (KineticsMode::TuningFork,       &[0, 2, 4]),
        (KineticsMode::Diamagnet,        &[0, 2, 4]),
    ];
    for (mode, expected) in modes_and_active {
        let layout = layout_fn(*mode as u8);
        assert_eq!(layout.active, *expected,
            "Kinetics {:?}: expected active {:?}, got {:?}", mode, expected, layout.active);
    }
}

/// Harmony has 8 modes. Curves: 0=AMOUNT, 1=THRESHOLD, 2=STABILITY, 3=SPREAD,
/// 4=COEFFICIENT, 5=MIX. STABILITY(2) is unused by all current kernels.
#[test]
fn harmony_active_layout_matches_kernel_signatures() {
    use spectral_forge::dsp::modules::harmony::HarmonyMode;

    let layout_fn = module_spec(ModuleType::Harmony).active_layout
        .expect("Harmony should declare an active_layout");

    let modes_and_active: &[(HarmonyMode, &[u8])] = &[
        (HarmonyMode::Chordification,    &[0, 1, 3, 5]),
        (HarmonyMode::Undertone,         &[0, 1, 3, 4, 5]),
        (HarmonyMode::Companding,        &[0, 4, 5]),
        (HarmonyMode::FormantRotation,   &[0, 4, 5]),
        (HarmonyMode::Lifter,            &[0, 3, 4, 5]),
        (HarmonyMode::Inharmonic,        &[0, 1, 4, 5]),
        (HarmonyMode::HarmonicGenerator, &[0, 1, 3, 4, 5]),
        (HarmonyMode::Shuffler,          &[0, 1, 3, 5]),
    ];
    for (mode, expected) in modes_and_active {
        let layout = layout_fn(*mode as u8);
        assert_eq!(layout.active, *expected,
            "Harmony {:?}: expected active {:?}, got {:?}", mode, expected, layout.active);
    }
}

/// Modulate has 8 modes. Curves: 0=AMOUNT, 1=REACH, 2=RATE, 3=THRESH, 4=AMPGATE, 5=MIX.
/// Each mode reads a distinct subset of curves.
#[test]
fn modulate_active_layout_matches_kernel_signatures() {
    use spectral_forge::dsp::modules::modulate::ModulateMode;

    let layout_fn = module_spec(ModuleType::Modulate).active_layout
        .expect("Modulate should declare an active_layout");

    let modes_and_active: &[(ModulateMode, &[u8])] = &[
        (ModulateMode::PhasePhaser,  &[0, 2, 3, 4, 5]),
        (ModulateMode::BinSwapper,   &[0, 1, 3, 5]),
        (ModulateMode::RmFmMatrix,   &[0, 1, 3, 5]),
        (ModulateMode::DiodeRm,      &[0, 1, 3, 5]),
        (ModulateMode::GroundLoop,   &[0, 1, 2, 3, 5]),
        (ModulateMode::GravityPhaser,&[0, 1, 3, 4, 5]),
        (ModulateMode::PllTear,      &[0, 1, 2, 3, 5]),
        (ModulateMode::FmNetwork,    &[0, 1, 4, 5]),
    ];
    for (mode, expected) in modes_and_active {
        let layout = layout_fn(*mode as u8);
        assert_eq!(layout.active, *expected,
            "Modulate {:?}: expected active {:?}, got {:?}", mode, expected, layout.active);
    }
}

/// Rhythm has 3 modes. Curves: 0=AMOUNT, 1=DIVISION, 2=ATTACK_FADE, 3=TARGET_PHASE, 4=MIX.
/// Euclidean and Arpeggiator do not read TARGET_PHASE(3); PhaseReset reads all 5.
#[test]
fn rhythm_active_layout_matches_kernel_signatures() {
    use spectral_forge::dsp::modules::rhythm::RhythmMode;

    let layout_fn = module_spec(ModuleType::Rhythm).active_layout
        .expect("Rhythm should declare an active_layout");

    let modes_and_active: &[(RhythmMode, &[u8])] = &[
        (RhythmMode::Euclidean,   &[0, 1, 2, 4]),
        (RhythmMode::Arpeggiator, &[0, 1, 2, 4]),
        (RhythmMode::PhaseReset,  &[0, 1, 2, 3, 4]),
    ];
    for (mode, expected) in modes_and_active {
        let layout = layout_fn(*mode as u8);
        assert_eq!(layout.active, *expected,
            "Rhythm {:?}: expected active {:?}, got {:?}", mode, expected, layout.active);
    }
}

/// Punch has 2 modes. Curves: 0=AMOUNT, 1=WIDTH, 2=FILL_MODE, 3=AMP_FILL, 4=HEAL, 5=MIX.
/// Both Direct and Inverse use the same kernel and read all 6 curves.
#[test]
fn punch_active_layout_matches_kernel_signatures() {
    use spectral_forge::dsp::modules::punch::PunchMode;

    let layout_fn = module_spec(ModuleType::Punch).active_layout
        .expect("Punch should declare an active_layout");

    let modes_and_active: &[(PunchMode, &[u8])] = &[
        (PunchMode::Direct,  &[0, 1, 2, 3, 4, 5]),
        (PunchMode::Inverse, &[0, 1, 2, 3, 4, 5]),
    ];
    for (mode, expected) in modes_and_active {
        let layout = layout_fn(*mode as u8);
        assert_eq!(layout.active, *expected,
            "Punch {:?}: expected active {:?}, got {:?}", mode, expected, layout.active);
    }
}

/// Geometry has 2 modes. Curves: 0=AMOUNT, 1=MODE_CAP, 2=DAMP_REL, 3=THRESH, 4=MIX.
/// Chladni does not read THRESH(3); Helmholtz reads all 5.
#[test]
fn geometry_active_layout_matches_kernel_signatures() {
    use spectral_forge::dsp::modules::geometry::GeometryMode;

    let layout_fn = module_spec(ModuleType::Geometry).active_layout
        .expect("Geometry should declare an active_layout");

    let modes_and_active: &[(GeometryMode, &[u8])] = &[
        (GeometryMode::Chladni,    &[0, 1, 2, 4]),
        (GeometryMode::Helmholtz,  &[0, 1, 2, 3, 4]),
    ];
    for (mode, expected) in modes_and_active {
        let layout = layout_fn(*mode as u8);
        assert_eq!(layout.active, *expected,
            "Geometry {:?}: expected active {:?}, got {:?}", mode, expected, layout.active);
    }
}

/// Circuit has 10 modes. Kernels were inspected to determine which curve indices
/// each one actually reads. Curves: 0=AMOUNT, 1=THRESH, 2=SPREAD, 3=RELEASE, 4=MIX.
#[test]
fn circuit_active_layout_matches_kernel_signatures() {
    use spectral_forge::dsp::modules::circuit::CircuitMode;

    let layout_fn = module_spec(ModuleType::Circuit).active_layout
        .expect("Circuit should declare an active_layout");

    let modes_and_active: &[(CircuitMode, &[u8])] = &[
        (CircuitMode::BbdBins,               &[0, 1, 3, 4]),
        (CircuitMode::SpectralSchmitt,       &[0, 1, 3, 4]),
        (CircuitMode::CrossoverDistortion,   &[0, 4]),
        (CircuitMode::Vactrol,               &[0, 3, 4]),
        (CircuitMode::TransformerSaturation, &[0, 1, 2, 3, 4]),
        (CircuitMode::PowerSag,              &[0, 1, 3, 4]),
        (CircuitMode::ComponentDrift,        &[0, 1, 3, 4]),
        (CircuitMode::PcbCrosstalk,          &[0, 2, 4]),
        (CircuitMode::SlewDistortion,        &[0, 1, 3, 4]),
        (CircuitMode::BiasFuzz,              &[0, 1, 2, 3, 4]),
    ];
    for (mode, expected) in modes_and_active {
        let layout = layout_fn(*mode as u8);
        assert_eq!(layout.active, *expected,
            "Circuit mode {:?}: expected active {:?}, got {:?}",
            mode, expected, layout.active);
    }
}
