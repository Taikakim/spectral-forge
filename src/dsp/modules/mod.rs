use nih_plug_egui::egui::Color32;
use num_complex::Complex;
use serde::{Deserialize, Serialize};
use std::cell::Cell;
use crate::params::{FxChannelTarget, StereoLink};
use crate::dsp::amp_modes::{AmpMode, AmpCellParams};

// ── Constants ──────────────────────────────────────────────────────────────

pub const MAX_SLOTS: usize = 9;
pub const MAX_SPLIT_VIRTUAL_ROWS: usize = 4;
pub const MAX_MATRIX_ROWS: usize = MAX_SLOTS + MAX_SPLIT_VIRTUAL_ROWS;

// ── ModuleType ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ModuleType {
    #[default]
    Empty,
    Dynamics,
    Freeze,
    PhaseSmear,
    Contrast,
    Gain,
    MidSide,
    TransientSustainedSplit,
    Harmonic,
    Future,
    Punch,
    Rhythm,
    Geometry,
    Modulate,
    Circuit,
    Life,
    Past,
    Kinetics,
    Master,
}

// ── GainMode ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum GainMode {
    #[default]
    Add,
    Subtract,
    Pull,
    Match,
}

// ── VirtualRowKind ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VirtualRowKind { Transient, Sustained }

// ── RouteMatrix ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteMatrix {
    pub send: [[f32; MAX_SLOTS]; MAX_MATRIX_ROWS],
    pub virtual_rows: [Option<(u8, VirtualRowKind)>; MAX_SPLIT_VIRTUAL_ROWS],
    #[serde(default = "default_amp_modes")]
    pub amp_mode: [[AmpMode; MAX_SLOTS]; MAX_MATRIX_ROWS],
    #[serde(default = "default_amp_params")]
    pub amp_params: [[AmpCellParams; MAX_SLOTS]; MAX_MATRIX_ROWS],
}

pub(crate) fn default_amp_modes() -> [[AmpMode; MAX_SLOTS]; MAX_MATRIX_ROWS] {
    [[AmpMode::Linear; MAX_SLOTS]; MAX_MATRIX_ROWS]
}

pub(crate) fn default_amp_params() -> [[AmpCellParams; MAX_SLOTS]; MAX_MATRIX_ROWS] {
    // Source the neutral values from `AmpCellParams::default()` so any change there
    // (new field, retuned default) automatically flows into matrix-cell defaults.
    [[AmpCellParams::default(); MAX_SLOTS]; MAX_MATRIX_ROWS]
}

impl Default for RouteMatrix {
    fn default() -> Self {
        let mut m = Self {
            send: [[0.0f32; MAX_SLOTS]; MAX_MATRIX_ROWS],
            virtual_rows: [None; MAX_SPLIT_VIRTUAL_ROWS],
            amp_mode:   default_amp_modes(),
            amp_params: default_amp_params(),
        };
        // Serial: slot 0 → slot 1 → slot 2 → Master (slot 8).
        m.send[0][1] = 1.0;
        m.send[1][2] = 1.0;
        m.send[2][8] = 1.0;
        m
    }
}

// ── ModuleContext ──────────────────────────────────────────────────────────

#[derive(Copy, Clone)]
pub struct ModuleContext<'block> {
    pub sample_rate:       f32,
    pub fft_size:          usize,
    pub num_bins:          usize,
    pub attack_ms:         f32,
    pub release_ms:        f32,
    pub sensitivity:       f32,
    pub suppression_width: f32,
    pub auto_makeup:       bool,
    pub delta_monitor:     bool,

    // Optional infra fields — populated by later phases. None by default.
    /// Phase 4.1 — per-bin unwrapped phase trajectory exposed as a slice of
    /// `Cell<f32>` so PLPV-aware modules (PhaseSmear, Freeze, MidSide) can both
    /// read AND write through the same field while `ModuleContext` stays
    /// `Copy + Clone`. The Pipeline's re-wrap stage reads the underlying
    /// `Vec<f32>` directly after the FxMatrix pass — modules' `set()` writes
    /// through the Cell mutate the same memory the Pipeline will re-wrap.
    pub unwrapped_phase:      Option<&'block [Cell<f32>]>, // Phase 4.1 / 4.3b
    pub peaks:                Option<&'block [PeakInfo]>, // Phase 4.2
    pub instantaneous_freq:   Option<&'block [f32]>,      // Phase 6.1
    /// Per-bin instantaneous-frequency offset from bin centre, normalised to
    /// fractional bin index. `if_offset[k] = (IF[k] - bin_center_hz(k)) /
    /// bin_center_hz(k)`. Filled by Pipeline once per block from the prior
    /// block's analysis snapshot; one-block latency is acceptable for Past
    /// consumers. v1 default fills with all zeros (centre-aligned); the value
    /// becomes meaningful once Phase 4's per-channel IF cache is wired in.
    pub if_offset: Option<&'block [f32]>,
    pub chromagram:           Option<&'block [f32; 12]>,  // Phase 6.2
    pub harmonic_groups:      Option<&'block [crate::dsp::harmonic_groups::HarmonicGroup]>, // Phase 6.2
    pub midi_notes:           Option<&'block [bool; 128]>, // Phase 6.3
    pub held_pitch_classes:   Option<&'block [bool; 12]>,  // Phase 6.3
    /// Per-channel cepstrum (length = `fft_size`). Populated by Pipeline
    /// inside the hop closure when at least one active slot declares
    /// `needs_cepstrum`. Phase 6.4 lazy infra; `None` otherwise.
    pub cepstrum_buf:         Option<&'block [f32]>,       // Phase 6.4
    pub bpm:                  f32,                         // live from host transport (0.0 if unavailable)
    pub beat_position:        f64,                         // live from host transport (0.0 if unavailable)
    pub sidechain_derivative: Option<&'block [f32]>,      // Phase 5b/Modulate Slew Lag
    pub bin_physics:          Option<&'block crate::dsp::bin_physics::BinPhysics>, // Phase 3.2
    /// Read-only handle to the per-channel STFT history ring. `None` when no
    /// history-consuming module is in the slot chain (the pipeline only
    /// attaches the borrow when at least one slot has `reads_history`).
    /// See `src/dsp/history_buffer.rs`.
    pub history: Option<&'block crate::dsp::history_buffer::HistoryBuffer>,
}

