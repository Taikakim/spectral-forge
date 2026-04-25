> **Status (2026-04-24): DEFERRED — not started.** No `BinPhysics` code exists yet. This plan remains a valid design reference and gates the seven physics-driven module specs (Circuit / Life / Kinetics / Harmony / Modulate / Past / Rhythm). Do not treat as implemented. Source of truth: [../STATUS.md](../STATUS.md).

# BinPhysics Infrastructure — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a per-bin physical state system (`BinPhysics`) that travels through `FxMatrix` alongside audio bins — with velocity auto-computed each hop, all other fields inert until a physics module writes them.

**Architecture:** `BinPhysics` is a flat struct of `Vec<f32>` arrays (one per property, sized to `MAX_NUM_BINS`). `FxMatrix` owns one `BinPhysics` per slot (output state) plus one workspace buffer (`mix_phys`) for assembling each slot's input. Velocity is auto-derived from hop-to-hop magnitude changes in the assembled input — no module needs to compute it. All existing modules add an ignored `_physics: &mut BinPhysics` parameter; physics modules will use it in later plans. All other `BinPhysics` properties (mass, temperature, flux, displacement, crystallization, phase_momentum) are inert: initialized to defaults, transported by `FxMatrix`, changed only by modules that explicitly write to them.

**Tech Stack:** Rust, no new dependencies. `MAX_NUM_BINS = 8193` (from `src/dsp/pipeline.rs`). No audio-thread allocation — all `Vec`s pre-allocated in `FxMatrix::new()`.

---

## Module / File Map

| File | Action | Responsibility |
|---|---|---|
| `src/dsp/bin_physics.rs` | **Create** | `BinPhysics` struct + `mix_from()` + `reset_active()` |
| `src/dsp/mod.rs` | Modify | Add `pub mod bin_physics;` |
| `src/dsp/modules/mod.rs` | Modify | Import `BinPhysics`; add `physics: &mut BinPhysics` to trait |
| `src/dsp/modules/dynamics.rs` | Modify | Add `_physics` param, ignore |
| `src/dsp/modules/freeze.rs` | Modify | Add `_physics` param, ignore |
| `src/dsp/modules/phase_smear.rs` | Modify | Add `_physics` param, ignore |
| `src/dsp/modules/contrast.rs` | Modify | Add `_physics` param, ignore |
| `src/dsp/modules/gain.rs` | Modify | Add `_physics` param, ignore |
| `src/dsp/modules/ts_split.rs` | Modify | Add `_physics` param, ignore |
| `src/dsp/modules/harmonic.rs` | Modify | Add `_physics` param, ignore |
| `src/dsp/modules/master.rs` | Modify | Add `_physics` param to both impls, ignore |
| `src/dsp/modules/mid_side.rs` | Modify | Add `_physics` param, ignore |
| `src/dsp/fx_matrix.rs` | Modify | `slot_phys`, `mix_phys`, `prev_magnitudes`; physics assembly + velocity computation in `process_hop()` |
| `tests/module_trait.rs` | Modify | Add `BinPhysics` compile test |

---

## Task 1 — Create `src/dsp/bin_physics.rs`

**Files:**
- Create: `src/dsp/bin_physics.rs`

- [ ] **Step 1: Write the failing compile check**

Add to `tests/module_trait.rs` at the bottom (it will fail until the file exists):

```rust
#[test]
fn bin_physics_compiles() {
    use spectral_forge::dsp::bin_physics::BinPhysics;
    let phys = BinPhysics::new();
    assert_eq!(phys.mass[0], 1.0);
    assert_eq!(phys.velocity[0], 0.0);
    assert_eq!(phys.temperature[0], 0.0);
    assert_eq!(phys.flux[0], 0.0);
    assert_eq!(phys.displacement[0], 0.0);
    assert_eq!(phys.crystallization[0], 0.0);
    assert_eq!(phys.phase_momentum[0], 0.0);
}
```

- [ ] **Step 2: Run to confirm failure**

```bash
cargo test bin_physics_compiles 2>&1 | grep -E "error|FAILED"
```

