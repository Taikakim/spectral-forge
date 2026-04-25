> **Status (2026-04-24): IMPLEMENTED.** Merged via `fd8f600`. Closed the 2026-04-24 spec-deviation review. Source of truth: the code + [../STATUS.md](../STATUS.md).

# UI Parameter Spec Cleanup — Implementation Plan

> **For agentic workers:** Use superpowers:subagent-driven-development to execute this plan task-by-task.

**Goal:** Close every deviation from `docs/superpowers/specs/2026-04-23-ui-parameter-spec-design.md` identified in the 2026-04-24 review.

**Architecture:** The cleanup is mostly a finish-the-migration job. The generated `s{slot}c{curve}_tilt`/`_offset` FloatParams already exist (build.rs:124+); the audio path still reads from the legacy `slot_curve_meta` Mutex. Six tasks: route the audio thread through the FloatParams and delete the Mutex (T1–T2), make offset semantics match the spec (T3), make X-axis Nyquist-aware (T4), wire grid/hover through `CurveDisplayConfig` (T5), enforce UI scaling (T6), and add contract comments (T7).

**Tech Stack:** Rust, nih-plug, egui, realfft. No new deps.

**Out of scope:** preset back-compat (early dev — old presets break). The migration block in `params.rs:548–564` is deleted, not preserved.

**Offset semantics (Task 3):** Option A (DSP calibration). Offset is per-curve calibrated in gain space so that ±1 spans the full `[y_min, y_max]` display range. Knob reads truthful physical values.

---

## Task 0: Worktree setup

- [ ] Use `superpowers:using-git-worktrees` to create `.worktrees/ui-spec-cleanup` on `master`. Run `cargo test` to confirm 81 tests pass as baseline.

---

## Task 1: Audio path reads tilt/offset from FloatParams

**Files:**
- Modify: `src/dsp/pipeline.rs:192–206`

- [ ] **Step 1:** Replace the `meta_guard = params.slot_curve_meta.try_lock()` block with three smoothed param reads per slot/curve. Pattern:

```rust
for s in 0..9 {
    for c in 0..7 {
        self.slot_curve_cache[s][c]
            .copy_from_slice(&shared.curve_rx[s][c].read()[..MAX_NUM_BINS]);
        let tilt = params.tilt_param(s, c)
            .map(|p| p.smoothed.next_step(block_size) * TILT_MAX)
            .unwrap_or(0.0);
        let offset = params.offset_param(s, c)
            .map(|p| p.smoothed.next_step(block_size))  // physical mapping in T3
            .unwrap_or(0.0);
        let curvature = params.curvature_param(s, c)
            .map(|p| p.smoothed.next_step(block_size))
            .unwrap_or(0.0);
        apply_curve_transform(
            &mut self.slot_curve_cache[s][c],
            tilt, offset, curvature,
            self.sample_rate, self.fft_size,
        );
    }
}
```

`TILT_MAX` lives in `src/editor_ui.rs` — move it to `src/dsp/modules/mod.rs` so both audio and GUI can see it.

- [ ] **Step 2:** Add `pub const TILT_MAX: f32 = 2.0;` near `apply_curve_transform` in `src/dsp/modules/mod.rs`. Remove the editor-side definition; update its import.

- [ ] **Step 3:** Run `cargo build --release`. Expected: clean build.

- [ ] **Step 4:** Run `cargo test`. Expected: 81 passing.

- [ ] **Step 5:** Commit: `feat(pipeline): read tilt/offset from generated FloatParams`.

---

## Task 2: Delete `slot_curve_meta` and introduce `CurveTransform`

**Files:**
- Modify: `src/params.rs` (lines 147, 176, 307, 548–564, 729, 778)
- Modify: `src/presets.rs` (lines 13, 64; remove `no_meta()` if now unused)
- Modify: `src/lib.rs:204` (migration JSON reader)
- Modify: `src/editor_ui.rs` (lines 755–757, 779–781 — drop the dual-write)
- Modify: `src/editor/module_popup.rs:146` (lock site)
- Create: `CurveTransform` struct in `src/dsp/modules/mod.rs`

- [ ] **Step 1:** Add the struct in `src/dsp/modules/mod.rs`:

