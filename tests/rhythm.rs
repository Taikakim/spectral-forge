use spectral_forge::dsp::modules::{ModuleType, module_spec};

#[test]
fn rhythm_module_spec_basic() {
    let spec = module_spec(ModuleType::Rhythm);
    assert_eq!(spec.num_curves, 5);
    assert_eq!(spec.curve_labels, &["AMOUNT", "DIVISION", "ATTACK_FADE", "TARGET_PHASE", "MIX"]);
    assert!(!spec.supports_sidechain);
    assert!(!spec.wants_sidechain);
    assert_eq!(spec.display_name, "Rhythm");
}

use spectral_forge::dsp::modules::rhythm::{RhythmModule, RhythmMode, ArpGrid};

#[test]
fn rhythm_mode_default_is_euclidean() {
    assert_eq!(RhythmMode::default(), RhythmMode::Euclidean);
}

#[test]
fn arp_grid_default_is_empty() {
    let g = ArpGrid::default();
    for v in 0..8 {
        assert_eq!(g.steps[v], 0u8, "voice {} should start with no active steps", v);
    }
}

#[test]
fn rhythm_module_zero_bpm_is_passthrough() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::{SpectralModule, ModuleContext};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let mut m = RhythmModule::new();
    m.reset(48000.0, 1024);
    let mut bins = vec![Complex::new(0.5, 0.1); 513];
    let original = bins.clone();
    let curves_storage: Vec<Vec<f32>> = (0..5).map(|_| vec![1.0f32; 513]).collect();
    let curves: Vec<&[f32]> = curves_storage.iter().map(|v| v.as_slice()).collect();
    let mut supp = vec![0.0f32; 513];
    let ctx = ModuleContext::new(
        48000.0, 1024, 513,
        10.0, 100.0, 0.5,
        1.0, false, false,
    );
    // bpm/beat_position default to 0.0 in ::new; nothing to set for this test.
    m.process(0, StereoLink::Linked, FxChannelTarget::All,
        &mut bins, None, &curves, &mut supp, &ctx);
    // BPM=0 → no rhythmic gating → output should equal input under MIX=0.5.
    // Since MIX=1.0 from neutral curve, and dry == wet at BPM=0, output = wet ≈ original
    for (a, b) in bins.iter().zip(original.iter()) {
        assert!((a.re - b.re).abs() < 1e-3 && (a.im - b.im).abs() < 1e-3);
    }
}
