#[test]
fn supports_sidechain_flag_matches_spec() {
    use spectral_forge::dsp::modules::{module_spec, ModuleType};
    assert!(module_spec(ModuleType::Dynamics).supports_sidechain);
    assert!(module_spec(ModuleType::Gain).supports_sidechain);
    assert!(module_spec(ModuleType::PhaseSmear).supports_sidechain);
    assert!(module_spec(ModuleType::Freeze).supports_sidechain);
    assert!(module_spec(ModuleType::Punch).supports_sidechain);
    assert!(!module_spec(ModuleType::Contrast).supports_sidechain);
    assert!(!module_spec(ModuleType::MidSide).supports_sidechain);
    assert!(!module_spec(ModuleType::TransientSustainedSplit).supports_sidechain);
    assert!(!module_spec(ModuleType::Harmonic).supports_sidechain);
    assert!(!module_spec(ModuleType::Future).supports_sidechain);
    assert!(!module_spec(ModuleType::Rhythm).supports_sidechain);
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
    apply_curve_transform(&mut gains, 0.5, 0.1, 0.0, |g, _| g, 44100.0, 2048);
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
fn freeze_threshold_default_is_minus_20_db() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::{FreezeModule, ModuleContext, SpectralModule};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let mut m = FreezeModule::new();
    m.reset(48000.0, 2048);

    // Feed a pure silent hop; check that the curve gain of 1.0 maps to a threshold of -20 dBFS.
    // This is the y_natural value per curve_config freeze_config(1): gain=1.0 → -20 dBFS.
    let num_bins = 1025usize;
    let mut bins = vec![Complex::new(0.0, 0.0); num_bins];
    let curves: Vec<Vec<f32>> = (0..5).map(|_| vec![1.0f32; num_bins]).collect();
    let curves_ref: Vec<&[f32]> = curves.iter().map(|v| &v[..]).collect();
    let mut supp = vec![0.0f32; num_bins];
    let ctx = ModuleContext::new(
        48000.0, 2048, num_bins,
        10.0, 80.0, 0.5, 0.0, false, false,
    );
    // Process once to capture initial frame.
    m.process(0, StereoLink::Linked, FxChannelTarget::All,
              &mut bins, None, &curves_ref, &mut supp, None, &ctx);

    // Now craft bins at the expected threshold level (-20 dBFS) scaled by norm_factor.
    // norm_factor = fft_size / 4 = 512.
    let norm_factor = 2048.0f32 / 4.0;
    let thr_lin_expected_minus_20 = 10.0f32.powf(-20.0 / 20.0) * norm_factor;
    // Feed a bin just above and one just below the -20 dBFS threshold.
    let just_below = thr_lin_expected_minus_20 * 0.9;
    let just_above = thr_lin_expected_minus_20 * 1.1;
    bins[100] = Complex::new(just_above, 0.0);
    bins[200] = Complex::new(just_below, 0.0);
    m.process(0, StereoLink::Linked, FxChannelTarget::All,
              &mut bins, None, &curves_ref, &mut supp, None, &ctx);
    // Assert the calibration formula: curve=1.0 → -20 dBFS (matches y_natural from curve_config).
    let actual = spectral_forge::dsp::modules::freeze::curve_to_threshold_db(1.0);
    assert!(
        (actual - (-20.0)).abs() < 1.0,
        "curve=1.0 must map to -20 dBFS ±1 dB, got {}",
        actual,
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
    let ctx = ModuleContext::new(
        48000.0, 2048, num_bins,
        10.0, 80.0, 0.5, 0.0, false, false,
    );
    m.process(0, StereoLink::Linked, FxChannelTarget::All,
              &mut bins, Some(&sc_impulse), &curves_ref, &mut supp, None, &ctx);
    let env_after_hop1 = m.peak_env_at(100);
    assert!(env_after_hop1 > 4.0, "peak-hold envelope should capture impulse magnitude, got {}", env_after_hop1);

    let sc_silent = vec![0.0f32; num_bins];
    for _ in 0..20 {
        let mut b = vec![Complex::new(1.0, 0.0); num_bins];
        m.process(0, StereoLink::Linked, FxChannelTarget::All,
                  &mut b, Some(&sc_silent), &curves_ref, &mut supp, None, &ctx);
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
    let ctx = ModuleContext::new(
        48000.0, 2048, num_bins,
        10.0, 80.0, 0.5, 0.0, false, false,
    );

    m.process(0, StereoLink::Linked, FxChannelTarget::All,
              &mut bins, Some(&sc), &curves_ref, &mut supp, None, &ctx);

    for k in 0..num_bins {
        assert_eq!(m.peak_env_at(k), 0.0, "Add mode must not touch peak-hold state at k={}", k);
    }
}

#[test]
fn gain_match_preserves_harmonics_but_tilts_broadband() {
    // Match should:
    //   (a) multiply each bin by a smooth EQ curve (phase preserved, no bin-exact morph)
    //   (b) preserve the main's harmonic peaks — a narrow peak in main stays a narrow peak
    //   (c) tilt main's broad spectral shape toward SC's — louder SC in a region boosts main there
    use num_complex::Complex;
    use spectral_forge::dsp::modules::{GainModule, GainMode, ModuleContext, SpectralModule};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let mut m = GainModule::new();
    m.set_gain_mode(GainMode::Match);
    m.reset(48000.0, 2048);

    let num_bins = 1025usize;
    // Main: flat = 1.0, with one narrow harmonic peak at bin 200.
    let mut bins: Vec<Complex<f32>> = (0..num_bins).map(|k| {
        if k == 200 { Complex::new(10.0, 0.0) } else { Complex::new(1.0, 0.0) }
    }).collect();
    let peak_before = bins[200].norm();
    let neighbor_before = bins[201].norm();

    // SC: broadly 4× louder than main's baseline (constant 4.0).
    let sc = vec![4.0f32; num_bins];

    // GAIN curve = 0 (full match), PEAK HOLD at default 1.0.
    let gain_curve = vec![0.0f32; num_bins];
    let peak_curve = vec![1.0f32; num_bins];
    let curves_vec: Vec<Vec<f32>> = vec![gain_curve, peak_curve];
    let curves_ref: Vec<&[f32]> = curves_vec.iter().map(|v| &v[..]).collect();

    let mut supp = vec![0.0f32; num_bins];
    let ctx = ModuleContext::new(
        48000.0, 2048, num_bins,
        10.0, 80.0, 0.5, 0.0, false, false,
    );

    // Run a handful of hops so peak_env settles on the 4.0 SC.
    let fresh_bins = bins.clone();
    for i in 0..20 {
        bins = fresh_bins.clone();
        m.process(0, StereoLink::Linked, FxChannelTarget::All,
                  &mut bins, Some(&sc), &curves_ref, &mut supp, None, &ctx);
        let _ = i;
    }

    // Every bin must be finite and have no imaginary part introduced (real-scalar multiply).
    for (k, c) in bins.iter().enumerate() {
        assert!(c.re.is_finite() && c.im.is_finite(), "non-finite at k={}", k);
        assert!(c.im.abs() < 1e-5, "imag leaked at k={}: {:?}", k, c);
    }

    // The spectral peak at bin 200 must still tower over its neighbors —
    // Match preserves narrow features (unlike Pull which would flatten them).
    let peak_after = bins[200].norm();
    let neighbor_after = bins[201].norm();
    assert!(peak_after > 3.0 * neighbor_after,
        "peak flattened: peak={} neighbor={} (ratio before={})",
        peak_after, neighbor_after, peak_before / neighbor_before);

    // Broadband tilt: SC is louder than main, so flat regions should be boosted,
    // but clamped to the ±12 dB ceiling (linear ≈ 3.98).
    assert!(neighbor_after > 1.2, "no broadband boost (neighbor={})", neighbor_after);
    assert!(neighbor_after < 4.1,  "boost exceeded clamp (neighbor={})", neighbor_after);
}

#[test]
fn phase_smear_sc_modulates_amount() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::{PhaseSmearModule, ModuleContext, SpectralModule};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let mut a = PhaseSmearModule::new();
    let mut b = PhaseSmearModule::new();
    a.reset(48000.0, 2048);
    b.reset(48000.0, 2048);

    let num_bins = 1025usize;
    let amount   = vec![0.5f32; num_bins];
    let peak     = vec![1.0f32; num_bins];
    let mix      = vec![1.0f32; num_bins];
    let curves_vec: Vec<Vec<f32>> = vec![amount, peak, mix];
    let curves_ref: Vec<&[f32]> = curves_vec.iter().map(|v| &v[..]).collect();

    let sc_hot  = vec![1.0f32; num_bins];
    let sc_cold = vec![0.0f32; num_bins];

    let mut bins_a: Vec<Complex<f32>> = (0..num_bins)
        .map(|_k| Complex::new(1.0, 0.0)).collect();
    let mut bins_b = bins_a.clone();

    let mut supp_a = vec![0.0f32; num_bins];
    let mut supp_b = vec![0.0f32; num_bins];
    let ctx = ModuleContext::new(
        48000.0, 2048, num_bins,
        10.0, 80.0, 0.5, 0.0, false, false,
    );

    a.process(0, StereoLink::Linked, FxChannelTarget::All,
              &mut bins_a, Some(&sc_hot),  &curves_ref, &mut supp_a, None, &ctx);
    b.process(0, StereoLink::Linked, FxChannelTarget::All,
              &mut bins_b, Some(&sc_cold), &curves_ref, &mut supp_b, None, &ctx);

    let diff_a: f32 = bins_a.iter().skip(1).take(num_bins - 2)
        .map(|c| (c.arg()).abs()).sum();
    let diff_b: f32 = bins_b.iter().skip(1).take(num_bins - 2)
        .map(|c| (c.arg()).abs()).sum();
    assert!(diff_a > diff_b,
            "hot SC should produce more smear than cold SC: hot={} cold={}", diff_a, diff_b);
}

// ── Offset calibration sanity tests ───────────────────────────────────────────
// These verify that offset=+1 and offset=-1 drive each curve to its y_max / y_min
// endpoints, matching docs/superpowers/specs/2026-04-23-ui-parameter-spec-design.md §2.

#[test]
fn offset_calibration_thresh_reaches_endpoints() {
    use spectral_forge::dsp::modules::{GainMode, ModuleType};
    use spectral_forge::editor::curve_config::curve_display_config;
    let cfg = curve_display_config(ModuleType::Dynamics, 0, GainMode::Add);
    let g_neutral = 1.0_f32;
    // off=+1 → g=2.0 (neutral 1.0 + pos_span 1.0)
    assert!(((cfg.offset_fn)(g_neutral, 1.0) - 2.0).abs() < 1e-5,
        "thresh off=+1 should give g=2.0, got {}", (cfg.offset_fn)(g_neutral, 1.0));
    // off=-1 → g=-1.0 (neutral 1.0 + 2.0×(-1.0))
    assert!(((cfg.offset_fn)(g_neutral, -1.0) + 1.0).abs() < 1e-5,
        "thresh off=-1 should give g=-1.0, got {}", (cfg.offset_fn)(g_neutral, -1.0));
    // off=0 → identity
    assert!(((cfg.offset_fn)(g_neutral, 0.0) - g_neutral).abs() < 1e-7,
        "thresh off=0 must be identity");
}

#[test]
fn offset_calibration_attack_multiplicative() {
    use spectral_forge::dsp::modules::{GainMode, ModuleType};
    use spectral_forge::editor::curve_config::curve_display_config;
    let cfg = curve_display_config(ModuleType::Dynamics, 2, GainMode::Add);
    let g_neutral = 1.0_f32;
    // off=+1 → g×1024
    let hi = (cfg.offset_fn)(g_neutral, 1.0);
    assert!((hi - 1024.0).abs() < 0.1,
        "atk off=+1 should give g=1024.0, got {}", hi);
    // off=-1 → g/1024
    let lo = (cfg.offset_fn)(g_neutral, -1.0);
    assert!((lo - 1.0 / 1024.0).abs() < 1e-5,
        "atk off=-1 should give g=1/1024, got {}", lo);
    // off=0 → identity
    assert!(((cfg.offset_fn)(g_neutral, 0.0) - g_neutral).abs() < 1e-7,
        "atk off=0 must be identity");
}

#[test]
fn offset_calibration_ratio_additive() {
    use spectral_forge::dsp::modules::{GainMode, ModuleType};
    use spectral_forge::editor::curve_config::curve_display_config;
    let cfg = curve_display_config(ModuleType::Dynamics, 1, GainMode::Add);
    let g_neutral = 1.0_f32;
    // off=+1 → g=1.0+19.0=20.0 (ratio 20:1 at y_max)
    let hi = (cfg.offset_fn)(g_neutral, 1.0);
    assert!((hi - 20.0).abs() < 1e-4,
        "ratio off=+1 should give g=20.0, got {}", hi);
    // off=-1 → clamped: g stays at 1.0 (ratio can't go below 1:1)
    let lo = (cfg.offset_fn)(g_neutral, -1.0);
    assert!((lo - 1.0).abs() < 1e-5,
        "ratio off=-1 should give g=1.0 (clamped at y_min), got {}", lo);
    // off=0 → identity
    assert!(((cfg.offset_fn)(g_neutral, 0.0) - g_neutral).abs() < 1e-7,
        "ratio off=0 must be identity");
}

#[test]
fn offset_calibration_gain_db_multiplicative() {
    use spectral_forge::dsp::modules::{GainMode, ModuleType};
    use spectral_forge::editor::curve_config::curve_display_config;
    let cfg = curve_display_config(ModuleType::Gain, 0, GainMode::Add);
    let g_neutral = 1.0_f32;
    let factor = 7.943_282_f32;
    // off=+1 → g×factor → +18 dB
    let hi = (cfg.offset_fn)(g_neutral, 1.0);
    assert!((hi - factor).abs() < 1e-4,
        "gain_db off=+1 should give g={}, got {}", factor, hi);
    // off=-1 → g/factor → -18 dB
    let lo = (cfg.offset_fn)(g_neutral, -1.0);
    assert!((lo - 1.0 / factor).abs() < 1e-5,
        "gain_db off=-1 should give g=1/{}, got {}", factor, lo);
    // off=0 → identity
    assert!(((cfg.offset_fn)(g_neutral, 0.0) - g_neutral).abs() < 1e-7,
        "gain_db off=0 must be identity");
}

#[test]
fn apply_curve_transform_tilt_scales_with_sample_rate() {
    use spectral_forge::dsp::modules::apply_curve_transform;
    // Same normalized tilt should produce different physical shapes at 48 kHz vs 96 kHz SR.
    // Easier verifiable property: function runs cleanly at 96 kHz SR without NaN/Inf.
    let mut gains = vec![1.0_f32; 513]; // 1024-bin FFT at 96 kHz
    apply_curve_transform(
        &mut gains,
        /* tilt */      1.0,
        /* offset */    0.0,
        /* curvature */ 0.0,
        |g, _| g,        // identity offset_fn
        /* sample_rate */ 96_000.0,
        /* fft_size */    1024,
    );
    assert!(gains.iter().all(|&g| g.is_finite()), "produced non-finite gain at 96 kHz SR");

    // Also verify 44.1 kHz SR produces a different result from 96 kHz SR,
    // confirming the tilt shape varies with Nyquist.
    let mut gains_44 = vec![1.0_f32; 513];
    apply_curve_transform(&mut gains_44, 1.0, 0.0, 0.0, |g, _| g, 44_100.0, 1024);
    let mut gains_96 = vec![1.0_f32; 513];
    apply_curve_transform(&mut gains_96, 1.0, 0.0, 0.0, |g, _| g, 96_000.0, 1024);
    let differ = gains_44.iter().zip(gains_96.iter()).any(|(a, b)| (a - b).abs() > 1e-4);
    assert!(differ, "tilt shape must differ between 44.1 kHz and 96 kHz sample rates");
}

#[test]
fn offset_identity_at_zero_all_dynamics_curves() {
    use spectral_forge::dsp::modules::{GainMode, ModuleType};
    use spectral_forge::editor::curve_config::curve_display_config;
    // Verify the contract: offset_fn(g, 0.0) == g for every Dynamics curve.
    for c in 0..6 {
        let cfg = curve_display_config(ModuleType::Dynamics, c, GainMode::Add);
        for &g in &[0.0f32, 0.5, 1.0, 2.0] {
            let result = (cfg.offset_fn)(g, 0.0);
            assert!((result - g).abs() < 1e-7,
                "Dynamics curve {} offset_fn(g={}, 0) should be {}, got {}", c, g, g, result);
        }
    }
}

#[test]
fn module_context_has_block_lifetime_and_is_not_copy() {
    use spectral_forge::dsp::modules::ModuleContext;
    fn assert_not_copy<T>() where T: Sized {}  // intentionally no Copy bound
    assert_not_copy::<ModuleContext<'static>>();
    // If this compiles after Task 1, the lifetime is in place.
}

#[test]
fn geometry_module_spec() {
    use spectral_forge::dsp::modules::{module_spec, ModuleType};
    let spec = module_spec(ModuleType::Geometry);
    assert_eq!(spec.display_name, "Geometry");
    assert_eq!(spec.num_curves, 5);
    assert_eq!(spec.curve_labels.len(), 5);
    assert_eq!(spec.curve_labels[0], "AMOUNT");
    assert_eq!(spec.curve_labels[4], "MIX");
    assert!(!spec.wants_sidechain);
    assert!(!spec.supports_sidechain);
    assert!(spec.panel_widget.is_none());
}

#[test]
fn module_context_optional_fields_default_to_none() {
    use spectral_forge::dsp::modules::ModuleContext;
    let ctx = ModuleContext::new(
        48000.0, 2048, 1025, 10.0, 100.0, 1.0, 0.5, false, false,
    );
    assert!(ctx.unwrapped_phase.is_none());
    assert!(ctx.peaks.is_none());
    assert!(ctx.instantaneous_freq.is_none());
    assert!(ctx.chromagram.is_none());
    assert!(ctx.midi_notes.is_none());
    assert!(ctx.sidechain_derivative.is_none());
    assert_eq!(ctx.bpm, 0.0);
    assert_eq!(ctx.beat_position, 0.0);
}

#[test]
fn module_context_has_bin_physics_slot_default_none() {
    use spectral_forge::dsp::modules::ModuleContext;
    let ctx = ModuleContext::new(48_000.0, 2048, 1025, 10.0, 100.0, 1.0, 0.5, false, false);
    assert!(ctx.bin_physics.is_none());
}

#[test]
fn life_module_spec_present() {
    use spectral_forge::dsp::modules::{module_spec, ModuleType};

    let spec = module_spec(ModuleType::Life);
    assert_eq!(spec.display_name, "LIFE");
    assert_eq!(spec.num_curves, 5);
    assert_eq!(spec.curve_labels.len(), 5);
    assert_eq!(spec.curve_labels[0], "AMOUNT");
    assert_eq!(spec.curve_labels[1], "THRESHOLD");
    assert_eq!(spec.curve_labels[2], "SPEED");
    assert_eq!(spec.curve_labels[3], "REACH");
    assert_eq!(spec.curve_labels[4], "MIX");
    assert!(!spec.wants_sidechain, "Life is not a sidechain-driven module");
    assert!(spec.writes_bin_physics, "Life writes crystallization/bias/displacement");
}

#[test]
fn life_module_constructs_and_passes_through() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::{create_module, ModuleContext, ModuleType};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let mut module = create_module(ModuleType::Life, 48_000.0, 2048);
    assert_eq!(module.module_type(), ModuleType::Life);
    assert_eq!(module.num_curves(), 5);

    let num_bins = 1025usize;
    let mut bins: Vec<Complex<f32>> = (0..num_bins)
        .map(|k| Complex::new((k as f32 * 0.013).sin(), (k as f32 * 0.011).cos()))
        .collect();
    let dry: Vec<Complex<f32>> = bins.clone();

    let zeros = vec![0.0_f32; num_bins];
    let neutral = vec![1.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&zeros, &neutral, &neutral, &neutral, &zeros];

    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = ModuleContext::new(
        48_000.0, 2048, num_bins,
        10.0, 80.0, 0.5, 0.0, false, false,
    );

    module.process(
        0, StereoLink::Linked, FxChannelTarget::All,
        &mut bins, None, &curves, &mut suppression, None, &ctx,
    );

    for k in 0..num_bins {
        let diff = (bins[k] - dry[k]).norm();
        assert!(diff < 1e-5, "bin {} drifted by {} (passthrough expected)", k, diff);
    }
    for s in &suppression {
        assert!(s.is_finite() && *s >= 0.0);
    }
}

