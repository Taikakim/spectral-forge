use num_complex::Complex;
use crate::dsp::engines::{BinParams, SpectralEngine, create_engine, EngineSelection};
use crate::params::{StereoLink, FxChannelTarget, EffectMode};

pub const MAX_SLOTS: usize = 8;

/// A single processing slot in the FxMatrix.
pub enum FxSlotKind {
    /// Dynamics: spectral compressor or contrast, using the existing 7-curve system.
    Dynamics {
        engine:   Box<dyn SpectralEngine>,
        engine_r: Box<dyn SpectralEngine>,
        contrast: Box<dyn SpectralEngine>,
    },
}

impl FxSlotKind {
    pub fn reset(&mut self, sample_rate: f32, fft_size: usize) {
        match self {
            Self::Dynamics { engine, engine_r, contrast } => {
                engine.reset(sample_rate, fft_size);
                engine_r.reset(sample_rate, fft_size);
                contrast.reset(sample_rate, fft_size);
            }
        }
    }

    /// Process `bins` in place, applying channel gating based on `target`.
    /// If the slot's target doesn't match the current channel/stereo_link, pass through.
    pub fn process_dynamics(
        &mut self,
        channel: usize,
        stereo_link: StereoLink,
        target: FxChannelTarget,
        bins: &mut [Complex<f32>],
        sidechain: Option<&[f32]>,
        params: &BinParams<'_>,
        effect_mode: EffectMode,
        sample_rate: f32,
        suppression_out: &mut [f32],
    ) {
        // Channel gating: if this slot targets Mid/Side but we're on the wrong channel, skip.
        let skip = match (target, stereo_link, channel) {
            (FxChannelTarget::Mid, StereoLink::MidSide, 1) => true,
            (FxChannelTarget::Side, StereoLink::MidSide, 0) => true,
            (FxChannelTarget::Mid, StereoLink::Linked, _) => true,
            (FxChannelTarget::Mid, StereoLink::Independent, _) => true,
            (FxChannelTarget::Side, StereoLink::Linked, _) => true,
            (FxChannelTarget::Side, StereoLink::Independent, _) => true,
            _ => false,
        };
        if skip {
            suppression_out.fill(0.0);
            return;
        }

        match self {
            Self::Dynamics { engine, engine_r, contrast } => {
                let eng: &mut Box<dyn SpectralEngine> = match stereo_link {
                    StereoLink::Independent if channel == 1 => engine_r,
                    _ => engine,
                };

                match effect_mode {
                    EffectMode::SpectralContrast => {
                        contrast.process_bins(bins, sidechain, params, sample_rate, suppression_out);
                    }
                    _ => {
                        eng.process_bins(bins, sidechain, params, sample_rate, suppression_out);
                    }
                }
            }
        }
    }
}

/// 8-slot spectral routing matrix.
pub struct FxMatrix {
    pub slots: [Option<FxSlotKind>; MAX_SLOTS],
    /// send[src][dst] = amplitude. src<dst: forward (current hop). src>dst: feedback (prev hop).
    pub send: [[f32; MAX_SLOTS]; MAX_SLOTS],
    slot_out_cur:  Vec<Vec<Complex<f32>>>,
    slot_out_prev: Vec<Vec<Complex<f32>>>,
    slot_supp:     Vec<Vec<f32>>,
}

impl FxMatrix {
    pub fn new(sample_rate: f32, fft_size: usize) -> Self {
        use crate::dsp::pipeline::NUM_BINS;

        let mut slots: [Option<FxSlotKind>; MAX_SLOTS] =
            std::array::from_fn(|_| None);

        let mut engine   = create_engine(EngineSelection::SpectralCompressor);
        let mut engine_r = create_engine(EngineSelection::SpectralCompressor);
        let mut contrast = create_engine(EngineSelection::SpectralContrast);
        engine.reset(sample_rate, fft_size);
        engine_r.reset(sample_rate, fft_size);
        contrast.reset(sample_rate, fft_size);

        slots[0] = Some(FxSlotKind::Dynamics { engine, engine_r, contrast });

        Self {
            slots,
            send: [[0.0f32; MAX_SLOTS]; MAX_SLOTS],
            slot_out_cur:  (0..MAX_SLOTS)
                .map(|_| vec![Complex::new(0.0f32, 0.0f32); NUM_BINS])
                .collect(),
            slot_out_prev: (0..MAX_SLOTS)
                .map(|_| vec![Complex::new(0.0f32, 0.0f32); NUM_BINS])
                .collect(),
            slot_supp:     (0..MAX_SLOTS)
                .map(|_| vec![0.0f32; NUM_BINS])
                .collect(),
        }
    }

