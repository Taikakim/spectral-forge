# Architecture Overview

Spectral Forge is a **spectral dynamics and modular multi-fx** CLAP plugin for
Linux and Windows, written in Rust. It performs per-FFT-bin processing across up
to 9 slots (8 user slots + Master), each assignable to a typed `SpectralModule`
(19 module types as of v0.15: Dynamics, Freeze, PhaseSmear, Contrast, Gain,
MidSide, T/S Split, Harmonic, Future, Punch, Rhythm, Geometry, Modulate, Circuit,
Life, Past, Kinetics, Harmony, Master; unassigned slots use `Empty`). Slots are
wired through a `RouteMatrix` and each is driven by up to 7 drawn parameter curves.

Update this document whenever a major subsystem changes. For the status of
design specs and implementation plans under `docs/superpowers/`, see
[`docs/superpowers/STATUS.md`](docs/superpowers/STATUS.md).

---

## 1. Project Structure

```
spectral_forge/
├── src/
│   ├── lib.rs                  Plugin entry point (nih-plug Plugin + ClapPlugin impl)
│   ├── params.rs               All shared state: audio params + persisted GUI state;
│   │                           include!s the build.rs-generated GeneratedParams
│   ├── param_ids.rs            Stable automation-param ID formatting (shared with build.rs)
│   ├── bridge.rs               SharedState: triple-buffer + atomic channels GUI↔Audio, AtomicF32
│   ├── preset.rs               User preset files: JSON save/load/apply, per-user preset directory
│   ├── presets.rs              Built-in (factory) preset definitions
│   ├── editor_ui.rs            Top-level egui frame; assembles all widgets
│   │
│   ├── editor/                 GUI subsystem
│   │   ├── mod.rs              pub use re-exports
│   │   ├── theme.rs            ALL visual constants (colours, sizes) — edit here to reskin
│   │   ├── curve.rs            CurveNode, compute_curve_response(), paint + interact
│   │   ├── curve_config.rs     Single source of per-curve display ranges, grid lines, units
│   │   ├── spectrum_display.rs Pre/post-FX spectrum gradient painter
│   │   ├── fx_matrix_grid.rs   Slot routing matrix widget (9 slots + T/S Split virtual rows)
│   │   ├── module_popup.rs     Right-click module assignment popup
│   │   ├── amp_popup.rs        Per-cell matrix amp-mode popup
│   │   ├── mod_ring.rs         Per-node modulation ring overlay (S/H, Sync, Legato)
│   │   ├── help_box.rs         Context help box shown right of the FX matrix
│   │   ├── preset_menu.rs      Preset save/load menu
│   │   └── …                   Per-module *_panel.rs / *_popup.rs files (Circuit, Life, Past, …)
│   │
│   └── dsp/                    Audio subsystem (real-time safe)
│       ├── mod.rs
│       ├── pipeline.rs         STFT overlap-add, M/S encode, per-slot curve application
│       ├── guard.rs            flush_denormals(), sanitize() (clamp NaN/Inf)
│       ├── fx_matrix.rs        FxMatrix: owns the 9 slot modules, dispatches them per hop in
│       │                       slot order, mixes routing sends/amp nodes, carries BinPhysics
│       ├── amp_modes.rs        Per-cell non-linear amp modes applied along matrix sends
│       ├── bin_physics.rs      BinPhysics per-bin physics state carried between slots
│       ├── physics_helpers.rs  Shared RT-safe helpers for physics-based modules (Kinetics, …)
│       ├── plpv.rs             Peak-locked phase vocoder: phase unwrap + peak/Voronoi detection
│       ├── phase.rs            LUT-based per-bin phase rotation helper
│       ├── instantaneous_freq.rs  Per-bin instantaneous frequency from consecutive phases
│       ├── history_buffer.rs   Per-channel rolling complex-spectrum history (read via ctx.history)
│       ├── cepstrum.rs         Cepstrum analysis scratch (log-magnitude → inverse FFT)
│       ├── chromagram.rs       12-pitch-class chromagram from bin magnitudes
│       ├── harmonic_groups.rs  Harmonic-group detection (up to 4 groups per hop)
│       ├── midi.rs             MIDI held-note bookkeeping (allocation-free)
│       ├── modulation_ring.rs  RingStateBank: per-node modulation ring state (9×7×6 = 378)
│       ├── circuit_kernels.rs  Scalar analog-component kernels shared by the Circuit module
│       ├── soft_clip.rs        Master output spectral soft clipper
│       ├── utils.rs            xorshift64 PRNG, linear→dBFS, ms→smoothing-coefficient helpers
│       │
│       ├── engines/            Per-bin spectral engine implementations
│       │   ├── mod.rs          SpectralEngine trait, BinParams<'_> struct, EngineSelection
│       │   ├── spectral_compressor.rs  Main compressor engine (envelope → gain → smooth)
│       │   └── spectral_contrast.rs   Contrast/transient engine
│       │
│       └── modules/            Slot module implementations (19 types + Empty)
│           ├── mod.rs          ModuleType enum, ModuleSpec, SpectralModule trait, RouteMatrix,
│           │                   apply_curve_transform(), create_module()
│           ├── dynamics.rs     Compressor/expander module
│           ├── freeze.rs       Spectral freeze module
│           ├── phase_smear.rs  Phase randomisation module
│           ├── contrast.rs     Spectral contrast (Spatial/Temporal/Tilt)
│           ├── gain.rs         Spectral gain shaping (Add/Subtract/Pull/Match)
│           ├── harmonic.rs     Harmonic emphasis (stub: no-op, 0 curves)
│           ├── mid_side.rs     M/S balance + phase decorrelation module
│           ├── ts_split.rs     Transient/Sustained split module
│           ├── future.rs       Spectral lookahead (Print-Through/Pre-Echo)
│           ├── punch.rs        Sidechain-driven transient carving (Direct/Inverse)
│           ├── rhythm.rs       BPM-synced gating (Euclidean/Arpeggiator/Phase Reset)
│           ├── geometry.rs     Chladni plate + Helmholtz trap modes
│           ├── modulate.rs     Phase/ring/FM modulation (8 modes incl. PLL Tear)
│           ├── circuit.rs      Analog circuit emulation (10 modes: BBD, Schmitt, Vactrol, …)
│           ├── life.rs         Fluid/physical-metaphor effects (10 modes)
│           ├── past.rs         History-buffer time effects (5 modes)
│           ├── kinetics.rs     Physical-force bin dynamics (8 modes)
│           ├── harmony.rs      Harmonic/chord manipulation (8 modes)
│           ├── harmony_helpers.rs  Peak picking, chord templates, Bessel-zero LUT
│           └── master.rs       Master output slot module + EmptyModule passthrough
│
├── tests/                      Integration tests (68 files, 600+ tests; use the rlib crate target)
│   ├── engine_contract.rs      BinParams contract: no NaN, no alloc, suppression ≥ 0
│   ├── stft_roundtrip.rs       Overlap-add identity test
│   ├── curve_sampling.rs       compute_curve_response() sampling correctness
│   ├── module_trait.rs         SpectralModule trait compliance per module type
│   ├── presets.rs              Preset serialisation round-trip
│   ├── common/mod.rs           Shared test helpers
│   └── …                       Per-module, BinPhysics, PLPV, MIDI, calibration tests
│
├── xtask/                      Build helper (nih-plug-xtask wrapper)
│   └── src/main.rs             Entry point: `cargo run -p xtask -- bundle ...`
│
├── vendor/
│   └── nih_plug_egui/          Patched local copy of nih-plug's egui adapter (see §7)
│
├── docs/
│   ├── MANUAL.md               End-user manual
│   ├── OPTIMISATION.md         Profiling notes and SIMD dispatch decisions
│   ├── superpowers/STATUS.md   Status index of specs and plans
│   ├── superpowers/plans/      AI-assisted implementation plans
│   ├── superpowers/specs/      AI-assisted design specs
│   └── superpowers/ideas/      Small feature ideas
│                               (other docs/ content is gitignored local scratch)
│
├── ideas/next-gen-modules/     Next-gen module design notes + research/ findings
│
├── build.rs                    Generates params_gen.rs (1440-param automation grid + 243 per-slot module scalars)
├── build.sh                    macOS build + sign + notarize (+ optional .pkg) script
├── Cargo.toml                  Package manifest, dependency versions, [patch] for vendor/
├── Cargo.lock                  Pinned dependency tree
├── .cargo/config.toml          Windows GNU cross-link settings (mingw gcc + lld)
├── ARCHITECTURE.md             This document
├── CHANGELOG.md                Release history (Keep a Changelog, loosely)
├── CLAUDE.md                   AI assistant guide (subsystem contracts, gotchas)
├── GUI.md                      GUI modding guide (reskinning, widget contract, HiDPI)
├── CREDITS.md                  Acknowledgements
├── notarize-signing-apple.md   macOS code-signing and notarization walkthrough
├── SPECTRAL_PLUGIN_TECHNICAL_SPEC.md  Original 0.1.0 design spec (superseded, historical)
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
│         │      SpectralModule::process(bins, curves, …)   │   │     │
│         ├─ spectrum_tx.publish() ────────────────────────────┘     │
│         └─ suppression_tx.publish() ────────────────────┘          │
└─────────────────────────────────────────────────────────────────────┘
```

