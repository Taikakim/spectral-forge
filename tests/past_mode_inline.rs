//! Inline PAST mode UI placeholder regression. Visual UX verified via
//! manual smoke test; this pins the public API surface.

use spectral_forge::editor::past_popup::{mode_label, MODES, SORT_KEYS};
use spectral_forge::dsp::modules::past::PastMode;

#[test]
fn past_mode_label_set_intact() {
    assert_eq!(mode_label(PastMode::Granular),    "Granular Window");
    assert_eq!(mode_label(PastMode::DecaySorter), "Decay Sorter");
    assert_eq!(mode_label(PastMode::Convolution), "Spectral Convolution");
    assert_eq!(mode_label(PastMode::Reverse),     "Reverse");
    assert_eq!(mode_label(PastMode::Stretch),     "Stretch");
}

#[test]
fn past_modes_array_has_5_entries() {
    assert_eq!(MODES.len(), 5);
}

#[test]
fn past_sort_keys_array_present() {
    assert!(SORT_KEYS.len() >= 2,
        "DecaySorter sub-picker still needs sort key options");
}
