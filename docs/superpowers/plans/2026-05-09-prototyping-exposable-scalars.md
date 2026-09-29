# Prototyping-Exposable Scalars Implementation Plan

> **Status:** IMPLEMENTED — all 7 tasks landed (`e1357aa` … `e71954e`) and merged to master via PR #3 (`bb646c3`). The code is the source of truth; this plan is kept for history. See [../STATUS.md](../STATUS.md).

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Expose hardcoded musical constants in Life, Kinetics, Circuit, Modulate, Contrast, and PhaseSmear as host-automatable per-slot params; rework Contrast into a 3-mode dispatcher with a THRESHOLD bypass-floor fix; gate the curated tuning UI behind the `dev-build` feature flag.

**Architecture:** Each module gets a `<Module>Scalars` struct + `safe_default()` matching current hardcoded constants, a `set_<m>_scalars` trait method, per-slot params via `build.rs` codegen, pipeline gather-and-dispatch every block, and a panel widget that renders mode-conditional knobs only in dev builds. Contrast additionally gains a `ContrastMode` enum and two new DSP kernels (Temporal, Tilt) alongside the existing Spatial kernel; PhaseSmear gains a 4th curve channel.

**Tech Stack:** Rust, nih-plug, egui, build.rs codegen, triple_buffer (existing patterns).

**Spec:** [`docs/superpowers/specs/2026-05-09-prototyping-exposable-scalars-design.md`](../specs/2026-05-09-prototyping-exposable-scalars-design.md). Reference templates live at:
- `src/dsp/modules/past.rs` (PastScalars struct + safe_default + set_past_scalars + test_past_scalars)
- `src/editor/past_panel.rs` (mode-conditional panel widget)
- `build.rs:442-535` (per-slot scalar codegen — fields + inits + map entries + dispatch macro)
- `src/dsp/fx_matrix.rs:323-345` (set_past_scalars + test_past_scalars dispatchers)
- `src/dsp/pipeline.rs:855-897` (per-block gather + dispatch)
- `src/params.rs:729-733` (accessor helpers)

The default-correctness invariant (§8 of spec) requires that with `MScalars = MScalars::safe_default()` every module's output is bit-identical to current behaviour. Each task below ends with a regression test that proves this.

---

## File Structure

```
src/dsp/modules/
  contrast.rs              — ModuleSpec + struct + ContrastMode + ContrastScalars (modify)
  life.rs                  — LifeScalars struct + multiplier wire-in (modify)
  kinetics.rs              — KineticsScalars struct + scalar reads (modify)
  circuit.rs               — CircuitScalars struct + Vactrol time constant reads (modify)
  modulate.rs              — ModulateScalars struct + zeta + tear_threshold reads (modify)
  phase_smear.rs           — num_curves 3 → 4, read curve idx 3 as PHASE_RANGE (modify)
  mod.rs                   — trait method defaults; ModuleSpec.panel_widget wiring (modify)

src/dsp/engines/
  spectral_contrast.rs     — THRESHOLD wiring + Temporal & Tilt kernels (modify)

src/dsp/
  fx_matrix.rs             — 5 new set_<m>_scalars + test_<m>_scalars dispatchers (modify)
  pipeline.rs              — 5 new per-block gathers (modify)

src/editor/
  life_panel.rs            — NEW (dev-gated panel)
  kinetics_panel.rs        — NEW (dev-gated panel)
  circuit_panel.rs         — NEW (dev-gated panel)
  modulate_panel.rs        — NEW (dev-gated panel)
  contrast_panel.rs        — NEW (dev-gated panel — mode picker + scalars)
  curve_config.rs          — phase_smear_config curve_idx=3 entry (modify)
  mod.rs                   — pub use new panels (modify)

src/
  params.rs                — accessor helpers + slot_contrast_mode mutex (modify)

build.rs                   — codegen for new params (modify)

tests/
  scalar_life.rs           — NEW (default-correctness + plumbing)
  scalar_kinetics.rs       — NEW
  scalar_circuit.rs        — NEW
  scalar_modulate.rs       — NEW
  scalar_contrast.rs       — NEW (covers modes + threshold + scalars)
  scalar_phase_smear.rs    — NEW (PHASE_RANGE curve calibration)
  curve_config.rs          — extend Past TIME / PhaseSmear assertions (modify)
```

Reference files NOT modified — used as patterns:
- `src/dsp/modules/past.rs` lines 175-202, 396-407
- `src/editor/past_panel.rs`
- `build.rs:442-535`

---

## Task 0: Contrast THRESHOLD bypass-floor wiring (production fix)

**Files:**
- Modify: `src/dsp/engines/spectral_contrast.rs:152-159`
- Modify: `src/dsp/modules/contrast.rs:122-126` (reset retained-default)
- Test: `tests/scalar_contrast.rs` (new file)

This ships in production (not dev-gated) — the THRESHOLD curve is currently dead code.

- [ ] **Step 1: Write the failing test**

Create `tests/scalar_contrast.rs`:

```rust
//! Contrast THRESHOLD bypass + mode + scalars regression suite.
use spectral_forge::dsp::engines::SpectralContrastEngine;
use spectral_forge::dsp::engines::{BinParams, SpectralEngine};
use num_complex::Complex;

#[test]
fn contrast_threshold_bins_below_floor_bypass() {
    let mut engine = SpectralContrastEngine::new();
    engine.reset(48_000.0, 1024);
    let n = 513;

    // bin[0..256] sit at -60 dBFS, bin[256..] sit at 0 dBFS.
    // THRESHOLD set to -40 dBFS: bins below it should bypass, above should be processed.
    let mut bins: Vec<Complex<f32>> = (0..n)
        .map(|k| if k < 256 { Complex::new(1e-3, 0.0) } else { Complex::new(1.0, 0.0) })
        .collect();
    let original: Vec<Complex<f32>> = bins.clone();
    let threshold: Vec<f32> = vec![-40.0; n];
    let ratio:     Vec<f32> = vec![5.0; n];
    let attack:    Vec<f32> = vec![10.0; n];
    let release:   Vec<f32> = vec![100.0; n];
    let knee:      Vec<f32> = vec![0.0; n];
    let makeup:    Vec<f32> = vec![0.0; n];
    let mix:       Vec<f32> = vec![1.0; n];
    let mut suppression: Vec<f32> = vec![0.0; n];

    let params = BinParams {
        threshold_db: &threshold, ratio: &ratio, attack_ms: &attack, release_ms: &release,
        knee_db: &knee, makeup_db: &makeup, mix: &mix, smoothing_semitones: 1.0,
        sensitivity: 1.0, auto_makeup: false,
        peaks: None, plpv_dynamics_enabled: false,
    };
    engine.process_bins(&mut bins, None, &params, 48_000.0, &mut suppression);

    // Bins below threshold (k < 256) must be untouched.
    for k in 0..256 {
        assert!((bins[k].re - original[k].re).abs() < 1e-6,
            "bin {k}: expected unchanged (below threshold), got {:?}", bins[k]);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test scalar_contrast contrast_threshold_bins_below_floor_bypass`
Expected: FAIL — bins below threshold are currently processed (no bypass).

- [ ] **Step 3: Implement bypass-floor in spectral_contrast.rs Pass 4**

Edit `src/dsp/engines/spectral_contrast.rs`. Replace the `Pass 4` block (currently lines 151-163, the `for k in 0..n { ... }` after the auto_makeup update) with:

```rust
        // Pass 4 — apply smoothed gain + makeup + auto-makeup + threshold-gated mix.
        for k in 0..n {
            let auto_comp   = if params.auto_makeup { -self.auto_makeup_db[k] } else { 0.0 };
            let total_db    = (self.smooth_buf[k] + params.makeup_db[k] + auto_comp).clamp(-80.0, 40.0);
            let linear_gain = 10.0f32.powf(total_db / 20.0);

            // THRESHOLD as bypass floor: bins quieter than the per-bin threshold
            // get full dry-mix, no contrast applied. Lets the noise floor sit
            // untouched while contrast still acts on louder content.
            let mag_db   = 20.0 * bins[k].norm().max(1e-10).log10();
            let bypass_t = if mag_db < params.threshold_db[k] { 1.0 } else { 0.0 };
            let mix      = params.mix[k].clamp(0.0, 1.0) * (1.0 - bypass_t);

            bins[k] = bins[k] * (1.0 - mix + mix * linear_gain);
            suppression_out[k] = (-self.smooth_buf[k]).max(0.0);
        }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --test scalar_contrast contrast_threshold_bins_below_floor_bypass`
Expected: PASS.

- [ ] **Step 5: Run full test suite to verify nothing else broke**

Run: `cargo test`
Expected: all tests pass. The default Contrast curve has THRESHOLD at curve gain 1.0 = -20 dBFS; existing patches with default THRESHOLD now bypass anything below -20 dBFS, which IS a behaviour change for anyone relying on the previous (buggy) state. This is intentional per spec §5.

- [ ] **Step 6: Commit**

```bash
git add src/dsp/engines/spectral_contrast.rs tests/scalar_contrast.rs
git commit -m "fix(contrast): wire THRESHOLD curve as per-bin bypass floor

The Contrast module set bp_threshold from the THRESHOLD curve but the
engine never read it — dead code since module split. Now bins below
threshold dBFS get full dry-mix, leaving the noise floor untouched
while contrast still acts on louder content.

Behaviour change for existing patches: default THRESHOLD curve gain
1.0 = -20 dBFS, so quiet content that was previously contrast-processed
now bypasses. Patches relying on the prior behaviour need to draw
THRESHOLD curve down to gain 0 (axis floor) for full-spectrum effect.

Spec: docs/superpowers/specs/2026-05-09-prototyping-exposable-scalars-design.md §5."
```

---

## Task 1: Life — `LifeScalars` struct + per-mode multiplier wiring

**Files:**
- Modify: `src/dsp/modules/life.rs`
- Modify: `src/dsp/modules/mod.rs` (trait method default)
- Modify: `src/dsp/fx_matrix.rs` (add set_life_scalars + test_life_scalars)
- Modify: `src/dsp/pipeline.rs` (add gather + dispatch)
- Modify: `src/params.rs` (add accessor)
- Modify: `build.rs` (emit fields + inits + dispatch)
- Create: `src/editor/life_panel.rs`
- Modify: `src/editor/mod.rs` (pub use life_panel)
- Test: `tests/scalar_life.rs` (new file)

- [ ] **Step 1: Write the failing default-correctness test**

Create `tests/scalar_life.rs`:

```rust
//! Life scalars: default-correctness + plumbing.
use spectral_forge::dsp::modules::life::LifeScalars;

#[test]
fn life_safe_default_matches_hardcoded_values() {
    let s = LifeScalars::safe_default();
    assert_eq!(s.viscosity_scale, 1.0);
    assert_eq!(s.surface_tension_scale, 1.0);
    assert_eq!(s.non_newtonian_scale, 1.0);
    assert_eq!(s.stiction_scale, 1.0);
    assert_eq!(s.yield_scale, 1.0);
    assert_eq!(s.capillary_scale, 1.0);
    assert_eq!(s.sandpaper_scale, 1.0);
    assert_eq!(s.brownian_scale, 1.0);
}
```

