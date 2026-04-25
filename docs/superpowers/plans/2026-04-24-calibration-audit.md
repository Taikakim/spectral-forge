> **Status (2026-04-24): IN PROGRESS.** T1 (probe type) + T2 (Dynamics probe + threshold clamp) + T3 (Freeze probe + formula/y_min fixes) are merged. T4–T11 remain. Source of truth: this plan (for remaining tasks) + [../STATUS.md](../STATUS.md).

# Calibration Audit Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** End-to-end audit that every per-curve `offset_fn`'s declared ±1 → [y_min, y_max] range is actually respected by the module-internal DSP, with a permanent regression test. Fix the known calibration bugs (ratio clamp, freeze length sign), the 10 kHz curve cutoff, and the Freeze control row layout. Append three subsections to the UI spec.

**Architecture:** Add a `#[cfg(test)] ProbeSnapshot` field and trait method to `SpectralModule`. Every module writes its bin-`num_bins/2` computed internal parameters to the probe in its `process()` under `cfg(test)`. A new integration test iterates a static `(module_type, curve_idx, gain_mode)` case table, feeds `offset_fn(1.0, ±1.0)` into the module, and asserts the probe reads back `[y_min, y_max]` within per-case tolerance. Zero cost in release.

**Tech Stack:** Rust, nih-plug, egui (for the GUI fixes), existing `realfft`/`triple_buffer` infrastructure.

**Spec:** `docs/superpowers/specs/2026-04-24-calibration-audit-design.md`

---

## File Structure

| File | Role in this plan |
|------|-------------------|
| `src/dsp/modules/mod.rs` | Add `ProbeSnapshot` struct (cfg-gated), `last_probe()` trait method with default impl. |
| `src/dsp/modules/dynamics.rs` | Implement `last_probe`, populate in `process()`, fix ratio clamp. |
| `src/dsp/modules/freeze.rs` | Implement `last_probe`, fix length sign inversion. |
| `src/dsp/modules/phase_smear.rs` | Implement `last_probe`. |
| `src/dsp/modules/contrast.rs` | Implement `last_probe`. |
| `src/dsp/modules/gain.rs` | Implement `last_probe`. |
| `src/dsp/modules/mid_side.rs` | Implement `last_probe`. |
| `src/dsp/modules/ts_split.rs` | Implement `last_probe`. |
| `tests/calibration_roundtrip.rs` | New integration test file; parameterized round-trip + neutral-contract tests. |
| `src/editor/curve.rs` / `src/editor_ui.rs` | Fix 10 kHz curve cutoff; unify Freeze control row. |
| `docs/superpowers/specs/2026-04-23-ui-parameter-spec-design.md` | Append §2.3, §3.4, §4.4. |

---

## Task 1: ProbeSnapshot type + trait method

**Files:**
- Modify: `src/dsp/modules/mod.rs`

- [ ] **Step 1: Add `ProbeSnapshot` and trait method**

In `src/dsp/modules/mod.rs`, add directly above the `pub trait SpectralModule` declaration:

```rust
/// Test-only snapshot of the last set of internal parameters a module derived
/// from its curves. Populated in `process()` when `cfg(test)` is active; zero
/// cost in release builds. Used by `tests/calibration_roundtrip.rs` to verify
/// every offset_fn's ±1 → [y_min, y_max] claim is respected end-to-end.
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
    pub balance_pct:   Option<f32>,
    pub expansion_pct: Option<f32>,
    pub decorrel_pct:  Option<f32>,
    pub transient_pct: Option<f32>,
    pub pan_pct:       Option<f32>,
    pub sensitivity_pct: Option<f32>,
    pub peak_hold_ms:  Option<f32>,
}
```

Inside the `pub trait SpectralModule` block, immediately after `fn num_outputs(&self) -> Option<usize> { None }`, add:

```rust
    /// Test-only: return the last set of internal parameters computed during
    /// `process()`. Default implementation returns an empty snapshot.
    /// See `tests/calibration_roundtrip.rs`.
    #[cfg(test)]
    fn last_probe(&self) -> ProbeSnapshot { ProbeSnapshot::default() }
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo build`
Expected: compiles cleanly, no warnings.

Run: `cargo test --no-run`
Expected: test artifacts build.

- [ ] **Step 3: Commit**

```bash
git add src/dsp/modules/mod.rs
git commit -m "feat(modules): add cfg(test) ProbeSnapshot + last_probe()

Foundation for the calibration audit test. Zero cost in release;
#[cfg(test)] field and trait method."
```

---

## Task 2: Dynamics probe + round-trip test + ratio clamp fix

**Files:**
- Modify: `src/dsp/modules/dynamics.rs`
- Create: `tests/calibration_roundtrip.rs`

- [ ] **Step 1: Add `#[cfg(test)] last_probe` field to `DynamicsModule`**

At the top of `src/dsp/modules/dynamics.rs`, find the struct definition. Add inside the struct body (after existing fields):

```rust
    #[cfg(test)]
    last_probe: crate::dsp::modules::ProbeSnapshot,
```

And inside `Default::default()` (if present) or `DynamicsModule::new()`, add the initialization:

```rust
    #[cfg(test)]
    last_probe: Default::default(),
```

- [ ] **Step 2: Populate `last_probe` at end of `process()`**

At the very end of `DynamicsModule::process()` (after the engine runs and fills `suppression_out`), add:

```rust
        #[cfg(test)]
        {
            let k = self.num_bins / 2;
            self.last_probe = crate::dsp::modules::ProbeSnapshot {
                threshold_db: Some(self.bp_threshold[k]),
                ratio:        Some(self.bp_ratio[k]),
                attack_ms:    Some(self.bp_attack[k]),
                release_ms:   Some(self.bp_release[k]),
                knee_db:      Some(self.bp_knee[k]),
                mix_pct:      Some(self.bp_mix[k] * 100.0),
                ..Default::default()
            };
        }
```

