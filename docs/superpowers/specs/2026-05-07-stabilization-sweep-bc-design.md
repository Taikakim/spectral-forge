# Stabilization Sweep B+C — Design

**Status:** IMPLEMENTED — delivered by plan [`2026-05-07-stabilization-sweep-bc.md`](../plans/2026-05-07-stabilization-sweep-bc.md) (`1496f12`, `5d6f3b4`, `d67fd91`, `1d2b706`, `8801840`, `590d41c`; closed in tracker `242e8f6`). The code is the source of truth. See [../STATUS.md](../STATUS.md).
**Date:** 2026-05-07
**Predecessor:** [2026-05-06 stabilization sweep A](../plans/2026-05-06-stabilization-sweep.md) — complete.

## Goal

Close the two remaining 🔴 critical issues (#13 module-switch carryover; #14 PAST mode UI dead) plus the related curve-UX cluster from sub-project C in one combined sweep. Scope is intentionally tight — this is "B + C-as-small-tasks", not the full curve UX redesign originally floated for C.

## Background

Sub-project A landed routing, master clipper, smearing, and bypass. After A and a follow-up UI pass (threshold WYSIWYG, matrix smoother, Phase Smear PLPV leak), the remaining critical issues are confined to module-switch state hygiene and a small set of curve-UX problems that share a common root: the offset slider's `−1..+1` range collapses to a dead half on curves whose natural rest sits at `y_max` (MIX, AMOUNT-style curves), and node-y is currently capped at `±1` so nodes get squeezed against the rect edges when the curve is offset toward an extreme.

## Items in scope

| # | Item | Source |
|---|---|---|
| B-1 | Tilt/offset/curvature FloatParam atomics reset on module switch | tracker #13 (residual) |
| B-2 | PAST mode UI inlined in slot row (popup removed) | tracker #14 |
| C-1 | Default offset = `+1` for MIX/AMOUNT-style curves | tracker #3, #5, #7 |
| C-2 | Node virtual range `−2..+2` with off-rect indicators | tracker #17, #18 + user direction |

## Architecture

Three concerns, three locations:

1. **Slot lifecycle** (`module_popup.rs` + `editor_ui.rs`) — when the user reassigns a slot's module, every per-slot/per-curve param the editor reads via `.value()` must be reset through `setter.set_parameter` so the FloatParam atomics match the new module's defaults. Currently `graph_node` is reset that way but tilt/offset/curvature are only `smoothed.reset(0.0)` — the atomic still holds the previous user value, which the smoother re-converges to.
2. **PAST inline UI** (`editor_ui.rs` slot row block) — the existing `Mode:` button → `past_popup::show_popup` flow is replaced with a horizontal row of selectable mode labels rendered directly in the slot row, parallel to how Kinetics/Life popups currently work but with the popup chrome removed. The DecaySorter sub-picker (sort key) stays as a secondary inline row, only visible when DecaySorter is the active mode.
3. **Curve UX** (`curve_config.rs`, `params.rs`, `editor/curve.rs`) — two coordinated changes:
   - Offset FloatParam default = `+1.0` for any curve flagged "natural-at-max" in `curve_display_config`. Slider semantics universal `−1..+1` unchanged.
   - Node y-range widened from `−1..+1` to `−2..+2`. Drag handler accepts the wider range; rendering clamps display position to the rect for visualization but preserves the underlying y. Off-rect nodes show a small red directional rect at the corresponding rect edge.

## B-1: Reset tilt/offset/curvature on module switch

### What's broken
`assign_module` (in `editor/module_popup.rs:142`) calls `params.tilt_param(slot, c).smoothed.reset(0.0)` etc. for each curve. This zeros the smoother's current value but does **not** write to the underlying `AtomicF32` that backs the FloatParam. The editor reads via `.value()`, which is the atomic, so the slider keeps showing the old user setting and the smoother re-converges to it.

### Fix
Mirror the existing `graph_node` reset block in `editor_ui.rs:1363-1377`. After `show_popup` returns `Some(changed_slot)`, iterate `c in 0..7` and call `setter.set_parameter(...)` on each of `tilt_param`, `offset_param`, `curvature_param` for that slot, writing `0.0`. Keep the existing `smoothed.reset(0.0)` calls in `assign_module` — they avoid a one-block discontinuity at the smoother level. The setter call ensures the atomic and the smoother both end at `0.0`.

### Files
- `src/editor_ui.rs` — extend the existing post-`show_popup` block (line ~1363).

### Test
- `tests/module_switch_state_reset.rs` (new) — assert that after constructing `SpectralForgeParams::default()`, manually setting tilt/offset/curvature for slot 2 to non-zero values, then calling `assign_module(slot=2, ty=Freeze)`, **and** running the editor's reset block (the existing graph_node setter pattern, refactored into a helper if practical), the FloatParam values are `0.0`. Note: testing the `editor_ui.rs` block directly is hard since it's UI; the practical test is to (a) assert `assign_module` resets `slot_curve_nodes` (already covered) and (b) hold the post-`show_popup` setter calls in a small helper function in `editor_ui.rs` so the test can call it with a stub setter, or just unit-test the helper that produces the (slot, curve, name, value) tuples to be written.

## B-2: PAST modes inline in slot row

### What changes
Remove the `Mode: <label>` button + `past_popup::show_popup` for PAST. In its place, render the 5 mode labels directly in the slot row's edit area when `slot_module_types[edit_slot] == Past`:

```
[Granular] [DecaySorter] [Convolution] [Reverse] [Stretch]
```

Each is a `selectable_label`. Click sets `params.slot_past_mode.lock()[slot]`. When the active mode is `DecaySorter`, a second inline row appears below for the sort key sub-picker (same behavior as the popup currently has, just inlined).

Hover text on each label = the existing `MODES` description string.

The 5 labels need to fit horizontally. Labels are short ("Granular", "DecaySorter", etc.) — should fit in the slot row width. If they don't fit at the user's current scale, wrap to a second row. Don't shrink the font.

### Why no popup
The user's reading is that PAST mode-switching is frequent enough that the popup adds friction; making the modes always-visible matches Kinetics-like fluidity (even though Kinetics still uses a popup — that one stays for now since Kinetics has 8 modes and inlining them would crowd the row).

### Files
- `src/editor_ui.rs` — replace the `is_past` branch in the slot row's mode-button section (line ~1025-1037).
- `src/editor/past_popup.rs` — keep `mode_label` as a public helper. Delete `show_popup` and `open_at` (or mark `#[deprecated]` and unused). Remove the `past_popup::show_popup` call in the popup-render block (~line 1393).

### Test
- `tests/past_mode_inline.rs` (new) — placeholder: assert `mode_label(PastMode::DecaySorter) == "Decay Sorter"` and the `MODES` array length is 5. Visual regression is a manual smoke test.

## C-1: Offset default `+1` for natural-at-max curves

### What changes
`curve_display_config` already encodes per-curve display metadata. Add a flag `natural_at_max: bool` (default `false`). For curves where the natural rest = `y_max` (MIX everywhere, PAST `AMOUNT`, PAST `SMEAR`, possibly others — to be enumerated by reading current configs), set `natural_at_max: true`.

In `params.rs`, the `offset_param` constructor reads this flag at default-construction time and sets the FloatParam default to `+1.0` instead of `0.0` for those curves. Slider semantics `−1..+1` unchanged. The existing `offset_fn` already maps `v=+1 → y_max`, so the load behavior lands at `y_max` — for MIX that's 100% wet (matches #5).

### Why this is enough
The "dead-half" only matters when the user starts at `v=0` and slides up to find no audible change. With `v=+1` as the load default, the user starts at `y_max` and only experiences the live half (sliding down toward `y_min`). Sliding up from `v=+1` is a no-op (already at max), but that's fine — there's nothing higher to go to and the user has no reason to slide that direction.

### Backward compatibility
Patches saved before this change persist `offset_param` values explicitly; they'll load at whatever the user set. Only **new** patches and freshly assigned slots see `+1.0` as the default. No migration needed.

### Files
- `src/editor/curve_config.rs` — add `natural_at_max` field to `CurveDisplayConfig`; flag the relevant curves.
- `src/params.rs` — `offset_param` default reads the flag.
- `tests/curve_config.rs` — assert MIX (every module) and PAST AMOUNT/SMEAR have `natural_at_max == true`; default offset value for those is `+1.0`.

### Open enumeration
Which curves get `natural_at_max = true`? The pre-existing list from the tracker:
- All MIX curves (every module's MIX is the last curve in its list, defaults at `y_max = 100%`).
- PAST `AMOUNT` (curve index TBD per past spec).
- PAST `SMEAR` (curve index TBD).

The implementer enumerates the actual list by inspecting `curve_display_config` for any curve with `y_natural == y_max` and flagging it. The test pins the list.

## C-2: Node virtual range `−2..+2` + off-rect indicators

### What changes
Three coordinated edits:

1. **Node y range** — `CurveNode::y` is currently constrained at `−1..+1` by drag handlers. Widen the constraint to `−2..+2`. The struct itself doesn't enforce — the editor's drag code does. Find every `clamp(-1.0, 1.0)` on node y in `editor/curve.rs` and widen.
2. **Drag handling** — the rect's full height represents 2 units of node-y (`−1` at the bottom, `+1` at the top). Drag sensitivity per pixel stays unchanged: `dy_node = 2 * dy_pixel / rect_height`. Dragging past the rect edge is now allowed; the node's y goes "virtual" beyond `±1`, clamping at `±2`. So traversing the full new range from `−2` to `+2` is two rect-heights of mouse movement instead of one.
3. **Rendering**:
   - Node circle: clamped to the rect edge for display when `|y| > 1`. The visible position is at the rect's top/bottom edge.
   - Off-rect indicator: a **red directional rect** (small triangle or chevron, ~6px) drawn just outside the rect edge, pointing in the direction the node "lives" beyond. One indicator per off-rect node, drawn at the same x as the node.
   - Curve drawing: the curve interpolation already uses node y as a parameter; off-rect node y values flow through naturally. The rendered curve clamps to rect edges (existing behavior).

### Why this addresses #17 + #18
- **#17 (virtual range)**: directly — node y can be `±2` while display rect represents `±1`, so there's headroom for "loud" or "quiet" extremes that aren't visible but still drive DSP.
- **#18 (offset-aware scaling)**: implicit — when offset is `+1`, the displayed curve is at `y_max` and a node at `y=+1` lands visibly at the top edge. Dragging up moves the underlying y to `+1.5`, off-screen but still recorded. Dragging down moves the underlying y back into the visible range. The user always has full drag headroom because the underlying space (`±2`) is double the visible space (`±1`).

### Receiver responsibility
Modules that consume node-y values (mapped through their `offset_fn`/`gain_to_display`) clamp to their physical limits as appropriate. Example: an attack-time curve receiver would clamp to >= 0.1 ms even though node-y could push the value lower. This is **not** the node's responsibility — the node honestly reports `−2..+2`, the receiver decides what makes audible sense.

### Files
- `src/editor/curve.rs` — drag clamps, node rendering, off-rect indicator drawing.
- `src/editor/theme.rs` — color/size constants for the off-rect indicator (red, ~6px).
- `tests/curve_node_range.rs` (new) — assert dragging beyond the rect produces y values up to `±2`; assert `|y| <= 2` always; assert receivers (sample one — e.g., the dynamics threshold curve) handle out-of-`±1` y without panicking.

### Open: legacy nodes from saved patches
Patches saved before this change have y values in `−1..+1`. Loading them uses the same value — no migration. Once the user drags a node off-rect, the value goes virtual; saving and reloading preserves it.

## Out of scope (deferred)

The following were on C's original list but are not addressed here:

- **Universal slider traversal full redesign** — only the dead-half problem is fixed (via offset default = `+1`). The broader question of "should every curve use the universal `v=−1 → y_min, v=0 → y_natural, v=+1 → y_max` rule everywhere" stays open.
- **Tilt 2× steeper** (#11), **Floor `−120` default** (#10), **Freeze PORTAMENTO 0..750ms** (#16) — sub-project D's scope.
- **PAST AMOUNT/SMEAR full plumbing across modes** (#6), **Freeze Resistance fix** (#15) — sub-project E's scope.
- **PEAK HOLD DSP mismatch** (#F) — separate plan.

## Test strategy

Each item gets a small focused test (listed under each item above). The combined test surface:

- 1 test pinning module-switch FloatParam reset semantics (B-1)
- 1 placeholder test pinning PAST mode label set (B-2)
- 1 test pinning offset default per `natural_at_max` flag (C-1)
- 1 test pinning node y-range expansion (C-2)

Plus the standard regression sweep at the end:
- `cargo test` — 0 failures
- `cargo test --features=probe` — exactly 5 pre-existing failures (unchanged baseline)
- `cargo build --release --features dev-build` — clean
- Dev plugin installed at `~/.clap/spectral/dev/spectral_dev.clap`

## Manual smoke checklist

1. Switch a slot's module from one with non-zero tilt/offset/curvature to another module → sliders show `0.0`, no carryover.
2. Click PAST mode labels in the slot row → mode changes audibly, labels reflect selection.
3. New patch with a MIX-curve slot → MIX defaults to 100% wet (offset slider visually at top).
4. Drag a node off the top of the rect → red indicator appears at top edge, node y goes to `+1.x`, audible effect continues. Drag back into rect → indicator disappears.

## Decisions captured this session

- B + C combined into one small sweep at user's direction; full C UX redesign deferred.
- PAST goes inline (Option C from brainstorm); Kinetics stays popup-based (8 modes is too many to inline).
- Dead-half resolved by defaulting offset to `+1` (Option B from brainstorm, with user simplification — no DSP changes, no `y_max` extension).
- Virtual node range `−2..+2` hard cap; receivers clamp to physical limits.
- Off-rect indicator: red directional rect at rect edge.
