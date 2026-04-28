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
