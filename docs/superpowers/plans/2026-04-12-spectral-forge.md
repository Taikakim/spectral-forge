> **Status (2026-04-24): IMPLEMENTED — historical.** The foundational plan that bootstrapped the plugin. The single-engine `Pipeline` it describes has since been replaced by the modular `FxMatrix`; this plan is kept only for history. Source of truth: the code + [../STATUS.md](../STATUS.md).

# Spectral Forge Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a Soothe-style spectral compressor CLAP plugin for Linux/Bitwig with a 7-parameter EQ curve GUI and per-bin dynamic compression.

**Architecture:** STFT (2048-point, 75% overlap) → per-bin compression using BinParams slices pre-computed from 7 EQ-style parameter curves → ISTFT. GUI runs the biquad computation on node change and sends `Vec<f32>` gains to the audio thread via triple-buffer. All visual constants live in `editor/theme.rs`.

**Tech Stack:** Rust, nih-plug (CLAP), nih-plug-egui, realfft, triple_buffer, parking_lot

---

## File Map

| File | Responsibility |
|------|---------------|
| `Cargo.toml` | Workspace + deps |
| `xtask/src/main.rs` | Bundle command |
| `src/lib.rs` | Plugin struct, CLAP export, initialize/reset/process/editor |
| `src/params.rs` | All global `FloatParam`/`EnumParam`/`BoolParam` definitions |
| `src/bridge.rs` | `SharedState`: 7 curve channels + spectrum + suppression triple-buffers |
| `src/editor.rs` | Top-level egui layout, parameter selector buttons |
| `src/editor/mod.rs` | Re-exports |
| `src/editor/theme.rs` | All colours, stroke widths, fonts — single reskin point |
| `src/editor/curve.rs` | `CurveNode`, biquad response, egui curve widget |
| `src/editor/spectrum_display.rs` | Background magnitude bars |
| `src/editor/suppression_display.rs` | Stalactite suppression bars |
| `src/dsp/mod.rs` | Re-exports |
| `src/dsp/guard.rs` | `sanitize()`, `flush_denormals()`, `is_ready()` |
| `src/dsp/pipeline.rs` | `StftHelper` setup, sidechain path, engine dispatch, BinParams assembly |
| `src/dsp/engines/mod.rs` | `SpectralEngine` trait, `BinParams`, `EngineSelection`, `create_engine()` |
| `src/dsp/engines/spectral_compressor.rs` | Per-bin compressor: threshold, ratio, knee, attack/release, bin-linking |
| `src/dsp/engines/README.md` | How to add a new engine |
| `tests/stft_roundtrip.rs` | Identity engine preserves 440 Hz sine |
| `tests/curve_sampling.rs` | Flat curve → unity gains |
| `tests/engine_contract.rs` | Engine trait invariants |

---

## Task 1: Cargo.toml + xtask scaffolding

**Files:**
- Create: `Cargo.toml`
- Create: `xtask/Cargo.toml`
- Create: `xtask/src/main.rs`

- [ ] **Create workspace `Cargo.toml`**

```toml
[workspace]
members = ["xtask"]
resolver = "2"

[package]
name = "spectral_forge"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]

[dependencies]
nih_plug = { git = "https://github.com/robbert-vdh/nih-plug.git", features = ["assert_process_allocs"] }
nih_plug_egui = { git = "https://github.com/robbert-vdh/nih-plug.git" }
realfft = "3"
triple_buffer = "9"
parking_lot = "0.12"
num-complex = "0.4"
serde = { version = "1", features = ["derive"] }

[dev-dependencies]
approx = "0.5"

[profile.release]
lto = "thin"
opt-level = 3
strip = "symbols"

[profile.dev]
opt-level = 1
```

- [ ] **Create `xtask/Cargo.toml`**

```toml
[package]
name = "xtask"
version = "0.1.0"
edition = "2021"

[dependencies]
nih_plug_xtask = { git = "https://github.com/robbert-vdh/nih-plug.git" }
```

- [ ] **Create `xtask/src/main.rs`**

```rust
fn main() {
    nih_plug_xtask::main()
}
```

- [ ] **Create `src/lib.rs` stub to satisfy cdylib**

```rust
use nih_plug::prelude::*;
use std::sync::Arc;

pub struct SpectralForge;

impl Plugin for SpectralForge {
    const NAME: &'static str = "Spectral Forge";
    const VENDOR: &'static str = "Kim";
    const URL: &'static str = "";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");
    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[
        AudioIOLayout {
            main_input_channels: NonZeroU32::new(2),
            main_output_channels: NonZeroU32::new(2),
            ..AudioIOLayout::const_default()
        },
    ];
    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        Arc::new(())
    }
    fn process(
        &mut self, buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers,
        _ctx: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        ProcessStatus::Normal
    }
}

impl ClapPlugin for SpectralForge {
    const CLAP_ID: &'static str = "com.spectral-forge.spectral-forge";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("Spectral compressor");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::AudioEffect, ClapFeature::Stereo,
    ];
}

nih_export_clap!(SpectralForge);
```

- [ ] **Verify build**

```bash
cargo build 2>&1 | head -20
```

Expected: compiles (may have warnings, no errors).

- [ ] **Commit**

```bash
git add Cargo.toml xtask/ src/lib.rs
git commit -m "feat: project scaffolding"
```

---

## Task 2: Guard layer with tests

**Files:**
- Create: `src/dsp/mod.rs`
- Create: `src/dsp/guard.rs`

- [ ] **Write failing tests first — add to bottom of `src/dsp/guard.rs`**

```rust
// src/dsp/guard.rs

/// Clamp NaN and Inf to 0.0 before FFT.
pub fn sanitize(buf: &mut [f32]) {
    for s in buf.iter_mut() {
        if !s.is_finite() {
            *s = 0.0;
        }
    }
}

/// Set FTZ/DAZ bits on x86_64 to flush denormals to zero.
/// No-op on other architectures.
pub fn flush_denormals() {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        use std::arch::x86_64::{
            _MM_FLUSH_ZERO_ON, _MM_SET_FLUSH_ZERO_MODE,
            _MM_DENORMALS_ZERO_ON, _MM_SET_DENORMALS_ZERO_MODE,
        };
        _MM_SET_FLUSH_ZERO_MODE(_MM_FLUSH_ZERO_ON);
        _MM_SET_DENORMALS_ZERO_MODE(_MM_DENORMALS_ZERO_ON);
    }
}

/// Returns false if SharedState is not yet initialised.
/// Guards against buggy hosts calling process() before initialize().
pub fn is_ready<T>(state: &Option<T>) -> bool {
    state.is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_clamps_nan() {
        let mut buf = [f32::NAN, 1.0, f32::INFINITY, -f32::INFINITY, 0.5];
        sanitize(&mut buf);
        assert_eq!(buf, [0.0, 1.0, 0.0, 0.0, 0.5]);
    }

    #[test]
    fn sanitize_passes_finite() {
        let mut buf = [0.0f32, 0.5, -0.5, 1.0, -1.0];
        let original = buf;
        sanitize(&mut buf);
        assert_eq!(buf, original);
    }

    #[test]
    fn is_ready_none() {
        let s: Option<u8> = None;
        assert!(!is_ready(&s));
    }

    #[test]
    fn is_ready_some() {
        let s: Option<u8> = Some(1);
        assert!(is_ready(&s));
    }
}
```

- [ ] **Create `src/dsp/mod.rs`**

```rust
pub mod guard;
pub mod pipeline;
pub mod engines;
```

(Create stub files for `pipeline.rs` and `engines/mod.rs` to satisfy `mod` declarations)

```rust
// src/dsp/pipeline.rs
// stub — implemented in Task 5
```

```rust
// src/dsp/engines/mod.rs
// stub — implemented in Task 3
```

```rust
// src/dsp/engines/README.md — see design doc §14
```

- [ ] **Add `mod dsp;` to `src/lib.rs`**

- [ ] **Run tests**

```bash
cargo test dsp::guard
```

Expected: 4 tests pass.

- [ ] **Commit**

```bash
git add src/dsp/
git commit -m "feat: guard layer with tests"
```

---

## Task 3: SpectralEngine trait + engine contract tests

**Files:**
- Create: `src/dsp/engines/mod.rs`
- Create: `tests/engine_contract.rs`

- [ ] **Write `src/dsp/engines/mod.rs`**

```rust
use num_complex::Complex;

/// Per-bin parameter values, physical units, pre-computed by pipeline.
pub struct BinParams<'a> {
    pub threshold_db: &'a [f32],  // dBFS per bin, e.g. -20.0
    pub ratio:        &'a [f32],  // ratio per bin, e.g. 4.0 = 4:1
    pub attack_ms:    &'a [f32],  // ms per bin, freq-scaled by pipeline
    pub release_ms:   &'a [f32],  // ms per bin, freq-scaled by pipeline
    pub knee_db:      &'a [f32],  // soft knee width in dB per bin
    pub makeup_db:    &'a [f32],  // makeup gain dB per bin
    pub mix:          &'a [f32],  // dry/wet per bin [0.0, 1.0]
}

pub trait SpectralEngine: Send {
    /// Called at initialize() and on sample rate / FFT size change.
    /// Pre-allocate all heap state here — never in process_bins().
    fn reset(&mut self, sample_rate: f32, fft_size: usize);

    /// Called once per STFT hop on the audio thread.
    /// Must not allocate, lock, or perform I/O.
    /// Write |gain_reduction_db| per bin into suppression_out for GUI stalactites.
    fn process_bins(
        &mut self,
        bins: &mut [Complex<f32>],
        sidechain: Option<&[f32]>,     // pre-smoothed sidechain magnitude, or None
        params: &BinParams,
        sample_rate: f32,
        suppression_out: &mut [f32],
    );

    /// Tail after silence. Override for engines with extended tails (e.g. Freeze).
    fn tail_length(&self, fft_size: usize) -> u32 {
        fft_size as u32
    }

    fn name(&self) -> &'static str;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineSelection {
    SpectralCompressor,
}

pub fn create_engine(sel: EngineSelection) -> Box<dyn SpectralEngine> {
    match sel {
        EngineSelection::SpectralCompressor => {
            Box::new(spectral_compressor::SpectralCompressorEngine::new())
        }
    }
}

pub mod spectral_compressor;
```

- [ ] **Create `src/dsp/engines/spectral_compressor.rs` stub**

```rust
use num_complex::Complex;
use super::{SpectralEngine, BinParams};

pub struct SpectralCompressorEngine;

impl SpectralCompressorEngine {
    pub fn new() -> Self { Self }
}

impl SpectralEngine for SpectralCompressorEngine {
    fn reset(&mut self, _sample_rate: f32, _fft_size: usize) {}

    fn process_bins(
        &mut self,
        _bins: &mut [Complex<f32>],
        _sidechain: Option<&[f32]>,
        _params: &BinParams,
        _sample_rate: f32,
        suppression_out: &mut [f32],
    ) {
        suppression_out.fill(0.0);
    }

    fn name(&self) -> &'static str { "Spectral Compressor" }
}
```

- [ ] **Write `tests/engine_contract.rs`**

