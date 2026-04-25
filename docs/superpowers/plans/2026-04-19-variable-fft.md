> **Status (2026-04-24): IMPLEMENTED.** Runtime-selectable FFT size (512 … 16384) via `FftSizeChoice`. Source of truth: the code + [../STATUS.md](../STATUS.md).

# Variable FFT Resolution Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the FFT window size user-selectable at runtime (512 / 1024 / 2048 / 4096 / 8192 / 16384 samples), pre-allocating all Pipeline buffers at the maximum size so no allocation ever occurs on the audio thread when changing resolution.

**Architecture:** Add `pub const MAX_FFT_SIZE: usize = 16384` and `pub const MAX_NUM_BINS: usize = MAX_FFT_SIZE / 2 + 1 = 8193`. All `Vec<f32>` and `Vec<Complex<f32>>` buffers in `Pipeline` are allocated at `MAX_NUM_BINS` at construction. `Pipeline` stores `fft_size: usize` (the currently active size) and uses `fft_size / 2 + 1` as the bin count in all processing loops. When the user changes the FFT size selector, `Plugin::initialize()` is called by the host, which rebuilds the pipeline with the new size (re-planning FFT, recreating `StftHelper`). The `bridge::SharedState` is already runtime-sized (`num_bins` parameter) so it adapts automatically.

**Tech Stack:** Rust, nih-plug, realfft, parking_lot, triple_buffer

---

## Files Modified

| File | Changes |
|------|---------|
| `src/dsp/pipeline.rs` | Add `MAX_FFT_SIZE`, `MAX_NUM_BINS`; store `fft_size: usize` in `Pipeline`; allocate all Vecs at `MAX_NUM_BINS`; replace `FFT_SIZE` references in loops with `self.fft_size / 2 + 1` |
| `src/params.rs` | Add `FftSizeChoice` enum and `fft_size: EnumParam<FftSizeChoice>` |
| `src/lib.rs` | Read `params.fft_size.value()` in `initialize()`, pass to `Pipeline::new()`, update latency |
| `tests/engine_contract.rs` | Add test verifying pipeline at 512 and 4096 both produce finite output |

---

## Context for implementers

`Pipeline::new(sample_rate, num_channels)` currently uses `FFT_SIZE = 2048` as a compile-time constant everywhere. After this plan, `Pipeline::new(sample_rate, num_channels, fft_size: usize)` takes the active size as a parameter.

The `StftHelper` from nih-plug is constructed with a fixed FFT size and must be recreated when that size changes. Reconstruction is safe on `initialize()` (non-real-time context). It is **not** safe to resize on the audio thread.

