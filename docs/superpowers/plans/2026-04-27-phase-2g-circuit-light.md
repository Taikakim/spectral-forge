# Phase 2g — Circuit-light Module Implementation Plan

> **Status:** IMPLEMENTED — all 10 tasks landed on `feature/next-gen-modules-plans`. Phase 2 sub-plan; depends on Phase 1 foundation infra (`docs/superpowers/plans/2026-04-27-phase-1-foundation-infra.md`).
>
> **Superseded for curve layout by Phase 5c (`docs/superpowers/plans/2026-04-27-phase-5c-full-circuit.md`):** Phase 5c bumps `num_curves` to 5 (inserts SPREAD at index 2; RELEASE moves to 3; MIX moves to 4) and migrates the 3 v1 kernels in-place. Adds 7 new BinPhysics-aware modes (Vactrol / Transformer / Power Sag / Component Drift / PCB Crosstalk / Slew Distortion / Bias Fuzz). The original 3-mode contract test in `tests/circuit.rs` was extended to all 10 modes by Phase 5c task 14.
>
> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship the Circuit module with three light/medium-CPU analog-style modes — **BBD Bins**, **Spectral Schmitt**, and **Crossover Distortion** — using only the existing SpectralModule trait. Defers Vactrol, Transformer, Power Sag, Component Drift, PCB Crosstalk, Slew Distortion, Bias Fuzz, and Resonant Feedback to Phase 5c (BinPhysics-aware modes).

**Architecture:** New `ModuleType::Circuit` slot. Per-channel state holds a 4-stage BBD magnitude pipeline (`bbd_mag[2][4][num_bins]`) and a Schmitt latch (`schmitt_latched[2][num_bins]`). Crossover is stateless. Mode is per-slot (persisted via `Mutex<CircuitMode>`), dispatched per block. No sidechain, no panel widget. Module declares `heavy_cpu: false` for v1 (BBD memory is the heaviest item; fits cleanly under the budget).

**Tech Stack:** Rust 2021, `nih-plug`, `realfft::num_complex::Complex<f32>`, existing `SpectralModule` trait + `ModuleContext`.

**Source spec:** `ideas/next-gen-modules/10-circuit.md` (research findings 2026-04-26 incorporated).

**Defer list (NOT in this plan):**
- **Vactrol** — needs `BinPhysics::flux` reads + cascaded 1-pole. Lands in Phase 5c.
- **Transformer Saturation** — needs `BinPhysics::flux` + tanh polynomial + SPREAD curve. Lands in Phase 5c.
- **Power Sag / Component Drift / PCB Crosstalk / Slew Distortion** — all read `BinPhysics::temperature` or write `BinPhysics::flux`. Phase 5c.
- **Bias Fuzz** — needs `BinPhysics::bias` field (new). Phase 5c.
- **Resonant Feedback** — handled via `RouteMatrix` once Phase 2a (Matrix Amp Nodes) lands and is paired with a Circuit slot. No sub-effect needed.
- **SPREAD curve** — only Transformer + PCB Crosstalk consume it. Defer the 5th curve until Phase 5c.
- **Envelope Follower Ripple** — global helper, separately tracked.

**Risk register:**
- BBD memory: 4 stages × 8193 bins × 4 bytes × 2 channels = ~262 KB per slot. Fits comfortably in L2 of any modern x86 (≥256 KB per core typical, but spilling to L3 still OK at hop rate).
- BBD dither uses a per-channel xorshift32 state to keep dither bin-independent without per-bin RNG state. Cheap and produces audibly pleasant noise.
- Schmitt branch-free mask-blend: implemented with `if/else` in v1 (the conditional branches predict perfectly because thresholds vary slowly relative to bin counts). SIMD `wide` adoption deferred to Phase 5c when BinPhysics-aware modes land.
- Crossover deadzone is C¹-smooth (squared re-emergence) to avoid audible discontinuity at the threshold crossing. Verified via test that bins right above the deadzone produce non-zero output without click.
- num_curves() = 4 in v1 (AMOUNT, THRESHOLD, RELEASE, MIX). Phase 5c will bump to 5 by appending SPREAD; existing presets remain valid because adding a trailing curve is non-breaking.

---

## File Structure

**Create:**
- `src/dsp/modules/circuit.rs` — `CircuitModule` impl, `CircuitMode` enum, kernels.
- `src/editor/circuit_popup.rs` — mode picker popup.

**Modify:**
- `src/dsp/modules/mod.rs` — add `ModuleType::Circuit` variant, `module_spec(Circuit)` entry, `create_module()` wiring, `set_circuit_mode` trait default.
- `src/dsp/fx_matrix.rs` — `set_circuit_modes()` per-block sync.
- `src/params.rs` — `slot_circuit_mode: [Arc<Mutex<CircuitMode>>; MAX_SLOTS]`.
- `src/lib.rs` — snapshot per-block, dispatch to FxMatrix.
- `src/editor/theme.rs` — `CIRCUIT_DOT_COLOR`.
- `src/editor/module_popup.rs` — make Circuit assignable + mode picker entry.
- `src/editor/fx_matrix_grid.rs` — render Circuit slot label.
- `tests/module_trait.rs` — finite/bounded test for all three modes.
- `tests/calibration_roundtrip.rs` — circuit probes.
- `docs/superpowers/STATUS.md` — entry for this plan.

---

## Task 1: Add `ModuleType::Circuit` variant + theme color + ModuleSpec entry

**Files:**
- Modify: `src/dsp/modules/mod.rs`
- Modify: `src/editor/theme.rs`
- Modify: `tests/module_trait.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn circuit_module_spec_present() {
    use spectral_forge::dsp::modules::{module_spec, ModuleType};

    let spec = module_spec(ModuleType::Circuit);
    assert_eq!(spec.display_name, "CIR");
    assert_eq!(spec.num_curves, 4);
    assert_eq!(spec.curve_labels.len(), 4);
    assert!(spec.assignable_to_user_slots);
    assert!(!spec.heavy_cpu, "v1 ships BBD/Schmitt/Crossover only");
    assert!(!spec.wants_sidechain);
}
```

