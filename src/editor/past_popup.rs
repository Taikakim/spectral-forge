use nih_plug_egui::egui::{self, Pos2, Ui};
use crate::dsp::modules::past::{PastMode, SortKey};
use crate::dsp::modules::MAX_SLOTS;
use crate::editor::theme as th;
use crate::params::SpectralForgeParams;

#[derive(Clone, Default)]
pub struct PastPopupState {
    pub open: bool,
    pub slot: usize,
    pub pos:  Pos2,
}

const MODES: &[(PastMode, &str, &str)] = &[
    (PastMode::Granular,    "Granular Window",     "Selective time-windowed freeze of stable bins"),
    (PastMode::DecaySorter, "Decay Sorter",        "Temporal reconstruction via summary-stat sorting"),
    (PastMode::Convolution, "Spectral Convolution","Per-bin self-resonance — convolve current with past"),
    (PastMode::Reverse,     "Reverse",             "Backward read of the history buffer"),
    (PastMode::Stretch,     "Stretch",             "Phase-coherent variable-rate playback (0.25\u{00d7} \u{2013} 4\u{00d7})"),
];

const SORT_KEYS: &[(SortKey, &str)] = &[
    (SortKey::Decay,     "Decay (ring time)"),
    (SortKey::Stability, "Stability (IF)"),
    (SortKey::Area,      "Area (RMS)"),
];

pub fn mode_label(mode: PastMode) -> &'static str {
    for &(m, label, _) in MODES {
        if m == mode { return label; }
    }
    "Unknown"
}

/// Render the popup if open. Call every frame from the main UI closure.
/// Returns true if the popup consumed a click.
///
/// UX note: unlike life_popup (which closes on mode selection), this popup
/// stays open after a mode click so the user can also set the sort key
/// for DecaySorter in a single trip. Only the "Close" button dismisses it.
pub fn show_popup(ui: &mut Ui, params: &SpectralForgeParams, scale: f32) -> bool {
    let key = ui.id().with("past_popup");
    let state: PastPopupState = ui.data(|d| d.get_temp(key).unwrap_or_default());
    if !state.open { return false; }

    let slot = state.slot;
    if slot >= MAX_SLOTS {
        ui.data_mut(|d| d.insert_temp(key, PastPopupState::default()));
        return false;
    }

    let current_mode = params.slot_past_mode.lock()[slot];
    let current_key  = params.slot_past_sort_key.lock()[slot];

    let mut new_state = state.clone();
    let mut consumed = false;

    egui::Area::new(egui::Id::new("past_popup_area"))
        .fixed_pos(state.pos)
        .order(egui::Order::Foreground)
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_min_width(200.0);
                ui.label(
                    egui::RichText::new("Past Mode")
                        .color(th::LABEL_DIM)
                        .size(th::scaled(th::FONT_SIZE_LABEL, scale))
                );
                ui.separator();

                for &(mode, label, hint) in MODES {
                    let selected = current_mode == mode;
                    let resp = ui.selectable_label(selected, label)
                        .on_hover_text(hint);
                    if resp.clicked() && !selected {
                        params.slot_past_mode.lock()[slot] = mode;
                        // Do NOT close: user may also want to set the sort key
                        // for DecaySorter. Close button dismisses.
                        consumed = true;
                    }
                }

                // Sort key sub-picker — only visible when mode is DecaySorter.
                if current_mode == PastMode::DecaySorter {
                    ui.separator();
                    ui.label(
                        egui::RichText::new("Sort Key")
                            .color(th::LABEL_DIM)
                            .size(th::scaled(th::FONT_SIZE_LABEL, scale))
                    );
                    for &(key, label) in SORT_KEYS {
                        let selected = current_key == key;
                        let resp = ui.selectable_label(selected, label);
                        if resp.clicked() && !selected {
                            params.slot_past_sort_key.lock()[slot] = key;
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
    let key = ui.id().with("past_popup");
    ui.data_mut(|d| d.insert_temp(key, PastPopupState { open: true, slot, pos }));
}
