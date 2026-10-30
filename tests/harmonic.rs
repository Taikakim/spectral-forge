//! Harmonic module: H/P Split, Partial EQ, Exciter + the shared virtual-row table.

use num_complex::Complex;
use spectral_forge::dsp::harmonic_groups::HarmonicGroup;
use spectral_forge::dsp::modules::harmonic::{HarmonicMode, HarmonicModule};
use spectral_forge::dsp::modules::{
    split_virtual_rows, ModuleContext, ModuleType, SpectralModule, VirtualRowKind,
};
use spectral_forge::params::{FxChannelTarget, StereoLink};

const FFT: usize = 2048;
const N: usize = FFT / 2 + 1;
const SR: f32 = 48_000.0;

fn module(mode: HarmonicMode) -> HarmonicModule {
    let mut m = HarmonicModule::new();
    m.reset(SR, FFT);
    m.set_mode(mode);
    m
}

/// Curves: [AMOUNT, BALANCE, TILT, REACH, MIX]; bipolar ones neutral at 1.0.
fn curves(amount: f32, balance: f32, tilt: f32, reach: f32, mix: f32) -> [Vec<f32>; 5] {
    [vec![amount; N], vec![balance; N], vec![tilt; N], vec![reach; N], vec![mix; N]]
}

fn run(m: &mut HarmonicModule, bins: &mut [Complex<f32>], c: &[Vec<f32>; 5], ctx: &ModuleContext<'_>) {
    let refs: Vec<&[f32]> = c.iter().map(|v| v.as_slice()).collect();
    let mut supp = vec![0.0f32; N];
    m.process(0, StereoLink::Linked, FxChannelTarget::All, bins, None, &refs, &mut supp, None, ctx);
}

fn ctx<'a>() -> ModuleContext<'a> {
    ModuleContext::new(SR, FFT, N, 10.0, 100.0, 1.0, 1.0, false, false)
}

#[test]
fn hp_split_default_is_exact_passthrough_and_streams_sum_to_input() {
    let mut m = module(HarmonicMode::HpSplit);
    let c = curves(1.0, 1.0, 1.0, 1.0, 1.0);
    for hop in 0..30 {
        let input: Vec<Complex<f32>> = (0..N)
            .map(|k| Complex::from_polar(((k + hop) as f32 * 0.37).sin().abs() + 0.01, k as f32 * 0.2))
            .collect();
        let mut bins = input.clone();
        run(&mut m, &mut bins, &c, &ctx());
        assert_eq!(bins, input, "neutral H/P BAL must be bit-exact at hop {hop}");
        let [h, p] = m.virtual_outputs().expect("H/P Split exposes virtual outputs");
        for k in 0..N {
            assert!((h[k] + p[k] - input[k]).norm() < 1e-4, "H + P != input at bin {k}");
        }
    }
}

#[test]
fn hp_split_separates_steady_tone_from_broadband_hit() {
    let mut m = module(HarmonicMode::HpSplit);
    let c = curves(1.0, 1.0, 1.0, 1.0, 1.0);
    // 20 frames of a steady partial at bin 200 over a quiet floor.
    for _ in 0..20 {
        let mut bins = vec![Complex::new(0.001, 0.0); N];
        bins[200] = Complex::new(1.0, 0.0);
        run(&mut m, &mut bins, &c, &ctx());
    }
    let [h, p] = m.virtual_outputs().unwrap();
    assert!(h[200].norm() > 0.9, "steady tone should be harmonic, H = {}", h[200].norm());
    assert!(p[200].norm() < 0.1, "steady tone should not be percussive, P = {}", p[200].norm());

    // One broadband frame (a hit): every bin jumps to 1.0.
    let mut bins = vec![Complex::new(1.0, 0.0); N];
    run(&mut m, &mut bins, &c, &ctx());
    let [h, p] = m.virtual_outputs().unwrap();
    assert!(p[500].norm() > 0.9, "broadband hit should be percussive, P = {}", p[500].norm());
    assert!(h[500].norm() < 0.1, "broadband hit should not be harmonic, H = {}", h[500].norm());
}

#[test]
fn hp_split_balance_extremes_select_one_stream() {
    let mut m = module(HarmonicMode::HpSplit);
    let only_h = curves(1.0, 0.0, 1.0, 1.0, 1.0); // BALANCE −100 %
    for _ in 0..20 {
        let mut bins = vec![Complex::new(0.001, 0.0); N];
        bins[200] = Complex::new(1.0, 0.0);
        run(&mut m, &mut bins, &only_h, &ctx());
    }
    let mut bins = vec![Complex::new(0.001, 0.0); N];
    bins[200] = Complex::new(1.0, 0.0);
    run(&mut m, &mut bins, &only_h, &ctx());
    let [h, _] = m.virtual_outputs().unwrap();
    assert!((bins[200] - h[200]).norm() < 1e-5, "BALANCE −100% must output the H stream");
}

fn group(f0_bin: usize, bins: &[u16]) -> HarmonicGroup {
    let mut g = HarmonicGroup::default();
    g.fundamental_hz = f0_bin as f32 * SR / FFT as f32;
    g.harmonic_count = bins.len() as u8;
    for (i, &b) in bins.iter().enumerate() { g.harmonic_bins[i] = b; }
    g
}

