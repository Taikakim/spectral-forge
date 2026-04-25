> **Status (2026-04-24): IMPLEMENTED.** Single SC port + per-slot gain/channel selector, peak-hold curve, Freeze default threshold fix all merged. Source of truth: the code + [../STATUS.md](../STATUS.md).

# Sidechain Refactor Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Refactor sidechain from 4 aux ports + global `sc_gain` to one stereo SC port with per-module gain and channel-selector controls, add a Gain-module per-bin peak-hold curve with live SC-envelope visualisation, fix Freeze default threshold.

**Architecture:** Additive changes first (new `ModuleSpec` flag, new per-slot params, new bridge channel), then atomic Pipeline refactor swaps old SC path for new, then editor UI changes, then final param removals and test coverage.

**Tech Stack:** Rust, nih-plug, nih-plug-egui, realfft, triple_buffer, parking_lot.

**Spec:** `docs/superpowers/specs/2026-04-21-sidechain-refactor-design.md`

**Working directory:** `/home/kim/Projects/spectral/.worktrees/sidechain-refactor` (branch `feature/sidechain-refactor`).

---

## File Map

### Modified files

- `src/lib.rs` — CLAP aux port count 4 → 1; gui_sidechain_active shape change.
- `src/params.rs` — new `ScChannel` enum; new `slot_sc_gain_db: [f32; 9]` and `slot_sc_channel: [ScChannel; 9]` persisted params; remove `sc_gain`, `slot_sidechain`.
- `src/bridge.rs` — `sidechain_active: [Arc<AtomicBool>; 4]` → `Arc<AtomicBool>`; new `sc_envelope_tx`/`sc_envelope_rx` triple-buffer channel for Gain peak-hold visualisation.
- `src/dsp/pipeline.rs` — collapse `sc_stfts` 4→1; SC channel resolution (LR/L/R/M/S from 2-ch SC); per-slot SC gain application; publish SC envelope to bridge.
- `src/dsp/modules/mod.rs` — `supports_sidechain: bool` on `ModuleSpec`; rename Gain curve `SC SMOOTH` → `PEAK HOLD`; rename PhaseSmear curve `SC SMOOTH` → `PEAK HOLD`; drop Contrast's `SC SMOOTH` curve (num_curves 2 → 1).
- `src/dsp/modules/gain.rs` — add per-bin peak-hold state; implement peak-hold DSP gated to `GainMode::Pull`; publish SC envelope.
- `src/dsp/modules/phase_smear.rs` — wire up SC: peak-hold curve smooths SC magnitude into per-bin smear modulator.
- `src/dsp/modules/freeze.rs` — Freeze threshold default mapping pivot: -20 dB → -50 dB.
- `src/dsp/modules/contrast.rs` — remove any consumption of the dropped `SC SMOOTH` curve (should already be unused; verify).
- `src/editor_ui.rs` — remove `sc_gain` knob from top bar; remove old 4-slot SC indicator; add 4-px yellow SC level meter right of Falloff; remove per-slot SC-assign popup; add per-module SC strip to SC-aware module panels.
- `src/editor/curve.rs` — overlay 1px darker animated line behind drawn peak-hold curve, driven by the new SC-envelope bridge channel.
- `src/editor/theme.rs` — add colour constants for SC yellow bar and SC envelope overlay line.
- `docs/MANUAL.md` — new Sidechain section.

### New test files

- `tests/sidechain.rs` — channel-selector resolution matrix test; Gain peak-hold decay test.

---

## Task 1: Add `supports_sidechain` flag to `ModuleSpec`

**Files:**
- Modify: `src/dsp/modules/mod.rs` (the `ModuleSpec` struct and `module_spec()` fn)
- Test: `tests/module_trait.rs` (add assertion)

- [ ] **Step 1: Write the failing test**

Append to `tests/module_trait.rs`:

```rust
#[test]
fn supports_sidechain_flag_matches_spec() {
    use spectral_forge::dsp::modules::{module_spec, ModuleType};
    assert!(module_spec(ModuleType::Dynamics).supports_sidechain);
    assert!(module_spec(ModuleType::Gain).supports_sidechain);
    assert!(module_spec(ModuleType::PhaseSmear).supports_sidechain);
    assert!(module_spec(ModuleType::Freeze).supports_sidechain);
    assert!(!module_spec(ModuleType::Contrast).supports_sidechain);
    assert!(!module_spec(ModuleType::MidSide).supports_sidechain);
    assert!(!module_spec(ModuleType::TransientSustainedSplit).supports_sidechain);
    assert!(!module_spec(ModuleType::Harmonic).supports_sidechain);
    assert!(!module_spec(ModuleType::Master).supports_sidechain);
    assert!(!module_spec(ModuleType::Empty).supports_sidechain);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test module_trait supports_sidechain_flag_matches_spec`
Expected: FAIL — field `supports_sidechain` does not exist on `ModuleSpec`.

- [ ] **Step 3: Add `supports_sidechain` field**

In `src/dsp/modules/mod.rs`, find the `pub struct ModuleSpec { ... }` definition and add `pub supports_sidechain: bool` at the end. Then, in the `module_spec()` function, update every `static` `ModuleSpec` instance:

- `DYN`, `GN` (Gain), `PSM` (PhaseSmear), `FRZ`: `supports_sidechain: true`
- `CON`, `MS`, `TS`, `HARM`, `MASTER`, `EMPTY`: `supports_sidechain: false`

Every `ModuleSpec { ... }` literal must have the new field to compile.

- [ ] **Step 4: Run test to verify pass + full test suite**

Run: `cargo test --test module_trait supports_sidechain_flag_matches_spec`
Expected: PASS

Run: `cargo test`
Expected: all existing tests still pass.

- [ ] **Step 5: Commit**

```bash
git add src/dsp/modules/mod.rs tests/module_trait.rs
git commit -m "feat(modules): add supports_sidechain flag to ModuleSpec"
```

---

## Task 2: Rename `SC SMOOTH` → `PEAK HOLD` in Gain and PhaseSmear; drop `SC SMOOTH` in Contrast

**Files:**
- Modify: `src/dsp/modules/mod.rs` (curve_labels for GN, PSM, CON; num_curves for CON)
- Modify: `src/dsp/modules/contrast.rs` (`num_curves()` return)

- [ ] **Step 1: Write the failing test**

Append to `tests/module_trait.rs`:

```rust
#[test]
fn curve_labels_post_refactor() {
    use spectral_forge::dsp::modules::{module_spec, ModuleType};
    assert_eq!(module_spec(ModuleType::Gain).curve_labels, &["GAIN", "PEAK HOLD"]);
    assert_eq!(module_spec(ModuleType::PhaseSmear).curve_labels, &["AMOUNT", "PEAK HOLD", "MIX"]);
    assert_eq!(module_spec(ModuleType::Contrast).curve_labels, &["AMOUNT"]);
    assert_eq!(module_spec(ModuleType::Contrast).num_curves, 1);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test module_trait curve_labels_post_refactor`
Expected: FAIL — current labels contain `SC SMOOTH`.

- [ ] **Step 3: Update curve_labels and num_curves**

In `src/dsp/modules/mod.rs`:

- `GN.curve_labels`: `&["GAIN", "SC SMOOTH"]` → `&["GAIN", "PEAK HOLD"]`
- `PSM.curve_labels`: `&["AMOUNT", "SC SMOOTH", "MIX"]` → `&["AMOUNT", "PEAK HOLD", "MIX"]`
- `CON.curve_labels`: `&["AMOUNT", "SC SMOOTH"]` → `&["AMOUNT"]`
- `CON.num_curves`: `2` → `1`

In `src/dsp/modules/contrast.rs`, locate `fn num_curves(&self) -> usize { 2 }` and change to `{ 1 }`.

Verify `contrast.rs`'s `process()` body doesn't read `curves.get(1)` — if it does (consuming the now-removed curve), either delete that code path or rework it. If a removal is needed, do it and add a comment `// SC SMOOTH curve removed in sidechain refactor`.

- [ ] **Step 4: Run tests**

Run: `cargo test`
Expected: all tests pass including the new `curve_labels_post_refactor`. Pay attention to the debug assertion `assert_eq!(m.num_curves(), module_spec(ty).num_curves)` in `create_module()` — this will panic if `ContrastModule::num_curves()` and `CON.num_curves` disagree.

- [ ] **Step 5: Commit**

```bash
git add src/dsp/modules/mod.rs src/dsp/modules/contrast.rs tests/module_trait.rs
git commit -m "refactor(modules): rename SC SMOOTH to PEAK HOLD, drop Contrast SC SMOOTH"
```

---

## Task 3: Add `ScChannel` enum

**Files:**
- Modify: `src/params.rs` (new enum near `StereoLink`)

- [ ] **Step 1: Write the failing test**

Append to `tests/module_trait.rs`:

```rust
#[test]
fn sc_channel_enum_variants() {
    use spectral_forge::params::ScChannel;
    let values = [ScChannel::Follow, ScChannel::LR, ScChannel::L,
                  ScChannel::R, ScChannel::M, ScChannel::S];
    assert_eq!(values.len(), 6);
    assert_eq!(ScChannel::default(), ScChannel::Follow);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test module_trait sc_channel_enum_variants`
Expected: FAIL — `ScChannel` does not exist.

- [ ] **Step 3: Add the enum**

In `src/params.rs`, after the `StereoLink` enum (~line 40), add:

```rust
/// Sidechain channel routing per SC-aware slot.
/// `Follow` resolves against `StereoLink` and `FxChannelTarget` — see docs/superpowers/specs/2026-04-21-sidechain-refactor-design.md §5.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Enum, serde::Serialize, serde::Deserialize)]
pub enum ScChannel {
    #[default]
    Follow,
    LR,
    L,
    R,
    M,
    S,
}
```

