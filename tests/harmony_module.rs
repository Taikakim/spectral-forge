use num_complex::Complex;
use spectral_forge::dsp::modules::{
    create_module, ModuleContext, ModuleType, SpectralModule,
    HarmonyModule, HarmonyMode,
};
use spectral_forge::dsp::modules::harmony_helpers::{find_top_k_peaks, PeakRecord};
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

#[test]
fn find_top_k_peaks_returns_strongest_above_threshold() {
    let mag: Vec<f32> = vec![
        0.0, 0.1, 0.5, 0.2, 0.0,   // peak at 2 with mag 0.5
        0.0, 0.0, 0.9, 0.1, 0.0,   // peak at 7 with mag 0.9
        0.0, 0.4, 0.7, 0.3, 0.0,   // peak at 12 with mag 0.7
        0.0, 0.2, 0.6, 0.1, 0.0,   // peak at 17 with mag 0.6
        0.0, 0.05, 0.08, 0.03, 0.0 // below threshold
    ];
    let mut out: Vec<PeakRecord> = vec![PeakRecord::default(); 4];
    let n = find_top_k_peaks(&mag, /* threshold = */ 0.1, &mut out);
    assert_eq!(n, 4);
    assert_eq!(out[0].bin, 7);  assert!((out[0].mag - 0.9).abs() < 1e-6);
    assert_eq!(out[1].bin, 12); assert!((out[1].mag - 0.7).abs() < 1e-6);
    assert_eq!(out[2].bin, 17); assert!((out[2].mag - 0.6).abs() < 1e-6);
    assert_eq!(out[3].bin, 2);  assert!((out[3].mag - 0.5).abs() < 1e-6);
}

#[test]
fn find_top_k_peaks_skips_below_threshold() {
    let mag: Vec<f32> = vec![0.0, 0.05, 0.08, 0.03, 0.0, 0.0, 0.5, 0.0];
    let mut out: Vec<PeakRecord> = vec![PeakRecord::default(); 4];
    let n = find_top_k_peaks(&mag, /* threshold = */ 0.1, &mut out);
    assert_eq!(n, 1, "only bin 6 (mag=0.5) is above threshold");
    assert_eq!(out[0].bin, 6);
}

#[test]
fn find_top_k_peaks_zeros_unused_slots() {
    let mag: Vec<f32> = vec![0.0, 0.0, 0.5, 0.0];
    let mut out: Vec<PeakRecord> = vec![PeakRecord { bin: 999, mag: 99.0 }; 4];
    let n = find_top_k_peaks(&mag, 0.1, &mut out);
    assert_eq!(n, 1);
    for slot in &out[1..] {
        assert_eq!(slot.bin, 0);
        assert_eq!(slot.mag, 0.0);
    }
}
