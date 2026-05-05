use serde::{Serialize, Deserialize};
use nih_plug_egui::egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, Ui, Vec2};
use crate::editor::theme as th;
use crate::editor::curve_config::CurveDisplayConfig;
use crate::dsp::modules::{GainMode, ModuleType};

/// Return the curve label to show for `(module_type, curve_idx)`, accounting for
/// gain-mode context. The Gain module's curve 0 is titled "GAIN" by default but
/// acts as a wet/dry MIX in Pull/Match modes; callers should display "MIX" there
/// so the tooltip unit (percentage) agrees with the axis label.
pub fn curve_label_for(
    module_type: ModuleType,
    curve_idx: usize,
    gain_mode: GainMode,
    default_label: &'static str,
) -> &'static str {
    if module_type == ModuleType::Gain
        && curve_idx == 0
        && matches!(gain_mode, GainMode::Pull | GainMode::Match)
    {
        "MIX"
    } else {
        default_label
    }
}

/// Paint a dashed polyline through `pts` with the given `stroke`.
/// `dash` and `gap` are in pixels.
fn paint_dashed_line(painter: &Painter, pts: &[Pos2], stroke: Stroke, dash: f32, gap: f32) {
    if pts.len() < 2 { return; }
    let cycle = dash + gap;
    let mut dist = 0.0_f32;
    let mut seg: Vec<Pos2> = Vec::new();

    for i in 1..pts.len() {
        let a = pts[i - 1];
        let b = pts[i];
        let step = (b - a).length();
        if step < 0.001 { continue; }
        let dir = (b - a) / step;
        let mut t = 0.0_f32;
        while t < step {
            let phase = dist % cycle;
            let in_dash = phase < dash;
            // Gap portion spans [dash, cycle), so remaining = cycle - phase (not gap - phase).
            // Using gap - phase would go negative whenever phase > gap, causing dist to
            // decrement and the loop to oscillate forever.
            let remaining_in_phase = if in_dash { dash - phase } else { cycle - phase };
            let end_t = (t + remaining_in_phase).min(step);
            let p0 = a + dir * t;
            let p1 = a + dir * end_t;
            if in_dash {
                if seg.is_empty() { seg.push(p0); }
                seg.push(p1);
            } else {
                if seg.len() >= 2 {
                    painter.add(Shape::line(std::mem::take(&mut seg), stroke));
                } else {
                    seg.clear();
                }
            }
            dist += end_t - t;
            t = end_t;
        }
    }
    if seg.len() >= 2 {
        painter.add(Shape::line(seg, stroke));
    }
}

// ─── Data types ───────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CurveNode {
    pub x: f32,  // [0.0, 1.0] normalised log-frequency (20 Hz at 0, 20 kHz at 1)
    pub y: f32,  // [-1.0, +1.0] gain: 0.0 = neutral
    pub q: f32,  // [0.0, 1.0] normalised octave-bandwidth
}

pub fn default_nodes() -> [CurveNode; 6] {
    [
        CurveNode { x: 0.0, y: 0.0, q: 0.3 },
        CurveNode { x: 0.2, y: 0.0, q: 0.5 },
        CurveNode { x: 0.4, y: 0.0, q: 0.5 },
        CurveNode { x: 0.6, y: 0.0, q: 0.5 },
        CurveNode { x: 0.8, y: 0.0, q: 0.5 },
        CurveNode { x: 1.0, y: 0.0, q: 0.3 },
    ]
}

/// Per-curve default nodes.  The ratio curve starts at approximately 1:2.
/// Low shelf is positioned at ~75 Hz (roll-off 50–100 Hz when adjusted).
/// High shelf at 20 Hz gives ≈ 2× gain across all audible frequencies.
pub fn default_nodes_for_curve(curve_idx: usize) -> [CurveNode; 6] {
    match curve_idx {
        1 /* RATIO */ => [
            // Low shelf at ~75 Hz (x = log10(75/20)/3 ≈ 0.19): positioned for 50–100 Hz
            // adjustment; currently neutral so all audible compression comes from node 5.
            CurveNode { x: 0.19, y: 0.0,   q: 0.3 },
            CurveNode { x: 0.2,  y: 0.0,   q: 0.5 },
            CurveNode { x: 0.4,  y: 0.0,   q: 0.5 },
            CurveNode { x: 0.6,  y: 0.0,   q: 0.5 },
            CurveNode { x: 0.8,  y: 0.0,   q: 0.5 },
            // High shelf at 20 Hz: boosts all audible frequencies to gain ≈ 2×
            CurveNode { x: 0.0,  y: 0.334, q: 0.3 },
        ],
        _ => default_nodes(),
    }
}

/// Module-aware default nodes for a freshly-assigned slot. Lets per-module
/// calibrations (e.g. Past's Age and Smear) start from a neutral midpoint so
/// the offset slider has headroom in both directions, instead of starting at
/// gain=1.0 (where positive offset clamps to a no-op).
///
/// Falls back to `default_nodes_for_curve(curve_idx)` for any (module, curve)
/// combination that hasn't been calibrated yet — modules pick this up
/// automatically when their UX overhaul lands.
/// See spec docs/superpowers/specs/2026-05-04-past-module-ux-design.md §7.2.
pub fn default_nodes_for_module_curve(
    module_type: crate::dsp::modules::ModuleType,
    curve_idx: usize,
) -> [CurveNode; 6] {
    use crate::dsp::modules::ModuleType;

    // Build a flat node row at a given y value, preserving the standard
    // x spacing used by `default_nodes()`.
    fn flat_at_y(y: f32) -> [CurveNode; 6] {
        [
            CurveNode { x: 0.0, y, q: 0.3 },
            CurveNode { x: 0.2, y, q: 0.5 },
            CurveNode { x: 0.4, y, q: 0.5 },
            CurveNode { x: 0.6, y, q: 0.5 },
            CurveNode { x: 0.8, y, q: 0.5 },
            CurveNode { x: 1.0, y, q: 0.3 },
        ]
    }

    match (module_type, curve_idx) {
        // Past Age / Delay (curve 1): default to gain ≈ 0.5 → display ≈ 50% of
        // total history. y = log(0.5)*20/18 ≈ -0.334 → compute_curve_response
        // produces gain = 10^(-0.334 * 18 / 20) = 0.5. Centring the default
        // means the offset slider has equal headroom in both directions.
        (ModuleType::Past, 1) => flat_at_y(-0.334),
        // Past Smear (curve 3): default sits exactly on the >0.5 toggle
        // threshold so positive offset enables smear and negative offset
        // disables it, with the slider lerp tracking the audible boundary.
        // y = -0.334 → gain = 10^(-0.334 * 18 / 20) = 0.5.
        (ModuleType::Past, 3) => flat_at_y(-0.334),
        _ => default_nodes_for_curve(curve_idx),
    }
}

