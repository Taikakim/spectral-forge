//! Per-slot panel widget for `PastModule`.
//!
//! Hosts the module-wide Soft Clip toggle (always visible) plus the
//! mode-specific scalar fields:
//!   * DecaySorter → Floor (Hz)
//!   * Reverse     → Window (s)
//!   * Stretch     → Rate (×) + Dither (%)
//!   * Granular / Convolution → toggle only
//!
//! See spec docs/superpowers/specs/2026-05-04-past-module-ux-design.md §2 + §3.
//!
//! Wired through `ModuleSpec::panel_widget` for `ModuleType::Past`. Runs on
//! the GUI thread; uses `ParamSetter` so host automation is notified
//! correctly on drag begin/change/end.

use nih_plug::prelude::ParamSetter;
use nih_plug_egui::egui::{self, Ui};

use crate::dsp::modules::past::PastMode;
use crate::editor::theme as th;
use crate::params::SpectralForgeParams;

/// Render the Past slot panel. Signature matches `PanelWidgetFn`.
pub fn draw(
    ui: &mut Ui,
    params: &SpectralForgeParams,
    setter: &ParamSetter<'_>,
    slot: usize,
) {
    if slot >= 9 { return; }

    let scale = *params.ui_scale.lock();
    let mode  = params.slot_past_mode.lock()[slot];

    ui.horizontal(|ui| {
        // ── Soft Clip toggle (always visible) ────────────────────────────
        if let Some(p) = params.past_soft_clip_param(slot) {
            let mut on = p.value();
            let resp = ui.checkbox(&mut on, "Soft Clip");
            if resp.changed() {
                setter.begin_set_parameter(p);
                setter.set_parameter(p, on);
                setter.end_set_parameter(p);
            }
        }

        // ── Mode-specific scalars ────────────────────────────────────────
        match mode {
            PastMode::DecaySorter => {
                if let Some(p) = params.past_floor_param(slot) {
                    add_label(ui, scale, "Floor");
                    let mut v = p.value();
                    let resp = ui.add(
                        egui::DragValue::new(&mut v)
                            .range(20.0..=2000.0)
                            .speed(2.0)
                            .fixed_decimals(0)
                            .suffix(" Hz"),
                    );
                    handle_float_change(setter, p, &resp, v, 20.0, 2000.0);
                }
            }
            PastMode::Reverse => {
                if let Some(p) = params.past_reverse_window_param(slot) {
                    add_label(ui, scale, "Window");
                    let mut v = p.value();
                    let resp = ui.add(
                        egui::DragValue::new(&mut v)
                            .range(0.05..=30.0)
                            .speed(0.05)
                            .fixed_decimals(2)
                            .suffix(" s"),
                    );
                    handle_float_change(setter, p, &resp, v, 0.05, 30.0);
                }
            }
            PastMode::Stretch => {
                if let Some(p) = params.past_stretch_rate_param(slot) {
                    add_label(ui, scale, "Rate");
                    let mut v = p.value();
                    let resp = ui.add(
                        egui::DragValue::new(&mut v)
                            .range(0.05..=4.0)
                            .speed(0.01)
                            .fixed_decimals(2)
                            .suffix("\u{00d7}"),
                    );
                    handle_float_change(setter, p, &resp, v, 0.05, 4.0);
                }
                if let Some(p) = params.past_stretch_dither_param(slot) {
                    add_label(ui, scale, "Dither");
                    let mut v = p.value();
                    let resp = ui.add(
                        egui::DragValue::new(&mut v)
                            .range(0.0..=100.0)
                            .speed(0.5)
                            .fixed_decimals(0)
                            .suffix(" %"),
                    );
                    handle_float_change(setter, p, &resp, v, 0.0, 100.0);
                }
            }
            PastMode::Granular | PastMode::Convolution => {
                // No mode-specific scalars; the row stays compact.
            }
        }
    });
}

fn add_label(ui: &mut Ui, scale: f32, text: &str) {
    ui.label(
        egui::RichText::new(text)
            .size(th::scaled(th::FONT_SIZE_LABEL, scale))
            .color(th::LABEL_DIM),
    );
}

fn handle_float_change(
    setter: &ParamSetter<'_>,
    p: &nih_plug::prelude::FloatParam,
    resp: &egui::Response,
    new_val: f32,
    lo: f32,
    hi: f32,
) {
    if resp.drag_started() { setter.begin_set_parameter(p); }
    if resp.changed()      { setter.set_parameter(p, new_val.clamp(lo, hi)); }
    if resp.drag_stopped() { setter.end_set_parameter(p); }
}
