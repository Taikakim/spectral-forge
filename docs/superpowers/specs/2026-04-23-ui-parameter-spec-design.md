> **Status (2026-05-04): IMPLEMENTED with addenda §7 and §8 PENDING IMPLEMENTATION.** This is the authoritative source of truth for curve display, transforms, axis rendering, hover text, and UI scaling. Existing addenda §2.3 (calibration contract), §3.4 (curve/node rendering at limits), and §4.4 (control row consistency) are LIVE in the codebase. New addenda §7 (internal parameter ranges) and §8 (per-mode CurveLayout + help-box) are normative for the next implementation pass — they are part of the per-module UX overhaul that begins with Past (see [`2026-05-04-past-module-ux-design.md`](2026-05-04-past-module-ux-design.md)). See [../STATUS.md](../STATUS.md).

# Spectral Forge — UI Parameter Specification

> **This document is the authoritative source of truth for all UI parameter display behaviour.**
> Any agent or developer touching curve display, transforms, axis rendering, hover text, or UI
> scaling MUST follow this spec exactly. If a situation arises where following the spec is
> unclear or would cause a problem, STOP and ask rather than guessing.

---

## Purpose

This spec exists to prevent display parameter drift across versions. Previously, values for
offset ranges, tilt scaling, grid lines, and UI scale factors were scattered across functions
and changed unpredictably. Everything is now defined here first; code implements it by reference.

---

## 1. CurveDisplayConfig — the single source of truth

A new file `src/editor/curve_config.rs` owns the `CurveDisplayConfig` struct and the
`curve_display_config()` function. **No display range, grid value, or unit label may be
defined anywhere else.**

```rust
/// See docs/superpowers/specs/2026-04-23-ui-parameter-spec-design.md for all values.
pub struct CurveDisplayConfig {
    pub y_label:      &'static str,   // axis unit label: "dBFS", "ratio", "ms", "dB", "%"
    pub y_min:        f32,            // physical bottom of display range
    pub y_max:        f32,            // physical top of display range
    pub y_log:        bool,           // true = log Y spacing (ratio, attack, release)
    pub grid_lines:   [f32; 4],       // 4 physical values for horizontal guide lines
    pub gain_to_phys: fn(f32) -> f32, // converts raw curve gain multiplier → physical unit
}

/// Returns display config for a given module type and curve index.
/// See docs/superpowers/specs/2026-04-23-ui-parameter-spec-design.md §1.
pub fn curve_display_config(module_type: ModuleType, curve_idx: usize) -> CurveDisplayConfig
```

### Canonical values — Dynamics module

| Curve | y_label | y_min | y_max | y_log | grid_lines |
|-------|---------|-------|-------|-------|------------|
| 0 Threshold | "dBFS"  | -60.0 | 0.0   | false | [-12, -24, -36, -48] |
| 1 Ratio     | "ratio" | 1.0   | 20.0  | true  | [1.5, 2.5, 5.0, 10.0] |
| 2 Attack    | "ms"    | 1.0   | 1024.0| true  | [4.0, 16.0, 64.0, 256.0] |
| 3 Release   | "ms"    | 1.0   | 1024.0| true  | [4.0, 16.0, 64.0, 256.0] |
| 4 Knee      | "dB"    | 0.0   | 48.0  | false | [6.0, 12.0, 24.0, 36.0] |
| 5 Makeup    | "dB"    | -36.0 | 36.0  | false | [-24.0, -12.0, 12.0, 24.0] |
| 6 Mix       | "%"     | 0.0   | 100.0 | false | [25.0, 50.0, 75.0, 100.0] |

Other module types (Freeze, PhaseSmear, Contrast, Gain, MidSide, TsSplit, Harmonic) each have
their own match arms in `curve_display_config()`. Any new module MUST add its arm there before
any display code is written.

---

## 2. Per-curve transforms: offset, tilt, curvature

All three transforms are per-curve (9 slots × 7 curves each). They are stored together in a
`CurveTransform` struct, replacing the old `(f32, f32)` tilt+offset tuple in `slot_curve_meta`.

```rust
/// Per-curve display transform. See docs/superpowers/specs/2026-04-23-ui-parameter-spec-design.md §2.
pub struct CurveTransform {
    pub offset:    f32,  // [-1.0, 1.0] → maps linearly to [y_min, y_max] of the curve's config
    pub tilt:      f32,  // [-1.0, 1.0] → ±45° effective slope across log-frequency
    pub curvature: f32,  // [0.0,  1.0] → 0 = straight tilt, 1 = full S-curve (see below)
}
```

### Offset

