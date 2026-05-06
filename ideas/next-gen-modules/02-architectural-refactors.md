# Architectural Refactors Implied by the Next-Gen Modules

**Created:** 2026-04-26
**Status:** RESEARCH

This file lists code-level changes that span more than one of the next-gen
modules. Each is referenced by the per-module files but argued only here.

---

## 1. SpectralModule trait extensions

The current trait (see `src/dsp/modules/mod.rs`) is:

```rust
pub trait SpectralModule: Send {
    fn process(&mut self, channel, stereo_link, target, bins, sidechain,
               curves, suppression_out, ctx);
    fn reset(&mut self, sample_rate, fft_size);
    fn tail_length(&self) -> u32 { 0 }
    fn module_type(&self) -> ModuleType;
    fn num_curves(&self) -> usize;
    fn set_gain_mode(&mut self, _: GainMode) {}
}
```

### Proposed additions

#### a) Optional BinPhysics access

```rust
fn process(
    &mut self,
    /* … existing args … */
    physics: Option<&mut BinPhysics>,   // NEW — None when BinPhysics is off
    ctx: &ModuleContext,
);
```

Adding an `Option<&mut>` to the existing signature is safe (modules that
don't care ignore it). The `Option` lets us ship BinPhysics behind a
plugin-level toggle for users who want the simpler / faster behaviour.

Alternative: stash the physics ref inside `ModuleContext`. Cleaner
signature, but requires careful lifetime work because ctx is currently
`Copy`.

**Question:** Copy-vs-borrow on `ModuleContext`. Today it's `Copy`. With
a `&mut BinPhysics` it can't be `Copy`. Worth converting now even before
BinPhysics lands — most modules already take it by reference.

#### b) Module declarations beyond `num_curves`

Today `ModuleSpec` carries `num_curves`, display name, colour, curve
labels. Several next-gen modules need the pipeline to know things at
schedule time, not at process time:

```rust
pub struct ModuleSpec {
    /* … existing … */
    pub needs_history:    bool,   // Past, Geometry
    pub needs_cepstrum:   bool,   // Harmony Formant Rotation, Cepstral Liftering
    pub needs_midi:       bool,   // Harmony, Rhythm, Kinetics gravity wells with MIDI
    pub needs_if:         bool,   // Harmony, Modulate, Kinetics orbital
    pub needs_chromagram: bool,   // Harmony, Geometry
    pub heavy_cpu:        bool,   // Circuit (BBD, transformer), Past, Geometry
}
```

Pipeline reads these at slot-assignment time (not in the audio loop) and:

- skips the IF / chromagram / cepstrum compute if no slot needs them
- shows a "heavy CPU" badge in the module popup if `heavy_cpu`
- routes MIDI events into `SharedState` only if any slot needs it

This is cheaper than runtime polling and keeps the per-block fast-path
tight.

#### c) Per-module probe — already exists for tests/probe; extend for telemetry

Each module already has `last_probe: ProbeSnapshot` under
`#[cfg(any(test, feature = "probe"))]`. Consider a release-build version
exposing a single `cpu_us: f32` reading per slot, displayed in the module
popup as a CPU bar. Costs one timer call per `process()` — acceptable.
Useful for users to know when they've stacked too many heavy modules.

---

## 2. ModuleContext additions

Currently carries: `sample_rate`, `fft_size`, `num_bins`, `attack_ms`,
`release_ms`, `sensitivity`, `suppression_width`, `auto_makeup`,
`delta_monitor`. All `Copy`.

### Proposed additions

| Field | Why | Cost |
|---|---|---|
| `if_array: Option<&[f32]>` | Instantaneous frequency per bin; None if no slot needs it. | One IF compute per hop per channel when on. |
| `chromagram: Option<&[f32; 12]>` | 12-element pitch-class profile. | Sparse-matrix multiply. |
| `cepstrum: Option<&[f32]>` | Real cepstrum of the current spectrum. | One real-FFT + log + real-IFFT per hop, gated by needs_cepstrum. |
| `bpm: f32` | Host tempo. | Free (already in transport). |
| `beat_position: f64` | Position in beats at start of block. | Free. |
| `held_notes: Option<&[bool; 128]>` | MIDI key state. | Free once MIDI is wired. |
| `history: Option<&HistoryBuffer>` | Read-only past frames. | History memory + write. |

`Option<&_>` lets each be `None` when no slot subscribes.

**Lifetime detail:** these all become `&'block` references — `ModuleContext`
gains a `'block` lifetime, lose `Copy`, become `Clone`. Refactor cost is
minor; touches about a dozen call sites.

---

## 3. MIDI input plumbing

### Current state

Plugin does not declare MIDI input. `Plugin::midi_config()` is at default.
nih-plug supports MIDI inputs on CLAP plugins via the same API as VST3.

### What needs to change

1. **`lib.rs`** — `impl ClapPlugin for SpectralForge`:
   ```rust
   const CLAP_FEATURES: &'static [...] = &[..., MidiEffect /* or similar */];
   ```
   And `impl Plugin`:
   ```rust
   const MIDI_INPUT: MidiConfig = MidiConfig::Basic;
   ```

2. **`bridge.rs`** — `SharedState` gains a lock-free 128-bool array
   plus a 12-bool pitch-class snapshot, both updated by the audio thread,
   read by GUI for visualisation.

3. **`Pipeline::process()`** — drain MIDI events from `aux.midi`, update
   `held_notes[note as usize] = true` / `false` on note-on / note-off
   events. Project events into a 12-bool pitch-class snapshot
   (note % 12).

4. **`ModuleContext`** — pass `held_notes` reference through.

### Question

Do we need per-voice (note-id, velocity, channel) or only the
"is-this-note-held" projection? Per-voice is required if Spectral
Arpeggiator wants to handle chord notes individually. The
"is-held + pitch-class" projection is sufficient for Chordification,
Harmonic Companding, gravity-well tracking, MIDI-driven rhythmic phase
reset.

**Recommendation:** ship the simple version first. The arpeggiator can
poll `held_notes` and grab notes in note-number order; that gives most of
the per-voice value without a more complex API.

### Bonus

Once MIDI input is in, MPE / per-note pitch bend would let modules track
bends — useful for Chordification's "this voice is gliding" behaviour.
But this is a phase-2 polish, not a v1 must-have.

---

## 4. Slot-count growth

Today `NUM_SLOTS = 9` (slots 0–7 user, slot 8 Master). The next-gen
module catalogue is 8 modules already specced + at least 3 new (Future,
Geometry, Punch) = 11 module *types*, but a user only needs ~7 active
slots in practice.

**Decision:** keep `NUM_SLOTS = 9`. The module catalogue grows; the slot
count stays. Users pick which modules they want active. The popup grows.

If anyone wants more parallel slots, increase to 13 (12 user + Master) —
the matrix grows quadratically (`13×13 = 169` send cells vs `9×9 = 81`),
but most cells are zero so the cost is in UI density, not RAM. Keep
`NUM_SLOTS = 9` for now and revisit if user testing reveals a real need.

---

## 5. Curve-count growth

Today `NUM_CURVE_SETS = 7`. The next-gen modules want:

- Circuit: 4
- Life: 5
- Kinetics: 5
- Past: 5
- Future: 5 (proposed)
- Harmony: 5
- Modulate: 5
- Rhythm: 4
- Geometry: 5 (proposed)

So 5 fits almost everything; 7 has slack. **Keep `NUM_CURVE_SETS = 7`.**

But — the *curve labels* differ per module. The shipped pattern (per
`ModuleSpec.curve_labels: &'static [&'static str]`) handles this fine.
Curves indexed beyond `num_curves()` are simply not driven by the module.
No change required.

---

## 6. Scratch buffer policy

Multiple new modules need scratch buffers (Life Viscosity, Kinetics
Springs, Past Decay Sorter sort key array). The shipped pattern (e.g.
Freeze's `freeze_target: Vec<Complex<f32>>` allocated in `reset()`) is
already correct.

### Rules to write down

1. **All scratch buffers live in the module struct, allocated in
   `reset()`**, sized at `MAX_NUM_BINS` not `num_bins` — that way an
   FFT-size change doesn't reallocate.
2. **Never use `Vec<Vec<f32>>`** — use a flat `Vec<f32>` indexed
   `[stage * MAX_NUM_BINS + k]`. This is what the BBD spec says; codify
   it for everyone.
3. **`assert_process_allocs` is on** — cargo will trap any RT allocation
   in test runs.
4. **`permit_alloc!` for `reset()` only** — never inside `process()`.

This is mostly already practice; the proposal is to make it a written
contract in `CLAUDE.md` so it stops being tribal knowledge.

---

## 7. Pre-allocation budget

If every module pre-allocates `MAX_NUM_BINS = 8193` of every state field
it owns, a heavy slot like Circuit (vactrol_level + latch + bucket[4 *
8193] + flux + temp) is ~250 KB. Times 9 slots = ~2.3 MB. Times 2
channels for Independent stereo = 4.6 MB. Add BinPhysics ~7 fields ×
8193 × 4 bytes = 230 KB.

This is fine. The HistoryBuffer (~17 MB at 256 frames) dominates.
Still well under any reasonable plugin memory budget.

The *deserialisation* path matters more — preset load currently rebuilds
modules. Confirm `permit_alloc!` is acceptable on preset load (it should
be — preset load is not the audio thread).

---

## 8. The Y-axis curve quantization feature

**Source:** Kim's note "Quantising the graph both by X and Y might be
beneficial for many of these, especially quantising Y by note frequencies,
but need MEL-bands for these? OR Cepstral math?"

### What it would mean

Each curve would gain optional snap-to-grid in either dimension:

- **X-snap:** node positions snap to 1/8th-octave, semitone, or MIDI-note
  positions
- **Y-snap:** node values snap to integer dB steps, integer ratios, or
  named scale degrees (for Harmony's Chordification snap-strength curve
  or Rhythm's Euclidean step-count curve)

### Where it lives

This is a `CurveDisplayConfig` extension (per
`docs/superpowers/specs/2026-04-23-ui-parameter-spec-design.md`). A
`snap_x: SnapMode`, `snap_y: SnapMode` enum per curve type. Editor draws
the snap grid as faint horizontal/vertical lines.

### Cost

Pure UI. The audio thread still reads continuous gain values from
`curve_rx[s][c]` — the snap is applied at curve-write time on the GUI
side (between user drag end and `tx.publish()`). Zero RT cost. ~50 LoC
per snap mode.

### Question

Mel-bands and cepstral are different things from "snap to musical
intervals." Mel-bands could be a *display* mode (X axis is Mel-spaced
not log-spaced) for any curve, useful for vocal/perceptual editing.
Cepstral X is for the Cepstral Liftering sub-effect specifically — it's
a transform of the X axis itself, where "low quefrency" = formants.

These are three orthogonal features; lump them together as
`CurveAxisMode { Linear, Log, Mel, Cepstral }` for X, and
`SnapMode { None, Octave, Semitone, ScaleDegree, IntegerDb }` for Y.

---

## 9. "Always-bypassed on low-end hardware" pattern

**Source:** Kim's note "the heavier stuff can be an always bypassed."

### Suggested model

- `ModuleSpec.heavy_cpu: bool` (already proposed in §1).
- A plugin-level setting `enable_heavy_modules: bool`, default true on
  desktop. Default false would be appropriate for, e.g., a future iPad
  port, but we don't ship that today.
- If `enable_heavy_modules == false` and a slot has a heavy module, the
  module's `process()` is short-circuited to `bins` passthrough; the
  module popup shows it greyed out with an explanation.

This is a soft guard, not a hard one. The user can always re-enable.

### Alternative, more interesting

**CPU governor:** measure per-slot `cpu_us` (per §1c) and if total
spectrum-stage CPU exceeds a budget (e.g. 70% of one core), the most
recently enabled heavy slot is *automatically* bypassed with a banner
notification. Would prevent dropouts. Adds non-deterministic behaviour
which DAW users may dislike. **Recommendation:** skip the governor for
now; ship the soft-guard pattern.

---

## 10. The "always bypass" badge and CPU honesty

For each new module, the per-module file lists a CPU class:

- **light:** scalar arithmetic per bin, no extra state arrays beyond
  one or two `Vec<f32>` (Modulate Phase Phaser, Rhythm Phase Reset).
- **medium:** per-bin state with a few arrays (Life Viscosity, Modulate
  PLL, most Harmony sub-effects).
- **heavy:** per-bin state with multi-stage delay buffers, large
  scratch operations, or per-hop sorts (Circuit BBD, Past Decay Sorter,
  Geometry persistent-homology).

The module popup colours the "💀" badge by class. Honest.

---

## 11. Calibration impact

Every shipped module has a calibration round-trip test (per the
`2026-04-24-calibration-audit-design.md`). Each new module needs:

1. A `ProbeSnapshot` adding the 6 calibration probes (curve→physical,
   amplitude, etc.) under `#[cfg(any(test, feature = "probe"))]`.
2. A round-trip test in `tests/calibration.rs` showing input curve →
   probed parameter → expected physical value.
3. UI hover-tooltip text matching the probe's physical units.

This is overhead per module but it caught real bugs in the recent
calibration-audit work. Keep the discipline. Each per-module file lists
the proposed probe set.

---

## 12. Refactor sequencing

If everything in this folder is built, the order of code-level changes
goes:

1. **MIDI plumbing** (low risk, one-block change, used by 4+ modules).
2. **`ModuleContext` lifetime + new fields** (touches every module's
   `process()` signature; do once).
3. **`ModuleSpec` declarations** (`needs_history`, etc.).
4. **BinPhysics** (the big one — see existing spec; rewrite §1
   merge-rules clarification first).
5. **Instantaneous Frequency in ModuleContext** (now consumers can be
   added).
6. **HistoryBuffer in Pipeline + ModuleContext** (now Past becomes
   buildable).
7. **Cepstrum lazy compute** (now Cepstral Liftering becomes buildable).
8. **Chromagram + harmony matrix in ModuleContext** (now Harmony becomes
   buildable).
9. **Modules** — one at a time, in the order from
   `99-implementation-roadmap.md`.

Each of 1–8 is a self-contained PR that does not change any
already-shipping behaviour. They can all land before any new module
ships.

---

## Open architectural questions

1. `ModuleContext`: refactor to non-Copy now (clean), or keep Copy +
   stuff Option<&'_> fields awkwardly (avoid lifetime work)? **Lean
   toward non-Copy.**
2. **MIDI per-voice vs. flat held-notes:** see §3. Lean flat.
3. **CPU governor vs. soft-guard for heavy modules:** see §9. Lean
   soft-guard.
4. **Curve Y-snap vs. X-axis-mode separation:** see §8. Lean separate.
5. **Heavy-module default-off behaviour:** if a user opens a preset that
   enables a heavy module on a host where heavy is disabled, do we
   silently load it disabled (with banner) or refuse the preset?
   **Lean silent + banner.**
