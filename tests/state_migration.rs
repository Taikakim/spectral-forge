use spectral_forge::params::SpectralForgeParams;
use std::sync::atomic::Ordering;

// ── migrate_legacy_if_needed tests ───────────────────────────────────────────
//
// NOTE: nih-plug does not expose a public API for setting FloatParam atomic
// values from outside the plugin wrapper (ParamMut::set_plain_value is
// pub(crate) only). migrate_legacy_if_needed() uses `smoother.reset(v)` so
// that `param.smoothed.next()` returns the migrated value — which is what the
// pipeline reads via `p.smoothed.next()` for matrix cells.
//
// `param.value()` continues to reflect the FloatParam default until the host
// calls deserialize_object (which goes through nih-plug internals). For
// proper value() correctness on project load, see Plugin::filter_state in lib.rs.

#[test]
fn migration_flag_is_set_after_run() {
    let params = SpectralForgeParams::default();
    assert!(!params.migrated_v1.load(Ordering::Relaxed), "should start false");
    params.migrate_legacy_if_needed();
    assert!(params.migrated_v1.load(Ordering::Relaxed), "should be true after migration");
}

#[test]
fn migration_does_not_rerun() {
    let params = SpectralForgeParams::default();
    // Pre-set flag to true — migration should be a no-op.
    params.migrated_v1.store(true, Ordering::Relaxed);

    // Write a non-default value into legacy store AFTER flag is set.
    {
        use spectral_forge::editor::curve::CurveNode;
        let mut nodes = params.slot_curve_nodes.lock();
        nodes[0][0][0] = CurveNode { x: 0.9, y: 0.9, q: 0.9 };
    }

    params.migrate_legacy_if_needed();

    // The graph_node smoother for (0, 0, 0) should still be at its default (x=0.0 for node 0),
    // NOT 0.9, because migration was skipped.
    let (x_p, _, _) = params.graph_node(0, 0, 0).unwrap();
    // The smoother was never reset to 0.9 so next() returns the initial target (default: 0.0).
    let x_val = x_p.smoothed.next();
    assert!(
        (x_val - 0.0).abs() < 1e-6,
        "x.smoothed.next() should be default 0.0 when migration was skipped, got {x_val}"
    );
}

#[test]
fn legacy_curve_nodes_migrate_smoother() {
    let params = SpectralForgeParams::default();

    // Write non-default value into legacy persist store.
    {
        use spectral_forge::editor::curve::CurveNode;
        let mut nodes = params.slot_curve_nodes.lock();
        nodes[2][1][3] = CurveNode { x: 0.7, y: 0.4, q: 0.9 };
    }

    params.migrate_legacy_if_needed();

    // The smoother should reflect the migrated value.
    let (x_p, y_p, q_p) = params.graph_node(2, 1, 3).unwrap();
    let x = x_p.smoothed.next();
    let y = y_p.smoothed.next();
    let q = q_p.smoothed.next();
    assert!((x - 0.7).abs() < 1e-6, "x smoother should be 0.7 after migration, got {x}");
    assert!((y - 0.4).abs() < 1e-6, "y smoother should be 0.4 after migration, got {y}");
    assert!((q - 0.9).abs() < 1e-6, "q smoother should be 0.9 after migration, got {q}");
    assert!(params.migrated_v1.load(Ordering::Relaxed));
}

#[test]
fn legacy_matrix_migrates_smoother() {
    let params = SpectralForgeParams::default();

    // Default route_matrix.send[0][1] = 1.0 (slot 0 → slot 1, serial chain).
    // After migration, matrix_cell(dst=1, src=0).smoothed.next() should be 1.0.
    params.migrate_legacy_if_needed();

    let cell = params.matrix_cell(1, 0).unwrap();
    let v = cell.smoothed.next();
    assert!(
        (v - 1.0).abs() < 1e-4,
        "matrix_cell(1, 0) smoother should be 1.0 (default serial chain), got {v}"
    );
}

// NOTE: legacy_tilt_migrates_smoother was removed — slot_curve_meta was deleted in Task 2.
// Tilt/offset/curvature are now stored only in the generated FloatParams; no migration path
// from a legacy persist field is needed. Preset back-compat is waived for early dev.

