# Research: Real-Time Polyphonic Pitch Tracking + Cepstral Liftering

**Source prompts:** prompts 2 and 11 in `90-research-prompts.md`
**Status:** Findings as of 2026-04-26
**Audience:** developer deciding implementation strategy for the Harmony module
(IF infrastructure → chromagram → harmonic-group detection → cepstral liftering).

---

## Refined research questions

The original prompts mix several concerns that the literature treats separately.
This document splits them along these axes:

1. **Per-bin frequency refinement** — given an STFT bin at index `k` whose centre
   freq is `k·fs/N`, what is the *actual* frequency of the dominant sinusoid
   present? This is the IF problem and is well-solved.
2. **Single-pitch tracking** — find one fundamental per frame. CREPE/PESTO/
   SwiftF0/FCPE territory; entirely solved at very low cost when monophonic.
3. **Multi-pitch / polyphonic pitch estimation (MPE)** — find a *set* of
   simultaneous fundamentals per frame. Klapuri 2006/Duan 2010 → Bittner
   2017 (Deep Salience) → Cwitkowitz 2024 (self-supervised).
4. **Chromagram** — collapse multi-pitch to a 12-element pitch-class profile.
   Two cheap routes (Fujishima 1999, IF-refined HPCP) and one neural
   (Korzeniowski's DeepChroma).
5. **Chord recognition** — read the chromagram and emit "C maj." Templates,
   HMMs, CNN+CRF, bidirectional Transformer, and (2025) ChordFormer.
6. **Harmonic-group / partial tracking** — group the sinusoidal peaks of a
   single voice. McAulay-Quatieri/PARSHL/SMS, Klapuri's harmonic-summation
   front-end, "Phase Vocoder Done Right" sinusoidal phase coherence.
7. **Spectral envelope / cepstrum** — separate vowel/timbre from
   pitch/excitation. Imai-Abe 1979 "true envelope," Roebel 2005 efficient
   true envelope, WORLD vocoder's CheapTrick (2014, 2016).

---

## Topic A — Polyphonic pitch tracking with phase info

### Key references (papers)

Ordered by usefulness for the Spectral Forge architecture, *not* by citation
count. Spectral Forge already has a per-hop STFT and phase, so reference
works that exploit that pipeline are preferred over end-to-end neural systems
that reinvent feature extraction.

#### IF / phase-derivative line of work (foundational, still load-bearing)

- **Brown & Puckette 1993, "Phase distortion analysis of the IF estimator"**
  — establishes that the per-bin phase derivative gives sub-bin-accurate
  frequency estimates. This is the single result Kim's IF infrastructure plan
  rests on. Free ProQuest preprint:
  <https://www.researchgate.net/publication/253271076_A_high_resolution_fundamental_frequency_determination_based_on_phase_changes_of_the_Fourier_transform>.
- **McAulay & Quatieri 1986, "Speech analysis/synthesis based on a sinusoidal
  representation"** — IEEE TASSP. The original peak-tracking partial
  birth/death model. Direct ancestor of what becomes "harmonic group
  detection" in the spec. Modern free retelling at JOS:
  <https://ccrma.stanford.edu/~jos/parshl/parshl.pdf> (PARSHL technical
  report).
- **Auger & Flandrin 1995, "Improving the readability of time-frequency
  representations by reassignment"** — the *reassignment method*. Per-bin
  IF + group-delay, sub-bin accuracy, no extra FFT. Matlab reference,
  Python reference (`librosa.reassigned_spectrogram`):
  <https://librosa.org/doc/main/generated/librosa.reassigned_spectrogram.html>.
- **Laroche & Dolson 1999, "New phase-vocoder techniques for real-time
  pitch shifting, harmonising and other exotic effects,"** JAES.
  Peak-locked phase coherence; this is the math behind the PVX work.
  PDF: <https://www.ee.columbia.edu/~dpwe/papers/LaroD99-pvoc.pdf>.
- **Průša & Holighaus 2017/2022, "Phase Vocoder Done Right"** —
  arXiv 2202.07382. RTPGHI: a phase-coherent reconstruction that does
  not require explicit peak detection, integrates phase gradients
  instead. Reference C/Matlab code in LTFAT project at
  <https://github.com/ltfat/pvdoneright>.

#### Deep neural single-pitch trackers (2017–2025)

These are monophonic but the architectural lessons apply.

- **Kim et al. 2018, "CREPE: A Convolutional Representation for Pitch
  Estimation"** — ICASSP. Sets the modern accuracy bar at >90% RPA(10c).
  Cost: 22M params, dominant on accuracy but heavy. arXiv 1802.06182.
  Repo: <https://github.com/marl/crepe>.
- **Engel et al. 2020, "Self-Supervised Pitch Estimation (SPICE)"** —
  arXiv 1910.11664. Equivalent accuracy to CREPE on monophonic, trained
  unsupervised by exploiting pitch-shift equivariance. Important precedent
  for PESTO.
- **Riou et al. 2023/2025, "PESTO: Pitch Estimation with Self-supervised
  Transposition-equivariant Objective"** — ISMIR 2023, then extended into
  Transactions of ISMIR 2025 with a streaming variant.
  - arXiv 2309.02265 (original) and arXiv 2508.01488 (real-time/streaming).
  - 130k params (700–800× smaller than CREPE), ~0.7 ms ONNX latency, supports
    stream input via cached convolutions, ONNX export shipped.
  - Repo: <https://github.com/SonyCSLParis/pesto>.
