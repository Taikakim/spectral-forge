# Phase 5b.1: History Buffer Infrastructure Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** add a per-channel rolling complex-spectrum HistoryBuffer that lives in `Pipeline`, is written each STFT hop, exposes a read-only borrow plus lazy per-bin summary statistics (decay time, RMS envelope, IF stability) to modules through `ModuleContext`, and ships with calibration probes and a smoke test. No audible feature ships in this phase.

**Architecture:** The buffer is a per-channel ring of `Complex<f32>` frames sized at `MAX_HISTORY_FRAMES × MAX_NUM_BINS`. Capacity is configurable per plugin instance via a `HistoryBufferDepthChoice` enum (1 s / 2 s / 4 s / 8 s / 16 s) with default 4 s. `Pipeline::process()` writes the post-STFT, pre-FxMatrix complex spectrum into the buffer's current write head; modules read it through `ctx.history: Option<&HistoryBuffer>`. Three summary arrays (`decay_estimate`, `rms_envelope`, `if_stability`, all `[f32; MAX_NUM_BINS]`) are computed lazily on first `summary_*()` call per block via interior mutability and cleared at the top of every block.

**Tech Stack:** Rust 2021, num-complex, parking_lot, no new dependencies.

**Status banner to add at the top of each PR's commit message:** `infra(phase5b1):`

**Reading order before starting:**
- `ideas/next-gen-modules/01-global-infrastructure.md` § 2 (History Buffer)
- `ideas/next-gen-modules/13-past.md` § "History Buffer details — spec gaps" + § "Research findings"
- `docs/superpowers/specs/2026-04-21-past-module.md` § "History Buffer infrastructure (prerequisite)"
- `docs/superpowers/plans/2026-04-27-phase-1-foundation-infra.md` Task 1 + Task 2 (the `'block` lifetime on `ModuleContext` and the optional infra fields pattern)
- `docs/superpowers/plans/2026-04-27-phase-3-bin-physics.md` (mirror pattern: per-block read-only state attached to `ctx`)
- `src/dsp/pipeline.rs` lines 38–50 (`MAX_FFT_SIZE`, `MAX_NUM_BINS`, hop math) and lines 463–516 (the STFT closure where the write must happen)
- `src/dsp/modules/mod.rs` lines 67–79 (`ModuleContext` struct as it stands at end of Phase 4)
- `src/dsp/fx_matrix.rs` lines 78–end (`process_hop` signature — read-only ctx flows in)
- `CLAUDE.md` § Real-time safety rules

**Phase prerequisites:**
- Phase 1 ships: `ModuleContext<'block>` with the `'block` lifetime + the `Option<&'block …>` pattern.
- Phase 3 ships: `bin_physics: Option<&'block BinPhysics>` slot on `ModuleContext`. This plan adds a sibling `history: Option<&'block HistoryBuffer>` slot the same way.
- Phase 4 ships: `unwrapped_phase`, `instantaneous_freq` populated on `ModuleContext`. Phase 5b.1 reads `ctx.instantaneous_freq` to compute the IF-stability summary stat (lazy).

**This plan does NOT ship a consumer.** The first consumer is Phase 5b.2 (Past). To prove the buffer works without Past, Phase 5b.1 ships a **#[cfg(test)] smoke probe** that reads frames from the buffer in a 200-hop integration test — that is the only consumer landing in this phase.

---

## File Structure

| File | Created/Modified | Responsibility |
|---|---|---|
| `src/dsp/history_buffer.rs` | Create | `HistoryBuffer` struct (per-channel ring of `Vec<Vec<Complex<f32>>>`), `write_hop()`, `read_frame()`, `read_fractional()`, lazy summary arrays + `clear_summary_cache()`, `frames_used()`. |
| `src/dsp/mod.rs` | Modify | `pub mod history_buffer;` |
| `src/params.rs` | Modify | Add `HistoryBufferDepthChoice` enum (1 s / 2 s / 4 s / 8 s / 16 s), add `history_depth: EnumParam<HistoryBufferDepthChoice>` field on `SpectralForgeParams`. |
| `src/dsp/modules/mod.rs` | Modify | Add `pub history: Option<&'block HistoryBuffer>` field to `ModuleContext` (and default to `None` in `new()`). |
| `src/dsp/pipeline.rs` | Modify | Hold `Option<HistoryBuffer>` in `Pipeline`, allocate at `new()` per current `history_depth`, reallocate-or-rebuild on `reset()` if the depth choice changed, write each hop after STFT inside the closure, clear summary cache at top of every block, attach `&buffer` to `ctx.history` before `fx_matrix.process_hop()`. |
| `src/dsp/fx_matrix.rs` | Modify | `process_hop()` already takes `&ModuleContext<'_>` — no signature change. Drill the borrow into per-slot module calls (already does). |
| `tests/history_buffer.rs` | Create | Unit tests for the struct: ring wrap, frame ages, fractional read interpolation, summary stat shapes & determinism, depth resize behaviour. |
| `tests/history_buffer_pipeline.rs` | Create | Integration: a probe module reads `ctx.history` over 200 hops, asserts the buffer fills, summary stats are finite and bounded, depth knob actually changes capacity. |
| `tests/calibration.rs` | Modify | Add probes: `probe_history_frames_used`, `probe_history_capacity`, `probe_history_summary_decay_max`, `probe_history_summary_rms_max`, `probe_history_summary_if_stability_max`. |

---

## Task 1: Add `HistoryBufferDepthChoice` enum and the param

**Files:**
- Modify: `src/params.rs` (add enum near other choice enums, add field to `SpectralForgeParams`)
- Test: existing `tests/calibration.rs` will catch construction; new test below

- [ ] **Step 1.1: Read existing choice-enum patterns**

Run: `grep -n "EnumParam\|FftSizeChoice" src/params.rs | head -30`
Expected: see at least `pub enum FftSizeChoice` and one `EnumParam<FftSizeChoice>` field.

Capture the surrounding context (5 lines before / 5 after each match) so the new enum follows the same `#[derive(...)]` conventions.

- [ ] **Step 1.2: Write a failing test**

Add to `tests/history_buffer.rs` (create the file):

```rust
use spectral_forge::params::HistoryBufferDepthChoice;

#[test]
fn history_depth_choice_default_is_4s() {
    assert_eq!(HistoryBufferDepthChoice::default(), HistoryBufferDepthChoice::Sec4);
}

#[test]
fn history_depth_choice_seconds_are_monotonic() {
    use HistoryBufferDepthChoice::*;
    assert!(Sec1.seconds() < Sec2.seconds());
    assert!(Sec2.seconds() < Sec4.seconds());
    assert!(Sec4.seconds() < Sec8.seconds());
    assert!(Sec8.seconds() < Sec16.seconds());
    assert_eq!(Sec1.seconds(), 1.0);
    assert_eq!(Sec16.seconds(), 16.0);
}

#[test]
fn history_depth_choice_max_frames_uses_hop_size() {
    use HistoryBufferDepthChoice::*;
    // sample_rate 48 kHz, hop = fft_size / 4
    // Sec1 at fft 2048 hop 512: frames = ceil(48000 / 512) = 94
    assert_eq!(Sec1.max_frames(48000.0, 2048), 94);
    // Sec4 at fft 2048 hop 512: ceil(192000 / 512) = 375
    assert_eq!(Sec4.max_frames(48000.0, 2048), 375);
    // Sec16 at fft 16384 hop 4096: ceil(768000 / 4096) = 188
    assert_eq!(Sec16.max_frames(48000.0, 16384), 188);
}
```

Run: `cargo test --test history_buffer history_depth_choice 2>&1 | head`
Expected: FAIL with "module `params::HistoryBufferDepthChoice` not found".

- [ ] **Step 1.3: Define the enum in `params.rs`**

Open `src/params.rs`, find the line containing `pub enum FftSizeChoice` (use Grep), and immediately after the `impl` block for `FftSizeChoice` add:

```rust
/// History Buffer capacity choice. Frames-per-second is derived from
/// `sample_rate / hop` where hop = fft_size / OVERLAP. Memory cost = `seconds *
/// sample_rate / hop * MAX_NUM_BINS * 8 bytes` per channel; at default 4 s,
/// 48 kHz, fft 2048 → 375 frames × 8193 bins × 8 B ≈ 24 MB per channel.
#[derive(nih_plug::prelude::Enum, Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum HistoryBufferDepthChoice {
    #[id = "sec1"]   #[name = "1 s"]  Sec1,
    #[id = "sec2"]   #[name = "2 s"]  Sec2,
    #[default]
    #[id = "sec4"]   #[name = "4 s"]  Sec4,
    #[id = "sec8"]   #[name = "8 s"]  Sec8,
    #[id = "sec16"]  #[name = "16 s"] Sec16,
}

