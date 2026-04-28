use nih_plug::prelude::*;
use nih_plug_egui::{create_egui_editor, egui};
use parking_lot::Mutex;
use triple_buffer::Input as TbInput;
use std::collections::HashMap;
use std::sync::{Arc, atomic::Ordering};
use crate::params::SpectralForgeParams;
use crate::editor::{curve as crv, spectrum_display as sd, theme as th};
use crate::editor::mod_ring::{ModRingState, mod_ring_overlay};


pub fn create_editor(
    params: Arc<SpectralForgeParams>,
    curve_tx: Vec<Vec<Arc<Mutex<TbInput<Vec<f32>>>>>>,
    sample_rate: Option<Arc<crate::bridge::AtomicF32>>,
    fft_size_arc: Arc<std::sync::atomic::AtomicUsize>,
    spectrum_rx: Option<Arc<parking_lot::Mutex<triple_buffer::Output<Vec<f32>>>>>,
    suppression_rx: Option<Arc<parking_lot::Mutex<triple_buffer::Output<Vec<f32>>>>>,
    sc_envelope_rx: Option<Arc<parking_lot::Mutex<triple_buffer::Output<Vec<f32>>>>>,
    sidechain_active: Option<Arc<std::sync::atomic::AtomicBool>>,
    reset_requested: Option<Arc<std::sync::atomic::AtomicBool>>,
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

            // Scaling: use the user's chosen scale directly as pixels_per_point.
            // This is stable (no feedback loop) and ensures content renders at the target
            // scale immediately, even before the host finishes resizing the window.
            {
                let scale = *params.ui_scale.lock();
                const NOMINAL_W: f32 = 900.0;
                const NOMINAL_H: f32 = 1010.0;
                let ppp = scale.clamp(0.5, 4.0);
                ctx.set_pixels_per_point(ppp);

                // Only send resize request when the target scale changes.
                let last_key = egui::Id::new("last_ui_scale");
                let last: f32 = ctx.data(|d| d.get_temp(last_key).unwrap_or(-1.0));
                if (last - scale).abs() > 0.001 {
                    let w = (NOMINAL_W * scale).round() as u32;
                    let h = (NOMINAL_H * scale).round() as u32;
                    params.editor_state.set_requested_size((w, h));
                    ctx.data_mut(|d| d.insert_temp(last_key, scale));
                }
            }

            // Load preset menu state from egui temp storage (persists across frames).
            let preset_key = egui::Id::new("preset_menu_state");
            let mut preset_state: crate::editor::PresetMenuState =
                ctx.data(|d| d.get_temp(preset_key)).unwrap_or_default();

            // UI parameter contract: see docs/superpowers/specs/2026-04-23-ui-parameter-spec-design.md §4
            // Frame-scoped UI scale: read ctx.pixels_per_point() once per frame; all sizes,
            // strokes, and font points below flow through th::scaled / th::scaled_stroke.
            let scale = ctx.pixels_per_point();

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
                    // ── Top bar: preset pulldown + curve selectors + range controls ──
                    ui.horizontal(|ui| {
                        ui.add_space(4.0);
                        crate::editor::preset_menu_ui(ui, &mut preset_state, &params, setter);
                        ui.separator();
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

                        // Snapshot GainMode for this slot — used below to gate the Gain
                        // PEAK HOLD tab (only meaningful in Pull mode).
                        let slot_gain_mode_snap = params.slot_gain_mode.lock()[editing_slot];

                        // Adaptive curve selector buttons
                        for (i, &default_label) in spec.curve_labels.iter().enumerate() {
                            let label = crv::curve_label_for(
                                editing_type, i, slot_gain_mode_snap, default_label,
                            );
                            let gain_disabled = editing_type == crate::dsp::modules::ModuleType::Gain
                                && i == 1
                                && !matches!(slot_gain_mode_snap,
                                    crate::dsp::modules::GainMode::Pull
                                    | crate::dsp::modules::GainMode::Match);
                            let is_active = editing_curve == i && !gain_disabled;
                            let (fill, text_color, stroke_color) = if gain_disabled {
                                (spec.color_dim, th::LABEL_DIM, spec.color_dim)
                            } else if is_active {
                                (spec.color_lit,
                                 egui::Color32::BLACK,
                                 spec.color_lit)
                            } else {
                                (spec.color_dim,
                                 spec.color_lit,
                                 spec.color_dim)
                            };
                            let btn = egui::Button::new(
                                egui::RichText::new(label).color(text_color).size(th::scaled(th::FONT_SIZE_VALUE, scale)),
                            )
                            .fill(fill)
                            .stroke(egui::Stroke::new(th::scaled_stroke(th::STROKE_BORDER, scale), stroke_color));
                            let sense = if gain_disabled {
                                egui::Sense::hover()
                            } else {
                                egui::Sense::click()
                            };
                            let resp = ui.add(btn.sense(sense));
                            if !gain_disabled && resp.clicked() {
                                *params.editing_curve.lock() = i as u8;
                            }
                        }

                        if spec.num_curves > 0 {
                            ui.add_space(8.0);
                            ui.separator();
                            ui.add_space(4.0);
                        }

                        ui.label(egui::RichText::new("Floor").color(th::LABEL_DIM).size(th::scaled(th::FONT_SIZE_LABEL, scale)));
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
                        ui.label(egui::RichText::new("Ceil").color(th::LABEL_DIM).size(th::scaled(th::FONT_SIZE_LABEL, scale)));
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
                        ui.label(egui::RichText::new("Falloff").color(th::LABEL_DIM).size(th::scaled(th::FONT_SIZE_LABEL, scale)));
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

                        ui.add_space(8.0);
                        let sc_lit = sidechain_active
                            .as_ref()
                            .map(|a| a.load(Ordering::Relaxed))
                            .unwrap_or(false);
                        let color = if sc_lit { th::SC_METER_COLOR_LIT } else { th::SC_METER_COLOR_DIM };
                        let (rect, _resp) = ui.allocate_exact_size(
                            egui::vec2(th::SC_METER_WIDTH_PX, th::SC_METER_HEIGHT_PX),
                            egui::Sense::hover(),
                        );
                        ui.painter().rect_filled(rect, 0.0, color);

                        // Reset to Default button — opens a confirm dialog.
                        ui.add_space(8.0);
                        let reset_btn = egui::Button::new(
                            egui::RichText::new("Reset to Default")
                                .color(th::LABEL_DIM)
                                .size(th::scaled(th::FONT_SIZE_LABEL, scale)),
                        )
                        .fill(th::BG)
                        .stroke(egui::Stroke::new(th::scaled_stroke(th::STROKE_BORDER, scale), th::BORDER));
                        if ui.add(reset_btn).clicked() {
                            let key = egui::Id::new("show_reset_dialog");
                            ui.ctx().data_mut(|d| d.insert_temp(key, true));
                        }
                    });

                    // Confirm dialog for Reset to Default.
                    {
                        let key = egui::Id::new("show_reset_dialog");
                        let show: bool = ctx.data(|d| d.get_temp(key).unwrap_or(false));
                        if show {
                            egui::Window::new("Reset all settings?")
                                .collapsible(false)
                                .resizable(false)
                                .show(ctx, |ui| {
                                    ui.label("Reset every parameter to defaults and clear all module state. This cannot be undone.");
                                    ui.horizontal(|ui| {
                                        if ui.button("Reset").clicked() {
                                            params.reset_to_defaults(setter);
                                            if let Some(ref arc) = reset_requested {
                                                arc.store(true, std::sync::atomic::Ordering::Release);
                                            }
                                            ctx.data_mut(|d| d.insert_temp(key, false));
                                        }
                                        if ui.button("Cancel").clicked() {
                                            ctx.data_mut(|d| d.insert_temp(key, false));
                                        }
                                    });
                                });
                        }
                    }

                    // ── Second bar: FFT size selector ─────────────────────────────
                    ui.horizontal(|ui| {
                        ui.add_space(4.0);
                        ui.label(egui::RichText::new("FFT").color(th::LABEL_DIM).size(th::scaled(th::FONT_SIZE_LABEL, scale)));
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
                                egui::RichText::new(label).color(text_color).size(th::scaled(th::FONT_SIZE_BUTTON, scale)),
                            )
                            .fill(fill)
                            .stroke(egui::Stroke::new(th::scaled_stroke(th::STROKE_BORDER, scale), th::BORDER));
                            if ui.add(btn).clicked() {
                                setter.begin_set_parameter(&params.fft_size);
                                setter.set_parameter(&params.fft_size, choice);
                                setter.end_set_parameter(&params.fft_size);
                            }
                        }

                        ui.add_space(12.0);
                        ui.label(egui::RichText::new("Scale").color(th::LABEL_DIM).size(th::scaled(th::FONT_SIZE_LABEL, scale)));
                        ui.add_space(2.0);

                        const SCALE_STEPS: &[(f32, &str)] = &[
                            (1.0,  "1×"),
                            (1.25, "1.25×"),
                            (1.5,  "1.5×"),
                            (1.75, "1.75×"),
                            (2.0,  "2×"),
                        ];
                        let cur_scale = *params.ui_scale.lock();
                        for &(step_scale, label) in SCALE_STEPS {
                            let is_active = (cur_scale - step_scale).abs() < 0.01;
                            let (fill, text_color) = if is_active {
                                (th::BORDER, th::BG)
                            } else {
                                (th::BG, th::LABEL_DIM)
                            };
                            let btn = egui::Button::new(
                                egui::RichText::new(label).color(text_color).size(th::scaled(th::FONT_SIZE_BUTTON, scale)),
                            )
                            .fill(fill)
                            .stroke(egui::Stroke::new(th::scaled_stroke(th::STROKE_BORDER, scale), th::BORDER));
                            if ui.add(btn).clicked() {
                                *params.ui_scale.lock() = step_scale;
                            }
                        }
                    });

                    ui.add_space(2.0);
                    {
                        let r = ui.available_rect_before_wrap();
                        ui.painter().line_segment(
                            [r.left_top(), r.right_top()],
                            egui::Stroke::new(th::scaled_stroke(th::STROKE_BORDER, scale), th::BORDER),
                        );
                    }

                    // ── Spectrum / curve area ─────────────────────────────────────
                    // strip_height reserves space for all content below the curve area:
                    //   105 px — separator + knobs + dynamics row
                    //    28 px — SC assignment strip (always shown)
                    //    28 px — GainMode selector (reserved even when hidden to prevent jumps)
                    //   528 px — routing matrix worst case (9×44 + 4 virtual half-rows×22 + header + pad)
                    const MATRIX_AREA_H: f32 = 9.0 * 44.0 + 4.0 * 22.0 + 14.0 + 30.0;
                    let strip_height = 105.0 + 28.0 + 28.0 + MATRIX_AREA_H;
                    let avail = ui.available_rect_before_wrap();
                    // Clamp to the actual visible surface — at small UI scales the top
                    // toolbar can overflow horizontally and push `available_rect_before_wrap`
                    // wider than the panel; using that unclamped value would draw the curve
                    // off the right edge of the window.
                    let scr_max_x = ui.ctx().screen_rect().max.x;
                    let curve_rect = egui::Rect::from_min_max(
                        avail.min,
                        egui::pos2(
                            avail.max.x.min(scr_max_x),
                            (avail.max.y - strip_height).max(avail.min.y),
                        ),
                    );
                    ui.allocate_rect(curve_rect, egui::Sense::hover());

                    // Read spectrum + suppression from bridge.
                    // Cache the last successful read so try_lock misses don't flicker.
                    let spec_cache_key = ui.id().with("spectrum_cache");
                    let supp_cache_key = ui.id().with("suppression_cache");

                    let mut raw_magnitudes: Option<Vec<f32>> = None;
                    if let Some(ref rx_arc) = spectrum_rx {
                        if let Some(mut rx) = rx_arc.try_lock() {
                            let v = rx.read()[..num_bins].to_vec();
                            ui.data_mut(|d| d.insert_temp(spec_cache_key, v.clone()));
                            raw_magnitudes = Some(v);
                        } else {
                            raw_magnitudes = ui.data(|d| d.get_temp(spec_cache_key));
                        }
                    }

                    let suppression_data: Vec<f32> = if let Some(ref rx_arc) = suppression_rx {
                        if let Some(mut rx) = rx_arc.try_lock() {
                            let v = rx.read()[..num_bins].to_vec();
                            ui.data_mut(|d| d.insert_temp(supp_cache_key, v.clone()));
                            v
                        } else {
                            ui.data(|d| d.get_temp::<Vec<f32>>(supp_cache_key))
                                .unwrap_or_default()
                        }
                    } else {
                        Vec::new()
                    };

                    // Peak-hold buffer
                    let peak_key = ui.id().with("peak_hold");
                    let mut peak_hold: Vec<f32> = ui.data(|d| d.get_temp(peak_key))
                        .unwrap_or_default();

                    // 1. Grid — drive horizontal lines and Y-axis label from CurveDisplayConfig
                    // See docs/superpowers/specs/2026-04-23-ui-parameter-spec-design.md §3.
                    let grid_editing_slot  = *params.editing_slot.lock() as usize;
                    let grid_editing_type  = params.slot_module_types.lock()[grid_editing_slot];
                    let grid_curve_raw     = *params.editing_curve.lock() as usize;
                    let grid_gain_mode     = params.slot_gain_mode.lock()[grid_editing_slot];
                    let grid_display_idx   = crv::display_curve_idx(
                        grid_editing_type, grid_curve_raw, grid_gain_mode,
                    );
                    let grid_cfg = crate::editor::curve_config::curve_display_config(
                        grid_editing_type, grid_curve_raw, grid_gain_mode,
                    );
                    crv::paint_grid(
                        ui.painter(), curve_rect, &grid_cfg, grid_display_idx,
                        db_min, db_max, sr,
                    );

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
                    // UI parameter contract: see docs/superpowers/specs/2026-04-23-ui-parameter-spec-design.md
                    {
                        let editing_slot  = *params.editing_slot.lock() as usize;
                        let slot_types    = *params.slot_module_types.lock();
                        let editing_type  = slot_types[editing_slot];
                        let slot_gain_mode_snap = params.slot_gain_mode.lock()[editing_slot];
                        let spec          = crate::dsp::modules::module_spec(editing_type);
                        let num_c         = spec.num_curves;
                        let raw_curve = *params.editing_curve.lock() as usize;
                        let editing_curve = if raw_curve >= num_c && num_c > 0 {
                            *params.editing_curve.lock() = 0;
                            0
                        } else {
                            raw_curve
                        };

                        // Read this slot's curve nodes lock-free from automatable params.
                        let slot_nodes: [[crv::CurveNode; 6]; 7] = std::array::from_fn(|c| {
                            std::array::from_fn(|n| {
                                params.graph_node(editing_slot, c, n)
                                    .map(|(x, y, q)| crv::CurveNode { x: x.value(), y: y.value(), q: q.value() })
                                    .unwrap_or_default()
                            })
                        });

                        // Cache key: invalidate when slot type, editing slot, or fft_size changes
                        let cache_key = ui.id().with(("slot_gains", editing_slot, editing_type as u8, fft_size));
                        let cached: Option<([[crv::CurveNode; 6]; 7], Vec<Vec<f32>>)> =
                            ui.data(|d| d.get_temp(cache_key));
                        let all_gains: Vec<Vec<f32>> = match cached {
                            Some((cn, cg)) if cn == slot_nodes => cg,
                            _ => {
                                let g: Vec<Vec<f32>> = (0..num_c.min(7))
                                    .map(|c| crv::compute_curve_response(
                                        &slot_nodes[c], num_bins, sr, fft_size,
                                    ))
                                    .collect();
                                ui.data_mut(|d| d.insert_temp(cache_key, (slot_nodes, g.clone())));
                                g
                            }
                        };

                        // Read tilt/offset/curvature lock-free from automatable params.
                        let slot_meta: [(f32, f32, f32); 7] = std::array::from_fn(|c| {
                            let t  = params.tilt_param(editing_slot, c).map(|p| p.value()).unwrap_or(0.0);
                            let o  = params.offset_param(editing_slot, c).map(|p| p.value()).unwrap_or(0.0);
                            let cv = params.curvature_param(editing_slot, c).map(|p| p.value()).unwrap_or(0.0);
                            (t, o, cv)
                        });

                        // Draw inactive curves (dim) — display_curve_idx maps to correct y-axis scale
                        for i in 0..num_c.min(7) {
                            if i == editing_curve { continue; }
                            let (tilt, offset, curvature) = slot_meta[i];
                            let disp_i = crv::display_curve_idx(editing_type, i, slot_gain_mode_snap);
                            let offset_fn = crate::editor::curve_config::curve_display_config(
                                editing_type, i, slot_gain_mode_snap,
                            ).offset_fn;
                            crv::paint_response_curve(
                                ui.painter(), curve_rect, &all_gains[i], disp_i,
                                spec.color_dim, 1.0,
                                db_min, db_max, atk_ms, rel_ms, sr, fft_size, tilt, offset, curvature,
                                offset_fn,
                            );
                        }

                        // Draw active curve (lit) + interactive widget
                        if editing_curve < num_c && !all_gains.is_empty() {
                            // Live SC envelope overlay — painted first so the active curve draws
                            // on top. SC affects every Gain mode, so show it for any Gain curve.
                            if editing_type == crate::dsp::modules::ModuleType::Gain {
                                if let Some(ref env_arc) = sc_envelope_rx {
                                    if let Some(mut rx) = env_arc.try_lock() {
                                        let env = rx.read();
                                        crv::paint_peak_hold_envelope_overlay(
                                            ui.painter(), curve_rect, &env[..num_bins],
                                            spec.color_lit, sr, fft_size,
                                            db_min, db_max,
                                        );
                                    }
                                }
                            }

                            let (tilt, offset, curvature) = slot_meta[editing_curve];
                            let disp_curve = crv::display_curve_idx(
                                editing_type, editing_curve, slot_gain_mode_snap,
                            );
                            let offset_fn = crate::editor::curve_config::curve_display_config(
                                editing_type, editing_curve, slot_gain_mode_snap,
                            ).offset_fn;
                            crv::paint_response_curve(
                                ui.painter(), curve_rect, &all_gains[editing_curve], disp_curve,
                                spec.color_lit, 2.0,
                                db_min, db_max, atk_ms, rel_ms, sr, fft_size, tilt, offset, curvature,
                                offset_fn,
                            );

                            let mut nodes = slot_nodes[editing_curve];
                            let cwr = crv::curve_widget(
                                ui, curve_rect, &mut nodes, editing_curve, sr,
                            );
                            if cwr.drag_started {
                                for n in 0..crate::param_ids::NUM_NODES {
                                    if let Some((x_p, y_p, q_p)) = params.graph_node(editing_slot, editing_curve, n) {
                                        setter.begin_set_parameter(x_p);
                                        setter.begin_set_parameter(y_p);
                                        setter.begin_set_parameter(q_p);
                                    }
                                }
                            }
                            if cwr.changed {
                                for n in 0..crate::param_ids::NUM_NODES {
                                    if let Some((x_p, y_p, q_p)) = params.graph_node(editing_slot, editing_curve, n) {
                                        setter.set_parameter(x_p, nodes[n].x);
                                        setter.set_parameter(y_p, nodes[n].y.clamp(-1.0, 1.0));
                                        setter.set_parameter(q_p, nodes[n].q);
                                    }
                                }
                                // Keep triple-buffer publish so DSP gets the updated curve.
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
                            if cwr.drag_stopped {
                                for n in 0..crate::param_ids::NUM_NODES {
                                    if let Some((x_p, y_p, q_p)) = params.graph_node(editing_slot, editing_curve, n) {
                                        setter.end_set_parameter(x_p);
                                        setter.end_set_parameter(y_p);
                                        setter.end_set_parameter(q_p);
                                    }
                                }
                            }

                            // Alt-click on a node opens the Modulation Ring overlay.
                            if let Some((node_screen_pos, node_idx)) = cwr.alt_clicked_node {
                                let anchor_data = Some((node_screen_pos, editing_slot, editing_curve, node_idx));
                                ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("mod_ring_anchor"), anchor_data));
                            }

                            // Cursor tooltip — unified path driven by CurveDisplayConfig.
                            // See docs/superpowers/specs/2026-04-23-ui-parameter-spec-design.md §3.
                            if let Some(hover) = ui.input(|i| i.pointer.hover_pos()) {
                                if curve_rect.contains(hover) {
                                    let hover_cfg = crate::editor::curve_config::curve_display_config(
                                        editing_type, editing_curve, slot_gain_mode_snap,
                                    );
                                    crv::paint_hover_text(
                                        ui.painter(), hover, curve_rect, disp_curve, &hover_cfg,
                                        db_min, db_max, sr,
                                    );
                                }
                            }

                            // Modulation Ring overlay — shown when a node was alt-clicked.
                            {
                                let anchor: Option<(egui::Pos2, usize, usize, usize)> =
                                    ui.ctx().data(|d| d.get_temp(egui::Id::new("mod_ring_anchor"))).flatten();
                                if let Some((anchor_pos, ring_slot, ring_curve, ring_node)) = anchor {
                                    let key = (ring_slot, ring_curve, ring_node);
                                    let mut states: HashMap<(usize, usize, usize), ModRingState> =
                                        ui.ctx().data(|d| d.get_temp(egui::Id::new("mod_ring_states"))).unwrap_or_default();
                                    let mut state = *states.get(&key).unwrap_or(&ModRingState::default());
                                    let toggle_clicked = mod_ring_overlay(ui, anchor_pos, &state);
                                    if let Some(t) = toggle_clicked {
                                        state.toggle(t);
                                    }
                                    states.insert(key, state);
                                    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("mod_ring_states"), states));
                                    // Close the ring when a click lands outside the overlay dots.
                                    // Note: egui's pointer.any_click() is NOT drained by per-widget
                                    // interact() calls — both a dot's clicked() and any_click() can
                                    // fire on the same frame. The toggle_clicked.is_none() guard is
                                    // what actually distinguishes "click on dot" (do not close) from
                                    // "click anywhere else" (close); event consumption is not doing
                                    // the work here. Phase 4 maintainers: always pair with this guard.
                                    if toggle_clicked.is_none() && ui.input(|i| i.pointer.any_click()) {
                                        ui.ctx().data_mut(|d| d.insert_temp(
                                            egui::Id::new("mod_ring_anchor"),
                                            Option::<(egui::Pos2, usize, usize, usize)>::None,
                                        ));
                                    }
                                }
                            }
                        }

                        // Per-module non-curve UI panel (Phase 1 hook; defaults to no-op for
                        // every shipped module). See `dsp/modules/mod.rs::PanelWidgetFn`.
                        if let Some(panel_fn) = crate::dsp::modules::module_spec(editing_type).panel_widget {
                            ui.separator();
                            panel_fn(ui, params.as_ref(), editing_slot);
                        }
                    }

                    // Graph header: "Editing: {module_name} — {channel_target}"
                    // Paint as an overlay on the curve area using painter + interact so we never
                    // call ui.put(), which resets the layout cursor backward into the curve area.
                    {
                        let edit_slot = *params.editing_slot.lock() as usize;
                        let edit_ty   = params.slot_module_types.lock()[edit_slot];
                        let edit_spec = crate::dsp::modules::module_spec(edit_ty);
                        let edit_curve = (*params.editing_curve.lock() as usize)
                            .min(edit_spec.num_curves.saturating_sub(1));
                        let edit_gain_mode = params.slot_gain_mode.lock()[edit_slot];
                        let default_label = edit_spec
                            .curve_labels
                            .get(edit_curve)
                            .copied()
                            .unwrap_or("");
                        let curve_label = crv::curve_label_for(
                            edit_ty, edit_curve, edit_gain_mode, default_label,
                        );

                        let name_edit_key = ui.id().with(("name_edit", edit_slot));
                        let is_editing: bool = ui.data(|d| d.get_temp(name_edit_key).unwrap_or(false));

                        if is_editing {
                            // Floating Area so the TextEdit widget doesn't touch the parent
                            // layout cursor.
                            let mut name_str = {
                                let names = params.slot_names.lock();
                                crate::editor::fx_matrix_grid::slot_name_str(&names[edit_slot])
                            };
                            egui::Area::new(egui::Id::new("slot_name_edit_area"))
                                .fixed_pos(curve_rect.min + egui::vec2(4.0, 4.0))
                                .order(egui::Order::Foreground)
                                .show(ui.ctx(), |ui| {
                                    let te = egui::TextEdit::singleline(&mut name_str)
                                        .font(egui::FontId::proportional(th::scaled(th::FONT_SIZE_BUTTON, scale)))
                                        .desired_width(120.0)
                                        .text_color(th::LABEL_DIM);
                                    let resp = ui.add(te);
                                    // Enforce 32-byte limit — pop chars to stay on a codepoint boundary
                                    while name_str.len() > 32 { name_str.pop(); }
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
                                });
                        } else {
                            // Painter-only text + interact — no layout cursor effect.
                            let name_str = {
                                let names = params.slot_names.lock();
                                crate::editor::fx_matrix_grid::slot_name_str(&names[edit_slot])
                            };
                            let header = if curve_label.is_empty() {
                                format!("Editing: {}", name_str)
                            } else {
                                format!("Editing: {} \u{2014} {}", name_str, curve_label)
                            };
                            let header_rect = egui::Rect::from_min_size(
                                curve_rect.min + egui::vec2(4.0, 4.0),
                                egui::vec2(300.0, 14.0),
                            );
                            ui.painter().text(
                                header_rect.left_top(),
                                egui::Align2::LEFT_TOP,
                                &header,
                                egui::FontId::proportional(th::scaled(th::FONT_SIZE_BUTTON, scale)),
                                th::LABEL_DIM,
                            );
                            let header_resp = ui.interact(
                                header_rect,
                                ui.id().with("slot_header_interact"),
                                egui::Sense::click(),
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

                    // ── Per-module SC strip (always reserves space so the
                    // tilt/offset row below doesn't shift between SC-aware and
                    // non-SC modules — the strip is rendered invisibly when the
                    // current module doesn't support sidechaining).
                    {
                        let edit_slot = *params.editing_slot.lock() as usize;
                        let slot_type = params.slot_module_types.lock()[edit_slot];
                        let supports_sc = crate::dsp::modules::module_spec(slot_type).supports_sidechain;
                        let mut sc_builder = egui::UiBuilder::new();
                        if !supports_sc { sc_builder = sc_builder.invisible(); }
                        ui.scope_builder(sc_builder, |ui| {
                            sc_strip_ui(ui, &params, edit_slot, scale);
                        });
                        ui.separator();
                    }

                    // ── GainMode selector — visible only for Gain modules, but the
                    // row is still laid out invisibly for other modules so the knob
                    // row below stays in the same vertical position regardless.
                    {
                        let edit_slot = *params.editing_slot.lock() as usize;
                        let slot_type = params.slot_module_types.lock()[edit_slot];
                        let is_gain = slot_type == crate::dsp::modules::ModuleType::Gain;

                        let mut mode_builder = egui::UiBuilder::new();
                        if !is_gain { mode_builder = mode_builder.invisible(); }
                        ui.scope_builder(mode_builder, |ui| {
                            ui.horizontal(|ui| {
                                ui.add_space(4.0);
                                ui.label(egui::RichText::new("Mode").color(th::LABEL_DIM).size(th::scaled(th::FONT_SIZE_LABEL, scale)));
                                ui.add_space(2.0);

                                let cur_mode = params.slot_gain_mode.lock()[edit_slot];
                                use crate::dsp::modules::GainMode;
                                for (label, mode) in [
                                    ("Add",      GainMode::Add),
                                    ("Subtract", GainMode::Subtract),
                                    ("Pull",     GainMode::Pull),
                                    ("Match",    GainMode::Match),
                                ] {
                                    let is_active = cur_mode == mode;
                                    let fill     = if is_active { th::BORDER } else { th::BG };
                                    let text_col = if is_active { egui::Color32::BLACK } else { th::LABEL_DIM };
                                    let btn = egui::Button::new(
                                        egui::RichText::new(label).color(text_col).size(th::scaled(th::FONT_SIZE_LABEL, scale))
                                    )
                                    .fill(fill)
                                    .stroke(egui::Stroke::new(th::scaled_stroke(th::STROKE_BORDER, scale), th::BORDER));
                                    let resp = ui.add(btn);
                                    if is_gain && resp.clicked() {
                                        params.slot_gain_mode.lock()[edit_slot] = mode;
                                    }
                                }
                            });
                        });
                        ui.add_space(2.0);
                    }

                    // ── Mode selector — visible only for Future and Punch modules, but the
                    // row is still laid out invisibly for other modules so the knob
                    // row below stays in the same vertical position regardless.
                    {
                        let edit_slot = *params.editing_slot.lock() as usize;
                        let slot_type = params.slot_module_types.lock()[edit_slot];
                        let is_future   = slot_type == crate::dsp::modules::ModuleType::Future;
                        let is_punch    = slot_type == crate::dsp::modules::ModuleType::Punch;
                        let is_rhythm   = slot_type == crate::dsp::modules::ModuleType::Rhythm;
                        let is_geometry = slot_type == crate::dsp::modules::ModuleType::Geometry;
                        let is_modulate = slot_type == crate::dsp::modules::ModuleType::Modulate;
                        let is_circuit  = slot_type == crate::dsp::modules::ModuleType::Circuit;
                        let is_life     = slot_type == crate::dsp::modules::ModuleType::Life;

                        let mut mode_builder = egui::UiBuilder::new();
                        if !is_future && !is_punch && !is_rhythm && !is_geometry && !is_modulate && !is_circuit && !is_life { mode_builder = mode_builder.invisible(); }
                        ui.scope_builder(mode_builder, |ui| {
                            ui.horizontal(|ui| {
                                ui.add_space(4.0);
                                ui.label(egui::RichText::new("Mode").color(th::LABEL_DIM).size(th::scaled(th::FONT_SIZE_LABEL, scale)));
                                ui.add_space(2.0);

                                if is_future {
                                    let cur_mode = params.slot_future_mode.lock()[edit_slot];
                                    use crate::dsp::modules::FutureMode;
                                    for (label, mode) in [
                                        ("Print-Thru", FutureMode::PrintThrough),
                                        ("Pre-Echo",   FutureMode::PreEcho),
                                    ] {
                                        let is_active = cur_mode == mode;
                                        let fill     = if is_active { th::BORDER } else { th::BG };
                                        let text_col = if is_active { egui::Color32::BLACK } else { th::LABEL_DIM };
                                        let btn = egui::Button::new(
                                            egui::RichText::new(label).color(text_col).size(th::scaled(th::FONT_SIZE_LABEL, scale))
                                        )
                                        .fill(fill)
                                        .stroke(egui::Stroke::new(th::scaled_stroke(th::STROKE_BORDER, scale), th::BORDER));
                                        let resp = ui.add(btn);
                                        if resp.clicked() {
                                            params.slot_future_mode.lock()[edit_slot] = mode;
                                        }
                                    }
                                } else if is_punch {
                                    let cur_mode = params.slot_punch_mode.lock()[edit_slot];
                                    use crate::dsp::modules::punch::PunchMode;
                                    for (label, mode) in [
                                        ("Direct",  PunchMode::Direct),
                                        ("Inverse", PunchMode::Inverse),
                                    ] {
                                        let is_active = cur_mode == mode;
                                        let fill     = if is_active { th::BORDER } else { th::BG };
                                        let text_col = if is_active { egui::Color32::BLACK } else { th::LABEL_DIM };
                                        let btn = egui::Button::new(
                                            egui::RichText::new(label).color(text_col).size(th::scaled(th::FONT_SIZE_LABEL, scale))
                                        )
                                        .fill(fill)
                                        .stroke(egui::Stroke::new(th::scaled_stroke(th::STROKE_BORDER, scale), th::BORDER));
                                        let resp = ui.add(btn);
                                        if resp.clicked() {
                                            params.slot_punch_mode.lock()[edit_slot] = mode;
                                        }
                                    }
                                } else if is_rhythm {
                                    let cur_mode = params.slot_rhythm_mode.lock()[edit_slot];
                                    use crate::dsp::modules::rhythm::RhythmMode;
                                    for (label, mode) in [
                                        ("Euclidean",   RhythmMode::Euclidean),
                                        ("Arpeggiator", RhythmMode::Arpeggiator),
                                        ("Phase Reset", RhythmMode::PhaseReset),
                                    ] {
                                        let is_active = cur_mode == mode;
                                        let fill     = if is_active { th::BORDER } else { th::BG };
                                        let text_col = if is_active { egui::Color32::BLACK } else { th::LABEL_DIM };
                                        let btn = egui::Button::new(
                                            egui::RichText::new(label).color(text_col).size(th::scaled(th::FONT_SIZE_LABEL, scale))
                                        )
                                        .fill(fill)
                                        .stroke(egui::Stroke::new(th::scaled_stroke(th::STROKE_BORDER, scale), th::BORDER));
                                        let resp = ui.add(btn);
                                        if resp.clicked() {
                                            params.slot_rhythm_mode.lock()[edit_slot] = mode;
                                        }
                                    }
                                } else if is_geometry {
                                    let cur_mode = params.slot_geometry_mode.lock()[edit_slot];
                                    use crate::dsp::modules::geometry::GeometryMode;
                                    for (label, mode) in [
                                        ("Chladni",   GeometryMode::Chladni),
                                        ("Helmholtz", GeometryMode::Helmholtz),
                                    ] {
                                        let is_active = cur_mode == mode;
                                        let fill     = if is_active { th::BORDER } else { th::BG };
                                        let text_col = if is_active { egui::Color32::BLACK } else { th::LABEL_DIM };
                                        let btn = egui::Button::new(
                                            egui::RichText::new(label).color(text_col).size(th::scaled(th::FONT_SIZE_LABEL, scale))
                                        )
                                        .fill(fill)
                                        .stroke(egui::Stroke::new(th::scaled_stroke(th::STROKE_BORDER, scale), th::BORDER));
                                        let resp = ui.add(btn);
                                        if resp.clicked() {
                                            params.slot_geometry_mode.lock()[edit_slot] = mode;
                                        }
                                    }
                                } else if is_modulate {
                                    let cur_mode = params.slot_modulate_mode.lock()[edit_slot];
                                    use crate::dsp::modules::modulate::ModulateMode;
                                    for (label, mode) in [
                                        ("Phase Phaser", ModulateMode::PhasePhaser),
                                        ("Bin Swapper",  ModulateMode::BinSwapper),
                                        ("RM/FM",        ModulateMode::RmFmMatrix),
                                        ("Diode RM",     ModulateMode::DiodeRm),
                                        ("Ground Loop",  ModulateMode::GroundLoop),
                                    ] {
                                        let is_active = cur_mode == mode;
                                        let fill     = if is_active { th::BORDER } else { th::BG };
                                        let text_col = if is_active { egui::Color32::BLACK } else { th::LABEL_DIM };
                                        let btn = egui::Button::new(
                                            egui::RichText::new(label).color(text_col).size(th::scaled(th::FONT_SIZE_LABEL, scale))
                                        )
                                        .fill(fill)
                                        .stroke(egui::Stroke::new(th::scaled_stroke(th::STROKE_BORDER, scale), th::BORDER));
                                        let resp = ui.add(btn);
                                        if resp.clicked() {
                                            params.slot_modulate_mode.lock()[edit_slot] = mode;
                                        }
                                    }
                                } else if is_circuit {
                                    let cur_mode = params.slot_circuit_mode.lock()[edit_slot];
                                    use crate::dsp::modules::circuit::CircuitMode;
                                    for (label, mode) in [
                                        ("Crossover", CircuitMode::CrossoverDistortion),
                                        ("Schmitt",   CircuitMode::SpectralSchmitt),
                                        ("BBD",       CircuitMode::BbdBins),
                                    ] {
                                        let is_active = cur_mode == mode;
                                        let fill     = if is_active { th::BORDER } else { th::BG };
                                        let text_col = if is_active { egui::Color32::BLACK } else { th::LABEL_DIM };
                                        let btn = egui::Button::new(
                                            egui::RichText::new(label).color(text_col).size(th::scaled(th::FONT_SIZE_LABEL, scale))
                                        )
                                        .fill(fill)
                                        .stroke(egui::Stroke::new(th::scaled_stroke(th::STROKE_BORDER, scale), th::BORDER));
                                        let resp = ui.add(btn);
                                        if resp.clicked() {
                                            params.slot_circuit_mode.lock()[edit_slot] = mode;
                                        }
                                    }
                                } else if is_life {
                                    let cur_mode = params.slot_life_mode.try_lock()
                                        .map(|arr| arr[edit_slot])
                                        .unwrap_or_default();
                                    let label = crate::editor::life_popup::mode_label(cur_mode);
                                    let btn_text = format!("Mode: {}", label);
                                    let btn = egui::Button::new(
                                        egui::RichText::new(btn_text).color(th::LABEL_DIM).size(th::scaled(th::FONT_SIZE_LABEL, scale))
                                    )
                                    .fill(th::BG)
                                    .stroke(egui::Stroke::new(th::scaled_stroke(th::STROKE_BORDER, scale), th::BORDER));
                                    let resp = ui.add(btn);
                                    if resp.clicked() {
                                        crate::editor::life_popup::open_at(ui, edit_slot, resp.rect.right_top());
                                    }
                                }
                            });
                        });
                        ui.add_space(2.0);
                    }

                    use nih_plug_egui::widgets::ParamSlider;

                    macro_rules! knob {
                        ($ui:expr, $param:expr, $label:expr) => {{
                            $ui.vertical(|ui| {
                                ui.add(ParamSlider::for_param($param, setter).with_width(36.0));
                                ui.label(
                                    egui::RichText::new($label).color(th::LABEL_DIM).size(th::scaled(th::FONT_SIZE_LABEL, scale)),
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
                            egui::RichText::new(label).color(text_color).size(th::scaled(th::FONT_SIZE_LABEL, scale)),
                        )
                        .fill(fill)
                        .stroke(egui::Stroke::new(th::scaled_stroke(th::STROKE_BORDER, scale), th::BORDER));
                        ui.add(btn).clicked()
                    };

                    // Row 1 — always visible: global gain/mix + toggle buttons
                    ui.horizontal(|ui| {
                        knob!(ui, &params.input_gain,  "IN");
                        knob!(ui, &params.output_gain, "OUT");
                        knob!(ui, &params.mix,         "MIX");

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

                    // Row 2a — module-specific controls (Dynamics group box).
                    // Always rendered (invisibly when not Dynamics) so the
                    // Offset/Tilt/Curv row below sits at the same vertical
                    // position regardless of the active module type — same
                    // pattern as the SC strip and GainMode selector above.
                    let editing_slot  = *params.editing_slot.lock() as usize;
                    let slot_types    = *params.slot_module_types.lock();
                    let editing_type  = slot_types[editing_slot];
                    let editing_gain_mode = params.slot_gain_mode.lock()[editing_slot];
                    let editing_curve = (*params.editing_curve.lock() as usize)
                        .min(crate::dsp::modules::module_spec(editing_type).num_curves.saturating_sub(1));

                    let is_dynamics = editing_type == crate::dsp::modules::ModuleType::Dynamics;
                    let mut dyn_builder = egui::UiBuilder::new();
                    if !is_dynamics { dyn_builder = dyn_builder.invisible(); }
                    ui.scope_builder(dyn_builder, |ui| {
                        ui.horizontal(|ui| {
                            let dyn_frame = egui::Frame::new()
                                .stroke(egui::Stroke::new(th::scaled_stroke(th::STROKE_BORDER, scale), th::GRID_LINE))
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
                                egui::FontId::proportional(th::scaled(th::FONT_SIZE_TINY, scale)), th::LABEL_DIM,
                            );
                        });
                    });

                    ui.add_space(2.0);

                    // Row 2b — per-curve Offset / Tilt / Curv. Single code path
                    // for every module type, fixed vertical position.
                    ui.horizontal(|ui| {
                        let spec = crate::dsp::modules::module_spec(editing_type);
                        if editing_curve < spec.num_curves {
                            ui.add_space(4.0);
                            let crv_col = spec.color_lit;
                            let curve_label = spec.curve_labels.get(editing_curve).copied().unwrap_or("");
                            if let Some(off_p) = params.offset_param(editing_slot, editing_curve) {
                                let mut off_norm = off_p.value();
                                // Capture what we need for the physical-unit formatter.
                                let off_cfg = crate::editor::curve_config::curve_display_config(
                                    editing_type, editing_curve, editing_gain_mode,
                                );
                                let off_disp_idx = crv::display_curve_idx(
                                    editing_type, editing_curve, editing_gain_mode,
                                );
                                let off_atk_ms  = atk_ms;
                                let off_rel_ms  = rel_ms;
                                let off_db_min  = db_min;
                                let off_db_max  = db_max;
                                ui.vertical(|ui| {
                                    let resp = ui.add(
                                        egui::DragValue::new(&mut off_norm)
                                            .range(-1.0..=1.0)
                                            .speed(1.0 / 300.0)
                                            .custom_formatter(move |v, _range| {
                                                let g_off = (off_cfg.offset_fn)(1.0, v as f32);
                                                let phys = crv::gain_to_display(
                                                    off_disp_idx, g_off,
                                                    off_atk_ms, off_rel_ms,
                                                    off_db_min, off_db_max,
                                                );
                                                if off_cfg.y_label.is_empty() {
                                                    format!("{:.2}", phys)
                                                } else {
                                                    format!("{:.1} {}", phys, off_cfg.y_label)
                                                }
                                            })
                                    );
                                    if resp.drag_started() { setter.begin_set_parameter(off_p); }
                                    if resp.changed() {
                                        let clamped = off_norm.clamp(-1.0, 1.0);
                                        setter.set_parameter(off_p, clamped);
                                    }
                                    if resp.drag_stopped() { setter.end_set_parameter(off_p); }
                                    crate::editor::delayed_tooltip(ui, &resp,
                                        format!("Slot {} · {} · Offset", editing_slot + 1, curve_label));
                                    ui.label(egui::RichText::new("Offset").color(crv_col).size(th::scaled(th::FONT_SIZE_LABEL, scale)));
                                });
                            }

                            if let Some(tilt_p) = params.tilt_param(editing_slot, editing_curve) {
                                let mut tilt_norm = tilt_p.value();
                                ui.vertical(|ui| {
                                    let resp = ui.add(
                                        egui::DragValue::new(&mut tilt_norm)
                                            .range(-1.0..=1.0)
                                            .speed(1.0 / 300.0)
                                            .fixed_decimals(2)
                                    );
                                    if resp.drag_started() { setter.begin_set_parameter(tilt_p); }
                                    if resp.changed() {
                                        let clamped = tilt_norm.clamp(-1.0, 1.0);
                                        setter.set_parameter(tilt_p, clamped);
                                    }
                                    if resp.drag_stopped() { setter.end_set_parameter(tilt_p); }
                                    crate::editor::delayed_tooltip(ui, &resp,
                                        format!("Slot {} · {} · Tilt", editing_slot + 1, curve_label));
                                    ui.label(egui::RichText::new("Tilt").color(crv_col).size(th::scaled(th::FONT_SIZE_LABEL, scale)));
                                });
                            }

                            if let Some(curv_p) = params.curvature_param(editing_slot, editing_curve) {
                                let mut curv_val = curv_p.value();
                                ui.vertical(|ui| {
                                    let resp = ui.add(
                                        egui::DragValue::new(&mut curv_val)
                                            .range(0.0..=1.0)
                                            .speed(1.0 / 300.0)
                                            .fixed_decimals(2)
                                    );
                                    if resp.drag_started() { setter.begin_set_parameter(curv_p); }
                                    if resp.changed() { setter.set_parameter(curv_p, curv_val.clamp(0.0, 1.0)); }
                                    if resp.drag_stopped() { setter.end_set_parameter(curv_p); }
                                    crate::editor::delayed_tooltip(ui, &resp,
                                        format!("Slot {} · {} · Curvature", editing_slot + 1, curve_label));
                                    ui.label(egui::RichText::new("Curv").color(crv_col).size(th::scaled(th::FONT_SIZE_LABEL, scale)));
                                });
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
                            .size(th::scaled(th::FONT_SIZE_LABEL, scale)),
                    );
                    ui.add_space(2.0);

                    // Snapshot current state from params
                    let edit_slot  = *params.editing_slot.lock() as usize;
                    let types_snap = *params.slot_module_types.lock();
                    let names_snap = *params.slot_names.lock();

                    // ScrollArea allows the matrix to scroll when the window is too short
                    // to display all rows (e.g. at large scale on a small screen).
                    let interaction = {
                        let mut route_guard = params.route_matrix.lock();
                        let route_matrix_ref = &mut *route_guard;
                        egui::ScrollArea::vertical()
                            .id_salt("matrix_scroll")
                            .show(ui, |ui| {
                                crate::editor::fx_matrix_grid::paint_fx_matrix_grid(
                                    ui,
                                    &types_snap,
                                    &names_snap,
                                    route_matrix_ref,
                                    edit_slot,
                                    scale,
                                )
                            })
                            .inner
                    };
                    if let Some(new_slot) = interaction.left_click_slot {
                        *params.editing_slot.lock() = new_slot as u8;
                    }
                    // Handle right-click → open module assignment popup
                    if let Some((slot, pos)) = interaction.right_click {
                        crate::editor::module_popup::open_popup(ui, slot, pos);
                    }
                    // Handle right-click on a send cell → open amp popup
                    if let Some((row, col, pos)) = interaction.amp_right_click {
                        crate::editor::amp_popup::open_at(ui, row, col, pos);
                    }
                    // Render popups (egui Area — appears above matrix)
                    let _ = crate::editor::module_popup::show_popup(ui, &params, scale);
                    let _ = crate::editor::amp_popup::show_popup(ui, &params, scale);
                    let _ = crate::editor::life_popup::show_popup(ui, &params, scale);

                    // Persist preset menu state across frames via egui temp storage.
                    ui.ctx().data_mut(|d| d.insert_temp(preset_key, preset_state.clone()));
                });
        },
    )
}

/// Per-slot sidechain strip: SC gain knob (−90…+18 dB, ≤ −90 shown as "−∞")
/// and SC channel selector (Follow / L+R / L / R / M / S).
/// Rendered only for SC-aware modules (see `ModuleSpec::supports_sidechain`).
fn sc_strip_ui(
    ui: &mut egui::Ui,
    params: &SpectralForgeParams,
    slot_idx: usize,
    scale: f32,
) {
    use crate::params::ScChannel;

    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("SC").color(th::LABEL_DIM).size(th::scaled(th::FONT_SIZE_LABEL, scale)));
        // SC gain knob
        {
            let cur = params.slot_sc_gain_db.lock()[slot_idx];
            let mut g = cur;
            let resp = ui.add(
                egui::DragValue::new(&mut g)
                    .range(-90.0..=18.0)
                    .speed(0.1)
                    .suffix(" dB")
                    .custom_formatter(|v, _| {
                        if v <= -90.0 { "−∞".to_owned() } else { format!("{:.1}", v) }
                    })
            );
            if resp.changed() {
                params.slot_sc_gain_db.lock()[slot_idx] = g;
            }
        }
        ui.separator();
        // SC channel selector
        {
            let cur = params.slot_sc_channel.lock()[slot_idx];
            let label = match cur {
                ScChannel::Follow => "Follow",
                ScChannel::LR => "L+R",
                ScChannel::L  => "L",
                ScChannel::R  => "R",
                ScChannel::M  => "M",
                ScChannel::S  => "S",
            };
            egui::ComboBox::new(("sc_chan_slot", slot_idx), "Source")
                .selected_text(label)
                .show_ui(ui, |ui| {
                    for (v, text) in [
                        (ScChannel::Follow, "Follow"),
                        (ScChannel::LR,     "L+R"),
                        (ScChannel::L,      "L"),
                        (ScChannel::R,      "R"),
                        (ScChannel::M,      "M"),
                        (ScChannel::S,      "S"),
                    ] {
                        if ui.selectable_label(cur == v, text).clicked() {
                            params.slot_sc_channel.lock()[slot_idx] = v;
                        }
                    }
                });
        }
    });
}
