# Circuit Module — Audit

**Existing spec:** `docs/superpowers/specs/2026-04-21-circuit-module.md`
**Status:** DEFERRED, depends on BinPhysics.
**Source brainstorm:** the "20 history-based circuit-modeled algorithms"
section in `ideas_for_the_wonderful_future.txt` (lines 162–270), Kim's
inline annotations as the tie-breaker.

## What the spec covers

8 sub-effects: `Vactrol`, `Schmitt`, `Transformer`, `BBD`, `Power Sag`,
`Component Drift`, `PCB Crosstalk`, `Slew Distortion`. Curves: AMOUNT,
THRESHOLD, RELEASE/DECAY, MIX. Reads/writes BinPhysics `flux`,
`temperature`. SoA layout, polynomial saturation, no `exp`/`sin` in
loops.

## Brainstorm cross-reference

| # | Idea | In spec? | Kim's note | Action |
|---|---|:---:|---|---|
| 1 | Spectral Schmitt Trigger | ✓ | "simple but potentially cool" | done |
| 2 | Transformer Core Saturation | ✓ | "could maybe spread to nearby bins" | **gap: spread option missing** |
| 3 | Tape Print-Through | (in Past spec) | "could all effects like this be bundled under the 'future' module" | **see `14-future.md`** |
| 4 | The "Stuck" Relay | ✗ | "not probably distinct and interesting enough" | drop |
| 5 | Vactrol Bin Smoothing | ✓ | "I love everything like this, _saturated bins_" | done |
| 6 | Slew-Rate Induced Distortion | ✓ | "Potentially very cool" | done; clarify phase-scramble |
| 7 | Thermal Runaway | ✗ | "sudden silence is maybe a bit violent per band, maybe sag, fluctuate" | **gap: refinement of Power Sag** |
| 8 | Power Supply Sag | ✓ | "cool as it is" | done |
| 9 | Dusty Potentiometer | ✗ | "no" | drop |
| 10 | Component Tolerance Drift | ✓ | "very cool idea, also time-based" | done |
| 11 | Crossover Distortion (Class A/B Deadzone) | ✗ | "useful and easy, maybe there's a category like 'Circuit'" | **GAP — add** |
| 12 | PLL Tearing | (in Modulate spec) | "in general chaotic state changes oscillating around two distant stages are fun" | done elsewhere |
| 13 | Bucket-Brigade Bins (BBD) | ✓ | "cool idea" | done |
| 14 | PCB Trace Crosstalk | ✓ | "circuit stuff" | done |
| 15 | Resonant Feedback Channel | ✗ | "Hmm the matrix has feedback, but adjustable feedback, pretty cool. Who says though, that the bins feed back into their own bins?" | **GAP — important question** |
| 16 | Ground Loop (50/60Hz Hum Intermodulation) | ✗ | "kind of bland, maybe combine with the subharmonic generator" | **drop here, see Harmony** |
| 17 | Diode Bridge Ring Mod | (in Modulate spec) | "i wonder if a dedicated RM/FM/PM module is needed? Modulate" | done elsewhere |
| 18 | Envelope Follower Ripple | ✗ | "not bad, maybe a switch not a module? But when and where?" | **defer — see Open Questions** |
| 19 | Asymmetric Bias Fuzz | ✗ | "yes the clipping and folding back of bins is a cool idea" | **GAP — add** |
| 20 | "Bypass Switch" Pop | ✗ | "no, this is a gimmick, and useless for production" | drop |

## Gap details

### a) Crossover Distortion — Class A/B Deadzone (idea #11)

**Concept.** Low-level signals are aggressively muted by an inverse-square
deadzone around zero. As a bin's magnitude crosses through the deadzone,
it produces sputtering, broken-radio fuzz on tails of sounds.

**Why add to Circuit.** Same mental model as the other Circuit sub-
effects — analog component imperfection. Curve-driven deadzone width.

**Sub-effect proposal: `Crossover` mode**
- Reads: bin magnitude.
- Writes: output magnitude (deadzoned with smooth re-emergence curve).
- State: none — pure stateless transform of magnitude.
- Curve: AMOUNT controls deadzone width (the "diode threshold"); RELEASE
  unused.
