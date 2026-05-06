# Next-Gen Modules — Research Notes

**Created:** 2026-04-26
**Status:** RESEARCH — not approved, not specced. Reading material for review.

## What this folder is

An audit of the eight DEFERRED design specs under
`docs/superpowers/specs/2026-04-21-*.md` against the source brainstorms in
`docs/future-ideas/`, plus implementation-order thinking and research
prompts for follow-up work in heavier AI sessions.

For each existing DEFERRED spec we:

1. **Cross-reference every brainstorm idea** against the spec to find what
   was glossed over or silently dropped (with Kim's own annotations as the
   tie-breaker).
2. **Add the missing sub-effects / clarifications** as proposals.
3. **Surface the technical questions** that need real research — embedded
   inline as "RESEARCH PROMPT" blocks that can be pasted directly into a
   frontier-AI session.
4. **Note where the spec's framing should change** in light of how the
   shipped architecture actually evolved (BinPhysics is still deferred,
   but Freeze, PhaseSmear, Dynamics, etc. all shipped — some specs
   reference patterns that no longer match the code).

It is **not** a plan. It is the layer that sits between brainstorm and
spec. Kim reviews these, runs whatever research is wanted in heavier AI
sessions, then we converge in a brainstorming + planning session to
produce the real design specs and implementation plans.

## How these notes relate to the existing DEFERRED specs

Eight specs were drafted on 2026-04-21 and are listed as DEFERRED in
`docs/superpowers/STATUS.md`. They already capture the bulk of the
brainstormed ideas. These research notes do four things on top of those:

1. **Pull in ideas that are NOT yet captured** in any spec (e.g. Cepstral
   Liftering, Helmholtz Traps, Chladni Plate Nodes, Punch, Tape Print-Through
   refinements, the Sandpaper Phase / Tuning Fork Intermod ideas).
2. **Propose two NEW modules** that do not exist in the spec set:
   - `Future` — splits the *write-ahead* / pre-echo / prediction half of
     "Past" out into its own module so the read-only history-buffer module
     stays clean and the two can be developed independently.
   - `Geometry` — a home for ideas that aren't 1-D (Chladni plate nodes,
     persistent homology of the history buffer, anything that needs a 2-D
     mapping over the spectrum).
3. **Reconsider where each idea lives** when the brainstorm hint and the
   spec disagree (e.g. Kim's annotation "this could be just a part of a
   well-thought-out spectral delay" pulls Bin-Specific Swing partly out of
   Rhythm).
4. **Surface global-infra and refactor questions** that touch every module
   so they get answered once, not eight times.

If a section here disagrees with the older spec, **the spec wins** until
this folder gets converted into new specs. Treat these notes as proposals.

## Files

