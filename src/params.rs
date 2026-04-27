use nih_plug::params::persist::{PersistentField, deserialize_field, serialize_field};
use nih_plug::prelude::*;
use nih_plug_egui::EguiState;
use parking_lot::Mutex;
use std::collections::BTreeMap;
use std::sync::Arc;
use crate::editor::curve::CurveNode;
use crate::dsp::modules::{GainMode, ModuleType, RouteMatrix};

// Pulls in `pub struct GeneratedParams { ... }` (1404 FloatParam fields),
// its `Default` impl, and `impl GeneratedParams { fn extend_param_map(...) }`
// from the build.rs output. Defined at the top level — must sit outside any
// struct or fn body because Rust disallows macro-expanded struct fields.
include!(concat!(env!("OUT_DIR"), "/params_gen.rs"));

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
pub enum FftSizeChoice { S512, S1024, S2048, S4096, S8192, S16384 }

pub fn fft_size_from_choice(c: FftSizeChoice) -> usize {
    match c {
        FftSizeChoice::S512   => 512,
        FftSizeChoice::S1024  => 1024,
        FftSizeChoice::S2048  => 2048,
        FftSizeChoice::S4096  => 4096,
        FftSizeChoice::S8192  => 8192,
        FftSizeChoice::S16384 => 16384,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub enum StereoLink { Independent, Linked, MidSide }

/// Sidechain channel routing per SC-aware slot.
/// `Follow` resolves against `StereoLink` and `FxChannelTarget` — see docs/superpowers/specs/2026-04-21-sidechain-refactor-design.md §5.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Enum, serde::Serialize, serde::Deserialize)]
pub enum ScChannel {
    #[default]
    Follow,
    LR,
    L,
    R,
    M,
    S,
}

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

/// Hand-written `Params` impl below; the `#[persist]` and `#[id]` attrs are gone
/// because the derive proc-macro is gone. The persist keys and parameter IDs are
/// wired into `param_map`, `serialize_fields`, and `deserialize_fields` by hand.
pub struct SpectralForgeParams {
    // ── Persisted non-param state ─────────────────────────────────────────
    pub editor_state: Arc<EguiState>,

    pub curve_nodes: Arc<Mutex<[[CurveNode; NUM_NODES]; NUM_CURVE_SETS]>>,

    // Slot order is fixed by curve_idx constants — never reorder them.
    pub active_curve: Arc<Mutex<u8>>,

    pub active_tab: Arc<Mutex<u8>>,   // 0 = Dynamics, 1 = Effects, 2 = Harmonic

    /// Nodes for the per-bin phase-randomisation amount curve (Effects tab, Phase mode).
    pub phase_curve_nodes: Arc<Mutex<[crate::editor::curve::CurveNode; NUM_NODES]>>,

    /// 4 nodes sets for Freeze per-bin curves: Length, Threshold, Portamento, Resistance.
    pub freeze_curve_nodes: Arc<Mutex<[[crate::editor::curve::CurveNode; NUM_NODES]; 4]>>,

    /// Which of the 4 freeze curves is selected for editing (0–3).
    pub freeze_active_curve: Arc<Mutex<u8>>,

    /// Which module slot is currently selected for curve editing (0–7).
    pub editing_slot: Arc<Mutex<u8>>,

    /// Module type for each of the 8 slots.
    pub fx_module_types: Arc<Mutex<[FxModuleType; 8]>>,

    // ── Per-slot modular architecture (Plan D1) ────────────────────────────

    /// Module type assigned to each slot (0..=8). Slot 8 = Master, immutable.
    pub slot_module_types: Arc<Mutex<[ModuleType; 9]>>,

    /// User-editable UTF-8 name per slot, zero-padded to 32 bytes.
    pub slot_names: Arc<Mutex<[[u8; 32]; 9]>>,

    /// Channel routing target per slot (All / Mid / Side).
    pub slot_targets: Arc<Mutex<[FxChannelTarget; 9]>>,

    /// GainMode per slot (only meaningful for Gain module slots).
    pub slot_gain_mode: Arc<Mutex<[GainMode; 9]>>,

    /// Per-slot SC input gain in dB. Range [-90.0, 18.0]; values <= -90.0 treated as "-∞" (SC disabled for slot).
    pub slot_sc_gain_db: Arc<Mutex<[f32; 9]>>,

    /// Per-slot SC channel routing.
    pub slot_sc_channel: Arc<Mutex<[ScChannel; 9]>>,

    /// Per-slot per-curve nodes. [slot 0..=8][curve 0..6][node 0..5].
    pub slot_curve_nodes: Arc<Mutex<[[[CurveNode; NUM_NODES]; 7]; 9]>>,

    /// Which curve within the editing slot is selected (0..num_curves for that type).
    pub editing_curve: Arc<Mutex<u8>>,

    /// Routing matrix. Coexists with legacy fx_route_matrix during D1.
    pub route_matrix: Arc<Mutex<RouteMatrix>>,

    /// User-editable display name for each slot.
    pub fx_module_names: Arc<Mutex<[String; 8]>>,

    /// Channel routing target for each slot.
    pub fx_module_targets: Arc<Mutex<[FxChannelTarget; 8]>>,

    /// 8×8 send matrix. send[src][dst] = linear amplitude [0..1].
    /// src < dst: forward send (current hop). src > dst: feedback (one-hop delayed).
    /// Slot 0 always receives the plugin's main audio input unconditionally — the matrix
    /// controls additional sends *between* slots, not the initial signal path. A fully-zeroed
    /// matrix is therefore valid: slot 0 still processes the input, and its output is the
    /// plugin's main output (last active slot wins).
    pub fx_route_matrix: Arc<Mutex<[[f32; 8]; 8]>>,

    // GUI display state — not audio parameters, not sent to audio thread
    pub graph_db_min: Arc<Mutex<f32>>,      // dBFS floor of spectrum display, default -100
    pub graph_db_max: Arc<Mutex<f32>>,      // dBFS ceiling of spectrum display, default 0
    pub peak_falloff_ms: Arc<Mutex<f32>>,   // spectrum peak hold decay time 0–5000 ms
    pub ui_scale: Arc<Mutex<f32>>,          // GUI scale factor: 1.0 / 1.25 / 1.5 / 1.75 / 2.0

    /// Migration flag: set to `true` after the one-shot copy from legacy persist fields
    /// (slot_curve_nodes, route_matrix) into the generated FloatParam fields.
    /// `pub` so tests can inspect it directly.
    pub migrated_v1: Arc<std::sync::atomic::AtomicBool>,

    // ── Exposed FloatParams / BoolParams / EnumParams (hand-written globals) ──
    pub input_gain: FloatParam,

    pub output_gain: FloatParam,

    pub mix: FloatParam,

    pub attack_ms: FloatParam,

    pub release_ms: FloatParam,

    // Per-curve tilt (dB/oct, pivot 1 kHz) and offset (dB).
    // Applied as gain multipliers: gain *= 10^(tilt * log2(f/1000) / 20) * 10^(offset / 20).
    // Named for host automation readability; displayed in the UI via the active-curve controls.
    pub threshold_tilt: FloatParam,
    pub threshold_offset: FloatParam,
    pub ratio_tilt: FloatParam,
    pub ratio_offset: FloatParam,
    pub attack_tilt: FloatParam,
    pub attack_offset: FloatParam,
    pub release_tilt: FloatParam,
    pub release_offset: FloatParam,
    pub knee_tilt: FloatParam,
    pub knee_offset: FloatParam,
    pub makeup_tilt: FloatParam,
    pub makeup_offset: FloatParam,
    pub mix_tilt: FloatParam,
    pub mix_offset: FloatParam,

    pub sc_attack_ms: FloatParam,

    pub sc_release_ms: FloatParam,

    pub lookahead_ms: FloatParam,

    pub stereo_link: EnumParam<StereoLink>,

    pub fft_size: EnumParam<FftSizeChoice>,

    pub threshold_mode: EnumParam<ThresholdMode>,

    pub sensitivity: FloatParam,

    /// Half-width of the gain-reduction blur kernel in semitones (log-frequency).
    /// 0 = no spatial smoothing; higher = wider suppression band.
    pub suppression_width: FloatParam,

    pub auto_makeup: BoolParam,

    pub delta_monitor: BoolParam,

    pub enable_heavy_modules: BoolParam,

    pub effect_mode: EnumParam<EffectMode>,

    pub phase_rand_amount: FloatParam,

    pub spectral_contrast_db: FloatParam,

    // ── Generated per-slot / per-curve / per-node automation params ──
    // 1134 graph-node fields (9×7×6×3), 126 tilt/offset fields (9×7×2),
    // 63 curvature fields (9×7), and 81 matrix-send fields (9×9). Total 1404.
    // Nested because Rust does not allow macro-expanded field declarations
    // inside a struct. See build.rs.
    pub generated: GeneratedParams,
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
            slot_gain_mode: Arc::new(Mutex::new([GainMode::Add; 9])),
            slot_sc_gain_db: Arc::new(Mutex::new([0.0f32; 9])),
            slot_sc_channel: Arc::new(Mutex::new([ScChannel::Follow; 9])),
            slot_curve_nodes: Arc::new(Mutex::new(
                std::array::from_fn(|_s| {
                    std::array::from_fn(|c| crate::editor::curve::default_nodes_for_curve(c))
                })
            )),
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
            ui_scale:        Arc::new(Mutex::new(1.0)),
            migrated_v1:     Arc::new(std::sync::atomic::AtomicBool::new(false)),

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
            mix_offset:       FloatParam::new(
                "Mix Offset", 0.0,
                FloatRange::Linear { min: -1.0, max: 1.0 },
            ).with_smoother(SmoothingStyle::Linear(50.0))
             .with_step_size(0.01),

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
            fft_size: EnumParam::new("FFT Size", FftSizeChoice::S2048),
            threshold_mode: EnumParam::new("Threshold Mode", ThresholdMode::Absolute),

            sensitivity: FloatParam::new(
                "Sensitivity", 0.5,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            ).with_smoother(SmoothingStyle::Linear(50.0))
             .with_step_size(0.01),

            suppression_width: FloatParam::new(
                "Suppression Width", 0.05,
                FloatRange::Linear { min: 0.0, max: 0.5 },
            ).with_smoother(SmoothingStyle::Linear(50.0))
             .with_step_size(0.01)
             .with_unit(" st"),

            auto_makeup: BoolParam::new("Auto Makeup", false),
            delta_monitor: BoolParam::new("Delta Monitor", false),
            enable_heavy_modules: BoolParam::new("Enable Heavy Modules", true),

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

            // Generated per-slot / per-curve / per-node FloatParam initializers.
            // Produced by build.rs; totals 1404 fields.
            generated: GeneratedParams::default(),
        }
    }
}

