# Spectral Forge — AI Assistant Guide

This document is for AI assistants (Claude, etc.) working on this codebase.

## UI Parameter Specification — READ BEFORE TOUCHING DISPLAY CODE

**`docs/superpowers/specs/2026-04-23-ui-parameter-spec-design.md` is the authoritative source
of truth for all curve display behaviour**: axis ranges, grid lines, unit labels, offset/tilt/
curvature transforms, hover text, and UI scaling rules. Any work touching these areas MUST
follow that spec exactly. If the spec is unclear or a situation arises where following it would
cause a problem, STOP and ask rather than guessing or improvising.

## What it is

A **spectral dynamics and modular multi-fx** CLAP plugin for Linux/Windows, written in Rust.
It runs audio through a per-FFT-bin processing pipeline with up to 9 independently configurable
slots, each driven by up to 7 drawn parameter curves. Slots are routed through a matrix and
can hold different module types (compressor, freeze, phase smear, contrast, gain, mid/side, etc.).

Target: Linux and Windows only. Primary host: Bitwig Studio.
Addition: A collaborator provides signed Mac binaries.
Patent-safe design — does not use the oeksound-patented Hilbert/convolution approach.

## Build

```bash
# Debug (fast compile, slow)
cargo build

# Release (optimised, what you want to profile)
cargo build --release

# Bundle as .clap file (installs go to target/bundled/)
cargo run --package xtask -- bundle spectral_forge --release

# Install to Bitwig's search path
cp target/bundled/spectral_forge.clap ~/.clap/
```

## Test

```bash
cargo test          # 28 tests across 5 test files
cargo test engine   # engine_contract tests only
cargo test stft     # stft_roundtrip only
cargo test module   # module_trait tests only
```

Test files live in `tests/`. They use the library crate (`rlib` target) — the `crate-type` in Cargo.toml includes both `cdylib` (the plugin) and `rlib` (for tests).

## Architecture overview

```
src/
  lib.rs              — Plugin entry point: Plugin/ClapPlugin impl, initialize/reset/process
  params.rs           — All nih-plug Params: floats, bools, enums, persisted slot state
  bridge.rs           — SharedState: triple-buffer channels GUI↔Audio (9 slots × 7 curves),
                        AtomicF32, sidechain_active flags
  editor_ui.rs        — create_editor(): top-level egui frame, assembles all widgets
  editor/
    curve.rs          — CurveNode, compute_curve_response(), curve_widget(), paint_response_curve()
    spectrum_display.rs — pre/post-FX spectrum gradient painter
    suppression_display.rs — legacy gain-reduction stalactite display (kept for reference)
    fx_matrix_grid.rs — 9×9 slot routing matrix widget
    module_popup.rs   — right-click module assignment popup
    theme.rs          — ALL visual constants (colours, sizes). Edit only here.
    mod.rs            — pub use
  dsp/
    pipeline.rs       — Pipeline: variable-FFT STFT overlap-add, M/S encode, 4-aux sidechain
                        STFT, slot curve application, delta monitor, FxMatrix call
    fx_matrix.rs      — FxMatrix: RouteMatrix-driven slot dispatch and inter-slot mixing
    guard.rs          — flush_denormals(), sanitize() (clamp NaN/Inf before FFT)
    utils.rs          — shared DSP helpers (linear_to_db, etc.)
    engines/
      mod.rs          — SpectralEngine trait + BinParams<'_> struct + EngineSelection enum
      spectral_compressor.rs — envelope → gain_computer → smoother → apply
      spectral_contrast.rs   — contrast/transient engine
    modules/
      mod.rs          — ModuleType enum, ModuleSpec, SpectralModule trait,
                        apply_curve_transform(), create_module(), RouteMatrix
      dynamics.rs     — Compressor/expander (wraps SpectralCompressorEngine)
      freeze.rs       — Spectral freeze
      phase_smear.rs  — Phase randomisation
      contrast.rs     — Spectral contrast
      gain.rs         — Per-bin gain shaping (Add / Subtract / Pull modes)
      mid_side.rs     — M/S balance, expansion, phase decorrelation
      ts_split.rs     — Transient/Sustained split
      harmonic.rs     — Harmonic emphasis
      master.rs       — Master output slot + EmptyModule passthrough
```

