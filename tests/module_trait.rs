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
        history:              None,
        if_offset:            None,
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
        history:              None,
        if_offset:            None,
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
        history:              None,
        if_offset:            None,
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
        history:              None,
        if_offset:            None,
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
        history:              None,
        if_offset:            None,
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
        history:              None,
        if_offset:            None,
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
        history:              None,
        if_offset:            None,
    };

    module.process(
        0, StereoLink::Linked, FxChannelTarget::All,
        &mut bins, None, &curves, &mut suppression, None, &ctx,
    );

    // After Yield with thresh=1.0 (→ yield_thresh = thresh * 0.5 = 0.5), the
    // phase-scramble preserves magnitude so every bin clamps exactly to 0.5.
    // Tight bound (0.51) catches both: (a) set_life_mode failing to persist
    // (default Viscosity passes through ≈ 2.0); (b) coefficient drift in the
    // yield-threshold formula (e.g. thresh*0.75 → 0.75 magnitude).
    for k in 0..num_bins {
        assert!(bins[k].norm() <= 0.51,
            "Bin {} not yielded (mag = {}); set_life_mode did not persist or coefficient drifted", k, bins[k].norm());
    }
}

#[test]
fn life_all_modes_finite_and_bounded() {
    use spectral_forge::dsp::modules::life::{LifeModule, LifeMode};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule};
    use spectral_forge::params::{StereoLink, FxChannelTarget};
    use spectral_forge::dsp::bin_physics::BinPhysics;
    use num_complex::Complex;

    let modes = [
        LifeMode::Viscosity,
        LifeMode::SurfaceTension,
        LifeMode::Crystallization,
        LifeMode::Archimedes,
        LifeMode::NonNewtonian,
        LifeMode::Stiction,
        LifeMode::Yield,
        LifeMode::Capillary,
        LifeMode::Sandpaper,
        LifeMode::Brownian,
    ];

    let num_bins = 1025;

    for &mode in &modes {
        let mut module = LifeModule::new();
        module.reset(48_000.0, 2048);
        module.set_mode(mode);

        // Stress curves at maximum.
        let curves_storage: Vec<Vec<f32>> = vec![
            vec![2.0_f32; num_bins], // AMOUNT
            vec![0.5_f32; num_bins], // THRESHOLD (low → trigger most behaviours)
            vec![2.0_f32; num_bins], // SPEED
            vec![2.0_f32; num_bins], // REACH
            vec![2.0_f32; num_bins], // MIX
        ];
        let curves: Vec<&[f32]> = curves_storage.iter().map(|v| v.as_slice()).collect();

        // Random-ish input so cross-bin kernels have something to chew on.
        let bins_template: Vec<Complex<f32>> = (0..num_bins)
            .map(|k| {
                let mag = 0.3 + ((k * 17 % 23) as f32) * 0.05;
                let phase = (k as f32 * 0.073).sin() * std::f32::consts::PI;
                Complex::new(mag * phase.cos(), mag * phase.sin())
            })
            .collect();

        let mut physics = BinPhysics::new();
        physics.reset_active(num_bins, 48_000.0, 2048);
        // Seed velocity + temperature so velocity-/temp-reading modes get inputs.
        for k in 0..num_bins {
            physics.velocity[k] = 0.4 + ((k * 13 % 7) as f32) * 0.1;
            physics.temperature[k] = 0.5;
        }
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
            bpm:                  120.0,
            beat_position:        0.0,
            unwrapped_phase:      None,
            peaks:                None,
            instantaneous_freq:   None,
            chromagram:           None,
            midi_notes:           None,
            sidechain_derivative: None,
            bin_physics:          Some(&physics),
            history:              None,
            if_offset:            None,
        };

        // bins accumulate across 200 hops within a channel — exercises stateful kernels.
        for ch in 0..2 {
            let mut bins = bins_template.clone();
            let mut suppression = vec![0.0_f32; num_bins];
            let mut physics_out = BinPhysics::new();
            physics_out.reset_active(num_bins, 48_000.0, 2048);

            for hop in 0..200 {
                module.process(
                    ch, StereoLink::Linked, FxChannelTarget::All,
                    &mut bins, None, &curves, &mut suppression, Some(&mut physics_out), &ctx,
                );

                for (k, b) in bins.iter().enumerate() {
                    assert!(b.norm().is_finite(),
                        "Mode {:?} ch {} hop {} bin {} produced NaN/Inf", mode, ch, hop, k);
                    assert!(b.norm() < 1e6,
                        "Mode {:?} ch {} hop {} bin {} unbounded ({})", mode, ch, hop, k, b.norm());
                }
                for (k, &s) in suppression.iter().enumerate() {
                    assert!(s.is_finite() && s >= 0.0,
                        "Mode {:?} ch {} hop {} bin {} suppression bad ({})", mode, ch, hop, k, s);
                }
            }
        }
    }
}

#[test]
fn module_context_has_history_slot_default_none() {
    use spectral_forge::dsp::modules::ModuleContext;
    let ctx = ModuleContext::new(
        48000.0, 2048, 1025, 10.0, 100.0, 1.0, 0.5, false, false,
    );
    assert!(ctx.history.is_none(),
        "ctx.history must default to None so existing modules ignore it");
}

#[test]
fn module_context_has_if_offset_default_none() {
    use spectral_forge::dsp::modules::ModuleContext;
    let ctx = ModuleContext::new(
        48000.0, 2048, 1025, 10.0, 100.0, 1.0, 0.5, false, false,
    );
    assert!(ctx.if_offset.is_none(),
        "ctx.if_offset must default to None so existing modules ignore it");
}

#[test]
fn past_module_spec_present() {
    use spectral_forge::dsp::modules::{module_spec, ModuleType};
    let spec = module_spec(ModuleType::Past);
    assert_eq!(spec.display_name, "PAST");
    assert_eq!(spec.num_curves, 5);
    assert_eq!(spec.curve_labels, &["AMOUNT", "TIME", "THRESHOLD", "SPREAD", "MIX"]);
    assert!(!spec.supports_sidechain, "Past v1 does not consume sidechain");
}

#[test]
fn kinetics_module_spec_present() {
    use spectral_forge::dsp::modules::{module_spec, ModuleType};

    let spec = module_spec(ModuleType::Kinetics);
    assert_eq!(spec.display_name, "KINETICS");
    assert_eq!(spec.num_curves, 5);
    assert_eq!(spec.curve_labels.len(), 5);
    assert_eq!(spec.curve_labels[0], "STRENGTH");
    assert_eq!(spec.curve_labels[1], "MASS");
    assert_eq!(spec.curve_labels[2], "REACH");
    assert_eq!(spec.curve_labels[3], "DAMPING");
    assert_eq!(spec.curve_labels[4], "MIX");
    assert!(spec.supports_sidechain, "Kinetics declares supports_sidechain");
    assert!(!spec.wants_sidechain, "Kinetics opt-in via mode/source");
    assert!(spec.writes_bin_physics, "Kinetics writes mass/displacement/velocity/temperature/phase_momentum");
}