impl<'block> ModuleContext<'block> {
    pub fn new(
        sample_rate: f32, fft_size: usize, num_bins: usize,
        attack_ms: f32, release_ms: f32, sensitivity: f32,
        suppression_width: f32, auto_makeup: bool, delta_monitor: bool,
    ) -> Self {
        Self {
            sample_rate, fft_size, num_bins, attack_ms, release_ms,
            sensitivity, suppression_width, auto_makeup, delta_monitor,
            unwrapped_phase: None,
            peaks: None,
            instantaneous_freq: None,
            chromagram: None,
            harmonic_groups: None,
            midi_notes: None,
            held_pitch_classes: None,
            cepstrum_buf: None,
            bpm: 0.0,
            beat_position: 0.0,
            sidechain_derivative: None,
            bin_physics: None,
            history: None,
            if_offset: None,
        }
    }
}

/// One detected spectral peak. Populated by Phase 4.2 (PLPV peak detection).
/// Bin index `k` plus the magnitude at that bin. Skirt edges (low_k, high_k)
/// describe the peak's region of influence used by Voronoi assignment.
#[derive(Clone, Copy, Debug)]
pub struct PeakInfo {
    pub k:      u32,
    pub mag:    f32,
    pub low_k:  u32,
    pub high_k: u32,
}

// ── ProbeSnapshot (test-only) ──────────────────────────────────────────────

/// Test-only snapshot of the last set of internal parameters a module derived
/// from its curves. Populated in `process()` when `cfg(any(test, feature = "probe"))`
/// is active; zero cost in normal builds. Used by `tests/calibration_roundtrip.rs`
/// to verify every offset_fn's ±1 → [y_min, y_max] claim is respected end-to-end.
#[cfg(any(test, feature = "probe"))]
#[derive(Clone, Copy, Debug, Default)]
pub struct ProbeSnapshot {
    pub threshold_db:  Option<f32>,
    pub ratio:         Option<f32>,
    pub attack_ms:     Option<f32>,
    pub release_ms:    Option<f32>,
    pub knee_db:       Option<f32>,
    pub mix_pct:       Option<f32>,
    pub length_ms:     Option<f32>,
    pub portamento_ms: Option<f32>,
    pub resistance:    Option<f32>,
    pub amount_pct:    Option<f32>,
    pub gain_db:       Option<f32>,
    pub gain_pct:      Option<f32>,
    pub balance_pct:   Option<f32>,
    pub expansion_pct: Option<f32>,
    pub decorrel_pct:  Option<f32>,
    pub transient_pct: Option<f32>,
    pub pan_pct:       Option<f32>,
    pub sensitivity_pct: Option<f32>,
    pub peak_hold_ms:  Option<f32>,

    // BinPhysics probe values — sampled at bin 100 (a non-edge mid-band bin).
    // Populated in Phase 5 by writer modules (Life, Kinetics, etc.); Phase 3
    // ships only the field shapes so calibration tests can compile against them.
    pub bp_mass:            Option<f32>,
    pub bp_temperature:     Option<f32>,
    pub bp_flux:            Option<f32>,
    pub bp_crystallization: Option<f32>,
    pub bp_phase_momentum:  Option<f32>,
    pub bp_slew:            Option<f32>,
    pub bp_bias:            Option<f32>,

    // Past module probes — populated by PastModule::process() under cfg(probe).
    pub past_amount_pct:           Option<f32>,
    pub past_time_seconds:         Option<f32>,
    pub past_active_mode_idx:      Option<u8>,
    pub past_history_frames_used:  Option<u32>,
    pub past_sort_key_idx:         Option<u8>,