## Key constants

```rust
// Variable FFT — chosen by user at runtime
MAX_FFT_SIZE = 16384                          // largest supported FFT
MAX_NUM_BINS = MAX_FFT_SIZE / 2 + 1  // 8193

// Default FFT (FftSizeChoice::S2048)
default FFT_SIZE = 2048
default NUM_BINS = 1025

OVERLAP  = 4                          // 75% overlap, hop = fft_size / 4
NORM     = 2.0 / (3.0 * fft_size)    // Hann² OLA normalisation (varies with fft_size)

NUM_CURVE_SETS = 7                    // curves per slot: threshold, ratio, attack, release,
                                      // knee, makeup (unused by Dynamics), mix
NUM_NODES      = 6                    // nodes per curve (0,5 = shelves; 1-4 = bells)
NUM_SLOTS      = 9                    // slots 0–7 are user modules; slot 8 = Master
```

## The slot and curve system

Each of the 9 slots has its own set of 7 curve channels in the bridge (`curve_rx[slot][curve]`).
The pipeline reads all 9×7 curves each block and stores them in `slot_curve_cache[slot][curve][bin]`.

Curves map linear gain values (1.0 = neutral) to physical units. The Dynamics module uses 6 of the 7:

| Index | Name       | 1.0 maps to          | Range          | Used by Dynamics |
|-------|------------|----------------------|----------------|------------------|
| 0     | THRESHOLD  | -20 dBFS             | -60 … 0 dBFS   | yes              |
| 1     | RATIO      | 1:1 (no compression) | 1:1 … 20:1     | yes              |
| 2     | ATTACK     | global attack × 1    | 0.1 … 500 ms   | yes              |
| 3     | RELEASE    | global release × 1   | 1 … 2000 ms    | yes              |
| 4     | KNEE       | 6 dB soft knee       | 0 … 24 dB      | yes              |
| 5     | MAKEUP     | (always 0.0 in Dyn)  | —              | no (Gain module) |
| 6     | MIX        | 100% wet             | 0 … 100%       | yes              |

Each module type has its own `ModuleSpec` listing which curve labels it uses; `num_curves()` for a
Dynamics slot is 6, for PhaseSmear 2, for Freeze 4, etc.

**Tilt/offset/curvature transforms** (`apply_curve_transform`) are applied on top of the raw
curve per-block. They are per-slot/per-curve FloatParams: `s{s}c{c}_tilt`, `s{s}c{c}_offset`,
`s{s}c{c}curv`. The `CurveTransform` struct in `dsp::modules` and `params.curve_transform(s, c)`
give a snapshot helper for GUI callers.

## Data flow

```
GUI curve editor → compute_curve_response() → curve_tx[slot][curve] (triple_buffer)
                                                              ↓
                                                   Pipeline::process()
                                                     ├─ curve_rx[slot][curve].read()
                                                     │    apply_curve_transform(tilt, offset)
                                                     │    → slot_curve_cache[slot][curve]
                                                     ├─ sc_stfts[0..4] for aux sidechains
                                                     ├─ STFT overlap-add (realfft)
                                                     │    FxMatrix::process_hop(channel, bins,
                                                     │        sc_args, slot_targets,
                                                     │        slot_curve_cache, route_matrix, ctx)
                                                     │      for each slot (RouteMatrix.send order):
                                                     │        SpectralModule::process(bins, curves,
                                                     │            sidechain, suppression_out)
                                                     ├─ spectrum_tx.publish()
                                                     └─ suppression_tx.publish()
                                                              ↓
                                                   GUI spectrum/suppression display
```

