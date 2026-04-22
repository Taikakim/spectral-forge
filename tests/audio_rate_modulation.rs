/// Tests that guard against NaN/Inf bugs caused by automation.
///
/// These tests exercise the param map that the host uses to automate the plugin
/// in Bitwig. If any param's default normalized value is non-finite or outside
/// [0.0, 1.0], Bitwig's automation engine may produce nonsensical values which
/// then propagate into the DSP as NaN or Inf.
///
/// The `unmodulated_normalized_value` is used (rather than `default_normalized_value`)
/// so that both the default and the current value at construction time are validated.
use spectral_forge::params::SpectralForgeParams;
use nih_plug::prelude::Params;

#[test]
fn all_param_defaults_are_finite_and_normalized() {
    let params = SpectralForgeParams::default();
    let map = params.param_map();

    // Must have discovered at least the known hand-written globals and the
    // 1341 generated entries — a sanity check that the param map is complete.
    assert!(
        map.len() > 1300,
        "param_map() returned only {} entries — expected 1374+",
        map.len()
    );

    for (id, ptr, _group) in &map {
        // SAFETY: `params` is alive for the duration of this test, so the
        // `ParamPtr`s returned by `param_map()` are valid.
        let default_v = unsafe { ptr.default_normalized_value() };
        assert!(
            default_v.is_finite(),
            "param '{}' has non-finite default normalized value: {}",
            id, default_v
        );
        assert!(
            (0.0..=1.0).contains(&default_v),
            "param '{}' default normalized value {} is outside [0.0, 1.0]",
            id, default_v
        );
    }
}

#[test]
fn param_map_has_no_duplicate_ids() {
    let params = SpectralForgeParams::default();
    let map = params.param_map();

    let mut seen = std::collections::HashSet::new();
    for (id, _ptr, _group) in &map {
        assert!(
            seen.insert(id.clone()),
            "duplicate param id found: '{}'",
            id
        );
    }
}

#[test]
fn all_param_current_values_are_finite_and_normalized() {
    let params = SpectralForgeParams::default();
    let map = params.param_map();

    for (id, ptr, _group) in &map {
        // SAFETY: `params` is alive for the duration of this test.
        let current_v = unsafe { ptr.unmodulated_normalized_value() };
        assert!(
            current_v.is_finite(),
            "param '{}' has non-finite unmodulated normalized value: {}",
            id, current_v
        );
        assert!(
            (0.0..=1.0).contains(&current_v),
            "param '{}' unmodulated normalized value {} is outside [0.0, 1.0]",
            id, current_v
        );
    }
}