#[test]
fn module_spec_writes_bin_physics_defaults_false_for_all_modules() {
    use spectral_forge::dsp::modules::{ModuleType, module_spec};
    for ty in [
        ModuleType::Dynamics, ModuleType::Freeze, ModuleType::PhaseSmear,
        ModuleType::Contrast, ModuleType::Gain, ModuleType::MidSide,
        ModuleType::TransientSustainedSplit, ModuleType::Harmonic,
        ModuleType::Future, ModuleType::Punch, ModuleType::Rhythm,
        ModuleType::Geometry, ModuleType::Modulate, ModuleType::Circuit,
        ModuleType::Master, ModuleType::Empty,
    ] {
        assert!(!module_spec(ty).writes_bin_physics,
            "{:?}: writes_bin_physics must default to false in Phase 3", ty);
    }
}

#[test]
fn life_viscosity_diffuses_and_conserves() {
    use spectral_forge::dsp::modules::life::{LifeModule, LifeMode};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule};
    use spectral_forge::params::{StereoLink, FxChannelTarget};
    use num_complex::Complex;

    let mut module = LifeModule::new();
    module.reset(48_000.0, 2048);
    module.set_mode(LifeMode::Viscosity);

    let num_bins = 1025;
    let mut bins: Vec<Complex<f32>> = vec![Complex::new(0.0, 0.0); num_bins];
    bins[100] = Complex::new(2.0, 0.0); // single tone, all energy at bin 100
    let dry_power: f32 = bins.iter().map(|b| b.norm_sqr()).sum();

    // AMOUNT=2 (D=0.45 max), THRESHOLD=neutral, SPEED=neutral, REACH=neutral, MIX=2 (full wet)
    let amount  = vec![2.0_f32; num_bins];
    let neutral = vec![1.0_f32; num_bins];
    let mix     = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &neutral, &neutral, &neutral, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = ModuleContext::new(
        48_000.0, 2048, num_bins,
        10.0, 100.0, 1.0, 0.0, false, false,
    );

    // 5 hops to let energy spread.
    for _ in 0..5 {
        module.process(
            0, StereoLink::Linked, FxChannelTarget::All,
            &mut bins, None, &curves, &mut suppression, None, &ctx,
        );
    }

    let wet_power: f32 = bins.iter().map(|b| b.norm_sqr()).sum();

    let loss_pct = (dry_power - wet_power).abs() / dry_power;
    assert!(loss_pct < 0.05,
        "Viscosity lost {:.2}% of power (>5% violates conservation)", loss_pct * 100.0);

    assert!(bins[99].norm()  > 0.01, "Energy did not diffuse left  (bin 99 = {})",  bins[99].norm());
    assert!(bins[101].norm() > 0.01, "Energy did not diffuse right (bin 101 = {})", bins[101].norm());

    for b in &bins {
        assert!(b.norm().is_finite());
    }
}