// ── Typed accessors for generated params ───────────────────────────────────

impl SpectralForgeParams {
    /// Returns references to `(x, y, q)` FloatParams for the given graph node.
    /// Returns `None` if any index is out of range.
    pub fn graph_node(
        &self,
        slot: usize,
        curve: usize,
        node: usize,
    ) -> Option<(&FloatParam, &FloatParam, &FloatParam)> {
        use crate::param_ids::{NUM_CURVES, NUM_NODES, NUM_SLOTS};
        if slot >= NUM_SLOTS || curve >= NUM_CURVES || node >= NUM_NODES {
            return None;
        }
        Some(graph_node_dispatch!(self, slot, curve, node))
    }

    /// Returns a reference to the tilt FloatParam for the given slot/curve.
    /// Returns `None` if any index is out of range.
    pub fn tilt_param(&self, slot: usize, curve: usize) -> Option<&FloatParam> {
        use crate::param_ids::{NUM_CURVES, NUM_SLOTS};
        if slot >= NUM_SLOTS || curve >= NUM_CURVES {
            return None;
        }
        Some(tilt_dispatch!(self, slot, curve))
    }

    /// Returns a reference to the offset FloatParam for the given slot/curve.
    /// Returns `None` if any index is out of range.
    pub fn offset_param(&self, slot: usize, curve: usize) -> Option<&FloatParam> {
        use crate::param_ids::{NUM_CURVES, NUM_SLOTS};
        if slot >= NUM_SLOTS || curve >= NUM_CURVES {
            return None;
        }
        Some(offset_dispatch!(self, slot, curve))
    }

