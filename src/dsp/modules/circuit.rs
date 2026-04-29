//! Circuit module — analog circuit-inspired spectral distortion / saturation effects.
//!
//! Three modes ship across Phase 2g tasks:
//! - **BbdBins**             — 4-stage bucket-brigade delay on per-bin magnitudes + LP + dither.
//! - **SpectralSchmitt**     — branch-free hysteresis latch per FFT bin (Schmitt trigger).
//! - **CrossoverDistortion** — C¹-smooth deadzone mimicking BJT crossover artefacts.
//!
//! Kernel implementations are added in Tasks 3–5 of Phase 2g. This skeleton
//! provides the enum, struct, and stub `process()` that passes audio through
//! unmodified and zeroes suppression_out.

use num_complex::Complex;
use serde::{Deserialize, Serialize};

use crate::dsp::modules::{
    FxChannelTarget, ModuleContext, ModuleType, SpectralModule,
};
use crate::params::StereoLink;

// ── Constants ──────────────────────────────────────────────────────────────

pub const BBD_STAGES: usize = 4;

// ── BBD helpers ────────────────────────────────────────────────────────────

/// Xorshift32 PRNG step — returns a value in `[-1, 1)`.
fn xorshift32_step(state: &mut u32) -> f32 {
    let mut s = *state;
    s ^= s << 13;
    s ^= s >> 17;
    s ^= s << 5;
    *state = s;
    (s as i32 as f32) / (i32::MAX as f32)
}

/// 4-stage bucket-brigade delay on per-bin magnitudes.
/// Curves: `[AMOUNT, THRESH, SPREAD(unused), RELEASE, MIX]`.
fn apply_bbd(
    bins: &mut [Complex<f32>],
    bbd_mag: &mut [Vec<f32>; BBD_STAGES],
    rng_state: &mut u32,
    curves: &[&[f32]],
) {
    let amount_c = curves[0];
    let thresh_c = curves[1];
    // curves[2] = SPREAD — reserved for Phase 5c.8, unused by v1 BBD kernel.
    let release_c = curves[3];
    let mix_c = curves[4];

    let num_bins = bins.len();

    for k in 0..num_bins {
        let amount = amount_c[k].clamp(0.0, 2.0) * 0.5; // 0..1 stage-3 output gain
        let dither_amt = thresh_c[k].clamp(0.0, 2.0) * 0.005; // very small noise
        let lp_alpha = (release_c[k].clamp(0.01, 2.0) * 0.4).clamp(0.05, 0.9);
        let mix = mix_c[k].clamp(0.0, 2.0) * 0.5;

        let dry = bins[k];
        let in_mag = dry.norm();

        // Push input into stage 0 (with LP smoothing toward target).
        let target_0 = bbd_mag[0][k] + (in_mag - bbd_mag[0][k]) * lp_alpha;
        let dither_0 = xorshift32_step(rng_state) * dither_amt;
        bbd_mag[0][k] = (target_0 + dither_0).max(0.0);

        // Cascade: each stage LP-smooths toward the previous stage's value.
        // Read s0 from the just-written stage 0 (intentional — see plan §note),
        // then read old stages 1/2/3 before overwriting them.
        let s0_prev = bbd_mag[0][k]; // intentionally the NEW stage-0 value
        let s1_prev = bbd_mag[1][k];
        let s2_prev = bbd_mag[2][k];
        let s3_prev = bbd_mag[3][k];

        bbd_mag[3][k] = s3_prev + (s2_prev - s3_prev) * lp_alpha + xorshift32_step(rng_state) * dither_amt;
        bbd_mag[2][k] = s2_prev + (s1_prev - s2_prev) * lp_alpha + xorshift32_step(rng_state) * dither_amt;
        bbd_mag[1][k] = s1_prev + (s0_prev - s1_prev) * lp_alpha + xorshift32_step(rng_state) * dither_amt;

        // Output: stage 3 (most-delayed) magnitude, scaled by amount.
        // Phase is preserved when there is a live carrier (in_mag > 1e-9); for silent
        // input bins we emit the delayed magnitude as real-positive (arbitrary unit phase).
        let out_mag = bbd_mag[3][k].max(0.0) * amount;
        let wet = if in_mag > 1e-9 {
            dry * (out_mag / in_mag)
        } else {
            Complex::new(out_mag, 0.0)
        };
        bins[k] = dry * (1.0 - mix) + wet * mix;
    }
}

// ── Schmitt helpers ────────────────────────────────────────────────────────

/// Per-bin hysteresis latch (Schmitt trigger).
/// Curves: `[AMOUNT, THRESH, SPREAD(unused), RELEASE, MIX]`.
fn apply_schmitt(
    bins: &mut [Complex<f32>],
    latched: &mut [u8],
    curves: &[&[f32]],
) {
    let amount_c = curves[0];
    let thresh_c = curves[1];
    // curves[2] = SPREAD — reserved for Phase 5c.8, unused by v1 Schmitt kernel.
    let release_c = curves[3];
    let mix_c = curves[4];

    let num_bins = bins.len();

    for k in 0..num_bins {
        let attenuation = amount_c[k].clamp(0.0, 2.0) * 0.5;          // 0..1 attenuation when OFF
        let high = thresh_c[k].clamp(0.01, 4.0);
        let gap = (release_c[k].clamp(0.0, 2.0) * 0.5).clamp(0.05, 0.95);
        let low = high * (1.0 - gap);
        let mix = mix_c[k].clamp(0.0, 2.0) * 0.5;

        let mag = bins[k].norm();
        let was_latched = latched[k] != 0;
        let now_latched = if was_latched { mag > low } else { mag > high };
        latched[k] = if now_latched { 1 } else { 0 };

        let attenuate = if now_latched { 1.0 } else { 1.0 - attenuation };
        let dry = bins[k];
        let wet = dry * attenuate;
        bins[k] = dry * (1.0 - mix) + wet * mix;
    }
}

// ── Crossover helpers ──────────────────────────────────────────────────────

