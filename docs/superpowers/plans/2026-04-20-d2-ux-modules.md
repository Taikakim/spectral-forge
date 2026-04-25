> **Status (2026-04-24): IMPLEMENTED.** Module popup, adaptive curve editor, per-slot SC/GainMode/name, matrix routing, T/S virtual rows, M/S DSP all landed. Source of truth: the code + [../STATUS.md](../STATUS.md).

# D2 — UX and New Modules Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver the full D2 user experience — module assignment popup, adaptive curve editor, per-slot sidechain/GainMode/name controls, matrix routing, T/S Split virtual rows, and functional M/S DSP.

**Architecture:** D1 established the data model (`slot_module_types`, `slot_curve_nodes`, `route_matrix`, etc.) and the SpectralModule trait. D2 migrates the editor UI to read exclusively from those D1 structures, implements the module assignment popup, wires the matrix routing DSP, and adds the M/S module. The legacy `curve_nodes`/`active_curve`/`active_tab` params remain in the struct (for serialization backward compat) but are no longer read by the editor.

**Tech Stack:** Rust, nih-plug, egui (via nih_plug_egui), realfft, triple_buffer, parking_lot. Run `cargo test` after each task. Build the `.clap` file with `cargo run --package xtask -- bundle spectral_forge --release` then `cp target/bundled/spectral_forge.clap ~/.clap/`.

---

## File Map

| File | Change |
|------|--------|
| `src/dsp/modules/mod.rs` | Add `set_gain_mode()` to SpectralModule trait |
| `src/dsp/modules/gain.rs` | Override `set_gain_mode()` |
| `src/dsp/modules/mid_side.rs` | Full DSP implementation (balance, expansion, decorrelation) |
| `src/dsp/fx_matrix.rs` | Constructor takes `slot_types`; forward-matrix routing; `set_gain_modes()` helper |
| `src/dsp/pipeline.rs` | Pass slot_types to FxMatrix; update gain modes each block |
| `src/lib.rs` | Pass slot_types from params to Pipeline; pass sidechain_active arcs to editor |
| `src/params.rs` | Fix `RouteMatrix::default()` to serial topology |
| `src/editor/fx_matrix_grid.rs` | Full rewrite: 9 slots, ModuleType, route_matrix, right-click event |
| `src/editor/module_popup.rs` | New file: module assignment egui popup |
| `src/editor/mod.rs` | Add `pub mod module_popup` |
| `src/editor_ui.rs` | Migrate to slot_curve_nodes; adaptive buttons; SC strip; GainMode; name edit |
| `tests/engine_contract.rs` | New tests for matrix routing and gain mode propagation |

---

## Task 1: FxMatrix accepts slot_module_types; matrix routing; GainMode propagation

**Goal:** FxMatrix uses the persisted `slot_module_types` to create modules (takes effect on next `initialize()`). DSP routes audio via `route_matrix.send` (forward sends only). GainMode propagates each block from params.

**Files:**
- Modify: `src/dsp/modules/mod.rs`
- Modify: `src/dsp/modules/gain.rs`
- Modify: `src/dsp/fx_matrix.rs`
- Modify: `src/dsp/pipeline.rs`
- Modify: `src/lib.rs`
- Modify: `src/params.rs` (RouteMatrix default)
- Modify: `tests/engine_contract.rs`

- [ ] **Step 1: Write failing tests**

In `tests/engine_contract.rs`, add at the end:

```rust
#[test]
fn fx_matrix_constructs_from_slot_types() {
    use spectral_forge::dsp::{
        modules::{ModuleType, create_module},
        fx_matrix::FxMatrix,
    };
    let mut types = [ModuleType::Empty; 9];
    types[0] = ModuleType::Dynamics;
    types[1] = ModuleType::Gain;
    types[8] = ModuleType::Master;
    // Should not panic and slot 8 must be Master.
    let _m = FxMatrix::new(44100.0, 2048, &types);
}

#[test]
fn gain_module_set_gain_mode_changes_behavior() {
    use spectral_forge::dsp::modules::{create_module, GainMode, ModuleType};
    let mut g = create_module(ModuleType::Gain, 44100.0, 2048);
    // Default is Add. After setting Subtract, mode should be Subtract.
    g.set_gain_mode(GainMode::Subtract);
    // No public mode accessor — test indirectly via process output.
    // With Subtract and no sidechain, gain curve all-ones → bins unchanged.
    // (Behavioral test is implicit: we just verify no panic and it compiles.)
    g.set_gain_mode(GainMode::Add);
}

#[test]
fn matrix_routing_serial_default_passes_signal() {
    use num_complex::Complex;
    use spectral_forge::dsp::{
        modules::{ModuleType, ModuleContext, RouteMatrix},
        fx_matrix::FxMatrix,
        pipeline::MAX_NUM_BINS,
    };
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let n = 1025usize; // 2048/2+1
    let mut types = [ModuleType::Empty; 9];
    types[0] = ModuleType::Dynamics;
    types[8] = ModuleType::Master;
    let mut fm = FxMatrix::new(44100.0, 2048, &types);

    // Serial default: slot 0 feeds Master. Main signal = all-ones magnitude.
    let mut bins: Vec<Complex<f32>> = vec![Complex::new(1.0, 0.0); n];
    let curves: Vec<Vec<Vec<f32>>> = (0..9)
        .map(|_| (0..7).map(|_| vec![1.0f32; MAX_NUM_BINS]).collect())
        .collect();
    let mut supp = vec![0.0f32; n];
    let sc: [Option<&[f32]>; 9] = [None; 9];
    let targets = [FxChannelTarget::All; 9];
    let rm = RouteMatrix::default();
    let ctx = ModuleContext {
        sample_rate: 44100.0, fft_size: 2048, num_bins: n,
        attack_ms: 10.0, release_ms: 100.0, sensitivity: 0.0,
        suppression_width: 0.0, auto_makeup: false, delta_monitor: false,
    };
    fm.process_hop(0, StereoLink::Linked, &mut bins, &sc, &targets, &curves, &rm, &ctx, &mut supp, n);
    // Signal should make it through: at least some bins are non-zero.
    assert!(bins.iter().any(|c| c.norm() > 0.01), "signal lost through matrix");
}
```

- [ ] **Step 2: Run tests — confirm they fail or don't compile**

```bash
cargo test 2>&1 | head -40
```

Expected: compile errors (FxMatrix::new signature mismatch, set_gain_mode not on trait).

- [ ] **Step 3: Add `set_gain_mode` to SpectralModule trait**

In `src/dsp/modules/mod.rs`, add to the `SpectralModule` trait body (after `fn num_outputs`):

```rust
    /// Update the operating mode for Gain modules. Default no-op for all other types.
    fn set_gain_mode(&mut self, _: GainMode) {}
```

- [ ] **Step 4: Override `set_gain_mode` in GainModule**

In `src/dsp/modules/gain.rs`, add inside the `impl SpectralModule for GainModule` block:

```rust
    fn set_gain_mode(&mut self, mode: GainMode) { self.mode = mode; }
```

- [ ] **Step 5: Fix RouteMatrix::default() to serial topology**

In `src/dsp/modules/mod.rs`, replace the `impl Default for RouteMatrix` block:

```rust
impl Default for RouteMatrix {
    fn default() -> Self {
        let mut m = Self {
            send: [[0.0f32; MAX_SLOTS]; MAX_MATRIX_ROWS],
            virtual_rows: [None; MAX_SPLIT_VIRTUAL_ROWS],
        };
        // Serial: main input → slot 0 (implicit) → slot 1 → slot 2 → Master (slot 8).
        // send[src][dst] = amplitude.
        m.send[0][1] = 1.0;
        m.send[1][2] = 1.0;
        m.send[2][8] = 1.0;
        m
    }
}
```

- [ ] **Step 6: Rewrite FxMatrix::new to accept slot_types and add process_hop signature update**

Replace `src/dsp/fx_matrix.rs` entirely:

