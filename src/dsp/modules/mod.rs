use nih_plug_egui::egui::Color32;
use num_complex::Complex;
use serde::{Deserialize, Serialize};
use crate::params::{FxChannelTarget, StereoLink};

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
    Master,
}

// ── GainMode ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum GainMode {
    #[default]
    Add,
    Subtract,
    Pull,
}

// ── VirtualRowKind ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VirtualRowKind { Transient, Sustained }

// ── RouteMatrix ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteMatrix {
    pub send: [[f32; MAX_SLOTS]; MAX_MATRIX_ROWS],
    pub virtual_rows: [Option<(u8, VirtualRowKind)>; MAX_SPLIT_VIRTUAL_ROWS],
}

impl Default for RouteMatrix {
    fn default() -> Self {
        let mut m = Self {
            send: [[0.0f32; MAX_SLOTS]; MAX_MATRIX_ROWS],
            virtual_rows: [None; MAX_SPLIT_VIRTUAL_ROWS],
        };
        // Serial: slot 0 → slot 1 → slot 2 → Master (slot 8).
        m.send[0][1] = 1.0;
        m.send[1][2] = 1.0;
        m.send[2][8] = 1.0;
        m
    }
}

// ── ModuleContext ──────────────────────────────────────────────────────────

pub struct ModuleContext {
    pub sample_rate:       f32,
    pub fft_size:          usize,
    pub num_bins:          usize,
    pub attack_ms:         f32,
    pub release_ms:        f32,
    pub sensitivity:       f32,
    pub suppression_width: f32,
    pub auto_makeup:       bool,
    pub delta_monitor:     bool,
}

// ── SpectralModule trait ───────────────────────────────────────────────────

pub trait SpectralModule: Send {
    fn process(
        &mut self,
        channel: usize,
        stereo_link: StereoLink,
        target: FxChannelTarget,
        bins: &mut [Complex<f32>],
        sidechain: Option<&[f32]>,
        curves: &[&[f32]],
        suppression_out: &mut [f32],
        ctx: &ModuleContext,
    );

    fn reset(&mut self, sample_rate: f32, fft_size: usize);

    fn tail_length(&self) -> u32 { 0 }

    fn module_type(&self) -> ModuleType;

    fn num_curves(&self) -> usize;

    fn num_outputs(&self) -> Option<usize> { None }

    /// Update the operating mode for Gain modules. Default no-op for all other types.
    fn set_gain_mode(&mut self, _: GainMode) {}

    /// For split modules (T/S Split), returns virtual output buffers.
    /// Index 0 = Transient, Index 1 = Sustained.
    /// Default: None (no virtual outputs).
    fn virtual_outputs(&self) -> Option<[&[Complex<f32>]; 2]> { None }
}

// ── ModuleSpec ─────────────────────────────────────────────────────────────

pub struct ModuleSpec {
    pub display_name: &'static str,
    pub color_lit:    Color32,
    pub color_dim:    Color32,
    pub num_curves:   usize,
    pub curve_labels: &'static [&'static str],
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
    };
    static FRZ: ModuleSpec = ModuleSpec {
        display_name: "Freeze",
        color_lit: Color32::from_rgb(0x50, 0x80, 0xc8),
        color_dim: Color32::from_rgb(0x18, 0x28, 0x42),
        num_curves: 5,
        curve_labels: &["LENGTH", "THRESHOLD", "PORTAMENTO", "RESISTANCE", "MIX"],
    };
    static PSM: ModuleSpec = ModuleSpec {
        display_name: "Phase Smear",
        color_lit: Color32::from_rgb(0x90, 0x60, 0xc8),
        color_dim: Color32::from_rgb(0x30, 0x20, 0x42),
        num_curves: 3,
        curve_labels: &["AMOUNT", "SC SMOOTH", "MIX"],
    };
    static CON: ModuleSpec = ModuleSpec {
        display_name: "Contrast",
        color_lit: Color32::from_rgb(0xb0, 0x60, 0xe0),
        color_dim: Color32::from_rgb(0x38, 0x20, 0x48),
        num_curves: 2,
        curve_labels: &["AMOUNT", "SC SMOOTH"],
    };
    static GN: ModuleSpec = ModuleSpec {
        display_name: "Gain",
        color_lit: Color32::from_rgb(0xc8, 0xa0, 0x50),
        color_dim: Color32::from_rgb(0x42, 0x34, 0x18),
        num_curves: 2,
        curve_labels: &["GAIN", "SC SMOOTH"],
    };
    static MS: ModuleSpec = ModuleSpec {
        display_name: "Mid/Side",
        color_lit: Color32::from_rgb(0xc0, 0x50, 0xa0),
        color_dim: Color32::from_rgb(0x40, 0x18, 0x34),
        num_curves: 5,
        curve_labels: &["BALANCE", "EXPANSION", "DECORREL", "TRANSIENT", "PAN"],
    };
    static TS: ModuleSpec = ModuleSpec {
        display_name: "T/S Split",
        color_lit: Color32::from_rgb(0x80, 0xb0, 0x60),
        color_dim: Color32::from_rgb(0x28, 0x38, 0x20),
        num_curves: 1,
        curve_labels: &["SENSITIVITY"],
    };
    static HARM: ModuleSpec = ModuleSpec {
        display_name: "Harmonic",
        color_lit: Color32::from_rgb(0x50, 0xc8, 0x80),
        color_dim: Color32::from_rgb(0x18, 0x42, 0x28),
        num_curves: 0,
        curve_labels: &[],
    };
    static MASTER: ModuleSpec = ModuleSpec {
        display_name: "Master",
        color_lit: Color32::from_rgb(0xcc, 0xcc, 0xcc),
        color_dim: Color32::from_rgb(0x44, 0x44, 0x44),
        num_curves: 0,
        curve_labels: &[],
    };
    static EMPTY: ModuleSpec = ModuleSpec {
        display_name: "Empty",
        color_lit: Color32::from_rgb(0x33, 0x33, 0x33),
        color_dim: Color32::from_rgb(0x22, 0x22, 0x22),
        num_curves: 0,
        curve_labels: &[],
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
        ModuleType::Master                 => &MASTER,
        ModuleType::Empty                  => &EMPTY,
    }
}

// ── apply_curve_transform ──────────────────────────────────────────────────

/// Apply spectral tilt (pivoted at 1 kHz in log-frequency space) and additive offset
/// to a slice of pre-bin curve gains, then clamp to `[0.0, ∞)`.
pub fn apply_curve_transform(gains: &mut [f32], tilt: f32, offset: f32, sample_rate: f32, fft_size: usize) {
    if gains.is_empty() { return; }
    if tilt.abs() < 1e-6 && offset.abs() < 1e-6 { return; }
    const LOG_20: f32 = 1.301_030;
    const LOG_RANGE: f32 = 3.0;
    const PIVOT: f32 = 0.566_32; // log10(1000/20) / log10(20000/20)
    for (k, g) in gains.iter_mut().enumerate() {
        let freq_hz = (k as f32 * sample_rate / fft_size as f32).max(20.0);
        let norm = ((freq_hz.log10() - LOG_20) / LOG_RANGE).clamp(0.0, 1.0);
        let t = tilt * (norm - PIVOT);
        *g = ((*g + offset) * (1.0 + t)).max(0.0);
    }
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
        ModuleType::MidSide                => Box::new(mid_side::MidSideModule::new()),
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
pub mod phase_smear;
pub mod contrast;
pub mod gain;
pub mod ts_split;
pub mod harmonic;
pub mod master;
pub mod mid_side;
