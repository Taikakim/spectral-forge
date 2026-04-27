# Phase 7 — Heavy / Research-Grade (Umbrella)

> **STATUS:** PLANNED, RESEARCH-BLOCKED. Authoritative status: `docs/superpowers/STATUS.md`.

> **For agentic workers:** This is an **index document, not an implementable plan.**
> Each sub-phase below is gated on a research deliverable (research prompts in
> `ideas/next-gen-modules/90-research-prompts.md`). **Do not begin TDD on any
> sub-phase until that prompt's deliverable lands and a sub-plan is written.**

**Goal:** ship the genuinely expensive features that may need GPU compute or are
research-grade — Geometry Wavefield, Geometry Persistent Homology, Harmony FM Replicator,
plus the optional GPU compute path. Per
`ideas/next-gen-modules/99-implementation-roadmap.md` § Phase 7.

**Scope decision:** Phase 7 covers four independent subsystems, each with its own research
blocker. Per `superpowers:writing-plans` "Scope Check," this phase is split into **four
sub-plans** plus this umbrella. None of the sub-plans exist yet — they're written *after*
the research lands.

**Tech stack (anticipated):** Rust, `realfft`, `nih-plug`, possibly `wgpu` for the GPU
path, possibly a `gudhi`-style topology library port for Persistent Homology, `BinPhysics`
infrastructure from Phase 3, IF infrastructure from Phase 6.1.

---

## Cross-phase prerequisites

Phase 7 sub-plans assume the following has already merged:

| Prereq | Why | Provided by |
|--------|-----|-------------|
| Phase 1 (foundation) | `ModuleContext` lifetime, ModuleSpec needs_* fields, ring scaffolding | Phase 1 |
| Phase 3 (BinPhysics) | All sub-phases read `mass`, `velocity`, `crystallization` | Phase 3 |
| Phase 5b.1 (HistoryBuffer) | Persistent Homology reads time × frequency 2-D windows | Phase 5b.1 |
| Phase 6.1 (IF) | FM Replicator reads stable partial frequencies | Phase 6.1 |
| Phase 5a (Life) — Geometry-light shipped | Validates the SpectralModule shape for 2-D state modules | Phase 2e (Geometry-light) |

If any of the above is missing when Phase 7 work begins, **stop** and complete the
prerequisite first. Phase 7 sub-plans should never inline foundation infrastructure.

---

## Sub-plans

| # | Sub-phase | Research blocker | Status | Sub-plan file (when written) |
|---|-----------|------------------|--------|------------------------------|
| 7.1 | Geometry Wavefield | `90-research-prompts.md` § 12 (real-time 2-D wave equation on Hilbert-mapped spectrum) | RESEARCH-BLOCKED | `2026-XX-XX-phase-7.1-geometry-wavefield.md` |
| 7.2 | Geometry Persistent Homology | `90-research-prompts.md` § 13 (real-time persistent homology on 2-D STFT) | RESEARCH-BLOCKED | `2026-XX-XX-phase-7.2-geometry-persistent-homology.md` |
| 7.3 | Harmony FM Replicator | none formally — see 15-harmony.md § c | DESIGN-INCOMPLETE | `2026-XX-XX-phase-7.3-harmony-fm-replicator.md` |
| 7.4 | GPU compute path (`wgpu`) | none — pure engineering, but conditional on the CPU paths shipping first | CONDITIONAL | `2026-XX-XX-phase-7.4-gpu-compute-path.md` |

### Dependency order

```
Phase 1 / 3 / 5a / 5b.1 / 6.1 already merged
        │
        ├──── 7.1 Wavefield  (needs Prompt 12 deliverable)
        ├──── 7.2 Persistent Homology  (needs Prompt 13 deliverable)
        ├──── 7.3 FM Replicator  (CPU-only first; design needs partial-tracking decision)
        │
        └──── 7.4 GPU compute path
                  (only after at least one of 7.1/7.2/7.3 ships on CPU and
                   profiling shows the CPU cost is unacceptable on target hardware)
```

