# Modulate Module — Audit

**Existing spec:** `docs/superpowers/specs/2026-04-21-modulate-module.md`
**Status:** DEFERRED, depends on BinPhysics + Instantaneous Frequency
infrastructure.
**Source brainstorm:** PVX section, "Survival of the free" section
(Phase Phaser, FM network, RM matrix, Gravity Phaser, FM Replicator),
ideas #12 (PLL Tearing), #14 (Bin Swapping), #16 (Ground Loop), #17
(Diode Bridge RM), Kim's annotations across the file.

## What the spec covers

Six sub-effects: Phase Phaser, Gravity Phaser, FM Network — Partial
Web, RM/FM Matrix Bin-to-Sidechain, PLL Tear, Bin Swapper. Curves:
AMOUNT, REACH, RATE, THRESHOLD, MIX (5). Reads/writes `phase_momentum`
on Gravity Phaser and PLL Tear; rest are stateless aside from internal
PLL state arrays.

## Brainstorm cross-reference

| Brainstorm idea | Kim's note | In spec? | Notes |
|---|---|---|---|
| Phase Phaser (Survival #1) | — | yes | Spec covers it. |
| Gravity Phaser (Survival #6) | — | yes | Spec covers, but only as *phase* randomizer, not the magnitude/frequency well variant from Cat 4 #13. |
| Switch to turn wells into repulsion | — | partial | Spec mentions Buchla-style triggers but no explicit Repel mode toggle. Gap. |
| Sidechain-positioned wells | "graphs set width, amplitude, strength and reach" | partial | Spec mentions sidechain peak-detection for node positions; the four-curve scheme isn't enumerated. Gap. |
| FM network of loudest partials (Survival #4) | — | yes | Spec covers. |
| FM Replicator (Survival, separate) | — | no | A 16-op FM synth re-synthesizing the spectrum. **This is in Harmony** per `15-harmony.md` § Re-Synthesis. Mention here for cross-link. |
| RM matrix (Survival #5) | "need to think about this" | yes | Spec covers as bin-to-sidechain RM. |
| Diode-Bridge RM (Cat 5 #17) | "wonder if a dedicated RM/FM/PM module is needed? Modulate" | no | **Modulate confirmed as the home.** Diode-bridge is a *variant* of RM with mismatch tolerance — gap, see § (a). |
| Ground Loop 60Hz Hum (Cat 4 #16) | "kind of bland, maybe combine with the subharmonic generator" | no | Gap. Consider sub-effect or Harmony co-feature, see § (b). |
| Lag for partial envelopes | "sidechain tracks speed of change of input which constricts partial amplitude rate of change, not amplitude" | no | Gap. Could live here or in Dynamics. See § (c). |
| Bin delay with directional drift | — | no | Belongs in **Past/Future** as a delay-line effect, not Modulate. Cross-link, do not duplicate. |
| PLL Tear (Cat 3 #12) | "chaotic state changes oscillating around two distant stages are fun" | yes | Spec covers; PLPV integration improves it, see § (d). |
| Bin Swapping (Cat 4 #14) | — | yes | Spec covers as Bin Swapper. |
| Negative Gravity (Cat 4 #13) | "this could be a 'kinetics' global function" | partial | Treated under Kinetics (`12-kinetics.md`); Modulate's Gravity Phaser is the *phase* analogue. Cross-link. |
| Amplitude-dependent crossover for phase change | "should enable the normal dynamic control for phase change too" — about smear/freeze | no | Spec doesn't expose amp-gating on Phase Phaser. Gap. See § (e). |
| PVX phase unwrapping/locking *(brainstorm name; we call it PLPV)* | full PVX section | implicit | Spec's PLL Tear references "phase from previous frame" but doesn't say *unwrapped*. Cross-cutting work, see `20-plpv-phase-cross-cutting.md` and § (d). |

## Gap details

### a) Diode-Bridge Ring Mod — analog-style RM with mismatch tolerance

**Concept (per Cat 5 #17).** Real diode-bridge ring modulators leak the
carrier through when the four diodes are not perfectly matched. The
graph sets the mismatch percentage — quiet inputs allow full carrier
bleed (broken-radio leakage), loud inputs suppress it (proper RM
behaviour).

**Sub-effect proposal: `Diode RM` mode**

- Inputs: bin magnitude (input), sidechain bin magnitude (carrier).
- Output mag = `|input * carrier|` (normal RM) blended with `mismatch *
  carrier` (leak) where `mismatch = clamp(1 - input_amp/threshold)`.
- Phase: standard RM phase = input_phase + carrier_phase (mod 2π);
  leak path passes carrier phase through unchanged.
- Curves: AMOUNT (RM depth), REACH (carrier band falloff), THRESHOLD
  (input level above which the diode "shuts" the leak), MIX.
- CPU: light. Same per-bin multiply as the existing RM matrix mode.

Distinct from the existing RM/FM Matrix mode because:

- Existing RM matrix is mathematically clean (point-wise multiply, no
  leak).
- Diode RM has the *amplitude-dependent leak path*, which is the
  audible signature of a real Buchla 285e or Moog ring mod.

**Recommendation:** ship as a second mode within the same module, not a
new module. The state delta is just one extra input curve and a
mismatch coefficient.

### b) Ground Loop Intermodulation — 60 Hz hum tied to sag

**Concept (per Cat 4 #16).** A 60 Hz sine (or 50 Hz on EU presets) and
its first 4 harmonics are mixed in *as bin energy*, but the energy
amount is gated by the global RMS — louder programme material drives
more hum *because the power supply is sagging*. Kim's annotation: "kind
of bland, maybe combine with the subharmonic generator."

**Two routes for this:**

1. **As a Modulate sub-effect.** Inject 60 Hz at bin = round(60 *
   fft_size / sample_rate), modulate amplitude by the sum of the
   programme RMS over the last N hops. Cross-multiply (intermod) the
   60 Hz energy with the bins above 200 Hz to get throbbing dirty
   beats. CPU: trivial.
2. **As a Harmony Undertone Generator co-feature.** When the Undertone
   Generator is active, also generate a 60 Hz "ground" undertone whose
   amplitude tracks the highest sustained partial. This is what Kim's
   "combine with the subharmonic generator" hint pointed to.

**Recommendation:** ship route 1 as a Modulate `Ground Loop` mode. It
is conceptually a modulator (it modulates spectrum bins via a mains-
like reference) and lives among the other "circuit reality" effects.
Route 2 is a Harmony module concern, not Modulate.

- Mode: `Ground Loop`
- Curves: AMOUNT (hum level), REACH (number of harmonics injected),
  RATE (mains frequency, defaults 50/60 Hz), THRESHOLD (sag
  sensitivity), MIX.
- CPU: light. One sine-table read per bin per harmonic.

### c) Lag — sidechain-controlled rate-of-change limiter

**Concept (per Kim).** "Lag for partial envelopes, sidechain tracks
speed of change of input which constricts partial amplitude rate of
change, not amplitude."

This is a slew limiter modulated by sidechain transient detection. When
the sidechain has a fast-moving signal (drums, plucked strings),
partial amplitudes are heavily slew-limited; when the sidechain is calm
(pads), partials respond freely.

**Where does this live?**

- Could live in **Dynamics** as a slew-rate ratio control — but
  Dynamics is already implemented and adding sidechain-modulated slew
  is a non-trivial extension.
- Could live in **Modulate** as a sub-effect: the *modulation* is
  applied to the slew rate, not directly to the amplitude.
- Could live in **Life** under Stiction — except Stiction is a static
  threshold, not a rate-derived constraint.

**Recommendation:** Modulate hosts it as `Slew Lag` mode. Justification:
the input that makes this interesting is the sidechain, which Modulate
already reads natively for RM and Gravity Phaser; Dynamics doesn't
currently route per-bin sidechain.

- Mode: `Slew Lag`
- Inputs: bin magnitude, sidechain instantaneous magnitude derivative
  (computed once per hop in `ModuleContext`).
- Output: bin mag's per-hop change is clamped to `±max_delta * (1 -
  sidechain_derivative_normalized)`.
- Curves: AMOUNT (max slew at zero sidechain), REACH (sidechain band to
  watch), THRESHOLD (derivative deadband), MIX.
- CPU: light. One `max()` and one `min()` per bin.

### d) PLPV phase unwrapping for PLL Tear and Phase Phaser

**Concept (per the brainstorm PVX section, now PLPV).** Standard FFT phase is
wrapped to ±π; using *unwrapped* phase for PLL tracking and phase-tilt
calculations makes the math much cleaner and avoids click artefacts at
window boundaries.

**Spec gap.** The Modulate spec's PLL Tear references "the bin's phase
from the previous frame" without specifying unwrapping. The Phase
Phaser specifies a "deterministic phase tilt" but doesn't say whether
it operates on wrapped or unwrapped phase.

**Recommendation:** make PLPV-style phase unwrapping a *Pipeline-level
amenity* exposed via `ModuleContext.unwrapped_phase: &[f32]`, computed
once per hop and shared with all modules that need it. Modulate's PLL
Tear and Phase Phaser both opt in. Other modules (Dynamics ducking,
PhaseSmear, Freeze) also benefit — see `20-plpv-phase-cross-cutting.md`
for the full picture.

- For PLL Tear specifically: tracking unwrapped phase *velocity*
  rather than wrapped phase makes the "lock loss" detection
  threshold-able in physical units (Hz/s deviation) rather than
  arbitrary radians/hop.
- For Phase Phaser: the phase rotation curve becomes a continuous
  additive offset on unwrapped phase, then re-wrapped before iFFT. No
  audible click at the ±π boundary.

This is intrusive enough to the Pipeline that it belongs in
`20-plpv-phase-cross-cutting.md` and the global infra discussion. Cross-
reference here.

### e) Amplitude-gated phase modulation

**Concept (per Kim).** From the brainstorm: "For smear -
Amplitude-dependent crossover that sets the probability of phase
change. Should enable the normal dynamic control for phase change too."

This pattern — phase manipulation gated on bin amplitude — applies
generally to all phase-touching modules. For Modulate specifically:

- **Phase Phaser** could expose an `amp_gate_curve` that scales the
  rotation amount per bin by `min(amp / threshold, 1)`. Quiet bins
  pass through clean; loud bins get the rotation.
- **Gravity Phaser** could expose the same gate.

**Mechanism.** Add an `AmpGate` curve to the Modulate module's curve
set, used by phase modes only. When set to 0 across the spectrum, the
phase mode behaves as today (no gating). When non-zero, it scales the
effect strength per bin.

This is light — one extra `&[f32]` curve, one multiply per bin. Worth
the curve slot.

### f) Repel toggle for Gravity Phaser

**Concept (per Kim).** "Switch to turn wells into locations pushing
bins away from them." The spec mentions "invert every peak" / "invert
every other peak" switches but not a fundamental Repel toggle.

**Recommendation:** add `Repel: bool` to the GravityPhaser sub-effect's
mode-specific state (not a curve — a single switch). When on, the
phase-randomization force is inverted: nearby bins have their phase
*pulled away* from a coherent value, increasing decorrelation rather
than locking.

This is one boolean and a sign flip in the inner loop.

### g) Sidechain-positioned gravity nodes

**Concept (per Kim).** "Both of above can taken locations from the side
chain, graphs set width, amplitude, strength and reach."

The spec says "Nodes can receive position from sidechain peak
detection" but doesn't enumerate the four-curve scheme. Concretize:

- **NODE WIDTH curve:** how many bins wide each node is (Q in spectral
  units).
- **AMPLITUDE curve:** how strong the phase-randomization force is at
  each node.
- **STRENGTH curve:** *(redundant with AMPLITUDE? Or is this the
  per-node attack/release rate?)* Open question — the brainstorm uses
  both "amplitude" and "strength" without distinguishing them.
- **REACH curve:** how far the force extends from each node centre.

The spec already maps REACH to `Curve 1`. Adding three more would push
total curves to 8, exceeding the 7-curve `NUM_CURVE_SETS` limit.

**Recommendation:** keep the existing 5 curves; expose
sidechain-positioned wells as a *mode toggle* on Gravity Phaser
(`SidechainPositioned: bool`), with the four properties multiplexed
through the existing curves:

- AMOUNT = STRENGTH (force scaling)
- REACH = node width × spread
- RATE = node motion rate (when sidechain moves)
- THRESHOLD = sidechain peak detection floor
- MIX = wet/dry

Not perfect — STRENGTH and AMPLITUDE collapse — but stays inside the
curve budget.

## Mode list — final

After audit, the recommended Modulate module sub-effects are:

| Mode | Origin | New? |
|---|---|---|
| Phase Phaser | spec | — |
| Gravity Phaser | spec, with Repel toggle (g) and Sidechain mode (h) | refined |
| FM Network — Partial Web | spec | — |
| RM/FM Matrix — Bin-to-Sidechain | spec | — |
| Diode RM | brainstorm Cat 5 #17 | new |
| PLL Tear | spec, with PLPV-unwrapped phase (§ d) | refined |
| Bin Swapper | spec | — |
| Ground Loop | brainstorm Cat 4 #16 | new |
| Slew Lag | brainstorm Lag idea | new |

Nine modes is heavy. Consider splitting into Modulate I (phase-domain)
and Modulate II (carrier/RM/sidechain-driven) for v2. For v1, ship 6 of
these and defer 3 (suggest defer Slew Lag, Ground Loop, Diode RM —
they're the most "flavor" and least architecturally novel).

## Curves

The spec has 5: AMOUNT, REACH, RATE, THRESHOLD, MIX. Adding `AmpGate`
(§ e) for phase modes would push to 6. Total stays under
`NUM_CURVE_SETS = 7`.

| Idx | Label | Used by |
|---|---|---|
| 0 | AMOUNT | All modes |
| 1 | REACH | FM Network, Gravity Phaser, RM (falloff), Diode RM (carrier band), Ground Loop (harmonics), Slew Lag (sidechain band) |
| 2 | RATE | PLL (lock speed), Phase Phaser (animation), Ground Loop (mains freq), Gravity Phaser sidechain mode (motion rate) |
| 3 | THRESHOLD | PLL (tear delta), FM Network (partial detection), Diode RM (mismatch shutoff), Ground Loop (sag sensitivity), Slew Lag (deadband) |
| 4 | AMPGATE | Phase Phaser (amp-gated rotation), Gravity Phaser (amp-gated force) |
| 5 | MIX | All modes |

6 curves. `num_curves() = 6`.

## Architecture fit

### SpectralModule slot, with PLPV-aware ModuleContext

Modulate needs from the global infra:

- **ModuleContext.unwrapped_phase: &[f32]** — required by Phase Phaser
  (clean tilt) and PLL Tear (Hz/s lock-loss threshold). See
  `20-plpv-phase-cross-cutting.md`.
- **ModuleContext.instantaneous_freq: &[f32]** — required by FM Network
  for partial detection (already specced).
- **ModuleContext.sidechain_derivative: &[f32]** — required by Slew
  Lag. New, see `01-global-infrastructure.md` § ModuleContext additions.
- **BinPhysics.phase_momentum** — required by Gravity Phaser and PLL
  Tear (already specced).

### Per-channel state

PLL Tear's `pll_phase[N]` and `pll_freq[N]` arrays are already speccede
as per-instance. With Independent stereo, these need to be per-channel:
`pll_phase[2][MAX_NUM_BINS]`. Memory: 8193 × 4 × 2 bytes = ~66 KB per
slot. Fine.

Gravity Phaser's node positions (when sidechain-positioned) need
per-channel state too if the user wants per-side asymmetric behaviour.
Default to mono nodes; per-channel is a v2 nice-to-have.

### Why the user-facing name "Modulate" matters

Per Kim's framing in `14-future.md`, the category names ("Past,"
"Future," "Life," "Kinetics") push the user away from "oh, a delay,
mmh" reactions. "Modulate" sits in the same family — it groups
phase-domain, carrier-domain, and sidechain-driven effects under a
single conceptual umbrella ("things that modulate other things") rather
than exposing the sub-techniques (RM, FM, PLL).

## CPU class

Light overall. The heaviest mode is FM Network (partial detection +
shifting), which is the same cost as Harmony's pitch-tracking partial
detection — share the `compute_instantaneous_freq()` utility.

PLL Tear is moderate (one second-order loop per active bin, but only
bins above the magnitude threshold need tracking — gate it).

Diode RM, Ground Loop, Bin Swapper, Slew Lag are all O(N) per bin.

`heavy_cpu = false` for the module overall. Per-mode `heavy` flag could
mark FM Network as slightly heavier.

## BinPhysics interactions

Reads:

- `phase_momentum` (Gravity Phaser, PLL Tear).

Writes:

- `phase_momentum += delta` (Gravity Phaser when active, PLL Tear when
  losing lock).

Modulate is one of the few modules that *writes* phase state. Order
matters in the slot chain: a Modulate slot before a Past slot will
have its phase_momentum visible to Past's phase-coherent stretch
playback.

## Calibration probe set

- `probe_amount_pct`
- `probe_active_mode_idx`
- `probe_pll_lock_pct` (for PLL Tear: % of bins currently locked)
- `probe_partial_count` (for FM Network: # of detected partials)
- `probe_node_count` (for Gravity Phaser: # of active gravity nodes)

## RESEARCH PROMPT — Real-time per-bin PLL stability

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

## RESEARCH PROMPT — Buchla-style amplitude envelope model for RM gating

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

## Open questions

1. **Mode count cap.** 9 modes is a lot for one module. Split into
   Modulate I + II for v2, or ship 6/9 and defer the rest? See § Mode
   list.
2. **Curve count.** Adding AmpGate pushes to 6. Are there other phase-
   gating contexts that warrant moving AmpGate into ModuleContext as a
   shared per-bin scaler? (Harmony, Life, Past could all reuse it.)
3. **PLPV integration timing.** The unwrapped-phase work belongs in
   global infra. Modulate is the highest-leverage consumer — should
   PLPV work block Modulate, or can Modulate ship with wrapped phase
   first and upgrade later? See `20-plpv-phase-cross-cutting.md`.
4. **Diode RM vs RM mode.** Ship as a separate mode or as a "Mismatch"
   curve added to the existing RM mode?
5. **Ground Loop frequency selection.** Hard-coded 50/60 Hz toggle, or
   user-defined? The brainstorm cites mains hum specifically. Hard-coded
   gives the period feel; user-defined opens it up to "ground loop at
   any pitch" which is a different effect entirely.
6. **Slew Lag location.** Modulate or Dynamics? Justification given
   above (Modulate already reads sidechain per bin) but worth a sanity
   check.
7. **Kinetics overlap.** Modulate's Gravity Phaser is *phase-only* and
   Kinetics' gravity wells are *magnitude / frequency*. Confirm the
   distinction stays clean in the UI — users will conflate "gravity"
   without explicit module-name prefixes.

## Research findings (2026-04-26)

PLL Tear and Buchla 281-style envelopes are covered respectively by
`research/01-pvx-phase-and-pll.md` (Topic B) and
`research/06-specialized-topics.md` (Topic D). Validated decisions:

1. **PLL bank topology = 2nd-order PI loop.** Per bin: `pll_phase[k]`,
   `pll_freq[k]`. Update is 4 FLOPs per bin per hop:
   ```
   phase_error = unwrapped[k] - predicted[k]
   pll_freq[k] += beta * phase_error
   pll_phase[k] += pll_freq[k] + alpha * phase_error
   predicted[k] = pll_phase[k]
   ```
   `alpha = 2·ζ·ωₙ`, `beta = ωₙ²` in cycles-per-hop units. Defaults
   `ωₙ=0.05`, `ζ=0.707` (Butterworth-flat). 32k FLOPs/hop at 8193
   bins — negligible. SoA layout, AVX2 (8 lanes) or AVX-512 (16 lanes).
2. **Lock-loss detector = instantaneous phase-error magnitude with
   hysteresis** (`> π/2 → torn`, `< π/8 for N hops → re-locked`).
   Sliding-window phase-error variance is the smooth secondary check;
   tear only when both fire. Tying tear sample-tight to the transient
   is what makes it musical, not syrupy.
3. **Don't enable PLL tracking below bin ~16** (~344 Hz at hop=128 /
   sr=44.1k). Sub-100Hz partials don't tear meaningfully and the
   tracking introduces artefacts.
4. **PVX unwrapped phase is a free input** when available, makes the
   loop branch-free (no ±π wrap inside the update). PLL bank should
   compute its own local unwrap when global PVX is off (~3 FLOPs/bin)
   so it's self-sufficient.
5. **Curves expose:** `RATE` → ωₙ (loop bandwidth), `THRESHOLD` →
   lose-lock threshold, `AMOUNT` → wet/dry of torn output, `REACH` →
   bin range affected.
6. **No suitable Rust PLL-bank library exists.** Reference: liquid-dsp
   PLL example (MIT, single-channel) for inner loop;
   alexandrefrancois/Oscillators for the SIMD pattern. Live in new
   `src/dsp/pll.rs`.
7. **Buchla envelope = vactrol AR (Buchla 292 character) by default.**
   Asymmetric 1-pole follower with `α_attack = exp(-hop/τ_attack)`,
   `α_release = exp(-hop/τ_release)`; defaults τ_attack ≈ 1 ms,
   τ_release ≈ 80 ms. **Optional: AR with exp-mapped rate** (Buchla
   281 character) — same code path, different update rule.
8. **Trigger detector = band-energy on ERB-spaced bands** (8 bands
   default, up to 32). The "Buchla bongo" character — a single
   sidechain peak fires a band of envelopes simultaneously. Per-bin
   trigger as alternative mode. Hysteresis: 6 dB difference between
   on/off thresholds. No special re-trigger logic — the asymmetric
   follower handles overlap naturally (sums upward).
9. **EnvelopeBank shared primitive.** The same SIMD envelope kernel
   serves Modulate (Buchla envelopes), Punch (healing-curve recovery),
   and Kinetics (Orbital satellite-rotation smoothing). Implement once
   in `src/dsp/envelope.rs` rather than three times. Branchless
   `mask.select(α_attack, α_release)` per lane; <0.1% of one core
   for the whole RM/FM Matrix slot in stereo.
