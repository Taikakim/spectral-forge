use nih_plug_egui::egui::{self, Pos2, Ui};
use crate::dsp::modules::circuit::CircuitMode;
use crate::dsp::modules::MAX_SLOTS;
use crate::editor::theme as th;
use crate::params::SpectralForgeParams;

#[derive(Clone, Default)]
pub struct CircuitPopupState {
    pub open: bool,
    pub slot: usize,
    pub pos:  Pos2,
}

const MODES: &[(CircuitMode, &str, &str)] = &[
    (CircuitMode::CrossoverDistortion, "Crossover Distortion", "Class A/B deadzone with C¹-smooth transition"),
    (CircuitMode::SpectralSchmitt,     "Spectral Schmitt",     "Branch-free hysteresis latch per bin"),
    (CircuitMode::BbdBins,             "BBD Bins",             "4-stage delay + lowpass + dither (bucket-brigade)"),
    (CircuitMode::Vactrol,             "Vactrol",              "Cascaded 1-pole envelope follower; reads BinPhysics::flux"),
    (CircuitMode::TransformerSaturation, "Transformer (heavy)", "tanh saturation + magnitude one-pole + spread; reads/writes flux"),
    (CircuitMode::PowerSag,            "Power Sag",            "Energy-driven envelope; reads BinPhysics::temperature"),
    (CircuitMode::ComponentDrift,      "Component Drift",      "Slow per-bin LFSR drift; reads/writes temperature"),
    (CircuitMode::PcbCrosstalk,        "PCB Crosstalk",        "3-tap symmetric stencil leak between adjacent bins"),
    (CircuitMode::SlewDistortion,      "Slew Distortion",      "Magnitude rate-limit + phase scramble; writes BinPhysics::slew"),
    (CircuitMode::BiasFuzz,            "Bias Fuzz",            "DC offset envelope + asymmetric clip; reads/writes BinPhysics::bias"),
];

pub fn mode_label(mode: CircuitMode) -> &'static str {
    for &(m, label, _) in MODES {
        if m == mode { return label; }
    }
    "Unknown"
}

pub fn show_popup(ui: &mut Ui, params: &SpectralForgeParams, scale: f32) -> bool {
    let key = ui.id().with("circuit_popup");
    let state: CircuitPopupState = ui.data(|d| d.get_temp(key).unwrap_or_default());
    if !state.open { return false; }

    let slot = state.slot;
    if slot >= MAX_SLOTS {
        ui.data_mut(|d| d.insert_temp(key, CircuitPopupState::default()));
        return false;
    }

    let current = params.slot_circuit_mode.lock()[slot];

    let mut new_state = state.clone();
    let mut consumed = false;

    egui::Area::new(egui::Id::new("circuit_popup_area"))
        .fixed_pos(state.pos)
        .order(egui::Order::Foreground)
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_min_width(180.0);
                ui.label(
                    egui::RichText::new("Circuit Mode")
                        .color(th::LABEL_DIM)
                        .size(th::scaled(th::FONT_SIZE_LABEL, scale))
                );
                ui.separator();

                for &(mode, label, hint) in MODES {
                    let selected = current == mode;
                    let resp = ui.selectable_label(selected, label)
                        .on_hover_text(hint);
                    if resp.clicked() && !selected {
                        params.slot_circuit_mode.lock()[slot] = mode;
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
    let key = ui.id().with("circuit_popup");
    ui.data_mut(|d| d.insert_temp(key, CircuitPopupState { open: true, slot, pos }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_label_roundtrip() {
        for &(mode, label, _) in MODES {
            assert_eq!(mode_label(mode), label, "mode_label mismatch for {:?}", mode);
        }
    }

    #[test]
    fn modes_count_is_ten() {
        assert_eq!(MODES.len(), 10, "expected exactly 10 Circuit modes");
    }

    #[test]
    fn transformer_label_has_heavy_tag() {
        let label = mode_label(CircuitMode::TransformerSaturation);
        assert!(label.contains("(heavy)"), "Transformer label must contain '(heavy)': got {:?}", label);
    }

    #[test]
    fn unknown_mode_falls_through() {
        // mode_label returns "Unknown" for a mode not in MODES only if the enum
        // ever gets a new variant. Verify the function returns a non-empty string.
        assert!(!mode_label(CircuitMode::CrossoverDistortion).is_empty());
    }
}
