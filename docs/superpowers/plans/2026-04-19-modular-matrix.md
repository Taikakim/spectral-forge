> **Status (2026-04-24): IMPLEMENTED.** `FxMatrix` and slot-based routing landed as described. Source of truth: the code + [../STATUS.md](../STATUS.md).

# Modular 8×8 Matrix Architecture Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the single-engine Pipeline with an 8-slot FxMatrix where each slot is a named, typed processing module. Modules sit on the diagonal of an 8×8 routing grid; off-diagonal cells set send amplitudes between modules (including one-hop-delayed feedback from later slots to earlier ones). Plan C implements the infrastructure and the Dynamics module only; Plan D adds M/S, Plan E adds HPSS.

**Architecture:** A new `src/dsp/fx_matrix.rs` holds `FxMatrix` (8 `Option<FxSlotKind>` slots + `[[f32; 8]; 8]` send matrix). Pipeline replaces its `engine`/`engine_r`/`contrast_engine` fields with a single `fx_matrix: FxMatrix`. The STFT closure calls `fx_matrix.process_hop(channel, complex_buf, ...)` instead of the direct engine calls. Each slot holds pre-allocated `MAX_NUM_BINS` output buffers so routing never allocates. The GUI gains a new 8×8 matrix area below the main graph, and the graph header shows "Editing: [Module Name] - [Channel Target]".

**Assumes Plan A (serial FX chain) is already implemented** — `EffectMode` has been replaced by `DynamicsMode`, `freeze_enabled`, and `phase_enabled`. This plan does **not** touch the freeze/phase serial chain logic; it wraps the Dynamics slot around the existing dynamics engine call.

**Tech Stack:** Rust, nih-plug, realfft, egui, parking_lot, triple_buffer, serde

---

## Files Modified

| File | Changes |
|------|---------|
| `src/params.rs` | Add `FxModuleType`, `FxChannelTarget` enums; add `fx_module_types`, `fx_module_names`, `fx_module_targets`, `fx_route_matrix`, `editing_slot` persist fields |
| `src/dsp/fx_matrix.rs` | New file: `FxSlotKind`, `FxMatrix`, `process_hop()`, routing logic |
| `src/dsp/mod.rs` | Add `pub mod fx_matrix` |
| `src/dsp/pipeline.rs` | Replace `engine`, `engine_r`, `contrast_engine` with `fx_matrix: FxMatrix`; update `new()`, `reset()`, inner STFT closure |
| `src/editor/fx_matrix_grid.rs` | New file: `paint_fx_matrix_grid()` widget |
| `src/editor/mod.rs` | Add `pub mod fx_matrix_grid` |
| `src/editor_ui.rs` | Add 8×8 grid area below graph; add graph header label; wire `editing_slot` |
| `tests/engine_contract.rs` | Add `fx_matrix_passthrough_preserves_finite` test |

---

## Context for implementers

**Current Pipeline structure:** `engine: Box<dyn SpectralEngine>` handles the left channel (or both in Linked/MidSide mode). `engine_r` handles right in Independent mode. `contrast_engine` is a separate Spectral Contrast engine (used when `effect_mode == SpectralContrast`). After Plan A, `contrast_engine` is integrated into the Dynamics serial chain as a `DynamicsMode` selection, so it becomes part of the Dynamics slot.

**FxMatrix slot model:**
- Slot 0 always receives the main audio input (the windowed complex FFT frame).
- Each subsequent slot's input = sum of forward sends from earlier slots (`send[j][i]` where `j < i`, current hop) + feedback sends from later slots (`send[j][i]` where `j > i`, previous hop with one-hop delay).
- The last active slot's output is written back to `complex_buf` as the main audio output.
- If no slots are active, `complex_buf` passes through unchanged.

**Module types (Plan C only):**
- `FxModuleType::Empty` — slot unused, no processing
- `FxModuleType::Dynamics` — wraps the existing SpectralCompressorEngine (or SpectralContrast per DynamicsMode param); uses the existing 7-curve channel system

**`FxChannelTarget`:** Controls which STFT channels are processed by a slot.
- `All` — slot processes every channel (default; current behaviour)
- `Mid` — slot only processes channel 0 in MidSide stereo mode
- `Side` — slot only processes channel 1 in MidSide stereo mode
Gate implemented: inside `FxSlotKind::process()`, skip processing (pass through unchanged) if target is Mid/Side and the current `channel`/`stereo_link` combination doesn't match.

**Params persisted with `Arc<Mutex<T>>`:** `FxModuleType` and `FxChannelTarget` use `#[derive(serde::Serialize, serde::Deserialize)]` (not nih-plug `Enum`) so they can live in `Arc<Mutex<[T; 8]>>` persist fields. Add `serde = "1"` to Cargo.toml if not already present (nih-plug re-exports it as a feature).

**Editing slot vs active curve:** `editing_slot` (0–7) selects which module's curves are displayed in the graph. `active_curve` remains the sub-curve index within that module. For Plan C with only Dynamics in slot 0, `editing_slot` is always 0 and `active_curve` 0–6 behave exactly as before.

**Bridge stays unchanged** for Plan C. The 7 dynamics curve channels in the bridge always correspond to slot 0 (Dynamics). Plans D/E will extend the bridge with per-slot curve channels.

---

## Task 1: Module type params

**Files:**
- Modify: `src/params.rs`
- Test: `tests/engine_contract.rs`

- [ ] **Step 1: Write the failing test**

Add to `tests/engine_contract.rs`:

```rust
#[test]
fn fx_module_type_dynamics_is_slot_zero() {
    use spectral_forge::params::{FxModuleType, FxChannelTarget, SpectralForgeParams};
    let p = SpectralForgeParams::default();
    let types = p.fx_module_types.lock();
    assert_eq!(types[0], FxModuleType::Dynamics);
    for i in 1..8 {
        assert_eq!(types[i], FxModuleType::Empty, "slot {i} should be Empty by default");
    }
    let targets = p.fx_module_targets.lock();
    assert!(targets.iter().all(|&t| t == FxChannelTarget::All));
    let names = p.fx_module_names.lock();
    assert_eq!(&names[0], "Dynamics");
    assert_eq!(*p.editing_slot.lock(), 0u8);
}
```

- [ ] **Step 2: Run to verify it fails**

```bash
cargo test fx_module_type_dynamics_is_slot_zero 2>&1 | tail -5
```

Expected: compile error — `FxModuleType` not found.

- [ ] **Step 3: Add enums to params.rs**

After the `EffectMode` enum (or `DynamicsMode` if Plan A is done), add:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum FxModuleType {
    #[default]
    Empty,
    Dynamics,
    // MidSide,  // Plan D
    // Hpss,     // Plan E
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum FxChannelTarget {
    #[default]
    All,
    Mid,
    Side,
}

impl FxChannelTarget {
    pub fn label(self) -> &'static str {
        match self { Self::All => "All", Self::Mid => "Mid", Self::Side => "Side" }
    }
}
```

- [ ] **Step 4: Add persist fields to SpectralForgeParams struct**

In the `#[derive(Params)]` struct, after the `freeze_active_curve` field:

```rust
    /// Which module slot is currently selected for curve editing (0–7).
    #[persist = "editing_slot"]
    pub editing_slot: Arc<Mutex<u8>>,

    /// Module type for each of the 8 slots.
    #[persist = "fx_module_types"]
    pub fx_module_types: Arc<Mutex<[FxModuleType; 8]>>,

    /// User-editable display name for each slot.
    #[persist = "fx_module_names"]
    pub fx_module_names: Arc<Mutex<[String; 8]>>,

    /// Channel routing target for each slot.
    #[persist = "fx_module_targets"]
    pub fx_module_targets: Arc<Mutex<[FxChannelTarget; 8]>>,

    /// 8×8 send matrix. send[src][dst] = linear amplitude [0..1].
    /// src < dst: forward send (current hop). src > dst: feedback (one-hop delayed).
    #[persist = "fx_route_matrix"]
    pub fx_route_matrix: Arc<Mutex<[[f32; 8]; 8]>>,
```

- [ ] **Step 5: Add defaults in `Default for SpectralForgeParams`**

In the `Self { ... }` initialiser, after `freeze_active_curve`:

```rust
            editing_slot: Arc::new(Mutex::new(0u8)),

            fx_module_types: Arc::new(Mutex::new({
                let mut arr = [FxModuleType::Empty; 8];
                arr[0] = FxModuleType::Dynamics;
                arr
            })),

            fx_module_names: Arc::new(Mutex::new([
                "Dynamics".to_string(),
                "Slot 1".to_string(),
                "Slot 2".to_string(),
                "Slot 3".to_string(),
                "Slot 4".to_string(),
                "Slot 5".to_string(),
                "Slot 6".to_string(),
                "Slot 7".to_string(),
            ])),

            fx_module_targets: Arc::new(Mutex::new([FxChannelTarget::All; 8])),

            fx_route_matrix: Arc::new(Mutex::new([[0.0f32; 8]; 8])),
```

- [ ] **Step 6: Run test**

```bash
cargo test fx_module_type_dynamics_is_slot_zero 2>&1 | tail -5
```

Expected: `test fx_module_type_dynamics_is_slot_zero ... ok`

- [ ] **Step 7: Build the whole project**

```bash
cargo build 2>&1 | grep "^error" | head -10
```

Expected: zero errors. If serde is not available, add to `Cargo.toml` under `[dependencies]`: `serde = { version = "1", features = ["derive"] }`.

- [ ] **Step 8: Commit**

```bash
git add src/params.rs tests/engine_contract.rs
git commit -m "feat: add FxModuleType, FxChannelTarget enums and per-slot matrix params"
```

---

## Task 2: FxMatrix DSP infrastructure

**Files:**
- Create: `src/dsp/fx_matrix.rs`
- Modify: `src/dsp/mod.rs`
- Test: `tests/engine_contract.rs`

- [ ] **Step 1: Write the failing test**

Add to `tests/engine_contract.rs`:

```rust
#[test]
fn fx_matrix_passthrough_preserves_finite() {
    use spectral_forge::dsp::fx_matrix::FxMatrix;
    use spectral_forge::dsp::engines::BinParams;
    use spectral_forge::params::StereoLink;
    use num_complex::Complex;

    let num_bins = 1025usize;
    let mut fx = FxMatrix::new(44100.0, num_bins * 2 - 2); // fft_size = 2048

    // Make a non-trivial complex spectrum
    let mut bins: Vec<Complex<f32>> = (0..num_bins)
        .map(|k| Complex::new((k as f32 * 0.001).sin(), (k as f32 * 0.001).cos()))
        .collect();

    let threshold = vec![-20.0f32; num_bins];
    let ratio     = vec![4.0f32; num_bins];
    let attack    = vec![10.0f32; num_bins];
    let release   = vec![80.0f32; num_bins];
    let knee      = vec![6.0f32; num_bins];
    let makeup    = vec![0.0f32; num_bins];
    let mix       = vec![1.0f32; num_bins];
    let params = BinParams {
        threshold_db: &threshold,
        ratio:        &ratio,
        attack_ms:    &attack,
        release_ms:   &release,
        knee_db:      &knee,
        makeup_db:    &makeup,
        mix:          &mix,
        sensitivity:  0.0,
        auto_makeup:  false,
        smoothing_semitones: 0.0,
    };

    let mut supp_out = vec![0.0f32; num_bins];
    fx.process_hop(0, StereoLink::Linked, &mut bins, None, &params, 44100.0, &mut supp_out, num_bins);

    for (k, b) in bins.iter().enumerate() {
        assert!(b.re.is_finite() && b.im.is_finite(), "bin {k} is not finite: {b:?}");
    }
    for (k, &s) in supp_out.iter().enumerate() {
        assert!(s.is_finite() && s >= 0.0, "suppression[{k}] = {s}");
    }
}
```

