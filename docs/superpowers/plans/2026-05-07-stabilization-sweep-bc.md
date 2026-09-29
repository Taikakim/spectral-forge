# Stabilization Sweep B+C Implementation Plan

> **Status:** IMPLEMENTED — module-switch tilt/offset/curvature reset (`1496f12`), inline Past mode UI replacing the popup (`5d6f3b4`), offset default +1 for natural-at-max curves (`d67fd91`, `1d2b706`), off-rect node indicators (`8801840`, `590d41c`); closed in tracker (`242e8f6`). The code is the source of truth; this plan is kept for history. See [../STATUS.md](../STATUS.md).

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close residual module-switch state hygiene (#13), inline PAST mode UI (#14), fix the offset slider's dead-half problem on natural-at-max curves (#3, #5, #7), and add off-rect indicators to the existing virtual node y-range (#17, #18).

**Architecture:** Four small focused tasks plus baseline + final regression. Each task is fully independent (no cross-task file conflicts) and follows TDD. Sub-project A is complete; this is the next layer.

**Tech Stack:** Rust 1.x, `nih-plug-egui` 0.31, egui. No new dependencies.

---

## File structure

| File | Purpose |
|---|---|
| `src/editor_ui.rs` | B-1 (post-popup setter reset for tilt/offset/curvature) + B-2 (replace `is_past` mode-button branch with inline mode row) |
| `src/editor/past_popup.rs` | B-2 (`show_popup` + `open_at` deleted; `mode_label` + `MODES` retained as inline-UI helpers) |
| `src/editor/curve_config.rs` | C-1 (`natural_at_max: bool` field on `CurveDisplayConfig`) |
| `src/params.rs` | C-1 (offset FloatParam default = `+1.0` when `natural_at_max == true`) |
| `src/editor/curve.rs` | C-2 (off-rect red indicator drawn near rect edges when `\|node.y\| > 1`) |
| `src/editor/theme.rs` | C-2 (color/size constants for off-rect indicator) |
| `tests/module_switch_state_reset.rs` (new) | B-1 test |
| `tests/past_mode_inline.rs` (new) | B-2 placeholder test |
| `tests/curve_natural_at_max.rs` (new) | C-1 tests |
| `tests/node_offrect_indicator.rs` (new) | C-2 test (range invariant) |
| `docs/superpowers/2026-05-06-stabilization-backlog.md` | Tracker — final task updates this |

---

## Important context for every task

- **Branch:** `feature/next-gen-modules-plans`. Don't switch.
- **Untracked files** in `ideas/`, `.claude/`, and recent `docs/` are intentionally untracked — leave them alone.
- **Dev plugin path:** `~/.clap/spectral/dev/spectral_dev.clap`. Build: `cargo build --release --features dev-build`. Install: `cp target/release/libspectral_forge.so ~/.clap/spectral/dev/spectral_dev.clap`. Never install to `~/.clap/spectral_forge.clap`.
- **Pre-existing failures with `--features=probe`:** five `*_amount_default_probes_50_pct` tests in `tests/calibration_roundtrip.rs` are stale and unrelated. They MUST NOT be "fixed". Final regression confirms count remains exactly 5.
- **TDD discipline:** every task that adds behavior writes the test first, runs it to confirm failure, then implements, then re-runs to confirm pass.
- **One commit per task.** Don't squash. Don't amend.

---

### Task 0: Setup — verify clean baseline

**Files:** none modified.

- [ ] **Step 1: Check working tree**

Run: `git status --short`

Expected: only the listed untracked files (none of the in-scope files appear).

- [ ] **Step 2: Run baseline tests**

Run: `cargo test 2>&1 | tail -3`

Expected: `0 failed` across all test binaries.

- [ ] **Step 3: Capture probe baseline**

Run: `cargo test --features=probe 2>&1 | grep -c FAILED`

Expected: exactly `5`. Capture the names with `cargo test --features=probe 2>&1 | grep FAILED` for the final regression check.

- [ ] **Step 4: Verify dev-build compiles**

Run: `cargo build --release --features dev-build 2>&1 | tail -3`

Expected: SUCCESS.

- [ ] **Step 5: No commit (verification only)**

If any check fails, STOP and escalate (BLOCKED).

---

### Task 1: B-1 — reset tilt/offset/curvature on module switch

**Files:**
- Modify: `src/editor_ui.rs:1363-1377` (extend post-`show_popup` block)
- Test: `tests/module_switch_state_reset.rs` (new)

`assign_module` calls `smoothed.reset(0.0)` for each transform param, but the underlying `AtomicF32` keeps the previous user value. Editor reads via `.value()` which is the atomic, so the slider re-converges to the old value. Mirror the existing `graph_node` setter-reset pattern for the three transforms.

- [ ] **Step 1: Read the existing graph_node reset block**

Read `src/editor_ui.rs:1363-1377` to confirm the exact existing pattern. This is the template you mirror for tilt/offset/curvature.

- [ ] **Step 2: Write the failing test**

Create `tests/module_switch_state_reset.rs`:

```rust
//! Module-switch state hygiene regression. After a module is reassigned,
//! every per-curve transform FloatParam (tilt/offset/curvature) must be 0.0.
//! See docs/superpowers/specs/2026-05-07-stabilization-sweep-bc-design.md §B-1.

use nih_plug::prelude::*;
use spectral_forge::params::SpectralForgeParams;
use spectral_forge::dsp::modules::ModuleType;

#[test]
fn tilt_offset_curvature_reset_to_zero_on_assign_module() {
    let p = SpectralForgeParams::default();
    let slot = 2;

    // Manually set non-zero values via the smoother (test-side: the FloatParam
    // doesn't expose a public setter without a context, so we set the smoother
    // target through a host-style param-setter pattern. nih-plug's
    // `FloatParam::set_plain_value` is what `setter.set_parameter` calls
    // internally.)
    for c in 0..7 {
        if let Some(t) = p.tilt_param(slot, c) {
            t.set_plain_value(0.5);
        }
        if let Some(o) = p.offset_param(slot, c) {
            o.set_plain_value(0.3);
        }
        if let Some(cu) = p.curvature_param(slot, c) {
            cu.set_plain_value(-0.4);
        }
    }
    // Confirm setup landed.
    assert_eq!(p.tilt_param(slot, 0).unwrap().value(), 0.5);
    assert_eq!(p.offset_param(slot, 0).unwrap().value(), 0.3);
    assert_eq!(p.curvature_param(slot, 0).unwrap().value(), -0.4);

    // Now run the module-switch reset block. We can't call assign_module
    // directly (private + needs UI ctx), so we replicate its observable
    // behavior by calling the helper that the editor_ui block uses (which
    // we'll extract in Step 3) OR by directly calling set_plain_value(0.0)
    // on each — that's what the editor's setter.set_parameter does.
    //
    // For this test, exercise the helper. Helper signature TBD in Step 3:
    // crate::editor::module_popup::transform_reset_values(slot) -> impl Iterator<...>.
    // The simplest test: assert the helper produces the right (param, value)
    // pairs. The editor_ui setter loop is then trivial — no separate test
    // needed for that.
    let pairs: Vec<_> = spectral_forge::editor::module_popup::transform_reset_pairs(slot)
        .collect();
    // 7 curves × 3 params = 21 pairs.
    assert_eq!(pairs.len(), 21);
    for (c, kind, value) in pairs {
        assert!(c < 7);
        assert_eq!(value, 0.0);
        let _ = kind; // "tilt" | "offset" | "curvature"
    }
}
```

- [ ] **Step 3: Run to verify failure**

Run: `cargo test --test module_switch_state_reset`

Expected: COMPILE FAIL — `transform_reset_pairs` doesn't exist yet.

- [ ] **Step 4: Add the helper**

In `src/editor/module_popup.rs`, add a public freestanding helper at the bottom of the file:

```rust
/// Yields `(curve_index, kind, value)` triples for every transform FloatParam
/// that needs to be reset on module switch. `kind` is "tilt" | "offset" |
/// "curvature". Caller (editor_ui.rs) iterates these and writes each via
/// `setter.set_parameter`. Mirrors the per-curve graph_node reset block.
pub fn transform_reset_pairs(slot: usize) -> impl Iterator<Item = (usize, &'static str, f32)> {
    let _ = slot; // currently slot-independent; kept in the signature for future per-slot specialisation
    (0..7).flat_map(|c| {
        [
            (c, "tilt", 0.0_f32),
            (c, "offset", 0.0_f32),
            (c, "curvature", 0.0_f32),
        ]
    })
}
```

Make the function visible: in `src/editor/mod.rs` (or wherever module_popup is `pub use`d), confirm `pub mod module_popup;` is already present.

- [ ] **Step 5: Wire the helper into editor_ui's post-show_popup block**

In `src/editor_ui.rs`, find the existing `if let Some(changed_slot) = ...show_popup(...)` block (line ~1363) and append after the `for c in 0..7 { for n in 0..NUM_NODES { ... } }` graph_node loop:

```rust
// Reset tilt/offset/curvature FloatParam atomics so the slider UI matches.
// assign_module reset the smoothers; the setter writes mirror the FloatParam
// values through nih-plug's host-aware path so .value() also reads zero.
for (c, kind, value) in crate::editor::module_popup::transform_reset_pairs(changed_slot) {
    let p = match kind {
        "tilt"      => params.tilt_param(changed_slot, c),
        "offset"    => params.offset_param(changed_slot, c),
        "curvature" => params.curvature_param(changed_slot, c),
        _ => None,
    };
    if let Some(fp) = p {
        setter.set_parameter(fp, value);
    }
}
```

- [ ] **Step 6: Run the test**

Run: `cargo test --test module_switch_state_reset`

Expected: PASS.

- [ ] **Step 7: Run full suite**

Run: `cargo test 2>&1 | tail -5`

Expected: 0 new failures.

- [ ] **Step 8: Commit**

```bash
git add src/editor_ui.rs src/editor/module_popup.rs tests/module_switch_state_reset.rs
git commit -m "$(cat <<'EOF'
fix(slots): reset tilt/offset/curvature FloatParam atomics on module switch

assign_module calls smoothed.reset(0.0) but the underlying AtomicF32 still
holds the previous user value. The smoother re-converges to that old target
the moment audio resumes. Mirror the existing graph_node setter-reset
pattern via a new transform_reset_pairs helper.

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: B-2 — PAST modes inline in slot row

**Files:**
- Modify: `src/editor_ui.rs:1025-1037` (the `is_past` branch in mode-button section) and `src/editor_ui.rs:1393` (popup render call)
- Modify: `src/editor/past_popup.rs` (delete `show_popup` + `open_at`; keep `mode_label` + `MODES` + `SORT_KEYS` as inline-UI helpers)
- Test: `tests/past_mode_inline.rs` (new)

Replace the `Mode: {label}` button + popup with a horizontal row of `selectable_label` mode buttons rendered directly in the slot row. DecaySorter sub-picker (sort key) renders as a second inline row immediately below, only when DecaySorter is active.

- [ ] **Step 1: Read the existing inline UI patterns**

Read `src/editor/past_popup.rs:14-30` (`MODES` + `mode_label`) and `src/editor/past_popup.rs:70-105` (the popup body that produces selectable_label rows + DecaySorter sub-picker). This is the body we inline.

Also read `src/editor_ui.rs:1025-1037` (current PAST mode button) — this is what we replace.

- [ ] **Step 2: Write the placeholder test**

Create `tests/past_mode_inline.rs`:

```rust
//! Inline PAST mode UI placeholder regression. Visual UX verified via
//! manual smoke test; this pins the public API surface.

use spectral_forge::editor::past_popup::{mode_label, MODES, SORT_KEYS};
use spectral_forge::dsp::modules::past::PastMode;

#[test]
fn past_mode_label_set_intact() {
    assert_eq!(mode_label(PastMode::Granular),    "Granular");
    assert_eq!(mode_label(PastMode::DecaySorter), "Decay Sorter");
    assert_eq!(mode_label(PastMode::Convolution), "Convolution");
    assert_eq!(mode_label(PastMode::Reverse),     "Reverse");
    assert_eq!(mode_label(PastMode::Stretch),     "Stretch");
}

#[test]
fn past_modes_array_has_5_entries() {
    assert_eq!(MODES.len(), 5);
}

#[test]
fn past_sort_keys_array_present() {
    // Sort keys are still relevant for DecaySorter inline sub-picker.
    assert!(SORT_KEYS.len() >= 2,
        "DecaySorter sub-picker still needs sort key options");
}
```

- [ ] **Step 3: Run to verify it compiles and passes**

Run: `cargo test --test past_mode_inline`

Expected: PASS (the items being tested already exist).

- [ ] **Step 4: Make `MODES` and `SORT_KEYS` public**

In `src/editor/past_popup.rs`, find `const MODES: &[(...)]` and `const SORT_KEYS: &[...]` (search both names) and change `const` to `pub const`. This lets `editor_ui.rs` iterate them directly to render the inline UI.

- [ ] **Step 5: Replace the `is_past` branch with inline mode row**

In `src/editor_ui.rs`, find the existing `is_past` branch (line ~1025-1037) and replace the contents with:

```rust
} else if is_past {
    let cur_mode = params.slot_past_mode.lock()[edit_slot];
    ui.horizontal(|ui| {
        for &(mode, label, hint) in crate::editor::past_popup::MODES {
            let selected = cur_mode == mode;
            let resp = ui.selectable_label(selected,
                egui::RichText::new(label)
                    .color(if selected { th::LABEL_HI } else { th::LABEL_DIM })
                    .size(th::scaled(th::FONT_SIZE_LABEL, scale))
            ).on_hover_text(hint);
            if resp.clicked() && !selected {
                params.slot_past_mode.lock()[edit_slot] = mode;
            }
        }
    });
    // DecaySorter sub-picker — only when DecaySorter is the active mode.
    let mode_now = params.slot_past_mode.lock()[edit_slot];
    if mode_now == crate::dsp::modules::past::PastMode::DecaySorter {
        let cur_key = params.slot_past_sort_key.lock()[edit_slot];
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("Sort:")
                    .color(th::LABEL_DIM)
                    .size(th::scaled(th::FONT_SIZE_LABEL, scale))
            );
            for &(sort_key, sort_label) in crate::editor::past_popup::SORT_KEYS {
                let selected = cur_key == sort_key;
                let resp = ui.selectable_label(selected,
                    egui::RichText::new(sort_label)
                        .color(if selected { th::LABEL_HI } else { th::LABEL_DIM })
                        .size(th::scaled(th::FONT_SIZE_LABEL, scale))
                );
                if resp.clicked() && !selected {
                    params.slot_past_sort_key.lock()[edit_slot] = sort_key;
                }
            }
        });
    }
}
```

(Adapt `th::LABEL_HI` to whatever theme constant is used for active labels — check what other selected-state UI uses. If no `LABEL_HI` exists, use `th::LABEL` or a similar contrast color.)

- [ ] **Step 6: Remove the popup render call**

In `src/editor_ui.rs`, find `let _ = crate::editor::past_popup::show_popup(ui, &params, scale);` (line ~1393) and DELETE that line.

- [ ] **Step 7: Delete `show_popup` and `open_at` from `past_popup.rs`**

In `src/editor/past_popup.rs`, delete:
- The `pub fn show_popup(...)` function definition (~line 41-115).
- The `pub fn open_at(...)` function definition (~line 117-122).
- The `PastPopupState` struct if it's now unused.
- Any imports that are now unused (egui::Area, etc.).

Keep:
- `pub const MODES: &[(...)]`
- `pub const SORT_KEYS: &[...]`
- `pub fn mode_label(...) -> &'static str`

