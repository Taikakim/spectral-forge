use serde::{Deserialize, Serialize};
use crate::dsp::modules::{GainMode, ModuleType, RouteMatrix};
use crate::editor::curve::CurveNode;
use crate::params::{FxChannelTarget, NUM_NODES};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginState {
    pub slot_module_types: [ModuleType; 9],
    pub slot_names:        [[u8; 32]; 9],
    pub slot_targets:      [FxChannelTarget; 9],
    pub slot_gain_mode:    [GainMode; 9],
    pub slot_curve_nodes:  [[[CurveNode; NUM_NODES]; 7]; 9],
    pub route:             RouteMatrix,
}

impl Default for PluginState {
    fn default() -> Self {
        preset_default()
    }
}

fn slot_name(s: &str) -> [u8; 32] {
    let mut buf = [0u8; 32];
    let b = s.as_bytes();
    let len = b.len().min(32);
    buf[..len].copy_from_slice(&b[..len]);
    buf
}

fn neutral_nodes() -> [CurveNode; NUM_NODES] {
    [CurveNode::default(); NUM_NODES]
}

fn neutral_curves() -> [[CurveNode; NUM_NODES]; 7] {
    [neutral_nodes(); 7]
}

/// Default preset: Dynamics → Dynamics → Gain → Master
pub fn preset_default() -> PluginState {
    let mut types = [ModuleType::Empty; 9];
    types[0] = ModuleType::Dynamics;
    types[1] = ModuleType::Dynamics;
    types[2] = ModuleType::Gain;
    types[8] = ModuleType::Master;

    let mut names = [[0u8; 32]; 9];
    names[0] = slot_name("Dynamics");
    names[1] = slot_name("Dynamics 2");
    names[2] = slot_name("Gain");
    for i in 3..8 { names[i] = slot_name(&format!("Slot {}", i + 1)); }
    names[8] = slot_name("Master");

    PluginState {
        slot_module_types: types,
        slot_names: names,
        slot_targets:  [FxChannelTarget::All; 9],
        slot_gain_mode: [GainMode::Add; 9],
        slot_curve_nodes: [neutral_curves(); 9],
        route: RouteMatrix::default(),
    }
}

/// Transient sculptor: Dyn → T/S Split → Freeze (sustained) + Gain (transient) → Master
pub fn preset_transient_sculptor() -> PluginState {
    let mut state = preset_default();
    state.slot_module_types[1] = ModuleType::TransientSustainedSplit;
    state.slot_names[1]        = slot_name("T/S Split");
    state.slot_module_types[2] = ModuleType::Freeze;
    state.slot_names[2]        = slot_name("Freeze (sus)");
    state.slot_module_types[3] = ModuleType::Gain;
    state.slot_names[3]        = slot_name("Gain (trans)");

    state.route = RouteMatrix::default();
    state.route.send[0][8] = 1.0;
    state.route.send[1][2] = 1.0;
    state.route.send[2][8] = 1.0;
    state.route.send[3][8] = 1.0;
    state
}

/// Spectral width: Dynamics (Mid) + Dynamics (Side) → M/S → Master
pub fn preset_spectral_width() -> PluginState {
    let mut state = preset_default();
    state.slot_targets[0] = FxChannelTarget::Mid;
    state.slot_targets[1] = FxChannelTarget::Side;
    state.slot_module_types[2] = ModuleType::MidSide;
    state.slot_names[2]        = slot_name("M/S");
    state.route = RouteMatrix::default();
    state.route.send[0][8] = 1.0;
    state.route.send[1][8] = 1.0;
    state.route.send[2][8] = 1.0;
    state
}

/// Phase sculptor: Dyn → PhaseSmear → Contrast → Master
pub fn preset_phase_sculptor() -> PluginState {
    let mut state = preset_default();
    state.slot_module_types[1] = ModuleType::PhaseSmear;
    state.slot_names[1]        = slot_name("Phase Smear");
    state.slot_module_types[2] = ModuleType::Contrast;
    state.slot_names[2]        = slot_name("Contrast");
    state.route = RouteMatrix::default();
    state.route.send[0][1] = 1.0;
    state.route.send[1][2] = 1.0;
    state.route.send[2][8] = 1.0;
    state
}

/// Freeze pad: Freeze (long) → Gain → Master
pub fn preset_freeze_pad() -> PluginState {
    let mut state = preset_default();
    state.slot_module_types[0] = ModuleType::Freeze;
    state.slot_names[0]        = slot_name("Freeze");
    state.slot_module_types[1] = ModuleType::Gain;
    state.slot_names[1]        = slot_name("Gain");
    state.slot_module_types[2] = ModuleType::Empty;
    state.slot_names[2]        = slot_name("Slot 3");
    state.route = RouteMatrix::default();
    state.route.send[0][1] = 1.0;
    state.route.send[1][8] = 1.0;
    state
}