#[test]
fn partial_eq_odd_even_and_no_groups_passthrough() {
    let mut m = module(HarmonicMode::PartialEq);
    let groups = [group(10, &[10, 20, 30, 40])];
    let mut c_ctx = ctx();
    c_ctx.harmonic_groups = Some(&groups);

    // ODD/EVEN +100 %, WIDTH 0 → evens ×2, odds ×0, fundamental untouched.
    let c = curves(1.0, 2.0, 1.0, 0.0, 1.0);
    let mut bins = vec![Complex::new(1.0, 0.0); N];
    run(&mut m, &mut bins, &c, &c_ctx);
    assert!((bins[10].norm() - 1.0).abs() < 1e-5, "fundamental must be untouched");
    assert!((bins[20].norm() - 2.0).abs() < 1e-5, "2nd partial should double");
    assert!(bins[30].norm() < 1e-5, "3rd partial should be removed");
    assert!((bins[40].norm() - 2.0).abs() < 1e-5, "4th partial should double");
    assert!((bins[25].norm() - 1.0).abs() < 1e-5, "non-partial bins untouched");

    // TILT +100 % on partial 16 would be 16×; the cap holds it at 4×.
    let groups = [group(10, &[10, 160])];
    let mut c_ctx = ctx();
    c_ctx.harmonic_groups = Some(&groups);
    let mut bins = vec![Complex::new(1.0, 0.0); N];
    run(&mut m, &mut bins, &curves(1.0, 1.0, 2.0, 0.0, 1.0), &c_ctx);
    assert!((bins[160].norm() - 4.0).abs() < 1e-4, "tilt gain must cap at +12 dB, got {}", bins[160].norm());

    // No groups → exact passthrough.
    let mut bins = vec![Complex::new(1.0, 0.0); N];
    run(&mut m, &mut bins, &curves(1.0, 2.0, 2.0, 1.0, 1.0), &ctx());
    assert!(bins.iter().all(|b| *b == Complex::new(1.0, 0.0)));
}

#[test]
fn exciter_adds_bounded_coherent_harmonics() {
    let mut m = module(HarmonicMode::Exciter);
    let mut bins = vec![Complex::new(0.0, 0.0); N];
    bins[50] = Complex::from_polar(1.0, 0.3);
    run(&mut m, &mut bins, &curves(1.0, 1.0, 1.0, 1.0, 1.0), &ctx());

    // ODD/EVEN neutral → each of h = 2..5 present, at most −12 dB.
    for h in 2..=5 {
        let mag = bins[50 * h].norm();
        assert!(mag > 0.0 && mag <= 0.25 + 1e-6, "harmonic {h} magnitude {mag} out of range");
        let want_phase = (h as f32 * 0.3).rem_euclid(std::f32::consts::TAU);
        let got_phase  = bins[50 * h].arg().rem_euclid(std::f32::consts::TAU);
        assert!((want_phase - got_phase).abs() < 1e-3, "harmonic {h} phase not h·φ");
    }
    assert!((bins[50].norm() - 1.0).abs() < 1e-6, "source bin must be unchanged");
    assert_eq!(bins[N - 1].im, 0.0, "Nyquist must stay real");

    // ODD/EVEN +100 % → only even harmonics.
    let mut bins = vec![Complex::new(0.0, 0.0); N];
    bins[50] = Complex::new(1.0, 0.0);
    run(&mut m, &mut bins, &curves(1.0, 2.0, 1.0, 1.0, 1.0), &ctx());
    assert!(bins[100].norm() > 0.0 && bins[150].norm() == 0.0);
}

#[test]
fn exciter_dense_input_stays_bounded() {
    let mut m = module(HarmonicMode::Exciter);
    let mut bins = vec![Complex::new(1.0, 0.0); N];
    run(&mut m, &mut bins, &curves(1.0, 1.0, 2.0, 1.0, 1.0), &ctx());
    let max = bins.iter().map(|b| b.norm()).fold(0.0f32, f32::max);
    // Each target receives at most one contribution per harmonic order (≤ 4 × 0.25).
    assert!(max <= 2.0 + 1e-4, "dense excitation exceeded +6 dB: {max}");
    assert!(bins.iter().all(|b| b.re.is_finite() && b.im.is_finite()));
}

#[test]
fn virtual_rows_follow_slot_order_mode_and_budget() {
    let mut types = [ModuleType::Empty; 9];
    let mut modes = [HarmonicMode::HpSplit; 9];
    types[1] = ModuleType::Harmonic;
    types[3] = ModuleType::TransientSustainedSplit;
    let rows = split_virtual_rows(&types, &modes);
    assert_eq!(rows[0], Some((1, VirtualRowKind::Harmonic)));
    assert_eq!(rows[1], Some((1, VirtualRowKind::Percussive)));
    assert_eq!(rows[2], Some((3, VirtualRowKind::Transient)));
    assert_eq!(rows[3], Some((3, VirtualRowKind::Sustained)));

    // Harmonic outside H/P Split mode owns no rows.
    modes[1] = HarmonicMode::Exciter;
    let rows = split_virtual_rows(&types, &modes);
    assert_eq!(rows[0], Some((3, VirtualRowKind::Transient)));
    assert_eq!(rows[2], None);

    // A third split beyond the 4-row budget gets nothing.
    modes[1] = HarmonicMode::HpSplit;
    types[5] = ModuleType::TransientSustainedSplit;
    let rows = split_virtual_rows(&types, &modes);
    assert!(rows.iter().all(|r| r.map(|(s, _)| s != 5).unwrap_or(true)));
}
