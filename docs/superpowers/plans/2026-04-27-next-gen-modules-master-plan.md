# Next-Gen Modules — Master Plan Index

> **STATUS:** PLANNED, NOT STARTED. Authoritative status: `docs/superpowers/STATUS.md`.

> **For agentic workers:** This is a **navigation index**, not an implementable
> plan. Each numbered sub-plan listed below is a self-contained TDD plan; pick
> one whose dependencies are met and follow either
> `superpowers:subagent-driven-development` (recommended) or
> `superpowers:executing-plans`. **Always check `STATUS.md` before starting** —
> a plan's banner can drift; the STATUS index wins.

**Purpose:** This document is the single entry point for all next-gen-module
implementation work derived from `ideas/next-gen-modules/`. It lists every
phase, every sub-plan, the dependency graph between them, the ship cadence,
and the parking-lot of explicitly-deferred items. It does **not** contain TDD
tasks of its own — every concrete task lives in a phase or sub-plan file.

---

## Source documents

| Doc | Role |
|-----|------|
| `ideas/next-gen-modules/00-toc.md` | Table of contents over the next-gen design corpus. |
| `ideas/next-gen-modules/99-implementation-roadmap.md` | Phased roadmap (Phases 0–7 + risks + parking lot). The phasing in this index mirrors that file 1:1. |
| `ideas/next-gen-modules/91-research-synthesis.md` | Index of closed/open research deliverables. |
| `ideas/next-gen-modules/90-research-prompts.md` | The research prompts gating Phase 7. |
| `docs/superpowers/STATUS.md` | Current implementation status of every plan/spec. |

---

## Phase summary

| Phase | Title | Sub-plan count | Status (umbrella) | Headline ship |
|-------|-------|----------------|-------------------|---------------|
| 1 | Foundation infra | 1 | PLANNED | Lifetime'd `ModuleContext` + Option fields scaffolded; `needs_*` ModuleSpec fields; ring widget inert. |
| 2 | Quick-win modules | 7 (a–g) | PLANNED | Ships 7 new module types in their light-mode form: Matrix Amp Nodes, Future, Punch, Rhythm, Geometry-light, Modulate-light, Circuit-light. |
| 3 | BinPhysics | 1 | PLANNED | `BinPhysics` struct (mass / velocity / crystallization / phase_momentum) + read/write API. |
| 4 | PLPV phase | 1 | PLANNED | Per-bin unwrapped phase + peak detection populated on `ModuleContext`. |
| 5a | Life | 1 | PLANNED | Conway-on-spectrum module shipping the SpectralModule shape used later by Geometry. |
| 5b | History buffer + Past + Kinetics + Modulate retrofit | 1 umbrella + 4 sub-plans | PLANNED | Ring-buffer history infra + Past + Kinetics + Modulate's full mode set. |
| 5c | Full Circuit | 1 | PLANNED | Circuit module's heavy modes wired on top of BinPhysics. |
| 6 | Pitch / Harmony | 1 umbrella + 7 sub-plans | PLANNED | IF / chromagram / cepstrum / MIDI infra → Harmony module → ring UI activation + FM Network + Arp NoteIn retrofits. |
| 7 | Heavy / research | 1 umbrella + 4 sub-plans (none yet written) | RESEARCH-BLOCKED | Wavefield + Persistent Homology + FM Replicator + GPU compute path. |

---

## All sub-plans (in suggested execution order)

> **Reading the dependency column:** each row lists what must be merged
> before this sub-plan starts. "Phase 1" means the entire Phase 1 plan; a
> sub-plan number (e.g. "6.1") means specifically that sub-plan.

### Phase 1 — Foundation infrastructure

| # | Plan file | Ships | Depends on |
|---|-----------|-------|------------|
| 1 | `2026-04-27-phase-1-foundation-infra.md` | Lifetime'd `ModuleContext<'block>` with `Option<&'block T>` fields for every infra payload Phase 3+ will populate; `ModuleSpec.needs_*` declarations; `mod_ring_states` editor state + inert ring widget. | none — landing point for everything else. |

