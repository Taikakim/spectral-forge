//! MIDI held-note bookkeeping helpers.
//!
//! Pure functions: no heap allocation. Called from `SpectralForge::process()`
//! (`apply_note_on`/`apply_note_off`) and `Pipeline::reset` (`clear_midi_state`).

/// Number of distinct MIDI notes (0..=127).
pub const NUM_MIDI_NOTES: usize = 128;
/// Number of pitch classes (C through B).
pub const NUM_PITCH_CLASSES: usize = 12;

/// Mark `note` held; set its pitch-class bit.
/// Notes outside 0..128 are silently ignored.
#[inline]
pub fn apply_note_on(
    note:    u8,
    held:    &mut [bool; NUM_MIDI_NOTES],
    classes: &mut [bool; NUM_PITCH_CLASSES],
) {
    let n = note as usize;
    if n >= NUM_MIDI_NOTES { return; }
    held[n] = true;
    classes[n % NUM_PITCH_CLASSES] = true;
}

/// Release `note`. The pitch-class bit is recomputed; it stays set if any other
/// held note maps to the same class (e.g. C4 release while C5 still held).
/// Notes outside 0..128 are silently ignored.
#[inline]
pub fn apply_note_off(
    note:    u8,
    held:    &mut [bool; NUM_MIDI_NOTES],
    classes: &mut [bool; NUM_PITCH_CLASSES],
) {
    let n = note as usize;
    if n >= NUM_MIDI_NOTES { return; }
    // Stale note-off (note already released): nothing to update — the
    // pitch-class bit is already correct for the current held set, so we
    // skip the stride scan entirely.
    if !held[n] { return; }
    held[n] = false;
    let pc = n % NUM_PITCH_CLASSES;
    classes[pc] = (pc..NUM_MIDI_NOTES)
        .step_by(NUM_PITCH_CLASSES)
        .any(|k| held[k]);
}

/// Reset both arrays to all-false. Called on `Pipeline::reset()`.
#[inline]
pub fn clear_midi_state(
    held:    &mut [bool; NUM_MIDI_NOTES],
    classes: &mut [bool; NUM_PITCH_CLASSES],
) {
    *held    = [false; NUM_MIDI_NOTES];
    *classes = [false; NUM_PITCH_CLASSES];
}
