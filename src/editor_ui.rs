use nih_plug::prelude::*;
use nih_plug_egui::{create_egui_editor, egui};
use parking_lot::Mutex;
use triple_buffer::Input as TbInput;
use std::sync::Arc;
use crate::params::{SpectralForgeParams, NUM_CURVE_SETS};
use crate::editor::theme as th;

const CURVE_LABELS: [&str; NUM_CURVE_SETS] =
    ["THRESHOLD", "RATIO", "ATTACK", "RELEASE", "KNEE", "MAKEUP", "MIX"];

pub fn create_editor(
    params: Arc<SpectralForgeParams>,
    curve_tx: Vec<Arc<Mutex<TbInput<Vec<f32>>>>>,
    sample_rate: Option<Arc<crate::bridge::AtomicF32>>,
    num_bins: usize,
) -> Option<Box<dyn Editor>> {
    create_egui_editor(
        params.editor_state.clone(),
        (),
        |ctx, _| {
            let mut visuals = egui::Visuals::dark();
            visuals.panel_fill = th::BG;
            ctx.set_visuals(visuals);
        },
        move |ctx, _setter, _state| {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE.fill(th::BG))
                .show(ctx, |ui| {
                    let active_idx = *params.active_curve.lock() as usize;

                    // Parameter selector row
                    ui.horizontal(|ui| {
                        ui.add_space(4.0);
                        for (i, label) in CURVE_LABELS.iter().enumerate() {
                            let is_active = active_idx == i;
                            let (fill, text_color) = if is_active {
                                (th::BTN_ACTIVE, th::BTN_TEXT_ON)
                            } else {
                                (th::BTN_INACTIVE, th::BTN_TEXT_OFF)
                            };
                            let btn = egui::Button::new(
                                egui::RichText::new(*label)
                                    .color(text_color)
                                    .size(11.0)
                            ).fill(fill)
                             .stroke(egui::Stroke::new(th::STROKE_BORDER, th::BORDER));
                            if ui.add(btn).clicked() {
                                *params.active_curve.lock() = i as u8;
                            }
                        }
                    });

                    ui.add_space(2.0);
                    let rect = ui.available_rect_before_wrap();
                    ui.painter().line_segment(
                        [rect.left_top(), rect.right_top()],
                        egui::Stroke::new(th::STROKE_BORDER, th::BORDER),
                    );

                    // Curve area
                    let curve_rect = ui.available_rect_before_wrap();
                    ui.allocate_rect(curve_rect, egui::Sense::hover());
                    let mut nodes = params.curve_nodes.lock()[active_idx];
                    let sr = sample_rate.as_ref().map(|a| a.load()).unwrap_or(44100.0);

                    // Paint response curve (using display resolution of 512 bins), cached per frame
                    let cache_key = ui.id().with("display_gains");
                    let cached: Option<(Vec<[crate::editor::curve::CurveNode; 6]>, Vec<f32>)> =
                        ui.data(|d| d.get_temp(cache_key));
                    let display_gains = if let Some((cached_nodes, cached_gains)) = cached {
                        if cached_nodes[active_idx] == nodes {
                            cached_gains
                        } else {
                            let g = crate::editor::curve::compute_curve_response(
                                &nodes, 512, sr, crate::dsp::pipeline::FFT_SIZE,
                            );
                            ui.data_mut(|d| d.insert_temp(cache_key, (params.curve_nodes.lock().clone(), g.clone())));
                            g
                        }
                    } else {
                        let g = crate::editor::curve::compute_curve_response(
                            &nodes, 512, sr, crate::dsp::pipeline::FFT_SIZE,
                        );
                        ui.data_mut(|d| d.insert_temp(cache_key, (params.curve_nodes.lock().clone(), g.clone())));
                        g
                    };
                    crate::editor::curve::paint_response_curve(ui, curve_rect, &display_gains);

                    // Handle node interaction
                    if crate::editor::curve::curve_widget(ui, curve_rect, &mut nodes) {
                        params.curve_nodes.lock()[active_idx] = nodes;
                        // Push full-resolution gains to audio bridge
                        if num_bins > 0 {
                            let full_gains = crate::editor::curve::compute_curve_response(
                                &nodes, num_bins, sr, crate::dsp::pipeline::FFT_SIZE,
                            );
                            if let Some(tx_arc) = curve_tx.get(active_idx) {
                                if let Some(mut tx) = tx_arc.try_lock() {
                                    tx.input_buffer_mut().copy_from_slice(&full_gains);
                                    tx.publish();
                                }
                            }
                        }
                    }
                });
        },
    )
}
