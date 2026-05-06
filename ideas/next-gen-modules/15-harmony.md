# Harmony Module — Audit

**Existing spec:** `docs/superpowers/specs/2026-04-21-harmony-module.md`
**Status:** DEFERRED, depends on BinPhysics + IF infra.
**Source brainstorm:** Category 1 of Original brainstorm (lines 56–75),
brainstorm "Bin pitch shift with logic for periodic inharmonicity"
section (lines 23–32), the harmonic series ideas, the FM replicator,
and `docs/future-ideas/harmony effects.txt` (entire file).

This is the densest module by gap count. Harmony has more uncovered
brainstorm ideas than any other.

## What the spec covers

4 sub-effects: `Chordification`, `Undertone Generator`, `Harmonic
Companding`, `Formant Rotation`. IF infrastructure prerequisite. 12×12
harmony probability matrix. Curves: AMOUNT, THRESHOLD, SPREAD,
STABILITY, MIX.

## Brainstorm cross-reference

| Idea | In spec? | Kim's note | Action |
|---|:---:|---|---|
| Spectral Chordification (#1) | ✓ | "Easiest here would be to track midi" | done — but **MIDI integration glossed over** |
| Undertone Generator (#2) | ✓ | "requires accurate pitch tracking" | done |
| Formant-Preserving Harmonic Rotation (#3) | ✓ | "Calculate in realtime how?" | done — **but realtime cepstral cost not addressed** |
| 3b: Bin shuffler with reach curve | ✗ | (degraded version of formant rotation) | **GAP — interesting cheap variant** |
| Spectral Arpeggiator (#4) | (in Rhythm spec) | (covered) | done elsewhere |
| Bin pitch shift with periodic inharmonicity | ✗ | "Stiffness Curve / Bessel / Prime Multiples" | **GAP — major** |
| Stiffness Curve (Piano/String): fn = n·f0·sqrt(1+B·n²) | ✗ | listed as option | **GAP — sub-effect** |
| Bessel Functions (Bells/FM): partials at Bessel roots | ✗ | listed as option | **GAP — sub-effect** |
| Prime Multiples: dense non-beating dissonances | ✗ | listed as option | **GAP — sub-effect** |
| FM Replicator: 16-op approximation of spectrum | ✗ | "anything interesting that can be done when dealing with only 16 oscs" | **GAP — sub-effect** |
| Harmonic Series Generator for loud partials | ✗ | "finds five loudest partials, then creates a harmonic series" | **GAP — sub-effect** |
| Harmonic series among sustained partials, transient inclusion | ✗ | "How hard would it be to find prospective harmonic series among all sustained partials?" | **GAP — research question** |
| Cepstral Liftering (#18) | ✗ | "need more info. Is this like DDSP?" | **GAP — sub-effect, with explanation in §a** |
| Tuning Fork Intermodulation (#15) | (in Kinetics) | "everything like this forever yes" | done elsewhere |
| Ground Loop (60Hz Hum, combined with subharmonic) | ✗ | "kind of bland, maybe combine with the subharmonic generator" | **GAP — refinement of Undertone** |
| Quantising graph by Y to note frequencies | (in `02-architectural-refactors.md` §8) | "MEL-bands? Cepstral math?" | done elsewhere |

## What "is Cepstral Liftering like DDSP?"

Kim's question deserves a direct answer.

**No.** DDSP (Differentiable Digital Signal Processing, Engel et al.
2020) is a *trainable* synthesis architecture — it parameterises a
classical vocoder (harmonic + noise + filter) and trains a neural net
to drive its parameters from input audio. It needs training data,
inference at runtime, and is fundamentally an ML system.

**Cepstral Liftering** is pure DSP — no ML, no training, no
data-dependence. The math:

1. Take the FFT magnitudes (already have them).
2. Apply log: `log_mag[k] = log(magnitude[k] + epsilon)`.
3. Take the inverse real FFT of the log magnitudes — the result is
   called the **cepstrum**, with units of "quefrency" (seconds, but
   in a different domain).
4. The cepstrum's low quefrency = slow envelope variation = formants /
   timbre. High quefrency = fast variation = pitch / harmonics.
5. **Edit the cepstrum** — multiply by a window that lets you boost,
   cut, or filter the formant region or the pitch region.
6. Forward FFT back to log-magnitude space.
7. Exponentiate to get edited magnitudes.
8. Re-apply the original phase. Inverse FFT to time domain (already
   handled by the existing OLA path).

The user can EQ the *envelope* (vowel character) without touching the
*pitch*, or vice versa. State of the art for vowel morphing, voice
character changes, and the formant-preservation step in pitch shifters.

It is older and simpler than DDSP. Cost: one extra real-FFT and one
real-IFFT per hop. With `realfft` already in deps, this is one
allocation and ~3 ms per 8192-bin FFT on a desktop.

## Gap details

### a) Cepstral Liftering (idea #18)

**Sub-effect proposal: `Lifter` mode**

Two curves shape the cepstrum:

- **Envelope curve:** scales the low-quefrency part — boosting
  brightens the vowel, cutting dulls it.
- **Pitch curve:** scales the high-quefrency part — boosting sharpens
  harmonics, cutting smooths into noise.

The X axis of these curves is *quefrency*, not frequency. The display
needs a new `CurveAxisMode { Quefrency }` per `02-architectural-
refactors.md` §8. The Y axis is multiplicative gain (1.0 = no change).

- Reads: bin magnitudes + phases.
- Writes: bin magnitudes (re-built from edited cepstrum).
- State: `cepstrum_buf[fft_size]` (real f32), `log_mag_buf[num_bins]`,
  one extra real-FFT plan.
- Curves: AMOUNT, ENVELOPE_GAIN (curve over low quefrency),
  PITCH_GAIN (curve over high quefrency), MIX. THRESHOLD unused; new
  curve labels needed — this is a 4-curve mode.
- CPU: medium (extra FFT pair).

### b) Stiffness / Bessel / Prime inharmonicity (the bin-pitch-shift idea)

Three musically-meaningful warpings of the harmonic series, all
implementable by repositioning loud partials:

#### `Stiffness` — Piano/String inharmonicity

Formula: `f_n = n × f_0 × sqrt(1 + B × n²)` where B is the stiffness
coefficient (curve-driven across the spectrum). At B=0, partials lie
at integer multiples; at B>0, they spread upward — the classic piano
"tubular" sound.

#### `Bessel` — Bells / FM character

Snap partials to the roots of Bessel functions of various orders.
Specifically, the n-th partial of a circular-membrane mode at
`α_{m,n} × f_0`, where α are the Bessel zeros.

This gives a characteristic metallic / FM-bell sound. The curve picks
which Bessel order the spectrum uses across frequency.

#### `Prime` — Prime-multiple dissonance

Partials at `f_0 × p_n` where p_n is the n-th prime (2, 3, 5, 7, 11,
13, …). Creates dense, non-beating dissonances — there's no shared
fundamental, so partials don't reinforce each other.

#### Implementation

All three take the *existing* loud partials (found via IF +
amplitude) and *move* them to new positions. This is per-bin pitch-
shifting using IF as the source frequency.

- Sub-effect: `Inharmonic` mode with a sub-mode selector
  `{ Stiffness, Bessel, Prime }`.
- Reads: IF array, magnitudes.
- Writes: bin magnitudes (zeroed at source position, accumulated at
  target position).
- State: target-position scratch buffer.
- Curves: AMOUNT, COEFFICIENT (B for stiffness, Bessel order for
  Bessel, prime offset for Prime), THRESHOLD (which bins count as
  "partials"), MIX.
- CPU: medium (one peak-find pass + one accumulation pass).

This is a single sub-effect with a sub-mode rather than three sub-
effects — same code path, different formula.

### c) FM Replicator — 16-operator approximation

**Concept.** Take the input spectrum and approximate it with at most
16 sine partials, each of which can frequency-modulate one other
partial. The "16-op DX7" has well-defined sound character; turning it
on a real spectrum gives a robotic / DX-flavoured re-synthesis.

**Sub-effect proposal: `FM Replicator` mode**
- Reads: IF + magnitudes (find 16 loudest stable partials).
- Writes: zero out original spectrum, write 16 sine partials with FM
  pair targets.
- State: `partial_table[16] = { freq, amp, mod_target }`. Recomputed
  per hop.
- Curves: AMOUNT (wet/dry against the original spectrum), STRENGTH
  (modulation depth), THRESHOLD (minimum partial amplitude), MIX.
- CPU: light (only 16 oscillators, tiny inner loop).

Integration with the harmony probability matrix: optionally, the 16
partial frequencies can be quantised to scale (chordification on top
of FM Replicator) for more musical re-synthesis.

### d) Harmonic Series Generator for loud partials

**Concept.** Find the 5 loudest stable partials. For each, *generate* a
harmonic series above it (2f, 3f, 4f, …) at curve-controlled
amplitude decay. The output is the original spectrum + synthesized
harmonic series — like turning every loud partial into a small organ
pipe.

**Sub-effect proposal: `Harmonic Generator` mode**
- Reads: IF + magnitudes (top-K peak detection).
- Writes: additive bin magnitudes + phases at harmonic positions.
- State: per-partial harmonic-position list.
- Curves: AMOUNT (harmonic series amplitude), STRENGTH (number of
  harmonics generated, integer 2-32), THRESHOLD (peak detection),
  SPREAD (harmonic decay rate), MIX.
- CPU: light (O(K × harmonic_count) where K ≤ 5).

This is the inverse operation of Undertone — Undertone goes down (1/2,
1/3), Generator goes up (×2, ×3). Together they let the user
synthesise a complete extended harmonic structure from a sparse input.

### e) Sustained-harmonic-series detection + transient inclusion

Kim's brainstorm question: "How hard would it be to find prospective
harmonic series among all sustained partials? For transients, if doing
noisy/sustained split, could look for a transient at the beginning of a
sustained resonance recognised as harmonic and include relevant bins
from the transient period."

**Translation.** Identify groups of sustained partials whose IFs
satisfy harmonic ratios (within tolerance). Each group is a "voice." The
user can then process voices independently (e.g. compress them as a
group).

This is a piece of analysis-side infrastructure, not a sub-effect by
itself. Several Harmony sub-effects benefit from it — Companding can
identify "what IS the fundamental and what ARE its harmonics." Today's
spec uses MIDI or pitch-tracking to find the fundamental; this adds a
fully-spectral alternative.

**Plan-of-record:** add `harmonic_group_detect()` to the IF
infrastructure block in `01-global-infrastructure.md` §3. Output is a
list of `(fundamental_freq, [harmonic_bin_indices])` per hop. Multiple
sub-effects subscribe.

The "transient inclusion" idea is more involved: link the transient
energy at the *attack* of a sustained note to the same harmonic group.
Requires:

1. T/S Split (already shipping) tells us which bins are transient.
2. Harmonic-group detector tells us which bins are sustained harmonics.
3. We retroactively associate transient bins that occurred immediately
   before a sustained group's first detection — they belong to the
   same voice.

This is a 3-stage analysis pipeline. Worth it? Maybe, but only after
the simpler sustained-only detection is in. Defer the transient-link
piece to v2.

### f) Ground Loop (60 Hz Hum) + Subharmonic combo (Kim's note)

The Ground Loop idea (#16 from the circuit list) was rejected by Kim
in Circuit, with the suggestion: "maybe combine with the subharmonic
generator."

**Refinement to `Undertone Generator`.** Add an optional "60 Hz
modulation" sub-mode:

- Generate a low-frequency "ground hum" at user-controllable freq
  (50/60/100/120 Hz) and depth.
- Modulate the undertone series amplitudes with that hum.
- The hum *also* intermodulates with loud partials per existing
  Power Sag math — louder signals beat against the ground hum.

This adds a "vintage power supply" tonal character to the Undertone
sub-effect specifically. Not a new sub-effect, just a parameter on the
existing one.

### g) Bin Shuffler (Kim's #3b)

Kim noted: "3b could be just bin shuffler with the graph setting the
distance at where a bin will look for a pair."

**Concept.** For each bin K, look at bin K+offset and swap them
according to a curve-controlled probability. Cheap version of formant
rotation that does not actually preserve formants but creates similar
"shifted character."

**Sub-effect proposal: `Shuffler` mode**
- Reads: bin pairs.
- Writes: swapped pairs (probabilistically).
- State: rng state.
- Curves: AMOUNT (shuffle probability), REACH (max swap distance),
  THRESHOLD (only shuffle bins above this magnitude), MIX.
- CPU: trivial.

A surprisingly useful "lo-fi formant shift" effect that requires no
cepstral math. Worth shipping alongside the proper Formant Rotation
sub-effect.

## Curve set

The Harmony module is large enough that it deserves up to 6 curves to
avoid overloading the names. Some sub-effects will use very different
curve sets:

| Idx | Label | Used by |
|---|---|---|
| 0 | AMOUNT | All modes |
| 1 | THRESHOLD | Most modes — partial-detection floor |
| 2 | STABILITY | Modes that detect stable partials (Undertone, Harmonic Gen, FM Replicator, Inharmonic) |
| 3 | SPREAD | Undertone (decay), Harmonic Gen (decay), Lifter (envelope curve), Inharmonic (per-partial deviation), Shuffler (reach) |
| 4 | COEFFICIENT | Inharmonic (B / Bessel order / prime offset), Ground-loop hum freq |
| 5 | MIX | All modes |

6 curves. **Recommendation:** bump `NUM_CURVE_SETS` for *display* in
this module to 6 (the system limit is 7). Or stay at 5 and let
COEFFICIENT and SPREAD share a curve slot per active mode.

Actually — the existing global `NUM_CURVE_SETS = 7` accommodates this
without any change. Just declare `num_curves() = 6` for Harmony.

## CPU class

Cepstral Liftering, FM Replicator, Inharmonic, and Harmonic Generator
all need either an extra FFT or per-partial loops. Tag `heavy_cpu =
true` for the module. Per-mode flag would mark Chordification and
Shuffler as light.

## BinPhysics interactions

Reads: `mass` (Companding — heavy bins resist companding), `velocity`
(modes that prefer "sustained"), `crystallization` (stable partials
get bonus weight in detection).
Writes: `crystallization` (Chordification snaps stable partials into
scale slots, increasing their crystallization).

## Calibration probe set

- `probe_amount_pct`
- `probe_threshold_db`
- `probe_active_mode_idx`
- `probe_active_inharmonic_submode` (Stiffness/Bessel/Prime if
  Inharmonic is the active mode)
- `probe_partial_count` (FM Replicator, Harmonic Gen — number of
  detected partials)
- `probe_chord_strength` (Chordification — magnetism of the
  current snap)
- `probe_chromagram_at_k` (which pitch class the bin is being snapped
  toward)

## RESEARCH PROMPT — SOTA real-time pitch tracking with phase information

```
Topic: State-of-the-art real-time pitch tracking for spectral plugins
using both magnitude and phase.

Context: Plugin has STFT data per hop (8193 bins at MAX, hop=128–512).
Wants per-hop pitch tracking that:
- Resolves bass notes (low E ≈ 82 Hz) below the FFT bin width at 512
  samples (87 Hz) using Instantaneous Frequency from phase derivatives.
- Survives polyphonic input (chord detection, not just single-note).
- Outputs (a) a list of fundamentals + harmonic groups, (b) a 12-element
  pitch-class profile (chromagram), (c) per-fundamental confidence.
- Costs <2 ms per hop on a modern desktop CPU at 8193 bins.

Specific questions:
1. Phase-derivative IF is well-known (Puckette, Brown). What's the
   modern best practice for *phase unwrapping* at fast frequency
   modulation (vibrato, pitch bends)? PVX repository math? Is there a
   newer reference?
2. For polyphonic chord tracking from a chromagram, the brainstorm
   suggests a 1-layer GRU or a hardcoded heuristic 12×12 matrix. What
   are recent (2024-2026) lightweight alternatives that fit in <0.5 ms
   inference at audio rates?
3. For finding "harmonic groups" of partials (a fundamental + its
   harmonics), what is the modern equivalent of Klapuri's iterative
   approach? Is there a single-pass spectral-peak-clustering algorithm
   that's cheap enough to run per hop?
4. Cepstrum-based pitch tracking is older. Does it complement IF, or
   is IF strictly better at our hop rates?
5. PVX-style "phase unwrapping" for ducking smoothness (per Kim's
   brainstorm intro) — how does this integrate with peak detection?
   Is it a separate post-processing step or can it be folded in?

Deliverable: A reference architecture for the IF + chromagram + harmonic-
group pipeline, with literature citations from 2024-2026 (or older if
nothing newer is better), and Rust pseudocode for the per-hop loop.
```

## RESEARCH PROMPT — Cepstral liftering edge cases

```
Topic: Real-time cepstral liftering across rapid spectral changes.

Context: Cepstral liftering relies on log-magnitude FFT → inverse FFT →
cepstrum edit → forward FFT → exp. Per hop, at 8193 bins, takes ~6 ms
of FFT cost.

Specific questions:
1. log(0) = -inf — what's the right epsilon clamp without audibly
   distorting quiet bins?
2. When the input is silent or near-silent, the cepstrum is mostly
   noise. The output spectrum after liftering will amplify that noise.
   How to skip the liftering on silent frames without click artifacts
   at the silence boundary?
3. Phase: cepstral liftering edits magnitude only. The original phase
   is reused. For sustained tones this is fine. For transients, phase
   coherence with the edited magnitudes is broken. Audible result?
4. Real-time formant morphing (vowel A → vowel E): is naive cepstral
   liftering sufficient, or do we need per-frame envelope warping
   (e.g. the "world vocoder" approach)?

Deliverable: Recommended epsilon, silence detection, and phase
treatment for our use case, with literature reference.
```

## Open questions

1. **Cepstral Liftering ship-or-shelve:** ship per the explanation in
   §a — it's tractable, useful, and low-ish CPU.
2. **Inharmonic mode with sub-mode selector** (Stiffness/Bessel/Prime):
   one sub-effect or three? One is cleaner.
3. **FM Replicator** belongs in Harmony (per its 16-op pitched
   character) or Modulate (per its FM nature)? Lean Harmony — the
   *pitched* spectrum re-synthesis is the point.
4. **Harmonic Generator and Undertone:** keep as separate sub-effects
   or merge into "Harmonic Series Builder" with up/down direction?
   Separate is clearer.
5. **Sustained-harmonic-series detection:** ship, but only the
   sustained-only version; defer transient-linking to v2.
6. **Curve count = 6** (one over default 5) — confirm.
7. **Cepstral X-axis display:** see `02-architectural-refactors.md` §8.
8. **PLPV phase unwrapping integration:** see `20-plpv-phase-cross-cutting.md`.
9. **Ground Loop refinement of Undertone:** ship as an Undertone
   parameter, not a new sub-effect.

## Research findings (2026-04-26)

See `research/02-pitch-and-cepstral.md` for the full digest covering
pitch tracking, harmonic-group detection, cepstral liftering, and chord
recognition. Validated decisions:

1. **Do not integrate any neural pitch model on the primary path.** The
   classical pipeline `IF refinement → Klapuri-style harmonic
   summation → IF-refined HPCP chromagram → 24/60-template chord matcher
   (cosine + bigram smoothing)` is ~500 lines of pure Rust on top of the
   existing STFT, sub-millisecond per hop, and matches the brainstorm's
   intent. CREPE / PESTO / RMVPE / SwiftF0 / FCPE all duplicate work we
   already do (per-bin phase / IF) and add 100 KB-100 MB of model
   weights for accuracy that classical methods deliver.
2. **Recommend chord-template depth = 60** (maj/min/dim/aug/sus2/sus4/7),
   not 24. <1 µs per hop and materially improves Chordification on
   expressive material.
3. **Reserve neural models as opt-in escape hatches:**
   - **PESTO RT** (130 k params, ONNX, ~0.7 ms) — for monophonic
     singer formant tracking. Hot-switch when chromagram entropy
     indicates monophonic material.
   - **BasicPitch** (Spotify, ONNX, with `basicpitch.cpp` C++ port) —
     the *only* realistic neural polyphonic detector. Use for *offline*
     MIDI export, not real-time DSP.
   - **RMVPE** — for "extract vocal pitch from a mix" if a karaoke
     feature ever lands.
4. **Hard avoid:** MT3, MR-MT3, YourMT3+, ChordFormer (all >100 M
   parameter Transformers, GPU-required, offline-only); Melodyne DNA
   (patented and closed); cepstral pitch as a *primary* F0 source (IF
   strictly beats it at our hop rates — keep cepstral as a *consistency
   check*).
5. **Cepstral Liftering** — ship `naive cepstrum` as the default Lifter
   mode (~2 extra FFTs/hop, weekend of work). Add **Roebel 2005 True
   Envelope** as `HQ mode` (1-2 weeks, iteration-budget tuning).
   Defer **WORLD CheapTrick** until F0 detection lands — its win is
   the F0-aware analysis window, which only pays off when F0 is
   reliable. ε = 1e-10 in magnitude-squared for `log` clamp; silence
   bypass with smooth cross-fade at –60 dBFS RMS default.
6. **Shared infrastructure** — Topic A (pitch) and Topic B (cepstral)
   share `ModuleContext::cepstrum_buf` and `ModuleContext::stable_peaks`.
   Don't over-engineer: declare `needs_cepstrum: bool` in `ModuleSpec`
   and let Pipeline compute on demand. Use the chromagram entropy from
   Topic A to drive the F0-confidence input to CheapTrick.
7. **WORLD CheapTrick (BSD-licensed, ~300 lines to port to Rust)** is
   worth borrowing for the Lifter HQ mode and any future formant-aware
   pipeline.
