# Research: Time Manipulation — Stretch / Predict / Drift

**Source prompts:** 3, 6, 10 in `90-research-prompts.md`
**Status:** Findings as of 2026-04-26
**Scope:** Three closely-coupled topics that all live on top of "manipulate
complex STFT frames coherently across time."

---

## Refined research questions

Distilled from the original prompts after literature pass:

1. (Topic A — Stretch) For an 8193-bin STFT history buffer played back at
   variable rate `α` in the range `[0.25, 4]`, what is the minimum
   per-bin phase math we need to avoid audible vocoder phasiness, and at
   what `α` deviation does the cheap math start to perceptually fail?
2. (Topic A — Stretch) Is there a meaningful CPU win from precomputed
   integer-ratio LUTs, or is the per-hop cost of the general formula so
   small that LUTs only add complexity?
3. (Topic A — Stretch) How much do we lose audibly by skipping
   Laroche/Dolson peak-locked phase coherence at our hop rates, and is
   "controlled phasiness" a usable musical knob?
4. (Topic A — Stretch) Wrap an existing GPL-incompatible library (Rubber
   Band, Bungee, Signalsmith Stretch) or write a minimal in-house kernel?
5. (Topic B — Future) For per-bin one-frame prediction over 8193 bins at
   hop ~256, is even cheap AR(2)/AR(3) Burg too expensive, or is naive
   linear extrapolation in dB / linear actually competitive?
6. (Topic B — Future) Can a per-bin "prediction confidence" score
   (spectral flux + phase-derivative variance) cheaply gate which bins
   we extrapolate vs. which we leave dry?
7. (Topic B — Future) Should phase ever be predicted, or is "extrapolate
   magnitude, keep current phase" the right default?
8. (Topic C — Punch fill) For a sub-bin pitch drift of 0–0.5 bins, is
   `Δφ = 2π · Δf · hop / sample_rate` the full story, or do we need
   IF-aware corrections to keep the iFFT clean?
9. (Topic C — Punch fill) Across release strategies (return-to-zero,
   freeze-in-place, slow drift back), which sounds most natural without a
   second audible motion when the sidechain quiets?
10. (Cross-topic) Can all three modes share infrastructure — e.g. a
    `PhaseRotator` helper that operates on `Complex<f32>` slices using
    bin-frequency × time-offset?

---

## Topic A — Phase-coherent variable-rate STFT playback

### Key references

- Flanagan & Golden (1966) — original phase vocoder. Foundational, but
  predates phasiness analysis.
- Puckette, "Phase-locked Vocoder" (IEEE WASPAA 1995):
  https://msp.ucsd.edu/Publications/mohonk95.pdf — first peak-locking
  proposal; introduces "lock phase of channel to phase of dominant
  neighbour."
- Laroche & Dolson, "Phase-vocoder: about this phasiness business"
  (WASPAA 1997):
  https://www.ee.columbia.edu/~dpwe/papers/LaroD97-phasiness.pdf —
  diagnoses phasiness as loss of vertical coherence.
- Laroche & Dolson, "Improved Phase Vocoder Time-Scale Modification of
  Audio" (IEEE TSAP 1999) +
  Laroche & Dolson, "New Phase-Vocoder Techniques for Real-Time
  Pitch-Shifting, Harmonizing, and Other Exotic Effects" (WASPAA 1999):
  https://www.ee.columbia.edu/~dpwe/papers/LaroD99-pvoc.pdf — proposes
  *identity phase locking* and *scaled phase locking*. The two papers
  together are the canonical reference.
- Roebel, "A new approach to transient processing in the phase vocoder"
  (DAFx 2003): https://www.mp3-tech.org/programmer/docs/dafx32.pdf —
  per-peak transient detection, no need to force `α=1` during transients.
- Bonada, "Automatic Technique in Frequency Domain for Near-Lossless
  Time-Scale Modification of Audio" (ICMC 2000) — multi-resolution PV;
  reset higher-frequency phases at attack while preserving low-frequency
  vertical coherence.
- Moinet & Dutoit, "PVSOLA: Phase Vocoder with Synchronized OverLap-Add"
  (DAFx 2011): http://recherche.ircam.fr/pub/dafx11/Papers/57_e.pdf —
  periodic full-frame phase reset using cross-correlation alignment.
- Karrer et al., "PhaVoRIT: A Phase Vocoder for Real-Time Interactive
  Time-Stretching" (ICMC 2006):
  https://quod.lib.umich.edu/i/icmc/bbp2372.2006.142/ — practical
  real-time algorithm comparison.
- Prusa & Holighaus, "Phase Vocoder Done Right" (arXiv:2202.07382, EUSIPCO
  2017): https://arxiv.org/abs/2202.07382 — phase gradient heap
  integration, propagates phase along the time-frequency gradient
  starting from spectrogram peaks. State-of-the-art quality without
  explicit peak picking.
