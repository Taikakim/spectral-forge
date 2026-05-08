# Changelog

All notable changes to Spectral Forge.

The project follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) loosely
and uses calendar-driven version numbers (the `0.MAJOR.MINOR` series tracks the
prerelease cycle, not strict SemVer).

---

## [0.15.0] — 2026-05-09

The 0.10 → 0.15 cycle is effectively a full rewrite. The 0.1.0 codebase was a
single per-bin spectral compressor with a monolithic engine. 0.15 ships a 9-slot
modular pipeline, 19 module types, host automation across every parameter, a
JSON preset system, and a calibrated curve editor with per-parameter
transforms. Existing patches will not load — reset to default and rebuild.

### Architecture — modular spectral pipeline

- **9-slot pipeline.** Slots 0–7 are user-assignable module instances; slot 8
  is the dedicated Master with the soft clipper. Each slot holds a
  `Box<dyn SpectralModule>` with its own state, curves, and stereo handling.
- **`SpectralModule` trait** — single interface for per-bin processing. Modules
  receive a pre-allocated `&mut [Complex<f32>]` block plus a slice of
  per-bin curve gains, an optional sidechain magnitude slice, and a
  `ModuleContext` carrying sample rate, FFT size, attack/release globals,
  delta-monitor flag, MIDI state, transport position, and instantaneous
  frequency reads.
- **Real-time safety enforced.** `assert_process_allocs` is on for the audio
  thread; lock-free triple-buffer for curves; `try_lock` with fallback for
  matrix snapshots; `flush_denormals()` sets FTZ+DAZ each block.
- **Variable FFT.** User picks 512 / 1024 / 2048 / 4096 / 8192 / 16384 at
  runtime. Buffers are pre-allocated for the maximum; bin count is passed
  through to every module call. Latency is reported per-host.
- **Stereo modes.** Linked (single STFT through the chain), Independent
  (per-channel module state), MidSide (pre-STFT M/S encode, modules see M
  on channel 0 and S on channel 1, decoded after the chain). Per-slot
  `FxChannelTarget` (All / Mid / Side) gates which slots act on which
  component.
- **Four aux sidechain inputs.** Each slot can listen to any of the four
  sidechain STFTs independently. SC envelope is published to GUI for
  overlay rendering on Gain curves.

### Routing matrix with feedback

- **Modular send/receive grid.** A `RouteMatrix` of `[[f32; 9]; 13]` drives
  inter-slot routing every block. Off-diagonal cells are arbitrary send
  amplitudes between any two slots, including loops back to earlier slots
  (feedback paths).
- **Negative sends and feedback.** Cells accept negative amplitudes for
  polarity inversion, and backward routing is permitted with built-in
  matrix-cell smoothing (per-block) to avoid zipper noise on automation.
- **Virtual rows for split outputs.** T/S Split exposes its Transient and
  Sustained outputs as separate matrix sources (rows 9–12), so each can
  be routed to a different downstream slot.
- **Hot-swap.** Module type changes reset state, reset curves, republish to
  the audio thread, and ramp the matrix cell over 100 ms — no clicks.
- **Browse-time help.** Hovering matrix cells, source/destination labels,
  module-popup buttons, and per-mode pickers updates a single help-box
  widget right of the matrix with the parameter's role, units, and
  sidechain notes.
- **Dynamic flow text.** The matrix now shows live "X feeds Y at Z dB"
  text for each connection; disconnected cells render dim.

### Module roster

| Module | Purpose | Modes |
|---|---|---|
| **Dynamics** | Per-bin compressor / expander | (single) |
| **Freeze** | Spectral freeze with per-bin resistance | (single) |
| **Phase Smear** | Phase randomisation with peak-hold envelope | (single) |
| **Contrast** | Spectral contrast / transient sculptor | (single) |
| **Gain** | Per-bin gain shaping | Add · Subtract · Pull · Match |
| **MidSide** | M/S balance, expansion, phase decorrelation | (single) |
| **T/S Split** | Transient / Sustained split with virtual outputs | (single) |
| **Harmonic** | Harmonic emphasis | (single, MIDI scaffolding pending) |
| **PAST** | Bin-buffered spectral history | Granular · DecaySorter · Convolution · Reverse · Stretch |
| **Geometry** | Spatial-physics resonance | Chladni · Helmholtz |
| **Circuit** | Analogue-circuit emulations | BBD Bins · Spectral Schmitt · Crossover Distortion · Vactrol · Transformer Saturation · Power Sag · Component Drift · PCB Crosstalk · Slew Distortion · Bias Fuzz |
| **Life** | Fluid / surface / friction physics | Viscosity · Surface Tension · Crystallization · Archimedes · Non-Newtonian · Stiction · Yield · Capillary · Sandpaper · Brownian |
| **Modulate** | Cross-bin modulation | Phase Phaser · Bin Swapper · RM/FM Matrix · Diode RM · Ground Loop · Gravity Phaser · PLL Tear · FM Network |
| **Rhythm** | Tempo-synced gating / arpeggiation / phase reset | Euclidean · Arpeggiator · Phase Reset |
| **Punch** | Sidechain punch shaping | Direct · Inverse |
| **Harmony** | Pitch / harmonic restructuring (MIDI-aware) | Chordification · Undertone · Companding · Formant Rotation · Lifter · Inharmonic (Stiffness / Bessel / Prime) · Harmonic Generator · Shuffler |
| **Kinetics** | Mass / spring / orbital physics | Hooke · Gravity Well · Inertial Mass · Orbital Phase · Ferromagnetism · Thermal Expansion · Tuning Fork · Diamagnet |
| **Future** | Print-through / pre-echo simulations | Print-Through · Pre-Echo |
| **Master** | Output stage with toggleable soft clipper | (fixed in slot 8) |

