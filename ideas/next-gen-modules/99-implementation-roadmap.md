# Implementation Roadmap

**Purpose:** propose a phasing for the next-gen module work. Maps the
audited features in this folder to a sequence of PRs that respects
dependencies, packages each cut as a shippable release, and identifies
where natural pause-points sit (so Kim can divert energy to
non-spectral work between them).

**Status:** RESEARCH proposal — not approved. The exact ordering will
be validated in the brainstorming + planning sessions Kim wants to
run after reviewing this folder.

## Reading order

1. § Dependency map — what blocks what.
2. § Phasing — six phases, each ships independently.
3. § PR breakdown per phase — concrete unit-of-work suggestions.
4. § Risk and parking lot — non-blockers and rabbit-hole flags.

---

## Dependency map

```
                ┌─────────────────────────────────────────────────┐
                │  Foundation infra (Phase 1)                      │
                │                                                  │
                │  ModuleContext additions ─┐                      │
                │  ModuleSpec hints         │                      │
                │  Per-mode heavy_cpu       │                      │
                │  Per-module UI panel      │                      │
                │  Reset-to-default button  │                      │
                │  Modulation Ring UI       │                      │
                └─────────────┬─────────────┬──────────────────────┘
                              │             │
                ┌─────────────▼─────────────▼──────────────────────┐
                │  Quick-win modules (Phase 2)                      │
                │  Future, Punch, Rhythm, Geometry-light,           │
                │  Matrix Amp Nodes (already specced — implement)   │
                │  Circuit-light (BBD, Schmitt), Modulate-light     │
                │  (Phase Phaser, Bin Swapper without PLPV)         │
                └─────────────┬───────────────────────────────────┬─┘
                              │                                   │
                ┌─────────────▼───────────────┐  ┌────────────────▼─────┐
                │  BinPhysics (Phase 3)        │  │ PLPV phase (Phase 4) │
                │  base traits + read/write    │  │ unwrap + peaks       │
                │                              │  │ + module integration │
                └─────────────┬────────────────┘  └────┬─────────────────┘
                              │                       │
                ┌─────────────▼──────────────┬────────▼──────────────────┐
                │  Heavy physics modules      │  Phase-aware module       │
                │  (Phase 5)                  │  upgrades (Phase 5b)      │
                │  Life, Kinetics, Circuit    │  Modulate PLL, Past       │
                │  (full), History Buffer +   │  Stretch, Freeze phase    │
                │  Past (full)                │  evolve, Dynamics PLPV    │
                └─────────────┬───────────────┴───────────────────────────┘
                              │
                ┌─────────────▼─────────────────────────────────────┐
                │  Pitch/Harmony layer (Phase 6)                     │
                │  IF + chromagram + harmonic groups → Harmony,      │
                │  Modulate FM Network, Rhythm Arpeggiator           │
                │  with NoteIn                                        │
                └─────────────┬─────────────────────────────────────┘
                              │
                ┌─────────────▼─────────────────────────────────────┐
                │  Heavy / research-grade (Phase 7)                  │
                │  Wavefield, Persistent Homology, FM Replicator,    │
                │  GPU/wgpu compute path                              │
                └────────────────────────────────────────────────────┘
```

---

## Phase 1 — Foundation infra

> **Status:** IMPLEMENTED (2026-04-27 → release `0.X.0`). See plan
> `docs/superpowers/plans/2026-04-27-phase-1-foundation-infra.md`.

**Goal:** make every later phase cheaper. None of these ship a new
audible feature; they are *enabling infrastructure*.

**Total estimate:** 1 release cycle (~2 weeks of focused work, less
if some pieces defer).

### PRs

1. **`ModuleContext` additions.**
   - Add fields with `Option<&[f32]>` so existing modules ignore
     them: `unwrapped_phase`, `peaks`, `instantaneous_freq`,
     `chromagram`, `midi_notes`, `bpm`, `beat_position`,
     `sidechain_derivative`.
   - All fields default to `None` until later phases populate them.
   - No module changes; all existing modules continue to work.

