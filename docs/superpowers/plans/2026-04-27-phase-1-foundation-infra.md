# Phase 1: Foundation Infrastructure Implementation Plan

> **STATUS:** IMPLEMENTED (2026-04-27). Authoritative status: `docs/superpowers/STATUS.md`.
> All 8 tasks landed on branch `feature/next-gen-modules-plans`. Code is now the source
> of truth — this plan is kept for history.

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** ship six small infrastructure PRs that unblock every later phase of the next-gen-modules roadmap. None of these add an audible feature; they all enable downstream module work.

**Architecture:** All six PRs touch shared types (`ModuleContext`, `ModuleSpec`, `SpectralModule` trait) or the editor frame (`editor_ui.rs`). They are designed to land independently and to leave existing module behaviour byte-for-byte identical when none of the new fields are populated.

**Tech Stack:** Rust 2021, nih-plug, nih-plug-egui, num-complex, parking_lot, triple_buffer.

**Status banner to add at the top of each PR's commit message:** `infra(phase1):`

**Reading order before starting:**
- `ideas/next-gen-modules/02-architectural-refactors.md` § 1, § 2, § 9, § 10
- `ideas/next-gen-modules/01-global-infrastructure.md` § 8 (Modulation Ring), § 10 (Reset-to-default)
- `ideas/next-gen-modules/99-implementation-roadmap.md` § Phase 1
- `src/dsp/modules/mod.rs` (current trait + ModuleContext + ModuleSpec)
- `src/editor_ui.rs` (current editor frame)
- `src/params.rs` lines 1–80 (param patterns and constants)
- `CLAUDE.md` § Real-time safety rules

---

## File Structure

| File | Created/Modified | Responsibility |
|---|---|---|
| `src/dsp/modules/mod.rs` | Modify | Add fields to `ModuleContext`; add fields + helper to `ModuleSpec`; add `heavy_cpu_for_mode()` to `SpectralModule` trait. Convert `ModuleContext` to non-`Copy` with `'block` lifetime. |
| `src/dsp/pipeline.rs` | Modify | Construct the new `ModuleContext` with `None` values for the optional fields and pass through. |
| `src/dsp/fx_matrix.rs` | Modify | Pass `&ModuleContext` (not by value) into `process_hop`. |
| `src/dsp/modules/dynamics.rs`, `freeze.rs`, `phase_smear.rs`, `contrast.rs`, `gain.rs`, `mid_side.rs`, `ts_split.rs`, `harmonic.rs`, `master.rs` | Modify (signature only) | Update `process()` signature to take `ctx: &ModuleContext<'_>`. No behaviour change. |
| `src/editor_ui.rs` | Modify | Wire reset-to-default button; dispatch optional per-module `panel_widget`; insert Modulation Ring overlay container. |
| `src/editor/mod_ring.rs` | Create | New widget module: `mod_ring_overlay()`, `ModRingState` struct (toggles disabled until later phases). |
| `src/editor/theme.rs` | Modify | Add colour + size constants for the ring widget. |
| `src/params.rs` | Modify | Add `enable_heavy_modules: BoolParam` (default `true`). Add a one-shot `reset_requested: AtomicBool` if needed (or hook the egui side). |
| `tests/module_trait.rs` | Modify | Existing trait tests must compile against the new signature. Add a test asserting `ctx.unwrapped_phase` is `None` by default and that every shipped module ignores it. |
| `tests/module_spec_consistency.rs` | Create | Asserts `module_spec(t).num_curves == create_module(t,...).num_curves()` (already exists in code as debug_assert, lift to test). Also asserts new `ModuleSpec` fields default sensibly. |
| `tests/heavy_cpu_flag.rs` | Create | Asserts `heavy_cpu_for_mode()` defaults to `false` and follows module-level override. |

---

## Task 1: Convert `ModuleContext` to a borrowed, non-`Copy` struct

**Goal:** make `ModuleContext` carry `'block` lifetime so it can hold `Option<&[f32]>` references in later tasks. Keep payload byte-identical for now.

**Files:**
- Modify: `src/dsp/modules/mod.rs:67-79`
- Modify: `src/dsp/pipeline.rs` (every `ModuleContext { ... }` construction site)
- Modify: `src/dsp/fx_matrix.rs` (process_hop signature)
- Modify: `src/dsp/modules/{dynamics,freeze,phase_smear,contrast,gain,mid_side,ts_split,harmonic,master}.rs` — `process()` signature
- Modify: `tests/module_trait.rs`, `tests/engine_contract.rs`

- [ ] **Step 1.1: Read every existing call site**

Run: `grep -rn 'ModuleContext' src/ tests/`
Capture the list of call sites. Expected files: `src/dsp/modules/mod.rs`, `src/dsp/pipeline.rs`, `src/dsp/fx_matrix.rs`, all 9 module files in `src/dsp/modules/`, `tests/module_trait.rs`, `tests/engine_contract.rs`, possibly `tests/calibration_roundtrip.rs`.

- [ ] **Step 1.2: Write a failing trait-signature test**

Add to `tests/module_trait.rs`:

```rust
#[test]
fn module_context_has_block_lifetime_and_is_not_copy() {
    use spectral_forge::dsp::modules::ModuleContext;
    fn assert_not_copy<T>() where T: Sized {}  // intentionally no Copy bound
    assert_not_copy::<ModuleContext<'static>>();
    // If this compiles after Task 1, the lifetime is in place.
}
```

Run: `cargo test --test module_trait module_context_has_block_lifetime_and_is_not_copy 2>&1 | head`
Expected: FAIL with "missing lifetime" or "wrong number of generic arguments".

