//! Master soft clipper tests. See spec §4 of
//! 2026-05-06-stabilization-sweep.md (toggle) plus the threshold-knee
//! reshape that replaced the original always-on K/(K+|bin|) algorithm.

use spectral_forge::params::SpectralForgeParams;
use spectral_forge::dsp::soft_clip::apply_soft_clip;
use nih_plug::prelude::Param;
use num_complex::Complex;

#[test]
fn master_clip_enabled_default_true() {
    let p = SpectralForgeParams::default();
    assert!(p.master_clip_enabled.value(),
        "master_clip_enabled should default to true (safety-on-by-default)");
}

#[test]
fn master_clip_threshold_db_default_zero() {
    let p = SpectralForgeParams::default();
    assert!((p.master_clip_threshold_db.value() - 0.0).abs() < 1e-6,
        "master_clip_threshold_db should default to 0 dB (least clipping)");
}

#[test]
fn soft_clip_silent_input_produces_silent_output() {
    let mut bins = vec![Complex::new(0.0, 0.0); 1025];
    apply_soft_clip(&mut bins, 1025, 0.0);
    for c in &bins {
        assert!(c.re.abs() < 1e-9 && c.im.abs() < 1e-9,
            "silent input should yield silent output, got {:?}", c);
    }
}

#[test]
fn soft_clip_below_threshold_is_bit_exact_passthrough() {
    // At threshold_db = 0 (t_lin = 1.0), bins with mag <= 1.0 must pass
    // through unchanged — that's the whole point of the threshold-knee
    // reshape. The previous K/(K+|bin|) algorithm scaled every bin even at
    // tiny magnitudes; this test pins the new behaviour.
    let mut bins = vec![Complex::new(0.5, 0.0); 1025];
    let original = bins.clone();
    apply_soft_clip(&mut bins, 1025, 0.0);
    for (a, b) in bins.iter().zip(original.iter()) {
        assert!((a.re - b.re).abs() < 1e-9 && (a.im - b.im).abs() < 1e-9,
            "below-threshold bins must be bit-exact passthrough, got {a:?} from {b:?}");
    }
}

#[test]
fn soft_clip_above_threshold_no_nan_bounded() {
    // threshold_db = 0 → t_lin = 1, ceiling = 4. Hot bin (mag=8) should be
    // clamped well below the asymptote.
    let mut bins = vec![Complex::new(8.0, 0.0); 1025];
    apply_soft_clip(&mut bins, 1025, 0.0);
    for c in &bins {
        assert!(c.re.is_finite() && c.im.is_finite(),
            "no NaN/Inf from soft clip");
        assert!(c.norm() < 4.0,
            "soft clip should bound magnitude under the ceiling (4× threshold), got {}", c.norm());
    }
}

#[test]
fn soft_clip_threshold_continuity_at_knee() {
    // At mag = t_lin exactly, output should equal t_lin (no jump).
    // threshold_db = -6 → t_lin = 0.501.
    let t_db = -6.0;
    let t_lin = 10f32.powf(t_db / 20.0);
    let mut bins = vec![Complex::new(t_lin, 0.0); 16];
    apply_soft_clip(&mut bins, 16, t_db);
    for c in &bins {
        assert!((c.norm() - t_lin).abs() < 1e-6,
            "expected continuity at knee: in {t_lin} out {}", c.norm());
    }
}

#[test]
fn master_module_applies_soft_clip_when_enabled() {
    use spectral_forge::dsp::modules::master::MasterModule;
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let mut master = MasterModule::new(true);
    let mut bins = vec![Complex::new(8.0, 0.0); 1025];
    let mut supp = vec![0.0_f32; 1025];
    let ctx = ModuleContext::new(48_000.0, 2048, 1025, 10.0, 100.0, 1.0, 0.5, false, false);

    master.process(0, StereoLink::Linked, FxChannelTarget::All,
        &mut bins, None, &[], &mut supp, None, &ctx);

    // Default threshold = 0 dB → ceiling 4. mag=8 should be soft-clipped
    // toward the ceiling.
    for c in &bins {
        assert!(c.norm() < 4.0, "expected clamp under ceiling, got {}", c.norm());
    }
}

#[test]
fn master_module_passthrough_when_disabled() {
    use spectral_forge::dsp::modules::master::MasterModule;
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let mut master = MasterModule::new(false);
    let mut bins = vec![Complex::new(8.0, 0.0); 1025];
    let mut supp = vec![0.0_f32; 1025];
    let ctx = ModuleContext::new(48_000.0, 2048, 1025, 10.0, 100.0, 1.0, 0.5, false, false);

    master.process(0, StereoLink::Linked, FxChannelTarget::All,
        &mut bins, None, &[], &mut supp, None, &ctx);

    for c in &bins {
        assert!((c.re - 8.0).abs() < 1e-6 && c.im.abs() < 1e-6);
    }
}

#[test]
fn master_module_silent_in_silent_out_regardless_of_clip() {
    use spectral_forge::dsp::modules::master::MasterModule;
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    for enabled in [true, false] {
        let mut master = MasterModule::new(enabled);
        let mut bins = vec![Complex::new(0.0, 0.0); 1025];
        let mut supp = vec![0.0_f32; 1025];
        let ctx = ModuleContext::new(48_000.0, 2048, 1025, 10.0, 100.0, 1.0, 0.5, false, false);

        master.process(0, StereoLink::Linked, FxChannelTarget::All,
            &mut bins, None, &[], &mut supp, None, &ctx);

        for c in &bins {
            assert!(c.re.abs() < 1e-9 && c.im.abs() < 1e-9,
                "silent in→silent out (enabled={enabled}); got {:?}", c);
        }
    }
}