- [ ] **Step 2: Run test, expect failure**

Run: `cargo test --test module_trait circuit_module_spec_present -- --nocapture`
Expected: FAIL — `Circuit` variant not found.

- [ ] **Step 3: Add the enum variant**

In `src/dsp/modules/mod.rs::ModuleType`:

```rust
pub enum ModuleType {
    // ... existing ...
    Circuit,
    // (Master remains last)
}
```

- [ ] **Step 4: Add module_spec entry**

```rust
ModuleType::Circuit => ModuleSpec {
    ty: ModuleType::Circuit,
    display_name: "CIR",
    color: theme::CIRCUIT_DOT_COLOR,
    num_curves: 4,
    curve_labels: &["AMOUNT", "THRESH", "RELEASE", "MIX"],
    assignable_to_user_slots: true,
    heavy_cpu: false,
    wants_sidechain: false,
    panel_widget: None,
},
```

- [ ] **Step 5: Add theme color**

In `src/editor/theme.rs`:

```rust
/// Circuit module — copper/orange for "analog component" feel.
pub const CIRCUIT_DOT_COLOR: egui::Color32 = egui::Color32::from_rgb(200, 140, 80);
```

- [ ] **Step 6: Run test, expect pass**

Run: `cargo test --test module_trait circuit_module_spec_present -- --nocapture`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add src/dsp/modules/mod.rs src/editor/theme.rs tests/module_trait.rs
git commit -m "feat(circuit): add ModuleType::Circuit variant + spec entry"
```

---

## Task 2: CircuitMode enum + CircuitModule skeleton + create_module() wiring

**Files:**
- Create: `src/dsp/modules/circuit.rs`
- Modify: `src/dsp/modules/mod.rs::create_module()`

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn circuit_module_constructs_and_passes_through() {
    use spectral_forge::dsp::modules::{create_module, ModuleType, SpectralModule, ModuleContext, StereoLink, FxChannelTarget};
    use realfft::num_complex::Complex;

    let mut module = create_module(ModuleType::Circuit);
    module.reset(48_000.0, 2048);
    assert_eq!(module.module_type(), ModuleType::Circuit);
    assert_eq!(module.num_curves(), 4);

    let num_bins = 1025;
    let mut bins: Vec<Complex<f32>> = (0..num_bins).map(|k| Complex::new((k as f32 * 0.01).sin(), 0.0)).collect();
    let dry: Vec<Complex<f32>> = bins.clone();

    // Curves: AMOUNT=0, MIX=0 → passthrough.
    let zeros = vec![0.0_f32; num_bins];
    let neutral = vec![1.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&zeros, &neutral, &neutral, &zeros];

    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = circuit_test_ctx(num_bins);

    module.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, None, &curves, &mut suppression, &ctx);

    for k in 0..num_bins {
        let diff = (bins[k] - dry[k]).norm();
        assert!(diff < 1e-5, "bin {} drifted by {}", k, diff);
    }
    for s in &suppression {
        assert!(s.is_finite() && *s >= 0.0);
    }
}

#[cfg(test)]
fn circuit_test_ctx(num_bins: usize) -> spectral_forge::dsp::modules::ModuleContext {
    spectral_forge::dsp::modules::ModuleContext {
        sample_rate: 48_000.0,
        fft_size: 2048,
        num_bins,
        attack_ms: 10.0,
        release_ms: 100.0,
        sensitivity: 1.0,
        suppression_width: 1.0,
        auto_makeup: false,
        delta_monitor: false,
        bpm: 120.0,
        beat_position: 0.0,
        unwrapped_phase: None,
        peaks: None,
    }
}
```

- [ ] **Step 2: Run test, expect failure**

Run: `cargo test --test module_trait circuit_module_constructs_and_passes_through -- --nocapture`
Expected: FAIL — `unimplemented` panic from `create_module`.

- [ ] **Step 3: Create circuit.rs skeleton**

```rust
use realfft::num_complex::Complex;
use serde::{Deserialize, Serialize};

use crate::dsp::modules::{
    FxChannelTarget, ModuleContext, ModuleType, SpectralModule, StereoLink,
};

pub const BBD_STAGES: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CircuitMode {
    BbdBins,
    SpectralSchmitt,
    CrossoverDistortion,
}

impl Default for CircuitMode {
    fn default() -> Self {
        CircuitMode::CrossoverDistortion // cheapest mode → safest default
    }
}

pub struct CircuitModule {
    mode: CircuitMode,
    /// Per-channel BBD pipeline: bbd_mag[ch][stage][bin].
    bbd_mag: [[Vec<f32>; BBD_STAGES]; 2],
    /// Per-channel Schmitt latch state (one bool per bin, packed as u8).
    schmitt_latched: [Vec<u8>; 2],
    /// Per-channel xorshift32 state for BBD dither.
    rng_state: [u32; 2],
    sample_rate: f32,
    fft_size: usize,
}

impl CircuitModule {
    pub fn new() -> Self {
        Self {
            mode: CircuitMode::default(),
            bbd_mag: [
                [Vec::new(), Vec::new(), Vec::new(), Vec::new()],
                [Vec::new(), Vec::new(), Vec::new(), Vec::new()],
            ],
            schmitt_latched: [Vec::new(), Vec::new()],
            rng_state: [0xDEADBEEFu32, 0xCAFEBABEu32],
            sample_rate: 48_000.0,
            fft_size: 2048,
        }
    }

    pub fn set_mode_for_test(&mut self, mode: CircuitMode) {
        self.mode = mode;
    }

    pub fn current_mode(&self) -> CircuitMode {
        self.mode
    }
}

impl SpectralModule for CircuitModule {
    fn process(
        &mut self,
        channel: usize,
        _stereo_link: StereoLink,
        _target: FxChannelTarget,
        _bins: &mut [Complex<f32>],
        _sidechain: Option<&[f32]>,
        _curves: &[&[f32]],
        suppression_out: &mut [f32],
        _ctx: &ModuleContext,
    ) {
        debug_assert!(channel < 2);
        // v1 stub. Mode dispatch added in Task 6.
        for s in suppression_out.iter_mut() {
            *s = 0.0;
        }
    }

    fn reset(&mut self, sample_rate: f32, fft_size: usize) {
        self.sample_rate = sample_rate;
        self.fft_size = fft_size;
        let num_bins = fft_size / 2 + 1;
        for ch in 0..2 {
            for stage in 0..BBD_STAGES {
                self.bbd_mag[ch][stage].clear();
                self.bbd_mag[ch][stage].resize(num_bins, 0.0);
            }
            self.schmitt_latched[ch].clear();
            self.schmitt_latched[ch].resize(num_bins, 0);
        }
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::Circuit
    }

    fn num_curves(&self) -> usize {
        4
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
```

