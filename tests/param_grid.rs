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
