// UI parameter contract: see docs/superpowers/specs/2026-04-23-ui-parameter-spec-design.md §1
// This is the ONLY place where per-curve display ranges, grid lines, and unit
// labels are defined. Do not hardcode these values anywhere else.

use crate::dsp::modules::{GainMode, ModuleType};

/// Single source of truth for all display properties of one curve.
/// See docs/superpowers/specs/2026-04-23-ui-parameter-spec-design.md §1.
pub struct CurveDisplayConfig {
    /// Physical unit label shown on the Y-axis: "dBFS", "ratio", "ms", "dB", "%", "".
    pub y_label:    &'static str,
    /// Bottom of the physical display range.
    pub y_min:      f32,
    /// Top of the physical display range.
    pub y_max:      f32,
    /// true = logarithmic Y spacing; false = linear.
    pub y_log:      bool,
    /// Exactly 4 horizontal guide lines: (physical_value, display_label).
    pub grid_lines: &'static [(f32, &'static str)],
    // NOTE: gain_to_phys is intentionally absent — unit conversion requires context
    // (db_min/db_max, global_attack_ms etc.) that a bare fn(f32)->f32 cannot carry.
    // Conversion logic lives in gain_to_display() / screen_y_to_physical() in curve.rs.
}

/// Return the display config for a given module type, curve index, and gain mode.
/// Every module's every curve must have an entry — add one before writing any
/// display code for a new module type.
/// See docs/superpowers/specs/2026-04-23-ui-parameter-spec-design.md §1.
pub fn curve_display_config(
    module_type: ModuleType,
    curve_idx:   usize,
    gain_mode:   GainMode,
) -> CurveDisplayConfig {
    match module_type {
        ModuleType::Dynamics => dynamics_config(curve_idx),
        ModuleType::Freeze   => freeze_config(curve_idx),
        ModuleType::PhaseSmear => phase_smear_config(curve_idx),
        ModuleType::Contrast => contrast_config(curve_idx),
        ModuleType::Gain     => gain_config(curve_idx, gain_mode),
        ModuleType::MidSide  => mid_side_config(curve_idx),
        ModuleType::TransientSustainedSplit => ts_split_config(curve_idx),
        // Modules with no display curves:
        ModuleType::Harmonic | ModuleType::Master | ModuleType::Empty => default_config(),
    }
}

// ── Per-module config helpers ────────────────────────────────────────────────

fn dynamics_config(i: usize) -> CurveDisplayConfig {
    // 0 Threshold · 1 Ratio · 2 Attack · 3 Release · 4 Knee · 5 Mix
    // (Dynamics has 6 curves; MAKEUP is handled by the standalone Gain module)
    match i {
        0 => CurveDisplayConfig {
            y_label: "dBFS", y_min: -60.0, y_max: 0.0, y_log: false,
            grid_lines: &[(-12.0, "-12"), (-24.0, "-24"), (-36.0, "-36"), (-48.0, "-48")],
        },
        1 => CurveDisplayConfig {
            y_label: "ratio", y_min: 1.0, y_max: 20.0, y_log: true,
            grid_lines: &[(1.5, "1:1.5"), (2.5, "1:2.5"), (5.0, "1:5"), (10.0, "1:10")],
        },
        2 | 3 => CurveDisplayConfig {
            y_label: "ms", y_min: 1.0, y_max: 1024.0, y_log: true,
            grid_lines: &[(4.0, "4ms"), (16.0, "16ms"), (64.0, "64ms"), (256.0, "256ms")],
        },
        4 => CurveDisplayConfig {
            y_label: "dB", y_min: 0.0, y_max: 48.0, y_log: false,
            grid_lines: &[(6.0, "6dB"), (12.0, "12dB"), (24.0, "24dB"), (36.0, "36dB")],
        },
        5 => CurveDisplayConfig {
            y_label: "%", y_min: 0.0, y_max: 100.0, y_log: false,
            grid_lines: &[(25.0, "25%"), (50.0, "50%"), (75.0, "75%"), (100.0, "100%")],
        },
        _ => default_config(),
    }
}