    /// Returns a reference to the curvature FloatParam for the given slot/curve.
    /// Returns `None` if any index is out of range.
    pub fn curvature_param(&self, slot: usize, curve: usize) -> Option<&FloatParam> {
        use crate::param_ids::{NUM_CURVES, NUM_SLOTS};
        if slot >= NUM_SLOTS || curve >= NUM_CURVES {
            return None;
        }
        Some(curv_dispatch!(self, slot, curve))
    }

    /// Snapshot the three transform params for one slot/curve. GUI-side convenience.
    /// See docs/superpowers/specs/2026-04-23-ui-parameter-spec-design.md §2.
    pub fn curve_transform(&self, slot: usize, curve: usize) -> crate::dsp::modules::CurveTransform {
        crate::dsp::modules::CurveTransform {
            offset:    self.offset_param(slot, curve).map_or(0.0, |p| p.value()),
            tilt:      self.tilt_param(slot, curve).map_or(0.0, |p| p.value()),
            curvature: self.curvature_param(slot, curve).map_or(0.0, |p| p.value()),
        }
    }

    /// Returns a reference to the matrix-cell FloatParam for the given row/col.
    /// Returns `None` if any index is out of range.
    pub fn matrix_cell(&self, row: usize, col: usize) -> Option<&FloatParam> {
        use crate::param_ids::{NUM_MATRIX_ROWS, NUM_SLOTS};
        if row >= NUM_MATRIX_ROWS || col >= NUM_SLOTS {
            return None;
        }
        Some(matrix_dispatch!(self, row, col))
    }

