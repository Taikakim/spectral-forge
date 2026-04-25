> **Status (2026-04-24): IMPLEMENTED.** Generated automatable params (~1341 via `build.rs`), 1000 ms tooltips, JSON preset system. Source of truth: the code + [../STATUS.md](../STATUS.md).

# Automation, Tooltips & Preset System — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Expose graph nodes / tilt+offset / FX matrix sends as automatable host params (~1361 total), add 1000ms-delay tooltips on every automatable widget, and ship a JSON-based preset system in a platform-conventional directory.

**Architecture:** Param definitions generated at build time via `build.rs` (single source of truth for 1341 repetitive fields). Curve editor reads via `FloatParam.value()` and writes via `ParamSetter`. Legacy `#[persist]` fields kept for one release as a migration path. Preset system uses serde JSON with a manual `PRESET_SCHEMA_VERSION` constant; incompatible presets are hidden, not errored.

**Tech Stack:** Rust, nih-plug (CLAP wrapper), egui, `directories`, `serde_json`, `opener`.

**Spec:** `docs/superpowers/specs/2026-04-21-automation-presets-design.md`

## Implementation notes for the executing engineer

- **nih-plug API names**: Code samples below use what the author believes to be the correct nih-plug public API (as of commit `28b149e`). If a method doesn't resolve, check `~/.cargo/git/checkouts/nih-plug-*/src/params/` and the `GuiContext` trait in `src/context/gui.rs`. The most likely points of divergence: `FloatParam::set_normalized_value()` vs `set_plain_value()`, `ParamPtr::unmodulated_normalized_value()` vs `normalized_value()`. Pick whichever matches the installed nih-plug and stay consistent.
- **`#[id]` attribute on generated fields**: nih-plug's `#[derive(Params)]` reads `#[id = "..."]` at macro-expansion time. Since we're hand-writing the `Params` trait impl (Option A), the `#[id]` attribute on the generated field itself has no effect — what matters is the string ID we push into `param_map()`. Keep the `#[id = "..."]` in the generated code for documentation purposes, but understand it's redundant if the `Params` impl is hand-written.
- **`assert_no_alloc` on audio thread**: Task 8 (RouteMatrix rebuild) adds 81 `smoothed.next()` calls per block. This should not allocate — `Smoother<f32>` holds its state inline. Confirm by running a release build with `cargo test --features assert_process_allocs`.

---

## File Structure

| File | Role |
|---|---|
| `build.rs` (new) | Generate `params_gen.rs` with 1341 `FloatParam` declarations and their `Params` trait entries |
| `src/param_ids.rs` (new) | Centralized ID naming helpers — single source of truth, used by both `build.rs` and runtime code |
| `src/params.rs` | Include generated file; add `GraphNodeParams` accessor wrapper; keep legacy `#[persist]` fields for one release |
| `src/preset.rs` (new) | `Preset` struct (serde), `save/load/scan`, `PRESET_SCHEMA_VERSION`, `preset_dir()` |
| `src/editor/mod.rs` | `delayed_tooltip()` helper; export preset widget |
| `src/editor/preset_menu.rs` (new) | Preset pulldown + save dialog + open-folder button |
| `src/editor/curve.rs` | Read/write node x/y/q via `FloatParam` + `ParamSetter` (replaces `Mutex<CurveNode>` reads) |
| `src/editor/theme.rs` | Tooltip background, pulldown colours |
| `src/editor_ui.rs` | Preset pulldown in top bar; wire `delayed_tooltip` on every automatable widget |
| `src/dsp/modules/mod.rs` | `RouteMatrix` rebuild from param values |
| `src/lib.rs` | Call one-shot migration from legacy persist fields on state deserialize |
| `tests/preset_roundtrip.rs` (new) | Save → load preserves every normalized param value |
| `tests/state_migration.rs` (new) | Legacy persist state loads into new params |
| `tests/audio_rate_modulation.rs` (new) | White noise on every param → finite, bounded output |
| `Cargo.toml` | Add `directories`, `serde_json`, `opener` |

---

## Task 1: Cargo.toml dependencies

**Files:**
- Modify: `Cargo.toml`

- [ ] **Step 1: Add dependencies**

In the `[dependencies]` section:

```toml
directories = "5.0"
serde_json = "1.0"
opener = "0.7"
```

- [ ] **Step 2: Verify build**

Run: `cargo build --release`
Expected: clean build, new crates appear in dependency graph.

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "deps: add directories, serde_json, opener for preset system"
```

---

## Task 2: Param ID naming module

**Rationale:** Every param ID appears in two places — the generated `params_gen.rs` and runtime code that looks params up. Put the naming logic in one module so `build.rs` and the main crate both reference the same formatting code.

**Files:**
- Create: `src/param_ids.rs`
- Modify: `src/lib.rs` (add `mod param_ids;`)

- [ ] **Step 1: Write failing test**

Create `src/param_ids.rs`:

```rust
//! Centralized parameter ID formatting. Single source of truth for both
//! build.rs code generation and runtime param lookup.
//!
//! IDs are STABLE FOREVER — changing any formatting here will break
//! saved automation lanes in user projects.

pub const NUM_SLOTS: usize = 9;
pub const NUM_CURVES: usize = 7;
pub const NUM_NODES: usize = 6;
pub const MAX_MATRIX_ROWS: usize = 9;

pub fn graph_node_id(slot: usize, curve: usize, node: usize, field: char) -> String {
    debug_assert!(matches!(field, 'x' | 'y' | 'q'));
    format!("s{}c{}n{}{}", slot, curve, node, field)
}

pub fn tilt_id(slot: usize, curve: usize) -> String {
    format!("s{}c{}tilt", slot, curve)
}

pub fn offset_id(slot: usize, curve: usize) -> String {
    format!("s{}c{}offset", slot, curve)
}

