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
    /// Physical value when gain = 1.0 (the curve's natural neutral).
    /// Used by offset_fn and the offset knob formatter to anchor the display.
    pub y_natural:  f32,
    /// Calibrated offset function: takes raw curve gain `g` and normalized offset `o ∈ [-1, 1]`,
    /// returns the transformed gain.  Must be a plain fn pointer (no closure captures) so it
    /// is zero-cost and safe to call on the audio thread.
    /// Contract: offset_fn(g, 0.0) == g for all g.
    pub offset_fn:  fn(f32, f32, (f32, f32, f32)) -> f32,
    /// True when `y_natural == y_max` — i.e. the curve's neutral value is already at the top
    /// of the display range. The offset FloatParam for such curves defaults to `+1.0` so the
    /// user loads at `y_max` (e.g. 100% wet for MIX) and slides down toward `y_min`.
    /// The slider mechanism stays universal `−1..+1`.
    pub natural_at_max: bool,
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
        ModuleType::Future   => future_config(curve_idx),
        ModuleType::Punch    => punch_config(curve_idx),
        ModuleType::Rhythm   => rhythm_config(curve_idx),
        ModuleType::Geometry => geometry_config(curve_idx),
        ModuleType::Modulate => modulate_config(curve_idx),
        ModuleType::Circuit  => circuit_config(curve_idx),
        ModuleType::Life     => life_config(curve_idx),
        ModuleType::Past     => past_config(curve_idx, 0),
        ModuleType::Kinetics => kinetics_config(curve_idx),
        ModuleType::Harmony  => harmony_config(curve_idx),
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
            y_label: "dBFS", y_min: -160.0, y_max: 0.0, y_log: false,
            grid_lines: &[(-20.0, "-20"), (-60.0, "-60"), (-100.0, "-100"), (-140.0, "-140")],
            y_natural: -20.0,
            offset_fn: off_thresh,
            natural_at_max: false,
        },
        1 => CurveDisplayConfig {
            y_label: "ratio", y_min: 1.0, y_max: 20.0, y_log: true,
            grid_lines: &[(1.5, "1:1.5"), (2.5, "1:2.5"), (5.0, "1:5"), (10.0, "1:10")],
            // gain=1.0 → ratio 1:1 (no compression); off=+1 → g=20.0 → ratio 20:1
            y_natural: 1.0,
            offset_fn: off_ratio,
            natural_at_max: false,
        },
        2 | 3 => CurveDisplayConfig {
            y_label: "ms", y_min: 1.0, y_max: 1024.0, y_log: true,
            grid_lines: &[(4.0, "4ms"), (16.0, "16ms"), (64.0, "64ms"), (256.0, "256ms")],
            // gain=1.0 → attack_ms / release_ms (substituted at runtime via runtime_anchors).
            // Geometric lerp (off_atk_rel uses runtime y_natural, see curve_config.rs).
            y_natural: 1.0,
            offset_fn: off_atk_rel,
            natural_at_max: false,
        },
        4 => CurveDisplayConfig {
            y_label: "dB", y_min: 0.0, y_max: 48.0, y_log: false,
            grid_lines: &[(6.0, "6dB"), (12.0, "12dB"), (24.0, "24dB"), (36.0, "36dB")],
            // gain=1.0 → 6 dB knee; off=+1 → g=8.0 → 48 dB; off=-1 → g=0.0 → 0 dB
            y_natural: 6.0,
            offset_fn: off_knee,
            natural_at_max: false,
        },
        5 => CurveDisplayConfig {
            y_label: "%", y_min: 0.0, y_max: 100.0, y_log: false,
            grid_lines: &[(25.0, "25%"), (50.0, "50%"), (75.0, "75%"), (100.0, "100%")],
            // gain=1.0 → 100% wet (already at y_max); off=-1 → g=0.0 → 0%
            y_natural: 100.0,
            offset_fn: off_mix,
            natural_at_max: true,
        },
        _ => default_config(),
    }
}