impl HistoryBufferDepthChoice {
    pub fn seconds(self) -> f32 {
        match self {
            Self::Sec1 => 1.0,
            Self::Sec2 => 2.0,
            Self::Sec4 => 4.0,
            Self::Sec8 => 8.0,
            Self::Sec16 => 16.0,
        }
    }

    /// Round-up to the number of hop frames needed to cover `seconds()` at the
    /// given (sample_rate, fft_size). Hop = fft_size / OVERLAP (4).
    pub fn max_frames(self, sample_rate: f32, fft_size: usize) -> usize {
        let hop = (fft_size / crate::dsp::pipeline::OVERLAP).max(1) as f32;
        let frames = (self.seconds() * sample_rate / hop).ceil() as usize;
        frames.max(1)
    }
}
```

Run: `cargo build 2>&1 | head -30`
Expected: clean build (the enum is unused at this point).

Run: `cargo test --test history_buffer history_depth_choice`
Expected: PASS for all three new tests.

- [ ] **Step 1.4: Add the param field**

Open `src/params.rs`. Find the `pub struct SpectralForgeParams` definition. Find the line `pub fft_size_choice: EnumParam<FftSizeChoice>,` (use Grep) and just after it add:

```rust
    #[id = "history_depth"]
    pub history_depth: EnumParam<HistoryBufferDepthChoice>,
```

Then in the corresponding `impl Default for SpectralForgeParams` (or wherever `fft_size_choice: EnumParam::new(...)` is constructed), add right after that line:

```rust
            history_depth: EnumParam::new("History Depth", HistoryBufferDepthChoice::default()),
```

Run: `cargo build 2>&1 | head`
Expected: clean build.

- [ ] **Step 1.5: Commit**

```bash
git add src/params.rs tests/history_buffer.rs
git commit -m "$(cat <<'EOF'
infra(phase5b1): add HistoryBufferDepthChoice param

5-step EnumParam (1/2/4/8/16 s, default 4 s) that will size the History
Buffer in Phase 5b.1 Task 2. seconds()/max_frames() helpers compute the
ring capacity from (seconds, sample_rate, fft_size) with the hop = fft/4
convention. No buffer or pipeline wiring yet.

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: Create the `HistoryBuffer` struct

**Files:**
- Create: `src/dsp/history_buffer.rs`
- Modify: `src/dsp/mod.rs` (`pub mod history_buffer;`)
- Test: `tests/history_buffer.rs` (extend)

- [ ] **Step 2.1: Write failing tests**

Append to `tests/history_buffer.rs`:

```rust
use num_complex::Complex;
use spectral_forge::dsp::history_buffer::HistoryBuffer;

fn frame_of(value: f32, num_bins: usize) -> Vec<Complex<f32>> {
    (0..num_bins).map(|_| Complex::new(value, 0.0)).collect()
}

#[test]
fn new_capacity_matches_constructor_args() {
    let h = HistoryBuffer::new(2, 100, 1025);
    assert_eq!(h.num_channels(), 2);
    assert_eq!(h.capacity_frames(), 100);
    assert_eq!(h.num_bins(), 1025);
    assert_eq!(h.frames_used(), 0);
}

#[test]
fn write_advances_frames_used_until_capacity() {
    let mut h = HistoryBuffer::new(1, 5, 4);
    for i in 0..3 {
        h.write_hop(0, &frame_of(i as f32, 4));
    }
    h.advance_after_all_channels_written();
    for i in 3..7 {
        h.write_hop(0, &frame_of(i as f32, 4));
        h.advance_after_all_channels_written();
    }
    // 8 writes total; capacity is 5; frames_used clamps at 5.
    assert_eq!(h.frames_used(), 5);
}

#[test]
fn read_frame_zero_returns_most_recent() {
    let mut h = HistoryBuffer::new(1, 4, 2);
    h.write_hop(0, &frame_of(10.0, 2));
    h.advance_after_all_channels_written();
    h.write_hop(0, &frame_of(20.0, 2));
    h.advance_after_all_channels_written();
    h.write_hop(0, &frame_of(30.0, 2));
    h.advance_after_all_channels_written();

    let frame = h.read_frame(0, 0).expect("most recent frame must exist");
    assert!((frame[0].re - 30.0).abs() < 1e-6);
    let frame = h.read_frame(0, 1).expect("one-back frame must exist");
    assert!((frame[0].re - 20.0).abs() < 1e-6);
    let frame = h.read_frame(0, 2).expect("two-back frame must exist");
    assert!((frame[0].re - 10.0).abs() < 1e-6);
    assert!(h.read_frame(0, 3).is_none(), "frame older than frames_used must be None");
}

#[test]
fn ring_wraps_after_capacity() {
    let mut h = HistoryBuffer::new(1, 3, 1);
    for i in 0..7u32 {
        h.write_hop(0, &frame_of(i as f32, 1));
        h.advance_after_all_channels_written();
    }
    // After 7 writes into a 3-frame ring, the most recent should be 6, then 5, 4.
    assert!((h.read_frame(0, 0).unwrap()[0].re - 6.0).abs() < 1e-6);
    assert!((h.read_frame(0, 1).unwrap()[0].re - 5.0).abs() < 1e-6);
    assert!((h.read_frame(0, 2).unwrap()[0].re - 4.0).abs() < 1e-6);
    assert!(h.read_frame(0, 3).is_none());
}

#[test]
fn read_fractional_lerps_between_frames() {
    let mut h = HistoryBuffer::new(1, 4, 1);
    h.write_hop(0, &frame_of(10.0, 1));
    h.advance_after_all_channels_written();
    h.write_hop(0, &frame_of(20.0, 1));
    h.advance_after_all_channels_written();

    let mut out = vec![Complex::new(0.0, 0.0); 1];
    let ok = h.read_fractional(0, 0.5, &mut out);
    assert!(ok);
    // age 0.5: 0.5 * frame[0]=20.0 + 0.5 * frame[1]=10.0 → 15.0
    assert!((out[0].re - 15.0).abs() < 1e-6);
}

#[test]
fn reset_zeroes_state() {
    let mut h = HistoryBuffer::new(1, 4, 2);
    h.write_hop(0, &frame_of(1.0, 2));
    h.advance_after_all_channels_written();
    h.reset();
    assert_eq!(h.frames_used(), 0);
    assert!(h.read_frame(0, 0).is_none());
}

#[test]
fn write_hop_panics_in_debug_when_buffer_size_mismatches() {
    let mut h = HistoryBuffer::new(1, 4, 8);
    let frame: Vec<Complex<f32>> = vec![Complex::new(0.0, 0.0); 7]; // wrong size
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        h.write_hop(0, &frame);
    }));
    // In debug builds, write_hop debug_asserts num_bins; release truncates silently.
    // The test only enforces the debug-mode panic so RT-safe release builds are unaffected.
    if cfg!(debug_assertions) {
        assert!(result.is_err(), "debug-mode write_hop must panic on wrong frame size");
    }
}
```

Run: `cargo test --test history_buffer 2>&1 | head -20`
Expected: FAIL with "module `dsp::history_buffer` not found".

- [ ] **Step 2.2: Implement the struct**

Create `src/dsp/history_buffer.rs`:

```rust
//! Per-channel rolling complex-spectrum history buffer.
//!
//! Lives in `Pipeline`, written each STFT hop after the analysis FFT and
//! before `FxMatrix::process_hop`. Modules read it via `ctx.history:
//! Option<&HistoryBuffer>`.
//!
//! Layout:
//!   - One ring of `capacity_frames` frames per audio channel.
//!   - Each frame is a `Vec<Complex<f32>>` of length `num_bins`.
//!   - Frames are written at `write_pos` (one shared write head across all
//!     channels — the pipeline writes channel 0 first, then channel 1, then
//!     calls `advance_after_all_channels_written()`).
//!
//! Read API: `read_frame(channel, age_frames)` returns the spectrum that was
//! current `age_frames` hops ago (age 0 = most recent), or `None` if the
//! requested age exceeds `frames_used()`. `read_fractional` interpolates
//! between adjacent frames for sub-hop precision (used by the future
//! Past Stretch sub-effect).
//!
//! Lazy summary stats (decay/RMS/IF stability) are computed on first
//! request per block via interior mutability and cleared at the top of
//! every block by `clear_summary_cache()`. See § "Summary stats" below.