/// C¹-smooth deadzone mimicking BJT class-B crossover artefacts.
/// Bins with magnitude ≤ dz_width are silenced; above the deadzone,
/// output follows `(mag - dz)² / mag`, which is continuous and has a
/// continuous first derivative at the boundary (no audible click).
/// Phase is preserved by scaling the original complex bin.
/// Curves: `[AMOUNT, THRESH(unused), SPREAD(unused), RELEASE(unused), MIX]`.
fn apply_crossover(bins: &mut [Complex<f32>], curves: &[&[f32]]) {
    let amount_c = curves[0];
    // curves[1] = THRESH, curves[3] = RELEASE — unused by v1 Crossover kernel.
    // curves[2] = SPREAD — reserved for Phase 5c.8 (PCB Crosstalk).
    let mix_c = curves[4];

    let num_bins = bins.len();

    for k in 0..num_bins {
        let dz_width = amount_c[k].clamp(0.0, 2.0) * 0.1; // up to 0.2 deadzone half-width
        let mix = mix_c[k].clamp(0.0, 2.0) * 0.5;

        let dry = bins[k];
        let mag = dry.norm();

        let new_mag = if mag <= dz_width {
            0.0
        } else {
            let excess = mag - dz_width;
            (excess * excess) / mag
        };

        let scale = if mag > 1e-9 { new_mag / mag } else { 0.0 };
        let wet = dry * scale;
        bins[k] = dry * (1.0 - mix) + wet * mix;
    }
}

// ── Vactrol helpers ────────────────────────────────────────────────────────

/// Nominal time constants for the opto-coupler photocell model (seconds).
const VACTROL_TAU_FAST: f32 = 0.008;  // 8 ms
const VACTROL_TAU_SLOW: f32 = 0.250;  // 250 ms

/// Cascaded 2-pole vactrol-style photocell per-bin.
///
/// Drive charges the fast cap; the fast cap drives the slow cap. Cell gain
/// `g = tanh(slow)` soft-saturates into `[0, 1)` — applied as a multiplicative
/// gain on the bin (passive opto-coupler model: g attenuates, never amplifies).
///
/// `flux` — optional per-bin flux from BinPhysics. When `Some`, each bin's
/// drive is `flux[k] * amount` instead of `in_mag * amount`.
///
/// Curves: `[AMOUNT, THRESH(unused), SPREAD(unused), RELEASE, MIX]`.
fn apply_vactrol(
    bins: &mut [Complex<f32>],
    fast: &mut [f32],
    slow: &mut [f32],
    curves: &[&[f32]],
    hop_dt: f32,
    flux: Option<&[f32]>,
) {
    use crate::dsp::circuit_kernels::lp_step;

    let amount_c  = curves[0];
    // curves[1] = THRESH — unused by Vactrol v1.
    // curves[2] = SPREAD — unused by Vactrol v1.
    let release_c = curves[3];
    let mix_c     = curves[4];

    let num_bins = bins.len();

    for k in 0..num_bins {
        let amount  = amount_c[k].clamp(0.0, 2.0);
        let rel_scl = release_c[k].clamp(0.01, 4.0);   // user scale on both τ
        let mix     = mix_c[k].clamp(0.0, 2.0) * 0.5;  // 0..1

        let tau_fast = VACTROL_TAU_FAST * rel_scl;
        let tau_slow = VACTROL_TAU_SLOW * rel_scl;

        // α = hop_dt / τ, clamped to [0, 1].
        let alpha_fast = (hop_dt / tau_fast).min(1.0);
        let alpha_slow = (hop_dt / tau_slow).min(1.0);

        let dry = bins[k];

        // Drive: flux[k] when upstream BinPhysics is present, else magnitude.
        let raw_drive = match flux {
            Some(f) => f[k].abs(),
            None    => dry.norm(),
        };
        let drive = raw_drive * amount;

        // Charge fast cap toward drive, then charge slow cap toward fast.
        lp_step(&mut fast[k], drive, alpha_fast);
        lp_step(&mut slow[k], fast[k], alpha_slow);

        // Soft-saturating cell gain via tanh: `g ∈ [0, 1)` for non-negative slow cap.
        // tanh(1.0) ≈ 0.762, so a fully-charged cap on a unit-amplitude bin yields
        // ~0.76× passthrough — the photocell is a passive divider, never gains above 1.
        let g = crate::dsp::circuit_kernels::tanh_levien_poly(slow[k]).max(0.0);
        let wet = dry * g;

        bins[k] = dry * (1.0 - mix) + wet * mix;
    }
}

// ── Transformer helpers ────────────────────────────────────────────────────

