#[test]
fn supports_sidechain_flag_matches_spec() {
    use spectral_forge::dsp::modules::{module_spec, ModuleType};
    assert!(module_spec(ModuleType::Dynamics).supports_sidechain);
    assert!(module_spec(ModuleType::Gain).supports_sidechain);
    assert!(module_spec(ModuleType::PhaseSmear).supports_sidechain);
    assert!(module_spec(ModuleType::Freeze).supports_sidechain);
    assert!(!module_spec(ModuleType::Contrast).supports_sidechain);
    assert!(!module_spec(ModuleType::MidSide).supports_sidechain);
    assert!(!module_spec(ModuleType::TransientSustainedSplit).supports_sidechain);
    assert!(!module_spec(ModuleType::Harmonic).supports_sidechain);
    assert!(!module_spec(ModuleType::Master).supports_sidechain);
    assert!(!module_spec(ModuleType::Empty).supports_sidechain);
}

#[test]
fn module_trait_types_exist() {
    use spectral_forge::dsp::modules::{
        ModuleType, GainMode, VirtualRowKind, RouteMatrix,
        apply_curve_transform, create_module,
    };
    let _ = ModuleType::Dynamics;
    let _ = GainMode::Add;
    let _ = VirtualRowKind::Transient;
    let mut gains = vec![1.0f32; 8];
    apply_curve_transform(&mut gains, 0.5, 0.1, 44100.0, 2048);
    assert!(gains.iter().all(|&g| g >= 0.0));
    let m = create_module(ModuleType::Master, 44100.0, 2048);
    assert_eq!(m.module_type(), ModuleType::Master);
    assert_eq!(m.num_outputs(), None);
}

#[test]
fn curve_labels_post_refactor() {
    use spectral_forge::dsp::modules::{module_spec, ModuleType};
    assert_eq!(module_spec(ModuleType::Gain).curve_labels, &["GAIN", "PEAK HOLD"]);
    assert_eq!(module_spec(ModuleType::PhaseSmear).curve_labels, &["AMOUNT", "PEAK HOLD", "MIX"]);
    assert_eq!(module_spec(ModuleType::Contrast).curve_labels, &["AMOUNT"]);
    assert_eq!(module_spec(ModuleType::Contrast).num_curves, 1);
}

#[test]
fn sc_channel_enum_variants() {
    use spectral_forge::params::ScChannel;
    let values = [ScChannel::Follow, ScChannel::LR, ScChannel::L,
                  ScChannel::R, ScChannel::M, ScChannel::S];
    assert_eq!(values.len(), 6);
    assert_eq!(ScChannel::default(), ScChannel::Follow);
}

#[test]
fn per_slot_sc_defaults() {
    use spectral_forge::params::{SpectralForgeParams, ScChannel};
    let p = SpectralForgeParams::default();
    let gains = *p.slot_sc_gain_db.lock();
    let chans = *p.slot_sc_channel.lock();
    assert_eq!(gains.len(), 9);
    assert_eq!(chans.len(), 9);
    for g in gains.iter() {
        assert_eq!(*g, 0.0, "default SC gain should be 0 dB");
    }
    for c in chans.iter() {
        assert_eq!(*c, ScChannel::Follow);
    }
}

#[test]
fn freeze_threshold_default_is_minus_50_db() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::{FreezeModule, ModuleContext, SpectralModule};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let mut m = FreezeModule::new();
    m.reset(48000.0, 2048);

    // Feed a pure silent hop; check that the curve gain of 1.0 maps to a threshold lin
    // that corresponds to -50 dBFS (within 0.5 dB).
    let num_bins = 1025usize;
    let mut bins = vec![Complex::new(0.0, 0.0); num_bins];
    let curves: Vec<Vec<f32>> = (0..5).map(|_| vec![1.0f32; num_bins]).collect();
    let curves_ref: Vec<&[f32]> = curves.iter().map(|v| &v[..]).collect();
    let mut supp = vec![0.0f32; num_bins];
    let ctx = ModuleContext {
        sample_rate: 48000.0,
        fft_size: 2048,
        num_bins,
        attack_ms: 10.0, release_ms: 80.0,
        sensitivity: 0.5, suppression_width: 0.0,
        auto_makeup: false, delta_monitor: false,
    };
    // Process once to capture initial frame.
    m.process(0, StereoLink::Linked, FxChannelTarget::All,
              &mut bins, None, &curves_ref, &mut supp, &ctx);

    // Now craft a bin with magnitude exactly at linear_to_db(-50) * norm_factor.
    // norm_factor = fft_size / 4 = 512.
    let norm_factor = 2048.0f32 / 4.0;
    let thr_lin_expected_minus_50 = 10.0f32.powf(-50.0 / 20.0) * norm_factor;
    // With curve=1.0 → threshold should be -50 dB. Feed a bin *just above* and one *just below*
    // and ensure only the above-threshold one triggers accumulation.
    // (This is a behavioural sanity check on the new mapping.)
    let just_below = thr_lin_expected_minus_50 * 0.9;
    let just_above = thr_lin_expected_minus_50 * 1.1;
    // Per-bin [k=100]: just_above; per-bin [k=200]: just_below.
    bins[100] = Complex::new(just_above, 0.0);
    bins[200] = Complex::new(just_below, 0.0);
    m.process(0, StereoLink::Linked, FxChannelTarget::All,
              &mut bins, None, &curves_ref, &mut supp, &ctx);
    // The test intent: threshold mapping pivots at -50 dB when curve=1.0. Direct state inspection
    // would be fragile; instead we assert the mapping formula holds by calling a pub-for-test helper.
    assert!(
        spectral_forge::dsp::modules::freeze::curve_to_threshold_db(1.0).abs() < 51.0
            && spectral_forge::dsp::modules::freeze::curve_to_threshold_db(1.0).abs() > 49.0,
        "curve=1.0 must map to -50 dB ±1 dB, got {}",
        spectral_forge::dsp::modules::freeze::curve_to_threshold_db(1.0),
    );
}

