# Technical Specification: Modular Rust Spectral Processor Plugin

> **STATUS: SUPERSEDED (historical).** This is the original 0.1.0 design spec,
> written before the modular architecture. The single-engine pipeline, single
> Catmull-Rom curve, top-level `editor.rs`, and CLAP-only / Linux-only
> targeting it describes were replaced in the 0.15 cycle by the modular
> `FxMatrix` (9 slots of typed `SpectralModule`s wired through a `RouteMatrix`,
> up to 7 curves per slot), exported as CLAP + VST3 for Linux and Windows.
> `SpectralEngine` survives only as an internal engine inside the Dynamics and
> Contrast modules, and invariants I-3, I-4 and I-6 no longer hold. Do not
> follow this spec; see `CLAUDE.md`, `ARCHITECTURE.md`,
> `docs/superpowers/STATUS.md`, and the code.

**Codename:** `spectral_forge`
**Version:** 0.1.0-spec
**Format:** CLAP (exclusively)
**Platform:** Linux (x86_64, EndeavourOS primary target)
**License:** TBD (ISC recommended for nih_plug compatibility)

---

## 1. Purpose & Design Philosophy

This plugin is a **real-time spectral processor** controlled by an interactive 6-node curve drawn in an egui GUI. Audio is decomposed via STFT into frequency bins; each bin's magnitude is scaled by a value sampled from the user's curve. The processed spectrum is resynthesised via overlap-add.

The architecture is designed for two audiences simultaneously:

1. **A human experimenter** (Kim) who will invent new spectral effects by swapping DSP engines without touching the GUI or transport plumbing.
2. **An agentic AI coding assistant** (Claude Code or similar) that must be able to implement, debug, and extend any single layer of the system without loading the entire codebase into its context window.

### Core Invariants

These rules are absolute. No refactoring, optimisation, or feature addition may violate them.

| ID | Invariant |
|----|-----------|
| **I-1** | The audio thread (`process()`) never allocates heap memory, never locks a mutex, never performs I/O. |
| **I-2** | The GUI thread never writes to the audio buffer or touches `StftHelper` state. |
| **I-3** | No file in `src/` exceeds 500 lines. If it does, it must be split before any new feature is added. |
| **I-4** | The `SpectralEngine` trait is the *only* interface between the STFT pipeline and the effect algorithm. No engine may reach outside its own module for DSP state. |
| **I-5** | The curve editor produces unit-normalised floats `[0.0, 1.0]`. Semantic interpretation (dB, depth, ratio) is the engine's responsibility, never the editor's. |
| **I-6** | All cross-thread data transfer uses `triple_buffer` (latest-frame semantics) or `Arc<AtomicF32>` (scalar values). No `crossbeam-channel`, no `std::sync::Mutex`. |

---

## 2. Directory Structure

```
spectral_forge/
├── Cargo.toml
├── xtask/
│   └── src/main.rs                  # cargo xtask bundle target
├── src/
│   ├── lib.rs                       # Plugin struct, Params, CLAP export macro. ≤200 lines.
│   ├── params.rs                    # #[derive(Params)] struct, all FloatParam/BoolParam defs.
│   ├── editor.rs                    # egui editor: curve widget, spectrum display, layout.
│   ├── editor/
│   │   ├── mod.rs                   # re-exports
│   │   ├── curve.rs                 # CurveEditor widget: 6-node Catmull-Rom, drag logic.
│   │   └── spectrum_display.rs      # Spectrum visualiser: reads triple_buffer, paints bars.
│   ├── dsp/
│   │   ├── mod.rs                   # re-exports
│   │   ├── pipeline.rs              # StftHelper setup, forward/inverse FFT, engine dispatch.
│   │   └── engines/
│   │       ├── mod.rs               # SpectralEngine trait definition + engine registry.
│   │       ├── curve_gain.rs        # Default engine: per-bin gain from curve. ~80 lines.
│   │       └── README.md            # Instructions for adding a new engine (agent-facing).
│   └── bridge.rs                    # All cross-thread shared state: triple_buffers, atomics.
└── tests/
    ├── stft_roundtrip.rs            # Sine → STFT → identity engine → ISTFT → compare.
    ├── curve_sampling.rs            # 6 nodes → 1025 floats, monotonicity / boundary checks.
    └── engine_contract.rs           # Property tests for SpectralEngine implementors.
```

