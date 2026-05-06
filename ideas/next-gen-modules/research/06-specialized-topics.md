# Research: Specialized Topics — Orbits / Homology / Holes / Buchla Envelopes

**Source prompts:** 5, 13, 14, 15 in `90-research-prompts.md`
**Status:** Findings as of 2026-04-26

This document covers four research questions that fall outside the bigger
clusters (PVX/IF, simd analog, diffusion, prediction). They share a property
in common: each is a *low-priority unblocker* — completing it enables one
specific sub-feature, not a whole module. The recommendations below are
written so a reader can decide whether to ship that sub-feature in v1, defer
it, or scrap it.

## Refined research questions

**Topic A — Orbital sub-effect.** Given a "massive bin" (loud peak) at index
`K` and a satellite at index `K±n`, what is the cheapest formula that
produces an *audible orbit* — sounds like a coupled vibrato/chorus, not like
random phase noise — when applied per STFT hop, with multiple massive bins
perturbing each other?

**Topic B — Persistent-homology peak gating.** Can we run persistent-homology
analysis on the 2-D `time × frequency` magnitude landscape every K hops,
extract the M most persistent maxima, and use them to gate the current
hop's bins, *within a real-time CPU budget* (say <2 ms per analysis at 64
frames × 8193 bins)?

**Topic C — Punch hole-and-fill perceptual quality.** When a sidechain
carves a magnitude hole in the input spectrum, do neighbouring bins fill
the hole more musically by **amplitude boost** or by **pitch drift**? What
is the right healing-curve shape and time constant? Is a "watch range"
curve worth the extra parameter slot?

**Topic D — Buchla-281 envelopes for RM gating.** Per-bin amplitude envelopes
(potentially 65k of them across 8193 × 4 channels × 2 stereo) need to gate a
ring-modulated bin spectrum, retaining the "bongo / Buchla 281" character
without sounding like a noise gate. What envelope shape, trigger detector,
and SIMD strategy?

---

## Topic A — Phase-orbit math for Orbital sub-effect

### Key references

The exotic "Kepler-style orbits in audio" search returned essentially
nothing — this is genuinely uncharted territory in the audio DSP literature.
However, the underlying mechanic has well-known cousins:

- **Frequency-domain phase modulation as chorus** — Driedger and Müller's
  DAFx/SMC line of papers and Dennis Cronin's writeups discuss DFT-bin
  phase rotation as a chorus mechanism, where modifications stay
  per-bin and never transfer energy across bins
  ([dsprelated.com](https://www.dsprelated.com/freebooks/sasp/Frequency_Modulation_FM_Synthesis.html),
  [dennis cronin DSP samples](https://denniscronin.net/dsp/vst.html)).
- **Acoustic feedback decorrelation via per-bin phase synthesizer** — the
  recent "Phase Synthesizer for Decorrelation" paper
  ([arXiv 2510.12377](https://arxiv.org/html/2510.12377v1)) shows that
  small per-bin phase rotations applied each hop produce convincing
  decorrelation/chorus without timbre damage. This is the closest existing
  precedent to what Orbital is doing.
- **McAulay/Quatieri partial tracking** as a model for "satellite phase
  follows fundamental phase" — when a peak migrates between hops, its
  skirt bins inherit a related phase update
  ([CCRMA PARSHL](https://ccrma.stanford.edu/~jos/parshl/parshl.pdf)).
- **Symplectic Euler integrator** is the textbook cheap method for orbit
  simulation
  ([Wikipedia symplectic integrator](https://en.wikipedia.org/wiki/Symplectic_integrator)).
- **Vibrato/chorus difference** — chorus = pitch-modulated delay; vibrato =
  pure pitch modulation. Frequency-domain phase rotation alone can
  produce *vibrato-flavoured* chorus without time delay
  ([landr modulation primer](https://blog.landr.com/modulation-effects-flanger-phaser-chorus/)).

The literature gap is real: there is no published "spectral mass-attractor
orbital phase modulation" effect. Whatever ships will be original work.

### Math choice

The decision is between three candidates of increasing cost and complexity:

| Option | Per-hop cost (per satellite) | What it sounds like | Recommended |
|---|---|---|---|
| **Linear phase rotation** Δφ = α · S_K / d² | 1 mul + 1 add | Coupled vibrato. The satellite drifts forward in phase relative to its master peak. | **Yes — v1** |
| **Symplectic-Euler "mini orbit"** keep `(φ, ω)` per satellite, force = α · S_K · sin(φ_K − φ) / d² | 1 sin + 4 muls + 2 adds | Multi-mass perturbation reads as chorus motion that ebbs and flows. | v2, evaluate |
| **Kepler elliptical** parameterise by `(a, e, ι, ν)` per satellite | 1 sqrt + 2 sin + 2 cos | Beautiful in theory, but the period is set by the orbit and decouples from STFT hop rate, so the audible result is hard to control and will collide with the FFT bin grid. | **No** |

**Recommended formula (linear, v1):**
```
Δφ_satellite = strength × magnitude_master / (distance² + ε)
              × sign(K_satellite − K_master)        // direction
              × phase_master_velocity_sign           // co-rotation cue
```
where `phase_master_velocity_sign = sign(unwrapped_phase_master[hop] -
unwrapped_phase_master[hop-1])`. The `× sign(distance)` term is what makes
two satellites at `K-3` and `K+3` orbit in opposite directions, producing
beat patterns when summed. The `× phase_velocity_sign` term is what makes
the orbit *follow* the master rather than drift independently — this is
what stops the result sounding like noise.

For multiple master bins, sum the contributions from each master with a
1/d² distance weight; satellites in shared "equigravisphere" zones get
nontrivial perturbations that read as chorus depth modulation without
actually being noise.

**Why not Kepler.** The Kepler orbit's period is set by the semi-major
axis, not the STFT hop rate. To produce 5–10 Hz chorus motion you'd be
solving Kepler's equation iteratively per hop per satellite (Newton-Raphson
needs ~3 iterations to converge for low eccentricity). Cost is ~30
mul-add per satellite vs 1 for linear rotation. The audible benefit is
that the orbit traces a measurable arc in phase-space — which the listener
cannot perceive directly because we hear the result through the iFFT,
which folds the phase trajectory back to a single time-domain signal.
Effectively, a Kepler integrator buys you *no audible upside* over linear
phase rotation. Kim's instinct ("cheap, sounds like phase-modulated
chorus") is correct.

**Why symplectic Euler is interesting for v2.** A 2-state (φ, ω) per-bin
integrator costs roughly 5x the linear version but introduces *memory*:
the satellite has angular momentum, so when a master bin's amplitude
suddenly drops, the satellite continues orbiting and slowly damps. That
"ringing-down chorus tail" is musically interesting and is something the
linear version cannot produce. Defer until v2 unless it lands cheaply.

### Implementation candidates

No public code exists for "spectral mass-attractor orbital chorus." The
nearest reference implementations to crib from:

- [arxiv 2510.12377 — Phase Synthesizer for Decorrelation](https://arxiv.org/html/2510.12377v1)
  — per-bin phase rotation as decorrelation, includes recommended modulation
  rates that don't break harmonic content.
- [Bitwig Phase-4](https://www.bitwig.com/phase-4/) — commercial reference
  for what a phase-rotation-based modulator should sound like. Not source.
- [PARSHL](https://ccrma.stanford.edu/~jos/parshl/parshl.pdf) — partial
  tracking provides the "which bin is a master and which is a satellite"
  classifier. Can be reused from the Harmony module's IF/peak-tracking
  infrastructure.

### Recommendations

1. **Ship Orbital as linear phase rotation in v1.** The formula above is
   ~3 ops per (master, satellite) pair, well within budget for the
   Kinetics module's CPU class.
2. **Tie satellite list to existing peak detection** (Harmony's pitch
   tracker output, or the Modulate module's FM Network partial detector).
   Avoid running an Orbital-specific peak detector — it's redundant.
3. **Cap the satellite count per master** at ~16 bins on each side. Beyond
   that, the 1/d² falloff makes the contribution inaudible.
4. **Defer the symplectic-Euler "memory" version to v2** — it is worth
   trying once Orbital is shipped and people have reactions to the
   linear version.
5. **Skip Kepler entirely.** It is conceptually beautiful but the audible
   difference vs. linear phase rotation is undetectable through the iFFT.
6. **Per-channel:** in Independent stereo, run two satellite tables. In
   Linked, share. In M/S, the same machinery operates on M and S
   independently — no special handling needed.

---

## Topic B — Real-time persistent homology of STFT

### Key references

This is the most active research area of the four topics, but the published
work is still primarily *batch*-oriented and not at audio rates.

**Theory:**
- Edelsbrunner & Harer's standard text on persistent homology computation —
  see the [roadmap paper](https://link.springer.com/article/10.1140/epjds/s13688-017-0109-5)
  for cubical complexes specifically.
- [Wasik & Reininghaus discrete Morse theory](https://link.springer.com/article/10.1007/s00454-013-9529-6)
  — current best practice for cubical PH on regular grids. O(n²) worst
  case but sub-linear in practice for typical natural images.
- [Lower-star filtration](https://en.wikipedia.org/wiki/Persistent_homology)
  — the right filtration for grayscale image-like data. Each pixel value
  determines the cell's birth height.

**Software libraries (benchmarked):**
- **Cubical Ripser** ([GitHub](https://github.com/shizuo-kaji/CubicalRipser),
  [arXiv 2005.12692](https://ar5iv.labs.arxiv.org/html/2005.12692)) — the
  fastest library for 2D cubical PH. Lena 512×512 = 0.18 s, 1024×1024 =
  0.81 s, 2048×2048 = 3.3 s, single thread, on a 1.6 GHz Core i5-4200U
  laptop.
- **GUDHI** ([gudhi.inria.fr](https://gudhi.inria.fr/cubicalcomplex/)) —
  most mature library, slower than Cubical Ripser by ~2-3x but more
  features (persistence diagrams, kernels, sklearn interface).
- **DIPHA** ([GitHub](https://github.com/DIPHA/dipha)) — distributed
  version, useful only for very large data.
- **Diamorse** ([GitHub](https://github.com/AppliedMathematicsANU/diamorse))
  — discrete Morse theory implementation, designed for material science but
  is the right algorithmic ancestor for streaming.
- **Sthu's persistent topology peak detection blog**
  ([sthu.org](https://www.sthu.org/blog/13-perstopology-peakdetection/index.html))
  — the cleanest pedagogical implementation of 1D PH peak detection;
  O(n log n) algorithm written in Python in ~50 lines.

**Audio applications (still rare):**
- [arXiv 2309.03516 — Topological Fingerprints for Audio Identification](https://arxiv.org/abs/2309.03516)
  — uses cubical complex PH on mel-spectrograms for audio fingerprinting.
  Batch-only. SIAM J. of Mathematics of Data Science 2024.
- [arXiv 2310.06508 — TDA of Human Vowels](https://arxiv.org/abs/2310.06508)
  — vowel classification using PH. Batch.
- [arXiv 1608.07373 — Topological Persistence in CNNs for Music Audio](https://arxiv.org/abs/1608.07373)
  — embeds persistence landscapes as features into a CNN. Batch.
- [arXiv 2506.13595 — Persistent Homology of Music Network](https://arxiv.org/abs/2506.13595)
  — graph PH on score-level music data. Batch.
- [DAFx 2024 paper 29 — Topology-Preserving Deformations of Digital Audio](https://www.dafx.de/paper-archive/2024/papers/DAFx24_paper_29.pdf)
  — Georg Essl, the rare DAFx contribution. Concerns time-domain audio
  topology preservation under deformation, not PH on STFTs.

**Streaming PH (theoretical):**
- [arXiv 1307.6188 — Sliding Windows and Persistence: Foundations of
  Computational Mathematics 2014](https://arxiv.org/abs/1307.6188)
  — Perea & Harer, foundational. Maximum persistence at point-cloud level
  quantifies periodicity.
- [NSF 10466285 — Computation of Persistent Homology on Streaming Data](https://par.nsf.gov/servlets/purl/10466285)
  — sliding-window model, O(m²) update per insertion/deletion where m =
  window size. Implemented in C++ as the "Lightweight Homology Framework"
  (LHF).

### Streaming vs batch tradeoff

Streaming PH algorithms maintain the persistence diagram incrementally as
new frames arrive and old frames slide out. Theoretical complexity for
sliding-window updates is O(m²) per frame insertion in m points, where m
is the window size in *complex cells*, not data points. For our case
(64 frames × 8193 bins = ~525k cells in the cubical complex), this is
*not* faster than batch recomputation — the constant factors in the
streaming algorithm are higher than in optimised batch cubical PH (Cubical
Ripser).

Practical recommendation: **don't use streaming PH for our use case.**
Instead, batch-recompute every K hops on a window of the most recent N
frames, and reuse the result for K hops. This is what
arXiv:2309.03516 does for fingerprinting.

### Library choice

For a Rust plugin, the realistic options are:

| Option | Approach | Notes |
|---|---|---|
| **Wrap CubicalRipser via FFI** | Best perf, depends on C++ | Not allocation-free; not realtime-safe inside the audio thread. Run on a worker thread. |
| **Port the lower-star algorithm** to Rust ourselves | Modest perf, fully realtime-safe | The 2D lower-star filtration on a regular grid is implementable in ~300 LoC of Rust. The reference implementation is the [efficient cubical algorithm by Wagner, Chen, Vuçini 2011](https://chaochen.github.io/publications/chen_topoinvis_2011.pdf). |
| **Use the [persistent-homology Rust crate `phat-rust`](https://github.com/blazewicz/phat-rust)** | Slow, simple | A port of Bauer/Kerber/Reininghaus PHAT. Functional but no benchmark advantage over GUDHI. |
| **1D PH per frame (sthu-style)** | Trivial port | Only gets you per-frame peak persistence. Loses the time-axis topology. |

### CPU feasibility for 8193 × 64 at hop rate

Doing the math against the Cubical Ripser benchmark:

- Cubical Ripser on Lena 512×512 = 0.18 s on a 2014 laptop core.
- Our grid is 8193 × 64 = 524,352 cells. Lena 512×512 = 262,144 cells. So
  our grid is roughly **2× larger**.
- Modern desktop CPU is ~3-5× faster per core than that 2014 i5-4200U.
- Net estimate: **~80-150 ms per analysis** on a single modern core.

That's **two orders of magnitude too slow for per-hop**. At hop=512,
sample_rate=44100, the hop period is 11.6 ms.

Even reducing the analysis cadence to every 16 hops (≈ 5 Hz refresh)
costs us 100 ms of CPU per 184 ms of audio = **54% single-core load**
just for the PH analysis. That's untenable for a slot inside a 9-slot
plugin.

To make this work we'd need:
1. **Drastically shorter time window:** 64 frames → 16 frames cuts analysis
   data 4×. Combined with downsampling the bin axis (8193 → 1024 by
   averaging or peak-picking before PH), we get a 256-cell grid that
   Cubical Ripser handles in ~5-10 ms.
2. **Asynchronous worker thread:** the audio thread reads the cached
   persistence diagram; a worker thread updates it every K hops on a
   downsampled grid. Audio thread pays only the cost of *applying* the
   gate, not computing it.
3. **Batched, not streaming:** as above, streaming PH offers no advantage
   here.

Even with these tricks, this is the heaviest CPU consumer in the entire
plugin design.

### Recommendations

1. **Defer to v2 minimum.** Persistent Homology mode in Geometry is the
   right idea but infeasible at native FFT resolution within an STFT hop.
2. **Prototype on a downsampled grid:** 1024 bins × 16 frames is
   tractable and could be the v1 form. Use the Hilbert/Morton mapping
   already discussed in `18-geometry.md` § (e) as the downsampler.
3. **Mark `always_bypassed_on_low_end = true`** in the ModuleSpec when
   this finally ships.
4. **Use a worker thread** with a lock-free triple-buffer for the
   persistence diagram — the same protocol the GUI uses for spectrum
   data. Audio thread reads the cached diagram each hop; never blocks.
5. **For 1D-only peak persistence** (per-frame peak gating, no
   time-axis), use the sthu O(n log n) algorithm — this *is* tractable
   per hop at 8193 bins (~200 µs in Rust). Worth shipping standalone
   even before the 2D version is ready. Could live in Harmony as a
   replacement for the IF/MQ peak picker, since persistent peaks are
   *more stable* across hops than threshold-based peaks.
6. **Output mapping:** Gaussian gate around each persistent maximum
   (sigma proportional to the maximum's persistence value). The
   rectangular-gate option creates audible bin boundaries; avoid.

In summary: **the deferred status is correct.** The 1D simplification is
worth investigating earlier than the 2D version.

---

## Topic C — Spectral hole-and-fill perceptual quality

### Key references

This topic has the strongest perception literature of the four — auditory
masking and spectral completion are well-studied phenomena.

**Auditory masking and gap-filling:**
- [PNAS — McDermott & Oxenham 2008 — Spectral completion of partially
  masked sounds](https://pmc.ncbi.nlm.nih.gov/articles/PMC2311350/) — the
  load-bearing paper. The auditory system fills inferred energy at
  approximately **10 dB below the masker level**, using continuous
  spectral neighbourhood and onset-pattern cues.
- [PNAS — McDermott Lab spectral completion demos](http://mcdermottlab.mit.edu/spec_comp_demos/spec_comp_page1.html)
  — listening examples that demonstrate the phenomenon.
- [Bregman 1990 — Auditory Scene Analysis: Continuity Illusion](https://books.google.com/books/about/Auditory_Scene_Analysis.html?id=jI8muSpAC5AC)
  — original reference. Gap-fill works when (a) the masker overlaps the
  target frequency band, (b) the gap is shorter than ~300 ms, and (c)
  the masker is sufficiently loud. Above all, the spectral notch must be
  smaller than the *critical bandwidth*.
- [Frontiers — Dynamics of the Auditory Continuity Illusion 2021](https://pmc.ncbi.nlm.nih.gov/articles/PMC8217826/)
  — neural model showing the temporal extent (~half-second scale).
- [Warren — phonemic restoration](https://en.wikipedia.org/wiki/Phonemic_restoration_effect)
  — the speech version, breakdown around word-length gaps.
- [Effects of simulated spectral holes on speech intelligibility](https://pmc.ncbi.nlm.nih.gov/articles/PMC2830263/)
  — speech intelligibility degrades when spectral holes exceed ~1 ERB
  width and cannot be filled.

**DSP / pitch-shifting:**
- [Laroche & Dolson DAFx 1999 — New phase-vocoder techniques](https://www.ee.columbia.edu/~dpwe/papers/LaroD99-pvoc.pdf)
  — phase-locked vocoder; sub-bin pitch shift is just phase rotation
  `2π × Δf × hop / sample_rate`. Quality artifact-free up to ±1 bin.
- [DAFx 2007 — Real-time pitch-shifting of musical signals](https://www.dafx.de/paper-archive/2007/Papers/p007.pdf)
  — pitch-shifting artifact-noise patterns.
- [DAFx 2003 — Spectral hole filling](https://dafx.de/paper-archive/2003/pdfs/dafx72.pdf)
  — formant-preserving frequency-domain reorganization.

### Pitch fill artefacts

The Punch sub-effect proposes that neighbouring bins drift toward the carved
hole via small per-hop phase rotation `Δφ = 2π × Δf × hop / sample_rate`.

**For harmonic content (single tones, chords):**
- Sub-bin (≤0.5 bin) drift is essentially inaudible as pitch change. The
  perceived pitch of a partial is dominated by IF, and a 0.5-bin shift is
  ≈ 1/2 × `sample_rate / fft_size` ≈ 5 Hz at 44.1 kHz / 2048. Below the
  detection threshold of 6 Hz vibrato most listeners report.
- The *modulation* of the drift each time a peak appears creates a tiny
  FM sideband. With careful smoothing (1-pole follower, τ ~ 50 ms), the
  sideband stays inside the masking-bandwidth of the master peak that
  triggered it — i.e. **inaudible by virtue of the same masker** that
  caused the carve in the first place. This is a happy coincidence.

**For noise/transient content:**
- The drift has no perceptual consequence because random-phase noise is
  invariant under small phase rotation.

**For dense polyphonic content:**
- Multiple drifts in close proximity sum into a small chorus motion. This
  is musically benign — it sounds like a soft modulation rather than a
  glitch.

**For sustained pure tones (worst case):**
- A pure 440 Hz tone with a sidechain peak at 432 Hz causing the 440 Hz
  bin to drift: the perceived pitch wobbles at the rate of the sidechain
  envelope. If the sidechain has fast transients (drums), the result is
  a pitch flutter that *is* audible and can be unmusical.
- Mitigation: clamp drift rate-of-change to a maximum of ~2-3 cents/hop
  (i.e. apply slew-rate limiting on `Δφ` updates). At hop=512, that's
  smooth enough to never trigger a vibrato.

**Conclusion:** pitch fill is musical for most content but needs slew
limiting on the drift parameter to be safe on pure tones.

### Healing curve shapes

The "healing" is the recovery of a carved bin's magnitude after the
sidechain peak releases. Three options:

| Shape | Formula | Sounds like | Recommended |
|---|---|---|---|
| **Linear** | `gain += release_rate × dt` clamped to [0, 1] | Mechanical, predictable, almost robotic | No |
| **Exponential** | `gain = 1 - (1 - gain) × decay_coeff` | Natural breath / spectral resilience. Default for perception literature. | **Yes — default** |
| **Sigmoid** | `gain = sigmoid(t × steepness)` | "S-curve" recovery — long tail at start, sharp middle, long tail at end. Reads as deliberate envelope. | Optional |

The auditory literature (McDermott & Oxenham, Bregman) does not specify
a single "natural" recovery shape, but the general consensus is that
**exponential matches our perceptual model of resilience** — quiet things
recover fast, loud things take time. Exponential is also the lowest-cost
to implement (one mul-add per bin per hop).

**Time constants** (from Bregman/PNAS):
- Auditory inference operates on a ~half-second scale; gaps shorter than
  this are filled, longer ones are heard as discontinuity.
- For a *plugin* effect we want the user to *hear* the carve, so healing
  should be perceptually visible:
  - Default exponential τ = **150 ms** (fast healing, snappy character).
  - User-controllable via `HEAL` curve from 20 ms (very snappy) to 2000 ms
    (long pad-like recovery).
- A 1-pole follower (`y[n] = α × y[n-1] + (1-α) × x[n]` with `α =
  exp(-hop_period / τ)`) is the natural primitive.

### Watch-range curve concept

The proposal: instead of detecting sidechain peaks across the entire
sidechain spectrum, draw a per-input-bin curve specifying *which slice of
the sidechain* each input bin should listen to.

**Pros:**
- Lets a user say "kick drum (50-100 Hz sidechain) carves the kick range
  (50-200 Hz input)" — a self-contained kick-ducking preset.
- Lets a user say "kick drum (50-100 Hz sidechain) carves the
  *upper-mid* range (1-3 kHz)" — a creative inversion that no current
  effect supports cleanly.
- Composes well with sidechain peak detection: each bin runs a per-bin
  peak detection on the watch-range slice.

**Cons:**
- Adds a curve, pushing Punch from 6 to 7 curves (at the limit).
- Computationally adds ~O(N × W) where W is average watch-range width.
- The semantics are non-obvious — users will probably not configure it
  by hand; it'll only be effective via presets.

**Recommendation:** ship Punch with a *single global* sidechain peak
detector for v1. Add the watch-range curve **as a v2 expansion** if user
demand is there. The 7-curve cap is unforgiving and the watch-range
curve is the most "advanced user" of the proposed curves — defer it.

If shipped, treat the watch-range curve as a **frequency mapping**: each
input bin picks one sidechain bin to listen to, drawn from the curve.
This costs O(N) lookups, not O(N × W).

### Recommendations

1. **Default fill mode = amplitude** with **exponential healing τ = 150
   ms**. This matches perceptual literature and is the cheapest to
   implement.
2. **Pitch fill is a useful flavour mode** but ship with mandatory slew
   limiting (~2 cents/hop max) to prevent pitch flutter on sustained
   tones. Make it user-selectable per slot.
3. **Healing curve shapes:** default exponential. Ship linear and sigmoid
   as named presets in the FILL_MODE curve dropdown so users can
   experiment without learning curve theory.
4. **Sidechain bandwidth:** global per-slot detection in v1, per-bin
   watch-range curve in v2. Don't push to 7 curves on day one.
5. **Healing time range:** 20 ms to 2000 ms; default 150 ms. Map curve
   value 1.0 = 150 ms.
6. **Smoothing on amplitude fill:** 1-pole follower with τ = 5 ms (fast
   enough to track sidechain transients, slow enough to avoid clicks at
   block boundaries).
7. **`depth × width` operating range:** the musically useful zone is
   depth 0.3-0.8 × width 1-8 bins. Beyond width 16 the carve is so wide
   it just sounds like a duck. Hardcode this as the visual range of the
   curve UI.

---

## Topic D — Buchla-style envelope for RM gating

### Key references

This is the most well-documented topic of the four — Buchla circuits are
extensively studied.

**Buchla 281 reference:**
- [Buchla 281 Modular Synthesis page](https://modularsynthesis.com/roman/buchla281/281_qfg.htm)
  — circuit-level description with scope captures.
- [Buchla 281 Clone Builder's Guide (Toppobrillo)](http://www.sdiy.org/toppobrillo/twoeightyone.html)
  — schematics. Site has TLS issues but content is mirrored on
  electro-music wiki.
- [JonDent — Exploring the 281](https://djjondent.blogspot.com/2021/09/buchla-281-quad-function-generator-not.html)
  — operational description of cycle/transient/sustained modes and
  AND/OR fuzzy-logic outputs.
- [Buchla 281e / 281t product pages](https://buchla.com/product/281e/) —
  current production specs, attack/decay 0.001-10 s.
- [Discussion: exponential curves with Buchla 281](https://modwiggler.com/forum/viewtopic.php?t=157829)
  — confirms positive CV → log curve, negative CV → exp curve.

**Buchla 292 (lopass gate) DSP modeling:**
- [DAFx 2013 — A Digital Model of the Buchla Lowpass-Gate](https://dafx.de/paper-archive/2013/papers/44.dafx2013_submission_56.pdf)
  — Parker, Bilbao, Smith — the canonical reference. Two discretizations:
  transfer-function (cheap) and topology-preserving (better under
  time-varying control). The vactrol's asymmetric response (fast attack,
  slow release) is modelled as a time-varying RC.
- [SuperCollider LPG class](https://doc.sccode.org/Classes/LPG.html) —
  reference implementation in SC.

**Trigger detection / SIMD:**
- [Rust std::simd](https://doc.rust-lang.org/std/simd/index.html) —
  portable SIMD with AVX2 dispatch.
- [JUCE forum — branchless SIMD envelope](https://forum.juce.com/t/solved-branching-with-simd/45473)
  — practitioner experience with masked updates.
- [SimdFSM](https://link.springer.com/chapter/10.1007/978-3-031-29927-8_37)
  — vectorised finite-state machines.

### Envelope shape comparison

| Shape | Operations / bin / hop | Buchla 281 fidelity | Notes |
|---|---|---|---|
| **AR (linear)** | 1 add, 1 cmp | Low — too mechanical | The 281 itself is *linear* slope but the *rate* is exp-controlled. |
| **AR (exponential)** | 1 mul, 1 add | Medium — too soft | Sounds like a typical envelope follower, lacks the "snap" of the 281's instant attack. |
| **AR (linear slope, exp-controlled rate)** | 1 mul, 1 add | **High — the 281 itself** | Linear ramp shape; rate set by exp-mapped CV. This is the actual 281 architecture per modwiggler thread. |
| **ADSR** | 1 mul, 1 add (state-machine 4 states) | Wrong — the 281 doesn't have sustain segment | The 281's "sustained" mode is a hold, not a level. ADSR adds parameter cost without character benefit. |
| **Lopass-shaped exponential (vactrol)** | 1 mul, 1 add (time-varying coef) | **Highest — Buchla 292 territory** | The asymmetric vactrol response (fast attack ~1 ms, slow exp release 30-300 ms) is the audible signature. |

**Recommendation: dual-mode envelope.**
- **Mode A: AR with exp-mapped rate** (Buchla 281 character). Linear ramp,
  but the rate is `rate_target = base_rate × exp(cv × scale)`, so the
  envelope responds exponentially to control voltage. Cost: 2 mul + 1
  add per bin.
- **Mode B: Vactrol AR** (Buchla 292 character). One-pole asymmetric
  follower with `α_attack = exp(-hop / τ_attack)` and `α_release =
  exp(-hop / τ_release)`. τ_attack ≈ 0.5-2 ms, τ_release ≈ 20-300 ms.
  Smooth, not snappy. Cost: 1 mul + 1 add + 1 select per bin.

Either mode has the "hit it and let it ring" character that distinguishes
Buchla envelopes from gates.

### Per-bin vs band-energy trigger

The brainstorm note (#19) asks whether to detect peaks per-bin or per-band.

**Per-bin trigger:**
- Each bin checks its own sidechain magnitude against threshold.
- 8193 thresholding ops per hop. Trivial.
- Triggers at exactly the bin where the sidechain is loud; no cross-talk.
- *Risk:* bin-level threshold makes triggers sensitive to small jitter
  in sidechain magnitudes between hops. Many envelopes retrigger
  spuriously, producing buzzing.
- Mitigation: hysteresis (separate on/off thresholds, e.g. on at -20
  dBFS, off at -30 dBFS).

**Band-energy trigger:**
- Group bins into M bands (e.g. 8 octave bands). Each band has its own
  threshold and energy detector.
- One trigger fires the entire band's worth of envelopes simultaneously.
- This is closer to how the original Buchla 281 works — discrete
  sub-bands, not per-bin.
- **Audible signature:** a single sidechain peak triggers a synchronised
  band of envelopes, which sounds like a *bell ringing* rather than a
  shimmer. This is the "Buchla bongo" character.
- 8 band detectors per hop; trivial.

**Recommendation: ship band-energy trigger as default**, expose per-bin as
an alternative mode. The band-energy mode is what gives the Buchla
character; per-bin is a more analytical mode for fine-grained gating.

For band assignment, use the **ERB scale** or **Bark scale** rather than
linear bins — perception of "a band of frequencies" follows critical bands.

### SIMD strategy for 65k envelopes

The math: 8193 bins × 4 channels × 2 stereo = 65,544 envelopes. Each
envelope at minimum: 1 mul + 1 add + 1 cmp + 1 select per hop. So:
- 4 ops × 65k = ~262k ops per hop
- At hop=512, sample_rate=44100, that's ~22M ops/sec
- A modern AVX2 core does ~30 GFLOPS. We'd be using <0.1% of one core.

Even **without SIMD**, this is trivially cheap. With SIMD it's free.

**The 65k figure is misleading though.** The use case is one Modulate slot
with one mode = RM/FM Matrix. So we have:
- 1 module × 8193 bins × N channels
- N=2 in stereo, N=2 again if Independent stereo, but only one slot at a
  time uses this mode.
- Realistic: ~16-32k envelopes max per active slot.

**SIMD layout (Rust std::simd):**
```rust
pub struct EnvelopeBank {
    state:        Vec<f32>,    // current envelope value per bin
    target:       Vec<f32>,    // target value (set by trigger)
    alpha_attack: f32,         // attack coef (or per-bin if curve-driven)
    alpha_release: f32,
}

impl EnvelopeBank {
    fn process_hop(&mut self, sidechain_mag: &[f32], threshold: f32) {
        // Branchless: compute alpha from per-bin condition
        for (state_chunk, sc_chunk) in self.state.chunks_mut(SIMD_W)
            .zip(sidechain_mag.chunks(SIMD_W)) {
            let s = f32x16::from_slice(state_chunk);
            let sc = f32x16::from_slice(sc_chunk);
            let above = sc.simd_gt(f32x16::splat(threshold));
            let target = above.select(f32x16::splat(1.0), f32x16::splat(0.0));
            let alpha = above.select(
                f32x16::splat(self.alpha_attack),
                f32x16::splat(self.alpha_release),
            );
            let new_state = s * alpha + target * (f32x16::splat(1.0) - alpha);
            new_state.copy_to_slice(state_chunk);
        }
    }
}
```

This is branchless, vectorises cleanly, and requires no per-envelope state
machine — the asymmetric response is implicit in `alpha_attack` vs
`alpha_release`.

For the AR-with-exp-rate (Buchla 281) variant, replace the multiply-and-add
with a linear ramp toward target, with rate `±slope` selected by mask. Same
shape, slightly different ops.

**Re-trigger smoothing:** if a new peak arrives before the envelope
decays, the natural behaviour of the asymmetric follower is to **sum**
the trigger pulses through the attack stage — it just keeps tracking
upward. No special re-trigger logic needed. This is closer to the
Buchla's behaviour than abort+retrigger.

### Recommendations

1. **Default envelope = vactrol AR** (Buchla 292 character).
   `τ_attack = 1 ms`, `τ_release = 80 ms`. Curves can override per-bin.
2. **Optional envelope = AR with exp-mapped rate** (Buchla 281 character).
   Same code path with different update rule.
3. **Trigger = band-energy on ERB-spaced bands** (8 bands default, user
   curve sets band count up to 32). Per-bin is an alternative mode.
4. **Hysteresis:** 6 dB difference between on and off thresholds.
5. **Re-trigger:** no special logic — let the asymmetric follower handle it.
6. **SIMD:** Rust std::simd, 16-wide where AVX-512 is available, 8-wide
   AVX2, 4-wide NEON. Branchless via mask-select for trigger update.
7. **CPU budget:** <0.1% of one core for the entire RM/FM Matrix slot
   in stereo. This is not a bottleneck.

---

## Cross-topic synthesis

### Shared infrastructure

Three of the four topics (A, C, D) need **per-bin envelope/follower
infrastructure**. The same `EnvelopeBank` SIMD primitive serves:
- **Topic A (Orbital):** smoothing the per-satellite phase rotation rate
  to prevent click on master-peak appearance/disappearance.
- **Topic C (Punch):** healing-curve recovery (1-pole follower per
  carved-bin).
- **Topic D (Buchla envelope):** the headline use case.

This is not a coincidence — they all share "per-bin temporal smoothing
indexed by sidechain or peak events." A single shared `EnvelopeBank`
type in the codebase would absorb all three. Recommend implementing it
once, reusing across modules.

Topic B (persistent homology) is the outlier — it has no shared
infrastructure with the other three topics.

### Phase mathematics shared

Topics A and C both manipulate per-bin phase by tiny rotations. The math
(`Δφ = 2π × Δf × hop / sample_rate`) is identical. A shared
`apply_phase_rotation_to_bins()` helper in `src/dsp/utils.rs` is worth
adding.

### Triggering shared

Topic D's band-energy trigger detector is the same operation as Punch
(Topic C) needs for its sidechain peak detection. Could be exposed as a
`PeakDetector` utility that returns a `Vec<BinIndex>` of currently-active
trigger sites. Reused by Punch, Buchla envelopes, possibly Tuning Fork
(Kinetics).

### CPU class sanity check

| Topic | Module | Expected hop CPU |
|---|---|---|
| A — Orbital | Kinetics sub-effect | Light. ~3 ops × 16 satellites × 16 masters = 768 ops/hop. |
| B — Persistent Homology | Geometry sub-effect | **Heavy.** ~80-150 ms per analysis even on downsampled grid. Worker thread. |
| C — Punch hole-and-fill | Punch module | Light. O(N) per hop, ~50k ops. |
| D — Buchla envelopes | Modulate sub-mode | Light. SIMD-friendly, <0.1% core. |

Three of the four are trivial; one (Topic B) is uniquely expensive and
needs special handling.

---

## Open questions

1. **Topic A — Master peak detection ownership.** Should Orbital share
   peak detection with Tuning Fork (also in Kinetics), or with Harmony's
   IF-based partial tracker, or run its own? Consolidating saves CPU but
   couples modules. Recommend: share with Kinetics's Tuning Fork module
   only (both need "loud bin master with skirt").

2. **Topic A — Phase wrap.** The per-hop phase rotation `Δφ` is added to
   wrapped phase. Need to ensure the sum doesn't accumulate quantisation
   error over many hops — easy to handle (`phi = (phi + dphi) mod 2π`)
   but worth noting.

3. **Topic B — Worker thread for PH.** Does the Pipeline support
   worker-thread DSP today? If not, this is a global infrastructure
   piece needed before PH mode can ship even on a downsampled grid.

4. **Topic B — Output gate semantics.** When the persistence diagram says
   "this bin participates in the M most-persistent features," do we
   *boost* it (gate-on) or *attenuate the rest* (gate-off)? Default is
   gate-off (attenuate non-persistent bins, keep persistent ones), but
   a "highlight" mode (boost persistent, leave others alone) is also
   musically interesting.

5. **Topic C — Fill mode user control.** A FILL_MODE curve giving
   per-bin choice between amp-fill (low values) and pitch-fill (high
   values) is more flexible than a per-slot enum, but harder to explain
   in the UI. Recommendation: ship enum first, expose per-bin curve as
   v2.

6. **Topic C — Mono vs stereo drift coherence.** In Independent stereo,
   should the pitch drift be locked across L/R (mono fill) or
   independent (stereo width)? Defer the question by exposing a
   `STEREO_LINK_FILL: bool` per slot; default to linked for safety.

7. **Topic D — Per-mode envelope exposure.** The vactrol vs 281-style
   envelope choice exposes user-facing tonal character. Worth a UI
   "envelope character" dropdown per slot, or just hardcode vactrol?
   Recommend: hardcode vactrol for v1, add user choice in v2.

8. **All topics — Cross-module interaction.** When two modules with
   per-bin envelopes (Punch healing, Buchla envelope) are in series,
   the two envelopes interact. Audible result is hard to predict. Worth
   a preset library demonstrating known-good combinations.

9. **Topic B — 1D PH peak detection as Harmony backend.** The sthu
   O(n log n) algorithm could replace Harmony's MQ peak picker entirely.
   Worth evaluating for *quality* (stability across hops, noise
   robustness) before committing.
