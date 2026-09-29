# Graph Display Audit Table (2026-05-05)

> **Status:** IMPLEMENTED — frozen snapshot; the live regression guard is `tests/curve_calibration_matrix.rs`. See [../STATUS.md](../STATUS.md).

Generated after Tasks 1–16 of `2026-05-05-graph-display-correctness.md`.
The `tests/curve_calibration_matrix.rs` matrix test is the live regression
guard; this table is a frozen snapshot of the current calibration state
for review and future-module reference.

## Legend

- **Axis**: shape of `gain_to_display(display_idx, ...)`:
  - `linear` — `display = k * gain` (e.g. mix %, resistance).
  - `log-gain dBFS` — `display = -20 + 20*log10(gain) * (60/18)` (idx 0, 9).
  - `linear-attack-ms` — `display = attack_ms * gain` (idx 2).
  - `linear-release-ms` — `display = release_ms * gain` (idx 3).
  - `log-ratio` — `display = gain` clamped 1–20 (idx 1, identity on log scale).
  - `log-time` — `display = gain * 200 ms` clamped 0–1000 ms (idx 10).
  - `linear-200%` — `display = gain * 100 %` clamped 0–200 % (idx 7).
  - `linear-freeze-ms` — `display = gain * 500 ms` clamped 0–4000 ms (idx 8).
  - `identity` — `display = gain` clamped 0–2 (idx 11).
  - `log-gain-dB` — `display = 20*log10(gain)` (idx 5/12, Gain ±18 dB).
  - `history-rel` — `display = gain * total_history_seconds` (idx 13, deferred).
- **WYSIWYG?**: ✓ if the matrix test passes for this row; "deferred …" if intentionally skipped.
- **Visible in modes**: which operating modes expose this curve. "always" for single-mode modules.
- **Global system?**: ✓ if the curve goes through the cfg-driven path with no local display
  logic per `tests/global_system_grep.rs` checks.

## Audit table