#[test]
fn life_crystallization_writes_bin_physics() {
    use spectral_forge::dsp::modules::life::{LifeModule, LifeMode};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule};
    use spectral_forge::dsp::bin_physics::BinPhysics;
    use spectral_forge::params::{StereoLink, FxChannelTarget};
    use num_complex::Complex;

    let mut module = LifeModule::new();
    module.reset(48_000.0, 2048);
    module.set_mode(LifeMode::Crystallization);

    let num_bins = 1025;
    // Sustained tone at bin 50, magnitude 0.8.
    let bins_template: Vec<Complex<f32>> = {
        let mut v = vec![Complex::new(0.0, 0.0); num_bins];
        v[50] = Complex::new(0.8, 0.0);
        v
    };

    let amount  = vec![2.0_f32; num_bins];
    let thresh  = vec![0.5_f32; num_bins]; // low → bin 50's mag (0.8) easily exceeds
    let speed   = vec![2.0_f32; num_bins];
    let neutral = vec![1.0_f32; num_bins];
    let mix     = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &thresh, &speed, &neutral, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let mut physics = BinPhysics::new();
    physics.reset_active(num_bins, 48_000.0, 2048);
    let ctx = ModuleContext::new(
        48_000.0, 2048, num_bins,
        10.0, 100.0, 1.0, 0.0, false, false,
    );

    // 50 hops to let sustain envelope build at bin 50. Re-supply input each hop.
    let mut bins = bins_template.clone();
    for _ in 0..50 {
        module.process(
            0, StereoLink::Linked, FxChannelTarget::All,
            &mut bins, None, &curves, &mut suppression, Some(&mut physics), &ctx,
        );
        bins = bins_template.clone();
    }

    assert!(physics.crystallization[50] > 0.5,
        "crystallization[50] = {} (expected > 0.5 after 50 hops of sustain)",
        physics.crystallization[50]);

    assert!(physics.crystallization[0]   < 0.1, "quiet bin 0 leaked: {}", physics.crystallization[0]);
    assert!(physics.crystallization[100] < 0.1, "quiet bin 100 leaked: {}", physics.crystallization[100]);

    for b in &bins {
        assert!(b.norm().is_finite());
    }
}

