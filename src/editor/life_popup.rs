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
    (LifeMode::Viscosity, "Viscosity",
     "Viscosity — FTCS finite-volume power diffusion smooths adjacent bin magnitudes like a fluid resistance. Higher AMOUNT = more spectral smoothing. Use it to take edge off harsh content without filtering. Sidechain: not used."),
    (LifeMode::SurfaceTension, "Surface Tension",
     "Surface Tension — strong peaks coalesce, stealing magnitude from weaker neighbours within REACH. Total spectral energy stays roughly conserved. Use it to consolidate a noisy cloud of bins into a single tonal core. Sidechain: not used."),
    (LifeMode::Crystallization, "Crystallization",
     "Crystallization — sustained tonal bins accumulate a crystallization envelope and lock to a stable phase. AMOUNT scales the growth rate; SPEED controls how fast the lock decays. Use it to glassify pads and held notes. Sidechain: not used."),
    (LifeMode::Archimedes, "Archimedes",
     "Archimedes — volume-conserving global ducking. When the total spectral magnitude exceeds the THRESHOLD capacity, every bin is scaled down proportionally. Use it as a self-balancing limiter that never colours the spectrum's shape. Sidechain: not used."),
    (LifeMode::NonNewtonian, "Non-Newtonian",
     "Non-Newtonian — oobleck behaviour: fast magnitude changes are clamped, slow ones pass freely. Squashes transients while leaving sustains intact. Sidechain: not used."),
    (LifeMode::Stiction, "Stiction",
     "Stiction — static + kinetic friction. Bins below a velocity threshold are held in place and decay to silence; moving bins pass freely. Use it to silence a wash of low-level noise while preserving moving content. Sidechain: not used."),
    (LifeMode::Yield, "Yield",
     "Yield — fabric tearing. Bins exceeding THRESHOLD have their phase scrambled and magnitude clamped; the tear heals over SPEED. Use it for controlled distortion that ramps with intensity. Sidechain: not used."),
    (LifeMode::Capillary, "Capillary",
     "Capillary — wicks energy upward through harmonic destinations. Source bins drain into harmonics above via a three-pass transport. Use it to pull a lo-fi signal into a richer harmonic spectrum. Sidechain: not used."),
    (LifeMode::Sandpaper, "Sandpaper",
     "Sandpaper — phase friction emits granular sparks of noise upward in frequency when bins rub against each other. Use it for textural high-end content tied to motion. Sidechain: not used."),
    (LifeMode::Brownian, "Brownian",
     "Brownian — temperature-driven random walk of per-bin magnitudes. AMOUNT scales the heat. Use it for unsettled, ever-shifting noise floors. Sidechain: not used."),
];

pub fn mode_label(mode: LifeMode) -> &'static str {
    for &(m, label, _) in MODES {
        if m == mode { return label; }
    }
    "Unknown"
}

pub fn mode_hint(mode: LifeMode) -> &'static str {
    for &(m, _, hint) in MODES {
        if m == mode { return hint; }
    }
    ""
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
                    crate::editor::help_box::track_help_strings(ui, &resp, label, hint);
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