### Phase 2 — Quick-win modules

7 module types ship in their light-mode (CPU-cheap) form. All depend on Phase 1
for the `needs_*` infra-gating fields, but otherwise are mutually independent.

| # | Plan file | Ships | Depends on |
|---|-----------|-------|------------|
| 2a | `2026-04-27-phase-2a-matrix-amp-nodes.md` | Matrix Amp Nodes module + UI. | Phase 1 |
| 2b | `2026-04-27-phase-2b-future.md` | Future module (lookahead-style fades). | Phase 1 |
| 2c | `2026-04-27-phase-2c-punch.md` | Punch module (transient emphasis, non-self-punch). | Phase 1 |
| 2d | `2026-04-27-phase-2d-rhythm.md` | Rhythm module: Euclidean + Arpeggiator (BPM-trigger only) + PhaseReset. | Phase 1, host BPM (already in nih-plug) |
| 2e | `2026-04-27-phase-2e-geometry-light.md` | Geometry-light: Voronoi / Lissajous / Polygon modes. Wavefield + PH deferred to Phase 7. | Phase 1 |
| 2f | `2026-04-27-phase-2f-modulate-light.md` | Modulate-light: PhasePhaser, BinSwapper, RmFmMatrix, DiodeRm, GroundLoop. FM Network deferred to Phase 6.6. | Phase 1 |
| 2g | `2026-04-27-phase-2g-circuit-light.md` | Circuit-light: light-mode Circuit operators. Heavier modes ship in Phase 5c. | Phase 1 |

### Phase 3 — BinPhysics infrastructure

| # | Plan file | Ships | Depends on |
|---|-----------|-------|------------|
| 3 | `2026-04-27-phase-3-bin-physics.md` | `BinPhysics` struct (per-bin mass / velocity / crystallization / phase_momentum), read/write API on `ModuleContext`, integration into Pipeline. No DSP wired yet — consumers in Phase 5/6/7. | Phase 1 |

### Phase 4 — PLPV phase

| # | Plan file | Ships | Depends on |
|---|-----------|-------|------------|
| 4 | `2026-04-27-phase-4-plpv-phase.md` | Per-bin unwrapped phase + peak detection populated on `ctx.unwrapped_phase` and `ctx.peaks`. Required by Phase 6.1 (IF). | Phase 1 |

### Phase 5 — Heavy physics + history

#### 5a Life

| # | Plan file | Ships | Depends on |
|---|-----------|-------|------------|
| 5a | `2026-04-27-phase-5a-life.md` | Life module: 2-D Conway-style state machine on the spectrum. First module to validate the SpectralModule shape Geometry will reuse in Phase 7. | Phase 1, Phase 3 |

#### 5b History-buffer + dependent retrofits

| # | Plan file | Ships | Depends on |
|---|-----------|-------|------------|
| 5b umb | (umbrella) | Index for 5b.1–5b.4. | — |
| 5b.1 | `2026-04-27-phase-5b1-history-buffer.md` | `HistoryBuffer` ring of recent STFT magnitudes; gated by `needs_history`. | Phase 1 |
| 5b.2 | `2026-04-27-phase-5b2-past.md` | Past module: time-reversed / decay-sorted spectral playback. | 5b.1, Phase 3 |
| 5b.3 | `2026-04-27-phase-5b3-kinetics.md` | Kinetics module: history-derived motion analysis. | 5b.1, Phase 3 |
| 5b.4 | `2026-04-27-phase-5b4-modulate-retrofit.md` | Wires Modulate's history-aware modes left blank in Phase 2f. | 5b.1, Phase 2f |

#### 5c Full Circuit

