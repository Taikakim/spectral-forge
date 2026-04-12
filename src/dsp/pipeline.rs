use num_complex::Complex;
use realfft::RealFftPlanner;
use nih_plug::util::StftHelper;
use crate::dsp::engines::{BinParams, SpectralEngine, create_engine, EngineSelection};
use crate::bridge::SharedState;

pub const FFT_SIZE: usize = 2048;
pub const NUM_BINS: usize = FFT_SIZE / 2 + 1;
pub const OVERLAP: usize = 4; // 75% overlap → hop = 512

pub struct Pipeline {
    stft: StftHelper,
    fft_plan:  std::sync::Arc<dyn realfft::RealToComplex<f32>>,
    ifft_plan: std::sync::Arc<dyn realfft::ComplexToReal<f32>>,
    window:         Vec<f32>,
    spectrum_buf:    Vec<f32>,
    suppression_buf: Vec<f32>,
    channel_supp_buf: Vec<f32>,
    complex_buf:     Vec<Complex<f32>>,
    engine: Box<dyn SpectralEngine>,
    bp_threshold: Vec<f32>,
    bp_ratio:     Vec<f32>,
    bp_attack:    Vec<f32>,
    bp_release:   Vec<f32>,
    bp_knee:      Vec<f32>,
    bp_makeup:    Vec<f32>,
    bp_mix:       Vec<f32>,
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

        let complex_buf = fft_plan.make_output_vec();

        let mut engine = create_engine(EngineSelection::SpectralCompressor);
        engine.reset(sample_rate, FFT_SIZE);