### File Ownership Rules (for agents)

Each file has exactly one owner concern. An agent prompt must declare which file(s) it will touch. If a task requires editing files from two different rows below, it is two separate agent tasks.

| File | Owner Concern | Agent May Read | Agent May Write |
|------|--------------|----------------|-----------------|
| `lib.rs` | Plugin lifecycle, CLAP export | Any file | Only `lib.rs` |
| `params.rs` | Parameter declarations | Any file | Only `params.rs` |
| `editor.rs` | Top-level GUI layout | `params.rs`, `bridge.rs` | Only `editor.rs` |
| `editor/curve.rs` | Curve widget internals | `bridge.rs` | Only `curve.rs` |
| `editor/spectrum_display.rs` | Spectrum painter | `bridge.rs` | Only this file |
| `dsp/pipeline.rs` | STFT plumbing | `bridge.rs`, `engines/mod.rs` | Only `pipeline.rs` |
| `dsp/engines/mod.rs` | Trait definition | Nothing else | Only `mod.rs` |
| `dsp/engines/*.rs` | Individual engine | `engines/mod.rs` | Only its own file |
| `bridge.rs` | Shared state types | `params.rs` | Only `bridge.rs` |

---

## 3. Dependency Stack

```toml
[dependencies]
nih_plug       = { git = "https://github.com/robbert-vdh/nih-plug.git", default-features = false }
nih_plug_egui  = { git = "https://github.com/robbert-vdh/nih-plug.git" }
realfft        = "3"
triple_buffer  = "9"
parking_lot    = "0.12"              # Only for Arc<Mutex<TripleBufferReader>> in GUI
num-complex    = "0.4"

[dev-dependencies]
approx = "0.5"                      # Float comparison in tests

[profile.release]
lto = "thin"
opt-level = 3
strip = "symbols"

[profile.dev]
# Catch RT violations during development
# Enable via: nih_plug = { ..., features = ["assert_process_allocs"] }
```

**Why no `crossbeam`?** See Invariant I-6. `crossbeam-channel` can allocate or block. `triple_buffer` is wait-free on both sides and automatically discards stale frames — the exact semantics needed for spectrum visualisation and curve value transfer.

**Why `realfft` over `rustfft`?** A 2048-point real FFT produces 1025 complex bins instead of 2048. This halves memory bandwidth and computation for real-valued audio signals. `realfft` wraps `rustfft` internally.

---

## 4. Data Contracts

These are the exact types that flow between modules. Every module boundary is typed; there are no stringly-typed or dynamically-sized surprises.

### 4.1 The SpectralEngine Trait

```rust
// src/dsp/engines/mod.rs

use num_complex::Complex;

/// The sole interface between the STFT pipeline and any spectral effect.
///
/// # Contract
/// - `process_bins` is called on the audio thread. It must not allocate,
///   lock, or perform I/O.
/// - `bins` is the forward-FFT output: N/2+1 complex values for an
///   N-point real FFT. Modify in place.
/// - `curve` contains exactly `bins.len()` floats in [0.0, 1.0],
///   sampled from the UI curve at initialisation or on curve change.
/// - `sample_rate` is provided for frequency-dependent calculations.
///
/// # Adding a New Engine
/// 1. Create `src/dsp/engines/my_engine.rs`
/// 2. `impl SpectralEngine for MyEngine`
/// 3. Add `pub mod my_engine;` to this file
/// 4. Update `EngineSelection` enum and `create_engine()` factory
/// 5. Run `cargo test --test engine_contract`
pub trait SpectralEngine: Send {
    /// Called once when the plugin initialises or when the host
    /// changes sample rate / buffer size.
    fn reset(&mut self, sample_rate: f32, fft_size: usize);

    /// Called once per STFT hop on the audio thread.
    /// Modify `bins` in place. `curve` values are pre-interpolated
    /// from the UI's 6 control nodes to bins.len() floats.
    fn process_bins(
        &mut self,
        bins: &mut [Complex<f32>],
        curve: &[f32],
        sample_rate: f32,
    );

    /// Human-readable name for GUI display / debug logging.
    fn name(&self) -> &'static str;
}

/// Registry of available engines. Extend this enum when adding engines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineSelection {
    CurveGain,
    // Future: Freeze, Blur, Chorus, Smear, ...
}

/// Factory function. This is the ONE line you change to load a new engine.
pub fn create_engine(selection: EngineSelection) -> Box<dyn SpectralEngine> {
    match selection {
        EngineSelection::CurveGain => Box::new(curve_gain::CurveGainEngine::new()),
    }
}
```

