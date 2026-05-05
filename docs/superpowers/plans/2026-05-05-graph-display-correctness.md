# Graph Display Correctness Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Bring every curve in every module onto the global config-driven UI
system, fix the WYSIWYG calibration mismatch (refined: axis-aware lerp,
linear for `y_log=false`, geometric for `y_log=true`), wire per-mode curve
visibility for all multi-mode modules, and codify the calibration contract
with an automated regression matrix.

**Architecture:** A signature-level change to `offset_fn` (now takes anchors
so log-axis lerps can use runtime y_natural), then surgical recalibration of
the offset_fns that fail the WYSIWYG matrix, then per-mode visibility
plumbing for the seven multi-mode modules that currently default `mode_byte`
to 0. Each calibration fix has a unit test in the matrix; the matrix becomes
the regression guard.

**Tech Stack:** Rust 1.x, `nih-plug-egui` 0.31, no new dependencies.

---

## File structure

| File | Purpose |
|---|---|
| `src/editor/curve_config.rs` | Anchors-aware offset_fn signature; recalibrated `off_thresh`, `off_freeze_thresh`, `off_atk_rel`; cfg.y_natural fixes (Past/2, Past/3) |
| `src/editor/curve.rs` | Extended `runtime_anchors`; axis-aware lerp helper; `apply_curve_adjustments` updated for anchors-aware offset_fn; matching `gain_to_display(4)` clamp widen |
| `src/dsp/modules/mod.rs` | `apply_curve_transform` updated for anchors-aware offset_fn |
| `src/dsp/pipeline.rs` | Plumb anchors into `apply_curve_transform` per slot/curve; mode-byte snapshot per multi-mode module type |
| `src/editor_ui.rs` | Slider formatter uses axis-aware lerp; mode-byte match for all multi-mode modules; calls into updated paint pipeline |
| `src/dsp/modules/future.rs` (and similar) | Add `active_layout` function where DSP varies by mode |
| `src/dsp/modules/freeze.rs` (config side) | Freeze LENGTH y_min 62.5 → 1.0 |
| `tests/curve_calibration_matrix.rs` | New: runs `check_wysiwyg` on every (module, curve) pair |
| `tests/global_system_grep.rs` | New: forbids local display logic outside cfg-driven path |
| `docs/superpowers/specs/2026-05-05-graph-display-audit-table.md` | Generated final audit table |

---

### Task 1: Add axis-aware lerp helper + extend `runtime_anchors`

**Files:**
- Modify: `src/editor/curve.rs:551-585` (runtime_anchors)
- Modify: `src/editor/curve.rs` (add `axis_aware_lerp` near `runtime_anchors`)
- Test: `tests/curve_display_extent.rs`

`runtime_anchors` currently substitutes `db_min`/`db_max` for idx 0 and
`total_history_seconds` for idx 13. We extend it to also substitute
`y_natural` for idx 2 (with `attack_ms`) and idx 3 (with `release_ms`).
Same call-site convention as the previous extension in the prior plan.

`axis_aware_lerp` is the single implementation of the §2 lerp; both the
slider formatter and the audit helper consume it.

- [ ] **Step 1: Write a failing test**

```rust
// Append to tests/curve_display_extent.rs

#[test]
fn runtime_anchors_substitutes_attack_ms_for_idx_2() {
    use spectral_forge::editor::curve::runtime_anchors;
    use spectral_forge::editor::curve_config::curve_display_config;
    use spectral_forge::dsp::modules::{ModuleType, GainMode};

    let cfg = curve_display_config(ModuleType::Dynamics, 2, GainMode::Add);
    // attack_ms = 10 should substitute y_natural; release_ms ignored for idx 2.
    let (lo, nat, hi) = runtime_anchors(&cfg, 2, 0.0, -60.0, 0.0, 10.0, 100.0);
    assert!((nat - 10.0).abs() < 1e-3, "y_natural should be attack_ms=10, got {nat}");
    assert!((lo  -  1.0).abs() < 1e-3);
    assert!((hi  - 1024.0).abs() < 1e-3);
}

#[test]
fn runtime_anchors_substitutes_release_ms_for_idx_3() {
    use spectral_forge::editor::curve::runtime_anchors;
    use spectral_forge::editor::curve_config::curve_display_config;
    use spectral_forge::dsp::modules::{ModuleType, GainMode};

    let cfg = curve_display_config(ModuleType::Dynamics, 3, GainMode::Add);
    let (_, nat, _) = runtime_anchors(&cfg, 3, 0.0, -60.0, 0.0, 10.0, 250.0);
    assert!((nat - 250.0).abs() < 1e-3, "y_natural should be release_ms=250, got {nat}");
}

#[test]
fn axis_aware_lerp_log_geometric_midpoint() {
    use spectral_forge::editor::curve::axis_aware_lerp;
    use spectral_forge::editor::curve_config::curve_display_config;
    use spectral_forge::dsp::modules::{ModuleType, GainMode};

    let cfg = curve_display_config(ModuleType::Dynamics, 1, GainMode::Add); // y_log=true, ratio
    let anchors = (1.0_f32, 1.0, 20.0);
    let mid = axis_aware_lerp(&cfg, anchors, 0.5);
    let expected = 20f32.powf(0.5); // ≈ 4.472
    assert!((mid - expected).abs() < 0.01, "geometric mid expected {expected}, got {mid}");
}

#[test]
fn axis_aware_lerp_linear_arithmetic_midpoint() {
    use spectral_forge::editor::curve::axis_aware_lerp;
    use spectral_forge::editor::curve_config::curve_display_config;
    use spectral_forge::dsp::modules::{ModuleType, GainMode};

    let cfg = curve_display_config(ModuleType::Dynamics, 5, GainMode::Add); // y_log=false, mix
    let anchors = (cfg.y_min, cfg.y_natural, cfg.y_max);
    let mid = axis_aware_lerp(&cfg, anchors, 0.5);
    let expected = cfg.y_natural + 0.5 * (cfg.y_max - cfg.y_natural);
    assert!((mid - expected).abs() < 0.01);
}
```

- [ ] **Step 2: Run tests to confirm they fail**

Run: `cargo test --test curve_display_extent runtime_anchors_substitutes_attack_ms`

Expected: COMPILE FAIL — `runtime_anchors` currently takes 5 args, the test passes 7.

- [ ] **Step 3: Update `runtime_anchors` signature and body**

Replace the existing function at `src/editor/curve.rs` (currently around lines
551–582):

```rust
/// Resolve a `CurveDisplayConfig`'s declared anchors `(y_min, y_natural, y_max)`
/// into runtime physical units. Substitutions:
///   - idx 0  (Dynamics threshold dBFS): y_min, y_max ← (db_min, db_max).
///   - idx 2  (Attack ms):                y_natural ← attack_ms.
///   - idx 3  (Release ms):               y_natural ← release_ms.
///   - idx 13 (Past Age/Delay):           all three scaled by total_history_seconds.
/// Other indices pass cfg anchors through unchanged.
pub fn runtime_anchors(
    cfg: &crate::editor::curve_config::CurveDisplayConfig,
    display_idx: usize,
    total_history_seconds: f32,
    db_min: f32,
    db_max: f32,
    attack_ms: f32,
    release_ms: f32,
) -> (f32, f32, f32) {
    match display_idx {
        13 => {
            let scale = total_history_seconds;
            (cfg.y_min * scale, cfg.y_natural * scale, cfg.y_max * scale)
        }
        0 => (db_min, cfg.y_natural, db_max),
        2 => (cfg.y_min, attack_ms, cfg.y_max),
        3 => (cfg.y_min, release_ms, cfg.y_max),
        _ => (cfg.y_min, cfg.y_natural, cfg.y_max),
    }
}

/// Axis-aware lerp from UI parameter spec §2. Linear in physical units when
/// y_log=false; geometric (log) in physical units when y_log=true.
#[inline]
pub fn axis_aware_lerp(
    cfg: &crate::editor::curve_config::CurveDisplayConfig,
    anchors: (f32, f32, f32),
    v: f32,
) -> f32 {
    let (y_min, y_nat, y_max) = anchors;
    if cfg.y_log {
        if v >= 0.0 { y_nat * (y_max / y_nat).powf(v) }
        else        { y_nat * (y_nat / y_min).powf(v) }
    } else {
        if v >= 0.0 { y_nat + v * (y_max - y_nat) }
        else        { y_nat + v * (y_nat - y_min) }
    }
}
```

- [ ] **Step 4: Update existing call sites to pass attack_ms/release_ms**

Run `grep -rn 'runtime_anchors(' src/ tests/` to find every caller. Two
production sites and at least one test site exist after the prior plan:

- `src/editor_ui.rs:1226` (offset slider formatter)
- `tests/curve_config.rs:238, 245`
- `tests/curve_display_extent.rs:*` (new tests just added)