        Self {
            stft: StftHelper::new(num_channels, FFT_SIZE, 0),
            fft_plan,
            ifft_plan,
            window,
            spectrum_buf:     vec![0.0; NUM_BINS],
            suppression_buf:  vec![0.0; NUM_BINS],
            channel_supp_buf: vec![0.0; NUM_BINS],
            complex_buf,
            engine,
            bp_threshold: vec![-20.0; NUM_BINS],
            bp_ratio:     vec![4.0;   NUM_BINS],
            bp_attack:    vec![10.0;  NUM_BINS],
            bp_release:   vec![80.0;  NUM_BINS],
            bp_knee:      vec![6.0;   NUM_BINS],
            bp_makeup:    vec![0.0;   NUM_BINS],
            bp_mix:       vec![1.0;   NUM_BINS],
            sample_rate,
        }
    }

    pub fn reset(&mut self, sample_rate: f32, num_channels: usize) {
        self.sample_rate = sample_rate;
        self.stft = StftHelper::new(num_channels, FFT_SIZE, 0);
        self.engine.reset(sample_rate, FFT_SIZE);
    }

    pub fn process(
        &mut self,
        buffer: &mut nih_plug::buffer::Buffer,
        shared: &mut SharedState,
        params: &crate::params::SpectralForgeParams,
    ) {
        // Read smoothed global parameter values (call next() once per block, not per sample)
        let attack_ms_base  = params.attack_ms.smoothed.next();
        let release_ms_base = params.release_ms.smoothed.next();
        let freq_scale      = params.freq_scale.smoothed.next();
        let input_gain_db   = params.input_gain.smoothed.next();
        let output_gain_db  = params.output_gain.smoothed.next();
        let global_mix      = params.mix.smoothed.next();

        // Read all 7 curve channels once.
        // Each read() requires &mut on its TbOutput; clone immediately so the
        // borrow ends before the next index is touched.
        let thresh_curve:  Vec<f32> = shared.curve_rx[0].read().clone();
        let ratio_curve:   Vec<f32> = shared.curve_rx[1].read().clone();
        let attack_curve:  Vec<f32> = shared.curve_rx[2].read().clone();
        let release_curve: Vec<f32> = shared.curve_rx[3].read().clone();
        let knee_curve:    Vec<f32> = shared.curve_rx[4].read().clone();
        let makeup_curve:  Vec<f32> = shared.curve_rx[5].read().clone();
        let mix_curve:     Vec<f32> = shared.curve_rx[6].read().clone();

        // Map to physical units, bin by bin
        let sample_rate = self.sample_rate;
        let num_bins = self.bp_threshold.len();

        for k in 0..num_bins {
            // Threshold: curve gain 1.0 → -20 dBFS base; range mapped to -60…0 dBFS.
            // flat curve (gain=1.0) → threshold = -20 dBFS
            // curve gain 0.0 → -60 dBFS (very low threshold = lots of compression)
            // curve gain 2.0 → 0 dBFS (threshold above max = no compression)
            // Linear interp: threshold_db = -20.0 + (gain - 1.0) * 20.0
            let t = thresh_curve.get(k).copied().unwrap_or(1.0);
            self.bp_threshold[k] = (-20.0 + (t - 1.0) * 20.0).clamp(-60.0, 0.0);

            // Ratio: curve gain 1.0 → ratio 1:1; gain 8.0 → ratio 8:1 (max)
            let r = ratio_curve.get(k).copied().unwrap_or(1.0);
            self.bp_ratio[k] = r.clamp(1.0, 20.0);

            // Frequency-dependent timing: lower frequencies get longer times
            let f_bin = (k as f32 * sample_rate / crate::dsp::pipeline::FFT_SIZE as f32).max(20.0);
            let scale = (1000.0_f32 / f_bin).powf(freq_scale * 0.5); // freq_scale ∈ [0,1]
            let atk_factor = attack_curve.get(k).copied().unwrap_or(1.0).max(0.01);
            let rel_factor = release_curve.get(k).copied().unwrap_or(1.0).max(0.01);
            self.bp_attack[k]  = (attack_ms_base  * scale * atk_factor).clamp(0.1, 500.0);
            self.bp_release[k] = (release_ms_base * scale * rel_factor).clamp(1.0, 2000.0);

            // Knee: curve gain 1.0 → 6 dB knee; range 0…24 dB
            let kn = knee_curve.get(k).copied().unwrap_or(1.0);
            self.bp_knee[k] = (kn * 6.0).clamp(0.0, 24.0);

            // Makeup: curve gain as dB (1.0 → 0 dB, >1 → positive makeup)
            let mk = makeup_curve.get(k).copied().unwrap_or(1.0);
            self.bp_makeup[k] = if mk > 1e-6 { 20.0 * mk.log10() } else { -96.0 };

            // Mix: curve gain 1.0 → full wet; scaled so 1.0 is 100%
            let mx = mix_curve.get(k).copied().unwrap_or(1.0);
            self.bp_mix[k] = (mx * global_mix).clamp(0.0, 1.0);
        }

        // Precompute input/output linear gains for capture into STFT closure
        let input_linear  = 10.0f32.powf(input_gain_db  / 20.0);
        let output_linear = 10.0f32.powf(output_gain_db / 20.0);

        // Reborrow fields as locals so the closure can capture them without
        // conflicting with the &mut self.stft borrow inside process_overlap_add.
        let fft_plan  = self.fft_plan.clone();
        let ifft_plan = self.ifft_plan.clone();
        let window         = &self.window;
        let engine            = &mut self.engine;
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
        let relative_mode = params.threshold_mode.value() == crate::params::ThresholdMode::Relative;
        let sample_rate  = self.sample_rate;
        // IFFT gives FFT_SIZE gain; Hann^2 OLA at 75% overlap gives 1.5 gain.
        // Combined normalization: 1 / (FFT_SIZE * 1.5) = 2 / (3 * FFT_SIZE)
        let norm = 2.0_f32 / (3.0 * FFT_SIZE as f32);

        self.stft.process_overlap_add(buffer, OVERLAP, |_channel, block| {
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
                threshold_db:  bp_threshold,
                ratio:         bp_ratio,
                attack_ms:     bp_attack,
                release_ms:    bp_release,
                knee_db:       bp_knee,
                makeup_db:     bp_makeup,
                mix:           bp_mix,
                relative_mode,
            };

            engine.process_bins(complex_buf, None, &params, sample_rate, channel_supp_buf);
            for k in 0..channel_supp_buf.len() {
                if channel_supp_buf[k] > suppression_buf[k] { suppression_buf[k] = channel_supp_buf[k]; }
            }

            ifft_plan.process(complex_buf, block).unwrap();

            // Synthesis window + IFFT normalization + output gain
            for (s, &w) in block.iter_mut().zip(window.iter()) {
                *s *= w * norm * output_linear;
            }
        });

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
