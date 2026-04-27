# Superpowers plans & specs — implementation status

**Last updated:** 2026-04-27 (Phase 2f Modulate-light)

This index is the authoritative status of every plan and design spec under
`docs/superpowers/`. **Agents: consult this file before following any plan or
design doc.** Each individual plan/spec also carries a status banner at the top,
but if the two disagree, this index wins.

**Status legend**

- **IMPLEMENTED** — merged to master; live in the codebase. The doc is kept for
  historical context. The code is now the source of truth, not the plan.
- **IN PROGRESS** — actively being executed task-by-task.
- **DEFERRED** — approved but intentionally not started yet. Still valid as a
  design reference.
- **SUPERSEDED** — never landed as written; a later plan/architecture replaced
  the approach. Do **not** follow these plans.

---

## Plans (`docs/superpowers/plans/`)

| Plan | Status | Notes |
|------|--------|-------|
| 2026-04-12-spectral-forge.md | IMPLEMENTED | Foundational plan. Single-engine `Pipeline` was later replaced by the modular `FxMatrix`; treat this plan as historical. |
| 2026-04-14-effects-engines.md | SUPERSEDED | Effects (Freeze, PhaseRand, Contrast) originally shipped as a fixed post-compressor stage. They now live as independent `SpectralModule` implementations in the FxMatrix. |
| 2026-04-14-gui-tabs-skeleton.md | SUPERSEDED | The Dynamics/Effects/Harmonic tab bar was removed when the per-slot module UI landed in plan D2. |
| 2026-04-19-modular-matrix.md | IMPLEMENTED | Infrastructure for the 8×8 FxMatrix + Dynamics slot. |
| 2026-04-19-plan-d1-module-foundation.md | IMPLEMENTED | `SpectralModule` trait, `RouteMatrix`, `apply_curve_transform`, module stubs. |
| 2026-04-19-serial-fx-chain.md | SUPERSEDED | Bridge plan to serialise Dynamics → Freeze → PhaseSmear via bool flags. Skipped in favour of the full FxMatrix approach. Do not follow. |
| 2026-04-19-variable-fft.md | IMPLEMENTED | Runtime-selectable FFT size (512 … 16384). |
| 2026-04-20-d2-ux-modules.md | IMPLEMENTED | Module popup, adaptive curve editor, per-slot SC/GainMode/name, matrix routing, T/S virtual rows, M/S DSP. |
| 2026-04-20-functional-module-bugs.md | IMPLEMENTED | Contrast / T/S Split / M/S Split / Freeze / silent-master fixes. |
| 2026-04-21-automation-presets.md | IMPLEMENTED | Generated automatable params (~1341), 1000 ms tooltips, JSON preset system. |
| 2026-04-21-bin-physics-infrastructure.md | DEFERRED | No `BinPhysics` code exists yet. Required for the Circuit/Life/Kinetics/Harmony/Modulate/Past/Rhythm module specs. |
| 2026-04-21-sidechain-refactor-plan.md | IMPLEMENTED | Single SC port + per-slot gain/channel selector; peak-hold curve; Freeze default threshold fix. |
| 2026-04-23-ui-parameter-spec.md | IMPLEMENTED | `CurveDisplayConfig`, curvature transform, shared hover routine, UI scaling. |
| 2026-04-24-calibration-audit.md | IMPLEMENTED | All 11 tasks merged: per-module ProbeSnapshot + calibration round-trip tests, Dynamics/Freeze DSP clamp fixes, 10 kHz curve cutoff fix, hover-tooltip flip, Freeze row layout fix, UI spec addenda §2.3/§3.4/§4.4. |
| 2026-04-24-ui-spec-cleanup.md | IMPLEMENTED | Closed the 2026-04-24 spec-deviation review. |
| 2026-04-27-phase-1-foundation-infra.md | IMPLEMENTED | All 8 PRs landed. ModuleContext borrowed, ModuleSpec hints (`wants_sidechain`, `panel_widget`, `heavy_cpu_for_mode`), `enable_heavy_modules` toggle, Reset-to-default button + `clear_state` plumbing, modulation-ring scaffold (toggles disabled until BPM sync). Unblocks every Phase 2+ sub-plan. |
| 2026-04-27-phase-2a-matrix-amp-nodes.md | IMPLEMENTED | All 12 tasks merged: AmpMode/AmpCellParams + per-mode kernels (Vactrol/Schmitt/Slew/Stiction), RouteMatrix amp_mode + amp_params with serde-default, lazy-allocated FxMatrix amp_state, per-cell apply at all three accumulation sites in process_hop, theme dot constants + popup + matrix-cell indicator/right-click, calibration probe behind `feature = "probe"`, finite/bounded e2e test. |
| 2026-04-27-phase-2b-future.md | IMPLEMENTED | All 9 tasks merged: FutureModule + PrintThrough/PreEcho kernels, per-channel ring (`MAX_ECHO_FRAMES = 64`), two-pass spread with phase-preserving `spread_scratch`, PreEcho feedback capped at 0.4 for closed-loop stability, per-slot `slot_future_mode` persistence + UI mode picker, ASSIGNABLE entry, calibration probes for both modes. Lookahead Duck + Crystal Ball deferred per audit. |
| 2026-04-27-phase-2c-punch.md | IMPLEMENTED | All 10 tasks merged: PunchModule + Direct/Inverse modes sharing one carve kernel (Inverse uses dedicated `peak_scratch`, never clobbers the smoothing follower), greedy local-max peak detector with `min_dist` veto + 256-slot on-stack scratch, 5 ms attack / HEAL-controlled release follower with τ=150 ms nominal + neighbour amp-fill, sub-bin pitch-fill via slew-limited `drift_accum` clamped to ±0.5 bins (phase rotation `Δφ = (π/2)·d` at OVERLAP=4), per-slot `slot_punch_mode` persistence (mirrors `slot_future_mode`), unified Mode-row UI (Future + Punch in a single visible-when-applicable row), calibration probes mode-agnostic. First module to default `wants_sidechain: true`. Self-Punch deferred per audit. |
| 2026-04-27-phase-2d-rhythm.md | IMPLEMENTED | All 11 tasks merged: Pipeline now populates `ctx.bpm` + `ctx.beat_position` from `Transport`. RhythmModule with Euclidean (Bjorklund into `[bool; 32]` on-stack scratch, DIVISION quantised to {1,2,4,8,16,32} steps), Arpeggiator (greedy peak picker on step crossings, 8×8 `ArpGrid` voice gating, per-voice envelope with `attack_step_frac` ramps), Phase Reset / Laser (per-step transient phase overwrite blending toward TARGET_PHASE; DC + Nyquist hard-skipped to keep realfft inverse well-defined). Per-slot mode + grid persistence via `Arc<Mutex<[T; 9]>>` (mirrors `slot_future_mode` / `slot_punch_mode`, NOT the plan's per-slot `[Mutex<T>; MAX_SLOTS]`); audio-thread dispatch is `try_lock`-once via `FxMatrix::set_rhythm_modes_and_grids`. `PanelWidgetFn` widened to `fn(&mut Ui, &SpectralForgeParams, slot)` so the 8×8 step-grid widget at `editor/rhythm_panel.rs` can read params; cell colours come from `module_spec(...).color_lit/.color_dim`, strokes via `th::scaled_stroke(th::STROKE_HAIRLINE, scale)` for UI scaling. UI Mode-row extended to a third module type using themed buttons (NOT the plan's `ComboBox`). 9 calibration probes (3 modes × {AMOUNT default/max, MIX max}) via a `run_rhythm_case` helper that sets `ctx.bpm = 120.0`, `beat_position = 0.0` (step_idx=0, where Bjorklund E(4,8) and E(8,8) both pulse). Bin Swing deferred (Spectral Delay infra); NoteIn-trigger deferred (Phase 6.3 MIDI). |
| 2026-04-27-phase-2e-geometry-light.md | IMPLEMENTED | All 10 tasks merged: GeometryModule with Chladni Plate Nodes (two-pass eigenmode kernel, 5%/hop AMOUNT cap, ~5% energy conservation) and Helmholtz Traps (8 fixed log-spaced traps + soft notch + overflow → 2nd-harmonic overtone re-injection). Center re-injection deliberately omitted (lies inside absorption band → leaky notch). Two safety nets in `apply_helmholtz` to bound runaway: `fill_level.min(2.0 * trigger)` and overtone-bin magnitude clamp at 1000.0 — added by Task 8 after the e2e test exposed orphan-overtone accumulation (e.g. bin 98 = 2×trap-4-center, outside every trap's band → reaches 1e6 norm by ~hop 106 under sustained excitation). Per-slot mode persistence via `Arc<Mutex<[GeometryMode; 9]>>` (matches `slot_future_mode` family, NOT the plan's `[Arc<Mutex<T>>; MAX_SLOTS]`); FxMatrix dispatch via `set_geometry_modes` + `set_geometry_mode` trait method. UI Mode-row extended to a fourth module type using themed Chladni/Helmholtz buttons (NOT the plan's separate `geometry_popup.rs`). Calibration uses shared `ProbeSnapshot { amount_pct, mix_pct }` (NOT the plan's bespoke `GeometryProbe`); 6 probes (2 modes × {AMOUNT default/max, MIX max}). 11 dedicated tests in `tests/geometry.rs`. Wavefield + Persistent Homology defer to Phase 7 (need SIMD wave kernel + History Buffer). |
| 2026-04-27-phase-2f-modulate-light.md | IMPLEMENTED | All 11 tasks merged: ModulateModule + 5 light kernels (Phase Phaser with hop_count rotation + AmpGate, Bin Swapper with scratch-buffer snapshot then offset blend, RM/FM Matrix blending sidechain-magnitude RM vs phase-rotation FM, Diode RM with amplitude-gated carrier leak, Ground Loop with 16-frame RMS history ring + 50/60Hz mains-hum injection). Per-channel state: `hop_count[2]`, `swap_scratch[2]: Vec<Complex<f32>>`, `rms_history[2][16]`. Per-slot mode persistence via `Arc<Mutex<[ModulateMode; 9]>>` (matches `slot_geometry_mode` family); FxMatrix dispatch via `set_modulate_modes`. UI Mode-row extended to a fifth module type using themed buttons (Phase Phaser / Bin Swapper / RM/FM / Diode RM / Ground Loop). Calibration uses shared `ProbeSnapshot { amount_pct, mix_pct }`; 15 probes (5 modes × {AMOUNT default/max, MIX max}) plus 16 dedicated kernel tests in `tests/modulate.rs`. `wants_sidechain: true` so RM/FM and Diode RM auto-route Sc(0) on assignment. Defers Gravity Phaser / PLL Tear / FM Network / Slew Lag to Phase 5/6. |

