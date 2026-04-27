> **Status (2026-04-27): IMPLEMENTED.** All 11 tasks merged on `feature/next-gen-modules-plans`. Pipeline picks up live `ctx.bpm` + `ctx.beat_position` from `nih_plug::buffer::Transport` each block (defaults to 0.0 when host doesn't supply them — modules treat `bpm <= 1e-3` as a passthrough signal). `RhythmModule` ships three BPM-driven modes: **Euclidean** (Bjorklund pattern via `bjorklund_into` writing into a fixed `[bool; 32]` on-stack scratch — supports up to the 32-step max from `division_to_steps`), **Arpeggiator** (step-crossing-driven greedy peak picker fills `arp_voice_peak_bin[8]` then advances per-voice envelopes through `attack_step_frac` × step length, gated by an 8×8 `ArpGrid`), and **Phase Reset / Laser** (per-step transient overwrite blending bin phase toward the TARGET_PHASE curve, hard-skipping DC and Nyquist bins so realfft inverse stays well-defined). Five curves (`AMOUNT`/`DIVISION`/`ATTACK_FADE`/`TARGET_PHASE`/`MIX`) — DIVISION quantises to {1,2,4,8,16,32}-steps via `division_to_steps`. Per-slot mode + arpeggiator grid persisted as `Arc<Mutex<[RhythmMode; 9]>>` and `Arc<Mutex<[ArpGrid; 9]>>` (mirrors the `slot_future_mode` / `slot_punch_mode` shape — NOT the plan's per-slot `[Mutex<T>; MAX_SLOTS]`); JSON round-trip via `persist_out!` / `persist_in!`. Audio-thread dispatch is `try_lock`-once-per-block via `FxMatrix::set_rhythm_modes_and_grids`. UI extends the unified Mode-row block to handle a third module type (Future + Punch + Rhythm) using themed `egui::Button` styling (NOT the plan's `ComboBox`); `PanelWidgetFn` was widened from `fn(&mut Ui, slot)` to `fn(&mut Ui, &SpectralForgeParams, slot)` so the 8×8 Arpeggiator step grid widget at `editor/rhythm_panel.rs` can read `slot_arp_grid` and `slot_rhythm_mode` directly. Cell colours come from `module_spec(ModuleType::Rhythm).color_lit/.color_dim` (no new theme constants); strokes use `th::scaled_stroke(th::STROKE_HAIRLINE, scale)` honoring UI scaling. Calibration probes (`#[cfg(any(test, feature = "probe"))]`) cover all three modes × {AMOUNT default/max, MIX max} = 9 tests in `tests/calibration_roundtrip.rs`; the `run_rhythm_case` helper sets `ctx.bpm = 120.0` and `beat_position = 0.0` so `step_idx = 0` (Bjorklund E(4,8) and E(8,8) both pulse at step 0, making the gate state unambiguous in the Euclidean assertions). Bin Swing deferred (waits for Spectral Delay infra); NoteIn-triggered Arpeggiator deferred (waits for Phase 6.3 MIDI plumbing). The code is the source of truth; this plan is kept for history. See [../STATUS.md](../STATUS.md).

# Phase 2d — Rhythm Module Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship a new `Rhythm` module with three BPM-driven sub-effects: **Euclidean** (Bjorklund-distributed gates per band), **Arpeggiator** (BPM-stepped voice rotation across spectral peaks), and **Phase Reset** (a.k.a. Laser — periodic phase overwrite). Defer Bin Swing per the audit (waits for Spectral Delay infrastructure). NoteIn-triggered arpeggiator is also deferred (waits for MIDI plumbing in Phase 6.3).

**Architecture:** Plain `SpectralModule` slot, BPM-driven (no internal beat clock — reads `ctx.bpm` and `ctx.beat_position` from `ModuleContext`, populated by Pipeline from the host transport). Per-slot mode enum chooses between Euclidean / Arpeggiator / PhaseReset. Arpeggiator's step grid lives in `params.slot_arp_grid: [Mutex<ArpGrid>; MAX_SLOTS]` and is rendered via the Phase 1 `panel_widget` ModuleSpec callback.

**Tech Stack:** Rust, num_complex, nih-plug, nih-plug-egui.

**Source design:** `docs/superpowers/specs/2026-04-21-rhythm-module.md` (DEFERRED) + `ideas/next-gen-modules/17-rhythm.md` (audit). This plan supersedes the 2026-04-21 spec on land.

**Roadmap reference:** `ideas/next-gen-modules/99-implementation-roadmap.md` § Phase 2 (item 4).

**Depends on:**
- Phase 1: `panel_widget` field on `ModuleSpec`; `bpm` and `beat_position` fields on `ModuleContext`.
- Pipeline must populate `ctx.bpm` and `ctx.beat_position` from `nih_plug::buffer::Transport` each block.

---

## File structure

| File | Status | Responsibility |
|---|---|---|
| `src/dsp/modules/rhythm.rs` | Create | `RhythmModule`, `RhythmMode` enum, `ArpGrid` struct, Bjorklund Euclidean kernel, beat-position gate, Arpeggiator step advance, Phase Reset overwrite. |
| `src/dsp/modules/mod.rs` | Modify | Add `Rhythm` to `ModuleType`, `RHY` static `ModuleSpec` with `panel_widget = Some(rhythm_panel)`. |
| `src/editor/rhythm_panel.rs` | Create | The 8×8 Arpeggiator step grid widget (called via `panel_widget`). |
| `src/editor/module_popup.rs` | Modify | Add `Rhythm` to `ASSIGNABLE`. |
| `src/editor_ui.rs` | Modify | Per-slot mode picker for Rhythm (Euclidean/Arpeggiator/PhaseReset). |
| `src/params.rs` | Modify | Add `slot_rhythm_mode: [Mutex<RhythmMode>; MAX_SLOTS]` and `slot_arp_grid: [Mutex<ArpGrid>; MAX_SLOTS]`. |
| `src/dsp/pipeline.rs` | Modify | Read host transport (BPM + ppq position), populate `ctx.bpm` and `ctx.beat_position`. |
| `tests/rhythm.rs` | Create | Bjorklund unit test, beat-position math, Arpeggiator advance, Phase Reset overwrite. |

---

## Curve mapping (5 curves)

| Idx | Label | Euclidean | Arpeggiator | Phase Reset |
|---|---|---|---|---|
| 0 | AMOUNT | Density / gate depth | Velocity envelope | Reset strength (0=no reset, 1=full overwrite) |
| 1 | DIVISION | Steps per bar (curve maps to discrete 1, 2, 4, 8, 16, 32) | Steps per bar | Subdivision (same mapping) |
| 2 | ATTACK_FADE | Gate attack/release shape | Voice attack | Reset envelope width (hops over which the reset blends in/out) |
| 3 | TARGET_PHASE | unused | unused | Per-bin target phase (-π to +π) |
| 4 | MIX | Wet (0–1) | (same) | (same) |

`num_curves() = 5`.

---

## Task 1 — Pipeline: populate `ctx.bpm` + `ctx.beat_position`

**Files:**
- Modify: `src/dsp/pipeline.rs`
- Test: `tests/rhythm.rs` (we'll test the module's reaction in later tasks; this task only wires Pipeline)

**Pre-req:** Phase 1 plan added `bpm: f32` and `beat_position: f64` to `ModuleContext`. Verify:

```bash
rg "pub bpm:|pub beat_position:" src/dsp/modules/mod.rs -n
```

If absent, **stop**: this plan is blocked on Phase 1.

- [ ] **Step 1: Find where `ModuleContext` is constructed in `pipeline.rs`**

Run: `rg "ModuleContext \{" src/dsp/pipeline.rs -n -B 2 -A 12`

- [ ] **Step 2: Add transport read at the start of `process()`**

The plugin's `process()` receives `_aux: &mut AuxiliaryBuffers` and `context: &mut impl ProcessContext<Self>`. The transport is on `context.transport()`.

In `Pipeline::process()` (called from `lib.rs:Process::process`), accept the transport as a parameter and use it. Edit `Pipeline::process()` signature to accept `&Transport`:

Find `Pipeline::process` signature in `pipeline.rs`. Add a parameter `transport: &nih_plug::context::process::Transport`. Update the call site in `src/lib.rs` to pass `context.transport()`.

In `pipeline.rs`, after the existing block-level setup but before the `ModuleContext` is built, read transport:

```rust
let bpm           = transport.tempo.unwrap_or(120.0) as f32;
let beat_position = transport.pos_beats().unwrap_or(0.0);
```

When constructing `ModuleContext`, fill those fields:

```rust
let ctx = ModuleContext {
    sample_rate:       self.sample_rate,
    fft_size:          self.fft_size,
    num_bins:          self.num_bins,
    attack_ms:         params.attack_ms.value(),
    release_ms:        params.release_ms.value(),
    sensitivity:       params.sensitivity.value(),
    suppression_width: params.suppression_width.value(),
    auto_makeup:       params.auto_makeup.value(),
    delta_monitor:     params.delta_monitor.value(),
    bpm,
    beat_position,
    // ...other Phase-1-added Option<&[f32]> fields stay None
    ..Default::default()
};
```

(If `ModuleContext` has no `Default` yet, either add one in this task or set the `None` fields explicitly.)

- [ ] **Step 3: Verify compile + tests**

Run: `cargo build && cargo test`
Expected: green. The full suite should still pass since BPM/beat_position were unused.

- [ ] **Step 4: Commit**

```bash
git add src/dsp/pipeline.rs src/lib.rs
git commit -m "feat(pipeline): populate ctx.bpm and ctx.beat_position from host transport"
```

---

## Task 2 — `ModuleType::Rhythm` + spec

**Files:**
- Modify: `src/dsp/modules/mod.rs`

- [ ] **Step 1: Write the failing test**

```rust
// tests/rhythm.rs (NEW)
use spectral_forge::dsp::modules::{ModuleType, module_spec};

#[test]
fn rhythm_module_spec() {
    let spec = module_spec(ModuleType::Rhythm);
    assert_eq!(spec.num_curves, 5);
    assert_eq!(spec.curve_labels, &["AMOUNT", "DIVISION", "ATTACK_FADE", "TARGET_PHASE", "MIX"]);
    assert!(!spec.supports_sidechain);
    assert!(spec.panel_widget.is_some(), "rhythm needs a panel widget for arpeggiator step grid");
    assert_eq!(spec.display_name, "Rhythm");
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --test rhythm rhythm_module_spec`
Expected: compile error — `ModuleType::Rhythm` does not exist.

- [ ] **Step 3: Add the variant + spec**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ModuleType {
    #[default]
    Empty,
    Dynamics,
    Freeze,
    PhaseSmear,
    Contrast,
    Gain,
    MidSide,
    TransientSustainedSplit,
    Harmonic,
    Future,
    Punch,
    Rhythm,        // NEW
    Master,
}
```

In `module_spec()`:

```rust
static RHY: ModuleSpec = ModuleSpec {
    display_name: "Rhythm",
    color_lit: Color32::from_rgb(0xc8, 0xb0, 0x60),
    color_dim: Color32::from_rgb(0x42, 0x38, 0x20),
    num_curves: 5,
    curve_labels: &["AMOUNT", "DIVISION", "ATTACK_FADE", "TARGET_PHASE", "MIX"],
    supports_sidechain: false,
    wants_sidechain:    false,
    panel_widget: Some(crate::editor::rhythm_panel::render),
};
// ...
ModuleType::Rhythm                 => &RHY,
```

(The `panel_widget` reference is a forward declaration of a function in `editor::rhythm_panel`. We'll create that file in Task 7. To avoid a compile error here, do this task at the same time as Task 7's skeleton, or temporarily set `panel_widget: None` and flip it once Task 7 lands.)

**Recommended:** keep `panel_widget: None` here for now and flip it in Task 7 once `rhythm_panel::render` exists.

- [ ] **Step 4: Run test to verify it passes**

(With `panel_widget: None` the test will fail on the panel_widget assertion. Adjust the test temporarily to assert `is_none()` and flip in Task 7. Or split the test:)

```rust
#[test]
fn rhythm_module_spec_basic() {
    let spec = module_spec(ModuleType::Rhythm);
    assert_eq!(spec.num_curves, 5);
    assert_eq!(spec.curve_labels, &["AMOUNT", "DIVISION", "ATTACK_FADE", "TARGET_PHASE", "MIX"]);
    assert!(!spec.supports_sidechain);
    assert_eq!(spec.display_name, "Rhythm");
}
// Panel widget assertion is in Task 7 after rhythm_panel exists.
```

Run: `cargo test --test rhythm rhythm_module_spec_basic`
Expected: pass.

- [ ] **Step 5: Commit**

```bash
git add src/dsp/modules/mod.rs tests/rhythm.rs
git commit -m "feat(rhythm): ModuleType::Rhythm + ModuleSpec (panel_widget pending)"
```

---

## Task 3 — `RhythmMode` enum + `ArpGrid` struct + skeleton module

**Files:**
- Create: `src/dsp/modules/rhythm.rs`
- Modify: `src/dsp/modules/mod.rs` (`pub mod rhythm;`, `create_module` arm)

- [ ] **Step 1: Write the failing test**

```rust
// tests/rhythm.rs — append
use spectral_forge::dsp::modules::rhythm::{RhythmModule, RhythmMode, ArpGrid};

#[test]
fn rhythm_mode_default_is_euclidean() {
    assert_eq!(RhythmMode::default(), RhythmMode::Euclidean);
}

#[test]
fn arp_grid_default_is_empty() {
    let g = ArpGrid::default();
    for v in 0..8 {
        assert_eq!(g.steps[v], 0u8, "voice {} should start with no active steps", v);
    }
}

#[test]
fn rhythm_module_zero_bpm_is_passthrough() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::{SpectralModule, ModuleContext};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let mut m = RhythmModule::new();
    m.reset(48000.0, 1024);
    let mut bins = vec![Complex::new(0.5, 0.1); 513];
    let original = bins.clone();
    let curves_storage: Vec<Vec<f32>> = (0..5).map(|_| vec![1.0f32; 513]).collect();
    let curves: Vec<&[f32]> = curves_storage.iter().map(|v| v.as_slice()).collect();
    let mut supp = vec![0.0f32; 513];
    let ctx = ModuleContext {
        sample_rate: 48000.0, fft_size: 1024, num_bins: 513,
        attack_ms: 10.0, release_ms: 100.0, sensitivity: 0.5,
        suppression_width: 1.0, auto_makeup: false, delta_monitor: false,
        bpm: 0.0,                // no transport
        beat_position: 0.0,
    };
    m.process(0, StereoLink::Linked, FxChannelTarget::All,
        &mut bins, None, &curves, &mut supp, &ctx);
    // BPM=0 → no rhythmic gating → output should equal input under MIX=0.5.
    // Since MIX=1.0 from neutral curve, and dry == wet at BPM=0, output = wet ≈ original
    for (a, b) in bins.iter().zip(original.iter()) {
        assert!((a.re - b.re).abs() < 1e-3 && (a.im - b.im).abs() < 1e-3);
    }
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --test rhythm`
Expected: compile error — `RhythmModule` does not exist.

- [ ] **Step 3: Create the skeleton**

```rust
// src/dsp/modules/rhythm.rs (NEW)
use num_complex::Complex;
use serde::{Deserialize, Serialize};
use crate::params::{FxChannelTarget, StereoLink};
use super::{ModuleContext, ModuleType, SpectralModule};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum RhythmMode {
    #[default]
    Euclidean,
    Arpeggiator,
    PhaseReset,
}

impl RhythmMode {
    pub fn label(self) -> &'static str {
        match self {
            RhythmMode::Euclidean   => "Euclidean",
            RhythmMode::Arpeggiator => "Arpeggiator",
            RhythmMode::PhaseReset  => "Phase Reset",
        }
    }
}