2. **`ModuleSpec` UX hints.**
   - Add `wants_sidechain: bool` (set true on Punch when it lands).
   - Add `panel_widget: Option<fn(...)>` for per-module non-curve UI.
   - GUI honours `wants_sidechain` for default routing on first
     assignment.

3. **Per-mode `heavy_cpu` flag.**
   - Today `SpectralModule::tail_length()` exists but no
     `heavy_per_mode_cpu()`. Add a method returning bool keyed by
     active mode index.
   - Used by an "always-bypassed-on-low-end-hardware" preset filter.
     Defaults to module-level `heavy_cpu` for backward compat.

4. **Per-module UI panel callback.**
   - `editor_ui.rs` dispatches the optional `panel_widget` below the
     curve editor when present. Prerequisite for Rhythm Arpeggiator
     step grid (Phase 6) and Future module's pre-delay length picker
     (Phase 5b).

5. **Reset-to-default button.**
   - User-requested, not blocking but small. Single button in main
     editor frame; resets all params to nih-plug defaults; shows a
     confirmation dialog.

6. **Modulation Ring UI scaffolding.**
   - Per the brainstorm UX note: alt-click on any curve node or
     parameter exposes a small ring with [S/H], [Sync 1/16],
     [Legato] toggles.
   - Phase 1 ships only the ring widget and the click handler. The
     S/H + Sync + Legato logic depends on BPM sync (Phase 4) so
     toggles are disabled until then.

### Ship-decision

Ship Phase 1 as `0.X.0` "infra prep" release. No audible changes.
Note in changelog: "preparing for next-generation modules."

---

## Phase 2 — Quick-win modules (no infra deps)

**Goal:** add the most audible value with the least architectural
work. Every module here is something the user can hear and use.

**Total estimate:** 2 release cycles.

### Modules in scope

| Module | Audit file | Sub-effects to ship in v1 |
|---|---|---|
| **Matrix Amp Nodes** | `03-matrix-amp-nodes.md` | All amp modes from existing spec; auditing surfaced no blockers. |
| **Future** | `14-future.md` | Tape Print-Through (relocated from Past), Pre-Echo with Pre-Delay. Defer Lookahead Duck (Pipeline reorder) and Crystal Ball. |
| **Punch** | `19-punch.md` | Direct Punch + Inverse Punch. Defer Self-Punch. |
| **Rhythm** | `17-rhythm.md` | Euclidean, Arpeggiator (BPM-trigger only), Phase Reset. Defer Bin Swing (waits for Spectral Delay). |
| **Geometry-light** | `18-geometry.md` | Chladni Plate Nodes + Helmholtz Traps. Defer Wavefield + Persistent Homology. |
| **Circuit-light** | `10-circuit.md` | BBD Bins, Spectral Schmitt, Crossover Distortion (cheap modes only). Defer Vactrol, Transformer Saturation, Resonant Feedback (need BinPhysics). |
| **Modulate-light** | `16-modulate.md` | Phase Phaser, Bin Swapper, RM/FM Matrix, Diode RM, Ground Loop. Defer Gravity Phaser (needs BinPhysics phase_momentum), PLL Tear (best with PLPV), FM Network (needs IF). |

### PRs (one per module typically)

Order suggestion based on audibility and risk:

1. **Matrix Amp Nodes** — already fully specced; concrete
   implementation.
2. **Future** — most novel, easiest path for Pre-Echo (already
   infrastructure for buffers). Tape Print-Through is a pure write-
   ahead variant.
3. **Punch** — clean sidechain effect, immediate musical use.
4. **Rhythm** — needs Phase 1's BPM-sync stub plumbing in
   ModuleContext.
5. **Geometry-light** — Chladni + Helmholtz are independent of any
   other Phase 2 module; can run in parallel.
6. **Modulate-light** — phase-domain modes; PLL Tear waits for PLPV.
7. **Circuit-light** — the cheapest analog modes; full Circuit
   waits for BinPhysics.

