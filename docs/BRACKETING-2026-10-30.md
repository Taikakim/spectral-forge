# Parameter bracketing — first sweep (2026-10-30)

Harness: `examples/bracket.rs` (`cargo run --release --example bracket -- --threads 4`).
Run on master + `feature/finish-modules-bracketing` @ `6c3b581`, 18 modules, every
mode, every curve, three stimuli (pink-ish noise −20 dBFS RMS, 3-voice saw chord
−6 dBFS peak, drum loop), 96 hops at 48 kHz / FFT 2048. 1 149 jobs × (13 offset
positions + 7 curve-node gains + 1 live sweep) in 166 s on 4 threads.
Raw data: `target/bracket/{settings,live,baselines,findings}.csv` (regenerate; not committed).

## Method and limits

- **Module level, not Pipeline level.** Each module gets the same inputs the
  Pipeline would give it: Hann-windowed unnormalised FFT bins, raw-magnitude
  sidechain (drums for noise/chord, noise for drums), IF, harmonic groups,
  chromagram, cepstrum, history buffer (Past), fresh `BinPhysics` per hop for
  writers (as `FxMatrix` does). PLPV peaks / unwrapped phase are **not** supplied.
- **One curve at a time.** Every other curve sits at gain 1.0 (the default after
  assignment). Effects that need two curves moved together (e.g. Dynamics needs
  RATIO > 1:1 before THRESHOLD matters) read as "dead".
- **Main output only.** Virtual outputs (T/S Split T/S, Harmonic H/P) are not
  measured, so their controls read as "dead" on the main output by design.
- Metrics: `gain_db` (energy out/in), `delta_db` (energy of out − in; sees
  phase-only edits), `peak_bin_gain_db` (loudest output bin vs the loudest input
  its ±2-bin neighbourhood ever had — holds/freezes stay ≤ 0 dB), `peak_out_db`
  (run output peak vs input peak), spread (% of bins changed > ~1 dB), spill.

## Clean bills

- **No NaN/Inf** in any static or live run.
- **No motion spikes:** no live sweep exceeded its worst static setting by > 12 dB.
- **No hidden effects:** no curve marked inactive by a mode's `CurveLayout` changes the output.

## Findings (most severe first)

Verified = confirmed by reading the code. Observed = harness measurement only.

1. **Sidechain arrives at raw FFT scale (verified, root cause of ~40 % of HIGH flags).**
   `Pipeline` feeds modules `sc_envelopes × slot gain`, i.e. raw magnitudes
   (a −20 dBFS tone ≈ 50). Modules written for a 0…1 sidechain then misbehave:
   Gain **Add** `bins *= g + sc` → **+23…26 dB** at default; Gain **Subtract**
   `(g − sc).max(0)` → −24 dB on drums; Modulate **DiodeRm** +16…20 dB at
   default; Punch's 0.05…0.30 peak thresholds are always exceeded. One fix point
   (normalise the envelope in `Pipeline`, e.g. by the Hann window sum FFT/2) but
   it recalibrates every SC module, so it needs a decision.
2. **Circuit Vactrol is silent on its own (verified).** `apply_vactrol` falls
   back to bin magnitude only when `physics` is `None`, but `FxMatrix` always
   passes `Some` to writer modules, with `flux = 0` unless an upstream slot
   wrote it → the LDR is never driven → −300 dB.
3. **Rhythm Arpeggiator is silent until steps are drawn (verified).**
   `ArpGrid::default()` is all-zero; switching a slot to Arpeggiator mutes it.
4. **Life Stiction mutes at AMOUNT ≥ 100 %, i.e. at default (observed).**
5. **Past Convolution: +33…45 dB at small Delay (observed).** Current ⊗ past
   multiplies raw magnitudes, so the result scales with level² — same family as 1.
6. **Harmony Lifter: SPREAD / COEFFICIENT explode past curve gain 2 (observed).**
   Node gain 2 → 4 jumps 54…88 dB; output peak +31…45 dB.
7. **Modulate FM Network: +15…23 dB at default; first AMOUNT step jumps 15…23 dB (observed).**
8. **Kinetics Hooke: +17…24 dB at default (observed).** Springs add energy.
9. **Contrast RATIO: output peak +12…27 dB; Tilt mode cliff 19…24 dB between gain 2.7 and 4.5 (observed).**
10. **AMOUNT = 0 mutes instead of bypassing (observed):** Future (both modes),
    Circuit BBD, Circuit Transformer Saturation (−280 dB cliff to the next step).
11. **Large default level changes (observed):** Future Print-Through −35…−39 dB,
    Circuit Bias Fuzz −16…−29 dB, Transformer −10…−22 dB, Life Archimedes −26 dB
    (noise), Circuit Slew Distortion THRESH first step 29…37 dB cliff.
12. **Mid/Side EXPANSION has no ceiling:** nodes at +12…18 dB give +12…17 dB peaks.

## Curves with no measurable effect (one-at-a-time; all three stimuli)