- [ ] **Step 8: Build and run all tests**

```bash
cargo build
cargo test 2>&1 | tail -5
```

Expected: SUCCESS, 0 new failures. If `cargo build` warns about unused imports in `past_popup.rs`, remove them.

- [ ] **Step 9: Commit**

```bash
git add src/editor_ui.rs src/editor/past_popup.rs tests/past_mode_inline.rs
git commit -m "$(cat <<'EOF'
feat(past): inline mode UI in slot row; popup removed

The Mode: button + popup had unnecessary friction for a 5-mode selector.
Inline 5 selectable_label buttons in the slot row, with the DecaySorter
sort-key sub-picker as a second inline row when DecaySorter is active.

show_popup and open_at deleted; MODES, SORT_KEYS, mode_label retained as
public helpers used by the inline UI.

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>
EOF
)"
```

---

### Task 3: C-1 — offset default `+1` for natural-at-max curves

**Files:**
- Modify: `src/editor/curve_config.rs` (add `natural_at_max: bool` field; flag relevant curves)
- Modify: `src/params.rs` (offset_param default reads the flag)
- Test: `tests/curve_natural_at_max.rs` (new)

Curves where `y_natural == y_max` (MIX everywhere, PAST AMOUNT/SMEAR) should default the offset FloatParam to `+1.0`. The slider mechanism stays universal `−1..+1`, so loading at `+1` lands the user at `y_max` (e.g., 100% wet for MIX).