    // Kinetics module probes — populated by KineticsModule::process() under cfg(probe).
    pub kinetics_strength:        Option<f32>,
    pub kinetics_mass:            Option<f32>,
    pub kinetics_displacement:    Option<f32>,
    pub kinetics_velocity:        Option<f32>,
    pub kinetics_active_mode_idx: Option<u8>,
    pub kinetics_well_count:      Option<u16>,

    // Modulate retrofit-mode probes — populated by ModulateModule::process()
    // when self.mode is GravityPhaser or PllTear.
    /// Gravity Phaser: count of detected sidechain peaks (0..=32).
    pub mod_gp_node_count:    Option<u16>,
    /// Gravity Phaser: repel-toggle state at the time of probe.
    pub mod_gp_repel:         Option<bool>,
    /// Gravity Phaser: sidechain-positioned mode state at the time of probe.
    pub mod_gp_sc_positioned: Option<bool>,
    /// PLL Tear: percentage of PLL-active bins currently locked, 0..=100.
    pub mod_pll_lock_pct:     Option<f32>,
}

// ── SpectralModule trait ───────────────────────────────────────────────────

pub trait SpectralModule: Send {
    /// Process one FFT hop for one channel.
    ///
    /// `physics`: per-hop BinPhysics state. `None` until Phase 3.5 wires the
    /// real ref via FxMatrix. Writer modules (those that set
    /// `ModuleSpec.writes_bin_physics = true`) get a `Some(&mut ...)` and
    /// are scheduled before any reader modules in the same hop.
    fn process(
        &mut self,
        channel: usize,
        stereo_link: StereoLink,
        target: FxChannelTarget,
        bins: &mut [Complex<f32>],
        sidechain: Option<&[f32]>,
        curves: &[&[f32]],
        suppression_out: &mut [f32],
        physics: Option<&mut crate::dsp::bin_physics::BinPhysics>,
        ctx: &ModuleContext<'_>,
    );

    fn reset(&mut self, sample_rate: f32, fft_size: usize);

    fn tail_length(&self) -> u32 { 0 }

    fn module_type(&self) -> ModuleType;

    fn num_curves(&self) -> usize;

    fn num_outputs(&self) -> Option<usize> { None }

    /// Test-only: return the last set of internal parameters computed during
    /// `process()`. Default implementation returns an empty snapshot.
    /// See `tests/calibration_roundtrip.rs`.
    #[cfg(any(test, feature = "probe"))]
    fn last_probe(&self) -> ProbeSnapshot { ProbeSnapshot::default() }

    /// Returns true if the module's currently-active mode is CPU-heavy.
    /// The "low-end-hardware" preset filter short-circuits process() when
    /// `enable_heavy_modules == false` and this returns true.
    /// Default: false. Modules with multiple modes return based on active mode.
    fn heavy_cpu_for_mode(&self) -> bool { false }

    /// Update the operating mode for Gain modules. Default no-op for all other types.
    fn set_gain_mode(&mut self, _: GainMode) {}

    /// Update the operating mode for Future modules. Default no-op for all other types.
    fn set_future_mode(&mut self, _: FutureMode) {}

    /// Update the operating mode for Punch modules. Default no-op for all other types.
    fn set_punch_mode(&mut self, _: crate::dsp::modules::punch::PunchMode) {}

    /// Update the operating mode for Rhythm modules. Default no-op for all other types.
    fn set_rhythm_mode(&mut self, _: crate::dsp::modules::rhythm::RhythmMode) {}

    /// Update the operating mode for Geometry modules. Default no-op for all other types.
    fn set_geometry_mode(&mut self, _: crate::dsp::modules::geometry::GeometryMode) {}

    /// Update the operating mode for Modulate modules. Default no-op for all other types.
    fn set_modulate_mode(&mut self, _: crate::dsp::modules::modulate::ModulateMode) {}

    /// Update the per-slot Repel toggle for Modulate.GravityPhaser. Default no-op.
    fn set_modulate_repel(&mut self, _repel: bool) {}

    /// Update the per-slot SidechainPositioned toggle for Modulate.GravityPhaser. Default no-op.
    fn set_modulate_sc_positioned(&mut self, _enabled: bool) {}

    /// Update the operating mode for Circuit modules. Default no-op for all other types.
    fn set_circuit_mode(&mut self, _: crate::dsp::modules::circuit::CircuitMode) {}

    /// Update the operating mode for Life modules. Default no-op for all other types.
    fn set_life_mode(&mut self, _: crate::dsp::modules::life::LifeMode) {}

    /// Update Past sub-mode. No-op for non-Past modules.
    fn set_past_mode(&mut self, _: crate::dsp::modules::past::PastMode) {}
    /// Update Past Decay-Sorter sort key. No-op for non-Past modules.
    fn set_past_sort_key(&mut self, _: crate::dsp::modules::past::SortKey) {}

