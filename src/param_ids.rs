//! Centralized parameter ID formatting. Single source of truth for both
//! build.rs code generation and runtime param lookup.
//!
//! IDs are STABLE FOREVER — changing any formatting here will break
//! saved automation lanes in user projects.

pub const NUM_SLOTS: usize = 9;
pub const NUM_CURVES: usize = 7;
pub const NUM_NODES: usize = 6;

/// Number of rows in the automation-exposed matrix grid.
/// NOTE: This is 9 (real slots only). The DSP-layer
/// `dsp::modules::MAX_MATRIX_ROWS` = 13 includes T/S Split virtual rows.
/// These are intentionally different; exposing virtual rows as automation
/// targets is a separate design decision not in this plan's scope.
pub const NUM_MATRIX_ROWS: usize = 9;

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
        assert_eq!(NUM_MATRIX_ROWS * NUM_SLOTS, 81);
    }
}