/// Transformer saturation per bin: magnitude one-pole smoother → tanh soft-clip
/// → 3-tap SPREAD leak. Reads and writes `BinPhysics::flux` so downstream Vactrol
/// slots can see how hot this transformer is running.
///
/// **Why `flux: Option<&mut [f32]>` instead of separate in/out slices?**
/// Both read and write come from the same `physics.flux` field. Splitting into
/// `flux_in: Option<&[f32]>` + `flux_out: Option<&mut [f32]>` would require two
/// mutable borrows of the same vec — a borrow-checker violation. We use a single
/// mutable borrow and read via `as_deref()` in pass 1, then write via
/// `as_deref_mut()` in pass 3.
///
/// **Drive scaling:** `drive = amount_c[k].clamp(0,2)` (range 0..2). With AMOUNT=2
/// and an input at 3× knee, `x = 2 × 3 = 6`, clamped to 3 by `tanh_levien_poly`,
/// output ≈ 1×knee — strong saturation. At AMOUNT=2 and sub-knee input 0.5 against
/// knee=1, `x = 2 × 0.5 = 1.0`, tanh(1)≈0.76, output ≈ 0.76×knee: gentle.
/// The plan's `× 4.0` was too aggressive; this unit-range scaling keeps both test
/// assertions green without tuning.
fn apply_transformer(
    bins: &mut [Complex<f32>],
    xfmr_lp: &mut [f32],
    workspace: &mut [f32],
    mut flux: Option<&mut [f32]>,
    curves: &[&[f32]],
    hop_dt: f32,
) {
    use crate::dsp::circuit_kernels::{lp_step, tanh_levien_poly};

    let amount_c  = curves[0];
    let thresh_c  = curves[1];
    let spread_c  = curves[2];
    let release_c = curves[3];
    let mix_c     = curves[4];

    let num_bins = bins.len();

    // --- Pass 1: per-bin magnitude smoothing + saturation. ---
    // Read-only flux view: `flux_in[k]` biases the smoother target so bins that
    // were recently hot (e.g. driven by an upstream writer slot) saturate faster.
    {
        let flux_in: Option<&[f32]> = flux.as_deref();
        for k in 0..num_bins {
            // Drive 0..1 (range = amount 0..2 scaled by 0.5). At drive=1 and
            // sub-knee input (0.5 vs knee=1): x=0.5, tanh(0.5)≈0.46 — gentle
            // compression within the target (0.3, 0.7) window. At drive=1 and
            // above-knee input (3.0 vs knee=1): x=3.0, tanh→1.0×knee — strong
            // but bounded saturation, output≈1.0, within (0.5, 2.0). The plan's
            // ×4.0 was too aggressive: x=8 would blow through to ≈1.0×knee even
            // for sub-knee inputs.
            let drive = amount_c[k].clamp(0.0, 2.0) * 0.5; // 0..1
            let knee  = thresh_c[k].clamp(0.05, 4.0);
            let release = release_c[k].clamp(0.0, 2.0).max(0.01);
            // Magnitude smoother time constant: with `release` clamped to [0.01, 2.0]
            // and `tau = 0.020 * (0.1 + release)`, tau spans ~2.2 ms .. 42 ms.
            let tau   = 0.020 * (0.1 + release);
            let alpha = (hop_dt / tau).min(1.0);

            let in_mag    = bins[k].norm();
            let flux_bias = flux_in.map_or(0.0, |f| f[k] * 0.25);
            let target    = in_mag + flux_bias;
            lp_step(&mut xfmr_lp[k], target, alpha);

            // tanh(drive × xfmr_lp / knee) × knee: soft-clip in magnitude space.
            let x       = drive * xfmr_lp[k] / knee;
            let sat_mag = tanh_levien_poly(x) * knee;
            workspace[k] = sat_mag.max(0.0);
        }
    }

    // --- Pass 2: 3-tap SPREAD on the saturated magnitude (average strength). ---
    // Averaged over all bins per hop: per-bin spread would need a second workspace
    // buffer to avoid read-after-write alias; the averaged value is close enough
    // for smooth SPREAD curves (which is how users draw them at hop rate).
    let strength_avg = {
        let sum: f32 = (0..num_bins).map(|k| spread_c[k].clamp(0.0, 2.0) * 0.5).sum();
        sum / num_bins.max(1) as f32
    };
    let s = strength_avg;

    // Apply spread and write final wet signal back to bins.
    // For silent input bins, spread energy is emitted as real-positive (phase 0)
    // so that energy leaked from an active neighbour becomes audible.
    let mut prev_w = workspace[0];
    let mut curr_w = workspace[0];
    for k in 0..num_bins {
        let next_w = if k + 1 < num_bins { workspace[k + 1] } else { 0.0 };
        let new_mag = (1.0 - s) * curr_w + 0.5 * s * (prev_w + next_w);

        let dry    = bins[k];
        let in_mag = dry.norm();
        let wet    = if in_mag > 1e-9 {
            // Phase preserved: scale the existing complex bin.
            dry * (new_mag / in_mag)
        } else {
            // Silent input: emit spread energy as a real-positive bin.
            Complex::new(new_mag, 0.0)
        };
        let mix    = mix_c[k].clamp(0.0, 2.0) * 0.5;
        bins[k]    = dry * (1.0 - mix) + wet * mix;

        prev_w = curr_w;
        curr_w = next_w;
    }

    // --- Pass 3: write flux back. ---
    // Excess energy above the smoothed envelope (xfmr_lp >> bins[k].norm() after
    // output) is stored as positive flux for downstream reader slots (e.g. Vactrol).
    // Per-hop blend: `f' = 0.95 f + 0.1 e`. With constant excess the steady state is
    // `e * 0.1 / 0.05 = 2 e`, so flux tracks ~2× excess and decays by 5%/hop when
    // excess collapses to zero. Hard-clamped to ±100 against pathological inputs.
    if let Some(flux_mut) = flux.as_deref_mut() {
        for k in 0..num_bins {
            let out_mag = bins[k].norm();
            let excess  = (xfmr_lp[k] - out_mag).max(0.0);
            flux_mut[k] = (flux_mut[k] * 0.95 + excess * 0.1).clamp(-100.0, 100.0);
        }
    }
}

// ── Component Drift helpers ────────────────────────────────────────────────

