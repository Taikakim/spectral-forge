//! End-to-end test for the Phase 3 BinPhysics writer→reader path.
//!
//! Slot 0 (MockWriter) sets `physics.mass[k] = 5.0` for all bins.
//! Slot 1 (MockReader) reads `ctx.bin_physics.mass[100]` and stores it
//! into a static atomic for the test to read back.
//!
//! Asserts the reader observed mass = 5.0, proving:
//! - FxMatrix calls writer with Some(&mut mix_phys)
//! - Phase 3.4 copies mix_phys → slot_phys[0] after writer returns
//! - Phase 3.4 mix_from blends slot_phys[0] into mix_phys for slot 1
//!   via route_matrix.send[0][1] = 1.0  (HeavierWins: max(1.0, 5.0) = 5.0)
//! - Phase 3.5 hands the reader physics=None + ctx.bin_physics=Some(&mix_phys)

use std::sync::atomic::{AtomicU32, Ordering};

use num_complex::Complex;
use spectral_forge::dsp::{
    bin_physics::BinPhysics,
    fx_matrix::FxMatrix,
    modules::{ModuleContext, ModuleType, RouteMatrix, MAX_SLOTS, MAX_MATRIX_ROWS},
    pipeline::MAX_NUM_BINS,
};
use spectral_forge::params::{FxChannelTarget, StereoLink};

/// Atomic for MockReader to publish its observation. f32 stored as u32 bits.
static OBSERVED_MASS_BITS: AtomicU32 = AtomicU32::new(0);

const TEST_MASS: f32 = 5.0;
const PROBE_BIN: usize = 100;

// ── MockWriter: writes mass = 5.0 for every active bin ────────────────────────

struct MockWriter;
impl spectral_forge::dsp::modules::SpectralModule for MockWriter {
    fn process(
        &mut self,
        _channel: usize,
        _stereo_link: StereoLink,
        _target: FxChannelTarget,
        _bins: &mut [Complex<f32>],
        _sidechain: Option<&[f32]>,
        _curves: &[&[f32]],
        suppression_out: &mut [f32],
        physics: Option<&mut BinPhysics>,
        ctx: &ModuleContext<'_>,
    ) {
        // Verify writer side of dispatch: physics must be Some.
        let p = physics.expect(
            "MockWriter installed at writer slot must receive physics: Some(&mut)",
        );
        for k in 0..ctx.num_bins {
            p.mass[k] = TEST_MASS;
        }
        suppression_out.fill(0.0);
        // ctx.bin_physics must be None for writers (Phase 3.5 contract).
        debug_assert!(
            ctx.bin_physics.is_none(),
            "writer slot must have ctx.bin_physics = None"
        );
    }
    fn reset(&mut self, _sample_rate: f32, _fft_size: usize) {}
    fn module_type(&self) -> ModuleType { ModuleType::Gain } // arbitrary non-Empty
    fn num_curves(&self) -> usize { 0 }
}

// ── MockReader: observes ctx.bin_physics.mass[PROBE_BIN] ──────────────────────

struct MockReader;
impl spectral_forge::dsp::modules::SpectralModule for MockReader {
    fn process(
        &mut self,
        _channel: usize,
        _stereo_link: StereoLink,
        _target: FxChannelTarget,
        _bins: &mut [Complex<f32>],
        _sidechain: Option<&[f32]>,
        _curves: &[&[f32]],
        suppression_out: &mut [f32],
        physics: Option<&mut BinPhysics>,
        ctx: &ModuleContext<'_>,
    ) {
        // Verify reader side: physics must be None, ctx.bin_physics must be Some.
        debug_assert!(physics.is_none(), "reader slot must have physics: None");
        let bp = ctx
            .bin_physics
            .expect("reader slot must receive ctx.bin_physics = Some(&)");
        let observed = bp.mass[PROBE_BIN];
        OBSERVED_MASS_BITS.store(observed.to_bits(), Ordering::SeqCst);
        suppression_out.fill(0.0);
    }
    fn reset(&mut self, _sample_rate: f32, _fft_size: usize) {}
    fn module_type(&self) -> ModuleType { ModuleType::Gain }
    fn num_curves(&self) -> usize { 0 }
}