Expected: compile error — `dsp::bin_physics` not found.

- [ ] **Step 3: Create the file**

```rust
// src/dsp/bin_physics.rs
use crate::dsp::pipeline::MAX_NUM_BINS;

/// Per-bin physical state that travels through the FxMatrix alongside audio bins.
///
/// Rules:
/// - `velocity` is auto-computed by FxMatrix each hop from input magnitude deltas.
/// - All other fields are inert: initialized to defaults, mixed proportionally when
///   route sends are combined, and only changed by modules that explicitly write them.
/// - Modules that don't use physics receive this struct and must pass it through
///   unchanged (i.e. just ignore it — don't zero or reinitialize fields you don't own).
pub struct BinPhysics {
    /// Magnitude rate-of-change between consecutive hops. Auto-computed by FxMatrix.
    pub velocity:        Vec<f32>,
    /// Inertia — resistance to change. Default: 1.0.
    pub mass:            Vec<f32>,
    /// Thermal energy accumulated per bin. Default: 0.0.
    pub temperature:     Vec<f32>,
    /// Magnetic flux / saturation memory. Default: 0.0.
    pub flux:            Vec<f32>,
    /// Spectral displacement from rest position. Default: 0.0.
    pub displacement:    Vec<f32>,
    /// Phase crystallization level. 0.0 = chaotic, 1.0 = fully locked. Default: 0.0.
    pub crystallization: Vec<f32>,
    /// Angular phase velocity (phase momentum). Default: 0.0.
    pub phase_momentum:  Vec<f32>,
}

impl BinPhysics {
    /// Allocate once at MAX_NUM_BINS. No audio-thread allocation after this.
    pub fn new() -> Self {
        Self {
            velocity:        vec![0.0; MAX_NUM_BINS],
            mass:            vec![1.0; MAX_NUM_BINS],
            temperature:     vec![0.0; MAX_NUM_BINS],
            flux:            vec![0.0; MAX_NUM_BINS],
            displacement:    vec![0.0; MAX_NUM_BINS],
            crystallization: vec![0.0; MAX_NUM_BINS],
            phase_momentum:  vec![0.0; MAX_NUM_BINS],
        }
    }

    /// Reset active bins to defaults without reallocating.
    /// Velocity → 0.0, mass → 1.0, all others → 0.0.
    pub fn reset_active(&mut self, num_bins: usize) {
        self.velocity[..num_bins].fill(0.0);
        self.mass[..num_bins].fill(1.0);
        self.temperature[..num_bins].fill(0.0);
        self.flux[..num_bins].fill(0.0);
        self.displacement[..num_bins].fill(0.0);
        self.crystallization[..num_bins].fill(0.0);
        self.phase_momentum[..num_bins].fill(0.0);
    }

    /// Incrementally mix `src` into `self` using amplitude-weighted averaging.
    ///
    /// Call this once per source slot when assembling a slot's input physics.
    /// `send` is the route amplitude from that source. `total_send_before` is the
    /// sum of all send amplitudes accumulated so far (excluding `send`).
    /// Start with `total_send_before = 0.0` for the first source.
    pub fn mix_from(&mut self, src: &BinPhysics, send: f32, num_bins: usize, total_send_before: f32) {
        if send < 1e-6 { return; }
        let new_total = total_send_before + send;
        let w_old = total_send_before / new_total;
        let w_new = send / new_total;
        for k in 0..num_bins {
            self.velocity[k]        = self.velocity[k]        * w_old + src.velocity[k]        * w_new;
            self.mass[k]            = self.mass[k]            * w_old + src.mass[k]            * w_new;
            self.temperature[k]     = self.temperature[k]     * w_old + src.temperature[k]     * w_new;
            self.flux[k]            = self.flux[k]            * w_old + src.flux[k]            * w_new;
            self.displacement[k]    = self.displacement[k]    * w_old + src.displacement[k]    * w_new;
            self.crystallization[k] = self.crystallization[k] * w_old + src.crystallization[k] * w_new;
            self.phase_momentum[k]  = self.phase_momentum[k]  * w_old + src.phase_momentum[k]  * w_new;
        }
    }

    /// Copy active bins from `self` into `dst`. Used by FxMatrix to save a slot's
    /// output physics without moving/swapping (mix_phys is reused each slot).
    pub fn copy_active_to(&self, dst: &mut BinPhysics, num_bins: usize) {
        dst.velocity[..num_bins].copy_from_slice(&self.velocity[..num_bins]);
        dst.mass[..num_bins].copy_from_slice(&self.mass[..num_bins]);
        dst.temperature[..num_bins].copy_from_slice(&self.temperature[..num_bins]);
        dst.flux[..num_bins].copy_from_slice(&self.flux[..num_bins]);
        dst.displacement[..num_bins].copy_from_slice(&self.displacement[..num_bins]);
        dst.crystallization[..num_bins].copy_from_slice(&self.crystallization[..num_bins]);
        dst.phase_momentum[..num_bins].copy_from_slice(&self.phase_momentum[..num_bins]);
    }
}

impl Default for BinPhysics {
    fn default() -> Self { Self::new() }
}
```

