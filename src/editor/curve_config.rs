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
    pub offset_fn:  fn(f32, f32) -> f32,
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
        // TODO(2e.3): replace with geometry_config(curve_idx) when that task is implemented
        ModuleType::Geometry => default_config(),
        // TODO(2f.3-2f.7): replace with modulate_config(curve_idx) when kernels land
        ModuleType::Modulate => modulate_config(curve_idx),
        // TODO(2g.3-2g.5): replace with circuit_config(curve_idx) when kernels land
        ModuleType::Circuit => default_config(),
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
            // gain=1.0 → -20 dBFS (neutral threshold)
            // off=+1 → g=2.0 → display ≈ 0 dBFS; off=-1 → g=-1.0 → display ≈ -60 dBFS
            y_natural: -20.0,
            offset_fn: off_thresh,
        },
        1 => CurveDisplayConfig {
            y_label: "ratio", y_min: 1.0, y_max: 20.0, y_log: true,
            grid_lines: &[(1.5, "1:1.5"), (2.5, "1:2.5"), (5.0, "1:5"), (10.0, "1:10")],
            // gain=1.0 → ratio 1:1 (no compression); off=+1 → g=20.0 → ratio 20:1
            y_natural: 1.0,
            offset_fn: off_ratio,
        },
        2 | 3 => CurveDisplayConfig {
            y_label: "ms", y_min: 1.0, y_max: 1024.0, y_log: true,
            grid_lines: &[(4.0, "4ms"), (16.0, "16ms"), (64.0, "64ms"), (256.0, "256ms")],
            // gain=1.0 → global_ms × 1; multiplicative: off=+1 → g×1024, off=-1 → g/1024
            y_natural: 1.0,
            offset_fn: off_atk_rel,
        },
        4 => CurveDisplayConfig {
            y_label: "dB", y_min: 0.0, y_max: 48.0, y_log: false,
            grid_lines: &[(6.0, "6dB"), (12.0, "12dB"), (24.0, "24dB"), (36.0, "36dB")],
            // gain=1.0 → 6 dB knee; off=+1 → g=8.0 → 48 dB; off=-1 → g=0.0 → 0 dB
            y_natural: 6.0,
            offset_fn: off_knee,
        },
        5 => CurveDisplayConfig {
            y_label: "%", y_min: 0.0, y_max: 100.0, y_log: false,
            grid_lines: &[(25.0, "25%"), (50.0, "50%"), (75.0, "75%"), (100.0, "100%")],
            // gain=1.0 → 100% wet (already at y_max); off=-1 → g=0.0 → 0%
            y_natural: 100.0,
            offset_fn: off_mix,
        },
        _ => default_config(),
    }
}

fn freeze_config(i: usize) -> CurveDisplayConfig {
    match i {
        // LENGTH: gain=1.0 → 500 ms (gain_to_display: gain*500, range 0–4000 ms)
        // Multiplicative: off=+1 → gain*8 → 4000 ms; off=-1 → gain/8 → 62.5 ms
        // factor = 4000/500 = 8.0; y_min matches off_freeze_length(1.0, -1.0) * 500
        0 => CurveDisplayConfig {
            y_label: "ms", y_min: 62.5, y_max: 4000.0, y_log: true,
            grid_lines: &[(100.0, "100ms"), (500.0, "500ms"), (1000.0, "1s"), (2000.0, "2s")],
            y_natural: 500.0,
            offset_fn: off_freeze_length,
        },
        // THRESHOLD: same formula as dynamics threshold
        // gain=1.0 → -20 dBFS; off=+1 → g=2.0 → 0 dBFS; off=-1 → g=-1.0 → -80 dBFS
        // pos_span=+1.0 (from 1.0 up to 2.0), neg_span_abs=4.0 (from 1.0 down to -3.0 → maps to -80 dBFS)
        1 => CurveDisplayConfig {
            y_label: "dBFS", y_min: -80.0, y_max: 0.0, y_log: false,
            grid_lines: &[(-12.0, "-12"), (-40.0, "-40"), (-60.0, "-60"), (-80.0, "-80")],
            y_natural: -20.0,
            offset_fn: off_freeze_thresh,
        },
        // PORTAMENTO: gain=1.0 → 200 ms; multiplicative with factor = 1000/200 = 5.0
        // y_min matches off_portamento(1.0, -1.0) * 200 = 40 ms
        2 => CurveDisplayConfig {
            y_label: "ms", y_min: 40.0, y_max: 1000.0, y_log: true,
            grid_lines: &[(40.0, "40ms"), (100.0, "100ms"), (500.0, "500ms"), (1000.0, "1s")],
            y_natural: 200.0,
            offset_fn: off_portamento,
        },
        // RESISTANCE: gain=1.0 → 1.0 (dimensionless); linear 0–2; additive, pos_span=1.0, neg_span=1.0
        3 => CurveDisplayConfig {
            y_label: "", y_min: 0.0, y_max: 2.0, y_log: false,
            grid_lines: &[(0.5, "0.5"), (1.0, "1.0"), (1.5, "1.5"), (2.0, "2.0")],
            y_natural: 1.0,
            offset_fn: off_resistance,
        },
        // MIX: same as dynamics mix
        4 => CurveDisplayConfig {
            y_label: "%", y_min: 0.0, y_max: 100.0, y_log: false,
            grid_lines: &[(20.0, "20%"), (40.0, "40%"), (60.0, "60%"), (80.0, "80%")],
            y_natural: 100.0,
            offset_fn: off_mix,
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
        },
        // MIX: same as dynamics mix
        2 => CurveDisplayConfig {
            y_label: "%", y_min: 0.0, y_max: 100.0, y_log: false,
            grid_lines: &[(20.0, "20%"), (40.0, "40%"), (60.0, "60%"), (80.0, "80%")],
            y_natural: 100.0,
            offset_fn: off_mix,
        },
        _ => default_config(),
    }
}

