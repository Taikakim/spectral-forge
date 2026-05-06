# Punch — NEW Module / Sidechain-Mode Proposal

**Existing spec:** none — this file is the first proposal.
**Status:** RESEARCH — not yet specced.
**Source brainstorm:** idea #21 (last addendum), Kim's "punch" concept:
"The sidechain punches holes in the spectrum either from top or below,
which either the pitch or amplitude of the neighbouring bins try to
fill in."

## What it is

A spectral effect where a sidechain peak detector identifies energy
concentrations in the sidechain, then **carves matching holes** in the
input spectrum at those frequencies (top-down or bottom-up), and the
*neighbouring* bins respond by either:

- **Pitch-shifting** into the hole (filling the gap by drift), or
- **Amplitude-boosting** into the hole (filling the gap by gain), or
- **Both** (filling by drift + boost).

This is qualitatively different from sidechain ducking. Ducking
*lowers* the amplitude where the sidechain is loud; Punch *removes*
the bin and the surrounding bins try to **physically reorganize** to
fill the void.

It's the opposite of a Helmholtz Trap (`18-geometry.md`) — instead of
filling a cavity, you carve a cavity and watch the spectrum heal.

## The "module vs sidechain mode" question

This is the crux of the proposal. Punch could live as:

### Option A: A new top-level module ("Punch")

- Has its own slot, curves, sub-effects.
- Sidechain is a normal input.
- Pros: explicit user-facing concept; users see "Punch" and know what
  it does.
- Cons: most of its logic is ducking-with-pitch-fill, which is small.
  Adding a whole module for one trick feels heavy.

### Option B: A sidechain *processor* module that other modules consume

- Punch sits as a module that *transforms* the sidechain bins into a
  "hole map" (negative magnitudes), which downstream modules
  (Dynamics, Gain, Mid/Side) then apply.
- Pros: composable; works with any module that has sidechain input.
- Cons: requires a new "sidechain transform" concept which the current
  RouteMatrix doesn't model. Significant infra work.

### Option C: A sub-mode of Dynamics (extend existing module)

- Dynamics already does sidechain ducking. Add a Punch mode that does
  the carve-and-fill behaviour.
- Pros: cheap; reuses Dynamics' sidechain plumbing.
- Cons: muddles Dynamics' identity (Dynamics is for compressors and
  expanders; Punch is structurally different — it *moves* bins).

### Option D: A "sidechain mode" enum on each module's slot

- Every slot already has `slot_sidechain[s]: SidechainSource`. Extend
  to `SidechainTransform` enum: `Direct`, `Punch(top|bottom)`,
  `Invert`, etc.
- The slot's module sees a transformed sidechain bin array instead of
  the raw one. Behaviour is per-module.
- Pros: maximally flexible.
- Cons: doesn't capture Punch's distinctive "fill" behaviour — only
  the carve.

**Recommendation:** **Option A** for v1. The carve-and-fill is its
own thing; users will understand "Punch" immediately; the
implementation is bounded. If later we find Punch's mechanics are
useful elsewhere, we can refactor toward Option B.

The remainder of this doc assumes Option A.

## Sub-effects

### a) Direct Punch — top-down carve

**Concept.** Sidechain peak detector finds the loudest M sidechain
bins. For each, attenuate the *input* bin at the same frequency to
near-zero (the "hole"). Neighbouring input bins (within a width set by
a curve) are *amplified* to compensate for the lost energy
(volume-conservation default), or *pitch-shifted* toward the hole
(the user picks behaviour via a switch).

This carves a brief, precise hole in the input spectrum every time the
sidechain has a peak.

- State: `prev_sidechain_peaks[N_PEAKS]` for hysteresis.
- Curves: AMOUNT (carve depth, 0.0 = no carve, 1.0 = full mute),
  WIDTH (how many neighbour bins to engage), FILL_MODE (curve sets
  per-bin "pitch fill amount" — neighbour-bin drift toward hole),
  AMP_FILL (curve sets per-bin amp boost), MIX.