Shifts the curve's neutral origin across the full physical display range. At `offset = 0.0`
the origin is at the curve's natural neutral (e.g. -20 dBFS for threshold, 1.0 for ratio).
At `offset = -1.0` the origin sits at `y_min`; at `+1.0` it sits at `y_max`. The mapping is
linear between these three anchor points. The `gain_to_phys` function handles unit conversion
internally — the offset rule is universal across all curve types.

### Tilt

Applies a slope across log-frequency space. At `tilt = ±1.0` the result is a ±45° effective
slope in display coordinates. At `curvature = 0` the tilt is linear in normalized log-frequency.

### Curvature

Bends the tilt into an S-shape that is perpendicular to the tilt direction, pivoting at ~1 kHz.

- At `curvature = 0`: tilt is a straight diagonal (current behaviour).
- At `curvature = 1`: tilt is applied through a smoothstep function (3x² − 2x³) in normalized
  log-frequency space, creating maximum sigmoid bend at the pivot and tapering flat at both ends.
- Intermediate values blend linearly between straight and full smoothstep.

Implementation sketch (in `apply_curve_transform`):
```rust
let x_norm = log10(freq_hz / 20.0) / log10(nyquist / 20.0);  // 0..1 in log-freq
let linear  = x_norm - 0.5;
let sigmoid = smoothstep(x_norm) - 0.5;  // smoothstep = 3x²-2x³
let shape   = lerp(linear, sigmoid, curvature);
let tilt_gain = 1.0 + tilt * shape * tilt_scale; // tilt_scale sets the ±45° mapping
```

### Storage migration

`slot_curve_meta` type changes from `[[(f32, f32); 7]; 9]` to `[[CurveTransform; 7]; 9]`.
Migration: read old `(tilt, offset)` tuple → `CurveTransform { tilt, offset, curvature: 0.0 }`.

### §2.3 Calibration contract

Every module's internal DSP must accept the full range implied by its curve's
declared `offset_fn` extremes. When the normalized `offset` is +1, the
DSP-observed parameter must reach the config's `y_max`; when `offset` is -1,
it must reach `y_min`. If a module clamps for DSP safety, the clamp values
MUST match `y_min` and `y_max`. Any tighter clamp is a bug.

This contract is verified end-to-end by `tests/calibration_roundtrip.rs`.
New modules MUST add themselves to that test's case table when they are
introduced.

---

## 3. Axes, grid lines, and hover text

### X-axis

- Always log-scaled frequency.
- Range: **20 Hz to Nyquist** (`sample_rate / 2`), derived from host sample rate each frame.
- Rightmost label shows the Nyquist frequency (e.g. "48 kHz" at 96 kHz sample rate).
- All painting functions that map X position receive `nyquist: f32` as a parameter. No function
  may hardcode 20 000 Hz as the X-axis maximum.
- X position formula: `x = rect.left + rect.width * log10(f / 20.0) / log10(nyquist / 20.0)`

### Y-axis

- The **active curve's** `y_label` is always rendered on the Y-axis.
- Grid lines use the 4 values from `CurveDisplayConfig::grid_lines`.
- Spacing follows `y_log`: logarithmic if true, linear if false.
- Grid lines reposition automatically when the display range changes (e.g. threshold follows
  `db_min`/`db_max`). This is already implemented; the spec makes it a hard requirement.

### Hover text

A single shared routine in `curve.rs` handles hover display for all curves:

```rust
/// See docs/superpowers/specs/2026-04-23-ui-parameter-spec-design.md §3.
fn paint_hover_text(painter, pos, freq_hz: f32, phys_value: f32, config: &CurveDisplayConfig)
```

Format: `"440 Hz  /  -18.3 dBFS"` (frequency left, physical value + unit label right).

**Rule:** No curve may implement its own hover text path. Every hover display goes through this
function. The physical value is computed by `config.gain_to_phys(gain)` at the cursor's bin.

### §3.4 Curve and node rendering at limits

- Curve values outside `[y_min, y_max]` are rendered as a flat line along the
  exceeded border (top or bottom edge of the graph), not omitted.
- Curve nodes whose computed y-position is outside the graph are drawn
  truncated to the border with the dot still fully visible.
- When a node is being dragged, its virtual (un-clipped) physical value is
  shown in the hover tooltip.
- Each curve config declares its allowed `[y_min, y_max]`; the UI renderer
  is the sole place that enforces the visual clip.

---

## 4. UI scaling rules

The UI scale factor is read once per frame as `ctx.pixels_per_point()` and passed down to
painting functions. It is never re-read inside individual drawing calls.

### Helpers (in `theme.rs`)

