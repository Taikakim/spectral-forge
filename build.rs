//! Generates `params_gen.rs` — a top-level `pub struct GeneratedParams` with
//! 1404 `FloatParam` fields for graph nodes, tilt/offset, curvature, and matrix
//! sends, plus a `Default` impl and an `extend_param_map` method.
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
        "// `GeneratedParams` (1404 FloatParams), its Default, and a helper"
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
    emit_curvature_fields(&mut f);
    emit_matrix_fields(&mut f);
    emit_past_scalar_fields(&mut f);
    writeln!(f, "}}").unwrap();
    writeln!(f).unwrap();

    // ─── impl Default for GeneratedParams ──────────────────────────────────
    writeln!(f, "impl Default for GeneratedParams {{").unwrap();
    writeln!(f, "    fn default() -> Self {{").unwrap();
    writeln!(f, "        Self {{").unwrap();
    emit_graph_node_inits(&mut f);
    emit_tilt_offset_inits(&mut f);
    emit_curvature_inits(&mut f);
    emit_matrix_inits(&mut f);
    emit_past_scalar_inits(&mut f);
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
    emit_curvature_map_entries(&mut f);
    emit_matrix_map_entries(&mut f);
    emit_past_scalar_map_entries(&mut f);
    writeln!(f, "    }}").unwrap();
    writeln!(f, "}}").unwrap();

    // ─── Dispatch macros for typed accessors ───────────────────────────────
    writeln!(f).unwrap();
    emit_graph_node_dispatch(&mut f);
    writeln!(f).unwrap();
    emit_tilt_dispatch(&mut f);
    writeln!(f).unwrap();
    emit_offset_dispatch(&mut f);
    writeln!(f).unwrap();
    emit_curvature_dispatch(&mut f);
    writeln!(f).unwrap();
    emit_matrix_dispatch(&mut f);
    writeln!(f).unwrap();
    emit_past_scalar_dispatch(&mut f);
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
                         .with_smoother(SmoothingStyle::Linear(1.0))\
                         .hide_in_generic_ui(),"
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
                     .with_smoother(SmoothingStyle::Linear(2.0))\
                     .hide_in_generic_ui(),"
                )
                .unwrap();
            }
        }
    }
}

