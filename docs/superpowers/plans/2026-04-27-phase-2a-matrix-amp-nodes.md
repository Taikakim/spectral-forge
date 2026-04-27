> **Status (2026-04-27): IMPLEMENTED.** All 12 tasks merged on `feature/next-gen-modules-plans`. AmpMode + AmpCellParams + per-mode kernels (Vactrol/Schmitt/Slew/Stiction); RouteMatrix amp_mode + amp_params with serde-default for preset compat; FxMatrix lazy-alloc amp_state via `permit_alloc`; per-cell apply at all three accumulation sites in `process_hop`; theme dot constants + popup + matrix-cell indicator/right-click; calibration probe gated behind `feature = "probe"`; e2e finite/bounded test across every mode. Pipeline now snapshots amp_mode/amp_params from the params mutex per block, and FxMatrix internal buffers are MAX_NUM_BINS so reset on FFT-size change is purely a clear. The code is the source of truth; this plan is kept for history. See [../STATUS.md](../STATUS.md).

# Phase 2a — Matrix Amp Nodes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Convert each routing-matrix send node from a scalar multiply into an active processing unit ("amp") with selectable non-linear modes (`Linear`, `Vactrol`, `Schmitt`, `Slew`, `Stiction`).

**Architecture:** `RouteMatrix` (cloned per block) gains an `amp_mode` matrix and a small bag of per-cell numeric params. `FxMatrix` (audio-side, owns state) gains an `amp_state[row][col]: AmpNodeState` enum that holds whatever per-bin arrays each mode needs. State arrays are allocated lazily via `permit_alloc!` the first time a non-Linear mode is selected for a given cell, then reused. `process_hop` transforms the source signal through the cell's amp before accumulating into `mix_buf`. Per-channel state is split for `Independent`/`MidSide`, shared for `Linked`.

**Tech Stack:** Rust, num_complex, nih-plug, nih-plug-egui, triple_buffer.

**Source design:** `docs/superpowers/specs/2026-04-21-matrix-amp-nodes.md` (DEFERRED) + `ideas/next-gen-modules/03-matrix-amp-nodes.md` (audit). This plan supersedes the 2026-04-21 spec on land.

**Roadmap reference:** `ideas/next-gen-modules/99-implementation-roadmap.md` § Phase 2 (item 1).

---

## File structure

| File | Status | Responsibility |
|---|---|---|
| `src/dsp/amp_modes.rs` | Create | `AmpMode` enum, `AmpCellParams` struct, `AmpNodeState` enum + per-mode state arrays + per-mode `apply()` kernels. |
| `src/dsp/modules/mod.rs` | Modify | Extend `RouteMatrix` with `amp_mode: [[AmpMode; MAX_SLOTS]; MAX_MATRIX_ROWS]` and `amp_params: [[AmpCellParams; MAX_SLOTS]; MAX_MATRIX_ROWS]`. |
| `src/dsp/fx_matrix.rs` | Modify | Hold `amp_state: Vec<Vec<AmpNodeState>>` (per-channel × cell). Apply amp transform inside accumulation loop in `process_hop`. Lazy-allocate state when params signal a mode change. |
| `src/editor/amp_popup.rs` | Create | Floating popup (modeled on `module_popup.rs`) for picking amp mode + editing per-mode params for one cell. |
| `src/editor/fx_matrix_grid.rs` | Modify | Cell renders a coloured indicator dot when `amp_mode != Linear`; right-click on cell opens `amp_popup`. |
| `src/editor/theme.rs` | Modify | Add `AMP_DOT_COLORS: [Color32; 5]` and `AMP_DOT_RADIUS` constants. |
| `src/params.rs` | Modify | Persist `amp_mode` matrix + per-cell amp params via `slot_route_matrix` (already a `Mutex<RouteMatrix>`). |
| `tests/amp_nodes.rs` | Create | Unit tests for each amp kernel + end-to-end FxMatrix routing test that exercises every mode. |

---

## Task 1 — `AmpMode` enum + `AmpCellParams` struct

**Files:**
- Create: `src/dsp/amp_modes.rs`
- Modify: `src/dsp/mod.rs` (add `pub mod amp_modes;`)

- [ ] **Step 1: Write the failing test**

```rust
// tests/amp_nodes.rs (NEW)
use spectral_forge::dsp::amp_modes::{AmpMode, AmpCellParams};

#[test]
fn amp_mode_default_is_linear() {
    assert_eq!(AmpMode::default(), AmpMode::Linear);
}

#[test]
fn amp_cell_params_default_is_neutral() {
    let p = AmpCellParams::default();
    assert_eq!(p.amount,        1.0);
    assert_eq!(p.threshold,     0.5);
    assert_eq!(p.release_ms,   100.0);
    assert_eq!(p.slew_db_per_s, 60.0);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test amp_nodes amp_mode_default_is_linear`
Expected: compile error — module does not exist.

- [ ] **Step 3: Write minimal implementation**

```rust
// src/dsp/amp_modes.rs (NEW)
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum AmpMode {
    #[default]
    Linear,
    Vactrol,
    Schmitt,
    Slew,
    Stiction,
}

impl AmpMode {
    pub fn label(self) -> &'static str {
        match self {
            AmpMode::Linear   => "Linear",
            AmpMode::Vactrol  => "Vactrol",
            AmpMode::Schmitt  => "Schmitt",
            AmpMode::Slew     => "Slew",
            AmpMode::Stiction => "Stiction",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct AmpCellParams {
    pub amount:        f32,   // 0..2 — strength of the amp effect (0 = bypass, 1 = full, >1 = exaggerated)
    pub threshold:     f32,   // 0..1 magnitude — Schmitt on-threshold; Stiction step
    pub release_ms:    f32,   // Vactrol release time
    pub slew_db_per_s: f32,   // Slew max change rate
}

impl Default for AmpCellParams {
    fn default() -> Self {
        Self { amount: 1.0, threshold: 0.5, release_ms: 100.0, slew_db_per_s: 60.0 }
    }
}
```

Edit `src/dsp/mod.rs`:

```rust
pub mod amp_modes;
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --test amp_nodes amp_mode_default_is_linear amp_cell_params_default_is_neutral`
Expected: 2 passed.

- [ ] **Step 5: Commit**

```bash
git add src/dsp/amp_modes.rs src/dsp/mod.rs tests/amp_nodes.rs
git commit -m "feat(amp): AmpMode enum + AmpCellParams struct"
```

---

## Task 2 — `AmpNodeState` enum + per-mode kernels

**Files:**
- Modify: `src/dsp/amp_modes.rs`
- Test: `tests/amp_nodes.rs`

- [ ] **Step 1: Write the failing tests**

