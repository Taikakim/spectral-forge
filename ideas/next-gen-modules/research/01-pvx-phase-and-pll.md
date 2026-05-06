# Research: PVX Phase Locking + Per-Bin PLL Tear

**Source prompts:** prompts 1 and 4 in `90-research-prompts.md`
**Status:** Findings as of 2026-04-26
**Researcher:** Claude Opus 4.7 via Agent dispatch

---

## Refined research questions

While searching, the original prompts split apart into a sharper set of
sub-questions. Each one drove (or could not be answered by) the search:

**On Topic A (PVX / peak-locked PV):**

1. **Does "PVX" actually exist as a published algorithm?** Or is the
   brainstorm mis-labelling something else (Laroche-Dolson, MPEX, ZTX,
   PGHI)? — Mostly answered. PVX has no academic paper; the term is
   used in Csound/CDP land as a *file format* extension (`.pvx`,
   PVOC-EX), not an algorithm. ProSoniq's flagship algorithms were
   **MPEX** (Multiple Component Feature Extraction, neural-network
   driven) and later **ZTX** (wavelet-based). Neither is a vanilla
   peak-locked PV. The "PVX phase-locking math" the brainstorm cites
   appears to be a folk synthesis of **Puckette 1995** (loose
   locking) + **Laroche & Dolson 1999** (rigid / scaled locking) +
   **Roebel 2003** (transient handling).
2. **What is the canonical "vertical phase coherence" formula?** —
   Answered. Two flavours: identity locking and scaled locking.
3. **How is a peak detected and how wide is its skirt?** — Answered
   for Laroche-Dolson (local 4-neighbour max + region-of-influence
   bounded by midpoint or magnitude-minimum between adjacent peaks).
4. **What replaces peak picking in 2020s PV literature?** — Answered.
   **PGHI / RTPGHI** (Pruša & Holighaus, 2017–2022) eliminates peak
   picking entirely and is the new state of the art for high-quality
   spectrogram inversion / time-stretching. Worth knowing about even
   though it isn't directly applicable to our ducking problem.
5. **Is there a Rust implementation we can lift?** — Answered: no
   open-source peak-locked Rust implementation exists. Five Rust PV
   crates were found, none implement Laroche-Dolson.

**On Topic B (per-bin PLL):**

6. **Do per-bin PLL banks appear in audio literature?** — Mostly
   answered: not as a named pattern. Closely related work exists in
   *adaptive notch filter banks* (one-IIR-per-frequency, frequency
   tracked by an inner PLL) and in the "frequency reassignment" /
   instantaneous-frequency-from-phase-derivative literature. A *bank*
   of per-FFT-bin PLLs is something we will largely have to design
   ourselves; the components (loop filter, lock detector) are well
   understood from communications DSP.
7. **What loop-filter design is appropriate at audio rates with FFT
   hop as the update period?** — Answered. Standard 2nd-order PI loop
   with parameters (ωₙ, ζ, K). Good open-source reference exists
   (liquid-dsp).
8. **What's a robust lock-loss criterion?** — Partially answered.
   The communications-DSP standard is **phase-error variance over a
   sliding window** for *steady-state* lock detection; for the
   *transient* "tear" we want, instantaneous phase-error magnitude +
   hysteresis is more musical. Both are tractable.
9. **SIMD-friendly PLL bank?** — Partially answered. No drop-in
   library exists; the 2nd-order PI loop maps cleanly to SoA SIMD
   (4–16 bins per AVX2/AVX-512 lane). We'll write this ourselves.
10. **How does per-bin PLL interact with PVX-unwrapped phase?** —
    Answered: the PLL only needs the per-hop *phase-error* signal
    (observed - expected). PVX unwrap helps because the error stays
    bounded and continuous; without it you have to handle ±π wraps
    inside the loop, which adds branches and reduces SIMD efficiency.
    PVX is therefore "free" infrastructure for the PLL bank.

---

## Topic A — PVX and peak-locked phase vocoders

### Key references

The most important first; everything below is freely accessible online.