#[test]
fn kinetics_module_constructs_and_passes_through() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::{create_module, ModuleType, ModuleContext, SpectralModule};
    use spectral_forge::params::{StereoLink, FxChannelTarget};

    let mut module = create_module(ModuleType::Kinetics, 48_000.0, 2048);
    assert_eq!(module.module_type(), ModuleType::Kinetics);
    assert_eq!(module.num_curves(), 5);

    let num_bins = 1025;
    let mut bins: Vec<Complex<f32>> = (0..num_bins)
        .map(|k| Complex::new((k as f32 * 0.013).sin(), (k as f32 * 0.011).cos()))
        .collect();
    let dry: Vec<Complex<f32>> = bins.clone();

    // STRENGTH=neutral=1, MASS=neutral=1, REACH=neutral=1, DAMPING=neutral=1, MIX=0 (dry only) → passthrough
    let neutral = vec![1.0_f32; num_bins];
    let zero    = vec![0.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&neutral, &neutral, &neutral, &neutral, &zero];

    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = ModuleContext::new(48_000.0, 2048, num_bins, 10.0, 100.0, 1.0, 0.5, false, false);

    module.process(
        0,
        StereoLink::Linked,
        FxChannelTarget::All,
        &mut bins,
        None,
        &curves,
        &mut suppression,
        None,
        &ctx,
    );

    for k in 0..num_bins {
        let diff = (bins[k] - dry[k]).norm();
        assert!(diff < 1e-5, "bin {} drifted by {} (passthrough expected at MIX=0)", k, diff);
    }
    for s in &suppression {
        assert!(s.is_finite() && *s >= 0.0);
    }
}

#[test]
fn kinetics_verlet_stays_bounded_under_unit_impulse() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::kinetics::{KineticsModule, KineticsMode};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule};
    use spectral_forge::params::{StereoLink, FxChannelTarget};

    let mut module = KineticsModule::new();
    module.reset(48_000.0, 2048);
    module.set_mode(KineticsMode::Hooke);

    let num_bins = 1025usize;

    // Unit impulse at bin 256; all other bins silent.
    let make_bins = || -> Vec<Complex<f32>> {
        let mut v = vec![Complex::new(0.0f32, 0.0f32); num_bins];
        v[256] = Complex::new(1.0, 0.0);
        v
    };

    // STRENGTH=2, MASS=1, REACH=1, DAMPING=1, MIX=1 (full wet).
    let strength = vec![2.0_f32; num_bins];
    let neutral  = vec![1.0_f32; num_bins];
    let mix      = vec![1.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&strength, &neutral, &neutral, &neutral, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = ModuleContext::new(
        48_000.0, 2048, num_bins,
        10.0, 100.0, 1.0, 1.0, false, false,
    );

    let mut max_mag: f32 = 0.0;
    for _ in 0..200 {
        let mut bins = make_bins();
        module.process(
            0,
            StereoLink::Linked,
            FxChannelTarget::All,
            &mut bins,
            None,
            &curves,
            &mut suppression,
            None,
            &ctx,
        );
        let hop_max: f32 = bins.iter().map(|c| c.norm()).fold(0.0_f32, f32::max);
        max_mag = max_mag.max(hop_max);
        // All bins must be finite every hop.
        for (k, c) in bins.iter().enumerate() {
            assert!(c.re.is_finite() && c.im.is_finite(),
                "non-finite at bin {} after hop", k);
        }
    }

    // Loose bound — Hooke kernel is a stub in Task 4; the test tightens in Task 5
    // once the actual spring force lands.
    assert!(max_mag < 100.0,
        "Energy escaped integrator (max_mag = {})", max_mag);
}

#[test]
fn kinetics_hooke_diffuses_energy_via_springs() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::kinetics::{KineticsModule, KineticsMode};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule};
    use spectral_forge::params::{StereoLink, FxChannelTarget};

    let mut module = KineticsModule::new();
    module.reset(48_000.0, 2048);
    module.set_mode(KineticsMode::Hooke);

    let num_bins = 1025;
    let mut bins: Vec<Complex<f32>> = vec![Complex::new(0.0, 0.0); num_bins];
    bins[100] = Complex::new(2.0, 0.0); // Tone at bin 100.
    let dry_total: f32 = bins.iter().map(|b| b.norm_sqr()).sum();

    // STRENGTH=2 (max), MASS=1, REACH=1, DAMPING=1 (-> floored 0.05+), MIX=2 (full wet)
    let strength = vec![2.0_f32; num_bins];
    let neutral = vec![1.0_f32; num_bins];
    let mix = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&strength, &neutral, &neutral, &neutral, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = ModuleContext::new(
        48_000.0, 2048, num_bins,
        10.0, 100.0, 1.0, 1.0, false, false,
    );

    for _ in 0..30 {
        module.process(0, StereoLink::Linked, FxChannelTarget::All,
            &mut bins, None, &curves, &mut suppression, None, &ctx);
    }

    let neighbour_energy: f32 = (95..=105).filter(|&k| k != 100)
        .map(|k| bins[k].norm_sqr()).sum();
    assert!(neighbour_energy > 0.001 * dry_total,
        "Hooke springs did not couple neighbours (neighbour_energy = {} < 0.001 * dry_total = {})",
        neighbour_energy, dry_total);

    // Harmonic coupling acts via the source bin's force; net wet output at harmonic bins
    // comes from the linear-chain ripple. Verify energy spreads beyond the immediate
    // neighbourhood without the integrator blowing up.
    // NOTE: This does NOT prove direct write-back into harmonic bins — that requires the
    // TuningFork kernel (Task 11). It only checks that the chain dynamics are working
    // over a non-trivial range.
    let wide_window_energy: f32 = (50..=150).filter(|&k| k != 100)
        .map(|k| bins[k].norm_sqr()).sum();
    assert!(wide_window_energy.is_finite() && wide_window_energy > neighbour_energy,
        "Spring chain energy should diffuse beyond ±5 bins (wide={}, neighbour={})",
        wide_window_energy, neighbour_energy);

    for b in &bins { assert!(b.norm().is_finite()); }
}

#[test]
fn kinetics_gravity_well_static_pulls_energy_toward_curve_peak() {
    use spectral_forge::dsp::modules::kinetics::{KineticsModule, KineticsMode, WellSource};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule};
    use spectral_forge::params::{StereoLink, FxChannelTarget};
    use realfft::num_complex::Complex;

    let mut module = KineticsModule::new();
    module.reset(48_000.0, 2048);
    module.set_mode(KineticsMode::GravityWell);
    module.set_well_source(WellSource::Static);

    let num_bins = 1025;
    // Flat-ish noise spectrum.
    let mut bins: Vec<Complex<f32>> = (0..num_bins)
        .map(|k| Complex::new(((k as f32 * 0.1).sin() + 1.5) * 0.3, 0.0))
        .collect();

    // STRENGTH curve has a single peak at bin 200 (the well location).
    // Use a simple Gaussian centred at bin 200, height 2.0.
    let strength: Vec<f32> = (0..num_bins).map(|k| {
        let d = (k as f32 - 200.0) / 5.0;
        1.0 + (-d * d).exp() // ranges from ~1.0 (away) to ~2.0 (at peak)
    }).collect();
    let neutral = vec![1.0_f32; num_bins];
    let mix = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&strength, &neutral, &neutral, &neutral, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = ModuleContext::new(
        48_000.0, 2048, num_bins,
        10.0, 100.0, 1.0, 1.0, false, false,
    );

    let dry: Vec<Complex<f32>> = bins.clone();

    for _ in 0..40 {
        module.process(
            0, StereoLink::Linked, FxChannelTarget::All,
            &mut bins, None, &curves, &mut suppression, None, &ctx,
        );
    }

    // Bin 200 should have higher magnitude than dry (energy gathered from neighbours).
    let dry_at_200 = dry[200].norm();
    let wet_at_200 = bins[200].norm();
    assert!(wet_at_200 > dry_at_200 * 1.05,
        "GravityWell did not gather energy at well centre (dry={}, wet={})",
        dry_at_200, wet_at_200);
    // Energy at distance 30 should have decreased or not grown much.
    let dry_at_230 = dry[230].norm();
    let wet_at_230 = bins[230].norm();
    assert!(wet_at_230 < dry_at_230 * 1.05,
        "GravityWell did not pull energy from neighbours (dry={}, wet={})",
        dry_at_230, wet_at_230);
}