- **Wei et al. 2023, "RMVPE: A Robust Model for Vocal Pitch Estimation in
  Polyphonic Music"** — arXiv 2306.15412. U-Net + GRU over mel
  spectrogram. Extracts the *vocal* pitch out of polyphony directly
  without source separation. Top of `lars76/pitch-benchmark` on vocal data.
  Repo: <https://github.com/Dream-High/RMVPE>.
- **Nieradzik 2025, "SwiftF0: Fast and Accurate Monophonic Pitch
  Detection"** — arXiv 2508.18440. 95.8k params, 42–90× faster than CREPE
  on CPU, robust to noise (91.8% HM at 10 dB SNR vs CREPE's much worse
  drop). Pitch ground-truth includes a new SpeechSynth synthetic dataset.
  Repo: <https://github.com/lars76/swift-f0>. **This is the one to copy
  for monophonic.**
- **CNChTu 2025, "FCPE: A Fast Context-based Pitch Estimation Model"** —
  arXiv 2509.15140. Lynx-Net (depth-wise separable convolutions), mel
  input, RTF 0.0062 on RTX 4090, monophonic, optimised for SVC pipelines.
  Repo: <https://github.com/CNChTu/FCPE>.

#### Polyphonic / multi-pitch line of work

- **Klapuri 2003/2006, "Multiple Fundamental Frequency Estimation by
  Summing Harmonic Amplitudes"** — ISMIR 2006, the still-cited reference.
  PDF: <https://www.ee.columbia.edu/~dpwe/papers/Klap03-multif0.pdf>.
  Three estimators: direct salience, iterative cancellation, joint.
- **Klapuri 2008, "Multipitch analysis of polyphonic music and speech
  signals using an auditory model"** — IEEE TASLP. Auditory front end +
  iterative cancellation. PDF:
  <http://hans.fugal.net/comps/papers/klapuri_2004.pdf>.
- **Duan, Han & Pardo 2010, "Multiple Fundamental Frequency Estimation by
  Modeling Spectral Peaks and Non-Peak Regions"** — IEEE TASLP. Joint
  estimation, not iterative; shows superior performance to Klapuri-style
  cancellation on dense polyphonic inputs. PDF:
  <https://labsites.rochester.edu/air/publications/DuanPardoZhang_MF0E_TASLP10.pdf>.
- **Bittner et al. 2017, "Deep Salience Representations for f0 Estimation
  in Polyphonic Music"** — ISMIR 2017. CNN over HCQT (Harmonic
  Constant-Q Transform with 5 harmonics + 1 sub-harmonic). Direct
  ancestor of BasicPitch. PDF:
  <https://brianmcfee.net/papers/ismir2017_salience.pdf>.
- **Bittner et al. 2022, "A Lightweight Instrument-Agnostic Model for
  Polyphonic Note Transcription and Multipitch Estimation"** — ICASSP. The
  BasicPitch paper. 16.8k params, polyphonic note transcription with pitch
  bend, ONNX/TFLite/CoreML available. Repo:
  <https://github.com/spotify/basic-pitch>. Crucially, the C++ port works
  with ONNXRuntime: <https://github.com/sevagh/basicpitch.cpp>.
- **Cwitkowitz et al. 2024, "Toward Fully Self-Supervised Multi-Pitch
  Estimation"** — arXiv 2402.15569. Convolutional autoencoder, salience
  output, trained only on synthetic single-note audio yet generalises to
  polyphonic music. Closes the data gap that has long plagued MPE.
- **Gardner et al. 2022, "MT3: Multi-Task Multitrack Music
  Transcription"** — ICLR 2022, arXiv 2111.03017. T5-based encoder/decoder
  emitting note-event tokens. Sets SOTA on per-instrument transcription
  but is a >>100M-parameter Transformer; not real-time.
- **Chang et al. 2024, "MR-MT3: Memory Retaining Multi-Track Music
  Transcription"** — arXiv 2403.10024. Adds memory to mitigate instrument
  leakage. Same scale as MT3.
- **Chang et al. 2024, "YourMT3+"** — arXiv 2407.04822. Hierarchical
  attention + MoE.

#### Chord recognition

- **Fujishima 1999, "Realtime Chord Recognition of Musical Sound"** —
  ICMC. The original PCP/template matching paper.
  <https://quod.lib.umich.edu/i/icmc/bbp2372.1999.446/>.
- **Lee 2006, "Automatic Chord Recognition from Audio Using Enhanced
  Pitch Class Profile"** — ICMC. EPCP via Harmonic Product Spectrum.
- **Korzeniowski & Widmer 2016, "Feature Learning for Chord Recognition:
  The Deep Chroma Extractor"** — ISMIR 2016, arXiv 1612.05065. CNN that
  outputs a chroma vector cleaner than HPCP. Ships in `madmom`.
  PDF: <https://archives.ismir.net/ismir2016/paper/000178.pdf>.
- **Korzeniowski 2018, "Improved Chord Recognition by Combining Duration
  and Harmonic Language Models"** — arXiv 1808.05335.
- **Park et al. 2019, "A Bi-Directional Transformer for Musical Chord
  Recognition (BTC)"** — arXiv 1907.02698. CQT input, no separate decoder.
