> **Status (2026-04-24): SUPERSEDED.** Effects (Freeze, PhaseRand, Contrast) landed as a fixed post-compressor stage per this plan, then were re-homed as independent `SpectralModule` implementations inside the modular FxMatrix. Do not follow this plan as written. Source of truth: the code + [../STATUS.md](../STATUS.md).

# Effects Engines Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement three spectral effects for the Effects tab — Freeze, Phase Randomize, and Spectral Contrast — as a post-compressor pass inside the existing STFT closure.

**Architecture:** A new `EffectMode` enum selects one of four behaviours (Bypass / Freeze / PhaseRand / SpectralContrast). The effects pass runs on `complex_buf` after `process_bins` and before `ifft_plan.process`, modifying the spectrum in-place. All state (frozen magnitudes, PRNG, contrast envelope) lives in pre-allocated `Pipeline` fields — no audio-thread allocation.

**Tech Stack:** Rust, nih-plug, egui (nih_plug_egui), `num_complex::Complex<f32>`, xorshift64 PRNG (inline, no crate).

---

## File Map

| File | Change |
|------|--------|
| `src/params.rs` | Add `EffectMode` enum; add `effect_mode`, `phase_rand_amount`, `spectral_contrast_db` to `SpectralForgeParams` |
| `src/dsp/pipeline.rs` | Add 4 struct fields; initialise in `new()` + `reset()`; read new params at top of `process()`; rebind 4 new locals before `process_overlap_add`; insert effects pass inside STFT closure |
| `src/editor_ui.rs` | Replace Effects tab placeholder with mode buttons + contextual knob |

---

## Task 1 — Add `EffectMode` params

**Files:**
- Modify: `src/params.rs`

- [ ] **Step 1: Add the `EffectMode` enum above `SpectralForgeParams`**

Open `src/params.rs`. After the `StereoLink` enum (currently around line 25) and before the `#[derive(Params)]` block, insert:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub enum EffectMode {
    Bypass,
    Freeze,
    PhaseRand,
    SpectralContrast,
}
```

- [ ] **Step 2: Add three fields to `SpectralForgeParams`**

After `delta_monitor: BoolParam,` (currently the last field), add:

```rust
    #[id = "effect_mode"]
    pub effect_mode: EnumParam<EffectMode>,

    #[id = "phase_rand_amount"]
    pub phase_rand_amount: FloatParam,

    #[id = "spectral_contrast_db"]
    pub spectral_contrast_db: FloatParam,
```

- [ ] **Step 3: Add defaults in `impl Default for SpectralForgeParams`**

After `delta_monitor: BoolParam::new("Delta Monitor", false),` add:

```rust
            effect_mode: EnumParam::new("Effect Mode", EffectMode::Bypass),

            phase_rand_amount: FloatParam::new(
                "Phase Rand Amount", 0.5,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            ).with_smoother(SmoothingStyle::Linear(50.0)),

            spectral_contrast_db: FloatParam::new(
                "Spectral Contrast", 6.0,
                FloatRange::Linear { min: 0.0, max: 12.0 },
            ).with_smoother(SmoothingStyle::Linear(50.0))
             .with_unit(" dB"),
```

- [ ] **Step 4: Verify it compiles**

```bash
cargo build 2>&1 | grep -E "^error"
```

Expected: no output (zero errors).

- [ ] **Step 5: Commit**

```bash
git add src/params.rs
git commit -m "feat: add EffectMode enum and effect params (mode, phase_rand_amount, spectral_contrast_db)"
```

---

## Task 2 — Pipeline effects pass

**Files:**
- Modify: `src/dsp/pipeline.rs`

### 2a — Struct fields

- [ ] **Step 1: Add four new fields to the `Pipeline` struct**

After the `dry_delay_write: usize,` field (currently around line 46) and before the `curve_cache` field, insert:

```rust
    /// Captured per-bin magnitudes for Spectral Freeze.
    frozen_mags: Vec<f32>,
    /// True once Freeze has captured its first frame; reset to false when mode changes.
    freeze_captured: bool,
    /// xorshift64 PRNG state for Phase Randomize. Initialised to a non-zero seed.
    rng_state: u64,
    /// Per-bin smoothed magnitude envelope for Spectral Contrast (~200 ms window).
    contrast_envelope: Vec<f32>,
```

- [ ] **Step 2: Initialise the new fields in `Pipeline::new()`**

In the `Self { ... }` constructor block (ends around line 98), after `dry_delay_write: 0,` add:

```rust
            frozen_mags:        vec![0.0f32; NUM_BINS],
            freeze_captured:    false,
            rng_state:          0xdeadbeef_cafebabe_u64,
            contrast_envelope:  vec![0.0f32; NUM_BINS],