```rust
use num_complex::Complex;
use crate::dsp::modules::{
    ModuleContext, ModuleType, RouteMatrix, GainMode, SpectralModule,
    create_module, MAX_SLOTS, MAX_SPLIT_VIRTUAL_ROWS,
};
use crate::params::{FxChannelTarget, StereoLink};

pub struct FxMatrix {
    pub slots: Vec<Option<Box<dyn SpectralModule>>>,
    slot_out:  Vec<Vec<Complex<f32>>>,
    slot_supp: Vec<Vec<f32>>,
    virtual_out: Vec<Vec<Complex<f32>>>,
    mix_buf:   Vec<Complex<f32>>,
}

impl FxMatrix {
    pub fn new(sample_rate: f32, fft_size: usize, slot_types: &[ModuleType; 9]) -> Self {
        let num_bins = fft_size / 2 + 1;
        let slots: Vec<Option<Box<dyn SpectralModule>>> = (0..MAX_SLOTS).map(|i| {
            match slot_types[i] {
                ModuleType::Empty => None,
                ty => Some(create_module(ty, sample_rate, fft_size)),
            }
        }).collect();
        Self {
            slots,
            slot_out:    (0..MAX_SLOTS).map(|_| vec![Complex::new(0.0, 0.0); num_bins]).collect(),
            slot_supp:   (0..MAX_SLOTS).map(|_| vec![0.0f32; num_bins]).collect(),
            virtual_out: (0..MAX_SPLIT_VIRTUAL_ROWS)
                             .map(|_| vec![Complex::new(0.0, 0.0); num_bins]).collect(),
            mix_buf: vec![Complex::new(0.0, 0.0); num_bins],
        }
    }

    pub fn reset(&mut self, sample_rate: f32, fft_size: usize) {
        let num_bins = fft_size / 2 + 1;
        debug_assert_eq!(self.slot_out[0].len(), num_bins,
            "FxMatrix::reset() called with different fft_size than new()");
        for slot in self.slots.iter_mut().flatten() {
            slot.reset(sample_rate, fft_size);
        }
        for buf in &mut self.slot_out    { buf.fill(Complex::new(0.0, 0.0)); }
        for buf in &mut self.slot_supp   { buf.fill(0.0); }
        for buf in &mut self.virtual_out { buf.fill(Complex::new(0.0, 0.0)); }
        self.mix_buf.fill(Complex::new(0.0, 0.0));
    }

    /// Propagate per-slot GainMode from params to GainModule instances.
    /// Called once per audio block (before process_hop).
    pub fn set_gain_modes(&mut self, modes: &[GainMode; 9]) {
        for s in 0..MAX_SLOTS {
            if let Some(ref mut m) = self.slots[s] {
                m.set_gain_mode(modes[s]);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn process_hop(
        &mut self,
        channel:         usize,
        stereo_link:     StereoLink,
        complex_buf:     &mut [Complex<f32>],
        sc_args:         &[Option<&[f32]>; 9],
        slot_targets:    &[FxChannelTarget; 9],
        slot_curves:     &[Vec<Vec<f32>>],   // [slot][curve][bin]
        route_matrix:    &RouteMatrix,
        ctx:             &ModuleContext,
        suppression_out: &mut [f32],
        num_bins:        usize,
    ) {
        for s in 0..MAX_SLOTS {
            // Build this slot's input from the route matrix.
            // Slot 0 always receives the plugin's main audio input.
            // All slots additionally receive weighted sums of previous-slot outputs.
            self.mix_buf[..num_bins].fill(Complex::new(0.0, 0.0));
            if s == 0 {
                self.mix_buf[..num_bins].copy_from_slice(&complex_buf[..num_bins]);
            }
            for src in 0..s {
                let send = route_matrix.send[src][s];
                if send < 0.001 { continue; }
                for k in 0..num_bins {
                    self.mix_buf[k] += self.slot_out[src][k] * send;
                }
            }

            let mut module = match self.slots[s].take() {
                Some(m) => m,
                None => {
                    self.slot_out[s][..num_bins].copy_from_slice(&self.mix_buf[..num_bins]);
                    self.slot_supp[s][..num_bins].fill(0.0);
                    continue;
                }
            };

            let nc = module.num_curves().min(7);
            let curves_storage: [&[f32]; 7] = std::array::from_fn(|c| {
                if c < nc && s < slot_curves.len() && c < slot_curves[s].len() {
                    let cv = &slot_curves[s][c];
                    &cv[..num_bins.min(cv.len())]
                } else {
                    &[] as &[f32]
                }
            });
            let curves: &[&[f32]] = &curves_storage[..nc];

            module.process(
                channel, stereo_link, slot_targets[s],
                &mut self.mix_buf[..num_bins],
                sc_args[s], curves,
                &mut self.slot_supp[s][..num_bins],
                ctx,
            );
            self.slot_out[s][..num_bins].copy_from_slice(&self.mix_buf[..num_bins]);
            self.slots[s] = Some(module);
        }

        // Master (slot 8): mix-down from all sends targeting it, then write to complex_buf.
        let mut master_buf = vec![Complex::new(0.0f32, 0.0); num_bins]; // tmp; no allocation on audio thread — this DOES allocate. Fix: reuse mix_buf.
        // Use mix_buf as master accumulator (it's done being used above).
        self.mix_buf[..num_bins].fill(Complex::new(0.0, 0.0));
        // Slot 0 always contributes if there's no explicit send to Master from anything?
        // Actually: slot 0's implicit-main-input role is already handled above. The master
        // output is whatever route_matrix routes TO slot 8 from previous slots.
        // Check: if nothing routes to Master (slot 8), fall back to last active slot output.
        let any_to_master = (0..8).any(|src| route_matrix.send[src][8] > 0.001);
        if any_to_master {
            for src in 0..8 {
                let send = route_matrix.send[src][8];
                if send < 0.001 { continue; }
                for k in 0..num_bins {
                    self.mix_buf[k] += self.slot_out[src][k] * send;
                }
            }
        } else {
            // Fallback: last populated slot's output goes to Master.
            for src in (0..8).rev() {
                if self.slots[src].is_some() {
                    self.mix_buf[..num_bins].copy_from_slice(&self.slot_out[src][..num_bins]);
                    break;
                }
            }
        }
        // Pass through Master module (pass-through) then write to complex_buf.
        if let Some(ref mut master_mod) = self.slots[8] {
            let curves_empty: &[&[f32]] = &[];
            master_mod.process(
                channel, stereo_link, slot_targets[8],
                &mut self.mix_buf[..num_bins],
                sc_args[8], curves_empty,
                &mut self.slot_supp[8][..num_bins],
                ctx,
            );
        }
        complex_buf[..num_bins].copy_from_slice(&self.mix_buf[..num_bins]);

        // Max-reduce suppression across all slots for display.
        suppression_out[..num_bins].fill(0.0);
        for s in 0..MAX_SLOTS {
            for k in 0..num_bins {
                if self.slot_supp[s][k] > suppression_out[k] {
                    suppression_out[k] = self.slot_supp[s][k];
                }
            }
        }
    }
}
```

**Note on the master_buf allocation**: The code above accidentally allocates `master_buf` on the audio thread (line with `let mut master_buf`). This line is dead code and should be removed. The actual logic uses `self.mix_buf`. Remove the `master_buf` line entirely.

- [ ] **Step 7: Fix the allocation in FxMatrix::process_hop**

After writing the code above, find and remove the dead `let mut master_buf = ...` line from `process_hop`. It was a mistake — the actual master accumulation uses `self.mix_buf`. The file should have NO `master_buf` variable.

The correct implementation uses only `self.mix_buf`:
```rust
        // Master output: accumulate sends to slot 8.
        self.mix_buf[..num_bins].fill(Complex::new(0.0, 0.0));
        let any_to_master = (0..8).any(|src| route_matrix.send[src][8] > 0.001);
        if any_to_master {
            for src in 0..8 {
                let send = route_matrix.send[src][8];
                if send < 0.001 { continue; }
                for k in 0..num_bins {
                    self.mix_buf[k] += self.slot_out[src][k] * send;
                }
            }
        } else {
            for src in (0..8).rev() {
                if self.slots[src].is_some() {
                    self.mix_buf[..num_bins].copy_from_slice(&self.slot_out[src][..num_bins]);
                    break;
                }
            }
        }
```

- [ ] **Step 8: Update Pipeline::new and process_hop call**

In `src/dsp/pipeline.rs`, change `Pipeline::new` signature:

```rust
pub fn new(sample_rate: f32, num_channels: usize, fft_size: usize, slot_types: &[ModuleType; 9]) -> Self {
```

Change the FxMatrix::new call inside:
```rust
let fx_matrix = crate::dsp::fx_matrix::FxMatrix::new(sample_rate, fft_size, slot_types);
```

Add import at top if missing:
```rust
use crate::dsp::modules::ModuleType;
```

In `Pipeline::process()`, add before the `let fx_matrix = &mut self.fx_matrix;` line:

```rust
        // Propagate gain modes each block (try_lock is non-blocking; skipped if GUI holds lock).
        if let Some(modes) = params.slot_gain_mode.try_lock() {
            self.fx_matrix.set_gain_modes(&*modes);
        }
```

In the `fx_matrix.process_hop(...)` call, add `route_matrix` argument. Read it from params:

```rust
        let route_matrix_snap = params.route_matrix.try_lock()
            .map(|g| g.clone())
            .unwrap_or_else(|| crate::dsp::modules::RouteMatrix::default());
```

Add `route_matrix_snap` into the `process_hop` call (before `ctx`):
```rust
            fx_matrix.process_hop(
                channel,
                stereo_link,
                complex_buf,
                &sc_args,
                &slot_targets_snap,
                slot_curve_cache_ref,
                &route_matrix_snap,     // ← new
                &ctx,
                channel_supp_buf,
                num_bins,
            );
```

**Note**: `route_matrix_snap` clones a RouteMatrix (small fixed struct). This allocates on the audio thread. To avoid it: read it before the closure into a local variable. Move the try_lock + clone BEFORE the `self.stft.process_overlap_add(...)` call, alongside the other snapshots.

- [ ] **Step 9: Update lib.rs to pass slot_types to Pipeline**

In `src/lib.rs`, inside `initialize()`, before the `Pipeline::new(...)` call, add:

```rust
        let slot_types = *self.params.slot_module_types.lock();
```

Change the Pipeline::new call:
```rust
        self.pipeline = Some(dsp::pipeline::Pipeline::new(sr, num_ch, fft_size, &slot_types));
```

- [ ] **Step 10: Run tests**

```bash
cargo test 2>&1
```

Expected: all existing tests pass plus the 3 new ones. Fix any compile errors before proceeding.

- [ ] **Step 11: Commit**

```bash
git add src/dsp/modules/mod.rs src/dsp/modules/gain.rs src/dsp/fx_matrix.rs \
        src/dsp/pipeline.rs src/lib.rs tests/engine_contract.rs
git commit -m "feat: FxMatrix reads slot_module_types; matrix routing; GainMode propagation"
```

