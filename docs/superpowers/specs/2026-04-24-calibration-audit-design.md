> **Status (2026-04-24): IN PROGRESS.** Paired with plan `../plans/2026-04-24-calibration-audit.md`. T1–T3 merged; T4–T11 pending. Source of truth: [../STATUS.md](../STATUS.md).

# Calibration Audit — Design

**Status:** approved for planning
**Date:** 2026-04-24
**Scope:** audit every per-curve `offset_fn` end-to-end (config → audio thread → module-internal parameter), fix the DSP sites that don't respect the declared range, fix the 10 kHz curve cutoff, add spec addenda that codify the contract.

**Out of scope (deferred):** range retunes (freeze portamento minimum, ratio-as-bypass design question), new safety features (bin soft-clip, freeze fadeout/decay), drag-based graph interactions.

---

## Problem

The T3 per-curve offset calibration established that `offset = ±1` should drive the displayed physical value to `[y_min, y_max]` for every curve. The T3 review verified the `offset_fn` math but did **not** verify that modules' internal DSP respects the range the config promises. Three concrete bugs were reported:

- **Ratio:** `off_ratio(1.0, +1.0) = 20.0` but the Dynamics module reaches only 2:1 compression. DSP is clamping at 2.0.
- **Freeze length:** `off_freeze_length(1.0, +1.0) = 4000.0` but offset=+1 yields 62.5 ms (= 500 / 8). Sign inverted somewhere.
- **Freeze row layout:** Offset/Tilt/Curve controls render at a different vertical position for Freeze than for Dynamics.

Additionally: at 44.1 kHz and 1× UI scale, the response curve polyline truncates at ~10 kHz instead of extending to Nyquist.

The root cause of the first two is that the spec and its implementation can drift silently — no test forces the module's internal parameter to match the declared `y_max`/`y_min`. This design closes that gap.

## Goal

A parameterized regression test that asserts, for every `(ModuleType, curve_idx, gain_mode)` combination with a non-identity `offset_fn`, that feeding the extreme-calibrated gain into `module.process()` causes the module to internally observe a parameter matching the config's `y_min` / `y_max` within tolerance.

The test stays in the repo as a permanent guardrail against future calibration drift.

## Architecture

### Test location

`tests/calibration_roundtrip.rs` (new integration test file, uses the `rlib` crate target like the other tests).

### Calibration probe

Each `SpectralModule` gains a `#[cfg(test)]` probe field that records the last-computed internal parameters from a `process()` call. Shape:

```rust
#[cfg(test)]
#[derive(Clone, Copy, Debug, Default)]
pub struct ProbeSnapshot {
    pub threshold_db:  Option<f32>,
    pub ratio:         Option<f32>,
    pub attack_ms:     Option<f32>,
    pub release_ms:    Option<f32>,
    pub knee_db:       Option<f32>,
    pub mix_pct:       Option<f32>,
    pub length_ms:     Option<f32>,
    pub portamento_ms: Option<f32>,
    pub resistance:    Option<f32>,
    pub amount_pct:    Option<f32>,
    pub gain_db:       Option<f32>,
    pub gain_pct:      Option<f32>,
}
```

All fields are `Option<f32>` so a module only populates the parameters it actually derives from curves. Modules expose:

```rust
#[cfg(test)]
fn last_probe(&self) -> ProbeSnapshot;
```

Default trait implementation returns `ProbeSnapshot::default()`; each module overrides for its own curves. The field is populated near the end of `process()` with the value computed for a single reference bin (bin `num_bins / 2`, chosen so the test exercises a non-degenerate frequency and any per-bin freq-scaling that applies), **only when `cfg(test)` is active**. Test cases fill the entire curves buffer with the extreme gain, so the reference bin's value equals the all-bins value modulo any freq-dependent scaling that the test tolerance accommodates. Zero cost in release builds.

### Test structure

One parameterized function iterates a static table:

```rust
struct CalibrationCase {
    module_type: ModuleType,
    curve_idx:   usize,
    gain_mode:   GainMode,
    field:       fn(&ProbeSnapshot) -> Option<f32>,
    expect_min:  f32,  // == config y_min
    expect_max:  f32,  // == config y_max
    tolerance:   f32,  // per-case (e.g. 0.1 for dB, 1.0 for ms, relative for log)
}
```

For each case:

1. `create_module(module_type)`.
2. For each of `+1.0` and `-1.0` offset:
   - Compute `g = offset_fn(1.0, offset)` from the config.
   - Build a curves vector filled with `g` for the target curve, `1.0` for others.
   - Call `module.process()` with a dummy bins buffer and this curves slice.
   - Read `field(&module.last_probe())`.
   - Assert the field is within `tolerance` of `expect_max` (for +1) or `expect_min` (for -1).

