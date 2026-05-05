# Spectral Forge Stabilization Backlog

**Last updated:** 2026-05-06
**Purpose:** Persistent tracker of known issues, user directives, and design decisions for the Spectral Forge stabilization effort. Survives context compaction. Update this doc as facts change.

---

## Build & deploy facts

- **Dev plugin install path:** `~/.clap/spectral/dev/spectral_dev.clap` (not `~/.clap/spectral_forge.clap`).
- **Dev build command:** `cargo build --release --features dev-build`. The `dev-build` Cargo feature (`Cargo.toml:35`, cfg-gates `CLAP_ID`/`VST3_CLASS_ID`/`NAME` in `lib.rs`) gives the dev plugin a distinct identity so Bitwig doesn't confuse it with the release version.
- **Install step:** `cp target/release/libspectral_forge.so ~/.clap/spectral/dev/spectral_dev.clap` (the .so is just a .clap with the Linux extension).
- **Workflow gotcha:** previously bundling to `~/.clap/spectral_forge.clap` did nothing because Bitwig was loading the dev path. All earlier "still broken" reports against post-fix builds were testing stale (pre-Tasks-1-16) code.

## User-stated directives (apply globally)

- **Universal slider traversal:** at slider value `v = -1` the offset should reach `y_min`; at `v = +1` reach `y_max`; at `v = 0` reach `y_natural`. This must hold for *every* curve. Implies the current `y_natural == y_max` patterns (MIX, AMOUNT) must be redesigned — the positive half of the slider must do something useful.
- **MIX default 100% wet for every module.**
- **Master output stage:** soft clipper belongs at the very last output stage (post-FxMatrix), not per-PAST.
- **Master Floor default:** -120 dBFS (currently -100).
- **Tilt range:** allow ~2× steeper angles than current.
- **Module-switch isolation:** switching a slot's module type should reset that slot's curves/nodes/per-mode state. Current behaviour leaks state across module types.
- **Dry/wet mix at 0% (full dry) gives true bypass** — already works. The dry path is bit-perfect.

## Open issue backlog

Numbered for cross-reference. Status: 🔴 critical · 🟠 important · 🟡 normal · ⚪ deferred / paused.

| # | Status | Issue | Source |
|---|---|---|---|
| 1 | 🔴 | All-modules-disabled still produces audible processing on the wet path | user msg |
| 2 | 🟠 | Soft clipper architecture: move from PAST-internal to master output stage; clamps even at silent input today | user msg |
| 3 | 🟡 | PAST AMOUNT/Age/Smear/MIX sliders cap at "0" — partially understood (Age has total_history_seconds=0 + 1-decimal rounding; AMOUNT/Smear/MIX hit `y_natural==y_max` dead-half) | user msg + diagnosed |
| 4 | 🔴 | Routing matrix globally non-functional — UI edits don't reach DSP, all modules just sequentially process. Bug class (a): GUI→params→pipeline plumbing. | user msg + screenshot |
| 5 | 🟠 | MIX default should be 100% wet for every module | user msg |
| 6 | 🟡 | PAST SMEAR is binary toggle at 50% (apply_granular only); ignored in 4 of 5 PAST modes | user msg + audit |
| 7 | 🟡 | The -1..+1 slider rigidity creates the dead-half problem on `y_natural==y_max` curves | user msg |
| 8 | ✅ | Dev plugin identity needs distinct CLAP ID — already exists via `dev-build` feature | resolved |
| 9 | 🟡 | Dynamics THRESHOLD floors at db_min (-60 default) — tied to (10) | user msg |
| 10 | 🟠 | Master Floor default should be -120 dBFS | user directive |
| 11 | 🟡 | Tilt range needs ~2× steeper angles | user directive |
| 12 | 🔴 | Smearing accumulates over time, even with no modules loaded. Power-cycling plugin clears it. Wet path only (mix=0% is clean). | user msg |
| 13 | 🔴 | Module-switch carryover is universal (graph display is centralised → every module potentially affected) | user clarification |
| 14 | 🔴 | PAST mode UI button doesn't respond. Open question: should PAST have a single mode-selector at all, vs per-mode buttons like Kinetics? | user msg |
| 15 | 🟡 | Freeze: most curves work; Resistance has weak audible effect — likely a level-mismatch in the kernel | user msg |
| 16 | 🟡 | Freeze PORTAMENTO range: should be 0ms (instant)..~750ms; currently 40..1000 | user msg |
| 17 | 🟡 | Node "virtual range" beyond graph rect: clamp display to rect, allow node.y past visible bounds for headroom | user msg |
| 18 | 🟡 | Offset-aware node y-range scaling: when baseline is offset toward an extreme, the same drag distance should yield more headroom toward the other extreme | user msg + screenshot |

## Sub-project decomposition

Six sub-projects with critical-path ordering:

- **(A) Pipeline bypass + routing + soft clipper + smearing fix** ← *currently brainstorming*. Combined per Approach 1 (single-spec stabilization sweep). Covers issues #1, #2, #4, #12. Critical path: blocks reliable testing of everything else.
- **(B) Module-state isolation + slot lifecycle.** Covers #13, #14. Universal carryover bug + PAST mode UI dead.
- **(C) Curve UX redesign (universal -1..+1 traversal).** Covers #3, #5, #7, #17, #18. Major UX rework — kills the `y_natural==y_max` dead-half pattern.
- **(D) Master axis defaults + per-curve range adjustments.** Covers #9, #10, #11, #16.
- **(E) DSP semantics completion.** Covers #6, #15. PAST AMOUNT/SMEAR plumbing across modes; Resistance fix.
- **(F) Spec / spec-table follow-ups.** PEAK HOLD DSP mismatch deferred from prior plan.

## Sub-project A — current state (in design)

- **Approach chosen:** Approach 1 — single-spec stabilization sweep covering routing + soft-clipper-move + Empty-slot bypass + smearing-over-time.
- **Phase plan** (from Section 1 of design):
  - Phase 1: diagnostics-only — characterize routing matrix break and smearing-over-time root cause.
  - Phase 2: routing matrix plumbing fix.
  - Phase 3: soft clipper architecture move (PAST → master output stage).
  - Phase 4: smearing-over-time fix (shape determined by Phase 1 diagnostic).
  - Empty-slot bypass semantics: paragraph-sized decision in Phase 2 or 3 — wet path with all slots Empty must be audibly transparent (matching dry); we do NOT add a true-bypass-skips-STFT mode.

## Diagnostic facts so far

- Routing failure mode: bug type (a) — UI edits don't reach DSP. The route_matrix snapshot in `pipeline::process` is using defaults regardless of user matrix edits. Code path looks correct on paper (`fx_matrix::process_hop` at lines 506-687 properly gates on `send < 0.001`), so the break is upstream — params or snapshot.
- Smearing-over-time happens with NO modules loaded → it's pipeline-base, not module-specific. Likely candidates: BinPhysics buffers, history buffer, STFT internal state, modulation ring, slot_curve_cache, FFT scratch.
- mix=0% gives true bypass → dry path is bit-perfect.

## Design decisions made

- 2026-05-05: dev-build identity via `dev-build` Cargo feature flag (already exists).
- 2026-05-06: stabilization sweep covers four issue clusters in one sub-project (Approach 1).
- 2026-05-06: Empty-slot bypass = "wet path transparent enough you can't tell wet from dry" (does NOT skip STFT — Bitwig's bypass button is the host-level escape).
- 2026-05-06: this tracker doc serves as the single source of truth across sessions and context resets.

## Update log

- 2026-05-06: doc created with full backlog, sub-project decomposition, Sub-project A Phase plan, dev-install workflow facts.
