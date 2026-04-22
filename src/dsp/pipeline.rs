use num_complex::Complex;
use realfft::RealFftPlanner;
use nih_plug::util::StftHelper;
use crate::bridge::SharedState;
use crate::params::{FxChannelTarget, ScChannel, StereoLink};

/// Which of the five derived SC magnitude streams a slot should key off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ScSource { L = 0, R = 1, LR = 2, M = 3, S = 4 }

/// Map (user choice, stereo mode, slot target, processing channel) → SC source.
/// See spec §5 for the canonical rule: Follow = "route the SC channel matching whatever this slot processes."
pub fn resolve_sc_source(
    choice: ScChannel,
    link: StereoLink,
    target: FxChannelTarget,
    channel: usize,
) -> ScSource {
    match choice {
        ScChannel::Follow => match link {
            StereoLink::Linked => ScSource::LR,
            StereoLink::Independent => if channel == 0 { ScSource::L } else { ScSource::R },
            StereoLink::MidSide => match target {
                FxChannelTarget::Mid  => ScSource::M,
                FxChannelTarget::Side => ScSource::S,
                FxChannelTarget::All  => ScSource::LR,
            },
        },
        ScChannel::LR => ScSource::LR,
        ScChannel::L  => ScSource::L,
        ScChannel::R  => ScSource::R,
        ScChannel::M  => ScSource::M,
        ScChannel::S  => ScSource::S,
    }
}

pub const FFT_SIZE: usize = 2048;
pub const NUM_BINS: usize = FFT_SIZE / 2 + 1;
pub const OVERLAP: usize = 4; // 75% overlap → hop = 512
pub const MAX_FFT_SIZE: usize = 16384;
pub const MAX_NUM_BINS: usize = MAX_FFT_SIZE / 2 + 1;

/// Maximum block size assumed for the delta monitor dry-delay ring buffer.
/// nih-plug typically processes in blocks of ≤ 8192 samples.
const MAX_BLOCK_SIZE: usize = 8192;

/// Maximum ring-buffer size per channel: accommodates the largest possible FFT latency + block.
const MAX_DRY_DELAY_SIZE: usize = MAX_FFT_SIZE + MAX_BLOCK_SIZE;

pub struct Pipeline {
    stft: StftHelper,
    fft_plan:  std::sync::Arc<dyn realfft::RealToComplex<f32>>,
    ifft_plan: std::sync::Arc<dyn realfft::ComplexToReal<f32>>,
    window:         Vec<f32>,
    spectrum_buf:    Vec<f32>,
    suppression_buf: Vec<f32>,
    channel_supp_buf: Vec<f32>,
    complex_buf:     Vec<Complex<f32>>,
    fx_matrix: crate::dsp::fx_matrix::FxMatrix,
    /// Ring buffer for delta monitor dry-signal delay: 2 channels × MAX_DRY_DELAY_SIZE entries.
    /// Channel c occupies [c * MAX_DRY_DELAY_SIZE .. (c+1) * MAX_DRY_DELAY_SIZE].
    /// Delayed by fft_size samples to align dry with STFT-latency-compensated wet.
    dry_delay: Vec<f32>,
    /// Current write head into dry_delay (wraps at fft_size + MAX_BLOCK_SIZE).
    dry_delay_write: usize,
    fft_size: usize,
    /// Pre-allocated per-slot curve cache. [slot][curve][bin]
    slot_curve_cache: Vec<Vec<Vec<f32>>>,
    /// Per-bin SC envelope magnitudes, one slice per derived source (L, R, LR, M, S).
    /// Shape: [5 sources][MAX_NUM_BINS]. Index matches ScSource ordinal (L=0, R=1, LR=2, M=3, S=4).
    sc_envelopes:   Vec<Vec<f32>>,
    /// Per-bin one-pole envelope state, per source. Shape matches sc_envelopes.
    sc_env_states:  Vec<Vec<f32>>,
    /// FFT output buffers for the 2-channel SC STFT. Shape: [2 channels][num_bins].
    sc_complex_bufs: Vec<Vec<Complex<f32>>>,
    /// Single 2-channel SC STFT.
    sc_stft: StftHelper,
    /// Per-channel, per-slot SC magnitude slice; slot_sc_input[channel][slot][bin]. Pre-allocated.
    slot_sc_input: Vec<Vec<Vec<f32>>>,
    sample_rate: f32,
}

