//! Integration test: stub module (IfProbe) reads ctx.instantaneous_freq via FxMatrix.
//!
//! Two tests:
//!  1. `if_probe_reads_array_when_present`  — ctx.instantaneous_freq = Some(arr),
//!     probe reads arr[100] and the test asserts it equals 1234.5.
//!  2. `if_probe_observes_none_when_ctx_has_none` — ctx.instantaneous_freq = None,
//!     probe stores 0.0, overwriting the 0xDEADBEEF sentinel.
//!
//! Run with `--test-threads=1` because OBSERVED_IF_BITS is a shared static.

use std::sync::atomic::{AtomicU32, Ordering};

use num_complex::Complex;
use spectral_forge::dsp::{
    fx_matrix::FxMatrix,
    modules::{ModuleContext, ModuleType, RouteMatrix, SpectralModule, MAX_SLOTS, MAX_MATRIX_ROWS},
    pipeline::MAX_NUM_BINS,
};
use spectral_forge::params::{FxChannelTarget, StereoLink};

/// Atomic store for IfProbe to publish the observed IF value (f32 stored as u32 bits).
static OBSERVED_IF_BITS: AtomicU32 = AtomicU32::new(0);

const PROBE_BIN: usize = 100;

// ── IfProbe ───────────────────────────────────────────────────────────────────

struct IfProbe;

impl SpectralModule for IfProbe {
    fn process(
        &mut self,
        _channel: usize,
        _stereo_link: StereoLink,
        _target: FxChannelTarget,
        _bins: &mut [Complex<f32>],
        _sidechain: Option<&[f32]>,
        _curves: &[&[f32]],
        suppression_out: &mut [f32],
        _physics: Option<&mut spectral_forge::dsp::bin_physics::BinPhysics>,
        ctx: &ModuleContext<'_>,
    ) {
        let observed = ctx
            .instantaneous_freq
            .and_then(|a| a.get(PROBE_BIN).copied())
            .unwrap_or(0.0);
        OBSERVED_IF_BITS.store(observed.to_bits(), Ordering::SeqCst);
        suppression_out.fill(0.0);
    }

    fn reset(&mut self, _sample_rate: f32, _fft_size: usize) {}
    fn module_type(&self) -> ModuleType { ModuleType::Gain } // arbitrary non-Empty
    fn num_curves(&self) -> usize { 0 }
}

// ── Shared test helpers ───────────────────────────────────────────────────────

fn build_fm() -> FxMatrix {
    let mut types = [ModuleType::Empty; 9];
    types[8] = ModuleType::Master;
    let mut fm = FxMatrix::new(48000.0, 2048, &types);
    fm.slots[0] = Some(Box::new(IfProbe));
    fm
}

fn build_rm() -> RouteMatrix {
    let mut rm = RouteMatrix::default();
    // Zero out the default serial wiring, then route slot 0 → Master (slot 8).
    rm.send = [[0.0f32; MAX_SLOTS]; MAX_MATRIX_ROWS];
    rm.send[0][8] = 1.0;
    rm
}

// ── Test 1 ────────────────────────────────────────────────────────────────────

#[test]
fn if_probe_reads_array_when_present() {
    let fft_size = 2048_usize;
    let num_bins = fft_size / 2 + 1; // 1025
    let sample_rate = 48000.0_f32;

    let mut fm = build_fm();
    let rm = build_rm();

    // Synthetic IF array: all zeros except bin 100 = 1234.5.
    let mut if_arr = vec![0.0_f32; num_bins];
    if_arr[PROBE_BIN] = 1234.5;

    // Build context then attach the IF slice.
    let mut ctx = ModuleContext::new(
        sample_rate, fft_size, num_bins,
        10.0, 100.0, 0.0, 0.0, false, false,
    );
    ctx.instantaneous_freq = Some(&if_arr[..]);

    let curves: Vec<Vec<Vec<f32>>> = (0..9)
        .map(|_| (0..7).map(|_| vec![1.0f32; MAX_NUM_BINS]).collect())
        .collect();
    let sc: [Option<&[f32]>; 9] = [None; 9];
    let targets = [FxChannelTarget::All; 9];
    let mut supp = vec![0.0f32; num_bins];
    let mut bins: Vec<Complex<f32>> = vec![Complex::new(1.0, 0.0); num_bins];

    OBSERVED_IF_BITS.store(0, Ordering::SeqCst);

    fm.process_hop(
        0, StereoLink::Linked, &mut bins, &sc, &targets,
        &curves, &rm, &ctx, &mut supp, num_bins,
        /*enable_heavy_modules=*/ true,
    );

    let observed = f32::from_bits(OBSERVED_IF_BITS.load(Ordering::SeqCst));
    assert_eq!(
        observed, 1234.5,
        "IfProbe at slot 0 should observe if_arr[{PROBE_BIN}] = 1234.5; got {observed}"
    );
}

// ── Test 2 ────────────────────────────────────────────────────────────────────

#[test]
fn if_probe_observes_none_when_ctx_has_none() {
    let fft_size = 2048_usize;
    let num_bins = fft_size / 2 + 1; // 1025
    let sample_rate = 48000.0_f32;

    let mut fm = build_fm();
    let rm = build_rm();

    // Context with instantaneous_freq = None (constructor default).
    let ctx = ModuleContext::new(
        sample_rate, fft_size, num_bins,
        10.0, 100.0, 0.0, 0.0, false, false,
    );
    // Verify the default is indeed None.
    assert!(ctx.instantaneous_freq.is_none());

    let curves: Vec<Vec<Vec<f32>>> = (0..9)
        .map(|_| (0..7).map(|_| vec![1.0f32; MAX_NUM_BINS]).collect())
        .collect();
    let sc: [Option<&[f32]>; 9] = [None; 9];
    let targets = [FxChannelTarget::All; 9];
    let mut supp = vec![0.0f32; num_bins];
    let mut bins: Vec<Complex<f32>> = vec![Complex::new(1.0, 0.0); num_bins];

    // Pre-seed with a sentinel so we can detect that process ran but stored 0.0.
    OBSERVED_IF_BITS.store(0xDEAD_BEEF, Ordering::SeqCst);

    fm.process_hop(
        0, StereoLink::Linked, &mut bins, &sc, &targets,
        &curves, &rm, &ctx, &mut supp, num_bins,
        /*enable_heavy_modules=*/ true,
    );

    let observed = f32::from_bits(OBSERVED_IF_BITS.load(Ordering::SeqCst));
    assert_eq!(
        observed, 0.0,
        "IfProbe must overwrite the sentinel with 0.0 when ctx.instantaneous_freq is None; \
         got {observed} (bits = {:#010x})",
        OBSERVED_IF_BITS.load(Ordering::SeqCst),
    );
}
