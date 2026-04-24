use nih_plug_egui::egui::{Rect, Painter, Pos2};
use crate::editor::theme as th;

/// Paint stalactite suppression bars hanging from the top.
/// `suppression`: gain reduction magnitude in dB per bin (>= 0).
///
/// UI parameter contract: see docs/superpowers/specs/2026-04-23-ui-parameter-spec-design.md
pub fn paint_suppression(painter: &Painter, rect: Rect, suppression: &[f32]) {
    if suppression.is_empty() { return; }
    let n = suppression.len();
    let bar_width = rect.width() / n as f32;
    let max_db = 24.0f32;

    for k in 0..n {
        let x_norm = k as f32 / n as f32;
        let x = rect.left() + x_norm * rect.width();
        let depth_norm = (suppression[k] / max_db).clamp(0.0, 1.0);
        if depth_norm < 0.001 { continue; }
        let bar_height = depth_norm * rect.height() * 0.3;
        let color = th::magnitude_color(depth_norm);
        painter.rect_filled(
            nih_plug_egui::egui::Rect::from_min_max(
                Pos2::new(x, rect.top()),
                Pos2::new(x + bar_width.max(1.0), rect.top() + bar_height),
            ),
            0.0,
            color,
        );
    }
}
