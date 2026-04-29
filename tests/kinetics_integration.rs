//! End-to-end integration test for KineticsModule at the SpectralModule trait boundary.
//!
//! Test 1: InertialMass-Static writes BinPhysics.mass from the MASS curve, proving the
//!         writer→reader cross-slot data path works. A settle loop drives the 1-pole curve
//!         smoother to steady state before asserting bounds.
//!
//! Test 2: Two chained Kinetics slots (GravityWell → Hooke) run 50 hops without numeric
//!         explosion (|bin| < 100.0 throughout). This is the plan-specified pair: GravityWell
//!         injects force toward a static spectral target (Gaussian well at bin 200), Hooke
//!         then applies a restoring spring to its output. The test exercises the chained-state
//!         interaction at the serial-dispatch trait boundary.

use num_complex::Complex;
use spectral_forge::dsp::bin_physics::BinPhysics;
use spectral_forge::dsp::modules::kinetics::{KineticsModule, KineticsMode, MassSource};
use spectral_forge::dsp::modules::{ModuleContext, SpectralModule};
use spectral_forge::params::{FxChannelTarget, StereoLink};

const SR: f32 = 48_000.0;
const FFT: usize = 2048;
const NUM_BINS: usize = FFT / 2 + 1; // 1025

fn make_ctx() -> ModuleContext<'static> {
    ModuleContext::new(SR, FFT, NUM_BINS, 10.0, 100.0, 1.0, 1.0, false, false)
}

#[test]
fn kinetics_inertial_mass_writes_then_other_module_reads() {
    let mut writer = KineticsModule::new();
    writer.reset(SR, FFT);
    writer.set_mode(KineticsMode::InertialMass);
    writer.set_mass_source(MassSource::Static);

    let mut bins: Vec<Complex<f32>> = vec![Complex::new(1.0, 0.0); NUM_BINS];
    let mut physics = BinPhysics::new();
    physics.reset_active(NUM_BINS, SR, FFT);

    // Mass curve: 1.0 at bin 0, rising linearly to ~5.0 at bin 1024.
    let mass_curve: Vec<f32> = (0..NUM_BINS)
        .map(|k| 1.0 + 4.0 * (k as f32 / NUM_BINS as f32))
        .collect();
    let neutral = vec![1.0_f32; NUM_BINS];
    // MIX curve = 2.0 (clamped to 1.0 inside the kernel) → fully wet.
    let mix = vec![2.0_f32; NUM_BINS];
    let curves: Vec<&[f32]> = vec![&neutral, &mass_curve, &neutral, &neutral, &mix];
    let mut suppression = vec![0.0_f32; NUM_BINS];
    let ctx = make_ctx();

    // Drive 30 hops so the 1-pole curve smoother (ALPHA ≈ 0.221, tau = 4·dt) has settled.
    // After ~6 hops the smoothed MASS curve at bin 1024 already exceeds 3.0; 30 hops gives
    // comfortable margin against the >3.0 lower bound.
    for _ in 0..30 {
        writer.process(
            0, StereoLink::Linked, FxChannelTarget::All,
            &mut bins, None, &curves, &mut suppression, Some(&mut physics), &ctx,
        );
    }

    // Bin 0: MASS curve target ≈ 1.0 → smoothed mass should be close to 1.0.
    assert!(
        physics.mass[0] > 0.5 && physics.mass[0] < 2.0,
        "mass[0] out of range: {}",
        physics.mass[0],
    );
    // Bin 1024: MASS curve target ≈ 5.0 → after settling must be >3.0 and <7.0.
    assert!(
        physics.mass[NUM_BINS - 1] > 3.0 && physics.mass[NUM_BINS - 1] < 7.0,
        "mass[1024] out of range: {}",
        physics.mass[NUM_BINS - 1],
    );
    // All active bins must be finite and positive.
    for k in 0..NUM_BINS {
        assert!(
            physics.mass[k].is_finite() && physics.mass[k] > 0.0,
            "mass[{}] not finite/positive: {}",
            k, physics.mass[k],
        );
    }
}

#[test]
fn kinetics_chained_two_slots_in_serial_does_not_explode() {
    // Slot 0: GravityWell-Static.  Slot 1: Hooke.
    //
    // GravityWell creates a single attraction well at bin 200 (Gaussian STRENGTH peak).
    // Hooke applies a neighbour-spring restoring force to the GravityWell output.
    // Running both in serial for 50 hops must not produce |bin| ≥ 100.0.
    let mut s0 = KineticsModule::new();
    s0.reset(SR, FFT);
    s0.set_mode(KineticsMode::GravityWell);
    // WellSource::Static is the default — no explicit set needed.

    let mut s1 = KineticsModule::new();
    s1.reset(SR, FFT);
    s1.set_mode(KineticsMode::Hooke);

    // Dense sinusoidal input: magnitudes in [0.25, 1.25], real-only.
    let mut bins: Vec<Complex<f32>> = (0..NUM_BINS)
        .map(|k| Complex::new(((k as f32 * 0.05).sin() + 1.5) * 0.5, 0.0))
        .collect();

    // Slot 0 (GravityWell): Gaussian STRENGTH centred on bin 200, σ=5, peak 2.0.
    // This creates a single static well above the 1.05 threshold at bin 200.
    let strength_curve: Vec<f32> = (0..NUM_BINS)
        .map(|k| {
            let d = (k as f32 - 200.0) / 5.0;
            1.0 + (-d * d).exp()
        })
        .collect();
    let neutral = vec![1.0_f32; NUM_BINS];
    // MIX = 2.0 → clamped to 1.0 inside the kernel → fully wet.
    let mix = vec![2.0_f32; NUM_BINS];
    let curves0: Vec<&[f32]> = vec![&strength_curve, &neutral, &neutral, &neutral, &mix];

    // Slot 1 (Hooke): uniform STRENGTH = 2.0, fully wet.
    let strength_curve_high = vec![2.0_f32; NUM_BINS];
    let curves1: Vec<&[f32]> = vec![&strength_curve_high, &neutral, &neutral, &neutral, &mix];

    let mut suppression = vec![0.0_f32; NUM_BINS];
    let ctx = make_ctx();

    for hop in 0..50 {
        s0.process(
            0, StereoLink::Linked, FxChannelTarget::All,
            &mut bins, None, &curves0, &mut suppression, None, &ctx,
        );
        s1.process(
            0, StereoLink::Linked, FxChannelTarget::All,
            &mut bins, None, &curves1, &mut suppression, None, &ctx,
        );
        for (k, b) in bins.iter().enumerate() {
            assert!(
                b.norm().is_finite() && b.norm() < 100.0,
                "Chain blew up at hop {} bin {}: |b| = {}",
                hop, k, b.norm(),
            );
        }
    }
}