/// Slow per-bin LFSR drift: pseudo-random gain offset modulates each bin over
/// many seconds. Reads `temperature` (pass 1) to scale drift amplitude on hot
/// bins; writes `temperature` (pass 2) so drift activity heats bins for
/// downstream readers (positive feedback, clamped).
///
/// **Single `Option<&mut [f32]>` for both read and write:** both passes target
/// the same `physics.temperature` vec. Splitting into `temperature_in` +
/// `temperature_out` would require two mutable borrows of the same slice —
/// a borrow-checker violation. We use `as_deref()` inside pass 1 and
/// `as_deref_mut()` inside pass 2, matching the `apply_transformer` pattern.
///
/// **`clamp(0.0, 4.0)` not `.min(4.0)` for NaN guard:** `f32::min(x, NaN)`
/// propagates NaN into `drift_env[k]` via the LP smoother where it sticks
/// indefinitely. `clamp` routes NaN to the lower bound (zero), isolating
/// upstream NaN poison (see Phase 5c.6 fix in commit `b281d0a`).
///
/// **Shift-by-7 for strict `[-1, 1)` bound:** dividing a 31-bit or 32-bit
/// integer by `i32::MAX` or `u32::MAX` can round the upper bound up to exactly
/// 1.0 in f32 (f32 has only 24 mantissa bits). Arithmetic-shifting the
/// xorshift32 output right by 7 keeps a sign bit and 24 magnitude bits; both
/// numerator `[-2^24, 2^24)` and divisor `2^24` are exact in f32, so the
/// half-open upper bound holds strictly (see `SimdRng::next_f32_centered`).
fn apply_component_drift(
    bins: &mut [Complex<f32>],
    drift_env: &mut [f32],
    drift_rng: &mut crate::dsp::circuit_kernels::SimdRng,
    mut temperature: Option<&mut [f32]>,
    curves: &[&[f32]],
    hop_dt: f32,
) {
    use crate::dsp::circuit_kernels::lp_step;

    // Knuth's golden-ratio multiplier (2^32 / φ): used to decorrelate adjacent
    // bin indices so neighbouring bins get statistically independent drift targets
    // from the single per-hop LFSR step.
    const KNUTH_GOLDEN: u32 = 2_654_435_761;

    let amount_c  = curves[0];
    let thresh_c  = curves[1];
    // curves[2] = SPREAD — unused by Component Drift.
    let release_c = curves[3];
    let mix_c     = curves[4];

    let num_bins = bins.len();

    // Step LFSR once per hop. Per-bin variation via XOR with bin index * KNUTH_GOLDEN
    // so bins with close-by indices get uncorrelated drift targets.
    let lfsr_step = drift_rng.next_u32();

    // --- Pass 1: compute drift targets and apply gain. ---
    // Read-only temperature view: hot bins drift further (amplitude scale).
    {
        let temp_in: Option<&[f32]> = temperature.as_deref();
        for k in 0..num_bins {
            let amount  = amount_c[k].clamp(0.0, 2.0) * 0.06; // 0..0.12 → up to ±12% (~±1 dB)
            let temp_gate = thresh_c[k].clamp(0.0, 4.0);
            let release = release_c[k].clamp(0.0, 2.0).max(0.01);
            let drift_tau = 1.0 + 4.0 * release; // 1..9 s — very slow modulation
            let alpha = (hop_dt / drift_tau).min(1.0);

            // Per-bin random target: XOR with KNUTH_GOLDEN-scaled bin index so
            // adjacent bins get statistically independent pseudo-random targets.
            // shift-by-7 idiom gives strict [-1, 1) range (see doc-comment above).
            let mixed = lfsr_step ^ (k as u32).wrapping_mul(KNUTH_GOLDEN);
            let centered = ((mixed as i32) >> 7) as f32 / ((1u32 << 24) as f32);

            // Hot bins (above temp_gate) drift further — positive feedback from upstream.
            // clamp(0.0, 4.0) sanitizes any NaN from upstream temperature (see guard doc).
            let temp = temp_in.map(|t| t[k].abs().clamp(0.0, 4.0)).unwrap_or(0.0);
            let temp_scale = if temp > temp_gate { 1.0 + (temp - temp_gate) } else { 1.0 };

            let target = centered * amount * temp_scale;
            lp_step(&mut drift_env[k], target, alpha);

            // Multiplicative gain: drift_env is a signed fractional offset around 1.0.
            let g = (1.0 + drift_env[k]).max(0.0);
            let dry = bins[k];
            let wet = dry * g;
            let mix = mix_c[k].clamp(0.0, 2.0) * 0.5;
            bins[k] = dry * (1.0 - mix) + wet * mix;
        }
    }

    // --- Pass 2: write temperature — drift activity heats bins (positive feedback). ---
    // `0.99` decay keeps the temperature from growing without bound; `clamp(0, 10)`
    // caps runaway in case many active bins all contribute simultaneously.
    if let Some(temp_out) = temperature.as_deref_mut() {
        for k in 0..num_bins {
            let activity = drift_env[k].abs() * 0.1;
            temp_out[k] = (temp_out[k] * 0.99 + activity).clamp(0.0, 10.0);
        }
    }
}

// ── CircuitMode ────────────────────────────────────────────────────────────

/// Per-channel power-sag envelope — energy rises above threshold drive a scalar
/// sag depth; hot bins (high `BinPhysics::temperature`) absorb more gain reduction
/// than cool bins. Reader-only: does not write any BinPhysics field.
///
/// AMOUNT/THRESHOLD/RELEASE drive a global per-channel envelope; the bin-0 sample
/// is taken as the canonical value rather than averaging — matches the user's mental
/// model of a single sag knob.
fn apply_power_sag(
    bins: &mut [Complex<f32>],
    sag_env: &mut f32,
    gain_red: &mut [f32],
    temperature: Option<&[f32]>,
    curves: &[&[f32]],
    hop_dt: f32,
) {
    use crate::dsp::circuit_kernels::lp_step;

    let amount_c  = curves[0];
    let thresh_c  = curves[1];
    // curves[2] = SPREAD: unused by Power Sag.
    let release_c = curves[3];
    let mix_c     = curves[4];

    let num_bins = bins.len();

    // --- 1. Compute total input energy (sum of magnitudes). Cheap proxy for power. ---
    let mut total_energy = 0.0_f32;
    for k in 0..num_bins {
        total_energy += bins[k].norm();
    }
    let energy_norm = total_energy / num_bins.max(1) as f32; // average per bin

    // --- 2. Update sag envelope: rises with energy above threshold, decays toward 0 below. ---
    // AMOUNT/THRESHOLD/RELEASE drive a global per-channel envelope, so the bin-0 sample
    // is taken as the canonical value rather than averaging — matches user's mental model
    // of a single sag knob.
    // Defensive `.get(0)` matches the file-wide pattern (see probe captures in `process()`):
    // pipeline always supplies full-length curves, but an empty slice from a future call
    // site shouldn't panic mid-audio-thread.
    let amount  = amount_c.get(0).copied().unwrap_or(0.0).clamp(0.0, 2.0) * 0.5;
    let thresh  = thresh_c.get(0).copied().unwrap_or(0.0).clamp(0.0, 4.0);
    let release = release_c.get(0).copied().unwrap_or(0.0).clamp(0.0, 2.0).max(0.01);
    let attack_tau  = 0.05;                        // 50 ms attack (sag onset)
    let release_tau = 0.5 * (0.1 + release);       // 50..1050 ms recovery
    let alpha_attack  = (hop_dt / attack_tau).min(1.0);
    let alpha_release = (hop_dt / release_tau).min(1.0);

    let drive = (energy_norm - thresh).max(0.0) * amount;
    let alpha = if drive > *sag_env { alpha_attack } else { alpha_release };
    lp_step(sag_env, drive, alpha);
    *sag_env = sag_env.clamp(0.0, 4.0);

    // --- 3. Per-bin gain reduction weighted by temperature. ---
    // `clamp(0.0, 4.0)` (not `.min(4.0)`): clamp sanitizes NaN to the lower bound, so a
    // single NaN-poisoned `temperature[k]` from an upstream writer can't propagate into
    // the smoothed gain and stick there forever (`f32::min` returns NaN against NaN).
    for k in 0..num_bins {
        let temp = temperature.map(|t| t[k].abs().clamp(0.0, 4.0)).unwrap_or(0.0_f32);
        // Hot bins absorb more sag. Reduction factor = 1 / (1 + sag * (1 + temp)).
        let target = 1.0 / (1.0 + *sag_env * (1.0 + temp));
        // Smooth the per-bin reduction to avoid hop-rate clicks.
        lp_step(&mut gain_red[k], target, alpha_attack);

        let dry = bins[k];
        let wet = dry * gain_red[k];
        let mix = mix_c[k].clamp(0.0, 2.0) * 0.5;
        bins[k] = dry * (1.0 - mix) + wet * mix;
    }
}

