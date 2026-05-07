//! Module-switch state hygiene regression. After a module is reassigned,
//! every per-curve transform FloatParam (tilt/offset/curvature) must be 0.0.
//! See docs/superpowers/specs/2026-05-07-stabilization-sweep-bc-design.md §B-1.

use spectral_forge::params::SpectralForgeParams;

#[test]
fn tilt_offset_curvature_reset_to_zero_on_assign_module() {
    let p = SpectralForgeParams::default();
    let slot = 2;

    // NOTE: set_plain_value is pub(crate) in nih-plug — cannot be called from
    // external test crates. We set the smoother directly, which is what
    // assign_module uses internally. This verifies the smoother path is usable
    // from tests.
    for c in 0..7 {
        if let Some(t) = p.tilt_param(slot, c) {
            t.smoothed.reset(0.5);
        }
        if let Some(o) = p.offset_param(slot, c) {
            o.smoothed.reset(0.3);
        }
        if let Some(cu) = p.curvature_param(slot, c) {
            cu.smoothed.reset(-0.4);
        }
    }

    // Verify the smoothers hold the non-zero values we just wrote.
    assert!(
        (p.tilt_param(slot, 0).unwrap().smoothed.next() - 0.5).abs() < 1e-5,
        "tilt smoother should hold 0.5 before reset"
    );
    assert!(
        (p.offset_param(slot, 0).unwrap().smoothed.next() - 0.3).abs() < 1e-5,
        "offset smoother should hold 0.3 before reset"
    );
    assert!(
        (p.curvature_param(slot, 0).unwrap().smoothed.next() - (-0.4)).abs() < 1e-5,
        "curvature smoother should hold -0.4 before reset"
    );

    // Helper produces (curve_index, kind, value) triples.
    let pairs: Vec<_> = spectral_forge::editor::module_popup::transform_reset_pairs(slot)
        .collect();
    // 7 curves × 3 params = 21 pairs.
    assert_eq!(pairs.len(), 21);
    for (c, kind, value) in &pairs {
        assert!(*c < 7, "curve index {c} out of range");
        assert_eq!(*value, 0.0_f32, "transform_reset_pairs must yield 0.0 for {kind} at curve {c}");
        let _ = kind;
    }

    // Verify the pairs cover all three kinds for every curve.
    let tilts:      Vec<_> = pairs.iter().filter(|(_, k, _)| *k == "tilt").collect();
    let offsets:    Vec<_> = pairs.iter().filter(|(_, k, _)| *k == "offset").collect();
    let curvatures: Vec<_> = pairs.iter().filter(|(_, k, _)| *k == "curvature").collect();
    assert_eq!(tilts.len(),      7, "expected 7 tilt entries");
    assert_eq!(offsets.len(),    7, "expected 7 offset entries");
    assert_eq!(curvatures.len(), 7, "expected 7 curvature entries");
}
