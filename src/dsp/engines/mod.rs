use num_complex::Complex;

/// Per-bin parameter values, physical units, pre-computed by pipeline.
pub struct BinParams<'a> {
    pub threshold_db: &'a [f32],  // dBFS per bin, e.g. -20.0
    pub ratio:        &'a [f32],  // ratio per bin, e.g. 4.0 = 4:1
    pub attack_ms:    &'a [f32],  // ms per bin, freq-scaled by pipeline
    pub release_ms:   &'a [f32],  // ms per bin, freq-scaled by pipeline
    pub knee_db:      &'a [f32],  // soft knee width in dB per bin
    pub makeup_db:    &'a [f32],  // makeup gain dB per bin
    pub mix:          &'a [f32],  // dry/wet per bin [0.0, 1.0]
    /// Spectral selectivity [0.0, 1.0].
    /// 0.0 = absolute compressor: any bin above threshold_db is compressed.
    /// 1.0 = fully selective: the effective threshold is raised to the local spectral
    ///       envelope level, so only bins that stick out above their neighbours are
    ///       compressed. Values between blend continuously between the two behaviours.
    pub sensitivity:  f32,
    pub auto_makeup:  bool,       // if true, add long-term average GR compensation per bin
    /// Log-frequency smoothing width in semitones (half-width each side).
    /// 0.0 = no spatial blur; engine blurs gr_db across adjacent bins on a log scale.
    pub smoothing_semitones: f32,
}

pub trait SpectralEngine: Send {
    /// Called at initialize() and on sample rate / FFT size change.
    /// Pre-allocate all heap state here — never in process_bins().
    fn reset(&mut self, sample_rate: f32, fft_size: usize);

    /// Called once per STFT hop on the audio thread.
    /// Must not allocate, lock, or perform I/O.
    /// Write |gain_reduction_db| per bin into suppression_out for GUI stalactites.
    ///
    /// Callers guarantee: `bins.len() == suppression_out.len() == fft_size/2+1`
    /// and `sidechain`, if present, has the same length.
    fn process_bins(
        &mut self,
        bins: &mut [Complex<f32>],
        sidechain: Option<&[f32]>,     // pre-smoothed sidechain magnitude per bin, or None
        params: &BinParams<'_>,
        sample_rate: f32,
        suppression_out: &mut [f32],
    );

    /// Tail after silence. Override for engines with extended tails (e.g. Freeze).
    fn tail_length(&self, fft_size: usize) -> u32 {
        debug_assert!(fft_size <= u32::MAX as usize);
        fft_size as u32
    }

    /// Zero all per-bin envelope and GR state without allocating.
    /// Called from SpectralModule::clear_state() on the audio thread.
    /// MUST NOT allocate, lock, or do I/O. Default is a no-op.
    fn clear_state(&mut self) {}

    fn name(&self) -> &'static str;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineSelection {
    SpectralCompressor,
    SpectralContrast,
}

pub fn create_engine(sel: EngineSelection) -> Box<dyn SpectralEngine> {
    match sel {
        EngineSelection::SpectralCompressor => {
            Box::new(spectral_compressor::SpectralCompressorEngine::new())
        }
        EngineSelection::SpectralContrast => {
            Box::new(spectral_contrast::SpectralContrastEngine::new())
        }
    }
}

pub mod spectral_compressor;
pub mod spectral_contrast;
