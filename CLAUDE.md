# Spectral Forge — AI Assistant Guide

This document is for AI assistants (Claude, etc.) working on this codebase.

## Planning docs — CHECK STATUS BEFORE FOLLOWING ANY PLAN

`docs/superpowers/` holds historical and current design specs and implementation plans.
**Several are SUPERSEDED or DEFERRED** and must not be followed as written. Before relying
on any file under `docs/superpowers/plans/` or `docs/superpowers/specs/`, consult
[`docs/superpowers/STATUS.md`](docs/superpowers/STATUS.md) for its current status. Every
individual plan/spec also carries a status banner at the top; if the banner and the STATUS
index disagree, the STATUS index wins.

Rule of thumb: **the code is the source of truth for anything marked IMPLEMENTED.** Plans
are kept only for history.

## UI Parameter Specification — READ BEFORE TOUCHING DISPLAY CODE

**`docs/superpowers/specs/2026-04-23-ui-parameter-spec-design.md` is the authoritative source
of truth for all curve display behaviour**: axis ranges, grid lines, unit labels, offset/tilt/
curvature transforms, hover text, and UI scaling rules. Any work touching these areas MUST
follow that spec exactly. If the spec is unclear or a situation arises where following it would
cause a problem, STOP and ask rather than guessing or improvising.

## Phase Handling — READ BEFORE TOUCHING PLPV / FREEZE / MODULATE / PHASE-SMEAR

**`docs/superpowers/specs/2026-05-06-phase-handling.md` is the authoritative reference for
all spectral-phase operations**: the wrapped/unwrapped domains, the per-hop wrap invariant
(every accumulator stays in `(-π, π]` after each update), the complex-blend rule for
geodesic phase mixing, the unwrap kernel contract, and a checklist for new phase code.
The pvx Python reference at `repos/pvx/src/pvx/core/voc.py` is cross-linked. Do not
periodic-reset accumulators (audible discontinuity). Do not linear-blend audible phases.

## What it is

A **spectral dynamics and modular multi-fx** plugin (exported as both CLAP and VST3) for Linux/Windows, written in Rust.
It runs audio through a per-FFT-bin processing pipeline with up to 9 independently configurable
slots, each driven by up to 7 drawn parameter curves. Slots are routed through a matrix and
can hold different module types: Dynamics, Freeze, Phase Smear, Contrast, Gain, Mid/Side,
T/S Split, Harmonic, Future, Punch, Rhythm, Geometry, Modulate, Circuit, Life, Past, Kinetics,
Harmony (+ Master and Empty).

Target: Linux and Windows only. Primary host: Bitwig Studio.
Addition: A collaborator provides signed Mac binaries.
Patent-safe design — does not use the oeksound-patented Hilbert/convolution approach.

## Build

```bash
# Debug (fast compile, slow)
cargo build

# Release (optimised, what you want to profile)
cargo build --release

# Bundle as .clap + .vst3 (installs go to target/bundled/)
cargo run --package xtask -- bundle spectral_forge --release

# Install to Bitwig's search path
cp target/bundled/spectral_forge.clap ~/.clap/
```

## Test

```bash
cargo test                   # default suite (68 test files in tests/ + unit tests in src/)
cargo test --features probe  # also builds the probe-gated tests (600+ tests in total)
cargo test --test engine_contract   # tests/engine_contract.rs only
cargo test --test stft_roundtrip    # tests/stft_roundtrip.rs only
cargo test --test module_trait      # tests/module_trait.rs only
cargo test --features probe --test calibration_roundtrip  # probe-gated binaries need the feature
```

Test files live in `tests/`. They use the library crate (`rlib` target) — the `crate-type` in Cargo.toml includes both `cdylib` (the plugin) and `rlib` (for tests).
Four test binaries declare `required-features = ["probe"]` in Cargo.toml (`calibration`,
`calibration_roundtrip`, `bin_physics_pipeline`, `circuit`) and are skipped without
`--features probe`; several other files also gate individual tests on that feature.

## Architecture overview

