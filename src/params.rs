use nih_plug::prelude::*;
use nih_plug_egui::EguiState;
use parking_lot::Mutex;
use std::sync::Arc;
use crate::editor::curve::CurveNode;
use crate::dsp::modules::{GainMode, ModuleType, RouteMatrix};

pub const NUM_CURVE_SETS: usize = 7;
pub const NUM_NODES: usize = 6;

/// Index into the 7 parameter curve sets.
pub mod curve_idx {
    pub const THRESHOLD: usize = 0;
    pub const RATIO:     usize = 1;
    pub const ATTACK:    usize = 2;
    pub const RELEASE:   usize = 3;
    pub const KNEE:      usize = 4;
    pub const MAKEUP:    usize = 5;
    pub const MIX:       usize = 6;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub enum ThresholdMode { Absolute, Relative }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub enum StereoLink { Independent, Linked, MidSide }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub enum EffectMode {
    Bypass,
    Freeze,
    PhaseRand,
    SpectralContrast,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum FxModuleType {
    #[default]
    Empty,
    Dynamics,
    // MidSide,  // Plan D
    // Hpss,     // Plan E
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum FxChannelTarget {
    #[default]
    All,
    Mid,
    Side,
}

impl FxChannelTarget {
    pub fn label(self) -> &'static str {
        match self { Self::All => "All", Self::Mid => "Mid", Self::Side => "Side" }
    }
}

#[derive(Params)]
pub struct SpectralForgeParams {
    #[persist = "editor_state"]
    pub editor_state: Arc<EguiState>,

    #[persist = "curve_nodes"]
    pub curve_nodes: Arc<Mutex<[[CurveNode; NUM_NODES]; NUM_CURVE_SETS]>>,

    // Slot order is fixed by curve_idx constants — never reorder them.
    #[persist = "active_curve"]
    pub active_curve: Arc<Mutex<u8>>,

    #[persist = "active_tab"]
    pub active_tab: Arc<Mutex<u8>>,   // 0 = Dynamics, 1 = Effects, 2 = Harmonic

    /// Nodes for the per-bin phase-randomisation amount curve (Effects tab, Phase mode).
    #[persist = "phase_curve_nodes"]
    pub phase_curve_nodes: Arc<Mutex<[crate::editor::curve::CurveNode; NUM_NODES]>>,

    /// 4 nodes sets for Freeze per-bin curves: Length, Threshold, Portamento, Resistance.
    #[persist = "freeze_curve_nodes"]
    pub freeze_curve_nodes: Arc<Mutex<[[crate::editor::curve::CurveNode; NUM_NODES]; 4]>>,

    /// Which of the 4 freeze curves is selected for editing (0–3).
    #[persist = "freeze_active_curve"]
    pub freeze_active_curve: Arc<Mutex<u8>>,

    /// Which module slot is currently selected for curve editing (0–7).
    #[persist = "editing_slot"]
    pub editing_slot: Arc<Mutex<u8>>,

    /// Module type for each of the 8 slots.
    #[persist = "fx_module_types"]
    pub fx_module_types: Arc<Mutex<[FxModuleType; 8]>>,

    // ── Per-slot modular architecture (Plan D1) ────────────────────────────

    /// Module type assigned to each slot (0..=8). Slot 8 = Master, immutable.
    #[persist = "slot_module_types"]
    pub slot_module_types: Arc<Mutex<[ModuleType; 9]>>,

    /// User-editable UTF-8 name per slot, zero-padded to 32 bytes.
    #[persist = "slot_names"]
    pub slot_names: Arc<Mutex<[[u8; 32]; 9]>>,

    /// Channel routing target per slot (All / Mid / Side).
    #[persist = "slot_targets"]
    pub slot_targets: Arc<Mutex<[FxChannelTarget; 9]>>,

    /// Sidechain input assignment: 0..=3 = aux input index; 255 = self-detect (main input).
    #[persist = "slot_sidechain"]
    pub slot_sidechain: Arc<Mutex<[u8; 9]>>,

    /// GainMode per slot (only meaningful for Gain module slots).
    #[persist = "slot_gain_mode"]
    pub slot_gain_mode: Arc<Mutex<[GainMode; 9]>>,

    /// Per-slot per-curve nodes. [slot 0..=8][curve 0..6][node 0..5].
    #[persist = "slot_curve_nodes"]
    pub slot_curve_nodes: Arc<Mutex<[[[CurveNode; NUM_NODES]; 7]; 9]>>,

    /// Per-slot per-curve tilt and offset. [slot][curve] = (tilt, offset). Default: (0.0, 0.0).
    #[persist = "slot_curve_meta"]
    pub slot_curve_meta: Arc<Mutex<[[(f32, f32); 7]; 9]>>,

    /// Which curve within the editing slot is selected (0..num_curves for that type).
    #[persist = "editing_curve"]
    pub editing_curve: Arc<Mutex<u8>>,

    /// Routing matrix. Coexists with legacy fx_route_matrix during D1.
    #[persist = "route_matrix"]
    pub route_matrix: Arc<Mutex<RouteMatrix>>,

    /// User-editable display name for each slot.
    #[persist = "fx_module_names"]
    pub fx_module_names: Arc<Mutex<[String; 8]>>,

    /// Channel routing target for each slot.
    #[persist = "fx_module_targets"]
    pub fx_module_targets: Arc<Mutex<[FxChannelTarget; 8]>>,

    /// 8×8 send matrix. send[src][dst] = linear amplitude [0..1].
    /// src < dst: forward send (current hop). src > dst: feedback (one-hop delayed).
    /// Slot 0 always receives the plugin's main audio input unconditionally — the matrix
    /// controls additional sends *between* slots, not the initial signal path. A fully-zeroed
    /// matrix is therefore valid: slot 0 still processes the input, and its output is the
    /// plugin's main output (last active slot wins).
    #[persist = "fx_route_matrix"]
    pub fx_route_matrix: Arc<Mutex<[[f32; 8]; 8]>>,

    // GUI display state — not audio parameters, not sent to audio thread
    #[persist = "graph_db_min"]
    pub graph_db_min: Arc<Mutex<f32>>,      // dBFS floor of spectrum display, default -100
    #[persist = "graph_db_max"]
    pub graph_db_max: Arc<Mutex<f32>>,      // dBFS ceiling of spectrum display, default 0
    #[persist = "peak_falloff_ms"]
    pub peak_falloff_ms: Arc<Mutex<f32>>,   // spectrum peak hold decay time 0–5000 ms

    #[id = "input_gain"]
    pub input_gain: FloatParam,

    #[id = "output_gain"]
    pub output_gain: FloatParam,

    #[id = "mix"]
    pub mix: FloatParam,

    #[id = "attack_ms"]
    pub attack_ms: FloatParam,

    #[id = "release_ms"]
    pub release_ms: FloatParam,

    // Per-curve tilt (dB/oct, pivot 1 kHz) and offset (dB).
    // Applied as gain multipliers: gain *= 10^(tilt * log2(f/1000) / 20) * 10^(offset / 20).
    // Named for host automation readability; displayed in the UI via the active-curve controls.
    #[id = "threshold_tilt"]
    pub threshold_tilt: FloatParam,
    #[id = "threshold_offset"]
    pub threshold_offset: FloatParam,
    #[id = "ratio_tilt"]
    pub ratio_tilt: FloatParam,
    #[id = "ratio_offset"]
    pub ratio_offset: FloatParam,
    #[id = "attack_tilt"]
    pub attack_tilt: FloatParam,
    #[id = "attack_offset"]
    pub attack_offset: FloatParam,
    #[id = "release_tilt"]
    pub release_tilt: FloatParam,
    #[id = "release_offset"]
    pub release_offset: FloatParam,
    #[id = "knee_tilt"]
    pub knee_tilt: FloatParam,
    #[id = "knee_offset"]
    pub knee_offset: FloatParam,
    #[id = "makeup_tilt"]
    pub makeup_tilt: FloatParam,
    #[id = "makeup_offset"]
    pub makeup_offset: FloatParam,
    #[id = "mix_tilt"]
    pub mix_tilt: FloatParam,
    #[id = "mix_offset"]
    pub mix_offset: FloatParam,

    #[id = "sc_gain"]
    pub sc_gain: FloatParam,

    #[id = "sc_attack_ms"]
    pub sc_attack_ms: FloatParam,

    #[id = "sc_release_ms"]
    pub sc_release_ms: FloatParam,

    #[id = "lookahead_ms"]
    pub lookahead_ms: FloatParam,

    #[id = "stereo_link"]
    pub stereo_link: EnumParam<StereoLink>,

    #[id = "threshold_mode"]
    pub threshold_mode: EnumParam<ThresholdMode>,

    #[id = "sensitivity"]
    pub sensitivity: FloatParam,

    /// Half-width of the gain-reduction blur kernel in semitones (log-frequency).
    /// 0 = no spatial smoothing; higher = wider suppression band.
    #[id = "suppression_width"]
    pub suppression_width: FloatParam,

    #[id = "auto_makeup"]
    pub auto_makeup: BoolParam,

    #[id = "delta_monitor"]
    pub delta_monitor: BoolParam,

    #[id = "effect_mode"]
    pub effect_mode: EnumParam<EffectMode>,

    #[id = "phase_rand_amount"]
    pub phase_rand_amount: FloatParam,

    #[id = "spectral_contrast_db"]
    pub spectral_contrast_db: FloatParam,
}

impl SpectralForgeParams {
    fn make_tilt(name: &str) -> FloatParam {
        FloatParam::new(name, 0.0, FloatRange::Linear { min: -6.0, max: 6.0 })
            .with_smoother(SmoothingStyle::Linear(50.0))
            .with_step_size(0.01)
            .with_unit(" dB/oct")
    }
    fn make_offset(name: &str) -> FloatParam {
        FloatParam::new(name, 0.0, FloatRange::Linear { min: -18.0, max: 18.0 })
            .with_smoother(SmoothingStyle::Linear(50.0))
            .with_step_size(0.01)
            .with_unit(" dB")
    }
}

impl Default for SpectralForgeParams {
    fn default() -> Self {
        Self {
            editor_state: EguiState::from_size(900, 1010),
            curve_nodes: Arc::new(Mutex::new(
                std::array::from_fn(|i| crate::editor::curve::default_nodes_for_curve(i))
            )),
            active_curve: Arc::new(Mutex::new(0)),
            active_tab: Arc::new(Mutex::new(0)),
            phase_curve_nodes: Arc::new(Mutex::new(
                crate::editor::curve::default_nodes()
            )),
            freeze_curve_nodes: Arc::new(Mutex::new(
                std::array::from_fn(|_| crate::editor::curve::default_nodes())
            )),
            freeze_active_curve: Arc::new(Mutex::new(0)),

            editing_slot: Arc::new(Mutex::new(0u8)),

            slot_module_types: Arc::new(Mutex::new({
                let mut t = [ModuleType::Empty; 9];
                t[0] = ModuleType::Dynamics;
                t[1] = ModuleType::Dynamics;
                t[2] = ModuleType::Gain;
                t[8] = ModuleType::Master;
                t
            })),
            slot_names: Arc::new(Mutex::new({
                let mut names = [[0u8; 32]; 9];
                let labels: &[&str] = &["Dynamics", "Dynamics 2", "Gain", "Slot 4", "Slot 5",
                                         "Slot 6", "Slot 7", "Slot 8", "Master"];
                for (i, label) in labels.iter().enumerate() {
                    let b = label.as_bytes();
                    let len = b.len().min(32);
                    names[i][..len].copy_from_slice(&b[..len]);
                }
                names
            })),
            slot_targets:   Arc::new(Mutex::new([FxChannelTarget::All; 9])),
            slot_sidechain: Arc::new(Mutex::new([255u8; 9])),
            slot_gain_mode: Arc::new(Mutex::new([GainMode::Add; 9])),
            slot_curve_nodes: Arc::new(Mutex::new(
                [[[CurveNode::default(); NUM_NODES]; 7]; 9]
            )),
            slot_curve_meta: Arc::new(Mutex::new([[(0.0f32, 0.0f32); 7]; 9])),
            editing_curve:   Arc::new(Mutex::new(0u8)),
            route_matrix:    Arc::new(Mutex::new(RouteMatrix::default())),

            fx_module_types: Arc::new(Mutex::new({
                let mut arr = [FxModuleType::Empty; 8];
                arr[0] = FxModuleType::Dynamics;
                arr
            })),

            fx_module_names: Arc::new(Mutex::new([
                "Dynamics".to_string(),
                "Slot 1".to_string(),
                "Slot 2".to_string(),
                "Slot 3".to_string(),
                "Slot 4".to_string(),
                "Slot 5".to_string(),
                "Slot 6".to_string(),
                "Slot 7".to_string(),
            ])),

            fx_module_targets: Arc::new(Mutex::new([FxChannelTarget::All; 8])),

            fx_route_matrix: Arc::new(Mutex::new([[0.0f32; 8]; 8])),

            graph_db_min:    Arc::new(Mutex::new(-100.0)),
            graph_db_max:    Arc::new(Mutex::new(0.0)),
            peak_falloff_ms: Arc::new(Mutex::new(300.0)),

            input_gain: FloatParam::new(
                "Input Gain", 0.0,
                FloatRange::Linear { min: -18.0, max: 18.0 },
            ).with_smoother(SmoothingStyle::Linear(50.0))
             .with_step_size(0.01)
             .with_unit(" dB"),

            output_gain: FloatParam::new(
                "Output Gain", 0.0,
                FloatRange::Linear { min: -18.0, max: 18.0 },
            ).with_smoother(SmoothingStyle::Linear(50.0))
             .with_step_size(0.01)
             .with_unit(" dB"),

            mix: FloatParam::new(
                "Mix", 1.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            ).with_smoother(SmoothingStyle::Linear(50.0))
             .with_step_size(0.01),

            attack_ms: FloatParam::new(
                "Attack", 10.0,
                FloatRange::Skewed { min: 0.5, max: 200.0, factor: FloatRange::skew_factor(-2.0) },
            ).with_smoother(SmoothingStyle::Logarithmic(50.0))
             .with_step_size(0.01)
             .with_unit(" ms"),

            release_ms: FloatParam::new(
                "Release", 80.0,
                FloatRange::Skewed { min: 1.0, max: 500.0, factor: FloatRange::skew_factor(-2.0) },
            ).with_smoother(SmoothingStyle::Logarithmic(50.0))
             .with_step_size(0.01)
             .with_unit(" ms"),

            threshold_tilt:   Self::make_tilt("Threshold Tilt"),
            threshold_offset: Self::make_offset("Threshold Offset"),
            ratio_tilt:       Self::make_tilt("Ratio Tilt"),
            ratio_offset:     Self::make_offset("Ratio Offset"),
            attack_tilt:      Self::make_tilt("Attack Tilt"),
            attack_offset:    Self::make_offset("Attack Offset"),
            release_tilt:     Self::make_tilt("Release Tilt"),
            release_offset:   Self::make_offset("Release Offset"),
            knee_tilt:        Self::make_tilt("Knee Tilt"),
            knee_offset:      Self::make_offset("Knee Offset"),
            makeup_tilt:      Self::make_tilt("Makeup Tilt"),
            makeup_offset:    Self::make_offset("Makeup Offset"),
            mix_tilt:         Self::make_tilt("Mix Tilt"),
            mix_offset:       Self::make_offset("Mix Offset"),

            sc_gain: FloatParam::new(
                "SC Gain", 0.0,
                FloatRange::Linear { min: -18.0, max: 18.0 },
            ).with_smoother(SmoothingStyle::Linear(50.0))
             .with_step_size(0.01)
             .with_unit(" dB"),

            sc_attack_ms: FloatParam::new(
                "SC Attack", 5.0,
                FloatRange::Skewed { min: 0.5, max: 100.0, factor: FloatRange::skew_factor(-2.0) },
            ).with_smoother(SmoothingStyle::Logarithmic(50.0))
             .with_step_size(0.01)
             .with_unit(" ms"),

            sc_release_ms: FloatParam::new(
                "SC Release", 50.0,
                FloatRange::Skewed { min: 1.0, max: 300.0, factor: FloatRange::skew_factor(-2.0) },
            ).with_smoother(SmoothingStyle::Logarithmic(50.0))
             .with_step_size(0.01)
             .with_unit(" ms"),

            lookahead_ms: FloatParam::new(
                "Lookahead", 0.0,
                FloatRange::Linear { min: 0.0, max: 10.0 },
            ).with_step_size(0.01)
             .with_unit(" ms"),

            stereo_link: EnumParam::new("Stereo Link", StereoLink::Linked),
            threshold_mode: EnumParam::new("Threshold Mode", ThresholdMode::Absolute),

            sensitivity: FloatParam::new(
                "Sensitivity", 0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            ).with_smoother(SmoothingStyle::Linear(50.0))
             .with_step_size(0.01),

            suppression_width: FloatParam::new(
                "Suppression Width", 0.2,
                FloatRange::Linear { min: 0.0, max: 0.5 },
            ).with_smoother(SmoothingStyle::Linear(50.0))
             .with_step_size(0.01)
             .with_unit(" st"),

            auto_makeup: BoolParam::new("Auto Makeup", false),
            delta_monitor: BoolParam::new("Delta Monitor", false),

            effect_mode: EnumParam::new("Effect Mode", EffectMode::Bypass),

            phase_rand_amount: FloatParam::new(
                "Phase Rand Amount", 0.5,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            ).with_smoother(SmoothingStyle::Linear(50.0))
             .with_step_size(0.01),

            spectral_contrast_db: FloatParam::new(
                "Spectral Contrast", 6.0,
                FloatRange::Linear { min: -12.0, max: 12.0 },
            ).with_smoother(SmoothingStyle::Linear(50.0))
             .with_step_size(0.01)
             .with_unit(" dB"),
        }
    }
}