fn freeze_config(i: usize) -> CurveDisplayConfig {
    match i {
        // LENGTH: gain=1.0 → 500 ms (gain_to_display: gain*500, range 1–4000 ms)
        // Geometric lerp: v≥0 → 500*(4000/500)^v = 500*8^v; v<0 → 500*(500/1)^v = 500*500^v
        // factor positive = 8; factor negative = 500 (asymmetric since y_min=1 ≠ y_max/y_nat^2)
        0 => CurveDisplayConfig {
            y_label: "ms", y_min: 1.0, y_max: 4000.0, y_log: true,
            grid_lines: &[(10.0, "10ms"), (100.0, "100ms"), (1000.0, "1s"), (4000.0, "4s")],
            y_natural: 500.0,
            offset_fn: off_freeze_length,
            natural_at_max: false,
        },
        // THRESHOLD: same formula as dynamics threshold (off_thresh/off_freeze_thresh are identical)
        1 => CurveDisplayConfig {
            y_label: "dBFS", y_min: -160.0, y_max: 0.0, y_log: false,
            grid_lines: &[(-20.0, "-20"), (-60.0, "-60"), (-100.0, "-100"), (-140.0, "-140")],
            y_natural: -20.0,
            offset_fn: off_freeze_thresh,
            natural_at_max: false,
        },
        // PORTAMENTO: matches D-1b DSP range (`curve * 150 ms` clamped to 1..750 ms).
        // Asymmetric log-axis so the slider hits 1 ms at v=-1 and 750 ms at v=+1
        // from the 150 ms neutral; uses off_freeze_length (ratio-from-anchors)
        // because the simpler off_portamento (fixed 5× ratio both sides) would
        // floor at 30 ms instead of 1 ms.
        2 => CurveDisplayConfig {
            y_label: "ms", y_min: 1.0, y_max: 750.0, y_log: true,
            grid_lines: &[(5.0, "5ms"), (50.0, "50ms"), (200.0, "200ms"), (500.0, "500ms")],
            y_natural: 150.0,
            offset_fn: off_freeze_length,
            natural_at_max: false,
        },
        // RESISTANCE: gain=1.0 → 1.0 (dimensionless); linear 0–2; additive, pos_span=1.0, neg_span=1.0
        3 => CurveDisplayConfig {
            y_label: "", y_min: 0.0, y_max: 2.0, y_log: false,
            grid_lines: &[(0.5, "0.5"), (1.0, "1.0"), (1.5, "1.5"), (2.0, "2.0")],
            y_natural: 1.0,
            offset_fn: off_resistance,
            natural_at_max: false,
        },
        // MIX: same as dynamics mix
        4 => CurveDisplayConfig {
            y_label: "%", y_min: 0.0, y_max: 100.0, y_log: false,
            grid_lines: &[(20.0, "20%"), (40.0, "40%"), (60.0, "60%"), (80.0, "80%")],
            y_natural: 100.0,
            offset_fn: off_mix,
            natural_at_max: true,
        },
        _ => default_config(),
    }
}

fn phase_smear_config(i: usize) -> CurveDisplayConfig {
    match i {
        // AMOUNT: gain=1.0 → 100%; gain_to_display: gain*100, range 0–200%
        // Additive: pos_span=1.0 (to reach gain=2.0 → 200%), neg_span=1.0 (to reach 0%)
        0 => CurveDisplayConfig {
            y_label: "%", y_min: 0.0, y_max: 200.0, y_log: false,
            grid_lines: &[(50.0, "50%"), (100.0, "100%"), (150.0, "150%"), (200.0, "200%")],
            y_natural: 100.0,
            offset_fn: off_amount_200,
            natural_at_max: false,
        },
        // PEAK HOLD: driven by shared `peak_hold_curve_to_ms` helper (log-piecewise,
        // clamps curve input to 0..=2 and maps to 1..50..500 ms). With
        // off_portamento(g, o) = g * 5^o, offset=-1 yields input 0.2 → ≈2.19 ms;
        // offset=0 yields 50 ms; offset=+1 yields input 5.0 (clamped to 2) → 500 ms.
        // y_natural stays at 50 ms (the helper's neutral output for gain=1.0).
        // The helper is shared with Gain's PEAK HOLD (T5's territory) — narrowing
        // the config here matches the real DSP range without touching the helper.
        1 => CurveDisplayConfig {
            y_label: "ms", y_min: 2.0, y_max: 500.0, y_log: true,
            grid_lines: &[(5.0, "5ms"), (50.0, "50ms"), (200.0, "200ms"), (500.0, "500ms")],
            y_natural: 50.0,
            offset_fn: off_portamento,
            natural_at_max: false,
        },
        // MIX: same as dynamics mix
        2 => CurveDisplayConfig {
            y_label: "%", y_min: 0.0, y_max: 100.0, y_log: false,
            grid_lines: &[(20.0, "20%"), (40.0, "40%"), (60.0, "60%"), (80.0, "80%")],
            y_natural: 100.0,
            offset_fn: off_mix,
            natural_at_max: true,
        },
        _ => default_config(),
    }
}

