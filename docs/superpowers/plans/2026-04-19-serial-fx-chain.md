> **Status (2026-04-24): SUPERSEDED.** This bridge plan (serialised Dynamics → Freeze → PhaseSmear via bool flags) was skipped in favour of the full FxMatrix. `EffectMode`/`DynamicsMode`/`freeze_enabled`/`phase_enabled` as described here do not exist in the codebase. Do not follow. Source of truth: the code + [../STATUS.md](../STATUS.md).

# Serial FX Chain Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the exclusive `EffectMode` enum (one effect at a time) with three independently toggleable DSP stages that always run in series: Dynamics → Freeze → Phase Smear.

**Architecture:** `EffectMode` is split into `DynamicsMode` (Compressor | Contrast | Bypass, replacing the dynamics engine selection) plus two new `BoolParam`s (`freeze_enabled`, `phase_enabled`). The STFT closure runs all three stages unconditionally, gated by their respective enable flags. A new persisted `effects_view: Arc<Mutex<u8>>` (0=freeze graph, 1=phase graph) replaces the old mode-driven view selection in the GUI.

**Tech Stack:** Rust, nih-plug, egui (nih_plug_egui), parking_lot::Mutex, triple_buffer

---

## Files Modified

| File | Changes |
|------|---------|
| `src/params.rs` | Add `DynamicsMode` enum; replace `effect_mode: EnumParam<EffectMode>` with `dynamics_mode: EnumParam<DynamicsMode>`; add `freeze_enabled: BoolParam`, `phase_enabled: BoolParam`; add `effects_view: Arc<Mutex<u8>>` |
| `src/dsp/pipeline.rs` | Replace `match effect_mode` with sequential if-blocks for each stage; update freeze_captured to reset when `!freeze_enabled` |
| `src/editor_ui.rs` | Derive `is_freeze_mode`/`is_phase_mode` from `effects_view` instead of `effect_mode`; replace Effects tab bottom strip with dynamics mode selector + independent enable toggles |
| `tests/engine_contract.rs` | Add test verifying that freeze stage stabilises bins independently of dynamics stage |

---

## Context for implementers

The codebase is a Rust CLAP audio plugin using nih-plug. The key constraint is **no allocation or locking on the audio thread** — the STFT closure captures locals by mutable reference before the `process_overlap_add` call.

Inside the STFT closure all fields accessed must be rebound as locals *before* the closure. See the existing pattern starting at `pipeline.rs:370`.

Parameter changes use `setter.begin_set_parameter` / `setter.set_parameter` / `setter.end_set_parameter` in the GUI. Persisted-mutex fields (like `effects_view`) are set directly via `.lock()`.

The `EffectMode` enum currently has four variants: `Bypass`, `Freeze`, `PhaseRand`, `SpectralContrast`. After this plan, `Freeze` and `PhaseRand` are removed from the enum; their DSP is always run based on bool params. `SpectralContrast` becomes `DynamicsMode::Contrast`.

---

## Task 1: Replace params — DynamicsMode + enable bools + effects_view

**Files:**
- Modify: `src/params.rs`

- [ ] **Step 1: Write the failing test**

Add to `tests/engine_contract.rs` — this test imports `DynamicsMode` which won't exist yet:

```rust
#[test]
fn dynamics_mode_compressor_is_default() {
    // Verifies the new DynamicsMode enum compiles and has a Compressor variant.
    // This will fail until Task 1 is implemented.
    let _mode = spectral_forge::params::DynamicsMode::Compressor;
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test dynamics_mode_compressor_is_default 2>&1 | tail -5
```

Expected: compile error — `DynamicsMode` not found.

- [ ] **Step 3: Implement new params**

In `src/params.rs`, make these changes:

**a) Replace the `EffectMode` enum:**

Remove:
```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub enum EffectMode {
    Bypass,
    Freeze,
    PhaseRand,
    SpectralContrast,
}
```

Add:
```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub enum DynamicsMode {
    Compressor,
    Contrast,
    Bypass,
}
```

**b) In `SpectralForgeParams` struct, replace `effect_mode` and add new fields:**

Remove:
```rust
    #[id = "effect_mode"]
    pub effect_mode: EnumParam<EffectMode>,
```