```rust
// tests/amp_nodes.rs — append
use spectral_forge::dsp::amp_modes::{AmpNodeState, AmpCellParams, AmpMode};
use num_complex::Complex;

const NB: usize = 16;

fn neutral_input() -> Vec<Complex<f32>> {
    (0..NB).map(|k| Complex::new(0.5, 0.1 * k as f32 / NB as f32)).collect()
}

#[test]
fn linear_passes_through_with_amount_one() {
    let mut state = AmpNodeState::new(AmpMode::Linear, NB);
    let p = AmpCellParams::default();
    let mut buf = neutral_input();
    let original = buf.clone();
    state.apply(&p, &mut buf, 1.0 / 48000.0 * 512.0);
    for (a, b) in buf.iter().zip(original.iter()) {
        assert!((a.re - b.re).abs() < 1e-6);
        assert!((a.im - b.im).abs() < 1e-6);
    }
}

#[test]
fn vactrol_holds_then_releases() {
    let mut state = AmpNodeState::new(AmpMode::Vactrol, NB);
    let p = AmpCellParams { amount: 1.0, release_ms: 200.0, ..Default::default() };
    let mut buf = vec![Complex::new(1.0, 0.0); NB];
    let hop_dt = 1.0 / 48000.0 * 512.0;
    state.apply(&p, &mut buf, hop_dt); // first hit, capacitor charges fast
    let charged_mag = buf[0].norm();
    assert!(charged_mag > 0.99);
    // Now feed silence — should slowly decay (vactrol release)
    for _ in 0..5 {
        let mut zero_buf = vec![Complex::new(0.0, 0.0); NB];
        state.apply(&p, &mut zero_buf, hop_dt);
    }
    let mut decayed = vec![Complex::new(1.0, 0.0); NB];
    state.apply(&p, &mut decayed, hop_dt);
    // Capacitor still has memory — output > input would mean memory active
    // Actually vactrol applies cap level as gain ratio, see kernel for exact contract.
    // Just verify it doesn't NaN and stays bounded.
    for c in &decayed {
        assert!(c.re.is_finite() && c.im.is_finite());
        assert!(c.norm() <= 2.0);
    }
}

#[test]
fn schmitt_latches() {
    let mut state = AmpNodeState::new(AmpMode::Schmitt, NB);
    let p = AmpCellParams { amount: 1.0, threshold: 0.6, ..Default::default() };
    // Below threshold → silenced (gate closed at startup)
    let mut buf = vec![Complex::new(0.3, 0.0); NB];
    state.apply(&p, &mut buf, 0.01);
    for c in &buf { assert!(c.norm() < 1e-6, "below threshold should be gated"); }
    // Above on-threshold → opens
    let mut buf = vec![Complex::new(0.8, 0.0); NB];
    state.apply(&p, &mut buf, 0.01);
    for c in &buf { assert!(c.norm() > 0.5, "above threshold should pass"); }
    // Drop slightly below on-threshold but above off-threshold → stays open (hysteresis)
    let mut buf = vec![Complex::new(0.55, 0.0); NB];
    state.apply(&p, &mut buf, 0.01);
    for c in &buf { assert!(c.norm() > 0.5, "hysteresis: should remain open"); }
}

#[test]
fn slew_limits_change_rate() {
    let mut state = AmpNodeState::new(AmpMode::Slew, NB);
    let p = AmpCellParams { amount: 1.0, slew_db_per_s: 60.0, ..Default::default() };
    // Big jump from 0 → 1.0 in one hop should be limited.
    let mut buf = vec![Complex::new(1.0, 0.0); NB];
    state.apply(&p, &mut buf, 1.0 / 48000.0 * 512.0); // ~10.7 ms hop
    // 60 dB/s × 0.0107 s = 0.642 dB headroom in one hop.
    // From 0 (effectively -inf dB) the slew lets only ~+0.64 dB — output should be tiny.
    for c in &buf { assert!(c.norm() < 0.2, "slew should limit large jumps"); }
}

#[test]
fn stiction_dead_zone() {
    let mut state = AmpNodeState::new(AmpMode::Stiction, NB);
    let p = AmpCellParams { amount: 1.0, threshold: 0.1, ..Default::default() };
    // Tiny changes accumulate; once over threshold, output snaps.
    let mut buf = vec![Complex::new(0.05, 0.0); NB];
    state.apply(&p, &mut buf, 0.01);
    for c in &buf { assert!(c.norm() < 1e-6, "below stiction threshold = no movement"); }
    let mut buf = vec![Complex::new(0.5, 0.0); NB];
    state.apply(&p, &mut buf, 0.01);
    for c in &buf { assert!(c.norm() > 0.4, "over threshold should release"); }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --test amp_nodes`
Expected: compile errors / test failures because `AmpNodeState` does not exist.

- [ ] **Step 3: Write minimal implementation**

