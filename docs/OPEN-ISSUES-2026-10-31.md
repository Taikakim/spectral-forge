# Open issues for review — October 31, 2026

Everything still open after branch `feature/finish-modules-bracketing`, in one
place. Sources: the offline bracketing sweep and the real-plugin audition in
[`BRACKETING-2026-10-30.md`](BRACKETING-2026-10-30.md), plus `cargo test`.
Listening page: run `tools/audition.py` (see CLAUDE.md), then open
`target/audition/index.html`.

Kinds: **Decision** — needs a design call before code. **Bug** — behavior is
wrong and the fix is clear. **Check** — measured oddity; confirm by ear or by
reading the code before deciding. Verified = confirmed in the code; observed =
measurement only.

## Decisions

| # | Issue | Evidence | Options |
|---|---|---|---|
| D1 | **Sidechain arrives at raw FFT scale** (verified). `Pipeline` passes `sc_envelopes × slot gain` (a −20 dBFS tone ≈ 50) to modules written for 0…1. Gain Add +23…26 dB at default, Gain Subtract −24 dB on drums, Modulate Diode RM +16…20 dB, Punch thresholds (0.05…0.30) always exceeded. Root of ~40 % of the offline HIGH flags. | Bracketing finding 1; `src/dsp/pipeline.rs` (`sc_envelopes`) | (a) Normalize once in `Pipeline` (e.g. divide by Hann sum, FFT/2) and recalibrate every SC module; (b) normalize per module. (a) is one fix point but changes every SC module's feel. Not testable through pedalboard (no SC bus); use the offline harness. |
| D2 | **Rhythm mutes with a stopped transport** (verified). With a tempo but no running transport, the beat position sits at a step edge, the attack-fade gate is 0, so AMOUNT 100 % mutes Euclidean and Arpeggiator. Passes through only when the host gives no tempo (`bpm <= 1e-3`). Same in a DAW when monitoring while stopped. | Audition verdict BROKEN (Euclidean, Arpeggiator); `src/dsp/modules/rhythm.rs:182` | (a) Bypass when stopped; (b) free-run on an internal clock at host tempo. |
| D3 | **Silent or inert on their own, by dependency.** Circuit Vactrol needs upstream flux (`apply_vactrol` only falls back to magnitude when `physics` is `None`, but writers always get `Some`) → −300 dB alone. Circuit Component Drift: temperature feedback starts at zero. Life Brownian: needs an upstream temperature writer. | Audition BROKEN (Vactrol), NO EFFECT (Component Drift, Brownian); `src/dsp/modules/circuit.rs:175`, `:370` | (a) Self-driven fallback when the field is all zero; (b) help-text note "needs an upstream … writer". |
| D4 | **AMOUNT = 0 mutes instead of bypassing** (observed): Future (both modes), Circuit BBD, Circuit Transformer Saturation (−280 dB cliff to the next step). | Bracketing finding 10 | Conform to the "0 = dry" convention or document as intended. |

## Bugs and level problems

| # | Issue | Evidence | Suggested fix |
|---|---|---|---|
| B1 | **Rhythm Arpeggiator is silent until steps are drawn** (verified). `ArpGrid::default()` is all zero. | Bracketing finding 3 | Seed a default pattern on assignment. |
| B2 | **Past Convolution +33…45 dB at small Delay** (observed). Current ⊗ past multiplies raw magnitudes, so output scales with level². | Bracketing finding 5; `src/dsp/modules/past.rs:550` | Normalize the product (same family as D1). |
| B3 | **Harmony Lifter SPREAD / COEFFICIENT explode past curve gain 2** (observed): gain 2 → 4 jumps 54…88 dB, output peak +31…45 dB. | Bracketing finding 6 | Clamp or compress the mapping above gain 2. |
| B4 | **Modulate FM Network +15…23 dB at default**; first AMOUNT step jumps the same (observed). | Bracketing finding 7 | Level-compensate. |
| B5 | **Kinetics Hooke +17…24 dB at default** (observed); springs add energy. | Bracketing finding 8 | Energy-conserving update or damping floor. |
| B6 | **Contrast RATIO** output peak +12…27 dB; Tilt mode cliff 19…24 dB between gain 2.7 and 4.5 (observed). | Bracketing finding 9 | Smooth the mapping. |
| B7 | **Large default level changes** (observed): Future Print-Through −35…−39 dB, Circuit Bias Fuzz −16…−29 dB, Transformer −10…−22 dB, Life Archimedes −26 dB (noise), Circuit Slew Distortion THRESH first step 29…37 dB cliff. | Bracketing finding 11 | Make-up gain or re-centered defaults. |
| B8 | **Mid/Side EXPANSION has no ceiling**: nodes at +12…18 dB give +12…17 dB peaks. | Bracketing finding 12 | Cap or soft-limit. |

