use nih_plug_egui::egui::{self, Pos2, Rect, Stroke, StrokeKind, Ui, UiBuilder, Vec2};
use crate::dsp::modules::{module_spec, ModuleType, RouteMatrix};
use crate::editor::theme as th;

const CELL: f32      = 44.0;
const HALF_CELL: f32 = CELL / 2.0;
const LABEL: f32     = 52.0;

pub struct MatrixInteraction {
    pub left_click_slot:  Option<usize>,
    pub right_click:      Option<(usize, Pos2)>,
}

/// Convert a slot_name bytes ([u8; 32]) to a display String.
pub fn slot_name_str(bytes: &[u8; 32]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(32);
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

/// Paint the 9×9 routing matrix grid, with optional virtual half-height rows for T/S Split slots.
///
/// Returns `MatrixInteraction` describing any clicks this frame.
pub fn paint_fx_matrix_grid(
    ui:           &mut Ui,
    module_types: &[ModuleType; 9],
    slot_names:   &[[u8; 32]; 9],
    route_matrix: &mut RouteMatrix,
    editing_slot: usize,
) -> MatrixInteraction {
    use crate::dsp::modules::{VirtualRowKind, MAX_SLOTS};

    const HDR: f32 = 14.0;
    let n = 9usize;

    // Build row list: Real rows for each of the 9 slots, plus virtual rows after T/S Split slots
    #[derive(Clone, Copy)]
    enum RowEntry {
        Real(usize),
        Virtual(usize, VirtualRowKind, usize), // parent_slot, kind, matrix_row_idx
    }

    let mut rows: Vec<RowEntry> = Vec::with_capacity(13);
    let mut vrow_idx = 0usize;
    for s in 0..9 {
        rows.push(RowEntry::Real(s));
        if module_types[s] == ModuleType::TransientSustainedSplit {
            rows.push(RowEntry::Virtual(s, VirtualRowKind::Transient, MAX_SLOTS + vrow_idx));
            rows.push(RowEntry::Virtual(s, VirtualRowKind::Sustained, MAX_SLOTS + vrow_idx + 1));
            vrow_idx += 2;
        }
    }

    // Compute total height
    let total_h: f32 = HDR + rows.iter().map(|r| match r {
        RowEntry::Real(_) => CELL,
        RowEntry::Virtual(..) => HALF_CELL,
    }).sum::<f32>();
    let total_w = LABEL + n as f32 * CELL;

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

    // Rows
    let mut y_offset = HDR;
    for row_entry in &rows {
        match row_entry {
            RowEntry::Real(row) => {
                let row = *row;
                let ty_row = module_types[row];
                let spec_row = module_spec(ty_row);
                let row_top = origin.y + y_offset;

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
                        // Off-diagonal send cell
                        let is_feedback = col > row;
                        let bg = if is_feedback { th::BG_FEEDBACK } else { th::BG_RAISED };
                        painter.rect(cell_rect, 0.0, bg, Stroke::new(0.5, th::GRID_LINE), StrokeKind::Middle);

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

                y_offset += CELL;
            }

            RowEntry::Virtual(parent_slot, kind, vrow_src) => {
                let row_top = origin.y + y_offset;
                let border_col = match kind {
                    VirtualRowKind::Transient => egui::Color32::from_rgb(0xe0, 0x70, 0x30),
                    VirtualRowKind::Sustained => egui::Color32::from_rgb(0x30, 0x70, 0xc0),
                };
                // Left border stripe
                painter.rect_filled(
                    Rect::from_min_size(egui::pos2(origin.x, row_top), Vec2::new(3.0, HALF_CELL)),
                    0.0, border_col,
                );
                // Row label
                let vrow_label = format!("{}{}",
                    parent_slot + 1,
                    if matches!(kind, VirtualRowKind::Transient) { "T" } else { "S" }
                );
                let lbl_rect = Rect::from_min_size(
                    egui::pos2(origin.x + 3.0, row_top),
                    Vec2::new(LABEL - 5.0, HALF_CELL),
                );
                painter.text(
                    lbl_rect.center(), egui::Align2::CENTER_CENTER,
                    &vrow_label, egui::FontId::proportional(7.5), border_col,
                );

                for col in 0..n {
                    let cell_rect = Rect::from_min_size(
                        egui::pos2(origin.x + LABEL + col as f32 * CELL, row_top),
                        Vec2::new(CELL - 1.0, HALF_CELL - 1.0),
                    );
                    painter.rect_filled(cell_rect, 0.0, th::BG_RAISED);
                    painter.rect_stroke(cell_rect, 0.0, Stroke::new(0.5, th::GRID_LINE), StrokeKind::Middle);

                    if col == *parent_slot || col == 8 {
                        // Self-send and Master column: show ⊘
                        painter.text(
                            cell_rect.center(), egui::Align2::CENTER_CENTER,
                            "\u{2298}", egui::FontId::proportional(7.0), th::GRID_LINE,
                        );
                    } else {
                        let send_val = &mut route_matrix.send[*vrow_src][col];
                        ui.allocate_new_ui(
                            UiBuilder::new().max_rect(cell_rect.shrink(2.0)),
                            |ui| {
                                ui.add(
                                    egui::DragValue::new(send_val)
                                        .range(0.0..=2.0).speed(0.005).fixed_decimals(2)
                                        .custom_formatter(|v, _| {
                                            if v < 0.005 { "\u{2014}".to_string() }
                                            else { format!("{v:.2}") }
                                        })
                                        .custom_parser(|s| s.parse::<f64>().ok()),
                                );
                            },
                        );
                    }
                }

                y_offset += HALF_CELL;
            }
        }
    }

    result
}
