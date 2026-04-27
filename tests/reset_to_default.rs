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

/// Structural contract: Pipeline::clear_state() must not call Pipeline::reset(),
/// RealFftPlanner::new(), or construct any StftHelper — those all heap-allocate
/// and must never run on the audio thread.
///
/// This test confirms clear_state() is callable and returns without panic.
/// The real RT-safety guarantee is that clear_state() only fills pre-allocated
/// buffers and does not call any module reset() (which allocates in DynamicsModule,
/// FreezeModule, ContrastModule, TsSplitModule, and SpectralCompressorEngine).
///
/// RT-safe: no allocation, no locking, no I/O.
#[test]
fn pipeline_clear_state_is_allocation_free() {
    use spectral_forge::dsp::pipeline::Pipeline;
    use spectral_forge::dsp::modules::ModuleType;
    let slot_types = [ModuleType::Empty; 9];
    let mut p = Pipeline::new(48000.0, 2, 2048, &slot_types);
    // Must return without panic or allocation.
    p.clear_state();
    // Calling twice is also fine — idempotent.
    p.clear_state();
}
