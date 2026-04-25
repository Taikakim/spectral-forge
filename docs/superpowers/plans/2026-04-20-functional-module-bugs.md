> **Status (2026-04-24): IMPLEMENTED.** Contrast / T/S Split / M/S Split / Freeze / silent-master fixes all merged. Source of truth: the code + [../STATUS.md](../STATUS.md).

# Functional Module Bugs — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix the root cause of all "not functional" module complaints and related routing bugs: Contrast, T/S Split, M/S Split, Freeze left running, silent output when nothing routed to Master.

**Architecture:** The root cause of most "not functional" bugs is that `FxMatrix` is only populated from `slot_module_types` at `initialize()` time. Any module the user adds or removes via the UI after initialization is invisible to the audio thread — the old slot keeps processing (or doesn't exist yet). Fix: sync `FxMatrix.slots` from `params.slot_module_types` at the top of each `process()` call. Secondary fixes: Contrast ratio formula, M/S module stereo-mode gate removal, passthrough-when-nothing-routed-to-Master removal, and T/S Split virtual row routing.

**Tech Stack:** Rust, nih-plug, num-complex, assert_no_alloc

---

## File Map

| File | Change |
|------|--------|
| `Cargo.toml` | Add `assert_no_alloc = "1"` direct dep |
| `src/dsp/fx_matrix.rs` | Add `sync_slot_types()`, add virtual row mix-in, remove passthrough fallback, clear `virtual_out` per hop |
| `src/dsp/pipeline.rs` | Call `fx_matrix.sync_slot_types()` before STFT loop |
| `src/dsp/modules/mod.rs` | Add `virtual_outputs()` default method to `SpectralModule` trait |
| `src/dsp/modules/ts_split.rs` | Implement `virtual_outputs()` on `TsSplitModule` |
| `src/dsp/modules/contrast.rs` | Fix ratio formula: `amount.max(1.0)` not `1.0 + amount` |
| `src/dsp/modules/mid_side.rs` | Remove `stereo_link != StereoLink::MidSide` early return |
| `tests/engine_contract.rs` | Add tests for sync, passthrough, contrast neutral, M/S active |

---

### Task 1: Add assert_no_alloc and FxMatrix::sync_slot_types

Module creation allocates Vecs. The audio thread normally forbids allocation via `assert_process_allocs`. We use `permit_alloc()` to explicitly allow this one-time operation when the user changes a module type — it only runs on the hop after a UI change, not every block.

**Files:**
- Modify: `Cargo.toml`
- Modify: `src/dsp/fx_matrix.rs`

- [ ] **Step 1: Write the failing test**

Add to `tests/engine_contract.rs`:

```rust
#[test]
fn fx_matrix_sync_slot_types_activates_new_module() {
    use spectral_forge::dsp::{
        modules::{ModuleType, ModuleContext, RouteMatrix},
        fx_matrix::FxMatrix,
        pipeline::MAX_NUM_BINS,
    };
    use spectral_forge::params::{StereoLink, FxChannelTarget};
    use num_complex::Complex;

    let n = 1025usize;
    // Start with only Master in slot 8
    let mut types = [ModuleType::Empty; 9];
    types[8] = ModuleType::Master;
    let mut fm = FxMatrix::new(44100.0, 2048, &types);

    // Sync: add Dynamics to slot 0
    types[0] = ModuleType::Dynamics;
    fm.sync_slot_types(&types, 44100.0, 2048);

    // Slot 0 must now contain a module of type Dynamics
    assert!(fm.slots[0].is_some(), "slot 0 should have Dynamics after sync");
    assert_eq!(
        fm.slots[0].as_ref().unwrap().module_type(),
        ModuleType::Dynamics
    );

    // Sync: remove it
    types[0] = ModuleType::Empty;
    fm.sync_slot_types(&types, 44100.0, 2048);
    assert!(fm.slots[0].is_none(), "slot 0 should be None after sync to Empty");
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test fx_matrix_sync_slot_types_activates_new_module -- --nocapture
```

Expected: FAIL — `sync_slot_types` doesn't exist yet.

- [ ] **Step 3: Add assert_no_alloc to Cargo.toml**

In `Cargo.toml`, add after the existing `[dependencies]`:

```toml
assert_no_alloc = "1"
```

- [ ] **Step 4: Implement sync_slot_types in fx_matrix.rs**

Read `src/dsp/fx_matrix.rs` first. Then add this import at the top of the file:

```rust
use assert_no_alloc::permit_alloc;
```

Add the following method to the `impl FxMatrix` block, after `set_gain_modes`:

```rust
    /// Sync slot modules to the given type array. Called once per audio block.
    /// - Slot going to Empty: drops the existing module (dealloc only, fast).
    /// - Slot getting a new type: creates a module via permit_alloc (intentional
    ///   one-time allocation on user action; not per-sample).
    pub fn sync_slot_types(&mut self, types: &[ModuleType; 9], sample_rate: f32, fft_size: usize) {
        for s in 0..MAX_SLOTS {
            let current = self.slots[s].as_ref().map(|m| m.module_type())
                .unwrap_or(ModuleType::Empty);
            if current == types[s] { continue; }
            if types[s] == ModuleType::Empty {
                self.slots[s] = None;
            } else {
                let new_mod = permit_alloc(|| create_module(types[s], sample_rate, fft_size));
                self.slots[s] = Some(new_mod);
            }
        }
    }
```

- [ ] **Step 5: Run test to verify it passes**

```bash
cargo test fx_matrix_sync_slot_types_activates_new_module -- --nocapture
```

Expected: PASS

- [ ] **Step 6: Wire sync_slot_types into Pipeline::process()**

In `src/dsp/pipeline.rs`, inside `Pipeline::process()`, add the following block **immediately before** the line `self.stft.process_overlap_add(buffer, OVERLAP, |channel, block| {`:

```rust
        // Sync module types from params (non-blocking; skipped if GUI holds lock).
        // Handles add/remove of modules at runtime after initialize().
        if let Some(types) = params.slot_module_types.try_lock() {
            self.fx_matrix.sync_slot_types(&*types, self.sample_rate, self.fft_size);
        }
```

- [ ] **Step 7: Build**

```bash
cargo build 2>&1 | head -40
```

Expected: no errors.

- [ ] **Step 8: Run all tests**

```bash
cargo test
```

Expected: all 14+ tests pass.

- [ ] **Step 9: Commit**

```bash
git add Cargo.toml src/dsp/fx_matrix.rs src/dsp/pipeline.rs tests/engine_contract.rs
git commit -m "fix: sync FxMatrix slot modules from params each block — root cause of module-not-functional bugs"
```

---

### Task 2: Remove no-route passthrough (silence when nothing reaches Master)

Currently when no route has a send to Master (`any_to_master = false`), `FxMatrix::process_hop` falls back to copying the last occupied slot's output to `complex_buf`. This causes audio to pass through even when the user has not wired anything to the Master output bus. The fix: when nothing routes to Master, output silence (leave `mix_buf` zeroed).

**Files:**
- Modify: `src/dsp/fx_matrix.rs`

- [ ] **Step 1: Write the failing test**

Add to `tests/engine_contract.rs`:

```rust
#[test]
fn fx_matrix_no_route_to_master_produces_silence() {
    use spectral_forge::dsp::{
        modules::{ModuleType, ModuleContext, RouteMatrix},
        fx_matrix::FxMatrix,
        pipeline::MAX_NUM_BINS,
    };
    use spectral_forge::params::{StereoLink, FxChannelTarget};
    use num_complex::Complex;

    let n = 1025usize;
    let mut types = [ModuleType::Empty; 9];
    types[0] = ModuleType::Dynamics;
    types[8] = ModuleType::Master;
    let mut fm = FxMatrix::new(44100.0, 2048, &types);

    // Use a route matrix with NO send to Master (slot 8)
    let mut rm = RouteMatrix::default();
    rm.send[0][1] = 1.0;
    rm.send[1][2] = 1.0;
    rm.send[2][8] = 0.0;   // explicitly clear the default route to Master

    let mut bins: Vec<Complex<f32>> = vec![Complex::new(1.0, 0.0); n];
    let curves: Vec<Vec<Vec<f32>>> = (0..9)
        .map(|_| (0..7).map(|_| vec![1.0f32; MAX_NUM_BINS]).collect())
        .collect();
    let mut supp = vec![0.0f32; n];
    let sc: [Option<&[f32]>; 9] = [None; 9];
    let targets = [FxChannelTarget::All; 9];
    let ctx = ModuleContext {
        sample_rate: 44100.0, fft_size: 2048, num_bins: n,
        attack_ms: 10.0, release_ms: 100.0, sensitivity: 0.0,
        suppression_width: 0.0, auto_makeup: false, delta_monitor: false,
    };
    fm.process_hop(0, StereoLink::Linked, &mut bins, &sc, &targets, &curves, &rm, &ctx, &mut supp, n);

    // All bins should be zero when nothing routes to Master
    for (k, b) in bins.iter().enumerate() {
        assert!(
            b.norm() < 1e-6,
            "bin {k} should be silent when nothing routes to Master, got {}", b.norm()
        );
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test fx_matrix_no_route_to_master_produces_silence -- --nocapture
```

Expected: FAIL — currently the fallback copies last slot output instead of silence.

- [ ] **Step 3: Remove the fallback in FxMatrix::process_hop**

In `src/dsp/fx_matrix.rs`, find the Master output section. It currently reads:

```rust
        // Master output: accumulate sends to slot 8.
        self.mix_buf[..num_bins].fill(Complex::new(0.0, 0.0));
        let any_to_master = (0..8).any(|src| route_matrix.send[src][8] > 0.001);
        if any_to_master {
            for src in 0..8 {
                let send = route_matrix.send[src][8];
                if send < 0.001 { continue; }
                for k in 0..num_bins {
                    self.mix_buf[k] += self.slot_out[src][k] * send;
                }
            }
        } else {
            // Fallback: last populated slot's output goes to Master.
            // If all slots 0-7 are empty, mix_buf stays zeroed → silence.
            for src in (0..8).rev() {
                if self.slots[src].is_some() {
                    self.mix_buf[..num_bins].copy_from_slice(&self.slot_out[src][..num_bins]);
                    break;
                }
            }
        }
```

Replace with:

```rust
        // Master output: accumulate sends to slot 8.
        // If nothing routes to Master, mix_buf stays zeroed → silence.
        self.mix_buf[..num_bins].fill(Complex::new(0.0, 0.0));
        for src in 0..8 {
            let send = route_matrix.send[src][8];
            if send < 0.001 { continue; }
            for k in 0..num_bins {
                self.mix_buf[k] += self.slot_out[src][k] * send;
            }
        }
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo test fx_matrix_no_route_to_master_produces_silence -- --nocapture
```

Expected: PASS

- [ ] **Step 5: Verify existing routing test still passes**

```bash
cargo test matrix_routing_serial_default_passes_signal -- --nocapture
```

Expected: PASS (the default route matrix has `send[2][8] = 1.0`, so signal still flows).

- [ ] **Step 6: Run all tests**

```bash
cargo test
```

Expected: all tests pass.

- [ ] **Step 7: Commit**

```bash
git add src/dsp/fx_matrix.rs tests/engine_contract.rs
git commit -m "fix: silence when nothing routes to Master — remove implicit passthrough fallback"
```

---

### Task 3: Fix Contrast module ratio formula

The Contrast module maps the AMOUNT curve via `ratio = (1.0 + amount)`. At a neutral curve (all nodes at y=0, linear gain = 1.0), this gives ratio = 2.0 — contrast is already actively expanding the spectrum. The correct formula is `ratio = amount.max(1.0)`: at gain=1.0 → ratio=1.0 (no effect). The user raises the AMOUNT curve to add contrast.

**Files:**
- Modify: `src/dsp/modules/contrast.rs`
- Modify: `tests/engine_contract.rs`

- [ ] **Step 1: Write the failing test**

Add to `tests/engine_contract.rs`. At neutral AMOUNT (all gains=1.0), the Contrast module should route to the contrast engine with ratio=1.0, which means no spectral effect on a flat input after convergence. We test at the module level by verifying the flat-spectrum flat-output property.

```rust
#[test]
fn contrast_module_neutral_curve_passes_flat_spectrum() {
    use spectral_forge::dsp::{
        modules::{create_module, ModuleType, ModuleContext, SpectralModule},
        pipeline::MAX_NUM_BINS,
    };
    use spectral_forge::params::{StereoLink, FxChannelTarget};
    use num_complex::Complex;

    let n = 1025usize;
    let mut m = create_module(ModuleType::Contrast, 44100.0, 2048);

    let ones = vec![1.0f32; n];
    let curves_storage: [&[f32]; 2] = [&ones, &ones];
    let curves: &[&[f32]] = &curves_storage;

    // Flat spectrum with uniform magnitude
    let input_mag = 128.0f32;
    let mut bins = vec![Complex::new(input_mag, 0.0); n];
    let mut supp = vec![0.0f32; n];
    let ctx = ModuleContext {
        sample_rate: 44100.0, fft_size: 2048, num_bins: n,
        attack_ms: 10.0, release_ms: 100.0, sensitivity: 0.0,
        suppression_width: 4.0, auto_makeup: false, delta_monitor: false,
    };

    // Converge the contrast envelope
    for _ in 0..500 {
        let mut b = vec![Complex::new(input_mag, 0.0); n];
        m.process(0, StereoLink::Linked, FxChannelTarget::All, &mut b, None, curves, &mut supp, &ctx);
    }
    let mut final_bins = vec![Complex::new(input_mag, 0.0); n];
    m.process(0, StereoLink::Linked, FxChannelTarget::All, &mut final_bins, None, curves, &mut supp, &ctx);

    // With neutral AMOUNT (ratio=1.0) and a flat spectrum, output should match input.
    for b in &final_bins {
        assert!(
            (b.norm() - input_mag).abs() < input_mag * 0.01,
            "contrast neutral curve on flat spectrum should pass through, got {:.3} vs {:.3}",
            b.norm(), input_mag
        );
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test contrast_module_neutral_curve_passes_flat_spectrum -- --nocapture
```

Expected: FAIL — currently ratio=2.0 at neutral makes the contrast engine expand, modifying the output even with a flat spectrum.

- [ ] **Step 3: Fix the ratio formula in contrast.rs**

In `src/dsp/modules/contrast.rs`, inside `ContrastModule::process()`, find:

```rust
            self.bp_ratio[k]     = (1.0 + amount).max(0.0).clamp(0.0, 20.0);
```

Replace with:

```rust
            self.bp_ratio[k]     = amount.max(1.0).min(20.0);
```

Explanation: `amount` is the AMOUNT curve's linear gain per bin. At y=0 (neutral), gain=1.0 → ratio=1.0 = no effect. Raising the curve above 0 gives ratio>1.0 = expanding contrast. The `.max(1.0)` prevents ratio<1 (which would mean "contrast compression" — a valid concept but not what this module is labelled to do). `.min(20.0)` preserves the 20:1 ceiling.

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo test contrast_module_neutral_curve_passes_flat_spectrum -- --nocapture
```

Expected: PASS

- [ ] **Step 5: Run all tests**

```bash
cargo test
```

Expected: all tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/dsp/modules/contrast.rs tests/engine_contract.rs
git commit -m "fix: Contrast module — neutral AMOUNT curve now maps to ratio=1.0 (no effect)"
```

---

### Task 4: M/S module processes in all stereo modes

`MidSideModule::process()` returns early when `stereo_link != StereoLink::MidSide`, making the module a no-op in the default Linked stereo mode. The fix: always apply balance, expansion, and decorrelation processing. In Linked mode, channel 0 receives "mid-style" treatment and channel 1 receives "side-style" treatment. This is semantically cleanest when the plugin is in MidSide mode (channels are already M/S encoded by the pipeline), but it does something useful even in Linked mode.

**Files:**
- Modify: `src/dsp/modules/mid_side.rs`
- Modify: `tests/engine_contract.rs`

- [ ] **Step 1: Write the failing test**

Add to `tests/engine_contract.rs`:

```rust
#[test]
fn mid_side_module_processes_in_linked_mode() {
    use spectral_forge::dsp::modules::{
        create_module, ModuleType, ModuleContext, SpectralModule,
    };
    use spectral_forge::params::{StereoLink, FxChannelTarget};
    use num_complex::Complex;

    let n = 1025usize;
    let mut m = create_module(ModuleType::MidSide, 44100.0, 2048);

    let ones = vec![1.0f32; n];
    // Balance=0.5 (cut mid to 0, boost side) should change the output in Linked mode
    let half = vec![0.5f32; n];
    let zeros = vec![0.0f32; n];
    let curves_storage: [&[f32]; 5] = [&half, &ones, &zeros, &ones, &ones];
    let curves: &[&[f32]] = &curves_storage;

    let mut bins = vec![Complex::new(1.0f32, 0.0); n];
    let mut supp = vec![0.0f32; n];
    let ctx = ModuleContext {
        sample_rate: 44100.0, fft_size: 2048, num_bins: n,
        attack_ms: 10.0, release_ms: 100.0, sensitivity: 0.0,
        suppression_width: 0.0, auto_makeup: false, delta_monitor: false,
    };

    // Channel 0 in Linked mode: balance=0.5 → mid_scale = sqrt(0.5) ≈ 0.707 → bins reduced
    m.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, None, curves, &mut supp, &ctx);
    let out_mag = bins[10].norm();
    assert!(
        out_mag < 0.95,
        "M/S module with balance=0.5 should reduce channel 0 in Linked mode, got {:.4}", out_mag
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test mid_side_module_processes_in_linked_mode -- --nocapture
```

Expected: FAIL — currently the module returns early when `stereo_link == Linked`.

- [ ] **Step 3: Remove the stereo_link guard**

In `src/dsp/modules/mid_side.rs`, inside `MidSideModule::process()`, find and remove:

```rust
        // Only active in MidSide mode
        if stereo_link != StereoLink::MidSide {
            return;
        }
```

(Remove those 3 lines entirely. The rest of the function already handles channels 0 and 1 correctly.)

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo test mid_side_module_processes_in_linked_mode -- --nocapture
```

Expected: PASS

- [ ] **Step 5: Verify the neutral pass-through test still passes**

```bash
cargo test mid_side_module_compiles_and_passes_through_at_neutral -- --nocapture
```

Expected: PASS (the neutral test uses `StereoLink::MidSide` with balance=1.0, which still passes through).

- [ ] **Step 6: Run all tests**

```bash
cargo test
```

Expected: all tests pass.

- [ ] **Step 7: Commit**

```bash
git add src/dsp/modules/mid_side.rs tests/engine_contract.rs
git commit -m "fix: MidSide module processes in all stereo modes, not only MidSide"
```

---

### Task 5: T/S Split virtual row routing

The `TsSplitModule` already classifies bins into transient and sustained internal buffers on every `process()` call. But `FxMatrix::process_hop()` never reads those buffers — the virtual row infrastructure exists in the `RouteMatrix` and `FxMatrix.virtual_out` allocation but is entirely unimplemented. This task wires everything together.

**Approach:**
1. Add `virtual_outputs() -> Option<[&[Complex<f32>]; 2]>` to the `SpectralModule` trait (default: `None`).
2. `TsSplitModule` overrides it to return `[transient_out, sustained_out]`.
3. In `FxMatrix::process_hop`: clear `virtual_out` at the start, then after each slot processes, if the module has virtual outputs, copy them to the matching `virtual_out[v]` slots (according to `route_matrix.virtual_rows`).
4. In the mix-building step for each slot, also accumulate weighted sums from virtual rows.

**Files:**
- Modify: `src/dsp/modules/mod.rs`
- Modify: `src/dsp/modules/ts_split.rs`
- Modify: `src/dsp/fx_matrix.rs`
- Modify: `tests/engine_contract.rs`

- [ ] **Step 1: Write the failing test**

Add to `tests/engine_contract.rs`:

```rust
#[test]
fn ts_split_virtual_outputs_populated_after_process() {
    use spectral_forge::dsp::modules::{
        create_module, ModuleType, ModuleContext, SpectralModule, VirtualRowKind,
    };
    use spectral_forge::params::{StereoLink, FxChannelTarget};
    use num_complex::Complex;

    let n = 1025usize;
    let mut m = create_module(ModuleType::TransientSustainedSplit, 44100.0, 2048);

    // Before process: virtual_outputs() should return Some
    assert!(m.virtual_outputs().is_some(), "TsSplitModule must expose virtual_outputs()");

    let ones = vec![1.0f32; n];
    let curves_storage: [&[f32]; 1] = [&ones];
    let curves: &[&[f32]] = &curves_storage;
    let mut bins = vec![Complex::new(1.0f32, 0.0); n];
    let mut supp = vec![0.0f32; n];
    let ctx = ModuleContext {
        sample_rate: 44100.0, fft_size: 2048, num_bins: n,
        attack_ms: 10.0, release_ms: 100.0, sensitivity: 0.0,
        suppression_width: 0.0, auto_makeup: false, delta_monitor: false,
    };
    m.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, None, curves, &mut supp, &ctx);

    let vouts = m.virtual_outputs().unwrap();
    // After first process: transient + sustained together must sum to roughly the input energy
    let total_energy: f32 = (0..n).map(|k| {
        (vouts[0][k].norm() + vouts[1][k].norm())
    }).sum();
    // Input was n bins at magnitude 1.0; total energy summed should be non-zero
    assert!(total_energy > 1.0, "T/S split should distribute input energy, got {}", total_energy);
}

