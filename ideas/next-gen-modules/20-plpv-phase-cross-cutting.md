# PLPV Phase Unwrapping & Locking — Cross-Cutting DSP Technique

> **Naming note (2026-04-27):** the technique is referred to as **PLPV**
> (Peak-Locked Phase Vocoder) throughout the forward-looking text below.
> The brainstorm and the original draft of this file called it "PVX"
> after ProSoniq; the rationale for the rename is documented in the
> Research findings section at the bottom. Quoted brainstorm text, the
> RESEARCH PROMPT block, and the original Research findings section
> retain the historical "PVX" wording on purpose.

**Existing spec:** none (this is not a module).
**Status:** IMPLEMENTED (2026-04-28). See plan
`docs/superpowers/plans/2026-04-27-phase-4-plpv-phase.md` and
[`docs/superpowers/STATUS.md`](../../docs/superpowers/STATUS.md).
**Source brainstorm:** the entire "PVX" section at the top of
`docs/future-ideas/ideas_for_the_wonderful_future.txt`:

> If we are ducking a signal, we shouldn't just scale each bin
> independently. We should scale the "Peak" and its neighbors as a
> single unit, and use the pvx phase-locking math to ensure they stay
> vertically aligned. This preserves the "shape" of the waveform even
> as its volume changes.
>
> FFT → Peak Detection → Phase Unwrapping → Magnitude Scaling →
> Phase Propagation → iFFT

## Why this is its own document

PLPV phase unwrapping and peak-region phase locking is **not a
module**. It is a *DSP technique* that improves the quality of
several already-shipped and several deferred modules:

**Shipped modules that benefit:**

- **Dynamics** (`src/dsp/modules/dynamics.rs`) — sidechain ducking
  currently scales each bin independently, which can break
  partial-phase relationships. PLPV peak-locking would scale the
  peak + its skirt as a single unit.
- **PhaseSmear** (`src/dsp/modules/phase_smear.rs`) — randomization
  could be applied to *unwrapped* phase trajectories rather than
  wrapped, giving smoother results.
- **Freeze** (`src/dsp/modules/freeze.rs`) — frozen-spectrum playback
  has audible window-boundary clicks; PLPV phase propagation would
  smooth them.
- **MidSide** — the M/S encode/decode is a phase-sensitive operation;
  PLPV-aware locking could preserve mid-channel phase coherence
  better. See § Inter-channel phase-drift probe below for the
  measurement we use to verify this.

**Deferred modules that benefit:**

- **Modulate** (`16-modulate.md`) — Phase Phaser and PLL Tear are
  both phase-domain operations.
- **Past** (`13-past.md`) — Stretch mode reads history at non-unity
  rate, which needs phase-coherent rotation.
- **Future** (`14-future.md`) — Predicted Spectrum Interpolation
  needs phase prediction.
- **Harmony** (`15-harmony.md`) — Inharmonic mode shifts partials,
  which needs phase coherence to avoid clicks.
- **Geometry** (`18-geometry.md`) — Wavefield substrate update
  needs phase-coherent injection.

This is the most leveraged DSP infrastructure work in the entire
deferred list. Doing it once in `Pipeline` benefits ~10 modules.

## What PLPV phase unwrapping is

### The "wrapped" problem

Standard FFT phase is bounded to ±π. As a partial drifts in
frequency, its phase advances each hop; when the cumulative advance
exceeds π, it *wraps* — jumps from +π to -π. Algorithms that operate
on phase deltas (PLLs, peak trackers, phase rotators) have to
explicitly handle this wrap or they produce artefacts.

### Unwrapping

For a peak at bin `k`, the *expected* phase advance per hop is:

```
expected_advance = 2π · k · hop / fft_size
```

The *observed* phase delta per hop is `current_phase - prev_phase`,
mod 2π. The **deviation** is:

```
deviation = mod(observed - expected, 2π) ∈ [-π, π]
```

The **unwrapped phase trajectory** is then the cumulative sum of
`expected_advance + deviation`, accumulated per bin per hop.

This gives a continuous-time-like phase signal that does not jump at
window boundaries. All phase-domain math becomes cleaner.

### Phase locking — the "vertical alignment" trick

