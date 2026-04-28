# Phase 5a — Life Module Implementation Plan

> **Status:** IMPLEMENTED — landed in commits `33ecea8`..`085a49d` (Phase 5a.0 → 5a.17). Source of truth: source code (`src/dsp/modules/life.rs`, `src/editor/life_popup.rs`, `tests/life_conservation.rs`).
>
> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship the Life module with **10 sub-effect modes** — Viscosity, SurfaceTension, Crystallization, Archimedes, NonNewtonian, Stiction, Yield, Capillary, Sandpaper, Brownian — using the existing SpectralModule trait, BinPhysics read/write, and per-slot mode persistence. Energy conservation is a stated invariant for diffusive/transport modes (Viscosity, SurfaceTension, Capillary, Archimedes) and explicitly exempt for state-creating modes (Crystallization, Yield, Brownian).

**Architecture:** New `ModuleType::Life` slot. Per-channel state holds:
- Per-bin floats: `wick_envelope`, `wick_carry`, `tear_state`, `sustain_envelope`, `is_moving`.
- Pre-allocated scratch buffers (`scratch_power`, `scratch_mag`) for two-pass kernels (Viscosity, SurfaceTension, Capillary).
- A `rng_state: u32` xorshift for Brownian.

Mode is per-slot (persisted via `Mutex<LifeMode>`), dispatched per block. The module declares `writes_bin_physics = true` because Crystallization writes `crystallization`, Yield writes `bias`, Stiction/NonNewtonian write `displacement`, Brownian amplifies `temperature` (when present).

**Tech Stack:** Rust 2021, `nih-plug`, `realfft::num_complex::Complex<f32>`, existing `SpectralModule` trait + `ModuleContext`, Phase 3 `BinPhysics`.

**Source spec:** `ideas/next-gen-modules/11-life.md` (audit + research findings 2026-04-26 incorporated). Original `docs/superpowers/specs/2026-04-21-life-module.md` superseded by the audit which adds Yield, Capillary, Sandpaper, Brownian.

**Defer list (NOT in this plan):**
- **Multi-mode-per-slot stacking** — v1 ships single-mode-per-slot; running two Life modes in parallel requires two slots. Confirmed in audit § Module ordering. v2.
- **Cepstral envelope baseline for Capillary** — research suggests porting WORLD CheapTrick (~300 lines, BSD). v1 uses a per-bin slow LP. v2 enhancement.
- **Adaptive `clamp_for_cfl()` helper shared with Kinetics** — Phase 5b will introduce it. v1 hardcodes `D ≤ 0.45` inline.
- **1-pole smoothing of per-bin parameter curves** — research recommendation; implemented inline as needed (e.g., `sustain_envelope` is itself a 1-pole). The shared helper lands with Kinetics.
- **Crystallization↔Freeze cooperation** — read direction (Freeze reads `BinPhysics.crystallization`) is enabled by this plan's writes; **Freeze-side reader integration** lands as a small follow-up PR after this ships (Task 18 calls it out).

**Risk register:**
- Diffusion stability: FTCS on power requires `D[k] ≤ 0.5` strictly. We clamp to `0.45` (10% safety margin) per research finding 3. Tested in Task 17.
- Yield phase scrambling uses an xorshift PRNG (cheap, period 2³²-1). Safe for audio because phase scramble is intentionally non-deterministic; we do not need a cryptographic RNG.
- Brownian without an upstream temperature writer is a no-op (multiplies by 0). Document this in the popup tooltip; do **not** add a fallback "noise injection" — that defeats the purpose per audit § d.
- Sandpaper spark deposit at `k + REACH × log_offset` can land on bins already at high magnitude — additive blend rather than overwrite, with an explicit clamp to avoid clipping the FFT output.

---

## File Structure

**Create:**
- `src/dsp/modules/life.rs` — `LifeModule` impl, `LifeMode` enum, all 10 kernel functions.
- `src/editor/life_popup.rs` — mode picker popup (10 modes).

**Modify:**
- `src/dsp/modules/mod.rs` — add `ModuleType::Life` variant, `module_spec(Life)` entry, `create_module()` wiring, `set_life_mode` trait default.
- `src/dsp/fx_matrix.rs` — add `slot_life_modes: [LifeMode; MAX_SLOTS]`, `set_life_modes()`.
- `src/params.rs` — add `slot_life_mode: [Arc<Mutex<LifeMode>>; MAX_SLOTS]`.
- `src/lib.rs` — snapshot per-block, dispatch to FxMatrix.
- `src/editor/theme.rs` — `LIFE_DOT_COLOR`.
- `src/editor/mod.rs` — `pub mod life_popup;`.
- `src/editor/module_popup.rs` — make Life assignable + invoke life popup on right-click.
- `src/editor/fx_matrix_grid.rs` — render Life slot label.
- `tests/module_trait.rs` — finite/bounded test for all 10 modes.
- `tests/calibration_roundtrip.rs` — Life probes.
- `tests/life_conservation.rs` (new) — energy-conservation invariant test.
- `docs/superpowers/STATUS.md` — entry for this plan.

---

## Task 1: Add `ModuleType::Life` variant + theme color + ModuleSpec entry

**Files:**
- Modify: `src/dsp/modules/mod.rs` (`ModuleType` enum, `module_spec()` catalog)
- Modify: `src/editor/theme.rs` (end of color block)
- Test: `tests/module_trait.rs`

- [ ] **Step 1: Write the failing test**

Append to `tests/module_trait.rs`:

```rust
#[test]
fn life_module_spec_present() {
    use spectral_forge::dsp::modules::{module_spec, ModuleType};

    let spec = module_spec(ModuleType::Life);
    assert_eq!(spec.display_name, "LIFE");
    assert_eq!(spec.num_curves, 5);
    assert_eq!(spec.curve_labels.len(), 5);
    assert_eq!(spec.curve_labels[0], "AMOUNT");
    assert_eq!(spec.curve_labels[1], "THRESHOLD");
    assert_eq!(spec.curve_labels[2], "SPEED");
    assert_eq!(spec.curve_labels[3], "REACH");
    assert_eq!(spec.curve_labels[4], "MIX");
    assert!(spec.assignable_to_user_slots, "Life must be user-assignable");
    assert!(!spec.heavy_cpu, "Light enough at 8193 bins per audit");
    assert!(!spec.wants_sidechain, "Life is not a sidechain-driven module");
    assert!(spec.writes_bin_physics, "Life writes crystallization/bias/displacement");
}
```

- [ ] **Step 2: Run test, expect failure**

Run: `cargo test --test module_trait life_module_spec_present -- --nocapture`
Expected: FAIL — `Life` variant not found.

- [ ] **Step 3: Add the enum variant**

In `src/dsp/modules/mod.rs`, locate the `ModuleType` enum and add `Life` immediately before `Master`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Enum, Serialize, Deserialize)]
pub enum ModuleType {
    // ... existing variants in declaration order ...
    Life,
    Master,
}
```

- [ ] **Step 4: Add `module_spec` entry**

In `src/dsp/modules/mod.rs`, in the `module_spec()` match, add:

```rust
ModuleType::Life => ModuleSpec {
    ty: ModuleType::Life,
    display_name: "LIFE",
    color: theme::LIFE_DOT_COLOR,
    num_curves: 5,
    curve_labels: &["AMOUNT", "THRESHOLD", "SPEED", "REACH", "MIX"],
    assignable_to_user_slots: true,
    heavy_cpu: false,
    wants_sidechain: false,
    writes_bin_physics: true,
    panel_widget: None,
},
```

- [ ] **Step 5: Add theme constant**

In `src/editor/theme.rs`, append (at the end of the colour block):

```rust
/// Life module — warm green for "biology / fluid life" feel.
pub const LIFE_DOT_COLOR: egui::Color32 = egui::Color32::from_rgb(110, 185, 100);
```

- [ ] **Step 6: Run test, expect pass**

Run: `cargo test --test module_trait life_module_spec_present -- --nocapture`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add src/dsp/modules/mod.rs src/editor/theme.rs tests/module_trait.rs
git commit -m "$(cat <<'EOF'
feat(life): add ModuleType::Life variant + spec entry

5 curves (AMOUNT, THRESHOLD, SPEED, REACH, MIX), green dot, declares
writes_bin_physics for Crystallization/Yield/Stiction/NonNewtonian
writers added in later tasks.

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: `LifeMode` enum + `LifeModule` struct skeleton + `create_module()` wiring

**Files:**
- Create: `src/dsp/modules/life.rs`
- Modify: `src/dsp/modules/mod.rs` (add `pub mod life;` + `create_module()` arm)
- Test: `tests/module_trait.rs`

- [ ] **Step 1: Write the failing test**

Append to `tests/module_trait.rs`:

```rust
#[test]
fn life_module_constructs_and_passes_through() {
    use spectral_forge::dsp::modules::{create_module, ModuleType, ModuleContext};
    use spectral_forge::dsp::modules::{StereoLink, FxChannelTarget};
    use realfft::num_complex::Complex;

    let mut module = create_module(ModuleType::Life);
    module.reset(48_000.0, 2048);
    assert_eq!(module.module_type(), ModuleType::Life);
    assert_eq!(module.num_curves(), 5);

    let num_bins = 1025;
    let mut bins: Vec<Complex<f32>> = (0..num_bins)
        .map(|k| Complex::new((k as f32 * 0.013).sin(), (k as f32 * 0.011).cos()))
        .collect();
    let dry: Vec<Complex<f32>> = bins.clone();

    // AMOUNT=0, THRESHOLD=neutral, SPEED=neutral, REACH=neutral, MIX=0 → passthrough
    let zeros = vec![0.0_f32; num_bins];
    let neutral = vec![1.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&zeros, &neutral, &neutral, &neutral, &zeros];

    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = ModuleContext {
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
        bin_physics: None,
    };

    module.process(
        0,
        StereoLink::Linked,
        FxChannelTarget::All,
        &mut bins,
        None,
        &curves,
        &mut suppression,
        None,
        &ctx,
    );

    for k in 0..num_bins {
        let diff = (bins[k] - dry[k]).norm();
        assert!(diff < 1e-5, "bin {} drifted by {} (passthrough expected)", k, diff);
    }
    for s in &suppression {
        assert!(s.is_finite() && *s >= 0.0);
    }
}
```

- [ ] **Step 2: Run test, expect failure**

Run: `cargo test --test module_trait life_module_constructs_and_passes_through -- --nocapture`
Expected: FAIL — `create_module(Life)` panics with `unimplemented`.

- [ ] **Step 3: Create `src/dsp/modules/life.rs` with skeleton**

```rust
use realfft::num_complex::Complex;
use serde::{Deserialize, Serialize};

use crate::dsp::bin_physics::BinPhysics;
use crate::dsp::modules::{
    FxChannelTarget, ModuleContext, ModuleType, SpectralModule, StereoLink,
};

/// FTCS diffusion clamp from `ideas/next-gen-modules/11-life.md` research finding 3.
/// Strict bound is 0.5; we clamp to 0.45 for a 10% safety margin across hop rates.
const VISCOSITY_D_MAX: f32 = 0.45;

/// Sustain LP coefficient (~50 ms time constant at 48 kHz / 256-sample hop).
/// Used by Capillary and Crystallization for "sustained-ness" detection.
const SUSTAIN_LP_ALPHA: f32 = 0.05;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LifeMode {
    Viscosity,
    SurfaceTension,
    Crystallization,
    Archimedes,
    NonNewtonian,
    Stiction,
    Yield,
    Capillary,
    Sandpaper,
    Brownian,
}

impl Default for LifeMode {
    fn default() -> Self {
        LifeMode::Viscosity
    }
}