fn contrast_config(i: usize) -> CurveDisplayConfig {
    match i {
        // AMOUNT: gain maps directly to bp_ratio (ratio 1–20); log scale; same as dynamics ratio
        0 => CurveDisplayConfig {
            y_label: "ratio", y_min: 1.0, y_max: 20.0, y_log: true,
            grid_lines: &[(1.25, "1:1.25"), (2.5, "1:2.5"), (5.0, "1:5"), (10.0, "1:10")],
            y_natural: 1.0,
            offset_fn: off_ratio,
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
            },
            _ => CurveDisplayConfig {
                y_label: "dB", y_min: -18.0, y_max: 18.0, y_log: false,
                grid_lines: &[(-12.0, "-12dB"), (-6.0, "-6dB"), (6.0, "+6dB"), (12.0, "+12dB")],
                // gain=1.0 → 0 dB; multiplicative with factor 7.943 (10^(18/20))
                y_natural: 0.0,
                offset_fn: off_gain_db,
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
        },
        // DECORREL / TRANSIENT / PAN: gain=1.0 → 100% (at y_max); off=-1 → 0%
        _ => CurveDisplayConfig {
            y_label: "%", y_min: 0.0, y_max: 100.0, y_log: false,
            grid_lines: &[(20.0, "20%"), (40.0, "40%"), (60.0, "60%"), (80.0, "80%")],
            y_natural: 100.0,
            offset_fn: off_mix,
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
        },
        _ => default_config(),
    }
}

/// Curve config for Future module. Stub — populated as Print-Through (Task 3) and
/// Pre-Echo (Task 4) kernels land. All five curves currently fall through to
/// default_config() until physical-unit display ranges are decided.
fn future_config(_curve_idx: usize) -> CurveDisplayConfig {
    default_config()
}

/// Curve config for Punch module. Stub — populated as the Punch DSP lands in
/// Tasks 2c.2-2c.6. All six curves currently fall through to default_config()
/// until physical-unit display ranges are decided.
fn punch_config(_curve_idx: usize) -> CurveDisplayConfig {
    default_config()
}

/// Curve config for Rhythm module. Stub — populated as the Rhythm DSP lands in
/// Tasks 2d.2-2d.9. All five curves currently fall through to default_config()
/// until physical-unit display ranges are decided.
fn rhythm_config(_curve_idx: usize) -> CurveDisplayConfig {
    default_config()
}

/// Curve config for Modulate module. Stub — populated as the Modulate DSP lands
/// in Tasks 2f.3-2f.7. All six curves currently fall through to default_config()
/// until physical-unit display ranges are decided.
fn modulate_config(_curve_idx: usize) -> CurveDisplayConfig {
    default_config()
}

fn default_config() -> CurveDisplayConfig {
    CurveDisplayConfig {
        y_label: "", y_min: 0.0, y_max: 1.0, y_log: false,
        grid_lines: &[(0.25, ""), (0.5, ""), (0.75, ""), (1.0, "")],
        y_natural: 1.0,
        offset_fn: off_identity,
    }
}

