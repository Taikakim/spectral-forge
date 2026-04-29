use nih_plug_egui::egui::{self, Pos2, Ui};
use crate::dsp::modules::kinetics::{KineticsMode, WellSource, MassSource};
use crate::dsp::modules::MAX_SLOTS;
use crate::editor::theme as th;
use crate::params::SpectralForgeParams;

#[derive(Clone, Default)]
pub struct KineticsPopupState {
    pub open: bool,
    pub slot: usize,
    pub pos:  Pos2,
}

const MODES: &[(KineticsMode, &str, &str)] = &[
    (KineticsMode::Hooke,            "Hooke",             "Spring restoring force diffuses energy across adjacent bins"),
    (KineticsMode::GravityWell,      "Gravity Well",      "Pull energy toward Static / Sidechain / MIDI source"),
    (KineticsMode::InertialMass,     "Inertial Mass",     "Per-bin mass written to BinPhysics for downstream readers"),
    (KineticsMode::OrbitalPhase,     "Orbital Phase",     "Phase orbits — satellites rotate opposite to planets"),
    (KineticsMode::Ferromagnetism,   "Ferromagnetism",    "Aligns neighbour phases to nearest spectral peak"),
    (KineticsMode::ThermalExpansion, "Thermal Expansion", "Heat accumulates and detunes phase (frequency drift)"),
    (KineticsMode::TuningFork,       "Tuning Fork",       "Peak-driven phase modulation of nearby neighbours"),
    (KineticsMode::Diamagnet,        "Diamagnet",         "Energy-conserving spectral carving with 1/d redistribution"),
];

const WELL_SOURCES: &[(WellSource, &str)] = &[
    (WellSource::Static,    "Static (curve-driven)"),
    (WellSource::Sidechain, "Sidechain (peak follower)"),
    (WellSource::MIDI,      "MIDI (note-on)"),
];

const MASS_SOURCES: &[(MassSource, &str)] = &[
    (MassSource::Static,    "Static (curve-driven)"),
    (MassSource::Sidechain, "Sidechain (rate of change)"),
];

pub fn mode_label(mode: KineticsMode) -> &'static str {
    for &(m, label, _) in MODES {
        if m == mode { return label; }
    }
    "Unknown"
}

/// Render the popup if open. Call every frame from the main UI closure.
/// Returns true if the popup consumed a click.
///
/// UX note: unlike life_popup (which closes on mode selection), this popup
/// stays open after a mode click so the user can also set the sub-source
/// for GravityWell or InertialMass in a single trip. Only the "Close" button
/// dismisses it.
pub fn show_popup(ui: &mut Ui, params: &SpectralForgeParams, scale: f32) -> bool {
    let key = ui.id().with("kinetics_popup");
    let state: KineticsPopupState = ui.data(|d| d.get_temp(key).unwrap_or_default());
    if !state.open { return false; }

    let slot = state.slot;
    if slot >= MAX_SLOTS {
        ui.data_mut(|d| d.insert_temp(key, KineticsPopupState::default()));
        return false;
    }

    let current_mode = params.slot_kinetics_mode.lock()[slot];

    let mut new_state = state.clone();
    let mut consumed = false;

    egui::Area::new(egui::Id::new("kinetics_popup_area"))
        .fixed_pos(state.pos)
        .order(egui::Order::Foreground)
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_min_width(200.0);
                ui.label(
                    egui::RichText::new("Kinetics Mode")
                        .color(th::LABEL_DIM)
                        .size(th::scaled(th::FONT_SIZE_LABEL, scale))
                );
                ui.separator();

                for &(mode, label, hint) in MODES {
                    let selected = current_mode == mode;
                    let resp = ui.selectable_label(selected, label)
                        .on_hover_text(hint);
                    if resp.clicked() && !selected {
                        params.slot_kinetics_mode.lock()[slot] = mode;
                        // Do NOT close: user may also want to set the sub-source
                        // for GravityWell or InertialMass. Close button dismisses.
                        consumed = true;
                    }
                }

                // Sub-source pickers — only visible for modes that have them.
                // Re-read mode here so a mode click on this same frame
                // (entering or leaving GravityWell/InertialMass) takes effect
                // immediately without a one-frame flicker.
                let mode_now = params.slot_kinetics_mode.lock()[slot];
                if mode_now == KineticsMode::GravityWell {
                    let current_source = params.slot_kinetics_well_source.lock()[slot];
                    ui.separator();
                    ui.label(
                        egui::RichText::new("Well Source")
                            .color(th::LABEL_DIM)
                            .size(th::scaled(th::FONT_SIZE_LABEL, scale))
                    );
                    for &(source, label) in WELL_SOURCES {
                        let selected = current_source == source;
                        let resp = ui.selectable_label(selected, label);
                        if resp.clicked() && !selected {
                            params.slot_kinetics_well_source.lock()[slot] = source;
                            consumed = true;
                        }
                    }
                } else if mode_now == KineticsMode::InertialMass {
                    let current_source = params.slot_kinetics_mass_source.lock()[slot];
                    ui.separator();
                    ui.label(
                        egui::RichText::new("Mass Source")
                            .color(th::LABEL_DIM)
                            .size(th::scaled(th::FONT_SIZE_LABEL, scale))
                    );
                    for &(source, label) in MASS_SOURCES {
                        let selected = current_source == source;
                        let resp = ui.selectable_label(selected, label);
                        if resp.clicked() && !selected {
                            params.slot_kinetics_mass_source.lock()[slot] = source;
                            consumed = true;
                        }
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

/// Open the popup for a slot at the given screen position.
pub fn open_at(ui: &mut Ui, slot: usize, pos: Pos2) {
    let key = ui.id().with("kinetics_popup");
    ui.data_mut(|d| d.insert_temp(key, KineticsPopupState { open: true, slot, pos }));
}
