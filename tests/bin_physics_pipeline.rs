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