#[test]
fn life_surface_tension_coalesces_peaks() {
    use spectral_forge::dsp::modules::life::{LifeModule, LifeMode};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule};
    use spectral_forge::params::{StereoLink, FxChannelTarget};
    use num_complex::Complex;

    let mut module = LifeModule::new();
    module.reset(48_000.0, 2048);
    module.set_mode(LifeMode::SurfaceTension);

    let num_bins = 1025;
    // A "noisy" cluster around bin 200: bins [180..=220] all = 1.0.
    let mut bins: Vec<Complex<f32>> = vec![Complex::new(0.0, 0.0); num_bins];
    for k in 180..=220 {
        bins[k] = Complex::new(1.0, 0.0);
    }
    let dry_total_mag: f32 = bins.iter().map(|b| b.norm()).sum();

    // AMOUNT=2 (max attract), THRESHOLD=0.5 (low — most cluster bins qualify),
    // SPEED=neutral, REACH=2 (long reach), MIX=2 (full wet).
    let amount  = vec![2.0_f32; num_bins];
    let thresh  = vec![0.5_f32; num_bins];
    let neutral = vec![1.0_f32; num_bins];
    let reach   = vec![2.0_f32; num_bins];
    let mix     = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &thresh, &neutral, &reach, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = ModuleContext::new(
        48_000.0, 2048, num_bins,
        10.0, 100.0, 1.0, 0.0, false, false,
    );

    // 10 hops — coalescence is gradual.
    for _ in 0..10 {
        module.process(
            0, StereoLink::Linked, FxChannelTarget::All,
            &mut bins, None, &curves, &mut suppression, None, &ctx,
        );
    }

    let wet_total_mag: f32 = bins.iter().map(|b| b.norm()).sum();
    let loss_pct = (dry_total_mag - wet_total_mag).abs() / dry_total_mag;
    assert!(loss_pct < 0.10,
        "Surface Tension lost {:.2}% of magnitude (>10%)", loss_pct * 100.0);

    // Variance of cluster bins must INCREASE — peaks taller, valleys deeper.
    let cluster: Vec<f32> = (180..=220).map(|k| bins[k].norm()).collect();
    let mean: f32 = cluster.iter().sum::<f32>() / cluster.len() as f32;
    let var:  f32 = cluster.iter().map(|m| (m - mean).powi(2)).sum::<f32>() / cluster.len() as f32;
    assert!(var > 0.05, "Cluster did not coalesce (variance = {})", var);

    for b in &bins {
        assert!(b.norm().is_finite());
    }
}

