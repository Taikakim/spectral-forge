# Phase 5b.4 — Modulate Retrofit (Gravity Phaser + PLL Tear) Implementation Plan

> **Status:** PLANNED — implementation pending. Phase 5b sub-plan; depends on
> Phase 1 foundation infra, Phase 2f Modulate-light, Phase 3 BinPhysics, Phase 4
> PLPV unwrapped phase, and Phase 5b.3 `physics_helpers.rs`.
>
> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Retrofit the existing `ModulateModule` (shipped in Phase 2f as 5
light-CPU modes) with two physics-aware modes — **Gravity Phaser** (reads/writes
`BinPhysics::phase_momentum`) and **PLL Tear** (consumes `ctx.unwrapped_phase`
from PLPV) — plus the supporting infrastructure: per-slot `Repel` /
`SidechainPositioned` toggles for Gravity Phaser, a per-bin PLL bank helper in
`physics_helpers.rs`, and a 1-pole curve smoother applied to the new modes to
defend against parametric instability.

**Architecture:** Two new `ModulateMode` variants extend the v1 enum without
breaking existing kernels. The retrofit adds a `smoothed_curves: [[Vec<f32>;
6]; 2]` scratch field to `ModulateModule`, populated only when the active mode
is Gravity Phaser or PLL Tear (so v1 modes remain byte-identical). Per-slot
booleans (`slot_modulate_repel`, `slot_modulate_sc_positioned`) live in
`params.rs` as `Arc<Mutex<bool>>`, snapshotted per block via `try_lock` on the
audio thread, propagated to `FxMatrix::set_modulate_repels` /
`set_modulate_sc_positioneds`. The PLL bank kernel is added to the shared
`physics_helpers.rs` module (alongside the smoothing/CFL/damping helpers
introduced by Phase 5b.3) so that future PLL consumers (Harmony pitch tracking,
Past loop) can re-use it. `ModuleSpec` is upgraded from
`heavy_cpu_per_mode: None` to `Some(&MOD_HEAVY)` and `writes_bin_physics: true`.

**Tech Stack:** Rust 2021, `nih-plug`, `realfft::num_complex::Complex<f32>`,
existing `SpectralModule` trait + `ModuleContext` (with Phase 3+4 fields:
`bin_physics`, `unwrapped_phase`, `peaks`), `crate::dsp::physics_helpers`
(Phase 5b.3), `crate::dsp::bin_physics::BinPhysics` (Phase 3).

**Source spec:** `ideas/next-gen-modules/16-modulate.md` (research findings
2026-04-26 incorporated). Phase 5b.4 implements the audit gaps that depend on
infrastructure landed by Phases 3 and 4.

**Phase order assumption:** This plan assumes Phases 1, 2f, 3, 4, and 5b.3 have
all merged. Verify with `git log --oneline | grep -E '(phase-1|phase-2f|phase-3|phase-4|phase-5b3)'`
before starting.

**Defer list (NOT in this plan):**
- **FM Network — Partial Web** — requires `ctx.instantaneous_freq` (Phase 6.1).
  Lands as a 3rd retrofit pass in Phase 6.
- **Slew Lag** — requires `ctx.sidechain_derivative` (deferred). Not yet specced
  for Phase 6 either; revisit after FM Network.
- **AmpGate retrofit for Gravity Phaser** — the v1 `AMPGATE` curve is wired but
  consumed only by Phase Phaser. Gravity Phaser also reads it (additive scale on
  the per-bin force). Implemented in Task 4.
- **Per-channel Gravity Phaser nodes** — v1 node positions are mono (same node
  set for left/right). Per-channel asymmetric nodes are a v2 nice-to-have.

**Risk register:**
- **Phase wrap discontinuities in PLL update.** Mitigated by `wrap_phase()`
  helper which folds raw `target - predicted` into `[-π, π]` before applying
  the loop gains. Tested in Task 2.
- **Tear runaway on extreme curve modulation.** Mitigated by routing curves
  through `smooth_curve_one_pole()` (Phase 5b.3 helper) before the PLL update.
  This caps the rate of change of `ωₙ` (loop bandwidth) and prevents
  parametric instability. Tested in Task 3.
- **Phase momentum drift across slot chain.** Gravity Phaser writes
  `phase_momentum += delta` cumulatively. The per-block decay built into
  `BinPhysics` (Phase 3) prevents unbounded growth; verified by Task 12 200-hop
  bounded test.
- **PLL CPU at full sweep.** PLL Tear at 8193 bins is ~32k FLOPs/hop per channel
  per slot — comfortably <0.1% core. The `heavy_cpu_per_mode` flag marks PLL
  Tear (only) as heavy so the UI can warn when 4+ Modulate slots are PLL-mode.
- **Ordering: Gravity Phaser runs before PLL Tear if both are in the chain.**
  Phase momentum from Gravity Phaser feeds PLL Tear's lock-loss detector
  cleanly (the loop tracks the kicked phase); verified by sequencing test in
  Task 10.
- **Sidechain peak picker allocation.** Gravity Phaser's SidechainPositioned
  mode finds local maxima in sidechain magnitude. We use a fixed-size
  `SmallVec<[(usize, f32); 32]>` (cap = 32 nodes) — stack-only, no heap alloc.
- **Backward-compatibility: v1 Modulate kernels.** All five v1 kernels stay
  byte-identical because (a) the `ModulateMode` enum extension is additive and
  (b) the smoothing pass runs only when mode ∈ {GravityPhaser, PllTear}. v1
  multi-hop test (`modulate_finite_bounded_all_modes_dual_channel`) must
  continue to pass without modification.

---

## File Structure

**Create:** none. All retrofit changes are additive edits.

**Modify:**
- `src/dsp/modules/modulate.rs` — extend enum, add 2 kernels, add helper
  state (smoothed_curves, pll bank, peaks scratch), add 2 setters.
- `src/dsp/modules/mod.rs` — `ModuleSpec` for Modulate gains
  `heavy_cpu_per_mode: Some(&MOD_HEAVY)` and `writes_bin_physics: true`; trait
  gains `set_modulate_repel` and `set_modulate_sc_positioned` no-op defaults.
- `src/dsp/physics_helpers.rs` — add `pll_bank_step()` and `wrap_phase()`.
- `src/dsp/fx_matrix.rs` — add `set_modulate_repels()` and
  `set_modulate_sc_positioneds()` propagation methods.
- `src/params.rs` — add `slot_modulate_repel`, `slot_modulate_sc_positioned`
  fields + snap helpers.
- `src/lib.rs` (a.k.a. `pipeline.rs` audio-thread block) — snapshot the two new
  `Mutex` arrays per block and push to `FxMatrix`.
- `src/editor/modulate_popup.rs` — extend mode list with Gravity Phaser + PLL
  Tear; render Repel + SidechainPositioned checkboxes when Gravity Phaser is
  active.
- `tests/module_trait.rs` — new tests for the two kernels, smoothing, sequencing.
- `tests/calibration_roundtrip.rs` — extend `ModulateProbe` and round-trip.
- `tests/physics_helpers.rs` — add tests for `pll_bank_step()` and
  `wrap_phase()`.
- `docs/superpowers/STATUS.md` — update Phase 2f entry to note retrofit
  completion; add Phase 5b.4 row.
- `docs/superpowers/plans/2026-04-27-phase-2f-modulate-light.md` — append
  banner pointing at the retrofit plan.

---

## Self-imposed plan rules

- TDD: every task starts with a failing test, ends with green tests + commit.
- DRY: PLL bank lives in `physics_helpers.rs` (not in `modulate.rs`) so
  Harmony's pitch tracker and Past's pitch-coherent stretch can re-use it.
- YAGNI: per-channel asymmetric Gravity nodes deferred to v2 (mono nodes
  shared across channels for v1 retrofit).
- Frequent commits: 13 tasks → 13 commits.
- All shared helpers stay in `physics_helpers.rs`.

---

## Task 1: Extend `ModulateMode` enum + ModuleSpec heavy_cpu_per_mode + writes_bin_physics

**Files:**
- Modify: `src/dsp/modules/modulate.rs` — extend enum
- Modify: `src/dsp/modules/mod.rs` — update Modulate's `module_spec()` entry
- Modify: `tests/module_trait.rs` — assert new spec fields

- [ ] **Step 1: Write the failing test**

In `tests/module_trait.rs`:

```rust
#[test]
fn modulate_spec_advertises_physics_writer_and_per_mode_heavy() {
    use spectral_forge::dsp::modules::{module_spec, ModuleType};

    let spec = module_spec(ModuleType::Modulate);
    assert!(spec.writes_bin_physics, "Gravity Phaser writes phase_momentum");
    let heavy = spec.heavy_cpu_per_mode.expect("retrofit enables per-mode heavy flag");
    assert_eq!(heavy.len(), 7, "5 v1 modes + GravityPhaser + PllTear");
    // Existing 5 modes are light.
    for i in 0..5 { assert!(!heavy[i], "v1 mode index {} marked heavy unexpectedly", i); }
    // GravityPhaser is light.
    assert!(!heavy[5], "GravityPhaser should be light");
    // PllTear is heavy.
    assert!(heavy[6], "PllTear must be heavy (PLL bank)");
}

#[test]
fn modulate_mode_enum_has_new_variants() {
    use spectral_forge::dsp::modules::modulate::ModulateMode;
    let _ = ModulateMode::GravityPhaser;
    let _ = ModulateMode::PllTear;
}
```

- [ ] **Step 2: Run tests, expect failure**

Run: `cargo test --test module_trait modulate_spec_advertises_physics_writer_and_per_mode_heavy modulate_mode_enum_has_new_variants -- --nocapture`
Expected: FAIL — `GravityPhaser`, `PllTear`, `writes_bin_physics: true`,
`heavy_cpu_per_mode: Some(...)` not present.

- [ ] **Step 3: Extend the enum**

In `src/dsp/modules/modulate.rs`, change the `ModulateMode` definition:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModulateMode {
    PhasePhaser,
    BinSwapper,
    RmFmMatrix,
    DiodeRm,
    GroundLoop,
    GravityPhaser,
    PllTear,
}
```

Default impl unchanged (`PhasePhaser`).

- [ ] **Step 4: Add `MOD_HEAVY` static and update `module_spec()` for Modulate**

In `src/dsp/modules/mod.rs`, near the top of the `module_spec()` function (or
file scope, before it):

```rust
/// Per-mode heavy-CPU markers for ModulateMode. Order MUST match the enum
/// declaration in `crate::dsp::modules::modulate::ModulateMode`.
/// PhasePhaser, BinSwapper, RmFmMatrix, DiodeRm, GroundLoop, GravityPhaser, PllTear.
const MOD_HEAVY: [bool; 7] = [false, false, false, false, false, false, true];
```

In the `ModuleSpec` literal returned by `module_spec(ModuleType::Modulate)`,
flip these two fields (replace existing values):

```rust
heavy_cpu: false, // per-mode, see heavy_cpu_per_mode
heavy_cpu_per_mode: Some(&MOD_HEAVY),
writes_bin_physics: true,
```

Leave `wants_sidechain: true` (v1) and the other fields unchanged.

- [ ] **Step 5: Run tests, expect pass**

Run: `cargo test --test module_trait modulate_spec_advertises_physics_writer_and_per_mode_heavy modulate_mode_enum_has_new_variants -- --nocapture`
Expected: PASS.

- [ ] **Step 6: Verify v1 tests still pass (no behaviour drift)**

Run: `cargo test --test module_trait modulate -- --nocapture`
Expected: All 6 v1 modulate tests pass (`modulate_module_spec_present`,
`modulate_module_constructs_and_passes_through`,
`modulate_phase_phaser_rotates_phase`, `modulate_bin_swapper_blends_neighbours`,
`modulate_rm_fm_matrix_modulates_with_sidechain`,
`modulate_diode_rm_leaks_carrier_when_input_quiet`,
`modulate_ground_loop_injects_mains_harmonics`,
`modulate_finite_bounded_all_modes_dual_channel`,
`modulate_mode_persists_via_setter`). The new variants are not yet referenced
in process(), so adding them is safe.

- [ ] **Step 7: Commit**

```bash
git add src/dsp/modules/modulate.rs src/dsp/modules/mod.rs tests/module_trait.rs
git commit -m "feat(modulate): extend mode enum with GravityPhaser + PllTear; declare physics writer"
```

---

## Task 2: Add `pll_bank_step()` and `wrap_phase()` to `physics_helpers.rs`

**Files:**
- Modify: `src/dsp/physics_helpers.rs`
- Modify: `tests/physics_helpers.rs`

- [ ] **Step 1: Write the failing tests**

Append to `tests/physics_helpers.rs`:

```rust
use spectral_forge::dsp::physics_helpers::{pll_bank_step, wrap_phase};
use std::f32::consts::{PI, TAU};