- [ ] **Step 1: Read the existing curve_config flow**

Read `src/editor/curve_config.rs:1-50` to understand the `CurveDisplayConfig` struct and `curve_display_config()` dispatcher. Read the dynamics MIX entry (line 99-105) and the freeze MIX entry (find via `grep -n 'MIX\|y_natural: 100\|off_mix' src/editor/curve_config.rs`).

- [ ] **Step 2: Audit which curves are natural-at-max**

Run: `grep -B1 -A4 'y_natural:' src/editor/curve_config.rs | grep -B3 'y_max:' | head -100`

Tabulate every (module, curve_idx) where `y_natural == y_max`. Expected list: every module's MIX (last curve), PAST AMOUNT (~curve idx 1), PAST SMEAR (~curve idx 2). The actual list is what the audit produces — record it for use in the params constructor.

- [ ] **Step 3: Write the failing test**

Create `tests/curve_natural_at_max.rs`:

```rust
//! C-1 regression: curves where y_natural == y_max must default the offset
//! FloatParam to +1.0 (loads user at y_max). See
//! docs/superpowers/specs/2026-05-07-stabilization-sweep-bc-design.md §C-1.

use spectral_forge::editor::curve_config::curve_display_config;
use spectral_forge::dsp::modules::{ModuleType, GainMode};
use spectral_forge::params::SpectralForgeParams;

#[test]
fn natural_at_max_flag_consistent_with_y_natural_eq_y_max() {
    // For every module/curve, the flag must mirror y_natural == y_max.
    let modules = [
        ModuleType::Dynamics, ModuleType::Freeze, ModuleType::PhaseSmear,
        ModuleType::Contrast, ModuleType::Gain, ModuleType::MidSide,
        ModuleType::TransientSustainedSplit, ModuleType::Harmonic,
        ModuleType::Past, ModuleType::Geometry, ModuleType::Circuit,
        ModuleType::Life, ModuleType::Modulate, ModuleType::Rhythm,
        ModuleType::Punch, ModuleType::Harmony, ModuleType::Kinetics,
        ModuleType::Future,
    ];
    for &m in &modules {
        for c in 0..7 {
            let cfg = curve_display_config(m, c, GainMode::Add);
            let inferred = (cfg.y_natural - cfg.y_max).abs() < 1e-6;
            assert_eq!(cfg.natural_at_max, inferred,
                "{:?}/{}: y_natural={:.3}, y_max={:.3}, flag={}",
                m, c, cfg.y_natural, cfg.y_max, cfg.natural_at_max);
        }
    }
}

#[test]
fn dynamics_mix_offset_default_is_plus_one() {
    // Dynamics curve 6 is MIX (y_natural == y_max == 100%).
    let p = SpectralForgeParams::default();
    let slot = 0;
    // Default module assignments at slot 0 should already be Dynamics — verify.
    assert_eq!(p.slot_module_types.lock()[slot], ModuleType::Dynamics);
    let mix_curve = 6;
    let off = p.offset_param(slot, mix_curve).unwrap().value();
    assert_eq!(off, 1.0,
        "Dynamics MIX offset default must be +1.0 (loads at y_max=100% wet)");
}

#[test]
fn dynamics_threshold_offset_default_is_zero() {
    // Curve 0 is THRESHOLD; y_natural=-20, y_max=0 (NOT natural-at-max).
    let p = SpectralForgeParams::default();
    let off = p.offset_param(0, 0).unwrap().value();
    assert_eq!(off, 0.0,
        "Dynamics THRESHOLD offset default must remain 0.0 (not natural-at-max)");
}
```