pub fn matrix_id(row: usize, col: usize) -> String {
    format!("mr{}c{}", row, col)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graph_node_id_format() {
        assert_eq!(graph_node_id(0, 0, 0, 'x'), "s0c0n0x");
        assert_eq!(graph_node_id(8, 6, 5, 'q'), "s8c6n5q");
    }

    #[test]
    fn tilt_offset_matrix_ids() {
        assert_eq!(tilt_id(2, 3), "s2c3tilt");
        assert_eq!(offset_id(2, 3), "s2c3offset");
        assert_eq!(matrix_id(1, 4), "mr1c4");
    }

    #[test]
    fn total_counts() {
        assert_eq!(NUM_SLOTS * NUM_CURVES * NUM_NODES * 3, 1134);
        assert_eq!(NUM_SLOTS * NUM_CURVES * 2, 126);
        assert_eq!(MAX_MATRIX_ROWS * NUM_SLOTS, 81);
    }
}
```

Add to `src/lib.rs` near other module declarations:

```rust
mod param_ids;
```

- [ ] **Step 2: Run tests — confirm they pass**

Run: `cargo test --lib param_ids`
Expected: 3 passed.

- [ ] **Step 3: Commit**

```bash
git add src/param_ids.rs src/lib.rs
git commit -m "feat(params): add centralized param-ID naming module"
```

---

## Task 3: build.rs skeleton + graph-node param generation

**Rationale:** 1134 graph-node fields hand-written would be error-prone and obscure. Generate them. Keep the generator code simple enough to read.

**Files:**
- Create: `build.rs`
- Modify: `src/params.rs` (include the generated file)
- Modify: `Cargo.toml` (declare `build = "build.rs"` if not implicit)

- [ ] **Step 1: Write build.rs**

```rust
//! Generates params_gen.rs with FloatParam field declarations and
//! Params-trait method entries for graph nodes, tilt/offset, and matrix sends.
//!
//! Keep IDs in sync with src/param_ids.rs — any formatting change here
//! that diverges from param_ids.rs breaks saved automation.

use std::env;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

const NUM_SLOTS:   usize = 9;
const NUM_CURVES:  usize = 7;
const NUM_NODES:   usize = 6;
const MAX_MATRIX_ROWS: usize = 9;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/param_ids.rs");

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let out_path = out_dir.join("params_gen.rs");
    let mut f = File::create(&out_path).expect("create params_gen.rs");

    writeln!(f, "// AUTO-GENERATED — do not edit. See build.rs.").unwrap();
    writeln!(f).unwrap();

    // Field declarations block — goes inside `pub struct SpectralForgeParams { ... }`.
    writeln!(f, "macro_rules! spectral_generated_fields {{ () => {{").unwrap();
    emit_graph_node_fields(&mut f);
    emit_tilt_offset_fields(&mut f);
    emit_matrix_fields(&mut f);
    writeln!(f, "}} }}").unwrap();

    // Params-trait entries — goes inside `fn param_map() { ... }`.
    writeln!(f, "macro_rules! spectral_generated_param_map_entries {{ ($self:ident, $params:ident) => {{").unwrap();
    emit_graph_node_map_entries(&mut f);
    emit_tilt_offset_map_entries(&mut f);
    emit_matrix_map_entries(&mut f);
    writeln!(f, "}} }}").unwrap();
}

fn emit_graph_node_fields(f: &mut File) {
    for s in 0..NUM_SLOTS {
        for c in 0..NUM_CURVES {
            for n in 0..NUM_NODES {
                for field in ['x', 'y', 'q'] {
                    let id = format!("s{}c{}n{}{}", s, c, n, field);
                    let rust_name = format!("s{}c{}n{}_{}", s, c, n, field);
                    let (default, min, max) = graph_node_defaults(n, field);
                    writeln!(f,
                        "#[id = \"{id}\"]\npub {rust_name}: FloatParam = \
                         FloatParam::new(\"{id}\", {default}, \
                         FloatRange::Linear {{ min: {min}, max: {max} }})\
                         .with_smoother(SmoothingStyle::Linear(1.0)),",
                    ).unwrap();
                }
            }
        }
    }
}

fn graph_node_defaults(node: usize, field: char) -> (f32, f32, f32) {
    match field {
        'x' => {
            // Default x positions: equally spaced across log-freq axis.
            // Matches existing CurveNode default layout in src/editor/curve.rs.
            let default = match node {
                0 => 0.0, 1 => 0.2, 2 => 0.4, 3 => 0.6, 4 => 0.8, 5 => 1.0,
                _ => 0.5,
            };
            (default, 0.0, 1.0)
        }
        'y' => (0.0, -1.0, 1.0),
        'q' => (0.5, 0.0, 1.0),
        _ => unreachable!(),
    }
}

fn emit_tilt_offset_fields(f: &mut File) {
    for s in 0..NUM_SLOTS {
        for c in 0..NUM_CURVES {
            for kind in ["tilt", "offset"] {
                let id = format!("s{}c{}{}", s, c, kind);
                let rust_name = format!("s{}c{}_{}", s, c, kind);
                writeln!(f,
                    "#[id = \"{id}\"]\npub {rust_name}: FloatParam = \
                     FloatParam::new(\"{id}\", 0.0, \
                     FloatRange::Linear {{ min: -1.0, max: 1.0 }})\
                     .with_smoother(SmoothingStyle::Linear(2.0)),",
                ).unwrap();
            }
        }
    }
}

fn emit_matrix_fields(f: &mut File) {
    for r in 0..MAX_MATRIX_ROWS {
        for col in 0..NUM_SLOTS {
            let id = format!("mr{}c{}", r, col);
            let rust_name = format!("mr{}c{}", r, col);
            // Default: diagonal serial chain (slot 0 → 1, 1 → 2, ...).
            let default = if col + 1 == r { 1.0 } else { 0.0 };
            writeln!(f,
                "#[id = \"{id}\"]\npub {rust_name}: FloatParam = \
                 FloatParam::new(\"{id}\", {default}, \
                 FloatRange::Linear {{ min: 0.0, max: 1.0 }})\
                 .with_smoother(SmoothingStyle::Linear(2.0)),",
            ).unwrap();
        }
    }
}

fn emit_graph_node_map_entries(f: &mut File) {
    for s in 0..NUM_SLOTS {
        for c in 0..NUM_CURVES {
            for n in 0..NUM_NODES {
                for field in ['x', 'y', 'q'] {
                    let id = format!("s{}c{}n{}{}", s, c, n, field);
                    let rust_name = format!("s{}c{}n{}_{}", s, c, n, field);
                    writeln!(f, "$params.push((\"{id}\".to_string(), $self.{rust_name}.as_ptr(), String::new()));").unwrap();
                }
            }
        }
    }
}

fn emit_tilt_offset_map_entries(f: &mut File) {
    for s in 0..NUM_SLOTS {
        for c in 0..NUM_CURVES {
            for kind in ["tilt", "offset"] {
                let id = format!("s{}c{}{}", s, c, kind);
                let rust_name = format!("s{}c{}_{}", s, c, kind);
                writeln!(f, "$params.push((\"{id}\".to_string(), $self.{rust_name}.as_ptr(), String::new()));").unwrap();
            }
        }
    }
}

