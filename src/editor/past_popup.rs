use crate::dsp::modules::past::{PastMode, SortKey};

pub const MODES: &[(PastMode, &str, &str)] = &[
    (PastMode::Granular,    "Granular Window",     "Selective time-windowed freeze of stable bins"),
    (PastMode::DecaySorter, "Decay Sorter",        "Temporal reconstruction via summary-stat sorting"),
    (PastMode::Convolution, "Spectral Convolution","Per-bin self-resonance — convolve current with past"),
    (PastMode::Reverse,     "Reverse",             "Backward read of the history buffer"),
    (PastMode::Stretch,     "Stretch",             "Phase-coherent variable-rate playback (0.25\u{00d7} \u{2013} 4\u{00d7})"),
];

pub const SORT_KEYS: &[(SortKey, &str)] = &[
    (SortKey::Decay,     "Decay (ring time)"),
    (SortKey::Stability, "Stability (IF)"),
    (SortKey::Area,      "Area (RMS)"),
];

pub fn mode_label(mode: PastMode) -> &'static str {
    for &(m, label, _) in MODES {
        if m == mode { return label; }
    }
    "Unknown"
}
