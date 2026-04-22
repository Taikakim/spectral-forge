use nih_plug::prelude::*;
use nih_plug_egui::egui;
use std::path::PathBuf;
use std::sync::Arc;
use crate::params::SpectralForgeParams;
use crate::preset::{Preset, GuiState, preset_dir, sanitize_filename};

#[derive(Clone)]
pub struct PresetMenuState {
    pub available: Vec<(String, PathBuf)>,
    pub selected: Option<String>,
    pub save_name: String,
    pub save_popup_open: bool,
}

impl Default for PresetMenuState {
    fn default() -> Self {
        Self {
            available: Preset::scan_compatible(&preset_dir()),
            selected: None,
            save_name: String::new(),
            save_popup_open: false,
        }
    }
}

impl PresetMenuState {
    pub fn refresh(&mut self) {
        self.available = Preset::scan_compatible(&preset_dir());
    }
}

pub fn preset_menu_ui(
    ui: &mut egui::Ui,
    state: &mut PresetMenuState,
    params: &Arc<SpectralForgeParams>,
    setter: &ParamSetter,
) {
    ui.horizontal(|ui| {
        let current_label = state.selected.as_deref().unwrap_or("-- Preset --");

        egui::ComboBox::from_id_salt("preset_pulldown")
            .selected_text(current_label)
            .width(180.0)
            .show_ui(ui, |ui| {
                for (name, path) in state.available.clone() {
                    let selected = state.selected.as_deref() == Some(name.as_str());
                    if ui.selectable_label(selected, &name).clicked() {
                        if let Ok(p) = Preset::load(&path) {
                            p.apply(params.as_ref(), setter);
                            state.selected = Some(name);
                        }
                    }
                }
            });

        if ui.button("Save").clicked() {
            state.save_popup_open = true;
        }

        if ui.button("Folder").clicked() {
            let dir = preset_dir();
            let _ = std::process::Command::new("xdg-open").arg(&dir).spawn();
        }

        if state.save_popup_open {
            let ctx = ui.ctx().clone();
            egui::Window::new("Save Preset")
                .collapsible(false)
                .resizable(false)
                .show(&ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Name:");
                        ui.text_edit_singleline(&mut state.save_name);
                    });
                    ui.horizontal(|ui| {
                        if ui.button("Save").clicked() && !state.save_name.is_empty() {
                            let safe = sanitize_filename(&state.save_name);
                            let path = preset_dir().join(format!("{}.sfpreset", safe));
                            let gui = GuiState {
                                editing_slot:     *params.editing_slot.lock() as u32,
                                editing_curve:    *params.editing_curve.lock() as u32,
                                slot_module_types: params.slot_module_types.lock()
                                    .iter().map(|t| *t as u8).collect(),
                                stereo_link:      params.stereo_link.value() as u32,
                                fft_size:         params.fft_size.value() as u32,
                            };
                            let preset = Preset::from_params(
                                state.save_name.clone(),
                                params.as_ref(),
                                gui,
                            );
                            if preset.save(&path).is_ok() {
                                state.selected = Some(state.save_name.clone());
                                state.refresh();
                            }
                            state.save_popup_open = false;
                            state.save_name.clear();
                        }
                        if ui.button("Cancel").clicked() {
                            state.save_popup_open = false;
                            state.save_name.clear();
                        }
                    });
                });
        }
    });
}