fn contrast_config(i: usize) -> CurveDisplayConfig {
    // 2026-05-08: 6-curve layout (THRESHOLD, RATIO, ATTACK, RELEASE, KNEE,
    // MIX) mirrors Dynamics so prototyping shares calibration.
    match i {
        // 0 THRESHOLD: dBFS, same anchors as Dynamics threshold.
        0 => CurveDisplayConfig {
            y_label: "dBFS", y_min: -160.0, y_max: 0.0, y_log: false,
            grid_lines: &[(-20.0, "-20"), (-60.0, "-60"), (-100.0, "-100"), (-140.0, "-140")],
            y_natural: -20.0,
            offset_fn: off_thresh,
            natural_at_max: false,
        },
        // 1 RATIO 1..20.
        1 => CurveDisplayConfig {
            y_label: "ratio", y_min: 1.0, y_max: 20.0, y_log: true,
            grid_lines: &[(1.5, "1:1.5"), (2.5, "1:2.5"), (5.0, "1:5"), (10.0, "1:10")],
            y_natural: 1.0,
            offset_fn: off_ratio,
            natural_at_max: false,
        },
        // 2 / 3 ATTACK / RELEASE — runtime y_natural substituted from the
        // global Atk/Rel knobs via runtime_anchors.
        2 | 3 => CurveDisplayConfig {
            y_label: "ms", y_min: 1.0, y_max: 1024.0, y_log: true,
            grid_lines: &[(4.0, "4ms"), (16.0, "16ms"), (64.0, "64ms"), (256.0, "256ms")],
            y_natural: 1.0,
            offset_fn: off_atk_rel,
            natural_at_max: false,
        },
        // 4 KNEE dB.
        4 => CurveDisplayConfig {
            y_label: "dB", y_min: 0.0, y_max: 48.0, y_log: false,
            grid_lines: &[(6.0, "6dB"), (12.0, "12dB"), (24.0, "24dB"), (36.0, "36dB")],
            y_natural: 6.0,
            offset_fn: off_knee,
            natural_at_max: false,
        },
        // 5 MIX %.
        5 => CurveDisplayConfig {
            y_label: "%", y_min: 0.0, y_max: 100.0, y_log: false,
            grid_lines: &[(25.0, "25%"), (50.0, "50%"), (75.0, "75%"), (100.0, "100%")],
            y_natural: 100.0,
            offset_fn: off_mix,
            natural_at_max: true,
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
                // gain=1.0 → 100% dry (wet/dry at y_max; off=-1 pulls it to 0%)
                y_natural: 100.0,
                offset_fn: off_gain_pct,
                natural_at_max: true,
            },
            _ => CurveDisplayConfig {
                y_label: "dB", y_min: -18.0, y_max: 18.0, y_log: false,
                grid_lines: &[(-12.0, "-12dB"), (-6.0, "-6dB"), (6.0, "+6dB"), (12.0, "+12dB")],
                // gain=1.0 → 0 dB; multiplicative with factor 7.943 (10^(18/20))
                y_natural: 0.0,
                offset_fn: off_gain_db,
                natural_at_max: false,
            },
        },
        // PEAK HOLD: same as phase smear peak hold — driven by shared
        // `peak_hold_curve_to_ms` helper (log-piecewise, clamps curve input to
        // 0..=2 and maps to 1..50..500 ms). With off_portamento(g, o) = g * 5^o,
        // offset=-1 yields input 0.2 → ≈2.19 ms; offset=0 yields 50 ms;
        // offset=+1 yields input 5.0 (clamped to 2) → 500 ms. The config range
        // matches the real DSP output range.
        1 => CurveDisplayConfig {
            y_label: "ms", y_min: 2.0, y_max: 500.0, y_log: true,
            grid_lines: &[(5.0, "5ms"), (50.0, "50ms"), (200.0, "200ms"), (500.0, "500ms")],
            y_natural: 50.0,
            offset_fn: off_portamento,
            natural_at_max: false,
        },
        _ => default_config(),
    }
}

fn mid_side_config(i: usize) -> CurveDisplayConfig {
    match i {
        // BALANCE / EXPANSION: gain=1.0 → 100% (neutral); 0–200% additive
        0 | 1 => CurveDisplayConfig {
            y_label: "%", y_min: 0.0, y_max: 200.0, y_log: false,
            grid_lines: &[(50.0, "50%"), (100.0, "100%"), (150.0, "150%"), (200.0, "200%")],
            y_natural: 100.0,
            offset_fn: off_amount_200,
            natural_at_max: false,
        },
        // DECORREL / TRANSIENT / PAN: gain=1.0 → 100% (at y_max); off=-1 → 0%
        _ => CurveDisplayConfig {
            y_label: "%", y_min: 0.0, y_max: 100.0, y_log: false,
            grid_lines: &[(20.0, "20%"), (40.0, "40%"), (60.0, "60%"), (80.0, "80%")],
            y_natural: 100.0,
            offset_fn: off_mix,
            natural_at_max: true,
        },
    }
}

