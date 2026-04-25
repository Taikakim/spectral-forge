# Architecture Overview

Spectral Forge is a **spectral dynamics and modular multi-fx** CLAP plugin for
Linux and Windows, written in Rust. It performs per-FFT-bin processing across up
to 9 slots (8 user slots + Master), each assignable to a typed `SpectralModule`
(Dynamics, Freeze, PhaseSmear, Contrast, Gain, MidSide, T/S Split, Harmonic,
Master). Slots are wired through a `RouteMatrix` and each is driven by up to 7
drawn parameter curves.

Update this document whenever a major subsystem changes. For the status of
design specs and implementation plans under `docs/superpowers/`, see
[`docs/superpowers/STATUS.md`](docs/superpowers/STATUS.md).

---

## 1. Project Structure

```
spectral_forge/
├── src/
│   ├── lib.rs                  Plugin entry point (nih-plug Plugin + ClapPlugin impl)
│   ├── params.rs               All shared state: audio params + persisted GUI state
│   ├── bridge.rs               SharedState: triple-buffer channels GUI↔Audio, AtomicF32
│   ├── presets.rs              Built-in preset definitions
│   ├── editor_ui.rs            Top-level egui frame; assembles all widgets
│   │
│   ├── editor/                 GUI subsystem
│   │   ├── mod.rs              pub use re-exports
│   │   ├── theme.rs            ALL visual constants (colours, sizes) — edit here to reskin
│   │   ├── curve.rs            CurveNode, compute_curve_response(), paint + interact
│   │   ├── spectrum_display.rs Pre/post-FX spectrum gradient painter
│   │   ├── suppression_display.rs  Legacy gain-reduction display (kept for reference)
│   │   ├── fx_matrix_grid.rs   9×9 slot routing matrix widget
│   │   └── module_popup.rs     Right-click module assignment popup
│   │
│   └── dsp/                    Audio subsystem (real-time safe)
│       ├── mod.rs
│       ├── pipeline.rs         STFT overlap-add, M/S encode, per-slot curve application
│       ├── guard.rs            flush_denormals(), sanitize() (clamp NaN/Inf)
│       ├── fx_matrix.rs        Routing matrix gain computation
│       │
│       ├── engines/            Per-bin spectral engine implementations
│       │   ├── mod.rs          SpectralEngine trait, BinParams<'_> struct, EngineSelection
│       │   ├── spectral_compressor.rs  Main compressor engine (envelope → gain → smooth)
│       │   └── spectral_contrast.rs   Contrast/transient engine
│       │
│       └── modules/            Slot module implementations
│           ├── mod.rs          ModuleType enum, ModuleSpec, SpectralModule trait,
│           │                   apply_curve_transform(), create_module()
│           ├── dynamics.rs     Compressor/expander module
│           ├── freeze.rs       Spectral freeze module
│           ├── phase_smear.rs  Phase randomisation module
│           ├── contrast.rs     Spectral contrast module
│           ├── gain.rs         Spectral gain shaping module
│           ├── harmonic.rs     Harmonic emphasis module
│           ├── mid_side.rs     M/S balance + phase decorrelation module
│           ├── ts_split.rs     Transient/Sustained split module
│           └── master.rs       Master output slot module
│
├── tests/                      Integration tests (use the rlib crate target)
│   ├── engine_contract.rs      BinParams contract: no NaN, no alloc, suppression ≥ 0
│   ├── stft_roundtrip.rs       Overlap-add identity test
│   ├── curve_sampling.rs       compute_curve_response() sampling correctness
│   ├── module_trait.rs         SpectralModule trait compliance per module type
│   └── presets.rs              Preset serialisation round-trip
│
├── xtask/                      Build helper (nih-plug-xtask wrapper)
│   └── src/main.rs             Entry point: `cargo run -p xtask -- bundle ...`
│
├── docs/
│   ├── MANUAL.md               End-user manual
│   ├── OPTIMISATION.md         Profiling notes and SIMD dispatch decisions
│   ├── research-background/    Academic references (RTF format)
│   ├── superpowers/plans/      AI-assisted implementation plans
│   └── superpowers/specs/      AI-assisted design specs
│
├── ascii art/                  Splash-screen / branding experiments (Python scripts)
│
├── patents/                    Reference copy of the oeksound patent (avoided by design)
│
├── Cargo.toml                  Package manifest and dependency versions
├── Cargo.lock                  Pinned dependency tree
├── .cargo/config.toml          Linker / target overrides
├── ARCHITECTURE.md             This document
├── CLAUDE.md                   AI assistant guide (subsystem contracts, gotchas)
├── GUI.md                      GUI modding guide (reskinning, widget contract, HiDPI)
├── CREDITS.md                  Acknowledgements
├── MANUAL.md                   (symlink / copy of docs/MANUAL.md)
└── README.md                   Project overview and quick-start
```

---