// ── filter_state tests ───────────────────────────────────────────────────────
//
// Plugin::filter_state() is called by nih-plug before state deserialization.
// It injects param values into state.params from legacy persist fields when
// migrated_v1 is absent. Once injected, nih-plug's internal set_plain_value
// path sets param.value() correctly.
//
// Call as: `use nih_plug::prelude::Plugin; SpectralForge::filter_state(&mut state);`

#[test]
fn filter_state_injects_graph_nodes_from_old_state() {
    use nih_plug::prelude::{Plugin, PluginState};
    use nih_plug::wrapper::state::ParamValue;
    use spectral_forge::SpectralForge;
    use spectral_forge::editor::curve::CurveNode;
    use spectral_forge::param_ids::{NUM_SLOTS, NUM_CURVES, NUM_NODES};

    // Build a legacy PluginState: has slot_curve_nodes but no migrated_v1 key.
    let mut nodes = [[[CurveNode::default(); NUM_NODES]; NUM_CURVES]; NUM_SLOTS];
    nodes[2][1][3] = CurveNode { x: 0.7, y: 0.4, q: 0.9 };

    let mut state = PluginState {
        version: String::new(),
        params: Default::default(),
        fields: {
            let mut m = std::collections::BTreeMap::new();
            m.insert(
                "slot_curve_nodes".to_string(),
                serde_json::to_string(&nodes).unwrap(),
            );
            m
        },
    };
    // No "migrated_v1" key → treated as old state.

    SpectralForge::filter_state(&mut state);

    // filter_state should have injected the node x/y/q values into state.params.
    let x_id = spectral_forge::param_ids::graph_node_id(2, 1, 3, 'x');
    let y_id = spectral_forge::param_ids::graph_node_id(2, 1, 3, 'y');
    let q_id = spectral_forge::param_ids::graph_node_id(2, 1, 3, 'q');

    match state.params.get(&x_id) {
        Some(ParamValue::F32(v)) =>
            assert!((v - 0.7).abs() < 1e-6, "x should be 0.7, got {v}"),
        other => panic!("expected F32(0.7) for {x_id}, got {other:?}"),
    }
    match state.params.get(&y_id) {
        Some(ParamValue::F32(v)) =>
            assert!((v - 0.4).abs() < 1e-6, "y should be 0.4, got {v}"),
        other => panic!("expected F32(0.4) for {y_id}, got {other:?}"),
    }
    match state.params.get(&q_id) {
        Some(ParamValue::F32(v)) =>
            assert!((v - 0.9).abs() < 1e-6, "q should be 0.9, got {v}"),
        other => panic!("expected F32(0.9) for {q_id}, got {other:?}"),
    }

    // migrated_v1 must be written into fields so next save persists the flag.
    assert!(
        state.fields.contains_key("migrated_v1"),
        "filter_state should insert migrated_v1 into fields"
    );
}

#[test]
fn filter_state_skips_when_already_migrated() {
    use nih_plug::prelude::{Plugin, PluginState};
    use spectral_forge::SpectralForge;
    use spectral_forge::editor::curve::CurveNode;
    use spectral_forge::param_ids::{NUM_SLOTS, NUM_CURVES, NUM_NODES};

    let mut nodes = [[[CurveNode::default(); NUM_NODES]; NUM_CURVES]; NUM_SLOTS];
    nodes[0][0][0] = CurveNode { x: 0.9, y: 0.9, q: 0.9 };

    let mut state = PluginState {
        version: String::new(),
        params: Default::default(),
        fields: {
            let mut m = std::collections::BTreeMap::new();
            m.insert("slot_curve_nodes".to_string(), serde_json::to_string(&nodes).unwrap());
            // Already migrated — should skip injection.
            m.insert("migrated_v1".to_string(), "true".to_string());
            m
        },
    };

    SpectralForge::filter_state(&mut state);

    // No node params should have been injected (already migrated).
    let x_id = spectral_forge::param_ids::graph_node_id(0, 0, 0, 'x');
    assert!(
        !state.params.contains_key(&x_id),
        "filter_state should not inject params when migrated_v1 is present"
    );
}