#[test]
fn writer_sets_mass_then_reader_observes_it() {
    let n = 1025usize; // num_bins for fft_size = 2048
    let fft_size = 2048usize;
    let sample_rate = 48000.0_f32;

    // Build FxMatrix with all-Empty slots except Master at 8.
    let mut types = [ModuleType::Empty; 9];
    types[8] = ModuleType::Master;
    let mut fm = FxMatrix::new(sample_rate, fft_size, &types);

    // Install mock modules at slots 0 and 1, then force slot 0 as a BinPhysics writer.
    fm.slots[0] = Some(Box::new(MockWriter));
    fm.slots[1] = Some(Box::new(MockReader));
    fm.test_force_writer(0);

    // Route: slot 0 → slot 1 → Master, all unit gain.
    let mut rm = RouteMatrix::default();
    rm.send = [[0.0f32; MAX_SLOTS]; MAX_MATRIX_ROWS];
    rm.send[0][1] = 1.0;
    rm.send[1][8] = 1.0;

    // Synthetic process_hop args.
    let curves: Vec<Vec<Vec<f32>>> = (0..9)
        .map(|_| (0..7).map(|_| vec![1.0f32; MAX_NUM_BINS]).collect())
        .collect();
    let sc: [Option<&[f32]>; 9] = [None; 9];
    let targets = [FxChannelTarget::All; 9];
    let ctx = ModuleContext::new(
        sample_rate, fft_size, n,
        10.0, 100.0, 0.0, 0.0, false, false,
    );
    let mut supp = vec![0.0f32; n];

    // Reset observed before the hop.
    OBSERVED_MASS_BITS.store(0, Ordering::SeqCst);

    let input_mag = 1.0_f32;
    let mut bins: Vec<Complex<f32>> = vec![Complex::new(input_mag, 0.0); n];
    fm.process_hop(
        0, StereoLink::Linked, &mut bins, &sc, &targets,
        &curves, &rm, &ctx, &mut supp, n,
        /*enable_heavy_modules=*/ true,
    );

    let observed = f32::from_bits(OBSERVED_MASS_BITS.load(Ordering::SeqCst));
    assert!(
        (observed - TEST_MASS).abs() < 1e-5,
        "MockReader at slot 1 should observe mass={TEST_MASS} written by MockWriter at slot 0; got {observed}"
    );
}

// ── phase_test_ctx helper ─────────────────────────────────────────────────────

fn phase_test_ctx<'a>(
    num_bins: usize,
    bin_physics: Option<&'a BinPhysics>,
) -> ModuleContext<'a> {
    let mut ctx = ModuleContext::new(48_000.0, 2048, num_bins, 10.0, 100.0, 1.0, 1.0, false, false);
    ctx.bin_physics = bin_physics;
    ctx
}

// ── Circuit BinPhysics integration tests ─────────────────────────────────────

#[test]
fn circuit_transformer_writes_flux_visible_to_next_slot() {
    use spectral_forge::dsp::modules::circuit::{CircuitModule, CircuitMode};
    use spectral_forge::dsp::modules::SpectralModule;
    use spectral_forge::params::StereoLink;
    use spectral_forge::params::FxChannelTarget;

    let mut module = CircuitModule::new();
    module.reset(48_000.0, 2048);
    module.set_circuit_mode(CircuitMode::TransformerSaturation);

    let num_bins = 1025;
    let mut bins: Vec<Complex<f32>> = vec![Complex::new(3.0, 0.0); num_bins];

    let amount  = vec![2.0_f32; num_bins];
    let thresh  = vec![1.0_f32; num_bins];
    let spread  = vec![0.0_f32; num_bins];
    let release = vec![1.0_f32; num_bins];
    let mix     = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &thresh, &spread, &release, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let mut physics = BinPhysics::new();
    physics.reset_active(num_bins, 48_000.0, 2048);

    let ctx = phase_test_ctx(num_bins, None);

    // Several hops to let xfmr_lp settle and accumulate flux.
    for _ in 0..40 {
        for b in bins.iter_mut() { *b = Complex::new(3.0, 0.0); }
        module.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, None, &curves, &mut suppression, Some(&mut physics), &ctx);
    }

    // Flux should have built up where the magnitude was saturating. With constant
    // input mag=3.0, knee=1.0, and 40 hops to settle, every bin's flux contribution
    // (≈ 2 × (lp - sat_mag) ≈ 2 × 2.1 ≈ 4.2) sums to ~4300 across 1025 bins. Use a
    // threshold an order of magnitude below the analytical mean to catch any
    // regression that drops most of the write energy without flaking on numeric
    // jitter.
    let total_flux: f32 = physics.flux[..num_bins].iter().sum();
    assert!(total_flux > 500.0, "Transformer should write substantial flux; total = {}", total_flux);
}