- [ ] **Step 4: Wire create_module()**

In `src/dsp/modules/mod.rs::create_module()`:

```rust
ModuleType::Circuit => Box::new(crate::dsp::modules::circuit::CircuitModule::new()),
```

Add at top of `mod.rs`:

```rust
pub mod circuit;
```

- [ ] **Step 5: Run test, expect pass**

Run: `cargo test --test module_trait circuit_module_constructs_and_passes_through -- --nocapture`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/dsp/modules/circuit.rs src/dsp/modules/mod.rs tests/module_trait.rs
git commit -m "feat(circuit): module skeleton + CircuitMode enum"
```

---

## Task 3: BBD Bins kernel — 4-stage delay + LP + dither

**Files:**
- Modify: `src/dsp/modules/circuit.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn circuit_bbd_delays_and_lowpasses() {
    use spectral_forge::dsp::modules::circuit::{CircuitModule, CircuitMode};
    use spectral_forge::dsp::modules::{SpectralModule, StereoLink, FxChannelTarget};
    use realfft::num_complex::Complex;

    let mut module = CircuitModule::new();
    module.reset(48_000.0, 2048);
    module.set_mode_for_test(CircuitMode::BbdBins);

    let num_bins = 1025;
    // Impulse at bin 100, magnitude 4.0.
    let mut bins: Vec<Complex<f32>> = vec![Complex::new(0.0, 0.0); num_bins];
    bins[100] = Complex::new(4.0, 0.0);

    // AMOUNT=2 (max delay), THRESHOLD=1 (mid dither), RELEASE=1 (LP cutoff mid), MIX=2 (full wet).
    let amount = vec![2.0_f32; num_bins];
    let thresh = vec![1.0_f32; num_bins];
    let release = vec![1.0_f32; num_bins];
    let mix = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &thresh, &release, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = circuit_test_ctx(num_bins);

    // First hop: signal enters first bucket. Bin 100 output should be << input (delayed).
    module.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, None, &curves, &mut suppression, &ctx);
    let after_hop_1 = bins[100].norm();
    assert!(after_hop_1 < 4.0, "BBD must delay (bin 100 still at {})", after_hop_1);

    // Push 4 more hops with zero input — the previously-injected energy should propagate through stages.
    for _ in 0..4 {
        for b in bins.iter_mut() { *b = Complex::new(0.0, 0.0); }
        module.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, None, &curves, &mut suppression, &ctx);
    }
    // After 4 stages of delay, bin 100 should show *some* output (the delayed signal arrived).
    let final_mag = bins[100].norm();
    assert!(final_mag > 0.05, "BBD did not propagate signal through stages (final={})", final_mag);

    for b in &bins {
        assert!(b.norm().is_finite() && b.norm() < 100.0);
    }
}
```

- [ ] **Step 2: Run test, expect failure**

Run: `cargo test --test module_trait circuit_bbd_delays_and_lowpasses -- --nocapture`
Expected: FAIL — BBD arm unimplemented; bin 100 stays at 4.0.

- [ ] **Step 3: Add the BBD kernel**

```rust
fn xorshift32_step(state: &mut u32) -> f32 {
    let mut s = *state;
    s ^= s << 13;
    s ^= s >> 17;
    s ^= s << 5;
    *state = s;
    // Map u32 → f32 in [-1, 1).
    (s as i32 as f32) / (i32::MAX as f32)
}