## 2. Data Flow

```
┌─────────────────────────────────────────────────────────────────────┐
│  GUI thread (≈60 fps)                                               │
│                                                                     │
│  egui widgets → SpectralForgeParams (Arc<Mutex<…>>)                │
│               → compute_curve_response()                            │
│               → triple_buffer::Input::publish()  ──────────────┐  │
│                                                                  │  │
│  ← spectrum_rx.read()  ←────────────────────────────────────┐  │  │
│  ← suppression_rx.read() ←──────────────────────────────┐   │  │  │
└──────────────────────────────────────────────────────────│───│──│──┘
                                                           │   │  │
┌─────────────────────────────────── audio thread ─────────│───│──│──┐
│                                                           │   │  │  │
│  Plugin::process()                                        │   │  │  │
│    └─ Pipeline::process()                                 │   │  │  │
│         ├─ curve_rx[slot][curve].read() ←────────────────│───│──┘  │
│         │    apply_curve_transform(tilt, offset)          │   │     │
│         │    → slot_curve_cache[slot][curve]              │   │     │
│         ├─ STFT overlap-add (realfft)                     │   │     │
│         │    for each slot:                               │   │     │
│         │      SpectralModule::process_bins(BinParams)    │   │     │
│         ├─ spectrum_tx.publish() ────────────────────────────┘     │
│         └─ suppression_tx.publish() ────────────────────┘          │
└─────────────────────────────────────────────────────────────────────┘
```

**Key rule**: the audio thread never locks a `Mutex`.  It uses `try_lock()` only for
`slot_curve_meta` (tilt/offset, non-critical) and reads curves via lock-free
`triple_buffer::Output::read()`.

---

## 3. Core Components

### 3.1 Plugin entry point — `src/lib.rs`

Implements `nih_plug::Plugin` and `nih_plug::ClapPlugin`.  Owns the `Pipeline`,
`SharedState`, and the `Arc<SpectralForgeParams>`.  `initialize()` sets up the STFT
and pushes initial curves to the triple-buffers.  `process()` calls
`Pipeline::process()` each block.

### 3.2 Parameters — `src/params.rs`

Single struct `SpectralForgeParams` holding every piece of state shared between
the GUI and the DSP.  Two categories:

| Category | Storage | Read by DSP |
|----------|---------|-------------|
| Audio params | `FloatParam`, `BoolParam`, `EnumParam` (nih-plug types) | Yes, smoothed |
| GUI/persist state | `Arc<Mutex<T>>` with `#[persist]` | Via `try_lock()` or not at all |

Persisted GUI state (curve nodes, slot names, routing matrix, ui_scale, etc.) is
serialised to the DAW preset by nih-plug automatically.

### 3.3 Bridge — `src/bridge.rs`

`SharedState` bundles all GUI↔audio communication channels:

- `curve_rx[9][7]` — 9 slots × 7 curves, lock-free `triple_buffer::Output<Vec<f32>>`
  (audio reads); corresponding `curve_tx[9][7]` on the GUI side.
- `spectrum_tx` / `suppression_tx` — audio writes, GUI reads.
- `AtomicF32` for sample rate.
- `gui_sidechain_active: [AtomicBool; 4]` — GUI reads SC activity indicators.

### 3.4 Pipeline — `src/dsp/pipeline.rs`

Owns one `StftHelper` (realfft overlap-add, hop = FFT/4), one for the sidechain,
and an array of `Box<dyn SpectralModule>` (9 slots).  Each block:

1. Reads curve caches from triple-buffer; applies tilt/offset.
2. Runs STFT on the main signal.
3. For each slot: assembles `BinParams<'_>` slices and calls
   `module.process_bins()`.
4. Writes spectrum and suppression to their triple-buffers for the GUI.

**SIMD dispatch**: per-bin gain application uses runtime multiversion dispatch
(`multiversion` crate) for AVX-512 / AVX2 / SSE2 / scalar paths.

### 3.5 Modules — `src/dsp/modules/`

Each file implements one `SpectralModule`.  The trait requires:

```rust
fn process_bins(&mut self, params: BinParams<'_>);
fn reset(&mut self);
fn tail_length(&self) -> u32;  // in samples, default 0
```

`BinParams<'_>` carries all per-bin slices (threshold, ratio, attack, release, knee,
makeup, mix, sidechain gains, suppression output).  Modules must not allocate and
must fill `suppression_out` with finite non-negative values.

### 3.6 Spectral engines — `src/dsp/engines/`

Lower-level primitives used by modules.  `spectral_compressor.rs` contains the
envelope follower → gain computer → per-bin smoother chain used by `DynamicsModule`.

### 3.7 GUI — `src/editor/` + `src/editor_ui.rs`

Built with `nih_plug_egui` (egui 0.31).  `editor_ui.rs` is the top-level frame
closure; it reads params, assembles widgets, publishes curve changes to triple-buffers.
All visual constants live in `theme.rs`.  See `GUI.md` for the modding guide.

