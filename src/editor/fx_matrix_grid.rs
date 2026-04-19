use nih_plug_egui::egui::{self, Rect, Stroke, StrokeKind, Ui, Vec2};
use crate::editor::theme as th;
use crate::params::FxModuleType;

const CELL: f32  = 48.0;
const LABEL: f32 = 48.0;

/// Paint the 8×8 routing matrix grid.
///
/// Returns the slot index of a diagonal module cell that was clicked (if any),
/// so the caller can update `editing_slot`.
pub fn paint_fx_matrix_grid(
    ui: &mut Ui,
    module_types:  &[FxModuleType; 8],
    module_names:  &[String; 8],
    send_matrix:   &mut [[f32; 8]; 8],
    editing_slot:  usize,
) -> Option<usize> {
    let total_w = LABEL + 8.0 * CELL;
    let total_h = 8.0 * CELL;

    let (outer_resp, painter) =
        ui.allocate_painter(Vec2::new(total_w, total_h), egui::Sense::hover());
    let origin = outer_resp.rect.min;

    let mut clicked_slot: Option<usize> = None;

    for row in 0..8usize {
        // Row label (left column)
        let label_rect = Rect::from_min_size(
            origin + egui::vec2(0.0, row as f32 * CELL),
            Vec2::new(LABEL - 2.0, CELL),
        );
        painter.text(
            label_rect.center(),
            egui::Align2::CENTER_CENTER,
            &module_names[row],
            egui::FontId::proportional(9.0),
            th::LABEL_DIM,
        );

        for col in 0..8usize {
            let cell_rect = Rect::from_min_size(
                origin + egui::vec2(LABEL + col as f32 * CELL, row as f32 * CELL),
                Vec2::new(CELL - 1.0, CELL - 1.0),
            );

            if row == col {
                // Diagonal: module cell
                let is_selected = row == editing_slot;
                let fill = match (module_types[row], is_selected) {
                    (FxModuleType::Empty, _)        => th::BG_RAISED,
                    (FxModuleType::Dynamics, true)  => th::MODULE_COLOR_LIT,
                    (FxModuleType::Dynamics, false) => th::MODULE_COLOR_DIM,
                };
                let stroke = if is_selected {
                    Stroke::new(1.5, th::BORDER)
                } else {
                    Stroke::new(0.5, th::GRID_LINE)
                };
                painter.rect(cell_rect, 2.0, fill, stroke, StrokeKind::Middle);

                let (label_str, text_color) = match module_types[row] {
                    FxModuleType::Empty    => ("+", th::LABEL_DIM),
                    FxModuleType::Dynamics => (module_names[row].as_str(),
                        if is_selected { th::BG } else { th::LABEL_DIM }),
                };
                painter.text(
                    cell_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    label_str,
                    egui::FontId::proportional(9.0),
                    text_color,
                );

                let interact = ui.interact(
                    cell_rect,
                    ui.id().with(("mat_diag", row)),
                    egui::Sense::click(),
                );
                if interact.clicked() {
                    clicked_slot = Some(row);
                }
            } else {
                // Off-diagonal: send amount DragValue
                let is_feedback = col > row; // upper triangle = feedback
                let bg = if is_feedback { th::BG_FEEDBACK } else { th::BG_RAISED };
                painter.rect(cell_rect, 0.0, bg, Stroke::new(0.5, th::GRID_LINE), StrokeKind::Middle);

                let send_val = &mut send_matrix[col][row];
                ui.allocate_ui_at_rect(cell_rect.shrink(4.0), |ui| {
                    ui.add(
                        egui::DragValue::new(send_val)
                            .range(0.0..=1.0)
                            .speed(0.005)
                            .fixed_decimals(2)
                            .custom_formatter(|v, _| {
                                if v < 0.005 { "\u{2014}".to_string() } else { format!("{v:.2}") }
                            })
                            .custom_parser(|s| s.parse::<f64>().ok()),
                    );
                });
            }
        }
    }

    clicked_slot
}