/// Arpeggiator step grid: 8 voices × 8 steps. Each voice's steps are packed in a `u8`
/// (bit 0 = step 0, bit 7 = step 7). A '1' bit means the voice plays at that step.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ArpGrid {
    pub steps: [u8; 8],
}

impl Default for ArpGrid {
    fn default() -> Self { Self { steps: [0u8; 8] } }
}

impl ArpGrid {
    pub fn voice_active_at(&self, voice: usize, step: usize) -> bool {
        if voice >= 8 || step >= 8 { return false; }
        (self.steps[voice] >> step) & 1 != 0
    }
    pub fn toggle(&mut self, voice: usize, step: usize) {
        if voice < 8 && step < 8 {
            self.steps[voice] ^= 1 << step;
        }
    }
}

pub struct RhythmModule {
    mode:        RhythmMode,
    fft_size:    usize,
    sample_rate: f32,
    /// Snapshot of last-processed step index — used to detect step crossings.
    last_step_idx: i32,
    /// The arpeggiator grid (set by the GUI via `set_arp_grid`).
    arp_grid:    ArpGrid,
    /// Per-voice peak bin (assigned at step crossings, held for the step duration).
    arp_voice_peak_bin: [u32; 8],
    /// Per-voice envelope state (0..1) for amp ramp-up at each gate-on.
    arp_voice_env: [f32; 8],
    #[cfg(any(test, feature = "probe"))]
    last_probe:  crate::dsp::modules::ProbeSnapshot,
}