| File | What it covers |
|---|---|
| `00-README.md` | This document. |
| `01-global-infrastructure.md` | BinPhysics, History Buffer, Instantaneous Frequency, host-BPM sync, MIDI input, pitch/chord detection, Modulation Ring UI, harmony probability matrix, GPU/SIMD discussion, the "Kinetics as global function vs. module" question. |
| `02-architectural-refactors.md` | Trait/struct extensions, slot-count and curve-count growth, MIDI plumbing, scratch-buffer policy, "always-bypassed" heavy modules, the Y-axis quantize feature, ModuleContext additions, pre-allocation rules, calibration impact. |
| `03-matrix-amp-nodes.md` | Audit of the existing Matrix Amp Nodes spec; ideas for additional amp modes; UI/state implications. |
| `10-circuit.md` | Audit of the Circuit module spec — gaps (Crossover Distortion, Asymmetric Bias Fuzz, Resonant Feedback, Transformer-spread), refinements, research prompts. |
| `11-life.md` | Audit of the Life module spec — gaps (Capillary Action, Yield Strength & Tearing, Sandpaper Phase, Brownian, Helmholtz Traps, Crystallization scope question vs Freeze). |
| `12-kinetics.md` | Audit of the Kinetics module spec — gaps (Diamagnetic Repulsion, Tuning-Fork Intermod, MIDI gravity wells), refinements. |
| `13-past.md` | Audit of the Past module spec — gaps and additional history-based sub-effects, decay-sorter variants. |
| `14-future.md` | NEW module proposal — pulls Tape Print-Through and other write-ahead ideas out of Past into a sibling module. |
| `15-harmony.md` | Audit of the Harmony module spec — large gap list (Stiffness/Bessel/Prime inharmonicity, FM Replicator, Harmonic Series Generator, Cepstral Liftering, sustained-harmonic-series detection, transient inclusion). Includes the biggest research prompt block. |
| `16-modulate.md` | Audit of the Modulate module spec — gaps (Diode-Bridge Ring Mod, Ground Loop, PLPV phase locking integration). |
| `17-rhythm.md` | Audit of the Rhythm module spec — largely complete; question on Bin Swing being subsumed by a future Spectral Delay; stochastic hi-hats relocation. |
| `18-geometry.md` | NEW module proposal — Chladni Plate Nodes, persistent-homology informed reconstruction, Helmholtz traps, anything needing a 2-D embedding. |
| `19-punch.md` | NEW module / sidechain-mode proposal — the "Punch" idea (#21) where sidechain punches holes that neighbouring bins try to fill. Discusses module vs. sidechain-mode framing. |
| `20-plpv-phase-cross-cutting.md` | Cross-cutting PLPV (Peak-Locked Phase Vocoder, formerly "PVX") phase-unwrapping / phase-locking technique that affects shipped modules (Dynamics ducking, PhaseSmear, Freeze) more than any single new module. |
| `90-research-prompts.md` | Consolidated copy-pasteable research prompts for frontier-AI sessions, grouped by topic. Mirrors the inline RESEARCH PROMPT blocks from the per-module files. |
| `91-research-synthesis.md` | Cross-cutting synthesis of the six `research/` reports — shared infrastructure, validated paths, dead ends, and proposed roadmap deltas. Read this *before* `99-implementation-roadmap.md`. |
| `99-implementation-roadmap.md` | Proposed phasing for the work — what to build first, what depends on what, where the natural release-cadence cuts are, "always-bypass-on-low-end-hardware" tier, suggested PR sequencing. |
| `research/` | Outputs from the parallel research-agent sweep dispatched 2026-04-26. Six files (`01-pvx-phase-and-pll.md` through `06-specialized-topics.md`), each consolidating 1–4 prompts from `90-research-prompts.md` into a single arxiv/GitHub/literature digest. See `91-research-synthesis.md` for the cross-cutting summary, and the "Research outputs" section at the bottom of `90-research-prompts.md` for the prompt-to-file mapping. |

## Naming convention used in this folder

- **Lowercase, hyphenated** filenames matching the existing
  `docs/superpowers/specs/` convention.
- A two-digit prefix to enforce a reading order: 00 = meta, 01–03 = infra,
  10+ = one number per module. Numeric gaps are intentional so a future
  module can slot in (e.g. `12.5-springs.md` if Kinetics needs to split).
- Module file names match the *user-facing module name* the user proposed
  in the ideas file ("Past", "Future", "Life", "Kinetics", etc.) rather
  than literal sub-effect names — Kim's note was that these category names
  are good because they "push the users further from the 'oh a delay mmh'-
  reaction."

## How to read each module file

Each per-module file follows the same structure:

1. **Cover** — name, category, dependency on global infra, status in the
   existing DEFERRED spec set (if any).
2. **What it is** — one paragraph identity for the module.
3. **Sub-effects** — the AmpMode-style mode list, with each sub-effect
   tagged by its origin (existing spec / new from ideas file / refinement).
4. **Curves** — proposed curve set and labels.
5. **Architecture fit** — concretely how it slots into the current
   `SpectralModule` + `FxMatrix` + `RouteMatrix` model. Reads/writes to
   `BinPhysics` are listed.
6. **Open questions** — unresolved choices, with enough context that
   answering them does not require re-reading the source.

## Things that are NOT modules

A few brainstorm items are not module-shaped and live in
`01-global-infrastructure.md` or `02-architectural-refactors.md`:

- **Kinetics-as-a-global-property** — Kim's note about a single global
  "kinetics" function for portamento / mass / hysteresis. Discussed as a
  potential refactor of how `BinPhysics.velocity` and `mass` are exposed.
- **Modulation Ring UI** — global S/H + Sync + Legato overlay for any
  knob/curve.
- **Y-axis quantization** of curves.
- **GPU / AVX-512 / wgpu** path for heavy modules.
- **Reset-to-default** button.

## Status of this folder

Nothing here has been promoted to a spec. Nothing here is approved. Nothing
here has been argued against the audio engine. All numbers ("≈ 2 ms",
"≈ 56 MB") are sanity checks copied or re-derived from the source brainstorm
and have not been benchmarked.

The expected lifecycle:

```
brainstorm (docs/future-ideas/) → research notes (this folder) →
design spec (docs/superpowers/specs/) → plan (docs/superpowers/plans/) →
code (src/)
```

This folder is the second box.