- Prusa & Holighaus, "A Phase Vocoder Based on Nonstationary Gabor
  Frames" (arXiv:1612.05156, 2016): https://arxiv.org/abs/1612.05156 —
  adaptive resolution + adaptive phase locking.
- Damskagg & Valimaki, "Audio Time Stretching with an Adaptive
  Multiresolution Phase Vocoder" (ICASSP 2017).

### Phase-locked vocoder lineage — paragraphs on each

**Classic phase vocoder (Flanagan/Portnoff).** For analysis hop `Ha` and
synthesis hop `Hs`, every bin's phase advances independently. The
synthesis phase is

```
ω_inst[k] = (2π·k / N) + princ(φ_a[k,n] − φ_a[k,n−1] − 2π·k·Ha/N) / Ha
φ_s[k,n]   = φ_s[k,n−1] + Hs · ω_inst[k]
```

where `princ()` wraps to (−π, π]. With `α = Hs / Ha` we get smooth
horizontal continuity per bin but no coupling between bins. Single
sinusoids stretch fine; mixed material develops "phasiness" — a
reverby/swirly artifact.

**Puckette 1995 — "phase-locked vocoder."** Compute synthesis phase
from a *complex sum* of neighbouring bins, not the bin itself. Effectively
forces every bin to inherit its phase from the locally dominant
sinusoid. Cheap (one complex add per bin). Quality jump on tonal
material.

**Laroche-Dolson 1999 — identity / scaled phase locking.** Generalises
Puckette: explicitly find peaks (a bin whose magnitude exceeds its 4
nearest neighbours), define a *region of influence* between each pair of
adjacent peaks (split at the magnitude minimum or midpoint), and lock
every non-peak bin's phase rotation to that of its peak.

- *Identity phase locking* (rigid): `Δφ_skirt = Δφ_peak`. Skirt bins
  rotate by the same delta as their peak.
- *Scaled phase locking*: `Δφ_skirt = β · Δφ_peak`, where `β` accounts
  for the fact that the skirt sits at a different actual frequency than
  the peak.

CPU cost: one peak scan + one assignment pass per bin. ~O(N). Memory:
a peak list (<<N entries, typically tens of peaks).

**Bonada 2000 — multi-resolution + transient phase reset.** Holds the
stretch factor at 1 during attacks for high frequencies (preserves
transient sharpness) while still stretching the low end (preserves
sustained partials). Avoids the trade-off between pre-echo and
phasiness.

**Roebel 2003 — peak-level transient handling.** Detects transients
*per spectral peak* using the second derivative of the unwrapped phase
(centre-of-energy moves discontinuously across a transient). When a
peak is identified as transient, its skirt phases are reinitialised
from the input directly. No need to force `α=1`. This is what Rubber
Band's R2 engine does conceptually.

**PVSOLA 2011.** Periodically replaces a synthesised frame with a raw
input frame, time-aligned by cross-correlation, then resumes the phase
vocoder. Resets vertical coherence regularly. Quality much better than
classic PV at moderate stretches; works real-time.

**Phase Vocoder Done Right (PVDR) 2017.** No explicit peak-picking.
Computes the phase gradient (∂φ/∂t and ∂φ/∂f) and integrates phase
along that gradient using a heap-based "highest magnitude first" walk
seeded from local maxima. State-of-the-art quality; CPU cost ~O(N log N)
per hop dominated by the heap. Practical on a desktop CPU, but not
free.

### Open-source candidates