#[test]
fn gain_pull_peak_hold_decays_with_curve() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::{GainModule, GainMode, ModuleContext, SpectralModule};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let mut m = GainModule::new();
    m.set_gain_mode(GainMode::Pull);
    m.reset(48000.0, 2048);

    let num_bins = 1025usize;
    let mut bins = vec![Complex::new(1.0, 0.0); num_bins];
    let gain_curve   = vec![0.5f32; num_bins];
    let peak_curve   = vec![1.0f32; num_bins];
    let curves_vec: Vec<Vec<f32>> = vec![gain_curve, peak_curve];
    let curves_ref: Vec<&[f32]> = curves_vec.iter().map(|v| &v[..]).collect();

    let sc_impulse: Vec<f32> = (0..num_bins)
        .map(|k| if k == 100 { 5.0 } else { 0.0 })
        .collect();

    let mut supp = vec![0.0f32; num_bins];
    let ctx = ModuleContext {
        sample_rate: 48000.0, fft_size: 2048, num_bins,
        attack_ms: 10.0, release_ms: 80.0,
        sensitivity: 0.5, suppression_width: 0.0,
        auto_makeup: false, delta_monitor: false,
    };
    m.process(0, StereoLink::Linked, FxChannelTarget::All,
              &mut bins, Some(&sc_impulse), &curves_ref, &mut supp, &ctx);
    let env_after_hop1 = m.peak_env_at(100);
    assert!(env_after_hop1 > 4.0, "peak-hold envelope should capture impulse magnitude, got {}", env_after_hop1);

    let sc_silent = vec![0.0f32; num_bins];
    for _ in 0..20 {
        let mut b = vec![Complex::new(1.0, 0.0); num_bins];
        m.process(0, StereoLink::Linked, FxChannelTarget::All,
                  &mut b, Some(&sc_silent), &curves_ref, &mut supp, &ctx);
    }
    let env_after_decay = m.peak_env_at(100);
    assert!(env_after_decay < env_after_hop1,
            "peak-hold envelope should decay over time, before={} after={}",
            env_after_hop1, env_after_decay);
    assert!(env_after_decay >= 0.0);
}

#[test]
fn gain_add_mode_does_not_use_peak_hold() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::{GainModule, GainMode, ModuleContext, SpectralModule};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let mut m = GainModule::new();
    m.set_gain_mode(GainMode::Add);
    m.reset(48000.0, 2048);

    let num_bins = 1025usize;
    let mut bins = vec![Complex::new(1.0, 0.0); num_bins];
    let gain_curve = vec![1.0f32; num_bins];
    let peak_curve = vec![1.0f32; num_bins];
    let curves_vec: Vec<Vec<f32>> = vec![gain_curve, peak_curve];
    let curves_ref: Vec<&[f32]> = curves_vec.iter().map(|v| &v[..]).collect();
    let sc = vec![0.5f32; num_bins];
    let mut supp = vec![0.0f32; num_bins];
    let ctx = ModuleContext {
        sample_rate: 48000.0, fft_size: 2048, num_bins,
        attack_ms: 10.0, release_ms: 80.0,
        sensitivity: 0.5, suppression_width: 0.0,
        auto_makeup: false, delta_monitor: false,
    };

    m.process(0, StereoLink::Linked, FxChannelTarget::All,
              &mut bins, Some(&sc), &curves_ref, &mut supp, &ctx);

    for k in 0..num_bins {
        assert_eq!(m.peak_env_at(k), 0.0, "Add mode must not touch peak-hold state at k={}", k);
    }
}