### 4.2 Cross-Thread Bridge

```rust
// src/bridge.rs

use std::sync::Arc;
use parking_lot::Mutex;
use triple_buffer::{TripleBuffer, Input as TbInput, Output as TbOutput};

/// FFT bin count for a 2048-point real FFT.
pub const NUM_BINS: usize = 1025;

/// Data flowing from Audio Thread → GUI Thread (spectrum visualisation).
/// Updated every STFT hop. GUI reads latest frame only.
pub type SpectrumData = [f32; NUM_BINS];

/// Data flowing from GUI Thread → Audio Thread (curve shape).
/// Updated on node drag. Audio reads latest frame only.
pub type CurveData = [f32; NUM_BINS];

/// All shared state between audio and GUI threads.
/// Created once in `Plugin::initialize()`, cloned via Arc into the editor.
pub struct SharedState {
    // Audio → GUI: magnitude spectrum for visualiser
    pub spectrum_tx: TbInput<SpectrumData>,
    pub spectrum_rx: Arc<Mutex<TbOutput<SpectrumData>>>,

    // GUI → Audio: interpolated curve values
    pub curve_tx: Arc<Mutex<TbInput<CurveData>>>,
    pub curve_rx: TbOutput<CurveData>,
}

impl SharedState {
    pub fn new() -> Self {
        let (spectrum_tx, spectrum_rx) = TripleBuffer::new(&[0.0f32; NUM_BINS]).split();
        let (curve_tx, curve_rx) = TripleBuffer::new(&[1.0f32; NUM_BINS]).split();
        Self {
            spectrum_tx,
            spectrum_rx: Arc::new(Mutex::new(spectrum_rx)),
            curve_tx: Arc::new(Mutex::new(curve_tx)),
            curve_rx,
        }
    }
}
```

### 4.3 Curve Node Model

```rust
// src/editor/curve.rs (partial — the data model only)

/// A single draggable control point on the curve.
/// X is in normalised log-frequency space [0.0, 1.0] mapping to [20 Hz, 20 kHz].
/// Y is in unit space [0.0, 1.0]. The engine decides what Y means.
#[derive(Clone, Copy, Debug)]
pub struct CurveNode {
    pub x: f32,  // 0.0 = 20 Hz, 1.0 = 20 kHz (log-scaled)
    pub y: f32,  // 0.0 = minimum effect, 1.0 = maximum effect
}

/// Fixed number of user-controllable nodes.
pub const NUM_NODES: usize = 6;

/// Default flat curve at 50% (engine-neutral midpoint).
pub const DEFAULT_NODES: [CurveNode; NUM_NODES] = [
    CurveNode { x: 0.0,  y: 0.5 },
    CurveNode { x: 0.2,  y: 0.5 },
    CurveNode { x: 0.4,  y: 0.5 },
    CurveNode { x: 0.6,  y: 0.5 },
    CurveNode { x: 0.8,  y: 0.5 },
    CurveNode { x: 1.0,  y: 0.5 },
];
```

---

## 5. STFT Pipeline Specification

### Parameters