#[test]
fn circuit_vactrol_reads_incoming_flux() {
    use spectral_forge::dsp::modules::circuit::{CircuitModule, CircuitMode};
    use spectral_forge::dsp::modules::SpectralModule;
    use spectral_forge::dsp::modules::circuit::CircuitProbe;
    use spectral_forge::params::StereoLink;
    use spectral_forge::params::FxChannelTarget;

    // A/B test: Vactrol must read flux through the writer-slot `physics` arg, not
    // `ctx.bin_physics`. Run twice with identical inputs except for the flux field
    // and assert the flux-set run charges its slow cap strictly more than the
    // zero-flux baseline. Mirrors the FxMatrix writer-slot pattern:
    // `physics = Some(&mut mix_phys)` and `ctx.bin_physics = None`.
    let num_bins = 1025;

    let amount  = vec![1.0_f32; num_bins];
    let thresh  = vec![1.0_f32; num_bins];
    let spread  = vec![0.0_f32; num_bins];
    let release = vec![1.0_f32; num_bins];
    let mix     = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &thresh, &spread, &release, &mix];

    fn run(num_bins: usize, curves: &[&[f32]], flux_value: f32) -> f32 {
        let mut module = CircuitModule::new();
        module.reset(48_000.0, 2048);
        module.set_circuit_mode(CircuitMode::Vactrol);

        let mut suppression = vec![0.0_f32; num_bins];
        let mut physics = BinPhysics::new();
        physics.reset_active(num_bins, 48_000.0, 2048);

        let mut bins: Vec<Complex<f32>> = vec![Complex::new(0.5, 0.0); num_bins];
        let ctx = phase_test_ctx(num_bins, None);

        for _ in 0..50 {
            // Re-seed flux every hop: Vactrol does not write `physics`, but other
            // writer-slot modes that set `physics: Some(&mut)` would; keep this
            // pattern consistent so the flux value the kernel reads is exact.
            for k in 0..num_bins { physics.flux[k] = flux_value; }
            for b in bins.iter_mut() { *b = Complex::new(0.5, 0.0); }
            module.process(
                0, StereoLink::Linked, FxChannelTarget::All,
                &mut bins, None, curves, &mut suppression,
                Some(&mut physics), &ctx,
            );
        }

        let probe: CircuitProbe = module.probe_state(0);
        probe.vactrol_slow_avg
    }

    let avg_flux_set  = run(num_bins, &curves, 4.0);
    let avg_zero_flux = run(num_bins, &curves, 0.0);

    // Drive in flux-set run is `flux.abs() * amount = 4.0`; in zero-flux run drive is
    // 0.0, so slow cap settles to 0. Gap should be substantial — well above any
    // numeric jitter — confirming Vactrol reads `physics.flux`, not the fallback
    // (`dry.norm()`) or `ctx.bin_physics`.
    assert!(
        avg_flux_set > avg_zero_flux + 1.0,
        "Vactrol must charge from physics.flux: flux=4.0 → slow_avg={}, flux=0.0 → slow_avg={}",
        avg_flux_set, avg_zero_flux,
    );
}

#[test]
fn circuit_bias_fuzz_roundtrips_bias_field() {
    use spectral_forge::dsp::modules::circuit::{CircuitModule, CircuitMode};
    use spectral_forge::dsp::modules::SpectralModule;
    use spectral_forge::params::StereoLink;
    use spectral_forge::params::FxChannelTarget;

    let mut module = CircuitModule::new();
    module.reset(48_000.0, 2048);
    module.set_circuit_mode(CircuitMode::BiasFuzz);

    let num_bins = 1025;

    let amount  = vec![2.0_f32; num_bins];
    let thresh  = vec![1.0_f32; num_bins];
    let spread  = vec![0.0_f32; num_bins];
    let release = vec![0.1_f32; num_bins];
    let mix     = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &thresh, &spread, &release, &mix];
    let mut suppression = vec![0.0_f32; num_bins];

    let mut physics = BinPhysics::new();
    physics.reset_active(num_bins, 48_000.0, 2048);

    let ctx = phase_test_ctx(num_bins, None);

    let mut bins: Vec<Complex<f32>> = vec![Complex::new(2.0, 0.0); num_bins];
    for _ in 0..100 {
        for b in bins.iter_mut() { *b = Complex::new(2.0, 0.0); }
        module.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, None, &curves, &mut suppression, Some(&mut physics), &ctx);
    }

    // Bias Fuzz writes bias_out = 0.95*bias_out + 0.05*bias_lp every hop, where
    // bias_lp converges toward in_mag = 2.0. After 100 hops bias_out is near 2.0
    // per bin; total ≈ 2050 across 1025 bins. Use ~10% of analytical mean to catch
    // regressions that drop most write energy without flaking on numeric jitter.
    let total_bias: f32 = physics.bias[..num_bins].iter().sum();
    assert!(total_bias > 200.0, "Bias Fuzz should write substantial bias; total = {}", total_bias);
}