**Key rule**: the audio thread never locks a `Mutex`.  It uses `try_lock()` (with a fallback
value on contention) to snapshot `#[persist]` GUI state such as `slot_module_types`,
`slot_targets`, `slot_curve_nodes`, and the modulation-ring bank (`ring_states`), and
reads curves via lock-free
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

`SharedState` bundles all GUI↔audio communication channels (all buffers are
pre-allocated at `MAX_NUM_BINS` so FFT-size changes never reallocate):

| Field | Type | Direction / purpose |
|-------|------|---------------------|
| `num_bins` | `usize` | Always `MAX_NUM_BINS`; kept for backward compatibility |
| `fft_size` | `Arc<AtomicUsize>` | Current active FFT size; GUI reads it to know how many bins are valid |
| `curve_tx` | `Vec<Vec<Arc<Mutex<triple_buffer::Input<Vec<f32>>>>>>` | GUI → audio: 9 slots × 7 curves, GUI write handles |
| `curve_rx` | `Vec<Vec<triple_buffer::Output<Vec<f32>>>>` | Audio-thread read handles for the same 9 × 7 curves (lock-free) |
| `sidechain_active` | `Arc<AtomicBool>` | Audio → GUI: whether the aux sidechain input carries signal |
| `reset_requested` | `Arc<AtomicBool>` | GUI → audio: "Reset to Default" requests a `Pipeline::reset()`; audio thread clears it with `swap(false)` |
| `spectrum_tx` / `spectrum_rx` | `triple_buffer::Input<Vec<f32>>` / `Arc<Mutex<Output<…>>>` | Audio → GUI spectrum |
| `suppression_tx` / `suppression_rx` | same as above | Audio → GUI per-bin suppression (gain reduction) |
| `sc_envelope_tx` / `sc_envelope_rx` | same as above | Audio → GUI sidechain peak-hold envelope for the Gain slot being edited |
| `sample_rate` | `Arc<AtomicF32>` | Current sample rate; `AtomicF32` is a bit-cast wrapper over `AtomicU32` defined in `bridge.rs` |
| `ring_states` | `Arc<Mutex<RingStateBank>>` | GUI → audio modulation ring state; GUI writes on click, audio thread `try_lock()`s once per block and copies the 378-entry bank (skips on contention) |