```rust
/// Scale a layout measurement (padding, radius, etc.) by the UI scale factor.
/// See docs/superpowers/specs/2026-04-23-ui-parameter-spec-design.md §4.
pub fn scaled(base: f32, scale: f32) -> f32 { base * scale }

/// Scale a stroke width. Snaps to 2× for scale ≥ 1.75 to avoid blurry sub-pixel lines.
/// See docs/superpowers/specs/2026-04-23-ui-parameter-spec-design.md §4.
pub fn scaled_stroke(base: f32, scale: f32) -> f32 {
    if scale >= 1.75 { base * 2.0 } else { base * scale }
}
```

### Rules

1. **All base sizes** are defined in `theme.rs` as `f32` constants at 1× scale.
   Example: `pub const STROKE_THIN: f32 = 1.0;`
   No pixel literal may appear in drawing code outside `theme.rs`.

2. **All stroke widths** use `scaled_stroke(STROKE_THIN, scale)`. Never a raw literal.

3. **All layout measurements** (padding, node radius, drag hit areas) use `scaled(BASE, scale)`.

4. **Font sizes** are defined as base pt in `theme.rs` and constructed as:
   `FontId::proportional(scaled(FONT_SIZE_LABEL, scale))`
   Font sizes are never set from a literal in drawing code.

5. **At 1×** the visual output is identical to the pre-spec state.
   **At 2×** every 1px line is 2px, every hit area proportionally larger, fonts are sharp.
   **At 1.25×–1.5×** sub-pixel AA handles fractional widths.
   **At 1.75×+** stroke widths snap to the 2× integer value.

### §4.4 Control row consistency

The Offset / Tilt / Curve DragValue row is rendered at a fixed vertical
position per slot, identical across all module types. Modules may not define
their own layout for these controls. The row is drawn by a single shared code
path in `editor_ui.rs` regardless of the slot's module type or curve count.

---

## 5. Reference in code

Every function that participates in curve display must carry an opening comment:

```rust
// UI parameter contract: see docs/superpowers/specs/2026-04-23-ui-parameter-spec-design.md
```

`CLAUDE.md` contains a section pointing agents to this file before touching any display code.

---

## 6. Extension checklist

When adding a new module type or curve:

- [ ] Add a `curve_display_config()` match arm in `curve_config.rs` with all 5 fields defined.
- [ ] Verify `gain_to_phys` covers the full `[y_min, y_max]` range without clamping surprises.
- [ ] Confirm grid lines are 4 values, sensibly spaced for the unit (linear or log).
- [ ] Run `cargo test` — the display config table is covered by a test asserting all
      `ModuleType` variants return a valid config.
- [ ] If the new curve has a unique unit, add its `y_label` string as a `const` in `curve_config.rs`.

---

## 7. Internal parameter ranges — -1..1 vs 0..1

Some per-curve parameters use **signed** internal ranges that look like 0..1 but aren't. Code consuming these MUST accept the full signed range; clamping at 0 silently throws away half the parameter.

| Parameter | Internal range | Notes |
|---|---|---|
| Curve node `y` (`s{s}c{c}n{n}_y`)         | -1.0 .. +1.0 | Maps via `compute_curve_response` to ~0.126× .. 7.94× linear gain (±18 dB). |
| Curve node `x`, `q`                       | 0.0 .. 1.0   | x = log-frequency normalised, q = bandwidth normalised. |
| Per-curve **tilt** (`s{s}c{c}_tilt`)      | -1.0 .. +1.0 | Multiplied by `TILT_MAX` for gain-space slope. |
| Per-curve **offset** (`s{s}c{c}_offset`)  | -1.0 .. +1.0 | Passed to `CurveDisplayConfig::offset_fn`. |
| Per-curve curvature (`s{s}c{c}_curv`)     | 0.0 .. 1.0   | S-curve blend: 0 = straight tilt, 1 = full smoothstep. |

### Common pitfalls

1. **Asymmetric `offset_fn`.** If the function only does something on one side of 0 (e.g. `off_mix` returns `g` unchanged for positive offset), the slider stops responding past 0 and the user perceives it as broken. Either use a symmetric `offset_fn` (`g + o`, `g * factor.powf(o)`, or a piecewise like `if o >= 0 { g + a*o } else { g + b*o }`) **or** explicitly state in the curve's spec why the asymmetry is intentional (e.g. y_natural is at y_max and there's no headroom to extend up).

2. **Silent clamping at 0.** Code that does `param.value().clamp(0.0, 1.0)` on a -1..1 parameter throws away the negative half without warning. Use `.clamp(-1.0, 1.0)` or the parameter's declared bounds.

