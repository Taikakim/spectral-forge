# Phase 6 — Pitch / Harmony Layer (Umbrella)

> **STATUS:** PLANNED, NOT STARTED. Authoritative status: `docs/superpowers/STATUS.md`.

> **For agentic workers:** This is an **index document**, not an implementable plan.
> Each sub-plan listed below is a self-contained TDD plan. Pick one, follow
> `superpowers:subagent-driven-development` (recommended) or
> `superpowers:executing-plans`. Sub-plan order matters — see § Dependency order.

**Goal:** unlock the Harmony module + the cross-cutting pitch/cepstrum infrastructure
that several other modules consume, per `ideas/next-gen-modules/99-implementation-roadmap.md`
§ Phase 6.

**Scope decision:** Phase 6 covers eight independent subsystems. Per
`superpowers:writing-plans` "Scope Check," each subsystem ships standalone, so
this phase is split into **seven sub-plans** plus this umbrella.

**Tech stack:** Rust, nih-plug, `realfft`, existing `Pipeline` / `FxMatrix` /
`SpectralModule` infrastructure. New utility modules in `src/dsp/{utils,
chromagram, harmonic_groups, cepstrum, midi}.rs`. New module `src/dsp/modules/harmony.rs`.

---

## Cross-phase prerequisites

Phase 6 sub-plans assume the following has already merged:

| Prereq | What it provides | Provided by |
|---|---|---|
| `ModuleContext` lifetime + Option fields | `instantaneous_freq`, `chromagram`, `midi_notes`, `peaks`, `unwrapped_phase`, `cepstrum_buf`, `bpm`, `beat_position`, `sidechain_derivative` all present as `Option<&'block T>` | Phase 1 (`2026-04-27-phase-1-foundation-infra.md`) |
| `ModuleSpec` extension fields | `wants_sidechain`, `panel_widget`, `needs_*: bool` declarations | Phase 1 |
| `BinPhysics` struct + read/write | `mass`, `velocity`, `crystallization`, `phase_momentum`, etc. used by Harmony Companding / Chordification | Phase 3 (`2026-04-27-phase-3-bin-physics.md`) |
| Per-bin unwrapped phase | `ctx.unwrapped_phase` populated, peaks via `ctx.peaks` | Phase 4 (`2026-04-27-phase-4-plpv.md`) |
| Modulate light v1 | FM Network mode declared but un-wired | Phase 2f (`2026-04-27-phase-2f-modulate-light.md`) |
| Rhythm v1 | Arpeggiator mode declared with `BPM` trigger source only | Phase 2d (`2026-04-27-phase-2d-rhythm.md`) |

The Phase 1 fields land as `None` and stay `None` until a Phase 6 sub-plan starts
populating them. The Phase 1 plan **does not** wire any DSP — Phase 6 is the consumer.

If you start Phase 6 work and discover a Phase 1 / 3 / 4 field missing, **stop**
and complete the prerequisite phase first. Don't paper over by adding the field
inline.

---

## Sub-plans

