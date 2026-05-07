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
    (CircuitMode::CrossoverDistortion, "Crossover Distortion",
     "Crossover Distortion — class A/B-style deadzone around zero with a C¹-smooth re-emergence curve. Quiet content is silenced and sputters back when it crosses out of the deadzone. Use it for broken-radio fuzz on tails. Sidechain: not used."),
    (CircuitMode::SpectralSchmitt, "Spectral Schmitt",
     "Spectral Schmitt — branch-free hysteresis latch per bin with two trip points. Bins below the lower threshold are gated; once a bin crosses the upper trip it latches on until it drops below the lower again. Use it for sample-rate-style chunky gating. Sidechain: not used."),
    (CircuitMode::BbdBins, "BBD Bins",
     "BBD Bins — 4 cascaded LP stages + chip-noise dither emulate a bucket-brigade delay per bin. Smears magnitude over a few hops and adds analog-feeling grit. Sidechain: not used."),
    (CircuitMode::Vactrol, "Vactrol",
     "Vactrol — cascaded fast/slow opto-coupler caps act as a soft-saturating envelope follower. Reads BinPhysics::flux. RELEASE controls the slow-cap time constant for the classic ringing release character. Sidechain: not used."),
    (CircuitMode::TransformerSaturation, "Transformer (heavy)",
     "Transformer Saturation — tanh soft-clipping with magnitude one-pole and bin-spread coupling. Reads/writes BinPhysics::flux. Heavy-CPU mode. Use it to add a transformer-iron warmth to clean material. Sidechain: not used."),
    (CircuitMode::PowerSag, "Power Sag",
     "Power Sag — sustained loud bins drive a sag envelope that pulls the entire output level down, recovering after the load lifts. Reads BinPhysics::temperature so hot bins contribute more sag. Use it for that retro pumped-PSU character. Sidechain: not used."),
    (CircuitMode::ComponentDrift, "Component Drift",
     "Component Drift — slow per-bin pseudo-random gain wander driven by an LFSR. Reads/writes BinPhysics::temperature so hot bins drift further. Use it for analog-feeling instability. Sidechain: not used."),
    (CircuitMode::PcbCrosstalk, "PCB Crosstalk",
     "PCB Crosstalk — 3-tap symmetric stencil leaks energy between adjacent bins, like analog trace-coupling. Subtle smearing that adds analog character without filtering. Sidechain: not used."),
    (CircuitMode::SlewDistortion, "Slew Distortion",
     "Slew Distortion — magnitude rate-limit per bin. Slowed transients spit excess slew energy out as phase scramble, writing it into BinPhysics::slew for downstream readers. Use it as transient softening with controllable phase fuzz. Sidechain: not used."),
    (CircuitMode::BiasFuzz, "Bias Fuzz",
     "Bias Fuzz — DC offset envelope shifts the bin's zero-point off-centre, then loud transients clip asymmetrically against the top rail. Reads/writes BinPhysics::bias. Adds even-order harmonics to the magnitude envelope. Sidechain: not used."),
];

pub fn mode_label(mode: CircuitMode) -> &'static str {
    for &(m, label, _) in MODES {
        if m == mode { return label; }
    }
    "Unknown"
}

pub fn mode_hint(mode: CircuitMode) -> &'static str {
    for &(m, _, hint) in MODES {
        if m == mode { return hint; }
    }
    ""
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
                    crate::editor::help_box::track_help_strings(ui, &resp, label, hint);
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
    fn modes_have_no_duplicate_variants() {
        for i in 0..MODES.len() {
            for j in (i + 1)..MODES.len() {
                assert_ne!(
                    MODES[i].0, MODES[j].0,
                    "MODES lists {:?} more than once (slots {} and {})",
                    MODES[i].0, i, j
                );
            }
        }
    }
}