For each call site, append `attack_ms, release_ms` arguments. The
`editor_ui.rs` site already has `atk_ms` and `rel_ms` in scope at that
location (read from params earlier in the same function — see surrounding
context). The test sites use any reasonable values (e.g., `10.0, 100.0`)
since the existing tests don't depend on those substitutions.

For the editor_ui.rs site:
```rust
let (y_min, y_nat, y_max) = crv::runtime_anchors(
    &off_cfg, off_disp_idx, off_total_history_seconds,
    off_db_min, off_db_max,
    off_atk_ms, off_rel_ms,
);
```

Add `let off_atk_ms = atk_ms; let off_rel_ms = rel_ms;` to the closure
captures (already done if `let _ = (off_atk_ms, off_rel_ms);` exists nearby).

- [ ] **Step 5: Run tests to confirm they pass**

Run: `cargo test --test curve_display_extent`

Expected: PASS for all tests including the four new ones.

- [ ] **Step 6: Confirm no other test broke**

Run: `cargo test`

Expected: 0 new failures (the 5 pre-existing `*_amount_default_probes_50_pct`
failures remain, unrelated).

- [ ] **Step 7: Commit**

```bash
git add src/editor/curve.rs src/editor_ui.rs tests/curve_display_extent.rs tests/curve_config.rs
git commit -m "feat(ui): runtime_anchors substitutes attack_ms/release_ms; add axis_aware_lerp"
```

---

### Task 2: Update offset slider formatter to use `axis_aware_lerp`

**Files:**
- Modify: `src/editor_ui.rs:1213-1237` (offset DragValue custom_formatter)

The slider formatter currently encodes the linear lerp inline. Replace with a
call to the new `axis_aware_lerp` helper so log-axis curves see the geometric
lerp and linear-axis curves see the linear lerp.

- [ ] **Step 1: Replace the formatter body**

In `src/editor_ui.rs`, find the closure inside the offset DragValue (around
line 1213). The current closure:

```rust
.custom_formatter(move |v, _range| {
    if off_cfg.y_label.is_empty() {
        return format!("{:+.2}", v as f32);
    }
    let (y_min, y_nat, y_max) = crv::runtime_anchors(
        &off_cfg, off_disp_idx, off_total_history_seconds,
        off_db_min, off_db_max, off_atk_ms, off_rel_ms,
    );
    let v = v as f32;
    let phys = if v >= 0.0 {
        y_nat + v * (y_max - y_nat)
    } else {
        y_nat + v * (y_nat - y_min)
    };
    let _ = (y_min,);
    format!("{:.1} {}", phys, off_cfg.y_label)
})
```

Replace the inline lerp with the helper call:

```rust
.custom_formatter(move |v, _range| {
    if off_cfg.y_label.is_empty() {
        return format!("{:+.2}", v as f32);
    }
    let anchors = crv::runtime_anchors(
        &off_cfg, off_disp_idx, off_total_history_seconds,
        off_db_min, off_db_max, off_atk_ms, off_rel_ms,
    );
    let phys = crv::axis_aware_lerp(&off_cfg, anchors, v as f32);
    format!("{:.1} {}", phys, off_cfg.y_label)
})
```

- [ ] **Step 2: Build and verify**

Run: `cargo build`

Expected: SUCCESS, no warnings introduced.

- [ ] **Step 3: Run editor tests**

Run: `cargo test --test curve_display_extent && cargo test --test curve_config`

Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add src/editor_ui.rs
git commit -m "refactor(ui): offset slider formatter uses axis_aware_lerp"
```

---

### Task 3: Change `offset_fn` signature to take anchors

**Files:**
- Modify: `src/editor/curve_config.rs` (CurveDisplayConfig + every `off_*` fn)
- Modify: `src/dsp/modules/mod.rs` (apply_curve_transform signature + caller)
- Modify: `src/editor/curve.rs` (apply_curve_adjustments signature + caller)
- Modify: `src/dsp/pipeline.rs` (apply_curve_transform call site)
- Modify: `src/editor_ui.rs` (paint_response_curve callers — already pass cfg)

The signature change ripples through both threads. After this task all
existing offset_fns still produce identical outputs (anchors discarded);
later tasks rely on the new arg to pass runtime y_natural for idx 2/3.

- [ ] **Step 1: Update `CurveDisplayConfig::offset_fn` field type**

In `src/editor/curve_config.rs:27`, change:

```rust
pub offset_fn:  fn(f32, f32) -> f32,
```

to:

```rust
pub offset_fn:  fn(f32, f32, (f32, f32, f32)) -> f32,
```

- [ ] **Step 2: Update every `off_*` function signature**

In `src/editor/curve_config.rs`, every `pub fn off_*(g: f32, o: f32) -> f32`
becomes `pub fn off_*(g: f32, o: f32, _anchors: (f32, f32, f32)) -> f32`.
Bodies unchanged. The 13 functions to update:

```rust
#[inline] pub fn off_amount_norm(g: f32, o: f32, _anchors: (f32,f32,f32)) -> f32 { (g + o).clamp(0.0, 1.0) }
#[inline] pub fn off_thresh(g: f32, o: f32, _anchors: (f32,f32,f32)) -> f32 {
    if o >= 0.0 { g + o } else { g + 2.0 * o }
}
#[inline] pub fn off_ratio(g: f32, o: f32, _anchors: (f32,f32,f32)) -> f32 {
    if o >= 0.0 { g + 19.0 * o } else { g }
}
#[inline] pub fn off_atk_rel(g: f32, o: f32, _anchors: (f32,f32,f32)) -> f32 {
    g * 1024.0_f32.powf(o)
}
#[inline] pub fn off_knee(g: f32, o: f32, _anchors: (f32,f32,f32)) -> f32 {
    if o >= 0.0 { g + 7.0 * o } else { g + o }
}
#[inline] pub fn off_mix(g: f32, o: f32, _anchors: (f32,f32,f32)) -> f32 {
    if o >= 0.0 { g } else { g + o }
}
#[inline] pub fn off_gain_db(g: f32, o: f32, _anchors: (f32,f32,f32)) -> f32 {
    g * 7.943_282_f32.powf(o)
}
#[inline] pub fn off_gain_pct(g: f32, o: f32, _anchors: (f32,f32,f32)) -> f32 {
    if o >= 0.0 { g } else { g + o }
}
#[inline] pub fn off_amount_200(g: f32, o: f32, _anchors: (f32,f32,f32)) -> f32 { g + o }
#[inline] pub fn off_freeze_length(g: f32, o: f32, _anchors: (f32,f32,f32)) -> f32 {
    g * 8.0_f32.powf(o)
}
#[inline] pub fn off_freeze_thresh(g: f32, o: f32, _anchors: (f32,f32,f32)) -> f32 {
    if o >= 0.0 { g + o } else { g + 4.0 * o }
}
#[inline] pub fn off_portamento(g: f32, o: f32, _anchors: (f32,f32,f32)) -> f32 {
    g * 5.0_f32.powf(o)
}
#[inline] pub fn off_resistance(g: f32, o: f32, _anchors: (f32,f32,f32)) -> f32 { g + o }
#[inline] pub fn off_identity(g: f32, _o: f32, _anchors: (f32,f32,f32)) -> f32 { g }
```

(Tasks 4–6 will rewrite three of these bodies. For now, only the signature
changes.)

- [ ] **Step 3: Update `apply_curve_transform` signature in `src/dsp/modules/mod.rs`**

Around line 889:

```rust
pub fn apply_curve_transform(
    gains: &mut [f32],
    tilt: f32,
    offset: f32,
    curvature: f32,
    offset_fn: fn(f32, f32, (f32, f32, f32)) -> f32,
    anchors: (f32, f32, f32),
    sample_rate: f32,
    fft_size: usize,
)
```

Inside the body, find the call to offset_fn (around line 918):

```rust
let g_off = offset_fn(*g, offset);
```

becomes:

```rust
let g_off = offset_fn(*g, offset, anchors);
```

- [ ] **Step 4: Update `apply_curve_transform` call in `src/dsp/pipeline.rs`**

Around line 479. Current call:

```rust
apply_curve_transform(
    &mut self.slot_curve_cache[s][c],
    tilt_norm * TILT_MAX,
    offset,
    curvature,
    offset_fn,
    self.sample_rate,
    self.fft_size,
);
```

Compute `anchors` just above the call. The pipeline already has `db_min`,
`db_max` (from threshold_min/max params), `attack_ms`, `release_ms` (from
attack_ms/release_ms params), and (for now) `total_history_seconds = 0.0`
since pipeline doesn't yet plumb the live history seconds (covered separately
in past-module-ux Task 14). Read using existing patterns:

```rust
let display_idx = crate::editor::curve::display_curve_idx(
    slot_types_snap[s], c, slot_gain_mode_snap[s],
);
let cfg = crate::editor::curve_config::curve_display_config(
    slot_types_snap[s], c, slot_gain_mode_snap[s],
);
let anchors = crate::editor::curve::runtime_anchors(
    &cfg, display_idx, /* total_history_seconds */ 0.0,
    db_min, db_max,
    attack_ms, release_ms,
);
let offset_fn = cfg.offset_fn;
apply_curve_transform(
    &mut self.slot_curve_cache[s][c],
    tilt_norm * TILT_MAX,
    offset,
    curvature,
    offset_fn,
    anchors,
    self.sample_rate,
    self.fft_size,
);
```

If `db_min`, `db_max`, `attack_ms`, `release_ms` are not already in scope at
this site, look at how similar values are read elsewhere in `pipeline.rs::process`
(they're per-block snapshots from params). Add the reads near the top of the
slot_curve loop.

- [ ] **Step 5: Update `apply_curve_adjustments` signature in `src/editor/curve.rs`**

Around line 521:

```rust
pub fn apply_curve_adjustments(
    gain: f32,
    freq_hz: f32,
    tilt: f32,
    offset: f32,
    curvature: f32,
    offset_fn: fn(f32, f32, (f32, f32, f32)) -> f32,
    anchors: (f32, f32, f32),
    nyquist: f32,
) -> f32
```

Body change inside (around line 547):

```rust
let g_off = offset_fn(gain, offset, anchors);
```

- [ ] **Step 6: Update caller in `paint_response_curve`**

In `src/editor/curve.rs` around line 802:

```rust
let adj = apply_curve_adjustments(gains[k], f_hz, tilt, offset, curvature, cfg.offset_fn, anchors, max_hz);
```

`anchors` is already computed earlier in `paint_response_curve` (the `let anchors = runtime_anchors(...)` line from the prior plan's Task 7).

- [ ] **Step 7: Run all tests**

Run: `cargo test`

Expected: 0 new failures (existing offset_fn behavior preserved — anchors are
discarded by every offset_fn body).

Run: `cargo build`

Expected: SUCCESS.

- [ ] **Step 8: Commit**

```bash
git add src/editor/curve_config.rs src/editor/curve.rs src/dsp/modules/mod.rs src/dsp/pipeline.rs
git commit -m "refactor(ui): offset_fn takes anchors; behavior unchanged"
```

---

### Task 4: Recalibrate `off_freeze_thresh` for log dBFS WYSIWYG

**Files:**
- Modify: `src/editor/curve_config.rs` (off_freeze_thresh body)
- Test: append to `tests/curve_display_extent.rs`

Per spec §3.1, replace the additive form with a multiplicative one whose log
maps to the spec's linear lerp in dBFS. Range: -80…0 dBFS, neutral -20.

- [ ] **Step 1: Write a failing WYSIWYG test**

```rust
// Append to tests/curve_display_extent.rs