```rust
use spectral_forge::dsp::engines::{
    BinParams, EngineSelection, SpectralEngine, create_engine,
};
use num_complex::Complex;

fn make_params(n: usize) -> (Vec<f32>, Vec<f32>, Vec<f32>, Vec<f32>, Vec<f32>, Vec<f32>, Vec<f32>) {
    (
        vec![-20.0f32; n],  // threshold_db
        vec![4.0f32; n],    // ratio
        vec![10.0f32; n],   // attack_ms
        vec![100.0f32; n],  // release_ms
        vec![6.0f32; n],    // knee_db
        vec![0.0f32; n],    // makeup_db
        vec![1.0f32; n],    // mix
    )
}

fn run_engine(engine: &mut Box<dyn SpectralEngine>, bins: &mut Vec<Complex<f32>>) {
    let n = bins.len();
    let (th, ra, at, re, kn, mk, mx) = make_params(n);
    let params = BinParams {
        threshold_db: &th, ratio: &ra, attack_ms: &at,
        release_ms: &re, knee_db: &kn, makeup_db: &mk, mix: &mx,
    };
    let mut suppression = vec![0.0f32; n];
    engine.process_bins(bins, None, &params, 44100.0, &mut suppression);
}

#[test]
fn all_zero_bins_stay_zero() {
    let mut engine = create_engine(EngineSelection::SpectralCompressor);
    engine.reset(44100.0, 2048);
    let mut bins = vec![Complex::new(0.0f32, 0.0); 1025];
    run_engine(&mut engine, &mut bins);
    for b in &bins {
        assert!(b.norm() < 1e-6, "zero bins should stay zero");
    }
}

#[test]
fn reset_callable_multiple_times() {
    let mut engine = create_engine(EngineSelection::SpectralCompressor);
    engine.reset(44100.0, 2048);
    engine.reset(48000.0, 4096);
    engine.reset(44100.0, 2048);
    // must not panic
}

#[test]
fn suppression_out_filled() {
    let mut engine = create_engine(EngineSelection::SpectralCompressor);
    engine.reset(44100.0, 2048);
    let n = 1025;
    let mut bins = vec![Complex::new(1.0f32, 0.0); n];
    let mut suppression = vec![-1.0f32; n]; // sentinel
    let (th, ra, at, re, kn, mk, mx) = make_params(n);
    let params = BinParams {
        threshold_db: &th, ratio: &ra, attack_ms: &at,
        release_ms: &re, knee_db: &kn, makeup_db: &mk, mix: &mx,
    };
    engine.process_bins(&mut bins, None, &params, 44100.0, &mut suppression);
    // All values must be >= 0 (gain reduction magnitude)
    for &s in &suppression {
        assert!(s >= 0.0, "suppression must be non-negative");
    }
}
```

- [ ] **Add `pub mod dsp;` and `pub use dsp::engines;` to `src/lib.rs` so tests can reach it**

```rust
// src/lib.rs — add at top
pub mod dsp;
```

- [ ] **Run tests**

```bash
cargo test --test engine_contract
```

Expected: all 3 pass (stub engine returns 0.0 everywhere).

- [ ] **Commit**

```bash
git add src/dsp/engines/ tests/engine_contract.rs
git commit -m "feat: SpectralEngine trait + contract tests (stub engine)"
```

---

## Task 4: Params struct

**Files:**
- Create: `src/params.rs`
- Modify: `src/lib.rs`

- [ ] **Write `src/params.rs`**

```rust
use nih_plug::prelude::*;
use nih_plug_egui::EguiState;
use parking_lot::Mutex;
use std::sync::Arc;
use crate::editor::curve::CurveNode;

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

#[derive(Params)]
pub struct SpectralForgeParams {
    #[persist = "editor_state"]
    pub editor_state: Arc<EguiState>,

    #[persist = "curve_nodes"]
    pub curve_nodes: Arc<Mutex<[[CurveNode; NUM_NODES]; NUM_CURVE_SETS]>>,

    #[persist = "active_curve"]
    pub active_curve: Arc<Mutex<usize>>,

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

    #[id = "freq_scale"]
    pub freq_scale: FloatParam,

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

    #[id = "auto_makeup"]
    pub auto_makeup: BoolParam,

    #[id = "delta_monitor"]
    pub delta_monitor: BoolParam,
}

impl SpectralForgeParams {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            editor_state: EguiState::from_size(900, 600),
            curve_nodes: Arc::new(Mutex::new(
                [crate::editor::curve::default_nodes(); NUM_CURVE_SETS]
            )),
            active_curve: Arc::new(Mutex::new(0)),

            input_gain: FloatParam::new(
                "Input Gain", 0.0,
                FloatRange::Linear { min: -18.0, max: 18.0 },
            ).with_smoother(SmoothingStyle::Logarithmic(20.0))
             .with_unit(" dB"),

            output_gain: FloatParam::new(
                "Output Gain", 0.0,
                FloatRange::Linear { min: -18.0, max: 18.0 },
            ).with_smoother(SmoothingStyle::Logarithmic(20.0))
             .with_unit(" dB"),

            mix: FloatParam::new(
                "Mix", 1.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            ).with_smoother(SmoothingStyle::Linear(10.0)),

            attack_ms: FloatParam::new(
                "Attack", 10.0,
                FloatRange::Skewed { min: 0.5, max: 200.0, factor: FloatRange::skew_factor(-2.0) },
            ).with_smoother(SmoothingStyle::Logarithmic(20.0))
             .with_unit(" ms"),

            release_ms: FloatParam::new(
                "Release", 80.0,
                FloatRange::Skewed { min: 1.0, max: 500.0, factor: FloatRange::skew_factor(-2.0) },
            ).with_smoother(SmoothingStyle::Logarithmic(20.0))
             .with_unit(" ms"),

            freq_scale: FloatParam::new(
                "Freq Scale", 0.5,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            ).with_smoother(SmoothingStyle::Linear(50.0)),

            sc_gain: FloatParam::new(
                "SC Gain", 0.0,
                FloatRange::Linear { min: -18.0, max: 18.0 },
            ).with_smoother(SmoothingStyle::Logarithmic(20.0))
             .with_unit(" dB"),

            sc_attack_ms: FloatParam::new(
                "SC Attack", 5.0,
                FloatRange::Skewed { min: 0.5, max: 100.0, factor: FloatRange::skew_factor(-2.0) },
            ).with_smoother(SmoothingStyle::Logarithmic(20.0))
             .with_unit(" ms"),

            sc_release_ms: FloatParam::new(
                "SC Release", 50.0,
                FloatRange::Skewed { min: 1.0, max: 300.0, factor: FloatRange::skew_factor(-2.0) },
            ).with_smoother(SmoothingStyle::Logarithmic(20.0))
             .with_unit(" ms"),

            lookahead_ms: FloatParam::new(
                "Lookahead", 0.0,
                FloatRange::Linear { min: 0.0, max: 10.0 },
            ).with_unit(" ms"),

            stereo_link: EnumParam::new("Stereo Link", StereoLink::Linked),
            threshold_mode: EnumParam::new("Threshold Mode", ThresholdMode::Absolute),
            auto_makeup: BoolParam::new("Auto Makeup", false),
            delta_monitor: BoolParam::new("Delta Monitor", false),
        })
    }
}
```

- [ ] **Add `pub mod editor;` stub and `pub mod params;` to `src/lib.rs`**

Create `src/editor/mod.rs`, `src/editor/curve.rs` stubs:

```rust
// src/editor/curve.rs (stub)
use serde::{Serialize, Deserialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct CurveNode {
    pub x: f32,  // [0.0, 1.0] normalised log-frequency
    pub y: f32,  // [-1.0, +1.0] normalised gain/effect
    pub q: f32,  // [0.0, 1.0] normalised octave-bandwidth
}

pub fn default_nodes() -> [CurveNode; 6] {
    [
        CurveNode { x: 0.0,  y: 0.0, q: 0.3 },
        CurveNode { x: 0.2,  y: 0.0, q: 0.5 },
        CurveNode { x: 0.4,  y: 0.0, q: 0.5 },
        CurveNode { x: 0.6,  y: 0.0, q: 0.5 },
        CurveNode { x: 0.8,  y: 0.0, q: 0.5 },
        CurveNode { x: 1.0,  y: 0.0, q: 0.3 },
    ]
}
```

```rust
// src/editor/mod.rs
pub mod curve;
pub mod theme;
pub mod spectrum_display;
pub mod suppression_display;
```

Create empty stub files: `src/editor/theme.rs`, `src/editor/spectrum_display.rs`, `src/editor/suppression_display.rs`.

- [ ] **Update `src/lib.rs` to use real params**

```rust
pub mod dsp;
pub mod editor;
pub mod params;

use nih_plug::prelude::*;
use params::SpectralForgeParams;
use std::sync::Arc;

pub struct SpectralForge {
    params: Arc<SpectralForgeParams>,
}

impl Default for SpectralForge {
    fn default() -> Self {
        Self { params: SpectralForgeParams::new() }
    }
}

impl Plugin for SpectralForge {
    const NAME: &'static str = "Spectral Forge";
    const VENDOR: &'static str = "Kim";
    const URL: &'static str = "";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");
    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[
        AudioIOLayout {
            main_input_channels: NonZeroU32::new(2),
            main_output_channels: NonZeroU32::new(2),
            ..AudioIOLayout::const_default()
        },
    ];
    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> { self.params.clone() }

    fn process(
        &mut self, _buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers,
        _ctx: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        ProcessStatus::Normal
    }
}

impl ClapPlugin for SpectralForge {
    const CLAP_ID: &'static str = "com.spectral-forge.spectral-forge";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("Spectral compressor");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::AudioEffect, ClapFeature::Stereo,
    ];
}

nih_export_clap!(SpectralForge);
```

- [ ] **Build check**

```bash
cargo build 2>&1 | grep -E "^error"
```

Expected: no errors.

- [ ] **Commit**

```bash
git add src/params.rs src/editor/ src/lib.rs
git commit -m "feat: params struct + CurveNode model"
```

---

## Task 5: Bridge (SharedState)

**Files:**
- Create: `src/bridge.rs`

- [ ] **Write `src/bridge.rs`**

```rust
use parking_lot::Mutex;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU8, Ordering},
};
use triple_buffer::{TripleBuffer, Input as TbInput, Output as TbOutput};

pub const CURVE_THRESHOLD: usize = 0;
pub const CURVE_RATIO:     usize = 1;
pub const CURVE_ATTACK:    usize = 2;
pub const CURVE_RELEASE:   usize = 3;
pub const CURVE_KNEE:      usize = 4;
pub const CURVE_MAKEUP:    usize = 5;
pub const CURVE_MIX:       usize = 6;
pub const NUM_CURVES:      usize = 7;

pub struct SharedState {
    pub num_bins: usize,

    // GUI → Audio: per-bin physical values for each parameter
    pub curve_tx: [Arc<Mutex<TbInput<Vec<f32>>>>; NUM_CURVES],
    pub curve_rx: [TbOutput<Vec<f32>>; NUM_CURVES],

    // Audio → GUI: magnitude spectrum
    pub spectrum_tx: TbInput<Vec<f32>>,
    pub spectrum_rx: Arc<Mutex<TbOutput<Vec<f32>>>>,

    // Audio → GUI: suppression per bin (|gain_reduction_db|)
    pub suppression_tx: TbInput<Vec<f32>>,
    pub suppression_rx: Arc<Mutex<TbOutput<Vec<f32>>>>,

    // Scalars
    pub sample_rate:    Arc<nih_plug::util::StutterBuffer>,  // replaced below
    pub pending_engine: Arc<AtomicU8>,
    pub sidechain_active: Arc<AtomicBool>,
}
```

