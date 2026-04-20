use nih_plug::prelude::*;
use nih_plug_egui::{create_egui_editor, egui};
use parking_lot::Mutex;
use triple_buffer::Input as TbInput;
use std::sync::{Arc, atomic::Ordering};
use crate::params::{SpectralForgeParams, NUM_CURVE_SETS};
use crate::editor::{curve as crv, spectrum_display as sd, theme as th};


pub fn create_editor(
    params: Arc<SpectralForgeParams>,
    curve_tx: Vec<Vec<Arc<Mutex<TbInput<Vec<f32>>>>>>,
    sample_rate: Option<Arc<crate::bridge::AtomicF32>>,
    fft_size_arc: Arc<std::sync::atomic::AtomicUsize>,
    spectrum_rx: Option<Arc<parking_lot::Mutex<triple_buffer::Output<Vec<f32>>>>>,
    suppression_rx: Option<Arc<parking_lot::Mutex<triple_buffer::Output<Vec<f32>>>>>,
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
                    let active_idx   = *params.active_curve.lock() as usize;
                    let sr           = sample_rate.as_ref().map(|a| a.load()).unwrap_or(44100.0);
                    let db_min       = *params.graph_db_min.lock();
                    let db_max       = *params.graph_db_max.lock();
                    let falloff      = *params.peak_falloff_ms.lock();
                    let atk_ms       = params.attack_ms.value();
                    let rel_ms       = params.release_ms.value();
                    let active_tab   = *params.active_tab.lock() as usize;
                    let cur_mode     = params.effect_mode.value();
                    let freeze_active = *params.freeze_active_curve.lock() as usize;

                    let is_freeze_mode = active_tab == 1
                        && cur_mode == crate::params::EffectMode::Freeze;
                    let is_phase_mode  = active_tab == 1
                        && cur_mode == crate::params::EffectMode::PhaseRand;

                    // Per-curve tilt and offset arrays (indexed by curve_idx).
                    let tilts = [
                        params.threshold_tilt.value(),
                        params.ratio_tilt.value(),
                        params.attack_tilt.value(),
                        params.release_tilt.value(),
                        params.knee_tilt.value(),
                        params.makeup_tilt.value(),
                        params.mix_tilt.value(),
                    ];
                    let offsets = [
                        params.threshold_offset.value(),
                        params.ratio_offset.value(),
                        params.attack_offset.value(),
                        params.release_offset.value(),
                        params.knee_offset.value(),
                        params.makeup_offset.value(),
                        params.mix_offset.value(),
                    ];

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
                    const MATRIX_AREA_H: f32 = 9.0 * 44.0 + 14.0 + 30.0; // 440 px
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

                    // Determine which curve_idx drives the grid
                    let grid_curve_idx = if is_freeze_mode {
                        8 + freeze_active
                    } else if is_phase_mode {
                        7
                    } else {
                        active_idx
                    };

                    // 1. Grid
                    crv::paint_grid(ui.painter(), curve_rect, grid_curve_idx, db_min, db_max, sr);

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

                    // 3 + 4. Response curves + interactive widget
                    if is_phase_mode {
                        // Phase mode: single per-bin phase-amount curve.
                        let phase_nodes = *params.phase_curve_nodes.lock();
                        let phase_gains = crv::compute_curve_response(
                            &phase_nodes, num_bins, sr,
                            fft_size,
                        );
                        crv::paint_response_curve(
                            ui.painter(), curve_rect, &phase_gains, 7,
                            th::phase_color_lit(), 2.0,
                            db_min, db_max, atk_ms, rel_ms, sr,
                            fft_size, 0.0, 0.0,
                        );
                        // Interactive widget
                        let mut nodes = phase_nodes;
                        if crv::curve_widget(
                            ui, curve_rect, &mut nodes, &phase_gains,
                            7, db_min, db_max, atk_ms, rel_ms, sr,
                            fft_size, 0.0, 0.0,
                        ) {
                            *params.phase_curve_nodes.lock() = nodes;
                            // Phase curve has no dedicated bridge channel in D1; persisted only.
                        }
                    } else if is_freeze_mode {
                        // Freeze mode: show only the selected freeze curve.
                        let freeze_nodes_all = *params.freeze_curve_nodes.lock();
                        let freeze_nodes = freeze_nodes_all[freeze_active];
                        let freeze_gains = crv::compute_curve_response(
                            &freeze_nodes, num_bins, sr,
                            fft_size,
                        );
                        let freeze_curve_idx = 8 + freeze_active;
                        crv::paint_response_curve(
                            ui.painter(), curve_rect, &freeze_gains, freeze_curve_idx,
                            th::freeze_color_lit(freeze_active), 2.0,
                            db_min, db_max, atk_ms, rel_ms, sr,
                            fft_size, 0.0, 0.0,
                        );
                        // Interactive widget
                        let mut nodes_mut = freeze_nodes;
                        if crv::curve_widget(
                            ui, curve_rect, &mut nodes_mut, &freeze_gains,
                            freeze_curve_idx, db_min, db_max, atk_ms, rel_ms, sr,
                            fft_size, 0.0, 0.0,
                        ) {
                            params.freeze_curve_nodes.lock()[freeze_active] = nodes_mut;
                            // Freeze curves have no dedicated bridge channel in D1; persisted only.
                        }
                    } else {
                        // Dynamics / other tab: show all 7 dynamics response curves.
                        let nodes_snapshot = *params.curve_nodes.lock();
                        let cache_key = ui.id().with(("all_display_gains", fft_size));
                        let cached: Option<([[crv::CurveNode; 6]; NUM_CURVE_SETS], Vec<Vec<f32>>)> =
                            ui.data(|d| d.get_temp(cache_key));
                        let all_gains: Vec<Vec<f32>> = match cached {
                            Some((cached_nodes, cached_gains)) if cached_nodes == nodes_snapshot => {
                                cached_gains
                            }
                            _ => {
                                let g: Vec<Vec<f32>> = (0..NUM_CURVE_SETS)
                                    .map(|i| crv::compute_curve_response(
                                        &nodes_snapshot[i], num_bins, sr,
                                        fft_size,
                                    ))
                                    .collect();
                                ui.data_mut(|d| d.insert_temp(cache_key, (nodes_snapshot, g.clone())));
                                g
                            }
                        };

                        for i in 0..NUM_CURVE_SETS {
                            if i == active_idx { continue; }
                            crv::paint_response_curve(
                                ui.painter(), curve_rect, &all_gains[i], i,
                                th::curve_color_dim(i), 1.0,
                                db_min, db_max, atk_ms, rel_ms, sr,
                                fft_size,
                                tilts[i], offsets[i],
                            );
                        }
                        crv::paint_response_curve(
                            ui.painter(), curve_rect, &all_gains[active_idx], active_idx,
                            th::curve_color_lit(active_idx), 2.0,
                            db_min, db_max, atk_ms, rel_ms, sr,
                            fft_size,
                            tilts[active_idx], offsets[active_idx],
                        );

                        // Interactive nodes — Dynamics tab only
                        if active_tab == 0 {
                            let mut nodes = nodes_snapshot[active_idx];
                            if crv::curve_widget(
                                ui, curve_rect, &mut nodes, &all_gains[active_idx],
                                active_idx, db_min, db_max, atk_ms, rel_ms, sr,
                                fft_size,
                                tilts[active_idx], offsets[active_idx],
                            ) {
                                params.curve_nodes.lock()[active_idx] = nodes;
                                {
                                    use crate::dsp::pipeline::MAX_NUM_BINS;
                                    let full_gains = crv::compute_curve_response(
                                        &nodes, MAX_NUM_BINS, sr, fft_size,
                                    );
                                    let editing_slot = *params.editing_slot.lock() as usize;
                                    if let Some(slot_curves) = curve_tx.get(editing_slot) {
                                        if let Some(tx_arc) = slot_curves.get(active_idx) {
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
                                    let val  = crv::screen_y_to_physical(hover.y, active_idx, db_min, db_max, curve_rect);
                                    let unit = crv::curve_y_unit(active_idx);
                                    let freq_str = if freq >= 1_000.0 {
                                        format!("{:.2} kHz", freq / 1_000.0)
                                    } else {
                                        format!("{:.0} Hz", freq)
                                    };
                                    let val_str = match active_idx {
                                        1 => format!("{:.2} {}", val, unit),
                                        2 | 3 => format!("{:.1} {}", val, unit),
                                        6 => format!("{:.1} {}", val, unit),
                                        _ => format!("{:.1} {}", val, unit),
                                    };
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

                    // Harmonic placeholder text
                    if active_tab == 2 {
                        ui.painter().text(
                            curve_rect.center(),
                            egui::Align2::CENTER_CENTER,
                            "Harmonic — coming soon",
                            egui::FontId::proportional(14.0),
                            th::LABEL_DIM,
                        );
                    }

                    // Graph header: "Editing: {module_name} — {channel_target}"
                    {
                        let edit_slot = *params.editing_slot.lock() as usize;
                        let names  = params.slot_names.lock();
                        let tgts   = params.slot_targets.lock();
                        let name_str = crate::editor::fx_matrix_grid::slot_name_str(&names[edit_slot]);
                        let header = format!("Editing: {} \u{2014} {}", name_str, tgts[edit_slot].label());
                        ui.painter().text(
                            curve_rect.min + egui::vec2(4.0, 4.0),
                            egui::Align2::LEFT_TOP,
                            &header,
                            egui::FontId::proportional(10.0),
                            th::LABEL_DIM,
                        );
                    }

                    // ── Bottom strip ─────────────────────────────────────────────
                    ui.add_space(4.0);
                    ui.separator();
                    ui.add_space(2.0);

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