- [ ] **Step 2: Run to verify it fails**

```bash
cargo test fx_matrix_passthrough_preserves_finite 2>&1 | tail -5
```

Expected: compile error — `fx_matrix` module not found.

- [ ] **Step 3: Add `pub mod fx_matrix` to `src/dsp/mod.rs`**

```rust
pub mod fx_matrix;
```

- [ ] **Step 4: Create `src/dsp/fx_matrix.rs`**

```rust
use num_complex::Complex;
use crate::dsp::engines::{BinParams, SpectralEngine, create_engine, EngineSelection};
use crate::params::StereoLink;

pub const MAX_SLOTS: usize = 8;

/// A single processing slot in the FxMatrix.
pub enum FxSlotKind {
    /// Dynamics: spectral compressor or contrast, using the existing 7-curve system.
    Dynamics {
        engine:   Box<dyn SpectralEngine>,
        engine_r: Box<dyn SpectralEngine>,   // right-channel engine for Independent mode
        contrast: Box<dyn SpectralEngine>,   // contrast engine (for DynamicsMode::Contrast)
    },
    // MidSide { ... }   // Plan D
    // Hpss    { ... }   // Plan E
}

impl FxSlotKind {
    pub fn reset(&mut self, sample_rate: f32, fft_size: usize) {
        match self {
            Self::Dynamics { engine, engine_r, contrast } => {
                engine.reset(sample_rate, fft_size);
                engine_r.reset(sample_rate, fft_size);
                contrast.reset(sample_rate, fft_size);
            }
        }
    }

    /// Process `bins` in place. Channel gating based on `target` and `stereo_link`
    /// is applied — if this slot targets Mid but we're on channel 1 (Side), pass through.
    pub fn process_dynamics(
        &mut self,
        channel: usize,
        stereo_link: StereoLink,
        target: crate::params::FxChannelTarget,
        bins: &mut [Complex<f32>],
        sidechain: Option<&[f32]>,
        params: &BinParams<'_>,
        dynamics_mode: crate::params::DynamicsMode,
        sample_rate: f32,
        suppression_out: &mut [f32],
    ) {
        use crate::params::FxChannelTarget;

        // Channel gating: skip processing if target doesn't match this channel.
        let skip = match (target, stereo_link, channel) {
            (FxChannelTarget::Mid,  StereoLink::MidSide, 1) => true,
            (FxChannelTarget::Side, StereoLink::MidSide, 0) => true,
            (FxChannelTarget::Mid  | FxChannelTarget::Side, StereoLink::Linked | StereoLink::Independent, _) => true,
            _ => false,
        };
        if skip {
            suppression_out.fill(0.0);
            return;
        }

        let Self::Dynamics { engine, engine_r, contrast } = self else { return };

        let eng: &mut Box<dyn SpectralEngine> = match stereo_link {
            StereoLink::Independent if channel == 1 => engine_r,
            _ => engine,
        };

        match dynamics_mode {
            crate::params::DynamicsMode::Compressor | crate::params::DynamicsMode::Bypass => {
                eng.process_bins(bins, sidechain, params, sample_rate, suppression_out);
            }
            crate::params::DynamicsMode::Contrast => {
                contrast.process_bins(bins, sidechain, params, sample_rate, suppression_out);
            }
        }
    }
}

/// 8-slot spectral routing matrix.
///
/// Slot 0 always receives the main audio input. Later slots receive audio through
/// `send[j][i]` amplitudes (j = source slot, i = destination slot).
/// Forward sends (j < i) use the current hop's output; feedback sends (j > i)
/// use the previous hop's output (one-hop delay).
/// The last active slot's output becomes the main audio output.
pub struct FxMatrix {
    /// Slot type instances. `None` = slot is unused (pass-through in chain).
    pub slots: [Option<FxSlotKind>; MAX_SLOTS],

    /// Routing matrix. `send[src][dst]` = linear amplitude of src's output
    /// added to dst's input. 0.0 = no connection.
    pub send: [[f32; MAX_SLOTS]; MAX_SLOTS],

    /// Per-slot complex output buffer, current hop. Sized at MAX_NUM_BINS.
    slot_out_cur:  Vec<Vec<Complex<f32>>>,

    /// Per-slot complex output buffer, previous hop (feedback reference).
    slot_out_prev: Vec<Vec<Complex<f32>>>,

    /// Per-slot gain-reduction output for the GUI suppression display.
    slot_supp: Vec<Vec<f32>>,
}

impl FxMatrix {
    /// Allocate at `fft_size`. `MAX_NUM_BINS = MAX_FFT_SIZE / 2 + 1` is used
    /// for all inner Vecs so the struct never reallocates when `fft_size` changes.
    pub fn new(sample_rate: f32, fft_size: usize) -> Self {
        use crate::dsp::pipeline::MAX_NUM_BINS;

        let mut slots: [Option<FxSlotKind>; MAX_SLOTS] =
            std::array::from_fn(|_| None);

        // Slot 0 = Dynamics by default.
        let mut engine   = create_engine(EngineSelection::SpectralCompressor);
        let mut engine_r = create_engine(EngineSelection::SpectralCompressor);
        let mut contrast = create_engine(EngineSelection::SpectralContrast);
        engine.reset(sample_rate, fft_size);
        engine_r.reset(sample_rate, fft_size);
        contrast.reset(sample_rate, fft_size);

        slots[0] = Some(FxSlotKind::Dynamics { engine, engine_r, contrast });

        Self {
            slots,
            send: [[0.0f32; MAX_SLOTS]; MAX_SLOTS],
            slot_out_cur:  (0..MAX_SLOTS)
                .map(|_| vec![Complex::new(0.0f32, 0.0f32); MAX_NUM_BINS])
                .collect(),
            slot_out_prev: (0..MAX_SLOTS)
                .map(|_| vec![Complex::new(0.0f32, 0.0f32); MAX_NUM_BINS])
                .collect(),
            slot_supp:     (0..MAX_SLOTS)
                .map(|_| vec![0.0f32; MAX_NUM_BINS])
                .collect(),
        }
    }

    pub fn reset(&mut self, sample_rate: f32, fft_size: usize) {
        let num_bins = fft_size / 2 + 1;
        for slot in self.slots.iter_mut().flatten() {
            slot.reset(sample_rate, fft_size);
        }
        for buf in self.slot_out_cur.iter_mut() {
            buf[..num_bins].fill(Complex::new(0.0, 0.0));
        }
        for buf in self.slot_out_prev.iter_mut() {
            buf[..num_bins].fill(Complex::new(0.0, 0.0));
        }
        for buf in self.slot_supp.iter_mut() {
            buf[..num_bins].fill(0.0);
        }
    }

    /// Rebuild slot 0 to match the given types array. Called from Pipeline when
    /// `FxModuleType` params change (future: reconfigure all 8 slots).
    /// For Plan C: only slot 0 is managed. If type is Empty, clears slot 0.
    pub fn sync_slot_types(
        &mut self,
        types: &[crate::params::FxModuleType; 8],
        sample_rate: f32,
        fft_size: usize,
    ) {
        use crate::params::FxModuleType;
        // Plan C: only slot 0 handled.
        match types[0] {
            FxModuleType::Dynamics => {
                if self.slots[0].is_none() {
                    let mut e  = create_engine(EngineSelection::SpectralCompressor);
                    let mut er = create_engine(EngineSelection::SpectralCompressor);
                    let mut c  = create_engine(EngineSelection::SpectralContrast);
                    e.reset(sample_rate, fft_size);
                    er.reset(sample_rate, fft_size);
                    c.reset(sample_rate, fft_size);
                    self.slots[0] = Some(FxSlotKind::Dynamics {
                        engine: e, engine_r: er, contrast: c,
                    });
                }
            }
            FxModuleType::Empty => {
                self.slots[0] = None;
            }
        }
    }

    /// Process one STFT hop through the slot chain.
    ///
    /// `complex_buf`: the windowed FFT frame (modified in place; slot 0 input, last-slot output).
    /// `sidechain`: optional pre-smoothed SC magnitude per bin.
    /// `suppression_out`: filled with gain-reduction dB from the first Dynamics slot (slot 0).
    /// `num_bins`: active bin count = `fft_size / 2 + 1`.
    pub fn process_hop(
        &mut self,
        channel: usize,
        stereo_link: StereoLink,
        complex_buf: &mut [Complex<f32>],
        sidechain: Option<&[f32]>,
        params: &BinParams<'_>,
        // Extra per-slot context params (only Dynamics needs these for Plan C)
        dynamics_mode: crate::params::DynamicsMode,
        target0: crate::params::FxChannelTarget,
        sample_rate: f32,
        suppression_out: &mut [f32],
        num_bins: usize,
    ) {
        let mut last_active: Option<usize> = None;

        for i in 0..MAX_SLOTS {
            if self.slots[i].is_none() {
                continue;
            }

            // Assemble slot i's input into slot_out_cur[i].
            // Slot 0: main audio input is complex_buf.
            // Slot i>0: only routing sends (no implicit serial chain).
            //
            // Borrow safety: we write to slot_out_cur[i] only after reading
            // slot_out_cur[j] for j < i (already committed this hop) and
            // slot_out_prev[j] for j > i (previous hop). No overlap.

            // Forward sends: slot_out_cur[j] for j < i (current hop, j already done).
            // We accumulate into slot_out_cur[i], which is untouched this iteration.
            {
                // scope to drop immutable borrows before the mutable process call below
                let init: &[Complex<f32>] = if i == 0 { &complex_buf[..num_bins] } else { &[] };

                // Fill slot_out_cur[i] with the assembled input.
                if i == 0 {
                    self.slot_out_cur[0][..num_bins]
                        .copy_from_slice(&complex_buf[..num_bins]);
                } else {
                    self.slot_out_cur[i][..num_bins].fill(Complex::new(0.0, 0.0));
                }
                drop(init); // just to silence the unused-variable warning

                // Forward sends (j < i): slot_out_cur[j], same hop.
                // Use split_at to avoid aliasing slot_out_cur[i] while reading slot_out_cur[j].
                if i > 0 {
                    let (left, right) = self.slot_out_cur.split_at_mut(i);
                    // left[j] = slot_out_cur[j] for j in 0..i
                    // right[0] = slot_out_cur[i] (destination)
                    for j in 0..i {
                        let amp = self.send[j][i];
                        if amp.abs() > 1e-6 {
                            for k in 0..num_bins {
                                right[0][k] += left[j][k] * amp;
                            }
                        }
                    }
                }

                // Feedback sends (j > i): slot_out_prev[j], previous hop.
                for j in (i + 1)..MAX_SLOTS {
                    let amp = self.send[j][i];
                    if amp.abs() > 1e-6 {
                        let src = &self.slot_out_prev[j];
                        for k in 0..num_bins {
                            self.slot_out_cur[i][k] += src[k] * amp;
                        }
                    }
                }
            }

            // Process slot i in place on slot_out_cur[i].
            // `self.slots[i]` and `self.slot_out_cur[i]` are different fields — no conflict.
            let target = if i == 0 { target0 } else { crate::params::FxChannelTarget::All };

            // Temporarily take the slot to avoid simultaneous borrow of self.slots and self.slot_out_cur.
            if let Some(mut slot) = self.slots[i].take() {
                match &mut slot {
                    FxSlotKind::Dynamics { .. } => {
                        slot.process_dynamics(
                            channel,
                            stereo_link,
                            target,
                            &mut self.slot_out_cur[i][..num_bins],
                            sidechain,
                            params,
                            dynamics_mode,
                            sample_rate,
                            &mut self.slot_supp[i][..num_bins],
                        );
                    }
                }
                self.slots[i] = Some(slot);
            }

            last_active = Some(i);
        }

        // Write last active slot's output back to complex_buf.
        if let Some(i) = last_active {
            complex_buf[..num_bins].copy_from_slice(&self.slot_out_cur[i][..num_bins]);
            // Suppression display: use slot 0 (the Dynamics slot).
            suppression_out[..num_bins].copy_from_slice(&self.slot_supp[0][..num_bins]);
        }
        // If no slots active: complex_buf is unchanged (audio passes through).

        // Rotate buffers: current hop's outputs become previous hop for next hop's feedback.
        std::mem::swap(&mut self.slot_out_cur, &mut self.slot_out_prev);
        for buf in self.slot_out_cur.iter_mut() {
            buf[..num_bins].fill(Complex::new(0.0, 0.0));
        }
    }
}
```