fn ts_split_config(i: usize) -> CurveDisplayConfig {
    match i {
        // SENSITIVITY: gain=1.0 → 100%; at y_max; off=-1 → 0%
        0 => CurveDisplayConfig {
            y_label: "%", y_min: 0.0, y_max: 100.0, y_log: false,
            grid_lines: &[(20.0, "20%"), (40.0, "40%"), (60.0, "60%"), (80.0, "80%")],
            y_natural: 100.0,
            offset_fn: off_mix,
            natural_at_max: true,
        },
        // SMOOTHNESS (2026-05-08): per-bin envelope-follower coefficient.
        // Curve gain 0..2, neutral 1.0 → slow_coeff 0.98 (the historical
        // hardcoded value). Uses the 0..2 dimensionless axis.
        1 => CurveDisplayConfig {
            y_label: "", y_min: 0.0, y_max: 2.0, y_log: false,
            grid_lines: &[(0.5, "0.5"), (1.0, "1.0"), (1.5, "1.5"), (2.0, "2.0")],
            y_natural: 1.0,
            offset_fn: off_resistance,
            natural_at_max: false,
        },
        _ => default_config(),
    }
}

fn geometry_config(i: usize) -> CurveDisplayConfig {
    match i {
        0 => CurveDisplayConfig {
            y_label: "%", y_min: 0.0, y_max: 100.0, y_log: false,
            grid_lines: &[(25.0, "25%"), (50.0, "50%"), (75.0, "75%"), (100.0, "100%")],
            y_natural: 100.0,
            offset_fn: off_mix,
            natural_at_max: true,
        },
        1 => CurveDisplayConfig {
            y_label: "%", y_min: 0.0, y_max: 200.0, y_log: false,
            grid_lines: &[(50.0, "50%"), (100.0, "100%"), (150.0, "150%"), (200.0, "200%")],
            y_natural: 100.0,
            offset_fn: off_amount_200,
            natural_at_max: false,
        },
        2 => CurveDisplayConfig {
            y_label: "%", y_min: 0.0, y_max: 100.0, y_log: false,
            grid_lines: &[(25.0, "25%"), (50.0, "50%"), (75.0, "75%"), (100.0, "100%")],
            y_natural: 100.0,
            offset_fn: off_mix,
            natural_at_max: true,
        },
        3 => CurveDisplayConfig {
            y_label: "%", y_min: 0.0, y_max: 200.0, y_log: false,
            grid_lines: &[(50.0, "50%"), (100.0, "100%"), (150.0, "150%"), (200.0, "200%")],
            y_natural: 100.0,
            offset_fn: off_amount_200,
            natural_at_max: false,
        },
        4 => CurveDisplayConfig {
            y_label: "%", y_min: 0.0, y_max: 100.0, y_log: false,
            grid_lines: &[(25.0, "25%"), (50.0, "50%"), (75.0, "75%"), (100.0, "100%")],
            y_natural: 100.0,
            offset_fn: off_mix,
            natural_at_max: true,
        },
        _ => default_config(),
    }
}

fn circuit_config(i: usize) -> CurveDisplayConfig {
    match i {
        // AMOUNT: effect depth 0–100 %
        0 | 2 | 4 => CurveDisplayConfig {
            y_label: "%", y_min: 0.0, y_max: 100.0, y_log: false,
            grid_lines: &[(25.0, "25%"), (50.0, "50%"), (75.0, "75%"), (100.0, "100%")],
            y_natural: 100.0,
            offset_fn: off_mix,
            natural_at_max: true,
        },
        // THRESH: normalised trigger level 0–100 % (gain=1.0 → max threshold → no trigger)
        1 => CurveDisplayConfig {
            y_label: "%", y_min: 0.0, y_max: 100.0, y_log: false,
            grid_lines: &[(25.0, "25%"), (50.0, "50%"), (75.0, "75%"), (100.0, "100%")],
            y_natural: 100.0,
            offset_fn: off_mix,
            natural_at_max: true,
        },
        // RELEASE: dimensionless time-constant scalar 0–2, neutral = 1.0
        3 => CurveDisplayConfig {
            y_label: "", y_min: 0.0, y_max: 2.0, y_log: false,
            grid_lines: &[(0.5, "0.5"), (1.0, "1.0"), (1.5, "1.5"), (2.0, "2.0")],
            y_natural: 1.0,
            offset_fn: off_resistance,
            natural_at_max: false,
        },
        _ => default_config(),
    }
}

fn life_config(i: usize) -> CurveDisplayConfig {
    // AMOUNT, THRESHOLD, SPEED, REACH, MIX: all 0–100 % (gain=1.0 → 100%)
    match i {
        0 | 1 | 2 | 3 | 4 => CurveDisplayConfig {
            y_label: "%", y_min: 0.0, y_max: 100.0, y_log: false,
            grid_lines: &[(25.0, "25%"), (50.0, "50%"), (75.0, "75%"), (100.0, "100%")],
            y_natural: 100.0,
            offset_fn: off_mix,
            natural_at_max: true,
        },
        _ => default_config(),
    }
}