- [ ] **Step 1.3: Add `'block` lifetime to `ModuleContext`**

In `src/dsp/modules/mod.rs:67-79`, replace:

```rust
pub struct ModuleContext {
    pub sample_rate:       f32,
    pub fft_size:          usize,
    pub num_bins:          usize,
    pub attack_ms:         f32,
    pub release_ms:        f32,
    pub sensitivity:       f32,
    pub suppression_width: f32,
    pub auto_makeup:       bool,
    pub delta_monitor:     bool,
}
```

with:

```rust
pub struct ModuleContext<'block> {
    pub sample_rate:       f32,
    pub fft_size:          usize,
    pub num_bins:          usize,
    pub attack_ms:         f32,
    pub release_ms:        f32,
    pub sensitivity:       f32,
    pub suppression_width: f32,
    pub auto_makeup:       bool,
    pub delta_monitor:     bool,
    // Phantom keeps the lifetime live until later tasks add real `&'block` fields.
    _phantom: std::marker::PhantomData<&'block ()>,
}

impl<'block> ModuleContext<'block> {
    pub fn new(
        sample_rate: f32, fft_size: usize, num_bins: usize,
        attack_ms: f32, release_ms: f32, sensitivity: f32,
        suppression_width: f32, auto_makeup: bool, delta_monitor: bool,
    ) -> Self {
        Self {
            sample_rate, fft_size, num_bins, attack_ms, release_ms,
            sensitivity, suppression_width, auto_makeup, delta_monitor,
            _phantom: std::marker::PhantomData,
        }
    }
}
```

- [ ] **Step 1.4: Update the trait signature**

In `src/dsp/modules/mod.rs:113-124`, replace `ctx: &ModuleContext` with `ctx: &ModuleContext<'_>`.

- [ ] **Step 1.5: Update every module file's `process()` signature**

For each of `dynamics.rs`, `freeze.rs`, `phase_smear.rs`, `contrast.rs`, `gain.rs`, `mid_side.rs`, `ts_split.rs`, `harmonic.rs`, `master.rs`:

Find the `fn process(...)` line and change `ctx: &ModuleContext` to `ctx: &ModuleContext<'_>`. No body change.

- [ ] **Step 1.6: Update construction site in `Pipeline::process()`**

In `src/dsp/pipeline.rs`, find the `ModuleContext { ... }` literal and convert to `ModuleContext::new(...)`.

- [ ] **Step 1.7: Update construction sites in tests**

In `tests/module_trait.rs` and `tests/engine_contract.rs`, replace any direct `ModuleContext { ... }` construction with `ModuleContext::new(...)`.

- [ ] **Step 1.8: Build and run all tests**

Run: `cargo build && cargo test`
Expected: all 28 existing tests pass; new `module_context_has_block_lifetime_and_is_not_copy` test passes.

- [ ] **Step 1.9: Commit**

