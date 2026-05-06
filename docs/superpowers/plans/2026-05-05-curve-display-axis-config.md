# Curve Display Axis-Config Wiring + Threshold Formula Fix Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix the broken Freeze/PAST threshold-curve display floor at display
index 9, and make the global Y-axis renderer source its linear/log axis choice
and `[y_min, y_max]` range from each module's `CurveDisplayConfig` instead of a
per-index `match` arm.

**Architecture:** Two changes that ship together. (1) Replace the linear
gain→dBFS formula at `display_curve_idx == 9` with the same log formula already
used at index 0, and propagate the matching change into `freeze.rs` and the four
threshold gates in `past.rs`. (2) Wire `cfg.y_min`, `cfg.y_max`, and `cfg.y_log`
through `physical_to_y` and `screen_y_to_physical` so axis behaviour is config-
driven; add runtime substitution for `db_min`/`db_max` (idx=0) alongside the
existing `total_history_seconds` substitution (idx=13). The integer
`display_curve_idx` is retained — only the gain→physical formula stays indexed,
since each curve's gain mapping carries module-specific units (dBFS, ratio, ms,
%) that the config does not currently encode. Linear/log is the *axis* choice,
which the config already declares.

**Tech Stack:** Rust 1.x; `nih-plug-egui` 0.31; `realfft`. No new dependencies.

---

## File Structure

- `src/editor/curve.rs` — Modify `gain_to_display`, `physical_to_y`,
  `screen_y_to_physical`, and grow `runtime_anchors` to cover `db_min`/`db_max`.
- `src/editor/curve_config.rs` — No new types. Existing `CurveDisplayConfig`
  fields (`y_min`, `y_max`, `y_log`) start carrying real meaning.
- `src/dsp/modules/freeze.rs` — Update `curve_to_threshold_db` to log formula
  matching the new display.
- `src/dsp/modules/past.rs` — Replace four `mag_sq < thr * thr` gates with a
  shared helper that interprets the curve as dBFS via `curve_to_threshold_db`.
- `docs/superpowers/specs/2026-04-23-ui-parameter-spec-design.md` — §1 and §3:
  document that `physical_to_y` honours `cfg.y_log`/`y_min`/`y_max` and that
  thresholds use a log gain→dBFS mapping.
- `tests/curve_display_extent.rs` — Add unit coverage for the new behaviour.

---

### Task 1: Pin the bug with a failing test

**Files:**
- Modify: `tests/curve_display_extent.rs` (append to existing file)

The current `gain_to_display(9, gain)` is linear in gain:
`(-40 + gain * 20).clamp(-80, 0)`. EQ nodes produce gains in roughly
`[0.126, 7.94]` (a ±18 dB bell). At `gain = 0.126` the formula yields
`-37.5 dBFS`, so the lower half of the −80 … 0 dBFS display is unreachable from
node moves alone. We want a regression test that documents the desired log
behaviour up front.

- [ ] **Step 1: Write the failing test**

```rust
// At the bottom of tests/curve_display_extent.rs

#[test]
fn freeze_threshold_dbfs_reaches_floor_with_eq_node_minimum() {
    // gain ≈ 0.126 corresponds to a -18 dB EQ bell (10^(-18/20)).
    // The display range is -80..0 dBFS; the formula must reach the floor.
    // db_min and db_max are unused by display_idx == 9 (it owns its own range).
    use spectral_forge::editor::curve::gain_to_display;

    let g_min = 10f32.powf(-18.0 / 20.0); // 0.1259
    let g_neutral = 1.0f32;
    let g_max = 10f32.powf(18.0 / 20.0);  // 7.943

    let dbfs_min     = gain_to_display(9, g_min,     0.0, 0.0, -60.0, 0.0, 0.0);
    let dbfs_neutral = gain_to_display(9, g_neutral, 0.0, 0.0, -60.0, 0.0, 0.0);
    let dbfs_max     = gain_to_display(9, g_max,     0.0, 0.0, -60.0, 0.0, 0.0);

    // Neutral curve (gain 1.0) → -20 dBFS (the y_natural anchor in freeze_config).
    assert!((dbfs_neutral - (-20.0)).abs() < 1e-3, "expected -20, got {dbfs_neutral}");
    // Bottom EQ node must reach the -80 dBFS floor.
    assert!(dbfs_min <= -79.0, "expected ≤ -79 (close to floor), got {dbfs_min}");
    // Top EQ node must reach the 0 dBFS ceiling.
    assert!(dbfs_max >= -1.0, "expected ≥ -1 (close to ceiling), got {dbfs_max}");
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --test curve_display_extent freeze_threshold_dbfs_reaches_floor_with_eq_node_minimum`