```rust
/// Per-curve display+DSP transform. See docs/superpowers/specs/2026-04-23-ui-parameter-spec-design.md §2.
#[derive(Clone, Copy, Debug, Default)]
pub struct CurveTransform {
    pub offset:    f32,  // [-1, 1] normalized; mapped to physical units per curve
    pub tilt:      f32,  // [-1, 1] normalized; ×TILT_MAX in audio path
    pub curvature: f32,  // [0, 1]
}
```

- [ ] **Step 2:** Add a helper on `SpectralForgeParams`:

```rust
/// Snapshot the three transform params for one slot/curve.
/// See docs/superpowers/specs/2026-04-23-ui-parameter-spec-design.md §2.
pub fn curve_transform(&self, slot: usize, curve: usize) -> CurveTransform {
    CurveTransform {
        offset:    self.offset_param(slot, curve).map_or(0.0, |p| p.value()),
        tilt:      self.tilt_param(slot, curve).map_or(0.0, |p| p.value()),
        curvature: self.curvature_param(slot, curve).map_or(0.0, |p| p.value()),
    }
}
```

GUI sites that previously read `slot_curve_meta[s][c]` now call `params.curve_transform(s, c)`.

- [ ] **Step 3:** Delete from `src/params.rs`:
  - Field declaration line 147
  - Default initializer line 307
  - Migration block lines 548–564
  - `persist_out!`/`persist_in!` lines 729, 778

- [ ] **Step 4:** Delete from `src/presets.rs`:
  - Field at line 13
  - Initializer at line 64
  - `no_meta()` helper if no longer referenced

- [ ] **Step 5:** Remove the `slot_curve_meta` JSON read in `src/lib.rs:204`.

- [ ] **Step 6:** In `src/editor_ui.rs`, delete the `meta.try_lock()` dual-writes at lines 755–757 and 779–781. The `setter.set_parameter` call already writes to the FloatParam; the audio thread now reads from there.

- [ ] **Step 7:** In `src/editor/module_popup.rs:146`, replace the `slot_curve_meta.lock()` reset (called when assigning a module) with a loop that resets each curve's three FloatParams via `params.offset_param(s, c).reset()` etc. Use `setter` if available; otherwise call `set_plain_value` on `Smoother` directly.

- [ ] **Step 8:** `cargo test` and `cargo build --release`. Expected: clean.

- [ ] **Step 9:** Commit: `refactor: delete slot_curve_meta, replace with CurveTransform helper`.

---

## Task 3: Offset semantics — DSP calibration so ±1 spans [y_min, y_max]

**Approach:** per-curve offset calibration in gain space. `CurveDisplayConfig` carries a function pointer that converts a normalized offset `[-1, 1]` into a gain-space operation (additive for linear-display curves, multiplicative for log-display curves). The audio thread calls it per bin; the GUI knob displays the resulting physical value.

**Why a function pointer:** threshold, ratio, and knee use additive offset in gain space; attack, release, and freeze hold-time are log-display (`y_log: true`) and need multiplicative offset so the visual shift is linear in log-space. Encoding both behaviours as a plain `fn(g: f32, offset_norm: f32) -> f32` keeps `apply_curve_transform` allocation-free and branch-free per bin.

**Per-curve calibration values** (what ±1 means in gain space):

| Curve | y_natural | y_min | y_max | Neg span (g at offset=-1) | Pos span (g at offset=+1) | Op |
|-------|-----------|-------|-------|---------------------------|---------------------------|----|
| Dynamics Threshold | -20 dBFS (g=1) | -60 dBFS (g=-1) | 0 dBFS (g=2)    | -2.0 | +1.0 | add |
| Dynamics Ratio     | 1:1   (g=1)    | 1:1   (g=1)     | 20:1 (g=20)     |  0.0 | +19.0 | add |
| Dynamics Attack    | global (g=1)   | 1 ms            | 1024 ms         | ×1/1024 | ×1024 | mul |
| Dynamics Release   | global (g=1)   | 1 ms            | 1024 ms         | ×1/1024 | ×1024 | mul |
| Dynamics Knee      | 6 dB (g=1)     | 0 dB (g=0)      | 48 dB (g=8)     | -1.0 | +7.0 | add |
| Dynamics Mix       | 100% (g=1)     | 0% (g=0)        | 100% (g=1)      | -1.0 |  0.0 | add |
| Ratio curves elsewhere | same as above | | | | | |
| Gain (dB mode)     | 0 dB (g=1)     | -18 dB (g≈0.126) | +18 dB (g≈7.94) | ×0.126 | ×7.94 | mul |
| Gain (Pull/Match)  | 100% (g=1)     | 0% (g=0)        | 100% (g=1)      | -1.0 |  0.0 | add |
| Phase Smear Amount | 100% (g=1)     | 0% (g=0)        | 200% (g=2)      | -1.0 | +1.0 | add |

