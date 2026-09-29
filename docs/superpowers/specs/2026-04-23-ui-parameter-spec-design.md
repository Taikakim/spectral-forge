> **Status (2026-09-29): IMPLEMENTED.** This is the authoritative source of truth for curve display, transforms, axis rendering, hover text, and UI scaling. Addenda §2.3 (calibration contract), §3.4 (curve/node rendering at limits), and §4.4 (control row consistency) are LIVE in the codebase. §8 (per-mode CurveLayout + help-box) is LIVE: it landed with the Past UX overhaul (`94208ed`..`99e03f4`, see [`2026-05-04-past-module-ux-design.md`](2026-05-04-past-module-ux-design.md)) and per-mode `active_layout` was then added to the other multi-mode modules (`a8e2a5b` … `44992d9`). §7 (internal parameter ranges) is a normative reference for code consuming these params; §9 records lessons learned (2026-05-08). See [../STATUS.md](../STATUS.md).

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

### §1.4 Threshold dBFS curves

Curves whose physical unit is dBFS and whose neutral is `−20 dBFS` (display
indices 0 and 9) use a logarithmic gain→dBFS mapping:

    threshold_db = clamp(-20 + 20·log10(gain) · (60/18), y_min, y_max)

This guarantees that a full ±18 dB EQ-node excursion sweeps the entire display
range, so node moves alone reach `y_min` (the lower clamp). DSP modules that
gate by the displayed threshold MUST share this mapping —
`freeze::curve_to_threshold_db` is the canonical implementation; both Freeze
and PAST consume it (PAST via `curve_gain_to_threshold_lin`).

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
At `offset = -1.0` the origin sits at `y_min`; at `+1.0` it sits at `y_max`.

**Lerp shape between anchor points is axis-aware**, governed by `cfg.y_log`:

- **`y_log == false`** (linear axis, e.g. dBFS, %, dB): linear interpolation in physical units.
  - `v ≥ 0`: `phys = y_natural + v · (y_max − y_natural)`
  - `v < 0`: `phys = y_natural + v · (y_natural − y_min)`
- **`y_log == true`** (log axis, e.g. ms, ratio): geometric (logarithmic) interpolation in
  physical units. The slider's midpoint reads as the **geometric midpoint** of the range,
  which lands at the visual middle of the log axis (the natural drag feel for a log display).
  - `v ≥ 0`: `phys = y_natural · (y_max / y_natural)^v`
  - `v < 0`: `phys = y_natural · (y_natural / y_min)^v`

Both the offset slider's `custom_formatter` and the response-curve render path MUST use this
same axis-aware lerp. WYSIWYG is mandatory: the value displayed in the slider equals the
physical value at which the curve's flat (un-noded) regions render.

The `offset_fn` paired with each curve produces a `gain_off` that, when fed through
`gain_to_display`, returns this same `phys` value. For curves where `gain_to_display` is
linear in gain (the default for `%`, `ms × multiplier`, etc.), the existing additive or
multiplicative `offset_fn` already satisfies this. For curves where `gain_to_display` is
logarithmic in gain (the dBFS thresholds at display indices 0 and 9), `offset_fn` is a
multiplicative function whose log produces the required `phys`. See §3.1 of
`2026-05-05-graph-display-correctness.md` for the calibration recipes.

### Tilt

Applies a slope across log-frequency space. The normalized `tilt ∈ [-1, +1]`
parameter is multiplied by `TILT_MAX` (4.0 dB/oct as of 2026-05-08) by both
the audio-thread DSP path (`pipeline.rs::process` → `apply_curve_transform`)
AND the GUI display path (`editor_ui.rs` → `slot_meta` → `paint_response_curve`).
Both paths MUST scale the same way — earlier display showed only `tilt_norm * shape`
without the TILT_MAX multiplier, so the visible curve was 1/N of the audible
slope (commit `bd00a2a`). At `curvature = 0` the tilt is linear in normalized
log-frequency.

**Tilt formula (2026-05-08, spec-literal):** tilt is applied as
`g * 10^(tilt_dB_per_oct * octaves_from_pivot / 20)`, i.e. multiplicative
in dB-space. `tilt_dB_per_oct = tilt_norm * TILT_MAX`, `octaves_from_pivot
= log2(freq / 1000)` warped by the curvature blend. This guarantees:
- Symmetric behaviour: positive and negative tilt are exact inverses.
- No saturation: gain never collapses to zero from tilt alone.
- Honest "dB/oct" semantics: a tilt of ±TILT_MAX dB/oct shifts the
  curve by exactly that many dB per octave away from 1 kHz.