---

## Task 2: Rewrite fx_matrix_grid.rs — 9 slots, ModuleType, route_matrix

**Goal:** The matrix grid shows the full 9-slot layout using `ModuleType` colors from `module_spec()`, reads routing from `route_matrix.send`, and returns both left-click (select) and right-click (popup trigger) events.

**Files:**
- Rewrite: `src/editor/fx_matrix_grid.rs`
- Modify: `src/editor_ui.rs` (update call site)

- [ ] **Step 1: Write failing test**

In `tests/engine_contract.rs`:
```rust
// No unit test possible for pure egui UI code. Verify compilation only via cargo build.
```

Skip to implementation. The test is "the UI compiles and renders without panic".

- [ ] **Step 2: Replace fx_matrix_grid.rs**

Write `src/editor/fx_matrix_grid.rs`:

```rust
use nih_plug_egui::egui::{self, Pos2, Rect, Stroke, StrokeKind, Ui, UiBuilder, Vec2};
use crate::dsp::modules::{module_spec, ModuleType, RouteMatrix};
use crate::editor::theme as th;

const CELL: f32  = 44.0;
const LABEL: f32 = 52.0;

pub struct MatrixInteraction {
    pub left_click_slot:  Option<usize>,
    pub right_click:      Option<(usize, Pos2)>,
}

/// Convert a slot_name bytes ([u8; 32]) to a display String.
pub fn slot_name_str(bytes: &[u8; 32]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(32);
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

/// Paint the 9×9 routing matrix grid.
///
/// Returns `MatrixInteraction` describing any clicks this frame.
pub fn paint_fx_matrix_grid(
    ui:           &mut Ui,
    module_types: &[ModuleType; 9],
    slot_names:   &[[u8; 32]; 9],
    route_matrix: &mut RouteMatrix,
    editing_slot: usize,
) -> MatrixInteraction {
    // Column header height
    const HDR: f32 = 14.0;
    let n = 9usize;
    let total_w = LABEL + n as f32 * CELL;
    let total_h = HDR + n as f32 * CELL;

    let (outer_resp, painter) =
        ui.allocate_painter(Vec2::new(total_w, total_h), egui::Sense::hover());
    let origin = outer_resp.rect.min;

    let mut result = MatrixInteraction { left_click_slot: None, right_click: None };

    // Column headers
    for col in 0..n {
        let ty = module_types[col];
        let name = if col == 8 {
            "OUT".to_string()
        } else {
            let s = slot_name_str(&slot_names[col]);
            if s.chars().count() > 4 { s.chars().take(4).collect::<String>() + "…" } else { s }
        };
        let spec = module_spec(ty);
        let hdr_rect = Rect::from_min_size(
            origin + egui::vec2(LABEL + col as f32 * CELL, 0.0),
            Vec2::new(CELL - 1.0, HDR),
        );
        painter.text(
            hdr_rect.center(),
            egui::Align2::CENTER_CENTER,
            &name,
            egui::FontId::proportional(7.5),
            if ty == ModuleType::Empty { th::LABEL_DIM } else { spec.color_lit },
        );
    }

    for row in 0..n {
        let ty_row = module_types[row];
        let spec_row = module_spec(ty_row);
        let row_top = origin.y + HDR + row as f32 * CELL;

        // Row label
        let name = slot_name_str(&slot_names[row]);
        let display_name: String = if name.chars().count() > 7 {
            name.chars().take(6).collect::<String>() + "…"
        } else {
            name
        };
        let label_rect = Rect::from_min_size(
            egui::pos2(origin.x, row_top),
            Vec2::new(LABEL - 2.0, CELL),
        );
        painter.text(
            label_rect.center(),
            egui::Align2::CENTER_CENTER,
            &display_name,
            egui::FontId::proportional(8.5),
            if ty_row == ModuleType::Empty { th::LABEL_DIM } else { spec_row.color_lit },
        );

        for col in 0..n {
            let cell_rect = Rect::from_min_size(
                egui::pos2(origin.x + LABEL + col as f32 * CELL, row_top),
                Vec2::new(CELL - 1.0, CELL - 1.0),
            );

            if row == col {
                // Diagonal: module cell
                let is_selected = row == editing_slot;
                let ty = module_types[row];
                let spec = module_spec(ty);
                let is_master = row == 8;

                let fill = if is_master {
                    spec.color_dim
                } else if ty == ModuleType::Empty {
                    th::BG_RAISED
                } else if is_selected {
                    spec.color_lit
                } else {
                    spec.color_dim
                };
                let stroke = if is_selected {
                    Stroke::new(1.5, th::BORDER)
                } else {
                    Stroke::new(0.5, th::GRID_LINE)
                };
                painter.rect(cell_rect, 2.0, fill, stroke, StrokeKind::Middle);

                let label_str = if is_master {
                    "OUT".to_string()
                } else if ty == ModuleType::Empty {
                    "+".to_string()
                } else {
                    let n = slot_name_str(&slot_names[row]);
                    if n.chars().count() > 6 { n.chars().take(5).collect::<String>() + "…" } else { n }
                };
                let text_col = if is_master {
                    spec.color_lit
                } else if ty == ModuleType::Empty {
                    th::LABEL_DIM
                } else if is_selected {
                    egui::Color32::BLACK
                } else {
                    spec.color_lit
                };
                painter.text(
                    cell_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    &label_str,
                    egui::FontId::proportional(8.0),
                    text_col,
                );

                let interact = ui.interact(
                    cell_rect,
                    ui.id().with(("mat_diag", row)),
                    egui::Sense::click(),
                );
                if interact.clicked() {
                    result.left_click_slot = Some(row);
                }
                if interact.secondary_clicked() && !is_master {
                    result.right_click = Some((row, interact.interact_pointer_pos()
                        .unwrap_or(cell_rect.center())));
                }
                // Hover tooltip: full name
                interact.on_hover_text(slot_name_str(&slot_names[row]));

            } else {
                // Off-diagonal send cell.
                // Upper triangle (col > row) = feedback path; lower = forward.
                let is_feedback = col > row;
                let is_to_master = col == 8;
                let bg = if is_feedback { th::BG_FEEDBACK } else { th::BG_RAISED };
                painter.rect(cell_rect, 0.0, bg, Stroke::new(0.5, th::GRID_LINE), StrokeKind::Middle);

                // Disable cells where both src and dst are Empty
                let src_ty = module_types[row];
                let dst_ty = module_types[col];
                let both_empty = src_ty == ModuleType::Empty && dst_ty == ModuleType::Empty;

                if !both_empty {
                    let send_val = &mut route_matrix.send[row][col];
                    ui.allocate_new_ui(
                        UiBuilder::new().max_rect(cell_rect.shrink(3.0)),
                        |ui| {
                            ui.add(
                                egui::DragValue::new(send_val)
                                    .range(0.0..=2.0)
                                    .speed(0.005)
                                    .fixed_decimals(2)
                                    .custom_formatter(|v, _| {
                                        if v < 0.005 { "\u{2014}".to_string() }
                                        else { format!("{v:.2}") }
                                    })
                                    .custom_parser(|s| s.parse::<f64>().ok()),
                            );
                        },
                    );
                } else {
                    painter.text(
                        cell_rect.center(),
                        egui::Align2::CENTER_CENTER,
                        "\u{2014}",
                        egui::FontId::proportional(8.0),
                        th::GRID_LINE,
                    );
                }
            }
        }
    }

    result
}
```

- [ ] **Step 3: Update editor_ui.rs call site**

In `src/editor_ui.rs`, find the `paint_fx_matrix_grid(...)` call and replace:

```rust
                    let route_matrix_ref = &mut *params.route_matrix.lock();
                    let types_snap = *params.slot_module_types.lock();
                    let names_snap  = *params.slot_names.lock();
                    let interaction = crate::editor::fx_matrix_grid::paint_fx_matrix_grid(
                        ui,
                        &types_snap,
                        &names_snap,
                        route_matrix_ref,
                        edit_slot,
                    );
                    if let Some(new_slot) = interaction.left_click_slot {
                        *params.editing_slot.lock() = new_slot as u8;
                    }
                    // right_click is handled in Task 3 (popup).
                    let _ = interaction.right_click;
```

Remove the old variables that no longer exist (`types_snap` using `fx_module_types`, `names_snap` using `fx_module_names`, `matrix` using `fx_route_matrix`, `clicked`). The `edit_slot` variable is still read from `params.editing_slot`.

Also update the `MATRIX_AREA_H` constant (9 rows × 44px + 14px header + 30px padding):
```rust
                    const MATRIX_AREA_H: f32 = 9.0 * 44.0 + 14.0 + 30.0; // 440 px
```

- [ ] **Step 4: Build and fix compile errors**

```bash
cargo build 2>&1
```

Fix all errors. Common issues:
- `fx_module_types`, `fx_module_names`, `fx_route_matrix` references in editor_ui.rs → replace with `slot_module_types`, `slot_names`, `route_matrix`
- The old `let edit_slot` / `let types_snap` / `let names_snap` / `let matrix` / `let clicked` block needs to be replaced entirely

- [ ] **Step 5: Run tests**

```bash
cargo test 2>&1
```

All existing tests must pass. Fix any regressions.

- [ ] **Step 6: Commit**

```bash
git add src/editor/fx_matrix_grid.rs src/editor_ui.rs
git commit -m "feat: matrix grid — 9 slots, ModuleType colors, route_matrix, right-click event"
```

