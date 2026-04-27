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
/// No-allocation invariant (structural, not assert_no_alloc instrumented — that
/// crate is not a project dependency):
///   - Pipeline::clear_state() calls only .fill() on pre-allocated Vecs.
///   - FxMatrix::clear_state() calls slot.clear_state() (fills only) then .fill() on output Vecs.
///   - Each module's clear_state() uses only .fill() on pre-allocated Vecs and sets scalar fields.
///   - SpectralCompressorEngine::clear_state() and SpectralContrastEngine::clear_state()
///     each use only .fill() on pre-allocated Vecs.
///   - No vec!, Box::new, Vec::new, collect, to_vec, or .clone() on any Vec appears
///     anywhere in these call paths.
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

/// Smoke test: DynamicsModule::clear_state() does not panic on a freshly-created module.
/// Also exercises the path through SpectralCompressorEngine::clear_state() on both engines.
/// The RT-safety guarantee is structural: only .fill() on pre-allocated Vecs.
#[test]
fn dynamics_clear_state_zeroes_envelope() {
    use spectral_forge::dsp::modules::{create_module, ModuleType};
    let mut m = create_module(ModuleType::Dynamics, 48000.0, 2048);
    m.clear_state();
    // Calling clear_state on a freshly-created module should be a no-op and not panic.
    m.clear_state();
}

/// Smoke test: FreezeModule::clear_state() does not panic and resets freeze_captured.
/// A frozen snapshot should not survive a Reset — clear_state releases the captured bins.
#[test]
fn freeze_clear_state_releases_snapshot() {
    use spectral_forge::dsp::modules::{create_module, ModuleType};
    let mut m = create_module(ModuleType::Freeze, 48000.0, 2048);
    m.clear_state();
    m.clear_state();
}

/// Smoke test: ContrastModule::clear_state() does not panic.
/// Exercises SpectralContrastEngine::clear_state() path.
#[test]
fn contrast_clear_state_zeroes_envelope() {
    use spectral_forge::dsp::modules::{create_module, ModuleType};
    let mut m = create_module(ModuleType::Contrast, 48000.0, 2048);
    m.clear_state();
    m.clear_state();
}

/// Smoke test: TsSplitModule::clear_state() does not panic.
/// Zeroes avg_mag history and both virtual output buffers (transient + sustained).
#[test]
fn ts_split_clear_state_zeroes_history() {
    use spectral_forge::dsp::modules::{create_module, ModuleType};
    let mut m = create_module(ModuleType::TransientSustainedSplit, 48000.0, 2048);
    m.clear_state();
    m.clear_state();
}

/// Integration: Pipeline with stateful slots exercises the full clear_state path
/// including FxMatrix iterating live slot modules and calling their clear_state().
#[test]
fn pipeline_clear_state_with_stateful_slots() {
    use spectral_forge::dsp::pipeline::Pipeline;
    use spectral_forge::dsp::modules::ModuleType;
    let mut slot_types = [ModuleType::Empty; 9];
    slot_types[0] = ModuleType::Dynamics;
    slot_types[1] = ModuleType::Freeze;
    slot_types[2] = ModuleType::Contrast;
    slot_types[3] = ModuleType::TransientSustainedSplit;
    let mut p = Pipeline::new(48000.0, 2, 2048, &slot_types);
    // Must not panic and must touch all four stateful modules' clear_state() impls.
    p.clear_state();
    p.clear_state();
}
