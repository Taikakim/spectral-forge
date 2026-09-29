# Stabilization Sweep Implementation Plan

> **Status:** IMPLEMENTED (sub-project A) — routing-matrix fix (`f3d7d53`), soft clipper moved from Past to `MasterModule` (`1ae696b`, `f53dfa6`), PLPV smearing fix (`f26c3ac`), Empty-slot wet-path transparency; closed in tracker (`5649285`). Sub-projects B + C continue in [`2026-05-07-stabilization-sweep-bc.md`](2026-05-07-stabilization-sweep-bc.md). The code is the source of truth; this plan is kept for history. See [../STATUS.md](../STATUS.md).

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix four entangled audio-path bugs — routing matrix non-functional, soft clipper at wrong layer (PAST→Master with toggle), smearing-over-time accumulating across blocks, and Empty-slot wet path opaqueness.

**Architecture:** Phase 1 produces a written diagnostic report; Phases 2-4 apply targeted fixes shaped by Phase 1; final phase regresses + installs to the dev path. Pre-flight investigation already identified the routing bug (GUI mutates Arc<Mutex<RouteMatrix>>, audio reads FloatParams — separate stores never sync). The smearing-over-time accumulator is unknown and Phase 1's job to find.

**Tech Stack:** Rust 1.x, `nih-plug-egui` 0.31, `realfft`. No new dependencies.

---

## File structure

| File | Purpose |
|---|---|
| `src/dsp/pipeline.rs` | Possibly site of smearing-fix; contains `route_matrix_snap` (correct) |
| `src/dsp/fx_matrix.rs` | Possibly site of smearing-fix (state containers); contains `prev_mags`, `amp_state`, `slot_phys`, `mix_phys` |
| `src/editor/fx_matrix_grid.rs` | **Routing fix site:** DragValue currently mutates raw `&mut f32`; must use `setter.set_parameter(matrix_cell(col, row), v)` |
| `src/dsp/modules/master.rs` | Adds master soft clipper logic to `MasterModule::process()` |
| `src/dsp/modules/past.rs` | Removes `apply_soft_clip` call (line 372), `PastScalars::soft_clip` field, the function definition (line 643) |
| `src/params.rs` | Adds `master_clip_enabled: BoolParam` |
| `src/editor_ui.rs` | Adds master clipper toggle button in master row; passes ParamSetter into matrix grid call |
| `tests/route_matrix_propagation.rs` (new) | Routing tests — 3 cases |
| `tests/master_soft_clip.rs` (new) | Soft clipper tests — 3 cases |
| `tests/empty_slot_smear_soak.rs` (new) | Smearing soak regression — 1 long-running test |
| `docs/superpowers/2026-05-06-phase-1-diagnostics.md` (new) | Phase 1 written report |
| `docs/superpowers/2026-05-06-stabilization-backlog.md` | Tracker doc — updated as plan progresses |
| `tests/past*.rs` | Cleanup — remove `soft_clip` references from test fixtures |

---

## Important context for every task

- **Work on branch `feature/next-gen-modules-plans`.** Don't switch branches. Don't merge.
- **Untracked files** in `ideas/`, `.claude/`, and a few `docs/superpowers/plans/2026-05-05-*` files are intentionally untracked — leave them alone.
- **Dev install path:** `~/.clap/spectral/dev/spectral_dev.clap`. Build command: `cargo build --release --features dev-build`. Install: `cp target/release/libspectral_forge.so ~/.clap/spectral/dev/spectral_dev.clap`. **Never** install to `~/.clap/spectral_forge.clap` — that path is ignored by Bitwig in the user's setup.
- **Pre-existing failures with `--features=probe`:** five `*_amount_default_probes_50_pct` tests in `tests/calibration_roundtrip.rs` are stale and unrelated to this plan. They MUST NOT be "fixed" — they're tracking unrelated work and would need their own plan. Just confirm count remains exactly 5 in the final regression.
- **TDD discipline:** every task that adds behavior writes the test first, runs it to confirm failure, then implements, then re-runs to confirm pass.
- **Commits per task as specified.** Don't squash. Don't amend (per CLAUDE.md).

---

### Task 1: Setup — verify dev-build baseline

**Files:** none modified.

- [ ] **Step 1: Verify clean working tree**

Run: `git status`

Expected: working tree clean (only the listed untracked files).

- [ ] **Step 2: Verify the dev-build feature exists**

Run: `grep -n 'dev-build' Cargo.toml`

Expected: a line `dev-build = []` near other feature definitions.

Run: `grep -n 'cfg(feature = "dev-build")' src/lib.rs`

Expected: at least 3 hits where `CLAP_ID`, `VST3_CLASS_ID`, `NAME` get cfg-gated overrides.

- [ ] **Step 3: Run baseline tests on master code**

Run: `cargo test`

Expected: 0 failures (the existing pre-Tasks-this-plan state is fully green for non-probe tests).

Run: `cargo test --features=probe 2>&1 | grep -c 'FAILED'`

Expected: exactly `5` (the 5 pre-existing `*_amount_default_probes_50_pct` failures). Capture this baseline number.

- [ ] **Step 4: Verify dev-build compiles**

Run: `cargo build --release --features dev-build`

Expected: SUCCESS, no errors. May warn about unused tokens — that's fine.

- [ ] **Step 5: No commit (this task is verification only)**

If any check fails, STOP and escalate (BLOCKED). The plan assumes a clean baseline.

---

### Task 2: Phase 1 — Routing matrix break audit

**Files:**
- Create: `docs/superpowers/2026-05-06-phase-1-diagnostics.md`

Static-audit the GUI→params→pipeline chain for routing. The pre-flight investigation already found the bug; this task verifies and documents.

- [ ] **Step 1: Read the pipeline snapshot site**

Run: `grep -n 'route_matrix_snap\|matrix_cell' src/dsp/pipeline.rs | head`

Read the function around line 950-977 and confirm: pipeline assembles `route_matrix_snap.send[col][r]` from `params.matrix_cell(r, col).smoothed.next()`. That's the FloatParam path.

- [ ] **Step 2: Read the GUI mutation site**

Run: `grep -n 'send_val\|DragValue::new(send_val)\|route_matrix.send\[' src/editor/fx_matrix_grid.rs`

Read the function `paint_fx_matrix_grid` around line 211-228. Confirm: the DragValue takes `&mut route_matrix.send[row][col]` (a raw `&mut f32`). It does NOT call `setter.set_parameter`. The FloatParam (`matrix_cell(col, row)`) is never written to from this site.

- [ ] **Step 3: Confirm the convention**

Run: `grep -n 'matrix_cell(dst, src)\|src.*dst.*send' src/params.rs | head`

Confirm the comment at params.rs:793 says: `route_matrix.send[src][dst] ↔ matrix_cell(dst, src)`. So pipeline reads `matrix_cell(r=dst, col=src)` and writes to `send[col=src][r=dst]` — semantically `send[src][dst]`. That's correct.

- [ ] **Step 4: Verify the audio path consumes from snap**