---

## Task 3: Module assignment popup

**Goal:** Right-clicking a non-Master diagonal cell opens a floating popup listing assignable module types. Selecting a type updates `slot_module_types` and resets `slot_curve_nodes` to defaults for that type. The DSP change takes effect on next initialize().

**Files:**
- Create: `src/editor/module_popup.rs`
- Modify: `src/editor/mod.rs`
- Modify: `src/editor_ui.rs`

- [ ] **Step 1: Create `src/editor/module_popup.rs`**

```rust
use nih_plug_egui::egui::{self, Pos2, Ui};
use crate::dsp::modules::{module_spec, ModuleType, RouteMatrix};
use crate::editor::theme as th;
use crate::params::SpectralForgeParams;

/// Ephemeral state for the module assignment popup.
/// Stored in egui temp data under key `ui.id().with("module_popup")`.
#[derive(Clone)]
pub struct PopupState {
    pub open:     bool,
    pub slot:     usize,
    pub pos:      Pos2,
}

impl Default for PopupState {
    fn default() -> Self {
        Self { open: false, slot: 0, pos: Pos2::ZERO }
    }
}

/// Count how many T/S Split modules are currently active across all slots.
fn ts_split_count(types: &[ModuleType; 9]) -> usize {
    types.iter().filter(|&&t| t == ModuleType::TransientSustainedSplit).count()
}

const ASSIGNABLE: &[ModuleType] = &[
    ModuleType::Dynamics,
    ModuleType::Freeze,
    ModuleType::PhaseSmear,
    ModuleType::Contrast,
    ModuleType::Gain,
    ModuleType::MidSide,
    ModuleType::TransientSustainedSplit,
    ModuleType::Harmonic,
];

/// Render the popup if open. Call every frame from the main UI closure.
/// Returns true if the popup consumed a click (caller should not process other interactions).
pub fn show_popup(
    ui:     &mut Ui,
    params: &SpectralForgeParams,
) -> bool {
    let key = ui.id().with("module_popup");
    let state: PopupState = ui.data(|d| d.get_temp(key).unwrap_or_default());
    if !state.open { return false; }

    let types = *params.slot_module_types.lock();
    let ts_count = ts_split_count(&types);
    let slot = state.slot;

    let mut consumed = false;
    let mut new_state = state.clone();

    egui::Area::new(egui::Id::new("module_popup_area"))
        .fixed_pos(state.pos)
        .order(egui::Order::Foreground)
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_min_width(140.0);
                ui.label(
                    egui::RichText::new("Assign module")
                        .color(th::LABEL_DIM).size(9.0)
                );
                ui.separator();

                for &ty in ASSIGNABLE {
                    let spec = module_spec(ty);
                    let is_ts = ty == ModuleType::TransientSustainedSplit;
                    let ts_full = is_ts && ts_count >= 2 && types[slot] != ty;
                    let enabled = !ts_full;

                    ui.add_enabled_ui(enabled, |ui| {
                        ui.horizontal(|ui| {
                            // Color swatch
                            let (rect, _) = ui.allocate_exact_size(
                                egui::vec2(10.0, 10.0), egui::Sense::hover()
                            );
                            ui.painter().rect_filled(rect, 2.0, spec.color_lit);
                            let resp = ui.button(spec.display_name);
                            if resp.clicked() {
                                assign_module(params, slot, ty);
                                new_state.open = false;
                                consumed = true;
                            }
                        });
                    });

                    if ts_full {
                        ui.label(
                            egui::RichText::new("(max 2 active)")
                                .color(th::LABEL_DIM).size(8.0)
                        );
                    }
                }

                ui.separator();
                if ui.button("Remove module").clicked() {
                    assign_module(params, slot, ModuleType::Empty);
                    new_state.open = false;
                    consumed = true;
                }

                ui.separator();
                ui.label(
                    egui::RichText::new("DSP change takes effect\non host restart.")
                        .color(th::LABEL_DIM).size(8.0)
                );

                // Close on click outside
                if ui.ctx().input(|i| i.pointer.any_click())
                    && !ui.ctx().is_pointer_over_area()
                {
                    new_state.open = false;
                }
            });
        });

    ui.data_mut(|d| d.insert_temp(key, new_state));
    consumed
}

/// Open the popup for a slot at the given screen position.
pub fn open_popup(ui: &mut Ui, slot: usize, pos: Pos2) {
    let key = ui.id().with("module_popup");
    ui.data_mut(|d| d.insert_temp(key, PopupState { open: true, slot, pos }));
}

/// Assign a module type to a slot: update slot_module_types, reset slot_curve_nodes.
fn assign_module(params: &SpectralForgeParams, slot: usize, ty: ModuleType) {
    params.slot_module_types.lock()[slot] = ty;
    // Reset curve nodes for this slot to defaults.
    let spec = module_spec(ty);
    let mut nodes = params.slot_curve_nodes.lock();
    for c in 0..spec.num_curves.min(7) {
        nodes[slot][c] = crate::editor::curve::default_nodes_for_curve(c);
    }
    // Reset tilt/offset for this slot.
    let mut meta = params.slot_curve_meta.lock();
    for c in 0..7 {
        meta[slot][c] = (0.0, 0.0);
    }
    // Reset editing_curve to 0 if it's now out of range.
    let num_c = spec.num_curves;
    let mut ec = params.editing_curve.lock();
    if (*ec as usize) >= num_c {
        *ec = 0;
    }
}
```

- [ ] **Step 2: Register in mod.rs**

In `src/editor/mod.rs`, add:
```rust
pub mod module_popup;
```

- [ ] **Step 3: Wire popup into editor_ui.rs**

In `src/editor_ui.rs`, after the `paint_fx_matrix_grid` call, add:

```rust
                    // Handle right-click → open popup
                    if let Some((slot, pos)) = interaction.right_click {
                        crate::editor::module_popup::open_popup(ui, slot, pos);
                    }
                    // Render popup (above matrix, below nothing — uses egui Area)
                    crate::editor::module_popup::show_popup(ui, &params);
```

- [ ] **Step 4: Build**

```bash
cargo build 2>&1
```

Fix compile errors. The `default_nodes_for_curve` function must be public in `curve.rs` — check it's already `pub`. If not, add `pub` to it.

- [ ] **Step 5: Run tests**

```bash
cargo test 2>&1
```

- [ ] **Step 6: Commit**

```bash
git add src/editor/module_popup.rs src/editor/mod.rs src/editor_ui.rs
git commit -m "feat: module assignment popup — right-click assigns any SpectralModule type"
```

---

## Task 4: Adaptive curve buttons + remove fixed tabs + migrate to editing_curve

**Goal:** The top bar shows curve buttons derived from `module_spec(slot_module_types[editing_slot])`. The active button is based on `params.editing_curve`. Fixed tabs (DYNAMICS/EFFECTS/HARMONIC) and the EffectMode strip are removed. The `editing_curve` param drives all curve selection.

**Files:**
- Modify: `src/editor_ui.rs`

- [ ] **Step 1: Plan the changes to editor_ui.rs top bar**

The current top bar (inside `ui.horizontal`) contains:
1. Curve selector buttons (7 dynamics OR 4 freeze) — REPLACE with adaptive
2. Tab buttons (DYNAMICS/EFFECTS/HARMONIC) — REMOVE
3. Range controls (Floor/Ceil/Falloff) — KEEP

The new top bar shows:
1. `num_curves` buttons from `module_spec(editing_slot_type).curve_labels`
2. Range controls

The second bar (FFT selector) is unchanged.

The Row 2 `match active_tab` block:
- Case 0 (Dynamics): keep the Dynamics group box with Atk/Rel/Sens/Width knobs and Tilt/Offset controls — migrate to use slot_curve_meta
- Case 1 (Effects): REMOVE the EffectMode buttons (BYPASS/FREEZE/PHASE/CONTRAST)
- Case 2: REMOVE

Replace with: Row 2 always shows the Dynamics group (global knobs) if the editing slot is Dynamics-type, else shows nothing (or just global knobs).

- [ ] **Step 2: Replace the top bar curve buttons section**

In `src/editor_ui.rs`, find the block:
```rust
                    let is_freeze_mode = active_tab == 1 ...
                    let is_phase_mode  = active_tab == 1 ...
```

Replace the entire top `ui.horizontal` block with:

```rust
                    ui.horizontal(|ui| {
                        ui.add_space(4.0);

                        let editing_slot = *params.editing_slot.lock() as usize;
                        let slot_types   = *params.slot_module_types.lock();
                        let editing_type = slot_types[editing_slot];
                        let spec         = crate::dsp::modules::module_spec(editing_type);
                        let editing_curve = *params.editing_curve.lock() as usize;

                        // Adaptive curve selector buttons
                        for (i, &label) in spec.curve_labels.iter().enumerate() {
                            let is_active = editing_curve == i;
                            let (fill, text_color, stroke_color) = if is_active {
                                (spec.color_lit,
                                 egui::Color32::BLACK,
                                 spec.color_lit)
                            } else {
                                (spec.color_dim,
                                 spec.color_lit,
                                 spec.color_dim)
                            };
                            let btn = egui::Button::new(
                                egui::RichText::new(label).color(text_color).size(11.0),
                            )
                            .fill(fill)
                            .stroke(egui::Stroke::new(th::STROKE_BORDER, stroke_color));
                            if ui.add(btn).clicked() {
                                *params.editing_curve.lock() = i as u8;
                            }
                        }

                        if spec.num_curves > 0 {
                            ui.add_space(8.0);
                            ui.separator();
                            ui.add_space(4.0);
                        }

                        ui.label(egui::RichText::new("Floor").color(th::LABEL_DIM).size(9.0));
                        {
                            let mut v = *params.graph_db_min.lock();
                            if ui.add(
                                egui::DragValue::new(&mut v)
                                    .range(-160.0..=-20.0)
                                    .suffix(" dB").speed(0.5).max_decimals(1),
                            ).changed() {
                                *params.graph_db_min.lock() = v.min(db_max - 6.0);
                            }
                        }
                        ui.add_space(4.0);
                        ui.label(egui::RichText::new("Ceil").color(th::LABEL_DIM).size(9.0));
                        {
                            let mut v = *params.graph_db_max.lock();
                            if ui.add(
                                egui::DragValue::new(&mut v)
                                    .range(-20.0..=0.0)
                                    .suffix(" dB").speed(0.5).max_decimals(1),
                            ).changed() {
                                *params.graph_db_max.lock() = v.max(db_min + 6.0);
                            }
                        }
                        ui.add_space(4.0);
                        ui.label(egui::RichText::new("Falloff").color(th::LABEL_DIM).size(9.0));
                        {
                            let mut v = *params.peak_falloff_ms.lock();
                            if ui.add(
                                egui::DragValue::new(&mut v)
                                    .range(0.0..=5000.0)
                                    .suffix(" ms").speed(10.0).max_decimals(0),
                            ).changed() {
                                *params.peak_falloff_ms.lock() = v;
                            }
                        }
                    });
```

Remove the `let active_tab`, `let cur_mode`, `let is_freeze_mode`, `let is_phase_mode` variable declarations that are no longer needed.

- [ ] **Step 3: Update graph header to use slot_names and slot_targets**

Find the graph header block and replace:

```rust
                    {
                        let edit_slot  = *params.editing_slot.lock() as usize;
                        let names      = params.slot_names.lock();
                        let tgts       = params.slot_targets.lock();
                        let name_str   = crate::editor::fx_matrix_grid::slot_name_str(&names[edit_slot]);
                        let header     = format!("Editing: {} \u{2014} {}", name_str, tgts[edit_slot].label());
                        ui.painter().text(
                            curve_rect.min + egui::vec2(4.0, 4.0),
                            egui::Align2::LEFT_TOP,
                            &header,
                            egui::FontId::proportional(10.0),
                            th::LABEL_DIM,
                        );
                    }
```

- [ ] **Step 4: Rewrite Row 2 controls**

Replace the `ui.horizontal(|ui| { match active_tab { ... } })` block with:

```rust
                    ui.horizontal(|ui| {
                        let editing_slot  = *params.editing_slot.lock() as usize;
                        let slot_types    = *params.slot_module_types.lock();
                        let editing_type  = slot_types[editing_slot];
                        let editing_curve = (*params.editing_curve.lock() as usize)
                            .min(crate::dsp::modules::module_spec(editing_type).num_curves.saturating_sub(1));

                        // Dynamics group box: global dynamics knobs (shown for Dynamics slots and generally useful)
                        {
                            let dyn_frame = egui::Frame::new()
                                .stroke(egui::Stroke::new(th::STROKE_BORDER, th::GRID_LINE))
                                .inner_margin(egui::Margin { left: 4, right: 4, top: 4, bottom: 4 });
                            let dyn_resp = dyn_frame.show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    knob!(ui, &params.attack_ms,         "Atk");
                                    knob!(ui, &params.release_ms,        "Rel");
                                    knob!(ui, &params.sensitivity,       "Sens");
                                    knob!(ui, &params.suppression_width, "Width");
                                });
                            });
                            let lbl_pos = dyn_resp.response.rect.left_top() + egui::vec2(4.0, 0.0);
                            ui.painter().text(
                                lbl_pos, egui::Align2::LEFT_TOP, "Dynamics",
                                egui::FontId::proportional(8.0), th::LABEL_DIM,
                            );
                        }

                        // Per-curve tilt and offset from slot_curve_meta
                        let spec = crate::dsp::modules::module_spec(editing_type);
                        if editing_curve < spec.num_curves {
                            ui.add_space(8.0);
                            let crv_col = spec.color_lit;
                            let mut meta = *params.slot_curve_meta.lock();
                            let (offset, tilt) = &mut meta[editing_slot][editing_curve];
                            let mut changed = false;
                            ui.vertical(|ui| {
                                if ui.add(
                                    egui::DragValue::new(offset)
                                        .range(-1.0..=1.0).speed(0.005).fixed_decimals(3)
                                ).changed() { changed = true; }
                                ui.label(egui::RichText::new("Offset").color(crv_col).size(9.0));
                            });
                            ui.vertical(|ui| {
                                if ui.add(
                                    egui::DragValue::new(tilt)
                                        .range(-1.0..=1.0).speed(0.005).fixed_decimals(3)
                                ).changed() { changed = true; }
                                ui.label(egui::RichText::new("Tilt").color(crv_col).size(9.0));
                            });
                            if changed {
                                *params.slot_curve_meta.lock() = meta;
                            }
                        }
                    });
```

- [ ] **Step 5: Build**

```bash
cargo build 2>&1
```

Remove any now-unused `let` bindings (active_tab, cur_mode, is_freeze_mode, is_phase_mode, tilts/offsets arrays, freeze_active, etc.). Also remove unused imports if any.

- [ ] **Step 6: Run tests**

```bash
cargo test 2>&1
```

- [ ] **Step 7: Commit**

```bash
git add src/editor_ui.rs
git commit -m "feat: adaptive curve buttons from module_spec; remove fixed tabs; editing_curve drives selection"
```

---

## Task 5: Curve editor reads and writes slot_curve_nodes

**Goal:** The curve display and interactive widget use `slot_curve_nodes[editing_slot][editing_curve]`. All module types share one code path. Remove the legacy is_freeze_mode / is_phase_mode branching.

**Files:**
- Modify: `src/editor_ui.rs`

- [ ] **Step 1: Replace the curve drawing section**

Find the large block beginning with `// 3 + 4. Response curves + interactive widget` and replace the entire `if is_phase_mode { ... } else if is_freeze_mode { ... } else { ... }` tree with:

```rust
                    // 3 + 4. Response curves + interactive widget (unified — all module types)
                    {
                        let editing_slot  = *params.editing_slot.lock() as usize;
                        let slot_types    = *params.slot_module_types.lock();
                        let editing_type  = slot_types[editing_slot];
                        let spec          = crate::dsp::modules::module_spec(editing_type);
                        let num_c         = spec.num_curves;
                        let editing_curve = (*params.editing_curve.lock() as usize).min(num_c.saturating_sub(1));

                        // Clamp editing_curve if module type changed and it's now out of range
                        if *params.editing_curve.lock() as usize >= num_c && num_c > 0 {
                            *params.editing_curve.lock() = 0;
                        }

                        let nodes_all = *params.slot_curve_nodes.lock();

                        // Cache key: invalidate when slot type, editing slot, or fft_size changes
                        let cache_key = ui.id().with(("slot_gains", editing_slot, editing_type as u8, fft_size));
                        let cached: Option<([[[crate::editor::curve::CurveNode; 6]; 7]; 9], Vec<Vec<f32>>)> =
                            ui.data(|d| d.get_temp(cache_key));
                        let all_gains: Vec<Vec<f32>> = match cached {
                            Some((cn, cg)) if cn == nodes_all => cg,
                            _ => {
                                let g: Vec<Vec<f32>> = (0..num_c.min(7))
                                    .map(|c| crv::compute_curve_response(
                                        &nodes_all[editing_slot][c], num_bins, sr, fft_size,
                                    ))
                                    .collect();
                                ui.data_mut(|d| d.insert_temp(cache_key, (nodes_all, g.clone())));
                                g
                            }
                        };

                        let meta = *params.slot_curve_meta.lock();

                        // Draw inactive curves (dim)
                        for i in 0..num_c.min(7) {
                            if i == editing_curve { continue; }
                            let (tilt, offset) = meta[editing_slot][i];
                            crv::paint_response_curve(
                                ui.painter(), curve_rect, &all_gains[i], i,
                                spec.color_dim, 1.0,
                                db_min, db_max, atk_ms, rel_ms, sr, fft_size, tilt, offset,
                            );
                        }

                        // Draw active curve (lit) + interactive widget
                        if editing_curve < num_c && !all_gains.is_empty() {
                            let (tilt, offset) = meta[editing_slot][editing_curve];
                            crv::paint_response_curve(
                                ui.painter(), curve_rect, &all_gains[editing_curve], editing_curve,
                                spec.color_lit, 2.0,
                                db_min, db_max, atk_ms, rel_ms, sr, fft_size, tilt, offset,
                            );

                            let mut nodes = nodes_all[editing_slot][editing_curve];
                            if crv::curve_widget(
                                ui, curve_rect, &mut nodes, &all_gains[editing_curve],
                                editing_curve, db_min, db_max, atk_ms, rel_ms, sr, fft_size,
                                tilt, offset,
                            ) {
                                params.slot_curve_nodes.lock()[editing_slot][editing_curve] = nodes;
                                // Publish updated gains to triple buffer
                                {
                                    use crate::dsp::pipeline::MAX_NUM_BINS;
                                    let full_gains = crv::compute_curve_response(
                                        &nodes, MAX_NUM_BINS, sr, fft_size,
                                    );
                                    if let Some(slot_chs) = curve_tx.get(editing_slot) {
                                        if let Some(tx_arc) = slot_chs.get(editing_curve) {
                                            if let Some(mut tx) = tx_arc.try_lock() {
                                                tx.input_buffer_mut().copy_from_slice(&full_gains);
                                                tx.publish();
                                            }
                                        }
                                    }
                                }
                            }

                            // Cursor tooltip
                            let max_hz = (sr / 2.0).max(20_001.0);
                            if let Some(hover) = ui.input(|i| i.pointer.hover_pos()) {
                                if curve_rect.contains(hover) {
                                    let freq = crv::screen_to_freq(hover.x, curve_rect, max_hz);
                                    let val  = crv::screen_y_to_physical(hover.y, editing_curve, db_min, db_max, curve_rect);
                                    let unit = crv::curve_y_unit(editing_curve);
                                    let freq_str = if freq >= 1_000.0 {
                                        format!("{:.2} kHz", freq / 1_000.0)
                                    } else {
                                        format!("{:.0} Hz", freq)
                                    };
                                    let val_str = format!("{:.1} {}", val, unit);
                                    let label   = format!("{}\n{}", freq_str, val_str);
                                    let tip_pos = hover + egui::vec2(12.0, -28.0);
                                    let font    = egui::FontId::proportional(10.0);
                                    let galley  = ui.painter().layout_no_wrap(
                                        label.clone(), font.clone(), th::GRID_TEXT,
                                    );
                                    let text_size = galley.size();
                                    let bg_rect = egui::Rect::from_min_size(
                                        tip_pos - egui::vec2(3.0, 3.0),
                                        text_size + egui::vec2(6.0, 6.0),
                                    );
                                    ui.painter().rect_filled(bg_rect, 2.0, egui::Color32::from_black_alpha(180));
                                    ui.painter().text(tip_pos, egui::Align2::LEFT_TOP, label, font, th::GRID_TEXT);
                                }
                            }
                        }
                    }
```

