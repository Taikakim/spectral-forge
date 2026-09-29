# Spectral Forge — GUI Modding Guide

This document explains how the graphical interface is structured and how to
customise it without breaking the audio engine.

---

## File map

```
src/editor/
  theme.rs              Visual constants (colours, stroke widths, node radius).
                        THE only file you need to touch for a full reskin.
  curve.rs              CurveNode type, compute_curve_response(), paint_response_curve(),
                        curve_widget() — the per-bin spectral EQ drawing and interaction.
  curve_config.rs       CurveDisplayConfig — the only place per-curve display ranges,
                        grid lines, and unit labels are defined.
  spectrum_display.rs   paint_spectrum_and_suppression() — pre/post-FX spectrum gradient.
  fx_matrix_grid.rs     The 9×9 slot routing matrix widget.
  module_popup.rs       Right-click module assignment popup.
  amp_popup.rs          Routing-matrix cell (send amplitude) popup.
  help_box.rs           Help-box widget rendered right of the routing matrix.
  preset_menu.rs        preset_menu_ui() — preset picker.
  mod_ring.rs           Modulation Ring overlay (S/H, Sync, Legato).
  *_popup.rs, *_panel.rs  Per-module mode popups and panels (circuit, harmony, kinetics,
                        life, past, rhythm, modulate, contrast). The circuit, contrast,
                        kinetics, life, and modulate panels compile only with the
                        dev-build feature.
  mod.rs                Module declarations and re-exports for the above.

src/editor_ui.rs        Top-level egui frame: assembles all widgets, reads params,
                        applies ui_scale, wires interaction to triple-buffer publish.
src/params.rs           SpectralForgeParams — the single source of truth for all state
                        shared between GUI and DSP.
src/dsp/
  pipeline.rs           Audio processing; reads params via triple-buffer, never locks.
  modules/mod.rs        ModuleSpec, module_spec(), apply_curve_transform() — curve
                        tilt/offset logic that must stay in sync with the GUI preview.
```

---

## The widget–engine contract

**Every param that exists in `SpectralForgeParams` drives the DSP.**
If you add or replace a widget but stop updating a param, the engine keeps running
with whatever value it had when the widget disappeared.  That may be intentional
("bypass") or a bug.  Be explicit:

### Case 1 — Replacing a widget with a better one

Read the same param and write it back. The signature does not change.

```rust
// Old
ui.add(egui::Slider::new(&mut *params.attack_ms.lock(), 1.0..=500.0));

// New custom knob
my_fancy_knob(ui, &params.attack_ms, setter);  // must call setter internally
```

### Case 2 — Hiding a param intentionally ("bypass")

Document it in a comment, set the param to its neutral value, and leave a note in
`GUI.md`:

```rust
// BYPASSED: mix param is always 100% in this skin — no wet/dry knob shown.
// Engine reads params.mix; neutral value is 1.0 (100%).
params.mix.lock();  // ensure lock is dropped immediately (no-op, just visibility)
```

If you want to make the bypass a build-time guarantee rather than a convention, add
a `#[cfg]` guard or a `const BYPASS_MIX: bool = true;` that the widget code checks.

### Case 3 — Params the engine reads that the GUI doesn't show at all

These already exist (e.g. legacy `curve_nodes`, `phase_curve_nodes`).  Their values
persist from the saved preset.  The engine uses them; the GUI just doesn't expose
controls for them.  This is fine as long as the values stay valid (they do because
they are never written to again after the initial `Default::default()`).

### Params that carry engine state