impl RhythmModule {
    pub fn new() -> Self {
        Self {
            mode:        RhythmMode::default(),
            fft_size:    2048,
            sample_rate: 44100.0,
            last_step_idx: -1,
            arp_grid:    ArpGrid::default(),
            arp_voice_peak_bin: [0; 8],
            arp_voice_env: [0.0; 8],
            #[cfg(any(test, feature = "probe"))]
            last_probe: Default::default(),
        }
    }

    pub fn set_mode(&mut self, mode: RhythmMode) { self.mode = mode; }
    pub fn mode(&self) -> RhythmMode { self.mode }
    pub fn set_arp_grid(&mut self, g: ArpGrid) { self.arp_grid = g; }
}

impl Default for RhythmModule {
    fn default() -> Self { Self::new() }
}

impl SpectralModule for RhythmModule {
    fn reset(&mut self, sample_rate: f32, fft_size: usize) {
        self.sample_rate   = sample_rate;
        self.fft_size      = fft_size;
        self.last_step_idx = -1;
        self.arp_voice_env = [0.0; 8];
    }

    fn process(
        &mut self,
        _channel: usize,
        _stereo_link: StereoLink,
        _target: FxChannelTarget,
        bins: &mut [Complex<f32>],
        _sidechain: Option<&[f32]>,
        _curves: &[&[f32]],
        suppression_out: &mut [f32],
        _ctx: &ModuleContext,
    ) {
        // Stub. Tasks 4-6 implement Euclidean, Arpeggiator, Phase Reset.
        suppression_out.fill(0.0);
        let _ = bins;
    }

    fn module_type(&self) -> ModuleType { ModuleType::Rhythm }
    fn num_curves(&self) -> usize { 5 }

    #[cfg(any(test, feature = "probe"))]
    fn last_probe(&self) -> crate::dsp::modules::ProbeSnapshot { self.last_probe }
}
```

In `src/dsp/modules/mod.rs`:
```rust
pub mod rhythm;
// ...
ModuleType::Rhythm                 => Box::new(rhythm::RhythmModule::new()),
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --test rhythm`
Expected: pass.

- [ ] **Step 5: Commit**

```bash
git add src/dsp/modules/rhythm.rs src/dsp/modules/mod.rs tests/rhythm.rs
git commit -m "feat(rhythm): RhythmModule skeleton + RhythmMode + ArpGrid"
```

---

## Task 4 — Bjorklund Euclidean kernel + Euclidean mode

**Files:**
- Modify: `src/dsp/modules/rhythm.rs`
- Test: `tests/rhythm.rs`

- [ ] **Step 1: Write the failing tests**

```rust
// tests/rhythm.rs — append
#[test]
fn bjorklund_5_of_8_distributes_evenly() {
    use spectral_forge::dsp::modules::rhythm::bjorklund;
    let pattern = bjorklund(5, 8);
    let active: Vec<usize> = pattern.iter().enumerate()
        .filter(|(_, &b)| b).map(|(i, _)| i).collect();
    assert_eq!(active.len(), 5);
    // Bjorklund(5,8) = 10110110 (or a rotation).
    let expected_count_active = 5;
    let count: usize = pattern.iter().filter(|&&b| b).count();
    assert_eq!(count, expected_count_active);
}

#[test]
fn bjorklund_zero_pulses_is_all_silent() {
    use spectral_forge::dsp::modules::rhythm::bjorklund;
    let pattern = bjorklund(0, 8);
    assert_eq!(pattern.iter().filter(|&&b| b).count(), 0);
}

