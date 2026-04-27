# Phase 3: BinPhysics Infrastructure Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** add per-bin persistent physics state (`BinPhysics`) that travels through `FxMatrix` between slots, with per-field merge rules, opt-in module read/write access, and calibration probes.

**Architecture:** `BinPhysics` is a flat struct of `Vec<f32>` arrays (one per property, sized to `MAX_NUM_BINS`). `FxMatrix` owns one `BinPhysics` per slot (output state) plus a workspace for assembled inputs. Modules subscribe by reading `ctx.bin_physics: Option<&BinPhysics>` (Phase 1 added the field) and by setting `ModuleSpec.writes_bin_physics = true` to opt into the writer schedule. Per-field merge rules replace the older "always amplitude-weighted" rule from the 2026-04-21 spec.

**Tech Stack:** Rust, no new dependencies.

**Status banner to add at the top of each PR's commit message:** `infra(phase3):`

**This plan supersedes** `docs/superpowers/plans/2026-04-21-bin-physics-infrastructure.md`. The older plan's task skeleton is good but it (a) misses the four new fields surfaced in the audit and (b) hard-codes amplitude-weighted mixing for every field. Read both files; this one wins where they disagree.

**Reading order before starting:**
- `ideas/next-gen-modules/01-global-infrastructure.md` § 1 (audit additions)
- `ideas/next-gen-modules/02-architectural-refactors.md` § 1a, § 7
- `docs/superpowers/specs/2026-04-21-bin-physics-infrastructure.md` (original spec)
- `docs/superpowers/plans/2026-04-21-bin-physics-infrastructure.md` (older plan — task skeleton)
- `src/dsp/fx_matrix.rs` (where the integration lives)
- `src/dsp/pipeline.rs` § the per-hop loop
- The Phase 1 plan (`2026-04-27-phase-1-foundation-infra.md`) — Task 2 added the `ctx.bin_physics` slot

**Phase 1 prerequisite:** `ModuleContext` has the `'block` lifetime. This plan adds `ctx.bin_physics: Option<&'block BinPhysics>` to that struct.

---

## File Structure

| File | Created/Modified | Responsibility |
|---|---|---|
| `src/dsp/bin_physics.rs` | Create | `BinPhysics` struct, per-field defaults, `mix_from()` with per-field rules, `compute_velocity()`, `reset_active()` |
| `src/dsp/mod.rs` | Modify | `pub mod bin_physics;` |
| `src/dsp/modules/mod.rs` | Modify | Add `bin_physics: Option<&'block BinPhysics>` to `ModuleContext`. Add `writes_bin_physics: bool` to `ModuleSpec`. Update trait `process()` signature to take `physics: Option<&mut BinPhysics>`. |
| `src/dsp/fx_matrix.rs` | Modify | Per-slot `BinPhysics` state arrays; assembly of mixed input physics from sends; auto-velocity from input magnitudes; writer-before-reader topological sort. |
| `src/dsp/pipeline.rs` | Modify | Set `ctx.bin_physics = Some(&fx_matrix.read_only_view())` after FxMatrix populates. |
| `src/dsp/modules/{dynamics,freeze,phase_smear,contrast,gain,mid_side,ts_split,harmonic,master}.rs` | Modify (signature only) | Add ignored `_physics: Option<&mut BinPhysics>` arg. No behaviour change. |
| `tests/bin_physics.rs` | Create | Unit tests for the struct, mix rules, and velocity computation. |
| `tests/bin_physics_pipeline.rs` | Create | Integration: a writer module sets `mass`, a reader sees it on the next slot; multi-send mixing test; writer-reader ordering test. |
| `tests/calibration.rs` | Modify | Add probes for `mass`, `temperature`, `flux`, `crystallization`, `phase_momentum`, `slew`, `bias`. |

---

## Task 1: Create `BinPhysics` struct with per-field merge rules

**Files:**
- Create: `src/dsp/bin_physics.rs`
- Test: `tests/bin_physics.rs` (new)

- [ ] **Step 1.1: Write failing tests**

Create `tests/bin_physics.rs`:

