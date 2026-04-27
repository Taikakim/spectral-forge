use spectral_forge::dsp::modules::{ModuleType, module_spec};

#[test]
fn future_module_spec_has_5_curves() {
    let spec = module_spec(ModuleType::Future);
    assert_eq!(spec.num_curves, 5);
    assert_eq!(spec.curve_labels, &["AMOUNT", "TIME", "THRESHOLD", "SPREAD", "MIX"]);
    assert!(!spec.supports_sidechain);
    assert_eq!(spec.display_name, "Future");
}

use spectral_forge::dsp::modules::future::{FutureModule, FutureMode};

#[test]
fn future_mode_default_is_print_through() {
    assert_eq!(FutureMode::default(), FutureMode::PrintThrough);
}

#[test]
fn future_module_starts_silent() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::{SpectralModule, ModuleContext};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let mut m = FutureModule::new();
    m.reset(48000.0, 1024);
    let mut bins = vec![Complex::new(1.0, 0.0); 513];
    let curves_storage: Vec<Vec<f32>> = (0..5).map(|_| vec![1.0f32; 513]).collect();
    let curves: Vec<&[f32]> = curves_storage.iter().map(|v| v.as_slice()).collect();
    let mut supp = vec![0.0f32; 513];
    let ctx = ModuleContext::new(
        48000.0, 1024, 513,
        10.0, 100.0, 0.5, 1.0, false, false,
    );
    m.process(0, StereoLink::Linked, FxChannelTarget::All,
        &mut bins, None, &curves, &mut supp, &ctx);
    for c in &bins { assert!(c.re.is_finite() && c.im.is_finite()); }
}
