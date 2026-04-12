use parking_lot::Mutex;
use std::sync::{Arc, atomic::{AtomicBool, AtomicU8}};
use triple_buffer::{TripleBuffer, Input as TbInput, Output as TbOutput};

pub const NUM_CURVES: usize = 7;
pub const CURVE_THRESHOLD: usize = 0;
pub const CURVE_RATIO:     usize = 1;
pub const CURVE_ATTACK:    usize = 2;
pub const CURVE_RELEASE:   usize = 3;
pub const CURVE_KNEE:      usize = 4;
pub const CURVE_MAKEUP:    usize = 5;
pub const CURVE_MIX:       usize = 6;

pub struct SharedState {
    pub num_bins: usize,

    // GUI → Audio (one channel per parameter curve)
    pub curve_tx: Vec<Arc<Mutex<TbInput<Vec<f32>>>>>,
    pub curve_rx: Vec<TbOutput<Vec<f32>>>,

    // Audio → GUI
    pub spectrum_tx:    TbInput<Vec<f32>>,
    pub spectrum_rx:    Arc<Mutex<TbOutput<Vec<f32>>>>,
    pub suppression_tx: TbInput<Vec<f32>>,
    pub suppression_rx: Arc<Mutex<TbOutput<Vec<f32>>>>,

    // Scalars (written once at initialize, read by GUI)
    pub sample_rate:      Arc<AtomicF32>,
    pub pending_engine:   Arc<AtomicU8>,
    pub sidechain_active: Arc<AtomicBool>,
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
    pub fn new(num_bins: usize, sample_rate: f32) -> Self {
        let zero_bins = vec![0.0f32; num_bins];

        let mut curve_tx = Vec::with_capacity(NUM_CURVES);
        let mut curve_rx = Vec::with_capacity(NUM_CURVES);

        // Default values per curve: threshold=-20dB, ratio=4, attack=10ms,
        // release=80ms, knee=6dB, makeup=0dB, mix=1.0
        let defaults: [f32; NUM_CURVES] = [-20.0, 4.0, 10.0, 80.0, 6.0, 0.0, 1.0];
        for i in 0..NUM_CURVES {
            let init = vec![defaults[i]; num_bins];
            let (tx, rx) = TripleBuffer::new(&init).split();
            curve_tx.push(Arc::new(Mutex::new(tx)));
            curve_rx.push(rx);
        }

        let (spectrum_tx, spectrum_rx) = TripleBuffer::new(&zero_bins).split();
        let (suppression_tx, suppression_rx) = TripleBuffer::new(&zero_bins).split();

        Self {
            num_bins,
            curve_tx,
            curve_rx,
            spectrum_tx,
            spectrum_rx: Arc::new(Mutex::new(spectrum_rx)),
            suppression_tx,
            suppression_rx: Arc::new(Mutex::new(suppression_rx)),
            sample_rate: Arc::new(AtomicF32::new(sample_rate)),
            pending_engine: Arc::new(AtomicU8::new(0)),
            sidechain_active: Arc::new(AtomicBool::new(false)),
        }
    }
}
