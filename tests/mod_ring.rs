use spectral_forge::editor::mod_ring::{ModRingState, ModRingToggle};

#[test]
fn ring_state_starts_with_all_toggles_off() {
    let s = ModRingState::default();
    assert!(!s.is_set(ModRingToggle::SampleHold));
    assert!(!s.is_set(ModRingToggle::Sync16));
    assert!(!s.is_set(ModRingToggle::Legato));
}

#[test]
fn ring_state_toggle_round_trip() {
    let mut s = ModRingState::default();
    s.toggle(ModRingToggle::SampleHold);
    assert!(s.is_set(ModRingToggle::SampleHold));
    s.toggle(ModRingToggle::SampleHold);
    assert!(!s.is_set(ModRingToggle::SampleHold));
}

#[test]
fn ring_toggles_are_enabled_after_phase_6_7() {
    // Phase 6.7 activates the ring toggles unconditionally (the audio thread
    // degrades gracefully when no transport is running). This replaces the Phase
    // 1 scaffold test that asserted `false` while the infrastructure was pending.
    let s = ModRingState::default();
    assert!(s.toggles_enabled());
}

#[test]
fn ring_state_all_toggles_round_trip_to_zero() {
    let mut s = ModRingState::default();
    let all = [
        ModRingToggle::SampleHold,
        ModRingToggle::Sync16,
        ModRingToggle::Legato,
    ];
    for &t in &all { s.toggle(t); }
    for &t in &all { assert!(s.is_set(t), "{t:?} should be on"); }
    for &t in &all { s.toggle(t); }
    for &t in &all { assert!(!s.is_set(t), "{t:?} should be off after second toggle"); }
}