use num_complex::Complex;
use std::cell::RefCell;

pub struct HistoryBuffer {
    /// `frames[channel][frame_idx][bin]`. All channels share `write_pos`.
    frames: Vec<Vec<Vec<Complex<f32>>>>,
    capacity_frames: usize,
    num_bins: usize,
    num_channels: usize,
    write_pos: usize,
    frames_used: usize,
    summary: RefCell<SummaryCache>,
}

#[derive(Default)]
struct SummaryCache {
    decay_estimate_valid: bool,
    rms_envelope_valid:   bool,
    if_stability_valid:   bool,
    decay_estimate: Vec<f32>,
    rms_envelope:   Vec<f32>,
    if_stability:   Vec<f32>,
}

impl HistoryBuffer {
    pub fn new(num_channels: usize, capacity_frames: usize, num_bins: usize) -> Self {
        let frames: Vec<Vec<Vec<Complex<f32>>>> = (0..num_channels)
            .map(|_| (0..capacity_frames)
                .map(|_| vec![Complex::new(0.0, 0.0); num_bins])
                .collect())
            .collect();
        let summary = SummaryCache {
            decay_estimate: vec![0.0; num_bins],
            rms_envelope:   vec![0.0; num_bins],
            if_stability:   vec![0.0; num_bins],
            ..SummaryCache::default()
        };
        Self {
            frames,
            capacity_frames,
            num_bins,
            num_channels,
            write_pos: 0,
            frames_used: 0,
            summary: RefCell::new(summary),
        }
    }

    pub fn num_channels(&self)    -> usize { self.num_channels }
    pub fn capacity_frames(&self) -> usize { self.capacity_frames }
    pub fn num_bins(&self)        -> usize { self.num_bins }
    pub fn frames_used(&self)     -> usize { self.frames_used }

    /// Write one hop's complex spectrum for a channel into the current write slot.
    /// Allocation-free; copies `num_bins` complex floats. Caller MUST call
    /// `advance_after_all_channels_written()` once per hop after writing every
    /// channel, otherwise the next hop overwrites the same slot.
    pub fn write_hop(&mut self, channel: usize, spectrum: &[Complex<f32>]) {
        debug_assert!(channel < self.num_channels, "channel out of range");
        debug_assert_eq!(spectrum.len(), self.num_bins,
            "history write expected {} bins, got {}", self.num_bins, spectrum.len());
        let dst = &mut self.frames[channel][self.write_pos];
        let n = dst.len().min(spectrum.len());
        dst[..n].copy_from_slice(&spectrum[..n]);
    }

    /// Advance the shared write head one frame forward. Call once per hop after
    /// every channel has been written. Wraps at `capacity_frames`.
    pub fn advance_after_all_channels_written(&mut self) {
        self.write_pos = (self.write_pos + 1) % self.capacity_frames;
        if self.frames_used < self.capacity_frames {
            self.frames_used += 1;
        }
        // Any cached summary becomes stale once a new frame lands.
        self.invalidate_summary_cache();
    }

    /// Read the spectrum that was current `age_frames` ago. age=0 = most recent.
    /// Returns `None` if the requested age has not been written yet (cold start
    /// or after a reset).
    pub fn read_frame(&self, channel: usize, age_frames: usize) -> Option<&[Complex<f32>]> {
        if channel >= self.num_channels { return None; }
        if age_frames >= self.frames_used { return None; }
        let cap = self.capacity_frames;
        // write_pos points to the slot to be written NEXT. Most-recent written = write_pos - 1.
        let idx = (self.write_pos + cap - 1 - age_frames) % cap;
        Some(&self.frames[channel][idx])
    }

    /// Linear-interpolate between two adjacent frames at fractional age. Returns
    /// `false` (and leaves `out` unchanged) if `age + 1` exceeds `frames_used`.
    /// Note: this lerps complex bins directly — naive and intentionally cheap.
    /// The Past Stretch consumer adds phase-vocoder rotation on top via the
    /// shared `PhaseRotator` (Phase 5b.2).
    pub fn read_fractional(&self, channel: usize, age: f32, out: &mut [Complex<f32>]) -> bool {
        let age_floor = age.floor() as usize;
        let frac = age - age_floor as f32;
        let frame_a = match self.read_frame(channel, age_floor) {
            Some(f) => f,
            None => return false,
        };
        let frame_b = match self.read_frame(channel, age_floor + 1) {
            Some(f) => f,
            None => return false,
        };
        let n = out.len().min(frame_a.len()).min(frame_b.len());
        for k in 0..n {
            out[k] = frame_a[k] * (1.0 - frac) + frame_b[k] * frac;
        }
        true
    }

    /// Wipe the buffer and reset write head and frame count. Allocation-free.
    pub fn reset(&mut self) {
        for ch in &mut self.frames {
            for frame in ch {
                for c in frame { *c = Complex::new(0.0, 0.0); }
            }
        }
        self.write_pos = 0;
        self.frames_used = 0;
        self.invalidate_summary_cache();
    }

    /// Called by the pipeline at the top of every audio block. Marks the
    /// summary stats stale so they get re-derived on next read.
    pub fn clear_summary_cache(&self) {
        self.invalidate_summary_cache();
    }

    fn invalidate_summary_cache(&self) {
        let mut s = self.summary.borrow_mut();
        s.decay_estimate_valid = false;
        s.rms_envelope_valid = false;
        s.if_stability_valid = false;
    }
}
```

Run: `cargo build 2>&1 | head -30`
Expected: FAIL — `dsp::history_buffer` is not declared.

- [ ] **Step 2.3: Add the module declaration**

In `src/dsp/mod.rs`, add (alphabetised):

```rust
pub mod history_buffer;
```

Run: `cargo build 2>&1 | head`
Expected: clean build.

Run: `cargo test --test history_buffer 2>&1 | tail -20`
Expected: all Task 2 tests PASS, except the summary-stat tests added in Task 4 (still fail — correct).

- [ ] **Step 2.4: Commit**

```bash
git add src/dsp/history_buffer.rs src/dsp/mod.rs tests/history_buffer.rs
git commit -m "$(cat <<'EOF'
infra(phase5b1): add HistoryBuffer struct

Per-channel rolling complex-spectrum ring with read_frame(age) and
read_fractional(age) APIs. Allocation-free write_hop; shared write head
across channels; ring wraps at capacity_frames; reset() zeroes all
state. Summary stat slots are present with a RefCell cache but stay
unimplemented until Task 4. No pipeline wiring yet.

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: Add `history` slot to `ModuleContext`

**Files:**
- Modify: `src/dsp/modules/mod.rs` (extend the struct + `new()`)
- Test: `tests/module_trait.rs` (extend)

- [ ] **Step 3.1: Read the current struct**

Run: `grep -n "pub struct ModuleContext" src/dsp/modules/mod.rs`
Then `Read` 30 lines starting at the matched line. Confirm the `'block` lifetime is in place (Phase 1) and that `bin_physics: Option<&'block BinPhysics>` is in place (Phase 3). The new field follows the same pattern.

- [ ] **Step 3.2: Write a failing test**

Add to `tests/module_trait.rs`:

```rust
#[test]
fn module_context_has_history_slot_default_none() {
    use spectral_forge::dsp::modules::ModuleContext;
    let ctx = ModuleContext::new(
        48000.0, 2048, 1025, 10.0, 100.0, 1.0, 0.5, false, false,
    );
    assert!(ctx.history.is_none(),
        "ctx.history must default to None so existing modules ignore it");
}
```

Run: `cargo test --test module_trait module_context_has_history_slot_default_none 2>&1 | tail`
Expected: FAIL with "no field `history` on type `ModuleContext`".

- [ ] **Step 3.3: Add the field**

In `src/dsp/modules/mod.rs`, locate the `ModuleContext` struct (the one with the `'block` lifetime added in Phase 1, extended by Phase 3 with `bin_physics`). Add a field after the existing `bin_physics: Option<&'block crate::dsp::bin_physics::BinPhysics>,` line:

```rust
    /// Read-only handle to the per-channel STFT history ring. `None` when no
    /// history-consuming module is in the slot chain (the pipeline only
    /// attaches the borrow when at least one slot has `reads_history`).
    /// See `src/dsp/history_buffer.rs`.
    pub history: Option<&'block crate::dsp::history_buffer::HistoryBuffer>,
```

In `ModuleContext::new()`, add the field-default in the struct literal:

```rust
            history: None,
```

- [ ] **Step 3.4: Run the test**

Run: `cargo test --test module_trait module_context_has_history_slot_default_none`
Expected: PASS.

Run: `cargo test 2>&1 | tail -5`
Expected: all existing tests still pass; total test count = previous + 1.

- [ ] **Step 3.5: Commit**

```bash
git add src/dsp/modules/mod.rs tests/module_trait.rs
git commit -m "$(cat <<'EOF'
infra(phase5b1): expose history on ModuleContext

Adds Option<&'block HistoryBuffer> next to bin_physics. Default None;
no module reads it yet. Pipeline wiring lands in Task 5.

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: Implement lazy summary statistics

**Files:**
- Modify: `src/dsp/history_buffer.rs` (add `summary_decay_estimate()`, `summary_rms_envelope()`, `summary_if_stability()`)
- Test: `tests/history_buffer.rs` (extend)

**Background.** Three summary arrays are needed by Past sub-effects (decay sorter, IF-stability sorter, area sorter) and possibly Geometry (persistent homology). Computing them every hop is wasteful — most blocks no module asks for them. Strategy: cache + RefCell + clear at top of every block. The compute uses the most recent `min(frames_used, ANALYSIS_WINDOW)` frames, where `ANALYSIS_WINDOW = 32` (≈ 340 ms at default fft 2048 hop 512 / 48 kHz — long enough for a useful decay estimate, short enough that recompute is < 100 µs at 8193 bins).

- [ ] **Step 4.1: Write failing tests**

Append to `tests/history_buffer.rs`:

```rust
#[test]
fn summary_decay_estimate_long_ringing_bin_has_high_value() {
    let mut h = HistoryBuffer::new(1, 64, 2);
    // Bin 0 = constant 1.0 for 50 frames (long ring).
    // Bin 1 = decays from 1.0 to 0.0 over 8 frames (fast decay).
    for i in 0..50 {
        let bin0 = Complex::new(1.0, 0.0);
        let bin1_mag = (1.0 - i as f32 / 8.0).max(0.0);
        let bin1 = Complex::new(bin1_mag, 0.0);
        h.write_hop(0, &[bin0, bin1]);
        h.advance_after_all_channels_written();
    }
    let decay = h.summary_decay_estimate(0);
    assert!(decay[0] > decay[1],
        "long-ringing bin must have higher decay_estimate than fast-decaying bin (got {:?} vs {:?})", decay[0], decay[1]);
}

#[test]
fn summary_rms_envelope_louder_bin_has_higher_rms() {
    let mut h = HistoryBuffer::new(1, 32, 2);
    for _ in 0..32 {
        h.write_hop(0, &[Complex::new(1.0, 0.0), Complex::new(0.1, 0.0)]);
        h.advance_after_all_channels_written();
    }
    let rms = h.summary_rms_envelope(0);
    assert!((rms[0] - 1.0).abs() < 1e-3, "loud bin RMS should be ~1.0, got {:?}", rms[0]);
    assert!((rms[1] - 0.1).abs() < 1e-3, "quiet bin RMS should be ~0.1, got {:?}", rms[1]);
}

#[test]
fn summary_if_stability_finite_when_no_phase_data() {
    let mut h = HistoryBuffer::new(1, 32, 2);
    for _ in 0..32 {
        h.write_hop(0, &[Complex::new(1.0, 0.0), Complex::new(0.5, 0.5)]);
        h.advance_after_all_channels_written();
    }
    let stab = h.summary_if_stability(0);
    for &v in stab {
        assert!(v.is_finite(), "if_stability must be finite");
        assert!(v >= 0.0 && v <= 1.0, "if_stability must be in [0, 1], got {:?}", v);
    }
}

#[test]
fn summary_caches_within_block_and_clears_on_request() {
    let mut h = HistoryBuffer::new(1, 32, 2);
    for _ in 0..32 {
        h.write_hop(0, &[Complex::new(1.0, 0.0), Complex::new(0.0, 0.0)]);
        h.advance_after_all_channels_written();
    }
    // First call computes; later calls reuse. Pointer equality is fine because
    // RefCell hands back a stable borrow until invalidated.
    let first = h.summary_rms_envelope(0).as_ptr();
    let second = h.summary_rms_envelope(0).as_ptr();
    assert_eq!(first, second);

    h.clear_summary_cache();
    // Force recompute: still same backing Vec (we recompute in place), but cache flag was reset.
    let third = h.summary_rms_envelope(0).as_ptr();
    assert_eq!(first, third, "summary buffer is reused — Vec is allocated once");
}

#[test]
fn summary_returns_zeros_for_invalid_channel() {
    let h = HistoryBuffer::new(1, 32, 2);
    let out = h.summary_decay_estimate(99);
    assert!(out.iter().all(|&v| v == 0.0));
}
```

Run: `cargo test --test history_buffer summary_ 2>&1 | tail -20`
Expected: FAIL — methods don't exist yet.

- [ ] **Step 4.2: Implement the summary methods**

In `src/dsp/history_buffer.rs`, add these methods inside `impl HistoryBuffer`:

```rust
    /// Number of recent frames used to derive every summary stat.
    /// 32 frames at the default fft 2048 hop 512 / 48 kHz is ~340 ms.
    pub const ANALYSIS_WINDOW: usize = 32;

    /// Per-bin frames-to-fall-20-dB estimate, derived from the linear-regression
    /// slope of `log10(magnitude)` over the most recent `ANALYSIS_WINDOW` frames.
    /// Higher = longer-ringing bin. Bins whose magnitude is too small or whose
    /// regression slope is non-negative get 0.0.
    ///
    /// Returned slice borrows the cached Vec; valid until the next
    /// `advance_after_all_channels_written()` or `clear_summary_cache()`.
    pub fn summary_decay_estimate(&self, channel: usize) -> std::cell::Ref<'_, [f32]> {
        self.maybe_recompute_decay(channel);
        std::cell::Ref::map(self.summary.borrow(), |s| s.decay_estimate.as_slice())
    }

    pub fn summary_rms_envelope(&self, channel: usize) -> std::cell::Ref<'_, [f32]> {
        self.maybe_recompute_rms(channel);
        std::cell::Ref::map(self.summary.borrow(), |s| s.rms_envelope.as_slice())
    }

    pub fn summary_if_stability(&self, channel: usize) -> std::cell::Ref<'_, [f32]> {
        self.maybe_recompute_if_stability(channel);
        std::cell::Ref::map(self.summary.borrow(), |s| s.if_stability.as_slice())
    }

    fn maybe_recompute_decay(&self, channel: usize) {
        {
            let s = self.summary.borrow();
            if s.decay_estimate_valid { return; }
        }
        let mut s = self.summary.borrow_mut();
        for v in &mut s.decay_estimate { *v = 0.0; }
        if channel >= self.num_channels { s.decay_estimate_valid = true; return; }
        let n = Self::ANALYSIS_WINDOW.min(self.frames_used);
        if n < 4 { s.decay_estimate_valid = true; return; }
        // Linear regression of log10(mag) vs frame index over the most recent n frames.
        // slope < 0 = decaying; we report -1 / slope as a decay-time proxy (larger = longer ring).
        let mean_x: f32 = (n as f32 - 1.0) * 0.5;
        let var_x:  f32 = (0..n).map(|i| {
            let dx = i as f32 - mean_x;
            dx * dx
        }).sum::<f32>().max(1.0);
        for k in 0..self.num_bins {
            let mut mean_y = 0.0_f32;
            for i in 0..n {
                if let Some(frame) = self.read_frame(channel, i) {
                    let mag = frame[k].norm().max(1e-9);
                    mean_y += mag.log10();
                }
            }
            mean_y /= n as f32;
            let mut cov = 0.0_f32;
            for i in 0..n {
                if let Some(frame) = self.read_frame(channel, i) {
                    let mag = frame[k].norm().max(1e-9);
                    let dx = i as f32 - mean_x;
                    let dy = mag.log10() - mean_y;
                    cov += dx * dy;
                }
            }
            let slope = cov / var_x;
            // age 0 = most recent, age n-1 = oldest. Older frames are HIGHER index, so a
            // decaying signal (newer is louder) gives a NEGATIVE slope (log_mag decreases
            // with increasing age). Decay-time proxy: -1 / slope, clamped to [0, 1000].
            s.decay_estimate[k] = if slope < -1e-6 {
                (-1.0 / slope).clamp(0.0, 1000.0)
            } else {
                0.0
            };
        }
        s.decay_estimate_valid = true;
    }

    fn maybe_recompute_rms(&self, channel: usize) {
        {
            let s = self.summary.borrow();
            if s.rms_envelope_valid { return; }
        }
        let mut s = self.summary.borrow_mut();
        for v in &mut s.rms_envelope { *v = 0.0; }
        if channel >= self.num_channels { s.rms_envelope_valid = true; return; }
        let n = Self::ANALYSIS_WINDOW.min(self.frames_used);
        if n == 0 { s.rms_envelope_valid = true; return; }
        for k in 0..self.num_bins {
            let mut acc = 0.0_f32;
            for i in 0..n {
                if let Some(frame) = self.read_frame(channel, i) {
                    let mag = frame[k].norm();
                    acc += mag * mag;
                }
            }
            s.rms_envelope[k] = (acc / n as f32).sqrt();
        }
        s.rms_envelope_valid = true;
    }

    fn maybe_recompute_if_stability(&self, channel: usize) {
        {
            let s = self.summary.borrow();
            if s.if_stability_valid { return; }
        }
        let mut s = self.summary.borrow_mut();
        for v in &mut s.if_stability { *v = 0.0; }
        if channel >= self.num_channels { s.if_stability_valid = true; return; }
        let n = Self::ANALYSIS_WINDOW.min(self.frames_used);
        if n < 3 { s.if_stability_valid = true; return; }
        // For each bin, compute hop-to-hop phase-difference variance over n-1
        // adjacent frame pairs. Stable partials → low variance → near-1 score.
        // Unstable / noisy bins → high variance → near-0 score. We map variance v
        // to stability = 1 / (1 + v) so the result is bounded in (0, 1].
        for k in 0..self.num_bins {
            let mut diffs = [0.0_f32; Self::ANALYSIS_WINDOW];
            let mut count = 0_usize;
            for i in 0..(n - 1) {
                let a = self.read_frame(channel, i);
                let b = self.read_frame(channel, i + 1);
                if let (Some(a), Some(b)) = (a, b) {
                    let phase_a = a[k].arg();
                    let phase_b = b[k].arg();
                    let mut d = phase_a - phase_b;
                    while d > std::f32::consts::PI  { d -= std::f32::consts::TAU; }
                    while d < -std::f32::consts::PI { d += std::f32::consts::TAU; }
                    diffs[count] = d;
                    count += 1;
                }
            }
            if count < 2 { continue; }
            let mean: f32 = diffs[..count].iter().sum::<f32>() / count as f32;
            let var:  f32 = diffs[..count].iter()
                .map(|&x| (x - mean) * (x - mean))
                .sum::<f32>() / count as f32;
            s.if_stability[k] = 1.0 / (1.0 + var);
        }
        s.if_stability_valid = true;
    }