```rust
use spectral_forge::dsp::bin_physics::{BinPhysics, MergeRule};

#[test]
fn defaults_match_spec() {
    let p = BinPhysics::new();
    assert_eq!(p.velocity[0], 0.0);
    assert_eq!(p.mass[0], 1.0);             // inertia = 1 (no resistance)
    assert_eq!(p.temperature[0], 0.0);
    assert_eq!(p.flux[0], 0.0);
    assert_eq!(p.displacement[0], 0.0);
    assert_eq!(p.crystallization[0], 0.0);
    assert_eq!(p.phase_momentum[0], 0.0);
    assert_eq!(p.slew[0], 0.0);
    assert_eq!(p.bias[0], 0.0);
    assert_eq!(p.decay_estimate[0], 0.0);
    // lock_target_freq defaults to bin centre frequency — set per-bin in reset_active.
}

#[test]
fn merge_rule_max_wins_picks_higher() {
    let mut dst = 0.2;
    BinPhysics::merge_one(&mut dst, 0.7, 0.3, MergeRule::Max);
    assert!((dst - 0.7).abs() < 1e-6);
}

#[test]
fn merge_rule_weighted_avg_blends() {
    let mut dst = 0.0;
    // Already-mixed dst (weight 0.0 implicit), incoming 1.0 with weight 0.5
    BinPhysics::merge_one(&mut dst, 1.0, 0.5, MergeRule::WeightedAvg);
    assert!((dst - 0.5).abs() < 1e-6);
    BinPhysics::merge_one(&mut dst, 0.0, 0.5, MergeRule::WeightedAvg);
    // dst now: prior 0.5 weighted by 0.5, plus 0.0 weighted by 0.5 = 0.25
    assert!((dst - 0.25).abs() < 1e-6);
}

#[test]
fn velocity_computed_from_magnitude_delta() {
    use num_complex::Complex;
    let mut p = BinPhysics::new();
    let prev = vec![Complex::new(0.0_f32, 0.0); 4];
    let curr = vec![Complex::new(0.5, 0.0), Complex::new(0.25, 0.0), Complex::new(0.0, 0.0), Complex::new(0.1, 0.0)];
    BinPhysics::compute_velocity(&mut p.velocity, &prev, &curr, 4);
    assert!((p.velocity[0] - 0.5).abs() < 1e-6);
    assert!((p.velocity[1] - 0.25).abs() < 1e-6);
    assert!((p.velocity[2] - 0.0).abs() < 1e-6);
    assert!((p.velocity[3] - 0.1).abs() < 1e-6);
}
```

Run: `cargo test --test bin_physics`
Expected: FAIL with "module `bin_physics` not found".

- [ ] **Step 1.2: Implement the struct**

Create `src/dsp/bin_physics.rs`:

```rust
//! Per-bin persistent physics state, transported through FxMatrix between slots.
//!
//! See `docs/superpowers/specs/2026-04-21-bin-physics-infrastructure.md` for the
//! original design and `ideas/next-gen-modules/01-global-infrastructure.md § 1`
//! for the audit additions (slew/bias/decay_estimate/lock_target_freq + per-field
//! merge rules).

use num_complex::Complex;
use crate::dsp::pipeline::MAX_NUM_BINS;

/// Per-field rule used when multiple sends mix into one destination slot's input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MergeRule {
    /// Standard amplitude-weighted average (matches audio-bin mixing).
    /// Use for: temperature, flux, displacement, phase_momentum, slew, bias,
    /// decay_estimate, lock_target_freq.
    WeightedAvg,
    /// Take the higher of the two values. Reasoning: harder to break than to form.
    /// Use for: crystallization.
    Max,
    /// Take the higher mass — heavier parent dominates.
    /// Use for: mass.
    HeavierWins,
}

pub struct BinPhysics {
    pub velocity:        Vec<f32>,   // auto-computed each hop, never stored
    pub mass:            Vec<f32>,   // default 1.0
    pub temperature:     Vec<f32>,
    pub flux:            Vec<f32>,
    pub displacement:    Vec<f32>,
    pub crystallization: Vec<f32>,   // [0, 1]
    pub phase_momentum:  Vec<f32>,
    pub slew:            Vec<f32>,   // audit add: rate-of-change *limit*, not the rate
    pub bias:            Vec<f32>,   // audit add: time-averaged DC offset of bin's complex value
    pub decay_estimate:  Vec<f32>,   // audit add: frames-to-fall-20-dB
    pub lock_target_freq: Vec<f32>,  // audit add: defaults to bin centre frequency
}

impl BinPhysics {
    pub fn new() -> Self {
        Self {
            velocity:        vec![0.0; MAX_NUM_BINS],
            mass:            vec![1.0; MAX_NUM_BINS],
            temperature:     vec![0.0; MAX_NUM_BINS],
            flux:            vec![0.0; MAX_NUM_BINS],
            displacement:    vec![0.0; MAX_NUM_BINS],
            crystallization: vec![0.0; MAX_NUM_BINS],
            phase_momentum:  vec![0.0; MAX_NUM_BINS],
            slew:            vec![0.0; MAX_NUM_BINS],
            bias:            vec![0.0; MAX_NUM_BINS],
            decay_estimate:  vec![0.0; MAX_NUM_BINS],
            lock_target_freq: vec![0.0; MAX_NUM_BINS],   // populated in reset_active()
        }
    }

    /// Reset the active region to defaults. `sample_rate / fft_size` lets us seed
    /// `lock_target_freq[k] = k * sample_rate / fft_size`.
    pub fn reset_active(&mut self, num_bins: usize, sample_rate: f32, fft_size: usize) {
        self.velocity[..num_bins].fill(0.0);
        self.mass[..num_bins].fill(1.0);
        self.temperature[..num_bins].fill(0.0);
        self.flux[..num_bins].fill(0.0);
        self.displacement[..num_bins].fill(0.0);
        self.crystallization[..num_bins].fill(0.0);
        self.phase_momentum[..num_bins].fill(0.0);
        self.slew[..num_bins].fill(0.0);
        self.bias[..num_bins].fill(0.0);
        self.decay_estimate[..num_bins].fill(0.0);
        let bin_hz = sample_rate / fft_size as f32;
        for k in 0..num_bins {
            self.lock_target_freq[k] = k as f32 * bin_hz;
        }
    }

    /// Apply a single send into a destination value using the merge rule.
    /// `weight` is the send amplitude clamped to [0, 1].
    #[inline]
    pub fn merge_one(dst: &mut f32, src: f32, weight: f32, rule: MergeRule) {
        let w = weight.clamp(0.0, 1.0);
        match rule {
            MergeRule::WeightedAvg => *dst = *dst * (1.0 - w) + src * w,
            MergeRule::Max         => *dst = dst.max(src),
            MergeRule::HeavierWins => if src > *dst { *dst = src; },
        }
    }

    /// Compute per-bin velocity from the magnitude delta between previous and
    /// current FFT frames. Velocity is the absolute change in magnitude per hop.
    /// Both slices must be at least `num_bins` long.
    pub fn compute_velocity(
        out_velocity: &mut [f32],
        prev_bins:    &[Complex<f32>],
        curr_bins:    &[Complex<f32>],
        num_bins:     usize,
    ) {
        for k in 0..num_bins {
            let prev_mag = prev_bins[k].norm();
            let curr_mag = curr_bins[k].norm();
            out_velocity[k] = (curr_mag - prev_mag).abs();
        }
    }

    /// Mix `other` into `self` with the given send weight, per per-field rule.
    /// `num_bins` bounds the active region.
    pub fn mix_from(&mut self, other: &BinPhysics, weight: f32, num_bins: usize) {
        for k in 0..num_bins {
            Self::merge_one(&mut self.mass[k],            other.mass[k],            weight, MergeRule::HeavierWins);
            Self::merge_one(&mut self.temperature[k],     other.temperature[k],     weight, MergeRule::WeightedAvg);
            Self::merge_one(&mut self.flux[k],            other.flux[k],            weight, MergeRule::WeightedAvg);
            Self::merge_one(&mut self.displacement[k],    other.displacement[k],    weight, MergeRule::WeightedAvg);
            Self::merge_one(&mut self.crystallization[k], other.crystallization[k], weight, MergeRule::Max);
            Self::merge_one(&mut self.phase_momentum[k],  other.phase_momentum[k],  weight, MergeRule::WeightedAvg);
            Self::merge_one(&mut self.slew[k],            other.slew[k],            weight, MergeRule::WeightedAvg);
            Self::merge_one(&mut self.bias[k],            other.bias[k],            weight, MergeRule::WeightedAvg);
            Self::merge_one(&mut self.decay_estimate[k],  other.decay_estimate[k],  weight, MergeRule::WeightedAvg);
            Self::merge_one(&mut self.lock_target_freq[k], other.lock_target_freq[k], weight, MergeRule::WeightedAvg);
        }
        // velocity is recomputed downstream — do not mix it here.
    }

    /// Cheap read-only view of the active region. For use in `ModuleContext`.
    pub fn active_view(&self, _num_bins: usize) -> &Self { self }
}
```