- [ ] **Step 3: Override `last_probe()` in the trait impl**

In the `impl SpectralModule for DynamicsModule` block, add:

```rust
    #[cfg(test)]
    fn last_probe(&self) -> crate::dsp::modules::ProbeSnapshot { self.last_probe }
```

- [ ] **Step 4: Create `tests/calibration_roundtrip.rs` with Dynamics cases**

Create the file with this content:

```rust
//! Calibration audit — every per-curve offset_fn's declared ±1 → [y_min, y_max]
//! range is verified end-to-end through the module's DSP.
//! See docs/superpowers/specs/2026-04-24-calibration-audit-design.md

use num_complex::Complex;
use spectral_forge::dsp::modules::{
    create_module, GainMode, ModuleContext, ModuleType, ProbeSnapshot, SpectralModule,
};
use spectral_forge::editor::curve_config::curve_display_config;
use spectral_forge::params::{FxChannelTarget, StereoLink};

const FFT_SIZE: usize = 2048;
const NUM_BINS: usize = FFT_SIZE / 2 + 1;
const SAMPLE_RATE: f32 = 48_000.0;

fn make_ctx() -> ModuleContext {
    ModuleContext {
        sample_rate: SAMPLE_RATE,
        fft_size: FFT_SIZE,
        num_bins: NUM_BINS,
        attack_ms: 10.0,
        release_ms: 100.0,
        sensitivity: 0.5,
        suppression_width: 0.0,
        auto_makeup: false,
        delta_monitor: false,
    }
}

/// Run the module with every curve filled with `gain_on_target` on
/// `target_curve_idx` and 1.0 on all other curves. Returns the probe.
fn run_case(
    module: &mut Box<dyn SpectralModule>,
    num_curves: usize,
    target_curve_idx: usize,
    gain_on_target: f32,
) -> ProbeSnapshot {
    let curves_storage: Vec<Vec<f32>> = (0..num_curves)
        .map(|c| if c == target_curve_idx {
            vec![gain_on_target; NUM_BINS]
        } else {
            vec![1.0; NUM_BINS]
        })
        .collect();
    let curves_refs: Vec<&[f32]> = curves_storage.iter().map(|v| v.as_slice()).collect();

    let mut bins: Vec<Complex<f32>> = vec![Complex::new(0.1, 0.0); NUM_BINS];
    let mut suppression: Vec<f32> = vec![0.0; NUM_BINS];
    let ctx = make_ctx();
    module.process(
        0,
        StereoLink::Linked,
        FxChannelTarget::All,
        &mut bins,
        None,
        &curves_refs,
        &mut suppression,
        &ctx,
    );
    module.last_probe()
}

#[test]
fn dynamics_threshold_offset_plus_one_hits_y_max() {
    let mut m = create_module(ModuleType::Dynamics);
    m.reset(SAMPLE_RATE, FFT_SIZE);
    let cfg = curve_display_config(ModuleType::Dynamics, 0, GainMode::Add);
    let g = (cfg.offset_fn)(1.0, 1.0);
    let probe = run_case(&mut m, m.num_curves(), 0, g);
    let observed = probe.threshold_db.expect("dynamics must probe threshold");
    assert!(
        (observed - cfg.y_max).abs() < 0.5,
        "offset=+1 should give threshold≈{}, got {}", cfg.y_max, observed,
    );
}

#[test]
fn dynamics_threshold_offset_minus_one_hits_y_min() {
    let mut m = create_module(ModuleType::Dynamics);
    m.reset(SAMPLE_RATE, FFT_SIZE);
    let cfg = curve_display_config(ModuleType::Dynamics, 0, GainMode::Add);
    let g = (cfg.offset_fn)(1.0, -1.0);
    let probe = run_case(&mut m, m.num_curves(), 0, g);
    let observed = probe.threshold_db.expect("dynamics must probe threshold");
    assert!(
        (observed - cfg.y_min).abs() < 0.5,
        "offset=-1 should give threshold≈{}, got {}", cfg.y_min, observed,
    );
}

#[test]
fn dynamics_ratio_offset_plus_one_hits_y_max() {
    let mut m = create_module(ModuleType::Dynamics);
    m.reset(SAMPLE_RATE, FFT_SIZE);
    let cfg = curve_display_config(ModuleType::Dynamics, 1, GainMode::Add);
    let g = (cfg.offset_fn)(1.0, 1.0);
    let probe = run_case(&mut m, m.num_curves(), 1, g);
    let observed = probe.ratio.expect("dynamics must probe ratio");
    assert!(
        (observed - cfg.y_max).abs() < 0.1,
        "offset=+1 should give ratio≈{}, got {}", cfg.y_max, observed,
    );
}

#[test]
fn dynamics_ratio_offset_minus_one_hits_y_min() {
    let mut m = create_module(ModuleType::Dynamics);
    m.reset(SAMPLE_RATE, FFT_SIZE);
    let cfg = curve_display_config(ModuleType::Dynamics, 1, GainMode::Add);
    let g = (cfg.offset_fn)(1.0, -1.0);
    let probe = run_case(&mut m, m.num_curves(), 1, g);
    let observed = probe.ratio.expect("dynamics must probe ratio");
    assert!(
        (observed - cfg.y_min).abs() < 0.1,
        "offset=-1 should give ratio≈{}, got {}", cfg.y_min, observed,
    );
}

#[test]
fn dynamics_knee_offset_extremes() {
    let mut m = create_module(ModuleType::Dynamics);
    m.reset(SAMPLE_RATE, FFT_SIZE);
    let cfg = curve_display_config(ModuleType::Dynamics, 4, GainMode::Add);
    let g_hi = (cfg.offset_fn)(1.0, 1.0);
    let probe = run_case(&mut m, m.num_curves(), 4, g_hi);
    let observed = probe.knee_db.unwrap();
    assert!((observed - cfg.y_max).abs() < 0.5, "knee hi: want {}, got {}", cfg.y_max, observed);

    let g_lo = (cfg.offset_fn)(1.0, -1.0);
    let probe = run_case(&mut m, m.num_curves(), 4, g_lo);
    let observed = probe.knee_db.unwrap();
    assert!((observed - cfg.y_min).abs() < 0.5, "knee lo: want {}, got {}", cfg.y_min, observed);
}

#[test]
fn dynamics_mix_offset_extremes() {
    let mut m = create_module(ModuleType::Dynamics);
    m.reset(SAMPLE_RATE, FFT_SIZE);
    let cfg = curve_display_config(ModuleType::Dynamics, 5, GainMode::Add);
    let g_hi = (cfg.offset_fn)(1.0, 1.0);
    let probe = run_case(&mut m, m.num_curves(), 5, g_hi);
    assert!((probe.mix_pct.unwrap() - cfg.y_max).abs() < 1.0);

    let g_lo = (cfg.offset_fn)(1.0, -1.0);
    let probe = run_case(&mut m, m.num_curves(), 5, g_lo);
    assert!((probe.mix_pct.unwrap() - cfg.y_min).abs() < 1.0);
}

// Attack/Release are multiplicative with factor 1024. The module scales
// ctx.attack_ms (10 ms) by the curve gain and clamps. Verify scaling works.
#[test]
fn dynamics_attack_offset_plus_one_multiplies_global() {
    let mut m = create_module(ModuleType::Dynamics);
    m.reset(SAMPLE_RATE, FFT_SIZE);
    let cfg = curve_display_config(ModuleType::Dynamics, 2, GainMode::Add);
    let g = (cfg.offset_fn)(1.0, 1.0); // = 1024.0
    let probe = run_case(&mut m, m.num_curves(), 2, g);
    // ctx.attack_ms=10 × 1024 = 10240, clamped at 500 (pipeline limit) — so the
    // y_max of 1024 in the config is actually a display-only limit, not a DSP
    // limit. The test just asserts the attack reaches the 500 ms DSP clamp.
    let observed = probe.attack_ms.unwrap();
    assert!(observed >= 500.0 - 1.0, "attack should reach DSP clamp 500, got {}", observed);
}
```

