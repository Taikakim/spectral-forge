//! Per-mode CurveLayout regression for Past. See
//! docs/superpowers/specs/2026-05-04-past-module-ux-design.md §1 + §4.

use spectral_forge::dsp::modules::past::{active_layout, PastMode};

#[test]
fn past_active_layout_granular_visible_curves() {
    let layout = active_layout(PastMode::Granular as u8);
    assert_eq!(layout.active, &[0u8, 1, 2, 3, 4], "Granular shows all 5 curves");
    // Age and Smear are mode-specific labels
    let mut got = layout.label_overrides.iter().copied().collect::<Vec<_>>();
    got.sort();
    let mut want: Vec<(u8, &'static str)> = vec![(1, "Age"), (3, "Smear")];
    want.sort();
    assert_eq!(got, want);
}

#[test]
fn past_active_layout_decay_sorter_visible_curves() {
    let layout = active_layout(PastMode::DecaySorter as u8);
    assert_eq!(layout.active, &[0u8, 2, 4]);
    assert!(layout.label_overrides.is_empty());
}

#[test]
fn past_active_layout_convolution_visible_curves() {
    let layout = active_layout(PastMode::Convolution as u8);
    assert_eq!(layout.active, &[0u8, 1, 2, 4]);
    let mut got = layout.label_overrides.iter().copied().collect::<Vec<_>>();
    got.sort();
    let mut want: Vec<(u8, &'static str)> = vec![(1, "Delay")];
    want.sort();
    assert_eq!(got, want);
}

#[test]
fn past_active_layout_reverse_visible_curves() {
    let layout = active_layout(PastMode::Reverse as u8);
    assert_eq!(layout.active, &[0u8, 2, 4]);
    assert!(layout.label_overrides.is_empty());
}

#[test]
fn past_active_layout_stretch_visible_curves() {
    let layout = active_layout(PastMode::Stretch as u8);
    assert_eq!(layout.active, &[0u8, 4]);
    assert!(layout.label_overrides.is_empty());
}

#[test]
fn past_active_layout_help_for_non_empty_for_every_active_curve() {
    for mode in [
        PastMode::Granular, PastMode::DecaySorter, PastMode::Convolution,
        PastMode::Reverse, PastMode::Stretch,
    ] {
        let layout = active_layout(mode as u8);
        for &curve_idx in layout.active {
            let help = (layout.help_for)(curve_idx);
            assert!(
                !help.is_empty(),
                "help_for({curve_idx}) is empty for mode {mode:?}",
            );
        }
    }
}