fn kinetics_config(i: usize) -> CurveDisplayConfig {
    match i {
        // STRENGTH, MASS, REACH, DAMPING: bidirectional 0–200 %, neutral = 100 %
        0 | 1 | 2 | 3 => CurveDisplayConfig {
            y_label: "%", y_min: 0.0, y_max: 200.0, y_log: false,
            grid_lines: &[(50.0, "50%"), (100.0, "100%"), (150.0, "150%"), (200.0, "200%")],
            y_natural: 100.0,
            offset_fn: off_amount_200,
            natural_at_max: false,
        },
        // MIX: 0–100 %
        4 => CurveDisplayConfig {
            y_label: "%", y_min: 0.0, y_max: 100.0, y_log: false,
            grid_lines: &[(25.0, "25%"), (50.0, "50%"), (75.0, "75%"), (100.0, "100%")],
            y_natural: 100.0,
            offset_fn: off_mix,
            natural_at_max: true,
        },
        _ => default_config(),
    }
}

fn harmony_config(i: usize) -> CurveDisplayConfig {
    match i {
        // THRESHOLD: bidirectional 0–100 % with neutral at 50 %, so the
        // offset slider can sweep the full range from either direction
        // (the prior natural-at-max config left the user "stuck at 100 %"
        // — see 2026-05-08 bug list).
        1 => CurveDisplayConfig {
            y_label: "%", y_min: 0.0, y_max: 100.0, y_log: false,
            grid_lines: &[(25.0, "25%"), (50.0, "50%"), (75.0, "75%"), (100.0, "100%")],
            y_natural: 50.0,
            offset_fn: off_threshold_pct,
            natural_at_max: false,
        },
        // AMOUNT, STABILITY, SPREAD, MIX: 0–100 %, natural-at-max
        0 | 2 | 3 | 5 => CurveDisplayConfig {
            y_label: "%", y_min: 0.0, y_max: 100.0, y_log: false,
            grid_lines: &[(25.0, "25%"), (50.0, "50%"), (75.0, "75%"), (100.0, "100%")],
            y_natural: 100.0,
            offset_fn: off_mix,
            natural_at_max: true,
        },
        // COEFFICIENT: mode-specific weighting 0–200 %, neutral = 100 %
        4 => CurveDisplayConfig {
            y_label: "%", y_min: 0.0, y_max: 200.0, y_log: false,
            grid_lines: &[(50.0, "50%"), (100.0, "100%"), (150.0, "150%"), (200.0, "200%")],
            y_natural: 100.0,
            offset_fn: off_amount_200,
            natural_at_max: false,
        },
        _ => default_config(),
    }
}

fn modulate_config(i: usize) -> CurveDisplayConfig {
    // AMOUNT, REACH, RATE, THRESH, AMPGATE, MIX: all 0–100 % (gain=1.0 → 100%)
    match i {
        0 | 1 | 2 | 3 | 4 | 5 => CurveDisplayConfig {
            y_label: "%", y_min: 0.0, y_max: 100.0, y_log: false,
            grid_lines: &[(25.0, "25%"), (50.0, "50%"), (75.0, "75%"), (100.0, "100%")],
            y_natural: 100.0,
            offset_fn: off_mix,
            natural_at_max: true,
        },
        _ => default_config(),
    }
}

fn rhythm_config(i: usize) -> CurveDisplayConfig {
    match i {
        // AMOUNT, ATTACK_FADE, TARGET_PHASE, MIX: 0–100 %
        0 | 2 | 3 | 4 => CurveDisplayConfig {
            y_label: "%", y_min: 0.0, y_max: 100.0, y_log: false,
            grid_lines: &[(25.0, "25%"), (50.0, "50%"), (75.0, "75%"), (100.0, "100%")],
            y_natural: 100.0,
            offset_fn: off_mix,
            natural_at_max: true,
        },
        // DIVISION: step-count scalar 0–200 % (gain=1.0 → 100% → 16 steps base)
        1 => CurveDisplayConfig {
            y_label: "%", y_min: 0.0, y_max: 200.0, y_log: false,
            grid_lines: &[(50.0, "50%"), (100.0, "100%"), (150.0, "150%"), (200.0, "200%")],
            y_natural: 100.0,
            offset_fn: off_amount_200,
            natural_at_max: false,
        },
        _ => default_config(),
    }
}