The GUI-side `Mutex`es wrap the reader/writer handles; the audio thread only
touches the `*_tx` inputs, `curve_rx`, the atomics, and `ring_states` via `try_lock()`.

### 3.4 Pipeline — `src/dsp/pipeline.rs`

Owns one `StftHelper` (realfft overlap-add, hop = FFT/4), one `sc_stft` for the
single stereo sidechain input, and an `FxMatrix` that owns the 9 slot modules
(`Box<dyn SpectralModule>`).  Each block:

1. Reads curve caches from triple-buffer; applies tilt/offset.
2. Runs STFT on the main signal.
3. For each slot: `FxMatrix::process_hop` calls `module.process()` with bins,
   curves, sidechain, suppression output, optional `BinPhysics`, and `ModuleContext`.
4. Writes spectrum and suppression to their triple-buffers for the GUI.

**SIMD dispatch**: per-bin gain application in `SpectralCompressorEngine` uses
runtime multiversion dispatch (`multiversion` crate) for AVX2+FMA / SSE4.1 / scalar
paths.

### 3.5 Modules — `src/dsp/modules/`

Each file implements one `SpectralModule`.  The core required methods:

```rust
fn process(
    &mut self,
    channel: usize,
    stereo_link: StereoLink,
    target: FxChannelTarget,
    bins: &mut [Complex<f32>],
    sidechain: Option<&[f32]>,
    curves: &[&[f32]],                // length = num_curves()
    suppression_out: &mut [f32],
    physics: Option<&mut BinPhysics>, // Some for BinPhysics writers
    ctx: &ModuleContext<'_>,
);
fn reset(&mut self, sample_rate: f32, fft_size: usize);
fn tail_length(&self) -> u32 { 0 }    // in samples
fn module_type(&self) -> ModuleType;
fn num_curves(&self) -> usize;
```