#[test]
fn fx_matrix_ts_split_routes_transient_to_next_slot() {
    use spectral_forge::dsp::{
        modules::{ModuleType, ModuleContext, RouteMatrix, VirtualRowKind, MAX_SLOTS},
        fx_matrix::FxMatrix,
        pipeline::MAX_NUM_BINS,
    };
    use spectral_forge::params::{StereoLink, FxChannelTarget};
    use num_complex::Complex;

    let n = 1025usize;
    // Slot 0 = T/S Split, Slot 1 = Gain (passes through), Slot 8 = Master
    let mut types = [ModuleType::Empty; 9];
    types[0] = ModuleType::TransientSustainedSplit;
    types[1] = ModuleType::Gain;
    types[8] = ModuleType::Master;
    let mut fm = FxMatrix::new(44100.0, 2048, &types);

    // Route: virtual row 0 (transient of slot 0) → slot 1 → Master
    let mut rm = RouteMatrix::default();
    rm.send = [[0.0f32; 9]; 13];             // clear all
    rm.send[MAX_SLOTS + 0][1] = 1.0;         // virtual row 0 → slot 1
    rm.send[1][8] = 1.0;                     // slot 1 → Master
    rm.virtual_rows[0] = Some((0, VirtualRowKind::Transient)); // row 0 = transient of slot 0

    // Steady-state input with one loud bin at 512 (transient candidate after convergence)
    let floor_mag = 0.1f32;
    let peak_mag  = 10.0f32;
    let mut bins: Vec<Complex<f32>> = vec![Complex::new(floor_mag, 0.0); n];
    bins[512] = Complex::new(peak_mag, 0.0);

    let curves: Vec<Vec<Vec<f32>>> = (0..9)
        .map(|_| (0..7).map(|_| vec![1.0f32; MAX_NUM_BINS]).collect())
        .collect();
    let mut supp = vec![0.0f32; n];
    let sc: [Option<&[f32]>; 9] = [None; 9];
    let targets = [FxChannelTarget::All; 9];
    let ctx = ModuleContext {
        sample_rate: 44100.0, fft_size: 2048, num_bins: n,
        attack_ms: 10.0, release_ms: 100.0, sensitivity: 0.0,
        suppression_width: 0.0, auto_makeup: false, delta_monitor: false,
    };

    // Converge the T/S split avg_mag tracker
    for _ in 0..200 {
        let mut b = bins.clone();
        fm.process_hop(0, StereoLink::Linked, &mut b, &sc, &targets, &curves, &rm, &ctx, &mut supp, n);
    }
    let mut final_bins = bins.clone();
    fm.process_hop(0, StereoLink::Linked, &mut final_bins, &sc, &targets, &curves, &rm, &ctx, &mut supp, n);

    // After routing transient → slot1 → Master, the output must be finite and non-zero
    assert!(final_bins.iter().any(|b| b.norm() > 1e-6),
        "T/S Split transient route should produce non-zero output at Master");
    for (k, b) in final_bins.iter().enumerate() {
        assert!(b.re.is_finite() && b.im.is_finite(), "bin {} is not finite", k);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test ts_split_virtual_outputs_populated_after_process fx_matrix_ts_split_routes_transient_to_next_slot -- --nocapture
```

Expected: FAIL — `virtual_outputs()` doesn't exist on the trait yet.

- [ ] **Step 3: Add virtual_outputs() to SpectralModule trait**

In `src/dsp/modules/mod.rs`, inside the `SpectralModule` trait, add after `fn num_outputs()`:

```rust
    /// For split modules (T/S Split), returns a fixed-size array of output bin buffers.
    /// Index 0 = first virtual output (Transient), index 1 = second (Sustained).
    /// Default implementation returns None (no virtual outputs).
    fn virtual_outputs(&self) -> Option<[&[Complex<f32>]; 2]> { None }
```

- [ ] **Step 4: Implement virtual_outputs() on TsSplitModule**

In `src/dsp/modules/ts_split.rs`, add the following inside `impl SpectralModule for TsSplitModule`:

```rust
    fn virtual_outputs(&self) -> Option<[&[Complex<f32>]; 2]> {
        Some([&self.transient_out, &self.sustained_out])
    }
```

- [ ] **Step 5: Run the first test to verify it partially passes**

```bash
cargo test ts_split_virtual_outputs_populated_after_process -- --nocapture
```

Expected: PASS (the module now exposes virtual outputs; this test doesn't need FxMatrix routing).

- [ ] **Step 6: Implement virtual row routing in FxMatrix::process_hop**

In `src/dsp/fx_matrix.rs`, make the following changes:

**6a.** Add at the very start of `process_hop`, after `for s in 0..8 {` ... no wait, add **before** the main slot loop. Find the line `for s in 0..8 {` and insert before it:

```rust
        // Clear virtual row output buffers for this hop.
        for v in 0..MAX_SPLIT_VIRTUAL_ROWS {
            self.virtual_out[v][..num_bins].fill(Complex::new(0.0, 0.0));
        }
```

**6b.** In the main slot loop, after the line:
```rust
            self.slot_out[s][..num_bins].copy_from_slice(&self.mix_buf[..num_bins]);
```
and before:
```rust
            self.slots[s] = Some(module);
```

Add:

```rust
            // Populate virtual row buffers from split modules.
            if let Some(vouts) = module.virtual_outputs() {
                for (v, &vrow) in route_matrix.virtual_rows.iter().enumerate() {
                    if let Some((src_slot, kind)) = vrow {
                        if src_slot as usize == s {
                            let src_buf = match kind {
                                VirtualRowKind::Transient => vouts[0],
                                VirtualRowKind::Sustained  => vouts[1],
                            };
                            let copy_len = num_bins.min(src_buf.len());
                            self.virtual_out[v][..copy_len].copy_from_slice(&src_buf[..copy_len]);
                        }
                    }
                }
            }
```

**6c.** In the mix-building part of the main slot loop, after:
```rust
            for src in 0..s {
                let send = route_matrix.send[src][s];
                if send < 0.001 { continue; }
                for k in 0..num_bins {
                    self.mix_buf[k] += self.slot_out[src][k] * send;
                }
            }
```

Add:

```rust
            // Also accumulate from virtual rows (e.g. T/S Split transient/sustained outputs).
            // Only include virtual rows whose source slot has already been processed (< s).
            for (v, &vrow) in route_matrix.virtual_rows.iter().enumerate() {
                if let Some((src_slot, _kind)) = vrow {
                    if (src_slot as usize) < s {
                        let send = route_matrix.send[MAX_SLOTS + v][s];
                        if send < 0.001 { continue; }
                        for k in 0..num_bins {
                            self.mix_buf[k] += self.virtual_out[v][k] * send;
                        }
                    }
                }
            }
```

Note: `MAX_SLOTS` is already imported from the modules crate in `fx_matrix.rs`.

- [ ] **Step 7: Run both new tests**

```bash
cargo test ts_split_virtual_outputs_populated_after_process fx_matrix_ts_split_routes_transient_to_next_slot -- --nocapture
```

Expected: both PASS

- [ ] **Step 8: Run all tests**

```bash
cargo test
```

Expected: all tests pass.

- [ ] **Step 9: Commit**

```bash
git add src/dsp/modules/mod.rs src/dsp/modules/ts_split.rs src/dsp/fx_matrix.rs tests/engine_contract.rs
git commit -m "feat: T/S Split virtual row routing — transient and sustained outputs now route through FxMatrix"
```

---

## Verification

Run the full test suite and build release:

```bash
cargo test && cargo build --release 2>&1 | tail -5
```

Expected: `14+` tests pass, release build succeeds with no errors.

Then rebuild the plugin bundle:

```bash
cargo run --package xtask -- bundle spectral_forge --release
cp target/bundled/spectral_forge.clap ~/.clap/
```

In Bitwig, rescan and test:
- Insert Contrast module: move the AMOUNT curve up — contrast effect should be audible.
- Insert M/S Split: set Balance curve to 0 for channel 0 — channel 0 should be silenced.
- Insert T/S Split: route transient output to one slot and sustained to another — separate processing should be audible.
- Remove a Freeze module while playing — freeze should stop immediately on the next hop.
- Clear all routes to Master in the matrix — output should be silence.