The earlier `g * (1 + t)` form had a hard cliff when `(1 + t) < 0` on
one side at large `|tilt|`, producing visibly broken behaviour at
`TILT_MAX = 4` (commit replacing this notes "doesn't fire threshold
check at high tilt"). The dB-space form removes the cliff while keeping
the visible slope similar at moderate tilt.

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
let octaves   = shape * (log10(nyquist / 20.0) / log10(2.0));
let tilt_gain = 10f32.powf(tilt_norm * TILT_MAX * octaves / 20.0);
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

**Neutral consistency:** `cfg.y_natural` MUST equal `gain_to_display(display_idx, 1.0, …)`
for the curve. If a module wants the slider's neutral to land on a different physical
value, the right answer is to change `gain_to_display` for that display index (or use a
different display index), not to lie via `y_natural`. A divergence here breaks WYSIWYG at
`v = 0`.

**Clamp consistency:** `gain_to_display` per-index clamps (e.g. `clamp(1.5, 48)` on the
knee axis) MUST match the `cfg.y_min` / `cfg.y_max` declared by every config that uses that
display index. If two curves with different ranges share a display index, either widen the
clamp (preferred) or split into two display indices.

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

### §3.5 Headroom strip above y_max (2026-05-08)

A fixed visual strip of `theme::HEADROOM_PX` pixels (50 at 1× scale,
scaled via `th::scaled`) sits ABOVE the y_max grid line in the curve
area, reserving space so loud bins above unity and dragged-up nodes
don't hit the top edge of the graph.

Implementation: `editor::curve::db_inner_rect(rect, scale)` shrinks
`rect` from the top by `HEADROOM_PX * scale` and returns the inner
rect that every db→y mapping site uses. Vertical Hz lines and Hz
labels still span the FULL `rect`; only horizontal grid lines, the
y-axis label, response curves, the SC envelope overlay, hover-text
y-conversion, the curve_widget node dots, and the spectrum gradient
use the inner rect.

The clamps in `linear_to_y`, `log_to_y`, `db_y`, and every
`gain_to_display` arm have been relaxed at the TOP (kept at the
bottom): values above y_max produce y above `inner_rect.top()`,
flowing into the headroom strip up to the outer rect's top where
egui's painter clips. Floor values still pin to `rect.bottom()`.

The virtual node range `−2..+2` and the red off-rect indicator
triangles continue to use the FULL outer rect, so virtual nodes
draw into and beyond the headroom strip with the same indicator
they had before.

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

### Help-box infrastructure (revised 2026-05-08)

A help panel renders to the right of the FX matrix, occupying space currently empty. It shows, in order of precedence:

1. **Per-widget topic** — set transiently by `track_help()` /
   `track_help_strings()` while the user hovers/drags any registered
   widget (top bar buttons, sliders, mode buttons, matrix cells, popup
   options, curve-tab tabs, etc.). Cleared at the top of every frame
   via `promote_focus()`.
2. **Per-curve / per-mode summary** when a curve is selected — pulled
   from the active layout's `help_for(curve_idx)`, or the centralized
   `multi_mode_curve_help(ty, mode_byte, curve_idx)` /
   `single_mode_curve_help(ty, curve_idx)` tables in
   `editor::help_box`.
3. **Mode overview** when a curve isn't in `active` — pulled from
   `mode_overview` if `Some`.
4. **Module fallback** — short module-level description with curve
   label appended.

**Pending → presented promotion model.** Widgets write claims to a
"pending" key during the frame; at the top of the NEXT frame
`promote_focus()` copies pending → presented and clears pending. The
help-box draws from "presented". This 1-frame indirection is what
lets popups (rendered AFTER the help-box draw call) still surface
their help text. Imperceptible at refresh-rate cadence.

**Width / wrap.** Help-box width is fixed via `th::HELP_BOX_WIDTH`.
Both heading and body use `egui::Label::new(RichText::new(...)).wrap()`
— two attempts to switch the body to LayoutJob both regressed wrap
(per-character first, then single-line overflow). The yellow inline
"Feedback" prefix used by routing-matrix feedback cells renders on
its own short row above the body when present.

**Toggle.** A `help_enabled: BoolParam` (default true, host-persisted)
controls whether the help-box renders content. When off, a "Help (off)"
placeholder keeps the layout from collapsing.

Help text is `&'static str` for static topics; dynamic per-frame
summaries (matrix-cell flow text) use owned `String` via the
`HelpFocus::Custom` variant. Help-panel layout (font, padding, max
width, scroll behaviour) is defined in `theme.rs` alongside the
existing display constants.

### Tab-strip behaviour with `CurveLayout`

When a slot's mode changes (via the popup), the visible curve tabs re-shape to match the new layout's `active` list. The `editing_curve` cursor clamps to the first active curve if the previously-edited curve is no longer active. The Offset / Tilt / Curve DragValue row (per §4.4) renders only if the currently-focused curve is in `active`.

### Extension checklist (adds to §6)

When adding a new mode-bearing module:

- [ ] Define `*_layout(mode: u8) -> CurveLayout` for every mode the module ships, returning the active curve set, label overrides, and per-curve help text.
- [ ] Wire it as `active_layout: Some(my_module::active_layout)` on the `ModuleSpec` literal.
- [ ] Write `mode_overview` text for every mode (or set `None` and use the module's static description).
- [ ] Verify the active set matches what the DSP actually consumes — a curve listed as active but ignored by DSP, or read by DSP but not in `active`, is the *same kind* of bug the legacy "always show 5 tabs" approach produced. The whole point of `active` is that it tells the truth.
- [ ] Add a `tests/<module>_layout.rs` regression assertion: every visible curve in every layout has non-empty `help_for(idx)`.

## 9. Lessons learned (2026-05-08)

Patterns that surfaced repeatedly during the D + E sweep and the help-system overhauls. Useful future-proofing:

### 9.1 Slider–graph WYSIWYG depends on three matched stores

The slider text shows `axis_aware_lerp(cfg, anchors, v)`. The graph response uses `gain_to_display(display_idx, offset_fn(g, v, anchors))`. They MUST agree at v ∈ {-1, 0, +1} (the calibration test in `tests/curve_calibration_matrix.rs` enforces this), AND they must agree at intermediate v. The `cfg.y_natural` value must equal `gain_to_display(display_idx, 1.0)` — if they don't, the slider and graph will read different values for "neutral curve, no offset". Several recent bugs (Gain Pull/Match, Phase Smear PEAK HOLD) were exactly this mismatch.

### 9.2 `default_nodes_for_curve(idx)` is Dynamics-specific

`curve_idx == 1` returns RATIO-style defaults (high shelf at 20 Hz, y=0.334) which produces a ~2× boost in the baseline curve gain across the audible band. For any module whose curve at index 1 isn't actually a ratio (Mid-Side EXPANSION, Future TIME, Punch WIDTH, Rhythm DIVISION, Geometry MODE_CAP, Kinetics MASS, Harmony THRESHOLD, etc.) this boost manifests as the "graph parks at 200 % / slider shows 100 %" desync. `default_nodes_for_module_curve` now routes only Dynamics through the legacy fallback; everyone else gets `default_nodes()` (flat y=0).

### 9.3 egui temp-data keys must use stable Ids across writers/readers

`ui.id().with("foo")` produces an ID derived from the current UI scope. If `open_at` is called from a deeply-nested closure and `show_popup` is called from the outer central panel, their `ui.id()` values differ → different keys → state set by open is invisible to show. Use `egui::Id::new("foo_state")` instead — global to the egui context, identical from any scope. This bit Circuit/Life/Kinetics/Harmony popups; they all silently failed to open until the keys were unified.

### 9.4 Help-box must be drawn LATE for popup widgets to claim focus

Any widget rendered AFTER `help_box::draw` cannot influence the help-box in the same frame because the box has already painted. Two viable architectures:
- Draw the help-box at the very end of the frame. Layout-disruptive in our current horizontal_top design.
- Use a pending → presented promotion: widgets write to "pending" anytime in the frame; at the START of the NEXT frame, copy pending → presented and clear pending; help-box reads from "presented". 1-frame lag, imperceptible. **This is what we shipped.**

### 9.5 `Label::wrap()` works for `RichText`, fails for `LayoutJob` in this nih-plug-egui

For text body content inside a constrained `set_width` Frame, the only reliable wrap pattern is:
```rust
ui.add(Label::new(RichText::new(text).color(c).font(f)).wrap());
```
Wrapping multi-style text via `LayoutJob` regressed twice (per-character first, then single-line overflow extending past the plugin border). The accepted compromise: yellow inline prefix moves to its own short row above the body, both rendered as separate Labels with `RichText + .wrap()`.

### 9.6 Display-vs-DSP scaling: tilt's TILT_MAX, threshold's norm_factor

When a normalized `[−1, +1]` parameter feeds both display and DSP, the SAME multiplier must be applied on both sides. Three-way bugs surfaced during this sweep:
- Tilt: DSP multiplied by `TILT_MAX = 4.0` but display didn't, so the visible curve was 1/4 of the audible slope. Fixed by scaling at the slot_meta read site.
- Freeze threshold: DSP compared raw FFT bin magnitude against threshold_lin without `fft_size/4` scaling, so the threshold was effectively 54 dB lower than the display showed. Fixed earlier.
- PAST kernels: same issue, fixed earlier (norm_factor multiplier).

The pattern: any time a magnitude-domain comparison happens in the DSP, both sides must be in the same scale (raw bin vs amplitude-equivalent vs dB). The `norm_factor = fft_size / 4` convention is now applied in Freeze, PAST (all 5 modes), and PhaseSmear PEAK HOLD.

### 9.7 Upper clamps in display functions defeat headroom

If `gain_to_display(idx, gain).clamp(min, max)` exists at the display function level, ANY headroom strip in the painter is wasted — the curve hits a hard ceiling at `max` before the y-mapping gets to render. The D-2 headroom only works because the clamps were demoted to `.max(min)` (floor only, no ceiling). Floors stay because below-floor values would trigger NaN/log10(negative).

### 9.8 Test the DEFAULT, not just calibrated extremes

The `tests/curve_calibration_matrix.rs` test verifies offset_fn endpoints (v=±1) match axis_aware_lerp. It does NOT verify the curve gain default (1.0) maps via gain_to_display to the cfg's y_natural. That's the gap the Gain Pull/Match bug fell into — endpoints matched, neutral didn't. A future test should pin `gain_to_display(display_idx, 1.0) ≈ cfg.y_natural` for every (module, curve) pair.
