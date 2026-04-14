use nih_plug::prelude::*;
use nih_plug_egui::{create_egui_editor, egui};
use parking_lot::Mutex;
use triple_buffer::Input as TbInput;
use std::sync::Arc;
use crate::params::{SpectralForgeParams, NUM_CURVE_SETS};
use crate::editor::{curve as crv, spectrum_display as sd, theme as th};

const CURVE_LABELS: [&str; NUM_CURVE_SETS] =
    ["THRESHOLD", "RATIO", "ATTACK", "RELEASE", "KNEE", "MAKEUP", "MIX"];

pub fn create_editor(
    params: Arc<SpectralForgeParams>,
    curve_tx: Vec<Arc<Mutex<TbInput<Vec<f32>>>>>,
    sample_rate: Option<Arc<crate::bridge::AtomicF32>>,
    num_bins: usize,
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
            // Close the window if the plugin instance has been destroyed.
            if plugin_alive.upgrade().is_none() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                return;
            }
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE.fill(th::BG))
                .show(ctx, |ui| {
                    let active_idx = *params.active_curve.lock() as usize;
                    let sr        = sample_rate.as_ref().map(|a| a.load()).unwrap_or(44100.0);
                    let db_min    = *params.graph_db_min.lock();
                    let db_max    = *params.graph_db_max.lock();
                    let falloff   = *params.peak_falloff_ms.lock();
                    let atk_ms   = params.attack_ms.value();
                    let rel_ms   = params.release_ms.value();
                    let freq_sc  = params.freq_scale.value();

                    // ── Top bar: curve selectors + dB range/falloff controls ──────
                    ui.horizontal(|ui| {
                        ui.add_space(4.0);
                        for (i, label) in CURVE_LABELS.iter().enumerate() {
                            let is_active = active_idx == i;
                            let (fill, text_color, stroke_color) = if is_active {
                                (th::curve_color_lit(i), th::curve_color_text_on(i), th::curve_color_lit(i))
                            } else {
                                (th::curve_color_dim(i), th::curve_color_lit(i), th::curve_color_dim(i))
                            };
                            let btn = egui::Button::new(
                                egui::RichText::new(*label)
                                    .color(text_color)
                                    .size(11.0),
                            )
                            .fill(fill)
                            .stroke(egui::Stroke::new(th::STROKE_BORDER, stroke_color));
                            if ui.add(btn).clicked() {
                                *params.active_curve.lock() = i as u8;
                            }
                        }

                        ui.add_space(12.0);
                        ui.label(egui::RichText::new("Floor").color(th::LABEL_DIM).size(9.0));
                        {
                            let mut v = *params.graph_db_min.lock();
                            if ui.add(
                                egui::DragValue::new(&mut v)
                                    .range(-160.0..=-20.0)
                                    .suffix(" dB")
                                    .speed(0.5),
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
                                    .suffix(" dB")
                                    .speed(0.5),
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
                                    .suffix(" ms")
                                    .speed(10.0),
                            ).changed() {
                                *params.peak_falloff_ms.lock() = v;
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

                    // ── Curve area ───────────────────────────────────────────────
                    let strip_height = 80.0;
                    let avail = ui.available_rect_before_wrap();
                    let curve_rect = egui::Rect::from_min_max(
                        avail.min,
                        egui::pos2(avail.max.x, (avail.max.y - strip_height).max(avail.min.y)),
                    );
                    ui.allocate_rect(curve_rect, egui::Sense::hover());

                    // Cache all 7 display-resolution curve responses
                    let nodes_snapshot = *params.curve_nodes.lock();
                    let cache_key = ui.id().with("all_display_gains");
                    let cached: Option<([[crv::CurveNode; 6]; NUM_CURVE_SETS], Vec<Vec<f32>>)> =
                        ui.data(|d| d.get_temp(cache_key));
                    let all_gains: Vec<Vec<f32>> = match cached {
                        Some((cached_nodes, cached_gains)) if cached_nodes == nodes_snapshot => {
                            cached_gains
                        }
                        _ => {
                            let g: Vec<Vec<f32>> = (0..NUM_CURVE_SETS)
                                .map(|i| crv::compute_curve_response(
                                    &nodes_snapshot[i],
                                    512,
                                    sr,
                                    crate::dsp::pipeline::FFT_SIZE,
                                ))
                                .collect();
                            ui.data_mut(|d| d.insert_temp(cache_key, (nodes_snapshot, g.clone())));
                            g
                        }
                    };

                    // Read spectrum (raw FFT magnitudes) and suppression (dB reduction) from bridge
                    let mut raw_magnitudes: Option<Vec<f32>> = None;
                    let mut suppression_data: Vec<f32> = Vec::new();
                    if let Some(ref rx_arc) = spectrum_rx {
                        if let Some(mut rx) = rx_arc.try_lock() {
                            raw_magnitudes = Some(rx.read().to_vec());
                        }
                    }
                    if let Some(ref rx_arc) = suppression_rx {
                        if let Some(mut rx) = rx_arc.try_lock() {
                            suppression_data = rx.read().to_vec();
                        }
                    }

                    // Peak-hold decay — normalize raw FFT magnitudes to 0 dBFS first.
                    // FFT bin magnitude for a 0 dBFS sine ≈ FFT_SIZE/4, so multiply by 4/FFT_SIZE.
                    let peak_key = ui.id().with("peak_hold");
                    let mut peak_hold: Vec<f32> = ui.data(|d| d.get_temp(peak_key))
                        .unwrap_or_default();

                    // 1. Grid (always painted)
                    crv::paint_grid(ui.painter(), curve_rect, active_idx, db_min, db_max, sr);

                    // 2. Spectrum + suppression gradient
                    if let Some(ref mags) = raw_magnitudes {
                        // Normalize to dBFS: 0 dBFS sine → magnitude 1.0
                        let norm = 4.0 / crate::dsp::pipeline::FFT_SIZE as f32;
                        let norm_mags: Vec<f32> = mags.iter().map(|m| m * norm).collect();
                        sd::decay_peak_hold(&norm_mags, &mut peak_hold, falloff, 1.0 / 60.0);
                        ui.data_mut(|d| d.insert_temp(peak_key, peak_hold.clone()));
                        let held_linear = sd::hold_to_linear(&peak_hold);
                        sd::paint_spectrum_and_suppression(
                            ui.painter(),
                            curve_rect,
                            &held_linear,
                            &suppression_data,
                            db_min,
                            db_max,
                            false, // sidechain overlay: wired up when SC routing is exposed
                            sr,
                            crate::dsp::pipeline::FFT_SIZE,
                        );
                    }

                    // 3. All 7 response curves — inactive (dim/1px) first, active (lit/2px) on top
                    for i in 0..NUM_CURVE_SETS {
                        if i == active_idx { continue; }
                        crv::paint_response_curve(
                            ui.painter(), curve_rect, &all_gains[i], i,
                            th::curve_color_dim(i), 1.0,
                            db_min, db_max, atk_ms, rel_ms, freq_sc, sr,
                            crate::dsp::pipeline::FFT_SIZE,
                        );
                    }
                    crv::paint_response_curve(
                        ui.painter(), curve_rect, &all_gains[active_idx], active_idx,
                        th::curve_color_lit(active_idx), 2.0,
                        db_min, db_max, atk_ms, rel_ms, freq_sc, sr,
                        crate::dsp::pipeline::FFT_SIZE,
                    );

                    // 4. Interactive nodes for the active curve
                    let mut nodes = nodes_snapshot[active_idx];
                    if crv::curve_widget(
                        ui, curve_rect, &mut nodes, &all_gains[active_idx],
                        active_idx, db_min, db_max, atk_ms, rel_ms, sr,
                        crate::dsp::pipeline::FFT_SIZE,
                    ) {
                        params.curve_nodes.lock()[active_idx] = nodes;
                        if num_bins > 0 {
                            let full_gains = crv::compute_curve_response(
                                &nodes,
                                num_bins,
                                sr,
                                crate::dsp::pipeline::FFT_SIZE,
                            );
                            if let Some(tx_arc) = curve_tx.get(active_idx) {
                                if let Some(mut tx) = tx_arc.try_lock() {
                                    tx.input_buffer_mut().copy_from_slice(&full_gains);
                                    tx.publish();
                                }
                            }
                        }
                    }

                    // 5. Cursor tooltip — frequency + active-curve value near the pointer
                    let max_hz = (sr / 2.0).max(20_001.0);
                    if let Some(hover) = ui.input(|i| i.pointer.hover_pos()) {
                        if curve_rect.contains(hover) {
                            let freq = crv::screen_to_freq(hover.x, curve_rect, max_hz);
                            let val  = crv::screen_y_to_physical(hover.y, active_idx, db_min, db_max, curve_rect);
                            let unit = crv::CURVE_Y_UNIT[active_idx];
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
                            let label = format!("{}\n{}", freq_str, val_str);
                            let tip_pos = hover + egui::vec2(12.0, -28.0);
                            let font = egui::FontId::proportional(10.0);
                            // Dark backing rectangle for readability
                            let galley = ui.painter().layout_no_wrap(
                                label.clone(),
                                font.clone(),
                                th::GRID_TEXT,
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

                    // ── Control strip ────────────────────────────────────────────
                    ui.add_space(4.0);
                    ui.separator();
                    ui.add_space(2.0);

                    ui.horizontal(|ui| {
                        use nih_plug_egui::widgets::ParamSlider;

                        macro_rules! knob {
                            ($param:expr, $label:expr) => {{
                                ui.vertical(|ui| {
                                    ui.add(ParamSlider::for_param($param, setter).with_width(40.0));
                                    ui.label(
                                        egui::RichText::new($label)
                                            .color(th::LABEL_DIM)
                                            .size(9.0),
                                    );
                                });
                            }};
                        }

                        knob!(&params.input_gain,  "IN");
                        knob!(&params.output_gain, "OUT");
                        knob!(&params.mix,         "MIX");
                        knob!(&params.sc_gain,     "SC GAIN");

                        ui.add_space(8.0);

                        // Dynamics group box — egui::Frame with tight margins so it fits the strip.
                        // "Dynamics" is painted as an overlay on the top border after layout.
                        let dyn_frame = egui::Frame::new()
                            .stroke(egui::Stroke::new(th::STROKE_BORDER, th::GRID_LINE))
                            .inner_margin(egui::Margin { left: 4, right: 4, top: 4, bottom: 4 });
                        let dyn_resp = dyn_frame.show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.vertical(|ui| {
                                    ui.add(ParamSlider::for_param(&params.attack_ms, setter).with_width(40.0));
                                    ui.label(egui::RichText::new("Atk").color(th::LABEL_DIM).size(9.0));
                                });
                                ui.vertical(|ui| {
                                    ui.add(ParamSlider::for_param(&params.release_ms, setter).with_width(40.0));
                                    ui.label(egui::RichText::new("Rel").color(th::LABEL_DIM).size(9.0));
                                });
                                ui.vertical(|ui| {
                                    ui.add(ParamSlider::for_param(&params.freq_scale, setter).with_width(40.0));
                                    ui.label(egui::RichText::new("Freq").color(th::LABEL_DIM).size(9.0));
                                });
                                ui.vertical(|ui| {
                                    ui.add(ParamSlider::for_param(&params.sensitivity, setter).with_width(40.0));
                                    ui.label(egui::RichText::new("Sens").color(th::LABEL_DIM).size(9.0));
                                });
                            });
                        });
                        // Paint "Dynamics" label over the top-left border
                        let lbl_pos = dyn_resp.response.rect.left_top() + egui::vec2(4.0, 0.0);
                        ui.painter().text(
                            lbl_pos,
                            egui::Align2::LEFT_TOP,
                            "Dynamics",
                            egui::FontId::proportional(8.0),
                            th::LABEL_DIM,
                        );

                        ui.add_space(8.0);

                        // Toggle buttons
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
                });
        },
    )
}