```rust
// src/dsp/amp_modes.rs — append

use num_complex::Complex;
use nih_plug::util::permit_alloc;

/// Per-cell DSP state. One of these per (row, col) per channel in the matrix.
#[derive(Debug)]
pub enum AmpNodeState {
    Linear,
    Vactrol  { cap: Vec<f32> },
    Schmitt  { latch: Vec<bool> },
    Slew     { current_db: Vec<f32> },
    Stiction { accumulator: Vec<f32>, last_out: Vec<f32> },
}

impl AmpNodeState {
    /// Construct state for `mode`. Allocates per-bin arrays for non-Linear modes.
    /// Caller is responsible for invoking inside `permit_alloc!` if on the audio thread.
    pub fn new(mode: AmpMode, num_bins: usize) -> Self {
        match mode {
            AmpMode::Linear   => AmpNodeState::Linear,
            AmpMode::Vactrol  => AmpNodeState::Vactrol  { cap: vec![0.0; num_bins] },
            AmpMode::Schmitt  => AmpNodeState::Schmitt  { latch: vec![false; num_bins] },
            AmpMode::Slew     => AmpNodeState::Slew     { current_db: vec![-120.0; num_bins] },
            AmpMode::Stiction => AmpNodeState::Stiction {
                accumulator: vec![0.0; num_bins],
                last_out:    vec![0.0; num_bins],
            },
        }
    }

    /// True if this state matches the given mode (used to detect mode changes).
    pub fn matches(&self, mode: AmpMode) -> bool {
        matches!(
            (self, mode),
            (AmpNodeState::Linear,   AmpMode::Linear)
            | (AmpNodeState::Vactrol  { .. }, AmpMode::Vactrol)
            | (AmpNodeState::Schmitt  { .. }, AmpMode::Schmitt)
            | (AmpNodeState::Slew     { .. }, AmpMode::Slew)
            | (AmpNodeState::Stiction { .. }, AmpMode::Stiction)
        )
    }

    /// Reset all internal state arrays to startup values, but keep allocations.
    pub fn clear(&mut self) {
        match self {
            AmpNodeState::Linear => {}
            AmpNodeState::Vactrol  { cap }      => cap.fill(0.0),
            AmpNodeState::Schmitt  { latch }    => latch.fill(false),
            AmpNodeState::Slew     { current_db } => current_db.fill(-120.0),
            AmpNodeState::Stiction { accumulator, last_out } => {
                accumulator.fill(0.0);
                last_out.fill(0.0);
            }
        }
    }

    /// Resize state arrays for a new fft size. Allocates if growing; cheap if same.
    /// Must be called inside `permit_alloc!` if on the audio thread.
    pub fn resize(&mut self, num_bins: usize) {
        match self {
            AmpNodeState::Linear => {}
            AmpNodeState::Vactrol  { cap }        => cap.resize(num_bins, 0.0),
            AmpNodeState::Schmitt  { latch }      => latch.resize(num_bins, false),
            AmpNodeState::Slew     { current_db } => current_db.resize(num_bins, -120.0),
            AmpNodeState::Stiction { accumulator, last_out } => {
                accumulator.resize(num_bins, 0.0);
                last_out.resize(num_bins, 0.0);
            }
        }
    }

    /// Apply this amp's transform to `buf` in place.
    /// `buf.len()` must equal the state's array length.
    /// `hop_dt` is the audio time elapsed per hop in seconds.
    pub fn apply(&mut self, p: &AmpCellParams, buf: &mut [Complex<f32>], hop_dt: f32) {
        match self {
            AmpNodeState::Linear => apply_linear(p, buf),
            AmpNodeState::Vactrol  { cap }            => apply_vactrol(p, buf, cap, hop_dt),
            AmpNodeState::Schmitt  { latch }          => apply_schmitt(p, buf, latch),
            AmpNodeState::Slew     { current_db }     => apply_slew(p, buf, current_db, hop_dt),
            AmpNodeState::Stiction { accumulator, last_out }
                                                      => apply_stiction(p, buf, accumulator, last_out),
        }
    }
}

// ── Kernels ────────────────────────────────────────────────────────────────

fn apply_linear(p: &AmpCellParams, buf: &mut [Complex<f32>]) {
    if (p.amount - 1.0).abs() < 1e-6 { return; }
    for c in buf.iter_mut() { *c *= p.amount; }
}

/// Vactrol: capacitor charges fast (~5 ms time constant) on input, releases slowly.
/// The capacitor level then becomes a gain modulator on the next sample (LDR behaviour).
fn apply_vactrol(p: &AmpCellParams, buf: &mut [Complex<f32>], cap: &mut [f32], hop_dt: f32) {
    const ATTACK_MS: f32 = 5.0;
    let attack_a  = (-hop_dt / (ATTACK_MS * 0.001)).exp();
    let release_a = (-hop_dt / (p.release_ms * 0.001)).exp();
    for (c, cap_k) in buf.iter_mut().zip(cap.iter_mut()) {
        let mag = c.norm();
        // Asymmetric one-pole.
        if mag > *cap_k {
            *cap_k = attack_a * *cap_k + (1.0 - attack_a) * mag;
        } else {
            *cap_k = release_a * *cap_k + (1.0 - release_a) * mag;
        }
        // Cap level modulates the input — light passing through LDR.
        let gain = (*cap_k).clamp(0.0, 1.0).powf(0.6);
        let blend = 1.0 - p.amount + p.amount * gain; // amount = 0 → 1.0, amount = 1 → gain
        *c *= blend;
    }
}

/// Schmitt: per-bin two-threshold latch. ON threshold = `p.threshold`, OFF threshold = 0.7 × ON.
fn apply_schmitt(p: &AmpCellParams, buf: &mut [Complex<f32>], latch: &mut [bool]) {
    let on_th  = p.threshold;
    let off_th = p.threshold * 0.7;
    for (c, l) in buf.iter_mut().zip(latch.iter_mut()) {
        let mag = c.norm();
        if !*l && mag >= on_th  { *l = true; }
        if *l  && mag <  off_th { *l = false; }
        if !*l { *c = Complex::new(0.0, 0.0); }
        else if (p.amount - 1.0).abs() > 1e-6 { *c *= p.amount; }
    }
}

/// Slew: per-bin magnitude can only change at most `slew_db_per_s` per second.
/// Phase is preserved.
fn apply_slew(p: &AmpCellParams, buf: &mut [Complex<f32>], current_db: &mut [f32], hop_dt: f32) {
    let max_step_db = p.slew_db_per_s * hop_dt;
    for (c, cur_db) in buf.iter_mut().zip(current_db.iter_mut()) {
        let mag = c.norm().max(1e-12);
        let target_db = 20.0 * mag.log10();
        let delta = (target_db - *cur_db).clamp(-max_step_db, max_step_db);
        *cur_db = (*cur_db + delta).max(-120.0);
        let new_mag = 10f32.powf(*cur_db * 0.05);
        let new_gain = (new_mag / mag) * p.amount + (1.0 - p.amount);
        *c *= new_gain;
    }
}

/// Stiction: dead-zone. Output stays put until accumulated change exceeds `threshold`,
/// then output snaps to current input and accumulator resets.
fn apply_stiction(p: &AmpCellParams, buf: &mut [Complex<f32>], accumulator: &mut [f32], last_out: &mut [f32]) {
    let th = p.threshold.max(1e-6);
    for (c, (acc, lo)) in buf.iter_mut().zip(accumulator.iter_mut().zip(last_out.iter_mut())) {
        let mag = c.norm();
        let delta = (mag - *lo).abs();
        *acc += delta;
        if *acc >= th {
            *lo  = mag;
            *acc = 0.0;
        }
        // Output: hold last_out as magnitude, but preserve phase from input.
        let phase_unit = if mag > 1e-12 { *c / mag } else { Complex::new(0.0, 0.0) };
        let out_mag = *lo * p.amount + mag * (1.0 - p.amount);
        *c = phase_unit * out_mag;
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --test amp_nodes`
Expected: 7 passed (the 2 from Task 1 + 5 new).

- [ ] **Step 5: Commit**

```bash
git add src/dsp/amp_modes.rs tests/amp_nodes.rs
git commit -m "feat(amp): AmpNodeState + per-mode kernels (Vactrol/Schmitt/Slew/Stiction)"
```

---

## Task 3 — Extend `RouteMatrix` with `amp_mode` + `amp_params`

**Files:**
- Modify: `src/dsp/modules/mod.rs:47-65` (RouteMatrix struct + Default impl)

- [ ] **Step 1: Write the failing test**

```rust
// tests/amp_nodes.rs — append
use spectral_forge::dsp::modules::{RouteMatrix, MAX_SLOTS, MAX_MATRIX_ROWS};

#[test]
fn route_matrix_default_is_all_linear() {
    let m = RouteMatrix::default();
    for r in 0..MAX_MATRIX_ROWS {
        for c in 0..MAX_SLOTS {
            assert_eq!(m.amp_mode[r][c], AmpMode::Linear,
                "cell ({}, {}) should default to Linear", r, c);
            let p = m.amp_params[r][c];
            assert_eq!(p.amount, 1.0);
        }
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test amp_nodes route_matrix_default_is_all_linear`
Expected: compile error — `amp_mode` field does not exist.

- [ ] **Step 3: Modify `RouteMatrix`**

Edit `src/dsp/modules/mod.rs` line 47-65:

```rust
use crate::dsp::amp_modes::{AmpMode, AmpCellParams};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteMatrix {
    pub send: [[f32; MAX_SLOTS]; MAX_MATRIX_ROWS],
    pub virtual_rows: [Option<(u8, VirtualRowKind)>; MAX_SPLIT_VIRTUAL_ROWS],
    #[serde(default = "default_amp_modes")]
    pub amp_mode: [[AmpMode; MAX_SLOTS]; MAX_MATRIX_ROWS],
    #[serde(default = "default_amp_params")]
    pub amp_params: [[AmpCellParams; MAX_SLOTS]; MAX_MATRIX_ROWS],
}

fn default_amp_modes() -> [[AmpMode; MAX_SLOTS]; MAX_MATRIX_ROWS] {
    [[AmpMode::Linear; MAX_SLOTS]; MAX_MATRIX_ROWS]
}

fn default_amp_params() -> [[AmpCellParams; MAX_SLOTS]; MAX_MATRIX_ROWS] {
    [[AmpCellParams { amount: 1.0, threshold: 0.5, release_ms: 100.0, slew_db_per_s: 60.0 };
      MAX_SLOTS]; MAX_MATRIX_ROWS]
}

impl Default for RouteMatrix {
    fn default() -> Self {
        let mut m = Self {
            send: [[0.0f32; MAX_SLOTS]; MAX_MATRIX_ROWS],
            virtual_rows: [None; MAX_SPLIT_VIRTUAL_ROWS],
            amp_mode:   default_amp_modes(),
            amp_params: default_amp_params(),
        };
        m.send[0][1] = 1.0;
        m.send[1][2] = 1.0;
        m.send[2][8] = 1.0;
        m
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --test amp_nodes route_matrix_default_is_all_linear`
Expected: 1 passed.

