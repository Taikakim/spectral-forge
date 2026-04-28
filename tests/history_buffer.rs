use spectral_forge::params::HistoryBufferDepthChoice;

#[test]
fn history_depth_choice_default_is_4s() {
    assert_eq!(HistoryBufferDepthChoice::default(), HistoryBufferDepthChoice::Sec4);
}

#[test]
fn history_depth_choice_seconds_are_monotonic() {
    use HistoryBufferDepthChoice::*;
    assert!(Sec1.seconds() < Sec2.seconds());
    assert!(Sec2.seconds() < Sec4.seconds());
    assert!(Sec4.seconds() < Sec8.seconds());
    assert!(Sec8.seconds() < Sec16.seconds());
    assert_eq!(Sec1.seconds(), 1.0);
    assert_eq!(Sec16.seconds(), 16.0);
}

#[test]
fn history_depth_choice_max_frames_uses_hop_size() {
    use HistoryBufferDepthChoice::*;
    // sample_rate 48 kHz, hop = fft_size / 4
    // Sec1 at fft 2048 hop 512: frames = ceil(48000 / 512) = 94
    assert_eq!(Sec1.max_frames(48000.0, 2048), 94);
    // Sec4 at fft 2048 hop 512: ceil(192000 / 512) = 375
    assert_eq!(Sec4.max_frames(48000.0, 2048), 375);
    // Sec16 at fft 16384 hop 4096: ceil(768000 / 4096) = 188
    assert_eq!(Sec16.max_frames(48000.0, 16384), 188);
}

use num_complex::Complex;
use spectral_forge::dsp::history_buffer::HistoryBuffer;

fn frame_of(value: f32, num_bins: usize) -> Vec<Complex<f32>> {
    (0..num_bins).map(|_| Complex::new(value, 0.0)).collect()
}

#[test]
fn new_capacity_matches_constructor_args() {
    let h = HistoryBuffer::new(2, 100, 1025);
    assert_eq!(h.num_channels(), 2);
    assert_eq!(h.capacity_frames(), 100);
    assert_eq!(h.num_bins(), 1025);
    assert_eq!(h.frames_used(), 0);
}

#[test]
fn write_advances_frames_used_until_capacity() {
    let mut h = HistoryBuffer::new(1, 5, 4);
    for i in 0..3 {
        h.write_hop(0, &frame_of(i as f32, 4));
    }
    h.advance_after_all_channels_written();
    for i in 3..7 {
        h.write_hop(0, &frame_of(i as f32, 4));
        h.advance_after_all_channels_written();
    }
    // 8 writes total; capacity is 5; frames_used clamps at 5.
    assert_eq!(h.frames_used(), 5);
}

#[test]
fn read_frame_zero_returns_most_recent() {
    let mut h = HistoryBuffer::new(1, 4, 2);
    h.write_hop(0, &frame_of(10.0, 2));
    h.advance_after_all_channels_written();
    h.write_hop(0, &frame_of(20.0, 2));
    h.advance_after_all_channels_written();
    h.write_hop(0, &frame_of(30.0, 2));
    h.advance_after_all_channels_written();

    let frame = h.read_frame(0, 0).expect("most recent frame must exist");
    assert!((frame[0].re - 30.0).abs() < 1e-6);
    let frame = h.read_frame(0, 1).expect("one-back frame must exist");
    assert!((frame[0].re - 20.0).abs() < 1e-6);
    let frame = h.read_frame(0, 2).expect("two-back frame must exist");
    assert!((frame[0].re - 10.0).abs() < 1e-6);
    assert!(h.read_frame(0, 3).is_none(), "frame older than frames_used must be None");
}

#[test]
fn ring_wraps_after_capacity() {
    let mut h = HistoryBuffer::new(1, 3, 1);
    for i in 0..7u32 {
        h.write_hop(0, &frame_of(i as f32, 1));
        h.advance_after_all_channels_written();
    }
    // After 7 writes into a 3-frame ring, the most recent should be 6, then 5, 4.
    assert!((h.read_frame(0, 0).unwrap()[0].re - 6.0).abs() < 1e-6);
    assert!((h.read_frame(0, 1).unwrap()[0].re - 5.0).abs() < 1e-6);
    assert!((h.read_frame(0, 2).unwrap()[0].re - 4.0).abs() < 1e-6);
    assert!(h.read_frame(0, 3).is_none());
}

#[test]
fn read_fractional_lerps_between_frames() {
    let mut h = HistoryBuffer::new(1, 4, 1);
    h.write_hop(0, &frame_of(10.0, 1));
    h.advance_after_all_channels_written();
    h.write_hop(0, &frame_of(20.0, 1));
    h.advance_after_all_channels_written();

    let mut out = vec![Complex::new(0.0, 0.0); 1];
    let ok = h.read_fractional(0, 0.5, &mut out);
    assert!(ok);
    // age 0.5: 0.5 * frame[0]=20.0 + 0.5 * frame[1]=10.0 → 15.0
    assert!((out[0].re - 15.0).abs() < 1e-6);
}

