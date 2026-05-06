# Kinetics Module — Audit

**Existing spec:** `docs/superpowers/specs/2026-04-21-kinetics-module.md`
**Status:** DEFERRED, depends on BinPhysics.
**Source brainstorm:** scattered — Hooke / Sympathetic Springs / Mass
in the "Physical effects" Solids section, Gravity Wells in the original
brainstorm "PVX" section, Magnetism/Gravity Category 5 in the Physical
section, plus Kim's "Kinetics" name idea.

## What the spec covers

6 sub-effects: `Hooke`, `Gravity Well`, `Inertial Mass`, `Orbital Phase`,
`Ferromagnetism`, `Thermal Expansion`. Curves: STRENGTH, MASS, REACH,
DAMPING, MIX. Reads/writes BinPhysics `velocity`, `displacement`,
`mass`, `flux`, `phase_momentum`, `temperature`. Two-pass schemes for
spring connections.

## Brainstorm cross-reference

Kinetics covers most of Categories 1 (Solids) and 5 (Magnetism/Gravity)
of the Physical effects list, plus the original brainstorm's
"Spectrum stretch / gravity wells" and "Gravity phaser" ideas.

| # | Idea | In spec? | Kim's note | Action |
|---|---|:---:|---|---|
| 1 | Hooke (Rubber Band) | ✓ | "yes with hysteresis and mass with a vector of movement" | done |
| 2 | Sympathetic Harmonic Springs | ✓ | "tracking the vibrations of the bins themselves? could be something" | done — covered as variant of Hooke |
| 3 | Inertial Mass | ✓ | "this is our mountain" | done |
| 10 | Thermal Expansion (Detuning) | ✓ | "very cool" | done |
| 15 | Tuning Fork Intermodulation | ✗ | "yes, everything like this forever yes" | **GAP — add** |
| 17 | Ferromagnetism (Phase Alignment) | ✓ | "Should this be somehow a potential feature of any graph?" | done — but Kim's question is open |
| 18 | Orbital Gravity (Phase Rotation) | ✓ | "yes, bring me to it" | done |
| 19 | Diamagnetic Repulsion (Spectral Carving) | ✗ | "cool idea for sure" | **GAP — add** |
| Brainstorm: Spectrum Stretch with gravity wells | (Gravity Well) | "Possibly tracks midi" | done — **MIDI tracking missing in spec** |
| Brainstorm: Gravity phaser | (Modulate) | (covered) | done elsewhere |
| Brainstorm: Lag for partial envelopes | ~partial | "sidechain tracks speed of change of input which constricts partial amplitude rate of change, not amplitude" | **GAP — sidechain-driven mass mode** |

## Gap details

### a) Tuning Fork Intermodulation (idea #15)

**Concept.** Loud bins are mathematically hardened into "tuning forks" —
their vibration physically modulates the phase of nearby quieter bins.
Distance-dependent. Generates physical-sounding beat frequencies and
metallic chorusing.

**Why Kinetics.** Force-at-distance from a peak — same family as
Gravity, Orbital, Ferromagnetism. The output is phase modulation of
neighbours, fitting the existing `phase_momentum` field.

**Sub-effect proposal: `Tuning Fork` mode**
- Reads: bin magnitudes (find loud peaks via threshold), neighbour
  phases.
- Writes: neighbour phases (modulated at tuning-fork frequency,
  amplitude-decayed by distance), `phase_momentum`.
- State: per-active-fork frequency snapshot (small list, ≤16 forks).
- Curves: STRENGTH (modulation depth), THRESHOLD (peak detection
  level), REACH (how many neighbours per fork), MIX.
- CPU: medium — the per-fork loop is small once the fork list is
  computed; the peak-find pass is O(N) with simple threshold.

### b) Diamagnetic Repulsion — Spectral Carving (idea #19)

**Concept.** Certain frequency bands defined by the curve are highly
diamagnetic — they violently repel energy. Energy approaching these
zones is *physically accelerated and pushed into neighbours*, preserving
total volume but carving a hard physical hole in the mix.

**How this differs from a notch EQ.** A notch EQ removes energy. This
*relocates* it — the carve preserves total energy. Audibly, the area
around the carve is *louder* than it would be otherwise, because the
displaced energy went there.

**Why Kinetics.** It's a force concept (repulsion). Kim's note hinted
at consolidating "gravity / repulsion / hole" into one module, and
that's what Kinetics already is.