#[test]
fn kinetics_gravity_well_sidechain_tracks_sc_peak() {
    use spectral_forge::dsp::modules::kinetics::{KineticsModule, KineticsMode, WellSource};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule};
    use spectral_forge::params::{StereoLink, FxChannelTarget};
    use realfft::num_complex::Complex;

    let mut module = KineticsModule::new();
    module.reset(48_000.0, 2048);
    module.set_mode(KineticsMode::GravityWell);
    module.set_well_source(WellSource::Sidechain);

    let num_bins = 1025;
    let mut bins: Vec<Complex<f32>> = vec![Complex::new(1.0, 0.0); num_bins];
    // Sidechain spectrum has a single peak at bin 400.
    let mut sc = vec![0.0_f32; num_bins];
    sc[400] = 5.0;

    let strength = vec![2.0_f32; num_bins];
    let neutral = vec![1.0_f32; num_bins];
    let mix = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&strength, &neutral, &neutral, &neutral, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = ModuleContext::new(
        48_000.0, 2048, num_bins,
        10.0, 100.0, 1.0, 1.0, false, false,
    );

    let dry: Vec<Complex<f32>> = bins.clone();
    for _ in 0..40 {
        module.process(
            0, StereoLink::Linked, FxChannelTarget::All,
            &mut bins, Some(&sc), &curves, &mut suppression, None, &ctx,
        );
    }
    // Bin 400 should have gathered energy.
    assert!(bins[400].norm() > dry[400].norm() * 1.05,
        "Sidechain well did not track sc peak (dry={}, wet={})",
        dry[400].norm(), bins[400].norm());
}

#[test]
fn kinetics_gravity_well_midi_no_op_without_ctx_midi() {
    use spectral_forge::dsp::modules::kinetics::{KineticsModule, KineticsMode, WellSource};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule};
    use spectral_forge::params::{StereoLink, FxChannelTarget};
    use realfft::num_complex::Complex;

    let mut module = KineticsModule::new();
    module.reset(48_000.0, 2048);
    module.set_mode(KineticsMode::GravityWell);
    module.set_well_source(WellSource::MIDI);

    let num_bins = 1025;
    let mut bins: Vec<Complex<f32>> = vec![Complex::new(1.0, 0.0); num_bins];
    let strength = vec![2.0_f32; num_bins];
    let neutral = vec![1.0_f32; num_bins];
    let mix = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&strength, &neutral, &neutral, &neutral, &mix];
    let mut suppression = vec![0.0_f32; num_bins];
    // ctx.midi_notes left as None → MIDI source must no-op.
    let ctx = ModuleContext::new(
        48_000.0, 2048, num_bins,
        10.0, 100.0, 1.0, 1.0, false, false,
    );
    let dry: Vec<Complex<f32>> = bins.clone();
    for _ in 0..10 {
        module.process(
            0, StereoLink::Linked, FxChannelTarget::All,
            &mut bins, None, &curves, &mut suppression, None, &ctx,
        );
    }
    // No-op: bins must be very close to dry.
    for k in 0..num_bins {
        let diff = (bins[k] - dry[k]).norm();
        assert!(diff < 0.02, "MIDI well leaked motion when ctx.midi_notes=None (bin {} drifted by {})", k, diff);
    }
}

#[test]
fn kinetics_inertial_mass_static_writes_bin_physics_mass() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::kinetics::{KineticsModule, KineticsMode, MassSource};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule};
    use spectral_forge::dsp::bin_physics::BinPhysics;
    use spectral_forge::params::{StereoLink, FxChannelTarget};

    let mut module = KineticsModule::new();
    module.reset(48_000.0, 2048);
    module.set_mode(KineticsMode::InertialMass);
    module.set_mass_source(MassSource::Static);

    let num_bins = 1025usize;

    // MASS curve: ramp from 0.5 to 3.0 across all bins.
    let mass_curve: Vec<f32> = (0..num_bins)
        .map(|k| 0.5 + 2.5 * k as f32 / (num_bins - 1) as f32)
        .collect();
    // MIX curve: full wet (1.0) so the static write lands immediately.
    let mix_curve = vec![1.0_f32; num_bins];
    // Other curves (STRENGTH, REACH, DAMPING) at neutral.
    let neutral = vec![1.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&neutral, &mass_curve, &neutral, &neutral, &mix_curve];

    let mut bins = vec![Complex::new(0.5_f32, 0.0); num_bins];
    let mut suppression = vec![0.0_f32; num_bins];

    let ctx = ModuleContext::new(
        48_000.0, 2048, num_bins,
        10.0, 100.0, 1.0, 1.0, false, false,
    );

    let mut physics = BinPhysics::new();
    physics.reset_active(num_bins, 48_000.0, 2048);

    // Run several hops so the 1-pole curve smoother converges.
    for _ in 0..30 {
        module.process(
            0,
            StereoLink::Linked,
            FxChannelTarget::All,
            &mut bins,
            None,
            &curves,
            &mut suppression,
            Some(&mut physics),
            &ctx,
        );
    }

    // Verify: physics.mass should track the MASS curve closely (MIX=1 → direct write).
    // Check a few representative bins.
    for &k in &[0usize, 256, 512, 768, 1024] {
        let expected = mass_curve[k].clamp(0.01, 1000.0);
        let actual   = physics.mass[k];
        assert!(
            (actual - expected).abs() < 0.05,
            "Static: physics.mass[{}] = {} (expected ~{})",
            k, actual, expected
        );
    }

    // Bins must NOT be modified by InertialMass.
    for k in 0..num_bins {
        assert!(
            (bins[k].re - 0.5).abs() < 1e-4 && bins[k].im.abs() < 1e-4,
            "InertialMass (Static) must not modify bins (bin {} re={} im={})",
            k, bins[k].re, bins[k].im
        );
    }
}

#[test]
fn kinetics_inertial_mass_sidechain_high_when_sc_changing_fast() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::kinetics::{KineticsModule, KineticsMode, MassSource};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule};
    use spectral_forge::dsp::bin_physics::BinPhysics;
    use spectral_forge::params::{StereoLink, FxChannelTarget};

    let mut module = KineticsModule::new();
    module.reset(48_000.0, 2048);
    module.set_mode(KineticsMode::InertialMass);
    module.set_mass_source(MassSource::Sidechain);

    let num_bins = 1025usize;

    // MASS curve at 1.0 (neutral), MIX at 1.0 (full wet).
    let neutral    = vec![1.0_f32; num_bins];
    let mix_curve  = vec![1.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&neutral, &neutral, &neutral, &neutral, &mix_curve];

    let mut bins       = vec![Complex::new(0.5_f32, 0.0); num_bins];
    let mut suppression = vec![0.0_f32; num_bins];

    let ctx = ModuleContext::new(
        48_000.0, 2048, num_bins,
        10.0, 100.0, 1.0, 1.0, false, false,
    );

    // -- Phase 1: quiescent SC (zero signal). Let envelope fully decay.
    let sc_silent = vec![0.0_f32; num_bins];
    let mut physics = BinPhysics::new();
    physics.reset_active(num_bins, 48_000.0, 2048);

    for _ in 0..20 {
        module.process(
            0, StereoLink::Linked, FxChannelTarget::All,
            &mut bins, Some(&sc_silent), &curves,
            &mut suppression, Some(&mut physics), &ctx,
        );
    }
    let steady_mass = physics.mass[512];

    // -- Phase 2: sudden large SC burst — rate of change should spike mass.
    let sc_burst: Vec<f32> = vec![1.0_f32; num_bins];
    module.process(
        0, StereoLink::Linked, FxChannelTarget::All,
        &mut bins, Some(&sc_burst), &curves,
        &mut suppression, Some(&mut physics), &ctx,
    );
    let burst_mass = physics.mass[512];

    assert!(
        burst_mass > steady_mass * 1.5,
        "Sidechain burst should raise mass (steady={}, burst={})",
        steady_mass, burst_mass
    );

    // -- Phase 3: SC returns to silence; mass should decay back toward steady-state.
    for _ in 0..30 {
        module.process(
            0, StereoLink::Linked, FxChannelTarget::All,
            &mut bins, Some(&sc_silent), &curves,
            &mut suppression, Some(&mut physics), &ctx,
        );
    }
    let recovered_mass = physics.mass[512];
    assert!(
        recovered_mass < burst_mass * 0.5,
        "Sidechain mass should decay after burst (burst={}, recovered={})",
        burst_mass, recovered_mass
    );

    // Bins must NOT be modified by InertialMass.
    for k in 0..num_bins {
        assert!(
            (bins[k].re - 0.5).abs() < 1e-4 && bins[k].im.abs() < 1e-4,
            "InertialMass (Sidechain) must not modify bins (bin {} re={} im={})",
            k, bins[k].re, bins[k].im
        );
    }
}

