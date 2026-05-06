# Consolidated Research Prompts

> **Status (2026-04-26):** the six HIGH/MED prompts in this file have
> been answered by the research sweep under `research/`. The
> cross-cutting takeaways live in
> [`91-research-synthesis.md`](91-research-synthesis.md). The
> remaining prompts here are LOW-priority follow-ups or open after
> the synthesis. Read the synthesis first.

**Purpose:** Copy-pasteable research prompts grouped by topic for use
in heavier frontier-AI sessions. Each prompt block is identical to the
one in the per-module audit file referenced — included here so the
whole research backlog can be scanned, reordered, or batch-handed-off
without spelunking through 16 files.

**How to use:** pick a block, paste into a frontier-AI session
(Claude Opus / Gemini 2.5 Pro / GPT-5 with extended thinking), iterate
on the answer. Bring the result back as either an update to the
relevant per-module file or as a new spec section.

**Prioritization heuristic.** Prompts marked **HIGH** are the ones
where the answer unblocks the most other modules. **MED** unblock a
single deferred module. **LOW** are quality-of-life refinements that
can wait.

---

## Phase / PVX / Pitch family

### 1. PVX peak-locking math validation against ProSoniq research — **HIGH**

**Source:** `20-plpv-phase-cross-cutting.md`
**Unblocks:** Dynamics quality, Modulate (PLL Tear, Phase Phaser),
Past (Stretch), Freeze, PhaseSmear, Harmony (Inharmonic).

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

### 2. SOTA real-time pitch tracking with phase information — **HIGH**

**Source:** `15-harmony.md`
**Unblocks:** Harmony (all sub-effects), Modulate (FM Network), Punch
(watch-range curve), Compander (poly-tracking).

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

### 3. Phase coherence in stretched STFT playback — **MED**

**Source:** `13-past.md`
**Unblocks:** Past (Stretch mode), Past (Reverse mode polish).

```
Topic: Phase-coherent playback of a frequency-domain history buffer
at variable read rates.

Context: We have a rolling buffer of complex STFT frames (8193 bins,
hop=128 to 1024 depending on user setting). We want to play back at
arbitrary rate (0.25× to 4× the recording rate) without phase glitches
that would manifest as audible artifacts in overlap-add reconstruction.

Goal: Identify the cheapest correct phase-rotation scheme for variable-
rate STFT playback. Naive linear interpolation of complex bins fails;
classic phase-vocoder phase rotation works but is expensive.

Specific questions:
1. Is "phase rotation by 2π × bin_freq × time_offset" sufficient when
   reading from history at fractional frame positions? What about
   bins where the actual partial drifts off bin-center (use IF)?
2. For 0.25× / 0.5× / 2× / 4× (integer + simple ratios), can we
   precompute phase-rotation LUTs?
3. Phase-locking neighbouring bins (per Laroche/Dolson) costs O(N)
   per hop. Is it worth it for our use case where the user might
   intentionally want some warble?

Deliverable: A Rust algorithm + reference implementation comparison
(naive lerp vs phase-vocoder vs phase-locked vocoder) with audio
examples and CPU costs at 8193 bins.
```

### 4. Real-time per-bin PLL stability — **MED**

**Source:** `16-modulate.md`
**Unblocks:** Modulate (PLL Tear quality).

```
Topic: Per-bin phase-locked-loop stability under transient input for
a real-time spectral plugin.

Context: We have a per-bin PLL (8193 bins) tracking the phase of the
previous STFT frame. Each PLL has 2nd-order dynamics (phase + freq).
When the input frequency in a bin glides faster than the loop
bandwidth, we want the PLL to *audibly* lose lock and emit chaotic
sub-octave phase noise until the input stabilizes. This is the "tear"
sound.

Specific questions:
1. Loop bandwidth scaling per bin: should higher-frequency bins have
   wider loop bandwidth (their phase advances more per hop) or the
   same? What about bins below 100 Hz where two hops may not contain
   a full cycle?
2. Lock-loss detection: instantaneous phase-error threshold vs.
   sliding-window phase-error variance? The latter is smoother but
   adds state. For a "tear" effect we want the loss to be punchy and
   coincide with the transient, so maybe instantaneous + hysteresis.
3. Re-lock behaviour: when the input stabilizes, does the loop snap
   back smoothly or does it "ring" through several hops? What's
   audibly best?
4. PVX unwrapped phase as PLL input: does using unwrapped phase
   actually help, given a per-bin PLL only needs delta-phase? The PVX
   unwrap may be redundant work here but cheap to share.
5. Stereo: Independent-mode stereo means two separate PLL banks. If
   the same input glide hits both channels, do they tear in sync (so
   the mono-sum is clean) or with slight phase drift (so the mono-sum
   has chorus-y artefacts)?

Deliverable: A reference PLL kernel + parameter tuning notes for the
"tear" character. Audio examples comparing the various lock-loss
detection schemes.
```