    /// Update the operating mode for Kinetics modules. Default no-op for all other types.
    fn set_kinetics_mode(&mut self, _: crate::dsp::modules::kinetics::KineticsMode) {}
    /// Update the WellSource for Kinetics-GravityWell mode. Default no-op for all other types.
    fn set_kinetics_well_source(&mut self, _: crate::dsp::modules::kinetics::WellSource) {}
    /// Update the MassSource for Kinetics-InertialMass mode. Default no-op for all other types.
    fn set_kinetics_mass_source(&mut self, _: crate::dsp::modules::kinetics::MassSource) {}

    /// Update the arpeggiator step grid for Rhythm modules. Default no-op for all other types.
    fn set_arp_grid(&mut self, _: crate::dsp::modules::rhythm::ArpGrid) {}

    /// Toggle the per-module PLPV peak-locked ducking path. Default no-op
    /// for everything except DynamicsModule. Mirrors the per-mode setter
    /// convention (`set_gain_mode`, `set_future_mode`, etc.) so each module
    /// owns its own toggle and a global setter can fire all of them without
    /// cross-talk.
    fn set_plpv_dynamics_enabled(&mut self, _: bool) {}

    /// Toggle the per-module PLPV unwrapped-phase randomization path on
    /// PhaseSmearModule. Default no-op for all other types.
    fn set_plpv_phase_smear_enabled(&mut self, _: bool) {}

    /// Toggle the per-module PLPV unwrapped-phase advance on Freeze modules.
    /// Default no-op for all other types — only FreezeModule overrides.
    fn set_plpv_freeze_enabled(&mut self, _: bool) {}

    /// Toggle the per-module PLPV peak-aligned mid/side decode on MidSide
    /// modules. Default no-op for all other types — only MidSideModule
    /// overrides. Phase 4.3d.
    fn set_plpv_midside_enabled(&mut self, _: bool) {}

    /// Zero per-module DSP state without allocating. Called from the audio thread
    /// when the user presses Reset. Default is a no-op for stateless modules.
    /// MUST NOT allocate, lock, or do I/O.
    fn clear_state(&mut self) {}

    /// For split modules (T/S Split), returns virtual output buffers.
    /// Index 0 = Transient, Index 1 = Sustained.
    /// Default: None (no virtual outputs).
    fn virtual_outputs(&self) -> Option<[&[Complex<f32>]; 2]> { None }
}

// ── ModuleSpec ─────────────────────────────────────────────────────────────

/// Per-module panel callback. Receives the egui `Ui`, the param store, and a
/// slot index so the panel can read/write that slot's parameters. Lives below
/// the curve editor area in editor_ui.rs. Restricted to non-curve UI (step
/// grids, mode pickers, etc.) — curves stay in their own canvas.
pub type PanelWidgetFn = fn(&mut nih_plug_egui::egui::Ui, &crate::params::SpectralForgeParams, slot: usize);

pub struct ModuleSpec {
    pub display_name:       &'static str,
    pub color_lit:          Color32,
    pub color_dim:          Color32,
    pub num_curves:         usize,
    pub curve_labels:       &'static [&'static str],
    pub supports_sidechain: bool,

    /// True if a freshly assigned slot of this module should auto-route a
    /// sidechain input. Editor honours this on first assignment; user can
    /// override afterwards. False by default for all shipped modules.
    pub wants_sidechain:    bool,

    /// Optional per-module panel callback drawn below the curve editor.
    /// `None` means no panel (most modules). See Task 5 for signature.
    pub panel_widget:       Option<PanelWidgetFn>,

    /// True if this module writes BinPhysics state. The pipeline uses this to
    /// schedule writers before readers within a hop, and to skip the BinPhysics
    /// assembly step entirely when no slot needs it.
    pub writes_bin_physics: bool,

    /// True if this module reads `ModuleContext::instantaneous_freq`. The
    /// pipeline only computes per-bin IF when at least one active slot declares
    /// this. Defaults false for every shipped module; flipped per-module by
    /// Phase 6.1+ consumers.
    pub needs_instantaneous_freq: bool,

    /// Set true when the module reads `ModuleContext::cepstrum_buf`.
    /// Pipeline only computes the extra inverse FFT when at least one active
    /// slot needs it. Phase 6.4 lazy gate.
    pub needs_cepstrum:           bool,

    /// Set true when the module reads `ModuleContext::chromagram`.
    /// Pipeline only computes the chroma vector when at least one active slot
    /// declares this. Phase 6.2 lazy gate.
    pub needs_chromagram:         bool,

    /// Set true when the module reads harmonic-group bin lists from the
    /// harmonic-groups kernel. Pipeline only runs that kernel when at least
    /// one active slot declares this. Phase 6.2 lazy gate.
    pub needs_harmonic_groups:    bool,
}