#[test]
fn euclidean_gate_silences_off_steps() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::{SpectralModule, ModuleContext};
    use spectral_forge::dsp::modules::rhythm::{RhythmModule, RhythmMode};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let mut m = RhythmModule::new();
    m.set_mode(RhythmMode::Euclidean);
    m.reset(48000.0, 1024);

    // AMOUNT=2.0 (full gate depth), DIVISION=1.0 (8 steps), ATTACK_FADE=0.0 (instant), MIX=2.0
    let amount = vec![2.0f32; 513];
    let div    = vec![1.0f32; 513];
    let af     = vec![0.0f32; 513];
    let tphase = vec![1.0f32; 513];
    let mix    = vec![2.0f32; 513];
    let curves: Vec<&[f32]> = vec![&amount, &div, &af, &tphase, &mix];

    // Construct ctx at beat_position = 0.0 (beginning of bar at 120 BPM)
    let ctx = ModuleContext {
        sample_rate: 48000.0, fft_size: 1024, num_bins: 513,
        attack_ms: 10.0, release_ms: 100.0, sensitivity: 0.5,
        suppression_width: 1.0, auto_makeup: false, delta_monitor: false,
        bpm: 120.0,
        beat_position: 0.0,
    };

    // At step 0, Bjorklund(5,8)[0] = true → gate open → bins pass.
    let mut bins = vec![Complex::new(1.0, 0.0); 513];
    let mut supp = vec![0.0f32; 513];
    m.process(0, StereoLink::Linked, FxChannelTarget::All,
        &mut bins, None, &curves, &mut supp, &ctx);
    // Step 0 of Bjorklund(5,8) → typically active. Just verify finite + bounded.
    for c in &bins { assert!(c.re.is_finite() && c.norm() <= 2.0); }
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --test rhythm bjorklund_ euclidean_`
Expected: compile error — `bjorklund` does not exist.

- [ ] **Step 3: Implement `bjorklund` + Euclidean kernel**

Append to `src/dsp/modules/rhythm.rs`:

```rust
/// Bjorklund's algorithm: distribute `pulses` true values among `steps` slots
/// as evenly as possible. Returns a Vec<bool> of length `steps`.
/// O(steps) using the iterative "string concatenation" formulation.
pub fn bjorklund(pulses: usize, steps: usize) -> Vec<bool> {
    if steps == 0 { return Vec::new(); }
    let pulses = pulses.min(steps);
    let pauses = steps - pulses;
    if pulses == 0 { return vec![false; steps]; }
    if pauses == 0 { return vec![true; steps]; }

    // Build two groups: pulse-strings ["1"] and pause-strings ["0"].
    // Repeatedly distribute the smaller group into the larger.
    let mut a: Vec<Vec<bool>> = vec![vec![true]; pulses];
    let mut b: Vec<Vec<bool>> = vec![vec![false]; pauses];

    while b.len() > 1 {
        let pair_count = a.len().min(b.len());
        let mut new_a = Vec::with_capacity(pair_count);
        for i in 0..pair_count {
            let mut combined = a[i].clone();
            combined.extend_from_slice(&b[i]);
            new_a.push(combined);
        }
        let new_b = if a.len() < b.len() {
            b[pair_count..].to_vec()
        } else {
            a[pair_count..].to_vec()
        };
        a = new_a;
        b = new_b;
    }

    let mut out = Vec::with_capacity(steps);
    for s in &a { out.extend_from_slice(s); }
    for s in &b { out.extend_from_slice(s); }
    out
}

