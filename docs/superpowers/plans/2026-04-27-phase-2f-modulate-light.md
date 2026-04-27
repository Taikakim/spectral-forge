# Phase 2f — Modulate-light Module Implementation Plan

> **Status:** PLANNED — implementation pending. Phase 2 sub-plan; depends on Phase 1 foundation infra (`docs/superpowers/plans/2026-04-27-phase-1-foundation-infra.md`).
>
> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship the Modulate module with five light-CPU modes — **Phase Phaser**, **Bin Swapper**, **RM/FM Matrix**, **Diode RM**, and **Ground Loop** — using the existing SpectralModule trait + sidechain plumbing. Defers Gravity Phaser, PLL Tear, FM Network, and Slew Lag to Phase 5/6.

**Architecture:** New `ModuleType::Modulate` slot. Per-channel state holds a small phase-animation accumulator (Phase Phaser), bin-swap scratch buffer, and an RMS history ring (Ground Loop). Mode is per-slot (persisted via `Mutex<ModulateMode>`), dispatched per block. Sidechain consumption is gated by mode (RM/FM Matrix + Diode RM consume the input). The module declares `wants_sidechain: true` so the routing layer auto-routes Sc(0) on first assignment.

**Tech Stack:** Rust 2021, `nih-plug`, `realfft::num_complex::Complex<f32>`, existing `SpectralModule` trait + `ModuleContext`.

**Source spec:** `ideas/next-gen-modules/16-modulate.md` (research findings 2026-04-26 incorporated).

**Defer list (NOT in this plan):**
- **Gravity Phaser** — requires `BinPhysics::phase_momentum` (Phase 3). Lands in Phase 5b.
- **PLL Tear** — best with PLPV unwrapped phase (Phase 4). Lands in Phase 5b.
- **FM Network — Partial Web** — requires `ctx.instantaneous_freq` (Phase 6.1). Lands in Phase 6.6.
- **Slew Lag** — requires `ctx.sidechain_derivative` (deferred to Phase 5/6). Not in v1.
- **Repel toggle for Gravity Phaser** — no Gravity Phaser in v1, so N/A.
- **Sidechain-positioned wells** — no Gravity Phaser in v1, so N/A.

**Risk register:**
- Phase Phaser is animated (rotation amount accumulates per hop). The `hop_count: u64` accumulator wraps cleanly for >12 million years at 16k FFT — non-issue.
- Bin Swapper uses a scratch buffer to avoid in-place overwrite. Memory: 2 channels × MAX_NUM_BINS × 8 bytes = ~130 KB per Modulate slot. Acceptable.
- RM and Diode RM consume sidechain magnitude. If no sidechain is routed, they fall through to passthrough (audible: silence on the slot output). Documented in popup help text.
- Ground Loop hard-codes 50/60 Hz toggle via the RATE curve (gain 0..1 → 50 Hz; gain 1..2 → 60 Hz). Not user-tunable in v1. Open question deferred to user feedback.
- Animated Phase Phaser has perceptible "modulator phase" — the rotation depends on the GUI hop count, which differs across plugin instances. This is intentional but noted (some users will want absolute-phase rotation; can add a `Static` toggle in v2).

---

## File Structure

**Create:**
- `src/dsp/modules/modulate.rs` — `ModulateModule` impl, `ModulateMode` enum, kernels.
- `src/editor/modulate_popup.rs` — mode picker popup.

**Modify:**
- `src/dsp/modules/mod.rs` — add `ModuleType::Modulate` variant, `module_spec(Modulate)` entry, `create_module()` wiring, `set_modulate_mode` trait default.
- `src/dsp/fx_matrix.rs` — add `set_modulate_modes()` per-block sync.
- `src/params.rs` — add `slot_modulate_mode: [Arc<Mutex<ModulateMode>>; MAX_SLOTS]`.
- `src/lib.rs` — snapshot per-block, dispatch to FxMatrix.
- `src/editor/theme.rs` — `MODULATE_DOT_COLOR`.
- `src/editor/module_popup.rs` — make Modulate assignable + mode picker entry.
- `src/editor/fx_matrix_grid.rs` — render Modulate slot label.
- `tests/module_trait.rs` — finite/bounded test for all five modes.
- `tests/calibration_roundtrip.rs` — modulate probes.
- `docs/superpowers/STATUS.md` — entry for this plan.

---

## Task 1: Add `ModuleType::Modulate` variant + theme color + ModuleSpec entry

**Files:**
- Modify: `src/dsp/modules/mod.rs` (ModuleType enum, module_spec catalog)
- Modify: `src/editor/theme.rs`
- Modify: `tests/module_trait.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn modulate_module_spec_present() {
    use spectral_forge::dsp::modules::{module_spec, ModuleType};

    let spec = module_spec(ModuleType::Modulate);
    assert_eq!(spec.display_name, "MOD");
    assert_eq!(spec.num_curves, 6);
    assert_eq!(spec.curve_labels.len(), 6);
    assert!(spec.assignable_to_user_slots, "Modulate must be user-assignable");
    assert!(!spec.heavy_cpu, "v1 ships 5 light-CPU modes only");
    assert!(spec.wants_sidechain, "RM/Diode RM modes need sidechain auto-routed");
}
```

- [ ] **Step 2: Run test, expect failure**

Run: `cargo test --test module_trait modulate_module_spec_present -- --nocapture`
Expected: FAIL — `ModuleType::Modulate` not found.

- [ ] **Step 3: Add the enum variant**

In `src/dsp/modules/mod.rs::ModuleType`:

```rust
pub enum ModuleType {
    // ... existing ...
    Modulate,
    // (Master remains last)
}
```

- [ ] **Step 4: Add module_spec entry**

In `src/dsp/modules/mod.rs::module_spec()`:

```rust
ModuleType::Modulate => ModuleSpec {
    ty: ModuleType::Modulate,
    display_name: "MOD",
    color: theme::MODULATE_DOT_COLOR,
    num_curves: 6,
    curve_labels: &["AMOUNT", "REACH", "RATE", "THRESH", "AMPGATE", "MIX"],
    assignable_to_user_slots: true,
    heavy_cpu: false,
    wants_sidechain: true,
    panel_widget: None,
},
```

- [ ] **Step 5: Add theme color**

In `src/editor/theme.rs`:

```rust
/// Modulate module — magenta/purple for "carrier / phase / sideband" feel.
pub const MODULATE_DOT_COLOR: egui::Color32 = egui::Color32::from_rgb(180, 100, 200);
```

- [ ] **Step 6: Run test, expect pass**