```

- [ ] **Step 3: Reset the new fields in `Pipeline::reset()`**

In `reset()` (around line 101), after `self.dry_delay_write = 0;` add:

```rust
        self.frozen_mags.fill(0.0);
        self.freeze_captured = false;
        // rng_state intentionally not reset — PRNG continuity across SR changes is harmless
        self.contrast_envelope.fill(0.0);
```

- [ ] **Step 4: Verify it still compiles**

```bash
cargo build 2>&1 | grep -E "^error"
```

Expected: no output.

### 2b — Read effect params and rebind locals

- [ ] **Step 5: Read the three new params at the top of `process()`**

After the existing smoother reads (around line 132, after `threshold_slope_db`), add:

```rust
        let effect_mode          = params.effect_mode.value();
        let phase_rand_amount    = params.phase_rand_amount.smoothed.next_step(block_size);
        let spectral_contrast_db = params.spectral_contrast_db.smoothed.next_step(block_size);
```

- [ ] **Step 6: Rebind the four new fields as locals before `process_overlap_add`**

After the existing rebind block (ending with `let norm = 2.0_f32 / (3.0 * FFT_SIZE as f32);`, around line 321) and before `self.stft.process_overlap_add(...)`, add:

```rust
        let frozen_mags       = &mut self.frozen_mags;
        let freeze_captured   = &mut self.freeze_captured;
        let rng_state         = &mut self.rng_state;
        let contrast_envelope = &mut self.contrast_envelope;
```

### 2c — Effects pass inside the STFT closure

- [ ] **Step 7: Insert the effects pass after the suppression fold, before `ifft_plan.process`**

The target location inside `self.stft.process_overlap_add(buffer, OVERLAP, |channel, block| { ... })` is after the suppression fold block:

```rust
            for k in 0..channel_supp_buf.len() {
                if channel_supp_buf[k] > suppression_buf[k] { suppression_buf[k] = channel_supp_buf[k]; }
            }
```

and before:

```rust
            ifft_plan.process(complex_buf, block).unwrap();
```

Between those two, insert:

```rust
            // Effects pass — modifies complex_buf in-place after compression.
            match effect_mode {
                crate::params::EffectMode::Bypass => {}

                crate::params::EffectMode::Freeze => {
                    if !*freeze_captured {
                        // Capture magnitudes from the first frame after Freeze is engaged.
                        for k in 0..complex_buf.len() {
                            frozen_mags[k] = complex_buf[k].norm();
                        }
                        *freeze_captured = true;
                    }
                    // Substitute frozen magnitudes while preserving current phases.
                    for k in 0..complex_buf.len() {
                        let phase = complex_buf[k].arg();
                        complex_buf[k] = Complex::from_polar(frozen_mags[k], phase);
                    }
                }

                crate::params::EffectMode::PhaseRand => {
                    // xorshift64: three shifts, no allocation, full 64-bit period.
                    let scale = phase_rand_amount * std::f32::consts::PI;
                    for k in 0..complex_buf.len() {
                        *rng_state ^= *rng_state << 13;
                        *rng_state ^= *rng_state >> 7;
                        *rng_state ^= *rng_state << 17;
                        // Map u64 to [-1, 1] then scale to [-π*amount, π*amount]
                        let rand_phase = (*rng_state as f32 / u64::MAX as f32 * 2.0 - 1.0) * scale;
                        let (mag, phase) = (complex_buf[k].norm(), complex_buf[k].arg());
                        complex_buf[k] = Complex::from_polar(mag, phase + rand_phase);
                    }
                }

                crate::params::EffectMode::SpectralContrast => {
                    // One-pole envelope follower at ~200 ms.
                    let hop_sz = FFT_SIZE / OVERLAP;
                    let time_hops = 0.2_f32 * sample_rate / hop_sz as f32;
                    let coeff = (-1.0_f32 / time_hops).exp();
                    let boost = 10.0f32.powf( spectral_contrast_db / 20.0);
                    let cut   = 10.0f32.powf(-spectral_contrast_db / 20.0);
                    for k in 0..complex_buf.len() {
                        let mag = complex_buf[k].norm();
                        contrast_envelope[k] = coeff * contrast_envelope[k] + (1.0 - coeff) * mag;
                        let env = contrast_envelope[k].max(1e-10);
                        // Bins above the local envelope get boosted; below get cut.
                        let gain = if mag >= env { boost } else { cut };
                        complex_buf[k] *= gain;
                    }
                }
            }

            // When mode leaves Freeze, clear the captured flag so re-engaging
            // Freeze always captures a fresh spectrum.
            if effect_mode != crate::params::EffectMode::Freeze {
                *freeze_captured = false;
            }
