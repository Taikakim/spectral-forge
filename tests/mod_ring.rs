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
fn ring_toggles_are_disabled_until_bpm_sync_lands() {
    let s = ModRingState::default();
    assert!(!s.toggles_enabled());
}