`realfft::RealFftPlanner` is deterministic — calling `.plan_fft_forward(N)` twice returns the same plan (they're cached internally). Regenerating plans on `initialize()` is fine.

The key invariant: **all Vecs are sized at `MAX_NUM_BINS` from construction**. Processing loops iterate only up to `num_bins = self.fft_size / 2 + 1`. Unused entries at the end of each Vec are never written to or read from during audio processing.

The `FFT_SIZE` constant in `pipeline.rs` is also referenced in `editor_ui.rs` for display computations (spectrum display, curve response). After this plan, `editor_ui.rs` should read the current FFT size from the bridge's sample_rate-analogous field, or we add a `num_bins` atomic that the GUI reads. The simplest approach: expose `shared.fft_size: Arc<AtomicUsize>` in bridge and pass it to the editor.

---

## Task 1: FFT size param and constants

**Files:**
- Modify: `src/params.rs`
- Modify: `src/dsp/pipeline.rs` (constants only, no struct changes yet)

- [ ] **Step 1: Write the failing test**

Add to `tests/engine_contract.rs`:

```rust
#[test]
fn fft_size_choice_has_correct_variants() {
    // Compile-time verification that all 6 size variants exist.
    use spectral_forge::params::FftSizeChoice;
    let sizes = [
        FftSizeChoice::S512,
        FftSizeChoice::S1024,
        FftSizeChoice::S2048,
        FftSizeChoice::S4096,
        FftSizeChoice::S8192,
        FftSizeChoice::S16384,
    ];
    let sample_sizes: [usize; 6] = [512, 1024, 2048, 4096, 8192, 16384];
    for (choice, &expected) in sizes.iter().zip(sample_sizes.iter()) {
        assert_eq!(spectral_forge::params::fft_size_from_choice(*choice), expected);
    }
}
```

- [ ] **Step 2: Run to verify it fails**

```bash
cargo test fft_size_choice_has_correct_variants 2>&1 | tail -5
```

Expected: compile error — `FftSizeChoice` not found.

- [ ] **Step 3: Add FftSizeChoice to params.rs**

Add after the `EffectMode` enum (or `DynamicsMode` if Plan A is done):

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub enum FftSizeChoice {
    S512,
    S1024,
    S2048,
    S4096,
    S8192,
    S16384,
}

/// Convert enum variant to actual sample count.
pub fn fft_size_from_choice(c: FftSizeChoice) -> usize {
    match c {
        FftSizeChoice::S512   =>   512,
        FftSizeChoice::S1024  =>  1024,
        FftSizeChoice::S2048  =>  2048,
        FftSizeChoice::S4096  =>  4096,
        FftSizeChoice::S8192  =>  8192,
        FftSizeChoice::S16384 => 16384,
    }
}
```

Add `fft_size` field to `SpectralForgeParams` struct (after `delta_monitor`):

```rust
    #[id = "fft_size"]
    pub fft_size: EnumParam<FftSizeChoice>,
```

Add to `Default for SpectralForgeParams`:

```rust
            fft_size: EnumParam::new("FFT Size", FftSizeChoice::S2048)
                .with_callback(Arc::new(|_| {})), // host will call initialize() on change
```

- [ ] **Step 4: Add MAX constants to pipeline.rs**

At the top of `src/dsp/pipeline.rs`, after the existing constants:

```rust
pub const MAX_FFT_SIZE: usize = 16384;
pub const MAX_NUM_BINS: usize = MAX_FFT_SIZE / 2 + 1;  // 8193
```

Keep `pub const FFT_SIZE: usize = 2048` and `pub const NUM_BINS: usize = FFT_SIZE / 2 + 1` for now — they will be used as the default in tests and in code not yet migrated. They will be removed in Task 2.

- [ ] **Step 5: Run test**

```bash
cargo test fft_size_choice_has_correct_variants 2>&1 | tail -5
```

Expected: `test fft_size_choice_has_correct_variants ... ok`

- [ ] **Step 6: Commit**

```bash
git add src/params.rs src/dsp/pipeline.rs tests/engine_contract.rs
git commit -m "feat: add FftSizeChoice enum and MAX_FFT_SIZE/MAX_NUM_BINS constants"
```

---

## Task 2: Pipeline — runtime fft_size + pre-allocation at MAX_NUM_BINS

**Files:**
- Modify: `src/dsp/pipeline.rs`

- [ ] **Step 1: Write the failing test**

Add to `tests/engine_contract.rs`:

```rust
#[test]
fn pipeline_at_512_produces_finite_output() {
    use spectral_forge::dsp::pipeline::process_block_for_test_with_size;
    let input = vec![0.5f32; 1024];
    let output = process_block_for_test_with_size(&input, 44100.0, 512);
    assert!(!output.is_empty());
    for &s in &output {
        assert!(s.is_finite(), "sample must be finite, got {s}");
    }
}

#[test]
fn pipeline_at_4096_produces_finite_output() {
    use spectral_forge::dsp::pipeline::process_block_for_test_with_size;
    let input = vec![0.5f32; 8192];
    let output = process_block_for_test_with_size(&input, 44100.0, 4096);
    assert!(!output.is_empty());
    for &s in &output {
        assert!(s.is_finite(), "sample must be finite, got {s}");
    }
}
```

- [ ] **Step 2: Run to verify they fail**

```bash
cargo test pipeline_at_512 pipeline_at_4096 2>&1 | tail -5
```

Expected: compile error — `process_block_for_test_with_size` not found.

- [ ] **Step 3: Add fft_size field to Pipeline struct**

In `src/dsp/pipeline.rs`, add `fft_size: usize` as the last field in the `Pipeline` struct (after `sample_rate: f32`):

```rust
    fft_size: usize,
```

- [ ] **Step 4: Change Pipeline::new() signature and pre-allocate at MAX_NUM_BINS**

Change the `pub fn new` signature:

```rust
    pub fn new(sample_rate: f32, num_channels: usize, fft_size: usize) -> Self {
```

Replace all `let complex_buf = fft_plan.make_output_vec()` and `fft_plan.make_output_vec()` with explicit allocation at `MAX_NUM_BINS`:

```rust
        // Plan FFT at the requested size (realfft caches plans internally)
        let fft_plan  = planner.plan_fft_forward(fft_size);
        let ifft_plan = planner.plan_fft_inverse(fft_size);

        let num_bins  = fft_size / 2 + 1;

        // Window function at the active fft_size (not MAX — only active size used in processing)
        let window: Vec<f32> = (0..fft_size)
            .map(|i| 0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32
                / (fft_size - 1) as f32).cos()))
            .collect();

        // FFT complex output buffers: sized to active num_bins (realfft requirement)
        let complex_buf    = fft_plan.make_output_vec();
        let sc_complex_buf = fft_plan.make_output_vec();