| Parameter | Value | Rationale |
|-----------|-------|-----------|
| FFT size | 2048 samples | 1025 bins, ~46 ms latency at 44.1 kHz. Good frequency resolution for spectral sculpting without excessive latency. |
| Window | Hann | Satisfies COLA at 75% overlap. Smooth spectral leakage. |
| Overlap | 75% (hop = 512) | Perfect reconstruction with Hann. 4× redundancy. |
| Normalisation | `2.0 / (3.0 * N)` after IFFT | Compensates Hann×Hann window energy at 4× overlap. |
| Latency | 2048 samples (reported to host) | One full FFT frame. nih_plug's `StftHelper` handles reporting. |

### Pipeline Flow (in `dsp/pipeline.rs`)

```
Host audio buffer (128–2048 samples)
  ↓
StftHelper::process_overlap_add(buffer, overlap_count=4, |channel, real_fft_scratch| {
    ↓
    [1] Forward realfft: 2048 real → 1025 Complex<f32>
    ↓
    [2] Compute magnitudes → write to spectrum_tx (triple_buffer, for GUI)
    ↓
    [3] Read curve_rx (triple_buffer, from GUI) → curve_values: &[f32; 1025]
    ↓
    [4] engine.process_bins(&mut bins, &curve_values, sample_rate)
    ↓
    [5] Inverse realfft: 1025 Complex<f32> → 2048 real, apply normalisation
    ↓
    [6] StftHelper writes result into overlap-add accumulator
})
  ↓
Host receives processed audio with latency compensation
```

### The Default Engine: `CurveGainEngine`

```rust
// src/dsp/engines/curve_gain.rs — the complete first engine

use super::SpectralEngine;
use num_complex::Complex;

/// Simplest possible spectral engine: multiply each bin's magnitude
/// by the curve value mapped to a dB range.
///
/// curve = 0.0 → -inf dB (silence)
/// curve = 0.5 → 0 dB (unity)
/// curve = 1.0 → +12 dB (boost)
pub struct CurveGainEngine;

impl CurveGainEngine {
    pub fn new() -> Self { Self }

    /// Map unit [0.0, 1.0] to linear gain.
    /// 0.0 → 0.0 (silence), 0.5 → 1.0 (unity), 1.0 → 3.98 (+12 dB)
    #[inline]
    fn curve_to_gain(value: f32) -> f32 {
        if value < 0.001 {
            return 0.0;
        }
        let db = (value - 0.5) * 24.0; // [-12, +12] dB range
        10.0_f32.powf(db / 20.0)
    }
}

impl SpectralEngine for CurveGainEngine {
    fn reset(&mut self, _sample_rate: f32, _fft_size: usize) {}

    fn process_bins(
        &mut self,
        bins: &mut [Complex<f32>],
        curve: &[f32],
        _sample_rate: f32,
    ) {
        debug_assert_eq!(bins.len(), curve.len());
        for (bin, &cv) in bins.iter_mut().zip(curve.iter()) {
            let gain = Self::curve_to_gain(cv);
            bin.re *= gain;
            bin.im *= gain;
        }
    }

    fn name(&self) -> &'static str { "Curve Gain" }
}
```

---

## 6. Curve Editor Specification

### Interaction Model

| Action | Behaviour |
|--------|-----------|
| **Drag node** | Moves X/Y within bounds. X clamped to `[prev_node.x + ε, next_node.x - ε]` to prevent crossing. Y clamped to `[0.0, 1.0]`. |
| **Double-click background** | Adds a node at click position (if `num_nodes < MAX_NODES`). |
| **Right-click node** | Removes node (if `num_nodes > MIN_NODES`). |
| **Scroll wheel on node** | Fine Y adjustment (±0.01 per tick). |

### Interpolation: Catmull-Rom → 1025 Samples

1. Sort nodes by X.
2. Prepend a phantom node: `(nodes[0].x - 0.1, nodes[0].y)`.
3. Append a phantom node: `(nodes[last].x + 0.1, nodes[last].y)`.
4. For each adjacent quadruple `(p0, p1, p2, p3)`, evaluate Catmull-Rom at ~20 points per segment.
5. Collect all interpolated (x, y) points.
6. Resample at 1025 evenly-spaced X positions (bin frequencies in log space).
7. Clamp all Y values to `[0.0, 1.0]`.
8. Write the 1025-float array to `curve_tx` via triple_buffer.