// ── CircuitMode ────────────────────────────────────────────────────────────

// ── PCB Crosstalk helpers ──────────────────────────────────────────────────

/// 3-tap spread stencil: bins leak into neighbours. SPREAD curve drives strength,
/// AMOUNT blends raw vs. spread, MIX controls dry/wet. No state — workspaces are
/// overwritten each hop, so no `set_circuit_mode()` reset is needed.
fn apply_pcb_crosstalk(
    bins: &mut [Complex<f32>],
    workspace: &mut [f32],
    workspace2: &mut [f32],
    curves: &[&[f32]],
) {
    use crate::dsp::circuit_kernels::spread_3tap;

    let amount_c = curves[0];
    // curves[1] = THRESHOLD: unused by PCB Crosstalk.
    let spread_c = curves[2];
    // curves[3] = RELEASE: unused by PCB Crosstalk.
    let mix_c = curves[4];

    let num_bins = bins.len();

    // 1. Read pass: copy magnitudes into workspace.
    for k in 0..num_bins {
        workspace[k] = bins[k].norm();
    }

    // 2. Average SPREAD strength over all bins (curves are smooth at hop rate).
    let spread_avg = if num_bins > 0 {
        let mut sum = 0.0_f32;
        for k in 0..num_bins {
            sum += spread_c[k].clamp(0.0, 2.0) * 0.5;
        }
        sum / num_bins as f32
    } else {
        0.0
    };

    // 3. Apply 3-tap spread into workspace2 (distinct slice — no aliasing).
    spread_3tap(&workspace[..num_bins], &mut workspace2[..num_bins], spread_avg);

    // 4. Write back: blend raw vs. spread by amount, then mix dry/wet.
    // For silent input bins where spread has leaked energy in, emit that energy
    // as a real-positive bin (arbitrary unit phase) — mirrors apply_transformer.
    for k in 0..num_bins {
        let amount = amount_c[k].clamp(0.0, 2.0) * 0.5;
        let mix = mix_c[k].clamp(0.0, 2.0) * 0.5;
        let in_mag = workspace[k];
        // amount blends raw magnitude vs. spread magnitude.
        let out_mag = workspace2[k] * amount + in_mag * (1.0 - amount);
        let dry = bins[k];
        let wet = if in_mag > 1e-9 {
            // Phase preserved: scale the existing complex bin.
            dry * (out_mag / in_mag)
        } else {
            // Silent input: emit spread energy as a real-positive bin.
            Complex::new(out_mag, 0.0)
        };
        bins[k] = dry * (1.0 - mix) + wet * mix;
    }
}

// ── Slew Distortion helpers ────────────────────────────────────────────────

/// Per-bin magnitude rate-limiter with excess-slew phase scramble.
///
/// Limits the delta-magnitude between consecutive hops to `rate_cap`. Any
/// magnitude change exceeding that cap is called "excess slew"; the excess is
/// converted into a random phase rotation proportional to `scramble_gain`.
///
/// Writes the per-bin rate cap into `slew_out` (when `Some`) so downstream
/// modules can read the active slew budget via `BinPhysics::slew`.
///
/// Curves: `[AMOUNT, THRESH, SPREAD(unused), RELEASE, MIX]`.
fn apply_slew_distortion(
    bins: &mut [Complex<f32>],
    prev_mag: &mut [f32],
    rng: &mut crate::dsp::circuit_kernels::SimdRng,
    slew_out: Option<&mut [f32]>,
    curves: &[&[f32]],
) {
    let amount_c  = curves[0];
    let thresh_c  = curves[1];
    // curves[2] = SPREAD: unused by Slew Distortion.
    let release_c = curves[3];
    let mix_c     = curves[4];

    let num_bins = bins.len();

    for k in 0..num_bins {
        let amount        = amount_c[k].clamp(0.0, 2.0) * 0.5;          // 0..1
        let rate_cap      = thresh_c[k].clamp(0.001, 4.0);               // max delta-mag per hop
        let scramble_gain = release_c[k].clamp(0.0, 2.0) * 0.5;          // 0..1
        let mix           = mix_c[k].clamp(0.0, 2.0) * 0.5;              // 0..1

        let dry = bins[k];
        let in_mag   = dry.norm();
        let in_phase = if in_mag > 1e-9 { dry.arg() } else { 0.0 };

        let prev = prev_mag[k];
        let delta   = in_mag - prev;
        // amount=0 → 0.5×rate_cap (gentle limiter); amount=1 → 1.0×rate_cap (full).
        let allowed = rate_cap * (0.5 + 0.5 * amount);

        let (capped_mag, excess) = if delta.abs() > allowed {
            let new_mag = prev + delta.signum() * allowed;
            (new_mag.max(0.0), delta.abs() - allowed)
        } else {
            (in_mag, 0.0)
        };
        prev_mag[k] = capped_mag;

        // Excess slew → random phase rotation proportional to the excess.
        let rand_centered = rng.next_f32_centered(); // strict [-1, 1)
        let phase_kick = rand_centered * excess * scramble_gain * std::f32::consts::PI;
        let new_phase  = in_phase + phase_kick;

        let wet = Complex::from_polar(capped_mag, new_phase);
        bins[k] = dry * (1.0 - mix) + wet * mix;
    }

    // Write the per-bin rate cap into the physics slew field so downstream
    // reader slots can see how tight the limiter was set.
    if let Some(sout) = slew_out {
        for k in 0..num_bins {
            sout[k] = thresh_c[k].clamp(0.001, 4.0);
        }
    }
}