```
src/
  lib.rs              — Plugin entry point: Plugin/ClapPlugin/Vst3Plugin impls, initialize/reset/process
  params.rs           — All nih-plug Params: floats, bools, enums, persisted slot state;
                        also include!s build.rs-generated GeneratedParams (1683 fields): the
                        1440-param automation grid (1134 graph-node + 126 tilt/offset + 63 curvature + 117 matrix;
                        asserted in tests/param_grid.rs) plus 243 per-slot module scalars
                        (Past, Life, Kinetics, Circuit, Modulate, Contrast; see build.rs header)
  param_ids.rs        — Stable string IDs for the generated automation params
  bridge.rs           — SharedState: triple-buffer channels GUI↔Audio (9 slots × 7 curves),
                        AtomicF32 sample_rate, sidechain_active AtomicBool, ring_states
  editor_ui.rs        — create_editor(): top-level egui frame, assembles all widgets
  preset.rs           — User preset file format, preset directory, load/save
  presets.rs          — Built-in factory presets (PluginState snapshots)
  editor/
    curve.rs          — CurveNode, compute_curve_response(), curve_widget(), paint_response_curve()
    curve_config.rs   — CurveDisplayConfig: per-module/per-curve display properties
    spectrum_display.rs — pre/post-FX spectrum gradient painter
    fx_matrix_grid.rs — 9×9 slot routing matrix widget
    module_popup.rs   — right-click module assignment popup
    amp_popup.rs      — per-cell AmpMode popup for the routing matrix
    help_box.rs       — help-box widget rendered right of the FX matrix
    mod_ring.rs       — Modulation Ring overlay (S/H, Sync, Legato)
    preset_menu.rs    — preset menu widget
    *_panel.rs, *_popup.rs — per-module panels and mode popups (past, rhythm, life,
                        kinetics, circuit, modulate, contrast, harmony); several panels are
                        compiled only with the `dev-build` feature
    theme.rs          — ALL visual constants (colours, sizes). Edit only here.
    mod.rs            — module declarations, pub use
  dsp/
    pipeline.rs       — Pipeline: variable-FFT STFT overlap-add, M/S encode, single stereo
                        sidechain STFT, slot curve application, delta monitor, FxMatrix call
    fx_matrix.rs      — FxMatrix: RouteMatrix-driven slot dispatch and inter-slot mixing
    guard.rs          — flush_denormals(), sanitize() (clamp NaN/Inf before FFT)
    utils.rs          — shared DSP helpers (linear_to_db, etc.)
    bin_physics.rs    — BinPhysics per-bin state carrier (mass, temperature, flux, …) + MergeRule
    plpv.rs           — PLPV kernels: Laroche-Dolson phase unwrap, peak detection, Voronoi skirts
    history_buffer.rs — Per-channel ring of past complex FFT frames (read by Past via ctx.history)
    instantaneous_freq.rs — Per-bin instantaneous frequency (phase-vocoder deviation formula)
    cepstrum.rs       — Lazy cepstrum (log-magnitude → inverse real FFT)
    chromagram.rs     — IF-refined 12-bin pitch-class profile
    harmonic_groups.rs — Greedy fundamental-first harmonic-group detection
    midi.rs           — Allocation-free MIDI held-note bookkeeping
    modulation_ring.rs — RingStateBank + S/H / Sync16 / Legato curve modulation
    phase.rs          — PhaseRotator (1024-entry sin/cos LUT)
    physics_helpers.rs — CFL clamp, damping floor, energy-rise hysteresis, wrap_phase, PLL bank step
    amp_modes.rs      — AmpMode kernels for matrix cells (Linear / Vactrol / Schmitt / Slew / Stiction)
    circuit_kernels.rs — Circuit scalar primitives (lp_step, tanh_levien_poly, spread_3tap, SimdRng)
    soft_clip.rs      — Master output soft clipper
    engines/
      mod.rs          — SpectralEngine trait + BinParams<'_> struct + EngineSelection enum
      spectral_compressor.rs — envelope → gain_computer → smoother → apply
      spectral_contrast.rs   — contrast engine (Spatial / Temporal / Tilt kernels)
    modules/
      mod.rs          — ModuleType enum, ModuleSpec, SpectralModule trait,
                        apply_curve_transform(), create_module(), RouteMatrix
      dynamics.rs     — Compressor/expander (wraps SpectralCompressorEngine)
      freeze.rs       — Spectral freeze
      phase_smear.rs  — Phase randomisation
      contrast.rs     — Spectral contrast (Spatial / Temporal / Tilt modes)
      gain.rs         — Per-bin gain shaping (Add / Subtract / Pull / Match modes)
      mid_side.rs     — M/S balance, expansion, phase decorrelation
      ts_split.rs     — Transient/Sustained split (exposes T and S virtual rows)
      harmonic.rs     — Harmonic placeholder stub (no DSP; 0 curves)
      future.rs       — Print-Through / Pre-Echo
      punch.rs        — Sidechain-driven peak carving (Direct / Inverse)
      rhythm.rs       — Tempo-synced Euclidean / Arpeggiator / Phase Reset
      geometry.rs     — Chladni Plate Nodes / Helmholtz Traps
      modulate.rs     — 8 modes: Phase Phaser, Bin Swapper, RM/FM Matrix, Diode RM, Ground Loop,
                        Gravity Phaser, PLL Tear, FM Network
      circuit.rs      — 10 analog-circuit modes: BBD, Schmitt, Crossover, Vactrol, Transformer
                        Saturation, Power Sag, Component Drift, PCB Crosstalk, Slew Distortion,
                        Bias Fuzz
      life.rs         — 10 physical-metaphor modes: Viscosity, Surface Tension, Crystallization,
                        Archimedes, Non-Newtonian, Stiction, Yield, Capillary, Sandpaper, Brownian
      past.rs         — History-buffer consumer: Granular, Decay Sorter, Convolution, Reverse,
                        Stretch
      kinetics.rs     — 8 physical-force modes: Hooke, Gravity Well, Inertial Mass, Orbital Phase,
                        Ferromagnetism, Thermal Expansion, Tuning Fork, Diamagnet
      harmony.rs      — 8 modes: Chordification, Undertone, Companding, Formant Rotation, Lifter,
                        Inharmonic, Harmonic Generator, Shuffler
      harmony_helpers.rs — Harmony helpers: peak picking, Bessel-zero / prime LUTs, chord templates
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

NUM_CURVE_SETS = 7                    // curve channels per slot (bridge capacity);
                                      // each module uses the first num_curves() of them
NUM_NODES      = 6                    // nodes per curve (0,5 = shelves; 1-4 = bells)
NUM_SLOTS      = 9                    // slots 0–7 are user modules; slot 8 = Master
```

