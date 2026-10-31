//! Life is a BinPhysics writer, so the FxMatrix hands it its own `physics`
//! handle and `ctx.bin_physics = None`. The velocity- and temperature-driven
//! modes must read the field from that handle; reading only `ctx` left them
//! seeing zeros (Non-Newtonian inert, Stiction muting, Brownian inert).

use num_complex::Complex;
use spectral_forge::dsp::bin_physics::BinPhysics;
use spectral_forge::dsp::modules::life::{LifeMode, LifeModule};
use spectral_forge::dsp::modules::{ModuleContext, SpectralModule};
use spectral_forge::params::{FxChannelTarget, StereoLink};

const N: usize = 1025;

/// Runs `hops` hops the way the FxMatrix runs a writer slot and returns the
/// last output's power relative to the input's.
fn writer_slot_power_ratio(mode: LifeMode, fill: impl Fn(&mut BinPhysics), hops: usize) -> (f32, Vec<Complex<f32>>) {
    let mut module = LifeModule::new();
    module.reset(48_000.0, 2048);
    module.set_mode(mode);
    let one = vec![1.0_f32; N];
    let curves: Vec<&[f32]> = vec![&one, &one, &one, &one, &one];
    let ctx = ModuleContext::new(48_000.0, 2048, N, 10.0, 100.0, 1.0, 0.0, false, false);
    let input: Vec<Complex<f32>> = (0..N).map(|k| Complex::from_polar(1.0, k as f32 * 0.7)).collect();
    let mut supp = vec![0.0_f32; N];
    let mut phys = BinPhysics::new();
    let mut out = input.clone();
    for _ in 0..hops {
        phys.reset_active(N, 48_000.0, 2048);
        fill(&mut phys);
        out = input.clone();
        module.process(0, StereoLink::Linked, FxChannelTarget::All,
                       &mut out, None, &curves, &mut supp, Some(&mut phys), &ctx);
    }
    let p_in: f32 = input.iter().map(|b| b.norm_sqr()).sum();
    let p_out: f32 = out.iter().map(|b| b.norm_sqr()).sum();
    (p_out / p_in, out)
}

#[test]
fn stiction_lets_moving_bins_through_in_a_writer_slot() {
    let (ratio, _) = writer_slot_power_ratio(LifeMode::Stiction, |p| p.velocity[..N].fill(10.0), 60);
    assert!(ratio > 0.5, "fast-moving bins must pass, got power ratio {ratio}");
}

#[test]
fn non_newtonian_reacts_to_velocity_in_a_writer_slot() {
    let (fast, _) = writer_slot_power_ratio(LifeMode::NonNewtonian, |p| p.velocity[..N].fill(10.0), 10);
    let (still, _) = writer_slot_power_ratio(LifeMode::NonNewtonian, |p| p.velocity[..N].fill(0.0), 10);
    assert!((still - 1.0).abs() < 1e-3, "zero velocity passes unchanged, got {still}");
    assert!(fast < still - 0.01, "fast changes must be clamped: fast={fast}, still={still}");
}

#[test]
fn brownian_drifts_hot_bins_in_a_writer_slot() {
    let (_, out) = writer_slot_power_ratio(LifeMode::Brownian, |p| p.temperature[..N].fill(1.0), 4);
    let moved = out.iter().enumerate()
        .filter(|(k, b)| (**b - Complex::from_polar(1.0, *k as f32 * 0.7)).norm() > 1e-4)
        .count();
    assert!(moved > N / 2, "hot bins must drift, only {moved} of {N} moved");
}