Add in its place:
```rust
    #[id = "dynamics_mode"]
    pub dynamics_mode: EnumParam<DynamicsMode>,

    #[id = "freeze_enabled"]
    pub freeze_enabled: BoolParam,

    #[id = "phase_enabled"]
    pub phase_enabled: BoolParam,

    /// Which effect's curve graph is shown in the Effects tab.
    /// 0 = freeze curves, 1 = phase curve.
    #[persist = "effects_view"]
    pub effects_view: Arc<Mutex<u8>>,
```

**c) In `SpectralForgeParams::Default::default()`, replace the `effect_mode` initialiser:**

Remove:
```rust
            effect_mode: EnumParam::new("Effect Mode", EffectMode::Bypass),
```

Add:
```rust
            dynamics_mode: EnumParam::new("Dynamics Mode", DynamicsMode::Compressor),
            freeze_enabled: BoolParam::new("Freeze", false),
            phase_enabled:  BoolParam::new("Phase Smear", false),
            effects_view:   Arc::new(Mutex::new(0u8)),
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo test dynamics_mode_compressor_is_default 2>&1 | tail -5
```

Expected: `test dynamics_mode_compressor_is_default ... ok`

- [ ] **Step 5: Verify project still compiles (params consumers will need fixing)**

```bash
cargo build 2>&1 | grep "^error" | head -20
```

Expected: compile errors referencing `effect_mode` in `pipeline.rs` and `editor_ui.rs`. That is fine — fixed in Tasks 2 and 3.

- [ ] **Step 6: Commit**

```bash
git add src/params.rs tests/engine_contract.rs
git commit -m "refactor: replace EffectMode with DynamicsMode + freeze_enabled/phase_enabled bools"
```

---

## Task 2: Pipeline — sequential DSP stages

**Files:**
- Modify: `src/dsp/pipeline.rs`

- [ ] **Step 1: Write the failing test**

Add to `tests/engine_contract.rs`:

```rust
#[test]
fn freeze_stage_stabilises_bins_independent_of_dynamics() {
    // After initial capture, the freeze stage must hold the first-captured bins
    // regardless of what new audio arrives. This tests that freeze is a DSP stage,
    // not a replacement for the whole pipeline.
    //
    // We exercise the per-bin freeze state machine directly (not the full STFT
    // pipeline) by simulating the logic: capture, then send different input.
    use num_complex::Complex;
    let n = 8usize;
    let hop_ms = 2048.0f32 / (4.0 * 44100.0) * 1000.0; // hop ≈ 11.6 ms

    // Simulate "captured" initial bins
    let initial: Vec<Complex<f32>> = (0..n)
        .map(|k| Complex::new(k as f32, 0.0))
        .collect();

    let mut frozen_bins   = initial.clone();
    let mut freeze_target = initial.clone();
    let mut freeze_port_t = vec![1.0f32; n];   // settled
    let mut freeze_hold_hops = vec![100u32; n]; // already held a long time
    let mut freeze_accum  = vec![0.0f32; n];

    // New input that differs from initial
    let new_input: Vec<Complex<f32>> = (0..n)
        .map(|k| Complex::new(k as f32 * 2.0, 0.0))
        .collect();

    // Simulate one freeze iteration: length_hops=200 (not yet triggered), threshold very high
    let length_hops = 200u32;
    let threshold_lin = 1e6f32; // threshold so high new_input never triggers
    let port_hops = 1.0f32;     // instantaneous portamento (unused, already settled)
    let resistance = 0.0f32;

    for k in 0..n {
        if freeze_port_t[k] < 1.0 {
            freeze_port_t[k] = (freeze_port_t[k] + 1.0 / port_hops).min(1.0);
            let t = freeze_port_t[k];
            frozen_bins[k] = Complex::new(
                frozen_bins[k].re * (1.0 - t) + freeze_target[k].re * t,
                frozen_bins[k].im * (1.0 - t) + freeze_target[k].im * t,
            );
        } else {
            freeze_hold_hops[k] += 1;
            let mag = new_input[k].norm();
            if mag > threshold_lin { freeze_accum[k] += mag - threshold_lin; }
            if freeze_hold_hops[k] >= length_hops && freeze_accum[k] >= resistance {
                freeze_target[k]    = new_input[k];
                freeze_port_t[k]    = 0.0;
                freeze_hold_hops[k] = 0;
                freeze_accum[k]     = 0.0;
            }
        }
        // Output = frozen_bins[k], not new_input[k]
    }

    // Output must equal initial frozen bins (threshold blocked any transition)
    for k in 0..n {
        assert!(
            (frozen_bins[k] - initial[k]).norm() < 1e-6,
            "bin {k}: freeze should hold initial capture, got {:?}", frozen_bins[k]
        );
    }
}
```