- [ ] **Step 5: Run tests — the ratio test should fail (ratio clamp bug)**

Run: `cargo test --test calibration_roundtrip`

Expected: `dynamics_ratio_offset_plus_one_hits_y_max` fails with observed ratio far below 20.0 (current DSP limits it) OR passes if the pipeline clamp upstream also respects 20. `dynamics_threshold_offset_*` passes. Other tests establish baseline.

If ratio test passes: the bug is upstream of the module (in `apply_curve_transform` or the offset FloatParam range). Proceed to Step 6 regardless — either way we will fix whatever blocks the test.

- [ ] **Step 6: Investigate and fix the ratio failure**

Most likely causes, in order:

(a) `DynamicsModule::process` at `src/dsp/modules/dynamics.rs:100` reads `r = curves[1][k]` but the value arriving is pre-clamped to 2.0 upstream. Check `apply_curve_transform` in `src/dsp/modules/mod.rs` — does it clamp the gain after `offset_fn`?

(b) The offset FloatParam's declared range in `build.rs` is smaller than ±1.0 for ratio (e.g., ±0.5), so `off_ratio(1.0, 0.5) = 1 + 19*0.5 = 10.5` falls short. Grep `build.rs` for the offset param range.

(c) The compressor engine's internal ratio formula clamps the effective compression ratio (separate from `bp_ratio`).

Trace the path: `curves[1][k]` → `self.bp_ratio[k]` → `SpectralCompressorEngine::process_bins`. Whichever link clamps is the bug. Fix it so the declared y_max is respected.

If (b) is the cause, widen the FloatParam range to [-1.0, 1.0] in `build.rs` — this is the correct fix because the spec says offset is [-1, 1].

Commit the fix:

```bash
git add -A
git commit -m "fix(dynamics): ratio offset reaches declared y_max=20

<describe specific fix based on investigation>"
```

- [ ] **Step 7: Re-run the Dynamics tests, all must pass**

Run: `cargo test --test calibration_roundtrip dynamics_`
Expected: all 7 Dynamics tests pass.

- [ ] **Step 8: Commit the probe infrastructure**

```bash
git add src/dsp/modules/dynamics.rs tests/calibration_roundtrip.rs
git commit -m "test: calibration round-trip for Dynamics module

Probe records bin num_bins/2 computed params. Tests assert offset=±1
drives each Dynamics curve to its config's [y_min, y_max]."
```

---

## Task 3: Freeze probe + round-trip tests + length sign fix

**Files:**
- Modify: `src/dsp/modules/freeze.rs`
- Modify: `tests/calibration_roundtrip.rs`

- [ ] **Step 1: Add probe field to `FreezeModule`**

Open `src/dsp/modules/freeze.rs`, find the struct, add:

```rust
    #[cfg(test)]
    last_probe: crate::dsp::modules::ProbeSnapshot,
```

Initialize it in the module's constructor (search for `FreezeModule::new` or the `Default` impl).

- [ ] **Step 2: Populate the probe at end of `process()`**