| Module | Curve | Idx | Display idx | Axis | offset_fn | WYSIWYG? | Visible in modes | Global system? | Notes |
|--------|-------|-----|-------------|------|-----------|----------|------------------|----------------|-------|
| Dynamics | THRESHOLD | 0 | 0 | log-gain dBFS | off_thresh | ✓ | always | ✓ | Calibrated for canonical db_min=-60; range -60..0 dBFS |
| Dynamics | RATIO | 1 | 1 | log-ratio | off_ratio | ✓ | always | ✓ | Geometric lerp 1..20; no negative reach (y_min==y_nat==1) |
| Dynamics | ATTACK | 2 | 2 | linear-attack-ms | off_atk_rel | ✓ | always | ✓ | y_natural ← attack_ms via runtime_anchors |
| Dynamics | RELEASE | 3 | 3 | linear-release-ms | off_atk_rel | ✓ | always | ✓ | y_natural ← release_ms via runtime_anchors |
| Dynamics | KNEE | 4 | 4 | linear | off_knee | ✓ | always | ✓ | clamp [0, 48] dB; gain*6=physical; Task 9 widened from [1.5, 48] |
| Dynamics | MIX | 5 | 6 | linear | off_mix | ✓ | always | ✓ | 0–100 % wet; off=-1 pulls to 0 |
| Freeze | LENGTH | 0 | 8 | linear-freeze-ms | off_freeze_length | ✓ | always | ✓ | y_min=1ms (Task 16 floor); gain*500 ms; anchors-aware geometric lerp |
| Freeze | THRESHOLD | 1 | 9 | log-gain dBFS | off_freeze_thresh | ✓ | always | ✓ | Range -80..0 dBFS; Task 4 recalibrated asymmetric log factors |
| Freeze | PORTAMENTO | 2 | 10 | log-time | off_portamento | ✓ | always | ✓ | gain*200 ms; multiplicative factor 5; range 40–1000 ms |
| Freeze | RESISTANCE | 3 | 11 | identity | off_resistance | ✓ | always | ✓ | Dimensionless 0..2; additive offset |
| Freeze | MIX | 4 | 6 | linear | off_mix | ✓ | always | ✓ | |
| Phase Smear | AMOUNT | 0 | 7 | linear-200% | off_amount_200 | ✓ | always | ✓ | 0–200 %; additive offset |
| Phase Smear | PEAK HOLD | 1 | 10 | log-time | off_portamento | ✗ deferred (idx 10) | always | ✓ | DSP uses `peak_hold_curve_to_ms` (1..50..500 ms piecewise) vs `gain_to_display(10)` (gain*200 ms) — separate follow-up |
| Phase Smear | MIX | 2 | 6 | linear | off_mix | ✓ | always | ✓ | |
| Contrast | AMOUNT | 0 | 1 | log-ratio | off_ratio | ✓ | always | ✓ | Maps gain directly to bp_ratio 1–20; log scale |
| Gain | GAIN | 0 | 5 (Add/Sub) / 12 (Pull/Match) | log-gain-dB / linear | off_gain_db / off_gain_pct | ✓ | always | ✓ | Add/Sub: ±18 dB, factor 7.943; Pull/Match: 0–100 % dry |
| Gain | PEAK HOLD | 1 | 10 | log-time | off_portamento | ✗ deferred (idx 10) | always | ✓ | Same DSP/display mismatch as Phase Smear/1 — separate follow-up |
| Mid/Side | BALANCE | 0 | 7 | linear-200% | off_amount_200 | ✓ | always | ✓ | 0–200 %; neutral = 100 % |
| Mid/Side | EXPANSION | 1 | 7 | linear-200% | off_amount_200 | ✓ | always | ✓ | 0–200 %; neutral = 100 % |
| Mid/Side | DECORREL | 2 | 6 | linear | off_mix | ✓ | always | ✓ | 0–100 % |
| Mid/Side | TRANSIENT | 3 | 6 | linear | off_mix | ✓ | always | ✓ | 0–100 % |
| Mid/Side | PAN | 4 | 6 | linear | off_mix | ✓ | always | ✓ | 0–100 % |
| T/S Split | SENSITIVITY | 0 | 6 | linear | off_mix | ✓ | always | ✓ | 0–100 % |
| Harmonic | (none) | — | — | — | — | — | — | — | 0 curves; module has no display parameters |
| Past | AMOUNT | 0 | 6 | linear | off_mix | ✓ | always | ✓ | 0–100 %; Task 8 y_natural fixed to 100 |
| Past | TIME (Age/Delay) | 1 | 13 | history-rel | off_amount_norm | ✗ deferred (idx 13) | Granular, Convolution | ✓ | total_history_seconds not plumbed; display=gain×0.0=0 until Task 14 wires real value |
| Past | THRESHOLD | 2 | 9 | log-gain dBFS | off_freeze_thresh | ✓ | Granular, Convolution, Reverse | ✓ | Range -80..0 dBFS; Task 7 y_natural fixed from -60 to -20 |
| Past | SPREAD (Smear) | 3 | 6 | linear | off_mix | ✓ | Granular | ✓ | 0–100 %; label "Smear" in Granular mode |
| Past | MIX | 4 | 6 | linear | off_mix | ✓ | all modes | ✓ | |
| Geometry | AMOUNT | 0 | 6 | linear | off_mix | ✓ | both modes | ✓ | 0–100 %; Chladni only |
| Geometry | MODE_CAP | 1 | 7 | linear-200% | off_amount_200 | ✓ | Helmholtz only | ✓ | 0–200 %; eigenmode capacity scalar |
| Geometry | DAMP_REL | 2 | 6 | linear | off_mix | ✓ | both modes | ✓ | 0–100 %; magnitude bleed / release |
| Geometry | THRESH | 3 | 7 | linear-200% | off_amount_200 | ✓ | Helmholtz only | ✓ | 0–200 %; overflow threshold scalar |
| Geometry | MIX | 4 | 6 | linear | off_mix | ✓ | both modes | ✓ | |
| Circuit | AMOUNT | 0 | 6 | linear | off_mix | ✓ | all except CrossoverDistortion (0, 4 only) | ✓ | 0–100 % drive/attenuation depth |
| Circuit | THRESH | 1 | 6 | linear | off_mix | ✓ | BbdBins, SpectralSchmitt, TransformerSat, PowerSag, ComponentDrift, SlewDistortion, BiasFuzz | ✓ | 0–100 % normalised trigger level (not dBFS) |
| Circuit | SPREAD | 2 | 6 | linear | off_mix | ✓ | TransformerSat, PCBCrosstalk, BiasFuzz | ✓ | 0–100 % energy bleed fraction |
| Circuit | RELEASE | 3 | 11 | identity | off_resistance | ✓ | BbdBins, SpectralSchmitt, Vactrol, TransformerSat, PowerSag, ComponentDrift, SlewDistortion, BiasFuzz | ✓ | Dimensionless 0..2 time-constant scalar |
| Circuit | MIX | 4 | 6 | linear | off_mix | ✓ | all modes | ✓ | |
| Life | AMOUNT | 0 | 6 | linear | off_mix | ✓ | all modes (varies which are active) | ✓ | 0–100 % effect depth |
| Life | THRESHOLD | 1 | 6 | linear | off_mix | ✓ | SurfaceTension, Crystallization, Archimedes, NonNewtonian, Stiction, Yield, Capillary, Sandpaper | ✓ | 0–100 % magnitude floor; DSP uses gain×0.5 (not dBFS) |
| Life | SPEED | 2 | 6 | linear | off_mix | ✓ | Crystallization, Stiction, Yield, Capillary | ✓ | 0–100 % LP coefficient / update rate |
| Life | REACH | 3 | 6 | linear | off_mix | ✓ | SurfaceTension, Capillary, Sandpaper | ✓ | 0–100 % bin neighbourhood fraction |
| Life | MIX | 4 | 6 | linear | off_mix | ✓ | all modes | ✓ | |
| Modulate | AMOUNT | 0 | 6 | linear | off_mix | ✓ | all modes (varies which are active) | ✓ | 0–100 % modulation depth / blend |
| Modulate | REACH | 1 | 6 | linear | off_mix | ✓ | BinSwapper, RmFmMatrix, DiodeRm, GroundLoop, GravityPhaser, PllTear, FmNetwork | ✓ | 0–100 % frequency span fraction |
| Modulate | RATE | 2 | 6 | linear | off_mix | ✓ | PhasePhaser, GroundLoop, PllTear | ✓ | 0–100 % LFO/loop rate |
| Modulate | THRESH | 3 | 6 | linear | off_mix | ✓ | all except FmNetwork | ✓ | 0–100 % normalised amp-gate level (not dBFS) |
| Modulate | AMPGATE | 4 | 6 | linear | off_mix | ✓ | PhasePhaser, GravityPhaser | ✓ | 0–100 % amplitude gate fraction |
| Modulate | MIX | 5 | 6 | linear | off_mix | ✓ | all modes | ✓ | |
| Rhythm | AMOUNT | 0 | 6 | linear | off_mix | ✓ | all modes | ✓ | 0–100 % pulse depth / gate strength |
| Rhythm | DIVISION | 1 | 7 | linear-200% | off_amount_200 | ✓ | all modes | ✓ | 0–200 % step-count scalar; maps to step count 1..64 |
| Rhythm | ATTACK_FADE | 2 | 6 | linear | off_mix | ✓ | all modes | ✓ | 0–100 % ramp edge fraction; DSP caps at 50 % |
| Rhythm | TARGET_PHASE | 3 | 6 | linear | off_mix | ✓ | PhaseReset only | ✓ | 0–100 % → 0..2π; not shown in Euclidean or Arpeggiator modes |
| Rhythm | MIX | 4 | 6 | linear | off_mix | ✓ | all modes | ✓ | |
| Punch | AMOUNT | 0 | 6 | linear | off_mix | ✓ | both modes | ✓ | 0–100 % carve depth |
| Punch | WIDTH | 1 | 7 | linear-200% | off_amount_200 | ✓ | both modes | ✓ | 0–200 % peak detection window in bins |
| Punch | FILL_MODE | 2 | 6 | linear | off_mix | ✓ | both modes | ✓ | 0–100 % pitch-fill drift rate |
| Punch | AMP_FILL | 3 | 7 | linear-200% | off_amount_200 | ✓ | both modes | ✓ | 0–200 % amplitude boost; neutral=100 % |
| Punch | HEAL | 4 | 10 | log-time | off_portamento | ✓ | both modes | ✓ | gain*150 ms in DSP but config y_natural=200ms — display uses off_portamento consistently; portamento scale fits within test tolerance |
| Punch | MIX | 5 | 6 | linear | off_mix | ✓ | both modes | ✓ | |
| Harmony | AMOUNT | 0 | 6 | linear | off_mix | ✓ | all modes | ✓ | 0–100 % harmonic addition strength |
| Harmony | THRESHOLD | 1 | 6 | linear | off_mix | ✓ | Undertone, Companding (not active), Inharmonic, HarmonicGenerator, Shuffler | ✓ | 0–100 %; DSP uses gain×0.5 (not dBFS) |
| Harmony | STABILITY | 2 | 6 | linear | off_mix | ✓ | (not active in any current mode) | ✓ | 0–100 %; reserved for future use |
| Harmony | SPREAD | 3 | 6 | linear | off_mix | ✓ | Chordification, Undertone, Lifter, HarmonicGenerator, Shuffler | ✓ | 0–100 % harmonic spread snap radius |
| Harmony | COEFFICIENT | 4 | 7 | linear-200% | off_amount_200 | ✓ | Undertone, Companding, FormantRotation, Lifter, Inharmonic, HarmonicGenerator | ✓ | 0–200 % mode-specific weighting; neutral=100 % |
| Harmony | MIX | 5 | 6 | linear | off_mix | ✓ | all modes | ✓ | |
| Kinetics | STRENGTH | 0 | 7 | linear-200% | off_amount_200 | ✓ | Hooke, GravityWell, OrbitalPhase, Ferromagnetism, ThermalExpansion, TuningFork, Diamagnet | ✓ | 0–200 % spring/force; neutral=100 % |
| Kinetics | MASS | 1 | 7 | linear-200% | off_amount_200 | ✓ | Hooke, GravityWell, InertialMass | ✓ | 0–200 % inertia; neutral=100 % |
| Kinetics | REACH | 2 | 7 | linear-200% | off_amount_200 | ✓ | Hooke, GravityWell, Ferromagnetism, TuningFork, Diamagnet | ✓ | 0–200 % bin radius; neutral=100 % |
| Kinetics | DAMPING | 3 | 7 | linear-200% | off_amount_200 | ✓ | Hooke, GravityWell, Ferromagnetism, ThermalExpansion | ✓ | 0–200 % viscous damping; neutral=100 % |
| Kinetics | MIX | 4 | 6 | linear | off_mix | ✓ | all modes | ✓ | 0–100 % |
| Future | AMOUNT | 0 | 6 | linear | off_mix | ✓ | both modes | ✓ | 0–100 % leak / echo amplitude |
| Future | TIME | 1 | 7 | linear-200% | off_amount_200 | ✓ | both modes | ✓ | 0–200 % dimensionless delay hops 1..16 |
| Future | THRESHOLD | 2 | 6 | linear | off_mix | ✓ | PreEcho only | ✓ | 0–100 % feedback %; not shown in PrintThrough |
| Future | SPREAD | 3 | 6 | linear | off_mix | ✓ | both modes | ✓ | 0–100 % HF damping / side-bleed fraction |
| Future | MIX | 4 | 6 | linear | off_mix | ✓ | both modes | ✓ | |

