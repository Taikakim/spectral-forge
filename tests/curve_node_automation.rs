use spectral_forge::params::SpectralForgeParams;

/// Verify that graph-node params are accessible and have the expected defaults.
///
/// ParamSetter requires a GuiContext which isn't available in unit tests.
/// This exercises the underlying param plumbing: that `graph_node()` returns
/// the correct FloatParam references and their `.value()` reflects defaults.
#[test]
fn setting_graph_node_param_is_reflected_in_value() {
    let params = SpectralForgeParams::default();
    let (x_p, y_p, q_p) = params.graph_node(2, 1, 3).unwrap();

    // Node 3 (a bell) default: x = 0.6, y = 0.0, q = 0.5
    // Check that the params are wired and readable.
    assert!(
        x_p.value() >= 0.0 && x_p.value() <= 1.0,
        "x param value out of [0,1] range: {}",
        x_p.value()
    );
    assert!(
        y_p.value() >= -1.0 && y_p.value() <= 1.0,
        "y param value out of [-1,1] range: {}",
        y_p.value()
    );
    assert!(
        q_p.value() >= 0.0 && q_p.value() <= 1.0,
        "q param value out of [0,1] range: {}",
        q_p.value()
    );

    // y and q neutral defaults
    assert!(
        y_p.value().abs() < 1e-5,
        "y default should be 0.0, got {}",
        y_p.value()
    );
    assert!(
        (q_p.value() - 0.5).abs() < 1e-5,
        "q default should be 0.5, got {}",
        q_p.value()
    );
}