    /// Reset every automatable Param to its nih-plug default via the ParamSetter.
    ///
    /// Iterates `param_map()` using the raw GuiContext API so host automation is properly
    /// notified. This covers all standard FloatParam / BoolParam / EnumParam fields but
    /// does NOT touch `#[persist]` fields (curve node graphs, route matrix, slot assignments).
    /// The audio-side `Pipeline::reset()` is triggered separately via `SharedState::reset_requested`.
    ///
    /// Must be called from the GUI thread (inside an egui frame, where `setter` is valid).
    pub fn reset_to_defaults(&self, setter: &nih_plug::prelude::ParamSetter<'_>) {
        let map = self.param_map();
        for (_id, ptr, _group) in &map {
            // SAFETY: `self` is held in an Arc<SpectralForgeParams> for the plugin's lifetime,
            // so the ParamPtrs returned by param_map() are valid here.
            unsafe {
                let default_normalized = ptr.default_normalized_value();
                setter.raw_context.raw_begin_set_parameter(*ptr);
                setter.raw_context.raw_set_parameter_normalized(*ptr, default_normalized);
                setter.raw_context.raw_end_set_parameter(*ptr);
            }
        }
    }

    /// One-shot migration: copies legacy `#[persist]` data (curve nodes, tilt, route matrix)
    /// into the generated FloatParam smoothers so the DSP and host see the correct values on
    /// first load of an old project.
    ///
    /// This function is idempotent (guarded by `migrated_v1`). Call it from `Plugin::initialize()`.
    ///
    /// # Why `smoother.reset()` instead of `set_plain_value`
    ///
    /// nih-plug does not expose a public API for setting FloatParam atomic values from outside
    /// the plugin wrapper. `smoother.reset(v)` sets the smoother target so that `smoothed.next()`
    /// (which is what the pipeline reads for the matrix) returns the correct value immediately.
    /// The underlying `param.value()` stays at the FloatParam default; correct `value()` is only
    /// achievable at state-load time via `Plugin::filter_state()` (see `lib.rs`).
    pub fn migrate_legacy_if_needed(&self) {
        use std::sync::atomic::Ordering;
        if self.migrated_v1.load(Ordering::Relaxed) { return; }

        // ── Graph nodes: slot_curve_nodes → graph_node smoother ──────────────
        // The DSP reads node data from the triple-buffer (populated from slot_curve_nodes in
        // initialize()), NOT from these FloatParams, so the smoother reset is mainly for
        // host-automation consistency.
        {
            let legacy_nodes = self.slot_curve_nodes.lock();
            for s in 0..crate::param_ids::NUM_SLOTS {
                for c in 0..crate::param_ids::NUM_CURVES {
                    for n in 0..crate::param_ids::NUM_NODES {
                        if let Some((x_p, y_p, q_p)) = self.graph_node(s, c, n) {
                            let node = legacy_nodes[s][c][n];
                            x_p.smoothed.reset(node.x);
                            y_p.smoothed.reset(node.y);
                            q_p.smoothed.reset(node.q);
                        }
                    }
                }
            }
        }

        // ── Matrix: route_matrix.send[src][dst] → matrix_cell(dst, src) ─────
        // The pipeline reads `matrix_cell(r, col).smoothed.next()` and writes
        // `route_matrix_snap.send[col][r]`. So matrix_cell(dst, src) ↔ send[src][dst].
        {
            let legacy_matrix = self.route_matrix.lock();
            for r in 0..crate::param_ids::NUM_MATRIX_ROWS {    // r = dst
                for col in 0..crate::param_ids::NUM_SLOTS {     // col = src
                    if let Some(p) = self.matrix_cell(r, col) {
                        // send[col][r] = send[src][dst]
                        p.smoothed.reset(legacy_matrix.send[col][r]);
                    }
                }
            }
        }

        self.migrated_v1.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod accessor_tests {
    use super::*;

    #[test]
    fn graph_node_accessor_defaults() {
        let p = SpectralForgeParams::default();
        // Node 4 of any curve: default x = 0.8, y = 0.0, q = 0.5
        let (x, y, q) = p.graph_node(3, 2, 4).unwrap();
        assert!(
            (x.value() - 0.8).abs() < 1e-6,
            "x default mismatch: {}",
            x.value()
        );
        assert!(
            y.value().abs() < 1e-6,
            "y default should be 0: {}",
            y.value()
        );
        assert!(
            (q.value() - 0.5).abs() < 1e-6,
            "q default mismatch: {}",
            q.value()
        );
    }

    #[test]
    fn graph_node_out_of_range() {
        let p = SpectralForgeParams::default();
        assert!(p.graph_node(9, 0, 0).is_none());
        assert!(p.graph_node(0, 7, 0).is_none());
        assert!(p.graph_node(0, 0, 6).is_none());
        assert!(p.tilt_param(0, 7).is_none());
        assert!(p.offset_param(9, 0).is_none());
        assert!(p.curvature_param(0, 7).is_none());
        assert!(p.curvature_param(9, 0).is_none());
        assert!(p.matrix_cell(9, 0).is_none());
        assert!(p.matrix_cell(0, 9).is_none());
    }

    #[test]
    fn tilt_offset_matrix_accessors_return_some() {
        let p = SpectralForgeParams::default();
        assert!(p.tilt_param(0, 0).is_some());
        assert!(p.offset_param(8, 6).is_some());
        assert!(p.curvature_param(0, 0).is_some());
        assert!(p.curvature_param(8, 6).is_some());
        assert!(p.matrix_cell(0, 0).is_some());
        assert!(p.matrix_cell(8, 8).is_some());
    }
}

// ── Hand-written Params trait impl ─────────────────────────────────────────
// The derive is gone; we replicate what it would have generated so we can inject
// the `spectral_generated_param_map_entries!` macro and keep full control of the
// serialize/deserialize plumbing for our `#[persist]` fields.
//
// SAFETY: same as the derive — the `ParamPtr`s stored in the returned Vec are
// valid as long as `self` is live. nih-plug holds us in an `Arc`, so that holds.
unsafe impl Params for SpectralForgeParams {
    fn param_map(&self) -> Vec<(String, ParamPtr, String)> {
        // Reserve for the ~35 hand-written globals + 1404 generated entries.
        let mut params: Vec<(String, ParamPtr, String)> = Vec::with_capacity(1450);

        // Hand-written globals (same IDs and order as the previous #[id = "..."] attrs).
        params.push(("input_gain".to_string(),   self.input_gain.as_ptr(),   String::new()));
        params.push(("output_gain".to_string(),  self.output_gain.as_ptr(),  String::new()));
        params.push(("mix".to_string(),          self.mix.as_ptr(),          String::new()));
        params.push(("attack_ms".to_string(),    self.attack_ms.as_ptr(),    String::new()));
        params.push(("release_ms".to_string(),   self.release_ms.as_ptr(),   String::new()));

        params.push(("threshold_tilt".to_string(),   self.threshold_tilt.as_ptr(),   String::new()));
        params.push(("threshold_offset".to_string(), self.threshold_offset.as_ptr(), String::new()));
        params.push(("ratio_tilt".to_string(),       self.ratio_tilt.as_ptr(),       String::new()));
        params.push(("ratio_offset".to_string(),     self.ratio_offset.as_ptr(),     String::new()));
        params.push(("attack_tilt".to_string(),      self.attack_tilt.as_ptr(),      String::new()));
        params.push(("attack_offset".to_string(),    self.attack_offset.as_ptr(),    String::new()));
        params.push(("release_tilt".to_string(),     self.release_tilt.as_ptr(),     String::new()));
        params.push(("release_offset".to_string(),   self.release_offset.as_ptr(),   String::new()));
        params.push(("knee_tilt".to_string(),        self.knee_tilt.as_ptr(),        String::new()));
        params.push(("knee_offset".to_string(),      self.knee_offset.as_ptr(),      String::new()));
        params.push(("makeup_tilt".to_string(),      self.makeup_tilt.as_ptr(),      String::new()));
        params.push(("makeup_offset".to_string(),    self.makeup_offset.as_ptr(),    String::new()));
        params.push(("mix_tilt".to_string(),         self.mix_tilt.as_ptr(),         String::new()));
        params.push(("mix_offset".to_string(),       self.mix_offset.as_ptr(),       String::new()));

        params.push(("sc_attack_ms".to_string(),  self.sc_attack_ms.as_ptr(),  String::new()));
        params.push(("sc_release_ms".to_string(), self.sc_release_ms.as_ptr(), String::new()));
        params.push(("lookahead_ms".to_string(),  self.lookahead_ms.as_ptr(),  String::new()));

        params.push(("stereo_link".to_string(),    self.stereo_link.as_ptr(),    String::new()));
        params.push(("fft_size".to_string(),       self.fft_size.as_ptr(),       String::new()));
        params.push(("threshold_mode".to_string(), self.threshold_mode.as_ptr(), String::new()));

        params.push(("sensitivity".to_string(),       self.sensitivity.as_ptr(),       String::new()));
        params.push(("suppression_width".to_string(), self.suppression_width.as_ptr(), String::new()));

        params.push(("auto_makeup".to_string(),   self.auto_makeup.as_ptr(),   String::new()));
        params.push(("delta_monitor".to_string(), self.delta_monitor.as_ptr(), String::new()));
        params.push(("enable_heavy_modules".to_string(), self.enable_heavy_modules.as_ptr(), String::new()));

        params.push(("effect_mode".to_string(),          self.effect_mode.as_ptr(),          String::new()));
        params.push(("phase_rand_amount".to_string(),    self.phase_rand_amount.as_ptr(),    String::new()));
        params.push(("spectral_contrast_db".to_string(), self.spectral_contrast_db.as_ptr(), String::new()));

        // 1404 generated entries (graph nodes + tilt/offset + curvature + matrix).
        self.generated.extend_param_map(&mut params);

        params
    }

    fn serialize_fields(&self) -> BTreeMap<String, String> {
        let mut serialized = BTreeMap::new();

        // Mirrors the derive-generated pattern: for each `#[persist = "key"]`
        // field, call PersistentField::map + serialize_field and insert into
        // the map. Key strings are identical to the previous attributes so
        // existing saved state continues to deserialize correctly.
        macro_rules! persist_out {
            ($key:literal, $field:ident) => {
                match PersistentField::map(&self.$field, serialize_field) {
                    Ok(data) => { serialized.insert(String::from($key), data); }
                    Err(err) => {
                        nih_plug::nih_debug_assert_failure!(
                            "Could not serialize '{}': {}", $key, err
                        );
                    }
                }
            };
        }

        persist_out!("editor_state",       editor_state);
        persist_out!("curve_nodes",        curve_nodes);
        persist_out!("active_curve",       active_curve);
        persist_out!("active_tab",         active_tab);
        persist_out!("phase_curve_nodes",  phase_curve_nodes);
        persist_out!("freeze_curve_nodes", freeze_curve_nodes);
        persist_out!("freeze_active_curve", freeze_active_curve);
        persist_out!("editing_slot",       editing_slot);
        persist_out!("fx_module_types",    fx_module_types);
        persist_out!("slot_module_types",  slot_module_types);
        persist_out!("slot_names",         slot_names);
        persist_out!("slot_targets",       slot_targets);
        persist_out!("slot_gain_mode",     slot_gain_mode);
        persist_out!("slot_curve_nodes",   slot_curve_nodes);
        persist_out!("editing_curve",      editing_curve);
        persist_out!("route_matrix",       route_matrix);
        persist_out!("fx_module_names",    fx_module_names);
        persist_out!("fx_module_targets",  fx_module_targets);
        persist_out!("fx_route_matrix",    fx_route_matrix);
        persist_out!("graph_db_min",       graph_db_min);
        persist_out!("graph_db_max",       graph_db_max);
        persist_out!("peak_falloff_ms",    peak_falloff_ms);
        persist_out!("ui_scale",           ui_scale);
        persist_out!("migrated_v1",        migrated_v1);

        serialized
    }

    fn deserialize_fields(&self, serialized: &BTreeMap<String, String>) {
        // Mirrors the derive-generated match on field name. Unknown keys are
        // traced (same behaviour as the derive) rather than erroring.
        macro_rules! persist_in {
            ($key:literal, $field:ident, $data:expr) => {
                match deserialize_field(&$data) {
                    Ok(deserialized) => {
                        PersistentField::set(&self.$field, deserialized);
                    }
                    Err(err) => {
                        nih_plug::nih_debug_assert_failure!(
                            "Could not deserialize '{}': {}", $key, err
                        );
                    }
                }
            };
        }

        for (field_name, data) in serialized {
            match field_name.as_str() {
                "editor_state"        => persist_in!("editor_state",       editor_state,       data),
                "curve_nodes"         => persist_in!("curve_nodes",        curve_nodes,        data),
                "active_curve"        => persist_in!("active_curve",       active_curve,       data),
                "active_tab"          => persist_in!("active_tab",         active_tab,         data),
                "phase_curve_nodes"   => persist_in!("phase_curve_nodes",  phase_curve_nodes,  data),
                "freeze_curve_nodes"  => persist_in!("freeze_curve_nodes", freeze_curve_nodes, data),
                "freeze_active_curve" => persist_in!("freeze_active_curve", freeze_active_curve, data),
                "editing_slot"        => persist_in!("editing_slot",       editing_slot,       data),
                "fx_module_types"     => persist_in!("fx_module_types",    fx_module_types,    data),
                "slot_module_types"   => persist_in!("slot_module_types",  slot_module_types,  data),
                "slot_names"          => persist_in!("slot_names",         slot_names,         data),
                "slot_targets"        => persist_in!("slot_targets",       slot_targets,       data),
                "slot_gain_mode"      => persist_in!("slot_gain_mode",     slot_gain_mode,     data),
                "slot_curve_nodes"    => persist_in!("slot_curve_nodes",   slot_curve_nodes,   data),
                "editing_curve"       => persist_in!("editing_curve",      editing_curve,      data),
                "route_matrix"        => persist_in!("route_matrix",       route_matrix,       data),
                "fx_module_names"     => persist_in!("fx_module_names",    fx_module_names,    data),
                "fx_module_targets"   => persist_in!("fx_module_targets", fx_module_targets, data),
                "fx_route_matrix"     => persist_in!("fx_route_matrix",    fx_route_matrix,    data),
                "graph_db_min"        => persist_in!("graph_db_min",       graph_db_min,       data),
                "graph_db_max"        => persist_in!("graph_db_max",       graph_db_max,       data),
                "peak_falloff_ms"     => persist_in!("peak_falloff_ms",    peak_falloff_ms,    data),
                "ui_scale"            => persist_in!("ui_scale",           ui_scale,           data),
                "migrated_v1"         => persist_in!("migrated_v1",        migrated_v1,        data),
                _ => nih_plug::nih_trace!(
                    "Unknown serialized field name: {} (this may not be accurate when using nested param structs)",
                    field_name
                ),
            }
        }
    }
}