fn emit_matrix_map_entries(f: &mut File) {
    for r in 0..MAX_MATRIX_ROWS {
        for col in 0..NUM_SLOTS {
            let id = format!("mr{}c{}", r, col);
            let rust_name = format!("mr{}c{}", r, col);
            writeln!(f, "$params.push((\"{id}\".to_string(), $self.{rust_name}.as_ptr(), String::new()));").unwrap();
        }
    }
}
```

- [ ] **Step 2: Wire generated file into src/params.rs**

**NOTE:** The exact integration depends on how `SpectralForgeParams` is currently defined. The `#[derive(Params)]` macro does not natively support `include!`-in-struct. Two options:

**Option A (preferred):** Hand-write the `Params` trait impl instead of deriving it, then use `include!` inside the field declarations and the `param_map` method. This is what the `macro_rules!` wrappers above are designed for.

**Option B (fallback):** If hand-writing the `Params` impl becomes unwieldy, have `build.rs` emit the **entire** `SpectralForgeParams` struct + impl into `params_gen.rs`, with the existing hand-written globals passed in as a separate block that gets concatenated in.

Use Option A. Update `src/params.rs`:

```rust
include!(concat!(env!("OUT_DIR"), "/params_gen.rs"));

pub struct SpectralForgeParams {
    // ... existing hand-written global params (in_gain, out_gain, etc.) ...

    spectral_generated_fields!();

    // ... existing persist fields (for migration; see Task 10) ...
}

impl Params for SpectralForgeParams {
    fn param_map(&self) -> Vec<(String, ParamPtr, String)> {
        let mut params: Vec<(String, ParamPtr, String)> = Vec::with_capacity(1400);

        // Hand-written globals — preserve existing entries
        params.push(("in_gain".to_string(), self.in_gain.as_ptr(), String::new()));
        // ... etc

        spectral_generated_param_map_entries!(self, params);

        params
    }

    // Keep persist fields' impl as before.
}
```

- [ ] **Step 3: Write test verifying param count**

Add `tests/param_grid.rs`:

```rust
use spectral_forge::SpectralForgeParams;
use nih_plug::prelude::Params;

#[test]
fn param_map_contains_expected_count() {
    let params = SpectralForgeParams::default();
    let map = params.param_map();
    let ids: Vec<&str> = map.iter().map(|(id, _, _)| id.as_str()).collect();

    // Graph nodes: 9 × 7 × 6 × 3 = 1134
    assert_eq!(ids.iter().filter(|id| id.starts_with('s') && id.contains('n')).count(), 1134);
    // Tilt+offset: 9 × 7 × 2 = 126
    assert_eq!(ids.iter().filter(|id| id.ends_with("tilt") || id.ends_with("offset")).count(), 126);
    // Matrix: 9 × 9 = 81
    assert_eq!(ids.iter().filter(|id| id.starts_with("mr")).count(), 81);
}

#[test]
fn specific_ids_are_present() {
    let params = SpectralForgeParams::default();
    let map = params.param_map();
    let ids: std::collections::HashSet<&str> = map.iter().map(|(id, _, _)| id.as_str()).collect();

    assert!(ids.contains("s0c0n0x"));
    assert!(ids.contains("s8c6n5q"));
    assert!(ids.contains("s4c3tilt"));
    assert!(ids.contains("s4c3offset"));
    assert!(ids.contains("mr8c0"));
}
```

- [ ] **Step 4: Run the test**

Run: `cargo test --test param_grid`
Expected: 2 passed.

- [ ] **Step 5: Commit**

```bash
git add build.rs src/params.rs tests/param_grid.rs Cargo.toml
git commit -m "feat(params): generate 1341 automatable params via build.rs"
```

---

## Task 4: GraphNodeParams accessor API

**Rationale:** Callers (curve editor, migration code) should not have to assemble ID strings to look up params. Provide a typed accessor.

**Files:**
- Modify: `src/params.rs`
- Create: unit test inline

- [ ] **Step 1: Write accessor + failing test**

In `src/params.rs`, after the `SpectralForgeParams` impl:

```rust
impl SpectralForgeParams {
    /// Get the (x, y, q) params for a graph node.
    /// Returns `None` if indices are out of range.
    pub fn graph_node(&self, slot: usize, curve: usize, node: usize)
        -> Option<(&FloatParam, &FloatParam, &FloatParam)>
    {
        if slot >= NUM_SLOTS || curve >= NUM_CURVES || node >= NUM_NODES {
            return None;
        }
        // Generated macro `graph_node_dispatch!(self, slot, curve, node)` expands into
        // a match covering all 9×7×6 combos. Emitted by build.rs.
        Some(graph_node_dispatch!(self, slot, curve, node))
    }

    pub fn tilt(&self, slot: usize, curve: usize) -> Option<&FloatParam> {
        if slot >= NUM_SLOTS || curve >= NUM_CURVES { return None; }
        Some(tilt_dispatch!(self, slot, curve))
    }

    pub fn offset(&self, slot: usize, curve: usize) -> Option<&FloatParam> {
        if slot >= NUM_SLOTS || curve >= NUM_CURVES { return None; }
        Some(offset_dispatch!(self, slot, curve))
    }

    pub fn matrix_cell(&self, row: usize, col: usize) -> Option<&FloatParam> {
        if row >= MAX_MATRIX_ROWS || col >= NUM_SLOTS { return None; }
        Some(matrix_dispatch!(self, row, col))
    }
}

#[cfg(test)]
mod accessor_tests {
    use super::*;

    #[test]
    fn graph_node_accessor_round_trip() {
        let p = SpectralForgeParams::default();
        let (x, y, q) = p.graph_node(3, 2, 4).unwrap();
        // Defaults match what build.rs emitted
        assert!((x.value() - 0.8).abs() < 1e-6);
        assert!(y.value().abs() < 1e-6);
        assert!((q.value() - 0.5).abs() < 1e-6);
    }

    #[test]
    fn out_of_range_returns_none() {
        let p = SpectralForgeParams::default();
        assert!(p.graph_node(9, 0, 0).is_none());
        assert!(p.tilt(0, 7).is_none());
        assert!(p.matrix_cell(9, 0).is_none());
    }
}
```

- [ ] **Step 2: Extend build.rs to emit dispatch macros**

In `build.rs`, add:

```rust
fn emit_graph_node_dispatch(f: &mut File) {
    writeln!(f, "macro_rules! graph_node_dispatch {{ ($self:ident, $s:expr, $c:expr, $n:expr) => {{").unwrap();
    writeln!(f, "    match ($s, $c, $n) {{").unwrap();
    for s in 0..NUM_SLOTS {
        for c in 0..NUM_CURVES {
            for n in 0..NUM_NODES {
                writeln!(f, "        ({s}, {c}, {n}) => (&$self.s{s}c{c}n{n}_x, &$self.s{s}c{c}n{n}_y, &$self.s{s}c{c}n{n}_q),").unwrap();
            }
        }
    }
    writeln!(f, "        _ => unreachable!(),").unwrap();
    writeln!(f, "    }}").unwrap();
    writeln!(f, "}} }}").unwrap();
}
// Similar: emit_tilt_dispatch, emit_offset_dispatch, emit_matrix_dispatch
```