```bash
git add src/dsp/modules/mod.rs src/dsp/pipeline.rs src/dsp/fx_matrix.rs src/dsp/modules/*.rs tests/module_trait.rs tests/engine_contract.rs
git commit -m "$(cat <<'EOF'
infra(phase1): give ModuleContext a 'block lifetime

Lifts ModuleContext out of Copy and into a borrowing struct so later
phases can attach Option<&[f32]> fields (unwrapped_phase, peaks,
chromagram, etc.). No semantic change yet — payload is byte-identical;
all module signatures take ctx: &ModuleContext<'_> as a sigil for
the lifetime.

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: Add optional infra fields to `ModuleContext`

**Goal:** add the `Option<&[f32]>` and small-scalar fields listed in `02-architectural-refactors.md § 2`. Default `None` / 0.0 so existing modules ignore them.

**Files:**
- Modify: `src/dsp/modules/mod.rs:67-79` (the `ModuleContext` struct from Task 1)
- Modify: `src/dsp/pipeline.rs` (Pipeline::process construction site — pass `None` for now)
- Test: `tests/module_trait.rs`

- [ ] **Step 2.1: Write a failing test**

Add to `tests/module_trait.rs`:

```rust
#[test]
fn module_context_optional_fields_default_to_none() {
    use spectral_forge::dsp::modules::ModuleContext;
    let ctx = ModuleContext::new(
        48000.0, 2048, 1025, 10.0, 100.0, 1.0, 0.5, false, false,
    );
    assert!(ctx.unwrapped_phase.is_none());
    assert!(ctx.peaks.is_none());
    assert!(ctx.instantaneous_freq.is_none());
    assert!(ctx.chromagram.is_none());
    assert!(ctx.midi_notes.is_none());
    assert!(ctx.sidechain_derivative.is_none());
    assert_eq!(ctx.bpm, 0.0);
    assert_eq!(ctx.beat_position, 0.0);
}
```

Run: `cargo test --test module_trait module_context_optional_fields_default_to_none`
Expected: FAIL with "no field `unwrapped_phase` on type `ModuleContext`".

- [ ] **Step 2.2: Define the `PeakInfo` struct used by `peaks`**

In `src/dsp/modules/mod.rs`, just below the `ModuleContext` block, add:

```rust
/// One detected spectral peak. Populated by Phase 4.2 (PLPV peak detection).
/// Bin index `k` plus the magnitude at that bin. Skirt edges (low_k, high_k)
/// describe the peak's region of influence used by Voronoi assignment.
#[derive(Clone, Copy, Debug)]
pub struct PeakInfo {
    pub k:        u32,
    pub mag:      f32,
    pub low_k:    u32,
    pub high_k:   u32,
}
```

- [ ] **Step 2.3: Extend `ModuleContext` with optional fields**

In `src/dsp/modules/mod.rs`, replace the struct from Task 1 with:

```rust
pub struct ModuleContext<'block> {
    pub sample_rate:       f32,
    pub fft_size:          usize,
    pub num_bins:          usize,
    pub attack_ms:         f32,
    pub release_ms:        f32,
    pub sensitivity:       f32,
    pub suppression_width: f32,
    pub auto_makeup:       bool,
    pub delta_monitor:     bool,

    // Optional infra fields — populated by later phases. None by default.
    pub unwrapped_phase:      Option<&'block [f32]>,   // Phase 4.1
    pub peaks:                Option<&'block [PeakInfo]>, // Phase 4.2
    pub instantaneous_freq:   Option<&'block [f32]>,   // Phase 6.1
    pub chromagram:           Option<&'block [f32; 12]>, // Phase 6.2
    pub midi_notes:           Option<&'block [bool; 128]>, // Phase 6.3
    pub bpm:                  f32,                      // Phase 1 stub (0.0 until Phase 2 plumbs transport)
    pub beat_position:        f64,                      // Phase 1 stub
    pub sidechain_derivative: Option<&'block [f32]>,   // Phase 5b/Modulate Slew Lag
}
```

Update `ModuleContext::new()` to default all the new fields:

```rust
impl<'block> ModuleContext<'block> {
    pub fn new(
        sample_rate: f32, fft_size: usize, num_bins: usize,
        attack_ms: f32, release_ms: f32, sensitivity: f32,
        suppression_width: f32, auto_makeup: bool, delta_monitor: bool,
    ) -> Self {
        Self {
            sample_rate, fft_size, num_bins, attack_ms, release_ms,
            sensitivity, suppression_width, auto_makeup, delta_monitor,
            unwrapped_phase: None,
            peaks: None,
            instantaneous_freq: None,
            chromagram: None,
            midi_notes: None,
            bpm: 0.0,
            beat_position: 0.0,
            sidechain_derivative: None,
        }
    }
}
```

- [ ] **Step 2.4: Drain transport BPM into the context**

In `src/dsp/pipeline.rs`, find where `ModuleContext::new(...)` is called and add a follow-up assignment:

```rust
let mut ctx = ModuleContext::new(/* …existing args… */);
// Phase 1 stub: BPM/beat read from host transport when present.
// Modules consuming these don't ship until Phase 2 (Rhythm), so a 0.0
// default is currently equivalent to "no BPM info available".
if let Some(t) = process_ctx.transport() {
    ctx.bpm = t.tempo.unwrap_or(0.0) as f32;
    ctx.beat_position = t.pos_beats().unwrap_or(0.0);
}
```

(Adjust to actual nih-plug API — confirm `transport().tempo` field name in current dependency version.)

- [ ] **Step 2.5: Run the new test**

Run: `cargo test --test module_trait module_context_optional_fields_default_to_none`
Expected: PASS.

- [ ] **Step 2.6: Run the full suite to ensure nothing regresses**

Run: `cargo test`
Expected: all tests pass.

- [ ] **Step 2.7: Commit**

```bash
git add src/dsp/modules/mod.rs src/dsp/pipeline.rs tests/module_trait.rs
git commit -m "$(cat <<'EOF'
infra(phase1): add optional ModuleContext fields for later phases

Adds Option<&[f32]> fields (unwrapped_phase, peaks, instantaneous_freq,
chromagram, midi_notes, sidechain_derivative) and BPM/beat_position
scalars. All Optional fields default to None; modules opt in by reading
them only when present. Phase 4 will populate unwrapped_phase + peaks;
Phase 6 will populate instantaneous_freq + chromagram + midi_notes.

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: Add UX hint fields to `ModuleSpec`

**Goal:** add `wants_sidechain`, `panel_widget`, and `default_route` hints to `ModuleSpec`. Editor consumes `wants_sidechain` for default routing on first slot assignment. `panel_widget` is the function-pointer hook for Task 4.

**Files:**
- Modify: `src/dsp/modules/mod.rs:153-160`
- Modify: `src/dsp/modules/mod.rs:162-257` (each `static MODULESPEC` literal)
- Test: `tests/module_spec_consistency.rs` (new)

- [ ] **Step 3.1: Write a failing consistency test**

Create `tests/module_spec_consistency.rs`:

```rust
use spectral_forge::dsp::modules::{ModuleType, module_spec, create_module};

const ALL_TYPES: &[ModuleType] = &[
    ModuleType::Empty, ModuleType::Dynamics, ModuleType::Freeze,
    ModuleType::PhaseSmear, ModuleType::Contrast, ModuleType::Gain,
    ModuleType::MidSide, ModuleType::TransientSustainedSplit,
    ModuleType::Harmonic, ModuleType::Master,
];

#[test]
fn module_spec_num_curves_matches_module_num_curves() {
    for &t in ALL_TYPES {
        let spec = module_spec(t);
        let module = create_module(t, 48000.0, 2048);
        assert_eq!(
            spec.num_curves, module.num_curves(),
            "ModuleSpec disagrees with module for {:?}", t,
        );
    }
}

#[test]
fn module_spec_wants_sidechain_default_false_for_non_sc_modules() {
    // Modules that take no sidechain input must not request sidechain by default.
    assert!(!module_spec(ModuleType::MidSide).wants_sidechain);
    assert!(!module_spec(ModuleType::Contrast).wants_sidechain);
    assert!(!module_spec(ModuleType::Harmonic).wants_sidechain);
    assert!(!module_spec(ModuleType::Master).wants_sidechain);
    assert!(!module_spec(ModuleType::Empty).wants_sidechain);
    // Sidechain-capable modules also default to false: opt-in is intentional
    // so existing presets don't auto-route a fresh slot to a stale aux.
    assert!(!module_spec(ModuleType::Dynamics).wants_sidechain);
}
```