fn future_config(i: usize) -> CurveDisplayConfig {
    match i {
        // AMOUNT, THRESHOLD, SPREAD, MIX: 0–100 %
        0 | 2 | 3 | 4 => CurveDisplayConfig {
            y_label: "%", y_min: 0.0, y_max: 100.0, y_log: false,
            grid_lines: &[(25.0, "25%"), (50.0, "50%"), (75.0, "75%"), (100.0, "100%")],
            y_natural: 100.0,
            offset_fn: off_mix,
            natural_at_max: true,
        },
        // TIME: lookahead scaling 0–200 % (gain=1.0 → 100% → 1 FFT-hop of lookahead)
        1 => CurveDisplayConfig {
            y_label: "%", y_min: 0.0, y_max: 200.0, y_log: false,
            grid_lines: &[(50.0, "50%"), (100.0, "100%"), (150.0, "150%"), (200.0, "200%")],
            y_natural: 100.0,
            offset_fn: off_amount_200,
            natural_at_max: false,
        },
        _ => default_config(),
    }
}

fn punch_config(i: usize) -> CurveDisplayConfig {
    match i {
        // AMOUNT, FILL_MODE, MIX: 0–100 %
        0 | 2 | 5 => CurveDisplayConfig {
            y_label: "%", y_min: 0.0, y_max: 100.0, y_log: false,
            grid_lines: &[(25.0, "25%"), (50.0, "50%"), (75.0, "75%"), (100.0, "100%")],
            y_natural: 100.0,
            offset_fn: off_mix,
            natural_at_max: true,
        },
        // WIDTH, AMP_FILL: 0–200 %, neutral = 100 %
        1 | 3 => CurveDisplayConfig {
            y_label: "%", y_min: 0.0, y_max: 200.0, y_log: false,
            grid_lines: &[(50.0, "50%"), (100.0, "100%"), (150.0, "150%"), (200.0, "200%")],
            y_natural: 100.0,
            offset_fn: off_amount_200,
            natural_at_max: false,
        },
        // HEAL: release time displayed as ms via portamento scale
        // display_curve_idx=10: gain_to_display = gain*200 ms; physical_to_y: log 40–1000 ms
        // DSP formula: heal_ms = gain*150 (clamped 20–300 ms for gain 0.05–2.0)
        4 => CurveDisplayConfig {
            y_label: "ms", y_min: 40.0, y_max: 1000.0, y_log: true,
            grid_lines: &[(40.0, "40ms"), (100.0, "100ms"), (250.0, "250ms"), (1000.0, "1s")],
            y_natural: 200.0,
            offset_fn: off_portamento,
            natural_at_max: false,
        },
        _ => default_config(),
    }
}

/// Per-curve display calibration for `ModuleType::Past`.
///
/// `mode` is currently unused at this level — the per-mode label overrides
/// (Age vs Delay) live in `past::active_layout` (curve_layout::label_overrides).
/// `past_config` produces axis units, ranges, grid lines, and offset_fn for the
/// **physical** display layer; per-mode label changes happen above it.
///
/// See docs/superpowers/specs/2026-05-04-past-module-ux-design.md §5.
pub fn past_config(curve_idx: usize, _mode: u8) -> CurveDisplayConfig {
    match curve_idx {
        0 => CurveDisplayConfig {
            y_label: "%", y_min: 0.0, y_max: 100.0, y_log: false,
            grid_lines: &[(25.0, "25%"), (50.0, "50%"), (75.0, "75%"), (100.0, "100%")],
            y_natural: 100.0,
            offset_fn: off_mix,
            natural_at_max: true,
        },
        1 => CurveDisplayConfig {
            // Display index 13 (Past Age/Delay) treats these anchors as
            // fractions of `total_history_seconds` — the runtime substitution
            // happens in `runtime_anchors()` and `gain_to_display(13, ...)`.
            // y_natural at 0.5 puts the offset slider's neutral at the
            // midpoint of the buffer so the slider has equal headroom in
            // both directions (spec §2 piecewise lerp).
            y_label: "s", y_min: 0.0, y_max: 1.0, y_log: false,
            grid_lines: &[(0.25, "25%"), (0.5, "50%"), (0.75, "75%"), (1.0, "100%")],
            y_natural: 0.5,
            offset_fn: off_amount_norm,
            natural_at_max: false,
        },
        2 => CurveDisplayConfig {
            y_label: "dBFS", y_min: -160.0, y_max: 0.0, y_log: false,
            grid_lines: &[(-20.0, "-20"), (-60.0, "-60"), (-100.0, "-100"), (-140.0, "-140")],
            y_natural: -20.0,
            offset_fn: off_freeze_thresh,
            natural_at_max: false,
        },
        3 => CurveDisplayConfig {
            y_label: "%", y_min: 0.0, y_max: 100.0, y_log: false,
            grid_lines: &[(25.0, "25%"), (50.0, "50%"), (75.0, "75%"), (100.0, "100%")],
            y_natural: 100.0,
            offset_fn: off_mix,
            natural_at_max: true,
        },
        4 => CurveDisplayConfig {
            y_label: "%", y_min: 0.0, y_max: 100.0, y_log: false,
            grid_lines: &[(25.0, "25%"), (50.0, "50%"), (75.0, "75%"), (100.0, "100%")],
            y_natural: 100.0,
            offset_fn: off_mix,
            natural_at_max: true,
        },
        _ => default_config(),
    }
}