### Coordinate Spaces

| Space | X range | Y range | Used by |
|-------|---------|---------|---------|
| **Domain** | `[0.0, 1.0]` (log freq) | `[0.0, 1.0]` (unit) | Node storage, engine input |
| **Screen** | `[rect.left, rect.right]` px | `[rect.top, rect.bottom]` px | egui painting |
| **Frequency** | `[20 Hz, 20000 Hz]` | — | Display labels |
| **Bin index** | `[0, 1024]` | — | FFT processing |

Conversion: `freq = 20.0 * (1000.0_f32).powf(x_norm)` for log-spaced X ∈ [0, 1] → [20, 20000].
Bin index: `bin = (freq / sample_rate) * fft_size`.

---

## 7. Parameter Specification

```rust
// src/params.rs

#[derive(Params)]
pub struct SpectralForgeParams {
    /// Master dry/wet mix. 0% = bypass, 100% = fully processed.
    #[id = "mix"]
    pub mix: FloatParam,

    /// Master output gain in dB.
    #[id = "output_gain"]
    pub output_gain: FloatParam,

    /// Input gain in dB (pre-FFT).
    #[id = "input_gain"]
    pub input_gain: FloatParam,

    // Curve node positions are NOT DAW parameters.
    // They are persisted via #[persist] as serialised JSON
    // to avoid polluting the host's automation lanes.
    #[persist = "curve_nodes"]
    pub curve_nodes: Arc<Mutex<Vec<CurveNode>>>,

    #[persist = "editor_state"]
    pub editor_state: Arc<EguiState>,
}
```

| Param | Range | Default | Smoothing | Rationale |
|-------|-------|---------|-----------|-----------|
| `mix` | 0.0–1.0 | 1.0 | Linear 10 ms | Gradual blend between dry and wet in time domain |
| `output_gain` | −24 to +24 dB | 0 dB | Log 50 ms | Post-processing level trim |
| `input_gain` | −24 to +24 dB | 0 dB | Log 50 ms | Drive into spectral processing |

Curve node persistence uses `#[persist = "..."]` with `serde_json`, not `FloatParam`. This keeps the automation lane clean — curves are spatial gestures, not per-sample automatable values.

---

## 8. Implementation Phases

Each phase is a self-contained agent task. The agent prompt for each phase should include only the files listed in the **Context** column.

### Phase 1: Skeleton

**Goal:** CLAP plugin loads in host, shows blank egui window, passes audio through unmodified.

| File | Action | Context |
|------|--------|---------|
| `Cargo.toml` | Create with deps | — |
| `lib.rs` | Plugin struct, CLAP export, `process()` passthrough | `params.rs` |
| `params.rs` | Params with mix/gain only | — |
| `editor.rs` | Blank egui window with "Spectral Forge" title | `params.rs` |
| `bridge.rs` | Stub `SharedState` with triple_buffers | — |

**Acceptance test:** `cargo xtask bundle spectral_forge --release` produces a `.clap` that loads in Carla/REAPER. Audio passes through. GUI opens.

### Phase 2: STFT Pipeline (headless)

**Goal:** Audio passes through STFT→IFFT roundtrip with identity processing. Latency is correctly reported.

| File | Action | Context |
|------|--------|---------|
| `dsp/pipeline.rs` | `StftHelper` + `realfft` setup, identity callback | `bridge.rs`, `engines/mod.rs` |
| `dsp/engines/mod.rs` | `SpectralEngine` trait definition | — |
| `dsp/engines/curve_gain.rs` | Identity engine (gain = 1.0 for all bins) | `engines/mod.rs` |
| `lib.rs` | Wire pipeline into `process()` | `dsp/pipeline.rs`, `bridge.rs` |

**Acceptance test:** `tests/stft_roundtrip.rs` — 440 Hz sine in, compare output. Max sample error < 1e-4.

### Phase 3: Curve Editor

**Goal:** 6 draggable nodes with Catmull-Rom interpolation. Curve values written to triple_buffer.