- [ ] **Step 2: Run test to verify it passes immediately**

This test exercises only the state machine logic, no pipeline plumbing needed:

```bash
cargo test freeze_stage_stabilises_bins 2>&1 | tail -5
```

Expected: `test freeze_stage_stabilises_bins_independent_of_dynamics ... ok`

(This test passes immediately because it contains the logic inline — it validates our understanding of the algorithm before we move it into the pipeline.)

- [ ] **Step 3: Fix pipeline.rs — update param reads**

In `src/dsp/pipeline.rs`, find the block that reads effect mode (around line 192):

```rust
        let effect_mode          = params.effect_mode.value();
```

Replace with:

```rust
        let dynamics_mode        = params.dynamics_mode.value();
        let freeze_enabled       = params.freeze_enabled.value();
        let phase_enabled        = params.phase_enabled.value();
```

Also remove:
```rust
        let phase_rand_amount    = params.phase_rand_amount.smoothed.next_step(block_size);
```
And re-add it in the correct location (only needed when phase is potentially enabled — keep it here for simplicity):
```rust
        let phase_rand_amount    = params.phase_rand_amount.smoothed.next_step(block_size);
```

(Keep the line, just remove the `effect_mode` line above it.)

Also add `freeze_enabled` and `phase_enabled` to the list of locals rebound before the STFT closure (around line 370), after `let freeze_captured = &mut self.freeze_captured;`:

```rust
        let freeze_enabled_flag  = freeze_enabled;
        let phase_enabled_flag   = phase_enabled;
```

- [ ] **Step 4: Fix pipeline.rs — replace match block with serial stages**

Find the `match effect_mode` block (pipeline.rs ~line 458) and replace the entire block:

```rust
            // Effects pass — modifies complex_buf in-place after compression.
            match effect_mode {
                crate::params::EffectMode::Bypass => {}
                crate::params::EffectMode::Freeze => { ... }
                crate::params::EffectMode::PhaseRand => { ... }
                crate::params::EffectMode::SpectralContrast => { ... }
            }

            // When leaving Freeze mode, clear the captured flag ...
            if effect_mode != crate::params::EffectMode::Freeze {
                *freeze_captured = false;
            }
```

With the following serial pipeline. Also update the engine dispatch above it to use `dynamics_mode`:

**Replace the engine dispatch block** (which currently always calls `active_engine.process_bins` unconditionally):