- [ ] **Step 1.3: Wire the module**

In `src/dsp/mod.rs`, add `pub mod bin_physics;`.

- [ ] **Step 1.4: Run tests**

Run: `cargo test --test bin_physics`
Expected: PASS.

- [ ] **Step 1.5: Commit**

```bash
git add src/dsp/bin_physics.rs src/dsp/mod.rs tests/bin_physics.rs
git commit -m "$(cat <<'EOF'
infra(phase3): create BinPhysics struct with per-field merge rules

Adds a per-bin physics carrier (mass, temperature, flux, displacement,
crystallization, phase_momentum + audit additions slew, bias,
decay_estimate, lock_target_freq) plus auto-velocity computation.
Per-field merge rules replace the older "always weighted-avg" mix:
- mass: heavier wins
- crystallization: max
- everything else: amplitude-weighted average

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: Add `bin_physics` to `ModuleContext` and `writes_bin_physics` to `ModuleSpec`

**Files:**
- Modify: `src/dsp/modules/mod.rs` (`ModuleContext` struct + `ModuleSpec` struct)
- Test: `tests/module_trait.rs`

- [ ] **Step 2.1: Write failing test**

Add to `tests/module_trait.rs`:

```rust
#[test]
fn module_context_has_bin_physics_slot() {
    use spectral_forge::dsp::modules::ModuleContext;
    let ctx = ModuleContext::new(48000.0, 2048, 1025, 10.0, 100.0, 1.0, 0.5, false, false);
    assert!(ctx.bin_physics.is_none());
}

#[test]
fn module_spec_writes_bin_physics_defaults_false() {
    use spectral_forge::dsp::modules::{ModuleType, module_spec};
    for ty in [ModuleType::Dynamics, ModuleType::Freeze, ModuleType::PhaseSmear] {
        assert!(!module_spec(ty).writes_bin_physics);
    }
}
```

Run: `cargo test --test module_trait module_context_has_bin_physics_slot module_spec_writes_bin_physics_defaults_false`
Expected: FAIL.

- [ ] **Step 2.2: Add `bin_physics` to `ModuleContext`**

In `src/dsp/modules/mod.rs`, add to the `ModuleContext` struct (where Phase 1 added the optional fields):

```rust
pub bin_physics: Option<&'block crate::dsp::bin_physics::BinPhysics>,
```

Update `ModuleContext::new()` to default `bin_physics: None`.

- [ ] **Step 2.3: Add `writes_bin_physics` to `ModuleSpec`**

In `src/dsp/modules/mod.rs`, in the `ModuleSpec` struct definition, add:

```rust
/// True if this module writes BinPhysics state. The pipeline uses this to
/// schedule writers before readers within a hop, and to skip the BinPhysics
/// assembly step entirely when no slot needs it.
pub writes_bin_physics: bool,
```

Add `writes_bin_physics: false,` to all 10 existing static `ModuleSpec` literals (Dynamics, Freeze, PhaseSmear, Contrast, Gain, MidSide, TS, Harmonic, Master, Empty).

- [ ] **Step 2.4: Run tests**

Run: `cargo test --test module_trait`
Expected: PASS.

- [ ] **Step 2.5: Commit**

```bash
git add src/dsp/modules/mod.rs tests/module_trait.rs
git commit -m "$(cat <<'EOF'
infra(phase3): expose BinPhysics via ModuleContext.bin_physics