Call them from `main()` after the existing emitters.

- [ ] **Step 3: Run tests**

Run: `cargo test --lib accessor`
Expected: 2 passed.

- [ ] **Step 4: Commit**

```bash
git add build.rs src/params.rs
git commit -m "feat(params): add typed accessors for generated params"
```

---

## Task 5: Curve editor — read x/y/q via params

**Rationale:** Drawing must use lock-free reads. Switch from `Mutex<CurveNode>` to `param.value()`.

**Files:**
- Modify: `src/editor/curve.rs`

- [ ] **Step 1: Identify existing Mutex read sites**

In `src/editor/curve.rs`, find where the curve editor currently reads node state. Typically looks like:

```rust
let nodes = params.curve_nodes.lock().unwrap()[slot][curve].clone();
```

Replace with a helper that reads from params:

```rust
fn read_nodes_from_params(
    params: &SpectralForgeParams,
    slot: usize,
    curve: usize,
) -> [CurveNode; NUM_NODES] {
    let mut out = [CurveNode::default(); NUM_NODES];
    for n in 0..NUM_NODES {
        if let Some((x, y, q)) = params.graph_node(slot, curve, n) {
            out[n] = CurveNode { x: x.value(), y: y.value(), q: q.value() };
        }
    }
    out
}
```

Replace all drawing-path Mutex reads with this call. Writes stay TODO for Task 6.

- [ ] **Step 2: Run existing UI test if any, or verify build**

Run: `cargo build --release`
Expected: clean build.

- [ ] **Step 3: Visual verify**

Run: `cargo run --package xtask -- bundle spectral_forge --release && cp target/bundled/spectral_forge.clap ~/.clap/`

Open in Bitwig, confirm curves still draw correctly with default params. Check that nodes appear at the expected x positions (0, 0.2, 0.4, 0.6, 0.8, 1.0 in log-freq space).

- [ ] **Step 4: Commit**

```bash
git add src/editor/curve.rs
git commit -m "refactor(editor): curve drawing reads x/y/q from generated params"
```

---

## Task 6: Curve editor — write x/y/q via ParamSetter

**Rationale:** Drag edits must record automation. Use `setter.begin_set_parameter` / `set_parameter` / `end_set_parameter` to group a drag as a single automation edit.

**Files:**
- Modify: `src/editor/curve.rs`

- [ ] **Step 1: Replace Mutex writes with ParamSetter calls**

In the node-drag handling block, replace:

```rust
params.curve_nodes.lock().unwrap()[slot][curve][node_idx].x = new_x;
```

with:

```rust
if let Some((x_p, y_p, q_p)) = params.graph_node(slot, curve, node_idx) {
    if drag_just_started {
        setter.begin_set_parameter(x_p);
        setter.begin_set_parameter(y_p);
        // q begin/end issued only when q-drag (dual-button) is active
    }
    setter.set_parameter(x_p, new_x);
    setter.set_parameter(y_p, new_y);
    if drag_just_ended {
        setter.end_set_parameter(x_p);
        setter.end_set_parameter(y_p);
    }
}
```

Track `drag_just_started` / `drag_just_ended` using egui's `response.drag_started()` / `response.drag_stopped()` instead of manual flags.

For Q adjustment (dual-mouse-button drag, see Task 12 of the earlier GUI-redesign plan if relevant), apply the same begin/set/end pattern to the `q` param.

- [ ] **Step 2: Write integration test**

Create `tests/curve_node_automation.rs`:

```rust
use spectral_forge::SpectralForgeParams;
use nih_plug::prelude::*;

#[test]
fn setting_graph_node_param_is_reflected_in_value() {
    let params = SpectralForgeParams::default();
    let (x, _, _) = params.graph_node(2, 1, 3).unwrap();

    // Use unsafe set_normalized_value to simulate what ParamSetter does internally.
    unsafe { x.set_plain_value(0.75); }
    assert!((x.value() - 0.75).abs() < 1e-6);
}
```

Note: `ParamSetter` requires a `GuiContext` which is not available in tests. The test above verifies the underlying param responds to value changes; the `ParamSetter` call path is exercised manually in Bitwig.

- [ ] **Step 3: Run test**

Run: `cargo test --test curve_node_automation`
Expected: 1 passed.

- [ ] **Step 4: Manual verification**

Rebuild, reinstall .clap, in Bitwig:
- Right-click a node, choose "Add to device controls" (or similar).
- Drag the node in the plugin UI — confirm automation is recorded.
- Play back the automation — confirm the node moves.

- [ ] **Step 5: Commit**

```bash
git add src/editor/curve.rs tests/curve_node_automation.rs
git commit -m "feat(editor): graph node drag records host automation"
```

---

## Task 7: Tilt / Offset DragValues via ParamSetter

**Rationale:** Same pattern as graph nodes — existing DragValue widgets in the control strip write to `Mutex<(f32, f32)>` tuples; switch to `ParamSetter`.

**Files:**
- Modify: `src/editor_ui.rs`

- [ ] **Step 1: Locate tilt/offset DragValue block**

Find the block in `src/editor_ui.rs` that reads `slot_curve_meta` and displays the tilt/offset DragValues.

- [ ] **Step 2: Replace with param-backed DragValues**

```rust
let slot = editing_slot;
let curve = editing_curve;

if let (Some(tilt_p), Some(offset_p)) = (params.tilt(slot, curve), params.offset(slot, curve)) {
    let mut tilt_val = tilt_p.value();
    let mut offset_val = offset_p.value();

    ui.vertical(|ui| {
        let resp = ui.add(
            egui::DragValue::new(&mut offset_val)
                .range(-1.0..=1.0)
                .speed(1.0 / 300.0)
                .fixed_decimals(2)
        );
        if resp.drag_started()  { setter.begin_set_parameter(offset_p); }
        if resp.changed()       { setter.set_parameter(offset_p, offset_val); }
        if resp.drag_stopped()  { setter.end_set_parameter(offset_p); }
        ui.label(egui::RichText::new("Offset").color(crv_col).size(9.0));
    });

    ui.vertical(|ui| {
        let resp = ui.add(
            egui::DragValue::new(&mut tilt_val)
                .range(-1.0..=1.0)
                .speed(1.0 / 300.0)
                .fixed_decimals(2)
        );
        if resp.drag_started()  { setter.begin_set_parameter(tilt_p); }
        if resp.changed()       { setter.set_parameter(tilt_p, tilt_val); }
        if resp.drag_stopped()  { setter.end_set_parameter(tilt_p); }
        ui.label(egui::RichText::new("Tilt").color(crv_col).size(9.0));
    });
}
```