At the bottom of `FreezeModule::process()`, inside a `#[cfg(test)]` block, record the values that were computed for bin `k = num_bins / 2`. The variables to capture are `length_ms`, `threshold_db`, `portamento_ms`, `resistance`, and `mix_pct`. The current code computes these per bin at `src/dsp/modules/freeze.rs:92-99` — snapshot the values at `k == num_bins/2`:

```rust
        // Before the loop:
        #[cfg(test)]
        let mut probe_length_ms:     f32 = 0.0;
        #[cfg(test)]
        let mut probe_threshold_db:  f32 = 0.0;
        #[cfg(test)]
        let mut probe_portamento_ms: f32 = 0.0;
        #[cfg(test)]
        let mut probe_resistance:    f32 = 0.0;
        #[cfg(test)]
        let mut probe_mix_pct:       f32 = 0.0;

        // Inside the loop, when k == num_bins / 2:
        #[cfg(test)]
        if k == self.num_bins / 2 {
            probe_length_ms     = length_ms;
            probe_threshold_db  = threshold_db;
            probe_portamento_ms = portamento_ms;    // use the actual local name
            probe_resistance    = resistance;        // use the actual local name
            probe_mix_pct       = mix * 100.0;       // use the actual local mix variable
        }

        // After the loop:
        #[cfg(test)]
        {
            self.last_probe = crate::dsp::modules::ProbeSnapshot {
                length_ms:     Some(probe_length_ms),
                threshold_db:  Some(probe_threshold_db),
                portamento_ms: Some(probe_portamento_ms),
                resistance:    Some(probe_resistance),
                mix_pct:       Some(probe_mix_pct),
                ..Default::default()
            };
        }
```

If the local variable names differ in the current file, adjust to whatever the actual per-bin computation produces. If a curve's parameter isn't computed per bin (e.g. portamento might be a single value, not per-bin), capture it once before the loop ends.

- [ ] **Step 3: Override `last_probe()` in `impl SpectralModule for FreezeModule`**

```rust
    #[cfg(test)]
    fn last_probe(&self) -> crate::dsp::modules::ProbeSnapshot { self.last_probe }
```

- [ ] **Step 4: Add Freeze tests to `tests/calibration_roundtrip.rs`**

Append to the end of the file:

```rust
#[test]
fn freeze_length_offset_plus_one_hits_y_max() {
    let mut m = create_module(ModuleType::Freeze);
    m.reset(SAMPLE_RATE, FFT_SIZE);
    let cfg = curve_display_config(ModuleType::Freeze, 0, GainMode::Add);
    let g = (cfg.offset_fn)(1.0, 1.0);
    let probe = run_case(&mut m, m.num_curves(), 0, g);
    let observed = probe.length_ms.expect("freeze must probe length");
    assert!(
        (observed - cfg.y_max).abs() < 50.0,
        "freeze length offset=+1 should give ≈{} ms, got {}", cfg.y_max, observed,
    );
}

#[test]
fn freeze_length_offset_minus_one_hits_y_min() {
    let mut m = create_module(ModuleType::Freeze);
    m.reset(SAMPLE_RATE, FFT_SIZE);
    let cfg = curve_display_config(ModuleType::Freeze, 0, GainMode::Add);
    let g = (cfg.offset_fn)(1.0, -1.0);
    let probe = run_case(&mut m, m.num_curves(), 0, g);
    let observed = probe.length_ms.expect("freeze must probe length");
    assert!(
        (observed - cfg.y_min).abs() < 5.0,
        "freeze length offset=-1 should give ≈{} ms, got {}", cfg.y_min, observed,
    );
}

#[test]
fn freeze_threshold_offset_extremes() {
    let mut m = create_module(ModuleType::Freeze);
    m.reset(SAMPLE_RATE, FFT_SIZE);
    let cfg = curve_display_config(ModuleType::Freeze, 1, GainMode::Add);

    let g_hi = (cfg.offset_fn)(1.0, 1.0);
    let probe = run_case(&mut m, m.num_curves(), 1, g_hi);
    assert!((probe.threshold_db.unwrap() - cfg.y_max).abs() < 1.0);

    let g_lo = (cfg.offset_fn)(1.0, -1.0);
    let probe = run_case(&mut m, m.num_curves(), 1, g_lo);
    assert!((probe.threshold_db.unwrap() - cfg.y_min).abs() < 1.0);
}

#[test]
fn freeze_portamento_offset_extremes() {
    let mut m = create_module(ModuleType::Freeze);
    m.reset(SAMPLE_RATE, FFT_SIZE);
    let cfg = curve_display_config(ModuleType::Freeze, 2, GainMode::Add);

    let g_hi = (cfg.offset_fn)(1.0, 1.0);
    let probe = run_case(&mut m, m.num_curves(), 2, g_hi);
    assert!((probe.portamento_ms.unwrap() - cfg.y_max).abs() < 5.0);

    let g_lo = (cfg.offset_fn)(1.0, -1.0);
    let probe = run_case(&mut m, m.num_curves(), 2, g_lo);
    assert!((probe.portamento_ms.unwrap() - cfg.y_min).abs() < 1.0);
}

#[test]
fn freeze_resistance_offset_extremes() {
    let mut m = create_module(ModuleType::Freeze);
    m.reset(SAMPLE_RATE, FFT_SIZE);
    let cfg = curve_display_config(ModuleType::Freeze, 3, GainMode::Add);

    let g_hi = (cfg.offset_fn)(1.0, 1.0);
    let probe = run_case(&mut m, m.num_curves(), 3, g_hi);
    assert!((probe.resistance.unwrap() - cfg.y_max).abs() < 0.05);

    let g_lo = (cfg.offset_fn)(1.0, -1.0);
    let probe = run_case(&mut m, m.num_curves(), 3, g_lo);
    assert!((probe.resistance.unwrap() - cfg.y_min).abs() < 0.05);
}

#[test]
fn freeze_mix_offset_extremes() {
    let mut m = create_module(ModuleType::Freeze);
    m.reset(SAMPLE_RATE, FFT_SIZE);
    let cfg = curve_display_config(ModuleType::Freeze, 4, GainMode::Add);

    let g_hi = (cfg.offset_fn)(1.0, 1.0);
    let probe = run_case(&mut m, m.num_curves(), 4, g_hi);
    assert!((probe.mix_pct.unwrap() - cfg.y_max).abs() < 1.0);

    let g_lo = (cfg.offset_fn)(1.0, -1.0);
    let probe = run_case(&mut m, m.num_curves(), 4, g_lo);
    assert!((probe.mix_pct.unwrap() - cfg.y_min).abs() < 1.0);
}
```