fn default_config() -> CurveDisplayConfig {
    // off_identity: offset has no audible/visual effect for un-calibrated
    // modules. Asymmetric alternatives (e.g. off_mix) introduce a "stops past
    // 0" perception bug because positive offset is a no-op when y_natural is
    // already at y_max. Per-module configs (past_config, geometry_config, …)
    // override this with calibrated offset_fn / y_label / grid_lines per the
    // UI parameter spec. The slider's custom_formatter detects y_label==""
    // and falls back to showing the raw [-1, 1] value so the drag is still
    // visible during a UI rebuild even though offset is inert here.
    CurveDisplayConfig {
        y_label: "", y_min: 0.0, y_max: 1.0, y_log: false,
        grid_lines: &[(0.25, ""), (0.5, ""), (0.75, ""), (1.0, "")],
        // y_natural intentionally not at y_max (0.5 ≠ 1.0) so natural_at_max=false
        // is mathematically consistent. The identity offset_fn makes this inert for
        // uncalibrated curves.
        y_natural: 0.5,
        offset_fn: off_identity,
        natural_at_max: false,
    }
}

// ── Per-calibration offset functions ─────────────────────────────────────────
//
// All satisfy: fn(g, 0.0) == g.
// All are plain fn pointers — no captures, safe on audio thread.
//
// See docs/superpowers/specs/2026-04-23-ui-parameter-spec-design.md §2.

/// Linear add, clamped to [0, 1]. For curves whose gain is interpreted as a
/// normalised fraction (e.g. Past's Age/Delay representing a fraction of the
/// history buffer's `capacity_frames`).
/// See docs/superpowers/specs/2026-04-23-ui-parameter-spec-design.md §7 and
/// docs/superpowers/specs/2026-05-04-past-module-ux-design.md §5.
#[inline] pub fn off_amount_norm(g: f32, o: f32, _anchors: (f32, f32, f32)) -> f32 {
    (g + o).clamp(0.0, 1.0)
}

/// Dynamics / Freeze THRESHOLD dBFS: multiplicative log-dBFS, calibrated so
/// the offset slider spans the full y_min=-160..y_max=0 axis from v=±1
/// when the curve gain is at its neutral 1.0.
///
/// `curve_to_threshold_db` (see `dsp::modules::freeze::curve_to_threshold_db`)
/// maps curve_gain g via t_db = 20·log10(g) and then:
///   - positive: display = -20 + (20/18)·t_db, so display=0 needs t_db=18.
///   - negative: display = -20 + (140/18)·t_db, so display=-160 needs t_db=-18.
/// Both ends require the SAME |t_db| = 18 (asymmetric only in display
/// scaling, not in curve-gain space), so a single symmetric multiplier
/// 10^(0.9·v) reaches y_max at v=+1 and y_min at v=-1.
///
/// Previous calibration used 0.3·v / 0.6·v which only reached display = -13
/// at v=+1 and -113 at v=-1 — leaving most of the axis unreachable from
/// the slider. Fixed 2026-05-07 (D-1c).
#[inline] pub fn off_thresh(g: f32, o: f32, _anchors: (f32, f32, f32)) -> f32 {
    g * 10f32.powf(0.9 * o)
}

/// Ratio 1–20: WYSIWYG with log axis (spec §2 axis-aware lerp).
/// Geometric lerp from y_natural=1 to y_max=20 on the positive side.
/// Negative side: y_min == y_natural == 1, so the slider has no negative
/// reach (ratio cannot go below 1:1) — return g unchanged.
///   v ≥ 0:  factor = (y_max / y_nat)^v = 20^v
///   v < 0:  factor = 1 (no-op)
#[inline] pub fn off_ratio(g: f32, o: f32, _anchors: (f32, f32, f32)) -> f32 {
    if o >= 0.0 { g * 20.0_f32.powf(o) } else { g }
}

/// Attack/Release ms: geometric lerp from y_natural (=runtime attack_ms or
/// release_ms after substitution by runtime_anchors) to y_min/y_max.
///   v ≥ 0:  factor = (y_max  / y_nat)^v
///   v < 0:  factor = (y_nat  / y_min)^v
/// gain_to_display(2, g, attack_ms) = attack_ms · g, so:
///   gain_off = phys / y_nat = factor (since phys = y_nat · factor).
#[inline] pub fn off_atk_rel(g: f32, o: f32, anchors: (f32, f32, f32)) -> f32 {
    let (y_min, y_nat, y_max) = anchors;
    let factor = if o >= 0.0 { (y_max / y_nat).powf(o) }
                 else        { (y_nat / y_min).powf(o) };
    g * factor
}