Total: 19 module types, 79+ distinct modes.

### Curve editor

- **Per-bin curve drawing.** 7 curve channels per slot (threshold, ratio,
  attack, release, knee, makeup, mix on Dynamics; module-specific
  labels elsewhere). 6 nodes per curve: shelves at indices 0 and 5,
  Gaussian bells at 1–4. Hover-to-read tooltip with frequency + value
  in physical units.
- **Per-curve transforms.** Each curve has its own offset, tilt, and
  curvature parameter. Tilt is now spec-literal multiplicative dB/oct
  (`g · 10^(dB_per_oct · oct_from_pivot / 20)`); offsets on log-axis
  curves are uniform shifts in display-dB space; curvature blends a
  smoothstep S-curve into the tilt shape, pivoted at 1 kHz.
- **WYSIWYG calibration.** A per-(module, curve_idx) `CurveDisplayConfig`
  defines axis range, grid lines, units, and offset_fn. The runtime
  contract `gain_to_display(offset_fn(g, v)) == axis_aware_lerp(v)` is
  asserted across every module × curve in `tests/curve_calibration_matrix.rs`.
- **Headroom strip.** ~50 px of curve area sits above the 0 dB / y_max
  grid line for dragged nodes and loud bins; virtual-node range extended
  to ±2 with a directional triangle indicator at the rect edge when a
  node is off-rect.
- **Live spectrum gradient** (pre-FX and post-FX), peak-hold spectrum,
  per-curve sidechain-envelope overlay on Gain curves.

### Modulation Ring

- Per-curve-node Y-value modulation source with three independent toggles:
  **Sample & Hold** (random hold per beat), **Sync 1/16** (16th-note
  re-trigger), **Legato** (linear ramp between nodes). Combinable.
- Real-time-safe: per-key fixed-array `RingTransformState`, transforms
  applied per block to `slot_curve_cache` before module dispatch.

### MIDI input

- Plugin declares `MidiConfig::Basic`. Pipeline maintains held-note
  bookkeeping (`held_pitch_classes`, current notes) exposed via
  `ModuleContext.midi_notes`.
- **Harmony** uses MIDI to drive Chordification root and Harmonic
  Generator pitch.
- **Kinetics** Gravity Well well-positions can be sourced from MIDI
  (`WellSource::MIDI`) — well centres at f_root × harmonics for each
  held note. Degrades cleanly to no-op when no notes are held.
- **Rhythm** Arpeggiator can step through held notes.

### Host automation & presets

- **Every parameter is automatable.** All slot/curve/transform/matrix
  controls are nih-plug `FloatParam` / `BoolParam` / `EnumParam`. Drag
  on graph nodes and on tilt/offset DragValues records automation via
  `ParamSetter` so host lanes capture the gesture, not just the
  end-state.
- **JSON preset system.** Schema-versioned (`PRESET_SCHEMA_VERSION = 1`),
  pulldown UI in the top bar with Save / Load / Open Folder. Preset
  scan refreshes from the user's preset directory at load. Includes 5
  typed default builders: `default`, `transient_sculptor`,
  `spectral_width`, `phase_sculptor`, `freeze_pad`.
- **One-shot legacy migration** copies state from the 0.1-era
  `persistent_state` blob into the new individual params on first load.

### Scalable UI

- 1.0× / 1.25× / 1.5× / 1.75× / 2.0× scale settings, persisted across
  sessions. Theme constants drive every paint call through `scaled()`
  / `scaled_stroke()` helpers. Stroke widths snap to 2× at scale ≥ 1.75
  to avoid blurry sub-pixel rendering.
