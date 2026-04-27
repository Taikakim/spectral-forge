use num_complex::Complex;
use crate::dsp::modules::{
    ModuleContext, ModuleType, RouteMatrix, GainMode, FutureMode, SpectralModule,
    create_module, MAX_SLOTS, MAX_SPLIT_VIRTUAL_ROWS, MAX_MATRIX_ROWS, VirtualRowKind,
};
use crate::dsp::modules::circuit::CircuitMode;
use crate::dsp::modules::geometry::GeometryMode;
use crate::dsp::modules::modulate::ModulateMode;
use crate::dsp::modules::punch::PunchMode;
use crate::dsp::modules::rhythm::{RhythmMode, ArpGrid};
use crate::dsp::amp_modes::AmpNodeState;
use crate::dsp::pipeline::MAX_NUM_BINS;
use crate::params::{FxChannelTarget, StereoLink};

pub struct FxMatrix {
    pub slots: Vec<Option<Box<dyn SpectralModule>>>,
    slot_out:  Vec<Vec<Complex<f32>>>,
    slot_supp: Vec<Vec<f32>>,
    /// D3: virtual row output buffers for T/S Split — not yet written by process_hop.
    virtual_out: Vec<Vec<Complex<f32>>>,
    mix_buf:   Vec<Complex<f32>>,
    /// Per-channel × row × col amp state. Channel 0 always present;
    /// channel 1 used only for Independent / MidSide stereo.
    pub amp_state: [Vec<Vec<AmpNodeState>>; 2],
    /// Scratch buffer for amp transforms (so we apply amp to a copy of each source,
    /// not the slot's own output buffer).
    amp_scratch: Vec<Complex<f32>>,
}

impl FxMatrix {
    pub fn new(sample_rate: f32, fft_size: usize, slot_types: &[ModuleType; 9]) -> Self {
        let slots: Vec<Option<Box<dyn SpectralModule>>> = (0..MAX_SLOTS).map(|i| {
            match slot_types[i] {
                ModuleType::Empty => None,
                ty => Some(create_module(ty, sample_rate, fft_size)),
            }
        }).collect();
        let mk_amp_grid = || (0..MAX_MATRIX_ROWS).map(|_|
            (0..MAX_SLOTS).map(|_| AmpNodeState::Linear).collect()
        ).collect();
        // Internal buffers sized to MAX_NUM_BINS so variable-FFT changes never need to
        // reallocate; process_hop and reset() just slice into [..num_bins].
        Self {
            slots,
            slot_out:    (0..MAX_SLOTS).map(|_| vec![Complex::new(0.0, 0.0); MAX_NUM_BINS]).collect(),
            slot_supp:   (0..MAX_SLOTS).map(|_| vec![0.0f32; MAX_NUM_BINS]).collect(),
            virtual_out: (0..MAX_SPLIT_VIRTUAL_ROWS)
                             .map(|_| vec![Complex::new(0.0, 0.0); MAX_NUM_BINS]).collect(),
            mix_buf: vec![Complex::new(0.0, 0.0); MAX_NUM_BINS],
            amp_state: [mk_amp_grid(), mk_amp_grid()],
            amp_scratch: vec![Complex::new(0.0, 0.0); MAX_NUM_BINS],
        }
    }

    pub fn reset(&mut self, sample_rate: f32, fft_size: usize) {
        for slot in self.slots.iter_mut().flatten() {
            slot.reset(sample_rate, fft_size);
        }
        for buf in &mut self.slot_out    { buf.fill(Complex::new(0.0, 0.0)); }
        for buf in &mut self.slot_supp   { buf.fill(0.0); }
        for buf in &mut self.virtual_out { buf.fill(Complex::new(0.0, 0.0)); }
        self.mix_buf.fill(Complex::new(0.0, 0.0));
        self.amp_scratch.fill(Complex::new(0.0, 0.0));
        self.clear_amp_state();
    }