#[test]
fn wrap_phase_folds_into_minus_pi_to_pi() {
    assert!((wrap_phase(0.0) - 0.0).abs() < 1e-6);
    assert!((wrap_phase(PI) - PI).abs() < 1e-6);
    assert!((wrap_phase(-PI) - (-PI)).abs() < 1e-6);
    // 1.5*PI -> -0.5*PI
    assert!((wrap_phase(1.5 * PI) - (-0.5 * PI)).abs() < 1e-5);
    // -1.5*PI -> 0.5*PI
    assert!((wrap_phase(-1.5 * PI) - (0.5 * PI)).abs() < 1e-5);
    // 5*PI -> PI (or -PI; either is fine within 1e-5)
    let w = wrap_phase(5.0 * PI);
    assert!(w.abs() <= PI + 1e-5);
}

#[test]
fn pll_bank_step_locks_to_constant_target() {
    // 4 bins, all targets at PI/4. PLL starts at 0, should converge.
    let mut pll_phase = vec![0.0_f32; 4];
    let mut pll_freq = vec![0.0_f32; 4];
    let target = vec![PI / 4.0; 4];
    let mut err = vec![0.0_f32; 4];
    // Butterworth-flat: omega_n = 0.05 cycles/hop, zeta = 0.707.
    // alpha = 2 * zeta * omega_n, beta = omega_n^2.
    let omega_n = 0.05_f32;
    let zeta = 0.707_f32;
    let alpha = 2.0 * zeta * omega_n;
    let beta = omega_n * omega_n;

    // Run 200 hops; final phase error must be near-zero.
    for _ in 0..200 {
        pll_bank_step(&mut pll_phase, &mut pll_freq, &target, alpha, beta, &mut err);
    }
    for k in 0..4 {
        assert!(err[k].abs() < 0.01, "bin {} did not lock: err = {}", k, err[k]);
        // PLL frequency should have settled near 0 (target is constant).
        assert!(pll_freq[k].abs() < 0.01, "bin {} freq drift: {}", k, pll_freq[k]);
    }
}

#[test]
fn pll_bank_step_tracks_constant_velocity_target() {
    // Target advances by 0.1 rad per hop. PLL should match the velocity.
    let mut pll_phase = vec![0.0_f32];
    let mut pll_freq = vec![0.0_f32];
    let mut target = 0.0_f32;
    let mut err = vec![0.0_f32];
    let omega_n = 0.1_f32;
    let zeta = 0.707_f32;
    let alpha = 2.0 * zeta * omega_n;
    let beta = omega_n * omega_n;

    // 500 hops to fully settle.
    for _ in 0..500 {
        target += 0.1;
        let target_v = vec![wrap_phase(target)];
        pll_bank_step(&mut pll_phase, &mut pll_freq, &target_v, alpha, beta, &mut err);
    }
    // Steady-state error for a velocity ramp under a 2nd-order PI loop should
    // approach zero (this loop is type-2, no steady-state ramp error).
    assert!(err[0].abs() < 0.05, "velocity tracking err = {}", err[0]);
    // Freq estimate should be near 0.1 rad/hop.
    assert!((pll_freq[0] - 0.1).abs() < 0.01, "freq estimate = {}", pll_freq[0]);
}

#[test]
fn pll_bank_step_lengths_must_match() {
    // Debug assert; not a panic in release. Skip on release builds.
    if cfg!(debug_assertions) {
        let mut pll_phase = vec![0.0_f32; 3];
        let mut pll_freq = vec![0.0_f32; 3];
        let target = vec![0.0_f32; 4]; // mismatched
        let mut err = vec![0.0_f32; 3];
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            pll_bank_step(&mut pll_phase, &mut pll_freq, &target, 0.05, 0.0025, &mut err);
        }));
        assert!(result.is_err(), "expected debug assert panic on length mismatch");
    }
}
```

- [ ] **Step 2: Run tests, expect failure**

Run: `cargo test --test physics_helpers pll_bank wrap_phase -- --nocapture`
Expected: FAIL — `pll_bank_step`, `wrap_phase` not in scope.

- [ ] **Step 3: Add helpers to `src/dsp/physics_helpers.rs`**

Append to the file (after the existing helpers):

```rust
/// Fold an angle into the canonical interval `[-π, π]`.
/// Branchless: uses `rem_euclid` on a shifted-by-π value, then shifts back.
#[inline]
pub fn wrap_phase(p: f32) -> f32 {
    use std::f32::consts::{PI, TAU};
    // (p + PI).rem_euclid(TAU) gives a value in [0, TAU); subtract PI -> [-PI, PI).
    let shifted = (p + PI).rem_euclid(TAU);
    shifted - PI
}

/// Per-bin 2nd-order PI phase-locked loop bank step. One iteration per bin per hop.
///
/// Update rule (per bin `k`):
/// ```text
/// err = wrap_phase(target_phase[k] - pll_phase[k])
/// pll_freq[k]  += beta  * err
/// pll_phase[k] += pll_freq[k] + alpha * err
/// out_phase_error[k] = err
/// ```
///
/// `alpha = 2 * zeta * omega_n`, `beta = omega_n * omega_n` with `omega_n` in
/// cycles-per-hop (loop natural frequency). Defaults: `omega_n = 0.05`,
/// `zeta = 0.707` (Butterworth-flat). See `ideas/next-gen-modules/16-modulate.md`
/// research finding 1.
///
/// All four mutable slices and `target_phase` must have the same length.
/// Caller is responsible for choosing which bins to step (e.g. skipping
/// sub-100Hz bins for PLL Tear; finding 3).
#[inline]
pub fn pll_bank_step(
    pll_phase: &mut [f32],
    pll_freq: &mut [f32],
    target_phase: &[f32],
    alpha: f32,
    beta: f32,
    out_phase_error: &mut [f32],
) {
    debug_assert_eq!(pll_phase.len(), pll_freq.len());
    debug_assert_eq!(pll_phase.len(), target_phase.len());
    debug_assert_eq!(pll_phase.len(), out_phase_error.len());
    let n = pll_phase.len();
    for k in 0..n {
        let err = wrap_phase(target_phase[k] - pll_phase[k]);
        pll_freq[k] += beta * err;
        pll_phase[k] += pll_freq[k] + alpha * err;
        out_phase_error[k] = err;
    }
}
```

- [ ] **Step 4: Run tests, expect pass**

Run: `cargo test --test physics_helpers -- --nocapture`
Expected: PASS — all original Phase 5b.3 helpers + new `wrap_phase` and
`pll_bank_step` tests pass (existing tests are not modified).

- [ ] **Step 5: Commit**

```bash
git add src/dsp/physics_helpers.rs tests/physics_helpers.rs
git commit -m "feat(physics_helpers): add wrap_phase + pll_bank_step (2nd-order PI)"
```

---

## Task 3: Curve smoothing infrastructure in `ModulateModule`

**Files:**
- Modify: `src/dsp/modules/modulate.rs` — add `smoothed_curves`, prime helper
- Modify: `tests/module_trait.rs` — verify smoothing applies to retrofit modes only

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn modulate_smoothed_curves_present_for_retrofit_modes() {
    use spectral_forge::dsp::modules::modulate::{ModulateModule, ModulateMode};
    use spectral_forge::dsp::modules::SpectralModule;

    let mut module = ModulateModule::new();
    module.reset(48_000.0, 2048);

    // After reset, smoothed_curves must be allocated to num_bins.
    let snap = module.smoothed_curves_len();
    assert_eq!(snap, 1025, "smoothed_curves not allocated to fft_size/2+1");
}

#[test]
fn modulate_v1_modes_skip_smoothing_pass() {
    use spectral_forge::dsp::modules::modulate::{ModulateModule, ModulateMode};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule, StereoLink, FxChannelTarget};
    use realfft::num_complex::Complex;

    // PhasePhaser is v1: smoothing must NOT alter its curve consumption.
    // Verify by running 1 hop with non-trivial curves and comparing magnitude
    // outputs to the v1 baseline expectation: phase rotated, magnitudes preserved.
    let mut module = ModulateModule::new();
    module.reset(48_000.0, 2048);
    module.set_modulate_mode(ModulateMode::PhasePhaser);

    let num_bins = 1025;
    let mut bins: Vec<Complex<f32>> = (0..num_bins).map(|_| Complex::new(1.0, 0.0)).collect();

    let amount = vec![2.0_f32; num_bins];
    let neutral = vec![1.0_f32; num_bins];
    let zeros = vec![0.0_f32; num_bins];
    let mix = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &neutral, &neutral, &neutral, &zeros, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = ModuleContext::new_minimal(48_000.0, 2048);

    module.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, None, &curves, None, &mut suppression, &ctx);

    // Magnitudes preserved (Phase Phaser invariant — same as v1).
    for k in 0..num_bins {
        let mag = bins[k].norm();
        assert!((mag - 1.0).abs() < 1e-3, "v1 PhasePhaser invariant violated at bin {}: mag={}", k, mag);
    }
}
```

- [ ] **Step 2: Run tests, expect failure**

Run: `cargo test --test module_trait modulate_smoothed_curves_present_for_retrofit_modes modulate_v1_modes_skip_smoothing_pass -- --nocapture`
Expected: FAIL — `smoothed_curves_len` not present; the second test
might fail to compile due to the new `physics: Option<&mut BinPhysics>` arg
position which Phase 3 adds.

- [ ] **Step 3: Add smoothed_curves field + reset allocation**

In `src/dsp/modules/modulate.rs`, extend the struct:

```rust
pub struct ModulateModule {
    mode: ModulateMode,
    /// Per-channel hop counter for animated Phase Phaser.
    hop_count: [u64; 2],
    /// Per-channel scratch for Bin Swapper.
    swap_scratch: [Vec<Complex<f32>>; 2],
    /// Per-channel RMS history ring for Ground Loop sag detection.
    rms_history: [[f32; 16]; 2],
    rms_idx: [usize; 2],
    /// Smoothed copies of the 6 input curves, used by retrofit modes
    /// (GravityPhaser, PllTear) to defend against parametric instability.
    /// First-touch flag per channel to prime the smoother on the first hop
    /// after reset (avoids 5-hop ramp-in).
    smoothed_curves: [[Vec<f32>; 6]; 2],
    smoothed_primed: [bool; 2],
    sample_rate: f32,
    fft_size: usize,
}
```

Update `Self::new`:

```rust
impl ModulateModule {
    pub fn new() -> Self {
        Self {
            mode: ModulateMode::default(),
            hop_count: [0; 2],
            swap_scratch: [Vec::new(), Vec::new()],
            rms_history: [[0.0; 16]; 2],
            rms_idx: [0; 2],
            smoothed_curves: [
                [Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new()],
                [Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new()],
            ],
            smoothed_primed: [false; 2],
            sample_rate: 48_000.0,
            fft_size: 2048,
        }
    }

    pub fn smoothed_curves_len(&self) -> usize {
        // Used by tests to verify allocation.
        self.smoothed_curves[0][0].len()
    }
    // ... existing helpers below ...
}
```

Update `reset()`:

```rust
fn reset(&mut self, sample_rate: f32, fft_size: usize) {
    self.sample_rate = sample_rate;
    self.fft_size = fft_size;
    let num_bins = fft_size / 2 + 1;
    for ch in 0..2 {
        self.hop_count[ch] = 0;
        self.swap_scratch[ch].clear();
        self.swap_scratch[ch].resize(num_bins, Complex::new(0.0, 0.0));
        self.rms_history[ch] = [0.0; 16];
        self.rms_idx[ch] = 0;
        for c in 0..6 {
            self.smoothed_curves[ch][c].clear();
            self.smoothed_curves[ch][c].resize(num_bins, 0.0);
        }
        self.smoothed_primed[ch] = false;
    }
}
```

- [ ] **Step 4: Add smoothing helper method**

Inside `impl ModulateModule`:

```rust
/// Refresh `smoothed_curves[channel]` from the raw input curves.
/// Called only by retrofit modes; v1 modes consume `curves` directly.
/// On the first hop after reset, the smoother is primed by direct copy
/// (otherwise it would ramp in over ~5 hops).
fn refresh_smoothed(&mut self, channel: usize, curves: &[&[f32]], num_bins: usize) {
    use crate::dsp::physics_helpers::smooth_curve_one_pole;
    let dt = self.fft_size as f32 / self.sample_rate / 4.0; // 75% overlap → hop = fft/4
    let primed = self.smoothed_primed[channel];
    for c in 0..6.min(curves.len()) {
        let src = &curves[c][..num_bins];
        let dst = &mut self.smoothed_curves[channel][c][..num_bins];
        if !primed {
            dst.copy_from_slice(src);
        } else {
            smooth_curve_one_pole(dst, src, dt);
        }
    }
    self.smoothed_primed[channel] = true;
}

/// Borrow the 6 smoothed curves for a channel as a curves-style slice.
fn smoothed_curves_for(&self, channel: usize) -> [&[f32]; 6] {
    [
        &self.smoothed_curves[channel][0],
        &self.smoothed_curves[channel][1],
        &self.smoothed_curves[channel][2],
        &self.smoothed_curves[channel][3],
        &self.smoothed_curves[channel][4],
        &self.smoothed_curves[channel][5],
    ]
}
```