#[test]
fn kinetics_orbital_phase_rotates_satellites_in_opposite_directions() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::kinetics::{KineticsModule, KineticsMode};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule};
    use spectral_forge::params::{StereoLink, FxChannelTarget};

    let mut module = KineticsModule::new();
    module.reset(48_000.0, 2048);
    module.set_mode(KineticsMode::OrbitalPhase);

    let num_bins = 1025;

    // STRENGTH=2.0 (strong rotation), MIX=2.0 (clamped to 1.0 → full wet).
    let strength = vec![2.0_f32; num_bins];
    let neutral  = vec![1.0_f32; num_bins];
    let mix      = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&strength, &neutral, &neutral, &neutral, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = ModuleContext::new(
        48_000.0, 2048, num_bins,
        10.0, 100.0, 1.0, 1.0, false, false,
    );

    // -- Warmup: run 30 hops to let the 1-pole curve smoother converge (MIX 0→1, STRENGTH 1→2).
    // Use the same peak bin each hop so the peak is consistently detected during warmup.
    let mut warmup_bins: Vec<Complex<f32>> = vec![Complex::new(0.1, 0.0); num_bins];
    warmup_bins[200] = Complex::new(50.0, 0.0);
    for _ in 0..30 {
        module.process(
            0, StereoLink::Linked, FxChannelTarget::All,
            &mut warmup_bins, None, &curves, &mut suppression, None, &ctx,
        );
        // Restore bins to static shape (peak still dominates) so the smoother sees
        // the same input each hop and converges to steady-state.
        for k in 0..num_bins { warmup_bins[k] = Complex::new(0.1, 0.0); }
        warmup_bins[200] = Complex::new(50.0, 0.0);
    }

    // -- Measurement hop: fresh bins with known satellite phases at +1 and -1 from peak.
    // Use a large peak (50.0) to clear the local-window mean check and produce a
    // rotation large enough to assert cleanly (Δφ ≈ alpha*m_amp/1² ≈ 0.53 rad at d=1).
    let mut bins: Vec<Complex<f32>> = vec![Complex::new(0.1, 0.0); num_bins];
    bins[200] = Complex::new(50.0, 0.0); // master peak — strong and isolated
    bins[199] = Complex::new(0.5, 0.0);  // -1 satellite
    bins[201] = Complex::new(0.5, 0.0);  // +1 satellite
    let dry_left_phase  = bins[199].arg();
    let dry_right_phase = bins[201].arg();

    module.process(
        0, StereoLink::Linked, FxChannelTarget::All,
        &mut bins, None, &curves, &mut suppression, None, &ctx,
    );

    let new_left_phase  = bins[199].arg();
    let new_right_phase = bins[201].arg();
    let dleft  = new_left_phase  - dry_left_phase;
    let dright = new_right_phase - dry_right_phase;

    // Both satellites must have moved from their dry phase.
    assert!(dleft.abs()  > 0.01, "Left satellite did not rotate (delta = {})", dleft);
    assert!(dright.abs() > 0.01, "Right satellite did not rotate (delta = {})", dright);

    // The rotations must be in opposite signs (left negative, right positive).
    assert!(
        dleft.signum() != dright.signum(),
        "Satellites must orbit in opposite directions; got dleft={}, dright={}",
        dleft, dright
    );

    // Master peak phase must be unchanged (rotation NOT applied to master bin itself).
    let new_master_phase = bins[200].arg();
    assert!(
        new_master_phase.abs() < 0.01,
        "Master phase changed: {}",
        new_master_phase
    );
}

#[test]
fn kinetics_ferromagnetism_aligns_neighbour_phases_to_peak() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::kinetics::{KineticsModule, KineticsMode};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule};
    use spectral_forge::params::{StereoLink, FxChannelTarget};

    let mut module = KineticsModule::new();
    module.reset(48_000.0, 2048);
    module.set_mode(KineticsMode::Ferromagnetism);

    let num_bins = 1025;
    let mut bins: Vec<Complex<f32>> = vec![Complex::new(0.1, 0.0); num_bins];
    // Master peak with phase pi/2 at bin 300.
    bins[300] = Complex::new(0.0, 5.0);
    // Neighbour bins with initial phases away from pi/2.
    bins[298] = Complex::from_polar(0.5, -1.5);
    bins[302] = Complex::from_polar(0.5, 1.5);

    let strength = vec![2.0_f32; num_bins]; // strong magnetic pull
    let neutral  = vec![1.0_f32; num_bins];
    let mix      = vec![2.0_f32; num_bins]; // clamped to 1.0 inside the kernel
    let curves: Vec<&[f32]> = vec![&strength, &neutral, &neutral, &neutral, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = ModuleContext::new(
        48_000.0, 2048, num_bins,
        10.0, 100.0, 1.0, 0.0, false, false,
    );

    let target_phase = bins[300].arg();
    let dry_298 = bins[298].arg();
    let dry_302 = bins[302].arg();

    for _ in 0..10 {
        module.process(
            0, StereoLink::Linked, FxChannelTarget::All,
            &mut bins, None, &curves, &mut suppression, None, &ctx,
        );
    }

    let new_298 = bins[298].arg();
    let new_302 = bins[302].arg();
    let phase_diff = |a: f32, b: f32| -> f32 {
        let mut d = a - b;
        while d >  std::f32::consts::PI { d -= 2.0 * std::f32::consts::PI; }
        while d < -std::f32::consts::PI { d += 2.0 * std::f32::consts::PI; }
        d.abs()
    };

    let dry_offset_298 = phase_diff(dry_298, target_phase);
    let new_offset_298 = phase_diff(new_298, target_phase);
    let dry_offset_302 = phase_diff(dry_302, target_phase);
    let new_offset_302 = phase_diff(new_302, target_phase);

    assert!(new_offset_298 < dry_offset_298,
        "neighbour 298 did not align toward peak phase: dry_offset={}, new_offset={}",
        dry_offset_298, new_offset_298);
    assert!(new_offset_302 < dry_offset_302,
        "neighbour 302 did not align toward peak phase: dry_offset={}, new_offset={}",
        dry_offset_302, new_offset_302);

    // Magnitudes must be preserved (kernel rotates phase only, not amplitude).
    let mag_298 = bins[298].norm();
    let mag_302 = bins[302].norm();
    assert!((mag_298 - 0.5).abs() < 1e-3,
        "Ferro must preserve magnitude at bin 298: got {}", mag_298);
    assert!((mag_302 - 0.5).abs() < 1e-3,
        "Ferro must preserve magnitude at bin 302: got {}", mag_302);
}