| File | Action | Context |
|------|--------|---------|
| `editor/curve.rs` | Full curve widget | `bridge.rs` |
| `editor.rs` | Integrate curve widget into GUI layout | `editor/curve.rs`, `params.rs` |
| `bridge.rs` | Wire `curve_tx` | — |

**Acceptance test:** Manual — drag nodes, verify smooth curve. `tests/curve_sampling.rs` — boundary conditions, 6 flat nodes → 1025 equal values.

### Phase 4: Connect Curve → Engine

**Goal:** Moving curve nodes changes the sound in real time.

| File | Action | Context |
|------|--------|---------|
| `dsp/pipeline.rs` | Read `curve_rx`, pass to `engine.process_bins()` | `bridge.rs`, `engines/mod.rs` |
| `dsp/engines/curve_gain.rs` | Implement actual gain mapping | `engines/mod.rs` |

**Acceptance test:** Flat curve at 0.5 → unity gain (no change). Curve pulled to 0.0 → silence in those frequency bands.

### Phase 5: Spectrum Visualiser

**Goal:** Real-time spectrum display in the GUI, painted behind the curve.

| File | Action | Context |
|------|--------|---------|
| `editor/spectrum_display.rs` | Read `spectrum_rx`, paint log-scaled bars | `bridge.rs` |
| `dsp/pipeline.rs` | Compute magnitudes, write to `spectrum_tx` | `bridge.rs` |
| `editor.rs` | Layer spectrum behind curve widget | `editor/spectrum_display.rs`, `editor/curve.rs` |

**Acceptance test:** Visual — spectrum reacts to audio input in real time.

### Phase 6: Polish & Persist

**Goal:** Curve nodes save/restore with DAW session. Mix/gain knobs in GUI.

| File | Action | Context |
|------|--------|---------|
| `params.rs` | `#[persist]` for curve nodes | — |
| `editor.rs` | Add `ParamSlider` for mix, input/output gain | `params.rs` |
| `lib.rs` | Restore curve state on `initialize()` | `params.rs`, `bridge.rs` |

---

## 9. Adding a New Engine (Agent Prompt Template)

This is the exact prompt to give an AI agent when experimenting with a new spectral effect. The modularity of the architecture means the agent needs **only two files** in its context.

```markdown
# Task: Implement a new SpectralEngine

## Files you may read:
- src/dsp/engines/mod.rs (for the SpectralEngine trait)
- src/dsp/engines/curve_gain.rs (as a reference implementation)

## Files you will create:
- src/dsp/engines/{engine_name}.rs

## Files you will modify:
- src/dsp/engines/mod.rs (add `pub mod {engine_name};` and extend EngineSelection enum)

## DO NOT read or modify:
- lib.rs, editor.rs, editor/*, pipeline.rs, bridge.rs, params.rs

## Requirements:
1. Create a struct that implements `SpectralEngine`.
2. `process_bins()` must not allocate, lock, or perform I/O.
3. `reset()` must pre-allocate any buffers the engine needs.
4. `curve` values are [0.0, 1.0]. You decide what they mean for your effect.
5. Add your engine to the `EngineSelection` enum and `create_engine()` match.
6. Write a doc comment explaining what curve=0.0 and curve=1.0 mean.
7. Run: `cargo test --test engine_contract`

## The effect to implement:
{DESCRIPTION OF THE DSP ALGORITHM}
```

### Example Engine Ideas for Future Experimentation

| Engine | `curve = 0.0` | `curve = 1.0` | Core Algorithm |
|--------|---------------|---------------|----------------|
| **Freeze** | Pass through | Hold magnitude from previous frame | `if cv > 0.5 { bin.mag = prev_mag[i] }` |
| **Blur** | No blur | Max spectral smearing | Weighted average of adjacent bins, width ∝ cv |
| **Gate** | Gate closed | Gate open | `if bin.mag < threshold(cv) { bin = 0 }` |
| **Rotate** | No rotation | Max phase rotation | `bin.phase += cv * π` |
| **Randomise** | Clean | Fully randomised phase | `bin.phase += rng.next() * cv * 2π` |
| **Spectral Tilt** | No tilt | Max high-frequency emphasis | Per-bin gain = `(bin_index / num_bins) * cv` |