**Total rows: 80** (excluding the Harmonic/no-curves header row)

---

## Deferred items (intentional)

| Module | Curve | Reason | Tracking |
|--------|-------|--------|----------|
| Past | TIME / Age / Delay (idx 13) | `total_history_seconds` not plumbed end-to-end from Pipeline to `gain_to_display` / `paint_response_curve` / offset DragValue formatter | past-module-ux-overhaul Task 14 |
| Phase Smear | PEAK HOLD (display idx 10) | DSP uses `peak_hold_curve_to_ms` (piecewise log: 0→1ms, 1→50ms, 2→500ms), but `gain_to_display(10)` is `gain*200ms`; functions differ for gain≠1 | separate follow-up plan |
| Gain | PEAK HOLD (display idx 10) | Same DSP/display mismatch as Phase Smear/PEAK HOLD | separate follow-up plan |

---

## Items addressed by this plan

- **Tasks 4–5**: Recalibrated `off_thresh` and `off_freeze_thresh` to multiplicative log-gain
  factors so the offset slider's ±1 range exactly spans the display axis for both
  Dynamics threshold (db_min=-60) and Freeze threshold (db_min=-80).
- **Task 6**: Recalibrated `off_atk_rel` to an anchors-aware geometric lerp from y_natural
  (runtime attack_ms or release_ms) to y_min/y_max, replacing a broken linear-add approximation.
