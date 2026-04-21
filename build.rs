//! Generates `params_gen.rs` — a top-level `pub struct GeneratedParams` with
//! 1341 `FloatParam` fields for graph nodes, tilt/offset, and matrix sends,
//! plus a `Default` impl and an `extend_param_map` method.
//!
//! The generated file is `include!`d at the top of `src/params.rs`. The main
//! `SpectralForgeParams` struct holds a `generated: GeneratedParams` field and
//! forwards `param_map()` entries through `extend_param_map`.
//!
//! Keep IDs in sync with src/param_ids.rs — any formatting change here
//! that diverges from param_ids.rs breaks saved automation.
//!
//! NOTE: `NUM_MATRIX_ROWS = 9` here matches `src/param_ids.rs` (real slots only).
//! The DSP-layer `dsp::modules::MAX_MATRIX_ROWS = 13` includes T/S Split virtual
//! rows and is intentionally different; exposing virtual rows as automation
//! targets is out of scope for this generator.

use std::env;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

const NUM_SLOTS: usize = 9;
const NUM_CURVES: usize = 7;
const NUM_NODES: usize = 6;
const NUM_MATRIX_ROWS: usize = 9;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/param_ids.rs");

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let out_path = out_dir.join("params_gen.rs");
    let mut f = File::create(&out_path).expect("create params_gen.rs");

    writeln!(f, "// AUTO-GENERATED — do not edit. See build.rs.").unwrap();
    writeln!(f, "//").unwrap();
    writeln!(
        f,
        "// This file is `include!`d at the top of src/params.rs. It defines"
    )
    .unwrap();
    writeln!(
        f,
        "// `GeneratedParams` (1341 FloatParams), its Default, and a helper"
    )
    .unwrap();
    writeln!(
        f,
        "// method that extends the plugin's `param_map()` vector with"
    )
    .unwrap();
    writeln!(f, "// (id, ptr, group) tuples for every generated field.").unwrap();
    writeln!(f).unwrap();

    // ─── struct GeneratedParams { ... } ────────────────────────────────────
    writeln!(f, "pub struct GeneratedParams {{").unwrap();
    emit_graph_node_fields(&mut f);
    emit_tilt_offset_fields(&mut f);
    emit_matrix_fields(&mut f);
    writeln!(f, "}}").unwrap();
    writeln!(f).unwrap();

    // ─── impl Default for GeneratedParams ──────────────────────────────────
    writeln!(f, "impl Default for GeneratedParams {{").unwrap();
    writeln!(f, "    fn default() -> Self {{").unwrap();
    writeln!(f, "        Self {{").unwrap();
    emit_graph_node_inits(&mut f);
    emit_tilt_offset_inits(&mut f);
    emit_matrix_inits(&mut f);
    writeln!(f, "        }}").unwrap();
    writeln!(f, "    }}").unwrap();
    writeln!(f, "}}").unwrap();
    writeln!(f).unwrap();

    // ─── impl GeneratedParams { fn extend_param_map(...) } ─────────────────
    writeln!(f, "impl GeneratedParams {{").unwrap();
    writeln!(
        f,
        "    /// Push one `(id, ParamPtr, group)` tuple per generated field into `out`."
    )
    .unwrap();
    writeln!(
        f,
        "    pub fn extend_param_map(&self, out: &mut Vec<(String, ParamPtr, String)>) {{"
    )
    .unwrap();
    emit_graph_node_map_entries(&mut f);
    emit_tilt_offset_map_entries(&mut f);
    emit_matrix_map_entries(&mut f);
    writeln!(f, "    }}").unwrap();
    writeln!(f, "}}").unwrap();
}

// ── Field declarations (bare, no initializers) ──────────────────────────────

fn emit_graph_node_fields(f: &mut File) {
    for s in 0..NUM_SLOTS {
        for c in 0..NUM_CURVES {
            for n in 0..NUM_NODES {
                for field in ['x', 'y', 'q'] {
                    let rust_name = format!("s{}c{}n{}_{}", s, c, n, field);
                    writeln!(f, "    pub {rust_name}: FloatParam,").unwrap();
                }
            }
        }
    }
}

fn emit_tilt_offset_fields(f: &mut File) {
    for s in 0..NUM_SLOTS {
        for c in 0..NUM_CURVES {
            for kind in ["tilt", "offset"] {
                let rust_name = format!("s{}c{}_{}", s, c, kind);
                writeln!(f, "    pub {rust_name}: FloatParam,").unwrap();
            }
        }
    }
}

