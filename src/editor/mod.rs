pub mod curve_config;
pub use curve_config::{CurveDisplayConfig, curve_display_config};
pub mod curve;
pub mod theme;
pub mod spectrum_display;
pub mod fx_matrix_grid;
pub mod module_popup;
pub mod amp_popup;
pub mod life_popup;
pub mod past_popup;
pub mod kinetics_popup;
pub mod harmony_popup;
pub mod circuit_popup;
pub mod preset_menu;
pub use preset_menu::{PresetMenuState, preset_menu_ui};
pub mod mod_ring;
pub use mod_ring::{ModRingState, ModRingToggle};
pub mod rhythm_panel;
pub mod past_panel;
pub mod help_box;

/// Show a tooltip for `response` only after the pointer has been stationary
/// over it for 1 second. Resets the timer whenever the pointer moves.
pub fn delayed_tooltip(ui: &nih_plug_egui::egui::Ui, response: &nih_plug_egui::egui::Response, text: impl Into<String>) {
    if !response.hovered() { return; }

    let text = text.into();
    let id   = response.id.with("tt_start");
    let now  = ui.input(|i| i.time);
    let motion = ui.input(|i| i.pointer.delta());

    let start: f64 = ui.data_mut(|d| {
        if motion.length() > 0.5 {
            d.insert_temp(id, now);
            now
        } else {
            *d.get_temp_mut_or_insert_with(id, || now)
        }
    });

    if now - start >= 1.0 {
        response.clone().on_hover_ui(|ui| { ui.label(&text); });
    }
}