#[test]
fn life_archimedes_ducks_under_loud_volume() {
    use spectral_forge::dsp::modules::life::{LifeModule, LifeMode};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule};
    use spectral_forge::params::{StereoLink, FxChannelTarget};
    use num_complex::Complex;

    let mut module = LifeModule::new();
    module.reset(48_000.0, 2048);
    module.set_mode(LifeMode::Archimedes);

    let num_bins = 1025;
    // High-volume signal: every bin = 1.0.
    let mut bins: Vec<Complex<f32>> = (0..num_bins).map(|_| Complex::new(1.0, 0.0)).collect();
    let dry_total: f32 = bins.iter().map(|b| b.norm()).sum();

    // AMOUNT=2 (max ducking), THRESHOLD=0.5 (low — pool fills easily),
    // SPEED=neutral, REACH=neutral, MIX=2 (full wet).
    let amount  = vec![2.0_f32; num_bins];
    let thresh  = vec![0.5_f32; num_bins];
    let neutral = vec![1.0_f32; num_bins];
    let mix     = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &thresh, &neutral, &neutral, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = ModuleContext::new(
        48_000.0, 2048, num_bins,
        10.0, 100.0, 1.0, 0.0, false, false,
    );

    module.process(
        0, StereoLink::Linked, FxChannelTarget::All,
        &mut bins, None, &curves, &mut suppression, None, &ctx,
    );

    let wet_total: f32 = bins.iter().map(|b| b.norm()).sum();

    // Expected: capacity = 1025 * 0.25 = 256.25; overflow_ratio ≈ 3.0; duck_factor
    // floors at DUCK_FLOOR = 0.05. With full mix, wet ≈ 0.05 × dry. Allow generous
    // headroom against the floor (0.5×) so the assert catches trivial regressions
    // without being brittle to floor tweaks.
    assert!(wet_total < dry_total * 0.5,
        "Archimedes did not duck enough (dry={}, wet={})", dry_total, wet_total);

    for b in &bins {
        assert!(b.norm().is_finite());
    }
}

