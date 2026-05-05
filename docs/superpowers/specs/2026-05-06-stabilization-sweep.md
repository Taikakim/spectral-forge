# Spectral Forge — Stabilization Sweep (Sub-project A)

**Status:** SPEC (2026-05-06)

**Goal:** Fix four entangled audio-path bugs in the Spectral Forge plugin: routing matrix non-functional, soft clipper at wrong layer, all-modules-disabled wet path not transparent, and smearing-over-time accumulating across blocks even with no modules loaded.

**Context:** The user reported these issues after testing the actual current build (a deployment misdirection, since resolved, had the previous testing happening against pre-fix code). All four are in `pipeline.rs` and `fx_matrix.rs` territory and benefit from being fixed together. The wider 18-issue stabilization backlog is tracked at `docs/superpowers/2026-05-06-stabilization-backlog.md`; this spec is sub-project A.

The current calibration system from prior plan `2026-05-05-graph-display-correctness.md` is correct and complete (matrix test green); the bugs here are unrelated to that work and predate it.

---

## 1. Strategy and deliverables

The spec produces one design doc (this file) and one implementation plan with four phases plus a final regression sweep:

- **Phase 1** — Diagnostics-only investigation. Add probe tests under `#[cfg(feature = "probe")]`. Characterize routing matrix break and smearing-over-time root cause. Output: written reports, no code fixes.
- **Phase 2** — Routing matrix plumbing fix, shaped by Phase 1's report.
- **Phase 3** — Soft clipper architecture move (PAST → master output stage).
- **Phase 4** — Smearing-over-time fix, shape determined by Phase 1's diagnostic.

Empty-slot bypass semantics is decided in this spec (§1.1) and applied during Phase 2 or 3.

### 1.1 Empty-slot bypass decision

When all 9 slots are Empty (no modules loaded), the wet path runs STFT roundtrip with default Hann² overlap-add. We do NOT add a true-bypass mode that skips STFT entirely — Bitwig's plugin-bypass button already provides host-level bypass. The criterion: with all slots Empty and mix=100% wet, the output must be **audibly transparent** (no audible difference vs. mix=0% dry). Modulo STFT delay, which the host compensates.

The spec is met when the soak test in §6.4 shows no progressive smear and no NaN/Inf for 60 seconds of continuous noise input through Empty-slot wet.

---

## 2. Phase 1 — Diagnostics

Phase 1 produces two written reports. Output committed to `docs/superpowers/2026-05-06-phase-1-diagnostics.md` and the tracker doc updated.

### 2.1 Routing matrix break — trace the GUI→params→pipeline chain

Three places the chain could break, each with its own diagnostic:

- **GUI side** — the matrix-grid widget's click callback isn't writing to params. Test: load Bitwig, click a cell, read the corresponding param via host inspector. If the param value doesn't match what the cell shows, this is the break.
- **Snapshot side** — `pipeline::process` reads stale or default values into `route_matrix`. Test: write a unit test that mutates a routing param via `setter.set_parameter`, runs one block of `pipeline::process`, and asserts `route_matrix.send[][]` reflects the mutation. If the assertion fails, this is the break.
- **Override** — some hardcoded fallback overrides the snapshot. Test: at the top of `process_hop`, dump the full `route_matrix.send` array and inspect whether it ever reflects user edits when changes occur.

Phase 1 deliverable for routing: 1-paragraph report saying "the break is at <site>, fix shape is <X>".

### 2.2 Smearing-over-time — characterize and localize

Symptom: wet path with no modules loaded smears progressively over minutes; clears on plugin power-cycle.

Diagnostic recipe:

1. Audio source: continuous spectrally-rich content (pink noise or a music loop). FFT size 2048. All slots Empty. Mix=100% wet. Record 60s; spectrogram reveals the smear character (low-pass drift, phase-coherence loss, additive ringing, amplitude growth).
2. State-cleanup probes (under `#[cfg(feature = "probe")]`): per-block log of magnitudes for each candidate state container — `BinPhysics` buffers; history buffer offsets; STFT helper OLA accumulator; modulation ring states; `slot_curve_cache`; `prev_mags` in FxMatrix; any `Vec<f32>`/`Vec<Complex<f32>>` member in pipeline.rs reused across blocks.
3. Sub-binary search: zero out each candidate at block start and re-run the recipe. The container that, when neutralized, makes the smear stop is the culprit.
4. Phase 1 deliverable: (a) reproduction recipe, (b) identified accumulator, (c) why it accumulates (math + control flow), (d) proposed fix shape (e.g., reset on every block, gate on `is_module_loaded`, bound the IIR).