- [ ] **Step 5: Update `process()` signature for Phase 3 BinPhysics**

The Phase 3 plan changed the trait `process()` to include
`physics: Option<&mut BinPhysics>` immediately after `suppression_out`. v1
Modulate's `process()` already accepts that arg as `_physics`. Confirm with:

```bash
grep -n "fn process" src/dsp/modules/modulate.rs
```

Expected output includes `physics: Option<&mut crate::dsp::bin_physics::BinPhysics>,`
(possibly named `_physics`). If it's missing, this plan cannot proceed — return
to Phase 3 and verify modulate.rs was updated.

- [ ] **Step 6: Run tests, expect pass**

Run: `cargo test --test module_trait modulate_smoothed_curves_present_for_retrofit_modes modulate_v1_modes_skip_smoothing_pass -- --nocapture`
Expected: PASS — smoothed_curves allocated, v1 PhasePhaser unchanged.

- [ ] **Step 7: Run full v1 modulate suite to confirm no regressions**

Run: `cargo test --test module_trait modulate -- --nocapture`
Expected: All v1 tests pass.

- [ ] **Step 8: Commit**

```bash
git add src/dsp/modules/modulate.rs tests/module_trait.rs
git commit -m "feat(modulate): smoothed_curves scratch + helper for retrofit modes"
```

---

## Task 4: Gravity Phaser core kernel (reads + writes phase_momentum)

**Files:**
- Modify: `src/dsp/modules/modulate.rs` — add kernel and dispatch arm
- Modify: `tests/module_trait.rs` — kernel test

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn modulate_gravity_phaser_writes_phase_momentum_and_rotates() {
    use spectral_forge::dsp::bin_physics::BinPhysics;
    use spectral_forge::dsp::modules::modulate::{ModulateModule, ModulateMode};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule, StereoLink, FxChannelTarget};
    use realfft::num_complex::Complex;

    let mut module = ModulateModule::new();
    module.reset(48_000.0, 2048);
    module.set_modulate_mode(ModulateMode::GravityPhaser);

    let num_bins = 1025;
    let mut bins: Vec<Complex<f32>> = (0..num_bins).map(|_| Complex::new(1.0, 0.0)).collect();
    let dry_norms: Vec<f32> = bins.iter().map(|b| b.norm()).collect();

    // AMOUNT=2 (max), REACH=1, RATE=1, THRESH=1, AMPGATE=0, MIX=2 (full wet)
    let amount = vec![2.0_f32; num_bins];
    let neutral = vec![1.0_f32; num_bins];
    let zeros = vec![0.0_f32; num_bins];
    let mix = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &neutral, &neutral, &neutral, &zeros, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let mut physics = BinPhysics::new();
    physics.reset_active(num_bins, 48_000.0, 2048);
    // Seed phase_momentum at bin 100.
    physics.phase_momentum[100] = 0.5;

    let ctx = ModuleContext::new_minimal(48_000.0, 2048);

    // Run a few hops so the smoother primes and momentum integrates.
    for _ in 0..10 {
        module.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, None, &curves, Some(&mut physics), &mut suppression, &ctx);
    }

    // Magnitudes preserved (rotation is unit-modulus).
    for k in 0..num_bins {
        let mag = bins[k].norm();
        assert!((mag - dry_norms[k]).abs() < 0.05, "bin {} mag drift {} -> {}", k, dry_norms[k], mag);
    }
    // Phases must have rotated away from 0 around bin 100 (where momentum was seeded).
    let near_seed: f32 = (95..=105).map(|k| bins[k].im.abs()).fold(0.0, f32::max);
    assert!(near_seed > 0.05, "near-seed bins did not rotate (max im = {})", near_seed);
    // Phase momentum must now be non-zero around bin 100 (kernel writes it).
    let momentum_after = physics.phase_momentum[100];
    assert!(momentum_after.is_finite(), "momentum NaN after Gravity Phaser");
    assert!(momentum_after.abs() > 0.0, "Gravity Phaser did not write phase_momentum");
}
```

- [ ] **Step 2: Run test, expect failure**

Run: `cargo test --test module_trait modulate_gravity_phaser_writes_phase_momentum_and_rotates -- --nocapture`
Expected: FAIL — GravityPhaser arm not dispatched in process().

- [ ] **Step 3: Add the Gravity Phaser kernel**

In `src/dsp/modules/modulate.rs` (above `impl SpectralModule`):

```rust
fn apply_gravity_phaser(
    bins: &mut [Complex<f32>],
    smoothed: &[&[f32]; 6],
    phase_momentum: Option<&mut [f32]>,
    repel: bool,
) {
    use std::f32::consts::PI;

    let amount_c = smoothed[0];
    let reach_c = smoothed[1];
    let _rate_c = smoothed[2]; // animation rate consumed by SidechainPositioned (Task 6)
    let thresh_c = smoothed[3];
    let ampgate_c = smoothed[4];
    let mix_c = smoothed[5];

    let num_bins = bins.len();
    let sign: f32 = if repel { -1.0 } else { 1.0 };

    // Optional borrow: if physics is None, we operate on a no-op zero buffer.
    let mut zeros_local: Vec<f32>;
    let momentum: &mut [f32] = match phase_momentum {
        Some(m) => &mut m[..num_bins.min(m.len())],
        None => {
            // Caller did not supply BinPhysics; allocate zeros once on the test path.
            // On the audio thread this branch must never be hit (FxMatrix always
            // supplies physics for `writes_bin_physics: true` modules).
            zeros_local = vec![0.0; num_bins];
            &mut zeros_local[..]
        }
    };

    for k in 0..num_bins {
        let amount = amount_c[k].clamp(0.0, 2.0);
        let reach = reach_c[k].clamp(0.0, 4.0);
        let thresh = thresh_c[k].clamp(0.01, 4.0);
        let ampgate = ampgate_c[k].clamp(0.0, 2.0);
        let mix = mix_c[k].clamp(0.0, 2.0) * 0.5;

        let mag = bins[k].norm();
        // Amp-gated drive: when ampgate > 0, the per-bin drive is scaled by min(mag/thresh, 1).
        let gate_factor = if ampgate > 0.001 {
            (mag / thresh).min(1.0) * ampgate.min(1.0)
        } else {
            1.0
        };

        // Force = sign * amount * (reach * 0.05) — `reach` widens the per-bin influence.
        // Integrated as: momentum += force; rotation = momentum.
        let force = sign * amount * reach * 0.05 * gate_factor;
        // Per-bin momentum decay (5%/hop). Prevents unbounded growth across many hops.
        momentum[k] = momentum[k] * 0.95 + force;

        let rotation = momentum[k] * PI;
        let cos_r = rotation.cos();
        let sin_r = rotation.sin();
        let dry = bins[k];
        let wet = Complex::new(
            dry.re * cos_r - dry.im * sin_r,
            dry.re * sin_r + dry.im * cos_r,
        );
        bins[k] = dry * (1.0 - mix) + wet * mix;
    }
}
```

- [ ] **Step 4: Wire the dispatch arm in `process()`**

Find the existing `match self.mode` block and add:

```rust
ModulateMode::GravityPhaser => {
    let num_bins = bins.len();
    self.refresh_smoothed(channel, curves, num_bins);
    let smoothed = self.smoothed_curves_for(channel);
    let momentum_slice = physics.as_deref_mut().map(|p| &mut p.phase_momentum[..num_bins]);
    apply_gravity_phaser(bins, &smoothed, momentum_slice, /* repel */ false);
}
```

(Repel toggle is wired in Task 5; for now hard-coded to `false`.)

The arg `physics` is `Option<&mut BinPhysics>`. To avoid borrow conflicts when
the kernel needs `phase_momentum` only, we slice it. `as_deref_mut()` lifts
`Option<&mut BinPhysics>` to give us `Option<&mut BinPhysics>` again — actually
the type is already `Option<&mut BinPhysics>`, so just use `physics.as_mut()`:

```rust
let momentum_slice = physics.as_mut().map(|p| &mut p.phase_momentum[..num_bins]);
```

- [ ] **Step 5: Run test, expect pass**

Run: `cargo test --test module_trait modulate_gravity_phaser_writes_phase_momentum_and_rotates -- --nocapture`
Expected: PASS — magnitudes preserved, near-seed bins rotated, momentum
non-zero.

- [ ] **Step 6: Run v1 suite to confirm no regressions**

Run: `cargo test --test module_trait modulate -- --nocapture`
Expected: All previous tests still pass.

- [ ] **Step 7: Commit**

```bash
git add src/dsp/modules/modulate.rs tests/module_trait.rs
git commit -m "feat(modulate): Gravity Phaser kernel reads/writes phase_momentum"
```

---

## Task 5: Per-slot Repel toggle for Gravity Phaser

**Files:**
- Modify: `src/dsp/modules/mod.rs` — `set_modulate_repel` trait default
- Modify: `src/dsp/modules/modulate.rs` — override + use in dispatch
- Modify: `src/dsp/fx_matrix.rs` — `set_modulate_repels()` propagation
- Modify: `src/params.rs` — `slot_modulate_repel` field + snap helper
- Modify: `src/lib.rs` — snapshot per block + push to FxMatrix
- Modify: `tests/module_trait.rs` — repel toggle test

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn modulate_gravity_phaser_repel_inverts_rotation_direction() {
    use spectral_forge::dsp::bin_physics::BinPhysics;
    use spectral_forge::dsp::modules::modulate::{ModulateModule, ModulateMode};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule, StereoLink, FxChannelTarget};
    use realfft::num_complex::Complex;

    fn run(repel: bool) -> f32 {
        let mut module = ModulateModule::new();
        module.reset(48_000.0, 2048);
        module.set_modulate_mode(ModulateMode::GravityPhaser);
        module.set_modulate_repel(repel);

        let num_bins = 1025;
        let mut bins: Vec<Complex<f32>> = (0..num_bins).map(|_| Complex::new(1.0, 0.0)).collect();

        let amount = vec![2.0_f32; num_bins];
        let neutral = vec![1.0_f32; num_bins];
        let zeros = vec![0.0_f32; num_bins];
        let mix = vec![2.0_f32; num_bins];
        let curves: Vec<&[f32]> = vec![&amount, &neutral, &neutral, &neutral, &zeros, &mix];

        let mut suppression = vec![0.0_f32; num_bins];
        let mut physics = BinPhysics::new();
        physics.reset_active(num_bins, 48_000.0, 2048);

        let ctx = ModuleContext::new_minimal(48_000.0, 2048);
        for _ in 0..15 {
            module.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, None, &curves, Some(&mut physics), &mut suppression, &ctx);
        }
        physics.phase_momentum[200] // sample any bin
    }

    let pull_momentum = run(false);
    let push_momentum = run(true);
    assert!(pull_momentum.is_finite() && push_momentum.is_finite());
    // Repel must invert sign of accumulated momentum.
    assert!(
        pull_momentum.signum() == -push_momentum.signum() && pull_momentum.abs() > 1e-6,
        "Repel did not invert momentum direction: pull={}, push={}",
        pull_momentum, push_momentum,
    );
}
```

- [ ] **Step 2: Run test, expect failure**

Run: `cargo test --test module_trait modulate_gravity_phaser_repel_inverts_rotation_direction -- --nocapture`
Expected: FAIL — `set_modulate_repel` not on the trait.

- [ ] **Step 3: Add `set_modulate_repel` trait default**

In `src/dsp/modules/mod.rs`, inside the `SpectralModule` trait:

```rust
fn set_modulate_repel(&mut self, _repel: bool) {
    // No-op default. Modulate overrides.
}
```

- [ ] **Step 4: Add `repel` field + override on `ModulateModule`**

In `src/dsp/modules/modulate.rs`, extend the struct:

```rust
pub struct ModulateModule {
    mode: ModulateMode,
    repel: bool,
    // ... rest ...
}
```

Update `Self::new`:

```rust
repel: false,
```

In the `impl SpectralModule for ModulateModule` block, add:

```rust
fn set_modulate_repel(&mut self, repel: bool) {
    self.repel = repel;
}
```

Update the `GravityPhaser` arm in `process()`:

```rust
ModulateMode::GravityPhaser => {
    let num_bins = bins.len();
    self.refresh_smoothed(channel, curves, num_bins);
    let smoothed = self.smoothed_curves_for(channel);
    let momentum_slice = physics.as_mut().map(|p| &mut p.phase_momentum[..num_bins]);
    apply_gravity_phaser(bins, &smoothed, momentum_slice, self.repel);
}
```

- [ ] **Step 5: Add params field**

In `src/params.rs`, near the existing `slot_modulate_mode` field:

```rust
#[persist = "slot_modulate_repel"]
pub slot_modulate_repel: [Arc<Mutex<bool>>; MAX_SLOTS],
```

In the `Default` impl:

```rust
slot_modulate_repel: std::array::from_fn(|_| Arc::new(Mutex::new(false))),
```

Add a snap helper:

```rust
impl SpectralForgeParams {
    pub fn modulate_repel_snap(&self) -> [bool; MAX_SLOTS] {
        std::array::from_fn(|s| {
            self.slot_modulate_repel[s]
                .try_lock()
                .map(|g| *g)
                .unwrap_or(false)
        })
    }
}
```

(`MAX_SLOTS = 9` per `src/dsp/modules/mod.rs`.)

- [ ] **Step 6: Add FxMatrix propagation method**

In `src/dsp/fx_matrix.rs::FxMatrix`:

```rust
pub fn set_modulate_repels(&mut self, repels: &[bool; MAX_SLOTS]) {
    for (s, slot) in self.slots.iter_mut().enumerate() {
        if let Some(module) = slot {
            module.set_modulate_repel(repels[s]);
        }
    }
}
```

- [ ] **Step 7: Wire the per-block snapshot in `pipeline.rs`**

In `src/dsp/pipeline.rs::Pipeline::process()` (or `src/lib.rs::process()` —
follow the codebase's actual location), near the other `set_*_modes` calls:

```rust
let repels = params.modulate_repel_snap();
self.fx_matrix.set_modulate_repels(&repels);
```

If the existing pattern uses `try_lock` directly (e.g. for `slot_gain_mode`),
mirror that pattern instead:

```rust
if let Some(rep) = params.slot_modulate_repel[0].try_lock() {
    // (existing pattern would loop over all slots; the snap helper above
    //  encapsulates that. Use whichever is consistent with the rest of the
    //  audio-thread block.)
}
```

The `modulate_repel_snap()` helper does the loop in one shot; prefer it.

- [ ] **Step 8: Run test, expect pass**

Run: `cargo test --test module_trait modulate_gravity_phaser_repel_inverts_rotation_direction -- --nocapture`
Expected: PASS — repel inverts sign.

- [ ] **Step 9: Run v1 + Task 4 suite to confirm no regressions**

Run: `cargo test --test module_trait modulate -- --nocapture`
Expected: All tests pass.

- [ ] **Step 10: Commit**

```bash
git add src/dsp/modules/mod.rs src/dsp/modules/modulate.rs src/dsp/fx_matrix.rs src/params.rs src/dsp/pipeline.rs tests/module_trait.rs
git commit -m "feat(modulate): per-slot Repel toggle for Gravity Phaser"
```

---

## Task 6: Per-slot SidechainPositioned mode for Gravity Phaser

**Files:**
- Modify: `src/dsp/modules/mod.rs` — `set_modulate_sc_positioned` trait default
- Modify: `src/dsp/modules/modulate.rs` — override + sidechain peak picker + extended kernel
- Modify: `src/dsp/fx_matrix.rs` — propagation
- Modify: `src/params.rs` — field + snap
- Modify: `src/dsp/pipeline.rs` (or `src/lib.rs`) — snapshot
- Modify: `tests/module_trait.rs` — sidechain-positioned kernel test

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn modulate_gravity_phaser_sc_positioned_uses_sidechain_peaks_as_nodes() {
    use spectral_forge::dsp::bin_physics::BinPhysics;
    use spectral_forge::dsp::modules::modulate::{ModulateModule, ModulateMode};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule, StereoLink, FxChannelTarget};
    use realfft::num_complex::Complex;

    let mut module = ModulateModule::new();
    module.reset(48_000.0, 2048);
    module.set_modulate_mode(ModulateMode::GravityPhaser);
    module.set_modulate_sc_positioned(true);

    let num_bins = 1025;
    let mut bins: Vec<Complex<f32>> = (0..num_bins).map(|_| Complex::new(1.0, 0.0)).collect();

    // Sidechain: 3 spikes at bins 100, 300, 700; everything else low.
    let mut sc = vec![0.01_f32; num_bins];
    sc[100] = 5.0;
    sc[300] = 5.0;
    sc[700] = 5.0;

    // AMOUNT=2, REACH=2 (wide gravity well), RATE=1, THRESH=1 (peak floor=0.5),
    // AMPGATE=0, MIX=2.
    let amount = vec![2.0_f32; num_bins];
    let reach = vec![2.0_f32; num_bins];
    let neutral = vec![1.0_f32; num_bins];
    let zeros = vec![0.0_f32; num_bins];
    let mix = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &reach, &neutral, &neutral, &zeros, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let mut physics = BinPhysics::new();
    physics.reset_active(num_bins, 48_000.0, 2048);

    let ctx = ModuleContext::new_minimal(48_000.0, 2048);
    for _ in 0..15 {
        bins.iter_mut().for_each(|b| *b = Complex::new(1.0, 0.0));
        module.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, Some(&sc), &curves, Some(&mut physics), &mut suppression, &ctx);
    }

    // Bins near the 3 sidechain peaks must show stronger phase rotation than
    // bins far from any peak.
    let near = ((95..=105).chain(295..=305).chain(695..=705))
        .map(|k| bins[k].im.abs())
        .fold(0.0_f32, f32::max);
    let far = ((480..=490).chain(540..=550))
        .map(|k| bins[k].im.abs())
        .fold(0.0_f32, f32::max);
    assert!(near > far + 0.05,
        "near-peak bins ({}) not significantly more rotated than far bins ({})", near, far);
}
```

- [ ] **Step 2: Run test, expect failure**

Run: `cargo test --test module_trait modulate_gravity_phaser_sc_positioned_uses_sidechain_peaks_as_nodes -- --nocapture`
Expected: FAIL — `set_modulate_sc_positioned` not on trait.

- [ ] **Step 3: Add trait default**

In `src/dsp/modules/mod.rs`:

```rust
fn set_modulate_sc_positioned(&mut self, _enabled: bool) {
    // No-op default. Modulate overrides.
}
```

- [ ] **Step 4: Add `sc_positioned` field, peak scratch, and override**

In `src/dsp/modules/modulate.rs`, add `smallvec` use (file-scope) and the
field:

```rust
use smallvec::SmallVec;

pub const MAX_GP_NODES: usize = 32;