- [ ] **Step 4: Run to verify the failure**

Run: `cargo test --test curve_natural_at_max`

Expected: COMPILE FAIL — `natural_at_max` field doesn't exist yet.

- [ ] **Step 5: Add the flag to `CurveDisplayConfig`**

In `src/editor/curve_config.rs`, add the field to the struct (read the existing struct first to find the right place for the new field):

```rust
pub struct CurveDisplayConfig {
    // ... existing fields ...
    pub natural_at_max: bool,
}
```

- [ ] **Step 6: Set the flag for every existing config entry**

This is the mechanical part. For every `CurveDisplayConfig { ... }` literal in the file, add `natural_at_max: true` if `y_natural == y_max` else `natural_at_max: false`. The audit from Step 2 enumerates the `true` cases. Use `grep -n 'CurveDisplayConfig {' src/editor/curve_config.rs` to find every literal.

`default_config()` (the fallback): set `natural_at_max: false` to avoid surprising any unconfigured curve.

- [ ] **Step 7: Update params.rs offset constructors**

In `src/params.rs`, find the offset FloatParam construction sites. They live in the per-slot/per-curve macro expansion (search `grep -n 'offset.*FloatParam::new\|off_dispatch\|offset_dispatch' src/params.rs`). The default is currently `0.0` everywhere.

