//! Per-slot panel widget for `KineticsModule` — dev-build only.
//!
//! Renders mode-conditional tuning scalar knobs for the currently selected
//! Kinetics mode. See spec docs/superpowers/specs/2026-05-09-prototyping-exposable-scalars-design.md §2.

#![cfg(feature = "dev-build")]

use nih_plug::prelude::ParamSetter;
use nih_plug_egui::egui::{self, Ui};

use crate::dsp::modules::kinetics::KineticsMode;
use crate::editor::theme as th;
use crate::params::SpectralForgeParams;

pub fn draw(ui: &mut Ui, params: &SpectralForgeParams, setter: &ParamSetter<'_>, slot: usize) {
    if slot >= 9 { return; }
    let scale = *params.ui_scale.lock();
    let mode  = params.slot_kinetics_mode.lock()[slot];

    ui.horizontal(|ui| {
        match mode {
            KineticsMode::GravityWell => {
                scalar_drag(ui, scale, setter, "Static Baseline",
                    params.kinetics_static_well_baseline_param(slot),
                    1.0, 2.0, 0.005, 3);
                scalar_drag(ui, scale, setter, "SC Threshold",
                    params.kinetics_sc_well_threshold_frac_param(slot),
                    0.1, 0.9, 0.01, 2);
                scalar_drag(ui, scale, setter, "SC Tau (hops)",
                    params.kinetics_sc_envelope_tau_hops_param(slot),
                    0.5, 4.0, 0.05, 2);
            }
            KineticsMode::InertialMass => {
                scalar_drag(ui, scale, setter, "SC Mass Scale",
                    params.kinetics_sc_mass_rate_scale_param(slot),
                    0.5, 10.0, 0.05, 2);
                scalar_drag(ui, scale, setter, "SC Tau (hops)",
                    params.kinetics_sc_envelope_tau_hops_param(slot),
                    0.5, 4.0, 0.05, 2);
            }
            KineticsMode::OrbitalPhase => {
                scalar_drag(ui, scale, setter, "Sat Half-Win (bins)",
                    params.kinetics_orbital_sat_half_window_param(slot),
                    4.0, 32.0, 1.0, 0);
                scalar_drag(ui, scale, setter, "Peak Thresh x",
                    params.kinetics_orbital_peak_threshold_factor_param(slot),
                    1.0, 5.0, 0.05, 2);
            }
            KineticsMode::TuningFork => {
                scalar_drag(ui, scale, setter, "Min Sep (bins)",
                    params.kinetics_tuning_fork_min_sep_param(slot),
                    1.0, 16.0, 1.0, 0);
            }
            // Hooke / Ferromagnetism / ThermalExpansion / Diamagnet have no scalar.
            _ => {}
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