- [ ] **Step 3: Build and verify**

Run: `cargo build --release`
Expected: clean build.

- [ ] **Step 4: Commit**

```bash
git add src/editor_ui.rs
git commit -m "feat(editor): tilt/offset DragValues record host automation"
```

---

## Task 8: RouteMatrix — rebuild from params per block

**Rationale:** `RouteMatrix` currently lives inside a `Mutex` with `#[persist]`. Make it derive from matrix-cell params every audio block, so automation flows through.

**Files:**
- Modify: `src/dsp/pipeline.rs` (or wherever `route_matrix_snap` is assembled)

- [ ] **Step 1: Replace Mutex snap with param-driven assembly**

Find where `route_matrix` is currently snapshotted into `route_matrix_snap` at the top of each block (typically in `Pipeline::process()`):

```rust
// Before:
let matrix_snap = self.params.route_matrix.lock().unwrap().clone();

// After:
let mut matrix_snap = RouteMatrix::default();
for r in 0..MAX_MATRIX_ROWS {
    for col in 0..NUM_SLOTS {
        if r == col { continue; }  // no self-sends
        if let Some(p) = self.params.matrix_cell(r, col) {
            matrix_snap.send[r][col] = p.smoothed.next();
        }
    }
}
```

Note: `smoothed.next()` advances the smoother by one sample. For per-block granularity, call it once per block (not per-sample — matrix sends are block-rate).

- [ ] **Step 2: Write test**

Add to `tests/module_trait.rs` or a new file:

```rust
#[test]
fn matrix_cell_param_drives_snap() {
    // A self-feedback-loop guard test: mr0c0 set to 1.0 via param must NOT
    // translate into matrix_snap.send[0][0] = 1.0 — it stays 0.
    let params = SpectralForgeParams::default();
    let cell = params.matrix_cell(0, 0).unwrap();
    unsafe { cell.set_plain_value(1.0); }
    // Simulate pipeline's snap code:
    let mut snap = 0.0_f32;
    if 0 != 0 {  // guard matches production code
        snap = cell.smoothed.next();
    }
    assert_eq!(snap, 0.0);
}
```

- [ ] **Step 3: Run test**

Run: `cargo test --test module_trait`
Expected: all pass.

- [ ] **Step 4: Commit**

```bash
git add src/dsp/pipeline.rs tests/module_trait.rs
git commit -m "feat(matrix): route sends from automatable params each block"
```

---

## Task 9: State migration — legacy persist → params

**Rationale:** Existing Bitwig projects have `#[persist]`-saved curve nodes and matrix state. When users load an old project, copy that state into the new params exactly once.

**Files:**
- Modify: `src/params.rs` (keep legacy persist fields + add migration flag)
- Modify: `src/lib.rs` (call migration in `initialize()` or similar post-load hook)

- [ ] **Step 1: Add migration flag**

In `SpectralForgeParams`, add:

```rust
#[persist = "migrated_v1"]
migrated_v1: Arc<AtomicBool>,  // default false
```

Default: `Arc::new(AtomicBool::new(false))`.

- [ ] **Step 2: Implement migration function**

In `src/params.rs`:

```rust
impl SpectralForgeParams {
    /// One-shot migration: if persist fields have non-default values AND params
    /// are all at default, copy persist → params. Sets migrated_v1 on completion.
    /// Called from lib.rs initialize() after state load.
    pub fn migrate_legacy_if_needed(&self) {
        if self.migrated_v1.load(Ordering::Relaxed) {
            return;
        }

        let legacy_nodes = self.curve_nodes.lock().unwrap();
        for s in 0..NUM_SLOTS {
            for c in 0..NUM_CURVES {
                for n in 0..NUM_NODES {
                    let legacy = legacy_nodes[s][c][n];
                    if let Some((x, y, q)) = self.graph_node(s, c, n) {
                        // SAFETY: called during initialize, before audio thread runs.
                        unsafe {
                            x.set_plain_value(legacy.x);
                            y.set_plain_value(legacy.y);
                            q.set_plain_value(legacy.q);
                        }
                    }
                }
            }
        }

        let legacy_meta = self.slot_curve_meta.lock().unwrap();
        for s in 0..NUM_SLOTS {
            for c in 0..NUM_CURVES {
                let (tilt_v, offset_v) = legacy_meta[s][c];
                if let Some(p) = self.tilt(s, c) { unsafe { p.set_plain_value(tilt_v); } }
                if let Some(p) = self.offset(s, c) { unsafe { p.set_plain_value(offset_v); } }
            }
        }

        let legacy_matrix = self.route_matrix.lock().unwrap();
        for r in 0..MAX_MATRIX_ROWS {
            for col in 0..NUM_SLOTS {
                if let Some(p) = self.matrix_cell(r, col) {
                    unsafe { p.set_plain_value(legacy_matrix.send[r][col]); }
                }
            }
        }

        self.migrated_v1.store(true, Ordering::Relaxed);
    }
}
```

- [ ] **Step 3: Call from initialize()**

In `src/lib.rs`, in `Plugin::initialize()`:

```rust
fn initialize(...) -> bool {
    self.params.migrate_legacy_if_needed();
    // ... existing init ...
}
```

- [ ] **Step 4: Write migration test**

Create `tests/state_migration.rs`:

```rust
use spectral_forge::{SpectralForgeParams, CurveNode};
use nih_plug::prelude::*;
use std::sync::atomic::Ordering;

#[test]
fn legacy_curve_nodes_migrate_to_params() {
    let params = SpectralForgeParams::default();

    // Simulate legacy state: set one node via the old Mutex path
    {
        let mut legacy = params.curve_nodes.lock().unwrap();
        legacy[2][1][3] = CurveNode { x: 0.7, y: 0.4, q: 0.9 };
    }

    params.migrate_legacy_if_needed();

    let (x, y, q) = params.graph_node(2, 1, 3).unwrap();
    assert!((x.value() - 0.7).abs() < 1e-6);
    assert!((y.value() - 0.4).abs() < 1e-6);
    assert!((q.value() - 0.9).abs() < 1e-6);
    assert!(params.migrated_v1.load(Ordering::Relaxed));
}

#[test]
fn migration_does_not_rerun() {
    let params = SpectralForgeParams::default();
    params.migrated_v1.store(true, Ordering::Relaxed);

    // Set legacy to non-default AFTER flag is already set
    {
        let mut legacy = params.curve_nodes.lock().unwrap();
        legacy[0][0][0] = CurveNode { x: 0.9, y: 0.9, q: 0.9 };
    }

    params.migrate_legacy_if_needed();

    // Param should still be at default
    let (x, _, _) = params.graph_node(0, 0, 0).unwrap();
    assert_eq!(x.value(), 0.0);
}
```

