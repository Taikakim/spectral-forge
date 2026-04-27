use spectral_forge::dsp::modules::ModuleType;
use spectral_forge::dsp::modules::geometry::{GeometryModule, GeometryMode, N_TRAPS, GEO_GRID_W, GEO_GRID_H};

#[test]
fn geometry_mode_default_is_chladni() {
    assert_eq!(GeometryMode::default(), GeometryMode::Chladni);
}

#[test]
fn geometry_constants_are_sensible() {
    assert_eq!(N_TRAPS, 8);
    assert_eq!(GEO_GRID_W, 128);
    assert_eq!(GEO_GRID_H, 64);
}

#[test]
fn geometry_module_skeleton_is_passthrough_and_zeros_suppression() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::{SpectralModule, ModuleContext};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let mut m = GeometryModule::new();
    m.reset(48_000.0, 2048);
    assert_eq!(m.module_type(), ModuleType::Geometry);
    assert_eq!(m.num_curves(), 5);

    let num_bins = 1025;
    let mut bins = vec![Complex::new(0.5, 0.1); num_bins];
    let original = bins.clone();
    let curves_storage: Vec<Vec<f32>> = (0..5).map(|_| vec![1.0f32; num_bins]).collect();
    let curves: Vec<&[f32]> = curves_storage.iter().map(|v| v.as_slice()).collect();
    let mut supp = vec![0.0f32; num_bins];
    let ctx = ModuleContext::new(
        48_000.0, 2048, num_bins,
        10.0, 100.0, 1.0,
        0.5, false, false,
    );

    m.process(0, StereoLink::Linked, FxChannelTarget::All,
              &mut bins, None, &curves, &mut supp, &ctx);

    // Skeleton stub: bins must be unchanged and suppression_out zeroed.
    for (a, b) in bins.iter().zip(original.iter()) {
        assert!((a.re - b.re).abs() < 1e-6 && (a.im - b.im).abs() < 1e-6);
    }
    assert!(supp.iter().all(|&x| x == 0.0));
}

#[test]
fn geometry_set_mode_round_trip() {
    let mut m = GeometryModule::new();
    assert_eq!(m.current_mode(), GeometryMode::Chladni);
    m.set_mode(GeometryMode::Helmholtz);
    assert_eq!(m.current_mode(), GeometryMode::Helmholtz);
}

#[test]
fn geometry_reset_preserves_mode() {
    use spectral_forge::dsp::modules::SpectralModule;
    let mut m = GeometryModule::new();
    m.set_mode(GeometryMode::Helmholtz);
    m.reset(48_000.0, 4096);   // FFT-size change
    assert_eq!(m.current_mode(), GeometryMode::Helmholtz,
        "reset must preserve user's mode choice across FFT-size changes");
}

#[test]
fn geometry_create_module_returns_geometry() {
    use spectral_forge::dsp::modules::create_module;
    let m = create_module(ModuleType::Geometry, 48_000.0, 2048);
    assert_eq!(m.module_type(), ModuleType::Geometry);
    assert_eq!(m.num_curves(), 5);
}