Expected: FAIL — the assertion `dbfs_min <= -79.0` fails because the current
formula produces `-37.48` for `gain = 0.126`.

- [ ] **Step 3: Commit the failing test**

```bash
git add tests/curve_display_extent.rs
git commit -m "test(curve_display): pin -80 dBFS floor reachability for idx=9"
```

---

### Task 2: Fix `gain_to_display(9)` to use the log formula

**Files:**
- Modify: `src/editor/curve.rs:608-612`

Replace the linear formula with the log formula already used at idx 0,
re-clamped to `-80 … 0` dBFS. The multiplier `60.0 / 18.0` is the same: it
maps a ±18 dB EQ excursion onto a ±60 dB display window centred at
`-20 dBFS`, saturating cleanly at both ends.

- [ ] **Step 1: Edit the match arm**

Replace lines 608–612 of `src/editor/curve.rs`:

```rust
        9 => {                                               // Freeze Threshold: dBFS
            // UI parameter spec §1: log gain→dBFS mapping. The display range
            // is -80..0 dBFS centred at gain=1.0 → -20 dBFS. Multiplier 60/18
            // maps a ±18 dB EQ excursion to a ±60 dB display swing, so node
            // moves alone can reach the -80 dBFS floor.
            let t_db = if gain > 1e-10 { 20.0 * gain.log10() } else { -120.0 };
            (-20.0 + t_db * (60.0 / 18.0)).clamp(-80.0, 0.0)
        }
```

- [ ] **Step 2: Run the new test**

Run: `cargo test --test curve_display_extent freeze_threshold_dbfs_reaches_floor_with_eq_node_minimum`

Expected: PASS.

- [ ] **Step 3: Run the full curve display test file to catch regressions**

Run: `cargo test --test curve_display_extent`

Expected: PASS for all tests.

- [ ] **Step 4: Commit**

```bash
git add src/editor/curve.rs
git commit -m "fix(ui): log gain→dBFS at display idx 9 so nodes reach -80 dBFS"
```

---

### Task 3: Update `freeze::curve_to_threshold_db` to match

**Files:**
- Modify: `src/dsp/modules/freeze.rs:5-10`
- Test: append to `tests/calibration.rs` (which covers DSP↔display calibration)

The DSP threshold formula must mirror the corrected display, otherwise the
audible threshold drifts away from the dBFS shown on the curve.

- [ ] **Step 1: Write a failing test in `tests/calibration.rs`**

```rust
// At the end of tests/calibration.rs

#[test]
fn freeze_threshold_dsp_matches_display_log_formula() {
    use spectral_forge::dsp::modules::freeze::curve_to_threshold_db;
    use spectral_forge::editor::curve::gain_to_display;

    for &g in &[0.126_f32, 0.5, 1.0, 2.0, 7.94] {
        let dsp = curve_to_threshold_db(g);
        let ui  = gain_to_display(9, g, 0.0, 0.0, -60.0, 0.0, 0.0);
        assert!((dsp - ui).abs() < 1e-3,
            "DSP {dsp} ≠ UI {ui} at gain={g}");
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --test calibration freeze_threshold_dsp_matches_display_log_formula`

Expected: FAIL — DSP still uses the linear formula.

- [ ] **Step 3: Replace the body of `curve_to_threshold_db`**

In `src/dsp/modules/freeze.rs`:

```rust
/// Map a per-bin threshold curve gain (linear, 1.0 = neutral) to dBFS threshold.
/// Mirrors `gain_to_display(9, ...)` exactly — see UI parameter spec §1.
/// gain=1.0 → -20 dBFS; gain≈0.126 → -80 dBFS; gain≈8.0 → 0 dBFS.
pub fn curve_to_threshold_db(curve_gain: f32) -> f32 {
    let t_db = if curve_gain > 1e-10 { 20.0 * curve_gain.log10() } else { -120.0 };
    (-20.0 + t_db * (60.0 / 18.0)).clamp(-80.0, 0.0)
}
```

- [ ] **Step 4: Run both the calibration test and freeze tests**

Run: `cargo test --test calibration && cargo test --test calibration_roundtrip`

Expected: PASS.

Run: `cargo test freeze`

Expected: PASS — Freeze threshold tests should still pass; the log mapping
preserves `gain=1.0 → -20 dBFS` and `gain=2.0 → 0 dBFS`, the two existing test
anchors.

- [ ] **Step 5: Commit**

```bash
git add src/dsp/modules/freeze.rs tests/calibration.rs
git commit -m "fix(freeze): log gain→dBFS in curve_to_threshold_db to match UI"
```

---

### Task 4: Add a shared `curve_gain_to_threshold_lin` helper for the magnitude domain

