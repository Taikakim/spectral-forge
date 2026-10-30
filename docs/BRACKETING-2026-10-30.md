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
