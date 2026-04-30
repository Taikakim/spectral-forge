use nih_plug_egui::egui::{self, Pos2, Ui};
use crate::dsp::modules::harmony::{HarmonyMode, HarmonyInharmonicSubmode};
use crate::dsp::modules::MAX_SLOTS;
use crate::editor::theme as th;
use crate::params::SpectralForgeParams;

#[derive(Clone, Default)]
pub struct HarmonyPopupState {
    pub open: bool,
    pub slot: usize,
    pub pos:  Pos2,
}

const MODES: &[(HarmonyMode, &str, &str)] = &[
    (HarmonyMode::Chordification,   "Chordification",   "Snap chromagram to nearest of 24 major/minor chord templates"),
    (HarmonyMode::Undertone,        "Undertone",         "IF-driven sub-harmonics + ground-loop hum"),
    (HarmonyMode::Companding,       "Companding",        "Per-class harmonic-group attenuation"),
    (HarmonyMode::FormantRotation,  "Formant Rotation",  "Cepstrum-preserved spectral envelope rotation"),
    (HarmonyMode::Lifter,           "Lifter",            "Cepstrum-domain envelope/pitch shaping (heavy)"),
    (HarmonyMode::Inharmonic,       "Inharmonic",        "Stiffness / Bessel / Prime detuning"),
    (HarmonyMode::HarmonicGenerator,"Harmonic Generator","Synthesise harmonics from detected peaks"),
    (HarmonyMode::Shuffler,         "Shuffler",          "Random bin swaps within SPREAD-controlled reach"),
];

const INHARMONIC_SUBMODES: &[(HarmonyInharmonicSubmode, &str, &str)] = &[
    (HarmonyInharmonicSubmode::Stiffness, "Stiffness", "Piano-string inharmonicity: f_n = n·f0·sqrt(1 + B·n²)"),
    (HarmonyInharmonicSubmode::Bessel,    "Bessel",    "Circular-membrane mode ratios (Bessel J0 zeros)"),
    (HarmonyInharmonicSubmode::Prime,     "Prime",     "Prime-number harmonic series"),
];

pub fn mode_label(mode: HarmonyMode) -> &'static str {
    for &(m, label, _) in MODES {
        if m == mode { return label; }
    }
    "Unknown"
}

/// Render the popup if open. Call every frame from the main UI closure.
/// Returns true if the popup consumed a click.
///
/// UX note: stays open after a mode click so the user can also pick the
/// Inharmonic sub-mode in a single trip. Only the "Close" button dismisses.
pub fn show_popup(ui: &mut Ui, params: &SpectralForgeParams, scale: f32) -> bool {
    let key = ui.id().with("harmony_popup");
    let state: HarmonyPopupState = ui.data(|d| d.get_temp(key).unwrap_or_default());
    if !state.open { return false; }

    let slot = state.slot;
    if slot >= MAX_SLOTS {
        ui.data_mut(|d| d.insert_temp(key, HarmonyPopupState::default()));
        return false;
    }

    let current_mode = params.slot_harmony_mode.lock()[slot];

    let mut new_state = state.clone();
    let mut consumed = false;

    egui::Area::new(egui::Id::new("harmony_popup_area"))
        .fixed_pos(state.pos)
        .order(egui::Order::Foreground)
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_min_width(220.0);
                ui.label(
                    egui::RichText::new("Harmony Mode")
                        .color(th::LABEL_DIM)
                        .size(th::scaled(th::FONT_SIZE_LABEL, scale))
                );
                ui.separator();

                for &(mode, label, hint) in MODES {
                    let selected = current_mode == mode;
                    let resp = ui.selectable_label(selected, label)
                        .on_hover_text(hint);
                    if resp.clicked() && !selected {
                        params.slot_harmony_mode.lock()[slot] = mode;
                        // Do NOT close: user may also want to set Inharmonic sub-mode.
                        consumed = true;
                    }
                }

                // Inharmonic sub-mode picker — only visible when Inharmonic is active.
                // Re-read mode so a mode click on this same frame takes effect immediately.
                let mode_now = params.slot_harmony_mode.lock()[slot];
                if mode_now == HarmonyMode::Inharmonic {
                    let current_sub = params.slot_harmony_inharmonic_submode.lock()[slot];
                    ui.separator();
                    ui.label(
                        egui::RichText::new("Inharmonic Sub-mode")
                            .color(th::LABEL_DIM)
                            .size(th::scaled(th::FONT_SIZE_LABEL, scale))
                    );
                    for &(sub, label, hint) in INHARMONIC_SUBMODES {
                        let selected = current_sub == sub;
                        let resp = ui.selectable_label(selected, label)
                            .on_hover_text(hint);
                        if resp.clicked() && !selected {
                            params.slot_harmony_inharmonic_submode.lock()[slot] = sub;
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
    let key = ui.id().with("harmony_popup");
    ui.data_mut(|d| d.insert_temp(key, HarmonyPopupState { open: true, slot, pos }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_label_roundtrip() {
        for &(mode, label, _) in MODES {
            assert_eq!(mode_label(mode), label,
                "mode_label({:?}) should be {:?}", mode, label);
        }
    }

    #[test]
    fn modes_count_is_eight() {
        assert_eq!(MODES.len(), 8, "expected exactly 8 HarmonyMode variants");
    }

    #[test]
    fn modes_have_no_duplicate_variants() {
        let mut seen = std::collections::HashSet::new();
        for &(mode, _, _) in MODES {
            // Use Debug repr as a surrogate for identity (mode is PartialEq+Eq).
            let key = format!("{:?}", mode);
            assert!(seen.insert(key.clone()), "duplicate mode variant: {}", key);
        }
    }

    #[test]
    fn submodes_count_is_three() {
        assert_eq!(INHARMONIC_SUBMODES.len(), 3,
            "expected exactly 3 HarmonyInharmonicSubmode variants");
    }
}