#[derive(Clone, Copy, Debug)]
pub enum BandType { LowShelf, Bell, HighShelf }

pub fn band_type_for(index: usize) -> BandType {
    match index {
        0 => BandType::LowShelf,
        5 => BandType::HighShelf,
        _ => BandType::Bell,
    }
}

// ─── Curve math (unchanged) ───────────────────────────────────────────────────

fn node_to_physical(node: &CurveNode) -> (f32, f32, f32) {
    let freq_hz = 20.0 * 1000.0f32.powf(node.x);
    let gain_db = node.y * 18.0;
    let bw_oct  = 0.1 * 40.0f32.powf(node.q);
    (freq_hz, gain_db, bw_oct)
}

fn magnitude_bell_curve(f_hz: f32, f0: f32, gain_db: f32, bw_oct: f32) -> f32 {
    if gain_db.abs() < 1e-6 { return 1.0; }
    let sigma    = bw_oct / 2.355;
    let log_ratio = (f_hz / f0).abs().max(0.001).ln() / std::f32::consts::LN_2;
    let bell     = (-(log_ratio * log_ratio) / (2.0 * sigma * sigma)).exp();
    1.0 + (10.0f32.powf(gain_db / 20.0) - 1.0) * bell
}

fn magnitude_shelf_curve(f_hz: f32, f0: f32, gain_db: f32, bw_oct: f32, is_high: bool) -> f32 {
    if gain_db.abs() < 1e-6 { return 1.0; }
    let gain_linear = 10.0f32.powf(gain_db / 20.0);
    let log_ratio   = (f_hz / f0).max(0.001).ln() / std::f32::consts::LN_2;
    let tw          = 2.0 + bw_oct;
    let t = if is_high {
        (log_ratio + tw / 2.0) / tw
    } else {
        (-log_ratio + tw / 2.0) / tw
    };
    let s = t.clamp(0.0, 1.0);
    let s = 3.0 * s * s - 2.0 * s * s * s;
    1.0 + (gain_linear - 1.0) * s
}

fn eq_band_magnitude(f_hz: f32, f0: f32, gain_db: f32, bw_oct: f32, band: BandType) -> f32 {
    match band {
        BandType::Bell      => magnitude_bell_curve(f_hz, f0, gain_db, bw_oct),
        BandType::LowShelf  => magnitude_shelf_curve(f_hz, f0, gain_db, bw_oct, false),
        BandType::HighShelf => magnitude_shelf_curve(f_hz, f0, gain_db, bw_oct, true),
    }
}

/// Compute combined linear gain response for all 6 nodes at `num_bins` frequencies.
pub fn compute_curve_response(
    nodes: &[CurveNode; 6],
    num_bins: usize,
    sample_rate: f32,
    fft_size: usize,
) -> Vec<f32> {
    let mut gains = vec![1.0f32; num_bins];
    for (i, node) in nodes.iter().enumerate() {
        if node.y.abs() < 1e-4 { continue; }
        let (freq_hz, gain_db, bw_oct) = node_to_physical(node);
        let band = band_type_for(i);
        for k in 0..num_bins {
            let f_bin = (k as f32 * sample_rate / fft_size as f32).max(1.0);
            gains[k] *= eq_band_magnitude(f_bin, freq_hz, gain_db, bw_oct, band);
        }
    }
    for g in &mut gains { *g = g.max(0.0); }
    gains
}

// ─── Screen coordinate helpers ────────────────────────────────────────────────

/// Map node.x (0..1 log-normalised to 20 Hz–20 kHz) to pixel x,
/// scaled so the right edge corresponds to `max_hz`.
/// At `max_hz = 20_000` this equals the old `x_to_screen`.
#[inline]
pub fn x_to_screen(node_x: f32, rect: Rect, max_hz: f32) -> f32 {
    // node.x = log10(f/20) / log10(1000)  →  f = 20 * 10^(3 * node.x)
    // scale keeps node.x=1 at 20 kHz while the right edge extends to max_hz
    let scale = 3.0 / (max_hz / 20.0).log10();
    rect.left() + node_x * scale * rect.width()
}

/// Map a frequency in Hz to log-scaled pixel x with a dynamic upper bound.
/// `max_hz` is typically `sample_rate / 2`.
#[inline]
pub fn freq_to_x_max(f_hz: f32, max_hz: f32, rect: Rect) -> f32 {
    let max_hz = max_hz.max(20_001.0); // guard against < 20 kHz pathological SR
    let f = f_hz.clamp(20.0, max_hz);
    let t = (f / 20.0).log10() / (max_hz / 20.0).log10();
    rect.left() + t * rect.width()
}

