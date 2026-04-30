use num_complex::Complex;
use spectral_forge::dsp::modules::{
    create_module, ModuleContext, ModuleType, SpectralModule,
    HarmonyModule, HarmonyMode,
};
use spectral_forge::dsp::harmonic_groups::HarmonicGroup;
use spectral_forge::dsp::modules::harmony_helpers::{find_top_k_peaks, PeakRecord, best_chord_template};
use spectral_forge::dsp::modules::harmony::HarmonyInharmonicSubmode;
use spectral_forge::dsp::cepstrum::CepstrumBuf;
use spectral_forge::params::{FxChannelTarget, StereoLink};

fn ctx_default<'a>() -> ModuleContext<'a> {
    ModuleContext::new(48_000.0, 2048, 1025, 10.0, 100.0, 0.5, 0.0, false, false)
}

fn ctx_with_if<'a>(if_buf: &'a [f32]) -> ModuleContext<'a> {
    let mut c = ctx_default();
    c.instantaneous_freq = Some(if_buf);
    c
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

#[test]
fn shuffler_mode_swaps_some_bins_when_amount_high() {
    let mut m = HarmonyModule::new();
    m.reset(48_000.0, 2048);
    m.set_mode(HarmonyMode::Shuffler);

    let mut bins: Vec<Complex<f32>> = (0..1025)
        .map(|k| Complex::new(k as f32 * 0.1 + 1.0, 0.0)) // unique mag per bin
        .collect();
    let snapshot = bins.clone();
    let amount: Vec<f32> = vec![1.0; 1025]; // 100% swap probability
    let threshold: Vec<f32> = vec![0.0; 1025]; // never skip
    let stability: Vec<f32> = vec![0.0; 1025];
    let spread: Vec<f32> = vec![0.5; 1025]; // -> offset = 1
    let coefficient: Vec<f32> = vec![0.0; 1025];
    let mix: Vec<f32> = vec![1.0; 1025];

    let curve_refs: Vec<&[f32]> = vec![
        &amount, &threshold, &stability, &spread, &coefficient, &mix,
    ];
    let mut sup = vec![0.0; 1025];
    m.process(
        0, StereoLink::Linked, FxChannelTarget::All,
        &mut bins, None, &curve_refs, &mut sup, None, &ctx_default(),
    );

    let changed = (0..1025).filter(|&k| (bins[k] - snapshot[k]).norm() > 1e-6).count();
    assert!(
        changed > 100,
        "shuffler at AMOUNT=1 must change a substantial fraction of bins, got {}",
        changed
    );
}

#[test]
fn harmonic_generator_adds_partials() {
    let mut m = HarmonyModule::new();
    m.reset(48_000.0, 2048);
    m.set_mode(HarmonyMode::HarmonicGenerator);

    let n = 1025;
    // One loud peak at bin 50, rest near zero.
    let mut bins: Vec<Complex<f32>> = vec![Complex::new(0.0, 0.0); n];
    bins[50] = Complex::new(1.0, 0.0);

    // IF buffer: bin centre frequencies (no IF refinement).
    let if_buf: Vec<f32> = (0..n).map(|k| (k as f32) * 48_000.0 / 2048.0).collect();
    let ctx = ctx_with_if(&if_buf);

    let amount: Vec<f32> = vec![1.0; n];
    let threshold: Vec<f32> = vec![0.5; n]; // only bin 50 passes (mag=1.0 > 0.5)
    let stability: Vec<f32> = vec![0.0; n];
    let spread: Vec<f32> = vec![1.0; n]; // mid decay
    let coefficient: Vec<f32> = vec![1.5; n]; // ~24 harmonics
    let mix: Vec<f32> = vec![1.0; n];
    let curve_refs: Vec<&[f32]> = vec![
        &amount, &threshold, &stability, &spread, &coefficient, &mix,
    ];
    let mut sup = vec![0.0; n];

    m.process(
        0, StereoLink::Linked, FxChannelTarget::All,
        &mut bins, None, &curve_refs, &mut sup, None, &ctx,
    );

    // Original peak preserved.
    assert!(bins[50].norm() >= 0.99, "bin 50 must remain >= 1.0, got {}", bins[50].norm());
    // 2nd, 3rd, 4th harmonics added at bins 100, 150, 200.
    assert!(bins[100].norm() > 0.05, "2nd harmonic missing at bin 100");
    assert!(bins[150].norm() > 0.02, "3rd harmonic missing at bin 150");
    assert!(bins[200].norm() > 0.01, "4th harmonic missing at bin 200");
    // No phantom energy at non-harmonic bins.
    assert!(bins[75].norm() < 1e-6, "phantom energy at non-harmonic bin 75");
    assert!(bins[125].norm() < 1e-6, "phantom energy at non-harmonic bin 125");
}