| # | Plan file | Ships | Depends on |
|---|-----------|-------|------------|
| 5c | `2026-04-27-phase-5c-full-circuit.md` | Heavy Circuit operators not shipped in Phase 2g. | Phase 2g, Phase 3 |

### Phase 6 — Pitch / Harmony

| # | Plan file | Ships | Depends on |
|---|-----------|-------|------------|
| 6 umb | `2026-04-27-phase-6-pitch-harmony-umbrella.md` | Phase 6 index. | — |
| 6.1 | `2026-04-27-phase-6.1-instantaneous-freq.md` | `compute_instantaneous_freq` helper; populates `ctx.instantaneous_freq`. | Phase 1, Phase 4 |
| 6.2 | `2026-04-27-phase-6.2-chromagram-harmonic-groups.md` | `compute_chromagram` + `harmonic_group_detect`; populates `ctx.chromagram` and `ctx.harmonic_groups`. | 6.1 |
| 6.3 | `2026-04-27-phase-6.3-midi-input.md` | MIDI input port + `held_notes`/`pitch_classes`/`note_events` in SharedState; populates `ctx.midi_notes` / `ctx.held_pitch_classes`. | Phase 1 |
| 6.4 | `2026-04-27-phase-6.4-cepstrum.md` | Lazy cepstrum (extra real-FFT pair) gated by `needs_cepstrum`; exposed via `ctx.cepstrum_buf`. | Phase 1 |
| 6.5 | `2026-04-27-phase-6.5-harmony-module.md` | `HarmonyModule` with 8 v1 sub-effects: Chordification, Undertone, Companding, Formant Rotation, Lifter, Inharmonic (Stiffness/Bessel/Prime), Harmonic Generator, Shuffler. FM Replicator + Persistent Homology deferred to Phase 7. | 6.1, 6.2, 6.3, 6.4, Phase 3 |
| 6.6 | `2026-04-27-phase-6.6-fm-network-arpeggiator-noteIn.md` | Modulate FM Network mode wired via 6.1's IF; Rhythm Arpeggiator `NoteIn` trigger wired via 6.3's MIDI. | 6.1, 6.3, Phase 2f, Phase 2d |
| 6.7 | `2026-04-27-phase-6.7-modulation-ring-ui.md` | Modulation Ring UI activation (S/H, Sync 1/16, Legato) for curve-node Y values; uses `ctx.bpm` and `ctx.midi_notes`. | 6.3, Phase 1 (ring widget scaffold), host BPM |

### Phase 7 — Heavy / research-grade

| # | Plan file | Ships | Depends on |
|---|-----------|-------|------------|
| 7 umb | `2026-04-27-phase-7-heavy-research.md` | Phase 7 index + research-blocker gating. | — |
| 7.1 | (not yet written) `2026-XX-XX-phase-7.1-geometry-wavefield.md` | Geometry Wavefield: 2-D wave-equation simulator on Hilbert-mapped spectrum. | Phase 1, Phase 3, Phase 2e, **Research Prompt 12 deliverable.** |
| 7.2 | (not yet written) `2026-XX-XX-phase-7.2-geometry-persistent-homology.md` | Persistent-homology denoiser on the time × frequency surface. | Phase 1, Phase 3, Phase 5b.1, **Research Prompt 13 deliverable.** |
| 7.3 | (not yet written) `2026-XX-XX-phase-7.3-harmony-fm-replicator.md` | 16-operator-DX-style FM re-spectraliser. Design-incomplete: scope vs FM Network, algorithm matrix, output blending. | 6.1, **DESIGN-INCOMPLETE questions resolved.** |
| 7.4 | (not yet written) `2026-XX-XX-phase-7.4-gpu-compute-path.md` | `wgpu`-based GPU compute path for Wavefield / PH / FM Replicator / Past. | At least one of 7.1/7.2/7.3 shipped on CPU and profiling shows CPU is insufficient. |

---

## Dependency graph