For peak detection: identify a peak bin `P` and its neighbours
`P-1, P-2, P+1, P+2` (the "skirt"). When we modify the peak's
magnitude (e.g. ducking it), we should *modify the skirt by the same
factor* AND maintain the relative phase relationships within the
skirt so the peak's iFFT-time-domain shape is preserved.

The "phase lock" math (per Laroche-Dolson 1999): rotate the skirt's
phases so they maintain the same relative offsets to the peak's
phase, even after the magnitude scaling. This avoids partial
smearing during dynamic processing.

```
For each skirt bin S relative to peak P:
  rel_phase = unwrapped_phase[S] - unwrapped_phase[P]  (preserved)
  new_phase[S] = new_phase[P] + rel_phase
```

The peak's new phase is computed via standard PV math; the skirt
*follows* the peak.

### Low-energy bin phase damping

Bins below a noise floor (`-60 dBFS RMS` default) carry phase that
is dominated by sensor / quantisation noise. Letting that noise into
the unwrapped trajectory pollutes the peak-relative math and shows
up as a faint chuffing texture on quiet passages.

The mitigation, lifted from `repos/pvx`'s
`PHASINESS_IMPLEMENTATION_PLAN` Phase 1, is to **damp the unwrapped
phase of low-energy bins toward their expected-advance value**,
using a soft sigmoid blend so the transition into and out of the
damped band does not click. The damping ratio is exposed as a single
plugin-level param `plpv_phase_noise_floor_db`, default `-60.0`,
range `-90.0 … -20.0`.

This is a Phase 1 deliverable (see § Implementation phasing) — it
ships with the unwrap kernel, before peak detection.

## Where PLPV integrates in the Pipeline

### Pipeline reordering

Today, `Pipeline::process()` runs:

1. STFT for main + sidechains.
2. Apply slot curves (per-bin, scalar gains).
3. FxMatrix dispatch (per-slot SpectralModule processing).
4. iFFT + OLA.

To integrate PLPV, insert two new stages:

1. STFT.
2. **NEW: Per-bin unwrap + low-energy damping.** Compute
   `unwrapped_phase[bin]` from `current_phase[bin]` + `prev_phase[bin]`
   + expected advance, then damp bins below
   `plpv_phase_noise_floor_db` toward the expected-advance value.
   Store in `ModuleContext.unwrapped_phase`.
3. **NEW: Peak detection.** Find the M loudest bins (M ≈ 64). For
   each peak, identify its skirt (neighbours within the magnitude
   threshold). Store as `ModuleContext.peaks: &[PeakInfo]`.
4. Apply slot curves.
5. FxMatrix dispatch.
6. **NEW: Phase re-wrap before iFFT.** Modules wrote unwrapped
   phases; convert back to ±π wrapped phases for the iFFT input.
7. iFFT + OLA.

Cost: per-bin unwrap is O(N), one subtract + one mod per bin per hop.
Trivial. Peak detection is O(N) (single pass, threshold-based) or
O(N log M) (priority queue). M=64 means ~4-9 µs per hop on a modern
CPU. Acceptable.

### `ModuleContext` additions

```rust
pub struct ModuleContext<'a> {
    // existing fields ...
    pub unwrapped_phase: Option<&'a [f32]>,    // None when PLPV disabled
    pub peaks: Option<&'a [PeakInfo]>,         // None when PLPV disabled
}

pub struct PeakInfo {
    pub bin: u32,
    pub magnitude: f32,
    pub skirt_lo: u16,    // first skirt bin (≤ bin)
    pub skirt_hi: u16,    // last skirt bin (≥ bin)
}
```

PLPV is opt-in per-instance (a global switch) so users with old CPUs
can disable it. When disabled, `unwrapped_phase` and `peaks` are
None and modules fall back to wrapped-phase behaviour.

### Per-channel state in Pipeline

```rust
prev_unwrapped_phase: Vec<Vec<f32>>,    // [channel][bin]
peak_buffer: Vec<PeakInfo>,             // capacity = MAX_PEAKS
```

Memory: 2 channels × 8193 bins × 4 bytes = 64 KB. Plus the peak
buffer (~2 KB). Cheap.

## Per-module integration plan

### Dynamics — sidechain ducking with peak locking

Today `DynamicsEngine::process_bins` operates per-bin. With PLPV:

1. For each peak in `ctx.peaks`, compute the gain reduction for the
   peak as today.
2. Apply that *same* gain reduction to the entire skirt
   (`skirt_lo..=skirt_hi`), not per-bin individual gain reduction.
3. Rotate the skirt's unwrapped phases to maintain relative offsets
   to the new peak phase (PLPV phase-lock math above).

Audible effect: cleaner ducking on tonal sources (no partial smear),
identical behaviour on noise (no peaks → no skirts → fall through).

CPU: same O(N) work, just gathered into peak groups. Slightly less
work overall (we only compute gain N_peaks times, not N times).

**Backward compatibility:** when PLPV is disabled, fall through to
the existing per-bin code path. No behaviour change.

### PhaseSmear — randomize unwrapped phase

Today PhaseSmear adds a random offset to the wrapped phase per bin.
With PLPV:

1. Use unwrapped phase as the base.
2. Add the random offset to unwrapped phase.
3. Re-wrap before iFFT.

Audible effect: smoother, less brittle randomization. The "shhh" is
preserved but the click character at boundaries is softened.

### Freeze — phase propagation across hops

Today Freeze stores the phase at freeze time and re-applies it each
hop. The re-application produces a static phase (peaks ringing at
the freeze instant). With PLPV:

1. At freeze time, record the *unwrapped* phase per bin.
2. Each hop, advance the unwrapped phase by `expected_advance`
   (true continuous-frequency phase evolution).
3. Re-wrap before iFFT.

Audible effect: frozen spectra continue to phase-evolve like
sustained partials, not stuck-tone artefacts.

### Modulate — Phase Phaser, PLL Tear

Already covered in `16-modulate.md` § (d). Both modes opt into
unwrapped phase from `ctx.unwrapped_phase`.

### Past — Stretch mode

Already covered in `13-past.md` § Stretch. Phase rotation by
`2π × bin_freq × delta_t` becomes mathematically clean on unwrapped
phase.

### Future — Predicted Spectrum Interpolation

Already covered in `14-future.md`. Phase prediction is "hard"
(brainstorm note); the recommendation is to predict magnitude only
and keep current phase. PLPV unwrapping doesn't help here directly,
but the recommendation still holds.

### Harmony — Inharmonic mode

Shifting partials means moving energy from one bin to another. With
PLPV, the source bin's unwrapped phase travels with the energy to
the destination bin, preserving partial coherence.

### Geometry — Wavefield substrate