### 2.3 What Phase 1 does NOT do

No fixes. Read-only investigation + diagnostic instrumentation. Phase 1 is one or two implementation tasks producing written reports, not patches.

---

## 3. Phase 2 — Routing matrix plumbing fix

Three fix shapes mapping to the three break sites identified in Phase 1. The implementer applies whichever Phase 1 narrowed down.

### 3.1 If the break is GUI-side

In `src/editor/fx_matrix_grid.rs`, the cell-click callback must call `setter.begin_set_parameter(param_id)` → `setter.set_parameter(param_id, new_value)` → `setter.end_set_parameter(param_id)`. Common bugs: writes to local copy without setter; serializes the cell's display string instead of float; short-circuits when old==new but old is stale.

### 3.2 If the break is snapshot-side

At the top of `pipeline::process` (or wherever `route_matrix` is assembled per block), explicitly read each `send[src][dst]` from the corresponding param:

```rust
for src in 0..MAX_SLOTS {
    for dst in 0..MAX_SLOTS {
        if let Some(p) = params.route_send_param(src, dst) {
            route_matrix.send[src][dst] = p.smoothed.next_step(block_size);
        }
    }
}
```

If `route_send_param` doesn't exist or returns `None` for valid coordinates, that's the bug — params aren't fully wired.

### 3.3 If the break is an override

Find the override site (likely a "default-routing fallback" guard like `if all_zero(send) { send = SERIAL_DEFAULT }`) and remove it, OR gate it on a "first-time-init" flag instead of every block.

### 3.4 Common requirements regardless of break site

- **Self-cells (`send[s][s]`) are NOT routes.** They're module-loaded indicators per `editor/fx_matrix_grid.rs`. The fix must not write to or read from `send[s][s]` as a real route.
- **Master output (`send[src][8]`) must be reachable** for all 8 source slots.
- **Virtual rows (T/S split outputs) live at `send[MAX_SLOTS + v][dst]`.** The fix must not break virtual-row routing.
- **Matrix mutation is GUI-thread, audio reads block-by-block.** Use lock-free atomic per cell, or `try_lock` with no-fallback (prev value held over). Match existing pipeline.rs idiom.

### 3.5 Regression coverage

- `tests/route_matrix_propagation.rs` (new): mutate via setter, run one block, assert reflects the new value. Cover idx 0, idx 7→8, virtual row.
- `tests/route_matrix_zero_cell_blocks_signal.rs` (new): set `send[0][8]=0.0`, send signal through slot 0 (Empty), assert master input silent.
- `tests/route_matrix_50pct_attenuates.rs` (new): set `send[0][8]=0.5`, assert master input is half-magnitude.
- Manual smoke (final task): the user's screenshot recipe (Dynamics→Contrast=1.0, Contrast→Master=0.5, Freeze standalone) must produce expected behaviour.

---

## 4. Phase 3 — Soft clipper architecture move

Move soft clipper from PAST-internal to master output stage. Three concerns: where it lives, when it runs, how it integrates with bypass.

### 4.1 Remove from PAST

In `src/dsp/modules/past.rs`: delete `PastScalars::soft_clip` field, `apply_soft_clip` call at line 371, and `apply_soft_clip` function definition if no other consumer needs it. Audit `pipeline.rs::process` for any place that sets `soft_clip` on the per-slot snapshot — remove.

Update tests in `tests/past*.rs` that pass `soft_clip: true/false` in `PastScalars::safe_default()`.

### 4.2 Add a master output stage

Run after FxMatrix produces its final mix. Two placements:

- **(a) Inside the `Master` module's `process()`.** Master becomes a real DSP unit. Conceptually right (Master is your final stage).
- **(b) Unconditional final pass in `pipeline::process` after `fx_matrix.process_hop` returns.** Few lines, no module-level wiring.

Recommendation: **(a)**. Master is the right home and keeps DSP where it belongs.

### 4.3 Clipper algorithm

Keep the existing `apply_soft_clip` algorithm body — only the placement moves. The audible character must not change. Audit shows it's likely tanh-based or cubic-soft; preserve.