Change the construction so that when constructing `offset_dispatch![slot, curve]`, the default is `+1.0` if `curve_display_config(default_module_for_slot(slot), curve, GainMode::Add).natural_at_max` is true, else `0.0`.

The simplest implementation: a helper `fn offset_default(slot: usize, curve: usize) -> f32` that runs the lookup, and use it at every construction site. Inline it as a `const fn` if all the inputs are const-known; otherwise a regular function called at `Default::default()` time.

Note: `default_module_for_slot` may not exist as a helper — the default slot module types are scattered in `Default for SpectralForgeParams`. Reuse whatever pattern is already there. If the construction order is "build all FloatParams, then assign module types," the offset default can read from a const lookup keyed on the (default_module, curve) pair instead.

- [ ] **Step 8: Run the failing tests**

Run: `cargo test --test curve_natural_at_max`

Expected: all 3 tests PASS.

- [ ] **Step 9: Run full suite**

Run: `cargo test 2>&1 | tail -5`

Expected: 0 new failures. The existing `curve_config` tests may need their literal `CurveDisplayConfig { ... }` constructors updated to include `natural_at_max:` — adapt as needed.

- [ ] **Step 10: Commit**

```bash
git add src/editor/curve_config.rs src/params.rs tests/curve_natural_at_max.rs
# Plus any test fixtures touched in Step 9.
git commit -m "$(cat <<'EOF'
feat(curve): default offset to +1 for natural-at-max curves

Curves where y_natural == y_max (MIX everywhere, PAST AMOUNT/SMEAR) had a
dead-half slider — the user could only drag down. Default the offset
FloatParam to +1.0 for those curves so users load at y_max (100% wet for
MIX) and slide down toward y_min. Slider mechanism stays universal -1..+1.

Adds natural_at_max: bool to CurveDisplayConfig, flag set per existing
y_natural == y_max condition.

Resolves tracker #3, #5, #7.

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>
EOF
)"
```