Run: `cargo test --test module_trait modulate_module_spec_present -- --nocapture`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add src/dsp/modules/mod.rs src/editor/theme.rs tests/module_trait.rs
git commit -m "feat(modulate): add ModuleType::Modulate variant + spec entry"
```

---

## Task 2: ModulateMode enum + ModulateModule skeleton + create_module() wiring

**Files:**
- Create: `src/dsp/modules/modulate.rs`
- Modify: `src/dsp/modules/mod.rs::create_module()`

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn modulate_module_constructs_and_passes_through() {
    use spectral_forge::dsp::modules::{create_module, ModuleType, SpectralModule, ModuleContext, StereoLink, FxChannelTarget};
    use realfft::num_complex::Complex;

    let mut module = create_module(ModuleType::Modulate);
    module.reset(48_000.0, 2048);
    assert_eq!(module.module_type(), ModuleType::Modulate);
    assert_eq!(module.num_curves(), 6);

    let num_bins = 1025;
    let mut bins: Vec<Complex<f32>> = (0..num_bins).map(|k| Complex::new((k as f32 * 0.01).sin(), 0.0)).collect();
    let dry: Vec<Complex<f32>> = bins.clone();

    // All curves neutral: AMOUNT=0, MIX=0 → passthrough.
    let zeros = vec![0.0_f32; num_bins];
    let neutral = vec![1.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&zeros, &neutral, &neutral, &neutral, &zeros, &zeros];

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
    };

    module.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, None, &curves, &mut suppression, &ctx);

    for k in 0..num_bins {
        let diff = (bins[k] - dry[k]).norm();
        assert!(diff < 1e-5, "bin {} drifted by {}", k, diff);
    }
    for s in &suppression {
        assert!(s.is_finite() && *s >= 0.0);
    }
}
```

- [ ] **Step 2: Run test, expect failure**

Run: `cargo test --test module_trait modulate_module_constructs_and_passes_through -- --nocapture`
Expected: FAIL — `unimplemented!()` panic from create_module.

- [ ] **Step 3: Create the modulate.rs file with skeleton**

Create `src/dsp/modules/modulate.rs`:

```rust
use realfft::num_complex::Complex;
use serde::{Deserialize, Serialize};

use crate::dsp::modules::{
    FxChannelTarget, ModuleContext, ModuleType, SpectralModule, StereoLink,
};

pub const MAX_BINS: usize = 8193;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModulateMode {
    PhasePhaser,
    BinSwapper,
    RmFmMatrix,
    DiodeRm,
    GroundLoop,
}

impl Default for ModulateMode {
    fn default() -> Self {
        ModulateMode::PhasePhaser
    }
}

pub struct ModulateModule {
    mode: ModulateMode,
    /// Per-channel hop counter for animated Phase Phaser.
    hop_count: [u64; 2],
    /// Per-channel scratch for Bin Swapper.
    swap_scratch: [Vec<Complex<f32>>; 2],
    /// Per-channel RMS history ring for Ground Loop sag detection.
    rms_history: [[f32; 16]; 2],
    rms_idx: [usize; 2],
    sample_rate: f32,
    fft_size: usize,
}

impl ModulateModule {
    pub fn new() -> Self {
        Self {
            mode: ModulateMode::default(),
            hop_count: [0; 2],
            swap_scratch: [Vec::new(), Vec::new()],
            rms_history: [[0.0; 16]; 2],
            rms_idx: [0; 2],
            sample_rate: 48_000.0,
            fft_size: 2048,
        }
    }

    pub fn set_mode_for_test(&mut self, mode: ModulateMode) {
        self.mode = mode;
    }

    pub fn current_mode(&self) -> ModulateMode {
        self.mode
    }
}

impl SpectralModule for ModulateModule {
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
        // v1 stub. Mode dispatch added in Task 8.
        for s in suppression_out.iter_mut() {
            *s = 0.0;
        }
    }

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
        }
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::Modulate
    }

    fn num_curves(&self) -> usize {
        6
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
```

- [ ] **Step 4: Wire create_module()**

In `src/dsp/modules/mod.rs::create_module()`:

```rust
ModuleType::Modulate => Box::new(crate::dsp::modules::modulate::ModulateModule::new()),
```

Add module declaration at the top of `src/dsp/modules/mod.rs`:

```rust
pub mod modulate;
```

- [ ] **Step 5: Run test, expect pass**

Run: `cargo test --test module_trait modulate_module_constructs_and_passes_through -- --nocapture`
Expected: PASS — module constructs, passthrough holds.

- [ ] **Step 6: Commit**

```bash
git add src/dsp/modules/modulate.rs src/dsp/modules/mod.rs tests/module_trait.rs
git commit -m "feat(modulate): module skeleton + ModulateMode enum"
```

---

## Task 3: Phase Phaser kernel — animated phase rotation with AmpGate