pub struct LifeModule {
    mode: LifeMode,
    /// Per-channel scratch buffers (allocated to fft_size/2+1 in reset()).
    scratch_power: [Vec<f32>; 2],
    scratch_mag: [Vec<f32>; 2],
    /// Per-channel slow-LP envelopes used by Capillary (sustain) + Crystallization.
    sustain_envelope: [Vec<f32>; 2],
    /// Per-channel Capillary in-flight carry buffer.
    wick_carry: [Vec<f32>; 2],
    /// Per-channel Yield tear-state (0.0 = elastic, 1.0 = torn, in-between = healing).
    tear_state: [Vec<f32>; 2],
    /// Per-channel Stiction "is-moving" bool encoded as f32 (0.0 / 1.0).
    is_moving: [Vec<f32>; 2],
    /// Per-channel xorshift RNG state for Brownian + Yield phase scramble.
    rng_state: [u32; 2],
    sample_rate: f32,
    fft_size: usize,
}

impl LifeModule {
    pub fn new() -> Self {
        Self {
            mode: LifeMode::default(),
            scratch_power: [Vec::new(), Vec::new()],
            scratch_mag: [Vec::new(), Vec::new()],
            sustain_envelope: [Vec::new(), Vec::new()],
            wick_carry: [Vec::new(), Vec::new()],
            tear_state: [Vec::new(), Vec::new()],
            is_moving: [Vec::new(), Vec::new()],
            rng_state: [0xCAFE_F00D, 0xDEAD_BEEF],
            sample_rate: 48_000.0,
            fft_size: 2048,
        }
    }

    #[cfg(any(test, feature = "probe"))]
    pub fn set_mode_for_test(&mut self, mode: LifeMode) {
        self.mode = mode;
    }

    pub(crate) fn set_mode(&mut self, mode: LifeMode) {
        self.mode = mode;
    }
}

#[inline]
fn xorshift32_step(state: &mut u32) -> u32 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *state = x;
    x
}

#[inline]
fn xorshift32_signed_unit(state: &mut u32) -> f32 {
    // Map u32 → f32 in [-1.0, 1.0).
    let u = xorshift32_step(state);
    (u as f32 / u32::MAX as f32) * 2.0 - 1.0
}

impl SpectralModule for LifeModule {
    fn process(
        &mut self,
        channel: usize,
        _stereo_link: StereoLink,
        _target: FxChannelTarget,
        _bins: &mut [Complex<f32>],
        _sidechain: Option<&[f32]>,
        _curves: &[&[f32]],
        suppression_out: &mut [f32],
        _physics: Option<&mut BinPhysics>,
        _ctx: &ModuleContext<'_>,
    ) {
        // v1 stub — kernels added in Tasks 3-12.
        debug_assert!(channel < 2);
        for s in suppression_out.iter_mut() {
            *s = 0.0;
        }
    }

    fn reset(&mut self, sample_rate: f32, fft_size: usize) {
        self.sample_rate = sample_rate;
        self.fft_size = fft_size;
        let num_bins = fft_size / 2 + 1;
        for ch in 0..2 {
            self.scratch_power[ch].clear();
            self.scratch_power[ch].resize(num_bins, 0.0);
            self.scratch_mag[ch].clear();
            self.scratch_mag[ch].resize(num_bins, 0.0);
            self.sustain_envelope[ch].clear();
            self.sustain_envelope[ch].resize(num_bins, 0.0);
            self.wick_carry[ch].clear();
            self.wick_carry[ch].resize(num_bins, 0.0);
            self.tear_state[ch].clear();
            self.tear_state[ch].resize(num_bins, 0.0);
            self.is_moving[ch].clear();
            self.is_moving[ch].resize(num_bins, 0.0);
        }
        // Reseed RNG so reset is deterministic across runs.
        self.rng_state = [0xCAFE_F00D, 0xDEAD_BEEF];
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::Life
    }

    fn num_curves(&self) -> usize {
        5
    }
}
```

- [ ] **Step 4: Wire `create_module()` and `mod.rs` declaration**

In `src/dsp/modules/mod.rs`, add at the top with the other `pub mod`:

```rust
pub mod life;
```

In `create_module()`:

```rust
ModuleType::Life => Box::new(crate::dsp::modules::life::LifeModule::new()),
```

- [ ] **Step 5: Run test, expect pass**

Run: `cargo test --test module_trait life_module_constructs_and_passes_through -- --nocapture`
Expected: PASS — module constructs, passthrough holds (suppression cleared to zero).

- [ ] **Step 6: Commit**

```bash
git add src/dsp/modules/life.rs src/dsp/modules/mod.rs tests/module_trait.rs
git commit -m "feat(life): module skeleton + LifeMode enum (10 modes)"
```

---

## Task 3: Viscosity kernel — FTCS finite-volume diffusion on power

**Files:**
- Modify: `src/dsp/modules/life.rs` (add `apply_viscosity` + dispatch)
- Test: `tests/module_trait.rs`

- [ ] **Step 1: Write the failing test**

Append to `tests/module_trait.rs`:

```rust
#[test]
fn life_viscosity_diffuses_and_conserves() {
    use spectral_forge::dsp::modules::life::{LifeModule, LifeMode};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule, StereoLink, FxChannelTarget};
    use realfft::num_complex::Complex;

    let mut module = LifeModule::new();
    module.reset(48_000.0, 2048);
    module.set_mode_for_test(LifeMode::Viscosity);

    let num_bins = 1025;
    let mut bins: Vec<Complex<f32>> = vec![Complex::new(0.0, 0.0); num_bins];
    bins[100] = Complex::new(2.0, 0.0); // Single tone, all energy at bin 100.
    let dry_power: f32 = bins.iter().map(|b| b.norm_sqr()).sum();

    // AMOUNT=2 (D=0.45 max), THRESHOLD=neutral, SPEED=neutral, REACH=neutral, MIX=2 (full wet).
    let amount = vec![2.0_f32; num_bins];
    let neutral = vec![1.0_f32; num_bins];
    let mix = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &neutral, &neutral, &neutral, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = ModuleContext {
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
        bin_physics: None,
    };

    // 5 hops to let energy spread.
    for _ in 0..5 {
        module.process(
            0, StereoLink::Linked, FxChannelTarget::All,
            &mut bins, None, &curves, &mut suppression, None, &ctx,
        );
    }

    let wet_power: f32 = bins.iter().map(|b| b.norm_sqr()).sum();

    // Conservation invariant: power loss < 5% (boundary effects + numerical roundoff).
    let loss_pct = (dry_power - wet_power).abs() / dry_power;
    assert!(loss_pct < 0.05, "Viscosity lost {}% of power (>5% violates conservation)", loss_pct * 100.0);

    // Energy must have spread away from bin 100 — energy at bins 99 and 101 must be > 0.
    assert!(bins[99].norm() > 0.01, "Energy did not diffuse left (bin 99 = {})", bins[99].norm());
    assert!(bins[101].norm() > 0.01, "Energy did not diffuse right (bin 101 = {})", bins[101].norm());

    for b in &bins {
        assert!(b.norm().is_finite());
    }
}
```

- [ ] **Step 2: Run test, expect failure**

Run: `cargo test --test module_trait life_viscosity_diffuses_and_conserves -- --nocapture`
Expected: FAIL — current `process()` is a stub; bins unchanged.

- [ ] **Step 3: Add the Viscosity kernel**

In `src/dsp/modules/life.rs`, add above `impl SpectralModule`:

```rust
/// FTCS finite-volume diffusion of `|bin|^2` (power) with harmonic-mean face flux.
/// Reflective boundaries (zero flux at k=0 and k=num_bins-1).
/// Phase preserved via complex scaling.
fn apply_viscosity(
    bins: &mut [Complex<f32>],
    scratch_power: &mut [f32],
    scratch_mag: &mut [f32],
    curves: &[&[f32]],
) {
    const EPS: f32 = 1e-12;

    let amount_c = curves[0];
    let mix_c = curves[4];

    let num_bins = bins.len();

    // Cache magnitudes + power.
    for k in 0..num_bins {
        let mag = bins[k].norm();
        scratch_mag[k] = mag;
        scratch_power[k] = mag * mag;
    }

    // Per-bin diffusion coefficient from AMOUNT (clamped to safe FTCS range).
    // AMOUNT in [0, 2] → D in [0, VISCOSITY_D_MAX].
    // Boundaries (k=0, k=num_bins-1) handled below by the loop bounds (reflective).
    for k in 1..num_bins - 1 {
        let d_k     = (amount_c[k]     * 0.5 * VISCOSITY_D_MAX).clamp(0.0, VISCOSITY_D_MAX);
        let d_kp1   = (amount_c[k + 1] * 0.5 * VISCOSITY_D_MAX).clamp(0.0, VISCOSITY_D_MAX);
        let d_km1   = (amount_c[k - 1] * 0.5 * VISCOSITY_D_MAX).clamp(0.0, VISCOSITY_D_MAX);
        let d_face_right = 2.0 * d_k * d_kp1 / (d_k + d_kp1 + EPS);
        let d_face_left  = 2.0 * d_k * d_km1 / (d_k + d_km1 + EPS);
        let p_new = scratch_power[k]
            + d_face_right * (scratch_power[k + 1] - scratch_power[k])
            - d_face_left  * (scratch_power[k]     - scratch_power[k - 1]);

        let p_new = p_new.max(0.0); // Numerical floor — diffusion of non-negative is non-negative.
        let mag_new = p_new.sqrt();
        let mix = (mix_c[k].clamp(0.0, 2.0)) * 0.5;

        let mag_old = scratch_mag[k];
        let scale_wet = if mag_old > EPS { mag_new / mag_old } else { 0.0 };
        let dry = bins[k];
        let wet = bins[k] * scale_wet;
        bins[k] = dry * (1.0 - mix) + wet * mix;
    }
}
```

- [ ] **Step 4: Wire dispatch in `process()` for Viscosity**

Replace the stub body of `SpectralModule::process` in `life.rs`:

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
    _physics: Option<&mut BinPhysics>,
    ctx: &ModuleContext<'_>,
) {
    debug_assert!(channel < 2);
    debug_assert_eq!(bins.len(), ctx.num_bins);

    let scratch_power = &mut self.scratch_power[channel];
    let scratch_mag = &mut self.scratch_mag[channel];

    match self.mode {
        LifeMode::Viscosity => {
            apply_viscosity(bins, scratch_power, scratch_mag, curves);
        }
        _ => {
            // Filled in Tasks 4-12.
        }
    }

    for s in suppression_out.iter_mut() {
        *s = 0.0;
    }
}
```

- [ ] **Step 5: Run test, expect pass**

Run: `cargo test --test module_trait life_viscosity_diffuses_and_conserves -- --nocapture`
Expected: PASS — total power within 5% of dry, energy spread to neighbours.

- [ ] **Step 6: Commit**

```bash
git add src/dsp/modules/life.rs tests/module_trait.rs
git commit -m "feat(life): Viscosity kernel — FTCS finite-volume diffusion on power"
```

---

## Task 4: Surface Tension kernel — adjacent peak coalescence

**Files:**
- Modify: `src/dsp/modules/life.rs` (add `apply_surface_tension` + dispatch)
- Test: `tests/module_trait.rs`

- [ ] **Step 1: Write the failing test**

Append to `tests/module_trait.rs`:

```rust
#[test]
fn life_surface_tension_coalesces_peaks() {
    use spectral_forge::dsp::modules::life::{LifeModule, LifeMode};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule, StereoLink, FxChannelTarget};
    use realfft::num_complex::Complex;

    let mut module = LifeModule::new();
    module.reset(48_000.0, 2048);
    module.set_mode_for_test(LifeMode::SurfaceTension);

    let num_bins = 1025;
    // A "noisy" cluster around bin 200: [180, 220] all = 1.0.
    let mut bins: Vec<Complex<f32>> = vec![Complex::new(0.0, 0.0); num_bins];
    for k in 180..=220 {
        bins[k] = Complex::new(1.0, 0.0);
    }
    let dry_total_mag: f32 = bins.iter().map(|b| b.norm()).sum();

    // AMOUNT=2 (max attract), THRESHOLD=0.5 (low — most bins qualify),
    // SPEED=neutral, REACH=2 (long reach), MIX=2 (full wet).
    let amount = vec![2.0_f32; num_bins];
    let thresh = vec![0.5_f32; num_bins];
    let neutral = vec![1.0_f32; num_bins];
    let reach = vec![2.0_f32; num_bins];
    let mix = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &thresh, &neutral, &reach, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = ModuleContext {
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
        bin_physics: None,
    };

    // 10 hops — coalescence is gradual.
    for _ in 0..10 {
        module.process(
            0, StereoLink::Linked, FxChannelTarget::All,
            &mut bins, None, &curves, &mut suppression, None, &ctx,
        );
    }

    let wet_total_mag: f32 = bins.iter().map(|b| b.norm()).sum();

    // Total magnitude approximately conserved (transport mode, < 10% loss for boundary effects).
    let loss_pct = (dry_total_mag - wet_total_mag).abs() / dry_total_mag;
    assert!(loss_pct < 0.10, "Surface Tension lost {}% of magnitude (>10%)", loss_pct * 100.0);

    // Variance must INCREASE (cluster becomes spikier — peaks taller, valleys deeper).
    let cluster: Vec<f32> = (180..=220).map(|k| bins[k].norm()).collect();
    let mean: f32 = cluster.iter().sum::<f32>() / cluster.len() as f32;
    let var: f32 = cluster.iter().map(|m| (m - mean).powi(2)).sum::<f32>() / cluster.len() as f32;
    assert!(var > 0.05, "Cluster did not coalesce (variance = {})", var);

    for b in &bins {
        assert!(b.norm().is_finite());
    }
}
```

- [ ] **Step 2: Run test, expect failure**

Run: `cargo test --test module_trait life_surface_tension_coalesces_peaks -- --nocapture`
Expected: FAIL — Surface Tension dispatch is in the `_` arm (no-op).

- [ ] **Step 3: Add the kernel**

In `src/dsp/modules/life.rs`, add above `impl SpectralModule`:

```rust
/// Adjacent peak attraction. Bins above THRESHOLD pull magnitude from neighbours
/// within ±REACH bins, weighted inversely by distance. Approximately conserves
/// total magnitude (transport, not creation).
fn apply_surface_tension(
    bins: &mut [Complex<f32>],
    scratch_mag: &mut [f32],
    curves: &[&[f32]],
) {
    let amount_c = curves[0];
    let thresh_c = curves[1];
    let reach_c = curves[3];
    let mix_c = curves[4];

    let num_bins = bins.len();

    // Pass 1: cache magnitudes.
    for k in 0..num_bins {
        scratch_mag[k] = bins[k].norm();
    }

    // Pass 2: for each bin above threshold, steal a tiny fraction from neighbours
    // within REACH (in bins).
    for k in 0..num_bins {
        let mag = scratch_mag[k];
        let thresh = (thresh_c[k] * 0.5).clamp(0.0, 2.0); // 0..1 magnitude
        if mag <= thresh {
            continue;
        }

        let amt = (amount_c[k] * 0.025).clamp(0.0, 0.05); // 5% per hop
        let reach_bins = ((reach_c[k] * 4.0) as i32).clamp(1, 8); // 1..8 bins

        let mut accum = 0.0_f32;
        for d in 1..=reach_bins {
            let kl = k as i32 - d;
            let kr = k as i32 + d;
            let weight = amt / d as f32; // 1/d falloff
            if kl >= 0 {
                let nb = scratch_mag[kl as usize];
                if nb < mag {
                    let take = nb * weight;
                    accum += take;
                    scratch_mag[kl as usize] -= take;
                }
            }
            if (kr as usize) < num_bins {
                let nb = scratch_mag[kr as usize];
                if nb < mag {
                    let take = nb * weight;
                    accum += take;
                    scratch_mag[kr as usize] -= take;
                }
            }
        }

        scratch_mag[k] = mag + accum;
    }

    // Pass 3: apply scratch_mag back to bins (preserving phase) with MIX blend.
    for k in 0..num_bins {
        let old_mag = bins[k].norm();
        let new_mag = scratch_mag[k].max(0.0);
        let scale_wet = if old_mag > 1e-9 { new_mag / old_mag } else { 0.0 };
        let mix = (mix_c[k].clamp(0.0, 2.0)) * 0.5;
        let dry = bins[k];
        let wet = bins[k] * scale_wet;
        bins[k] = dry * (1.0 - mix) + wet * mix;
    }
}
```

- [ ] **Step 4: Add dispatch arm**

In `process()`, replace the `_ =>` arm with:

```rust
LifeMode::SurfaceTension => {
    apply_surface_tension(bins, scratch_mag, curves);
}
_ => {
    // Filled in Tasks 5-12.
}
```

- [ ] **Step 5: Run test, expect pass**

Run: `cargo test --test module_trait life_surface_tension_coalesces_peaks -- --nocapture`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/dsp/modules/life.rs tests/module_trait.rs
git commit -m "feat(life): Surface Tension kernel — adjacent peak coalescence"
```

---

## Task 5: Crystallization kernel — sustain-driven phase lock + BinPhysics write

**Files:**
- Modify: `src/dsp/modules/life.rs` (add `apply_crystallization` + dispatch)
- Test: `tests/module_trait.rs`

- [ ] **Step 1: Write the failing test**

Append to `tests/module_trait.rs`:

```rust
#[test]
fn life_crystallization_writes_bin_physics() {
    use spectral_forge::dsp::modules::life::{LifeModule, LifeMode};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule, StereoLink, FxChannelTarget};
    use spectral_forge::dsp::bin_physics::BinPhysics;
    use realfft::num_complex::Complex;

    let mut module = LifeModule::new();
    module.reset(48_000.0, 2048);
    module.set_mode_for_test(LifeMode::Crystallization);

    let num_bins = 1025;
    // Sustained tone at bin 50 with magnitude 0.8.
    let bins_template: Vec<Complex<f32>> = {
        let mut v = vec![Complex::new(0.0, 0.0); num_bins];
        v[50] = Complex::new(0.8, 0.0);
        v
    };

    let amount = vec![2.0_f32; num_bins];
    let thresh = vec![0.5_f32; num_bins]; // Low → bin 50's mag (0.8) easily exceeds.
    let speed = vec![2.0_f32; num_bins];
    let neutral = vec![1.0_f32; num_bins];
    let mix = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &thresh, &speed, &neutral, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let mut physics = BinPhysics::new();
    physics.reset_active(num_bins, 48_000.0, 2048);
    let ctx = ModuleContext {
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
        bin_physics: None,
    };

    // 50 hops to let sustain envelope build at bin 50.
    let mut bins = bins_template.clone();
    for _ in 0..50 {
        module.process(
            0, StereoLink::Linked, FxChannelTarget::All,
            &mut bins, None, &curves, &mut suppression, Some(&mut physics), &ctx,
        );
        // Re-supply the input each hop (sustained tone simulation).
        bins = bins_template.clone();
    }

    // After many hops, BinPhysics.crystallization at bin 50 must be > 0.5.
    assert!(physics.crystallization[50] > 0.5,
        "crystallization[50] = {} (expected > 0.5 after 50 hops of sustain)",
        physics.crystallization[50]);

    // Quiet bins should still have crystallization ≈ 0.
    assert!(physics.crystallization[0] < 0.1);
    assert!(physics.crystallization[100] < 0.1);

    for b in &bins {
        assert!(b.norm().is_finite());
    }
}
```

- [ ] **Step 2: Run test, expect failure**

Run: `cargo test --test module_trait life_crystallization_writes_bin_physics -- --nocapture`
Expected: FAIL — Crystallization not yet wired.

- [ ] **Step 3: Add the kernel**

In `src/dsp/modules/life.rs`:

```rust
/// Sustained tonal bins build crystallization. Writes to BinPhysics.crystallization
/// for downstream readers (Freeze, future Life Crystallization stacking).
/// AMOUNT scales the crystallization growth rate; THRESHOLD is the magnitude floor
/// above which a bin counts as "sustained"; SPEED scales the LP decay (larger =
/// faster build/decay).
fn apply_crystallization(
    bins: &mut [Complex<f32>],
    sustain_envelope: &mut [f32],
    curves: &[&[f32]],
    physics: Option<&mut BinPhysics>,
    num_bins: usize,
) {
    let amount_c = curves[0];
    let thresh_c = curves[1];
    let speed_c = curves[2];
    let mix_c = curves[4];

    for k in 0..num_bins {
        let mag = bins[k].norm();
        let thresh = (thresh_c[k] * 0.5).clamp(0.0, 2.0);
        let speed = (speed_c[k] * 0.5).clamp(0.0, 1.0);
        let alpha = SUSTAIN_LP_ALPHA * (1.0 + speed * 4.0); // 0.05 .. 0.25

        // Update sustain envelope (slow LP).
        let sustained = if mag > thresh { 1.0 } else { 0.0 };
        sustain_envelope[k] = sustain_envelope[k] * (1.0 - alpha) + sustained * alpha;

        let amt = (amount_c[k] * 0.5).clamp(0.0, 1.0);
        let crystal_local = (sustain_envelope[k] * amt).clamp(0.0, 1.0);

        // Phase lock toward the bin's frozen phase (here: real axis, simplest).
        // Real-world: lock toward the bin's first observed phase per slot. v1 = real axis.
        let mix = (mix_c[k].clamp(0.0, 2.0)) * 0.5;
        let target = Complex::new(mag, 0.0);
        let locked = bins[k] * (1.0 - crystal_local) + target * crystal_local;
        bins[k] = bins[k] * (1.0 - mix) + locked * mix;
    }

    if let Some(p) = physics {
        for k in 0..num_bins {
            // Per-field merge rule for crystallization is `Max` — see Phase 3 BinPhysics.
            // We write the local crystallization; downstream consumers (Freeze) read it.
            p.crystallization[k] = p.crystallization[k].max(
                (sustain_envelope[k] * (amount_c[k] * 0.5).clamp(0.0, 1.0)).clamp(0.0, 1.0)
            );
        }
    }
}
```

- [ ] **Step 4: Add dispatch arm**

In `process()`, immediately after the `SurfaceTension` arm, before the `_` fallback, add:

```rust
LifeMode::Crystallization => {
    let sustain = &mut self.sustain_envelope[channel];
    apply_crystallization(bins, sustain, curves, _physics, ctx.num_bins);
}
```

Note: the `_physics` parameter must be renamed to `physics` (drop the underscore) in the `process()` signature so it can be passed through. Update the existing function signature in `life.rs`:

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
    physics: Option<&mut BinPhysics>,
    ctx: &ModuleContext<'_>,
) {
```

And pass `physics` into the Crystallization arm:

```rust
LifeMode::Crystallization => {
    let sustain = &mut self.sustain_envelope[channel];
    apply_crystallization(bins, sustain, curves, physics, ctx.num_bins);
}
```

- [ ] **Step 5: Run test, expect pass**

Run: `cargo test --test module_trait life_crystallization_writes_bin_physics -- --nocapture`
Expected: PASS — `crystallization[50]` builds above 0.5 after 50 hops.

- [ ] **Step 6: Commit**

```bash
git add src/dsp/modules/life.rs tests/module_trait.rs
git commit -m "feat(life): Crystallization kernel + BinPhysics write"
```

---

## Task 6: Archimedes kernel — volume-conserving global ducking

**Files:**
- Modify: `src/dsp/modules/life.rs` (add `apply_archimedes` + dispatch)
- Test: `tests/module_trait.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn life_archimedes_ducks_under_loud_volume() {
    use spectral_forge::dsp::modules::life::{LifeModule, LifeMode};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule, StereoLink, FxChannelTarget};
    use realfft::num_complex::Complex;

    let mut module = LifeModule::new();
    module.reset(48_000.0, 2048);
    module.set_mode_for_test(LifeMode::Archimedes);

    let num_bins = 1025;
    // High-volume signal: every bin = 1.0.
    let mut bins: Vec<Complex<f32>> = (0..num_bins).map(|_| Complex::new(1.0, 0.0)).collect();
    let dry_total: f32 = bins.iter().map(|b| b.norm()).sum();

    // AMOUNT=2 (max ducking), THRESHOLD=0.5 (low — pool fills easily),
    // SPEED=neutral, REACH=neutral, MIX=2 (full wet).
    let amount = vec![2.0_f32; num_bins];
    let thresh = vec![0.5_f32; num_bins];
    let neutral = vec![1.0_f32; num_bins];
    let mix = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &thresh, &neutral, &neutral, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = ModuleContext {
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
        bin_physics: None,
    };

    module.process(
        0, StereoLink::Linked, FxChannelTarget::All,
        &mut bins, None, &curves, &mut suppression, None, &ctx,
    );

    let wet_total: f32 = bins.iter().map(|b| b.norm()).sum();

    // Loud spectrum should be ducked: total magnitude must drop.
    assert!(wet_total < dry_total * 0.95,
        "Archimedes did not duck (dry={}, wet={})", dry_total, wet_total);

    // No bin should be NaN/Inf.
    for b in &bins {
        assert!(b.norm().is_finite());
    }
}
```