```

Then allocate all per-bin Vecs at `MAX_NUM_BINS`:

```rust
        Self {
            stft:      StftHelper::new(num_channels, fft_size, 0),
            sc_stft:   StftHelper::new(2, fft_size, 0),
            fft_plan,
            ifft_plan,
            window,
            sc_envelope:       vec![0.0f32; MAX_NUM_BINS],
            sc_env_state:      vec![0.0f32; MAX_NUM_BINS],
            sc_complex_buf,
            spectrum_buf:      vec![0.0f32; MAX_NUM_BINS],
            suppression_buf:   vec![0.0f32; MAX_NUM_BINS],
            channel_supp_buf:  vec![0.0f32; MAX_NUM_BINS],
            complex_buf,
            engine,
            engine_r,
            bp_threshold: vec![-20.0f32; MAX_NUM_BINS],
            bp_ratio:     vec![4.0f32;   MAX_NUM_BINS],
            bp_attack:    vec![10.0f32;  MAX_NUM_BINS],
            bp_release:   vec![80.0f32;  MAX_NUM_BINS],
            bp_knee:      vec![6.0f32;   MAX_NUM_BINS],
            bp_makeup:    vec![0.0f32;   MAX_NUM_BINS],
            bp_mix:       vec![1.0f32;   MAX_NUM_BINS],
            dry_delay:         vec![0.0f32; 2 * DRY_DELAY_SIZE],
            dry_delay_write:   0,
            frozen_bins:       vec![Complex::new(0.0f32, 0.0f32); MAX_NUM_BINS],
            freeze_target:     vec![Complex::new(0.0f32, 0.0f32); MAX_NUM_BINS],
            freeze_port_t:     vec![1.0f32; MAX_NUM_BINS],
            freeze_hold_hops:  vec![0u32; MAX_NUM_BINS],
            freeze_accum:      vec![0.0f32; MAX_NUM_BINS],
            freeze_captured:   false,
            rng_state:         0xdeadbeef_cafebabe_u64,
            contrast_engine,
            curve_cache:       std::array::from_fn(|_| vec![1.0f32; MAX_NUM_BINS]),
            phase_curve_cache: vec![1.0f32; MAX_NUM_BINS],
            freeze_curve_cache: std::array::from_fn(|_| vec![1.0f32; MAX_NUM_BINS]),
            sample_rate,
            fft_size,
        }
    }
```

- [ ] **Step 5: Update Pipeline::reset() to take fft_size**

```rust
    pub fn reset(&mut self, sample_rate: f32, num_channels: usize, fft_size: usize) {
        self.sample_rate = sample_rate;
        self.fft_size    = fft_size;

        // Regenerate FFT plans and StftHelper at new size
        let mut planner   = realfft::RealFftPlanner::<f32>::new();
        self.fft_plan     = planner.plan_fft_forward(fft_size);
        self.ifft_plan    = planner.plan_fft_inverse(fft_size);
        self.stft         = StftHelper::new(num_channels, fft_size, 0);
        self.sc_stft      = StftHelper::new(2, fft_size, 0);

        // Resize complex_buf to new num_bins (realfft requires exact size)
        self.complex_buf    = self.fft_plan.make_output_vec();
        self.sc_complex_buf = self.fft_plan.make_output_vec();

        // Regenerate window
        self.window = (0..fft_size)
            .map(|i| 0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32
                / (fft_size - 1) as f32).cos()))
            .collect();

        // Reset accumulators (do NOT resize — they're at MAX_NUM_BINS)
        let num_bins = fft_size / 2 + 1;
        for v in self.sc_envelope[..num_bins].iter_mut()   { *v = 0.0; }
        for v in self.sc_env_state[..num_bins].iter_mut()  { *v = 0.0; }
        self.dry_delay.fill(0.0);
        self.dry_delay_write = 0;
        for b in self.frozen_bins[..num_bins].iter_mut()    { *b = Complex::new(0.0, 0.0); }
        for b in self.freeze_target[..num_bins].iter_mut()  { *b = Complex::new(0.0, 0.0); }
        for t in self.freeze_port_t[..num_bins].iter_mut()  { *t = 1.0; }
        for h in self.freeze_hold_hops[..num_bins].iter_mut() { *h = 0; }
        for a in self.freeze_accum[..num_bins].iter_mut()   { *a = 0.0; }
        self.freeze_captured = false;
        self.engine.reset(sample_rate, fft_size);
        self.engine_r.reset(sample_rate, fft_size);
        self.contrast_engine.reset(sample_rate, fft_size);
    }