Note: nih-plug re-exports `AtomicF32` as `nih_plug::util::AtomicF32`. Use that for `sample_rate`.

```rust
use parking_lot::Mutex;
use std::sync::{Arc, atomic::{AtomicBool, AtomicU8}};
use triple_buffer::{TripleBuffer, Input as TbInput, Output as TbOutput};
use nih_plug::util::StutterBuffer; // not what we want — use atomic_float or nih's AtomicF32

// nih_plug exposes AtomicF32 via:
use std::sync::atomic::AtomicU32; // use transmute trick or...
```

Actually nih-plug exports `nih_plug::util::AtomicF32`. Full correct implementation:

```rust
// src/bridge.rs
use parking_lot::Mutex;
use std::sync::{Arc, atomic::{AtomicBool, AtomicU8}};
use triple_buffer::{TripleBuffer, Input as TbInput, Output as TbOutput};

pub const NUM_CURVES: usize = 7;
pub const CURVE_THRESHOLD: usize = 0;
pub const CURVE_RATIO:     usize = 1;
pub const CURVE_ATTACK:    usize = 2;
pub const CURVE_RELEASE:   usize = 3;
pub const CURVE_KNEE:      usize = 4;
pub const CURVE_MAKEUP:    usize = 5;
pub const CURVE_MIX:       usize = 6;

pub struct SharedState {
    pub num_bins: usize,

    // GUI → Audio (one channel per parameter curve)
    pub curve_tx: Vec<Arc<Mutex<TbInput<Vec<f32>>>>>,
    pub curve_rx: Vec<TbOutput<Vec<f32>>>,

    // Audio → GUI
    pub spectrum_tx:    TbInput<Vec<f32>>,
    pub spectrum_rx:    Arc<Mutex<TbOutput<Vec<f32>>>>,
    pub suppression_tx: TbInput<Vec<f32>>,
    pub suppression_rx: Arc<Mutex<TbOutput<Vec<f32>>>>,

    // Scalars (written once at initialize, read by GUI)
    pub sample_rate:      Arc<AtomicF32>,
    pub pending_engine:   Arc<AtomicU8>,
    pub sidechain_active: Arc<AtomicBool>,
}

// Wrap f32 in an atomic using bit-casting (safe for this use case)
#[derive(Default)]
pub struct AtomicF32(std::sync::atomic::AtomicU32);

impl AtomicF32 {
    pub fn new(v: f32) -> Self { Self(std::sync::atomic::AtomicU32::new(v.to_bits())) }
    pub fn load(&self) -> f32 { f32::from_bits(self.0.load(std::sync::atomic::Ordering::Relaxed)) }
    pub fn store(&self, v: f32) { self.0.store(v.to_bits(), std::sync::atomic::Ordering::Relaxed) }
}

impl SharedState {
    pub fn new(num_bins: usize, sample_rate: f32) -> Self {
        let zero_bins = vec![0.0f32; num_bins];
        let unity_bins = vec![1.0f32; num_bins];

        let mut curve_tx = Vec::with_capacity(NUM_CURVES);
        let mut curve_rx = Vec::with_capacity(NUM_CURVES);

        // Default values per curve: threshold=-20dB, ratio=4, attack=10ms,
        // release=80ms, knee=6dB, makeup=0dB, mix=1.0
        let defaults: [f32; NUM_CURVES] = [-20.0, 4.0, 10.0, 80.0, 6.0, 0.0, 1.0];
        for i in 0..NUM_CURVES {
            let init = vec![defaults[i]; num_bins];
            let (tx, rx) = TripleBuffer::new(&init).split();
            curve_tx.push(Arc::new(Mutex::new(tx)));
            curve_rx.push(rx);
        }

        let (spectrum_tx, spectrum_rx) = TripleBuffer::new(&zero_bins).split();
        let (suppression_tx, suppression_rx) = TripleBuffer::new(&zero_bins).split();

        Self {
            num_bins,
            curve_tx,
            curve_rx,
            spectrum_tx,
            spectrum_rx: Arc::new(Mutex::new(spectrum_rx)),
            suppression_tx,
            suppression_rx: Arc::new(Mutex::new(suppression_rx)),
            sample_rate: Arc::new(AtomicF32::new(sample_rate)),
            pending_engine: Arc::new(AtomicU8::new(0)),
            sidechain_active: Arc::new(AtomicBool::new(false)),
        }
    }
}
```

- [ ] **Add `pub mod bridge;` to `src/lib.rs`**

- [ ] **Build check**

```bash
cargo build 2>&1 | grep "^error"
```

Expected: no errors.

- [ ] **Commit**

```bash
git add src/bridge.rs src/lib.rs
git commit -m "feat: bridge SharedState with 7 curve triple-buffers"
```

---

## Task 6: STFT pipeline + identity passthrough + roundtrip test

**Files:**
- Create: `src/dsp/pipeline.rs`
- Create: `tests/stft_roundtrip.rs`
- Modify: `src/lib.rs`

- [ ] **Write `tests/stft_roundtrip.rs` first**

```rust
// tests/stft_roundtrip.rs
// Verifies that identity processing (no gain change) through the full
// STFT → ISTFT pipeline preserves a sine wave with max error < 1e-3.

use approx::assert_abs_diff_eq;

#[test]
fn sine_roundtrip_identity() {
    use spectral_forge::dsp::pipeline::process_block_for_test;

    let sample_rate = 44100.0f32;
    let freq = 440.0f32;
    let n_samples = 8192usize;

    // Generate 440 Hz sine
    let input: Vec<f32> = (0..n_samples)
        .map(|i| (2.0 * std::f32::consts::PI * freq * i as f32 / sample_rate).sin())
        .collect();

    let output = process_block_for_test(&input, sample_rate);

    // Skip first 2048 samples (pipeline latency)
    let latency = 2048usize;
    for i in latency..n_samples {
        assert_abs_diff_eq!(
            output[i], input[i - latency],
            epsilon = 1e-3,
            "sample {} mismatch: got {}, expected {}",
            i, output[i], input[i - latency]
        );
    }
}
```

- [ ] **Write `src/dsp/pipeline.rs`**

```rust
use num_complex::Complex;
use realfft::RealFftPlanner;
use nih_plug::util::StftHelper;
use crate::dsp::engines::{BinParams, SpectralEngine, create_engine, EngineSelection};
use crate::bridge::SharedState;

pub const FFT_SIZE: usize = 2048;
pub const NUM_BINS: usize = FFT_SIZE / 2 + 1;
pub const OVERLAP: usize = 4;  // 75% overlap = hop of 512

pub struct Pipeline {
    stft: StftHelper,
    fft_plan: std::sync::Arc<dyn realfft::RealToComplex<f32>>,
    ifft_plan: std::sync::Arc<dyn realfft::ComplexToReal<f32>>,
    window: Vec<f32>,
    spectrum_buf: Vec<f32>,      // magnitude per bin
    suppression_buf: Vec<f32>,
    engine: Box<dyn SpectralEngine>,
    // Flat BinParams storage (pre-allocated, filled from bridge each hop)
    bp_threshold: Vec<f32>,
    bp_ratio: Vec<f32>,
    bp_attack: Vec<f32>,
    bp_release: Vec<f32>,
    bp_knee: Vec<f32>,
    bp_makeup: Vec<f32>,
    bp_mix: Vec<f32>,
    sample_rate: f32,
}

impl Pipeline {
    pub fn new(sample_rate: f32, num_channels: usize) -> Self {
        let mut planner = RealFftPlanner::<f32>::new();
        let fft_plan = planner.plan_fft_forward(FFT_SIZE);
        let ifft_plan = planner.plan_fft_inverse(FFT_SIZE);

        // Hann window
        let window: Vec<f32> = (0..FFT_SIZE)
            .map(|i| {
                0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32
                    / (FFT_SIZE - 1) as f32).cos())
            })
            .collect();

        let stft = StftHelper::new(num_channels, FFT_SIZE, 0);

        Self {
            stft,
            fft_plan,
            ifft_plan,
            window,
            spectrum_buf: vec![0.0; NUM_BINS],
            suppression_buf: vec![0.0; NUM_BINS],
            engine: create_engine(EngineSelection::SpectralCompressor),
            bp_threshold: vec![-20.0; NUM_BINS],
            bp_ratio:     vec![4.0;   NUM_BINS],
            bp_attack:    vec![10.0;  NUM_BINS],
            bp_release:   vec![80.0;  NUM_BINS],
            bp_knee:      vec![6.0;   NUM_BINS],
            bp_makeup:    vec![0.0;   NUM_BINS],
            bp_mix:       vec![1.0;   NUM_BINS],
            sample_rate,
        }
    }

    pub fn reset(&mut self, sample_rate: f32, num_channels: usize) {
        self.sample_rate = sample_rate;
        self.stft = StftHelper::new(num_channels, FFT_SIZE, 0);
        self.engine.reset(sample_rate, FFT_SIZE);
    }

    /// Process a stereo buffer in place, reading curve data from bridge.
    pub fn process(
        &mut self,
        buffer: &mut nih_plug::buffer::Buffer,
        shared: &mut SharedState,
    ) {
        // Update BinParams from latest curve data
        for (dst, src_rx) in [
            &mut self.bp_threshold,
            &mut self.bp_ratio,
            &mut self.bp_attack,
            &mut self.bp_release,
            &mut self.bp_knee,
            &mut self.bp_makeup,
            &mut self.bp_mix,
        ].iter_mut().zip(shared.curve_rx.iter_mut()) {
            let latest = src_rx.read();
            if latest.len() == dst.len() {
                dst.copy_from_slice(latest);
            }
        }

        let fft_plan = self.fft_plan.clone();
        let ifft_plan = self.ifft_plan.clone();
        let window = &self.window;
        let engine = &mut self.engine;
        let spectrum_buf = &mut self.spectrum_buf;
        let suppression_buf = &mut self.suppression_buf;
        let bp_threshold = &self.bp_threshold;
        let bp_ratio     = &self.bp_ratio;
        let bp_attack    = &self.bp_attack;
        let bp_release   = &self.bp_release;
        let bp_knee      = &self.bp_knee;
        let bp_makeup    = &self.bp_makeup;
        let bp_mix       = &self.bp_mix;
        let sample_rate  = self.sample_rate;

        // Normalisation factor for Hann window at 75% overlap
        let norm = 2.0 / (3.0 * FFT_SIZE as f32);

        self.stft.process_overlap_add(buffer, OVERLAP, |_channel, block| {
            // Apply analysis window
            for (s, &w) in block.iter_mut().zip(window.iter()) {
                *s *= w;
            }

            // Forward FFT
            let mut spectrum = fft_plan.make_output_vec();
            fft_plan.process(block, &mut spectrum).unwrap();

            // Write magnitudes to spectrum_buf (for GUI)
            for (i, c) in spectrum.iter().enumerate() {
                spectrum_buf[i] = c.norm();
            }

            // Build BinParams
            let params = BinParams {
                threshold_db: bp_threshold,
                ratio:        bp_ratio,
                attack_ms:    bp_attack,
                release_ms:   bp_release,
                knee_db:      bp_knee,
                makeup_db:    bp_makeup,
                mix:          bp_mix,
            };

            // Run engine
            engine.process_bins(&mut spectrum, None, &params, sample_rate, suppression_buf);

            // Inverse FFT
            ifft_plan.process(&mut spectrum, block).unwrap();

            // Apply synthesis window + normalise
            for (s, &w) in block.iter_mut().zip(window.iter()) {
                *s *= w * norm;
            }
        });

        // Push spectrum and suppression to GUI
        shared.spectrum_tx.write().copy_from_slice(spectrum_buf);
        shared.suppression_tx.write().copy_from_slice(suppression_buf);
    }
}

/// Test-only: run identity processing on a mono Vec<f32>, return output.
#[cfg(test)]
pub fn process_block_for_test(input: &[f32], sample_rate: f32) -> Vec<f32> {
    use nih_plug::buffer::Buffer;
    // Build a single-channel Buffer from the slice
    // (nih-plug Buffer manipulation for tests)
    let mut output = vec![0.0f32; input.len()];
    // Simplified direct STFT test without nih-plug Buffer abstraction
    let mut planner = RealFftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(FFT_SIZE);
    let ifft = planner.plan_fft_inverse(FFT_SIZE);
    let hop = FFT_SIZE / OVERLAP;
    let norm = 2.0 / (3.0 * FFT_SIZE as f32);
    let window: Vec<f32> = (0..FFT_SIZE)
        .map(|i| 0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32
            / (FFT_SIZE - 1) as f32).cos()))
        .collect();

    let padded_input = {
        let mut v = vec![0.0f32; FFT_SIZE + input.len()];
        v[FFT_SIZE..FFT_SIZE + input.len()].copy_from_slice(input);
        v
    };
    let mut accum = vec![0.0f32; FFT_SIZE + input.len()];

    let num_hops = input.len() / hop;
    for h in 0..num_hops {
        let start = h * hop;
        let mut frame: Vec<f32> = (0..FFT_SIZE)
            .map(|i| padded_input[start + i] * window[i])
            .collect();

        let mut spectrum = fft.make_output_vec();
        fft.process(&mut frame, &mut spectrum).unwrap();

        // Identity: no modification

        let mut out_frame = ifft.make_output_vec();
        ifft.process(&mut spectrum, &mut out_frame).unwrap();

        for i in 0..FFT_SIZE {
            accum[start + i] += out_frame[i] * window[i] * norm;
        }
    }

    output.copy_from_slice(&accum[FFT_SIZE..FFT_SIZE + input.len()]);
    output
}
```