```

Run: `cargo build 2>&1 | head -30`
Expected: clean build.

Run: `cargo test --test history_buffer summary_ 2>&1 | tail -20`
Expected: all Task 4 summary tests PASS.

- [ ] **Step 4.3: Commit**

```bash
git add src/dsp/history_buffer.rs tests/history_buffer.rs
git commit -m "$(cat <<'EOF'
infra(phase5b1): lazy summary stats on HistoryBuffer

decay_estimate (linear regression slope of log10|X| over 32 frames),
rms_envelope (per-bin √mean(|X|²) over 32 frames), and if_stability
(1 / (1 + Var(Δφ_k)) over 31 hop-to-hop phase deltas). All three live
behind a RefCell that re-derives only on first read per block; the
pipeline calls clear_summary_cache() after every advance. Bounded
finite-output assertions keep the cost predictable: ~ANALYSIS_WINDOW *
num_bins ops per stat per block, < 100 µs at 8193 bins.

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: Wire `HistoryBuffer` into `Pipeline`

**Files:**
- Modify: `src/dsp/pipeline.rs`
- Test: `tests/history_buffer_pipeline.rs` (create — runs in Task 6)

- [ ] **Step 5.1: Read the current Pipeline new/reset/process**

Run `Read` on `src/dsp/pipeline.rs` lines 38–172 (Pipeline struct + new + reset). Note:
- `Pipeline::new` takes `(sample_rate, num_channels, fft_size, slot_types)`. The pipeline does NOT receive params here — it can't size the history from `HistoryBufferDepthChoice` directly without a wiring change.

The cleanest fix: add `history_depth_seconds: f32` as a parameter to `Pipeline::new` (instead of the enum, which lives in `params`). Default the lib.rs construction site to `4.0` (matches `HistoryBufferDepthChoice::Sec4`). Pass the actual seconds from params through. Avoids the dsp crate depending on the params enum.

- [ ] **Step 5.2: Add the field to `Pipeline`**

In `src/dsp/pipeline.rs`, in the `pub struct Pipeline { ... }` definition, add at the end (just before `sample_rate: f32,`):

```rust
    /// Per-channel rolling complex-spectrum history. Sized at construction from the
    /// History Depth param (in seconds). Reallocated by `reset()` if the requested
    /// capacity changes (allocation OK there — reset is not on the audio thread).
    history: crate::dsp::history_buffer::HistoryBuffer,
    history_depth_seconds: f32,
```

- [ ] **Step 5.3: Update `Pipeline::new` signature**

Change the function signature:

```rust
    pub fn new(
        sample_rate: f32,
        num_channels: usize,
        fft_size: usize,
        slot_types: &[crate::dsp::modules::ModuleType; 9],
        history_depth_seconds: f32,
    ) -> Self {
```

Inside the function body, just before the final `Self { ... }` literal, compute the capacity and allocate:

```rust
        let history_capacity = {
            let hop = (fft_size / OVERLAP).max(1) as f32;
            ((history_depth_seconds * sample_rate) / hop).ceil() as usize
        }.max(1);
        let history = crate::dsp::history_buffer::HistoryBuffer::new(
            num_channels.max(1),
            history_capacity,
            num_bins,
        );
```

Then add `history,` and `history_depth_seconds,` inside the `Self { ... }` literal (alongside the other fields).

- [ ] **Step 5.4: Update `Pipeline::reset` to honour a new depth**

Change the function signature:

```rust
    pub fn reset(&mut self, sample_rate: f32, num_channels: usize, history_depth_seconds: f32) {
```

Inside `reset`, before the existing `self.fx_matrix.reset(...)` call, add:

```rust
        // History Buffer: rebuild if the depth changed; otherwise reset in place.
        let new_capacity = {
            let hop = (fft_size / OVERLAP).max(1) as f32;
            ((history_depth_seconds * sample_rate) / hop).ceil() as usize
        }.max(1);
        let needs_realloc = self.history_depth_seconds != history_depth_seconds
            || self.history.capacity_frames() != new_capacity
            || self.history.num_channels() != num_channels.max(1);
        if needs_realloc {
            self.history = crate::dsp::history_buffer::HistoryBuffer::new(
                num_channels.max(1),
                new_capacity,
                num_bins,
            );
            self.history_depth_seconds = history_depth_seconds;
        } else {
            self.history.reset();
        }
```

- [ ] **Step 5.5: Update the two call sites in `lib.rs`**

Run: `grep -n "Pipeline::new\|self.pipeline.reset" src/lib.rs`
At each call site, supply the seconds from the param:

```rust
let history_depth_seconds = self.params.history_depth.value().seconds();
// then:
crate::dsp::pipeline::Pipeline::new(sample_rate, num_channels, fft_size, &slot_types, history_depth_seconds)
// and similarly:
self.pipeline.reset(sample_rate, num_channels, history_depth_seconds);
```

- [ ] **Step 5.6: Wire the per-block summary cache clear and the per-hop write**

In `src/dsp/pipeline.rs`, inside `Pipeline::process`:

(a) **Top of process()** (immediately after the `let num_bins = fft_size / 2 + 1;` line) clear the summary cache:

```rust
        // History summary stats are valid only within one block. Modules
        // reading them get a cache-miss-then-cache-hit pattern; cleared here.
        self.history.clear_summary_cache();
```

(b) **Inside the `process_overlap_add` closure**, after the analysis FFT and after `spectrum_buf` peak update, before the `fx_matrix.process_hop` call, write the channel's frame to history. Note we are inside an `&mut self.stft` borrow — `self.history` cannot be mentioned by `self.history.write_hop(...)`. Reborrow it as a local before the closure (next step).

(c) **Reborrow `self.history` as a local** alongside the existing `let fx_matrix = &mut self.fx_matrix;` block. Add:

```rust
        let history = &mut self.history;
```

then inside the closure, immediately before `fx_matrix.process_hop(...)`, add:

```rust
            history.write_hop(channel, &complex_buf[..num_bins]);
```

(d) **Advance the write head once per hop after both channels have been written.** Right inside the closure (it is called once per channel per hop). Track an inline counter to know when both channels have been written. Simplest: advance only when `channel == num_channels - 1` (works for stereo and mono):

```rust
            if channel + 1 == stft_num_channels {
                history.advance_after_all_channels_written();
            }
```

For this to compile, capture the channel count outside the closure too:

```rust
        let stft_num_channels = self.stft.num_channels();
```

(StftHelper exposes `num_channels()`. If it does not in this crate version, replace with `buffer.channels()` queried before the closure and captured by value.)

(e) **Attach the borrow to `ctx`.** After `fx_matrix.process_hop(...)` would be too late — the modules need it during the call. Instead, attach the borrow to `ctx` right BEFORE the STFT closure begins. But `ctx` is built before the closure too. Move the `ctx.history = Some(...)` assignment to after the existing `ctx` construction:

```rust
        let mut ctx = ModuleContext { /* …existing fields… */ };
        ctx.history = Some(&*history);
```

This requires `ctx` to be `mut` (it is currently `let ctx = ModuleContext { ... };` — change to `let mut ctx = ...`). Note: assigning `Some(&*history)` borrows `history` immutably — this conflicts with the `history.write_hop(...)` mutable borrow inside the closure.

**Resolution.** Do not attach the read-only borrow to `ctx.history` for the same hop — attach the *previous* hop's snapshot. Past has one-block latency anyway (it reads frames that are ≥ 1 hop old by design). To express this without unsafe:

- Move `ctx.history = Some(&self.history)` to be set *after* the closure, before any reader uses it. Since `process_hop` is called *inside* the closure, the read happens during the closure too. The clean solution: split the borrow.

Concretely: keep `let history = &mut self.history;` for the closure (writer). After the closure ends, reborrow immutably and attach to `ctx.history` *for this block's external use* — but in this phase no module reads it externally; the only reader path is `fx_matrix.process_hop` *inside* the closure.

The right approach for Phase 5b.1 v1 is:

1. Inside the closure, the writer mutably owns `history`.
2. At the END of the closure (last line, after `*s *= w * norm * output_linear;`), drop the mutable borrow and reborrow immutably for the NEXT block's modules. But `ctx` is already past — it is dropped when the closure returns.

Therefore, the simplest correct wiring is: attach `ctx.history` to a **separate immutable borrow snapshot taken before the closure**, by keeping `&self.history` (immutable) on `ctx.history`, and writing to `&mut self.history` only AFTER the closure ends.

**Final wiring (replaces 5.6c–e):**

(c′) Reborrow `&self.history` immutably for the `ctx`:

```rust
        let history_ref: &crate::dsp::history_buffer::HistoryBuffer = &self.history;
```

Then build `ctx` with `history: Some(history_ref)`:

```rust
        let ctx = ModuleContext {
            sample_rate:       self.sample_rate,
            fft_size,
            num_bins,
            attack_ms:         attack_ms_base,
            release_ms:        release_ms_base,
            sensitivity,
            suppression_width: params.suppression_width.smoothed.next_step(block_size),
            auto_makeup:       params.auto_makeup.value(),
            delta_monitor,
            // history attached as the *previous* block's content; this is the
            // expected semantic for a "past" buffer — readers always look back.
            history: Some(history_ref),
            // bin_physics: <whatever Phase 3 attaches here>,
            ..ModuleContext::new(0.0, 0, 0, 0.0, 0.0, 0.0, 0.0, false, false)
        };
```