fn freeze_config(i: usize) -> CurveDisplayConfig {
    match i {
        0 => CurveDisplayConfig {
            y_label: "ms", y_min: 10.0, y_max: 4000.0, y_log: true,
            grid_lines: &[(100.0, "100ms"), (500.0, "500ms"), (1000.0, "1s"), (2000.0, "2s")],
        },
        1 => CurveDisplayConfig {
            y_label: "dBFS", y_min: -80.0, y_max: 0.0, y_log: false,
            grid_lines: &[(-12.0, "-12"), (-40.0, "-40"), (-60.0, "-60"), (-80.0, "-80")],
        },
        2 => CurveDisplayConfig {
            y_label: "ms", y_min: 1.0, y_max: 1000.0, y_log: true,
            grid_lines: &[(10.0, "10ms"), (100.0, "100ms"), (500.0, "500ms"), (1000.0, "1s")],
        },
        3 => CurveDisplayConfig {
            y_label: "", y_min: 0.0, y_max: 2.0, y_log: false,
            grid_lines: &[(0.5, "0.5"), (1.0, "1.0"), (1.5, "1.5"), (2.0, "2.0")],
        },
        4 => CurveDisplayConfig {
            y_label: "%", y_min: 0.0, y_max: 100.0, y_log: false,
            grid_lines: &[(20.0, "20%"), (40.0, "40%"), (60.0, "60%"), (80.0, "80%")],
        },
        _ => default_config(),
    }
}

fn phase_smear_config(i: usize) -> CurveDisplayConfig {
    match i {
        0 => CurveDisplayConfig {
            y_label: "%", y_min: 0.0, y_max: 200.0, y_log: false,
            grid_lines: &[(50.0, "50%"), (100.0, "100%"), (150.0, "150%"), (200.0, "200%")],
        },
        1 => CurveDisplayConfig {
            y_label: "ms", y_min: 1.0, y_max: 1000.0, y_log: true,
            grid_lines: &[(10.0, "10ms"), (100.0, "100ms"), (500.0, "500ms"), (1000.0, "1s")],
        },
        2 => CurveDisplayConfig {
            y_label: "%", y_min: 0.0, y_max: 100.0, y_log: false,
            grid_lines: &[(20.0, "20%"), (40.0, "40%"), (60.0, "60%"), (80.0, "80%")],
        },
        _ => default_config(),
    }
}

fn contrast_config(i: usize) -> CurveDisplayConfig {
    match i {
        0 => CurveDisplayConfig {
            y_label: "ratio", y_min: 1.0, y_max: 20.0, y_log: true,
            grid_lines: &[(1.25, "1:1.25"), (2.5, "1:2.5"), (5.0, "1:5"), (10.0, "1:10")],
        },
        _ => default_config(),
    }
}

fn gain_config(i: usize, gain_mode: GainMode) -> CurveDisplayConfig {
    match i {
        0 => match gain_mode {
            GainMode::Pull | GainMode::Match => CurveDisplayConfig {
                y_label: "%", y_min: 0.0, y_max: 100.0, y_log: false,
                grid_lines: &[(75.0, "75%"), (50.0, "50%"), (25.0, "25%"), (0.0, "0%")],
            },
            _ => CurveDisplayConfig {
                y_label: "dB", y_min: -18.0, y_max: 18.0, y_log: false,
                grid_lines: &[(-12.0, "-12dB"), (-6.0, "-6dB"), (6.0, "+6dB"), (12.0, "+12dB")],
            },
        },
        1 => CurveDisplayConfig {
            y_label: "ms", y_min: 1.0, y_max: 1000.0, y_log: true,
            grid_lines: &[(10.0, "10ms"), (100.0, "100ms"), (500.0, "500ms"), (1000.0, "1s")],
        },
        _ => default_config(),
    }
}

fn mid_side_config(i: usize) -> CurveDisplayConfig {
    match i {
        0 | 1 => CurveDisplayConfig {
            y_label: "%", y_min: 0.0, y_max: 200.0, y_log: false,
            grid_lines: &[(50.0, "50%"), (100.0, "100%"), (150.0, "150%"), (200.0, "200%")],
        },
        // 2 DECORREL · 3 TRANSIENT · 4 PAN — all use 0–100% range
        _ => CurveDisplayConfig {
            y_label: "%", y_min: 0.0, y_max: 100.0, y_log: false,
            grid_lines: &[(20.0, "20%"), (40.0, "40%"), (60.0, "60%"), (80.0, "80%")],
        },
    }
}

fn ts_split_config(i: usize) -> CurveDisplayConfig {
    match i {
        0 => CurveDisplayConfig {
            y_label: "%", y_min: 0.0, y_max: 100.0, y_log: false,
            grid_lines: &[(20.0, "20%"), (40.0, "40%"), (60.0, "60%"), (80.0, "80%")],
        },
        _ => default_config(),
    }
}

fn default_config() -> CurveDisplayConfig {
    CurveDisplayConfig {
        y_label: "", y_min: 0.0, y_max: 1.0, y_log: false,
        grid_lines: &[(0.25, ""), (0.5, ""), (0.75, ""), (1.0, "")],
    }
}
