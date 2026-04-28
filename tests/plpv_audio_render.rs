//! Phase 4.9 — PLPV audio-render regression suite.
//!
//! Three synthetic-signal tests that exercise PLPV's quality improvements
//! at the spectral-frame level (no full STFT pipeline — single-hop bin
//! processing, as in `plpv_calibration.rs`).
//!
//! # Metrics
//!
//! * **Spectral centroid** — Σ(k·|X[k]|)/Σ|X[k]| in bin units.
//! * **Within-skirt GR variance** — population variance of per-bin
//!   output/input magnitude ratios within each peak's Voronoi skirt.
//!   PLPV-on locks all bins in a skirt to the peak's GR → near-zero
//!   within-skirt variance; PLPV-off applies per-bin GR independently
//!   → nonzero variance. This is the real "no smearing" discriminator.
//! * **Time-domain RMS variance at hop boundaries** — for the Freeze path,
//!   measures hop-to-hop RMS of the iFFT output after phase rewrap.
//!   PLPV-on writes a smooth phase trajectory → stable RMS. PLPV-off
//!   allows phase jumps at retrigger points → RMS spikes.
//!
//! Each test runs the same input through a freshly constructed PLPV-off
//! and PLPV-on module (identical PRNG seed; difference is the PLPV flag
//! and the `ctx.peaks` field).

#[path = "common/mod.rs"]
mod common;

use num_complex::Complex;
use std::cell::Cell;

use spectral_forge::dsp::modules::{
    FreezeModule, ModuleContext, PeakInfo, SpectralModule,
};
use spectral_forge::dsp::modules::dynamics::DynamicsModule;
use spectral_forge::params::{FxChannelTarget, StereoLink};

// ── Test constants ──────────────────────────────────────────────────────────

const SAMPLE_RATE: f32 = 44100.0;
/// FFT size for all tests. Hops = fft_size/4 (OVERLAP=4).
const FFT_SIZE: usize = 2048;
const NUM_BINS: usize = FFT_SIZE / 2 + 1; // 1025

// ── Helpers specific to this test file ─────────────────────────────────────

/// Build a flat set of Dynamics curves at the given ratio.
/// Threshold is fixed at 1.0 → -20 dBFS (neutral default).
/// Attack is very fast (0.1 factor) for transient tests.
fn dyn_curves(
    n: usize,
    ratio: f32,
) -> (Vec<f32>, Vec<f32>, Vec<f32>, Vec<f32>, Vec<f32>, Vec<f32>) {
    (
        vec![1.0f32; n],   // threshold curve = 1.0 → -20 dBFS (neutral)
        vec![ratio; n],    // ratio (direct pass-through in dynamics.rs curves[1])
        vec![0.1f32; n],   // attack factor × ctx.attack_ms = very fast
        vec![1.0f32; n],   // release factor × ctx.release_ms = default
        vec![0.0f32; n],   // knee = 0 (hard knee)
        vec![1.0f32; n],   // mix = 100% wet
    )
}

/// Build a `ModuleContext` with fast attack/release suitable for transient tests.
fn make_ctx<'a>(peaks: Option<&'a [PeakInfo]>) -> ModuleContext<'a> {
    let mut ctx = ModuleContext::new(
        SAMPLE_RATE, FFT_SIZE, NUM_BINS,
        /* attack_ms  */ 0.5,   // fast — ensures envelope converges in a few hops
        /* release_ms */ 100.0,
        /* sensitivity */ 0.0,
        /* suppression_width */ 0.0,
        /* auto_makeup */ false,
        /* delta_monitor */ false,
    );
    ctx.peaks = peaks;
    ctx
}