// ── CircuitMode ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CircuitMode {
    BbdBins,
    SpectralSchmitt,
    CrossoverDistortion,
    Vactrol,
    TransformerSaturation,
    PowerSag,
    ComponentDrift,
    PcbCrosstalk,
    SlewDistortion,
}

impl Default for CircuitMode {
    fn default() -> Self {
        CircuitMode::CrossoverDistortion
    }
}

// ── CircuitModule ──────────────────────────────────────────────────────────

pub struct CircuitModule {
    mode: CircuitMode,
    bbd_mag: [[Vec<f32>; BBD_STAGES]; 2],   // bbd_mag[ch][stage][bin]
    schmitt_latched: [Vec<u8>; 2],           // packed bool per bin
    rng_state: [u32; 2],                     // xorshift32 per channel for BBD dither
    // Vactrol state: per-channel, per-bin fast/slow 1-pole caps.
    vactrol_fast: [Vec<f32>; 2],
    vactrol_slow: [Vec<f32>; 2],
    // Transformer state: magnitude one-pole smoother + SPREAD scratch.
    xfmr_lp:        [Vec<f32>; 2],
    xfmr_workspace:  [Vec<f32>; 2],
    // Power Sag state: per-channel scalar sag depth + per-bin smoothed gain reduction.
    sag_envelope:     [f32; 2],
    sag_gain_reduction: [Vec<f32>; 2],
    // Component Drift state: per-bin smoothed drift offset + per-channel LFSR.
    // `drift_rng` is NOT reset on FFT-size change — preserves the pseudo-random
    // trajectory across host-driven size switches so the drift doesn't restart audibly.
    drift_env: [Vec<f32>; 2],
    drift_rng: [crate::dsp::circuit_kernels::SimdRng; 2],
    // PCB Crosstalk state: per-channel magnitude scratch (workspace) and spread output
    // (workspace2). Both are overwritten each hop — no set_circuit_mode() reset needed.
    pcb_workspace:  [Vec<f32>; 2],
    pcb_workspace2: [Vec<f32>; 2],
    // Slew Distortion state: per-bin previous magnitude + per-channel PRNG.
    // `slew_rng` uses SimdRng for strict [-1, 1) bounds (see circuit_kernels doc).
    slew_prev_mag: [Vec<f32>; 2],
    slew_rng: [crate::dsp::circuit_kernels::SimdRng; 2],
    sample_rate: f32,
    fft_size: usize,
    #[cfg(any(test, feature = "probe"))]
    last_probe: crate::dsp::modules::ProbeSnapshot,
}

impl CircuitModule {
    pub fn new() -> Self {
        Self {
            mode: CircuitMode::default(),
            bbd_mag: [
                [Vec::new(), Vec::new(), Vec::new(), Vec::new()],
                [Vec::new(), Vec::new(), Vec::new(), Vec::new()],
            ],
            schmitt_latched: [Vec::new(), Vec::new()],
            rng_state: [0xDEAD_BEEFu32, 0xCAFE_BABEu32],
            vactrol_fast: [Vec::new(), Vec::new()],
            vactrol_slow: [Vec::new(), Vec::new()],
            xfmr_lp:       [Vec::new(), Vec::new()],
            xfmr_workspace: [Vec::new(), Vec::new()],
            sag_envelope:      [0.0, 0.0],
            sag_gain_reduction: [Vec::new(), Vec::new()],
            drift_env: [Vec::new(), Vec::new()],
            drift_rng: [
                crate::dsp::circuit_kernels::SimdRng::new(0xACED_DEAD),
                crate::dsp::circuit_kernels::SimdRng::new(0xFEED_FACE),
            ],
            pcb_workspace:  [Vec::new(), Vec::new()],
            pcb_workspace2: [Vec::new(), Vec::new()],
            slew_prev_mag: [Vec::new(), Vec::new()],
            slew_rng: [
                crate::dsp::circuit_kernels::SimdRng::new(0xBADF00D5),
                crate::dsp::circuit_kernels::SimdRng::new(0x0BADBEEF),
            ],
            sample_rate: 48_000.0,
            fft_size: 2048,
            #[cfg(any(test, feature = "probe"))]
            last_probe: crate::dsp::modules::ProbeSnapshot::default(),
        }
    }

    pub fn set_mode(&mut self, mode: CircuitMode) {
        if mode != self.mode {
            self.mode = mode;
            // Reset transient kernel state on mode change so kernels do not leak between modes.
            for ch in 0..2 {
                for stage in 0..BBD_STAGES {
                    for v in self.bbd_mag[ch][stage].iter_mut() {
                        *v = 0.0;
                    }
                }
                for v in self.schmitt_latched[ch].iter_mut() {
                    *v = 0;
                }
                for v in self.vactrol_fast[ch].iter_mut() {
                    *v = 0.0;
                }
                for v in self.vactrol_slow[ch].iter_mut() {
                    *v = 0.0;
                }
                for v in self.xfmr_lp[ch].iter_mut() {
                    *v = 0.0;
                }
                for v in self.xfmr_workspace[ch].iter_mut() {
                    *v = 0.0;
                }
                self.sag_envelope[ch] = 0.0;
                for v in self.sag_gain_reduction[ch].iter_mut() {
                    *v = 1.0;
                }
                for v in self.drift_env[ch].iter_mut() {
                    *v = 0.0;
                }
                // drift_rng is intentionally left intact: preserving the LFSR state
                // across mode changes avoids an audible restart of the drift trajectory.
                for v in self.slew_prev_mag[ch].iter_mut() {
                    *v = 0.0;
                }
                // slew_rng is intentionally left intact (same reasoning as drift_rng).
            }
        }
    }