### 5. Phase-orbit math for Orbital sub-effect — **LOW**

**Source:** `12-kinetics.md`
**Unblocks:** Kinetics (Orbital sub-effect).

```
Topic: Computing physically plausible "orbit" of a phase around a
mass-attracted center, per FFT hop.

Context: A Kinetics sub-effect treats loud bin K as a "massive object"
and lighter neighbour bins K±n as "satellites." Each satellite's phase
is forced to orbit the massive bin's phase at a rate proportional to
1/distance.

Goal: Define what "orbit" means in phase space such that:
- The output sounds like physically modeled vibrato + spatial width
- Zero phase = the satellite is "behind" the massive bin in some sense
- Two satellites at opposite distances from the same mass orbit in
  opposite directions (creating beat patterns)
- Multiple massive bins create non-trivial orbit perturbations
  (audible chorus, not noise)

Open question: do we use Kepler-style elliptical orbits (cool but
expensive) or just rotate the satellite phase by a per-hop angle
proportional to 1/distance × strength (cheap, sounds like phase-
modulated chorus)?

Deliverable: a formula + audio examples literature reference.
```

### 6. Phase-coherent neighbour-bin pitch drift — **LOW**

**Source:** `19-punch.md`
**Unblocks:** Punch (Pitch-Fill sub-mode).

```
Topic: Per-bin phase-coherent micro-pitch-shifting of small
neighbour bin clusters toward a target bin (the carved hole).

Context: When a hole is carved at bin H, neighbouring bins H±1, H±2,
... drift toward H by a small phase rotation each hop. We need this
drift to be phase-coherent with the existing bin's content — i.e.,
the iFFT shouldn't reveal a click or a discontinuity.

Specific questions:
1. The drift offset is a fraction of a bin (0–0.5 bins). Standard
   phase-vocoder pitch shifting works at integer hops; sub-bin
   drift is a phase rotation = `2π × Δf × hop / sample_rate`. Is
   that sufficient or do we need IF-aware corrections?
2. When the drift releases (sidechain quiets), do we drift back to
   zero or just freeze in place? Drifting back risks a second
   audible motion; freezing creates a small permanent bin shift
   until the next carve.
3. Stereo: in Independent mode, both channels carve independently.
   Should the drift be locked across channels (mono fill) or
   independent (stereo width)?

Deliverable: Reference kernel + comparison of three drift release
strategies (return-to-zero, freeze-in-place, slow-drift-to-zero) on
sustained pad with periodic carve.
```

---

## DSP kernels — per-bin numerical math

### 7. SIMD analog kernels — **HIGH**

**Source:** `10-circuit.md`
**Unblocks:** Circuit (entire module), Life (cheap saturators), Modulate
(Diode RM).

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

### 8. Numerical stability of spring networks at audio rates — **MED**

**Source:** `12-kinetics.md`
**Unblocks:** Kinetics (Springs sub-effect, Sympathetic sub-effect).

