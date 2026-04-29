use num_complex::Complex;
use realfft::RealFftPlanner;
use nih_plug::util::StftHelper;
use crate::bridge::SharedState;
use crate::dsp::modules::PeakInfo;
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

/// Maximum capacity for the per-channel peak buffer used by Phase 4.2 PLPV
/// peak detection. The runtime `plpv_max_peaks` parameter is clamped to this
/// when written into `peak_buf`.
pub const MAX_PEAKS: usize = 256;

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
    /// PLPV: previous wrapped phase per channel. [channel][bin]
    prev_phase: Vec<Vec<f32>>,
    /// PLPV: previous unwrapped phase per channel. [channel][bin]
    prev_unwrapped_phase: Vec<Vec<f32>>,
    /// PLPV: current unwrapped phase per channel. Exposed to modules via ctx.unwrapped_phase.
    unwrapped_phase: Vec<Vec<f32>>,
    /// PLPV: per-channel re-wrap workspace used before iFFT. [channel][bin]
    rewrap_buf: Vec<Vec<f32>>,
    /// PLPV: mono scratch for the current hop's wrapped phase (filled per closure invocation).
    scratch_curr_phase: Vec<f32>,
    /// PLPV: mono scratch for per-bin expected cumulative phase advance (per closure invocation).
    scratch_expected: Vec<f32>,
    /// PLPV: mono scratch for per-bin magnitudes used by low-energy phase damping.
    scratch_mags: Vec<f32>,
    /// PLPV: per-channel pre-allocated peak buffer (Phase 4.2). Capacity MAX_PEAKS;
    /// `detect_peaks` writes only the first `n_peaks` entries each hop.
    peak_buf: Vec<Vec<PeakInfo>>,
    /// PLPV: per-channel cumulative hop counter feeding `expected_phase = 2π·k·N·H/F`.
    /// Each channel runs its own STFT closure call per hop, so they grow independently.
    /// `u64` so this never overflows in any realistic session length.
    total_hops_per_ch: [u64; 2],
    /// Per-channel rolling complex-spectrum history. Sized at construction from the
    /// History Depth param (in seconds). Reallocated by `reset()` if the requested
    /// capacity changes (allocation OK there — reset is not on the audio thread).
    history: crate::dsp::history_buffer::HistoryBuffer,
    history_depth_seconds: f32,
    /// Scratch pad for one hop's per-channel complex spectrum, copied out of
    /// the StftHelper closure and drained into `history` after the closure.
    pending_hop_frames: Vec<Vec<Complex<f32>>>,
    /// Per-bin |IF − f_centre| / f_centre cache, filled at the start of every
    /// process() call from the prior block's analysis FFT. Sized at
    /// MAX_NUM_BINS; only `[0..num_bins]` is meaningful for the active FFT.
    /// v1 always fills with zeros (IF == centre); replaced once Phase 4's
    /// per-channel IF lookup is plumbed in.
    if_offset_buf: Vec<f32>,
    sample_rate: f32,
    num_channels: usize,
}