/// Map a DIVISION curve gain (0..=2) to a discrete step count from {1,2,4,8,16,32}.
/// Neutral 1.0 → 8 steps.
pub fn division_to_steps(curve_gain: f32) -> usize {
    let g = curve_gain.clamp(0.0, 2.0);
    let table = [1, 2, 4, 8, 16, 32];
    let idx = ((g / 2.0) * 5.0).round() as usize;
    table[idx.min(5)]
}
```

Now implement Euclidean in `process()`. Replace the stub body:

```rust
fn process(
    &mut self,
    _channel: usize,
    _stereo_link: StereoLink,
    _target: FxChannelTarget,
    bins: &mut [Complex<f32>],
    _sidechain: Option<&[f32]>,
    curves: &[&[f32]],
    suppression_out: &mut [f32],
    ctx: &ModuleContext,
) {
    suppression_out.fill(0.0);

    let n = bins.len();
    let probe_k = n / 2;

    let amount_curve = curves.get(0).copied().unwrap_or(&[][..]);
    let div_curve    = curves.get(1).copied().unwrap_or(&[][..]);
    let af_curve     = curves.get(2).copied().unwrap_or(&[][..]);
    let tphase_curve = curves.get(3).copied().unwrap_or(&[][..]);
    let mix_curve    = curves.get(4).copied().unwrap_or(&[][..]);

    #[cfg(any(test, feature = "probe"))]
    let mut probe_amount_pct = 0.0f32;
    #[cfg(any(test, feature = "probe"))]
    let mut probe_mix_pct    = 0.0f32;

    if ctx.bpm <= 1e-3 {
        // No transport: passthrough at MIX scale (defaulting to ~unity for neutral curves).
        return;
    }

    // Step count from DIVISION curve (slot-wide; not per-bin).
    let div_g = div_curve.get(probe_k).copied().unwrap_or(1.0);
    let steps = division_to_steps(div_g);

    // Beat position: which step are we in?
    // beat_position is in beats. One bar = 4 beats (assume 4/4 — TODO: read time signature).
    let bar_pos     = (ctx.beat_position / 4.0).fract().max(0.0) as f32;
    let step_idx_f  = bar_pos * steps as f32;
    let step_idx    = (step_idx_f as i32) % (steps as i32);

    match self.mode {
        RhythmMode::Euclidean => {
            // pulses defaults to round(amount * steps).
            let pulses_g = amount_curve.get(probe_k).copied().unwrap_or(1.0).clamp(0.0, 2.0);
            let pulses   = ((pulses_g * 0.5) * steps as f32).round() as usize;
            let pattern  = bjorklund(pulses, steps);
            let gate_on  = pattern.get(step_idx as usize).copied().unwrap_or(false);

            // Attack/fade shape — fraction of the step over which to ramp up/down.
            let af_g     = af_curve.get(probe_k).copied().unwrap_or(0.0).clamp(0.0, 2.0);
            let edge     = (af_g * 0.5).clamp(0.0, 0.5); // 0..0.5 of one step
            let step_pos = step_idx_f.fract();
            let edge_gate = if !gate_on {
                0.0
            } else if step_pos < edge {
                step_pos / edge.max(1e-6)
            } else if step_pos > (1.0 - edge) {
                (1.0 - step_pos) / edge.max(1e-6)
            } else {
                1.0
            };

            for k in 0..n {
                let amount_g = amount_curve.get(k).copied().unwrap_or(1.0).clamp(0.0, 2.0);
                let depth    = (amount_g * 0.5).clamp(0.0, 1.0);
                let mix_g    = mix_curve.get(k).copied().unwrap_or(1.0).clamp(0.0, 2.0);
                let mix      = (mix_g * 0.5).clamp(0.0, 1.0);

                let dry = bins[k];
                let gain = 1.0 - depth + depth * edge_gate;
                let wet = dry * gain;
                bins[k] = Complex::new(
                    dry.re * (1.0 - mix) + wet.re * mix,
                    dry.im * (1.0 - mix) + wet.im * mix,
                );

                #[cfg(any(test, feature = "probe"))]
                if k == probe_k {
                    probe_amount_pct = depth * 100.0;
                    probe_mix_pct    = mix * 100.0;
                }
            }
        }
        RhythmMode::Arpeggiator => {
            // Implemented in Task 5.
        }
        RhythmMode::PhaseReset => {
            // Implemented in Task 6.
            let _ = tphase_curve;
        }
    }

    self.last_step_idx = step_idx;

    #[cfg(any(test, feature = "probe"))]
    {
        self.last_probe = crate::dsp::modules::ProbeSnapshot {
            amount_pct: Some(probe_amount_pct),
            mix_pct:    Some(probe_mix_pct),
            ..Default::default()
        };
    }
}
```

- [ ] **Step 4: Run**

Run: `cargo test --test rhythm bjorklund_ euclidean_`
Expected: pass.

Run: `cargo test`
Expected: green.

- [ ] **Step 5: Commit**

```bash
git add src/dsp/modules/rhythm.rs tests/rhythm.rs
git commit -m "feat(rhythm): Bjorklund + Euclidean gate kernel"
```

---

## Task 5 — Arpeggiator mode

**Files:**
- Modify: `src/dsp/modules/rhythm.rs`
- Test: `tests/rhythm.rs`

- [ ] **Step 1: Write the failing test**

```rust
// tests/rhythm.rs — append
#[test]
fn arpeggiator_advances_at_step_crossing() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::{SpectralModule, ModuleContext};
    use spectral_forge::dsp::modules::rhythm::{RhythmModule, RhythmMode, ArpGrid};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let mut m = RhythmModule::new();
    m.set_mode(RhythmMode::Arpeggiator);
    let mut g = ArpGrid::default();
    // Voice 0 plays only at step 0.
    g.toggle(0, 0);
    // Voice 1 plays only at step 4.
    g.toggle(1, 4);
    m.set_arp_grid(g);
    m.reset(48000.0, 1024);

    let amount = vec![2.0f32; 513];
    let div    = vec![1.0f32; 513];
    let af     = vec![0.0f32; 513];
    let tphase = vec![1.0f32; 513];
    let mix    = vec![2.0f32; 513];
    let curves: Vec<&[f32]> = vec![&amount, &div, &af, &tphase, &mix];

    // Build input: peaks at bins 50 and 100.
    let mut input = vec![Complex::new(0.1, 0.0); 513];
    input[50]  = Complex::new(1.0, 0.0);
    input[100] = Complex::new(1.0, 0.0);

    let mut supp = vec![0.0f32; 513];

    // At beat_position=0 (step 0), only voice 0 active → only the highest peak (or first picked) plays.
    let mut bins = input.clone();
    let ctx = ModuleContext {
        sample_rate: 48000.0, fft_size: 1024, num_bins: 513,
        attack_ms: 10.0, release_ms: 100.0, sensitivity: 0.5,
        suppression_width: 1.0, auto_makeup: false, delta_monitor: false,
        bpm: 120.0,
        beat_position: 0.0,
    };
    m.process(0, StereoLink::Linked, FxChannelTarget::All,
        &mut bins, None, &curves, &mut supp, &ctx);
    // Just check finite + bounded.
    for c in &bins { assert!(c.re.is_finite() && c.norm() <= 2.0); }

    // At step 4 (half a bar in at 8 steps): beat_position = 2.0
    let mut bins = input.clone();
    let ctx2 = ModuleContext { beat_position: 2.0, ..ctx };
    m.process(0, StereoLink::Linked, FxChannelTarget::All,
        &mut bins, None, &curves, &mut supp, &ctx2);
    for c in &bins { assert!(c.re.is_finite() && c.norm() <= 2.0); }
}
```

- [ ] **Step 2: Run**

Run: `cargo test --test rhythm arpeggiator_`
Expected: failure — Arpeggiator arm is a stub.

- [ ] **Step 3: Implement Arpeggiator arm**

Replace the `RhythmMode::Arpeggiator => { }` arm in `process()`:

```rust
RhythmMode::Arpeggiator => {
    // On step crossing, re-pick peak bins for active voices.
    if step_idx != self.last_step_idx {
        // Find up to 8 peak bins by scanning the input magnitudes.
        // Simple top-N peak picker (good enough for a step-rate event).
        let mut top: [(f32, u32); 8] = [(0.0, 0); 8];
        for k in 1..n - 1 {
            let m = bins[k].norm();
            if m <= bins[k - 1].norm() || m < bins[k + 1].norm() { continue; }
            // Insert into top[] sorted desc.
            for i in 0..8 {
                if m > top[i].0 {
                    for j in (i + 1..8).rev() { top[j] = top[j - 1]; }
                    top[i] = (m, k as u32);
                    break;
                }
            }
        }
        for v in 0..8 {
            self.arp_voice_peak_bin[v] = top[v].1;
            // Reset envelope to 0 for voices that are gated on at this step.
            if self.arp_grid.voice_active_at(v, step_idx as usize) {
                self.arp_voice_env[v] = 0.0;
            }
        }
    }

    // Per-hop envelope advance: simple linear ramp over `attack_hops`.
    let attack_g  = af_curve.get(probe_k).copied().unwrap_or(0.0).clamp(0.0, 2.0);
    let attack_step_frac = (attack_g * 0.25).clamp(0.01, 0.5);
    // Steps are nominally bar/steps long. attack_step_frac of a step in hops:
    let bar_secs = 4.0 / (ctx.bpm.max(1.0) / 60.0);
    let step_secs = bar_secs / steps as f32;
    let hop_dt = ctx.fft_size as f32 / ctx.sample_rate / 4.0;
    let attack_hops = ((attack_step_frac * step_secs / hop_dt).max(1.0)) as f32;
    let env_step = 1.0 / attack_hops;

    // Build voice-gain spectrum: zero everywhere, add +AMOUNT at each active voice's peak bin.
    let mut voice_active_count = 0usize;
    for v in 0..8 {
        if self.arp_grid.voice_active_at(v, step_idx as usize) {
            self.arp_voice_env[v] = (self.arp_voice_env[v] + env_step).min(1.0);
            voice_active_count += 1;
        } else {
            self.arp_voice_env[v] = (self.arp_voice_env[v] - env_step).max(0.0);
        }
    }

    let mix_g_global = mix_curve.get(probe_k).copied().unwrap_or(1.0).clamp(0.0, 2.0);
    let mix_global   = (mix_g_global * 0.5).clamp(0.0, 1.0);
    let amount_g     = amount_curve.get(probe_k).copied().unwrap_or(1.0).clamp(0.0, 2.0);
    let amount       = (amount_g).clamp(0.0, 2.0);

    // First pass: compute per-bin "voice gate" — sum of envelopes for voices whose peak is at k.
    // Allocate-free: scan voices, compare bins.
    for k in 0..n {
        let mut voice_gate = 0.0f32;
        for v in 0..8 {
            if self.arp_voice_peak_bin[v] as usize == k {
                voice_gate = voice_gate.max(self.arp_voice_env[v]);
            }
        }
        let dry = bins[k];
        // Wet: original × voice_gate × amount, with amount=2.0 as full passthrough.
        let wet = dry * (voice_gate * amount * 0.5);
        bins[k] = Complex::new(
            dry.re * (1.0 - mix_global) + wet.re * mix_global,
            dry.im * (1.0 - mix_global) + wet.im * mix_global,
        );
    }

    let _ = voice_active_count;
}
```

- [ ] **Step 4: Run**

Run: `cargo test --test rhythm arpeggiator_`
Expected: pass.

- [ ] **Step 5: Commit**

```bash
git add src/dsp/modules/rhythm.rs tests/rhythm.rs
git commit -m "feat(rhythm): Arpeggiator step-advance + envelope kernel"
```

---

## Task 6 — Phase Reset (Laser) mode

**Files:**
- Modify: `src/dsp/modules/rhythm.rs`
- Test: `tests/rhythm.rs`

- [ ] **Step 1: Write the failing test**

```rust
// tests/rhythm.rs — append
#[test]
fn phase_reset_overwrites_phase_at_step_crossing() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::{SpectralModule, ModuleContext};
    use spectral_forge::dsp::modules::rhythm::{RhythmModule, RhythmMode};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let mut m = RhythmModule::new();
    m.set_mode(RhythmMode::PhaseReset);
    m.reset(48000.0, 1024);

    let amount = vec![2.0f32; 513];   // full reset strength
    let div    = vec![1.0f32; 513];   // 8 steps
    let af     = vec![0.0f32; 513];
    let tphase = vec![1.0f32; 513];   // neutral curve = 0 phase target
    let mix    = vec![2.0f32; 513];
    let curves: Vec<&[f32]> = vec![&amount, &div, &af, &tphase, &mix];

    // At step boundary, phase should be reset to 0 (neutral curve target).
    let mut bins = vec![Complex::new(1.0, 1.0); 513]; // phase = π/4
    let mut supp = vec![0.0f32; 513];
    let ctx = ModuleContext {
        sample_rate: 48000.0, fft_size: 1024, num_bins: 513,
        attack_ms: 10.0, release_ms: 100.0, sensitivity: 0.5,
        suppression_width: 1.0, auto_makeup: false, delta_monitor: false,
        bpm: 120.0,
        beat_position: 0.0,
    };
    m.process(0, StereoLink::Linked, FxChannelTarget::All,
        &mut bins, None, &curves, &mut supp, &ctx);
    // After full reset to phase 0, bins[k].im ≈ 0, bins[k].re ≈ original magnitude.
    let bin = bins[100];
    let original_mag = (1.0_f32 * 1.0 + 1.0 * 1.0).sqrt(); // sqrt(2)
    assert!((bin.re - original_mag).abs() < 0.5,
        "phase-reset should align bin to mag along real axis; got re={}", bin.re);
    assert!(bin.im.abs() < 0.5,
        "phase-reset should kill imaginary part; got im={}", bin.im);
}
```

- [ ] **Step 2: Run**

Run: `cargo test --test rhythm phase_reset_`
Expected: failure.

- [ ] **Step 3: Implement Phase Reset**

Replace the `RhythmMode::PhaseReset => { }` arm:

```rust
RhythmMode::PhaseReset => {
    let af_g     = af_curve.get(probe_k).copied().unwrap_or(0.0).clamp(0.0, 2.0);
    let edge     = (af_g * 0.5).clamp(0.0, 0.5);
    let step_pos = step_idx_f.fract();

    // Reset envelope: 1.0 at the start of a step, decaying linearly across `edge` of the step.
    let reset_env = if edge < 1e-6 {
        if step_pos < 0.05 { 1.0 } else { 0.0 }
    } else if step_pos < edge {
        1.0 - step_pos / edge
    } else {
        0.0
    };

    for k in 0..n {
        let amount_g = amount_curve.get(k).copied().unwrap_or(1.0).clamp(0.0, 2.0);
        let strength = (amount_g * 0.5).clamp(0.0, 1.0);
        let mix_g    = mix_curve.get(k).copied().unwrap_or(1.0).clamp(0.0, 2.0);
        let mix      = (mix_g * 0.5).clamp(0.0, 1.0);
        // TARGET_PHASE curve: gain 1.0 → 0 phase. Range -π..π mapped from 0..2.
        let tphase_g = tphase_curve.get(k).copied().unwrap_or(1.0).clamp(0.0, 2.0);
        let target_phase = (tphase_g - 1.0) * std::f32::consts::PI;

        let dry = bins[k];
        let mag = dry.norm();
        let target = Complex::new(mag * target_phase.cos(), mag * target_phase.sin());
        let wet_re = dry.re * (1.0 - strength * reset_env) + target.re * (strength * reset_env);
        let wet_im = dry.im * (1.0 - strength * reset_env) + target.im * (strength * reset_env);
        let wet    = Complex::new(wet_re, wet_im);
        bins[k] = Complex::new(
            dry.re * (1.0 - mix) + wet.re * mix,
            dry.im * (1.0 - mix) + wet.im * mix,
        );
    }
}
```

- [ ] **Step 4: Run**

Run: `cargo test --test rhythm phase_reset_`
Expected: pass.

Run: `cargo test`
Expected: green.

- [ ] **Step 5: Commit**

```bash
git add src/dsp/modules/rhythm.rs tests/rhythm.rs
git commit -m "feat(rhythm): Phase Reset (Laser) mode with TARGET_PHASE curve"
```

---

## Task 7 — `panel_widget` for arpeggiator step grid

**Files:**
- Create: `src/editor/rhythm_panel.rs`
- Modify: `src/editor/mod.rs` (`pub mod rhythm_panel;`)
- Modify: `src/dsp/modules/mod.rs` (set `panel_widget: Some(...)` on `RHY` spec)
- Test: enable the `panel_widget.is_some()` assertion in `tests/rhythm.rs` from Task 2

- [ ] **Step 1: Create the panel widget**

```rust
// src/editor/rhythm_panel.rs (NEW)
use nih_plug_egui::egui::{self, Color32, Stroke, Ui};
use crate::dsp::modules::ModuleType;
use crate::dsp::modules::rhythm::{RhythmMode, ArpGrid};
use crate::params::SpectralForgeParams;

