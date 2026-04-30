use num_complex::Complex;
use spectral_forge::dsp::modules::{
    create_module, ModuleContext, ModuleType, SpectralModule,
    HarmonyModule, HarmonyMode,
};
use spectral_forge::params::{FxChannelTarget, StereoLink};

fn ctx_default<'a>() -> ModuleContext<'a> {
    ModuleContext::new(48_000.0, 2048, 1025, 10.0, 100.0, 0.5, 0.0, false, false)
}

#[test]
fn harmony_mode_dispatch_passthrough_when_amount_zero() {
    let mut m = HarmonyModule::new();
    m.reset(48_000.0, 2048);
    m.set_mode(HarmonyMode::Shuffler);
    let mut bins: Vec<Complex<f32>> = (0..1025)
        .map(|k| Complex::new(k as f32 * 0.01, 0.0))
        .collect();
    let snapshot = bins.clone();
    let curves: Vec<f32> = vec![0.0; 1025]; // AMOUNT = 0 means passthrough
    let one: Vec<f32> = vec![1.0; 1025];
    let curve_refs: Vec<&[f32]> = vec![
        &curves, &one, &one, &one, &one, &one,
    ];
    let mut sup = vec![0.0; 1025];
    m.process(
        0, StereoLink::Linked, FxChannelTarget::All,
        &mut bins, None, &curve_refs, &mut sup, None, &ctx_default(),
    );
    for k in 0..1025 {
        assert!(
            (bins[k] - snapshot[k]).norm() < 1e-6,
            "amount=0 must be exact passthrough at bin {}: got {:?}, want {:?}",
            k, bins[k], snapshot[k]
        );
    }
}

#[test]
fn harmony_create_module_roundtrip() {
    // Verify create_module wires Harmony correctly and num_curves matches spec.
    let mut m = create_module(ModuleType::Harmony, 48_000.0, 2048);
    assert_eq!(m.module_type(), ModuleType::Harmony);
    assert_eq!(m.num_curves(), 6);
    let mut bins = vec![Complex::new(1.0f32, 0.0); 1025];
    let zero = vec![0.0f32; 1025];
    let one = vec![1.0f32; 1025];
    let curve_refs: Vec<&[f32]> = vec![&zero, &one, &one, &one, &one, &one];
    let mut sup = vec![0.0f32; 1025];
    let ctx = ctx_default();
    m.process(
        0, StereoLink::Linked, FxChannelTarget::All,
        &mut bins, None, &curve_refs, &mut sup, None, &ctx,
    );
    // All bins finite after passthrough.
    for b in &bins {
        assert!(b.re.is_finite() && b.im.is_finite());
    }
}