### Ship cadence

- Ship each module as it lands (`0.X.1`, `0.X.2`, etc.). Don't wait
  for the bundle.
- Each module's PR is self-contained and reverts cleanly.
- Default presets ship with each new module.

---

## Phase 3 — BinPhysics infrastructure

> **Status:** IMPLEMENTED (2026-04-28 → release `0.X+1.0`). See plan
> `docs/superpowers/plans/2026-04-27-phase-3-bin-physics.md`.

**Goal:** unblock the heavy physics modules (Life, Kinetics,
full Circuit, Modulate's Gravity Phaser).

**Total estimate:** 1 release cycle.

### PRs

1. **BinPhysics base struct.** Per-channel arrays for the established
   fields in `01-global-infrastructure.md`: `velocity`, `acceleration`,
   `mass`, `flux`, `temperature`, `crystallization`, `phase_momentum`,
   `noise_floor`, `decay_estimate`, `slew`, `bias`, `lock_target_freq`.
2. **Read access in ModuleContext.** Modules opt in via a new
   `bin_physics: Option<&BinPhysics>` field.
3. **Write access via a `BinPhysicsWriter`.** Modules that mutate
   physics state declare so in their ModuleSpec via
   `writes_bin_physics: bool`. Pipeline orders writers before
   readers within a hop.
4. **Calibration probes.** BinPhysics state is exposed to the
   calibration probe system so its evolution can be regression-tested.

### Ship-decision

Phase 3 is one PR (the infra) plus the audit-recommended trait
extensions. No new audible features yet. Released as `0.X+1.0`
minor bump.

---

## Phase 4 — PLPV phase unwrapping

> **Status:** IMPLEMENTED (2026-04-28). See plan
> `docs/superpowers/plans/2026-04-27-phase-4-plpv-phase.md`.

**Goal:** unlock the cleaner-phase quality across all phase-touching
modules.

**Total estimate:** 1 release cycle for infra, then ongoing per-module
opt-in.

### PRs (per `20-plpv-phase-cross-cutting.md`)

1. **Phase 4.1** — per-bin unwrap, expose
   `ctx.unwrapped_phase: Option<&[f32]>`. Global on/off switch
   (`plpv_enable: BoolParam`). No module changes. Default on.
2. **Phase 4.1.5** — low-energy bin phase damping. New plugin-level
   `plpv_phase_noise_floor_db` FloatParam. Damping kernel ships
   with the unwrap stage. See `20-plpv-phase-cross-cutting.md`
   § Phase 1.5.
3. **Phase 4.2** — peak detection, expose `ctx.peaks`. Configurable
   peak count + threshold.
4. **Phase 4.3a** — Dynamics PLPV integration. Audible improvement.
5. **Phase 4.3b** — PhaseSmear PLPV integration. Audible improvement.
6. **Phase 4.3c** — Freeze PLPV integration. Audible improvement.
7. **Phase 4.3d** — MidSide (shipped) PLPV integration; verify with
   the inter-channel phase-drift probe in
   `20-plpv-phase-cross-cutting.md` § Calibration impact.

Each module integration is gated by its own boolean param so users
can A/B compare. Calibration tests verify probe traces match within
ε between PLPV-on and PLPV-off paths (a forced invariant). Adaptive
per-frame coherence policy is **deferred to v2** — see the same
file's § v2 — Adaptive per-frame coherence policy.

### Research prerequisite

`90-research-prompts.md` Prompt 1 (PVX peak-locking math validation)
should run *before* Phase 4.2 lands so the peak-detection algorithm
is the right one.

---

## Phase 5 — Heavy physics modules + Past

**Goal:** the big-ticket physics modules that have been deferred
since the original spec.

**Total estimate:** 3 release cycles (these modules are large).

### Phase 5a — Life

- Implements all sub-effects from `11-life.md`.
- Multi-mode-per-slot deferred to v2 per the audit.
- Energy-conservation invariant tested in `tests/`.
- **Research prerequisite:** Prompt 9 (energy-conservation in
  spectral diffusion).

### Phase 5b — Kinetics + History Buffer + Past

- History Buffer infra (`01-global-infrastructure.md` § History
  Buffer) ships with Past.
- Past ships all sub-effects per `13-past.md` (Granular, Decay
  Sorter, Convolution, Reverse, Stretch).
- Kinetics ships all sub-effects per `12-kinetics.md`.
- Modulate's Gravity Phaser and PLL Tear get retrofitted to use
  `ctx.peaks` from Phase 4.
- **Research prerequisite:** Prompt 3 (phase-coherent stretch
  playback) for Past Stretch; Prompt 8 (spring stability) for
  Kinetics.

### Phase 5c — Full Circuit

- All Circuit sub-effects per `10-circuit.md` (Vactrol, Transformer,
  Resonant Feedback via matrix routing).
- **Research prerequisite:** Prompt 7 (SIMD analog kernels).

---

## Phase 6 — Pitch/Harmony layer

**Goal:** the Harmony module + IF/chromagram infrastructure that
several other modules consume.

**Total estimate:** 2 release cycles.

### PRs

1. **Phase 6.1** — IF infrastructure. Per-bin instantaneous
   frequency exposed via `ctx.instantaneous_freq`. Used by Modulate
   FM Network, Past advanced modes, Harmony.
2. **Phase 6.2** — Chromagram + harmonic-group infrastructure.
   Per `01-global-infrastructure.md` § Pitch & chord detection.
3. **Phase 6.3** — MIDI input plumbing. Per
   `01-global-infrastructure.md` § MIDI input plumbing. Used by
   Harmony, Rhythm Arpeggiator (NoteIn trigger), Compander
   (poly-tracked).
4. **Phase 6.4** — Cepstral analysis utility (FFT inverse-FFT
   wrapper). Used by Harmony Lifter sub-effect. **Research
   prerequisite:** Prompt 11 (cepstral edge cases).
5. **Phase 6.5** — Harmony module shipping per `15-harmony.md`,
   minus deferred sub-effects (FM Replicator → Phase 7,
   Persistent Homology → Phase 7).
6. **Phase 6.6** — Modulate FM Network (now possible with IF).
7. **Phase 6.7** — Rhythm Arpeggiator NoteIn upgrade.
8. **Phase 6.8** — Modulation Ring UI activation (the toggles
   become live now that BPM + Sync infrastructure is mature).

### Research prerequisite

Prompt 2 (SOTA real-time pitch tracking) should run before Phase 6.1
so the IF/chromagram architecture is informed by current best
practice.

---

## Phase 7 — Heavy / research-grade

**Goal:** the genuinely expensive features that may need GPU compute
or are research-grade.

**Total estimate:** open-ended. May span multiple release cycles.

### PRs

1. **Geometry Wavefield.** Per `18-geometry.md` § Wavefield. Mark
   `heavy_cpu = true` per-mode. **Research prerequisite:** Prompt 12.
2. **Geometry Persistent Homology.** Per `18-geometry.md` §
   Persistent Homology. Marked `always_bypassed_on_low_end`.
   **Research prerequisite:** Prompt 13.
3. **Harmony FM Replicator.** Per `15-harmony.md` § Re-Synthesis.
   Heavy CPU; consider GPU compute.
4. **GPU compute path (wgpu).** For users who want Wavefield /
   FM Replicator without paying CPU. Per
   `01-global-infrastructure.md` § GPU/SIMD discussion.
   - This is a major undertaking; only worth doing if Phase 7's
     CPU-only modes prove too expensive for the target hardware.

---

## Risk and parking lot

### Risks

1. **PLPV may not deliver audible improvement.** The brainstorm
   asserts it does, but until benchmarked we can't be sure.
   Mitigation: Phase 4.1 ships the unwrap *infrastructure* without
   any module change, so we can A/B with sample audio before
   committing module integration.
2. **BinPhysics state explosion.** Adding `slew`, `bias`,
   `decay_estimate`, `lock_target_freq` per channel per bin grows
   memory. Sanity-check: 8193 × 4 fields × 4 bytes × 2 channels =
   ~520 KB per slot. Multiplied by 9 slots = ~4.7 MB. Acceptable
   but not trivial.
3. **Per-module UI panel scope creep.** Once panels exist, every
   module will want one. Risk: consistency drift. Mitigation:
   restrict panels to *non-curve* state only (step grids, mode
   pickers, etc.).
4. **Calibration regression cost.** Each new module needs a
   calibration probe set + golden trace. Phase 2 adds 6 modules =
   6 sets of probes. Mitigation: make probe-add part of the module
   PR template.
5. **GPU path is a tar pit.** Phase 7's wgpu compute work could
   easily eat 6 months. Don't start unless Phase 7's CPU path is
   demonstrably insufficient.

### Parking lot — explicitly NOT in this roadmap

- **Spectral Delay module.** Mentioned as the future home for Bin
  Swing (`17-rhythm.md` § a). Not specced anywhere. Deferred until
  someone explicitly asks for it.
- **Polyphonic compander tied to MIDI tracking.** Brainstorm note
  on Cat 3 #11 Harmonic Companding. Belongs eventually with
  Harmony and MIDI plumbing. Not in any module yet.
- **Lookahead Duck (Future module sub-effect).** Requires a
  Pipeline reorder. Deferred to v2 of Future.
- **Self-Punch.** Deferred per `19-punch.md`.
- **Sidechain history.** Deferred per `13-past.md` § Sidechain
  history.
- **Multi-mode-per-slot.** Multiple sub-effects active in one slot.
  Mentioned for Circuit and Life. Deferred to v2 across the board.
- **Cepstral X-axis display.** Per `02-architectural-refactors.md`
  if I read that right. Specialised display for the Harmony Lifter
  sub-effect. Worth doing alongside Harmony but not blocking it.

---

## Suggested cadence summary

| Phase | What | Cycles | Audible? |
|---|---|---|---|
| 1 | Foundation infra | 1 | No |
| 2 | Quick-win modules | 2 | Yes — 7 new modules |
| 3 | BinPhysics | 1 | No |
| 4 | PLPV phase | 1 + N | Yes — quality improvements |
| 5 | Life + Kinetics + Past + Circuit-full | 3 | Yes — 4 large modules |
| 6 | Pitch/Harmony layer | 2 | Yes — Harmony + retrofits |
| 7 | Heavy / research-grade | open | Yes — Wavefield, FM Replicator |

**Total audible-change phases:** 5 (Phases 2, 4, 5, 6, 7).
**Total infrastructure-only phases:** 2 (Phases 1, 3).

The infrastructure-only phases are short on purpose. They unlock
disproportionate downstream value, but they don't ship features —
so they're easy to deprioritise. Don't.

## Branch / PR strategy

- **One module = one PR.** Avoid bundling.
- **One infra change = one PR.** Trait extensions, ModuleContext
  fields, etc., land separately from the consumers.
- **Each PR has its own calibration trace.** No "I'll fix the
  probes later."
- **No PR is merged without a fresh audio-render-on-test-input.**
  Even if the test suite passes, take 30 seconds to listen.
- **Banner the spec status.** Every spec under
  `docs/superpowers/specs/` should have its banner updated when its
  module ships.

## When to convert these notes into specs

After Kim's review, the per-module audit files in this folder become
the *input* for the brainstorming + planning sessions. The output of
those sessions is a fresh design spec under
`docs/superpowers/specs/2026-XX-XX-<module>-module-spec.md` that
supersedes the deferred 2026-04-21 specs. The audit files in this
folder can then either:

1. Be archived (moved to `docs/future-ideas/archive/`), or
2. Stay in place as a research-trail reference.

Recommendation: option 2. The audits trace *why* the new spec
chose what it chose; that traceability is worth keeping.