- [ ] **Step 5: Run test**

```bash
cargo test fx_matrix_passthrough_preserves_finite 2>&1 | tail -5
```

Expected: `test fx_matrix_passthrough_preserves_finite ... ok`

- [ ] **Step 6: Run all tests**

```bash
cargo test 2>&1 | tail -8
```

Expected: all pass.

- [ ] **Step 7: Commit**

```bash
git add src/dsp/fx_matrix.rs src/dsp/mod.rs tests/engine_contract.rs
git commit -m "feat: FxMatrix DSP infrastructure with single Dynamics slot and routing buffers"
```

---

## Task 3: Pipeline integration

**Files:**
- Modify: `src/dsp/pipeline.rs`

This task removes `engine`, `engine_r`, and `contrast_engine` from Pipeline and replaces them with `fx_matrix: FxMatrix`. Behaviour is identical for Plan C (single Dynamics slot, same serial chain as before).

- [ ] **Step 1: Write the failing test**

This test verifies that removing the old engine fields and using FxMatrix produces identical finite output as before.

Add to `tests/engine_contract.rs`:

```rust
#[test]
fn pipeline_with_fx_matrix_produces_finite_output() {
    use spectral_forge::dsp::pipeline::process_block_for_test;
    let input = vec![0.3f32; 4096];
    let output = process_block_for_test(&input, 44100.0);
    assert!(!output.is_empty());
    for &s in &output {
        assert!(s.is_finite(), "sample must be finite, got {s}");
    }
}
```