```

- [ ] **Step 6: Update all processing loops to use runtime num_bins**

In `Pipeline::process()`, replace all occurrences of:

```rust
let num_bins = self.bp_threshold.len();
```

with:

```rust
let num_bins = self.fft_size / 2 + 1;
```

Replace all occurrences of `FFT_SIZE` used as the active size in processing (inside `process()`) with `self.fft_size`. Key locations:

1. `let hop = FFT_SIZE / OVERLAP;` → `let hop = self.fft_size / OVERLAP;`
2. `let f_k_hz = (k as f32 * sample_rate / FFT_SIZE as f32)` → `self.fft_size`
3. `let norm = 2.0_f32 / (3.0 * FFT_SIZE as f32);` → `self.fft_size`
4. Freeze: `let hop_ms = FFT_SIZE as f32 / (OVERLAP as f32 * sample_rate)` → `self.fft_size`
5. `context.set_latency_samples(dsp::pipeline::FFT_SIZE as u32)` in `lib.rs` → `self.fft_size`

Also update the triple-buffer copy loop — curves are read into `self.curve_cache` slices at `num_bins`:

```rust
        // Read curve caches — only copy num_bins entries (rest stay at default 1.0)
        let num_bins = self.fft_size / 2 + 1;
        self.curve_cache[0][..num_bins].copy_from_slice(shared.curve_rx[0].read());
        // ... etc for all 7 + phase + 4 freeze
```

Note: `shared.curve_rx[i].read()` returns a slice of `shared.num_bins` length. The bridge's `num_bins` must match `self.fft_size / 2 + 1`. This is ensured in Task 3.

- [ ] **Step 7: Add process_block_for_test_with_size**

At the bottom of `pipeline.rs`, after the existing `process_block_for_test`, add:

```rust
#[doc(hidden)]
pub fn process_block_for_test_with_size(input: &[f32], sample_rate: f32, fft_size: usize) -> Vec<f32> {
    let mut planner = realfft::RealFftPlanner::<f32>::new();
    let fft  = planner.plan_fft_forward(fft_size);
    let ifft = planner.plan_fft_inverse(fft_size);
    let hop  = fft_size / OVERLAP;
    let norm = 2.0_f32 / (3.0 * fft_size as f32);

    let window: Vec<f32> = (0..fft_size)
        .map(|i| 0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32
            / (fft_size - 1) as f32).cos()))
        .collect();

    let num_bins = fft_size / 2 + 1;
    let mut complex_buf = fft.make_output_vec();
    let mut out = vec![0.0f32; input.len()];
    let mut overlap_buf = vec![0.0f32; fft_size];
    let mut in_buf  = vec![0.0f32; fft_size];

    let mut pos = 0usize;
    while pos + fft_size <= input.len() {
        in_buf.copy_from_slice(&input[pos..pos + fft_size]);
        for (s, &w) in in_buf.iter_mut().zip(window.iter()) { *s *= w; }
        crate::dsp::guard::sanitize(&mut in_buf);
        fft.process(&mut in_buf, &mut complex_buf).unwrap();
        ifft.process(&mut complex_buf, &mut in_buf).unwrap();
        for (i, &s) in in_buf.iter().enumerate() {
            let out_i = pos + i;
            if out_i < out.len() {
                out[out_i] += s * window[i] * norm;
            }
        }
        pos += hop;
    }
    out
}
```

- [ ] **Step 8: Run tests**

```bash
cargo test pipeline_at_512 pipeline_at_4096 2>&1 | tail -5
```

Expected: both pass.

- [ ] **Step 9: Run full test suite**

```bash
cargo test 2>&1 | tail -8
```

Expected: all tests pass.

- [ ] **Step 10: Commit**

```bash
git add src/dsp/pipeline.rs tests/engine_contract.rs
git commit -m "feat: Pipeline stores runtime fft_size, pre-allocates at MAX_NUM_BINS=8193"
```

---

## Task 3: Lib.rs and bridge — wire fft_size through initialize()

**Files:**
- Modify: `src/lib.rs`
- Modify: `src/bridge.rs`

- [ ] **Step 1: Add fft_size to SharedState**

In `src/bridge.rs`, add to `SharedState`:

```rust
    pub fft_size: Arc<crate::AtomicUsize>,