Cases are listed explicitly in the test table — NOT auto-generated from `curve_display_config()` — so that silent config changes break a test rather than invisibly shifting what's being verified.

### Neutral-contract sub-test

Separately, a small unit test asserts `offset_fn(g, 0.0) == g` for `g ∈ {0.1, 0.5, 1.0, 2.0, 10.0}` across all offset functions. This is cheap insurance and independent of DSP.

## Bug fixes carried in the same plan

The audit test is the oracle. As each case fails, we fix it. Known-failing cases at plan start:

- **Dynamics ratio:** Internal clamp at 2.0 is wrong; must match `y_max = 20.0`.
- **Freeze length:** Sign or operator inversion (62.5 instead of 4000 at offset=+1).
- **Freeze row layout:** not caught by calibration test — handled as a separate direct fix in `editor_ui.rs`. Verify the Offset/Tilt/Curve row is rendered from the same code path regardless of module type.
- **10 kHz curve cutoff:** not caught by calibration test — handled as a separate direct fix. Hunt for hardcoded upper frequency bounds in `editor/curve.rs` and `editor_ui.rs` polyline loops. Add a GUI unit test that asserts the last polyline point's x-coordinate reaches the Nyquist grid position.

The audit test may also surface latent bugs in Freeze threshold, PhaseSmear, Contrast, MidSide, TransientSustainedSplit, or Gain — those are fixed inline when the test flags them.

## Spec addenda

Three new subsections appended to `docs/superpowers/specs/2026-04-23-ui-parameter-spec-design.md`:

### §2.3 Calibration contract

Every module's internal DSP must accept the full range implied by its curve's declared `offset_fn` extremes. When offset = +1, the DSP-observed parameter must reach `y_max`; when offset = -1, it must reach `y_min`. If a module clamps for safety, the clamp values MUST match `y_min` / `y_max`. Any tighter clamp is a bug.

This contract is verified by `tests/calibration_roundtrip.rs`. New modules MUST add themselves to that test's case table.

### §3.4 Node and curve rendering at limits

- Curve values outside `[y_min, y_max]` are rendered as a flat line along the exceeded border (top or bottom), not omitted.
- Curve nodes whose computed y-position is outside the graph are drawn truncated to the border with the dot still visible.
- When a node is being dragged, its virtual (un-clipped) physical value is shown in the hover tooltip.
- Each curve config declares its allowed `[y_min, y_max]`; the UI renderer is the sole place that enforces the visual clip.

### §4.4 Control row consistency

The Offset / Tilt / Curve DragValue row is rendered at a fixed vertical position per slot, identical across all module types. Modules may not define their own layout for these controls. The row comes from one shared widget in `editor_ui.rs`.

## Components touched

| File | Change |
|------|--------|
| `tests/calibration_roundtrip.rs` | New test file. |
| `src/dsp/modules/mod.rs` | Add `#[cfg(test)] ProbeSnapshot` + default `last_probe()` on `SpectralModule`. |
| `src/dsp/modules/dynamics.rs` | Implement `last_probe`, fix ratio clamp. |
| `src/dsp/modules/freeze.rs` | Implement `last_probe`, fix length sign. |
| `src/dsp/modules/phase_smear.rs`, `contrast.rs`, `gain.rs`, `mid_side.rs`, `ts_split.rs` | Implement `last_probe` for each curve they consume. |
| `src/editor/curve.rs`, `src/editor_ui.rs` | Fix 10 kHz cutoff; unify Freeze control row. |
| `docs/superpowers/specs/2026-04-23-ui-parameter-spec-design.md` | Append §2.3, §3.4, §4.4. |

## Real-time safety

`last_probe` writes are guarded by `#[cfg(test)]` and the field itself is `#[cfg(test)]`. The release build has no probe field and no probe writes — the module struct layout and `process()` hot path are identical to today.

## Done criteria

- `cargo test` passes (existing 86 tests + new calibration cases + neutral-contract cases + GUI polyline-extent test).
- `cargo build --release` clean (no warnings, no new allocations).
- Manual smoke test in Bitwig at 44.1 kHz: response curve polyline reaches the right edge of the graph; Freeze offset slider at +1 sets 4 s length; Dynamics ratio offset at +1 reaches 20:1; Freeze Offset/Tilt/Curve row aligned with Dynamics row.
- Spec addenda committed.