// ── Per-calibration offset functions ─────────────────────────────────────────
//
// All satisfy: fn(g, 0.0) == g.
// All are plain fn pointers — no captures, safe on audio thread.
//
// See docs/superpowers/specs/2026-04-23-ui-parameter-spec-design.md §2.

/// Threshold dBFS: gain=1.0 → -20 dBFS.
/// off=+1 → g=2.0 → 0 dBFS;  off=-1 → g=-1.0 → -60 dBFS (clamped by audio path).
#[inline] pub fn off_thresh(g: f32, o: f32) -> f32 {
    if o >= 0.0 { g + o } else { g + 2.0 * o }
}

/// Ratio 1–20: gain=1.0 → ratio 1:1.
/// off=+1 → g=20.0 → ratio 20:1; off=-1 → clamped at y_min (ratio can't go below 1).
#[inline] pub fn off_ratio(g: f32, o: f32) -> f32 {
    if o >= 0.0 { g + 19.0 * o } else { g }
}

/// Attack/Release multiplier: multiplicative with factor 1024.
/// off=+1 → g×1024; off=-1 → g/1024.
#[inline] pub fn off_atk_rel(g: f32, o: f32) -> f32 {
    g * 1024.0_f32.powf(o)
}

/// Knee dB: gain=1.0 → 6 dB knee (neutral).
/// off=+1 → g=8.0 → 48 dB; off=-1 → g=0.0 → 0 dB.
#[inline] pub fn off_knee(g: f32, o: f32) -> f32 {
    if o >= 0.0 { g + 7.0 * o } else { g + o }
}

/// Mix %: gain=1.0 → 100% (at y_max); off=-1 → g=0.0 → 0%.
#[inline] pub fn off_mix(g: f32, o: f32) -> f32 {
    if o >= 0.0 { g } else { g + o }
}

/// Gain dB (Add/Subtract): multiplicative with factor 10^(18/20) ≈ 7.9433.
/// off=+1 → g×7.9433 → +18 dB; off=-1 → g/7.9433 → -18 dB.
#[inline] pub fn off_gain_db(g: f32, o: f32) -> f32 {
    g * 7.943_282_f32.powf(o)
}

/// Gain Pull/Match (%): gain=1.0 → 100% dry (at y_max); off=-1 → 0%.
#[inline] pub fn off_gain_pct(g: f32, o: f32) -> f32 {
    if o >= 0.0 { g } else { g + o }
}

/// Amount 0–200%: gain=1.0 → 100% (neutral); pos_span=1.0, neg_span=1.0.
/// off=+1 → g=2.0 → 200%; off=-1 → g=0.0 → 0%.
#[inline] pub fn off_amount_200(g: f32, o: f32) -> f32 {
    g + o
}

/// Freeze LENGTH: multiplicative, factor = 4000/500 = 8.0.
/// off=+1 → g×8 → 4000 ms; off=-1 → g/8 → ~62 ms.
#[inline] pub fn off_freeze_length(g: f32, o: f32) -> f32 {
    g * 8.0_f32.powf(o)
}

/// Freeze THRESHOLD dBFS: same formula as dynamics threshold but range is -80–0 dBFS.
/// gain=1.0 → -20 dBFS; off=+1 → g=2.0 → 0 dBFS; off=-1 → g=-3.0 → -80 dBFS (clamped).
/// neg_span_abs = 4.0 so gain goes from 1.0 to -3.0 (very negative → clamped to -80 dBFS).
#[inline] pub fn off_freeze_thresh(g: f32, o: f32) -> f32 {
    if o >= 0.0 { g + o } else { g + 4.0 * o }
}

/// Portamento/SC-smooth ms: multiplicative, factor = 1000/200 = 5.0.
/// gain=1.0 → 200 ms; off=+1 → g×5 → 1000 ms; off=-1 → g/5 → ~40 ms.
#[inline] pub fn off_portamento(g: f32, o: f32) -> f32 {
    g * 5.0_f32.powf(o)
}

/// Resistance 0–2: gain=1.0 → 1.0 (neutral); additive, pos_span=1.0, neg_span=1.0.
/// off=+1 → g=2.0 → 2.0; off=-1 → g=0.0 → 0.0.
#[inline] pub fn off_resistance(g: f32, o: f32) -> f32 {
    g + o
}

/// Identity: no offset effect. Used for curves with unclear calibration and default_config.
#[inline] pub fn off_identity(g: f32, _o: f32) -> f32 {
    g
}