impl Pipeline {
    pub fn new(
        sample_rate: f32,
        num_channels: usize,
        fft_size: usize,
        slot_types: &[crate::dsp::modules::ModuleType; 9],
        history_depth_seconds: f32,
    ) -> Self {
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

        // PLPV per-channel phase buffers (2 channels × MAX_NUM_BINS each).
        let prev_phase:           Vec<Vec<f32>> = (0..2).map(|_| vec![0.0f32; MAX_NUM_BINS]).collect();
        let prev_unwrapped_phase: Vec<Vec<f32>> = (0..2).map(|_| vec![0.0f32; MAX_NUM_BINS]).collect();
        let unwrapped_phase:      Vec<Vec<f32>> = (0..2).map(|_| vec![0.0f32; MAX_NUM_BINS]).collect();
        let rewrap_buf:           Vec<Vec<f32>> = (0..2).map(|_| vec![0.0f32; MAX_NUM_BINS]).collect();
        let scratch_curr_phase:   Vec<f32>      = vec![0.0f32; MAX_NUM_BINS];
        let scratch_expected:     Vec<f32>      = vec![0.0f32; MAX_NUM_BINS];
        let scratch_mags:         Vec<f32>      = vec![0.0f32; MAX_NUM_BINS];
        let peak_buf: Vec<Vec<PeakInfo>> = (0..2)
            .map(|_| vec![PeakInfo { k: 0, mag: 0.0, low_k: 0, high_k: 0 }; MAX_PEAKS])
            .collect();

        let history_capacity = {
            let hop = (fft_size / OVERLAP).max(1) as f32;
            ((history_depth_seconds * sample_rate) / hop).ceil() as usize
        }.max(1);
        let history = crate::dsp::history_buffer::HistoryBuffer::new(
            num_channels.max(1),
            history_capacity,
            num_bins,
        );
        let pending_hop_frames: Vec<Vec<Complex<f32>>> = (0..2)
            .map(|_| vec![Complex::new(0.0, 0.0); MAX_NUM_BINS])
            .collect();
        let if_offset_buf: Vec<f32> = vec![0.0; MAX_NUM_BINS];

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
            prev_phase,
            prev_unwrapped_phase,
            unwrapped_phase,
            rewrap_buf,
            scratch_curr_phase,
            scratch_expected,
            scratch_mags,
            peak_buf,
            total_hops_per_ch: [0; 2],
            history,
            history_depth_seconds,
            pending_hop_frames,
            if_offset_buf,
            sample_rate,
            fft_size,
            num_channels,
        }
    }

    /// Zero out DSP runtime state without touching FFT infrastructure.
    ///
    /// This is the **audio-thread path** for the GUI Reset button. It must not
    /// call `Pipeline::reset()` or any module `reset()` impl because those
    /// heap-allocate (RealFftPlanner, vec!, StftHelper::new, etc.).
    ///
    /// What this does:
    /// - Zeros every stateful f32/complex pre-allocated buffer in Pipeline and FxMatrix.
    /// - Resets the dry-delay write head to 0.
    /// - Zeros `pending_hop_frames` and `if_offset_buf`, and calls `HistoryBuffer::reset()`
    ///   (rewinds the ring write head + frame count + summary cache; allocation-free —
    ///   same Vec slots, just `fill(Complex::ZERO)`).
    /// - Does NOT reset StftHelper overlap-add ring buffers — those are private to
    ///   nih-plug and cannot be zeroed here. The result is a brief one-hop click, which
    ///   is acceptable for a user-initiated hard reset.
    /// - Does NOT call module.reset() — module envelope/state will be stale for one
    ///   FFT window then naturally overwritten by process(). This matches the behaviour
    ///   of any other parameter change.
    ///
    /// RT-safe: no allocation, no locking, no I/O.
    pub fn clear_state(&mut self) {
        self.dry_delay.fill(0.0);
        self.dry_delay_write = 0;
        for sc in &mut self.sc_envelopes  { sc.fill(0.0); }
        for sc in &mut self.sc_env_states { sc.fill(0.0); }
        for ch in &mut self.slot_sc_input {
            for slot_buf in ch.iter_mut() { slot_buf.fill(0.0); }
        }
        for ch_bufs in &mut self.sc_complex_bufs {
            ch_bufs.fill(Complex::new(0.0, 0.0));
        }
        for v in &mut self.prev_phase           { v.fill(0.0); }
        for v in &mut self.prev_unwrapped_phase { v.fill(0.0); }
        for v in &mut self.unwrapped_phase      { v.fill(0.0); }
        for v in &mut self.rewrap_buf           { v.fill(0.0); }
        self.scratch_curr_phase.fill(0.0);
        self.scratch_expected.fill(0.0);
        self.scratch_mags.fill(0.0);
        for ch in &mut self.peak_buf {
            ch.fill(PeakInfo { k: 0, mag: 0.0, low_k: 0, high_k: 0 });
        }
        self.total_hops_per_ch = [0; 2];
        for v in &mut self.pending_hop_frames { for c in v { *c = Complex::new(0.0, 0.0); } }
        for v in &mut self.if_offset_buf { *v = 0.0; }
        self.history.reset();
        self.fx_matrix.clear_state();
    }

    pub fn reset(&mut self, sample_rate: f32, num_channels: usize, history_depth_seconds: f32) {
        let fft_size = self.fft_size;
        let num_bins = fft_size / 2 + 1;
        self.sample_rate = sample_rate;
        self.num_channels = num_channels;

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
        for v in &mut self.prev_phase           { v.fill(0.0); }
        for v in &mut self.prev_unwrapped_phase { v.fill(0.0); }
        for v in &mut self.unwrapped_phase      { v.fill(0.0); }
        for v in &mut self.rewrap_buf           { v.fill(0.0); }
        self.scratch_curr_phase.fill(0.0);
        self.scratch_expected.fill(0.0);
        self.scratch_mags.fill(0.0);
        for ch in &mut self.peak_buf {
            ch.fill(PeakInfo { k: 0, mag: 0.0, low_k: 0, high_k: 0 });
        }
        self.total_hops_per_ch = [0; 2];
        for v in &mut self.pending_hop_frames { for c in v { *c = Complex::new(0.0, 0.0); } }
        for v in &mut self.if_offset_buf { *v = 0.0; }
        // History Buffer: rebuild if the depth changed; otherwise reset in place.
        let new_capacity = {
            let hop = (fft_size / OVERLAP).max(1) as f32;
            ((history_depth_seconds * sample_rate) / hop).ceil() as usize
        }.max(1);
        let needs_realloc = self.history_depth_seconds != history_depth_seconds
            || self.history.capacity_frames() != new_capacity
            || self.history.num_channels() != num_channels.max(1);
        if needs_realloc {
            self.history = crate::dsp::history_buffer::HistoryBuffer::new(
                num_channels.max(1),
                new_capacity,
                num_bins,
            );
            self.history_depth_seconds = history_depth_seconds;
        } else {
            self.history.reset();
        }
        // Reset clears all amp-node state — preset load + FFT-size change both warm up from zero.
        self.fx_matrix.reset(sample_rate, fft_size);
    }

    pub fn process(
        &mut self,
        buffer: &mut nih_plug::buffer::Buffer,
        aux: &mut nih_plug::prelude::AuxiliaryBuffers,
        shared: &mut SharedState,
        params: &crate::params::SpectralForgeParams,
        transport: &nih_plug::context::process::Transport,
    ) {
        use crate::dsp::modules::{apply_curve_transform, ModuleContext, TILT_MAX};
        use crate::editor::curve_config::curve_display_config;

        // Drain the GUI-side reset request flag. The swap is lock-free.
        // clear_state() zeros pre-allocated buffers only — no FFT planner, no
        // StftHelper construction, no module reset() calls that heap-allocate.
        if shared.reset_requested.swap(false, std::sync::atomic::Ordering::AcqRel) {
            self.clear_state();
        }

        let fft_size = self.fft_size;
        let num_bins = fft_size / 2 + 1;
        // History summary stats are valid only within one block. Modules
        // reading them get a cache-miss-then-cache-hit pattern; cleared here.
        self.history.clear_summary_cache();
        let block_size = buffer.samples() as u32;
        let attack_ms_base    = params.attack_ms.smoothed.next_step(block_size);
        let release_ms_base   = params.release_ms.smoothed.next_step(block_size);
        let input_gain_db     = params.input_gain.smoothed.next_step(block_size);
        let output_gain_db    = params.output_gain.smoothed.next_step(block_size);
        let global_mix        = params.mix.smoothed.next_step(block_size).clamp(0.0, 1.0);

        // Snapshot slot module types and gain modes early — needed for offset_fn lookup below.
        // Uses try_lock with fallback so we never block on the audio thread.
        let slot_types_snap: [crate::dsp::modules::ModuleType; 9] = params.slot_module_types.try_lock()
            .map(|g| *g)
            .unwrap_or([crate::dsp::modules::ModuleType::Empty; 9]);
        let slot_gain_mode_snap: [crate::dsp::modules::GainMode; 9] = params.slot_gain_mode.try_lock()
            .map(|g| *g)
            .unwrap_or([crate::dsp::modules::GainMode::Add; 9]);

        // ── Read all 9×7 slot curves from triple-buffer + apply tilt/offset/curvature ──
        // See docs/superpowers/specs/2026-04-23-ui-parameter-spec-design.md §2.
        for s in 0..9 {
            for c in 0..7 {
                self.slot_curve_cache[s][c]
                    .copy_from_slice(&shared.curve_rx[s][c].read()[..MAX_NUM_BINS]);
                let tilt_norm = params.tilt_param(s, c)
                    .map(|p| p.smoothed.next_step(block_size))
                    .unwrap_or(0.0);
                let offset = params.offset_param(s, c)
                    .map(|p| p.smoothed.next_step(block_size))
                    .unwrap_or(0.0);
                let curvature = params.curvature_param(s, c)
                    .map(|p| p.smoothed.next_step(block_size))
                    .unwrap_or(0.0);
                // Look up per-curve calibrated offset function (no allocation).
                let offset_fn = curve_display_config(
                    slot_types_snap[s],
                    c,
                    slot_gain_mode_snap[s],
                ).offset_fn;
                apply_curve_transform(
                    &mut self.slot_curve_cache[s][c],
                    tilt_norm * TILT_MAX,
                    offset,
                    curvature,
                    offset_fn,
                    self.sample_rate,
                    self.fft_size,
                );
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
        let enable_heavy_modules = params.enable_heavy_modules.value();
        let plpv_enable = params.plpv_enable.value();
        let plpv_dynamics_enable = params.plpv_dynamics_enable.value();
        let plpv_phase_smear_enable = params.plpv_phase_smear_enable.value();
        let plpv_freeze_enable = params.plpv_freeze_enable.value();
        let plpv_midside_enable = params.plpv_midside_enable.value();
        let plpv_phase_noise_floor_db = params.plpv_phase_noise_floor_db.smoothed.next_step(block_size);
        // Phase 4.2: control-rate peak-detection params. Read once per block.
        let max_peaks_capped: usize = (params.plpv_max_peaks.value() as usize).min(MAX_PEAKS);
        let peak_threshold_db: f32 = params.plpv_peak_threshold_db.smoothed.next_step(block_size);
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

        // Derive if_offset from the previous block's last analysis FFT. One-block
        // latency is acceptable for Past consumers and avoids a per-hop borrow
        // conflict with the StftHelper closure. v1 stub: all zeros (IF == centre).
        // Phase 4's per-channel IF lookup will replace `let inst = centre;` below.
        let inv_fft = 1.0_f32 / fft_size as f32;
        for k in 1..num_bins {
            let centre = k as f32 * self.sample_rate * inv_fft;
            let inst = centre; // TODO Phase 4 wiring: pull from per-channel IF cache.
            self.if_offset_buf[k] = (inst - centre) / centre.max(1e-6);
        }
        self.if_offset_buf[0] = 0.0;

        // Immutable borrow of history for ctx — captures the prior block's state.
        // The mutable write path (pending_hop_frames → history) happens after the
        // closure via the separate pending_hop_frames field.
        let history_ref: &crate::dsp::history_buffer::HistoryBuffer = &self.history;

        // Build ModuleContext
        let mut ctx = ModuleContext::new(
            self.sample_rate,
            fft_size,
            num_bins,
            attack_ms_base,
            release_ms_base,
            sensitivity,
            params.suppression_width.smoothed.next_step(block_size),
            params.auto_makeup.value(),
            delta_monitor,
        );
        // Phase 1 stub: BPM/beat read from host transport when present.
        // Modules consuming these don't ship until Phase 2 (Rhythm), so a 0.0
        // default is currently equivalent to "no BPM info available".
        ctx.bpm = transport.tempo.unwrap_or(0.0) as f32;
        ctx.beat_position = transport.pos_beats().unwrap_or(0.0);
        // Attach history as the *prior* block's snapshot — readers always look back.
        ctx.history = Some(history_ref);
        // Modules that read instantaneous-frequency offset (e.g. Past Stretch)
        // see the prior-block snapshot. None == not yet wired (we always wire
        // it now that the cache exists).
        ctx.if_offset = Some(&self.if_offset_buf[..num_bins]);

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

        // Propagate future modes each block (try_lock is non-blocking; skipped if GUI holds lock).
        if let Some(modes) = params.slot_future_mode.try_lock() {
            self.fx_matrix.set_future_modes(&*modes);
        }

        // Propagate punch modes each block (try_lock is non-blocking; skipped if GUI holds lock).
        if let Some(modes) = params.slot_punch_mode.try_lock() {
            self.fx_matrix.set_punch_modes(&*modes);
        }

        // Propagate geometry modes each block (try_lock is non-blocking; skipped if GUI holds lock).
        if let Some(modes) = params.slot_geometry_mode.try_lock() {
            self.fx_matrix.set_geometry_modes(&*modes);
        }

        // Propagate modulate modes each block (try_lock is non-blocking; skipped if GUI holds lock).
        if let Some(modes) = params.slot_modulate_mode.try_lock() {
            self.fx_matrix.set_modulate_modes(&*modes);
        }

        // Propagate circuit modes each block (try_lock is non-blocking; skipped if GUI holds lock).
        if let Some(modes) = params.slot_circuit_mode.try_lock() {
            self.fx_matrix.set_circuit_modes(&*modes);
        }

        // Propagate life modes each block (try_lock is non-blocking; skipped if GUI holds lock).
        if let Some(modes) = params.slot_life_mode.try_lock() {
            self.fx_matrix.set_life_modes(&*modes);
        }

        // Propagate past modes each block (try_lock is non-blocking; skipped if GUI holds lock).
        if let Some(modes) = params.slot_past_mode.try_lock() {
            self.fx_matrix.set_past_modes(&*modes);
        }
        if let Some(keys) = params.slot_past_sort_key.try_lock() {
            self.fx_matrix.set_past_sort_keys(&*keys);
        }

        // Propagate kinetics modes + sources each block (try_lock is non-blocking; skipped if GUI holds lock).
        if let Some(modes) = params.slot_kinetics_mode.try_lock() {
            self.fx_matrix.set_kinetics_modes(&*modes);
        }
        if let Some(well_srcs) = params.slot_kinetics_well_source.try_lock() {
            self.fx_matrix.set_kinetics_well_sources(&*well_srcs);
        }
        if let Some(mass_srcs) = params.slot_kinetics_mass_source.try_lock() {
            self.fx_matrix.set_kinetics_mass_sources(&*mass_srcs);
        }

        // Propagate rhythm modes + grids each block (try_lock is non-blocking; skipped if GUI holds lock).
        // The two locks are independent — if either is held by GUI, skip dispatch this block;
        // the next block will pick up the GUI-side write.
        if let (Some(modes), Some(grids)) = (
            params.slot_rhythm_mode.try_lock(),
            params.slot_arp_grid.try_lock(),
        ) {
            self.fx_matrix.set_rhythm_modes_and_grids(&*modes, &*grids);
        }

        // Phase 4.3a — propagate the Dynamics-PLPV enable flag each block. Lock-free
        // BoolParam read above; this just walks the 9 slots and pokes the trait method
        // (no-op for everything except DynamicsModule).
        self.fx_matrix.set_plpv_dynamics_enable(plpv_dynamics_enable);

        // Phase 4.3b — propagate the PhaseSmear-PLPV enable flag each block. Same
        // pattern as 4.3a; trait default is a no-op for non-PhaseSmear modules.
        self.fx_matrix.set_plpv_phase_smear_enable(plpv_phase_smear_enable);

        // Phase 4.3c — propagate the Freeze-PLPV enable flag each block. Same
        // pattern as 4.3a/b; trait default is a no-op for non-Freeze modules.
        self.fx_matrix.set_plpv_freeze_enable(plpv_freeze_enable);

        // Phase 4.3d — propagate the MidSide-PLPV enable flag each block. Same
        // pattern; trait default is a no-op for non-MidSide modules.
        self.fx_matrix.set_plpv_midside_enable(plpv_midside_enable);

        // Build route matrix from automatable params each block.
        // virtual_rows + amp_mode + amp_params are not exposed as automation
        // targets, so we read them from the Mutex — but never block waiting for it.
        let (virt, amp_mode, amp_params) = params.route_matrix.try_lock()
            .map(|g| (g.virtual_rows, g.amp_mode, g.amp_params))
            .unwrap_or_else(|| (
                Default::default(),
                crate::dsp::modules::default_amp_modes(),
                crate::dsp::modules::default_amp_params(),
            ));
        let mut route_matrix_snap = crate::dsp::modules::RouteMatrix {
            send: [[0.0f32; crate::dsp::modules::MAX_SLOTS]; crate::dsp::modules::MAX_MATRIX_ROWS],
            virtual_rows: virt,
            amp_mode,
            amp_params,
        };
        for r in 0..crate::param_ids::NUM_MATRIX_ROWS {
            for col in 0..crate::param_ids::NUM_SLOTS {
                if r == col { continue; } // skip diagonal to prevent self-feedback
                if let Some(p) = params.matrix_cell(r, col) {
                    route_matrix_snap.send[col][r] = p.smoothed.next();
                }
            }
        }

        // Sync amp state to the route matrix once per audio block, before any hop.
        // permit_alloc: sync_amp_modes may allocate if mode changed (user action).
        self.fx_matrix.sync_amp_modes(&route_matrix_snap, num_bins);

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
        // PLPV per-channel buffers + mono scratch (rebound here so the closure can borrow them
        // without taking a second &mut self alongside &mut self.stft).
        let prev_phase_ref           = &mut self.prev_phase;
        let prev_unwrapped_phase_ref = &mut self.prev_unwrapped_phase;
        let unwrapped_phase_ref      = &mut self.unwrapped_phase;
        let rewrap_buf_ref           = &mut self.rewrap_buf;
        let scratch_curr_phase_ref   = &mut self.scratch_curr_phase;
        let scratch_expected_ref     = &mut self.scratch_expected;
        let scratch_mags_ref         = &mut self.scratch_mags;
        let peak_buf_ref             = &mut self.peak_buf;
        let total_hops_ref           = &mut self.total_hops_per_ch;
        let pending_hop_frames       = &mut self.pending_hop_frames;
        let mut pending_hops: usize  = 0;
        let stft_num_channels        = self.stft.num_channels();
        // Reset peak-hold accumulators.
        for v in spectrum_buf.iter_mut()   { *v = 0.0; }
        for v in suppression_buf.iter_mut() { *v = 0.0; }
        // IFFT gives fft_size gain; Hann^2 OLA at 75% overlap gives 1.5 gain.
        // Combined normalization: 1 / (fft_size * 1.5) = 2 / (3 * fft_size)
        let norm = 2.0_f32 / (3.0 * fft_size as f32);
        let hop_size = fft_size / OVERLAP;

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

            // Copy this hop's complex spectrum into the pending pad. Drained into
            // `history` after the closure completes — the closure cannot mutate
            // `self.history` while StftHelper holds the &mut on the buffer.
            pending_hop_frames[channel.min(1)][..num_bins].copy_from_slice(&complex_buf[..num_bins]);
            if channel + 1 == stft_num_channels {
                pending_hops += 1;
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

            // PLPV: compute per-bin unwrapped phase trajectory before module dispatch.
            // Defensively clamp ch index to the 2 channels we pre-allocated for.
            let ch = channel.min(1);
            let mut hop_ctx = ctx;
            if plpv_enable {
                for k in 0..num_bins {
                    scratch_curr_phase_ref[k] = complex_buf[k].arg();
                }
                crate::dsp::plpv::unwrap_phase(
                    &scratch_curr_phase_ref[..num_bins],
                    &prev_phase_ref[ch][..num_bins],
                    &mut prev_unwrapped_phase_ref[ch][..num_bins],
                    &mut unwrapped_phase_ref[ch][..num_bins],
                    fft_size,
                    hop_size,
                    num_bins,
                );
                // Roll prev_phase forward for the next hop.
                prev_phase_ref[ch][..num_bins]
                    .copy_from_slice(&scratch_curr_phase_ref[..num_bins]);

                // Phase 4.1.5: damp low-energy bins toward expected cumulative advance.
                // Per-channel hop counter so two channels grow independently at the
                // correct per-channel rate. Acceptable f32-precision loss after ~30 h.
                let hop_total = total_hops_ref[ch] as f32;
                let two_pi_hop_over_n = 2.0 * std::f32::consts::PI
                    * (hop_size as f32) / (fft_size as f32);
                for k in 0..num_bins {
                    scratch_expected_ref[k] = two_pi_hop_over_n * (k as f32) * hop_total;
                }
                for k in 0..num_bins {
                    scratch_mags_ref[k] = complex_buf[k].norm();
                }
                crate::dsp::plpv::damp_low_energy_bins(
                    &mut unwrapped_phase_ref[ch][..num_bins],
                    &scratch_mags_ref[..num_bins],
                    &scratch_expected_ref[..num_bins],
                    plpv_phase_noise_floor_db,
                    num_bins,
                );
                total_hops_ref[ch] = total_hops_ref[ch].wrapping_add(1);

                // Phase 4.2: detect spectral peaks + assign Voronoi skirts.
                // Operates on the raw FFT magnitudes already in scratch_mags_ref
                // (filled above for damping; identical input).
                let n_peaks = crate::dsp::plpv::detect_peaks(
                    &scratch_mags_ref[..num_bins],
                    num_bins,
                    peak_threshold_db,
                    max_peaks_capped,
                    &mut peak_buf_ref[ch][..],
                );
                crate::dsp::plpv::assign_voronoi_skirts(
                    &mut peak_buf_ref[ch][..n_peaks],
                    num_bins,
                );
                hop_ctx.peaks = Some(&peak_buf_ref[ch][..n_peaks]);

                // Expose unwrapped phase to modules. Phase 4.3b: hand out a slice of
                // `Cell<f32>` (alloc-free, unsafe-free) so PLPV-aware modules can both
                // read AND write through the same field while ModuleContext stays Copy.
                // The Pipeline's re-wrap stage below reads `unwrapped_phase_ref` directly
                // as `&[f32]`, which sees the same memory the Cell-slice writes through.
                hop_ctx.unwrapped_phase = Some(
                    std::cell::Cell::from_mut(&mut unwrapped_phase_ref[ch][..num_bins])
                        .as_slice_of_cells(),
                );
            }

            // Run all modules through the fx_matrix slot chain.
            fx_matrix.process_hop(
                channel,
                stereo_link,
                complex_buf,
                &sc_args,
                &slot_targets_snap,
                slot_curve_cache_ref,
                &route_matrix_snap,
                &hop_ctx,
                channel_supp_buf,
                num_bins,
                enable_heavy_modules,
            );
            for k in 0..channel_supp_buf.len() {
                if channel_supp_buf[k] > suppression_buf[k] { suppression_buf[k] = channel_supp_buf[k]; }
            }

            // PLPV: re-wrap unwrapped phase back into (-π, π] and recombine with the (possibly
            // modified) magnitude. When no module touched ctx.unwrapped_phase this is a no-op
            // up to f32 round-off (typically ≤ 1 ULP).
            if plpv_enable {
                crate::dsp::plpv::rewrap_phase(
                    &unwrapped_phase_ref[ch][..num_bins],
                    &mut rewrap_buf_ref[ch][..num_bins],
                    num_bins,
                );
                for k in 0..num_bins {
                    let m = complex_buf[k].norm();
                    let p = rewrap_buf_ref[ch][k];
                    complex_buf[k] = Complex::from_polar(m, p);
                }
            }

            ifft_plan.process(complex_buf, block).unwrap();

            // Synthesis window + IFFT normalization + output gain
            for (s, &w) in block.iter_mut().zip(window.iter()) {
                *s *= w * norm * output_linear;
            }
        });

        // Drain pending hop frames into history after the StftHelper closure.
        // Multi-hop blocks (block_size > hop_size) overwrite the pad each iteration,
        // so at most ONE frame lands in history per block — `pending_hops > 0`
        // guards against block_size < hop_size (no hop completed inside the closure).
        // The pad and history both top out at the StftHelper's channel count;
        // bound the loop on `history.num_channels()` so the drain can never write
        // past whatever HistoryBuffer was sized for at construction.
        if pending_hops > 0 {
            let channels = stft_num_channels
                .min(self.history.num_channels())
                .min(self.pending_hop_frames.len());
            for ch in 0..channels {
                self.history.write_hop(ch, &self.pending_hop_frames[ch][..num_bins]);
            }
            self.history.advance_after_all_channels_written();
        }

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

    /// Test-only snapshot of HistoryBuffer state. Used by `tests/calibration.rs`
    /// to assert the buffer fills, summary stats stay finite, and depth changes
    /// take effect.
    #[cfg(any(test, feature = "probe"))]
    pub fn history_probe(&self, channel: usize) -> HistoryProbe {
        let frames_used = self.history.frames_used();
        let capacity    = self.history.capacity_frames();
        let decay = self.history.summary_decay_estimate(channel);
        let rms   = self.history.summary_rms_envelope(channel);
        let stab  = self.history.summary_if_stability(channel);
        HistoryProbe {
            frames_used,
            capacity,
            summary_decay_max:        decay.iter().cloned().fold(0.0f32, f32::max),
            summary_rms_max:          rms.iter().cloned().fold(0.0f32, f32::max),
            summary_if_stability_max: stab.iter().cloned().fold(0.0f32, f32::max),
        }
    }
}

#[cfg(any(test, feature = "probe"))]
#[derive(Clone, Copy, Debug)]
pub struct HistoryProbe {
    pub frames_used: usize,
    pub capacity:    usize,
    pub summary_decay_max:        f32,
    pub summary_rms_max:          f32,
    pub summary_if_stability_max: f32,
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