## Real-time safety rules (NEVER break these)

- **No allocation on the audio thread.** `Vec::clone()`, `Vec::new()`, `collect()` are all forbidden inside `Pipeline::process()`, `FxMatrix::process_hop()`, and any `SpectralModule::process()`. Use pre-allocated buffers.
- **No locking on the audio thread.** Use `try_lock()` only in the GUI thread. Audio reads curves via lock-free triple-buffer (`curve_rx[s][c].read()`). `slot_curve_meta`, `slot_targets`, `slot_sidechain`, `route_matrix`, and `slot_gain_mode` use `try_lock()` with a fallback so they never block.
- **No I/O on the audio thread.** No file access, no `println!`.
- `assert_process_allocs` feature is enabled in Cargo.toml — it will abort if the audio thread allocates.
- The `guard::flush_denormals()` call at the top of `process()` sets FTZ+DAZ CPU flags each block to prevent denormal slowdowns.

## CurveNode coordinate system

```
x: 0.0 = 20 Hz, 1.0 = 20 kHz  (log-linear: freq = 20 * 1000^x)
y: -1.0 = -18 dB, 0.0 = neutral, +1.0 = +18 dB
q: 0.0 = 4 octaves bandwidth, 1.0 = 0.1 octave bandwidth  (0.1 * 40^q octaves)
```

Nodes 0 and 5 are shelves (low/high). Nodes 1–4 are Gaussian bells.
`compute_curve_response()` returns a `Vec<f32>` of linear multipliers, one per FFT bin.

## BinParams<'_>

Used internally by `SpectralEngine` implementations. All slices are `num_bins` long.
`process_bins()` must not allocate and must fill `suppression_out` completely with
non-negative finite values (NaN sentinel tested in `engine_contract.rs`).

## SpectralModule trait

The `SpectralModule` trait in `src/dsp/modules/mod.rs` is the top-level interface for slot processing:

```rust
pub trait SpectralModule: Send {
    fn process(
        &mut self,
        channel: usize,
        stereo_link: StereoLink,
        target: FxChannelTarget,
        bins: &mut [Complex<f32>],
        sidechain: Option<&[f32]>,
        curves: &[&[f32]],       // slice of length num_curves()
        suppression_out: &mut [f32],
        ctx: &ModuleContext,
    );

    fn reset(&mut self, sample_rate: f32, fft_size: usize);
    fn tail_length(&self) -> u32 { 0 }
    fn module_type(&self) -> ModuleType;
    fn num_curves(&self) -> usize;
    fn set_gain_mode(&mut self, _: GainMode) {}   // no-op unless module uses it
}
```

`ModuleContext` carries `sample_rate`, `fft_size`, `num_bins`, `attack_ms`, `release_ms`,
`sensitivity`, `suppression_width`, `auto_makeup`, and `delta_monitor`. All are `Copy` —
it is assembled in `Pipeline::process()` and passed by reference.

`FxChannelTarget` (`All` / `Mid` / `Side`) gates whether the slot processes the current channel
in MidSide mode. Modules handle this by checking target vs. channel inside `process()`.

## FxMatrix and RouteMatrix

`FxMatrix` (`src/dsp/fx_matrix.rs`) holds the 9 `Option<Box<dyn SpectralModule>>` slots and
pre-allocated per-slot output buffers. `process_hop()` iterates slots 0–7 (slot 8 = Master is
handled separately), assembles each slot's input from the route matrix, dispatches to the module,
then routes the output.

`RouteMatrix` (`src/dsp/modules/mod.rs`) is a plain struct of `[[f32; MAX_SLOTS]; MAX_MATRIX_ROWS]`
send amplitudes. Default serial wiring: slot 0 → 1 → 2 → Master (slot 8). Off-diagonal cells set
send amplitude between any two slots; `virtual_rows` support T/S split outputs.