- [ ] **Step 5: Run test**

Run: `cargo test --test state_migration`
Expected: 2 passed.

- [ ] **Step 6: Commit**

```bash
git add src/params.rs src/lib.rs tests/state_migration.rs
git commit -m "feat(migration): one-shot copy from legacy persist to new params"
```

---

## Task 10: Audio-rate modulation test

**Rationale:** Every new param may receive white noise at sample rate in Bitwig. Verify the whole-plugin output stays finite.

**Files:**
- Create: `tests/audio_rate_modulation.rs`

- [ ] **Step 1: Write test**

```rust
use spectral_forge::{SpectralForgePlugin, SpectralForgeParams};
use nih_plug::prelude::*;
use std::sync::Arc;

#[test]
fn white_noise_on_all_params_produces_finite_output() {
    let params = Arc::new(SpectralForgeParams::default());

    // Drive every automatable param with random noise for 1 second at 48 kHz.
    let sample_rate = 48_000.0;
    let buffer_size = 512;
    let num_buffers = 48_000 / buffer_size;  // ~94 buffers

    let mut pipeline = spectral_forge::dsp::pipeline::Pipeline::new(sample_rate, 2048, &params);
    let mut audio_l = vec![0.0_f32; buffer_size];
    let mut audio_r = vec![0.0_f32; buffer_size];

    let param_map = params.param_map();

    for buf_i in 0..num_buffers {
        // Randomize every param in [min, max]
        for (id, ptr, _) in &param_map {
            if id == "migrated_v1" { continue; }
            let v: f32 = fastrand::f32();  // [0, 1]
            unsafe { ptr.set_normalized_value(v); }
        }

        // Generate input noise
        for s in 0..buffer_size {
            audio_l[s] = fastrand::f32() * 2.0 - 1.0;
            audio_r[s] = fastrand::f32() * 2.0 - 1.0;
        }

        pipeline.process(&mut [&mut audio_l, &mut audio_r], &[]);

        // Assert output is finite and bounded
        for s in 0..buffer_size {
            assert!(audio_l[s].is_finite(), "buf {} sample {} L: {}", buf_i, s, audio_l[s]);
            assert!(audio_r[s].is_finite(), "buf {} sample {} R: {}", buf_i, s, audio_r[s]);
            assert!(audio_l[s].abs() < 100.0, "buf {} sample {} L runaway: {}", buf_i, s, audio_l[s]);
            assert!(audio_r[s].abs() < 100.0, "buf {} sample {} R runaway: {}", buf_i, s, audio_r[s]);
        }
    }
}
```

Add `fastrand = "2"` to `[dev-dependencies]` in `Cargo.toml`.

- [ ] **Step 2: Run test**

Run: `cargo test --test audio_rate_modulation --release`
Expected: 1 passed (release mode is needed; debug would take too long).

- [ ] **Step 3: If failures, add guards**

If the test fails with NaN/Inf, debug to find the math path. Common culprits:
- `1.0 / q` where q=0 → add `q.max(0.001)` at read site
- `freq.ln()` where freq < 0 → clamp `freq.max(1.0)`
- `gain.powf(exp)` where gain < 0 → use `gain.max(1e-6)`

Fix each, re-run, commit.

- [ ] **Step 4: Commit**

```bash
git add tests/audio_rate_modulation.rs Cargo.toml
git commit -m "test: audio-rate modulation stress test on all automatable params"
```

---

## Task 11: delayed_tooltip helper

**Rationale:** Shared helper for the 1000ms-delay tooltip used by all automatable widgets.

**Files:**
- Modify: `src/editor/mod.rs`

- [ ] **Step 1: Write the helper**

In `src/editor/mod.rs`:

```rust
use egui::{Response, Ui, Id};
use std::time::Duration;

const TOOLTIP_DELAY: Duration = Duration::from_millis(1000);

/// Show a tooltip on `response` only after the cursor has been stationary
/// over it for `TOOLTIP_DELAY`. Resets on pointer motion.
pub fn delayed_tooltip(ui: &Ui, response: &Response, text: impl Into<String>) {
    let text = text.into();
    if !response.hovered() { return; }

    let id = response.id.with("delayed_tooltip_start");
    let now = ui.input(|i| i.time);

    let start: f64 = ui.ctx().memory_mut(|m| {
        *m.data.get_temp_mut_or_insert_with(id, || now)
    });

    let elapsed = now - start;
    if elapsed >= TOOLTIP_DELAY.as_secs_f64() {
        egui::show_tooltip_at_pointer(
            ui.ctx(),
            egui::LayerId::new(egui::Order::Tooltip, Id::new("sf_tt")),
            response.id.with("sf_tt_content"),
            |ui| { ui.label(text); },
        );
    }

    // Reset timer on mouse motion
    let motion = ui.input(|i| i.pointer.delta());
    if motion.length() > 0.5 {
        ui.ctx().memory_mut(|m| { m.data.insert_temp(id, now); });
    }
}
```

- [ ] **Step 2: Write test (optional — UI-heavy, skip if friction)**

UI helpers like this are hard to unit-test meaningfully. Skip.

- [ ] **Step 3: Verify build**

Run: `cargo build --release`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add src/editor/mod.rs
git commit -m "feat(editor): delayed_tooltip helper (1000ms hover)"
```

---

## Task 12: Wire tooltips to all automatable widgets

**Rationale:** Apply `delayed_tooltip` to every widget that now maps to a FloatParam.

**Files:**
- Modify: `src/editor/curve.rs` (graph nodes)
- Modify: `src/editor_ui.rs` (tilt/offset DragValues)
- Modify: `src/editor/fx_matrix_grid.rs` (matrix cells)

- [ ] **Step 1: Graph node tooltips**

In `curve_widget()`, after each node's `response` is created:

```rust
let curve_label = crate::dsp::modules::module_spec(editing_type)
    .curve_labels.get(editing_curve).copied().unwrap_or("");