| Library | License | Lang | Phase-locking | Real-time | Variable rate | CPU | Maturity | Rust binding |
|---|---|---|---|---|---|---|---|---|
| Rubber Band R2 | GPL/commercial | C++ | Roebel-style transient resets + lamination | Yes (~50 ms latency) | Yes | Low | Mature, used in dozens of DAWs | jlank/rubberband (FFI), no idiomatic crate |
| Rubber Band R3 ("Finer") | GPL/commercial | C++ | Lamination + finer per-frame analysis | Yes (~50 ms+ latency) | Yes | High (much more than R2) | Mature | same as above |
| zplane elastique Pro | Commercial (€€€) | C++ | Proprietary multi-resolution / psychoacoustic | Yes | Yes | Mid | The DAW industry default | None |
| Signalsmith Stretch | MIT | C++11 | "Method 4" from Geraint Luff's ADC22 talk — local complex average + peak energy redistribution; not Laroche-Dolson | Yes (`splitComputation` flag for amortised CPU) | Yes (best 0.75–1.5×) | Mid | Maintained, recently used in Web Audio, audiomentations | bmisiak/ssstretch + colinmarc/signalsmith-stretch-rs |
| Bungee | MPL-2.0 | C++ | "Simple, fast phase-vocoder algorithm" with PFFFT | Yes; granular + streaming modes | Yes (granular mode supports arbitrary playback speed) | Low | Active, claims comparison vs Rubber Band | None |
| `phase_vocoder` (jneem) | MIT | Rust | Basic, no peak-locking | Block API only | Yes | Low | Author note: "very immature, use at your own risk." | (n/a) |
| `pvoc-rs` (nwoeanhinnogaehr) | GPL-3 | Rust | Basic, no peak-locking | Closure-based bin processing | Yes | Low | ~59 commits, no recent activity | (n/a) |
| `pitch_shift` (NathanRoyer) | unspecified | Rust | Ported from `cpuimage/pitchshift`, no peak-locking | Block (128 samples) API; fixed 1024-sample latency | Yes | Low | Single-author, modest | (n/a) |
| `rocoder` (ajyoon) | unspecified | Rust | Naive 3-step phase vocoder | Live-codeable but not RT-safe | Yes | Low | Toy/livecoding | (n/a) |
| Audacity Paulstretch | GPL | C++ | Random-phase smear (NOT PV) | Offline | Extreme stretch only (5–500×) | High | Mature | (n/a) |

### Recommendations for our Rust integration

**Don't wrap Rubber Band.** GPL contagion is a hard blocker for a
proprietary CLAP plugin. Even with a commercial licence, the FFI surface
plus the fact that `Pipeline::process()` already runs a custom
overlap-add STFT means most of Rubber Band's value (its own STFT
infrastructure) is wasted on us.

**Don't wrap Signalsmith Stretch either, even though MIT and tempting.**
Two issues: (a) it owns its own STFT — we'd be running a second
overlap-add inside our existing one, doubling FFT cost and latency for
*just* the Stretch module; (b) Signalsmith's algorithm is optimised for
"one stretch ratio applied to a stream" rather than "fractional
read-pointer into a complex history buffer," which is what Past needs.
The two semantics aren't compatible.

**Write our own minimal Stretch kernel inside the Past module.** The
correct shape:

1. Per bin, compute the per-hop phase advance from the rolling history:
   `Δφ_hist[k,n] = princ(φ[k,n] − φ[k,n−1] − 2π·k·Ha/N)`. Cache this as
   `if_offset[k]` (instantaneous-frequency offset from bin centre).
2. The actual playback frequency of bin `k` is
   `ω_play[k] = 2π·k/N + if_offset[k] / Ha`.
3. To read from history at fractional frame offset `f` (frames, not
   samples), set the synthesis bin to
   `Z[k] = M_lerp[k] · exp(j · (φ_anchor[k] + ω_play[k] · f · Ha))`,
   where `M_lerp[k]` is the magnitude linearly interpolated between the
   two adjacent history frames and `φ_anchor[k]` is the older frame's
   phase plus its own `Δφ`-accumulator.
4. Update `φ_anchor[k]` each output hop by `ω_play[k] · Hs` so bins
   with sub-bin frequency offsets keep advancing correctly.

This is a *minimal* phase vocoder kernel — no peak locking, no
transient detection. CPU: ~1 sin/cos per bin per hop = 8193 ×
44100/256 ≈ 1.4 M sin/cos per second. Use a 256- to 1024-entry sin/cos
LUT with linear interpolation; brings it under 0.2 % of one core.

**Phasiness is acceptable as v1.** The Past Stretch sub-effect lives
next to "Granular" and "Reverse" — modes whose audible character is
*explicitly* time-smeared. A small amount of Laroche-Dolson-class
phasiness will read as "tape-style stretch artifact," which is
desirable. A dedicated `phase_lock: bool` curve switch can expose
Puckette-style phase locking (cheap; one complex sum per bin per hop)
when the user wants cleaner output. Don't ship rigid Laroche-Dolson
peak detection in v1 — it's a meaningful CPU+code-complexity step that
we don't need until users complain.

**Integer-ratio LUTs: don't bother.** The general formula above already
costs <1 % CPU. Special-casing α ∈ {0.25, 0.5, 1, 2, 4} would buy a
few microseconds at the cost of a code-path explosion.

**For Future module's Crystal Ball / Pre-Echo, use the same kernel
backwards.** The Future module's write-ahead echo (Tape Print-Through,
Pre-Echo) needs the same `PhaseRotator` infrastructure if we ever want
echoes to remain phase-coherent at non-unity stretch. Same ~1 sin/cos
per bin per hop budget.

---

## Topic B — Predictive spectral extrapolation

### Key references

- Janssen, Veldhuis, Vries, "Adaptive Interpolation of Discrete-Time
  Signals That Can Be Modeled as Autoregressive Processes" (IEEE ASSP
  1986) — the seminal AR-based audio interpolation algorithm.
