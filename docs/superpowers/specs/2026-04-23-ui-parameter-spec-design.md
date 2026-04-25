> **Status (2026-04-24): IMPLEMENTED and LIVE.** This is the authoritative source of truth for curve display, transforms, axis rendering, hover text, and UI scaling — it remains normative even though the matching plan is merged. Addenda §2.3 / §3.4 / §4.4 are pending from calibration-audit T10. See [../STATUS.md](../STATUS.md).

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
