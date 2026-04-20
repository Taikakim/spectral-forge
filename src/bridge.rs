use parking_lot::Mutex;
use std::sync::{Arc, atomic::{AtomicBool, AtomicUsize, Ordering}};
use triple_buffer::{TripleBuffer, Input as TbInput, Output as TbOutput};

pub const NUM_CURVES: usize = 7;
pub const CURVE_THRESHOLD: usize = 0;
pub const CURVE_RATIO:     usize = 1;
pub const CURVE_ATTACK:    usize = 2;
pub const CURVE_RELEASE:   usize = 3;
pub const CURVE_KNEE:      usize = 4;
pub const CURVE_MAKEUP:    usize = 5;
pub const CURVE_MIX:       usize = 6;

pub const NUM_SLOTS: usize = 9;

pub struct SharedState {
    /// Always MAX_NUM_BINS — kept for backward compat; use fft_size for current active bin count.
    pub num_bins: usize,

    /// Current active FFT size (power of 2). GUI reads this to know how many bins are valid.
    pub fft_size: Arc<AtomicUsize>,

    /// curve_tx[slot][curve] = GUI write handle. 9 slots × 7 curves.
    pub curve_tx: Vec<Vec<Arc<Mutex<TbInput<Vec<f32>>>>>>,
    /// curve_rx[slot][curve] = audio-thread read handle.
    pub curve_rx: Vec<Vec<TbOutput<Vec<f32>>>>,

    /// Whether each of the 4 aux sidechain inputs is carrying signal.
    pub sidechain_active: [Arc<AtomicBool>; 4],

    // Audio → GUI
    pub spectrum_tx:    TbInput<Vec<f32>>,
    pub spectrum_rx:    Arc<Mutex<TbOutput<Vec<f32>>>>,
    pub suppression_tx: TbInput<Vec<f32>>,
    pub suppression_rx: Arc<Mutex<TbOutput<Vec<f32>>>>,

    pub sample_rate: Arc<AtomicF32>,
}

/// Wait-free f32 atomic using bit-casting.
#[derive(Default)]
pub struct AtomicF32(std::sync::atomic::AtomicU32);

impl AtomicF32 {
    pub fn new(v: f32) -> Self {
        Self(std::sync::atomic::AtomicU32::new(v.to_bits()))
    }
    pub fn load(&self) -> f32 {
        f32::from_bits(self.0.load(std::sync::atomic::Ordering::Relaxed))
    }
    pub fn store(&self, v: f32) {
        self.0.store(v.to_bits(), std::sync::atomic::Ordering::Relaxed)
    }
}

impl SharedState {
    /// Create a SharedState pre-allocated at MAX_NUM_BINS.
    /// `initial_fft_size` is the default FFT size (stored in the AtomicUsize).
    pub fn new(initial_fft_size: usize, sample_rate: f32) -> Self {
        use crate::dsp::pipeline::MAX_NUM_BINS;
        let zero_bins = vec![0.0f32; MAX_NUM_BINS];

        // Build 9×7 triple buffers at MAX_NUM_BINS, initialized to 1.0 (neutral) for all curves.
        let mut curve_tx = Vec::with_capacity(NUM_SLOTS);
        let mut curve_rx = Vec::with_capacity(NUM_SLOTS);
        for _ in 0..NUM_SLOTS {
            let mut slot_tx = Vec::with_capacity(NUM_CURVES);
            let mut slot_rx = Vec::with_capacity(NUM_CURVES);
            for _ in 0..NUM_CURVES {
                let init = vec![1.0f32; MAX_NUM_BINS];
                let (tx, rx) = TripleBuffer::new(&init).split();
                slot_tx.push(Arc::new(Mutex::new(tx)));
                slot_rx.push(rx);
            }
            curve_tx.push(slot_tx);
            curve_rx.push(slot_rx);
        }

        let (spectrum_tx, spectrum_rx) = TripleBuffer::new(&zero_bins).split();
        let (suppression_tx, suppression_rx) = TripleBuffer::new(&zero_bins).split();

        Self {
            num_bins: MAX_NUM_BINS,
            fft_size: Arc::new(AtomicUsize::new(initial_fft_size)),
            curve_tx,
            curve_rx,
            sidechain_active: std::array::from_fn(|_| Arc::new(AtomicBool::new(false))),
            spectrum_tx,
            spectrum_rx: Arc::new(Mutex::new(spectrum_rx)),
            suppression_tx,
            suppression_rx: Arc::new(Mutex::new(suppression_rx)),
            sample_rate: Arc::new(AtomicF32::new(sample_rate)),
        }
    }
}