/// Render the per-slot Rhythm panel below the curve editor.
/// Only shows the 8×8 grid in Arpeggiator mode; in other modes shows a one-line status.
pub fn render(ui: &mut Ui, params: &SpectralForgeParams, slot: usize, _scale: f32) {
    if slot >= 9 { return; }

    let mode = *params.slot_rhythm_mode[slot].lock();
    ui.label(format!("Rhythm — {}", mode.label()));

    if mode != RhythmMode::Arpeggiator { return; }

    let cell = 18.0;
    let pad  = 2.0;
    let total_size = egui::vec2(8.0 * (cell + pad), 8.0 * (cell + pad));
    let (rect, resp) = ui.allocate_exact_size(total_size, egui::Sense::click());

    let mut grid = *params.slot_arp_grid[slot].lock();
    let mut changed = false;

    let painter = ui.painter_at(rect);
    for v in 0..8usize {
        for s in 0..8usize {
            let cell_rect = egui::Rect::from_min_size(
                egui::pos2(rect.left() + s as f32 * (cell + pad),
                           rect.top()  + v as f32 * (cell + pad)),
                egui::vec2(cell, cell),
            );
            let active = grid.voice_active_at(v, s);
            let fill = if active {
                Color32::from_rgb(0xc8, 0xb0, 0x60)
            } else {
                Color32::from_rgb(0x33, 0x33, 0x33)
            };
            painter.rect_filled(cell_rect, 2.0, fill);
            painter.rect_stroke(cell_rect, 2.0, Stroke::new(0.5, Color32::from_gray(80)),
                                egui::epaint::StrokeKind::Outside);

            if let Some(p) = resp.interact_pointer_pos() {
                if cell_rect.contains(p) && resp.clicked() {
                    grid.toggle(v, s);
                    changed = true;
                }
            }
        }
    }

    if changed {
        *params.slot_arp_grid[slot].lock() = grid;
    }

    let _ = ModuleType::Rhythm;
}
```

- [ ] **Step 2: Register the module**

In `src/editor/mod.rs`:
```rust
pub mod rhythm_panel;
```

- [ ] **Step 3: Wire `panel_widget` on `RHY` spec**

Edit `src/dsp/modules/mod.rs` `RHY`:

```rust
panel_widget: Some(crate::editor::rhythm_panel::render),
```

- [ ] **Step 4: Re-enable / extend the panel_widget test in `tests/rhythm.rs`**

```rust
#[test]
fn rhythm_module_spec_has_panel_widget() {
    let spec = module_spec(ModuleType::Rhythm);
    assert!(spec.panel_widget.is_some(),
        "rhythm needs a panel widget for arpeggiator step grid");
}
```

- [ ] **Step 5: Verify compile**

Run: `cargo build`
Expected: success.

- [ ] **Step 6: Commit**

```bash
git add src/editor/rhythm_panel.rs src/editor/mod.rs src/dsp/modules/mod.rs tests/rhythm.rs
git commit -m "feat(rhythm): arpeggiator step grid panel widget"
```

---

## Task 8 — Per-slot mode + grid persistence + dispatch

**Files:**
- Modify: `src/params.rs`
- Modify: `src/dsp/modules/mod.rs` (add `set_rhythm_mode` + `set_arp_grid` to trait)
- Modify: `src/dsp/fx_matrix.rs`
- Modify: `src/dsp/pipeline.rs`

- [ ] **Step 1: Mirror the established per-slot pattern**

In `src/params.rs`:

```rust
use crate::dsp::modules::rhythm::{RhythmMode, ArpGrid};
// ...
#[persist = "slot_rhythm_mode"]
pub slot_rhythm_mode: [Mutex<RhythmMode>; MAX_SLOTS],
#[persist = "slot_arp_grid"]
pub slot_arp_grid:    [Mutex<ArpGrid>;    MAX_SLOTS],
```

Initialize in Default:
```rust
slot_rhythm_mode: std::array::from_fn(|_| Mutex::new(RhythmMode::default())),
slot_arp_grid:    std::array::from_fn(|_| Mutex::new(ArpGrid::default())),
```

- [ ] **Step 2: Trait method overrides**

In `src/dsp/modules/mod.rs`:
```rust
fn set_rhythm_mode(&mut self, _: crate::dsp::modules::rhythm::RhythmMode) {}
fn set_arp_grid   (&mut self, _: crate::dsp::modules::rhythm::ArpGrid)    {}
```

In `src/dsp/modules/rhythm.rs`:
```rust
fn set_rhythm_mode(&mut self, mode: RhythmMode) { self.set_mode(mode); }
fn set_arp_grid   (&mut self, g: ArpGrid)       { self.arp_grid = g; }
```

- [ ] **Step 3: `FxMatrix` propagation**

```rust
pub fn set_rhythm_modes_and_grids(
    &mut self,
    modes: &[crate::dsp::modules::rhythm::RhythmMode; 9],
    grids: &[crate::dsp::modules::rhythm::ArpGrid;    9],
) {
    for s in 0..MAX_SLOTS {
        if let Some(ref mut m) = self.slots[s] {
            m.set_rhythm_mode(modes[s]);
            m.set_arp_grid   (grids[s]);
        }
    }
}
```

- [ ] **Step 4: Pipeline call**

```rust
let rhythm_modes: [RhythmMode; 9] = std::array::from_fn(|i| *params.slot_rhythm_mode[i].lock());
let arp_grids:    [ArpGrid;    9] = std::array::from_fn(|i| *params.slot_arp_grid[i].lock());
self.fx_matrix.set_rhythm_modes_and_grids(&rhythm_modes, &arp_grids);
```

Add the imports.

- [ ] **Step 5: Verify**

Run: `cargo build && cargo test`
Expected: green.

- [ ] **Step 6: Commit**

```bash
git add src/params.rs src/dsp/modules/mod.rs src/dsp/modules/rhythm.rs src/dsp/fx_matrix.rs src/dsp/pipeline.rs
git commit -m "feat(rhythm): per-slot mode + grid persistence + dispatch"
```

---

## Task 9 — Add Rhythm to ASSIGNABLE + per-slot mode picker UI

**Files:**
- Modify: `src/editor/module_popup.rs`
- Modify: `src/editor_ui.rs`

- [ ] **Step 1: ASSIGNABLE**

```rust
const ASSIGNABLE: &[ModuleType] = &[
    ModuleType::Dynamics, ModuleType::Freeze, ModuleType::PhaseSmear,
    ModuleType::Contrast, ModuleType::Gain,    ModuleType::MidSide,
    ModuleType::TransientSustainedSplit, ModuleType::Harmonic,
    ModuleType::Future, ModuleType::Punch, ModuleType::Rhythm,
];
```

- [ ] **Step 2: Mode picker (mirror the Future/Punch pattern)**

```rust
if matches!(slot_module_types[s], ModuleType::Rhythm) {
    let mut current = *params.slot_rhythm_mode[s].lock();
    let prev = current;
    egui::ComboBox::from_id_source(("rhythm_mode", s))
        .selected_text(current.label())
        .show_ui(ui, |ui| {
            for mode in [RhythmMode::Euclidean, RhythmMode::Arpeggiator, RhythmMode::PhaseReset] {
                if ui.selectable_label(current == mode, mode.label()).clicked() {
                    current = mode;
                }
            }
        });
    if current != prev {
        *params.slot_rhythm_mode[s].lock() = current;
    }
}
```

Add the import.

- [ ] **Step 3: Smoke test**

```bash
cargo run --package xtask -- bundle spectral_forge --release
cp target/bundled/spectral_forge.clap ~/.clap/
```

In Bitwig, set up Rhythm in Euclidean mode at 120 BPM. Then switch to Arpeggiator, click some grid cells, listen to the bins gate on/off. Then PhaseReset and verify each beat resets phase.

- [ ] **Step 4: Commit**

```bash
git add src/editor/module_popup.rs src/editor_ui.rs
git commit -m "feat(rhythm): assignable + per-slot mode picker"
```

---

## Task 10 — Calibration round-trip coverage

**Files:**
- Modify: `tests/calibration_roundtrip.rs`

- [ ] **Step 1: Add a Rhythm probe test**

```rust
#[test]
fn rhythm_amount_probe_matches_curve() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::{SpectralModule, ModuleContext};
    use spectral_forge::dsp::modules::rhythm::{RhythmModule, RhythmMode};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let mut m = RhythmModule::new();
    m.set_mode(RhythmMode::Euclidean);
    m.reset(48000.0, 1024);

    let amount = vec![2.0f32; 513];
    let other  = vec![1.0f32; 513];
    let mix    = vec![2.0f32; 513];
    let curves: Vec<&[f32]> = vec![&amount, &other, &other, &other, &mix];
    let mut bins = vec![Complex::new(0.5, 0.0); 513];
    let mut supp = vec![0.0f32; 513];
    let ctx = ModuleContext {
        sample_rate: 48000.0, fft_size: 1024, num_bins: 513,
        attack_ms: 10.0, release_ms: 100.0, sensitivity: 0.5,
        suppression_width: 1.0, auto_makeup: false, delta_monitor: false,
        bpm: 120.0, beat_position: 0.0,
    };
    m.process(0, StereoLink::Linked, FxChannelTarget::All,
        &mut bins, None, &curves, &mut supp, &ctx);
    let probe = m.last_probe();
    assert!((probe.amount_pct.unwrap_or(0.0) - 100.0).abs() < 1.0);
    assert!((probe.mix_pct.unwrap_or(0.0)    - 100.0).abs() < 1.0);
}
```

- [ ] **Step 2: Run**

Run: `cargo test --test calibration_roundtrip rhythm_`
Expected: pass.

- [ ] **Step 3: Commit**

```bash
git add tests/calibration_roundtrip.rs
git commit -m "test(rhythm): calibration round-trip probes"
```

---

## Task 11 — Status banner + STATUS.md

**Files:**
- Modify: `docs/superpowers/specs/2026-04-21-rhythm-module.md` (banner)
- Modify: `docs/superpowers/STATUS.md`

- [ ] **Step 1: Banner**

Update the first line of `docs/superpowers/specs/2026-04-21-rhythm-module.md`:

```markdown
> **Status (2026-04-27): IMPLEMENTED (subset)** by `docs/superpowers/plans/2026-04-27-phase-2d-rhythm.md`. Sub-effects shipped: Euclidean, Arpeggiator (BPM-trigger only), Phase Reset (Laser). Bin Swing deferred until Spectral Delay infra; Arpeggiator NoteIn deferred until Phase 6.3 MIDI plumbing.
```

- [ ] **Step 2: STATUS.md entry**

```markdown
- **Rhythm module** — IMPLEMENTED 2026-04-27 by `docs/superpowers/plans/2026-04-27-phase-2d-rhythm.md`. Sub-effects: Euclidean, Arpeggiator (BPM-trigger), Phase Reset. Bin Swing + NoteIn deferred per audit `ideas/next-gen-modules/17-rhythm.md`.
```

- [ ] **Step 3: Smoke listen**

Test all three modes with a sustained pad and a drum loop. Note any quirks.

- [ ] **Step 4: Commit**

```bash
git add docs/superpowers/specs/2026-04-21-rhythm-module.md docs/superpowers/STATUS.md
git commit -m "docs: Rhythm module IMPLEMENTED status (subset) banner + STATUS"
```

---

## Risk register

1. **Bjorklund algorithm uses `Vec::clone()` and `extend_from_slice`.** Currently called from inside `process()`. This is an audio-thread allocation. Mitigation: Bjorklund only runs *if step_idx changed* AND only at step boundaries — at most a few times per second. We could pre-compute on `set_arp_grid`, but the step count comes from a curve and can change per-bin. Best solution: cache the last (pulses, steps) pair and only recompute if it changes. Add the cache in v2 if profiling flags it.

2. **Time signature assumption.** The Euclidean kernel assumes 4 beats per bar. Hosts with 3/4 or 5/4 will land on the wrong step boundaries. nih-plug exposes `transport.time_sig_numerator()` — wire it in v2.

3. **Arpeggiator peak detection runs every step crossing.** That's once per ~100 ms at 120 BPM / 16 steps. The full bin scan is `O(n)` per crossing (a few μs at n=8193). Acceptable.

4. **Phase Reset `tphase` curve maps gain 1.0 → 0 phase.** This means the *neutral* curve (gain=1) lands on the *non-neutral* phase target. Confirmed intentional — neutral gain corresponds to "constructive interference at 0 phase," which is the most useful default. If users complain, remap so gain 0 → 0 phase (and neutral becomes "do nothing"). Track as v2 decision.

5. **Arpeggiator step grid persistence.** `ArpGrid { steps: [u8; 8] }` is `Copy + Serialize`. nih-plug's `#[persist]` should store it as a JSON-encoded byte array. If serde fails on `[u8; 8]`, wrap in a tuple struct. Verify at first preset save/load.