## The slot and curve system

Each of the 9 slots has its own set of 7 curve channels in the bridge (`curve_rx[slot][curve]`).
The pipeline reads all 9×7 curves each block and stores them in `slot_curve_cache[slot][curve][bin]`.

Curves map linear gain values (1.0 = neutral) to physical units. Each module type defines its
own curve mapping via `ModuleSpec::curve_labels`; the bridge always allocates 7 channels per slot
and a module reads only its first `num_curves()` entries. The Dynamics module uses 6 curves
(MAKEUP, formerly curve 5, is now the standalone Gain module):

| Index | Name       | 1.0 maps to          | Range           |
|-------|------------|----------------------|-----------------|
| 0     | THRESHOLD  | -20 dBFS             | -120 … +24 dBFS |
| 1     | RATIO      | 1:1 (no compression) | 1:1 … 20:1      |
| 2     | ATTACK     | global attack × 1    | 0.1 … 500 ms    |
| 3     | RELEASE    | global release × 1   | 1 … 2000 ms     |
| 4     | KNEE       | 6 dB soft knee       | 0 … 24 dB       |
| 5     | MIX        | 100% wet             | 0 … 100%        |

Other modules define their own indices — do not assume the Dynamics mapping elsewhere. Current
`num_curves()`: Dynamics 6, Freeze 5, PhaseSmear 4, Contrast 6, Gain 2, MidSide 5, T/S Split 2,
Harmonic 0, Future 5, Punch 6, Rhythm 5, Geometry 5, Modulate 6, Circuit 5, Life 5, Past 5,
Kinetics 5, Harmony 6 (Master and Empty 0). `module_spec()` in `src/dsp/modules/mod.rs` is the
authoritative source for counts and labels.

**Tilt/offset/curvature transforms** (`apply_curve_transform`) are applied on top of the raw
curve per-block. They are per-slot/per-curve FloatParams: `s{s}c{c}tilt`, `s{s}c{c}offset`,
`s{s}c{c}curv` (no underscore; formatted by `src/param_ids.rs`, stable forever). The `CurveTransform` struct in `dsp::modules` and `params.curve_transform(s, c)`
give a snapshot helper for GUI callers.

