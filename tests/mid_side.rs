//! Mid/Side TRANSIENT and PAN (bipolar curves, gain 1.0 = neutral).

use num_complex::Complex;
use spectral_forge::dsp::modules::{create_module, ModuleContext, ModuleType, SpectralModule};
use spectral_forge::params::{FxChannelTarget, StereoLink};

const FFT: usize = 2048;
const N: usize = FFT / 2 + 1;

/// One hop through both channels (mid on 0, side on 1). Returns (mid', side').
fn hop(
    m: &mut Box<dyn SpectralModule>,
    mid: f32,
    side: f32,
    curves: &[Vec<f32>; 5],
) -> (Vec<Complex<f32>>, Vec<Complex<f32>>) {
    let ctx = ModuleContext::new(48_000.0, FFT, N, 10.0, 100.0, 1.0, 1.0, false, false);
    let refs: Vec<&[f32]> = curves.iter().map(|c| c.as_slice()).collect();
    let mut supp = vec![0.0f32; N];
    let mut mb = vec![Complex::new(mid, 0.0); N];
    let mut sb = vec![Complex::new(side, 0.0); N];
    m.process(0, StereoLink::MidSide, FxChannelTarget::All, &mut mb, None, &refs, &mut supp, None, &ctx);
    m.process(1, StereoLink::MidSide, FxChannelTarget::All, &mut sb, None, &refs, &mut supp, None, &ctx);
    (mb, sb)
}

/// BALANCE/EXPANSION neutral, DECORREL off, TRANSIENT/PAN as given.
fn curves(transient: f32, pan: f32) -> [Vec<f32>; 5] {
    [vec![1.0; N], vec![1.0; N], vec![0.0; N], vec![transient; N], vec![pan; N]]
}

#[test]
fn neutral_transient_and_pan_are_bit_exact_passthrough() {
    let mut m = create_module(ModuleType::MidSide, 48_000.0, FFT);
    m.reset(48_000.0, FFT);
    for i in 0..20 {
        let mid = if i == 10 { 4.0 } else { 1.0 }; // includes an onset
        let (mb, sb) = hop(&mut m, mid, 0.5, &curves(1.0, 1.0));
        assert!(mb.iter().all(|b| *b == Complex::new(mid, 0.0)), "mid changed at hop {i}");
        assert!(sb.iter().all(|b| *b == Complex::new(0.5, 0.0)), "side changed at hop {i}");
    }
}

#[test]
fn pan_hard_right_is_constant_power() {
    let mut m = create_module(ModuleType::MidSide, 48_000.0, FFT);
    m.reset(48_000.0, FFT);
    // Mono source: M = 1, S = 0. L ∝ M+S, R ∝ M−S.
    let (mb, sb) = hop(&mut m, 1.0, 0.0, &curves(1.0, 2.0)); // PAN +100 %
    let l = mb[100] + sb[100];
    let r = mb[100] - sb[100];
    assert!(l.norm() < 1e-5, "hard right: L should be silent, got {}", l.norm());
    assert!((r.norm() - std::f32::consts::SQRT_2).abs() < 1e-4,
        "hard right: R should carry all power (√2), got {}", r.norm());

    let (mb, sb) = hop(&mut m, 1.0, 0.0, &curves(1.0, 0.0)); // PAN −100 %
    let l = mb[100] + sb[100];
    let r = mb[100] - sb[100];
    assert!(r.norm() < 1e-5, "hard left: R should be silent, got {}", r.norm());
    assert!((l.norm() - std::f32::consts::SQRT_2).abs() < 1e-4);
}

#[test]
fn transient_mono_collapses_side_on_onsets_only() {
    let mut m = create_module(ModuleType::MidSide, 48_000.0, FFT);
    m.reset(48_000.0, FFT);
    let c = curves(0.0, 1.0); // TRANSIENT −100 %
    let mut steady_side = 0.0;
    for _ in 0..200 {
        steady_side = hop(&mut m, 1.0, 0.5, &c).1[100].norm();
    }
    assert!((steady_side - 0.5).abs() < 0.01, "steady state should leave side alone, got {steady_side}");

    let onset_side = hop(&mut m, 4.0, 0.5, &c).1[100].norm();
    assert!(onset_side < 0.05, "onset (×4 mid) should mute the side, got {onset_side}");
}