- [ ] **Run roundtrip test**

```bash
cargo test --test stft_roundtrip 2>&1
```

Expected: passes (identity engine doesn't modify bins).

- [ ] **Wire pipeline into `src/lib.rs`**

Add `pipeline: Option<Pipeline>` and `shared: Option<SharedState>` fields to `SpectralForge`. Initialise in `initialize()`, call `pipeline.process()` in `process()`.

```rust
// Add to SpectralForge struct:
pipeline: Option<dsp::pipeline::Pipeline>,
shared: Option<bridge::SharedState>,

// In initialize():
fn initialize(
    &mut self,
    audio_io_layout: &AudioIOLayout,
    buffer_config: &BufferConfig,
    context: &mut impl InitContext<Self>,
) -> bool {
    let sr = buffer_config.sample_rate;
    let num_ch = audio_io_layout.main_output_channels
        .map(|c| c.get() as usize).unwrap_or(2);
    let num_bins = dsp::pipeline::FFT_SIZE / 2 + 1;
    self.shared = Some(bridge::SharedState::new(num_bins, sr));
    self.pipeline = Some(dsp::pipeline::Pipeline::new(sr, num_ch));
    context.set_latency_samples(dsp::pipeline::FFT_SIZE as u32);
    true
}

// In reset():
fn reset(&mut self) {
    // pipeline.reset() called here when sample rate changes
}

// In process():
fn process(
    &mut self, buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers,
    _ctx: &mut impl ProcessContext<Self>,
) -> ProcessStatus {
    super::dsp::guard::flush_denormals();
    for ch in buffer.iter_samples() {
        for s in ch {
            if !s.is_finite() { *s = 0.0; }
        }
    }
    if let (Some(pipeline), Some(shared)) = (&mut self.pipeline, &mut self.shared) {
        pipeline.process(buffer, shared);
    }
    ProcessStatus::Normal
}
```

- [ ] **Bundle and load in Bitwig**

```bash
cargo xtask bundle spectral_forge
mkdir -p ~/.clap
ln -sf $(pwd)/target/bundled/spectral_forge.clap ~/.clap/
```

Open Bitwig → add Spectral Forge to a track → verify audio passes through without artifacts.

- [ ] **Commit**

```bash
git add src/dsp/pipeline.rs tests/stft_roundtrip.rs src/lib.rs
git commit -m "feat: STFT passthrough pipeline + roundtrip test"
```

---

## Task 7: Theme constants

**Files:**
- Create: `src/editor/theme.rs`

- [ ] **Write `src/editor/theme.rs`**

```rust
// src/editor/theme.rs
// THE ONLY file that defines visual constants. Reskin by editing this file.

use nih_plug_egui::egui::Color32;

// Backgrounds
pub const BG:         Color32 = Color32::from_rgb(0x12, 0x12, 0x14);
pub const GRID:       Color32 = Color32::from_rgb(0x1a, 0x2a, 0x28);

// Structural lines
pub const BORDER:     Color32 = Color32::from_rgb(0x00, 0xcc, 0xbb);
pub const DIVIDER:    Color32 = Color32::from_rgb(0x00, 0x88, 0x80);

// Curve
pub const CURVE:      Color32 = Color32::from_rgb(0x00, 0xff, 0xdd);
pub const NODE_FILL:  Color32 = Color32::from_rgb(0x00, 0xcc, 0xbb);
pub const NODE_HOVER: Color32 = Color32::from_rgb(0x44, 0xff, 0xee);

// Text
pub const LABEL:      Color32 = Color32::from_rgb(0x88, 0xdd, 0xcc);
pub const LABEL_DIM:  Color32 = Color32::from_rgb(0x44, 0x88, 0x80);

// Buttons
pub const BTN_ACTIVE:   Color32 = Color32::from_rgb(0x00, 0xcc, 0xbb);
pub const BTN_INACTIVE: Color32 = Color32::from_rgb(0x22, 0x33, 0x30);
pub const BTN_TEXT_ON:  Color32 = Color32::from_rgb(0x00, 0x10, 0x0e);
pub const BTN_TEXT_OFF: Color32 = Color32::from_rgb(0x88, 0xdd, 0xcc);

// Stroke widths
pub const STROKE_THIN:   f32 = 1.0;
pub const STROKE_BORDER: f32 = 1.5;
pub const STROKE_CURVE:  f32 = 1.5;
pub const NODE_RADIUS:   f32 = 5.0;

/// Spectrum / suppression bar colour. Input: normalised magnitude [0.0, 1.0].
/// Gradient: dark blue → blue → green → yellow → red.
pub fn magnitude_color(norm: f32) -> Color32 {
    let n = norm.clamp(0.0, 1.0);
    if n < 0.25 {
        let t = n / 0.25;
        Color32::from_rgb(
            0,
            (20.0 * t) as u8,
            (80.0 + 120.0 * t) as u8,
        )
    } else if n < 0.5 {
        let t = (n - 0.25) / 0.25;
        Color32::from_rgb(
            0,
            (20.0 + 180.0 * t) as u8,
            (200.0 - 150.0 * t) as u8,
        )
    } else if n < 0.75 {
        let t = (n - 0.5) / 0.25;
        Color32::from_rgb(
            (200.0 * t) as u8,
            200,
            (50.0 - 50.0 * t) as u8,
        )
    } else {
        let t = (n - 0.75) / 0.25;
        Color32::from_rgb(
            (200.0 + 55.0 * t) as u8,
            (200.0 - 200.0 * t) as u8,
            0,
        )
    }
}
```

- [ ] **Commit**

```bash
git add src/editor/theme.rs
git commit -m "feat: 80s vector theme constants"
```

---

## Task 8: Biquad response computation + curve_sampling test

**Files:**
- Modify: `src/editor/curve.rs`
- Create: `tests/curve_sampling.rs`

- [ ] **Write `tests/curve_sampling.rs` first**

```rust
use approx::assert_abs_diff_eq;

#[test]
fn flat_curve_unity_gains() {
    use spectral_forge::editor::curve::{default_nodes, compute_curve_response};
    let nodes = default_nodes(); // all y=0.0 → 0 dB gain → linear gain = 1.0
    let num_bins = 1025;
    let sample_rate = 44100.0f32;
    let fft_size = 2048usize;
    let gains = compute_curve_response(&nodes, num_bins, sample_rate, fft_size);
    assert_eq!(gains.len(), num_bins);
    for (i, &g) in gains.iter().enumerate() {
        assert_abs_diff_eq!(g, 1.0, epsilon = 1e-4,
            "bin {} gain {} != 1.0", i, g);
    }
}

#[test]
fn full_boost_greater_than_unity() {
    use spectral_forge::editor::curve::{CurveNode, compute_curve_response};
    let mut nodes = spectral_forge::editor::curve::default_nodes();
    // Set all nodes to y=1.0 (+18 dB → linear ≈ 7.94)
    for n in &mut nodes { n.y = 1.0; }
    let gains = compute_curve_response(&nodes, 1025, 44100.0, 2048);
    for &g in &gains {
        assert!(g > 1.0, "boost should be > 1.0, got {}", g);
    }
}

#[test]
fn full_cut_less_than_unity() {
    use spectral_forge::editor::curve::{CurveNode, compute_curve_response};
    let mut nodes = spectral_forge::editor::curve::default_nodes();
    for n in &mut nodes { n.y = -1.0; }
    let gains = compute_curve_response(&nodes, 1025, 44100.0, 2048);
    for &g in &gains {
        assert!(g < 1.0, "cut should be < 1.0, got {}", g);
        assert!(g >= 0.0, "gain must be non-negative");
    }
}
```

- [ ] **Implement `compute_curve_response` in `src/editor/curve.rs`**

```rust
// src/editor/curve.rs

use serde::{Serialize, Deserialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct CurveNode {
    pub x: f32,  // [0.0, 1.0] normalised log-frequency
    pub y: f32,  // [-1.0, +1.0] gain: 0.0 = neutral
    pub q: f32,  // [0.0, 1.0] normalised octave-bandwidth
}

pub fn default_nodes() -> [CurveNode; 6] {
    [
        CurveNode { x: 0.0,  y: 0.0, q: 0.3 },
        CurveNode { x: 0.2,  y: 0.0, q: 0.5 },
        CurveNode { x: 0.4,  y: 0.0, q: 0.5 },
        CurveNode { x: 0.6,  y: 0.0, q: 0.5 },
        CurveNode { x: 0.8,  y: 0.0, q: 0.5 },
        CurveNode { x: 1.0,  y: 0.0, q: 0.3 },
    ]
}

#[derive(Clone, Copy, Debug)]
pub enum BandType { LowShelf, Bell, HighShelf }

pub fn band_type_for(index: usize) -> BandType {
    match index { 0 => BandType::LowShelf, 5 => BandType::HighShelf, _ => BandType::Bell }
}

/// Convert normalised node fields to physical units.
fn node_to_physical(node: &CurveNode) -> (f32, f32, f32) {
    let freq_hz = 20.0 * 1000.0f32.powf(node.x);
    let gain_db = node.y * 18.0;
    let bw_oct  = 0.1 * 40.0f32.powf(node.q);
    (freq_hz, gain_db, bw_oct)
}

/// RBJ biquad magnitude response |H(e^jω)| at frequency f_hz.
fn biquad_magnitude(f_hz: f32, f0: f32, gain_db: f32, bw_oct: f32,
                    band: BandType, sample_rate: f32) -> f32 {
    if gain_db.abs() < 1e-6 { return 1.0; }

    let a  = 10.0f32.powf(gain_db / 40.0); // sqrt of linear gain
    let w0 = 2.0 * std::f32::consts::PI * f0 / sample_rate;
    let bw_rads = 2.0 * std::f32::consts::PI * f0 / sample_rate
        * (2.0f32.powf(bw_oct / 2.0) - 2.0f32.powf(-bw_oct / 2.0));
    let q  = w0 / bw_rads.max(1e-6);

    // Evaluate at frequency f_hz using the bilinear transform magnitude
    let w  = 2.0 * std::f32::consts::PI * f_hz / sample_rate;

    // Use direct frequency-domain evaluation: |H(e^jω)|
    // For simplicity use the analog prototype evaluated at jΩ = j*tan(w/2)
    let omega = (w / 2.0).tan();
    let omega2 = omega * omega;

    match band {
        BandType::Bell => {
            // Peaking EQ: H(s) = (s^2 + s*(A/Q)*w0 + w0^2) / (s^2 + s*(1/(A*Q))*w0 + w0^2)
            let w02 = 1.0; // normalised to w0
            let om = omega / (w0 / sample_rate * std::f32::consts::PI); // re-normalise
            let om = (w / 2.0).tan() / ((w0 / 2.0).tan());
            let om2 = om * om;
            let num = (om2 - 1.0).powi(2) + (a * om / q).powi(2);
            let den = (om2 - 1.0).powi(2) + (om / (a * q)).powi(2);
            (num / den.max(1e-30)).sqrt()
        }
        BandType::LowShelf => {
            let om = (w / 2.0).tan() / ((w0 / 2.0).tan());
            let om2 = om * om;
            let sq_a = a.sqrt();
            let num = (a * om2 + sq_a / q * om + 1.0).powi(1);
            let den = (om2 / a + sq_a / q * om / a.sqrt() + 1.0).powi(1);
            // Simplified: use gain_db linear approximation near f0
            // Full RBJ low shelf magnitude:
            let b0 = a * ((a + 1.0) - (a - 1.0) * w0.cos() + 2.0 * a.sqrt() * w0.sin() / q);
            let b1 = 2.0 * a * ((a - 1.0) - (a + 1.0) * w0.cos());
            let b2 = a * ((a + 1.0) - (a - 1.0) * w0.cos() - 2.0 * a.sqrt() * w0.sin() / q);
            let a0 = (a + 1.0) + (a - 1.0) * w0.cos() + 2.0 * a.sqrt() * w0.sin() / q;
            let a1 = -2.0 * ((a - 1.0) + (a + 1.0) * w0.cos());
            let a2 = (a + 1.0) + (a - 1.0) * w0.cos() - 2.0 * a.sqrt() * w0.sin() / q;
            evaluate_biquad_magnitude(b0/a0, b1/a0, b2/a0, 1.0, a1/a0, a2/a0, w)
        }
        BandType::HighShelf => {
            let b0 = a * ((a + 1.0) + (a - 1.0) * w0.cos() + 2.0 * a.sqrt() * w0.sin() / q);
            let b1 = -2.0 * a * ((a - 1.0) + (a + 1.0) * w0.cos());
            let b2 = a * ((a + 1.0) + (a - 1.0) * w0.cos() - 2.0 * a.sqrt() * w0.sin() / q);
            let a0 = (a + 1.0) - (a - 1.0) * w0.cos() + 2.0 * a.sqrt() * w0.sin() / q;
            let a1 = 2.0 * ((a - 1.0) - (a + 1.0) * w0.cos());
            let a2 = (a + 1.0) - (a - 1.0) * w0.cos() - 2.0 * a.sqrt() * w0.sin() / q;
            evaluate_biquad_magnitude(b0/a0, b1/a0, b2/a0, 1.0, a1/a0, a2/a0, w)
        }
    }
}

/// Evaluate |H(e^jω)| for biquad coefficients b0,b1,b2,1,a1,a2 at radian freq w.
fn evaluate_biquad_magnitude(b0: f32, b1: f32, b2: f32, _a0: f32, a1: f32, a2: f32, w: f32) -> f32 {
    let cos_w  = w.cos();
    let cos_2w = (2.0 * w).cos();
    let num = b0*b0 + b1*b1 + b2*b2 + 2.0*(b0*b1 + b1*b2)*cos_w + 2.0*b0*b2*cos_2w;
    let den = 1.0  + a1*a1 + a2*a2 + 2.0*(a1 + a1*a2)*cos_w + 2.0*a2*cos_2w;
    (num / den.max(1e-30)).sqrt()
}

/// Compute combined biquad magnitude response for all 6 nodes at num_bins frequencies.
/// Returns a Vec<f32> of linear gain values (1.0 = unity, >1 = boost, <1 = cut).
pub fn compute_curve_response(
    nodes: &[CurveNode; 6],
    num_bins: usize,
    sample_rate: f32,
    fft_size: usize,
) -> Vec<f32> {
    let mut gains = vec![1.0f32; num_bins];

    for (i, node) in nodes.iter().enumerate() {
        if node.y.abs() < 1e-4 { continue; } // skip unity bands (optimisation)
        let (freq_hz, gain_db, bw_oct) = node_to_physical(node);
        let band = band_type_for(i);
        let q = 1.0 / (2.0 * (std::f32::consts::LN_2 / 2.0 * bw_oct).sinh());

        for k in 0..num_bins {
            let f_bin = k as f32 * sample_rate / fft_size as f32;
            let f_bin = f_bin.max(1.0); // avoid DC division issues
            let mag = biquad_magnitude(f_bin, freq_hz, gain_db, bw_oct, band, sample_rate);
            gains[k] *= mag;
        }
    }

    // Clamp to non-negative (should not be needed but guards float edge cases)
    for g in &mut gains { *g = g.max(0.0); }
    gains
}
```

- [ ] **Run curve tests**

```bash
cargo test --test curve_sampling
```

Expected: all 3 pass.

- [ ] **Commit**

```bash
git add src/editor/curve.rs tests/curve_sampling.rs
git commit -m "feat: biquad curve response + curve_sampling tests"
```

---

## Task 9: Blank GUI with parameter selector

**Files:**
- Modify: `src/editor.rs`
- Modify: `src/lib.rs`

- [ ] **Write `src/editor.rs`**

```rust
use nih_plug::prelude::*;
use nih_plug_egui::{create_egui_editor, egui, EguiState};
use std::sync::Arc;
use crate::params::{SpectralForgeParams, NUM_CURVE_SETS};
use crate::bridge::SharedState;
use crate::editor::theme as th;

const CURVE_LABELS: [&str; NUM_CURVE_SETS] =
    ["THRESHOLD", "RATIO", "ATTACK", "RELEASE", "KNEE", "MAKEUP", "MIX"];

pub fn create_editor(
    params: Arc<SpectralForgeParams>,
    shared: Arc<parking_lot::Mutex<Option<SharedState>>>,
) -> Option<Box<dyn Editor>> {
    create_egui_editor(
        params.editor_state.clone(),
        (),
        |ctx, _| {
            // Set dark background
            let mut visuals = egui::Visuals::dark();
            visuals.panel_fill = th::BG;
            ctx.set_visuals(visuals);
        },
        move |ctx, setter, _state| {
            egui::CentralPanel::default()
                .frame(egui::Frame::none().fill(th::BG))
                .show(ctx, |ui| {
                    // Parameter selector row
                    ui.horizontal(|ui| {
                        ui.add_space(4.0);
                        let mut active = *params.active_curve.lock();
                        for (i, label) in CURVE_LABELS.iter().enumerate() {
                            let is_active = active == i;
                            let (fill, text_color) = if is_active {
                                (th::BTN_ACTIVE, th::BTN_TEXT_ON)
                            } else {
                                (th::BTN_INACTIVE, th::BTN_TEXT_OFF)
                            };
                            let btn = egui::Button::new(
                                egui::RichText::new(*label)
                                    .color(text_color)
                                    .size(11.0)
                            ).fill(fill)
                             .stroke(egui::Stroke::new(th::STROKE_BORDER, th::BORDER));
                            if ui.add(btn).clicked() {
                                *params.active_curve.lock() = i;
                            }
                        }
                    });

                    ui.add_space(2.0);
                    // Divider
                    let rect = ui.available_rect_before_wrap();
                    ui.painter().line_segment(
                        [rect.left_top(), rect.right_top()],
                        egui::Stroke::new(th::STROKE_BORDER, th::BORDER),
                    );

                    // Curve area placeholder — filled in Task 10
                    let curve_rect = ui.available_rect_before_wrap();
                    ui.painter().rect_filled(curve_rect, 0.0, th::BG);
                    ui.painter().text(
                        curve_rect.center(),
                        egui::Align2::CENTER_CENTER,
                        "curve editor — coming soon",
                        egui::FontId::monospace(12.0),
                        th::LABEL_DIM,
                    );
                    ui.allocate_rect(curve_rect, egui::Sense::hover());
                });
        },
    )
}
```

- [ ] **Wire editor into `src/lib.rs`**

```rust
// Add to SpectralForge:
fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
    crate::editor::create_editor(
        self.params.clone(),
        // shared needs to be Arc-wrapped for GUI — see next step
    )
}
```

Note: `SharedState` needs to be wrapped in `Arc<Mutex<...>>` or split for GUI access. For the blank GUI, pass a dummy until Task 10 wires the bridge. Use `Arc::new(parking_lot::Mutex::new(None::<SharedState>))` as a placeholder.

- [ ] **Bundle and visually verify in Bitwig**

```bash
cargo xtask bundle spectral_forge && cp target/bundled/spectral_forge.clap ~/.clap/
```

Open plugin GUI in Bitwig. Verify:
- Dark background
- 7 turquoise-bordered buttons at top (THRESHOLD, RATIO, ATTACK, RELEASE, KNEE, MAKEUP, MIX)
- Clicking buttons highlights them
- "curve editor — coming soon" text in centre

- [ ] **Commit**

```bash
git add src/editor.rs
git commit -m "feat: GUI skeleton with parameter selector"
```

---

## Task 10: Full spectral compressor engine

**Files:**
- Modify: `src/dsp/engines/spectral_compressor.rs`

This is the core DSP. Implement after the GUI is visually working so you can hear results.

- [ ] **Update engine contract tests to verify compression actually occurs**

Add to `tests/engine_contract.rs`:

```rust
#[test]
fn loud_signal_gets_compressed() {
    let mut engine = create_engine(EngineSelection::SpectralCompressor);
    engine.reset(44100.0, 2048);
    let n = 1025;
    // Signal at -6 dBFS linear ≈ 0.5 amplitude
    let input_mag = 0.5f32;
    let mut bins: Vec<Complex<f32>> = (0..n)
        .map(|_| Complex::new(input_mag, 0.0))
        .collect();

    let threshold = vec![-20.0f32; n]; // -20 dBFS — signal is above threshold
    let ratio     = vec![4.0f32; n];
    let attack    = vec![0.1f32; n];   // very fast attack
    let release   = vec![100.0f32; n];
    let knee      = vec![0.0f32; n];   // hard knee
    let makeup    = vec![0.0f32; n];
    let mix       = vec![1.0f32; n];

    let params = BinParams {
        threshold_db: &threshold, ratio: &ratio,
        attack_ms: &attack, release_ms: &release,
        knee_db: &knee, makeup_db: &makeup, mix: &mix,
    };
    let mut suppression = vec![0.0f32; n];

    // Run several hops to let envelope follower converge
    for _ in 0..200 {
        engine.process_bins(&mut bins.clone(), None, &params, 44100.0, &mut suppression);
    }
    // After convergence, output magnitude should be less than input
    let mut final_bins = bins.clone();
    engine.process_bins(&mut final_bins, None, &params, 44100.0, &mut suppression);
    let output_mag = final_bins[512].norm(); // mid-band bin
    assert!(output_mag < input_mag,
        "compression should reduce level: {} >= {}", output_mag, input_mag);
}
```

- [ ] **Run test — verify it fails**

```bash
cargo test --test engine_contract loud_signal
```

Expected: FAIL (stub doesn't compress).

- [ ] **Implement full compressor engine**

```rust
// src/dsp/engines/spectral_compressor.rs

use num_complex::Complex;
use super::{SpectralEngine, BinParams};

pub struct SpectralCompressorEngine {
    /// Per-bin envelope state (smoothed dBFS level tracking)
    env_db: Vec<f32>,
    num_bins: usize,
    sample_rate: f32,
    fft_size: usize,
    hop_size: usize,
}

impl SpectralCompressorEngine {
    pub fn new() -> Self {
        Self {
            env_db: Vec::new(),
            num_bins: 0,
            sample_rate: 44100.0,
            fft_size: 2048,
            hop_size: 512,
        }
    }

    /// Soft-knee gain computer. Returns gain reduction in dB (≤ 0).
    #[inline]
    fn gain_computer(level_db: f32, threshold_db: f32, ratio: f32, knee_db: f32) -> f32 {
        let overshoot = level_db - threshold_db;
        if knee_db < 0.001 {
            // Hard knee
            if overshoot <= 0.0 { 0.0 }
            else { overshoot * (1.0 / ratio - 1.0) }
        } else {
            // Soft knee (quadratic)
            let half_knee = knee_db / 2.0;
            if overshoot <= -half_knee {
                0.0
            } else if overshoot <= half_knee {
                (overshoot + half_knee).powi(2) / (2.0 * knee_db) * (1.0 / ratio - 1.0)
            } else {
                overshoot * (1.0 / ratio - 1.0)
            }
        }
    }

    /// Convert milliseconds to per-hop coefficient for one-pole filter.
    #[inline]
    fn ms_to_coeff(ms: f32, sample_rate: f32, hop_size: usize) -> f32 {
        if ms < 0.001 { return 0.0; }
        let hops_per_sec = sample_rate / hop_size as f32;
        let time_hops = ms * 0.001 * hops_per_sec;
        (-1.0 / time_hops).exp()
    }
}

impl SpectralEngine for SpectralCompressorEngine {
    fn reset(&mut self, sample_rate: f32, fft_size: usize) {
        self.sample_rate = sample_rate;
        self.fft_size = fft_size;
        self.hop_size = fft_size / 4; // 75% overlap
        let num_bins = fft_size / 2 + 1;
        self.num_bins = num_bins;
        self.env_db = vec![-96.0f32; num_bins]; // initialise to silence
    }

    fn process_bins(
        &mut self,
        bins: &mut [Complex<f32>],
        sidechain: Option<&[f32]>,
        params: &BinParams,
        sample_rate: f32,
        suppression_out: &mut [f32],
    ) {
        debug_assert_eq!(bins.len(), self.num_bins);
        let hop = self.hop_size;

        for k in 0..bins.len().min(self.num_bins) {
            // 1. Detect level (use sidechain magnitude if provided)
            let level_linear = match sidechain {
                Some(sc) => sc.get(k).copied().unwrap_or(0.0),
                None => bins[k].norm(),
            };
            let level_db = if level_linear > 1e-10 {
                20.0 * level_linear.log10()
            } else {
                -96.0
            };

            // 2. Envelope follower (one-pole, per-hop)
            let threshold_db = params.threshold_db[k];
            let attack_ms    = params.attack_ms[k].max(0.1);
            let release_ms   = params.release_ms[k].max(1.0);

            let coeff = if level_db > self.env_db[k] {
                Self::ms_to_coeff(attack_ms, sample_rate, hop)
            } else {
                Self::ms_to_coeff(release_ms, sample_rate, hop)
            };
            self.env_db[k] = coeff * self.env_db[k] + (1.0 - coeff) * level_db;

            // 3. Gain computer
            let ratio  = params.ratio[k].max(1.0);
            let knee   = params.knee_db[k].max(0.0);
            let gr_db  = Self::gain_computer(self.env_db[k], threshold_db, ratio, knee);
            // gr_db ≤ 0

            // 4. Makeup gain
            let total_db = gr_db + params.makeup_db[k];
            let linear_gain = 10.0f32.powf(total_db / 20.0);

            // 5. Apply with per-bin dry/wet
            let mix = params.mix[k].clamp(0.0, 1.0);
            let dry = bins[k];
            bins[k] = dry * (1.0 - mix + mix * linear_gain);

            // 6. Write suppression for GUI stalactites (positive dB value)
            suppression_out[k] = (-gr_db).max(0.0);
        }
    }

    fn name(&self) -> &'static str { "Spectral Compressor" }
}
```

- [ ] **Run all engine contract tests**

```bash
cargo test --test engine_contract
```

Expected: all 4 pass including the compression test.

- [ ] **Commit**

```bash
git add src/dsp/engines/spectral_compressor.rs tests/engine_contract.rs
git commit -m "feat: spectral compressor engine with envelope follower"
```

---

## Task 11: Wire curves to engine + frequency scaling

**Files:**
- Modify: `src/dsp/pipeline.rs`

- [ ] **Add physical-unit mapping to pipeline's BinParams assembly**

In `Pipeline::process()`, after reading curve channels, convert normalised curve values to physical units. Update the BinParams assembly section:

```rust
// In Pipeline::process(), replace the BinParams block:

// curve_rx[CURVE_THRESHOLD] carries linear-gain values from biquad.
// Map them back to dBFS for the engine:
// threshold: y=0 → -20 dBFS default; full range -60 to 0 dBFS
// The curve response is a multiplier on the base threshold.
// Simple mapping: response * base_threshold_db (both in same range).
// Use: threshold_db[k] = -60.0 + threshold_curve[k] * 60.0
// (curve gain 0.0→-60dB, 1.0→0dBFS, default ~0.33→-20dB)
for k in 0..num_bins {
    let t = threshold_curve[k].clamp(0.0, 1.0);
    self.bp_threshold[k] = -60.0 + t * 60.0;

    let r = ratio_curve[k].clamp(1.0, 20.0);
    self.bp_ratio[k] = r;

    // Frequency-dependent timing
    let f_bin = (k as f32 * sample_rate / FFT_SIZE as f32).max(20.0);
    let scale = (1000.0 / f_bin).powf(freq_scale);
    self.bp_attack[k]  = (attack_ms_base * scale).clamp(0.1, 500.0);
    self.bp_release[k] = (release_ms_base * scale).clamp(1.0, 2000.0);

    let kn = knee_curve[k].clamp(0.0, 24.0);
    self.bp_knee[k] = kn;

    let mk = makeup_curve[k]; // linear gain from curve
    self.bp_makeup[k] = if mk > 1e-6 { 20.0 * mk.log10() } else { -96.0 };

    self.bp_mix[k] = mix_curve[k].clamp(0.0, 1.0);
}
```

Note: `ratio_curve` from the engine uses the biquad response as a multiplier (1.0–20.0 range). The curve's y=0 node gives gain=1.0 (no ratio), y=+1 gives gain up to ~8.0 (high ratio). The ratio pipeline receives it as the curve's linear gain output.

For the threshold curve, map `curve_response[k]` (which is around 1.0 for flat curve) to a dBFS threshold. Store the base threshold as a pipeline parameter (from `params.threshold_base` if added, or hardcode -20 dBFS for now).

- [ ] **Pull global knob values into pipeline**

```rust
// In Pipeline::process(), read from nih_plug params via shared atomics:
let attack_ms_base  = // read from shared.attack_ms AtomicF32 or pass via process() args
let release_ms_base = // same
let freq_scale      = // same
```

The cleanest approach: pass `&SpectralForgeParams` reference into `Pipeline::process()`. The params struct contains `SmoothedValue` fields; call `.next()` once per block to get the smoothed value.

Update `Pipeline::process()` signature:

```rust
pub fn process(
    &mut self,
    buffer: &mut nih_plug::buffer::Buffer,
    shared: &mut SharedState,
    params: &SpectralForgeParams,
)
```

- [ ] **Manual test in Bitwig**

Load pink noise → Spectral Forge. Pull the THRESHOLD curve down at 1 kHz. Verify audible dip in that frequency range. Open Bitwig's spectrum analyser on the output to confirm.

- [ ] **Commit**

```bash
git add src/dsp/pipeline.rs
git commit -m "feat: wire parameter curves to engine with frequency scaling"
```

---

## Task 12: EQ curve widget (interactive GUI)

**Files:**
- Modify: `src/editor/curve.rs`
- Modify: `src/editor.rs`

- [ ] **Add egui widget to `src/editor/curve.rs`**

```rust
use nih_plug_egui::egui::{self, Rect, Response, Sense, Ui, Vec2, Pos2, Stroke, Shape};
use crate::editor::theme as th;

/// Converts normalised x [0,1] → screen x pixel within rect.
fn x_to_screen(x: f32, rect: Rect) -> f32 {
    rect.left() + x * rect.width()
}

/// Converts screen x → normalised x.
fn screen_to_x(px: f32, rect: Rect) -> f32 {
    ((px - rect.left()) / rect.width()).clamp(0.0, 1.0)
}

/// Converts normalised y [-1, +1] → screen y (top = +1, bottom = -1).
fn y_to_screen(y: f32, rect: Rect) -> f32 {
    rect.top() + (1.0 - (y + 1.0) / 2.0) * rect.height()
}

fn screen_to_y(py: f32, rect: Rect) -> f32 {
    let norm = 1.0 - (py - rect.top()) / rect.height();
    (norm * 2.0 - 1.0).clamp(-1.0, 1.0)
}

/// Draw the 6-node EQ curve and handle drag/scroll interaction.
/// Returns true if any node was changed (trigger curve recompute).
pub fn curve_widget(
    ui: &mut Ui,
    rect: Rect,
    nodes: &mut [CurveNode; 6],
) -> bool {
    let mut changed = false;

    // Draw 0 dB centre line
    let centre_y = y_to_screen(0.0, rect);
    ui.painter().line_segment(
        [Pos2::new(rect.left(), centre_y), Pos2::new(rect.right(), centre_y)],
        Stroke::new(th::STROKE_THIN, th::GRID),
    );

    // Draw node handles and handle interaction
    for i in 0..6 {
        let sx = x_to_screen(nodes[i].x, rect);
        let sy = y_to_screen(nodes[i].y, rect);
        let node_pos = Pos2::new(sx, sy);

        // Hit area
        let node_rect = Rect::from_center_size(node_pos, Vec2::splat(th::NODE_RADIUS * 3.0));
        let resp = ui.interact(node_rect, ui.id().with(("node", i)), Sense::drag());

        // Drag
        if resp.dragged() {
            let delta = resp.drag_delta();
            nodes[i].x = (nodes[i].x + delta.x / rect.width()).clamp(0.0, 1.0);
            nodes[i].y = (nodes[i].y - delta.y / rect.height() * 2.0).clamp(-1.0, 1.0);
            changed = true;
        }

        // Scroll → adjust q
        let scroll = ui.input(|inp| {
            if node_rect.contains(inp.pointer.hover_pos().unwrap_or(Pos2::ZERO)) {
                inp.scroll_delta.y
            } else { 0.0 }
        });
        if scroll.abs() > 0.01 {
            nodes[i].q = (nodes[i].q + scroll * 0.01).clamp(0.0, 1.0);
            changed = true;
        }

        // Double-click → reset
        if resp.double_clicked() {
            let defaults = default_nodes();
            nodes[i] = defaults[i];
            changed = true;
        }

        // Draw node
        let color = if resp.hovered() { th::NODE_HOVER } else { th::NODE_FILL };
        ui.painter().circle_filled(node_pos, th::NODE_RADIUS, color);
        ui.painter().circle_stroke(node_pos, th::NODE_RADIUS,
            Stroke::new(th::STROKE_BORDER, th::BORDER));
    }

    changed
}

/// Paint the combined gain response curve (from pre-computed gains Vec).
pub fn paint_response_curve(ui: &Ui, rect: Rect, gains: &[f32]) {
    if gains.is_empty() { return; }
    let n = gains.len();
    // Map log-frequency bin index to screen x; map dB to screen y
    // gains are linear; convert to dB for display
    let db_range = 18.0f32; // ±18 dB display range
    let points: Vec<Pos2> = (0..n)
        .map(|k| {
            let x_norm = k as f32 / (n - 1) as f32;
            let db = if gains[k] > 1e-6 { 20.0 * gains[k].log10() } else { -db_range };
            let y_norm = (db / db_range).clamp(-1.0, 1.0);
            Pos2::new(x_to_screen(x_norm, rect), y_to_screen(y_norm, rect))
        })
        .collect();

    ui.painter().add(Shape::line(points, Stroke::new(th::STROKE_CURVE, th::CURVE)));
}
```

- [ ] **Replace placeholder in `editor.rs` with real curve widget**

```rust
// In editor.rs, replace the "coming soon" section:

use crate::editor::curve::{compute_curve_response, curve_widget, paint_response_curve};

// In the closure:
let curve_rect = ui.available_rect_before_wrap();
ui.allocate_rect(curve_rect, Sense::hover());

// Read + edit nodes for the active curve
let active_idx = *params.active_curve.lock();
let mut nodes = params.curve_nodes.lock()[active_idx];
let sample_rate = {
    if let Some(shared) = shared.lock().as_ref() {
        shared.sample_rate.load()
    } else { 44100.0 }
};

// Compute display gains
let display_gains = compute_curve_response(&nodes, 512, sample_rate, 2048);

// Paint response curve
paint_response_curve(ui, curve_rect, &display_gains);

// Handle node interaction
if curve_widget(ui, curve_rect, &mut nodes) {
    params.curve_nodes.lock()[active_idx] = nodes;
    // Push updated curve to bridge
    if let Some(shared) = shared.lock().as_ref() {
        let full_gains = compute_curve_response(&nodes, shared.num_bins, sample_rate, 2048);
        if let Ok(mut tx) = shared.curve_tx[active_idx].try_lock() {
            *tx.input_buffer() = full_gains;
            tx.publish();
        }
    }
}
```

- [ ] **Bundle + visual test in Bitwig**

```bash
cargo xtask bundle spectral_forge && cp target/bundled/spectral_forge.clap ~/.clap/
```

Verify:
- 6 nodes visible with turquoise circles
- Dragging nodes moves them
- Curve line draws correctly through node positions
- Switching parameter buttons shows/restores different curves

- [ ] **Commit**

```bash
git add src/editor/curve.rs src/editor.rs
git commit -m "feat: interactive EQ curve widget"
```

---

## Task 13: Spectrum + suppression display

**Files:**
- Create: `src/editor/spectrum_display.rs`
- Create: `src/editor/suppression_display.rs`
- Modify: `src/editor.rs`

- [ ] **Write `src/editor/spectrum_display.rs`**

```rust
use nih_plug_egui::egui::{self, Rect, Painter, Pos2};
use crate::editor::theme as th;

/// Paint log-scaled magnitude spectrum bars behind the curve.
/// `magnitudes`: linear magnitude per bin (num_bins values).
pub fn paint_spectrum(painter: &Painter, rect: Rect, magnitudes: &[f32]) {
    if magnitudes.is_empty() { return; }
    let n = magnitudes.len();
    let bar_width = rect.width() / n as f32;

    // Find peak for normalisation (rolling max)
    let peak = magnitudes.iter().cloned().fold(1e-10f32, f32::max);

    for k in 0..n {
        let x_norm = k as f32 / n as f32;
        let x = rect.left() + x_norm * rect.width();
        let mag_norm = (magnitudes[k] / peak).clamp(0.0, 1.0);
        // Log scaling for visual
        let height_norm = if mag_norm > 1e-6 {
            (1.0 + 20.0 * mag_norm.log10() / 60.0).clamp(0.0, 1.0)
        } else { 0.0 };
        let bar_height = height_norm * rect.height();
        let top = rect.bottom() - bar_height;
        let color = th::magnitude_color(height_norm);
        painter.rect_filled(
            egui::Rect::from_min_max(
                Pos2::new(x, top),
                Pos2::new(x + bar_width.max(1.0), rect.bottom()),
            ),
            0.0,
            color,
        );
    }
}
```

- [ ] **Write `src/editor/suppression_display.rs`**

```rust
use nih_plug_egui::egui::{self, Rect, Painter, Pos2};
use crate::editor::theme as th;

/// Paint stalactite suppression bars hanging from the top.
/// `suppression`: gain reduction magnitude in dB per bin (>= 0).
pub fn paint_suppression(painter: &Painter, rect: Rect, suppression: &[f32]) {
    if suppression.is_empty() { return; }
    let n = suppression.len();
    let bar_width = rect.width() / n as f32;
    let max_db = 24.0f32; // full bar = 24 dB reduction

    for k in 0..n {
        let x_norm = k as f32 / n as f32;
        let x = rect.left() + x_norm * rect.width();
        let depth_norm = (suppression[k] / max_db).clamp(0.0, 1.0);
        if depth_norm < 0.001 { continue; }
        let bar_height = depth_norm * rect.height() * 0.3; // max 30% of height
        let color = th::magnitude_color(depth_norm);
        painter.rect_filled(
            egui::Rect::from_min_max(
                Pos2::new(x, rect.top()),
                Pos2::new(x + bar_width.max(1.0), rect.top() + bar_height),
            ),
            0.0,
            color,
        );
    }
}
```

- [ ] **Wire spectrum + suppression reads into `editor.rs`**

In the editor closure, before painting the curve, read from bridge and paint bars:

```rust
use crate::editor::{spectrum_display, suppression_display};

if let Some(sh) = shared.lock().as_ref() {
    // Spectrum bars (background)
    if let Ok(mut rx) = sh.spectrum_rx.try_lock() {
        let mags = rx.read().clone();
        spectrum_display::paint_spectrum(ui.painter(), curve_rect, &mags);
    }
    // Suppression stalactites (top)
    if let Ok(mut rx) = sh.suppression_rx.try_lock() {
        let supp = rx.read().clone();
        suppression_display::paint_suppression(ui.painter(), curve_rect, &supp);
    }
}
```

- [ ] **Bundle + verify in Bitwig**

Play audio through the plugin. Verify:
- Coloured bars animate in the background (dark blue → red depending on frequency energy)
- When compression occurs, thin stalactites appear at the top
- Curve overlay paints on top of both

- [ ] **Commit**

```bash
git add src/editor/spectrum_display.rs src/editor/suppression_display.rs src/editor.rs
git commit -m "feat: spectrum bars + suppression stalactites"
```

---

## Task 14: Bin-linking

**Files:**
- Modify: `src/dsp/engines/spectral_compressor.rs`

Bin-linking smooths the gain reduction across adjacent bins to avoid narrow-band notching artifacts.

- [ ] **Add to `SpectralCompressorEngine` struct**

```rust
gr_smoothed: Vec<f32>,  // smoothed gain reduction per bin (dB, ≤ 0)
```

Initialise to `vec![0.0f32; num_bins]` in `reset()`.

- [ ] **Add bin-linking pass after gain computation**

In `process_bins()`, separate the gain computation from the application into two passes:

```rust
// Pass 1: compute raw gain reduction per bin into gr_smoothed
for k in 0..bins.len() {
    // ... (existing gain computer code, write to self.gr_smoothed[k])
}

// Pass 2: smooth gain reduction across neighbours (Gaussian-ish with width ~3 bins)
let mut smoothed = vec![0.0f32; self.num_bins];
for k in 0..self.num_bins {
    let w0 = 0.5;
    let w1 = 0.25;
    let prev = if k > 0 { self.gr_smoothed[k-1] } else { self.gr_smoothed[k] };
    let next = if k + 1 < self.num_bins { self.gr_smoothed[k+1] } else { self.gr_smoothed[k] };
    smoothed[k] = w0 * self.gr_smoothed[k] + w1 * prev + w1 * next;
}

// Pass 3: apply smoothed gain
for k in 0..bins.len() {
    let total_db = smoothed[k] + params.makeup_db[k];
    let linear_gain = 10.0f32.powf(total_db / 20.0);
    let mix = params.mix[k].clamp(0.0, 1.0);
    let dry = bins[k];
    bins[k] = dry * (1.0 - mix + mix * linear_gain);
    suppression_out[k] = (-smoothed[k]).max(0.0);
}
```

Note: the `smoothed` Vec allocation violates I-1. Pre-allocate as `smooth_buf: Vec<f32>` in `reset()` and reuse.

- [ ] **Run engine contract tests**

```bash
cargo test --test engine_contract
```

Expected: all pass (bin-linking doesn't change zero-in/zero-out behaviour).

- [ ] **Commit**

```bash
git add src/dsp/engines/spectral_compressor.rs
git commit -m "feat: bin-linking smooths gain reduction across adjacent bins"
```

---

## Task 15: Relative threshold mode

**Files:**
- Modify: `src/dsp/engines/spectral_compressor.rs`
- Modify: `src/dsp/pipeline.rs`

- [ ] **Add median envelope state to engine**

```rust
spectral_envelope: Vec<f32>,  // smoothed local median magnitude per bin
```

Initialise in `reset()`.

- [ ] **Add relative mode processing**

Add a `relative_mode: bool` flag to `process_bins` — pass it via a new field on `BinParams` or via a separate method. Simplest: add to `BinParams`:

```rust
pub relative_mode: bool,
```

In `process_bins`, before computing level_db:

```rust
// Relative mode: compute local median envelope across neighbouring bins
if params.relative_mode {
    // Use a simple 3-bin median (extend to wider window for "sharpness" knob later)
    for k in 0..self.num_bins {
        let lo = if k > 0 { magnitudes[k-1] } else { magnitudes[k] };
        let hi = if k+1 < self.num_bins { magnitudes[k+1] } else { magnitudes[k] };
        let med = {
            let mut arr = [lo, magnitudes[k], hi];
            arr.sort_by(|a, b| a.partial_cmp(b).unwrap());
            arr[1]
        };
        // Smooth envelope over time
        let env_coeff = Self::ms_to_coeff(50.0, sample_rate, hop);
        self.spectral_envelope[k] = env_coeff * self.spectral_envelope[k]
            + (1.0 - env_coeff) * med;
    }
    // level_db = level relative to envelope
    // (already handled per-bin: divide before log)
}
```

Then in the per-bin loop, when `relative_mode`:

```rust
let level_linear = bins[k].norm();
let detection_linear = if params.relative_mode && self.spectral_envelope[k] > 1e-10 {
    level_linear / self.spectral_envelope[k]
} else {
    level_linear
};
let level_db = if detection_linear > 1e-10 {
    20.0 * detection_linear.log10()
} else { -96.0 };
```

- [ ] **Wire `threshold_mode` param into pipeline → BinParams**

In `pipeline.rs`, read `params.threshold_mode.value()` and set `BinParams::relative_mode`.

- [ ] **Manual test: absolute vs relative**

In Bitwig: play a signal with a prominent resonance at 2 kHz. Toggle threshold mode. In relative mode, only the resonance should be attenuated (it sticks out above neighbours); in absolute mode, any bin above the threshold gets compressed regardless.

- [ ] **Commit**

```bash
git add src/dsp/engines/spectral_compressor.rs src/dsp/pipeline.rs
git commit -m "feat: relative threshold mode with median envelope detection"
```

---

## Task 16: Sidechain input

**Files:**
- Modify: `src/lib.rs`
- Modify: `src/dsp/pipeline.rs`

- [ ] **Add sidechain audio layout to `src/lib.rs`**

```rust
const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[
    // Stereo with sidechain
    AudioIOLayout {
        main_input_channels: NonZeroU32::new(2),
        main_output_channels: NonZeroU32::new(2),
        aux_input_ports: &[new_nonzero_u32!(2)],
        ..AudioIOLayout::const_default()
    },
    // Stereo without sidechain
    AudioIOLayout {
        main_input_channels: NonZeroU32::new(2),
        main_output_channels: NonZeroU32::new(2),
        ..AudioIOLayout::const_default()
    },
];
```

- [ ] **Add sidechain STFT state to `Pipeline`**

```rust
sc_stft: StftHelper,
sc_envelope: Vec<f32>,   // smoothed sidechain magnitude per bin
sc_env_state: Vec<f32>,  // one-pole state
```

- [ ] **Process sidechain in `pipeline.rs`**

In `Pipeline::process()`, accept `aux: &mut AuxiliaryBuffers` and extract sidechain:

```rust
pub fn process(
    &mut self,
    buffer: &mut nih_plug::buffer::Buffer,
    aux: &mut AuxiliaryBuffers,
    shared: &mut SharedState,
    params: &SpectralForgeParams,
)
```

Before the main STFT loop:

```rust
// Process sidechain if present
let sc_active = !aux.inputs.is_empty();
shared.sidechain_active.store(sc_active, Ordering::Relaxed);

if sc_active {
    let sc_buf = &mut aux.inputs[0];
    // sanitize
    // run sc_stft.process_overlap_add to compute per-bin magnitudes
    // envelope-follow with sc_attack_ms / sc_release_ms
    // store result in self.sc_envelope
}
```

Pass `Some(&self.sc_envelope)` vs `None` to `engine.process_bins()`.

- [ ] **Bundle + test sidechain in Bitwig**

- Route a separate signal to the sidechain input
- Enable sidechain in Bitwig plugin settings
- Verify stalactites respond to the sidechain signal instead of the main signal

- [ ] **Commit**

```bash
git add src/lib.rs src/dsp/pipeline.rs
git commit -m "feat: sidechain STFT path with envelope follower"
```

---

## Task 17: Stereo link + M/S

**Files:**
- Modify: `src/dsp/pipeline.rs`

- [ ] **Implement stereo link modes**

After computing gain reduction per channel (run engine on both channels), in Linked mode take the max GR per bin across channels and apply to both. In M/S mode, convert L/R to M/S before the STFT and back after.

For the `SpectralCompressorEngine`, the engine already tracks per-bin state. For stereo link, run both channels through the engine independently to get GR values, then override each bin's GR with `max(gr_L[k], gr_R[k])` before applying.

```rust
// In pipeline.rs process() with stereo link:
match stereo_link_mode {
    StereoLink::Independent => {
        // process L and R independently (current behaviour)
    }
    StereoLink::Linked => {
        // run engine on both, collect GR per bin, max, re-apply
        // Requires engine to expose a "compute_gr_only" path, or
        // store the last GR values in engine state and post-correct.
    }
    StereoLink::MidSide => {
        // M = (L + R) / sqrt(2), S = (L - R) / sqrt(2) before FFT
        // process M and S independently
        // convert back after ISTFT
    }
}
```

- [ ] **Commit**

```bash
git add src/dsp/pipeline.rs
git commit -m "feat: stereo link and M/S processing modes"
```

---

## Task 18: Lookahead + auto-makeup + delta monitor

**Files:**
- Modify: `src/dsp/pipeline.rs`

- [ ] **Lookahead**

Add a `lookahead_buf: Vec<f32>` ring buffer to `Pipeline`. Pre-delay the main signal by `lookahead_samples` before the STFT. Update `context.set_latency_samples(FFT_SIZE + lookahead_samples)` in `initialize()`.

```rust
// In Pipeline::process(), before STFT:
let lookahead_samples = (params.lookahead_ms.value() * 0.001 * self.sample_rate) as usize;
// Write current input to ring buffer, read delayed version for processing.
```

- [ ] **Auto-makeup**

Track a long-term average gain reduction per bin (1-second smoothing). When `auto_makeup` is enabled, add the negative of that average to `BinParams::makeup_db` per bin, compensating for average level reduction.

```rust
// In engine, separately track:
auto_makeup_db: Vec<f32>,  // long-term average GR, smoothed

// Update per hop:
let coeff_slow = Self::ms_to_coeff(1000.0, sample_rate, hop);
self.auto_makeup_db[k] = coeff_slow * self.auto_makeup_db[k]
    + (1.0 - coeff_slow) * gr_db;
// In BinParams: if auto_makeup enabled, makeup_db[k] += -self.auto_makeup_db[k]
```

- [ ] **Delta monitor**

After `Pipeline::process()` computes the wet output, if `delta_monitor` is enabled, subtract wet from the delayed dry signal:

```rust
if params.delta_monitor.value() {
    for (dry, wet) in dry_delayed.iter().zip(buffer.iter_mut()) {
        *wet = dry - *wet;
    }
}
```

- [ ] **Bundle + test in Bitwig**

- Lookahead: verify latency increases by lookahead amount (Bitwig ADC should compensate)
- Auto-makeup: verify perceived loudness stays roughly constant when toggled
- Delta monitor: verify you hear only the compressed material (what's being removed)

- [ ] **Commit**

```bash
git add src/dsp/pipeline.rs
git commit -m "feat: lookahead, auto-makeup, delta monitor"
```

---

## Task 19: Bottom control strip + persist + CLAP polish

**Files:**
- Modify: `src/editor.rs`
- Modify: `src/lib.rs`

- [ ] **Add control strip to GUI bottom**

Below the curve area, add a row of labelled knobs/toggles using nih-plug's `ParamSlider` or egui's `DragValue` widgets for: INPUT, OUTPUT, ATTACK, RELEASE, FREQ SCALE, SC GAIN, SC ATK, SC REL, LOOKAHEAD, STEREO LINK, THRESHOLD MODE, AUTO MAKEUP, DELTA.

```rust
// In editor.rs, after curve area:
ui.separator();
ui.horizontal(|ui| {
    use nih_plug_egui::widgets::ParamSlider;
    ui.add(ParamSlider::for_param(&params.attack_ms, setter).with_width(60.0));
    ui.label(egui::RichText::new("ATK").color(th::LABEL).size(10.0));
    // repeat for other params
});
```

- [ ] **Verify persist on session save/reload**

Save a Bitwig project with custom curves → close → reopen → verify all 7 curves are restored. This works automatically via `#[persist]` as long as the `CurveNode` struct derives `Serialize`/`Deserialize`.

- [ ] **Final CLAP compliance check**

- `Plugin::latency_samples()` returns correct value (FFT_SIZE + lookahead)
- `Plugin::tail_length()` returns `FFT_SIZE` samples
- `Plugin::reset()` flushes STFT overlap buffers
- Reported latency matches actual (test with Bitwig's ADC by routing parallel dry/wet)

- [ ] **Release build + install**

```bash
cargo xtask bundle spectral_forge --release
ln -sf $(pwd)/target/bundled/spectral_forge.clap ~/.clap/spectral_forge.clap
```

- [ ] **Commit**

```bash
git add src/editor.rs src/lib.rs
git commit -m "feat: control strip, persist, CLAP latency/tail polish"
```

---

## Final Integration Checklist

- [ ] `cargo test --all` — all tests pass
- [ ] `cargo build --release` — no errors, no warnings
- [ ] Pink noise → flat curves → output matches input (verify with analyser)
- [ ] Audio-rate modulate `attack_ms` in Bitwig → no clicks or crashes
- [ ] Sidechain input connected → stalactites react to sidechain, not main signal
- [ ] Delta monitor → hear only removed material
- [ ] Save/close/reopen Bitwig project → all 7 curves restored
- [ ] Transport restart → no glitch (reset() works)
- [ ] `assert_process_allocs` feature: no allocation errors in dev build during playback