---

## 10. Testing Strategy

### Unit Tests (per-module, run in CI)

| Test File | What It Validates |
|-----------|-------------------|
| `tests/stft_roundtrip.rs` | Identity engine through full STFT→ISTFT pipeline preserves signal (max error < 1e-4). |
| `tests/curve_sampling.rs` | Catmull-Rom with 6 equidistant nodes at y=0.5 produces 1025 values all ≈ 0.5. Boundary nodes clamp correctly. |
| `tests/engine_contract.rs` | Property test: any `SpectralEngine` given all-zero bins returns all-zero bins. Any engine given bins with curve=0.5 does not panic. `reset()` can be called multiple times. |

### Integration Test (manual, in DAW)

1. Load in Carla or REAPER on EndeavourOS.
2. Play pink noise → flat curve → output matches input.
3. Pull curve to zero at 1 kHz → visible notch in spectrum analyser.
4. Save session → close → reopen → curve state restored.
5. Automate `mix` parameter → smooth transition from dry to wet.

### Performance Benchmark

Target: < 5% single-core CPU at 44.1 kHz stereo, 512-sample buffer, 2048-point FFT. Measure with `perf stat` or nih_plug's built-in timing. The `assert_process_allocs` feature must pass with zero allocations in `process()`.

---

## 11. Build & Deploy

```bash
# Development build
cargo xtask bundle spectral_forge

# Release build (LTO, stripped)
cargo xtask bundle spectral_forge --release

# Install for current user (symlink to CLAP scan path)
mkdir -p ~/.clap
ln -sf $(pwd)/target/bundled/spectral_forge.clap ~/.clap/

# Run tests
cargo test --all
cargo test --test engine_contract
```

### Linux Dependencies (EndeavourOS / Arch)

```bash
sudo pacman -S libx11 libxcursor libxcb mesa
```

---

## Appendix A: Reference Repositories

| Repository | What to Study | Key Files |
|------------|---------------|-----------|
| [nih-plug/spectral_compressor](https://github.com/robbert-vdh/nih-plug/tree/master/plugins/spectral_compressor) | Production STFT pipeline, per-bin curve processing | `src/lib.rs`, `src/compressor_bank.rs` |
| [nih-plug/examples/stft](https://github.com/robbert-vdh/nih-plug/tree/master/plugins/examples/stft) | Minimal `StftHelper` usage | `src/lib.rs` |
| [nih-plug/examples/gain_gui_egui](https://github.com/robbert-vdh/nih-plug/tree/master/plugins/examples/gain_gui_egui) | egui integration, `AtomicF32` peak meter | `src/lib.rs` |
| [nih-plug/diopser](https://github.com/robbert-vdh/nih-plug/tree/master/plugins/diopser) | Custom `SpectrumInput`/`SpectrumOutput` lock-free bridge | `src/spectrum.rs` |
| [triple-buffer](https://github.com/HadrienG2/triple-buffer) | Wait-free data exchange API | `src/lib.rs` |
| [CYMA](https://sr.ht/~voidstar-audio/CYMA/) | `MonoBus`/`StereoBus` lock-free visualiser architecture | — |

## Appendix B: nih_plug API Quick Reference

| Concept | API |
|---------|-----|
| CLAP-only export | `nih_export_clap!(PluginStruct);` |
| Parameter read (audio) | `self.params.mix.value()` or `.smoothed.next()` |
| Parameter write (GUI) | `setter.begin_set_parameter(&params.mix); setter.set_parameter(&params.mix, v); setter.end_set_parameter(&params.mix);` |
| Persistent state | `#[persist = "key"] field: Arc<Mutex<T>>` where T: Serialize + Deserialize |
| STFT helper | `nih_plug::util::StftHelper::new(num_channels, fft_size, max_block_size)` |
| Window functions | `nih_plug::util::window::hann(window_size)` |
| dB ↔ linear | `nih_plug::util::db_to_gain(db)`, `gain_to_db(gain)` |