impl Pipeline {
    pub fn new(sample_rate: f32, num_channels: usize, fft_size: usize, slot_types: &[crate::dsp::modules::ModuleType; 9]) -> Self {
        let num_bins = fft_size / 2 + 1;
        let mut planner = RealFftPlanner::<f32>::new();
        let fft_plan  = planner.plan_fft_forward(fft_size);
        let ifft_plan = planner.plan_fft_inverse(fft_size);

        let window: Vec<f32> = (0..fft_size)
            .map(|i| 0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32
                / (fft_size - 1) as f32).cos()))
            .collect();

        let complex_buf = fft_plan.make_output_vec();

        let fx_matrix = crate::dsp::fx_matrix::FxMatrix::new(sample_rate, fft_size, slot_types);

        // 9 slots × 7 curves × MAX_NUM_BINS, all-ones (neutral); only [0..num_bins] are used
        let slot_curve_cache: Vec<Vec<Vec<f32>>> = (0..9)
            .map(|_| (0..7).map(|_| vec![1.0f32; MAX_NUM_BINS]).collect())
            .collect();

        // 5 SC sources (L, R, LR, M, S) pre-allocated at MAX_NUM_BINS; only [0..num_bins] are used.
        let sc_envelopes:  Vec<Vec<f32>> = (0..5).map(|_| vec![0.0f32; MAX_NUM_BINS]).collect();
        let sc_env_states: Vec<Vec<f32>> = (0..5).map(|_| vec![0.0f32; MAX_NUM_BINS]).collect();
        // Single 2-channel SC STFT; two complex buffers for L and R.
        let sc_complex_bufs: Vec<Vec<Complex<f32>>> = (0..2)
            .map(|_| vec![Complex::new(0.0f32, 0.0f32); num_bins])
            .collect();
        let sc_stft = StftHelper::new(2, fft_size, 0);
        // Per-channel, per-slot SC magnitude slices (gained copies of the source envelope).
        let slot_sc_input: Vec<Vec<Vec<f32>>> = (0..2)
            .map(|_| (0..9).map(|_| vec![0.0f32; MAX_NUM_BINS]).collect())
            .collect();

        Self {
            stft: StftHelper::new(num_channels, fft_size, 0),
            fft_plan,
            ifft_plan,
            window,
            spectrum_buf:     vec![0.0; MAX_NUM_BINS],
            suppression_buf:  vec![0.0; MAX_NUM_BINS],
            channel_supp_buf: vec![0.0; MAX_NUM_BINS],
            complex_buf,
            fx_matrix,
            dry_delay: vec![0.0f32; 2 * MAX_DRY_DELAY_SIZE],
            dry_delay_write: 0,
            slot_curve_cache,
            sc_envelopes,
            sc_env_states,
            sc_complex_bufs,
            sc_stft,
            slot_sc_input,
            sample_rate,
            fft_size,
        }
    }

    pub fn reset(&mut self, sample_rate: f32, num_channels: usize) {
        let fft_size = self.fft_size;
        let num_bins = fft_size / 2 + 1;
        self.sample_rate = sample_rate;

        let mut planner = RealFftPlanner::<f32>::new();
        self.fft_plan  = planner.plan_fft_forward(fft_size);
        self.ifft_plan = planner.plan_fft_inverse(fft_size);

        self.window = (0..fft_size)
            .map(|i| 0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32
                / (fft_size - 1) as f32).cos()))
            .collect();
        self.complex_buf = self.fft_plan.make_output_vec();
        for buf in &mut self.sc_complex_bufs {
            buf.resize(num_bins, Complex::new(0.0, 0.0));
            buf.fill(Complex::new(0.0, 0.0));
        }

        self.stft = StftHelper::new(num_channels, fft_size, 0);
        self.sc_stft = StftHelper::new(2, fft_size, 0);
        self.dry_delay.fill(0.0);
        self.dry_delay_write = 0;
        for sc in &mut self.sc_envelopes  { sc.fill(0.0); }
        for sc in &mut self.sc_env_states { sc.fill(0.0); }
        for ch in &mut self.slot_sc_input {
            for slot_buf in ch {
                slot_buf.fill(0.0);
            }
        }
        self.fx_matrix.reset(sample_rate, fft_size);
    }

    pub fn process(
        &mut self,
        buffer: &mut nih_plug::buffer::Buffer,
        aux: &mut nih_plug::prelude::AuxiliaryBuffers,
        shared: &mut SharedState,
        params: &crate::params::SpectralForgeParams,
    ) {
        use crate::dsp::modules::{apply_curve_transform, ModuleContext};

        let fft_size = self.fft_size;
        let num_bins = fft_size / 2 + 1;
        let block_size = buffer.samples() as u32;
        let attack_ms_base    = params.attack_ms.smoothed.next_step(block_size);
        let release_ms_base   = params.release_ms.smoothed.next_step(block_size);
        let input_gain_db     = params.input_gain.smoothed.next_step(block_size);
        let output_gain_db    = params.output_gain.smoothed.next_step(block_size);
        let global_mix        = params.mix.smoothed.next_step(block_size).clamp(0.0, 1.0);

        // ── Read all 9×7 slot curves from triple-buffer + apply tilt/offset ──
        // Non-blocking: if GUI holds the lock this block, skip tilt/offset (not catastrophic).
        let meta_guard = params.slot_curve_meta.try_lock();
        for s in 0..9 {
            for c in 0..7 {
                self.slot_curve_cache[s][c].copy_from_slice(&shared.curve_rx[s][c].read()[..MAX_NUM_BINS]);
                if let Some(ref meta) = meta_guard {
                    let (tilt, offset) = meta[s][c];
                    apply_curve_transform(&mut self.slot_curve_cache[s][c], tilt, offset, self.sample_rate, self.fft_size);
                }
            }
        }

        // ── Process single stereo sidechain input ──
        let mut sc_active = false;
        {
            let hop = fft_size / OVERLAP;
            let fft_plan = self.fft_plan.clone();
            let window = &self.window;
            let sample_rate = self.sample_rate;
            let sc_attack_ms  = params.sc_attack_ms.smoothed.next_step(block_size);
            let sc_release_ms = params.sc_release_ms.smoothed.next_step(block_size);

            let has_aux = aux.inputs.get(0).map(|a| a.samples() > 0).unwrap_or(false);
            if !has_aux {
                for src in &mut self.sc_envelopes  { for v in src.iter_mut() { *v = 0.0; } }
                for src in &mut self.sc_env_states { for v in src.iter_mut() { *v = 0.0; } }
            } else {
                // Zero the output envelopes (peak-capture accumulators) — state is preserved.
                for src in &mut self.sc_envelopes { for v in src.iter_mut() { *v = 0.0; } }

                let sc_complex_bufs = &mut self.sc_complex_bufs;
                let sc_env_states = &mut self.sc_env_states;
                let sc_envelopes  = &mut self.sc_envelopes;

                self.sc_stft.process_overlap_add(&mut aux.inputs[0], OVERLAP, |channel, block| {
                    for (s, &w) in block.iter_mut().zip(window.iter()) {
                        *s *= w;
                    }
                    crate::dsp::guard::sanitize(block);
                    fft_plan.process(block, &mut sc_complex_bufs[channel]).unwrap();

                    // Only run envelope update on the second channel, once we have both L and R in sc_complex_bufs.
                    if channel == 1 {
                        let hops_per_sec = sample_rate / hop as f32;
                        let atk_t = sc_attack_ms.max(0.1) * 0.001 * hops_per_sec;
                        let rel_t = sc_release_ms.max(1.0) * 0.001 * hops_per_sec;
                        let atk_coeff = (-1.0_f32 / atk_t).exp();
                        let rel_coeff = (-1.0_f32 / rel_t).exp();

                        let num = sc_complex_bufs[0].len();
                        const SQRT2_INV: f32 = std::f32::consts::FRAC_1_SQRT_2;
                        for k in 0..num {
                            let l_mag = sc_complex_bufs[0][k].norm();
                            let r_mag = sc_complex_bufs[1][k].norm();
                            // L+R sum magnitude (conservative: average, not complex sum magnitude).
                            let lr_mag = 0.5 * (l_mag + r_mag);
                            // M/S from complex sums/diffs then magnitude.
                            let m_cpx = (sc_complex_bufs[0][k] + sc_complex_bufs[1][k]) * SQRT2_INV;
                            let s_cpx = (sc_complex_bufs[0][k] - sc_complex_bufs[1][k]) * SQRT2_INV;
                            let m_mag = m_cpx.norm();
                            let s_mag = s_cpx.norm();

                            let mags = [l_mag, r_mag, lr_mag, m_mag, s_mag];
                            for (src_idx, &mag) in mags.iter().enumerate() {
                                let state_ref = &mut sc_env_states[src_idx][k];
                                let coeff = if mag > *state_ref { atk_coeff } else { rel_coeff };
                                *state_ref = coeff * *state_ref + (1.0 - coeff) * mag;
                                if *state_ref > sc_envelopes[src_idx][k] {
                                    sc_envelopes[src_idx][k] = *state_ref;
                                }
                            }
                        }
                    }
                });

                sc_active = self.sc_envelopes[ScSource::LR as usize].iter().any(|&v| v > 1e-9);
            }
        }

        shared.sidechain_active.store(sc_active, std::sync::atomic::Ordering::Relaxed);

        // ── Read feature flags and stereo link ──
        let delta_monitor = params.delta_monitor.value();
        let stereo_link = params.stereo_link.value();
        let is_mid_side = stereo_link == StereoLink::MidSide;

        let input_linear  = 10.0f32.powf(input_gain_db  / 20.0);
        let output_linear = 10.0f32.powf(output_gain_db / 20.0);

        // ThresholdMode::Relative legacy flag maps to sensitivity=1.0 if set;
        // otherwise use the continuous sensitivity parameter.
        let sensitivity = if params.threshold_mode.value() == crate::params::ThresholdMode::Relative {
            1.0f32
        } else {
            params.sensitivity.smoothed.next_step(block_size)
        };

        // Build ModuleContext (all Copy fields, no borrows)
        let ctx = ModuleContext {
            sample_rate:       self.sample_rate,
            fft_size,
            num_bins,
            attack_ms:         attack_ms_base,
            release_ms:        release_ms_base,
            sensitivity,
            suppression_width: params.suppression_width.smoothed.next_step(block_size),
            auto_makeup:       params.auto_makeup.value(),
            delta_monitor,
        };

        // Snapshot of slot targets (needed for SC channel resolution in MidSide mode).
        let slot_targets_snap: [FxChannelTarget; 9] = params.slot_targets.try_lock()
            .map(|g| *g)
            .unwrap_or([FxChannelTarget::All; 9]);

        // ── Build per-slot SC input slices (allocation-free) ──
        let slot_sc_gain_db_arr: [f32; 9] = params.slot_sc_gain_db.try_lock()
            .map(|g| *g)
            .unwrap_or([0.0f32; 9]);
        let slot_sc_channel_arr: [ScChannel; 9] = params.slot_sc_channel.try_lock()
            .map(|g| *g)
            .unwrap_or([ScChannel::Follow; 9]);

        let slot_types_snap: [crate::dsp::modules::ModuleType; 9] = params.slot_module_types.try_lock()
            .map(|g| *g)
            .unwrap_or([crate::dsp::modules::ModuleType::Empty; 9]);

        // Precompute ScSource per (channel, slot).
        let mut slot_sc_source_ch: [[ScSource; 9]; 2] = [[ScSource::LR; 9]; 2];
        for ch in 0..2usize {
            for s in 0..9usize {
                let ty = slot_types_snap[s];
                let supports = crate::dsp::modules::module_spec(ty).supports_sidechain;
                if !supports { continue; }
                slot_sc_source_ch[ch][s] = resolve_sc_source(
                    slot_sc_channel_arr[s],
                    stereo_link,
                    slot_targets_snap[s],
                    ch,
                );
            }
        }

        // Fill slot_sc_input[ch][s] with gained SC magnitudes, or zero if the slot is inactive.
        for ch in 0..2usize {
            for s in 0..9usize {
                let ty = slot_types_snap[s];
                let supports = crate::dsp::modules::module_spec(ty).supports_sidechain;
                let gain_db = slot_sc_gain_db_arr[s];
                let gain_lin = if gain_db <= -90.0 { 0.0 } else { 10.0f32.powf(gain_db / 20.0) };
                let active_for_slot = supports && gain_lin > 0.0 && sc_active;
                if !active_for_slot {
                    for v in self.slot_sc_input[ch][s].iter_mut().take(num_bins) { *v = 0.0; }
                    continue;
                }
                let src_idx = slot_sc_source_ch[ch][s] as usize;
                // Split borrow: read from sc_envelopes, write to slot_sc_input. They're different fields.
                let src = &self.sc_envelopes[src_idx];
                let dst = &mut self.slot_sc_input[ch][s];
                for k in 0..num_bins { dst[k] = src[k] * gain_lin; }
            }
        }

        let dry_delay_size = fft_size + MAX_BLOCK_SIZE;
        // We need the delayed dry signal for either the delta monitor or the global wet/dry
        // mix. A small epsilon avoids running the dry path for a mix knob that's effectively
        // at unity.
        let need_dry = delta_monitor || global_mix < 0.999_5;
        // Capture dry samples into the ring buffer at the current write head.
        if need_dry {
            let mut dry_idx = 0usize;
            for sample_block in buffer.iter_samples() {
                debug_assert!(dry_idx < MAX_BLOCK_SIZE, "block size exceeded MAX_BLOCK_SIZE={MAX_BLOCK_SIZE}");
                let pos = (self.dry_delay_write + dry_idx) % dry_delay_size;
                for (ch_idx, sample) in sample_block.into_iter().enumerate() {
                    self.dry_delay[ch_idx * MAX_DRY_DELAY_SIZE + pos] = *sample;
                }
                dry_idx += 1;
            }
        }

        // M/S encode: L/R → Mid/Side (before STFT)
        if is_mid_side {
            const SQRT2_INV: f32 = std::f32::consts::FRAC_1_SQRT_2;
            for mut sample_block in buffer.iter_samples() {
                let mut ch = sample_block.iter_mut();
                if let (Some(l), Some(r)) = (ch.next(), ch.next()) {
                    let m = (*l + *r) * SQRT2_INV;
                    let s = (*l - *r) * SQRT2_INV;
                    *l = m;
                    *r = s;
                }
            }
        }

        // Sync module types from params (non-blocking; skipped if GUI holds lock).
        // Handles add/remove of modules at runtime after initialize().
        if let Some(types) = params.slot_module_types.try_lock() {
            self.fx_matrix.sync_slot_types(&*types, self.sample_rate, self.fft_size);
        }

        // Propagate gain modes each block (try_lock is non-blocking; skipped if GUI holds lock).
        if let Some(modes) = params.slot_gain_mode.try_lock() {
            self.fx_matrix.set_gain_modes(&*modes);
        }

        // Build route matrix from automatable params each block.
        // virtual_rows (T/S Split bindings) are not exposed as automation targets,
        // so we still read them from the Mutex — but never block waiting for it.
        let virt = params.route_matrix.try_lock()
            .map(|g| g.virtual_rows)
            .unwrap_or_default();
        let mut route_matrix_snap = crate::dsp::modules::RouteMatrix {
            send: [[0.0f32; crate::dsp::modules::MAX_SLOTS]; crate::dsp::modules::MAX_MATRIX_ROWS],
            virtual_rows: virt,
        };
        for r in 0..crate::param_ids::NUM_MATRIX_ROWS {
            for col in 0..crate::param_ids::NUM_SLOTS {
                if r == col { continue; } // skip diagonal to prevent self-feedback
                if let Some(p) = params.matrix_cell(r, col) {
                    route_matrix_snap.send[col][r] = p.smoothed.next();
                }
            }
        }

        // Reborrow fields as locals so the closure can capture them without
        // conflicting with the &mut self.stft borrow inside process_overlap_add.
        let fft_plan  = self.fft_plan.clone();
        let ifft_plan = self.ifft_plan.clone();
        let window         = &self.window;
        let fx_matrix         = &mut self.fx_matrix;
        let complex_buf       = &mut self.complex_buf;
        let spectrum_buf      = &mut self.spectrum_buf;
        let suppression_buf   = &mut self.suppression_buf;
        let channel_supp_buf  = &mut self.channel_supp_buf;
        let slot_curve_cache_ref = &self.slot_curve_cache;
        let slot_sc_input_ref = &self.slot_sc_input;
        // Reset peak-hold accumulators.
        for v in spectrum_buf.iter_mut()   { *v = 0.0; }
        for v in suppression_buf.iter_mut() { *v = 0.0; }
        // IFFT gives fft_size gain; Hann^2 OLA at 75% overlap gives 1.5 gain.
        // Combined normalization: 1 / (fft_size * 1.5) = 2 / (3 * fft_size)
        let norm = 2.0_f32 / (3.0 * fft_size as f32);

        self.stft.process_overlap_add(buffer, OVERLAP, |channel, block| {
            // Analysis window + input gain
            for (s, &w) in block.iter_mut().zip(window.iter()) {
                *s *= w * input_linear;
            }

            // Guard: clamp NaN/Inf from broken drivers before FFT.
            crate::dsp::guard::sanitize(block);

            fft_plan.process(block, complex_buf).unwrap();

            for (i, c) in complex_buf.iter().enumerate() {
                let mag = c.norm();
                if mag > spectrum_buf[i] { spectrum_buf[i] = mag; }
            }

            // Build this hop's per-slot sc_args from the pre-gained slot_sc_input.
            // Clamp channel index to 0..=1 for potential mono cases (slot_sc_input only has 2 channels).
            let sc_ch = channel.min(1);
            let sc_args: [Option<&[f32]>; 9] = std::array::from_fn(|s| {
                let ty = slot_types_snap[s];
                let supports = crate::dsp::modules::module_spec(ty).supports_sidechain;
                let gain_db = slot_sc_gain_db_arr[s];
                if !supports || gain_db <= -90.0 || !sc_active {
                    None
                } else {
                    Some(&slot_sc_input_ref[sc_ch][s][..num_bins])
                }
            });

            // Run all modules through the fx_matrix slot chain.
            fx_matrix.process_hop(
                channel,
                stereo_link,
                complex_buf,
                &sc_args,
                &slot_targets_snap,
                slot_curve_cache_ref,
                &route_matrix_snap,
                &ctx,
                channel_supp_buf,
                num_bins,
            );
            for k in 0..channel_supp_buf.len() {
                if channel_supp_buf[k] > suppression_buf[k] { suppression_buf[k] = channel_supp_buf[k]; }
            }

            ifft_plan.process(complex_buf, block).unwrap();

            // Synthesis window + IFFT normalization + output gain
            for (s, &w) in block.iter_mut().zip(window.iter()) {
                *s *= w * norm * output_linear;
            }
        });

        // M/S decode: Mid/Side → L/R (after STFT)
        if is_mid_side {
            const SQRT2_INV: f32 = std::f32::consts::FRAC_1_SQRT_2;
            for mut sample_block in buffer.iter_samples() {
                let mut ch = sample_block.iter_mut();
                if let (Some(m), Some(s)) = (ch.next(), ch.next()) {
                    let l = (*m + *s) * SQRT2_INV;
                    let r = (*m - *s) * SQRT2_INV;
                    *m = l;
                    *s = r;
                }
            }
        }

        // Post-processing dry/wet combine. Delta monitor wins over the global mix: when the
        // user turns on Δ they want to hear the removed signal, full stop. Otherwise crossfade
        // the wet output with the fft_size-delayed dry signal so mix=0 is a perfect bypass.
        if need_dry {
            let block_samples = buffer.samples();
            let mut dry_idx = 0usize;
            if delta_monitor {
                for sample_block in buffer.iter_samples() {
                    let read_pos =
                        (self.dry_delay_write + dry_idx + dry_delay_size - fft_size) % dry_delay_size;
                    for (ch_idx, sample) in sample_block.into_iter().enumerate() {
                        let dry_val = self.dry_delay[ch_idx * MAX_DRY_DELAY_SIZE + read_pos];
                        *sample = dry_val - *sample;
                    }
                    dry_idx += 1;
                }
            } else {
                let wet = global_mix;
                let dry = 1.0 - global_mix;
                for sample_block in buffer.iter_samples() {
                    let read_pos =
                        (self.dry_delay_write + dry_idx + dry_delay_size - fft_size) % dry_delay_size;
                    for (ch_idx, sample) in sample_block.into_iter().enumerate() {
                        let dry_val = self.dry_delay[ch_idx * MAX_DRY_DELAY_SIZE + read_pos];
                        *sample = wet * *sample + dry * dry_val;
                    }
                    dry_idx += 1;
                }
            }
            // Advance write head now that both write (above) and read are done.
            self.dry_delay_write = (self.dry_delay_write + block_samples) % dry_delay_size;
        }

        // Push latest spectra to GUI triple-buffers (allocation-free: mutate in-place then publish).
        // Bridge buffers are MAX_NUM_BINS; bins beyond num_bins are left as zero (silent).
        shared.spectrum_tx.input_buffer_mut().copy_from_slice(&spectrum_buf[..MAX_NUM_BINS]);
        shared.spectrum_tx.publish();
        shared.suppression_tx.input_buffer_mut().copy_from_slice(&suppression_buf[..MAX_NUM_BINS]);
        shared.suppression_tx.publish();

        // Publish SC envelope for the currently-edited slot, if it is an SC-aware Gain slot.
        let editing_slot = params.editing_slot.try_lock().map(|g| *g as usize).unwrap_or(0);
        let editing_is_gain = editing_slot < 9 &&
            matches!(slot_types_snap[editing_slot], crate::dsp::modules::ModuleType::Gain);
        if editing_is_gain {
            let src = &self.slot_sc_input[0][editing_slot];
            shared.sc_envelope_tx.input_buffer_mut().copy_from_slice(&src[..MAX_NUM_BINS]);
        } else {
            shared.sc_envelope_tx.input_buffer_mut().fill(0.0);
        }
        shared.sc_envelope_tx.publish();
    }
}