```
                                  Phase 1
                                     │
                      ┌──────────────┼──────────────────────────────┐
                      │              │                              │
                   Phase 2          Phase 3                       Phase 4
                  (a–g, 7         (BinPhysics)                  (PLPV phase)
                  parallel)           │                              │
                      │               │                              │
                      │     ┌─────────┼─────────┐                    │
                      │     │         │         │                    │
                      │     │      Phase 5a   Phase 5b.1             │
                      │     │      (Life)   (HistoryBuffer)          │
                      │     │                  │                     │
                      │     │      ┌───────────┼──────────┐          │
                      │     │      │           │          │          │
                      │     │   5b.2 Past   5b.3 Kin   5b.4 Mod      │
                      │     │                                        │
                      ├─ 2g ┘                                        │
                      │  └──────────── Phase 5c (Full Circuit)       │
                      │                                              │
                      │                                              │
                      │                ┌─────────────────────────────┤
                      │                │                             │
                      │                │ Phase 6                     │
                      │                │   ├── 6.1 IF ◄──────────────┘
                      │                │   ├── 6.3 MIDI
                      │                │   ├── 6.4 Cepstrum
                      │                │   ├── 6.2 Chromagram (needs 6.1)
                      │                │   ├── 6.5 Harmony (needs 6.1/2/3/4 + Phase 3)
                      │                │   ├── 6.6 FM Net + Arp NoteIn
                      │                │   │     (needs 6.1 + 6.3 + Phase 2f + Phase 2d)
                      │                │   └── 6.7 Mod-Ring UI (needs 6.3)
                      │                │
                      │                └── Phase 7 (RESEARCH-BLOCKED)
                      │                      ├── 7.1 Wavefield (Prompt 12)
                      │                      ├── 7.2 Persistent Homology (Prompt 13)
                      │                      ├── 7.3 FM Replicator (design-incomplete)
                      │                      └── 7.4 GPU path (conditional on CPU)
                      │
                      └── Phase 2 modules ship independently as soon as 1 lands.
```

### Critical-path read

Shortest path to **Harmony module shipping** (the headline 6.5 release):

```
Phase 1 → Phase 3 → Phase 4 → 6.1 → 6.2 → 6.3 → 6.4 → 6.5
```

Eight plans deep. 6.3 and 6.4 can be parallelised with 6.1/6.2.

Shortest path to the **first Phase 2 module shipping** (any quick-win):

```
Phase 1 → 2X
```

Two plans.

Shortest path to **Past module shipping**:

```
Phase 1 → Phase 3 → 5b.1 → 5b.2
```

Four plans.

---

## Ship cadence (suggested release tagging)

The codebase already ships `0.X.Y` minor releases. Suggested mapping of phases
to release notes:

| Release tag idea | Contents |
|---|---|
| `0.1.0` "Infra" | Phase 1 (foundation) — no audible change but unblocks everything. |
| `0.2.0` "Quick wins" | Phase 2 (any subset that lands together) — visible new module types. |
| `0.3.0` "Bin Physics" | Phase 3 + Phase 4 + Phase 5a. Audible: Life. |
| `0.4.0` "History" | Phase 5b (HistoryBuffer + Past + Kinetics + Modulate retrofit). |
| `0.5.0` "Circuit" | Phase 5c (Full Circuit). |
| `0.6.0` "Harmony" | Phase 6 — the headline release. |
| `0.7.0+` "Research" | Phase 7 — one minor per shipped sub-phase (7.1, 7.2, 7.3, 7.4). |

This is a **suggestion**, not a contract — phase ordering is the contract; the
exact tag/version is a release-time decision.

---

## How to start work on a plan

1. **Check `docs/superpowers/STATUS.md`** to confirm the plan you want to work
   on is still PLANNED (not SUPERSEDED, not already IN PROGRESS by another
   agent or another branch).
2. **Check the plan's dependency row above** — every dependency must be
   IMPLEMENTED before you start. If a dependency is missing, do that one
   first; do not inline the dependency's infrastructure into your plan.
