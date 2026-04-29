use spectral_forge::dsp::midi::{apply_note_off, apply_note_on, clear_midi_state};

#[test]
fn note_on_sets_held_note_and_pitch_class() {
    let mut held    = [false; 128];
    let mut classes = [false; 12];
    apply_note_on(60, &mut held, &mut classes);          // middle C
    assert!(held[60]);
    assert!(classes[0]);                                  // 60 % 12 == 0
    apply_note_on(62, &mut held, &mut classes);          // D
    assert!(held[62]);
    assert!(classes[2]);
}

#[test]
fn note_off_clears_held_note_and_class_when_no_other_holds_it() {
    let mut held    = [false; 128];
    let mut classes = [false; 12];
    apply_note_on(60, &mut held, &mut classes);
    apply_note_off(60, &mut held, &mut classes);
    assert!(!held[60]);
    assert!(!classes[0]);
}

#[test]
fn note_off_keeps_pitch_class_set_when_octave_held() {
    let mut held    = [false; 128];
    let mut classes = [false; 12];
    apply_note_on(60, &mut held, &mut classes);          // C4
    apply_note_on(72, &mut held, &mut classes);          // C5
    apply_note_off(60, &mut held, &mut classes);         // release C4
    assert!(!held[60]);
    assert!(held[72]);
    assert!(classes[0], "pitch class C must remain set while C5 is held");
}

#[test]
fn note_off_for_unheld_note_is_a_noop() {
    let mut held    = [false; 128];
    let mut classes = [false; 12];
    apply_note_off(45, &mut held, &mut classes);
    assert!(!held[45]);
    assert!(classes.iter().all(|&c| !c));
}

#[test]
fn out_of_range_note_does_not_panic() {
    let mut held    = [false; 128];
    let mut classes = [false; 12];
    apply_note_on(255, &mut held, &mut classes);          // out of MIDI range
    apply_note_off(255, &mut held, &mut classes);
    assert!(held.iter().all(|&h| !h));
    assert!(classes.iter().all(|&c| !c));
}

#[test]
fn clear_midi_state_zeroes_both_arrays() {
    let mut held    = [false; 128];
    let mut classes = [false; 12];
    apply_note_on(60, &mut held, &mut classes);
    apply_note_on(67, &mut held, &mut classes);
    apply_note_on(72, &mut held, &mut classes);
    clear_midi_state(&mut held, &mut classes);
    assert!(held.iter().all(|&h| !h),    "clear_midi_state must zero held[]");
    assert!(classes.iter().all(|&c| !c), "clear_midi_state must zero classes[]");
}
