//! Verifies that the hand-written serialize_fields / deserialize_fields impl
//! in params.rs round-trips persist state faithfully.
//!
//! The `Params` trait methods are `pub` (they're part of the trait), but their
//! returned/accepted type is a `BTreeMap<String, String>` of serde-JSON blobs.
//! We call them directly via the trait to exercise the real code path.

use nih_plug::prelude::Params;
use spectral_forge::params::SpectralForgeParams;

#[test]
fn persist_roundtrip_active_curve() {
    let p1 = SpectralForgeParams::default();
    // Mutate one persist field.
    *p1.active_curve.lock() = 3;

    let serialized = p1.serialize_fields();
    assert!(serialized.contains_key("active_curve"), "active_curve not serialized");

    // Deserialize into a fresh params.
    let p2 = SpectralForgeParams::default();
    p2.deserialize_fields(&serialized);

    assert_eq!(*p2.active_curve.lock(), 3);
}

#[test]
fn persist_roundtrip_editing_slot() {
    let p1 = SpectralForgeParams::default();
    *p1.editing_slot.lock() = 5;

    let serialized = p1.serialize_fields();
    let p2 = SpectralForgeParams::default();
    p2.deserialize_fields(&serialized);

    assert_eq!(*p2.editing_slot.lock(), 5);
}

#[test]
fn persist_roundtrip_all_keys_present() {
    let p = SpectralForgeParams::default();
    let serialized = p.serialize_fields();

    // Every persist key that was in the original #[persist = "..."] list must appear.
    let expected_keys = [
        "editor_state", "curve_nodes", "active_curve", "active_tab",
        "phase_curve_nodes", "freeze_curve_nodes", "freeze_active_curve",
        "editing_slot", "fx_module_types", "slot_module_types", "slot_names",
        "slot_targets", "slot_gain_mode", "slot_future_mode", "slot_punch_mode",
        "slot_rhythm_mode", "slot_geometry_mode", "slot_arp_grid", "slot_curve_nodes",
        "editing_curve", "route_matrix", "fx_module_names",
        "fx_module_targets", "fx_route_matrix", "graph_db_min", "graph_db_max",
        "peak_falloff_ms", "ui_scale", "migrated_v1",
    ];
    for key in &expected_keys {
        assert!(serialized.contains_key(*key), "missing persist key: {}", key);
    }
    assert_eq!(serialized.len(), expected_keys.len(),
        "unexpected extra keys: {:?}", serialized.keys().collect::<Vec<_>>());
}