/// Inverse of `freq_to_x_max` — pixel x → Hz.
#[inline]
pub fn screen_to_freq(x: f32, rect: Rect, max_hz: f32) -> f32 {
    let t = ((x - rect.left()) / rect.width()).clamp(0.0, 1.0);
    20.0 * 10.0_f32.powf(t * (max_hz / 20.0).log10())
}

/// Map a per-module curve slot index to the canonical display index used by
/// gain_to_display, physical_to_y, and screen_y_to_physical. The per-curve
/// grid lines and Y-axis unit label now live in `CurveDisplayConfig` and are
/// looked up via `curve_config::curve_display_config`.
///
/// Display indices:
///   0  Threshold dBFS           1  Ratio 1–20           2/3 Attack/Release ms
///   4  Knee dB                  5  Makeup/Gain dB        6  Mix %
///   7  Amount 0–200%            8  Freeze Length ms      9  Freeze Threshold dBFS
///   10 Portamento/SC-smooth ms  11 Resistance 0–2        12 Dry-mix % (shares idx-5 dB mapping)
pub fn display_curve_idx(module_type: ModuleType, curve_idx: usize, gain_mode: GainMode) -> usize {
    match module_type {
        ModuleType::Dynamics => match curve_idx {
            5 => 6,             // MIX → % display (was wrongly shown as Makeup dB)
            n => n,
        },
        ModuleType::Freeze => match curve_idx {
            0 => 8,             // LENGTH → ms (log, 10–4000)
            1 => 9,             // THRESHOLD → dBFS
            2 => 10,            // PORTAMENTO → ms (log, 1–1000)
            3 => 11,            // RESISTANCE → dimensionless 0–2
            4 => 6,             // MIX → %
            _ => curve_idx,
        },
        ModuleType::PhaseSmear => match curve_idx {
            0 => 7,             // AMOUNT → 0–200 %
            1 => 10,            // PEAK HOLD → ms (log, treated as time constant)
            2 => 6,             // MIX → %
            _ => curve_idx,
        },
        ModuleType::Contrast => match curve_idx {
            0 => 1,             // AMOUNT → ratio 1–20 (maps gain directly to bp_ratio)
            _ => curve_idx,
        },
        ModuleType::Gain => match curve_idx {
            // In Add/Subtract the curve is a ±18 dB gain; in Pull/Match it's a
            // clamped [0, 1] wet/dry mix drawn on the same screen scale but with
            // percentage grid labels.
            0 => match gain_mode {
                GainMode::Pull | GainMode::Match => 12,
                _ => 5,
            },
            1 => 10,            // PEAK HOLD → ms
            _ => curve_idx,
        },
        ModuleType::MidSide => match curve_idx {
            0 | 1 => 7,         // BALANCE / EXPANSION → 0–200 % (neutral = 100 %)
            _ => 6,             // DECORREL / TRANSIENT / PAN → 0–100 %
        },
        ModuleType::TransientSustainedSplit => match curve_idx {
            0 => 6,             // SENSITIVITY → 0–100 %
            _ => curve_idx,
        },
        ModuleType::Past => match curve_idx {
            0 => 6,   // AMOUNT → 0–100 %
            // TIME → seconds-history. Until Task 14 re-plumbs paint_response_curve and
            // the offset DragValue formatter to receive Pipeline's real total_history_seconds,
            // both render with total=0.0 and produce flat-zero output. Tracked in Task 14.
            1 => 13,
            2 => 9,   // THRESHOLD → dBFS (mirrors Freeze threshold scale)
            3 => 6,   // SPREAD / Smear → 0–100 %
            4 => 6,   // MIX → 0–100 %
            _ => curve_idx,
        },
        _ => curve_idx,
    }
}

/// Maximum absolute value for the Offset drag-slider, per display index.
/// The offset is added to the raw linear gain before display mapping, so
/// the range should span from the parameter minimum to maximum for every
/// curve type. Values chosen so that ±off_max reaches (or saturates) the
/// full display range relative to the neutral gain of 1.0.
pub fn curve_offset_max(display_idx: usize) -> f32 {
    // UI parameter contract: see docs/superpowers/specs/2026-04-23-ui-parameter-spec-design.md
    match display_idx {
        0 | 9  => 1.5,   // Threshold dBFS: ±1.5 → ±neutral; clamps cover full ±80 dBFS
        1      => 19.0,  // Ratio 1–20: offset +19 reaches 20:1 from neutral 1:1
        2 | 3  => 2.0,   // Attack/Release multiplier: 0× to 3× of global value
        4      => 8.0,   // Knee dB 1.5–48: offset +7 reaches 48 dB from neutral (6 dB)
        5 | 12 => 8.0,   // Makeup/Gain dB / dry-mix %: ±8 gain ≈ ±18 dB around neutral
        6      => 1.0,   // Mix %: ±1 covers 0–200 %
        7      => 1.0,   // Amount/Balance 0–200 %: ±1 covers full range
        8      => 7.0,   // Freeze Length 0–4000 ms: offset +7 reaches 4000 ms from 500 ms
        10     => 4.0,   // Portamento/SC Smooth 0–1000 ms: offset +4 reaches 1000 ms
        11     => 1.0,   // Resistance 0–2: ±1 covers full range around neutral 1.0
        _      => 1.5,
    }
}