```

And add `use std::sync::atomic::AtomicUsize;` and define:

```rust
/// Wait-free usize atomic.
pub struct AtomicUsize(std::sync::atomic::AtomicUsize);
impl AtomicUsize {
    pub fn new(v: usize) -> Self { Self(std::sync::atomic::AtomicUsize::new(v)) }
    pub fn load(&self) -> usize { self.0.load(std::sync::atomic::Ordering::Relaxed) }
    pub fn store(&self, v: usize) { self.0.store(v, std::sync::atomic::Ordering::Relaxed) }
}
```

In `SharedState::new()`:

```rust
    pub fn new(num_bins: usize, sample_rate: f32) -> Self {
        // ... existing init ...
        Self {
            // ... existing fields ...
            fft_size: Arc::new(AtomicUsize::new(num_bins.saturating_sub(1) * 2)), // approximate
        }
    }
```

Actually, pass fft_size explicitly:

```rust
    pub fn new(num_bins: usize, sample_rate: f32, fft_size: usize) -> Self {
        // ...
        Self {
            // ...
            fft_size: Arc::new(AtomicUsize::new(fft_size)),
        }
    }
```

- [ ] **Step 2: Update lib.rs initialize()**

In `src/lib.rs`, `initialize()`:

```rust
    fn initialize(
        &mut self,
        audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        context: &mut impl InitContext<Self>,
    ) -> bool {
        let sr = buffer_config.sample_rate;
        let num_ch = audio_io_layout.main_output_channels
            .map(|c| c.get() as usize).unwrap_or(2);
        self.num_channels = num_ch;
        self.sample_rate  = sr;

        // Read user-selected FFT size
        let fft_size = crate::params::fft_size_from_choice(self.params.fft_size.value());
        let num_bins = fft_size / 2 + 1;

        // Rebuild pipeline and bridge with new size
        let shared = bridge::SharedState::new(num_bins, sr, fft_size);

        self.gui_curve_tx         = shared.curve_tx.clone();
        self.gui_phase_curve_tx   = shared.phase_curve_tx.clone();
        self.gui_freeze_curve_tx  = shared.freeze_curve_tx.clone();
        self.gui_sample_rate      = Some(shared.sample_rate.clone());
        self.gui_num_bins         = num_bins;
        self.gui_spectrum_rx      = Some(shared.spectrum_rx.clone());
        self.gui_suppression_rx   = Some(shared.suppression_rx.clone());
        self.gui_fft_size         = Some(shared.fft_size.clone());
        self.shared               = Some(shared);

        self.pipeline = Some(dsp::pipeline::Pipeline::new(sr, num_ch, fft_size));
        context.set_latency_samples(fft_size as u32);

        // Push initial curves...
        // (same as before but use num_bins variable)
        if let Some(ref sh) = self.shared {
            sh.sample_rate.store(sr);
            sh.fft_size.store(fft_size);
            // ... push initial curves using num_bins ...
        }
        true
    }
```

Also add `gui_fft_size: Option<Arc<bridge::AtomicUsize>>` field to `SpectralForge` struct and `Default` impl.

Update `reset()`:

```rust
    fn reset(&mut self) {
        if let Some(pipeline) = &mut self.pipeline {
            let fft_size = crate::params::fft_size_from_choice(self.params.fft_size.value());
            pipeline.reset(self.sample_rate, self.num_channels, fft_size);
        }
    }
```

- [ ] **Step 3: Pass fft_size to editor**

In `create_editor()` call, add the `gui_fft_size` arc so the editor can display "FFT: 2048" or similar in the top bar. For now, just thread it through so future tasks can use it:

In `editor_ui.rs` `create_editor` signature, add:

```rust
    gui_fft_size: Option<Arc<crate::bridge::AtomicUsize>>,