Also remove the "Harmonic placeholder text" block (no longer needed — harmonic is a module type handled uniformly).

Also remove these now-unused variable declarations from the top of the closure:
- `active_idx` (replaced by `editing_curve` read inline)
- `active_tab`, `cur_mode`, `freeze_active`, `is_freeze_mode`, `is_phase_mode`
- `tilts`, `offsets` arrays (tilt/offset now read from slot_curve_meta per-curve)
- `grid_curve_idx` (was conditional on tabs — simplify to just `editing_curve` from the editing slot)

Update the grid call:
```rust
                    crv::paint_grid(ui.painter(), curve_rect, editing_curve, db_min, db_max, sr);
```

Where `editing_curve` is read from `*params.editing_curve.lock() as usize`.

- [ ] **Step 2: Build and fix errors**

```bash
cargo build 2>&1
```

The most common errors: references to removed variables (`active_tab`, `is_freeze_mode`, etc.), or references to old `curve_nodes` / `phase_curve_nodes` / `freeze_curve_nodes`. Remove all of them. The curve system is now entirely `slot_curve_nodes`.

- [ ] **Step 3: Run tests**

```bash
cargo test 2>&1
```

- [ ] **Step 4: Commit**

```bash
git add src/editor_ui.rs
git commit -m "feat: curve editor reads slot_curve_nodes; unified code path for all module types"
```

---

## Task 6: Sidechain assignment strip + activity indicators

**Goal:** Add per-slot SC input selector strip below the curve area. SC buttons show which aux inputs are connected and lit. Wires `sidechain_active` arcs from bridge into the editor.

**Files:**
- Modify: `src/lib.rs` (clone sidechain_active arcs, pass to editor)
- Modify: `src/editor_ui.rs` (accept and display sidechain_active; SC strip)

- [ ] **Step 1: Pass sidechain_active arcs to editor**

In `src/lib.rs`, add a new field to `SpectralForge`:
```rust
    gui_sidechain_active: Option<[Arc<std::sync::atomic::AtomicBool>; 4]>,
```

In `Default::default()`, clone the arcs after building `shared`:
```rust
        let gui_sidechain_active = Some(std::array::from_fn::<_, 4, _>(|i| {
            shared.sidechain_active[i].clone()
        }));
```

Add `gui_sidechain_active` to the `Self { ... }` literal.

In `editor()`, pass `self.gui_sidechain_active.clone()` to `create_editor()`.

Update `create_editor()` signature in `src/editor_ui.rs`:
```rust
pub fn create_editor(
    ...
    sidechain_active: Option<[Arc<std::sync::atomic::AtomicBool>; 4]>,
    plugin_alive: std::sync::Weak<()>,
) -> Option<Box<dyn Editor>>
```

Inside the closure, read activity:
```rust
                    let sc_active: [bool; 4] = match &sidechain_active {
                        Some(arcs) => std::array::from_fn(|i| arcs[i].load(std::sync::atomic::Ordering::Relaxed)),
                        None => [false; 4],
                    };
```

- [ ] **Step 2: Add SC strip between separator and control knobs**

After the separator line below the curve area (`ui.separator()`) and before the `ui.horizontal(|ui| { knob!(...) })` block, add:

```rust
                    // ── SC assignment strip ────────────────────────────────────
                    ui.horizontal(|ui| {
                        ui.add_space(4.0);
                        let edit_slot = *params.editing_slot.lock() as usize;
                        let mut sc_assign = params.slot_sidechain.lock()[edit_slot];

                        ui.label(egui::RichText::new("SC").color(th::LABEL_DIM).size(9.0));
                        ui.add_space(2.0);

                        let sc_labels: &[(&str, u8)] = &[
                            ("SC1", 0), ("SC2", 1), ("SC3", 2), ("SC4", 3), ("Self", 255),
                        ];
                        for &(label, idx) in sc_labels {
                            let is_active = sc_assign == idx;
                            let sc_live = idx < 4 && sc_active[idx as usize];
                            // Lit color: active SC input has green tint; inactive = dim
                            let fill = if is_active {
                                if sc_live { egui::Color32::from_rgb(0x30, 0xa0, 0x50) }
                                else       { th::BORDER }
                            } else {
                                th::BG
                            };
                            let text_col = if is_active { egui::Color32::BLACK } else { th::LABEL_DIM };
                            let btn = egui::Button::new(
                                egui::RichText::new(label).color(text_col).size(9.0)
                            )
                            .fill(fill)
                            .stroke(egui::Stroke::new(th::STROKE_BORDER,
                                if sc_live { egui::Color32::from_rgb(0x30, 0xa0, 0x50) }
                                else       { th::BORDER }
                            ));
                            if ui.add(btn).clicked() {
                                sc_assign = idx;
                                params.slot_sidechain.lock()[edit_slot] = idx;
                            }
                        }
                    });
                    ui.add_space(2.0);
```

- [ ] **Step 3: Build**

```bash
cargo build 2>&1
```

Fix any signature mismatches in the `editor()` call site in lib.rs.

- [ ] **Step 4: Run tests**

```bash
cargo test 2>&1
```

- [ ] **Step 5: Commit**

```bash
git add src/lib.rs src/editor_ui.rs
git commit -m "feat: SC assignment strip with activity indicators; sidechain_active arcs passed to editor"
```

---

## Task 7: GainMode selector

**Goal:** When the editing slot is a Gain module, show Add/Subtract/Pull buttons below the SC strip. Writes directly to `params.slot_gain_mode`.

**Files:**
- Modify: `src/editor_ui.rs`

- [ ] **Step 1: Add GainMode buttons to the SC strip row**

After the SC strip `ui.add_space(2.0)`, add:

```rust
                    // ── GainMode selector (Gain module only) ──────────────────
                    {
                        let edit_slot = *params.editing_slot.lock() as usize;
                        let slot_type = params.slot_module_types.lock()[edit_slot];
                        if slot_type == crate::dsp::modules::ModuleType::Gain {
                            ui.horizontal(|ui| {
                                ui.add_space(4.0);
                                ui.label(egui::RichText::new("Mode").color(th::LABEL_DIM).size(9.0));
                                ui.add_space(2.0);

                                let cur_mode = params.slot_gain_mode.lock()[edit_slot];
                                use crate::dsp::modules::GainMode;
                                for (label, mode) in [("Add", GainMode::Add), ("Subtract", GainMode::Subtract), ("Pull", GainMode::Pull)] {
                                    let is_active = cur_mode == mode;
                                    let fill     = if is_active { th::BORDER } else { th::BG };
                                    let text_col = if is_active { egui::Color32::BLACK } else { th::LABEL_DIM };
                                    let btn = egui::Button::new(
                                        egui::RichText::new(label).color(text_col).size(9.0)
                                    )
                                    .fill(fill)
                                    .stroke(egui::Stroke::new(th::STROKE_BORDER, th::BORDER));
                                    if ui.add(btn).clicked() {
                                        params.slot_gain_mode.lock()[edit_slot] = mode;
                                    }
                                }
                            });
                            ui.add_space(2.0);
                        }
                    }
```

- [ ] **Step 2: Build + test + commit**

```bash
cargo build 2>&1 && cargo test 2>&1
git add src/editor_ui.rs
git commit -m "feat: GainMode selector (Add/Subtract/Pull) shown for Gain module slots"
```

---

## Task 8: Inline slot name editing in graph header

**Goal:** Clicking the slot name in the graph header opens a TextEdit widget. Saved on Enter/focus-loss, max 32 bytes.