/// Inverse of `physical_to_y` — pixel y → physical value for tooltip display.
pub fn screen_y_to_physical(y: f32, curve_idx: usize, db_min: f32, db_max: f32, rect: Rect) -> f32 {
    let t = ((rect.bottom() - y) / rect.height()).clamp(0.0, 1.0);
    match curve_idx {
        0 => db_min + t * (db_max - db_min),
        1 => 1.0 * 20.0_f32.powf(t),
        2 | 3 => 1024.0_f32.powf(t),
        4 => 1.5 * (48.0_f32 / 1.5).powf(t),
        5 => -18.0 + t * 36.0,
        6 => t * 100.0,
        7 => t * 200.0,
        8 => 62.5 * (4000.0_f32 / 62.5).powf(t),   // Freeze Length: 62.5ms–4000ms log (matches config y_min)
        9 => -80.0 + t * 80.0,
        10 => 40.0 * (1000.0_f32 / 40.0).powf(t),  // Portamento: 40ms–1000ms log (matches config y_min)
        11 => t * 2.0,                    // Resistance 0–2
        12 => {
            // Dry-mix %: screen y → dB (-18..+18) → clamped wet/dry percentage.
            // Matches the grid labels: 0 dB=100 %, -6 dB=50 %, -12 dB=25 %, -18 dB≈12 %.
            let db = -18.0 + t * 36.0;
            (100.0 * 10f32.powf(db / 20.0)).clamp(0.0, 100.0)
        }
        _ => 0.0,
    }
}

/// Format a frequency in Hz as a human-readable string: "440 Hz" or "1.00 kHz".
pub fn format_freq_hz(hz: f32) -> String {
    if hz >= 1_000.0 {
        format!("{:.2} kHz", hz / 1_000.0)
    } else {
        format!("{:.0} Hz", hz)
    }
}

/// Paint cursor tooltip: "440 Hz  /  -18.3 dBFS"
/// Single shared routine for all curves — no per-curve hover paths.
/// The physical Y value comes from `screen_y_to_physical` (indexed by `display_idx`);
/// the unit label and formatting come from `cfg`.
/// See docs/superpowers/specs/2026-04-23-ui-parameter-spec-design.md §3.
pub fn paint_hover_text(
    painter: &Painter,
    cursor_pos: Pos2,
    rect: Rect,
    display_idx: usize,
    cfg: &CurveDisplayConfig,
    db_min: f32,
    db_max: f32,
    sample_rate: f32,
) {
    use nih_plug_egui::egui::{FontId, vec2};
    let nyquist = (sample_rate / 2.0).max(20_001.0);
    let freq_hz = screen_to_freq(cursor_pos.x, rect, nyquist);
    let phys    = screen_y_to_physical(cursor_pos.y, display_idx, db_min, db_max, rect);
    let text = if cfg.y_label.is_empty() {
        format!("{}  /  {:.2}", format_freq_hz(freq_hz), phys)
    } else {
        format!("{}  /  {:.1} {}", format_freq_hz(freq_hz), phys, cfg.y_label)
    };
    let scale  = painter.ctx().pixels_per_point();
    let galley = painter.layout_no_wrap(text, FontId::proportional(th::scaled(th::FONT_SIZE_HOVER, scale)), th::GRID_TEXT);
    // Default position: above-right of cursor. Flip left if it would clip the right edge,
    // flip down if it would clip the top edge.
    let mut tip_pos = cursor_pos + vec2(12.0, -28.0);
    if tip_pos.x + galley.size().x + 6.0 > rect.right() {
        tip_pos.x = cursor_pos.x - 12.0 - galley.size().x;
    }
    if tip_pos.y - 3.0 < rect.top() {
        tip_pos.y = cursor_pos.y + 16.0;
    }
    let bg_rect = Rect::from_min_size(
        tip_pos - vec2(3.0, 3.0),
        galley.size() + vec2(6.0, 6.0),
    );
    painter.rect_filled(bg_rect, 2.0, Color32::from_black_alpha(180));
    painter.galley(tip_pos, galley, th::GRID_TEXT);
}

/// Map a physical value to pixel y using a linear scale.
#[inline]
fn linear_to_y(v: f32, y_min: f32, y_max: f32, rect: Rect) -> f32 {
    let t = ((v - y_min) / (y_max - y_min)).clamp(0.0, 1.0);
    rect.bottom() - t * rect.height()
}

/// Map a physical value to pixel y using a logarithmic scale.
#[inline]
fn log_to_y(v: f32, y_min: f32, y_max: f32, rect: Rect) -> f32 {
    let v   = v.max(y_min);
    let t   = ((v / y_min).log10() / (y_max / y_min).log10()).clamp(0.0, 1.0);
    rect.bottom() - t * rect.height()
}

// ─── Physical value mapping ───────────────────────────────────────────────────

/// Apply per-curve tilt, calibrated offset, and curvature (S-curve blend) to a raw gain.
/// `curvature` ∈ [0, 1]: 0 = straight tilt, 1 = full smoothstep S-curve pivoted at 1 kHz.
/// `offset_fn` is the per-curve calibrated transform from `CurveDisplayConfig::offset_fn`.
/// `nyquist` — host Nyquist frequency (sample_rate / 2); governs the log-range used for
/// the tilt shape so it matches any sample rate.
/// See docs/superpowers/specs/2026-04-23-ui-parameter-spec-design.md §2 and §3.
#[inline]
pub fn apply_curve_adjustments(
    gain: f32,
    freq_hz: f32,
    tilt: f32,
    offset: f32,
    curvature: f32,
    offset_fn: fn(f32, f32) -> f32,
    nyquist: f32,
) -> f32 {
    // curvature only shapes the tilt; if tilt=0, curvature has no effect.
    // offset_fn(g, 0.0) == g for all calibrations, so offset=0 is also a no-op.
    if tilt.abs() < 1e-6 && offset.abs() < 1e-6 { return gain; }
    // Map freq to log-normalised [0, 1] (20 Hz → nyquist).
    // Pivot at 1 kHz — computed dynamically so it stays centred at 1 kHz across sample rates.
    const LOG_20: f32 = 1.301_030; // log10(20.0)
    let log_range  = (nyquist / 20.0).log10(); // e.g. 3.0 at 20 kHz Nyquist (40 kHz SR)
    let pivot      = (1000.0_f32 / 20.0).log10() / log_range;
    // Smoothstep value at the pivot — centres the sigmoid shape there.
    let s_pivot    = 3.0 * pivot * pivot - 2.0 * pivot * pivot * pivot;
    let norm = ((freq_hz.max(20.0).log10() - LOG_20) / log_range).clamp(0.0, 1.0);
    let linear_shape  = norm - pivot;
    let s             = 3.0 * norm * norm - 2.0 * norm * norm * norm; // smoothstep(norm)
    let sigmoid_shape = s - s_pivot;
    let shape = linear_shape + curvature * (sigmoid_shape - linear_shape);
    let t = tilt * shape;
    let g_off = offset_fn(gain, offset);
    (g_off * (1.0 + t)).max(0.0)
}

