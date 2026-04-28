//! End-to-end smoke: drive 200 hops of synthetic spectra through a PastModule
//! plus HistoryBuffer for every (mode × sort_key) combination, asserting
//! outputs stay finite and non-explosive.

use num_complex::Complex;
use spectral_forge::dsp::history_buffer::HistoryBuffer;
use spectral_forge::dsp::modules::past::{PastModule, PastMode, SortKey};
use spectral_forge::dsp::modules::{ModuleContext, SpectralModule};
use spectral_forge::params::{FxChannelTarget, StereoLink};

const NUM_BINS: usize = 1025;

fn synth_input(hop: usize) -> Vec<Complex<f32>> {
    let mut frame = vec![Complex::new(0.0, 0.0); NUM_BINS];
    frame[100] = Complex::from_polar(1.0, hop as f32 * 0.1);
    frame[200] = Complex::from_polar(0.5 * (1.0 + (hop as f32 / 100.0).sin()), 0.0);
    frame
}

fn drive(mode: PastMode, sort_key: SortKey) -> Vec<Vec<Complex<f32>>> {
    let mut h = HistoryBuffer::new(2, 100, NUM_BINS);
    let mut m = PastModule::new(48000.0, 2048);
    m.set_mode(mode);
    m.set_sort_key(sort_key);

    let amount    = vec![1.0_f32; NUM_BINS];
    let time      = vec![0.5_f32; NUM_BINS];
    let threshold = vec![0.05_f32; NUM_BINS];
    let spread    = vec![0.0_f32; NUM_BINS];
    let mix       = vec![0.5_f32; NUM_BINS];
    let curves: Vec<&[f32]> = vec![&amount, &time, &threshold, &spread, &mix];

    let mut outputs: Vec<Vec<Complex<f32>>> = Vec::new();
    for hop in 0..200 {
        let frame = synth_input(hop);
        h.write_hop(0, &frame);
        h.write_hop(1, &frame);
        h.advance_after_all_channels_written();

        let mut bins = frame.clone();
        let mut supp = vec![0.0_f32; NUM_BINS];
        let mut ctx = ModuleContext::new(48000.0, 2048, NUM_BINS, 10.0, 100.0, 1.0, 0.5, false, false);
        ctx.history = Some(&h);
        m.process(0, StereoLink::Linked, FxChannelTarget::All,
                  &mut bins, None, &curves, &mut supp, None, &ctx);
        outputs.push(bins);
    }
    outputs
}

#[test]
fn all_past_modes_stay_finite_and_bounded() {
    for mode in [PastMode::Granular, PastMode::DecaySorter, PastMode::Convolution,
                 PastMode::Reverse, PastMode::Stretch] {
        for sort_key in [SortKey::Decay, SortKey::Stability, SortKey::Area] {
            let frames = drive(mode, sort_key);
            for (hop, frame) in frames.iter().enumerate() {
                for (k, c) in frame.iter().enumerate() {
                    assert!(c.re.is_finite() && c.im.is_finite(),
                        "non-finite output at mode {:?} sort {:?} hop {} bin {}",
                        mode, sort_key, hop, k);
                    assert!(c.norm() < 100.0,
                        "explosive output ({}) at mode {:?} hop {} bin {}", c.norm(), mode, hop, k);
                }
            }
        }
    }
}
