# Geometry Module — NEW Module Proposal

**Existing spec:** none — this file is the first proposal.
**Status:** RESEARCH — not yet specced.
**Source brainstorm:** Cat 4 #13 (Chladni Plate Nodes), Cat 4 #14
(Helmholtz Traps), Kim's hint about "persistent homology patterns of
the history buffer," and the underlying observation that several
brainstormed effects don't fit the 1-D spectrum metaphor cleanly.

## Why Geometry is its own module

Most spectral modules treat the spectrum as a flat 1-D array of bins.
A handful of brainstormed ideas implicitly require a *2-D embedding*
of the spectrum:

- **Chladni plate** maps the 1-D spectrum to vibration modes of a 2-D
  metal plate. Bins become positions on the plate; energy distributes
  according to the plate's eigenmodes.
- **Helmholtz trap** treats certain frequency neighbourhoods as
  *cavities* with a finite volume (energy capacity) and a single
  resonant mode each — geometric in the sense that "cavity volume"
  and "neck cross-section" are 2-D acoustic concepts.
- **Persistent homology of the history buffer** (Kim's note) is a
  topological tool — it identifies persistent peaks, valleys, and
  saddles in the 2-D (time × frequency) STFT magnitude landscape.
- A future **wavefield / 2-D wave equation** simulator could let bins
  exchange energy through a 2-D substrate (e.g. a string fixed at
  both ends, or a drum head).

Cramming these into Life or Kinetics would muddle those modules'
identities. Life is *biology / chemistry / fluid dynamics* on a 1-D
manifold. Kinetics is *spring-mass-damper* on a 1-D chain. Geometry
is the home for genuinely 2-D / topological math that needs its own
mental model.

The key constraint: any 2-D math we do has to *project back to 1-D*
each hop, because the iFFT consumes 1-D bins. So Geometry modes are
"think in 2-D, output in 1-D."

## Sub-effects

### a) Chladni Plate Nodes

**Concept (per Cat 4 #13).** Map the 1-D spectrum to a 2-D plate of
size `M × N` bins. Each bin becomes a position on the plate. The
plate has natural vibration modes (eigenmodes of the 2-D wave
equation): nodes (zero vibration) and antinodes (max vibration). When
you "shake" the plate by injecting energy at one frequency, the
plate's eigenmodes redistribute that energy spatially. Over time, the
spectral "sand" settles at the nodes (which are dark, because nodes
are zero-amplitude).

**Mathematical sketch.**

For a square plate, the eigenmodes are:
```
ψ_{m,n}(x, y) = sin(mπx / L) · sin(nπy / L)
```
with eigenfrequencies proportional to `√(m² + n²)`.

For our purposes:

1. Map bin index `k` ∈ [0, num_bins) to a 2-D coordinate `(x, y)`
   via a Hilbert curve (preserves locality) or row-major (simpler).
2. Pick a base mode `(m, n)` from the curve at that bin's frequency.
3. The "settle force" on bin `k` is proportional to `|ψ_{m,n}(x, y)|`.
4. Bins at antinodes get their magnitude *suppressed* over time;
   bins at nodes get the displaced energy *added*.
5. Conservation: total magnitude per hop is preserved; only the
   spatial distribution changes.

**State per channel:** `plate_phase[MAX_NUM_BINS]` (current settle
phase), `plate_mode_index` (which (m, n) is active). Memory ~33 KB.

**Curves:**

- AMOUNT: settle rate (how fast energy drifts toward nodes per hop).
- MODE: which eigenmode (curve maps bin frequency to (m, n) index).
- DAMPING: how much energy is lost per hop (vs. perfect conservation).
- MIX: dry/wet.

**CPU class:** light. The Hilbert mapping is precomputed; per-hop
work is O(N) bin updates.

### b) Helmholtz Traps — finite-capacity dynamic EQ

**Concept (per Cat 4 #14, Kim asked for a clearer concept).** A
Helmholtz resonator is a cavity with a single resonant frequency
determined by `f_r = (c / 2π) √(A / (V·L))` — cavity volume `V`,
neck area `A`, neck length `L`. The acoustical model: a parallel LC
circuit with damping.

In our spectrum:

1. The user defines N traps (1–8) by curve-shaping a "trap
   activation" curve over the spectrum.
2. Each trap has its own resonant frequency (centre of the curve
   peak) and finite capacity (set by curve height).
3. Energy in bins inside the trap's bandwidth flows *into* the trap
   each hop.
4. When the trap exceeds capacity, it *spills over* — releases the
   excess as a re-injection at the trap's resonant frequency *and*
   its first overtone.
5. When the trap is below capacity, it acts as a soft notch (eats
   incoming energy).

This is genuinely a dynamic EQ with a feedback path that none of the
shipped modules have. The "spill over" creates the dirty-amp /
overflowing-cavity sound Kim wanted.

**State per channel per trap:** `fill_level[N_TRAPS]` (current energy
in the trap), `release_phase[N_TRAPS]` (fractional position in the
overflow envelope). Memory: trivial (8 traps × 8 bytes).

**Curves:**

- AMOUNT: trap depth (how aggressively bins are absorbed).
- CAPACITY: trap fill capacity per bin (curve sets per-trap
  capacity).
- RELEASE: how fast the trap drains when input quiets.
- THRESHOLD: overflow threshold.
- MIX.

**Resonance dynamics.** When overflow triggers, the released energy
gets distributed across several hops (envelope) so it doesn't all
appear in one hop. Use a per-trap exponential release: each hop,
release `τ × overflow_remaining`.

**CPU class:** light. ~8 traps, each does a small bin range per hop.

### c) Wavefield — 2-D wave-equation substrate

**Concept.** Map bins to a 2-D `M × N` grid (Hilbert-curve, same as
Chladni). Run a 2-D wave equation step each hop: each grid cell's
amplitude is updated based on its neighbours. Bins exchange energy
through this substrate, producing acoustic-like ringing and resonance
patterns that no 1-D effect can replicate.

The 2-D wave equation discretized with finite differences:
```
u(x, y, t+dt) = 2u(x,y,t) - u(x,y,t-dt) + c² · dt² · ∇²u
```
where `∇²u = u(x+1,y) + u(x-1,y) + u(x,y+1) + u(x,y-1) - 4u(x,y)`.

This is well-known for plate/membrane synthesis. CPU is moderate:
each grid cell needs ~6 multiply-adds per step. At 8193 bins on a
~91×91 grid, that's ~50k mac per channel per hop. Fits a 4-channel
SIMD loop nicely.

**State per channel:** `u_curr[M×N]`, `u_prev[M×N]`. Memory: 8193 ×
2 × 4 bytes ≈ 64 KB per channel.

**Curves:**

- AMOUNT: input energy injected each hop.
- WAVE_SPEED (`c`): controls dispersion / ringing pitch.
- DAMPING: per-step energy loss.
- BOUNDARY: curve sets per-bin boundary reflectivity (0 = absorbing,
  1 = perfectly reflective). Lets the user "carve" the boundary
  shape spectrally.
- MIX.

**CPU class:** moderate-to-heavy. Mark `heavy_cpu = true` for this
mode. The wave step doesn't vectorize as cleanly as 1-D updates
because of the cross-row neighbour access.

### d) Persistent-homology-driven Reconstruction

**Concept (per Kim's hint).** Persistent homology, applied to the
2-D (time × frequency) magnitude landscape from the History Buffer,
identifies *persistent peaks and saddles* — features that survive
across many time slices. We can use those features to reconstruct a
"de-noised" or "feature-emphasized" spectrum:

1. Build the 2-D grid of magnitudes from the History Buffer
   (frames × bins).
2. Run persistent-homology analysis to identify the most persistent
   maxima.
3. Re-synthesize the current frame using *only* the bins that
   participate in those persistent features (others muted or
   smoothed).

This is a *tracking* / source-separation style effect. Subjectively:
it strips transient and noise content, leaving the spectrum's
"skeleton."

**Cost.** Persistent-homology analysis is O(N log N) in the number
of features, but with N up to 8193 × 32 frames, the constant factor
matters. Real-time persistent homology is an active research area.
For our purposes:

- Run analysis at a reduced rate (every 4 hops, say).
- Cache the persistence diagram and re-use it for 4 hops.
- Smooth between updates.

**State:** History Buffer (already a global resource per
`13-past.md`); cached persistence diagram (~8 KB).

**Curves:**

- AMOUNT: dry/wet of the reconstruction.
- PERSISTENCE_THRESHOLD: minimum persistence value for a feature to
  count.
- TIME: history window length to analyze.
- MIX.

**CPU class:** heavy. This is the most expensive mode in the module.
Recommend `always_bypassed_on_low_end` flag (see
`02-architectural-refactors.md`).

### e) Hilbert / Z-order projection (utility, not a sub-effect)

The 1-D-to-2-D mapping is the same for Chladni, Wavefield, and
potentially Persistent Homology. Implement once as a precomputed
LUT inside the module:

- Hilbert curve: locality-preserving, ideal for wave equation
  (neighbours in 2-D are neighbours in 1-D).
- Z-order (Morton): cheaper to compute, less locality.

Use Hilbert. Precompute at `reset()` for the current `num_bins`.
Memory: 8193 × 4 bytes = 32 KB per channel.

## Mode list — initial v1 cut

For v1 ship Chladni and Helmholtz Traps. They are the cheapest
modes and exercise the 2-D-projection plumbing without committing
to the heavy Wavefield or Persistent Homology paths.

| Mode | v1 ship? | Reason |
|---|---|---|
| Chladni Plate | yes | Cheap, distinctive, exercises 2-D mapping. |
| Helmholtz Traps | yes | Genuinely novel dynamic EQ; light CPU. |
| Wavefield | defer | Heavy CPU; needs SIMD optimization. |
| Persistent Homology | defer | Research-grade; depends on History Buffer. |

## Curves

Combining all modes:

| Idx | Label | Used by |
|---|---|---|
| 0 | AMOUNT | All modes |
| 1 | MODE / CAPACITY | Chladni (mode index), Helmholtz (per-trap capacity), Wavefield (boundary) |
| 2 | DAMPING / RELEASE | Chladni (damping), Helmholtz (release), Wavefield (damping) |
| 3 | THRESHOLD / WAVE_SPEED | Helmholtz (overflow), Wavefield (c), Persistent Homology (persistence threshold) |
| 4 | TIME | Persistent Homology (window) |
| 5 | MIX | All modes |

6 curves. `num_curves() = 6`.

## Architecture fit

### SpectralModule slot, with own 2-D state

Geometry needs from global infra:

- For Persistent Homology mode only: HistoryBuffer infrastructure
  (covered in `01-global-infrastructure.md` § History Buffer).
- For the rest: nothing global.

Does NOT need:

- BinPhysics (Geometry has its own 2-D state, not BinPhysics fields).
- IF / chromagram / cepstrum.
- MIDI.
- BPM sync.

Per-channel state:

- Hilbert LUT (32 KB shared).
- Plate phase (33 KB per channel for Chladni).
- Trap fill levels (small, ~1 KB).
- Wavefield grids (64 KB per channel — only when Wavefield active).

Total memory budget: ~130 KB per channel per Geometry slot. Cheap.

### Why the user-facing name "Geometry" matters

Same naming logic as Past/Future/Life/Kinetics: "Geometry" is a
concept users understand. Alternatives: "Topology" (too math-y),
"Plate" (too narrow), "Surface" (vague). "Geometry" lands cleanly.

## CPU class

Per-mode:

| Mode | CPU |
|---|---|
| Chladni Plate | light |
| Helmholtz Traps | light |
| Wavefield | moderate-heavy (mark `heavy_cpu`) |
| Persistent Homology | heavy (mark `heavy_cpu`, `always_bypassed_on_low_end`) |

Module-level `heavy_cpu = false` for v1 (only Chladni + Helmholtz
ship). When Wavefield/Persistent Homology arrive, per-mode flag
becomes essential — see `02-architectural-refactors.md` § Per-mode
heavy_cpu.

## BinPhysics interactions

Reads: nothing.
Writes: nothing.

Geometry runs entirely in its own 2-D substrate state. It does not
write into BinPhysics fields, since those are 1-D and the geometry
math doesn't map cleanly back without losing the 2-D structure.

## Calibration probe set

- `probe_amount_pct`
- `probe_active_mode_idx`
- `probe_mode_index` (Chladni: which (m, n) eigenmode)
- `probe_active_trap_count` (Helmholtz: # of traps above the
  activation threshold)
- `probe_max_fill_pct` (Helmholtz: highest fill among active traps)
- `probe_persistence_features` (Persistent Homology: # of features
  in the cached diagram)

## RESEARCH PROMPT — Real-time 2-D wave equation on Hilbert-mapped spectrum

```
Topic: Real-time discrete 2-D wave equation simulation on a
locality-preserving 1-D-to-2-D mapping (Hilbert curve), with bin
indexing and SIMD-friendly memory access.

Context: We have a 1-D spectrum of N bins (N up to 8193). We want to
map it to a 2-D grid (M × M ≈ 91 × 91 for N=8193) via a Hilbert
curve so bin neighbours are spatial neighbours. Each hop, run one
finite-difference 2-D wave-equation step:
  u(x,y, t+dt) = 2u - u_prev + c² dt² · (Δ_x² + Δ_y²) u
where the Laplacian uses the 4-connected stencil. Then project the
2-D grid back to 1-D via the same Hilbert mapping for iFFT.

Specific questions:
1. Does the Hilbert curve's locality preservation hold up under the
   wave equation, or do non-Hilbert-neighbour bins still "feel" each
   other strongly enough that the mapping is irrelevant?
2. CFL stability: c × dt / dx < 1/√2 in 2-D. What c value gives
   audibly interesting ringing without violating CFL? At what hop
   rate (dt = hop / sample_rate) is c constrained?
3. Boundary conditions: absorbing (Mur first-order), reflective
   (Neumann), periodic (toroidal). Which gives the most musically
   useful behaviour for the spectrum-as-substrate metaphor?
4. SIMD: a 91×91 wave step has annoying boundary handling. Pad to
   96×96 and ignore boundary rows? Use AVX-512 to do 16 cells per
   instruction?
5. Per-bin boundary curve: the user draws a boundary-reflectivity
   curve over the 1-D spectrum. Mapping that to 2-D via Hilbert
   should give a meaningful spatial boundary, but does it "feel"
   meaningful to a producer turning a knob?

Deliverable: Rust kernel + audio examples on a sustained sine, a
chord, and a drum loop. Compare boundary modes audibly.
```

## RESEARCH PROMPT — Persistent homology of STFT magnitude for source isolation

```
Topic: Real-time persistent homology on the 2-D (time × frequency)
STFT magnitude landscape, used to isolate persistent spectral
features.

Context: We hold a rolling history buffer of N STFT frames (32 to
256, settable). We want to run persistent-homology analysis on the
2-D grid every K hops, identify the M most persistent maxima, and
use that subset to gate or weight the current hop's bins.

Specific questions:
1. Sub-level vs super-level filtration: which captures
   musically-relevant features? Maxima are super-level; saddles and
   valleys are sub-level.
2. Streaming computation: most persistent-homology libraries are
   batch. What's the streaming algorithm cost when the analysis
   window slides by one frame each hop? Can we incrementally update
   the persistence diagram?
3. Persistence threshold mapping: the user draws a curve setting the
   threshold per frequency. Does this map sensibly to "show me peaks
   that survive at least X dB of magnitude variation"?
4. Real-time CPU: at N=64 frames × 8193 bins, is even the
   batch-recompute cost feasible at 4-hop rate?
5. Output mapping: once we have the persistent maxima list, how do
   we translate to bin gates? Rectangular gate around each maximum,
   or smooth (Gaussian) per maximum, or something else?

Deliverable: a reference implementation comparison (one batch
algorithm baseline, one streaming attempt) with CPU profiles and
audio examples on a sustained chord with noise added.
```

## Open questions

1. **Module name.** "Geometry" feels right per the naming logic, but
   a producer might expect "Geometry" to mean panning / spatial
   audio. Alternatives: "Surface," "Plate," "Topology," "Field."
   "Geometry" stays unless someone has a better one.
2. **2-D mapping choice.** Hilbert vs row-major vs Z-order. Hilbert
   is most defensible but slower to compute (precompute LUT once).
3. **v1 mode count.** Ship 2 (Chladni + Helmholtz) or wait for
   Wavefield to be SIMD-tuned and ship 3?
4. **Persistent Homology depends on History Buffer.** Defer until
   `13-past.md` work lands?
5. **Helmholtz overflow envelope shape.** Linear release, exp release,
   or exp release with a small initial "burst"? The burst feels more
   physical (cavity bursting) but is more aggressive.
6. **Do we need a way for Geometry's modes to share 2-D state across
   slots?** E.g. two Geometry-Wavefield slots both injecting energy
   into the same 2-D substrate? Probably no — keep modules
   independent.

## Research findings (2026-04-26)

Wavefield (2D wave) and Persistent Homology are covered respectively by
`research/03-physical-models.md` (Topic C) and
`research/06-specialized-topics.md` (Topic B). Validated decisions:

1. **2D wave grid = 128×64** padded to 8192 cells (one virtual cell at
   end). AVX-256 inner loop (8 floats per vector). Total 256 SIMD ops
   per hop per channel — trivially real-time.
2. **Standard 4-neighbour Laplacian stencil for v1.** Upgrade to
   8/9-neighbour only if "uneven cross-talk" from the Hilbert long-tail
   is audible.
3. **CFL ceiling: `c ≤ 0.65`** in normalized units (92% of the strict
   `1/√2` limit). Gives audible high-frequency dispersion without
   crashing.
4. **Hilbert LUT precomputed at reset** (16 KB read-only). Used both
   for energy injection (1D bin → 2D grid cell) and energy extraction
   (grid cell → 1D bin). Same LUT, both directions.
5. **Boundary curve = mixed Neumann / Mur first-order ABC.** A `[0,1]`
   per-bin curve where 0 = Mur (absorbent) and 1 = Neumann (reflective)
   gives clean control over "where the spectrum rings vs where it
   dies." Toroidal/periodic is a separate **discrete** boundary mode
   (not a curve value — periodic + Mur don't mix cleanly).
6. **DEAD ENDS:**
   - PML boundary — 4-8 extra cells of memory + CPU for absorption
     quality we don't need.
   - 8-neighbour stencil for v1 — premature; ship 4-neighbour first.
7. **Persistent Homology mode is infeasible at native FFT resolution.**
   Cubical Ripser on our 8193×64 grid is ~80-150 ms per analysis on a
   modern core (>10× too slow for hop rate, ~54% of one core even at
   1/16 cadence). Two paths forward:
   - **v1 ship: 1D-only peak persistence per frame** (sthu-style,
     O(n log n), ~200 µs per hop in Rust on 8193 bins). Persistent
     peaks are *more stable* across hops than threshold-based peaks —
     could replace Harmony's IF/MQ peak picker.
   - **v2 ship: 2D PH on a downsampled grid** (1024 bins × 16 frames)
     via a worker thread + lock-free triple-buffer for the persistence
     diagram. Mark `always_bypassed_on_low_end = true`. Output mapping
     = Gaussian gate around each persistent maximum (sigma ∝
     persistence value); avoid rectangular gates (audible bin
     boundaries).
8. **Skip streaming PH algorithms.** O(m²) update per insertion beats
   batch only when m is tiny — for our window sizes, batch recompute
   every K hops is faster.
9. **Patent-safe.** Verlet, leapfrog, FTCS, FDTD, Mur ABC, Hilbert
   curves are all 1960s-1990s textbook methods. No oeksound overlap.