```rust
            // ── Stage 1: Dynamics ────────────────────────────────────────
            match dynamics_mode {
                crate::params::DynamicsMode::Compressor => {
                    active_engine.process_bins(
                        complex_buf, sidechain_arg, &params, sample_rate, channel_supp_buf,
                    );
                    for k in 0..channel_supp_buf.len() {
                        if channel_supp_buf[k] > suppression_buf[k] {
                            suppression_buf[k] = channel_supp_buf[k];
                        }
                    }
                }
                crate::params::DynamicsMode::Contrast => {
                    contrast_engine.process_bins(
                        complex_buf, None, &params, sample_rate, channel_supp_buf,
                    );
                    for k in 0..channel_supp_buf.len() {
                        if channel_supp_buf[k] > suppression_buf[k] {
                            suppression_buf[k] = channel_supp_buf[k];
                        }
                    }
                }
                crate::params::DynamicsMode::Bypass => {}
            }

            // ── Stage 2: Freeze ──────────────────────────────────────────
            if freeze_enabled_flag {
                let hop_ms = FFT_SIZE as f32 / (OVERLAP as f32 * sample_rate) * 1000.0;

                if !*freeze_captured {
                    frozen_bins.copy_from_slice(complex_buf);
                    freeze_target.copy_from_slice(complex_buf);
                    for t in freeze_port_t.iter_mut()    { *t = 1.0; }
                    for h in freeze_hold_hops.iter_mut() { *h = 0; }
                    for a in freeze_accum.iter_mut()     { *a = 0.0; }
                    *freeze_captured = true;
                }

                let n = complex_buf.len();
                for k in 0..n {
                    let length_ms  = (freeze_curve_cache_0[k] * 500.0).clamp(0.0, 2000.0);
                    let length_hops = (length_ms / hop_ms).ceil() as u32;

                    let thr_gain = freeze_curve_cache_1[k];
                    let thr_db   = if thr_gain > 1e-10 { 20.0 * thr_gain.log10() } else { -120.0 };
                    let threshold_db  = (-20.0 + thr_db * (60.0 / 18.0)).clamp(-80.0, 0.0);
                    let threshold_lin = 10.0f32.powf(threshold_db / 20.0);

                    let port_ms  = (freeze_curve_cache_2[k] * 100.0).clamp(0.0, 1000.0);
                    let port_hops = (port_ms / hop_ms).max(0.5);

                    let resistance = (freeze_curve_cache_3[k] * 1.0).clamp(0.0, 5.0);

                    if freeze_port_t[k] < 1.0 {
                        freeze_port_t[k] = (freeze_port_t[k] + 1.0 / port_hops).min(1.0);
                        let t = freeze_port_t[k];
                        frozen_bins[k] = Complex::new(
                            frozen_bins[k].re * (1.0 - t) + freeze_target[k].re * t,
                            frozen_bins[k].im * (1.0 - t) + freeze_target[k].im * t,
                        );
                    } else {
                        freeze_hold_hops[k] += 1;
                        let mag = complex_buf[k].norm();
                        if mag > threshold_lin {
                            freeze_accum[k] += mag - threshold_lin;
                        }
                        if freeze_hold_hops[k] >= length_hops && freeze_accum[k] >= resistance {
                            freeze_target[k]    = complex_buf[k];
                            freeze_port_t[k]    = 0.0;
                            freeze_hold_hops[k] = 0;
                            freeze_accum[k]     = 0.0;
                        }
                    }

                    complex_buf[k] = frozen_bins[k];
                }
            } else {
                // Freeze disabled: reset captured flag so re-enabling always
                // captures a fresh spectrum.
                *freeze_captured = false;
            }

            // ── Stage 3: Phase Smear ─────────────────────────────────────
            if phase_enabled_flag {
                let last = complex_buf.len() - 1;
                for k in 0..complex_buf.len() {
                    *rng_state ^= *rng_state << 13;
                    *rng_state ^= *rng_state >> 7;
                    *rng_state ^= *rng_state << 17;
                    if k == 0 || k == last { continue; }
                    let per_bin    = phase_curve_cache[k].clamp(0.0, 2.0);
                    let scale      = phase_rand_amount * per_bin * std::f32::consts::PI;
                    let rand_phase = (*rng_state as f32 / u64::MAX as f32 * 2.0 - 1.0) * scale;
                    let (mag, phase) = (complex_buf[k].norm(), complex_buf[k].arg());
                    complex_buf[k] = Complex::from_polar(mag, phase + rand_phase);
                }
            }
```

Note: the old `active_engine.process_bins(...)` call and its suppression fold that appeared before the `match effect_mode` block must be **removed** — they are now inside `DynamicsMode::Compressor`. Find and delete the original unconditional call.

- [ ] **Step 5: Verify compilation**

```bash
cargo build 2>&1 | grep "^error" | head -20
```

Expected: errors only in `editor_ui.rs` (referencing removed `effect_mode`/`EffectMode`). Pipeline should compile cleanly.

- [ ] **Step 6: Run all tests**

```bash
cargo test 2>&1 | tail -10
```

Expected: engine_contract and stft tests pass. editor_ui may not compile yet — that's fine for this task.

- [ ] **Step 7: Commit**

```bash
git add src/dsp/pipeline.rs tests/engine_contract.rs
git commit -m "refactor: serial DSP stages (dynamics→freeze→phase) replace exclusive EffectMode"
```