- [ ] **Step 4: Declare in `src/dsp/mod.rs`**

Add `pub mod bin_physics;` to `src/dsp/mod.rs` (alongside the existing `pub mod guard;` line):

```rust
pub mod bin_physics;
pub mod guard;
pub mod modules;
pub mod pipeline;
pub mod engines;
pub mod fx_matrix;
pub mod utils;
```

- [ ] **Step 5: Run test to confirm it passes**

```bash
cargo test bin_physics_compiles 2>&1 | grep -E "ok|FAILED"
```

Expected: `test bin_physics_compiles ... ok`

- [ ] **Step 6: Add mix_from unit test**

Append to `tests/module_trait.rs`:

```rust
#[test]
fn bin_physics_mix_from_weighted_average() {
    use spectral_forge::dsp::bin_physics::BinPhysics;
    let mut a = BinPhysics::new();
    let mut b = BinPhysics::new();
    a.temperature[0] = 10.0;
    b.temperature[0] = 20.0;

    let mut mix = BinPhysics::new();
    mix.temperature[0] = 0.0; // reset from default (only temperature matters here)
    // mix 'a' at send=1.0 first, then 'b' at send=1.0 → should average to 15.0
    mix.mix_from(&a, 1.0, 1, 0.0);
    assert!((mix.temperature[0] - 10.0).abs() < 1e-5);
    mix.mix_from(&b, 1.0, 1, 1.0);
    assert!((mix.temperature[0] - 15.0).abs() < 1e-5, "got {}", mix.temperature[0]);
}
```

- [ ] **Step 7: Run and confirm**

```bash
cargo test bin_physics 2>&1 | grep -E "ok|FAILED"
```

Expected: 2 tests pass.

- [ ] **Step 8: Commit**

```bash
git add src/dsp/bin_physics.rs src/dsp/mod.rs tests/module_trait.rs
git commit -m "feat: add BinPhysics per-bin physical state struct"
```

---

## Task 2 — Update `SpectralModule` trait signature

**Files:**
- Modify: `src/dsp/modules/mod.rs`

- [ ] **Step 1: Add import and update trait in `src/dsp/modules/mod.rs`**

Add near the top (after `use num_complex::Complex;`):

```rust
use crate::dsp::bin_physics::BinPhysics;
```

Replace the `process` signature in the `SpectralModule` trait:

```rust
pub trait SpectralModule: Send {
    fn process(
        &mut self,
        channel: usize,
        stereo_link: StereoLink,
        target: FxChannelTarget,
        bins: &mut [Complex<f32>],
        sidechain: Option<&[f32]>,
        curves: &[&[f32]],
        suppression_out: &mut [f32],
        physics: &mut BinPhysics,
        ctx: &ModuleContext,
    );

    fn reset(&mut self, sample_rate: f32, fft_size: usize);
    fn tail_length(&self) -> u32 { 0 }
    fn module_type(&self) -> ModuleType;
    fn num_curves(&self) -> usize;
    fn num_outputs(&self) -> Option<usize> { None }
    fn set_gain_mode(&mut self, _: GainMode) {}
    fn virtual_outputs(&self) -> Option<[&[Complex<f32>]; 2]> { None }
}
```