---

## 4. Key Constants

| Constant | Value | Location |
|----------|-------|----------|
| `MAX_NUM_BINS` | 8193 (FFT 16384 / 2 + 1) | `pipeline.rs` |
| `OVERLAP` | 4 (75% overlap) | `pipeline.rs` |
| `NUM_CURVE_SETS` | 7 | `params.rs` / `CLAUDE.md` |
| `NUM_NODES` | 6 per curve | `curve.rs` |
| Base window size | 900 × 1010 logical px | `params.rs` |

---

## 5. Build & Test

```bash
# Debug build
cargo build

# Release build
cargo build --release

# Bundle as .clap + .vst3
cargo run --package xtask -- bundle spectral_forge --release

# Install to Bitwig search path
cp target/bundled/spectral_forge.clap ~/.clap/

# Run all tests (28 total)
cargo test

# Standalone headless mode (for UI iteration without a DAW)
cargo run --bin spectral_forge_standalone
```

**CI**: no CI pipeline yet.  Run `cargo test` before committing.

---

## 6. Real-Time Safety Rules

The audio thread (`Plugin::process` and everything it calls) must never:

- Allocate (`Vec::new`, `collect`, `clone` on `Vec`, etc.)
- Lock a `Mutex` (use `try_lock()` only for non-critical meta, skip on contention)
- Perform I/O or `println!`

The `assert_process_allocs` Cargo feature is enabled and will abort on violation.
`guard::flush_denormals()` sets FTZ+DAZ CPU flags at the start of each block.

---

## 7. Dependencies

| Crate | Purpose |
|-------|---------|
| `nih-plug` | Plugin framework (CLAP, VST3, standalone) |
| `nih-plug-egui` | egui integration for nih-plug |
| `realfft` | Real-valued FFT (in-place, no allocation after init) |
| `triple_buffer` | Lock-free single-producer/single-consumer GUI↔audio bridge |
| `parking_lot` | Fast `Mutex` for GUI-side locks |
| `num-complex` | Complex number type for FFT bins |
| `multiversion` | Runtime SIMD dispatch (AVX-512, AVX2, SSE2, scalar) |
| `serde` / `serde_json` | Preset and curve-node serialisation |
| `approx` | Approximate floating-point comparisons in tests |

---

## 8. Design Decisions & Constraints

- **Patent-safe**: avoids the oeksound Hilbert/convolution approach (see `patents/`).
  The per-bin compressor is a straightforward envelope follower applied bin-by-bin.
- **Linux only**: targets ALSA/PipeWire + Bitwig.  No macOS / Windows paths.
- **Single crate**: both the `cdylib` plugin and the `rlib` test target share one
  `Cargo.toml`.
- **No DAW automation**: `slot_curve_nodes`, `slot_module_types`, and routing state
  are `#[persist]` GUI state, not automatable `FloatParam` fields.  Automatable
  params are only the global knobs (input gain, output gain, mix, etc.).

---

## 9. Future Considerations

- Per-slot frequency scaling (curve tilt vs. true freq-dependent A/R).
- Proper CI with `cargo test` on push.
- JSON-based theme loading for runtime reskinning (see `GUI.md`).
- Window resize via host API (currently via `ViewportCommand::InnerSize`; may need
  nih-plug upstream change for reliable cross-host resizing).

---

## 10. Project Identification

**Project name**: Spectral Forge  
**Version**: 0.15.x (see `Cargo.toml`)  
**Platform**: Linux, CLAP + VST3  
**Primary AI guide**: `CLAUDE.md`  
**Date of last update**: 2026-04-20

---

## 11. Glossary

| Term | Definition |
|------|-----------|
| BinParams | Struct of slices passed to each module's `process_bins()` — one value per FFT bin for each parameter curve |
| CLAP | CLever Audio Plugin format — the primary plugin format target |
| Curve | One of 7 per-slot parameter shapes drawn by the user (threshold, ratio, attack, release, knee, makeup, mix) |
| DSP | Digital Signal Processing — refers to the audio-thread code in `src/dsp/` |
| FFT | Fast Fourier Transform — converts time-domain audio blocks to per-bin frequency data |
| Module | A `SpectralModule` implementation assigned to one of 9 slots in the routing matrix |
| Node | A control point in a drawn parameter curve (`CurveNode`: x=frequency, y=gain, q=bandwidth) |
| OLA | Overlap-Add — the STFT reconstruction method; 75% overlap (hop = FFT/4) |
| Slot | One of 9 parallel processing lanes; each has a module type, 7 curves, and routing connections |
| Triple-buffer | Lock-free single-writer / single-reader buffer used for GUI→audio (curves) and audio→GUI (spectrum) |
| T/S Split | Transient/Sustained Split module — splits the signal into transient and tonal components across bins |
