# Sidechain Refactor — Design

**Date:** 2026-04-21
**Status:** Approved, awaiting implementation plan
**Branch:** `feature/sidechain-refactor`

## 1. Summary

Refactor the sidechain (SC) model from a plugin-global, four-aux-port design to a **single stereo SC input with per-module controls**. Only modules that meaningfully use SC expose SC controls. The SC signal stays stereo internally so cross-channel keying ("duck the mids by the side of the SC input") works inside a single plugin instance. A Gain/Pull-specific per-bin peak-hold curve is added to prevent pumping, with a live per-bin modulation visualisation behind it.

## 2. Motivation

Current state:

- 4 CLAP aux stereo input ports (`lib.rs:76`), none of which are populated by most DAWs. Bitwig (primary target) exposes one SC input.
- Global `params.sc_gain` knob in the top bar and a plugin-wide "SC active" button.
- Per-slot `params.slot_sidechain: Arc<Mutex<[u8; 9]>>` stores `0..=3` (aux index) or `255` (self-detect).
- Modules that don't use SC still share the global routing UI, adding noise.
- Gain/Pull mode has no peak-hold mechanism, leading to audible pumping with percussive SC sources.
- Freeze's threshold default is too hot for practical use.

The refactor:

- One stereo SC port with optional stereo-preserving routing (L/R/M/S) per module.
- Per-module SC gain (−∞…+18 dB) and channel selector in the module's own panel.
- Peak-hold curve in the Gain module to tame Pull-mode pumping.
- Plugin-global SC visible only as a compact 4-px yellow level meter in the top bar.
- Freeze threshold default → −50 dB.

No preset migration: the plugin is pre-release, no shipping presets exist. Preset-format work is happening in a parallel session and will absorb the new shape when it starts reading state.

## 3. Plugin Input Topology

`lib.rs` changes `aux_input_ports: &[new_nonzero_u32(2), ..., new_nonzero_u32(2)]` (4 stereo ports) to `aux_input_ports: &[new_nonzero_u32(2)]` (1 stereo port). Aux ports 1–3 disappear from the CLAP manifest. Reported latency remains `fft_size` samples.

## 4. Per-Module SC Controls

### 4.1 SC-aware module set

**Dynamics, Gain, PhaseSmear, Freeze.** Contrast, MidSide, T/S Split, Harmonic, Master, Empty are *not* SC-aware; their panels show no SC strip.

### 4.2 Shared SC strip

Every SC-aware module panel carries, at minimum:

- **SC input gain knob**, range **−∞ to +18 dB**. −∞ disables SC for that module (no separate enable toggle).
- **SC channel selector**: `Follow / L+R / L / R / M / S`.

### 4.3 Module-specific extras

- **Gain:** replaces the dead `SC SMOOTH` curve with a functional `PEAK HOLD` curve (§6).
- **Dynamics, PhaseSmear, Freeze:** no extras in v1. Panels leave room for future module-specific SC controls.

### 4.4 `ModuleSpec` changes

Add a boolean `supports_sidechain` field to `ModuleSpec`. Set `true` for Dynamics, Gain, PhaseSmear, Freeze. Drives whether the editor renders the SC strip.

### 4.5 Trait signature

`SpectralModule::process()` keeps its existing `sidechain: Option<&[f32]>` parameter. Non-SC-aware modules continue to ignore it. The Pipeline passes `None` for any slot whose module is not SC-aware. For SC-aware slots, the Pipeline prepares a per-slot SC magnitude slice by applying the slot's `sc_input_gain` and `sc_channel` routing; if `sc_input_gain` is −∞, the Pipeline passes `None` rather than allocating and zeroing a slice.

## 5. Channel Selector Semantics

`Follow` resolves against the current stereo mode:

| Stereo mode     | Follow behaviour                                                                  |
|-----------------|-----------------------------------------------------------------------------------|
| **Linked**      | SC L+R summed                                                                     |
| **Independent** | Channel-paired: main-L keys off SC-L; main-R keys off SC-R                        |
| **MidSide**     | Target-paired: `Mid` target → SC-M; `Side` target → SC-S; `All` target → SC L+R   |