- Etter, "Restoration of a Discrete-Time Signal Segment by Interpolation
  Based on the Left-Sided and Right-Sided Autoregressive Parameters"
  (IEEE SP 1996) — bidirectional Janssen variant.
- Vos, "A Fast Implementation of Burg's Method" (Skype/Microsoft, 2013):
  https://www.opus-codec.org/docs/vos_fastburg.pdf — Burg with `Nm + 5m²`
  multiplications instead of `3Nm − m²`. Used by Opus PLC. Per-sample
  cost is dominated by the data-window length `N`, not order `m`.
- Granese & Mokry et al., "Implementation and Optimization of Burg's
  Method for Real-Time Packet Loss Concealment in Networked Music
  Performance Applications" (Personal & Ubiquitous Computing 2024):
  https://link.springer.com/article/10.1007/s00779-024-01806-8 —
  benchmarks Burg on embedded targets; informs feasibility envelope.
- Mokry, "Tweaking autoregressive methods for inpainting of gaps in
  audio signals" (arXiv:2403.04433, 2024):
  https://arxiv.org/abs/2403.04433 — gap-wise AR variant.
- Mokry, "Janssen 2.0: Audio Inpainting in the Time-frequency Domain"
  (arXiv:2409.06392, EUSIPCO 2025):
  https://arxiv.org/abs/2409.06392 — first proper Janssen-in-STFT
  algorithm. Verdict on real-time: *not* feasible (10–20 minutes per
  5-second clip on i7).
- Esquef et al., "A Method for Long Extrapolation of Audio Signals"
  (J. Audio Eng. Soc. 2006).
- Adler et al., "Audio Inpainting" (IEEE TASLP 2012):
  https://elad.cs.technion.ac.il/wp-content/uploads/2018/02/AudioInpainting_IEEEASSP.pdf —
  original "audio inpainting" paper.
- Marafioti et al., "Audio Inpainting: Revisited and Reweighted"
  (arXiv:2001.02480) — sparse-NMF approach, useful background.

### Linear vs AR vs neural prediction

For our use case — predict frame N+1 magnitudes from frames N−K…N at
8193 bins, hop ≈ 256, with K probably in {3, 4, 8} — three options:

**(a) Per-bin linear extrapolation in dB.**
```
m_pred[k] = m[k,n] + (m[k,n] − m[k,n−1])             // Δ-extrapolation
m_pred[k] = m[k,n] + 0.5·(m[k,n] − m[k,n−2])         // 2-tap smoothed
```
in `log10(|·|+ε)` space. CPU: ~3 ops per bin per hop. Total
8193 × 3 ÷ 256 × 44100 ≈ 4.2 M ops/sec — negligible. Predictions are
correct on slowly-varying material, catastrophic on transients (huge
overshoots when log-magnitude jumps 30 dB in one hop).

**(b) Per-bin AR(K) via Burg, K ∈ {2,3,4}.**
Cost per bin per fit: `N·K + 5·K²` multiplications, where `N` is the
analysis-window length (number of past frames considered), typically
8–16. For K=3, N=8: that's 24 + 45 = ~70 mults per bin per fit. At
8193 bins × 172 hops/sec = 1.4 M fits/sec → **~100 M mults/sec**, or
~3 % of one core. *Feasible*. Quality is much better than linear on
mildly evolving spectra; still fails on transients but the failure
mode is bounded by the AR poles staying inside the unit circle (Burg
guarantees stability).

**(c) Per-bin AR(K) via Yule-Walker.** Don't. The Wharton tech note is
unambiguous — Yule-Walker is poorly conditioned for short windows and
can produce unstable poles. Use Burg.

**(d) Frequency-domain Janssen (Janssen 2.0).** Verdict from the 2024
paper: *not real-time*, takes minutes per second of audio. Our use case
also doesn't need its iterative refinement — we want one-step
prediction, not gap restoration.

**(e) Neural prediction.** Modern neural audio codecs (Spectral Codec
arXiv:2406.05298, MelCap arXiv:2510.01903, SpectroStream
arXiv:2508.05207) all do compression / synthesis, not prediction in the
"frame N+1 from N" sense we want. Even a 5-layer CNN inference at
8193 bins per hop would burn >30 % of one core. Skip.

**Recommended default:** *Linear extrapolation in dB* with confidence
gating (see below). Cheapest, correct on the easy case, and the
confidence gate handles the hard case. Upgrade to per-bin Burg AR(2)
later only if we hear a quality difference users care about.

### Per-bin prediction confidence

Two cheap signals correlate well with "is this bin currently
predictable":

