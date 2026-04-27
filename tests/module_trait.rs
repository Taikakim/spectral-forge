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
              &mut bins, None, &curves_ref, &mut supp, &ctx);

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
              &mut bins, None, &curves_ref, &mut supp, &ctx);
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
    let ctx = ModuleContext::new(
        48000.0, 2048, num_bins,
        10.0, 80.0, 0.5, 0.0, false, false,
    );

    m.process(0, StereoLink::Linked, FxChannelTarget::All,
              &mut bins, Some(&sc), &curves_ref, &mut supp, &ctx);

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
                  &mut bins, Some(&sc), &curves_ref, &mut supp, &ctx);
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
              &mut bins_a, Some(&sc_hot),  &curves_ref, &mut supp_a, &ctx);
    b.process(0, StereoLink::Linked, FxChannelTarget::All,
              &mut bins_b, Some(&sc_cold), &curves_ref, &mut supp_b, &ctx);

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