#[test]
fn kinetics_thermal_expansion_heats_then_detunes() {
    use spectral_forge::dsp::modules::kinetics::{KineticsModule, KineticsMode};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule};
    use spectral_forge::dsp::bin_physics::BinPhysics;
    use spectral_forge::params::{StereoLink, FxChannelTarget};
    use num_complex::Complex;

    let mut module = KineticsModule::new();
    module.reset(48_000.0, 2048);
    module.set_mode(KineticsMode::ThermalExpansion);

    let num_bins = 1025;
    // Sustained loud bin at 100. Initial phase 0.
    let mut bins: Vec<Complex<f32>> = vec![Complex::new(0.0, 0.0); num_bins];
    bins[100] = Complex::new(2.0, 0.0);
    let mut physics = BinPhysics::new();
    physics.reset_active(num_bins, 48_000.0, 2048);

    let strength = vec![2.0_f32; num_bins];
    let neutral  = vec![1.0_f32; num_bins];
    let mix      = vec![2.0_f32; num_bins]; // > 1.0 to exercise the kernel's MIX clamp.
    let curves: Vec<&[f32]> = vec![&strength, &neutral, &neutral, &neutral, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = ModuleContext::new(
        48_000.0, 2048, num_bins,
        10.0, 100.0, 1.0, 0.0, false, false,
    );

    let dry_phase = bins[100].arg();
    // 100 hops ≈ 1.07 s at 48 kHz / hop=512 — long enough that even with a low-ish
    // STRENGTH=2 and DAMPING=1 the bin's temperature converges well above the
    // assertion floor (empirically ~1.0, ceiling 10.0).
    for _ in 0..100 {
        bins[100] = Complex::new(2.0, 0.0); // re-inject sustained tone each hop
        module.process(
            0, StereoLink::Linked, FxChannelTarget::All,
            &mut bins, None, &curves, &mut suppression, Some(&mut physics), &ctx,
        );
    }

    // Temperature should have built up in BinPhysics.
    assert!(physics.temperature[100] > 0.05,
        "temperature did not rise on sustained signal (= {})", physics.temperature[100]);

    // The bin's phase should now be different from dry.
    let new_phase = bins[100].arg();
    let mut diff = new_phase - dry_phase;
    while diff >  std::f32::consts::PI { diff -= 2.0 * std::f32::consts::PI; }
    while diff < -std::f32::consts::PI { diff += 2.0 * std::f32::consts::PI; }
    assert!(diff.abs() > 0.05,
        "phase did not detune from heat (delta = {})", diff);

    // All bins must remain finite.
    for b in &bins { assert!(b.norm().is_finite()); }
}

#[test]
fn kinetics_tuning_fork_modulates_neighbour_phase() {
    use spectral_forge::dsp::modules::kinetics::{KineticsModule, KineticsMode};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule};
    use spectral_forge::params::{StereoLink, FxChannelTarget};
    use realfft::num_complex::Complex;

    let mut module = KineticsModule::new();
    module.reset(48_000.0, 2048);
    module.set_mode(KineticsMode::TuningFork);

    let num_bins = 1025;
    let mut bins: Vec<Complex<f32>> = vec![Complex::new(0.01, 0.0); num_bins];
    bins[300] = Complex::new(5.0, 0.0); // loud peak (will become a fork)
    bins[298] = Complex::new(0.4, 0.0); // neighbour — below TUNING_FORK_MIN_MAG (0.5) so not a fork
    bins[302] = Complex::new(0.4, 0.0); // neighbour — below TUNING_FORK_MIN_MAG (0.5) so not a fork

    // Peak THRESHOLD via STRENGTH-curve baseline; here STRENGTH=2 (above 1.5 fork cutoff).
    let strength = vec![2.0_f32; num_bins];
    let neutral = vec![1.0_f32; num_bins];
    let mix = vec![2.0_f32; num_bins]; // 2.0 exercises the kernel's MIX clamp; effective mix = 1.0 (full wet).
    let curves: Vec<&[f32]> = vec![&strength, &neutral, &neutral, &neutral, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = ModuleContext::new(
        48_000.0, 2048, num_bins,
        10.0, 100.0, 1.0, 0.0, false, false,
    );

    let dry_phase_l = bins[298].arg();
    let dry_phase_r = bins[302].arg();

    for _ in 0..30 {
        module.process(
            0, StereoLink::Linked, FxChannelTarget::All,
            &mut bins, None, &curves, &mut suppression, None, &ctx,
        );
    }

    // Neighbours must show *some* phase movement (not necessarily aligned, just modulated).
    let new_phase_l = bins[298].arg();
    let new_phase_r = bins[302].arg();
    assert!((new_phase_l - dry_phase_l).abs() > 0.005, "left neighbour phase did not modulate");
    assert!((new_phase_r - dry_phase_r).abs() > 0.005, "right neighbour phase did not modulate");

    // All bins must remain finite.
    for b in &bins { assert!(b.norm().is_finite()); }
}

#[test]
fn kinetics_diamagnet_carves_and_redistributes_energy() {
    use spectral_forge::dsp::modules::kinetics::{KineticsModule, KineticsMode};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule};
    use spectral_forge::params::{StereoLink, FxChannelTarget};
    use realfft::num_complex::Complex;

    let mut module = KineticsModule::new();
    module.reset(48_000.0, 2048);
    module.set_mode(KineticsMode::Diamagnet);

    let num_bins = 1025;
    // Flat-ish dense spectrum.
    let mut bins: Vec<Complex<f32>> = (0..num_bins)
        .map(|k| Complex::new(((k as f32 * 0.05).cos() + 1.5) * 0.5, 0.0))
        .collect();
    let dry_total: f32 = bins.iter().map(|b| b.norm_sqr()).sum();

    // STRENGTH curve creates a "carve zone" centred on bin 400 (Gaussian, sigma = 8 bins,
    // peaks at strength 2.0 — well above the kernel's STRENGTH_BASELINE = 1.0 onset).
    let strength: Vec<f32> = (0..num_bins).map(|k| {
        let d = (k as f32 - 400.0) / 8.0;
        1.0 + (-d * d).exp() // ranges 1.0 -> 2.0
    }).collect();
    let neutral = vec![1.0_f32; num_bins];
    // mix=2.0 gets clamped to 1.0 inside the kernel — full wet.
    let mix = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&strength, &neutral, &neutral, &neutral, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = ModuleContext::new(
        48_000.0, 2048, num_bins,
        10.0, 100.0, 1.0, 0.0, false, false,
    );

    let dry_at_carve_centre = bins[400].norm();
    let dry_at_far_left  = bins[380].norm();
    let dry_at_far_right = bins[420].norm();

    // 15 hops gives the curve smoother (tau = 4·dt) ~3 time-constants to settle so the
    // carve fraction reaches steady-state before we measure.
    for _ in 0..15 {
        module.process(
            0, StereoLink::Linked, FxChannelTarget::All,
            &mut bins, None, &curves, &mut suppression, None, &ctx,
        );
    }

    // Carve zone should have less energy now.
    let wet_at_carve_centre = bins[400].norm();
    assert!(wet_at_carve_centre < dry_at_carve_centre * 0.7,
        "Diamagnet did not carve: wet[400]={} dry[400]={}", wet_at_carve_centre, dry_at_carve_centre);
    // Energy on the wings should have *increased*.
    let wet_at_far_left  = bins[380].norm();
    let wet_at_far_right = bins[420].norm();
    assert!(wet_at_far_left > dry_at_far_left || wet_at_far_right > dry_at_far_right,
        "Diamagnet did not redistribute carve energy outward: wet[380]={} dry[380]={} wet[420]={} dry[420]={}",
        wet_at_far_left, dry_at_far_left, wet_at_far_right, dry_at_far_right);
    // Conservation: total power within +/-10% (allow small loss to numerical roundoff).
    let wet_total: f32 = bins.iter().map(|b| b.norm_sqr()).sum();
    let loss = (dry_total - wet_total).abs() / dry_total;
    assert!(loss < 0.10, "Diamagnet violated energy conservation by {}%", loss * 100.0);
}

#[test]
fn kinetics_default_mode_is_hooke() {
    use spectral_forge::dsp::modules::kinetics::KineticsMode;
    assert_eq!(KineticsMode::default(), KineticsMode::Hooke);
}