/// Test-only: run identity processing on a mono signal, return output Vec.
/// Uses raw FFT/OLA without StftHelper to avoid nih-plug Buffer complexity in tests.
/// Hidden from docs; compiled in all configurations so integration tests can reach it.
#[doc(hidden)]
pub fn process_block_for_test(input: &[f32], _sample_rate: f32) -> Vec<f32> {
    let mut planner = RealFftPlanner::<f32>::new();
    let fft  = planner.plan_fft_forward(FFT_SIZE);
    let ifft = planner.plan_fft_inverse(FFT_SIZE);
    let hop  = FFT_SIZE / OVERLAP;
    let norm = 2.0_f32 / (3.0 * FFT_SIZE as f32);

    let window: Vec<f32> = (0..FFT_SIZE)
        .map(|i| 0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32
            / (FFT_SIZE - 1) as f32).cos()))
        .collect();

    // Pre-pad by FFT_SIZE zeros to model pipeline latency
    let mut padded = vec![0.0f32; FFT_SIZE + input.len()];
    padded[FFT_SIZE..].copy_from_slice(input);

    let mut accum = vec![0.0f32; FFT_SIZE + input.len()];
    let num_hops = input.len() / hop;

    for h in 0..num_hops {
        let start = h * hop;
        let mut frame: Vec<f32> = (0..FFT_SIZE)
            .map(|i| padded[start + i] * window[i])
            .collect();

        let mut spectrum = fft.make_output_vec();
        fft.process(&mut frame, &mut spectrum).unwrap();

        // Identity: no modification to spectrum

        let mut out_frame = ifft.make_output_vec();
        ifft.process(&mut spectrum, &mut out_frame).unwrap();

        for i in 0..FFT_SIZE {
            accum[start + i] += out_frame[i] * window[i] * norm;
        }
    }

    // Return the full input.len() worth of samples starting at accum[0].
    // The first FFT_SIZE samples are the latency region (transition from zero-padding).
    // The test skips these via the `latency` offset, checking accum[FFT_SIZE..] vs input[..].
    accum[0..input.len()].to_vec()
}