3. **Open the plan file** at `docs/superpowers/plans/<plan-file>.md`. Read
   the banner. If it says PLANNED here but IMPLEMENTED in the banner, trust
   STATUS.md and re-verify the codebase.
4. **Pick an execution mode:**
   - **Subagent-driven** (recommended for plans with ≥6 tasks): use
     `superpowers:subagent-driven-development`. Fresh subagent per task,
     two-stage review.
   - **Inline** (small plans or hot iteration): use
     `superpowers:executing-plans`. Batch execution with checkpoints.
5. **When the plan ships:**
   - Flip its STATUS.md row to IMPLEMENTED.
   - Flip its banner to IMPLEMENTED.
   - Add a Self-Review checklist tick to the plan.
   - Update this index's "Status (umbrella)" column if the plan completes
     a phase.

---

## Out-of-scope items (parking lot)

These appear in `99-implementation-roadmap.md` § Parking lot or in module
specs but are explicitly **not** part of any phase plan. Re-evaluate after
Phase 6 ships.

- **Spectral Delay** — feature in many designs but no phase owner.
- **Polyphonic compander tied to MIDI tracking** — extends Harmony Companding
  beyond v1. Wait until 6.5 ships.
- **Lookahead Duck** — Future module v2.
- **Self-Punch** — Punch module v2.
- **Sidechain history** — Past module v2.
- **Multi-mode-per-slot** — design-wide v2.
- **Cepstral X-axis display** — UI polish; can land alongside any Phase 6
  ship as a small follow-up.
- **Dynamic automation naming** (`docs/superpowers/specs/2026-04-21-dynamic-automation-naming.md`)
  — explicitly DEFERRED at design time.

---

## Notes on this index

- Sub-plan files use the date `2026-04-27` because they were all authored on
  that date. The date is **the date of the plan**, not the date the work
  starts. When work starts, the relevant agent should update STATUS.md, not
  rename the plan.
- Where a sub-plan lives under an umbrella (e.g. 6.5 under Phase 6), the
  umbrella exists for navigation only. The umbrella does not need to be
  "implemented" — only its sub-plans.
- Phase numbering matches `99-implementation-roadmap.md` exactly. If a future
  edit to the roadmap renumbers a phase, this index must be updated to match.
- **Do not write a Phase 7 sub-plan ahead of its research deliverable.** See
  the Phase 7 umbrella for the gating rules.

---

## Self-review (for this index itself)

- [x] Every phase from `99-implementation-roadmap.md` is represented.
- [x] Every sub-plan file under `docs/superpowers/plans/` matching
      `2026-04-27-phase-*` is enumerated, except those whose plans are not
      yet written (called out as "(not yet written)").
- [x] Each row's "Depends on" column is satisfied somewhere earlier in the
      table or named as an external prerequisite (e.g. host BPM).
- [x] Phase 7's research-blocked status is called out explicitly so an agent
      can't accidentally start a Phase 7 sub-plan.
- [x] The index does not contain TDD tasks of its own — it points to the
      sub-plans that do.
- [x] The cross-references at the top (`STATUS.md`, roadmap) are the right
      ones for an agent to consult before starting work.

---

## Cross-references

- `ideas/next-gen-modules/99-implementation-roadmap.md` — original roadmap.
- `ideas/next-gen-modules/00-toc.md` — table of contents over the design corpus.
- `docs/superpowers/STATUS.md` — current implementation status (always wins
  over per-plan banners).
- `docs/superpowers/plans/2026-04-27-phase-1-foundation-infra.md` — the entry
  point for all next-gen work.
- `docs/superpowers/plans/2026-04-27-phase-6-pitch-harmony-umbrella.md` — Phase
  6 umbrella.
- `docs/superpowers/plans/2026-04-27-phase-7-heavy-research.md` — Phase 7
  umbrella.