- CPU: light. M peaks × bandwidth bins per hop.

### b) Inverse Punch — bottom-up carve

**Concept.** Same as Direct Punch but inverted: where the sidechain is
*quiet*, the input bin gets boosted; where the sidechain is loud, it
gets carved. Useful for "make my snare cut through wherever the bass
isn't" scenarios.

- Same state/curves as Direct Punch with one inversion.

### c) Self-Punch — auto-carve

**Concept.** The input is its own sidechain. The N loudest input bins
each get a "shadow" carved around them — energy is pushed away from
peaks into the surrounding spectrum. This is a peak-spreader / spectral
de-emphasis effect.

- Curves: same as Direct Punch.
- Useful for vocal de-essing where the harshest peaks need their
  energy redistributed rather than just muted.

### d) Pitch-Fill — slow neighbour drift

**Concept (refined from Kim's original).** Rather than amp-fill, the
neighbouring bins gradually *pitch-shift* toward the hole (the bin
they're trying to fill). Implementation: each neighbour bin gets its
phase rotated each hop by a small `Δφ` proportional to its distance to
the hole, equivalent to a small frequency offset toward the hole.

- Requires per-channel `pitch_drift_phase[MAX_NUM_BINS]` state.
  Memory: 33 KB per channel.
- CPU: light.
- The drift is bounded — neighbours don't shift more than half a bin,
  preventing them from collapsing into the hole.

### e) Healing rate — recovery curve

**Concept.** When the sidechain peak releases, the carved holes heal
back to their original magnitude/pitch. The healing rate is a curve —
the user can have fast healing (snappy holes) or slow (the spectrum
breathes).

- This is just an envelope on the carve depth, but exposing it as a
  user-facing curve is what gives Punch its character.

### f) "Punch the kick" preset behaviour

**Concept.** A quality-of-life preset that auto-tunes Punch to match
a typical kick drum (carve depth at ~60–80 Hz, healing 200ms). One
button in the UI sets the curves. Not a sub-effect per se but a
preset worth shipping.

## Mode list — final

For v1 ship Direct Punch. Inverse and Self-Punch are quick to add
later as toggle modes.

| Mode | v1 ship? | Reason |
|---|---|---|
| Direct Punch (top-down carve) | yes | The defining effect. |
| Inverse Punch (bottom-up carve) | yes | One inversion, free addition. |
| Self-Punch | defer | Different audience (de-essing); add v2. |

Pitch-Fill and Healing-rate are *behaviours within Direct Punch* not
separate modes, exposed via curves.

## Curves

| Idx | Label | Used by |
|---|---|---|
| 0 | AMOUNT | All modes — carve depth |
| 1 | WIDTH | All modes — how many neighbour bins engaged |
| 2 | FILL_MODE | Per-bin pitch-fill amount (neighbour drift toward hole) |
| 3 | AMP_FILL | Per-bin amp-fill amount (neighbour boost into hole) |
| 4 | HEAL | Per-bin healing rate after sidechain peak releases |
| 5 | MIX | All modes |

6 curves. `num_curves() = 6`.

## Architecture fit

### SpectralModule slot, with sidechain enabled by default

Punch needs from global infra:

- Sidechain plumbing (already shipped — `slot_sidechain[s]`).
- `ModuleContext.sample_rate` (for healing time-constant
  conversions).

Does NOT need:

- BinPhysics (Punch has its own state).
- IF / chromagram / cepstrum.
- MIDI.
- BPM sync.

Per-channel state:

- `prev_carve_depth[MAX_NUM_BINS]` (for hysteresis on healing): 33
  KB per channel.
- `pitch_drift_phase[MAX_NUM_BINS]` (for Pitch-Fill): 33 KB per
  channel.

Total memory: ~66 KB per channel per Punch slot. Cheap.

### Sidechain default-routing UX

Most slots default to no sidechain. Punch is *useless* without one. UX
proposal: when a slot is set to Punch, the editor auto-routes
`slot_sidechain[s] = Sc(0)` (the first aux input) and surfaces a
"sidechain not connected" warning if it's None. This is a small
ModuleSpec hint: `wants_sidechain: bool` in the spec, used by the GUI
to default the routing on first assignment.

See `02-architectural-refactors.md` § ModuleSpec.wants_sidechain.

### Why the user-facing name "Punch" matters

Same naming logic as Past/Future/Life/Kinetics. "Punch" is a producer
term ("this drum has more punch") and immediately suggests transient-
focused, sidechain-driven behaviour. Alternatives ("Carve," "Hole,"
"Negative Space") are descriptive but less evocative.

## CPU class

Light. The peak-detection on the sidechain is the same algorithm
Dynamics already runs (no new heavy work). Per-bin updates are O(N).

`heavy_cpu = false`.

## BinPhysics interactions

Reads: nothing.
Writes: nothing.

Punch is a self-contained effect with its own state.

## Calibration probe set

- `probe_amount_pct`
- `probe_active_mode_idx`
- `probe_active_holes` (number of carve sites currently active)
- `probe_max_carve_depth` (deepest carve currently in flight)
- `probe_avg_drift_offset` (average pitch drift across active fills)

## RESEARCH PROMPT — Spectral hole-and-fill perceptual quality

```
Topic: Perceptual quality of sidechain-driven spectral hole carving
with neighbour-pitch-fill versus neighbour-amp-fill.

Context: A sidechain spectrum drives the carving of "holes" in the
input spectrum at the sidechain's peak frequencies. The surrounding
bins fill the hole either by amplitude boost (the "duck and lift"
behaviour) or by pitch drift toward the hole (the "neighbour fall in"
behaviour). We want to understand which fill mode reads as more
musical / less artefacted.

Specific questions:
1. Pitch fill creates *frequency modulation* of the neighbour bins
   each time the sidechain peaks. Does this artefact-free for
   harmonic content, or does it create audible chirping?
2. Amp fill creates per-hop magnitude jumps. Smoothing across hops
   (1-pole follower) is required to avoid clicks. What time constant?
3. The "depth × width" 2-D space (depth 0–1, width 0–N bins) — what
   region is musically useful? Some combinations are surely just
   pumping artefacts.
4. Healing curve shape: linear, exponential, sigmoid? Which feels
   most like a natural spectral resilience?
5. Sidechain bandwidth: do we want to detect peaks across the entire
   sidechain spectrum, or only within a per-bin "watch range" (the
   user draws a curve specifying where in the spectrum each input
   bin should listen for sidechain peaks)?

Deliverable: Rust implementations of both fill modes with audio
examples on (a) bass + kick, (b) lead vocal + sibilant noise, (c)
drum loop into reverb tail. Comparative listening notes.
```

## RESEARCH PROMPT — Phase-coherent neighbour-bin pitch drift

```
Topic: Per-bin phase-coherent micro-pitch-shifting of small
neighbour bin clusters toward a target bin (the carved hole).

Context: When a hole is carved at bin H, neighbouring bins H±1, H±2,
... drift toward H by a small phase rotation each hop. We need this
drift to be phase-coherent with the existing bin's content — i.e.,
the iFFT shouldn't reveal a click or a discontinuity.

Specific questions:
1. The drift offset is a fraction of a bin (0–0.5 bins). Standard
   phase-vocoder pitch shifting works at integer hops; sub-bin
   drift is a phase rotation = `2π × Δf × hop / sample_rate`. Is
   that sufficient or do we need IF-aware corrections?
2. When the drift releases (sidechain quiets), do we drift back to
   zero or just freeze in place? Drifting back risks a second
   audible motion; freezing creates a small permanent bin shift
   until the next carve.
3. Stereo: in Independent mode, both channels carve independently.
   Should the drift be locked across channels (mono fill) or
   independent (stereo width)?

Deliverable: Reference kernel + comparison of three drift release
strategies (return-to-zero, freeze-in-place, slow-drift-to-zero) on
sustained pad with periodic carve.
```

## Open questions

1. **Module vs sidechain-transform.** Confirm Option A (full module).
   Justification given but worth a sanity check.
2. **`wants_sidechain` in ModuleSpec.** Add as a UX hint? See
   `02-architectural-refactors.md`.
3. **Self-Punch deferral.** Ship for v1 or wait?
4. **Healing curve default shape.** Exponential is the easy default.
   Worth offering linear and sigmoid as preset shapes?
5. **Watch-range curve.** Do we need a per-bin "where in the
   sidechain to look for peaks" curve, or is global peak detection
   across the full sidechain fine? The former is more flexible but
   adds another curve and pushes total to 7 (the curve cap).
6. **Interaction with Dynamics ducking.** A Punch slot followed by a
   Dynamics ducking slot will *both* react to the same sidechain.
   Document the ordering recommendation (Punch first, Dynamics
   second, so Dynamics doesn't compress the holes Punch carved).

## Research findings (2026-04-26)

Hole-and-fill perceptual quality, healing curves, watch-range, and
phase-coherent fill are covered by `research/06-specialized-topics.md`
(Topic C) and `research/05-time-manipulation.md` (Topic C). Validated
decisions:

1. **Default fill mode = amplitude** with **exponential healing
   τ = 150 ms**. Matches McDermott & Oxenham 2008 spectral-completion
   perception literature (~10 dB below masker level) and is the
   cheapest to implement (one mul-add per bin per hop).
2. **Pitch fill is a useful flavour mode** but ship with mandatory
   slew-rate limiting (~2 cents/hop max) to prevent pitch flutter on
   sustained pure tones. User-selectable per slot.
3. **Healing curve shapes:** default exponential. Ship linear and
   sigmoid as named presets in the FILL_MODE curve dropdown.
4. **HEAL curve range:** 20 ms (snappy) to 2000 ms (long pad-like
   recovery); default 150 ms maps to curve value 1.0.
5. **Smoothing on amplitude fill:** 1-pole follower with τ = 5 ms
   (fast enough to track sidechain transients, slow enough to avoid
   block-boundary clicks).
6. **`depth × width` operating range:** musically useful zone is
   depth 0.3-0.8 × width 1-8 bins. Beyond width 16 the carve sounds
   like a duck. Hardcode as the visual range of the curve UI.
7. **Sidechain bandwidth: global per-slot detection in v1.** Defer
   the per-bin watch-range curve to v2 — adds the 7th curve (at the
   limit), the semantics are non-obvious for hand-config, and most
   users will only hit it via presets. If shipped, treat watch-range
   as a **frequency mapping** (each input bin picks one sidechain bin
   to listen to via a curve lookup) — O(N) lookups, not O(N×W).
8. **Pitch-fill phase rotation = exact at `|d| ≤ 0.5` bins.** No
   IF-aware correction needed — the partial never crosses a bin
   boundary by construction. Phase rotation per hop is `Δφ = (π/2) · d`
   at OVERLAP=4. Pre-compute `exp(j·Δφ)` per active drift site, cache
   until release. Cost: 4 muls + 2 adds per drifted bin per hop.
9. **Drift release = slow-drift-to-zero** with τ = 200-500 ms (1-pole
   follower). Cap accumulated `d` at 0.5 bins (don't cross bin
   boundaries; clamp silently if user curves push beyond). Limit
   active drift sites to ~64 per channel.
10. **Reuse `EnvelopeBank` SIMD primitive** from Modulate's Buchla
    envelopes — same 1-pole asymmetric attack/release math.
11. **Reuse `PhaseRotator` helper + `if_offset[k]` cache** in
    `ModuleContext` from the Past Stretch / Future work — same
    `Complex<f32> × (freq_offset, time_delta)` math (see
    `research/05-time-manipulation.md` cross-topic synthesis).
