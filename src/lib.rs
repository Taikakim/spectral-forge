pub mod dsp;
pub mod editor;
pub mod editor_ui;
pub mod param_ids;
pub mod params;
pub mod bridge;
pub mod presets;

use nih_plug::prelude::*;
use params::SpectralForgeParams;
use std::sync::Arc;

pub struct SpectralForge {
    params:   Arc<SpectralForgeParams>,
    pipeline: Option<dsp::pipeline::Pipeline>,
    shared:   Option<bridge::SharedState>,
    // Cloned Arc handles for the GUI — wired up in Default::default() so editor()
    // always has live handles regardless of whether the host calls it before initialize().
    /// gui_curve_tx[slot][curve]: 9 slots × 7 curves.
    gui_curve_tx:          Vec<Vec<Arc<parking_lot::Mutex<triple_buffer::Input<Vec<f32>>>>>>,
    gui_sample_rate:       Option<Arc<bridge::AtomicF32>>,
    gui_fft_size:          Arc<std::sync::atomic::AtomicUsize>,
    gui_spectrum_rx:       Option<Arc<parking_lot::Mutex<triple_buffer::Output<Vec<f32>>>>>,
    gui_suppression_rx:    Option<Arc<parking_lot::Mutex<triple_buffer::Output<Vec<f32>>>>>,
    gui_sidechain_active: Option<[Arc<std::sync::atomic::AtomicBool>; 4]>,
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
        let default_fft_size = dsp::pipeline::FFT_SIZE;
        let shared = bridge::SharedState::new(default_fft_size, dummy_sr);

        let gui_curve_tx         = shared.curve_tx.clone();
        let gui_sample_rate      = Some(shared.sample_rate.clone());
        let gui_fft_size         = shared.fft_size.clone();
        let gui_spectrum_rx      = Some(shared.spectrum_rx.clone());
        let gui_suppression_rx   = Some(shared.suppression_rx.clone());
        let gui_sidechain_active = Some(std::array::from_fn::<_, 4, _>(|i| {
            shared.sidechain_active[i].clone()
        }));

        Self {
            params:   Arc::new(SpectralForgeParams::default()),
            pipeline: None,
            shared:   Some(shared),
            gui_curve_tx,
            gui_sample_rate,
            gui_fft_size,
            gui_spectrum_rx,
            gui_suppression_rx,
            gui_sidechain_active,
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
        // Layout 0: stereo with 4 aux sidechain inputs
        AudioIOLayout {
            main_input_channels:  NonZeroU32::new(2),
            main_output_channels: NonZeroU32::new(2),
            aux_input_ports: &[new_nonzero_u32(2), new_nonzero_u32(2), new_nonzero_u32(2), new_nonzero_u32(2)],
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
            self.gui_fft_size.clone(),
            self.gui_spectrum_rx.clone(),
            self.gui_suppression_rx.clone(),
            self.gui_sidechain_active.clone(),
            Arc::downgrade(&self.plugin_alive),
        )
    }

    fn initialize(
        &mut self,
        audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        context: &mut impl InitContext<Self>,
    ) -> bool {
        use std::sync::atomic::Ordering;
        let sr = buffer_config.sample_rate;
        let num_ch = audio_io_layout.main_output_channels
            .map(|c| c.get() as usize).unwrap_or(2);
        self.num_channels = num_ch;
        self.sample_rate  = sr;

        let fft_size = params::fft_size_from_choice(self.params.fft_size.value());
        let max_num_bins = dsp::pipeline::MAX_NUM_BINS;

        let slot_types = *self.params.slot_module_types.lock();
        self.pipeline = Some(dsp::pipeline::Pipeline::new(sr, num_ch, fft_size, &slot_types));
        context.set_latency_samples(fft_size as u32);

        if let Some(ref sh) = self.shared {
            sh.sample_rate.store(sr);
            sh.fft_size.store(fft_size, Ordering::Relaxed);

            // The editing slot is shown in the GUI using `curve_nodes` (the legacy 7-curve
            // store). Publish from there so the displayed curve response matches the DSP on
            // first load — otherwise the GUI might show a 2:1 ratio while the DSP applies 1:1.
            let editing_slot = (*self.params.editing_slot.lock() as usize).min(8);
            {
                let legacy = self.params.curve_nodes.lock();
                for c in 0..7 {
                    let gains = crate::editor::curve::compute_curve_response(
                        &legacy[c], max_num_bins, sr, fft_size,
                    );
                    if let Some(mut tx) = self.gui_curve_tx[editing_slot][c].try_lock() {
                        tx.input_buffer_mut().copy_from_slice(&gains);
                        tx.publish();
                    }
                }
            }
            // All other slots: publish from their persisted slot_curve_nodes.
            {
                let nodes = self.params.slot_curve_nodes.lock();
                for s in 0..9 {
                    if s == editing_slot { continue; }
                    for c in 0..7 {
                        let gains = crate::editor::curve::compute_curve_response(
                            &nodes[s][c], max_num_bins, sr, fft_size,
                        );
                        if let Some(mut tx) = self.gui_curve_tx[s][c].try_lock() {
                            tx.input_buffer_mut().copy_from_slice(&gains);
                            tx.publish();
                        }
                    }
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

impl Vst3Plugin for SpectralForge {
    // Every VST3 plugin requires a globally unique 16-byte ID.
    // This is exactly 16 characters long.
    const VST3_CLASS_ID: [u8; 16] = *b"TaikakimSpcForge";

    // This tells the DAW what folder to put your plugin in.
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[
        Vst3SubCategory::Fx,
        Vst3SubCategory::Dynamics,
    ];
}

nih_export_clap!(SpectralForge);
nih_export_vst3!(SpectralForge);