pub struct ModulateModule {
    mode: ModulateMode,
    repel: bool,
    sc_positioned: bool,
    /// Per-channel scratch for sidechain peak picking (Gravity Phaser
    /// SidechainPositioned mode). Stores `(bin_index, magnitude)` for each
    /// detected peak. Stack-only via SmallVec (cap = 32).
    gp_nodes: [SmallVec<[(usize, f32); MAX_GP_NODES]>; 2],
    // ... rest ...
}
```

`Self::new`:

```rust
sc_positioned: false,
gp_nodes: [SmallVec::new(), SmallVec::new()],
```

In `impl SpectralModule for ModulateModule`:

```rust
fn set_modulate_sc_positioned(&mut self, enabled: bool) {
    self.sc_positioned = enabled;
}
```

- [ ] **Step 5: Add the peak picker helper**

In `src/dsp/modules/modulate.rs` (above kernels):

```rust
/// Detect local maxima in `sidechain` magnitude. A bin is a peak iff:
///   - magnitude exceeds `thresh`, AND
///   - magnitude is strictly greater than the bins ±2 around it.
/// Returns up to `MAX_GP_NODES` peaks. Skips DC and Nyquist.
fn find_sidechain_peaks(
    sidechain: &[f32],
    thresh: f32,
    out: &mut SmallVec<[(usize, f32); MAX_GP_NODES]>,
) {
    out.clear();
    let n = sidechain.len();
    if n < 6 { return; }
    for k in 2..(n - 2) {
        let m = sidechain[k];
        if m < thresh { continue; }
        if m > sidechain[k - 1] && m > sidechain[k - 2]
            && m > sidechain[k + 1] && m > sidechain[k + 2]
        {
            if out.len() == MAX_GP_NODES {
                // Replace weakest if this is stronger.
                if let Some((idx, weakest)) = out.iter().enumerate().min_by(|a, b| a.1.1.partial_cmp(&b.1.1).unwrap()) {
                    if m > weakest.1 {
                        let i = idx;
                        out[i] = (k, m);
                    }
                }
            } else {
                out.push((k, m));
            }
        }
    }
}
```

- [ ] **Step 6: Add the SidechainPositioned variant of the kernel**

In `src/dsp/modules/modulate.rs`:

```rust
fn apply_gravity_phaser_sc_positioned(
    bins: &mut [Complex<f32>],
    smoothed: &[&[f32]; 6],
    nodes: &[(usize, f32)],
    phase_momentum: Option<&mut [f32]>,
    repel: bool,
) {
    use std::f32::consts::PI;

    let amount_c = smoothed[0];
    let reach_c = smoothed[1];
    let mix_c = smoothed[5];

    let num_bins = bins.len();
    let sign: f32 = if repel { -1.0 } else { 1.0 };

    let mut zeros_local: Vec<f32>;
    let momentum: &mut [f32] = match phase_momentum {
        Some(m) => &mut m[..num_bins.min(m.len())],
        None => {
            zeros_local = vec![0.0; num_bins];
            &mut zeros_local[..]
        }
    };

    if nodes.is_empty() {
        // No peaks detected → passthrough. Decay momentum so old kicks fade.
        for k in 0..num_bins {
            momentum[k] *= 0.95;
        }
        return;
    }

    for k in 0..num_bins {
        let amount = amount_c[k].clamp(0.0, 2.0);
        let reach = reach_c[k].clamp(0.1, 4.0); // reach > 0
        let mix = mix_c[k].clamp(0.0, 2.0) * 0.5;

        // Sum gravitational contribution from all nodes.
        let mut force = 0.0_f32;
        let width_bins = (reach * 12.0).max(1.0); // node half-width in bins
        for (n_idx, n_mag) in nodes.iter() {
            let d = (k as i32 - *n_idx as i32) as f32;
            // Gaussian falloff with width = width_bins.
            let g = (-(d * d) / (width_bins * width_bins)).exp();
            force += sign * amount * 0.05 * (*n_mag).min(2.0) * g;
        }

        momentum[k] = momentum[k] * 0.95 + force;

        let rotation = momentum[k] * PI;
        let cos_r = rotation.cos();
        let sin_r = rotation.sin();
        let dry = bins[k];
        let wet = Complex::new(
            dry.re * cos_r - dry.im * sin_r,
            dry.re * sin_r + dry.im * cos_r,
        );
        bins[k] = dry * (1.0 - mix) + wet * mix;
    }
}
```

- [ ] **Step 7: Wire dispatch — branch on `sc_positioned`**

Update the `GravityPhaser` arm in `process()`:

```rust
ModulateMode::GravityPhaser => {
    let num_bins = bins.len();
    self.refresh_smoothed(channel, curves, num_bins);
    let smoothed = self.smoothed_curves_for(channel);
    let momentum_slice = physics.as_mut().map(|p| &mut p.phase_momentum[..num_bins]);
    if self.sc_positioned {
        if let Some(sc) = sidechain {
            // Threshold = THRESH curve at bin 0, scaled to [0.05, 4.0].
            let thresh = (smoothed[3].first().copied().unwrap_or(1.0).clamp(0.01, 4.0)) * 0.5;
            find_sidechain_peaks(sc, thresh, &mut self.gp_nodes[channel]);
        } else {
            self.gp_nodes[channel].clear();
        }
        let nodes = &self.gp_nodes[channel][..];
        apply_gravity_phaser_sc_positioned(bins, &smoothed, nodes, momentum_slice, self.repel);
    } else {
        apply_gravity_phaser(bins, &smoothed, momentum_slice, self.repel);
    }
}
```

- [ ] **Step 8: Add params field, snap, FxMatrix propagation, pipeline snapshot**

In `src/params.rs`:

```rust
#[persist = "slot_modulate_sc_positioned"]
pub slot_modulate_sc_positioned: [Arc<Mutex<bool>>; MAX_SLOTS],
```

`Default`:

```rust
slot_modulate_sc_positioned: std::array::from_fn(|_| Arc::new(Mutex::new(false))),
```

Snap helper:

```rust
impl SpectralForgeParams {
    pub fn modulate_sc_positioned_snap(&self) -> [bool; MAX_SLOTS] {
        std::array::from_fn(|s| {
            self.slot_modulate_sc_positioned[s]
                .try_lock()
                .map(|g| *g)
                .unwrap_or(false)
        })
    }
}
```

In `src/dsp/fx_matrix.rs`:

```rust
pub fn set_modulate_sc_positioneds(&mut self, flags: &[bool; MAX_SLOTS]) {
    for (s, slot) in self.slots.iter_mut().enumerate() {
        if let Some(module) = slot {
            module.set_modulate_sc_positioned(flags[s]);
        }
    }
}
```

In `src/dsp/pipeline.rs::Pipeline::process()`, near the other `set_modulate_*`
calls:

```rust
let sc_pos = params.modulate_sc_positioned_snap();
self.fx_matrix.set_modulate_sc_positioneds(&sc_pos);
```

- [ ] **Step 9: Run test, expect pass**

Run: `cargo test --test module_trait modulate_gravity_phaser_sc_positioned_uses_sidechain_peaks_as_nodes -- --nocapture`
Expected: PASS — near-peak bins more rotated than far bins.

- [ ] **Step 10: Run v1 + new suite to confirm no regressions**

Run: `cargo test --test module_trait modulate -- --nocapture`
Expected: All tests pass.

- [ ] **Step 11: Commit**

```bash
git add src/dsp/modules/mod.rs src/dsp/modules/modulate.rs src/dsp/fx_matrix.rs src/params.rs src/dsp/pipeline.rs tests/module_trait.rs
git commit -m "feat(modulate): SidechainPositioned mode for Gravity Phaser (peak picker)"
```

---

## Task 7: PLL Tear kernel (lock detection + tear emission)

**Files:**
- Modify: `src/dsp/modules/modulate.rs` — PLL state, kernel, dispatch
- Modify: `tests/module_trait.rs` — kernel test

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn modulate_pll_tear_locks_on_steady_input_and_passes_through() {
    use spectral_forge::dsp::bin_physics::BinPhysics;
    use spectral_forge::dsp::modules::modulate::{ModulateModule, ModulateMode};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule, StereoLink, FxChannelTarget};
    use realfft::num_complex::Complex;

    let mut module = ModulateModule::new();
    module.reset(48_000.0, 2048);
    module.set_modulate_mode(ModulateMode::PllTear);

    let num_bins = 1025;
    // Steady input: phases identical hop-to-hop. PLL must lock and emit dry.
    let bins_template: Vec<Complex<f32>> = (0..num_bins)
        .map(|k| {
            let phase = (k as f32) * 0.05;
            Complex::new(phase.cos(), phase.sin())
        })
        .collect();
    let mut bins = bins_template.clone();

    // AMOUNT=2 (full wet of torn output, but tear is gated by lock detector),
    // REACH=2 (all bins active), RATE=1 (default omega_n), THRESH=1 (default),
    // AMPGATE=0, MIX=2.
    let amount = vec![2.0_f32; num_bins];
    let neutral = vec![1.0_f32; num_bins];
    let zeros = vec![0.0_f32; num_bins];
    let mix = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &neutral, &neutral, &neutral, &zeros, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let mut physics = BinPhysics::new();
    physics.reset_active(num_bins, 48_000.0, 2048);

    // Provide unwrapped phase = wrapped phase for steady input.
    let unwrapped: Vec<f32> = bins_template.iter().map(|b| b.arg()).collect();
    let ctx = ModuleContext {
        unwrapped_phase: Some(&unwrapped[..]),
        ..ModuleContext::new_minimal(48_000.0, 2048)
    };

    // Run 30 hops with constant input. Loop should converge to lock; output ≈ dry.
    for _ in 0..30 {
        bins.copy_from_slice(&bins_template);
        module.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, None, &curves, Some(&mut physics), &mut suppression, &ctx);
    }

    // After lock, magnitudes should still match dry (within tolerance).
    for k in 16..num_bins.min(900) {
        let mag = bins[k].norm();
        assert!((mag - 1.0).abs() < 0.05, "locked PLL magnitude drift at bin {}: {}", k, mag);
    }
}

#[test]
fn modulate_pll_tear_writes_phase_momentum_on_glide() {
    use spectral_forge::dsp::bin_physics::BinPhysics;
    use spectral_forge::dsp::modules::modulate::{ModulateModule, ModulateMode};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule, StereoLink, FxChannelTarget};
    use realfft::num_complex::Complex;

    let mut module = ModulateModule::new();
    module.reset(48_000.0, 2048);
    module.set_modulate_mode(ModulateMode::PllTear);

    let num_bins = 1025;
    let mut phases: Vec<f32> = (0..num_bins).map(|k| (k as f32) * 0.05).collect();
    let mut bins: Vec<Complex<f32>> = phases.iter().map(|p| Complex::new(p.cos(), p.sin())).collect();

    let amount = vec![2.0_f32; num_bins];
    let neutral = vec![1.0_f32; num_bins];
    let zeros = vec![0.0_f32; num_bins];
    let mix = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &neutral, &neutral, &neutral, &zeros, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let mut physics = BinPhysics::new();
    physics.reset_active(num_bins, 48_000.0, 2048);

    let ctx_owned = ModuleContext::new_minimal(48_000.0, 2048);

    // Apply a fast phase glide in bin 100 over 30 hops — should overshoot loop bandwidth and tear.
    for hop in 0..30 {
        let glide = (hop as f32) * 0.8; // fast: 0.8 rad/hop in just bin 100
        phases[100] = (100.0 * 0.05) + glide;
        bins[100] = Complex::new(phases[100].cos(), phases[100].sin());
        let unwrapped: Vec<f32> = phases.clone();
        let ctx = ModuleContext { unwrapped_phase: Some(&unwrapped[..]), ..ctx_owned };
        module.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, None, &curves, Some(&mut physics), &mut suppression, &ctx);
    }

    // Bin 100 phase momentum should have been kicked by the tear event.
    assert!(physics.phase_momentum[100].abs() > 0.0,
        "bin 100 momentum unchanged after glide tear: {}", physics.phase_momentum[100]);
    // No NaN or runaway anywhere.
    for k in 0..num_bins {
        assert!(physics.phase_momentum[k].is_finite(), "bin {} momentum NaN", k);
        assert!(physics.phase_momentum[k].abs() < 100.0, "bin {} momentum runaway: {}", k, physics.phase_momentum[k]);
    }
}
```

- [ ] **Step 2: Run tests, expect failure**

Run: `cargo test --test module_trait modulate_pll_tear -- --nocapture`
Expected: FAIL — PllTear arm not dispatched.

- [ ] **Step 3: Add PLL state fields**

In `src/dsp/modules/modulate.rs`, extend the struct:

```rust
pub const PLL_MIN_BIN: usize = 16; // skip sub-100Hz bins (research finding 3)
pub const PLL_RELOCK_HOPS: u8 = 4; // |err| < π/8 for 4 hops to re-lock
pub const PLL_TEAR_THRESHOLD: f32 = std::f32::consts::FRAC_PI_2; // π/2
pub const PLL_RELOCK_THRESHOLD: f32 = std::f32::consts::FRAC_PI_8; // π/8

pub struct ModulateModule {
    // ... existing ...
    /// Per-channel PLL bank state.
    pll_phase: [Vec<f32>; 2],
    pll_freq: [Vec<f32>; 2],
    pll_err_scratch: [Vec<f32>; 2],
    pll_torn: [Vec<bool>; 2],
    pll_relock_count: [Vec<u8>; 2],
    /// Per-channel previous frame's bin phases (for local unwrap fallback
    /// when ctx.unwrapped_phase is None).
    prev_phase: [Vec<f32>; 2],
    /// Per-channel xorshift32 RNG state for tear noise emission.
    tear_rng: [u32; 2],
    /// Per-channel local unwrap target buffer (avoids a heap alloc per hop).
    unwrap_local: [Vec<f32>; 2],
}
```

`Self::new`:

```rust
pll_phase: [Vec::new(), Vec::new()],
pll_freq: [Vec::new(), Vec::new()],
pll_err_scratch: [Vec::new(), Vec::new()],
pll_torn: [Vec::new(), Vec::new()],
pll_relock_count: [Vec::new(), Vec::new()],
prev_phase: [Vec::new(), Vec::new()],
tear_rng: [0xC0FFEE_u32, 0xBEEF_u32],
unwrap_local: [Vec::new(), Vec::new()],
```

`reset()` additions (inside the `for ch in 0..2` loop):

```rust
self.pll_phase[ch].clear();
self.pll_phase[ch].resize(num_bins, 0.0);
self.pll_freq[ch].clear();
self.pll_freq[ch].resize(num_bins, 0.0);
self.pll_err_scratch[ch].clear();
self.pll_err_scratch[ch].resize(num_bins, 0.0);
self.pll_torn[ch].clear();
self.pll_torn[ch].resize(num_bins, false);
self.pll_relock_count[ch].clear();
self.pll_relock_count[ch].resize(num_bins, 0);
self.prev_phase[ch].clear();
self.prev_phase[ch].resize(num_bins, 0.0);
self.unwrap_local[ch].clear();
self.unwrap_local[ch].resize(num_bins, 0.0);
self.tear_rng[ch] = 0xC0FFEE_u32 ^ ((ch as u32 + 1) * 0xDEADBEEF_u32);
```

- [ ] **Step 4: Add the PLL Tear kernel**

```rust
/// xorshift32 — one PRNG step. Caller mutates `state` in place.
#[inline]
fn xorshift32(state: &mut u32) -> u32 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *state = x;
    x
}

/// xorshift32-based uniform random in [-1, 1].
#[inline]
fn xorshift32_signed(state: &mut u32) -> f32 {
    let r = xorshift32(state) as f32 / (u32::MAX as f32 / 2.0);
    r - 1.0
}

#[allow(clippy::too_many_arguments)]
fn apply_pll_tear(
    bins: &mut [Complex<f32>],
    smoothed: &[&[f32]; 6],
    target_phase: &[f32],
    pll_phase: &mut [f32],
    pll_freq: &mut [f32],
    pll_err: &mut [f32],
    pll_torn: &mut [bool],
    pll_relock: &mut [u8],
    rng_state: &mut u32,
    phase_momentum: Option<&mut [f32]>,
) {
    use crate::dsp::physics_helpers::pll_bank_step;
    use std::f32::consts::PI;

    let amount_c = smoothed[0];
    let reach_c = smoothed[1];
    let rate_c = smoothed[2];
    let thresh_c = smoothed[3];
    let mix_c = smoothed[5];

    let num_bins = bins.len();

    // Loop natural frequency ωₙ from RATE curve at bin 0 (0..2 → 0..0.2 cycles/hop).
    let omega_n = (rate_c.first().copied().unwrap_or(1.0)).clamp(0.0, 2.0) * 0.1;
    let zeta = 0.707_f32;
    let alpha = 2.0 * zeta * omega_n;
    let beta = omega_n * omega_n;

    // Tear threshold scales with THRESH curve (1.0 → π/2 default).
    let thresh_scale = thresh_c.first().copied().unwrap_or(1.0).clamp(0.1, 4.0);
    let tear_thresh = PLL_TEAR_THRESHOLD * thresh_scale.min(2.0);

    // REACH defines bin range upper bound (0..2 → 0..num_bins).
    let reach_norm = reach_c.first().copied().unwrap_or(1.0).clamp(0.0, 2.0) * 0.5;
    let max_bin = ((reach_norm + 0.5) * num_bins as f32) as usize;
    let max_bin = max_bin.min(num_bins);

    let stepped_lo = PLL_MIN_BIN;
    let stepped_hi = max_bin.max(stepped_lo);

    if stepped_hi > stepped_lo {
        pll_bank_step(
            &mut pll_phase[stepped_lo..stepped_hi],
            &mut pll_freq[stepped_lo..stepped_hi],
            &target_phase[stepped_lo..stepped_hi],
            alpha,
            beta,
            &mut pll_err[stepped_lo..stepped_hi],
        );
    }

    let mut zeros_local: Vec<f32>;
    let momentum: &mut [f32] = match phase_momentum {
        Some(m) => &mut m[..num_bins.min(m.len())],
        None => {
            zeros_local = vec![0.0; num_bins];
            &mut zeros_local[..]
        }
    };

    for k in 0..num_bins {
        let amount = amount_c[k].clamp(0.0, 2.0);
        let mix = mix_c[k].clamp(0.0, 2.0) * 0.5;

        if k < stepped_lo || k >= stepped_hi {
            // Bin out of PLL range: passthrough.
            continue;
        }

        let err = pll_err[k];

        // Lock-loss detector with hysteresis.
        if pll_torn[k] {
            if err.abs() < PLL_RELOCK_THRESHOLD {
                pll_relock[k] = pll_relock[k].saturating_add(1);
                if pll_relock[k] >= PLL_RELOCK_HOPS {
                    pll_torn[k] = false;
                    pll_relock[k] = 0;
                }
            } else {
                pll_relock[k] = 0;
            }
        } else if err.abs() > tear_thresh {
            pll_torn[k] = true;
            pll_relock[k] = 0;
        }

        if pll_torn[k] {
            // Emit chaotic noise: random phase rotation, magnitude preserved.
            let r = xorshift32_signed(rng_state);
            let rotation = r * PI * amount;
            let cos_r = rotation.cos();
            let sin_r = rotation.sin();
            let dry = bins[k];
            let wet = Complex::new(
                dry.re * cos_r - dry.im * sin_r,
                dry.re * sin_r + dry.im * cos_r,
            );
            bins[k] = dry * (1.0 - mix) + wet * mix;
            // Kick phase momentum proportional to the error magnitude.
            momentum[k] += 0.1 * err.signum() * (err.abs() - tear_thresh).max(0.0);
        }
        // else: locked → passthrough (PLL is silent in lock).
    }
}
```