Run: `grep -n 'route_matrix.send\[\|fn process_hop' src/dsp/fx_matrix.rs | head`

Read `process_hop` lines 506-687. Confirm it reads `route_matrix.send[src][s]` (line 515) and `route_matrix.send[src][8]` (line 677) — semantically `send[src][dst]`. The route_matrix passed in IS the snap from pipeline (so the convention matches).

- [ ] **Step 5: Write the diagnostic report**

Create `docs/superpowers/2026-05-06-phase-1-diagnostics.md`:

```markdown
# Phase 1 — Stabilization Sweep Diagnostics

## 1. Routing matrix break

**Bug confirmed:** GUI→params→pipeline chain breaks at the GUI→params link.

**Evidence:**
- `paint_fx_matrix_grid` at `src/editor/fx_matrix_grid.rs:211-228` mutates `route_matrix.send[row][col]` via a raw `&mut f32` reference exposed to the egui DragValue.
- The audio path reads from `params.matrix_cell(r, col).smoothed.next()` at `src/dsp/pipeline.rs:970`. This is a separate FloatParam.
- The Arc<Mutex<RouteMatrix>> in params is read by pipeline ONLY for `virtual_rows`, `amp_mode`, `amp_params` (lines 953-959) — NOT for `send`.
- Convention is correct: `route_matrix.send[src][dst] ↔ matrix_cell(dst, src)` per the comment at `params.rs:793`.

**Fix shape:** rewire the DragValue at `fx_matrix_grid.rs:217` to call
`setter.set_parameter(matrix_cell(col, row), value)` instead of mutating
the f32 directly. Caller (editor_ui.rs) must pass a `&ParamSetter` into
`paint_fx_matrix_grid`.

## 2. Smearing-over-time

(Filled in by Task 3.)
```

Commit will land after Task 3 fills the smearing section.

- [ ] **Step 6: Run a regression test that pins the routing break**

Add to a new file `tests/route_matrix_propagation.rs`:

```rust
//! Phase 1 regression: pinning the GUI→DSP routing break observed in 2026-05-06
//! diagnostics. After Phase 2, this test asserts the chain works.

use spectral_forge::params::SpectralForgeParams;

#[test]
fn matrix_cell_param_exists_for_all_valid_coordinates() {
    use spectral_forge::param_ids::{NUM_MATRIX_ROWS, NUM_SLOTS};
    let p = SpectralForgeParams::default();
    for r in 0..NUM_MATRIX_ROWS {
        for col in 0..NUM_SLOTS {
            if r == col { continue; } // diagonal is module-loaded indicator, not a route
            assert!(p.matrix_cell(r, col).is_some(),
                "matrix_cell({r}, {col}) should be Some");
        }
    }
    // Out-of-range returns None.
    assert!(p.matrix_cell(NUM_MATRIX_ROWS, 0).is_none());
    assert!(p.matrix_cell(0, NUM_SLOTS).is_none());
}
```

Run: `cargo test --test route_matrix_propagation`

Expected: PASS. (This test confirms the FloatParam infrastructure is healthy; the actual routing-fix tests come in Task 5.)

- [ ] **Step 7: No commit yet — combined with Task 3's report**

---

### Task 3: Phase 1 — Smearing-over-time accumulator audit

**Files:**
- Modify: `docs/superpowers/2026-05-06-phase-1-diagnostics.md` (append §2)

Static-audit `pipeline.rs` and `fx_matrix.rs` for state containers that update across blocks. Goal: identify any container whose update is NOT properly gated when `bin_physics_in_use` is false AND no audible transformation should be happening.

- [ ] **Step 1: Inventory state containers in `FxMatrix`**

Run: `grep -n 'self\.\(prev_mags\|amp_state\|slot_phys\|mix_phys\|writer_bits\|virtual_out\|slot_out\|mix_buf\|amp_scratch\|slot_supp\)' src/dsp/fx_matrix.rs | head -40`

For each, characterize: is it cleared per block (mix_buf, slot_out)? Updated only when bin_physics_in_use? Or stateful across blocks?

- [ ] **Step 2: Check `amp_state` carefully**

Run: `grep -n 'amp_state\|AmpState\|AmpMode\|fn apply' src/dsp/amp_modes.rs src/dsp/fx_matrix.rs | head -30`

Read `AmpState::apply` body. Determine: does it carry state between blocks (e.g., a smoother lerping toward target)? If yes, what bounds it? At amp_mode=Linear with default amp_params, the apply should be `gain * v` with no state — verify.

- [ ] **Step 3: Check pipeline state containers**

Run: `grep -n 'pub \(struct\|enum\)\s*Pipeline\|^pub struct Pipeline\|^\s\+self\.' src/dsp/pipeline.rs | head`

Read `Pipeline` struct fields. Look for: smoothers, IIR coefficients, history buffers, decay accumulators. Especially: `slot_curve_cache`, `peak_hold`, anything with the word `state`.

- [ ] **Step 4: Identify candidate accumulators**

Make a list of candidates. For each, hypothesize:
- (a) what state it carries
- (b) whether it's properly bounded
- (c) whether it's cleared/reset on block boundaries
- (d) whether it's gated by some "is module loaded" condition

- [ ] **Step 5: Test the accumulators with a unit test**

Create `tests/empty_slot_smear_audit.rs`:

```rust
//! Phase 1 audit: drive a Pipeline with all-Empty slots and continuous noise,
//! capture state-container max magnitudes after a short and long run, and
//! flag any container that grew significantly between the two.

use spectral_forge::dsp::pipeline::Pipeline;
use num_complex::Complex;

#[test]
#[ignore] // Phase 1 audit — run manually with `cargo test --test empty_slot_smear_audit -- --include-ignored --nocapture`
fn audit_state_growth_under_empty_slot_noise() {
    let sample_rate = 48_000.0_f32;
    let fft_size = 2048_usize;
    let mut pipeline = Pipeline::new(sample_rate, fft_size);

    // Drive 1 second (then 10 seconds) of pink noise through the pipeline,
    // measuring output magnitude statistics. With all Empty slots and
    // default routing, output should equal input (modulo STFT roundtrip
    // noise <1e-3). Any progressive growth is the smear.

    // Implementation note: This is an instrumented harness, not a strict
    // assertion. The pipeline doesn't expose its internals directly —
    // the audit reads `cfg(feature = "probe")`-gated probe outputs.

    // For now, this test is a placeholder for Phase 1 — the implementer
    // adds probe instrumentation in src/dsp/pipeline.rs gated under
    // #[cfg(feature = "probe")] and reads it from this test. Specifics:
    //
    //   - Pipeline::process() snapshots: max(|prev_mags|), max(|mix_phys.velocity|),
    //     sum of |slot_phys[s].velocity| over s, max of |amp_state[*][*][*]| internal.
    //   - Each snapshot stored in a thread-local Vec, read by this test
    //     after driving the pipeline for N blocks.
    //
    // This test is skipped by default (#[ignore]) since the probe data
    // collection adds overhead. It's meant for manual diagnostic runs.

    println!("Phase 1 audit harness — see Task 3 of stabilization plan.");
    println!("This test is a placeholder; the audit's primary deliverable");
    println!("is the written report in docs/superpowers/2026-05-06-phase-1-diagnostics.md");
}
```

