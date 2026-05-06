# Life Module — Audit

**Existing spec:** `docs/superpowers/specs/2026-04-21-life-module.md`
**Status:** DEFERRED, depends on BinPhysics.
**Source brainstorm:** "Physical effects" section in
`ideas_for_the_wonderful_future.txt` (lines 339+), with Kim's note
"this should go under a wider category of 'Life' algorithms inspired by
biology and chemistry."

## What the spec covers

6 sub-effects: `Viscosity`, `Surface Tension`, `Crystallization`,
`Archimedes`, `Non-Newtonian`, `Stiction`. Curves: AMOUNT, THRESHOLD,
SPEED, REACH, MIX. Reads/writes BinPhysics `displacement`,
`temperature`, `crystallization`, `velocity`.

## Brainstorm cross-reference

The "Physical effects" section in the brainstorm has 21 ideas (20 + a
late "Punch" addition). They span Solids (1-4), Fluids (5-8),
Friction/Heat (9-12), Acoustics (13-16), and Magnetism/Gravity (17-20)
— with Kinetics covering most of Magnetism/Gravity and parts of Solids.

| # | Idea | In Life? | Kim's note | Action |
|---|---|:---:|---|---|
| 1 | Hooke's Law (Rubber Band) | (Kinetics) | "yes with hysteresis and mass" | done elsewhere |
| 2 | Sympathetic Harmonic Springs | (Kinetics) | "tracking the vibrations of the bins themselves" | done elsewhere |
| 3 | Inertial Magnitude (Mass) | (Kinetics) | "things like this, yes this is our mountain" | done elsewhere |
| 4 | Yield Strength & Tearing | ✗ | "Better, amplitude freezes, it's less jarring" | **GAP — refined, add** |
| 5 | Viscosity | ✓ | "I love this idea in general, and it could inform many of the 'Life' effects. Energy does not disappear, it has to go somewhere." | done — but the *conservation* principle deserves explicit doc |
| 6 | Archimedes Displacement | ✓ | "yes this is good stuff. Could have an escape valve" | done — escape valve exists in spec |
| 7 | Capillary Action (Harmonic Wicking) | ✗ | "I think we had that above somewhere" *(it isn't anywhere)* | **GAP — add** |
| 8 | Surface Tension (Coalescence) | ✓ | "Yes, 'Life' stuff also" | done |
| 9 | Stiction | ✓ | "cool, we'll do this" | done |
| 10 | Thermal Expansion (Detuning) | (Kinetics) | "very cool" | done elsewhere |
| 11 | Sandpaper Phase (Granular Friction) | ✗ | "things like this could be something, with the bins talking to each other" | **GAP — add** |
| 12 | Non-Newtonian (Oobleck) | ✓ | "yes, I want to hear this" | done |
| 13 | Chladni Plate Nodes | (proposed Geometry) | "Could the persistent homology patterns of the history buffer inform some kind of geometric translation?" | see `18-geometry.md` |
| 14 | Helmholtz Traps | (proposed Geometry) | "I need a clearer concept for this" | see `18-geometry.md` |
| 15 | Tuning Fork Intermodulation | (Kinetics) | "yes, everything like this forever yes" | see `12-kinetics.md` |
| 16 | Crystallization (Phase Lattice) | ✓ | "the more active the sustained portion is, the more the noisy part gets frozen/contrast-flattened, etc. But could this be a part of a re-upped freeze module?" | done — **but see Open Questions** |
| 17 | Ferromagnetism | (Kinetics) | "Should this be somehow a potential feature of any graph?" | done elsewhere |
| 18 | Orbital Gravity (Phase Rotation) | (Kinetics) | "yes, bring me to it" | done elsewhere |
| 19 | Diamagnetic Repulsion (Spectral Carving) | (Kinetics) | "cool idea for sure, hmm there's now a bunch of such gravity/repulsion/hole etc effects. Maybe one module ?" | see `12-kinetics.md` |
| 20 | Brownian Motion (Thermal Agitation) | ✗ | "maybe indistinct from noise?" | **GAP — keep with reservations** |
| 21 | Punch (sidechain holes) | ✗ | (no annotation; appended late) | see `19-punch.md` |

## Gap details

### a) Yield Strength & Tearing — refined per Kim (idea #4)

**Concept (refined).** Bins act like fabric. Up to a yield amplitude
they stretch (compress smoothly). Above yield, the fabric "tears":
amplitude *freezes* at the yield level (does not drop to zero), and
phase scrambles until amplitude returns below yield + recovery
hysteresis.

**Why Life and not Kinetics.** Yield/tear is a material-science concept,
not a force concept. It's about the *bin's own integrity* under stress,
which is the Life module's framing. Kinetics handles inter-bin forces.

**Sub-effect proposal: `Yield` mode**
- Reads: bin magnitude, `BinPhysics.displacement` (cumulative stress
  from earlier modules).
- Writes: output magnitude (clamped at yield), output phase (scrambled
  if torn). Writes `BinPhysics.displacement` (stress accumulator).
- State: `tear_state[N]` (0 = elastic, 1 = torn, in (0,1) = healing).
- Curves: AMOUNT, THRESHOLD (yield strength), SPEED (heal rate), MIX.
- CPU: light.

### b) Capillary Action — Harmonic Wicking (idea #7)

**Concept.** Sustained loud bins slowly leak magnitude into higher,
quieter bins — water climbing a paper towel. Causes sustained bass
notes to bloom into bright, harmonic-rich pads over time.

**Why Life.** This is fluid behaviour in spectrum-space. Kim's note
suggests this overlaps with Tape Print-Through (Past) and Capillary
should be the *upward-sustained* version. Different time-base and
spectrum direction make it a distinct sub-effect.

**Sub-effect proposal: `Capillary` mode**
- Reads: bin magnitude, sustained-energy estimate (per-bin slow LP
  envelope).
- Writes: output magnitude (some stolen from source bin, deposited at
  K + reach * step).
- State: `wick_envelope[N]` (slow LP), `wick_carry[N]` (in-flight
  energy moving upward).
- Curves: AMOUNT, REACH (how far up to wick), SPEED (climb rate),
  THRESHOLD (minimum sustain to start wicking), MIX.
- CPU: medium (two passes — drain source, deposit at target).

### c) Sandpaper Phase — Granular Friction (idea #11)

**Concept.** Two adjacent bins with high magnitude but radically
different phase "rub" against each other. The rub generates a high-
frequency injection — sparks of distortion in the upper spectrum
proportional to phase mismatch in the low/mid.

**Why Life.** Kim's note "the bins talking to each other" — bin-to-bin
behaviour, not material-stress (Yield) or fluid-flow (Viscosity). It's
its own family. Could equally fit in Modulate (since it's phase-driven)
but the *generative spark* output is Life-flavoured.

**Sub-effect proposal: `Sandpaper` mode**
- Reads: pairs of adjacent bins (magnitudes + phases).
- Writes: output magnitudes elsewhere in the spectrum (the spark
  destinations — some logarithmic offset upward from the rub site).
- State: none beyond what BinPhysics already provides.
- Curves: AMOUNT, REACH (where sparks land — short = local distortion,
  long = airy crackle in the highs), THRESHOLD (minimum phase mismatch
  to trigger), MIX.
- CPU: medium.

### d) Brownian Motion (idea #20)

**Concept.** A "Temperature" knob that randomly drifts magnitudes and
phases. At absolute zero the FFT is mathematically perfect; at high
heat it's analog-modeled hiss.

**Kim's reservation:** "maybe indistinct from noise?" Valid concern. A
plain noise injection is uninteresting. The interesting version uses
`BinPhysics.temperature` *that other modules wrote* — bins that have
been worked hard by upstream modules drift more. Now it's emergent
behaviour, not just static hiss.

**Sub-effect proposal: `Brownian` mode**
- Reads: `BinPhysics.temperature` (set by Circuit, Kinetics Thermal
  Expansion, etc.).
- Writes: output magnitude + phase with random walks scaled by
  temperature.
- State: `rng_state` (single u64, xorshift).
- Curves: AMOUNT (scaling on temperature → drift), MIX. (Just two —
  this is a polish mode.)
- CPU: light.

If users find this bland, drop it; but tied to upstream temperature
it's *the* mode that makes the BinPhysics chain audible as cumulative
weathering. Worth shipping.

## Architectural questions

### Question: Crystallization scope vs Freeze module

Kim asked: "could this be a part of a re-upped freeze module?"

**My read:** they should remain separate, but with explicit interaction.

Freeze (current shipped module) captures and replays a frozen frame
with portamento — the user actively triggers it and chooses a freeze
moment. The current code (see `src/dsp/modules/freeze.rs`) is a
threshold-driven freeze on a per-bin basis with hold-hops counters.

Crystallization (Life sub-effect) is a *gradual* phase-locking process
that emerges from sustained tonality — not a freeze, a coherence-grow.
The user cannot trigger it; it just happens.

**Proposed interaction:** Life Crystallization writes
`BinPhysics.crystallization` when active. Freeze reads it as an
*additional* freeze trigger — a fully crystallized bin will accumulate
in Freeze faster. Inverse direction: Freeze writes
`BinPhysics.crystallization = 1.0` for currently frozen bins, so a
downstream Life Crystallization mode "knows" those bins are already
locked.

This makes the two cooperate without merging. **Question:** is that
worth specifying in the BinPhysics spec rewrite, or is it a Life
implementation detail?

### Question: energy-conservation as the Life invariant

Kim's note: "Energy does not disappear, it has to go somewhere."

The spec's Viscosity sub-effect says "energy is approximately conserved"
but Surface Tension and Crystallization don't address conservation. Kim
treats it as the binding theme of the whole module.

**Proposal:** make energy conservation a stated invariant of the Life
module — every sub-effect with energy redistribution honours it within
some tolerance. Sub-effects that *create* or *destroy* energy
(Crystallization, Yield/tear) state explicitly that they are exempt.

This becomes a documentation discipline, not a code discipline. But it
helps users predict "if I crank Life, do I lose level?"

## Curve set

The spec proposes 5 curves. With the new Capillary, Yield, and
Sandpaper modes added we may want a 6th, but the labels can be
reused:

| Idx | Label | Used by |
|---|---|---|
| 0 | AMOUNT | All modes |
| 1 | THRESHOLD | Viscosity, Stiction, Non-Newtonian, Yield, Capillary, Sandpaper |
| 2 | SPEED | Viscosity, Crystallization, Yield (heal rate), Capillary (climb rate) |
| 3 | REACH | Viscosity, Surface Tension, Capillary (distance up), Sandpaper (spark distance) |
| 4 | MIX | All modes |

5 curves still fit. Keep `num_curves() = 5`.

## CPU class

Spec calls it medium. With Capillary (two-pass) and Sandpaper (cross-
bin) added, it nudges toward medium-heavy. Tag `heavy_cpu = false` —
none of these are pathological. Viscosity is the most expensive (already
two-pass).

## BinPhysics interactions

Reads: `displacement`, `temperature`, `crystallization`, `velocity`,
new field `bias` (from Yield's stress accumulator).
Writes: `displacement`, `crystallization`, `temperature` (Brownian
amplifies), `bias`.

## Calibration probe set

For each mode:
- `probe_amount_pct`
- `probe_threshold_*` (units depend on mode — yield strength, viscosity
  threshold, etc.)
- `probe_reach_bins` (number of neighbouring bins affected)
- `probe_active_mode_idx`
- `probe_mode_state` (per-mode internal state at probe bin — tear_state
  for Yield, wick_carry for Capillary, crystallization for the
  Crystallization mode)

## Module ordering implications

Multiple Life sub-effects in series gives a chain like "viscosity-blur
+ surface-tension-coalesce." The current architecture supports this:
two slots, both Life, different modes. **Question:** is there demand
for a single Life slot to run two modes? Multi-mode-per-slot was a
deferred question for Circuit too. Same answer: v1 single, v2 stack.

## RESEARCH PROMPT — energy-conservation in spectral diffusion

```
Topic: Energy-conserving spectral diffusion operators

Context: We have a spectral plugin doing per-hop magnitude redistribution
across an STFT (8193 bins, sub-millisecond budget per hop). We want a
diffusion operator (per Kim's "energy must go somewhere") that:
- Spreads loud-bin magnitude into adjacent bins
- Conserves total energy (sum of |bin|^2) within tolerance
- Is SIMD-friendly (stride-1 reads/writes)
- Has a per-bin reach parameter (graph-driven)

Specifically: is a discrete heat equation (1D Laplacian smoothing of
magnitude) sufficient, or do we need a Lattice Boltzmann or finite-
volume scheme to get visually-correct conservation? Audio-perceptually,
is the difference noticeable, or does any conservative-enough scheme
sound fine?

Bonus: how does adding "viscosity" as a per-bin diffusion coefficient
interact with stability — does the operator become non-monotone, do
we need flux-limiters?

Deliverable: One Rust kernel with explicit conservation analysis
(numerical + audio-perceptual), reference to literature, and a
benchmarked comparison of plain Laplacian vs flux-limited.
```

## Open questions

1. **Crystallization x Freeze interaction:** specify explicitly in
   BinPhysics rewrite, or leave as Life-internal?
2. **Energy conservation as invariant:** doc only, or a CI test that
   measures pre/post energy ratios?
3. **Sandpaper Phase home:** Life or Modulate?
4. **Brownian Motion ship-or-shelve:** ship behind a "USE BINPHYSICS
   TEMPERATURE" toggle so that without upstream temperature it has
   nothing to do.
5. **Multi-mode-per-slot:** v1 single, v2 stack — confirm.

## Research findings (2026-04-26)

Diffusion is covered by `research/03-physical-models.md` (Topic B).
Validated decisions:

1. **Diffusion scheme = plain finite-volume FTCS on `|X|^2` (power)**
   with harmonic-mean face flux for per-bin viscosity:
   ```rust
   let d_face_right = 2.0 * d[k] * d[k+1] / (d[k] + d[k+1] + EPS);
   let d_face_left  = 2.0 * d[k] * d[k-1] / (d[k] + d[k-1] + EPS);
   p_new[k] = p[k] + d_face_right * (p[k+1] - p[k])
                   - d_face_left  * (p[k]   - p[k-1]);
   ```
   6 muls + 4 adds + 2 divs per bin, vectorisable across bins.
2. **Operate on power, output magnitude.** `mag = |bin|`, `power =
   mag²`, diffuse, `mag_new = sqrt(power_new)`, scale the complex bin
   by `mag_new / mag`. Phase preserved. Conservation in `power` is
   what matches perceptual loudness conservation.
3. **Clamp `D[k] ∈ [0, 0.45]`** for FTCS stability across all hop
   rates (0.5 is the strict bound; 0.45 is 10% safety margin).
4. **Reflective boundaries by default.** First and last bin: clamp
   flux to zero (`J[-1/2] = J[N-1/2] = 0`). Exact conservation
   including boundary, no ghost-cell tricks.
5. **DEAD ENDS:**
   - Lattice Boltzmann — sub-threshold audible benefit at our `D`
     range for 2× state and complexity.
   - Flux-limited TVD / Superbee — addresses overshoot in *advection*,
     irrelevant for diffusion of a positive magnitude spectrum.
   - Crank-Nicolson — tridiagonal solve per step is overkill; FTCS at
     `D ≤ 0.45` is conservative to within 1e-7 per hop relative error.
6. **Same Mathieu / 1-pole-smoothing requirement as Kinetics** — apply
   the shared `clamp_for_cfl()` and 1-pole curve smoothing helpers
   across all per-bin parameter curves before they reach the integrator.
7. **Borrow WORLD CheapTrick spectral envelope** (BSD-licensed,
   ~300 lines to port to Rust) for the Capillary / Healing modes if
   they need a smooth-envelope baseline; see
   `research/02-pitch-and-cepstral.md` Topic B.
