//! Per-slot panel widget for `CircuitModule` — dev-build only.
//!
//! Renders Vactrol mode time-constant knobs. Other CircuitModes show no scalars.
//! See spec docs/superpowers/specs/2026-05-09-prototyping-exposable-scalars-design.md §3.

#![cfg(feature = "dev-build")]

use nih_plug::prelude::ParamSetter;
use nih_plug_egui::egui::{self, Ui};

use crate::dsp::modules::circuit::CircuitMode;
use crate::editor::theme as th;
use crate::params::SpectralForgeParams;

pub fn draw(ui: &mut Ui, params: &SpectralForgeParams, setter: &ParamSetter<'_>, slot: usize) {
    if slot >= 9 { return; }
    let scale = *params.ui_scale.lock();
    let mode  = params.slot_circuit_mode.lock()[slot];

    ui.horizontal(|ui| {
        if matches!(mode, CircuitMode::Vactrol) {
            scalar_drag(ui, scale, setter, "Fast (ms)",
                params.circuit_vactrol_fast_ms_param(slot), 1.0, 50.0, 0.1, 1);
            scalar_drag(ui, scale, setter, "Slow (ms)",
                params.circuit_vactrol_slow_ms_param(slot), 50.0, 1000.0, 1.0, 1);
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