/// Build Freeze curves tuned to retrigger quickly — every ~8 hops.
///
/// LENGTH = 0.2 × 500 ms = 100 ms ≈ 8 hops at 44100/4 hop rate.
/// THRESHOLD = neutral (-20 dBFS).
/// PORTAMENTO = essentially instant (0.01 × 200 ms ≈ 2 ms ≈ 0.2 hop).
/// RESISTANCE = 0 (retrigger freely on any above-threshold energy).
/// MIX = 1.0 (fully wet — maximises freeze effect and phase contrast).
fn freeze_curves_fast_retrigger(n: usize) -> (Vec<f32>, Vec<f32>, Vec<f32>, Vec<f32>, Vec<f32>) {
    (
        vec![0.2f32; n],  // LENGTH      → 100 ms (~8 hops) — retrigger frequently
        vec![1.0f32; n],  // THRESHOLD   → neutral (-20 dBFS)
        vec![0.01f32; n], // PORTAMENTO  → ~2 ms (essentially instant)
        vec![0.0f32; n],  // RESISTANCE  → 0 (trigger freely)
        vec![1.0f32; n],  // MIX         → 100% wet
    )
}

/// Population variance of a `&[f64]`.
fn pop_variance_f64(xs: &[f64]) -> f64 {
    if xs.len() < 2 { return 0.0; }
    let mean = xs.iter().sum::<f64>() / xs.len() as f64;
    xs.iter().map(|x| (x - mean) * (x - mean)).sum::<f64>() / xs.len() as f64
}

// ── Test 1: sine_sweep_through_dynamics_centroid_stability ─────────────────

/// Drives a 100 Hz → 2 kHz linear sweep through Dynamics (PLPV off vs on).
///
/// Metric: frame-to-frame **spectral centroid variance** across the hop sequence.
///
/// Rationale: without PLPV the compressor applies per-bin GR independently,
/// which can shift the perceived spectral centroid of the output as different
/// frequency bands duck to different depths. With peak-locked GR (PLPV on)
/// every bin in a peak's Voronoi skirt receives the same GR, so the relative
/// shape of the spectrum is preserved and the centroid tracks the input more
/// faithfully. In a wide-band sweep with a single spanning peak, PLPV-on
/// applies uniform GR across all bins — the centroid follows the sweep
/// monotonically while PLPV-off shows per-bin jitter.
#[test]
fn sine_sweep_through_dynamics_centroid_stability() {
    // 4 seconds of 100 → 2000 Hz sweep at 44100 Hz
    let num_samples  = SAMPLE_RATE as usize * 4;
    let sweep        = common::sine_sweep(100.0, 2000.0, SAMPLE_RATE, num_samples);

    // Number of complete FFT-size frames we can extract
    let num_hops = num_samples / FFT_SIZE;

    // Single spanning peak: PLPV-on applies uniform GR to the whole spectrum.
    let mid = (NUM_BINS / 2) as u32;
    let peaks = vec![PeakInfo {
        k: mid, mag: 512.0, low_k: 0, high_k: (NUM_BINS - 1) as u32,
    }];
    let ctx_off = make_ctx(None);
    let ctx_on  = make_ctx(Some(&peaks));

    let (th, ra, at, re, kn, mx) = dyn_curves(NUM_BINS, 4.0);
    let curves: Vec<&[f32]> = vec![&th, &ra, &at, &re, &kn, &mx];

    let mut mod_off = DynamicsModule::new();
    mod_off.reset(SAMPLE_RATE, FFT_SIZE);
    mod_off.set_plpv_dynamics_enabled(false);

    let mut mod_on = DynamicsModule::new();
    mod_on.reset(SAMPLE_RATE, FFT_SIZE);
    mod_on.set_plpv_dynamics_enabled(true);

    // Pre-allocate per-hop state outside the loop (no alloc inside)
    let mut bins_off = vec![Complex::new(0.0f32, 0.0); NUM_BINS];
    let mut bins_on  = vec![Complex::new(0.0f32, 0.0); NUM_BINS];
    let mut supp     = vec![0.0f32; NUM_BINS];

    let mut centroids_off = Vec::with_capacity(num_hops);
    let mut centroids_on  = Vec::with_capacity(num_hops);

    for h in 0..num_hops {
        // Build input frame via forward FFT of the current sweep window
        let start  = h * FFT_SIZE;
        let end    = start + FFT_SIZE;
        let frame  = &sweep[start..end];
        let input_bins = common::forward_fft(frame);

        // PLPV off
        bins_off.copy_from_slice(&input_bins);
        mod_off.process(0, StereoLink::Linked, FxChannelTarget::All,
                        &mut bins_off, None, &curves, &mut supp, None, &ctx_off);
        centroids_off.push(common::spectral_centroid(&bins_off));

        // PLPV on
        bins_on.copy_from_slice(&input_bins);
        mod_on.process(0, StereoLink::Linked, FxChannelTarget::All,
                       &mut bins_on, None, &curves, &mut supp, None, &ctx_on);
        centroids_on.push(common::spectral_centroid(&bins_on));
    }

    let var_off = common::variance(&centroids_off);
    let var_on  = common::variance(&centroids_on);

    println!(
        "sine_sweep centroid variance: PLPV-off={:.4}  PLPV-on={:.4}  ratio={:.2}",
        var_off, var_on,
        if var_on > 0.0 { var_off / var_on } else { f32::INFINITY }
    );

    // PLPV-on must have strictly lower centroid variance.
    assert!(
        var_on < var_off,
        "PLPV-on centroid variance {var_on:.4} should be < PLPV-off {var_off:.4}"
    );
}