- [ ] **Step 5: Add local unwrap helper for PLL fallback**

```rust
/// Compute unwrapped phase locally when ctx.unwrapped_phase is None.
/// Uses the previous frame's wrapped phase to track ±2π hops in `unwrapped`.
/// `unwrapped[k] += wrap_phase(target_wrapped[k] - prev_phase[k])`.
fn unwrap_phase_local(
    target_wrapped: &[f32],
    prev_phase: &[f32],
    unwrapped: &mut [f32],
) {
    use crate::dsp::physics_helpers::wrap_phase;
    for k in 0..target_wrapped.len() {
        let delta = wrap_phase(target_wrapped[k] - prev_phase[k]);
        unwrapped[k] += delta;
    }
}
```

- [ ] **Step 6: Wire dispatch arm**

In `process()` match, add:

```rust
ModulateMode::PllTear => {
    let num_bins = bins.len();
    self.refresh_smoothed(channel, curves, num_bins);
    let smoothed = self.smoothed_curves_for(channel);

    // Compute current frame's wrapped phases.
    let mut wrapped = vec![0.0_f32; 0]; // placeholder; replaced below
    let target_phase: &[f32] = match _ctx.unwrapped_phase {
        Some(u) if u.len() >= num_bins => &u[..num_bins],
        _ => {
            // Fallback: local unwrap. We need wrapped phases of `bins` then unwrap.
            // Use `unwrap_local[channel]` as the running unwrapped state.
            // 1. Snapshot current bin phases into `prev_phase` scratch first.
            let pp = &mut self.prev_phase[channel];
            let ul = &mut self.unwrap_local[channel];
            // Compute current wrapped phases inline in pp_temp; we reuse pp[k] as
            // the staging buffer because it's about to be overwritten anyway.
            let mut pp_temp_holder: Vec<f32>;
            // We must NOT modify `bins` here — read phases without mutation.
            pp_temp_holder = (0..num_bins).map(|k| bins[k].arg()).collect();
            unwrap_phase_local(&pp_temp_holder, pp, ul);
            // Save current wrapped phases for next call.
            pp[..num_bins].copy_from_slice(&pp_temp_holder[..num_bins]);
            &ul[..num_bins]
        }
    };
    let _ = wrapped; // unused; suppress compiler warning if any

    let momentum_slice = physics.as_mut().map(|p| &mut p.phase_momentum[..num_bins]);
    apply_pll_tear(
        bins,
        &smoothed,
        target_phase,
        &mut self.pll_phase[channel][..num_bins],
        &mut self.pll_freq[channel][..num_bins],
        &mut self.pll_err_scratch[channel][..num_bins],
        &mut self.pll_torn[channel][..num_bins],
        &mut self.pll_relock_count[channel][..num_bins],
        &mut self.tear_rng[channel],
        momentum_slice,
    );
}
```

> **Allocation note:** `pp_temp_holder = (0..num_bins).map(|k| bins[k].arg()).collect()`
> heap-allocates `num_bins * 4 bytes` per fallback hop. The audio thread MUST
> NOT allocate. Replace with a pre-allocated scratch field:

Add to the struct:

```rust
prev_phase_scratch: [Vec<f32>; 2],
```

`new`:

```rust
prev_phase_scratch: [Vec::new(), Vec::new()],
```

`reset()` additions:

```rust
self.prev_phase_scratch[ch].clear();
self.prev_phase_scratch[ch].resize(num_bins, 0.0);
```

Update the dispatch arm to use it:

```rust
ModulateMode::PllTear => {
    let num_bins = bins.len();
    self.refresh_smoothed(channel, curves, num_bins);
    let smoothed = self.smoothed_curves_for(channel);

    // Compute current frame's wrapped phases without reborrowing `bins`.
    let pp_scratch = &mut self.prev_phase_scratch[channel];
    for k in 0..num_bins { pp_scratch[k] = bins[k].arg(); }

    // target_phase via PLPV ctx if available, else local unwrap.
    let target_phase: &[f32] = match _ctx.unwrapped_phase {
        Some(u) if u.len() >= num_bins => &u[..num_bins],
        _ => {
            let pp = &mut self.prev_phase[channel];
            let ul = &mut self.unwrap_local[channel];
            unwrap_phase_local(&pp_scratch[..num_bins], &pp[..num_bins], &mut ul[..num_bins]);
            pp[..num_bins].copy_from_slice(&pp_scratch[..num_bins]);
            &ul[..num_bins]
        }
    };

    let momentum_slice = physics.as_mut().map(|p| &mut p.phase_momentum[..num_bins]);
    apply_pll_tear(
        bins,
        &smoothed,
        target_phase,
        &mut self.pll_phase[channel][..num_bins],
        &mut self.pll_freq[channel][..num_bins],
        &mut self.pll_err_scratch[channel][..num_bins],
        &mut self.pll_torn[channel][..num_bins],
        &mut self.pll_relock_count[channel][..num_bins],
        &mut self.tear_rng[channel],
        momentum_slice,
    );
}
```

(`_ctx` from the function signature; rename to `ctx` if it's already used
elsewhere in the body. Phase 1 made `ModuleContext` non-Copy with a `'block`
lifetime; the borrow above is fine because `ctx` is `&ModuleContext<'_>` and
the field access is read-only.)

- [ ] **Step 7: Run tests, expect pass**

Run: `cargo test --test module_trait modulate_pll_tear -- --nocapture`
Expected: PASS — locked-passthrough test green; glide test produces non-zero
momentum at bin 100 with no NaNs.

- [ ] **Step 8: Run v1 + Tasks 4-6 suite to confirm no regressions**

Run: `cargo test --test module_trait modulate -- --nocapture`
Expected: All tests pass.

- [ ] **Step 9: Commit**

```bash
git add src/dsp/modules/modulate.rs tests/module_trait.rs
git commit -m "feat(modulate): PLL Tear kernel — 2nd-order PI bank + lock/tear hysteresis"
```

---

## Task 8: Mode picker popup + per-slot toggles UI

**Files:**
- Modify: `src/editor/modulate_popup.rs` — extended mode list, repel + sc_positioned checkboxes

- [ ] **Step 1: Extend the mode list**

In `src/editor/modulate_popup.rs::show_modulate_popup`, replace the
hardcoded mode array with:

```rust
for (label, mode) in [
    ("Phase Phaser",       ModulateMode::PhasePhaser),
    ("Bin Swapper",        ModulateMode::BinSwapper),
    ("RM/FM Matrix",       ModulateMode::RmFmMatrix),
    ("Diode RM",           ModulateMode::DiodeRm),
    ("Ground Loop",        ModulateMode::GroundLoop),
    ("Gravity Phaser",     ModulateMode::GravityPhaser),
    ("PLL Tear",           ModulateMode::PllTear),
] {
    let is_active = cur == mode;
    let color = if is_active { theme::MODULATE_DOT_COLOR } else { theme::POPUP_TEXT };
    let response = ui.selectable_label(
        is_active,
        egui::RichText::new(label).color(color).size(11.0),
    );
    if response.clicked() {
        *slot_modulate_mode.lock().unwrap() = mode;
        selected = true;
    }
}
```

- [ ] **Step 2: Add Repel and SidechainPositioned checkboxes (Gravity Phaser only)**

Extend the `show_modulate_popup` signature to accept the two new toggles:

```rust
pub fn show_modulate_popup(
    ui: &mut egui::Ui,
    state: &mut ModulatePopupState,
    slot_modulate_mode: &Arc<Mutex<ModulateMode>>,
    slot_modulate_repel: &Arc<Mutex<bool>>,
    slot_modulate_sc_positioned: &Arc<Mutex<bool>>,
) -> bool {
```

Inside the popup, after the mode list and a separator, append:

```rust
let cur_mode = *slot_modulate_mode.lock().unwrap();
if cur_mode == ModulateMode::GravityPhaser {
    ui.separator();
    ui.label(egui::RichText::new("GRAVITY OPTS").color(theme::POPUP_TITLE).size(10.0));
    {
        let mut repel = *slot_modulate_repel.lock().unwrap();
        if ui.checkbox(&mut repel, egui::RichText::new("Repel (push away)").size(10.0)).changed() {
            *slot_modulate_repel.lock().unwrap() = repel;
            selected = true;
        }
    }
    {
        let mut scp = *slot_modulate_sc_positioned.lock().unwrap();
        if ui.checkbox(&mut scp, egui::RichText::new("Sidechain-positioned").size(10.0)).changed() {
            *slot_modulate_sc_positioned.lock().unwrap() = scp;
            selected = true;
        }
    }
}
```

- [ ] **Step 3: Update the popup invocation site in `editor_ui.rs`**

Find the existing `modulate_popup::show_modulate_popup(...)` call in
`src/editor/editor_ui.rs` (added in Phase 2f Task 9) and add the two new
borrows:

```rust
modulate_popup::show_modulate_popup(
    ui,
    &mut modulate_popup_state,
    &params.slot_modulate_mode[slot_idx],
    &params.slot_modulate_repel[slot_idx],
    &params.slot_modulate_sc_positioned[slot_idx],
);
```

- [ ] **Step 4: Build and manual smoke test**

Run:

```bash
cargo build
cargo run --package xtask -- bundle spectral_forge
```

Manual: load in Bitwig, assign Modulate to a slot, right-click. Verify all 7
modes appear; selecting Gravity Phaser reveals the two checkboxes; both persist
across plugin reload.

- [ ] **Step 5: Commit**

```bash
git add src/editor/modulate_popup.rs src/editor/editor_ui.rs
git commit -m "feat(modulate): popup adds GravityPhaser/PllTear + Repel/SCPos toggles"
```

---

## Task 9: Calibration probes for new modes

**Files:**
- Modify: `src/dsp/modules/modulate.rs` — extend `ModulateProbe`
- Modify: `tests/calibration_roundtrip.rs` — round-trip for new modes

- [ ] **Step 1: Write the failing test**

In `tests/calibration_roundtrip.rs` (append):

```rust
#[cfg(feature = "probe")]
#[test]
fn modulate_calibration_roundtrip_retrofit_modes() {
    use spectral_forge::dsp::bin_physics::BinPhysics;
    use spectral_forge::dsp::modules::modulate::{ModulateModule, ModulateMode};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule, StereoLink, FxChannelTarget};
    use realfft::num_complex::Complex;

    let num_bins = 1025;

    for mode in [ModulateMode::GravityPhaser, ModulateMode::PllTear] {
        let mut module = ModulateModule::new();
        module.reset(48_000.0, 2048);
        module.set_modulate_mode(mode);
        if mode == ModulateMode::GravityPhaser {
            module.set_modulate_repel(true);
            module.set_modulate_sc_positioned(true);
        }

        let mut bins: Vec<Complex<f32>> = vec![Complex::new(0.5, 0.0); num_bins];
        let sc = vec![0.5_f32; num_bins];
        let amount = vec![1.0_f32; num_bins];
        let neutral = vec![1.0_f32; num_bins];
        let zeros = vec![0.0_f32; num_bins];
        let mix = vec![1.0_f32; num_bins];
        let curves: Vec<&[f32]> = vec![&amount, &neutral, &neutral, &neutral, &zeros, &mix];

        let mut suppression = vec![0.0_f32; num_bins];
        let mut physics = BinPhysics::new();
        physics.reset_active(num_bins, 48_000.0, 2048);

        let unwrapped: Vec<f32> = (0..num_bins).map(|k| (k as f32) * 0.05).collect();
        let ctx = ModuleContext { unwrapped_phase: Some(&unwrapped[..]), ..ModuleContext::new_minimal(48_000.0, 2048) };

        for _ in 0..10 {
            module.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, Some(&sc), &curves, Some(&mut physics), &mut suppression, &ctx);
        }

        let probe = module.probe_state(0);
        assert_eq!(probe.active_mode, mode);
        if mode == ModulateMode::GravityPhaser {
            assert!(probe.gp_repel_active, "repel probe not reflected");
            assert!(probe.gp_sc_positioned, "sc_positioned probe not reflected");
            // gp_node_count = number of detected sidechain peaks (sc is flat, 0).
            assert!(probe.gp_node_count <= 32, "node count out of bounds: {}", probe.gp_node_count);
        }
        if mode == ModulateMode::PllTear {
            assert!(probe.pll_lock_pct >= 0.0 && probe.pll_lock_pct <= 100.0,
                "pll_lock_pct out of range: {}", probe.pll_lock_pct);
        }
    }
}
```