pub fn module_spec(ty: ModuleType) -> &'static ModuleSpec {
    // 6 curves: THRESHOLD, RATIO, ATTACK, RELEASE, KNEE, MIX
    // Note: MAKEUP (was curve 5 in the legacy system) is now the standalone Gain module.
    static DYN: ModuleSpec = ModuleSpec {
        display_name: "Dynamics",
        color_lit: Color32::from_rgb(0x50, 0xc0, 0xc4),
        color_dim: Color32::from_rgb(0x18, 0x40, 0x42),
        num_curves: 6,
        curve_labels: &["THRESHOLD", "RATIO", "ATTACK", "RELEASE", "KNEE", "MIX"],
        supports_sidechain: true,
        wants_sidechain: false,
        panel_widget: None,
        writes_bin_physics: false,
        needs_instantaneous_freq: false,
        needs_cepstrum: false,
        needs_chromagram: false,
        needs_harmonic_groups: false,
    };
    static FRZ: ModuleSpec = ModuleSpec {
        display_name: "Freeze",
        color_lit: Color32::from_rgb(0x50, 0x80, 0xc8),
        color_dim: Color32::from_rgb(0x18, 0x28, 0x42),
        num_curves: 5,
        curve_labels: &["LENGTH", "THRESHOLD", "PORTAMENTO", "RESISTANCE", "MIX"],
        supports_sidechain: true,
        wants_sidechain: false,
        panel_widget: None,
        writes_bin_physics: false,
        needs_instantaneous_freq: false,
        needs_cepstrum: false,
        needs_chromagram: false,
        needs_harmonic_groups: false,
    };
    static PSM: ModuleSpec = ModuleSpec {
        display_name: "Phase Smear",
        color_lit: Color32::from_rgb(0x90, 0x60, 0xc8),
        color_dim: Color32::from_rgb(0x30, 0x20, 0x42),
        num_curves: 3,
        curve_labels: &["AMOUNT", "PEAK HOLD", "MIX"],
        supports_sidechain: true,
        wants_sidechain: false,
        panel_widget: None,
        writes_bin_physics: false,
        needs_instantaneous_freq: false,
        needs_cepstrum: false,
        needs_chromagram: false,
        needs_harmonic_groups: false,
    };
    static CON: ModuleSpec = ModuleSpec {
        display_name: "Contrast",
        color_lit: Color32::from_rgb(0xb0, 0x60, 0xe0),
        color_dim: Color32::from_rgb(0x38, 0x20, 0x48),
        num_curves: 1,
        curve_labels: &["AMOUNT"],
        supports_sidechain: false,
        wants_sidechain: false,
        panel_widget: None,
        writes_bin_physics: false,
        needs_instantaneous_freq: false,
        needs_cepstrum: false,
        needs_chromagram: false,
        needs_harmonic_groups: false,
    };
    static GN: ModuleSpec = ModuleSpec {
        display_name: "Gain",
        color_lit: Color32::from_rgb(0xc8, 0xa0, 0x50),
        color_dim: Color32::from_rgb(0x42, 0x34, 0x18),
        num_curves: 2,
        curve_labels: &["GAIN", "PEAK HOLD"],
        supports_sidechain: true,
        wants_sidechain: false,
        panel_widget: None,
        writes_bin_physics: false,
        needs_instantaneous_freq: false,
        needs_cepstrum: false,
        needs_chromagram: false,
        needs_harmonic_groups: false,
    };
    static MS: ModuleSpec = ModuleSpec {
        display_name: "Mid/Side",
        color_lit: Color32::from_rgb(0xc0, 0x50, 0xa0),
        color_dim: Color32::from_rgb(0x40, 0x18, 0x34),
        num_curves: 5,
        curve_labels: &["BALANCE", "EXPANSION", "DECORREL", "TRANSIENT", "PAN"],
        supports_sidechain: false,
        wants_sidechain: false,
        panel_widget: None,
        writes_bin_physics: false,
        needs_instantaneous_freq: false,
        needs_cepstrum: false,
        needs_chromagram: false,
        needs_harmonic_groups: false,
    };
    static TS: ModuleSpec = ModuleSpec {
        display_name: "T/S Split",
        color_lit: Color32::from_rgb(0x80, 0xb0, 0x60),
        color_dim: Color32::from_rgb(0x28, 0x38, 0x20),
        num_curves: 1,
        curve_labels: &["SENSITIVITY"],
        supports_sidechain: false,
        wants_sidechain: false,
        panel_widget: None,
        writes_bin_physics: false,
        needs_instantaneous_freq: false,
        needs_cepstrum: false,
        needs_chromagram: false,
        needs_harmonic_groups: false,
    };
    static HARM: ModuleSpec = ModuleSpec {
        display_name: "Harmonic",
        color_lit: Color32::from_rgb(0x50, 0xc8, 0x80),
        color_dim: Color32::from_rgb(0x18, 0x42, 0x28),
        num_curves: 0,
        curve_labels: &[],
        supports_sidechain: false,
        wants_sidechain: false,
        panel_widget: None,
        writes_bin_physics: false,
        needs_instantaneous_freq: false,
        needs_cepstrum: false,
        needs_chromagram: false,
        needs_harmonic_groups: false,
    };
    static FUT: ModuleSpec = ModuleSpec {
        display_name: "Future",
        color_lit: Color32::from_rgb(0x60, 0xa0, 0xc8),
        color_dim: Color32::from_rgb(0x20, 0x34, 0x42),
        num_curves: 5,
        curve_labels: &["AMOUNT", "TIME", "THRESHOLD", "SPREAD", "MIX"],
        supports_sidechain: false,
        wants_sidechain: false,
        panel_widget: None,
        writes_bin_physics: false,
        needs_instantaneous_freq: false,
        needs_cepstrum: false,
        needs_chromagram: false,
        needs_harmonic_groups: false,
    };
    static PUNCH: ModuleSpec = ModuleSpec {
        display_name: "Punch",
        color_lit: Color32::from_rgb(0xe0, 0x70, 0x60),
        color_dim: Color32::from_rgb(0x48, 0x20, 0x20),
        num_curves: 6,
        curve_labels: &["AMOUNT", "WIDTH", "FILL_MODE", "AMP_FILL", "HEAL", "MIX"],
        supports_sidechain: true,
        // First module to opt-in to sidechain auto-routing by default — fresh
        // Punch slot prompts the host to assign an aux input.
        wants_sidechain: true,
        panel_widget: None,
        writes_bin_physics: false,
        needs_instantaneous_freq: false,
        needs_cepstrum: false,
        needs_chromagram: false,
        needs_harmonic_groups: false,
    };
    static RHY: ModuleSpec = ModuleSpec {
        display_name: "Rhythm",
        color_lit: Color32::from_rgb(0xc8, 0xb0, 0x60),
        color_dim: Color32::from_rgb(0x42, 0x38, 0x20),
        num_curves: 5,
        curve_labels: &["AMOUNT", "DIVISION", "ATTACK_FADE", "TARGET_PHASE", "MIX"],
        supports_sidechain: false,
        wants_sidechain:    false,
        panel_widget: Some(crate::editor::rhythm_panel::render),
        writes_bin_physics: false,
        needs_instantaneous_freq: false,
        needs_cepstrum: false,
        needs_chromagram: false,
        needs_harmonic_groups: false,
    };
    static GEO: ModuleSpec = ModuleSpec {
        display_name: "Geometry",
        color_lit: Color32::from_rgb(0x50, 0xb4, 0xa0),
        color_dim: Color32::from_rgb(0x18, 0x3c, 0x34),
        num_curves: 5,
        curve_labels: &["AMOUNT", "MODE_CAP", "DAMP_REL", "THRESH", "MIX"],
        supports_sidechain: false,
        wants_sidechain: false,
        panel_widget: None,
        writes_bin_physics: false,
        needs_instantaneous_freq: false,
        needs_cepstrum: false,
        needs_chromagram: false,
        needs_harmonic_groups: false,
    };
    static MODULATE: ModuleSpec = ModuleSpec {
        display_name: "Modulate",
        color_lit: crate::editor::theme::MODULATE_DOT_COLOR,
        color_dim: Color32::from_rgb(0x36, 0x1e, 0x3c),
        num_curves: 6,
        curve_labels: &["AMOUNT", "REACH", "RATE", "THRESH", "AMPGATE", "MIX"],
        supports_sidechain: true,
        wants_sidechain: true,
        panel_widget: None,
        writes_bin_physics: true,
        needs_instantaneous_freq: false,
        needs_cepstrum: false,
        needs_chromagram: false,
        needs_harmonic_groups: false,
    };
    static CIR: ModuleSpec = ModuleSpec {
        display_name: "Circuit",
        color_lit: crate::editor::theme::CIRCUIT_DOT_COLOR,
        color_dim: Color32::from_rgb(0x40, 0x2c, 0x18),
        num_curves: 5,
        curve_labels: &["AMOUNT", "THRESH", "SPREAD", "RELEASE", "MIX"],
        supports_sidechain: false,
        wants_sidechain: false,
        panel_widget: None,
        // Opt-in ahead of Phases 5c.4–5c.10 (Vactrol/Transformer/Sag/Drift/Slew/BiasFuzz).
        // v1 BBD/Schmitt/Crossover kernels do not yet write any BinPhysics field.
        writes_bin_physics: true,
        needs_instantaneous_freq: false,
        needs_cepstrum: false,
        needs_chromagram: false,
        needs_harmonic_groups: false,
    };
    static LIFE: ModuleSpec = ModuleSpec {
        display_name: "LIFE",
        color_lit: crate::editor::theme::LIFE_DOT_COLOR,
        color_dim: Color32::from_rgb(0x1e, 0x36, 0x1c),
        num_curves: 5,
        curve_labels: &["AMOUNT", "THRESHOLD", "SPEED", "REACH", "MIX"],
        supports_sidechain: false,
        wants_sidechain: false,
        panel_widget: None,
        writes_bin_physics: true,
        needs_instantaneous_freq: false,
        needs_cepstrum: false,
        needs_chromagram: false,
        needs_harmonic_groups: false,
    };
    static PAST: ModuleSpec = ModuleSpec {
        display_name: "PAST",
        color_lit: Color32::from_rgb(0xa0, 0x80, 0xb0),
        color_dim: Color32::from_rgb(0x30, 0x24, 0x38),
        num_curves: 5,
        curve_labels: &["AMOUNT", "TIME", "THRESHOLD", "SPREAD", "MIX"],
        supports_sidechain: false,
        wants_sidechain:    false,
        panel_widget:       None,
        writes_bin_physics: false,
        needs_instantaneous_freq: false,
        needs_cepstrum: false,
        needs_chromagram: false,
        needs_harmonic_groups: false,
    };
    static KIN: ModuleSpec = ModuleSpec {
        display_name: "KINETICS",
        color_lit: Color32::from_rgb(0xc8, 0x80, 0x40),
        color_dim: Color32::from_rgb(0x44, 0x2a, 0x14),
        num_curves: 5,
        curve_labels: &["STRENGTH", "MASS", "REACH", "DAMPING", "MIX"],
        supports_sidechain: true,
        wants_sidechain: false,
        panel_widget: None,
        writes_bin_physics: true,
        needs_instantaneous_freq: false,
        needs_cepstrum: false,
        needs_chromagram: false,
        needs_harmonic_groups: false,
    };
    static MASTER: ModuleSpec = ModuleSpec {
        display_name: "Master",
        color_lit: Color32::from_rgb(0xcc, 0xcc, 0xcc),
        color_dim: Color32::from_rgb(0x44, 0x44, 0x44),
        num_curves: 0,
        curve_labels: &[],
        supports_sidechain: false,
        wants_sidechain: false,
        panel_widget: None,
        writes_bin_physics: false,
        needs_instantaneous_freq: false,
        needs_cepstrum: false,
        needs_chromagram: false,
        needs_harmonic_groups: false,
    };
    static EMPTY: ModuleSpec = ModuleSpec {
        display_name: "Empty",
        color_lit: Color32::from_rgb(0x33, 0x33, 0x33),
        color_dim: Color32::from_rgb(0x22, 0x22, 0x22),
        num_curves: 0,
        curve_labels: &[],
        supports_sidechain: false,
        wants_sidechain: false,
        panel_widget: None,
        writes_bin_physics: false,
        needs_instantaneous_freq: false,
        needs_cepstrum: false,
        needs_chromagram: false,
        needs_harmonic_groups: false,
    };
    match ty {
        ModuleType::Dynamics               => &DYN,
        ModuleType::Freeze                 => &FRZ,
        ModuleType::PhaseSmear             => &PSM,
        ModuleType::Contrast               => &CON,
        ModuleType::Gain                   => &GN,
        ModuleType::MidSide                => &MS,
        ModuleType::TransientSustainedSplit => &TS,
        ModuleType::Harmonic               => &HARM,
        ModuleType::Future                 => &FUT,
        ModuleType::Punch                  => &PUNCH,
        ModuleType::Rhythm                 => &RHY,
        ModuleType::Geometry               => &GEO,
        ModuleType::Modulate               => &MODULATE,
        ModuleType::Circuit                => &CIR,
        ModuleType::Life                   => &LIFE,
        ModuleType::Past                   => &PAST,
        ModuleType::Kinetics               => &KIN,
        ModuleType::Master                 => &MASTER,
        ModuleType::Empty                  => &EMPTY,
    }
}

