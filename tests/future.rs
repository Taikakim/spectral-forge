use spectral_forge::dsp::modules::{ModuleType, module_spec};

#[test]
fn future_module_spec_has_5_curves() {
    let spec = module_spec(ModuleType::Future);
    assert_eq!(spec.num_curves, 5);
    assert_eq!(spec.curve_labels, &["AMOUNT", "TIME", "THRESHOLD", "SPREAD", "MIX"]);
    assert!(!spec.supports_sidechain);
    assert_eq!(spec.display_name, "Future");
}

use spectral_forge::dsp::modules::future::{FutureModule, FutureMode};

#[test]
fn future_mode_default_is_print_through() {
    assert_eq!(FutureMode::default(), FutureMode::PrintThrough);
}

#[test]
fn future_module_starts_silent() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::{SpectralModule, ModuleContext};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let mut m = FutureModule::new();
    m.reset(48000.0, 1024);
    let mut bins = vec![Complex::new(1.0, 0.0); 513];
    let curves_storage: Vec<Vec<f32>> = (0..5).map(|_| vec![1.0f32; 513]).collect();
    let curves: Vec<&[f32]> = curves_storage.iter().map(|v| v.as_slice()).collect();
    let mut supp = vec![0.0f32; 513];
    let ctx = ModuleContext::new(
        48000.0, 1024, 513,
        10.0, 100.0, 0.5, 1.0, false, false,
    );
    m.process(0, StereoLink::Linked, FxChannelTarget::All,
        &mut bins, None, &curves, &mut supp, &ctx);
    for c in &bins { assert!(c.re.is_finite() && c.im.is_finite()); }
}

#[test]
fn future_clear_state_zeroes_ring_and_resets_write_pos() {
    use spectral_forge::dsp::modules::SpectralModule;
    let mut m = FutureModule::new();
    m.reset(48000.0, 1024);
    // clear_state must not panic and must be idempotent.
    m.clear_state();
    m.clear_state();
}

#[test]
fn print_through_writes_ahead_then_reads() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::{SpectralModule, ModuleContext};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let mut m = FutureModule::new();
    m.set_mode(FutureMode::PrintThrough);
    m.reset(48000.0, 1024);

    // AMOUNT=1.0 (5% leak), TIME=1.0 (8 hops), THRESHOLD unused, SPREAD=0.0, MIX=2.0 → mix=1.0 (full wet).
    let amount = vec![1.0f32; 513];
    let time   = vec![1.0f32; 513];
    let thresh = vec![1.0f32; 513];
    let spread = vec![0.0f32; 513];
    let mix    = vec![2.0f32; 513];

    let ctx = ModuleContext::new(
        48000.0, 1024, 513,
        10.0, 100.0, 0.5, 1.0, false, false,
    );

    let curves: Vec<&[f32]> = vec![&amount, &time, &thresh, &spread, &mix];

    // Hop 0: feed unit impulse at bin 100. Wet output should be silent (buffer empty).
    let mut bins = vec![Complex::new(0.0, 0.0); 513];
    bins[100] = Complex::new(1.0, 0.0);
    let mut supp = vec![0.0f32; 513];
    m.process(0, StereoLink::Linked, FxChannelTarget::All,
        &mut bins, None, &curves, &mut supp, &ctx);
    assert!(bins[100].norm() < 0.01,
        "hop 0 wet should be silent (no historical data yet)");

    // Hops 1..=7: silence in.
    for _ in 1..=7 {
        let mut buf = vec![Complex::new(0.0, 0.0); 513];
        m.process(0, StereoLink::Linked, FxChannelTarget::All,
            &mut buf, None, &curves, &mut supp, &ctx);
    }

    // Hop 8: silence in; the impulse written at hop 0 should now read out.
    let mut bins = vec![Complex::new(0.0, 0.0); 513];
    m.process(0, StereoLink::Linked, FxChannelTarget::All,
        &mut bins, None, &curves, &mut supp, &ctx);
    // 5% leak × MIX=1.0 → expect ~0.05 magnitude at bin 100.
    assert!(bins[100].norm() > 0.03 && bins[100].norm() < 0.08,
        "hop 8 should read back the print-through with ~5% leak; got {}",
        bins[100].norm());
}

