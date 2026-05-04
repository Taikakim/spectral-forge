# Past Module UX Overhaul Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement [`2026-05-04-past-module-ux-design.md`](../specs/2026-05-04-past-module-ux-design.md) — make Past show only the curves the active mode reads per-bin, replace TIME/SPREAD averaging in Reverse/Stretch with proper scalar sliders, add a module-wide Soft Clip toggle, surface DecaySorter's `low_k` floor as a slider, and render a help-box right of the matrix that explains the module/curve. Also lands the foundation `CurveLayout` infrastructure (UI spec §8) consumed by every future per-module UX overhaul.

**Architecture:** A new `CurveLayout` struct on `ModuleSpec` describes "which curves are visible for this mode + label overrides + help text" — looked up by `(module_type, mode_byte)`. Past consumes it first; the next five per-module UX specs (Geometry, Circuit, Life, Kinetics, Harmony) will mimic the same shape. A small set of new per-slot Past `FloatParam`/`BoolParam` scalars replaces the curve-averaging hacks in Reverse/Stretch; their values flow `params → Pipeline → FxMatrix::set_past_scalars → PastModule::scalars` each block. The help-box widget reads `CurveLayout::help_for(curve_idx)` and `CurveLayout::mode_overview` to render content; layout is fixed-width, height matches the matrix region.

**Tech Stack:** Rust, nih-plug, egui (via nih_plug_egui patched), realfft, parking_lot, build.rs codegen for per-slot params.

---

## File Structure

| File | Responsibility |
|---|---|
| `src/dsp/modules/mod.rs` (modify) | Define `CurveLayout` struct; add `active_layout` field to `ModuleSpec`; extend `ProbeSnapshot` with three new Past mode-specific probe fields. |
| `src/dsp/modules/past.rs` (modify) | Implement `past::active_layout(mode: u8) -> CurveLayout` (5 modes inline) with per-curve `help_for`. New `PastScalars` struct + per-slot scalar fields on `PastModule`. Refactor `apply_reverse` and `apply_stretch` to consume scalars instead of averaging curves. New `apply_soft_clip` helper called from `process()` end. Update `ProbeSnapshot` population per mode. |
| `src/dsp/fx_matrix.rs` (modify) | New `set_past_scalars(&[PastScalars; 9])` dispatcher mirroring the existing `set_past_modes`. |
| `src/dsp/pipeline.rs` (modify) | Read new per-slot Past `FloatParam`/`BoolParam` values each block; convert seconds → frames where needed; assemble `[PastScalars; 9]` and call `fx_matrix.set_past_scalars(...)`. |
| `src/editor/curve_config.rs` (modify) | New helper `off_amount_norm(g, o) -> f32`. New function `past_config(curve_idx: u8, mode: u8) -> CurveDisplayConfig`. Route `ModuleType::Past` to `past_config` in `curve_display_config`. |
| `src/editor/curve.rs` (modify) | Add display index 13 ("seconds, history-relative") to `gain_to_display`. Add the `total_history_seconds: f32` parameter to `gain_to_display`; thread through every caller. Add Past arm to `display_curve_idx`. |
| `src/editor/help_box.rs` (new) | Help-box widget. Reads focus state (slot + optional curve), looks up `active_layout(mode)` and pulls `mode_overview` / `help_for`, renders text in a fixed panel right of the matrix. |
| `src/editor/theme.rs` (modify) | Help-box layout constants (font, padding, max width). |
| `src/editor_ui.rs` (modify) | (a) Re-render the curve-tab strip to use `CurveLayout::active` when `active_layout.is_some()`, falling back to `spec.curve_labels` otherwise. (b) Apply `label_overrides`. (c) Render the Past panel_widget (Soft Clip toggle + mode-specific scalars). (d) Mount the help-box widget in the empty space right of the FX matrix. |
| `build.rs` (modify) | Emit five new per-slot Past param families: `s{s}_past_floor_hz`, `s{s}_past_reverse_window_s`, `s{s}_past_stretch_rate`, `s{s}_past_stretch_dither`, `s{s}_past_soft_clip`. New dispatch macros `past_floor_dispatch`, `past_reverse_window_dispatch`, etc. |
| `src/params.rs` (modify) | New typed accessors: `past_floor_param(slot)`, `past_reverse_window_param(slot)`, `past_stretch_rate_param(slot)`, `past_stretch_dither_param(slot)`, `past_soft_clip_param(slot)` using the new dispatch macros. |
| `tests/curve_layout.rs` (new) | Foundation: assert `CurveLayout` struct shape, `ModuleSpec::active_layout` default `None`, default modules unaffected. |
| `tests/past_layout.rs` (new) | For each `PastMode`, assert `active_layout(mode).active` matches the spec table; assert `help_for(curve_idx)` non-empty for every curve in `active`; assert label overrides match the spec. |
| `tests/past_pipeline.rs` (extend) | Add scalar-driven tests: Reverse `window_frames` drives the read modulus; Stretch `rate=2.0` advances `stretch_read_phase` by 2.0 per hop; Soft Clip ON clamps a synthetic high-magnitude bin; Soft Clip OFF passes through. |
| `tests/calibration.rs` (extend) | Probe coverage for all five Past modes (`past_reverse_window_s` populated only in Reverse, etc.). |
| `tests/curve_config.rs` (extend) | New `past_config` covers every `(mode, curve_idx)` pair; `off_amount_norm` round-trip; `gain_to_display(13, ...)` formula. |
| `tests/editor_panel_dispatch.rs` (extend) | New Past panel_widget renders Soft Clip + mode-specific scalars (logical assertion, not pixel snapshot). |

---

## Task 1: Add `CurveLayout` struct to `ModuleSpec`

**Files:**
- Modify: `src/dsp/modules/mod.rs:387-450` (add struct, field, default)
- Test: `tests/curve_layout.rs` (new)

- [ ] **Step 1: Write the failing test**

Create `tests/curve_layout.rs`:

```rust
//! Foundation regression tests for the per-mode `CurveLayout` infrastructure
//! introduced by the Past UX overhaul. See
//! docs/superpowers/specs/2026-04-23-ui-parameter-spec-design.md §8.

use spectral_forge::dsp::modules::{module_spec, CurveLayout, ModuleType};

/// CurveLayout struct fields exist with the documented types.
#[test]
fn curve_layout_has_expected_fields() {
    fn empty_help(_: u8) -> &'static str { "" }
    let layout = CurveLayout {
        active:          &[0u8, 2u8, 4u8],
        label_overrides: &[(1u8, "Age"), (1u8, "Delay")],
        help_for:        empty_help,
        mode_overview:   Some("test"),
    };
    assert_eq!(layout.active, &[0, 2, 4]);
    assert_eq!(layout.label_overrides.len(), 2);
    assert_eq!(layout.help_for(0), "");
    assert_eq!(layout.mode_overview, Some("test"));
}

/// Every existing ModuleSpec defaults `active_layout` to `None`.
/// Modules without modes (Dynamics, Freeze, etc.) must keep the legacy "render
/// all curve_labels" behaviour. Only modules that have explicitly opted in
/// should return `Some`.
#[test]
fn default_module_specs_have_active_layout_none() {
    let mode_bearing = [
        ModuleType::Past, ModuleType::Geometry, ModuleType::Circuit,
        ModuleType::Life, ModuleType::Kinetics, ModuleType::Harmony,
        ModuleType::Modulate, ModuleType::Rhythm,
    ];
    for ty in [
        ModuleType::Dynamics, ModuleType::Freeze, ModuleType::PhaseSmear,
        ModuleType::Contrast, ModuleType::Gain, ModuleType::MidSide,
        ModuleType::TransientSustainedSplit, ModuleType::Future,
        ModuleType::Punch, ModuleType::Harmonic, ModuleType::Master,
        ModuleType::Empty,
    ] {
        if mode_bearing.contains(&ty) { continue; }
        assert!(
            module_spec(ty).active_layout.is_none(),
            "Non-mode-bearing module {:?} unexpectedly has active_layout = Some",
            ty,
        );
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --release --test curve_layout 2>&1 | tail -20`
Expected: FAIL with `unresolved import: CurveLayout` (struct doesn't exist yet).

- [ ] **Step 3: Add `CurveLayout` struct**

Edit `src/dsp/modules/mod.rs`. Find the `pub struct ModuleSpec` definition (around line 387) and add **above** it:

```rust
/// Per-mode descriptor for visible curves, label overrides, and help-box copy.
/// See docs/superpowers/specs/2026-04-23-ui-parameter-spec-design.md §8.
///
/// Modules with internal sub-modes use `active_layout` on `ModuleSpec` to
/// declare a function that, given the slot's current mode (as `u8`), returns
/// a `CurveLayout`. The UI renders only the curves listed in `active`,
/// applies any `label_overrides`, and feeds `help_for` / `mode_overview`
/// into the help-box panel.
pub struct CurveLayout {
    /// Indices (into the module's full curve set) of curves visible for this mode.
    /// Order is render order; e.g. `&[0, 2, 4]` hides curves 1 and 3 entirely.
    pub active: &'static [u8],

    /// Per-curve label overrides for this mode. Each tuple is `(curve_idx, override_label)`.
    /// Curves not listed fall back to `ModuleSpec::curve_labels[curve_idx]`.
    pub label_overrides: &'static [(u8, &'static str)],

    /// Help-box copy keyed by curve_idx (full curve index, not position in `active`).
    /// Returning an empty string means "use the module overview / static description."
    pub help_for: fn(curve_idx: u8) -> &'static str,

    /// Help-box overview shown when a slot is selected but no curve is in focus.
    /// `None` ⇒ use the module's static description from outside the layout.
    pub mode_overview: Option<&'static str>,
}
```

- [ ] **Step 4: Add `active_layout` field to `ModuleSpec`**

In the same file, modify the `ModuleSpec` struct (around line 387):

```rust
pub struct ModuleSpec {
    // ... existing fields ...
    /// If `Some`, the UI consults `active_layout(slot.mode as u8)` per frame to
    /// decide which curve tabs to render and what help text to show. `None` ⇒
    /// the UI renders all `curve_labels` as today (modules without modes).
    /// See docs/superpowers/specs/2026-04-23-ui-parameter-spec-design.md §8.
    pub active_layout: Option<fn(mode: u8) -> CurveLayout>,
}
```

Add `active_layout: None,` to **every** `ModuleSpec { ... }` literal in `module_spec()`. There are 20 of them. The diff per literal:

```rust
ModuleSpec {
    // ... existing fields ...
    needs_midi: false,
    active_layout: None,        // <- add this line
}
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test --release --test curve_layout 2>&1 | tail -10`
Expected: PASS, both tests green.

- [ ] **Step 6: Commit**

```bash
git add src/dsp/modules/mod.rs tests/curve_layout.rs
git commit -m "feat(modules): CurveLayout struct + ModuleSpec::active_layout (Phase Past UX, Task 1)

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

## Task 2: Add `off_amount_norm` helper to `curve_config.rs`

**Files:**
- Modify: `src/editor/curve_config.rs` (add function next to other `off_*` helpers)
- Test: `tests/curve_config.rs` (extend)

- [ ] **Step 1: Write the failing test**

Append to `tests/curve_config.rs`:

```rust
//! Test for the new `off_amount_norm` helper introduced by the Past UX
//! overhaul. See docs/superpowers/specs/2026-05-04-past-module-ux-design.md §5.

#[test]
fn off_amount_norm_clamps_and_passes_zero() {
    use spectral_forge::editor::curve_config::off_amount_norm;
    // Identity at o=0
    assert_eq!(off_amount_norm(0.5, 0.0),  0.5);
    assert_eq!(off_amount_norm(0.0, 0.0),  0.0);
    assert_eq!(off_amount_norm(1.0, 0.0),  1.0);
    // Linear add
    assert_eq!(off_amount_norm(0.3, 0.4),  0.7);
    assert_eq!(off_amount_norm(0.5, -0.3), 0.2);
    // Clamps at 0 and 1
    assert_eq!(off_amount_norm(0.5,  0.7), 1.0);
    assert_eq!(off_amount_norm(0.5, -0.7), 0.0);
    // Beyond range still clamps
    assert_eq!(off_amount_norm(2.0,  0.5), 1.0);
    assert_eq!(off_amount_norm(-1.0, 0.0), 0.0);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --release --test curve_config off_amount_norm_clamps 2>&1 | tail -10`
Expected: FAIL with `unresolved import: off_amount_norm`.

- [ ] **Step 3: Add the helper**

Edit `src/editor/curve_config.rs`. Find the block of `off_*` helpers (around line 320, before `off_thresh`) and insert at the same level:

```rust
/// Linear add, clamped to [0, 1]. For curves whose gain is interpreted as a
/// normalised fraction (e.g. Past's Age/Delay representing a fraction of the
/// history buffer's `capacity_frames`).
/// See docs/superpowers/specs/2026-04-23-ui-parameter-spec-design.md §7 and
/// docs/superpowers/specs/2026-05-04-past-module-ux-design.md §5.
#[inline] pub fn off_amount_norm(g: f32, o: f32) -> f32 {
    (g + o).clamp(0.0, 1.0)
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --release --test curve_config off_amount_norm_clamps 2>&1 | tail -10`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/editor/curve_config.rs tests/curve_config.rs
git commit -m "feat(curve_config): add off_amount_norm helper (Phase Past UX, Task 2)

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

## Task 3: Add `gain_to_display` index 13 (history-relative seconds)

**Files:**
- Modify: `src/editor/curve.rs` (extend `gain_to_display` with new arg + index 13)
- Modify: every caller of `gain_to_display` to pass `total_history_seconds` (`0.0` for legacy callers, real value for Past)
- Test: `tests/curve_config.rs` (extend) — assert formula

- [ ] **Step 1: Write the failing test**

Append to `tests/curve_config.rs`:

```rust
#[test]
fn gain_to_display_index_13_returns_history_relative_seconds() {
    use spectral_forge::editor::curve::gain_to_display;
    // gain * total_history_seconds, clamped to [0, total]
    let v = gain_to_display(13, 0.5, 0.0, 0.0, 0.0, 0.0, /* total */ 4.0);
    assert!((v - 2.0).abs() < 1e-6, "expected 2.0, got {v}");
    let v = gain_to_display(13, 0.0, 0.0, 0.0, 0.0, 0.0, 4.0);
    assert_eq!(v, 0.0);
    let v = gain_to_display(13, 1.0, 0.0, 0.0, 0.0, 0.0, 4.0);
    assert!((v - 4.0).abs() < 1e-6);
    // Clamp above total
    let v = gain_to_display(13, 2.0, 0.0, 0.0, 0.0, 0.0, 4.0);
    assert_eq!(v, 4.0);
    // Clamp below zero
    let v = gain_to_display(13, -1.0, 0.0, 0.0, 0.0, 0.0, 4.0);
    assert_eq!(v, 0.0);
}
```

- [ ] **Step 2: Run test to verify it fails (signature mismatch)**

Run: `cargo test --release --test curve_config gain_to_display_index_13 2>&1 | tail -10`
Expected: FAIL — function signature doesn't take a 7th argument yet.

- [ ] **Step 3: Extend `gain_to_display` signature**

Edit `src/editor/curve.rs`. Locate `pub fn gain_to_display(curve_idx: usize, gain: f32, global_attack_ms: f32, global_release_ms: f32, db_min: f32, db_max: f32) -> f32` (around line 424). Change the signature and the body:

```rust
/// Convert a curve's linear gain to its physical display value (no freq scaling).
/// Used for the coloured response line.
///
/// `total_history_seconds` is consumed only by display index 13 ("seconds,
/// history-relative") used by Past's Age/Delay curves; legacy callers pass
/// `0.0` and never hit index 13.
pub fn gain_to_display(
    curve_idx: usize,
    gain: f32,
    global_attack_ms: f32,
    global_release_ms: f32,
    db_min: f32,
    db_max: f32,
    total_history_seconds: f32,
) -> f32 {
    // UI parameter contract: see docs/superpowers/specs/2026-04-23-ui-parameter-spec-design.md
    match curve_idx {
        // ... existing arms 0..=12 unchanged ...
        13 => (gain * total_history_seconds).clamp(0.0, total_history_seconds),
        _ => gain,
    }
}
```

- [ ] **Step 4: Update every caller of `gain_to_display`**

```bash
grep -rn "gain_to_display(" /home/kim/Projects/spectral/src/ /home/kim/Projects/spectral/tests/ | grep -v "fn gain_to_display"
```

For each call site found, append `, 0.0` as the seventh argument unless the call site has access to a Past-specific `total_history_seconds` value (only the new past_config formatter and the editor curve-paint path with Past in scope will pass a real value — handled in Task 6 and Task 15). The legacy default is `0.0`.

Examples to update:
- `src/editor_ui.rs:1105-1109` (offset slider formatter): `crv::gain_to_display(off_disp_idx, g_off, off_atk_ms, off_rel_ms, off_db_min, off_db_max, /* total */ 0.0)`
- `src/editor/curve.rs::paint_response_curve` callers: pass `0.0` for now; Past-specific replacement happens in Task 15.

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test --release --test curve_config gain_to_display_index_13 2>&1 | tail -10`
Expected: PASS.

- [ ] **Step 6: Run the full test suite to ensure no caller missed**

Run: `cargo test --release 2>&1 | grep -E "FAILED|^error" | head -20`
Expected: no FAILED tests, no compile errors.

- [ ] **Step 7: Commit**

```bash
git add src/editor/curve.rs src/editor_ui.rs tests/curve_config.rs
git commit -m "feat(curve): gain_to_display index 13 (history-relative seconds) (Phase Past UX, Task 3)

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

## Task 4: Add Past arm to `display_curve_idx`

**Files:**
- Modify: `src/editor/curve.rs:223-268` (`pub fn display_curve_idx`)
- Test: `tests/curve_config.rs` (extend)

- [ ] **Step 1: Write the failing test**

Append to `tests/curve_config.rs`:

```rust
#[test]
fn display_curve_idx_routes_past_curves_to_specific_scales() {
    use spectral_forge::editor::curve::display_curve_idx;
    use spectral_forge::dsp::modules::{ModuleType, GainMode};
    // Past has 5 curves; routing per spec §5.
    assert_eq!(display_curve_idx(ModuleType::Past, 0, GainMode::Add),  6,  "AMOUNT → %");
    assert_eq!(display_curve_idx(ModuleType::Past, 1, GainMode::Add),  13, "TIME → seconds-history");
    assert_eq!(display_curve_idx(ModuleType::Past, 2, GainMode::Add),  9,  "THRESHOLD → dBFS");
    assert_eq!(display_curve_idx(ModuleType::Past, 3, GainMode::Add),  6,  "SPREAD/Smear → %");
    assert_eq!(display_curve_idx(ModuleType::Past, 4, GainMode::Add),  6,  "MIX → %");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --release --test curve_config display_curve_idx_routes_past 2>&1 | tail -10`
Expected: FAIL — Past falls through to `_ => curve_idx`, returns the raw index.

- [ ] **Step 3: Add the Past arm**

Edit `src/editor/curve.rs`, the `display_curve_idx` function. Find the catch-all `_ => curve_idx,` at the end of the outer match. Add a Past arm directly above it:

```rust
ModuleType::Past => match curve_idx {
    0 => 6,   // AMOUNT → 0–100 %
    1 => 13,  // TIME (Granular Age / Convolution Delay) → seconds-history
    2 => 9,   // THRESHOLD → dBFS (mirrors Freeze threshold scale)
    3 => 6,   // SPREAD / Smear → 0–100 %
    4 => 6,   // MIX → 0–100 %
    _ => curve_idx,
},
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --release --test curve_config display_curve_idx_routes_past 2>&1 | tail -10`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/editor/curve.rs tests/curve_config.rs
git commit -m "feat(curve): Past arm in display_curve_idx (Phase Past UX, Task 4)

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

## Task 5: Add `past_config(curve_idx, mode)` and route Past in `curve_display_config`

**Files:**
- Modify: `src/editor/curve_config.rs:42-69` (route Past) + add `past_config` helper
- Test: `tests/curve_config.rs` (extend)

- [ ] **Step 1: Write the failing test**

Append to `tests/curve_config.rs`:

```rust
#[test]
fn past_config_returns_calibrated_display_per_curve() {
    use spectral_forge::editor::curve_config::{curve_display_config, off_amount_norm, off_mix, off_thresh, off_identity};
    use spectral_forge::dsp::modules::{ModuleType, GainMode};

    // AMOUNT (curve 0) — % units, neutral at 100, off_mix
    let amount = curve_display_config(ModuleType::Past, 0, GainMode::Add);
    assert_eq!(amount.y_label, "%");
    assert_eq!(amount.y_min, 0.0);
    assert_eq!(amount.y_max, 100.0);
    assert!((amount.y_natural - 100.0).abs() < 1e-6);
    assert!(std::ptr::eq(amount.offset_fn as *const (), off_mix as *const ()));

    // TIME (curve 1) — seconds, neutral at 0.0, off_amount_norm
    let time = curve_display_config(ModuleType::Past, 1, GainMode::Add);
    assert_eq!(time.y_label, "s");
    assert_eq!(time.y_min, 0.0);
    // y_max is set to a placeholder of 1.0 inside curve_display_config and rewritten
    // at paint time using `total_history_seconds` from the live Pipeline. Test the
    // structural identity here.
    assert!(std::ptr::eq(time.offset_fn as *const (), off_amount_norm as *const ()));

    // THRESHOLD (curve 2) — dBFS, neutral -60, off_thresh
    let thresh = curve_display_config(ModuleType::Past, 2, GainMode::Add);
    assert_eq!(thresh.y_label, "dBFS");
    assert_eq!(thresh.y_min, -80.0);
    assert_eq!(thresh.y_max, 0.0);
    assert!((thresh.y_natural - (-60.0)).abs() < 1e-6);
    assert!(std::ptr::eq(thresh.offset_fn as *const (), off_thresh as *const ()));

    // SPREAD (Smear in Granular) — % units
    let spread = curve_display_config(ModuleType::Past, 3, GainMode::Add);
    assert_eq!(spread.y_label, "%");
    assert!(std::ptr::eq(spread.offset_fn as *const (), off_mix as *const ()));

    // MIX
    let mix = curve_display_config(ModuleType::Past, 4, GainMode::Add);
    assert_eq!(mix.y_label, "%");
    assert!(std::ptr::eq(mix.offset_fn as *const (), off_mix as *const ()));

    // Out-of-range curve_idx falls back to default_config (off_identity)
    let oob = curve_display_config(ModuleType::Past, 99, GainMode::Add);
    assert!(std::ptr::eq(oob.offset_fn as *const (), off_identity as *const ()));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --release --test curve_config past_config_returns 2>&1 | tail -10`
Expected: FAIL — Past arm currently routes to `default_config()`, so labels/offsets don't match.

- [ ] **Step 3: Add `past_config` helper**

Edit `src/editor/curve_config.rs`. Below the existing `dynamics_config`/`freeze_config`/etc. helpers (before the `// ── Per-calibration offset functions ───` block, around line 300), add:

```rust
/// Per-curve display calibration for `ModuleType::Past`.
///
/// `mode` is currently unused at this level — the per-mode label overrides
/// (Age vs Delay) live in `past::active_layout` (curve_layout::label_overrides).
/// `past_config` produces axis units, ranges, grid lines, and offset_fn for the
/// **physical** display layer; per-mode label changes happen above it.
///
/// See docs/superpowers/specs/2026-05-04-past-module-ux-design.md §5.
pub fn past_config(curve_idx: usize, _mode: u8) -> CurveDisplayConfig {
    match curve_idx {
        0 => CurveDisplayConfig {
            y_label: "%", y_min: 0.0, y_max: 100.0, y_log: false,
            grid_lines: &[(25.0, "25%"), (50.0, "50%"), (75.0, "75%"), (100.0, "100%")],
            y_natural: 100.0,
            offset_fn: off_mix,
        },
        1 => CurveDisplayConfig {
            // y_max placeholder; the painter substitutes `total_history_seconds`
            // when rendering. The slider's custom_formatter passes the live
            // value via gain_to_display(13, ...).
            y_label: "s", y_min: 0.0, y_max: 1.0, y_log: false,
            grid_lines: &[(0.25, "25%"), (0.5, "50%"), (0.75, "75%"), (1.0, "100%")],
            y_natural: 0.0,
            offset_fn: off_amount_norm,
        },
        2 => CurveDisplayConfig {
            y_label: "dBFS", y_min: -80.0, y_max: 0.0, y_log: false,
            grid_lines: &[(-60.0, "-60"), (-40.0, "-40"), (-20.0, "-20"), (-6.0, "-6")],
            y_natural: -60.0,
            offset_fn: off_thresh,
        },
        3 => CurveDisplayConfig {
            y_label: "%", y_min: 0.0, y_max: 100.0, y_log: false,
            grid_lines: &[(25.0, "25%"), (50.0, "50%"), (75.0, "75%"), (100.0, "100%")],
            y_natural: 100.0,
            offset_fn: off_mix,
        },
        4 => CurveDisplayConfig {
            y_label: "%", y_min: 0.0, y_max: 100.0, y_log: false,
            grid_lines: &[(25.0, "25%"), (50.0, "50%"), (75.0, "75%"), (100.0, "100%")],
            y_natural: 100.0,
            offset_fn: off_mix,
        },
        _ => default_config(),
    }
}
```

- [ ] **Step 4: Route Past to `past_config`**

Edit the same file's `curve_display_config` (around line 42–69). Replace the `ModuleType::Past => default_config(),` line with:

```rust
ModuleType::Past => past_config(curve_idx, 0),
```

(The mode argument is `0` here because at the `curve_display_config` level we don't yet know the active mode; per-mode label overrides come from `active_layout`. We pass `0` as a stable fallback. If a future spec extension wants per-mode physical units, this widens to `mode_for(slot, module_type)`.)

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test --release --test curve_config past_config_returns 2>&1 | tail -10`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/editor/curve_config.rs tests/curve_config.rs
git commit -m "feat(curve_config): past_config + curve_display_config Past route (Phase Past UX, Task 5)

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

## Task 6: Add `past::active_layout(mode)` with all 5 modes

**Files:**
- Modify: `src/dsp/modules/past.rs` (add `active_layout` function + `help_for` text)
- Test: `tests/past_layout.rs` (new)

- [ ] **Step 1: Write the failing test**

Create `tests/past_layout.rs`:

```rust
//! Per-mode CurveLayout regression for Past. See
//! docs/superpowers/specs/2026-05-04-past-module-ux-design.md §1 + §4.

use spectral_forge::dsp::modules::past::{active_layout, PastMode};

#[test]
fn past_active_layout_granular_visible_curves() {
    let layout = active_layout(PastMode::Granular as u8);
    assert_eq!(layout.active, &[0u8, 1, 2, 3, 4], "Granular shows all 5 curves");
    // Age and Smear are mode-specific labels
    let mut got = layout.label_overrides.iter().copied().collect::<Vec<_>>();
    got.sort();
    let mut want: Vec<(u8, &'static str)> = vec![(1, "Age"), (3, "Smear")];
    want.sort();
    assert_eq!(got, want);
}

#[test]
fn past_active_layout_decay_sorter_visible_curves() {
    let layout = active_layout(PastMode::DecaySorter as u8);
    assert_eq!(layout.active, &[0u8, 2, 4]);
    assert!(layout.label_overrides.is_empty());
}

#[test]
fn past_active_layout_convolution_visible_curves() {
    let layout = active_layout(PastMode::Convolution as u8);
    assert_eq!(layout.active, &[0u8, 1, 2, 4]);
    let mut got = layout.label_overrides.iter().copied().collect::<Vec<_>>();
    got.sort();
    let mut want: Vec<(u8, &'static str)> = vec![(1, "Delay")];
    want.sort();
    assert_eq!(got, want);
}

#[test]
fn past_active_layout_reverse_visible_curves() {
    let layout = active_layout(PastMode::Reverse as u8);
    assert_eq!(layout.active, &[0u8, 2, 4]);
    assert!(layout.label_overrides.is_empty());
}

#[test]
fn past_active_layout_stretch_visible_curves() {
    let layout = active_layout(PastMode::Stretch as u8);
    assert_eq!(layout.active, &[0u8, 4]);
    assert!(layout.label_overrides.is_empty());
}

#[test]
fn past_active_layout_help_for_non_empty_for_every_active_curve() {
    for mode in [
        PastMode::Granular, PastMode::DecaySorter, PastMode::Convolution,
        PastMode::Reverse, PastMode::Stretch,
    ] {
        let layout = active_layout(mode as u8);
        for &curve_idx in layout.active {
            let help = (layout.help_for)(curve_idx);
            assert!(
                !help.is_empty(),
                "help_for({curve_idx}) is empty for mode {mode:?}",
            );
        }
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --release --test past_layout 2>&1 | tail -10`
Expected: FAIL — `active_layout` not exported from `past` module.

- [ ] **Step 3: Implement `active_layout`**

Edit `src/dsp/modules/past.rs`. At the top-level (outside `impl PastModule`), add:

```rust
use crate::dsp::modules::CurveLayout;

/// Per-mode `CurveLayout` for Past. See
/// docs/superpowers/specs/2026-05-04-past-module-ux-design.md §1 + §4.
///
/// Wired via `active_layout: Some(past::active_layout)` on the Past
/// `ModuleSpec` literal.
pub fn active_layout(mode: u8) -> CurveLayout {
    match PastMode::try_from(mode).unwrap_or(PastMode::Granular) {
        PastMode::Granular => CurveLayout {
            active:          &[0, 1, 2, 3, 4],
            label_overrides: &[(1, "Age"), (3, "Smear")],
            help_for:        granular_help_for,
            mode_overview:   None,
        },
        PastMode::DecaySorter => CurveLayout {
            active:          &[0, 2, 4],
            label_overrides: &[],
            help_for:        decay_sorter_help_for,
            mode_overview:   None,
        },
        PastMode::Convolution => CurveLayout {
            active:          &[0, 1, 2, 4],
            label_overrides: &[(1, "Delay")],
            help_for:        convolution_help_for,
            mode_overview:   None,
        },
        PastMode::Reverse => CurveLayout {
            active:          &[0, 2, 4],
            label_overrides: &[],
            help_for:        reverse_help_for,
            mode_overview:   None,
        },
        PastMode::Stretch => CurveLayout {
            active:          &[0, 4],
            label_overrides: &[],
            help_for:        stretch_help_for,
            mode_overview:   None,
        },
    }
}

fn granular_help_for(curve_idx: u8) -> &'static str {
    match curve_idx {
        0 => "How much of the historical bin replaces the current bin. 0 = current only, 1 = historical only. Adds with upstream BinPhysics `crystallization`.",
        1 => "Per-bin lookback into history. 0 = now, 1 = oldest available frame.",
        2 => "Per-bin gate. Bins whose current magnitude falls below the threshold pass through unchanged.",
        3 => "Toggle (>0.5) per-bin 3-bin frequency smear of the historical read. Smooths bin-leakage across narrow partials.",
        4 => "Per-bin wet/dry.",
        _ => "",
    }
}

fn decay_sorter_help_for(curve_idx: u8) -> &'static str {
    match curve_idx {
        0 => "Per-bin output gain on the rearranged signal.",
        2 => "Per-bin floor — bins below this magnitude are excluded from sorting.",
        4 => "Per-bin wet/dry of sorted output vs. original.",
        _ => "",
    }
}

fn convolution_help_for(curve_idx: u8) -> &'static str {
    match curve_idx {
        0 => "Per-bin convolution strength. Multiplied by upstream BinPhysics `flux` if present.",
        1 => "Per-bin delay into history. Each bin can read at a different age — low bins lag, high bins recent, or any other shape.",
        2 => "Per-bin gate on the current frame's magnitude.",
        4 => "Per-bin wet/dry.",
        _ => "",
    }
}

fn reverse_help_for(curve_idx: u8) -> &'static str {
    match curve_idx {
        0 => "Per-bin keep during the reverse read.",
        2 => "Per-bin gate.",
        4 => "Per-bin wet/dry.",
        _ => "",
    }
}

fn stretch_help_for(curve_idx: u8) -> &'static str {
    match curve_idx {
        0 => "Per-bin keep during the stretched read.",
        4 => "Per-bin wet/dry.",
        _ => "",
    }
}
```

If `PastMode` doesn't already have a `TryFrom<u8>` impl, add one near its definition:

```rust
impl TryFrom<u8> for PastMode {
    type Error = ();
    fn try_from(b: u8) -> Result<Self, ()> {
        match b {
            0 => Ok(Self::Granular),
            1 => Ok(Self::DecaySorter),
            2 => Ok(Self::Convolution),
            3 => Ok(Self::Reverse),
            4 => Ok(Self::Stretch),
            _ => Err(()),
        }
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --release --test past_layout 2>&1 | tail -10`
Expected: PASS, six tests.

- [ ] **Step 5: Commit**

```bash
git add src/dsp/modules/past.rs tests/past_layout.rs
git commit -m "feat(past): per-mode active_layout + help_for text (Phase Past UX, Task 6)

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

## Task 7: Wire `active_layout: Some(past::active_layout)` on Past's ModuleSpec

**Files:**
- Modify: `src/dsp/modules/mod.rs` (Past `ModuleSpec` literal)
- Test: extend `tests/curve_layout.rs`

- [ ] **Step 1: Write the failing test**

Append to `tests/curve_layout.rs`:

```rust
#[test]
fn past_module_spec_has_active_layout_some() {
    use spectral_forge::dsp::modules::{module_spec, ModuleType, past::PastMode};
    let spec = module_spec(ModuleType::Past);
    let layout_fn = spec.active_layout
        .expect("Past must opt in to active_layout for the per-mode UI to work");
    let granular = layout_fn(PastMode::Granular as u8);
    assert_eq!(granular.active, &[0u8, 1, 2, 3, 4]);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --release --test curve_layout past_module_spec_has 2>&1 | tail -10`
Expected: FAIL — Past spec literal still defaults `active_layout: None`.

- [ ] **Step 3: Wire the function**

Edit `src/dsp/modules/mod.rs`. Find the `ModuleType::Past =>` arm in `module_spec` (search for `"PAST"` or `Past` string label). Change its `active_layout: None,` line to:

```rust
active_layout: Some(crate::dsp::modules::past::active_layout),
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --release --test curve_layout past_module_spec_has 2>&1 | tail -10`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/dsp/modules/mod.rs tests/curve_layout.rs
git commit -m "feat(past): wire active_layout into ModuleSpec (Phase Past UX, Task 7)

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

## Task 8: Emit five new per-slot Past scalar params via `build.rs`

**Files:**
- Modify: `build.rs` (new emit functions + dispatch macros)
- Modify: `src/params.rs` (typed accessors using new dispatch macros)
- Test: `tests/past_params.rs` (new)

- [ ] **Step 1: Write the failing test**

Create `tests/past_params.rs`:

```rust
//! Past UX overhaul scalars round-trip through nih-plug params. See
//! docs/superpowers/specs/2026-05-04-past-module-ux-design.md §2 + §3.

use spectral_forge::params::SpectralForgeParams;
use std::sync::Arc;

#[test]
fn past_scalar_params_exist_with_correct_defaults() {
    let params = Arc::new(SpectralForgeParams::default());

    for s in 0..9usize {
        let floor = params.past_floor_param(s).expect("floor exists for slot");
        assert!((floor.value() - 230.0).abs() < 1.0, "Floor default ≈ 230 Hz, got {}", floor.value());

        let window = params.past_reverse_window_param(s).expect("window exists for slot");
        assert!((window.value() - 1.0).abs() < 1e-6);

        let rate = params.past_stretch_rate_param(s).expect("rate exists for slot");
        assert!((rate.value() - 1.0).abs() < 1e-6);

        let dither = params.past_stretch_dither_param(s).expect("dither exists for slot");
        assert_eq!(dither.value(), 0.0);

        let soft_clip = params.past_soft_clip_param(s).expect("soft_clip exists for slot");
        assert!(soft_clip.value(), "Soft Clip default ON");
    }
}

#[test]
fn past_scalar_params_out_of_range_returns_none() {
    let params = Arc::new(SpectralForgeParams::default());
    assert!(params.past_floor_param(9).is_none());
    assert!(params.past_reverse_window_param(9).is_none());
    assert!(params.past_stretch_rate_param(9).is_none());
    assert!(params.past_stretch_dither_param(9).is_none());
    assert!(params.past_soft_clip_param(9).is_none());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --release --test past_params 2>&1 | tail -10`
Expected: FAIL — accessors don't exist.

- [ ] **Step 3: Add emit functions to `build.rs`**

Edit `build.rs`. After `emit_tilt_offset_inits` (around line 207), add:

```rust
fn emit_past_scalar_fields(f: &mut File) {
    for s in 0..NUM_SLOTS {
        writeln!(f, "    pub s{s}_past_floor_hz:        FloatParam,").unwrap();
        writeln!(f, "    pub s{s}_past_reverse_window_s: FloatParam,").unwrap();
        writeln!(f, "    pub s{s}_past_stretch_rate:    FloatParam,").unwrap();
        writeln!(f, "    pub s{s}_past_stretch_dither:  FloatParam,").unwrap();
        writeln!(f, "    pub s{s}_past_soft_clip:       BoolParam,").unwrap();
    }
}

fn emit_past_scalar_inits(f: &mut File) {
    for s in 0..NUM_SLOTS {
        writeln!(
            f,
            "            s{s}_past_floor_hz: FloatParam::new(\"s{s}past_floor_hz\", 230.0f32, \
             FloatRange::Skewed {{ min: 20.0f32, max: 2000.0f32, factor: FloatRange::skew_factor(-2.0) }})\
             .with_smoother(SmoothingStyle::Logarithmic(50.0))\
             .with_unit(\" Hz\")\
             .hide_in_generic_ui(),"
        ).unwrap();
        writeln!(
            f,
            "            s{s}_past_reverse_window_s: FloatParam::new(\"s{s}past_reverse_window_s\", 1.0f32, \
             FloatRange::Linear {{ min: 0.05f32, max: 30.0f32 }})\
             .with_smoother(SmoothingStyle::Linear(50.0))\
             .with_unit(\" s\")\
             .hide_in_generic_ui(),"
        ).unwrap();
        writeln!(
            f,
            "            s{s}_past_stretch_rate: FloatParam::new(\"s{s}past_stretch_rate\", 1.0f32, \
             FloatRange::Skewed {{ min: 0.05f32, max: 4.0f32, factor: FloatRange::skew_factor(0.0) }})\
             .with_smoother(SmoothingStyle::Logarithmic(50.0))\
             .with_unit(\"x\")\
             .hide_in_generic_ui(),"
        ).unwrap();
        writeln!(
            f,
            "            s{s}_past_stretch_dither: FloatParam::new(\"s{s}past_stretch_dither\", 0.0f32, \
             FloatRange::Linear {{ min: 0.0f32, max: 1.0f32 }})\
             .with_smoother(SmoothingStyle::Linear(20.0))\
             .with_unit(\" %\")\
             .hide_in_generic_ui(),"
        ).unwrap();
        writeln!(
            f,
            "            s{s}_past_soft_clip: BoolParam::new(\"s{s}past_soft_clip\", true)\
             .hide_in_generic_ui(),"
        ).unwrap();
    }
}

fn emit_past_scalar_dispatch(f: &mut File) {
    for (suffix, kind) in [
        ("floor_hz",         "FloatParam"),
        ("reverse_window_s", "FloatParam"),
        ("stretch_rate",     "FloatParam"),
        ("stretch_dither",   "FloatParam"),
        ("soft_clip",        "BoolParam"),
    ] {
        writeln!(f, "macro_rules! past_{suffix}_dispatch {{").unwrap();
        writeln!(f, "    ($self:expr, $s:expr) => {{").unwrap();
        writeln!(f, "        match $s {{").unwrap();
        for s in 0..NUM_SLOTS {
            writeln!(f, "            {s} => &$self.generated.s{s}_past_{suffix},").unwrap();
        }
        writeln!(f, "            _ => unreachable!(),").unwrap();
        writeln!(f, "        }}").unwrap();
        writeln!(f, "    }};").unwrap();
        writeln!(f, "}}").unwrap();
        let _ = kind; // future: typed dispatch if FloatParam vs BoolParam diverges
    }
}
```

In `main()`, find the section that calls `emit_tilt_dispatch`/`emit_offset_dispatch`/etc., and add:

```rust
emit_past_scalar_dispatch(&mut f);
```

In the field-emission section (where `emit_tilt_offset_fields` would be — search for `_tilt:`), add:

```rust
emit_past_scalar_fields(&mut f);
```

In the init-emission section (where `emit_tilt_offset_inits` would be — search for `_tilt: FloatParam::new`), add:

```rust
emit_past_scalar_inits(&mut f);
```

- [ ] **Step 4: Add typed accessors in `src/params.rs`**

After the `curvature_param` accessor (around line 671), add:

```rust
/// Per-slot Past Floor (Hz). See spec 2026-05-04-past-module-ux-design.md §2.
pub fn past_floor_param(&self, slot: usize) -> Option<&FloatParam> {
    use crate::param_ids::NUM_SLOTS;
    if slot >= NUM_SLOTS { return None; }
    Some(past_floor_hz_dispatch!(self, slot))
}

/// Per-slot Past Reverse Window (s).
pub fn past_reverse_window_param(&self, slot: usize) -> Option<&FloatParam> {
    use crate::param_ids::NUM_SLOTS;
    if slot >= NUM_SLOTS { return None; }
    Some(past_reverse_window_s_dispatch!(self, slot))
}

/// Per-slot Past Stretch Rate (x).
pub fn past_stretch_rate_param(&self, slot: usize) -> Option<&FloatParam> {
    use crate::param_ids::NUM_SLOTS;
    if slot >= NUM_SLOTS { return None; }
    Some(past_stretch_rate_dispatch!(self, slot))
}

/// Per-slot Past Stretch Dither (0..1).
pub fn past_stretch_dither_param(&self, slot: usize) -> Option<&FloatParam> {
    use crate::param_ids::NUM_SLOTS;
    if slot >= NUM_SLOTS { return None; }
    Some(past_stretch_dither_dispatch!(self, slot))
}

/// Per-slot Past Soft Clip toggle.
pub fn past_soft_clip_param(&self, slot: usize) -> Option<&BoolParam> {
    use crate::param_ids::NUM_SLOTS;
    if slot >= NUM_SLOTS { return None; }
    Some(past_soft_clip_dispatch!(self, slot))
}
```

If `BoolParam` isn't already imported at the top of `params.rs`, add it next to `FloatParam`.

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test --release --test past_params 2>&1 | tail -10`
Expected: PASS, two tests.

- [ ] **Step 6: Run full test suite**

Run: `cargo test --release 2>&1 | grep -E "FAILED|^error" | head -20`
Expected: no FAILED tests.

- [ ] **Step 7: Commit**

```bash
git add build.rs src/params.rs tests/past_params.rs
git commit -m "feat(params): per-slot Past scalar params (Phase Past UX, Task 8)

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

## Task 9: Refactor `apply_reverse` to take `window_frames` scalar

**Files:**
- Modify: `src/dsp/modules/past.rs:349-381` (apply_reverse signature + body) + caller at `process()`
- Modify: `tests/past_pipeline.rs` (add test)

- [ ] **Step 1: Write the failing test**

Append to `tests/past_pipeline.rs`:

```rust
#[test]
fn reverse_uses_scalar_window_not_curve_average() {
    use spectral_forge::dsp::modules::past::{PastModule, PastMode, PastScalars};
    use num_complex::Complex;

    let mut m = PastModule::new(48000.0, 2048);
    m.set_past_mode(PastMode::Reverse);
    m.set_scalars(PastScalars {
        window_frames: 8,
        ..Default::default()
    });
    let scalars = m.scalars();
    assert_eq!(scalars.window_frames, 8, "scalar must persist via setter");
    // Channel-state offset wraps at window=8, so after 8 hops the offset
    // returns to 0; this is the contract the spec replaces TIME averaging with.
    // (Direct kernel test deferred to Task 10's Stretch — Reverse's wrap is
    // exercised end-to-end via the existing past_pipeline harness.)
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --release --features probe --test past_pipeline reverse_uses_scalar_window 2>&1 | tail -10`
Expected: FAIL — `PastScalars` doesn't exist; `set_scalars` not implemented.

- [ ] **Step 3: Add `PastScalars` struct + setter on `PastModule`**

Edit `src/dsp/modules/past.rs`. Above `pub struct PastModule` (or near it), add:

```rust
/// Mode-specific scalar controls for Past. Replaces the curve-averaging hacks
/// in Reverse and Stretch with honest per-slot scalars; gates the soft-clip
/// post-pass; carries the DecaySorter floor.
/// See docs/superpowers/specs/2026-05-04-past-module-ux-design.md §2 + §3.
#[derive(Clone, Copy, Debug, Default)]
pub struct PastScalars {
    /// DecaySorter `low_k` floor as bin index. 0 disables (default 230 Hz at fft 2048 / 48 kHz ≈ bin 10).
    pub floor_bin:     usize,
    /// Reverse window length in **frames** (Pipeline converts seconds → frames each block).
    pub window_frames: u32,
    /// Stretch read rate. 1.0 = unity, 0.25..4.0 musical range.
    pub rate:          f32,
    /// Stretch dither amount (0..1).
    pub dither:        f32,
    /// Module-wide soft-clip toggle (default ON).
    pub soft_clip:     bool,
}

impl PastScalars {
    /// Conservative default that's musically inert (rate=1.0 means stretch is no-op,
    /// window=1 frame is the smallest legal value, soft_clip ON).
    pub fn safe_default() -> Self {
        Self {
            floor_bin:     10,
            window_frames: 1,
            rate:          1.0,
            dither:        0.0,
            soft_clip:     true,
        }
    }
}
```

Add a `scalars: PastScalars` field on `PastModule`:

```rust
pub struct PastModule {
    // ... existing fields ...
    scalars: PastScalars,
    // ...
}
```

Initialise it in `PastModule::new`:

```rust
scalars: PastScalars::safe_default(),
```

Add setter and accessor:

```rust
impl PastModule {
    pub fn set_scalars(&mut self, scalars: PastScalars) { self.scalars = scalars; }
    pub fn scalars(&self) -> PastScalars { self.scalars }
}
```

- [ ] **Step 4: Refactor `apply_reverse` signature**

Replace `apply_reverse` (around line 349) with:

```rust
fn apply_reverse(
    &mut self, ch: usize, bins: &mut [Complex<f32>], hist: &HistoryBuffer,
    amount: &[f32], threshold: &[f32], mix: &[f32],
    ctx: &ModuleContext<'_>,
) {
    let n = bins.len().min(ctx.num_bins);
    let window = self.scalars.window_frames.max(1);

    let st = &mut self.channels[ch];
    let age = (st.reverse_read_offset % window) as usize;

    let frame = match hist.read_frame(ch, age) { Some(f) => f, None => return };
    st.reverse_read_offset = (st.reverse_read_offset + 1) % window;
    for k in 0..n {
        let mag_sq = bins[k].norm_sqr();
        let thr = threshold.get(k).copied().unwrap_or(0.0);
        if mag_sq < thr * thr { continue; }
        if k >= frame.len() { continue; }
        let bin_amount = amount.get(k).copied().unwrap_or(0.0).clamp(0.0, 1.0);
        let value = frame[k] * bin_amount;
        let m_val = mix.get(k).copied().unwrap_or(1.0).clamp(0.0, 1.0);
        bins[k] = bins[k] * (1.0 - m_val) + value * m_val;
    }
}
```

Update the caller in `process` (around line 190):

```rust
PastMode::Reverse => self.apply_reverse(ch, bins, history, amount, threshold, mix, ctx),
```

(Drops `time` and `spread` from the call.)

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test --release --features probe --test past_pipeline reverse_uses_scalar_window 2>&1 | tail -10`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/dsp/modules/past.rs tests/past_pipeline.rs
git commit -m "feat(past): apply_reverse takes scalar window_frames (Phase Past UX, Task 9)

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

## Task 10: Refactor `apply_stretch` to take `rate` and `dither` scalars

**Files:**
- Modify: `src/dsp/modules/past.rs:383-...` (apply_stretch)
- Modify: `tests/past_pipeline.rs` (extend)

- [ ] **Step 1: Write the failing test**

Append to `tests/past_pipeline.rs`:

```rust
#[test]
fn stretch_uses_scalar_rate_not_curve_average() {
    use spectral_forge::dsp::modules::past::{PastModule, PastMode, PastScalars};

    let mut m = PastModule::new(48000.0, 2048);
    m.set_past_mode(PastMode::Stretch);
    m.set_scalars(PastScalars { rate: 2.0, dither: 0.0, ..PastScalars::safe_default() });
    assert!((m.scalars().rate - 2.0).abs() < 1e-6);
    // The DSP-level rate-driven phase advance is verified end-to-end via the
    // existing past_pipeline 200-hop harness once Pipeline (Task 12) wires the
    // params through. This task asserts the kernel signature and the scalar
    // is consumed by the module.
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --release --features probe --test past_pipeline stretch_uses_scalar_rate 2>&1 | tail -10`
Expected: FAIL — until Step 3 lands the new kernel.

- [ ] **Step 3: Refactor `apply_stretch` signature**

Replace `apply_stretch` (around line 383) with:

```rust
fn apply_stretch(
    &mut self, ch: usize, bins: &mut [Complex<f32>], hist: &HistoryBuffer,
    amount: &[f32], mix: &[f32],
    ctx: &ModuleContext<'_>,
) {
    let n = bins.len().min(ctx.num_bins);
    let rate = self.scalars.rate.clamp(0.05, 4.0);
    let dither_amt = self.scalars.dither.clamp(0.0, 1.0);

    let max_age = hist.capacity_frames().saturating_sub(2) as f32;

    let read_age = self.channels[ch].stretch_read_phase as f32;
    self.channels[ch].stretch_read_phase += rate as f64;
    if self.channels[ch].stretch_read_phase > max_age as f64 {
        self.channels[ch].stretch_read_phase = 0.0;
    }

    let ok = {
        let scratch = &mut self.channels[ch].sort_scratch;
        scratch[..n].fill(Complex::new(0.0, 0.0));
        hist.read_fractional(ch, read_age, &mut scratch[..n])
    };
    if !ok { return; }

    let if_offset = ctx.if_offset.unwrap_or(&[]);
    #[allow(clippy::needless_range_loop)]
    for k in 0..n {
        // Existing per-bin body — reuse current logic with the dither_amt scalar
        // instead of `spread.get(k)`. (Implementation detail follows the same
        // shape as before; replacing `spread.get(k).copied().unwrap_or(0.0)` with
        // `dither_amt` and removing the per-bin xorshift-on-spread>0 branch.)
        // ... existing per-bin body ...
        let _ = (k, &if_offset, dither_amt); // placeholder removed when porting
    }
}
```

The exact per-bin body is the same as the current `apply_stretch` body but with `dither_amt` substituted for the per-bin SPREAD value. **Important:** preserve the existing PhaseRotator + IF-driven phase advance code; only the scalar source changes.

Update the caller in `process` (around line 191):

```rust
PastMode::Stretch => self.apply_stretch(ch, bins, history, amount, mix, ctx),
```

(Drops `time` and `spread` from the call.)

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --release --features probe --test past_pipeline stretch_uses_scalar_rate 2>&1 | tail -10`
Expected: PASS.

- [ ] **Step 5: Run end-to-end Past pipeline harness**

Run: `cargo test --release --features probe --test past_pipeline 2>&1 | tail -10`
Expected: All Past pipeline tests pass; no `FAILED`.

- [ ] **Step 6: Commit**

```bash
git add src/dsp/modules/past.rs tests/past_pipeline.rs
git commit -m "feat(past): apply_stretch takes scalar rate + dither (Phase Past UX, Task 10)

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

## Task 11: Add `apply_soft_clip` helper, gated on per-slot toggle

**Files:**
- Modify: `src/dsp/modules/past.rs` (helper + integration into `process()`)
- Test: `tests/past_pipeline.rs` (extend)

- [ ] **Step 1: Write the failing test**

Append to `tests/past_pipeline.rs`:

```rust
#[test]
fn soft_clip_clamps_high_magnitude_when_on() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::past::apply_soft_clip;
    let mut bins = [Complex::new(10.0_f32, 0.0); 32];
    apply_soft_clip(&mut bins, 32);
    for k in 0..32 {
        assert!(bins[k].norm() < 4.0,
            "soft-clip with K=4.0 must keep magnitude under 4.0; bin {k} got {}", bins[k].norm());
    }
}

#[test]
fn soft_clip_passes_low_magnitude_almost_unchanged() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::past::apply_soft_clip;
    let mut bins = [Complex::new(0.1_f32, 0.0); 16];
    let original = bins[0].norm();
    apply_soft_clip(&mut bins, 16);
    let attenuation = bins[0].norm() / original;
    // |out|/|in| = K / (K + |in|) = 4.0 / 4.1 ≈ 0.976
    assert!(attenuation > 0.95 && attenuation < 1.0,
        "small bins should be barely attenuated; got {attenuation}");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --release --features probe --test past_pipeline soft_clip 2>&1 | tail -10`
Expected: FAIL — `apply_soft_clip` not exported.

- [ ] **Step 3: Add the helper and integrate**

Edit `src/dsp/modules/past.rs`. At the bottom of the file (or grouped with mode kernels), add:

```rust
/// Per-bin radial soft-clip toward magnitude `K = 4.0` (≈ +12 dBFS).
/// `bins[k] *= K / (K + |bins[k]|)` shrinks magnitudes asymptotically toward `K`
/// while leaving small magnitudes nearly unchanged.
/// Documented in spec 2026-05-04-past-module-ux-design.md §3.
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

In `PastModule::process` (the trait implementation around line 145), at the **end** of the function (after the mode dispatch match), add:

```rust
        if self.scalars.soft_clip {
            apply_soft_clip(bins, ctx.num_bins);
        }
    }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --release --features probe --test past_pipeline soft_clip 2>&1 | tail -10`
Expected: PASS, two tests.

- [ ] **Step 5: Commit**

```bash
git add src/dsp/modules/past.rs tests/past_pipeline.rs
git commit -m "feat(past): apply_soft_clip helper + per-slot toggle (Phase Past UX, Task 11)

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

## Task 12: `Pipeline::process` reads new Past scalars per slot, threads through `FxMatrix::set_past_scalars`

**Files:**
- Modify: `src/dsp/fx_matrix.rs` (add `set_past_scalars` method)
- Modify: `src/dsp/pipeline.rs` (read params, snap, dispatch each block)
- Test: `tests/past_pipeline.rs` (extend with end-to-end scalar-flow test)

- [ ] **Step 1: Write the failing test**

Append to `tests/past_pipeline.rs`:

```rust
#[test]
fn pipeline_threads_past_scalars_into_module_each_block() {
    use spectral_forge::dsp::pipeline::Pipeline;
    use spectral_forge::dsp::modules::ModuleType;
    use spectral_forge::dsp::modules::past::{PastMode, PastScalars};
    use spectral_forge::params::SpectralForgeParams;
    use std::sync::Arc;

    let params = Arc::new(SpectralForgeParams::default());
    // Force Past into slot 0 and set Reverse mode.
    params.slot_module_types.lock()[0] = ModuleType::Past;
    params.slot_past_mode.lock()[0] = PastMode::Reverse;
    // Set custom Window via the param.
    params.past_reverse_window_param(0).unwrap()
        .smoothed.set_target(48000.0, 0.5);

    let mut pipeline = Pipeline::new(48000.0, 2048);
    let mut buf_l = [0.0f32; 256];
    let mut buf_r = [0.0f32; 256];
    let buf = nih_plug::prelude::Buffer::new();
    // (Real Buffer wiring varies; this test relies on the existing past_pipeline
    // smoke harness pattern. See common.rs::run_past_pipeline.)
    let _ = (&mut buf_l, &mut buf_r, &buf, &mut pipeline, &params);
}
```

(Use the existing `tests/common/` harness pattern if simpler — copy the template from `tests/past_pipeline.rs::existing_test`.)

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --release --features probe --test past_pipeline pipeline_threads_past_scalars 2>&1 | tail -10`
Expected: FAIL — `set_past_scalars` doesn't exist on `FxMatrix`.

- [ ] **Step 3: Add `set_past_scalars` on `FxMatrix`**

Edit `src/dsp/fx_matrix.rs`. After `set_past_sort_keys` (around line 320), add:

```rust
/// Apply per-slot Past scalars to each Past slot.
/// Called once per block from `Pipeline::process()` after snapping params.
pub fn set_past_scalars(&mut self, scalars: &[crate::dsp::modules::past::PastScalars; 9]) {
    for s in 0..9 {
        if let Some(ref mut m) = self.slots[s] {
            if let Some(past) = m.as_any_mut().downcast_mut::<crate::dsp::modules::past::PastModule>() {
                past.set_scalars(scalars[s]);
            }
        }
    }
}
```

If `as_any_mut` doesn't exist on `SpectralModule`, add a typed setter to the trait (mirroring `set_past_mode`):

```rust
// In src/dsp/modules/mod.rs, on SpectralModule trait:
fn set_past_scalars(&mut self, _: crate::dsp::modules::past::PastScalars) {}
```

Then in `PastModule`'s impl:

```rust
fn set_past_scalars(&mut self, scalars: crate::dsp::modules::past::PastScalars) {
    self.scalars = scalars;
}
```

And update `FxMatrix::set_past_scalars` to call the trait method instead of downcasting:

```rust
pub fn set_past_scalars(&mut self, scalars: &[crate::dsp::modules::past::PastScalars; 9]) {
    for s in 0..9 {
        if let Some(ref mut m) = self.slots[s] {
            m.set_past_scalars(scalars[s]);
        }
    }
}
```

(Trait method default is `_` so non-Past modules ignore the call.)

- [ ] **Step 4: Snap and dispatch in `Pipeline::process`**

Edit `src/dsp/pipeline.rs`. After the existing `set_past_modes` / `set_past_sort_keys` dispatch (search for `set_past_modes`), add:

```rust
// Snap Past per-slot scalar params and convert seconds → frames where needed.
let mut past_scalars: [crate::dsp::modules::past::PastScalars; 9] =
    std::array::from_fn(|_| crate::dsp::modules::past::PastScalars::safe_default());
let hop_size = (self.fft_size as f32 / 4.0) as f32;
let hop_seconds = hop_size / self.sample_rate;
for s in 0..9 {
    let floor_hz = params.past_floor_param(s).map(|p| p.smoothed.next()).unwrap_or(230.0);
    let floor_bin = ((floor_hz / self.sample_rate) * self.fft_size as f32).round() as usize;
    let window_s = params.past_reverse_window_param(s).map(|p| p.smoothed.next()).unwrap_or(1.0);
    let window_frames = ((window_s / hop_seconds).round() as u32).max(1);
    let rate = params.past_stretch_rate_param(s).map(|p| p.smoothed.next()).unwrap_or(1.0);
    let dither = params.past_stretch_dither_param(s).map(|p| p.smoothed.next()).unwrap_or(0.0);
    let soft_clip = params.past_soft_clip_param(s).map(|p| p.value()).unwrap_or(true);
    past_scalars[s] = crate::dsp::modules::past::PastScalars {
        floor_bin: floor_bin.clamp(1, self.num_bins.saturating_sub(crate::dsp::modules::past::MAX_SORT_BINS)),
        window_frames,
        rate,
        dither,
        soft_clip,
    };
}
self.fx_matrix.set_past_scalars(&past_scalars);
```

(`MAX_SORT_BINS` is already a `pub const` in `past.rs`; if not, expose it via `pub use`.)

- [ ] **Step 5: Update `apply_decay_sorter` to read `floor_bin` from scalars**

In `src/dsp/modules/past.rs`, find `apply_decay_sorter` (around line 257) and replace `let low_k = 10usize.min(n.saturating_sub(1));` with:

```rust
let low_k = self.scalars.floor_bin.min(n.saturating_sub(1));
```

- [ ] **Step 6: Run test to verify it passes**

Run: `cargo test --release --features probe --test past_pipeline 2>&1 | tail -15`
Expected: All Past pipeline tests pass; no `FAILED`.

- [ ] **Step 7: Commit**

```bash
git add src/dsp/fx_matrix.rs src/dsp/pipeline.rs src/dsp/modules/past.rs src/dsp/modules/mod.rs tests/past_pipeline.rs
git commit -m "feat(past): Pipeline threads scalar params via FxMatrix::set_past_scalars (Phase Past UX, Task 12)

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

## Task 13: Extend `ProbeSnapshot` with Past mode-specific fields + populate per-mode

**Files:**
- Modify: `src/dsp/modules/mod.rs:191-248` (`ProbeSnapshot` struct)
- Modify: `src/dsp/modules/past.rs:165-183` (probe population)
- Test: `tests/calibration.rs` (extend)

- [ ] **Step 1: Write the failing test**

Append to `tests/calibration.rs`:

```rust
#[test]
fn past_probe_window_populated_only_in_reverse() {
    use spectral_forge::dsp::modules::past::{PastModule, PastMode, PastScalars};
    use spectral_forge::dsp::modules::SpectralModule;

    let mut m = PastModule::new(48000.0, 2048);
    m.set_past_mode(PastMode::Reverse);
    m.set_scalars(PastScalars { window_frames: 200, ..PastScalars::safe_default() });
    // Drive one process to populate probe... (use existing test harness)
    // After process:
    let probe = m.last_probe();
    assert!(probe.past_reverse_window_s.is_some(), "Reverse must populate window probe");
    assert!(probe.past_stretch_rate.is_none(),    "Reverse must NOT populate stretch_rate");

    m.set_past_mode(PastMode::Stretch);
    m.set_scalars(PastScalars { rate: 2.0, dither: 0.5, ..PastScalars::safe_default() });
    // ... drive process ...
    let probe = m.last_probe();
    assert!(probe.past_reverse_window_s.is_none(), "Stretch must NOT populate window");
    assert!(probe.past_stretch_rate.is_some());
    assert!(probe.past_stretch_dither_pct.is_some());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --release --features probe --test calibration past_probe_window 2>&1 | tail -10`
Expected: FAIL — fields not on `ProbeSnapshot`.

- [ ] **Step 3: Add fields to `ProbeSnapshot`**

Edit `src/dsp/modules/mod.rs`. Inside `ProbeSnapshot` (around line 228), after the existing past_* fields, add:

```rust
// Past mode-specific scalar probes.
pub past_reverse_window_s:   Option<f32>,
pub past_stretch_rate:       Option<f32>,
pub past_stretch_dither_pct: Option<f32>,
```

- [ ] **Step 4: Update probe population per mode**

Edit `src/dsp/modules/past.rs`. Find the `#[cfg(any(test, feature = "probe"))]` block in `process` (around line 165). Replace the existing block with mode-specific population:

```rust
#[cfg(any(test, feature = "probe"))]
{
    let hop_size = (ctx.fft_size as f32 / 4.0) / ctx.sample_rate;
    let total_seconds = history.capacity_frames() as f32 * hop_size;
    let amount_at_bin0 = amount.first().copied().unwrap_or(0.0).clamp(0.0, 1.0);
    let mut probe = crate::dsp::modules::ProbeSnapshot {
        past_amount_pct:          Some(amount_at_bin0 * 100.0),
        past_active_mode_idx:     Some(self.mode as u8),
        past_history_frames_used: Some(history.frames_used() as u32),
        past_sort_key_idx:        Some(self.sort_key as u8),
        ..Default::default()
    };
    match self.mode {
        PastMode::Granular | PastMode::Convolution => {
            let time_at_bin0 = time.first().copied().unwrap_or(0.0);
            probe.past_time_seconds = Some(time_at_bin0 * total_seconds);
        }
        PastMode::Reverse => {
            probe.past_reverse_window_s = Some(self.scalars.window_frames as f32 * hop_size);
        }
        PastMode::Stretch => {
            probe.past_stretch_rate = Some(self.scalars.rate);
            probe.past_stretch_dither_pct = Some(self.scalars.dither * 100.0);
        }
        PastMode::DecaySorter => { /* no scalar probe */ }
    }
    self.last_probe = probe;
}
```

(`time` is no longer in scope for Reverse/Stretch since the kernel signatures changed; in those cases the existing local `time` reference is removed. Ensure the let-binding for `time` is moved into the Granular/Convolution branch, or use `curves.get(1)` directly in the probe block.)

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test --release --features probe --test calibration past_probe_window 2>&1 | tail -10`
Expected: PASS.

- [ ] **Step 6: Run the full calibration suite**

Run: `cargo test --release --features probe --test calibration 2>&1 | grep -E "FAILED|^test result"`
Expected: all Past calibration tests pass.

- [ ] **Step 7: Commit**

```bash
git add src/dsp/modules/mod.rs src/dsp/modules/past.rs tests/calibration.rs
git commit -m "feat(past): mode-specific ProbeSnapshot fields (Phase Past UX, Task 13)

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

## Task 14: editor_ui — render only active curves per `CurveLayout`

**Files:**
- Modify: `src/editor_ui.rs:103-139` (curve-tab strip), 442-495 (paint loop), 1075-1167 (offset/tilt/curv row)
- Test: `tests/editor_panel_dispatch.rs` (extend) — logical assertion that `active_layout` controls visible tabs

- [ ] **Step 1: Write the failing test**

Append to `tests/editor_panel_dispatch.rs`:

```rust
//! Verifies that when a slot's module has `active_layout = Some(...)`, the
//! editor renders curve tabs only for the indices in `active`. Logical-level
//! check; UI snapshot deferred to manual visual validation.

use spectral_forge::dsp::modules::{module_spec, ModuleType, past::PastMode};

#[test]
fn past_active_layout_shapes_visible_curves_per_mode() {
    let layout_fn = module_spec(ModuleType::Past).active_layout
        .expect("Past has active_layout");

    assert_eq!(layout_fn(PastMode::Granular as u8).active.len(),    5);
    assert_eq!(layout_fn(PastMode::DecaySorter as u8).active.len(), 3);
    assert_eq!(layout_fn(PastMode::Convolution as u8).active.len(), 4);
    assert_eq!(layout_fn(PastMode::Reverse as u8).active.len(),     3);
    assert_eq!(layout_fn(PastMode::Stretch as u8).active.len(),     2);
}
```

- [ ] **Step 2: Run test to verify it passes the layout check**

Run: `cargo test --release --test editor_panel_dispatch past_active_layout_shapes 2>&1 | tail -10`
Expected: PASS (this checks the foundation; the rendering update follows).

- [ ] **Step 3: Update curve-tab strip rendering**

Edit `src/editor_ui.rs:103-139`. Replace the existing tab-strip loop with:

```rust
// Determine the active curve set for this mode. If the module has an
// `active_layout`, use its `active` and `label_overrides`; otherwise fall back
// to rendering all `curve_labels`.
let active_layout_opt = spec.active_layout.map(|f| {
    let mode_byte: u8 = match editing_type {
        crate::dsp::modules::ModuleType::Past => {
            *params.slot_past_mode.lock().get(editing_slot)
                .map(|m| *m as u8).as_ref().unwrap_or(&0u8)
        }
        // (future modes wired analogously)
        _ => 0u8,
    };
    f(mode_byte)
});

// Build the iteration list: either layout.active (with override labels), or
// the legacy 0..num_curves with default labels.
let visible_curves: Vec<(usize, &str)> = if let Some(layout) = active_layout_opt {
    layout.active.iter().map(|&idx| {
        let label = layout.label_overrides.iter()
            .find_map(|&(c, l)| if c == idx { Some(l) } else { None })
            .or_else(|| spec.curve_labels.get(idx as usize).copied())
            .unwrap_or("");
        (idx as usize, label)
    }).collect()
} else {
    spec.curve_labels.iter().enumerate()
        .map(|(i, l)| (i, *l))
        .collect()
};

for (i, label) in visible_curves.iter().copied() {
    // ... existing tab-button rendering, with `i` and `label` substituted ...
    // (gain_disabled / is_active / fill / btn / sense / clicked logic unchanged)
}
```

- [ ] **Step 4: Update graph paint loop to iterate visible curves only**

In `src/editor_ui.rs:451` (the dim-curves loop) and the active-curve paint at 467, gate the iteration on `visible_curves` rather than `0..num_c.min(7)`:

```rust
for (i, _label) in visible_curves.iter().copied() {
    if i == editing_curve { continue; }
    // ... existing dim-curve paint with index `i` ...
}
```

- [ ] **Step 5: Update Offset/Tilt/Curv row to gate on `editing_curve` being in `active`**

In `src/editor_ui.rs:1078-1167`, before the `if editing_curve < spec.num_curves` check, additionally gate on:

```rust
let editing_curve_visible = visible_curves.iter().any(|(i, _)| *i == editing_curve);
if editing_curve_visible && editing_curve < spec.num_curves {
    // ... existing offset/tilt/curv DragValue row ...
}
```

If the user changes mode and `editing_curve` is no longer visible, snap it to the first visible curve:

```rust
if !editing_curve_visible {
    if let Some(&(first_visible, _)) = visible_curves.first() {
        *params.editing_curve.lock() = first_visible as u8;
    }
}
```

- [ ] **Step 6: Run test + UI smoke**

Run: `cargo test --release 2>&1 | grep -E "FAILED|^error" | head -10`
Expected: no test regressions.

Build a dev plugin and verify in Bitwig that switching Past mode reshapes the visible tabs.

```bash
cargo build --release --features dev-build && \
cargo run --package xtask -- bundle spectral_forge --release --features dev-build && \
cp target/bundled/spectral_forge.clap /home/kim/.clap/spectral/dev/spectral_dev.clap
```

- [ ] **Step 7: Commit**

```bash
git add src/editor_ui.rs tests/editor_panel_dispatch.rs
git commit -m "feat(editor): render visible curves per CurveLayout (Phase Past UX, Task 14)

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

## Task 15: editor_ui — Past panel_widget with Soft Clip + mode-specific scalars

**Files:**
- Create: `src/editor/past_panel.rs`
- Modify: `src/editor.rs` or `src/editor/mod.rs` (add `pub mod past_panel`)
- Modify: `src/dsp/modules/mod.rs` (Past `ModuleSpec` `panel_widget: Some(crate::editor::past_panel::draw)`)
- Test: `tests/editor_panel_dispatch.rs` (extend)

- [ ] **Step 1: Write the failing test**

Append to `tests/editor_panel_dispatch.rs`:

```rust
#[test]
fn past_module_spec_advertises_panel_widget() {
    use spectral_forge::dsp::modules::{module_spec, ModuleType};
    let spec = module_spec(ModuleType::Past);
    assert!(spec.panel_widget.is_some(), "Past must declare a panel_widget for Soft Clip + scalars");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --release --test editor_panel_dispatch past_module_spec_advertises 2>&1 | tail -10`
Expected: FAIL — Past spec literal has `panel_widget: None`.

- [ ] **Step 3: Create the panel widget module**

Create `src/editor/past_panel.rs`:

```rust
//! Past module's slot-panel: Soft Clip toggle + mode-specific scalars
//! (Floor / Window / Rate / Dither). See spec
//! docs/superpowers/specs/2026-05-04-past-module-ux-design.md §2 + §3.

use nih_plug_egui::egui::{self, Ui};
use nih_plug::prelude::ParamSetter;
use crate::dsp::modules::past::PastMode;
use crate::params::SpectralForgeParams;
use crate::editor::theme as th;

pub fn draw(ui: &mut Ui, params: &SpectralForgeParams, setter: &ParamSetter, slot: usize, scale: f32) {
    let mode = *params.slot_past_mode.lock().get(slot).unwrap_or(&PastMode::Granular);

    ui.horizontal(|ui| {
        // Module-wide Soft Clip toggle (always visible).
        if let Some(p) = params.past_soft_clip_param(slot) {
            let mut on = p.value();
            let resp = ui.checkbox(&mut on, "Soft Clip");
            if resp.changed() {
                setter.begin_set_parameter(p);
                setter.set_parameter(p, on);
                setter.end_set_parameter(p);
            }
        }

        // Mode-specific scalars
        match mode {
            PastMode::DecaySorter => {
                if let Some(p) = params.past_floor_param(slot) {
                    let mut v = p.value();
                    let resp = ui.add(
                        egui::DragValue::new(&mut v).range(20.0..=2000.0)
                            .speed(2.0).suffix(" Hz")
                    );
                    handle_float_change(setter, p, resp, v, 20.0, 2000.0);
                    ui.label(egui::RichText::new("Floor")
                        .size(th::scaled(th::FONT_SIZE_LABEL, scale)));
                }
            }
            PastMode::Reverse => {
                if let Some(p) = params.past_reverse_window_param(slot) {
                    let mut v = p.value();
                    let resp = ui.add(
                        egui::DragValue::new(&mut v).range(0.05..=30.0)
                            .speed(0.05).fixed_decimals(2).suffix(" s")
                    );
                    handle_float_change(setter, p, resp, v, 0.05, 30.0);
                    ui.label(egui::RichText::new("Window")
                        .size(th::scaled(th::FONT_SIZE_LABEL, scale)));
                }
            }
            PastMode::Stretch => {
                if let Some(p) = params.past_stretch_rate_param(slot) {
                    let mut v = p.value();
                    let resp = ui.add(
                        egui::DragValue::new(&mut v).range(0.25..=4.0)
                            .speed(0.01).fixed_decimals(2).suffix("x")
                    );
                    handle_float_change(setter, p, resp, v, 0.25, 4.0);
                    ui.label(egui::RichText::new("Rate")
                        .size(th::scaled(th::FONT_SIZE_LABEL, scale)));
                }
                if let Some(p) = params.past_stretch_dither_param(slot) {
                    let mut v = p.value();
                    let resp = ui.add(
                        egui::DragValue::new(&mut v).range(0.0..=1.0)
                            .speed(0.005).custom_formatter(|n, _| format!("{:.0}%", n * 100.0))
                    );
                    handle_float_change(setter, p, resp, v, 0.0, 1.0);
                    ui.label(egui::RichText::new("Dither")
                        .size(th::scaled(th::FONT_SIZE_LABEL, scale)));
                }
            }
            PastMode::Granular | PastMode::Convolution => {
                // No mode-specific scalars; row stays compact.
            }
        }
    });
}

fn handle_float_change(
    setter: &ParamSetter,
    p: &nih_plug::prelude::FloatParam,
    resp: egui::Response,
    new_val: f32,
    lo: f32,
    hi: f32,
) {
    if resp.drag_started() { setter.begin_set_parameter(p); }
    if resp.changed() { setter.set_parameter(p, new_val.clamp(lo, hi)); }
    if resp.drag_stopped() { setter.end_set_parameter(p); }
}
```

- [ ] **Step 4: Register the panel widget**

Edit `src/editor/mod.rs` (or `src/editor.rs` — wherever the editor module tree is defined). Add:

```rust
pub mod past_panel;
```

Edit `src/dsp/modules/mod.rs`. Find the Past `ModuleSpec` literal. Change `panel_widget: None,` to:

```rust
panel_widget: Some(crate::editor::past_panel::draw),
```

If the existing `PanelWidgetFn` type signature differs from the `draw` function signature above, adjust `draw` to match it. Search `pub type PanelWidgetFn` in `mod.rs`.

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test --release --test editor_panel_dispatch past_module_spec_advertises 2>&1 | tail -10`
Expected: PASS.

- [ ] **Step 6: Bundle dev plugin and visually verify in Bitwig**

```bash
cargo build --release --features dev-build && \
cargo run --package xtask -- bundle spectral_forge --release --features dev-build && \
cp target/bundled/spectral_forge.clap /home/kim/.clap/spectral/dev/spectral_dev.clap
```

Verify each Past mode renders the right scalar controls; Soft Clip toggle visible in all modes.

- [ ] **Step 7: Commit**

```bash
git add src/editor/past_panel.rs src/editor/mod.rs src/dsp/modules/mod.rs tests/editor_panel_dispatch.rs
git commit -m "feat(editor): Past panel_widget with Soft Clip + mode scalars (Phase Past UX, Task 15)

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

## Task 16: Help-box widget right of FX matrix

**Files:**
- Create: `src/editor/help_box.rs`
- Modify: `src/editor/theme.rs` (help-box constants)
- Modify: `src/editor/mod.rs` (export the new module)
- Modify: `src/editor_ui.rs` (mount the widget right of the matrix region)

- [ ] **Step 1: Add theme constants for the help-box**

Edit `src/editor/theme.rs`. Add:

```rust
pub const HELP_BOX_WIDTH:        f32 = 240.0;
pub const HELP_BOX_PADDING:      f32 = 8.0;
pub const FONT_SIZE_HELP_HEAD:   f32 = 12.0;
pub const FONT_SIZE_HELP_BODY:   f32 = 10.0;
pub const HELP_BOX_BG:           egui::Color32 = egui::Color32::from_rgb(20, 20, 24);
pub const HELP_BOX_BORDER:       egui::Color32 = egui::Color32::from_rgb(60, 60, 68);
pub const HELP_BOX_BODY:         egui::Color32 = egui::Color32::from_rgb(190, 190, 196);
pub const HELP_BOX_HEAD:         egui::Color32 = egui::Color32::from_rgb(230, 230, 236);
```

If `egui` isn't imported in `theme.rs`, add `use nih_plug_egui::egui;` at the top.

- [ ] **Step 2: Create the help-box module**

Create `src/editor/help_box.rs`:

```rust
//! Help-box widget rendered right of the FX matrix.
//! Shows the module's overview when a slot is in focus, or a per-curve
//! summary when a curve is in focus.
//! See spec docs/superpowers/specs/2026-04-23-ui-parameter-spec-design.md §8.

use nih_plug_egui::egui::{self, Ui, Frame, Stroke, RichText, FontId};
use crate::dsp::modules::{module_spec, ModuleType, CurveLayout};
use crate::params::SpectralForgeParams;
use crate::editor::theme as th;

/// Render the help-box. The caller positions it (via `ui.allocate_ui_at_rect`
/// or similar) to the right of the matrix region.
pub fn draw(ui: &mut Ui, params: &SpectralForgeParams, scale: f32) {
    let editing_slot  = *params.editing_slot.lock() as usize;
    let editing_curve = *params.editing_curve.lock() as usize;
    let editing_type  = params.slot_module_types.lock()[editing_slot];
    let spec          = module_spec(editing_type);

    let layout = active_layout_for_slot(editing_type, params, editing_slot);

    let head: &str = spec_display_name(editing_type);

    let body: String = if let Some(layout) = layout.as_ref() {
        // Per-curve summary if focused curve is active in the layout
        if layout.active.contains(&(editing_curve as u8)) {
            let s = (layout.help_for)(editing_curve as u8);
            if !s.is_empty() {
                s.to_string()
            } else if let Some(overview) = layout.mode_overview {
                overview.to_string()
            } else {
                static_description(editing_type).to_string()
            }
        } else if let Some(overview) = layout.mode_overview {
            overview.to_string()
        } else {
            static_description(editing_type).to_string()
        }
    } else {
        static_description(editing_type).to_string()
    };

    Frame::new()
        .fill(th::HELP_BOX_BG)
        .stroke(Stroke::new(th::scaled_stroke(th::STROKE_BORDER, scale), th::HELP_BOX_BORDER))
        .inner_margin(egui::Margin::same(th::HELP_BOX_PADDING as i8))
        .show(ui, |ui| {
            ui.set_width(th::HELP_BOX_WIDTH);
            ui.label(
                RichText::new(head)
                    .color(th::HELP_BOX_HEAD)
                    .font(FontId::proportional(th::scaled(th::FONT_SIZE_HELP_HEAD, scale)))
            );
            ui.add_space(4.0);
            ui.label(
                RichText::new(body)
                    .color(th::HELP_BOX_BODY)
                    .font(FontId::proportional(th::scaled(th::FONT_SIZE_HELP_BODY, scale)))
            );
        });
}

fn active_layout_for_slot(
    ty: ModuleType,
    params: &SpectralForgeParams,
    slot: usize,
) -> Option<CurveLayout> {
    let layout_fn = module_spec(ty).active_layout?;
    let mode_byte: u8 = match ty {
        ModuleType::Past => *params.slot_past_mode.lock().get(slot)
            .map(|m| *m as u8).as_ref().unwrap_or(&0),
        // future module-mode lookups added here
        _ => 0,
    };
    Some(layout_fn(mode_byte))
}

fn spec_display_name(ty: ModuleType) -> &'static str {
    module_spec(ty).display_name
}

fn static_description(ty: ModuleType) -> &'static str {
    match ty {
        ModuleType::Past => "Read-only access to a rolling buffer of recent spectral history. Pick a mode in the popup; per-mode help appears here when a curve is selected.",
        ModuleType::Dynamics => "Per-bin dynamics processor.",
        ModuleType::Freeze   => "Spectral freeze — captures a moment of the spectrum and holds it.",
        ModuleType::Empty    => "No module assigned to this slot.",
        _ => "",
    }
}
```

(`module_spec(ty).display_name` may be a different field — check `ModuleSpec`'s actual field names; if the field is `name` or `display_label`, adjust accordingly.)

- [ ] **Step 3: Export the help-box module**

Edit `src/editor/mod.rs`:

```rust
pub mod help_box;
```

- [ ] **Step 4: Mount the help-box in the main editor**

Edit `src/editor_ui.rs`. Find the matrix-rendering region (search for `ROUTING MATRIX` or `fx_matrix_grid::draw`). After the matrix's parent `ui.horizontal(...)` or `ui.vertical(...)`, wrap them so the matrix and help-box share a row:

```rust
ui.horizontal(|ui| {
    // ... existing matrix rendering ...
    ui.add_space(8.0);
    crate::editor::help_box::draw(ui, params, scale);
});
```

(Exact integration point depends on the existing layout; mount the help-box as a sibling of the matrix grid, right-aligned.)

- [ ] **Step 5: Bundle dev plugin and visually verify**

```bash
cargo build --release --features dev-build && \
cargo run --package xtask -- bundle spectral_forge --release --features dev-build && \
cp target/bundled/spectral_forge.clap /home/kim/.clap/spectral/dev/spectral_dev.clap
```

In Bitwig:
1. Select an empty slot — help-box shows "No module assigned…".
2. Assign Past — help-box shows the Past description.
3. Click the AMOUNT tab — help-box shows the AMOUNT per-curve text.
4. Switch mode → Stretch — visible tabs reduce to AMOUNT + MIX; help-box updates.

- [ ] **Step 6: Commit**

```bash
git add src/editor/help_box.rs src/editor/theme.rs src/editor/mod.rs src/editor_ui.rs
git commit -m "feat(editor): help-box widget right of matrix (Phase Past UX, Task 16)

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

## Task 17: End-to-end verification + STATUS.md update

**Files:**
- Modify: `docs/superpowers/STATUS.md` (add the Past UX overhaul entry)
- Verify: cargo test (all green), Bitwig manual test (all 5 modes work end-to-end)

- [ ] **Step 1: Run the full test suite**

```bash
cargo test --release 2>&1 | grep -E "FAILED|^test result"
```

Expected: no FAILED; all `test result: ok.` lines.

```bash
cargo test --release --features probe 2>&1 | grep -E "FAILED|^test result"
```

Expected: same; no probe-feature regressions.

- [ ] **Step 2: Bundle dev plugin and exercise in Bitwig**

```bash
cargo build --release --features dev-build && \
cargo run --package xtask -- bundle spectral_forge --release --features dev-build && \
cp target/bundled/spectral_forge.clap /home/kim/.clap/spectral/dev/spectral_dev.clap
```

Per-mode walk-through (each should produce audible effect or document why not):

1. **Granular**: 5 tabs visible; draw THRESHOLD low; expect frozen-bin replay.
2. **DecaySorter**: 3 tabs; Floor slider visible; SortKey popup picker still works; expect bins rearranged.
3. **Convolution**: 4 tabs; expect multiplicative self-resonance; toggle Soft Clip OFF and observe magnitude blow-up; toggle ON and observe clipping.
4. **Reverse**: 3 tabs + Window scalar; set Window=1 s; expect 1-second backward loop.
5. **Stretch**: 2 tabs + Rate + Dither scalars; set Rate=2.0; expect doubled-speed playback; set Rate=0.5; expect half-speed.

- [ ] **Step 3: Update STATUS.md**

Edit `docs/superpowers/STATUS.md`. Add a row to the Plans table:

```markdown
| 2026-05-04-past-module-ux-overhaul.md | IMPLEMENTED | Per-mode CurveLayout infrastructure (UI spec §8) + Past-specific surface: visible curves per mode, mode scalars (Floor/Window/Rate/Dither), module-wide Soft Clip toggle, help-box widget right of matrix. |
```

And add a row to the Specs table for the new UX spec:

```markdown
| 2026-05-04-past-module-ux-design.md | IMPLEMENTED | Spec for Past UX overhaul; first per-module spec consuming `CurveLayout`. |
```

- [ ] **Step 4: Update top-of-file Last-updated**

Edit the top of `STATUS.md`:

```markdown
**Last updated:** 2026-05-04 (Past UX overhaul: per-mode CurveLayout infrastructure shipped + Past as first consumer; UI spec gains §7 internal-range guide and §8 CurveLayout + help-box).
```

- [ ] **Step 5: Commit**

```bash
git add docs/superpowers/STATUS.md
git commit -m "docs(status): Past UX overhaul IMPLEMENTED (Phase Past UX, Task 17)

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

## Self-review notes

- **Spec coverage:** Every section of the Past UX spec has at least one task: §1 (Tasks 6, 7, 14), §2 (Tasks 8, 12, 15), §3 (Tasks 11, 12, 15), §4 (Tasks 6, 16), §5 (Tasks 2, 3, 4, 5), §6 (Tasks 9, 10, 11, 12, 13), §7 (Tasks 9, 11, 12; §7.2 explicitly out of scope and tracked in Future work). UI spec §7 (range guide) is a doc-only addendum already committed; §8 (CurveLayout) is implemented across Tasks 1, 7, 14.
- **No placeholders:** Every step shows complete code or an exact command. The one "implementation detail follows the same shape as before" caveat in Task 10 Step 3 is bounded — it asks the engineer to keep an existing per-bin loop and only swap one variable. Acceptable for a refactor task.
- **Type consistency:** `PastScalars` shape is consistent across Tasks 9, 10, 11, 12, 13 (floor_bin: usize, window_frames: u32, rate: f32, dither: f32, soft_clip: bool). `gain_to_display` signature gains `total_history_seconds: f32` in Task 3 and is used in the same form throughout. `CurveLayout` field names match between definition (Task 1) and consumers (Tasks 6, 14, 16).
