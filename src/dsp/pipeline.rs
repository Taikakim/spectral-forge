use num_complex::Complex;
use realfft::RealFftPlanner;
use nih_plug::util::StftHelper;
use crate::dsp::engines::{BinParams, SpectralEngine, create_engine, EngineSelection};
use crate::bridge::SharedState;

pub const FFT_SIZE: usize = 2048;
pub const NUM_BINS: usize = FFT_SIZE / 2 + 1;
pub const OVERLAP: usize = 4; // 75% overlap → hop = 512

/// Maximum block size assumed for the delta monitor dry-delay ring buffer.
/// nih-plug typically processes in blocks of ≤ 8192 samples.
const MAX_BLOCK_SIZE: usize = 8192;

/// Ring-buffer size per channel for the dry-signal delay in the delta monitor.
/// Must be ≥ FFT_SIZE + MAX_BLOCK_SIZE so the ring never overwrites samples still needed.
const DRY_DELAY_SIZE: usize = FFT_SIZE + MAX_BLOCK_SIZE;

pub struct Pipeline {
    stft: StftHelper,
    fft_plan:  std::sync::Arc<dyn realfft::RealToComplex<f32>>,
    ifft_plan: std::sync::Arc<dyn realfft::ComplexToReal<f32>>,
    window:         Vec<f32>,
    spectrum_buf:    Vec<f32>,
    suppression_buf: Vec<f32>,
    channel_supp_buf: Vec<f32>,
    complex_buf:     Vec<Complex<f32>>,
    sc_stft:      StftHelper,
    sc_envelope:  Vec<f32>,   // smoothed sidechain magnitude per bin
    sc_env_state: Vec<f32>,   // one-pole LP state (separate from main envelope)
    sc_complex_buf: Vec<Complex<f32>>,
    engine:   Box<dyn SpectralEngine>,
    engine_r: Box<dyn SpectralEngine>,  // right-channel engine for Independent mode
    bp_threshold: Vec<f32>,
    bp_ratio:     Vec<f32>,
    bp_attack:    Vec<f32>,
    bp_release:   Vec<f32>,
    bp_knee:      Vec<f32>,
    bp_makeup:    Vec<f32>,
    bp_mix:       Vec<f32>,
    /// Ring buffer for delta monitor dry-signal delay: 2 channels × DRY_DELAY_SIZE entries.
    /// Channel c occupies [c * DRY_DELAY_SIZE .. (c+1) * DRY_DELAY_SIZE].
    /// Delayed by FFT_SIZE samples to align dry with STFT-latency-compensated wet.
    dry_delay: Vec<f32>,
    /// Current write head into dry_delay (wraps at DRY_DELAY_SIZE).
    dry_delay_write: usize,
    /// Captured full complex spectrum for Spectral Freeze (Re+Im per bin).
    frozen_bins: Vec<Complex<f32>>,
    /// True once Freeze has captured its first frame; reset to false when mode changes.
    freeze_captured: bool,
    /// xorshift64 PRNG state for Phase Randomize. Must never be zero.
    rng_state: u64,
    /// Per-bin smoothed magnitude envelope for Spectral Contrast (~200 ms window).
    contrast_envelope: Vec<f32>,
    /// Pre-allocated curve read caches — populated via copy_from_slice each block
    /// so the audio thread never allocates. One Vec per curve channel (7 total).
    curve_cache: [Vec<f32>; 7],
    sample_rate: f32,
}