## Data flow

```
GUI curve editor → compute_curve_response() → curve_tx[slot][curve] (triple_buffer)
                                                              ↓
                                                   Pipeline::process()
                                                     ├─ curve_rx[slot][curve].read()
                                                     │    apply_curve_transform(tilt, offset)
                                                     │    → slot_curve_cache[slot][curve]
                                                     ├─ sc_stft (one stereo SC input → L/R/LR/M/S envelopes)
                                                     ├─ STFT overlap-add (realfft)
                                                     │    FxMatrix::process_hop(channel, bins,
                                                     │        sc_args, slot_targets,
                                                     │        slot_curve_cache, route_matrix, ctx)
                                                     │      for each slot 0–7 (numerical order):
                                                     │        SpectralModule::process(channel,
                                                     │            stereo_link, target, bins,
                                                     │            sidechain, curves,
                                                     │            suppression_out, physics, ctx)
                                                     ├─ spectrum_tx.publish()
                                                     └─ suppression_tx.publish()
                                                              ↓
                                                   GUI spectrum/suppression display
```

## Real-time safety rules (NEVER break these)

- **No allocation on the audio thread.** `Vec::clone()`, `Vec::new()`, `collect()` are all forbidden inside `Pipeline::process()`, `FxMatrix::process_hop()`, and any `SpectralModule::process()`. Use pre-allocated buffers.
- **No blocking locks on the audio thread.** Never call `lock()` there. Audio reads curves via lock-free triple-buffer (`curve_rx[s][c].read()`). Mutex-guarded persisted state in `params.rs` (e.g. `slot_module_types`, `slot_targets`, `slot_sc_channel`, `slot_sc_gain_db`, `slot_curve_nodes`, `route_matrix`, `slot_gain_mode` and the other `slot_<type>_mode` arrays) plus `shared.ring_states` is read in `Pipeline::process()` only via `try_lock()` with a fallback (previous/default value, or skip for this block) so it never blocks.
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

## SpectralEngine and BinParams<'_>

`SpectralEngine` (`src/dsp/engines/mod.rs`) is a lower-level trait than `SpectralModule`, used
internally by `DynamicsModule` and `ContrastModule`. Its `process_bins(bins, sidechain, params,
sample_rate, suppression_out)` takes a `BinParams<'_>` struct of per-bin slices (threshold, ratio,
attack, release, knee, makeup, mix) plus scalars. All slices are `num_bins` long.
`process_bins()` must not allocate and must fill `suppression_out` completely with
non-negative finite values (NaN sentinel tested in `engine_contract.rs`). You do not touch this
trait when adding a new `SpectralModule`.

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
        physics: Option<&mut BinPhysics>,  // Some for writers (ModuleSpec.writes_bin_physics)
        ctx: &ModuleContext<'_>,
    );

    fn reset(&mut self, sample_rate: f32, fft_size: usize);
    fn tail_length(&self) -> u32 { 0 }
    fn module_type(&self) -> ModuleType;
    fn num_curves(&self) -> usize;
    fn num_outputs(&self) -> Option<usize> { None }
    fn heavy_cpu_for_mode(&self) -> bool { false }
    fn set_gain_mode(&mut self, _: GainMode) {}   // no-op unless module uses it
    // ...many more default-no-op per-module setters (set_future_mode, set_past_mode,
    // set_life_scalars, ...); see src/dsp/modules/mod.rs for the full list.
    fn clear_state(&mut self) {}   // zero DSP state on Reset; audio thread, no alloc/lock/I/O
    fn virtual_outputs(&self) -> Option<[&[Complex<f32>]; 2]> { None }  // T/S Split: [T, S]
}
```

Writers and readers of `BinPhysics` are **not** reordered: `FxMatrix::process_hop()` runs slots
in numerical order, so a reader sees a writer's current-hop output only if the writer sits at a
lower slot index; a higher-numbered writer's physics arrives one call late (see the `writer_bits`
comment in `src/dsp/fx_matrix.rs` and the trait doc in `src/dsp/modules/mod.rs`).

`ModuleContext<'_>` carries `sample_rate`, `fft_size`, `num_bins`, `attack_ms`, `release_ms`,
`sensitivity`, `suppression_width`, `auto_makeup`, `delta_monitor`, the host transport values
`bpm` and `beat_position` (0.0 if unavailable), and optional infra fields populated by later
phases: `unwrapped_phase`, `peaks`, `instantaneous_freq`, `if_offset`, `chromagram`,
`harmonic_groups`, `midi_notes`, `held_pitch_classes`, `cepstrum_buf`, `sidechain_derivative`,
`bin_physics`, and `history`. The optional fields are `None` unless the relevant infrastructure
is active (several are gated by `ModuleSpec.needs_*` flags). The struct is `Copy + Clone` (the
optional fields are borrowed slices/references) — it is assembled in `Pipeline::process()` and
passed by reference.

