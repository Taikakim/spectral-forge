> **Status (2026-04-24): IMPLEMENTED.** `SpectralModule` trait, `RouteMatrix`, `apply_curve_transform`, module stubs landed. Source of truth: the code + [../STATUS.md](../STATUS.md).

# Plan D1 — Module Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Refactor Spectral Forge from a monolithic fixed-stage DSP pipeline into a modular slot-based architecture with identical audio behaviour.

**Architecture:** Introduce a `SpectralModule` trait; each DSP stage lives in its own file; 9 user slots + 1 Master slot routed via `RouteMatrix`; the bridge uses a 9×7 per-slot curve layout; `FxSlotKind` is replaced by `Box<dyn SpectralModule>`; per-curve tilt/offset lives in one shared function; a typed Rust preset API is wired to the CLAP factory preset discovery system.

**Tech Stack:** Rust stable, nih-plug, egui, triple_buffer, num_complex, parking_lot, serde + serde_json.

**Spec:** `docs/superpowers/specs/2026-04-19-modular-architecture-design.md`

---

## File map

| Action | Path |
|--------|------|
| Modify | `src/dsp/mod.rs` |
| Create | `src/dsp/modules/mod.rs` |
| Create | `src/dsp/modules/dynamics.rs` |
| Create | `src/dsp/modules/freeze.rs` |
| Create | `src/dsp/modules/phase_smear.rs` |
| Create | `src/dsp/modules/contrast.rs` |
| Create | `src/dsp/modules/gain.rs` |
| Create | `src/dsp/modules/ts_split.rs` |
| Create | `src/dsp/modules/harmonic.rs` |
| Create | `src/dsp/modules/master.rs` |
| Create | `src/dsp/modules/mid_side.rs` |
| Modify | `src/params.rs` — add new fields; **keep all old fields** (editor_ui.rs stays alive) |
| Modify | `src/bridge.rs` — 9×7 curve channels; 4 sidechain_active |
| Modify | `src/lib.rs` — 4 aux inputs; new SharedState construction |
| Modify | `src/editor_ui.rs` — minimal: change `curve_tx[c]` → `curve_tx[0][c]` |
| Modify | `src/dsp/pipeline.rs` — slot_curve_cache; 4 sidechain paths; thin STFT closure |
| Modify | `src/dsp/fx_matrix.rs` — Box<dyn SpectralModule>; RouteMatrix; Master slot 8 |
| Create | `src/presets.rs` — PluginState; 5 preset builders; CLAP factory API |

**Important constraints:**
- `src/editor_ui.rs` is **not** redesigned in D1 — only the minimum changes to keep it compiling.
- All old `SpectralForgeParams` fields remain until the D2 UI redesign removes them.
- No allocation on the audio thread. All `Vec` state is pre-allocated in `reset()` or `new()`.
- `parking_lot::Mutex` everywhere (existing convention — not `std::sync::Mutex`).
- `num_complex::Complex` (existing convention — not `rustfft::num_complex`).

---

## Task 1: SpectralModule trait and routing infrastructure

**Files:**
- Create: `src/dsp/modules/mod.rs`
- Modify: `src/dsp/mod.rs`

- [ ] **Step 1: Write compile test**

```rust
// tests/module_trait.rs
#[test]
fn module_trait_types_exist() {
    use spectral_forge::dsp::modules::{
        ModuleType, GainMode, VirtualRowKind, RouteMatrix,
        apply_curve_transform, create_module,
    };
    let _ = ModuleType::Dynamics;
    let _ = GainMode::Add;
    let _ = VirtualRowKind::Transient;
    let mut gains = vec![1.0f32; 8];
    apply_curve_transform(&mut gains, 0.5, 0.1);
    assert!(gains.iter().all(|&g| g >= 0.0));
    let m = create_module(ModuleType::Master, 44100.0, 2048);
    assert_eq!(m.module_type(), ModuleType::Master);
    assert_eq!(m.num_outputs(), None);
}
```

Run: `cargo test module_trait_types_exist 2>&1 | head -20`
Expected: FAIL — module `modules` not found.

- [ ] **Step 2: Add `pub mod modules;` to `src/dsp/mod.rs`**

```rust
pub mod guard;
pub mod modules;   // ← add this line
pub mod pipeline;
pub mod engines;
pub mod fx_matrix;
```

- [ ] **Step 3: Write `src/dsp/modules/mod.rs`**

```rust
use egui::Color32;
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

/// Routing matrix for the modular processing graph.
/// Rows 0..MAX_SLOTS are real slots; rows MAX_SLOTS..MAX_MATRIX_ROWS are
/// virtual rows from T/S Split modules (source-only — cannot receive).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteMatrix {
    /// send[source_row][dest_slot] = mix level 0.0–1.0.
    pub send: [[f32; MAX_SLOTS]; MAX_MATRIX_ROWS],
    /// Active virtual rows: (real_slot_index, kind).
    pub virtual_rows: [Option<(u8, VirtualRowKind)>; MAX_SPLIT_VIRTUAL_ROWS],
}

impl Default for RouteMatrix {
    fn default() -> Self {
        let mut m = Self {
            send: [[0.0f32; MAX_SLOTS]; MAX_MATRIX_ROWS],
            virtual_rows: [None; MAX_SPLIT_VIRTUAL_ROWS],
        };
        // Default preset: slots 0, 1, 2 → Master (slot 8)
        m.send[0][8] = 1.0;
        m.send[1][8] = 1.0;
        m.send[2][8] = 1.0;
        m
    }
}

// ── ModuleContext ──────────────────────────────────────────────────────────

/// Read-only shared context passed to every module's `process()` call.
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
    /// Process one STFT hop in-place.
    ///
    /// `curves` has exactly `num_curves()` elements; each element is a slice
    /// of `ctx.num_bins` linear gain multipliers (tilt/offset already applied).
    ///
    /// `suppression_out` must be fully written with non-negative dB values.
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

    /// Extra tail beyond one FFT window. Default: 0.
    fn tail_length(&self) -> u32 { 0 }

    fn module_type(&self) -> ModuleType;

    /// Number of curves this module uses (0–7).
    fn num_curves(&self) -> usize;

    /// Returns `Some(2)` for T/S Split; `None` for all other modules.
    fn num_outputs(&self) -> Option<usize> { None }
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
        num_curves: 4,
        curve_labels: &["LENGTH", "THRESHOLD", "PORTAMENTO", "RESISTANCE"],
    };
    static PSM: ModuleSpec = ModuleSpec {
        display_name: "Phase Smear",
        color_lit: Color32::from_rgb(0x90, 0x60, 0xc8),
        color_dim: Color32::from_rgb(0x30, 0x20, 0x42),
        num_curves: 2,
        curve_labels: &["AMOUNT", "SC SMOOTH"],
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

/// Apply tilt and offset to a pre-computed gain curve, in-place.
///
/// `tilt` (approx −1..+1): linear ramp from −tilt/2 at bin 0 to +tilt/2 at
/// the last bin, multiplied onto each gain value.
///
/// `offset` (approx −1..+1): additive shift applied in linear gain space before tilt.
///
/// Output is clamped to ≥ 0. Called once per slot×curve by the pipeline.
pub fn apply_curve_transform(gains: &mut [f32], tilt: f32, offset: f32) {
    let n = gains.len();
    if n == 0 { return; }
    let n_f = n as f32;
    for (k, g) in gains.iter_mut().enumerate() {
        let t = tilt * (k as f32 / n_f - 0.5);
        *g = ((*g + offset) * (1.0 + t)).max(0.0);
    }
}

// ── create_module ──────────────────────────────────────────────────────────

/// Factory: only place that names concrete module types outside their own files.
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
        ModuleType::Master | ModuleType::Empty => Box::new(master::MasterModule),
    };
    m.reset(sample_rate, fft_size);
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
```

- [ ] **Step 4: Run test**

