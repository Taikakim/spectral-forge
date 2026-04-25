# Superpowers plans & specs — implementation status

**Last updated:** 2026-04-24

This index is the authoritative status of every plan and design spec under
`docs/superpowers/`. **Agents: consult this file before following any plan or
design doc.** Each individual plan/spec also carries a status banner at the top,
but if the two disagree, this index wins.

**Status legend**

- **IMPLEMENTED** — merged to master; live in the codebase. The doc is kept for
  historical context. The code is now the source of truth, not the plan.
- **IN PROGRESS** — actively being executed task-by-task.
- **DEFERRED** — approved but intentionally not started yet. Still valid as a
  design reference.
- **SUPERSEDED** — never landed as written; a later plan/architecture replaced
  the approach. Do **not** follow these plans.

---

## Plans (`docs/superpowers/plans/`)

| Plan | Status | Notes |
|------|--------|-------|
| 2026-04-12-spectral-forge.md | IMPLEMENTED | Foundational plan. Single-engine `Pipeline` was later replaced by the modular `FxMatrix`; treat this plan as historical. |
| 2026-04-14-effects-engines.md | SUPERSEDED | Effects (Freeze, PhaseRand, Contrast) originally shipped as a fixed post-compressor stage. They now live as independent `SpectralModule` implementations in the FxMatrix. |
| 2026-04-14-gui-tabs-skeleton.md | SUPERSEDED | The Dynamics/Effects/Harmonic tab bar was removed when the per-slot module UI landed in plan D2. |
| 2026-04-19-modular-matrix.md | IMPLEMENTED | Infrastructure for the 8×8 FxMatrix + Dynamics slot. |
| 2026-04-19-plan-d1-module-foundation.md | IMPLEMENTED | `SpectralModule` trait, `RouteMatrix`, `apply_curve_transform`, module stubs. |
| 2026-04-19-serial-fx-chain.md | SUPERSEDED | Bridge plan to serialise Dynamics → Freeze → PhaseSmear via bool flags. Skipped in favour of the full FxMatrix approach. Do not follow. |
| 2026-04-19-variable-fft.md | IMPLEMENTED | Runtime-selectable FFT size (512 … 16384). |
| 2026-04-20-d2-ux-modules.md | IMPLEMENTED | Module popup, adaptive curve editor, per-slot SC/GainMode/name, matrix routing, T/S virtual rows, M/S DSP. |
| 2026-04-20-functional-module-bugs.md | IMPLEMENTED | Contrast / T/S Split / M/S Split / Freeze / silent-master fixes. |
| 2026-04-21-automation-presets.md | IMPLEMENTED | Generated automatable params (~1341), 1000 ms tooltips, JSON preset system. |
| 2026-04-21-bin-physics-infrastructure.md | DEFERRED | No `BinPhysics` code exists yet. Required for the Circuit/Life/Kinetics/Harmony/Modulate/Past/Rhythm module specs. |
| 2026-04-21-sidechain-refactor-plan.md | IMPLEMENTED | Single SC port + per-slot gain/channel selector; peak-hold curve; Freeze default threshold fix. |
| 2026-04-23-ui-parameter-spec.md | IMPLEMENTED | `CurveDisplayConfig`, curvature transform, shared hover routine, UI scaling. |
| 2026-04-24-calibration-audit.md | IN PROGRESS | T1–T3 merged (probe type, Dynamics probe, Freeze probe + formula fixes). T4–T11 pending. Tracked in the active TaskList. |
| 2026-04-24-ui-spec-cleanup.md | IMPLEMENTED | Closed the 2026-04-24 spec-deviation review. |

## Design specs (`docs/superpowers/specs/`)

| Spec | Status | Notes |
|------|--------|-------|
| 2026-04-12-spectral-forge-design.md | IMPLEMENTED | Foundational; superseded in places by the modular architecture spec. |
| 2026-04-19-modular-architecture-design.md | IMPLEMENTED | Authoritative for slot/matrix/module architecture. |
| 2026-04-21-automation-presets-design.md | IMPLEMENTED | |
| 2026-04-21-bin-physics-infrastructure.md | DEFERRED | Blocks the seven physics-driven module specs below. |
| 2026-04-21-circuit-module.md | DEFERRED | Depends on BinPhysics. |
| 2026-04-21-dynamic-automation-naming.md | DEFERRED | Explicitly deferred at design time. |
| 2026-04-21-harmony-module.md | DEFERRED | Depends on BinPhysics + instantaneous-frequency infra. |
| 2026-04-21-kinetics-module.md | DEFERRED | Depends on BinPhysics. |
| 2026-04-21-life-module.md | DEFERRED | Depends on BinPhysics. |
| 2026-04-21-matrix-amp-nodes.md | DEFERRED | |
| 2026-04-21-modulate-module.md | DEFERRED | Depends on BinPhysics + instantaneous-frequency infra. |
| 2026-04-21-past-module.md | DEFERRED | Depends on BinPhysics + history-buffer infra. |
| 2026-04-21-rhythm-module.md | DEFERRED | Depends on BinPhysics + host-BPM-sync infra. |
| 2026-04-21-sidechain-refactor-design.md | IMPLEMENTED | |
| 2026-04-23-ui-parameter-spec-design.md | IMPLEMENTED | **Authoritative source of truth for UI parameter display.** Addenda §2.3 / §3.4 / §4.4 pending from calibration-audit T10. |
| 2026-04-24-calibration-audit-design.md | IN PROGRESS | Paired with plan `2026-04-24-calibration-audit.md`. |

---

## Rules for editing this file

- When a plan merges to master, flip its row to **IMPLEMENTED** and update the
  matching banner at the top of the plan file.
- When a plan is abandoned, mark it **SUPERSEDED** and name the replacement.
- Never delete a plan or spec — add a status line and leave the content in place
  so later agents can read the history.
- Keep the table entries to one line; put longer context in the individual
  plan/spec banner.