Explicit choices (`L+R`, `L`, `R`, `M`, `S`) always apply literally regardless of stereo mode. `M` and `S` are computed from the SC L/R on the fly even when the main signal is Linked or Independent, so `M` and `S` always produce the expected result.

**Rule for manual:** *Follow = route the SC channel that matches whatever this slot is currently processing.*

## 6. Gain Module Peak-Hold Curve

### 6.1 Curve set

Gain's `curve_labels` changes from `["GAIN", "SC SMOOTH"]` to `["GAIN", "PEAK HOLD"]`. Curve count stays 2. PhaseSmear's `curve_labels` changes from `["AMOUNT", "SC SMOOTH", "MIX"]` to `["AMOUNT", "PEAK HOLD", "MIX"]` for consistency with Gain's SC-peak-hold treatment. Contrast's `SC SMOOTH` curve is removed entirely (Contrast is not SC-aware); its `curve_labels` becomes `["AMOUNT"]`, `num_curves: 1`.

### 6.2 Peak-hold semantics

Per-bin peak-hold time, log-scaled, approximately **1 ms to 500 ms** (exact range to be tuned during implementation).

- **Gain module:** Applies only in `GainMode::Pull`. In `Add` and `Subtract`, the curve tab is **visually grayed** in the curve editor, and the values are not consulted by the DSP.
- **PhaseSmear module:** Always active (PhaseSmear has no modes). Same curve-range semantics; feeds the per-bin smear-amount SC follower to prevent pumping on percussive SC material. No grayed state.

Note for implementers: verify during implementation that PhaseSmear's current `SC SMOOTH` curve isn't already doing something meaningful — if it is, reconcile the rename with the existing behaviour (either generalise to peak-hold, or keep a separate smoothing path alongside). The user reported `SC SMOOTH` is a no-op in Gain; status in PhaseSmear is to be confirmed.

### 6.3 Live modulation visualisation

Inside the curve-edit painter, render a **1 px animated, darker-toned line** behind the user's drawn peak-hold curve. The line reflects the **live per-bin peak-hold envelope** derived from the current SC magnitudes, sampled at the block rate. This gives users a real-time view of what the peak-hold curve is shaping. Colour: a dimmed variant of the module's `color_lit`. Updates via the existing triple-buffer channels; a new dedicated bridge channel for "SC modulation envelope per bin" is added if no existing channel is suitable (implementation detail).

## 7. Freeze Threshold Default

`FreezeModule`'s threshold parameter default changes to **−50 dB**. Everything else about the module is unchanged in this refactor.

## 8. Global UI

### 8.1 Remove

- `params.sc_gain` knob from the top bar (`editor_ui.rs:593`).
- Global `sidechain_active` indicator button.

### 8.2 Add

- **4 px yellow solid-bar SC input meter** in the upper-right corner, placed **immediately to the right of the Falloff setting**. Driven by the stereo SC input level (peak, or short peak-hold — whichever reads clearly at 4 px). Replaces the removed SC button. Dim when no SC signal is detected; lit yellow when active.

No per-slot SC-assignment UI in the module popup or elsewhere — that concept is gone.

## 9. Parameter and State Model Changes

### 9.1 Removed

- `params.sc_gain: FloatParam`
- `params.slot_sidechain: Arc<Mutex<[u8; 9]>>`
- `bridge::SharedState.sidechain_active: Option<[Arc<AtomicBool>; 4]>` — replaced by a single `AtomicBool`.
- `pipeline.sc_stfts: Vec<StftHelper>` (length 4) — collapses to a single `StftHelper` (still 2-channel).
- Aux input ports 1–3 from CLAP manifest.

### 9.2 Added

For each SC-aware module (Dynamics, Gain, PhaseSmear, Freeze), **per-slot** nih-plug params:

- `sc_input_gain: FloatParam` — range `-inf..=+18 dB`, default `0 dB`. Persisted.
- `sc_channel: EnumParam<ScChannel>` — variants `Follow`, `LR`, `L`, `R`, `M`, `S`; default `Follow`. Persisted.

These are **per slot** (not per module-type) because the same module type can occupy multiple slots. Likely storage shape: `Arc<Mutex<[ScChannel; 9]>>` and an `[FloatParam; 9]` array, mirroring the existing per-slot pattern used by `slot_gain_mode`. Final wiring is an implementation detail; the key contract is that each slot running an SC-aware module has its own two values.