This test already exists. Run it to confirm it passes before making changes:

```bash
cargo test pipeline_with_fx_matrix_produces_finite_output 2>&1 | tail -5
```

(It may not exist under that exact name — find the equivalent test in `engine_contract.rs` that calls `process_block_for_test`. Confirm it passes first, then add the above if it doesn't exist.)

- [ ] **Step 2: Replace fields in Pipeline struct**

In `src/dsp/pipeline.rs`, remove:

```rust
    engine:   Box<dyn SpectralEngine>,
    engine_r: Box<dyn SpectralEngine>,
    // ...
    contrast_engine: Box<dyn SpectralEngine>,
```

Add:

```rust
    fx_matrix: crate::dsp::fx_matrix::FxMatrix,
```

Also remove the `use crate::dsp::engines::{..., create_engine, EngineSelection};` import line (FxMatrix handles engine creation internally). Keep `BinParams`, `SpectralEngine` if used elsewhere in pipeline.rs, or remove if unused.

- [ ] **Step 3: Update Pipeline::new()**

Remove the engine/engine_r/contrast_engine construction and reset calls:

```rust
        // Remove:
        // let mut engine = create_engine(EngineSelection::SpectralCompressor);
        // engine.reset(sample_rate, FFT_SIZE);
        // let mut engine_r = ...
        // let mut contrast_engine = ...

        // Add:
        let fx_matrix = crate::dsp::fx_matrix::FxMatrix::new(sample_rate, FFT_SIZE);
```

In the `Self { ... }` block, replace the three engine fields with:

```rust
            fx_matrix,
```

- [ ] **Step 4: Update Pipeline::reset()**

Replace:

```rust
        self.engine.reset(sample_rate, FFT_SIZE);
        self.engine_r.reset(sample_rate, FFT_SIZE);
        self.contrast_engine.reset(sample_rate, FFT_SIZE);
```

With:

```rust
        self.fx_matrix.reset(sample_rate, FFT_SIZE);
```

- [ ] **Step 5: Update the STFT closure in Pipeline::process()**

The inner STFT closure currently calls the engine directly. Find the section that calls `engine.process_bins(...)` and the `effect_mode` / `contrast_engine` branch. Replace with a call to `fx_matrix.process_hop()`.

The current call site looks like (simplified from your existing Plan A state — adjust to match actual code):

```rust
// Before (Plan A serial chain style):
engine.process_bins(&mut complex_buf, sidechain.as_deref(), &bin_params, sr, &mut channel_supp_buf);
// + freeze and phase stages
```

After:

```rust
// Read DynamicsMode and editing_slot targets from params (read once before STFT closure).
// These are captured by value in the closure.
let dynamics_mode   = params.dynamics_mode.value();  // Plan A field name
let target0: crate::params::FxChannelTarget = {
    let tgts = params.fx_module_targets.lock();
    tgts[0]
};

// Inside the STFT closure, replace direct engine call with:
fx_matrix.process_hop(
    channel,
    stereo_link,
    complex_buf,
    sidechain_slice,
    &bin_params,
    dynamics_mode,
    target0,
    sample_rate,
    &mut channel_supp_buf,
    num_bins,
);
```

**Important:** `fx_matrix` and all closure-captured locals must be rebound before the STFT closure to avoid conflicting borrows of `self`. Follow the existing pattern in pipeline.rs where fields are extracted to locals before `stft.process_overlap_add`:

```rust
        let fx_matrix = &mut self.fx_matrix;
        let channel_supp_buf = &mut self.channel_supp_buf;
        // ... other locals ...
        self.stft.process_overlap_add(buffer, OVERLAP, |channel, complex_buf| {
            // use fx_matrix, channel_supp_buf, etc. here
        });
```

- [ ] **Step 6: Build**

```bash
cargo build 2>&1 | grep "^error" | head -20
```

Fix any borrow checker errors (typically: rebind more fields as locals before the STFT closure). Expected: zero errors.

- [ ] **Step 7: Run all tests**

```bash
cargo test 2>&1 | tail -8
```

Expected: all tests pass. If `process_block_for_test` uses `Pipeline` internals, it may need updating to not reference `engine`/`engine_r` directly.

- [ ] **Step 8: Commit**

```bash
git add src/dsp/pipeline.rs
git commit -m "feat: replace engine/engine_r/contrast_engine with fx_matrix in Pipeline"
```

---

## Task 4: 8×8 GUI matrix widget

**Files:**
- Create: `src/editor/fx_matrix_grid.rs`
- Modify: `src/editor/mod.rs`
- Modify: `src/editor_ui.rs`

The 8×8 grid sits below the main graph area. It is always visible. Each row/column pair (i, j) is a cell:
- `i == j` (diagonal): module cell — shows module type name, click selects it for editing.
- `i != j` (off-diagonal): send cell — shows a DragValue for `send[col][row]` (col = source, row = dest).
  - Lower triangle (`col < row`): forward send. Normal brightness.
  - Upper triangle (`col > row`): feedback send. Shown dimmer (darker background).
- Empty slots (type = Empty): diagonal cell shows "+" button to add a module.

Cell size: 48×48 px. Grid is 8×48 = 384 px wide, 8×48 = 384 px tall. Add 48 px left label column for row labels → total 432×384 px. This area goes at the bottom of the plugin window. The plugin window height needs to increase by ~400 px.

- [ ] **Step 1: Update window height**

In `src/params.rs`, `Default for SpectralForgeParams`:

```rust
editor_state: EguiState::from_size(900, 1010),  // was 600, add 410 for matrix
```

- [ ] **Step 2: Create `src/editor/fx_matrix_grid.rs`**

```rust
use nih_plug_egui::egui::{self, Color32, Painter, Rect, Stroke, Ui, Vec2};
use parking_lot::Mutex;
use std::sync::Arc;
use crate::editor::theme as th;
use crate::params::{FxModuleType, FxChannelTarget};

const CELL: f32  = 48.0;
const LABEL: f32 = 48.0;   // left column width for row labels

/// Draw the 8×8 routing matrix grid. Returns the slot index clicked (if any),
/// so the caller can update `editing_slot`.
///
/// `module_types`:  which module occupies each slot.
/// `module_names`:  display name per slot.
/// `send_matrix`:   `send[src][dst]` read from `params.fx_route_matrix`.
/// `editing_slot`:  which slot is currently selected for curve editing.
pub fn paint_fx_matrix_grid(
    ui: &mut Ui,
    module_types:  &[FxModuleType; 8],
    module_names:  &[String; 8],
    send_matrix:   &mut [[f32; 8]; 8],
    editing_slot:  usize,
) -> Option<usize> {
    let (response, painter) =
        ui.allocate_painter(Vec2::new(LABEL + 8.0 * CELL, 8.0 * CELL), egui::Sense::hover());
    let origin = response.rect.min;

    let mut clicked_slot: Option<usize> = None;

    for row in 0..8usize {
        // Row label (module name, left column)
        let label_rect = Rect::from_min_size(
            origin + egui::vec2(0.0, row as f32 * CELL),
            Vec2::new(LABEL - 2.0, CELL),
        );
        painter.text(
            label_rect.center(),
            egui::Align2::CENTER_CENTER,
            &module_names[row],
            egui::FontId::proportional(9.0),
            th::LABEL_DIM,
        );

        for col in 0..8usize {
            let cell_rect = Rect::from_min_size(
                origin + egui::vec2(LABEL + col as f32 * CELL, row as f32 * CELL),
                Vec2::new(CELL - 1.0, CELL - 1.0),
            );

            if row == col {
                // Diagonal: module cell
                paint_module_cell(
                    ui, &painter, cell_rect,
                    module_types[row],
                    &module_names[row],
                    row == editing_slot,
                    &mut clicked_slot,
                    row,
                );
            } else {
                // Off-diagonal: send amount
                let is_feedback = col > row; // upper triangle = feedback
                paint_send_cell(
                    ui, cell_rect,
                    &mut send_matrix[col][row],
                    is_feedback,
                );
            }
        }
    }

    clicked_slot
}

fn paint_module_cell(
    ui: &mut Ui,
    painter: &Painter,
    rect: Rect,
    module_type: FxModuleType,
    name: &str,
    is_selected: bool,
    clicked_slot: &mut Option<usize>,
    slot_idx: usize,
) {
    let fill = match (module_type, is_selected) {
        (FxModuleType::Empty, _)         => th::BG_RAISED,
        (FxModuleType::Dynamics, true)   => th::CURVE_COLORS_LIT[0],
        (FxModuleType::Dynamics, false)  => th::CURVE_COLORS_DIM[0],
    };
    let stroke = if is_selected {
        Stroke::new(1.5, th::BORDER)
    } else {
        Stroke::new(0.5, th::GRID_LINE)
    };
    painter.rect(rect, 2.0, fill, stroke);

    let text = match module_type {
        FxModuleType::Empty    => "+",
        FxModuleType::Dynamics => name,
    };
    let text_color = match (module_type, is_selected) {
        (FxModuleType::Empty, _) => th::LABEL_DIM,
        (_, true)                => th::BG,
        (_, false)               => th::LABEL_DIM,
    };
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        text,
        egui::FontId::proportional(9.0),
        text_color,
    );

    let interact = ui.interact(rect, ui.id().with(("mod_cell", slot_idx)), egui::Sense::click());
    if interact.clicked() {
        *clicked_slot = Some(slot_idx);
    }
}

fn paint_send_cell(
    ui: &mut Ui,
    rect: Rect,
    send: &mut f32,
    is_feedback: bool,
) {
    let bg = if is_feedback { th::BG_FEEDBACK } else { th::BG_RAISED };
    let (response, painter) =
        ui.allocate_painter(rect.size(), egui::Sense::hover());
    let actual_rect = response.rect;
    painter.rect(actual_rect, 0.0, bg, Stroke::new(0.5, th::GRID_LINE));

    // DragValue for the send amplitude [0..1]
    ui.allocate_ui_at_rect(actual_rect.shrink(2.0), |ui| {
        let drag = egui::DragValue::new(send)
            .range(0.0..=1.0)
            .speed(0.005)
            .fixed_decimals(2)
            .custom_formatter(|v, _| {
                if v < 0.005 { "—".to_string() } else { format!("{v:.2}") }
            });
        ui.add(drag);
    });
}
```

**Theme constants needed** — add to `src/editor/theme.rs` if not already present:

```rust
pub const BG_RAISED:   Color32 = Color32::from_rgb(0x20, 0x20, 0x20);
pub const BG_FEEDBACK: Color32 = Color32::from_rgb(0x14, 0x14, 0x1e);  // slightly blue-tinted
pub const GRID_LINE:   Color32 = Color32::from_rgb(0x30, 0x30, 0x30);
pub const GRID_TEXT:   Color32 = Color32::from_rgb(0x45, 0x45, 0x45);
pub const LABEL_DIM:   Color32 = Color32::from_rgb(0x70, 0x70, 0x70);
pub const BORDER:      Color32 = Color32::from_rgb(0xa0, 0xa0, 0xa0);
```

If `CURVE_COLORS_LIT` and `CURVE_COLORS_DIM` don't exist yet (they're added in the GUI redesign plan), use placeholders:

```rust
// Temporary until GUI redesign plan adds CURVE_COLORS_LIT/DIM arrays:
pub const MODULE_COLOR_LIT: Color32 = Color32::from_rgb(0x50, 0xc0, 0xc4); // teal
pub const MODULE_COLOR_DIM: Color32 = Color32::from_rgb(0x20, 0x40, 0x41);
```

Then in `paint_module_cell`, substitute `MODULE_COLOR_LIT`/`MODULE_COLOR_DIM` for `th::CURVE_COLORS_LIT[0]`/`th::CURVE_COLORS_DIM[0]`.

- [ ] **Step 3: Add `pub mod fx_matrix_grid` to `src/editor/mod.rs`**

```rust
pub mod fx_matrix_grid;
```

- [ ] **Step 4: Wire the grid into `editor_ui.rs`**

In the `create_editor` closure, after the existing graph area and control strip, add a new section for the matrix:

```rust
                    // ── FX Matrix ──────────────────────────────────────────────
                    ui.add_space(4.0);
                    ui.separator();
                    ui.add_space(4.0);

                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new("ROUTING MATRIX")
                                .color(th::LABEL_DIM).size(9.0),
                        );
                    });

                    let mut types   = *params.fx_module_types.lock();
                    let mut names   = params.fx_module_names.lock().clone();
                    let mut targets = *params.fx_module_targets.lock();
                    let mut matrix  = *params.fx_route_matrix.lock();
                    let edit_slot   = *params.editing_slot.lock() as usize;

                    let clicked = crate::editor::fx_matrix_grid::paint_fx_matrix_grid(
                        ui,
                        &types,
                        &names,
                        &mut matrix,
                        edit_slot,
                    );

                    // Write back any matrix changes
                    *params.fx_route_matrix.lock() = matrix;

                    // Update editing slot if a module cell was clicked
                    if let Some(new_slot) = clicked {
                        *params.editing_slot.lock() = new_slot as u8;
                    }
```

