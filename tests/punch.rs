use spectral_forge::dsp::modules::{ModuleType, module_spec};

#[test]
fn punch_module_spec() {
    let spec = module_spec(ModuleType::Punch);
    assert_eq!(spec.num_curves, 6);
    assert_eq!(spec.curve_labels, &["AMOUNT", "WIDTH", "FILL_MODE", "AMP_FILL", "HEAL", "MIX"]);
    assert!(spec.supports_sidechain);
    assert!(spec.wants_sidechain);
    assert_eq!(spec.display_name, "Punch");
}

#[test]
fn punch_mode_default_is_direct() {
    use spectral_forge::dsp::modules::punch::PunchMode;
    assert_eq!(PunchMode::default(), PunchMode::Direct);
}

#[test]
fn punch_module_no_sidechain_is_passthrough() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::punch::PunchModule;
    use spectral_forge::dsp::modules::{SpectralModule, ModuleContext};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let mut m = PunchModule::new();
    m.reset(48000.0, 1024);
    let mut bins = vec![Complex::new(0.5, 0.1); 513];
    let original = bins.clone();
    let curves_storage: Vec<Vec<f32>> = (0..6).map(|_| vec![1.0f32; 513]).collect();
    let curves: Vec<&[f32]> = curves_storage.iter().map(|v| v.as_slice()).collect();
    let mut supp = vec![0.0f32; 513];
    let ctx = ModuleContext::new(
        48000.0, 1024, 513,
        10.0, 100.0, 0.5,
        1.0, false, false,
    );
    // No sidechain → no carve → output ≈ input
    m.process(0, StereoLink::Linked, FxChannelTarget::All,
        &mut bins, None, &curves, &mut supp, &ctx);
    for (a, b) in bins.iter().zip(original.iter()) {
        assert!((a.re - b.re).abs() < 1e-4 && (a.im - b.im).abs() < 1e-4,
            "no-sidechain Punch should be transparent, got {:?} vs {:?}", a, b);
    }
}

#[test]
fn detect_peaks_finds_local_maxima_above_threshold() {
    use spectral_forge::dsp::modules::punch::detect_peaks;

    let mut sc = vec![0.0f32; 64];
    sc[10] = 0.9;
    sc[20] = 0.5;
    sc[30] = 0.95;
    sc[40] = 0.1; // below threshold
    let mut peaks = [0u32; 32];
    let count = detect_peaks(&sc, &mut peaks, 0.3, 8);

    assert!(count >= 3, "expected ≥3 peaks, got {}", count);
    let bins: std::collections::HashSet<u32> = peaks[..count].iter().copied().collect();
    assert!(bins.contains(&10));
    assert!(bins.contains(&20));
    assert!(bins.contains(&30));
    assert!(!bins.contains(&40));
}

#[test]
fn detect_peaks_enforces_minimum_distance() {
    use spectral_forge::dsp::modules::punch::detect_peaks;

    let mut sc = vec![0.0f32; 64];
    sc[10] = 0.5;
    sc[12] = 0.6; // higher and within min_dist of 10 → wins; 10 is suppressed
    sc[30] = 0.7;
    let mut peaks = [0u32; 32];
    let count = detect_peaks(&sc, &mut peaks, 0.3, 8);

    let bins: std::collections::HashSet<u32> = peaks[..count].iter().copied().collect();
    assert!(bins.contains(&12));
    assert!(!bins.contains(&10), "bin 10 should be suppressed by bin 12 (within min_dist=8)");
    assert!(bins.contains(&30));
}
