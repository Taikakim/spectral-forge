//! Forbids local display logic outside the cfg-driven path.
//! See docs/superpowers/specs/2026-05-05-graph-display-correctness.md §5.

use std::process::Command;

fn grep(pattern: &str, paths: &[&str]) -> Vec<String> {
    let mut args = vec!["-rEn", "--include=*.rs", pattern];
    args.extend_from_slice(paths);
    let out = Command::new("grep").args(&args).output().expect("grep");
    String::from_utf8_lossy(&out.stdout)
        .lines().map(str::to_string).collect()
}

#[test]
fn no_local_linear_or_log_to_y_in_modules() {
    let hits = grep(r"\b(linear_to_y|log_to_y)\b", &["src/dsp/modules/"]);
    assert!(hits.is_empty(),
        "DSP modules must not call linear_to_y/log_to_y directly:\n{}",
        hits.join("\n"));
}

#[test]
fn no_local_display_idx_match_in_modules() {
    // A `match curve_idx { 0..=N => display_value }` style branch in a DSP
    // module is a smell — display ranges belong in curve_config.rs.
    // Match on curve_idx for DSP behaviour (parameter selection) is fine;
    // we only flag arms that produce dB/ms/% literals.
    let hits = grep(r#"=>\s*\([^)]*\b(dB|dBFS|ms|%)\b"#, &["src/dsp/modules/"]);
    assert!(hits.is_empty(),
        "DSP modules must not encode display unit literals:\n{}",
        hits.join("\n"));
}

#[test]
fn no_y_label_string_outside_curve_config() {
    // Only scan .rs files (--include=*.rs in grep() above); tilde backups
    // are excluded automatically. params.rs uses with_unit(" dB/oct") for
    // nih-plug host-automation labels — that is NOT curve display logic, so
    // we filter it out along with the canonical homes curve_config.rs and
    // curve.rs.
    let hits: Vec<_> = grep(r#""\s*(dBFS|dB/oct)\s*""#, &["src/"])
        .into_iter()
        .filter(|l| !l.starts_with("src/editor/curve_config.rs:"))
        .filter(|l| !l.starts_with("src/editor/curve.rs:"))
        .filter(|l| !l.starts_with("src/params.rs:"))
        .collect();
    assert!(hits.is_empty(),
        "Y-axis unit literals must live in curve_config.rs:\n{}",
        hits.join("\n"));
}