Energy injection into the 2-D wavefield should be amplitude-only;
phase is irrelevant inside the substrate (it's a real-valued field).
On read-back, the field's amplitude updates the bin magnitude;
phase is left untouched (or PLPV-advanced). Low integration burden.

## CPU and quality trade-offs

| PLPV state | CPU cost | Quality benefit |
|---|---|---|
| Off | 0 | Today's behaviour. |
| Unwrap only (no peaks) | +1 % | Smoother phase math, no peak locking |
| Unwrap + peak detection | +3 % | Peak locking on ducking, partial-coherent processing |
| Unwrap + peaks + per-skirt locking | +4 % | Full PLPV behaviour |

Numbers are rough estimates from the brainstorm and need
benchmarking. Worth assuming the cost is < 5 % on the audio thread
based on FLOPs-per-bin counting; needs validation.

## Implementation phasing

### Phase 1 — Foundation (1 PR)

- Add `unwrapped_phase` storage to Pipeline (per-channel).
- Compute unwrapped phase each hop, expose in ModuleContext.
- Global enable/disable switch in params (`plpv_enable: BoolParam`).
- No module changes. Existing behaviour preserved.

### Phase 1.5 — Low-energy bin phase damping (1 PR)

- Add `plpv_phase_noise_floor_db` plugin-level FloatParam,
  default `-60.0`, range `-90.0 … -20.0`, displayed in dB FS.
- After computing the unwrapped phase, damp bins whose magnitude is
  below the noise floor toward their expected-advance value, with a
  soft-sigmoid blend across a ±6 dB band centred on the threshold.
- No module changes; the damping happens inside the Pipeline's
  unwrap kernel before `ctx.unwrapped_phase` is exposed.
- Document the failure mode in the UI: when the displayed
  noise-floor cursor crosses a peak in the spectrum view, mark it
  so users know they have set the floor too high.

This phase ships before peak detection because every downstream
consumer of `ctx.unwrapped_phase` benefits from cleaner low-energy
bins, even before peak-locking is wired in.

### Phase 2 — Peak detection (1 PR)

- Add `peaks: Vec<PeakInfo>` to Pipeline.
- Implement Laroche-Dolson local 4-neighbour max + region-of-influence
  peak detection (per the Research findings).
- Expose `ctx.peaks` to modules.
- Still no module changes.

### Phase 3 — Module integration, one at a time

- Dynamics: opt into peak locking.
- PhaseSmear: opt into unwrapped randomization.
- Freeze: opt into unwrapped phase advance.
- MidSide (shipped): opt into peak-aligned mid-side decode (verify
  with the inter-channel phase-drift probe in § Calibration impact).

Each module change is its own PR with audio tests.

### Phase 4 — Deferred modules use it from the start

When Modulate, Past Stretch, Harmony, etc. are implemented, they
consume PLPV from day one.

### v2 — Adaptive per-frame coherence policy (deferred)

`repos/pvx`'s `PHASINESS_IMPLEMENTATION_PLAN` proposes an *adaptive*
coherence policy that switches between tonal and noisy/percussive
modes per analysis frame (different lock-radius, different damping).
This is **deferred to v2** — v1 ships a single static policy
(rigid Laroche-Dolson locking, ±2-bin skirts, Voronoi-by-nearest-
peak rule for skirt assignment when peaks are dense).

Reasons to defer:

1. The static policy is good enough for the shipped Dynamics +
   PhaseSmear + Freeze use cases per the brainstorm intent.
2. Adaptive switching introduces a frame-classification step with
   its own tuning surface; we want listening tests on the static
   path before opening that.
3. The classification logic (tonal vs. percussive) overlaps with
   Roebel's COG transient detector that several deferred modules
   already need — better to land Roebel COG once and then layer
   adaptive PLPV on top in a single coherent change.

## Calibration impact

PLPV-on means the calibration probes for affected modules will see
slightly different round-trip values than PLPV-off. Two strategies:

1. **Calibrate per PLPV state:** snapshot probes once with PLPV off,
   once with PLPV on. Each module has two reference traces.
2. **Run calibration with PLPV off only:** PLPV is a quality
   improvement, not a parameter. The probe behaviour should remain
   identical regardless of PLPV state. Verify by snapshotting both
   states and asserting the probes match within ε.

Recommendation: option 2 (probes are PLPV-invariant). If they're
not, the module's PLPV path has a bug.

### Inter-channel phase-drift probe (shipped MidSide mode)

The MidSide mode is the shipped feature most exposed to phase-
locking quality. Mid-channel processing should not introduce
inter-channel phase drift between L and R; if it does, mono
compatibility breaks and the stereo image collapses on summing.

Add a calibration probe for every test signal that runs through a
slot in MidSide mode:

```
J = Σ_k |Δφ_out[k] − Δφ_in[k]|
where Δφ[k] = phase_L[k] − phase_R[k]
```

`J` is the inter-channel phase-drift objective — the sum (over
bins) of the absolute change in the L/R phase difference between
the input and the output. PLPV-off establishes the baseline;
PLPV-on must not increase `J` by more than a small ε on tonal
test signals (sine sweep, sustained chord, voice). On noise it can
drift arbitrarily — gate the assertion to bins above the noise
floor.

This probe is run as part of the existing calibration suite; the
helper lives alongside the rest of the PLPV calibration tests and
shares the peak-detection scaffolding.

The math is taken from `repos/pvx`'s `MATHEMATICAL_FOUNDATIONS.md`
§ 8 (Spatial Coherence and Channel Alignment).

## Testing

Three new test categories:

- **Phase coherence test:** synthesize a slow sine sweep, run through
  Dynamics (ducking) with and without PLPV, measure spectral
  centroid stability. PLPV-on should be stabler.
- **Boundary click test:** synthesize a sustained tone, run through
  Freeze, measure RMS at hop boundaries. PLPV-on should reduce
  click energy.
- **Inter-channel phase-drift test:** run a tonal stereo signal
  through a slot in MidSide mode, assert the `J` metric (above)
  stays within the configured ε for PLPV-on vs PLPV-off.

## RESEARCH PROMPT — PVX peak-locking math validation against ProSoniq research

```
Topic: Validation of the PVX (ProSoniq) peak-locked phase-vocoder math
for use in a real-time spectral plugin.

Context: We're building a spectral plugin that does sidechain
ducking, freeze, phase smearing, and predictive effects. The PVX
research (ProSoniq Pitch-Vocoder eXtensions) describes a phase-
unwrapping + peak-region locking technique that promises cleaner
spectral processing. We need to validate that this technique:

1. Actually produces audibly cleaner output for our use cases
   (ducking, freezing, magnitude scaling).
2. Has a robust peak-detection threshold that works on transient,
   tonal, and noisy material.
3. Doesn't introduce its own artefacts (e.g. phase locking onto a
   wrong peak during partial detection failure).

Specific questions:

1. PVX's "vertical alignment" math: is the standard formulation
   `new_phase[skirt] = new_phase[peak] + (old_phase[skirt] -
   old_phase[peak])` correct, or are there subtleties (e.g.
   weighting by magnitude, decay with distance from peak)?

2. Peak detection robustness: what threshold (relative to local
   noise floor, or absolute, or per-bin SNR) gives stable peak
   sets? Stability matters because peaks moving between hops
   create skirt-membership jitter.

3. Skirt definition: how wide should the skirt be? Fixed (e.g.
   ±2 bins), magnitude-defined (down to -20 dB from peak), or
   IF-defined (bins within ½ bin of the peak's IF)?

4. Comparison: PVX vs Laroche/Dolson phase-locked vocoder vs
   classic Roebel phase-vocoder vs IF-tracked spectral processing.
   For our use cases (real-time per-hop, 8193 bins), which gives
   best quality-per-CPU?

5. Failure modes: when does PVX phase-locking go wrong? Crossfading
   peaks, dense polyphonic content, noise-dominated bins?

Deliverable: A reference Rust implementation of PVX peak-locked
ducking + audio examples comparing per-bin gain reduction vs
peak-locked gain reduction on (a) sustained chord, (b) drum loop,
(c) vocal with sibilants. Spectrogram + listening notes.
```

## Open questions

1. **Global on/off switch.** Add a `plpv_enable: BoolParam` in
   params, default true. Yes / no?
2. **Backward compatibility.** All shipped modules need to behave
   identically when PLPV is off. Confirm by running the existing
   calibration tests with PLPV off — must produce the same probe
   traces.
3. **Peak detection algorithm.** Threshold-based, parabolic-
   interpolation, or IF-driven? See Research findings — Laroche-
   Dolson local 4-neighbour max with region-of-influence is the
   recommendation.
4. **Skirt width policy.** Fixed (±2 bins), magnitude-derived, or
   per-module override? v1 ships fixed ±2 with Voronoi-by-nearest-
   peak when peaks are dense; per-module override is v2.
5. **Per-channel handling.** Each channel has its own peak set in
   Independent stereo. In Linked / MidSide, peaks are computed on
   the active spectrum (mid for MidSide). Confirm.
6. **Phase 1 scope.** Just the unwrapping + ModuleContext exposure,
   or also a no-op default for `peaks`? The latter unblocks Phase 3
   integration tests earlier.

## Research findings (2026-04-26)

See `research/01-pvx-phase-and-pll.md` for the full digest. Key updates
to the framing in this file:

1. **"PVX" is not a real algorithm name.** ProSoniq's actual flagship
   algorithms were MPEX (neural) and ZTX (wavelet). The "PVX phase
   locking" math the brainstorm cites is a folk synthesis of:
   - Puckette 1995 (loose locking, no peak detection),
   - Laroche & Dolson 1999 (rigid/scaled locking with explicit peak
     detection + region-of-influence),
   - Roebel 2003 (transient classification — disable locking on attacks).
   The ProSoniq peak-shifting patent (US6549884B1) has expired. Rename
   this technique to **PLPV** (Peak-Locked Phase Vocoder) or **LDR**
   (Laroche-Dolson-Roebel) in future docs to avoid misattribution.
   `.pvx` in CDP/Csound land is a *file format* extension (PVOC-EX),
   not an algorithm.
2. **Decision: land unwrapping + peak detector once at Pipeline level**,
   exposed via `ModuleContext::unwrapped_phase` and `ctx.peaks`. Both
   are computed in the analysis FFT path before FxMatrix dispatch.
   Eight modules benefit (Dynamics, PhaseSmear, Freeze, Past Stretch,
   Modulate PLL Tear, Harmony Inharmonic, Punch fill, Geometry
   Wavefield) for the cost of one PR. The PLL bank's "track only peak
   bins" optimisation reduces 8193-bin work to ~100-bin work for free
   if peaks are already computed.
3. **Peak detection algorithm:** Laroche-Dolson local 4-neighbour max
   + region-of-influence bounded by midpoint between adjacent peaks.
   Roebel COG (centre-of-gravity) classifier disables locking on
   transient peaks.
4. **PGHI / RTPGHI** (Pruša & Holighaus 2017-2022, arXiv 2202.07382)
   eliminates peak picking entirely and is the SOTA for spectrogram
   inversion / time-stretching. Worth knowing about but **not relevant
   to ducking** — keep the LDR path for our use case.
5. **No suitable Rust implementation to lift.** Five Rust phase-vocoder
   crates exist, none implement Laroche-Dolson. We write this from
   scratch.

## Research findings addendum (2026-04-27) — re-audit of `repos/pvx`

A second pass over the local `repos/pvx` checkout (Colby Leider's
MIT-licensed Python toolkit) added the following to the picture
above. The full cross-cutting addendum is in
`91-research-synthesis.md`; this section captures the
PLPV-specific deltas.

1. **Cite Laroche-Dolson 1997, "About this Phasiness Business."**
   The 1999 WASPAA paper everyone references for the peak-locking
   math is the *implementation* paper. The 1997 paper is the
   *motivation* paper that defines "phasiness" perceptually and
   establishes why peak-locking is needed in the first place. Add
   it to the canonical reference list:
   - Laroche, J. and Dolson, M. (1997). *About this Phasiness
     Business.* Proc. ICMC 1997.
   - Laroche, J. and Dolson, M. (1999). *Improved Phase Vocoder
     Time-Scale Modification of Audio.* IEEE Trans. on Speech and
     Audio Processing, 7(3).

2. **Implementation size revised down.** The original synthesis
   estimated ~200 lines of Rust for the identity-locking kernel.
   The reference in `repos/pvx`'s `src/pvx/core/voc.py`
   (`apply_identity_phase_locking`) is ~33 lines of NumPy; the
   straight Rust port is ~50 lines after replacing array slicing
   with explicit loops. The Phase 2 PR is correspondingly smaller.

3. **Voronoi (nearest-peak) skirt assignment is the v1 default.**
   When peaks are dense and skirts overlap, assign each non-peak
   bin to its *nearest* peak (in bin-index distance). This is what
   `apply_identity_phase_locking` does and it works in production.
   `magnitude × distance` weighting and IF-based assignment are v2
   refinements; do not pay the complexity speculatively.

4. **Inter-channel phase-drift probe** (now in § Calibration impact
   above) implements `J = Σ |Δφ_out − Δφ_in|` from `repos/pvx`'s
   `MATHEMATICAL_FOUNDATIONS.md` § 8. Same metric; same
   interpretation. Keep the math identical so future cross-
   reference between our test outputs and the reference
   implementation stays meaningful.

5. **Adaptive per-frame coherence policy: defer to v2** (now
   recorded in § Implementation phasing above). `repos/pvx`'s
   plan ships an adaptive policy in its Phase 3; we ship a static
   one in v1 and revisit after listening tests.

6. **No code lift from `repos/pvx`.** License is compatible (MIT)
   but the runtime is wrong (Python + CuPy). Treat `voc.py` as a
   *reference implementation* for verifying our Rust port produces
   the same per-frame behaviour. The benefit is that we can A/B
   any future change in our Rust kernel against the Python
   reference and detect regressions.

7. **Hybrid PV + WSOLA path is a `Past` Stretch concern, not a
   PLPV concern.** Recorded here so it is not lost: when Past
   Stretch lands, it should support a `transient-mode` parameter
   that follows `repos/pvx`'s lead — `wsola` for percussive,
   `pv` for tonal, `hybrid` (default) which switches based on a
   transient detector. The Past audit will own that decision.