## Checks

| # | Issue | Evidence |
|---|---|---|
| C1 | **No effect alone at any single-knob setting**: Geometry Chladni (−40 dB change at most), Life Surface Tension (−28 dB), Past Granular and Convolution, Kinetics Gravity Well. Offline harness agrees. Look at defaults and amount mapping. | Audition NO EFFECT; `geometry.rs:86`, `life.rs:305`, `past.rs:415`, `past.rs:550`, `kinetics.rs:428` |
| C2 | **Changes the sound but the help-text claim is not confirmed — listen**: Gain Match, Future Print-Through / Pre-Echo (no energy before the onset on the pad, so the "pre" part is not happening), Slew Lag (no hang over true silence; only scales existing content), Circuit PCB Crosstalk, Life Viscosity, Past Reverse / Stretch, Harmony Inharmonic, Rhythm Arpeggiator with a drawn grid. | Audition verdict OTHER EFFECT |
| C3 | **Curves with no measurable effect one at a time** (beyond the by-design interaction cases): Gain PEAK HOLD in Add/Subtract (shown but unused); Punch FILL_MODE (drift slew-limited to 0.005 bin/hop); Past Granular / Convolution / Stretch AMOUNT and MIX; Kinetics Inertial Mass MASS/MIX, Tuning Fork and Diamagnet REACH/MIX; Harmony Formant Rotation AMOUNT/MIX. Life Non-Newtonian and Brownian were on this list before the physics-access fix; re-run to confirm. | Bracketing "Curves with no measurable effect" |
| C4 | **Sidechain modes untested through the real plugin**: Punch Direct/Inverse, Modulate RM/FM and Diode RM. Pedalboard exposes no SC bus. Settle D1 first, then test in a DAW. | Audition SKIP |

## Tests

| # | Issue |
|---|---|
| T1 | **11 tests already fail on `master` `a6b58fe`** (not caused by this branch): `all_offset_fns_are_neutral_at_zero`, `contrast_amount_offset_extremes`, `geometry_helmholtz_amount_default_probes_50_pct`, `punch_direct_amount_default_probes_50_pct`, `rhythm_{arpeggiator,euclidean,phase_reset}_amount_default_probes_50_pct`, and four `display_mapping_contract` cases (`freeze_threshold_gain_1p5_matches_log_dsp`, `freeze_threshold_gain_2_maps_to_0_dbfs`, `portamento_physical_to_y_at_y_min_is_rect_bottom`, `portamento_screen_y_to_physical_at_bottom_is_y_min`). Either the calibration contract or the code drifted; decide which is right. |
| T2 | **`if_probe_observes_none_when_ctx_has_none` is flaky** — fails intermittently, passes on rerun. Likely shared global probe state between parallel tests. |

Last full run on this branch (`3ad6311`): 666 passed, 11 failed (T1).

## Closed on this branch (for reference)

PLPV re-wrap erasing phase-only effects; split rows not reaching Master; Life
Non-Newtonian / Stiction / Brownian physics access (closes bracketing finding
4); Slew Lag muting after silence; swapped Circuit BBD / Crossover help.
Details in `BRACKETING-2026-10-30.md`, "Plugin bugs it found".