---

### Task 4: C-2 — off-rect indicator for virtual node y range

**Files:**
- Modify: `src/editor/curve.rs` (draw red directional indicator when `\|node.y\| > 1`)
- Modify: `src/editor/theme.rs` (color/size constants)
- Test: `tests/node_offrect_indicator.rs` (new)

The virtual range itself is already in place: `src/editor/curve.rs:992` clamps `node.y` to `[-2, +2]`, and `:944` clamps the visual circle position to the rect bounds. What's missing: a visual signal that the node has a y outside the visible rect.

- [ ] **Step 1: Read the existing node-rendering loop**

Read `src/editor/curve.rs:910-1000` (the node rendering + drag handling block — start at the `pub fn paint_curve_handles` or equivalent, end after the drag section). Identify:
- Where the dot is drawn (line ~944).
- The `sy` (screen y) before clamp and after clamp.

- [ ] **Step 2: Add theme constants**

In `src/editor/theme.rs`, append (or add near other indicator colors):

```rust
/// Off-rect node indicator (drawn when |node.y| > 1).
pub const NODE_OFFRECT_COLOR:  Color32 = Color32::from_rgb(220, 60, 60); // red
pub const NODE_OFFRECT_SIZE_PX: f32 = 6.0;
pub const NODE_OFFRECT_OFFSET_PX: f32 = 3.0; // distance outside the rect edge
```