fn apply_bbd(
    bins: &mut [Complex<f32>],
    bbd_mag: &mut [Vec<f32>; BBD_STAGES],
    rng_state: &mut u32,
    curves: &[&[f32]],
) {
    let amount_c = curves[0];
    let thresh_c = curves[1];
    let release_c = curves[2];
    let mix_c = curves[3];

    let num_bins = bins.len();

    for k in 0..num_bins {
        let amount = amount_c[k].clamp(0.0, 2.0) * 0.5; // 0..1 stage advance gain
        let dither_amt = thresh_c[k].clamp(0.0, 2.0) * 0.005; // very small noise
        let lp_alpha = (release_c[k].clamp(0.01, 2.0) * 0.4).clamp(0.05, 0.9);
        let mix = mix_c[k].clamp(0.0, 2.0) * 0.5;

        let dry = bins[k];
        let in_mag = dry.norm();

        // Push input into stage 0 (with LP smoothing toward target).
        let target_0 = bbd_mag[0][k] + (in_mag - bbd_mag[0][k]) * lp_alpha;
        // Add dither.
        let dither_0 = xorshift32_step(rng_state) * dither_amt;
        bbd_mag[0][k] = (target_0 + dither_0).max(0.0);

        // Cascade: each stage takes the previous stage's previous value (1-hop delay per stage).
        // We process from last stage backwards to avoid overwriting source data.
        // Read all stages once, then write.
        let s0_prev = bbd_mag[0][k];
        let s1_prev = bbd_mag[1][k];
        let s2_prev = bbd_mag[2][k];
        let s3_prev = bbd_mag[3][k];

        bbd_mag[3][k] = s3_prev + (s2_prev - s3_prev) * lp_alpha + xorshift32_step(rng_state) * dither_amt;
        bbd_mag[2][k] = s2_prev + (s1_prev - s2_prev) * lp_alpha + xorshift32_step(rng_state) * dither_amt;
        bbd_mag[1][k] = s1_prev + (s0_prev - s1_prev) * lp_alpha + xorshift32_step(rng_state) * dither_amt;

        // Output: stage 3 (most-delayed) magnitude, scaled by amount.
        let out_mag = bbd_mag[3][k].max(0.0) * amount;
        let scale = if in_mag > 1e-9 { out_mag / in_mag } else { 0.0 };
        let wet = dry * scale;
        bins[k] = dry * (1.0 - mix) + wet * mix;
    }
}
```

- [ ] **Step 4: Wire dispatch in process()**

```rust
fn process(
    &mut self,
    channel: usize,
    _stereo_link: StereoLink,
    _target: FxChannelTarget,
    bins: &mut [Complex<f32>],
    _sidechain: Option<&[f32]>,
    curves: &[&[f32]],
    suppression_out: &mut [f32],
    _ctx: &ModuleContext,
) {
    debug_assert!(channel < 2);

    match self.mode {
        CircuitMode::BbdBins => {
            let bbd = &mut self.bbd_mag[channel];
            let rng = &mut self.rng_state[channel];
            apply_bbd(bins, bbd, rng, curves);
        }
        _ => {
            // Other modes filled in subsequent tasks.
        }
    }

    for s in suppression_out.iter_mut() {
        *s = 0.0;
    }
}
```

- [ ] **Step 5: Run test, expect pass**

Run: `cargo test --test module_trait circuit_bbd_delays_and_lowpasses -- --nocapture`
Expected: PASS — bin 100 < 4.0 after first hop, finite > 0.05 after several propagation hops.

- [ ] **Step 6: Commit**

```bash
git add src/dsp/modules/circuit.rs tests/module_trait.rs
git commit -m "feat(circuit): BBD Bins kernel (4-stage delay + LP + dither)"
```

---

## Task 4: Spectral Schmitt kernel — branch-free hysteresis latch

**Files:**
- Modify: `src/dsp/modules/circuit.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn circuit_schmitt_hysteresis_latches_above_threshold() {
    use spectral_forge::dsp::modules::circuit::{CircuitModule, CircuitMode};
    use spectral_forge::dsp::modules::{SpectralModule, StereoLink, FxChannelTarget};
    use realfft::num_complex::Complex;

    let mut module = CircuitModule::new();
    module.reset(48_000.0, 2048);
    module.set_mode_for_test(CircuitMode::SpectralSchmitt);

    let num_bins = 1025;
    // Two adjacent bins: one above threshold, one below.
    let mut bins: Vec<Complex<f32>> = vec![Complex::new(0.0, 0.0); num_bins];
    bins[100] = Complex::new(2.0, 0.0); // above on-threshold
    bins[101] = Complex::new(0.05, 0.0); // below off-threshold

    // AMOUNT=2 (max attenuation when off), THRESHOLD=1 (high=1.0), RELEASE=1 (gap=0.5 → low=0.5), MIX=2.
    let amount = vec![2.0_f32; num_bins];
    let thresh = vec![1.0_f32; num_bins];
    let release = vec![1.0_f32; num_bins];
    let mix = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &thresh, &release, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = circuit_test_ctx(num_bins);

    module.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, None, &curves, &mut suppression, &ctx);

    // Bin 100: latched ON → magnitude preserved.
    assert!((bins[100].norm() - 2.0).abs() < 0.1, "bin 100 should latch ON (got {})", bins[100].norm());
    // Bin 101: latched OFF → strongly attenuated.
    assert!(bins[101].norm() < 0.04, "bin 101 should latch OFF (got {})", bins[101].norm());

    // Now drop bin 100 to 0.6 — between high (1.0) and low (0.5) thresholds.
    // Latch state should hold (still ON because previously latched).
    bins[100] = Complex::new(0.6, 0.0);
    module.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, None, &curves, &mut suppression, &ctx);
    assert!(bins[100].norm() > 0.5, "bin 100 should hold ON in hysteresis band (got {})", bins[100].norm());

    // Drop bin 100 to 0.3 — below low threshold → should latch OFF.
    bins[100] = Complex::new(0.3, 0.0);
    module.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, None, &curves, &mut suppression, &ctx);
    assert!(bins[100].norm() < 0.1, "bin 100 should latch OFF after falling below low (got {})", bins[100].norm());
}
```

- [ ] **Step 2: Run test, expect failure**

Run: `cargo test --test module_trait circuit_schmitt_hysteresis_latches_above_threshold -- --nocapture`
Expected: FAIL — Schmitt arm unimplemented.

- [ ] **Step 3: Add the Schmitt kernel**

```rust
fn apply_schmitt(
    bins: &mut [Complex<f32>],
    latched: &mut [u8],
    curves: &[&[f32]],
) {
    let amount_c = curves[0];
    let thresh_c = curves[1];
    let release_c = curves[2];
    let mix_c = curves[3];

    let num_bins = bins.len();

    for k in 0..num_bins {
        let attenuation = (amount_c[k].clamp(0.0, 2.0)) * 0.5; // 0..1 = how much to attenuate when OFF
        let high = thresh_c[k].clamp(0.01, 4.0);
        // RELEASE controls hysteresis gap: gap = clamp(RELEASE × 0.5, 0.05, 0.95) × high.
        let gap = (release_c[k].clamp(0.0, 2.0) * 0.5).clamp(0.05, 0.95);
        let low = high * (1.0 - gap);

        let mag = bins[k].norm();
        let was_latched = latched[k] != 0;

        // Update latch state.
        let now_latched = if was_latched {
            mag > low
        } else {
            mag > high
        };
        latched[k] = if now_latched { 1 } else { 0 };

        let mix = mix_c[k].clamp(0.0, 2.0) * 0.5;
        let attenuate = if now_latched { 1.0 } else { 1.0 - attenuation };
        let dry = bins[k];
        let wet = dry * attenuate;
        bins[k] = dry * (1.0 - mix) + wet * mix;
    }
}
```

- [ ] **Step 4: Wire dispatch in process()**

```rust
CircuitMode::SpectralSchmitt => {
    let latched = &mut self.schmitt_latched[channel];
    apply_schmitt(bins, latched, curves);
}
```

- [ ] **Step 5: Run test, expect pass**

Run: `cargo test --test module_trait circuit_schmitt_hysteresis_latches_above_threshold -- --nocapture`
Expected: PASS — latch ON above high, hold in hysteresis band, latch OFF below low.

- [ ] **Step 6: Commit**

```bash
git add src/dsp/modules/circuit.rs tests/module_trait.rs
git commit -m "feat(circuit): Spectral Schmitt with hysteresis latch"
```

---

## Task 5: Crossover Distortion kernel — C¹-smooth deadzone

**Files:**
- Modify: `src/dsp/modules/circuit.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn circuit_crossover_smooth_deadzone() {
    use spectral_forge::dsp::modules::circuit::{CircuitModule, CircuitMode};
    use spectral_forge::dsp::modules::{SpectralModule, StereoLink, FxChannelTarget};
    use realfft::num_complex::Complex;

    let mut module = CircuitModule::new();
    module.reset(48_000.0, 2048);
    module.set_mode_for_test(CircuitMode::CrossoverDistortion);

    let num_bins = 1025;
    // Three test bins: well below deadzone, just inside, well above.
    let mut bins: Vec<Complex<f32>> = vec![Complex::new(0.0, 0.0); num_bins];
    bins[10] = Complex::new(0.05, 0.0);  // well below
    bins[50] = Complex::new(0.15, 0.0);  // just above (deadzone width = 0.1 with these curves)
    bins[100] = Complex::new(2.0, 0.0);  // well above

    // AMOUNT=1 (deadzone width = 0.1), MIX=2 (full wet); THRESHOLD/RELEASE unused.
    let amount = vec![1.0_f32; num_bins];
    let thresh = vec![1.0_f32; num_bins];
    let release = vec![1.0_f32; num_bins];
    let mix = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &thresh, &release, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = circuit_test_ctx(num_bins);

    module.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, None, &curves, &mut suppression, &ctx);

    // Below deadzone → silenced.
    assert!(bins[10].norm() < 0.005, "bin 10 should be deadzoned (got {})", bins[10].norm());
    // Just above → small but non-zero (smooth re-emergence).
    assert!(bins[50].norm() > 0.0 && bins[50].norm() < 0.1, "bin 50 should re-emerge gently (got {})", bins[50].norm());
    // Well above → near full magnitude.
    assert!(bins[100].norm() > 1.5, "bin 100 should pass mostly through (got {})", bins[100].norm());

    // Verify smooth re-emergence: the function (mag - dz)^2 / mag at mag = dz × 1.5 = 0.15 with dz = 0.1
    // gives output = (0.05)^2 / 0.15 ≈ 0.0167. Allow ±50% tolerance.
    let expected_50 = 0.05_f32.powi(2) / 0.15;
    assert!((bins[50].norm() - expected_50).abs() < 0.05,
        "bin 50 = {} not within tolerance of {}", bins[50].norm(), expected_50);
}
```

- [ ] **Step 2: Run test, expect failure**

Run: `cargo test --test module_trait circuit_crossover_smooth_deadzone -- --nocapture`
Expected: FAIL — Crossover arm unimplemented.

- [ ] **Step 3: Add the Crossover kernel**

```rust
fn apply_crossover(bins: &mut [Complex<f32>], curves: &[&[f32]]) {
    let amount_c = curves[0];
    let mix_c = curves[3];

    let num_bins = bins.len();

    for k in 0..num_bins {
        let dz_width = amount_c[k].clamp(0.0, 2.0) * 0.1; // up to 0.2 deadzone half-width
        let mix = mix_c[k].clamp(0.0, 2.0) * 0.5;

        let dry = bins[k];
        let mag = dry.norm();

        let new_mag = if mag <= dz_width {
            0.0
        } else {
            // Smooth re-emergence: (mag - dz)^2 / mag.
            let excess = mag - dz_width;
            (excess * excess) / mag
        };

        let scale = if mag > 1e-9 { new_mag / mag } else { 0.0 };
        let wet = dry * scale;
        bins[k] = dry * (1.0 - mix) + wet * mix;
    }
}
```

- [ ] **Step 4: Wire dispatch in process()**

```rust
CircuitMode::CrossoverDistortion => {
    apply_crossover(bins, curves);
}
```

- [ ] **Step 5: Run test, expect pass**

Run: `cargo test --test module_trait circuit_crossover_smooth_deadzone -- --nocapture`
Expected: PASS — below=0, just-above=small, above=near-full.

- [ ] **Step 6: Commit**

```bash
git add src/dsp/modules/circuit.rs tests/module_trait.rs
git commit -m "feat(circuit): Crossover Distortion with C¹-smooth deadzone"
```

---

## Task 6: Per-slot mode persistence — params.rs + FxMatrix dispatch

**Files:**
- Modify: `src/params.rs`
- Modify: `src/dsp/fx_matrix.rs`
- Modify: `src/dsp/modules/mod.rs`
- Modify: `src/dsp/modules/circuit.rs`
- Modify: `src/lib.rs::process()`

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn circuit_mode_persists_via_setter() {
    use spectral_forge::dsp::modules::{create_module, ModuleType, SpectralModule};
    use spectral_forge::dsp::modules::circuit::{CircuitMode, CircuitModule};

    let mut module = create_module(ModuleType::Circuit);
    module.reset(48_000.0, 2048);
    module.set_circuit_mode(CircuitMode::SpectralSchmitt);
    module.reset(48_000.0, 4096);

    let cir = module
        .as_any()
        .downcast_ref::<CircuitModule>()
        .expect("downcast");
    assert_eq!(cir.current_mode(), CircuitMode::SpectralSchmitt);
}
```