**Critical constraint** (from user issue #2 — clipper clamps even at silent input): the relocated algorithm must produce zero output from zero input. Unit test:

```rust
#[test]
fn soft_clipper_silent_input_produces_silent_output() {
    let mut bins = vec![Complex::new(0.0, 0.0); 1025];
    apply_master_soft_clip(&mut bins, 1025);
    for c in &bins {
        assert!(c.re.abs() < 1e-9 && c.im.abs() < 1e-9);
    }
}
```

If the existing algorithm fails this test (Phase 3 verifies), find why and fix — likely a NaN guard or noise floor adding energy at zero input. The fix is in scope here even though it counts as an "algorithm bug" technically; placement-only is the goal but a broken zero-input behaviour blocks the spec criterion.

### 4.4 Clipper toggle and threshold

**Decision: always on, fixed threshold, no UI toggle.** The clipper is a safety mechanism, not a creative effect. Threshold = 4.0 magnitude (12 dB above unity), almost always inactive — only catches runaway buffers (NaN, huge magnitudes from feedback bug).

A future task can add a UI toggle if the user requests one. Out of scope here.

### 4.5 Empty-slot bypass interaction

With threshold 4.0 and STFT-roundtrip artifacts staying under unity magnitude, the master clipper never engages on Empty-slot wet path. Bypass behaviour stays clean per §1.1.

### 4.6 Regression coverage

- Silent-in-silent-out test (above).
- Below-threshold passthrough: input magnitude 0.5, output magnitude 0.5 (within 1e-6).
- Above-threshold soft clamp: input magnitude 8.0, output bounded near threshold ceiling, no NaN.
- Existing PAST tests cleanup: removing `PastScalars::soft_clip` requires updating any test that sets `soft_clip: false` to opt out — those become unconditional.

---

## 5. Phase 4 — Smearing-over-time fix

Shape determined by Phase 1's report. Candidate fix shapes per the state container Phase 1 names:

### 5.1 If the accumulator is `BinPhysics`

`bin_physics_in_use` flag exists in `FxMatrix`. Verify it's `false` when no slot has `writes_bin_physics: true` AND no slot has any `needs_*` flag. When `false`, every BinPhysics block-level update should skip — including the per-slot `mix_phys.mix_from(...)` and the master accumulator at line 691. Phase 4 audits each `if self.bin_physics_in_use` branch and confirms the false-branch is truly inert. If state still leaks (containers keep last values across blocks even when conditional skips updates), add a `clear_when_unused()` call at the top of `process_hop` when `bin_physics_in_use` is false.

### 5.2 If the accumulator is the STFT helper's OLA buffer

The user reports plugin power-cycle clears the smear (instance state). `Plugin::reset()` is only called by the host on transport stop or load — if Bitwig doesn't call it on play-after-pause, StftHelper accumulates pre-pause audio. Force-reset internal buffer via `stft.set_block_size()` or equivalent. Lower likelihood — Hann² OLA is mathematically lossless block-to-block.

### 5.3 If the accumulator is `prev_mags` in FxMatrix

`prev_mags` (used for `mix_phys.velocity`) updates unconditionally at line 562. With no modules using physics, prev_mags shouldn't be updated. Wrap the update in `if self.bin_physics_in_use`. Most likely culprit if §5.1 doesn't pin it down.

### 5.4 If the accumulator is `slot_curve_cache`

Per-slot curve caches drift even when no module reads them. The read side reads previously-set values when no GUI update arrives. Force a clear-on-no-update.

### 5.5 If the accumulator is the modulation ring

`ring_states` snapshot scales `slot_curve_cache` with growing values across blocks. Audit Sync16/SH/Legato modes for unbounded multiplicative paths (e.g., phase counter without modulo). Reset on disable, gate on "any ring active." `pipeline.rs:505` already gates on `ring_snapshot.entry_count() > 0` — if smear happens with zero rings active, this isn't it.

### 5.6 If the accumulator is something else

Phase 1 may find a sixth candidate (cached FFT scratch, per-slot delay line, soft-clipper internal state once relocated). The plan structure handles it: the fix task says "apply the fix prescribed by Phase 1's report." Plan structure is shape-agnostic.

### 5.7 Phase 4 task structure

- **Task: fix the identified accumulator.** Code change scoped to the one site Phase 1 named, plus a unit test that exercises the recipe (continuous noise input → empty slots → 60s → output magnitude bounded).
- **Task: reset audit.** Audit every other state container in pipeline.rs and fx_matrix.rs, verify it's correctly bounded/reset per `Plugin::reset()`.

### 5.8 Open question kept on the tracker

If Phase 1 finds the smear is intrinsic to STFT roundtrip (overlap-add accumulating tiny error per block over minutes), two options:

- **(α) Accept as fundamental STFT limitation**, document, recommend host-bypass for true bypass.
- **(β) Periodically force-reset internal STFT state** every N blocks, accepting tiny click at reset.

Spec does not pre-decide. If Phase 1 lands here, it goes back to the user with the question. The tracker doc captures the open decision.

---

## 6. Testing strategy

Five tiers, automated to manual:

### 6.1 Phase 1 diagnostic artifacts (deliverables, not tests)

- Routing-break report: file:line evidence + fix-shape determination.
- Smearing-root-cause report: recipe + identified accumulator + math/control-flow + proposed fix shape.

Both committed as `docs/superpowers/2026-05-06-phase-1-diagnostics.md`. Tracker doc updated with "Phase 1 findings" section.

### 6.2 Routing matrix unit tests (Phase 2)

- `tests/route_matrix_propagation.rs` — mutation propagates in one block.
- `tests/route_matrix_zero_cell_blocks_signal.rs` — `send=0` blocks signal.
- `tests/route_matrix_50pct_attenuates.rs` — `send=0.5` halves magnitude.

### 6.3 Soft clipper unit tests (Phase 3)

- Silent in → silent out.
- Below-threshold passthrough.
- Above-threshold soft clamp.

### 6.4 Smearing soak test (Phase 4)

`tests/empty_slot_smear_soak.rs` — Build pipeline with all Empty slots, drive 60s pink noise (or 5s for CI / 60s nightly), capture output every 1s, assert no monotonic spectral-energy growth (output max-magnitude bounded by `input_max * 1.05`), no NaN/Inf.

### 6.5 Manual smoke tests (final task)

- Routing scenario (screenshot recipe): Dynamics→Contrast=1.0, Contrast→Master=0.5, Freeze standalone. Verify by ear and spectrum.
- Empty-slot bypass: all slots Empty, mix=100%, 30s music loop. Audibly transparent.
- Soft clipper at silence: mute source, slots Empty, mix=100%. Output silent.
- Smearing soak: 5 minutes continuous music with all Empty slots. No degradation.

Final task documents pass/fail per scenario.

---

## 7. Out of scope

These backlog items are explicitly deferred. Each remains open in the tracker doc after this sub-project lands.

### 7.1 Module-state isolation (Sub-project B)

Issues #13 (carryover) and #14 (PAST mode UI dead). In `src/editor/`, not the audio path.

### 7.2 Curve UX redesign (Sub-project C)

Issues #3, #5, #7, #17, #18 — universal slider traversal, MIX defaults, node virtual range, offset-aware scaling. Major UX change touching curve_config.rs and offset_fn architecture.

### 7.3 Master axis defaults (Sub-project D)

Issues #9, #10, #11, #16. Floor=-120, Tilt 2× steeper, Freeze PORTAMENTO 0..750ms. Config-only, future small plan.

### 7.4 DSP semantics completion (Sub-project E)

Issues #6 (PAST SMEAR continuous) and #15 (Resistance level fix). PAST AMOUNT plumbing audit.

### 7.5 Phase 1 STFT-intrinsic smear decision (§5.8)

If Phase 1 lands on STFT-intrinsic smear, the α-vs-β decision returns to the user as a question. This spec does not pre-decide.

### 7.6 Master clipper algorithm change

§4.3 keeps existing tanh/cubic. Replacing with a different soft-clip family is out of scope. Algorithm bug fixes (zero-in-zero-out) ARE in scope per §4.3.

### 7.7 Master clipper UI controls

Always on, fixed threshold per §4.4. Toggle/threshold knob is a future task.

---

## 8. Deliverables and file structure

| File | Purpose |
|---|---|
| `docs/superpowers/specs/2026-05-06-stabilization-sweep.md` | This design doc |
| `docs/superpowers/2026-05-06-stabilization-backlog.md` | Persistent tracker (already exists, updated as plan progresses) |
| `docs/superpowers/2026-05-06-phase-1-diagnostics.md` | Phase 1 written report (created in implementation) |
| `src/dsp/pipeline.rs` | Routing matrix snapshot fix; bin_physics_in_use audit; possibly soft clipper integration |
| `src/dsp/fx_matrix.rs` | Possibly the routing matrix mutation site or override removal; possibly prev_mags reset |
| `src/dsp/modules/master.rs` | Master soft clipper implementation (if §4.2 option (a) chosen) |
| `src/dsp/modules/past.rs` | Remove `PastScalars::soft_clip`, `apply_soft_clip` |
| `src/editor/fx_matrix_grid.rs` | Possibly cell-click callback fix |
| `src/params.rs` | Possibly route_send_param wiring fix |
| `tests/route_matrix_propagation.rs` (new) | Routing tests |
| `tests/master_soft_clip.rs` (new) | Soft clipper tests |
| `tests/empty_slot_smear_soak.rs` (new) | Smearing soak regression |
| `tests/past*.rs` | Cleanup of `soft_clip` references |