- [ ] **Step 2: Run test — expect compile error (struct doesn't exist)**

Run: `cargo test --test scalar_life`
Expected: compile error: `LifeScalars` not found.

- [ ] **Step 3: Define LifeScalars in src/dsp/modules/life.rs**

After the existing `LifeMode` enum (around line 132), add:

```rust
/// Per-mode hardcoded-scale multipliers exposed for prototyping. Each field's
/// safe_default = 1.0 reproduces the current hardcoded behaviour exactly.
/// At 0.0 the corresponding mode goes inert; at 2.0 it runs at twice the
/// hardcoded scale.
///
/// See docs/superpowers/specs/2026-05-09-prototyping-exposable-scalars-design.md §1.
#[derive(Clone, Copy, Debug)]
pub struct LifeScalars {
    pub viscosity_scale:        f32,
    pub surface_tension_scale:  f32,
    pub non_newtonian_scale:    f32,
    pub stiction_scale:         f32,
    pub yield_scale:            f32,
    pub capillary_scale:        f32,
    pub sandpaper_scale:        f32,
    pub brownian_scale:         f32,
}

impl LifeScalars {
    pub fn safe_default() -> Self {
        Self {
            viscosity_scale:        1.0,
            surface_tension_scale:  1.0,
            non_newtonian_scale:    1.0,
            stiction_scale:         1.0,
            yield_scale:            1.0,
            capillary_scale:        1.0,
            sandpaper_scale:        1.0,
            brownian_scale:         1.0,
        }
    }
}

impl Default for LifeScalars {
    fn default() -> Self { Self::safe_default() }
}
```

- [ ] **Step 4: Add `scalars` field to LifeModule and getters**

Locate `pub struct LifeModule { ... }` in `src/dsp/modules/life.rs` (around line 200). Add at end of struct:

```rust
    scalars: LifeScalars,
```

In `LifeModule::new()` initialise it: `scalars: LifeScalars::safe_default(),`.

- [ ] **Step 5: Run default-correctness test**

Run: `cargo test --test scalar_life life_safe_default_matches_hardcoded_values`
Expected: PASS.

- [ ] **Step 6: Add trait method default in mod.rs**

In `src/dsp/modules/mod.rs`, near other `set_*_scalars` defaults (search for `set_past_scalars`), add:

```rust
    fn set_life_scalars(&mut self, _: crate::dsp::modules::life::LifeScalars) {}

    #[cfg(any(test, feature = "probe"))]
    fn test_life_scalars(&self) -> Option<crate::dsp::modules::life::LifeScalars> { None }
```

- [ ] **Step 7: Implement set_life_scalars on LifeModule**

In `src/dsp/modules/life.rs`, in the `impl SpectralModule for LifeModule { ... }` block, add:

```rust
    fn set_life_scalars(&mut self, scalars: crate::dsp::modules::life::LifeScalars) {
        self.scalars = scalars;
    }

    #[cfg(any(test, feature = "probe"))]
    fn test_life_scalars(&self) -> Option<crate::dsp::modules::life::LifeScalars> {
        Some(self.scalars)
    }
```

- [ ] **Step 8: Wire each per-mode kernel to multiply its hardcoded constant**

For each kernel, multiply its `*_AMOUNT_SCALE`-equivalent constant by the corresponding `self.scalars.<mode>_scale`. The wiring is mechanical — open `src/dsp/modules/life.rs` and apply these substitutions inside the relevant kernel functions:

| Kernel | Find | Replace with |
|---|---|---|
| Viscosity (~line 230) | `let d_max = VISCOSITY_D_MAX;` (or inline `* VISCOSITY_D_MAX` uses) | use `VISCOSITY_D_MAX * self.scalars.viscosity_scale` |
| Surface Tension (~292) | `let amt_max = SURFACE_TENSION_AMT_MAX;` | `let amt_max = SURFACE_TENSION_AMT_MAX * self.scalars.surface_tension_scale;` |
| Non-Newtonian (~469) | `.min(NON_NEWTONIAN_DISPLACEMENT_CAP);` | `.min(NON_NEWTONIAN_DISPLACEMENT_CAP * self.scalars.non_newtonian_scale);` |
| Stiction (~501) | `STICTION_DECAY_MIN + speed * STICTION_DECAY_RANGE` | `STICTION_DECAY_MIN + speed * STICTION_DECAY_RANGE * self.scalars.stiction_scale` |
| Yield (~553) | `YIELD_HEAL_MIN + speed * YIELD_HEAL_RANGE` | `YIELD_HEAL_MIN + speed * YIELD_HEAL_RANGE * self.scalars.yield_scale` |
| Capillary (~623) | `(amount_c[k] * CAPILLARY_AMOUNT_SCALE).clamp(0.0, CAPILLARY_AMOUNT_MAX)` | `(amount_c[k] * CAPILLARY_AMOUNT_SCALE * self.scalars.capillary_scale).clamp(0.0, CAPILLARY_AMOUNT_MAX * self.scalars.capillary_scale)` |
| Sandpaper (~near `SANDPAPER_AMOUNT_SCALE` use) | `* SANDPAPER_AMOUNT_SCALE` | `* SANDPAPER_AMOUNT_SCALE * self.scalars.sandpaper_scale` (also update the AMOUNT_MAX clamp identically) |
| Brownian (~near `BROWNIAN_AMOUNT_SCALE` use) | `* BROWNIAN_AMOUNT_SCALE` | `* BROWNIAN_AMOUNT_SCALE * self.scalars.brownian_scale` |

Use `grep` for each constant name to find all use sites and update consistently. The clamp ceilings (e.g. CAPILLARY_AMOUNT_MAX) must scale with the same multiplier so that scalar=2 doesn't clamp at the original ceiling.

- [ ] **Step 9: Verify default-correctness still passes**

Run: `cargo test --test scalar_life`
Expected: PASS — wiring multiplies by 1.0 at safe_default, so no behaviour change.

- [ ] **Step 10: Run full test suite**

Run: `cargo test`
Expected: all tests pass.

- [ ] **Step 11: Add fx_matrix dispatcher**

In `src/dsp/fx_matrix.rs`, after the existing `set_past_scalars` (line 326) and `test_past_scalars` (line 340), add:

```rust
    /// Propagate per-slot Life scalars from params to LifeModule instances.
    pub fn set_life_scalars(&mut self, scalars: &[crate::dsp::modules::life::LifeScalars; 9]) {
        for s in 0..MAX_SLOTS {
            if let Some(ref mut m) = self.slots[s] {
                m.set_life_scalars(scalars[s]);
            }
        }
    }

    #[cfg(any(test, feature = "probe"))]
    pub fn test_life_scalars(
        &self,
        slot: usize,
    ) -> Option<crate::dsp::modules::life::LifeScalars> {
        self.slots.get(slot)?.as_ref()?.test_life_scalars()
    }
```

- [ ] **Step 12: Add per-slot params via build.rs**

In `build.rs`, follow the pattern of `emit_past_scalar_fields` / `emit_past_scalar_inits` / `emit_past_scalar_dispatch` (lines 442-535). Add a new section after the Past one. The 8 Life suffixes are: `life_viscosity_scale`, `life_surface_tension_scale`, `life_non_newtonian_scale`, `life_stiction_scale`, `life_yield_scale`, `life_capillary_scale`, `life_sandpaper_scale`, `life_brownian_scale`.

Field declaration (per slot, per suffix):

```rust
writeln!(f, "    pub s{s}_{suffix}: FloatParam,").unwrap();
```

Init:

```rust
writeln!(
    f,
    "            s{s}_{suffix}: FloatParam::new(\"s{s}{suffix}\", 1.0f32, \
     FloatRange::Linear {{ min: 0.0f32, max: 2.0f32 }})\
     .with_smoother(SmoothingStyle::Linear(50.0))\
     .hide_in_generic_ui(),"
).unwrap();
```

Map entry (for state save/load):

```rust
let id = format!("s{s}{suffix}");
let rust_name = format!("s{s}_{suffix}");
writeln!(
    f,
    "        out.push(({id:?}.to_string(), self.{rust_name}.as_ptr(), String::new()));"
).unwrap();
```

Dispatch macro per suffix (replace `past_<suffix>_dispatch` → `<suffix>_dispatch`):

```rust
writeln!(f, "macro_rules! {suffix}_dispatch {{").unwrap();
// ... rest mirrors emit_past_scalar_dispatch
```

Call all three emit functions from `main()` (search for the existing `emit_past_scalar_*` calls and add Life ones after).

- [ ] **Step 13: Add params.rs accessor**

In `src/params.rs`, after `past_floor_param` (around line 729), add 8 helpers (one per suffix):

```rust
pub fn life_viscosity_scale_param(&self, slot: usize) -> Option<&FloatParam> {
    if slot >= 9 { return None; }
    Some(life_viscosity_scale_dispatch!(self, slot))
}
// ... repeat for other 7 suffixes
```

- [ ] **Step 14: Pipeline gather + dispatch**

In `src/dsp/pipeline.rs`, after the `set_past_scalars` block (~line 897), add:

```rust
{
    let mut life_scalars: [crate::dsp::modules::life::LifeScalars; 9] =
        std::array::from_fn(|_| crate::dsp::modules::life::LifeScalars::safe_default());
    for s in 0..9 {
        life_scalars[s] = crate::dsp::modules::life::LifeScalars {
            viscosity_scale:        params.life_viscosity_scale_param(s).map(|p| p.smoothed.next()).unwrap_or(1.0),
            surface_tension_scale:  params.life_surface_tension_scale_param(s).map(|p| p.smoothed.next()).unwrap_or(1.0),
            non_newtonian_scale:    params.life_non_newtonian_scale_param(s).map(|p| p.smoothed.next()).unwrap_or(1.0),
            stiction_scale:         params.life_stiction_scale_param(s).map(|p| p.smoothed.next()).unwrap_or(1.0),
            yield_scale:            params.life_yield_scale_param(s).map(|p| p.smoothed.next()).unwrap_or(1.0),
            capillary_scale:        params.life_capillary_scale_param(s).map(|p| p.smoothed.next()).unwrap_or(1.0),
            sandpaper_scale:        params.life_sandpaper_scale_param(s).map(|p| p.smoothed.next()).unwrap_or(1.0),
            brownian_scale:         params.life_brownian_scale_param(s).map(|p| p.smoothed.next()).unwrap_or(1.0),
        };
    }
    self.fx_matrix.set_life_scalars(&life_scalars);
}
```

- [ ] **Step 15: Add plumbing test**

Append to `tests/scalar_life.rs`:

```rust
#[test]
#[cfg(feature = "probe")]
fn life_scalars_round_trip_through_fx_matrix() {
    use spectral_forge::dsp::fx_matrix::FxMatrix;
    use spectral_forge::dsp::modules::{create_module, ModuleType};

    let mut fxm = FxMatrix::new();
    fxm.swap_slot(0, Some(create_module(ModuleType::Life)));

    let custom = LifeScalars {
        viscosity_scale: 0.25,
        ..LifeScalars::safe_default()
    };
    let mut arr = [LifeScalars::safe_default(); 9];
    arr[0] = custom;

    fxm.set_life_scalars(&arr);
    let read_back = fxm.test_life_scalars(0).expect("slot 0 should hold Life");
    assert!((read_back.viscosity_scale - 0.25).abs() < 1e-6);
}
```

Run: `cargo test --test scalar_life --features probe`
Expected: PASS.

- [ ] **Step 16: Create life_panel.rs (dev-build gated)**

Create `src/editor/life_panel.rs`:

```rust
//! Per-slot panel widget for `LifeModule` — dev-build only.
//!
//! Renders mode-conditional MASTER_SCALE knobs for the currently selected
//! Life mode. See spec docs/superpowers/specs/2026-05-09-prototyping-exposable-scalars-design.md §1.

#![cfg(feature = "dev-build")]

use nih_plug::prelude::ParamSetter;
use nih_plug_egui::egui::{self, Ui};

use crate::dsp::modules::life::LifeMode;
use crate::editor::theme as th;
use crate::params::SpectralForgeParams;

pub fn draw(ui: &mut Ui, params: &SpectralForgeParams, setter: &ParamSetter<'_>, slot: usize) {
    if slot >= 9 { return; }
    let scale = *params.ui_scale.lock();
    let mode  = params.slot_life_mode.lock()[slot];

    ui.horizontal(|ui| {
        match mode {
            LifeMode::Viscosity       => scalar_drag(ui, scale, setter, "Viscosity ×",      params.life_viscosity_scale_param(slot)),
            LifeMode::SurfaceTension  => scalar_drag(ui, scale, setter, "Surface Tension ×", params.life_surface_tension_scale_param(slot)),
            LifeMode::NonNewtonian    => scalar_drag(ui, scale, setter, "Non-Newtonian ×",   params.life_non_newtonian_scale_param(slot)),
            LifeMode::Stiction        => scalar_drag(ui, scale, setter, "Stiction ×",        params.life_stiction_scale_param(slot)),
            LifeMode::Yield           => scalar_drag(ui, scale, setter, "Yield ×",           params.life_yield_scale_param(slot)),
            LifeMode::Capillary       => scalar_drag(ui, scale, setter, "Capillary ×",       params.life_capillary_scale_param(slot)),
            LifeMode::Sandpaper       => scalar_drag(ui, scale, setter, "Sandpaper ×",       params.life_sandpaper_scale_param(slot)),
            LifeMode::Brownian        => scalar_drag(ui, scale, setter, "Brownian ×",        params.life_brownian_scale_param(slot)),
            // Crystallization, Archimedes have no scalar — panel renders empty.
            _ => {}
        }
    });
}

fn scalar_drag(
    ui: &mut Ui, scale: f32, setter: &ParamSetter<'_>,
    label: &str, param: Option<&nih_plug::prelude::FloatParam>,
) {
    if let Some(p) = param {
        ui.label(egui::RichText::new(label).size(th::FONT_SMALL * scale));
        let mut v = p.value();
        let resp = ui.add(
            egui::DragValue::new(&mut v).range(0.0..=2.0).speed(0.01).fixed_decimals(2),
        );
        if resp.changed() {
            setter.begin_set_parameter(p);
            setter.set_parameter(p, v.clamp(0.0, 2.0));
            setter.end_set_parameter(p);
        }
        if resp.drag_stopped() { setter.end_set_parameter(p); }
    }
}
```

- [ ] **Step 17: Wire panel_widget in ModuleSpec**

In `src/dsp/modules/mod.rs`, find `static LIFE: ModuleSpec = ModuleSpec { ... }`. Update its `panel_widget` field to:

```rust
panel_widget: {
    #[cfg(feature = "dev-build")]
    { Some(crate::editor::life_panel::draw as PanelWidgetFn) }
    #[cfg(not(feature = "dev-build"))]
    { None }
},
```

- [ ] **Step 18: Add `pub mod life_panel;` to src/editor/mod.rs**

```rust
#[cfg(feature = "dev-build")]
pub mod life_panel;
```

- [ ] **Step 19: Build both flavours**

Run:
```bash
cargo build
cargo build --features dev-build
```

Expected: both succeed.

- [ ] **Step 20: Commit**

```bash
git add -A
git commit -m "feat(life): expose per-mode MASTER_SCALE multipliers as dev-gated scalars

8 multipliers (one per Life mode that has an *_AMOUNT_SCALE constant):
viscosity, surface_tension, non_newtonian, stiction, yield, capillary,
sandpaper, brownian. Each defaults to 1.0 (current hardcoded behaviour),
range 0..2. Crystallization + Archimedes have no clean multiplier
semantics — panel renders empty for those modes.

FloatParams emitted via build.rs for every slot; pipeline gathers and
dispatches each block. Panel widget is dev-build-gated.

Spec: docs/superpowers/specs/2026-05-09-prototyping-exposable-scalars-design.md §1."
```

---

## Task 2: Kinetics — `KineticsScalars`

**Files:** same shape as Task 1, substituting `kinetics` for `life`.

The 7 scalars and their replacements:

| Scalar | Default | Range | Replaces |
|---|---|---|---|
| `sc_envelope_tau_hops` | 1.0 | 0.5..4.0 | `SC_ENVELOPE_TAU_HOPS` (line 75) |
| `sc_mass_rate_scale` | 5.0 | 0.5..10.0 | `SC_MASS_RATE_SCALE` (line 79) |
| `tuning_fork_min_sep` | 4.0 | 1.0..16.0 | `TUNING_FORK_MIN_SEP` (line 80) — `as usize` at use site |
| `orbital_sat_half_window` | 16.0 | 4.0..32.0 | `ORBITAL_SAT_HALF_WINDOW` (line 82) — `as usize` |
| `orbital_peak_threshold_factor` | 2.0 | 1.0..5.0 | `ORBITAL_PEAK_THRESHOLD_FACTOR` (line 86) |
| `static_well_baseline` | 1.05 | 1.0..2.0 | `STATIC_WELL_BASELINE` (line 88) |
| `sc_well_threshold_frac` | 0.4 | 0.1..0.9 | `SC_WELL_THRESHOLD_FRAC` (line 90) |

- [ ] **Step 1: Write default-correctness test**

Create `tests/scalar_kinetics.rs` with the 7-field default check (same shape as Task 1 Step 1).

- [ ] **Step 2: Define KineticsScalars in src/dsp/modules/kinetics.rs**

After `KineticsMode`/`WellSource`/`MassSource` enums:

```rust
#[derive(Clone, Copy, Debug)]
pub struct KineticsScalars {
    pub sc_envelope_tau_hops:          f32,
    pub sc_mass_rate_scale:            f32,
    pub tuning_fork_min_sep:           f32,
    pub orbital_sat_half_window:       f32,
    pub orbital_peak_threshold_factor: f32,
    pub static_well_baseline:          f32,
    pub sc_well_threshold_frac:        f32,
}

impl KineticsScalars {
    pub fn safe_default() -> Self {
        Self {
            sc_envelope_tau_hops:          1.0,
            sc_mass_rate_scale:            5.0,
            tuning_fork_min_sep:           4.0,
            orbital_sat_half_window:       16.0,
            orbital_peak_threshold_factor: 2.0,
            static_well_baseline:          1.05,
            sc_well_threshold_frac:        0.4,
        }
    }
}

impl Default for KineticsScalars {
    fn default() -> Self { Self::safe_default() }
}
```

- [ ] **Step 3: Add `scalars: KineticsScalars` field to KineticsModule and reset to safe_default in `new()`**

- [ ] **Step 4: Replace each constant use site with `self.scalars.<field>`**

For `tuning_fork_min_sep` and `orbital_sat_half_window`, the consumer expects `usize`. Cast: `self.scalars.tuning_fork_min_sep as usize`. Same for orbital window.

- [ ] **Step 5: Add trait method default + impl on KineticsModule** (mirror Task 1 Step 6-7).

- [ ] **Step 6: Add fx_matrix dispatcher** (mirror Task 1 Step 11).

- [ ] **Step 7: Add per-slot params via build.rs** (mirror Task 1 Step 12, 7 suffixes; ranges per the table above; defaults per the table).

- [ ] **Step 8: Add params.rs accessors** (7 helpers).

- [ ] **Step 9: Pipeline gather + dispatch** (mirror Task 1 Step 14, 7 fields).

- [ ] **Step 10: Plumbing test** (mirror Task 1 Step 15).

- [ ] **Step 11: Create kinetics_panel.rs**

Mode-conditional layout: `KineticsMode::GravityWell` → `static_well_baseline` (Static source) and `sc_well_threshold_frac` (Sidechain source) and `sc_envelope_tau_hops`. `InertialMass` → `sc_mass_rate_scale` and `sc_envelope_tau_hops`. `OrbitalPhase` → `orbital_sat_half_window` + `orbital_peak_threshold_factor`. `TuningFork` → `tuning_fork_min_sep`. Other modes render empty.

Use the same `scalar_drag` helper structure as `life_panel.rs`. For `tuning_fork_min_sep` and `orbital_sat_half_window`, use `fixed_decimals(0)` and step 1.0 to make the integer-ness visible.

- [ ] **Step 12: Wire panel_widget in ModuleSpec for KIN**

- [ ] **Step 13: Add `pub mod kinetics_panel;` (dev-gated) to src/editor/mod.rs**

- [ ] **Step 14: Build both flavours** (`cargo build` + `cargo build --features dev-build`).

- [ ] **Step 15: Run all tests** (`cargo test` + `cargo test --features probe`).

- [ ] **Step 16: Commit**

```bash
git add -A
git commit -m "feat(kinetics): expose 7 mode-specific tuning scalars as dev-gated knobs

sc_envelope_tau_hops, sc_mass_rate_scale, tuning_fork_min_sep,
orbital_sat_half_window, orbital_peak_threshold_factor,
static_well_baseline, sc_well_threshold_frac. Defaults match prior
hardcoded values; ranges per spec §2. Mode-conditional panel renders
only knobs relevant to the current Kinetics mode.

Spec: docs/superpowers/specs/2026-05-09-prototyping-exposable-scalars-design.md §2."
```

---

## Task 3: Circuit — `CircuitScalars` (Vactrol time constants)

**Files:** same shape as Task 1.

Two scalars, both Vactrol-only:

| Scalar | Default | Range | Replaces |
|---|---|---|---|
| `vactrol_fast_ms` | 8.0 | 1.0..50.0 | `VACTROL_TAU_FAST = 0.008` (s → ms × 1000) |
| `vactrol_slow_ms` | 250.0 | 50.0..1000.0 | `VACTROL_TAU_SLOW = 0.250` (s → ms × 1000) |

The DSP reads the current constants in seconds. Convert ms → s: kernel reads `self.scalars.vactrol_fast_ms * 1e-3`.

- [ ] **Step 1: Write default-correctness test** (`tests/scalar_circuit.rs`).

- [ ] **Step 2: Define CircuitScalars in src/dsp/modules/circuit.rs**

```rust
#[derive(Clone, Copy, Debug)]
pub struct CircuitScalars {
    pub vactrol_fast_ms: f32,
    pub vactrol_slow_ms: f32,
}

impl CircuitScalars {
    pub fn safe_default() -> Self {
        Self { vactrol_fast_ms: 8.0, vactrol_slow_ms: 250.0 }
    }
}

impl Default for CircuitScalars {
    fn default() -> Self { Self::safe_default() }
}
```

- [ ] **Step 3: Add `scalars` field to `CircuitModule`, init to safe_default in `new()`**

- [ ] **Step 4: Replace VACTROL_TAU_FAST and VACTROL_TAU_SLOW use sites in Vactrol kernel (line ~202-203)**

Substitute:
```rust
let tau_fast = (self.scalars.vactrol_fast_ms * 1e-3) * rel_scl;
let tau_slow = (self.scalars.vactrol_slow_ms * 1e-3) * rel_scl;
```

- [ ] **Step 5: Add trait method default + impl** (mirror Task 1).

- [ ] **Step 6: fx_matrix dispatcher** (mirror Task 1).

- [ ] **Step 7: build.rs codegen** (2 suffixes, ranges per table).

For `vactrol_fast_ms`:
```rust
"            s{s}_circuit_vactrol_fast_ms: FloatParam::new(\"s{s}circuit_vactrol_fast_ms\", 8.0f32, \
 FloatRange::Linear {{ min: 1.0f32, max: 50.0f32 }})\
 .with_smoother(SmoothingStyle::Linear(50.0))\
 .with_unit(\" ms\")\
 .hide_in_generic_ui(),"
```

For `vactrol_slow_ms`: same shape, default 250.0, range 50..1000.

- [ ] **Step 8: params.rs accessors (2)**

- [ ] **Step 9: Pipeline gather + dispatch**

- [ ] **Step 10: Plumbing test**

- [ ] **Step 11: Create circuit_panel.rs**

Mode-conditional: `CircuitMode::Vactrol` shows both knobs. Other modes render empty.

- [ ] **Step 12: Wire CIR ModuleSpec.panel_widget**

- [ ] **Step 13: Add `pub mod circuit_panel;` to src/editor/mod.rs**

- [ ] **Step 14: Build both flavours**

- [ ] **Step 15: Run all tests**

- [ ] **Step 16: Commit**

```bash
git add -A
git commit -m "feat(circuit): expose Vactrol fast/slow time constants as dev-gated scalars

vactrol_fast_ms (default 8 ms, range 1..50) and vactrol_slow_ms
(default 250 ms, range 50..1000). Kernel reads ms × 1e-3 to convert
back to seconds. Defaults reproduce VACTROL_TAU_FAST / VACTROL_TAU_SLOW
exactly. Panel only renders in Vactrol mode.

Spec: docs/superpowers/specs/2026-05-09-prototyping-exposable-scalars-design.md §3."
```

---

## Task 4: Modulate — `ModulateScalars` (PLL Tear damping + tear angle)

**Files:** same shape as Task 1.

Two scalars, PLL Tear-only:

| Scalar | Default | Range | Replaces |
|---|---|---|---|
| `damping` | 0.707 | 0.1..2.0 | `let zeta = 0.707_f32;` (modulate.rs:462) |
| `tear_angle_rad` | π/2 ≈ 1.5708 | π/8..π | `PLL_TEAR_THRESHOLD = π/2` (modulate.rs:570) |

- [ ] **Step 1: Write default-correctness test** (`tests/scalar_modulate.rs`).

```rust
use spectral_forge::dsp::modules::modulate::ModulateScalars;
use std::f32::consts::FRAC_PI_2;

#[test]
fn modulate_safe_default_matches_hardcoded_values() {
    let s = ModulateScalars::safe_default();
    assert!((s.damping - 0.707).abs() < 1e-6);
    assert!((s.tear_angle_rad - FRAC_PI_2).abs() < 1e-6);
}
```

- [ ] **Step 2: Define ModulateScalars in src/dsp/modules/modulate.rs**

```rust
#[derive(Clone, Copy, Debug)]
pub struct ModulateScalars {
    pub damping:         f32,
    pub tear_angle_rad:  f32,
}

impl ModulateScalars {
    pub fn safe_default() -> Self {
        Self {
            damping:         0.707,
            tear_angle_rad:  std::f32::consts::FRAC_PI_2,
        }
    }
}

impl Default for ModulateScalars {
    fn default() -> Self { Self::safe_default() }
}
```

- [ ] **Step 3: Add `scalars` field to `ModulateModule`, init in `new()`**

- [ ] **Step 4: Wire damping in apply_pll_tear (line 462)**

Replace `let zeta = 0.707_f32;` with `let zeta = self.scalars.damping;`. Need to change `apply_pll_tear` signature (and call site) to take `&self` or a `damping: f32` argument — pick the cleanest route. If it's a free fn (not method), pass `damping` as a parameter and have `process()` extract it from `self.scalars`.

- [ ] **Step 5: Wire tear_angle in apply_pll_tear (line ~471)**

Replace `PLL_TEAR_THRESHOLD * thresh_scale.min(2.0)` with `self.scalars.tear_angle_rad * thresh_scale.min(2.0)` (or pass `tear_angle` as a parameter to `apply_pll_tear`).

- [ ] **Step 6: Add trait method default + impl** (mirror Task 1).

- [ ] **Step 7: fx_matrix dispatcher**.

- [ ] **Step 8: build.rs codegen** (2 suffixes; tear_angle uses `\" rad\"` unit).

- [ ] **Step 9: params.rs accessors (2)**.

- [ ] **Step 10: Pipeline gather + dispatch**.

- [ ] **Step 11: Plumbing test**.

- [ ] **Step 12: Create modulate_panel.rs**

Mode-conditional: `ModulateMode::PllTear` shows both knobs. Other modes render empty.

- [ ] **Step 13: Wire MODULATE ModuleSpec.panel_widget**

- [ ] **Step 14: Add `pub mod modulate_panel;` to src/editor/mod.rs**

- [ ] **Step 15: Build both flavours**

- [ ] **Step 16: Run all tests**

- [ ] **Step 17: Commit**

```bash
git add -A
git commit -m "feat(modulate): expose PLL Tear damping + tear angle as dev-gated scalars

damping (default 0.707, range 0.1..2.0) replaces let zeta = 0.707_f32
in apply_pll_tear. tear_angle_rad (default pi/2, range pi/8..pi)
replaces PLL_TEAR_THRESHOLD. Panel renders only for PllTear mode;
audit-claimed GravityPhaser zeta usage was inaccurate — that mode
has its own 0.95 momentum decay and is not affected.

Spec: docs/superpowers/specs/2026-05-09-prototyping-exposable-scalars-design.md §4."
```

---

## Task 5: Contrast — modes + scalars

Adds `ContrastMode` enum (Spatial / Temporal / Tilt), two new DSP kernels in `spectral_contrast.rs`, mode dispatch in the module wrapper, and `ContrastScalars { mean_window_st, tilt_slope_db_per_oct }`.

**Files:**
- Modify: `src/dsp/engines/spectral_contrast.rs` (Spatial kernel kept; add Temporal + Tilt; add `process_bins_temporal`, `process_bins_tilt` methods or a mode parameter)
- Modify: `src/dsp/modules/contrast.rs` (struct fields, mode dispatch, scalars wiring)
- Modify: all the Task 1-style plumbing files (mod.rs, fx_matrix.rs, pipeline.rs, params.rs, build.rs)
- Create: `src/editor/contrast_panel.rs`
- Modify: `src/editor/mod.rs`
- Test: `tests/scalar_contrast.rs` (extend, not replace — Task 0's THRESHOLD test is already there)

- [ ] **Step 1: Write the Temporal-mode test (engine-level)**

Append to `tests/scalar_contrast.rs`:

```rust
#[test]
fn contrast_temporal_converges_to_unity_on_steady_input() {
    // Temporal mode: each bin compared against its own long-running mean.
    // After enough blocks for the time-constant to converge, current = mean
    // → zero deviation → unity gain. Output magnitudes should match input
    // magnitudes (within tolerance) regardless of bin-to-bin shape.
    let mut engine = SpectralContrastEngine::new();
    engine.reset(48_000.0, 1024);
    let n = 513;

    let attack:  Vec<f32> = vec![10.0;  n];
    let release: Vec<f32> = vec![100.0; n];
    let knee:    Vec<f32> = vec![0.0;   n];
    let makeup:  Vec<f32> = vec![0.0;   n];
    let mix:     Vec<f32> = vec![1.0;   n];
    let thresh:  Vec<f32> = vec![-200.0; n];  // bypass disabled
    let ratio:   Vec<f32> = vec![5.0;   n];   // strong expand

    // Asymmetric spectrum: bin 200 louder than rest. Spatial mode would push
    // bin 200 even higher (it deviates from neighbours). Temporal mode should
    // leave every bin unchanged once each bin's per-bin mean has converged.
    let original_input: Vec<Complex<f32>> = (0..n).map(|k| {
        if k == 200 { Complex::new(1.0, 0.0) } else { Complex::new(0.1, 0.0) }
    }).collect();

    let mut suppression: Vec<f32> = vec![0.0; n];
    let mut output_after = original_input.clone();

    for _ in 0..200 {
        let mut bins = original_input.clone();
        let params = BinParams {
            threshold_db: &thresh, ratio: &ratio, attack_ms: &attack, release_ms: &release,
            knee_db: &knee, makeup_db: &makeup, mix: &mix, smoothing_semitones: 1.0,
            sensitivity: 1.0, auto_makeup: false,
            peaks: None, plpv_dynamics_enabled: false,
        };
        engine.process_bins_temporal(&mut bins, &params, 48_000.0, &mut suppression);
        output_after = bins;
    }

    for k in 0..n {
        let in_mag  = original_input[k].norm();
        let out_mag = output_after[k].norm();
        assert!((out_mag - in_mag).abs() < 0.05,
            "bin {k}: in={in_mag:.4} out={out_mag:.4} (Temporal should converge to unity gain)");
    }
}
```

- [ ] **Step 2: Define ContrastMode + ContrastScalars in src/dsp/modules/contrast.rs**

```rust
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ContrastMode {
    #[default]
    Spatial,
    Temporal,
    Tilt,
}

#[derive(Clone, Copy, Debug)]
pub struct ContrastScalars {
    pub mean_window_st:        f32,  // Spatial only — log-freq neighbourhood width
    pub tilt_slope_db_per_oct: f32,  // Tilt only
}

impl ContrastScalars {
    pub fn safe_default() -> Self {
        Self { mean_window_st: 1.0, tilt_slope_db_per_oct: 0.0 }
    }
}

impl Default for ContrastScalars {
    fn default() -> Self { Self::safe_default() }
}
```

- [ ] **Step 3: Add `mode: ContrastMode` and `scalars: ContrastScalars` to ContrastModule**

Init in `new()`. Add `set_mode(&mut self, mode: ContrastMode) { self.mode = mode; }`.

- [ ] **Step 4: Add Temporal + Tilt kernels in src/dsp/engines/spectral_contrast.rs**

Add two new methods on `SpectralContrastEngine`:

```rust
pub fn process_bins_temporal(
    &mut self,
    bins: &mut [Complex<f32>],
    params: &BinParams<'_>,
    sample_rate: f32,
    suppression_out: &mut [f32],
) {
    debug_assert_eq!(bins.len(), self.num_bins);
    let n   = bins.len();
    let hop = self.hop_size;

    // Per-bin temporal mean tracker (uses release_ms for the time constant —
    // attack tracks rising magnitudes quickly, release tracks falling).
    for k in 0..n {
        let attack_ms  = params.attack_ms[k].max(0.1);
        let release_ms = params.release_ms[k].max(1.0);
        let mag = bins[k].norm();
        let coeff = if mag > self.contrast_env[k] {
            ms_to_coeff(attack_ms,  sample_rate, hop)
        } else {
            ms_to_coeff(release_ms, sample_rate, hop)
        };
        self.contrast_env[k] = coeff * self.contrast_env[k] + (1.0 - coeff) * mag;

        let env = self.contrast_env[k].max(1e-10);
        let deviation_db = (20.0 * (mag / env).log10()).clamp(-48.0, 48.0);
        let ratio   = params.ratio[k].clamp(0.0, 20.0);
        let knee_db = params.knee_db[k].max(0.0);
        self.gr_db[k] = Self::contrast_gain(deviation_db, ratio, knee_db);
    }
    // Reuse the smooth + auto-makeup + apply passes (Pass 3 + 4) from
    // process_bins. Factor those into a helper function to avoid duplication.
    self.smooth_and_apply(bins, params, sample_rate, suppression_out);
}

pub fn process_bins_tilt(
    &mut self,
    bins: &mut [Complex<f32>],
    params: &BinParams<'_>,
    fft_size: usize,
    sample_rate: f32,
    slope_db_per_oct: f32,
    suppression_out: &mut [f32],
) {
    debug_assert_eq!(bins.len(), self.num_bins);
    let n = bins.len();

    // Reference: at 1 kHz, expected_db = bin0 average (or fixed). Slope adds
    // slope_db_per_oct × log2(freq/1000) to the reference, per bin.
    // For simplicity baseline = average dBFS across bins; this gives a
    // self-tuning reference that adapts to overall loudness.
    let mut sum_db = 0.0f32;
    let mut count = 0u32;
    for k in 1..n {
        let mag = bins[k].norm().max(1e-10);
        sum_db += 20.0 * mag.log10();
        count += 1;
    }
    let baseline_db = sum_db / count.max(1) as f32;

    for k in 0..n {
        let freq_hz = (k as f32 * sample_rate / fft_size as f32).max(20.0);
        let oct_from_1k = (freq_hz / 1000.0).log2();
        let expected_db = baseline_db + slope_db_per_oct * oct_from_1k;
        let mag_db = 20.0 * bins[k].norm().max(1e-10).log10();
        let deviation_db = (mag_db - expected_db).clamp(-48.0, 48.0);
        let ratio   = params.ratio[k].clamp(0.0, 20.0);
        let knee_db = params.knee_db[k].max(0.0);
        self.gr_db[k] = Self::contrast_gain(deviation_db, ratio, knee_db);
    }
    self.smooth_and_apply(bins, params, sample_rate, suppression_out);
}

// Factor out Pass 3 + Pass 4 from existing process_bins into:
fn smooth_and_apply(
    &mut self,
    bins: &mut [Complex<f32>],
    params: &BinParams<'_>,
    sample_rate: f32,
    suppression_out: &mut [f32],
) {
    // ... existing Pass 3 + Pass 4 body, exactly as it appears in process_bins
    // (post-Task-0 with THRESHOLD bypass).
}
```

(Refactoring Pass 3+4 into `smooth_and_apply` is cleanly done by cut-paste from `process_bins` so the original Spatial mode keeps the same behaviour.)

- [ ] **Step 5: Replace Spatial kernel's hardcoded `params.smoothing_semitones.max(1.0)` with the scalar**

In `process_bins`, the line `let width_ratio = 2.0f32.powf(params.smoothing_semitones.max(1.0) / 12.0);` becomes the engine reading `mean_window_st` instead. Pass `mean_window_st` as a method parameter:

```rust
pub fn process_bins_spatial(
    &mut self,
    bins: &mut [Complex<f32>],
    params: &BinParams<'_>,
    sample_rate: f32,
    mean_window_st: f32,
    suppression_out: &mut [f32],
) {
    // ... existing body, but width_ratio uses mean_window_st instead:
    let width_ratio = 2.0f32.powf(mean_window_st.max(0.1) / 12.0);
    // ... rest unchanged
}
```

(Rename old `process_bins` → `process_bins_spatial`. Keep `process_bins` as a thin wrapper that defaults `mean_window_st = 1.0` for backward compatibility with any callers.)

- [ ] **Step 6: Wire mode dispatch in ContrastModule::process**

In `src/dsp/modules/contrast.rs`, the existing `engine.process_bins(...)` call becomes:

```rust
match self.mode {
    ContrastMode::Spatial => {
        self.engine.process_bins_spatial(
            bins, &params, ctx.sample_rate,
            self.scalars.mean_window_st,
            suppression_out,
        );
    }
    ContrastMode::Temporal => {
        self.engine.process_bins_temporal(bins, &params, ctx.sample_rate, suppression_out);
    }
    ContrastMode::Tilt => {
        self.engine.process_bins_tilt(
            bins, &params, ctx.fft_size, ctx.sample_rate,
            self.scalars.tilt_slope_db_per_oct,
            suppression_out,
        );
    }
}
```

- [ ] **Step 7: Add trait methods**

`set_contrast_mode`, `set_contrast_scalars`, `test_contrast_scalars`. Defaults in `mod.rs`, impls in `contrast.rs`.

- [ ] **Step 8: Add fx_matrix dispatchers** (set_contrast_modes + set_contrast_scalars + test_contrast_scalars).

- [ ] **Step 9: Add per-slot params via build.rs**

`s{s}_contrast_mean_window_st` (FloatParam: range 0.1..24.0, default 1.0, unit `" st"`),
`s{s}_contrast_tilt_slope_db_per_oct` (FloatParam: range -6.0..6.0, default 0.0, unit `" dB/oct"`).

For `s{s}_contrast_mode`, an `EnumParam<ContrastMode>` is needed. nih-plug's EnumParam takes the type directly — codegen needs slightly different shape than FloatParam. Add a new emit fn `emit_contrast_mode_fields` / `emit_contrast_mode_inits`:

```rust
writeln!(f, "    pub s{s}_contrast_mode: EnumParam<crate::dsp::modules::contrast::ContrastMode>,").unwrap();
// init:
writeln!(f, "            s{s}_contrast_mode: EnumParam::new(\"s{s}contrast_mode\", \
    crate::dsp::modules::contrast::ContrastMode::Spatial)\
    .hide_in_generic_ui(),").unwrap();
```

- [ ] **Step 10: Add params.rs accessors (3: mode + 2 floats)**

- [ ] **Step 11: Pipeline gather + dispatch**

Gather modes into `[ContrastMode; 9]`, scalars into `[ContrastScalars; 9]`, dispatch via `fx_matrix.set_contrast_modes(...)` and `fx_matrix.set_contrast_scalars(...)`.

- [ ] **Step 12: Plumbing test** (mode round-trip + scalar round-trip).

- [ ] **Step 13: Create contrast_panel.rs (dev-gated)**

Layout: always show a 3-button mode picker (Spatial / Temporal / Tilt). Then mode-conditional knobs:
- Spatial → `mean_window_st` knob
- Temporal → no extra knob (uses ATTACK/RELEASE curves)
- Tilt → `tilt_slope_db_per_oct` knob

Mode buttons should call `setter.set_parameter(s{s}_contrast_mode, mode)`.

- [ ] **Step 14: Wire CON ModuleSpec.panel_widget**

- [ ] **Step 15: Add `pub mod contrast_panel;` (dev-gated) to src/editor/mod.rs**

- [ ] **Step 16: Build both flavours**

- [ ] **Step 17: Run all tests**

- [ ] **Step 18: Commit**

```bash
git add -A
git commit -m "feat(contrast): add Temporal + Tilt modes; expose mean window + tilt slope

Three-mode dispatch in ContrastModule: Spatial (current behaviour,
default), Temporal (per-bin deviation from each bin's own long-running
mean — uses ATTACK/RELEASE curves as the time constants), Tilt
(per-bin deviation from a fitted slope_db_per_oct reference).

ContrastScalars: mean_window_st (Spatial), tilt_slope_db_per_oct (Tilt).
Mode-conditional dev-gated panel renders mode picker + relevant scalars.

Spec: docs/superpowers/specs/2026-05-09-prototyping-exposable-scalars-design.md §5."
```

---

## Task 6: PhaseSmear — `PHASE_RANGE` curve

**Files:**
- Modify: `src/dsp/modules/phase_smear.rs` (num_curves 3 → 4, read curve idx 3)
- Modify: `src/dsp/modules/mod.rs` (PSM ModuleSpec curve_labels + num_curves)
- Modify: `src/editor/curve_config.rs` (phase_smear_config curve_idx=3 entry)
- Test: `tests/scalar_phase_smear.rs` (new file) + extension to `tests/curve_config.rs`

- [ ] **Step 1: Write the calibration test**

Create `tests/scalar_phase_smear.rs`:

```rust
//! PhaseSmear PHASE_RANGE curve calibration + DSP plumbing.
use spectral_forge::editor::curve_config::curve_display_config;
use spectral_forge::dsp::modules::{ModuleType, GainMode};

#[test]
fn phase_smear_phase_range_curve_config_present() {
    let cfg = curve_display_config(ModuleType::PhaseSmear, 3, GainMode::Add);
    assert_eq!(cfg.y_label, "× π", "PHASE_RANGE should show as multiples of pi");
    assert!((cfg.y_min - 0.0).abs() < 1e-6);
    assert!((cfg.y_max - 2.0).abs() < 1e-6);
    assert!((cfg.y_natural - 1.0).abs() < 1e-6);
}
```

- [ ] **Step 2: Run test — expect failure**

Run: `cargo test --test scalar_phase_smear`
Expected: FAIL — index 3 currently returns the default config.

- [ ] **Step 3: Add curve_idx=3 entry in src/editor/curve_config.rs phase_smear_config**

Find `pub fn phase_smear_config(...)` (search for "fn phase_smear_config"). Add a `3 => CurveDisplayConfig { ... }` arm:

```rust
3 => CurveDisplayConfig {
    y_label: "× π", y_min: 0.0, y_max: 2.0, y_log: false,
    grid_lines: &[(0.5, "0.5×π"), (1.0, "π"), (1.5, "1.5×π"), (2.0, "2×π")],
    y_natural: 1.0,
    offset_fn: off_amount_200,
    natural_at_max: false,
},
```

- [ ] **Step 4: Run calibration test — expect pass**

Run: `cargo test --test scalar_phase_smear`
Expected: PASS.

- [ ] **Step 5: Update PSM ModuleSpec in src/dsp/modules/mod.rs (line 540-541)**

```rust
num_curves: 4,
curve_labels: &["AMOUNT", "PEAK HOLD", "MIX", "PHASE_RANGE"],
```

- [ ] **Step 6: Update PhaseSmearModule::num_curves() (line 164)**

```rust
fn num_curves(&self) -> usize { 4 }
```

- [ ] **Step 7: Wire PHASE_RANGE in DSP (phase_smear.rs:107)**

Find the line `... = ... * std::f32::consts::PI` (around line 107). Replace the `std::f32::consts::PI` factor with a curve read:

```rust
let phase_range_g = curves.get(3).and_then(|c| c.get(k)).copied().unwrap_or(1.0);
let max_phase = phase_range_g * std::f32::consts::PI;
// ... use max_phase wherever the literal PI was previously
```

- [ ] **Step 8: Run full test suite**

Run: `cargo test`
Expected: all tests pass, including the existing `tests/curve_calibration_matrix.rs` which now covers idx 3 of PhaseSmear automatically.

- [ ] **Step 9: Verify in dev plugin**

Run: `cargo build --features dev-build && cargo run --quiet --package xtask -- bundle spectral_forge --release --features dev-build && cp -r target/bundled/spectral_forge.clap ~/.clap/spectral/dev/spectral_dev.clap`

Open Bitwig, instantiate Spectral Forge (Dev), assign a slot to PhaseSmear. Click through to the 4th curve tab; verify it shows "PHASE_RANGE" and the curve grid shows π / 2π marks. Drag a node up — high freq smearing increases.

- [ ] **Step 10: Commit**

```bash
git add -A
git commit -m "feat(phase_smear): expose per-bin PHASE_RANGE as 4th curve

Curve gain 1.0 = pi (current hardcoded behaviour); 2.0 = 2*pi (full
rotations); 0 = no smearing per bin. Display unit '× π' with quartile
grid lines. Calibration via off_amount_200 + y_natural=1.0
+ natural_at_max=false matches the existing AMOUNT 0..200% pattern.

Spec: docs/superpowers/specs/2026-05-09-prototyping-exposable-scalars-design.md §6."
```

---

## Final regression sweep

After all 7 tasks complete:

- [ ] **Run full test suite both feature variants**

```bash
cargo test
cargo test --features probe
cargo test --features dev-build
```

Expected: 0 failures across the board.

- [ ] **Build dev + release**

```bash
cargo build --release
cargo build --release --features dev-build
```

Both succeed.

- [ ] **Reinstall dev plugin and smoke-check**

```bash
cargo run --quiet --package xtask -- bundle spectral_forge --release --features dev-build
cp -r target/bundled/spectral_forge.clap ~/.clap/spectral/dev/spectral_dev.clap
```

Bitwig: load a patch, switch each module type through the slot, verify the dev panel shows the new knobs only when in the right mode. No knobs visible on production build.

- [ ] **Push branch**

```bash
git push origin feature/next-gen-modules-plans
```
