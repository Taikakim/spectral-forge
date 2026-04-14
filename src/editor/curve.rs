use serde::{Serialize, Deserialize};
use nih_plug_egui::egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, Ui, Vec2};
use crate::editor::theme as th;

// ─── Data types ───────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
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

/// Inverse of `physical_to_y` — pixel y → physical value for tooltip display.
pub fn screen_y_to_physical(y: f32, curve_idx: usize, db_min: f32, db_max: f32, rect: Rect) -> f32 {
    let t = ((rect.bottom() - y) / rect.height()).clamp(0.0, 1.0);
    match curve_idx {
        0 => db_min + t * (db_max - db_min),
        1 => 1.0 * 20.0_f32.powf(t),
        2 | 3 => 1024.0_f32.powf(t),
        4 => 1.5 * (48.0_f32 / 1.5).powf(t),
        5 => -36.0 + t * 72.0,
        6 => t * 100.0,
        _ => 0.0,
    }
}

/// Unit label for each curve's y-axis (for the cursor tooltip).
pub const CURVE_Y_UNIT: [&str; 7] = ["dBFS", "x", "ms", "ms", "dB", "dB", "%"];

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

/// Convert a curve's linear gain to its physical display value (no freq scaling).
/// Used for the coloured response line.
pub fn gain_to_display(
    curve_idx: usize,
    gain: f32,
    global_attack_ms: f32,
    global_release_ms: f32,
    db_min: f32,
    db_max: f32,
) -> f32 {
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
        5 => if gain > 1e-6 { (20.0 * gain.log10()).clamp(-36.0, 36.0) } else { -36.0 },
        6 => (gain * 100.0).clamp(0.0, 100.0),
        _ => gain,
    }
}

/// Convert a curve's linear gain to its physical display value WITH frequency scaling.
/// Used for the grey true-time line on attack/release curves.
pub fn gain_to_true_time(
    _curve_idx: usize,
    gain: f32,
    global_ms: f32,
    freq_scale: f32,
    bin_k: usize,
    sample_rate: f32,
    fft_size: usize,
) -> f32 {
    let f     = (bin_k as f32 * sample_rate / fft_size as f32).max(20.0);
    let scale = (1000.0_f32 / f).powf(freq_scale * 0.5);
    (global_ms * scale * gain.max(0.01)).clamp(1.0, 1024.0)
}