/// Knee dB: gain=1.0 → 6 dB knee (neutral).
/// off=+1 → g=8.0 → 48 dB; off=-1 → g=0.0 → 0 dB.
#[inline] pub fn off_knee(g: f32, o: f32, _anchors: (f32, f32, f32)) -> f32 {
    if o >= 0.0 { g + 7.0 * o } else { g + o }
}

/// Mix %: gain=1.0 → 100% (at y_max); off=-1 → g=0.0 → 0%.
#[inline] pub fn off_mix(g: f32, o: f32, _anchors: (f32, f32, f32)) -> f32 {
    if o >= 0.0 { g } else { g + o }
}

/// Gain dB (Add/Subtract): multiplicative with factor 10^(18/20) ≈ 7.9433.
/// off=+1 → g×7.9433 → +18 dB; off=-1 → g/7.9433 → -18 dB.
#[inline] pub fn off_gain_db(g: f32, o: f32, _anchors: (f32, f32, f32)) -> f32 {
    g * 7.943_282_f32.powf(o)
}

/// Gain Pull/Match (%): gain=1.0 → 100% dry (at y_max); off=-1 → 0%.
#[inline] pub fn off_gain_pct(g: f32, o: f32, _anchors: (f32, f32, f32)) -> f32 {
    if o >= 0.0 { g } else { g + o }
}

/// Amount 0–200%: gain=1.0 → 100% (neutral); pos_span=1.0, neg_span=1.0.
/// off=+1 → g=2.0 → 200%; off=-1 → g=0.0 → 0%.
#[inline] pub fn off_amount_200(g: f32, o: f32, _anchors: (f32, f32, f32)) -> f32 {
    g + o
}

/// Freeze LENGTH ms: geometric lerp from y_natural to y_min/y_max.
///   v ≥ 0:  factor = (y_max / y_nat)^v  (e.g. 8^v at canonical anchors)
///   v < 0:  factor = (y_nat / y_min)^v  (e.g. 500^v at canonical anchors)
#[inline] pub fn off_freeze_length(g: f32, o: f32, anchors: (f32, f32, f32)) -> f32 {
    let (y_min, y_nat, y_max) = anchors;
    let factor = if o >= 0.0 { (y_max / y_nat).powf(o) }
                 else        { (y_nat / y_min).powf(o) };
    g * factor
}

/// Freeze/Past THRESHOLD dBFS: multiplicative log-dBFS, range -80..0, neutral -20.
///   v ≥ 0:  display = -20 + 20·v  → factor 10^(0.3·v)
///   v < 0:  display = -20 + 60·v  → factor 10^(0.9·v)
#[inline] pub fn off_freeze_thresh(g: f32, o: f32, _anchors: (f32, f32, f32)) -> f32 {
    if o >= 0.0 { g * 10f32.powf(0.3 * o) } else { g * 10f32.powf(0.9 * o) }
}

/// Portamento/SC-smooth ms: multiplicative, factor = 1000/200 = 5.0.
/// gain=1.0 → 200 ms; off=+1 → g×5 → 1000 ms; off=-1 → g/5 → ~40 ms.
#[inline] pub fn off_portamento(g: f32, o: f32, _anchors: (f32, f32, f32)) -> f32 {
    g * 5.0_f32.powf(o)
}

/// Resistance 0–2: gain=1.0 → 1.0 (neutral); additive, pos_span=1.0, neg_span=1.0.
/// off=+1 → g=2.0 → 2.0; off=-1 → g=0.0 → 0.0.
#[inline] pub fn off_resistance(g: f32, o: f32, _anchors: (f32, f32, f32)) -> f32 {
    g + o
}

/// Threshold % with neutral at 50 % on a 0..100 % axis. Pairs with
/// `gain_to_display` idx 6 (`gain * 100`). Maps:
///   gain=1.0, o=-1 → 0.0  → display 0 %
///   gain=1.0, o= 0 → 0.5  → display 50 %
///   gain=1.0, o=+1 → 1.0  → display 100 %
/// Curve nodes still scale via `g * 0.5`, so a node dragged to its top
/// (gain=2.0) maps to display 100 % with the offset slider at 0.
#[inline] pub fn off_threshold_pct(g: f32, o: f32, _anchors: (f32, f32, f32)) -> f32 {
    (g * 0.5 + o * 0.5).max(0.0)
}

/// Identity: no offset effect. Used for curves with unclear calibration and default_config.
#[inline] pub fn off_identity(g: f32, _o: f32, _anchors: (f32, f32, f32)) -> f32 {
    g
}
