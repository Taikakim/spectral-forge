use spectral_forge::bridge::SharedState;
use std::sync::atomic::Ordering;

#[test]
fn reset_requested_flag_round_trips() {
    let s = SharedState::new(2048, 48000.0);
    assert!(!s.reset_requested.load(Ordering::Acquire));
    s.reset_requested.store(true, Ordering::Release);
    assert!(s.reset_requested.load(Ordering::Acquire));
    // Audio side resets the flag after handling.
    s.reset_requested.store(false, Ordering::Release);
    assert!(!s.reset_requested.load(Ordering::Acquire));
}