#[test]
fn print_through_spread_bleeds_to_adjacent_bins() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::{SpectralModule, ModuleContext};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let mut m = FutureModule::new();
    m.set_mode(FutureMode::PrintThrough);
    m.reset(48000.0, 1024);

    let amount = vec![1.0f32; 513];
    let time   = vec![1.0f32; 513];
    let thresh = vec![1.0f32; 513];
    let spread = vec![1.0f32; 513];   // 20% spread to k±1
    let mix    = vec![2.0f32; 513];

    let ctx = ModuleContext::new(
        48000.0, 1024, 513,
        10.0, 100.0, 0.5, 1.0, false, false,
    );
    let curves: Vec<&[f32]> = vec![&amount, &time, &thresh, &spread, &mix];

    let mut bins = vec![Complex::new(0.0, 0.0); 513];
    bins[100] = Complex::new(1.0, 0.0);
    let mut supp = vec![0.0f32; 513];
    m.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, None, &curves, &mut supp, &ctx);

    for _ in 1..=7 {
        let mut buf = vec![Complex::new(0.0, 0.0); 513];
        m.process(0, StereoLink::Linked, FxChannelTarget::All, &mut buf, None, &curves, &mut supp, &ctx);
    }

    let mut bins = vec![Complex::new(0.0, 0.0); 513];
    m.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, None, &curves, &mut supp, &ctx);
    assert!(bins[99].norm()  > 0.005, "spread should bleed left, got {}",  bins[99].norm());
    assert!(bins[101].norm() > 0.005, "spread should bleed right, got {}", bins[101].norm());
}

#[test]
fn print_through_spread_at_max_preserves_neighbour_phase() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::{SpectralModule, ModuleContext};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let mut m = FutureModule::new();
    m.set_mode(FutureMode::PrintThrough);
    m.reset(48000.0, 1024);

    let amount = vec![1.0f32; 513];
    let time   = vec![1.0f32; 513];
    let thresh = vec![1.0f32; 513];
    let spread = vec![2.0f32; 513];   // MAX SPREAD: centre 0%, neighbours 50% each
    let mix    = vec![2.0f32; 513];

    let ctx = ModuleContext::new(48000.0, 1024, 513, 10.0, 100.0, 0.5, 1.0, false, false);
    let curves: Vec<&[f32]> = vec![&amount, &time, &thresh, &spread, &mix];

    // Hop 0: pure imaginary impulse at bin 100 (phase = +π/2).
    let mut bins = vec![Complex::new(0.0, 0.0); 513];
    bins[100] = Complex::new(0.0, 1.0);
    let mut supp = vec![0.0f32; 513];
    m.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, None, &curves, &mut supp, &ctx);

    for _ in 1..=7 {
        let mut buf = vec![Complex::new(0.0, 0.0); 513];
        m.process(0, StereoLink::Linked, FxChannelTarget::All, &mut buf, None, &curves, &mut supp, &ctx);
    }
    let mut bins = vec![Complex::new(0.0, 0.0); 513];
    m.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, None, &curves, &mut supp, &ctx);

    // At max spread, centre bin 100 should be much smaller than neighbours (secondary echo
    // re-accumulation means it won't be exactly zero, but it should be small). Bin 99 + 101
    // should carry the phase of the original dry signal (imaginary), not the real axis.
    assert!(bins[100].norm() < 0.02, "centre should be ~zero at max spread, got {}", bins[100].norm());
    assert!(bins[99].im.abs() > bins[99].re.abs() * 5.0,
        "bin 99 should carry imaginary phase from original dry, got re={} im={}", bins[99].re, bins[99].im);
    assert!(bins[101].im.abs() > bins[101].re.abs() * 5.0,
        "bin 101 should carry imaginary phase from original dry, got re={} im={}", bins[101].re, bins[101].im);
}