Interaction/by-design (expected): Dynamics and Contrast THRESHOLD/ATTACK/RELEASE/KNEE/MIX
(RATIO defaults to 1:1); T/S Split SENSITIVITY/SMOOTHNESS and Harmonic H/P SEPARATE/SIZE/MIX
(virtual rows only); Harmonic Partial EQ AMOUNT/WIDTH/MIX (ODD/EVEN and TILT neutral).

Worth a look: Gain PEAK HOLD in Add/Subtract (curve shown but unused);
Punch FILL_MODE in every mode (drift slew-limited to 0.005 bin/hop);
Past Granular / Convolution / Stretch AMOUNT and MIX; Life Non-Newtonian
AMOUNT/THRESHOLD/MIX, Brownian AMOUNT/MIX; Kinetics Gravity Well (all), Inertial Mass
MASS/MIX, Tuning Fork and Diamagnet REACH/MIX; Harmony Formant Rotation AMOUNT/MIX.

## Real-plugin audition — October 31, 2026

`examples/bracket.rs` runs modules in isolation, so it cannot see bugs in the
pipeline around them. `tools/audition.py` (see CLAUDE.md) drives the built VST3
through pedalboard instead: one module in slot 0 → Master, the fresh-assignment
defaults, then each active curve's Offset at −1 / +1, judged against each
mode's help-text claim on material chosen to show it (pad with silences for
holds/pre-echo, drums for transients, a three-partial tone for new partials,
the stereo mix for width). The empty-slot null test leaves −66 dB of residual,
so pedalboard's latency compensation lines up exactly.

### Plugin bugs it found (fixed on this branch)

1. **PLPV re-wrap erased every phase-only effect (verified).** With
   `plpv_enable` on (the default) the pipeline rebuilt every bin from the
   module's magnitude and the *pre-module* phase. Phase Phaser, Gravity
   Phaser, PLL Tear, Rhythm Phase Reset, Kinetics Orbital Phase,
   Ferromagnetism and Thermal Expansion were bit-exact null (−66 dB); Exciter
   partials and Mid/Side DECORREL were altered. Fix:
   `plpv::recombine_with_module_phase` keeps the phase of bins a module rotated
   directly (> 1e-3 rad) and rebuilds only the rest. All seven now pass.
2. **Split rows sent straight to Master were silent (verified).** The Master
   sum in `FxMatrix` only added real slots; T/S and H/P rows reached Master
   only through another slot. Fix: virtual rows join the Master sum and the
   reach-Master check. T stream is now 90 dB punchier than S on drums, P 47 dB
   punchier than H.
3. **Life Non-Newtonian, Stiction and Brownian read physics only from `ctx`
   (verified).** Life is a BinPhysics writer, so the FxMatrix hands it its own
   `physics` handle and `ctx.bin_physics = None`; velocity and temperature were
   always zero. Non-Newtonian did nothing, Stiction muted at default (finding 4
   above), Brownian never drifted even after a temperature writer. Fix: read
   from whichever is present. Non-Newtonian and Stiction now pass; Brownian
   alone is still inert by design (needs an upstream temperature writer).
4. **Slew Lag muted notes after silence (verified, new mode).** The slewed
   level fell to the 1e-9 floor (≈ 240 dB below a note at raw-FFT scale), so
   the next note needed seconds to climb back. Now floored at 60 dB below the
   input: a note after silence swells in over at most 3 s (20 dB/s).
5. **Circuit help text for BBD and Crossover Distortion was swapped**
   (mode byte 0 showed Crossover text). Keyed by enum value now, with a test.

### Still open after the fixes

- **Silent alone, by dependency:** Circuit Vactrol (needs upstream flux),
  Component Drift (temperature feedback starts at zero), Life Brownian
  (needs temperature). Each is silent or inert as the only module; worth either
  a self-driven fallback or a "needs an upstream …" note in the help text.
- **Rhythm mutes with a stopped transport (verified).** With a tempo but no
  running transport, the beat position sits at a step edge and the
  attack-fade gate is 0, so AMOUNT 100 % mutes. The same happens in a DAW when
  monitoring with the transport stopped. Rhythm passes through only when the
  host gives no tempo at all. Design call: bypass or free-run when stopped.
- **No effect alone at any single-knob setting:** Geometry Chladni (−40 dB
  change at most), Life Surface Tension (−28 dB), Past Granular and
  Convolution, Kinetics Gravity Well. Same list as the offline harness;
  these need a look at their defaults and amount mapping.
- **Changes the sound, claim not confirmed by the metric (listen):** Gain
  Match, Future Print-Through / Pre-Echo (no energy before the onset, so the
  "pre" part is not happening on a pad), Slew Lag (no hang over true
  silence — it only scales existing content), PCB Crosstalk, Viscosity, Past
  Reverse / Stretch, Harmony Inharmonic.
- **Not covered here:** sidechain modes (Punch Direct/Inverse, RM/FM, Diode
  RM) — pedalboard exposes no sidechain bus; see the offline harness.