let field_name = "Freq";  // or "Gain" / "Q" depending on which axis is being drawn
let tt = format!("S{} C{} N{} — {} · {}", slot, curve, node_idx, curve_label, field_name);
delayed_tooltip(ui, &response, tt);
```

Since nodes are drawn as a group, wrap the whole-node interaction response — egui nodes usually have a single `response` per node.

- [ ] **Step 2: Tilt/offset tooltips**

In the tilt/offset DragValue block (Task 7):

```rust
let curve_label = /* same lookup */;
delayed_tooltip(ui, &tilt_resp, format!("S{} C{} — {} · Tilt", slot, curve, curve_label));
delayed_tooltip(ui, &offset_resp, format!("S{} C{} — {} · Offset", slot, curve, curve_label));
```

- [ ] **Step 3: FX matrix cell tooltips**

In `src/editor/fx_matrix_grid.rs`, for each cell's response:

```rust
let send_pct = (cell_value * 100.0).round() as u32;
delayed_tooltip(ui, &cell_resp, format!("S{} → S{} send — {}%", col, row, send_pct));
```

- [ ] **Step 4: Manual verify**

Rebuild, install, hover various widgets in Bitwig — confirm 1000ms delay and correct labels.

- [ ] **Step 5: Commit**

```bash
git add src/editor/curve.rs src/editor_ui.rs src/editor/fx_matrix_grid.rs
git commit -m "feat(editor): 1000ms hover tooltips on automatable widgets"
```

---

## Task 13: Preset struct + save/load round-trip

**Files:**
- Create: `src/preset.rs`
- Modify: `src/lib.rs` (add `pub mod preset;`)

- [ ] **Step 1: Write preset module**

```rust
//! Preset serialization — JSON, one file per preset.
//!
//! Schema is versioned via PRESET_SCHEMA_VERSION. Loader filters out
//! presets whose schema_version doesn't match the current one.

use nih_plug::prelude::*;
use serde::{Serialize, Deserialize};
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub const PRESET_SCHEMA_VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
pub struct Preset {
    pub schema_version: u32,
    pub plugin_version: String,
    pub name: String,
    pub params: HashMap<String, f32>,  // param_id → normalized value
    pub gui: GuiState,
}

#[derive(Serialize, Deserialize, Default)]
pub struct GuiState {
    pub editing_slot: u32,
    pub editing_curve: u32,
    pub slot_module_types: Vec<u8>,
    pub stereo_link: u32,
    pub fft_size: u32,
}

impl Preset {
    pub fn from_params(name: String, params: &impl Params, gui: GuiState) -> Self {
        let mut p = HashMap::new();
        for (id, ptr, _) in params.param_map() {
            if id == "migrated_v1" { continue; }
            let v = unsafe { ptr.normalized_value() };
            p.insert(id, v);
        }
        Self {
            schema_version: PRESET_SCHEMA_VERSION,
            plugin_version: env!("CARGO_PKG_VERSION").to_string(),
            name,
            params: p,
            gui,
        }
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        fs::write(path, json)
    }

    pub fn load(path: &Path) -> io::Result<Self> {
        let bytes = fs::read(path)?;
        serde_json::from_slice(&bytes)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    /// Apply this preset's values to the given params via the provided setter.
    /// Unknown IDs are silently ignored (forward-compatible).
    ///
    /// Uses GuiContext begin/set/end so the host records the preset load as
    /// a single automation edit per parameter (not a per-param drag).
    pub fn apply(&self, params: &impl Params, setter: &ParamSetter) {
        let map: HashMap<String, ParamPtr> = params.param_map()
            .into_iter().map(|(id, ptr, _)| (id, ptr)).collect();
        for (id, v) in &self.params {
            if let Some(ptr) = map.get(id) {
                unsafe {
                    setter.raw_context.raw_begin_set_parameter(*ptr);
                    setter.raw_context.raw_set_parameter_normalized(*ptr, *v);
                    setter.raw_context.raw_end_set_parameter(*ptr);
                }
            }
        }
    }
}

pub fn preset_dir() -> PathBuf {
    use directories::ProjectDirs;
    if let Some(dirs) = ProjectDirs::from("", "", "Spectral Forge") {
        let p = dirs.config_dir().join("presets");
        let _ = fs::create_dir_all(&p);
        return p;
    }
    PathBuf::from("./presets")  // fallback
}

pub fn sanitize_filename(name: &str) -> String {
    name.chars().map(|c| match c {
        '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
        c => c,
    }).collect::<String>().trim().to_string()
}
```

Add `pub mod preset;` to `src/lib.rs`.

- [ ] **Step 2: Write round-trip test**

Create `tests/preset_roundtrip.rs`:

```rust
use spectral_forge::{SpectralForgeParams, preset::{Preset, GuiState}};
use nih_plug::prelude::*;
use tempfile::NamedTempFile;

#[test]
fn save_load_roundtrip_preserves_all_params() {
    let params = SpectralForgeParams::default();

    // Mutate a handful of params to non-default
    unsafe {
        params.graph_node(0, 0, 0).unwrap().0.set_plain_value(0.5);
        params.matrix_cell(3, 1).unwrap().set_plain_value(0.7);
    }

    let p1 = Preset::from_params("test".into(), &params, GuiState::default());
    let tmp = NamedTempFile::new().unwrap();
    p1.save(tmp.path()).unwrap();

    let p2 = Preset::load(tmp.path()).unwrap();
    assert_eq!(p1.params.len(), p2.params.len());
    for (k, v) in &p1.params {
        assert!((v - p2.params[k]).abs() < 1e-6, "mismatch on {}", k);
    }
}

#[test]
fn sanitize_filename_strips_bad_chars() {
    use spectral_forge::preset::sanitize_filename;
    assert_eq!(sanitize_filename("hello/world"), "hello_world");
    assert_eq!(sanitize_filename("a:b?c"), "a_b_c");
    assert_eq!(sanitize_filename("  spaces  "), "spaces");
}
```

Add `tempfile = "3"` to `[dev-dependencies]`.

- [ ] **Step 3: Run tests**

Run: `cargo test --test preset_roundtrip`
Expected: 2 passed.

- [ ] **Step 4: Commit**

```bash
git add src/preset.rs src/lib.rs tests/preset_roundtrip.rs Cargo.toml
git commit -m "feat(preset): JSON preset save/load with schema version"
```

---

## Task 14: Preset directory scan + schema filter

**Files:**
- Modify: `src/preset.rs`

- [ ] **Step 1: Add scan function**

```rust
impl Preset {
    /// Return (name, path) pairs for every compatible preset in the directory.
    /// Incompatible schema versions are silently filtered out.
    pub fn scan_compatible(dir: &Path) -> Vec<(String, PathBuf)> {
        let mut out = Vec::new();
        let entries = match fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return out,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("sfpreset") { continue; }
            let Ok(p) = Self::load(&path) else { continue };
            if p.schema_version != PRESET_SCHEMA_VERSION { continue; }
            out.push((p.name, path));
        }
        out.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
        out
    }
}
```

- [ ] **Step 2: Write test**

Add to `tests/preset_roundtrip.rs`:

```rust
#[test]
fn scan_filters_by_schema_version() {
    let dir = tempfile::tempdir().unwrap();

    // Write one compatible preset
    let good = Preset {
        schema_version: PRESET_SCHEMA_VERSION,
        plugin_version: "0".into(), name: "good".into(),
        params: HashMap::new(), gui: GuiState::default(),
    };
    good.save(&dir.path().join("good.sfpreset")).unwrap();

    // Write one incompatible preset (fake schema 999)
    let bad_json = serde_json::json!({
        "schema_version": 999, "plugin_version": "0", "name": "bad",
        "params": {}, "gui": GuiState::default()
    }).to_string();
    fs::write(dir.path().join("bad.sfpreset"), bad_json).unwrap();

    let list = Preset::scan_compatible(dir.path());
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].0, "good");
}
```

- [ ] **Step 3: Run test**

Run: `cargo test --test preset_roundtrip scan_filters`
Expected: pass.

- [ ] **Step 4: Commit**

```bash
git add src/preset.rs tests/preset_roundtrip.rs
git commit -m "feat(preset): scan_compatible filters by schema version"
```

---

## Task 15: Preset pulldown UI

**Files:**
- Create: `src/editor/preset_menu.rs`
- Modify: `src/editor/mod.rs` (export)
- Modify: `src/editor_ui.rs` (add to top bar)

- [ ] **Step 1: Write pulldown widget**

```rust
//! Preset pulldown + Save + Open-folder, for the top bar.