- [ ] **Step 5: Run tests — `freeze_length_*` must fail first**

Run: `cargo test --test calibration_roundtrip freeze_`
Expected: `freeze_length_offset_plus_one_hits_y_max` fails — observed ≈ 62.5 ms. Other freeze tests may also fail; note which.

- [ ] **Step 6: Investigate and fix the length sign inversion**

The flow is: offset FloatParam value (say +1.0) → `apply_curve_transform` calls `off_freeze_length(g, offset)` → multiplicative factor `g * 8.0^offset` → at offset=+1 should return g*8=8.0 → then multiplied by 500 in the module for 4000 ms.

If the test shows observed = 62.5, the factor is `8.0^(-1)`. Most likely causes:

(a) `apply_curve_transform` passes `-offset` instead of `offset` to `offset_fn`. Check `src/dsp/modules/mod.rs` `apply_curve_transform` implementation.

(b) The offset FloatParam is defined with inverted polarity in `build.rs` (some generated FloatParams have inverted ranges).

(c) The GUI-side offset DragValue writes -offset.

Grep for `apply_curve_transform` definition and trace. Fix so offset=+1 at the user-facing layer produces the multiplicative factor 8.0 at the consumer.

Commit:

```bash
git add -A
git commit -m "fix(freeze): length offset=+1 now reaches 4000 ms (was inverted)

<specifics>"
```

- [ ] **Step 7: Re-run, all Freeze tests must pass**

Run: `cargo test --test calibration_roundtrip freeze_`
Expected: all 6 freeze tests pass.

- [ ] **Step 8: Commit probe**

```bash
git add src/dsp/modules/freeze.rs tests/calibration_roundtrip.rs
git commit -m "test: calibration round-trip for Freeze module"
```

---

## Task 4: PhaseSmear + Contrast probes and tests

**Files:**
- Modify: `src/dsp/modules/phase_smear.rs`
- Modify: `src/dsp/modules/contrast.rs`
- Modify: `tests/calibration_roundtrip.rs`

- [ ] **Step 1: PhaseSmear probe**

Same pattern as Dynamics/Freeze. In `src/dsp/modules/phase_smear.rs`:

```rust
    #[cfg(test)]
    last_probe: crate::dsp::modules::ProbeSnapshot,
```

In `process()`, at bin `num_bins/2`, capture whatever local variables represent: AMOUNT (%), PEAK HOLD (ms), MIX (%). Snapshot into `last_probe`:

```rust
        #[cfg(test)]
        {
            self.last_probe = crate::dsp::modules::ProbeSnapshot {
                amount_pct:   Some(/* computed amount % at k=num_bins/2 */),
                peak_hold_ms: Some(/* computed peak hold ms */),
                mix_pct:      Some(/* computed mix % */),
                ..Default::default()
            };
        }
```

Override `last_probe()` in the trait impl.

- [ ] **Step 2: Contrast probe**

In `src/dsp/modules/contrast.rs`, the only curve is AMOUNT mapping to ratio. Capture as `ratio` in the probe. Same pattern.

- [ ] **Step 3: Add tests**

Append to `tests/calibration_roundtrip.rs`:

```rust
#[test]
fn phase_smear_amount_offset_extremes() {
    let mut m = create_module(ModuleType::PhaseSmear);
    m.reset(SAMPLE_RATE, FFT_SIZE);
    let cfg = curve_display_config(ModuleType::PhaseSmear, 0, GainMode::Add);

    let g_hi = (cfg.offset_fn)(1.0, 1.0);
    let probe = run_case(&mut m, m.num_curves(), 0, g_hi);
    assert!((probe.amount_pct.unwrap() - cfg.y_max).abs() < 2.0);

    let g_lo = (cfg.offset_fn)(1.0, -1.0);
    let probe = run_case(&mut m, m.num_curves(), 0, g_lo);
    assert!((probe.amount_pct.unwrap() - cfg.y_min).abs() < 2.0);
}

#[test]
fn phase_smear_peak_hold_offset_extremes() {
    let mut m = create_module(ModuleType::PhaseSmear);
    m.reset(SAMPLE_RATE, FFT_SIZE);
    let cfg = curve_display_config(ModuleType::PhaseSmear, 1, GainMode::Add);

    let g_hi = (cfg.offset_fn)(1.0, 1.0);
    let probe = run_case(&mut m, m.num_curves(), 1, g_hi);
    assert!((probe.peak_hold_ms.unwrap() - cfg.y_max).abs() < 5.0);

    let g_lo = (cfg.offset_fn)(1.0, -1.0);
    let probe = run_case(&mut m, m.num_curves(), 1, g_lo);
    assert!((probe.peak_hold_ms.unwrap() - cfg.y_min).abs() < 1.0);
}

#[test]
fn phase_smear_mix_offset_extremes() {
    let mut m = create_module(ModuleType::PhaseSmear);
    m.reset(SAMPLE_RATE, FFT_SIZE);
    let cfg = curve_display_config(ModuleType::PhaseSmear, 2, GainMode::Add);

    let g_hi = (cfg.offset_fn)(1.0, 1.0);
    let probe = run_case(&mut m, m.num_curves(), 2, g_hi);
    assert!((probe.mix_pct.unwrap() - cfg.y_max).abs() < 1.0);

    let g_lo = (cfg.offset_fn)(1.0, -1.0);
    let probe = run_case(&mut m, m.num_curves(), 2, g_lo);
    assert!((probe.mix_pct.unwrap() - cfg.y_min).abs() < 1.0);
}

#[test]
fn contrast_amount_offset_extremes() {
    let mut m = create_module(ModuleType::Contrast);
    m.reset(SAMPLE_RATE, FFT_SIZE);
    let cfg = curve_display_config(ModuleType::Contrast, 0, GainMode::Add);

    let g_hi = (cfg.offset_fn)(1.0, 1.0);
    let probe = run_case(&mut m, m.num_curves(), 0, g_hi);
    assert!((probe.ratio.unwrap() - cfg.y_max).abs() < 0.1);

    let g_lo = (cfg.offset_fn)(1.0, -1.0);
    let probe = run_case(&mut m, m.num_curves(), 0, g_lo);
    assert!((probe.ratio.unwrap() - cfg.y_min).abs() < 0.1);
}
```