1. **Spectral flux per bin.** `flux[k] = |m[k,n] − m[k,n−1]|`. High
   flux = transient = unpredictable. We already compute this (or
   something like it) in the Dynamics envelope follower; cheap to
   share.
2. **Phase-derivative variance per bin.** Compute the running variance
   of `if_offset[k]` (the IF-offset above) over the last 4–8 hops.
   High variance = bin is jumping between sinusoids = unpredictable.
   ~2 mults + 1 add per bin per hop with a Welford-style streaming
   variance.

Confidence-weighted output per bin:
```
c[k] = exp(−α · flux[k]² − β · ifvar[k])    // α, β empirical
m_out[k] = c[k] · m_pred[k] + (1 − c[k]) · m_dry[k]
```

Per-bin downgrades during transients prevent the catastrophic-overshoot
failure mode and take ~5 ops/bin. Total cost still negligible.

A smarter variant: gate the *blend amount* of the entire Future module
by the maximum confidence across bins — when the input is broadly
unpredictable, fade the wet signal out. This avoids per-bin discrete
"some bins predicted, some not" artifacts at the cost of one more 1-pole
follower.

### Phase prediction

Don't predict phase. The literature is consistent on this: phase
unwrapping is fragile, and predicted phase that is wrong by more than
~0.5 radians produces audible cancellation against the dry mix at the
output OLA. Two viable strategies:

- **Reuse current phase (recommended).** Use the predicted *magnitude*
  and the *current* phase. Output bin = `m_pred[k] · exp(j φ[k,n])`.
  This is what Janssen 2.0 effectively does — magnitude is constrained
  to fit the spectrogram, phase emerges from the implicit STFT
  reconstruction.
- **Advance phase by `ω_inst · Hs`.** As we already do for the Stretch
  module, advance by the per-bin instantaneous frequency. This is a
  short-horizon phase prediction that is correct as long as the partial
  is genuinely steady. Combined with magnitude prediction this gives
  what hand-wavy "look-ahead spectrogram" feels like to producers.

### Recommendations

1. **Ship "Predicted-Spectrum Interpolation" sub-effect with linear-in-dB
   prediction + flux-and-IF-variance confidence gating + reused phase.**
   Total CPU: <1 % of one core at 8193 bins / hop 256 / 4× independent
   stereo. Code complexity: ~80 lines.
2. **Defer AR-Burg.** Only revisit if listening tests show users can
   tell the difference. Don't pay the 3 % CPU and the implementation
   complexity speculatively.
3. **Defer neural.** Wrong tool for one-frame prediction; the moment
   we want longer-horizon prediction (multiple frames) we can revisit
   with a small RNN / TCN, but that's its own research project.
4. **Document the failure mode in the UI.** When the confidence-weighted
   blend drops below 0.2, surface a small "transient detected — drying
   out" indicator near the slot. Helps producers understand why the
   effect "disappears" on drum hits.
5. **Reuse the spectral-flux infrastructure that Dynamics needs anyway.**
   No new per-bin state if we share. (BinPhysics already has `flux` per
   the Past audit.)

---

## Topic C — Sub-bin micro-pitch-drift

### Key references

- Laroche & Dolson, "New Phase-Vocoder Techniques" (1999) — sub-bin
  pitch shifting via per-peak phase rotation.
- Bristow-Johnson & Bogdanowicz, "A Fast & Simple Pitch Shifter for
  Real-Time Use" — intra-frame sinusoidal sweep.
- Bernsee, "Pitch Shifting using the Fourier Transform" (zynaptiq blog):
  http://blogs.zynaptiq.com/bernsee/pitch-shifting-using-the-ft/ —
  practical implementation tutorial; the canonical "true frequency from
  phase derivative" code.
- Katja Vetter, "Pitch shifting":
  https://www.katjaas.nl/pitchshift/pitchshift.html — phase-clock
  intuition; explains 4× overlap requirement for ±0.5 bin
  representability.
- Huang & Dong, "A Real-Time Variable-Q Non-Stationary Gabor Transform
  for Pitch Shifting" (Interspeech 2015):
  https://www.isca-archive.org/interspeech_2015/huang15f_interspeech.pdf
  — pitch shift by frequency translation rather than spectral stretch.

### Phase-rotation math

The Punch fill needs to drift bin `k` toward bin `H` by an amount
`d ∈ [0, 0.5]` bins. Bin centre frequency in cycles-per-sample is
`f_c[k] = k / N`. The drift adds `Δf = d · (1/N)` cycles-per-sample.
Per output hop `Hs`, the phase advance is

```
Δφ_hop = 2π · Δf · Hs
       = 2π · d · Hs / N
       = (π/2) · d            for OVERLAP=4 (Hs = N/4)
```