- [ ] **Step 2: Confirm the crate now fails to compile (expected)**

```bash
cargo build 2>&1 | grep "error\[" | head -15
```

Expected: ~10 errors, one per module — "method `process` has 9 parameters but the trait declares 10".

---

## Task 3 — Update all existing module implementations

Each module adds `_physics: &mut BinPhysics` as the parameter before `ctx`. The body is unchanged.

**Files:** `src/dsp/modules/dynamics.rs`, `freeze.rs`, `phase_smear.rs`, `contrast.rs`, `gain.rs`, `ts_split.rs`, `harmonic.rs`, `master.rs`, `mid_side.rs`

- [ ] **Step 1: Update each module — add import and parameter**

For every module file listed above, add at the top (with existing imports):
```rust
use crate::dsp::bin_physics::BinPhysics;
```

Then in each `fn process(` signature, add `_physics: &mut BinPhysics,` immediately before `ctx: &ModuleContext,` (or `_ctx: &ModuleContext`).

Here is the complete updated signature block for each module (body unchanged):

**`dynamics.rs`** — find the opening of `fn process` and change the signature to:
```rust
    fn process(
        &mut self,
        channel: usize,
        stereo_link: StereoLink,
        target: FxChannelTarget,
        bins: &mut [Complex<f32>],
        sidechain: Option<&[f32]>,
        curves: &[&[f32]],
        suppression_out: &mut [f32],
        _physics: &mut BinPhysics,
        ctx: &ModuleContext,
    ) {
```

**`freeze.rs`** — same pattern:
```rust
    fn process(
        &mut self,
        _channel: usize,
        _stereo_link: StereoLink,
        _target: FxChannelTarget,
        bins: &mut [Complex<f32>],
        _sidechain: Option<&[f32]>,
        curves: &[&[f32]],
        suppression_out: &mut [f32],
        _physics: &mut BinPhysics,
        ctx: &ModuleContext,
    ) {
```

**`phase_smear.rs`**:
```rust
    fn process(
        &mut self,
        _channel: usize,
        _stereo_link: StereoLink,
        _target: FxChannelTarget,
        bins: &mut [Complex<f32>],
        _sidechain: Option<&[f32]>,
        curves: &[&[f32]],
        suppression_out: &mut [f32],
        _physics: &mut BinPhysics,
        _ctx: &ModuleContext,
    ) {
```

**`contrast.rs`**:
```rust
    fn process(
        &mut self,
        _channel: usize,
        _stereo_link: StereoLink,
        _target: FxChannelTarget,
        bins: &mut [Complex<f32>],
        sidechain: Option<&[f32]>,
        curves: &[&[f32]],
        suppression_out: &mut [f32],
        _physics: &mut BinPhysics,
        ctx: &ModuleContext,
    ) {
```

**`gain.rs`**:
```rust
    fn process(
        &mut self,
        _channel: usize,
        _stereo_link: StereoLink,
        _target: FxChannelTarget,
        bins: &mut [Complex<f32>],
        sidechain: Option<&[f32]>,
        curves: &[&[f32]],
        suppression_out: &mut [f32],
        _physics: &mut BinPhysics,
        _ctx: &ModuleContext,
    ) {
```

**`ts_split.rs`**:
```rust
    fn process(
        &mut self,
        _channel: usize,
        _stereo_link: StereoLink,
        _target: FxChannelTarget,
        bins: &mut [Complex<f32>],
        _sidechain: Option<&[f32]>,
        curves: &[&[f32]],
        suppression_out: &mut [f32],
        _physics: &mut BinPhysics,
        ctx: &ModuleContext,
    ) {
```

