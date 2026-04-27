use spectral_forge::dsp::amp_modes::{AmpMode, AmpCellParams};

#[test]
fn amp_mode_default_is_linear() {
    assert_eq!(AmpMode::default(), AmpMode::Linear);
}

#[test]
fn amp_cell_params_default_is_neutral() {
    let p = AmpCellParams::default();
    assert_eq!(p.amount,        1.0);
    assert_eq!(p.threshold,     0.5);
    assert_eq!(p.release_ms,   100.0);
    assert_eq!(p.slew_db_per_s, 60.0);
}