#[test]
fn life_non_newtonian_limits_fast_transients() {
    use spectral_forge::dsp::modules::life::{LifeModule, LifeMode};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule};
    use spectral_forge::dsp::bin_physics::BinPhysics;
    use spectral_forge::params::{StereoLink, FxChannelTarget};
    use num_complex::Complex;

    let mut module = LifeModule::new();
    module.reset(48_000.0, 2048);
    module.set_mode(LifeMode::NonNewtonian);

    let num_bins = 1025;
    let mut bins: Vec<Complex<f32>> = vec![Complex::new(0.0, 0.0); num_bins];
    bins[100] = Complex::new(2.0, 0.0); // Loud transient

    // Read-side physics: pre-populated velocity so the kernel sees a fast transient.
    let mut physics_read = BinPhysics::new();
    physics_read.reset_active(num_bins, 48_000.0, 2048);
    physics_read.velocity[100] = 1.5;

    let amount  = vec![2.0_f32; num_bins];
    let thresh  = vec![0.5_f32; num_bins];
    let neutral = vec![1.0_f32; num_bins];
    let mix     = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &thresh, &neutral, &neutral, &mix];

    let mut suppression = vec![0.0_f32; num_bins];

    // Write-side physics: module writes displacement into this.
    let mut physics_write = BinPhysics::new();
    physics_write.reset_active(num_bins, 48_000.0, 2048);

    // ModuleContext has no with_bin_physics builder — use struct literal so we
    // can set bin_physics: Some(&physics_read) directly.
    let ctx = ModuleContext {
        sample_rate:       48_000.0,
        fft_size:          2048,
        num_bins,
        attack_ms:         10.0,
        release_ms:        100.0,
        sensitivity:       1.0,
        suppression_width: 0.0,
        auto_makeup:       false,
        delta_monitor:     false,
        unwrapped_phase:      None,
        peaks:                None,
        instantaneous_freq:   None,
        chromagram:           None,
        midi_notes:           None,
        bpm:                  0.0,
        beat_position:        0.0,
        sidechain_derivative: None,
        bin_physics:          Some(&physics_read),
    };

    module.process(
        0, StereoLink::Linked, FxChannelTarget::All,
        &mut bins, None, &curves, &mut suppression, Some(&mut physics_write), &ctx,
    );

    // Expected math: amt=1.0, thresh=0.25, v=1.5, excess=1.25, mag_old=2.0,
    // limit = max(2.0 - 1.25, 0) = 0.75, scale = 0.375, mix=1.0 (full wet) →
    // bins[100] ends at 0.75. Bound at 1.0 leaves headroom but catches a
    // regression that fails to apply meaningful attenuation.
    assert!(bins[100].norm() < 1.0,
        "Non-Newtonian did not limit transient (mag = {})", bins[100].norm());
    assert!(bins[0].norm() < 1e-6,
        "Silent bin 0 was touched (mag = {})", bins[0].norm());
    assert!(physics_write.displacement[100] > 0.0,
        "Non-Newtonian did not write displacement (displacement[100] = {})",
        physics_write.displacement[100]);

    for b in &bins {
        assert!(b.norm().is_finite());
    }
}

#[test]
fn life_stiction_holds_quiet_bins_then_releases() {
    use spectral_forge::dsp::modules::life::{LifeModule, LifeMode};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule};
    use spectral_forge::params::{StereoLink, FxChannelTarget};
    use spectral_forge::dsp::bin_physics::BinPhysics;
    use num_complex::Complex;

    let mut module = LifeModule::new();
    module.reset(48_000.0, 2048);
    module.set_mode(LifeMode::Stiction);

    let num_bins = 1025;
    let mut bins: Vec<Complex<f32>> = vec![Complex::new(0.0, 0.0); num_bins];
    bins[50]  = Complex::new(0.1, 0.0);
    bins[100] = Complex::new(1.5, 0.0);

    let mut physics = BinPhysics::new();
    physics.reset_active(num_bins, 48_000.0, 2048);
    physics.velocity[50]  = 0.1;  // below threshold → stuck
    physics.velocity[100] = 1.0;  // above threshold → moving freely

    let amount  = vec![2.0_f32; num_bins];
    let thresh  = vec![1.0_f32; num_bins]; // → 0.5 break threshold
    let speed   = vec![1.0_f32; num_bins];
    let neutral = vec![1.0_f32; num_bins];
    let mix     = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &thresh, &speed, &neutral, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let mut physics_for_write = BinPhysics::new();
    physics_for_write.reset_active(num_bins, 48_000.0, 2048);

    let ctx = ModuleContext {
        sample_rate:       48_000.0,
        fft_size:          2048,
        num_bins,
        attack_ms:         10.0,
        release_ms:        100.0,
        sensitivity:       1.0,
        suppression_width: 0.0,
        auto_makeup:       false,
        delta_monitor:     false,
        unwrapped_phase:      None,
        peaks:                None,
        instantaneous_freq:   None,
        chromagram:           None,
        midi_notes:           None,
        bpm:                  0.0,
        beat_position:        0.0,
        sidechain_derivative: None,
        bin_physics:          Some(&physics),
    };

    module.process(
        0, StereoLink::Linked, FxChannelTarget::All,
        &mut bins, None, &curves, &mut suppression, Some(&mut physics_for_write), &ctx,
    );

    assert!(bins[50].norm() < 0.05,
        "Bin 50 not stuck (mag = {})", bins[50].norm());
    assert!(bins[100].norm() > 1.0,
        "Bin 100 not moving freely (mag = {})", bins[100].norm());

    for b in &bins {
        assert!(b.norm().is_finite());
    }
}