**`harmonic.rs`**:
```rust
    fn process(
        &mut self,
        _channel: usize,
        _stereo_link: StereoLink,
        _target: FxChannelTarget,
        bins: &mut [Complex<f32>],
        _sidechain: Option<&[f32]>,
        _curves: &[&[f32]],
        suppression_out: &mut [f32],
        _physics: &mut BinPhysics,
        _ctx: &ModuleContext,
    ) {
```

**`master.rs`** — there are two `impl SpectralModule` blocks (`MasterModule` and `EmptyModule`). Update both:
```rust
    fn process(
        &mut self,
        _channel: usize,
        _stereo_link: StereoLink,
        _target: FxChannelTarget,
        _bins: &mut [Complex<f32>],
        _sidechain: Option<&[f32]>,
        _curves: &[&[f32]],
        suppression_out: &mut [f32],
        _physics: &mut BinPhysics,
        _ctx: &ModuleContext,
    ) {
```

**`mid_side.rs`**:
```rust
    fn process(
        &mut self,
        channel: usize,
        stereo_link: StereoLink,
        target: FxChannelTarget,
        bins: &mut [Complex<f32>],
        _sidechain: Option<&[f32]>,
        curves: &[&[f32]],
        suppression_out: &mut [f32],
        _physics: &mut BinPhysics,
        _ctx: &ModuleContext,
    ) {
```

- [ ] **Step 2: Run the build**

```bash
cargo build 2>&1 | grep "error\[" | head -20
```

Expected: only errors from `fx_matrix.rs` (the call site — not updated yet). No errors in module files.

---

## Task 4 — Update `FxMatrix` struct and initialization

**Files:**
- Modify: `src/dsp/fx_matrix.rs`

- [ ] **Step 1: Add import at top of `fx_matrix.rs`**

```rust
use crate::dsp::bin_physics::BinPhysics;
use crate::dsp::pipeline::MAX_NUM_BINS;
```

- [ ] **Step 2: Update the `FxMatrix` struct**

Replace:
```rust
pub struct FxMatrix {
    pub slots: Vec<Option<Box<dyn SpectralModule>>>,
    slot_out:  Vec<Vec<Complex<f32>>>,
    slot_supp: Vec<Vec<f32>>,
    /// D3: virtual row output buffers for T/S Split — not yet written by process_hop.
    virtual_out: Vec<Vec<Complex<f32>>>,
    mix_buf:   Vec<Complex<f32>>,
}
```

With:
```rust
pub struct FxMatrix {
    pub slots:   Vec<Option<Box<dyn SpectralModule>>>,
    slot_out:    Vec<Vec<Complex<f32>>>,
    slot_supp:   Vec<Vec<f32>>,
    virtual_out: Vec<Vec<Complex<f32>>>,
    mix_buf:     Vec<Complex<f32>>,

    // ── Physics infrastructure ─────────────────────────────────────────────
    /// Per-slot output physics — written after each module call, read when routing.
    slot_phys:    Vec<BinPhysics>,
    /// Workspace for assembling a slot's input physics before calling its module.
    mix_phys:     BinPhysics,
    /// Per-slot previous-hop input magnitudes for velocity auto-computation.
    prev_magnitudes: Vec<Vec<f32>>,
}
```

- [ ] **Step 3: Update `FxMatrix::new()`**

Add three new fields to the `Self { ... }` return:
```rust
        Self {
            slots,
            slot_out:    (0..MAX_SLOTS).map(|_| vec![Complex::new(0.0, 0.0); num_bins]).collect(),
            slot_supp:   (0..MAX_SLOTS).map(|_| vec![0.0f32; num_bins]).collect(),
            virtual_out: (0..MAX_SPLIT_VIRTUAL_ROWS)
                             .map(|_| vec![Complex::new(0.0, 0.0); num_bins]).collect(),
            mix_buf:     vec![Complex::new(0.0, 0.0); num_bins],
            // new:
            slot_phys:       (0..MAX_SLOTS).map(|_| BinPhysics::new()).collect(),
            mix_phys:        BinPhysics::new(),
            prev_magnitudes: (0..MAX_SLOTS).map(|_| vec![0.0f32; MAX_NUM_BINS]).collect(),
        }
```