**Files:**
- Modify: `src/dsp/modules/freeze.rs` (add second helper next to `curve_to_threshold_db`)
- Test: append to `tests/calibration.rs`

PAST's four threshold gates compare squared magnitudes. To keep the gate
matching the displayed dBFS, we need a single helper that converts curve gain
to linear magnitude (=`10^(threshold_db / 20)`). Put it next to
`curve_to_threshold_db` so both modules import the same function.

- [ ] **Step 1: Write a failing test**

```rust
// In tests/calibration.rs

#[test]
fn curve_gain_to_threshold_lin_round_trips_through_dbfs() {
    use spectral_forge::dsp::modules::freeze::{curve_gain_to_threshold_lin, curve_to_threshold_db};

    for &g in &[0.126_f32, 0.5, 1.0, 2.0, 7.94] {
        let db = curve_to_threshold_db(g);
        let lin_expected = 10f32.powf(db / 20.0);
        let lin_actual   = curve_gain_to_threshold_lin(g);
        assert!((lin_actual - lin_expected).abs() < 1e-6,
            "expected {lin_expected}, got {lin_actual} at gain={g}");
    }
}
```

- [ ] **Step 2: Run to verify it fails (function doesn't exist)**

Run: `cargo test --test calibration curve_gain_to_threshold_lin_round_trips_through_dbfs`

Expected: FAIL with "cannot find function `curve_gain_to_threshold_lin`".

- [ ] **Step 3: Add the helper in `src/dsp/modules/freeze.rs`**

Append below `curve_to_threshold_db`:

```rust
/// Map a per-bin threshold curve gain to a linear magnitude floor, matching
/// the dBFS shown by the UI. Compare a bin's magnitude (not magnitude-squared)
/// directly against the return value, or compare squared magnitude against
/// `lin * lin`. See UI parameter spec §1.
#[inline]
pub fn curve_gain_to_threshold_lin(curve_gain: f32) -> f32 {
    10.0_f32.powf(curve_to_threshold_db(curve_gain) / 20.0)
}
```

- [ ] **Step 4: Run the test**

Run: `cargo test --test calibration curve_gain_to_threshold_lin_round_trips_through_dbfs`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/dsp/modules/freeze.rs tests/calibration.rs
git commit -m "feat(freeze): expose curve_gain_to_threshold_lin helper"
```

---

### Task 5: Replace PAST's four `mag_sq < thr * thr` gates with the helper

**Files:**
- Modify: `src/dsp/modules/past.rs:430, 460, 525, 556`
- Test: append to `tests/past.rs`

Today PAST treats the threshold curve as a raw linear magnitude — at
`gain = 1.0` (the default), every bin with magnitude below 1.0 is gated out.
Most spectral bins are well below 1.0, so the default produces near-silence.
The fix: route the curve gain through `curve_gain_to_threshold_lin` so the
gate matches the dBFS the UI displays. At `gain = 1.0` the gate becomes
`mag < 10^(-20/20) ≈ 0.1`, meaning bins above roughly −20 dBFS pass through —
matching the curve's `y_natural`.

- [ ] **Step 1: Write a failing PAST gate test**

Append to `tests/past.rs`. The wiring follows the same pattern as
`granular_replacement_at_correct_age` already in that file: build a
`HistoryBuffer`, write hops, build a `ModuleContext` and assign
`ctx.history = Some(&h)`. `PastModule::new(sample_rate, fft_size)` takes two
args; `process` takes `physics: Option<&mut BinPhysics>` between
`suppression_out` and `ctx`.

```rust
#[test]
fn past_threshold_at_neutral_gain_passes_typical_spectral_content() {
    // At curve gain=1.0 (neutral), the new gate threshold is -20 dBFS.
    // Bin 50 (mag 0.5  → -6 dBFS) must pass through the gate (be processed).
    // Bin 200 (mag 0.01 → -40 dBFS) must be gated → left at its input value.
    use spectral_forge::dsp::history_buffer::HistoryBuffer;
    use spectral_forge::dsp::modules::past::{PastModule, PastMode};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule};
    use spectral_forge::params::{FxChannelTarget, StereoLink};
    use num_complex::Complex;

    let n = 256;
    // Pre-fill history so the granular kernel has frames to read.
    let mut h = HistoryBuffer::new(1, 64, n);
    for _ in 0..16 {
        let mut frame = vec![Complex::new(0.0, 0.0); n];
        frame[50]  = Complex::new(0.5, 0.0);
        h.write_hop(0, &frame);
        h.advance_after_all_channels_written();
    }

    let mut m = PastModule::new(48_000.0, 2048);
    m.set_mode(PastMode::Granular);

    let mut bins = vec![Complex::new(0.0, 0.0); n];
    bins[50]  = Complex::new(0.5,  0.0); // -6 dBFS  → above gate
    bins[200] = Complex::new(0.01, 0.0); // -40 dBFS → below gate

    let amount    = vec![1.0_f32; n];
    let time      = vec![0.5_f32; n];
    let threshold = vec![1.0_f32; n]; // neutral curve → -20 dBFS gate
    let spread    = vec![0.0_f32; n];
    let mix       = vec![1.0_f32; n];
    let curves: Vec<&[f32]> = vec![&amount, &time, &threshold, &spread, &mix];

    let mut supp = vec![0.0_f32; n];
    let mut ctx  = ModuleContext::new(48_000.0, 2048, n, 10.0, 100.0, 1.0, 0.5, false, false);
    ctx.history  = Some(&h);

    m.process(0, StereoLink::Linked, FxChannelTarget::All,
              &mut bins, None, &curves, &mut supp, None, &ctx);

    // Bin 200 (-40 dBFS) was below the -20 dBFS gate; the kernel hit `continue`
    // before any write, so the bin must equal its input value exactly.
    assert!((bins[200].re - 0.01).abs() < 1e-4 && bins[200].im.abs() < 1e-6,
        "bin below threshold should pass through unchanged, got {:?}", bins[200]);
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --test past past_threshold_at_neutral_gain_passes_typical_spectral_content`

Expected: FAIL — bin 100 stays at 0.5 unprocessed because the current gate
`0.25 < 1.0` blocks every bin under magnitude 1.0.

- [ ] **Step 3: Add the import and replace the four gates**

In `src/dsp/modules/past.rs`, add the import near the top of the file (next to
the existing `use super::*` block):

```rust
use super::freeze::curve_gain_to_threshold_lin;
```

Then update each of the four gate sites. The pattern transforms:

```rust
let thr = threshold.get(k).copied().unwrap_or(0.0);
if mag_sq < thr * thr { continue; }
```

into:

```rust
let thr_gain = threshold.get(k).copied().unwrap_or(1.0);  // 1.0 = neutral default
let thr_lin  = curve_gain_to_threshold_lin(thr_gain);
if mag_sq < thr_lin * thr_lin { continue; }
```

Apply at lines 429–430, 459–460, 524–525, and 555–556. The `unwrap_or` default
changes from `0.0` (no gate) to `1.0` (neutral threshold) — this matches the
default curve value the rest of the module assumes.

- [ ] **Step 4: Run the PAST tests**

Run: `cargo test past`

Expected: PASS for all `past*` tests including the new one.

- [ ] **Step 5: Run the full test suite to catch regressions**

Run: `cargo test`

Expected: PASS. Existing tests that pass `threshold = vec![0.0; n]` keep
working under the new helper: `curve_gain_to_threshold_lin(0.0) = 10^(-80/20) =
1e-4`, so the gate becomes `mag_sq < 1e-8` — wide open for any audible bin.
Tests that pass `threshold = vec![1.0; n]` (neutral) now get a `-20 dBFS` gate;
inspect any failure to confirm whether the test's intent was "wide open"
(should change to `0.0`) or "neutral threshold" (correctly tightened).

- [ ] **Step 6: Commit**

```bash
git add src/dsp/modules/past.rs tests/past.rs
git commit -m "fix(past): gate by dBFS threshold so curve neutral = -20 dBFS"
```

---

### Task 6: Extend `runtime_anchors` to substitute `db_min`/`db_max` for absolute-dBFS curves

**Files:**
- Modify: `src/editor/curve.rs:563-575`
- Test: append to `tests/curve_display_extent.rs`

`runtime_anchors` already substitutes `total_history_seconds` for display index
13. The Dynamics threshold (idx 0) has the same structural issue: its config
declares static `y_min = -60`, `y_max = 0`, but `physical_to_y` currently uses
runtime `db_min`/`db_max`. Once Task 7 makes `physical_to_y` config-driven, we
need `runtime_anchors` to deliver runtime-substituted anchors.

- [ ] **Step 1: Write a failing test**

```rust
// tests/curve_display_extent.rs

#[test]
fn runtime_anchors_substitutes_db_range_for_threshold_idx_0() {
    use spectral_forge::editor::curve::runtime_anchors;
    use spectral_forge::editor::curve_config::{curve_display_config};
    use spectral_forge::dsp::modules::{GainMode, ModuleType};

    let cfg = curve_display_config(ModuleType::Dynamics, 0, GainMode::Add);

    // db_min=-72, db_max=-3 should override cfg.y_min/y_max for display idx 0.
    let (y_min, y_natural, y_max) = runtime_anchors(&cfg, 0, 0.0, -72.0, -3.0);
    assert!((y_min - -72.0).abs() < 1e-3, "expected -72, got {y_min}");
    assert!((y_max - -3.0).abs() < 1e-3, "expected -3, got {y_max}");
    // y_natural is the config's neutral (-20 dBFS) — not substituted.
    assert!((y_natural - -20.0).abs() < 1e-3, "expected -20, got {y_natural}");

    // Display index 13 still substitutes total_history_seconds, unaffected.
    let past_cfg = curve_display_config(ModuleType::Past, 1, GainMode::Add);
    let (a, b, c) = runtime_anchors(&past_cfg, 13, 4.0, -60.0, 0.0);
    assert!((c - 4.0).abs() < 1e-3, "history substitution still works, got {c}");
    let _ = (a, b);

    // Other display indices pass through unchanged.
    let phase_cfg = curve_display_config(ModuleType::PhaseSmear, 0, GainMode::Add);
    let (lo, _, hi) = runtime_anchors(&phase_cfg, 7, 0.0, -60.0, 0.0);
    assert!((lo - 0.0).abs() < 1e-3 && (hi - 200.0).abs() < 1e-3);
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --test curve_display_extent runtime_anchors_substitutes_db_range_for_threshold_idx_0`

Expected: FAIL — `runtime_anchors` currently takes 3 arguments, not 5; this is
a compilation error.

- [ ] **Step 3: Update `runtime_anchors` signature and body**

Replace the function in `src/editor/curve.rs`:

```rust
/// Resolve a `CurveDisplayConfig`'s declared anchors `(y_min, y_natural, y_max)`
/// into runtime physical units. Two display indices need runtime substitution:
///   - idx 0 (Dynamics threshold dBFS): `(y_min, y_max)` are taken from the
///     active `db_min`/`db_max` parameters; `y_natural` (-20 dBFS) is preserved.
///   - idx 13 (Past Age/Delay): all three anchors are scaled by
///     `total_history_seconds`.
/// All other indices pass `cfg` anchors through unchanged.
///
/// See UI parameter spec §2.
pub fn runtime_anchors(
    cfg: &crate::editor::curve_config::CurveDisplayConfig,
    display_idx: usize,
    total_history_seconds: f32,
    db_min: f32,
    db_max: f32,
) -> (f32, f32, f32) {
    match display_idx {
        13 => {
            let s = total_history_seconds;
            (cfg.y_min * s, cfg.y_natural * s, cfg.y_max * s)
        }
        0 => (db_min, cfg.y_natural, db_max),
        _ => (cfg.y_min, cfg.y_natural, cfg.y_max),
    }
}
```

- [ ] **Step 4: Update existing call sites of `runtime_anchors`**

Search for the existing call:

```bash
grep -rn "runtime_anchors" src/
```

Each call site needs the two new arguments. Threading is straightforward — the
call sites already have `db_min`/`db_max` in scope (they're paint parameters).

For example, in `src/editor/curve.rs` around the offset slider formatter (the
sole current caller), change:

```rust
let (y_min, y_natural, y_max) = runtime_anchors(&cfg, display_idx, total_history_seconds);
```

to:

```rust
let (y_min, y_natural, y_max) = runtime_anchors(&cfg, display_idx, total_history_seconds, db_min, db_max);
```

If a caller doesn't yet have `db_min`/`db_max` in scope, plumb them through —
they already flow into `paint_grid` and `paint_response_curve` from the
top-level frame.

- [ ] **Step 5: Run the test and the rest of the editor tests**

Run: `cargo test --test curve_display_extent`

Expected: PASS.

Run: `cargo test --test curve_node_automation && cargo test --test curve_layout`

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/editor/curve.rs tests/curve_display_extent.rs
git commit -m "feat(ui): runtime_anchors substitutes db_min/db_max at idx 0"
```

---

### Task 7: Refactor `physical_to_y` to consume `CurveDisplayConfig`

**Files:**
- Modify: `src/editor/curve.rs:622-638` (function body)
- Modify: `src/editor/curve.rs:736, 803` (call sites inside `paint_grid` and `paint_response_curve`)
- Test: append to `tests/curve_display_extent.rs`

After this change, `physical_to_y` no longer cares about the integer index for
the linear/log axis decision: the caller passes the resolved
`CurveDisplayConfig` and runtime anchors. The integer `display_idx` is only
needed if the caller wants to defer to `runtime_anchors` itself, but for the
function under test the anchors are pre-computed.

- [ ] **Step 1: Write a failing test**

```rust
// tests/curve_display_extent.rs

#[test]
fn physical_to_y_uses_cfg_y_log_for_axis_choice() {
    use spectral_forge::editor::curve::physical_to_y;
    use spectral_forge::editor::curve_config::{curve_display_config};
    use spectral_forge::dsp::modules::{GainMode, ModuleType};
    use nih_plug_egui::egui::{Pos2, Rect};

    let rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(100.0, 100.0));

    // Dynamics RATIO (idx 1, log axis 1..20). At y_min the pixel is bottom;
    // at y_max it's top; at the geometric midpoint sqrt(20)≈4.47 it's centre.
    let ratio_cfg = curve_display_config(ModuleType::Dynamics, 1, GainMode::Add);
    let anchors_ratio = (ratio_cfg.y_min, ratio_cfg.y_natural, ratio_cfg.y_max);

    let y_bottom = physical_to_y(1.0, &ratio_cfg, anchors_ratio, rect);
    let y_top    = physical_to_y(20.0, &ratio_cfg, anchors_ratio, rect);
    let y_mid    = physical_to_y(20f32.sqrt(), &ratio_cfg, anchors_ratio, rect);

    assert!((y_bottom - rect.bottom()).abs() < 1e-3);
    assert!((y_top    - rect.top()).abs() < 1e-3);
    assert!((y_mid    - rect.center().y).abs() < 1.0);

    // PAST mix (idx 6, linear axis 0..100). 50 maps to centre.
    let mix_cfg = curve_display_config(ModuleType::Past, 4, GainMode::Add);
    let anchors_mix = (mix_cfg.y_min, mix_cfg.y_natural, mix_cfg.y_max);
    let y_50 = physical_to_y(50.0, &mix_cfg, anchors_mix, rect);
    assert!((y_50 - rect.center().y).abs() < 1.0);
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --test curve_display_extent physical_to_y_uses_cfg_y_log_for_axis_choice`

Expected: FAIL — current `physical_to_y` signature is
`(v, curve_idx, db_min, db_max, rect)`, so the test won't compile.

- [ ] **Step 3: Replace the `physical_to_y` body**

In `src/editor/curve.rs`:

```rust
/// Map a physical value to pixel y for a given display config.
/// `anchors` is `(y_min, y_natural, y_max)` from `runtime_anchors`, i.e. with
/// runtime substitutions for idx 0 (db range) and idx 13 (history seconds)
/// already applied. The linear/log axis choice comes from `cfg.y_log`.
/// See UI parameter spec §3.
pub fn physical_to_y(
    v: f32,
    cfg: &crate::editor::curve_config::CurveDisplayConfig,
    anchors: (f32, f32, f32),
    rect: Rect,
) -> f32 {
    let (y_min, _y_natural, y_max) = anchors;
    if cfg.y_log {
        log_to_y(v.max(y_min), y_min, y_max, rect)
    } else {
        linear_to_y(v, y_min, y_max, rect)
    }
}
```

- [ ] **Step 4: Update the two call sites**

Find each caller in `src/editor/curve.rs`:

```bash
grep -n "physical_to_y(" src/editor/curve.rs
```

There are two paint-time callers (around lines 736 and 803 in `paint_grid` /
`paint_response_curve`). Each already has both `cfg` and the integer
`display_idx` in scope. Change the call from the old signature

```rust
let y = physical_to_y(v, display_idx, db_min, db_max, rect);
```

to

```rust
let anchors = runtime_anchors(cfg, display_idx, total_history_seconds, db_min, db_max);
let y = physical_to_y(v, cfg, anchors, rect);
```

Pull the `runtime_anchors` call to the top of each function so it's done once,
not per grid line / per response sample.

- [ ] **Step 5: Run all editor-related tests**

Run: `cargo test --test curve_display_extent && cargo test --test curve_node_automation && cargo test --test curve_layout && cargo test --test curve_sampling`

Expected: PASS.

- [ ] **Step 6: Build the plugin to confirm no callers broke**

Run: `cargo build`

Expected: SUCCESS — no remaining references to the old `physical_to_y` shape.

- [ ] **Step 7: Commit**

```bash
git add src/editor/curve.rs tests/curve_display_extent.rs
git commit -m "refactor(ui): physical_to_y reads y_log/y_min/y_max from cfg"
```

---

### Task 8: Refactor `screen_y_to_physical` to consume `CurveDisplayConfig`

**Files:**
- Modify: `src/editor/curve.rs:422-444` (function body)
- Modify: `src/editor/curve.rs:473` (the one call inside `paint_hover_text`)
- Test: append to `tests/curve_display_extent.rs`

Mirror Task 7 for the inverse mapping (pixel-y → physical value, used by
hover tooltips).

- [ ] **Step 1: Write a failing test**

```rust
// tests/curve_display_extent.rs

#[test]
fn screen_y_to_physical_inverts_physical_to_y_for_log_and_linear() {
    use spectral_forge::editor::curve::{physical_to_y, screen_y_to_physical};
    use spectral_forge::editor::curve_config::{curve_display_config};
    use spectral_forge::dsp::modules::{GainMode, ModuleType};
    use nih_plug_egui::egui::{Pos2, Rect};

    let rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(100.0, 100.0));

    // Log axis (Dynamics ratio).
    let ratio_cfg = curve_display_config(ModuleType::Dynamics, 1, GainMode::Add);
    let anchors_ratio = (ratio_cfg.y_min, ratio_cfg.y_natural, ratio_cfg.y_max);
    for &v in &[1.5_f32, 4.0, 10.0] {
        let y    = physical_to_y(v, &ratio_cfg, anchors_ratio, rect);
        let back = screen_y_to_physical(y, &ratio_cfg, anchors_ratio, rect);
        assert!((back - v).abs() < 0.05, "round-trip {v} → {y} → {back}");
    }

    // Linear axis (Mix %).
    let mix_cfg = curve_display_config(ModuleType::Past, 4, GainMode::Add);
    let anchors_mix = (mix_cfg.y_min, mix_cfg.y_natural, mix_cfg.y_max);
    for &v in &[12.5_f32, 33.0, 78.0] {
        let y    = physical_to_y(v, &mix_cfg, anchors_mix, rect);
        let back = screen_y_to_physical(y, &mix_cfg, anchors_mix, rect);
        assert!((back - v).abs() < 0.5, "round-trip {v} → {y} → {back}");
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --test curve_display_extent screen_y_to_physical_inverts_physical_to_y_for_log_and_linear`

Expected: FAIL — compile error from new signature.

- [ ] **Step 3: Replace the body**

```rust
/// Inverse of `physical_to_y` — pixel y → physical value, for hover tooltips.
/// Reads `cfg.y_log` and the runtime-substituted `(y_min, _, y_max)` anchors.
/// See UI parameter spec §3.
pub fn screen_y_to_physical(
    y: f32,
    cfg: &crate::editor::curve_config::CurveDisplayConfig,
    anchors: (f32, f32, f32),
    rect: Rect,
) -> f32 {
    let (y_min, _, y_max) = anchors;
    let t = ((rect.bottom() - y) / rect.height()).clamp(0.0, 1.0);
    if cfg.y_log {
        let lo = y_min.max(1e-6);
        lo * (y_max / lo).powf(t)
    } else {
        y_min + t * (y_max - y_min)
    }
}
```

- [ ] **Step 4: Add `total_history_seconds` to `paint_hover_text` and update the call**

`paint_hover_text` currently takes
`(painter, cursor_pos, rect, display_idx, cfg, db_min, db_max, sample_rate)`.
Add `total_history_seconds: f32` between `db_max` and `sample_rate`:

```rust
pub fn paint_hover_text(
    painter: &Painter,
    cursor_pos: Pos2,
    rect: Rect,
    display_idx: usize,
    cfg: &CurveDisplayConfig,
    db_min: f32,
    db_max: f32,
    total_history_seconds: f32,
    sample_rate: f32,
) {
    use nih_plug_egui::egui::{FontId, vec2};
    let nyquist = (sample_rate / 2.0).max(20_001.0);
    let freq_hz = screen_to_freq(cursor_pos.x, rect, nyquist);
    let anchors = runtime_anchors(cfg, display_idx, total_history_seconds, db_min, db_max);
    let phys    = screen_y_to_physical(cursor_pos.y, cfg, anchors, rect);
    // … rest of the body unchanged
```

Update the single caller (search with `grep -n 'paint_hover_text(' src/`). The
caller is inside the curve widget, which already owns `total_history_seconds`
because it forwards the same value to `paint_response_curve`. Pass it in.

- [ ] **Step 5: Run all curve tests**

Run: `cargo test --test curve_display_extent && cargo test --test curve_node_automation`

Expected: PASS.

- [ ] **Step 6: Build the plugin**

Run: `cargo build`

Expected: SUCCESS.

- [ ] **Step 7: Commit**

```bash
git add src/editor/curve.rs tests/curve_display_extent.rs
git commit -m "refactor(ui): screen_y_to_physical reads y_log/anchors from cfg"
```

---

### Task 9: Update the UI parameter spec

**Files:**
- Modify: `docs/superpowers/specs/2026-04-23-ui-parameter-spec-design.md` §1, §3

The plan changes two contract-level facts: thresholds use a log gain→dBFS
formula, and the Y-axis renderer is config-driven rather than index-driven.
Both belong in the spec so future modules calibrate correctly.

- [ ] **Step 1: Update §1 with the threshold formula**

Find §1 (the `CurveDisplayConfig` definition / threshold notes). Add a
subsection at the end:

```markdown
### §1.4 Threshold dBFS curves

Curves whose physical unit is dBFS and whose neutral is `−20 dBFS` (display
indices 0 and 9) use a logarithmic gain→dBFS mapping:

    threshold_db = clamp(-20 + 20·log10(gain) · (60/18), y_min, y_max)

This guarantees that a full ±18 dB EQ-node excursion sweeps the entire display
range, so node moves alone reach `y_min` (the lower clamp). DSP modules that
gate by the displayed threshold MUST share this mapping —
`freeze::curve_to_threshold_db` is the canonical implementation; both Freeze
and PAST consume it (PAST via `curve_gain_to_threshold_lin`).
```

- [ ] **Step 2: Update §3 Y-axis with the wiring requirement**

In §3 ("Axes, grid lines, and hover text"), under "Y-axis", replace the
existing bullet list with:

```markdown
### Y-axis

- The active curve's `y_label` is always rendered on the Y-axis.
- The vertical mapping `physical → pixel` is driven by `CurveDisplayConfig`:
  - `cfg.y_log == true` → logarithmic spacing.
  - `cfg.y_log == false` → linear spacing.
  - The `[y_min, y_max]` range comes from `runtime_anchors(cfg, display_idx, …)`,
    which substitutes `db_min`/`db_max` for display index 0 and
    `total_history_seconds` for display index 13. All other indices pass
    `cfg.y_min`/`cfg.y_max` through unchanged.
- Grid lines use the four entries in `cfg.grid_lines`. Each is mapped by the
  same `physical_to_y(v, cfg, anchors, rect)` call as the response curve, so
  grid and curve cannot drift.
- `paint_grid`, `paint_response_curve`, and `paint_hover_text` MUST go through
  `physical_to_y` / `screen_y_to_physical`. No painter is allowed to encode the
  axis choice inline.
```

- [ ] **Step 3: Verify markdown is well-formed**

Run: `grep -n '^##' docs/superpowers/specs/2026-04-23-ui-parameter-spec-design.md | head`

Expected: section numbering still consistent (no broken headings).

- [ ] **Step 4: Commit**

```bash
git add docs/superpowers/specs/2026-04-23-ui-parameter-spec-design.md
git commit -m "docs(spec): mandate config-driven Y-axis and log threshold formula"
```

---

### Task 10: Final regression sweep

**Files:** none modified.

- [ ] **Step 1: Run the full test suite**

Run: `cargo test`

Expected: PASS.

- [ ] **Step 2: Run a release build to confirm no audio-thread-allocation regressions**

Run: `cargo build --release`

Expected: SUCCESS. (`assert_process_allocs` runs in tests, not release; this
step just confirms the release build still compiles.)

- [ ] **Step 3: Bundle and smoke-test in Bitwig**

Run:
```bash
cargo run --package xtask -- bundle spectral_forge --release && cp target/bundled/spectral_forge.clap ~/.clap/
```

Expected: bundle succeeds. Open the plugin in Bitwig; load a Freeze module on
a slot; drag a node on the THRESHOLD curve down — confirm the curve visibly
extends below `−40 dBFS` (it should reach `−80` at the lowest EQ node y) and
that audible gating tracks the displayed threshold.

This is a manual visual/audio check; no automated assertion. Capture any
remaining issues (like the unrelated PAST AMOUNT/SMEAR DSP bugs) as
follow-ups — they are out of scope for this plan.

---

## Out of scope (follow-up plans)

These items came up alongside the threshold bug but are tracked separately so
this plan stays focused:

- **PAST AMOUNT no audible effect**, **PAST SMEAR no audible effect** — DSP
  bugs in the granular/decay/convolution apply paths.
- **Curve label capitalization** — module specs use `ALL_CAPS` (`AMOUNT`,
  `THRESH`); user wants Word Caps everywhere. Global rename in
  `src/dsp/modules/mod.rs` and per-module `ModuleSpec::curve_labels`.
- **MIX default at 100% wet** — verify all modules' default curve gain (1.0)
  maps to 100% mix at initial render.
- **Age offset graph not updating** — display idx 13 renders with
  `total_history_seconds = 0` because the offset DragValue formatter doesn't
  receive the runtime value (Task 14 in the past-module-ux-overhaul plan).
- **Generalising `gain_to_display`** — fold it into `CurveDisplayConfig` via
  a `gain_to_phys: fn(f32, &Ctx) -> f32` field. Larger refactor; not required
  to fix the linear/log axis question.