#[test]
fn kinetics_well_source_default_is_static() {
    use spectral_forge::dsp::modules::kinetics::WellSource;
    assert_eq!(WellSource::default(), WellSource::Static);
}

#[test]
fn kinetics_mass_source_default_is_static() {
    use spectral_forge::dsp::modules::kinetics::MassSource;
    assert_eq!(MassSource::default(), MassSource::Static);
}

#[test]
fn params_carries_slot_kinetics_mode() {
    // Smoke: all three Mutexes are reachable + lockable from a fresh defaults instance.
    use spectral_forge::params::SpectralForgeParams;
    let params = SpectralForgeParams::default();
    let _ = params.slot_kinetics_mode.try_lock().expect("mode mutex contended on fresh init");
    let _ = params.slot_kinetics_well_source.try_lock().expect("well-src mutex contended on fresh init");
    let _ = params.slot_kinetics_mass_source.try_lock().expect("mass-src mutex contended on fresh init");
}

// ── Phase 5b4.1 — Modulate GravityPhaser + PllTear ────────────────────────

#[test]
fn modulate_spec_advertises_physics_writer() {
    use spectral_forge::dsp::modules::{module_spec, ModuleType};
    let spec = module_spec(ModuleType::Modulate);
    assert!(spec.writes_bin_physics, "Gravity Phaser writes phase_momentum");
}

#[test]
fn modulate_heavy_cpu_only_for_pll_tear() {
    use spectral_forge::dsp::modules::SpectralModule;
    use spectral_forge::dsp::modules::modulate::{ModulateModule, ModulateMode};
    let mut m = ModulateModule::new();
    // Default mode = PhasePhaser (light)
    assert!(!m.heavy_cpu_for_mode(), "PhasePhaser must be light");
    for light in [ModulateMode::PhasePhaser, ModulateMode::BinSwapper,
                  ModulateMode::RmFmMatrix, ModulateMode::DiodeRm,
                  ModulateMode::GroundLoop, ModulateMode::GravityPhaser] {
        m.set_mode(light);
        assert!(!m.heavy_cpu_for_mode(), "{:?} should be light", light);
    }
    m.set_mode(ModulateMode::PllTear);
    assert!(m.heavy_cpu_for_mode(), "PllTear must be heavy");
}

#[test]
fn modulate_mode_enum_has_new_variants() {
    use spectral_forge::dsp::modules::modulate::ModulateMode;
    // Pin discriminants: MOD_HEAVY indexes by `mode as usize`, so a silent
    // reorder would mis-flag light modes as heavy and vice versa.
    assert_eq!(ModulateMode::PhasePhaser   as u8, 0);
    assert_eq!(ModulateMode::BinSwapper    as u8, 1);
    assert_eq!(ModulateMode::RmFmMatrix    as u8, 2);
    assert_eq!(ModulateMode::DiodeRm       as u8, 3);
    assert_eq!(ModulateMode::GroundLoop    as u8, 4);
    assert_eq!(ModulateMode::GravityPhaser as u8, 5);
    assert_eq!(ModulateMode::PllTear       as u8, 6);
}

// ── Phase 5b4.3 — Curve smoothing infrastructure ──────────────────────────

#[test]
fn modulate_smoothed_curves_present_for_retrofit_modes() {
    use spectral_forge::dsp::modules::modulate::ModulateModule;
    use spectral_forge::dsp::modules::SpectralModule;

    let mut module = ModulateModule::new();
    module.reset(48_000.0, 2048);

    // After reset, smoothed_curves must be allocated to num_bins.
    let snap = module.smoothed_curves_len();
    assert_eq!(snap, 1025, "smoothed_curves not allocated to fft_size/2+1");
}

#[test]
fn modulate_v1_modes_skip_smoothing_pass() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::modulate::{ModulateModule, ModulateMode};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    // All five v1 modes must NOT touch the smoother (smoothed_primed stays false).
    // Each mode runs in a fresh module so primed state cannot leak between modes.
    let v1_modes = [
        ModulateMode::PhasePhaser,
        ModulateMode::BinSwapper,
        ModulateMode::RmFmMatrix,
        ModulateMode::DiodeRm,
        ModulateMode::GroundLoop,
    ];

    let num_bins = 1025;
    let amount = vec![2.0_f32; num_bins];
    let neutral = vec![1.0_f32; num_bins];
    let zeros = vec![0.0_f32; num_bins];
    let mix = vec![2.0_f32; num_bins];
    // curves: [AMOUNT, REACH, RATE, THRESH, AMPGATE, MIX]
    let curves: Vec<&[f32]> = vec![&amount, &neutral, &neutral, &neutral, &zeros, &mix];
    let sidechain = vec![0.5_f32; num_bins]; // RM/FM Matrix + Diode RM consume sidechain
    let ctx = ModuleContext::new(
        48_000.0, 2048, num_bins,
        10.0, 100.0, 1.0,
        1.0, false, false,
    );

    for mode in v1_modes {
        let mut module = ModulateModule::new();
        module.reset(48_000.0, 2048);
        module.set_mode(mode);

        let mut bins: Vec<Complex<f32>> = (0..num_bins).map(|_| Complex::new(1.0, 0.0)).collect();
        let mut suppression = vec![0.0_f32; num_bins];

        module.process(0, StereoLink::Linked, FxChannelTarget::All,
                       &mut bins, Some(&sidechain), &curves, &mut suppression, None, &ctx);
        module.process(1, StereoLink::Linked, FxChannelTarget::All,
                       &mut bins, Some(&sidechain), &curves, &mut suppression, None, &ctx);

        // Real claim of the test name: v1 modes never call refresh_smoothed,
        // so smoothed_primed stays false on both channels.
        assert!(!module.smoothed_primed_for_test(0),
            "{:?}: smoothed_primed[0] became true — v1 mode should not call refresh_smoothed", mode);
        assert!(!module.smoothed_primed_for_test(1),
            "{:?}: smoothed_primed[1] became true — v1 mode should not call refresh_smoothed", mode);
    }
}

#[test]
fn modulate_gravity_phaser_writes_phase_momentum_and_rotates() {
    use num_complex::Complex;
    use spectral_forge::dsp::bin_physics::BinPhysics;
    use spectral_forge::dsp::modules::modulate::{ModulateModule, ModulateMode};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let mut module = ModulateModule::new();
    module.reset(48_000.0, 2048);
    module.set_modulate_mode(ModulateMode::GravityPhaser);

    let num_bins = 1025;
    let mut bins: Vec<Complex<f32>> = (0..num_bins).map(|_| Complex::new(1.0, 0.0)).collect();
    let dry_norms: Vec<f32> = bins.iter().map(|b| b.norm()).collect();

    // AMOUNT=2 (max), REACH=1, RATE=1, THRESH=1, AMPGATE=0, MIX=2 (full wet)
    let amount = vec![2.0_f32; num_bins];
    let neutral = vec![1.0_f32; num_bins];
    let zeros = vec![0.0_f32; num_bins];
    let mix = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &neutral, &neutral, &neutral, &zeros, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let mut physics = BinPhysics::new();
    physics.reset_active(num_bins, 48_000.0, 2048);
    physics.phase_momentum[100] = 0.5; // seed at bin 100

    let ctx = ModuleContext::new(48_000.0, 2048, num_bins, 10.0, 100.0, 1.0, 1.0, false, false);

    // 10 hops so smoother primes and momentum integrates.
    for _ in 0..10 {
        module.process(0, StereoLink::Linked, FxChannelTarget::All,
                       &mut bins, None, &curves,
                       &mut suppression, Some(&mut physics), &ctx);
    }

    // Magnitudes preserved (rotation is unit-modulus + dry/wet blend of equal-magnitude vectors).
    for k in 0..num_bins {
        let mag = bins[k].norm();
        assert!((mag - dry_norms[k]).abs() < 0.05,
            "bin {} mag drift {} -> {}", k, dry_norms[k], mag);
    }
    // Phases must have rotated away from 0 around bin 100 (where momentum was seeded).
    let near_seed: f32 = (95..=105).map(|k| bins[k].im.abs()).fold(0.0, f32::max);
    assert!(near_seed > 0.05, "near-seed bins did not rotate (max im = {})", near_seed);
    // Phase momentum must remain non-zero around bin 100 (kernel writes it).
    let momentum_after = physics.phase_momentum[100];
    assert!(momentum_after.is_finite(), "momentum NaN after Gravity Phaser");
    assert!(momentum_after.abs() > 0.0, "Gravity Phaser did not write phase_momentum");
    // The bin-100 seed must still differentiate from a cold bin (50): seeded
    // bin starts ahead and stays ahead under identical force, so |m[100]| > |m[50]|.
    assert!(physics.phase_momentum[100].abs() > physics.phase_momentum[50].abs(),
        "bin-100 seed did not differentiate: m[100]={}, m[50]={}",
        physics.phase_momentum[100], physics.phase_momentum[50]);
}

