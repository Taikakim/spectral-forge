use nih_plug_egui::egui::{self, Pos2, Rect, Stroke, StrokeKind, Ui, UiBuilder, Vec2};
use crate::dsp::modules::{module_spec, ModuleType, RouteMatrix};
use crate::editor::theme as th;

const CELL: f32  = 44.0;
const LABEL: f32 = 52.0;

pub struct MatrixInteraction {
    pub left_click_slot:  Option<usize>,
    pub right_click:      Option<(usize, Pos2)>,
}

/// Convert a slot_name bytes ([u8; 32]) to a display String.
pub fn slot_name_str(bytes: &[u8; 32]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(32);
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

/// Paint the 9×9 routing matrix grid.
///
/// Returns `MatrixInteraction` describing any clicks this frame.
pub fn paint_fx_matrix_grid(
    ui:           &mut Ui,
    module_types: &[ModuleType; 9],
    slot_names:   &[[u8; 32]; 9],
    route_matrix: &mut RouteMatrix,
    editing_slot: usize,
) -> MatrixInteraction {
    // Column header height
    const HDR: f32 = 14.0;
    let n = 9usize;
    let total_w = LABEL + n as f32 * CELL;
    let total_h = HDR + n as f32 * CELL;

    let (outer_resp, painter) =
        ui.allocate_painter(Vec2::new(total_w, total_h), egui::Sense::hover());
    let origin = outer_resp.rect.min;

    let mut result = MatrixInteraction { left_click_slot: None, right_click: None };

    // Column headers
    for col in 0..n {
        let ty = module_types[col];
        let name = if col == 8 {
            "OUT".to_string()
        } else {
            let s = slot_name_str(&slot_names[col]);
            if s.chars().count() > 4 { s.chars().take(4).collect::<String>() + "\u{2026}" } else { s }
        };
        let spec = module_spec(ty);
        let hdr_rect = Rect::from_min_size(
            origin + egui::vec2(LABEL + col as f32 * CELL, 0.0),
            Vec2::new(CELL - 1.0, HDR),
        );
        painter.text(
            hdr_rect.center(),
            egui::Align2::CENTER_CENTER,
            &name,
            egui::FontId::proportional(7.5),
            if ty == ModuleType::Empty { th::LABEL_DIM } else { spec.color_lit },
        );
    }

    for row in 0..n {
        let ty_row = module_types[row];
        let spec_row = module_spec(ty_row);
        let row_top = origin.y + HDR + row as f32 * CELL;

        // Row label
        let name = slot_name_str(&slot_names[row]);
        let display_name: String = if name.chars().count() > 7 {
            name.chars().take(6).collect::<String>() + "\u{2026}"
        } else {
            name
        };
        let label_rect = Rect::from_min_size(
            egui::pos2(origin.x, row_top),
            Vec2::new(LABEL - 2.0, CELL),
        );
        painter.text(
            label_rect.center(),
            egui::Align2::CENTER_CENTER,
            &display_name,
            egui::FontId::proportional(8.5),
            if ty_row == ModuleType::Empty { th::LABEL_DIM } else { spec_row.color_lit },
        );

        for col in 0..n {
            let cell_rect = Rect::from_min_size(
                egui::pos2(origin.x + LABEL + col as f32 * CELL, row_top),
                Vec2::new(CELL - 1.0, CELL - 1.0),
            );

            if row == col {
                // Diagonal: module cell
                let is_selected = row == editing_slot;
                let ty = module_types[row];
                let spec = module_spec(ty);
                let is_master = row == 8;

                let fill = if is_master {
                    spec.color_dim
                } else if ty == ModuleType::Empty {
                    th::BG_RAISED
                } else if is_selected {
                    spec.color_lit
                } else {
                    spec.color_dim
                };
                let stroke = if is_selected {
                    Stroke::new(1.5, th::BORDER)
                } else {
                    Stroke::new(0.5, th::GRID_LINE)
                };
                painter.rect(cell_rect, 2.0, fill, stroke, StrokeKind::Middle);

                let label_str = if is_master {
                    "OUT".to_string()
                } else if ty == ModuleType::Empty {
                    "+".to_string()
                } else {
                    let slot_label = slot_name_str(&slot_names[row]);
                    if slot_label.chars().count() > 6 { slot_label.chars().take(5).collect::<String>() + "\u{2026}" } else { slot_label }
                };
                let text_col = if is_master {
                    spec.color_lit
                } else if ty == ModuleType::Empty {
                    th::LABEL_DIM
                } else if is_selected {
                    egui::Color32::BLACK
                } else {
                    spec.color_lit
                };
                painter.text(
                    cell_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    &label_str,
                    egui::FontId::proportional(8.0),
                    text_col,
                );

                let interact = ui.interact(
                    cell_rect,
                    ui.id().with(("mat_diag", row)),
                    egui::Sense::click(),
                );
                if interact.clicked() {
                    result.left_click_slot = Some(row);
                }
                if interact.secondary_clicked() && !is_master {
                    result.right_click = Some((row, interact.interact_pointer_pos()
                        .unwrap_or(cell_rect.center())));
                }
                if is_master {
                    interact.on_hover_text("Master output");
                } else {
                    interact.on_hover_text(slot_name_str(&slot_names[row]));
                }

            } else {
                // Off-diagonal send cell.
                // Upper triangle (col > row) = feedback path; lower = forward.
                let is_feedback = col > row;
                let bg = if is_feedback { th::BG_FEEDBACK } else { th::BG_RAISED };
                painter.rect(cell_rect, 0.0, bg, Stroke::new(0.5, th::GRID_LINE), StrokeKind::Middle);

                // Disable cells where both src and dst are Empty
                let src_ty = module_types[row];
                let dst_ty = module_types[col];
                let both_empty = src_ty == ModuleType::Empty && dst_ty == ModuleType::Empty;

                if !both_empty {
                    let send_val = &mut route_matrix.send[row][col];
                    ui.allocate_new_ui(
                        UiBuilder::new().max_rect(cell_rect.shrink(3.0)),
                        |ui| {
                            ui.add(
                                egui::DragValue::new(send_val)
                                    .range(0.0..=2.0)
                                    .speed(0.005)
                                    .fixed_decimals(2)
                                    .custom_formatter(|v, _| {
                                        if v < 0.005 { "\u{2014}".to_string() }
                                        else { format!("{v:.2}") }
                                    })
                                    .custom_parser(|s| s.parse::<f64>().ok()),
                            );
                        },
                    );
                } else {
                    painter.text(
                        cell_rect.center(),
                        egui::Align2::CENTER_CENTER,
                        "\u{2014}",
                        egui::FontId::proportional(8.0),
                        th::GRID_LINE,
                    );
                }
            }
        }
    }

    result
}