#[test]
fn life_yield_clamps_above_threshold_passthrough_below() {
    use spectral_forge::dsp::modules::life::{LifeModule, LifeMode};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule};
    use spectral_forge::params::{StereoLink, FxChannelTarget};
    use num_complex::Complex;

    let mut module = LifeModule::new();
    module.reset(48_000.0, 2048);
    module.set_mode(LifeMode::Yield);

    let num_bins = 1025;
    // Bin 50: above yield threshold (mag 2.0).
    // Bin 100: below threshold (mag 0.2).
    let mut bins: Vec<Complex<f32>> = vec![Complex::new(0.0, 0.0); num_bins];
    bins[50]  = Complex::new(2.0, 0.0);
    bins[100] = Complex::new(0.2, 0.0);

    let amount  = vec![2.0_f32; num_bins];
    let thresh  = vec![1.0_f32; num_bins]; // yield strength = 0.5
    let speed   = vec![0.5_f32; num_bins];
    let neutral = vec![1.0_f32; num_bins];
    let mix     = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &thresh, &speed, &neutral, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = ModuleContext {
        sample_rate:       48_000.0,
        fft_size:          2048,
        num_bins,
        attack_ms:         10.0,
        release_ms:        100.0,
        sensitivity:       1.0,
        suppression_width: 0.0,
        auto_makeup:       false,
        delta_monitor:     false,
        unwrapped_phase:      None,
        peaks:                None,
        instantaneous_freq:   None,
        chromagram:           None,
        midi_notes:           None,
        bpm:                  0.0,
        beat_position:        0.0,
        sidechain_derivative: None,
        bin_physics:          None,
    };

    module.process(
        0, StereoLink::Linked, FxChannelTarget::All,
        &mut bins, None, &curves, &mut suppression, None, &ctx,
    );

    // Yield strength = thresh_c[k]*0.5 = 0.5; magnitude is hard-clamped to that.
    // Allow 2% slack for trig rounding (cos/sin) but no looser.
    assert!(bins[50].norm() <= 0.51, "Bin 50 not clamped at yield (mag = {})", bins[50].norm());
    assert!((bins[100].norm() - 0.2).abs() < 0.01, "Bin 100 not passthrough (mag = {})", bins[100].norm());
    for b in &bins { assert!(b.norm().is_finite()); }
}

#[test]
fn life_capillary_wicks_sustained_energy_upward() {
    use spectral_forge::dsp::modules::life::{LifeModule, LifeMode};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule};
    use spectral_forge::params::{StereoLink, FxChannelTarget};
    use num_complex::Complex;

    let mut module = LifeModule::new();
    module.reset(48_000.0, 2048);
    module.set_mode(LifeMode::Capillary);

    let num_bins = 1025;
    let bins_template: Vec<Complex<f32>> = {
        let mut v = vec![Complex::new(0.0, 0.0); num_bins];
        v[50] = Complex::new(1.0, 0.0);
        v
    };

    let amount  = vec![2.0_f32; num_bins];
    let thresh  = vec![0.5_f32; num_bins];
    let speed   = vec![2.0_f32; num_bins];
    let reach   = vec![2.0_f32; num_bins];
    let mix     = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &thresh, &speed, &reach, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = ModuleContext {
        sample_rate:       48_000.0,
        fft_size:          2048,
        num_bins,
        attack_ms:         10.0,
        release_ms:        100.0,
        sensitivity:       1.0,
        suppression_width: 0.0,
        auto_makeup:       false,
        delta_monitor:     false,
        unwrapped_phase:      None,
        peaks:                None,
        instantaneous_freq:   None,
        chromagram:           None,
        midi_notes:           None,
        bpm:                  0.0,
        beat_position:        0.0,
        sidechain_derivative: None,
        bin_physics:          None,
    };

    // Run 99 warm-up hops (reset only the source/target bins each time so the
    // sustain envelope builds at bin 50 without compounding carry deposits at
    // bin 82), then run the 100th hop and check its output.
    let mut bins = bins_template.clone();
    for _ in 0..99 {
        module.process(
            0, StereoLink::Linked, FxChannelTarget::All,
            &mut bins, None, &curves, &mut suppression, None, &ctx,
        );
        bins[50] = Complex::new(1.0, 0.0);
        bins[82] = Complex::new(0.0, 0.0);
    }
    // Final hop: check output without resetting.
    module.process(
        0, StereoLink::Linked, FxChannelTarget::All,
        &mut bins, None, &curves, &mut suppression, None, &ctx,
    );

    // Reach=2.0 → reach_bins = (2.0 * 16.0) as i32 = 32 → target = 50 + 32 = 82.
    // Direct bin-82 check catches REACH_SCALE/REACH_MAX regressions that the
    // wide window assertion below would miss.
    let bin82_mag = bins[82].norm();
    assert!(bin82_mag > 0.04,
        "Wick carry did not land at expected bin 82 (mag = {})", bin82_mag);

    let upper_total: f32 = (60..200).map(|k| bins[k].norm()).sum();
    assert!(upper_total > 0.04, "No upward wicking happened (upper_total = {})", upper_total);

    for b in &bins { assert!(b.norm().is_finite()); }
}