- **Context-sensitive help-box** to the right of the matrix updates as
  the user browses any control, popup option, mode picker, or graph.
  Toggleable on/off. Pending/presented promotion model so popups stay
  responsive without flicker.
- **Per-module inline panels** (PAST mode strip + DecaySorter sub-picker;
  Rhythm Arpeggiator grid; per-mode amp-popup filter graph) sit next to
  the slot row instead of in a separate popup, so editing them doesn't
  resize the plugin window.

### Stabilisation sweep (sub-projects A–G)

This release closed 18 numbered backlog items across seven sub-project
sweeps tracked in `docs/superpowers/2026-05-06-stabilization-backlog.md`:

- **A — routing, smearing, soft clipper move.** PLPV
  (`prev_unwrapped_phase`) was accumulating unbounded → bounded
  per-hop wrap, complex-blend in Freeze. Soft clipper moved from PAST
  into the Master slot with a host-registered `master_clip_enabled`
  toggle and `master_clip_threshold_db` knob.
- **B — module-switch hygiene + PAST mode UI.** Tilt / offset / curvature
  atomics reset on module switch with module-aware natural defaults;
  PAST mode picker inlined as a 5-button strip; DecaySorter sub-picker
  inline.
- **C — universal slider semantics.** Resolved the "100 % offset" pattern
  systemically (`default_nodes_for_module_curve` now returns
  module-correct defaults; only Dynamics retains the legacy RATIO
  high-shelf preset). Asymmetric `off_mix` for natural-at-max curves
  documented as the spec-allowed pattern.
- **D — axis defaults + headroom.** Master floor at -160 dBFS;
  `TILT_MAX = 4` dB/oct; visible curve area extended above 0 dB by ~50
  px; virtual-node range extended to ±2.
- **E — DSP semantics completion.** Freeze RESISTANCE log-scaled excess
  (now audibly meaningful, ~120 ms at curve 1.0); PAST SPREAD active
  in all 5 modes with mode-specific semantics; T/S Split SMOOTHNESS
  curve replaces the hardcoded `slow_coeff = 0.98`; Contrast extended
  from 1 to 6 curves like Dynamics.
- **F — PAST Age/Delay calibration.** `total_history_seconds` now flows
  end-to-end through `runtime_anchors` and the offset_fn; calibration
  matrix is no longer deferring any display index.
- **G — virtual matrix rows + PEAK HOLD + threshold parity.**
  Virtual-row sends persist + automate; PEAK HOLD curve mapping uses
  shared `peak_hold_curve_to_ms` helper; Threshold idx-9 DSP/UI parity
  pinned by a probe-feature regression test.
- **Tilt formula** rewritten to spec-literal multiplicative dB/oct in
  both audio and display paths; the prior `g · (1 + t)` form clipped
  to zero on one side at `TILT_MAX = 4`, producing the "doesn't fire
  at high tilt" symptom — gone now.

### Spec docs

These specs are the authoritative reference for ongoing work:

- `docs/superpowers/specs/2026-04-23-ui-parameter-spec-design.md` — axis
  ranges, grid lines, transforms, hover text, scaling.
- `docs/superpowers/specs/2026-05-06-phase-handling.md` — wrapped /
  unwrapped phase invariant, geodesic phase blend, unwrap kernel
  contract.
- `docs/superpowers/specs/2026-05-04-past-module-ux-design.md` — PAST
  mode dispatch and per-mode curve labels.

### Known issues carried forward to dev branch

- PhaseSmear AMOUNT/MIX and Gain Pull-Match offset_fns can't reach axis
  bounds at extreme curve gain (linear additive on small linear axes).
  Trade-off, not a bug — fixing it would saturate the slider's lower
  half at neutral curve.
- Harmonic module is structurally present but DSP and MIDI receptiveness
  are pending a dedicated brainstorming session.
- PAST default node y-values map curve_gain=1 to oldest frame (full
  history), which is the musically-inert end of the buffer per
  `2026-05-04-past-module-ux-design.md` §7.2. Calibration is consistent;
  the default-nodes choice is the next change.

### Build

```bash
cargo build --release
cargo run --package xtask -- bundle spectral_forge --release
cp target/bundled/spectral_forge.clap ~/.clap/
```

A `dev-build` Cargo feature gates an alternate CLAP id
(`com.spectral-forge.spectral-forge-dev`) and "(Dev)" name, so a dev
plugin can coexist with the production plugin without host conflict.

---

## [0.1.0] — 2026-04-15

Initial public tag. Single-module per-bin spectral compressor with the
`SpectralCompressorEngine` and `SpectralContrastEngine` engines, no
modular pipeline, no preset system, no host automation surface beyond
the engine knobs.