- [ ] **Step 2: Run test, expect failure**

Run: `cargo test --test module_trait life_archimedes_ducks_under_loud_volume -- --nocapture`
Expected: FAIL — Archimedes not wired.

- [ ] **Step 3: Add the kernel**

```rust
/// Volume-conserving ducking. Total spectral magnitude is treated as fluid volume;
/// when total exceeds capacity (controlled by THRESHOLD), bins are scaled down
/// proportionally. AMOUNT scales the displacement; THRESHOLD sets the capacity.
/// MIX blends wet/dry. Does not use BinPhysics directly.
fn apply_archimedes(
    bins: &mut [Complex<f32>],
    curves: &[&[f32]],
    num_bins: usize,
) {
    let amount_c = curves[0];
    let thresh_c = curves[1];
    let mix_c = curves[4];

    // Compute total magnitude (the "volume").
    let mut total_mag = 0.0_f32;
    for k in 0..num_bins {
        total_mag += bins[k].norm();
    }

    // Average AMOUNT and THRESHOLD across the band (curves are per-bin but the
    // effect is global — average reduces UI surprise).
    let mut sum_amt = 0.0_f32;
    let mut sum_thresh = 0.0_f32;
    for k in 0..num_bins {
        sum_amt += amount_c[k];
        sum_thresh += thresh_c[k];
    }
    let avg_amt = (sum_amt / num_bins as f32 * 0.5).clamp(0.0, 1.0);
    let avg_thresh = (sum_thresh / num_bins as f32 * 0.5).clamp(0.0, 2.0);

    // Capacity = num_bins × avg_thresh (target average magnitude per bin).
    let capacity = (num_bins as f32 * avg_thresh).max(1e-6);
    let overflow_ratio = (total_mag / capacity - 1.0).max(0.0);
    let duck_factor = 1.0 - (overflow_ratio * avg_amt).min(0.95);

    // Apply per-bin: scale wet by duck_factor, blend with dry via per-bin MIX.
    for k in 0..num_bins {
        let mix = (mix_c[k].clamp(0.0, 2.0)) * 0.5;
        let dry = bins[k];
        let wet = bins[k] * duck_factor;
        bins[k] = dry * (1.0 - mix) + wet * mix;
    }
}
```

- [ ] **Step 4: Add dispatch arm**

```rust
LifeMode::Archimedes => {
    apply_archimedes(bins, curves, ctx.num_bins);
}
```

- [ ] **Step 5: Run test, expect pass**

Run: `cargo test --test module_trait life_archimedes_ducks_under_loud_volume -- --nocapture`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/dsp/modules/life.rs tests/module_trait.rs
git commit -m "feat(life): Archimedes kernel — volume-conserving ducking"
```

---

## Task 7: Non-Newtonian kernel — rate-limit transients

**Files:**
- Modify: `src/dsp/modules/life.rs` (add `apply_non_newtonian` + dispatch)
- Test: `tests/module_trait.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn life_non_newtonian_limits_fast_transients() {
    use spectral_forge::dsp::modules::life::{LifeModule, LifeMode};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule, StereoLink, FxChannelTarget};
    use spectral_forge::dsp::bin_physics::BinPhysics;
    use realfft::num_complex::Complex;

    let mut module = LifeModule::new();
    module.reset(48_000.0, 2048);
    module.set_mode_for_test(LifeMode::NonNewtonian);

    let num_bins = 1025;
    let mut bins: Vec<Complex<f32>> = vec![Complex::new(0.0, 0.0); num_bins];
    bins[100] = Complex::new(2.0, 0.0); // Loud transient

    // Simulate a fast-changing input by setting velocity[100] = 1.5 (large delta).
    let mut physics = BinPhysics::new();
    physics.reset_active(num_bins, 48_000.0, 2048);
    physics.velocity[100] = 1.5;

    // Build ctx with bin_physics READ slot pointing at physics.
    // (In production Pipeline does this; we hand-wire for the test.)
    let physics_view: &BinPhysics = &physics;

    let amount = vec![2.0_f32; num_bins];
    let thresh = vec![0.5_f32; num_bins]; // velocity 1.5 > 0.5*0.5=0.25 threshold → solidify
    let neutral = vec![1.0_f32; num_bins];
    let mix = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &thresh, &neutral, &neutral, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let mut physics_for_write = BinPhysics::new();
    physics_for_write.reset_active(num_bins, 48_000.0, 2048);
    let ctx = ModuleContext {
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
        bin_physics: Some(physics_view),
    };

    module.process(
        0, StereoLink::Linked, FxChannelTarget::All,
        &mut bins, None, &curves, &mut suppression, Some(&mut physics_for_write), &ctx,
    );

    // Bin 100 must be limited (magnitude reduced from 2.0).
    assert!(bins[100].norm() < 2.0,
        "Non-Newtonian did not limit transient (mag = {})", bins[100].norm());
    // Quiet bins must passthrough.
    assert!(bins[0].norm() < 1e-6);
    // displacement[100] should be written (non-zero).
    assert!(physics_for_write.displacement[100] > 0.0,
        "Non-Newtonian did not write displacement");

    for b in &bins {
        assert!(b.norm().is_finite());
    }
}
```

- [ ] **Step 2: Run test, expect failure**

Run: `cargo test --test module_trait life_non_newtonian_limits_fast_transients -- --nocapture`
Expected: FAIL — NonNewtonian not wired.

- [ ] **Step 3: Add the kernel**

```rust
/// Oobleck — solidifies under fast amplitude changes (large velocity), passes
/// slow changes freely. Reads `BinPhysics.velocity` (auto-computed by Pipeline).
/// Writes `BinPhysics.displacement` so downstream Stiction/Yield can react.
fn apply_non_newtonian(
    bins: &mut [Complex<f32>],
    curves: &[&[f32]],
    velocity: Option<&[f32]>,
    physics_out: Option<&mut BinPhysics>,
    num_bins: usize,
) {
    let amount_c = curves[0];
    let thresh_c = curves[1];
    let mix_c = curves[4];

    let vel = velocity; // None falls through to passthrough.
    let mut displacement_writes = vec![0.0_f32; 0]; // sentinel: not used
    let _ = displacement_writes; // placeholder reference; real write is in the loop below

    for k in 0..num_bins {
        let v = vel.map(|vs| vs[k]).unwrap_or(0.0);
        let thresh = (thresh_c[k] * 0.5).clamp(0.0, 2.0);
        let amt = (amount_c[k] * 0.5).clamp(0.0, 1.0);

        if v > thresh {
            // Solidify: limit |bin| to current_mag - excess × amt
            let excess = v - thresh;
            let limit = (bins[k].norm() - excess * amt).max(0.0);
            let mag_old = bins[k].norm();
            let scale = if mag_old > 1e-9 { limit / mag_old } else { 0.0 };
            let mix = (mix_c[k].clamp(0.0, 2.0)) * 0.5;
            let dry = bins[k];
            let wet = bins[k] * scale;
            bins[k] = dry * (1.0 - mix) + wet * mix;
        }
    }

    // Write displacement = velocity above threshold (single pass).
    if let Some(p) = physics_out {
        for k in 0..num_bins {
            let v = vel.map(|vs| vs[k]).unwrap_or(0.0);
            let thresh = (thresh_c[k] * 0.5).clamp(0.0, 2.0);
            if v > thresh {
                p.displacement[k] = (p.displacement[k] + (v - thresh)).min(10.0);
            }
        }
    }
}
```

- [ ] **Step 4: Add dispatch arm**

```rust
LifeMode::NonNewtonian => {
    let velocity = ctx.bin_physics.map(|bp| &bp.velocity[..ctx.num_bins]);
    apply_non_newtonian(bins, curves, velocity, physics, ctx.num_bins);
}
```

- [ ] **Step 5: Run test, expect pass**

Run: `cargo test --test module_trait life_non_newtonian_limits_fast_transients -- --nocapture`
Expected: PASS — bin 100 limited, displacement written.

- [ ] **Step 6: Commit**

```bash
git add src/dsp/modules/life.rs tests/module_trait.rs
git commit -m "feat(life): Non-Newtonian kernel — velocity-driven transient limiter"
```

---

## Task 8: Stiction kernel — static/kinetic friction

**Files:**
- Modify: `src/dsp/modules/life.rs` (add `apply_stiction` + dispatch)
- Test: `tests/module_trait.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn life_stiction_holds_quiet_bins_then_releases() {
    use spectral_forge::dsp::modules::life::{LifeModule, LifeMode};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule, StereoLink, FxChannelTarget};
    use spectral_forge::dsp::bin_physics::BinPhysics;
    use realfft::num_complex::Complex;

    let mut module = LifeModule::new();
    module.reset(48_000.0, 2048);
    module.set_mode_for_test(LifeMode::Stiction);

    let num_bins = 1025;
    // Bin 50: small change (below static friction → stuck).
    // Bin 100: large change (above static friction → moves).
    let mut bins: Vec<Complex<f32>> = vec![Complex::new(0.0, 0.0); num_bins];
    bins[50] = Complex::new(0.1, 0.0);
    bins[100] = Complex::new(1.5, 0.0);

    let mut physics = BinPhysics::new();
    physics.reset_active(num_bins, 48_000.0, 2048);
    physics.velocity[50] = 0.1;
    physics.velocity[100] = 1.0;

    let amount = vec![2.0_f32; num_bins];
    let thresh = vec![1.0_f32; num_bins]; // → 0.5 break threshold
    let speed = vec![1.0_f32; num_bins];
    let neutral = vec![1.0_f32; num_bins];
    let mix = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &thresh, &speed, &neutral, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let mut physics_for_write = BinPhysics::new();
    physics_for_write.reset_active(num_bins, 48_000.0, 2048);
    let ctx = ModuleContext {
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
        bin_physics: Some(&physics),
    };

    module.process(
        0, StereoLink::Linked, FxChannelTarget::All,
        &mut bins, None, &curves, &mut suppression, Some(&mut physics_for_write), &ctx,
    );

    // Bin 50 (low velocity, below threshold): stuck — magnitude reduced toward 0.
    assert!(bins[50].norm() < 0.05,
        "Bin 50 not stuck (mag = {})", bins[50].norm());

    // Bin 100 (high velocity, above threshold): kinetic — passes through close to dry.
    assert!(bins[100].norm() > 1.0,
        "Bin 100 not moving freely (mag = {})", bins[100].norm());

    for b in &bins {
        assert!(b.norm().is_finite());
    }
}
```

- [ ] **Step 2: Run test, expect failure**

Run: `cargo test --test module_trait life_stiction_holds_quiet_bins_then_releases -- --nocapture`
Expected: FAIL — Stiction not wired.

- [ ] **Step 3: Add the kernel**

```rust
/// Static + kinetic friction. Bins below THRESHOLD velocity are "stuck" — they
/// decay to zero. Bins above THRESHOLD are "moving" — passthrough. Once moving,
/// they stay moving for SPEED hops before re-locking.
fn apply_stiction(
    bins: &mut [Complex<f32>],
    is_moving: &mut [f32],
    curves: &[&[f32]],
    velocity: Option<&[f32]>,
    physics_out: Option<&mut BinPhysics>,
    num_bins: usize,
) {
    let amount_c = curves[0];
    let thresh_c = curves[1];
    let speed_c = curves[2];
    let mix_c = curves[4];

    for k in 0..num_bins {
        let v = velocity.map(|vs| vs[k]).unwrap_or(0.0);
        let thresh = (thresh_c[k] * 0.5).clamp(0.0, 2.0);
        let speed = (speed_c[k] * 0.5).clamp(0.0, 1.0);

        if v > thresh {
            is_moving[k] = 1.0; // Break free.
        } else {
            // Decay the moving flag at SPEED rate (small SPEED = sticks fast).
            let decay = 0.05 + speed * 0.45; // 0.05..0.5 per hop
            is_moving[k] = (is_moving[k] - decay).max(0.0);
        }

        let amt = (amount_c[k] * 0.5).clamp(0.0, 1.0);
        let stuck_factor = 1.0 - (1.0 - is_moving[k]) * amt; // 1.0 = free, 1-amt = fully stuck

        let mix = (mix_c[k].clamp(0.0, 2.0)) * 0.5;
        let dry = bins[k];
        let wet = bins[k] * stuck_factor;
        bins[k] = dry * (1.0 - mix) + wet * mix;
    }

    if let Some(p) = physics_out {
        for k in 0..num_bins {
            // Stiction's contribution to displacement: distance the bin "wants" to
            // move but cannot. Approximated as (1 - is_moving) × velocity.
            let v = velocity.map(|vs| vs[k]).unwrap_or(0.0);
            let stuck = (1.0 - is_moving[k]).clamp(0.0, 1.0);
            p.displacement[k] = (p.displacement[k] + stuck * v).min(10.0);
        }
    }
}
```

- [ ] **Step 4: Add dispatch arm**

```rust
LifeMode::Stiction => {
    let velocity = ctx.bin_physics.map(|bp| &bp.velocity[..ctx.num_bins]);
    let is_moving = &mut self.is_moving[channel];
    apply_stiction(bins, is_moving, curves, velocity, physics, ctx.num_bins);
}
```

- [ ] **Step 5: Run test, expect pass**

Run: `cargo test --test module_trait life_stiction_holds_quiet_bins_then_releases -- --nocapture`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/dsp/modules/life.rs tests/module_trait.rs
git commit -m "feat(life): Stiction kernel — static/kinetic friction"
```

