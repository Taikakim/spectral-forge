# Research: SIMD-Friendly Per-Bin Analog Component Models

**Source prompt:** 7 in `90-research-prompts.md`
**Status:** Findings as of 2026-04-26

## Refined research questions

The original prompt asked five specific questions about cheap-to-implement,
accurate-enough numerical schemes for eight analog component models, with
explicit AVX2/AVX-512 vectorisation patterns. After surveying the literature
and existing open-source kernels the prompt refines into the following
research targets:

1. **Math kernel selection.** Which polynomial / rational / rsqrt-based
   approximations of `tanh`, `exp`, `sin`, `log` are simultaneously
   (a) fast on a vector unit, (b) accurate enough for audio, and
   (c) phase-coherent enough not to introduce harmonic mismatches between
   adjacent bins?
2. **SIMD substrate.** Which Rust SIMD library survives 2026-era stable
   toolchain constraints while delivering AVX2 / AVX-512 / NEON dispatch
   without sacrificing portable-fallback compilation?
3. **Per-component cheapest-correct schemes.** For each of the eight
   sub-effects in the Circuit module audit (`10-circuit.md`), what is the
   minimum-state-and-arithmetic recipe that captures the audible
   "analog imperfection" character? "Audible character" trumps physical
   fidelity — these are colour effects, not modelling restoration.
4. **Spectral-rate vs sample-rate translation.** Most analog modelling
   literature operates per audio sample. We operate per STFT hop
   (factor of 128 to 1024 lower update rate). Which models translate
   cleanly to a hop-rate update, and which require either oversampling
   per hop or moving the model to the time domain altogether?
5. **State-of-array shared math.** When eight sub-effects share Structure-
   of-Arrays state (`Vec<f32>` per field), which kernels can be batched
   into a single AVX-512 fused-multiply-add tape and which need to stay
   separate?

The deliverable is a per-component recipe + a chosen substrate, not a
single new algorithm.

## SIMD math primitives in Rust

### Polynomial approximations for the inner-loop forbiddens

The prohibition on `tanh()`, `exp()`, `sinf()`, `logf()` inside per-bin
loops is well established in the audio DSP community. The community
standards (and the libraries that pre-bake them) are:

#### tanh