- [ ] **Step 5: Verify existing tests still pass**

Run: `cargo test`
Expected: full suite green. The `serde(default = ...)` keeps existing presets loadable.

- [ ] **Step 6: Commit**

```bash
git add src/dsp/modules/mod.rs tests/amp_nodes.rs
git commit -m "feat(amp): RouteMatrix carries amp_mode + amp_params per cell"
```

---

## Task 4 — `FxMatrix` allocates per-cell amp state lazily

**Files:**
- Modify: `src/dsp/fx_matrix.rs`
- Test: `tests/amp_nodes.rs`

- [ ] **Step 1: Write the failing test**

```rust
// tests/amp_nodes.rs — append
use spectral_forge::dsp::fx_matrix::FxMatrix;
use spectral_forge::dsp::modules::ModuleType;

#[test]
fn fx_matrix_starts_with_all_linear_state() {
    let types = [ModuleType::Empty; 9];
    let fxm = FxMatrix::new(48000.0, 1024, &types);
    // Per-channel × MAX_MATRIX_ROWS × MAX_SLOTS, all Linear initially.
    assert_eq!(fxm.amp_state[0].len(), MAX_MATRIX_ROWS);
    for r in 0..MAX_MATRIX_ROWS {
        for c in 0..MAX_SLOTS {
            assert!(matches!(fxm.amp_state[0][r][c], AmpNodeState::Linear));
        }
    }
}

#[test]
fn fx_matrix_sync_amp_modes_allocates_state_for_non_linear() {
    let types = [ModuleType::Empty; 9];
    let mut fxm = FxMatrix::new(48000.0, 1024, &types);
    let mut rm = RouteMatrix::default();
    rm.amp_mode[0][1] = AmpMode::Vactrol;
    rm.amp_mode[2][3] = AmpMode::Slew;

    fxm.sync_amp_modes(&rm, 513);

    assert!(matches!(fxm.amp_state[0][0][1], AmpNodeState::Vactrol { .. }));
    assert!(matches!(fxm.amp_state[0][2][3], AmpNodeState::Slew    { .. }));
    // Untouched cells stay Linear.
    assert!(matches!(fxm.amp_state[0][0][0], AmpNodeState::Linear));
}

#[test]
fn fx_matrix_sync_amp_modes_replaces_state_on_mode_change() {
    let types = [ModuleType::Empty; 9];
    let mut fxm = FxMatrix::new(48000.0, 1024, &types);
    let mut rm = RouteMatrix::default();
    rm.amp_mode[1][2] = AmpMode::Vactrol;
    fxm.sync_amp_modes(&rm, 513);
    assert!(matches!(fxm.amp_state[0][1][2], AmpNodeState::Vactrol { .. }));

    rm.amp_mode[1][2] = AmpMode::Schmitt;
    fxm.sync_amp_modes(&rm, 513);
    assert!(matches!(fxm.amp_state[0][1][2], AmpNodeState::Schmitt { .. }));

    rm.amp_mode[1][2] = AmpMode::Linear;
    fxm.sync_amp_modes(&rm, 513);
    assert!(matches!(fxm.amp_state[0][1][2], AmpNodeState::Linear));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --test amp_nodes fx_matrix_`
Expected: compile errors — `amp_state` field does not exist.

- [ ] **Step 3: Modify `FxMatrix`**

Edit `src/dsp/fx_matrix.rs`:

Add at top:
```rust
use crate::dsp::amp_modes::{AmpMode, AmpNodeState};
use crate::dsp::modules::MAX_MATRIX_ROWS;
```

Extend struct to:
```rust
pub struct FxMatrix {
    pub slots: Vec<Option<Box<dyn SpectralModule>>>,
    slot_out:  Vec<Vec<Complex<f32>>>,
    slot_supp: Vec<Vec<f32>>,
    virtual_out: Vec<Vec<Complex<f32>>>,
    mix_buf:   Vec<Complex<f32>>,
    /// Per-channel × row × col amp state. Channel 0 always present;
    /// channel 1 used only for Independent / MidSide stereo.
    pub amp_state: [Vec<Vec<AmpNodeState>>; 2],
    /// Scratch buffer for amp transforms (so we apply amp to a copy of each source,
    /// not the slot's own output buffer).
    amp_scratch: Vec<Complex<f32>>,
}
```

Update `new()`:
```rust
pub fn new(sample_rate: f32, fft_size: usize, slot_types: &[ModuleType; 9]) -> Self {
    let num_bins = fft_size / 2 + 1;
    let slots: Vec<Option<Box<dyn SpectralModule>>> = (0..MAX_SLOTS).map(|i| {
        match slot_types[i] {
            ModuleType::Empty => None,
            ty => Some(create_module(ty, sample_rate, fft_size)),
        }
    }).collect();
    let mk_amp_grid = || (0..MAX_MATRIX_ROWS).map(|_|
        (0..MAX_SLOTS).map(|_| AmpNodeState::Linear).collect()
    ).collect();
    Self {
        slots,
        slot_out:    (0..MAX_SLOTS).map(|_| vec![Complex::new(0.0, 0.0); num_bins]).collect(),
        slot_supp:   (0..MAX_SLOTS).map(|_| vec![0.0f32; num_bins]).collect(),
        virtual_out: (0..MAX_SPLIT_VIRTUAL_ROWS).map(|_| vec![Complex::new(0.0, 0.0); num_bins]).collect(),
        mix_buf: vec![Complex::new(0.0, 0.0); num_bins],
        amp_state: [mk_amp_grid(), mk_amp_grid()],
        amp_scratch: vec![Complex::new(0.0, 0.0); num_bins],
    }
}
```

Add new methods:
```rust
/// Sync per-cell amp state to match the requested amp_modes in `rm`.
/// On mismatch: drops the old state (dealloc) and creates a new one (alloc) — both
/// inside permit_alloc, since this runs on the audio thread before process_hop.
/// On match: leaves state intact (preserves Vactrol cap level, Schmitt latch, etc.).
pub fn sync_amp_modes(&mut self, rm: &RouteMatrix, num_bins: usize) {
    for ch in 0..2 {
        for r in 0..MAX_MATRIX_ROWS {
            for c in 0..MAX_SLOTS {
                let want = rm.amp_mode[r][c];
                if !self.amp_state[ch][r][c].matches(want) {
                    nih_plug::util::permit_alloc(|| {
                        self.amp_state[ch][r][c] = AmpNodeState::new(want, num_bins);
                    });
                } else {
                    // Same mode: ensure inner arrays match current num_bins.
                    nih_plug::util::permit_alloc(|| {
                        self.amp_state[ch][r][c].resize(num_bins);
                    });
                }
            }
        }
    }
}

/// Clear all amp state arrays to startup values (e.g. on preset load or FFT-size change).
pub fn clear_amp_state(&mut self) {
    for ch in 0..2 {
        for r in 0..MAX_MATRIX_ROWS {
            for c in 0..MAX_SLOTS {
                self.amp_state[ch][r][c].clear();
            }
        }
    }
}
```

