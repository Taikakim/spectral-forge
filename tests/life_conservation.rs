//! Verifies the Life module's energy-conservation invariant for transport modes
//! (Viscosity, SurfaceTension, Capillary, Archimedes). State-creating modes
//! (Crystallization, Yield, Brownian) and rate-limiting modes (NonNewtonian,
//! Stiction) are explicitly exempt — see `ideas/next-gen-modules/11-life.md`
//! § "energy-conservation as the Life invariant".

use spectral_forge::dsp::modules::life::{LifeModule, LifeMode};
use spectral_forge::dsp::modules::{ModuleContext, SpectralModule};
use spectral_forge::params::{StereoLink, FxChannelTarget};
use num_complex::Complex;

fn run_mode(mode: LifeMode, bins_template: &[Complex<f32>], hops: usize) -> Vec<Complex<f32>> {
    let num_bins = bins_template.len();
    let mut module = LifeModule::new();
    module.reset(48_000.0, 2048);
    module.set_mode(mode);

    let amount = vec![1.5_f32; num_bins];
    let thresh = vec![0.5_f32; num_bins];
    let speed = vec![1.0_f32; num_bins];
    let reach = vec![1.0_f32; num_bins];
    // mix=2.0 saturates to fully wet so the kernel's output drives the test, not the dry mix.
    let mix = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &thresh, &speed, &reach, &mix];

    let mut bins = bins_template.to_vec();
    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = ModuleContext::new(
        48_000.0, 2048, num_bins,
        10.0, 100.0, 1.0, 0.0, false, false,
    );

    for _ in 0..hops {
        module.process(
            0, StereoLink::Linked, FxChannelTarget::All,
            &mut bins, None, &curves, &mut suppression, None, &ctx,
        );
    }
    bins
}

fn power(bins: &[Complex<f32>]) -> f32 {
    bins.iter().map(|b| b.norm_sqr()).sum()
}

#[test]
fn viscosity_conserves_power() {
    let num_bins = 1025;
    let mut bins: Vec<Complex<f32>> = vec![Complex::new(0.0, 0.0); num_bins];
    bins[300] = Complex::new(2.0, 0.0);
    let dry_p = power(&bins);

    let wet = run_mode(LifeMode::Viscosity, &bins, 10);
    let wet_p = power(&wet);

    let loss_pct = (dry_p - wet_p).abs() / dry_p;
    // Diffusion is exact-conservative on power except for boundary edge effects.
    // 5% tolerance accommodates them at this hop count.
    assert!(loss_pct < 0.05,
        "Viscosity lost {}% of power (>5% violates conservation)", loss_pct * 100.0);
}

#[test]
fn surface_tension_conserves_magnitude_within_tolerance() {
    let num_bins = 1025;
    let mut bins: Vec<Complex<f32>> = vec![Complex::new(0.0, 0.0); num_bins];
    for k in 200..220 {
        bins[k] = Complex::new(0.7, 0.0);
    }
    let dry_mag: f32 = bins.iter().map(|b| b.norm()).sum();

    let wet = run_mode(LifeMode::SurfaceTension, &bins, 10);
    let wet_mag: f32 = wet.iter().map(|b| b.norm()).sum();

    let loss_pct = (dry_mag - wet_mag).abs() / dry_mag;
    // 10% slack: coalescence redistributes magnitude unevenly across the band.
    assert!(loss_pct < 0.10,
        "SurfaceTension lost {}% of magnitude (>10% violates conservation)", loss_pct * 100.0);
}

#[test]
fn capillary_conserves_magnitude_within_tolerance() {
    let num_bins = 1025;
    let mut bins: Vec<Complex<f32>> = vec![Complex::new(0.0, 0.0); num_bins];
    bins[100] = Complex::new(1.0, 0.0);
    let dry_mag: f32 = bins.iter().map(|b| b.norm()).sum();

    let wet = run_mode(LifeMode::Capillary, &bins, 10);
    let wet_mag: f32 = wet.iter().map(|b| b.norm()).sum();

    let loss_pct = (dry_mag - wet_mag).abs() / dry_mag;
    // 15% slack: harmonic wicking carries energy across larger gaps than diffusion.
    assert!(loss_pct < 0.15,
        "Capillary lost {}% of magnitude (>15% violates conservation)", loss_pct * 100.0);
}

#[test]
fn archimedes_redistributes_without_creation() {
    // Archimedes can REDUCE total but should never INCREASE.
    let num_bins = 1025;
    let bins: Vec<Complex<f32>> = (0..num_bins).map(|_| Complex::new(0.5, 0.0)).collect();
    let dry_mag: f32 = bins.iter().map(|b| b.norm()).sum();

    let wet = run_mode(LifeMode::Archimedes, &bins, 5);
    let wet_mag: f32 = wet.iter().map(|b| b.norm()).sum();

    assert!(wet_mag <= dry_mag * 1.001,
        "Archimedes created energy (dry={}, wet={})", dry_mag, wet_mag);
}