    /// Sync per-cell amp state to match the requested amp_modes in `rm`.
    /// On mismatch: drops the old state (dealloc) and creates a new one (alloc) — both
    /// inside `permit_alloc`, since this runs on the audio thread before process_hop.
    /// On match for a non-Linear mode: ensure the inner Vecs are sized for `num_bins`
    /// (cheap when unchanged, sanctioned alloc when growing).
    /// On match for `Linear`: skip — no state to size, no syscall to make.
    pub fn sync_amp_modes(&mut self, rm: &RouteMatrix, num_bins: usize) {
        for ch in 0..2 {
            for r in 0..MAX_MATRIX_ROWS {
                for c in 0..MAX_SLOTS {
                    let want = rm.amp_mode[r][c];
                    if !self.amp_state[ch][r][c].matches(want) {
                        nih_plug::util::permit_alloc(|| {
                            self.amp_state[ch][r][c] = AmpNodeState::new(want, num_bins);
                        });
                    } else if !matches!(self.amp_state[ch][r][c], AmpNodeState::Linear) {
                        // Linear has no per-bin arrays — skip the resize entirely
                        // so the all-Linear common case is a pure compare loop.
                        nih_plug::util::permit_alloc(|| {
                            self.amp_state[ch][r][c].resize(num_bins);
                        });
                    }
                }
            }
        }
    }

    /// Clear all amp state arrays to startup values (e.g. on preset load or FFT-size change).
    pub fn clear_amp_state(&mut self) {
        for ch in 0..2 {
            for r in 0..MAX_MATRIX_ROWS {
                for c in 0..MAX_SLOTS {
                    self.amp_state[ch][r][c].clear();
                }
            }
        }
    }