    pub fn current_mode(&self) -> CircuitMode {
        self.mode
    }
}

impl SpectralModule for CircuitModule {
    fn process(
        &mut self,
        channel: usize,
        _stereo_link: StereoLink,
        _target: FxChannelTarget,
        bins: &mut [Complex<f32>],
        _sidechain: Option<&[f32]>,
        curves: &[&[f32]],
        suppression_out: &mut [f32],
        physics: Option<&mut crate::dsp::bin_physics::BinPhysics>,
        ctx: &ModuleContext,
    ) {
        debug_assert!(channel < 2);

        // Probe capture: all three kernels share the same mapping for curves[0] and curves[4].
        // curves[0] (AMOUNT): g=1.0 → 50%, g=2.0 → 100%  (g.clamp(0,2) * 50.0)
        // curves[4] (MIX):   g=1.0 → 50%, g=2.0 → 100%  (g.clamp(0,2) * 50.0)
        #[cfg(any(test, feature = "probe"))]
        let probe_amount_pct = curves[0].get(0).copied().unwrap_or(0.0).clamp(0.0, 2.0) * 50.0;
        #[cfg(any(test, feature = "probe"))]
        let probe_mix_pct = curves[4].get(0).copied().unwrap_or(0.0).clamp(0.0, 2.0) * 50.0;

        // Compute hop duration in seconds (variable FFT size).
        let hop_dt = (ctx.fft_size / 4) as f32 / ctx.sample_rate;

        match self.mode {
            CircuitMode::BbdBins => {
                let bbd = &mut self.bbd_mag[channel];
                let rng = &mut self.rng_state[channel];
                apply_bbd(bins, bbd, rng, curves);
            }
            CircuitMode::SpectralSchmitt => {
                let latched = &mut self.schmitt_latched[channel];
                apply_schmitt(bins, latched, curves);
            }
            CircuitMode::CrossoverDistortion => {
                apply_crossover(bins, curves);
            }
            CircuitMode::Vactrol => {
                // Read upstream flux via the writer-slot's mixed `physics`, not
                // `ctx.bin_physics`: Circuit declares `writes_bin_physics: true`,
                // so FxMatrix passes `physics = Some(&mut mix_phys)` and leaves
                // `ctx.bin_physics = None`. Vactrol does not write — `physics` is
                // read-only here.
                let flux: Option<&[f32]> = physics.as_ref().and_then(|bp| {
                    let f = &bp.flux[..];
                    if f.len() >= bins.len() { Some(&f[..bins.len()]) } else { None }
                });
                let fast = &mut self.vactrol_fast[channel];
                let slow = &mut self.vactrol_slow[channel];
                apply_vactrol(bins, fast, slow, curves, hop_dt, flux);
            }
            CircuitMode::TransformerSaturation => {
                // As a writer slot, `physics = Some(&mut mix_phys)` from FxMatrix;
                // `ctx.bin_physics = None`. We read flux (pass 1 bias) and write
                // updated flux (pass 3) through the same `Option<&mut [f32]>` to
                // avoid a borrow-checker split on the same Vec.
                let nb = ctx.num_bins;
                let flux: Option<&mut [f32]> = if let Some(bp) = physics {
                    if bp.flux.len() >= nb { Some(&mut bp.flux[..nb]) } else { None }
                } else {
                    None
                };
                let lp  = &mut self.xfmr_lp[channel][..nb];
                let ws  = &mut self.xfmr_workspace[channel][..nb];
                apply_transformer(&mut bins[..nb], lp, ws, flux, curves, hop_dt);
            }
            CircuitMode::PowerSag => {
                // Read upstream temperature via the writer-slot's mixed `physics`,
                // not `ctx.bin_physics`: Circuit declares `writes_bin_physics: true`,
                // so FxMatrix passes `physics = Some(&mut mix_phys)` and leaves
                // `ctx.bin_physics = None`. Power Sag is reader-only — it does not
                // write any BinPhysics field.
                let nb = ctx.num_bins;
                let temp: Option<&[f32]> = physics.as_ref().and_then(|bp| {
                    let t = &bp.temperature[..];
                    if t.len() >= nb { Some(&t[..nb]) } else { None }
                });
                apply_power_sag(
                    &mut bins[..nb],
                    &mut self.sag_envelope[channel],
                    &mut self.sag_gain_reduction[channel][..nb],
                    temp,
                    curves,
                    hop_dt,
                );
            }
            CircuitMode::ComponentDrift => {
                // As a writer slot, `physics = Some(&mut mix_phys)` from FxMatrix;
                // `ctx.bin_physics = None`. We read temperature (pass 1: hot-bin scale)
                // and write updated temperature (pass 2: drift activity heats bins)
                // through a single `Option<&mut [f32]>` to avoid a borrow-checker split
                // on the same Vec — mirrors the `apply_transformer` writer-slot pattern.
                let nb = ctx.num_bins;
                let temperature: Option<&mut [f32]> = if let Some(bp) = physics {
                    if bp.temperature.len() >= nb { Some(&mut bp.temperature[..nb]) } else { None }
                } else {
                    None
                };
                apply_component_drift(
                    &mut bins[..nb],
                    &mut self.drift_env[channel][..nb],
                    &mut self.drift_rng[channel],
                    temperature,
                    curves,
                    hop_dt,
                );
            }
            CircuitMode::PcbCrosstalk => {
                // PCB Crosstalk does not read or write BinPhysics.
                let _ = physics;
                let nb = ctx.num_bins;
                // Rebind both workspace vecs as locals before the call so the borrow
                // checker sees two distinct &mut borrows (one from pcb_workspace,
                // one from pcb_workspace2). These are separate Vec allocations so their
                // slices are guaranteed non-overlapping — no aliasing.
                let ws  = &mut self.pcb_workspace[channel][..nb];
                let ws2 = &mut self.pcb_workspace2[channel][..nb];
                apply_pcb_crosstalk(&mut bins[..nb], ws, ws2, curves);
            }
            CircuitMode::SlewDistortion => {
                // As a writer slot, `physics = Some(&mut mix_phys)` from FxMatrix;
                // `ctx.bin_physics = None`. We write the per-bin rate cap into
                // `physics.slew` so downstream reader slots can see the active budget.
                // Pattern mirrors ComponentDrift: `if let Some(bp) = physics` consumes
                // the Option, yielding a `&mut BinPhysics` without reborrowing issues.
                let nb = ctx.num_bins;
                let slew_out: Option<&mut [f32]> = if let Some(bp) = physics {
                    if bp.slew.len() >= nb { Some(&mut bp.slew[..nb]) } else { None }
                } else {
                    None
                };
                apply_slew_distortion(
                    &mut bins[..nb],
                    &mut self.slew_prev_mag[channel][..nb],
                    &mut self.slew_rng[channel],
                    slew_out,
                    curves,
                );
            }
        }

        for s in suppression_out.iter_mut() {
            *s = 0.0;
        }

        #[cfg(any(test, feature = "probe"))]
        {
            self.last_probe = crate::dsp::modules::ProbeSnapshot {
                amount_pct: Some(probe_amount_pct),
                mix_pct:    Some(probe_mix_pct),
                ..Default::default()
            };
        }
    }