**Sub-effect proposal: `Diamagnet` mode**
- Reads: bin magnitudes, current displacement field.
- Writes: bin magnitudes (energy moved out of the carve zone into
  neighbours, weighted by distance), `displacement`.
- State: scratch buffer for the source magnitudes (read-pass).
- Curves: STRENGTH (repulsion amount), REACH (how far displaced energy
  travels), THRESHOLD (where in the spectrum the curve crosses into
  carve zones), MIX.
- CPU: medium (two-pass).

### c) MIDI-tracked gravity wells

The brainstorm's Spectrum Stretch / Gravity Wells: "Possibly tracks
midi, possible to set number of virtual partials with amplitude decay."

The spec says wells "can track MIDI or sidechain peaks" but does not
say *how* MIDI is supplied. Once `01-global-infrastructure.md` §4
ships MIDI, this becomes:

- Wells can be in three modes: **Static** (curve-positioned),
  **Sidechain** (positioned at sidechain peaks), **MIDI** (positioned
  at held-note frequencies).
- In MIDI mode, the user sets a "harmonic count" — wells appear at
  f_root × {1, 2, 3, …, N} per held note, with amplitude decay over
  the harmonic series (controlled by a curve).
- Held-chord MIDI = many simultaneous wells; the per-well strength
  drops to keep total influence finite.

This refines the existing Gravity Well mode without adding a new sub-
effect. Add a `WellSource` enum field. UI: a small mode selector under
the Gravity Well section.

### d) Sidechain-driven mass (Lag for partial envelopes)

Kim's brainstorm: "sidechain tracks speed of change of input which
constricts partial amplitude rate of change, not amplitude."

This is `BinPhysics.mass` driven by an external sidechain envelope:
when the sidechain is changing fast, mass is high (partials respond
slowly). When the sidechain is steady, mass drops (partials respond
quickly).

**Refinement to existing `Inertial Mass` sub-effect.** Add a
`MassSource` enum: `Static` (existing — curve-driven), `Sidechain` (the
new behaviour, sidechain rate-of-change → per-bin mass).

This is a small addition. UI: a mode dot on the mass curve.

### e) Ferromagnetism: "feature of any graph?" (Kim's question on #17)

Kim asked whether phase-alignment-toward-loud-peaks should be available
to *any* curve, not just a Kinetics sub-effect.

**Read.** This is a Modulation Ring–style cross-cutting idea. Any curve
could optionally include "ferromagnetic snapping" as a per-curve
modulation. But this is a UI generalisation that touches every curve in
every module — vast scope creep.

**Recommendation:** ship Ferromagnetism as a Kinetics sub-effect (per
spec). If users love it, design a global "Ferromagnetic Snap" Modulation
Ring option in v2 that any curve can opt into.

## Architectural questions

### Question: Kinetics vs BinPhysics framing

`01-global-infrastructure.md` §7 argued that BinPhysics is the global
"kinetics" function and the Kinetics module is the editing UI for it.

This means:

- BinPhysics fields (`velocity`, `mass`, `displacement`,
  `phase_momentum`) are the *what*.
- Kinetics module is the *user-facing knob* to set them.
- Other modules read/write the same fields — Life, Circuit, Past, etc.
- A Kinetics-less plugin where BinPhysics is on still has every other
  module benefitting from accumulated physics state.

The Kinetics spec should explicitly say "this module is one way the
user shapes BinPhysics; the system works without it." Right now the
spec reads as if Kinetics owns the physics fields, which is the wrong
framing.

### Question: order of Kinetics in the chain

If a user puts Kinetics first, downstream modules see physics state
that was deliberately authored. If they put it last, the module is
*reacting* to whatever upstream did. Both are useful; both are
supported by the matrix routing.