Populate these during implementation by checking each module's `gain_to_display()` mapping — the values above are derived from today's formulas, not guessed.

**Files:**
- Modify: `src/editor/curve_config.rs` (extend `CurveDisplayConfig`, populate calibration per arm)
- Modify: `src/dsp/modules/mod.rs` (rewire `apply_curve_transform` to take `offset_fn`)
- Modify: `src/dsp/pipeline.rs` (pass `offset_fn` per slot/curve into the call)
- Modify: `src/editor_ui.rs:740–763` (offset knob custom_formatter shows physical)

- [ ] **Step 1:** Extend `CurveDisplayConfig` in `curve_config.rs`:

```rust
pub struct CurveDisplayConfig {
    // …existing fields…
    pub y_natural:  f32,
    /// Applies the normalized offset in gain space. offset_norm ∈ [-1, 1].
    /// Must be branch-free and alloc-free; called per bin on the audio thread.
    /// See docs/superpowers/specs/2026-04-23-ui-parameter-spec-design.md §2.
    pub offset_fn:  fn(g: f32, offset_norm: f32) -> f32,
}
```

Provide two reusable helpers in `curve_config.rs`:

```rust
/// Piecewise-linear additive offset: gain += offset * (neg_span if offset<0 else pos_span).
pub const fn add_offset(neg_span: f32, pos_span: f32) -> fn(f32, f32) -> f32 {
    // Can't actually close over constants in a const fn; instead define per-curve helpers below.
}

#[inline] fn add_thresh(g: f32, o: f32) -> f32 { if o >= 0.0 { g + o * 1.0 } else { g + o * 2.0 } }
#[inline] fn mul_attack(g: f32, o: f32) -> f32 { g * 1024.0_f32.powf(o) }
// …one small named fn per calibration profile…
```

Each curve arm picks the appropriate helper: `offset_fn: add_thresh`, `offset_fn: mul_attack`, etc.

- [ ] **Step 2:** Rewrite `apply_curve_transform` signature in `src/dsp/modules/mod.rs`:

```rust
pub fn apply_curve_transform(
    gains: &mut [f32],
    tilt: f32,
    offset: f32,
    curvature: f32,
    offset_fn: fn(f32, f32) -> f32,    // NEW
    sample_rate: f32,
    fft_size: usize,
)
```

Replace the inner `*g = ((*g + offset) * (1.0 + t)).max(0.0);` with:

```rust
let g_off = offset_fn(*g, offset);
*g = (g_off * (1.0 + t)).max(0.0);
```

- [ ] **Step 3:** In `src/dsp/pipeline.rs` (the loop updated in Task 1), look up the curve's offset_fn per slot/curve:

```rust
// Module type per slot is held in params.slot_module_types (already snapshotted into FxMatrix).
let cfg = curve_display_config(module_type, c, gain_mode);
apply_curve_transform(
    &mut self.slot_curve_cache[s][c],
    tilt, offset, curvature,
    cfg.offset_fn,
    self.sample_rate, self.fft_size,
);
```

`module_type` comes from the same source the matrix uses; snapshot once per block into an `[ModuleType; 9]` stack array. `gain_mode` comes from `params.gain_mode_param(s)` if the module is `Gain`.

- [ ] **Step 4:** In `editor_ui.rs:740–763`, add a `custom_formatter` to the offset DragValue that shows physical units. The formatter calls `cfg.offset_fn(1.0, offset_norm)` to derive the gain, then passes through the existing `gain_to_display()` with the cursor at neutral (or at 1 kHz for curves that vary by frequency — attack/release).

```rust
let display_str = {
    let g_at_neutral = cfg.offset_fn(1.0, off_norm);
    let phys = gain_to_display(module_type, editing_curve, g_at_neutral, ctx);
    format!("{phys:.1} {}", cfg.y_label)
};
```

- [ ] **Step 5:** Update `tests/module_trait.rs` fixtures that pass hardcoded offset scalars. Add a new test covering: threshold offset=+1 produces gain=2.0 at neutral (matches `add_thresh(1.0, 1.0) == 2.0`); attack offset=+1 produces gain=1024 (matches `mul_attack(1.0, 1.0) == 1024.0`).