`FxChannelTarget` (`All` / `Mid` / `Side`) gates whether the slot processes the current channel
in MidSide mode. Modules handle this by checking target vs. channel inside `process()`.

## FxMatrix and RouteMatrix

`FxMatrix` (`src/dsp/fx_matrix.rs`) holds the 9 `Option<Box<dyn SpectralModule>>` slots and
pre-allocated per-slot output buffers. `process_hop()` iterates slots 0–7 (slot 8 = Master is
handled separately), assembles each slot's input from the route matrix, dispatches to the module,
then routes the output.

`RouteMatrix` (`src/dsp/modules/mod.rs`) is a plain struct of `[[f32; MAX_SLOTS]; MAX_MATRIX_ROWS]`
send amplitudes. Default serial wiring: Dynamics (slot 0) → Gain (slot 1) → Master (slot 8),
matching the default slot types. Off-diagonal cells set send amplitude between any two slots;
`virtual_rows` support T/S split outputs. `RouteMatrix` also carries `amp_mode` and `amp_params`
for per-cell AmpMode kernels (Linear / Vactrol / Schmitt / Slew / Stiction).

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
   Set `writes_bin_physics`, `needs_instantaneous_freq`, `needs_cepstrum`, `needs_chromagram`,
   `needs_harmonic_groups`, and `needs_midi` as needed.
3. Create a new file in `src/dsp/modules/`, implement `SpectralModule`.
4. Wire the variant in `create_module()`.
5. Write at least one test in `tests/module_trait.rs` covering the new type.
6. Override `tail_length()` if the module holds state beyond one FFT window (e.g. Freeze).
7. If the module has sub-modes, add a `set_<type>_mode()` default no-op to `SpectralModule` and
   override it in the module. Add a `slot_<type>_mode: Arc<Mutex<[Mode; 9]>>` to `params.rs`,
   an `FxMatrix::set_<type>_modes()` fan-out, and a per-block `try_lock()` propagation call in
   `Pipeline::process()` — follow the existing Gain / Future / Past pattern.
8. If the module needs a dedicated panel, create `src/editor/<module>_panel.rs` and set
   `panel_widget` in its `ModuleSpec`.

The module receives `curves: &[&[f32]]` of length `num_curves()`. Index 0 is curve 0,
etc. — **never index beyond `num_curves()`**. The curve-to-parameter mapping is the module's
own responsibility; see existing modules for the pattern.

## Gotchas

- `StftHelper::process_overlap_add()` takes `&mut self` inside a closure. Rebind all `self.*` fields as locals before the call to avoid conflicting borrows. See pipeline.rs for the established pattern.
- `triple_buffer::Output::read()` takes `&mut self` — each call must be a separate statement.
- `slot_curve_cache` in `Pipeline` is `Vec<Vec<Vec<f32>>>` (9 slots × 7 curves × MAX_NUM_BINS). Only `[0..num_bins]` is valid for the current FFT size.
- `FxMatrix::process_hop` temporarily `take()`s each slot out of `self.slots[s]` to avoid a simultaneous borrow of `slots` and `slot_out`. Always `put` it back unconditionally.
- There is ONE `sc_stft: StftHelper` processing the single stereo sidechain input. The pipeline derives L, R, L+R, M, and S envelopes from it into `sc_envelopes[0..5]` (indexed by `ScSource`); each slot picks its source via `slot_sc_channel`.
- All visual constants live in `editor/theme.rs` — do not hardcode colours or sizes elsewhere.
- `assert_eq!(m.num_curves(), module_spec(ty).num_curves)` is debug-asserted in `create_module()` — keep these in sync when adding a new module.