```
Topic: Stable spring/mass simulation across an FFT spectrum at audio
hop rate.

Context: We want each FFT bin to behave as a mass connected to its
neighbours by springs (and optionally to harmonic-related bins).
Update happens once per STFT hop — at 44.1 kHz / hop=512, that's
~86 Hz update rate. At 44.1 kHz / hop=128, ~344 Hz. Springs are
stiff; mass varies per bin; user-driven curves can change everything
between hops.

Goal: A semi-implicit or symplectic integrator that:
- Stays stable across the hop rate range without per-hop substepping
  (substepping kills CPU)
- Handles fast user changes to spring constants without exploding
- Is SIMD-friendly (per-bin mass + displacement + velocity arrays,
  adjacent reads for spring forces)

Specific questions:
1. Verlet vs Velocity Verlet vs Implicit Euler: which gives the best
   stability/CPU tradeoff at our update rate?
2. Stiffness limits: at what max spring constant does the integrator
   blow up at hop=512? At hop=128?
3. When the user modulates spring stiffness fast (per-hop), do we get
   parametric amplification (audible "ringing up" of the simulation)?
   How to damp safely without losing the desired ringing?
4. Sympathetic harmonic springs: bin K connected to 2K, 3K, 4K. The
   memory access pattern is non-stride-1. Worth a CSR-style sparse
   matrix with cached neighbour offsets, or per-bin small fixed array?

Deliverable: a Rust integrator skeleton with stability proof / numerical
analysis, plus a per-mode CPU benchmark estimate at 8193 bins.
```

### 9. Energy-conservation in spectral diffusion — **MED**

**Source:** `11-life.md`
**Unblocks:** Life (Spectral Bleed, Viscosity, Capillary).

```
Topic: Energy-conserving spectral diffusion operators

Context: We have a spectral plugin doing per-hop magnitude redistribution
across an STFT (8193 bins, sub-millisecond budget per hop). We want a
diffusion operator (per Kim's "energy must go somewhere") that:
- Spreads loud-bin magnitude into adjacent bins
- Conserves total energy (sum of |bin|^2) within tolerance
- Is SIMD-friendly (stride-1 reads/writes)
- Has a per-bin reach parameter (graph-driven)

Specifically: is a discrete heat equation (1D Laplacian smoothing of
magnitude) sufficient, or do we need a Lattice Boltzmann or finite-
volume scheme to get visually-correct conservation? Audio-perceptually,
is the difference noticeable, or does any conservative-enough scheme
sound fine?

Bonus: how does adding "viscosity" as a per-bin diffusion coefficient
interact with stability — does the operator become non-monotone, do
we need flux-limiters?

Deliverable: One Rust kernel with explicit conservation analysis
(numerical + audio-perceptual), reference to literature, and a
benchmarked comparison of plain Laplacian vs flux-limited.
```

### 10. Predictive spectral extrapolation accuracy — **MED**

**Source:** `14-future.md`
**Unblocks:** Future (Predicted Spectrum Interpolation).

```
Topic: Per-bin predictive extrapolation of spectral magnitudes for
real-time anticipation effects.

Context: We have a sequence of STFT frames (one per hop, hop=128–512
samples). We want to predict frame N+1's magnitudes from frames
N-K…N for various K (3, 4, 8). Goal: prediction good enough that
mixing the predicted frame in at 30% gives perceptual "tightness"
without sounding broken.

Specific questions:
1. Linear extrapolation of magnitude in dB vs. linear: which is more
   audibly forgiving when the prediction is wrong (and it will be wrong)?
2. Per-bin AR(K) with simple Burg/Yule-Walker fit each block: too
   expensive at 8193 bins?
3. Phase prediction is hard (phase is wrapped). Just keep current
   phase and only predict magnitude?
4. When the input is steady (sustained chord), prediction is trivially
   accurate. When it's transient (drum hit), prediction is catastrophic.
   Is there a simple "prediction confidence" per bin we can compute
   cheaply, and downgrade to dry signal when confidence is low?

Deliverable: a Rust kernel with confidence weighting + audio examples
showing the failure mode (mispredicting a transient).
```

### 11. Cepstral liftering edge cases — **MED**

**Source:** `15-harmony.md`
**Unblocks:** Harmony (Lifter sub-effect).

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

### 12. Real-time 2-D wave equation on Hilbert-mapped spectrum — **LOW**

**Source:** `18-geometry.md`
**Unblocks:** Geometry (Wavefield sub-effect, deferred to v2).