- **Lanz et al. 2025, "ChordFormer: A Conformer-Based Architecture for
  Large-Vocabulary Audio Chord Recognition"** — arXiv 2502.11840.
  Conformer + linear-CRF decoder, 2 % frame-wise / 6 % class-wise
  accuracy gains on 1217-song benchmark.
- **2025, "Enhancing Automatic Chord Recognition through LLM
  Chain-of-Thought Reasoning"** — arXiv 2509.18700. Out of scope for our
  RT plugin (LLM-call latency too high) but worth noting.

### State of the art synthesis

For the constraints in `15-harmony.md` (per-hop, 8193 bins, hop 128–1024,
<2 ms/hop on desktop, *uses existing STFT*), the modern best practice is a
*hybrid* IF + harmonic-summation pipeline, not a neural single-shot
detector. Five reasons:

1. We *already pay for the FFT*. Anything that would re-extract a
   spectrogram (CREPE/RMVPE/FCPE/SwiftF0) duplicates work.
2. The neural detectors all want a fixed input rate (mostly 22050 Hz)
   and a fixed window (1024 samples for CREPE; ~64 ms mel for SwiftF0).
   Bolting them onto an arbitrary-FFT-size pipeline costs a resampling
   stage, breaks per-hop budgets, and does not work below the input
   window.
3. The neural detectors are nearly all *monophonic*. The polyphonic ones
   (BasicPitch, MT3, RMVPE) are either heavy-Transformer or designed
   only for vocal isolation in a polyphonic mix. Neither matches our
   problem of identifying the chord *and* the harmonic group of every
   prominent partial.
4. IF + harmonic summation is *literally* what Klapuri's salience
   function does. Implementing it on top of the existing STFT phase data
   is ~50 lines of Rust and runs at the FFT cost we already pay.
5. We already have phase data and can run the per-bin phase derivative
   at zero cost — neural detectors throw that information away.

The recommended pipeline:

1. **Per-bin IF** (Brown-Puckette / reassignment method): subtract the
   expected phase advance `2π · k · hop / N` from the actual phase delta,
   wrap to (−π, π], divide by hop, add the bin centre frequency. Cost:
   one phase delta + one wrap per bin. Cheap, SIMD-friendly. This is
   what the existing `01-global-infrastructure.md` §3 plan already
   specifies.

2. **Stable-peak detection**: find local maxima in the magnitude
   spectrum, refine to sub-bin via parabolic interpolation in log-mag,
   keep only those whose IF agrees within ±¼ bin of the parabolic
   estimate (the IF-vs-frequency consistency test from Auger-Flandrin
   reassignment). This filters out non-sinusoidal noise floor bins.

3. **Klapuri-style harmonic summation**: for each candidate fundamental
   `f₀` in a log-spaced grid (every 1/12 octave, 20 Hz – 2 kHz, ≈ 96
   candidates is enough), compute salience
   `S(f₀) = Σₕ g(h) · M(h·f₀)` where `g(h)` is a harmonic weighting
   (Klapuri uses `(h + α)/(h·β + α)` with α≈52 Hz, β≈320 — the spectral
   smoothness prior) and `M` is the magnitude evaluated at sub-bin
   resolution by interpolation between IF-refined peaks. This is one
   pass; not iterative cancellation. For up to 96 candidates × 20
   harmonics × one interpolated lookup = ~2k ops, which is sub-millisecond.

4. **Iterative top-K extraction** (optional): pick the highest-salience
   `f₀`, subtract its harmonics from the spectrum (Klapuri's
   spectral-smoothness-shaped subtraction), repeat 2-4 times. This gives
   the harmonic-groups list directly.

5. **Chromagram by IF-refined HPCP**: for each bin above a threshold,
   take its IF, convert to MIDI cents, take pitch class mod 12, and
   accumulate magnitude into a 12-element vector. Smoothing across hops
   with a one-pole IIR (τ = 50 ms) gives the chromagram. This is
   "Enhanced HPCP" in literature (Gomez 2006).

6. **Chord identification**: cosine match the chromagram against 24
   templates (12 maj, 12 min, optional dim/aug/7). Pure linear algebra,
   <100 ops. Optional: a single-layer GRU on the 12-element chromagram
   smooths chord boundaries — but a 1-st order Markov chain over the
   24-state lattice (chord-bigram transition log-probs hand-set or
   trained on McGill Billboard) gives the same audible benefit at <1 µs.

This is what `harmony effects.txt` already proposes, validated against
the 2024-2026 literature: nothing newer beats it for our constraints.
The neural detectors *would* be the answer if we did not already have
STFT phase, but we do.

### CREPE / BasicPitch / SPICE / others — comparison table