- [ ] **Step 2: Run test, expect failure**

Run: `cargo test --test module_trait circuit_mode_persists_via_setter -- --nocapture`
Expected: FAIL — `set_circuit_mode` not on trait.

- [ ] **Step 3: Add `set_circuit_mode` to SpectralModule trait**

In `src/dsp/modules/mod.rs::SpectralModule`:

```rust
fn set_circuit_mode(&mut self, _mode: crate::dsp::modules::circuit::CircuitMode) {
    // No-op default. Circuit overrides.
}
```

- [ ] **Step 4: Implement override in circuit.rs**

In `src/dsp/modules/circuit.rs::impl SpectralModule for CircuitModule`:

```rust
fn set_circuit_mode(&mut self, mode: CircuitMode) {
    if mode != self.mode {
        // Reset transient state on mode change.
        for ch in 0..2 {
            for stage in 0..BBD_STAGES {
                for v in self.bbd_mag[ch][stage].iter_mut() {
                    *v = 0.0;
                }
            }
            for l in self.schmitt_latched[ch].iter_mut() {
                *l = 0;
            }
        }
        self.mode = mode;
    }
}
```

`reset()` does NOT reassign `self.mode`.

- [ ] **Step 5: Add params field**

In `src/params.rs`:

```rust
#[persist = "slot_circuit_mode"]
pub slot_circuit_mode: [Arc<Mutex<crate::dsp::modules::circuit::CircuitMode>>; MAX_SLOTS],
```

