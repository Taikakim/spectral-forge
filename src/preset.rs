use nih_plug::prelude::*;
use serde::{Serialize, Deserialize};
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub const PRESET_SCHEMA_VERSION: u32 = 1;

#[derive(Serialize, Deserialize, Clone)]
pub struct Preset {
    pub schema_version: u32,
    pub plugin_version: String,
    pub name: String,
    pub params: HashMap<String, f32>,
    pub gui: GuiState,
}

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct GuiState {
    pub editing_slot: u32,
    pub editing_curve: u32,
    pub slot_module_types: Vec<u8>,
    pub stereo_link: u32,
    pub fft_size: u32,
}

impl Preset {
    /// Snapshot all automatable params from the plugin's current state.
    pub fn from_params(name: String, params: &impl Params, gui: GuiState) -> Self {
        let mut p = HashMap::new();
        for (id, ptr, _group) in params.param_map() {
            // Skip the migration sentinel — it's a persist field, not an automatable param
            if id == "migrated_v1" {
                continue;
            }
            // SAFETY: `params` is alive for the duration of this call and the ParamPtr
            // is valid for as long as `params` is alive.
            let v = unsafe { ptr.unmodulated_normalized_value() };
            p.insert(id, v);
        }
        Self {
            schema_version: PRESET_SCHEMA_VERSION,
            plugin_version: env!("CARGO_PKG_VERSION").to_string(),
            name,
            params: p,
            gui,
        }
    }

    /// Serialize to a pretty-printed JSON file at `path`.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        fs::write(path, json)
    }

    /// Deserialize from a JSON file at `path`.
    pub fn load(path: &Path) -> io::Result<Self> {
        let bytes = fs::read(path)?;
        serde_json::from_slice(&bytes)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    /// Apply this preset by setting all known param IDs via the host's GUI context.
    ///
    /// Unknown IDs are silently ignored (forward-compatible loading).
    /// Must be called from the GUI thread with a valid `ParamSetter`.
    pub fn apply(&self, params: &impl Params, setter: &ParamSetter) {
        let map: HashMap<String, ParamPtr> = params
            .param_map()
            .into_iter()
            .map(|(id, ptr, _group)| (id, ptr))
            .collect();

        for (id, &v) in &self.params {
            if let Some(ptr) = map.get(id) {
                // SAFETY: `params` is alive and the ParamPtr is valid. We use the raw
                // GuiContext methods so the host is notified of the automation gesture.
                unsafe {
                    setter.raw_context.raw_begin_set_parameter(*ptr);
                    setter.raw_context.raw_set_parameter_normalized(*ptr, v);
                    setter.raw_context.raw_end_set_parameter(*ptr);
                }
            }
        }
    }

    /// Scan `dir` for `.sfpreset` files whose schema version matches
    /// [`PRESET_SCHEMA_VERSION`]. Returns `(display_name, path)` pairs sorted
    /// case-insensitively by name.
    pub fn scan_compatible(dir: &Path) -> Vec<(String, PathBuf)> {
        let mut out = Vec::new();
        let Ok(entries) = fs::read_dir(dir) else {
            return out;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("sfpreset") {
                continue;
            }
            let Ok(p) = Self::load(&path) else {
                continue;
            };
            if p.schema_version != PRESET_SCHEMA_VERSION {
                continue;
            }
            out.push((p.name, path));
        }
        out.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
        out
    }
}

/// Return the user's preset directory, creating it if needed.
///
/// Falls back to `./presets` if the platform config directory cannot be determined.
pub fn preset_dir() -> PathBuf {
    if let Some(dirs) = directories::ProjectDirs::from("", "", "Spectral Forge") {
        let p = dirs.config_dir().join("presets");
        let _ = fs::create_dir_all(&p);
        return p;
    }
    PathBuf::from("./presets")
}

/// Replace characters that are illegal in file names across Linux/Windows with `_`.
/// Also trims leading/trailing whitespace.
pub fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c => c,
        })
        .collect::<String>()
        .trim()
        .to_string()
}