impl Pipeline {
    pub fn new(sample_rate: f32, num_channels: usize) -> Self {
        let mut planner = RealFftPlanner::<f32>::new();
        let fft_plan  = planner.plan_fft_forward(FFT_SIZE);
        let ifft_plan = planner.plan_fft_inverse(FFT_SIZE);

        let window: Vec<f32> = (0..FFT_SIZE)
            .map(|i| 0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32
                / (FFT_SIZE - 1) as f32).cos()))
            .collect();

        let complex_buf    = fft_plan.make_output_vec();
        let sc_complex_buf = fft_plan.make_output_vec();

        let mut engine = create_engine(EngineSelection::SpectralCompressor);
        engine.reset(sample_rate, FFT_SIZE);
        let mut engine_r = create_engine(EngineSelection::SpectralCompressor);
        engine_r.reset(sample_rate, FFT_SIZE);

        Self {
            stft: StftHelper::new(num_channels, FFT_SIZE, 0),
            sc_stft:      StftHelper::new(2, FFT_SIZE, 0),
            sc_envelope:  vec![0.0f32; NUM_BINS],
            sc_env_state: vec![0.0f32; NUM_BINS],
            sc_complex_buf,
            fft_plan,
            ifft_plan,
            window,
            spectrum_buf:     vec![0.0; NUM_BINS],
            suppression_buf:  vec![0.0; NUM_BINS],
            channel_supp_buf: vec![0.0; NUM_BINS],
            complex_buf,
            engine,
            engine_r,
            bp_threshold: vec![-20.0; NUM_BINS],
            bp_ratio:     vec![4.0;   NUM_BINS],
            bp_attack:    vec![10.0;  NUM_BINS],
            bp_release:   vec![80.0;  NUM_BINS],
            bp_knee:      vec![6.0;   NUM_BINS],
            bp_makeup:    vec![0.0;   NUM_BINS],
            bp_mix:       vec![1.0;   NUM_BINS],
            dry_delay: vec![0.0f32; 2 * DRY_DELAY_SIZE],
            dry_delay_write: 0,
            frozen_bins:       vec![Complex::new(0.0f32, 0.0f32); NUM_BINS],
            freeze_captured:   false,
            rng_state:         0xdeadbeef_cafebabe_u64,
            contrast_envelope: vec![0.0f32; NUM_BINS],
            curve_cache: std::array::from_fn(|_| vec![1.0f32; NUM_BINS]),
            sample_rate,
        }
    }

    pub fn reset(&mut self, sample_rate: f32, num_channels: usize) {
        self.sample_rate = sample_rate;
        self.stft    = StftHelper::new(num_channels, FFT_SIZE, 0);
        self.sc_stft = StftHelper::new(2, FFT_SIZE, 0);
        self.sc_envelope  = vec![0.0f32; NUM_BINS];
        self.sc_env_state = vec![0.0f32; NUM_BINS];
        self.dry_delay.fill(0.0);
        self.dry_delay_write = 0;
        for b in self.frozen_bins.iter_mut() { *b = Complex::new(0.0, 0.0); }
        self.freeze_captured = false;
        // rng_state intentionally not reset — continuity across SR changes is harmless
        self.contrast_envelope.fill(0.0);
        self.engine.reset(sample_rate, FFT_SIZE);
        self.engine_r.reset(sample_rate, FFT_SIZE);
    }

    pub fn process(
        &mut self,
        buffer: &mut nih_plug::buffer::Buffer,
        aux: &mut nih_plug::prelude::AuxiliaryBuffers,
        shared: &mut SharedState,
        params: &crate::params::SpectralForgeParams,
    ) {
        // Advance each smoother by block_size samples so it converges in wall-clock time
        // matching its configured ms value, regardless of block size.
        // Without this, calling next() once per block only steps 1/block_size of the way
        // through a smoother configured for N samples, making changes N× too slow.
        let block_size = buffer.samples() as u32;
        let attack_ms_base    = params.attack_ms.smoothed.next_step(block_size);
        let release_ms_base   = params.release_ms.smoothed.next_step(block_size);
        let freq_scale        = params.freq_scale.smoothed.next_step(block_size);
        let input_gain_db     = params.input_gain.smoothed.next_step(block_size);
        let output_gain_db    = params.output_gain.smoothed.next_step(block_size);
        let global_mix        = params.mix.smoothed.next_step(block_size);
        let suppression_width  = params.suppression_width.smoothed.next_step(block_size);
        let threshold_slope_db = params.threshold_slope.smoothed.next_step(block_size);

        let effect_mode          = params.effect_mode.value();
        let phase_rand_amount    = params.phase_rand_amount.smoothed.next_step(block_size);
        let spectral_contrast_db = params.spectral_contrast_db.smoothed.next_step(block_size);

        // Read all 7 curve channels into pre-allocated cache buffers (no allocation).
        // Each read() borrow ends before the next copy_from_slice begins.
        self.curve_cache[0].copy_from_slice(shared.curve_rx[0].read());
        self.curve_cache[1].copy_from_slice(shared.curve_rx[1].read());
        self.curve_cache[2].copy_from_slice(shared.curve_rx[2].read());
        self.curve_cache[3].copy_from_slice(shared.curve_rx[3].read());
        self.curve_cache[4].copy_from_slice(shared.curve_rx[4].read());
        self.curve_cache[5].copy_from_slice(shared.curve_rx[5].read());
        self.curve_cache[6].copy_from_slice(shared.curve_rx[6].read());

        // --- Sidechain processing ---
        let sc_active = !aux.inputs.is_empty();

        let sc_gain_db    = params.sc_gain.smoothed.next_step(block_size);
        let sc_gain_lin   = 10.0f32.powf(sc_gain_db / 20.0);
        let sc_attack_ms  = params.sc_attack_ms.smoothed.next_step(block_size);
        let sc_release_ms = params.sc_release_ms.smoothed.next_step(block_size);

        if sc_active {
            for v in self.sc_envelope.iter_mut() { *v = 0.0; }

            let sc_stft      = &mut self.sc_stft;
            let sc_envelope  = &mut self.sc_envelope;
            let sc_env_state = &mut self.sc_env_state;
            let sc_complex   = &mut self.sc_complex_buf;
            let sample_rate  = self.sample_rate;
            let hop = FFT_SIZE / OVERLAP;

            let fft_plan = self.fft_plan.clone();
            let window   = &self.window;

            sc_stft.process_overlap_add(&mut aux.inputs[0], OVERLAP, |_ch, block| {
                for (s, &w) in block.iter_mut().zip(window.iter()) {
                    *s *= w * sc_gain_lin;
                }
                crate::dsp::guard::sanitize(block);
                fft_plan.process(block, sc_complex).unwrap();

                let hops_per_sec = sample_rate / hop as f32;
                for k in 0..sc_complex.len() {
                    let mag = sc_complex[k].norm();
                    let coeff = if mag > sc_env_state[k] {
                        let time_hops = sc_attack_ms.max(0.1) * 0.001 * hops_per_sec;
                        (-1.0_f32 / time_hops).exp()
                    } else {
                        let time_hops = sc_release_ms.max(1.0) * 0.001 * hops_per_sec;
                        (-1.0_f32 / time_hops).exp()
                    };
                    sc_env_state[k] = coeff * sc_env_state[k] + (1.0 - coeff) * mag;
                    if sc_env_state[k] > sc_envelope[k] { sc_envelope[k] = sc_env_state[k]; }
                }
            });
        }

        // Map to physical units, bin by bin
        let sample_rate = self.sample_rate;
        let num_bins = self.bp_threshold.len();

        // Map curve cache values to physical units, bin by bin.
        // Rust 2021 split field borrows: curve_cache (read) and bp_* (write) are disjoint fields.
        for k in 0..num_bins {
            // Threshold: curve gain 1.0 (neutral node) → -20 dBFS.
            // Log-based mapping amplifies the ±18 dB node range to ±60 dBFS:
            //   y = -1 → gain ≈ 0.126 (−18 dB) → threshold = −80 dBFS
            //   y =  0 → gain = 1.0  (  0 dB) → threshold = −20 dBFS
            //   y = +1 → gain ≈ 7.94 (+18 dB) → threshold →  0 dBFS (clamped)
            // threshold_slope_db adds a linear tilt (dB/octave, pivot 1 kHz).
            let t = self.curve_cache[0].get(k).copied().unwrap_or(1.0);
            let t_db = if t > 1e-10 { 20.0 * t.log10() } else { -120.0 };
            let f_k_hz = (k as f32 * sample_rate / FFT_SIZE as f32).max(20.0);
            let slope_offset = threshold_slope_db * (f_k_hz / 1000.0_f32).log2();
            self.bp_threshold[k] = (-20.0 + t_db * (60.0 / 18.0) + slope_offset).clamp(-80.0, 0.0);

            // Ratio: curve gain 1.0 → ratio 1:1; gain 8.0 → ratio 8:1 (max)
            let r = self.curve_cache[1].get(k).copied().unwrap_or(1.0);
            self.bp_ratio[k] = r.clamp(1.0, 20.0);

            // Frequency-dependent timing: lower frequencies get longer times
            let f_bin = (k as f32 * sample_rate / crate::dsp::pipeline::FFT_SIZE as f32).max(20.0);
            let scale = (1000.0_f32 / f_bin).powf(freq_scale * 0.5); // freq_scale ∈ [0,1]
            let atk_factor = self.curve_cache[2].get(k).copied().unwrap_or(1.0).max(0.01);
            let rel_factor = self.curve_cache[3].get(k).copied().unwrap_or(1.0).max(0.01);
            self.bp_attack[k]  = (attack_ms_base  * scale * atk_factor).clamp(0.1, 500.0);
            self.bp_release[k] = (release_ms_base * scale * rel_factor).clamp(1.0, 2000.0);

            // Knee: curve gain 1.0 → 6 dB knee; range 0…24 dB
            let kn = self.curve_cache[4].get(k).copied().unwrap_or(1.0);
            self.bp_knee[k] = (kn * 6.0).clamp(0.0, 48.0);

            // Makeup: curve gain as dB (1.0 → 0 dB, >1 → positive makeup)
            let mk = self.curve_cache[5].get(k).copied().unwrap_or(1.0);
            self.bp_makeup[k] = if mk > 1e-6 { 20.0 * mk.log10() } else { -96.0 };

            // Mix: curve gain 1.0 → full wet; scaled so 1.0 is 100%
            let mx = self.curve_cache[6].get(k).copied().unwrap_or(1.0);
            self.bp_mix[k] = (mx * global_mix).clamp(0.0, 1.0);
        }

        // Read lookahead parameter.
        // TODO: implement per-channel delay ring buffer for actual lookahead.
        // Currently the STFT latency (FFT_SIZE samples) provides effective lookahead;
        // `lookahead_ms` is reserved for a future delay-line implementation.
        let _lookahead_ms = params.lookahead_ms.value();

        // Read boolean feature flags
        let auto_makeup   = params.auto_makeup.value();
        let delta_monitor = params.delta_monitor.value();

        // Read stereo link mode
        use crate::params::StereoLink;
        let stereo_link    = params.stereo_link.value();
        let is_independent = stereo_link == StereoLink::Independent;
        let is_mid_side    = stereo_link == StereoLink::MidSide;

        // Precompute input/output linear gains for capture into STFT closure
        let input_linear  = 10.0f32.powf(input_gain_db  / 20.0);
        let output_linear = 10.0f32.powf(output_gain_db / 20.0);

        // If the sidechain bus exists but is silent (e.g. nothing connected in DAW),
        // fall back to self-detection so normal compression still works.
        let sc_has_signal = sc_active && self.sc_envelope.iter().any(|&v| v > 1e-9);
        shared.sidechain_active.store(sc_has_signal, std::sync::atomic::Ordering::Relaxed);

        // Capture sc_envelope before the mutable borrow of self.stft
        let sc_envelope = &self.sc_envelope;
        let sidechain_arg: Option<&[f32]> = if sc_has_signal { Some(sc_envelope) } else { None };

        // Delta monitor: write dry samples into the ring buffer at the current write head.
        // They will be read back (delayed by FFT_SIZE) after STFT to align with wet latency.
        if delta_monitor {
            let mut dry_idx = 0usize;
            for sample_block in buffer.iter_samples() {
                debug_assert!(dry_idx < MAX_BLOCK_SIZE, "block size exceeded MAX_BLOCK_SIZE={MAX_BLOCK_SIZE}");
                let pos = (self.dry_delay_write + dry_idx) % DRY_DELAY_SIZE;
                for (ch_idx, sample) in sample_block.into_iter().enumerate() {
                    self.dry_delay[ch_idx * DRY_DELAY_SIZE + pos] = *sample;
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

        // Reborrow fields as locals so the closure can capture them without
        // conflicting with the &mut self.stft borrow inside process_overlap_add.
        let fft_plan  = self.fft_plan.clone();
        let ifft_plan = self.ifft_plan.clone();
        let window         = &self.window;
        let engine            = &mut self.engine;
        let engine_r          = &mut self.engine_r;
        let complex_buf       = &mut self.complex_buf;
        let spectrum_buf      = &mut self.spectrum_buf;
        let suppression_buf   = &mut self.suppression_buf;
        let channel_supp_buf  = &mut self.channel_supp_buf;
        // Reset peak-hold accumulators. channel_supp_buf is not zeroed here
        // because process_bins fully overwrites it before the fold below.
        for v in spectrum_buf.iter_mut()   { *v = 0.0; }
        for v in suppression_buf.iter_mut() { *v = 0.0; }
        let bp_threshold = &self.bp_threshold;
        let bp_ratio     = &self.bp_ratio;
        let bp_attack    = &self.bp_attack;
        let bp_release   = &self.bp_release;
        let bp_knee      = &self.bp_knee;
        let bp_makeup    = &self.bp_makeup;
        let bp_mix       = &self.bp_mix;
        // ThresholdMode::Relative legacy flag maps to sensitivity=1.0 if set;
        // otherwise use the continuous sensitivity parameter.
        let sensitivity = if params.threshold_mode.value() == crate::params::ThresholdMode::Relative {
            1.0f32
        } else {
            params.sensitivity.smoothed.next_step(block_size)
        };
        let sample_rate  = self.sample_rate;
        // IFFT gives FFT_SIZE gain; Hann^2 OLA at 75% overlap gives 1.5 gain.
        // Combined normalization: 1 / (FFT_SIZE * 1.5) = 2 / (3 * FFT_SIZE)
        let norm = 2.0_f32 / (3.0 * FFT_SIZE as f32);

        let frozen_bins       = &mut self.frozen_bins;
        let freeze_captured   = &mut self.freeze_captured;
        let rng_state         = &mut self.rng_state;
        let contrast_envelope = &mut self.contrast_envelope;

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

            let params = BinParams {
                threshold_db:       bp_threshold,
                ratio:              bp_ratio,
                attack_ms:          bp_attack,
                release_ms:         bp_release,
                knee_db:            bp_knee,
                makeup_db:          bp_makeup,
                mix:                bp_mix,
                sensitivity,
                auto_makeup,
                smoothing_semitones: suppression_width,
            };

            // Select engine: Independent mode uses engine_r for channel 1
            let active_engine: &mut Box<dyn SpectralEngine> = if is_independent && channel == 1 {
                engine_r
            } else {
                engine
            };
            active_engine.process_bins(complex_buf, sidechain_arg, &params, sample_rate, channel_supp_buf);
            for k in 0..channel_supp_buf.len() {
                if channel_supp_buf[k] > suppression_buf[k] { suppression_buf[k] = channel_supp_buf[k]; }
            }

            // Effects pass — modifies complex_buf in-place after compression.
            match effect_mode {
                crate::params::EffectMode::Bypass => {}

                crate::params::EffectMode::Freeze => {
                    if !*freeze_captured {
                        // Capture the entire complex frame (magnitude + phase).
                        // Copying Re+Im ensures DC and Nyquist bins stay real,
                        // and avoids the amplitude spikes from phase-drift reconstruction.
                        frozen_bins.copy_from_slice(complex_buf);
                        *freeze_captured = true;
                    }
                    // Replace current frame wholesale with the captured one.
                    complex_buf.copy_from_slice(frozen_bins);
                }

                crate::params::EffectMode::PhaseRand => {
                    let scale = phase_rand_amount * std::f32::consts::PI;
                    let last  = complex_buf.len() - 1;
                    for k in 0..complex_buf.len() {
                        // Always advance PRNG to keep the sequence independent of skipping.
                        *rng_state ^= *rng_state << 13;
                        *rng_state ^= *rng_state >> 7;
                        *rng_state ^= *rng_state << 17;
                        // DC (k=0) and Nyquist (k=last) must stay real for IFFT
                        // correctness — skipping phase rotation preserves Im=0.
                        if k == 0 || k == last { continue; }
                        let rand_phase = (*rng_state as f32 / u64::MAX as f32 * 2.0 - 1.0) * scale;
                        let (mag, phase) = (complex_buf[k].norm(), complex_buf[k].arg());
                        complex_buf[k] = Complex::from_polar(mag, phase + rand_phase);
                    }
                }

                crate::params::EffectMode::SpectralContrast => {
                    let hop_sz = FFT_SIZE / OVERLAP;
                    let time_hops = 0.2_f32 * sample_rate / hop_sz as f32;
                    let coeff = (-1.0_f32 / time_hops).exp();
                    let n = complex_buf.len();

                    // Pass 1 — update temporal envelope from raw magnitudes.
                    // All envelopes are computed from unmodified bins before any gain
                    // is applied, so no bin's gain computation sees already-modified data.
                    for k in 0..n {
                        let mag = complex_buf[k].norm();
                        contrast_envelope[k] = coeff * contrast_envelope[k] + (1.0 - coeff) * mag;
                    }

                    // Pass 2 — apply boost/cut relative to the now-updated envelopes.
                    // Clamping depth to [-12, +12]: positive depth = expand contrast,
                    // negative depth = flatten (bring peaks toward neighbours).
                    let depth = spectral_contrast_db.clamp(-12.0, 12.0);
                    let boost = 10.0f32.powf( depth / 20.0);
                    let cut   = 10.0f32.powf(-depth / 20.0);
                    for k in 0..n {
                        let mag = complex_buf[k].norm();
                        let env = contrast_envelope[k].max(1e-10);
                        let gain = if mag >= env { boost } else { cut };
                        complex_buf[k] *= gain;
                    }
                }
            }

            // When leaving Freeze mode, clear the captured flag so re-engaging always
            // captures a fresh spectrum.
            if effect_mode != crate::params::EffectMode::Freeze {
                *freeze_captured = false;
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

        // Delta monitor: output dry(delayed by FFT_SIZE) − wet = the removed signal.
        // Reading FFT_SIZE positions behind the write head aligns the captured dry with
        // the STFT-latency-shifted wet output, giving a clean difference signal.
        if delta_monitor {
            let block_samples = buffer.samples();
            let mut dry_idx = 0usize;
            for sample_block in buffer.iter_samples() {
                let read_pos =
                    (self.dry_delay_write + dry_idx + DRY_DELAY_SIZE - FFT_SIZE) % DRY_DELAY_SIZE;
                for (ch_idx, sample) in sample_block.into_iter().enumerate() {
                    let dry_val = self.dry_delay[ch_idx * DRY_DELAY_SIZE + read_pos];
                    *sample = dry_val - *sample;
                }
                dry_idx += 1;
            }
            // Advance write head now that both write (above) and read are done
            self.dry_delay_write = (self.dry_delay_write + block_samples) % DRY_DELAY_SIZE;
        }

        // Push latest spectra to GUI triple-buffers (allocation-free: mutate in-place then publish)
        shared.spectrum_tx.input_buffer_mut().copy_from_slice(spectrum_buf);
        shared.spectrum_tx.publish();
        shared.suppression_tx.input_buffer_mut().copy_from_slice(suppression_buf);
        shared.suppression_tx.publish();
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