    /// Zero all per-module DSP state and pre-allocated output/scratch buffers.
    /// Called from Pipeline::clear_state() on the audio thread when the user
    /// presses Reset — honouring the dialog promise "clear all module state".
    ///
    /// Module reset() impls may heap-allocate, so they must not be called here.
    /// Instead, each module's clear_state() zeroes only pre-allocated buffers.
    ///
    /// RT-safe: no allocation, no locking, no I/O.
    pub fn clear_state(&mut self) {
        for slot in self.slots.iter_mut().flatten() {
            slot.clear_state();
        }
        for buf in &mut self.slot_out    { buf.fill(Complex::new(0.0, 0.0)); }
        for buf in &mut self.slot_supp   { buf.fill(0.0); }
        for buf in &mut self.virtual_out { buf.fill(Complex::new(0.0, 0.0)); }
        self.mix_buf.fill(Complex::new(0.0, 0.0));
        // Vactrol caps, Schmitt latches, Slew histories — all cleared so user-facing
        // Reset honours its "clear all module state" promise. RT-safe (in-place fills).
        self.clear_amp_state();
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

    /// Propagate per-slot FutureMode from params to FutureModule instances.
    /// Called once per audio block (before process_hop).
    pub fn set_future_modes(&mut self, modes: &[FutureMode; 9]) {
        for s in 0..MAX_SLOTS {
            if let Some(ref mut m) = self.slots[s] {
                m.set_future_mode(modes[s]);
            }
        }
    }

    /// Propagate per-slot PunchMode from params to PunchModule instances.
    /// Called once per audio block (before process_hop).
    pub fn set_punch_modes(&mut self, modes: &[PunchMode; 9]) {
        for s in 0..MAX_SLOTS {
            if let Some(ref mut m) = self.slots[s] {
                m.set_punch_mode(modes[s]);
            }
        }
    }

    /// Propagate per-slot GeometryMode from params to GeometryModule instances.
    /// Called once per audio block (before process_hop).
    pub fn set_geometry_modes(&mut self, modes: &[GeometryMode; 9]) {
        for s in 0..MAX_SLOTS {
            if let Some(ref mut m) = self.slots[s] {
                m.set_geometry_mode(modes[s]);
            }
        }
    }

    /// Propagate per-slot ModulateMode from params to ModulateModule instances.
    /// Called once per audio block (before process_hop).
    pub fn set_modulate_modes(&mut self, modes: &[ModulateMode; 9]) {
        for s in 0..MAX_SLOTS {
            if let Some(ref mut m) = self.slots[s] {
                m.set_modulate_mode(modes[s]);
            }
        }
    }

    /// Propagate per-slot CircuitMode from params to CircuitModule instances.
    /// Called once per audio block (before process_hop).
    pub fn set_circuit_modes(&mut self, modes: &[CircuitMode; 9]) {
        for s in 0..MAX_SLOTS {
            if let Some(ref mut m) = self.slots[s] {
                m.set_circuit_mode(modes[s]);
            }
        }
    }

    /// Propagate per-slot RhythmMode + ArpGrid from params to RhythmModule instances.
    /// Called once per audio block (before process_hop).
    pub fn set_rhythm_modes_and_grids(
        &mut self,
        modes: &[RhythmMode; 9],
        grids: &[ArpGrid;    9],
    ) {
        for s in 0..MAX_SLOTS {
            if let Some(ref mut m) = self.slots[s] {
                m.set_rhythm_mode(modes[s]);
                m.set_arp_grid(grids[s]);
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
        debug_assert!(self.amp_scratch.len() >= num_bins);

        // hop_dt: wall-clock time elapsed per hop in seconds.
        // OVERLAP=4, so hop = fft_size / 4 samples.
        let hop_dt = ctx.fft_size as f32 / ctx.sample_rate / 4.0;

        // amp_ch: which amp_state channel to use. Linked always reads channel 0.
        let amp_ch = match stereo_link {
            crate::params::StereoLink::Linked => 0,
            _ => channel.min(1),
        };

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
                // Copy source into scratch, apply amp, then accumulate.
                self.amp_scratch[..num_bins].copy_from_slice(&self.slot_out[src][..num_bins]);
                let amp_params_cell = &route_matrix.amp_params[src][s];
                let amp_state_cell  = &mut self.amp_state[amp_ch][src][s];
                amp_state_cell.apply(amp_params_cell, &mut self.amp_scratch[..num_bins], hop_dt);
                for k in 0..num_bins {
                    self.mix_buf[k] += self.amp_scratch[k] * send;
                }
            }
            // Accumulate from virtual rows (T/S Split transient/sustained outputs).
            for (v, &vrow) in route_matrix.virtual_rows.iter().enumerate() {
                if let Some((src_slot, _kind)) = vrow {
                    if (src_slot as usize) < s {
                        let send = route_matrix.send[MAX_SLOTS + v][s];
                        if send < 0.001 { continue; }
                        // Copy virtual-row source into scratch, apply amp, then accumulate.
                        self.amp_scratch[..num_bins].copy_from_slice(&self.virtual_out[v][..num_bins]);
                        let amp_params_cell = &route_matrix.amp_params[MAX_SLOTS + v][s];
                        let amp_state_cell  = &mut self.amp_state[amp_ch][MAX_SLOTS + v][s];
                        amp_state_cell.apply(amp_params_cell, &mut self.amp_scratch[..num_bins], hop_dt);
                        for k in 0..num_bins {
                            self.mix_buf[k] += self.amp_scratch[k] * send;
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
            // Copy source into scratch, apply amp, then accumulate.
            self.amp_scratch[..num_bins].copy_from_slice(&self.slot_out[src][..num_bins]);
            let amp_params_cell = &route_matrix.amp_params[src][8];
            let amp_state_cell  = &mut self.amp_state[amp_ch][src][8];
            amp_state_cell.apply(amp_params_cell, &mut self.amp_scratch[..num_bins], hop_dt);
            for k in 0..num_bins {
                self.mix_buf[k] += self.amp_scratch[k] * send;
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