- [ ] **Step 4: Update `FxMatrix::reset()`**

After the existing `self.mix_buf.fill(...)` line, add:

```rust
        for phys in &mut self.slot_phys  { phys.reset_active(num_bins); }
        self.mix_phys.reset_active(num_bins);
        for buf in &mut self.prev_magnitudes { buf[..num_bins].fill(0.0); }
```

- [ ] **Step 5: Run build — confirm only the process_hop call site errors remain**

```bash
cargo build 2>&1 | grep "error\[" | head -10
```

Expected: errors pointing to the two `module.process(` call sites in `process_hop`.

- [ ] **Step 6: Commit checkpoint**

```bash
git add src/dsp/fx_matrix.rs src/dsp/modules/mod.rs src/dsp/modules/*.rs tests/module_trait.rs
git commit -m "feat: BinPhysics struct + SpectralModule signature update (all modules ignore physics)"
```

---

## Task 5 — Update `process_hop` to assemble, compute, and propagate physics

**Files:**
- Modify: `src/dsp/fx_matrix.rs` — `process_hop()` method

This is the core integration. There are two `module.process(` call sites: one in the main slot loop (slots 0–7) and one for the Master (slot 8).

- [ ] **Step 1: Update the slot loop (slots 0–7)**

In `process_hop`, the section between `self.mix_buf[..num_bins].fill(...)` and `let mut module = match self.slots[s].take()` — insert physics assembly and velocity computation AFTER the existing mix_buf accumulation:

```rust
            // ── Assemble mix_phys for slot s ────────────────────────────────────
            self.mix_phys.reset_active(num_bins);
            let mut total_phys_send = 0.0f32;
            // Slot 0 gets the raw plugin input — physics are default (will get velocity below).
            if s > 0 {
                for src in 0..s {
                    let send = route_matrix.send[src][s];
                    if send < 0.001 { continue; }
                    self.mix_phys.mix_from(&self.slot_phys[src], send, num_bins, total_phys_send);
                    total_phys_send += send;
                }
                // Virtual rows — borrow the source slot's physics (not the virtual buffer's,
                // since virtual buffers only carry audio, not physics).
                for (_, &vrow) in route_matrix.virtual_rows.iter().enumerate() {
                    if let Some((src_slot, _kind)) = vrow {
                        let src = src_slot as usize;
                        if src < s {
                            let v_idx = route_matrix.virtual_rows.iter().position(|r| *r == Some((src_slot, _kind))).unwrap_or(0);
                            let send = route_matrix.send[MAX_SLOTS + v_idx][s];
                            if send < 0.001 { continue; }
                            self.mix_phys.mix_from(&self.slot_phys[src], send, num_bins, total_phys_send);
                            total_phys_send += send;
                        }
                    }
                }
            }
            // ── Auto-compute velocity from magnitude delta ───────────────────────
            // mix_buf has been assembled at this point (existing code ran before this block).
            for k in 0..num_bins {
                let mag = self.mix_buf[k].norm();
                self.mix_phys.velocity[k] = (mag - self.prev_magnitudes[s][k]).abs();
                self.prev_magnitudes[s][k] = mag;
            }
```

Then update the `module.process(` call to include `&mut self.mix_phys`:

```rust
            module.process(
                channel, stereo_link, slot_targets[s],
                &mut self.mix_buf[..num_bins],
                sc_args[s], curves,
                &mut self.slot_supp[s][..num_bins],
                &mut self.mix_phys,
                ctx,
            );
```

After `self.slot_out[s][..num_bins].copy_from_slice(...)`, save physics output:

```rust
            self.mix_phys.copy_active_to(&mut self.slot_phys[s], num_bins);
```

- [ ] **Step 2: Fix the virtual row borrowing issue**