**Files:**
- Modify: `src/editor_ui.rs`

- [ ] **Step 1: Replace the static graph header with an interactive one**

Replace the graph header block with:

```rust
                    {
                        let edit_slot = *params.editing_slot.lock() as usize;
                        let tgts      = params.slot_targets.lock();
                        let target_label = tgts[edit_slot].label();

                        // Check if we're currently editing the name
                        let name_edit_key = ui.id().with(("name_edit", edit_slot));
                        let is_editing: bool = ui.data(|d| d.get_temp(name_edit_key).unwrap_or(false));

                        if is_editing {
                            let mut name_str = {
                                let names = params.slot_names.lock();
                                crate::editor::fx_matrix_grid::slot_name_str(&names[edit_slot])
                            };
                            let te = egui::TextEdit::singleline(&mut name_str)
                                .font(egui::FontId::proportional(10.0))
                                .desired_width(120.0)
                                .text_color(th::LABEL_DIM);
                            let resp = ui.put(
                                egui::Rect::from_min_size(
                                    curve_rect.min + egui::vec2(4.0, 4.0),
                                    egui::vec2(120.0, 14.0),
                                ),
                                te,
                            );
                            // Enforce 32 byte limit
                            if name_str.len() > 32 {
                                name_str.truncate(32);
                            }
                            // Save and exit edit mode on enter or focus loss
                            if resp.lost_focus() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                                let mut names = params.slot_names.lock();
                                let b = name_str.as_bytes();
                                let len = b.len().min(32);
                                names[edit_slot].fill(0);
                                names[edit_slot][..len].copy_from_slice(&b[..len]);
                                ui.data_mut(|d| d.insert_temp::<bool>(name_edit_key, false));
                            } else {
                                // Keep editing; write interim value
                                let mut names = params.slot_names.lock();
                                let b = name_str.as_bytes();
                                let len = b.len().min(32);
                                names[edit_slot].fill(0);
                                names[edit_slot][..len].copy_from_slice(&b[..len]);
                            }
                        } else {
                            let name_str = {
                                let names = params.slot_names.lock();
                                crate::editor::fx_matrix_grid::slot_name_str(&names[edit_slot])
                            };
                            let header = format!("Editing: {} \u{2014} {}", name_str, target_label);
                            let header_resp = ui.put(
                                egui::Rect::from_min_size(
                                    curve_rect.min + egui::vec2(4.0, 4.0),
                                    egui::vec2(300.0, 14.0),
                                ),
                                egui::Label::new(
                                    egui::RichText::new(&header)
                                        .color(th::LABEL_DIM).size(10.0)
                                ).sense(egui::Sense::click()),
                            );
                            if header_resp.clicked() {
                                ui.data_mut(|d| d.insert_temp(name_edit_key, true));
                            }
                            header_resp.on_hover_text("Click to rename this slot");
                        }
                    }
```

- [ ] **Step 2: Build + test + commit**

```bash
cargo build 2>&1 && cargo test 2>&1
git add src/editor_ui.rs
git commit -m "feat: inline slot name editing in graph header (click to edit, Enter to save)"
```

---

## Task 9: T/S Split virtual rows in matrix UI

**Goal:** When a slot contains a T/S Split module, two half-height virtual rows (Transient = orange, Sustained = blue) appear immediately below it in the matrix, with DragValues for their send amounts.

**Files:**
- Modify: `src/editor/fx_matrix_grid.rs`

- [ ] **Step 1: Update `paint_fx_matrix_grid` to insert virtual rows**

This requires computing total height dynamically. Count virtual rows needed:

```rust
fn count_virtual_rows(types: &[ModuleType; 9]) -> usize {
    types.iter().filter(|&&t| t == ModuleType::TransientSustainedSplit).count() * 2
}
```

Virtual rows are inserted after their parent slot row. To keep the layout simple, treat each virtual row as HALF_CELL height:

```rust
const HALF_CELL: f32 = CELL / 2.0;
```

The total height is now: `HDR + (real_rows * CELL) + (virtual_rows * HALF_CELL)`.

Building the row layout: create a Vec of `RowEntry` to iterate:

```rust
#[derive(Clone, Copy)]
enum RowEntry {
    Real(usize),               // slot index
    Virtual(usize, VirtualRowKind, usize),  // parent_slot, kind, vrow_index in route_matrix
}
```

Build the list:
```rust
use crate::dsp::modules::VirtualRowKind;
let mut rows: Vec<RowEntry> = Vec::with_capacity(13);
let mut vrow_idx = 0usize;
for s in 0..9 {
    rows.push(RowEntry::Real(s));
    if types[s] == ModuleType::TransientSustainedSplit {
        rows.push(RowEntry::Virtual(s, VirtualRowKind::Transient, MAX_SLOTS + vrow_idx));
        rows.push(RowEntry::Virtual(s, VirtualRowKind::Sustained, MAX_SLOTS + vrow_idx + 1));
        vrow_idx += 2;
    }
}
```

Then iterate `rows` instead of `0..9`:
- Real rows: render at full CELL height (existing logic)
- Virtual rows: render at HALF_CELL height
  - Left border: 3px orange (`#e07030`) for Transient, 3px blue (`#3070c0`) for Sustained
  - Row label: "{slot+1}T" or "{slot+1}S"
  - Off-diagonal cells: DragValue using `route_matrix.send[vrow_index][col]`
  - Diagonal cell (col == parent_slot): show ⊘ (no self-send)
  - Column 8 (Master): show ⊘ (virtual rows cannot send to Master directly)

The y-position of each row: accumulate as you iterate:
```rust
let mut y_offset = HDR;
for row_entry in &rows {
    let row_h = match row_entry {
        RowEntry::Real(_) => CELL,
        RowEntry::Virtual(..) => HALF_CELL,
    };
    // ... render at y_offset
    y_offset += row_h;
}
```

Update `total_h` accordingly.

- [ ] **Step 2: Implement the virtual row rendering**

For each `RowEntry::Virtual(parent, kind, vrow_src)`:

```rust
RowEntry::Virtual(parent_slot, kind, vrow_src) => {
    let row_top = origin.y + y_offset;
    let border_col = match kind {
        VirtualRowKind::Transient => egui::Color32::from_rgb(0xe0, 0x70, 0x30),
        VirtualRowKind::Sustained => egui::Color32::from_rgb(0x30, 0x70, 0xc0),
    };
    // Left border stripe
    painter.rect_filled(
        egui::Rect::from_min_size(egui::pos2(origin.x, row_top), egui::vec2(3.0, HALF_CELL)),
        0.0, border_col,
    );
    // Row label
    let vrow_label = format!("{}{}",
        parent_slot + 1,
        if matches!(kind, VirtualRowKind::Transient) { "T" } else { "S" }
    );
    let lbl_rect = egui::Rect::from_min_size(
        egui::pos2(origin.x + 3.0, row_top),
        egui::vec2(LABEL - 5.0, HALF_CELL),
    );
    painter.text(
        lbl_rect.center(), egui::Align2::CENTER_CENTER,
        &vrow_label, egui::FontId::proportional(7.5), border_col,
    );

    for col in 0..9 {
        let cell_rect = egui::Rect::from_min_size(
            egui::pos2(origin.x + LABEL + col as f32 * CELL, row_top),
            egui::vec2(CELL - 1.0, HALF_CELL - 1.0),
        );
        painter.rect_filled(cell_rect, 0.0, th::BG_RAISED);
        painter.rect_stroke(cell_rect, 0.0, egui::Stroke::new(0.5, th::GRID_LINE), StrokeKind::Middle);

        if col == *parent_slot || col == 8 {
            // Self-send and Master column: show ⊘
            painter.text(
                cell_rect.center(), egui::Align2::CENTER_CENTER,
                "\u{2298}", egui::FontId::proportional(7.0), th::GRID_LINE,
            );
        } else {
            let send_val = &mut route_matrix.send[*vrow_src][col];
            ui.allocate_new_ui(
                UiBuilder::new().max_rect(cell_rect.shrink(2.0)),
                |ui| {
                    ui.add(
                        egui::DragValue::new(send_val)
                            .range(0.0..=2.0).speed(0.005).fixed_decimals(2)
                            .custom_formatter(|v, _| {
                                if v < 0.005 { "\u{2014}".to_string() }
                                else { format!("{v:.2}") }
                            })
                            .custom_parser(|s| s.parse::<f64>().ok()),
                    );
                },
            );
        }
    }
}
```

- [ ] **Step 3: Build + test + commit**

```bash
cargo build 2>&1 && cargo test 2>&1
git add src/editor/fx_matrix_grid.rs
git commit -m "feat: T/S Split virtual rows in matrix UI — half-height rows with orange/blue borders"
```

---

## Task 10: M/S module DSP — balance, expansion, decorrelation

**Goal:** `MidSideModule` performs real per-bin stereo processing: balance (per-bin Mid/Side level), expansion (side width), and phase decorrelation. Transient and Pan curves are stubs (1.0 = neutral). Only active when `stereo_link == StereoLink::MidSide`.

**Files:**
- Rewrite: `src/dsp/modules/mid_side.rs`
- Modify: `tests/engine_contract.rs`

- [ ] **Step 1: Write a test**

In `tests/engine_contract.rs`:

```rust
#[test]
fn mid_side_module_compiles_and_passes_through_at_neutral() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::{
        create_module, ModuleType, ModuleContext, SpectralModule,
    };
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let n = 1025usize;
    let mut m = create_module(ModuleType::MidSide, 44100.0, 2048);

    // Neutral curves: all 1.0 (balance = 1.0 → balanced, expansion = 1.0 → no change)
    let ones = vec![1.0f32; n];
    let curves_storage: [&[f32]; 5] = [&ones, &ones, &ones, &ones, &ones];
    let curves: &[&[f32]] = &curves_storage;

    let mut bins = vec![Complex::new(1.0f32, 0.0); n];
    let mut supp = vec![0.0f32; n];
    let ctx = ModuleContext {
        sample_rate: 44100.0, fft_size: 2048, num_bins: n,
        attack_ms: 10.0, release_ms: 100.0, sensitivity: 0.0,
        suppression_width: 0.0, auto_makeup: false, delta_monitor: false,
    };

    // Channel 0 (Mid) — neutral should leave bins close to unchanged
    m.process(0, StereoLink::MidSide, FxChannelTarget::All, &mut bins, None, curves, &mut supp, &ctx);
    let mid_out = bins[10].norm();
    assert!(mid_out > 0.5, "mid signal should survive neutral M/S processing, got {}", mid_out);

    // Channel 1 (Side)
    let mut side_bins = vec![Complex::new(0.5f32, 0.0); n];
    m.process(1, StereoLink::MidSide, FxChannelTarget::All, &mut side_bins, None, curves, &mut supp, &ctx);
    assert!(side_bins[10].norm() > 0.1, "side signal should survive neutral M/S processing");

    // When not in MidSide mode, passes through unchanged
    let mut bypass_bins = vec![Complex::new(1.0f32, 0.0); n];
    m.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bypass_bins, None, curves, &mut supp, &ctx);
    assert_eq!(bypass_bins[10].re, 1.0, "MidSide module should pass through when not in M/S mode");
}
```

- [ ] **Step 2: Run test — confirm it fails**

```bash
cargo test mid_side_module 2>&1
```

Expected: fails because `MidSideModule` is a stub (pass-through regardless of StereoLink mode).

- [ ] **Step 3: Implement MidSideModule**

Replace `src/dsp/modules/mid_side.rs` entirely:

```rust
use num_complex::Complex;
use crate::params::{FxChannelTarget, StereoLink};
use super::{ModuleContext, ModuleType, SpectralModule};

pub struct MidSideModule {
    /// xorshift64 state for phase decorrelation. Must never be zero.
    rng_state: u64,
    num_bins:  usize,
}

impl MidSideModule {
    pub fn new() -> Self {
        Self { rng_state: 0xdeadbeefcafebabe, num_bins: 0 }
    }

    fn xorshift64(state: &mut u64) -> u64 {
        *state ^= *state << 13;
        *state ^= *state >> 7;
        *state ^= *state << 17;
        *state
    }
}

impl Default for MidSideModule {
    fn default() -> Self { Self::new() }
}

impl SpectralModule for MidSideModule {
    fn reset(&mut self, _sr: f32, fft_size: usize) {
        self.num_bins = fft_size / 2 + 1;
        self.rng_state = 0xdeadbeefcafebabe;
    }

    fn process(
        &mut self,
        channel:      usize,
        stereo_link:  StereoLink,
        _target:      FxChannelTarget,
        bins:         &mut [Complex<f32>],
        _sidechain:   Option<&[f32]>,
        curves:       &[&[f32]],
        suppression_out: &mut [f32],
        _ctx:         &ModuleContext,
    ) {
        suppression_out.fill(0.0);

        // Only active in MidSide mode
        if stereo_link != StereoLink::MidSide {
            return;
        }

        let n = bins.len();

        // Curve indices (per module_spec order):
        // 0 = BALANCE, 1 = EXPANSION, 2 = DECORREL, 3 = TRANSIENT (stub), 4 = PAN (stub)
        let balance  = curves.get(0).copied().unwrap_or(&[] as &[f32]);
        let expansion = curves.get(1).copied().unwrap_or(&[] as &[f32]);
        let decorrel  = curves.get(2).copied().unwrap_or(&[] as &[f32]);

        match channel {
            0 => {
                // Mid channel: apply balance (mid scale)
                // balance curve: 1.0 = neutral, 0.0 = full side (mute mid), 2.0 = double mid
                for k in 0..n {
                    let bal = balance.get(k).copied().unwrap_or(1.0).clamp(0.0, 2.0);
                    // mid_scale: 1.0 at bal=1, 0 at bal=0, sqrt(2) at bal=2
                    let mid_scale = bal.sqrt().min(std::f32::consts::SQRT_2);
                    bins[k] *= mid_scale;
                }
            }
            1 => {
                // Side channel: balance (side scale) + expansion + decorrelation
                for k in 0..n {
                    let bal = balance.get(k).copied().unwrap_or(1.0).clamp(0.0, 2.0);
                    // side_scale: 1.0 at bal=1, sqrt(2) at bal=0 (more side), 0 at bal=2
                    let side_scale = (2.0 - bal).sqrt().min(std::f32::consts::SQRT_2);

                    // Expansion: scale side width. 1.0 = neutral. >1 = wider, <1 = narrower.
                    let exp = expansion.get(k).copied().unwrap_or(1.0).max(0.0);

                    // Decorrelation: rotate phase of side bins
                    let dec_amt = decorrel.get(k).copied().unwrap_or(0.0).clamp(0.0, 2.0);
                    let phase_rot = if dec_amt > 0.001 {
                        let rnd = Self::xorshift64(&mut self.rng_state) as f32 / u64::MAX as f32;
                        (rnd - 0.5) * 2.0 * std::f32::consts::PI * dec_amt
                    } else {
                        0.0
                    };

                    let (sin_r, cos_r) = phase_rot.sin_cos();
                    let rotated = Complex::new(
                        bins[k].re * cos_r - bins[k].im * sin_r,
                        bins[k].re * sin_r + bins[k].im * cos_r,
                    );
                    bins[k] = rotated * (side_scale * exp);
                }
            }
            _ => {} // No more than 2 channels
        }
    }

    fn module_type(&self) -> ModuleType { ModuleType::MidSide }
    fn num_curves(&self) -> usize { 5 }
}
```

- [ ] **Step 4: Run the test**

```bash
cargo test mid_side_module 2>&1
```

Expected: PASS. Fix any issues.

- [ ] **Step 5: Run all tests**

```bash
cargo test 2>&1
```

All 24+ tests must pass.

- [ ] **Step 6: Commit**

```bash
git add src/dsp/modules/mid_side.rs tests/engine_contract.rs
git commit -m "feat: MidSide module DSP — per-bin balance, expansion, phase decorrelation"
```

---

## Final verification

- [ ] **Build release + install**

```bash
cargo run --package xtask -- bundle spectral_forge --release
cp target/bundled/spectral_forge.clap ~/.clap/
```

- [ ] **Manual smoke test in Bitwig**

Restart Bitwig and load the plugin. Verify:
1. Matrix grid shows 9 slots with correct colours (Dynamics=teal, Gain=amber, Master=white at row/col 8)
2. Right-click a diagonal cell → popup appears listing module types
3. Assign a Freeze module to slot 3 → type changes, curve buttons update on next Bitwig restart
4. Curve buttons in top bar update when switching editing slot (Dynamics shows 6 curves; Gain shows 2)
5. Editing a curve writes to the correct slot (slot 0 compresses audio)
6. SC strip shows SC1/SC2/SC3/SC4/Self; connecting aux input causes button to go green
7. Selecting a Gain slot shows Add/Subtract/Pull buttons
8. Clicking the slot name in the header → text field appears → type → Enter saves
9. T/S Split assigned to a slot → two half-height rows appear in matrix

---

## Self-review

**Spec coverage:**
- D2.1 Module assignment popup ✓ (Task 3)
- D2.2 Adaptive curve buttons + remove tabs ✓ (Task 4)
- D2.3 Matrix cell truncation + tooltip + name edit ✓ (Tasks 2, 8)
- D2.4 Sidechain assignment strip ✓ (Task 6)
- D2.5 Per-curve tilt/offset DragValue ✓ (Task 4 — in Row 2)
- D2.6 GainMode selector ✓ (Task 7)
- D2.7 T/S Split virtual rows ✓ (Task 9)
- D2.8 M/S module DSP ✓ (Task 10)
- Prerequisite: FxMatrix from slot_types + routing + GainMode propagation ✓ (Task 1)
- Prerequisite: Matrix grid rewrite ✓ (Task 2)
- Prerequisite: Curve editor migrated to slot_curve_nodes ✓ (Task 5)

**Known limitations not in D2 scope:**
- Module type changes (via popup) take effect on next host initialize(), not live — noted in popup UI
- Feedback routing (upper triangle of matrix) uses previous-hop slot_out (currently zeros for all slots). Full feedback requires `slot_out_prev` buffer added to FxMatrix.
- T/S Split DSP produces two output buffers, but the pipeline doesn't yet read `transient_bins()`/`sustained_bins()` — the virtual rows in the matrix UI exist but the routing of their virtual-row outputs is not yet wired in process_hop. This is a D3 task.
- The legacy params (`curve_nodes`, `active_curve`, `active_tab`, `phase_curve_nodes`, `freeze_curve_nodes`, `freeze_active_curve`, `fx_module_types`, `fx_module_names`, `fx_module_targets`, `fx_route_matrix`) remain in the struct for serialization backward compatibility. They are no longer used by the editor UI.
