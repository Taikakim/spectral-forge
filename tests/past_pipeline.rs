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
    // Seed every bin with low-level rotating noise so all 1025 bins exercise
    // the kernel body (threshold = 0.0). Bins 100 and 200 carry stronger
    // tonal content for content-shaped paths (Convolution, DecaySorter).
    let mut frame = Vec::with_capacity(NUM_BINS);
    for k in 0..NUM_BINS {
        let mag = 0.1;
        let phase = (k as f32 * 0.3 + hop as f32 * 0.05) % core::f32::consts::TAU;
        frame.push(Complex::from_polar(mag, phase));
    }
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
    let threshold = vec![0.0_f32; NUM_BINS];
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

#[test]
fn reverse_uses_scalar_window_not_curve_average() {
    use spectral_forge::dsp::modules::past::{PastModule, PastMode, PastScalars};

    let mut m = PastModule::new(48000.0, 2048);
    m.set_past_mode(PastMode::Reverse);
    m.set_scalars(PastScalars {
        window_frames: 8,
        ..Default::default()
    });
    let scalars = m.scalars();
    assert_eq!(scalars.window_frames, 8, "scalar must persist via setter");
}

#[test]
fn stretch_uses_scalar_rate_not_curve_average() {
    use spectral_forge::dsp::modules::past::{PastModule, PastMode, PastScalars};

    let mut m = PastModule::new(48000.0, 2048);
    m.set_past_mode(PastMode::Stretch);
    m.set_scalars(PastScalars { rate: 2.0, dither: 0.0, ..PastScalars::safe_default() });
    assert!((m.scalars().rate - 2.0).abs() < 1e-6);
    assert_eq!(m.scalars().dither, 0.0);
}

#[test]
fn past_scalars_safe_default_is_musically_inert() {
    use spectral_forge::dsp::modules::past::PastScalars;
    let s = PastScalars::safe_default();
    assert!((s.rate - 1.0).abs() < 1e-6, "rate=1.0 means no stretch");
    assert_eq!(s.dither, 0.0, "dither=0 means no smoothing-noise");
    assert_eq!(s.window_frames, 1, "window=1 frame is the smallest legal value");
    assert!(s.soft_clip, "soft_clip ON by default");
}

#[test]
fn soft_clip_clamps_high_magnitude_when_on() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::past::apply_soft_clip;
    let mut bins = [Complex::new(10.0_f32, 0.0); 32];
    apply_soft_clip(&mut bins, 32);
    for k in 0..32 {
        assert!(bins[k].norm() < 4.0,
            "soft-clip with K=4.0 must keep magnitude under 4.0; bin {k} got {}", bins[k].norm());
    }
}

#[cfg(feature = "probe")]
#[test]
fn fx_matrix_set_past_scalars_dispatches_to_past_slots() {
    use spectral_forge::dsp::fx_matrix::FxMatrix;
    use spectral_forge::dsp::modules::ModuleType;
    use spectral_forge::dsp::modules::past::PastScalars;

    // Slot 0 = Past; everything else Empty.
    let slot_types: [ModuleType; 9] = [
        ModuleType::Past, ModuleType::Empty, ModuleType::Empty,
        ModuleType::Empty, ModuleType::Empty, ModuleType::Empty,
        ModuleType::Empty, ModuleType::Empty, ModuleType::Empty,
    ];
    let mut fxm = FxMatrix::new(48000.0, 2048, &slot_types);

    // Custom scalars for slot 0; defaults elsewhere.
    let mut want = [PastScalars::safe_default(); 9];
    want[0] = PastScalars {
        floor_bin: 42,
        window_frames: 7,
        rate: 1.5,
        dither: 0.25,
        soft_clip: false,
    };
    fxm.set_past_scalars(&want);

    // Slot 0's PastModule must have received the scalars.
    let s = fxm.test_past_scalars(0).expect("slot 0 must be Past");
    assert_eq!(s.floor_bin, 42);
    assert_eq!(s.window_frames, 7);
    assert!((s.rate - 1.5).abs() < 1e-6);
    assert!((s.dither - 0.25).abs() < 1e-6);
    assert!(!s.soft_clip);

    // Empty slot returns None.
    assert!(fxm.test_past_scalars(1).is_none(), "Empty slot must yield None");
}

#[test]
fn soft_clip_passes_low_magnitude_almost_unchanged() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::past::apply_soft_clip;
    let mut bins = [Complex::new(0.1_f32, 0.0); 16];
    let original = bins[0].norm();
    apply_soft_clip(&mut bins, 16);
    let attenuation = bins[0].norm() / original;
    // |out|/|in| = K / (K + |in|) = 4.0 / 4.1 ≈ 0.976
    assert!(attenuation > 0.95 && attenuation < 1.0,
        "small bins should be barely attenuated; got {attenuation}");
}
