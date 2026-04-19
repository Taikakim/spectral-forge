use num_complex::Complex;
use crate::dsp::modules::{
    ModuleContext, ModuleType, SpectralModule,
    create_module,
};
use crate::params::{FxChannelTarget, StereoLink};

pub const MAX_SLOTS: usize = 9;    // 8 effect slots + 1 Master slot
pub const MAX_SPLIT_VIRTUAL_ROWS: usize = 4;

pub struct FxMatrix {
    pub slots: Vec<Option<Box<dyn SpectralModule>>>,
    /// Per-slot output buffers (current hop). [slot][bin]
    slot_out: Vec<Vec<Complex<f32>>>,
    /// Per-slot suppression output. [slot][bin]
    slot_supp: Vec<Vec<f32>>,
    /// Virtual row output buffers for T/S Split. [vrow][bin]
    virtual_out: Vec<Vec<Complex<f32>>>,
    /// Working mix buffer (reused each slot, no allocation).
    mix_buf: Vec<Complex<f32>>,
}

impl FxMatrix {
    pub fn new(sample_rate: f32, fft_size: usize) -> Self {
        let num_bins = fft_size / 2 + 1;

        // Default: slots 0,1 = Dynamics; slot 2 = Gain; slot 8 = Master; rest Empty
        let slots: Vec<Option<Box<dyn SpectralModule>>> = (0..MAX_SLOTS).map(|i| {
            let ty = match i {
                0 | 1 => ModuleType::Dynamics,
                2     => ModuleType::Gain,
                8     => ModuleType::Master,
                _     => ModuleType::Empty,
            };
            Some(create_module(ty, sample_rate, fft_size))
        }).collect();

        Self {
            slots,
            slot_out:    (0..MAX_SLOTS).map(|_| vec![Complex::new(0.0, 0.0); num_bins]).collect(),
            slot_supp:   (0..MAX_SLOTS).map(|_| vec![0.0f32; num_bins]).collect(),
            virtual_out: (0..MAX_SPLIT_VIRTUAL_ROWS)
                             .map(|_| vec![Complex::new(0.0, 0.0); num_bins]).collect(),
            mix_buf: vec![Complex::new(0.0, 0.0); num_bins],
        }
    }

    pub fn reset(&mut self, sample_rate: f32, fft_size: usize) {
        let num_bins = fft_size / 2 + 1;
        for slot in self.slots.iter_mut().flatten() {
            slot.reset(sample_rate, fft_size);
        }
        for buf in &mut self.slot_out    { buf.resize(num_bins, Complex::new(0.0, 0.0)); buf.fill(Complex::new(0.0, 0.0)); }
        for buf in &mut self.slot_supp   { buf.resize(num_bins, 0.0); buf.fill(0.0); }
        for buf in &mut self.virtual_out { buf.resize(num_bins, Complex::new(0.0, 0.0)); buf.fill(Complex::new(0.0, 0.0)); }
        self.mix_buf.resize(num_bins, Complex::new(0.0, 0.0));
        self.mix_buf.fill(Complex::new(0.0, 0.0));
    }

    #[allow(clippy::too_many_arguments)]
    pub fn process_hop(
        &mut self,
        channel: usize,
        stereo_link: StereoLink,
        complex_buf: &mut [Complex<f32>],
        sc_args: &[Option<&[f32]>; 9],
        slot_targets: &[FxChannelTarget; 9],
        slot_curves: &Vec<Vec<Vec<f32>>>,  // [slot][curve][bin]
        ctx: &ModuleContext,
        suppression_out: &mut [f32],
        num_bins: usize,
    ) {
        // Process each slot in order (0..MAX_SLOTS).
        // Use .take() / put-back pattern so module borrows don't conflict with self.slots reads.
        for s in 0..MAX_SLOTS {
            let mut module = match self.slots[s].take() {
                Some(m) => m,
                None => {
                    self.slot_out[s][..num_bins].fill(Complex::new(0.0, 0.0));
                    self.slot_supp[s][..num_bins].fill(0.0);
                    continue;
                }
            };

            // Build input for this slot: simple serial chain.
            // Slot 0 reads main input; each subsequent slot reads the previous active slot's output.
            self.mix_buf[..num_bins].fill(Complex::new(0.0, 0.0));
            if s == 0 {
                self.mix_buf[..num_bins].copy_from_slice(&complex_buf[..num_bins]);
            } else {
                // Find the last non-empty slot before s and use its output.
                let mut found = false;
                for prev in (0..s).rev() {
                    if self.slots[prev].is_some() {
                        self.mix_buf[..num_bins].copy_from_slice(&self.slot_out[prev][..num_bins]);
                        found = true;
                        break;
                    }
                }
                if !found {
                    self.mix_buf[..num_bins].copy_from_slice(&complex_buf[..num_bins]);
                }
            }

            // Build curve slice references from slot_curves[s]
            let nc = module.num_curves().min(7);
            let curves_storage: [&[f32]; 7] = std::array::from_fn(|c| {
                if c < nc && s < slot_curves.len() && c < slot_curves[s].len() {
                    let curve = &slot_curves[s][c];
                    &curve[..num_bins.min(curve.len())]
                } else {
                    &[] as &[f32]
                }
            });
            let curves: &[&[f32]] = &curves_storage[..nc];

            let sidechain = sc_args[s];

            module.process(
                channel,
                stereo_link,
                slot_targets[s],
                &mut self.mix_buf[..num_bins],
                sidechain,
                curves,
                &mut self.slot_supp[s][..num_bins],
                ctx,
            );

            self.slot_out[s][..num_bins].copy_from_slice(&self.mix_buf[..num_bins]);

            // Put the module back
            self.slots[s] = Some(module);
        }

        // Master (slot 8) output -> write back to complex_buf
        complex_buf[..num_bins].copy_from_slice(&self.slot_out[8][..num_bins]);

        // Max-reduce suppression across all slots for display
        suppression_out[..num_bins].fill(0.0);
        for s in 0..MAX_SLOTS {
            for k in 0..num_bins {
                if self.slot_supp[s][k] > suppression_out[k] {
                    suppression_out[k] = self.slot_supp[s][k];
                }
            }
        }
    }
}
