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
    spectrum_buf:   Vec<f32>,
    suppression_buf: Vec<f32>,
    complex_buf:    Vec<Complex<f32>>,
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
            spectrum_buf:    vec![0.0; NUM_BINS],
            suppression_buf: vec![0.0; NUM_BINS],
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
    ) {
        // Pull latest curve data from GUI (lock-free triple-buffer reads)
        {
            let slices: [&mut Vec<f32>; 7] = [
                &mut self.bp_threshold, &mut self.bp_ratio,
                &mut self.bp_attack,   &mut self.bp_release,
                &mut self.bp_knee,     &mut self.bp_makeup,
                &mut self.bp_mix,
            ];
            for (dst, rx) in slices.into_iter().zip(shared.curve_rx.iter_mut()) {
                let latest = rx.read();
                if latest.len() == dst.len() {
                    dst.copy_from_slice(latest);
                }
            }
        }

        // Reborrow fields as locals so the closure can capture them without
        // conflicting with the &mut self.stft borrow inside process_overlap_add.
        let fft_plan  = self.fft_plan.clone();
        let ifft_plan = self.ifft_plan.clone();
        let window         = &self.window;
        let engine         = &mut self.engine;
        let complex_buf    = &mut self.complex_buf;
        let spectrum_buf   = &mut self.spectrum_buf;
        let suppression_buf = &mut self.suppression_buf;
        let bp_threshold = &self.bp_threshold;
        let bp_ratio     = &self.bp_ratio;
        let bp_attack    = &self.bp_attack;
        let bp_release   = &self.bp_release;
        let bp_knee      = &self.bp_knee;
        let bp_makeup    = &self.bp_makeup;
        let bp_mix       = &self.bp_mix;
        let sample_rate  = self.sample_rate;
        // IFFT gives FFT_SIZE gain; Hann^2 OLA at 75% overlap gives 1.5 gain.
        // Combined normalization: 1 / (FFT_SIZE * 1.5) = 2 / (3 * FFT_SIZE)
        let norm = 2.0_f32 / (3.0 * FFT_SIZE as f32);

        self.stft.process_overlap_add(buffer, OVERLAP, |_channel, block| {
            // Analysis window
            for (s, &w) in block.iter_mut().zip(window.iter()) {
                *s *= w;
            }

            // Guard: clamp NaN/Inf from broken drivers before FFT.
            crate::dsp::guard::sanitize(block);

            fft_plan.process(block, complex_buf).unwrap();

            // FIXME(multichannel): spectrum_buf and suppression_buf are overwritten
            // per channel; with stereo audio only the last channel's data reaches the
            // GUI. Fix in Task 13 (spectrum/suppression display) by accumulating a
            // peak-hold across channels before the publish call.
            for (i, c) in complex_buf.iter().enumerate() {
                spectrum_buf[i] = c.norm();
            }

            let params = BinParams {
                threshold_db: bp_threshold,
                ratio:        bp_ratio,
                attack_ms:    bp_attack,
                release_ms:   bp_release,
                knee_db:      bp_knee,
                makeup_db:    bp_makeup,
                mix:          bp_mix,
            };

            engine.process_bins(complex_buf, None, &params, sample_rate, suppression_buf);

            ifft_plan.process(complex_buf, block).unwrap();

            // Synthesis window + IFFT normalization
            for (s, &w) in block.iter_mut().zip(window.iter()) {
                *s *= w * norm;
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