A typical preset might do:
- Slot 1: Kinetics with curve-driven mass (sets up "this band is
  heavy").
- Slot 2: Dynamics (compressor — sees the inertial mass and reacts
  more sluggishly to heavy bands). [Note: Dynamics doesn't read
  BinPhysics today; would need a small extension.]
- Slot 4: Life Yield (catastrophic state when stress exceeds yield).
- Slot 8 (Master): output.

This kind of preset is the *point* of BinPhysics. The Kinetics file
should include a "preset stories" section motivating the architecture.

## Curve set

Spec proposes 5: STRENGTH, MASS, REACH, DAMPING, MIX. With Diamagnet
and Tuning Fork added, still 5 fits — re-using REACH for distance
parameters is natural.

| Idx | Label | Used by |
|---|---|---|
| 0 | STRENGTH | All modes — force magnitude |
| 1 | MASS | Inertial Mass, Gravity, Springs, Tuning Fork (mass = "fork rigidity") |
| 2 | REACH | Gravity, Springs, Orbital, Ferromagnetism, Tuning Fork, Diamagnet |
| 3 | DAMPING | Springs, Orbital, Tuning Fork, Diamagnet |
| 4 | MIX | All modes |

## CPU class

Spec calls it medium with springs being the most expensive. Tuning Fork
and Diamagnet add medium-cost passes. **`heavy_cpu = true`** if a user
selects Springs, Tuning Fork, or Diamagnet. Lighter modes (Mass,
Thermal Expansion) are fine.

A more honest model: a per-mode `heavy_cpu` flag, not a per-module
flag. The `ModuleSpec` extension in `02-architectural-refactors.md` §1b
should support `heavy_cpu_per_mode: &'static [bool]` indexed by mode
position.

## BinPhysics interactions

Reads: `velocity`, `displacement`, `mass`, `flux`, `phase_momentum`,
`temperature` (Thermal Expansion).
Writes: `velocity`, `displacement`, `mass`, `flux`, `phase_momentum`,
`temperature`.

This is the most physics-active module of the lot — everything it does
touches BinPhysics. Get it landed as the first user-facing Kinetics
implementation; it will exercise most of the BinPhysics infra.

## Calibration probe set

Per mode:
- `probe_strength` (force magnitude at probe bin)
- `probe_mass`
- `probe_displacement`
- `probe_velocity`
- `probe_active_mode_idx`
- `probe_well_count` (Gravity, Tuning Fork — number of active sources)

## RESEARCH PROMPT — Numerical stability of spring networks at audio rates

```
Topic: Stable spring/mass simulation across an FFT spectrum at audio
hop rate.

Context: We want each FFT bin to behave as a mass connected to its
neighbours by springs (and optionally to harmonic-related bins).
Update happens once per STFT hop — at 44.1 kHz / hop=512, that's
~86 Hz update rate. At 44.1 kHz / hop=128, ~344 Hz. Springs are
stiff; mass varies per bin; user-driven curves can change everything
between hops.

Goal: A semi-implicit or symplectic integrator that:
- Stays stable across the hop rate range without per-hop substepping
  (substepping kills CPU)
- Handles fast user changes to spring constants without exploding
- Is SIMD-friendly (per-bin mass + displacement + velocity arrays,
  adjacent reads for spring forces)

Specific questions:
1. Verlet vs Velocity Verlet vs Implicit Euler: which gives the best
   stability/CPU tradeoff at our update rate?
2. Stiffness limits: at what max spring constant does the integrator
   blow up at hop=512? At hop=128?
3. When the user modulates spring stiffness fast (per-hop), do we get
   parametric amplification (audible "ringing up" of the simulation)?
   How to damp safely without losing the desired ringing?
4. Sympathetic harmonic springs: bin K connected to 2K, 3K, 4K. The
   memory access pattern is non-stride-1. Worth a CSR-style sparse
   matrix with cached neighbour offsets, or per-bin small fixed array?

Deliverable: a Rust integrator skeleton with stability proof / numerical
analysis, plus a per-mode CPU benchmark estimate at 8193 bins.
```

## RESEARCH PROMPT — Phase-orbit math for Orbital sub-effect

```
Topic: Computing physically plausible "orbit" of a phase around a
mass-attracted center, per FFT hop.

Context: A Kinetics sub-effect treats loud bin K as a "massive object"
and lighter neighbour bins K±n as "satellites." Each satellite's phase
is forced to orbit the massive bin's phase at a rate proportional to
1/distance.

Goal: Define what "orbit" means in phase space such that:
- The output sounds like physically modeled vibrato + spatial width
- Zero phase = the satellite is "behind" the massive bin in some sense
- Two satellites at opposite distances from the same mass orbit in
  opposite directions (creating beat patterns)
- Multiple massive bins create non-trivial orbit perturbations
  (audible chorus, not noise)

Open question: do we use Kepler-style elliptical orbits (cool but
expensive) or just rotate the satellite phase by a per-hop angle
proportional to 1/distance × strength (cheap, sounds like phase-
modulated chorus)?

Deliverable: a formula + audio examples literature reference.
```

## Open questions

1. **Per-mode heavy_cpu flag** vs per-module: would let Mass / Thermal
   stay light while flagging Springs / Diamagnet / Tuning Fork as heavy.
2. **Dynamics module reading BinPhysics:** worth retrofitting? It's a
   shipped module. Would let "mass changes how compressors react." Adds
   a feature to a feature-frozen module — defer to v2.
3. **Kinetics-as-editing-UI framing:** rewrite the spec intro?
4. **Verlet vs implicit integrator** for springs: see RESEARCH PROMPT.
5. **Phase-orbit math:** see RESEARCH PROMPT.
6. **Ferromagnetic-snap as global per-curve modulation:** v2 question.

## Research findings (2026-04-26)

Spring/mass integrators and Orbital phase rotation are covered by
`research/03-physical-models.md` (Topic A) and
`research/06-specialized-topics.md` (Topic A). Validated decisions:

1. **Integrator = Velocity Verlet** (= Stoermer-Verlet) with SoA layout
   over `displacement[]`, `velocity[]`, `mass[]`, `stiffness[]`. Same
   stability bound as Symplectic Euler (`omega·dt < 2`) but
   second-order accurate at no real CPU cost. Bilbao's *Numerical
   Sound Synthesis* is the canonical reference.
2. **CFL ceiling: clamp `omega_max < 1.5/dt`** (50% safety margin from
   the strict bound). Express user-facing stiffness as **angular
   frequency** (Hz × 2π), not raw spring constant — then the CFL bound
   becomes a single visible "max spring frequency" displayed in the
   UI per FFT-size choice. At hop=512 / sr=44100 this caps spring
   resonance at ~50 Hz; the user sees this and understands. Substepping
   the integrator (8× oversample for the Springs slot only) is a v2
   path that lifts the ceiling to ~400 Hz at the cost of CPU.
3. **CRITICAL: 1-pole-smooth all per-bin parameter curves at hop
   boundaries** (time constant ≈ 4·dt) before the integrator sees
   them. User-knob modulation at hop boundaries is **Mathieu
   parametric forcing** — there are well-defined instability tongues
   at hop-rate harmonics that pump the chains into ringing artefacts.
   Without this smoothing the simulation will silently mis-behave.
   Single shared helper across Kinetics / Life / Geometry.
4. **Per-bin viscous damping ≥ 0.05 minimum.** Makes the system
   provably stable under all parameter modulations within the CFL
   bound.
5. **Per-bin energy-rise clamp** as runtime safety net: scale
   velocities by sqrt(0.5) on any bin where kinetic+potential energy
   doubles in *2 consecutive* hops (hysteresis avoids triggering on
   legitimate transients). Cheap, branchless after SIMD compare.
6. **DEAD ENDS:**
   - Forward Euler — unconditionally unstable for undamped oscillators.
   - Backward Euler — over-damps, kills the spring character.
   - Implicit midpoint — Dinev/Liu 2018 shows it *explodes* on stiff
     systems; per-step Newton solve is too expensive anyway.
   - Standard XPBD — Gauss-Seidel sequencing kills SIMD. Only
     Jacobi-XPBD would work and is undocumented in the literature.
7. **Sympathetic harmonic springs cap = 8 harmonics.** Document as v1
   limitation. Beyond this the gather pattern degrades.
8. **Orbital sub-effect: linear phase rotation `Δφ = α · S_K / d²` for
   v1.** ~3 ops per (master, satellite) pair, well within Kinetics'
   CPU class. Multiply by `sign(distance)` so satellites at K-3 and
   K+3 orbit in opposite directions; multiply by
   `sign(unwrapped_phase_master_velocity)` so the orbit *follows* the
   master rather than drifting independently — this is what stops the
   result sounding like noise. Cap satellite count per master at ~16
   bins on each side; beyond, 1/d² makes the contribution inaudible.
9. **Symplectic-Euler "mini orbit"** with per-satellite `(φ, ω)` state
   is a v2 upgrade — 6 ops/satellite, sounds like ebb-and-flow chorus.
10. **Skip Kepler entirely.** Beautiful in theory, indistinguishable
    from cheap linear phase rotation through the iFFT, ~30× the cost.
11. **Tie satellite list to existing peak detection** (Harmony's pitch
    tracker output, or PLPV peak set). Avoid an Orbital-specific peak
    detector — it's redundant.
