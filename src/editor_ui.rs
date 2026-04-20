use nih_plug::prelude::*;
use nih_plug_egui::{create_egui_editor, egui};
use parking_lot::Mutex;
use triple_buffer::Input as TbInput;
use std::sync::{Arc, atomic::Ordering};
use crate::params::SpectralForgeParams;
use crate::editor::{curve as crv, spectrum_display as sd, theme as th};


pub fn create_editor(
    params: Arc<SpectralForgeParams>,
    curve_tx: Vec<Vec<Arc<Mutex<TbInput<Vec<f32>>>>>>,
    sample_rate: Option<Arc<crate::bridge::AtomicF32>>,
    fft_size_arc: Arc<std::sync::atomic::AtomicUsize>,
    spectrum_rx: Option<Arc<parking_lot::Mutex<triple_buffer::Output<Vec<f32>>>>>,
    suppression_rx: Option<Arc<parking_lot::Mutex<triple_buffer::Output<Vec<f32>>>>>,
    sidechain_active: Option<[Arc<std::sync::atomic::AtomicBool>; 4]>,
    plugin_alive: std::sync::Weak<()>,
) -> Option<Box<dyn Editor>> {
    create_egui_editor(
        params.editor_state.clone(),
        (),
        |ctx, _| {
            let mut visuals = egui::Visuals::dark();
            visuals.panel_fill = th::BG;
            ctx.set_visuals(visuals);
        },
        move |ctx, setter, _state| {
            if plugin_alive.upgrade().is_none() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                return;
            }
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE.fill(th::BG))
                .show(ctx, |ui| {
                    let fft_size     = fft_size_arc.load(Ordering::Relaxed).max(512);
                    let num_bins     = fft_size / 2 + 1;
                    let sr           = sample_rate.as_ref().map(|a| a.load()).unwrap_or(44100.0);
                    let db_min       = *params.graph_db_min.lock();
                    let db_max       = *params.graph_db_max.lock();
                    let falloff      = *params.peak_falloff_ms.lock();
                    let atk_ms       = params.attack_ms.value();
                    let rel_ms       = params.release_ms.value();
                    let sc_active: [bool; 4] = match &sidechain_active {
                        Some(arcs) => std::array::from_fn(|i| arcs[i].load(std::sync::atomic::Ordering::Relaxed)),
                        None => [false; 4],
                    };

                    // ── Top bar: curve selectors + range controls ──────────────
                    ui.horizontal(|ui| {
                        ui.add_space(4.0);

                        let editing_slot = *params.editing_slot.lock() as usize;
                        let slot_types   = *params.slot_module_types.lock();
                        let editing_type = slot_types[editing_slot];
                        let spec         = crate::dsp::modules::module_spec(editing_type);
                        let mut editing_curve_raw = *params.editing_curve.lock() as usize;
                        if spec.num_curves > 0 && editing_curve_raw >= spec.num_curves {
                            editing_curve_raw = 0;
                            *params.editing_curve.lock() = 0u8;
                        }
                        let editing_curve = editing_curve_raw;

                        // Adaptive curve selector buttons
                        for (i, &label) in spec.curve_labels.iter().enumerate() {
                            let is_active = editing_curve == i;
                            let (fill, text_color, stroke_color) = if is_active {
                                (spec.color_lit,
                                 egui::Color32::BLACK,
                                 spec.color_lit)
                            } else {
                                (spec.color_dim,
                                 spec.color_lit,
                                 spec.color_dim)
                            };
                            let btn = egui::Button::new(
                                egui::RichText::new(label).color(text_color).size(11.0),
                            )
                            .fill(fill)
                            .stroke(egui::Stroke::new(th::STROKE_BORDER, stroke_color));
                            if ui.add(btn).clicked() {
                                *params.editing_curve.lock() = i as u8;
                            }
                        }

                        if spec.num_curves > 0 {
                            ui.add_space(8.0);
                            ui.separator();
                            ui.add_space(4.0);
                        }

                        ui.label(egui::RichText::new("Floor").color(th::LABEL_DIM).size(9.0));
                        {
                            let mut v = *params.graph_db_min.lock();
                            if ui.add(
                                egui::DragValue::new(&mut v)
                                    .range(-160.0..=-20.0)
                                    .suffix(" dB").speed(0.5).max_decimals(1),
                            ).changed() {
                                *params.graph_db_min.lock() = v.min(db_max - 6.0);
                            }
                        }
                        ui.add_space(4.0);
                        ui.label(egui::RichText::new("Ceil").color(th::LABEL_DIM).size(9.0));
                        {
                            let mut v = *params.graph_db_max.lock();
                            if ui.add(
                                egui::DragValue::new(&mut v)
                                    .range(-20.0..=0.0)
                                    .suffix(" dB").speed(0.5).max_decimals(1),
                            ).changed() {
                                *params.graph_db_max.lock() = v.max(db_min + 6.0);
                            }
                        }
                        ui.add_space(4.0);
                        ui.label(egui::RichText::new("Falloff").color(th::LABEL_DIM).size(9.0));
                        {
                            let mut v = *params.peak_falloff_ms.lock();
                            if ui.add(
                                egui::DragValue::new(&mut v)
                                    .range(0.0..=5000.0)
                                    .suffix(" ms").speed(10.0).max_decimals(0),
                            ).changed() {
                                *params.peak_falloff_ms.lock() = v;
                            }
                        }
                    });

                    // ── Second bar: FFT size selector ─────────────────────────────
                    ui.horizontal(|ui| {
                        ui.add_space(4.0);
                        ui.label(egui::RichText::new("FFT").color(th::LABEL_DIM).size(9.0));
                        ui.add_space(2.0);

                        use crate::params::FftSizeChoice;
                        const FFT_LABELS: &[(&str, FftSizeChoice)] = &[
                            ("512",  FftSizeChoice::S512),
                            ("1k",   FftSizeChoice::S1024),
                            ("2k",   FftSizeChoice::S2048),
                            ("4k",   FftSizeChoice::S4096),
                            ("8k",   FftSizeChoice::S8192),
                            ("16k",  FftSizeChoice::S16384),
                        ];
                        let cur_choice = params.fft_size.value();
                        for &(label, choice) in FFT_LABELS {
                            let is_active = cur_choice == choice;
                            let (fill, text_color) = if is_active {
                                (th::BORDER, th::BG)
                            } else {
                                (th::BG, th::LABEL_DIM)
                            };
                            let btn = egui::Button::new(
                                egui::RichText::new(label).color(text_color).size(10.0),
                            )
                            .fill(fill)
                            .stroke(egui::Stroke::new(th::STROKE_BORDER, th::BORDER));
                            if ui.add(btn).clicked() {
                                setter.begin_set_parameter(&params.fft_size);
                                setter.set_parameter(&params.fft_size, choice);
                                setter.end_set_parameter(&params.fft_size);
                            }
                        }
                    });

                    ui.add_space(2.0);
                    {
                        let r = ui.available_rect_before_wrap();
                        ui.painter().line_segment(
                            [r.left_top(), r.right_top()],
                            egui::Stroke::new(th::STROKE_BORDER, th::BORDER),
                        );
                    }

                    // ── Spectrum / curve area ─────────────────────────────────────
                    // strip_height reserves space for: control knobs (105) + routing matrix section
                    // (9 × 44px cells + 14px header + 30px padding = 440). The window height (1010) was
                    // set to accommodate all three areas.
                    const MATRIX_AREA_H: f32 = 9.0 * 44.0 + 4.0 * 22.0 + 14.0 + 30.0; // 528 px worst case
                    let strip_height = 105.0 + MATRIX_AREA_H;
                    let avail = ui.available_rect_before_wrap();
                    let curve_rect = egui::Rect::from_min_max(
                        avail.min,
                        egui::pos2(avail.max.x, (avail.max.y - strip_height).max(avail.min.y)),
                    );
                    ui.allocate_rect(curve_rect, egui::Sense::hover());

                    // Read spectrum + suppression from bridge
                    let mut raw_magnitudes: Option<Vec<f32>> = None;
                    let mut suppression_data: Vec<f32> = Vec::new();
                    if let Some(ref rx_arc) = spectrum_rx {
                        if let Some(mut rx) = rx_arc.try_lock() {
                            raw_magnitudes = Some(rx.read()[..num_bins].to_vec());
                        }
                    }
                    if let Some(ref rx_arc) = suppression_rx {
                        if let Some(mut rx) = rx_arc.try_lock() {
                            suppression_data = rx.read()[..num_bins].to_vec();
                        }
                    }

                    // Peak-hold buffer
                    let peak_key = ui.id().with("peak_hold");
                    let mut peak_hold: Vec<f32> = ui.data(|d| d.get_temp(peak_key))
                        .unwrap_or_default();

                    // 1. Grid
                    let grid_curve = *params.editing_curve.lock() as usize;
                    crv::paint_grid(ui.painter(), curve_rect, grid_curve, db_min, db_max, sr);

                    // 2. Spectrum + suppression gradient (always shown)
                    if let Some(ref mags) = raw_magnitudes {
                        let norm = 4.0 / fft_size as f32;
                        let norm_mags: Vec<f32> = mags.iter().map(|m| m * norm).collect();
                        sd::decay_peak_hold(&norm_mags, &mut peak_hold, falloff, 1.0 / 60.0);
                        ui.data_mut(|d| d.insert_temp(peak_key, peak_hold.clone()));
                        let held_linear = sd::hold_to_linear(&peak_hold);
                        sd::paint_spectrum_and_suppression(
                            ui.painter(), curve_rect,
                            &held_linear, &suppression_data,
                            db_min, db_max, false, sr,
                            fft_size,
                        );
                    }

                    // 3 + 4. Response curves + interactive widget (unified — all module types)
                    {
                        let editing_slot  = *params.editing_slot.lock() as usize;
                        let slot_types    = *params.slot_module_types.lock();
                        let editing_type  = slot_types[editing_slot];
                        let spec          = crate::dsp::modules::module_spec(editing_type);
                        let num_c         = spec.num_curves;
                        let raw_curve = *params.editing_curve.lock() as usize;
                        let editing_curve = if raw_curve >= num_c && num_c > 0 {
                            *params.editing_curve.lock() = 0;
                            0
                        } else {
                            raw_curve
                        };

                        let nodes_all = *params.slot_curve_nodes.lock();

                        // Cache key: invalidate when slot type, editing slot, or fft_size changes
                        let cache_key = ui.id().with(("slot_gains", editing_slot, editing_type as u8, fft_size));
                        let cached: Option<([[[crv::CurveNode; 6]; 7]; 9], Vec<Vec<f32>>)> =
                            ui.data(|d| d.get_temp(cache_key));
                        let all_gains: Vec<Vec<f32>> = match cached {
                            Some((cn, cg)) if cn == nodes_all => cg,
                            _ => {
                                let g: Vec<Vec<f32>> = (0..num_c.min(7))
                                    .map(|c| crv::compute_curve_response(
                                        &nodes_all[editing_slot][c], num_bins, sr, fft_size,
                                    ))
                                    .collect();
                                ui.data_mut(|d| d.insert_temp(cache_key, (nodes_all, g.clone())));
                                g
                            }
                        };

                        let meta = *params.slot_curve_meta.lock();

                        // Draw inactive curves (dim)
                        for i in 0..num_c.min(7) {
                            if i == editing_curve { continue; }
                            let (tilt, offset) = meta[editing_slot][i];
                            crv::paint_response_curve(
                                ui.painter(), curve_rect, &all_gains[i], i,
                                spec.color_dim, 1.0,
                                db_min, db_max, atk_ms, rel_ms, sr, fft_size, tilt, offset,
                            );
                        }

                        // Draw active curve (lit) + interactive widget
                        if editing_curve < num_c && !all_gains.is_empty() {
                            let (tilt, offset) = meta[editing_slot][editing_curve];
                            crv::paint_response_curve(
                                ui.painter(), curve_rect, &all_gains[editing_curve], editing_curve,
                                spec.color_lit, 2.0,
                                db_min, db_max, atk_ms, rel_ms, sr, fft_size, tilt, offset,
                            );

                            let mut nodes = nodes_all[editing_slot][editing_curve];
                            if crv::curve_widget(
                                ui, curve_rect, &mut nodes, &all_gains[editing_curve],
                                editing_curve, db_min, db_max, atk_ms, rel_ms, sr, fft_size,
                                tilt, offset,
                            ) {
                                params.slot_curve_nodes.lock()[editing_slot][editing_curve] = nodes;
                                // Publish updated gains to triple buffer
                                {
                                    use crate::dsp::pipeline::MAX_NUM_BINS;
                                    let full_gains = crv::compute_curve_response(
                                        &nodes, MAX_NUM_BINS, sr, fft_size,
                                    );
                                    if let Some(slot_chs) = curve_tx.get(editing_slot) {
                                        if let Some(tx_arc) = slot_chs.get(editing_curve) {
                                            if let Some(mut tx) = tx_arc.try_lock() {
                                                tx.input_buffer_mut().copy_from_slice(&full_gains);
                                                tx.publish();
                                            }
                                        }
                                    }
                                }
                            }

                            // Cursor tooltip
                            let max_hz = (sr / 2.0).max(20_001.0);
                            if let Some(hover) = ui.input(|i| i.pointer.hover_pos()) {
                                if curve_rect.contains(hover) {
                                    let freq = crv::screen_to_freq(hover.x, curve_rect, max_hz);
                                    let val  = crv::screen_y_to_physical(hover.y, editing_curve, db_min, db_max, curve_rect);
                                    let unit = crv::curve_y_unit(editing_curve);
                                    let freq_str = if freq >= 1_000.0 {
                                        format!("{:.2} kHz", freq / 1_000.0)
                                    } else {
                                        format!("{:.0} Hz", freq)
                                    };
                                    let val_str = format!("{:.1} {}", val, unit);
                                    let label   = format!("{}\n{}", freq_str, val_str);
                                    let tip_pos = hover + egui::vec2(12.0, -28.0);
                                    let font    = egui::FontId::proportional(10.0);
                                    let galley  = ui.painter().layout_no_wrap(
                                        label.clone(), font.clone(), th::GRID_TEXT,
                                    );
                                    let text_size = galley.size();
                                    let bg_rect = egui::Rect::from_min_size(
                                        tip_pos - egui::vec2(3.0, 3.0),
                                        text_size + egui::vec2(6.0, 6.0),
                                    );
                                    ui.painter().rect_filled(bg_rect, 2.0, egui::Color32::from_black_alpha(180));
                                    ui.painter().text(tip_pos, egui::Align2::LEFT_TOP, label, font, th::GRID_TEXT);
                                }
                            }
                        }
                    }

                    // Graph header: "Editing: {module_name} — {channel_target}"
                    {
                        let edit_slot = *params.editing_slot.lock() as usize;
                        let tgts      = params.slot_targets.lock();
                        let target_label = tgts[edit_slot].label();
                        drop(tgts);

                        let name_edit_key = ui.id().with(("name_edit", edit_slot));
                        let is_editing: bool = ui.data(|d| d.get_temp(name_edit_key).unwrap_or(false));

                        if is_editing {
                            let mut name_str = {
                                let names = params.slot_names.lock();
                                crate::editor::fx_matrix_grid::slot_name_str(&names[edit_slot])
                            };
                            let te = egui::TextEdit::singleline(&mut name_str)
                                .font(egui::FontId::proportional(10.0))
                                .desired_width(120.0)
                                .text_color(th::LABEL_DIM);
                            let resp = ui.put(
                                egui::Rect::from_min_size(
                                    curve_rect.min + egui::vec2(4.0, 4.0),
                                    egui::vec2(120.0, 14.0),
                                ),
                                te,
                            );
                            // Enforce 32-byte limit — pop chars to stay on a codepoint boundary
                            while name_str.len() > 32 {
                                name_str.pop();
                            }
                            // Save name back every frame (interim) + exit edit mode on enter or focus loss
                            {
                                let mut names = params.slot_names.lock();
                                let b = name_str.as_bytes();
                                let len = b.len().min(32);
                                names[edit_slot].fill(0);
                                names[edit_slot][..len].copy_from_slice(&b[..len]);
                            }
                            if resp.lost_focus() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                                ui.data_mut(|d| d.insert_temp::<bool>(name_edit_key, false));
                            }
                        } else {
                            let name_str = {
                                let names = params.slot_names.lock();
                                crate::editor::fx_matrix_grid::slot_name_str(&names[edit_slot])
                            };
                            let header = format!("Editing: {} \u{2014} {}", name_str, target_label);
                            let header_resp = ui.put(
                                egui::Rect::from_min_size(
                                    curve_rect.min + egui::vec2(4.0, 4.0),
                                    egui::vec2(300.0, 14.0),
                                ),
                                egui::Label::new(
                                    egui::RichText::new(&header)
                                        .color(th::LABEL_DIM).size(10.0)
                                ).sense(egui::Sense::click()),
                            );
                            if header_resp.clicked() {
                                ui.data_mut(|d| d.insert_temp(name_edit_key, true));
                            }
                            header_resp.on_hover_text("Click to rename this slot");
                        }
                    }

                    // ── Bottom strip ─────────────────────────────────────────────
                    ui.add_space(4.0);
                    ui.separator();
                    ui.add_space(2.0);

                    // ── SC assignment strip ────────────────────────────────────
                    ui.horizontal(|ui| {
                        ui.add_space(4.0);
                        let edit_slot = *params.editing_slot.lock() as usize;
                        let mut sc_assign = params.slot_sidechain.lock()[edit_slot];

                        ui.label(egui::RichText::new("SC").color(th::LABEL_DIM).size(9.0));
                        ui.add_space(2.0);

                        let sc_labels: &[(&str, u8)] = &[
                            ("SC1", 0), ("SC2", 1), ("SC3", 2), ("SC4", 3), ("Self", 255),
                        ];
                        for &(label, idx) in sc_labels {
                            let is_active = sc_assign == idx;
                            let sc_live = idx < 4 && sc_active[idx as usize];
                            let fill = if is_active {
                                if sc_live { egui::Color32::from_rgb(0x30, 0xa0, 0x50) }
                                else       { th::BORDER }
                            } else {
                                th::BG
                            };
                            let text_col = if is_active { egui::Color32::BLACK } else { th::LABEL_DIM };
                            let btn = egui::Button::new(
                                egui::RichText::new(label).color(text_col).size(9.0)
                            )
                            .fill(fill)
                            .stroke(egui::Stroke::new(th::STROKE_BORDER,
                                if sc_live { egui::Color32::from_rgb(0x30, 0xa0, 0x50) }
                                else       { th::BORDER }
                            ));
                            if ui.add(btn).clicked() {
                                sc_assign = idx;
                                params.slot_sidechain.lock()[edit_slot] = idx;
                            }
                        }
                    });
                    ui.add_space(2.0);

                    // ── GainMode selector (Gain module only) ──────────────────
                    {
                        let edit_slot = *params.editing_slot.lock() as usize;
                        let slot_type = params.slot_module_types.lock()[edit_slot];
                        if slot_type == crate::dsp::modules::ModuleType::Gain {
                            ui.horizontal(|ui| {
                                ui.add_space(4.0);
                                ui.label(egui::RichText::new("Mode").color(th::LABEL_DIM).size(9.0));
                                ui.add_space(2.0);

                                let cur_mode = params.slot_gain_mode.lock()[edit_slot];
                                use crate::dsp::modules::GainMode;
                                for (label, mode) in [("Add", GainMode::Add), ("Subtract", GainMode::Subtract), ("Pull", GainMode::Pull)] {
                                    let is_active = cur_mode == mode;
                                    let fill     = if is_active { th::BORDER } else { th::BG };
                                    let text_col = if is_active { egui::Color32::BLACK } else { th::LABEL_DIM };
                                    let btn = egui::Button::new(
                                        egui::RichText::new(label).color(text_col).size(9.0)
                                    )
                                    .fill(fill)
                                    .stroke(egui::Stroke::new(th::STROKE_BORDER, th::BORDER));
                                    if ui.add(btn).clicked() {
                                        params.slot_gain_mode.lock()[edit_slot] = mode;
                                    }
                                }
                            });
                            ui.add_space(2.0);
                        }
                    }

                    use nih_plug_egui::widgets::ParamSlider;

                    macro_rules! knob {
                        ($ui:expr, $param:expr, $label:expr) => {{
                            $ui.vertical(|ui| {
                                ui.add(ParamSlider::for_param($param, setter).with_width(36.0));
                                ui.label(
                                    egui::RichText::new($label).color(th::LABEL_DIM).size(9.0),
                                );
                            });
                        }};
                    }

                    let toggle = |ui: &mut egui::Ui, val: bool, label: &str| -> bool {
                        let (fill, text_color) = if val {
                            (th::BORDER, th::BG)
                        } else {
                            (th::BG, th::LABEL_DIM)
                        };
                        let btn = egui::Button::new(
                            egui::RichText::new(label).color(text_color).size(9.0),
                        )
                        .fill(fill)
                        .stroke(egui::Stroke::new(th::STROKE_BORDER, th::BORDER));
                        ui.add(btn).clicked()
                    };

                    // Row 1 — always visible: global gain/mix + toggle buttons
                    ui.horizontal(|ui| {
                        knob!(ui, &params.input_gain,  "IN");
                        knob!(ui, &params.output_gain, "OUT");
                        knob!(ui, &params.mix,         "MIX");
                        knob!(ui, &params.sc_gain,     "SC");

                        ui.add_space(8.0);

                        let auto_mk = params.auto_makeup.value();
                        if toggle(ui, auto_mk, "AUTO MK") {
                            setter.begin_set_parameter(&params.auto_makeup);
                            setter.set_parameter(&params.auto_makeup, !auto_mk);
                            setter.end_set_parameter(&params.auto_makeup);
                        }
                        ui.add_space(4.0);
                        let delta = params.delta_monitor.value();
                        if toggle(ui, delta, "DELTA") {
                            setter.begin_set_parameter(&params.delta_monitor);
                            setter.set_parameter(&params.delta_monitor, !delta);
                            setter.end_set_parameter(&params.delta_monitor);
                        }
                    });

                    ui.add_space(2.0);

                    // Row 2 — module-specific controls
                    ui.horizontal(|ui| {
                        let editing_slot  = *params.editing_slot.lock() as usize;
                        let slot_types    = *params.slot_module_types.lock();
                        let editing_type  = slot_types[editing_slot];
                        let editing_curve = (*params.editing_curve.lock() as usize)
                            .min(crate::dsp::modules::module_spec(editing_type).num_curves.saturating_sub(1));

                        // Dynamics group box: global dynamics knobs
                        {
                            let dyn_frame = egui::Frame::new()
                                .stroke(egui::Stroke::new(th::STROKE_BORDER, th::GRID_LINE))
                                .inner_margin(egui::Margin { left: 4, right: 4, top: 4, bottom: 4 });
                            let dyn_resp = dyn_frame.show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    knob!(ui, &params.attack_ms,         "Atk");
                                    knob!(ui, &params.release_ms,        "Rel");
                                    knob!(ui, &params.sensitivity,       "Sens");
                                    knob!(ui, &params.suppression_width, "Width");
                                });
                            });
                            let lbl_pos = dyn_resp.response.rect.left_top() + egui::vec2(4.0, 0.0);
                            ui.painter().text(
                                lbl_pos, egui::Align2::LEFT_TOP, "Dynamics",
                                egui::FontId::proportional(8.0), th::LABEL_DIM,
                            );
                        }

                        // Per-curve tilt and offset from slot_curve_meta
                        let spec = crate::dsp::modules::module_spec(editing_type);
                        if editing_curve < spec.num_curves {
                            ui.add_space(8.0);
                            let crv_col = spec.color_lit;
                            let mut meta = *params.slot_curve_meta.lock();
                            let (offset, tilt) = &mut meta[editing_slot][editing_curve];
                            let mut changed = false;
                            ui.vertical(|ui| {
                                if ui.add(
                                    egui::DragValue::new(offset)
                                        .range(-1.0..=1.0).speed(0.005).fixed_decimals(3)
                                ).changed() { changed = true; }
                                ui.label(egui::RichText::new("Offset").color(crv_col).size(9.0));
                            });
                            ui.vertical(|ui| {
                                if ui.add(
                                    egui::DragValue::new(tilt)
                                        .range(-1.0..=1.0).speed(0.005).fixed_decimals(3)
                                ).changed() { changed = true; }
                                ui.label(egui::RichText::new("Tilt").color(crv_col).size(9.0));
                            });
                            if changed {
                                *params.slot_curve_meta.lock() = meta;
                            }
                        }
                    });

                    // ── FX Routing Matrix ────────────────────────────────────────
                    ui.add_space(4.0);
                    ui.separator();
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new("ROUTING MATRIX")
                            .color(th::LABEL_DIM)
                            .size(9.0),
                    );
                    ui.add_space(2.0);

                    // Snapshot current state from params
                    let edit_slot  = *params.editing_slot.lock() as usize;
                    let types_snap = *params.slot_module_types.lock();
                    let names_snap = *params.slot_names.lock();
                    let mut route_guard = params.route_matrix.lock();
                    let route_matrix_ref = &mut *route_guard;
                    let interaction = crate::editor::fx_matrix_grid::paint_fx_matrix_grid(
                        ui,
                        &types_snap,
                        &names_snap,
                        route_matrix_ref,
                        edit_slot,
                    );
                    if let Some(new_slot) = interaction.left_click_slot {
                        *params.editing_slot.lock() = new_slot as u8;
                    }
                    // Handle right-click → open module assignment popup
                    if let Some((slot, pos)) = interaction.right_click {
                        crate::editor::module_popup::open_popup(ui, slot, pos);
                    }
                    // Render popup (egui Area — appears above matrix)
                    let _ = crate::editor::module_popup::show_popup(ui, &params);
                });
        },
    )
}