- [ ] **Step 4: Run, investigate any failures**

Run: `cargo test --test calibration_roundtrip phase_smear_ contrast_`

For each failure, trace the same way as Tasks 2 and 3: is the DSP clamping tighter than y_min/y_max, or is the signal upstream distorted? Fix inline.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "test: calibration round-trip for PhaseSmear and Contrast"
```

---

## Task 5: Gain module probe + tests (all 4 modes)

**Files:**
- Modify: `src/dsp/modules/gain.rs`
- Modify: `tests/calibration_roundtrip.rs`

- [ ] **Step 1: Add probe to `GainModule`**

Same pattern. `GainModule` has one curve but four `GainMode` variants with different physical-unit mappings:
- `GainMode::Add` / `GainMode::Subtract` → dB (use `probe.gain_db`)
- `GainMode::Pull` / `GainMode::Match` → % (use `probe.gain_pct`)

Populate the correct probe field based on `self.gain_mode` at `k = num_bins/2`.

- [ ] **Step 2: Add tests**

```rust
#[test]
fn gain_add_offset_extremes() {
    let mut m = create_module(ModuleType::Gain);
    m.set_gain_mode(GainMode::Add);
    m.reset(SAMPLE_RATE, FFT_SIZE);
    let cfg = curve_display_config(ModuleType::Gain, 0, GainMode::Add);

    let g_hi = (cfg.offset_fn)(1.0, 1.0);
    let probe = run_case(&mut m, m.num_curves(), 0, g_hi);
    assert!((probe.gain_db.unwrap() - cfg.y_max).abs() < 0.5);

    let g_lo = (cfg.offset_fn)(1.0, -1.0);
    let probe = run_case(&mut m, m.num_curves(), 0, g_lo);
    assert!((probe.gain_db.unwrap() - cfg.y_min).abs() < 0.5);
}

#[test]
fn gain_pull_offset_extremes() {
    let mut m = create_module(ModuleType::Gain);
    m.set_gain_mode(GainMode::Pull);
    m.reset(SAMPLE_RATE, FFT_SIZE);
    let cfg = curve_display_config(ModuleType::Gain, 0, GainMode::Pull);

    let g_hi = (cfg.offset_fn)(1.0, 1.0);
    let probe = run_case(&mut m, m.num_curves(), 0, g_hi);
    assert!((probe.gain_pct.unwrap() - cfg.y_max).abs() < 1.0);

    let g_lo = (cfg.offset_fn)(1.0, -1.0);
    let probe = run_case(&mut m, m.num_curves(), 0, g_lo);
    assert!((probe.gain_pct.unwrap() - cfg.y_min).abs() < 1.0);
}
```

Subtract and Match mirror Add and Pull respectively. Skip them unless Step 3 reveals a difference.

- [ ] **Step 3: Run, fix any failures, commit**

```bash
cargo test --test calibration_roundtrip gain_
git add -A
git commit -m "test: calibration round-trip for Gain module"
```

---

## Task 6: MidSide + TS Split probes and tests

**Files:**
- Modify: `src/dsp/modules/mid_side.rs`
- Modify: `src/dsp/modules/ts_split.rs`
- Modify: `tests/calibration_roundtrip.rs`

- [ ] **Step 1: MidSide probe**

5 curves: BALANCE, EXPANSION, DECORREL, TRANSIENT, PAN (all as %). Map to `probe.balance_pct`, `probe.expansion_pct`, `probe.decorrel_pct`, `probe.transient_pct`, `probe.pan_pct` respectively at `k=num_bins/2`.

- [ ] **Step 2: TS Split probe**

1 curve: SENSITIVITY as %. Map to `probe.sensitivity_pct`.

- [ ] **Step 3: Add tests**

For each of the 5 MidSide curves and 1 TS Split curve, add `*_offset_extremes` tests with the corresponding probe field. Tolerance: `±2.0` for the 0–200% range curves, `±1.0` for 0–100%.

- [ ] **Step 4: Run, fix any failures, commit**

```bash
cargo test --test calibration_roundtrip mid_side_ ts_split_
git add -A
git commit -m "test: calibration round-trip for MidSide and TS Split"
```

---

## Task 7: Neutral-contract test

**Files:**
- Modify: `tests/calibration_roundtrip.rs`

- [ ] **Step 1: Add the test**

Append to the file:

```rust
#[test]
fn all_offset_fns_are_neutral_at_zero() {
    use spectral_forge::editor::curve_config::{
        off_thresh, off_ratio, off_atk_rel, off_knee, off_mix, off_gain_db,
        off_gain_pct, off_amount_200, off_freeze_length, off_freeze_thresh,
        off_portamento, off_resistance, off_identity,
    };
    let fns: &[(&str, fn(f32, f32) -> f32)] = &[
        ("thresh",        off_thresh),
        ("ratio",         off_ratio),
        ("atk_rel",       off_atk_rel),
        ("knee",          off_knee),
        ("mix",           off_mix),
        ("gain_db",       off_gain_db),
        ("gain_pct",      off_gain_pct),
        ("amount_200",    off_amount_200),
        ("freeze_length", off_freeze_length),
        ("freeze_thresh", off_freeze_thresh),
        ("portamento",    off_portamento),
        ("resistance",    off_resistance),
        ("identity",      off_identity),
    ];
    for (name, f) in fns {
        for &g in &[0.1_f32, 0.5, 1.0, 2.0, 10.0] {
            let result = f(g, 0.0);
            assert!(
                (result - g).abs() < 1e-5,
                "{} violates neutral contract: f({}, 0.0) = {}, expected {}", name, g, result, g,
            );
        }
    }
}
```

- [ ] **Step 2: Confirm `off_*` are pub**

Check `src/editor/curve_config.rs` — all `off_*` functions must already be `pub fn`. If any are not, make them pub.

- [ ] **Step 3: Run**

Run: `cargo test --test calibration_roundtrip all_offset_fns_are_neutral_at_zero`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add tests/calibration_roundtrip.rs
git commit -m "test: neutral-contract — every offset_fn satisfies f(g, 0) == g"
```