The single most popular cheap polynomial is **Levien's tanh approximation**
(from Raph Levien's [favourite-sigmoids blog post][levien]):

```rust
#[inline]
pub fn tanh_levien(x: f32) -> f32 {
    let x2 = x * x;
    let x3 = x2 * x;
    let x5 = x3 * x2;
    let a = x + (0.16489087 * x3) + (0.00985468 * x5);
    a / (1.0 + (a * a)).sqrt()
}
```

Accuracy: 2e-4 maximum error.
Speed: 0.55 ns/sample on Levien's test hardware.
SIMD: trivially vectorisable — the inner sequence is six muls, two adds,
one rsqrt. Both AVX2 and AVX-512 expose `vrsqrtps` /`vrsqrt14ps`.

Levien's blog ([raphlinus.github.io][levien]) also documents an even
cheaper variant `x / sqrt(1 + x*x)` at 0.453 ns/sample — same
shape, fewer ops, but slightly more harmonic distortion than tanh.
For our colour-effect use case the cheaper variant is fine when the
saturator sits inside the Transformer or BiasFuzz path; the Levien
form is preferable when the saturator dominates the audible signature
(e.g. Transformer flux SoA pass).

The classic [musicdsp.org rational tanh][rationaltanh]:

```c
return x * (27.0 + x*x) / (27.0 + 9.0 * x*x);
```

Maximum error ~2.6 % in the ±4.5 range. Fastest scalar option, but the
implied reciprocal (`vrcp14ps`) raises the noise floor about 1 LSB ahead
of bit-true division — fine for colour saturators, marginal for a clean
makeup-gain stage.

The [musicdsp.org Padé tanh(x/2) form][padetanh]:

```
a = 6 + x*(6 + x*(3 + x))
tanh(x/2) ≈ (a - 6) / (a + 6)
```

Greatest deviation about 5 % at ±1.46. Cheap (4 muls, 4 adds, 1 div).

#### exp

The community standard is from [musicdsp.org "Fast exp() approximations"][fastexp].
The fastest 32-bit variant uses bit-twiddling on the IEEE-754 layout to
get the exponent for free, then a degree-3 polynomial for the mantissa
correction. SLEEF's `expf_u10` is the gold-standard vectorised reference
([sleef.org][sleef], [arXiv:2001.09258][sleefarxiv]), with 1 ULP and
3.5 ULP accuracy variants and AVX2/AVX-512/NEON dispatch — about 8-12 ns
per scalar exp depending on width.

For our hop-rate work, the typical `exp` use is the per-bin envelope
release coefficient, computed once per hop:

```
release_coef = exp(-1.0 / (sample_rate * release_seconds))
```

This can be replaced by `exp(-x) ≈ 1 / (1 + x + 0.5*x*x)` for small `x`,
which is a 2-mul, 2-add plus a `vdivps`. Even cheaper if we precompute
release_coef on the GUI thread and pass it through the curve cache —
which the existing `apply_curve_transform` pipeline already does via the
THRESHOLD/RELEASE/etc. curves.

#### sin

For sin/cos at audio rates, two strategies dominate:
1. **Quadratic parabolic approximation**:
   `sin(x) ≈ (4/π) * x - (4/π²) * x * |x|` for `x ∈ [-π, π]`.
   Maximum error ~5.6 %. Used by Devine when bin-by-bin mod-frequency is
   approximate (e.g. the Modulate `Ground Loop` mode).
2. **Lookup table + linear interp.** A 256-entry table of `sin(x)` plus
   linear interpolation gives 1e-4 accuracy at one gather + one mul + one
   add + one mask. AVX2/AVX-512 hardware gather (`vpgatherdps`) can issue
   8/16 lookups per instruction. Best when the input range is bounded.

Note: hardware gather on AVX-512 is faster than on AVX2 (the AVX2
implementation serialises through the L1 cache port) but is still ~2x
slower than a vector load — only worth it when the LUT is small enough
to live in L1 and the address pattern is non-sequential.

#### log

For `log()` — used in dB-domain envelope tracking — the IEEE-754
exponent extraction trick gives free `log2` to ~6-bit precision; one
polynomial step (degree 5) refines to 24-bit. SLEEF's `logf_u35` is the
reference SIMD implementation. The chowdsp_wdf C++ library uses the
xsimd library's `log_approx` with similar performance characteristics
([github.com/Chowdhury-DSP/chowdsp_wdf][chowdspwdf]).

For our use case `log` is mostly **avoided** by working in linear gain
space and doing the dB conversion only at GUI / spectrum-display time.
The existing `params.threshold_db_to_linear()` path already does this.

#### Composite recommendation

The Circuit module's eight sub-effects need:
- Vactrol: 1 exp call per hop per bin (release coef) → precompute or
  use small-`x` polynomial.
- Schmitt: zero transcendentals (compare + mask).
- Transformer flux: 1 tanh per hop per bin → Levien.
- BBD: 1 lerp per stage per hop per bin → just a mul + add, no
  transcendental.
- Power Sag: 1 RMS sqrt per hop, global → not per bin.
- Component Drift: 1 PRNG step per hop per bin → vectorised xorshift.
- PCB Crosstalk: zero transcendentals (linear convolution).
- Slew distortion: zero transcendentals (clamp + delta).

Total: 1 cheap tanh and 1 cheap PRNG call per bin, plus state updates.
Easily inside the budget for a per-hop kernel at 8193 bins.

### Rust SIMD libraries comparison

| Library | Status | API style | Cross-arch | Multiversioning | License | Notes |
|---|---|---|---|---|---|---|
| [`std::simd`][stdsimd] | nightly | portable, `Simd<T,N>` | AVX, AVX-512, NEON, WASM | needs the `multiversion` crate | BSD/MIT | Best portability; nightly-only blocker. |
| [`wide`][wide] | stable | trait-based, `f32x4`, `f32x8` | x86, NEON, WASM | none built-in | Zlib | Mature; powers FunDSP. |
| [`pulp`][pulp] | stable | high-level, batched | AVX2, AVX-512, NEON | built-in runtime dispatch | MIT/Apache-2 | Powers `faer` linear algebra. |
| [`fearless_simd`][fearlesssimd] | active dev | high-level | NEON, WASM, SSE4.2 | partial | MIT/Apache-2 | Linebender's revival; not yet covering newer x86 widths. |
| [`safe_arch`][safearch] | stable | thin wrapper over intrinsics | x86, NEON | manual | Zlib | Used internally by `wide`. |
| [`xsimd`][xsimd] (C++) | stable | not Rust | AVX2, AVX-512, NEON | built-in | BSD-3 | Reference for what a mature library looks like; powers chowdsp. |

Sources for this comparison: Sergey Davidoff's
["State of SIMD in Rust in 2025"][shnatsel] and Raph Levien's
["Towards fearless SIMD, 7 years later"][fearlessSimd2025].

### Recommendation

**Pick `wide` for v1, `pulp` for v2 if runtime-dispatch becomes
necessary.** Specific reasoning:

1. `wide` is what `FunDSP` already uses ([github.com/SamiPerttu/fundsp][fundsp]).
   It is stable, mature, has Zlib licence (compatible with our GPL eventual
   release), and exposes `f32x8` directly — which is the natural width for
   AVX2 (8 lanes of f32) and a half-width slice of AVX-512.
2. `pulp` is the library to graduate to once we want one binary that
   detects AVX-512 at runtime and falls back to AVX2 / SSE / scalar
   automatically. The `faer` numerics library is the production proof of
   this (and the most likely future audience for an open-sourced version
   of the kernels).
3. `std::simd` would be technically the cleanest API but locks the build
   to `nightly`. Bitwig / Reaper / FL users on Windows install pre-built
   binaries; we cannot tie release builds to a nightly compiler.
4. `fearless_simd` is too immature for production right now (no
   AVX2-with-FMA path) but is worth tracking — Linebender's track record
   on hardware abstractions (Vello, Druid) is strong.
5. Either choice keeps us out of the `assert_process_allocs` trap because
   none of these libraries allocate.

For per-bin processing of 8193 bins, the loop pattern with `wide` is:

```rust
use wide::f32x8;
const LANES: usize = 8;

let mut k = 0;
while k + LANES <= num_bins {
    let mag  = f32x8::from(&mag_arr[k..k+LANES]);
    let prev = f32x8::from(&state_arr[k..k+LANES]);
    let coef = f32x8::from(&coef_arr[k..k+LANES]);

    let new = prev + (mag - prev) * coef;
    state_arr[k..k+LANES].copy_from_slice(new.as_array_ref());

    k += LANES;
}
// Tail loop handles the last <8 bins scalarly. With NUM_BINS = 8193,
// 8193 = 1024*8 + 1, so only one tail iteration.
```

Most of the kernels in this document follow that pattern.

## Per-component analysis

### 1. Vactrol (photoresistor)

**Reference papers:**
- Parker & D'Angelo, "A Digital Model of the Buchla Lowpass-Gate," DAFx-13
  ([dafx.de/paper-archive/2013/papers/44.dafx2013_submission_56.pdf][parker2013]).
  Describes a three-segment vactrol model: LED current → resistance via
  exponential coupling, resistance → smoothed-output via a one-pole lowpass
  whose cutoff *is* the resistance. The release path is the audibly
  important non-trivial bit: real vactrols recover in two stages — a fast
  millisecond-scale snap, then a slow hundreds-of-milliseconds tail.
- Najnudel, Falaize, Hélie et al., "Power-balanced dynamic modelling of
  vactrols," DAFx-23
  ([dafx.de/paper-archive/2023/DAFx23_paper_50.pdf][najnudel2023]).
  Port-Hamiltonian formulation of the LED↔photoresistor coupling, with
  parameters fit against measurements of real vactrols. Heavier than
  Parker's model but provides a passive-energy guarantee that Parker's
  doesn't.
- Randy Jones (Madrona Labs) Aalto: a "multistage low-pass filter that
  emulates the response of a Vactrol-style opto-isolator"
  ([cdm.link/madronas-randy-jones-on-aalto][cdmAalto]). Black-box
  description but Madrona's release describes a chain of 1-pole LPs with
  per-stage time constants tuned by ear.

**State of the art:**
The literature converges on a two-time-constant release: an attack
modelled by a fast 1-pole LP (~5 ms typical) and a release modelled by
either a switched cascade of 1-pole LPs with widely-spaced time
constants (Madrona heuristic; Parker analytical) or an explicitly
modelled photo-conductor curve (Najnudel power-balanced).

For colour effects on per-bin envelopes, the audible signature is the
release "ringing" character — the plateau where the slow tail catches
the fast tail. Two cascaded 1-poles with τ_fast = 8 ms and
τ_slow = 250 ms is the cheapest convincing approximation.

**Cheap kernel proposal:**
```rust
// Per-bin state: vactrol_fast[k], vactrol_slow[k]
// Per-block precomputed: alpha_attack, alpha_fast_release, alpha_slow_release
// (these are exp(-hop_seconds / tau), one scalar each — not per bin)
//
// In-loop: take target = current bin magnitude envelope.
fn vactrol_step_simd(
    target: &[f32], fast: &mut [f32], slow: &mut [f32], out: &mut [f32],
    a_attack: f32, a_fast_rel: f32, a_slow_rel: f32,
) {
    // Decide attack vs release per-bin via mask.
    let aa = f32x8::splat(a_attack);
    let af = f32x8::splat(a_fast_rel);
    let asl = f32x8::splat(a_slow_rel);
    let mut k = 0;
    while k + 8 <= target.len() {
        let t = f32x8::from(&target[k..k+8]);
        let f = f32x8::from(&fast[k..k+8]);
        let s = f32x8::from(&slow[k..k+8]);
        let attacking = t.cmp_gt(f); // mask
        // Attack: 1-pole LP toward t with alpha_attack.
        // Release: cascade 2 LPs with two different alphas.
        let f_attack = f + (t - f) * aa;
        let f_release = f * af; // decay toward 0
        let f_new = attacking.blend(f_attack, f_release);

        let s_attack = s + (f_new - s) * aa;
        let s_release = s * asl;
        let s_new = attacking.blend(s_attack, s_release);

        // Output = sum of fast and slow (or product, depending on taste).
        let out_v = f_new + s_new;

        f_new.write_to_slice_unaligned(&mut fast[k..k+8]);
        s_new.write_to_slice_unaligned(&mut slow[k..k+8]);
        out_v.write_to_slice_unaligned(&mut out[k..k+8]);
        k += 8;
    }
    // Scalar tail.
}
```
Cost per bin: ~10 floating-point ops, two state slots. For a slot
with all 8193 bins active that is ~80 kFLOP per hop, well inside budget.

The audible "ringing" character emerges naturally from the gap between
the fast and slow time constants. Tune `tau_fast` and `tau_slow` from
the RELEASE curve (one per bin) for character variation.

### 2. Schmitt trigger

**Reference papers:**
- Wikipedia [Schmitt trigger][wikischmitt] is sufficient for the math.
- The audio-DSP community discussion at
  [dsprelated.com][dsprelschmitt] confirms the standard branch-free
  pattern: store one bool of state per channel, compare with two
  thresholds, update with mask logic.

**State of the art:**
There is no academic literature specific to per-bin spectral Schmitt
triggers — the construction is too primitive. The implementation
question is purely: how to vectorise across 8193 bins without branching.

**Cheap kernel proposal:**
```rust
fn schmitt_step_simd(
    mag: &[f32], state: &mut [u8], out: &mut [f32],
    on_thresh: f32, off_thresh: f32,
) {
    let on_v = f32x8::splat(on_thresh);
    let off_v = f32x8::splat(off_thresh);
    let mut k = 0;
    while k + 8 <= mag.len() {
        let m = f32x8::from(&mag[k..k+8]);
        // Read 8 bytes, expand to 8x i32, then to 8x f32 for blending.
        // Equivalent: treat each bool as a mask in i32x8.
        let prev_state = i32x8::from_array([
            state[k] as i32, state[k+1] as i32, state[k+2] as i32, state[k+3] as i32,
            state[k+4] as i32, state[k+5] as i32, state[k+6] as i32, state[k+7] as i32,
        ]);
        let was_on = prev_state.cmp_eq(i32x8::splat(1));
        let cross_off = m.cmp_lt(off_v);
        let cross_on = m.cmp_gt(on_v);
        // New state: was_on AND NOT cross_off, OR (NOT was_on) AND cross_on.
        let now_on = (was_on & !cross_off) | (!was_on & cross_on);
        // Output: m if on, 0 if off.
        let out_v = now_on.blend(m, f32x8::splat(0.0));

        let new_state = now_on.to_int_mask(); // 0 or 1
        for i in 0..8 { state[k+i] = ((new_state >> i) & 1) as u8; }
        out_v.write_to_slice_unaligned(&mut out[k..k+8]);
        k += 8;
    }
}
```
This is the canonical "branch-free Schmitt with mask blend." On AVX2 the
mask is a `__m256` of all-1s/all-0s; on AVX-512 it's a real `kmask`
register, even smaller. State storage is `u8` per bin (8193 bytes total,
fits in 2 cache lines per slot — really 129 cache lines, but accessed
sequentially so prefetcher handles it).

### 3. Transformer flux saturation

**Reference papers:**
- Holters & Lambeth, "Modelling Audio Transformers Using Volterra Series"
  was cited in the brainstorm. While I could not retrieve a clean copy of
  that specific paper, the AES/DAFx Volterra-series literature is well-
  surveyed in
  [arxiv:2308.07229][volterracategorical] (compositional Volterra) and
  in the Yeh thesis ([ccrma.stanford.edu/~dtyeh][yehthesis]).
- Jiles–Atherton hysteresis model ([Wikipedia][wikija];
  [Jiles & Atherton 1984][royalsocja]) is the workhorse for transformer
  flux. Jatin Chowdhury implemented it for tape ([Medium "Complex
  Nonlinearities Episode 3: Hysteresis"][chowhysteresis];
  [DAFx writeup][chowtapedafx]) using a Trapezoidal-Rule discretisation
  with RK2 / RK4 / Newton-Raphson solver options. SIMD speedups of 38-63%
  measured ([Medium "Faster Tape Emulation with SIMD"][chowsimd]).
- For audio purposes, Volterra series is overkill and Jiles-Atherton is
  borderline: the audible signature of a transformer is mostly soft
  clipping (tanh-like) plus a memory-of-direction term (the hysteresis
  loop's width).

**State of the art:**
Three options stack up:
1. **Pure tanh.** Cheap, no memory. Fine if the only target is "warm
   colour."
2. **Tanh + magnitude one-pole.** Adds a low-frequency memory term. Two
   ops per bin. Captures most of the audible "transformer thickness."
3. **Full Jiles-Atherton.** Captures hysteresis loop width and history-
   dependent harmonic content. ~50 FLOPs per bin per RK2 step. Overkill
   for spectral colour, appropriate if the transformer is the
   plugin's signature.

For the Circuit module's spectral context, **option 2 wins**. We are
already operating per-bin per-hop, so the "memory of direction" effect
is a one-pole LP on the bin's magnitude envelope. Combined with tanh
saturation and a SPREAD coupling term to neighbours (per the
`10-circuit.md` audit refinement), this gives a convincing
"transformer flux that leaks to neighbours" character.

**Cheap kernel proposal:**
```rust
fn transformer_flux_simd(
    mag: &[f32], flux_lp: &mut [f32], out: &mut [f32],
    drive: f32, alpha: f32,
) {
    let drive_v = f32x8::splat(drive);
    let alpha_v = f32x8::splat(alpha);
    let one = f32x8::splat(1.0);
    let mut k = 0;
    while k + 8 <= mag.len() {
        let m = f32x8::from(&mag[k..k+8]);
        let lp = f32x8::from(&flux_lp[k..k+8]);
        // Hysteresis-like memory: previous LP biases the saturator centre.
        let centred = (m - lp) * drive_v;
        // Levien tanh.
        let x2 = centred * centred;
        let x3 = centred * x2;
        let x5 = x3 * x2;
        let a = centred + x3 * f32x8::splat(0.16489087)
                       + x5 * f32x8::splat(0.00985468);
        let denom = (one + a*a).sqrt();
        let sat = a / denom;
        // Update LP: alpha = 1 - exp(-hop / tau).
        let new_lp = lp + (m - lp) * alpha_v;
        new_lp.write_to_slice_unaligned(&mut flux_lp[k..k+8]);
        // Output: original mag scaled by saturator response, plus DC bias.
        let out_v = sat + lp;
        out_v.write_to_slice_unaligned(&mut out[k..k+8]);
        k += 8;
    }
}
```
For SPREAD between neighbours, a separate two-pass kernel that reads
`flux_lp[k-1]`, `flux_lp[k]`, `flux_lp[k+1]` from a previous-pass
buffer and writes back to a new-pass buffer (per the audit doc's
ordering rule). This costs an extra read-multiply-add per bin.

### 4. BBD (bucket-brigade)

**Reference papers:**
- Holters & Parker, "A Combined Model for a Bucket Brigade Device and
  its Input and Output Filters," DAFx-18
  ([hsu-hh.de PDF][holtersbbd]). Variable-sample-rate model with
  surrounding RC filters folded in to avoid the interpolation usually
  needed for BBD time variation.
- Raffel & Smith, "Practical Modeling of Bucket-Brigade Device Circuits,"
  DAFx-10 ([colinraffel.com][raffelbbd]). Earlier paper that lays out
  the standard 4-stage cascade with per-stage charge-loss and dither-
  injection model.
- The classic Panasonic chips (MN3007 family) carry up to 4096 stages —
  per-bin emulation at that depth is not realistic. The Circuit
  brainstorm proposes 4 stages per bin, which is a colour effect, not a
  delay effect.

**State of the art:**
A real BBD's audible signature comes from:
1. Per-stage charge-loss low-pass (each "bucket" leaks a bit of its
   charge → short-time exponential smoothing).
2. Sample-and-hold zero-order-hold artefacts (clock-rate aliasing).
3. Dither injected at input (small noise above the audio band, mostly
   used to randomise the quantisation).

For a per-bin, per-hop colour effect, items 1 and 3 matter; item 2 is
swallowed by the STFT itself. Each bin gets a 4-stage SoA cascade plus
one additive dither term per stage.

**Cheap kernel proposal:**
```rust
// State: bbd_stage[STAGES][MAX_NUM_BINS] = 4 × 8193 × 4 = 130 KB per BBD.
fn bbd_step_simd(
    input: &[f32],
    stages: &mut [Vec<f32>; 4],
    out: &mut [f32],
    leak: f32,            // 1 - per_stage_charge_loss; <1 (e.g. 0.985)
    dither_lvl: f32,
    rng: &mut SimdRng,
) {
    let leak_v = f32x8::splat(leak);
    let dither_v = f32x8::splat(dither_lvl);
    let mut k = 0;
    while k + 8 <= input.len() {
        let mut x = f32x8::from(&input[k..k+8]);
        for s in 0..4 {
            let prev = f32x8::from(&stages[s][k..k+8]);
            // Add per-stage dither (cheap LFSR — see Component Drift section).
            let dither = rng.next_f32x8() * dither_v;
            // Each stage: y = prev * leak + (x - prev * leak) * 1.
            // Equivalently, this stage outputs the previous bucket's value
            // and stores x as its new value. Simplest "cascade hold" model.
            let new = x + dither;
            new.write_to_slice_unaligned(&mut stages[s][k..k+8]);
            x = prev * leak_v;
        }
        x.write_to_slice_unaligned(&mut out[k..k+8]);
        k += 8;
    }
}
```
Per-stage cost: 1 mul, 1 add, 1 PRNG call, 8x parallel.
Total per bin: ~12 FLOPs + 4 PRNG steps.
Memory: 130 KB per BBD instance, in 4 sequentially accessed stage
arrays — fits comfortably in L2 on any modern CPU.

**On the dither LFSR question:** the prompt asked whether we can share
one LFSR across bins. Answer: **no** for a 4-stage cascade, **yes** if
all bins share one stream that is pulled in stride-1 fashion. The
audible difference is whether the dither correlates across bins — a
single shared LFSR pulled stride-1 *appears* uncorrelated to the ear
because each bin gets a different draw. The SIMDxorshift pattern
([github.com/lemire/SIMDxorshift][simdxorshift]) generates 8 fresh
draws per AVX2 step at ~1.88 cycles/draw, so per-bin per-stage dither
is essentially free.

The choice that matters: **the LFSR seed must be different per stage**
to avoid cross-stage correlation that creates audible periodic patterns.
Use `WyRand` or `Xoshiro256+` with stage-specific seeds.

### 5. Power supply sag

**Reference papers:**
- AmpBooks "Digital Modeling of a Guitar Amplifier Power Supply"
  ([ampbooks.com/mobile/dsp/power-supply][ampbooks]). Introduces the
  rectifier-tube voltage drop, transformer winding resistance, and
  filter-capacitor sizing as the three primary sag sources.
- Resonant DSP "Swanky Amp" sag implementation
  ([kvraudio.com forum thread][swankyampsag]) describes a SPICE-based
  fit reduced to a per-sample ODE — fundamentally a one-pole LP whose
  cutoff frequency is current-dependent.
- Neural DSP and Kemper-style commercial models capture sag as part of
  end-to-end NN training; not a useful reference for hand-coded models.

**State of the art:**
Power supply sag is a **global** effect, not a per-bin effect. The rail
voltage drops as total programme power increases; this scales the
output of every bin by the same factor. The audible signature is
breathing/pumping that is correlated across the spectrum.

For the Circuit module's `Power Sag` mode, the implementation is:
1. One global RMS estimate of the input spectrum (sum of mag² over all
   bins, smoothed with one-pole LP).
2. The smoothed RMS drives a "rail voltage" estimate, e.g.
   `rail = 1.0 - sag_amount * (rms / rms_threshold).clamp(0, 1)`.
3. All bins are scaled by `rail`.

The audit doc's refinement (`10-circuit.md` § e) extends this with a
per-bin temperature term: hot bins (sustained energy) contribute more
sag than cool bins. This requires a per-bin temperature LP, but the
audible difference is subtle.

**Cheap kernel proposal:**
```rust
// Per-block, scalar:
let mut total_pow = 0.0_f32;
let mut k = 0;
let zero = f32x8::splat(0.0);
let mut accum = f32x8::splat(0.0);
while k + 8 <= mag.len() {
    let m = f32x8::from(&mag[k..k+8]);
    accum += m * m;
    k += 8;
}
total_pow = accum.reduce_add() + scalar_tail_sum;
let rms = (total_pow * inv_n).sqrt();
self.rail_lp += (target_from_rms(rms) - self.rail_lp) * rail_alpha;
let rail_factor = 1.0 - sag_amount * (1.0 - self.rail_lp);

// Per-bin scaling, vectorised, splatting one scalar:
let rail_v = f32x8::splat(rail_factor);
let mut k = 0;
while k + 8 <= mag.len() {
    let m = f32x8::from(&mag[k..k+8]);
    (m * rail_v).write_to_slice_unaligned(&mut out[k..k+8]);
    k += 8;
}
```
This is essentially free — one vector dot product (RMS) per hop, one
scalar update, then one multiply per bin.

For the per-bin temperature variant, replace `rail_v` with a per-bin
`rail[k]` array updated by `rail[k] += (target_from_rms_and_temp(rms,
temp[k]) - rail[k]) * rail_alpha`. Same cost class.

### 6. Component drift

**Reference papers:**
- Random walk with drift, 1/f noise: see
  [pythonspeed.com SIMD][pythonspeedSimd] for the broad SIMD patterns
  and [musicdsp.org analog drift discussions][musicdspdrift] for
  audio-specific tunings.
- Voss-McCartney 1/f algorithm is the audio standard
  ([asp-eurasipjournals.springeropen.com][murapaper]).
- For per-bin drift, the audible target is "each bin's gain wanders
  slowly by ±0.5 dB on a 10 s timescale, independent across bins."

**State of the art:**
The simplest 1/f-like drift is a one-pole LP on white noise:
- `drift[k] += (white_noise() - drift[k]) * alpha_drift`
- `gain_offset[k] = drift[k] * drift_amount_db / 20 * ln(10)`

Voss-McCartney is more spectrally accurate but unnecessary for colour.
A cascade of 3 LPs at logarithmically-spaced cutoffs (e.g. 0.1 Hz, 1 Hz,
10 Hz) gives an excellent 1/f approximation for ~6 dB/decade slope.

**Cheap kernel proposal:**
```rust
// Per-bin state: drift_lp1, drift_lp2, drift_lp3 (3 cascaded LPs).
// Per-block scalar: alpha1, alpha2, alpha3 (precomputed from
// hop_seconds and the three cutoffs).
fn drift_step_simd(
    drift_lp1: &mut [f32], drift_lp2: &mut [f32], drift_lp3: &mut [f32],
    out: &mut [f32], rng: &mut SimdRng,
    a1: f32, a2: f32, a3: f32, drift_amount: f32,
) {
    let a1v = f32x8::splat(a1);
    let a2v = f32x8::splat(a2);
    let a3v = f32x8::splat(a3);
    let amount_v = f32x8::splat(drift_amount);
    let mut k = 0;
    while k + 8 <= drift_lp1.len() {
        let n = rng.next_f32x8_centred(); // -1..1
        let lp1 = f32x8::from(&drift_lp1[k..k+8]);
        let lp2 = f32x8::from(&drift_lp2[k..k+8]);
        let lp3 = f32x8::from(&drift_lp3[k..k+8]);
        let new_lp1 = lp1 + (n - lp1) * a1v;
        let new_lp2 = lp2 + (new_lp1 - lp2) * a2v;
        let new_lp3 = lp3 + (new_lp2 - lp3) * a3v;
        new_lp1.write_to_slice_unaligned(&mut drift_lp1[k..k+8]);
        new_lp2.write_to_slice_unaligned(&mut drift_lp2[k..k+8]);
        new_lp3.write_to_slice_unaligned(&mut drift_lp3[k..k+8]);
        // Output: 1.0 + (lp1+lp2+lp3) * amount/sqrt(3).
        let drift_total = (new_lp1 + new_lp2 + new_lp3) * amount_v;
        let mult = f32x8::splat(1.0) + drift_total;
        mult.write_to_slice_unaligned(&mut out[k..k+8]);
        k += 8;
    }
}
```
Total cost per bin: 3 LPs × (1 sub + 1 mul + 1 add) = 9 FLOPs, plus 1
PRNG draw. ~80 kFLOP per hop — negligible.

State memory: 3 × 8193 × 4 = 98 KB per Drift instance.

### 7. PCB crosstalk

**Reference papers:**
- General PCB crosstalk theory ([protoexpress.com][protoexpress];
  [altium.com][altium]; [cadence.com][cadencecouple]) describes the
  capacitive-coupling-as-displacement-current mechanism. For per-bin
  spectral DSP, this maps onto a leakage between bin K and bins K±1
  (and possibly K±2) of a fraction of the bin's magnitude.
- This is essentially the same operator as the audit doc's
  "Transformer SPREAD" refinement — a 1D Laplacian on magnitude.

**State of the art:**
PCB crosstalk in spectral DSP is a 1D linear convolution with a
short kernel. For a 3-tap symmetric kernel [c, 1-2c, c] (where c is
the per-bin coupling fraction), the operator conserves total energy
and is trivially vectorisable.

For asymmetric coupling (the high-end of the bin range talks more
strongly to the low-end, modelling a ground-trace pickup), an
asymmetric kernel works the same way.

**Cheap kernel proposal:**
```rust
// Two-pass: write to scratch buffer, then copy back. No in-place
// because adjacent bins would read stale data.
fn pcb_crosstalk_simd(
    mag_in: &[f32], scratch: &mut [f32],
    coupling: f32,
) {
    let c_v = f32x8::splat(coupling);
    let centre_v = f32x8::splat(1.0 - 2.0 * coupling);

    // Boundary bin 0:
    scratch[0] = mag_in[0] * (1.0 - coupling) + mag_in[1] * coupling;

    let mut k = 1;
    // Vector loop. Note non-aligned loads for the offset reads.
    while k + 8 <= mag_in.len() - 1 {
        let prev = f32x8::from(&mag_in[k-1..k+7]);
        let cur = f32x8::from(&mag_in[k..k+8]);
        let next = f32x8::from(&mag_in[k+1..k+9]);
        let out = cur * centre_v + (prev + next) * c_v;
        out.write_to_slice_unaligned(&mut scratch[k..k+8]);
        k += 8;
    }
    // Scalar tail.
    while k < mag_in.len() - 1 {
        scratch[k] = mag_in[k] * (1.0 - 2.0 * coupling)
                   + (mag_in[k-1] + mag_in[k+1]) * coupling;
        k += 1;
    }
    // Boundary bin N-1.
    let n = mag_in.len() - 1;
    scratch[n] = mag_in[n] * (1.0 - coupling) + mag_in[n-1] * coupling;
}
```
Cost: ~5 FLOPs per bin (2 muls, 2 adds, 1 mul on centre). ~40 kFLOP per
hop. Memory: one extra scratch buffer (32 KB for 8193 bins). The non-
aligned load (`mag_in[k-1..k+7]`) is the only architectural caveat:
on AVX2 unaligned loads are within a few % of aligned; on AVX-512 they
are bit-equal in cost.

### 8. Slew-rate distortion (SID)

**Reference papers:**
- Slew rate analytical references
  ([electronics-notes.com][slewen]; [ittc.ku.edu Stiles slides][stilesslew])
  describe per-sample slew limiting as a clamp on the per-sample
  derivative — cheap and well-defined in the time domain.
- Sachs, "Slew Rate Limiters: Nonlinear and Proud of It!"
  ([embeddedrelated.com/showarticle/646.php][sachsslew]) discusses the
  nonlinear character — distortion is *not* the same as the harmonic
  pattern of clipping; it produces low-order harmonics specifically
  near the slew transition.

**State of the art:**
For per-bin per-hop, the operator is:
1. Compute `delta = mag[k] - prev_mag[k]`.
2. Clamp `|delta|` to `max_slew_per_hop`.
3. Output is `prev_mag[k] + clamped_delta`.

The brainstorm note "spits clipped energy out as phase-scramble" is
audibly significant. Two implementation options:
1. **Magnitude clip + phase pass-through.** Cheapest; most musical.
   The "lost" magnitude is just discarded, no phase modification.
2. **Magnitude clip + phase noise addition.** The excess magnitude
   above the slew limit is converted into a per-bin random phase
   rotation. Audibly: the bin "smears" instead of just compressing.

Option 2 is more interesting but requires also writing to the phase
slot. For the spectral pipeline this means the kernel needs both
`mag` and `phase` arrays as input — easy because `BinPhysics`
already exposes both.

**Recommendation:** ship option 1 in v1, expose option 2 as a
`SlewMode::PhaseScramble` toggle in v2. The audible difference is
real but the v1 magnitude-clip is the foundation either way.

**Cheap kernel proposal:**
```rust
fn slew_step_simd(
    mag: &[f32], prev_mag: &mut [f32], out_mag: &mut [f32],
    out_phase: &mut [f32], phase_in: &[f32],
    max_slew: f32, phase_scramble: bool,
    rng: &mut SimdRng,
) {
    let max_v = f32x8::splat(max_slew);
    let neg_max = -max_v;
    let mut k = 0;
    while k + 8 <= mag.len() {
        let m = f32x8::from(&mag[k..k+8]);
        let p = f32x8::from(&prev_mag[k..k+8]);
        let raw_delta = m - p;
        let delta = raw_delta.fast_max(neg_max).fast_min(max_v);
        let new_mag = p + delta;
        new_mag.write_to_slice_unaligned(&mut prev_mag[k..k+8]);
        new_mag.write_to_slice_unaligned(&mut out_mag[k..k+8]);

        if phase_scramble {
            let excess = (raw_delta - delta).abs();
            let scramble = rng.next_f32x8_centred() * excess
                * f32x8::splat(std::f32::consts::PI);
            let phi = f32x8::from(&phase_in[k..k+8]) + scramble;
            phi.write_to_slice_unaligned(&mut out_phase[k..k+8]);
        }
        k += 8;
    }
}
```
Cost: 5 FLOPs per bin (clip with two mins/maxes), plus one PRNG and
one add for the scramble. ~50 kFLOP per hop.

## Open-source implementation candidates

| Repo | Lang | License | What it implements | Reusability |
|---|---|---|---|---|
| [chowdsp_wdf][chowdspwdf] | C++ | BSD-3 | WDF library + SIMD acceleration via xsimd; vactrol, BBD, diode clipper, Baxandall EQ examples | High (port equations, reimplement glue in Rust) |
| [WaveDigitalFilters][chowwdfsamples] | C++ | GPL-3 | BBD delay, Sallen-Key, diode clipper, Baxandall EQ, TR-808 examples | Medium (study models, GPL-3 conflicts with our intended licence if mixed) |
| [ChowTape][chowtape] | C++ | GPL-3 | Jiles-Atherton hysteresis with RK2/RK4/NR4/NR8 solvers, SIMD-accelerated | High (study Jiles-Atherton inner loop math) |
| [SLEEF][sleefgithub] | C | BSL-1.0 | Vectorised libm: tanh, exp, sin, log; AVX/AVX-512/NEON; <4 ULP accuracy | Reference for inner-loop math accuracy budgets |
| [SIMDxorshift][simdxorshift] | C | Apache-2.0 | AVX/AVX-512 xorshift128+; ~1.5-2 cycles/draw | High (port to Rust; ~50 lines) |
| [Xoshiro256PlusSIMD][xoshirosimd] | C++ | Apache-2.0 | Serial+SIMD xoshiro256+ | Alternative PRNG |
| [BillyDM/Fast-DSP-Approximations][fastdspapprox] | Rust | Public domain | Levien tanh, fast exp, DSP cheats | Direct copy of `tanh_levien` etc. |
| [BillyDM/awesome-audio-dsp][awesomeaudiodsp] | — | — | Curated reading list and reference index | Use as bibliography source |
| [Lokathor/wide][wide] | Rust | Zlib | Stable Rust SIMD types `f32x8` etc. | Direct dependency |
| [SamiPerttu/fundsp][fundsp] | Rust | MIT/Apache-2 | Audio DSP library; uses `wide::f32x8` extensively | Reference for SIMD patterns in real Rust audio code |
| [robbert-vdh/spectral-compressor][spectralcomp] | Rust | GPL-3 | Per-bin spectral compression in nih-plug | Reference for our exact problem domain |
| [robbert-vdh/nih-plug][nihplug] | Rust | ISC | Plugin framework + SIMD adapters | We already use this |
| [unevens/audio-dsp][unevensaudiodsp] | C++ | various | SIMD audio DSP template classes | Reference for class design patterns |
| [kfrlib/kfr][kfrlib] | C++ | GPL-2/comm | SSE/AVX/AVX-512/NEON/RVV DSP framework with FFT, FIR/IIR | Reference for cross-arch dispatch design |

The C++ libraries are the deepest source of validated WDF / saturator /
hysteresis code. None of them are direct drop-ins for Rust, but the
inner-loop math is portable: read the relevant `process()` function,
re-derive in Rust with `wide::f32x8` substituted for `xsimd::batch`.

## Audio-rate vs spectral-rate tradeoffs

This is the single most important architectural insight for choosing
which models to port.

Most analog modelling literature (Yeh, Holters, Werner, Pakarinen, the
DAFx canon) operates **per audio sample** at 44.1-96 kHz. Their cost
budgets are stated in cycles-per-sample.

We operate **per FFT hop** at the analysis rate (44.1 kHz / hop_size).
For hop=512 that is 86.1 Hz; for hop=128 it is 344.5 Hz. Three orders
of magnitude lower than audio-rate.

This affects each model class differently:

1. **Memoryless nonlinearities (tanh, soft clip, Schmitt).** Translate
   *cleanly* to hop rate. The operation is `f(mag[k])` regardless of
   sample vs. hop rate. Aliasing concerns flip — at hop rate we are
   already analysis-band-limited by the FFT's bin spacing, so the
   classical anti-aliasing concerns (BLAMP, polyBLAMP, oversampling)
   that Esqueda, Pekonen, and Välimäki worked on
   ([eurasip.org soft clip][esqueda2015]; [DAFx wavefolder][esqueda2017])
   simply don't apply. The non-linearity acts on bin magnitudes, not
   waveforms; spectral-domain magnitude shaping does not generate
   bin-Nyquist aliasing because there is no bin-Nyquist.
2. **One-pole LPs (vactrol, drift, transformer flux memory, sag).**
   Translate cleanly with parameter rescaling: `alpha_hop = 1 -
   exp(-hop_seconds / tau)`. The audible time constants are unchanged.
   Stability is preserved.
3. **Multi-stage cascades (BBD, multi-stage vactrol).** Translate with
   caveats. A 4-stage BBD updated at audio rate produces audible
   delay artefacts in the hundreds of microseconds; at hop rate a
   4-stage cascade smears across 4 hops × hop_size / sample_rate
   ≈ 20-50 ms. The audible signature shifts from "BBD chorus" to
   "smeary echo." This is *good* for a colour effect, *bad* for an
   accurate BBD emulation. We are not after accurate emulation.
4. **State-stiff hysteresis (Jiles-Atherton).** Translates *poorly*. The
   J-A ODE is genuinely stiff (Newton-Raphson required for stability
   at high drive). At hop rate the implicit time step is too large for
   N-R to converge in a few iterations on extreme transients. The
   alternative is to do J-A at audio rate — but then it leaves the
   STFT pipeline. **Recommendation:** use the cheap "tanh + magnitude
   one-pole" approximation in the spectral path; reserve J-A for a
   future time-domain Tape module if we ever build one.
5. **PRNG-driven jitter (drift, BBD dither).** Translate cleanly. The
   audible character of the PRNG is dominated by the per-bin LP
   filter, not the underlying PRNG.
6. **Linear convolutions (PCB crosstalk, transformer SPREAD).**
   Translate cleanly. These are bin-domain operations from the start.
7. **Slew-rate distortion.** Translates with semantic shift. At audio
   rate, slew rate clips the time-domain derivative. At hop rate, it
   clips the hop-to-hop magnitude derivative — i.e. it becomes a
   transient limiter or a magnitude-domain smoother depending on the
   slew-amount setting. The audible signature is *not* the
   characteristic IM-distortion of an op-amp slewing; it's a cleaner
   "transient softening." **Recommendation:** keep the name `Slew
   distortion` because it is the conceptual root, but document
   internally that the audible result is closer to a soft-knee
   transient compressor than to op-amp slew limiting.

## Cross-component synthesis

Several kernels share state and math, opening up batching opportunities:

1. **PRNG calls.** Drift, BBD dither, and (optional) Slew phase-scramble
   all consume PRNG draws. A single `SimdRng` per slot with a per-
   component sub-stream (using xoshiro's `jump()` to derive
   uncorrelated streams) lets all three share one `next_f32x8` per
   loop.
2. **One-pole LP cascades.** Vactrol (2 stages), Drift (3 stages),
   Transformer-flux memory (1 stage), and Sag (1 global) all use the
   same kernel structure: `lp_new = lp + (target - lp) * alpha`. A
   shared `lp_step_simd(state, target, alpha)` SIMD helper covers all
   four. Saves binary size and hits the same ICache line.
3. **Magnitude-conditional selectors.** Schmitt and SlewDistortion both
   use mask-blend patterns on per-bin magnitudes against thresholds.
   The `wide::CmpGt` + `Mask::blend` plumbing is identical.
4. **Spread/Laplacian.** Transformer SPREAD and PCB Crosstalk are the
   same operator with different coupling coefficients. A single shared
   kernel `spread_3tap_simd(input, output, c_left, c_centre, c_right)`
   covers both. (And could also serve a future "Capillary diffusion"
   in the Life module, per the audit.)
5. **Global RMS.** Sag uses one global RMS; the existing `Pipeline`
   already computes a similar quantity for the spectrum display
   (`spectrum_tx`). Consider adding a `pipeline.rms_per_hop` that all
   modules can read from `ModuleContext` rather than recomputing.

These shared kernels suggest a **`circuit_kernels.rs` SoA helper
module** living alongside `dsp/modules/circuit.rs`, exposing pure
functions like `lp_step_simd`, `tanh_levien_simd`, `spread_3tap_simd`,
`slew_clip_simd`. Each Circuit sub-effect's `process()` would read
state arrays from `self`, call the shared helpers, and write back.

## Recommendations

### Implementation order

1. **First**: The `wide::f32x8` substrate + `tanh_levien_simd` +
   `lp_step_simd` + a `SimdRng` (port from SIMDxorshift). These three
   helpers cover the inner loop of every other component.
2. **Second**: Schmitt, Slew, Drift, Power Sag — these use *only* the
   shared helpers; they are pure functional kernels with state
   that Circuit owns. Easy SIMD wins. Implementation cost: low. Audible
   win: high.
3. **Third**: Vactrol — needs the 2-stage release dynamic. Test against
   Parker's 2013 Buchla LPG paper for character match; tune by ear.
4. **Fourth**: BBD and PCB Crosstalk — both involve memory cascades
   (BBD) or 3-tap convolution (Crosstalk) that need extra state arrays.
   Implementation cost: medium. Tests against Holters & Parker 2018 for
   BBD character.
5. **Fifth**: Transformer flux — start with the cheap "tanh + memory"
   form. Defer Jiles-Atherton-style hysteresis until a separate Tape
   module exists.

### Validation strategy

- **Unit tests**: For each helper, golden-test a 16-bin slice against
  a scalar reference. Check `assert_relative_eq!(simd_out, scalar_out,
  max_relative = 1e-5)` for each helper.
- **Calibration probes**: Per the existing calibration-audit protocol
  (`docs/superpowers/specs/2026-04-22-calibration-audit-fix-design.md`),
  expose `probe_amount_pct`, `probe_state_at_k`, etc. for each Circuit
  mode. Round-trip GUI ↔ audio thread snapshot must match.
- **Performance**: Run `cargo bench` (or a `criterion` harness) for
  each kernel. Target: each helper completes 8193-bin scalar update in
  under 50 µs; SIMD should be 4-8× faster.
- **No-allocation**: The plugin's `assert_process_allocs` feature flag
  already catches any allocation in the audio thread. Any helper that
  trips this fails CI.
- **Audio listening tests**: Per-mode A/B against a scalar reference
  on (a) sine sweep, (b) drum loop, (c) sustained pad. Spectrograms
  must match within ±0.5 dB; null tests should give >40 dB suppression
  for kernels that are mathematically equivalent (Schmitt, Slew, PCB
  Crosstalk, Sag), and may differ for the floating-point-sensitive ones
  (Vactrol cascade, BBD cascade, Drift).

### Components to ship first

For a v0.1 of the Circuit module, ship Schmitt, Slew, Drift, and Sag
together as the "easy four." These give immediate audible variety,
share the most code with each other, and require no per-bin cascaded
state.

For v0.2 add Vactrol and BBD — these unlock the most distinctive
"vintage-circuit" character.

For v0.3 add Transformer flux (with SPREAD) and PCB Crosstalk.

This staging matches the audit doc's CPU-class assignments and gives
the largest character-per-development-week ratio in the early stages.

## Open questions

1. **xsimd port?** Do we need a Rust port of `xsimd::log_approx` and
   `xsimd::exp_approx`, or are the precomputed `exp(-hop/tau)` block
   coefficients enough? The latter is simpler but locks the time
   constants to per-block updates. Worth investigating after the core
   kernels are in.
2. **AVX-512 detection.** Should the build dispatch at runtime
   (requires `multiversion` or `pulp`) or compile two binaries? Bitwig
   on Linux can ship two; on Windows the convention is one binary with
   runtime dispatch. **Recommendation:** start with `wide::f32x8`
   (covers AVX2 well, falls back to scalar on missing CPU features),
   add a `pulp`-based AVX-512 path in v2 if profiling shows it helps.
3. **Phase-scramble Slew implementation.** The audit said "v1 magnitude
   clip, v2 phase scramble." Confirm the phase scramble actually adds
   audible interest before designing the v2 surface.
4. **Per-stage BBD dither stream**: if we share one PRNG across stages
   we save state but risk audible cross-stage correlation. Worth a
   listening test: 4 PRNG streams (per stage) vs 1 stream advanced
   stride-1 across stages. A/B blind test on a sustained noise input
   should reveal whether 1 stream is audibly different.
5. **Component Drift LP cascade**: 3 cascaded LPs at decade-spaced
   cutoffs is a heuristic 1/f approximation. Voss-McCartney is more
   spectrally accurate; is the audible difference worth the
   implementation cost? Probably not — 3 LPs is already at the limit
   of what's audibly identifiable as "drift" rather than "wobble."
6. **Vactrol audit refinement**: Najnudel et al.'s power-balanced
   formulation guarantees passivity at the cost of more state and
   more arithmetic per step. For a colour effect we don't need the
   passivity guarantee, but it would be nice to avoid the rare cases
   where the cheap 2-pole model can self-oscillate or accumulate
   numerical energy. Test by feeding a sustained tone and checking
   that the output does not grow unbounded over thousands of hops.

[levien]: https://raphlinus.github.io/audio/2018/09/05/sigmoid.html
[rationaltanh]: https://www.musicdsp.org/en/latest/Other/238-rational-tanh-approximation.html
[padetanh]: https://www.musicdsp.org/en/latest/Other/178-reasonably-accurate-fastish-tanh-approximation.html
[fastexp]: https://www.musicdsp.org/en/latest/Other/222-fast-exp-approximations.html
[sleef]: https://sleef.org/
[sleefarxiv]: https://arxiv.org/abs/2001.09258
[sleefgithub]: https://github.com/shibatch/sleef
[chowdspwdf]: https://github.com/Chowdhury-DSP/chowdsp_wdf
[chowwdfsamples]: https://github.com/jatinchowdhury18/WaveDigitalFilters
[chowtape]: https://github.com/jatinchowdhury18/AnalogTapeModel
[chowhysteresis]: https://jatinchowdhury18.medium.com/complex-nonlinearities-episode-3-hysteresis-fdeb2cd3e3f6
[chowtapedafx]: https://ccrma.stanford.edu/~jatin/420/tape/TapeModel_DAFx.pdf
[chowsimd]: https://medium.com/codex/faster-tape-emulation-with-simd-49287d7b24cf
[stdsimd]: https://doc.rust-lang.org/std/simd/index.html
[wide]: https://github.com/Lokathor/wide
[pulp]: https://lib.rs/crates/pulp
[fearlesssimd]: https://github.com/linebender/fearless_simd
[fearlessSimd2025]: https://linebender.org/blog/towards-fearless-simd/
[safearch]: https://docs.rs/safe_arch
[xsimd]: https://github.com/xtensor-stack/xsimd
[shnatsel]: https://shnatsel.medium.com/the-state-of-simd-in-rust-in-2025-32c263e5f53d
[fundsp]: https://github.com/SamiPerttu/fundsp
[fastdspapprox]: https://github.com/BillyDM/Fast-DSP-Approximations
[awesomeaudiodsp]: https://github.com/BillyDM/awesome-audio-dsp
[parker2013]: https://dafx.de/paper-archive/2013/papers/44.dafx2013_submission_56.pdf
[najnudel2023]: https://www.dafx.de/paper-archive/2023/DAFx23_paper_50.pdf
[holtersbbd]: https://www.hsu-hh.de/ant/wp-content/uploads/sites/699/2018/09/Holters-Parker-2018-A-Combined-Model-for-a-Bucket-Brigade-Device-and-its-Input-and-Output-Filters.pdf
[raffelbbd]: https://colinraffel.com/publications/dafx2010practical.pdf
[wikija]: https://en.wikipedia.org/wiki/Jiles%E2%80%93Atherton_model
[royalsocja]: https://royalsocietypublishing.org/doi/10.1098/rspa.1983.0035
[ampbooks]: https://www.ampbooks.com/mobile/dsp/power-supply/
[swankyampsag]: https://www.kvraudio.com/forum/viewtopic.php?t=565616
[wikischmitt]: https://en.wikipedia.org/wiki/Schmitt_trigger
[dsprelschmitt]: https://www.dsprelated.com/showthread/comp.dsp/120299-1.php
[volterracategorical]: https://arxiv.org/abs/2308.07229
[yehthesis]: https://ccrma.stanford.edu/~dtyeh/papers/DavidYehThesissinglesided.pdf
[esqueda2015]: https://www.eurasip.org/Proceedings/Eusipco/Eusipco2015/papers/1570104119.pdf
[esqueda2017]: https://www.dafx17.eca.ed.ac.uk/papers/DAFx17_paper_82.pdf
[simdxorshift]: https://github.com/lemire/SIMDxorshift
[xoshirosimd]: https://github.com/stephanfr/Xoshiro256PlusSIMD
[spectralcomp]: https://github.com/robbert-vdh/spectral-compressor
[nihplug]: https://github.com/robbert-vdh/nih-plug
[unevensaudiodsp]: https://github.com/unevens/audio-dsp
[kfrlib]: https://github.com/kfrlib/kfr
[protoexpress]: https://www.protoexpress.com/blog/crosstalk-high-speed-pcb-design/
[altium]: https://resources.altium.com/p/crosstalk-or-coupling
[cadencecouple]: https://resources.system-analysis.cadence.com/blog/msa2021-minimize-crosstalk-with-capacitive-coupling-noise-reduction-methods
[slewen]: https://www.electronics-notes.com/articles/analogue_circuits/operational-amplifier-op-amp/slew-rate.php
[stilesslew]: https://www.ittc.ku.edu/~jstiles/412/handouts/2.6%20Large%20Signal%20Operation%20of%20Op%20Amps/Slew%20Rate%20lecture.pdf
[sachsslew]: https://www.embeddedrelated.com/showarticle/646.php
[pythonspeedSimd]: https://pythonspeed.com/articles/simd-stable-rust/
[musicdspdrift]: https://www.kvraudio.com/forum/viewtopic.php?t=176447
[murapaper]: https://asp-eurasipjournals.springeropen.com/articles/10.1155/2011/940784
[cdmAalto]: https://cdm.link/madronas-randy-jones-on-aalto-soft-synth-design-small-makers-and-soundplane-multitouch-controller/