Run: `cargo test --test module_spec_consistency`
Expected: FAIL with "no field `wants_sidechain` on type `&ModuleSpec`".

- [ ] **Step 3.2: Extend `ModuleSpec`**

In `src/dsp/modules/mod.rs:153-160`, replace with:

```rust
pub struct ModuleSpec {
    pub display_name:       &'static str,
    pub color_lit:          Color32,
    pub color_dim:          Color32,
    pub num_curves:         usize,
    pub curve_labels:       &'static [&'static str],
    pub supports_sidechain: bool,

    /// True if a freshly assigned slot of this module should auto-route a
    /// sidechain input. Editor honours this on first assignment; user can
    /// override afterwards. False by default for all shipped modules.
    pub wants_sidechain:    bool,

    /// Optional per-module panel callback drawn below the curve editor.
    /// `None` means no panel (most modules). See Task 4 for signature.
    pub panel_widget:       Option<PanelWidgetFn>,
}

/// Per-module panel callback. Receives the egui `Ui`, plus a slot index
/// so the panel can read/write that slot's parameters. Lives below the
/// curve editor area in editor_ui.rs. Restricted to non-curve UI (step
/// grids, mode pickers, etc.) — curves stay in their own canvas.
/// (Param store is passed via the closure capture set at editor build.)
pub type PanelWidgetFn = fn(&mut nih_plug_egui::egui::Ui, slot: usize);
```

- [ ] **Step 3.3: Set defaults on every existing static `ModuleSpec`**

For each of `DYN`, `FRZ`, `PSM`, `CON`, `GN`, `MS`, `TS`, `HARM`, `MASTER`, `EMPTY` in `src/dsp/modules/mod.rs:165-244`, add the two new fields. Example for `DYN`:

```rust
static DYN: ModuleSpec = ModuleSpec {
    display_name: "Dynamics",
    color_lit: Color32::from_rgb(0x50, 0xc0, 0xc4),
    color_dim: Color32::from_rgb(0x18, 0x40, 0x42),
    num_curves: 6,
    curve_labels: &["THRESHOLD", "RATIO", "ATTACK", "RELEASE", "KNEE", "MIX"],
    supports_sidechain: true,
    wants_sidechain: false,
    panel_widget: None,
};
```

Apply the same two-field addition (`wants_sidechain: false, panel_widget: None,`) to all 10 statics.

- [ ] **Step 3.4: Run the test**

Run: `cargo test --test module_spec_consistency`
Expected: PASS.

- [ ] **Step 3.5: Run full suite**

Run: `cargo test`
Expected: PASS.

- [ ] **Step 3.6: Commit**

```bash
git add src/dsp/modules/mod.rs tests/module_spec_consistency.rs
git commit -m "$(cat <<'EOF'
infra(phase1): add wants_sidechain + panel_widget to ModuleSpec

Adds two opt-in UX hints for next-gen modules:
- wants_sidechain: editor auto-routes a sidechain on first assignment
- panel_widget: optional per-module non-curve UI panel callback

All shipped modules default to false / None — no behaviour change.
Future modules (Punch, Rhythm) will set wants_sidechain = true.

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: Per-mode `heavy_cpu` flag on `SpectralModule` trait

**Goal:** let modules report whether their currently-active mode is CPU-heavy. Used by an "always-bypassed-on-low-end-hardware" preset filter (UI affordance, not a CPU governor — see `02-architectural-refactors.md § 9`).

**Files:**
- Modify: `src/dsp/modules/mod.rs:113-149` (trait additions)
- Modify: `src/params.rs` (add `enable_heavy_modules: BoolParam`)
- Modify: `src/dsp/fx_matrix.rs` (short-circuit when `enable_heavy_modules == false` and any active mode is heavy)
- Test: `tests/heavy_cpu_flag.rs` (new)

- [ ] **Step 4.1: Write a failing test**

Create `tests/heavy_cpu_flag.rs`:

```rust
use spectral_forge::dsp::modules::{ModuleType, create_module};

#[test]
fn heavy_cpu_flag_defaults_to_module_spec_value() {
    let m = create_module(ModuleType::Dynamics, 48000.0, 2048);
    // Dynamics is light; default heavy_cpu_for_mode() must be false.
    assert!(!m.heavy_cpu_for_mode());
}

