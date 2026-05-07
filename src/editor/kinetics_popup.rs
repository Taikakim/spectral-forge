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
    (KineticsMode::Hooke, "Hooke",
     "Hooke — spring restoring forces couple adjacent bins so energy diffuses sideways through the spectrum. STRENGTH sets the spring constant, REACH adds sympathetic harmonic resonators, DAMPING controls settling. Use it for soft, blooming spectral diffusion. Sidechain: not used."),
    (KineticsMode::GravityWell, "Gravity Well",
     "Gravity Well — Newtonian gravitational attraction pulls bins toward the well centre. Well source picked in this popup: Static (curve-driven), Sidechain (peak follower), or MIDI (note-on). Use it as a freeze-into-tone effect or a pitch quantizer driven by an external part. Sidechain: yes (when Sidechain source is selected)."),
    (KineticsMode::InertialMass, "Inertial Mass",
     "Inertial Mass — does not change audio; instead writes per-bin mass into BinPhysics so downstream Kinetics modes (Hooke, Gravity Well) see lighter/heavier bins. Mass source picked in this popup: Static curve or Sidechain rate-of-change. Sidechain: yes (Sidechain source)."),
    (KineticsMode::OrbitalPhase, "Orbital Phase",
     "Orbital Phase — detected spectral peaks become 'planets'; nearby satellite bins receive opposite-sign phase rotation, like moons in retrograde. STRENGTH sets the per-hop rotation. Use it for subtle phasing tied to the loudest content. Sidechain: not used."),
    (KineticsMode::Ferromagnetism, "Ferromagnetism",
     "Ferromagnetism — neighbour bins phase-align toward the nearest spectral peak's phase. Smooths phase relationships and tightens tonal clusters. Sidechain: not used."),
    (KineticsMode::ThermalExpansion, "Thermal Expansion",
     "Thermal Expansion — energy heats each bin; heat then drives a frequency-detune phase rotation that gradually cools. Loud sustains drift slightly out of tune over time. Sidechain: not used."),
    (KineticsMode::TuningFork, "Tuning Fork",
     "Tuning Fork — detects the strongest peak (the 'fork'), then sympathetic-resonates a cluster of nearby bins into matched modulation. Use it to make a single tone bloom into a small cluster of partials. Sidechain: not used."),
    (KineticsMode::Diamagnet, "Diamagnet",
     "Diamagnet — energy-conserving spectral carving: bins are pushed away from a target frequency and the carved magnitude is redistributed into neighbours by 1/d falloff. Use it as a notch-with-fill effect that doesn't change total spectral energy. Sidechain: not used."),
];

const WELL_SOURCES: &[(WellSource, &str, &str)] = &[
    (WellSource::Static,    "Static (curve-driven)",    "Well centre tracks the STRENGTH curve peak"),
    (WellSource::Sidechain, "Sidechain (peak follower)","Well centre tracks the loudest sidechain bin"),
    (WellSource::MIDI,      "MIDI (note-on)",           "Well centre snaps to the most-recent MIDI note frequency"),
];

const MASS_SOURCES: &[(MassSource, &str, &str)] = &[
    (MassSource::Static,    "Static (curve-driven)",       "Mass per bin = MASS curve value"),
    (MassSource::Sidechain, "Sidechain (rate of change)",  "Mass per bin scales with sidechain envelope velocity"),
];

pub fn mode_label(mode: KineticsMode) -> &'static str {
    for &(m, label, _) in MODES {
        if m == mode { return label; }
    }
    "Unknown"
}

pub fn mode_hint(mode: KineticsMode) -> &'static str {
    for &(m, _, hint) in MODES {
        if m == mode { return hint; }
    }
    ""
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
                    crate::editor::help_box::track_help_strings(ui, &resp, label, hint);
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
                    for &(source, label, hint) in WELL_SOURCES {
                        let selected = current_source == source;
                        let resp = ui.selectable_label(selected, label)
                            .on_hover_text(hint);
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
                    for &(source, label, hint) in MASS_SOURCES {
                        let selected = current_source == source;
                        let resp = ui.selectable_label(selected, label)
                            .on_hover_text(hint);
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