7.1, 7.2, and 7.3 are mutually independent and can be developed in parallel by separate
streams if research lands for multiple at once. 7.4 is strictly downstream of "we know
which kernel is too slow on CPU."

---

## Sub-phase scope summaries

### 7.1 — Geometry Wavefield

Per `ideas/next-gen-modules/18-geometry.md` § c. A 2-D discrete wave-equation simulator
running on a Hilbert-mapped spectrum: bins are arranged on a 2-D grid (Hilbert curve to
preserve frequency locality), each grid cell injects energy from its bin's magnitude, and
a finite-difference wave step propagates that energy across the grid. The grid's wave
state then *modulates* the spectrum back: cells with high wave amplitude boost their
corresponding bin.

**Cost class:** moderate-heavy. With 8193 bins on a ~91×91 grid, a 2-D wave step is ~25 k
multiply-adds per hop per channel. SIMD viable, GPU advantageous.

**Open research questions** (Prompt 12): Hilbert vs Z-order locality preservation; ghost-cell
boundary conditions; SIMD-tuned 2-D stencils on AVX-512; appropriate `c` (wave-speed)
range and stability bounds.

**Ship gating decision:** mark `heavy_cpu = true` per-mode; ship if SIMD path keeps it
under the OLA budget at MAX_FFT_SIZE = 16384, else defer to Phase 7.4 (GPU).

### 7.2 — Geometry Persistent Homology

Per `ideas/next-gen-modules/18-geometry.md` § d. Persistent-homology analysis on the 2-D
(time × frequency) STFT magnitude surface. Identifies "topological features" (peaks, ridges,
valleys) with persistence values, then uses those features to drive a reconstruction:
high-persistence features survive, low-persistence features get attenuated. Effectively a
denoiser that respects topological structure rather than amplitude.

**Cost class:** heavy. Most persistent-homology libraries (gudhi, ripser, Dionysus) are
batch-only — streaming variants are research-grade.

**Open research questions** (Prompt 13): streaming PH algorithms for sliding STFT windows;
appropriate persistence threshold range; sub-FFT-resolution feature reconstruction;
windowed update vs full recompute every N hops.

**Ship gating:** mark `heavy_cpu = true` AND `always_bypassed_on_low_end = true`. May
require GPU compute (Phase 7.4) to be playable in real time at all.

### 7.3 — Harmony FM Replicator

Per `ideas/next-gen-modules/15-harmony.md` § c. A 16-operator-DX-style re-spectraliser:
take the input spectrum, find up to 16 stable partials via IF, treat them as carriers,
then frequency-modulate each by another via per-operator algorithm matrices (DX7 algorithm
1-32). Outputs a spectrum that's the FM-synthesis approximation of the input.

**Design-incomplete questions:**
1. Algorithm matrix selection — UI for picking among DX7's 32 algorithms, or auto-pick
   based on detected partial relationships?
2. Modulation index range — DX7 uses 0-99; map to which range of FFT-bin sideband counts?
3. Output blending — pure FM-resynthesis (zero original spectrum) vs additive (original +
   FM)?
4. FM Replicator vs FM Network (Modulate's mode 6, Phase 6.6) — both use IF + partial pairs.
   Confirm scope boundary so the two don't duplicate. Lean: FM Network is a *modulation*
   effect (input audible + FM character mixed in), FM Replicator is a *re-synthesis* effect
   (input replaced by FM approximation).

**Ship gating:** CPU-first. May need GPU for the 16×16 algorithm matrix per hop if profile
shows it's too slow.

### 7.4 — GPU compute path (`wgpu`)

Per `ideas/next-gen-modules/01-global-infrastructure.md` § GPU/SIMD discussion. A
`cargo feature = "wgpu-compute"` that lets opted-in modules (Wavefield, Persistent
Homology, FM Replicator, Past with decay-sort) dispatch their per-bin loop to a compute
shader.

**Why this is its own sub-phase:** introducing `wgpu` to the build adds a 1-2 second
compile cost, a runtime dependency on a working GPU driver, and platform-specific debug
overhead. It's only worth doing when the CPU paths are demonstrably insufficient.

**Ship gating:** **conditional**. Only start once a Phase 7 module has shipped on CPU and
profiling shows it can't keep up at the target FFT size on target hardware (Bitwig on a
modern desktop CPU). Apple Silicon's unified-memory advantage (per the spec discussion)
means Mac collaborators benefit "for free" if/when this exists.