- [ ] **Step 5: Build**

```bash
cargo build 2>&1 | grep "^error" | head -20
```

Expected: zero errors.

- [ ] **Step 6: Bundle and visual check**

```bash
cargo run --package xtask -- bundle spectral_forge --release && cp target/bundled/spectral_forge.clap ~/.clap/
```

Open in Bitwig. Confirm:
- [ ] Plugin window is taller (matrix grid visible below control strip)
- [ ] 8×8 grid renders with a teal diagonal cell at (0,0) labelled "Dynamics"
- [ ] Cells (1,1)–(7,7) show "+" (Empty slots)
- [ ] Off-diagonal cells show "—" (send = 0)
- [ ] Clicking the Dynamics cell highlights it with a border
- [ ] Audio still processes correctly (no regression)

- [ ] **Step 7: Run all tests**

```bash
cargo test 2>&1 | tail -8
```

Expected: all pass.

- [ ] **Step 8: Commit**

```bash
git add src/editor/fx_matrix_grid.rs src/editor/mod.rs src/editor_ui.rs src/editor/theme.rs src/params.rs
git commit -m "feat: 8x8 routing matrix GUI grid with module cells and send DragValues"
```

---

## Task 5: Graph header and editing slot

**Files:**
- Modify: `src/editor_ui.rs`