`slot_module_types`, `slot_curve_nodes`, `slot_curve_meta`, `route_matrix` — these
are read every audio block via `try_lock()`.  Any widget that writes them must
acquire the lock, modify, and release before the next audio block (at 60 fps GUI,
that's ~16 ms — the audio thread will always get the lock at its next hop).

---

## Reskinning (colours and geometry only)

Edit **`src/editor/theme.rs`** exclusively:

- **Per-curve colours** — generated from LCH at build time via `build_curve_colors()`.
  Change the L/C/H parameters in that function.
- **Named colours** — `BG`, `BORDER`, `LABEL_DIM`, `GRID_LINE`, etc. are `const Color32`.
- **Stroke widths** — `STROKE_THIN`, `STROKE_BORDER`, `STROKE_CURVE`.
- **Node radius** — `NODE_RADIUS`.

No other file defines visual constants.  If you find a hardcoded `Color32` or
pixel literal outside `theme.rs`, please move it there.

---

## Adding a new curve module

1. Add a variant to `ModuleType` in `src/dsp/modules/mod.rs`.
2. Add a `ModuleSpec` entry in `module_spec()` — set `display_name`, `num_curves`,
   `curve_labels`, `color_lit`, `color_dim`.
3. Add the variant to `ASSIGNABLE` in `src/editor/module_popup.rs`.
4. Implement `SpectralModule` for the new type, wire it into `create_module()`.
5. Optionally add `default_nodes_for_curve()` entries in `curve.rs` for non-flat
   initial shapes.

No GUI file outside `module_popup.rs` needs to change — the top bar and curve widget
adapt automatically from `ModuleSpec`.

---

## Adding a new global control

1. Add the param to `SpectralForgeParams` in `src/params.rs` (`#[persist = ...]`
   for GUI state, `FloatParam`/`BoolParam` etc. for audio params).
2. Add a default value in `Default::default()`.
3. Add the widget in `src/editor_ui.rs` — read the param, render the widget, write
   back on change.
4. If it's an audio param, decide whether the pipeline reads it directly (via the
   `params` arc shared into the audio closure) or via a smoothed value on the audio
   thread.

---

## HiDPI and scaling

The UI scale is controlled by `params.ui_scale` (persisted, default 1.0).  The
available steps are 1×, 1.25×, 1.5×, 1.75×, 2×, selectable from the FFT bar.

Internally, every frame calls:

```rust
ctx.set_pixels_per_point(scale);
ctx.send_viewport_cmd(ViewportCommand::InnerSize(vec2(900 * scale, 1010 * scale)));
```

`set_pixels_per_point` scales all egui content uniformly.
`ViewportCommand::InnerSize` asks the host to resize the plugin window to match.
Bitwig and most modern hosts honour this; if yours does not, content will clip at
the original 900×1010 physical pixels.

**If you add new widgets that expand the window height** (e.g. extra rows in the
matrix area), update the `MATRIX_AREA_H` constant and the window base size in both
`editor_ui.rs` (`strip_height` comment) and the `ViewportCommand` calculation.

All size literals in `editor_ui.rs` are in **logical pixels at 1× scale**.  At 2×,
egui doubles them automatically via `pixels_per_point`.  Do not scale pixel literals
manually.

---

## Replacing the entire UI

If you want a fully custom renderer (e.g. a hand-drawn skin using egui Painter
directly, or a different UI toolkit):

1. Keep `src/params.rs` unchanged — it is the contract between GUI and DSP.
2. Keep `src/dsp/` unchanged.
3. Replace `src/editor_ui.rs` and `src/editor/` with your implementation.
4. Your frame callback **must** do the following every frame, even if your UI does
   not display the corresponding control:

   | Action | Why |
   |--------|-----|
   | Read `params.editing_slot` and `editing_curve` | Pipeline reads these to decide which slot is being previewed |
   | Write to `params.slot_curve_nodes` when nodes change, then publish via the triple-buffer in `curve_tx` | Without publish, DSP sees stale curves |
   | Set `params.ui_scale` to your desired ppp | Window sizing |
   | Call `ctx.set_pixels_per_point(...)` | Avoids egui defaulting to system ppp |

5. For any audio param you don't expose: either set it to a safe neutral value in
   `Default::default()` (preferred), or accept that the persisted preset value will
   be used as-is.

### Triple-buffer publish (required after curve edits)

```rust
// After modifying params.slot_curve_nodes[slot][curve]:
use crate::dsp::pipeline::MAX_NUM_BINS;
let gains = crv::compute_curve_response(&nodes, MAX_NUM_BINS, sample_rate, fft_size);
if let Some(tx_arc) = curve_tx.get(slot).and_then(|row| row.get(curve)) {
    if let Some(mut tx) = tx_arc.try_lock() {
        tx.input_buffer_mut()[..gains.len()].copy_from_slice(&gains);
        tx.publish();
    }
}
```

Without this, the audio thread receives the old gains until the next time the GUI
publishes — which for a custom UI that never calls `curve_widget()` is never.

---

## What not to touch

- `src/dsp/` — real-time safety rules apply (no alloc, no lock, no I/O).
- `src/lib.rs` `initialize()` / `process()` — plugin lifecycle, not UI.
- `src/bridge.rs` — triple-buffer plumbing; the topology must match `pipeline.rs`.
- `CLAUDE.md` — AI assistant guide; update it if you add new subsystems.