---

## Task 8: 10 kHz curve cutoff investigation and fix

**Files:**
- Read: `src/editor/curve.rs`
- Read: `src/editor_ui.rs`
- Possibly modify one or both

- [ ] **Step 1: Write the failing test**

Create `tests/curve_display_extent.rs`:

```rust
//! UI regression: response-curve polyline extends to Nyquist at the displayed
//! sample rate, not a hardcoded upper frequency.

use spectral_forge::editor::curve::{compute_curve_response, CurveNode};

#[test]
fn curve_response_spans_full_num_bins_at_44_1_khz() {
    // Flat curve: a single unity node. Result is a vec of length num_bins.
    let nodes: [CurveNode; 6] = Default::default();
    let sample_rate = 44_100.0_f32;
    let fft_size = 2048_usize;
    let num_bins = fft_size / 2 + 1;
    let gains = compute_curve_response(&nodes, num_bins, sample_rate, fft_size);
    assert_eq!(gains.len(), num_bins,
        "compute_curve_response must return num_bins samples ({})", num_bins);
    assert!(gains.iter().all(|g| g.is_finite()),
        "all gains must be finite");
}
```

This verifies the compute side. The drawing side (where the 10 kHz cutoff lives) is GUI-only and needs a visual/integration check — do that in Step 4.

- [ ] **Step 2: Run the test**

Run: `cargo test --test curve_display_extent`
Expected: PASS (compute function is likely fine — the bug is in drawing).

- [ ] **Step 3: Hunt the 10 kHz cutoff in the drawing code**

The polyline is painted by `paint_response_curve` in `src/editor/curve.rs`. Read the function (it iterates `0..n` where `n = gains.len()`). If the iteration or the x-mapping function truncates, this is the bug.

Check these suspects in `src/editor/curve.rs` and `src/editor_ui.rs`:

- **`x_to_screen()` at curve.rs:188-194**: uses `scale = 3.0 / (max_hz / 20.0).log10()`. At 44.1 kHz this is 3.0/log10(22050/20)=3.0/3.043=0.986. Curve node x=1 lands at 98.6% of rect width. Not a 10 kHz cutoff.

- **Node rendering**: the node shapes might be drawn at `x_to_screen(node.x, ...)` where node.x is clamped to 1.0 = 20 kHz. That's a node bug, not a polyline bug. But if the polyline uses node positions, that could truncate.

- **The painter's clip rect**: `ui.painter()` inherits a clip rect from the parent region. If the curve rect is narrower than expected, the polyline is clipped visually.

- **`freq_to_x_max` at curve.rs:199-204**: `max_hz.max(20_001.0)`. At 44.1 kHz, max_hz = 22050. Log math: log10(22050/20) = 3.043. This is fine.

- **`compute_curve_response` at curve.rs:163-181**: returns exactly `num_bins` gains. At fft_size=2048, num_bins=1025.

- **`num_bins` passed to `paint_response_curve`**: check `src/editor_ui.rs` call sites. `paint_response_curve(..., &all_gains[i], ...)`. If `all_gains[i].len() != num_bins`, that's the truncation.

- **Loop upper bound**: `for k in 0..n` where `n = gains.len()`. If `n` is passed as `num_bins / 2` somewhere, Nyquist halves — at fft_size=2048/sr=44100 this gives 1025/2 = 512 bins → max freq = 512*44100/2048 ≈ 11025 Hz ≈ **10 kHz, MATCHES the reported cutoff**.

