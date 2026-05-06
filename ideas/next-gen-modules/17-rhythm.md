# Rhythm Module — Audit

**Existing spec:** `docs/superpowers/specs/2026-04-21-rhythm-module.md`
**Status:** DEFERRED, depends on host BPM sync infrastructure (covered
under `01-global-infrastructure.md` § Host BPM sync) and BinPhysics
(only weakly — most Rhythm modes are stateless on bin physics).
**Source brainstorm:** Cat 2 ideas #5 (Spectral Euclidean Rhythms),
#6 (Transient-Triggered Phase Reset), #7 (Bin-Specific Swing), #8
(Rhythmic Phase Scramble), and the "Modulation Ring" UI proposal.

## What the spec covers

Four sub-effects: Euclidean Rhythms, Spectral Arpeggiator, Bin Swing,
Rhythmic Phase Reset (Laser). Curves: AMOUNT, DIVISION, ATTACK/FADE,
MIX (4). Defines the host BPM sync infrastructure in-line as a
prerequisite.

## Brainstorm cross-reference

| Brainstorm idea | Kim's note | In spec? | Notes |
|---|---|---|---|
| Spectral Euclidean Rhythms (#5) | "quantise Y to 8 steps would cater for most useful rhythms" | yes | Spec already cites Y quantization. |
| Transient-Triggered Phase Reset (#6) | "Could also use note input" | yes | Spec covers as Laser; mentions note-trigger via T/S Split sidechain. |
| Bin-Specific Swing (#7) | "could be just a part of a well-thought out spectral delay" | yes | Spec covers; Kim's hint suggests this is *not* the right home. See § (a). |
| Rhythmic Phase Scramble — Stochastic Hi-Hats (#8) | "should be a part of especially noisy modules like Freeze and smear, not necessarily their own effect" | partial | Spec doesn't include this explicitly; Kim's hint says relocate to Freeze/PhaseSmear. See § (b). |
| Spectral Arpeggiator (#4) | — | yes | Spec covers. |
| Modulation Ring UI (S/H + Sync + Legato per knob) | — | no | Cross-cuts everywhere — see `01-global-infrastructure.md` § Modulation Ring UI. Not Rhythm-specific. |
| BPM sync for time-based parameters | "shows a transparent overlay for the tab in question" | no | Same as Modulation Ring — global UX, not Rhythm. |

## Gap details

### a) Bin Swing — relocate to a future Spectral Delay module?

**Kim's annotation:** "This could be just a part of a well-thought out
spectral delay."

The spec's Bin Swing implementation needs per-bin delay buffers, which
is *exactly* the infrastructure a future Spectral Delay module would
require. Implementing Bin Swing inside Rhythm now means:

- Per-bin ring buffers (~2 MB at 16 hops × 8193 bins × 4 bytes per
  channel) sit in Rhythm's module struct.
- A future Spectral Delay module would either duplicate the buffers or
  share via some new infrastructure.

**Options:**

1. **Ship Bin Swing in Rhythm now**, accept the duplication when
   Spectral Delay arrives.
2. **Defer Bin Swing**, ship Rhythm without it; introduce Bin Swing as
   a sub-effect of a future Spectral Delay module. Rhythm becomes a
   3-mode module for v1.
3. **Build a shared `SpectralDelayBus` infrastructure** in
   `01-global-infrastructure.md`, both Rhythm and Spectral Delay route
   through it. More work upfront.

**Recommendation:** option 2. Defer Bin Swing until Spectral Delay
exists. It is the only Rhythm sub-effect that needs significant
state, and pulling it out simplifies Rhythm to a stateless +
beat-position module. Cross-link to a future "Spectral Delay" idea
file (not yet drafted; consider `21-delay.md` if the plan moves
forward).

### b) Stochastic Hi-Hats — relocate to Freeze and PhaseSmear

**Kim's annotation:** "These kind of ideas should be a part of
especially noisy modules like Freeze and smear, not necessarily their
own effect."

The pattern is: a phase-randomizing or freeze-style effect is *gated*
by a beat-synced trigger. Currently neither Freeze nor PhaseSmear has
beat-sync awareness — they react instantaneously.

**Recommendation:** add a `beat_gate_curve: Option<&[f32]>` per slot,
populated by a future "BeatSync" sidechain mode that exposes the
current beat position as a per-bin envelope. Then any module
(Freeze, PhaseSmear, Modulate, etc.) can opt into beat-synced
gating without Rhythm having to host every variation.

This is global infra work, not a Rhythm gap — flag in
`01-global-infrastructure.md` § Host BPM sync. For Rhythm, no action
needed.

### c) Phase Reset target curve

The spec's Laser mode "forces the phase of all active bins to 0 (or
a graph-defined target phase)." It doesn't specify how the user draws
the target phase across the spectrum. Add an explicit `TARGET_PHASE`
curve mapping bin → target phase angle (-π to +π). Default curve:
flat at 0 (constructive interference).

This adds a 5th curve to the module. Total stays under
`NUM_CURVE_SETS = 7`.

### d) Arpeggiator with note-input triggering

**Spec:** Arpeggiator advances on BPM-synced steps.
**Gap:** doesn't accept MIDI note-on as a step-advance trigger. For
producers, a host-MIDI'd arpeggio sync is more useful than
beat-counted because it stays musical even if the host transport is
running free.

**Recommendation:** add a `trigger_source` enum to Arpeggiator
sub-effect: `BPM` (default) or `NoteIn` (advance on each MIDI
note-on). MIDI plumbing is global infra (see
`01-global-infrastructure.md` § MIDI input plumbing).

### e) Step-sequencer UI for arpeggiator

The spec mentions "a row of 8 toggle buttons per voice" as a
sequencer UI. This is not draw-curve-shaped — it's a discrete grid.
**Concrete UI proposal:**

- 8 voices × 8 steps grid (64 toggles per slot in Arpeggiator mode).
- Stored as `[u8; 8]` per voice (each u8 is a bitmask of which steps
  the voice plays).
- Editor renders only when slot is in Arpeggiator mode; other modes
  hide the grid.
- Persistence: store as a packed string in nih-plug params (8 bytes
  per slot when Arpeggiator).

Cross-link to `02-architectural-refactors.md` § Per-mode UI panels
(no such mechanism exists yet — most modules have only curve
editors).

## Sub-effect status — final

| Mode | Status |
|---|---|
| Euclidean Rhythms | keep, ship for v1 |
| Spectral Arpeggiator | keep, ship for v1, add NoteIn trigger source |
| Bin Swing | **defer** to a future Spectral Delay module per § (a) |
| Rhythmic Phase Reset (Laser) | keep, ship for v1, add TARGET_PHASE curve § (c) |

3 modes for v1. Once the Spectral Delay infrastructure exists, Rhythm
gets a `Swing` mode added back as a thin wrapper on the shared
delay-bus.

## Curves

After audit:

| Idx | Label | Used by |
|---|---|---|
| 0 | AMOUNT | Euclidean (density), Arpeggiator (velocity envelope), Phase Reset (strength) |
| 1 | DIVISION | Euclidean (step count N per band), Phase Reset (subdivision) |
| 2 | ATTACK / FADE | Euclidean/Arpeggiator gate attack/release shape |
| 3 | TARGET_PHASE | Phase Reset (per-bin target phase angle) |
| 4 | MIX | All modes |

5 curves. `num_curves() = 5`.

## Architecture fit

### Plain SpectralModule slot, with BPM in ModuleContext

Rhythm needs from global infra:

- **ModuleContext.bpm: f32** — already specced.
- **ModuleContext.beat_position: f64** — already specced.
- **ModuleContext.midi_notes: &[NoteOn]** — required by Arpeggiator
  NoteIn trigger source (§ d). New, see `01-global-infrastructure.md`
  § MIDI input plumbing.

Does NOT need:

- BinPhysics (none of the modes write bin physics state).
- Instantaneous Frequency (Arpeggiator can use simple peak detection
  on bin magnitude — IF is overkill).
- History Buffer.

### Per-mode UI panel

Spectral Arpeggiator's step-sequencer UI is the first per-module
non-curve UI requirement (§ e). Currently `editor/curve.rs` only
renders curves. Either:

- Add a `module_panel(ui, slot, module_type)` callback in
  `editor_ui.rs` dispatched per-module.
- Or: extend the existing module popup to host the per-mode UI
  inline.

The popup approach is simpler but loses persistent visibility. The
panel approach matches how plugins like Photosounder expose per-mode
controls. **Recommendation:** add a per-module `panel_widget` callback
in `ModuleSpec`, render below the curve editor when present. See
`02-architectural-refactors.md` § Per-module UI panels.

### Why the user-facing name "Rhythm" matters

Per the same naming logic as Past/Future/Life/Kinetics, "Rhythm" is a
concept users immediately grasp. The alternative ("BPM-Synced Spectral
Gate") is descriptive but flat.

## CPU class

Light. Euclidean = 1 multiply per bin per hop. Arpeggiator = peak
detection (one sort per hop) + 1 envelope per active voice. Laser =
overwrite N phases per hop (when triggered).

`heavy_cpu = false`.

## BinPhysics interactions

Reads: nothing (Rhythm is BPM-driven, not physics-driven).
Writes: nothing.

This makes Rhythm one of the easiest modules to ship — it's almost
purely a beat-position consumer.

## Calibration probe set

- `probe_amount_pct`
- `probe_active_mode_idx`
- `probe_current_beat_pos` (0.0 to 1.0 within current bar)
- `probe_arp_step_idx` (Arpeggiator only)
- `probe_active_voice_count` (Arpeggiator only)

## RESEARCH PROMPT — none required

Rhythm is the cleanest of the deferred modules. The audit surfaces
deferrals (Bin Swing) and refinements (TARGET_PHASE curve, NoteIn
trigger) but no DSP research is needed — the math is well-understood
(Bjorklund algorithm for Euclidean, simple peak-pick + sequencer for
Arpeggiator, phase-overwrite for Laser).

The only research-style question is the per-mode UI panel mechanism,
which is an architectural concern not a DSP one. Addressed in
`02-architectural-refactors.md`.

## Open questions

1. **Bin Swing deferral.** Approve § (a) — ship Rhythm with 3 modes,
   add Swing back when Spectral Delay infrastructure exists?
2. **NoteIn trigger.** Worth the MIDI plumbing for Arpeggiator alone?
   See § MIDI input plumbing global infra; if MIDI is added for
   Harmony / Compander anyway, this comes free.
3. **Per-module UI panel mechanism.** Approve the `panel_widget`
   callback in `ModuleSpec`? This unlocks Arpeggiator's step grid
   *and* future modules that need non-curve UI (e.g. Future module's
   pre-delay length picker).
4. **Stochastic Hi-Hats relocation.** Confirm Kim's hint — gate Freeze
   and PhaseSmear via the same beat-sync mechanism rather than adding
   a Rhythm sub-effect.
5. **Eight-voice Arpeggiator step grid persistence.** 64 toggles per
   slot in Arpeggiator mode. Pack into nih-plug params or use a
   custom serialization byte stream?