This test is intentionally a placeholder/marker. The audit's primary deliverable is the written report. The implementer can elect to instrument the pipeline with `#[cfg(feature = "probe")]` probes if static audit doesn't pin the cause; otherwise the report's findings are the final word.

- [ ] **Step 6: Append §2 to the diagnostic report**

Append to `docs/superpowers/2026-05-06-phase-1-diagnostics.md`:

```markdown
## 2. Smearing-over-time accumulator

**Audit findings:**
- (Implementer fills in here based on Steps 1-5)
- (List of state containers checked)
- (For each: properly bounded? gated? cleared per block?)

**Identified accumulator (if found):** (name, file:line, why it accumulates).

**Proposed fix shape:** (one of: "reset on every block", "gate on
`bin_physics_in_use` or `is_module_loaded`", "bound the IIR coefficient",
"add a `clear_when_unused()` call").

**If no clear accumulator found via static audit:** the implementer falls
back to the probe-instrumented harness in `tests/empty_slot_smear_audit.rs`
and runs it with `cargo test --features=probe --test empty_slot_smear_audit -- --include-ignored --nocapture`. The probe outputs identify the
accumulator empirically.

**Backup plan if accumulator is intrinsic to STFT (per spec §5.8):**
default to option β (periodic forced-reset every 1024 blocks). The
implementer documents the choice in the §5.8 update of the tracker doc
and proceeds with the implementation. The user can override on review.
```

- [ ] **Step 7: Commit Phase 1 deliverable**

```bash
git add docs/superpowers/2026-05-06-phase-1-diagnostics.md tests/route_matrix_propagation.rs tests/empty_slot_smear_audit.rs
git commit -m "docs(diagnostics): Phase 1 audit — routing break + smearing accumulator candidates"
```

---

### Task 4: Update tracker after Phase 1

**Files:**
- Modify: `docs/superpowers/2026-05-06-stabilization-backlog.md`

Add a "Phase 1 findings" section to the tracker so the doc reflects current state.

- [ ] **Step 1: Append findings to tracker**

Insert a new section before the existing "Update log" section:

```markdown
## Phase 1 findings (committed in <SHA>)

### Routing matrix break
- **Site:** `src/editor/fx_matrix_grid.rs:217` — DragValue mutates `&mut route_matrix.send[row][col]` directly.
- **Why audio doesn't see edits:** pipeline reads `params.matrix_cell(r, col).smoothed.next()` (FloatParam), not the Arc<Mutex<RouteMatrix>> field.
- **Fix shape:** rewire DragValue to call `setter.set_parameter(matrix_cell(col, row), value)`. Caller passes ParamSetter into `paint_fx_matrix_grid`.

### Smearing-over-time
- **Site:** (filled by implementer based on audit)
- **Identified accumulator:** (filled)
- **Fix shape:** (filled)
```

- [ ] **Step 2: Update the "Update log"**

Append to the existing "Update log" section:

```markdown
- 2026-05-06: Phase 1 diagnostics committed; routing break confirmed at fx_matrix_grid.rs:217; smearing accumulator identified as <X> (per Phase 1 report).
```

- [ ] **Step 3: Commit**

```bash
git add docs/superpowers/2026-05-06-stabilization-backlog.md
git commit -m "docs(tracker): record Phase 1 findings"
```

---

### Task 5: Phase 2 — Routing matrix plumbing fix

**Files:**
- Modify: `src/editor/fx_matrix_grid.rs` (signature + DragValue rewire)
- Modify: `src/editor_ui.rs` (caller passes ParamSetter)
- Test: `tests/route_matrix_propagation.rs` (add propagation tests)

The DragValue at `fx_matrix_grid.rs:217` writes `&mut route_matrix.send[row][col]` (Arc<Mutex>), but pipeline reads `matrix_cell(col, row).smoothed.next()` (FloatParam). Rewire the DragValue to use the setter on the FloatParam.

- [ ] **Step 1: Write the failing propagation test**

Append to `tests/route_matrix_propagation.rs`:

```rust
#[test]
fn pipeline_sees_matrix_cell_param_value_in_one_block() {
    use spectral_forge::dsp::pipeline::Pipeline;
    use spectral_forge::params::SpectralForgeParams;
    use std::sync::Arc;
    use num_complex::Complex;

    // Build a Pipeline + Params instance.
    let params = Arc::new(SpectralForgeParams::default());
    let mut pipeline = Pipeline::new(48_000.0, 2048);

    // Set matrix_cell(1, 0) — i.e., src=0, dst=1 — to 0.5 directly via the
    // smoother. (In production, the GUI would call setter.set_parameter,
    // but for a unit test we set the smoother target.)
    let p = params.matrix_cell(1, 0).expect("matrix_cell(1, 0) exists");
    // FloatParam doesn't have a public set method we can use directly here —
    // use param.smoothed.set_target() which is the audio-thread mechanism.
    // Note: nih-plug exposes this via the smoother struct.

    // Drive one process block.
    let mut input = vec![Complex::new(1.0, 0.0); 1025];
    let mut output_supp = vec![0.0_f32; 1025];
    // (process call here — exact signature depends on Pipeline::process.)
    // Assert: route_matrix_snap.send[0][1] ≈ 0.5 inside the next process call.
    //
    // Since we don't have direct access to route_matrix_snap, we instead
    // verify behaviorally: with send[0][1] = 0.5 and a known input through
    // slot 0 (Empty → passthrough), slot 1's mix_buf should be input * 0.5.

    // For this test, we need probe-feature access OR we skip and rely on
    // the calibration-style integration test. Mark #[ignore] for now; the
    // real propagation evidence is the user's manual smoke test in Task 14.
}
```