---

## Task 9: Yield kernel — fabric tearing with phase scramble

**Files:**
- Modify: `src/dsp/modules/life.rs` (add `apply_yield` + dispatch)
- Test: `tests/module_trait.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn life_yield_freezes_at_threshold_and_heals() {
    use spectral_forge::dsp::modules::life::{LifeModule, LifeMode};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule, StereoLink, FxChannelTarget};
    use realfft::num_complex::Complex;

    let mut module = LifeModule::new();
    module.reset(48_000.0, 2048);
    module.set_mode_for_test(LifeMode::Yield);

    let num_bins = 1025;
    // Bin 50: above yield threshold (mag 2.0).
    // Bin 100: below threshold (mag 0.2).
    let mut bins: Vec<Complex<f32>> = vec![Complex::new(0.0, 0.0); num_bins];
    bins[50] = Complex::new(2.0, 0.0);
    bins[100] = Complex::new(0.2, 0.0);

    let amount = vec![2.0_f32; num_bins];
    let thresh = vec![1.0_f32; num_bins]; // yield strength = 0.5
    let speed = vec![0.5_f32; num_bins];  // slow heal
    let neutral = vec![1.0_f32; num_bins];
    let mix = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &thresh, &speed, &neutral, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = ModuleContext {
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
        bin_physics: None,
    };

    module.process(
        0, StereoLink::Linked, FxChannelTarget::All,
        &mut bins, None, &curves, &mut suppression, None, &ctx,
    );

    // Bin 50 (above yield) must be clamped at or below the yield threshold (0.5).
    assert!(bins[50].norm() <= 0.6,
        "Bin 50 not clamped at yield (mag = {})", bins[50].norm());

    // Bin 100 (below yield) passes through.
    assert!((bins[100].norm() - 0.2).abs() < 0.01,
        "Bin 100 not passthrough (mag = {})", bins[100].norm());

    for b in &bins {
        assert!(b.norm().is_finite());
    }
}
```

- [ ] **Step 2: Run test, expect failure**

Run: `cargo test --test module_trait life_yield_freezes_at_threshold_and_heals -- --nocapture`
Expected: FAIL — Yield not wired.

- [ ] **Step 3: Add the kernel**

```rust
/// Fabric tearing. Bins below THRESHOLD are elastic (passthrough). Above THRESHOLD,
/// the bin "tears": magnitude is clamped at the yield level and phase is scrambled.
/// Heals at SPEED rate. Writes `BinPhysics.bias` (cumulative stress) for downstream.
fn apply_yield(
    bins: &mut [Complex<f32>],
    tear_state: &mut [f32],
    rng_state: &mut u32,
    curves: &[&[f32]],
    physics_out: Option<&mut BinPhysics>,
    num_bins: usize,
) {
    let amount_c = curves[0];
    let thresh_c = curves[1];
    let speed_c = curves[2];
    let mix_c = curves[4];

    for k in 0..num_bins {
        let mag = bins[k].norm();
        let thresh = (thresh_c[k] * 0.5).clamp(0.0, 2.0);
        let amt = (amount_c[k] * 0.5).clamp(0.0, 1.0);
        let speed = (speed_c[k] * 0.5).clamp(0.0, 1.0);
        let heal_rate = 0.005 + speed * 0.045; // 0.005..0.05 per hop

        if mag > thresh {
            tear_state[k] = 1.0;
        } else {
            tear_state[k] = (tear_state[k] - heal_rate).max(0.0);
        }

        let mix = (mix_c[k].clamp(0.0, 2.0)) * 0.5;

        if tear_state[k] > 0.0 && mag > 1e-9 {
            // Clamp magnitude at yield + scramble phase by random rotation.
            let yield_mag = thresh.min(mag);
            let phase_scramble = xorshift32_signed_unit(rng_state) * std::f32::consts::PI;
            let new_re = yield_mag * phase_scramble.cos();
            let new_im = yield_mag * phase_scramble.sin();
            // Blend wet (torn) toward dry as tear_state heals (1.0 = fully torn, 0.0 = elastic).
            let torn_strength = tear_state[k] * amt;
            let wet = Complex::new(new_re, new_im);
            let elastic = bins[k];
            let result = elastic * (1.0 - torn_strength) + wet * torn_strength;
            bins[k] = bins[k] * (1.0 - mix) + result * mix;
        }
    }

    if let Some(p) = physics_out {
        for k in 0..num_bins {
            let mag = bins[k].norm();
            let thresh = (thresh_c[k] * 0.5).clamp(0.0, 2.0);
            if mag > thresh {
                // Write to bias: cumulative tear stress.
                p.bias[k] = (p.bias[k] + (mag - thresh)).min(10.0);
            }
        }
    }
}
```

- [ ] **Step 4: Add dispatch arm**

```rust
LifeMode::Yield => {
    let tear = &mut self.tear_state[channel];
    let rng = &mut self.rng_state[channel];
    apply_yield(bins, tear, rng, curves, physics, ctx.num_bins);
}
```

- [ ] **Step 5: Run test, expect pass**

Run: `cargo test --test module_trait life_yield_freezes_at_threshold_and_heals -- --nocapture`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/dsp/modules/life.rs tests/module_trait.rs
git commit -m "feat(life): Yield kernel — fabric tearing with phase scramble"
```

---

## Task 10: Capillary kernel — upward harmonic wicking (two-pass)

**Files:**
- Modify: `src/dsp/modules/life.rs` (add `apply_capillary` + dispatch)
- Test: `tests/module_trait.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn life_capillary_wicks_sustained_energy_upward() {
    use spectral_forge::dsp::modules::life::{LifeModule, LifeMode};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule, StereoLink, FxChannelTarget};
    use realfft::num_complex::Complex;

    let mut module = LifeModule::new();
    module.reset(48_000.0, 2048);
    module.set_mode_for_test(LifeMode::Capillary);

    let num_bins = 1025;
    // Sustained low-frequency tone at bin 50.
    let bins_template: Vec<Complex<f32>> = {
        let mut v = vec![Complex::new(0.0, 0.0); num_bins];
        v[50] = Complex::new(1.0, 0.0);
        v
    };

    let amount = vec![2.0_f32; num_bins];
    let thresh = vec![0.5_f32; num_bins]; // Low threshold → bin 50 always sustained.
    let speed = vec![2.0_f32; num_bins];
    let reach = vec![2.0_f32; num_bins]; // Long upward reach.
    let mix = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &thresh, &speed, &reach, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = ModuleContext {
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
        bin_physics: None,
    };

    let mut bins = bins_template.clone();
    // Run many hops to let the wick build up.
    for _ in 0..100 {
        module.process(
            0, StereoLink::Linked, FxChannelTarget::All,
            &mut bins, None, &curves, &mut suppression, None, &ctx,
        );
        bins = bins_template.clone(); // re-supply input
    }

    // After long sustain, energy should appear in higher bins (wicked upward).
    let upper_total: f32 = (60..200).map(|k| bins[k].norm()).sum();
    assert!(upper_total > 0.05,
        "No upward wicking happened (upper_total = {})", upper_total);

    for b in &bins {
        assert!(b.norm().is_finite());
    }
}
```

- [ ] **Step 2: Run test, expect failure**

Run: `cargo test --test module_trait life_capillary_wicks_sustained_energy_upward -- --nocapture`
Expected: FAIL — Capillary not wired.

- [ ] **Step 3: Add the kernel**

```rust
/// Upward harmonic wicking. Sustained loud bins slowly leak magnitude to higher
/// (quieter) bins — like water climbing a paper towel. Two-pass: drain source,
/// deposit at target. AMOUNT = leak rate, REACH = number of bins upward,
/// SPEED = LP envelope rate, THRESHOLD = sustain level required to start wicking.
fn apply_capillary(
    bins: &mut [Complex<f32>],
    sustain_envelope: &mut [f32],
    wick_carry: &mut [f32],
    scratch_mag: &mut [f32],
    curves: &[&[f32]],
    num_bins: usize,
) {
    let amount_c = curves[0];
    let thresh_c = curves[1];
    let speed_c = curves[2];
    let reach_c = curves[3];
    let mix_c = curves[4];

    // Pass 1: cache magnitudes, update sustain envelopes, compute drains.
    for k in 0..num_bins {
        scratch_mag[k] = bins[k].norm();
        let speed = (speed_c[k] * 0.5).clamp(0.0, 1.0);
        let alpha = SUSTAIN_LP_ALPHA * (1.0 + speed * 4.0);
        let inst = if scratch_mag[k] > (thresh_c[k] * 0.5).clamp(0.0, 2.0) { 1.0 } else { 0.0 };
        sustain_envelope[k] = sustain_envelope[k] * (1.0 - alpha) + inst * alpha;
        // Reset wick_carry from previous hop — accumulator only.
        wick_carry[k] = 0.0;
    }

    // Pass 2: drain at source, accumulate at target.
    for k in 0..num_bins {
        let amt = (amount_c[k] * 0.025).clamp(0.0, 0.05); // 5% per hop max
        let reach_bins = ((reach_c[k] * 16.0) as i32).clamp(1, 32);
        let drain = scratch_mag[k] * amt * sustain_envelope[k];
        let target = (k as i32 + reach_bins).clamp(0, num_bins as i32 - 1) as usize;
        scratch_mag[k] -= drain;
        wick_carry[target] += drain;
    }

    // Pass 3: apply final magnitudes (with carry) to bins.
    for k in 0..num_bins {
        let new_mag = (scratch_mag[k] + wick_carry[k]).max(0.0);
        let old_mag = bins[k].norm();
        let scale_wet = if old_mag > 1e-9 {
            new_mag / old_mag
        } else if new_mag > 0.0 {
            // Bin was silent; deposit becomes a real-axis tone (phase = 0).
            // We can't just scale, so blend in a fresh complex at deposit magnitude.
            0.0
        } else {
            0.0
        };
        let mix = (mix_c[k].clamp(0.0, 2.0)) * 0.5;
        let dry = bins[k];
        let wet = if old_mag > 1e-9 {
            bins[k] * scale_wet
        } else {
            Complex::new(new_mag, 0.0)
        };
        bins[k] = dry * (1.0 - mix) + wet * mix;
    }
}
```

- [ ] **Step 4: Add dispatch arm**

```rust
LifeMode::Capillary => {
    let sustain = &mut self.sustain_envelope[channel];
    let wick = &mut self.wick_carry[channel];
    apply_capillary(bins, sustain, wick, scratch_mag, curves, ctx.num_bins);
}
```

- [ ] **Step 5: Run test, expect pass**

Run: `cargo test --test module_trait life_capillary_wicks_sustained_energy_upward -- --nocapture`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/dsp/modules/life.rs tests/module_trait.rs
git commit -m "feat(life): Capillary kernel — upward harmonic wicking (two-pass)"
```