fn emit_matrix_inits(f: &mut File) {
    // Param ID convention: mr{row}c{col}  where  row = DESTINATION slot,  col = SOURCE slot.
    // This is the TRANSPOSE of RouteMatrix.send[src][dst] (first index = source).
    // When building RouteMatrix from params: send[col][r] = mr{r}c{col}.value()
    // Default: serial chain. mr1c0=1 (slot 0 → row 1 = slot 1), mr2c1=1, etc.
    for r in 0..NUM_MATRIX_ROWS {
        for col in 0..NUM_SLOTS {
            let id = format!("mr{}c{}", r, col);
            let rust_name = format!("mr{}c{}", r, col);
            let default: f32 = if col + 1 == r { 1.0 } else { 0.0 };
            writeln!(
                f,
                "            {rust_name}: FloatParam::new(\"{id}\", {default}f32, \
                 FloatRange::Linear {{ min: 0.0f32, max: 1.0f32 }})\
                 .with_smoother(SmoothingStyle::Linear(2.0))\
                 .hide_in_generic_ui(),"
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

// ── Dispatch macros for typed accessors ────────────────────────────────────

fn emit_graph_node_dispatch(f: &mut File) {
    writeln!(f, "macro_rules! graph_node_dispatch {{").unwrap();
    writeln!(f, "    ($self:expr, $s:expr, $c:expr, $n:expr) => {{").unwrap();
    writeln!(f, "        match ($s, $c, $n) {{").unwrap();
    for s in 0..NUM_SLOTS {
        for c in 0..NUM_CURVES {
            for n in 0..NUM_NODES {
                writeln!(
                    f,
                    "            ({s}, {c}, {n}) => (\
                     &$self.generated.s{s}c{c}n{n}_x, \
                     &$self.generated.s{s}c{c}n{n}_y, \
                     &$self.generated.s{s}c{c}n{n}_q),"
                )
                .unwrap();
            }
        }
    }
    writeln!(f, "            _ => unreachable!(),").unwrap();
    writeln!(f, "        }}").unwrap();
    writeln!(f, "    }};").unwrap();
    writeln!(f, "}}").unwrap();
}

fn emit_tilt_dispatch(f: &mut File) {
    writeln!(f, "macro_rules! tilt_dispatch {{").unwrap();
    writeln!(f, "    ($self:expr, $s:expr, $c:expr) => {{").unwrap();
    writeln!(f, "        match ($s, $c) {{").unwrap();
    for s in 0..NUM_SLOTS {
        for c in 0..NUM_CURVES {
            writeln!(f, "            ({s}, {c}) => &$self.generated.s{s}c{c}_tilt,").unwrap();
        }
    }
    writeln!(f, "            _ => unreachable!(),").unwrap();
    writeln!(f, "        }}").unwrap();
    writeln!(f, "    }};").unwrap();
    writeln!(f, "}}").unwrap();
}

fn emit_offset_dispatch(f: &mut File) {
    writeln!(f, "macro_rules! offset_dispatch {{").unwrap();
    writeln!(f, "    ($self:expr, $s:expr, $c:expr) => {{").unwrap();
    writeln!(f, "        match ($s, $c) {{").unwrap();
    for s in 0..NUM_SLOTS {
        for c in 0..NUM_CURVES {
            writeln!(f, "            ({s}, {c}) => &$self.generated.s{s}c{c}_offset,").unwrap();
        }
    }
    writeln!(f, "            _ => unreachable!(),").unwrap();
    writeln!(f, "        }}").unwrap();
    writeln!(f, "    }};").unwrap();
    writeln!(f, "}}").unwrap();
}

fn emit_matrix_dispatch(f: &mut File) {
    writeln!(f, "macro_rules! matrix_dispatch {{").unwrap();
    writeln!(f, "    ($self:expr, $r:expr, $col:expr) => {{").unwrap();
    writeln!(f, "        match ($r, $col) {{").unwrap();
    for r in 0..NUM_MATRIX_ROWS {
        for col in 0..NUM_SLOTS {
            writeln!(f, "            ({r}, {col}) => &$self.generated.mr{r}c{col},").unwrap();
        }
    }
    writeln!(f, "            _ => unreachable!(),").unwrap();
    writeln!(f, "        }}").unwrap();
    writeln!(f, "    }};").unwrap();
    writeln!(f, "}}").unwrap();
}

fn emit_curvature_fields(f: &mut File) {
    for s in 0..NUM_SLOTS {
        for c in 0..NUM_CURVES {
            let rust_name = format!("s{}c{}_curv", s, c);
            writeln!(f, "    pub {rust_name}: FloatParam,").unwrap();
        }
    }
}

fn emit_curvature_inits(f: &mut File) {
    for s in 0..NUM_SLOTS {
        for c in 0..NUM_CURVES {
            let id        = format!("s{}c{}curv", s, c);
            let rust_name = format!("s{}c{}_curv", s, c);
            writeln!(
                f,
                "            {rust_name}: FloatParam::new(\"{id}\", 0.0f32, \
                 FloatRange::Linear {{ min: 0.0f32, max: 1.0f32 }})\
                 .with_smoother(SmoothingStyle::Linear(2.0))\
                 .hide_in_generic_ui(),"
            ).unwrap();
        }
    }
}

fn emit_curvature_map_entries(f: &mut File) {
    for s in 0..NUM_SLOTS {
        for c in 0..NUM_CURVES {
            let id        = format!("s{}c{}curv", s, c);
            let rust_name = format!("s{}c{}_curv", s, c);
            writeln!(
                f,
                "        out.push(({id:?}.to_string(), self.{rust_name}.as_ptr(), String::new()));"
            ).unwrap();
        }
    }
}

fn emit_curvature_dispatch(f: &mut File) {
    writeln!(f, "macro_rules! curv_dispatch {{").unwrap();
    writeln!(f, "    ($self:expr, $s:expr, $c:expr) => {{").unwrap();
    writeln!(f, "        match ($s, $c) {{").unwrap();
    for s in 0..NUM_SLOTS {
        for c in 0..NUM_CURVES {
            writeln!(f, "            ({s}, {c}) => &$self.generated.s{s}c{c}_curv,").unwrap();
        }
    }
    writeln!(f, "            _ => unreachable!(),").unwrap();
    writeln!(f, "        }}").unwrap();
    writeln!(f, "    }};").unwrap();
    writeln!(f, "}}").unwrap();
}

// ── Past UX Overhaul: per-slot scalar params ────────────────────────────────
//
// Five new per-slot params for the Past module (5 × 9 = 45 fields):
//   - past_floor_hz        Skewed 20..2000 Hz, default 230
//   - past_reverse_window_s Linear 0.05..30 s, default 1.0
//   - past_stretch_rate    Skewed 0.05..4.0 ×, default 1.0
//   - past_stretch_dither  Linear 0..1, default 0.0
//   - past_soft_clip       Bool, default true
//
// See docs/superpowers/specs/2026-05-04-past-module-ux-design.md §2 + §3.

fn emit_past_scalar_fields(f: &mut File) {
    for s in 0..NUM_SLOTS {
        writeln!(f, "    pub s{s}_past_floor_hz:        FloatParam,").unwrap();
        writeln!(f, "    pub s{s}_past_reverse_window_s: FloatParam,").unwrap();
        writeln!(f, "    pub s{s}_past_stretch_rate:    FloatParam,").unwrap();
        writeln!(f, "    pub s{s}_past_stretch_dither:  FloatParam,").unwrap();
        writeln!(f, "    pub s{s}_past_soft_clip:       BoolParam,").unwrap();
    }
}

fn emit_past_scalar_inits(f: &mut File) {
    for s in 0..NUM_SLOTS {
        writeln!(
            f,
            "            s{s}_past_floor_hz: FloatParam::new(\"s{s}past_floor_hz\", 230.0f32, \
             FloatRange::Skewed {{ min: 20.0f32, max: 2000.0f32, factor: FloatRange::skew_factor(-2.0) }})\
             .with_smoother(SmoothingStyle::Logarithmic(50.0))\
             .with_unit(\" Hz\")\
             .hide_in_generic_ui(),"
        ).unwrap();
        writeln!(
            f,
            "            s{s}_past_reverse_window_s: FloatParam::new(\"s{s}past_reverse_window_s\", 1.0f32, \
             FloatRange::Linear {{ min: 0.05f32, max: 30.0f32 }})\
             .with_smoother(SmoothingStyle::Linear(50.0))\
             .with_unit(\" s\")\
             .hide_in_generic_ui(),"
        ).unwrap();
        writeln!(
            f,
            "            s{s}_past_stretch_rate: FloatParam::new(\"s{s}past_stretch_rate\", 1.0f32, \
             FloatRange::SymmetricalSkewed {{ min: 0.05f32, max: 4.0f32, \
             factor: FloatRange::skew_factor(-1.0), center: 1.0f32 }})\
             .with_smoother(SmoothingStyle::Logarithmic(50.0))\
             .with_unit(\"\u{00d7}\")\
             .hide_in_generic_ui(),"
        ).unwrap();
        // Dither: stored as 0..100 with unit \"%\" so the host's automation lane
        // displays \"50 %\" rather than \"0.50 %\". DSP rescales /100 → [0, 1].
        writeln!(
            f,
            "            s{s}_past_stretch_dither: FloatParam::new(\"s{s}past_stretch_dither\", 0.0f32, \
             FloatRange::Linear {{ min: 0.0f32, max: 100.0f32 }})\
             .with_smoother(SmoothingStyle::Linear(20.0))\
             .with_unit(\" %\")\
             .hide_in_generic_ui(),"
        ).unwrap();
        writeln!(
            f,
            "            s{s}_past_soft_clip: BoolParam::new(\"s{s}past_soft_clip\", true)\
             .hide_in_generic_ui(),"
        ).unwrap();
    }
}

fn emit_past_scalar_map_entries(f: &mut File) {
    for s in 0..NUM_SLOTS {
        for suffix in ["floor_hz", "reverse_window_s", "stretch_rate", "stretch_dither", "soft_clip"] {
            let id = format!("s{s}past_{suffix}");
            let rust_name = format!("s{s}_past_{suffix}");
            writeln!(
                f,
                "        out.push(({id:?}.to_string(), self.{rust_name}.as_ptr(), String::new()));"
            ).unwrap();
        }
    }
}

fn emit_past_scalar_dispatch(f: &mut File) {
    for suffix in ["floor_hz", "reverse_window_s", "stretch_rate", "stretch_dither", "soft_clip"] {
        writeln!(f, "macro_rules! past_{suffix}_dispatch {{").unwrap();
        writeln!(f, "    ($self:expr, $s:expr) => {{").unwrap();
        writeln!(f, "        match $s {{").unwrap();
        for s in 0..NUM_SLOTS {
            writeln!(f, "            {s} => &$self.generated.s{s}_past_{suffix},").unwrap();
        }
        writeln!(f, "            _ => unreachable!(),").unwrap();
        writeln!(f, "        }}").unwrap();
        writeln!(f, "    }};").unwrap();
        writeln!(f, "}}").unwrap();
        writeln!(f).unwrap();
    }
}