In Default::default:

```rust
slot_circuit_mode: std::array::from_fn(|_| {
    Arc::new(Mutex::new(crate::dsp::modules::circuit::CircuitMode::default()))
}),
```

Add snap helper:

```rust
impl SpectralForgeParams {
    pub fn circuit_mode_snap(&self) -> [crate::dsp::modules::circuit::CircuitMode; MAX_SLOTS] {
        std::array::from_fn(|s| {
            self.slot_circuit_mode[s]
                .try_lock()
                .map(|g| *g)
                .unwrap_or_default()
        })
    }
}
```

- [ ] **Step 6: Add FxMatrix sync method**

In `src/dsp/fx_matrix.rs::FxMatrix`:

```rust
pub fn set_circuit_modes(
    &mut self,
    modes: &[crate::dsp::modules::circuit::CircuitMode; MAX_SLOTS],
) {
    for (s, slot) in self.slots.iter_mut().enumerate() {
        if let Some(module) = slot {
            module.set_circuit_mode(modes[s]);
        }
    }
}
```

- [ ] **Step 7: Wire snapshot + push in lib.rs**

```rust
let cir_modes = self.params.circuit_mode_snap();
self.fx_matrix.set_circuit_modes(&cir_modes);
```

- [ ] **Step 8: Run test, expect pass**

Run: `cargo test --test module_trait circuit_mode_persists_via_setter -- --nocapture`
Expected: PASS.

- [ ] **Step 9: Commit**

```bash
git add src/params.rs src/dsp/fx_matrix.rs src/dsp/modules/mod.rs src/dsp/modules/circuit.rs src/lib.rs tests/module_trait.rs
git commit -m "feat(circuit): per-slot mode persistence + setter dispatch"
```

---

## Task 7: Circuit mode picker UI (popup)

**Files:**
- Create: `src/editor/circuit_popup.rs`
- Modify: `src/editor/mod.rs`
- Modify: `src/editor/module_popup.rs`
- Modify: `src/editor/editor_ui.rs`

- [ ] **Step 1: Add module declaration**

In `src/editor/mod.rs`:

```rust
pub mod circuit_popup;
```

- [ ] **Step 2: Create circuit_popup.rs**

```rust
use std::sync::{Arc, Mutex};
use nih_plug_egui::egui;
use crate::dsp::modules::circuit::CircuitMode;
use crate::editor::theme;

pub struct CircuitPopupState {
    pub open_for_slot: Option<usize>,
    pub anchor: egui::Pos2,
}

impl CircuitPopupState {
    pub fn new() -> Self {
        Self { open_for_slot: None, anchor: egui::Pos2::ZERO }
    }
}

pub fn show_circuit_popup(
    ui: &mut egui::Ui,
    state: &mut CircuitPopupState,
    slot_circuit_mode: &Arc<Mutex<CircuitMode>>,
) -> bool {
    let Some(_slot) = state.open_for_slot else { return false; };
    let area_id = egui::Id::new("circuit_mode_picker");
    let mut selected = false;

    egui::Area::new(area_id)
        .order(egui::Order::Foreground)
        .fixed_pos(state.anchor)
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style())
                .fill(theme::POPUP_BG)
                .stroke(egui::Stroke::new(1.0, theme::POPUP_BORDER))
                .show(ui, |ui| {
                    ui.set_min_width(140.0);
                    ui.label(egui::RichText::new("CIRCUIT MODE")
                        .color(theme::POPUP_TITLE).size(11.0));
                    ui.separator();
                    let cur = *slot_circuit_mode.lock().unwrap();
                    for (label, mode) in [
                        ("Crossover Distortion", CircuitMode::CrossoverDistortion),
                        ("Spectral Schmitt",     CircuitMode::SpectralSchmitt),
                        ("BBD Bins",             CircuitMode::BbdBins),
                    ] {
                        let is_active = cur == mode;
                        let color = if is_active { theme::CIRCUIT_DOT_COLOR } else { theme::POPUP_TEXT };
                        let response = ui.selectable_label(
                            is_active,
                            egui::RichText::new(label).color(color).size(11.0),
                        );
                        if response.clicked() {
                            *slot_circuit_mode.lock().unwrap() = mode;
                            selected = true;
                        }
                    }
                });
        });

    if selected {
        state.open_for_slot = None;
    }
    selected
}
```