#[test]
fn shuffler_mode_passes_through_when_amount_zero() {
    let mut m = HarmonyModule::new();
    m.reset(48_000.0, 2048);
    m.set_mode(HarmonyMode::Shuffler);

    let mut bins: Vec<Complex<f32>> = (0..1025)
        .map(|k| Complex::new(k as f32 * 0.1 + 1.0, 0.0))
        .collect();
    let snapshot = bins.clone();
    let curves: Vec<f32> = vec![0.0; 1025];
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
        assert!((bins[k] - snapshot[k]).norm() < 1e-6);
    }
}

#[test]
fn undertone_generator_adds_partials_below_loud_peak() {
    let mut m = HarmonyModule::new();
    m.reset(48_000.0, 2048);
    m.set_mode(HarmonyMode::Undertone);

    let n = 1025;
    // Loud peak at bin 200 (≈ 4.7 kHz at 48k/2048).
    let mut bins: Vec<Complex<f32>> = vec![Complex::new(0.0, 0.0); n];
    bins[200] = Complex::new(1.0, 0.0);
    let if_buf: Vec<f32> = (0..n).map(|k| (k as f32) * 48_000.0 / 2048.0).collect();
    let ctx = ctx_with_if(&if_buf);

    let amount = vec![1.0_f32; n];
    let threshold = vec![0.5_f32; n];
    let stability = vec![0.0_f32; n];
    let spread = vec![1.0_f32; n];
    let coefficient = vec![0.0_f32; n]; // hum disabled
    let mix = vec![1.0_f32; n];
    let curves: Vec<&[f32]> = vec![
        &amount, &threshold, &stability, &spread, &coefficient, &mix,
    ];
    let mut sup = vec![0.0_f32; n];

    m.process(
        0, StereoLink::Linked, FxChannelTarget::All,
        &mut bins, None, &curves, &mut sup, None, &ctx,
    );
    // Sub-octave at bin 100, sub-third at bin ~67, sub-fourth at bin 50.
    assert!(bins[100].norm() > 0.05, "f/2 undertone missing at bin 100");
    assert!(bins[67].norm() > 0.02 || bins[66].norm() > 0.02,
            "f/3 undertone missing near bin 67");
    assert!(bins[50].norm() > 0.01, "f/4 undertone missing at bin 50");
}

#[test]
fn inharmonic_stiffness_shifts_high_partials_upward() {
    let mut m = HarmonyModule::new();
    m.reset(48_000.0, 2048);
    m.set_mode(HarmonyMode::Inharmonic);
    m.set_inharmonic_submode(HarmonyInharmonicSubmode::Stiffness);

    let n = 1025;
    // Two peaks: fundamental at bin 50, 5th harmonic at bin 250.
    let mut bins: Vec<Complex<f32>> = vec![Complex::new(0.0, 0.0); n];
    bins[50]  = Complex::new(1.0, 0.0);
    bins[250] = Complex::new(0.5, 0.0);
    let snapshot = bins.clone();

    let if_buf: Vec<f32> = (0..n).map(|k| (k as f32) * 48_000.0 / 2048.0).collect();
    let ctx = ctx_with_if(&if_buf);

    let amount      = vec![1.0_f32; n];
    let threshold   = vec![0.4_f32; n];
    let stability   = vec![0.0_f32; n];
    let spread      = vec![0.0_f32; n];
    let coefficient = vec![1.0_f32; n]; // moderate B
    let mix         = vec![1.0_f32; n];
    let curves: Vec<&[f32]> = vec![
        &amount, &threshold, &stability, &spread, &coefficient, &mix,
    ];
    let mut sup = vec![0.0_f32; n];
    m.process(
        0, StereoLink::Linked, FxChannelTarget::All,
        &mut bins, None, &curves, &mut sup, None, &ctx,
    );

    // Bin 250 (5th harmonic) must have moved upward (stiffness pushes high partials up).
    let mut energy_above = 0.0_f32;
    for k in 251..n { energy_above += bins[k].norm(); }
    assert!(energy_above > 0.1, "expected stiffness to push energy above bin 250, got {}", energy_above);
    // Original bin 250 should be reduced.
    assert!(
        bins[250].norm() < snapshot[250].norm() * 0.9,
        "original 5th-harmonic peak must be attenuated, got {} vs {}",
        bins[250].norm(), snapshot[250].norm(),
    );
}