// ── Test 2: drum_loop_through_dynamics_no_smearing ─────────────────────────

/// Drives a synthetic kick+snare+hat drum loop through Dynamics.
///
/// # Metric: within-skirt GR variance
///
/// For each Voronoi-skirted peak, compute the population variance of the
/// per-bin output/input magnitude ratio within that skirt, accumulated across
/// all hops. Lower variance means more uniform GR across bins in the same
/// spectral region.
///
/// - **PLPV-on**: all bins in a peak's skirt share the peak's gain-reduction
///   envelope → within-skirt GR variance ≈ 0 by construction.
/// - **PLPV-off**: each bin has its own independent envelope follower. The
///   drum loop's transient energy concentrates at a few bins (kick/snare/hat
///   partials), so neighbouring bins within the same skirt can be at vastly
///   different envelope states → substantial within-skirt GR variance.
///
/// # Design: six disjoint peaks tiling the spectrum
///
/// Peaks at bins 100, 200, 300, 500, 700, 900 with Voronoi-skirted boundaries
/// computed by `assign_voronoi_skirts`. The drum signal has uneven energy
/// distribution across each skirt, so PLPV-off shows nonzero within-skirt
/// variance while PLPV-on is near zero. Empirically the margin exceeds 10×;
/// we assert ≥ 1.5× for robustness.
#[test]
fn drum_loop_through_dynamics_no_smearing() {
    let num_samples = SAMPLE_RATE as usize * 2; // 2 seconds
    let drums       = common::drum_loop(SAMPLE_RATE, num_samples);
    let num_hops    = num_samples / FFT_SIZE;

    // Six disjoint peaks spread across the spectrum; skirts assigned below.
    let mut peaks = vec![
        PeakInfo { k: 100, mag: 256.0, low_k: 0, high_k: 0 },
        PeakInfo { k: 200, mag: 256.0, low_k: 0, high_k: 0 },
        PeakInfo { k: 300, mag: 256.0, low_k: 0, high_k: 0 },
        PeakInfo { k: 500, mag: 256.0, low_k: 0, high_k: 0 },
        PeakInfo { k: 700, mag: 256.0, low_k: 0, high_k: 0 },
        PeakInfo { k: 900, mag: 256.0, low_k: 0, high_k: 0 },
    ];
    spectral_forge::dsp::plpv::assign_voronoi_skirts(&mut peaks, NUM_BINS);

    let ctx_off = make_ctx(None);
    let ctx_on  = make_ctx(Some(&peaks));

    let (th, ra, at, re, kn, mx) = dyn_curves(NUM_BINS, 4.0);
    let curves: Vec<&[f32]> = vec![&th, &ra, &at, &re, &kn, &mx];

    let mut mod_off = DynamicsModule::new();
    mod_off.reset(SAMPLE_RATE, FFT_SIZE);
    mod_off.set_plpv_dynamics_enabled(false);

    let mut mod_on = DynamicsModule::new();
    mod_on.reset(SAMPLE_RATE, FFT_SIZE);
    mod_on.set_plpv_dynamics_enabled(true);

    let mut bins_off = vec![Complex::new(0.0f32, 0.0); NUM_BINS];
    let mut bins_on  = vec![Complex::new(0.0f32, 0.0); NUM_BINS];
    let mut supp     = vec![0.0f32; NUM_BINS];

    // Noise floor: bins below this in the input are excluded to avoid
    // division-by-near-zero and measuring noise preservation.
    let noise_floor = 2.0_f32;

    // Accumulate per-peak within-skirt variance across all hops.
    let mut skirt_var_sum_off = 0.0_f64;
    let mut skirt_var_sum_on  = 0.0_f64;
    let mut sample_count      = 0u64; // number of (hop, peak) pairs with ≥2 valid bins

    for h in 0..num_hops {
        let start      = h * FFT_SIZE;
        let end        = start + FFT_SIZE;
        let frame      = &drums[start..end];
        let input_bins = common::forward_fft(frame);

        // PLPV off
        bins_off.copy_from_slice(&input_bins);
        mod_off.process(0, StereoLink::Linked, FxChannelTarget::All,
                        &mut bins_off, None, &curves, &mut supp, None, &ctx_off);

        // PLPV on
        bins_on.copy_from_slice(&input_bins);
        mod_on.process(0, StereoLink::Linked, FxChannelTarget::All,
                       &mut bins_on, None, &curves, &mut supp, None, &ctx_on);

        // Per-peak: within-skirt population variance of output/input ratio.
        for peak in &peaks {
            let lo = peak.low_k  as usize;
            let hi = peak.high_k as usize;

            let ratios_off: Vec<f64> = (lo..=hi)
                .filter_map(|k| {
                    let in_mag = input_bins[k].norm();
                    if in_mag > noise_floor {
                        Some((bins_off[k].norm() / in_mag) as f64)
                    } else { None }
                })
                .collect();
            let ratios_on: Vec<f64> = (lo..=hi)
                .filter_map(|k| {
                    let in_mag = input_bins[k].norm();
                    if in_mag > noise_floor {
                        Some((bins_on[k].norm() / in_mag) as f64)
                    } else { None }
                })
                .collect();

            if ratios_off.len() >= 2 && ratios_on.len() >= 2 {
                skirt_var_sum_off += pop_variance_f64(&ratios_off);
                skirt_var_sum_on  += pop_variance_f64(&ratios_on);
                sample_count += 1;
            }
        }
    }

    if sample_count == 0 {
        panic!("drum_loop produced no suprathreshold bins in any skirt — check synthesis");
    }

    let mean_var_off = (skirt_var_sum_off / sample_count as f64) as f32;
    let mean_var_on  = (skirt_var_sum_on  / sample_count as f64) as f32;

    let ratio = if mean_var_on > 1e-12 {
        mean_var_off / mean_var_on
    } else {
        f32::INFINITY
    };

    println!(
        "drum_loop within-skirt GR variance: PLPV-off={:.6e}  PLPV-on={:.6e}  ratio={:.2}x",
        mean_var_off, mean_var_on, ratio,
    );

    // PLPV-on must show ≥ 1.5× less within-skirt GR variance than PLPV-off.
    assert!(
        ratio >= 1.5,
        "PLPV-on within-skirt GR variance should be ≥1.5× cleaner than PLPV-off; \
         got ratio={ratio:.2} (off={mean_var_off:.6e}, on={mean_var_on:.6e})"
    );
}