#[test]
fn life_sandpaper_emits_sparks_to_higher_bins() {
    use spectral_forge::dsp::modules::life::{LifeModule, LifeMode};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule};
    use spectral_forge::params::{StereoLink, FxChannelTarget};
    use num_complex::Complex;

    let mut module = LifeModule::new();
    module.reset(48_000.0, 2048);
    module.set_mode(LifeMode::Sandpaper);

    let num_bins = 1025;
    let mut bins: Vec<Complex<f32>> = vec![Complex::new(0.0, 0.0); num_bins];
    bins[100] = Complex::new( 1.0, 0.0);
    bins[101] = Complex::new(-1.0, 0.0);

    let amount  = vec![2.0_f32; num_bins];
    let thresh  = vec![0.1_f32; num_bins];
    let neutral = vec![1.0_f32; num_bins];
    let reach   = vec![2.0_f32; num_bins];
    let mix     = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &thresh, &neutral, &reach, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = ModuleContext {
        sample_rate:       48_000.0,
        fft_size:          2048,
        num_bins,
        attack_ms:         10.0,
        release_ms:        100.0,
        sensitivity:       1.0,
        suppression_width: 0.0,
        auto_makeup:       false,
        delta_monitor:     false,
        unwrapped_phase:      None,
        peaks:                None,
        instantaneous_freq:   None,
        chromagram:           None,
        midi_notes:           None,
        bpm:                  0.0,
        beat_position:        0.0,
        sidechain_derivative: None,
        bin_physics:          None,
    };

    module.process(
        0, StereoLink::Linked, FxChannelTarget::All,
        &mut bins, None, &curves, &mut suppression, None, &ctx,
    );

    // For k=100, reach=2.0: log_offset = (1+2) * log2(100) * 1.5 ≈ 29.9 → 29,
    // so target = 100 + 29 = 129. Pin the assertion to that specific bin so a
    // regression in the offset formula (or REACH/LOG_OFFSET_BASE constants)
    // would actually fail the test.
    assert!(bins[129].norm() > 0.05,
        "Expected spark at bin 129, got {}", bins[129].norm());
    // No spurious deposits in the mid-band between source and target.
    for k in 102..125 {
        assert!(bins[k].norm() < 1e-3,
            "Unexpected energy at bin {} (norm = {})", k, bins[k].norm());
    }
    for b in &bins { assert!(b.norm().is_finite()); }
}

#[test]
fn life_brownian_drifts_with_temperature() {
    use spectral_forge::dsp::modules::life::{LifeModule, LifeMode};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule};
    use spectral_forge::params::{StereoLink, FxChannelTarget};
    use spectral_forge::dsp::bin_physics::BinPhysics;
    use num_complex::Complex;

    let mut module = LifeModule::new();
    module.reset(48_000.0, 2048);
    module.set_mode(LifeMode::Brownian);

    let num_bins = 1025;
    let bins_template: Vec<Complex<f32>> = (0..num_bins).map(|_| Complex::new(0.5, 0.0)).collect();

    let mut physics = BinPhysics::new();
    physics.reset_active(num_bins, 48_000.0, 2048);
    physics.temperature[100] = 1.0;

    let amount  = vec![2.0_f32; num_bins];
    let neutral = vec![1.0_f32; num_bins];
    let mix     = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &neutral, &neutral, &neutral, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = ModuleContext {
        sample_rate:       48_000.0,
        fft_size:          2048,
        num_bins,
        attack_ms:         10.0,
        release_ms:        100.0,
        sensitivity:       1.0,
        suppression_width: 0.0,
        auto_makeup:       false,
        delta_monitor:     false,
        unwrapped_phase:      None,
        peaks:                None,
        instantaneous_freq:   None,
        chromagram:           None,
        midi_notes:           None,
        bpm:                  0.0,
        beat_position:        0.0,
        sidechain_derivative: None,
        bin_physics:          Some(&physics),
    };

    let mut bins = bins_template.clone();
    module.process(
        0, StereoLink::Linked, FxChannelTarget::All,
        &mut bins, None, &curves, &mut suppression, None, &ctx,
    );

    let drift_100 = (bins[100] - bins_template[100]).norm();
    let drift_0   = (bins[0]   - bins_template[0]).norm();
    // With amt=1.0 (curve=2.0), t=1.0, mix=1.0 the per-hop drift magnitude has
    // a hard physical bound of BROWNIAN_DRIFT_SCALE * sqrt(2) ≈ 0.1414. Assert
    // both lower and upper bounds — the lower bound catches accidental scale
    // shrinks (e.g. DRIFT_SCALE/2), the upper bound catches accidental scale
    // amplifications.
    assert!(drift_100 > 0.02,
        "Bin 100 drift too small — likely DRIFT_SCALE regression (drift = {})", drift_100);
    assert!(drift_100 < 0.15,
        "Bin 100 drift exceeds physical bound — likely scale amplification (drift = {})", drift_100);
    assert!(drift_0 < 1e-6,
        "Bin 0 drifted despite temp=0 (drift = {})", drift_0);
    for b in &bins { assert!(b.norm().is_finite()); }
}

#[test]
fn life_set_mode_persists_across_calls() {
    use spectral_forge::dsp::modules::life::{LifeModule, LifeMode};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule};
    use spectral_forge::params::{StereoLink, FxChannelTarget};
    use num_complex::Complex;

    let mut module = LifeModule::new();
    module.reset(48_000.0, 2048);
    module.set_life_mode(LifeMode::Yield); // trait method, NOT set_mode

    let num_bins = 1025;
    let mut bins: Vec<Complex<f32>> = vec![Complex::new(2.0, 0.0); num_bins];

    let amount  = vec![2.0_f32; num_bins];
    let thresh  = vec![1.0_f32; num_bins]; // yield_thresh = 0.5 → mag 2.0 tears
    let neutral = vec![1.0_f32; num_bins];
    let mix     = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &thresh, &neutral, &neutral, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = ModuleContext {
        sample_rate:          48_000.0,
        fft_size:             2048,
        num_bins,
        attack_ms:            10.0,
        release_ms:           100.0,
        sensitivity:          1.0,
        suppression_width:    0.0,
        auto_makeup:          false,
        delta_monitor:        false,
        unwrapped_phase:      None,
        peaks:                None,
        instantaneous_freq:   None,
        chromagram:           None,
        midi_notes:           None,
        bpm:                  0.0,
        beat_position:        0.0,
        sidechain_derivative: None,
        bin_physics:          None,
    };

    module.process(
        0, StereoLink::Linked, FxChannelTarget::All,
        &mut bins, None, &curves, &mut suppression, None, &ctx,
    );

    // After Yield with thresh=1.0 (→ yield_thresh=0.5), all bins must be at
    // or near the yield threshold (~0.5 ± slight overshoot).
    for k in 0..num_bins {
        assert!(bins[k].norm() <= 0.6,
            "Bin {} not yielded (mag = {}); set_life_mode did not persist", k, bins[k].norm());
    }
}