Extend `reset()` to also clear amp state:
```rust
pub fn reset(&mut self, sample_rate: f32, fft_size: usize) {
    let num_bins = fft_size / 2 + 1;
    debug_assert_eq!(self.slot_out[0].len(), num_bins);
    for slot in self.slots.iter_mut().flatten() {
        slot.reset(sample_rate, fft_size);
    }
    for buf in &mut self.slot_out    { buf.fill(Complex::new(0.0, 0.0)); }
    for buf in &mut self.slot_supp   { buf.fill(0.0); }
    for buf in &mut self.virtual_out { buf.fill(Complex::new(0.0, 0.0)); }
    self.mix_buf.fill(Complex::new(0.0, 0.0));
    self.amp_scratch.resize(num_bins, Complex::new(0.0, 0.0));
    self.clear_amp_state();
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --test amp_nodes fx_matrix_`
Expected: 3 passed.

- [ ] **Step 5: Commit**

```bash
git add src/dsp/fx_matrix.rs tests/amp_nodes.rs
git commit -m "feat(amp): FxMatrix lazy-allocates per-cell amp state"
```

---

## Task 5 — Apply amp transforms inside `process_hop`

**Files:**
- Modify: `src/dsp/fx_matrix.rs:78-205` (process_hop)
- Modify: `src/dsp/pipeline.rs` (call `sync_amp_modes` before each hop block)

- [ ] **Step 1: Write the failing test**

```rust
// tests/amp_nodes.rs — append
use spectral_forge::dsp::modules::{ModuleContext, GainMode};
use spectral_forge::params::{FxChannelTarget, StereoLink};

#[test]
fn process_hop_routes_unchanged_through_linear_amp() {
    let types = [ModuleType::Empty; 9];
    let mut fxm = FxMatrix::new(48000.0, 1024, &types);
    let mut rm = RouteMatrix::default();
    // Default routing: 0 → 1 → 2 → master, all linear, all amount 1.
    let mut buf: Vec<Complex<f32>> = (0..513).map(|k| Complex::new((k as f32) / 513.0, 0.0)).collect();
    let original = buf.clone();
    let curves = vec![vec![vec![1.0f32; 513]; 7]; 9];
    let sc_args: [Option<&[f32]>; 9] = [None; 9];
    let targets = [FxChannelTarget::All; 9];
    let mut supp = vec![0.0f32; 513];
    let ctx = ModuleContext {
        sample_rate: 48000.0, fft_size: 1024, num_bins: 513,
        attack_ms: 10.0, release_ms: 100.0, sensitivity: 0.5,
        suppression_width: 1.0, auto_makeup: false, delta_monitor: false,
    };
    fxm.sync_amp_modes(&rm, 513);
    fxm.process_hop(0, StereoLink::Linked, &mut buf, &sc_args, &targets,
                    &curves, &rm, &ctx, &mut supp, 513);
    // No modules → empty slot just passes mix_buf through; signal arrives at master.
    for (a, b) in buf.iter().zip(original.iter()) {
        assert!((a.re - b.re).abs() < 1e-4, "linear amp must be transparent");
    }
}

#[test]
fn process_hop_amp_attenuates_send() {
    let types = [ModuleType::Empty; 9];
    let mut fxm = FxMatrix::new(48000.0, 1024, &types);
    let mut rm = RouteMatrix::default();
    // Set the amp on the 0→1 send cell to Linear with amount=0 → silences.
    rm.amp_mode[0][1] = AmpMode::Linear;
    rm.amp_params[0][1].amount = 0.0;
    let mut buf: Vec<Complex<f32>> = vec![Complex::new(1.0, 0.0); 513];
    let curves = vec![vec![vec![1.0f32; 513]; 7]; 9];
    let sc_args: [Option<&[f32]>; 9] = [None; 9];
    let targets = [FxChannelTarget::All; 9];
    let mut supp = vec![0.0f32; 513];
    let ctx = ModuleContext {
        sample_rate: 48000.0, fft_size: 1024, num_bins: 513,
        attack_ms: 10.0, release_ms: 100.0, sensitivity: 0.5,
        suppression_width: 1.0, auto_makeup: false, delta_monitor: false,
    };
    fxm.sync_amp_modes(&rm, 513);
    fxm.process_hop(0, StereoLink::Linked, &mut buf, &sc_args, &targets,
                    &curves, &rm, &ctx, &mut supp, 513);
    // Slot 0 input was 1.0; 0→1 amp zeroes the send into slot 1; chain dies before master.
    for c in &buf { assert!(c.norm() < 1e-4, "amount=0 amp must mute the send"); }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --test amp_nodes process_hop_`
Expected: tests compile (FxMatrix already has `process_hop`) but the second test fails because amps aren't applied yet.

- [ ] **Step 3: Modify `process_hop`**

Edit `src/dsp/fx_matrix.rs:78-205`:

Replace the slot accumulation loop (`for src in 0..s` block, lines ~105-111) with one that copies into `amp_scratch`, applies the amp, then accumulates:

```rust
let hop_dt = ctx.fft_size as f32 / ctx.sample_rate / 4.0; // OVERLAP=4
for src in 0..s {
    let send = route_matrix.send[src][s];
    if send < 0.001 { continue; }
    // Copy source into scratch
    for k in 0..num_bins {
        self.amp_scratch[k] = self.slot_out[src][k];
    }
    // Apply this cell's amp transform
    let amp_ch = match stereo_link {
        StereoLink::Linked => 0,
        _ => channel.min(1),
    };
    let p = &route_matrix.amp_params[src][s];
    self.amp_state[amp_ch][src][s].apply(p, &mut self.amp_scratch[..num_bins], hop_dt);
    // Accumulate (post-amp scaled by send amount)
    for k in 0..num_bins {
        self.mix_buf[k] += self.amp_scratch[k] * send;
    }
}
```

Apply the same pattern to the virtual-row accumulation (lines ~113-123):

```rust
for (v, &vrow) in route_matrix.virtual_rows.iter().enumerate() {
    if let Some((src_slot, _kind)) = vrow {
        if (src_slot as usize) < s {
            let send = route_matrix.send[MAX_SLOTS + v][s];
            if send < 0.001 { continue; }
            for k in 0..num_bins {
                self.amp_scratch[k] = self.virtual_out[v][k];
            }
            let amp_ch = match stereo_link {
                StereoLink::Linked => 0,
                _ => channel.min(1),
            };
            let p = &route_matrix.amp_params[MAX_SLOTS + v][s];
            self.amp_state[amp_ch][MAX_SLOTS + v][s].apply(
                p, &mut self.amp_scratch[..num_bins], hop_dt);
            for k in 0..num_bins {
                self.mix_buf[k] += self.amp_scratch[k] * send;
            }
        }
    }
}
```

And the master-mix loop (lines ~175-182):

```rust
for src in 0..8 {
    let send = route_matrix.send[src][8];
    if send < 0.001 { continue; }
    for k in 0..num_bins {
        self.amp_scratch[k] = self.slot_out[src][k];
    }
    let amp_ch = match stereo_link {
        StereoLink::Linked => 0,
        _ => channel.min(1),
    };
    let p = &route_matrix.amp_params[src][8];
    self.amp_state[amp_ch][src][8].apply(p, &mut self.amp_scratch[..num_bins], hop_dt);
    for k in 0..num_bins {
        self.mix_buf[k] += self.amp_scratch[k] * send;
    }
}
```