- CPU: trivial.

### b) Asymmetric Bias Fuzz (idea #19)

**Concept.** The bin's "zero point" is pushed off-centre by recent
history (DC offset). Loud transients clip asymmetrically against the
"top rail," adding even-order harmonics to the magnitude envelope.

**Why add to Circuit.** Maps to the analog-imperfection vocabulary. Adds
a useful even-order character that none of the existing Circuit modes
provide.

**Sub-effect proposal: `Bias Fuzz` mode**
- Reads: bin magnitude history (a one-pole LP envelope per bin acts as
  the DC offset).
- Writes: output magnitude (clipped against `top_rail = 1.0 - bias[k]`).
- State: `bias_lp[MAX_NUM_BINS]` (1-pole envelope).
- Could read/write `BinPhysics.bias` (proposed new field in `01-global-
  infrastructure.md`).
- Curve: AMOUNT (clip amount), THRESHOLD (top-rail gain), RELEASE (bias
  envelope time constant).
- CPU: trivial.

### c) Resonant Feedback Channel (idea #15) and Kim's question

**Kim's annotation:** "Who says though, that the bins feed back into
their own bins?"

This is a load-bearing question. Three plausible answers:

1. **Intra-bin feedback** (the spec's default reading). Each bin's
   output is fed back into its own input on the next hop with a per-bin
   feedback amount from a curve. Easy to implement, easy to understand,
   risks self-oscillation per bin.
2. **Spectral-shifted feedback.** Feed bin K's output back into bin
   K+offset, where offset is set by a curve. Gives evolving harmonic
   shifts — a single sustained partial sweeps slowly upward through the
   spectrum.
3. **Matrix-routed feedback.** Don't do feedback inside the Circuit
   module at all — let the user wire feedback through the existing
   `RouteMatrix` (Slot N → Slot M with M < N is already a feedback
   loop). The Circuit module exposes its output, the matrix handles the
   topology. Pair with **Matrix Amp Nodes** (file `03`) to put a
   `Vactrol` on the feedback edge.

**Recommendation:** Option 3 is the cleanest. The feedback architecture
already exists in `RouteMatrix`; making the Circuit module add its own
parallel feedback layer duplicates work and creates two ways for the
user to do the same thing. The Circuit module should *expose* feedback-
hostile state (high resonance per bin) and let the matrix route it.

If users explicitly want intra-bin feedback as a sub-effect, add
`Resonant` mode later — but ship without it, see if anyone asks.

### d) Transformer spread to neighbours (Kim's note on #2)

**Concept.** Transformer flux saturation in bin K influences neighbours
K±1 via leakage (analog-style flux coupling). Today the spec is per-bin
isolated.

**Refinement to existing `Transformer` sub-effect.** Add a SPREAD curve
or a global SPREAD parameter:

```rust
flux_after_spread[k] = flux[k]
  + spread * 0.5 * (flux_in[k-1] + flux_in[k+1])
```

Two-pass to avoid order artifacts (read pass, write pass).

CPU class: still **light** — one extra add per bin. SPREAD = 0 by
default to preserve current behaviour.

### e) Thermal Runaway refinement (Kim's note on #7)

**Spec state:** the existing spec has `Component Drift` reading
`temperature`, but `Thermal Runaway` itself is not a sub-effect. The
brainstorm version had violent self-oscillation if temperature crosses
threshold. Kim wants "sag, fluctuate" instead of "violent silence."

**Sub-effect proposal: refine into `Sag` instead of separate Runaway**
- The existing `Power Sag` already does global pumping. Extend it to
  also read per-bin `temperature` — a hot bin contributes more sag
  weight than a cool bin. Then bins that *cause* sag through sustained
  energy *are* the bins that get hit hardest.
- Output: cooling curve is fluctuating not zeroed. Use a 1-pole
  smoother on the `gain_reduction` envelope to make the recovery
  audibly soft.

This avoids adding a Thermal Runaway sub-effect and instead enriches the
existing one. Honors Kim's "fluctuate" preference.

### f) Envelope Follower Ripple (idea #18)

Kim asked: "maybe a switch not a module? But when and where?"

**Answer.** This is best implemented as a global switch on any module
that *uses* an envelope follower internally. Today that's:
- `Dynamics` (compressor envelope)
- `Freeze` (peak hold envelope)
- `PhaseSmear` (peak envelope)
- Most Circuit / Life / Kinetics modes that depend on smoothed magnitude

A `EnvelopeRipple { Off, Light, Heavy }` enum on each consuming module
adds a small frequency-dependent oscillation to the smoothed envelope.
Cheap (one extra LFO term in the smoother). UI: a small dot on the
attack/release time controls.

**Recommendation:** defer. Add a `with_envelope_ripple` helper to
`utils.rs` so any module can adopt it once we decide it earns its UI
real-estate.

## Curve set

The spec proposes 4 curves (AMOUNT, THRESHOLD, RELEASE, MIX). With the
gaps added we may need a 5th, but `AMOUNT/THRESHOLD/SPREAD/RELEASE/MIX`
covers everything cleanly. **Recommendation:** bump to 5 curves.

| Idx | Label | Used by |
|---|---|---|
| 0 | AMOUNT | All modes |
| 1 | THRESHOLD | Schmitt (on), Transformer, Slew, Crossover, Bias Fuzz |
| 2 | SPREAD | Transformer (new), PCB Crosstalk, Bias Fuzz |
| 3 | RELEASE | Vactrol, Transformer, Sag |
| 4 | MIX | All modes |

## CPU class

Spec describes BBD as needing per-bin per-stage delay buffers. With 4
stages × 8193 bins × 4 bytes = 130 KB per BBD instance. One BBD per
slot worst case. Total ~1 MB. Manageable.

Vactrol, Schmitt, Slew, Bias Fuzz: **medium** (per-bin LP / latch).
BBD, PCB Crosstalk, Transformer-with-spread: **medium-heavy**.

Tag the module `heavy_cpu = true` overall. Matches the spec's intent.

## BinPhysics interactions

Reads: `flux`, `temperature` (existing).
Writes: `flux`, `temperature` (existing).
With proposals: also reads/writes `bias` (new field — see `01-global-
infrastructure.md` §1).

## Calibration probe set

For round-trip tests:
- `probe_amount_pct` — AMOUNT curve resolved at probe bin
- `probe_threshold_db` — THRESHOLD-derived dBFS threshold for active mode
- `probe_release_ms` — release time the user dialled in
- `probe_mode` — AmpMode-style enum currently selected
- `probe_state_at_k` — current per-bin internal state (vactrol cap,
  latch bool, flux value, etc.) at the test bin

## Module ordering implications

Circuit's sub-effects are sequential signal-path stages — but the user
sees them as alternates (one mode at a time). **Question:** is there
demand for *Circuit-as-stack* — the user picks 2-3 sub-effects and they
chain inside one slot? This is what guitar-pedalboard plugins do. Adds
significant UI complexity but a single Circuit slot that runs Vactrol →
Transformer → Slew gives a real "amp-channel" feel.

**Recommendation:** v1 is single-mode-per-slot like other modules; v2
considers a stack. Note for follow-up.

## RESEARCH PROMPT — SIMD analog kernels

```
Topic: Per-bin SIMD-friendly analog component models for spectral DSP

Context: We have a CLAP plugin (Rust, nih-plug, realfft) doing
overlap-add STFT with up to 8193 bins per hop. We want to model 8 analog
components (vactrol, Schmitt, transformer flux, BBD, power sag,
component drift, PCB crosstalk, slew distortion) per-bin in real time
on a modern desktop CPU. Layout is Structure-of-Arrays
(separate Vec<f32> per state field). Today's loops are scalar.

Goal: Identify the cheapest-to-implement, accurate-enough numerical
schemes for each, with explicit AVX2/AVX-512 vectorisation patterns.
Avoid exp(), sin(), tanh() in the inner loop — use polynomial
approximations or 256-element lookup tables with hardware gather.

Specific questions:
1. Vactrol release: real photoresistors have multi-stage non-linear
   recovery. What is the cheapest 2-3 segment polynomial approximation
   that captures the perceptual "ringing" character without burning
   memory bandwidth?
2. Transformer flux saturation: tanh-style soft clipping with hysteresis.
   What polynomial degree is enough? Reference: Lambeth & Holters,
   "Modeling Audio Transformers using Volterra Series" — is this
   overkill for our use case?
3. BBD bucket model: 4 stages of LP + dither per bin. Can we share the
   dither LFSR across bins or does it need to be per-bin to avoid
   audible patterns?
4. Slew-rate distortion: clipping by rate-of-change between hops. The
   existing brainstorm says "spits clipped energy out as phase-scramble."
   Mechanically: do we add the excess slew amount to a per-bin random-
   phase angle, or do we add Gaussian noise to the magnitude? What is
   the audible difference?
5. Schmitt hysteresis: branch-free implementation per bin. Best done
   with a mask-based update?

Deliverable: For each component, a Rust function signature that takes
&mut [f32] state arrays and processes 8 bins at once via std::simd or
core_arch. Include literature references.
```

## Open questions

1. **Stack vs single-mode (above):** v1 single, v2 stack — confirm.
2. **Bias field in BinPhysics:** approve adding it?
3. **Resonant Feedback as a sub-effect or matrix-routed:** matrix-routed
   recommended.
4. **Envelope Follower Ripple:** is the global-switch idea worth a
   second look, or shelve?
5. **CPU class default:** `heavy_cpu = true`. Confirm — it does mean
   it's bypassed by default on a low-end-hardware tier (per `02-architectural-
   refactors.md` §9), but desktop default is enabled.

## Research findings (2026-04-26)

SIMD analog kernel implementation is covered by
`research/04-simd-analog.md`. Validated decisions:

1. **SIMD substrate = `wide` (Lokathor) for v1, `pulp` for v2** if
   runtime AVX-512 dispatch becomes necessary. `wide` is what FunDSP
   uses in production, stable Rust, Zlib licence (compatible with
   GPL eventual release), exposes `f32x8` (the natural width for AVX2,
   half-width for AVX-512). **NOT `std::simd`** — locks the build
   to nightly, and shipped binaries cannot tie to a nightly compiler.
   `fearless_simd` is too immature (no AVX2-with-FMA path).
2. **Six of the eight Circuit components reduce to combinations of
   four shared SIMD primitives.** Build a small
   `src/dsp/circuit_kernels.rs` exposing:
   - `tanh_levien_simd(buf, k_drive)` — soft saturation, polynomial
     approximation, 4-5 muls + 1 div per 8 lanes.
   - `lp_step_simd(state, target, alpha)` — branchless 1-pole filter,
     blendable for asymmetric attack/release.
   - `spread_3tap_simd(buf, kernel)` — 3-tap stencil for transformer
     spread / PCB crosstalk.
   - `SimdRng` (SIMDxorshift-derived) — branchless per-lane PRNG for
     dither / drift / Brownian motion.
3. **Vactrol = two cascaded 1-poles** with τ_fast ≈ 8 ms and
   τ_slow ≈ 250 ms — cheapest convincing approximation of the
   release "ringing" character. The audible signature emerges
   naturally from the gap between the two time constants. Two state
   slots per bin; ~10 FLOPs per bin per hop.
4. **Schmitt = branch-free mask-blend** with two thresholds and one
   `u8` of state per bin. Canonical pattern, no academic literature
   needed — Wikipedia is sufficient.
5. **Transformer flux saturation = tanh + magnitude one-pole**, NOT
   full Volterra series. The cheap approximation wins precisely
   because of the rate mismatch — at hop rate, anti-aliasing for
   nonlinear waveshaping (BLAMP/polyBLAMP) is irrelevant because
   magnitudes don't have a bin-Nyquist to alias against.
6. **DEAD ENDS:**
   - Volterra-series transformer modeling — overkill, the cited
     Holters & Lambeth paper was not retrievable but the cheap path
     is fine.
   - Full Jiles-Atherton hysteresis (Chowdhury's tape model) —
     becomes *worse* at hop rate because the implicit ODE solver
     wants small time steps.
7. **Power Sag / Thermal-Runaway refinement** — implement Kim's
   "fluctuate / sag rather than sudden silence" note as a per-band
   `sag_envelope` that decays and recovers via the shared
   `lp_step_simd` primitive.