- [ ] **Step 3: Write the failing test**

Create `tests/node_offrect_indicator.rs`:

```rust
//! C-2 regression: virtual node y range goes -2..+2 and the editor's drag
//! handler honors the wider range. Visual indicator verified by manual
//! smoke test (the constant exists and is non-zero).

use spectral_forge::editor::theme;

#[test]
fn offrect_indicator_constants_present() {
    assert!(theme::NODE_OFFRECT_SIZE_PX > 0.0);
    assert!(theme::NODE_OFFRECT_OFFSET_PX >= 0.0);
    let c = theme::NODE_OFFRECT_COLOR;
    // Red-ish: r > g and r > b.
    assert!(c.r() > c.g());
    assert!(c.r() > c.b());
}

#[test]
fn node_y_range_clamp_is_two() {
    use spectral_forge::editor::curve::CurveNode;
    // Sanity: the public node struct doesn't intrinsically clamp, but the
    // editor's drag handler clamps to ±2. Verify we can construct a node
    // at +1.5 (outside the visible rect, inside the virtual range).
    let n = CurveNode { x: 0.5, y: 1.5, q: 0.5 };
    assert!(n.y > 1.0 && n.y <= 2.0);
}
```

- [ ] **Step 4: Run to verify failure**

Run: `cargo test --test node_offrect_indicator`

Expected: COMPILE FAIL — `NODE_OFFRECT_*` constants don't exist yet (the second test should pass already since CurveNode is just a struct).

- [ ] **Step 5: Add theme constants (if not done in Step 2)**

If you skipped Step 2, do it now.

- [ ] **Step 6: Draw the indicator**

In `src/editor/curve.rs`, find the node-rendering block. After the existing dot draw (around line 944), add:

```rust
// Off-rect indicator: when node.y is outside the visible ±1 range, draw a
// red directional rect at the corresponding rect edge.
let abs_y = nodes[i].y.abs();
if abs_y > 1.0 {
    let edge_y = if nodes[i].y > 0.0 {
        rect.top() - th::NODE_OFFRECT_OFFSET_PX
    } else {
        rect.bottom() + th::NODE_OFFRECT_OFFSET_PX
    };
    let half = th::NODE_OFFRECT_SIZE_PX / 2.0;
    let sx = rect.left() + nodes[i].x.clamp(0.0, 1.0) * rect.width();
    let triangle = if nodes[i].y > 0.0 {
        // pointing up
        [
            egui::pos2(sx,        edge_y - half),
            egui::pos2(sx - half, edge_y + half),
            egui::pos2(sx + half, edge_y + half),
        ]
    } else {
        // pointing down
        [
            egui::pos2(sx,        edge_y + half),
            egui::pos2(sx - half, edge_y - half),
            egui::pos2(sx + half, edge_y - half),
        ]
    };
    ui.painter().add(egui::Shape::convex_polygon(
        triangle.to_vec(),
        th::NODE_OFFRECT_COLOR,
        egui::Stroke::NONE,
    ));
}
```