/// Map a physical value to pixel y for a given curve type.
pub fn physical_to_y(v: f32, curve_idx: usize, db_min: f32, db_max: f32, rect: Rect) -> f32 {
    match curve_idx {
        0 => linear_to_y(v, db_min, db_max, rect),
        1 => log_to_y(v, 1.0, 20.0, rect),
        2 | 3 => log_to_y(v, 1.0, 1024.0, rect),
        4 => log_to_y(v, 1.5, 48.0, rect),
        5 => linear_to_y(v, -36.0, 36.0, rect),
        6 => linear_to_y(v, 0.0, 100.0, rect),
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
const HZ_LABELS: &[(f32, &str)] = &[(100., "100"), (1_000., "1k"), (10_000., "10k"), (20_000., "20k")];

/// Grid horizontal lines per curve type: (physical value, label).
fn curve_grid_lines(curve_idx: usize, db_min: f32, db_max: f32) -> Vec<(f32, String)> {
    match curve_idx {
        0 => {
            // Threshold: one reference line at -12 dBFS (fixed)
            if -12.0 >= db_min && -12.0 <= db_max {
                vec![(-12.0, "-12 dB".to_string())]
            } else {
                vec![]
            }
        }
        1 => vec![
            (1.25,  "1:1.25".to_string()),
            (2.5,   "1:2.5".to_string()),
            (5.0,   "1:5".to_string()),
            (10.0,  "1:10".to_string()),
        ],
        2 | 3 => vec![
            (64.0,  "64ms".to_string()),
            (128.0, "128ms".to_string()),
            (256.0, "256ms".to_string()),
            (512.0, "512ms".to_string()),
        ],
        4 => vec![
            (3.0,  "3dB".to_string()),
            (6.0,  "6dB".to_string()),
            (12.0, "12dB".to_string()),
            (24.0, "24dB".to_string()),
        ],
        5 => vec![
            (-24.0, "-24dB".to_string()),
            (-12.0, "-12dB".to_string()),
            (0.0,   "0dB".to_string()),
            (12.0,  "+12dB".to_string()),
            (24.0,  "+24dB".to_string()),
        ],
        6 => vec![
            (20.0,  "20%".to_string()),
            (40.0,  "40%".to_string()),
            (60.0,  "60%".to_string()),
            (80.0,  "80%".to_string()),
        ],
        _ => vec![],
    }
}

/// Paint background grid: vertical Hz lines + curve-specific horizontal lines.
/// `sample_rate` is used to extend the grid beyond 20 kHz at high sample rates.
pub fn paint_grid(painter: &Painter, rect: Rect, curve_idx: usize, db_min: f32, db_max: f32, sample_rate: f32) {
    let nyquist = sample_rate / 2.0;
    let max_hz  = nyquist.max(20_001.0);
    let grid_stroke = Stroke::new(th::STROKE_THIN, th::GRID_LINE);
    let font = nih_plug_egui::egui::FontId::proportional(9.0);

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
    if sample_rate > 48_000.0 {
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
    // Extra label at Nyquist for high SR
    if sample_rate > 48_000.0 {
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

    // Horizontal lines per curve type
    for (v, label) in curve_grid_lines(curve_idx, db_min, db_max) {
        let y = physical_to_y(v, curve_idx, db_min, db_max, rect);
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
    freq_scale: f32,
    sample_rate: f32,
    fft_size: usize,
) {
    if gains.len() < 2 { return; }
    let n = gains.len();
    let max_hz = (sample_rate / 2.0).max(20_001.0);

    // True-time grey line for attack/release (drawn first, under the coloured line)
    if curve_idx == 2 || curve_idx == 3 {
        let global_ms = if curve_idx == 2 { global_attack_ms } else { global_release_ms };
        let grey_pts: Vec<Pos2> = (0..n).map(|k| {
            let f_hz = (k as f32 * sample_rate / fft_size as f32).max(20.0);
            let x    = freq_to_x_max(f_hz, max_hz, rect);
            let v    = gain_to_true_time(curve_idx, gains[k], global_ms, freq_scale, k, sample_rate, fft_size);
            let y    = physical_to_y(v, curve_idx, db_min, db_max, rect);
            Pos2::new(x, y)
        }).collect();
        painter.add(Shape::line(grey_pts, Stroke::new(th::STROKE_CURVE, th::TRUE_TIME_LINE)));
    }

    // Coloured response line
    let pts: Vec<Pos2> = (0..n).map(|k| {
        let f_hz = (k as f32 * sample_rate / fft_size as f32).max(20.0);
        let x    = freq_to_x_max(f_hz, max_hz, rect);
        let v    = gain_to_display(curve_idx, gains[k], global_attack_ms, global_release_ms, db_min, db_max);
        let y    = physical_to_y(v, curve_idx, db_min, db_max, rect);
        Pos2::new(x, y)
    }).collect();
    painter.add(Shape::line(pts, Stroke::new(stroke_width, color)));
}

// ─── Interactive widget ───────────────────────────────────────────────────────

/// Draw interactive nodes for the active curve. Returns true if any node changed.
/// Node handles are drawn at the physical y position of the curve (not normalised space),
/// so they sit on top of the curve line. Node handles are shifted left by half a node
/// radius so the right edge of the handle visually marks the affected frequency.
pub fn curve_widget(
    ui: &mut Ui,
    rect: Rect,
    nodes: &mut [CurveNode; 6],
    gains: &[f32],           // pre-computed gains for this curve (display-resolution)
    curve_idx: usize,
    db_min: f32,
    db_max: f32,
    global_attack_ms: f32,
    global_release_ms: f32,
    sample_rate: f32,
    fft_size: usize,
) -> bool {
    use nih_plug_egui::egui::Sense;

    let max_hz = (sample_rate / 2.0).max(20_001.0);
    let mut changed = false;
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
        // Physical y position: look up the gain at the node's frequency bin,
        // convert to physical units, then to screen y. This places the handle
        // directly on the curve line rather than in normalised space.
        let freq_hz = 20.0 * 1000.0_f32.powf(nodes[i].x);
        let bin_k = ((freq_hz / sample_rate) * fft_size as f32).round() as usize;
        let bin_k = bin_k.clamp(0, gains.len().saturating_sub(1));
        let physical = gain_to_display(
            curve_idx, gains[bin_k], global_attack_ms, global_release_ms, db_min, db_max,
        );
        let sy = physical_to_y(physical, curve_idx, db_min, db_max, rect);

        // Visual position scaled to the current SR's Nyquist range
        let sx_actual = x_to_screen(nodes[i].x, rect, max_hz);
        let sx_draw   = sx_actual - th::NODE_RADIUS * 0.5;
        let node_pos  = Pos2::new(sx_actual, sy);
        let draw_pos  = Pos2::new(sx_draw,  sy);

        let node_rect = Rect::from_center_size(node_pos, Vec2::splat(th::NODE_RADIUS * 3.0));
        let resp = ui.interact(node_rect, ui.id().with(("node", i)), Sense::drag());

        if resp.dragged() {
            let delta = resp.drag_delta();
            nodes[i].x = (nodes[i].x + delta.x / rect.width()).clamp(0.0, 1.0);
            nodes[i].y = (nodes[i].y - (delta.y / rect.height()) * 2.0).clamp(-1.0, 1.0);
            changed = true;
        }

        // Smooth scroll wheel Q adjustment (0.002 per pixel — much smoother than before)
        let scroll = ui.input(|inp| {
            if node_rect.contains(inp.pointer.hover_pos().unwrap_or(Pos2::ZERO)) {
                inp.raw_scroll_delta.y
            } else {
                0.0
            }
        });
        if scroll.abs() > 0.01 {
            nodes[i].q = (nodes[i].q + scroll * 0.002).clamp(0.0, 1.0);
            changed = true;
        }

        if resp.double_clicked() {
            nodes[i] = default_nodes()[i];
            changed = true;
        }

        let color = if resp.hovered() { node_color_hover } else { node_color_lit };
        ui.painter().circle_filled(draw_pos, th::NODE_RADIUS, color);
        ui.painter().circle_stroke(
            draw_pos,
            th::NODE_RADIUS,
            Stroke::new(th::STROKE_BORDER, th::BORDER),
        );
    }

    changed
}
