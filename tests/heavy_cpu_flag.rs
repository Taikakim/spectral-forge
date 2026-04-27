use spectral_forge::dsp::modules::{ModuleType, create_module};

#[test]
fn heavy_cpu_flag_defaults_to_module_spec_value() {
    let m = create_module(ModuleType::Dynamics, 48000.0, 2048);
    // Dynamics is light; default heavy_cpu_for_mode() must be false.
    assert!(!m.heavy_cpu_for_mode());
}

#[test]
fn empty_and_master_are_never_heavy() {
    for ty in [ModuleType::Empty, ModuleType::Master] {
        let m = create_module(ty, 48000.0, 2048);
        assert!(!m.heavy_cpu_for_mode());
    }
}

// ── Test-only heavy module ────────────────────────────────────────────────────

/// A stub SpectralModule that:
/// - reports heavy_cpu_for_mode() == true
/// - in process(), writes all-zero into `bins` (distinguishable from any non-zero passthrough)
struct HeavyZeroModule;

impl spectral_forge::dsp::modules::SpectralModule for HeavyZeroModule {
    fn process(
        &mut self,
        _channel: usize,
        _stereo_link: spectral_forge::params::StereoLink,
        _target: spectral_forge::params::FxChannelTarget,
        bins: &mut [num_complex::Complex<f32>],
        _sidechain: Option<&[f32]>,
        _curves: &[&[f32]],
        suppression_out: &mut [f32],
        _physics: Option<&mut spectral_forge::dsp::bin_physics::BinPhysics>,
        _ctx: &spectral_forge::dsp::modules::ModuleContext<'_>,
    ) {
        // Stamp every bin with zero so the caller can detect whether this ran.
        for b in bins.iter_mut() {
            *b = num_complex::Complex::new(0.0, 0.0);
        }
        suppression_out.fill(0.0);
    }

    fn reset(&mut self, _sample_rate: f32, _fft_size: usize) {}
    fn module_type(&self) -> ModuleType { ModuleType::Gain }  // arbitrary non-Empty type
    fn num_curves(&self) -> usize { 0 }
    fn heavy_cpu_for_mode(&self) -> bool { true }
}

#[test]
fn heavy_module_short_circuit_bypasses_processing() {
    use num_complex::Complex;
    use spectral_forge::dsp::{
        modules::{ModuleType, ModuleContext, RouteMatrix, MAX_SLOTS, MAX_MATRIX_ROWS},
        fx_matrix::FxMatrix,
        pipeline::MAX_NUM_BINS,
    };
    use spectral_forge::params::{StereoLink, FxChannelTarget};

    let n = 1025usize; // 2048/2+1
    // Slot 0 = arbitrary non-Empty stub (injected directly), slot 8 = Master.
    let mut types = [ModuleType::Empty; 9];
    types[8] = ModuleType::Master;
    let mut fm = FxMatrix::new(44100.0, 2048, &types);

    // Install the heavy stub in slot 0 directly (it's not a registered ModuleType variant).
    fm.slots[0] = Some(Box::new(HeavyZeroModule));

    // Route: slot 0 → Master (slot 8) only.
    let mut rm = RouteMatrix::default();
    rm.send = [[0.0f32; MAX_SLOTS]; MAX_MATRIX_ROWS];
    rm.send[0][8] = 1.0;

    let input_mag = 2.0f32;
    let curves: Vec<Vec<Vec<f32>>> = (0..9)
        .map(|_| (0..7).map(|_| vec![1.0f32; MAX_NUM_BINS]).collect())
        .collect();
    let sc: [Option<&[f32]>; 9] = [None; 9];
    let targets = [FxChannelTarget::All; 9];
    let ctx = ModuleContext::new(
        44100.0, 2048, n,
        10.0, 100.0, 0.0, 0.0, false, false,
    );
    let mut supp = vec![0.0f32; n];

    // ── Flag OFF: heavy module is bypassed; output must equal input (passthrough) ──
    let mut bins_off: Vec<Complex<f32>> = vec![Complex::new(input_mag, 0.0); n];
    fm.process_hop(
        0, StereoLink::Linked, &mut bins_off, &sc, &targets,
        &curves, &rm, &ctx, &mut supp, n,
        /*enable_heavy_modules=*/ false,
    );
    // The HeavyZeroModule's process() zeros all bins. With the flag OFF it must
    // NOT run, so every bin should carry the original non-zero input magnitude.
    for (k, b) in bins_off.iter().enumerate() {
        assert!(
            b.norm() > 0.5,
            "bin {k}: short-circuit should passthrough input (mag={input_mag}), got {}",
            b.norm()
        );
    }

    // ── Flag ON: heavy module runs; output must be zero (its process() stamps zeros) ──
    // Re-install the stub (process_hop may have taken/replaced it).
    fm.slots[0] = Some(Box::new(HeavyZeroModule));
    let mut bins_on: Vec<Complex<f32>> = vec![Complex::new(input_mag, 0.0); n];
    fm.process_hop(
        0, StereoLink::Linked, &mut bins_on, &sc, &targets,
        &curves, &rm, &ctx, &mut supp, n,
        /*enable_heavy_modules=*/ true,
    );
    // With the flag ON, process() runs and stamps zeros into mix_buf → slot_out[0] = 0.
    // Master accumulates from slot 0 and writes zeros to complex_buf.
    for (k, b) in bins_on.iter().enumerate() {
        assert!(
            b.norm() < 1e-6,
            "bin {k}: heavy module should have zeroed output when flag is ON, got {}",
            b.norm()
        );
    }
}