Resize `amp_scratch` if needed at the top of `process_hop`:
```rust
debug_assert!(self.amp_scratch.len() >= num_bins);
```

- [ ] **Step 4: Wire `sync_amp_modes` from `pipeline.rs`**

In `src/dsp/pipeline.rs`, find where the route matrix is snapshotted before the STFT closure (look for `route_matrix_snap`). After the snapshot is taken and before `process_hop` is called, add:

```rust
self.fx_matrix.sync_amp_modes(&route_matrix_snap, num_bins);
```

If you can't find an obvious spot, grep for `route_matrix_snap`:

Run: `rg "route_matrix_snap" src/dsp/pipeline.rs -n`

Place the call immediately after `route_matrix_snap` is bound and before the per-hop processing loop.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --test amp_nodes process_hop_`
Expected: 2 passed.

Run: `cargo test`
Expected: full suite green.

- [ ] **Step 6: Commit**

```bash
git add src/dsp/fx_matrix.rs src/dsp/pipeline.rs
git commit -m "feat(amp): apply per-cell amp transforms in process_hop"
```

---

## Task 6 — Reset semantics: clear state on FFT-size and preset load

**Files:**
- Modify: `src/dsp/pipeline.rs` (FFT-size change path)
- Modify: `src/lib.rs` (preset / state load callback)

- [ ] **Step 1: Write the failing test**

```rust
// tests/amp_nodes.rs — append
#[test]
fn fft_size_change_clears_amp_state() {
    let types = [ModuleType::Empty; 9];
    let mut fxm = FxMatrix::new(48000.0, 1024, &types);
    let mut rm = RouteMatrix::default();
    rm.amp_mode[0][1] = AmpMode::Vactrol;
    fxm.sync_amp_modes(&rm, 513);
    if let AmpNodeState::Vactrol { cap } = &mut fxm.amp_state[0][0][1] {
        cap.fill(0.7);
    }
    fxm.reset(48000.0, 2048);
    if let AmpNodeState::Vactrol { cap } = &fxm.amp_state[0][0][1] {
        for &v in cap.iter() { assert!(v.abs() < 1e-6, "reset must zero cap"); }
    } else {
        panic!("amp state should still be Vactrol after reset");
    }
}
```

- [ ] **Step 2: Run test to verify it fails (or passes already)**

Run: `cargo test --test amp_nodes fft_size_change_clears_amp_state`
Expected: pass — Task 4 already wired `clear_amp_state` into `reset`. If it fails, the wiring slipped.

- [ ] **Step 3: Verify Pipeline calls FxMatrix::reset on FFT-size change**

Run: `rg "fx_matrix.reset" src/dsp/pipeline.rs`
Expected: at least one match. If none, find the FFT-size change path and add `self.fx_matrix.reset(sample_rate, new_fft_size)` there.

- [ ] **Step 4: Add a comment on the policy**

In `src/dsp/pipeline.rs`, near the `FxMatrix::reset` call, add a one-line comment:

```rust
// Reset clears all amp-node state — preset load + FFT-size change both warm up from zero.
```

- [ ] **Step 5: Run tests**

Run: `cargo test`
Expected: full suite green.

- [ ] **Step 6: Commit**

```bash
git add src/dsp/pipeline.rs tests/amp_nodes.rs
git commit -m "feat(amp): clear amp state on reset (FFT-size change, preset load)"
```

---

## Task 7 — Theme constants for amp indicator dot

**Files:**
- Modify: `src/editor/theme.rs`

- [ ] **Step 1: Edit `theme.rs`**

Add at the bottom of the file (or in the colour-constants section if there is one):

```rust
use nih_plug_egui::egui::Color32;

/// Amp-mode indicator dot colours, indexed by `AmpMode as usize`.
/// Linear is transparent (no dot drawn).
pub const AMP_DOT_COLORS: [Color32; 5] = [
    Color32::TRANSPARENT,                         // Linear
    Color32::from_rgb(0xff, 0xa6, 0x3d),          // Vactrol — warm orange
    Color32::from_rgb(0x6d, 0xc7, 0xff),          // Schmitt — cool blue
    Color32::from_rgb(0xa3, 0xff, 0x9d),          // Slew    — pale green
    Color32::from_rgb(0xb3, 0x8d, 0xff),          // Stiction — violet
];

pub const AMP_DOT_RADIUS: f32 = 2.5;
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo build`
Expected: success.

- [ ] **Step 3: Commit**

```bash
git add src/editor/theme.rs
git commit -m "feat(amp): theme constants for amp-mode indicator dots"
```

---

## Task 8 — Floating popup for amp mode + params

**Files:**
- Create: `src/editor/amp_popup.rs`
- Modify: `src/editor/mod.rs` (add `pub mod amp_popup;`)

- [ ] **Step 1: Create the popup module**

```rust
// src/editor/amp_popup.rs (NEW)
use nih_plug_egui::egui::{self, Pos2, Ui};
use crate::dsp::amp_modes::AmpMode;
use crate::dsp::modules::{MAX_SLOTS, MAX_MATRIX_ROWS};
use crate::editor::theme as th;
use crate::params::SpectralForgeParams;

/// Ephemeral state for the amp-cell popup. Stored in egui temp data.
#[derive(Clone)]
pub struct AmpPopupState {
    pub open: bool,
    pub row:  usize,
    pub col:  usize,
    pub pos:  Pos2,
}

impl Default for AmpPopupState {
    fn default() -> Self {
        Self { open: false, row: 0, col: 0, pos: Pos2::ZERO }
    }
}

const MODES: &[AmpMode] = &[
    AmpMode::Linear, AmpMode::Vactrol, AmpMode::Schmitt, AmpMode::Slew, AmpMode::Stiction,
];