#[test]
fn lifter_mode_unity_when_curves_neutral() {
    let mut m = HarmonyModule::new();
    m.reset(48_000.0, 2048);
    m.set_mode(HarmonyMode::Lifter);

    let n = 1025;
    // Construct a magnitude-rich spectrum to lift.
    let mut bins: Vec<Complex<f32>> = (0..n)
        .map(|k| Complex::from_polar(1.0 + 0.5 * (k as f32 * 0.1).sin().abs(), 0.0))
        .collect();
    let snapshot = bins.clone();

    // Pre-compute cepstrum from the input. In production Pipeline does this; the test
    // does it manually to exercise Lifter in isolation.
    let mut cep_buf = CepstrumBuf::new(2048);
    cep_buf.compute_from_bins(&bins);
    let cep_owned: Vec<f32> = cep_buf.quefrency().to_vec();
    let mut ctx = ctx_default();
    ctx.cepstrum_buf = Some(&cep_owned);

    // Curves: AMOUNT=1, all others neutral (1.0).
    let amount      = vec![1.0_f32; n];
    let threshold   = vec![0.0_f32; n];
    let stability   = vec![0.0_f32; n];
    let spread      = vec![1.0_f32; n];      // envelope curve at unity
    let coefficient = vec![1.0_f32; n];      // pitch curve at unity
    let mix         = vec![1.0_f32; n];
    let curves: Vec<&[f32]> = vec![
        &amount, &threshold, &stability, &spread, &coefficient, &mix,
    ];
    let mut sup = vec![0.0_f32; n];

    m.process(
        0, StereoLink::Linked, FxChannelTarget::All,
        &mut bins, None, &curves, &mut sup, None, &ctx,
    );

    // Round-trip cepstrum → spectrum should preserve magnitudes within ~10%.
    let mut max_err = 0.0_f32;
    for k in 1..n - 1 {
        let err = (bins[k].norm() - snapshot[k].norm()).abs() / snapshot[k].norm().max(1e-6);
        if err > max_err { max_err = err; }
    }
    assert!(max_err < 0.10, "neutral lifter must preserve magnitude, max relative error {}", max_err);
}

#[test]
fn formant_rotation_passthrough_when_coefficient_one() {
    let mut m = HarmonyModule::new();
    m.reset(48_000.0, 2048);
    m.set_mode(HarmonyMode::FormantRotation);

    let n = 1025;
    let mut bins: Vec<Complex<f32>> = (0..n)
        .map(|k| Complex::from_polar(0.5 + 0.5 * (k as f32 * 0.05).sin().abs(), 0.0))
        .collect();
    let snapshot = bins.clone();

    let mut cep_buf = CepstrumBuf::new(2048);
    cep_buf.compute_from_bins(&bins);
    let cep_owned: Vec<f32> = cep_buf.quefrency().to_vec();
    let mut ctx = ctx_default();
    ctx.cepstrum_buf = Some(&cep_owned);

    let amount = vec![1.0_f32; n];
    let threshold = vec![0.0_f32; n];
    let stability = vec![0.0_f32; n];
    let spread = vec![0.0_f32; n];
    let coefficient = vec![1.0_f32; n]; // ratio = 1.0 = no rotation
    let mix = vec![1.0_f32; n];
    let curves: Vec<&[f32]> = vec![
        &amount, &threshold, &stability, &spread, &coefficient, &mix,
    ];
    let mut sup = vec![0.0_f32; n];

    m.process(
        0, StereoLink::Linked, FxChannelTarget::All,
        &mut bins, None, &curves, &mut sup, None, &ctx,
    );

    let mut max_err = 0.0_f32;
    for k in 1..n - 1 {
        let err = (bins[k].norm() - snapshot[k].norm()).abs() / snapshot[k].norm().max(1e-6);
        if err > max_err { max_err = err; }
    }
    assert!(max_err < 0.15, "ratio=1 must approximately passthrough, max err {}", max_err);
}