The graph area's top-left corner shows a label: `"Editing: {module_name} - {channel_target}"`. When the editing slot changes (by clicking a matrix diagonal cell in Task 4), the curve editor continues to operate on the same 7 Dynamics curves — in Plan C there is only one module type, so the curves are always the same. In future Plans D/E, `editing_slot` will determine which set of curves is shown.

- [ ] **Step 1: Write the failing test**

This is a pure GUI change with no unit-testable logic. Instead, verify compilation:

```bash
cargo build 2>&1 | grep "^error" | head -5
```

Expected: zero errors (test that the code at least compiles after the change).

- [ ] **Step 2: Add graph header label to the graph area**

In `editor_ui.rs`, find where the graph `Rect` is allocated (the curve editor area). Just before or just inside the `ui.allocate_painter(...)` call for the graph, add a floating label in the top-left corner:

```rust
                    // Graph header: "Editing: {name} - {target}"
                    {
                        let edit_slot  = *params.editing_slot.lock() as usize;
                        let names  = params.fx_module_names.lock();
                        let tgts   = params.fx_module_targets.lock();
                        let header = format!("Editing: {} — {}", names[edit_slot], tgts[edit_slot].label());
                        // Draw in the top-left of the graph rect, 2 px inside the border.
                        // `graph_rect` is the Rect of the curve editor area — adjust to the actual variable name.
                        ui.painter().text(
                            graph_rect.min + egui::vec2(4.0, 4.0),
                            egui::Align2::LEFT_TOP,
                            &header,
                            egui::FontId::proportional(10.0),
                            th::LABEL_DIM,
                        );
                    }
```

If the graph rect is computed later in the layout, locate the `Painter` or `Ui` available at graph paint time and insert the label there. The `ui.painter()` call uses the painter for the current `Ui` region.

- [ ] **Step 3: Verify editing_slot updates when matrix cell is clicked**

From Task 4, `*params.editing_slot.lock() = new_slot as u8` is already wired. Confirm that re-renders after a click show the updated module name in the header. (Verify visually in Bitwig — no unit test for this interaction.)

- [ ] **Step 4: Build**

```bash
cargo build --release 2>&1 | grep "^error" | head -5
```

Expected: zero errors.

- [ ] **Step 5: Bundle and manual verify**

```bash
cargo run --package xtask -- bundle spectral_forge --release && cp target/bundled/spectral_forge.clap ~/.clap/
```

In Bitwig:
- [ ] Graph area shows "Editing: Dynamics — All" in top-left corner
- [ ] Clicking the Dynamics cell in the matrix keeps it selected and header unchanged
- [ ] Clicking an empty slot cell (shows "+") changes `editing_slot` — header reads "Editing: Slot N — All"
- [ ] All 7 dynamics curves still function correctly
- [ ] Audio processing unchanged

- [ ] **Step 6: Run full test suite**

```bash
cargo test 2>&1 | tail -5
```

Expected: all pass.

- [ ] **Step 7: Final commit**

```bash
git add src/editor_ui.rs
git commit -m "feat: graph header shows editing slot name and channel target"
```

---

## Verification

```bash
cargo test            # all tests must pass
cargo build --release # zero errors, zero warnings
```

Then:

```bash
cargo run --package xtask -- bundle spectral_forge --release
cp target/bundled/spectral_forge.clap ~/.clap/
```

In Bitwig, confirm end-to-end:

- [ ] Audio processes correctly with one active Dynamics module
- [ ] 8×8 routing matrix is visible below the existing controls
- [ ] Clicking a diagonal cell selects it and updates the graph header
- [ ] Off-diagonal DragValues in lower triangle: dragging changes the value from "—" to a number
- [ ] Off-diagonal cells in upper triangle: rendered with a different (darker/blue-tinted) background to indicate feedback
- [ ] Graph header reads "Editing: Dynamics — All" for slot 0
- [ ] Preset save/restore: slot names, types, route matrix, editing_slot all survive a Bitwig project reload
- [ ] No audio glitches or crashes with the FxMatrix path active