// ── CurveTransform and apply_curve_transform ──────────────────────────────

/// Per-curve display+DSP transform (offset, tilt, curvature).
/// See docs/superpowers/specs/2026-04-23-ui-parameter-spec-design.md §2.
#[derive(Clone, Copy, Debug, Default)]
pub struct CurveTransform {
    pub offset:    f32,  // [-1, 1] normalized
    pub tilt:      f32,  // [-1, 1] normalized (multiply by TILT_MAX for gain-space)
    pub curvature: f32,  // [0, 1]
}

/// Maximum physical tilt in dB/octave units (normalized tilt × TILT_MAX = physical tilt).
/// Shared between the audio thread (pipeline.rs) and the GUI (editor_ui.rs).
pub const TILT_MAX: f32 = 2.0;

/// Apply spectral tilt (pivoted at 1 kHz), calibrated offset, and curvature (S-curve blend)
/// to a slice of per-bin curve gains, then clamp to [0, ∞).
/// curvature ∈ [0, 1]: 0 = straight tilt, 1 = full smoothstep S-curve pivoted at 1 kHz.
/// `offset_fn` maps (raw_gain, offset_norm) → offset-shifted gain; must be a plain fn pointer
/// (no allocation, no locking) and must satisfy offset_fn(g, 0.0) == g.
/// See docs/superpowers/specs/2026-04-23-ui-parameter-spec-design.md §2.
pub fn apply_curve_transform(
    gains: &mut [f32],
    tilt: f32,
    offset: f32,
    curvature: f32,
    offset_fn: fn(f32, f32) -> f32,
    sample_rate: f32,
    fft_size: usize,
) {
    if gains.is_empty() { return; }
    // curvature only shapes the tilt; if tilt=0, curvature has no effect.
    // offset_fn(g, 0.0) == g for all calibrations, so offset=0 is also a no-op.
    if tilt.abs() < 1e-6 && offset.abs() < 1e-6 { return; }
    const LOG_20: f32 = 1.301_030;
    // Compute log range and pivot from sample_rate so the tilt shape is correct at any Nyquist.
    // See docs/superpowers/specs/2026-04-23-ui-parameter-spec-design.md §2.
    let nyquist   = sample_rate * 0.5;
    let log_range = (nyquist / 20.0).log10(); // 3.0 at 20 kHz Nyquist (40 kHz SR)
    let pivot     = (1000.0_f32 / 20.0).log10() / log_range;
    // Smoothstep value at the pivot — used to zero the sigmoid shape there.
    let s_pivot   = 3.0 * pivot * pivot - 2.0 * pivot * pivot * pivot;
    for (k, g) in gains.iter_mut().enumerate() {
        let freq_hz = (k as f32 * sample_rate / fft_size as f32).max(20.0);
        let norm = ((freq_hz.log10() - LOG_20) / log_range).clamp(0.0, 1.0);
        let linear_shape  = norm - pivot;
        let s             = 3.0 * norm * norm - 2.0 * norm * norm * norm; // smoothstep(norm)
        let sigmoid_shape = s - s_pivot;
        let shape = linear_shape + curvature * (sigmoid_shape - linear_shape);
        let t = tilt * shape;
        let g_off = offset_fn(*g, offset);
        *g = (g_off * (1.0 + t)).max(0.0);
    }
}