Adds `bin_physics: Option<&'block BinPhysics>` to ModuleContext (still
None — Pipeline wiring lands in Task 5) and `writes_bin_physics: bool`
to ModuleSpec for the writer-before-reader scheduler. All shipped
modules opt out by default.

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: Extend the trait with `physics: Option<&mut BinPhysics>`

**Files:**
- Modify: `src/dsp/modules/mod.rs` (trait signature)
- Modify: each of `dsp/modules/{dynamics,freeze,phase_smear,contrast,gain,mid_side,ts_split,harmonic,master}.rs` (signature only)

- [ ] **Step 3.1: Update the trait signature**

In `src/dsp/modules/mod.rs`, change the trait to:

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
    physics: Option<&mut crate::dsp::bin_physics::BinPhysics>,  // NEW
    ctx: &ModuleContext<'_>,
);
```

- [ ] **Step 3.2: Update every shipped module's `process()` signature**

For each shipped module, add `_physics: Option<&mut crate::dsp::bin_physics::BinPhysics>,` to the arg list immediately before `ctx`. Body unchanged.

Example diff for `src/dsp/modules/dynamics.rs`:

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
    _physics: Option<&mut crate::dsp::bin_physics::BinPhysics>,  // NEW
    ctx: &ModuleContext<'_>,
) {
    // …existing body, unchanged…
}
```

- [ ] **Step 3.3: Update FxMatrix dispatch site**

In `src/dsp/fx_matrix.rs`, update the `module.process(...)` call to pass `None` for physics for now. Task 5 will plumb the real reference.

- [ ] **Step 3.4: Build & run all tests**

Run: `cargo build && cargo test`
Expected: all tests pass.

- [ ] **Step 3.5: Commit**

```bash
git add src/dsp/modules/mod.rs src/dsp/modules/*.rs src/dsp/fx_matrix.rs
git commit -m "$(cat <<'EOF'
infra(phase3): add physics: Option<&mut BinPhysics> to SpectralModule

Extends the trait signature so physics modules (Phase 5) can mutate
state. All shipped modules ignore the new arg — physics ref is None
until Task 5 plumbs the FxMatrix slot arrays.

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: FxMatrix per-slot BinPhysics arrays + auto-velocity

**Files:**
- Modify: `src/dsp/fx_matrix.rs`

- [ ] **Step 4.1: Inspect FxMatrix's current state**

Run: `grep -n 'pub struct FxMatrix\|fn new\|fn reset\|fn process_hop' src/dsp/fx_matrix.rs`
Identify the field list and the per-hop dispatch loop.

- [ ] **Step 4.2: Add per-slot BinPhysics fields**

In `src/dsp/fx_matrix.rs`, add to the struct:

```rust
/// One BinPhysics per slot — the *output* state after the slot's process().
/// Indexed by slot 0..MAX_SLOTS.
pub slot_phys: Vec<crate::dsp::bin_physics::BinPhysics>,

/// Workspace BinPhysics — used to assemble the mixed input for one slot.
/// Reused across slots within a hop.
mix_phys: crate::dsp::bin_physics::BinPhysics,

/// Per-slot previous-frame magnitudes for auto-velocity. SoA layout:
/// `prev_mags[slot * MAX_NUM_BINS + k]`.
prev_mags: Vec<f32>,