/// Resolve a `CurveDisplayConfig`'s declared anchors `(y_min, y_natural, y_max)`
/// into runtime physical units. The default identity passthrough is the right
/// answer for every absolute-units calibration (dBFS, ratio, ms, %, etc.). The
/// one exception is **display index 13** (Past's Age/Delay, history-relative
/// seconds): the config stores the anchors as fractions of `y_max`, and at
/// runtime we substitute `y_max → total_history_seconds` and scale `y_min` and
/// `y_natural` by the same factor.
///
/// Per UI parameter spec §2, the offset slider's displayed value is a
/// piecewise-linear interpolation between these three anchors keyed on the
/// normalised offset `[-1, 1]`. The slider formatter calls this helper so it
/// receives the runtime-correct anchors regardless of curve type.
pub fn runtime_anchors(
    cfg: &crate::editor::curve_config::CurveDisplayConfig,
    display_idx: usize,
    total_history_seconds: f32,
) -> (f32, f32, f32) {
    if display_idx == 13 {
        // Past Age/Delay: cfg anchors are fractions of total_history_seconds.
        let scale = total_history_seconds;
        (cfg.y_min * scale, cfg.y_natural * scale, cfg.y_max * scale)
    } else {
        (cfg.y_min, cfg.y_natural, cfg.y_max)
    }
}

/// Convert a curve's linear gain to its physical display value (no freq scaling).
/// Used for the coloured response line.
///
/// `total_history_seconds` is consumed only by display index 13 ("seconds,
/// history-relative") used by Past's Age/Delay curves; legacy callers pass
/// `0.0` and never hit index 13.
pub fn gain_to_display(
    curve_idx: usize,
    gain: f32,
    global_attack_ms: f32,
    global_release_ms: f32,
    db_min: f32,
    db_max: f32,
    total_history_seconds: f32,
) -> f32 {
    // UI parameter contract: see docs/superpowers/specs/2026-04-23-ui-parameter-spec-design.md
    match curve_idx {
        0 => {
            // Matches the pipeline formula: log-based ±60 dBFS range centred at −20 dBFS.
            let t_db = if gain > 1e-10 { 20.0 * gain.log10() } else { -120.0 };
            (-20.0 + t_db * (60.0 / 18.0)).clamp(db_min, db_max)
        }
        1 => gain.clamp(1.0, 20.0),
        2 => (global_attack_ms  * gain.max(0.01)).clamp(1.0, 1024.0),
        3 => (global_release_ms * gain.max(0.01)).clamp(1.0, 1024.0),
        4 => (gain * 6.0).clamp(1.5, 48.0),
        5 | 12 => if gain > 1e-6 { 20.0 * gain.log10() } else { -60.0 },
        6 => (gain * 100.0).clamp(0.0, 100.0),
        // Effects curves — tilt/offset not used (passed as 0.0/0.0 from UI)
        7 => gain.clamp(0.0, 2.0) * 100.0,                  // Phase Amount: 0-200%
        8 => (gain * 500.0).clamp(0.0, 4000.0),             // Freeze Length: 0-4000ms (neutral=500ms)
        9 => {                                               // Freeze Threshold: dBFS
            // Matches the DSP formula in freeze::curve_to_threshold_db:
            // linear in gain — gain=1.0 → -20, gain=2.0 → 0, gain=-2.0 → -80 (clamped).
            (-40.0 + gain * 20.0).clamp(-80.0, 0.0)
        }
        10 => (gain * 200.0).clamp(0.0, 1000.0),            // Portamento/SC Smooth: 0-1000ms (neutral=200ms)
        11 => gain.clamp(0.0, 2.0),                          // Resistance: 0-2 (normalised excess)
        13 => (gain * total_history_seconds).clamp(0.0, total_history_seconds),
        _ => gain,
    }
}


/// Map a physical value to pixel y for a given curve type.
pub fn physical_to_y(v: f32, curve_idx: usize, db_min: f32, db_max: f32, rect: Rect) -> f32 {
    // UI parameter contract: see docs/superpowers/specs/2026-04-23-ui-parameter-spec-design.md
    match curve_idx {
        0 => linear_to_y(v, db_min, db_max, rect),
        1 => log_to_y(v, 1.0, 20.0, rect),
        2 | 3 => log_to_y(v, 1.0, 1024.0, rect),
        4 => log_to_y(v, 1.5, 48.0, rect),
        5 | 12 => linear_to_y(v, -18.0, 18.0, rect),
        6 => linear_to_y(v, 0.0, 100.0, rect),
        7 => linear_to_y(v, 0.0, 200.0, rect),
        8 => log_to_y(v.max(62.5), 62.5, 4000.0, rect),    // Freeze Length 62.5ms–4000ms (matches config y_min)
        9 => linear_to_y(v, -80.0, 0.0, rect),
        10 => log_to_y(v.max(40.0), 40.0, 1000.0, rect),   // Portamento 40ms–1000ms (matches config y_min)
        11 => linear_to_y(v, 0.0, 2.0, rect),               // Resistance 0–2
        _ => rect.center().y,
    }
}