```bash
cargo test module_trait_types_exist 2>&1 | head -30
```
Expected: FAIL — cannot find module files (dynamics.rs etc. don't exist yet). This is expected and guides Task 2.

- [ ] **Step 5: Commit**

```bash
git add src/dsp/mod.rs src/dsp/modules/mod.rs tests/module_trait.rs
git commit -m "feat: add SpectralModule trait, RouteMatrix, apply_curve_transform"
```

---

## Task 2: Module implementations

**Files:** Create all 9 module files in `src/dsp/modules/`

Each module must:
- not allocate on the audio thread (all `Vec` state pre-allocated in `reset()`)
- fully write `suppression_out` (fill with 0.0 if no suppression)
- implement all trait methods

### 2a: `src/dsp/modules/dynamics.rs`

Wraps `SpectralCompressorEngine`. Migrates physical-unit mapping from `pipeline.rs` lines 260–309. The `curves` slice has 6 elements (threshold=0, ratio=1, attack=2, release=3, knee=4, mix=5). Makeup is always zero — that is the Gain module's job.

```rust
use num_complex::Complex;
use crate::dsp::engines::{BinParams, SpectralEngine, create_engine, EngineSelection};
use crate::params::{FxChannelTarget, StereoLink};
use super::{ModuleContext, ModuleType, SpectralModule};

pub struct DynamicsModule {
    engine:   Box<dyn SpectralEngine>,
    engine_r: Box<dyn SpectralEngine>,
    // Pre-allocated BinParams backing buffers
    bp_threshold: Vec<f32>,
    bp_ratio:     Vec<f32>,
    bp_attack:    Vec<f32>,
    bp_release:   Vec<f32>,
    bp_knee:      Vec<f32>,
    bp_makeup:    Vec<f32>,  // always 0.0 — Gain module handles makeup
    bp_mix:       Vec<f32>,
    num_bins: usize,
    sample_rate: f32,
}

impl DynamicsModule {
    pub fn new() -> Self {
        Self {
            engine:   create_engine(EngineSelection::SpectralCompressor),
            engine_r: create_engine(EngineSelection::SpectralCompressor),
            bp_threshold: Vec::new(),
            bp_ratio:     Vec::new(),
            bp_attack:    Vec::new(),
            bp_release:   Vec::new(),
            bp_knee:      Vec::new(),
            bp_makeup:    Vec::new(),
            bp_mix:       Vec::new(),
            num_bins: 0,
            sample_rate: 44100.0,
        }
    }
}

impl SpectralModule for DynamicsModule {
    fn reset(&mut self, sample_rate: f32, fft_size: usize) {
        self.sample_rate = sample_rate;
        self.num_bins = fft_size / 2 + 1;
        self.engine.reset(sample_rate, fft_size);
        self.engine_r.reset(sample_rate, fft_size);
        let n = self.num_bins;
        self.bp_threshold = vec![-20.0f32; n];
        self.bp_ratio     = vec![1.0f32;   n];
        self.bp_attack    = vec![10.0f32;  n];
        self.bp_release   = vec![100.0f32; n];
        self.bp_knee      = vec![6.0f32;   n];
        self.bp_makeup    = vec![0.0f32;   n];
        self.bp_mix       = vec![1.0f32;   n];
    }

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
    ) {
        // Channel gating (ported from fx_matrix.rs FxSlotKind::process_dynamics)
        let skip = match (target, stereo_link, channel) {
            (FxChannelTarget::Mid,  StereoLink::MidSide,     1) => true,
            (FxChannelTarget::Side, StereoLink::MidSide,     0) => true,
            (FxChannelTarget::Mid,  StereoLink::Linked,      _) => true,
            (FxChannelTarget::Mid,  StereoLink::Independent, _) => true,
            (FxChannelTarget::Side, StereoLink::Linked,      _) => true,
            (FxChannelTarget::Side, StereoLink::Independent, _) => true,
            _ => false,
        };
        if skip {
            suppression_out.fill(0.0);
            return;
        }

        let n = self.num_bins;
        let sr = self.sample_rate;
        let atk  = ctx.attack_ms;
        let rel  = ctx.release_ms;

        // Map curve linear gains → physical units (ported from pipeline.rs lines 260–309)
        // curves[0]=threshold, [1]=ratio, [2]=attack, [3]=release, [4]=knee, [5]=mix
        for k in 0..n {
            let t = curves.get(0).and_then(|c| c.get(k)).copied().unwrap_or(1.0);
            let t_db = if t > 1e-10 { 20.0 * t.log10() } else { -120.0 };
            self.bp_threshold[k] = (-20.0 + t_db * (60.0 / 18.0)).clamp(-80.0, 0.0);

            let r = curves.get(1).and_then(|c| c.get(k)).copied().unwrap_or(1.0);
            self.bp_ratio[k] = r.clamp(1.0, 20.0);

            let af = curves.get(2).and_then(|c| c.get(k)).copied().unwrap_or(1.0).max(0.01);
            self.bp_attack[k] = (atk * af).clamp(0.1, 500.0);

            let rf = curves.get(3).and_then(|c| c.get(k)).copied().unwrap_or(1.0).max(0.01);
            self.bp_release[k] = (rel * rf).clamp(1.0, 2000.0);

            let kn = curves.get(4).and_then(|c| c.get(k)).copied().unwrap_or(1.0);
            self.bp_knee[k] = (kn * 6.0).clamp(0.0, 48.0);

            let mx = curves.get(5).and_then(|c| c.get(k)).copied().unwrap_or(1.0);
            self.bp_mix[k] = mx.clamp(0.0, 1.0);
        }

        let params = BinParams {
            threshold_db: &self.bp_threshold,
            ratio:        &self.bp_ratio,
            attack_ms:    &self.bp_attack,
            release_ms:   &self.bp_release,
            knee_db:      &self.bp_knee,
            makeup_db:    &self.bp_makeup,
            mix:          &self.bp_mix,
            sensitivity:  ctx.sensitivity,
            auto_makeup:  ctx.auto_makeup,
            smoothing_semitones: ctx.suppression_width,
        };

        let eng: &mut Box<dyn SpectralEngine> = match stereo_link {
            StereoLink::Independent if channel == 1 => &mut self.engine_r,
            _ => &mut self.engine,
        };
        eng.process_bins(bins, sidechain, &params, sr, suppression_out);
    }

    fn module_type(&self) -> ModuleType { ModuleType::Dynamics }
    fn num_curves(&self) -> usize { 6 }
}
```

### 2b: `src/dsp/modules/freeze.rs`

DSP migrated verbatim from `pipeline.rs` lines 467–523. The four curves map as:
- `curves[0]` = length (500ms full scale)
- `curves[1]` = threshold (same log mapping as dynamics threshold)
- `curves[2]` = portamento (100ms full scale)
- `curves[3]` = resistance (0–5 full scale)

```rust
use num_complex::Complex;
use crate::params::{FxChannelTarget, StereoLink};
use super::{ModuleContext, ModuleType, SpectralModule};

pub struct FreezeModule {
    frozen_bins:    Vec<Complex<f32>>,
    freeze_target:  Vec<Complex<f32>>,
    freeze_port_t:  Vec<f32>,
    freeze_hold_hops: Vec<u32>,
    freeze_accum:   Vec<f32>,
    freeze_captured: bool,
    fft_size: usize,
    sample_rate: f32,
}

impl FreezeModule {
    pub fn new() -> Self {
        Self {
            frozen_bins:    Vec::new(),
            freeze_target:  Vec::new(),
            freeze_port_t:  Vec::new(),
            freeze_hold_hops: Vec::new(),
            freeze_accum:   Vec::new(),
            freeze_captured: false,
            fft_size: 2048,
            sample_rate: 44100.0,
        }
    }
}

impl SpectralModule for FreezeModule {
    fn reset(&mut self, sample_rate: f32, fft_size: usize) {
        self.sample_rate = sample_rate;
        self.fft_size = fft_size;
        let n = fft_size / 2 + 1;
        self.frozen_bins    = vec![Complex::new(0.0, 0.0); n];
        self.freeze_target  = vec![Complex::new(0.0, 0.0); n];
        self.freeze_port_t  = vec![1.0f32; n];
        self.freeze_hold_hops = vec![0u32; n];
        self.freeze_accum   = vec![0.0f32; n];
        self.freeze_captured = false;
    }

    fn process(
        &mut self,
        _channel: usize,
        _stereo_link: StereoLink,
        _target: FxChannelTarget,
        bins: &mut [Complex<f32>],
        _sidechain: Option<&[f32]>,
        curves: &[&[f32]],
        suppression_out: &mut [f32],
        _ctx: &ModuleContext,
    ) {
        use crate::dsp::pipeline::{FFT_SIZE, OVERLAP};
        let hop_ms = FFT_SIZE as f32 / (OVERLAP as f32 * self.sample_rate) * 1000.0;

        if !self.freeze_captured {
            self.frozen_bins.copy_from_slice(bins);
            self.freeze_target.copy_from_slice(bins);
            self.freeze_port_t.fill(1.0);
            self.freeze_hold_hops.fill(0);
            self.freeze_accum.fill(0.0);
            self.freeze_captured = true;
        }

        let n = bins.len();
        for k in 0..n {
            let length_ms  = curves.get(0).and_then(|c| c.get(k))
                                   .copied().unwrap_or(1.0) * 500.0;
            let length_ms  = length_ms.clamp(0.0, 2000.0);
            let length_hops = (length_ms / hop_ms).ceil() as u32;

            let thr_gain = curves.get(1).and_then(|c| c.get(k)).copied().unwrap_or(1.0);
            let thr_db   = if thr_gain > 1e-10 { 20.0 * thr_gain.log10() } else { -120.0 };
            let threshold_db  = (-20.0 + thr_db * (60.0 / 18.0)).clamp(-80.0, 0.0);
            let threshold_lin = 10.0f32.powf(threshold_db / 20.0);

            let port_gain = curves.get(2).and_then(|c| c.get(k)).copied().unwrap_or(1.0);
            let port_ms   = (port_gain * 100.0).clamp(0.0, 1000.0);
            let port_hops = (port_ms / hop_ms).max(0.5);

            let resistance = curves.get(3).and_then(|c| c.get(k)).copied().unwrap_or(1.0)
                             * 1.0;
            let resistance = resistance.clamp(0.0, 5.0);

            if self.freeze_port_t[k] < 1.0 {
                self.freeze_port_t[k] = (self.freeze_port_t[k] + 1.0 / port_hops).min(1.0);
                let t = self.freeze_port_t[k];
                self.frozen_bins[k] = Complex::new(
                    self.frozen_bins[k].re * (1.0 - t) + self.freeze_target[k].re * t,
                    self.frozen_bins[k].im * (1.0 - t) + self.freeze_target[k].im * t,
                );
            } else {
                self.freeze_hold_hops[k] += 1;
                let mag = bins[k].norm();
                if mag > threshold_lin {
                    self.freeze_accum[k] += mag - threshold_lin;
                }
                if self.freeze_hold_hops[k] >= length_hops && self.freeze_accum[k] >= resistance {
                    self.freeze_target[k]    = bins[k];
                    self.freeze_port_t[k]    = 0.0;
                    self.freeze_hold_hops[k] = 0;
                    self.freeze_accum[k]     = 0.0;
                }
            }

            bins[k] = self.frozen_bins[k];
        }
        suppression_out.fill(0.0);
    }

    fn tail_length(&self) -> u32 { self.fft_size as u32 }
    fn module_type(&self) -> ModuleType { ModuleType::Freeze }
    fn num_curves(&self) -> usize { 4 }
}
```

### 2c: `src/dsp/modules/phase_smear.rs`

DSP migrated from `pipeline.rs` lines 525–541. The `rng_state` must never be zero (xorshift64 invariant — init to any non-zero value).

- `curves[0]` = amount per bin (0.0–2.0 full-scale; values > 1.0 = more than π randomisation)
- `curves[1]` = sc_smooth (reserved; unused in D1 — placeholder for sidechain smoothing)

```rust
use num_complex::Complex;
use crate::params::{FxChannelTarget, StereoLink};
use super::{ModuleContext, ModuleType, SpectralModule};

pub struct PhaseSmearModule {
    rng_state: u64,
}

impl PhaseSmearModule {
    pub fn new() -> Self { Self { rng_state: 0x123456789abcdef0 } }

    #[inline(always)]
    fn xorshift(&mut self) -> u64 {
        self.rng_state ^= self.rng_state << 13;
        self.rng_state ^= self.rng_state >> 7;
        self.rng_state ^= self.rng_state << 17;
        self.rng_state
    }
}

impl SpectralModule for PhaseSmearModule {
    fn reset(&mut self, _sample_rate: f32, _fft_size: usize) {
        // Keep rng_state to avoid clicks when re-activating.
    }

    fn process(
        &mut self,
        _channel: usize,
        _stereo_link: StereoLink,
        _target: FxChannelTarget,
        bins: &mut [Complex<f32>],
        _sidechain: Option<&[f32]>,
        curves: &[&[f32]],
        suppression_out: &mut [f32],
        _ctx: &ModuleContext,
    ) {
        let last = bins.len() - 1;
        for k in 0..bins.len() {
            // Always advance PRNG to keep sequence independent of skipping bins.
            let rand = self.xorshift();
            // DC and Nyquist must stay real.
            if k == 0 || k == last { continue; }
            let per_bin = curves.get(0).and_then(|c| c.get(k))
                                 .copied().unwrap_or(1.0).clamp(0.0, 2.0);
            let scale = per_bin * std::f32::consts::PI;
            let rand_phase = (rand as f32 / u64::MAX as f32 * 2.0 - 1.0) * scale;
            let (mag, phase) = (bins[k].norm(), bins[k].arg());
            bins[k] = Complex::from_polar(mag, phase + rand_phase);
        }
        suppression_out.fill(0.0);
    }

    fn module_type(&self) -> ModuleType { ModuleType::PhaseSmear }
    fn num_curves(&self) -> usize { 2 }
}
```

### 2d: `src/dsp/modules/contrast.rs`

Migrated from `EffectMode::SpectralContrast` — wraps `SpectralContrastEngine`.

- `curves[0]` = amount (modulates the contrast engine's ratio)
- `curves[1]` = sc_smooth (reserved in D1)

```rust
use num_complex::Complex;
use crate::dsp::engines::{BinParams, SpectralEngine, create_engine, EngineSelection};
use crate::params::{FxChannelTarget, StereoLink};
use super::{ModuleContext, ModuleType, SpectralModule};

pub struct ContrastModule {
    engine: Box<dyn SpectralEngine>,
    // BinParams backing buffers
    bp_threshold: Vec<f32>,
    bp_ratio:     Vec<f32>,
    bp_attack:    Vec<f32>,
    bp_release:   Vec<f32>,
    bp_knee:      Vec<f32>,
    bp_makeup:    Vec<f32>,
    bp_mix:       Vec<f32>,
    num_bins: usize,
    sample_rate: f32,
}

impl ContrastModule {
    pub fn new() -> Self {
        Self {
            engine:       create_engine(EngineSelection::SpectralContrast),
            bp_threshold: Vec::new(),
            bp_ratio:     Vec::new(),
            bp_attack:    Vec::new(),
            bp_release:   Vec::new(),
            bp_knee:      Vec::new(),
            bp_makeup:    Vec::new(),
            bp_mix:       Vec::new(),
            num_bins: 0,
            sample_rate: 44100.0,
        }
    }
}

impl SpectralModule for ContrastModule {
    fn reset(&mut self, sample_rate: f32, fft_size: usize) {
        self.sample_rate = sample_rate;
        self.num_bins = fft_size / 2 + 1;
        self.engine.reset(sample_rate, fft_size);
        let n = self.num_bins;
        self.bp_threshold = vec![-20.0f32; n];
        self.bp_ratio     = vec![2.0f32;   n];   // contrast default: 2× exaggeration
        self.bp_attack    = vec![10.0f32;  n];
        self.bp_release   = vec![100.0f32; n];
        self.bp_knee      = vec![6.0f32;   n];
        self.bp_makeup    = vec![0.0f32;   n];
        self.bp_mix       = vec![1.0f32;   n];
    }

    fn process(
        &mut self,
        _channel: usize,
        _stereo_link: StereoLink,
        _target: FxChannelTarget,
        bins: &mut [Complex<f32>],
        sidechain: Option<&[f32]>,
        curves: &[&[f32]],
        suppression_out: &mut [f32],
        ctx: &ModuleContext,
    ) {
        let n = self.num_bins;
        for k in 0..n {
            // Amount curve modulates the ratio (1.0 → 2× contrast base)
            let amount = curves.get(0).and_then(|c| c.get(k)).copied().unwrap_or(1.0);
            let base = (1.0 + amount).max(0.0);
            self.bp_ratio[k]     = base.clamp(0.0, 20.0);
            self.bp_threshold[k] = -20.0;
            self.bp_attack[k]    = ctx.attack_ms.clamp(0.1, 500.0);
            self.bp_release[k]   = ctx.release_ms.clamp(1.0, 2000.0);
            self.bp_knee[k]      = 6.0;
            self.bp_mix[k]       = 1.0;
        }
        let params = BinParams {
            threshold_db: &self.bp_threshold,
            ratio:        &self.bp_ratio,
            attack_ms:    &self.bp_attack,
            release_ms:   &self.bp_release,
            knee_db:      &self.bp_knee,
            makeup_db:    &self.bp_makeup,
            mix:          &self.bp_mix,
            sensitivity:  ctx.sensitivity,
            auto_makeup:  false,
            smoothing_semitones: ctx.suppression_width,
        };
        self.engine.process_bins(bins, sidechain, &params, self.sample_rate, suppression_out);
    }

    fn module_type(&self) -> ModuleType { ModuleType::Contrast }
    fn num_curves(&self) -> usize { 2 }
}
```

### 2e: `src/dsp/modules/gain.rs`

Per-bin spectral gain. `GainMode` selects Add / Subtract / Pull.
- `curves[0]` = GAIN (linear multiplier; 1.0 = 0 dB; values > 1 = boost, < 1 = cut)
- `curves[1]` = SC SMOOTH (reserved in D1; sidechain smoothing placeholder)

```rust
use num_complex::Complex;
use crate::params::{FxChannelTarget, StereoLink};
use super::{GainMode, ModuleContext, ModuleType, SpectralModule};

pub struct GainModule {
    pub mode: GainMode,
}

impl GainModule {
    pub fn new() -> Self { Self { mode: GainMode::Add } }
}

impl SpectralModule for GainModule {
    fn reset(&mut self, _sample_rate: f32, _fft_size: usize) {}

    fn process(
        &mut self,
        _channel: usize,
        _stereo_link: StereoLink,
        _target: FxChannelTarget,
        bins: &mut [Complex<f32>],
        sidechain: Option<&[f32]>,
        curves: &[&[f32]],
        suppression_out: &mut [f32],
        _ctx: &ModuleContext,
    ) {
        let n = bins.len();
        match self.mode {
            GainMode::Add => {
                for k in 0..n {
                    let g = curves.get(0).and_then(|c| c.get(k)).copied().unwrap_or(1.0);
                    let sc_boost = sidechain.and_then(|sc| sc.get(k))
                                            .copied().unwrap_or(0.0).max(0.0);
                    bins[k] *= g + sc_boost;
                }
            }
            GainMode::Subtract => {
                for k in 0..n {
                    let g = curves.get(0).and_then(|c| c.get(k)).copied().unwrap_or(1.0);
                    let sc_cut = sidechain.and_then(|sc| sc.get(k))
                                          .copied().unwrap_or(0.0).max(0.0);
                    bins[k] *= (g - sc_cut).max(0.0);
                }
            }
            GainMode::Pull => {
                // Pull each bin's magnitude toward the sidechain bin's magnitude.
                for k in 0..n {
                    let g = curves.get(0).and_then(|c| c.get(k)).copied().unwrap_or(1.0);
                    let sc_mag = sidechain.and_then(|sc| sc.get(k)).copied().unwrap_or(0.0);
                    let cur_mag = bins[k].norm();
                    if cur_mag > 1e-10 {
                        let target_mag = cur_mag * g + sc_mag * (1.0 - g.clamp(0.0, 1.0));
                        bins[k] *= target_mag / cur_mag;
                    }
                }
            }
        }
        suppression_out.fill(0.0);
    }

    fn module_type(&self) -> ModuleType { ModuleType::Gain }
    fn num_curves(&self) -> usize { 2 }
}
```

### 2f: `src/dsp/modules/ts_split.rs`

Classifies each bin as transient or sustained. Uses ratio of current magnitude to a
windowed-average magnitude: bins with ratio > per-bin sensitivity route to `transient_out`;
the remainder to `sustained_out`. Extra accessors allow the pipeline to read both outputs.

`num_outputs()` returns `Some(2)` — signals two virtual rows to the routing matrix.

```rust
use num_complex::Complex;
use crate::params::{FxChannelTarget, StereoLink};
use super::{ModuleContext, ModuleType, SpectralModule};

pub struct TsSplitModule {
    avg_mag:       Vec<f32>,         // windowed average per bin
    transient_out: Vec<Complex<f32>>,
    sustained_out: Vec<Complex<f32>>,
    fft_size: usize,
}

impl TsSplitModule {
    pub fn new() -> Self {
        Self {
            avg_mag:       Vec::new(),
            transient_out: Vec::new(),
            sustained_out: Vec::new(),
            fft_size: 2048,
        }
    }

    /// Transient output bins (valid after `process()` returns).
    pub fn transient_bins(&self) -> &[Complex<f32>] { &self.transient_out }

    /// Sustained output bins (valid after `process()` returns).
    pub fn sustained_bins(&self) -> &[Complex<f32>] { &self.sustained_out }
}

impl SpectralModule for TsSplitModule {
    fn reset(&mut self, _sample_rate: f32, fft_size: usize) {
        self.fft_size = fft_size;
        let n = fft_size / 2 + 1;
        self.avg_mag       = vec![0.0f32;               n];
        self.transient_out = vec![Complex::new(0.0, 0.0); n];
        self.sustained_out = vec![Complex::new(0.0, 0.0); n];
    }

    fn process(
        &mut self,
        _channel: usize,
        _stereo_link: StereoLink,
        _target: FxChannelTarget,
        bins: &mut [Complex<f32>],
        _sidechain: Option<&[f32]>,
        curves: &[&[f32]],
        suppression_out: &mut [f32],
        _ctx: &ModuleContext,
    ) {
        let n = bins.len();
        // Slow-follower coefficient: ~200ms half-life at 44100/512 hop rate
        let slow_coeff: f32 = 0.98;

        for k in 0..n {
            let mag = bins[k].norm();
            self.avg_mag[k] = slow_coeff * self.avg_mag[k] + (1.0 - slow_coeff) * mag;

            let sensitivity = curves.get(0).and_then(|c| c.get(k))
                                     .copied().unwrap_or(1.0).clamp(0.0, 2.0);
            // A bin is transient if its magnitude exceeds the average by more than sensitivity.
            let is_transient = mag > self.avg_mag[k] * (1.0 + sensitivity);

            if is_transient {
                self.transient_out[k] = bins[k];
                self.sustained_out[k] = Complex::new(0.0, 0.0);
            } else {
                self.transient_out[k] = Complex::new(0.0, 0.0);
                self.sustained_out[k] = bins[k];
            }
        }
        // Pass-through: bins unchanged (downstream modules receive via virtual rows)
        suppression_out.fill(0.0);
    }

    fn tail_length(&self) -> u32 { self.fft_size as u32 }
    fn module_type(&self) -> ModuleType { ModuleType::TransientSustainedSplit }
    fn num_curves(&self) -> usize { 1 }
    fn num_outputs(&self) -> Option<usize> { Some(2) }
}
```

### 2g: `src/dsp/modules/harmonic.rs`

Pass-through stub. DSP TBD.

```rust
use num_complex::Complex;
use crate::params::{FxChannelTarget, StereoLink};
use super::{ModuleContext, ModuleType, SpectralModule};

pub struct HarmonicModule;

impl SpectralModule for HarmonicModule {
    fn reset(&mut self, _sample_rate: f32, _fft_size: usize) {}
    fn process(
        &mut self, _channel: usize, _stereo_link: StereoLink, _target: FxChannelTarget,
        _bins: &mut [Complex<f32>], _sidechain: Option<&[f32]>, _curves: &[&[f32]],
        suppression_out: &mut [f32], _ctx: &ModuleContext,
    ) { suppression_out.fill(0.0); }
    fn module_type(&self) -> ModuleType { ModuleType::Harmonic }
    fn num_curves(&self) -> usize { 0 }
}
```

### 2h: `src/dsp/modules/master.rs`

Transparent pass-through. Slot 8 only.

```rust
use num_complex::Complex;
use crate::params::{FxChannelTarget, StereoLink};
use super::{ModuleContext, ModuleType, SpectralModule};

pub struct MasterModule;

impl SpectralModule for MasterModule {
    fn reset(&mut self, _sample_rate: f32, _fft_size: usize) {}
    fn process(
        &mut self, _channel: usize, _stereo_link: StereoLink, _target: FxChannelTarget,
        _bins: &mut [Complex<f32>], _sidechain: Option<&[f32]>, _curves: &[&[f32]],
        suppression_out: &mut [f32], _ctx: &ModuleContext,
    ) { suppression_out.fill(0.0); }
    fn module_type(&self) -> ModuleType { ModuleType::Master }
    fn num_curves(&self) -> usize { 0 }
}
```

### 2i: `src/dsp/modules/mid_side.rs`

Stub. Full DSP in Plan D2 (ported from spectral2).

```rust
use num_complex::Complex;
use crate::params::{FxChannelTarget, StereoLink};
use super::{ModuleContext, ModuleType, SpectralModule};

pub struct MidSideModule;

impl MidSideModule {
    pub fn new() -> Self { Self }
}

impl SpectralModule for MidSideModule {
    fn reset(&mut self, _sample_rate: f32, _fft_size: usize) {}
    fn process(
        &mut self, _channel: usize, _stereo_link: StereoLink, _target: FxChannelTarget,
        _bins: &mut [Complex<f32>], _sidechain: Option<&[f32]>, _curves: &[&[f32]],
        suppression_out: &mut [f32], _ctx: &ModuleContext,
    ) { suppression_out.fill(0.0); }
    fn module_type(&self) -> ModuleType { ModuleType::MidSide }
    fn num_curves(&self) -> usize { 5 }
}
```

- [ ] **Step 1: Write all 9 module files** (copy code above into the respective files)

- [ ] **Step 2: Run compile test from Task 1**

```bash
cargo test module_trait_types_exist 2>&1 | head -30
```
Expected: PASS (all modules exist and compile).

- [ ] **Step 3: Run all existing tests**

```bash
cargo test 2>&1 | tail -20
```
Expected: all 14 existing tests pass (modules don't touch the audio path yet).

- [ ] **Step 4: Commit**

```bash
git add src/dsp/modules/
git commit -m "feat: add all SpectralModule implementations (dynamics, freeze, phase_smear, contrast, gain, ts_split, harmonic, master, mid_side)"
```

---

## Task 3: New params fields

**Files:** Modify `src/params.rs`

**Rule: Do not remove any existing fields.** Editor_ui.rs still uses them. They are removed in D2.

The new fields require types from `crate::dsp::modules` — Task 1 must be committed first.

- [ ] **Step 1: Add imports to `src/params.rs`**

At the top of `src/params.rs`, add:
```rust
use crate::dsp::modules::{GainMode, ModuleType, RouteMatrix};
```

- [ ] **Step 2: Add the 8 new persisted fields to `SpectralForgeParams`**

Add these fields after the existing `freeze_active_curve` field (before the float params section):

```rust
    // ── Per-slot modular architecture params ───────────────────────────────

    /// Module type assigned to each slot (0..=8). Slot 8 = Master, immutable.
    #[persist = "slot_module_types"]
    pub slot_module_types: Arc<Mutex<[ModuleType; 9]>>,

    /// User-editable UTF-8 name per slot, zero-padded to 32 bytes.
    #[persist = "slot_names"]
    pub slot_names: Arc<Mutex<[[u8; 32]; 9]>>,

    /// Channel routing target per slot (All / Mid / Side).
    #[persist = "slot_targets"]
    pub slot_targets: Arc<Mutex<[FxChannelTarget; 9]>>,

    /// Sidechain input assignment: 0..=3 = aux input index; 255 = self-detect.
    #[persist = "slot_sidechain"]
    pub slot_sidechain: Arc<Mutex<[u8; 9]>>,

    /// GainMode per slot (only meaningful for Gain module slots).
    #[persist = "slot_gain_mode"]
    pub slot_gain_mode: Arc<Mutex<[GainMode; 9]>>,

    /// Per-slot per-curve nodes. [slot 0..=8][curve 0..6][node 0..5].
    #[persist = "slot_curve_nodes"]
    pub slot_curve_nodes: Arc<Mutex<[[[CurveNode; NUM_NODES]; 7]; 9]>>,

    /// Per-slot per-curve tilt and offset. [slot][curve] = (tilt, offset). Default (0.0, 0.0).
    #[persist = "slot_curve_meta"]
    pub slot_curve_meta: Arc<Mutex<[[(f32, f32); 7]; 9]>>,

    /// Which curve within the editing slot is selected (0..num_curves for that type).
    #[persist = "editing_curve"]
    pub editing_curve: Arc<Mutex<u8>>,

    /// Routing matrix. Replaces fx_route_matrix in D2; both coexist during D1.
    #[persist = "route_matrix"]
    pub route_matrix: Arc<Mutex<RouteMatrix>>,
```

- [ ] **Step 3: Add defaults in `SpectralForgeParams::default()` / `impl Default`**

The `#[derive(Params)]` macro generates `default()` from field defaults. Each `Arc<Mutex<T>>` field needs an explicit `Default` impl or inline initialization. Use nih-plug's pattern:

Add a custom `Default` impl block that initialises the new fields. Find the existing
`impl Default for SpectralForgeParams` (or the `#[derive(Default)]` — check the file).
If the struct uses `#[derive(Default)]`, add:

```rust
impl SpectralForgeParams {
    fn make_default_slot_module_types() -> [ModuleType; 9] {
        [
            ModuleType::Dynamics,
            ModuleType::Dynamics,
            ModuleType::Gain,
            ModuleType::Empty,
            ModuleType::Empty,
            ModuleType::Empty,
            ModuleType::Empty,
            ModuleType::Empty,
            ModuleType::Master,
        ]
    }

    fn make_default_slot_names() -> [[u8; 32]; 9] {
        let mut names = [[0u8; 32]; 9];
        let labels = ["Dynamics", "Dynamics 2", "Gain", "Slot 3", "Slot 4",
                      "Slot 5", "Slot 6", "Slot 7", "Master"];
        for (i, label) in labels.iter().enumerate() {
            let bytes = label.as_bytes();
            let len = bytes.len().min(32);
            names[i][..len].copy_from_slice(&bytes[..len]);
        }
        names
    }
}
```

Then in the `Default` impl for `SpectralForgeParams`, initialise the new fields:

```rust
slot_module_types: Arc::new(Mutex::new(Self::make_default_slot_module_types())),
slot_names:        Arc::new(Mutex::new(Self::make_default_slot_names())),
slot_targets:      Arc::new(Mutex::new([FxChannelTarget::All; 9])),
slot_sidechain:    Arc::new(Mutex::new([255u8; 9])),
slot_gain_mode:    Arc::new(Mutex::new([GainMode::Add; 9])),
slot_curve_nodes:  Arc::new(Mutex::new(
    [[[CurveNode::default(); NUM_NODES]; 7]; 9])),
slot_curve_meta:   Arc::new(Mutex::new([[(0.0f32, 0.0f32); 7]; 9])),
editing_curve:     Arc::new(Mutex::new(0u8)),
route_matrix:      Arc::new(Mutex::new(RouteMatrix::default())),
```

Note: Check whether `SpectralForgeParams` uses `#[derive(Default)]` or has a manual `impl Default`. Adjust accordingly — if nih-plug generates `default()` from the `#[derive(Params)]` macro, the above helper initialisation goes into `impl Default for SpectralForgeParams`.

- [ ] **Step 4: Ensure `CurveNode` implements `Default`**

`slot_curve_nodes` uses `[[[CurveNode; NUM_NODES]; 7]; 9]` which requires `CurveNode: Default + Copy`. Verify in `src/editor/curve.rs`. If `CurveNode` doesn't derive `Copy` and `Default`, add them:

```rust
#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
pub struct CurveNode { ... }
```

- [ ] **Step 5: Compile check**

```bash
cargo build 2>&1 | grep "^error" | head -20
```
Expected: no errors. If nih-plug complains about a `#[persist]` type not implementing serde, add `#[derive(Serialize, Deserialize)]` to the missing type.

- [ ] **Step 6: Run all tests**

```bash
cargo test 2>&1 | tail -10
```
Expected: all 14 tests pass.

- [ ] **Step 7: Commit**

```bash
git add src/params.rs src/editor/curve.rs
git commit -m "feat: add per-slot params (slot_module_types, slot_curve_nodes, route_matrix, etc.)"
```

---

## Task 4: Bridge restructure + lib.rs + minimal editor_ui.rs (atomic)

**Files:** `src/bridge.rs`, `src/lib.rs`, `src/editor_ui.rs`

These three files must compile together — change all three before committing.

### 4a — `src/bridge.rs`

Replace flat 7-channel `curve_tx/rx` with 9×7, add 4 `sidechain_active`, remove `pending_engine`.

The new `SharedState::new` signature adds `fft_size: usize` (used for future per-slot metadata; pass through for now).

```rust
use parking_lot::Mutex;
use std::sync::{atomic::{AtomicBool, AtomicU32, Ordering}, Arc};
use triple_buffer::{Input as TbInput, Output as TbOutput, TripleBuffer};

pub use std::sync::atomic::AtomicU32 as AtomicF32;  // reuse existing alias if present

pub struct SharedState {
    /// curve_tx[slot][curve] = GUI write handle.
    pub curve_tx: Vec<Vec<Arc<Mutex<TbInput<Vec<f32>>>>>>,
    /// curve_rx[slot][curve] = audio-thread read handle.
    pub curve_rx: Vec<Vec<TbOutput<Vec<f32>>>>,

    /// Whether each of the 4 aux sidechain inputs has signal.
    pub sidechain_active: [Arc<AtomicBool>; 4],

    // Spectrum/suppression display buffers (unchanged)
    pub spectrum_tx:    TbInput<Vec<f32>>,
    pub spectrum_rx:    Arc<Mutex<TbOutput<Vec<f32>>>>,
    pub suppression_tx: TbInput<Vec<f32>>,
    pub suppression_rx: Arc<Mutex<TbOutput<Vec<f32>>>>,

    pub sample_rate: Arc<AtomicF32>,
    pub num_bins:    usize,
}

impl SharedState {
    pub fn new(num_bins: usize, sample_rate: f32) -> Self {
        let init_curve = || vec![1.0f32; num_bins];

        // 9 slots × 7 curves each
        let mut curve_tx = Vec::with_capacity(9);
        let mut curve_rx = Vec::with_capacity(9);
        for _ in 0..9 {
            let mut slot_tx = Vec::with_capacity(7);
            let mut slot_rx = Vec::with_capacity(7);
            for _ in 0..7 {
                let (inp, out) = TripleBuffer::new(&init_curve()).split();
                slot_tx.push(Arc::new(Mutex::new(inp)));
                slot_rx.push(out);
            }
            curve_tx.push(slot_tx);
            curve_rx.push(slot_rx);
        }

        let (spec_tx, spec_rx)   = TripleBuffer::new(&vec![0.0f32; num_bins]).split();
        let (supp_tx, supp_rx)   = TripleBuffer::new(&vec![0.0f32; num_bins]).split();

        // AtomicF32 alias — use AtomicU32 with f32::to_bits if that's the existing convention.
        // Check bridge.rs for the existing type; replicate here.
        let sr_bits = sample_rate.to_bits();
        let sample_rate_atomic = Arc::new(AtomicU32::new(sr_bits));

        Self {
            curve_tx,
            curve_rx,
            sidechain_active: [
                Arc::new(AtomicBool::new(false)),
                Arc::new(AtomicBool::new(false)),
                Arc::new(AtomicBool::new(false)),
                Arc::new(AtomicBool::new(false)),
            ],
            spectrum_tx:    spec_tx,
            spectrum_rx:    Arc::new(Mutex::new(spec_rx)),
            suppression_tx: supp_tx,
            suppression_rx: Arc::new(Mutex::new(supp_rx)),
            sample_rate:    sample_rate_atomic,
            num_bins,
        }
    }
}
```

**Note:** Look at the existing `bridge.rs` for the exact `AtomicF32` definition. If it's a type alias for `AtomicU32` with `f32::to_bits()` encoding, keep that pattern. Copy the `sample_rate` getter/setter methods if they exist.

### 4b — `src/lib.rs`

Three changes:
1. Expand `AUDIO_IO_LAYOUTS` to 4 aux sidechain inputs.
2. Change `gui_curve_tx` from `Vec<Arc<...>>` to `Vec<Vec<Arc<...>>>`.
3. Remove references to `gui_phase_curve_tx` and `gui_freeze_curve_tx`.

```rust
// In Plugin impl:
const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[
    AudioIOLayout {
        main_input_channels:  NonZeroU32::new(2),
        main_output_channels: NonZeroU32::new(2),
        aux_input_ports: &[
            new_nonzero_u32(2),  // sidechain 1
            new_nonzero_u32(2),  // sidechain 2
            new_nonzero_u32(2),  // sidechain 3
            new_nonzero_u32(2),  // sidechain 4
        ],
        ..AudioIOLayout::const_default()
    },
    AudioIOLayout {
        main_input_channels:  NonZeroU32::new(2),
        main_output_channels: NonZeroU32::new(2),
        ..AudioIOLayout::const_default()
    },
];
```

Change the `SpectralForge` struct:
```rust
pub struct SpectralForge {
    params:   Arc<SpectralForgeParams>,
    pipeline: Option<dsp::pipeline::Pipeline>,
    shared:   Option<bridge::SharedState>,
    // 9×7 curve handles for the GUI
    gui_curve_tx: Vec<Vec<Arc<parking_lot::Mutex<triple_buffer::Input<Vec<f32>>>>>>,
    // Remove: gui_phase_curve_tx, gui_freeze_curve_tx
    gui_sample_rate:    Option<Arc<bridge::AtomicF32>>,
    gui_num_bins:       usize,
    gui_spectrum_rx:    Option<Arc<parking_lot::Mutex<triple_buffer::Output<Vec<f32>>>>>,
    gui_suppression_rx: Option<Arc<parking_lot::Mutex<triple_buffer::Output<Vec<f32>>>>>,
    plugin_alive: Arc<()>,
    num_channels: usize,
    sample_rate:  f32,
}
```

Update `Default::default()`:
```rust
impl Default for SpectralForge {
    fn default() -> Self {
        let dummy_sr = 44100.0;
        let num_bins = dsp::pipeline::FFT_SIZE / 2 + 1;
        let shared = bridge::SharedState::new(num_bins, dummy_sr);

        let gui_curve_tx    = shared.curve_tx.clone();
        let gui_sample_rate = Some(shared.sample_rate.clone());
        let gui_num_bins    = shared.num_bins;
        let gui_spectrum_rx    = Some(shared.spectrum_rx.clone());
        let gui_suppression_rx = Some(shared.suppression_rx.clone());

        Self {
            params:   Arc::new(SpectralForgeParams::default()),
            pipeline: None,
            shared:   Some(shared),
            gui_curve_tx,
            gui_sample_rate,
            gui_num_bins,
            gui_spectrum_rx,
            gui_suppression_rx,
            plugin_alive: Arc::new(()),
            num_channels: 2,
            sample_rate:  dummy_sr,
        }
    }
}
```

In `initialize()`, update the curve push loop:
```rust
// Push 9×7 curves from persisted slot_curve_nodes
let nodes = self.params.slot_curve_nodes.lock();
for slot in 0..9 {
    for curve in 0..7 {
        let gains = crate::editor::curve::compute_curve_response(
            &nodes[slot][curve], num_bins);
        let mut tx = self.gui_curve_tx[slot][curve].lock();
        tx.input_buffer_mut().copy_from_slice(&gains);
        tx.publish();
    }
}
```

### 4c — `src/editor_ui.rs` (minimal)

Find all uses of `shared.gui_curve_tx[curve]` (or however the editor accesses the flat 7 channels)
and change them to `shared.gui_curve_tx[editing_slot][curve]` where `editing_slot` is
the value of `params.editing_slot.value() as usize`.

Also remove any references to `shared.gui_phase_curve_tx` and `shared.gui_freeze_curve_tx`.
These display paths (phase/freeze curve display) will be dead code in D1 — wrap the
entire phase/freeze UI block in a `#[cfg(any())]` or simply delete the dead code
(since the old `active_tab` param still exists, the branches can be left as dead code
if they now compile without the missing fields).

The key change is:
```rust
// Before:
let mut tx = shared.gui_curve_tx[ci].lock();

// After:
let editing_slot = params.editing_slot.value() as usize;
let mut tx = shared.gui_curve_tx[editing_slot][ci].lock();
```

- [ ] **Step 1: Make all three changes in bridge.rs, lib.rs, editor_ui.rs**

- [ ] **Step 2: Compile check**

```bash
cargo build 2>&1 | grep "^error" | head -30
```
Fix any remaining compilation errors (usually unused variable warnings or a missed reference to the old flat tx).

- [ ] **Step 3: Run all tests**

```bash
cargo test 2>&1 | tail -10
```
Expected: all 14 tests pass.

- [ ] **Step 4: Commit (atomic — all three files)**

```bash
git add src/bridge.rs src/lib.rs src/editor_ui.rs
git commit -m "refactor: bridge → 9×7 curve channels; 4 sidechain_active; 4 CLAP aux inputs"
```

---

## Task 5: Pipeline refactor

**Files:** Modify `src/dsp/pipeline.rs`

Replace `curve_cache: [Vec<f32>; 7]` with `slot_curve_cache: Vec<Vec<Vec<f32>>>` (9×7×num_bins).
Add 4 sidechain paths. Shrink the STFT closure by removing the `match effect_mode` block
(DSP now lives in modules). Call `fx_matrix.process_hop(...)` with the new signature.

- [ ] **Step 1: Update Pipeline struct fields**

Remove these fields (the DSP they owned now lives in module files):
- `frozen_bins`, `freeze_target`, `freeze_port_t`, `freeze_hold_hops`, `freeze_accum`, `freeze_captured`
- `rng_state`
- `curve_cache: [Vec<f32>; 7]`
- `phase_curve_cache`
- `freeze_curve_cache`
- `bp_threshold`, `bp_ratio`, `bp_attack`, `bp_release`, `bp_knee`, `bp_makeup`, `bp_mix`

Add:
```rust
/// Pre-allocated per-slot curve cache. [slot][curve][bin]
slot_curve_cache: Vec<Vec<Vec<f32>>>,
/// Per-aux sidechain envelope followers (one per aux input, up to 4).
sc_envelopes: [Vec<f32>; 4],
sc_env_states: [Vec<f32>; 4],
```

- [ ] **Step 2: Update `Pipeline::new()` / `reset()`**

In `reset()`:
```rust
let num_bins = fft_size / 2 + 1;

// Pre-allocate 9×7 curve cache
self.slot_curve_cache = (0..9).map(|_| {
    (0..7).map(|_| vec![1.0f32; num_bins]).collect()
}).collect();

// Pre-allocate 4 sidechain envelopes
for i in 0..4 {
    self.sc_envelopes[i]   = vec![0.0f32; num_bins];
    self.sc_env_states[i]  = vec![0.0f32; num_bins];
}
```

- [ ] **Step 3: Replace curve-cache read block**

The existing code reads from `shared.curve_rx[c]` for 7 curves. Replace with:

```rust
use crate::dsp::modules::apply_curve_transform;

let slot_curve_meta = params.slot_curve_meta.lock();
for s in 0..9 {
    for c in 0..7 {
        let src = shared.curve_rx[s][c].read();
        self.slot_curve_cache[s][c][..num_bins].copy_from_slice(&src[..num_bins]);
        let (tilt, offset) = slot_curve_meta[s][c];
        apply_curve_transform(&mut self.slot_curve_cache[s][c][..num_bins], tilt, offset);
    }
}
```

Note: `shared.curve_rx` is now `&mut` per TripleBuffer convention — `read()` requires `&mut self`. Ensure the pipeline holds the rx array with `&mut` access (it should, since it owns the SharedState rx side).

- [ ] **Step 4: Add 4 sidechain paths**

Replace the existing single-sidechain STFT block:

```rust
// Process up to 4 aux sidechain inputs
let mut sc_active_flags = [false; 4];
for (i, sc_env) in self.sc_envelopes.iter_mut().enumerate() {
    let aux_idx = i;
    let has_aux = context.aux.inputs.get(aux_idx).map(|a| !a.is_empty()).unwrap_or(false);
    if !has_aux { continue; }

    // Run envelope follower on aux input i (same logic as existing single-SC path)
    let sc_buf = context.aux.inputs[aux_idx].as_slice();
    // ... (copy existing sc_stft logic, adapting index)
    sc_active_flags[i] = sc_env.iter().any(|&v| v > 1e-9);
}

for i in 0..4 {
    shared.sidechain_active[i].store(sc_active_flags[i], Ordering::Relaxed);
}
```

Build per-slot sidechain slices (no allocation — use references):
```rust
let slot_sidechain = params.slot_sidechain.lock();
// sc_args[slot] = Option<&[f32]> chosen based on slot_sidechain[slot]
let sc_args: [Option<&[f32]>; 9] = std::array::from_fn(|s| {
    let idx = slot_sidechain[s];
    if idx == 255 {
        if sc_active_flags[0] { Some(self.sc_envelopes[0].as_slice()) } else { None }
    } else {
        let i = idx as usize;
        if i < 4 && sc_active_flags[i] { Some(self.sc_envelopes[i].as_slice()) } else { None }
    }
});
```

- [ ] **Step 5: Shrink STFT closure**

Remove the entire `match effect_mode { Freeze => {...}, PhaseRand => {...} }` block
from inside the STFT closure. Replace with:

```rust
// Build slot_curves references (stack-allocated, no heap)
let slot_targets  = params.slot_targets.lock();
let slot_sc_local = &sc_args;
let ctx = ModuleContext {
    sample_rate:       self.sample_rate,
    fft_size:          FFT_SIZE,
    num_bins,
    attack_ms:         attack_ms_base,
    release_ms:        release_ms_base,
    sensitivity:       params.sensitivity.value(),
    suppression_width: params.suppression_width.value(),
    auto_makeup:       params.auto_makeup.value(),
    delta_monitor:     params.delta_monitor.value(),
};

fx_matrix.process_hop(
    channel,
    stereo_link,
    complex_buf,
    slot_sc_local,
    &*slot_sidechain,
    &*slot_targets,
    &self.slot_curve_cache,
    &ctx,
    channel_supp_buf,
    num_bins,
);
```

Note: The `slot_curve_cache` is captured by reference from `self` before the closure borrows `self.stft` mutably. Use the Rust 2021 split-field borrow pattern (rebind as a local before the call).

- [ ] **Step 6: Remove now-dead imports and variables**

Remove `EffectMode`, `effect_mode`, `phase_rand_amount`, `spectral_contrast_db`, the old tilt/offset adjuster closure, and all `bp_*` field references. The compiler will flag these.

- [ ] **Step 7: Compile and test**

```bash
cargo build 2>&1 | grep "^error" | head -20
cargo test 2>&1 | tail -10
```
Expected: compiles cleanly; all 14 tests pass.

- [ ] **Step 8: Commit**

```bash
git add src/dsp/pipeline.rs
git commit -m "refactor: pipeline uses slot_curve_cache, 4 sidechain paths, calls fx_matrix with new API"
```

---

## Task 6: FxMatrix refactor

**Files:** Modify `src/dsp/fx_matrix.rs`

Replace `FxSlotKind` enum with `Box<dyn SpectralModule>`. Wire `RouteMatrix`. Add Master at slot 8. Implement the new `process_hop` signature.

- [ ] **Step 1: Replace the struct**

Delete `FxSlotKind` enum entirely. Replace `FxMatrix` struct:

```rust
use num_complex::Complex;
use crate::dsp::modules::{
    ModuleContext, ModuleType, RouteMatrix, SpectralModule,
    MAX_MATRIX_ROWS, MAX_SLOTS, MAX_SPLIT_VIRTUAL_ROWS,
    create_module,
    ts_split::TsSplitModule,
};
use crate::params::{FxChannelTarget, StereoLink};

pub struct FxMatrix {
    pub slots: [Option<Box<dyn SpectralModule>>; MAX_SLOTS],
    pub route: RouteMatrix,
    /// Per-slot output buffers (current hop). [slot][bin]
    slot_out: Vec<Vec<Complex<f32>>>,
    /// Per-slot suppression output. [slot][bin]
    slot_supp: Vec<Vec<f32>>,
    /// Virtual row output buffers for T/S Split. [vrow][bin]
    virtual_out: Vec<Vec<Complex<f32>>>,
    /// Working mix buffer (reused each slot, no allocation).
    mix_buf: Vec<Complex<f32>>,
}
```

- [ ] **Step 2: Implement `FxMatrix::new()`**

```rust
impl FxMatrix {
    pub fn new(sample_rate: f32, fft_size: usize) -> Self {
        let num_bins = fft_size / 2 + 1;
        // Slot 8 = Master, always present
        let slots: [Option<Box<dyn SpectralModule>>; MAX_SLOTS] = std::array::from_fn(|i| {
            if i == 8 {
                Some(create_module(ModuleType::Master, sample_rate, fft_size))
            } else if i < 2 {
                Some(create_module(ModuleType::Dynamics, sample_rate, fft_size))
            } else if i == 2 {
                Some(create_module(ModuleType::Gain, sample_rate, fft_size))
            } else {
                None  // Empty slots
            }
        });

        Self {
            slots,
            route: RouteMatrix::default(),
            slot_out:    (0..MAX_SLOTS).map(|_| vec![Complex::new(0.0, 0.0); num_bins]).collect(),
            slot_supp:   (0..MAX_SLOTS).map(|_| vec![0.0f32; num_bins]).collect(),
            virtual_out: (0..MAX_SPLIT_VIRTUAL_ROWS)
                             .map(|_| vec![Complex::new(0.0, 0.0); num_bins]).collect(),
            mix_buf: vec![Complex::new(0.0, 0.0); num_bins],
        }
    }

    pub fn reset(&mut self, sample_rate: f32, fft_size: usize) {
        let num_bins = fft_size / 2 + 1;
        for slot in self.slots.iter_mut().flatten() {
            slot.reset(sample_rate, fft_size);
        }
        for buf in &mut self.slot_out   { buf.resize(num_bins, Complex::new(0.0, 0.0)); }
        for buf in &mut self.slot_supp  { buf.resize(num_bins, 0.0); }
        for buf in &mut self.virtual_out { buf.resize(num_bins, Complex::new(0.0, 0.0)); }
        self.mix_buf.resize(num_bins, Complex::new(0.0, 0.0));
    }
}
```

- [ ] **Step 3: Implement `process_hop`**

```rust
impl FxMatrix {
    pub fn process_hop(
        &mut self,
        channel: usize,
        stereo_link: StereoLink,
        complex_buf: &mut [Complex<f32>],
        sc_args: &[Option<&[f32]>; 9],
        slot_sidechain: &[u8; 9],
        slot_targets: &[FxChannelTarget; 9],
        slot_curves: &Vec<Vec<Vec<f32>>>,  // [slot][curve][bin]
        ctx: &ModuleContext,
        suppression_out: &mut [f32],
        num_bins: usize,
    ) {
        // ── Step 1: Collect input signal into slot 0 mix (input bus)
        // The signal arriving here is already the input bus.
        // We treat the incoming `complex_buf` as the source for all slots that
        // have send weight from the "input" — in this architecture, all real slots
        // read from `complex_buf` as their source (mixed via route matrix weights).

        // ── Step 2: Process each slot in order (0..8)
        for s in 0..MAX_SLOTS {
            let Some(module) = &mut self.slots[s] else { continue; };

            // Build the input for this slot: weighted sum of sources that send to it.
            self.mix_buf.fill(Complex::new(0.0, 0.0));
            let mut has_input = false;

            // Real slots 0..MAX_SLOTS as sources
            for src in 0..MAX_SLOTS {
                let w = self.route.send[src][s];
                if w.abs() < 1e-9 { continue; }
                if src == 0 && s == 0 {
                    // Slot 0 reads from the main input bus
                    for k in 0..num_bins {
                        self.mix_buf[k] += complex_buf[k] * w;
                    }
                    has_input = true;
                } else if src < s {
                    // Only backward sends in D1 (forward sends are a D2 topology feature).
                    for k in 0..num_bins {
                        self.mix_buf[k] += self.slot_out[src][k] * w;
                    }
                    has_input = true;
                }
            }
            // Virtual row sources
            for (vr_idx, vr) in self.route.virtual_rows.iter().enumerate() {
                if let Some((_real_slot, _kind)) = vr {
                    let w = self.route.send[MAX_SLOTS + vr_idx][s];
                    if w.abs() < 1e-9 { continue; }
                    for k in 0..num_bins {
                        self.mix_buf[k] += self.virtual_out[vr_idx][k] * w;
                    }
                    has_input = true;
                }
            }
            // Default: if no send reaches this slot, use main input bus (for slot 0 only)
            if !has_input && s == 0 {
                self.mix_buf[..num_bins].copy_from_slice(&complex_buf[..num_bins]);
            }

            // Build curve slice references (stack-allocated)
            let nc = module.num_curves().min(7);
            let curves_storage: [&[f32]; 7] = std::array::from_fn(|c| {
                if c < nc { &slot_curves[s][c][..num_bins] } else { &[] }
            });
            let curves: &[&[f32]] = &curves_storage[..nc];

            // Select sidechain
            let sidechain = sc_args[s];

            module.process(
                channel,
                stereo_link,
                slot_targets[s],
                &mut self.mix_buf[..num_bins],
                sidechain,
                curves,
                &mut self.slot_supp[s][..num_bins],
                ctx,
            );

            self.slot_out[s][..num_bins].copy_from_slice(&self.mix_buf[..num_bins]);

            // If T/S Split, populate virtual row buffers
            if module.num_outputs() == Some(2) {
                // Downcast to access transient_bins() / sustained_bins()
                // Use virtual_rows to find which vr indices belong to this slot.
                for (vr_idx, vr) in self.route.virtual_rows.iter().enumerate() {
                    if let Some((real_slot, kind)) = vr {
                        if *real_slot as usize == s {
                            // Unsafe downcast: we know it's a TsSplitModule
                            // Safe because: only TsSplitModule returns num_outputs() == Some(2).
                            let ts = unsafe {
                                &*(module.as_ref() as *const dyn SpectralModule
                                   as *const TsSplitModule)
                            };
                            let src = match kind {
                                crate::dsp::modules::VirtualRowKind::Transient => ts.transient_bins(),
                                crate::dsp::modules::VirtualRowKind::Sustained => ts.sustained_bins(),
                            };
                            self.virtual_out[vr_idx][..num_bins].copy_from_slice(src);
                        }
                    }
                }
            }
        }

        // ── Step 3: Master (slot 8) output → write back to complex_buf
        complex_buf[..num_bins].copy_from_slice(&self.slot_out[8][..num_bins]);

        // ── Step 4: Max-reduce suppression across all slots for display
        suppression_out.fill(0.0);
        for s in 0..MAX_SLOTS {
            for k in 0..num_bins {
                if self.slot_supp[s][k] > suppression_out[k] {
                    suppression_out[k] = self.slot_supp[s][k];
                }
            }
        }
    }
}
```

**Note on the unsafe downcast:** The raw pointer cast is safe here because `num_outputs() == Some(2)` is only ever returned by `TsSplitModule`. A cleaner alternative for D2 is to add a `fn as_ts_split(&self) -> Option<&TsSplitModule>` to the trait with a default `None` impl; for D1 the cast is acceptable.

- [ ] **Step 4: Remove old `process_hop` / `FxSlotKind` references from `pipeline.rs`**

The old `fx_matrix.process_hop(...)` call in pipeline.rs used a different signature.
Update the call site to match the new signature from Step 3.

- [ ] **Step 5: Compile and test**

```bash
cargo build 2>&1 | grep "^error" | head -20
cargo test 2>&1 | tail -10
```
Expected: compiles; all 14 tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/dsp/fx_matrix.rs src/dsp/pipeline.rs
git commit -m "refactor: fx_matrix uses Box<dyn SpectralModule>, RouteMatrix, Master at slot 8"
```

---

## Task 7: Preset architecture

**Files:** Create `src/presets.rs`, modify `src/lib.rs`

Add `pub mod presets;` to `src/lib.rs`.

```rust
// src/presets.rs

use serde::{Deserialize, Serialize};
use crate::dsp::modules::{GainMode, ModuleType, RouteMatrix};
use crate::editor::curve::CurveNode;
use crate::params::{FxChannelTarget, NUM_NODES};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginState {
    pub slot_module_types: [ModuleType; 9],
    pub slot_names:        [[u8; 32]; 9],
    pub slot_targets:      [FxChannelTarget; 9],
    pub slot_sidechain:    [u8; 9],
    pub slot_gain_mode:    [GainMode; 9],
    pub slot_curve_nodes:  [[[CurveNode; NUM_NODES]; 7]; 9],
    pub slot_curve_meta:   [[(f32, f32); 7]; 9],
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

fn no_meta() -> [(f32, f32); 7] {
    [(0.0, 0.0); 7]
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
        slot_sidechain: [255u8; 9],
        slot_gain_mode: [GainMode::Add; 9],
        slot_curve_nodes: [neutral_curves(); 9],
        slot_curve_meta:  [no_meta(); 9],
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

    // Routing: 0→8(1.0), 1T→3(1.0), 1S→2(1.0), 2→8(1.0), 3→8(1.0)
    state.route = RouteMatrix::default();
    state.route.send[0][8] = 1.0;   // Dynamics → Master
    state.route.send[1][2] = 1.0;   // T/S → Freeze (direct; vrows wired at runtime)
    state.route.send[2][8] = 1.0;   // Freeze → Master
    state.route.send[3][8] = 1.0;   // Gain → Master
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
    state.route.send[0][1] = 1.0;   // Dyn → PhaseSmear
    state.route.send[1][2] = 1.0;   // PhaseSmear → Contrast
    state.route.send[2][8] = 1.0;   // Contrast → Master
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
    state.route.send[0][1] = 1.0;   // Freeze → Gain
    state.route.send[1][8] = 1.0;   // Gain → Master
    state.route.send[2][8] = 0.0;   // Empty
    state
}
```

- [ ] **Step 1: Wire CLAP factory preset discovery in `src/lib.rs`**

nih-plug exposes CLAP factory presets via a `ClapPlugin` trait method. Add to the `ClapPlugin` impl:

```rust
impl ClapPlugin for SpectralForge {
    // ... existing methods ...

    fn clap_preset_discovery_factories(
        &self,
    ) -> Option<Vec<nih_plug::prelude::clap_preset_discovery::PresetFile>> {
        use crate::presets::*;
        let presets: &[(&str, fn() -> crate::presets::PluginState)] = &[
            ("Default",            preset_default),
            ("Transient Sculptor", preset_transient_sculptor),
            ("Spectral Width",     preset_spectral_width),
            ("Phase Sculptor",     preset_phase_sculptor),
            ("Freeze Pad",         preset_freeze_pad),
        ];
        // If nih-plug's CLAP preset API isn't available in this version,
        // stub this out and return None — presets are still accessible via
        // PluginState serialization and the DAW's own preset mechanisms.
        None  // TODO: wire when nih-plug stabilises ClapPresetDiscovery API
    }
}
```

**Note:** nih-plug's CLAP preset discovery API may not be stable yet. If `clap_preset_discovery_factories` doesn't exist on the trait, skip this step and leave a TODO comment. The `PluginState` struct and builder functions are the deliverable — the CLAP wiring is secondary.

- [ ] **Step 2: Add `pub mod presets;` to `src/lib.rs`**

```rust
pub mod dsp;
pub mod editor;
pub mod editor_ui;
pub mod params;
pub mod bridge;
pub mod presets;  // ← add
```

- [ ] **Step 3: Write tests for preset builders**

```rust
// tests/presets.rs
#[test]
fn preset_default_has_correct_types() {
    let s = spectral_forge::presets::preset_default();
    use spectral_forge::dsp::modules::ModuleType;
    assert_eq!(s.slot_module_types[0], ModuleType::Dynamics);
    assert_eq!(s.slot_module_types[1], ModuleType::Dynamics);
    assert_eq!(s.slot_module_types[2], ModuleType::Gain);
    assert_eq!(s.slot_module_types[8], ModuleType::Master);
    // Slots 3-7 are Empty
    for i in 3..8 {
        assert_eq!(s.slot_module_types[i], ModuleType::Empty);
    }
}

#[test]
fn preset_roundtrips_through_json() {
    let s = spectral_forge::presets::preset_default();
    let json = serde_json::to_string(&s).expect("serialize");
    let s2: spectral_forge::presets::PluginState = serde_json::from_str(&json)
        .expect("deserialize");
    assert_eq!(s.slot_module_types, s2.slot_module_types);
    assert_eq!(s.slot_sidechain, s2.slot_sidechain);
}

#[test]
fn all_presets_compile_and_serialize() {
    use spectral_forge::presets::*;
    let builders: &[fn() -> PluginState] = &[
        preset_default,
        preset_transient_sculptor,
        preset_spectral_width,
        preset_phase_sculptor,
        preset_freeze_pad,
    ];
    for build in builders {
        let state = build();
        let json = serde_json::to_string(&state).expect("serialize failed");
        assert!(!json.is_empty());
    }
}
```

- [ ] **Step 4: Add `serde_json` to `Cargo.toml`**

```toml
[dependencies]
serde_json = "1"
```

- [ ] **Step 5: Run all tests**

```bash
cargo test 2>&1 | tail -20
```
Expected: all original 14 tests pass, plus the 3 new preset tests = 17 total.

- [ ] **Step 6: Commit**

```bash
git add src/presets.rs src/lib.rs Cargo.toml tests/presets.rs
git commit -m "feat: PluginState struct and 5 typed preset builders (preset_default, transient_sculptor, spectral_width, phase_sculptor, freeze_pad)"
```

---

## Final verification

- [ ] **Build release and confirm it bundles**

```bash
cargo build --release 2>&1 | grep "^error"
cargo run --package xtask -- bundle spectral_forge --release
```
Expected: clean release build; `.clap` produced in `target/bundled/`.

- [ ] **Run complete test suite**

```bash
cargo test 2>&1 | tail -5
```
Expected: all tests pass (at minimum the original 14 plus new ones from Tasks 1, 2, 7).

- [ ] **Install and smoke-test in Bitwig**

```bash
cp target/bundled/spectral_forge.clap ~/.clap/
```

In Bitwig (after rescan):
- Insert plugin on an audio track playing `test_flac/breakbeat_4030hz_bell-curve-high-q_resonance.flac`
- Confirm audio plays and compression is audible (Dynamics slot active)
- Open the plugin GUI — curve editor should show the dynamics curves for slot 0
- Confirm no crashes or silent output

---

## Notes for D2

These are left deliberately incomplete in D1 (flagged with TODO):
- `editor_ui.rs` still reads old `params.curve_nodes` — D2 switches to `slot_curve_nodes`
- Old params fields (`curve_nodes`, `active_curve`, `phase_curve_nodes`, etc.) are removed in D2
- `EffectMode` enum in `params.rs` is removed in D2
- Phase/freeze tab UI code in `editor_ui.rs` is dead code in D1, replaced by D2 adaptive UI
- CLAP factory preset discovery wired when nih-plug API stabilises
- `MidSideModule` is a stub; full DSP ported in D2 Task 8
- T/S Split virtual row UI (half-height rows) is D2 Task 7