    pub fn reset(&mut self, sample_rate: f32, fft_size: usize) {
        let num_bins = fft_size / 2 + 1;
        for slot in self.slots.iter_mut().flatten() {
            slot.reset(sample_rate, fft_size);
        }
        for buf in self.slot_out_cur.iter_mut()  { buf[..num_bins].fill(Complex::new(0.0, 0.0)); }
        for buf in self.slot_out_prev.iter_mut() { buf[..num_bins].fill(Complex::new(0.0, 0.0)); }
        for buf in self.slot_supp.iter_mut()     { buf[..num_bins].fill(0.0); }
    }

    /// Process one STFT hop through the slot chain.
    ///
    /// Slot 0 always receives `complex_buf` as main audio input. The last active slot's
    /// output is written back to `complex_buf`. If no slots are active, `complex_buf`
    /// passes through unchanged. The `send` matrix provides additional routing on top
    /// of this default serial flow.
    #[allow(clippy::too_many_arguments)]
    pub fn process_hop(
        &mut self,
        channel: usize,
        stereo_link: StereoLink,
        complex_buf: &mut [Complex<f32>],
        sidechain: Option<&[f32]>,
        params: &BinParams<'_>,
        effect_mode: EffectMode,
        target0: FxChannelTarget,
        sample_rate: f32,
        suppression_out: &mut [f32],
        num_bins: usize,
    ) {
        let mut last_active: Option<usize> = None;

        for i in 0..MAX_SLOTS {
            if self.slots[i].is_none() {
                continue;
            }

            // Assemble slot i's input into slot_out_cur[i].
            if i == 0 {
                // Slot 0: main audio input + any feedback sends from later slots (prev hop).
                self.slot_out_cur[0][..num_bins]
                    .copy_from_slice(&complex_buf[..num_bins]);
                for j in 1..MAX_SLOTS {
                    let amp = self.send[j][0];
                    if amp.abs() > 1e-6 {
                        let src = &self.slot_out_prev[j];
                        for k in 0..num_bins { self.slot_out_cur[0][k] += src[k] * amp; }
                    }
                }
            } else {
                // Slot i>0: assemble from forward sends (j<i, current hop) + feedback (j>i, prev hop).
                self.slot_out_cur[i][..num_bins].fill(Complex::new(0.0, 0.0));

                // Forward sends: use split_at to avoid aliasing slot_out_cur[j<i] with slot_out_cur[i].
                {
                    let (left, right) = self.slot_out_cur.split_at_mut(i);
                    for j in 0..i {
                        let amp = self.send[j][i];
                        if amp.abs() > 1e-6 {
                            for k in 0..num_bins { right[0][k] += left[j][k] * amp; }
                        }
                    }
                }

                // Feedback sends (j > i): previous hop output.
                for j in (i + 1)..MAX_SLOTS {
                    let amp = self.send[j][i];
                    if amp.abs() > 1e-6 {
                        let src = &self.slot_out_prev[j];
                        for k in 0..num_bins { self.slot_out_cur[i][k] += src[k] * amp; }
                    }
                }
            }

            // Process slot i in place on slot_out_cur[i].
            // Use .take() to satisfy the borrow checker: slot_out_cur[i] and slots[i]
            // are different fields but Rust can't see that through &mut self.
            let target = if i == 0 { target0 } else { FxChannelTarget::All };

            if let Some(mut slot) = self.slots[i].take() {
                slot.process_dynamics(
                    channel,
                    stereo_link,
                    target,
                    &mut self.slot_out_cur[i][..num_bins],
                    sidechain,
                    params,
                    effect_mode,
                    sample_rate,
                    &mut self.slot_supp[i][..num_bins],
                );
                self.slots[i] = Some(slot);
            }

            last_active = Some(i);
        }

        // Write last active slot's output to complex_buf; report slot 0 suppression.
        if let Some(i) = last_active {
            complex_buf[..num_bins].copy_from_slice(&self.slot_out_cur[i][..num_bins]);
            suppression_out[..num_bins].copy_from_slice(&self.slot_supp[0][..num_bins]);
        }
        // If no slots active: complex_buf passes through unchanged; suppression stays zero.

        // Rotate buffers: current hop becomes previous hop for next hop's feedback.
        std::mem::swap(&mut self.slot_out_cur, &mut self.slot_out_prev);
        for buf in self.slot_out_cur.iter_mut() {
            buf[..num_bins].fill(Complex::new(0.0, 0.0));
        }
    }
}