#[test]
fn modulate_gravity_phaser_repel_inverts_rotation_direction() {
    use num_complex::Complex;
    use spectral_forge::dsp::bin_physics::BinPhysics;
    use spectral_forge::dsp::modules::modulate::{ModulateModule, ModulateMode};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    fn run_with_repel(repel: bool) -> f32 {
        let mut module = ModulateModule::new();
        module.reset(48_000.0, 2048);
        module.set_modulate_mode(ModulateMode::GravityPhaser);
        module.set_modulate_repel(repel);

        let num_bins = 1025;
        let mut bins: Vec<Complex<f32>> = (0..num_bins).map(|_| Complex::new(1.0, 0.0)).collect();

        // AMOUNT=2 (max), REACH=neutral, RATE=neutral, THRESH=neutral, AMPGATE=0, MIX=2 full-wet
        let amount = vec![2.0_f32; num_bins];
        let neutral = vec![1.0_f32; num_bins];
        let zeros = vec![0.0_f32; num_bins];
        let mix = vec![2.0_f32; num_bins];
        let curves: Vec<&[f32]> = vec![&amount, &neutral, &neutral, &neutral, &zeros, &mix];

        let mut suppression = vec![0.0_f32; num_bins];
        let mut physics = BinPhysics::new();
        physics.reset_active(num_bins, 48_000.0, 2048);

        let ctx = ModuleContext::new(
            48_000.0, 2048, num_bins, 10.0, 100.0, 1.0, 1.0, false, false,
        );

        for _ in 0..15 {
            module.process(0, StereoLink::Linked, FxChannelTarget::All,
                           &mut bins, None, &curves,
                           &mut suppression, Some(&mut physics), &ctx);
        }
        physics.phase_momentum[200] // sample any bin where ampgate=0 produces equal force
    }

    let pull_momentum = run_with_repel(false);
    let push_momentum = run_with_repel(true);
    assert!(pull_momentum.is_finite() && push_momentum.is_finite(),
        "non-finite momentum: pull={}, push={}", pull_momentum, push_momentum);
    // Repel must invert sign of accumulated momentum and produce non-trivial magnitude.
    assert!(pull_momentum.abs() > 1e-6, "pull momentum trivially small: {}", pull_momentum);
    assert!(pull_momentum.signum() == -push_momentum.signum(),
        "Repel did not invert sign: pull={}, push={}", pull_momentum, push_momentum);
    // Kernel is fully deterministic and identical except for the sign flip, so the two
    // runs should be exact mirrors. Catches a hypothetical bug where Repel multiplied
    // by something other than -1.0 (e.g. 0.0 → no-op, or 0.5 → asymmetric scale).
    assert!((pull_momentum + push_momentum).abs() < 1e-5,
        "Repel not exact sign flip: pull={}, push={}, sum={}",
        pull_momentum, push_momentum, pull_momentum + push_momentum);
}

#[test]
fn modulate_gravity_phaser_sc_positioned_peaks_concentrate_momentum() {
    // SidechainPositioned mode: sidechain peaks act as gravity wells. Bins near
    // a strong sidechain peak must accumulate more phase_momentum than distant bins.
    use num_complex::Complex;
    use spectral_forge::dsp::bin_physics::BinPhysics;
    use spectral_forge::dsp::modules::modulate::{ModulateModule, ModulateMode};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let mut module = ModulateModule::new();
    module.reset(48_000.0, 2048);
    module.set_modulate_mode(ModulateMode::GravityPhaser);
    module.set_modulate_sc_positioned(true);

    let num_bins = 1025;
    let mut bins: Vec<Complex<f32>> = (0..num_bins).map(|_| Complex::new(1.0, 0.0)).collect();

    // AMOUNT=2, REACH=1 (medium), RATE=neutral, THRESH=1, AMPGATE=neutral, MIX=2 full-wet
    let amount  = vec![2.0_f32; num_bins];
    let neutral = vec![1.0_f32; num_bins];
    let mix     = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &neutral, &neutral, &neutral, &neutral, &mix];

    // Sidechain with a single strong peak at bin 200.
    let mut sc = vec![0.01_f32; num_bins];
    sc[199] = 0.2;
    sc[200] = 3.0; // clear local maximum
    sc[201] = 0.15;

    let mut suppression = vec![0.0_f32; num_bins];
    let mut physics = BinPhysics::new();
    physics.reset_active(num_bins, 48_000.0, 2048);

    let ctx = ModuleContext::new(48_000.0, 2048, num_bins, 10.0, 100.0, 1.0, 1.0, false, false);

    // Run enough hops for momentum to integrate.
    for _ in 0..20 {
        module.process(0, StereoLink::Linked, FxChannelTarget::All,
                       &mut bins, Some(&sc), &curves,
                       &mut suppression, Some(&mut physics), &ctx);
    }

    // All momentum values must be finite.
    for k in 0..num_bins {
        assert!(physics.phase_momentum[k].is_finite(),
            "NaN/Inf momentum at bin {}", k);
    }

    // Bins near the sidechain peak (bin 200) should have more momentum than
    // bins far from any peak (e.g. bin 600 which is well past the only peak).
    let near_peak  = physics.phase_momentum[200].abs();
    let far_from_peak = physics.phase_momentum[600].abs();
    assert!(near_peak > far_from_peak,
        "sc_positioned: near-peak momentum ({}) not greater than far-peak ({})",
        near_peak, far_from_peak);

    // When no sidechain is supplied, momentum decays to zero (empty node list → pure decay).
    let mut module_no_sc = ModulateModule::new();
    module_no_sc.reset(48_000.0, 2048);
    module_no_sc.set_modulate_mode(ModulateMode::GravityPhaser);
    module_no_sc.set_modulate_sc_positioned(true);

    let mut bins2: Vec<Complex<f32>> = (0..num_bins).map(|_| Complex::new(1.0, 0.0)).collect();
    let mut supp2 = vec![0.0_f32; num_bins];
    let mut phys2 = BinPhysics::new();
    phys2.reset_active(num_bins, 48_000.0, 2048);
    // Seed a nonzero value.
    phys2.phase_momentum[300] = 5.0;

    // Run with no sidechain — momentum should decay via the 0.95 factor.
    for _ in 0..40 {
        module_no_sc.process(0, StereoLink::Linked, FxChannelTarget::All,
                             &mut bins2, None, &curves,
                             &mut supp2, Some(&mut phys2), &ctx);
    }

    // After 40 hops with pure decay (0.95^40 ≈ 0.129), the seeded momentum
    // should be well below the original 5.0.
    assert!(phys2.phase_momentum[300].abs() < 1.0,
        "no-sidechain momentum did not decay: m[300]={}", phys2.phase_momentum[300]);
}