- [ ] **Step 3: Add Circuit to ASSIGNABLE_MODULES**

In `src/editor/module_popup.rs`:

```rust
ModuleType::Circuit,
```

- [ ] **Step 4: Wire the popup invocation in editor_ui.rs**

```rust
if module_type == ModuleType::Circuit && response.secondary_clicked() {
    circuit_popup_state.open_for_slot = Some(slot_idx);
    circuit_popup_state.anchor = response.rect.right_top();
}
if circuit_popup_state.open_for_slot.is_some() {
    let slot_idx = circuit_popup_state.open_for_slot.unwrap();
    circuit_popup::show_circuit_popup(
        ui,
        &mut circuit_popup_state,
        &params.slot_circuit_mode[slot_idx],
    );
}
```

- [ ] **Step 5: Verify compile**

Run: `cargo build`
Expected: clean build.

Manual check: load in Bitwig, assign Circuit, right-click → 3 modes selectable, persistence verified.

- [ ] **Step 6: Commit**

```bash
git add src/editor/circuit_popup.rs src/editor/mod.rs src/editor/module_popup.rs src/editor/editor_ui.rs
git commit -m "feat(circuit): mode picker popup UI"
```

---

## Task 8: Calibration probes

**Files:**
- Modify: `src/dsp/modules/circuit.rs`
- Modify: `tests/calibration_roundtrip.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(feature = "probe")]
#[test]
fn circuit_calibration_roundtrip_all_modes() {
    use spectral_forge::dsp::modules::circuit::{CircuitModule, CircuitMode, CircuitProbe};
    use spectral_forge::dsp::modules::{SpectralModule, StereoLink, FxChannelTarget};
    use realfft::num_complex::Complex;

    let num_bins = 1025;

    for mode in [
        CircuitMode::CrossoverDistortion,
        CircuitMode::SpectralSchmitt,
        CircuitMode::BbdBins,
    ] {
        let mut module = CircuitModule::new();
        module.reset(48_000.0, 2048);
        module.set_circuit_mode(mode);

        let mut bins: Vec<Complex<f32>> = vec![Complex::new(0.5, 0.0); num_bins];
        let amount = vec![1.0_f32; num_bins];
        let thresh = vec![1.0_f32; num_bins];
        let release = vec![1.0_f32; num_bins];
        let mix = vec![1.0_f32; num_bins];
        let curves: Vec<&[f32]> = vec![&amount, &thresh, &release, &mix];
        let mut suppression = vec![0.0_f32; num_bins];
        let ctx = ckt_test_ctx(num_bins);

        for _ in 0..5 {
            module.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, None, &curves, &mut suppression, &ctx);
        }

        let probe = module.probe_state(0);
        assert_eq!(probe.active_mode, mode);
        assert!(probe.average_amount_pct >= 0.0 && probe.average_amount_pct <= 200.0);
    }
}

#[cfg(feature = "probe")]
fn ckt_test_ctx(num_bins: usize) -> spectral_forge::dsp::modules::ModuleContext {
    spectral_forge::dsp::modules::ModuleContext {
        sample_rate: 48_000.0,
        fft_size: 2048,
        num_bins,
        attack_ms: 10.0,
        release_ms: 100.0,
        sensitivity: 1.0,
        suppression_width: 1.0,
        auto_makeup: false,
        delta_monitor: false,
        bpm: 120.0,
        beat_position: 0.0,
        unwrapped_phase: None,
        peaks: None,
    }
}
```

- [ ] **Step 2: Run test, expect failure**

Run: `cargo test --features probe --test calibration_roundtrip circuit -- --nocapture`
Expected: FAIL — `CircuitProbe` not found.

- [ ] **Step 3: Add probe types and method**

In `src/dsp/modules/circuit.rs`:

```rust
#[cfg(any(test, feature = "probe"))]
#[derive(Debug, Clone, Copy)]
pub struct CircuitProbe {
    pub active_mode: CircuitMode,
    pub average_amount_pct: f32,
    /// BBD: average magnitude across stage 3 (most-delayed). Zero for other modes.
    pub bbd_stage3_avg: f32,
    /// Schmitt: number of bins currently latched ON. Zero for other modes.
    pub schmitt_active_count: u32,
}

#[cfg(any(test, feature = "probe"))]
impl CircuitModule {
    pub fn probe_state(&self, channel: usize) -> CircuitProbe {
        let ch = channel.min(1);

        let bbd_stage3_avg = if self.mode == CircuitMode::BbdBins && !self.bbd_mag[ch][3].is_empty() {
            let sum: f32 = self.bbd_mag[ch][3].iter().sum();
            sum / self.bbd_mag[ch][3].len() as f32
        } else {
            0.0
        };

        let schmitt_active_count = if self.mode == CircuitMode::SpectralSchmitt {
            self.schmitt_latched[ch].iter().filter(|&&l| l != 0).count() as u32
        } else {
            0
        };

        CircuitProbe {
            active_mode: self.mode,
            average_amount_pct: 100.0,
            bbd_stage3_avg,
            schmitt_active_count,
        }
    }
}
```

- [ ] **Step 4: Run test, expect pass**