/// Set true at slot-assignment time when any active slot opts into BinPhysics
/// (read OR write). When false, the assembly + velocity loops are skipped.
bin_physics_in_use: bool,
```

- [ ] **Step 4.3: Initialize in `FxMatrix::new()`**

```rust
slot_phys: (0..MAX_SLOTS).map(|_| crate::dsp::bin_physics::BinPhysics::new()).collect(),
mix_phys:  crate::dsp::bin_physics::BinPhysics::new(),
prev_mags: vec![0.0; MAX_SLOTS * MAX_NUM_BINS],
bin_physics_in_use: false,
```

(`MAX_NUM_BINS` is in `src/dsp/pipeline.rs`; import it.)

- [ ] **Step 4.4: Reset in `FxMatrix::reset()`**

```rust
for p in &mut self.slot_phys {
    p.reset_active(num_bins, sample_rate, fft_size);
}
self.mix_phys.reset_active(num_bins, sample_rate, fft_size);
self.prev_mags.fill(0.0);
```

- [ ] **Step 4.5: Compute `bin_physics_in_use` at slot assignment time**

In the FxMatrix method that's called when a slot's module changes (find by `grep -n 'set_slot_module\|update_slots' src/dsp/fx_matrix.rs`), recompute:

```rust
self.bin_physics_in_use = (0..MAX_SLOTS).any(|s| {
    let ty = self.slot_module_type(s);  // or the equivalent accessor
    let spec = crate::dsp::modules::module_spec(ty);
    spec.writes_bin_physics
});
```

(Phase 5 modules will set `writes_bin_physics = true`. Phase 6 readers will set `reads_bin_physics: bool` — defer that field; for now any opt-in via `writes_bin_physics` makes the assembly happen.)

- [ ] **Step 4.6: Topological writer-before-reader sort**

In the `process_hop()` dispatch loop, replace the linear `for s in route_order` with a two-pass split:

```rust
// Pass 1: writers first (so readers in pass 2 see fresh state).
for &s in writer_order.iter() {
    self.dispatch_slot(s, /* …existing args… */);
}
// Pass 2: pure readers (modules that read but do not write physics).
for &s in reader_order.iter() {
    self.dispatch_slot(s, /* …existing args… */);
}
```

`writer_order` and `reader_order` are pre-computed `Vec<usize>` fields cached on the matrix (recomputed only when the slot type set changes — never per block).

- [ ] **Step 4.7: Auto-compute velocity in `process_hop()`**

Inside `dispatch_slot(s, …)` for each slot, *after* the input assembly for that slot but *before* calling `module.process()`:

```rust
if self.bin_physics_in_use {
    let prev_off = s * MAX_NUM_BINS;
    let prev_slice = &self.prev_mags[prev_off..prev_off + num_bins];
    // Compute |curr| - |prev| absolute delta into mix_phys.velocity.
    for k in 0..num_bins {
        let curr_mag = assembled_input[k].norm();
        self.mix_phys.velocity[k] = (curr_mag - prev_slice[k]).abs();
    }
    // Update prev_mags for next hop.
    for k in 0..num_bins {
        self.prev_mags[prev_off + k] = assembled_input[k].norm();
    }
}
```

- [ ] **Step 4.8: Pass `&mut mix_phys` into the module dispatch**

In the `module.process(...)` call inside `dispatch_slot`:

```rust
let physics_arg = if self.bin_physics_in_use {
    Some(&mut self.mix_phys)
} else {
    None
};
module.process(/* …existing args…, */ physics_arg, ctx);
```

After `process()` returns, copy the (possibly mutated) `mix_phys` into `slot_phys[s]`:

```rust
if self.bin_physics_in_use {
    self.slot_phys[s].copy_from(&self.mix_phys, num_bins);
    // Reset mix_phys for the next slot's input assembly.
    self.mix_phys.reset_active(num_bins, sample_rate, fft_size);
}
```

`copy_from` is a one-line helper to add to `BinPhysics`:

```rust
pub fn copy_from(&mut self, src: &BinPhysics, num_bins: usize) {
    self.velocity[..num_bins].copy_from_slice(&src.velocity[..num_bins]);
    self.mass[..num_bins].copy_from_slice(&src.mass[..num_bins]);
    self.temperature[..num_bins].copy_from_slice(&src.temperature[..num_bins]);
    self.flux[..num_bins].copy_from_slice(&src.flux[..num_bins]);
    self.displacement[..num_bins].copy_from_slice(&src.displacement[..num_bins]);
    self.crystallization[..num_bins].copy_from_slice(&src.crystallization[..num_bins]);
    self.phase_momentum[..num_bins].copy_from_slice(&src.phase_momentum[..num_bins]);
    self.slew[..num_bins].copy_from_slice(&src.slew[..num_bins]);
    self.bias[..num_bins].copy_from_slice(&src.bias[..num_bins]);
    self.decay_estimate[..num_bins].copy_from_slice(&src.decay_estimate[..num_bins]);
    self.lock_target_freq[..num_bins].copy_from_slice(&src.lock_target_freq[..num_bins]);
}
```

- [ ] **Step 4.9: Mix BinPhysics from upstream slots into mix_phys**

In `dispatch_slot(s, …)`, when assembling input for slot `s`, for each upstream slot `u` with `route_matrix.send[u][s] > 0.0`:

```rust
if self.bin_physics_in_use {
    let weight = route_matrix.send[u][s];
    self.mix_phys.mix_from(&self.slot_phys[u], weight, num_bins);
}
```

Order: zero `mix_phys` *before* the upstream loop (it was reset at end of previous slot — confirm). Then mix each upstream send.

- [ ] **Step 4.10: Build and run all tests**

Run: `cargo build && cargo test`
Expected: all tests pass; no allocation in `process()`.

- [ ] **Step 4.11: Commit**

```bash
git add src/dsp/fx_matrix.rs src/dsp/bin_physics.rs
git commit -m "$(cat <<'EOF'
infra(phase3): wire BinPhysics through FxMatrix

Adds per-slot output BinPhysics arrays + a workspace mix buffer + the
prev-magnitudes array used for auto-velocity. Writer-before-reader
ordering computed once at slot-assignment time. The whole assembly
loop is short-circuited (bin_physics_in_use=false) until any Phase 5
module opts in.

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: Pipeline plumbs `ctx.bin_physics`

**Files:**
- Modify: `src/dsp/pipeline.rs`