Both are cheaply cloned from params each block (`route_matrix_snap`) to avoid holding the lock
across the STFT closure.

## Triple-buffer protocol

```rust
// GUI → Audio (write side, from GUI thread, per slot/curve):
let mut tx = shared.curve_tx[slot][curve].try_lock().unwrap();
tx.input_buffer_mut().copy_from_slice(&gains);
tx.publish();

// Audio → GUI (write side, from audio thread — no lock needed on TbInput):
shared.spectrum_tx.input_buffer_mut().copy_from_slice(&spectrum_buf[..MAX_NUM_BINS]);
shared.spectrum_tx.publish();

// GUI read side (try_lock to avoid blocking):
if let Some(mut rx) = spectrum_rx.try_lock() {
    paint_spectrum(painter, rect, rx.read());
}
```

## Stereo modes (`params.stereo_link`)

Stereo mode is handled at the **Pipeline** level, not inside individual modules:

- **Linked** (default): single STFT call processes both channels with the same slot chain.
- **Independent**: each channel runs through the same slots; modules track per-channel state
  (e.g. `DynamicsModule` has `engine` for ch0 and `engine_r` for ch1).
- **MidSide**: L/R → M/S (FRAC_1_SQRT_2 matrix) **before** the STFT closure, decode **after**.
  The slot chain sees M on channel 0 and S on channel 1. `FxChannelTarget::Mid/Side` gates
  which slots process which component.

## Variable FFT size

The user can change FFT size at runtime via `FftSizeChoice` (512 / 1024 / 2048 / 4096 / 8192 / 16384).
`Pipeline::new()` and `Pipeline::reset()` accept `fft_size` as a parameter. All inner buffers are
allocated at `MAX_NUM_BINS` so no reallocation is needed on change. The active bin count
(`num_bins = fft_size / 2 + 1`) is passed through to every module and engine call.

Latency reported to the host = `fft_size` samples. Bitwig compensates automatically.

## Adding a new SpectralModule

1. Add a variant to `ModuleType` in `modules/mod.rs`.
2. Add a `ModuleSpec` entry in `module_spec()` with display name, colours, and `curve_labels`.
3. Create a new file in `src/dsp/modules/`, implement `SpectralModule`.
4. Wire the variant in `create_module()`.
5. Write at least one test in `tests/module_trait.rs` covering the new type.
6. Override `tail_length()` if the module holds state beyond one FFT window (e.g. Freeze).
7. Implement `set_gain_mode()` if the module has Add/Subtract/Pull gain behaviour.

The module receives `curves: &[&[f32]]` of length `num_curves()`. Index 0 is curve 0,
etc. — **never index beyond `num_curves()`**. The curve-to-parameter mapping is the module's
own responsibility; see existing modules for the pattern.

## Gotchas

- `StftHelper::process_overlap_add()` takes `&mut self` inside a closure. Rebind all `self.*` fields as locals before the call to avoid conflicting borrows. See pipeline.rs for the established pattern.
- `triple_buffer::Output::read()` takes `&mut self` — each call must be a separate statement.
- `slot_curve_cache` in `Pipeline` is `Vec<Vec<Vec<f32>>>` (9 slots × 7 curves × MAX_NUM_BINS). Only `[0..num_bins]` is valid for the current FFT size.
- `FxMatrix::process_hop` temporarily `take()`s each slot out of `self.slots[s]` to avoid a simultaneous borrow of `slots` and `slot_out`. Always `put` it back unconditionally.
- The 4 aux sidechain STFTs (`sc_stfts[0..4]`) are separate `StftHelper` instances. Each is indexed by the same `i` used in `slot_sidechain` params.
- All visual constants live in `editor/theme.rs` — do not hardcode colours or sizes elsewhere.
- `assert_eq!(m.num_curves(), module_spec(ty).num_curves)` is debug-asserted in `create_module()` — keep these in sync when adding a new module.