---

## Task 11: Sandpaper kernel — granular phase friction sparks

**Files:**
- Modify: `src/dsp/modules/life.rs` (add `apply_sandpaper` + dispatch)
- Test: `tests/module_trait.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn life_sandpaper_emits_sparks_to_higher_bins() {
    use spectral_forge::dsp::modules::life::{LifeModule, LifeMode};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule, StereoLink, FxChannelTarget};
    use realfft::num_complex::Complex;

    let mut module = LifeModule::new();
    module.reset(48_000.0, 2048);
    module.set_mode_for_test(LifeMode::Sandpaper);

    let num_bins = 1025;
    // Two adjacent bins with phases ~180° apart and high magnitude → max friction.
    let mut bins: Vec<Complex<f32>> = vec![Complex::new(0.0, 0.0); num_bins];
    bins[100] = Complex::new(1.0, 0.0); // phase = 0
    bins[101] = Complex::new(-1.0, 0.0); // phase = π

    let amount = vec![2.0_f32; num_bins];
    let thresh = vec![0.1_f32; num_bins]; // Low → sparks easily emitted.
    let neutral = vec![1.0_f32; num_bins];
    let reach = vec![2.0_f32; num_bins]; // Long offset.
    let mix = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &thresh, &neutral, &reach, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = ModuleContext {
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
        bin_physics: None,
    };

    module.process(
        0, StereoLink::Linked, FxChannelTarget::All,
        &mut bins, None, &curves, &mut suppression, None, &ctx,
    );

    // Sparks should appear above bin 110 (logarithmic upward offset from 100).
    let upper_total: f32 = (110..num_bins).map(|k| bins[k].norm()).sum();
    assert!(upper_total > 0.01,
        "No sparks emitted upward (upper_total = {})", upper_total);

    for b in &bins {
        assert!(b.norm().is_finite());
    }
}
```

- [ ] **Step 2: Run test, expect failure**

Run: `cargo test --test module_trait life_sandpaper_emits_sparks_to_higher_bins -- --nocapture`
Expected: FAIL — Sandpaper not wired.

- [ ] **Step 3: Add the kernel**

```rust
/// Granular friction. Adjacent bins with high magnitude and large phase mismatch
/// "rub" against each other, emitting tiny sparks at logarithmically-offset
/// higher bins. AMOUNT = spark strength, THRESHOLD = phase mismatch floor (radians),
/// REACH = upward offset multiplier, MIX = wet/dry blend.
fn apply_sandpaper(
    bins: &mut [Complex<f32>],
    scratch_mag: &mut [f32],
    curves: &[&[f32]],
    num_bins: usize,
) {
    use std::f32::consts::PI;

    let amount_c = curves[0];
    let thresh_c = curves[1];
    let reach_c = curves[3];
    let mix_c = curves[4];

    // Cache initial magnitudes so we don't read sparks we wrote in the same pass.
    for k in 0..num_bins {
        scratch_mag[k] = bins[k].norm();
    }

    // For each adjacent pair, compute spark + deposit at logarithmic offset.
    for k in 0..num_bins - 1 {
        let m_left = scratch_mag[k];
        let m_right = scratch_mag[k + 1];
        if m_left < 1e-6 || m_right < 1e-6 {
            continue;
        }
        let phase_left = bins[k].arg();
        let phase_right = bins[k + 1].arg();
        let mut diff = (phase_right - phase_left).abs();
        if diff > PI {
            diff = 2.0 * PI - diff;
        }

        let thresh = (thresh_c[k] * PI * 0.5).clamp(0.0, PI);
        if diff <= thresh {
            continue;
        }

        let amt = (amount_c[k] * 0.05).clamp(0.0, 0.1);
        let mag_avg = 0.5 * (m_left + m_right);
        let spark = mag_avg * amt * (diff / PI);

        let reach_factor = reach_c[k].clamp(0.0, 2.0);
        let log_offset = ((1.0 + reach_factor) * (k as f32).max(1.0).log2() * 1.5) as usize;
        let target = (k + log_offset.max(2)).min(num_bins - 1);

        let mix = (mix_c[target].clamp(0.0, 2.0)) * 0.5;
        let cur = bins[target];
        let cur_mag = cur.norm();
        let new_mag = (cur_mag + spark).min(10.0); // hard cap to avoid clipping
        let scale = if cur_mag > 1e-9 {
            new_mag / cur_mag
        } else {
            0.0
        };
        let wet = if cur_mag > 1e-9 {
            cur * scale
        } else {
            Complex::new(spark, 0.0)
        };
        bins[target] = cur * (1.0 - mix) + wet * mix;
    }
}
```

- [ ] **Step 4: Add dispatch arm**

```rust
LifeMode::Sandpaper => {
    apply_sandpaper(bins, scratch_mag, curves, ctx.num_bins);
}
```

- [ ] **Step 5: Run test, expect pass**

Run: `cargo test --test module_trait life_sandpaper_emits_sparks_to_higher_bins -- --nocapture`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/dsp/modules/life.rs tests/module_trait.rs
git commit -m "feat(life): Sandpaper kernel — granular phase friction sparks"
```

---

## Task 12: Brownian kernel — temperature-driven random walk

**Files:**
- Modify: `src/dsp/modules/life.rs` (add `apply_brownian` + dispatch)
- Test: `tests/module_trait.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn life_brownian_drifts_with_temperature() {
    use spectral_forge::dsp::modules::life::{LifeModule, LifeMode};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule, StereoLink, FxChannelTarget};
    use spectral_forge::dsp::bin_physics::BinPhysics;
    use realfft::num_complex::Complex;

    let mut module = LifeModule::new();
    module.reset(48_000.0, 2048);
    module.set_mode_for_test(LifeMode::Brownian);

    let num_bins = 1025;
    let bins_template: Vec<Complex<f32>> = (0..num_bins).map(|_| Complex::new(0.5, 0.0)).collect();

    // Set bin 100's temperature to 1.0; rest zero.
    let mut physics = BinPhysics::new();
    physics.reset_active(num_bins, 48_000.0, 2048);
    physics.temperature[100] = 1.0;

    let amount = vec![2.0_f32; num_bins];
    let neutral = vec![1.0_f32; num_bins];
    let mix = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &neutral, &neutral, &neutral, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = ModuleContext {
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
        bin_physics: Some(&physics),
    };

    let mut bins = bins_template.clone();
    module.process(
        0, StereoLink::Linked, FxChannelTarget::All,
        &mut bins, None, &curves, &mut suppression, None, &ctx,
    );

    // Bin 100 (high temp) must drift away from the 0.5 baseline.
    let drift_100 = (bins[100] - bins_template[100]).norm();
    // Bin 0 (zero temp) must NOT drift.
    let drift_0 = (bins[0] - bins_template[0]).norm();

    assert!(drift_100 > 0.001,
        "Bin 100 did not drift (drift = {})", drift_100);
    assert!(drift_0 < 1e-6,
        "Bin 0 drifted despite temp=0 (drift = {})", drift_0);

    for b in &bins {
        assert!(b.norm().is_finite());
    }
}
```

- [ ] **Step 2: Run test, expect failure**

Run: `cargo test --test module_trait life_brownian_drifts_with_temperature -- --nocapture`
Expected: FAIL — Brownian not wired.

- [ ] **Step 3: Add the kernel**

```rust
/// Temperature-driven random walk. Reads `BinPhysics.temperature[k]` (set by
/// upstream Circuit/Kinetics modules) and applies a small random drift to each
/// bin scaled by AMOUNT × temperature. Without an upstream temperature writer,
/// this mode is a no-op (multiplies by 0).
fn apply_brownian(
    bins: &mut [Complex<f32>],
    rng_state: &mut u32,
    curves: &[&[f32]],
    temperature: Option<&[f32]>,
    num_bins: usize,
) {
    let amount_c = curves[0];
    let mix_c = curves[4];

    let temp = temperature; // None → no-op

    for k in 0..num_bins {
        let t = temp.map(|ts| ts[k]).unwrap_or(0.0);
        if t <= 1e-6 {
            continue;
        }
        let amt = (amount_c[k] * 0.5).clamp(0.0, 1.0);
        let drift_re = xorshift32_signed_unit(rng_state) * amt * t * 0.1;
        let drift_im = xorshift32_signed_unit(rng_state) * amt * t * 0.1;
        let mix = (mix_c[k].clamp(0.0, 2.0)) * 0.5;
        let dry = bins[k];
        let wet = bins[k] + Complex::new(drift_re, drift_im);
        bins[k] = dry * (1.0 - mix) + wet * mix;
    }
}
```

- [ ] **Step 4: Add dispatch arm**

```rust
LifeMode::Brownian => {
    let temp = ctx.bin_physics.map(|bp| &bp.temperature[..ctx.num_bins]);
    let rng = &mut self.rng_state[channel];
    apply_brownian(bins, rng, curves, temp, ctx.num_bins);
}
```

- [ ] **Step 5: Run test, expect pass**

Run: `cargo test --test module_trait life_brownian_drifts_with_temperature -- --nocapture`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/dsp/modules/life.rs tests/module_trait.rs
git commit -m "feat(life): Brownian kernel — temperature-driven random walk"
```

---

## Task 13: Per-slot mode persistence — params, FxMatrix, lib.rs

**Files:**
- Modify: `src/params.rs` (add `slot_life_mode: [Arc<Mutex<LifeMode>>; MAX_SLOTS]`)
- Modify: `src/dsp/fx_matrix.rs` (add `slot_life_modes: [LifeMode; MAX_SLOTS]`, `set_life_modes()`)
- Modify: `src/dsp/modules/mod.rs` (`SpectralModule::set_life_mode()` default no-op + impl in `LifeModule`)
- Modify: `src/lib.rs` (snapshot + dispatch each block)
- Test: `tests/module_trait.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn life_set_mode_persists_across_calls() {
    use spectral_forge::dsp::modules::life::{LifeModule, LifeMode};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule, StereoLink, FxChannelTarget};
    use realfft::num_complex::Complex;

    let mut module = LifeModule::new();
    module.reset(48_000.0, 2048);
    module.set_life_mode(LifeMode::Yield);

    let num_bins = 1025;
    let mut bins: Vec<Complex<f32>> = vec![Complex::new(2.0, 0.0); num_bins];

    let amount = vec![2.0_f32; num_bins];
    let thresh = vec![1.0_f32; num_bins]; // yield = 0.5 → mag 2.0 tears
    let neutral = vec![1.0_f32; num_bins];
    let mix = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &thresh, &neutral, &neutral, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = ModuleContext {
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
        bin_physics: None,
    };

    module.process(
        0, StereoLink::Linked, FxChannelTarget::All,
        &mut bins, None, &curves, &mut suppression, None, &ctx,
    );

    // After Yield, all bins must be at or below the yield threshold (0.5).
    for k in 0..num_bins {
        assert!(bins[k].norm() <= 0.6,
            "Bin {} not yielded (mag = {}); set_life_mode did not persist", k, bins[k].norm());
    }
}
```

- [ ] **Step 2: Run test, expect failure**

Run: `cargo test --test module_trait life_set_mode_persists_across_calls -- --nocapture`
Expected: FAIL — `set_life_mode` not yet on the trait.

- [ ] **Step 3: Add the trait method**

In `src/dsp/modules/mod.rs`, in the `SpectralModule` trait, add (next to other `set_*_mode` defaults):

