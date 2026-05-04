//! Past UX overhaul scalars round-trip through nih-plug params. See
//! docs/superpowers/specs/2026-05-04-past-module-ux-design.md §2 + §3.

use spectral_forge::params::SpectralForgeParams;

#[test]
fn past_scalar_params_exist_with_correct_defaults() {
    let params = SpectralForgeParams::default();

    for s in 0..9usize {
        let floor = params.past_floor_param(s).expect("floor exists for slot");
        assert!((floor.value() - 230.0).abs() < 1.0, "Floor default ≈ 230 Hz, got {}", floor.value());

        let window = params.past_reverse_window_param(s).expect("window exists for slot");
        assert!((window.value() - 1.0).abs() < 1e-6);

        let rate = params.past_stretch_rate_param(s).expect("rate exists for slot");
        assert!((rate.value() - 1.0).abs() < 1e-6);

        let dither = params.past_stretch_dither_param(s).expect("dither exists for slot");
        assert_eq!(dither.value(), 0.0);

        let soft_clip = params.past_soft_clip_param(s).expect("soft_clip exists for slot");
        assert!(soft_clip.value(), "Soft Clip default ON");
    }
}

#[test]
fn past_scalar_params_out_of_range_returns_none() {
    let params = SpectralForgeParams::default();
    assert!(params.past_floor_param(9).is_none());
    assert!(params.past_reverse_window_param(9).is_none());
    assert!(params.past_stretch_rate_param(9).is_none());
    assert!(params.past_stretch_dither_param(9).is_none());
    assert!(params.past_soft_clip_param(9).is_none());
}