```

- [ ] **Step 8: Verify the full build**

```bash
cargo build 2>&1 | grep -E "^error"
```

Expected: no output.

- [ ] **Step 9: Run the test suite**

```bash
cargo test 2>&1 | tail -8
```

Expected: `test result: ok. 14 passed; 0 failed`.

- [ ] **Step 10: Commit**

```bash
git add src/dsp/pipeline.rs
git commit -m "feat: spectral effects pass — Freeze, PhaseRand, SpectralContrast in STFT closure"
```

---

## Task 3 — Effects tab UI

**Files:**
- Modify: `src/editor_ui.rs`

- [ ] **Step 1: Replace the Effects tab placeholder**

Find and replace the entire `} else if active_tab == 1 {` block (currently lines 405-415):

```rust
                    } else if active_tab == 1 {
                        // Effects tab — placeholder
                        let avail = ui.available_rect_before_wrap();
                        ui.allocate_rect(avail, egui::Sense::hover());
                        ui.painter().text(
                            avail.center(),
                            egui::Align2::CENTER_CENTER,
                            "Effects — coming soon",
                            egui::FontId::proportional(14.0),
                            th::LABEL_DIM,
                        );
```

with:

```rust
                    } else if active_tab == 1 {
                        // Effects tab
                        use nih_plug_egui::widgets::ParamSlider;
                        let cur_mode = params.effect_mode.value();

                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            ui.add_space(8.0);
                            // Mode selector buttons
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
                                    .stroke(egui::Stroke::new(1.0, th::BORDER))
                                    .min_size(egui::vec2(64.0, 18.0))
                                ).clicked() {
                                    setter.begin_set_parameter(&params.effect_mode);
                                    setter.set_parameter(&params.effect_mode, mode);
                                    setter.end_set_parameter(&params.effect_mode);
                                }
                                ui.add_space(4.0);
                            }
                        });

                        // Contextual controls — only shown when the selected mode has a parameter
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            ui.add_space(8.0);
                            match cur_mode {
                                crate::params::EffectMode::Bypass
                                | crate::params::EffectMode::Freeze => {
                                    ui.label(
                                        egui::RichText::new("No controls for this mode.")
                                            .color(th::LABEL_DIM).size(10.0)
                                    );
                                }
                                crate::params::EffectMode::PhaseRand => {
                                    ui.vertical(|ui| {
                                        ui.add(ParamSlider::for_param(
                                            &params.phase_rand_amount, setter).with_width(80.0));
                                        ui.label(egui::RichText::new("Amount")
                                            .color(th::LABEL_DIM).size(9.0));
                                    });
                                }
                                crate::params::EffectMode::SpectralContrast => {
                                    ui.vertical(|ui| {
                                        ui.add(ParamSlider::for_param(
                                            &params.spectral_contrast_db, setter).with_width(80.0));
                                        ui.label(egui::RichText::new("Depth")
                                            .color(th::LABEL_DIM).size(9.0));
                                    });
                                }
                            }
                        });
```

- [ ] **Step 2: Verify full build**

```bash
cargo build 2>&1 | grep -E "^error"
```

Expected: no output.

- [ ] **Step 3: Run the test suite**

```bash
cargo test 2>&1 | tail -8
```

Expected: `test result: ok. 14 passed; 0 failed`.

- [ ] **Step 4: Commit**

```bash
git add src/editor_ui.rs
git commit -m "feat: Effects tab — mode buttons (Bypass/Freeze/Phase/Contrast) + contextual knobs"
```

---

## Task 4 — Bundle and smoke-test

- [ ] **Step 1: Build release bundle**

```bash
cargo run --package xtask -- bundle spectral_forge --release 2>&1 | tail -5
```

Expected: `Bundled spectral_forge` (no errors).

- [ ] **Step 2: Install**

```bash
cp target/bundled/spectral_forge.clap ~/.clap/
```

- [ ] **Step 3: Manual smoke-test checklist**

In Bitwig, rescan plugins, insert Spectral Forge on a track with audio:

1. Switch to Effects tab — four buttons visible (BYPASS / FREEZE / PHASE / CONTRAST)
2. **BYPASS**: audio passes unchanged (default)
3. **FREEZE**: click while audio plays — spectrum freezes (pitch/timbre held)
4. Switch back to BYPASS — audio returns to normal; clicking FREEZE again captures a new frame
5. **PHASE**: Amount knob appears; at Amount=0 sounds nearly unchanged; at Amount=1 clearly smeared/diffuse
6. **CONTRAST**: Depth knob appears; tonal material gets sharper/boostier, noise gets cut; at Depth=0 nearly transparent
7. **Dynamics tab** still works — curve editor, compression audible
8. All 14 `cargo test` pass (verified in Task 3)

- [ ] **Step 4: Final commit (if any last-minute fixes)**

```bash
git add -p   # stage only the fixes
git commit -m "fix: <description of any smoke-test fix>"
```

If no fixes needed, skip this step.