**Tar-pit warning:** per `99-implementation-roadmap.md` § Risks #5, this can easily eat
6 months. Don't start unless Phase 7's CPU path is *demonstrably* insufficient.

---

## Ship cadence

Each sub-plan ships as its own minor release after its prerequisite research is complete:

- 7.1 Wavefield: ship as `0.X.0` "Wavefield" release once Prompt 12 deliverable + CPU
  benchmark land.
- 7.2 Persistent Homology: ship as `0.X.0` "Topology" release once Prompt 13 deliverable
  + streaming PH library port land.
- 7.3 FM Replicator: ship as `0.X.0` "FM Replicator" release once 7.3 design questions
  resolve.
- 7.4 GPU path: ship as `0.X.0` "GPU" release; gated as above.

There's no expected ordering between 7.1, 7.2, and 7.3 — whichever finishes its research
first ships first.

---

## When to write the sub-plans

**Do not write a sub-plan until:**
1. The research prompt is closed (a research deliverable exists in
   `ideas/next-gen-modules/research/`), OR
2. The DESIGN-INCOMPLETE questions for that sub-phase are resolved (logged as decisions
   in the source spec or in CHANGELOG/notes).

When ready, the sub-plan author should follow `superpowers:writing-plans` exactly (TDD
2-5 minute steps with complete code, no placeholders) and save to
`docs/superpowers/plans/2026-XX-XX-phase-7.X-<feature-name>.md`.

The sub-plan should:
1. State the research deliverable it consumes (path + version).
2. Map each algorithm step from the deliverable into a concrete TDD task.
3. Add the calibration probe set per `ideas/next-gen-modules/18-geometry.md` § Calibration
   probe set (or the equivalent for FM Replicator / GPU path).
4. Update `docs/superpowers/STATUS.md` from `RESEARCH-BLOCKED` to `IN PROGRESS` when work
   starts and `IMPLEMENTED` when it ships.

---

## Out of scope

These items are *not* part of Phase 7, even though they may seem heavy/research-flavoured:

- **Spectral Delay** — parking lot per the roadmap. Not in any phase.
- **Polyphonic compander tied to MIDI tracking** — parking lot.
- **Lookahead Duck** — Future v2.
- **Self-Punch** — deferred per `19-punch.md`.
- **Sidechain history** — deferred per `13-past.md`.
- **Multi-mode-per-slot** — v2 across the board.
- **Cepstral X-axis display** — UI work not blocking any module ship; can land alongside
  Phase 6.5 Harmony Lifter as a polish pass.

---

## Self-review (for the umbrella itself)

- [x] Each item from `99-implementation-roadmap.md` § Phase 7 is enumerated as a sub-phase.
- [x] Each sub-phase's research blocker is named with a citation to the prompt.
- [x] No sub-plan content is written here — the umbrella's job is solely to gate work
      and direct the sub-plan author to the research.
- [x] Cross-phase prerequisites are listed so a future writer can't miss them.
- [x] The "tar pit" risk for the GPU path is called out in the gating discussion.

---

## Cross-references

- `ideas/next-gen-modules/99-implementation-roadmap.md` § Phase 7 — original roadmap entry.
- `ideas/next-gen-modules/18-geometry.md` § c, d — Wavefield + Persistent Homology specs.
- `ideas/next-gen-modules/15-harmony.md` § c — FM Replicator spec.
- `ideas/next-gen-modules/01-global-infrastructure.md` § GPU/SIMD — GPU path discussion.
- `ideas/next-gen-modules/90-research-prompts.md` § 12, § 13 — research prompts.
- `ideas/next-gen-modules/91-research-synthesis.md` — research findings index.
- `docs/superpowers/STATUS.md` — current implementation status.
- `docs/superpowers/plans/2026-04-27-phase-6-pitch-harmony-umbrella.md` — Phase 6
  precedent for umbrella + sub-plans split.