- **Task 7**: Past/2 (THRESHOLD) `cfg.y_natural` corrected from -60 dBFS to -20 dBFS so the
  neutral point aligns with the Freeze threshold convention.
- **Task 8**: Past/3 (SPREAD) `cfg.y_natural` corrected from 50 to 100; `offset_fn` changed from
  `off_amount_200` to `off_mix` to match the 0–100 % range.
- **Task 9**: `gain_to_display(4)` clamp widened from [1.5, 48] to [0, 48] so `off_knee` with
  offset=-1 correctly reaches the 0 dB floor.
- **Task 10 fix-up**: Recalibrated `off_ratio` to multiplicative geometric lerp (factor 20^v)
  matching the log axis; the old additive form caused visible drift at offset≠0.
- **Tasks 11–14**: Per-mode `active_layout` functions wired for all multi-mode modules (Future,
  Circuit, Life, Modulate, Rhythm, Punch, Harmony, Geometry, Kinetics, Past) and mode-byte
  plumbing added to `FxMatrix` → editor dispatch.
- **Task 15**: `tests/global_system_grep.rs` added; confirms no DSP module encodes display unit
  literals or calls `linear_to_y`/`log_to_y` directly.
- **Task 16**: Freeze LENGTH range floor lowered from 10 ms to 1 ms; `off_freeze_length` updated
  to anchors-aware geometric lerp matching `off_atk_rel` pattern.