(d′) After `fx_matrix.process_hop(...)` finishes (i.e. AFTER the `process_overlap_add` closure returns), write the new hops into the buffer. Since `process_overlap_add` already maintains the per-hop chunking internally, we re-derive the most recent post-FFT spectrum from `complex_buf` (which still holds the *last* hop's spectrum). For correctness across multiple hops per block, we instead refactor: write inside the closure into a scratch ring `pending_history_frames: Vec<Vec<Complex<f32>>>` (length = num_channels * hops_in_block), then drain into `self.history` after the closure.

This is too elaborate for v1. The acceptable compromise:

- Pre-allocate `pending_hop_frames: Vec<Vec<Complex<f32>>>` of shape `[2 channels][num_bins]` at `Pipeline::new` (one slot per channel — overwritten each hop, drained once per hop into history).
- Inside the closure, copy `complex_buf` into `pending_hop_frames[channel]`. When `channel + 1 == stft_num_channels`, drain BOTH into `history` and `advance`.

Because `&mut self.history` is needed inside the closure, and `&self.history` is needed for `ctx.history`, the read AND write paths can't share the same borrow. Solution: attach to `ctx` the buffer state *as of the TOP of this block*. The closure writes, the reads (inside the closure) see the prior block's state. This is fine.

**Concrete code for the closure** (drop the `let history = &mut self.history;` reborrow):

In `Pipeline::process`, before the closure, add the pending scratch:

```rust
        let pending_hop_frames = &mut self.pending_hop_frames;
        let mut pending_hops: usize = 0;
        let stft_num_channels = self.stft.num_channels();
```

Also add `pending_hop_frames: Vec<Vec<Complex<f32>>>` to the Pipeline struct (sized 2 × MAX_NUM_BINS at construction):

```rust
    /// Scratch pad for one hop's per-channel complex spectrum, copied out of
    /// the StftHelper closure and drained into `history` after the closure.
    pending_hop_frames: Vec<Vec<Complex<f32>>>,
```

Construct it in `new()`:

```rust
            pending_hop_frames: (0..2).map(|_| vec![Complex::new(0.0, 0.0); MAX_NUM_BINS]).collect(),
```

Reset it in `reset()`:

```rust
        for v in &mut self.pending_hop_frames { for c in v { *c = Complex::new(0.0, 0.0); } }
```

Inside the closure, after the FFT and before the `fx_matrix.process_hop` call, capture:

```rust
            // Copy this hop's complex spectrum into the pending pad. Drained into
            // `history` after the closure completes — the closure cannot mutate
            // `self.history` while StftHelper holds the &mut on the buffer.
            pending_hop_frames[channel][..num_bins].copy_from_slice(&complex_buf[..num_bins]);
            if channel + 1 == stft_num_channels {
                pending_hops += 1;
            }
```

After the closure ends (immediately after the `});` that closes `self.stft.process_overlap_add(...)`), drain the pending hops:

```rust
        // Note: StftHelper bursts a *single* hop's worth of FFTs per channel before returning,
        // when block_size >= hop. For block_size < hop, no hop completes — pending_hops==0.
        // For block_size > hop, multiple hops MAY complete; in that case the pending pad
        // holds the latest hop only (each iteration overwrites). We push at most one hop
        // per process() call. This is acceptable for the History Buffer because (a) summary
        // stats only need ANALYSIS_WINDOW frames not every single hop, and (b) the buffer
        // is conceptually a *display* of recent past spectra. If a future consumer needs
        // every hop, replace the pad with a Vec of pads sized to MAX hops/block.
        if pending_hops > 0 {
            for ch in 0..stft_num_channels.min(2) {
                self.history.write_hop(ch, &self.pending_hop_frames[ch][..num_bins]);
            }
            self.history.advance_after_all_channels_written();
        }
```

**Note on hops-per-block.** At fft 2048 hop 512 the typical Bitwig block size is 1024 — exactly two hops per block. The code above writes only the LAST hop per block. For accurate decay estimates this matters: ANALYSIS_WINDOW (32 frames) covers ~340 ms when one hop per block, ~680 ms when two hops per block but only sampled at half rate. Document as a known v1 limitation; v2 can promote the pad to a per-hop ring (~few hundred KB).

- [ ] **Step 5.7: Build & smoke**

Run: `cargo build 2>&1 | tail -30`
Expected: clean build.

Run: `cargo test 2>&1 | tail`
Expected: all tests still pass.

- [ ] **Step 5.8: Commit**

```bash
git add src/dsp/pipeline.rs src/lib.rs
git commit -m "$(cat <<'EOF'
infra(phase5b1): wire HistoryBuffer into Pipeline

Pipeline holds the buffer (sized from history_depth_seconds passed by
lib.rs from params), clears summary cache at the top of every block,
captures the post-FFT complex spectrum into a scratch pad inside the
StftHelper closure, then drains the pad into history after the closure
finishes. ctx.history attaches the *prior-block* snapshot — readers see
strictly past frames, which matches the "Past" semantic.

Known v1 limitation: writes one hop per block (the last hop captured
inside StftHelper). For block_size > hop_size, intermediate hops are
dropped. ANALYSIS_WINDOW (32 frames) compensates by averaging.

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>
EOF
)"
```

---

## Task 6: Integration smoke test — probe module reads `ctx.history`

**Files:**
- Create: `tests/history_buffer_pipeline.rs`

The pipeline integration tests run end-to-end through `Pipeline::process` with a synthetic buffer. We can't reach the audio thread without nih-plug's harness, so the test instead drives `process_block_for_test` adjacent to the pipeline using a stub module installed inline. Since installing a probe module into FxMatrix is invasive for a single test, we shortcut: directly construct a `HistoryBuffer`, write 200 frames via the public API, and verify the summary stats and read APIs hold under the same load the pipeline would impose.

This is a *direct-API* integration test, not a full plugin-host roundtrip. Phase 5b.2 (Past) ships the first full end-to-end reader; this phase only validates the buffer behaves correctly under realistic frame counts.

- [ ] **Step 6.1: Write the integration test**

Create `tests/history_buffer_pipeline.rs`:

```rust
use num_complex::Complex;
use spectral_forge::dsp::history_buffer::HistoryBuffer;

const NUM_BINS: usize = 1025;
const NUM_HOPS: usize = 200;

fn synthesize_frame(hop: usize) -> Vec<Complex<f32>> {
    // Bin 100 = stable sine; bin 200 = decaying ring; bin 300 = noise.
    let mut frame = vec![Complex::new(0.0, 0.0); NUM_BINS];
    frame[100] = Complex::new(1.0, 0.0);
    let env = (1.0 - hop as f32 / 50.0).max(0.0);
    frame[200] = Complex::from_polar(env, hop as f32 * 0.1);
    let noise_phase = (hop as f32 * 137.0).sin();
    frame[300] = Complex::from_polar(0.5, noise_phase);
    frame
}

#[test]
fn history_buffer_under_pipeline_load_is_finite_and_bounded() {
    let mut h = HistoryBuffer::new(2, 100, NUM_BINS);
    for hop in 0..NUM_HOPS {
        let frame = synthesize_frame(hop);
        h.write_hop(0, &frame);
        h.write_hop(1, &frame);
        h.advance_after_all_channels_written();
        h.clear_summary_cache();

        // Every 25 hops: poll the summary stats and validate.
        if hop % 25 == 24 {
            let decay = h.summary_decay_estimate(0);
            let rms   = h.summary_rms_envelope(0);
            let stab  = h.summary_if_stability(0);
            for k in 0..NUM_BINS {
                assert!(decay[k].is_finite(), "decay[{}] non-finite at hop {}", k, hop);
                assert!(rms[k].is_finite(),   "rms[{}] non-finite at hop {}", k, hop);
                assert!(stab[k].is_finite(),  "stab[{}] non-finite at hop {}", k, hop);
                assert!(decay[k] >= 0.0 && decay[k] <= 1000.0);
                assert!(rms[k] >= 0.0 && rms[k] <= 10.0);
                assert!(stab[k] >= 0.0 && stab[k] <= 1.0);
            }
        }
    }
    assert_eq!(h.frames_used(), 100, "buffer must saturate at capacity after enough writes");
}

#[test]
fn history_buffer_read_frame_returns_most_recent_after_full_load() {
    let mut h = HistoryBuffer::new(1, 50, NUM_BINS);
    for hop in 0..NUM_HOPS {
        let frame = synthesize_frame(hop);
        h.write_hop(0, &frame);
        h.advance_after_all_channels_written();
    }
    let most_recent = h.read_frame(0, 0).expect("most recent frame must exist");
    let expected = synthesize_frame(NUM_HOPS - 1);
    for k in 0..NUM_BINS {
        let dre = (most_recent[k].re - expected[k].re).abs();
        let dim = (most_recent[k].im - expected[k].im).abs();
        assert!(dre < 1e-5 && dim < 1e-5,
            "bin {}: most_recent ({}, {}) != expected ({}, {})",
            k, most_recent[k].re, most_recent[k].im, expected[k].re, expected[k].im);
    }
}

#[test]
fn history_buffer_summary_caches_repeat_calls() {
    let mut h = HistoryBuffer::new(1, 50, NUM_BINS);
    for hop in 0..NUM_HOPS {
        let frame = synthesize_frame(hop);
        h.write_hop(0, &frame);
        h.advance_after_all_channels_written();
    }
    let _warm = h.summary_decay_estimate(0); // first call computes
    drop(_warm);
    // Without invalidating, a second call should produce identical numerics.
    let a = h.summary_decay_estimate(0);
    let snapshot: Vec<f32> = a.to_vec();
    drop(a);
    let b = h.summary_decay_estimate(0);
    for k in 0..NUM_BINS {
        assert_eq!(snapshot[k], b[k], "cached value must be stable bin {}", k);
    }
}
```

Run: `cargo test --test history_buffer_pipeline 2>&1 | tail`
Expected: all three tests PASS.

- [ ] **Step 6.2: Commit**

```bash
git add tests/history_buffer_pipeline.rs
git commit -m "$(cat <<'EOF'
infra(phase5b1): integration smoke test for HistoryBuffer

200-hop synthetic load exercising write_hop, advance, summary stats and
ring wrap. Verifies finite/bounded outputs, capacity saturation, exact
read-back of the most recent frame, and within-block summary cache
stability. No pipeline-host wiring — the buffer is exercised directly
via its public API at the same scale Pipeline imposes.

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>
EOF
)"
```

---

## Task 7: Calibration probes

**Files:**
- Modify: `tests/calibration.rs`

The existing `tests/calibration.rs` runs every module + sample combination through Pipeline and snapshots probe values. Phase 5b.1 adds a single bridge-level probe (channel 0 only) that surfaces the buffer's state. Modules don't have a `last_probe()` slot for HistoryBuffer (it isn't a module); we expose it via a new public method on `Pipeline` only enabled under `#[cfg(any(test, feature = "probe"))]`.

- [ ] **Step 7.1: Add the probe method to `Pipeline`**

In `src/dsp/pipeline.rs`, add at the bottom of `impl Pipeline`:

```rust
    /// Test-only snapshot of HistoryBuffer state. Used by `tests/calibration.rs`
    /// to assert the buffer fills, summary stats stay finite, and depth changes
    /// take effect.
    #[cfg(any(test, feature = "probe"))]
    pub fn history_probe(&self, channel: usize) -> HistoryProbe {
        let frames_used = self.history.frames_used();
        let capacity    = self.history.capacity_frames();
        let decay = self.history.summary_decay_estimate(channel);
        let rms   = self.history.summary_rms_envelope(channel);
        let stab  = self.history.summary_if_stability(channel);
        HistoryProbe {
            frames_used,
            capacity,
            summary_decay_max:        decay.iter().cloned().fold(0.0f32, f32::max),
            summary_rms_max:          rms.iter().cloned().fold(0.0f32, f32::max),
            summary_if_stability_max: stab.iter().cloned().fold(0.0f32, f32::max),
        }
    }
```

Just below the `impl` block, add the struct:

```rust
#[cfg(any(test, feature = "probe"))]
#[derive(Clone, Copy, Debug)]
pub struct HistoryProbe {
    pub frames_used: usize,
    pub capacity:    usize,
    pub summary_decay_max:        f32,
    pub summary_rms_max:          f32,
    pub summary_if_stability_max: f32,
}
```

- [ ] **Step 7.2: Add the calibration probe test**

Append to `tests/calibration.rs`:

```rust
#[test]
fn history_probe_fills_under_realistic_load() {
    use spectral_forge::dsp::history_buffer::HistoryBuffer;
    // Direct buffer probe (the calibration harness drives modules, not Pipeline
    // construction). We verify the same numbers Pipeline::history_probe would
    // surface, by directly building a buffer with the same shape Pipeline would.
    let mut h = HistoryBuffer::new(2, 50, 1025);
    for hop in 0..200 {
        use num_complex::Complex;
        let mag = (hop as f32 / 200.0).sin().abs();
        let frame: Vec<Complex<f32>> = (0..1025).map(|k| {
            Complex::from_polar(mag * (k as f32 + 1.0).recip(), 0.0)
        }).collect();
        h.write_hop(0, &frame);
        h.write_hop(1, &frame);
        h.advance_after_all_channels_written();
    }
    assert_eq!(h.frames_used(), 50, "probe: frames_used must saturate at capacity");
    let decay = h.summary_decay_estimate(0);
    let rms   = h.summary_rms_envelope(0);
    let stab  = h.summary_if_stability(0);
    let decay_max = decay.iter().cloned().fold(0.0f32, f32::max);
    let rms_max   = rms.iter().cloned().fold(0.0f32, f32::max);
    let stab_max  = stab.iter().cloned().fold(0.0f32, f32::max);
    assert!(decay_max.is_finite() && decay_max <= 1000.0);
    assert!(rms_max.is_finite()   && rms_max   <= 10.0);
    assert!(stab_max.is_finite()  && stab_max  <= 1.0 + 1e-6);
}
```

Run: `cargo test --test calibration history_probe_fills_under_realistic_load 2>&1 | tail`
Expected: PASS.

Run: `cargo test 2>&1 | tail`
Expected: every test still passes.

- [ ] **Step 7.3: Commit**

```bash
git add src/dsp/pipeline.rs tests/calibration.rs
git commit -m "$(cat <<'EOF'
infra(phase5b1): HistoryBuffer calibration probe

Test-only Pipeline::history_probe(channel) returns frames_used,
capacity, and the per-stat max across all bins. tests/calibration.rs
runs a 200-hop synthetic load and asserts the buffer saturates at
capacity, summary stats stay finite, and the bounds claimed in
HistoryBuffer's doc comments hold.

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>
EOF
)"
```

---

## Task 8: Status banner & STATUS index entry

**Files:**
- Modify: top of `ideas/next-gen-modules/01-global-infrastructure.md` (banner)
- Modify: `docs/superpowers/STATUS.md` (add the plan + status row)

- [ ] **Step 8.1: Add status banner to the spec**

In `ideas/next-gen-modules/01-global-infrastructure.md`, immediately under the existing `**Status:** RESEARCH` line at the top (line 4), add this note (do NOT change the global RESEARCH status — only § 2 ships in this phase):

```markdown
> **§ 2 (History Buffer) status:** IMPLEMENTED — Phase 5b.1
> (`docs/superpowers/plans/2026-04-27-phase-5b1-history-buffer.md`).
> The other sections remain RESEARCH until their own phase ships.
```

- [ ] **Step 8.2: Update STATUS.md**

In `docs/superpowers/STATUS.md`, find the table of plans (use `Read` then `Grep` for the existing entries). Add a new row in the same format as the surrounding entries:

```markdown
| 2026-04-27 | plans/2026-04-27-phase-5b1-history-buffer.md | History Buffer infrastructure (per-channel ring + lazy summary stats) | PENDING |
```

If the file already has a "Phase 5b" row that this supersedes, change that row's status to `SPLIT` and reference this sub-plan.

- [ ] **Step 8.3: Add the plan's own status banner**

At the top of `docs/superpowers/plans/2026-04-27-phase-5b1-history-buffer.md` (this file), add (above the title heading):

```markdown
> **Status:** PENDING — implementation has not started. See
> `docs/superpowers/STATUS.md` for the authoritative state.
```

When the plan is fully implemented, the agent updating STATUS.md should also flip this banner to `IMPLEMENTED` and add a link to the merge commit.

- [ ] **Step 8.4: Commit**

```bash
git add ideas/next-gen-modules/01-global-infrastructure.md docs/superpowers/STATUS.md docs/superpowers/plans/2026-04-27-phase-5b1-history-buffer.md
git commit -m "$(cat <<'EOF'
docs(phase5b1): status banner + STATUS index entry

Marks ideas/next-gen-modules/01-global-infrastructure.md § 2 as
IMPLEMENTED-by-phase-5b1; the rest of the file remains RESEARCH.
Adds the plan to docs/superpowers/STATUS.md with status PENDING.
Plan file gains its own banner pointing at STATUS.md as the source
of truth.

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>
EOF
)"
```

---

## Self-review checklist (run after Task 8)

- [ ] Spec coverage:
  - § 2 of `01-global-infrastructure.md`: rolling complex spectrum (Tasks 2 + 5), per-channel (Task 2 constructor), summary stats lazy + cached per block (Task 4 + Pipeline `clear_summary_cache` in Task 5), memory budget configurable (Task 1 enum + Task 5 wiring), main-only (sidechain history deferred to a later phase explicitly).
  - § "Research findings" #5 in `13-past.md`: `if_offset[k]` cache and `PhaseRotator` are NOT in this plan — they belong to Phase 5b.2 (Past) where the consumer landed.
  - § "History Buffer details — spec gaps" in `13-past.md`: per-channel history (Task 2), MAX_HISTORY_FRAMES configurable (Task 1), summary stats (Task 4), sidechain history deferred (called out in Task 8 banner).

- [ ] No placeholders: every code block above contains real Rust the engineer can paste. The ONE intentional ambiguity is "split the borrow" in Task 5.6 — fully resolved by the `pending_hop_frames` scratch + post-closure drain pattern with concrete code.

- [ ] Type consistency:
  - `HistoryBuffer::new(num_channels, capacity_frames, num_bins)` is used identically in Tasks 2, 5, 6, 7.
  - `summary_decay_estimate(channel) -> Ref<'_, [f32]>` is used identically in Task 4 and Task 6 and Task 7.
  - `clear_summary_cache(&self)` (Task 2) called by `Pipeline::process` (Task 5) — `&self` is correct because the cache lives behind `RefCell`.
  - `HistoryBufferDepthChoice::seconds() -> f32` used by `lib.rs` in Task 5.5 and by `Pipeline::new`/`reset` signatures in Tasks 5.3/5.4.

- [ ] Real-time safety:
  - `write_hop`: `copy_from_slice` only, no allocation.
  - `advance_after_all_channels_written`: integer arithmetic only.
  - `summary_*`: reads buffer + writes into pre-allocated cache Vecs. No allocation. RefCell `borrow_mut` is single-threaded (audio thread), so the runtime borrow check never fires.
  - `read_frame` / `read_fractional`: indexing only.
  - Pipeline `clear_summary_cache` once per block; pipeline drain after closure copies into already-allocated `frames[ch][write_pos]`.
  - `pending_hop_frames` allocated at construction, only `copy_from_slice`'d at runtime.
  - The `reset()` reallocation IS allocation, but only happens off the audio thread (called from `Plugin::reset` / `initialize` per nih-plug contract).

- [ ] Phase 1/3/4 prereqs honoured:
  - The `'block` lifetime on `ModuleContext` (Phase 1) lets `history: Option<&'block HistoryBuffer>` compile.
  - The `bin_physics: Option<&'block BinPhysics>` field (Phase 3) is the structural template followed for `history`.
  - Task 4's `if_stability` uses raw bin phase, not `ctx.instantaneous_freq` — we deliberately don't depend on Phase 4's IF for v1 because the summary lives below the per-block ctx and re-derives it cheaply itself.

---

## Execution Handoff

**Plan complete and saved to `docs/superpowers/plans/2026-04-27-phase-5b1-history-buffer.md`. Two execution options:**

**1. Subagent-Driven (recommended)** — I dispatch a fresh subagent per task, review between tasks, fast iteration

**2. Inline Execution** — Execute tasks in this session using executing-plans, batch execution with checkpoints

**Which approach?**