// ─── Grid ─────────────────────────────────────────────────────────────────────

const HZ_VERTICALS: &[f32] = &[
    10., 20., 30., 40., 50., 60., 70., 80., 90.,
    100., 200., 300., 400., 500., 600., 700., 800., 900.,
    1_000., 2_000., 3_000., 4_000., 5_000., 6_000., 7_000., 8_000., 9_000.,
    10_000., 11_000., 12_000., 13_000., 14_000., 15_000., 16_000., 17_000., 18_000., 19_000., 20_000.,
];
// Extra verticals drawn only when sample_rate > 44.1 kHz
const HZ_VERTICALS_HI: &[f32] = &[
    21_000., 22_000., 24_000., 26_000., 28_000.,
    30_000., 35_000., 40_000., 45_000.,
];
/// Fixed frequency labels shown on the X axis (excluding the rightmost Nyquist label,
/// which is added dynamically by `paint_grid`).
/// See docs/superpowers/specs/2026-04-23-ui-parameter-spec-design.md §3.
const HZ_LABELS: &[(f32, &str)] = &[(100., "100"), (1_000., "1k"), (10_000., "10k")];

/// Paint background grid: vertical Hz lines + curve-specific horizontal lines + Y-axis label.
/// `cfg` supplies the horizontal grid lines and Y-axis unit label.
/// `display_idx` is the canonical curve index used by `physical_to_y` to map each
/// `cfg.grid_lines` physical value back to a pixel Y position.
/// `sample_rate` is used to extend the grid beyond 20 kHz at high sample rates.
/// See docs/superpowers/specs/2026-04-23-ui-parameter-spec-design.md §3.
pub fn paint_grid(
    painter: &Painter,
    rect: Rect,
    cfg: &CurveDisplayConfig,
    display_idx: usize,
    db_min: f32,
    db_max: f32,
    sample_rate: f32,
) {
    // UI parameter contract: see docs/superpowers/specs/2026-04-23-ui-parameter-spec-design.md §4
    let scale   = painter.ctx().pixels_per_point();
    let nyquist = sample_rate / 2.0;
    let max_hz  = nyquist.max(20_001.0);
    let grid_stroke = Stroke::new(th::scaled_stroke(th::STROKE_THIN, scale), th::GRID_LINE);
    let font = nih_plug_egui::egui::FontId::proportional(th::scaled(th::FONT_SIZE_GRID, scale));

    // Vertical lines at Hz intervals
    for &f in HZ_VERTICALS {
        if f > max_hz { continue; }
        let x = freq_to_x_max(f, max_hz, rect);
        painter.line_segment(
            [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
            grid_stroke,
        );
    }
    // Extra high-SR lines
    if sample_rate > 44_100.0 {
        for &f in HZ_VERTICALS_HI {
            if f > max_hz { continue; }
            let x = freq_to_x_max(f, max_hz, rect);
            painter.line_segment(
                [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
                grid_stroke,
            );
        }
    }
    // Hz labels at bottom
    for &(f, label) in HZ_LABELS {
        if f > max_hz * 1.05 { continue; }
        let x = freq_to_x_max(f, max_hz, rect);
        painter.text(
            Pos2::new(x + 2.0, rect.bottom() - 10.0),
            nih_plug_egui::egui::Align2::LEFT_BOTTOM,
            label,
            font.clone(),
            th::GRID_TEXT,
        );
    }
    // Always show the Nyquist label at the right edge.
    // See docs/superpowers/specs/2026-04-23-ui-parameter-spec-design.md §3.
    {
        let nyq_khz = (nyquist / 1000.0).round() as u32;
        let label = format!("{}k", nyq_khz);
        let x = freq_to_x_max(nyquist, max_hz, rect);
        painter.text(
            Pos2::new(x + 2.0, rect.bottom() - 10.0),
            nih_plug_egui::egui::Align2::LEFT_BOTTOM,
            label,
            font.clone(),
            th::GRID_TEXT,
        );
    }

    // Horizontal grid lines driven by CurveDisplayConfig.grid_lines.
    // See docs/superpowers/specs/2026-04-23-ui-parameter-spec-design.md §3.
    for &(v, label) in cfg.grid_lines {
        // Skip values that fall outside the curve's configured display range
        // (e.g. Threshold grid line -48 dBFS when db_min > -48).
        if v < cfg.y_min || v > cfg.y_max { continue; }
        // For curves whose runtime display range is user-adjustable (threshold),
        // also respect the current db_min/db_max window.
        if display_idx == 0 && (v < db_min || v > db_max) { continue; }
        let y = physical_to_y(v, display_idx, db_min, db_max, rect);
        painter.line_segment(
            [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
            grid_stroke,
        );
        painter.text(
            Pos2::new(rect.left() + 2.0, y - 2.0),
            nih_plug_egui::egui::Align2::LEFT_BOTTOM,
            label,
            font.clone(),
            th::GRID_TEXT,
        );
    }

    // Y-axis unit label at top-left of the rect.
    // See docs/superpowers/specs/2026-04-23-ui-parameter-spec-design.md §3.
    if !cfg.y_label.is_empty() {
        let label_font = nih_plug_egui::egui::FontId::proportional(
            th::scaled(th::FONT_SIZE_LABEL, scale),
        );
        painter.text(
            Pos2::new(rect.left() + 2.0, rect.top() + 2.0),
            nih_plug_egui::egui::Align2::LEFT_TOP,
            cfg.y_label,
            label_font,
            th::GRID_TEXT,
        );
    }
}

// ─── Curve rendering ──────────────────────────────────────────────────────────

/// Paint the response curve for one curve channel.
/// `gains` — output of `compute_curve_response()` (linear).
/// The coloured line maps gains to physical display values (no freq scaling).
/// For attack/release curves, also paints a grey true-time line with freq scaling.
/// `stroke_width` — 1.0 for inactive curves, 2.0 for the active curve.
pub fn paint_response_curve(
    painter: &Painter,
    rect: Rect,
    gains: &[f32],
    curve_idx: usize,
    color: Color32,
    stroke_width: f32,
    db_min: f32,
    db_max: f32,
    global_attack_ms: f32,
    global_release_ms: f32,
    sample_rate: f32,
    fft_size: usize,
    tilt: f32,
    offset: f32,
    curvature: f32,
    offset_fn: fn(f32, f32) -> f32,
    total_history_seconds: f32,
) {
    if gains.len() < 2 { return; }
    let n = gains.len();
    let max_hz = (sample_rate / 2.0).max(20_001.0);

    // Coloured response line — dashed for attack/release, solid for all others.
    // Tilt, offset, and curvature are applied to the raw gain before display mapping.
    let pts: Vec<Pos2> = (0..n).map(|k| {
        let f_hz = (k as f32 * sample_rate / fft_size as f32).max(20.0);
        let x    = freq_to_x_max(f_hz, max_hz, rect);
        let adj  = apply_curve_adjustments(gains[k], f_hz, tilt, offset, curvature, offset_fn, max_hz);
        let v    = gain_to_display(curve_idx, adj, global_attack_ms, global_release_ms, db_min, db_max, total_history_seconds);
        let y    = physical_to_y(v, curve_idx, db_min, db_max, rect);
        Pos2::new(x, y)
    }).collect();
    let line_stroke = Stroke::new(stroke_width, color);
    if curve_idx == 2 || curve_idx == 3 {
        paint_dashed_line(painter, &pts, line_stroke, 4.0, 2.0);
    } else {
        painter.add(Shape::line(pts, line_stroke));
    }
}

/// Paint a 1-px darker overlay line reflecting the live SC peak-hold envelope
/// behind the active Gain curve. Uses the same log-frequency mapping as
/// `paint_response_curve` so the overlay aligns with the drawn response.
///
/// `envelope[k]` is the raw FFT-bin linear magnitude for bin `k`. This matches
/// the convention of `spectrum_display`: the same `4/fft_size` normalisation
/// is applied here so the overlay sits on the same dBFS scale as the pre/post
/// spectrum display (a unit sinusoid peaks at roughly 0 dB).
pub fn paint_peak_hold_envelope_overlay(
    painter: &Painter,
    rect: Rect,
    envelope: &[f32],
    curve_color: Color32,
    sample_rate: f32,
    fft_size: usize,
    db_min: f32,
    db_max: f32,
) {
    if envelope.is_empty() || fft_size == 0 { return; }
    // UI parameter contract: see docs/superpowers/specs/2026-04-23-ui-parameter-spec-design.md §4
    let scale = painter.ctx().pixels_per_point();
    // Derive a darker tone from curve_color (r/3, g/3, b/3, opaque).
    let dim = Color32::from_rgb(
        curve_color.r() / 3,
        curve_color.g() / 3,
        curve_color.b() / 3,
    );
    let n = envelope.len();
    let max_hz = (sample_rate / 2.0).max(20_001.0);
    let norm_factor = 4.0 / fft_size as f32;
    let range = (db_max - db_min).max(1.0);
    let overlay_stroke = Stroke::new(th::scaled_stroke(th::STROKE_THIN, scale), dim);
    let mut prev: Option<Pos2> = None;
    for k in 1..n {
        let f_hz = (k as f32 * sample_rate / fft_size as f32).max(20.0);
        let x    = freq_to_x_max(f_hz, max_hz, rect);
        let mag  = (envelope[k] * norm_factor).max(1e-12);
        let db   = 20.0 * mag.log10();
        // Map dB onto the spectrum display range [db_min..db_max]. Top = db_max, bottom = db_min.
        let norm = ((db - db_min) / range).clamp(0.0, 1.0);
        let y    = rect.max.y - norm * rect.height();
        if let Some(p) = prev {
            painter.line_segment([p, Pos2::new(x, y)], overlay_stroke);
        }
        prev = Some(Pos2::new(x, y));
    }
}

// ─── Interactive widget ───────────────────────────────────────────────────────

/// Return value from [`curve_widget`].
pub struct CurveWidgetResult {
    /// True if any node's parameters changed this frame.
    pub changed: bool,
    /// True if a drag gesture started this frame (use to call `begin_set_parameter`).
    pub drag_started: bool,
    /// True if a drag gesture ended this frame (use to call `end_set_parameter`).
    pub drag_stopped: bool,
    /// If an alt-click landed on a node this frame: `Some((screen_pos, node_index))`.
    pub alt_clicked_node: Option<(Pos2, usize)>,
}

/// Draw interactive nodes for the active curve. Returns drag/change state.
/// Handles are drawn at normalised y positions (node.y ∈ [-1, +1] mapped to [bottom, top])
/// so they cover the full display height and track the cursor 1:1 when dragged.
pub fn curve_widget(
    ui: &mut Ui,
    rect: Rect,
    nodes: &mut [CurveNode; 6],
    curve_idx: usize,
    sample_rate: f32,
) -> CurveWidgetResult {
    use nih_plug_egui::egui::Sense;

    let max_hz = (sample_rate / 2.0).max(20_001.0);
    let mut changed = false;
    let mut drag_started = false;
    let mut drag_stopped = false;
    let mut alt_clicked_node: Option<(Pos2, usize)> = None;
    let node_color_lit  = th::curve_color_lit(curve_idx);
    let node_color_hover = {
        let c = node_color_lit;
        Color32::from_rgb(
            (c.r() as u16 + 40).min(255) as u8,
            (c.g() as u16 + 40).min(255) as u8,
            (c.b() as u16 + 40).min(255) as u8,
        )
    };

    for i in 0..6 {
        // Normalised y position: node.y ∈ [-1, +1] maps linearly to [bottom, top] of rect.
        // node.y has ±2 parameter headroom (see drag clamp below) but the dot is pinned to the
        // graph rect so it can never stray into the routing matrix above or below. The headroom
        // still influences the rendered response curve and underlying parameters.
        let sy_raw = rect.bottom() - (nodes[i].y + 1.0) / 2.0 * rect.height();
        let sy = sy_raw.clamp(rect.top(), rect.bottom());

        // Visual position scaled to the current SR's Nyquist range.
        // Low shelf (i=0): fixed +20px nudge right so it stays visible at the left edge.
        // High shelf (i=5): nudge right by however much is needed to keep the ◀ tip on-screen.
        //   The leftmost vertex of the ◀ triangle is at draw_pos.x - NODE_RADIUS, and
        //   draw_pos.x = sx_actual - NODE_RADIUS*0.5, so tip is at sx_actual - 1.5*NODE_RADIUS.
        //   Nudge = max(0, rect.left() + 1.5*r - raw_sx) ensures the tip is never clipped.
        let shelf_nudge = if i == 0 {
            20.0
        } else if i == 5 {
            let raw_sx = x_to_screen(nodes[i].x, rect, max_hz);
            (rect.left() + th::NODE_RADIUS * 1.5 - raw_sx).max(0.0)
        } else {
            0.0
        };
        let sx_actual = x_to_screen(nodes[i].x, rect, max_hz) + shelf_nudge;
        let sx_draw   = sx_actual - th::NODE_RADIUS * 0.5;
        let node_pos  = Pos2::new(sx_actual, sy);
        let draw_pos  = Pos2::new(sx_draw,  sy);

        let node_rect = Rect::from_center_size(node_pos, Vec2::splat(th::NODE_RADIUS * 3.0));
        let resp = ui.interact(node_rect, ui.id().with(("node", i)), Sense::drag());
        drag_started |= resp.drag_started();
        drag_stopped |= resp.drag_stopped();

        // Dual-button drag for Q — when both primary and secondary mouse buttons are held,
        // dragging up/down adjusts Q smoothly.  Scale: 500px → full Q range (0→1),
        // corresponding roughly to the distance from the centre to the top of a mouse mat.
        let (both_down, ptr_delta, hover_here) = ui.input(|inp| {
            let hov = inp.pointer.hover_pos().unwrap_or(Pos2::ZERO);
            (
                inp.pointer.primary_down() && inp.pointer.secondary_down(),
                inp.pointer.delta(),
                node_rect.contains(hov),
            )
        });

        if both_down && hover_here {
            // Both buttons → Q drag, suppress position drag.
            if ptr_delta.y.abs() > 0.0 {
                nodes[i].q = (nodes[i].q - ptr_delta.y / 500.0).clamp(0.0, 1.0);
                changed = true;
            }
        } else if resp.dragged() {
            // Single primary button → move node position.
            let delta = resp.drag_delta();
            nodes[i].x = (nodes[i].x + delta.x / rect.width()).clamp(0.0, 1.0);
            nodes[i].y = (nodes[i].y - (delta.y / rect.height()) * 2.0).clamp(-2.0, 2.0);
            changed = true;
        }

        // Scroll wheel Q — coarse jumps (kept for quick rough adjustment)
        if hover_here && !both_down {
            let scroll = ui.input(|inp| inp.raw_scroll_delta.y);
            if scroll.abs() > 0.01 {
                nodes[i].q = (nodes[i].q + scroll * 0.002).clamp(0.0, 1.0);
                changed = true;
            }
        }

        if resp.double_clicked() {
            nodes[i] = default_nodes()[i];
            changed = true;
        }

        // Alt-click opens the Modulation Ring overlay for this node.
        let alt_down = ui.input(|inp| inp.modifiers.alt);
        if resp.clicked() && alt_down {
            alt_clicked_node = Some((node_pos, i));
        }

        crate::editor::delayed_tooltip(ui, &resp, format!("Node {} \u{2014} drag to adjust", i));

        let color = if resp.hovered() { node_color_hover } else { node_color_lit };
        let r = th::NODE_RADIUS;
        match band_type_for(i) {
            BandType::LowShelf => {
                // Right-pointing equilateral triangle ▶
                let pts = vec![
                    draw_pos + Vec2::new( r,    0.0),
                    draw_pos + Vec2::new(-r * 0.5,  r * 0.866),
                    draw_pos + Vec2::new(-r * 0.5, -r * 0.866),
                ];
                ui.painter().add(Shape::convex_polygon(pts, color,
                    Stroke::new(th::STROKE_BORDER, th::BORDER)));
            }
            BandType::HighShelf => {
                // Left-pointing equilateral triangle ◀
                let pts = vec![
                    draw_pos + Vec2::new(-r,    0.0),
                    draw_pos + Vec2::new( r * 0.5, -r * 0.866),
                    draw_pos + Vec2::new( r * 0.5,  r * 0.866),
                ];
                ui.painter().add(Shape::convex_polygon(pts, color,
                    Stroke::new(th::STROKE_BORDER, th::BORDER)));
            }
            BandType::Bell => {
                ui.painter().circle_filled(draw_pos, r, color);
                ui.painter().circle_stroke(draw_pos, r,
                    Stroke::new(th::STROKE_BORDER, th::BORDER));
            }
        }
    }

    CurveWidgetResult { changed, drag_started, drag_stopped, alt_clicked_node }
}