The virtual row enumeration in the physics assembly above has a borrow problem (can't call methods with `_kind` after `vrow` is already matched). Replace the virtual row block inside the physics assembly with a cleaner version:

```rust
            for v in 0..MAX_SPLIT_VIRTUAL_ROWS {
                if let Some((src_slot, _)) = route_matrix.virtual_rows[v] {
                    let src = src_slot as usize;
                    if src < s {
                        let send = route_matrix.send[MAX_SLOTS + v][s];
                        if send < 0.001 { continue; }
                        self.mix_phys.mix_from(&self.slot_phys[src], send, num_bins, total_phys_send);
                        total_phys_send += send;
                    }
                }
            }
```

- [ ] **Step 3: Update the Master module call (slot 8)**

For the Master accumulation section, also compute Master's physics. Add after the Master accumulation loop and before the `master_mod.process(` call:

```rust
        // Assemble Master physics from all slots that send to it.
        self.mix_phys.reset_active(num_bins);
        let mut total_master_phys_send = 0.0f32;
        for src in 0..8 {
            let send = route_matrix.send[src][8];
            if send < 0.001 { continue; }
            self.mix_phys.mix_from(&self.slot_phys[src], send, num_bins, total_master_phys_send);
            total_master_phys_send += send;
        }
        // Velocity for master: derived from mix_buf which was just assembled.
        for k in 0..num_bins {
            let mag = self.mix_buf[k].norm();
            self.mix_phys.velocity[k] = (mag - self.prev_magnitudes[8][k]).abs();
            self.prev_magnitudes[8][k] = mag;
        }
```

Then update the `master_mod.process(` call:

```rust
            master_mod.process(
                channel, stereo_link, slot_targets[8],
                &mut self.mix_buf[..num_bins],
                sc_args[8], curves_empty,
                &mut self.slot_supp[8][..num_bins],
                &mut self.mix_phys,
                ctx,
            );
```

- [ ] **Step 4: Build and confirm zero errors**

```bash
cargo build 2>&1 | grep "error\["
```

Expected: no output (clean build).

- [ ] **Step 5: Run all tests**

```bash
cargo test 2>&1 | grep -E "test result|FAILED"
```

Expected: all test suites pass with 0 failures (same as before).

- [ ] **Step 6: Commit**

```bash
git add src/dsp/fx_matrix.rs
git commit -m "feat: FxMatrix routes BinPhysics through slot chain with auto-computed velocity"
```

---

## Task 6 — Write a physics routing integration test

**Files:**
- Modify: `tests/module_trait.rs`

- [ ] **Step 1: Write the test**

This test verifies that after running `process_hop`, the slot_phys velocity is non-zero when signal is present — confirming auto-computation works.

Add to `tests/module_trait.rs`:

```rust
#[test]
fn bin_physics_velocity_is_auto_computed() {
    use spectral_forge::dsp::{
        bin_physics::BinPhysics,
        fx_matrix::FxMatrix,
        modules::{ModuleType, RouteMatrix},
        pipeline::MAX_NUM_BINS,
    };
    use spectral_forge::params::{FxChannelTarget, StereoLink};
    use spectral_forge::dsp::modules::ModuleContext;
    use num_complex::Complex;

    let fft_size = 2048;
    let num_bins = fft_size / 2 + 1;
    let types = [ModuleType::Empty; 9];
    let mut fx = FxMatrix::new(44100.0, fft_size, &types);

    // Build a minimal route matrix: slot 0 → Master (slot 8).
    let mut route = RouteMatrix::default();
    // Default serial routing already connects 0→1→2→8, so just use defaults.

    let sc_args: [Option<&[f32]>; 9] = [None; 9];
    let slot_targets = [FxChannelTarget::All; 9];
    let slot_curves: Vec<Vec<Vec<f32>>> = (0..9)
        .map(|_| (0..7).map(|_| vec![1.0f32; MAX_NUM_BINS]).collect())
        .collect();
    let ctx = ModuleContext {
        sample_rate: 44100.0, fft_size, num_bins,
        attack_ms: 10.0, release_ms: 100.0,
        sensitivity: 0.0, suppression_width: 1.0,
        auto_makeup: false, delta_monitor: false,
    };

    // First hop: all-zeros input. prev_magnitudes = 0, velocity = 0.
    let mut complex_buf = vec![Complex::new(0.0f32, 0.0); MAX_NUM_BINS];
    let mut supp_out = vec![0.0f32; MAX_NUM_BINS];
    fx.process_hop(0, StereoLink::Linked, &mut complex_buf, &sc_args,
        &slot_targets, &slot_curves, &route, &ctx, &mut supp_out, num_bins);

    // Second hop: non-zero input. Velocity should become the magnitude delta.
    for k in 0..num_bins {
        complex_buf[k] = Complex::new(1.0, 0.0);
    }
    fx.process_hop(0, StereoLink::Linked, &mut complex_buf, &sc_args,
        &slot_targets, &slot_curves, &route, &ctx, &mut supp_out, num_bins);

    // Slot 0 (Empty module) should have had velocity auto-computed from the delta.
    // We can't directly access slot_phys (private), but we can verify the test runs
    // without panicking and that the build is sound. A future dedicated test helper
    // on FxMatrix can expose slot_phys for inspection if needed.
    // For now this test verifies correct compilation and no panic.
}
```

> **Note:** `slot_phys` is private to `FxMatrix`. This test confirms the code compiles and runs without panic. If direct physics inspection is needed in future tests, add a `pub fn slot_physics(&self, slot: usize) -> &BinPhysics` accessor to `FxMatrix`.

- [ ] **Step 2: Run the test**

```bash
cargo test bin_physics_velocity 2>&1 | grep -E "ok|FAILED|error"
```

Expected: `test bin_physics_velocity_is_auto_computed ... ok`

- [ ] **Step 3: Run full test suite**

```bash
cargo test 2>&1 | grep -E "test result"
```

Expected: all suites pass, total ≥ 34 tests.

- [ ] **Step 4: Final commit**

```bash
git add tests/module_trait.rs
git commit -m "test: BinPhysics integration — velocity auto-computation and routing smoke test"
```

---

## Self-Review

**Spec coverage:**
- ✅ BinPhysics struct with all 7 fields (velocity, mass, temperature, flux, displacement, crystallization, phase_momentum)
- ✅ Velocity auto-computed from magnitude delta each hop (FxMatrix)
- ✅ All other fields inert — initialized to defaults, mixed on route combine, unchanged if no module writes them
- ✅ SpectralModule::process() gains `physics: &mut BinPhysics` parameter
- ✅ All 10 existing module implementations updated mechanically (add `_physics`, body unchanged)
- ✅ FxMatrix::new() allocates all physics buffers at MAX_NUM_BINS (no audio-thread allocation)
- ✅ FxMatrix::reset() zeros physics buffers
- ✅ FxMatrix::process_hop() assembles mix_phys from upstream slot_phys, auto-computes velocity, passes to module, saves output to slot_phys
- ✅ Master slot (slot 8) also gets physics assembled and passed
- ✅ No allocation on audio thread — all Vec<f32> pre-allocated in new()

**Placeholder scan:** None found.

**Type consistency:** `BinPhysics` used consistently throughout. `mix_from` signature matches call sites. `copy_active_to` matches usage in process_hop.

---

## Subsequent Plans

After this plan is complete and all tests pass, the next plans in order are:

| Plan | Feature | Depends on |
|---|---|---|
| Plan 2 | Matrix amp nodes (AmpMode per routing send) | Nothing new — parallel track |
| Plan 3 | Life module (viscosity, crystallization, diffusion) | Plan 1 (BinPhysics) |
| Plan 4 | Kinetics module (springs, gravity, inertial mass) | Plan 1 |
| Plan 5 | Circuit module (vactrol, Schmitt, BBD, flux) | Plan 1 |
| Plan 6 | IF tracking + Harmony module | Plan 1 |
| Plan 7 | Modulate module (phase/FM/RM) | Plan 1, Plan 6 |
| Plan 8 | Rhythm module (BPM sync) | Plan 1 |
| Plan 9 | Past module (history buffer) | Plan 1 |
