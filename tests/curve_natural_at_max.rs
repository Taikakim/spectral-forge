//! C-1 regression: curves where y_natural == y_max must default the offset
//! FloatParam to +1.0 (loads user at y_max). See
//! docs/superpowers/specs/2026-05-07-stabilization-sweep-bc-design.md §C-1.

use spectral_forge::editor::curve_config::curve_display_config;
use spectral_forge::dsp::modules::{ModuleType, GainMode};
use spectral_forge::params::SpectralForgeParams;

#[test]
fn natural_at_max_flag_consistent_with_y_natural_eq_y_max() {
    let modules = [
        ModuleType::Dynamics, ModuleType::Freeze, ModuleType::PhaseSmear,
        ModuleType::Contrast, ModuleType::Gain, ModuleType::MidSide,
        ModuleType::TransientSustainedSplit, ModuleType::Harmonic,
        ModuleType::Past, ModuleType::Geometry, ModuleType::Circuit,
        ModuleType::Life, ModuleType::Modulate, ModuleType::Rhythm,
        ModuleType::Punch, ModuleType::Harmony, ModuleType::Kinetics,
        ModuleType::Future,
    ];
    for &m in &modules {
        for c in 0..7 {
            let cfg = curve_display_config(m, c, GainMode::Add);
            let inferred = (cfg.y_natural - cfg.y_max).abs() < 1e-6;
            assert_eq!(cfg.natural_at_max, inferred,
                "{:?}/{}: y_natural={:.3}, y_max={:.3}, flag={}",
                m, c, cfg.y_natural, cfg.y_max, cfg.natural_at_max);
        }
    }
}

#[test]
fn dynamics_mix_offset_default_is_plus_one() {
    let p = SpectralForgeParams::default();
    let slot = 0;
    // Dynamics MIX is at local curve index 5.
    // Default slot 0 = Dynamics; MIX (curve 5) is natural-at-max (y_natural==y_max==100%).
    let mix_curve = 5;
    let off = p.offset_param(slot, mix_curve).unwrap().value();
    assert_eq!(off, 1.0,
        "Dynamics MIX offset default must be +1.0 (loads at y_max=100% wet)");
}

#[test]
fn dynamics_threshold_offset_default_is_zero() {
    let p = SpectralForgeParams::default();
    let off = p.offset_param(0, 0).unwrap().value();
    assert_eq!(off, 0.0,
        "Dynamics THRESHOLD offset default must remain 0.0 (not natural-at-max)");
}
