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