Modules must not allocate and must fill `suppression_out` with finite non-negative
values.  `physics` is `Some` only for modules that declare
`ModuleSpec.writes_bin_physics = true`; readers access physics via `ctx.bin_physics`.
Slots (audio and physics alike) run strictly in numerical slot order; there is no
writer-before-reader scheduling.  To feed a reader from a writer's output, place the
writer at a lower slot index.

`BinParams<'_>` is **not** the `SpectralModule` interface: it is the lower-level struct
of per-bin threshold/ratio/attack/release/knee/makeup/mix slices taken by
`SpectralEngine::process_bins()` (see §3.6), used internally by `DynamicsModule` and
`ContrastModule`.

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
| `NUM_NODES` | 6 per curve | `params.rs` and `param_ids.rs` (also mirrored in `build.rs`) |
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

# Run all tests (600+ tests across 68 files)
cargo test

# Include the probe-gated tests (calibration, calibration_roundtrip,
# bin_physics_pipeline, circuit)
cargo test --features probe
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
| `nih_plug` | Plugin framework (CLAP, VST3); git dependency, `assert_process_allocs` feature |
| `nih_plug_egui` | egui integration for nih-plug; patched to the local `vendor/` copy (see below) |
| `realfft` | Real-valued FFT (in-place, no allocation after init) |
| `triple_buffer` | Lock-free single-producer/single-consumer GUI↔audio bridge |
| `parking_lot` | Fast `Mutex` for GUI-side locks |
| `num-complex` | Complex number type for FFT bins |
| `multiversion` | Runtime SIMD dispatch (AVX2+FMA, SSE4.1, scalar) |
| `serde` / `serde_json` | Preset and curve-node serialisation |
| `directories` | Platform config directory for user preset files (`preset::preset_dir()`) |
| `opener` | Declared in `Cargo.toml`; currently no call sites in `src/` |
| `smallvec` | Stack-allocated small vectors for RT-safe peak lists in the Kinetics module |
| `approx` (dev) | Approximate floating-point comparisons in tests |
| `tempfile` (dev) | Temporary directories for preset round-trip tests |