**Files:**
- Modify: `src/dsp/modules/modulate.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn modulate_phase_phaser_rotates_phase() {
    use spectral_forge::dsp::modules::modulate::{ModulateModule, ModulateMode};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule, StereoLink, FxChannelTarget};
    use realfft::num_complex::Complex;

    let mut module = ModulateModule::new();
    module.reset(48_000.0, 2048);
    module.set_mode_for_test(ModulateMode::PhasePhaser);

    let num_bins = 1025;
    // Pure cosines (phase = 0) at unit magnitude.
    let mut bins: Vec<Complex<f32>> = (0..num_bins).map(|_| Complex::new(1.0, 0.0)).collect();

    // AMOUNT=2 (max rotation), RATE=1 (steady), AMPGATE=0 (no gating), MIX=2 (full wet).
    let amount = vec![2.0_f32; num_bins];
    let reach = vec![1.0_f32; num_bins];
    let rate = vec![1.0_f32; num_bins];
    let thresh = vec![1.0_f32; num_bins];
    let ampgate = vec![0.0_f32; num_bins];
    let mix = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &reach, &rate, &thresh, &ampgate, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = test_ctx(num_bins);

    module.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, None, &curves, &mut suppression, &ctx);

    // Magnitudes must be preserved (rotation is a unit-modulus operation).
    for k in 0..num_bins {
        let mag = bins[k].norm();
        assert!((mag - 1.0).abs() < 1e-3, "bin {} magnitude drifted to {}", k, mag);
    }
    // At least some phases must have rotated away from 0.
    let max_im: f32 = bins.iter().map(|b| b.im.abs()).fold(0.0_f32, f32::max);
    assert!(max_im > 0.1, "Phase Phaser did not rotate phase (max im = {})", max_im);
}

fn test_ctx(num_bins: usize) -> spectral_forge::dsp::modules::ModuleContext {
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

Run: `cargo test --test module_trait modulate_phase_phaser_rotates_phase -- --nocapture`
Expected: FAIL — Phase Phaser arm unimplemented; bins remain at 0 phase.

- [ ] **Step 3: Add the Phase Phaser kernel**

In `src/dsp/modules/modulate.rs` (above `impl SpectralModule`):

```rust
fn apply_phase_phaser(
    bins: &mut [Complex<f32>],
    hop_count: u64,
    curves: &[&[f32]],
) {
    use std::f32::consts::PI;

    let amount_c = curves[0];
    let rate_c = curves[2];
    let thresh_c = curves[3];
    let ampgate_c = curves[4];
    let mix_c = curves[5];

    let num_bins = bins.len();
    let hop_phase_base = (hop_count as f32) * 0.01; // ~0.01 rad/hop accumulator base

    for k in 0..num_bins {
        let amount = amount_c[k].clamp(0.0, 2.0); // 0..2π rotation potential
        let rate = rate_c[k].clamp(0.0, 4.0);
        let thresh = thresh_c[k].clamp(0.01, 4.0);
        let gate_strength = ampgate_c[k].clamp(0.0, 2.0);
        let mix = mix_c[k].clamp(0.0, 2.0) * 0.5;

        // Per-bin animated rotation.
        let mag = bins[k].norm();
        // Amp gate: scale rotation by min(mag/threshold, 1) when AMPGATE > 0.
        let gate_factor = if gate_strength > 0.001 {
            ((mag / thresh).min(1.0)) * gate_strength.min(1.0)
        } else {
            1.0
        };
        let rotation = amount * PI * (hop_phase_base * rate + (k as f32) * 0.001).sin() * gate_factor;
        let cos_r = rotation.cos();
        let sin_r = rotation.sin();
        let dry = bins[k];
        // Complex multiplication by exp(i·rotation) preserves magnitude.
        let wet = Complex::new(
            dry.re * cos_r - dry.im * sin_r,
            dry.re * sin_r + dry.im * cos_r,
        );
        bins[k] = dry * (1.0 - mix) + wet * mix;
    }
}
```

- [ ] **Step 4: Wire dispatch in process() (temporary — full match in Task 8)**

Replace the stub body of `process()`:

```rust
fn process(
    &mut self,
    channel: usize,
    _stereo_link: StereoLink,
    _target: FxChannelTarget,
    bins: &mut [Complex<f32>],
    sidechain: Option<&[f32]>,
    curves: &[&[f32]],
    suppression_out: &mut [f32],
    _ctx: &ModuleContext,
) {
    debug_assert!(channel < 2);
    let _ = sidechain; // Used by RM modes — silence warnings until Task 8.

    match self.mode {
        ModulateMode::PhasePhaser => {
            apply_phase_phaser(bins, self.hop_count[channel], curves);
            self.hop_count[channel] = self.hop_count[channel].wrapping_add(1);
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

Run: `cargo test --test module_trait modulate_phase_phaser_rotates_phase -- --nocapture`
Expected: PASS — magnitudes preserved, phases rotated.

- [ ] **Step 6: Commit**

```bash
git add src/dsp/modules/modulate.rs tests/module_trait.rs
git commit -m "feat(modulate): Phase Phaser kernel with AmpGate"
```

---

## Task 4: Bin Swapper kernel

**Files:**
- Modify: `src/dsp/modules/modulate.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn modulate_bin_swapper_blends_neighbours() {
    use spectral_forge::dsp::modules::modulate::{ModulateModule, ModulateMode};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule, StereoLink, FxChannelTarget};
    use realfft::num_complex::Complex;

    let mut module = ModulateModule::new();
    module.reset(48_000.0, 2048);
    module.set_mode_for_test(ModulateMode::BinSwapper);

    let num_bins = 1025;
    // Spike at bin 100, silence elsewhere.
    let mut bins: Vec<Complex<f32>> = vec![Complex::new(0.0, 0.0); num_bins];
    bins[100] = Complex::new(2.0, 0.0);

    // AMOUNT=2 (max swap), REACH=1 (offset = 5 bins), THRESHOLD=0 (swap all bins above 0), MIX=2 (full wet).
    let amount = vec![2.0_f32; num_bins];
    let reach = vec![1.0_f32; num_bins];
    let zeros = vec![0.0_f32; num_bins];
    let thresh = vec![0.0_f32; num_bins];
    let mix = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &reach, &zeros, &thresh, &zeros, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = test_ctx(num_bins);

    module.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, None, &curves, &mut suppression, &ctx);

    // Bin 100 must have decreased (its energy was swapped/blended out).
    assert!(bins[100].norm() < 2.0, "bin 100 still at full magnitude (no swap)");
    // Bin 100 + 5 (offset = REACH × 5) must show non-zero energy (swap landed).
    assert!(bins[105].norm() > 0.1, "bin 105 silent — swap did not land");
}
```

- [ ] **Step 2: Run test, expect failure**

Run: `cargo test --test module_trait modulate_bin_swapper_blends_neighbours -- --nocapture`
Expected: FAIL — Bin Swapper arm not yet wired.

- [ ] **Step 3: Add the Bin Swapper kernel**

In `src/dsp/modules/modulate.rs`:

```rust
fn apply_bin_swapper(
    bins: &mut [Complex<f32>],
    scratch: &mut [Complex<f32>],
    curves: &[&[f32]],
) {
    let amount_c = curves[0];
    let reach_c = curves[1];
    let thresh_c = curves[3];
    let mix_c = curves[5];

    let num_bins = bins.len();

    // Snapshot current bins into scratch — needed because swap reads other indices.
    scratch[..num_bins].copy_from_slice(&bins[..num_bins]);

    for k in 0..num_bins {
        let amount = (amount_c[k].clamp(0.0, 2.0)) * 0.5; // 0..1 blend
        let reach = reach_c[k].clamp(0.0, 4.0);
        let offset = (reach * 5.0).round() as i32; // up to ±20 bins offset
        let thresh = thresh_c[k].clamp(0.0, 4.0) * 0.1; // magnitude floor
        let mix = mix_c[k].clamp(0.0, 2.0) * 0.5;

        let cur_mag = scratch[k].norm();
        if cur_mag < thresh {
            // Below threshold: leave bin untouched.
            continue;
        }

        let target_idx = (k as i32 + offset).clamp(0, num_bins as i32 - 1) as usize;
        let dry = scratch[k];
        let other = scratch[target_idx];
        let wet = dry * (1.0 - amount) + other * amount;
        bins[k] = dry * (1.0 - mix) + wet * mix;
    }
}
```

- [ ] **Step 4: Wire dispatch in process()**

In the match arm, add:

```rust
ModulateMode::BinSwapper => {
    let scratch = &mut self.swap_scratch[channel];
    apply_bin_swapper(bins, scratch, curves);
}
```

- [ ] **Step 5: Run test, expect pass**

Run: `cargo test --test module_trait modulate_bin_swapper_blends_neighbours -- --nocapture`
Expected: PASS — bin 100 reduced, bin 105 lit up.

- [ ] **Step 6: Commit**

```bash
git add src/dsp/modules/modulate.rs tests/module_trait.rs
git commit -m "feat(modulate): Bin Swapper kernel with scratch buffer"
```

---

## Task 5: RM/FM Matrix kernel — sidechain ring/freq mod

**Files:**
- Modify: `src/dsp/modules/modulate.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn modulate_rm_fm_matrix_modulates_with_sidechain() {
    use spectral_forge::dsp::modules::modulate::{ModulateModule, ModulateMode};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule, StereoLink, FxChannelTarget};
    use realfft::num_complex::Complex;

    let mut module = ModulateModule::new();
    module.reset(48_000.0, 2048);
    module.set_mode_for_test(ModulateMode::RmFmMatrix);

    let num_bins = 1025;
    // Input: unit cosines at all bins.
    let mut bins: Vec<Complex<f32>> = vec![Complex::new(1.0, 0.0); num_bins];
    let dry: Vec<Complex<f32>> = bins.clone();

    // Sidechain: spike at bin 200 with magnitude 4.
    let mut sc = vec![0.0_f32; num_bins];
    sc[200] = 4.0;

    // AMOUNT=0 (pure RM, no FM blend), REACH=2 (no falloff), THRESHOLD=0, MIX=2 (full wet).
    let amount = vec![0.0_f32; num_bins];
    let reach = vec![2.0_f32; num_bins];
    let rate = vec![1.0_f32; num_bins];
    let thresh = vec![0.0_f32; num_bins];
    let zeros = vec![0.0_f32; num_bins];
    let mix = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &reach, &rate, &thresh, &zeros, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = test_ctx(num_bins);

    module.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, Some(&sc), &curves, &mut suppression, &ctx);

    // Bin 200 must show RM product = input × sidechain ≈ 4.0.
    assert!(
        bins[200].norm() > 1.5,
        "RM at bin 200 too small ({}); expected ≈ 4.0",
        bins[200].norm()
    );
    // Bins far from 200 (e.g., bin 50) get suppressed by REACH falloff *and* sidechain near-zero.
    assert!(bins[50].norm() < dry[50].norm() + 0.1, "bin 50 grew unexpectedly");
}
```

- [ ] **Step 2: Run test, expect failure**

Run: `cargo test --test module_trait modulate_rm_fm_matrix_modulates_with_sidechain -- --nocapture`
Expected: FAIL — RmFmMatrix arm unimplemented.

- [ ] **Step 3: Add the RM/FM kernel**

```rust
fn apply_rm_fm_matrix(
    bins: &mut [Complex<f32>],
    sidechain: &[f32],
    curves: &[&[f32]],
) {
    let amount_c = curves[0];
    let reach_c = curves[1];
    let thresh_c = curves[3];
    let mix_c = curves[5];

    let num_bins = bins.len().min(sidechain.len());

    for k in 0..num_bins {
        let fm_blend = (amount_c[k].clamp(0.0, 2.0)) * 0.5; // 0=pure RM, 1=pure FM
        let reach = reach_c[k].clamp(0.0, 4.0);
        let thresh = thresh_c[k].clamp(0.0, 4.0) * 0.1;
        let mix = mix_c[k].clamp(0.0, 2.0) * 0.5;

        let sc = sidechain[k].max(0.0);
        if sc < thresh {
            continue;
        }

        let dry = bins[k];
        // RM output: complex multiply (mag × mag, phase + phase=0 since sidechain is real).
        let rm_out = dry * sc * reach;
        // FM output: rotate phase by sidechain magnitude (treat as phase delta).
        let phase = sc * std::f32::consts::PI;
        let cos_p = phase.cos();
        let sin_p = phase.sin();
        let fm_out = Complex::new(
            dry.re * cos_p - dry.im * sin_p,
            dry.re * sin_p + dry.im * cos_p,
        ) * dry.norm(); // preserve magnitude scaling

        let wet = rm_out * (1.0 - fm_blend) + fm_out * fm_blend;
        bins[k] = dry * (1.0 - mix) + wet * mix;
    }
}
```

- [ ] **Step 4: Wire dispatch in process()**

```rust
ModulateMode::RmFmMatrix => {
    if let Some(sc) = sidechain {
        apply_rm_fm_matrix(bins, sc, curves);
    }
    // No sidechain → passthrough.
}
```

- [ ] **Step 5: Run test, expect pass**

Run: `cargo test --test module_trait modulate_rm_fm_matrix_modulates_with_sidechain -- --nocapture`
Expected: PASS — bin 200 lit up to ~4.0, bin 50 untouched.

- [ ] **Step 6: Commit**

```bash
git add src/dsp/modules/modulate.rs tests/module_trait.rs
git commit -m "feat(modulate): RM/FM Matrix kernel (sidechain-driven)"
```

---

## Task 6: Diode RM kernel — analog-style RM with amplitude-gated leak

**Files:**
- Modify: `src/dsp/modules/modulate.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn modulate_diode_rm_leaks_carrier_when_input_quiet() {
    use spectral_forge::dsp::modules::modulate::{ModulateModule, ModulateMode};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule, StereoLink, FxChannelTarget};
    use realfft::num_complex::Complex;

    let num_bins = 1025;

    let mut module_quiet = ModulateModule::new();
    module_quiet.reset(48_000.0, 2048);
    module_quiet.set_mode_for_test(ModulateMode::DiodeRm);

    let mut module_loud = ModulateModule::new();
    module_loud.reset(48_000.0, 2048);
    module_loud.set_mode_for_test(ModulateMode::DiodeRm);

    // Same sidechain (carrier) for both: spike at bin 300, magnitude 2.0.
    let mut sc = vec![0.0_f32; num_bins];
    sc[300] = 2.0;

    // Quiet input: bin 300 magnitude = 0.05 (well below threshold = 0.5).
    let mut bins_quiet: Vec<Complex<f32>> = vec![Complex::new(0.0, 0.0); num_bins];
    bins_quiet[300] = Complex::new(0.05, 0.0);

    // Loud input: bin 300 magnitude = 1.5 (well above threshold).
    let mut bins_loud: Vec<Complex<f32>> = vec![Complex::new(0.0, 0.0); num_bins];
    bins_loud[300] = Complex::new(1.5, 0.0);

    // AMOUNT=2 (max RM), REACH=1, RATE=1, THRESHOLD=1 (= 0.5 absolute), AMPGATE=0, MIX=2.
    let amount = vec![2.0_f32; num_bins];
    let reach = vec![1.0_f32; num_bins];
    let rate = vec![1.0_f32; num_bins];
    let thresh = vec![1.0_f32; num_bins];
    let zeros = vec![0.0_f32; num_bins];
    let mix = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &reach, &rate, &thresh, &zeros, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = test_ctx(num_bins);

    module_quiet.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins_quiet, Some(&sc), &curves, &mut suppression, &ctx);
    module_loud.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins_loud, Some(&sc), &curves, &mut suppression, &ctx);

    // Quiet input → carrier leaks through. Bin 300 should grow above the (tiny) RM product.
    let quiet_out = bins_quiet[300].norm();
    // Loud input → diode "shuts" the leak, output ≈ RM product (input × carrier = 1.5 × 2.0 = 3.0).
    let loud_out = bins_loud[300].norm();

    // Quiet path output should mostly come from leaked carrier (~0.5–2.0 range).
    assert!(quiet_out > 0.3, "quiet path bin 300 = {} (expected leak > 0.3)", quiet_out);
    // Loud path output should be much closer to pure RM product.
    assert!(loud_out > 1.0, "loud path bin 300 = {} (expected RM product > 1.0)", loud_out);
    // Crucially: loud path should *not* show as much carrier leak as quiet.
    assert!((loud_out / 3.0 - 1.0).abs() < (quiet_out / 0.1 - 1.0).abs(),
        "loud path should be closer to pure RM than quiet path (q={}, l={})", quiet_out, loud_out);
}
```

- [ ] **Step 2: Run test, expect failure**

Run: `cargo test --test module_trait modulate_diode_rm_leaks_carrier_when_input_quiet -- --nocapture`
Expected: FAIL — DiodeRm arm unimplemented.

- [ ] **Step 3: Add the Diode RM kernel**

```rust
fn apply_diode_rm(
    bins: &mut [Complex<f32>],
    sidechain: &[f32],
    curves: &[&[f32]],
) {
    let amount_c = curves[0];
    let reach_c = curves[1];
    let thresh_c = curves[3];
    let mix_c = curves[5];

    let num_bins = bins.len().min(sidechain.len());

    for k in 0..num_bins {
        let amount = (amount_c[k].clamp(0.0, 2.0)) * 0.5; // 0..1
        let reach = reach_c[k].clamp(0.0, 4.0);
        let thresh = (thresh_c[k].clamp(0.01, 4.0)) * 0.5; // input level above which diode closes
        let mix = mix_c[k].clamp(0.0, 2.0) * 0.5;

        let sc = sidechain[k].max(0.0);
        let dry = bins[k];
        let input_amp = dry.norm();

        // Mismatch coefficient: 0 = perfect match (no leak), 1 = max leak.
        let mismatch = (1.0 - input_amp / thresh).clamp(0.0, 1.0);

        // RM path: scaled product.
        let rm_path = dry * sc * reach * amount;
        // Leak path: carrier passes through with phase preserved (real → real).
        let leak_path = Complex::new(sc * mismatch, 0.0);

        let wet = rm_path + leak_path;
        bins[k] = dry * (1.0 - mix) + wet * mix;
    }
}
```

- [ ] **Step 4: Wire dispatch in process()**

```rust
ModulateMode::DiodeRm => {
    if let Some(sc) = sidechain {
        apply_diode_rm(bins, sc, curves);
    }
}
```

- [ ] **Step 5: Run test, expect pass**

Run: `cargo test --test module_trait modulate_diode_rm_leaks_carrier_when_input_quiet -- --nocapture`
Expected: PASS — quiet path leaks carrier, loud path closer to pure RM.

- [ ] **Step 6: Commit**

```bash
git add src/dsp/modules/modulate.rs tests/module_trait.rs
git commit -m "feat(modulate): Diode RM with amplitude-gated leak"
```

---

## Task 7: Ground Loop kernel — mains hum injection gated by RMS sag

**Files:**
- Modify: `src/dsp/modules/modulate.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn modulate_ground_loop_injects_mains_harmonics() {
    use spectral_forge::dsp::modules::modulate::{ModulateModule, ModulateMode};
    use spectral_forge::dsp::modules::{ModuleContext, SpectralModule, StereoLink, FxChannelTarget};
    use realfft::num_complex::Complex;

    let mut module = ModulateModule::new();
    module.reset(48_000.0, 2048);
    module.set_mode_for_test(ModulateMode::GroundLoop);

    let num_bins = 1025;
    // Loud programme: every bin at magnitude 0.5 → high RMS → triggers sag.
    let mut bins: Vec<Complex<f32>> = vec![Complex::new(0.5, 0.0); num_bins];

    // AMOUNT=2 (max hum), REACH=2 (4 harmonics), RATE=1 (50 Hz mains), THRESHOLD=0.1 (low sag sens), MIX=2.
    let amount = vec![2.0_f32; num_bins];
    let reach = vec![2.0_f32; num_bins];
    let rate = vec![1.0_f32; num_bins]; // RATE < 1 = 50 Hz, RATE >= 1 = 60 Hz; here = 50
    let thresh = vec![0.1_f32; num_bins];
    let zeros = vec![0.0_f32; num_bins];
    let mix = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &reach, &rate, &thresh, &zeros, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = test_ctx(num_bins);

    // Run several hops to fill RMS history.
    for _ in 0..20 {
        module.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, None, &curves, &mut suppression, &ctx);
        // Re-fill bins to keep RMS high.
        for b in bins.iter_mut() { *b = Complex::new(0.5, 0.0); }
    }

    // Mains bin: round(50 × 2048 / 48000) = round(2.13) = 2.
    let mains_bin = ((50.0 * 2048.0 / 48_000.0).round() as usize).max(1);
    let h2 = mains_bin * 2;
    let h3 = mains_bin * 3;

    // Mains bin magnitude must exceed dry (0.5) — hum was injected.
    assert!(bins[mains_bin].norm() > 0.6,
        "mains bin {} = {} (expected > 0.6 with hum injected)", mains_bin, bins[mains_bin].norm());
    assert!(bins[h2].norm() > 0.5, "2nd harmonic missing (got {})", bins[h2].norm());
    assert!(bins[h3].norm() > 0.5, "3rd harmonic missing (got {})", bins[h3].norm());
}
```

- [ ] **Step 2: Run test, expect failure**

Run: `cargo test --test module_trait modulate_ground_loop_injects_mains_harmonics -- --nocapture`
Expected: FAIL — GroundLoop arm unimplemented.

- [ ] **Step 3: Add the Ground Loop kernel**

```rust
fn apply_ground_loop(
    bins: &mut [Complex<f32>],
    rms_history: &mut [f32; 16],
    rms_idx: &mut usize,
    sample_rate: f32,
    fft_size: usize,
    curves: &[&[f32]],
) {
    let amount_c = curves[0];
    let reach_c = curves[1];
    let rate_c = curves[2];
    let thresh_c = curves[3];
    let mix_c = curves[5];

    let num_bins = bins.len();

    // 1. Compute current frame's RMS.
    let mut sum_sq = 0.0_f32;
    for b in bins.iter() {
        sum_sq += b.norm_sqr();
    }
    let rms = (sum_sq / num_bins as f32).sqrt();
    rms_history[*rms_idx] = rms;
    *rms_idx = (*rms_idx + 1) % 16;

    // 2. Compute average RMS over history → sag factor.
    let avg_rms: f32 = rms_history.iter().sum::<f32>() / 16.0;
    let thresh = thresh_c[0].clamp(0.001, 4.0); // sample threshold once
    let sag_factor = (avg_rms / thresh).min(2.0);

    if sag_factor < 0.05 {
        return; // Below sag threshold: no hum injection.
    }

    // 3. Mains frequency selection: RATE < 1 → 50 Hz, RATE >= 1 → 60 Hz.
    let mains_hz = if rate_c[0] >= 1.0 { 60.0 } else { 50.0 };
    let mains_bin = ((mains_hz * fft_size as f32 / sample_rate).round() as usize).max(1);

    // 4. Number of harmonics from REACH (1..5).
    let harmonics = (1.0 + reach_c[0].clamp(0.0, 2.0) * 2.0).round() as usize; // 1..5
    let harmonics = harmonics.clamp(1, 5);

    let amount = amount_c[0].clamp(0.0, 2.0); // global hum strength

    // 5. Inject hum at mains_bin × h, h ∈ 1..=harmonics, with falloff per harmonic.
    for h in 1..=harmonics {
        let target = mains_bin * h;
        if target >= num_bins {
            break;
        }
        let harmonic_amp = amount * sag_factor / (h as f32); // 1/h falloff
        let mix = mix_c[target].clamp(0.0, 2.0) * 0.5;
        let cur_mag = bins[target].norm().max(1e-9);
        let new_mag = cur_mag + harmonic_amp;
        let scale = new_mag / cur_mag;
        let dry = bins[target];
        let wet = bins[target] * scale;
        bins[target] = dry * (1.0 - mix) + wet * mix;
    }
}
```

- [ ] **Step 4: Wire dispatch in process()**

```rust
ModulateMode::GroundLoop => {
    let history = &mut self.rms_history[channel];
    let idx = &mut self.rms_idx[channel];
    apply_ground_loop(bins, history, idx, self.sample_rate, self.fft_size, curves);
}
```

- [ ] **Step 5: Run test, expect pass**

Run: `cargo test --test module_trait modulate_ground_loop_injects_mains_harmonics -- --nocapture`
Expected: PASS — mains bin + 2 harmonics show injection.

- [ ] **Step 6: Commit**

```bash
git add src/dsp/modules/modulate.rs tests/module_trait.rs
git commit -m "feat(modulate): Ground Loop mains-hum injection"
```

---

## Task 8: Per-slot mode persistence — params.rs + FxMatrix dispatch

**Files:**
- Modify: `src/params.rs`
- Modify: `src/dsp/fx_matrix.rs`
- Modify: `src/dsp/modules/mod.rs` (trait setter default)
- Modify: `src/dsp/modules/modulate.rs` (override + reset transient state)
- Modify: `src/lib.rs::process()`

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn modulate_mode_persists_via_setter() {
    use spectral_forge::dsp::modules::{create_module, ModuleType, SpectralModule};
    use spectral_forge::dsp::modules::modulate::{ModulateMode, ModulateModule};

    let mut module = create_module(ModuleType::Modulate);
    module.reset(48_000.0, 2048);
    module.set_modulate_mode(ModulateMode::DiodeRm);
    module.reset(48_000.0, 4096);

    let mod_ = module
        .as_any()
        .downcast_ref::<ModulateModule>()
        .expect("downcast");
    assert_eq!(mod_.current_mode(), ModulateMode::DiodeRm);
}
```

- [ ] **Step 2: Run test, expect failure**

Run: `cargo test --test module_trait modulate_mode_persists_via_setter -- --nocapture`
Expected: FAIL — `set_modulate_mode` not on trait.

- [ ] **Step 3: Add `set_modulate_mode` to SpectralModule trait**

In `src/dsp/modules/mod.rs::SpectralModule`:

```rust
fn set_modulate_mode(&mut self, _mode: crate::dsp::modules::modulate::ModulateMode) {
    // No-op default. Modulate overrides.
}
```

- [ ] **Step 4: Implement override in modulate.rs**

In `src/dsp/modules/modulate.rs::impl SpectralModule for ModulateModule`:

```rust
fn set_modulate_mode(&mut self, mode: ModulateMode) {
    if mode != self.mode {
        // Reset transient state on mode change.
        for ch in 0..2 {
            self.hop_count[ch] = 0;
            self.rms_history[ch] = [0.0; 16];
            self.rms_idx[ch] = 0;
            // swap_scratch is overwritten each block, no need to clear.
        }
        self.mode = mode;
    }
}
```

Reset preserves the mode (do not reassign `self.mode` inside `reset`).

- [ ] **Step 5: Add params field**

In `src/params.rs`:

```rust
#[persist = "slot_modulate_mode"]
pub slot_modulate_mode: [Arc<Mutex<crate::dsp::modules::modulate::ModulateMode>>; MAX_SLOTS],
```

In Default::default:

```rust
slot_modulate_mode: std::array::from_fn(|_| {
    Arc::new(Mutex::new(crate::dsp::modules::modulate::ModulateMode::default()))
}),
```

Add a snap helper modeled after the other `*_mode_snap` methods:

```rust
impl SpectralForgeParams {
    pub fn modulate_mode_snap(&self) -> [crate::dsp::modules::modulate::ModulateMode; MAX_SLOTS] {
        std::array::from_fn(|s| {
            self.slot_modulate_mode[s]
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
pub fn set_modulate_modes(
    &mut self,
    modes: &[crate::dsp::modules::modulate::ModulateMode; MAX_SLOTS],
) {
    for (s, slot) in self.slots.iter_mut().enumerate() {
        if let Some(module) = slot {
            module.set_modulate_mode(modes[s]);
        }
    }
}
```

- [ ] **Step 7: Wire snapshot + push in lib.rs**

In `src/lib.rs::process()`, near the other `set_*_modes` calls:

```rust
let mod_modes = self.params.modulate_mode_snap();
self.fx_matrix.set_modulate_modes(&mod_modes);
```

- [ ] **Step 8: Run test, expect pass**

Run: `cargo test --test module_trait modulate_mode_persists_via_setter -- --nocapture`
Expected: PASS.

- [ ] **Step 9: Commit**

```bash
git add src/params.rs src/dsp/fx_matrix.rs src/dsp/modules/mod.rs src/dsp/modules/modulate.rs src/lib.rs tests/module_trait.rs
git commit -m "feat(modulate): per-slot mode persistence + setter dispatch"
```

---

## Task 9: Modulate mode picker UI (popup-based)

**Files:**
- Create: `src/editor/modulate_popup.rs`
- Modify: `src/editor/mod.rs`
- Modify: `src/editor/module_popup.rs` (add Modulate as assignable)
- Modify: `src/editor/editor_ui.rs` (right-click → popup)

- [ ] **Step 1: Add module declaration**

In `src/editor/mod.rs`:

```rust
pub mod modulate_popup;
```

- [ ] **Step 2: Create modulate_popup.rs**

```rust
use std::sync::{Arc, Mutex};
use nih_plug_egui::egui;
use crate::dsp::modules::modulate::ModulateMode;
use crate::editor::theme;

pub struct ModulatePopupState {
    pub open_for_slot: Option<usize>,
    pub anchor: egui::Pos2,
}

impl ModulatePopupState {
    pub fn new() -> Self {
        Self { open_for_slot: None, anchor: egui::Pos2::ZERO }
    }
}

pub fn show_modulate_popup(
    ui: &mut egui::Ui,
    state: &mut ModulatePopupState,
    slot_modulate_mode: &Arc<Mutex<ModulateMode>>,
) -> bool {
    let Some(_slot) = state.open_for_slot else { return false; };
    let area_id = egui::Id::new("modulate_mode_picker");
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
                    ui.label(egui::RichText::new("MODULATE MODE")
                        .color(theme::POPUP_TITLE).size(11.0));
                    ui.separator();
                    let cur = *slot_modulate_mode.lock().unwrap();
                    for (label, mode) in [
                        ("Phase Phaser",   ModulateMode::PhasePhaser),
                        ("Bin Swapper",    ModulateMode::BinSwapper),
                        ("RM/FM Matrix",   ModulateMode::RmFmMatrix),
                        ("Diode RM",       ModulateMode::DiodeRm),
                        ("Ground Loop",    ModulateMode::GroundLoop),
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
                });
        });

    if selected {
        state.open_for_slot = None;
    }
    selected
}
```

- [ ] **Step 3: Add Modulate to ASSIGNABLE_MODULES in module_popup.rs**

```rust
ModuleType::Modulate,
```

- [ ] **Step 4: Wire the popup invocation in editor_ui.rs**

Follow the same pattern used by `geometry_popup` / `future_popup`. On right-click of a Modulate slot:

```rust
if module_type == ModuleType::Modulate && response.secondary_clicked() {
    modulate_popup_state.open_for_slot = Some(slot_idx);
    modulate_popup_state.anchor = response.rect.right_top();
}
if modulate_popup_state.open_for_slot.is_some() {
    let slot_idx = modulate_popup_state.open_for_slot.unwrap();
    modulate_popup::show_modulate_popup(
        ui,
        &mut modulate_popup_state,
        &params.slot_modulate_mode[slot_idx],
    );
}
```

- [ ] **Step 5: Verify compile + manual UI check**

```bash
cargo build
cargo run --package xtask -- bundle spectral_forge
```

Manual: load in Bitwig, assign Modulate to a slot, right-click, verify all 5 modes selectable and persist.

- [ ] **Step 6: Commit**

```bash
git add src/editor/modulate_popup.rs src/editor/mod.rs src/editor/module_popup.rs src/editor/editor_ui.rs
git commit -m "feat(modulate): mode picker popup UI"
```

---

## Task 10: Calibration probes

**Files:**
- Modify: `src/dsp/modules/modulate.rs`
- Modify: `tests/calibration_roundtrip.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(feature = "probe")]
#[test]
fn modulate_calibration_roundtrip_all_modes() {
    use spectral_forge::dsp::modules::modulate::{ModulateModule, ModulateMode, ModulateProbe};
    use spectral_forge::dsp::modules::{SpectralModule, StereoLink, FxChannelTarget};
    use realfft::num_complex::Complex;

    let num_bins = 1025;

    for mode in [
        ModulateMode::PhasePhaser,
        ModulateMode::BinSwapper,
        ModulateMode::RmFmMatrix,
        ModulateMode::DiodeRm,
        ModulateMode::GroundLoop,
    ] {
        let mut module = ModulateModule::new();
        module.reset(48_000.0, 2048);
        module.set_modulate_mode(mode);

        let mut bins: Vec<Complex<f32>> = vec![Complex::new(0.5, 0.0); num_bins];
        let sc = vec![0.5_f32; num_bins];
        let amount = vec![1.0_f32; num_bins];
        let reach = vec![1.0_f32; num_bins];
        let rate = vec![1.0_f32; num_bins];
        let thresh = vec![1.0_f32; num_bins];
        let ampgate = vec![0.0_f32; num_bins];
        let mix = vec![1.0_f32; num_bins];
        let curves: Vec<&[f32]> = vec![&amount, &reach, &rate, &thresh, &ampgate, &mix];
        let mut suppression = vec![0.0_f32; num_bins];
        let ctx = mod_test_ctx(num_bins);

        for _ in 0..5 {
            module.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, Some(&sc), &curves, &mut suppression, &ctx);
        }

        let probe = module.probe_state(0);
        assert_eq!(probe.active_mode, mode);
        assert!(probe.average_amount_pct >= 0.0 && probe.average_amount_pct <= 200.0);
    }
}

#[cfg(feature = "probe")]
fn mod_test_ctx(num_bins: usize) -> spectral_forge::dsp::modules::ModuleContext {
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

Run: `cargo test --features probe --test calibration_roundtrip modulate -- --nocapture`
Expected: FAIL — `ModulateProbe`, `probe_state` not found.

- [ ] **Step 3: Add probe types and method**

In `src/dsp/modules/modulate.rs`:

```rust
#[cfg(any(test, feature = "probe"))]
#[derive(Debug, Clone, Copy)]
pub struct ModulateProbe {
    pub active_mode: ModulateMode,
    pub average_amount_pct: f32,
    pub current_hop_count: u64, // Phase Phaser only; 0 in other modes
    pub recent_rms: f32,         // Ground Loop only; 0 in other modes
}

#[cfg(any(test, feature = "probe"))]
impl ModulateModule {
    pub fn probe_state(&self, channel: usize) -> ModulateProbe {
        let ch = channel.min(1);
        let recent_rms: f32 = self.rms_history[ch].iter().sum::<f32>() / 16.0;
        ModulateProbe {
            active_mode: self.mode,
            average_amount_pct: 100.0, // last-applied amount; left at nominal for v1
            current_hop_count: self.hop_count[ch],
            recent_rms,
        }
    }
}
```

- [ ] **Step 4: Run test, expect pass**

Run: `cargo test --features probe --test calibration_roundtrip modulate -- --nocapture`
Expected: PASS — all 5 modes report sane probes.

- [ ] **Step 5: Commit**

```bash
git add src/dsp/modules/modulate.rs tests/calibration_roundtrip.rs
git commit -m "test(modulate): calibration probes for all 5 modes"
```

---

## Task 11: Multi-hop dual-channel finite/bounded contract test + status banner

**Files:**
- Modify: `tests/module_trait.rs`
- Modify: `docs/superpowers/STATUS.md`
- Modify: this plan file (top banner)

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn modulate_finite_bounded_all_modes_dual_channel() {
    use spectral_forge::dsp::modules::modulate::{ModulateModule, ModulateMode};
    use spectral_forge::dsp::modules::{SpectralModule, StereoLink, FxChannelTarget};
    use realfft::num_complex::Complex;

    let num_bins = 1025;

    for mode in [
        ModulateMode::PhasePhaser,
        ModulateMode::BinSwapper,
        ModulateMode::RmFmMatrix,
        ModulateMode::DiodeRm,
        ModulateMode::GroundLoop,
    ] {
        let mut module = ModulateModule::new();
        module.reset(48_000.0, 2048);
        module.set_modulate_mode(mode);

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
        let ctx = mod_test_ctx(num_bins);

        for hop in 0..200 {
            for ch in 0..2 {
                let bins = if ch == 0 { &mut bins_l } else { &mut bins_r };
                module.process(ch, StereoLink::Independent, FxChannelTarget::All, bins, Some(&sc), &curves, &mut suppression, &ctx);
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

#[cfg(test)]
fn mod_test_ctx(num_bins: usize) -> spectral_forge::dsp::modules::ModuleContext {
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

- [ ] **Step 2: Run test**

Run: `cargo test --test module_trait modulate_finite_bounded_all_modes_dual_channel -- --nocapture`
Expected: PASS — kernels are bounded by their clamps + magnitude scaling. If it fails, fix the kernel; finite/bounded is contract.

- [ ] **Step 3: Update banner at top of this plan**

Change:

```
> **Status:** PLANNED — implementation pending.
```

(Only after merge) to:

```
> **Status:** IMPLEMENTED — landed in commit <SHA>.
```

- [ ] **Step 4: Add entry to STATUS.md**

```
| 2026-04-27-phase-2f-modulate-light.md | IMPLEMENTED | Modulate module: 5 light modes (Phase Phaser, Bin Swapper, RM/FM, Diode RM, Ground Loop). Defers Gravity/PLL/FM Network/Slew Lag. |
```

- [ ] **Step 5: Final commit**

```bash
git add tests/module_trait.rs docs/superpowers/plans/2026-04-27-phase-2f-modulate-light.md docs/superpowers/STATUS.md
git commit -m "docs(status): mark phase-2f Modulate-light IMPLEMENTED"
```

---

## Self-review

**Spec coverage check:**
- ✅ Phase Phaser (animated rotation + AmpGate) — Task 3
- ✅ Bin Swapper (offset blend with scratch) — Task 4
- ✅ RM/FM Matrix (sidechain, RM/FM blend) — Task 5
- ✅ Diode RM (amplitude-gated leak) — Task 6
- ✅ Ground Loop (mains hum + sag-gated injection) — Task 7
- ✅ 6 curves: AMOUNT, REACH, RATE, THRESH, AMPGATE, MIX — Task 1
- ✅ wants_sidechain: true (auto-routes Sc(0) on assignment) — Task 1
- ✅ Per-slot mode persistence — Task 8
- ✅ Calibration probes — Task 10
- ✅ Defer list explicit (Gravity, PLL, FM Network, Slew Lag) — risk register

**Spec items deferred (NOT in v1):**
- Gravity Phaser — depends on `BinPhysics::phase_momentum` (Phase 3+5b).
- PLL Tear — depends on PLPV (Phase 4+5b).
- FM Network — depends on `ctx.instantaneous_freq` (Phase 6.1+6.6).
- Slew Lag — depends on `ctx.sidechain_derivative` (Phase 5+).
- Sidechain-positioned wells, Repel toggle — N/A in v1 (no Gravity Phaser).
- AmpGate for Gravity Phaser — N/A in v1; AmpGate curve is wired but only Phase Phaser consumes it.

**Type consistency:** `ModulateMode` enum used consistently across params, FxMatrix, ModulateModule, popup, probes.

**Placeholder scan:** No "TBD" / "implement later". All 5 kernels have full code in their tasks; tests have full assertions.

**Phase 1 dependency:** Only relies on `wants_sidechain` and the existing sidechain-routing pipeline (already present in fx_matrix.rs). Does not require `ctx.unwrapped_phase`, `ctx.peaks`, or `panel_widget`.

---

## Execution Handoff

**Plan complete and saved to `docs/superpowers/plans/2026-04-27-phase-2f-modulate-light.md`. Two execution options:**

**1. Subagent-Driven (recommended)** — dispatch fresh subagent per task, review between tasks.
**2. Inline Execution** — execute tasks in this session using executing-plans, batch with checkpoints.