| Method | Year | Type | Latency CPU | Params | Polyphonic | Phase-aware | Open source | License |
|---|---|---|---|---|---|---|---|---|
| YIN | 2002 | classical (CMNDF) | ~0.2 ms / 1024 | n/a | no | no | yes | various |
| pYIN | 2014 | classical+HMM | ~1 ms / 1024 | n/a | no | no | yes (`c4dm/pyin`) | GPL |
| MPM (McLeod) | 2005 | classical (autocorr) | <0.5 ms / 1024 | n/a | no | no | yes (`sevagh/pitch-detection`, has Rust port) | MIT |
| SWIPE | 2008 | classical | ~1 ms / 1024 | n/a | no | no | yes | various |
| Klapuri 2006 | 2006 | classical (harmonic sum) | ~3 ms / hop | n/a | yes (multi-F0) | optional | refs in Java/Python | various |
| Duan 2010 | 2010 | classical (joint) | offline | n/a | yes | no | reference impl. | research |
| CREPE | 2018 | CNN | 25–60 ms / 1024 | 22 M | no | no | yes (TF, ONNX via `yqzhishen/onnxcrepe`) | MIT |
| SPICE | 2020 | CNN, SSL | similar to CREPE | ~180 k | no | no | yes (TF Hub) | Apache 2 |
| BasicPitch | 2022 | CNN over HCQT | streamable, ~3 ms / frame on CPU | 16.8 k | yes (note-level) | no | yes, ONNX/CoreML/TFLite, C++ port | Apache 2 |
| RMVPE | 2023 | U-Net + GRU | ~30 ms / hop | 60–80 M | vocal in poly | no | yes (PyTorch) | MIT |
| Deep Salience | 2017 | CNN over HCQT | offline | 1–2 M | yes (multi-F0) | no | yes | MIT |
| MT3 | 2022 | T5 encoder-decoder | offline | 100M+ | yes (note transcription) | no | yes | Apache 2 |
| YourMT3+ | 2024 | hierarchical Transformer + MoE | offline | 100M+ | yes | no | yes | research |
| Self-supervised MPE (Cwitkowitz) | 2024 | conv autoencoder | offline / few ms | small | yes (multi-F0) | no | yes | research |
| FCPE | 2024/2025 | Lynx-Net (DSC) | RTF 0.006 GPU; CPU not reported | small | no | no | yes (PyTorch) | MIT |
| PESTO | 2023, RT 2025 | Siamese CNN over VQT | 0.7 ms ONNX, 12× faster than RT on CPU | 130 k | no | no | yes (PyTorch + ONNX export) | CC BY-NC-SA |
| SwiftF0 | 2025 | small CNN | ~2.8 ms/s CPU (≈ Praat) | 95.8 k | no | no | yes (PyTorch) | open |

CPU latency figures are *order of magnitude*; vendors do not report the
same conditions. The takeaway is the rank order, which is stable.

### Implementation candidates (open-source) — Rust-relevant

For building inside a Rust CLAP plugin, the choices are:

- **Bring no neural model; build IF+chromagram from existing STFT.**
  Cost: implementation work, ~500 lines Rust. Zero new deps.
  Pure-Rust SIMD via `std::simd` once stabilised, or `wide`/`packed_simd`
  in the meantime. **Recommended for v1.**
- **`pyin-rs`** — <https://github.com/Sytronik/pyin-rs>. Pure-Rust pYIN
  on `ndarray`. Monophonic only. License unclear but `ndarray`-based,
  RT-safety not guaranteed (allocates internally during processing).
  Useful as a *second pitch source* for the monophonic case (e.g.
  bass/lead identification).
- **`paramako/autopitch`** — <https://github.com/paramako/autopitch>.
  Modular Rust pitch-detection lib, "fast, zero-dep, real-time ready"
  per its own description. Untested by us; has Rust idioms close to
  what we need. Worth a 30-min audit to confirm RT-safety.
- **`pitch_detection` crate (`sevagh/pitch-detection`)** —
  <https://docs.rs/pitch-detection>. Wraps MPM/YIN. Has a Rust impl of
  MPM. Monophonic.
- **`rt-cqt` (C++)** — <https://github.com/jmerkt/rt-cqt>. Header-only
  real-time Constant-Q transform. If a CQT becomes preferable to STFT
  for the chromagram path, this is an FFI candidate. Not needed for the
  IF-refined HPCP route.
