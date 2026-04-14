pub mod dsp;
pub mod editor;
pub mod editor_ui;
pub mod params;
pub mod bridge;

use nih_plug::prelude::*;
use params::SpectralForgeParams;
use std::sync::Arc;

pub struct SpectralForge {
    params:   Arc<SpectralForgeParams>,
    pipeline: Option<dsp::pipeline::Pipeline>,
    shared:   Option<bridge::SharedState>,
    // Cloned Arc handles for the GUI — wired up in Default::default() so editor()
    // always has live handles regardless of whether the host calls it before initialize().
    gui_curve_tx:       Vec<Arc<parking_lot::Mutex<triple_buffer::Input<Vec<f32>>>>>,
    gui_sample_rate:    Option<Arc<bridge::AtomicF32>>,
    gui_num_bins:       usize,
    gui_spectrum_rx:    Option<Arc<parking_lot::Mutex<triple_buffer::Output<Vec<f32>>>>>,
    gui_suppression_rx: Option<Arc<parking_lot::Mutex<triple_buffer::Output<Vec<f32>>>>>,
    /// Liveness token: the editor holds a Weak clone of this. When the plugin
    /// is destroyed (this Arc drops), the editor detects it and closes itself.
    plugin_alive: Arc<()>,
    // Stored for reset()
    num_channels: usize,
    sample_rate:  f32,
}

impl Default for SpectralForge {
    fn default() -> Self {
        let dummy_sr = 44100.0;
        let num_bins = dsp::pipeline::FFT_SIZE / 2 + 1;
        let shared = bridge::SharedState::new(num_bins, dummy_sr);

        let gui_curve_tx       = shared.curve_tx.clone();
        let gui_sample_rate    = Some(shared.sample_rate.clone());
        let gui_num_bins       = shared.num_bins;
        let gui_spectrum_rx    = Some(shared.spectrum_rx.clone());
        let gui_suppression_rx = Some(shared.suppression_rx.clone());

        Self {
            params:   Arc::new(SpectralForgeParams::default()),
            pipeline: None,
            shared:   Some(shared),
            gui_curve_tx,
            gui_sample_rate,
            gui_num_bins,
            gui_spectrum_rx,
            gui_suppression_rx,
            plugin_alive: Arc::new(()),
            num_channels: 2,
            sample_rate:  dummy_sr,
        }
    }
}

impl Plugin for SpectralForge {
    const NAME: &'static str = "Spectral Forge";
    const VENDOR: &'static str = "Kim";
    const URL: &'static str = "";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");
    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[
        // Layout 0: stereo with sidechain
        AudioIOLayout {
            main_input_channels:  NonZeroU32::new(2),
            main_output_channels: NonZeroU32::new(2),
            aux_input_ports: &[new_nonzero_u32(2)],
            ..AudioIOLayout::const_default()
        },
        // Layout 1: stereo without sidechain
        AudioIOLayout {
            main_input_channels:  NonZeroU32::new(2),
            main_output_channels: NonZeroU32::new(2),
            ..AudioIOLayout::const_default()
        },
    ];
    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> { self.params.clone() }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        editor_ui::create_editor(
            self.params.clone(),
            self.gui_curve_tx.clone(),
            self.gui_sample_rate.clone(),
            self.gui_num_bins,
            self.gui_spectrum_rx.clone(),
            self.gui_suppression_rx.clone(),
            Arc::downgrade(&self.plugin_alive),
        )
    }

    fn initialize(
        &mut self,
        audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        context: &mut impl InitContext<Self>,
    ) -> bool {
        let sr = buffer_config.sample_rate;
        let num_ch = audio_io_layout.main_output_channels
            .map(|c| c.get() as usize).unwrap_or(2);
        self.num_channels = num_ch;
        self.sample_rate  = sr;
        let num_bins = dsp::pipeline::FFT_SIZE / 2 + 1;
        self.pipeline = Some(dsp::pipeline::Pipeline::new(sr, num_ch));
        context.set_latency_samples(dsp::pipeline::FFT_SIZE as u32);
        if let Some(ref sh) = self.shared {
            sh.sample_rate.store(sr);

            // Push initial per-bin curves computed from persisted curve_nodes so
            // restored sessions start with the correct gain values on the first block.
            let nodes_snapshot = *self.params.curve_nodes.lock();
            for (i, tx_arc) in sh.curve_tx.iter().enumerate() {
                let gains = crate::editor::curve::compute_curve_response(
                    &nodes_snapshot[i], num_bins, sr, dsp::pipeline::FFT_SIZE,
                );
                if let Some(mut tx) = tx_arc.try_lock() {
                    tx.input_buffer_mut().copy_from_slice(&gains);
                    tx.publish();
                }
            }
        }
        true
    }

    fn reset(&mut self) {
        if let Some(pipeline) = &mut self.pipeline {
            pipeline.reset(self.sample_rate, self.num_channels);
        }
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        aux: &mut AuxiliaryBuffers,
        _ctx: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        dsp::guard::flush_denormals();
        if let (Some(pipeline), Some(shared)) = (&mut self.pipeline, &mut self.shared) {
            pipeline.process(buffer, aux, shared, &self.params);
        }
        ProcessStatus::Normal
    }
}

impl ClapPlugin for SpectralForge {
    const CLAP_ID: &'static str = "com.spectral-forge.spectral-forge";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("Spectral compressor");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::AudioEffect, ClapFeature::Stereo,
    ];
}

nih_export_clap!(SpectralForge);