- [ ] **Step 4: Run test to verify pass**

Run: `cargo test --test module_trait sc_channel_enum_variants`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/params.rs tests/module_trait.rs
git commit -m "feat(params): add ScChannel enum for per-slot SC routing"
```

---

## Task 4: Add per-slot `slot_sc_gain_db` and `slot_sc_channel` persisted params

**Files:**
- Modify: `src/params.rs` (struct fields + defaults)

- [ ] **Step 1: Write the failing test**

Append to `tests/module_trait.rs`:

```rust
#[test]
fn per_slot_sc_defaults() {
    use spectral_forge::params::{SpectralForgeParams, ScChannel};
    let p = SpectralForgeParams::default();
    let gains = *p.slot_sc_gain_db.lock();
    let chans = *p.slot_sc_channel.lock();
    assert_eq!(gains.len(), 9);
    assert_eq!(chans.len(), 9);
    for g in gains.iter() {
        assert_eq!(*g, 0.0, "default SC gain should be 0 dB");
    }
    for c in chans.iter() {
        assert_eq!(*c, ScChannel::Follow);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test module_trait per_slot_sc_defaults`
Expected: FAIL — fields don't exist.

- [ ] **Step 3: Add fields to `SpectralForgeParams`**

In `src/params.rs`, in the `SpectralForgeParams` struct, add near the other persisted slot params (after `slot_gain_mode`):

```rust
    /// Per-slot SC input gain in dB. Range [-90.0, 18.0]; values <= -90.0 treated as "-∞" (SC disabled for slot).
    #[persist = "slot_sc_gain_db"]
    pub slot_sc_gain_db: Arc<Mutex<[f32; 9]>>,

    /// Per-slot SC channel routing.
    #[persist = "slot_sc_channel"]
    pub slot_sc_channel: Arc<Mutex<[ScChannel; 9]>>,
```

In `impl Default for SpectralForgeParams`, add the initialisers in the `Self { ... }` block near the other slot defaults:

```rust
            slot_sc_gain_db: Arc::new(Mutex::new([0.0f32; 9])),
            slot_sc_channel: Arc::new(Mutex::new([ScChannel::Follow; 9])),
```

(Make sure `ScChannel` is imported or referenced via the in-file path.)

- [ ] **Step 4: Run tests**

Run: `cargo test`
Expected: all tests pass including `per_slot_sc_defaults`.

- [ ] **Step 5: Commit**

```bash
git add src/params.rs tests/module_trait.rs
git commit -m "feat(params): add per-slot slot_sc_gain_db and slot_sc_channel"
```

---

## Task 5: Add SC envelope bridge channel

**Files:**
- Modify: `src/bridge.rs` (new triple-buffer for SC peak-hold envelope per Gain slot)

This publishes the live per-bin SC peak-hold envelope for the currently-editing Gain slot. One buffer (MAX_NUM_BINS `f32`s). Pipeline writes it; curve-editor reads it.

- [ ] **Step 1: Add field and init**

In `src/bridge.rs`, inside `SharedState`:

```rust
    // Audio → GUI: SC peak-hold envelope for the currently-edited Gain slot (per-bin f32 magnitudes).
    pub sc_envelope_tx: TbInput<Vec<f32>>,
    pub sc_envelope_rx: Arc<Mutex<TbOutput<Vec<f32>>>>,
```

In `SharedState::new()`, add near the spectrum/suppression pair:

```rust
        let (sc_envelope_tx, sc_envelope_rx) = TripleBuffer::new(&zero_bins).split();
```

And in the `Self { ... }` block:

```rust
            sc_envelope_tx,
            sc_envelope_rx: Arc::new(Mutex::new(sc_envelope_rx)),
```

- [ ] **Step 2: Verify compilation**

Run: `cargo build`
Expected: build succeeds. No behaviour change yet.

- [ ] **Step 3: Commit**

```bash
git add src/bridge.rs
git commit -m "feat(bridge): add SC envelope triple-buffer channel for GUI"
```

---

## Task 6: Collapse `sidechain_active` from `[4]` to single `Arc<AtomicBool>`

**Files:**
- Modify: `src/bridge.rs` (type change)
- Modify: `src/lib.rs` (propagate type change)
- Modify: `src/dsp/pipeline.rs` (single-element store)
- Modify: `src/editor_ui.rs` (single-element read; still used by SC meter until Task 13 replaces it)

- [ ] **Step 1: Change bridge field**

In `src/bridge.rs`:

```rust
// old:
    pub sidechain_active: [Arc<AtomicBool>; 4],
// new:
    pub sidechain_active: Arc<AtomicBool>,
```

In `SharedState::new()`, change init:

```rust
// old:
            sidechain_active: std::array::from_fn(|_| Arc::new(AtomicBool::new(false))),
// new:
            sidechain_active: Arc::new(AtomicBool::new(false)),
```

- [ ] **Step 2: Update lib.rs**

In `src/lib.rs`:

```rust
// old:
    gui_sidechain_active: Option<[Arc<std::sync::atomic::AtomicBool>; 4]>,
// new:
    gui_sidechain_active: Option<Arc<std::sync::atomic::AtomicBool>>,
```

In `impl Default` for `SpectralForge`:

```rust
// old:
        let gui_sidechain_active = Some(std::array::from_fn::<_, 4, _>(|i| {
            shared.sidechain_active[i].clone()
        }));
// new:
        let gui_sidechain_active = Some(shared.sidechain_active.clone());
```

- [ ] **Step 3: Update pipeline.rs (temporary shim)**

In `src/dsp/pipeline.rs`, locate the 4-slot `sc_active_flags` store (around line 214-216):

```rust
// old:
        for i in 0..4 {
            shared.sidechain_active[i].store(sc_active_flags[i], std::sync::atomic::Ordering::Relaxed);
        }
// new (temporary, replaced in Task 8):
        let any_sc_active = sc_active_flags.iter().any(|&b| b);
        shared.sidechain_active.store(any_sc_active, std::sync::atomic::Ordering::Relaxed);
```

- [ ] **Step 4: Update editor_ui.rs**

In `src/editor_ui.rs`, locate `sidechain_active: Option<[Arc<std::sync::atomic::AtomicBool>; 4]>` (~line 17). Change to `Option<Arc<std::sync::atomic::AtomicBool>>`. Locate the read site (~line 66) — the `let sc_active: [bool; 4] = ...` block — change to a single `bool` read:

```rust
// old:
                    let sc_active: [bool; 4] = match &sidechain_active {
                        Some(arr) => std::array::from_fn(|i| arr[i].load(std::sync::atomic::Ordering::Relaxed)),
                        None => [false; 4],
                    };
// new:
                    let sc_active: bool = sidechain_active
                        .as_ref()
                        .map(|a| a.load(std::sync::atomic::Ordering::Relaxed))
                        .unwrap_or(false);
```

In the same file locate the labels loop (around line 502) that iterates `sc_labels`. Simplify that block to treat SC as single-source: replace multi-button render with a single "SC" indicator. Look for the `sc_labels: &[(&str, u8)] = &[` block and surrounding logic: remove `sc_assign`/`slot_sidechain` reads/writes here (will also be removed when `params.slot_sidechain` is dropped in Task 15). Temporarily hard-code the SC indicator to use the single `sc_active` bool and always-255 (self-detect) as the selected source. This keeps the popup compiling until Task 13 rewrites it properly.

- [ ] **Step 5: Run tests**

Run: `cargo build && cargo test`
Expected: compiles, tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/bridge.rs src/lib.rs src/dsp/pipeline.rs src/editor_ui.rs
git commit -m "refactor(sc): collapse sidechain_active 4-slot to single AtomicBool"
```

---

## Task 7: Drop 3 aux ports from CLAP manifest

**Files:**
- Modify: `src/lib.rs` (AUDIO_IO_LAYOUTS)

- [ ] **Step 1: Edit AUDIO_IO_LAYOUTS**

In `src/lib.rs` around line 71-85, change:

```rust
// old:
    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[
        // Layout 0: stereo with 4 aux sidechain inputs
        AudioIOLayout {
            main_input_channels:  NonZeroU32::new(2),
            main_output_channels: NonZeroU32::new(2),
            aux_input_ports: &[new_nonzero_u32(2), new_nonzero_u32(2), new_nonzero_u32(2), new_nonzero_u32(2)],
            ..AudioIOLayout::const_default()
        },
// new:
    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[
        // Layout 0: stereo with 1 aux stereo sidechain input
        AudioIOLayout {
            main_input_channels:  NonZeroU32::new(2),
            main_output_channels: NonZeroU32::new(2),
            aux_input_ports: &[new_nonzero_u32(2)],
            ..AudioIOLayout::const_default()
        },
```

Leave Layout 1 (no sidechain) unchanged.

- [ ] **Step 2: Run tests and build**

Run: `cargo build && cargo test`
Expected: compiles, tests pass. Host-visible change but plugin still functions.

- [ ] **Step 3: Commit**

```bash
git add src/lib.rs
git commit -m "refactor(clap): drop 3 aux sidechain ports, keep 1"
```

---

## Task 8: Atomic Pipeline SC refactor (single SC STFT + channel routing + per-slot gain)

This is the largest task. It replaces the 4-instance SC STFT / envelope / routing with a single 2-channel SC STFT, derives per-slot SC magnitude slices from the new per-slot params (`slot_sc_gain_db` + `slot_sc_channel`), and publishes the SC envelope to the bridge for visualisation (published every block from the currently editing slot, hooked up in Task 13).

**Files:**
- Modify: `src/dsp/pipeline.rs`

- [ ] **Step 1: Write a channel-resolution test**

Create `tests/sidechain.rs`:

```rust
use spectral_forge::dsp::pipeline::resolve_sc_source;
use spectral_forge::params::{FxChannelTarget, ScChannel, StereoLink};

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum Expected { L, R, LR, M, S }

fn exp(c: ScChannel, link: StereoLink, target: FxChannelTarget, ch: usize) -> Expected {
    match resolve_sc_source(c, link, target, ch) {
        spectral_forge::dsp::pipeline::ScSource::L => Expected::L,
        spectral_forge::dsp::pipeline::ScSource::R => Expected::R,
        spectral_forge::dsp::pipeline::ScSource::LR => Expected::LR,
        spectral_forge::dsp::pipeline::ScSource::M => Expected::M,
        spectral_forge::dsp::pipeline::ScSource::S => Expected::S,
    }
}

#[test]
fn follow_linked_is_lr() {
    assert_eq!(exp(ScChannel::Follow, StereoLink::Linked, FxChannelTarget::All, 0), Expected::LR);
    assert_eq!(exp(ScChannel::Follow, StereoLink::Linked, FxChannelTarget::All, 1), Expected::LR);
}

#[test]
fn follow_independent_pairs_channels() {
    assert_eq!(exp(ScChannel::Follow, StereoLink::Independent, FxChannelTarget::All, 0), Expected::L);
    assert_eq!(exp(ScChannel::Follow, StereoLink::Independent, FxChannelTarget::All, 1), Expected::R);
}

#[test]
fn follow_midside_respects_target() {
    assert_eq!(exp(ScChannel::Follow, StereoLink::MidSide, FxChannelTarget::Mid,  0), Expected::M);
    assert_eq!(exp(ScChannel::Follow, StereoLink::MidSide, FxChannelTarget::Side, 1), Expected::S);
    assert_eq!(exp(ScChannel::Follow, StereoLink::MidSide, FxChannelTarget::All,  0), Expected::LR);
}

#[test]
fn explicit_channels_always_apply_literally() {
    for link in [StereoLink::Linked, StereoLink::Independent, StereoLink::MidSide] {
        for target in [FxChannelTarget::All, FxChannelTarget::Mid, FxChannelTarget::Side] {
            for ch in [0usize, 1] {
                assert_eq!(exp(ScChannel::L, link, target, ch), Expected::L);
                assert_eq!(exp(ScChannel::R, link, target, ch), Expected::R);
                assert_eq!(exp(ScChannel::LR, link, target, ch), Expected::LR);
                assert_eq!(exp(ScChannel::M, link, target, ch), Expected::M);
                assert_eq!(exp(ScChannel::S, link, target, ch), Expected::S);
            }
        }
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test sidechain`
Expected: FAIL — `resolve_sc_source` and `ScSource` don't exist yet.

- [ ] **Step 3: Add `ScSource` enum and `resolve_sc_source` helper to `pipeline.rs`**

At the top of `src/dsp/pipeline.rs`, after existing imports:

```rust
use crate::params::{FxChannelTarget, ScChannel, StereoLink};

/// Which of the four derived SC magnitude streams a slot should key off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScSource { L, R, LR, M, S }

/// Map (user choice, stereo mode, slot target, processing channel) → SC source.
/// See spec §5 for the canonical rule: Follow = "route the SC channel matching whatever this slot processes."
pub fn resolve_sc_source(
    choice: ScChannel,
    link: StereoLink,
    target: FxChannelTarget,
    channel: usize,
) -> ScSource {
    match choice {
        ScChannel::Follow => match link {
            StereoLink::Linked => ScSource::LR,
            StereoLink::Independent => if channel == 0 { ScSource::L } else { ScSource::R },
            StereoLink::MidSide => match target {
                FxChannelTarget::Mid  => ScSource::M,
                FxChannelTarget::Side => ScSource::S,
                FxChannelTarget::All  => ScSource::LR,
            },
        },
        ScChannel::LR => ScSource::LR,
        ScChannel::L  => ScSource::L,
        ScChannel::R  => ScSource::R,
        ScChannel::M  => ScSource::M,
        ScChannel::S  => ScSource::S,
    }
}
```

- [ ] **Step 4: Run channel-resolution test**

Run: `cargo test --test sidechain`
Expected: PASS.

- [ ] **Step 5: Rework Pipeline struct fields for single SC**

In `src/dsp/pipeline.rs`, change the SC-related struct fields:

```rust
// old:
    sc_envelopes: Vec<Vec<f32>>,          // per-aux [4] × bins
    sc_env_states: Vec<Vec<f32>>,         // per-aux [4] × bins
    sc_complex_bufs: Vec<Vec<Complex<f32>>>, // per-aux [4] × bins
    sc_stfts: Vec<StftHelper>,            // per-aux [4]
// new:
    /// Per-bin SC envelope magnitudes, one slice per derived source (L, R, LR, M, S).
    /// Shape: [5 sources][MAX_NUM_BINS]. Index matches ScSource ordinal (L=0, R=1, LR=2, M=3, S=4).
    sc_envelopes:   Vec<Vec<f32>>,
    /// Per-bin one-pole envelope state, per source. Shape matches sc_envelopes.
    sc_env_states:  Vec<Vec<f32>>,
    /// FFT output buffers for the 2-channel SC STFT. Shape: [2 channels][num_bins].
    sc_complex_bufs: Vec<Vec<Complex<f32>>>,
    /// Single 2-channel SC STFT.
    sc_stft: StftHelper,
    /// Per-slot SC magnitude slice used as `sidechain: Option<&[f32]>` into modules.
    /// Derived each block by applying per-slot gain to the resolved SC source.
    /// Shape: [9 slots][MAX_NUM_BINS].
    slot_sc_input: Vec<Vec<f32>>,
```

Update `Pipeline::new()` correspondingly:

```rust
        let sc_envelopes:  Vec<Vec<f32>> = (0..5).map(|_| vec![0.0f32; MAX_NUM_BINS]).collect();
        let sc_env_states: Vec<Vec<f32>> = (0..5).map(|_| vec![0.0f32; MAX_NUM_BINS]).collect();
        let sc_complex_bufs: Vec<Vec<Complex<f32>>> = (0..2)
            .map(|_| vec![Complex::new(0.0f32, 0.0f32); num_bins])
            .collect();
        let sc_stft = StftHelper::new(2, fft_size, 0);
        let slot_sc_input: Vec<Vec<f32>> = (0..9).map(|_| vec![0.0f32; MAX_NUM_BINS]).collect();
```

And in the `Self { ... }` block replace the old 4-tuple of SC fields with the new `sc_envelopes`, `sc_env_states`, `sc_complex_bufs`, `sc_stft`, `slot_sc_input`.

Update `reset()` analogously — `self.sc_stft = StftHelper::new(2, fft_size, 0);`, resize `sc_complex_bufs[0..2]`, zero all envelopes and `slot_sc_input` entries.

- [ ] **Step 6: Rework the SC processing block in `process()`**

Replace the entire `// ── Process up to 4 aux sidechain inputs ──` block (original lines ~164-216) with single-SC processing. Here's the full replacement:

```rust
        // ── Process single stereo sidechain input ──
        let mut sc_active = false;
        {
            let hop = fft_size / OVERLAP;
            let fft_plan = self.fft_plan.clone();
            let window = &self.window;
            let sample_rate = self.sample_rate;
            let sc_attack_ms  = params.sc_attack_ms.smoothed.next_step(block_size);
            let sc_release_ms = params.sc_release_ms.smoothed.next_step(block_size);

            let has_aux = aux.inputs.get(0).map(|a| a.samples() > 0).unwrap_or(false);
            if !has_aux {
                for src in &mut self.sc_envelopes  { for v in src.iter_mut() { *v = 0.0; } }
                for src in &mut self.sc_env_states { for v in src.iter_mut() { *v = 0.0; } }
            } else {
                // Zero the output envelopes (peak-capture accumulators)
                for src in &mut self.sc_envelopes { for v in src.iter_mut() { *v = 0.0; } }

                let sc_complex_bufs = &mut self.sc_complex_bufs;
                let sc_env_states = &mut self.sc_env_states;
                let sc_envelopes  = &mut self.sc_envelopes;

                self.sc_stft.process_overlap_add(&mut aux.inputs[0], OVERLAP, |channel, block| {
                    for (s, &w) in block.iter_mut().zip(window.iter()) {
                        *s *= w;
                    }
                    crate::dsp::guard::sanitize(block);
                    fft_plan.process(block, &mut sc_complex_bufs[channel]).unwrap();

                    // Only run envelope update on the second channel, once we have both L and R in sc_complex_bufs.
                    if channel == 1 {
                        let hops_per_sec = sample_rate / hop as f32;
                        let atk_t = sc_attack_ms.max(0.1) * 0.001 * hops_per_sec;
                        let rel_t = sc_release_ms.max(1.0) * 0.001 * hops_per_sec;
                        let atk_coeff = (-1.0_f32 / atk_t).exp();
                        let rel_coeff = (-1.0_f32 / rel_t).exp();

                        let num = sc_complex_bufs[0].len();
                        const SQRT2_INV: f32 = std::f32::consts::FRAC_1_SQRT_2;
                        for k in 0..num {
                            let l_mag = sc_complex_bufs[0][k].norm();
                            let r_mag = sc_complex_bufs[1][k].norm();
                            // L+R sum magnitude (conservative: average, not complex sum magnitude).
                            let lr_mag = 0.5 * (l_mag + r_mag);
                            // M/S from complex sums/diffs then magnitude.
                            let m_cpx = (sc_complex_bufs[0][k] + sc_complex_bufs[1][k]) * SQRT2_INV;
                            let s_cpx = (sc_complex_bufs[0][k] - sc_complex_bufs[1][k]) * SQRT2_INV;
                            let m_mag = m_cpx.norm();
                            let s_mag = s_cpx.norm();

                            let mags = [l_mag, r_mag, lr_mag, m_mag, s_mag];
                            for (src_idx, &mag) in mags.iter().enumerate() {
                                let state_ref = &mut sc_env_states[src_idx][k];
                                let coeff = if mag > *state_ref { atk_coeff } else { rel_coeff };
                                *state_ref = coeff * *state_ref + (1.0 - coeff) * mag;
                                if *state_ref > sc_envelopes[src_idx][k] {
                                    sc_envelopes[src_idx][k] = *state_ref;
                                }
                            }
                        }
                    }
                });

                sc_active = self.sc_envelopes[2].iter().any(|&v| v > 1e-9); // LR source as activity probe
            }
        }

        shared.sidechain_active.store(sc_active, std::sync::atomic::Ordering::Relaxed);
```

Note the `shared.sidechain_active.store(...)` replaces the old multi-store block lower down — remove that old block (Task 6 inserted a temporary shim; replace it with this line).

- [ ] **Step 7: Build per-slot SC input slices**

Replace the old `sc_args` construction (the block that reads `slot_sidechain` and builds `sc_args: [Option<&[f32]>; 9]`) with new per-slot SC resolution. Since `resolve_sc_source` needs `channel`, and `channel` varies per hop inside the STFT closure, we need to build per-channel `sc_args`. Two choices:

(a) Build 2 sets of `sc_args` (one per channel) before the STFT closure and pick by channel inside.
(b) Build them inside the closure per-hop.

Use (a) — matches the allocation-free rule and is cheaper than per-hop routing.

Insert, replacing the old `slot_sidechain_arr` / `sc_args` block:

```rust
        // ── Build per-slot SC input slices (allocation-free) ──
        // Read per-slot SC gain and channel choice (non-blocking).
        let slot_sc_gain_db_arr: [f32; 9] = params.slot_sc_gain_db.try_lock()
            .map(|g| *g)
            .unwrap_or([0.0f32; 9]);
        let slot_sc_channel_arr: [ScChannel; 9] = params.slot_sc_channel.try_lock()
            .map(|g| *g)
            .unwrap_or([ScChannel::Follow; 9]);

        let slot_types_snap = params.slot_module_types.try_lock()
            .map(|g| *g)
            .unwrap_or([crate::dsp::modules::ModuleType::Empty; 9]);

        // slot_sc_input is pre-allocated; we fill it for each slot, once per channel pairing.
        // For simplicity, fill the buffer assuming the *dominant* channel pairing, and let
        // the STFT closure pick the right source per-channel via the per-channel sc_args arrays.
        //
        // Precompute ScSource per (slot, channel).
        let mut slot_sc_source_ch: [[ScSource; 9]; 2] = [[ScSource::LR; 9]; 2];
        for ch in 0..2usize {
            for s in 0..9usize {
                let ty = slot_types_snap[s];
                let supports = crate::dsp::modules::module_spec(ty).supports_sidechain;
                // For modules that don't support SC, choose any default; sc_args will be None.
                if !supports { continue; }
                slot_sc_source_ch[ch][s] = resolve_sc_source(
                    slot_sc_channel_arr[s],
                    stereo_link,
                    slot_targets_snap[s],
                    ch,
                );
            }
        }

        // Build per-channel sc_args: Option<&[f32]> per slot.
        // `None` if (a) slot module is not SC-aware, (b) slot gain is "-∞" (≤ -90 dB), or (c) SC input silent.
        // For the "SC applied" case, we scale the envelope-source slice into slot_sc_input[s] and reference it.
        //
        // Implementation: iterate slots; for each SC-aware slot with valid gain, copy from the right sc_envelopes
        // source and multiply in-place by the linear gain. Because a slot may use different sources per channel,
        // we build this once for ch=0 into slot_sc_input[s] and replicate (rebuild) inside the STFT closure only
        // if the channel-1 source differs. Simpler: compute separate buffers per channel.

        // Reuse `slot_sc_input` as the ch=0 scratch. For ch=1 we need a second scratch — add fields: actually
        // because modules run per-channel within the STFT closure and each call gets a specific `channel`, the
        // simplest correct solution is to pick the right source directly from sc_envelopes without a per-slot
        // scratch copy, and apply gain inside the closure. BUT: no allocation is already satisfied (sc_envelopes
        // is owned by self). The only catch is the gain multiplication — we cannot mutate sc_envelopes.
        //
        // Solution: pre-allocate `slot_sc_input: [2 channels][9 slots][MAX_NUM_BINS]` in the Pipeline struct;
        // fill in the block below; reference from the STFT closure.
```

Now adjust `slot_sc_input` to be 2-dimensional. Revise the field and its init:

```rust
// in struct:
    /// Per-channel, per-slot SC magnitude slice; slot_sc_input[channel][slot][bin]. Pre-allocated.
    slot_sc_input: Vec<Vec<Vec<f32>>>,

// in new():
        let slot_sc_input: Vec<Vec<Vec<f32>>> = (0..2)
            .map(|_| (0..9).map(|_| vec![0.0f32; MAX_NUM_BINS]).collect())
            .collect();

// in reset(): zero all entries.
        for ch in &mut self.slot_sc_input { for s in ch { s.fill(0.0); } }
```

Then in `process()`, after the `slot_sc_source_ch` is built:

```rust
        // Fill slot_sc_input[ch][s] with gained SC magnitudes, or zero if the slot is inactive.
        for ch in 0..2usize {
            for s in 0..9usize {
                let ty = slot_types_snap[s];
                let supports = crate::dsp::modules::module_spec(ty).supports_sidechain;
                let gain_db = slot_sc_gain_db_arr[s];
                let gain_lin = if gain_db <= -90.0 { 0.0 } else { 10.0f32.powf(gain_db / 20.0) };
                let active_for_slot = supports && gain_lin > 0.0 && sc_active;
                if !active_for_slot {
                    // Zero first num_bins entries so any stale SC data isn't read.
                    for v in self.slot_sc_input[ch][s].iter_mut().take(num_bins) { *v = 0.0; }
                    continue;
                }
                let src_idx = slot_sc_source_ch[ch][s] as usize;
                let src = &self.sc_envelopes[src_idx];
                let dst = &mut self.slot_sc_input[ch][s];
                for k in 0..num_bins { dst[k] = src[k] * gain_lin; }
            }
        }
```

(Note: `ScSource` variants are `L=0, R=1, LR=2, M=3, S=4`. Rely on explicit `as usize` cast — add `#[repr(u8)]` and explicit discriminants to the enum to make this safe:)

Edit the enum definition:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ScSource { L = 0, R = 1, LR = 2, M = 3, S = 4 }
```

- [ ] **Step 8: Pass per-channel sc_args into the STFT closure**

Inside the main STFT closure (`self.stft.process_overlap_add`), just before `fx_matrix.process_hop(...)`, construct per-channel `sc_args`:

```rust
            // Build this hop's per-slot sc_args from the pre-gained slot_sc_input.
            let sc_args: [Option<&[f32]>; 9] = std::array::from_fn(|s| {
                let ty = slot_types_snap[s];
                let supports = crate::dsp::modules::module_spec(ty).supports_sidechain;
                let gain_db = slot_sc_gain_db_arr[s];
                if !supports || gain_db <= -90.0 || !sc_active {
                    None
                } else {
                    Some(&slot_sc_input_ref[channel][s][..num_bins])
                }
            });
```

Where `slot_sc_input_ref` is `&self.slot_sc_input` rebound above the closure (matching the existing rebind pattern for other `&self.*` fields used inside the closure).

Remove the old `sc_args` construction above the closure entirely.

- [ ] **Step 9: Publish SC envelope for the editing Gain slot**

At the end of `process()`, after `suppression_tx.publish()`, add:

```rust
        // Publish SC envelope for the currently-edited slot, if it is an SC-aware Gain slot.
        // The GUI curve-editor reads this to draw the live peak-hold line (Task 11 wires the reader).
        let editing_slot = params.editing_slot.try_lock().map(|g| *g as usize).unwrap_or(0);
        let editing_is_gain = editing_slot < 9 &&
            matches!(slot_types_snap[editing_slot], crate::dsp::modules::ModuleType::Gain);
        if editing_is_gain {
            // Use channel 0's SC input for the editing slot (for display only — both channels agree in Linked mode).
            let src = &self.slot_sc_input[0][editing_slot];
            shared.sc_envelope_tx.input_buffer_mut().copy_from_slice(&src[..MAX_NUM_BINS]);
        } else {
            // No SC-relevant view: publish zeros.
            shared.sc_envelope_tx.input_buffer_mut().fill(0.0);
        }
        shared.sc_envelope_tx.publish();
```

- [ ] **Step 10: Build and test**

Run: `cargo build`
Expected: compiles cleanly.

Run: `cargo test`
Expected: all tests pass, including `tests/sidechain.rs` and existing `module_trait` / `engine_contract` tests.

- [ ] **Step 11: Commit**

```bash
git add src/dsp/pipeline.rs tests/sidechain.rs
git commit -m "refactor(pipeline): single SC STFT + per-slot SC routing & gain"
```

---

## Task 9: Fix Freeze threshold default to -50 dB

**Files:**
- Modify: `src/dsp/modules/freeze.rs` (threshold mapping pivot)

- [ ] **Step 1: Write a failing test**

Append to `tests/module_trait.rs`:

```rust
#[test]
fn freeze_threshold_default_is_minus_50_db() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::{FreezeModule, ModuleContext, SpectralModule};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let mut m = FreezeModule::new();
    m.reset(48000.0, 2048);

    // Feed a pure silent hop; check that the curve gain of 1.0 maps to a threshold lin
    // that corresponds to -50 dBFS (within 0.5 dB).
    let num_bins = 1025usize;
    let mut bins = vec![Complex::new(0.0, 0.0); num_bins];
    let curves: Vec<Vec<f32>> = (0..5).map(|_| vec![1.0f32; num_bins]).collect();
    let curves_ref: Vec<&[f32]> = curves.iter().map(|v| &v[..]).collect();
    let mut supp = vec![0.0f32; num_bins];
    let ctx = ModuleContext {
        sample_rate: 48000.0,
        fft_size: 2048,
        num_bins,
        attack_ms: 10.0, release_ms: 80.0,
        sensitivity: 0.5, suppression_width: 0.0,
        auto_makeup: false, delta_monitor: false,
    };
    // Process once to capture initial frame.
    m.process(0, StereoLink::Linked, FxChannelTarget::All,
              &mut bins, None, &curves_ref, &mut supp, &ctx);

    // Now craft a bin with magnitude exactly at linear_to_db(-50) * norm_factor.
    // norm_factor = fft_size / 4 = 512.
    let norm_factor = 2048.0f32 / 4.0;
    let thr_lin_expected_minus_50 = 10.0f32.powf(-50.0 / 20.0) * norm_factor;
    // With curve=1.0 → threshold should be -50 dB. Feed a bin *just above* and one *just below*
    // and ensure only the above-threshold one triggers accumulation.
    // (This is a behavioural sanity check on the new mapping.)
    let just_below = thr_lin_expected_minus_50 * 0.9;
    let just_above = thr_lin_expected_minus_50 * 1.1;
    // Per-bin [k=100]: just_above; per-bin [k=200]: just_below.
    bins[100] = Complex::new(just_above, 0.0);
    bins[200] = Complex::new(just_below, 0.0);
    m.process(0, StereoLink::Linked, FxChannelTarget::All,
              &mut bins, None, &curves_ref, &mut supp, &ctx);
    // The test intent: threshold mapping pivots at -50 dB when curve=1.0. Direct state inspection
    // would be fragile; instead we assert the mapping formula holds by calling a pub-for-test helper.
    assert!(
        spectral_forge::dsp::modules::freeze::curve_to_threshold_db(1.0).abs() < 51.0
            && spectral_forge::dsp::modules::freeze::curve_to_threshold_db(1.0).abs() > 49.0,
        "curve=1.0 must map to -50 dB ±1 dB, got {}",
        spectral_forge::dsp::modules::freeze::curve_to_threshold_db(1.0),
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test module_trait freeze_threshold_default_is_minus_50_db`
Expected: FAIL — `curve_to_threshold_db` doesn't exist.

- [ ] **Step 3: Add helper + change mapping pivot**

In `src/dsp/modules/freeze.rs`, above the `impl SpectralModule for FreezeModule` block, add:

```rust
/// Map a per-bin threshold curve gain (linear, 1.0 = neutral) to dBFS threshold.
/// Default pivot: curve=1.0 → -50 dB (spec §7).
pub fn curve_to_threshold_db(curve_gain: f32) -> f32 {
    use crate::dsp::utils::linear_to_db;
    let thr_db = linear_to_db(curve_gain);
    (-50.0 + thr_db * (60.0 / 18.0)).clamp(-80.0, 0.0)
}
```

Inside `process()`, replace:

```rust
// old:
            let thr_gain      = curves.get(1).and_then(|c| c.get(k)).copied().unwrap_or(1.0);
            let thr_db        = linear_to_db(thr_gain);
            let threshold_db  = (-20.0 + thr_db * (60.0 / 18.0)).clamp(-80.0, 0.0);
// new:
            let thr_gain      = curves.get(1).and_then(|c| c.get(k)).copied().unwrap_or(1.0);
            let threshold_db  = curve_to_threshold_db(thr_gain);
```

Remove the `use crate::dsp::utils::linear_to_db;` line at the top of the `process()` fn if it becomes unused (only if unused).

Make sure `freeze` module is re-exported so `tests/module_trait.rs` can see `curve_to_threshold_db`. In `src/dsp/modules/mod.rs`, verify `pub mod freeze;` exists. Update the test's path to match actual re-export: `spectral_forge::dsp::modules::freeze::curve_to_threshold_db`.

- [ ] **Step 4: Run test**

Run: `cargo test --test module_trait freeze_threshold_default_is_minus_50_db`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/dsp/modules/freeze.rs tests/module_trait.rs
git commit -m "fix(freeze): default threshold at curve=1.0 now -50 dB, not -20 dB"
```

---

## Task 10: Gain module peak-hold DSP (Pull mode only)

**Files:**
- Modify: `src/dsp/modules/gain.rs` (per-bin peak-hold state + Pull-mode branch using curve 1)

- [ ] **Step 1: Write a failing test**

Append to `tests/module_trait.rs`:

```rust
#[test]
fn gain_pull_peak_hold_decays_with_curve() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::{GainModule, GainMode, ModuleContext, SpectralModule};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let mut m = GainModule::new();
    m.set_gain_mode(GainMode::Pull);
    m.reset(48000.0, 2048);

    let num_bins = 1025usize;
    let mut bins = vec![Complex::new(1.0, 0.0); num_bins]; // all-1.0 signal
    let gain_curve   = vec![0.5f32; num_bins]; // 0.5 = partial pull toward SC
    // PEAK HOLD curve: unity (1.0) = some "neutral" hold time.
    let peak_curve   = vec![1.0f32; num_bins];
    let curves_vec: Vec<Vec<f32>> = vec![gain_curve, peak_curve];
    let curves_ref: Vec<&[f32]> = curves_vec.iter().map(|v| &v[..]).collect();

    let sc_impulse: Vec<f32> = (0..num_bins)
        .map(|k| if k == 100 { 5.0 } else { 0.0 })
        .collect();

    let mut supp = vec![0.0f32; num_bins];
    let ctx = ModuleContext {
        sample_rate: 48000.0, fft_size: 2048, num_bins,
        attack_ms: 10.0, release_ms: 80.0,
        sensitivity: 0.5, suppression_width: 0.0,
        auto_makeup: false, delta_monitor: false,
    };
    // Hop 1: impulse present.
    m.process(0, StereoLink::Linked, FxChannelTarget::All,
              &mut bins, Some(&sc_impulse), &curves_ref, &mut supp, &ctx);
    let env_after_hop1 = m.peak_env_at(100);
    assert!(env_after_hop1 > 4.0, "peak-hold envelope should capture impulse magnitude, got {}", env_after_hop1);

    // Hops 2-20: silent SC. Envelope should decay but not instantly.
    let sc_silent = vec![0.0f32; num_bins];
    for _ in 0..20 {
        let mut b = vec![Complex::new(1.0, 0.0); num_bins];
        m.process(0, StereoLink::Linked, FxChannelTarget::All,
                  &mut b, Some(&sc_silent), &curves_ref, &mut supp, &ctx);
    }
    let env_after_decay = m.peak_env_at(100);
    assert!(env_after_decay < env_after_hop1,
            "peak-hold envelope should decay over time, before={} after={}",
            env_after_hop1, env_after_decay);
    assert!(env_after_decay >= 0.0);
}

#[test]
fn gain_add_mode_does_not_use_peak_hold() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::{GainModule, GainMode, ModuleContext, SpectralModule};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let mut m = GainModule::new();
    m.set_gain_mode(GainMode::Add);
    m.reset(48000.0, 2048);

    let num_bins = 1025usize;
    let mut bins = vec![Complex::new(1.0, 0.0); num_bins];
    let gain_curve = vec![1.0f32; num_bins];
    let peak_curve = vec![1.0f32; num_bins];
    let curves_vec: Vec<Vec<f32>> = vec![gain_curve, peak_curve];
    let curves_ref: Vec<&[f32]> = curves_vec.iter().map(|v| &v[..]).collect();
    let sc = vec![0.5f32; num_bins];
    let mut supp = vec![0.0f32; num_bins];
    let ctx = ModuleContext {
        sample_rate: 48000.0, fft_size: 2048, num_bins,
        attack_ms: 10.0, release_ms: 80.0,
        sensitivity: 0.5, suppression_width: 0.0,
        auto_makeup: false, delta_monitor: false,
    };

    m.process(0, StereoLink::Linked, FxChannelTarget::All,
              &mut bins, Some(&sc), &curves_ref, &mut supp, &ctx);

    // In Add mode peak-hold state must not be updated.
    for k in 0..num_bins {
        assert_eq!(m.peak_env_at(k), 0.0, "Add mode must not touch peak-hold state at k={}", k);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test module_trait gain_pull_peak_hold_decays_with_curve`
Expected: FAIL — `peak_env_at` doesn't exist.

- [ ] **Step 3: Implement peak-hold state and DSP**

Rewrite `src/dsp/modules/gain.rs` with per-bin peak-hold:

```rust
use num_complex::Complex;
use crate::params::{FxChannelTarget, StereoLink};
use super::{GainMode, ModuleContext, ModuleType, SpectralModule};
use crate::dsp::pipeline::{MAX_NUM_BINS, OVERLAP};

pub struct GainModule {
    pub(crate) mode: GainMode,
    /// Per-bin peak-hold envelope state. Only written in Pull mode.
    peak_env: Vec<f32>,
    sample_rate: f32,
    fft_size: usize,
}

impl GainModule {
    pub fn new() -> Self {
        Self {
            mode: GainMode::Add,
            peak_env: vec![0.0f32; MAX_NUM_BINS],
            sample_rate: 44100.0,
            fft_size: 2048,
        }
    }

    /// Pub-for-test inspector: current peak-hold envelope at bin `k`.
    #[doc(hidden)]
    pub fn peak_env_at(&self, k: usize) -> f32 {
        self.peak_env.get(k).copied().unwrap_or(0.0)
    }

    /// Map PEAK HOLD curve gain (linear, 1.0 = neutral) to hold time in ms.
    /// Log-scaled, range [1.0, 500.0] ms; default (curve=1.0) chosen so that the
    /// familiar "moderate pumping tamed" default is ~50 ms.
    #[inline]
    fn curve_to_hold_ms(curve: f32) -> f32 {
        // Map curve 0.0 → 1 ms, 1.0 → 50 ms, 2.0 → 500 ms (log).
        let c = curve.clamp(0.0, 2.0);
        // Interpolate in log-time.
        let log_min = 1.0f32.ln();    // 0
        let log_mid = 50.0f32.ln();
        let log_max = 500.0f32.ln();
        let log_t = if c <= 1.0 {
            log_min + (log_mid - log_min) * c
        } else {
            log_mid + (log_max - log_mid) * (c - 1.0)
        };
        log_t.exp()
    }
}

impl Default for GainModule {
    fn default() -> Self { Self::new() }
}

impl SpectralModule for GainModule {
    fn reset(&mut self, sample_rate: f32, fft_size: usize) {
        self.sample_rate = sample_rate;
        self.fft_size = fft_size;
        for v in &mut self.peak_env { *v = 0.0; }
    }

    fn process(
        &mut self,
        _channel: usize,
        _stereo_link: StereoLink,
        _target: FxChannelTarget,
        bins: &mut [Complex<f32>],
        sidechain: Option<&[f32]>,
        curves: &[&[f32]],
        suppression_out: &mut [f32],
        _ctx: &ModuleContext,
    ) {
        let n = bins.len();
        match self.mode {
            GainMode::Add => {
                for k in 0..n {
                    let g  = curves.get(0).and_then(|c| c.get(k)).copied().unwrap_or(1.0);
                    let sc = sidechain.and_then(|s| s.get(k)).copied().unwrap_or(0.0).max(0.0);
                    bins[k] *= g + sc;
                }
            }
            GainMode::Subtract => {
                for k in 0..n {
                    let g  = curves.get(0).and_then(|c| c.get(k)).copied().unwrap_or(1.0);
                    let sc = sidechain.and_then(|s| s.get(k)).copied().unwrap_or(0.0).max(0.0);
                    bins[k] *= (g - sc).max(0.0);
                }
            }
            GainMode::Pull => {
                let hop_ms = self.fft_size as f32 / (OVERLAP as f32 * self.sample_rate) * 1000.0;
                for k in 0..n {
                    let g = curves.get(0).and_then(|c| c.get(k)).copied()
                            .unwrap_or(1.0).clamp(0.0, 1.0);
                    let sc_mag_raw = sidechain.and_then(|s| s.get(k)).copied().unwrap_or(0.0).max(0.0);

                    // Update per-bin peak-hold envelope.
                    let hold_curve = curves.get(1).and_then(|c| c.get(k)).copied().unwrap_or(1.0);
                    let hold_ms = Self::curve_to_hold_ms(hold_curve);
                    // Instant attack; release uses one-pole with τ = hold_ms.
                    let release_coeff = (-hop_ms / hold_ms.max(0.1)).exp();
                    if sc_mag_raw > self.peak_env[k] {
                        self.peak_env[k] = sc_mag_raw;
                    } else {
                        self.peak_env[k] = release_coeff * self.peak_env[k]
                            + (1.0 - release_coeff) * sc_mag_raw;
                    }
                    let sc_eff = self.peak_env[k];

                    let cur_mag = bins[k].norm();
                    if cur_mag > 1e-10 {
                        let target_mag = cur_mag * g + sc_eff * (1.0 - g);
                        bins[k] *= target_mag / cur_mag;
                    }
                }
            }
        }
        suppression_out.fill(0.0);
    }

    fn set_gain_mode(&mut self, mode: GainMode) { self.mode = mode; }

    fn module_type(&self) -> ModuleType { ModuleType::Gain }
    fn num_curves(&self) -> usize { 2 }
}
```

Note the import of `MAX_NUM_BINS` and `OVERLAP` from `crate::dsp::pipeline`. Verify `OVERLAP` and `MAX_NUM_BINS` are `pub` in `pipeline.rs` — they are (constants shown in file).

- [ ] **Step 4: Run tests**

Run: `cargo test`
Expected: `gain_pull_peak_hold_decays_with_curve` and `gain_add_mode_does_not_use_peak_hold` pass, along with all previous tests.

- [ ] **Step 5: Commit**

```bash
git add src/dsp/modules/gain.rs tests/module_trait.rs
git commit -m "feat(gain): per-bin peak-hold envelope in Pull mode via PEAK HOLD curve"
```

---

## Task 11: PhaseSmear SC-follow DSP via PEAK HOLD curve

**Files:**
- Modify: `src/dsp/modules/phase_smear.rs`

- [ ] **Step 1: Write a failing test**

Append to `tests/module_trait.rs`:

```rust
#[test]
fn phase_smear_sc_modulates_amount() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::{PhaseSmearModule, ModuleContext, SpectralModule};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let mut a = PhaseSmearModule::new();
    let mut b = PhaseSmearModule::new();
    a.reset(48000.0, 2048);
    b.reset(48000.0, 2048);

    let num_bins = 1025usize;
    let amount   = vec![1.0f32; num_bins];
    let peak     = vec![1.0f32; num_bins];
    let mix      = vec![1.0f32; num_bins];
    let curves_vec: Vec<Vec<f32>> = vec![amount, peak, mix];
    let curves_ref: Vec<&[f32]> = curves_vec.iter().map(|v| &v[..]).collect();

    let sc_hot  = vec![1.0f32; num_bins];
    let sc_cold = vec![0.0f32; num_bins];

    let mut bins_a: Vec<Complex<f32>> = (0..num_bins)
        .map(|k| Complex::new(1.0, 0.0)).collect();
    let mut bins_b = bins_a.clone();

    let mut supp_a = vec![0.0f32; num_bins];
    let mut supp_b = vec![0.0f32; num_bins];
    let ctx = ModuleContext {
        sample_rate: 48000.0, fft_size: 2048, num_bins,
        attack_ms: 10.0, release_ms: 80.0,
        sensitivity: 0.5, suppression_width: 0.0,
        auto_makeup: false, delta_monitor: false,
    };

    a.process(0, StereoLink::Linked, FxChannelTarget::All,
              &mut bins_a, Some(&sc_hot),  &curves_ref, &mut supp_a, &ctx);
    b.process(0, StereoLink::Linked, FxChannelTarget::All,
              &mut bins_b, Some(&sc_cold), &curves_ref, &mut supp_b, &ctx);

    // Hot SC should introduce a larger phase deviation than cold SC at DC-free bins.
    // Compare phase variance: bins_a should differ from identity more than bins_b on average.
    let diff_a: f32 = bins_a.iter().skip(1).take(num_bins - 2)
        .map(|c| (c.arg()).abs()).sum();
    let diff_b: f32 = bins_b.iter().skip(1).take(num_bins - 2)
        .map(|c| (c.arg()).abs()).sum();
    assert!(diff_a > diff_b,
            "hot SC should produce more smear than cold SC: hot={} cold={}", diff_a, diff_b);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test module_trait phase_smear_sc_modulates_amount`
Expected: FAIL — current PhaseSmear ignores SC.

- [ ] **Step 3: Wire up SC in PhaseSmear**

Replace `src/dsp/modules/phase_smear.rs` body with:

```rust
use num_complex::Complex;
use crate::params::{FxChannelTarget, StereoLink};
use crate::dsp::utils::xorshift64;
use crate::dsp::pipeline::{MAX_NUM_BINS, OVERLAP};
use super::{ModuleContext, ModuleType, SpectralModule};

pub struct PhaseSmearModule {
    rng_state: u64,
    peak_env: Vec<f32>,
    sample_rate: f32,
    fft_size: usize,
}

impl PhaseSmearModule {
    pub fn new() -> Self {
        Self {
            rng_state: 0x123456789abcdef0,
            peak_env: vec![0.0f32; MAX_NUM_BINS],
            sample_rate: 44100.0,
            fft_size: 2048,
        }
    }

    #[inline]
    fn curve_to_hold_ms(curve: f32) -> f32 {
        // Same mapping as Gain; see gain.rs::curve_to_hold_ms.
        let c = curve.clamp(0.0, 2.0);
        let log_min = 1.0f32.ln();
        let log_mid = 50.0f32.ln();
        let log_max = 500.0f32.ln();
        let log_t = if c <= 1.0 {
            log_min + (log_mid - log_min) * c
        } else {
            log_mid + (log_max - log_mid) * (c - 1.0)
        };
        log_t.exp()
    }
}

impl Default for PhaseSmearModule {
    fn default() -> Self { Self::new() }
}

impl SpectralModule for PhaseSmearModule {
    fn reset(&mut self, sample_rate: f32, fft_size: usize) {
        self.sample_rate = sample_rate;
        self.fft_size = fft_size;
        for v in &mut self.peak_env { *v = 0.0; }
    }

    fn process(
        &mut self,
        _channel: usize,
        _stereo_link: StereoLink,
        _target: FxChannelTarget,
        bins: &mut [Complex<f32>],
        sidechain: Option<&[f32]>,
        curves: &[&[f32]],
        suppression_out: &mut [f32],
        _ctx: &ModuleContext,
    ) {
        if bins.is_empty() { suppression_out.fill(0.0); return; }
        let last = bins.len() - 1;
        let hop_ms = self.fft_size as f32 / (OVERLAP as f32 * self.sample_rate) * 1000.0;

        for k in 0..bins.len() {
            let dry = bins[k];
            let rand = xorshift64(&mut self.rng_state);
            if k == 0 || k == last { continue; }

            // SC peak-hold envelope (same shape as Gain).
            let sc_raw  = sidechain.and_then(|s| s.get(k)).copied().unwrap_or(0.0).max(0.0);
            let hold_c  = curves.get(1).and_then(|c| c.get(k)).copied().unwrap_or(1.0);
            let hold_ms = Self::curve_to_hold_ms(hold_c);
            let rel     = (-hop_ms / hold_ms.max(0.1)).exp();
            if sc_raw > self.peak_env[k] {
                self.peak_env[k] = sc_raw;
            } else {
                self.peak_env[k] = rel * self.peak_env[k] + (1.0 - rel) * sc_raw;
            }
            // Clamp SC-follower to [0,1] by a simple saturating map. Anything above 1.0
            // saturates to 1.0; tiny SC magnitudes produce little added smear.
            let sc_mod = (self.peak_env[k]).min(1.0);

            let amount_curve = curves.get(0).and_then(|c| c.get(k))
                               .copied().unwrap_or(1.0).clamp(0.0, 2.0);
            // Effective smear amount = amount_curve * (1 + sc_mod). At sc_mod = 0, behaves identically
            // to the old module; at sc_mod = 1, doubles the smear.
            let per_bin = (amount_curve * (1.0 + sc_mod)).clamp(0.0, 2.0);

            let scale      = per_bin * std::f32::consts::PI;
            let rand_phase = (rand as f32 / u64::MAX as f32 * 2.0 - 1.0) * scale;
            let (mag, phase) = (bins[k].norm(), bins[k].arg());
            let wet = Complex::from_polar(mag, phase + rand_phase);
            let mix = curves.get(2).and_then(|c| c.get(k)).copied().unwrap_or(1.0).clamp(0.0, 1.0);
            bins[k] = Complex::new(
                dry.re * (1.0 - mix) + wet.re * mix,
                dry.im * (1.0 - mix) + wet.im * mix,
            );
        }
        suppression_out.fill(0.0);
    }

    fn module_type(&self) -> ModuleType { ModuleType::PhaseSmear }
    fn num_curves(&self) -> usize { 3 }
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test`
Expected: all pass including `phase_smear_sc_modulates_amount`.

- [ ] **Step 5: Commit**

```bash
git add src/dsp/modules/phase_smear.rs tests/module_trait.rs
git commit -m "feat(phase_smear): SC-follow per-bin amount via PEAK HOLD curve"
```

---

## Task 12: Remove old `sc_gain` and `slot_sidechain` from params

**Files:**
- Modify: `src/params.rs` — delete `sc_gain` and `slot_sidechain` fields + defaults
- Modify: `src/dsp/pipeline.rs` — drop the `params.sc_gain` read (no longer used; envelope normalisation is now intrinsic)
- Modify: `src/editor_ui.rs` — drop references

Note: Pipeline already stopped reading `slot_sidechain` in Task 8. `sc_gain` was still read for the SC STFT scaling; since the new per-slot gain handles per-module scaling, remove the global pre-gain (absorbed into per-slot gain).

- [ ] **Step 1: Remove global `sc_gain` read in pipeline.rs**

Find and delete the `let sc_gain_db = params.sc_gain...` and `sc_gain_lin` lines inside the SC STFT block. Remove the corresponding `for (s, &w) in ... *s *= w * sc_gain_lin;` scaling — replace with plain `*s *= w;` (already done in Task 8's Step 6 replacement — verify no stray reference remains).

- [ ] **Step 2: Remove fields from `SpectralForgeParams`**

In `src/params.rs`, delete:

```rust
    #[persist = "slot_sidechain"]
    pub slot_sidechain: Arc<Mutex<[u8; 9]>>,
```

and:

```rust
    #[id = "sc_gain"]
    pub sc_gain: FloatParam,
```

In `impl Default for SpectralForgeParams`, delete:

```rust
            slot_sidechain: Arc::new(Mutex::new([255u8; 9])),
```

and:

```rust
            sc_gain: FloatParam::new(
                "SC Gain", 0.0,
                FloatRange::Linear { min: -18.0, max: 18.0 },
            ).with_smoother(SmoothingStyle::Linear(50.0))
             .with_step_size(0.01)
             .with_unit(" dB"),
```

- [ ] **Step 3: Remove editor references**

In `src/editor_ui.rs` locate:

```rust
                        knob!(ui, &params.sc_gain,     "SC");
```

Delete that line. Also delete the per-slot sidechain popup block (`sc_labels`, `sc_assign` read/write loops around lines 497-525). The block is wholly removed; the SC indicator is the global meter added in Task 13.

- [ ] **Step 4: Build and test**

Run: `cargo build && cargo test`
Expected: compiles, tests pass. You may get unused-import warnings — clean them.

- [ ] **Step 5: Commit**

```bash
git add src/params.rs src/dsp/pipeline.rs src/editor_ui.rs
git commit -m "refactor(params): remove sc_gain and slot_sidechain"
```

---

## Task 13: Top-bar SC meter — replace knob with 4-px yellow level bar

**Files:**
- Modify: `src/editor_ui.rs` — where the old SC knob lived, add meter widget right of Falloff
- Modify: `src/editor/theme.rs` — add yellow colour + bar height constants

The meter reads `shared.sidechain_active: Arc<AtomicBool>` (already available) AND a running peak level. For a 4-px bar, "level" can be represented as brightness: dim when silent, solid lit yellow when SC present. Use `sidechain_active` as the core gate.

- [ ] **Step 1: Add theme constants**

In `src/editor/theme.rs`, add:

```rust
pub const SC_METER_HEIGHT_PX: f32 = 4.0;
pub const SC_METER_WIDTH_PX:  f32 = 80.0;
pub const SC_METER_COLOR_LIT: egui::Color32 = egui::Color32::from_rgb(0xe0, 0xc0, 0x30); // yellow
pub const SC_METER_COLOR_DIM: egui::Color32 = egui::Color32::from_rgb(0x55, 0x48, 0x10); // dim yellow
```

- [ ] **Step 2: Wire `sidechain_active` handle into the editor closure**

The `create_editor` function already threads `sidechain_active`. Rename occurrences from `[Arc<AtomicBool>; 4]` to `Arc<AtomicBool>` per Task 6. In the top bar rendering (near where Falloff is laid out), add:

```rust
ui.add_space(8.0);
let sc_lit = sidechain_active
    .as_ref()
    .map(|a| a.load(std::sync::atomic::Ordering::Relaxed))
    .unwrap_or(false);
let color = if sc_lit {
    crate::editor::theme::SC_METER_COLOR_LIT
} else {
    crate::editor::theme::SC_METER_COLOR_DIM
};
let (rect, _resp) = ui.allocate_exact_size(
    egui::vec2(crate::editor::theme::SC_METER_WIDTH_PX, crate::editor::theme::SC_METER_HEIGHT_PX),
    egui::Sense::hover(),
);
ui.painter().rect_filled(rect, 0.0, color);
```

Place this immediately after the Falloff widget in the top bar. If you cannot identify Falloff at a glance, grep for `peak_falloff_ms` in `src/editor_ui.rs` — that's the Falloff handle.

- [ ] **Step 3: Build and visually verify**

Run: `cargo build --release`
Expected: builds.

For visual check:
```bash
cargo run --package xtask -- bundle spectral_forge --release
cp target/bundled/spectral_forge.clap ~/.clap/
```
Then open the plugin in Bitwig; confirm the yellow bar appears right of Falloff, dim by default, lit when SC audio is piped in.

- [ ] **Step 4: Commit**

```bash
git add src/editor/theme.rs src/editor_ui.rs
git commit -m "feat(editor): SC level meter (4-px yellow bar) replacing old SC knob"
```

---

## Task 14: Per-module SC strip in Dynamics, Gain, Smear, Freeze panels

**Files:**
- Modify: `src/editor_ui.rs` — add SC strip render helper + call in each SC-aware module's panel section

The SC strip shows: an SC gain knob (range -90…+18 dB, reading `params.slot_sc_gain_db[slot]`) and a channel selector (`ScChannel`, reading `params.slot_sc_channel[slot]`).

- [ ] **Step 1: Add helper function**

At the bottom of `src/editor_ui.rs` (or in a small submodule inside `editor/`), add:

```rust
fn sc_strip_ui(
    ui: &mut egui::Ui,
    params: &SpectralForgeParams,
    slot_idx: usize,
) {
    use crate::params::ScChannel;

    ui.horizontal(|ui| {
        ui.label("SC");
        // SC gain knob
        {
            let mut gains = params.slot_sc_gain_db.lock();
            let mut g = gains[slot_idx];
            let resp = ui.add(
                egui::DragValue::new(&mut g)
                    .clamp_range(-90.0..=18.0)
                    .speed(0.1)
                    .suffix(" dB")
                    .custom_formatter(|v, _| {
                        if v <= -90.0 { "−∞".to_owned() } else { format!("{:.1}", v) }
                    })
            );
            if resp.changed() { gains[slot_idx] = g; }
        }
        ui.separator();
        // SC channel selector
        {
            let mut chans = params.slot_sc_channel.lock();
            let cur = chans[slot_idx];
            let label = match cur {
                ScChannel::Follow => "Follow",
                ScChannel::LR => "L+R",
                ScChannel::L  => "L",
                ScChannel::R  => "R",
                ScChannel::M  => "M",
                ScChannel::S  => "S",
            };
            egui::ComboBox::new(format!("sc_chan_slot_{}", slot_idx), "Source")
                .selected_text(label)
                .show_ui(ui, |ui| {
                    for (v, text) in [
                        (ScChannel::Follow, "Follow"),
                        (ScChannel::LR,     "L+R"),
                        (ScChannel::L,      "L"),
                        (ScChannel::R,      "R"),
                        (ScChannel::M,      "M"),
                        (ScChannel::S,      "S"),
                    ] {
                        if ui.selectable_label(cur == v, text).clicked() {
                            chans[slot_idx] = v;
                        }
                    }
                });
        }
    });
}
```

- [ ] **Step 2: Call from SC-aware module panels**

In `src/editor_ui.rs`, locate each module-panel rendering path (Dynamics, Gain, Smear, Freeze). In each, right after the module-name header and before the curve tabs, call:

```rust
if crate::dsp::modules::module_spec(module_type).supports_sidechain {
    sc_strip_ui(ui, &params, edit_slot);
    ui.separator();
}
```

If you cannot identify each panel cleanly, centralise: find the single place where the active module's panel renders (searching for `editing_slot` or `edit_slot`), and add the conditional block once there, gated on `supports_sidechain`.

- [ ] **Step 3: Build and verify**

Run: `cargo build`
Expected: builds.

Bundle and visually check:
```bash
cargo run --package xtask -- bundle spectral_forge --release
```
Put a Gain module, a Dynamics module, a Smear module, a Freeze module in different slots. Verify each shows the SC strip, Contrast and others do not.

- [ ] **Step 4: Commit**

```bash
git add src/editor_ui.rs
git commit -m "feat(editor): per-module SC strip (gain + channel) for SC-aware modules"
```

---

## Task 15: Curve-editor overlay — 1px animated darker line for Gain peak-hold envelope

**Files:**
- Modify: `src/editor/curve.rs` — overlay painter
- Modify: `src/editor_ui.rs` — thread `sc_envelope_rx` into the curve painter

- [ ] **Step 1: Thread the SC envelope handle**

In `create_editor`'s closure, add an `sc_envelope_rx: Option<Arc<Mutex<TbOutput<Vec<f32>>>>>` parameter. Plumb from `lib.rs`'s `editor()`:

- Add `gui_sc_envelope_rx: Option<Arc<Mutex<triple_buffer::Output<Vec<f32>>>>>` field to `SpectralForge`.
- In `impl Default`, set `gui_sc_envelope_rx = Some(shared.sc_envelope_rx.clone())`.
- Pass through in `editor()`.

- [ ] **Step 2: Paint the overlay in the curve widget**

In `src/editor/curve.rs`, find `paint_response_curve` (or equivalent curve-painting function that draws the user-drawn curve). Extend it with optional live-envelope overlay:

```rust
pub fn paint_peak_hold_envelope_overlay(
    painter: &egui::Painter,
    rect: egui::Rect,
    envelope: &[f32],
    curve_color: egui::Color32,
) {
    if envelope.is_empty() { return; }
    // Derive a darker tone from curve_color.
    let dim = egui::Color32::from_rgba_premultiplied(
        curve_color.r() / 3, curve_color.g() / 3, curve_color.b() / 3, 0xff
    );
    let n = envelope.len();
    let mut prev: Option<egui::Pos2> = None;
    // Map bin index to x; map magnitude (0..~norm) to y with log scale, matching existing painter.
    for k in 1..n {
        let t = k as f32 / (n - 1) as f32;
        let x = rect.min.x + t * rect.width();
        // Map magnitude to normalised 0..1 (log-dB), clamping.
        let mag = envelope[k].max(1e-12);
        let db = 20.0 * mag.log10();
        let norm = ((db + 90.0) / 90.0).clamp(0.0, 1.0); // 0 dB top, -90 dB bottom
        let y = rect.max.y - norm * rect.height();
        if let Some(p) = prev {
            painter.line_segment([p, egui::pos2(x, y)], egui::Stroke::new(1.0, dim));
        }
        prev = Some(egui::pos2(x, y));
    }
}
```

- [ ] **Step 3: Call the overlay when editing a Gain PEAK HOLD curve**

In the curve-editor rendering pathway (where the drawn curve is already painted), after the main curve stroke:

```rust
let show_overlay = module_type == ModuleType::Gain && editing_curve_idx == 1; // PEAK HOLD
if show_overlay {
    if let Some(mut rx) = sc_envelope_rx.as_ref().and_then(|r| r.try_lock()) {
        let env = rx.read();
        paint_peak_hold_envelope_overlay(painter, rect, env, module_color);
    }
}
```

Also: when `module_type == ModuleType::Gain` and `gain_mode != GainMode::Pull`, gray the curve tab. Locate the curve-tab row UI and apply `ui.set_enabled(false)` or similar for the PEAK HOLD tab in Add/Subtract. Don't delete data; only the visual state changes.

- [ ] **Step 4: Build and verify**

Run: `cargo build`
Expected: builds.

Visually: place a Gain slot, select Pull mode, route SC in with a drum loop, open the PEAK HOLD curve. A live darker line should move behind the drawn curve. Flip to Add mode: tab is grayed.

- [ ] **Step 5: Commit**

```bash
git add src/lib.rs src/editor_ui.rs src/editor/curve.rs
git commit -m "feat(editor): live SC peak-hold envelope overlay on Gain PEAK HOLD curve"
```

---

## Task 16: Update manual

**Files:**
- Modify: `docs/MANUAL.md` — add Sidechain section

- [ ] **Step 1: Add section**

Append (or insert at the appropriate place) a "Sidechain" section to `docs/MANUAL.md`:

```markdown
## Sidechain

Spectral Forge accepts one stereo sidechain (SC) input. In Bitwig, route any track's output to the plugin's sidechain input as usual.

### Which modules use the sidechain?

| Module          | SC role                                                                  |
|-----------------|--------------------------------------------------------------------------|
| **Dynamics**    | External detector for per-bin gain reduction.                            |
| **Gain**        | In Pull mode: pulls output magnitude toward the SC magnitude per bin. PEAK HOLD curve smooths it. |
| **Phase Smear** | Modulates per-bin smear amount by SC magnitude, smoothed by PEAK HOLD curve. |
| **Freeze**      | Gates the freeze threshold; louder SC raises effective threshold.        |

Other modules (Contrast, Mid/Side, T/S Split, Harmonic) do not use the sidechain and show no SC controls.

### Per-module SC controls

Each SC-aware module panel carries:

- **SC gain** (−∞ to +18 dB) — level applied to the SC signal for *this slot only*. −∞ disables SC for the slot.
- **SC source** — which channel of the stereo SC signal the slot keys off:

| Choice    | Behaviour                                                                  |
|-----------|----------------------------------------------------------------------------|
| **Follow**    | Routes the SC channel matching whatever the slot is currently processing. See table below. |
| **L+R**       | Sum of SC left and right.                                                  |
| **L**         | SC left channel only.                                                      |
| **R**         | SC right channel only.                                                     |
| **M**         | Mid (L+R)/√2 of the SC.                                                    |
| **S**         | Side (L−R)/√2 of the SC.                                                   |

### Follow semantics

| Stereo Link    | Follow resolves to                                                        |
|----------------|---------------------------------------------------------------------------|
| Linked         | L+R                                                                       |
| Independent    | Channel-paired: main L → SC L, main R → SC R                              |
| Mid/Side       | Target-paired: Mid-target slot → SC M, Side-target slot → SC S, All-target slot → L+R |

To duck the mids by the sides of the SC input: route a stereo SC, target the slot to Mid, set SC source to **S**.

### SC level indicator

The small yellow bar in the top bar (right of Falloff) lights up when the plugin is receiving audio on the SC input. If the bar stays dim while playing your project, the SC isn't reaching the plugin — check your host routing.

### Gain Pull peak-hold curve

In Gain/Pull mode, the second curve — **PEAK HOLD** — sets per-bin peak-hold time (1 ms to ~500 ms, log). Longer hold prevents pumping on percussive SC material; shorter hold tracks detail. In Add and Subtract modes the curve is grayed and has no effect.

When editing the PEAK HOLD curve, a thin animated darker line shows the live per-bin SC envelope the module is currently seeing.
```

- [ ] **Step 2: Commit**

```bash
git add docs/MANUAL.md
git commit -m "docs(manual): add Sidechain section"
```

---

## Task 17: Final test pass + cleanup

**Files:** (none — test and cleanup pass)

- [ ] **Step 1: Full test + lint**

Run: `cargo test`
Expected: all tests pass.

Run: `cargo build --release`
Expected: release builds cleanly.

Fix any new warnings introduced during the refactor (unused imports, dead code). Do NOT silence warnings that reveal real issues; fix them.

- [ ] **Step 2: Manual end-to-end smoke (if time permits)**

```bash
cargo run --package xtask -- bundle spectral_forge --release
cp target/bundled/spectral_forge.clap ~/.clap/
```
Open Bitwig. Load the plugin on a track. Route a drum loop to the SC. Verify:
- Top-bar yellow SC bar lights up.
- Dynamics slot: SC strip appears, gain/channel work.
- Gain slot in Pull mode: PEAK HOLD curve tab enabled, shows live dim line, ducking follows SC.
- Gain slot in Add mode: PEAK HOLD tab grayed.
- Contrast, Mid/Side, T/S Split, Harmonic: no SC strip.
- Freeze slot: defaults capture the dry signal at ~-50 dB-like threshold (much more sensitive than before).

- [ ] **Step 3: Final commit**

If any warnings/fixes/cleanup landed:

```bash
git add -A
git commit -m "chore: post-refactor cleanup and warnings"
```

If not, the branch is ready for review.

---

## Post-plan

When all tasks above pass:

1. Announce completion.
2. Invoke `superpowers:finishing-a-development-branch` to decide on merge / PR / further review.