3. **Default-as-mid assumption.** For -1..1 params the neutral value is **0.0**, not 0.5. Code computing "distance from default" must use 0.0 as the anchor.

4. **Display formatter that ignores the offset value.** A `custom_formatter` that computes a constant phys reading (because it calls an `off_identity` `offset_fn` against gain=1.0) shows a frozen number on screen. The slider still mutates the param internally, but the user can't see the change. The fallback when `y_label` is empty is to show the raw normalised value (`{:+.2}`) so the drag is visible during a UI rebuild.

### `default_config()` is intentionally inert

`curve_config::default_config()` returns `offset_fn: off_identity`. Modules that fall through to it (no explicit per-module arm in `curve_display_config()`) get an offset slider that updates visually (raw `{:+.2}` value) but has no audible effect — by design, until the module ships its own calibrated config. Earlier we tried `off_mix` here as a "do *something*" fallback; that's an asymmetric offset_fn (pitfall 1) and produced exactly the "stops past 0" complaint. Don't use it.

---

## 8. Per-mode CurveLayout — active curves, label overrides, help text

Modules with internal sub-modes (Past, Geometry, Circuit, Life, Kinetics, Harmony, Modulate, Rhythm) typically use only a *subset* of their declared `num_curves` per mode, sometimes with mode-specific labels (e.g. Past's curve 1 is "Age" in Granular but "Delay" in Convolution). The legacy approach of always rendering all `num_curves` tabs leaves dead controls visible and lets users draw curves the active mode silently ignores.

### `CurveLayout` struct

```rust
/// Per-mode descriptor for visible curves, label overrides, and help-box copy.
/// See docs/superpowers/specs/2026-04-23-ui-parameter-spec-design.md §8.
pub struct CurveLayout {
    /// Indices (into the module's full curve set) of curves visible for this mode.
    /// Order is render order; e.g. `&[0, 2, 4]` hides curves 1 and 3 entirely.
    pub active: &'static [u8],

    /// Per-curve label overrides for this mode. Each tuple is (curve_idx, override_label).
    /// Curves not listed fall back to `ModuleSpec::curve_labels[curve_idx]`.
    pub label_overrides: &'static [(u8, &'static str)],

    /// Help-box copy keyed by curve_idx (full curve index, not position in `active`).
    /// Returning an empty string means "use the module's general help text."
    pub help_for: fn(curve_idx: u8) -> &'static str,

    /// Help-box module overview shown when a slot is selected but no curve is in focus.
    /// `None` ⇒ use the module's static description.
    pub mode_overview: Option<&'static str>,
}
```

### `ModuleSpec` field

```rust
pub active_layout: Option<fn(mode: u8) -> CurveLayout>,
```

When `None` (modules without modes — Dynamics, Freeze, etc.), the UI renders all `curve_labels` as today. When `Some`, the UI looks up the layout for the slot's current mode and renders only the active curves with their (overridden) labels and help-box copy. Mode is encoded as `u8` because every module's mode enum already derives `as u8`.

### Help-box infrastructure

A help panel renders to the right of the FX matrix, occupying space currently empty. It shows:

1. **Module overview** when a slot is selected but no curve is in focus — pulled from `mode_overview` if `Some`, else the module's general description.
2. **Per-curve summary** when a curve is selected — pulled from `help_for(curve_idx)`.

Help text is `&'static str` (no allocation, no per-frame formatting). Help-panel layout (font, padding, max width, scroll behaviour) is defined in `theme.rs` alongside the existing display constants. Width is fixed at design-time; height matches the matrix region.

### Tab-strip behaviour with `CurveLayout`

When a slot's mode changes (via the popup), the visible curve tabs re-shape to match the new layout's `active` list. The `editing_curve` cursor clamps to the first active curve if the previously-edited curve is no longer active. The Offset / Tilt / Curve DragValue row (per §4.4) renders only if the currently-focused curve is in `active`.

### Extension checklist (adds to §6)

When adding a new mode-bearing module:

- [ ] Define `*_layout(mode: u8) -> CurveLayout` for every mode the module ships, returning the active curve set, label overrides, and per-curve help text.
- [ ] Wire it as `active_layout: Some(my_module::active_layout)` on the `ModuleSpec` literal.
- [ ] Write `mode_overview` text for every mode (or set `None` and use the module's static description).
- [ ] Verify the active set matches what the DSP actually consumes — a curve listed as active but ignored by DSP, or read by DSP but not in `active`, is the *same kind* of bug the legacy "always show 5 tabs" approach produced. The whole point of `active` is that it tells the truth.
- [ ] Add a `tests/<module>_layout.rs` regression assertion: every visible curve in every layout has non-empty `help_for(idx)`.