```
Topic: Real-time discrete 2-D wave equation simulation on a
locality-preserving 1-D-to-2-D mapping (Hilbert curve), with bin
indexing and SIMD-friendly memory access.

Context: We have a 1-D spectrum of N bins (N up to 8193). We want to
map it to a 2-D grid (M × M ≈ 91 × 91 for N=8193) via a Hilbert
curve so bin neighbours are spatial neighbours. Each hop, run one
finite-difference 2-D wave-equation step:
  u(x,y, t+dt) = 2u - u_prev + c² dt² · (Δ_x² + Δ_y²) u
where the Laplacian uses the 4-connected stencil. Then project the
2-D grid back to 1-D via the same Hilbert mapping for iFFT.

Specific questions:
1. Does the Hilbert curve's locality preservation hold up under the
   wave equation, or do non-Hilbert-neighbour bins still "feel" each
   other strongly enough that the mapping is irrelevant?
2. CFL stability: c × dt / dx < 1/√2 in 2-D. What c value gives
   audibly interesting ringing without violating CFL? At what hop
   rate (dt = hop / sample_rate) is c constrained?
3. Boundary conditions: absorbing (Mur first-order), reflective
   (Neumann), periodic (toroidal). Which gives the most musically
   useful behaviour for the spectrum-as-substrate metaphor?
4. SIMD: a 91×91 wave step has annoying boundary handling. Pad to
   96×96 and ignore boundary rows? Use AVX-512 to do 16 cells per
   instruction?
5. Per-bin boundary curve: the user draws a boundary-reflectivity
   curve over the 1-D spectrum. Mapping that to 2-D via Hilbert
   should give a meaningful spatial boundary, but does it "feel"
   meaningful to a producer turning a knob?

Deliverable: Rust kernel + audio examples on a sustained sine, a
chord, and a drum loop. Compare boundary modes audibly.
```

---

## Source separation / analysis

### 13. Persistent homology of STFT magnitude for source isolation — **LOW**

**Source:** `18-geometry.md`
**Unblocks:** Geometry (Persistent-Homology sub-effect, deferred to v2).

```
Topic: Real-time persistent homology on the 2-D (time × frequency)
STFT magnitude landscape, used to isolate persistent spectral
features.

Context: We hold a rolling history buffer of N STFT frames (32 to
256, settable). We want to run persistent-homology analysis on the
2-D grid every K hops, identify the M most persistent maxima, and
use that subset to gate or weight the current hop's bins.

Specific questions:
1. Sub-level vs super-level filtration: which captures
   musically-relevant features? Maxima are super-level; saddles and
   valleys are sub-level.
2. Streaming computation: most persistent-homology libraries are
   batch. What's the streaming algorithm cost when the analysis
   window slides by one frame each hop? Can we incrementally update
   the persistence diagram?
3. Persistence threshold mapping: the user draws a curve setting the
   threshold per frequency. Does this map sensibly to "show me peaks
   that survive at least X dB of magnitude variation"?
4. Real-time CPU: at N=64 frames × 8193 bins, is even the
   batch-recompute cost feasible at 4-hop rate?
5. Output mapping: once we have the persistent maxima list, how do
   we translate to bin gates? Rectangular gate around each maximum,
   or smooth (Gaussian) per maximum, or something else?

Deliverable: a reference implementation comparison (one batch
algorithm baseline, one streaming attempt) with CPU profiles and
audio examples on a sustained chord with noise added.
```

---

## Perceptual / quality

### 14. Spectral hole-and-fill perceptual quality — **MED**

**Source:** `19-punch.md`
**Unblocks:** Punch (entire module).

```
Topic: Perceptual quality of sidechain-driven spectral hole carving
with neighbour-pitch-fill versus neighbour-amp-fill.

Context: A sidechain spectrum drives the carving of "holes" in the
input spectrum at the sidechain's peak frequencies. The surrounding
bins fill the hole either by amplitude boost (the "duck and lift"
behaviour) or by pitch drift toward the hole (the "neighbour fall in"
behaviour). We want to understand which fill mode reads as more
musical / less artefacted.

Specific questions:
1. Pitch fill creates *frequency modulation* of the neighbour bins
   each time the sidechain peaks. Does this artefact-free for
   harmonic content, or does it create audible chirping?
2. Amp fill creates per-hop magnitude jumps. Smoothing across hops
   (1-pole follower) is required to avoid clicks. What time constant?
3. The "depth × width" 2-D space (depth 0–1, width 0–N bins) — what
   region is musically useful? Some combinations are surely just
   pumping artefacts.
4. Healing curve shape: linear, exponential, sigmoid? Which feels
   most like a natural spectral resilience?
5. Sidechain bandwidth: do we want to detect peaks across the entire
   sidechain spectrum, or only within a per-bin "watch range" (the
   user draws a curve specifying where in the spectrum each input
   bin should listen for sidechain peaks)?

Deliverable: Rust implementations of both fill modes with audio
examples on (a) bass + kick, (b) lead vocal + sibilant noise, (c)
drum loop into reverb tail. Comparative listening notes.
```

