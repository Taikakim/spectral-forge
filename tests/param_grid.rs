//! Param generation smoke tests.
//!
//! Verifies that `SpectralForgeParams::default().param_map()` includes the
//! full 1341-entry automation grid produced by `build.rs`.
//!
//! Counts (from the plan):
//!   - Graph nodes : 9 slots × 7 curves × 6 nodes × 3 fields (x,y,q) = 1134
//!   - Tilt+offset : 9 slots × 7 curves × 2 kinds                    =  126
//!   - Matrix      : 9 rows × 9 cols                                 =   81
//!   - Total                                                         = 1341

use nih_plug::prelude::Params;
use spectral_forge::params::SpectralForgeParams;

/// Graph-node IDs look like `s{slot}c{curve}n{node}{x|y|q}`, e.g. `s0c0n0x`.
///
/// Exact shape: `s` + digit + `c` + digit + `n` + digit + one of `x`/`y`/`q`.
/// This matcher is strict so it does not collide with legacy IDs that happen to
/// contain an `n` (e.g. `sensitivity`, `stereo_link`, `spectral_contrast_db`,
/// `sc_attack_ms`, `suppression_width`).
fn is_graph_node_id(id: &str) -> bool {
    let b = id.as_bytes();
    if b.len() < 7 {
        return false;
    }
    b[0] == b's'
        && b[1].is_ascii_digit()
        && b[2] == b'c'
        && b[3].is_ascii_digit()
        && b[4] == b'n'
        && b[5].is_ascii_digit()
        && matches!(b[6], b'x' | b'y' | b'q')
        && b.len() == 7
}

/// Generator-emitted tilt/offset IDs look like `s{slot}c{curve}tilt` or
/// `s{slot}c{curve}offset`. The legacy hand-written globals (threshold_tilt,
/// ratio_offset, etc.) also end in these suffixes, so we discriminate on the
/// leading `s` + digit + `c` + digit form.
fn is_generated_tilt_or_offset_id(id: &str) -> bool {
    if !(id.ends_with("tilt") || id.ends_with("offset")) {
        return false;
    }
    let b = id.as_bytes();
    b.len() >= 5
        && b[0] == b's'
        && b[1].is_ascii_digit()
        && b[2] == b'c'
        && b[3].is_ascii_digit()
}

fn is_matrix_id(id: &str) -> bool {
    id.starts_with("mr")
}

// ── Constants cross-check (I2) ────────────────────────────────────────────────
//
// build.rs has its own local copies of NUM_SLOTS / NUM_CURVES / NUM_NODES /
// NUM_MATRIX_ROWS. They must match param_ids::NUM_* or the param_map counts
// above will silently diverge. This test pins the expected values so a change
// in param_ids.rs without a matching change in build.rs fails loudly.

#[test]
fn param_ids_constants_match_expected_dimensions() {
    // If these change, build.rs constants must be updated to match.
    assert_eq!(spectral_forge::param_ids::NUM_SLOTS,       9);
    assert_eq!(spectral_forge::param_ids::NUM_CURVES,      7);
    assert_eq!(spectral_forge::param_ids::NUM_NODES,       6);
    assert_eq!(spectral_forge::param_ids::NUM_MATRIX_ROWS, 9);
    // Derived counts must produce the numbers the param_map test checks.
    let expected_graph  = spectral_forge::param_ids::NUM_SLOTS
        * spectral_forge::param_ids::NUM_CURVES
        * spectral_forge::param_ids::NUM_NODES
        * 3;  // x, y, q
    assert_eq!(expected_graph, 1134);
    let expected_to = spectral_forge::param_ids::NUM_SLOTS
        * spectral_forge::param_ids::NUM_CURVES
        * 2;  // tilt, offset
    assert_eq!(expected_to, 126);
    let expected_matrix = spectral_forge::param_ids::NUM_MATRIX_ROWS
        * spectral_forge::param_ids::NUM_SLOTS;
    assert_eq!(expected_matrix, 81);
}

#[test]
fn param_map_contains_expected_count() {
    let params = SpectralForgeParams::default();
    let map = params.param_map();
    let ids: Vec<&str> = map.iter().map(|(id, _, _)| id.as_str()).collect();

    let graph_count  = ids.iter().filter(|id| is_graph_node_id(id)).count();
    let to_count     = ids.iter().filter(|id| is_generated_tilt_or_offset_id(id)).count();
    let matrix_count = ids.iter().filter(|id| is_matrix_id(id)).count();

    // Graph nodes: 9 × 7 × 6 × 3 = 1134
    assert_eq!(graph_count, 1134, "graph-node ID count mismatch");
    // Tilt+offset: 9 × 7 × 2 = 126 (generator-emitted only; legacy globals excluded)
    assert_eq!(to_count, 126, "tilt/offset ID count mismatch");
    // Matrix: 9 × 9 = 81
    assert_eq!(matrix_count, 81, "matrix ID count mismatch");
}

#[test]
fn specific_ids_are_present() {
    let params = SpectralForgeParams::default();
    let map = params.param_map();
    let ids: std::collections::HashSet<&str> =
        map.iter().map(|(id, _, _)| id.as_str()).collect();

    assert!(ids.contains("s0c0n0x"), "missing s0c0n0x");
    assert!(ids.contains("s8c6n5q"), "missing s8c6n5q");
    assert!(ids.contains("s4c3tilt"), "missing s4c3tilt");
    assert!(ids.contains("s4c3offset"), "missing s4c3offset");
    assert!(ids.contains("mr8c0"), "missing mr8c0");
}

// ── Matrix default-value tests (Task 8) ──────────────────────────────────────

#[test]
fn matrix_serial_chain_defaults() {
    // build.rs encodes a full linear chain using the rule: default = 1.0 when col + 1 == r.
    // This means: mr1c0=1.0 (0→1), mr2c1=1.0 (1→2), mr3c2=1.0 (2→3), ..., mr8c7=1.0 (7→Master).
    let p = SpectralForgeParams::default();

    let v_1_0 = p.matrix_cell(1, 0).expect("mr1c0 should exist").value();
    assert!(
        (v_1_0 - 1.0).abs() < 1e-6,
        "mr1c0 (slot 0→1 send) should default to 1.0, got {v_1_0}"
    );

    let v_2_1 = p.matrix_cell(2, 1).expect("mr2c1 should exist").value();
    assert!(
        (v_2_1 - 1.0).abs() < 1e-6,
        "mr2c1 (slot 1→2 send) should default to 1.0, got {v_2_1}"
    );

    // mr8c7: slot 7 → Master (slot 8), the last link in the full chain.
    let v_8_7 = p.matrix_cell(8, 7).expect("mr8c7 should exist").value();
    assert!(
        (v_8_7 - 1.0).abs() < 1e-6,
        "mr8c7 (slot 7→Master send) should default to 1.0, got {v_8_7}"
    );

    // A non-serial cell should be zero.
    let v_8_2 = p.matrix_cell(8, 2).expect("mr8c2 should exist").value();
    assert!(
        v_8_2.abs() < 1e-6,
        "mr8c2 (slot 2→Master direct, not in serial chain) should default to 0.0, got {v_8_2}"
    );
}

#[test]
fn matrix_self_send_cell_exists_and_is_zero() {
    // Diagonal cells exist in params but the pipeline skips them (self-feedback guard).
    // Verify the cell is accessible and defaults to 0.0.
    let p = SpectralForgeParams::default();
    let v = p.matrix_cell(0, 0).expect("mr0c0 should exist").value();
    assert!(
        v.abs() < 1e-6,
        "mr0c0 (self-send) should default to 0.0, got {v}"
    );
}