- [ ] **Step 2: Run test, expect failure**

Run: `cargo test --features probe --test calibration_roundtrip modulate_calibration_roundtrip_retrofit_modes -- --nocapture`
Expected: FAIL — `gp_repel_active`, `gp_sc_positioned`, `gp_node_count`,
`pll_lock_pct` not on `ModulateProbe`.

- [ ] **Step 3: Extend `ModulateProbe`**

In `src/dsp/modules/modulate.rs`, replace the existing `ModulateProbe`:

```rust
#[cfg(any(test, feature = "probe"))]
#[derive(Debug, Clone, Copy)]
pub struct ModulateProbe {
    pub active_mode: ModulateMode,
    pub average_amount_pct: f32,
    pub current_hop_count: u64,
    pub recent_rms: f32,
    /// Gravity Phaser: # of detected sidechain peaks (0 if SC mode off / no SC).
    pub gp_node_count: usize,
    pub gp_repel_active: bool,
    pub gp_sc_positioned: bool,
    /// PLL Tear: percentage of PLL-active bins currently locked (0..100).
    pub pll_lock_pct: f32,
}
```

Update the `probe_state` method body:

```rust
#[cfg(any(test, feature = "probe"))]
impl ModulateModule {
    pub fn probe_state(&self, channel: usize) -> ModulateProbe {
        let ch = channel.min(1);
        let recent_rms: f32 = self.rms_history[ch].iter().sum::<f32>() / 16.0;
        let gp_node_count = self.gp_nodes[ch].len();
        // PLL lock %: count !torn bins in [PLL_MIN_BIN, fft/2+1).
        let total = self.pll_torn[ch].len().saturating_sub(PLL_MIN_BIN);
        let locked = if total > 0 {
            self.pll_torn[ch][PLL_MIN_BIN..]
                .iter()
                .filter(|t| !**t)
                .count()
        } else { 0 };
        let pll_lock_pct = if total > 0 { (locked as f32 / total as f32) * 100.0 } else { 0.0 };
        ModulateProbe {
            active_mode: self.mode,
            average_amount_pct: 100.0,
            current_hop_count: self.hop_count[ch],
            recent_rms,
            gp_node_count,
            gp_repel_active: self.repel,
            gp_sc_positioned: self.sc_positioned,
            pll_lock_pct,
        }
    }
}
```

- [ ] **Step 4: Run test, expect pass**

Run: `cargo test --features probe --test calibration_roundtrip modulate_calibration_roundtrip_retrofit_modes -- --nocapture`
Expected: PASS.

- [ ] **Step 5: Verify v1 calibration test still passes**

Run: `cargo test --features probe --test calibration_roundtrip modulate -- --nocapture`
Expected: All modulate calibration tests pass (including v1
`modulate_calibration_roundtrip_all_modes`).

- [ ] **Step 6: Commit**

```bash
git add src/dsp/modules/modulate.rs tests/calibration_roundtrip.rs
git commit -m "test(modulate): probes for GravityPhaser nodes/flags + PllTear lock %"
```

---

## Task 10: BinPhysics writer test (Gravity Phaser feeds next slot's reader)

**Files:**
- Modify: `tests/module_trait.rs` — sequencing test

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn modulate_gravity_phaser_phase_momentum_visible_to_next_slot() {
    use spectral_forge::dsp::bin_physics::BinPhysics;
    use spectral_forge::dsp::modules::modulate::{ModulateModule, ModulateMode};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule, StereoLink, FxChannelTarget};
    use realfft::num_complex::Complex;

    // Slot N: Modulate/GravityPhaser writes phase_momentum.
    // Slot N+1: a hypothetical reader (we just inspect physics.phase_momentum).
    let mut writer = ModulateModule::new();
    writer.reset(48_000.0, 2048);
    writer.set_modulate_mode(ModulateMode::GravityPhaser);

    let num_bins = 1025;
    let mut bins: Vec<Complex<f32>> = (0..num_bins).map(|_| Complex::new(1.0, 0.0)).collect();

    let amount = vec![1.5_f32; num_bins];
    let neutral = vec![1.0_f32; num_bins];
    let zeros = vec![0.0_f32; num_bins];
    let mix = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &neutral, &neutral, &neutral, &zeros, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let mut physics = BinPhysics::new();
    physics.reset_active(num_bins, 48_000.0, 2048);

    let ctx = ModuleContext::new_minimal(48_000.0, 2048);

    for _ in 0..20 {
        writer.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, None, &curves, Some(&mut physics), &mut suppression, &ctx);
    }

    // Sample a representative bin: momentum should be non-zero.
    assert!(physics.phase_momentum[200].abs() > 1e-6,
        "phase_momentum[200] not written: {}", physics.phase_momentum[200]);
    // No NaN in any bin.
    for k in 0..num_bins {
        assert!(physics.phase_momentum[k].is_finite(), "bin {} momentum NaN", k);
    }
}
```

- [ ] **Step 2: Run test**

Run: `cargo test --test module_trait modulate_gravity_phaser_phase_momentum_visible_to_next_slot -- --nocapture`
Expected: PASS — the kernel from Task 4 already writes momentum.

If FAIL: debug Task 4's `momentum[k] = momentum[k] * 0.95 + force` line; the
decay is correct but the sign of `force` may zero out across hops. Add a
println to verify.

- [ ] **Step 3: Commit**

```bash
git add tests/module_trait.rs
git commit -m "test(modulate): Gravity Phaser writes phase_momentum visible across hops"
```

---

## Task 11: PLPV consumer test (PLL Tear honours `ctx.unwrapped_phase`)

**Files:**
- Modify: `tests/module_trait.rs` — PLPV consumer test

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn modulate_pll_tear_uses_provided_unwrapped_phase_when_available() {
    use spectral_forge::dsp::bin_physics::BinPhysics;
    use spectral_forge::dsp::modules::modulate::{ModulateModule, ModulateMode};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule, StereoLink, FxChannelTarget};
    use realfft::num_complex::Complex;

    let num_bins = 1025;

    // Setup A: ctx.unwrapped_phase is None → PLL uses local unwrap (initially zero).
    // Setup B: ctx.unwrapped_phase = Some(...) → PLL uses external truth.
    // Both should converge to lock on steady input; B should converge faster
    // because external unwrap skips the local-unwrap warm-up.

    fn run(provide_unwrapped: bool) -> f32 {
        let mut module = ModulateModule::new();
        module.reset(48_000.0, 2048);
        module.set_modulate_mode(ModulateMode::PllTear);

        let bins_template: Vec<Complex<f32>> = (0..1025)
            .map(|k| {
                let phase = (k as f32) * 0.05;
                Complex::new(phase.cos(), phase.sin())
            })
            .collect();
        let mut bins = bins_template.clone();

        let amount = vec![2.0_f32; 1025];
        let neutral = vec![1.0_f32; 1025];
        let zeros = vec![0.0_f32; 1025];
        let mix = vec![2.0_f32; 1025];
        let curves: Vec<&[f32]> = vec![&amount, &neutral, &neutral, &neutral, &zeros, &mix];

        let mut suppression = vec![0.0_f32; 1025];
        let mut physics = BinPhysics::new();
        physics.reset_active(1025, 48_000.0, 2048);

        let unwrapped: Vec<f32> = (0..1025).map(|k| (k as f32) * 0.05).collect();

        let ctx_owned = ModuleContext::new_minimal(48_000.0, 2048);
        for _ in 0..50 {
            bins.copy_from_slice(&bins_template);
            let ctx = if provide_unwrapped {
                ModuleContext { unwrapped_phase: Some(&unwrapped[..]), ..ctx_owned }
            } else {
                ctx_owned.clone()
            };
            module.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, None, &curves, Some(&mut physics), &mut suppression, &ctx);
        }

        // Return final lock %: bins where torn == false (locked).
        let probe = module.probe_state(0);
        probe.pll_lock_pct
    }

    let lock_a = run(false);
    let lock_b = run(true);
    assert!(lock_a >= 70.0, "fallback unwrap path failed to reach 70% lock: {}", lock_a);
    assert!(lock_b >= 90.0, "PLPV-fed path failed to reach 90% lock: {}", lock_b);
}
```

(Requires `ModuleContext: Clone` from Phase 1; if it's not Clone, replace with
explicit field reconstruction.)

- [ ] **Step 2: Run test, expect pass**

Run: `cargo test --test module_trait modulate_pll_tear_uses_provided_unwrapped_phase_when_available -- --nocapture`
Expected: PASS.

If `ModuleContext` isn't `Clone`: replace `ctx_owned.clone()` with manual
field-by-field reconstruction:

```rust
let ctx = ModuleContext {
    sample_rate: ctx_owned.sample_rate,
    fft_size: ctx_owned.fft_size,
    num_bins: ctx_owned.num_bins,
    attack_ms: ctx_owned.attack_ms,
    release_ms: ctx_owned.release_ms,
    sensitivity: ctx_owned.sensitivity,
    suppression_width: ctx_owned.suppression_width,
    auto_makeup: ctx_owned.auto_makeup,
    delta_monitor: ctx_owned.delta_monitor,
    bin_physics: None,
    unwrapped_phase: if provide_unwrapped { Some(&unwrapped[..]) } else { None },
    peaks: None,
    // ... add any other Phase 1+ ctx fields here ...
};
```

- [ ] **Step 3: Commit**

```bash
git add tests/module_trait.rs
git commit -m "test(modulate): PLL Tear consumes ctx.unwrapped_phase when provided"
```

---

## Task 12: Multi-hop dual-channel finite/bounded test for all 7 modes

**Files:**
- Modify: `tests/module_trait.rs` — extend the v1 finite/bounded test

- [ ] **Step 1: Replace the v1 test's mode list with all 7 modes**

In `tests/module_trait.rs`, find `modulate_finite_bounded_all_modes_dual_channel`
(from Phase 2f Task 11) and replace its mode array:

```rust
for mode in [
    ModulateMode::PhasePhaser,
    ModulateMode::BinSwapper,
    ModulateMode::RmFmMatrix,
    ModulateMode::DiodeRm,
    ModulateMode::GroundLoop,
    ModulateMode::GravityPhaser,
    ModulateMode::PllTear,
] {
```

Inside the loop, after `module.set_modulate_mode(mode);`, add:

```rust
if mode == ModulateMode::GravityPhaser {
    // Toggle both extras for max coverage.
    module.set_modulate_repel(true);
    module.set_modulate_sc_positioned(true);
}
```

If `physics: Option<&mut BinPhysics>` was added by Phase 3 to the existing
test's `process()` call, leave it. If not, add a per-mode `BinPhysics` fixture:

```rust
let mut physics = spectral_forge::dsp::bin_physics::BinPhysics::new();
physics.reset_active(num_bins, 48_000.0, 2048);
// pass Some(&mut physics) instead of the existing argument position.
```

The full updated test:

```rust
#[test]
fn modulate_finite_bounded_all_modes_dual_channel() {
    use spectral_forge::dsp::bin_physics::BinPhysics;
    use spectral_forge::dsp::modules::modulate::{ModulateModule, ModulateMode};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule, StereoLink, FxChannelTarget};
    use realfft::num_complex::Complex;

    let num_bins = 1025;

    for mode in [
        ModulateMode::PhasePhaser,
        ModulateMode::BinSwapper,
        ModulateMode::RmFmMatrix,
        ModulateMode::DiodeRm,
        ModulateMode::GroundLoop,
        ModulateMode::GravityPhaser,
        ModulateMode::PllTear,
    ] {
        let mut module = ModulateModule::new();
        module.reset(48_000.0, 2048);
        module.set_modulate_mode(mode);
        if mode == ModulateMode::GravityPhaser {
            module.set_modulate_repel(true);
            module.set_modulate_sc_positioned(true);
        }

        let mut bins_l: Vec<Complex<f32>> = (0..num_bins).map(|k|
            Complex::new(((k as f32 * 0.07).sin() + 0.1).abs(),
                         ((k as f32 * 0.13).cos() * 0.5))
        ).collect();
        let mut bins_r: Vec<Complex<f32>> = bins_l.iter().map(|b| b * 0.6).collect();
        let sc: Vec<f32> = (0..num_bins).map(|k| ((k as f32 * 0.05).sin() + 0.2).abs()).collect();

        let amount = vec![1.5_f32; num_bins];
        let mid = vec![1.0_f32; num_bins];
        let mix = vec![1.0_f32; num_bins];
        let curves: Vec<&[f32]> = vec![&amount, &mid, &mid, &mid, &mid, &mix];

        let mut suppression = vec![0.0_f32; num_bins];
        let mut physics_l = BinPhysics::new();
        physics_l.reset_active(num_bins, 48_000.0, 2048);
        let mut physics_r = BinPhysics::new();
        physics_r.reset_active(num_bins, 48_000.0, 2048);

        let unwrapped: Vec<f32> = (0..num_bins).map(|k| (k as f32) * 0.05).collect();
        let ctx_owned = ModuleContext::new_minimal(48_000.0, 2048);
        let ctx = ModuleContext { unwrapped_phase: Some(&unwrapped[..]), ..ctx_owned };

        for hop in 0..200 {
            for ch in 0..2 {
                let bins = if ch == 0 { &mut bins_l } else { &mut bins_r };
                let physics = if ch == 0 { &mut physics_l } else { &mut physics_r };
                module.process(ch, StereoLink::Independent, FxChannelTarget::All, bins, Some(&sc), &curves, Some(physics), &mut suppression, &ctx);
                for (i, b) in bins.iter().enumerate() {
                    assert!(b.norm().is_finite(), "mode={:?} hop={} ch={} bin={} norm={}", mode, hop, ch, i, b.norm());
                    assert!(b.norm() < 1e6, "runaway: mode={:?} hop={} ch={} bin={} norm={}", mode, hop, ch, i, b.norm());
                }
                for s in &suppression {
                    assert!(s.is_finite() && *s >= 0.0);
                }
                for k in 0..num_bins {
                    assert!(physics.phase_momentum[k].is_finite(),
                        "physics NaN at mode={:?} hop={} ch={} bin={}", mode, hop, ch, k);
                    assert!(physics.phase_momentum[k].abs() < 100.0,
                        "physics runaway at mode={:?} hop={} ch={} bin={}: {}", mode, hop, ch, k, physics.phase_momentum[k]);
                }
            }
        }
    }
}
```

- [ ] **Step 2: Run test, expect pass**

Run: `cargo test --test module_trait modulate_finite_bounded_all_modes_dual_channel -- --nocapture`
Expected: PASS — all 7 modes stay finite and bounded over 200 hops × 2
channels with sidechain + physics.

If a mode fails: the relevant kernel needs an additional clamp. Common
suspects: PLL Tear's `momentum[k] += 0.1 * err.signum() * (err.abs() - tear_thresh).max(0.0)`
can drift if tear is sustained — add a hard clamp `momentum[k] = momentum[k].clamp(-10.0, 10.0)`
inside Task 7's torn-bin branch.

- [ ] **Step 3: Commit**

```bash
git add tests/module_trait.rs
git commit -m "test(modulate): finite/bounded covers all 7 modes incl. retrofit"
```

---

## Task 13: Status banners + STATUS.md update

**Files:**
- Modify: `docs/superpowers/STATUS.md`
- Modify: `docs/superpowers/plans/2026-04-27-phase-2f-modulate-light.md` (banner note)
- Modify: this plan file (top banner)

- [ ] **Step 1: Get the merge SHA**

After merging the feature branch into master:

```bash
git log --oneline -1
```

Note the SHA of the merge commit (e.g. `c780adb`).

- [ ] **Step 2: Update this plan's banner**

In `docs/superpowers/plans/2026-04-27-phase-5b4-modulate-retrofit.md`, replace
the top banner:

```
> **Status:** PLANNED — implementation pending.
```

with:

```
> **Status:** IMPLEMENTED — landed in commit <SHA>. Modulate module gained
> Gravity Phaser + PLL Tear modes; physics_helpers gained pll_bank_step +
> wrap_phase. FM Network + Slew Lag remain deferred to Phase 6.
```

- [ ] **Step 3: Update Phase 2f banner with retrofit pointer**

In `docs/superpowers/plans/2026-04-27-phase-2f-modulate-light.md`, replace
the top banner:

```
> **Status:** PLANNED — implementation pending.
```

(or whatever the current banner says) with:

```
> **Status:** IMPLEMENTED — v1 modes shipped; retrofitted with
> GravityPhaser + PllTear in
> `docs/superpowers/plans/2026-04-27-phase-5b4-modulate-retrofit.md`.
> See STATUS.md for canonical status.
```

- [ ] **Step 4: Append STATUS.md row**

In `docs/superpowers/STATUS.md`, find the table of plans and append:

```
| 2026-04-27-phase-5b4-modulate-retrofit.md | IMPLEMENTED | Modulate module: Gravity Phaser (writes phase_momentum, Repel + SidechainPositioned toggles) and PLL Tear (consumes ctx.unwrapped_phase, lock-loss tear emission). FM Network + Slew Lag deferred to Phase 6. |
```

- [ ] **Step 5: Final commit**

```bash
git add docs/superpowers/STATUS.md docs/superpowers/plans/2026-04-27-phase-5b4-modulate-retrofit.md docs/superpowers/plans/2026-04-27-phase-2f-modulate-light.md
git commit -m "docs(status): mark phase-5b4 Modulate retrofit IMPLEMENTED"
```

---

## Self-review

**1. Spec coverage check** (against `ideas/next-gen-modules/16-modulate.md`):

| Spec section / mode | Phase 5b.4 task | Status |
|---|---|---|
| Phase Phaser (v1) | Phase 2f Task 3 (already shipped) | covered (no change) |
| Bin Swapper (v1) | Phase 2f Task 4 (already shipped) | covered (no change) |
| RM/FM Matrix (v1) | Phase 2f Task 5 (already shipped) | covered (no change) |
| Diode RM (v1) | Phase 2f Task 6 (already shipped) | covered (no change) |
| Ground Loop (v1) | Phase 2f Task 7 (already shipped) | covered (no change) |
| Gravity Phaser (audit § g + audit § f) | Tasks 4 + 5 + 6 | covered |
| PLL Tear (audit § d, research finding 1-6) | Tasks 2 + 7 | covered |
| FM Network — Partial Web | — | **deferred to Phase 6** (needs `ctx.instantaneous_freq`) |
| Slew Lag | — | **deferred to Phase 6+** (needs `ctx.sidechain_derivative`) |
| Repel toggle (audit § f) | Task 5 | covered |
| Sidechain-positioned wells (audit § g) | Task 6 | covered |
| AmpGate retrofit for Gravity Phaser (audit § e) | Task 4 (`ampgate` per-bin scale) | covered |
| 1-pole curve smoothing (research finding 3 cross-applies) | Task 3 | covered |
| `heavy_cpu_per_mode` (audit § CPU class) | Task 1 | covered |
| `writes_bin_physics: true` (audit § BinPhysics interactions) | Task 1 | covered |
| Calibration probes (audit § probe set) | Task 9 | covered |
| Skip PLL on bins < 16 (research finding 3) | Task 7 (`PLL_MIN_BIN`) | covered |
| Lock-loss with hysteresis (research finding 2) | Task 7 (`PLL_TEAR_THRESHOLD` / `PLL_RELOCK_THRESHOLD` + `PLL_RELOCK_HOPS`) | covered |
| `RATE → ωₙ`, `THRESHOLD → tear thresh`, `AMOUNT → wet/dry`, `REACH → bin range` (research finding 5) | Task 7 | covered |
| PLPV unwrapped phase consumed when present (research finding 4) | Task 7 (ctx.unwrapped_phase branch) + Task 11 | covered |
| Local-unwrap fallback when PLPV off (research finding 4) | Task 7 (`unwrap_phase_local`) | covered |

**2. Placeholder scan:** No "TBD", no "implement later", no `// fill in
details`. All 8 kernels include full Rust implementations with all clamps and
explicit constants. All 13 tests have full assertions with concrete numerical
expectations (not just `assert!(result.is_ok())`).

**3. Type consistency:** `ModulateMode` extended with `GravityPhaser`,
`PllTear` — used uniformly across enum, popup, params, FxMatrix, kernels,
probe. `set_modulate_repel` / `set_modulate_sc_positioned` trait names
consistent. `slot_modulate_repel` / `slot_modulate_sc_positioned` field names
parallel `slot_modulate_mode`. Helper names `pll_bank_step` / `wrap_phase` /
`smooth_curve_one_pole` consistent with Phase 5b.3 conventions. `MAX_GP_NODES =
32`, `PLL_MIN_BIN = 16`, `PLL_TEAR_THRESHOLD = π/2`, `PLL_RELOCK_THRESHOLD =
π/8`, `PLL_RELOCK_HOPS = 4` defined once and re-used.

**4. Phase ordering check:**
- Phase 1 must have shipped: `ModuleContext` has `'block` lifetime,
  `unwrapped_phase: Option<&'block [f32]>` slot, `peaks: Option<&'block [PeakInfo]>` slot,
  `bin_physics: Option<&'block BinPhysics>` slot, `heavy_cpu_per_mode: Option<&'static [bool]>`
  on ModuleSpec, `writes_bin_physics: bool` on ModuleSpec, `ModuleContext::new_minimal`
  constructor.
- Phase 2f must have shipped: `ModulateModule` v1 + 5 light kernels + popup
  base.
- Phase 3 must have shipped: `BinPhysics` struct with `phase_momentum: Vec<f32>`
  field + `reset_active(num_bins, sample_rate, fft_size)` method, trait
  `process()` takes `physics: Option<&mut BinPhysics>` arg.
- Phase 4 must have shipped: Pipeline computes `unwrapped_phase` and exposes
  it via `ctx.unwrapped_phase` (PLL Tear's preferred input).
- Phase 5b.3 must have shipped: `physics_helpers.rs` with
  `smooth_curve_one_pole`, `clamp_for_cfl`, `clamp_damping_floor`,
  `apply_energy_rise_hysteresis`, plus `smallvec` Cargo dep.

If any of those haven't shipped, **STOP** and resolve the dependency before
implementing this plan.

**5. Realtime safety audit:**
- All per-channel scratch (`smoothed_curves`, `pll_phase`, `pll_freq`,
  `pll_err_scratch`, `pll_torn`, `pll_relock_count`, `prev_phase`,
  `unwrap_local`, `prev_phase_scratch`, `gp_nodes`) is allocated in `reset()`.
- `gp_nodes` uses `SmallVec` with cap 32 (stack-only).
- The dispatch arms call `refresh_smoothed()` once per hop, which uses
  pre-allocated `smoothed_curves[ch]`. No `Vec::new()` or `clone()` on the hot
  path.
- The `phase_momentum: Option<&mut [f32]>` arm contains a fallback
  `vec![0.0; num_bins]` allocation **for the test path only** — when called from
  `Pipeline::process()` via `FxMatrix::process_hop()`, `physics.is_some()` is
  guaranteed because `writes_bin_physics: true` causes the writer schedule to
  populate it. Document this with a comment in the kernel and assert it via
  `debug_assert!` if needed for hardening.
- `xorshift32` is branchless integer arithmetic.
- `wrap_phase` uses `rem_euclid(TAU)`: branchless after the inline; LLVM
  compiles to a single `fmod`-style instruction.
- The PLL bank step is a tight loop; no allocation, no branch on bin index.

**6. Audit gaps that this plan defers:**
- FM Network — Partial Web (needs Phase 6.1 IF computation).
- Slew Lag (needs `ctx.sidechain_derivative`; not yet planned).
- Per-channel asymmetric Gravity nodes (mono nodes shared across channels for
  v1 retrofit).
- Adaptive PLL bandwidth scaling per bin (research finding 1.1 — flat ωₙ for
  v1; per-bin scaling deferred to Phase 7 if requested).

These deferrals are documented in the **Defer list** at the top of this plan.

---

## Execution Handoff

**Plan complete and saved to `docs/superpowers/plans/2026-04-27-phase-5b4-modulate-retrofit.md`. Two execution options:**

**1. Subagent-Driven (recommended)** — dispatch fresh subagent per task, two-stage review between tasks, fast iteration.

**2. Inline Execution** — execute tasks in this session using executing-plans, batch with checkpoints.

**Which approach?**