### 15. Buchla-style amplitude envelope model for RM gating — **LOW**

**Source:** `16-modulate.md`
**Unblocks:** Modulate (RM/FM Matrix character).

```
Topic: Audio-rate amplitude envelope generators that gate ring-mod
output for "bongo" / Buchla 281 character without sounding like a
gate.

Context: Our RM/FM Matrix mode multiplies bin-by-bin with a sidechain
spectrum. The brainstorm note (#19) asks for "buchla-esque bongo
envelopes triggered by notes or peaks, to not just come at a wall of
noise." We need a per-bin envelope that triggers on sidechain peaks,
has fast attack, exponential decay, and re-triggers cleanly under
rapid input.

Specific questions:
1. Envelope shape: AR vs ADSR vs lopass-shaped exponential. Which
   gives the most "Buchla 281" character without sounding like a
   plain gate?
2. Per-bin trigger detection: cross every bin's threshold individually
   or use a sidechain band-energy detector that triggers a band of
   envelopes simultaneously? The latter is closer to Buchla's
   discrete sub-band design.
3. Re-trigger smoothing: how to handle a new peak arriving before the
   envelope decays — abort + retrigger, sum, or maxed?
4. CPU: 8193 envelopes per hop is fine, but we may be at 4 channels +
   independent stereo = 65k envelopes. SIMD strategy?

Deliverable: A reference envelope kernel comparing AR/ADSR/lopass-exp,
plus audio examples on a synth-pad sidechain through a noise input.
```

---

## Suggested batching for one frontier-AI session

If you have one heavy session and want maximum coverage:

1. **Foundational batch (start here):** prompts 1, 2, 7. These
   together unblock 80 % of the deferred work. PVX + pitch-tracking
   + SIMD analog patterns are the three load-bearing pieces.
2. **Quality batch:** prompts 3, 14. Phase-coherent stretch and
   hole-fill perceptual evaluation feed directly into the most
   audible new modules (Past Stretch, Punch).
3. **Integration batch:** prompts 4, 9, 10, 11. PLL + diffusion +
   prediction + cepstral edges are tractable individually but
   benefit from a single session that can compare formulations
   across them (they all involve "what's the right per-bin
   numerical scheme").
4. **Deferral candidates:** prompts 5, 6, 8, 12, 13, 15. Each
   unblocks a single feature; defer until that feature is the next
   priority.

Total prompts: 15. Total topics: 4. The list is intentionally long
— each prompt is a real research question that would otherwise
require Kim to re-derive context across multiple files. Having them
in one place is the time saving.

---

## Research outputs (in-flight as of 2026-04-26)

Six parallel research agents were dispatched to consume arxiv / GitHub /
DSP literature and write findings into `research/`. Each file consolidates
multiple prompts from the list above:

| Output file | Prompts covered | Topic cluster |
|---|---|---|
| `research/01-pvx-phase-and-pll.md` | 1, 4 | PVX peak-locking math + per-bin PLL stability |
| `research/02-pitch-and-cepstral.md` | 2, 11 | SOTA real-time pitch tracking + cepstral liftering |
| `research/03-physical-models.md` | 8, 9, 12 | Spring/mass integrators + diffusion/heat eq + Helmholtz/Chladni |
| `research/04-simd-analog.md` | 7 | SIMD analog kernels (Vactrol, Schmitt, BBD, Transformer) |
| `research/05-time-manipulation.md` | 3, 6, 10 | Phase-coherent stretch + tape print-through + onset prediction |
| `research/06-specialized-topics.md` | 5, 13, 14, 15 | Hilbert-curve mapping + spectral homology + hole-fill perception + Buchla envelopes |

When a file lands, the corresponding prompt(s) above can be marked
**ANSWERED** and the per-module audit file can be updated with the
findings. The research files are the canonical answer; the prompts here
exist only for re-running or deepening a specific question later.