fn emit_matrix_fields(f: &mut File) {
    for r in 0..NUM_MATRIX_ROWS {
        for col in 0..NUM_SLOTS {
            let rust_name = format!("mr{}c{}", r, col);
            writeln!(f, "    pub {rust_name}: FloatParam,").unwrap();
        }
    }
}

// ── Default initializers ────────────────────────────────────────────────────

fn emit_graph_node_inits(f: &mut File) {
    for s in 0..NUM_SLOTS {
        for c in 0..NUM_CURVES {
            for n in 0..NUM_NODES {
                for field in ['x', 'y', 'q'] {
                    let id = format!("s{}c{}n{}{}", s, c, n, field);
                    let rust_name = format!("s{}c{}n{}_{}", s, c, n, field);
                    let (default, min, max) = graph_node_defaults(n, field);
                    writeln!(
                        f,
                        "            {rust_name}: FloatParam::new(\"{id}\", {default}f32, \
                         FloatRange::Linear {{ min: {min}f32, max: {max}f32 }})\
                         .with_smoother(SmoothingStyle::Linear(1.0)),"
                    )
                    .unwrap();
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
                0 => 0.0,
                1 => 0.2,
                2 => 0.4,
                3 => 0.6,
                4 => 0.8,
                5 => 1.0,
                _ => 0.5,
            };
            (default, 0.0, 1.0)
        }
        'y' => (0.0, -1.0, 1.0),
        'q' => (0.5, 0.0, 1.0),
        _ => unreachable!(),
    }
}

fn emit_tilt_offset_inits(f: &mut File) {
    for s in 0..NUM_SLOTS {
        for c in 0..NUM_CURVES {
            for kind in ["tilt", "offset"] {
                let id = format!("s{}c{}{}", s, c, kind);
                let rust_name = format!("s{}c{}_{}", s, c, kind);
                writeln!(
                    f,
                    "            {rust_name}: FloatParam::new(\"{id}\", 0.0f32, \
                     FloatRange::Linear {{ min: -1.0f32, max: 1.0f32 }})\
                     .with_smoother(SmoothingStyle::Linear(2.0)),"
                )
                .unwrap();
            }
        }
    }
}

fn emit_matrix_inits(f: &mut File) {
    for r in 0..NUM_MATRIX_ROWS {
        for col in 0..NUM_SLOTS {
            let id = format!("mr{}c{}", r, col);
            let rust_name = format!("mr{}c{}", r, col);
            // Default: serial chain. mr1c0=1 (slot 0 → row 1), mr2c1=1, etc.
            // Matches RouteMatrix::default() serial wiring 0→1→2→…
            let default: f32 = if col + 1 == r { 1.0 } else { 0.0 };
            writeln!(
                f,
                "            {rust_name}: FloatParam::new(\"{id}\", {default}f32, \
                 FloatRange::Linear {{ min: 0.0f32, max: 1.0f32 }})\
                 .with_smoother(SmoothingStyle::Linear(2.0)),"
            )
            .unwrap();
        }
    }
}

// ── extend_param_map body ───────────────────────────────────────────────────

fn emit_graph_node_map_entries(f: &mut File) {
    for s in 0..NUM_SLOTS {
        for c in 0..NUM_CURVES {
            for n in 0..NUM_NODES {
                for field in ['x', 'y', 'q'] {
                    let id = format!("s{}c{}n{}{}", s, c, n, field);
                    let rust_name = format!("s{}c{}n{}_{}", s, c, n, field);
                    writeln!(
                        f,
                        "        out.push(({id:?}.to_string(), self.{rust_name}.as_ptr(), String::new()));"
                    )
                    .unwrap();
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
                writeln!(
                    f,
                    "        out.push(({id:?}.to_string(), self.{rust_name}.as_ptr(), String::new()));"
                )
                .unwrap();
            }
        }
    }
}

fn emit_matrix_map_entries(f: &mut File) {
    for r in 0..NUM_MATRIX_ROWS {
        for col in 0..NUM_SLOTS {
            let id = format!("mr{}c{}", r, col);
            let rust_name = format!("mr{}c{}", r, col);
            writeln!(
                f,
                "        out.push(({id:?}.to_string(), self.{rust_name}.as_ptr(), String::new()));"
            )
            .unwrap();
        }
    }
}