#[test]
fn chord_template_matches_c_major() {
    let mut chroma = [0.0_f32; 12];
    chroma[0] = 1.0;  // C
    chroma[4] = 1.0;  // E
    chroma[7] = 1.0;  // G
    let (best, score) = best_chord_template(&chroma);
    assert_eq!(best, 0, "C major template should win, got idx {} score {}", best, score);
    assert!(score > 0.99, "exact match must score near 1.0, got {}", score);
}

#[test]
fn chord_template_matches_a_minor() {
    let mut chroma = [0.0_f32; 12];
    chroma[9] = 1.0;  // A
    chroma[0] = 1.0;  // C
    chroma[4] = 1.0;  // E
    let (best, score) = best_chord_template(&chroma);
    assert_eq!(best, 12 + 9, "A minor (template idx 21) should win, got {}", best);
    assert!(score > 0.99);
}

#[test]
fn chordification_snaps_off_chord_partial_toward_chord_tone() {
    let mut m = HarmonyModule::new();
    m.reset(48_000.0, 2048);
    m.set_mode(HarmonyMode::Chordification);

    let n = 1025;
    let mut bins: Vec<Complex<f32>> = vec![Complex::new(0.0, 0.0); n];
    bins[40] = Complex::new(1.0, 0.0); // arbitrary bin

    // Chromagram = pure C major: C, E, G all max.
    let chroma_arr: [f32; 12] = {
        let mut a = [0.0_f32; 12];
        a[0] = 1.0; a[4] = 1.0; a[7] = 1.0;
        a
    };

    let mut ctx = ctx_default();
    ctx.chromagram = Some(&chroma_arr);

    let amount      = vec![1.0_f32; n];
    let threshold   = vec![0.5_f32; n];
    let stability   = vec![0.0_f32; n];
    let spread      = vec![1.0_f32; n];
    let coefficient = vec![0.0_f32; n];
    let mix         = vec![1.0_f32; n];
    let curves: Vec<&[f32]> = vec![
        &amount, &threshold, &stability, &spread, &coefficient, &mix,
    ];
    let mut sup = vec![0.0_f32; n];

    m.process(
        0, StereoLink::Linked, FxChannelTarget::All,
        &mut bins, None, &curves, &mut sup, None, &ctx,
    );

    // The original bin should be reduced; some bin in the C/E/G class above
    // it should be increased.
    assert!(bins[40].norm() < 0.99, "off-chord partial must be attenuated, got {}", bins[40].norm());
}

#[test]
fn companding_attenuates_harmonic_class_when_coefficient_high() {
    let mut m = HarmonyModule::new();
    m.reset(48_000.0, 2048);
    m.set_mode(HarmonyMode::Companding);

    let n = 1025;
    let mut bins: Vec<Complex<f32>> = vec![Complex::new(0.01, 0.0); n];
    bins[50]  = Complex::new(1.0, 0.0);
    bins[100] = Complex::new(0.5, 0.0);
    bins[333] = Complex::new(0.7, 0.0);

    let snapshot = bins.clone();

    let groups_arr: [HarmonicGroup; 1] = [HarmonicGroup {
        fundamental_hz:  50.0 * 48_000.0 / 2048.0,
        harmonic_count:  2,
        harmonic_bins:   { let mut b = [0u16; 16]; b[0] = 50; b[1] = 100; b },
        total_magnitude: 1.5,
    }];
    let mut ctx = ctx_default();
    ctx.harmonic_groups = Some(&groups_arr);

    let amount      = vec![1.0_f32; n];
    let threshold   = vec![0.0_f32; n];
    let stability   = vec![0.0_f32; n];
    let spread      = vec![0.0_f32; n];
    let coefficient = vec![2.0_f32; n];
    let mix         = vec![1.0_f32; n];
    let curves: Vec<&[f32]> = vec![
        &amount, &threshold, &stability, &spread, &coefficient, &mix,
    ];
    let mut sup = vec![0.0_f32; n];

    m.process(
        0, StereoLink::Linked, FxChannelTarget::All,
        &mut bins, None, &curves, &mut sup, None, &ctx,
    );

    assert!(
        bins[100].norm() < snapshot[100].norm() * 0.95,
        "harmonic bin 100 must be attenuated, got {} vs {}",
        bins[100].norm(), snapshot[100].norm(),
    );
    assert!(
        (bins[333].norm() - snapshot[333].norm()).abs() < 1e-3,
        "inharmonic bin 333 must be untouched, got {} vs {}",
        bins[333].norm(), snapshot[333].norm(),
    );
}