So for a half-bin drift at OVERLAP=4, the per-hop rotation is exactly
`π/4`. Fast: a real-imaginary 2×2 matrix multiply on the bin's complex
value, or equivalently `Z[k] *= exp(j · Δφ_hop)`. Cost: 4 mults + 2
adds per drifted bin per hop. With ~50 active drift sites at 50 fills
per second this is sub-microsecond.

Importantly: the drift `d` is *added on top* of the bin's existing
instantaneous-frequency offset (the `if_offset[k]` we already compute).
So the full phase rotation per hop for a drifted bin is

```
φ[k] ← φ[k] + 2π · (k/N + if_offset[k]/Ha + d/N) · Hs
```

and we precompute `exp(j · 2π · d · Hs / N)` per drift site — a single
complex multiplier reused each hop until drift releases.

**IF-aware correction.** Strictly necessary? *No, not at our scale.*
The reason is that `d ≤ 0.5` bin and the target frequency is well
within one bin of the centre. The classic IF-aware correction matters
when you're shifting by *many* bins — then the partial leaves its bin
of origin and you need to re-place it. Sub-bin drift never crosses a
bin boundary (by construction), so the basic phase-rotation formula is
exact. We can skip the IF-aware path entirely for v1.

**Stereo locking.**
- *Linked* mode: trivially share drift state across channels.
- *Independent* mode: per-channel drift, but use the same `H` (target
  bin) for both channels — sidechain peak detection runs on the M sum.
  Drift phases evolve independently; this gives a tiny stereo width
  bonus to the fill without making it incoherent.
- *MidSide*: drift only on the channel the slot processes (M or S per
  `FxChannelTarget`). Don't cross M↔S.

Memory: `pitch_drift_phase[MAX_NUM_BINS]` per channel = 33 KB per
channel per Punch slot, as the prompt guesses. Free at our scale.

### Drift release strategies

Three candidates from the original prompt:

| Strategy | Behaviour at sidechain release | Audible result | Implementation |
|---|---|---|---|
| Return-to-zero | Phase rotation wound back to zero over `release_ms`, mirroring the carve | Smooth swell + smooth release; symmetry is reassuring | Lerp `d` from current value to 0 over release time |
| Freeze-in-place | `d` retained at its current value; bin keeps being detuned | Permanent (until next carve) micro-detune; can build up over many fills | Stop updating `d`; keep applying the precomputed rotator |
| Slow-drift-to-zero | `d` exponentially decays toward 0 over a long timescale | Natural-sounding "spectrum heals back" — the canonical "Punch" feel | 1-pole follower with τ = 200–500 ms |

**Recommendation:** *Slow-drift-to-zero* as default, with `release_ms`
and `decay_τ` exposed via the Punch HEAL curve. Reasoning:

- Return-to-zero produces a *symmetric* artifact — every Punch event
  has both a forward and reverse motion. Forces a 1:1 ratio between
  attack and release artefacts, which audibly reads as
  "sidechain modulation" rather than "spectrum healing."
- Freeze-in-place accumulates: ten Punch hits in a row will leave the
  spectrum permanently detuned by the sum of ten drift events. Sounds
  drifty in the bad way (long-term pitch wander).
- Slow-drift-to-zero gives the perceptual "the carve happened, then
  the surrounding bins quietly slumped back into place" — exactly the
  metaphor in the brainstorm.

**Edge case — sustained sidechain.** If the sidechain stays loud for
seconds, the carve depth saturates but the drift `d` should *also*
saturate (not keep accumulating). Implementation: cap the per-hop drift
rate `dd/dt` and the accumulated `d` separately. Both clamps live in
the Punch state; ~12 bytes per drift site.

**Edge case — overlapping drift sites.** Two adjacent peaks both want
bin `k` to drift toward different targets. Resolution: the drift target
is the loudest peak in `k`'s neighbourhood at the current hop;
re-evaluate every hop. The `d` accumulator carries over but the
direction may flip — natural-sounding wobble.

### Recommendations

1. **Skip IF-aware corrections for sub-bin drift.** Basic phase rotation
   is exact at `|d| ≤ 0.5` bins. Saves code and CPU.
2. **Default to slow-drift-to-zero release.** Expose `release_τ` via the
   HEAL curve.
3. **Per-channel drift state, mono peak-detection.** Best
   stereo-character compromise.
4. **Pre-compute the per-hop rotator `exp(j · Δφ_hop)` per active drift
   site** and cache until release. Avoids one transcendental per drift
   site per hop.
5. **Limit active drift sites to 64 per channel.** With 50–60 sidechain
   peaks active simultaneously you've already saturated the perceptual
   space; more is just CPU.

---

## Cross-topic synthesis

All three modes share **"manipulate `Complex<f32>` STFT bins
phase-coherently across hops based on a per-bin frequency offset and a
time delta."** Concretely:

- **Stretch** reads from a fractional history-frame position; the time
  delta is `(read_pos − write_pos) · Ha`.
- **Future / Predict** writes to the *current* output frame using a
  forward time delta `+Hs`.
- **Punch fill** rotates phase by `+Δφ_hop = 2π · d · Hs / N` per
  active drift site.

A shared helper makes sense:

```rust
pub struct PhaseRotator {
    sin_lut: &'static [f32; 1024],   // 1 KB shared
    cos_lut: &'static [f32; 1024],
}

impl PhaseRotator {
    /// Rotate `bin` by `freq_offset_hz · time_delta_samples / sample_rate`.
    /// Caller passes the precomputed instantaneous-frequency offset; this
    /// helper just does the complex multiply with a LUT-interpolated sin/cos.
    #[inline]
    pub fn rotate(
        &self,
        bin: Complex<f32>,
        freq_offset_hz: f32,
        time_delta_samples: f32,
        sample_rate: f32,
    ) -> Complex<f32> { ... }
}
```

Place under `dsp::utils` (already exists) or a new `dsp::phase`
module. Shared also by the Modulate (PLL Tear) and Harmony (Inharmonic)
modules in the longer-term roadmap. A 1024-entry sin/cos LUT with linear
interpolation rounds the per-call cost to ~6 mults + 3 adds — well
inside the budget for any single module call site.

A second shared piece: the **per-bin instantaneous-frequency cache**.
We already need `if_offset[k]` for Stretch; Punch needs it (to add to
its drift); Future's phase-advance reuse needs it. Cache it once per
hop in `Pipeline::process()` after the analysis FFT, before dispatch
to FxMatrix. Shape: `Vec<Vec<f32>>` of `[channel][bin]`; size ≈ 2 ×
8193 × 4 = 65 KB; fill cost ≈ 16k subtract+wrap ops per hop, ~0.3 % of
one core.

This cache is then a "free input" for any module that wants
phase-coherent operations. Add a field to `ModuleContext`:

```rust
pub struct ModuleContext<'a> {
    // ...existing fields...
    pub if_offset: &'a [f32],   // num_bins, per current channel
}
```

This unifies Topic A/B/C and any future phase-aware module without
forcing each module to recompute the same data.

---

## Open questions

1. **Phasiness budget for Stretch.** Need a listening test to confirm
   that producers will accept Puckette/lamination-style phase coherence
   instead of full Laroche-Dolson rigid phase locking, in exchange for
   a 2× CPU saving and ~200 LOC less code. *Assumption:* Past Stretch
   is positioned as "tape-style stretch artifact"; the artifact is the
   feature. If Kim's listening notes contradict this, the upgrade path
   is well-understood (add peak detection, wire it through the same
   `PhaseRotator`).
2. **Confidence-gating threshold for Future.** What's the right
   `(α, β)` in the confidence formula? Will need empirical tuning on
   3–4 reference tracks (sustained pad, drum loop, vocal, full mix).
   Suggest a "confidence floor" curve in the Future module so the user
   can override.
3. **Per-bin AR-Burg as a future option.** If we *do* eventually add
   AR(2) as a quality upgrade, do we expose it as a sub-effect mode or
   a hidden quality knob? Hidden quality knob is simpler; sub-effect is
   more discoverable.
4. **Shared `if_offset` cache lifetime.** Filling it at the top of every
   STFT closure is correct, but stale-data risk: if a module mutates
   bins *before* a downstream module reads `if_offset`, the cache no
   longer matches the downstream module's input. *Resolution:* the cache
   is always "as of the analysis FFT, before any FxMatrix processing."
   Modules that care about *post-processing* IF must compute it
   themselves. Document this in `ModuleContext` doc-comment.
5. **Drift saturation cap in Punch.** Need a value for the maximum
   accumulated `d` per bin. Tentatively 0.5 bins (don't cross bin
   boundaries). If user-driven curves push beyond this, clamp silently
   — the hard limit is structural, not configurable.
6. **Stereo locking matrix.** For Stretch and Future, when stereo mode
   is Independent, do we want both channels to read the same fractional
   history offset (mono lag/lead but stereo content) or independent
   offsets per channel (potentially stereo flam)? Current
   recommendation: same offset, independent content. Same for Future's
   prediction frame. Confirm in design review.
7. **Phaseness measurement.** Laroche-Dolson's `D_M` consistency measure
   is a good objective metric. Worth wiring as a calibration probe so
   we can A/B different Stretch implementations objectively, not just
   subjectively.
8. **Crystal Ball / one-frame look-ahead.** Per the Future module audit,
   this is a simple primitive (output the next-hop's spectrum as the
   current output). It could share infrastructure with Predict, but
   architecturally it's a *delay*, not a phase rotation. Probably best
   to keep separate. Open whether to ship at all in v1 — the audit
   recommends deferring.