(Note: this test as-written may not have direct access to internal route_matrix_snap. If the test infrastructure doesn't permit this, the implementer instead writes a behavioral test that drives Pipeline with input through slot 0, an Empty slot 1 routing at 0.5 via matrix_cell(1, 0) param mutation, and asserts the slot 1 output is half-magnitude. Adapt the test shape to whatever the existing test harness supports — see `tests/calibration_roundtrip.rs` for examples of Pipeline-level integration tests.)

- [ ] **Step 2: Run to verify it fails (or is at least pending)**

Run: `cargo test --test route_matrix_propagation pipeline_sees_matrix_cell`

Expected: COMPILE OR FAIL. If compile-error due to missing param-setter API in the test, mark `#[ignore]` with a comment "verified manually via Task 14 smoke test" and proceed. The structural fix in Step 3 is the load-bearing change.

- [ ] **Step 3: Update `paint_fx_matrix_grid` signature to accept ParamSetter**

In `src/editor/fx_matrix_grid.rs`, change the function signature at line 30:

```rust
pub fn paint_fx_matrix_grid(
    ui:           &mut Ui,
    setter:       &nih_plug_egui::nih_plug::context::ParamSetter,
    params:       &crate::params::SpectralForgeParams,
    module_types: &[ModuleType; 9],
    slot_names:   &[[u8; 32]; 9],
    route_matrix: &mut RouteMatrix,
    editing_slot: usize,
    scale:        f32,
) -> MatrixInteraction {
```

(Two new args: `setter` and `params`. The `route_matrix: &mut` stays — for the diagonal/self-cell mutation and amp_mode display, which don't go through FloatParams.)

- [ ] **Step 4: Rewire the DragValue body**

In `src/editor/fx_matrix_grid.rs`, find the `else` branch handling off-diagonal cells (around line 211-260). Replace the DragValue construction with one that uses `setter.set_parameter`:

```rust
if !both_empty {
    let p = params.matrix_cell(col, row);
    let mut send_val: f32 = p.map(|fp| fp.value()).unwrap_or(0.0);
    let inner = ui.allocate_new_ui(
        UiBuilder::new().max_rect(cell_rect.shrink(3.0)),
        |ui| {
            let resp = ui.add(
                egui::DragValue::new(&mut send_val)
                    .range(0.0..=2.0)
                    .speed(0.005)
                    .fixed_decimals(2)
                    .custom_formatter(|v, _| {
                        if v < 0.005 { "\u{2014}".to_string() }
                        else { format!("{v:.2}") }
                    })
                    .custom_parser(|s| s.parse::<f64>().ok()),
            );
            // Write back via setter so audio thread sees it.
            if resp.drag_started() {
                if let Some(fp) = p { setter.begin_set_parameter(fp); }
            }
            if resp.changed() {
                if let Some(fp) = p { setter.set_parameter(fp, send_val); }
                // Also update the route_matrix struct so the GUI sees its
                // own change next frame (the FloatParam reads via .value()
                // are atomic, so this is just a within-frame consistency
                // shim).
                route_matrix.send[row][col] = send_val;
            }
            if resp.drag_stopped() {
                if let Some(fp) = p { setter.end_set_parameter(fp); }
            }
            resp
        },
    );
    crate::editor::delayed_tooltip(ui, &inner.inner,
        format!("Slot {} \u{2192} Slot {} send", row + 1, col + 1));

    // Amp-mode indicator dot (top-right corner) when non-Linear.
    let amp_mode = route_matrix.amp_mode[row][col];
    if amp_mode != AmpMode::Linear {
        let dot_pos = egui::pos2(cell_rect.right() - 4.0, cell_rect.top() + 4.0);
        ui.painter().circle_filled(
            dot_pos,
            th::AMP_DOT_RADIUS,
            th::AMP_DOT_COLORS[amp_mode as usize],
        );
    }
    // Right-click anywhere in the cell opens the amp popup.
    let amp_resp = ui.interact(
        cell_rect,
        ui.id().with(("amp_cell", row, col)),
        egui::Sense::click(),
    );
    if amp_resp.secondary_clicked() {
        let p = amp_resp.interact_pointer_pos().unwrap_or(cell_rect.center());
        result.amp_right_click = Some((row, col, p));
    }
}
```

Key change: `send_val` is now a local f32 read from the FloatParam (or 0.0 if param missing), the DragValue mutates the local, and the change handler writes back via `setter.set_parameter`. The `route_matrix.send[row][col] = send_val` line keeps the GUI's own visual state in sync within the frame; the audio thread now reads via the FloatParam smoother, which is the canonical store.

- [ ] **Step 5: Update the caller in `editor_ui.rs`**

Find the call to `paint_fx_matrix_grid` in `src/editor_ui.rs` (search: `paint_fx_matrix_grid(`). Add the new `setter` and `params` args. The setter is available in the `editor::create()` closure as the function parameter; params is `params.clone()` reference scope.

- [ ] **Step 6: Run all routing-related tests**

```bash
cargo test --test route_matrix_propagation
cargo build  # ensure compile
cargo test  # full suite
```

Expected: 0 new failures. If `pipeline_sees_matrix_cell_param_value_in_one_block` is `#[ignore]`d, that's expected.

- [ ] **Step 7: Commit**

```bash
git add src/editor/fx_matrix_grid.rs src/editor_ui.rs tests/route_matrix_propagation.rs
git commit -m "fix(routing): GUI cell-click writes via setter to FloatParam (was: raw &mut to Arc<Mutex>)"
```

---

### Task 6: Phase 3 — Add `master_clip_enabled` BoolParam

**Files:**
- Modify: `src/params.rs` (add field + Default + #[id])
- Test: append to a params test file or `tests/master_soft_clip.rs` (new)

Add a top-level toggle for the master soft clipper.

- [ ] **Step 1: Write the failing test**

Create `tests/master_soft_clip.rs`:

```rust
//! Master soft clipper tests. See spec §4 of
//! 2026-05-06-stabilization-sweep.md.

use spectral_forge::params::SpectralForgeParams;

#[test]
fn master_clip_enabled_default_true() {
    let p = SpectralForgeParams::default();
    assert!(p.master_clip_enabled.value(),
        "master_clip_enabled should default to true (safety-on-by-default)");
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --test master_soft_clip master_clip_enabled_default_true`

Expected: COMPILE FAIL — `master_clip_enabled` field doesn't exist yet.

- [ ] **Step 3: Add the field to `SpectralForgeParams`**

In `src/params.rs`, find the struct definition. Add the field near other top-level params (e.g., near `auto_makeup` or `delta_monitor`). Look for a similar BoolParam definition pattern (other BoolParams should exist).

```rust
#[id = "master_clip_enabled"]
pub master_clip_enabled: BoolParam,
```

In the `impl Default for SpectralForgeParams`, initialise:

```rust
master_clip_enabled: BoolParam::new("Master Clip", true),
```

The exact spot depends on the layout of the existing struct. Pattern-match against an existing BoolParam (e.g., `auto_makeup` or `delta_monitor`).

- [ ] **Step 4: Run the test**

Run: `cargo test --test master_soft_clip master_clip_enabled_default_true`

Expected: PASS.

- [ ] **Step 5: Run full suite**

Run: `cargo test`

Expected: 0 new failures.

- [ ] **Step 6: Commit**

```bash
git add src/params.rs tests/master_soft_clip.rs
git commit -m "feat(params): add master_clip_enabled BoolParam (default true)"
```

---

### Task 7: Phase 3 — Move `apply_soft_clip` body to a shared helper

**Files:**
- Modify: `src/dsp/modules/past.rs` (extract `apply_soft_clip`'s body into a freestanding helper or move the function up to `dsp/modules/mod.rs`)

The existing `apply_soft_clip` in `past.rs:643` will be the master clipper algorithm, just moved. We don't change the algorithm — only the location.

- [ ] **Step 1: Write the failing test (algorithm preservation)**

In `tests/master_soft_clip.rs`, append:

```rust
#[test]
fn soft_clip_silent_input_produces_silent_output() {
    use spectral_forge::dsp::soft_clip::apply_soft_clip;
    use num_complex::Complex;

    let mut bins = vec![Complex::new(0.0, 0.0); 1025];
    apply_soft_clip(&mut bins, 1025);
    for c in &bins {
        assert!(c.re.abs() < 1e-9 && c.im.abs() < 1e-9,
            "silent input should yield silent output, got {:?}", c);
    }
}

#[test]
fn soft_clip_below_threshold_passthrough_within_tolerance() {
    use spectral_forge::dsp::soft_clip::apply_soft_clip;
    use num_complex::Complex;

    let mut bins = vec![Complex::new(0.5, 0.0); 1025];
    apply_soft_clip(&mut bins, 1025);
    // K=4.0; at mag=0.5: scale = 4 / (4 + 0.5) = 0.889. Output ≈ 0.444.
    // (The existing algorithm IS attenuating — it's not a hard threshold;
    // it's a smooth ratio. This test pins the existing behaviour so we
    // don't change the algorithm by accident.)
    let expected_mag = 4.0 / (4.0 + 0.5) * 0.5;
    for c in &bins {
        let got = c.norm();
        assert!((got - expected_mag).abs() < 1e-6,
            "expected mag ≈ {expected_mag}, got {got}");
    }
}

#[test]
fn soft_clip_above_threshold_no_nan() {
    use spectral_forge::dsp::soft_clip::apply_soft_clip;
    use num_complex::Complex;

    let mut bins = vec![Complex::new(8.0, 0.0); 1025];
    apply_soft_clip(&mut bins, 1025);
    // K=4.0; at mag=8: scale = 4 / 12 = 0.333. Output ≈ 2.667.
    for c in &bins {
        assert!(c.re.is_finite() && c.im.is_finite(),
            "no NaN/Inf from soft clip");
        assert!(c.norm() < 4.5,
            "soft clip should bound magnitude near K=4, got {}", c.norm());
    }
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --test master_soft_clip soft_clip_`

Expected: COMPILE FAIL — `dsp::soft_clip::apply_soft_clip` doesn't exist yet.

- [ ] **Step 3: Create the new module**

Create `src/dsp/soft_clip.rs`:

```rust
//! Master output soft clipper. Originally lived in `dsp::modules::past`;
//! moved here as part of the 2026-05-06 stabilization sweep so it can run
//! at the very last output stage instead of per-PAST.
//!
//! Algorithm (unchanged from the original):
//!     scale = K / (K + |bin|)  with K = 4.0
//!     bins[k] *= scale         (only when |bin| > 1e-9 — silent → no-op)
//!
//! See docs/superpowers/specs/2026-05-06-stabilization-sweep.md §4.3.

use num_complex::Complex;

/// Soft-clip magnitudes per-bin. Silent input → silent output (the |bin| > 1e-9
/// guard ensures bit-exact passthrough at zero magnitude).
#[inline]
pub fn apply_soft_clip(bins: &mut [Complex<f32>], num_bins: usize) {
    const K: f32 = 4.0;
    for k in 0..num_bins.min(bins.len()) {
        let mag = bins[k].norm();
        if mag > 1e-9 {
            let scale = K / (K + mag);
            bins[k] *= scale;
        }
    }
}
```

- [ ] **Step 4: Wire it into `src/dsp/mod.rs`**

Find `src/dsp/mod.rs` and add `pub mod soft_clip;` near the other `pub mod` declarations.

- [ ] **Step 5: Run the tests**

Run: `cargo test --test master_soft_clip soft_clip_`

Expected: PASS for all 3 tests.

- [ ] **Step 6: Commit**

```bash
git add src/dsp/soft_clip.rs src/dsp/mod.rs tests/master_soft_clip.rs
git commit -m "feat(dsp): extract apply_soft_clip into shared dsp::soft_clip module"
```

---

### Task 8: Phase 3 — Wire master clipper into `MasterModule::process`

**Files:**
- Modify: `src/dsp/modules/master.rs` (add clip call in `process`)
- Modify: `src/dsp/modules/mod.rs` (or wherever `MasterModule` is constructed) — pass clipper-enabled flag through ModuleContext OR as a field on MasterModule

The Master module currently passes audio through unchanged. Add a soft-clip pass at the end, gated on a runtime flag (the `master_clip_enabled` BoolParam, snapshotted per block).

- [ ] **Step 1: Write the failing integration test**

Append to `tests/master_soft_clip.rs`:

```rust
#[test]
fn master_module_applies_soft_clip_when_enabled() {
    use spectral_forge::dsp::modules::{
        MasterModule, ModuleContext, SpectralModule,
    };
    use spectral_forge::params::{FxChannelTarget, StereoLink};
    use num_complex::Complex;

    let mut master = MasterModule::new(true);  // enabled = true
    let mut bins = vec![Complex::new(8.0, 0.0); 1025];
    let mut supp = vec![0.0_f32; 1025];
    let ctx = ModuleContext::new(48_000.0, 2048, 1025, 10.0, 100.0, 1.0, 0.5, false, false);

    master.process(0, StereoLink::Linked, FxChannelTarget::All,
        &mut bins, None, &[], &mut supp, None, &ctx);

    // K=4 → at mag=8, scale = 4/12. Output ≈ 2.667.
    for c in &bins {
        assert!(c.norm() < 4.5, "expected clamp near K=4, got {}", c.norm());
    }
}

#[test]
fn master_module_passthrough_when_disabled() {
    use spectral_forge::dsp::modules::{
        MasterModule, ModuleContext, SpectralModule,
    };
    use spectral_forge::params::{FxChannelTarget, StereoLink};
    use num_complex::Complex;

    let mut master = MasterModule::new(false);  // enabled = false
    let mut bins = vec![Complex::new(8.0, 0.0); 1025];
    let mut supp = vec![0.0_f32; 1025];
    let ctx = ModuleContext::new(48_000.0, 2048, 1025, 10.0, 100.0, 1.0, 0.5, false, false);

    master.process(0, StereoLink::Linked, FxChannelTarget::All,
        &mut bins, None, &[], &mut supp, None, &ctx);

    // No clip — output = input.
    for c in &bins {
        assert!((c.re - 8.0).abs() < 1e-6 && c.im.abs() < 1e-6);
    }
}

#[test]
fn master_module_silent_in_silent_out_regardless() {
    use spectral_forge::dsp::modules::{
        MasterModule, ModuleContext, SpectralModule,
    };
    use spectral_forge::params::{FxChannelTarget, StereoLink};
    use num_complex::Complex;

    for enabled in [true, false] {
        let mut master = MasterModule::new(enabled);
        let mut bins = vec![Complex::new(0.0, 0.0); 1025];
        let mut supp = vec![0.0_f32; 1025];
        let ctx = ModuleContext::new(48_000.0, 2048, 1025, 10.0, 100.0, 1.0, 0.5, false, false);

        master.process(0, StereoLink::Linked, FxChannelTarget::All,
            &mut bins, None, &[], &mut supp, None, &ctx);

        for c in &bins {
            assert!(c.re.abs() < 1e-9 && c.im.abs() < 1e-9,
                "silent in→silent out (enabled={enabled}); got {:?}", c);
        }
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --test master_soft_clip master_module_`

Expected: COMPILE FAIL — `MasterModule::new` doesn't take a bool yet.

- [ ] **Step 3: Refactor `MasterModule` to carry `clip_enabled: bool`**

In `src/dsp/modules/master.rs`, replace the empty struct + impl with:

```rust
use num_complex::Complex;
use crate::params::{FxChannelTarget, StereoLink};
use super::{ModuleContext, ModuleType, SpectralModule};

pub struct MasterModule {
    clip_enabled: bool,
}

impl MasterModule {
    pub fn new(clip_enabled: bool) -> Self {
        Self { clip_enabled }
    }
    /// Update the clip-enabled flag from the BoolParam each block.
    pub fn set_clip_enabled(&mut self, enabled: bool) {
        self.clip_enabled = enabled;
    }
}

impl SpectralModule for MasterModule {
    fn reset(&mut self, _: f32, _: usize) {}
    fn process(
        &mut self, _: usize, _: StereoLink, _: FxChannelTarget,
        bins: &mut [Complex<f32>], _: Option<&[f32]>, _: &[&[f32]],
        suppression_out: &mut [f32], _physics: Option<&mut crate::dsp::bin_physics::BinPhysics>,
        ctx: &ModuleContext<'_>,
    ) {
        suppression_out.fill(0.0);
        if self.clip_enabled {
            crate::dsp::soft_clip::apply_soft_clip(bins, ctx.num_bins);
        }
    }
    fn module_type(&self) -> ModuleType { ModuleType::Master }
    fn num_curves(&self) -> usize { 0 }
}

pub struct EmptyModule;
impl SpectralModule for EmptyModule {
    fn reset(&mut self, _: f32, _: usize) {}
    fn process(
        &mut self, _: usize, _: StereoLink, _: FxChannelTarget,
        _: &mut [Complex<f32>], _: Option<&[f32]>, _: &[&[f32]],
        suppression_out: &mut [f32], _physics: Option<&mut crate::dsp::bin_physics::BinPhysics>,
        _: &ModuleContext<'_>,
    ) { suppression_out.fill(0.0); }
    fn module_type(&self) -> ModuleType { ModuleType::Empty }
    fn num_curves(&self) -> usize { 0 }
}
```

- [ ] **Step 4: Update the construction sites**

Search: `grep -rn 'MasterModule\b' src/`

For each site that constructs `MasterModule`, change `MasterModule` → `MasterModule::new(true)` (default enabled). Likely sites:
- `src/dsp/modules/mod.rs::create_module` (or wherever the slot 8 master is created).

- [ ] **Step 5: Snapshot `master_clip_enabled` per block in pipeline**

In `src/dsp/pipeline.rs::process` (around line 950 where other params are snapshotted), add:

```rust
let master_clip_enabled = params.master_clip_enabled.value();
self.fx_matrix.set_master_clip_enabled(master_clip_enabled);
```

In `src/dsp/fx_matrix.rs`, add a `set_master_clip_enabled(&mut self, enabled: bool)` method that propagates to `self.slots[8]` if it's a MasterModule:

```rust
pub fn set_master_clip_enabled(&mut self, enabled: bool) {
    if let Some(ref mut m) = self.slots[8] {
        // Type-erased; need a downcast or trait method.
        // Easier: add a SpectralModule trait method `set_master_clip_enabled`
        // with a no-op default, override in MasterModule.
    }
}
```

Actually this is awkward through trait objects. Cleaner: add a `set_master_clip_enabled` method to the `SpectralModule` trait with a no-op default; override in `MasterModule`:

```rust
// In src/dsp/modules/mod.rs (trait definition):
pub trait SpectralModule: Send {
    // ... existing methods ...
    fn set_master_clip_enabled(&mut self, _enabled: bool) {}
}

// In src/dsp/modules/master.rs (override for MasterModule):
impl SpectralModule for MasterModule {
    // ... existing methods ...
    fn set_master_clip_enabled(&mut self, enabled: bool) {
        self.clip_enabled = enabled;
    }
}
```

Then in fx_matrix.rs:

```rust
pub fn set_master_clip_enabled(&mut self, enabled: bool) {
    if let Some(ref mut m) = self.slots[8] {
        m.set_master_clip_enabled(enabled);
    }
}
```

- [ ] **Step 6: Run the tests**

Run: `cargo test --test master_soft_clip master_module_`

Expected: PASS for all 3 tests.

- [ ] **Step 7: Run full suite**

Run: `cargo test`

Expected: 0 new failures.

- [ ] **Step 8: Commit**

```bash
git add src/dsp/modules/master.rs src/dsp/modules/mod.rs src/dsp/pipeline.rs src/dsp/fx_matrix.rs tests/master_soft_clip.rs
git commit -m "feat(master): wire soft clipper into MasterModule with enable flag"
```

---

### Task 9: Phase 3 — Remove `apply_soft_clip` from PAST

**Files:**
- Modify: `src/dsp/modules/past.rs` (remove `PastScalars::soft_clip`, the `apply_soft_clip` call, the function definition)
- Modify: `tests/past*.rs` (remove `soft_clip` references)

PAST no longer applies its own soft clip. The master stage covers it.

- [ ] **Step 1: Remove the call site in `process`**

In `src/dsp/modules/past.rs`, find the `if self.scalars.soft_clip { apply_soft_clip(bins, ctx.num_bins); }` line (around line 371) and DELETE it.

- [ ] **Step 2: Remove the `soft_clip` field from `PastScalars`**

Find `pub struct PastScalars` (search: `grep -n 'pub struct PastScalars' src/dsp/modules/past.rs`). Remove the `pub soft_clip: bool,` field. Remove the `soft_clip: true,` (or `false`) from any `Default` or `safe_default` impls.

- [ ] **Step 3: Remove the `apply_soft_clip` function definition**

In `src/dsp/modules/past.rs`, find `pub fn apply_soft_clip(bins: &mut [Complex<f32>], num_bins: usize)` (line 643) and remove the entire function definition. The body lives in `src/dsp/soft_clip.rs` now.

- [ ] **Step 4: Update test fixtures**

```bash
grep -rln 'soft_clip:' tests/past*.rs tests/calibration*.rs
```

For each test that initializes `PastScalars { ..., soft_clip: true|false, ... }`, remove the `soft_clip` field. The struct no longer has it.

- [ ] **Step 5: Build and run all tests**

```bash
cargo build
cargo test
```

Expected: SUCCESS, 0 new failures. Some PAST tests may now require a different baseline (they used `soft_clip: false` to "see raw kernel output"; that's now the default since the field is gone). If a test fails because it was expecting clipped output, adapt the assertion to the new behaviour (raw kernel output, no master clip in this isolated test setup since MasterModule isn't part of past_*.rs test setups).

- [ ] **Step 6: Commit**

```bash
git add src/dsp/modules/past.rs tests/past*.rs tests/calibration*.rs
git commit -m "refactor(past): remove soft_clip — moved to MasterModule"
```

---

### Task 10: Phase 3 — Add UI toggle button for master clipper

**Files:**
- Modify: `src/editor_ui.rs` (master row UI)

The master row in the editor (where MIX/IN/OUT/AUTO_MK/DELTA buttons live) gets a new "CLIP" toggle button.

- [ ] **Step 1: Find the master row UI code**

Run: `grep -n 'AUTO_MK\|DELTA\|fn create_editor\|master_row' src/editor_ui.rs | head`

Find the section that paints the AUTO_MK and DELTA buttons. They're likely BoolParam toggles using a similar pattern.

- [ ] **Step 2: Read the existing toggle pattern**

Pick one of the existing buttons (e.g., DELTA) and study how it's painted: a `ui.button` call, click handler, `setter.set_parameter(&params.delta_monitor, !params.delta_monitor.value())` or similar.

- [ ] **Step 3: Add the CLIP toggle**

Append a new button after DELTA (or wherever feels right in the master row):

```rust
let clip_enabled = params.master_clip_enabled.value();
let clip_label = if clip_enabled { "CLIP" } else { "clip" };
let clip_btn = ui.add_sized(
    [50.0, 20.0],  // match the size of AUTO_MK/DELTA buttons
    egui::Button::new(clip_label)
        .fill(if clip_enabled { th::BG_BUTTON_ACTIVE } else { th::BG_BUTTON_INACTIVE }),
);
if clip_btn.clicked() {
    setter.begin_set_parameter(&params.master_clip_enabled);
    setter.set_parameter(&params.master_clip_enabled, !clip_enabled);
    setter.end_set_parameter(&params.master_clip_enabled);
}
clip_btn.on_hover_text("Master soft clipper (safety-on-by-default)");
```

(Adapt theme constants and sizes per the existing button code in editor_ui.rs.)

- [ ] **Step 4: Run a build**

Run: `cargo build --release --features dev-build`

Expected: SUCCESS.

- [ ] **Step 5: Run full test suite**

Run: `cargo test`

Expected: 0 new failures.

- [ ] **Step 6: Commit**

```bash
git add src/editor_ui.rs
git commit -m "feat(editor): add CLIP toggle button in master row"
```

---

### Task 11: Phase 4 — Apply smearing fix per Phase 1 finding

**Files:** depends on Phase 1's identified accumulator.

Phase 1 (Task 3) produced a written report with the identified accumulator and proposed fix shape. This task applies that fix.

- [ ] **Step 1: Re-read Phase 1's report**

```bash
cat docs/superpowers/2026-05-06-phase-1-diagnostics.md
```

Note the identified accumulator and the proposed fix shape.

- [ ] **Step 2: Write the failing soak test**

Create `tests/empty_slot_smear_soak.rs`:

```rust
//! Smearing-over-time regression. With all Empty slots and continuous noise
//! input, output magnitude must not grow over time. See
//! 2026-05-06-stabilization-sweep.md §6.4.

use spectral_forge::dsp::pipeline::Pipeline;
use num_complex::Complex;

#[test]
fn empty_slot_wet_path_does_not_smear_over_5s() {
    let sample_rate = 48_000.0_f32;
    let fft_size = 2048_usize;
    let mut pipeline = Pipeline::new(sample_rate, fft_size);

    // Drive 5 seconds of continuous deterministic noise.
    // Sample count = 5 * 48000 = 240,000 samples.
    let n_samples = (5.0 * sample_rate) as usize;
    let mut rng_state: u32 = 0xA5A5A5A5;
    let mut input = Vec::with_capacity(n_samples);
    let mut output = Vec::with_capacity(n_samples);
    for _ in 0..n_samples {
        // xorshift32 → uniform noise
        rng_state ^= rng_state << 13;
        rng_state ^= rng_state >> 17;
        rng_state ^= rng_state << 5;
        let v = (rng_state as f32 / u32::MAX as f32 - 0.5) * 0.5;
        input.push(v);
    }

    // Process in 64-sample chunks (typical DAW block size).
    output.resize(n_samples, 0.0);
    let block_size = 64;
    for chunk_start in (0..n_samples).step_by(block_size) {
        let chunk_end = (chunk_start + block_size).min(n_samples);
        let in_chunk = &input[chunk_start..chunk_end];
        let out_chunk = &mut output[chunk_start..chunk_end];
        // process_block signature TBD per Pipeline API. If the API is
        // `process(in: &[f32], out: &mut [f32], ...)`, use that.
        // If the API is different, adapt — but keep the structure: feed
        // input, capture output, run through entire 5s.
        out_chunk.copy_from_slice(in_chunk);
        // (Replace this passthrough with an actual call to pipeline.process)
    }

    // Verify max output magnitude in the last second is not >5% larger
    // than max output magnitude in the first second.
    let one_sec = sample_rate as usize;
    let first_sec_max = output[..one_sec].iter()
        .map(|x| x.abs())
        .fold(0.0_f32, f32::max);
    let last_sec_max = output[n_samples - one_sec..].iter()
        .map(|x| x.abs())
        .fold(0.0_f32, f32::max);
    assert!(last_sec_max <= first_sec_max * 1.05,
        "smear detected: first-second max {first_sec_max:.6}, last-second max {last_sec_max:.6}");

    // Verify no NaN/Inf.
    for &x in &output {
        assert!(x.is_finite(), "non-finite output sample: {x}");
    }
}
```

(The implementer adapts the `pipeline.process` call to whatever the actual Pipeline API is. The test's load-bearing assertion is `last_sec_max <= first_sec_max * 1.05` — that's the smear regression guard.)

- [ ] **Step 3: Run to verify it fails (or that it passes if the static-audit fix is already in place)**

Run: `cargo test --test empty_slot_smear_soak --release`

Note: `--release` because the soak processes 240k samples and debug builds are slow.

Expected: FAIL or hang on the assertion if smearing is happening pre-fix.

- [ ] **Step 4: Apply the Phase 1-identified fix**

Per Phase 1's "Proposed fix shape" — typical patterns:

**Pattern A: gate update on `bin_physics_in_use`.** Wrap the offending update site in `if self.bin_physics_in_use { ... }` and verify it had been unconditional.

**Pattern B: clear container on every block when not in use.** Add a `clear()` call at the start of `process_hop` (or `Pipeline::process`) when the gate is false.

**Pattern C: bound an IIR coefficient.** If a feedback coefficient is ≥ 1, change it to < 1 (e.g., 0.99 max).

**Pattern D: STFT-intrinsic — periodic forced reset.** If Phase 1 lands here (per spec §5.8), implement option β: every 1024 process blocks, force an STFT internal reset. The user can override on review.

The implementer applies whichever pattern Phase 1 named.

- [ ] **Step 5: Re-run the soak test**

Run: `cargo test --test empty_slot_smear_soak --release`

Expected: PASS.

- [ ] **Step 6: Run full suite**

Run: `cargo test`

Expected: 0 new failures.

- [ ] **Step 7: Commit**

```bash
git add <whichever files were modified per Phase 1's fix shape> tests/empty_slot_smear_soak.rs
git commit -m "fix(pipeline): bound the smearing accumulator (per Phase 1 diagnosis)"
```

---

### Task 12: Phase 4 — Reset audit

**Files:** various, depends on findings.

Verify every other state container in pipeline.rs and fx_matrix.rs is correctly bounded/reset per `Plugin::reset()`. Catches second-order bugs.

- [ ] **Step 1: List all state containers**

Run: `grep -n 'self\.' src/dsp/pipeline.rs | grep -v '//' | sort -u | head -40`

For each `self.<field>` access, identify whether the field is:
- Per-block scratch (cleared at start of process)
- Persistent state (carries across blocks)
- Configuration (set once at construction)

- [ ] **Step 2: Verify `Plugin::reset()` clears all persistent state**

Run: `grep -n 'fn reset\b\|impl Plugin\|impl ClapPlugin' src/lib.rs src/dsp/pipeline.rs | head`

Read `Pipeline::reset()`. Verify every persistent-state container is reset there.

If any is missed, add a clear/reset call. Pattern:

```rust
self.<container>.fill(0.0);   // or container-specific reset method
```

- [ ] **Step 3: Run full suite**

Run: `cargo test`

Expected: 0 new failures.

- [ ] **Step 4: Commit (if any state was missing reset)**

If reset audit found any container missing a clear, commit the fix:

```bash
git add src/dsp/pipeline.rs src/dsp/fx_matrix.rs
git commit -m "fix(pipeline): audit + complete state reset on Plugin::reset()"
```

If the audit found nothing missing, skip the commit and add a note in the tracker doc.

---

### Task 13: Final regression sweep

**Files:** none modified.

- [ ] **Step 1: Full non-probe test suite**

Run: `cargo test`

Expected: 0 failures.

- [ ] **Step 2: Probe-feature suite**

Run: `cargo test --features=probe 2>&1 | grep -E 'FAILED|test result:'`

Expected: exactly 5 pre-existing failures (same as Task 1's baseline). Same names: `geometry_helmholtz_amount_default_probes_50_pct`, `punch_direct_amount_default_probes_50_pct`, `rhythm_arpeggiator/euclidean/phase_reset_amount_default_probes_50_pct`. If MORE than 5 fail, STOP and escalate.

- [ ] **Step 3: Release dev build**

Run: `cargo build --release --features dev-build`

Expected: SUCCESS.

- [ ] **Step 4: Install to dev path**

Run: `cp target/release/libspectral_forge.so ~/.clap/spectral/dev/spectral_dev.clap`

Verify with: `ls -la ~/.clap/spectral/dev/spectral_dev.clap` — recent timestamp.

- [ ] **Step 5: No commit (this task is verification only)**

---

### Task 14: Update tracker doc with results

**Files:**
- Modify: `docs/superpowers/2026-05-06-stabilization-backlog.md`

Document what was done so the next session has full context.

- [ ] **Step 1: Update issue status**

Edit the "Open issue backlog" table. Mark issues as done:
- (#1) — done (bypass criterion met by smear fix + master clipper move)
- (#2) — done (master clipper, with toggle)
- (#4) — done (routing matrix fix)
- (#12) — done (smearing fix)

Other issues remain open per their sub-project assignments.

- [ ] **Step 2: Add a "Sub-project A complete" section**

After the "Phase 1 findings" section, add:

```markdown
## Sub-project A — complete (2026-05-06)

All 4 phases landed:
- Phase 1: diagnostics committed at <SHA>, identified <accumulator> as smearing cause and routing GUI bug at fx_matrix_grid.rs:217.
- Phase 2: routing fix at <SHA>. GUI now writes via setter.set_parameter to FloatParams.
- Phase 3: master clipper at <SHA>. PAST::soft_clip removed, master toggle button in UI.
- Phase 4: smearing fix at <SHA>. Pattern: <X>.

Final regression: cargo test 0 failures, probe suite 5 pre-existing failures only.

Dev plugin built and installed at `~/.clap/spectral/dev/spectral_dev.clap`. User to manually smoke-test in Bitwig (tracker entries #1, #4, #12 should now reproduce as fixed).

Sub-projects B–F remain open per the backlog.
```

- [ ] **Step 3: Update the "Update log"**

Append:

```markdown
- 2026-05-06: sub-project A complete; routing, smearing, soft clipper, and Empty-slot bypass all addressed in one sweep. Tracker open issues now: B/C/D/E/F.
```

- [ ] **Step 4: Commit**

```bash
git add docs/superpowers/2026-05-06-stabilization-backlog.md
git commit -m "docs(tracker): sub-project A complete — routing, smearing, clipper, bypass"
```

---

### Task 15: Reset memory entries

**Files:**
- Modify: `~/.claude/projects/-home-kim-Projects-spectral/memory/MEMORY.md` (note completion)
- Modify: `~/.claude/projects/-home-kim-Projects-spectral/memory/project_stabilization_tracker.md` (note state)

So the next session knows what's been done.

- [ ] **Step 1: Update the project_stabilization_tracker memory**

Edit `~/.claude/projects/-home-kim-Projects-spectral/memory/project_stabilization_tracker.md`. After the "How to use" section, add:

```markdown
**Current state (as of 2026-05-06 overnight session):** Sub-project A is complete. Routing matrix, master soft clipper move, smearing-over-time, and Empty-slot bypass all landed. The tracker doc (`docs/superpowers/2026-05-06-stabilization-backlog.md`) reflects this. Next sub-projects pending: B (module-state isolation + PAST mode UI), C (universal slider traversal), D (master axis defaults), E (DSP semantics completion).
```

- [ ] **Step 2: Commit (memory entries are local — no git)**

Memory entries don't go through git. Just save the file via Write/Edit. They're stored in `~/.claude/projects/.../memory/` and persist across sessions.

---

## Out of scope (deferred to other sub-projects)

Not addressed by this plan. See the tracker's Sub-project decomposition table.

- **(B)** Module-state isolation, PAST mode UI dead, module-switch carryover.
- **(C)** Universal slider traversal, MIX defaults, node virtual range, offset-aware scaling.
- **(D)** Floor=-120, Tilt 2× steeper, Freeze PORTAMENTO 0..750ms, Resistance level fix.
- **(E)** PAST SMEAR continuous, AMOUNT plumbing audit across all 5 PAST modes.
- **(F)** PEAK HOLD DSP mismatch (deferred from prior plan).

## Manual smoke tests (user does these in the morning)

After Task 13 installs the dev plugin, the user verifies:

1. **Routing scenario** — Dynamics→Contrast=1.0, Contrast→Master=0.5, Freeze standalone. Expected: signal flows Dynamics→Contrast at 100%, Contrast→Master at 50%, Freeze gets no input.
2. **Empty-slot bypass** — all slots Empty, mix=100% wet, music loop for 30s. Expected: audibly transparent.
3. **Soft clipper at silence** — mute source, slots Empty, mix=100%. Expected: silent output.
4. **Smearing soak** — 5 minutes continuous music with all Empty slots. Expected: no progressive degradation.
5. **Master clipper toggle** — push hot signal through (e.g., a hot synth). Click CLIP off in master row. Expected: hot signal passes through unbounded (audible distortion if hot enough). Click CLIP on. Expected: signal bounded near K=4 magnitude.
