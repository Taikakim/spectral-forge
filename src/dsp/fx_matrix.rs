use num_complex::Complex;
use crate::dsp::modules::{
    ModuleContext, ModuleType, RouteMatrix, GainMode, SpectralModule,
    create_module, MAX_SLOTS, MAX_SPLIT_VIRTUAL_ROWS, VirtualRowKind,
};
use crate::params::{FxChannelTarget, StereoLink};

pub struct FxMatrix {
    pub slots: Vec<Option<Box<dyn SpectralModule>>>,
    slot_out:  Vec<Vec<Complex<f32>>>,
    slot_supp: Vec<Vec<f32>>,
    /// D3: virtual row output buffers for T/S Split — not yet written by process_hop.
    virtual_out: Vec<Vec<Complex<f32>>>,
    mix_buf:   Vec<Complex<f32>>,
}

impl FxMatrix {
    pub fn new(sample_rate: f32, fft_size: usize, slot_types: &[ModuleType; 9]) -> Self {
        let num_bins = fft_size / 2 + 1;
        let slots: Vec<Option<Box<dyn SpectralModule>>> = (0..MAX_SLOTS).map(|i| {
            match slot_types[i] {
                ModuleType::Empty => None,
                ty => Some(create_module(ty, sample_rate, fft_size)),
            }
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
        debug_assert_eq!(self.slot_out[0].len(), num_bins,
            "FxMatrix::reset() called with different fft_size than new()");
        for slot in self.slots.iter_mut().flatten() {
            slot.reset(sample_rate, fft_size);
        }
        for buf in &mut self.slot_out    { buf.fill(Complex::new(0.0, 0.0)); }
        for buf in &mut self.slot_supp   { buf.fill(0.0); }
        for buf in &mut self.virtual_out { buf.fill(Complex::new(0.0, 0.0)); }
        self.mix_buf.fill(Complex::new(0.0, 0.0));
    }

    /// Sync slot modules to the given type array. Called once per audio block.
    /// - Slot going to Empty: drops the existing module (dealloc only, fast).
    /// - Slot getting a new type: creates a module via permit_alloc (intentional
    ///   one-time allocation on user action; not per-sample).
    pub fn sync_slot_types(&mut self, types: &[ModuleType; 9], sample_rate: f32, fft_size: usize) {
        for s in 0..MAX_SLOTS {
            let current = self.slots[s].as_ref().map(|m| m.module_type())
                .unwrap_or(ModuleType::Empty);
            if current == types[s] { continue; }
            if types[s] == ModuleType::Empty {
                nih_plug::util::permit_alloc(|| { self.slots[s] = None; });
            } else {
                nih_plug::util::permit_alloc(|| {
                    self.slots[s] = Some(create_module(types[s], sample_rate, fft_size));
                });
            }
        }
    }

    /// Propagate per-slot GainMode from params to GainModule instances.
    /// Called once per audio block (before process_hop).
    pub fn set_gain_modes(&mut self, modes: &[GainMode; 9]) {
        for s in 0..MAX_SLOTS {
            if let Some(ref mut m) = self.slots[s] {
                m.set_gain_mode(modes[s]);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn process_hop(
        &mut self,
        channel:              usize,
        stereo_link:          StereoLink,
        complex_buf:          &mut [Complex<f32>],
        sc_args:              &[Option<&[f32]>; 9],
        slot_targets:         &[FxChannelTarget; 9],
        slot_curves:          &[Vec<Vec<f32>>],   // [slot][curve][bin]
        route_matrix:         &RouteMatrix,
        ctx:                  &ModuleContext<'_>,
        suppression_out:      &mut [f32],
        num_bins:             usize,
        enable_heavy_modules: bool,
    ) {
        // Clear virtual row output buffers for this hop.
        for v in 0..MAX_SPLIT_VIRTUAL_ROWS {
            self.virtual_out[v][..num_bins].fill(Complex::new(0.0, 0.0));
        }

        for s in 0..8 {  // 0..8, not 0..MAX_SLOTS; Master (slot 8) is handled separately below
            // Build this slot's input from the route matrix.
            // Slot 0 always receives the plugin's main audio input.
            // All slots additionally receive weighted sums of previous-slot outputs.
            self.mix_buf[..num_bins].fill(Complex::new(0.0, 0.0));
            if s == 0 {
                self.mix_buf[..num_bins].copy_from_slice(&complex_buf[..num_bins]);
            }
            for src in 0..s {
                let send = route_matrix.send[src][s];
                if send < 0.001 { continue; }
                for k in 0..num_bins {
                    self.mix_buf[k] += self.slot_out[src][k] * send;
                }
            }
            // Accumulate from virtual rows (T/S Split transient/sustained outputs).
            for (v, &vrow) in route_matrix.virtual_rows.iter().enumerate() {
                if let Some((src_slot, _kind)) = vrow {
                    if (src_slot as usize) < s {
                        let send = route_matrix.send[MAX_SLOTS + v][s];
                        if send < 0.001 { continue; }
                        for k in 0..num_bins {
                            self.mix_buf[k] += self.virtual_out[v][k] * send;
                        }
                    }
                }
            }

            let mut module = match self.slots[s].take() {
                Some(m) => m,
                None => {
                    self.slot_out[s][..num_bins].copy_from_slice(&self.mix_buf[..num_bins]);
                    self.slot_supp[s][..num_bins].fill(0.0);
                    continue;
                }
            };

            let nc = module.num_curves().min(7);
            let curves_storage: [&[f32]; 7] = std::array::from_fn(|c| {
                if c < nc && s < slot_curves.len() && c < slot_curves[s].len() {
                    let cv = &slot_curves[s][c];
                    &cv[..num_bins.min(cv.len())]
                } else {
                    &[] as &[f32]
                }
            });
            let curves: &[&[f32]] = &curves_storage[..nc];

            if !enable_heavy_modules && module.heavy_cpu_for_mode() {
                // Short-circuit: copy input to output, leave suppression at 0.
                self.slot_out[s][..num_bins].copy_from_slice(&self.mix_buf[..num_bins]);
                self.slot_supp[s][..num_bins].fill(0.0);
                // If this slot declares virtual outputs (e.g. a future heavy T/S-Split),
                // publish slot_out[s] (the passthrough) into every virtual row it owns.
                // Using the module's internal buffers here would be wrong: they contain
                // stale data from the last processed hop, not the bypassed signal.
                if module.virtual_outputs().is_some() {
                    for (v, &vrow) in route_matrix.virtual_rows.iter().enumerate() {
                        if let Some((src_slot, _kind)) = vrow {
                            if src_slot as usize == s {
                                let copy_len = num_bins.min(self.slot_out[s].len());
                                self.virtual_out[v][..copy_len]
                                    .copy_from_slice(&self.slot_out[s][..copy_len]);
                            }
                        }
                    }
                }
            } else {
                module.process(
                    channel, stereo_link, slot_targets[s],
                    &mut self.mix_buf[..num_bins],
                    sc_args[s], curves,
                    &mut self.slot_supp[s][..num_bins],
                    ctx,
                );
                self.slot_out[s][..num_bins].copy_from_slice(&self.mix_buf[..num_bins]);

                // Populate virtual row buffers from split modules.
                if let Some(vouts) = module.virtual_outputs() {
                    for (v, &vrow) in route_matrix.virtual_rows.iter().enumerate() {
                        if let Some((src_slot, kind)) = vrow {
                            if src_slot as usize == s {
                                let src_buf = match kind {
                                    VirtualRowKind::Transient => vouts[0],
                                    VirtualRowKind::Sustained  => vouts[1],
                                };
                                let copy_len = num_bins.min(src_buf.len());
                                self.virtual_out[v][..copy_len].copy_from_slice(&src_buf[..copy_len]);
                            }
                        }
                    }
                }
            }

            self.slots[s] = Some(module);
        }

        // Master output: accumulate sends to slot 8.
        // If nothing routes to Master, mix_buf stays zeroed → silence.
        self.mix_buf[..num_bins].fill(Complex::new(0.0, 0.0));
        for src in 0..8 {
            let send = route_matrix.send[src][8];
            if send < 0.001 { continue; }
            for k in 0..num_bins {
                self.mix_buf[k] += self.slot_out[src][k] * send;
            }
        }
        // Pass through Master module (slot 8) then write to complex_buf.
        if let Some(ref mut master_mod) = self.slots[8] {
            let curves_empty: &[&[f32]] = &[];
            master_mod.process(
                channel, stereo_link, slot_targets[8],
                &mut self.mix_buf[..num_bins],
                sc_args[8], curves_empty,
                &mut self.slot_supp[8][..num_bins],
                ctx,
            );
        }
        complex_buf[..num_bins].copy_from_slice(&self.mix_buf[..num_bins]);

        // Max-reduce suppression across all slots for display.
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