- **`basicpitch.cpp`** — <https://github.com/sevagh/basicpitch.cpp>.
  ONNX-Runtime C++ inference of BasicPitch. Could be exposed via FFI if
  Kim ever wants the *note-level* polyphonic transcription as a side
  feature (not required for the harmony module's primary use). Adds an
  ONNX-Runtime dep (~20 MB binary), so non-trivial.
- **`SonyCSLParis/pesto`** — has ONNX export; could be FFI'd via
  `ort` (Rust bindings to ONNX-Runtime) if monophonic pitch with
  best-in-class accuracy is needed for a feature like "tune the formants
  to the input pitch." The streaming variant from arXiv 2508.01488 is
  particularly attractive (cached convolutions for proper streaming).

### Recommendations

**Prototype first, integrate second, defer last.**

- **Prototype** the IF + harmonic-summation + chromagram pipeline in
  pure Rust on top of the existing STFT. Validate against:
  - solo bass guitar (low-E rejection at hop=512, sub-bin IF resolution),
  - solo piano (multiple harmonic groups detected, fundamentals correct
    in 90 % of frames),
  - sustained piano chord (chromagram gives correct 3 pitch classes),
  - drum loop (chromagram correctly *fails* — high entropy across all
    pitch classes, no chord match — which the chord-template stage uses
    as a "no chord" signal).
- **Integrate** the chromagram into `ModuleContext` so any module can
  read it (Harmony, Rhythm, Modulate). Cost: ~400 bytes/hop in shared
  state.
- **Integrate** the harmonic-group list (5–10 fundamentals + their bin
  indices) similarly. This is the data substrate every Harmony
  sub-effect (Chordification, Undertone, Companding, Inharmonic, FM
  Replicator, Harmonic Generator) reads.
- **Defer** any neural pitch model until a use case demands it.
  Candidates if needed later:
  - PESTO RT (130 k params, ONNX) for singer formant tracking.
  - BasicPitch (via ONNX) for *offline* MIDI-export of the input — this
    is a UI feature, not a DSP feature.
  - RMVPE for "extract vocal pitch from a mix" if a karaoke-style
    feature ever lands.
- **Hard avoid** MT3, ChordFormer, YourMT3+. They are all >100 MB
  parameter Transformers — not real-time, not desktop-friendly, and the
  bigger models all need a GPU.

Specifically for *chord recognition* (12-element chromagram → chord
label), the cheap stack to ship is:

1. cosine match against 24 templates (maj/min) → top-2 chord candidates,
2. 1st-order chord-bigram log-prior smoothing across hops,
3. confidence = ratio between top-1 and top-2 cosine scores.

A 1-layer GRU is *not* needed at this stage. If the smoothed templates
prove insufficient (Kim listens, hears flickering chord labels on dense
arrangements), the upgrade path is the madmom CRF — but at that point
we are running a CNN+CRF, which is the >2 ms/hop budget territory and
should be opt-in via a "deeper chord recognition" toggle.

---

## Topic B — Cepstral liftering edge cases

### Key references

- **Bogert, Healy & Tukey 1963, "The Quefrency Alanysis of Time Series
  for Echoes."** Original cepstrum paper.
- **Imai & Abe 1979, "Spectral envelope extraction by improved cepstral
  method."** Establishes the iterative cepstral envelope estimator —
  later renamed True Envelope.
- **Roebel & Rodet 2005, "Efficient Spectral Envelope Estimation and its
  application to pitch shifting and envelope preservation,"** DAFx.
  Speeds up True Envelope by 2-9×, makes it real-time. PDF:
  <https://hal.science/hal-01161334/document>.
- **Roebel 2005, "Real Time Signal Transposition with Envelope
  Preservation in the Phase Vocoder,"** ICMC. Real-time formant-preserving
  transposition using True Envelope. PDF:
  <https://hal.science/hal-01161347/document>.
- **Cappé & Moulines 1996, "Regularization techniques for
  discrete cepstrum estimation,"** IEEE Signal Processing Letters.
- **Galas & Rodet 1991, "Generalized functional approximation for source-
  filter system modeling."** Discrete cepstrum, the precursor to True
  Envelope.
- **Morise et al. 2014, "CheapTrick, a spectral envelope estimator for
  high-quality speech synthesis,"** Speech Communication 67. PDF:
  <https://www.semanticscholar.org/paper/CheapTrick%2C-a-spectral-envelope-estimator-for-Morise/64cc52369c7e778ff9b0bb4efe0a6f58ad60395a>.
- **Morise, Yokomori & Ozawa 2016, "WORLD: A Vocoder-Based High-Quality
  Speech Synthesis System for Real-Time Applications,"** IEICE
  Transactions. The full WORLD package (DIO/StoneMask + CheapTrick +
  D4C). Repo: <https://github.com/mmorise/World>.
- **Smith, *Spectral Audio Signal Processing,*** online book, chapter on
  Cepstral Windowing. Free at:
  <https://www.dsprelated.com/freebooks/sasp/Spectral_Envelope_Cepstral_Windowing.html>.
- **Aalto Speech Processing book, ch. 3.8** — practical guide,
  including the epsilon-clamping discussion:
  <https://speechprocessingbook.aalto.fi/Representations/Melcepstrum.html>.

### Synthesis

#### log(0) and the epsilon clamp

Standard practice in speech processing (MFCC pipelines for >25 years) is
`y = log(|X|² + ε)` where ε is set so that `log(ε)` lies a few dB below
the noise floor of the system. For 32-bit float audio, sensible values:

- **ε = 1e-10** (as a magnitude-squared) → `log(ε) ≈ −23` (natural log)
  ≈ −100 dB. Anything quieter than that hits the floor; nothing audible
  is at –100 dB if signal levels are normalised.
- For our use case, where bin magnitudes are on the order of 1e-3 …
  1e0 (after `NORM = 2/(3·N)`), ε of magnitude 1e-12 → `log(ε) ≈ –12`
  ≈ –120 dB is overkill but cheap. Use `ε = 1e-10` and log-base-e
  (natural). When converting back, `exp(y)` is finite and <1.

The classical "softer" alternative is `log(|X| + δ)` (linear, not power
spectrum) with `δ = 1e-6`. Same idea, different scale. Either is fine
provided round-trip `exp(log(|X| + δ)) – δ ≈ |X|` for `|X| > δ`. Choose
one and document.

A *third* approach used by some MFCC implementations:
`log_mag = max(log(|X|), -10)`. Pure clamp, no epsilon. Slightly cheaper
(no add), exactly equivalent for `|X| > 1e-5`. Audibly identical for our
use case. We can use this if benchmarking shows the `+ ε` add is
non-trivial under SIMD.

**Recommendation:** ε = 1e-10 added in the magnitude-squared domain
(matches Aalto reference and standard MFCC practice). One add per bin,
SIMD-trivial.

#### Silence detection / boundary handling

The brainstorm prompt's concern is real: on a silent frame, the cepstrum
is dominated by `log(ε)` everywhere → after liftering and `exp()`, the
output is `exp(0) = 1.0` magnitude in every bin → DC + broadband noise
output, much louder than the silent input. Audibly: a click + hiss when
silence begins, another click when sound returns.

Two robust answers:

1. **Bypass-on-silence with cross-fade.** Compute frame energy
   `E = Σ|X|² / N`. If `E < E_threshold` (e.g. -60 dBFS RMS = 1e-6 in
   normalised mag-sq), set the per-bin output to the *unmodified* input
   bin. To avoid clicks at the boundary, smooth the bypass with a
   one-pole `α = 0.05` over a 5-frame window:
   `wet = α·target_wet + (1-α)·prev_wet`, where `target_wet ∈ {0, 1}`
   for silent/active. Actual output `Xout = wet·Xliftered + (1-wet)·X`.

2. **Magnitude-conservation post-step.** After `exp()` to magnitude
   space, scale the output bins so that `Σ|Xout|² = Σ|Xin|²`. This
   prevents the cepstrum-driven output from raising the noise floor on
   silent input. Cheap (one ratio computation per frame). This is what
   the WORLD vocoder does internally for its synthesis stage.

We recommend *both*. The bypass handles the catastrophic-silence case
cleanly and avoids spending CPU on garbage. The magnitude-conservation
handles partial silence (one channel quiet, one loud) and the soft
boundary case.

`E_threshold = 1e-6 in normalised |X|²` ≈ -60 dBFS is conservative; on
classical/orchestral material with low passages the user might want
this lower (or off entirely with a bypass-disable toggle). Expose
`silence_floor_db` as a user parameter, default -60 dBFS, range
-120…-30 dBFS.

#### Phase coherence with magnitude-only edits

Cepstral liftering only modifies the magnitudes; the original phases
are reused. For sustained tones this is the textbook-clean case
(magnitudes change slowly, phases naturally track them). For transients,
the issue is more nuanced than the brainstorm states:

1. **Within-frame**: phases are not "broken." The output is still a
   valid spectrum (any (mag, phase) pair is valid). What changes is the
   *time-localisation* of the transient: scaling the magnitude envelope
   shifts the energy distribution, which slightly smears the transient.
2. **Cross-frame OLA**: magnitudes change → small shifts in
   group-delay → small additional smear at frame boundaries. The
   smearing is at most 1-2 hops × hop_size = a few ms for hop=512 at
   44.1 kHz. Audible? On dry kicks/snares: yes, slightly. On most
   instrumental material: no.

For Spectral Forge's use case (a slot inside a chain that the user
opts into for vowel-morph / formant work), this is *acceptable*. It is
not acceptable as a default-on global filter. Two mitigations if needed
later:

- **MIX curve** (curve 6 in the standard slot) — already in the system.
  Default 100 % wet but easy to dial back.
- **Transient bypass.** If the T/S Split module is upstream, we can
  read its `transient_mask` per bin (already in the BinPhysics field
  list per `01-global-infrastructure.md`) and reduce liftering depth on
  flagged transient bins. This is the Lifter-mode equivalent of
  "transient preservation" in a multiband compressor.

For a sustained-tone use case (vocal vowel morphing, organ chord
shaping), the phase issue does not arise.

#### World vocoder vs naive cepstrum for envelope morphing

These are not interchangeable. They solve different problems with
different costs.

- **Naive cepstrum (Bogert/Tukey 1963)** = log-mag, IFFT, lifter low
  quefrencies, FFT, exp. *Two extra FFTs per hop.* Smooths over the
  pitch's harmonic series nicely on stationary tones; biased *high* on
  voiced speech because the harmonic peaks pull the envelope toward
  themselves (the "ripple" problem in speech-recognition MFCC pipelines).
- **Discrete cepstrum (Galas-Rodet 1991, Cappé-Moulines 1996)** =
  least-squares fit a smooth cepstral curve to *only the spectral peaks*.
  Fixes the bias but requires peak detection and a regularised solve.
- **True Envelope (Imai-Abe 1979, Roebel 2005)** = iterative cepstrum:
  start from `log|X|`, repeatedly take `max(log|X|, current_estimate)`
  and re-cepstrally-smooth, until convergence (typically 5-20
  iterations). Result: the envelope sits *on top of* the harmonic
  peaks, not between them. **This is the modern best practice for
  formant-preserving pitch shifting.** Cost: 5-20× the naive cost,
  i.e. ~30-120 ms/hop at 8193 bins on CPU, single-threaded. Roebel's
  optimisation gets this down to ~5-10 ms/hop, real-time feasible
  within a single slot.
- **CheapTrick (Morise 2014)** = WORLD's envelope estimator. Adapts
  the analysis window to the F0, applies cepstral smoothing in a
  pitch-synchronous way, then does liftering with a scale-invariant
  shape function. Comparable accuracy to True Envelope at lower cost
  but *requires accurate F0*. If we have F0 from the IF + harmonic
  pipeline (Topic A), CheapTrick is ~3-5× cheaper than True Envelope.

For the Lifter sub-effect:

- **Default mode**: naive cepstrum with low-quefrency lifter shape.
  Cost: 2 FFTs/hop. Adequate for "EQ the timbre." Does what users
  expect.