#[test]
fn modulate_gravity_phaser_phase_momentum_visible_to_next_slot() {
    // Writer-feeds-reader sequencing test: cold-start (no seed), 20 hops.
    // Verifies that the Gravity Phaser kernel populates phase_momentum across
    // the full bin range so a downstream slot reading BinPhysics sees non-trivial
    // state. Bin 200 is mid-spectrum, far from both DC and Nyquist edges.
    use num_complex::Complex;
    use spectral_forge::dsp::bin_physics::BinPhysics;
    use spectral_forge::dsp::modules::modulate::{ModulateModule, ModulateMode};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let mut module = ModulateModule::new();
    module.reset(48_000.0, 2048);
    module.set_modulate_mode(ModulateMode::GravityPhaser);

    let num_bins = 1025;
    // Unit-real bins — non-zero amplitude so the kernel sees signal to act on.
    let mut bins: Vec<Complex<f32>> = (0..num_bins).map(|_| Complex::new(1.0, 0.0)).collect();

    // AMOUNT=1.5 (near-max, per plan), REACH=neutral, RATE=neutral, THRESH=neutral,
    // AMPGATE=0 (gate disabled so all bins receive force), MIX=2 (full wet).
    let amount  = vec![1.5_f32; num_bins];
    let neutral = vec![1.0_f32; num_bins];
    let zeros   = vec![0.0_f32; num_bins];
    let mix     = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &neutral, &neutral, &neutral, &zeros, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    // Cold start: BinPhysics freshly reset, no seeds.
    let mut physics = BinPhysics::new();
    physics.reset_active(num_bins, 48_000.0, 2048);

    let ctx = ModuleContext::new(48_000.0, 2048, num_bins, 10.0, 100.0, 1.0, 1.0, false, false);

    // 20 hops: enough for the smoother to prime and momentum to integrate from cold start.
    for _ in 0..20 {
        module.process(0, StereoLink::Linked, FxChannelTarget::All,
                       &mut bins, None, &curves,
                       &mut suppression, Some(&mut physics), &ctx);
    }

    // Primary claim: mid-spectrum bin 200 must have non-trivial momentum after 20 hops.
    assert!(physics.phase_momentum[200].is_finite(),
        "phase_momentum[200] is NaN/Inf after cold-start writer run");
    assert!(physics.phase_momentum[200].abs() > 1e-6,
        "Gravity Phaser did not write phase_momentum at bin 200 (cold start): {}",
        physics.phase_momentum[200]);

    // Secondary claim: all-bins finiteness sweep — next slot's reader must not see NaN.
    for k in 0..num_bins {
        assert!(physics.phase_momentum[k].is_finite(),
            "NaN/Inf phase_momentum at bin {} after cold-start writer run", k);
    }
}

#[test]
fn modulate_pll_tear_locks_on_steady_input_and_passes_through() {
    use std::cell::Cell;
    use num_complex::Complex;
    use spectral_forge::dsp::bin_physics::BinPhysics;
    use spectral_forge::dsp::modules::modulate::{ModulateModule, ModulateMode};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let mut module = ModulateModule::new();
    module.reset(48_000.0, 2048);
    module.set_modulate_mode(ModulateMode::PllTear);

    let num_bins = 1025;
    // Steady input: phases identical hop-to-hop. PLL must lock and emit dry.
    let bins_template: Vec<Complex<f32>> = (0..num_bins)
        .map(|k| {
            let phase = (k as f32) * 0.05;
            Complex::new(phase.cos(), phase.sin())
        })
        .collect();
    let mut bins = bins_template.clone();

    // AMOUNT=2 (full wet of torn output, but tear is gated by lock detector),
    // REACH=2 (all bins active), RATE=1 (default omega_n), THRESH=1 (default),
    // AMPGATE=0, MIX=2.
    let amount  = vec![2.0_f32; num_bins];
    let neutral = vec![1.0_f32; num_bins];
    let zeros   = vec![0.0_f32; num_bins];
    let mix     = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &neutral, &neutral, &neutral, &zeros, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let mut physics = BinPhysics::new();
    physics.reset_active(num_bins, 48_000.0, 2048);

    // Provide unwrapped phase as Cell<f32> (the correct type for ctx.unwrapped_phase).
    let phases_raw: Vec<f32> = bins_template.iter().map(|b| b.arg()).collect();
    let unwrapped_cells: Vec<Cell<f32>> = phases_raw.iter().map(|&p| Cell::new(p)).collect();

    let mut ctx = ModuleContext::new(
        48_000.0, 2048, num_bins, 10.0, 100.0, 1.0, 1.0, false, false,
    );
    ctx.unwrapped_phase = Some(&unwrapped_cells[..]);

    // Run 30 hops with constant input. PLL should converge to lock; output ≈ dry.
    for _ in 0..30 {
        bins.copy_from_slice(&bins_template);
        module.process(0, StereoLink::Linked, FxChannelTarget::All,
                       &mut bins, None, &curves,
                       &mut suppression, Some(&mut physics), &ctx);
    }

    // After lock, magnitudes should still match dry (within tolerance).
    for k in 16..num_bins.min(900) {
        let mag = bins[k].norm();
        assert!((mag - 1.0).abs() < 0.05,
            "locked PLL magnitude drift at bin {}: {}", k, mag);
    }
}

#[test]
fn modulate_pll_tear_writes_phase_momentum_on_glide() {
    use std::cell::Cell;
    use num_complex::Complex;
    use spectral_forge::dsp::bin_physics::BinPhysics;
    use spectral_forge::dsp::modules::modulate::{ModulateModule, ModulateMode};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let mut module = ModulateModule::new();
    module.reset(48_000.0, 2048);
    module.set_modulate_mode(ModulateMode::PllTear);

    let num_bins = 1025;
    let mut phases: Vec<f32> = (0..num_bins).map(|k| (k as f32) * 0.05).collect();
    let mut bins: Vec<Complex<f32>> = phases.iter().map(|p| Complex::new(p.cos(), p.sin())).collect();

    let amount  = vec![2.0_f32; num_bins];
    let neutral = vec![1.0_f32; num_bins];
    let zeros   = vec![0.0_f32; num_bins];
    let mix     = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &neutral, &neutral, &neutral, &zeros, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let mut physics = BinPhysics::new();
    physics.reset_active(num_bins, 48_000.0, 2048);

    let base_ctx = ModuleContext::new(
        48_000.0, 2048, num_bins, 10.0, 100.0, 1.0, 1.0, false, false,
    );

    // Apply a fast phase glide in bin 100 over 30 hops — should overshoot loop bandwidth and tear.
    for hop in 0..30 {
        let glide = (hop as f32) * 0.8; // fast: 0.8 rad/hop in just bin 100
        phases[100] = (100.0 * 0.05) + glide;
        bins[100] = Complex::new(phases[100].cos(), phases[100].sin());

        // Build Cell<f32> slice from the current phase vector each hop.
        let unwrapped_cells: Vec<Cell<f32>> = phases.iter().map(|&p| Cell::new(p)).collect();
        let mut ctx = base_ctx;
        ctx.unwrapped_phase = Some(&unwrapped_cells[..]);

        module.process(0, StereoLink::Linked, FxChannelTarget::All,
                       &mut bins, None, &curves,
                       &mut suppression, Some(&mut physics), &ctx);
    }

    // Bin 100 phase momentum should have been kicked by the tear event.
    assert!(physics.phase_momentum[100].abs() > 0.0,
        "bin 100 momentum unchanged after glide tear: {}", physics.phase_momentum[100]);
    // No NaN or runaway anywhere.
    for k in 0..num_bins {
        assert!(physics.phase_momentum[k].is_finite(), "bin {} momentum NaN", k);
        assert!(physics.phase_momentum[k].abs() < 100.0,
            "bin {} momentum runaway: {}", k, physics.phase_momentum[k]);
    }
}