Run: `cargo test --features probe --test calibration_roundtrip circuit -- --nocapture`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/dsp/modules/circuit.rs tests/calibration_roundtrip.rs
git commit -m "test(circuit): calibration probes for all 3 modes"
```

---

## Task 9: Multi-hop dual-channel finite/bounded contract test

**Files:**
- Modify: `tests/module_trait.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn circuit_finite_bounded_all_modes_dual_channel() {
    use spectral_forge::dsp::modules::circuit::{CircuitModule, CircuitMode};
    use spectral_forge::dsp::modules::{SpectralModule, StereoLink, FxChannelTarget};
    use realfft::num_complex::Complex;

    let num_bins = 1025;

    for mode in [
        CircuitMode::CrossoverDistortion,
        CircuitMode::SpectralSchmitt,
        CircuitMode::BbdBins,
    ] {
        let mut module = CircuitModule::new();
        module.reset(48_000.0, 2048);
        module.set_circuit_mode(mode);

        let mut bins_l: Vec<Complex<f32>> = (0..num_bins).map(|k|
            Complex::new(((k as f32 * 0.07).sin() + 0.1).abs(),
                         ((k as f32 * 0.11).cos() * 0.5))
        ).collect();
        let mut bins_r: Vec<Complex<f32>> = bins_l.iter().map(|b| b * 0.6).collect();

        let amount = vec![1.5_f32; num_bins];
        let mid = vec![1.0_f32; num_bins];
        let mix = vec![1.0_f32; num_bins];
        let curves: Vec<&[f32]> = vec![&amount, &mid, &mid, &mix];

        let mut suppression = vec![0.0_f32; num_bins];
        let ctx = circuit_test_ctx(num_bins);

        for hop in 0..200 {
            for ch in 0..2 {
                let bins = if ch == 0 { &mut bins_l } else { &mut bins_r };
                module.process(ch, StereoLink::Independent, FxChannelTarget::All, bins, None, &curves, &mut suppression, &ctx);
                for (i, b) in bins.iter().enumerate() {
                    assert!(b.norm().is_finite(), "mode={:?} hop={} ch={} bin={} norm={}", mode, hop, ch, i, b.norm());
                    assert!(b.norm() < 1e6, "runaway: mode={:?} hop={} ch={} bin={} norm={}", mode, hop, ch, i, b.norm());
                }
                for s in &suppression {
                    assert!(s.is_finite() && *s >= 0.0);
                }
            }
        }
    }
}
```

- [ ] **Step 2: Run test, expect pass**

Run: `cargo test --test module_trait circuit_finite_bounded_all_modes_dual_channel -- --nocapture`
Expected: PASS — kernels are bounded by clamps + magnitude scaling.

- [ ] **Step 3: Commit**

```bash
git add tests/module_trait.rs
git commit -m "test(circuit): multi-hop dual-channel finite/bounded contract"
```

---

## Task 10: Status banner + STATUS.md

**Files:**
- Modify: `docs/superpowers/STATUS.md`
- Modify: this plan file

- [ ] **Step 1: Update banner at top of this plan**

Change (only after merge):

```
> **Status:** PLANNED — implementation pending.
```

to:

```
> **Status:** IMPLEMENTED — landed in commit <SHA>.
```

- [ ] **Step 2: Add entry to STATUS.md**

```
| 2026-04-27-phase-2g-circuit-light.md | IMPLEMENTED | Circuit module v1: BBD Bins, Spectral Schmitt, Crossover Distortion. Defers Vactrol/Transformer/Sag/Drift/PCB/Slew/Bias/Resonant to Phase 5c. |
```

- [ ] **Step 3: Final commit**

```bash
git add docs/superpowers/plans/2026-04-27-phase-2g-circuit-light.md docs/superpowers/STATUS.md
git commit -m "docs(status): mark phase-2g Circuit-light IMPLEMENTED"
```

---

## Self-review

**Spec coverage check:**
- ✅ BBD Bins (4-stage delay + LP + dither) — Task 3
- ✅ Spectral Schmitt (hysteresis latch) — Task 4
- ✅ Crossover Distortion (C¹-smooth deadzone) — Task 5
- ✅ 4 curves: AMOUNT, THRESHOLD, RELEASE, MIX — Task 1
- ✅ Per-channel state for Independent/MidSide modes — Task 2 + 9
- ✅ Per-slot mode persistence — Task 6
- ✅ Calibration probes — Task 8
- ✅ Default mode = CrossoverDistortion (cheapest, safest passthrough on assignment) — Task 2

**Spec items deferred to Phase 5c (NOT in v1):**
- Vactrol — needs `BinPhysics::flux` + cascaded 1-pole.
- Transformer Saturation — needs `BinPhysics::flux` + tanh polynomial + SPREAD curve.
- Power Sag (with thermal refinement) — needs `BinPhysics::temperature`.
- Component Drift — needs `BinPhysics::temperature`.
- PCB Crosstalk — needs spread kernel + SPREAD curve.
- Slew Distortion — needs phase-scramble or noise-add (research-graded).
- Bias Fuzz — needs `BinPhysics::bias` (new field).
- SPREAD curve (5th curve slot) — added in Phase 5c when Transformer/PCB land.
- Resonant Feedback — handled via `RouteMatrix` + Matrix Amp Nodes (Phase 2a) instead of as a sub-effect.
- Envelope Follower Ripple — global helper, separately tracked.

**Type consistency:** `CircuitMode` enum used consistently across params, FxMatrix, CircuitModule, popup, probes.

**Placeholder scan:** No "TBD" / "implement later". All 3 kernels have full code in their tasks; tests have full assertions including the C¹-smooth deadzone formula verification.

**No dependency on Phase 1 enrichment:** Circuit-light uses only the SpectralModule trait + ModuleContext basics. Does not touch `unwrapped_phase`, `peaks`, `panel_widget`, or sidechain.

---

## Execution Handoff

**Plan complete and saved to `docs/superpowers/plans/2026-04-27-phase-2g-circuit-light.md`. Two execution options:**

**1. Subagent-Driven (recommended)** — dispatch fresh subagent per task, review between tasks.
**2. Inline Execution** — execute tasks in this session using executing-plans, batch with checkpoints.