The most likely culprit: somewhere, `num_bins` is halved before being passed to the curve drawing. Grep for `num_bins / 2` and `num_bins >> 1` in `src/editor_ui.rs` and `src/editor/curve.rs`. The reference-bin index `num_bins/2` used in the probe is fine (that's just a single bin), but if drawing iterates `0..num_bins/2`, that's the bug.

- [ ] **Step 4: Fix the cutoff**

Once located, replace the halving with `num_bins` (the full count). Commit:

```bash
git add -A
git commit -m "fix(ui): response curve polyline extends to Nyquist, not 10 kHz

Curve-drawing loop was iterating 0..num_bins/2 instead of 0..num_bins.
At 44.1 kHz / fft_size=2048 this truncated at bin 512 ≈ 11 kHz."
```

- [ ] **Step 5: Manual verification**

Launch the plugin in Bitwig at 44.1 kHz default FFT 2048. Confirm the response curve line extends to the right edge of the graph, not stopping at 10 kHz.

---

## Task 9: Freeze control row layout fix

**Files:**
- Read: `src/editor_ui.rs`
- Modify: `src/editor_ui.rs`

- [ ] **Step 1: Locate the row-rendering code**

Grep `src/editor_ui.rs` for "Offset" or "Tilt" DragValue creation. Identify the block that renders the Offset/Tilt/Curve row. Check whether it sits inside a module-specific branch or a shared helper.

- [ ] **Step 2: Determine why Freeze renders it elsewhere**

Look at `editor_ui.rs` around the curve panel composition. Possibilities:

(a) A conditional like `if module_type == Freeze { /* render controls at X */ } else { /* render at Y */ }`. Remove the branch.

(b) Freeze's curve-count is 5 and the row layout uses `num_curves` to position itself, landing at a different y-coordinate. Rework the layout so the row position is absolute (or anchored to the curve grid's bottom edge) regardless of curve count.

(c) The Offset/Tilt/Curve row is placed after the curve drawing, but Freeze's curve drawing has extra vertical padding for the 5-curve layout. Remove the asymmetric padding.

- [ ] **Step 3: Apply the fix**

Refactor so there's a single code path that renders the Offset/Tilt/Curve row at a fixed vertical offset from the curve panel bottom, identical for every module type. If there's no shared helper, extract one called `paint_curve_control_row(ui, params, slot, curve, scale)` and call it from the one place the row is drawn.

- [ ] **Step 4: Manual verification**

Launch the plugin, switch between Dynamics and Freeze in a slot. The Offset/Tilt/Curve row must appear at the same vertical position.

- [ ] **Step 5: Commit**

```bash
git add src/editor_ui.rs
git commit -m "fix(ui): Freeze Offset/Tilt/Curve row at same y as Dynamics

<describe what the actual cause was>"
```

---

## Task 10: Spec addenda

**Files:**
- Modify: `docs/superpowers/specs/2026-04-23-ui-parameter-spec-design.md`

- [ ] **Step 1: Append §2.3, §3.4, §4.4**

Open the spec file. Find §2 and append §2.3. Find §3 and append §3.4. Find §4 and append §4.4. If a subsection numbering already overlaps, pick the next free subsection number.

§2.3 body:

```markdown
### §2.3 Calibration contract

Every module's internal DSP must accept the full range implied by its curve's
declared `offset_fn` extremes. When the normalized `offset` is +1, the
DSP-observed parameter must reach the config's `y_max`; when `offset` is -1,
it must reach `y_min`. If a module clamps for DSP safety, the clamp values
MUST match `y_min` and `y_max`. Any tighter clamp is a bug.

This contract is verified end-to-end by `tests/calibration_roundtrip.rs`.
New modules MUST add themselves to that test's case table when they are
introduced.
```

§3.4 body:

```markdown
### §3.4 Curve and node rendering at limits

- Curve values outside `[y_min, y_max]` are rendered as a flat line along the
  exceeded border (top or bottom edge of the graph), not omitted.
- Curve nodes whose computed y-position is outside the graph are drawn
  truncated to the border with the dot still fully visible.
- When a node is being dragged, its virtual (un-clipped) physical value is
  shown in the hover tooltip.
- Each curve config declares its allowed `[y_min, y_max]`; the UI renderer
  is the sole place that enforces the visual clip.
```

§4.4 body:

```markdown
### §4.4 Control row consistency

The Offset / Tilt / Curve DragValue row is rendered at a fixed vertical
position per slot, identical across all module types. Modules may not define
their own layout for these controls. The row is drawn by a single shared code
path in `editor_ui.rs` regardless of the slot's module type or curve count.
```

- [ ] **Step 2: Commit**

```bash
git add docs/superpowers/specs/2026-04-23-ui-parameter-spec-design.md
git commit -m "docs(spec): §2.3 calibration contract, §3.4 node rendering, §4.4 row consistency"
```

---

## Task 11: Final verification

- [ ] **Step 1: All tests pass**

Run: `cargo test`
Expected: all tests pass (existing 86 + calibration_roundtrip cases + curve_display_extent).

- [ ] **Step 2: Release build clean**

Run: `cargo build --release`
Expected: no warnings, no errors.

- [ ] **Step 3: Smoke test in Bitwig**

Bundle and install:

```bash
cargo run --package xtask -- bundle spectral_forge --release
cp target/bundled/spectral_forge.clap ~/.clap/
```

Rescan in Bitwig at 44.1 kHz. Verify, in order:
- Response curve polyline reaches the right edge of the graph (not 10 kHz).
- Freeze Offset/Tilt/Curve row aligned with Dynamics row when switching slot types.
- Dynamics ratio offset at +1 → 20:1 compression audible on a loud signal.
- Freeze length offset at +1 → long freeze (several seconds).
- Freeze length offset at -1 → short freeze (tens of ms).

If any manual check fails: file a follow-up task and fix. The calibration test proves the DSP side is correct; any remaining gap is in the user-facing parameter plumbing (smoothers, FloatParam ranges, units on the DragValue).