9. **Buffer alignment for Past Stretch + Future.** The HistoryBuffer
   already exists and is shared by Past. The Future module's
   write-ahead buffer is independent. Should there be a *single* shared
   ring of complex frames, with read offsets in both directions?
   Architecturally cleaner; memory cost identical (one big ring instead
   of two smaller ones); risk is making Past dependent on Future
   shipping. Defer the decision until both modules are unblocked.
10. **Worth pursuing PVDR (Phase Vocoder Done Right) for v2?** It is the
    objective state of the art (per arXiv:2202.07382). CPU is acceptable
    on desktop. Implementation is non-trivial — the heap-driven phase
    integration is ~500 LOC of careful index arithmetic. Verdict: tag
    as "v2 quality upgrade if users complain about Stretch artifacts."

---

## Appendix: minimal Rust skeleton

For Topic A, the inner loop of the Stretch kernel — illustrative, not
production:

```rust
// Inputs:
//   history: &[Vec<Complex<f32>>]  rolling, length = history_frames
//   read_pos: f64                  fractional read position in frames
//   write_pos: usize               next slot for analysis to write into
//   if_offset: &[f32]              per-bin IF offset (cycles/sample), num_bins
//   sample_rate, fft_size, hop_a, hop_s
// Output:
//   out: &mut [Complex<f32>]       num_bins
//
// State (per slot, per channel):
//   phase_anchor: Vec<f32>         num_bins; persists across hops

let n = read_pos.floor() as usize % history_frames;
let frac = (read_pos - read_pos.floor()) as f32;
let next_n = (n + 1) % history_frames;

for k in 0..num_bins {
    let m_lerp = (1.0 - frac) * history[n][k].norm()
               +        frac  * history[next_n][k].norm();
    // Per-hop phase advance from history's instantaneous frequency
    let omega_play = (k as f32) / (fft_size as f32) + if_offset[k];
    phase_anchor[k] += 2.0 * PI * omega_play * (hop_s as f32);
    // Wrap to (-π, π] for numerical stability — actual sin/cos doesn't care
    phase_anchor[k] -= 2.0 * PI * (phase_anchor[k] / (2.0 * PI)).round();

    let (s, c) = phase_anchor[k].sin_cos(); // replace with LUT lerp
    out[k] = Complex::new(c * m_lerp, s * m_lerp);
}
```

Cost: ~12 ops + 1 sin/cos per bin per hop. Replace `sin_cos` with the
shared `PhaseRotator` LUT for production use.

For Topic B, the inner loop of Predict:

```rust
// State (per slot, per channel):
//   prev_log_mag: [Vec<f32>; K+1]   ring of last K+1 frames' log magnitudes
//   flux_smooth:  Vec<f32>          1-pole flux follower per bin
//
for k in 0..num_bins {
    let log_m = (current_mag[k] + 1e-9).log10();
    let log_m_prev = prev_log_mag[ring_idx][k];
    let flux = (log_m - log_m_prev).abs();
    flux_smooth[k] = 0.9 * flux_smooth[k] + 0.1 * flux;

    // Linear prediction in dB
    let log_m_pred = log_m + (log_m - log_m_prev);
    let m_pred = 10f32.powf(log_m_pred);

    // Confidence gate
    let c = (-2.0 * flux_smooth[k] * flux_smooth[k]).exp(); // [0,1]
    let m_out = c * m_pred + (1.0 - c) * current_mag[k];

    // Reuse current phase
    let phase = current_bin[k].arg();
    out[k] = Complex::from_polar(m_out, phase);

    prev_log_mag[ring_idx][k] = log_m;
}
```

Cost: ~15 ops + 1 exp + 1 powf + 1 atan2 + 1 sin_cos per bin per hop.
Replace `powf` with `exp10f` and `atan2`+`sin_cos` with cartesian
shortcuts where possible. Total ~3 % of one core at 8193 bins.

For Topic C, the inner loop of Punch fill drift:

```rust
// State (per slot, per channel):
//   active_drifts: SmallVec<[(target_bin, neighbour_bin, d, rotator); 64]>
//
for &(target, k, d, rot) in active_drifts.iter() {
    bins[k] = bins[k] * rot;   // complex multiply
}

// Per-hop release: decay d toward 0
for drift in active_drifts.iter_mut() {
    drift.d *= release_decay;     // 1-pole at τ = HEAL curve value
    drift.rotator = ...           // recompute only if d changed by > epsilon
}
```

Cost: 4 mults + 2 adds per drift site per hop. With 64 active sites at
hop=256: ~14k ops/sec, lost in the noise.