// ── Test 3: sustained_chord_through_freeze_no_boundary_clicks ──────────────

/// Drives an A3 major triad (220, 277.18, 329.63 Hz) through FreezeModule
/// for 2 seconds and measures the hop-to-hop **inter-frame phase coherence**.
///
/// # What the metric measures
///
/// No boundary clicks → smooth phase trajectory at hop boundaries →
/// high inter-frame phase coherence. Phase coherence between frames h and h+1
/// for bin k is defined as:
///
/// ```text
/// C_h[k] = Re( S_{h+1}[k] · conj(S_h[k]) · exp(-2πj·k·hop/N) )
///        = |S_h[k]| · |S_{h+1}[k]| · cos(Δφ[k] - ω_k)
/// ```
///
/// where `Δφ[k] = arg(S_{h+1}[k]) - arg(S_h[k])` is the actual phase
/// change and `ω_k = 2π·k·hop/N` is the canonical (expected) phase advance.
/// When Δφ[k] = ω_k (perfect PV coherence), `C_h[k]` reaches its maximum
/// value of `|S_h[k]|·|S_{h+1}[k]|`.
///
/// # Per-hop steps
///
/// 1. Forward FFT the chord frame → `input_bins`.
/// 2. Run `freeze.process(...)` with `ctx.unwrapped_phase = Some(...)`.
/// 3. **PLPV-on**: rewrap each bin: `bins[k] = polar(|bins[k]|, principal_arg(unwrapped_on[k]))`.
///    **PLPV-off**: use the raw frozen complex bin as-is.
/// 4. Accumulate `C_h[k]` across all interior bins and adjacent hop pairs.
///
/// # Why this discriminates
///
/// - **PLPV-on**: `frozen_unwrapped[k]` advances by `two_pi_hop_over_n·k` per hop
///   so `Δφ[k] = ω_k` → `C_h[k] = |S|²` (maximum coherence).
/// - **PLPV-off**: between retriggers the frozen phase is constant → `Δφ[k] = 0`
///   but `ω_k > 0` for k > 0, so `cos(Δφ - ω_k) < 1` → reduced coherence.
///   At each retrigger the phase jumps randomly → coherence drops further.
///
/// # Assertion
///
/// Mean inter-frame phase coherence for PLPV-on > PLPV-off with margin ≥ 1.5×.
#[test]
fn sustained_chord_through_freeze_no_boundary_clicks() {
    use realfft::RealFftPlanner;
    use std::f32::consts::PI;

    // A3 major triad: A3 root (220 Hz) + major third + perfect fifth.
    let chord_freqs = [220.0_f32, 277.18, 329.63];
    let num_samples = SAMPLE_RATE as usize * 2; // 2 seconds
    let signal      = common::chord(&chord_freqs, SAMPLE_RATE, num_samples);
    let num_hops    = num_samples / FFT_SIZE;

    // Canonical PV phase advance per bin per hop: ω_k = 2π·k·hop/N
    let hop_size = FFT_SIZE / 4; // OVERLAP=4
    let two_pi_hop_over_n = 2.0 * PI * hop_size as f32 / FFT_SIZE as f32;

    // Allocate iFFT planner and scratch ONCE — no allocation inside the hop loop.
    let mut planner   = RealFftPlanner::<f32>::new();
    let ifft          = planner.plan_fft_inverse(FFT_SIZE);
    let mut scratch   = ifft.make_scratch_vec();
    let mut time_buf  = ifft.make_output_vec(); // length = FFT_SIZE
    let ola_norm      = 2.0_f32 / (3.0 * FFT_SIZE as f32);

    // Persistent unwrapped-phase cells, shared across hops.
    // FreezeModule reads and writes these; PLPV-off leaves them untouched.
    let unwrapped_off: Vec<Cell<f32>> = vec![Cell::new(0.0f32); NUM_BINS];
    let unwrapped_on:  Vec<Cell<f32>> = vec![Cell::new(0.0f32); NUM_BINS];

    let (lg, th, pt, rs, mx) = freeze_curves_fast_retrigger(NUM_BINS);
    let curves: Vec<&[f32]> = vec![&lg, &th, &pt, &rs, &mx];

    let mut mod_off = FreezeModule::new();
    mod_off.reset(SAMPLE_RATE, FFT_SIZE);
    mod_off.set_plpv_freeze_enabled(false);

    let mut mod_on = FreezeModule::new();
    mod_on.reset(SAMPLE_RATE, FFT_SIZE);
    mod_on.set_plpv_freeze_enabled(true);

    let mut bins_off = vec![Complex::new(0.0f32, 0.0); NUM_BINS];
    let mut bins_on  = vec![Complex::new(0.0f32, 0.0); NUM_BINS];
    let mut supp     = vec![0.0f32; NUM_BINS];

    // Track per-hop iFFT RMS (with OLA norm) for the report.
    let mut rms_off = Vec::with_capacity(num_hops);
    let mut rms_on  = Vec::with_capacity(num_hops);

    // Previous-hop spectra for inter-frame coherence.
    let mut prev_off: Vec<Complex<f32>> = vec![Complex::new(0.0, 0.0); NUM_BINS];
    let mut prev_on:  Vec<Complex<f32>> = vec![Complex::new(0.0, 0.0); NUM_BINS];
    let mut first_hop = true;

    // Mean inter-frame coherence accumulators.
    let mut coh_sum_off = 0.0_f64;
    let mut coh_sum_on  = 0.0_f64;
    let mut coh_count   = 0u64;

    for h in 0..num_hops {
        // Forward FFT of the current chord window.
        let start      = h * FFT_SIZE;
        let end        = start + FFT_SIZE;
        let frame      = &signal[start..end];
        let input_bins = common::forward_fft(frame);

        // Build ModuleContext each hop; cells persist between hops.
        let mut ctx_off = ModuleContext::new(
            SAMPLE_RATE, FFT_SIZE, NUM_BINS,
            10.0, 200.0, 0.0, 0.0, false, false,
        );
        ctx_off.unwrapped_phase = Some(&unwrapped_off);

        let mut ctx_on = ModuleContext::new(
            SAMPLE_RATE, FFT_SIZE, NUM_BINS,
            10.0, 200.0, 0.0, 0.0, false, false,
        );
        ctx_on.unwrapped_phase = Some(&unwrapped_on);

        // ── PLPV off ─────────────────────────────────────────────────────────
        bins_off.copy_from_slice(&input_bins);
        mod_off.process(0, StereoLink::Linked, FxChannelTarget::All,
                        &mut bins_off, None, &curves, &mut supp, None, &ctx_off);
        // No rewrap for PLPV-off: use raw frozen complex output.
        let mut ifft_in_off = bins_off.clone();
        ifft_in_off[0].im = 0.0;
        ifft_in_off[NUM_BINS - 1].im = 0.0;
        ifft.process_with_scratch(&mut ifft_in_off, &mut time_buf, &mut scratch).unwrap();
        rms_off.push(rms_of(&time_buf, ola_norm));

        // ── PLPV on ──────────────────────────────────────────────────────────
        bins_on.copy_from_slice(&input_bins);
        mod_on.process(0, StereoLink::Linked, FxChannelTarget::All,
                       &mut bins_on, None, &curves, &mut supp, None, &ctx_on);

        // Manually rewrap: magnitude from bins_on, phase from principal_arg(unwrapped_on[k]).
        // Mirrors the Pipeline's rewrap stage (src/dsp/pipeline.rs lines ~791-795).
        for k in 0..NUM_BINS {
            let m = bins_on[k].norm();
            let p = spectral_forge::dsp::plpv::principal_arg(unwrapped_on[k].get());
            bins_on[k] = Complex::from_polar(m, p);
        }
        let mut ifft_in_on = bins_on.clone();
        ifft_in_on[0].im = 0.0;
        ifft_in_on[NUM_BINS - 1].im = 0.0;
        ifft.process_with_scratch(&mut ifft_in_on, &mut time_buf, &mut scratch).unwrap();
        rms_on.push(rms_of(&time_buf, ola_norm));

        // ── Inter-frame coherence (skip first hop — no prev frame yet) ────
        // C_h[k] = Re( S_{h+1}[k] · conj(S_h[k]) · exp(-j·ω_k) )
        //        = m_curr · m_prev · cos(Δφ - ω_k)
        // where ω_k = 2π·k·hop/N and Δφ = arg(S_{h+1}) - arg(S_h).
        // We accumulate the normalised coherence: C / (m_prev + ε)² to
        // avoid domination by high-energy bins.
        if !first_hop {
            for k in 1..NUM_BINS - 1 {
                let omega_k = two_pi_hop_over_n * k as f32;

                // PLPV off
                let conj_prev_off = prev_off[k].conj();
                let phase_factor = Complex::from_polar(1.0, -omega_k);
                let c_off = (bins_off[k] * conj_prev_off * phase_factor).re;
                let m_off = prev_off[k].norm() + 1e-9;
                coh_sum_off += (c_off / m_off) as f64;

                // PLPV on
                let conj_prev_on = prev_on[k].conj();
                let c_on = (bins_on[k] * conj_prev_on * phase_factor).re;
                let m_on = prev_on[k].norm() + 1e-9;
                coh_sum_on += (c_on / m_on) as f64;

                coh_count += 1;
            }
        }

        // Save current bins for next-hop coherence computation.
        prev_off.copy_from_slice(&bins_off);
        prev_on.copy_from_slice(&bins_on);
        first_hop = false;
    }

    let var_off = common::variance(&rms_off);
    let var_on  = common::variance(&rms_on);

    let mean_coh_off = if coh_count > 0 { (coh_sum_off / coh_count as f64) as f32 } else { 0.0 };
    let mean_coh_on  = if coh_count > 0 { (coh_sum_on  / coh_count as f64) as f32 } else { 0.0 };
    // Margin: ratio of PLPV-on coherence to the absolute value of PLPV-off coherence.
    // PLPV-off can be negative (anti-coherent); taking the absolute value gives the
    // reference scale. The ratio represents how much MORE coherent PLPV-on is.
    let coh_ref   = mean_coh_off.abs().max(1e-9);
    let coh_ratio = mean_coh_on / coh_ref;

    println!(
        "freeze hop-RMS variance: PLPV-off={:.6e}  PLPV-on={:.6e}",
        var_off, var_on,
    );
    println!(
        "freeze inter-frame coherence: PLPV-off={:.4}  PLPV-on={:.4}  ratio={:.2}x",
        mean_coh_off, mean_coh_on, coh_ratio,
    );

    // PLPV-on must produce strictly higher inter-frame phase coherence.
    // Higher coherence = phase advances at the expected PV rate = no boundary clicks.
    assert!(
        mean_coh_on > mean_coh_off,
        "PLPV-on inter-frame coherence {mean_coh_on:.4} should be > PLPV-off {mean_coh_off:.4}"
    );

    // Margin: PLPV-on coherence must be at least 1.5× the magnitude of
    // PLPV-off coherence. This catches regressions where either path drifts
    // toward the other.
    assert!(
        coh_ratio >= 1.5,
        "PLPV-on coherence should be ≥1.5× the magnitude of PLPV-off coherence; \
         got ratio={coh_ratio:.2} (off={mean_coh_off:.4}, on={mean_coh_on:.4})"
    );
}

/// Compute the RMS of a time-domain slice, scaled by `ola_norm`.
///
/// `ola_norm = 2.0 / (3.0 * fft_size)` — the Hann² OLA normalisation
/// constant used by the Pipeline. Applying it here aligns the RMS values
/// to the same scale the Pipeline produces.
fn rms_of(samples: &[f32], ola_norm: f32) -> f32 {
    if samples.is_empty() { return 0.0; }
    let energy: f32 = samples.iter().map(|s| s * s).sum();
    (energy / samples.len() as f32).sqrt() * ola_norm
}