- [ ] **Step 6:** `cargo test` and `cargo build --release`.

- [ ] **Step 7:** Commit: `feat: per-curve offset calibration (spec §2 DSP-side)`.

---

## Task 4: Nyquist-aware X-axis and tilt math

**Files:**
- Modify: `src/dsp/modules/mod.rs:240–254` (`apply_curve_transform`)
- Modify: `src/editor/curve.rs:473, 480, 565+` (HZ_LINES, HZ_LABELS, paint_grid)

- [ ] **Step 1:** Replace the hardcoded `LOG_RANGE: f32 = 3.0` and `PIVOT: f32 = 0.566_32` constants with values computed from `sample_rate / 2.0`:

```rust
let nyquist = sample_rate * 0.5;
let log_range = (nyquist / 20.0).log10();          // was 3.0 at 20 kHz
let pivot     = (1000.0_f32 / 20.0).log10() / log_range;
let s_pivot   = 3.0 * pivot * pivot - 2.0 * pivot * pivot * pivot;
```

Pull these out of the per-bin loop. Keep `LOG_20: f32 = 1.301_030` const.

- [ ] **Step 2:** Change `HZ_LINES` (curve.rs:473) and `HZ_LABELS` (curve.rs:480) from `static` arrays to functions that take `nyquist: f32` and emit values up to nyquist. Pattern:

```rust
fn hz_labels_for(nyquist: f32) -> &'static [(f32, &'static str)] {
    if nyquist >= 22_000.0 { &[(100., "100"), (1_000., "1k"), (10_000., "10k"), (20_000., "20k")] }
    else if nyquist >= 11_000.0 { &[(100., "100"), (1_000., "1k"), (10_000., "10k")] }
    else { &[(100., "100"), (1_000., "1k")] }
}
```

Plus a dynamic rightmost label at `nyquist` itself: format as `"{:.0}k"` for ≥1000 Hz.

- [ ] **Step 3:** Add `nyquist: f32` parameter to `paint_grid()` and any function that consumes `HZ_LINES`. Update call sites in `editor_ui.rs` to pass `params.sample_rate.load() * 0.5` (or whatever the established read is).

- [ ] **Step 4:** Update test fixtures in `tests/module_trait.rs` if any test touches `apply_curve_transform` with assumed-20 kHz numbers.

- [ ] **Step 5:** `cargo test` and `cargo build --release`.

- [ ] **Step 6:** Commit: `feat: Nyquist-aware X-axis and tilt math (no more hardcoded 20 kHz)`.

---

## Task 5: Grid/hover consume `CurveDisplayConfig`; render Y-label; remove Gain bespoke hover

**Files:**
- Modify: `src/editor/curve.rs:565+` (paint_grid uses CurveDisplayConfig.grid_lines)
- Modify: `src/editor/curve.rs:335+` (paint_hover_text takes &CurveDisplayConfig)
- Modify: `src/editor_ui.rs:471–504` (delete Gain Pull/Match bespoke hover path)
- Delete: `curve_grid_lines()` helper in `curve.rs` if now unused

- [ ] **Step 1:** Change `paint_grid()` signature to accept `cfg: &CurveDisplayConfig` (or look it up from `module_type` + `curve_idx` internally). Iterate `cfg.grid_lines` for horizontal lines and labels. Drop `curve_grid_lines()` helper.

- [ ] **Step 2:** In `paint_grid()`, after drawing grid lines, render `cfg.y_label` once at the top-left of the rect using `th::FONT_SIZE_LABEL`/`th::GRID_TEXT`.

- [ ] **Step 3:** Change `paint_hover_text()` signature from individual args to `(painter, pos, freq_hz, phys_value, cfg: &CurveDisplayConfig)`. Format becomes `"{freq} Hz  /  {value:.1} {cfg.y_label}"`.

- [ ] **Step 4:** In `editor_ui.rs:471–504`, delete the Gain Pull/Match special hover block. `curve_display_config()` already has the Pull/Match branch (`curve_config.rs:131–142`); the shared `paint_hover_text()` handles it via `cfg.y_label = "%"`.

- [ ] **Step 5:** `cargo test` and `cargo build --release`. Verify: hovering over a Gain Pull curve still shows percent; hovering over a threshold curve shows dBFS.

- [ ] **Step 6:** Commit: `feat(ui): grid and hover consume CurveDisplayConfig; render Y-label`.