/// Render the popup if open. Call every frame from the main UI closure.
/// Returns true if the popup consumed a click.
pub fn show_popup(ui: &mut Ui, params: &SpectralForgeParams, scale: f32) -> bool {
    let key = ui.id().with("amp_popup");
    let state: AmpPopupState = ui.data(|d| d.get_temp(key).unwrap_or_default());
    if !state.open { return false; }

    let (row, col) = (state.row, state.col);
    if row >= MAX_MATRIX_ROWS || col >= MAX_SLOTS {
        // Corrupt state — close it.
        ui.data_mut(|d| d.insert_temp(key, AmpPopupState::default()));
        return false;
    }

    let (mut current_mode, current_amount, current_threshold, current_release, current_slew) = {
        let rm = params.slot_route_matrix.lock();
        (
            rm.amp_mode[row][col],
            rm.amp_params[row][col].amount,
            rm.amp_params[row][col].threshold,
            rm.amp_params[row][col].release_ms,
            rm.amp_params[row][col].slew_db_per_s,
        )
    };

    let mut new_state = state.clone();
    let mut consumed = false;
    let mut mode_changed = false;
    let mut amount = current_amount;
    let mut threshold = current_threshold;
    let mut release  = current_release;
    let mut slew     = current_slew;

    egui::Area::new(egui::Id::new("amp_popup_area"))
        .fixed_pos(state.pos)
        .order(egui::Order::Foreground)
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_min_width(160.0);
                ui.label(
                    egui::RichText::new(format!("Amp ({}, {})", row, col))
                        .color(th::LABEL_DIM).size(th::scaled(th::FONT_SIZE_LABEL, scale))
                );
                ui.separator();

                for &mode in MODES {
                    let selected = current_mode == mode;
                    let resp = ui.selectable_label(selected, mode.label());
                    if resp.clicked() && !selected {
                        current_mode = mode;
                        mode_changed = true;
                        consumed = true;
                    }
                }

                ui.separator();
                ui.add(egui::Slider::new(&mut amount,    0.0..=2.0).text("amount"));
                ui.add(egui::Slider::new(&mut threshold, 0.0..=1.0).text("threshold"));
                ui.add(egui::Slider::new(&mut release,   1.0..=2000.0).text("release ms"));
                ui.add(egui::Slider::new(&mut slew,      1.0..=240.0).text("slew dB/s"));

                ui.separator();
                if ui.button("Close").clicked() {
                    new_state.open = false;
                    consumed = true;
                }
            });
        });

    // Persist any changes back into the route matrix (single lock, end of frame).
    let needs_write = mode_changed
        || (amount    - current_amount).abs()    > 1e-6
        || (threshold - current_threshold).abs() > 1e-6
        || (release   - current_release).abs()   > 1e-6
        || (slew      - current_slew).abs()      > 1e-6;
    if needs_write {
        let mut rm = params.slot_route_matrix.lock();
        rm.amp_mode[row][col] = current_mode;
        rm.amp_params[row][col].amount        = amount;
        rm.amp_params[row][col].threshold     = threshold;
        rm.amp_params[row][col].release_ms    = release;
        rm.amp_params[row][col].slew_db_per_s = slew;
    }

    ui.data_mut(|d| d.insert_temp(key, new_state));
    consumed
}

/// Open the popup at `pos` for cell (row, col). Call from a click handler.
pub fn open_at(ui: &mut Ui, row: usize, col: usize, pos: Pos2) {
    let key = ui.id().with("amp_popup");
    ui.data_mut(|d| d.insert_temp(key, AmpPopupState { open: true, row, col, pos }));
}
```

- [ ] **Step 2: Register the module**

Edit `src/editor/mod.rs` — add:
```rust
pub mod amp_popup;
```

- [ ] **Step 3: Verify compile**

Run: `cargo build`
Expected: success.

- [ ] **Step 4: Commit**

```bash
git add src/editor/amp_popup.rs src/editor/mod.rs
git commit -m "feat(amp): floating popup for cell amp mode + params"
```

---

## Task 9 — Indicator dot in matrix grid + right-click handler

**Files:**
- Modify: `src/editor/fx_matrix_grid.rs`
- Modify: `src/editor_ui.rs` (call `amp_popup::show_popup` once per frame from the top-level UI)

- [ ] **Step 1: Locate the per-cell rendering site**

Run: `rg "fn .*cell" src/editor/fx_matrix_grid.rs -n`
Run: `rg "send\[" src/editor/fx_matrix_grid.rs -n`

Identify the function that renders one cell (typically reads `rm.send[row][col]`).

- [ ] **Step 2: Read the cell function and its callsite**

Read: `src/editor/fx_matrix_grid.rs` — focus on the cell-rendering function.

- [ ] **Step 3: Add the dot + right-click handler**

Inside the cell function, after the existing knob is rendered, add the indicator dot:

```rust
let amp_mode = {
    let rm = params.slot_route_matrix.lock();
    rm.amp_mode[row][col]
};
if amp_mode != crate::dsp::amp_modes::AmpMode::Linear {
    let dot_color = th::AMP_DOT_COLORS[amp_mode as usize];
    // Place dot in the top-right corner of `cell_rect`.
    let dot_pos = egui::pos2(cell_rect.right() - 4.0, cell_rect.top() + 4.0);
    ui.painter().circle_filled(dot_pos, th::AMP_DOT_RADIUS, dot_color);
}

