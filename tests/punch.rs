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

#[test]
fn direct_punch_carves_at_sidechain_peaks() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::{SpectralModule, ModuleContext};
    use spectral_forge::dsp::modules::punch::{PunchModule, PunchMode};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let mut m = PunchModule::new();
    m.set_mode(PunchMode::Direct);
    m.reset(48000.0, 1024);

    let mut sc = vec![0.0f32; 513];
    sc[100] = 0.9; // strong peak

    // AMOUNT=2.0 (full carve), WIDTH=1.0 (4 bins each side), AMP_FILL=1.0 (no boost),
    // HEAL=0.13 (≈20 ms snappy, full carve on first hop), MIX=2.0 (full wet)
    let amount = vec![2.0f32; 513];
    let width  = vec![1.0f32; 513];
    let fillm  = vec![1.0f32; 513];
    let ampfl  = vec![1.0f32; 513];
    let heal   = vec![0.13f32; 513];
    let mix    = vec![2.0f32; 513];
    let curves: Vec<&[f32]> = vec![&amount, &width, &fillm, &ampfl, &heal, &mix];

    let mut bins = vec![Complex::new(1.0, 0.0); 513];
    let mut supp = vec![0.0f32; 513];
    let ctx = ModuleContext::new(
        48000.0, 1024, 513,
        10.0, 100.0, 0.5,
        1.0, false, false,
    );

    // Run several hops so the attack follower converges (5 ms attack, 8 hops × ~5 ms).
    for _ in 0..8 {
        m.process(0, StereoLink::Linked, FxChannelTarget::All,
            &mut bins, Some(&sc), &curves, &mut supp, &ctx);
    }

    // Bin 100 should be heavily attenuated by the carve.
    assert!(bins[100].norm() < 0.5,
        "direct punch should carve bin 100; got {}", bins[100].norm());
    // Far-away bin 200 should be untouched (no carve, no amp-fill).
    assert!((bins[200].norm() - 1.0).abs() < 0.1,
        "far-away bin should be untouched; got {}", bins[200].norm());
}

#[test]
fn inverse_punch_carves_quiet_valleys_not_loud_peaks() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::{SpectralModule, ModuleContext};
    use spectral_forge::dsp::modules::punch::{PunchModule, PunchMode};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let mut m = PunchModule::new();
    m.set_mode(PunchMode::Inverse);
    m.reset(48000.0, 1024);

    // Discriminating SC: loud baseline 0.9, a louder peak at 100 (Inverse skips),
    // a sharp quiet valley at 200 (Inverse carves). Inverted-SC baseline = 0.1,
    // which sits below the threshold = 0.1125 used at AMOUNT=1.5 — so plateau-edge
    // spurious maxima around the loud SC peak at 100 are filtered out and only the
    // genuine inverse peak at 200 (inv = 0.95) registers.
    let mut sc = vec![0.9f32; 513];
    sc[100] = 0.95; // loud — Inverse skips this (inv plateau edges below threshold)
    sc[199] = 0.6;  // brackets the valley so 200 is a clean local minimum
    sc[200] = 0.05; // quiet — Inverse carves here (inv = 0.95)
    sc[201] = 0.6;

    // AMOUNT=1.5 gives threshold≈0.1125 (filters plateau-edge maxima at inv≈0.1)
    // and depth=0.75 (carve to 25% of dry at peak, well under the 0.5 assertion).
    let amount = vec![1.5f32; 513];
    let width  = vec![1.0f32; 513];
    let fillm  = vec![1.0f32; 513];
    let ampfl  = vec![1.0f32; 513];
    let heal   = vec![0.13f32; 513];
    let mix    = vec![2.0f32; 513];
    let curves: Vec<&[f32]> = vec![&amount, &width, &fillm, &ampfl, &heal, &mix];

    let mut bins = vec![Complex::new(1.0, 0.0); 513];
    let mut supp = vec![0.0f32; 513];
    let ctx = ModuleContext::new(
        48000.0, 1024, 513,
        10.0, 100.0, 0.5,
        1.0, false, false,
    );

    for _ in 0..8 {
        m.process(0, StereoLink::Linked, FxChannelTarget::All,
            &mut bins, Some(&sc), &curves, &mut supp, &ctx);
    }

    // Bin 100 (SC loud) should be PRESERVED in Inverse mode.
    assert!(bins[100].norm() > 0.7,
        "inverse punch should preserve bin where SC is loud; got {}", bins[100].norm());
    // Bin 200 (SC quiet — local valley) SHOULD be carved.
    assert!(bins[200].norm() < 0.5,
        "inverse punch should carve quiet valley at 200; got {}", bins[200].norm());
}

#[test]
fn pitch_fill_caps_drift_at_half_bin() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::{SpectralModule, ModuleContext};
    use spectral_forge::dsp::modules::punch::{PunchModule, PunchMode};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let mut m = PunchModule::new();
    m.set_mode(PunchMode::Direct);
    m.reset(48000.0, 1024);

    let mut sc = vec![0.0f32; 513];
    sc[100] = 0.9;

    let amount = vec![2.0f32; 513];
    let width  = vec![1.0f32; 513];
    let fillm  = vec![2.0f32; 513];   // max pitch fill
    let ampfl  = vec![1.0f32; 513];
    let heal   = vec![0.13f32; 513];
    let mix    = vec![2.0f32; 513];
    let curves: Vec<&[f32]> = vec![&amount, &width, &fillm, &ampfl, &heal, &mix];

    // Run 50 hops with the same input — pitch drift should accumulate but cap at 0.5 bins.
    let mut bins;
    let mut supp = vec![0.0f32; 513];
    let ctx = ModuleContext::new(
        48000.0, 1024, 513,
        10.0, 100.0, 0.5,
        1.0, false, false,
    );
    for _ in 0..50 {
        bins = vec![Complex::new(1.0, 0.0); 513];
        m.process(0, StereoLink::Linked, FxChannelTarget::All,
            &mut bins, Some(&sc), &curves, &mut supp, &ctx);
    }
    // Inspect the per-channel drift_accum array (test-only public).
    for k in 0..513 {
        assert!(m.drift_accum_slice(0)[k].abs() <= 0.5 + 1e-4,
            "drift at bin {} = {} exceeded 0.5", k, m.drift_accum_slice(0)[k]);
    }
}