**Vendored `nih_plug_egui`**: `Cargo.toml` has a `[patch."https://github.com/robbert-vdh/nih-plug.git"]`
entry that replaces `nih_plug_egui` with `vendor/nih_plug_egui` (added in commit
`3badedc`, April 21, 2026).  The patch fixes a resize-down bug: upstream consumed
`requested_size` before calling `request_resize()`, so the host re-read a stale size
and left black borders when the UI scale shrank.  The vendored `editor.rs` stores the
new size before requesting the resize and reverts it if the host refuses.  It also
relies on `EguiState::set_requested_size()` being `pub`, which `editor_ui.rs` calls for
user-controlled UI scaling.  Re-check the patch whenever the upstream nih-plug revision
is bumped.

---

## 8. Design Decisions & Constraints

- **Patent-safe**: avoids the oeksound Hilbert/convolution approach (the reference
  patent copy is kept outside the repository).  The per-bin compressor is a straightforward envelope follower applied bin-by-bin.
- **Linux and Windows**: primary host is Bitwig Studio on Linux (ALSA/PipeWire);
  Windows is also supported.  macOS is not a primary target (a collaborator provides
  signed Mac binaries separately).  Both CLAP and VST3 are exported
  (`nih_export_clap!` + `nih_export_vst3!`).
- **Single crate**: both the `cdylib` plugin and the `rlib` test target share one
  `Cargo.toml`.
- **Host automation**: `build.rs` generates 1440 automatable `FloatParam` fields
  (1134 curve-node x/y/q + 126 per-curve tilt/offset + 63 curvature + 117 matrix
  sends, 9 destinations × 13 sources), plus 243 per-slot module scalars (Past,
  Life, Kinetics, Circuit, Modulate, Contrast; 1683 generated fields in all),
  alongside the hand-written global knobs.  Slot module types and per-slot mode selectors remain
  `#[persist]` GUI state and are not automatable; legacy `slot_curve_nodes` /
  `route_matrix` are still persisted and migrated into the generated params on load.

---

## 9. Future Considerations

- Per-slot frequency scaling (curve tilt vs. true freq-dependent A/R).
- Proper CI with `cargo test` on push.
- JSON-based theme loading for runtime reskinning (see `GUI.md`).
- Window resize via host API (currently via `EguiState::set_requested_size()` in the
  vendored `nih_plug_egui`; upstreaming the resize fix would let the `vendor/` patch go).

---

## 10. Project Identification

**Project name**: Spectral Forge  
**Version**: 0.15.x (see `Cargo.toml`)  
**Platform**: Linux + Windows, CLAP + VST3  
**Primary AI guide**: `CLAUDE.md`  
**Date of last update**: 2026-09-29

---

## 11. Glossary

| Term | Definition |
|------|-----------|
| BinParams | Struct of per-bin slices (threshold, ratio, attack, release, knee, makeup, mix) passed to `SpectralEngine::process_bins()`. Used internally by DynamicsModule and ContrastModule; not the `SpectralModule::process()` interface |
| BinPhysics | Per-bin physics state carried between slots; writer modules receive it as `physics`, reader modules read it via `ModuleContext::bin_physics` |
| CLAP | CLever Audio Plugin format — the primary plugin format target |
| Curve | One of up to 7 per-slot parameter shapes drawn by the user; labels are module-specific (`ModuleSpec.curve_labels`) |
| DSP | Digital Signal Processing — refers to the audio-thread code in `src/dsp/` |
| FFT | Fast Fourier Transform — converts time-domain audio blocks to per-bin frequency data |
| Module | A `SpectralModule` implementation assigned to one of 9 slots in the routing matrix |
| Node | A control point in a drawn parameter curve (`CurveNode`: x=frequency, y=gain, q=bandwidth) |
| OLA | Overlap-Add — the STFT reconstruction method; 75% overlap (hop = FFT/4) |
| Slot | One of 9 parallel processing lanes; each has a module type, 7 curves, and routing connections |
| Triple-buffer | Lock-free single-writer / single-reader buffer used for GUI→audio (curves) and audio→GUI (spectrum) |
| T/S Split | Transient/Sustained Split module — splits the signal into transient and tonal components across bins |