## Design specs (`docs/superpowers/specs/`)

| Spec | Status | Notes |
|------|--------|-------|
| 2026-04-12-spectral-forge-design.md | IMPLEMENTED | Foundational; superseded in places by the modular architecture spec. |
| 2026-04-19-modular-architecture-design.md | IMPLEMENTED | Authoritative for slot/matrix/module architecture. |
| 2026-04-21-automation-presets-design.md | IMPLEMENTED | |
| 2026-04-21-bin-physics-infrastructure.md | DEFERRED | Blocks the seven physics-driven module specs below. |
| 2026-04-21-circuit-module.md | DEFERRED | Depends on BinPhysics. |
| 2026-04-21-dynamic-automation-naming.md | DEFERRED | Explicitly deferred at design time. |
| 2026-04-21-harmony-module.md | DEFERRED | Depends on BinPhysics + instantaneous-frequency infra. |
| 2026-04-21-kinetics-module.md | DEFERRED | Depends on BinPhysics. |
| 2026-04-21-life-module.md | DEFERRED | Depends on BinPhysics. |
| 2026-04-21-matrix-amp-nodes.md | IMPLEMENTED | Implemented by Phase 2a plan `2026-04-27-phase-2a-matrix-amp-nodes.md`. |
| 2026-04-21-modulate-module.md | DEFERRED | Depends on BinPhysics + instantaneous-frequency infra. |
| 2026-04-21-past-module.md | DEFERRED | Depends on BinPhysics + history-buffer infra. |
| 2026-04-21-rhythm-module.md | DEFERRED | Depends on BinPhysics + host-BPM-sync infra. |
| 2026-04-21-sidechain-refactor-design.md | IMPLEMENTED | |
| 2026-04-23-ui-parameter-spec-design.md | IMPLEMENTED | **Authoritative source of truth for UI parameter display.** Includes addenda §2.3 / §3.4 / §4.4 merged from calibration-audit T10. |
| 2026-04-24-calibration-audit-design.md | IMPLEMENTED | Paired with plan `2026-04-24-calibration-audit.md`; all 11 tasks merged. |

---

## Rules for editing this file

- When a plan merges to master, flip its row to **IMPLEMENTED** and update the
  matching banner at the top of the plan file.
- When a plan is abandoned, mark it **SUPERSEDED** and name the replacement.
- Never delete a plan or spec — add a status line and leave the content in place
  so later agents can read the history.
- Keep the table entries to one line; put longer context in the individual
  plan/spec banner.