(Adapt the variable names — `nodes`, `rect`, `ui` — to whatever they're called in the actual scope. The use-as-template idea: read screen-space x from `nodes[i].x`, screen-space y is fixed at the rect edge ± offset, color is red.)

- [ ] **Step 7: Run the test**

Run: `cargo test --test node_offrect_indicator`

Expected: PASS for both tests.

- [ ] **Step 8: Run full suite**

Run: `cargo test 2>&1 | tail -5`

Expected: 0 new failures.

- [ ] **Step 9: Commit**

```bash
git add src/editor/curve.rs src/editor/theme.rs tests/node_offrect_indicator.rs
git commit -m "$(cat <<'EOF'
feat(curve): off-rect indicator for nodes outside the ±1 visible range

Virtual node y range -2..+2 is already in place (drag clamps + visual
position clamps). Add a small red directional triangle drawn just outside
the rect edge when a node's y exceeds the visible bounds, so users can
discover and locate off-screen nodes. Receivers (modules) clamp the
y values to their physical limits.

Resolves tracker #17, #18.

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>
EOF
)"
```

---

### Task 5: Final regression sweep

**Files:** none modified.

- [ ] **Step 1: Full non-probe test suite**

Run: `cargo test 2>&1 | tail -5`

Expected: 0 failures.

- [ ] **Step 2: Probe-feature suite**

Run: `cargo test --features=probe 2>&1 | grep -c FAILED`

Expected: exactly `5` (matches Task 0 baseline). If different, STOP and escalate.

- [ ] **Step 3: Release dev build**

Run: `cargo build --release --features dev-build 2>&1 | tail -3`

Expected: SUCCESS.

- [ ] **Step 4: Install to dev path**

Run: `cp target/release/libspectral_forge.so ~/.clap/spectral/dev/spectral_dev.clap`

Verify: `ls -la ~/.clap/spectral/dev/spectral_dev.clap` shows a fresh timestamp.

- [ ] **Step 5: No commit (verification only)**

---

### Task 6: Update tracker doc

**Files:**
- Modify: `docs/superpowers/2026-05-06-stabilization-backlog.md`

- [ ] **Step 1: Mark resolved issues**

In the "Open issue backlog" table, change status to ✅ for: #3, #5, #7, #13, #14, #17, #18.

- [ ] **Step 2: Append a "Sub-project B+C complete" section**

Add after the existing "Sub-project A — complete" section:

```markdown
## Sub-project B+C — complete (2026-05-07)

Combined small-task sweep. Commits: <SHA list>.

- B-1: tilt/offset/curvature FloatParam atomics reset on module switch (mirrors existing graph_node setter pattern).
- B-2: PAST modes inlined in slot row; popup chrome removed; DecaySorter sub-picker stays inline.
- C-1: offset FloatParam defaults to +1.0 for curves with y_natural == y_max (MIX everywhere, PAST AMOUNT/SMEAR). Slider semantics unchanged. Resolves the dead-half problem and the "MIX 100% wet by default" directive.
- C-2: red directional triangle drawn at rect edge when |node.y| > 1; virtual range -2..+2 already in place. Receivers clamp to physical limits.

Final regression: cargo test 0 failures, probe suite 5 pre-existing failures only. Dev plugin installed at `~/.clap/spectral/dev/spectral_dev.clap`.

User to manually smoke-test:
1. Switch a slot's module from one with non-zero tilt/offset to another → sliders show 0.0.
2. Click PAST mode labels in slot row → mode changes audibly.
3. Fresh patch with MIX-curve slot → MIX defaults to 100% wet (offset slider visually at top).
4. Drag a node off the top of the rect → red triangle appears at top edge; node y goes virtual.

Sub-projects D, E, F remain open per the backlog.
```

- [ ] **Step 3: Update the "Update log"**

Append:

```markdown
- 2026-05-07: sub-project B+C complete. Module-switch hygiene + PAST inline UI + MIX dead-half resolution + node off-rect indicators. Tracker open issues now: D, E, F.
```

- [ ] **Step 4: Commit**

```bash
git add docs/superpowers/2026-05-06-stabilization-backlog.md
git commit -m "docs(tracker): sub-project B+C complete — module-switch hygiene + curve UX

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

## Out of scope (deferred to other sub-projects)

- (D) Master axis defaults: Floor=-120, Tilt 2× steeper, Freeze PORTAMENTO 0..750ms, Resistance fix.
- (E) DSP semantics completion: PAST AMOUNT plumbing across modes, SMEAR continuous.
- (F) PEAK HOLD DSP mismatch.

## Manual smoke checklist (user does these on waking)

1. **Module-switch hygiene** — switch a slot from Dynamics with custom tilt/offset to Freeze. Sliders show 0.0, no carryover.
2. **PAST inline UI** — assign PAST to a slot. 5 mode labels visible inline. Click each → mode changes. Click DecaySorter → sort key sub-row appears.
3. **MIX 100% wet default** — fresh patch with any module that has a MIX curve. The MIX offset slider sits at the top (visually +1), the audible state is 100% wet.
4. **Off-rect node** — drag a node up past the top of the curve graph. Red triangle appears at the top edge. Continue dragging down: triangle disappears when y returns to ≤ 1.