```rust
fn set_life_mode(&mut self, _mode: crate::dsp::modules::life::LifeMode) {}
```

In `src/dsp/modules/life.rs`, override the default for `LifeModule`:

```rust
impl SpectralModule for LifeModule {
    // ...existing methods...

    fn set_life_mode(&mut self, mode: LifeMode) {
        self.set_mode(mode);
    }
}
```

- [ ] **Step 4: Run test, expect pass**

Run: `cargo test --test module_trait life_set_mode_persists_across_calls -- --nocapture`
Expected: PASS.

- [ ] **Step 5: Wire `slot_life_mode` in `params.rs`**

In `src/params.rs`, alongside `slot_geometry_mode` (and other per-slot mode persistents):

```rust
#[persist = "slot_life_mode"]
pub slot_life_mode: [Arc<Mutex<crate::dsp::modules::life::LifeMode>>; MAX_SLOTS],
```

In `Default`:

```rust
slot_life_mode: std::array::from_fn(|_| Arc::new(Mutex::new(crate::dsp::modules::life::LifeMode::default()))),
```

- [ ] **Step 6: Wire `slot_life_modes` in `FxMatrix`**

In `src/dsp/fx_matrix.rs`:

```rust
pub struct FxMatrix {
    // ...existing fields...
    slot_life_modes: [crate::dsp::modules::life::LifeMode; MAX_SLOTS],
}

impl FxMatrix {
    pub fn new() -> Self {
        // ...existing init...
        slot_life_modes: [crate::dsp::modules::life::LifeMode::default(); MAX_SLOTS],
    }

    pub fn set_life_modes(&mut self, modes: [crate::dsp::modules::life::LifeMode; MAX_SLOTS]) {
        for s in 0..MAX_SLOTS {
            if self.slot_life_modes[s] != modes[s] {
                self.slot_life_modes[s] = modes[s];
                if let Some(slot) = self.slots[s].as_mut() {
                    slot.set_life_mode(modes[s]);
                }
            }
        }
    }
}
```

Also: in the slot-construction path (when a new module is assigned to a slot), call `module.set_life_mode(self.slot_life_modes[s])` so the reassigned slot inherits the persisted mode.

- [ ] **Step 7: Snapshot per-block in `lib.rs`**

In `src/lib.rs::process()`, alongside other per-block mode snapshots:

```rust
let life_modes_snap: [crate::dsp::modules::life::LifeMode; MAX_SLOTS] =
    std::array::from_fn(|s| {
        params.slot_life_mode[s].try_lock().map(|m| *m).unwrap_or_default()
    });
fx_matrix.set_life_modes(life_modes_snap);
```

- [ ] **Step 8: Run all tests**

Run: `cargo test`
Expected: PASS — all existing tests + new Life tests.

- [ ] **Step 9: Commit**

```bash
git add src/params.rs src/dsp/fx_matrix.rs src/dsp/modules/mod.rs src/dsp/modules/life.rs src/lib.rs tests/module_trait.rs
git commit -m "$(cat <<'EOF'
feat(life): per-slot mode persistence

Adds `slot_life_mode: [Arc<Mutex<LifeMode>>; MAX_SLOTS]` to params,
snapshotted per block in lib.rs and pushed into FxMatrix via
set_life_modes. Reassignment inherits the persisted mode.

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>
EOF
)"
```

---

## Task 14: Mode picker UI — `life_popup.rs`

**Files:**
- Create: `src/editor/life_popup.rs`
- Modify: `src/editor/mod.rs` (`pub mod life_popup;`)
- Modify: `src/editor/module_popup.rs` (add Life as assignable + invoke life popup on right-click)
- Modify: `src/editor/editor_ui.rs` (instantiate `LifePopupState` + dispatch)

- [ ] **Step 1: Add module declaration**

In `src/editor/mod.rs`:

```rust
pub mod life_popup;
```

- [ ] **Step 2: Create the popup file**

Create `src/editor/life_popup.rs`:

```rust
use std::sync::{Arc, Mutex};

use nih_plug_egui::egui;

use crate::dsp::modules::life::LifeMode;

pub struct LifePopupState {
    pub open_for_slot: Option<usize>,
    pub anchor: egui::Pos2,
}

impl LifePopupState {
    pub fn new() -> Self {
        Self {
            open_for_slot: None,
            anchor: egui::Pos2::ZERO,
        }
    }

    pub fn close(&mut self) {
        self.open_for_slot = None;
    }
}

/// Show the Life mode picker popup. Returns true if the popup is still open.
pub fn show_life_popup(
    ui: &mut egui::Ui,
    state: &mut LifePopupState,
    slot_life_mode: &Arc<Mutex<LifeMode>>,
) -> bool {
    let anchor = state.anchor;
    let mut still_open = true;

    egui::Window::new("Life Mode")
        .fixed_pos(anchor)
        .collapsible(false)
        .resizable(false)
        .show(ui.ctx(), |ui| {
            let modes = [
                (LifeMode::Viscosity, "Viscosity", "Diffusion across bins"),
                (LifeMode::SurfaceTension, "Surface Tension", "Coalesce adjacent peaks"),
                (LifeMode::Crystallization, "Crystallization", "Sustained tones lock to phase"),
                (LifeMode::Archimedes, "Archimedes", "Volume-conserving global ducking"),
                (LifeMode::NonNewtonian, "Non-Newtonian", "Limit fast transients"),
                (LifeMode::Stiction, "Stiction", "Static + kinetic friction"),
                (LifeMode::Yield, "Yield", "Fabric tearing at threshold"),
                (LifeMode::Capillary, "Capillary", "Wick energy upward to harmonics"),
                (LifeMode::Sandpaper, "Sandpaper", "Phase friction emits sparks"),
                (LifeMode::Brownian, "Brownian", "Drift scaled by upstream temperature"),
            ];

            let current = slot_life_mode.try_lock().map(|m| *m).unwrap_or_default();

            for (mode, label, hint) in modes {
                let selected = mode == current;
                if ui
                    .selectable_label(selected, label)
                    .on_hover_text(hint)
                    .clicked()
                {
                    if let Ok(mut g) = slot_life_mode.try_lock() {
                        *g = mode;
                    }
                    still_open = false;
                }
            }

            ui.separator();
            if ui.button("Close").clicked() {
                still_open = false;
            }
        });

    if !still_open {
        state.close();
    }
    still_open
}
```

- [ ] **Step 3: Make Life assignable in `module_popup.rs`**

In `src/editor/module_popup.rs`, find the assignment list and add Life. Also add a "Configure Life…" entry that opens `LifePopupState`.

```rust
// Inside the per-module assignable list, add:
ModuleType::Life => "Life",
```

In the right-click handler (where Geometry/Circuit etc. open their popups):

```rust
if module_ty == ModuleType::Life {
    life_popup_state.open_for_slot = Some(slot_idx);
    life_popup_state.anchor = response.rect.right_top();
}
```

- [ ] **Step 4: Wire the popup state in `editor_ui.rs`**

In `src/editor/editor_ui.rs::create_editor()`, alongside `geometry_popup_state`:

```rust
let life_popup_state = Arc::new(Mutex::new(crate::editor::life_popup::LifePopupState::new()));
```

In the per-frame body, after the Geometry popup dispatch:

```rust
if let Ok(mut state) = life_popup_state.try_lock() {
    if let Some(slot_idx) = state.open_for_slot {
        crate::editor::life_popup::show_life_popup(
            ui,
            &mut state,
            &params.slot_life_mode[slot_idx],
        );
    }
}
```

- [ ] **Step 5: Verify build + open the editor manually**

Run: `cargo build --release`
Expected: clean build.

Bundle + install:
```bash
cargo run --package xtask -- bundle spectral_forge --release
cp target/bundled/spectral_forge.clap ~/.clap/
```

Open Bitwig, instantiate Spectral Forge, right-click a slot → assign **Life** → confirm popup appears with all 10 modes → click each mode and verify the slot label updates.

- [ ] **Step 6: Commit**

```bash
git add src/editor/life_popup.rs src/editor/mod.rs src/editor/module_popup.rs src/editor/editor_ui.rs
git commit -m "feat(life): mode picker popup UI (10 modes)"
```

---

## Task 15: Calibration probes — `LifeProbe`

**Files:**
- Modify: `src/dsp/modules/life.rs` (add `#[cfg(any(test, feature = "probe"))]` probe getters)
- Modify: `tests/calibration_roundtrip.rs` (add Life probe smoke test)

- [ ] **Step 1: Write the failing test**

In `tests/calibration_roundtrip.rs`:

```rust
#[test]
fn life_probe_reports_active_mode() {
    use spectral_forge::dsp::modules::life::{LifeModule, LifeMode, LifeProbe};
    use spectral_forge::dsp::modules::SpectralModule;

    let mut module = LifeModule::new();
    module.reset(48_000.0, 2048);
    module.set_mode_for_test(LifeMode::Capillary);

    let probe = module.probe();
    assert_eq!(probe.active_mode, LifeMode::Capillary);
    assert_eq!(probe.average_amount_pct, 0.0);
    assert_eq!(probe.recent_sustain_max, 0.0);
}
```

- [ ] **Step 2: Run test, expect failure**

Run: `cargo test --test calibration_roundtrip life_probe_reports_active_mode -- --nocapture`
Expected: FAIL — `LifeProbe` not defined.

- [ ] **Step 3: Add the probe struct + getter**

Append to `src/dsp/modules/life.rs`:

```rust
#[cfg(any(test, feature = "probe"))]
#[derive(Debug, Clone, Copy)]
pub struct LifeProbe {
    pub active_mode: LifeMode,
    pub average_amount_pct: f32,
    pub recent_sustain_max: f32,
    pub recent_tear_count: usize,
}

#[cfg(any(test, feature = "probe"))]
impl LifeModule {
    pub fn probe(&self) -> LifeProbe {
        let sustain_max_ch0 = self.sustain_envelope[0]
            .iter()
            .copied()
            .fold(0.0_f32, f32::max);
        let sustain_max_ch1 = self.sustain_envelope[1]
            .iter()
            .copied()
            .fold(0.0_f32, f32::max);
        let recent_sustain_max = sustain_max_ch0.max(sustain_max_ch1);

        let tear_count_ch0 = self.tear_state[0].iter().filter(|&&t| t > 0.5).count();
        let tear_count_ch1 = self.tear_state[1].iter().filter(|&&t| t > 0.5).count();
        let recent_tear_count = tear_count_ch0 + tear_count_ch1;

        LifeProbe {
            active_mode: self.mode,
            average_amount_pct: 0.0, // populated by probe-write path; v1 default zero.
            recent_sustain_max,
            recent_tear_count,
        }
    }
}
```

- [ ] **Step 4: Run test, expect pass**

