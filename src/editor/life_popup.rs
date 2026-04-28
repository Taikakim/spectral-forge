use nih_plug_egui::egui::{self, Pos2, Ui};
use crate::dsp::modules::life::LifeMode;
use crate::dsp::modules::MAX_SLOTS;
use crate::editor::theme as th;
use crate::params::SpectralForgeParams;

#[derive(Clone, Default)]
pub struct LifePopupState {
    pub open: bool,
    pub slot: usize,
    pub pos:  Pos2,
}

const MODES: &[(LifeMode, &str, &str)] = &[
    (LifeMode::Viscosity,       "Viscosity",       "FTCS finite-volume diffusion across adjacent bins"),
    (LifeMode::SurfaceTension,  "Surface Tension", "Coalesce adjacent spectral peaks toward the stronger"),
    (LifeMode::Crystallization, "Crystallization", "Sustained tones lock to a stable phase"),
    (LifeMode::Archimedes,      "Archimedes",      "Volume-conserving global ducking by overflow"),
    (LifeMode::NonNewtonian,    "Non-Newtonian",   "Rate-limit fast transients; slow signals pass freely"),
    (LifeMode::Stiction,        "Stiction",        "Static + kinetic friction: bins resist small movements"),
    (LifeMode::Yield,           "Yield",           "Fabric tearing at threshold with phase scramble"),
    (LifeMode::Capillary,       "Capillary",       "Wick energy upward into harmonic partials"),
    (LifeMode::Sandpaper,       "Sandpaper",       "Phase friction emits granular sparks up the spectrum"),
    (LifeMode::Brownian,        "Brownian",        "Temperature-driven random walk of bin magnitudes"),
];

pub fn mode_label(mode: LifeMode) -> &'static str {
    for &(m, label, _) in MODES {
        if m == mode { return label; }
    }
    "Unknown"
}

pub fn show_popup(ui: &mut Ui, params: &SpectralForgeParams, scale: f32) -> bool {
    let key = ui.id().with("life_popup");
    let state: LifePopupState = ui.data(|d| d.get_temp(key).unwrap_or_default());
    if !state.open { return false; }

    let slot = state.slot;
    if slot >= MAX_SLOTS {
        ui.data_mut(|d| d.insert_temp(key, LifePopupState::default()));
        return false;
    }

    let current = params.slot_life_mode.lock()[slot];

    let mut new_state = state.clone();
    let mut consumed = false;

    egui::Area::new(egui::Id::new("life_popup_area"))
        .fixed_pos(state.pos)
        .order(egui::Order::Foreground)
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_min_width(180.0);
                ui.label(
                    egui::RichText::new("Life Mode")
                        .color(th::LABEL_DIM)
                        .size(th::scaled(th::FONT_SIZE_LABEL, scale))
                );
                ui.separator();

                for &(mode, label, hint) in MODES {
                    let selected = current == mode;
                    let resp = ui.selectable_label(selected, label)
                        .on_hover_text(hint);
                    if resp.clicked() && !selected {
                        params.slot_life_mode.lock()[slot] = mode;
                        new_state.open = false;
                        consumed = true;
                    }
                }

                ui.separator();
                if ui.button("Close").clicked() {
                    new_state.open = false;
                    consumed = true;
                }
            });
        });

    ui.data_mut(|d| d.insert_temp(key, new_state));
    consumed
}

pub fn open_at(ui: &mut Ui, slot: usize, pos: Pos2) {
    let key = ui.id().with("life_popup");
    ui.data_mut(|d| d.insert_temp(key, LifePopupState { open: true, slot, pos }));
}