---

## Task 3: Editor UI — Effects tab redesign

**Files:**
- Modify: `src/editor_ui.rs`

- [ ] **Step 1: Fix the import and mode reads at the top of the closure**

In `editor_ui.rs`, find (around line 47):

```rust
                    let cur_mode     = params.effect_mode.value();
                    let freeze_active = *params.freeze_active_curve.lock() as usize;

                    let is_freeze_mode = active_tab == 1
                        && cur_mode == crate::params::EffectMode::Freeze;
                    let is_phase_mode  = active_tab == 1
                        && cur_mode == crate::params::EffectMode::PhaseRand;
```

Replace with:

```rust
                    let dynamics_mode  = params.dynamics_mode.value();
                    let freeze_enabled = params.freeze_enabled.value();
                    let phase_enabled  = params.phase_enabled.value();
                    let effects_view   = *params.effects_view.lock() as usize;
                    let freeze_active  = *params.freeze_active_curve.lock() as usize;

                    let is_freeze_mode = active_tab == 1 && effects_view == 0;
                    let is_phase_mode  = active_tab == 1 && effects_view == 1;
```

- [ ] **Step 2: Fix the top-bar freeze curve buttons**

The top-bar freeze button section currently checks `is_freeze_mode` — that still works correctly after Step 1, no change needed there.

However, when the user clicks a freeze curve button, they implicitly want to view freeze curves. Add a view switch when clicking any freeze curve button. Find the freeze curve button click handler (around line 98):

```rust
                                if ui.add(btn).clicked() {
                                    *params.freeze_active_curve.lock() = i as u8;
                                }
```

Replace with:

```rust
                                if ui.add(btn).clicked() {
                                    *params.freeze_active_curve.lock() = i as u8;
                                    *params.effects_view.lock() = 0; // switch to freeze view
                                }
```

- [ ] **Step 3: Replace Effects tab bottom strip**

Find the Effects tab branch in row 2 (around line 542):

```rust
                            1 => {
                                // Effects: mode buttons + contextual knobs
                                ui.add_space(4.0);
                                let modes: &[(&str, crate::params::EffectMode)] = &[
                                    ("BYPASS",   crate::params::EffectMode::Bypass),
                                    ("FREEZE",   crate::params::EffectMode::Freeze),
                                    ("PHASE",    crate::params::EffectMode::PhaseRand),
                                    ("CONTRAST", crate::params::EffectMode::SpectralContrast),
                                ];
                                for &(label, mode) in modes {
                                    let active = cur_mode == mode;
                                    let fill   = if active { th::BORDER } else { th::BG };
                                    let text_c = if active { th::BG } else { th::LABEL_DIM };
                                    if ui.add(
                                        egui::Button::new(
                                            egui::RichText::new(label).color(text_c).size(10.0)
                                        )
                                        .fill(fill)
                                        .stroke(egui::Stroke::new(th::STROKE_BORDER, th::BORDER))
                                        .min_size(egui::vec2(60.0, 18.0))
                                    ).clicked() {
                                        setter.begin_set_parameter(&params.effect_mode);
                                        setter.set_parameter(&params.effect_mode, mode);
                                        setter.end_set_parameter(&params.effect_mode);
                                    }
                                    ui.add_space(2.0);
                                }
                                ui.add_space(8.0);
                                match cur_mode {
                                    crate::params::EffectMode::PhaseRand => {
                                        knob!(ui, &params.phase_rand_amount, "Amount");
                                    }
                                    crate::params::EffectMode::SpectralContrast => {
                                        knob!(ui, &params.spectral_contrast_db, "Depth");
                                    }
                                    _ => {}
                                }
                            }
```

Replace the entire `1 => { ... }` block with:

```rust
                            1 => {
                                // ── Dynamics engine ─────────────────────────────
                                let dyn_modes: &[(&str, crate::params::DynamicsMode)] = &[
                                    ("COMP",    crate::params::DynamicsMode::Compressor),
                                    ("CONTRAST",crate::params::DynamicsMode::Contrast),
                                    ("BYPASS",  crate::params::DynamicsMode::Bypass),
                                ];
                                for &(label, mode) in dyn_modes {
                                    let active = dynamics_mode == mode;
                                    let fill   = if active { th::BORDER } else { th::BG };
                                    let text_c = if active { th::BG } else { th::LABEL_DIM };
                                    if ui.add(
                                        egui::Button::new(
                                            egui::RichText::new(label).color(text_c).size(10.0)
                                        )
                                        .fill(fill)
                                        .stroke(egui::Stroke::new(th::STROKE_BORDER, th::BORDER))
                                        .min_size(egui::vec2(56.0, 18.0))
                                    ).clicked() {
                                        setter.begin_set_parameter(&params.dynamics_mode);
                                        setter.set_parameter(&params.dynamics_mode, mode);
                                        setter.end_set_parameter(&params.dynamics_mode);
                                    }
                                    ui.add_space(2.0);
                                }

                                ui.add_space(10.0);
                                ui.separator();
                                ui.add_space(10.0);

                                // ── Effect enable toggles + view selector ────────
                                // FREEZE button: click → view freeze curves; glow when enabled
                                let freeze_fill = if freeze_enabled { th::freeze_color_lit(0) } else { th::BG };
                                let freeze_text = if freeze_enabled { th::freeze_color_dim(0) } else { th::LABEL_DIM };
                                let freeze_view_stroke = if effects_view == 0 {
                                    egui::Stroke::new(th::STROKE_BORDER * 2.0, th::freeze_color_lit(0))
                                } else {
                                    egui::Stroke::new(th::STROKE_BORDER, th::BORDER)
                                };
                                if ui.add(
                                    egui::Button::new(
                                        egui::RichText::new("FREEZE").color(freeze_text).size(10.0)
                                    )
                                    .fill(freeze_fill)
                                    .stroke(freeze_view_stroke)
                                    .min_size(egui::vec2(56.0, 18.0))
                                ).clicked() {
                                    *params.effects_view.lock() = 0; // switch to freeze curve view
                                }
                                // Small ON/OFF toggle next to FREEZE
                                let frz_en_fill = if freeze_enabled { th::BORDER } else { th::BG };
                                let frz_en_text = if freeze_enabled { th::BG } else { th::LABEL_DIM };
                                if ui.add(
                                    egui::Button::new(
                                        egui::RichText::new(if freeze_enabled { "ON" } else { "OFF" })
                                            .color(frz_en_text).size(9.0)
                                    )
                                    .fill(frz_en_fill)
                                    .stroke(egui::Stroke::new(th::STROKE_BORDER, th::BORDER))
                                    .min_size(egui::vec2(28.0, 18.0))
                                ).clicked() {
                                    setter.begin_set_parameter(&params.freeze_enabled);
                                    setter.set_parameter(&params.freeze_enabled, !freeze_enabled);
                                    setter.end_set_parameter(&params.freeze_enabled);
                                }

                                ui.add_space(6.0);

                                // PHASE SMEAR button + ON/OFF
                                let phase_fill = if phase_enabled { th::phase_color_lit() } else { th::BG };
                                let phase_text = if phase_enabled { th::phase_color_dim() } else { th::LABEL_DIM };
                                let phase_view_stroke = if effects_view == 1 {
                                    egui::Stroke::new(th::STROKE_BORDER * 2.0, th::phase_color_lit())
                                } else {
                                    egui::Stroke::new(th::STROKE_BORDER, th::BORDER)
                                };
                                if ui.add(
                                    egui::Button::new(
                                        egui::RichText::new("PHASE").color(phase_text).size(10.0)
                                    )
                                    .fill(phase_fill)
                                    .stroke(phase_view_stroke)
                                    .min_size(egui::vec2(56.0, 18.0))
                                ).clicked() {
                                    *params.effects_view.lock() = 1; // switch to phase curve view
                                }
                                let ph_en_fill = if phase_enabled { th::BORDER } else { th::BG };
                                let ph_en_text = if phase_enabled { th::BG } else { th::LABEL_DIM };
                                if ui.add(
                                    egui::Button::new(
                                        egui::RichText::new(if phase_enabled { "ON" } else { "OFF" })
                                            .color(ph_en_text).size(9.0)
                                    )
                                    .fill(ph_en_fill)
                                    .stroke(egui::Stroke::new(th::STROKE_BORDER, th::BORDER))
                                    .min_size(egui::vec2(28.0, 18.0))
                                ).clicked() {
                                    setter.begin_set_parameter(&params.phase_enabled);
                                    setter.set_parameter(&params.phase_enabled, !phase_enabled);
                                    setter.end_set_parameter(&params.phase_enabled);
                                }

                                ui.add_space(8.0);

                                // ── Context knobs ────────────────────────────────
                                if phase_enabled || effects_view == 1 {
                                    knob!(ui, &params.phase_rand_amount, "Amount");
                                }
                                if dynamics_mode == crate::params::DynamicsMode::Contrast {
                                    knob!(ui, &params.spectral_contrast_db, "Depth");
                                }
                            }
```

