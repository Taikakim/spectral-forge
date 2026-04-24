use nih_plug_egui::egui::{Color32, Mesh, Painter, Pos2, Rect, Shape, Stroke};
use crate::editor::theme as th;

/// Map an FFT bin index to a log-scaled x position using the same formula as
/// `freq_to_x_max` so the spectrum aligns with the grid and response curves.
#[inline]
fn bin_x(k: usize, sample_rate: f32, fft_size: usize, rect: Rect) -> f32 {
    let nyquist = sample_rate / 2.0;
    let max_hz  = nyquist.max(20_001.0);
    let f_hz    = (k as f32 * sample_rate / fft_size as f32).max(20.0).min(max_hz);
    let t       = (f_hz / 20.0).log10() / (max_hz / 20.0).log10();
    rect.left() + t * rect.width()
}

/// Map a dB value to a y position in `rect` (higher dB = higher on screen).
#[inline]
fn db_y(db: f32, db_min: f32, db_max: f32, rect: Rect) -> f32 {
    let t = ((db - db_min) / (db_max - db_min)).clamp(0.0, 1.0);
    rect.bottom() - t * rect.height()
}

/// Paint the pre-FX spectrum peak line (turquoise) and post-FX output line (pink),
/// with a gradient fill in between showing gain reduction.
///
/// - `magnitudes`  — peak-held linear input magnitudes per FFT bin.
/// - `suppression` — gain reduction per bin in dB (≥ 0).
/// - `db_min/max`  — vertical display range from graph settings.
/// - `sidechain_active` — if true, overlays a second SC-coloured gradient layer.
///
/// UI parameter contract: see docs/superpowers/specs/2026-04-23-ui-parameter-spec-design.md
pub fn paint_spectrum_and_suppression(
    painter: &Painter,
    rect: Rect,
    magnitudes: &[f32],
    suppression: &[f32],
    db_min: f32,
    db_max: f32,
    sidechain_active: bool,
    sample_rate: f32,
    fft_size: usize,
) {
    let n = magnitudes.len();
    if n < 2 { return; }

    let mut pre_pts:  Vec<Pos2> = Vec::with_capacity(n);
    let mut post_pts: Vec<Pos2> = Vec::with_capacity(n);

    for k in 0..n {
        let x = bin_x(k, sample_rate, fft_size, rect);
        let mag    = magnitudes[k];
        let pre_db = if mag > 1e-10 { 20.0 * mag.log10() } else { db_min - 1.0 };
        let gr_db  = suppression.get(k).copied().unwrap_or(0.0).max(0.0);
        let post_db = pre_db - gr_db;
        pre_pts.push( Pos2::new(x, db_y(pre_db,  db_min, db_max, rect)) );
        post_pts.push(Pos2::new(x, db_y(post_db, db_min, db_max, rect)) );
    }

    // Gradient fill between the two lines (turquoise top → pink bottom)
    painter.add(Shape::mesh(build_gradient_mesh(&pre_pts, &post_pts, th::SPECTRUM_LINE, th::POSTFX_LINE)));

    // Sidechain overlay (semi-transparent second gradient)
    if sidechain_active {
        let ca = Color32::from_rgba_unmultiplied(th::SC_LINE_A.r(), th::SC_LINE_A.g(), th::SC_LINE_A.b(), 120);
        let cb = Color32::from_rgba_unmultiplied(th::SC_LINE_B.r(), th::SC_LINE_B.g(), th::SC_LINE_B.b(), 120);
        painter.add(Shape::mesh(build_gradient_mesh(&pre_pts, &post_pts, ca, cb)));
    }

    // Post-FX line (1px pink) then pre-FX line (1px turquoise) on top
    painter.add(Shape::line(post_pts, Stroke::new(th::STROKE_CURVE, th::POSTFX_LINE)));
    painter.add(Shape::line(pre_pts,  Stroke::new(th::STROKE_CURVE, th::SPECTRUM_LINE)));
}

fn build_gradient_mesh(top: &[Pos2], bot: &[Pos2], col_top: Color32, col_bot: Color32) -> Mesh {
    let mut mesh = Mesh::default();
    for k in 0..top.len().saturating_sub(1) {
        let base = mesh.vertices.len() as u32;
        mesh.colored_vertex(top[k],     col_top);
        mesh.colored_vertex(top[k + 1], col_top);
        mesh.colored_vertex(bot[k + 1], col_bot);
        mesh.colored_vertex(bot[k],     col_bot);
        mesh.add_triangle(base, base + 1, base + 2);
        mesh.add_triangle(base, base + 2, base + 3);
    }
    mesh
}

/// Apply peak-hold decay to a dB buffer in place.
/// `dt_s` — frame delta time in seconds (use ~1.0/60.0 if unknown).
pub fn decay_peak_hold(magnitudes: &[f32], hold_db: &mut Vec<f32>, falloff_ms: f32, dt_s: f32) {
    if hold_db.len() != magnitudes.len() {
        hold_db.resize(magnitudes.len(), f32::NEG_INFINITY);
    }
    // Rate: enough to drop ~60 dB over falloff_ms
    let drop = if falloff_ms < 1.0 { f32::INFINITY } else { 60.0 / (falloff_ms * 0.001) * dt_s };
    for (h, &mag) in hold_db.iter_mut().zip(magnitudes.iter()) {
        let db = if mag > 1e-10 { 20.0 * mag.log10() } else { -120.0 };
        if db >= *h { *h = db; } else { *h = (*h - drop).max(db); }
    }
}

/// Convert a peak-hold dB buffer to linear magnitudes for paint_spectrum_and_suppression.
pub fn hold_to_linear(hold_db: &[f32]) -> Vec<f32> {
    hold_db.iter().map(|&db| 10.0f32.powf(db / 20.0)).collect()
}
