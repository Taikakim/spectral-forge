//! Per-slot panel widget for `ContrastModule` — dev-build only.
//!
//! Renders a 3-button mode picker (Spatial / Temporal / Tilt) plus mode-conditional
//! scalar knobs. See spec docs/superpowers/specs/2026-05-09-prototyping-exposable-scalars-design.md §5.

#![cfg(feature = "dev-build")]

use nih_plug::prelude::ParamSetter;
use nih_plug_egui::egui::{self, Ui};

use crate::dsp::modules::contrast::ContrastMode;
use crate::editor::theme as th;
use crate::params::SpectralForgeParams;

pub fn draw(ui: &mut Ui, params: &SpectralForgeParams, setter: &ParamSetter<'_>, slot: usize) {
    if slot >= 9 { return; }
    let scale = *params.ui_scale.lock();
    let mut mode = params.slot_contrast_mode.lock()[slot];

    ui.horizontal(|ui| {
        // Mode picker — 3 buttons
        for (m, label) in [
            (ContrastMode::Spatial,  "Spatial"),
            (ContrastMode::Temporal, "Temporal"),
            (ContrastMode::Tilt,     "Tilt"),
        ] {
            let selected = mode == m;
            let resp = ui.selectable_label(selected, label);
            if resp.clicked() && !selected {
                mode = m;
                params.slot_contrast_mode.lock()[slot] = m;
            }
        }

        ui.separator();

        match mode {
            ContrastMode::Spatial => {
                scalar_drag(ui, scale, setter, "Mean Win (st)",
                    params.contrast_mean_window_st_param(slot), 0.1, 24.0, 0.05, 2);
            }
            ContrastMode::Temporal => {
                // No extra scalar — Temporal uses ATTACK/RELEASE curves as time constants.
                ui.label(
                    egui::RichText::new("Uses ATTACK/RELEASE curves")
                        .size(th::scaled(th::FONT_SIZE_LABEL, scale))
                        .color(th::LABEL_DIM),
                );
            }
            ContrastMode::Tilt => {
                scalar_drag(ui, scale, setter, "Tilt (dB/oct)",
                    params.contrast_tilt_slope_db_per_oct_param(slot), -6.0, 6.0, 0.05, 2);
            }
        }
    });
}

fn scalar_drag(
    ui: &mut Ui,
    scale: f32,
    setter: &ParamSetter<'_>,
    label: &str,
    param: Option<&nih_plug::prelude::FloatParam>,
    lo: f32,
    hi: f32,
    speed: f32,
    decimals: usize,
) {
    if let Some(p) = param {
        ui.label(
            egui::RichText::new(label)
                .size(th::scaled(th::FONT_SIZE_LABEL, scale))
                .color(th::LABEL_DIM),
        );
        let mut v = p.value();
        let resp = ui.add(
            egui::DragValue::new(&mut v)
                .range(lo..=hi)
                .speed(speed)
                .fixed_decimals(decimals),
        );
        if resp.drag_started() { setter.begin_set_parameter(p); }
        if resp.changed()      { setter.set_parameter(p, v.clamp(lo, hi)); }
        if resp.drag_stopped() { setter.end_set_parameter(p); }
    }
}