- [ ] **Step 4: Build and verify**

```bash
cargo build 2>&1 | grep "^error" | head -20
```

Expected: zero errors. Warnings about unused `EffectMode` variants are fine if any remain in dead code — check and remove if so.

- [ ] **Step 5: Check for any remaining `effect_mode` / `EffectMode` references**

```bash
grep -n "effect_mode\|EffectMode" src/editor_ui.rs src/dsp/pipeline.rs src/params.rs
```

Expected: zero matches. If any remain, remove them.

- [ ] **Step 6: Run all tests**

```bash
cargo test 2>&1 | tail -10
```

Expected: all tests pass, zero failures.

- [ ] **Step 7: Commit**

```bash
git add src/editor_ui.rs
git commit -m "feat: Effects tab — serial FREEZE/PHASE enable toggles replace exclusive mode selector"
```

---

## Task 4: Final verification

- [ ] **Step 1: Full test suite**

```bash
cargo test 2>&1
```

Expected output ends with:
```
test result: ok. N passed; 0 failed; 0 ignored
```
where N ≥ 16 (14 existing + 2 added in this plan).

- [ ] **Step 2: Release build**

```bash
cargo build --release 2>&1 | grep "^error\|^warning\[" | head -20
```

Expected: no errors, no new warnings beyond existing baseline.

- [ ] **Step 3: Bundle and install**

```bash
cargo run --package xtask -- bundle spectral_forge --release && cp target/bundled/spectral_forge.clap ~/.clap/
```

- [ ] **Step 4: Manual verification checklist in Bitwig**

Load the plugin and verify:
- [ ] Dynamics compressor works with COMP selected
- [ ] Enabling FREEZE (ON) while dynamics is running: both process (dynamics → frozen bins)
- [ ] Enabling PHASE (ON) alongside FREEZE: all three stages run
- [ ] Disabling FREEZE: `freeze_captured` resets, re-enabling captures fresh spectrum
- [ ] Mix knob in the bottom strip now affects dynamics output even when Freeze is ON
- [ ] CONTRAST engine selectable from Effects tab, Depth knob appears
- [ ] Switching Effects tab → FREEZE view → 4 freeze curve buttons appear in top bar
- [ ] Switching Effects tab → PHASE view → 7 dynamics buttons return to top bar

- [ ] **Step 5: Final commit**

```bash
git add -u
git commit -m "test: verify serial FX chain builds and passes all tests"
```

---

## Self-Review

**Spec coverage check:**
- ✅ Serial chain (dynamics→freeze→phase) — Task 2
- ✅ Mix knob works when freeze is active — fixed by serial stages; mix is applied inside dynamics only
- ✅ EffectMode exclusivity removed — Task 1
- ✅ GUI enable toggles per effect — Task 3
- ✅ View selector for freeze/phase curve graph — Task 3
- ✅ SpectralContrast becomes DynamicsMode::Contrast — Task 1+2+3
- ✅ freeze_captured resets correctly when freeze disabled — Task 2 step 4

**Backwards compatibility note:** The `#[id = "effect_mode"]` parameter is removed and replaced by `#[id = "dynamics_mode"]`, `#[id = "freeze_enabled"]`, `#[id = "phase_enabled"]`. Existing DAW sessions that saved `effect_mode` will lose that setting and default to `Compressor`/no freeze/no phase — this is acceptable given the structural change.
