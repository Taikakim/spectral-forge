use num_complex::Complex;
use crate::dsp::modules::{
    ModuleContext, ModuleType, RouteMatrix, GainMode, FutureMode, SpectralModule,
    create_module, MAX_SLOTS, MAX_SPLIT_VIRTUAL_ROWS, MAX_MATRIX_ROWS, VirtualRowKind,
};
use crate::dsp::modules::circuit::CircuitMode;
use crate::dsp::modules::life::LifeMode;
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

    /// Per-slot output BinPhysics — slot_phys[s] = the state after slot s's process().
    /// Sized MAX_SLOTS so slot_phys[8] = Master output. Reset to default at FFT-size change.
    pub slot_phys: Vec<crate::dsp::bin_physics::BinPhysics>,

    /// Workspace BinPhysics — assembled from upstream slot_phys[u] mixes for the current
    /// slot's input. Reused across slots within a hop (zeroed via reset_active after
    /// being copied into slot_phys[s] at end of each slot's iteration).
    mix_phys: crate::dsp::bin_physics::BinPhysics,

    /// Per-slot previous-frame |mix_buf[k]| magnitudes for auto-velocity. SoA:
    /// `prev_mags[slot * MAX_NUM_BINS + k]`. Zeroed at reset.
    prev_mags: Vec<f32>,

    /// Slot iteration order for the main 0..8 loop: writers (writes_bin_physics=true)
    /// first, then non-writers, both in numerical sub-order. Always size 8 (slots 0..7).
    /// Recomputed only at sync_slot_types — never per block. When bin_physics_in_use is
    /// false, equals (0..8).collect() (current numerical order, no semantic change).
    phys_order: Vec<usize>,

    /// True if any slot in 0..8 (or Master) opts in via spec.writes_bin_physics. When
    /// false, the BinPhysics assembly + velocity loops are skipped entirely.
    bin_physics_in_use: bool,

    /// Per-slot writer flag, mirrors module_spec(ty).writes_bin_physics.
    /// Indexed 0..MAX_SLOTS (includes Master at index 8). Recomputed by
    /// recompute_phys_topology whenever a slot's module changes.
    writer_bits: [bool; MAX_SLOTS],
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
        let mut this = Self {
            slots,
            slot_out:    (0..MAX_SLOTS).map(|_| vec![Complex::new(0.0, 0.0); MAX_NUM_BINS]).collect(),
            slot_supp:   (0..MAX_SLOTS).map(|_| vec![0.0f32; MAX_NUM_BINS]).collect(),
            virtual_out: (0..MAX_SPLIT_VIRTUAL_ROWS)
                             .map(|_| vec![Complex::new(0.0, 0.0); MAX_NUM_BINS]).collect(),
            mix_buf: vec![Complex::new(0.0, 0.0); MAX_NUM_BINS],
            amp_state: [mk_amp_grid(), mk_amp_grid()],
            amp_scratch: vec![Complex::new(0.0, 0.0); MAX_NUM_BINS],
            slot_phys:   (0..MAX_SLOTS).map(|_| crate::dsp::bin_physics::BinPhysics::new()).collect(),
            mix_phys:    crate::dsp::bin_physics::BinPhysics::new(),
            prev_mags:   vec![0.0; MAX_SLOTS * MAX_NUM_BINS],
            phys_order:  { let mut v = Vec::with_capacity(8); v.extend(0..8usize); v },
            bin_physics_in_use: false,
            writer_bits: [false; MAX_SLOTS],
        };
        this.recompute_phys_topology();
        this
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
        let num_bins = fft_size / 2 + 1;
        for p in &mut self.slot_phys {
            p.reset_active(num_bins, sample_rate, fft_size);
        }
        self.mix_phys.reset_active(num_bins, sample_rate, fft_size);
        self.prev_mags.fill(0.0);
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
        let mut any_changed = false;
        for s in 0..MAX_SLOTS {
            let current = self.slots[s].as_ref().map(|m| m.module_type())
                .unwrap_or(ModuleType::Empty);
            if current == types[s] { continue; }
            any_changed = true;
            if types[s] == ModuleType::Empty {
                nih_plug::util::permit_alloc(|| { self.slots[s] = None; });
            } else {
                nih_plug::util::permit_alloc(|| {
                    self.slots[s] = Some(create_module(types[s], sample_rate, fft_size));
                });
            }
        }
        if any_changed {
            nih_plug::util::permit_alloc(|| { self.recompute_phys_topology(); });
        }
    }

    /// Recompute phys_order (writers first, then non-writers) and bin_physics_in_use.
    /// Called from sync_slot_types whenever a slot's module changes.
    /// Alloc-free: uses stack-allocated [usize; 8] scratch arrays; phys_order.clear()
    /// retains capacity 8 (allocated in new()), so push never reallocates.
    fn recompute_phys_topology(&mut self) {
        use crate::dsp::modules::module_spec;
        let mut writers: [usize; 8] = [0; 8];
        let mut readers: [usize; 8] = [0; 8];
        let mut nw = 0usize;
        let mut nr = 0usize;
        let mut any_writer = false;

        for s in 0..8 {
            let ty = self.slots[s].as_ref().map(|m| m.module_type())
                .unwrap_or(ModuleType::Empty);
            let spec = module_spec(ty);
            self.writer_bits[s] = spec.writes_bin_physics;
            if spec.writes_bin_physics {
                writers[nw] = s; nw += 1;
                any_writer = true;
            } else {
                readers[nr] = s; nr += 1;
            }
        }

        // Master (slot 8) is always last, never in phys_order — but if Master ever opts in
        // it can still flip bin_physics_in_use:
        let master_ty = self.slots[8].as_ref().map(|m| m.module_type())
            .unwrap_or(ModuleType::Empty);
        let master_spec = module_spec(master_ty);
        self.writer_bits[8] = master_spec.writes_bin_physics;
        if master_spec.writes_bin_physics { any_writer = true; }

        self.phys_order.clear();
        for i in 0..nw { self.phys_order.push(writers[i]); }
        for i in 0..nr { self.phys_order.push(readers[i]); }
        self.bin_physics_in_use = any_writer;
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

    /// Propagate per-slot Modulate Repel toggle from params to ModulateModule instances.
    /// Called once per audio block (before process_hop).
    pub fn set_modulate_repels(&mut self, repels: &[bool; 9]) {
        for s in 0..MAX_SLOTS {
            if let Some(ref mut m) = self.slots[s] {
                m.set_modulate_repel(repels[s]);
            }
        }
    }

    /// Propagate per-slot SidechainPositioned toggle from params to ModulateModule instances.
    /// Called once per audio block (before process_hop).
    pub fn set_modulate_sc_positioneds(&mut self, flags: &[bool; 9]) {
        for s in 0..MAX_SLOTS {
            if let Some(ref mut m) = self.slots[s] {
                m.set_modulate_sc_positioned(flags[s]);
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

    /// Propagate per-slot LifeMode from params to LifeModule instances.
    /// Called once per audio block (before process_hop).
    pub fn set_life_modes(&mut self, modes: &[LifeMode; 9]) {
        for s in 0..MAX_SLOTS {
            if let Some(ref mut m) = self.slots[s] {
                m.set_life_mode(modes[s]);
            }
        }
    }

    /// Propagate per-slot PastMode from params to PastModule instances.
    /// Called once per audio block (before process_hop).
    pub fn set_past_modes(&mut self, modes: &[crate::dsp::modules::past::PastMode; 9]) {
        for s in 0..MAX_SLOTS {
            if let Some(ref mut m) = self.slots[s] {
                m.set_past_mode(modes[s]);
            }
        }
    }

    /// Propagate per-slot SortKey from params to PastModule instances.
    /// Called once per audio block (before process_hop).
    pub fn set_past_sort_keys(&mut self, keys: &[crate::dsp::modules::past::SortKey; 9]) {
        for s in 0..MAX_SLOTS {
            if let Some(ref mut m) = self.slots[s] {
                m.set_past_sort_key(keys[s]);
            }
        }
    }

    /// Propagate per-slot KineticsMode from params to KineticsModule instances.
    /// Called once per audio block (before process_hop).
    pub fn set_kinetics_modes(&mut self, modes: &[crate::dsp::modules::kinetics::KineticsMode; 9]) {
        for s in 0..MAX_SLOTS {
            if let Some(ref mut m) = self.slots[s] {
                m.set_kinetics_mode(modes[s]);
            }
        }
    }

    /// Propagate per-slot WellSource from params to KineticsModule instances.
    /// Called once per audio block (before process_hop).
    pub fn set_kinetics_well_sources(&mut self, srcs: &[crate::dsp::modules::kinetics::WellSource; 9]) {
        for s in 0..MAX_SLOTS {
            if let Some(ref mut m) = self.slots[s] {
                m.set_kinetics_well_source(srcs[s]);
            }
        }
    }

    /// Propagate per-slot MassSource from params to KineticsModule instances.
    /// Called once per audio block (before process_hop).
    pub fn set_kinetics_mass_sources(&mut self, srcs: &[crate::dsp::modules::kinetics::MassSource; 9]) {
        for s in 0..MAX_SLOTS {
            if let Some(ref mut m) = self.slots[s] {
                m.set_kinetics_mass_source(srcs[s]);
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

    /// Phase 4.3a — propagate the global Dynamics-PLPV enable flag to every
    /// slot. The trait's default `set_plpv_dynamics_enabled` is a no-op for
    /// non-Dynamics modules, so calling it on every slot is safe and cheap
    /// (one cmp + branch per slot). Called once per audio block (before
    /// process_hop).
    pub fn set_plpv_dynamics_enable(&mut self, enabled: bool) {
        for s in 0..MAX_SLOTS {
            if let Some(ref mut m) = self.slots[s] {
                m.set_plpv_dynamics_enabled(enabled);
            }
        }
    }

    /// Phase 4.3b — propagate the global PhaseSmear-PLPV enable flag to every
    /// slot. The trait's default `set_plpv_phase_smear_enabled` is a no-op for
    /// non-PhaseSmear modules. Called once per audio block (before process_hop).
    pub fn set_plpv_phase_smear_enable(&mut self, enabled: bool) {
        for s in 0..MAX_SLOTS {
            if let Some(ref mut m) = self.slots[s] {
                m.set_plpv_phase_smear_enabled(enabled);
            }
        }
    }

    /// Phase 4.3c — propagate the global Freeze-PLPV enable flag to every slot.
    /// The trait's default `set_plpv_freeze_enabled` is a no-op for non-Freeze
    /// modules. Called once per audio block (before process_hop).
    pub fn set_plpv_freeze_enable(&mut self, enabled: bool) {
        for s in 0..MAX_SLOTS {
            if let Some(ref mut m) = self.slots[s] {
                m.set_plpv_freeze_enabled(enabled);
            }
        }
    }

    /// Phase 4.3d — propagate the global MidSide-PLPV enable flag to every slot.
    /// The trait's default `set_plpv_midside_enabled` is a no-op for non-MidSide
    /// modules. Called once per audio block (before process_hop).
    pub fn set_plpv_midside_enable(&mut self, enabled: bool) {
        for s in 0..MAX_SLOTS {
            if let Some(ref mut m) = self.slots[s] {
                m.set_plpv_midside_enabled(enabled);
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
        // Guard against the stale-audio defect that arises when bin_physics_in_use=true
        // and phys_order reorders slots: audio assembly (for src in 0..s) reads slot_out
        // in numerical order, which would contain stale (previous-hop) data for any writer
        // slot moved earlier in phys_order. Phase 5 must resolve topology before opting in
        // via writes_bin_physics=true. Today this is inert (no module opts in).
        debug_assert!(
            !self.bin_physics_in_use
                || self.phys_order.iter().enumerate().all(|(i, &s)| s == i),
            "BinPhysics writer reordering is active but audio assembly still uses \
             numerical order — would read stale slot_out. Phase 5 must resolve."
        );

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

        for i in 0..self.phys_order.len() {  // writers first, then non-writers; Master (slot 8) is handled separately below
            let s = self.phys_order[i];
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

            // BinPhysics assembly: mix upstream slot_phys[u] into mix_phys per route weight,
            // then compute auto-velocity from the magnitude delta of mix_buf vs prev frame.
            if self.bin_physics_in_use {
                // mix_phys was reset_active at end of previous slot (or in reset() for first hop).
                // Mix upstream physics outputs into mix_phys per the same route weights as audio.
                // s == 0: no upstream physics; mix_phys stays at zero/default.
                for u in 0..s {
                    let send = route_matrix.send[u][s];
                    if send < 0.001 { continue; }
                    // slot_phys[u] and mix_phys are disjoint struct fields — safe split borrow.
                    self.mix_phys.mix_from(&self.slot_phys[u], send, num_bins);
                }

                // Auto-velocity: |curr_mag[k] - prev_mag[k]| written into mix_phys.velocity.
                let prev_off = s * MAX_NUM_BINS;
                for k in 0..num_bins {
                    let curr_mag = self.mix_buf[k].norm();
                    self.mix_phys.velocity[k] = (curr_mag - self.prev_mags[prev_off + k]).abs();
                    self.prev_mags[prev_off + k] = curr_mag;
                }
            }

            let mut module = match self.slots[s].take() {
                Some(m) => m,
                None => {
                    // No module: pass-through audio, zero suppression.
                    // Still need to snapshot/reset mix_phys for physics continuity.
                    if self.bin_physics_in_use {
                        // mix_phys and slot_phys[s] are disjoint struct fields — safe split borrow.
                        // Copy via a temporary to avoid double-borrow: read out then write.
                        let (mix_phys, slot_phys) = (&self.mix_phys, &mut self.slot_phys[s]);
                        slot_phys.copy_from(mix_phys, num_bins);
                        self.mix_phys.reset_active(num_bins, ctx.sample_rate, ctx.fft_size);
                    }
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
                // Writer/reader split for BinPhysics dispatch:
                //   Writer slots (writes_bin_physics=true): physics = Some(&mut mix_phys),
                //     ctx.bin_physics = None  — they MUTATE physics state.
                //   Reader slots: physics = None, ctx.bin_physics = Some(&mix_phys)
                //     — they OBSERVE physics via ctx.
                // The two branches are mutually exclusive at runtime, so only one kind of
                // borrow on mix_phys is live at any call site — borrow-checker safe.
                let is_writer = self.writer_bits[s];
                let mut ctx_for_slot = *ctx;
                let physics_arg: Option<&mut crate::dsp::bin_physics::BinPhysics>;
                if self.bin_physics_in_use && is_writer {
                    ctx_for_slot.bin_physics = None;
                    physics_arg = Some(&mut self.mix_phys);
                } else {
                    if self.bin_physics_in_use {
                        ctx_for_slot.bin_physics = Some(&self.mix_phys);
                    } else {
                        ctx_for_slot.bin_physics = None;
                    }
                    physics_arg = None;
                }
                module.process(
                    channel, stereo_link, slot_targets[s],
                    &mut self.mix_buf[..num_bins],
                    sc_args[s], curves,
                    &mut self.slot_supp[s][..num_bins],
                    physics_arg,
                    &ctx_for_slot,
                );
                // Snapshot mix_phys → slot_phys[s] and reset the workspace for the next slot.
                if self.bin_physics_in_use {
                    // mix_phys and slot_phys[s] are disjoint struct fields — safe split borrow.
                    let (mix_phys, slot_phys) = (&self.mix_phys, &mut self.slot_phys[s]);
                    slot_phys.copy_from(mix_phys, num_bins);
                    self.mix_phys.reset_active(num_bins, ctx.sample_rate, ctx.fft_size);
                }
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

        // BinPhysics for Master input: weighted mix of slot_phys[0..8] per send to Master.
        if self.bin_physics_in_use {
            for u in 0..8 {
                let send = route_matrix.send[u][8];
                if send < 0.001 { continue; }
                // slot_phys[u] and mix_phys are disjoint struct fields — safe split borrow.
                self.mix_phys.mix_from(&self.slot_phys[u], send, num_bins);
            }
            // Auto-velocity for Master input.
            let prev_off = 8 * MAX_NUM_BINS;
            for k in 0..num_bins {
                let curr_mag = self.mix_buf[k].norm();
                self.mix_phys.velocity[k] = (curr_mag - self.prev_mags[prev_off + k]).abs();
                self.prev_mags[prev_off + k] = curr_mag;
            }
        }

        // Pass through Master module (slot 8) then write to complex_buf.
        if let Some(ref mut master_mod) = self.slots[8] {
            let curves_empty: &[&[f32]] = &[];
            // Writer/reader split for Master (slot 8) — same semantics as main slot loop.
            let is_writer = self.writer_bits[8];
            let mut ctx_for_master = *ctx;
            let physics_arg: Option<&mut crate::dsp::bin_physics::BinPhysics>;
            if self.bin_physics_in_use && is_writer {
                ctx_for_master.bin_physics = None;
                physics_arg = Some(&mut self.mix_phys);
            } else {
                if self.bin_physics_in_use {
                    ctx_for_master.bin_physics = Some(&self.mix_phys);
                } else {
                    ctx_for_master.bin_physics = None;
                }
                physics_arg = None;
            }
            master_mod.process(
                channel, stereo_link, slot_targets[8],
                &mut self.mix_buf[..num_bins],
                sc_args[8], curves_empty,
                &mut self.slot_supp[8][..num_bins],
                physics_arg,
                &ctx_for_master,
            );
        }
        // Snapshot mix_phys → slot_phys[8] and reset workspace.
        if self.bin_physics_in_use {
            // mix_phys and slot_phys[8] are disjoint struct fields — safe split borrow.
            let (mix_phys, slot_phys) = (&self.mix_phys, &mut self.slot_phys[8]);
            slot_phys.copy_from(mix_phys, num_bins);
            self.mix_phys.reset_active(num_bins, ctx.sample_rate, ctx.fft_size);
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

#[cfg(feature = "probe")]
impl FxMatrix {
    /// Test-only: force a slot to be treated as a BinPhysics writer.
    ///
    /// `recompute_phys_topology` derives `writer_bits` from
    /// `module_spec(module_type).writes_bin_physics`, which is `false` for all
    /// real module types today. Mock test modules can't easily declare
    /// themselves writers via the spec, so this helper directly sets the
    /// writer bit, flips `bin_physics_in_use`, and promotes the slot to the
    /// front of `phys_order` (the writer-first iteration order).
    ///
    /// Slot 8 (Master) is also valid; it is not in `phys_order` (Master
    /// always runs last) but its writer_bit still gates the dispatch split.
    ///
    /// **Not for production use.** Phase 5 will remove the need for this once
    /// real writer modules ship and the spec-based path is exercised directly.
    ///
    /// # PANICS
    ///
    /// **Only `slot == 0` or `slot == 8` are safe today.**
    ///
    /// Calling this with `slot ∈ 1..=7` will:
    /// 1. Promote that slot to the front of `phys_order` (writer-first ordering).
    /// 2. Immediately trip the `debug_assert!` in `process_hop` (lines 327–332)
    ///    that guards the stale-audio defect: audio assembly (`for src in 0..s`)
    ///    reads `slot_out` in *numerical* order, so any writer slot moved earlier
    ///    in `phys_order` would read previous-hop (stale) data.
    ///
    /// Phase 5 must teach the audio-assembly loop to follow `phys_order` (not
    /// numerical order) before this restriction can be lifted. Until then,
    /// **do not call `test_force_writer` with slot 1..=7 in any test**.
    pub fn test_force_writer(&mut self, slot: usize) {
        debug_assert!(slot < MAX_SLOTS);
        debug_assert!(
            slot == 0 || slot == 8,
            "test_force_writer: only slot 0 or 8 are safe today; \
             slots 1..=7 trip the stale-audio guard in process_hop. \
             Phase 5 will lift this."
        );
        self.writer_bits[slot] = true;
        self.bin_physics_in_use = true;
        if slot < 8 {
            if let Some(pos) = self.phys_order.iter().position(|&s| s == slot) {
                if pos != 0 {
                    let s_val = self.phys_order.remove(pos);
                    self.phys_order.insert(0, s_val);
                }
            }
        }
    }
}