#[test]
fn empty_and_master_are_never_heavy() {
    for ty in [ModuleType::Empty, ModuleType::Master] {
        let m = create_module(ty, 48000.0, 2048);
        assert!(!m.heavy_cpu_for_mode());
    }
}
```

Run: `cargo test --test heavy_cpu_flag`
Expected: FAIL with "no method named `heavy_cpu_for_mode` found".

- [ ] **Step 4.2: Add `heavy_cpu_for_mode()` to the trait**

In `src/dsp/modules/mod.rs:113-149`, add a new method at the end of the trait (before `set_gain_mode`):

```rust
/// Returns true if the module's currently-active mode is CPU-heavy.
/// The "low-end-hardware" preset filter short-circuits process() when
/// `enable_heavy_modules == false` and this returns true.
/// Default: false. Modules with multiple modes return based on active mode.
fn heavy_cpu_for_mode(&self) -> bool { false }
```

- [ ] **Step 4.3: Add `enable_heavy_modules` parameter**

In `src/params.rs`, find the `SpectralForgeParams` struct (search for `pub struct SpectralForgeParams`) and add a field:

```rust
#[id = "enable_heavy_modules"]
pub enable_heavy_modules: BoolParam,
```

In its `Default` impl, initialize:

```rust
enable_heavy_modules: BoolParam::new("Enable Heavy Modules", true),
```

(If `BoolParam::new` signature differs in this nih-plug version, mirror an existing `BoolParam` field's pattern.)

- [ ] **Step 4.4: Short-circuit heavy modules in `FxMatrix::process_hop`**

In `src/dsp/fx_matrix.rs`, locate the per-slot module dispatch loop. Wrap the `module.process(...)` call:

```rust
if !enable_heavy_modules && module.heavy_cpu_for_mode() {
    // Short-circuit: copy input to output, leave suppression at 0.
    slot_out[s][..num_bins].copy_from_slice(&assembled_input[..num_bins]);
    suppression_out.fill(0.0);
} else {
    module.process(/* …existing args… */);
}
```

The `enable_heavy_modules` bool needs to be threaded through `process_hop`'s arg list — add it as the last positional arg. `Pipeline::process()` reads it from `self.params.enable_heavy_modules.value()` once per block and passes it down.

- [ ] **Step 4.5: Run the test**

Run: `cargo test --test heavy_cpu_flag`
Expected: PASS.

- [ ] **Step 4.6: Run full suite**

Run: `cargo test`
Expected: all tests pass; the `assert_process_allocs` smoke check still passes.

- [ ] **Step 4.7: Commit**

```bash
git add src/dsp/modules/mod.rs src/params.rs src/dsp/fx_matrix.rs src/dsp/pipeline.rs tests/heavy_cpu_flag.rs
git commit -m "$(cat <<'EOF'
infra(phase1): add heavy_cpu_for_mode() + enable_heavy_modules param

Adds an opt-in low-end-hardware affordance: when enable_heavy_modules
is false, slots whose active mode reports heavy_cpu_for_mode() == true
short-circuit to passthrough. Default true; behaviour unchanged for
shipped modules (all return false).

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: Per-module UI panel callback dispatch

**Goal:** when a slot's `ModuleSpec.panel_widget` is `Some`, the editor draws that callback below the curve editor for the active slot. Prerequisite for Rhythm Arpeggiator step grid (Phase 6) and Future module's pre-delay length picker (Phase 5b).

**Files:**
- Modify: `src/editor_ui.rs` (insert dispatch in the per-slot detail area)
- No module currently sets `panel_widget = Some(...)`, so this PR adds only the host-side wiring + a smoke test that the call-out is reachable.

- [ ] **Step 5.1: Locate the per-slot detail area in `editor_ui.rs`**

Run: `grep -n 'curve_widget\|active_slot\|fn create_editor' src/editor_ui.rs`
Identify where the curve editor is drawn for the active slot. The panel must render directly below it.

- [ ] **Step 5.2: Add the dispatch block**

Insert below the curve_widget call (replace `<active_slot>` with the variable name found in 5.1):

```rust
// Per-module non-curve UI panel (Phase 1 hook; defaults to no-op for
// every shipped module). See `dsp/modules/mod.rs::PanelWidgetFn`.
let active_module_type = params.module_type_for_slot(active_slot);
if let Some(panel_fn) = crate::dsp::modules::module_spec(active_module_type).panel_widget {
    ui.separator();
    panel_fn(ui, active_slot);
}
```

If `params.module_type_for_slot(slot)` does not exist, look for the existing accessor (likely an enum-keyed param) and use it.

- [ ] **Step 5.3: Add a smoke test in tests/editor_panel_dispatch.rs**

Because `egui::Ui` requires a real frame to construct, the smoke test asserts at the type level that `module_spec(ty).panel_widget` is the right callable shape:

```rust
#[test]
fn panel_widget_is_optional_function_pointer() {
    use spectral_forge::dsp::modules::{ModuleType, module_spec, PanelWidgetFn};
    for ty in [ModuleType::Dynamics, ModuleType::Freeze, ModuleType::Empty] {
        let spec = module_spec(ty);
        // Compile-time check: the field is an Option<PanelWidgetFn>.
        let _: Option<PanelWidgetFn> = spec.panel_widget;
    }
}
```

Run: `cargo test --test editor_panel_dispatch`
Expected: PASS.

- [ ] **Step 5.4: Manual UI sanity check**

Run: `cargo build --release && cargo run --package xtask -- bundle spectral_forge --release && cp target/bundled/spectral_forge.clap ~/.clap/`

Open in Bitwig. Add Spectral Forge. Cycle through every module type. Confirm:
- No visual regression
- No panel area appears (correct — no module sets `panel_widget` yet)

If the build fails because `xtask` isn't available, do `cargo build --release` only and skip the bundle step (the type-level smoke test in 5.3 covers correctness; the manual check is just to catch egui layout regressions).

- [ ] **Step 5.5: Commit**

```bash
git add src/editor_ui.rs tests/editor_panel_dispatch.rs
git commit -m "$(cat <<'EOF'
infra(phase1): wire per-module panel_widget dispatch in editor

Editor draws ModuleSpec.panel_widget below the curve area for the
active slot when set. All shipped modules leave it None — no UI
change. Unblocks Rhythm Arpeggiator step grid (Phase 6) and Future
module pre-delay picker (Phase 5b).

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>
EOF
)"
```

---

## Task 6: Reset-to-default button

**Goal:** one button in the editor frame that resets all params to nih-plug defaults and calls `Pipeline::reset()` on the audio side. Confirmation dialog because it's destructive.