Run: `cargo test --test calibration_roundtrip life_probe_reports_active_mode -- --nocapture`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/dsp/modules/life.rs tests/calibration_roundtrip.rs
git commit -m "feat(life): calibration probes (active mode + sustain + tear counts)"
```

---

## Task 16: Multi-mode multi-hop dual-channel finite/bounded smoke test

**Files:**
- Modify: `tests/module_trait.rs`

- [ ] **Step 1: Write the test**

```rust
#[test]
fn life_all_modes_finite_and_bounded() {
    use spectral_forge::dsp::modules::life::{LifeModule, LifeMode};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule, StereoLink, FxChannelTarget};
    use spectral_forge::dsp::bin_physics::BinPhysics;
    use realfft::num_complex::Complex;

    let modes = [
        LifeMode::Viscosity,
        LifeMode::SurfaceTension,
        LifeMode::Crystallization,
        LifeMode::Archimedes,
        LifeMode::NonNewtonian,
        LifeMode::Stiction,
        LifeMode::Yield,
        LifeMode::Capillary,
        LifeMode::Sandpaper,
        LifeMode::Brownian,
    ];

    let num_bins = 1025;

    for &mode in &modes {
        let mut module = LifeModule::new();
        module.reset(48_000.0, 2048);
        module.set_mode_for_test(mode);

        // Stress curves at maximum.
        let curves_storage: Vec<Vec<f32>> = vec![
            vec![2.0_f32; num_bins], // AMOUNT
            vec![0.5_f32; num_bins], // THRESHOLD (low → trigger most behaviours)
            vec![2.0_f32; num_bins], // SPEED
            vec![2.0_f32; num_bins], // REACH
            vec![2.0_f32; num_bins], // MIX
        ];
        let curves: Vec<&[f32]> = curves_storage.iter().map(|v| v.as_slice()).collect();

        // Random-ish input so cross-bin kernels have something to chew on.
        let bins_template: Vec<Complex<f32>> = (0..num_bins)
            .map(|k| {
                let mag = 0.3 + ((k * 17 % 23) as f32) * 0.05;
                let phase = (k as f32 * 0.073).sin() * std::f32::consts::PI;
                Complex::new(mag * phase.cos(), mag * phase.sin())
            })
            .collect();

        let mut physics = BinPhysics::new();
        physics.reset_active(num_bins, 48_000.0, 2048);
        // Seed velocity + temperature so velocity-/temp-reading modes get inputs.
        for k in 0..num_bins {
            physics.velocity[k] = 0.4 + ((k * 13 % 7) as f32) * 0.1;
            physics.temperature[k] = 0.5;
        }
        let physics_ref: &BinPhysics = &physics;
        let ctx = ModuleContext {
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
            bin_physics: Some(physics_ref),
        };

        // 200 hops × 2 channels.
        for ch in 0..2 {
            let mut bins = bins_template.clone();
            let mut suppression = vec![0.0_f32; num_bins];
            let mut physics_out = BinPhysics::new();
            physics_out.reset_active(num_bins, 48_000.0, 2048);

            for hop in 0..200 {
                module.process(
                    ch, StereoLink::Linked, FxChannelTarget::All,
                    &mut bins, None, &curves, &mut suppression, Some(&mut physics_out), &ctx,
                );

                for (k, b) in bins.iter().enumerate() {
                    assert!(b.norm().is_finite(),
                        "Mode {:?} ch {} hop {} bin {} produced NaN/Inf", mode, ch, hop, k);
                    assert!(b.norm() < 1e6,
                        "Mode {:?} ch {} hop {} bin {} unbounded ({})", mode, ch, hop, k, b.norm());
                }
                for (k, &s) in suppression.iter().enumerate() {
                    assert!(s.is_finite() && s >= 0.0,
                        "Mode {:?} ch {} hop {} bin {} suppression bad ({})", mode, ch, hop, k, s);
                }
            }
        }
    }
}
```

- [ ] **Step 2: Run test, expect pass**

Run: `cargo test --test module_trait life_all_modes_finite_and_bounded -- --nocapture`
Expected: PASS — all 10 modes × 2 channels × 200 hops, no NaN/Inf, bounded.

If a mode fails this test, the kernel is broken — find the offending hop/bin in the assertion message and fix the kernel before proceeding.

- [ ] **Step 3: Commit**

```bash
git add tests/module_trait.rs
git commit -m "test(life): multi-mode multi-hop dual-channel finite/bounded test"
```

---

## Task 17: Energy conservation invariant test

**Files:**
- Create: `tests/life_conservation.rs`

- [ ] **Step 1: Write the test file**

Create `tests/life_conservation.rs`:

```rust
//! Verifies the Life module's energy-conservation invariant for transport modes
//! (Viscosity, SurfaceTension, Capillary, Archimedes). State-creating modes
//! (Crystallization, Yield, Brownian) and rate-limiting modes (NonNewtonian,
//! Stiction) are explicitly exempt — see `ideas/next-gen-modules/11-life.md`
//! § "energy-conservation as the Life invariant".

use spectral_forge::dsp::modules::life::{LifeModule, LifeMode};
use spectral_forge::dsp::modules::{ModuleContext, SpectralModule, StereoLink, FxChannelTarget};
use realfft::num_complex::Complex;

fn run_mode(mode: LifeMode, bins_template: &[Complex<f32>], hops: usize) -> Vec<Complex<f32>> {
    let num_bins = bins_template.len();
    let mut module = LifeModule::new();
    module.reset(48_000.0, 2048);
    module.set_mode_for_test(mode);

    let amount = vec![1.5_f32; num_bins];
    let thresh = vec![0.5_f32; num_bins];
    let speed = vec![1.0_f32; num_bins];
    let reach = vec![1.0_f32; num_bins];
    let mix = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &thresh, &speed, &reach, &mix];

    let mut bins = bins_template.to_vec();
    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = ModuleContext {
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
        bin_physics: None,
    };

    for _ in 0..hops {
        module.process(
            0, StereoLink::Linked, FxChannelTarget::All,
            &mut bins, None, &curves, &mut suppression, None, &ctx,
        );
    }
    bins
}

fn power(bins: &[Complex<f32>]) -> f32 {
    bins.iter().map(|b| b.norm_sqr()).sum()
}

#[test]
fn viscosity_conserves_power() {
    let num_bins = 1025;
    let mut bins: Vec<Complex<f32>> = vec![Complex::new(0.0, 0.0); num_bins];
    bins[300] = Complex::new(2.0, 0.0);
    let dry_p = power(&bins);

    let wet = run_mode(LifeMode::Viscosity, &bins, 10);
    let wet_p = power(&wet);

    let loss_pct = (dry_p - wet_p).abs() / dry_p;
    // Diffusion is exact-conservative on power except for boundary edge effects.
    // 5% tolerance accommodates them at this hop count.
    assert!(loss_pct < 0.05,
        "Viscosity lost {}% of power (>5% violates conservation)", loss_pct * 100.0);
}

#[test]
fn surface_tension_conserves_magnitude_within_tolerance() {
    let num_bins = 1025;
    let mut bins: Vec<Complex<f32>> = vec![Complex::new(0.0, 0.0); num_bins];
    for k in 200..220 {
        bins[k] = Complex::new(0.7, 0.0);
    }
    let dry_mag: f32 = bins.iter().map(|b| b.norm()).sum();

    let wet = run_mode(LifeMode::SurfaceTension, &bins, 10);
    let wet_mag: f32 = wet.iter().map(|b| b.norm()).sum();

    let loss_pct = (dry_mag - wet_mag).abs() / dry_mag;
    assert!(loss_pct < 0.10,
        "SurfaceTension lost {}% of magnitude (>10% violates conservation)", loss_pct * 100.0);
}

#[test]
fn capillary_conserves_magnitude_within_tolerance() {
    let num_bins = 1025;
    let mut bins: Vec<Complex<f32>> = vec![Complex::new(0.0, 0.0); num_bins];
    bins[100] = Complex::new(1.0, 0.0);
    let dry_mag: f32 = bins.iter().map(|b| b.norm()).sum();

    let wet = run_mode(LifeMode::Capillary, &bins, 10);
    let wet_mag: f32 = wet.iter().map(|b| b.norm()).sum();

    let loss_pct = (dry_mag - wet_mag).abs() / dry_mag;
    assert!(loss_pct < 0.15,
        "Capillary lost {}% of magnitude (>15% violates conservation)", loss_pct * 100.0);
}

#[test]
fn archimedes_redistributes_without_creation() {
    // Archimedes can REDUCE total but should never INCREASE.
    let num_bins = 1025;
    let bins: Vec<Complex<f32>> = (0..num_bins).map(|_| Complex::new(0.5, 0.0)).collect();
    let dry_mag: f32 = bins.iter().map(|b| b.norm()).sum();

    let wet = run_mode(LifeMode::Archimedes, &bins, 5);
    let wet_mag: f32 = wet.iter().map(|b| b.norm()).sum();

    assert!(wet_mag <= dry_mag * 1.001,
        "Archimedes created energy (dry={}, wet={})", dry_mag, wet_mag);
}
```

- [ ] **Step 2: Run all conservation tests**

Run: `cargo test --test life_conservation -- --nocapture`
Expected: PASS — all 4 tests within tolerance.

- [ ] **Step 3: Commit**

```bash
git add tests/life_conservation.rs
git commit -m "test(life): energy conservation invariant for transport modes"
```

---

## Task 18: Status banner + STATUS.md entry + Crystallization↔Freeze follow-up note

**Files:**
- Modify: `docs/superpowers/STATUS.md`
- Modify: top of this plan file (add banner block)

- [ ] **Step 1: Update the plan banner**

The top of `docs/superpowers/plans/2026-04-27-phase-5a-life.md` currently reads:

```markdown
> **Status:** PLANNED — implementation pending. ...
```

When implementation lands, change to:

```markdown
> **Status:** IMPLEMENTED — landed in commit <SHA>. Source of truth: source code.
```

- [ ] **Step 2: Add STATUS.md entry**

In `docs/superpowers/STATUS.md`, add a row to the Plans table:

```
| 2026-04-27-phase-5a-life | Life module — 10 modes including 4 audit gap modes (Yield, Capillary, Sandpaper, Brownian) | IMPLEMENTED | Phase 5a |
```

- [ ] **Step 3: Document the Crystallization↔Freeze follow-up**

Open `docs/superpowers/specs/2026-04-21-life-module.md` (or its replacement if one was written) and append a "Follow-ups" section:

```markdown
## Follow-ups (post Phase 5a)

- **Freeze reads `BinPhysics.crystallization`** — Phase 5a Life writes
  `crystallization[k]` from the Crystallization mode but Freeze does not yet
  read it. Add a small follow-up PR to make Freeze accumulate faster on
  bins where `crystallization > 0` (per audit § Crystallization scope vs
  Freeze module). This is intentionally NOT in Phase 5a's scope to keep
  the plan focused on the Life module itself.
- **Multi-mode-per-slot stacking** — v2 enhancement (audit § Module ordering).
- **Cepstral envelope baseline for Capillary** — v2 enhancement (audit
  research finding 7).
```

- [ ] **Step 4: Final commit**

```bash
git add docs/superpowers/STATUS.md docs/superpowers/specs/2026-04-21-life-module.md docs/superpowers/plans/2026-04-27-phase-5a-life.md
git commit -m "$(cat <<'EOF'
docs(life): mark Phase 5a implemented + STATUS entry + follow-ups

Adds the Crystallization↔Freeze cooperation as a documented follow-up
(intentionally out of scope for 5a) plus the multi-mode-per-slot and
cepstral-envelope v2 enhancements.

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>
EOF
)"
```

---

## Self-review notes

**Spec coverage check:**
- ✓ Viscosity (audit § sub-effects + research finding 1) → Task 3
- ✓ Surface Tension (audit § sub-effects) → Task 4
- ✓ Crystallization + BinPhysics write (audit § sub-effects + § Crystallization scope) → Task 5
- ✓ Archimedes (audit § sub-effects) → Task 6
- ✓ Non-Newtonian (audit § sub-effects) → Task 7
- ✓ Stiction (audit § sub-effects) → Task 8
- ✓ Yield gap mode (audit § a) → Task 9
- ✓ Capillary gap mode (audit § b) → Task 10
- ✓ Sandpaper gap mode (audit § c) → Task 11
- ✓ Brownian gap mode (audit § d) → Task 12
- ✓ 5 curves (audit § Curve set) → Task 1
- ✓ Per-slot mode persistence → Task 13
- ✓ UI mode picker → Task 14
- ✓ Calibration probes (audit § Calibration probe set) → Task 15
- ✓ Multi-hop dual-channel finite/bounded → Task 16
- ✓ Energy conservation invariant test (audit § Open question 2) → Task 17
- ✓ Crystallization↔Freeze follow-up documented (audit § Open question 1) → Task 18

**Type consistency check:**
- `LifeMode` variants used identically across Tasks 2-18.
- `apply_*` kernel signatures consistent: take `&mut [Complex<f32>]`, scratch buffers, `&[&[f32]]` curves, `num_bins`, optional physics.
- `set_life_mode` trait method (Task 13) matches existing `set_geometry_mode`/`set_circuit_mode`/`set_modulate_mode` pattern from Phase 2e/2f/2g.
- `BinPhysics` field accesses (`crystallization`, `bias`, `displacement`, `temperature`, `velocity`) all match Phase 3 plan definitions.