---

## Task 6: UI scale factor frame-scoped; eliminate pixel literals

**Files:**
- Modify: `src/editor_ui.rs` (read `pixels_per_point` once at frame top, pass `scale: f32` down)
- Modify: `src/editor/theme.rs` (add font/stroke constants used by remaining literals)
- Modify: `src/editor/fx_matrix_grid.rs` (6 font literals, 3 stroke literals)
- Modify: `src/editor/module_popup.rs:90, 105` (`.size(8.0)`)
- Modify: `src/editor/curve.rs:721` (`Stroke::new(1.0, dim)`)

- [ ] **Step 1:** In `theme.rs`, add the missing base constants. Inventory:
  - `FONT_SIZE_VALUE: f32 = 11.0` (editor_ui.rs button labels)
  - `FONT_SIZE_TINY: f32 = 8.0` (module_popup secondary text)
  - `FONT_SIZE_MATRIX_AXIS: f32 = 7.5` (fx_matrix axis labels)
  - `FONT_SIZE_MATRIX_CELL: f32 = 8.0` (matrix cell text)
  - `FONT_SIZE_MATRIX_VROW: f32 = 7.0` (T/S virtual-row icon)
  - `STROKE_HAIRLINE: f32 = 0.5`
  - `STROKE_MEDIUM: f32 = 1.5`

- [ ] **Step 2:** In `create_editor()`, read `let scale = ctx.pixels_per_point();` once at the top of the frame closure and thread it through to widget calls that paint manually (`fx_matrix_grid::show`, `module_popup::show_popup`, `curve_widget`, etc.) as a `scale: f32` parameter.

- [ ] **Step 3:** Migrate every literal in the four target files using `scaled(BASE, scale)` for layout/font and `scaled_stroke(BASE, scale)` for stroke widths. Pattern for each `.size(N)`:

```rust
.size(th::scaled(th::FONT_SIZE_VALUE, scale))
```

For `FontId::proportional(N)`:
```rust
egui::FontId::proportional(th::scaled(th::FONT_SIZE_MATRIX_AXIS, scale))
```

For `Stroke::new(N, color)`:
```rust
egui::Stroke::new(th::scaled_stroke(th::STROKE_HAIRLINE, scale), color)
```

- [ ] **Step 4:** Add a `clippy::disallowed_methods` lint config (or document it) forbidding new pixel literals in `editor_ui.rs` / `editor/*.rs`. Optional but worth it.

- [ ] **Step 5:** `cargo test`, `cargo build --release`. Visual smoke test at 1× and 2× scale (Bitwig honours `pixels_per_point` per system DPI).

- [ ] **Step 6:** Commit: `feat(ui): frame-scoped UI scale; eliminate pixel literals per spec §4`.

---

## Task 7: Contract comments

**Files:**
- Modify: `src/editor/spectrum_display.rs` (top of every public paint function)
- Modify: `src/editor/suppression_display.rs`
- Modify: `src/editor/fx_matrix_grid.rs`
- Modify: `src/editor/module_popup.rs`
- Modify: `src/editor_ui.rs` (curve-drawing blocks: above the per-curve paint loop, above hover handling, above grid/spectrum dispatch)

- [ ] **Step 1:** Add the contract comment above each function that paints curve display, hover text, grid, spectrum, suppression, or matrix:

```rust
// UI parameter contract: see docs/superpowers/specs/2026-04-23-ui-parameter-spec-design.md
```

- [ ] **Step 2:** `cargo build --release`. Expected: clean (comments only).

- [ ] **Step 3:** Commit: `docs: add UI parameter contract comments per spec §5`.

---

## Verification

After all 7 tasks:

```bash
cargo test                                            # all tests passing
cargo build --release                                 # clean
cargo run --package xtask -- bundle spectral_forge --release
cp target/bundled/spectral_forge.clap ~/.clap/
```

In Bitwig:
- Load any module slot. Drag offset/tilt/curvature knobs → confirm audio responds.
- Right-click an automation lane on offset → confirm host sees the param.
- Change Bitwig's UI scale (or system HiDPI) → confirm fonts/strokes scale cleanly.
- Hover over a Gain Pull curve → confirm hover text reads `"… Hz  /  N.N %"`.
- Switch sample rate from 48 kHz to 96 kHz → confirm rightmost X-axis label updates from "20k" to e.g. "48k".
