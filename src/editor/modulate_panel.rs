//! Per-slot panel widget for `ModulateModule` — dev-build only.
//!
//! Renders PllTear mode tuning knobs. Other ModulateModes show no scalars.
//! See spec docs/superpowers/specs/2026-05-09-prototyping-exposable-scalars-design.md §4.

#![cfg(feature = "dev-build")]

use nih_plug::prelude::ParamSetter;
use nih_plug_egui::egui::{self, Ui};

use crate::dsp::modules::modulate::ModulateMode;
use crate::editor::theme as th;
use crate::params::SpectralForgeParams;

pub fn draw(ui: &mut Ui, params: &SpectralForgeParams, setter: &ParamSetter<'_>, slot: usize) {
    if slot >= 9 { return; }
    let scale = *params.ui_scale.lock();
    let mode  = params.slot_modulate_mode.lock()[slot];

    ui.horizontal(|ui| {
        if matches!(mode, ModulateMode::PllTear) {
            scalar_drag(ui, scale, setter, "Damping",
                params.modulate_damping_param(slot), 0.1, 2.0, 0.01, 3);
            scalar_drag(ui, scale, setter, "Tear Angle (rad)",
                params.modulate_tear_angle_rad_param(slot), 0.39269908, 3.14159265, 0.01, 3);
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