- **High-quality mode** (opt-in): True Envelope with N=10 iterations.
  Cost: ~12 FFT-equivalents/hop. Use for vowel morphing and serious
  formant work.
- **CheapTrick mode** (opt-in, requires Harmony's F0 stream): ~5
  FFT-equivalents/hop. Use when the input is monophonic and reasonably
  pitched. In a polyphonic context where F0 is unstable, fall back to
  True Envelope.

The user picks per-slot (cheap by default, premium on demand). The CPU
class flag in `ModuleSpec` already supports this kind of tiering.

### World vocoder vs naive cepstrum

| Aspect | Naive cepstrum | True Envelope (Roebel) | CheapTrick (WORLD) |
|---|---|---|---|
| Year | 1963 | 1979/2005 | 2014 |
| Cost per hop (8193 bins) | 2 FFTs | 5-20 FFTs (Roebel: ~5) | ~5 FFT-equivalents |
| F0 dependency | none | none | requires F0 |
| Bias on voiced signals | biased low (envelope sits between harmonics) | unbiased | unbiased |
| Suitability for vowel morphing | adequate | excellent | excellent if F0 known |
| Real-time feasible (hop=512) | trivial | with care | yes |
| Code references | Smith SASP, Aalto book | IRCAM (closed but well-documented) | `mmorise/World` (BSD) |
| Best fit | "Lifter" default | Lifter HQ mode | future "VowelMorph" sub-effect |

### Implementation candidates

Pure-DSP cepstrum is so simple no library is needed; we already have
`realfft` for the inner FFT pair. The True Envelope and CheapTrick paths
can borrow code structure but the actual implementation is per-project.

Reference implementations to consult:

- **WORLD vocoder, C++** — <https://github.com/mmorise/World>. BSD
  license. The CheapTrick module is in `cheaptrick.cpp`/`.h`. Translate
  to Rust; ~300 lines. The DIO+StoneMask F0 path is *not* needed if we
  bring our own F0 from Topic A.
- **`world-class` C++ wrapper** — alternative to mmorise/World's
  reference. Same algorithms. Same license.
- **`pyworld`** — <https://pypi.org/project/pyworld/>. Python wrapper.
  Useful for prototyping/testing reference outputs.
- **`pyceps`** — <https://github.com/hwang9u/pyceps>. Python ref impl
  of cepstral analysis (F0 + envelope estimation). 200 lines of NumPy,
  good educational resource for the naive cepstrum implementation.
- **`pvoc-rs`** — <https://github.com/nwoeanhinnogaehr/pvoc-rs>. Rust
  phase-vocoder. Not directly cepstrum, but the FFT-pair scaffolding
  pattern is the same. Useful for matching the existing project style.
- **librosa.feature.melspectrogram + librosa.power_to_db** — Python
  reference for log-mag handling and de-clipping; shows
  `librosa.power_to_db(..., top_db=80.0)` as the standard "noise-floor
  clamp" technique.
- **Roebel's True Envelope description with figures** — best free
  description in the original 2005 DAFx PDF
  (<https://hal.science/hal-01161334/document>). The iterative core
  algorithm is ~20 lines of pseudocode. We can implement directly from
  that paper.

### Recommendations

- **Ship naive cepstrum as the Lifter default.** 2 extra FFTs/hop fits
  comfortably in our budget. Validates the X-axis quefrency display
  spec (`02-architectural-refactors.md` §8) and the
  envelope/pitch curve UX. Cost: one weekend.
- **Add True Envelope as "HQ mode."** Opt-in flag in `ModuleSpec`
  (`needs_true_envelope: bool` or per-instance). Cost: 1-2 weeks
  including iteration-budget tuning and SIMD optimisation.
- **Defer CheapTrick** until both Lifter is shipping and the Harmony
  module's F0 detection is in place. CheapTrick's win over True
  Envelope is the F0-aware analysis window, which only helps when F0
  is reliable. Until we have that, True Envelope HQ is the right
  choice.
- **ε = 1e-10** in magnitude-squared for log-clamp, consistent across
  all cepstrum-using paths.
- **Silence bypass** with smooth cross-fade, default threshold –60
  dBFS RMS, exposed as user parameter.
- **Magnitude conservation** post-step in *all* cepstrum modes. Cheap,
  audibly correct.
- **Phase reuse** (no phase modification) for v1. If transient
  artefacts surface in user testing, add transient-mask-driven depth
  reduction in v2.

---

## Cross-topic synthesis

The two topics share infrastructure and the integration is natural:

1. **Both want a single `ModuleContext::cepstrum_buf`.**
   - Cepstral liftering reads it, edits it, writes it back via FFT
     to overwrite magnitudes.
   - Cepstrum-based pitch tracking reads it (the high-quefrency peak
     gives F0 in non-noisy tonal material) as a *cross-check* on IF.
   - Currently no other module is planned to use the cepstrum, so this
     is shareable between Lifter and a future "cepstral pitch" detector
     only. Do not over-engineer the share — declare
     `needs_cepstrum: bool` in `ModuleSpec` and let the Pipeline
     compute on demand.

2. **Both want a robust F0 stream.**
   - Topic A's pipeline outputs (a) chromagram, (b) harmonic-group
     list, (c) per-group `f₀` and confidence.
   - CheapTrick (Topic B's HQ envelope path) can read `f₀` directly to
     pick its window length. This is exactly the dependency direction
     in WORLD: F0 → CheapTrick → liftering.
   - In a polyphonic context, "the F0" is ambiguous. Use the
     highest-confidence harmonic group's fundamental as CheapTrick's
     working F0; if confidence is low, fall back to True Envelope.

3. **Cepstrum can complement IF, doesn't replace it.**
   - At our hop rates and fft sizes, IF is strictly better for *bin-
     accurate* frequency estimation (sub-bin resolution, no F0 needed).
   - Cepstrum-based pitch is better for *globally-consistent* F0 on
     stable tones: the rahmonic peak in the cepstrum doesn't drift
     when the harmonic series is clean. It is *worse* on noisy signals
     and on signals where the F0 is below the cepstral resolution
     (bass below ~80 Hz needs a long FFT for cepstral pitch to work,
     longer than our default hop).
   - Use both as a *consistency check*: IF + harmonic summation
     proposes F0 candidates; cepstrum confirms or rejects them. If they
     disagree by >50 cents, raise uncertainty.

4. **Phase information from PVX peak-locking integrates with both.**
   - Stable-peak detection (Topic A step 2) is the *same set* of peaks
     that PVX peak-locking acts on. Compute once in the Harmony /
     Topic A pipeline; share via `ModuleContext::stable_peaks` or a
     `peak_set: Vec<PeakDescriptor>` for downstream PVX modules.
   - Cepstral edits do not modify phase, so PVX peak-lock is
     unaffected by Lifter sitting in the same chain.

5. **No shared neural model is needed.** This is the single biggest
   architectural finding: the modern neural pitch detectors (CREPE,
   PESTO, BasicPitch, RMVPE, FCPE, SwiftF0) all duplicate work we
   already do, are mostly monophonic, and add 100 KB-100 MB of model
   data plus an ONNX-Runtime dependency for a feature that classical
   IF + harmonic summation already delivers at sub-millisecond cost.
   A neural model only earns its keep if a feature *requires* its
   robustness on a specific signal class — vocal pitch from a mix
   (RMVPE), or extremely noisy material (SwiftF0). Add as opt-in,
   not as primary path.

---

## Open questions

1. **What is the right F0 candidate grid for Klapuri-style harmonic
   summation in our 20 Hz – 20 kHz range with hop=512 at 44.1 kHz?**
   Theoretical floor: log-spaced 1/12-octave gives ~120 candidates
   over 10 octaves. Coarser grid (1/6 octave) gives 60 — enough for
   chord detection, marginal for fine-pitched bass. Empirical
   validation needed against Bach10 / MAPS / MAESTRO.

2. **How many harmonics should the harmonic-summation salience use?**
   Klapuri 2006 uses 20 with the (h+α)/(h·β+α) weighting. Lower
   harmonic counts (8-12) are cheaper and might be sufficient for
   chord-level accuracy. CPU vs accuracy sweep needed.

3. **Is iterative cancellation worth the extra cost over single-pass
   peak-picking on the salience function?** Klapuri's iterative method
   improves polyphonic accuracy but costs O(K · num_bins) for K
   iterations. For 1-3 voice polyphony (common case), single-pass
   top-K extraction may be sufficient; for 5+ voices (organ, full
   piano), iterative wins. Decision likely "ship single-pass, expose
   iteration count as advanced parameter."

4. **Chord template depth: 24 (maj/min) or 60 (+dim/aug/sus2/sus4/7)?**
   60-template matching is still <1 µs per hop and improves expressive
   chord recognition (which makes Chordification more interesting).
   Recommend 60 from day one.

5. **Should the chromagram be a per-bin contribution (Fujishima 1999) or
   a per-IF contribution (the IF-refined HPCP we propose)?** IF-refined
   is strictly more accurate but costs an IF lookup per bin. We have
   IF anyway. Use IF.

6. **Does the chromagram need the 5/3 partial reduction
   (Gomez 2006 EHPCP) or is the IF-refined version self-correcting?**
   Empirical question; both should be implemented and ABX-compared on
   solo piano vs. full mix.

7. **Naive cepstrum vs True Envelope for the default Lifter mode:** is
   the bias of naive cepstrum audible on the user's typical material?
   If Kim's testing on instrument samples shows it's fine, ship naive
   cepstrum as default to keep CPU low. If the bias is audible (likely
   on solo voice / sax / harmonica), promote True Envelope to default.

8. **Quefrency-axis display unit:** seconds (literal cepstrum domain),
   1/quefrency in Hz (audibly meaningful, "this region affects
   things at *X* Hz harmonic spacing"), or a custom dimensionless
   "cepstral index 0-N"? Display spec deferred to
   `02-architectural-refactors.md` §8.

9. **Does the Lifter need a "phase manipulation" curve at high
   quefrencies?** Editing the phase cepstrum (rather than just the
   magnitude cepstrum) gives access to phase-only effects. Almost no
   one does this in the literature; possibly a unique angle for the
   plugin. Out of scope for v1, worth noting.

10. **Real-time hot-swap of F0 algorithm:** if/when we add a neural F0
    (PESTO RT for monophonic), should the Harmony module hot-switch
    between IF-only and IF+PESTO based on whether the signal is
    monophonic? Detection: chromagram entropy. If entropy < threshold
    → monophonic → use PESTO; otherwise use IF + harmonic summation.
    Adds complexity but materially improves accuracy on solo material.
