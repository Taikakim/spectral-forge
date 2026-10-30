use nih_plug_egui::egui::{self, Pos2, Rect, Stroke, StrokeKind, Ui, UiBuilder, Vec2};
use crate::dsp::amp_modes::AmpMode;
use crate::dsp::modules::{module_spec, ModuleType, RouteMatrix};
use crate::editor::theme as th;

const CELL: f32      = 44.0;
const HALF_CELL: f32 = CELL / 2.0;
const LABEL: f32     = 52.0;

pub struct MatrixInteraction {
    pub left_click_slot:  Option<usize>,
    pub right_click:      Option<(usize, Pos2)>,
    /// Right-click on a send cell: (matrix_row, col, screen_pos) for the amp popup.
    pub amp_right_click:  Option<(usize, usize, Pos2)>,
}

/// Convert a slot_name bytes ([u8; 32]) to a display String.
pub fn slot_name_str(bytes: &[u8; 32]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(32);
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

/// Format `Slot N: Module` for one matrix endpoint, falling back to "Master"
/// for the master slot and the user-set slot name when present.
fn endpoint_label(slot_idx: usize, ty: ModuleType, slot_name: &[u8; 32]) -> String {
    if slot_idx == 8 {
        return "Master".to_string();
    }
    let custom = slot_name_str(slot_name);
    let module = if ty == ModuleType::Empty { "Empty" } else { module_spec(ty).display_name };
    if custom.is_empty() {
        format!("Slot {}: {}", slot_idx + 1, module)
    } else {
        format!("Slot {}: {} ({})", slot_idx + 1, module, custom)
    }
}

/// Build the dynamic help-box body for a routing-matrix send cell.
///
/// When `is_feedback` is true the caller passes the body through
/// `track_help_strings_yellow` with a "Feedback" prefix, so the body itself
/// starts with a lowercase "routing"; the heading already carries the
/// from→to slot info, so no Flow line is needed.
fn build_cell_help_body(
    amp_mode:      AmpMode,
    is_feedback:   bool,
    disconnected:  bool,
) -> String {
    let mut body = String::new();
    if is_feedback {
        body.push_str("routing matrix send. Drag to set the amplitude (0 = off, 1 = unity, up to 2). Right-click for the amp filter popup. The audio engine processes sends as forward-only (source slot before destination), so this cell is silent.");
    } else {
        body.push_str("Routing matrix send. Drag to set the amplitude (0 = off, 1 = unity, up to 2). Right-click for the amp filter popup.");
    }
    body.push_str("\n\n");
    body.push_str(amp_mode.hint());
    if disconnected {
        body.push_str("\n\nDisconnected — one endpoint is Empty, so no audio flows through this send.");
    }
    body
}

/// Paint the 9×9 routing matrix grid, with optional virtual half-height rows for split
/// slots (T/S Split, Harmonic in H/P Split mode).
///
/// Returns `MatrixInteraction` describing any clicks this frame.
///
/// UI parameter contract: see docs/superpowers/specs/2026-04-23-ui-parameter-spec-design.md §4
/// `scale` is the frame-scoped UI scale factor (ctx.pixels_per_point()); all fonts and
/// strokes flow through `th::scaled` / `th::scaled_stroke`.
pub fn paint_fx_matrix_grid(
    ui:           &mut Ui,
    setter:       &nih_plug::prelude::ParamSetter,
    params:       &crate::params::SpectralForgeParams,
    module_types: &[ModuleType; 9],
    slot_names:   &[[u8; 32]; 9],
    route_matrix: &mut RouteMatrix,
    editing_slot: usize,
    scale:        f32,
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

    // Same derivation the audio thread uses, so a virtual row's matrix column
    // always matches the stream it carries.
    let harmonic_modes = *params.slot_harmonic_mode.lock();
    let vrows = crate::dsp::modules::split_virtual_rows(module_types, &harmonic_modes);
    let mut rows: Vec<RowEntry> = Vec::with_capacity(13);
    for s in 0..9 {
        rows.push(RowEntry::Real(s));
        for (v, vrow) in vrows.iter().enumerate() {
            if let Some((src, kind)) = *vrow {
                if src as usize == s {
                    rows.push(RowEntry::Virtual(s, kind, MAX_SLOTS + v));
                }
            }
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

    let mut result = MatrixInteraction {
        left_click_slot: None,
        right_click: None,
        amp_right_click: None,
    };

    // Column headers
    for col in 0..n {
        let ty = module_types[col];
        let name = if col == 8 {
            "OUT".to_string()
        } else if ty == ModuleType::Empty {
            String::new()
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
            egui::FontId::proportional(th::scaled(th::FONT_SIZE_MATRIX_AXIS, scale)),
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
                let display_name: String = if ty_row == ModuleType::Empty {
                    String::new()
                } else {
                    let name = slot_name_str(&slot_names[row]);
                    if name.chars().count() > 7 {
                        name.chars().take(6).collect::<String>() + "\u{2026}"
                    } else {
                        name
                    }
                };
                let label_rect = Rect::from_min_size(
                    egui::pos2(origin.x, row_top),
                    Vec2::new(LABEL - 2.0, CELL),
                );
                painter.text(
                    label_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    &display_name,
                    egui::FontId::proportional(th::scaled(th::FONT_SIZE_MATRIX_ROW, scale)),
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
                            Stroke::new(th::scaled_stroke(th::STROKE_MEDIUM, scale), th::BORDER)
                        } else {
                            Stroke::new(th::scaled_stroke(th::STROKE_HAIRLINE, scale), th::GRID_LINE)
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
                            egui::FontId::proportional(th::scaled(th::FONT_SIZE_MATRIX_CELL, scale)),
                            text_col,
                        );

                        let interact = ui.interact(
                            cell_rect,
                            ui.id().with(("mat_diag", row)),
                            egui::Sense::click(),
                        );
                        crate::editor::help_box::track_help(
                            ui, &interact, crate::editor::help_box::HelpTopic::MatrixSlotSelect,
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
                        // The audio engine processes sends as forward-only
                        // (`route_matrix.send[src][dst]` only takes effect
                        // when src < dst — see fx_matrix.rs:199). Cells where
                        // `col < row` (dst earlier than src) are therefore
                        // feedback routings — silent in this engine — so we
                        // dim their background and prefix their help text
                        // with a yellow "Feedback" word so the user sees at
                        // a glance that the cell is non-audible.
                        let is_feedback = col < row;
                        let bg = if is_feedback { th::BG_FEEDBACK } else { th::BG_RAISED };
                        painter.rect(cell_rect, 0.0, bg, Stroke::new(th::scaled_stroke(th::STROKE_HAIRLINE, scale), th::GRID_LINE), StrokeKind::Middle);

                        let src_ty = module_types[row];
                        let dst_ty = module_types[col];
                        let both_empty = src_ty == ModuleType::Empty && dst_ty == ModuleType::Empty;

                        if !both_empty {
                            // Read current value from the FloatParam (the canonical store the
                            // audio thread reads). Falls back to 0.0 if out of range (shouldn't
                            // happen for valid in-range cells).
                            let p = params.matrix_cell(col, row);
                            let mut send_val: f32 = p.map(|fp| fp.value()).unwrap_or(0.0);
                            // Disconnected cell: configured send level but one side
                            // missing. Render the DragValue with dimmed text so the
                            // user can see at a glance that the send isn't audible.
                            let active = send_val >= 0.005;
                            let disconnected = active && (src_ty == ModuleType::Empty || dst_ty == ModuleType::Empty);
                            let inner = ui.allocate_new_ui(
                                UiBuilder::new().max_rect(cell_rect.shrink(3.0)),
                                |ui| {
                                    if disconnected {
                                        let v = &mut ui.style_mut().visuals;
                                        v.override_text_color = Some(th::LABEL_DIM);
                                    }
                                    let resp = ui.add(
                                        egui::DragValue::new(&mut send_val)
                                            .range(0.0..=2.0)
                                            .speed(0.005)
                                            .fixed_decimals(2)
                                            .custom_formatter(|v, _| {
                                                if v < 0.005 { "\u{2014}".to_string() }
                                                else { format!("{v:.2}") }
                                            })
                                            .custom_parser(|s| s.parse::<f64>().ok()),
                                    );
                                    // Write back to the FloatParam via the setter so the audio
                                    // thread sees the change. Also mirror into
                                    // route_matrix.send[row][col] for within-frame visual
                                    // consistency (the next frame will re-read via fp.value()).
                                    if resp.drag_started() {
                                        if let Some(fp) = p { setter.begin_set_parameter(fp); }
                                    }
                                    if resp.changed() {
                                        if let Some(fp) = p { setter.set_parameter(fp, send_val); }
                                        route_matrix.send[row][col] = send_val;
                                    }
                                    if resp.drag_stopped() {
                                        if let Some(fp) = p { setter.end_set_parameter(fp); }
                                    }
                                    resp
                                },
                            );
                            crate::editor::delayed_tooltip(ui, &inner.inner,
                                format!("Slot {} -> Slot {} send", row + 1, col + 1));
                            // Amp-mode indicator dot (top-right corner) when non-Linear.
                            let amp_mode = route_matrix.amp_mode[row][col];
                            // Heading carries the from/to slot info (with custom
                            // names where set) and the active amp filter; body is
                            // the static "Routing matrix send" sentence + amp
                            // hint, with a yellow "Feedback" prefix when the cell
                            // is below the diagonal. Disconnected sends append a
                            // one-liner.
                            let head = format!(
                                "{}  ->  {}  ({})",
                                endpoint_label(row, src_ty, &slot_names[row]),
                                endpoint_label(col, dst_ty, &slot_names[col]),
                                amp_mode.label(),
                            );
                            let body = build_cell_help_body(amp_mode, is_feedback, disconnected);
                            if is_feedback {
                                crate::editor::help_box::track_help_strings_yellow(
                                    ui, &inner.inner, head.clone(), body.clone(), "Feedback",
                                );
                            } else {
                                crate::editor::help_box::track_help_strings(
                                    ui, &inner.inner, head.clone(), body.clone(),
                                );
                            }
                            if amp_mode != AmpMode::Linear {
                                let dot_pos = egui::pos2(cell_rect.right() - 4.0, cell_rect.top() + 4.0);
                                ui.painter().circle_filled(
                                    dot_pos,
                                    th::AMP_DOT_RADIUS,
                                    th::AMP_DOT_COLORS[amp_mode as usize],
                                );
                            }
                            // Right-click anywhere in the cell opens the amp popup.
                            let amp_resp = ui.interact(
                                cell_rect,
                                ui.id().with(("amp_cell", row, col)),
                                egui::Sense::click(),
                            );
                            // Hovering anywhere on the cell (including the corner
                            // dot, outside the inner DragValue) shows the same help.
                            if is_feedback {
                                crate::editor::help_box::track_help_strings_yellow(
                                    ui, &amp_resp, head, body, "Feedback",
                                );
                            } else {
                                crate::editor::help_box::track_help_strings(
                                    ui, &amp_resp, head, body,
                                );
                            }
                            if amp_resp.secondary_clicked() {
                                let p = amp_resp.interact_pointer_pos().unwrap_or(cell_rect.center());
                                result.amp_right_click = Some((row, col, p));
                            }
                        } else {
                            painter.text(
                                cell_rect.center(),
                                egui::Align2::CENTER_CENTER,
                                "\u{2014}",
                                egui::FontId::proportional(th::scaled(th::FONT_SIZE_MATRIX_CELL, scale)),
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
                    VirtualRowKind::Transient | VirtualRowKind::Percussive =>
                        egui::Color32::from_rgb(0xe0, 0x70, 0x30),
                    VirtualRowKind::Sustained | VirtualRowKind::Harmonic =>
                        egui::Color32::from_rgb(0x30, 0x70, 0xc0),
                };
                // Left border stripe
                painter.rect_filled(
                    Rect::from_min_size(egui::pos2(origin.x, row_top), Vec2::new(3.0, HALF_CELL)),
                    0.0, border_col,
                );
                // Row label
                let vrow_label = format!("{}{}", parent_slot + 1, kind.suffix());
                let lbl_rect = Rect::from_min_size(
                    egui::pos2(origin.x + 3.0, row_top),
                    Vec2::new(LABEL - 5.0, HALF_CELL),
                );
                painter.text(
                    lbl_rect.center(), egui::Align2::CENTER_CENTER,
                    &vrow_label, egui::FontId::proportional(th::scaled(th::FONT_SIZE_MATRIX_AXIS, scale)), border_col,
                );

                for col in 0..n {
                    let cell_rect = Rect::from_min_size(
                        egui::pos2(origin.x + LABEL + col as f32 * CELL, row_top),
                        Vec2::new(CELL - 1.0, HALF_CELL - 1.0),
                    );
                    painter.rect_filled(cell_rect, 0.0, th::BG_RAISED);
                    painter.rect_stroke(cell_rect, 0.0, Stroke::new(th::scaled_stroke(th::STROKE_HAIRLINE, scale), th::GRID_LINE), StrokeKind::Middle);

                    if col == *parent_slot || col == 8 {
                        // Self-send and Master column: show ⊘
                        painter.text(
                            cell_rect.center(), egui::Align2::CENTER_CENTER,
                            "\u{2298}", egui::FontId::proportional(th::scaled(th::FONT_SIZE_MATRIX_VROW, scale)), th::GRID_LINE,
                        );
                    } else {
                        // 2026-05-08: virtual-row sources now have FloatParams.
                        // `NUM_MATRIX_SOURCES = 13` covers the 4 virtual rows at
                        // `MAX_SLOTS..MAX_SLOTS+MAX_SPLIT_VIRTUAL_ROWS`, so
                        // `matrix_cell(col=dst_slot, *vrow_src)` returns Some
                        // and the setter path persists virtual-row sends.
                        let p = params.matrix_cell(col, *vrow_src);
                        let mut send_val: f32 = p.map(|fp| fp.value())
                            .unwrap_or(route_matrix.send[*vrow_src][col]);
                        let vrow_label = format!("Slot {}{}", parent_slot + 1, kind.suffix());
                        let vinner = ui.allocate_new_ui(
                            UiBuilder::new().max_rect(cell_rect.shrink(2.0)),
                            |ui| {
                                let resp = ui.add(
                                    egui::DragValue::new(&mut send_val)
                                        .range(0.0..=2.0).speed(0.005).fixed_decimals(2)
                                        .custom_formatter(|v, _| {
                                            if v < 0.005 { "\u{2014}".to_string() }
                                            else { format!("{v:.2}") }
                                        })
                                        .custom_parser(|s| s.parse::<f64>().ok()),
                                );
                                if resp.drag_started() {
                                    if let Some(fp) = p { setter.begin_set_parameter(fp); }
                                }
                                if resp.changed() {
                                    if let Some(fp) = p { setter.set_parameter(fp, send_val); }
                                    route_matrix.send[*vrow_src][col] = send_val;
                                }
                                if resp.drag_stopped() {
                                    if let Some(fp) = p { setter.end_set_parameter(fp); }
                                }
                                resp
                            },
                        );
                        crate::editor::delayed_tooltip(ui, &vinner.inner,
                            format!("{} -> Slot {} send", vrow_label, col + 1));
                        let amp_mode = route_matrix.amp_mode[*vrow_src][col];
                        let dst_ty   = module_types[col];
                        let stream   = kind.stream_name();
                        let head = format!(
                            "{} ({})  ->  {}  ({})",
                            endpoint_label(*parent_slot, module_types[*parent_slot], &slot_names[*parent_slot]),
                            stream,
                            endpoint_label(col, dst_ty, &slot_names[col]),
                            amp_mode.label(),
                        );
                        let mut body = String::new();
                        body.push_str("Split virtual-row send. Drag to set send amplitude for the ");
                        body.push_str(stream);
                        body.push_str(" feeding this slot. Right-click for the amp filter popup.\n\n");
                        body.push_str(amp_mode.hint());
                        crate::editor::help_box::track_help_strings(
                            ui, &vinner.inner, head.clone(), body.clone(),
                        );
                        if amp_mode != AmpMode::Linear {
                            let dot_pos = egui::pos2(cell_rect.right() - 4.0, cell_rect.top() + 3.0);
                            ui.painter().circle_filled(
                                dot_pos,
                                th::AMP_DOT_RADIUS,
                                th::AMP_DOT_COLORS[amp_mode as usize],
                            );
                        }
                        let amp_resp = ui.interact(
                            cell_rect,
                            ui.id().with(("amp_vcell", *vrow_src, col)),
                            egui::Sense::click(),
                        );
                        crate::editor::help_box::track_help_strings(
                            ui, &amp_resp, head, body,
                        );
                        if amp_resp.secondary_clicked() {
                            let p = amp_resp.interact_pointer_pos().unwrap_or(cell_rect.center());
                            result.amp_right_click = Some((*vrow_src, col, p));
                        }
                    }
                }

                y_offset += HALF_CELL;
            }
        }
    }

    result
}