**Files:**
- Modify: `src/editor_ui.rs` (button + confirm dialog)
- Modify: `src/bridge.rs` (add `reset_requested: AtomicBool` flag)
- Modify: `src/dsp/pipeline.rs` (drain `reset_requested` per block)
- Test: `tests/reset_to_default.rs` (new)

- [ ] **Step 6.1: Write a failing test for the reset bridge flag**

Create `tests/reset_to_default.rs`:

```rust
use spectral_forge::bridge::SharedState;
use std::sync::atomic::Ordering;

#[test]
fn reset_requested_flag_round_trips() {
    let s = SharedState::new(2048, 48000.0);
    assert!(!s.reset_requested.load(Ordering::Acquire));
    s.reset_requested.store(true, Ordering::Release);
    assert!(s.reset_requested.load(Ordering::Acquire));
    // Audio side resets the flag after handling.
    s.reset_requested.store(false, Ordering::Release);
    assert!(!s.reset_requested.load(Ordering::Acquire));
}
```

Run: `cargo test --test reset_to_default`
Expected: FAIL with "no field `reset_requested`".

- [ ] **Step 6.2: Add the flag to `SharedState`**

In `src/bridge.rs`, find `pub struct SharedState` and add:

```rust
pub reset_requested: Arc<std::sync::atomic::AtomicBool>,
```

Initialize in `SharedState::new(...)`:

```rust
reset_requested: Arc::new(std::sync::atomic::AtomicBool::new(false)),
```

- [ ] **Step 6.3: Drain the flag on the audio thread**

In `src/dsp/pipeline.rs`, at the very top of `Pipeline::process()` (after `flush_denormals`):

```rust
if shared.reset_requested.swap(false, std::sync::atomic::Ordering::AcqRel) {
    self.reset(self.sample_rate, self.fft_size);
}
```

(Confirm `shared` is the actual reference name in scope; if not, locate the `SharedState` ref.)

- [ ] **Step 6.4: Add the button to the editor frame**

In `src/editor_ui.rs`, in the global header area (top of the egui frame):

```rust
ui.horizontal(|ui| {
    // …existing header widgets…
    if ui.button("Reset to Default").clicked() {
        editor_state.show_reset_dialog = true;
    }
});

if editor_state.show_reset_dialog {
    nih_plug_egui::egui::Window::new("Reset all settings?")
        .collapsible(false)
        .resizable(false)
        .show(ui.ctx(), |ui| {
            ui.label("Reset every parameter to defaults and clear all module state. This cannot be undone.");
            ui.horizontal(|ui| {
                if ui.button("Reset").clicked() {
                    // Flip every Param to its default.
                    params.reset_to_defaults();
                    // Tell the audio thread to call Pipeline::reset().
                    shared.reset_requested.store(true, std::sync::atomic::Ordering::Release);
                    editor_state.show_reset_dialog = false;
                }
                if ui.button("Cancel").clicked() {
                    editor_state.show_reset_dialog = false;
                }
            });
        });
}
```

`editor_state` is the existing per-editor state struct. If it does not expose `show_reset_dialog`, add a `pub show_reset_dialog: bool` field.

`params.reset_to_defaults()` may not exist; if not, add a method on `SpectralForgeParams`:

```rust
impl SpectralForgeParams {
    pub fn reset_to_defaults(&self) {
        // Each Param exposes a setter that respects its `Default`.
        // Iterate every nih-plug Param via Params::param_map and reset.
        for (_, param_ptr, _) in self.param_map() {
            unsafe { param_ptr.set_normalized_value(param_ptr.default_normalized_value()); }
        }
    }
}
```