```

In the editor closure, compute active num_bins from it:

```rust
let active_fft = gui_fft_size.as_ref().map(|a| a.load()).unwrap_or(2048);
let active_num_bins = active_fft / 2 + 1;
```

Pass `active_num_bins` to `compute_curve_response()` calls instead of `crate::dsp::pipeline::NUM_BINS`.

Also display the FFT size in the top bar (after the Ceil/Falloff controls):

```rust
ui.add_space(8.0);
ui.label(
    egui::RichText::new(format!("FFT {}", active_fft))
        .color(th::LABEL_DIM).size(9.0)
);
```

- [ ] **Step 4: Build**

```bash
cargo build 2>&1 | grep "^error" | head -20
```

Expected: zero errors.

- [ ] **Step 5: Run all tests**

```bash
cargo test 2>&1 | tail -8
```

Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add src/lib.rs src/bridge.rs src/editor_ui.rs
git commit -m "feat: wire fft_size through initialize, bridge, and editor"
```

---

## Task 4: GUI — FFT size selector in settings area

**Files:**
- Modify: `src/editor_ui.rs`

- [ ] **Step 1: Add FFT size buttons to top bar (after the Falloff DragValue)**

In the top bar horizontal layout, after the Falloff control:

```rust
                        ui.add_space(8.0);
                        ui.separator();
                        ui.add_space(4.0);
                        ui.label(egui::RichText::new("FFT").color(th::LABEL_DIM).size(9.0));

                        let current_fft = params.fft_size.value();
                        use crate::params::FftSizeChoice;
                        let fft_choices = [
                            (FftSizeChoice::S512,   "512"),
                            (FftSizeChoice::S1024,  "1k"),
                            (FftSizeChoice::S2048,  "2k"),
                            (FftSizeChoice::S4096,  "4k"),
                            (FftSizeChoice::S8192,  "8k"),
                            (FftSizeChoice::S16384, "16k"),
                        ];
                        for (choice, label) in fft_choices {
                            let active = current_fft == choice;
                            let fill   = if active { th::BORDER } else { th::BG };
                            let text_c = if active { th::BG } else { th::LABEL_DIM };
                            if ui.add(
                                egui::Button::new(
                                    egui::RichText::new(label).color(text_c).size(9.0)
                                )
                                .fill(fill)
                                .stroke(egui::Stroke::new(th::STROKE_BORDER, th::BORDER))
                                .min_size(egui::vec2(26.0, 16.0))
                            ).clicked() {
                                setter.begin_set_parameter(&params.fft_size);
                                setter.set_parameter(&params.fft_size, choice);
                                setter.end_set_parameter(&params.fft_size);
                            }
                        }
```

Note: Changing the FFT size will cause the host to call `initialize()` again (nih-plug triggers this when a param with `with_callback` changes, or on next process after the param changes). Display a brief note so the user knows a restart/reinitialize is expected:

Actually, in nih-plug, changing an `EnumParam` does not automatically trigger `initialize()`. The host won't necessarily call it either. We need to handle the size change gracefully. The safest approach: in `Pipeline::process()`, check if `self.fft_size` differs from `params.fft_size.value()`, and if so, rebuild FFT internals. However this risks allocation on the audio thread.

**Alternative approach (simpler, correct):** The FFT size change takes effect on the next `reset()` call. Add a note in the UI that the FFT size change takes effect on playback stop/start or plugin reload:

```rust
                        ui.add_space(4.0);
                        ui.label(
                            egui::RichText::new("(restart)")
                                .color(th::LABEL_DIM).size(8.0)
                        );
```

- [ ] **Step 2: Build and verify**

```bash
cargo build --release 2>&1 | grep "^error" | head -5
```

Expected: zero errors.

- [ ] **Step 3: Bundle and test**

```bash
cargo run --package xtask -- bundle spectral_forge --release && cp target/bundled/spectral_forge.clap ~/.clap/
```

Manual verification in Bitwig:
- [ ] FFT size buttons appear in top bar
- [ ] Selecting 512 and reloading: latency changes (plugin reports 512 samples latency)  
- [ ] Selecting 16384: plugin reports 16384 samples latency, frequency resolution visibly improves in spectrum display
- [ ] No audio glitches at any size

- [ ] **Step 4: Run all tests**

```bash
cargo test 2>&1 | tail -5
```

Expected: all pass.

- [ ] **Step 5: Final commit**

```bash
git add src/editor_ui.rs
git commit -m "feat: FFT size selector buttons in top bar (512–16384)"
```
