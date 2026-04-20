use nih_plug_egui::egui::{self, Pos2, Ui};
use crate::dsp::modules::{module_spec, ModuleType};
use crate::editor::theme as th;
use crate::params::SpectralForgeParams;

/// Ephemeral state for the module assignment popup.
/// Stored in egui temp data under key `ui.id().with("module_popup")`.
#[derive(Clone)]
pub struct PopupState {
    pub open: bool,
    pub slot: usize,
    pub pos:  Pos2,
}

impl Default for PopupState {
    fn default() -> Self {
        Self { open: false, slot: 0, pos: Pos2::ZERO }
    }
}

/// Count how many T/S Split modules are currently active across all slots.
fn ts_split_count(types: &[ModuleType; 9]) -> usize {
    types.iter().filter(|&&t| t == ModuleType::TransientSustainedSplit).count()
}

const ASSIGNABLE: &[ModuleType] = &[
    ModuleType::Dynamics,
    ModuleType::Freeze,
    ModuleType::PhaseSmear,
    ModuleType::Contrast,
    ModuleType::Gain,
    ModuleType::MidSide,
    ModuleType::TransientSustainedSplit,
    ModuleType::Harmonic,
];

/// Render the popup if open. Call every frame from the main UI closure.
/// Returns true if the popup consumed a click.
pub fn show_popup(
    ui:     &mut Ui,
    params: &SpectralForgeParams,
) -> bool {
    let key = ui.id().with("module_popup");
    let state: PopupState = ui.data(|d| d.get_temp(key).unwrap_or_default());
    if !state.open { return false; }

    let types = *params.slot_module_types.lock();
    let ts_count = ts_split_count(&types);
    let slot = state.slot;

    let mut consumed = false;
    let mut new_state = state.clone();

    let area_response = egui::Area::new(egui::Id::new("module_popup_area"))
        .fixed_pos(state.pos)
        .order(egui::Order::Foreground)
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_min_width(140.0);
                ui.label(
                    egui::RichText::new("Assign module")
                        .color(th::LABEL_DIM).size(9.0)
                );
                ui.separator();

                for &ty in ASSIGNABLE {
                    let spec = module_spec(ty);
                    let is_ts = ty == ModuleType::TransientSustainedSplit;
                    let ts_full = is_ts && ts_count >= 2 && types[slot] != ty;
                    let enabled = !ts_full;

                    ui.add_enabled_ui(enabled, |ui| {
                        ui.horizontal(|ui| {
                            let (rect, _) = ui.allocate_exact_size(
                                egui::vec2(10.0, 10.0), egui::Sense::hover()
                            );
                            ui.painter().rect_filled(rect, 2.0, spec.color_lit);
                            let resp = ui.button(spec.display_name);
                            if resp.clicked() {
                                assign_module(params, slot, ty);
                                new_state.open = false;
                                consumed = true;
                            }
                        });
                    });

                    if ts_full {
                        ui.label(
                            egui::RichText::new("(max 2 active)")
                                .color(th::LABEL_DIM).size(8.0)
                        );
                    }
                }

                ui.separator();
                if ui.button("Remove module").clicked() {
                    assign_module(params, slot, ModuleType::Empty);
                    new_state.open = false;
                    consumed = true;
                }

                ui.separator();
                ui.label(
                    egui::RichText::new("DSP change takes effect\non host restart.")
                        .color(th::LABEL_DIM).size(8.0)
                );
            });
        });

    // Close on click outside: left-click only (right-click opens the popup, any_click would close
    // it on the same frame it was opened)
    if ui.ctx().input(|i| i.pointer.primary_clicked())
        && !area_response.response.contains_pointer()
    {
        new_state.open = false;
    }

    ui.data_mut(|d| d.insert_temp(key, new_state));
    consumed
}

/// Open the popup for a slot at the given screen position.
pub fn open_popup(ui: &mut Ui, slot: usize, pos: Pos2) {
    let key = ui.id().with("module_popup");
    ui.data_mut(|d| d.insert_temp(key, PopupState { open: true, slot, pos }));
}

/// Assign a module type to a slot: update slot_module_types, reset slot_curve_nodes.
fn assign_module(params: &SpectralForgeParams, slot: usize, ty: ModuleType) {
    params.slot_module_types.lock()[slot] = ty;
    // Reset curve nodes for this slot to defaults.
    let spec = module_spec(ty);
    let mut nodes = params.slot_curve_nodes.lock();
    for c in 0..spec.num_curves.min(7) {
        nodes[slot][c] = crate::editor::curve::default_nodes_for_curve(c);
    }
    // Reset tilt/offset for this slot.
    let mut meta = params.slot_curve_meta.lock();
    for c in 0..7 {
        meta[slot][c] = (0.0, 0.0);
    }
    // Reset editing_curve to 0 if it's now out of range.
    let num_c = spec.num_curves;
    let mut ec = params.editing_curve.lock();
    if (*ec as usize) >= num_c && num_c > 0 {
        *ec = 0;
    }
}