#[test]
fn off_freeze_thresh_wysiwyg_at_v_minus_half() {
    use spectral_forge::editor::curve::{gain_to_display, runtime_anchors, axis_aware_lerp};
    use spectral_forge::editor::curve_config::{curve_display_config, off_freeze_thresh};
    use spectral_forge::dsp::modules::{ModuleType, GainMode};

    let cfg = curve_display_config(ModuleType::Freeze, 1, GainMode::Add);
    let anchors = runtime_anchors(&cfg, 9, 0.0, -60.0, 0.0, 10.0, 100.0);
    for &v in &[-1.0_f32, -0.5, 0.0, 0.5, 1.0] {
        let g_off = off_freeze_thresh(1.0, v, anchors);
        let display_actual = gain_to_display(9, g_off, 10.0, 100.0, -80.0, 0.0, 0.0);
        let display_expected = axis_aware_lerp(&cfg, anchors, v);
        assert!((display_actual - display_expected).abs() < 0.5,
            "v={v}: expected {display_expected:.2}, got {display_actual:.2}");
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --test curve_display_extent off_freeze_thresh_wysiwyg`

Expected: FAIL — at v=-0.5, expected -50.0, got -80.0 (current additive
formula clamps to floor).

- [ ] **Step 3: Replace `off_freeze_thresh` body**

In `src/editor/curve_config.rs`, find `off_freeze_thresh` (around line 613).
Replace the body:

```rust
/// Freeze THRESHOLD dBFS: WYSIWYG with log-gain dBFS axis (spec §3.1 of
/// 2026-05-05-graph-display-correctness.md). Range -80..0, neutral -20.
/// Inverse of `gain_to_display(9)` evaluated against the spec lerp:
///   v ≥ 0:  display = -20 + 20·v  → factor 10^(0.3·v)
///   v < 0:  display = -20 + 60·v  → factor 10^(0.9·v)
#[inline] pub fn off_freeze_thresh(g: f32, o: f32, _anchors: (f32,f32,f32)) -> f32 {
    if o >= 0.0 { g * 10f32.powf(0.3 * o) } else { g * 10f32.powf(0.9 * o) }
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test --test curve_display_extent off_freeze_thresh_wysiwyg`

Expected: PASS.

- [ ] **Step 5: Run the full suite**

Run: `cargo test`

Expected: 0 new failures.

- [ ] **Step 6: Commit**

```bash
git add src/editor/curve_config.rs tests/curve_display_extent.rs
git commit -m "fix(ui): off_freeze_thresh multiplicative for log dBFS WYSIWYG"
```

---

### Task 5: Recalibrate `off_thresh` for log dBFS WYSIWYG (Dynamics)

**Files:**
- Modify: `src/editor/curve_config.rs` (off_thresh body)
- Test: append to `tests/curve_display_extent.rs`

Same shape as Task 4 but baked for canonical `db_min = -60`. Users who set
`db_min` lower see slight WYSIWYG drift in the negative half (acknowledged in
spec §7.4).

- [ ] **Step 1: Write a failing WYSIWYG test**

```rust
// Append to tests/curve_display_extent.rs

#[test]
fn off_thresh_wysiwyg_at_canonical_db_min() {
    use spectral_forge::editor::curve::{gain_to_display, runtime_anchors, axis_aware_lerp};
    use spectral_forge::editor::curve_config::{curve_display_config, off_thresh};
    use spectral_forge::dsp::modules::{ModuleType, GainMode};

    let cfg = curve_display_config(ModuleType::Dynamics, 0, GainMode::Add);
    let anchors = runtime_anchors(&cfg, 0, 0.0, -60.0, 0.0, 10.0, 100.0);
    for &v in &[-1.0_f32, -0.5, 0.0, 0.5, 1.0] {
        let g_off = off_thresh(1.0, v, anchors);
        let display_actual = gain_to_display(0, g_off, 10.0, 100.0, -60.0, 0.0, 0.0);
        let display_expected = axis_aware_lerp(&cfg, anchors, v);
        assert!((display_actual - display_expected).abs() < 0.5,
            "v={v}: expected {display_expected:.2}, got {display_actual:.2}");
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --test curve_display_extent off_thresh_wysiwyg`

Expected: FAIL.

- [ ] **Step 3: Replace `off_thresh` body**

In `src/editor/curve_config.rs`, find `off_thresh` (around line 559). Replace:

```rust
/// Dynamics THRESHOLD dBFS: WYSIWYG with log-gain dBFS axis (spec §3.1).
/// Calibrated for canonical db_min = -60, db_max = 0. Users who set db_min
/// lower see slight drift in the negative half — see spec §7.4.
///   v ≥ 0:  display = -20 + 20·v  → factor 10^(0.3·v)
///   v < 0:  display = -20 + 40·v  → factor 10^(0.6·v)
#[inline] pub fn off_thresh(g: f32, o: f32, _anchors: (f32,f32,f32)) -> f32 {
    if o >= 0.0 { g * 10f32.powf(0.3 * o) } else { g * 10f32.powf(0.6 * o) }
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test --test curve_display_extent off_thresh_wysiwyg`

Expected: PASS.

- [ ] **Step 5: Run full suite**

Run: `cargo test`

Expected: 0 new failures. Note: `tests/calibration_roundtrip.rs::display_mapping_contract::freeze_threshold_gain_1p5_matches_log_dsp`
remains green (that test pins `gain_to_display(9, 1.5)`, independent of
`off_thresh`).

- [ ] **Step 6: Commit**

```bash
git add src/editor/curve_config.rs tests/curve_display_extent.rs
git commit -m "fix(ui): off_thresh multiplicative for log dBFS WYSIWYG"
```

---

### Task 6: Recalibrate `off_atk_rel` to use anchors (geometric lerp on runtime y_natural)

**Files:**
- Modify: `src/editor/curve_config.rs` (off_atk_rel body)
- Test: append to `tests/curve_display_extent.rs`

`off_atk_rel` is the first offset_fn that consumes anchors. Current
`g · 1024^o` assumes y_natural=1; with attack_ms substituted at runtime, we
need `g · (y_max/y_nat)^o` and `g · (y_nat/y_min)^o`.

- [ ] **Step 1: Write failing tests**

```rust
// Append to tests/curve_display_extent.rs

#[test]
fn off_atk_rel_wysiwyg_at_attack_ms_10() {
    use spectral_forge::editor::curve::{gain_to_display, runtime_anchors, axis_aware_lerp};
    use spectral_forge::editor::curve_config::{curve_display_config, off_atk_rel};
    use spectral_forge::dsp::modules::{ModuleType, GainMode};

    let cfg = curve_display_config(ModuleType::Dynamics, 2, GainMode::Add);
    let attack_ms = 10.0_f32;
    let anchors = runtime_anchors(&cfg, 2, 0.0, -60.0, 0.0, attack_ms, 100.0);
    for &v in &[-1.0_f32, -0.5, 0.0, 0.5, 1.0] {
        let g_off = off_atk_rel(1.0, v, anchors);
        let display_actual = gain_to_display(2, g_off, attack_ms, 100.0, -60.0, 0.0, 0.0);
        let display_expected = axis_aware_lerp(&cfg, anchors, v);
        assert!((display_actual - display_expected).abs() < 0.5,
            "v={v}: expected {display_expected:.3}, got {display_actual:.3}");
    }
}

#[test]
fn off_atk_rel_wysiwyg_at_release_ms_250() {
    use spectral_forge::editor::curve::{gain_to_display, runtime_anchors, axis_aware_lerp};
    use spectral_forge::editor::curve_config::{curve_display_config, off_atk_rel};
    use spectral_forge::dsp::modules::{ModuleType, GainMode};

    let cfg = curve_display_config(ModuleType::Dynamics, 3, GainMode::Add);
    let release_ms = 250.0_f32;
    let anchors = runtime_anchors(&cfg, 3, 0.0, -60.0, 0.0, 10.0, release_ms);
    for &v in &[-1.0_f32, -0.5, 0.0, 0.5, 1.0] {
        let g_off = off_atk_rel(1.0, v, anchors);
        let display_actual = gain_to_display(3, g_off, 10.0, release_ms, -60.0, 0.0, 0.0);
        let display_expected = axis_aware_lerp(&cfg, anchors, v);
        assert!((display_actual - display_expected).abs() < 0.5,
            "v={v}: expected {display_expected:.3}, got {display_actual:.3}");
    }
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --test curve_display_extent off_atk_rel_wysiwyg`

Expected: FAIL — current `1024^o` doesn't honor runtime y_natural.

- [ ] **Step 3: Replace `off_atk_rel` body**

In `src/editor/curve_config.rs`:

```rust
/// Attack/Release ms: geometric lerp from y_natural (=runtime attack_ms or
/// release_ms after substitution by runtime_anchors) to y_min/y_max.
///   v ≥ 0:  factor = (y_max  / y_nat)^v
///   v < 0:  factor = (y_nat  / y_min)^v
#[inline] pub fn off_atk_rel(g: f32, o: f32, anchors: (f32, f32, f32)) -> f32 {
    let (y_min, y_nat, y_max) = anchors;
    let factor = if o >= 0.0 { (y_max / y_nat).powf(o) }
                 else        { (y_nat / y_min).powf(o) };
    g * factor
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --test curve_display_extent off_atk_rel_wysiwyg`

Expected: PASS.

- [ ] **Step 5: Run full suite**

Run: `cargo test`

Expected: 0 new failures.

- [ ] **Step 6: Commit**

```bash
git add src/editor/curve_config.rs tests/curve_display_extent.rs
git commit -m "fix(ui): off_atk_rel anchors-aware geometric lerp"
```

---

### Task 7: Fix `Past/2 THRESHOLD` cfg.y_natural mismatch

**Files:**
- Modify: `src/editor/curve_config.rs` (past_config curve 2)
- Test: append to `tests/curve_display_extent.rs`

Per spec §3.5, `cfg.y_natural` MUST match `gain_to_display(idx, 1.0)`. For
Past curve 2 (THRESHOLD, display idx 9), current `y_natural = -60` lies about
neutral; the actual neutral for idx 9 is -20. The asymmetric reach to -80 is
delivered by the new multiplicative `off_freeze_thresh` in Task 4.

- [ ] **Step 1: Write failing test**

```rust
// Append to tests/curve_display_extent.rs

#[test]
fn past_threshold_y_natural_matches_idx_9_neutral() {
    use spectral_forge::editor::curve::gain_to_display;
    use spectral_forge::editor::curve_config::curve_display_config;
    use spectral_forge::dsp::modules::{ModuleType, GainMode};

    let cfg = curve_display_config(ModuleType::Past, 2, GainMode::Add);
    let neutral_display = gain_to_display(9, 1.0, 10.0, 100.0, -80.0, 0.0, 0.0);
    assert!((cfg.y_natural - neutral_display).abs() < 0.1,
        "Past/2 cfg.y_natural={} should match gain_to_display(9, 1.0)={}",
        cfg.y_natural, neutral_display);
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --test curve_display_extent past_threshold_y_natural`

Expected: FAIL — y_natural=-60, neutral=-20.

- [ ] **Step 3: Update past_config curve 2**

In `src/editor/curve_config.rs`, find `past_config` (around line 476). Update
the curve 2 arm:

```rust
2 => CurveDisplayConfig {
    y_label: "dBFS", y_min: -80.0, y_max: 0.0, y_log: false,
    grid_lines: &[(-60.0, "-60"), (-40.0, "-40"), (-20.0, "-20"), (-6.0, "-6")],
    y_natural: -20.0,
    // Asymmetric reach to -80 dBFS is delivered by off_freeze_thresh's
    // multiplicative calibration (factor 10^(0.9·v) for v<0).
    offset_fn: off_freeze_thresh,
},
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test --test curve_display_extent past_threshold_y_natural`

Expected: PASS.

- [ ] **Step 5: Run full suite**

Run: `cargo test`

Expected: 0 new failures.

- [ ] **Step 6: Commit**

```bash
git add src/editor/curve_config.rs tests/curve_display_extent.rs
git commit -m "fix(past): cfg.y_natural for THRESHOLD matches gain_to_display(9) neutral"
```

---

### Task 8: Fix `Past/3 SPREAD` cfg.y_natural mismatch

**Files:**
- Modify: `src/editor/curve_config.rs` (past_config curve 3)
- Test: append to `tests/curve_display_extent.rs`

Past/3 has `y_natural=50` but `gain_to_display(6, 1.0) = 100`. Update to 100.

- [ ] **Step 1: Write failing test**

```rust
// Append to tests/curve_display_extent.rs

#[test]
fn past_spread_y_natural_matches_idx_6_neutral() {
    use spectral_forge::editor::curve::gain_to_display;
    use spectral_forge::editor::curve_config::curve_display_config;
    use spectral_forge::dsp::modules::{ModuleType, GainMode};

    let cfg = curve_display_config(ModuleType::Past, 3, GainMode::Add);
    let neutral_display = gain_to_display(6, 1.0, 10.0, 100.0, -60.0, 0.0, 0.0);
    assert!((cfg.y_natural - neutral_display).abs() < 0.1,
        "Past/3 cfg.y_natural={} should match gain_to_display(6, 1.0)={}",
        cfg.y_natural, neutral_display);
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --test curve_display_extent past_spread_y_natural`

Expected: FAIL — y_natural=50, neutral=100.

- [ ] **Step 3: Update past_config curve 3**

In `src/editor/curve_config.rs`, in `past_config`, update the curve 3 arm:

```rust
3 => CurveDisplayConfig {
    y_label: "%", y_min: 0.0, y_max: 100.0, y_log: false,
    grid_lines: &[(25.0, "25%"), (50.0, "50%"), (75.0, "75%"), (100.0, "100%")],
    y_natural: 100.0,
    offset_fn: off_mix,
},
```

(Note: also changing `offset_fn` from `off_amount_norm` to `off_mix`, since
neutral=100 with y_min=0 means the slider only has negative reach — `off_mix`
is the additive-on-negative offset that delivers exactly that.)

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test --test curve_display_extent past_spread_y_natural`

Expected: PASS.

- [ ] **Step 5: Run full suite**

Run: `cargo test`

Expected: 0 new failures. The change of offset_fn from `off_amount_norm` to
`off_mix` shifts behavior: with neutral=100 and y_min=0, the user can only
draw the curve down. That's correct for "spread coefficient" — there's no
"more than 100%" semantically.

- [ ] **Step 6: Commit**

```bash
git add src/editor/curve_config.rs tests/curve_display_extent.rs
git commit -m "fix(past): cfg.y_natural for SPREAD matches idx 6 neutral"
```

---

### Task 9: Widen `gain_to_display(4)` clamp to `[0, 48]`

**Files:**
- Modify: `src/editor/curve.rs:602` (gain_to_display arm 4)
- Modify: `src/editor/curve.rs:428` (screen_y_to_physical arm 4 — keep in sync)
- Test: append to `tests/curve_display_extent.rs`

KNEE config declares `y_min=0` but `gain_to_display(4)` clamps to `[1.5, 48]`.
A 0 dB knee is a legitimate "hard knee" value and should be reachable.

- [ ] **Step 1: Write failing test**

```rust
// Append to tests/curve_display_extent.rs

#[test]
fn knee_idx_4_reaches_zero_db_at_gain_zero() {
    use spectral_forge::editor::curve::gain_to_display;
    let v = gain_to_display(4, 0.0, 10.0, 100.0, -60.0, 0.0, 0.0);
    assert!(v <= 0.01, "expected ≈0 dB knee at gain=0, got {v}");
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --test curve_display_extent knee_idx_4_reaches_zero`

Expected: FAIL — current clamp `[1.5, 48]` floors at 1.5.

- [ ] **Step 3: Widen the clamp**

In `src/editor/curve.rs`, find the `gain_to_display` match arm for index 4
(around line 602):

```rust
4 => (gain * 6.0).clamp(1.5, 48.0),
```

Change to:

```rust
4 => (gain * 6.0).clamp(0.0, 48.0),
```

In the same file, find `screen_y_to_physical` (around line 428). Note: after
Task 8 of the prior plan, `screen_y_to_physical` reads cfg.y_min and cfg.y_max
through anchors — there's no explicit per-index clamp. Verify by inspection;
no edit needed if it's already cfg-driven.

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test --test curve_display_extent knee_idx_4_reaches_zero`

Expected: PASS.

- [ ] **Step 5: Run full suite**

Run: `cargo test`

Expected: 0 new failures.

- [ ] **Step 6: Commit**

```bash
git add src/editor/curve.rs tests/curve_display_extent.rs
git commit -m "fix(ui): widen knee clamp to [0, 48] so 0 dB hard knee is reachable"
```

---

### Task 10: Add WYSIWYG calibration matrix test

**Files:**
- Create: `tests/curve_calibration_matrix.rs`

The matrix is the regression guard. Loops over every (module, curve) pair
and runs `check_wysiwyg`. After Tasks 4-9 land, it should be green for
everything except deferred rows (which we mark `#[ignore]` or skip with a
documented exception list).

- [ ] **Step 1: Create the matrix test**

```rust
//! WYSIWYG calibration matrix — UI parameter spec §2 + 2026-05-05-graph-display-correctness.md.
//! Asserts that for every (module, curve_idx), the offset_fn produces a
//! gain that gain_to_display maps back to the spec's axis_aware_lerp value.

use spectral_forge::dsp::modules::{module_spec, GainMode, ModuleType};
use spectral_forge::editor::curve::{
    axis_aware_lerp, display_curve_idx, gain_to_display, runtime_anchors,
};
use spectral_forge::editor::curve_config::curve_display_config;

/// Display indices currently deferred from WYSIWYG enforcement.
/// idx 13: PAST Age/Delay — total_history_seconds plumbing pending.
/// idx 10: PEAK HOLD on PhaseSmear/1 + Gain/1 — DSP function mismatch (separate plan).
fn is_deferred(module: ModuleType, curve_idx: usize, display_idx: usize) -> bool {
    if display_idx == 13 { return true; }
    matches!((module, curve_idx, display_idx),
        (ModuleType::PhaseSmear, 1, 10) | (ModuleType::Gain, 1, 10))
}

fn check_one(module: ModuleType, curve_idx: usize) -> Result<(), String> {
    let cfg = curve_display_config(module, curve_idx, GainMode::Add);
    let display_idx = display_curve_idx(module, curve_idx, GainMode::Add);
    if is_deferred(module, curve_idx, display_idx) { return Ok(()); }

    let attack_ms  = 10.0_f32;
    let release_ms = 100.0_f32;
    let db_min     = -60.0_f32;
    let db_max     = 0.0_f32;
    let history    = 0.0_f32;
    let anchors = runtime_anchors(
        &cfg, display_idx, history, db_min, db_max, attack_ms, release_ms,
    );

    for &v in &[-1.0_f32, -0.5, 0.0, 0.5, 1.0] {
        let g_off = (cfg.offset_fn)(1.0, v, anchors);
        let display_actual = gain_to_display(
            display_idx, g_off, attack_ms, release_ms, db_min, db_max, history,
        );
        let display_expected = axis_aware_lerp(&cfg, anchors, v);
        if (display_actual - display_expected).abs() > 0.5 {
            return Err(format!(
                "{:?}/{} (idx {display_idx}): v={v:+.2} expected {display_expected:.3}, got {display_actual:.3}",
                module, curve_idx
            ));
        }
    }
    Ok(())
}

#[test]
fn calibration_matrix_all_modules_all_curves() {
    let modules: &[ModuleType] = &[
        ModuleType::Dynamics, ModuleType::Freeze, ModuleType::PhaseSmear,
        ModuleType::Contrast, ModuleType::Gain, ModuleType::MidSide,
        ModuleType::TransientSustainedSplit, ModuleType::Harmonic,
        ModuleType::Past, ModuleType::Geometry, ModuleType::Circuit,
        ModuleType::Life, ModuleType::Modulate, ModuleType::Rhythm,
        ModuleType::Punch, ModuleType::Harmony, ModuleType::Kinetics,
        ModuleType::Future,
    ];
    let mut failures = Vec::new();
    for &m in modules {
        let spec = module_spec(m);
        for c in 0..spec.num_curves.min(7) {
            if let Err(msg) = check_one(m, c) {
                failures.push(msg);
            }
        }
    }
    if !failures.is_empty() {
        panic!("{} WYSIWYG failures:\n{}", failures.len(), failures.join("\n"));
    }
}
```

- [ ] **Step 2: Run the test**

Run: `cargo test --test curve_calibration_matrix`

Expected: PASS — all rows except deferred should now WYSIWYG.

If any rows fail, they reveal a calibration gap not covered by Tasks 4-9.
Fix in a new `Task Nx` and re-run.

- [ ] **Step 3: Commit**

```bash
git add tests/curve_calibration_matrix.rs
git commit -m "test(ui): WYSIWYG calibration matrix across all modules"
```

---

### Task 11: Extend mode-byte plumbing for all multi-mode modules

**Files:**
- Modify: `src/editor_ui.rs:488-497` (active_layout mode_byte match)
- Modify: `src/dsp/pipeline.rs` (mirror match if used)

Today only PAST consults `slot_past_mode`; other multi-mode modules fall
through to `mode_byte = 0`. Extend to cover Future, Circuit, Life, Modulate,
Rhythm, Punch, Harmony, Geometry. Param names follow the `slot_<module>_mode`
convention.

- [ ] **Step 1: Verify which slot_*_mode params exist**

Run: `grep -n 'slot_.*_mode' src/params.rs`

Capture the list. The plan assumes all of `slot_past_mode`,
`slot_future_mode`, `slot_circuit_mode`, `slot_life_mode`,
`slot_modulate_mode`, `slot_rhythm_mode`, `slot_punch_mode`,
`slot_harmony_mode`, `slot_geometry_mode` exist (per the next-gen modules
work). If any are missing, defer that module's match arm to a follow-up
(noted in the commit message).

- [ ] **Step 2: Extend the match in `editor_ui.rs`**

Find the existing match (around line 488):

```rust
let mode_byte: u8 = match editing_type {
    crate::dsp::modules::ModuleType::Past => {
        params.slot_past_mode.lock()[editing_slot] as u8
    }
    _ => 0u8,
};
```

Replace with:

```rust
let mode_byte: u8 = match editing_type {
    crate::dsp::modules::ModuleType::Past     => params.slot_past_mode.lock()[editing_slot]     as u8,
    crate::dsp::modules::ModuleType::Future   => params.slot_future_mode.lock()[editing_slot]   as u8,
    crate::dsp::modules::ModuleType::Circuit  => params.slot_circuit_mode.lock()[editing_slot]  as u8,
    crate::dsp::modules::ModuleType::Life     => params.slot_life_mode.lock()[editing_slot]     as u8,
    crate::dsp::modules::ModuleType::Modulate => params.slot_modulate_mode.lock()[editing_slot] as u8,
    crate::dsp::modules::ModuleType::Rhythm   => params.slot_rhythm_mode.lock()[editing_slot]   as u8,
    crate::dsp::modules::ModuleType::Punch    => params.slot_punch_mode.lock()[editing_slot]    as u8,
    crate::dsp::modules::ModuleType::Harmony  => params.slot_harmony_mode.lock()[editing_slot]  as u8,
    crate::dsp::modules::ModuleType::Geometry => params.slot_geometry_mode.lock()[editing_slot] as u8,
    crate::dsp::modules::ModuleType::Kinetics => params.slot_kinetics_mode.lock()[editing_slot] as u8,
    _ => 0u8,
};
```

If a module's `slot_<x>_mode` param doesn't exist (per Step 1), omit that arm
and document in the commit message.

- [ ] **Step 3: Same change in `pipeline.rs` if mirror match exists**

Run: `grep -n 'slot_past_mode' src/dsp/pipeline.rs`

If pipeline mirrors the per-module mode read for active_layout cache keying
or kernel dispatch, apply the same extension. Otherwise skip.

- [ ] **Step 4: Build**

Run: `cargo build`

Expected: SUCCESS.

- [ ] **Step 5: Smoke test the cache invalidation**

Run: `cargo test --test curve_layout`

Expected: PASS. The mode-byte match doesn't directly affect curve_layout
tests, but ensures no regression.

- [ ] **Step 6: Commit**

```bash
git add src/editor_ui.rs src/dsp/pipeline.rs
git commit -m "feat(ui): extend mode-byte plumbing to all multi-mode modules"
```

---

### Task 12: Define `active_layout` for Future module

**Files:**
- Modify: `src/dsp/modules/future.rs` (add `active_layout` function)
- Modify: `src/dsp/modules/mod.rs:630-646` (FUT spec — set `active_layout`)
- Test: append to `tests/curve_layout.rs`

Future has 2 modes (PrintThrough, PreEcho) with different DSP signatures.
Inspect each kernel to determine which curves it consumes; encode in
`active_layout`.

- [ ] **Step 1: Read kernels in `src/dsp/modules/future.rs`**

```bash
grep -n 'fn apply_print_through\|fn apply_pre_echo\|FutureMode::' src/dsp/modules/future.rs
```

Read each kernel's parameters. Map the parameter list back to curve indices
(0=AMOUNT, 1=TIME, 2=THRESHOLD, 3=SPREAD, 4=MIX). Record which curves each
mode consumes.

- [ ] **Step 2: Write a failing test**

Add to `tests/curve_layout.rs`:

```rust
#[test]
fn future_active_layout_matches_kernel_signatures() {
    use spectral_forge::dsp::modules::future::{FutureModule, FutureMode};
    use spectral_forge::dsp::modules::module_spec;
    use spectral_forge::dsp::modules::ModuleType;

    let layout_fn = module_spec(ModuleType::Future).active_layout
        .expect("Future should declare an active_layout");

    // Both modes use AMOUNT, TIME, MIX. PreEcho also uses THRESHOLD (feedback)
    // and SPREAD (HF damping); PrintThrough uses neither.
    // Adjust per Step 1's findings.
    let layout_pt = layout_fn(FutureMode::PrintThrough as u8);
    assert_eq!(layout_pt.active, &[0u8, 1, 4],
        "PrintThrough should expose AMOUNT, TIME, MIX");

    let layout_pe = layout_fn(FutureMode::PreEcho as u8);
    assert_eq!(layout_pe.active, &[0u8, 1, 2, 3, 4],
        "PreEcho should expose all 5 curves");
}
```

(Update the expected `active` lists per Step 1's actual kernel inspection if
they differ.)

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test --test curve_layout future_active_layout`

Expected: FAIL with "Future should declare an active_layout" (until Step 4).

- [ ] **Step 4: Add `active_layout` function to `src/dsp/modules/future.rs`**

```rust
use crate::dsp::modules::CurveLayout;

pub fn active_layout(mode_byte: u8) -> CurveLayout {
    let mode = if mode_byte == 0 { FutureMode::PrintThrough } else { FutureMode::PreEcho };
    match mode {
        // Per Step 1 inspection — adjust active lists as needed.
        FutureMode::PrintThrough => CurveLayout {
            active:          &[0, 1, 4],          // AMOUNT, TIME, MIX
            label_overrides: &[],
            help_for:        |_| "",
            mode_overview:   Some("Print-Through: applies a delayed echo of the input by writing through the FFT history."),
        },
        FutureMode::PreEcho => CurveLayout {
            active:          &[0, 1, 2, 3, 4],
            label_overrides: &[],
            help_for:        |_| "",
            mode_overview:   Some("Pre-Echo: feedback-driven pre-echo with HF damping and threshold gating."),
        },
    }
}
```

- [ ] **Step 5: Wire `active_layout` into the spec**

In `src/dsp/modules/mod.rs`, find the `FUT` static (around line 630) and
change `active_layout: None,` to:

```rust
active_layout: Some(crate::dsp::modules::future::active_layout),
```

- [ ] **Step 6: Run test to verify it passes**

Run: `cargo test --test curve_layout future_active_layout`

Expected: PASS. If the `active` list differs from your Step 1 findings,
update either the test expectations or the layout to match the kernels.

- [ ] **Step 7: Run full suite**

Run: `cargo test`

Expected: 0 new failures.

- [ ] **Step 8: Commit**

```bash
git add src/dsp/modules/future.rs src/dsp/modules/mod.rs tests/curve_layout.rs
git commit -m "feat(future): per-mode active_layout"
```

---

### Task 13: Define `active_layout` for Circuit module

**Files:**
- Modify: `src/dsp/modules/circuit.rs`
- Modify: `src/dsp/modules/mod.rs` (CIRC spec)
- Test: append to `tests/curve_layout.rs`

Circuit has multiple kernels (Schmitt, Comparator, ComponentDrift, Slew, etc.
— inspect to enumerate). Each likely consumes a different subset of
{AMOUNT, THRESH, SPREAD, RELEASE, MIX}.

- [ ] **Step 1: Read kernels**

```bash
grep -n 'fn apply_\|CircuitMode::' src/dsp/modules/circuit.rs
```

For each kernel, list the parameter names consumed. Map to curve indices
(0=AMOUNT, 1=THRESH, 2=SPREAD, 3=RELEASE, 4=MIX).

- [ ] **Step 2: Write a failing test**

Add to `tests/curve_layout.rs`:

```rust
#[test]
fn circuit_active_layout_per_mode() {
    use spectral_forge::dsp::modules::circuit::{CircuitModule, CircuitMode};
    use spectral_forge::dsp::modules::{module_spec, ModuleType};

    let layout_fn = module_spec(ModuleType::Circuit).active_layout
        .expect("Circuit should declare an active_layout");

    // Per Step 1: each mode declares which of [AMOUNT, THRESH, SPREAD, RELEASE, MIX]
    // its kernel actually consumes. Replace with actual findings.
    for mode in CircuitMode::all() {
        let layout = layout_fn(mode as u8);
        assert!(!layout.active.is_empty(),
            "{:?} should expose at least one curve", mode);
        assert!(layout.active.iter().all(|&i| (i as usize) < 5),
            "{:?} active indices must be < num_curves", mode);
    }
}
```

(Replace the loose assertions with mode-specific exact lists per Step 1.)

- [ ] **Step 3: Add `active_layout` function and wire to spec**

Circuit has 10 modes (per `pub enum CircuitMode` at `circuit.rs:746`):
`BbdBins`, `SpectralSchmitt`, `CrossoverDistortion`, `Vactrol`,
`TransformerSaturation`, `PowerSag`, `ComponentDrift`, `PcbCrosstalk`,
`SlewDistortion`, `BiasFuzz`. Each kernel function `apply_<mode>` lives at
its own line in the file (per `grep -n 'fn apply_'`). For each, identify
which of the 5 curve params (AMOUNT=0, THRESH=1, SPREAD=2, RELEASE=3,
MIX=4) the kernel reads via the `curves: &[&[f32]]` slice — search each
function body for `curves.get(N)`.

Then in `src/dsp/modules/circuit.rs`, append:

```rust
use crate::dsp::modules::CurveLayout;

pub fn active_layout(mode_byte: u8) -> CurveLayout {
    // Cast back; if the mode_byte is out of range, default to mode 0.
    let mode = if (mode_byte as usize) < 10 {
        unsafe { std::mem::transmute::<u8, CircuitMode>(mode_byte) }
    } else {
        CircuitMode::BbdBins
    };
    match mode {
        // One match arm per variant. The active list is the curve indices
        // each kernel consumes (per Step 1's grep). Default: all 5 if a
        // kernel uses every param. mode_overview is a 1-2 sentence summary
        // for the help box.
        CircuitMode::BbdBins              => CurveLayout { active: &[0, 1, 2, 3, 4], label_overrides: &[], help_for: |_| "", mode_overview: Some("BBD-style bin delay with chip-noise bleed.") },
        CircuitMode::SpectralSchmitt      => CurveLayout { active: &[0, 1, 3, 4],    label_overrides: &[], help_for: |_| "", mode_overview: Some("Schmitt trigger: hysteresis-based level toggle.") },
        CircuitMode::CrossoverDistortion  => CurveLayout { active: &[0, 1, 4],       label_overrides: &[], help_for: |_| "", mode_overview: Some("Crossover distortion at the bin's zero crossing.") },
        CircuitMode::Vactrol              => CurveLayout { active: &[0, 1, 3, 4],    label_overrides: &[], help_for: |_| "", mode_overview: Some("Vactrol-style slew with photoresistor lag.") },
        CircuitMode::TransformerSaturation=> CurveLayout { active: &[0, 1, 4],       label_overrides: &[], help_for: |_| "", mode_overview: Some("Soft saturation modeling transformer iron.") },
        CircuitMode::PowerSag             => CurveLayout { active: &[0, 3, 4],       label_overrides: &[], help_for: |_| "", mode_overview: Some("Power-rail sag under load.") },
        CircuitMode::ComponentDrift       => CurveLayout { active: &[0, 4],          label_overrides: &[], help_for: |_| "", mode_overview: Some("Slow random parameter drift (component aging).") },
        CircuitMode::PcbCrosstalk         => CurveLayout { active: &[0, 2, 4],       label_overrides: &[], help_for: |_| "", mode_overview: Some("Capacitive crosstalk between adjacent bins.") },
        CircuitMode::SlewDistortion       => CurveLayout { active: &[0, 1, 3, 4],    label_overrides: &[], help_for: |_| "", mode_overview: Some("Slew-rate-limited bin envelopes.") },
        CircuitMode::BiasFuzz             => CurveLayout { active: &[0, 1, 4],       label_overrides: &[], help_for: |_| "", mode_overview: Some("DC-bias fuzz with asymmetric clipping.") },
    }
}
```

(The `active` lists above are an **estimate per kernel naming**. Step 1's
grep is the authoritative source — adjust each arm to match what the kernel
actually reads. Add a `// matches kernel signature` comment per arm when
verified.)

In `src/dsp/modules/mod.rs`, find the `CIRC` static and set:

```rust
active_layout: Some(crate::dsp::modules::circuit::active_layout),
```

In `src/dsp/modules/mod.rs`, find the `CIRC` static and set
`active_layout: Some(crate::dsp::modules::circuit::active_layout),`.

- [ ] **Step 4: Run tests**

Run: `cargo test --test curve_layout circuit_active_layout`

Expected: PASS.

- [ ] **Step 5: Run full suite**

Run: `cargo test`

Expected: 0 new failures.

- [ ] **Step 6: Commit**

```bash
git add src/dsp/modules/circuit.rs src/dsp/modules/mod.rs tests/curve_layout.rs
git commit -m "feat(circuit): per-mode active_layout"
```

---

### Task 14: Define `active_layout` for Life, Modulate, Rhythm, Punch, Harmony, Geometry, Kinetics

**Files:**
- Modify: `src/dsp/modules/life.rs`, `modulate.rs`, `rhythm.rs`, `punch.rs`, `harmony.rs`, `geometry.rs`, `kinetics.rs`
- Modify: `src/dsp/modules/mod.rs` (each module's static spec)
- Test: append to `tests/curve_layout.rs` (one test per module)

Repeat the Task 12/13 pattern for the remaining seven multi-mode modules.
Mode enumerations confirmed via grep on the source files:

| Module | Modes |
|---|---|
| Life (`pub enum LifeMode` at `life.rs:122`) | Viscosity, SurfaceTension, Crystallization, Archimedes, NonNewtonian, Stiction, Yield, Capillary, Sandpaper, Brownian (10 total) |
| Modulate (`pub enum ModulateMode` at `modulate.rs:575`) | PhasePhaser, BinSwapper, RmFmMatrix, DiodeRm, GroundLoop, GravityPhaser, PllTear, FmNetwork (8 total) |
| Rhythm (`pub enum RhythmMode` at `rhythm.rs:57`) | 4 modes including Euclidean, Arpeggiator, PhaseReset (read the file for the 4th) |
| Punch (`pub enum PunchMode` at `punch.rs:10`) | Direct, Inverse (2 total) |
| Harmony (`pub enum HarmonyMode` at `harmony.rs:8`) | Chordification, Undertone, Companding, FormantRotation, Lifter, Inharmonic, HarmonicGenerator, Shuffler (8 total — Stiffness/Bessel/Prime are sub-options of HarmonicGenerator, NOT separate modes) |
| Geometry (`pub enum GeometryMode` at `geometry.rs:34`) | Chladni, Helmholtz (2 total) |
| Kinetics (`pub enum KineticsMode` at `kinetics.rs:29`) | Hooke, GravityWell, InertialMass, OrbitalPhase, Ferromagnetism, ThermalExpansion, TuningFork, Diamagnet (8 total — verify exact list against the enum) |

For each of the six modules:

- [ ] **Step 1: Inspect kernels per module**

```bash
grep -n 'fn apply_\|<Module>Mode::' src/dsp/modules/<module>.rs
```

Read each kernel's `curves.get(N)` calls to identify which curve indices it
consumes. Curve indices follow the module's `curve_labels` order in
`src/dsp/modules/mod.rs`. For example, Life's `curve_labels = &["AMOUNT",
"THRESHOLD", "SPEED", "REACH", "MIX"]` → 0=AMOUNT, 1=THRESHOLD, 2=SPEED,
3=REACH, 4=MIX.

- [ ] **Step 2: Add `active_layout` function**

Pattern (from Task 13). One match arm per mode variant, listing the curve
indices that mode's kernel actually reads. For Geometry, the only 2-mode
example:

```rust
// In src/dsp/modules/geometry.rs:

use crate::dsp::modules::CurveLayout;

pub fn active_layout(mode_byte: u8) -> CurveLayout {
    let mode = if mode_byte == 0 { GeometryMode::Chladni } else { GeometryMode::Helmholtz };
    match mode {
        GeometryMode::Chladni   => CurveLayout {
            active: &[0, 1, 2, 4],   // AMOUNT, MODE_CAP, DAMP_REL, MIX (THRESH not used)
            label_overrides: &[],
            help_for: |_| "",
            mode_overview: Some("Chladni: standing-wave pattern at FFT bin spacing."),
        },
        GeometryMode::Helmholtz => CurveLayout {
            active: &[0, 1, 2, 3, 4],
            label_overrides: &[],
            help_for: |_| "",
            mode_overview: Some("Helmholtz: resonant cavity emphasis above THRESH."),
        },
    }
}
```

(Active lists above are estimates — verify against kernel signatures.)

For modules with many modes (Life, Modulate, Circuit-style), follow the same
pattern — one match arm per variant, active list per kernel inspection.

- [ ] **Step 3: Wire spec.active_layout**

In `src/dsp/modules/mod.rs`, find each module's static spec (LIFE, MOD, RHY,
PUNCH, HARM, GEOM) and set:

```rust
active_layout: Some(crate::dsp::modules::<module>::active_layout),
```

- [ ] **Step 4: Add curve_layout test per module**

Append to `tests/curve_layout.rs`. Pattern from Task 12, adjusted per module.
Each test asserts that for each mode, `active_layout(mode as u8).active`
matches the curve indices the kernel actually reads.

```rust
#[test]
fn geometry_active_layout_per_mode() {
    use spectral_forge::dsp::modules::geometry::GeometryMode;
    use spectral_forge::dsp::modules::{module_spec, ModuleType};

    let layout_fn = module_spec(ModuleType::Geometry).active_layout
        .expect("Geometry should declare an active_layout");

    let chladni = layout_fn(GeometryMode::Chladni as u8);
    assert_eq!(chladni.active, &[0u8, 1, 2, 4]);

    let helmholtz = layout_fn(GeometryMode::Helmholtz as u8);
    assert_eq!(helmholtz.active, &[0u8, 1, 2, 3, 4]);
}
```

Adapt the assertion lists per module per Step 1's findings.

- [ ] **Step 5: Run tests after each module**

```bash
cargo test --test curve_layout <module>_active_layout
cargo test
```

Expected: PASS for the new test, 0 new global failures.

- [ ] **Step 6: Commit per module**

One commit per module, message format:

```
feat(<module>): per-mode active_layout
```

If Step 1 inspection reveals all modes consume all curves (no hiding
needed), use:

```
style(<module>): keep static active_layout — all modes use all curves
```

and skip the active_layout function (leave `spec.active_layout = None`).

---

### Task 15: Add global-system grep test

**Files:**
- Create: `tests/global_system_grep.rs`

Codify spec §5.3-§5.5 as a CI-fail test. Catches future regressions where
someone adds a new module with local display logic.

- [ ] **Step 1: Create the test**

```rust
//! Forbids local display logic outside the cfg-driven path.
//! See docs/superpowers/specs/2026-05-05-graph-display-correctness.md §5.

use std::process::Command;

fn grep(pattern: &str, paths: &[&str]) -> Vec<String> {
    let mut args = vec!["-rEn", pattern];
    args.extend_from_slice(paths);
    let out = Command::new("grep").args(&args).output().expect("grep");
    String::from_utf8_lossy(&out.stdout)
        .lines().map(str::to_string).collect()
}

#[test]
fn no_local_linear_or_log_to_y_in_modules() {
    let hits = grep(r"\b(linear_to_y|log_to_y)\b", &["src/dsp/modules/"]);
    assert!(hits.is_empty(),
        "DSP modules must not call linear_to_y/log_to_y directly:\n{}",
        hits.join("\n"));
}

#[test]
fn no_local_display_idx_match_in_modules() {
    // A `match curve_idx { 0..=N => display_value }` style branch in a DSP
    // module is a smell — display ranges belong in curve_config.rs.
    // Match on curve_idx for DSP behaviour (parameter selection) is fine;
    // we only flag arms that produce dB/ms/% literals.
    let hits = grep(r#"=>\s*\([^)]*\b(dB|dBFS|ms|%)\b"#, &["src/dsp/modules/"]);
    assert!(hits.is_empty(),
        "DSP modules must not encode display unit literals:\n{}",
        hits.join("\n"));
}

#[test]
fn no_y_label_string_outside_curve_config() {
    let hits: Vec<_> = grep(r#""\s*(dBFS|dB/oct)\s*""#, &["src/"])
        .into_iter()
        .filter(|l| !l.starts_with("src/editor/curve_config.rs:"))
        .filter(|l| !l.starts_with("src/editor/curve.rs:"))
        .collect();
    assert!(hits.is_empty(),
        "Y-axis unit literals must live in curve_config.rs:\n{}",
        hits.join("\n"));
}
```

- [ ] **Step 2: Run the test**

Run: `cargo test --test global_system_grep`

Expected: PASS for the existing codebase. If any test fails, the failure
output points at the offending file:line — fix per spec §5 (move the local
logic into `curve_config.rs`'s `CurveDisplayConfig`).

- [ ] **Step 3: If a test fails, fix the source and re-run**

Each unique failure is its own small fix. Common fixes:

- Replace a local `linear_to_y(v, -60.0, 0.0, rect)` call with
  `physical_to_y(v, &cfg, anchors, rect)` — the standard pattern.
- Move a hardcoded "%" or "dBFS" string from a module file into
  `curve_config.rs`'s y_label.

Re-run the test after each fix.

- [ ] **Step 4: Commit**

```bash
git add tests/global_system_grep.rs <any source files modified>
git commit -m "test(ui): forbid local display logic outside cfg-driven path"
```

---

### Task 16: Freeze LENGTH 1 ms floor

**Files:**
- Modify: `src/editor/curve_config.rs:116-121` (freeze_config curve 0)
- Modify: `src/editor/curve.rs:432, 632` (screen_y_to_physical / physical_to_y log clamps)
- Modify: `tests/calibration_roundtrip.rs:600-622` (Freeze length tests)

The current floor is 62.5 ms; user wants 1 ms. The change cascades through
the cfg, the log_to_y/screen_y_to_physical clamps, and the existing
calibration_roundtrip tests that pin 62.5.

- [ ] **Step 1: Update freeze_config curve 0**

```rust
0 => CurveDisplayConfig {
    y_label: "ms", y_min: 1.0, y_max: 4000.0, y_log: true,
    grid_lines: &[(10.0, "10ms"), (100.0, "100ms"), (1000.0, "1s"), (4000.0, "4s")],
    y_natural: 500.0,
    offset_fn: off_freeze_length,
},
```

- [ ] **Step 2: Update screen_y_to_physical and physical_to_y clamps**

In `src/editor/curve.rs`, find the idx 8 arms in `screen_y_to_physical`
(around line 432) and `physical_to_y` (around line 632). After Task 8 of the
prior plan these are config-driven, but if any explicit `62.5` literal
remains, change to `1.0`. Verify by inspection.

- [ ] **Step 3: Update calibration_roundtrip tests**

In `tests/calibration_roundtrip.rs`, find the freeze-length tests (around
lines 600-622). They pin `62.5` as the y_min. Update to `1.0`:

```rust
#[test]
fn freeze_length_physical_to_y_at_y_min_is_rect_bottom() {
    let r = rect();
    let cfg = curve_display_config(ModuleType::Freeze, 0, GainMode::Add);
    let anchors = runtime_anchors(&cfg, 8, 0.0, -80.0, 0.0, 10.0, 100.0);
    let y = physical_to_y(1.0, &cfg, anchors, r);
    assert!((y - r.bottom()).abs() < 1.0,
        "v=1 ms (y_min) should map to rect.bottom()={}, got {}", r.bottom(), y);
}
```

(Note: this test now also passes the new `attack_ms`, `release_ms` args to
`runtime_anchors`. If the prior plan didn't add them yet, defer this Task
until after Task 1.)

- [ ] **Step 4: Run tests**

Run: `cargo test --features=probe --test calibration_roundtrip freeze_length`

Expected: PASS.

- [ ] **Step 5: Run full suite**

Run: `cargo test`

Expected: 0 new failures.

- [ ] **Step 6: Commit**

```bash
git add src/editor/curve_config.rs src/editor/curve.rs tests/calibration_roundtrip.rs
git commit -m "feat(freeze): widen LENGTH range to 1ms..4000ms floor"
```

---

### Task 17: Generate the audit table

**Files:**
- Create: `docs/superpowers/specs/2026-05-05-graph-display-audit-table.md`

Run the matrix once and capture its output verbatim into the table doc, then
extend with per-row "Visible in modes" and "Global system?" verdicts.

- [ ] **Step 1: Run the matrix and capture output**

```bash
cargo test --test curve_calibration_matrix -- --nocapture > /tmp/matrix.txt
```

- [ ] **Step 2: Hand-author the audit table**

Create `docs/superpowers/specs/2026-05-05-graph-display-audit-table.md` with
this structure:

```markdown
# Graph Display Audit Table (2026-05-05)

Generated from `tests/curve_calibration_matrix.rs` after Tasks 1-15 of
`docs/superpowers/plans/2026-05-05-graph-display-correctness.md`.

| Module | Curve | idx | Axis | offset_fn | WYSIWYG? | Visible in modes | Global system? | Notes |
|--------|-------|-----|------|-----------|----------|------------------|----------------|-------|
| Dynamics | THRESHOLD | 0 | log-gain dBFS | off_thresh | ✓ | always | ✓ | Calibrated for db_min=-60 |
| Dynamics | RATIO | 1 | log | off_ratio | ✓ | always | ✓ | |
| Dynamics | ATTACK | 2 | log | off_atk_rel | ✓ | always | ✓ | y_natural ← attack_ms |
| Dynamics | RELEASE | 3 | log | off_atk_rel | ✓ | always | ✓ | y_natural ← release_ms |
| Dynamics | KNEE | 4 | linear | off_knee | ✓ | always | ✓ | clamp [0, 48] |
| Dynamics | MIX | 5/6 | linear % | off_mix | ✓ | always | ✓ | |
| Freeze | LENGTH | 0/8 | log | off_freeze_length | ✓ | always | ✓ | y_min=1ms |
| Freeze | THRESHOLD | 1/9 | log-gain dBFS | off_freeze_thresh | ✓ | always | ✓ | |
... (one row per (module, curve), filled in from /tmp/matrix.txt and
manual inspection of active_layouts and the global_system_grep test) ...
| PhaseSmear | PEAK HOLD | 1/10 | log | off_portamento | ✗ DEFERRED | always | ✓ | DSP fn mismatch — separate plan |
| Past | TIME | 1/13 | linear (history-rel) | off_amount_norm | ✗ DEFERRED | Granular,Convolution | ✓ | history seconds plumbing — past-module-ux Task 14 |
```

(Fill the full ~80-row table from the matrix output and the active_layouts
defined in Tasks 12-14.)

- [ ] **Step 3: Commit**

```bash
git add docs/superpowers/specs/2026-05-05-graph-display-audit-table.md
git commit -m "docs(spec): graph display audit table"
```

---

### Task 18: Final regression sweep + smoke test

**Files:** none modified.

- [ ] **Step 1: Full test suite**

Run: `cargo test`

Expected: 0 failures across all binaries (the 5 pre-existing
`*_amount_default_probes_50_pct` failures only manifest with `--features=probe`
and are unrelated to this plan).

- [ ] **Step 2: Probe-feature suite**

Run: `cargo test --features=probe`

Expected: only the 5 pre-existing failures remain.

- [ ] **Step 3: Release build**

Run: `cargo build --release`

Expected: SUCCESS.

- [ ] **Step 4: Bundle and install**

```bash
cargo run --package xtask -- bundle spectral_forge --release
cp target/bundled/spectral_forge.clap ~/.clap/
```

- [ ] **Step 5: Manual smoke test in Bitwig**

Walk through every module:

1. **Calibration check**: drag the Offset slider on a representative curve
   and visually confirm the curve baseline tracks the slider's displayed
   value. Spot-check Freeze THRESHOLD specifically (the bug that motivated
   this plan): drag below -40 dBFS and confirm the curve continues toward
   -80 dBFS.
2. **Visibility check**: switch each multi-mode module's mode and confirm
   the tab strip changes to show only that mode's active curves.
3. **Update check**: move a node in each module and confirm the response
   curve redraws immediately. Future module specifically — the user-reported
   "no graphs update" symptom should now be fixed by Task 11's mode-byte
   plumbing + Task 12's Future active_layout.

If any check fails, capture the failing module/curve in a GitHub issue (or
follow-up plan) and address.

- [ ] **Step 6: Update commit log / branch state**

This task closes the plan; no further commits expected. The audit table
(Task 17) is the standing artifact.