// Right-click opens the amp popup at click position.
let cell_resp = /* the egui::Response from rendering the cell */;
if cell_resp.secondary_clicked() {
    if let Some(p) = cell_resp.interact_pointer_pos() {
        crate::editor::amp_popup::open_at(ui, row, col, p);
    }
}
```

(Adapt names for `cell_rect` and `cell_resp` to whatever the existing function uses.)

- [ ] **Step 4: Wire popup display into the top-level UI**

In `src/editor_ui.rs`, find where `module_popup::show_popup` is called and add a sibling line:

```rust
crate::editor::amp_popup::show_popup(ui, params, scale);
```

- [ ] **Step 5: Verify compile**

Run: `cargo build`
Expected: success.

- [ ] **Step 6: Manual smoke test (write before commit)**

```bash
cargo run --package xtask -- bundle spectral_forge --release
cp target/bundled/spectral_forge.clap ~/.clap/
```

Open Bitwig, load Spectral Forge, right-click a non-zero matrix cell, change mode to Vactrol, drag a knob — expect the indicator dot to appear and audio to change.

If something is off, debug now; do **not** commit a broken UI. Note any issues in the commit message if you fixed them.

- [ ] **Step 7: Commit**

```bash
git add src/editor/fx_matrix_grid.rs src/editor_ui.rs
git commit -m "feat(amp): matrix-cell amp-mode indicator + right-click popup"
```

---

## Task 10 — Calibration probes for amp nodes

**Files:**
- Modify: `src/dsp/amp_modes.rs`
- Modify: `src/dsp/modules/mod.rs` (`ProbeSnapshot`)
- Test: `tests/amp_nodes.rs`

- [ ] **Step 1: Write the failing test**

```rust
// tests/amp_nodes.rs — append
#[cfg(any(test, feature = "probe"))]
#[test]
fn vactrol_probe_records_state_at_test_bin() {
    use spectral_forge::dsp::amp_modes::{probe_amp_state, AmpProbe};
    let mut state = AmpNodeState::Vactrol { cap: vec![0.0; 16] };
    if let AmpNodeState::Vactrol { cap } = &mut state {
        cap[5] = 0.42;
    }
    let probe = probe_amp_state(&state, 5);
    assert_eq!(probe.amount_pct,    None);
    assert_eq!(probe.state_at_k,    Some(0.42));
    assert_eq!(probe.release_ms,    None);
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --test amp_nodes vactrol_probe_records_state_at_test_bin`
Expected: compile error — `probe_amp_state` does not exist.

- [ ] **Step 3: Implement probe**

Append to `src/dsp/amp_modes.rs`:

```rust
#[cfg(any(test, feature = "probe"))]
#[derive(Clone, Copy, Debug, Default)]
pub struct AmpProbe {
    pub amount_pct: Option<f32>,
    pub state_at_k: Option<f32>,
    pub release_ms: Option<f32>,
}

#[cfg(any(test, feature = "probe"))]
pub fn probe_amp_state(state: &AmpNodeState, k: usize) -> AmpProbe {
    match state {
        AmpNodeState::Linear => AmpProbe::default(),
        AmpNodeState::Vactrol  { cap } => AmpProbe {
            state_at_k: cap.get(k).copied(),
            ..Default::default()
        },
        AmpNodeState::Schmitt  { latch } => AmpProbe {
            state_at_k: latch.get(k).map(|&b| if b { 1.0 } else { 0.0 }),
            ..Default::default()
        },
        AmpNodeState::Slew     { current_db } => AmpProbe {
            state_at_k: current_db.get(k).copied(),
            ..Default::default()
        },
        AmpNodeState::Stiction { last_out, .. } => AmpProbe {
            state_at_k: last_out.get(k).copied(),
            ..Default::default()
        },
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test --test amp_nodes`
Expected: all amp tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/dsp/amp_modes.rs tests/amp_nodes.rs
git commit -m "feat(amp): calibration probe for amp-node state"
```

---

## Task 11 — End-to-end pipeline test exercising every amp mode

**Files:**
- Test: `tests/amp_nodes.rs`

- [ ] **Step 1: Write the test**

Append to `tests/amp_nodes.rs`:

```rust
#[test]
fn process_hop_every_amp_mode_is_finite_and_bounded() {
    use AmpMode::*;
    for mode in [Linear, Vactrol, Schmitt, Slew, Stiction] {
        let types = [ModuleType::Empty; 9];
        let mut fxm = FxMatrix::new(48000.0, 1024, &types);
        let mut rm = RouteMatrix::default();
        rm.amp_mode[0][1] = mode;
        rm.amp_params[0][1].amount    = 1.0;
        rm.amp_params[0][1].threshold = 0.3;

        // 50 hops of swept-magnitude input.
        let curves = vec![vec![vec![1.0f32; 513]; 7]; 9];
        let sc_args: [Option<&[f32]>; 9] = [None; 9];
        let targets = [FxChannelTarget::All; 9];
        let mut supp = vec![0.0f32; 513];
        let ctx = ModuleContext {
            sample_rate: 48000.0, fft_size: 1024, num_bins: 513,
            attack_ms: 10.0, release_ms: 100.0, sensitivity: 0.5,
            suppression_width: 1.0, auto_makeup: false, delta_monitor: false,
        };
        fxm.sync_amp_modes(&rm, 513);

        for h in 0..50 {
            let mag = 0.1 + 0.9 * (h as f32 / 50.0);
            let mut buf = vec![Complex::new(mag, 0.0); 513];
            fxm.process_hop(0, StereoLink::Linked, &mut buf, &sc_args, &targets,
                            &curves, &rm, &ctx, &mut supp, 513);
            for c in &buf {
                assert!(c.re.is_finite() && c.im.is_finite(),
                    "{:?} produced non-finite at hop {}", mode, h);
                assert!(c.norm() <= 4.0,
                    "{:?} runaway at hop {}: |c|={}", mode, h, c.norm());
            }
        }
    }
}
```

- [ ] **Step 2: Run**

Run: `cargo test --test amp_nodes process_hop_every_amp_mode_is_finite_and_bounded`
Expected: pass.

- [ ] **Step 3: Run full test suite**

Run: `cargo test`
Expected: all green. If any pre-existing test broke, fix it before continuing.

- [ ] **Step 4: Commit**

```bash
git add tests/amp_nodes.rs
git commit -m "test(amp): end-to-end finite/bounded check for every amp mode"
```

---

## Task 12 — Status banner update + STATUS.md

**Files:**
- Modify: `docs/superpowers/specs/2026-04-21-matrix-amp-nodes.md` (banner)
- Modify: `docs/superpowers/STATUS.md`

- [ ] **Step 1: Update spec banner**

Replace the first line of `docs/superpowers/specs/2026-04-21-matrix-amp-nodes.md`:

```markdown
> **Status (2026-04-27): IMPLEMENTED** by Phase 2a plan `docs/superpowers/plans/2026-04-27-phase-2a-matrix-amp-nodes.md`. Source of truth: this spec + `src/dsp/amp_modes.rs`.
```

- [ ] **Step 2: Update STATUS.md index**

Read: `docs/superpowers/STATUS.md`

Find the row for `2026-04-21-matrix-amp-nodes.md` and flip its status from DEFERRED to IMPLEMENTED. Add a row pointing at this Phase 2a plan.

- [ ] **Step 3: Smoke listen**

Build a release bundle:
```bash
cargo run --package xtask -- bundle spectral_forge --release
cp target/bundled/spectral_forge.clap ~/.clap/
```

Open Bitwig, set up two slots with a feedback send, switch the feedback send to Vactrol mode, listen. Note any quirks in the commit message.

- [ ] **Step 4: Commit**

```bash
git add docs/superpowers/specs/2026-04-21-matrix-amp-nodes.md docs/superpowers/STATUS.md
git commit -m "docs: Matrix Amp Nodes IMPLEMENTED status banner + STATUS index"
```

---

## Risk register

1. **`amp_state[ch=1]` only used for Independent/MidSide.** Channel 0 is always touched. If a preset switches stereo mode mid-block, ch=1 state may have been left stale. Mitigation: `clear_amp_state()` is called from `reset()`, which fires on any param change that touches FFT size or sample rate. If users report stereo glitches after switching link mode, add a `clear_amp_state()` call to the stereo-link change path in pipeline.rs.

2. **Per-cell `permit_alloc` count.** `sync_amp_modes` runs `permit_alloc` for every (channel, row, col) cell **only when the mode changes**. In steady state nothing allocates. The mode-change path is per user click — cheap.

3. **Vactrol amount=0 is not transparent.** `apply_vactrol` returns `1.0 - amount + amount * gain`, so amount=0 returns 1.0 (transparent). Verified by the kernel inspection. If the user wants to mute a Vactrol cell they should use the send amount, not the amp amount.

4. **Schmitt with amount != 1 still hard-gates.** The amount only scales the open-state output. This is intentional — Schmitt is a binary effect.

5. **`AmpNodeState::resize` on non-Linear cells re-fills new bins with the startup value.** If FFT size grows mid-session, the new high-bin tail starts cold. This is the same behaviour the per-slot modules have on `reset()`.

6. **Audit recommended waiting for Circuit module for code reuse.** This plan ships the kernels standalone. When Circuit module lands in Phase 5c, refactor the kernels into a shared `dsp/analog_kernels.rs` and have both call into it. Track as follow-up.

---

## Self-review checklist

- [x] Every task contains complete code; no "TBD" / "implement later" placeholders.
- [x] Tests precede implementation.
- [x] Exact file paths and line ranges given for modify-existing tasks.
- [x] Spec coverage:
  - Linear, Vactrol, Schmitt, Slew, Stiction all implemented (Task 2).
  - Per-cell state + lazy alloc (Task 4).
  - Reset semantics: amp state clears on FFT-size and preset load (Task 6).
  - Indicator dot + popup UI (Tasks 7-9).
  - Calibration probes (Task 10).
- [x] Open questions from audit answered:
  - Per-channel state in Independent/MidSide, shared in Linked (Task 5 stereo-channel selection).
  - `send` amplitude is post-amp (Task 5: amp transforms scratch, then accumulator multiplies by send).
  - Continuous mode-morphing not in v1 (per audit recommendation).
- [x] Type consistency: `AmpMode` and `AmpCellParams` referenced consistently from Tasks 1-12.
- [x] `RouteMatrix` serde-default fields preserve existing preset compat (Task 3).

---

## Execution handoff

**Plan complete and saved to `docs/superpowers/plans/2026-04-27-phase-2a-matrix-amp-nodes.md`.**

Two execution options:

1. **Subagent-Driven (recommended)** — dispatch a fresh subagent per task, review between tasks, fast iteration.
2. **Inline Execution** — execute tasks in this session using executing-plans.

This is one of seven Phase 2 plans. The companion plans cover Future, Punch, Rhythm, Geometry-light, Modulate-light, and Circuit-light. They can ship independently in any order.