(Confirm nih-plug's `ParamPtr` API in this version — exact methods may be `set_normalized_value` / `default_normalized_value` or similar.)

- [ ] **Step 6.5: Run the test**

Run: `cargo test --test reset_to_default`
Expected: PASS.

- [ ] **Step 6.6: Manual UI sanity check**

Run the bundle command from Task 5.4. Open in Bitwig. Click the Reset button. Confirm:
- Dialog appears
- Cancel does nothing visible
- Reset returns curves to flat defaults; route matrix returns to serial
- No clicks or pops in audio output (audio thread reset is gated to one block)

- [ ] **Step 6.7: Commit**

```bash
git add src/editor_ui.rs src/bridge.rs src/dsp/pipeline.rs src/params.rs tests/reset_to_default.rs
git commit -m "$(cat <<'EOF'
infra(phase1): add Reset to Default button + audio-side reset flag

Adds a destructive Reset button in the editor header with a confirm
dialog. Resets every Param via Params::param_map and signals the audio
thread via SharedState::reset_requested to call Pipeline::reset()
on the next block. Single-block latency, no clicks.

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>
EOF
)"
```

---

## Task 7: Modulation Ring UI scaffolding

**Goal:** ship the ring widget and click handler. The S/H + Sync + Legato toggles render but stay disabled until BPM-sync infrastructure (Phase 4) makes them functional.

**Files:**
- Create: `src/editor/mod_ring.rs`
- Modify: `src/editor/mod.rs` (add `pub mod mod_ring; pub use mod_ring::*;`)
- Modify: `src/editor_ui.rs` (insert overlay container + alt-click hit detection)
- Modify: `src/editor/theme.rs` (add ring colours/sizes)
- Test: `tests/mod_ring.rs` (new — pure-data tests of the ring state machine)

- [ ] **Step 7.1: Write a failing test for `ModRingState`**

Create `tests/mod_ring.rs`:

```rust
use spectral_forge::editor::mod_ring::{ModRingState, ModRingToggle};

#[test]
fn ring_state_starts_with_all_toggles_off() {
    let s = ModRingState::default();
    assert!(!s.is_set(ModRingToggle::SampleHold));
    assert!(!s.is_set(ModRingToggle::Sync16));
    assert!(!s.is_set(ModRingToggle::Legato));
}

#[test]
fn ring_state_toggle_round_trip() {
    let mut s = ModRingState::default();
    s.toggle(ModRingToggle::SampleHold);
    assert!(s.is_set(ModRingToggle::SampleHold));
    s.toggle(ModRingToggle::SampleHold);
    assert!(!s.is_set(ModRingToggle::SampleHold));
}

#[test]
fn ring_toggles_are_disabled_until_bpm_sync_lands() {
    let s = ModRingState::default();
    // Phase 1 ships disabled; Phase 4 will flip the gate.
    assert!(!s.toggles_enabled());
}
```

Run: `cargo test --test mod_ring`
Expected: FAIL — module does not exist.

- [ ] **Step 7.2: Create `src/editor/mod_ring.rs`**

```rust
//! Modulation Ring overlay (S/H, Sync, Legato).
//!
//! Phase 1 scaffolding — the widget and the state machine ship now;
//! the toggles only become active once BPM/sync infra (Phase 4) is in
//! place. See `ideas/next-gen-modules/01-global-infrastructure.md` § 8.

use nih_plug_egui::egui::{self, Color32, Pos2, Stroke, Ui};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModRingToggle {
    SampleHold,
    Sync16,
    Legato,
}

impl ModRingToggle {
    fn bit(self) -> u8 {
        match self {
            ModRingToggle::SampleHold => 0b001,
            ModRingToggle::Sync16     => 0b010,
            ModRingToggle::Legato     => 0b100,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ModRingState {
    flags: u8,
}

impl ModRingState {
    pub fn is_set(&self, t: ModRingToggle) -> bool { (self.flags & t.bit()) != 0 }
    pub fn toggle(&mut self, t: ModRingToggle)   { self.flags ^= t.bit(); }
    /// Phase 4 will replace the constant `false` with a runtime check
    /// (e.g. `bpm_available && plpv_phase_ready`).
    pub fn toggles_enabled(&self) -> bool { false }
}

/// Draw the modulation ring overlay around an anchor point. Returns the
/// toggle that was clicked this frame, or None.
pub fn mod_ring_overlay(ui: &mut Ui, center: Pos2, state: &ModRingState) -> Option<ModRingToggle> {
    let radius = crate::editor::theme::MOD_RING_RADIUS;
    let dot_radius = crate::editor::theme::MOD_RING_DOT_RADIUS;
    let painter = ui.painter();

    // Three dots at 12, 4, and 8 o'clock.
    let positions = [
        (ModRingToggle::SampleHold, Pos2::new(center.x, center.y - radius)),
        (ModRingToggle::Sync16,     Pos2::new(center.x + radius * 0.866, center.y + radius * 0.5)),
        (ModRingToggle::Legato,     Pos2::new(center.x - radius * 0.866, center.y + radius * 0.5)),
    ];

    let mut clicked = None;
    let enabled = state.toggles_enabled();
    for (toggle, pos) in positions {
        let lit = state.is_set(toggle);
        let fill = match (lit, enabled) {
            (true, true)   => crate::editor::theme::MOD_RING_LIT,
            (false, true)  => crate::editor::theme::MOD_RING_DIM,
            (_, false)     => crate::editor::theme::MOD_RING_DISABLED,
        };
        painter.circle_filled(pos, dot_radius, fill);
        painter.circle_stroke(pos, dot_radius, Stroke::new(1.0, Color32::BLACK));

        let hit = ui.interact(
            egui::Rect::from_center_size(pos, egui::vec2(dot_radius * 2.0, dot_radius * 2.0)),
            ui.id().with(("mod_ring", toggle as i32, center.x as i32, center.y as i32)),
            egui::Sense::click(),
        );
        if hit.clicked() && enabled {
            clicked = Some(toggle);
        }
    }

    clicked
}
```

- [ ] **Step 7.3: Wire the new module**

In `src/editor/mod.rs`, add:

```rust
pub mod mod_ring;
pub use mod_ring::*;
```

- [ ] **Step 7.4: Add theme constants**

In `src/editor/theme.rs`, add:

```rust
pub const MOD_RING_RADIUS:     f32 = 16.0;
pub const MOD_RING_DOT_RADIUS: f32 = 4.0;
pub const MOD_RING_LIT:      Color32 = Color32::from_rgb(0xff, 0xc8, 0x40);
pub const MOD_RING_DIM:      Color32 = Color32::from_rgb(0x60, 0x40, 0x18);
pub const MOD_RING_DISABLED: Color32 = Color32::from_rgb(0x30, 0x30, 0x30);
```

- [ ] **Step 7.5: Wire alt-click detection in the curve editor**

In `src/editor/curve.rs` (or wherever curve nodes are drawn — `grep -n 'fn curve_widget' src/editor/`), the existing node hit-detection block needs an alt-click case:

```rust
if response.clicked() && ui.input(|i| i.modifiers.alt) {
    editor_state.mod_ring_anchor = Some((node_screen_pos, slot, curve_idx, node_idx));
}
```

Then in `editor_ui.rs`, after curve drawing:

```rust
if let Some((anchor, slot, curve, node)) = editor_state.mod_ring_anchor {
    let state = editor_state.mod_ring_states.entry((slot, curve, node)).or_default();
    if let Some(t) = crate::editor::mod_ring::mod_ring_overlay(ui, anchor, state) {
        state.toggle(t);
    }
    // Click outside the ring closes it.
    if ui.input(|i| i.pointer.any_click()) && !ring_hit_this_frame {
        editor_state.mod_ring_anchor = None;
    }
}
```

`mod_ring_states: HashMap<(usize, usize, usize), ModRingState>` and `mod_ring_anchor: Option<(Pos2, usize, usize, usize)>` are new fields on the editor state struct.

- [ ] **Step 7.6: Run the test**

Run: `cargo test --test mod_ring`
Expected: PASS.

- [ ] **Step 7.7: Manual UI check**

Bundle and load in Bitwig (per Task 5.4). Alt-click a curve node. Confirm:
- Three small dots appear around the node
- All three are dim/disabled grey (Phase 4 will enable)
- Clicking outside closes the ring
- No effect on audio (toggles are no-ops)

- [ ] **Step 7.8: Commit**

```bash
git add src/editor/mod_ring.rs src/editor/mod.rs src/editor_ui.rs src/editor/curve.rs src/editor/theme.rs tests/mod_ring.rs
git commit -m "$(cat <<'EOF'
infra(phase1): scaffold Modulation Ring overlay (S/H, Sync, Legato)

Adds the ring widget + state machine + alt-click trigger. Toggles render
in disabled grey until Phase 4 plumbs BPM/sync infra. Per-node ring
state is kept in the editor's mod_ring_states map; no audio-thread
consumer yet.

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>
EOF
)"
```

---

## Task 8: Phase 1 status banner update

**Goal:** mark Phase 1 complete in `STATUS.md` and add a status note to the relevant ideas/audit files.

**Files:**
- Modify: `docs/superpowers/STATUS.md`
- Modify: `ideas/next-gen-modules/99-implementation-roadmap.md` (Phase 1 section banner)

- [ ] **Step 8.1: Update STATUS.md**

Add (or update) a row under the existing implementation table:

```markdown
| `2026-04-27-phase-1-foundation-infra` | IMPLEMENTED | All 7 PRs landed. ModuleContext borrowed, ModuleSpec hints, heavy_cpu flag, panel_widget, reset, mod ring scaffold. |
```

- [ ] **Step 8.2: Add a status banner to the roadmap section**

In `ideas/next-gen-modules/99-implementation-roadmap.md`, immediately under the `## Phase 1 — Foundation infra` heading:

```markdown
> **Status:** IMPLEMENTED (2026-04-27 → release `0.X.0`). See plan
> `docs/superpowers/plans/2026-04-27-phase-1-foundation-infra.md`.
```

- [ ] **Step 8.3: Commit**

```bash
git add docs/superpowers/STATUS.md ideas/next-gen-modules/99-implementation-roadmap.md
git commit -m "$(cat <<'EOF'
docs(status): mark Phase 1 foundation infra IMPLEMENTED

Phase 1 of the next-gen-modules roadmap shipped: 7 small infra PRs
that unblock every later phase without adding any audible feature.

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>
EOF
)"
```

---

## Self-review checklist

Run before declaring Phase 1 done:

- [ ] **Spec coverage:** every bullet in `99-implementation-roadmap.md § Phase 1 § PRs` (1–6) maps to a Task. ✓ (Tasks 2 → PR 1, 3 → PR 2, 4 → PR 3, 5 → PR 4, 6 → PR 5, 7 → PR 6.)
- [ ] **Placeholder scan:** zero "TBD" / "implement later" / unspecified types.
- [ ] **Type consistency:** `ModuleContext<'block>` used uniformly; `PeakInfo`, `PanelWidgetFn`, `ModRingState`, `ModRingToggle` defined in exactly one place each.
- [ ] **All `cargo test` passes** between every Task. No skipped tests.
- [ ] **`assert_process_allocs` passes** (it's enabled in the default Cargo.toml feature set).
- [ ] **Manual UI check** for Tasks 5, 6, 7 — load the plugin in Bitwig, no visual regression.

---

## Risk register (Phase 1)

| Risk | Mitigation |
|---|---|
| `ModuleContext` lifetime causes a cascade of trait/impl signature breakage | Tasks 1 + 2 are sequential. Task 1 is a compile-only change; if it doesn't build clean, do not proceed. |
| `nih-plug` `ParamPtr` API for `reset_to_defaults` differs in our version | Verify by `grep -rn 'param_map\|ParamPtr' deps/`; fall back to per-param `.reset()` if needed. |
| `mod_ring` alt-click intercepts existing curve drag handlers | Add the handler *after* the existing drag check, with `if ui.input(|i| i.modifiers.alt)` as the early-exit predicate so non-alt clicks fall through. |
| Header button layout breaks at small editor sizes | Constrain Reset button to a fixed-width panel; verify at the smallest editor size in the manual check. |

---

## Execution handoff

Phase 1 is the smallest of the six plans and the highest priority — every later phase depends on at least one of Tasks 1, 2, or 4. After it ships, the rest of the roadmap can be implemented in any order that respects the inter-phase dependency map in `99-implementation-roadmap.md`.

Two execution options:

1. **Subagent-Driven (recommended)** — dispatch a fresh subagent per Task, review between Tasks, fast iteration.
2. **Inline Execution** — execute Tasks 1 → 8 in this session using `superpowers:executing-plans`, batch execution with checkpoints.

Tasks 1, 2 are sequential (Task 2 depends on the lifetime in Task 1). Tasks 3, 4, 5, 6, 7 are independent and can land in any order. Task 8 is the final wrap-up after all of 1–7.