### 9.3 Bridge changes

- `SharedState.sidechain_active`: `Option<[AtomicBool; 4]>` → `Arc<AtomicBool>` (always present; single SC port).
- New bridge channel for "SC modulation envelope per bin" (mandatory — drives the Gain peak-hold visualisation). Uses `triple_buffer` following the existing pattern for spectrum/suppression displays. Audio thread publishes; GUI reads via `try_lock().read()`. Indexed per Gain slot so the curve-edit painter can look up the right envelope when the user is editing that slot's peak-hold curve.

### 9.4 `ModuleSpec` changes

```rust
pub struct ModuleSpec {
    pub display_name: &'static str,
    pub color_lit:    Color32,
    pub color_dim:    Color32,
    pub num_curves:   usize,
    pub curve_labels: &'static [&'static str],
    pub supports_sidechain: bool,  // NEW
}
```

Set `supports_sidechain: true` for `DYN`, `GN`, `PSM`, `FRZ`. All others `false`.

## 10. Real-Time Safety

All the usual rules apply. Specific points for this refactor:

- The per-module `sc_input_gain` and `sc_channel` params are read once per block into local buffers before the STFT closure, never mid-closure, following the `route_matrix_snap` pattern in `pipeline.rs`.
- The stereo SC STFT collapses from 4 instances to 1; no reallocation on the audio thread.
- Channel selector evaluation (`Follow` resolution, `M`/`S` computation from L/R) happens during SC buffer preparation, before the per-slot loop, and writes into a pre-allocated per-slot SC input slice array.
- Per-bin peak-hold state for Gain's `PEAK HOLD` curve lives in a pre-allocated `Vec<f32>` per Gain slot, sized `MAX_NUM_BINS`. No allocation inside `process()`.

## 11. Testing

Tests to add or update:

- `module_trait.rs` — add cases covering the new per-module SC gain and channel selector on each of the 4 SC-aware modules.
- New: channel-selector resolution matrix test (all combinations of stereo mode × Follow/LR/L/R/M/S).
- New: Gain-module peak-hold curve decay behaviour (bin-rate peak tracking against a synthetic SC impulse).
- Update `engine_contract.rs` if any engine-level contracts shift (unlikely — most change is at the module level).

Baseline: 34 tests passing (`cargo test` in worktree, 2026-04-21).

## 12. Manual Updates

`docs/MANUAL.md` gains a Sidechain section describing:

- One stereo SC input, hooked up in the host.
- Per-module SC controls: gain (−∞…+18 dB) and channel selector.
- Channel selector semantics table (reproducing §5).
- Gain/Pull peak-hold curve and the live modulation line visualisation.
- Which modules are SC-aware.

## 13. Out of Scope

- Preset migration (no existing presets to migrate).
- Multiple simultaneous SC inputs (explicit design decision: one stereo SC; users with multi-key needs chain plugin instances).
- Parameter-automation-driven modulation (handled in a separate session; complementary, not overlapping for spectral keying).
- Any behaviour change for Dynamics' engine beyond wiring it to the new per-module SC gain/channel.

## 14. Implementation Order Hint

A rough sensible order (the implementation plan will refine):

1. `ModuleSpec` + `supports_sidechain` flag, trait signature unchanged.
2. CLAP manifest: drop 3 aux ports.
3. Pipeline: collapse `sc_stfts` 4 → 1; new SC channel resolution step.
4. Remove `params.sc_gain` and `params.slot_sidechain`; remove top-bar SC knob + popup assign UI.
5. Per-module `sc_input_gain` + `sc_channel` params + SC strip in each SC-aware module's panel.
6. Gain curve rename `SC SMOOTH` → `PEAK HOLD`; implement peak-hold DSP in `GainModule`.
7. PhaseSmear curve rename `SC SMOOTH` → `PEAK HOLD`; hook up analogously.
8. Contrast: remove `SC SMOOTH` curve entirely; drop `num_curves` to 1.
9. Live SC modulation-envelope bridge channel + curve-editor painter overlay.
10. Top-bar 4-px yellow SC level meter, right of Falloff.
11. Freeze threshold default → −50 dB.
12. Manual section + tests.