    fn reset(&mut self, sample_rate: f32, fft_size: usize) {
        self.sample_rate = sample_rate;
        self.fft_size = fft_size;
        let num_bins = fft_size / 2 + 1;
        for ch in 0..2 {
            for stage in 0..BBD_STAGES {
                self.bbd_mag[ch][stage].clear();
                self.bbd_mag[ch][stage].resize(num_bins, 0.0);
            }
            self.schmitt_latched[ch].clear();
            self.schmitt_latched[ch].resize(num_bins, 0);
            self.vactrol_fast[ch].clear();
            self.vactrol_fast[ch].resize(num_bins, 0.0);
            self.vactrol_slow[ch].clear();
            self.vactrol_slow[ch].resize(num_bins, 0.0);
            self.xfmr_lp[ch].clear();
            self.xfmr_lp[ch].resize(num_bins, 0.0);
            self.xfmr_workspace[ch].clear();
            self.xfmr_workspace[ch].resize(num_bins, 0.0);
            self.sag_envelope[ch] = 0.0;
            self.sag_gain_reduction[ch].clear();
            self.sag_gain_reduction[ch].resize(num_bins, 1.0); // 1.0 = no reduction
            self.drift_env[ch].clear();
            self.drift_env[ch].resize(num_bins, 0.0);
            // drift_rng is NOT reset on FFT-size change (preserves LFSR trajectory).
            self.pcb_workspace[ch].clear();
            self.pcb_workspace[ch].resize(num_bins, 0.0);
            self.pcb_workspace2[ch].clear();
            self.pcb_workspace2[ch].resize(num_bins, 0.0);
            self.slew_prev_mag[ch].clear();
            self.slew_prev_mag[ch].resize(num_bins, 0.0);
            // slew_rng is NOT reset on FFT-size change (preserves PRNG trajectory).
        }
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::Circuit
    }

    fn set_circuit_mode(&mut self, mode: CircuitMode) {
        self.set_mode(mode);
    }

    fn num_curves(&self) -> usize {
        5
    }

    #[cfg(any(test, feature = "probe"))]
    fn last_probe(&self) -> crate::dsp::modules::ProbeSnapshot { self.last_probe }
}

// ── CircuitProbe (test / probe builds only) ────────────────────────────────

/// Per-module probe snapshot for Circuit. Returned by `probe_state()`.
#[cfg(any(test, feature = "probe"))]
#[derive(Debug, Clone, Copy)]
pub struct CircuitProbe {
    pub active_mode:       CircuitMode,
    pub vactrol_fast_avg:  f32,
    pub vactrol_slow_avg:  f32,
    /// Average magnitude one-pole smoother value across all bins.
    /// Non-zero only in `TransformerSaturation` mode.
    pub xfmr_lp_avg:      f32,
    /// Per-channel scalar sag depth. Non-zero only in `PowerSag` mode.
    pub sag_envelope:     f32,
    /// Mean absolute drift offset across all bins. Non-zero only in `ComponentDrift` mode.
    pub drift_env_avg:    f32,
}

#[cfg(any(test, feature = "probe"))]
impl CircuitModule {
    pub fn probe_state(&self, channel: usize) -> CircuitProbe {
        let ch = channel.min(1);
        let (fa, sa) = if self.mode == CircuitMode::Vactrol && !self.vactrol_slow[ch].is_empty() {
            let n = self.vactrol_slow[ch].len() as f32;
            let fa: f32 = self.vactrol_fast[ch].iter().sum::<f32>() / n;
            let sa: f32 = self.vactrol_slow[ch].iter().sum::<f32>() / n;
            (fa, sa)
        } else {
            (0.0, 0.0)
        };
        let xfmr_lp_avg = if self.mode == CircuitMode::TransformerSaturation
            && !self.xfmr_lp[ch].is_empty()
        {
            self.xfmr_lp[ch].iter().sum::<f32>() / self.xfmr_lp[ch].len() as f32
        } else {
            0.0
        };
        let sag_envelope = if self.mode == CircuitMode::PowerSag {
            self.sag_envelope[ch]
        } else {
            0.0
        };
        let drift_env_avg = if self.mode == CircuitMode::ComponentDrift
            && !self.drift_env[ch].is_empty()
        {
            self.drift_env[ch].iter().map(|v| v.abs()).sum::<f32>()
                / self.drift_env[ch].len() as f32
        } else {
            0.0
        };
        CircuitProbe {
            active_mode:      self.mode,
            vactrol_fast_avg: fa,
            vactrol_slow_avg: sa,
            xfmr_lp_avg,
            sag_envelope,
            drift_env_avg,
        }
    }
}