| # | Plan file | What it ships | Depends on |
|---|-----------|--------------|------------|
| 6.1 ✅ IMPLEMENTED | `2026-04-27-phase-6.1-instantaneous-freq.md` | `compute_instantaneous_freq` helper; populate `ctx.instantaneous_freq` per hop, gated by `needs_instantaneous_freq`. Landed 2026-04-30 on `feature/next-gen-modules-plans`. | Phase 1, Phase 4 |
| 6.2 | `2026-04-27-phase-6.2-chromagram-harmonic-groups.md` | `compute_chromagram` (12-element pitch class) + `harmonic_group_detect` (Klapuri-style); populate `ctx.chromagram` and `ctx.harmonic_groups`. | 6.1 |
| 6.3 | `2026-04-27-phase-6.3-midi-input.md` | nih-plug MIDI port wiring; `held_notes: [bool; 128]`, `pitch_classes: [bool; 12]`, `note_events: &[NoteEvent]` in `SharedState`; populate `ctx.midi_notes` / `ctx.held_pitch_classes`. | Phase 1 |
| 6.4 | `2026-04-27-phase-6.4-cepstrum.md` | Lazy cepstrum (extra real-FFT pair) gated by `needs_cepstrum`, exposed via `ctx.cepstrum_buf`. | Phase 1 |
| 6.5 | `2026-04-27-phase-6.5-harmony-module.md` | `HarmonyModule` with 8 v1 sub-effects: Chordification, Undertone, Companding, Formant Rotation, Lifter, Inharmonic (Stiffness/Bessel/Prime), Harmonic Generator, Shuffler. Defers FM Replicator + Persistent Homology to Phase 7. | 6.1, 6.2, 6.3, 6.4, Phase 3 |
| 6.6 | `2026-04-27-phase-6.6-fm-network-arpeggiator-noteIn.md` | Modulate FM Network mode wired via 6.1's IF; Rhythm Arpeggiator `NoteIn` trigger source wired via 6.3's MIDI. | 6.1, 6.3 |
| 6.7 | `2026-04-27-phase-6.7-modulation-ring-ui.md` | Modulation Ring UI activation (S/H, Sync 1/16, Legato) for the parameter categories Phase 1 scaffolded; uses `ctx.bpm` and `ctx.midi_notes`. | 6.3, Phase 5b BPM sync |

### Dependency order

```
Phase 1 / 3 / 4 already merged
        │
        ├──── 6.1 IF infrastructure ────┐
        │                               │
        ├──── 6.3 MIDI plumbing ────────┤
        │                               │
        ├──── 6.4 Cepstrum utility ─────┤
        │                               │
        │                               ▼
        │                          6.2 Chromagram + harmonic groups
        │                               │
        │                               ▼
        │                          6.5 Harmony module
        │
        ├──── 6.6 Modulate FM + Arp NoteIn (parallel to 6.5)
        │
        └──── 6.7 Modulation Ring UI (parallel to 6.5)
```

**Recommended landing order:** 6.1 → 6.3 → 6.4 → 6.2 → 6.6 (parallel) → 6.5 → 6.7.

6.6 and 6.7 can be developed in parallel with 6.5 because they touch separate
modules (Modulate / Rhythm / UI) and only consume already-published infra.

---

## Ship cadence

Each sub-plan ships as its own minor release:

- 6.1, 6.3, 6.4 are infrastructure-only — no audible change. Bundle into one
  `0.X.0` minor bump if all three land in the same cycle.
- 6.2 is infrastructure-only but observable via the calibration probe.
- 6.5 is the headline feature. Ship as `0.X+1.0` "Harmony" release.
- 6.6 and 6.7 ship as patches against the previous release if they merge after.

---

## Out of scope (deferred to Phase 7)

- **FM Replicator** (Harmony § c) — needs heavy partial-tracking + GPU consideration.
- **Persistent Homology** (Geometry §) — research-grade.
- **Sustained-harmonic-series with transient inclusion** (Harmony § e) — defer
  the transient-link pass; ship sustained-only detection in 6.2.
- **Polyphonic compander tied to MIDI tracking** — note in roadmap parking lot.
  Re-evaluate after 6.5 ships.

---

## Self-review checklist (run when each sub-plan completes)

- [ ] Sub-plan ships standalone (compiles + tests pass on its own).
- [ ] Calibration probe set extended for the new infra/module.
- [ ] STATUS.md updated to `IMPLEMENTED`; sub-plan banner flipped.
- [ ] Cross-phase docs that reference the new field updated (e.g. Phase 1 plan
      should back-pointer to which Phase 6 sub-plan populates each Option).

---

## See also

- `ideas/next-gen-modules/15-harmony.md` — source spec for Harmony module
- `ideas/next-gen-modules/16-modulate.md` — source spec for FM Network mode
- `ideas/next-gen-modules/17-rhythm.md` — source spec for Arpeggiator NoteIn
- `ideas/next-gen-modules/01-global-infrastructure.md` §3, §4, §6 — IF, chromagram, cepstrum
- `ideas/next-gen-modules/01-global-infrastructure.md` §8 — Modulation Ring
- `ideas/next-gen-modules/99-implementation-roadmap.md` § Phase 6 — original roadmap entry
- `docs/superpowers/STATUS.md` — current implementation status of all plans