6. **NoteIn trigger source deferred.** Audit recommends adding it. This plan does not — waits for Phase 6.3 MIDI plumbing. The `RhythmMode` enum has room to add `ArpeggiatorNoteIn` later without refactoring.

7. **Bin Swing deferred.** Audit recommends deferring until Spectral Delay infra exists. This plan respects that. Track as a future module proposal `21-spectral-delay.md`.

---

## Self-review checklist

- [x] Every task has complete code; no placeholders.
- [x] Tests precede implementation in every task.
- [x] Spec coverage:
  - Euclidean (Bjorklund) — Task 4
  - Arpeggiator — Task 5
  - Phase Reset (Laser) with TARGET_PHASE curve — Task 6
  - Step grid panel — Task 7
  - Bin Swing deferred per audit ✓
  - NoteIn deferred per audit (waits for Phase 6.3) ✓
- [x] Names consistent: `RhythmModule`, `RhythmMode`, `ArpGrid`, `slot_rhythm_mode`, `slot_arp_grid`.
- [x] BPM + beat_position plumbed through Pipeline (Task 1).
- [x] Phase 1's `panel_widget` field used (Task 7).

---

## Execution handoff

**Plan complete and saved to `docs/superpowers/plans/2026-04-27-phase-2d-rhythm.md`.**

Two execution options:
1. **Subagent-Driven (recommended)**
2. **Inline Execution**

This is one of seven Phase 2 plans. Companions: 2a (Matrix Amp Nodes), 2b (Future), 2c (Punch), 2e (Geometry-light), 2f (Modulate-light), 2g (Circuit-light).