- **Puckette, M.** (1995). *Phase-locked Vocoder.* IEEE ASSP Workshop
  on Applications of Signal Processing to Audio and Acoustics. 4
  pages. — The originator of *vertical* phase coherence ("loose
  locking"). Defines the trick of summing a target bin with negated
  weighted neighbours so the resulting complex sum has a phase
  dominated by the local peak. No explicit peak detection.
  https://msp.ucsd.edu/Publications/mohonk95.pdf
  https://users.iem.at/zmoelnig/publications/phaslock/
- **Laroche, J. & Dolson, M.** (1999). *Improved Phase Vocoder
  Time-Scale Modification of Audio.* IEEE Trans. Speech & Audio
  Processing 7(3):323–332. — The canonical "rigid" / "scaled"
  peak-locking paper. Defines explicit peak detection (local 4-
  neighbour max), region of influence (bounded by midpoint or local
  magnitude minimum between adjacent peaks), and two locking modes
  (identity vs scaled). 2× cheaper than per-bin PV at equal quality.
  https://www.ee.columbia.edu/~dpwe/papers/LaroD99-pvoc.pdf
  https://www.ece.uvic.ca/~peterd/48409/Dobson1999.pdf (mirror)
- **Laroche, J. & Dolson, M.** (1999). *New phase-vocoder techniques
  for pitch-shifting, harmonizing and other exotic effects.* WASPAA
  Proc. — Same authors, sibling paper, focuses on pitch shifting
  using the same peak-shifting machinery. Patented as US6549884B1
  (Creative Technology Ltd, expired). https://patents.google.com/patent/US6549884B1/en
- **Laroche, J. & Dolson, M.** (1997). *Phase-vocoder: about this
  phasiness business.* ICMC. — The "phasiness" diagnosis paper that
  motivated the 1999 fix.
- **Roebel, A.** (2003). *A new approach to transient processing in
  the phase vocoder.* DAFx-03. — The fix for "Laroche-Dolson breaks
  on transients." Roebel's idea: classify each spectral peak as
  attack-transient or steady-state using the centre-of-gravity (COG)
  of its short-time energy envelope; for transient peaks, *do not
  apply phase locking* (treat them as if PV were off) so the attack
  isn't smeared. https://hal.science/hal-01161124
  http://recherche.ircam.fr/anasyn/roebel/publications.html
- **Roebel, A.** (2003). *Transient detection and preservation in the
  phase vocoder.* ICMC. — Companion paper, more on the COG criterion.
  https://hal.science/hal-01161125
- **Karrer, T., Lee, E., & Borchers, J.** (2006). *PhaVoRIT: A Phase
  Vocoder for Real-Time Interactive Time-Stretching.* ICMC. — The
  closest in scope to what we want: real-time, integrates Laroche-
  Dolson + Roebel + a "loose" mode and is implementation-oriented.
  Still 2006; nothing newer comes close.
  https://hci.rwth-aachen.de/publications/karrer2006a.pdf
- **Liuni, M., Holighaus, N., & Dörfler, M.** (2016). *A Phase Vocoder
  based on Nonstationary Gabor Frames.* arXiv:1612.05156. — Uses
  *adaptive* time-frequency tilings (NSGF) and adaptive locking,
  estimating phase only at peaks. Reduces the per-bin work but adds
  significant infrastructure cost. https://arxiv.org/abs/1612.05156
- **Pruša, Z. & Holighaus, N.** (2017). *Phase Vocoder Done Right.*
  EUSIPCO. — Introduces **PGHI** (Phase Gradient Heap Integration);
  a non-iterative phase reconstruction method derived from the STFT
  phase–magnitude relation. *No peak picking.* No transient detection.
  No special case treatment. Best-quality time-stretch in the
  literature as of writing. https://arxiv.org/abs/2202.07382
  (extended journal version, 2022). Open-source reference in MATLAB
  and Python at https://ltfat.org/notes/050/
- **Pruša, Z., Balazs, P., & Søndergaard, P.** (2017). *A Noniterative
  Method for Reconstruction of Phase from STFT Magnitude.* IEEE/ACM
  TASLP. — The original PGHI paper. Real-time variant **RTPGHI**
  documented at http://ltfat.org/phaseret/doc/gabor/rtpghi.html
- **Master, A.** (2002). *Peak-Adaptive Phase Vocoder.* CCRMA / ICASSP.
  — Adaptive analysis-channel construction *around* time-varying peaks;
  recovers narrow-band AM/FM within a frame. More expensive than
  vanilla Laroche-Dolson but conceptually closest to what our
  brainstorm document calls "PVX."
  https://ccrma.stanford.edu/~jos/mus423h/Peak_Adaptive_Phase_Vocoder.html
- **DAFX 2nd ed.** (Zölzer, 2011), Chapter 7. — Textbook treatment,
  cites everything above.

**Negative results — don't waste time here:**

- The term "PVX" as an algorithm name does not appear in IEEE/ACM/
  arXiv literature. ProSoniq (and its successor Zynaptiq) don't
  publish algorithm details for MPEX/ZTX; their public material
  vaguely says "wavelets + neural networks" and stops there. The
  original brainstorm reference to PVX is best read as "the family of
  peak-locked phase vocoder techniques," not a specific cited paper.
- Patents adjacent to the area: US6549884B1 (Laroche/Dolson, expired
  2023) covers the *pitch-shifting* application of peak-shifting; the
  underlying *time-scaling* peak-lock from the 1999 IEEE TSAP paper
  is not patented (it's prior art to itself).

### Synthesis

The research arc from 1995 to 2017 is essentially a single conversation:

**1995 — Puckette ("loose locking").** Sums a bin with a small
negative weighted contribution from each adjacent bin (his Eq. 8):

```
Z[i, k] = Y[i, k] - μ·Y[i, k-1] - μ·Y[i, k+1]
```

with μ ≈ 0.5–1. Because phase of a complex sum tends to follow the
largest-magnitude summand, Z's phase is dominated by whichever of
{k-1, k, k+1} has the largest magnitude. The bin then *resynthesizes
with that "locked" phase*. There is no peak detection — every bin gets
this treatment, and the math automatically pulls non-peak bins toward
their loudest neighbour.

**1999 — Laroche & Dolson ("rigid locking").** They observe that
Puckette's loose locking still leaves residual phasiness when the
nearest peak is more than one bin away. So they:

1. **Detect peaks explicitly.** A bin `k` is a peak iff
   `|X[k]| > |X[k±1]|, |X[k±2]|`. (Laroche-Dolson use 4-neighbour
   greater-than, not 2-neighbour, to suppress noise-floor false
   positives.)
2. **Define a region of influence.** Two adjacent peaks at `k_p` and
   `k_{p+1}`: the boundary between their regions is either
   (a) the midpoint `(k_p + k_{p+1}) / 2`, or (b) the bin of *lowest*
   magnitude between them (more robust). Everything in `[k_p_lo,
   k_p_hi]` is "owned" by peak `k_p`.
3. **Compute the peak's new phase** via standard PV phase
   propagation (using deviation from expected advance):

   ```
   ω̂_k = (2π·k/N) + Δφ(k) / R     where R = hop size
   φ_new[k_p] = φ_prev[k_p] + ω̂_{k_p} · R_synth
   ```

4. **Lock the skirt.** Two flavours:
   - **Identity locking:**
     `φ_new[k] = φ_new[k_p] + (φ_old[k] - φ_old[k_p])`
     (preserve the analysis-time phase relationship exactly).
   - **Scaled locking:**
     `φ_new[k] = φ_new[k_p] + β · (φ_old[k] - φ_old[k_p])`
     where β is the time-scale ratio. This is *correct* under the
     model "the skirt rotates at the same rate as the peak when you
     stretch."

The "vertical alignment" formula in our brainstorm —
`new_phase[skirt] = new_phase[peak] + (old_phase[skirt] -
old_phase[peak])` — is *exactly* identity locking. So the brainstorm
is mathematically right; the literature just calls it that.

For ducking (our actual use case) the time-scale factor β = 1, so
identity locking = scaled locking = same formula. **For magnitude-
scaling work like ducking, identity locking is the canonical
choice.**

**2003 — Roebel ("transient preservation").** Laroche-Dolson breaks
on attack transients because at attack onset the spectral peak
hasn't fully formed yet — the magnitude profile is broad and noisy,
and locking the wide skirt to the half-formed peak phase smears the
attack. Roebel's fix: compute a per-peak *centre-of-gravity* of its
short-time energy distribution; if the COG is past a threshold (i.e.
the attack hasn't filled the analysis window), classify the peak as
"transient" and *bypass phase locking for that peak only*. Stationary
peaks in the same frame still get locked. This is a per-peak
classification — important to note because it means the skirt
classification machinery has to be per-peak too.

**2017 — Pruša & Holighaus ("PGHI").** This is a different beast
entirely. Instead of detecting peaks and treating them specially,
PGHI uses the analytical relation that **STFT phase gradients are
constrained by STFT magnitude gradients via a PDE-like relation
(specifically, that the divergence of the phase gradient equals the
log-magnitude gradient field, in continuous STFT)**. By estimating
the gradient field from magnitude alone and integrating it via a
priority-queue ("heap") traversal starting from local maxima, PGHI
reconstructs phase without ever picking peaks explicitly. Real-time
variant RTPGHI (1-frame delay, ~O(N log N) per hop) is documented
but not heavily benchmarked at our hop sizes.

**For our use case (per-bin gain shaping with phase preservation),
PGHI is overkill** — it's a *phase reconstruction from magnitude*
algorithm, designed for time-stretching where you have to invent
new phase anyway. We *have* the original phase. We just need to scale
magnitudes without breaking partials. Laroche-Dolson identity locking
is the right tool, and its overhead is marginal compared to PGHI.

**Modern best practice for time-stretching is PGHI / NSGF.** Modern
best practice for *gain-shaping with phase preservation* (us) is
still Laroche-Dolson + Roebel. Nothing has obsoleted it for that
task because nothing newer is enough cheaper.

### Implementation candidates (open-source)

No open-source Rust implementation includes peak-locking. The C++
options are limited and have small star counts.

**Rust (none implement peak locking — all are educational):**

- **pvoc-rs** (nwoeanhinnogaehr/pvoc-rs). 31 stars. GPL-3.0. Rust
  100%. Last release 2018-ish. Exposes a per-bin callback that gives
  you `&[Vec<Bin>]` per channel — perfect for *us* to layer
  Laroche-Dolson on top, but the library itself is naïve PV.
  https://github.com/nwoeanhinnogaehr/pvoc-rs
- **rocoder** (ajyoon/rocoder). 15 stars. CC0. Rust 100%. Self-
  describes as "fairly naive, probably not quite correct." Adapted
  from Paulstretch. Kernel-based programming model. Useful for
  *experimental Paulstretch-style smearing* but doesn't help us with
  peak locking. https://github.com/ajyoon/rocoder
- **phase_vocoder** (jneem/phase_vocoder). 2 stars. MIT. "very
  immature." Skip.
- **TheDevelo/phase-vocoder.** 4 stars. CLAP/VST3, nih-plug. License
  unspecified. The most architecturally similar to us (also a CLAP
  plugin in Rust), but no peak detection in the source.
  https://github.com/TheDevelo/phase-vocoder
- **pvoc-plugins** (nwoeanhinnogaehr/pvoc-plugins). LADSPA suite
  built on pvoc-rs.

**C++ (some implement peak locking):**

- **ybdarrenwang/PhaseVocoder.** 21 stars. Implements Laroche-Dolson
  scaled phase locking explicitly (`--phaseLock` CLI flag). Cites
  the 1999 paper directly. C++ 95%. Useful as a *reference algorithm
  implementation* — port it to Rust.
  https://github.com/ybdarrenwang/PhaseVocoder
- **stekyne/PhaseVocoder.** Educational, JUCE, no peak locking but
  active maintenance. https://github.com/stekyne/PhaseVocoder
- **Signalsmith-Audio/signalsmith-stretch.** 468 stars. MIT. C++.
  Spectral processing for pitch shifting; uses a magnitude
  redistribution scheme rather than peak-lock-style. Worth reading
  for *general spectral architecture in C++* but not directly
  applicable.
  https://github.com/Signalsmith-Audio/signalsmith-stretch
- **olvb/phaze.** Web Audio AudioWorklet; phase vocoder pitch shift
  in JS. No peak locking. https://github.com/olvb/phaze

**Algorithm-grade reference implementations (not for porting; for
verification):**

- **MATLAB**: `phaseret` (LTFAT toolbox) implements both PGHI and
  RTPGHI with full source. https://github.com/ltfat/phaseret
- **Python**: `tifresi` (Marafioti) implements PGHI in PyTorch with
  GPU acceleration. https://github.com/andimarafioti/tifresi
- **Csound**: `pvstanal` and the PVS opcode family.
  https://csound.com/docs/manual/pvstanal.html
- **SuperCollider**: `TPV` (Tracking Phase Vocoder, McAulay-Quatieri)
  in `sc3-plugins`. https://doc.sccode.org/Classes/TPV.html
- **Essentia**: `PeakDetection` and `SpectralPeaks` algorithms,
  AGPLv3. The peak detection logic is exactly what we need to port.
  https://github.com/MTG/essentia/blob/master/src/algorithms/standard/peakdetection.cpp

### Recommendations for our Rust implementation

**Phase 1 — port Laroche-Dolson 1999 identity locking (1 PR):**

1. **Write a `peaks::detect` module** in `src/dsp/`:
   - Local 4-neighbour magnitude max (greater than ±1 and ±2 in dB).
   - Optional magnitude floor (e.g. `peak_db > local_noise_db + 6`)
     to suppress noise-floor false positives. Local noise = median
     of magnitude in a ±32-bin window.
   - Output `Vec<PeakInfo>` with `bin: u16, magnitude: f32, skirt_lo:
     u16, skirt_hi: u16`.
   - Skirt boundary: take the magnitude-minimum bin between this peak
     and each neighbour. (Lower jitter than midpoint per the
     literature.) Cap skirt half-width at e.g. 12 bins regardless to
     avoid over-greedy regions on isolated peaks.
   - Pre-allocated peak buffer in Pipeline. `MAX_PEAKS = 128` bins'
     worth of structure (~3 KB) is fine.
2. **Write a `phase::unwrap` helper** that computes
   `unwrapped_phase[bin] = unwrapped_phase_prev[bin] + expected_adv +
   wrap(observed - prev - expected, ±π)` per the brainstorm. Per-
   channel state in Pipeline (~64 KB for two channels at 8193 bins).
3. **Expose `ctx.peaks` and `ctx.unwrapped_phase` in
   `ModuleContext`.** Set both to `None` if PVX is disabled.
4. **Update Dynamics module** to consume the peaks. For each peak,
   compute the gain-reduction once (using the peak's magnitude),
   apply that gain to all bins in `[skirt_lo, skirt_hi]`. Outside
   any skirt: fall through to per-bin behaviour.
5. **Apply identity locking on the skirt**:
   `new_phase[bin] = new_phase[peak] + (old_phase[bin] -
   old_phase[peak])`. Note that for ducking this is *only* useful if
   we're storing intermediate phase; for pure magnitude scaling the
   complex bin's phase doesn't need to change. The "phase locking"
   matters for *modules that modify phase* (PhaseSmear, Modulate),
   not for pure gain-modules.

**Important nuance for our codebase:** in the brainstorm, the value
of PVX for Dynamics is "scale the peak + skirt as a single unit"
(magnitude grouping), not "rotate the skirt phases" (the locking
math). The phase-locking math becomes important for *PhaseSmear,
Freeze, Past Stretch, Harmony Inharmonic*, where we *are* changing
phases. Phase 1 of integration should focus on the magnitude-grouping
part for Dynamics; phase locking *math* lands when those other
modules need it.

**Phase 2 — Roebel transient classification (1 PR, optional):**

For Dynamics ducking specifically, transient classification is less
important than for time-stretching. A duck on a kick drum *should*
respond per-bin, not per-skirt — we want the kick's broad-band hit
to be addressed wholesale. Roebel-classify peaks: if a peak is
"transient" (its COG < threshold), fall back to per-bin behaviour
within its would-be skirt. This is a 2-line per-peak addition.

Skip Roebel for the first iteration. Add when we have audio examples
showing peak-locked ducking sounds bad on transients.

**Phase 3 — defer PGHI / NSGF entirely.** They're for time-stretching.
We don't time-stretch. Re-evaluate only if a future module needs
phase reconstruction from magnitude (e.g. a "magnitude paint" mode
where the user draws a spectrum and we synthesize from it).

**Don't attempt:**

- Peak-Adaptive PV (Master 2002): it allocates per-frame channels,
  which is ill-suited to our fixed FFT size + lock-free
  triple-buffer architecture.
- McAulay-Quatieri sinusoidal modeling: it tracks partials *across*
  frames, which is real-time but expensive (O(P²) matching) and
  its "birth/death" of tracks doesn't map naturally to per-hop
  spectral processing.

**Risks specific to our codebase:**

- **Allocation in the audio thread**: peak detection naturally
  involves a `Vec<PeakInfo>`, which we must pre-allocate at
  `Pipeline::new()` and `clear() + push()` rather than `new()`. Use
  a fixed-capacity `arrayvec::ArrayVec` or similar.
- **Lock-free curve cache invariance**: the curve cache is per-bin.
  When we apply a peak's gain to a skirt range, we're not touching
  the curve cache, we're applying gain *after* curve sampling.
  Order: (curve sample) → (peak-aware gain) → (write back). Verify
  the calibration probes still see equivalent outputs when PVX is
  off (Phase 1's contract).

---

## Topic B — Per-bin PLL bank for "tear" effect

### Key references

- **Best, R.E.** (2007). *Phase-Locked Loops: Design, Simulation,
  and Applications*, 6th ed. McGraw-Hill. — The standard reference
  for PLL design. Chapter on 2nd-order PI loops gives the
  (ωₙ, ζ, K) parameterisation we'd use.
- **Gardner, F.M.** (2005). *Phaselock Techniques*, 3rd ed. Wiley.
  — Communications-DSP standard. Lock-loss criteria (especially the
  *cycle slip* analysis) are directly relevant to what we want
  (intentional, audible cycle slips = "tear").
- **liquid-dsp** by Joseph Gaeddert. — Practical reference C
  implementation of 2nd-order PI PLL with `iirdes_pll_active_lag()`
  generating filter coefficients from (ωₙ, ζ, K). 2.2k stars, MIT,
  active (last v1.7.0, Feb 2025). Has SIMD enabled by default. The
  PLL is *not* per-bin / banked — it's a single-channel SDR PLL —
  but the inner-loop math is exactly transferable.
  https://github.com/jgaeddert/liquid-dsp
  Tutorial: https://liquidsdr.org/blog/pll-howto/
- **Brown, J. & Puckette, M.** (1993). *A high resolution fundamental
  frequency determination based on phase changes of the Fourier
  transform.* JASA 94(2):662–667. — The phase-derivative IF
  formulation we already use; relevant as the "expected" phase
  advance per bin per hop.
- **Boashash, B.** (1992). *Estimating and Interpreting the
  Instantaneous Frequency of a Signal — Parts 1 & 2.* Proc. IEEE
  80(4):520–568. — Classic reference on instantaneous frequency.
  https://www.math.ucdavis.edu/~saito/data/sonar/boashash1.pdf
- **Niedźwiecki, M. & Meller, M.** (various, 2008–2014). *Adaptive
  notch filters with PLL-based frequency tracking.* IEEE Trans. SP.
  — A 2nd-order adaptive notch with embedded PLL is functionally
  equivalent to what we want per bin: tracks a sinusoid's frequency,
  loses lock when the input deviates outside the loop bandwidth,
  re-locks when input returns. Their analysis transfers.
- **Sithamparanathan, K.** (2007). *A Software Phase-Locked Loop
  from Theory to Practice.* — Free PDF tutorial, audio-rate
  examples. https://opus.lib.uts.edu.au/bitstream/10453/19598/1/9_sithamparanathan.pdf

### Synthesis

A PLL is a feedback loop that drives a local NCO (numerically
controlled oscillator) such that its phase tracks an input
reference's phase. In the frequency domain — our setting — the
"reference" for bin `k` is the phase of the kth FFT bin per hop, and
the "NCO" is the *predicted* phase of that bin assuming continuous
sinusoidal evolution. The loop's job is to update the predicted
frequency (= NCO rate) so the prediction matches reality, and to
report when reality and prediction diverge by more than the loop
bandwidth allows.

**Loop topology — 2nd-order PI loop (the right choice).**

For each bin `k`, store two state variables:
- `pll_phase[k]` — the NCO's current phase (radians)
- `pll_freq[k]` — the NCO's current frequency (radians per hop)

Per hop:
```
predicted_phase    = pll_phase[k]
observed_phase     = arg(X[hop, k])
phase_error        = wrap(observed_phase - predicted_phase, ±π)
pll_freq[k]       += beta  * phase_error
pll_phase[k]      += pll_freq[k] + alpha * phase_error
```

`alpha` and `beta` are loop gains derived from desired natural
frequency `ωₙ` and damping `ζ`:
```
alpha = 2 * ζ * ωₙ * T_hop
beta  = (ωₙ * T_hop)²
```
where `T_hop = hop_size / sample_rate` — but in our PLL update unit
the time step is "1 hop", so just use ωₙ in *cycles per hop*. A
typical choice for audio: `ωₙ_hop = 0.05` (loop tracks within ~20
hops), `ζ = 0.707` (Butterworth-flat). That gives `alpha ≈ 0.0707`,
`beta ≈ 0.0025`. These are tunable per the "lock speed" curve in
the Modulate spec.

**Loop bandwidth scaling per bin.** Yes — higher bins should have
*proportionally* wider loop bandwidth in frequency, but the same
ωₙ in *cycles-per-hop* is reasonable because phase advances by
`2π · k · hop / N` per hop regardless of bin index. The phase-error
quantity is in radians-per-hop, naturally normalised. **No per-bin
bandwidth scaling needed** for the basic loop. Optionally, the
"lock speed" curve can scale ωₙ per bin if the user wants e.g.
faster tracking in the upper octaves.

**Bins below 100 Hz at hop=128, sample_rate=44100.** A 100 Hz
sinusoid has period 441 samples = 3.5 hops. The PLL needs a few
hops worth of evidence per cycle, so its tracking time constant for
sub-100-Hz partials is naturally O(10 hops). This is fine for
"tear" — slow bass partials don't tear meaningfully, the effect is
musical at higher frequencies anyway. Recommend: don't enable PLL
tracking below e.g. bin 16 (~344 Hz at default hop). Saves
arithmetic and avoids tracking artefacts.

**Lock-loss detection.** Two complementary signals:

1. **Instantaneous phase-error magnitude with hysteresis.**
   `|phase_error| > threshold_lose_lock` (say π/2) → enter "torn"
   state. `|phase_error| < threshold_relock` (say π/8) for N
   consecutive hops → return to "locked" state. This is the audibly
   *punchy* detector — fires on the same hop as the transient.
2. **Sliding-window phase-error variance.** Maintain
   `var = (1-α)·var + α·phase_error²` (one-pole). Compare against
   threshold. Smoother, but adds latency (the variance only rises
   over several hops). Use as a *secondary* gate: tear only when
   *both* instantaneous and variance exceed thresholds, to avoid
   tearing on single-hop noise spikes.

For the "tear" character, **instantaneous + hysteresis is the right
default**. Tying the tear sample-tight to the transient is what
makes it musical, not syrupy.

**Re-lock behaviour.** When the input stabilises, we want the loop
to converge in 5–20 hops without overshoot. With `ζ = 0.707` the
step response is critically damped; settling time ≈ 4/(ζ·ωₙ) hops.
At ωₙ = 0.05, settling ≈ 113 hops, which is too slow. Bump ωₙ to
~0.2 for faster recovery, accepting more noise in lock. Audio-tune.
A "ringy" re-lock (ωₙ low, ζ low) is *also* musically interesting —
the loop overshoots and oscillates around the new frequency for
a few hops, which sounds like a mini-chorus settling.

**PVX unwrapped phase as PLL input — yes, helpful.** Without unwrap,
the loop has to handle ±π wrap inside its update step (extra branch
per bin, breaks SIMD). With unwrap, `phase_error = unwrapped_phase
- predicted_phase`, no wrap handling, branch-free, SIMD-perfect.
Even if PVX is "off" globally, the PLL bank should compute its own
local unwrap using the same formula (it's literally three FLOPs per
bin: subtract, fma, add). Sharing PVX's unwrap is a small win, not
a load-bearing dependency.

**Stereo behaviour.** With Independent stereo, two PLL banks. If
the same input glide hits both channels, they tear in *near* sync
because their phase-error responses are identical given identical
input. There's no random component. The mono-sum will be clean
*if* the input was mono-correlated; if stereo information differs,
mono-sum has the same tear in each channel summed, which preserves
the tear character.

**SIMD layout.** The PI loop is 4 FLOPs per bin per hop:
```
phase_error = unwrapped[k] - predicted[k]
pll_freq[k] += beta * phase_error
pll_phase[k] += pll_freq[k] + alpha * phase_error
predicted[k] = pll_phase[k]
```
SoA layout (`Vec<f32>` for `pll_phase`, separate `Vec<f32>` for
`pll_freq`, `Vec<f32>` for `predicted_phase`) maps directly to
AVX2 (8 f32 lanes) or AVX-512 (16 f32 lanes). Per-channel, 8193
bins × 4 FLOPs = 32k FLOPs per hop. At 16 lanes / instruction =
~2k vector instructions / hop. Negligible.

**Lock detector** adds branch (set/unset "torn" bit per bin). Use
SIMD comparison + bitmask packing into a `Vec<u32>` (256 bins per
u32 word — 8193 bins = 33 words, ~130 bytes). Branch-free.

**The "tear" output** — what does the module actually emit in the
torn state? Possibilities from the brainstorm:
1. Pass through the input unmodified (PLL silently re-tracks).
2. Output the *prediction* (what the locked PLL *would* have output)
   — sounds like the partial "freezes" briefly.
3. Output chaotic sub-octave noise — emit `cos(pll_phase[k] / 2 +
   chaotic_term)` for the magnitude of the bin. This is the
   brainstorm's "chaotic sub-octave phase noise" idea.

Option 3 is the most extreme; option 2 is the most musical default.
The Modulate spec's RATE curve can interpolate between them.

### Implementation candidates

**Direct ports / references:**

- **liquid-dsp PLL example.** Single-channel reference. Port the
  inner loop, replace its scalar math with our SoA SIMD. License
  MIT.
  https://github.com/jgaeddert/liquid-dsp/blob/master/examples/pll_example.c
- **ZipCPU/dpll.** A collection of PLL projects, mostly Verilog/HDL
  oriented but with C reference models. Less directly useful but
  good cross-validation. https://github.com/ZipCPU/dpll
- **csdr.** Has SIMD-accelerated PLL in software-defined radio
  context. https://github.com/ha7ilm/csdr — useful for *patterns*
  (NEON intrinsics, etc.) but not direct reuse.

**Adaptive notch / resonator banks (closer in spirit to per-bin):**

- **alexandrefrancois/Oscillators** — Resonator bank in C++ with
  SIMD via Apple Accelerate. Tracks individual frequencies adaptively.
  Closest existing implementation to what we want, modulo PLL
  topology (he uses resonator IIRs). Worth reading for the
  vectorisation pattern.
  https://github.com/alexandrefrancois/Oscillators

**No suitable Rust library exists. We will write this from scratch.**

The PLL bank will live in either:
- `src/dsp/pll.rs` — new file. Holds `PllBank` struct with SoA
  state arrays. Public interface: `update(unwrapped_phase: &[f32],
  output_predicted: &mut [f32], output_torn_mask: &mut [u32], cfg:
  &PllConfig)`.
- Used by ModulateModule's `PllTear` mode in `src/dsp/modules/
  modulate.rs` (when that lands).

### Recommendations

**Phase 1 — kernel + tests (1 PR, ~400 LOC):**

1. `src/dsp/pll.rs`: the `PllBank` struct + scalar update function.
   No SIMD yet.
2. Tests in `tests/pll_kernel.rs`:
   - **Lock-acquire test:** synthesize a pure sinusoid at exactly
     bin 100 (so observed phase advances by `2π·100·hop/N` per hop).
     Initialize PLL with wrong frequency. Assert convergence within
     N hops to within `ε` radians.
   - **Lock-loss test:** synthesize a frequency glide that exceeds
     loop bandwidth. Assert lock-loss flag fires when phase error
     exceeds threshold, clears when input stabilises.
   - **Re-lock test:** as above, then stable input — assert flag
     clears within M hops.
   - **Stability test:** white noise input, assert PLL state stays
     bounded (no NaN/Inf) over 10 000 hops.
3. **Bench:** scalar baseline before SIMD. Get a number.

**Phase 2 — SIMD (1 PR):**

Replace inner loop with `core::simd` or `std::simd` (both work in
nightly; we already use realfft so we're nightly-friendly). Benchmark
SIMD vs scalar — expect 4–8× on AVX2.

**Phase 3 — Modulate module integration (later, gated on the module
landing):**

Plug `PllBank` into ModulateModule's PllTear mode. Pre-allocate one
`PllBank` per channel for Independent stereo.

**Tunable parameters surfaced as curves:**

| Curve    | PLL parameter        | Range                 |
|----------|----------------------|-----------------------|
| RATE     | `ωₙ` (loop bw)       | 0.005 … 0.5 cyc/hop  |
| THRESHOLD| `lose_lock_thresh`   | π/16 … π            |
| AMOUNT   | wet/dry of torn output| 0 … 100%            |
| REACH    | bin range affected    | 16 … all bins       |

**Don't attempt:**

- Per-channel coupling (stereo PLL "share lock state"). Independent
  is simpler and the brainstorm's stereo subtlety is a minor concern.
- Higher-order loops (3rd+). 2nd-order PI is enough for monotonic
  glides; the input doesn't accelerate fast enough to need higher
  order.

---

## Cross-topic synthesis

PVX (peak detection + unwrap) and per-bin PLL are *complementary*
infrastructure that share two pieces of state and one update pass:

1. **Unwrapped phase storage.** PVX needs it for skirt phase
   manipulation; PLL needs it as the input "observed phase." If both
   are enabled, compute *once* in `Pipeline::process()` and pass via
   `ModuleContext`. Storage: `Vec<Vec<f32>>` per channel × bin.
2. **Peak set.** PVX uses it for skirt definition. The PLL bank
   *could* use it to gate which bins to track (only track peak bins
   + skirts, set everything else to "untracked" / pass-through),
   which would cut PLL workload from 8193 bins to ~100. **Recommend
   making this an opt-in optimisation in PllBank**: a `track_mask:
   &[u32]` argument that selects which bins are active. When a peak
   appears in a bin that wasn't tracked last hop, snap-init the PLL
   state to the observed values (no convergence delay).

So **the same peak detector serves both topics**, and the same
unwrap pass serves both. The "global infra" PR for PVX should:

- Compute unwrapped phase per channel per hop.
- Compute peak set per channel per hop (with N-peak cap).
- Expose both via `ModuleContext`.

After that single PR, both Topic A (Dynamics, PhaseSmear, Freeze,
…) and Topic B (Modulate's PllTear) can integrate independently.

**Order of work:**

1. PVX Phase 1 (unwrap + peak detection) — 1 PR. Unblocks every
   downstream consumer with no behaviour change.
2. Dynamics PVX integration — 1 PR. First *user* of PVX. Validates
   the API.
3. PLL kernel + tests — 1 PR. Independent of PVX.
4. PLL SIMD — 1 PR.
5. PhaseSmear PVX integration — 1 PR.
6. Freeze PVX integration — 1 PR.
7. Modulate module (carries PLL Tear, Phase Phaser, Gravity Phaser
   as sub-modes; consumes PVX from day one) — bigger PR, depends
   on Modulate spec being unblocked.

**One concrete code-level suggestion**: make the `PeakInfo` struct
include enough state for *both* uses:

```rust
pub struct PeakInfo {
    pub bin: u16,
    pub magnitude: f32,
    pub skirt_lo: u16,
    pub skirt_hi: u16,
    pub if_hz: f32,             // instantaneous frequency from unwrap derivative
    pub is_transient: bool,     // Roebel COG classification (Phase 2)
}
```

`if_hz` falls out for free from the unwrap pass and is what the PLL
should be initialized to when a new peak appears. Also useful for
Modulate's FM Network and Harmony's pitch tracking (cross-link to
research prompt #2).

---

## Open questions for next research round

1. **Roebel COG threshold tuning.** The 2003 paper gives a value but
   it's window-dependent. We use Hann-² overlap-add at hop=N/4 — what
   COG threshold gives the best transient/steady classification on
   our exact STFT? Needs measurement on actual material (drums,
   piano, vocals).
2. **Skirt cap policy.** Hard cap (e.g. ±12 bins) vs purely
   magnitude-derived. Magnitude-derived is more "correct" per
   Laroche-Dolson but can make peaks own *huge* skirts on isolated
   sinusoids, which de-correlates the rest of the spectrum from
   user expectations. Need A/B audio examples.
3. **PGHI for the Freeze module specifically.** Freeze needs to
   re-synthesize phase from the frozen magnitude across many hops.
   PGHI is *exactly* the right tool here — its "no peak picking"
   trade-off doesn't matter because Freeze is a single-source
   spectrum (no multi-peak coherence to preserve). RTPGHI's 1-hop
   delay is acceptable at typical Freeze use cases. **Worth a
   prototype** to compare against the current "store and replay
   phase" approach. Low-priority for now but file this insight.
4. **PLL "tear" sound design.** The three output options (input,
   prediction, chaotic) need audio examples to choose a sensible
   default and a usable RATE curve mapping. Probably needs Kim's
   ear, not more research.
5. **Peak stability between hops.** Peak indices can shift by a bin
   between hops as a partial drifts. The skirt membership therefore
   jitters, which can audibly modulate the peak group. Mitigations:
   (a) **temporal smoothing of peak set** — a peak is only "active"
   if it appeared in the last K hops; (b) **hysteresis on skirt
   boundaries** — once a bin enters a skirt it takes M hops without
   meeting the criterion to leave. Both are easy to implement but
   need empirical tuning.
6. **Cross-channel peak coherence in Linked stereo.** Linked stereo
   shares one STFT call but two phases. Should the peak set be
   computed on `|L| + |R|` (mono-sum magnitude) or per-channel?
   Mono-sum is more stable; per-channel preserves stereo
   information. Spec.
7. **Real-world MPEX/ZTX.** ProSoniq's actual algorithms appear to
   be wavelet+NN, *not* peak-locked PV. If we ever want to chase
   their quality bar, that's a different research thread (probably
   not worth it given the patent landscape and complexity).
8. **PV Done Right (PGHI) for Past Stretch.** For the deferred Past
   Stretch mode (variable-rate playback from history buffer), PGHI
   is meaningfully better than per-bin phase rotation. File for
   that module's research round.

---

## Appendix: useful tables

### Algorithm capability matrix

| Algorithm           | Peak detect? | Phase lock? | Transient handling | RT-friendly | Quality | CPU/hop |
|---------------------|:------------:|:-----------:|:------------------:|:-----------:|:-------:|:-------:|
| Vanilla phase voc   | no           | no          | no (smear)         | yes         | low     | 1.0×    |
| Puckette 1995 loose | implicit     | yes (sum)   | no                 | yes         | mid     | 1.1×    |
| Laroche-Dolson 1999 | yes (4-nbr)  | yes (id/sc) | no                 | yes         | high    | 0.5×*   |
| L-D + Roebel 2003   | yes          | yes         | yes (COG)          | yes         | very hi | 0.6×    |
| McAulay-Quatieri    | yes (parab)  | tracks      | tracks             | hard        | very hi | 2-5×    |
| PVOC NSGF 2016      | yes (peak)   | adaptive    | yes                | yes         | very hi | 1.5×    |
| PGHI / RTPGHI 2017  | no           | implicit    | implicit           | yes (1-hop) | best    | 1.2×    |
| Peak-Adaptive 2002  | yes          | yes         | yes (AM/FM)        | hard        | very hi | 3×      |

*Laroche-Dolson is *cheaper* than vanilla because it skips per-bin
phase math, only doing it at peaks (~50–200 of 8193).

### Suggested PLL parameter ranges

| Parameter         | Recommended | Range            | Audible effect |
|-------------------|------------|------------------|----------------|
| `ωₙ` (cyc/hop)    | 0.05       | 0.005 – 0.5      | Lock speed; high = brittle, low = chorusy |
| `ζ` damping       | 0.707      | 0.3 – 1.0        | Re-lock overshoot (low = ringy, hi = damped) |
| `lose_lock` thresh| π/2        | π/8 – π          | Tear sensitivity |
| `relock` thresh   | π/8        | π/64 – π/4       | Tear release smoothness |
| `relock_hops`     | 4          | 1 – 16           | Hysteresis to prevent flutter |
| Min tracked bin   | 16         | 0 – 64           | Below = no PLL (saves CPU) |
| Max tracked bin   | NUM_BINS-1 | 64 – NUM_BINS-1  | Above = no PLL |

### Sources cited (consolidated for easy clicking)

- https://msp.ucsd.edu/Publications/mohonk95.pdf — Puckette 1995
- https://users.iem.at/zmoelnig/publications/phaslock/ — Puckette 1995 (mirror + commentary)
- https://www.ee.columbia.edu/~dpwe/papers/LaroD99-pvoc.pdf — Laroche-Dolson 1999 WASPAA
- https://www.ece.uvic.ca/~peterd/48409/Dobson1999.pdf — Laroche-Dolson 1999 (mirror)
- https://patents.google.com/patent/US6549884B1/en — US patent (expired)
- https://hal.science/hal-01161124 — Roebel 2003 transient processing
- https://hal.science/hal-01161125 — Roebel 2003 transient detection
- https://hci.rwth-aachen.de/publications/karrer2006a.pdf — PhaVoRIT
- https://arxiv.org/abs/1612.05156 — NSGF phase vocoder
- https://arxiv.org/abs/2202.07382 — Phase Vocoder Done Right
- https://ltfat.org/notes/050/ — PGHI implementation notes
- https://ltfat.org/notes/ltfatnote043.pdf — RTPGHI paper
- http://ltfat.org/phaseret/doc/gabor/rtpghi.html — RTPGHI doc
- https://ccrma.stanford.edu/~jos/mus423h/Peak_Adaptive_Phase_Vocoder.html — Master 2002
- https://ccrma.stanford.edu/~jos/parshl/Peak_Detection_Steps_3.html — Smith parabolic peak
- https://ccrma.stanford.edu/~jos/sasp/ — Smith SASP textbook
- https://github.com/MTG/essentia/blob/master/src/algorithms/standard/peakdetection.cpp — Essentia peak detection (AGPL)
- https://github.com/ybdarrenwang/PhaseVocoder — C++ Laroche-Dolson reference
- https://github.com/nwoeanhinnogaehr/pvoc-rs — Rust PV crate (no peak lock)
- https://github.com/Signalsmith-Audio/signalsmith-stretch — pitch shift in C++
- https://github.com/jgaeddert/liquid-dsp — PLL reference (MIT)
- https://liquidsdr.org/blog/pll-howto/ — 2nd-order PI PLL tutorial
- http://blogs.zynaptiq.com/bernsee/time-pitch-overview/ — Bernsee overview
- http://blogs.zynaptiq.com/bernsee/pitch-shifting-using-the-ft/ — Bernsee tutorial
- https://github.com/alexandrefrancois/Oscillators — SIMD resonator bank (Swift+C++)
- https://github.com/spluta/PV_Control — SuperCollider PV plugin (C++)
- https://www.dafx.de/paper-archive/ — DAFx paper archive
