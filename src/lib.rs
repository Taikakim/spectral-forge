pub mod dsp;
pub mod editor;
pub mod params;
pub mod bridge;

use nih_plug::prelude::*;
use params::SpectralForgeParams;
use std::sync::Arc;

pub struct SpectralForge {
    params: Arc<SpectralForgeParams>,
}

impl Default for SpectralForge {
    fn default() -> Self {
        Self { params: Arc::new(SpectralForgeParams::default()) }
    }
}

impl Plugin for SpectralForge {
    const NAME: &'static str = "Spectral Forge";
    const VENDOR: &'static str = "Kim";
    const URL: &'static str = "";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");
    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[
        AudioIOLayout {
            main_input_channels: NonZeroU32::new(2),
            main_output_channels: NonZeroU32::new(2),
            ..AudioIOLayout::const_default()
        },
    ];
    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> { self.params.clone() }

    fn process(
        &mut self, _buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers,
        _ctx: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
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