- [ ] **Step 5.1: Set `ctx.bin_physics` per slot**

The Pipeline does not directly call `module.process()` — FxMatrix does. So the cleanest place to attach `ctx.bin_physics` is *inside* `FxMatrix::dispatch_slot`, just before constructing the per-slot ctx variant:

```rust
let mut ctx_for_slot = *ctx;  // Copy-with-PhantomData; ctx had no &mut refs.
ctx_for_slot.bin_physics = if self.bin_physics_in_use {
    Some(&self.slot_phys[s])  // upstream output, after Pass 1 writers
} else {
    None
};
module.process(/* … */, &ctx_for_slot);
```

Wait — `ModuleContext` is no longer `Copy` after Phase 1 Task 1. Use a builder pattern or rebuild:

```rust
let ctx_for_slot = ModuleContext {
    bin_physics: if self.bin_physics_in_use { Some(&self.slot_phys[s]) } else { None },
    ..ctx.clone()
};
```

Add `Clone` derive to `ModuleContext` in `src/dsp/modules/mod.rs` (it's now non-`Copy` but `Clone` is fine because all fields are `Copy` or `Option<&_>`).

- [ ] **Step 5.2: Build and run all tests**

Run: `cargo build && cargo test`
Expected: PASS.

- [ ] **Step 5.3: Commit**

```bash
git add src/dsp/fx_matrix.rs src/dsp/modules/mod.rs
git commit -m "$(cat <<'EOF'
infra(phase3): expose BinPhysics to modules via ctx.bin_physics

FxMatrix builds a per-slot ModuleContext that points ctx.bin_physics
at the upstream slot's output state. Reader modules see fresh values
written by upstream writer modules in the same hop.

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>
EOF
)"
```

---

## Task 6: Integration test — writer module sets mass, reader sees it

**Goal:** prove the full read+write path works end-to-end with two mock modules. Not a real shipping module — just two test fixtures.

**Files:**
- Create: `tests/bin_physics_pipeline.rs`

- [ ] **Step 6.1: Define mock writer/reader test modules**

Create `tests/bin_physics_pipeline.rs`:

```rust
use num_complex::Complex;
use spectral_forge::dsp::bin_physics::BinPhysics;
use spectral_forge::dsp::modules::{
    ModuleContext, ModuleType, SpectralModule, GainMode,
    /* PeakInfo, */
};
use spectral_forge::params::{FxChannelTarget, StereoLink};

struct MockWriter;
impl SpectralModule for MockWriter {
    fn process(
        &mut self,
        _ch: usize, _link: StereoLink, _tgt: FxChannelTarget,
        bins: &mut [Complex<f32>], _sc: Option<&[f32]>, _curves: &[&[f32]],
        _supp: &mut [f32], physics: Option<&mut BinPhysics>,
        ctx: &ModuleContext<'_>,
    ) {
        // Set mass = 5.0 for every active bin. Pass through audio.
        if let Some(p) = physics {
            for k in 0..ctx.num_bins { p.mass[k] = 5.0; }
        }
        // Audio passthrough — bins already in place.
        let _ = bins;
    }
    fn reset(&mut self, _sr: f32, _fft: usize) {}
    fn module_type(&self) -> ModuleType { ModuleType::Empty }
    fn num_curves(&self) -> usize { 0 }
}

struct MockReader { observed_mass: f32 }
impl SpectralModule for MockReader {
    fn process(
        &mut self,
        _ch: usize, _link: StereoLink, _tgt: FxChannelTarget,
        _bins: &mut [Complex<f32>], _sc: Option<&[f32]>, _curves: &[&[f32]],
        _supp: &mut [f32], _physics: Option<&mut BinPhysics>,
        ctx: &ModuleContext<'_>,
    ) {
        if let Some(p) = ctx.bin_physics {
            self.observed_mass = p.mass[0];
        }
    }
    fn reset(&mut self, _sr: f32, _fft: usize) {}
    fn module_type(&self) -> ModuleType { ModuleType::Empty }
    fn num_curves(&self) -> usize { 0 }
}
```

- [ ] **Step 6.2: Write the integration test**

Add to the same file:

```rust
#[test]
fn writer_sets_mass_then_reader_observes_it() {
    use spectral_forge::dsp::fx_matrix::FxMatrix;
    let mut fxm = FxMatrix::new();
    fxm.reset(48000.0, 2048);

    // Install MockWriter at slot 0 (also flag it as a writer in the spec).
    // (Tests that need a writer-flagged spec hook in via a back-door API:
    //  fxm.test_install_module(slot, Box::new(MockWriter), writes_physics=true)
    //  — add this method behind #[cfg(test)] in fx_matrix.rs.)
    fxm.test_install_module(0, Box::new(MockWriter), true);
    let reader = Box::new(MockReader { observed_mass: -1.0 });
    fxm.test_install_module(1, reader, false);
    fxm.test_set_route(0, 1, 1.0);  // slot 0 → slot 1 at unit gain

    // One hop with non-silent input.
    let mut bins_l = vec![Complex::new(0.5_f32, 0.0); 1025];
    let mut bins_r = bins_l.clone();
    let curves = vec![vec![1.0_f32; 1025]; 7];
    let curve_refs: Vec<&[f32]> = curves.iter().map(|c| c.as_slice()).collect();
    fxm.test_dispatch_one_hop(&mut bins_l, &mut bins_r, &curve_refs);

    // Read back the reader's observed mass.
    let observed = fxm.test_module_observed_mass(1);
    assert!((observed - 5.0).abs() < 1e-5,
        "MockReader at slot 1 should see mass = 5.0 written by MockWriter at slot 0; got {}", observed);
}
```

`test_install_module`, `test_set_route`, `test_dispatch_one_hop`, `test_module_observed_mass` are `#[cfg(test)]` helpers added to `FxMatrix` in this task. Each is ~5 lines of "expose what you need". Keep them out of the public API.

- [ ] **Step 6.3: Implement the test helpers in `fx_matrix.rs`**

Add at the bottom of `src/dsp/fx_matrix.rs`:

```rust
#[cfg(test)]
impl FxMatrix {
    pub fn test_install_module(
        &mut self,
        slot: usize,
        module: Box<dyn SpectralModule>,
        writes_physics: bool,
    ) {
        self.slots[slot] = Some(module);
        if writes_physics {
            self.bin_physics_in_use = true;
            // Also recompute writer/reader orders.
            self.recompute_dispatch_order();
        }
    }
    pub fn test_set_route(&mut self, from: usize, to: usize, amount: f32) {
        self.route_matrix.send[from][to] = amount;
    }
    pub fn test_dispatch_one_hop(
        &mut self,
        bins_l: &mut [Complex<f32>],
        bins_r: &mut [Complex<f32>],
        curves: &[&[f32]],
    ) {
        // Minimal sub-call into process_hop with synthetic ctx.
        // Adapt to whatever process_hop's actual signature is.
        let ctx = ModuleContext::new(48000.0, 2048, 1025, 10.0, 100.0, 1.0, 0.5, false, false);
        self.process_hop(0, bins_l, &[], &[], curves, &Default::default(), &ctx);
        self.process_hop(1, bins_r, &[], &[], curves, &Default::default(), &ctx);
    }
    pub fn test_module_observed_mass(&self, slot: usize) -> f32 {
        // Downcast the slot's module to MockReader. Unsafe-ish but test-only.
        // Cleaner: store last-observed mass in the test module via interior mutability,
        // and read it by trait downcast. For simplicity, assume the test module
        // stashes its result in a global atomic — see test setup.
        // (If `&dyn SpectralModule` doesn't allow downcast, add `Any` superbound
        // behind cfg(test).)
        unimplemented!("see Task 6 step 6.4 for the Any-based downcast pattern");
    }
}
```

- [ ] **Step 6.4: Use a test-only `Any` superbound for downcast**

In `src/dsp/modules/mod.rs`:

```rust
#[cfg(test)]
pub trait SpectralModuleAny: SpectralModule + std::any::Any {}
#[cfg(test)]
impl<T: SpectralModule + std::any::Any> SpectralModuleAny for T {}
```

Then `test_module_observed_mass` becomes:

```rust
let m_any = self.slots[slot].as_ref().unwrap().as_ref() as &dyn std::any::Any;
m_any.downcast_ref::<MockReader>().unwrap().observed_mass
```

(If trait object types don't compose cleanly here, an alternative is to use a `static AtomicU32` in the test module that records the observed mass in `process()`.)

- [ ] **Step 6.5: Run the test**

Run: `cargo test --test bin_physics_pipeline`
Expected: PASS.

- [ ] **Step 6.6: Commit**

```bash
git add tests/bin_physics_pipeline.rs src/dsp/fx_matrix.rs src/dsp/modules/mod.rs
git commit -m "$(cat <<'EOF'
test(phase3): integration test for BinPhysics writer→reader path

Two mock modules: writer sets mass=5.0 in slot 0, reader at slot 1
observes mass=5.0 via ctx.bin_physics. Verifies the writer-before-
reader dispatch order, the route-weighted mix, and the per-slot
ctx_for_slot construction.

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>
EOF
)"
```

---

## Task 7: Calibration probes for BinPhysics fields

**Goal:** every BinPhysics field is exposed to the calibration probe system so its evolution can be regression-tested.

**Files:**
- Modify: `src/dsp/modules/mod.rs` (`ProbeSnapshot` already exists; add fields)
- Modify: `tests/calibration.rs` (add round-trip probes)

- [ ] **Step 7.1: Extend `ProbeSnapshot`**

In `src/dsp/modules/mod.rs:87-109`, add to `ProbeSnapshot`:

```rust
// BinPhysics probe values — sampled at bin 100 (a non-edge mid-band bin).
pub bp_mass:            Option<f32>,
pub bp_temperature:     Option<f32>,
pub bp_flux:            Option<f32>,
pub bp_crystallization: Option<f32>,
pub bp_phase_momentum:  Option<f32>,
pub bp_slew:            Option<f32>,
pub bp_bias:            Option<f32>,
```

- [ ] **Step 7.2: Wire probes in mock writer for the test path**

Modules that *write* BinPhysics will populate these probe fields under `#[cfg(any(test, feature = "probe"))]`. Writer modules ship in Phase 5; for now, the mock writer from Task 6 is updated to populate them:

```rust
#[cfg(any(test, feature = "probe"))]
{
    self.last_probe.bp_mass = Some(p.mass[100]);
    // …other fields as appropriate…
}
```

(Skip if MockWriter is purely test-internal and no real module writes physics yet — Phase 5 will add the real probe wiring per-module. Track this as a TODO in Phase 5.)

- [ ] **Step 7.3: Add a calibration round-trip stub**

Add to `tests/calibration.rs`:

```rust
#[test]
fn bin_physics_round_trip_stub() {
    // Phase 5 modules (Life, Kinetics) will fill this in:
    // 1. Set the relevant curve to a known value.
    // 2. Process one block.
    // 3. Read back ProbeSnapshot.bp_mass / bp_temperature etc.
    // 4. Assert the value matches the curve→physical mapping.
    // For Phase 3 we ship the probe slots only.
    eprintln!("Phase 3 ships probe field shapes; Phase 5 fills in the round-trip.");
}
```

- [ ] **Step 7.4: Run tests**

Run: `cargo test`
Expected: PASS (the stub just prints).

- [ ] **Step 7.5: Commit**

```bash
git add src/dsp/modules/mod.rs tests/calibration.rs
git commit -m "$(cat <<'EOF'
infra(phase3): add BinPhysics probe slots for calibration

Adds bp_mass/bp_temperature/bp_flux/bp_crystallization/bp_phase_momentum/
bp_slew/bp_bias to ProbeSnapshot. Round-trip wiring lands per-module
in Phase 5 when the first real writer module ships.

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>
EOF
)"
```

---

## Task 8: Status update

- [ ] **Step 8.1: Update STATUS.md**

```markdown
| `2026-04-27-phase-3-bin-physics` | IMPLEMENTED | Per-bin physics carrier with per-field merge rules. Supersedes 2026-04-21-bin-physics-infrastructure. |
| `2026-04-21-bin-physics-infrastructure` | SUPERSEDED | See `2026-04-27-phase-3-bin-physics.md`. |
```

- [ ] **Step 8.2: Update banner in original spec/plan**

Add at top of `docs/superpowers/specs/2026-04-21-bin-physics-infrastructure.md` and the matching plan:

```markdown
> **Status (2026-04-27): SUPERSEDED by `docs/superpowers/plans/2026-04-27-phase-3-bin-physics.md`.**
```

- [ ] **Step 8.3: Update roadmap banner**

In `ideas/next-gen-modules/99-implementation-roadmap.md`, under `## Phase 3 — BinPhysics infrastructure`:

```markdown
> **Status:** IMPLEMENTED (2026-04-27 → release `0.X+1.0`). See plan
> `docs/superpowers/plans/2026-04-27-phase-3-bin-physics.md`.
```

- [ ] **Step 8.4: Commit**

```bash
git add docs/superpowers/STATUS.md docs/superpowers/specs/2026-04-21-bin-physics-infrastructure.md docs/superpowers/plans/2026-04-21-bin-physics-infrastructure.md ideas/next-gen-modules/99-implementation-roadmap.md
git commit -m "docs(status): mark Phase 3 BinPhysics IMPLEMENTED, supersede 2026-04-21 plan

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

## Self-review checklist

- [ ] **Spec coverage:**
  - Audit § 1 four-new-fields → Task 1 ✓ (`slew`, `bias`, `decay_estimate`, `lock_target_freq` added)
  - Audit § 1 per-field merge rules → Task 1 ✓ (`MergeRule` enum)
  - Audit § 1a optional `&mut BinPhysics` in trait → Task 3 ✓
  - Audit § 1b `writes_bin_physics: bool` → Task 2 ✓
  - Audit § 7 "BinPhysics is the global kinetics function in disguise" → no code change; documented in Task 1 doc-comment.
  - Roadmap § Phase 3 PR 4 calibration probes → Task 7 ✓ (slots added; Phase 5 fills in)
- [ ] **Placeholder scan:** Task 6 step 6.3 has helper-method skeletons that need finalising during execution; mark each one in the diff. No hidden TBDs.
- [ ] **Type consistency:** `BinPhysics`, `MergeRule`, `bin_physics: Option<&'block BinPhysics>`, `physics: Option<&mut BinPhysics>`, `writes_bin_physics: bool` — naming uniform.
- [ ] **All `cargo test` passes** between every Task.

---

## Risk register (Phase 3)

| Risk | Mitigation |
|---|---|
| Velocity computed twice (once in BinPhysics, once in any module that re-derives it) | Document that `physics.velocity` is canonical; modules MUST NOT recompute. |
| Memory budget — 11 fields × 8193 bins × 4 bytes × 9 slots = ~3.2 MB plus 2 channel-arrays = ~6.5 MB | Already under the budget noted in audit § 7; FxMatrix lazy-skips when `bin_physics_in_use == false`. |
| Topological sort introduces a hot-path branch | Pre-computed at slot-assignment time; per-block cost is one `if`. |
| Modules forget to update `prev_mags` ordering | Centralized in FxMatrix; modules cannot reach `prev_mags`. |

## Execution handoff

Phase 3 lands cleanly between Phase 2 (no BinPhysics) and Phase 5 (heavy physics modules). It's pure infra — no audible change. The integration test in Task 6 is the proof-of-life. Tasks 1, 2, 3, 4, 5 are sequential. Tasks 6, 7, 8 finalise.