#[test]
fn reset_zeroes_state() {
    let mut h = HistoryBuffer::new(1, 4, 2);
    h.write_hop(0, &frame_of(1.0, 2));
    h.advance_after_all_channels_written();
    h.reset();
    assert_eq!(h.frames_used(), 0);
    assert!(h.read_frame(0, 0).is_none());
}

#[test]
fn write_hop_panics_in_debug_when_buffer_size_mismatches() {
    let mut h = HistoryBuffer::new(1, 4, 8);
    let frame: Vec<Complex<f32>> = vec![Complex::new(0.0, 0.0); 7]; // wrong size
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        h.write_hop(0, &frame);
    }));
    // In debug builds, write_hop debug_asserts num_bins; release truncates silently.
    // The test only enforces the debug-mode panic so RT-safe release builds are unaffected.
    if cfg!(debug_assertions) {
        assert!(result.is_err(), "debug-mode write_hop must panic on wrong frame size");
    }
}

#[test]
fn summary_decay_estimate_long_ringing_bin_has_high_value() {
    // The regression reads frames at age 0..n where age 0 = most recent, age n-1 = oldest.
    // For a bin whose magnitude is currently at peak and decaying outward into older frames,
    // log10(mag) decreases as age increases → negative slope → non-zero decay_estimate.
    // Bin 0: very slow decay into the past (0.99^age), bin 1: fast decay (0.5^age).
    // We achieve this by writing magnitudes in REVERSE order (loudest last).
    let mut h = HistoryBuffer::new(1, 64, 2);
    let n_frames = 50usize;
    for i in 0..n_frames {
        // age after all writes = n_frames - 1 - i. We want mag = base^age, so mag = base^(n-1-i).
        let age = (n_frames - 1 - i) as f32;
        let bin0_mag = (0.99_f32).powf(age); // slow decay: nearly flat in analysis window
        let bin1_mag = (0.50_f32).powf(age); // fast decay: drops sharply with age
        h.write_hop(0, &[Complex::new(bin0_mag, 0.0), Complex::new(bin1_mag, 0.0)]);
        h.advance_after_all_channels_written();
    }
    let decay = h.summary_decay_estimate(0);
    assert!(decay[0] > decay[1],
        "slow-decay bin must have higher decay_estimate than fast-decay bin (got {:?} vs {:?})", decay[0], decay[1]);
}

#[test]
fn summary_rms_envelope_louder_bin_has_higher_rms() {
    let mut h = HistoryBuffer::new(1, 32, 2);
    for _ in 0..32 {
        h.write_hop(0, &[Complex::new(1.0, 0.0), Complex::new(0.1, 0.0)]);
        h.advance_after_all_channels_written();
    }
    let rms = h.summary_rms_envelope(0);
    assert!((rms[0] - 1.0).abs() < 1e-3, "loud bin RMS should be ~1.0, got {:?}", rms[0]);
    assert!((rms[1] - 0.1).abs() < 1e-3, "quiet bin RMS should be ~0.1, got {:?}", rms[1]);
}

#[test]
fn summary_if_stability_finite_when_no_phase_data() {
    let mut h = HistoryBuffer::new(1, 32, 2);
    for _ in 0..32 {
        h.write_hop(0, &[Complex::new(1.0, 0.0), Complex::new(0.5, 0.5)]);
        h.advance_after_all_channels_written();
    }
    let stab = h.summary_if_stability(0);
    for &v in stab.iter() {
        assert!(v.is_finite(), "if_stability must be finite");
        assert!(v >= 0.0 && v <= 1.0, "if_stability must be in [0, 1], got {:?}", v);
    }
}

#[test]
fn summary_caches_within_block_and_clears_on_request() {
    let mut h = HistoryBuffer::new(1, 32, 2);
    for _ in 0..32 {
        h.write_hop(0, &[Complex::new(1.0, 0.0), Complex::new(0.0, 0.0)]);
        h.advance_after_all_channels_written();
    }
    // First call computes; later calls reuse. Pointer equality is fine because
    // RefCell hands back a stable borrow until invalidated.
    let first = h.summary_rms_envelope(0).as_ptr();
    let second = h.summary_rms_envelope(0).as_ptr();
    assert_eq!(first, second);

    h.clear_summary_cache();
    // Force recompute: still same backing Vec (we recompute in place), but cache flag was reset.
    let third = h.summary_rms_envelope(0).as_ptr();
    assert_eq!(first, third, "summary buffer is reused — Vec is allocated once");
}

#[test]
fn summary_returns_zeros_for_invalid_channel() {
    let h = HistoryBuffer::new(1, 32, 2);
    let out = h.summary_decay_estimate(99);
    assert!(out.iter().all(|&v| v == 0.0));
}
