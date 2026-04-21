use spectral_forge::preset::{Preset, GuiState, sanitize_filename, PRESET_SCHEMA_VERSION};
use spectral_forge::params::SpectralForgeParams;
use std::collections::HashMap;
use std::fs;

#[test]
fn save_load_roundtrip_preserves_all_params() {
    let params = SpectralForgeParams::default();
    // Save defaults — no mutation needed; round-trip fidelity is what we're testing.
    let p1 = Preset::from_params("test".into(), &params, GuiState::default());

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.sfpreset");
    p1.save(&path).unwrap();

    let p2 = Preset::load(&path).unwrap();
    assert_eq!(p1.params.len(), p2.params.len());
    for (k, v) in &p1.params {
        let v2 = p2.params.get(k).unwrap_or_else(|| panic!("missing key {}", k));
        assert!(
            (v - v2).abs() < 1e-6,
            "mismatch on {}: {} vs {}",
            k,
            v,
            v2
        );
    }
}

#[test]
fn sanitize_filename_strips_bad_chars() {
    assert_eq!(sanitize_filename("hello/world"), "hello_world");
    assert_eq!(sanitize_filename("a:b?c"), "a_b_c");
    assert_eq!(sanitize_filename("  spaces  "), "spaces");
}

#[test]
fn scan_filters_by_schema_version() {
    let dir = tempfile::tempdir().unwrap();

    let good = Preset {
        schema_version: PRESET_SCHEMA_VERSION,
        plugin_version: "0".into(),
        name: "good".into(),
        params: HashMap::new(),
        gui: GuiState::default(),
    };
    good.save(&dir.path().join("good.sfpreset")).unwrap();

    let bad_json = serde_json::json!({
        "schema_version": 999,
        "plugin_version": "0",
        "name": "bad",
        "params": {},
        "gui": {
            "editing_slot": 0,
            "editing_curve": 0,
            "slot_module_types": [],
            "stereo_link": 0,
            "fft_size": 0
        }
    })
    .to_string();
    fs::write(dir.path().join("bad.sfpreset"), bad_json).unwrap();

    let list = Preset::scan_compatible(dir.path());
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].0, "good");
}