// ── shared PEAK HOLD curve mapping ─────────────────────────────────────────

/// Map a PEAK HOLD curve gain (linear; the curve's x-axis is 0..=2) to a
/// hold time in milliseconds. Log-scaled; 0→1 ms, 1→50 ms, 2→500 ms.
/// Shared by `gain::GainModule` (Pull mode) and `phase_smear::PhaseSmearModule`.
#[inline]
pub fn peak_hold_curve_to_ms(curve: f32) -> f32 {
    let c = curve.clamp(0.0, 2.0);
    let log_min = 1.0f32.ln();
    let log_mid = 50.0f32.ln();
    let log_max = 500.0f32.ln();
    let log_t = if c <= 1.0 {
        log_min + (log_mid - log_min) * c
    } else {
        log_mid + (log_max - log_mid) * (c - 1.0)
    };
    log_t.exp()
}

// ── create_module ──────────────────────────────────────────────────────────

pub fn create_module(
    ty: ModuleType,
    sample_rate: f32,
    fft_size: usize,
) -> Box<dyn SpectralModule> {
    let mut m: Box<dyn SpectralModule> = match ty {
        ModuleType::Dynamics               => Box::new(dynamics::DynamicsModule::new()),
        ModuleType::Freeze                 => Box::new(freeze::FreezeModule::new()),
        ModuleType::PhaseSmear             => Box::new(phase_smear::PhaseSmearModule::new()),
        ModuleType::Contrast               => Box::new(contrast::ContrastModule::new()),
        ModuleType::Gain                   => Box::new(gain::GainModule::new()),
        ModuleType::TransientSustainedSplit => Box::new(ts_split::TsSplitModule::new()),
        ModuleType::Harmonic               => Box::new(harmonic::HarmonicModule),
        ModuleType::Future                 => Box::new(future::FutureModule::new()),
        ModuleType::Punch                  => Box::new(punch::PunchModule::new()),
        ModuleType::Rhythm                 => Box::new(rhythm::RhythmModule::new()),
        ModuleType::MidSide                => Box::new(mid_side::MidSideModule::new()),
        ModuleType::Geometry               => Box::new(geometry::GeometryModule::new()),
        ModuleType::Modulate               => Box::new(modulate::ModulateModule::new()),
        ModuleType::Circuit                => Box::new(circuit::CircuitModule::new()),
        ModuleType::Life   => Box::new(life::LifeModule::new()),
        ModuleType::Past   => Box::new(past::PastModule::new(sample_rate, fft_size)),
        ModuleType::Kinetics => Box::new(kinetics::KineticsModule::new()),
        ModuleType::Master => Box::new(master::MasterModule),
        ModuleType::Empty  => Box::new(master::EmptyModule),
    };
    m.reset(sample_rate, fft_size);
    debug_assert_eq!(
        m.num_curves(),
        module_spec(ty).num_curves,
        "module_spec and num_curves() disagree for {:?}", ty
    );
    m
}

// ── Submodules ─────────────────────────────────────────────────────────────

pub mod dynamics;
pub mod freeze;
pub use freeze::FreezeModule;
pub mod phase_smear;
pub use phase_smear::PhaseSmearModule;
pub mod contrast;
pub mod gain;
pub use gain::GainModule;
pub mod ts_split;
pub mod harmonic;
pub mod master;
pub mod mid_side;
pub use mid_side::MidSideModule;
pub mod future;
pub use future::FutureMode;
pub mod punch;
pub mod rhythm;
pub mod geometry;
pub mod modulate;
pub use modulate::ModulateMode;
pub mod circuit;
pub use circuit::CircuitMode;
pub mod life;
pub use life::LifeMode;
pub mod past;
pub mod kinetics;