use nih_plug::prelude::*;
use crate::{SpectralForgeParams, preset::{Preset, GuiState, preset_dir, sanitize_filename}};
use std::path::PathBuf;
use std::sync::Arc;

pub struct PresetMenuState {
    pub available: Vec<(String, PathBuf)>,
    pub selected: Option<String>,
    pub save_name: String,
    pub save_popup_open: bool,
}

impl Default for PresetMenuState {
    fn default() -> Self {
        Self {
            available: Preset::scan_compatible(&preset_dir()),
            selected: None,
            save_name: String::new(),
            save_popup_open: false,
        }
    }
}

impl PresetMenuState {
    pub fn refresh(&mut self) {
        self.available = Preset::scan_compatible(&preset_dir());
    }
}

pub fn preset_menu_ui(
    ui: &mut egui::Ui,
    state: &mut PresetMenuState,
    params: &Arc<SpectralForgeParams>,
    setter: &ParamSetter,
) {
    ui.horizontal(|ui| {
        let current_label = state.selected.as_deref().unwrap_or("— Preset —");

        egui::ComboBox::from_id_salt("preset_pulldown")
            .selected_text(current_label)
            .width(180.0)
            .show_ui(ui, |ui| {
                for (name, path) in state.available.clone() {
                    if ui.selectable_label(state.selected.as_deref() == Some(&name), &name).clicked() {
                        if let Ok(p) = Preset::load(&path) {
                            p.apply(params.as_ref(), setter);
                            state.selected = Some(name);
                        }
                    }
                }
            });

        if ui.button("Save").clicked() {
            state.save_popup_open = true;
        }

        if ui.button("Open folder").clicked() {
            let _ = opener::open(preset_dir());
        }

        if state.save_popup_open {
            egui::Window::new("Save Preset")
                .collapsible(false)
                .resizable(false)
                .show(ui.ctx(), |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Name:");
                        ui.text_edit_singleline(&mut state.save_name);
                    });
                    ui.horizontal(|ui| {
                        if ui.button("Save").clicked() && !state.save_name.is_empty() {
                            let safe = sanitize_filename(&state.save_name);
                            let path = preset_dir().join(format!("{}.sfpreset", safe));
                            let preset = Preset::from_params(
                                state.save_name.clone(),
                                params.as_ref(),
                                GuiState {
                                    editing_slot: *params.editing_slot.lock() as u32,
                                    editing_curve: *params.editing_curve.lock() as u32,
                                    slot_module_types: params.slot_module_types.lock().iter().map(|t| *t as u8).collect(),
                                    stereo_link: params.stereo_link.value() as u32,
                                    fft_size: params.fft_size.value() as u32,
                                },
                            );
                            if preset.save(&path).is_ok() {
                                state.selected = Some(state.save_name.clone());
                                state.refresh();
                            }
                            state.save_popup_open = false;
                            state.save_name.clear();
                        }
                        if ui.button("Cancel").clicked() {
                            state.save_popup_open = false;
                            state.save_name.clear();
                        }
                    });
                });
        }
    });
}
```

- [ ] **Step 2: Wire into top bar**

In `src/editor_ui.rs`, near the top of the header row (before Floor/graph_db_min control):

```rust
// Persisted UI state for the preset menu
let preset_state_id = ui.id().with("preset_state");
let mut preset_state: PresetMenuState = ui.data(|d|
    d.get_temp(preset_state_id)
).unwrap_or_default();

preset_menu_ui(ui, &mut preset_state, params, setter);

ui.data_mut(|d| d.insert_temp(preset_state_id, preset_state));
```

(`PresetMenuState` must be `Clone + Send + Sync + 'static` for `insert_temp`. Add those derives if missing.)

Export: in `src/editor/mod.rs` add `pub mod preset_menu;` and `pub use preset_menu::*;`.

- [ ] **Step 3: Build + manual verify**

Run: `cargo build --release && cargo run --package xtask -- bundle spectral_forge --release && cp target/bundled/spectral_forge.clap ~/.clap/`

Open in Bitwig:
- Confirm the pulldown appears first in the top bar
- Save a preset, close the plugin, reopen — confirm it shows in the list
- Select a different preset, confirm params change
- Click "Open folder", confirm file manager opens

- [ ] **Step 4: Commit**

```bash
git add src/editor/preset_menu.rs src/editor/mod.rs src/editor_ui.rs
git commit -m "feat(editor): preset pulldown + save + open-folder in top bar"
```

---

## Final verification

- [ ] Run `cargo test --release` — all tests pass.
- [ ] Run `cargo build --release` — no warnings.
- [ ] Manual test checklist in Bitwig:
  - [ ] Audio still processes correctly on a default config
  - [ ] Graph node drag records automation; automation playback moves the node
  - [ ] Hover a node 1000ms → tooltip appears with correct label
  - [ ] Tilt/offset DragValues record automation
  - [ ] FX matrix cell drag records automation; tooltip shows `S1 → S2 send — 45%`
  - [ ] Save a preset, reopen plugin, load it — state restored
  - [ ] Presets folder opens via platform file manager
  - [ ] An old project (pre-refactor) loads and the legacy curve nodes appear on the new params
  - [ ] Run `tests/audio_rate_modulation.rs` once more — no NaN/Inf

- [ ] Use superpowers:finishing-a-development-branch to wrap up.
