> **Status (2026-04-24): IMPLEMENTED.** Authoritative for slot/matrix/module architecture (slots, FxMatrix, RouteMatrix, SpectralModule trait). Source of truth: the code + [../STATUS.md](../STATUS.md).

# Modular Processing Architecture — Design Spec

**Date:** 2026-04-19  
**Status:** Approved — ready for implementation planning

## Goals

Replace the current fixed-stage DSP pipeline (single Dynamics slot + hardcoded EffectMode switching) with a fully modular architecture where:

- Every processing type lives in its own self-contained file implementing a common trait
- Up to 8 user-assignable slots + 1 fixed Master slot, routed via a routing matrix
- Each slot has independent per-bin curve parameters, each curve with its own tilt/offset
- Adding a new module type in future = one new file + one match arm in `create_module()`; nothing else changes
- 4 stereo CLAP sidechain inputs; each slot can be assigned to any of them
- The plugin UI adapts dynamically to whatever module is selected for editing
- Full state serialization from day one; typed Rust preset builders; no JSON files

---

## 1. SpectralModule Trait

**File:** `src/dsp/modules/mod.rs`

This is the single interface all modules implement. Nothing outside a module file needs to know the module's internals.

```rust
pub trait SpectralModule: Send {
    /// Process one STFT hop in-place.
    ///
    /// - `channel`: 0 = left/mid, 1 = right/side
    /// - `stereo_link`: plugin-wide stereo mode
    /// - `target`: which channels this slot processes (All / Mid / Side)
    /// - `bins`: complex FFT frame, modified in-place
    /// - `sidechain`: smoothed magnitude envelope from the assigned aux input,
    ///   or None if no sidechain is connected
    /// - `curves`: `[0..num_curves()]` slices, each `ctx.num_bins` long,
    ///   linear gain multipliers from the per-slot curve nodes (tilt/offset already applied)
    /// - `suppression_out`: filled with ≥0 dB values for the display;
    ///   must be fully written (not left uninitialised)
    /// - `ctx`: read-only shared context (sample rate, fft size, global scalars)
    fn process(
        &mut self,
        channel: usize,
        stereo_link: StereoLink,
        target: FxChannelTarget,
        bins: &mut [Complex<f32>],
        sidechain: Option<&[f32]>,
        curves: &[&[f32]],
        suppression_out: &mut [f32],
        ctx: &ModuleContext,
    );

    fn reset(&mut self, sample_rate: f32, fft_size: usize);

    /// Extra output tail beyond one FFT window. Default: 0.
    fn tail_length(&self) -> u32 { 0 }

    fn module_type(&self) -> ModuleType;

    /// How many curve slots this module uses (0..=7).
    fn num_curves(&self) -> usize;

    /// For T/S Split: returns Some(2) to signal two virtual output rows.
    /// All other modules return None (single output).
    fn num_outputs(&self) -> Option<usize> { None }
}
```

### ModuleContext

Passed read-only to every `process()` call. Contains the global scalar params that modules share.

```rust
pub struct ModuleContext<'a> {
    pub sample_rate: f32,
    pub fft_size:    usize,
    pub num_bins:    usize,
    pub attack_ms:          f32,
    pub release_ms:         f32,
    pub sensitivity:        f32,
    pub suppression_width:  f32,
    pub auto_makeup:        bool,
    pub delta_monitor:      bool,
    // extend as needed; modules access only what they use
    pub _params: std::marker::PhantomData<&'a ()>,
}
```

### ModuleType

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default,
         serde::Serialize, serde::Deserialize)]
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
    Master,   // only valid at slot 8; never stored in user-assignable slots
}
```

### ModuleSpec

Statically known metadata per type. Used by the GUI (colors, button labels) and by `create_module()`.

```rust
pub struct ModuleSpec {
    pub display_name:  &'static str,
    pub color_lit:     Color32,   // active/selected
    pub color_dim:     Color32,   // present but not selected
    pub num_curves:    usize,
    pub curve_labels:  &'static [&'static str],
}

pub fn module_spec(ty: ModuleType) -> &'static ModuleSpec { /* match */ }
```

**Fixed colours and curve counts:**

| Type                 | Color lit   | Curves | Labels |
|----------------------|-------------|--------|--------|
| Dynamics             | `#50c0c4`   | 6      | THRESHOLD, RATIO, ATTACK, RELEASE, KNEE, MIX |
| Freeze               | `#5080c8`   | 4      | LENGTH, THRESHOLD, PORTAMENTO, RESISTANCE |
| PhaseSmear           | `#9060c8`   | 2      | AMOUNT, SC SMOOTH |
| Contrast             | `#b060e0`   | 2      | AMOUNT, SC SMOOTH |
| Gain                 | `#c8a050`   | 2      | GAIN, SC SMOOTH |
| MidSide              | `#c050a0`   | 5      | BALANCE, EXPANSION, DECORREL, TRANSIENT, PAN |
| TransientSustainedSplit | `#80b060` | 1   | SENSITIVITY |
| Harmonic             | `#50c880`   | 0      | — |
| Master               | `#cccccc`   | 0      | — |

### create_module

```rust
pub fn create_module(
    ty: ModuleType,
    sample_rate: f32,
    fft_size: usize,
) -> Box<dyn SpectralModule>
```

The only place the concrete type names appear outside their own files. Every new module adds one arm here.

---

## 2. Module Files

**Directory:** `src/dsp/modules/`

Each file is fully self-contained. No module file may import from another module file.

### `dynamics.rs`
- Wraps the existing `SpectralEngine` (compressor)
- 6 curves: threshold, ratio, attack, release, knee, mix
- Note: Makeup is no longer a Dynamics curve — it is the standalone Gain module
- Channel gating logic moves here from `fx_matrix.rs`
- `auto_makeup` read from `ctx`
- The old `EffectMode::SpectralContrast` is removed; use the Contrast module instead

### `freeze.rs`
- Owns all freeze state: `frozen_bins`, `freeze_target`, `freeze_port_t`, `freeze_hold_hops`, `freeze_accum`, `freeze_captured` (all `Vec<_>` pre-allocated at `MAX_NUM_BINS`)
- 4 curves: length, threshold, portamento, resistance
- DSP moved verbatim from `pipeline.rs` STFT closure; only the curve reads change
- `tail_length()` returns `FFT_SIZE as u32`

### `phase_smear.rs`
- Owns `rng_state: u64` (xorshift64, never zero)
- 2 curves: amount, sc_smooth
- `amount` curve: per-bin phase randomisation depth (0.0–1.0)
- `sc_smooth` curve: temporal RMS smoothing window applied to sidechain magnitude before modulating
  smear intensity — prevents sudden sidechain spikes from disrupting the texture
- When `sidechain` is `Some(sc)`: smear intensity per bin = `amount[k] * smoothed_sc[k]`
- DSP moved from `pipeline.rs`

### `contrast.rs`
- Separate from PhaseSmear; handles spectral contrast enhancement (peak/valley exaggeration)
- 2 curves: amount, sc_smooth (same sidechain smoothing pattern as PhaseSmear)
- Sidechain modulates the contrast depth per bin
- Owns its own internal state (spectral envelope follower for peak/valley detection)
- The old `EffectMode::SpectralContrast` DSP migrates here

### `gain.rs`
- Per-bin gain with three switchable operating modes (stored as `GainMode` in params):
  ```rust
  #[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
  pub enum GainMode { Add, Subtract, Pull }
  ```
  - `Add`: apply gain curve; sidechain adds additional dB on top
  - `Subtract`: apply gain curve; sidechain subtracts dB
  - `Pull`: pull each bin's magnitude toward the sidechain bin's magnitude
    (EQ matching — bin tends toward the "colour" of the sidechain)
- 2 curves: GAIN (−24 to +18 dB per bin), SC SMOOTH (RMS temporal smoothing on sidechain)
- `suppression_out` is zeroed (gain shows in spectrum display, not as suppression)
- `GainMode` stored per-slot in params (see Section 3a)

### `ts_split.rs` (TransientSustainedSplit)
- Transient/sustained signal splitter
- 1 curve: SENSITIVITY (controls transient detection threshold per bin)
- `num_outputs()` returns `Some(2)` — signals to the routing matrix that this slot has two
  virtual output rows (Transient and Sustained) instead of one
- Internally produces two output buffers: `transient_out` and `sustained_out`
  (pre-allocated at `MAX_NUM_BINS` complex)
- DSP: onset/transient detection per bin (e.g. ratio of current to windowed-average magnitude);
  bins above the per-bin sensitivity threshold route to `transient_out`, the remainder to
  `sustained_out`; a crossfade zone prevents hard discontinuities
- The pipeline calls `process()` normally; then reads `transient_bins()` and `sustained_bins()`
  extra accessors to populate the two virtual row output buffers
  ```rust
  fn transient_bins(&self) -> &[Complex<f32>];
  fn sustained_bins(&self) -> &[Complex<f32>];
  ```
- `tail_length()` returns `FFT_SIZE as u32` (needs context window)

### `mid_side.rs`
- Ported from `spectral2/src/dsp/pipeline.rs` lines 446–501
- 5 curves: balance, expansion, decorrelation, transient, pan
- Only active when `stereo_link == StereoLink::MidSide`; passes through otherwise
- Requires M/S encode/decode to be active at the pipeline level (same requirement as today)

### `harmonic.rs`
- Pass-through; 0 curves
- Reserved for future harmonic processing

### `master.rs`
- Transparent pass-through; 0 curves
- Slot 8 only; cannot be assigned or removed by the user
- `suppression_out` zeroed

---

## 3. Data Model

### 3a. Params (`src/params.rs`)

**Remove:**
- `curve_nodes`, `active_curve`
- `phase_curve_nodes`, `freeze_curve_nodes`, `freeze_active_curve`
- `fx_module_types`, `fx_module_names`, `fx_module_targets`
- `effect_mode: EnumParam<EffectMode>`
- `phase_rand_amount: FloatParam`
- `spectral_contrast_db: FloatParam`

**Add:**
```rust
/// Per-slot curve nodes. [slot 0..=8][curve 0..6][node 0..5].
/// Slot 8 = Master; its curves are unused but stored for uniform access.
#[persist = "slot_curve_nodes"]
pub slot_curve_nodes: Arc<Mutex<[[[CurveNode; NUM_NODES]; 7]; 9]>>,

/// Per-slot per-curve tilt and offset. [slot][curve] = (tilt: f32, offset: f32).
/// Applied uniformly via apply_curve_transform() before sending to the module.
#[persist = "slot_curve_meta"]
pub slot_curve_meta: Arc<Mutex<[[(f32, f32); 7]; 9]>>,  // default: (0.0, 0.0)

/// Module type assigned to each slot.
/// Slot 8 is always ModuleType::Master; the value stored here is ignored.
#[persist = "slot_module_types"]
pub slot_module_types: Arc<Mutex<[ModuleType; 9]>>,

/// User-editable name for each slot. UTF-8, zero-padded to 32 bytes.
/// Display truncates to ~10 visible characters; full name shown on hover.
#[persist = "slot_names"]
pub slot_names: Arc<Mutex<[[u8; 32]; 9]>>,

/// Channel routing target per slot (All / Mid / Side).
#[persist = "slot_targets"]
pub slot_targets: Arc<Mutex<[FxChannelTarget; 9]>>,

/// Sidechain input assignment per slot. 0..=3 = aux input index.
/// 255 = self-detect (use main input when no sidechain is connected).
#[persist = "slot_sidechain"]
pub slot_sidechain: Arc<Mutex<[u8; 9]>>,

/// GainMode per slot (only meaningful for Gain module slots).
#[persist = "slot_gain_mode"]
pub slot_gain_mode: Arc<Mutex<[GainMode; 9]>>,

/// Which curve within the editing slot is selected (0..num_curves).
#[persist = "editing_curve"]
pub editing_curve: Arc<Mutex<u8>>,
```

**Rename/extend:**
- `editing_slot` stays (now 0..=8)
- `fx_route_matrix` extended to accommodate T/S Split virtual rows:
  `Arc<Mutex<RouteMatrix>>` where `RouteMatrix` is defined in Section 4

**Keep (global scalars, unaffected):**
- `input_gain`, `output_gain`, `mix`
- `attack_ms`, `release_ms`, `sc_gain`, `sc_attack_ms`, `sc_release_ms`
- `lookahead_ms`, `stereo_link`, `threshold_mode`, `sensitivity`
- `suppression_width`, `auto_makeup`, `delta_monitor`
- `graph_db_min`, `graph_db_max`, `peak_falloff_ms`

**Remove enums:**
- `EffectMode` — deleted
- `FxModuleType` — replaced by `ModuleType` in `modules/mod.rs`

**Default state:**
- Slot 0: `ModuleType::Dynamics`, name `"Dynamics"`, target `All`, sidechain `255`
- Slot 1: `ModuleType::Dynamics`, name `"Dynamics 2"`, target `All`, sidechain `255`
- Slot 2: `ModuleType::Gain`, name `"Gain"`, target `All`, sidechain `255`
- Slots 3–7: `ModuleType::Empty`, names `"Slot 3"` … `"Slot 7"`
- Slot 8: `ModuleType::Master`, name `"Master"`, immutable
- All `slot_curve_nodes`: default neutral nodes per curve type
- All `slot_curve_meta`: `(0.0, 0.0)` (no tilt, no offset)
- Default routing: Slot 0 → Master (1.0), Slot 1 → Master (1.0), Slot 2 → Master (1.0), all others zeroed

### 3b. Bridge (`src/bridge.rs`)

Replace 7 flat curve channels with 9×7 = 63:

```rust
pub curve_tx: Vec<Vec<Arc<Mutex<TbInput<Vec<f32>>>>>>,  // [slot][curve]
pub curve_rx: Vec<Vec<TbOutput<Vec<f32>>>>,              // [slot][curve]
```

`SharedState::new(num_bins, sample_rate, fft_size)` — add `fft_size` parameter (Plan B already planned this).

Add per-aux sidechain activity:
```rust
pub sidechain_active: [Arc<AtomicBool>; 4],
```

Remove: `pending_engine: Arc<AtomicU8>` (dead code since Plan C).

### 3c. Pipeline (`src/dsp/pipeline.rs`)

**Slot curve cache:**

Replace `curve_cache: [Vec<f32>; 7]` with:
```rust
slot_curve_cache: Vec<Vec<Vec<f32>>>,  // [9][7][MAX_NUM_BINS], pre-allocated
```

Read + tilt/offset application (no allocation):
```rust
for s in 0..9 {
    for c in 0..7 {
        let src = shared.curve_rx[s][c].read();
        self.slot_curve_cache[s][c][..num_bins].copy_from_slice(src);
        let (tilt, offset) = slot_curve_meta[s][c];
        apply_curve_transform(&mut self.slot_curve_cache[s][c][..num_bins], tilt, offset);
    }
}
```

**Sidechain:**

Process up to 4 aux inputs independently:
```rust
let mut sc_envelopes: [Option<Vec<f32>>; 4] = [None, None, None, None];
for i in 0..aux.inputs.len().min(4) {
    // run StftHelper + envelope follower for aux.inputs[i]
    // store smoothed magnitude envelope in sc_envelopes[i]
}
```

Per-slot sidechain selection:
```rust
let sc = match slot_sidechain[i] {
    255 => if sc_envelopes[0].as_ref().map(|e| e.iter().any(|&v| v > 1e-9)).unwrap_or(false) {
        sc_envelopes[0].as_deref()
    } else { None },
    idx => sc_envelopes[idx as usize].as_deref(),
};
```

**STFT closure** shrinks: no more `match effect_mode`. Just calls `fx_matrix.process_hop(...)` for each channel.

### 3d. CLAP layouts (`src/lib.rs`)

```rust
const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[
    AudioIOLayout {
        main_input_channels:  NonZeroU32::new(2),
        main_output_channels: NonZeroU32::new(2),
        aux_input_ports: &[
            new_nonzero_u32(2),  // sidechain 1
            new_nonzero_u32(2),  // sidechain 2
            new_nonzero_u32(2),  // sidechain 3
            new_nonzero_u32(2),  // sidechain 4
        ],
        ..AudioIOLayout::const_default()
    },
    AudioIOLayout {
        main_input_channels:  NonZeroU32::new(2),
        main_output_channels: NonZeroU32::new(2),
        ..AudioIOLayout::const_default()
    },
];
```

---

## 4. Routing Matrix and T/S Split Virtual Rows

### RouteMatrix

The routing matrix is extended to support T/S Split modules, which produce two virtual output
rows (Transient, Sustained) rather than one. Virtual rows are **source-only** — they emit signal
but cannot receive it.

```rust
/// Maximum number of virtual output rows from split modules.
/// 2 splits × 2 virtual rows each = 4 maximum.
pub const MAX_SPLIT_VIRTUAL_ROWS: usize = 4;

/// Total matrix rows: 9 real slots + up to 4 virtual rows.
pub const MAX_MATRIX_ROWS: usize = 9 + MAX_SPLIT_VIRTUAL_ROWS;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RouteMatrix {
    /// Send level from each source row to each destination slot.
    /// rows: real slots 0..=8, then virtual rows indexed by VirtualRowId.
    /// cols: real slots 0..=8 only (virtual rows cannot receive).
    pub send: [[f32; 9]; MAX_MATRIX_ROWS],

    /// Which real slots currently have active virtual rows, and their virtual row indices.
    /// Entry: (real_slot_index, VirtualRowKind::Transient | Sustained).
    /// At most MAX_SPLIT_VIRTUAL_ROWS entries; unused entries are None.
    pub virtual_rows: [Option<(u8, VirtualRowKind)>; MAX_SPLIT_VIRTUAL_ROWS],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum VirtualRowKind { Transient, Sustained }
```

**Virtual row UI rendering:**
- In the matrix grid, virtual rows appear immediately after their parent slot row
- Each virtual row is rendered at half the height of a normal slot row
- The two virtual rows together occupy the same vertical space as one normal slot
- A coloured left border (orange = Transient, blue = Sustained) distinguishes them
- Virtual row labels: `"{slot}T"` and `"{slot}S"` (e.g. `"3T"`, `"3S"`)
- Columns for virtual rows show ⊘ on the master column — virtual rows cannot bypass to output directly

**Pipeline wiring for split modules:**
1. `process_hop` processes the T/S Split slot normally (calls `module.process()`)
2. After `process()`, if `module.num_outputs() == Some(2)`:
   - Read `module.transient_bins()` into virtual row Transient buffer
   - Read `module.sustained_bins()` into virtual row Sustained buffer
3. These buffers then act as input sources for any downstream slots that route from those virtual rows

**Constraint:** At most 2 T/S Split modules may be active simultaneously (caps virtual row count at 4). The assignment popup disables the T/S Split option when 2 are already in use.

### FxMatrix Changes (`src/dsp/fx_matrix.rs`)

```rust
pub const MAX_SLOTS: usize = 9;

pub struct FxMatrix {
    pub slots: [Option<Box<dyn SpectralModule>>; MAX_SLOTS],
    pub route: RouteMatrix,
    slot_out_cur:      Vec<Vec<Complex<f32>>>,  // [MAX_SLOTS][MAX_NUM_BINS]
    slot_out_prev:     Vec<Vec<Complex<f32>>>,
    slot_supp:         Vec<Vec<f32>>,
    virtual_out:       Vec<Vec<Complex<f32>>>,  // [MAX_SPLIT_VIRTUAL_ROWS][MAX_NUM_BINS]
}
```

`FxSlotKind` enum is deleted entirely; replaced by `Box<dyn SpectralModule>`.

**Slot 8 (Master):**
- Initialised in `FxMatrix::new()` as `Some(Box::new(MasterModule::new(...)))`
- `process_hop` treats slot 8 as the designated output: after processing, writes slot 8's
  output buffer back to `complex_buf`
- Cannot be assigned or removed via any public API

**process_hop signature:**
```rust
pub fn process_hop(
    &mut self,
    channel: usize,
    stereo_link: StereoLink,
    complex_buf: &mut [Complex<f32>],
    sc_envelopes: &[Option<&[f32]>; 4],
    slot_sidechain: &[u8; 9],
    slot_targets: &[FxChannelTarget; 9],
    slot_curves: &[&[&[f32]; 7]; 9],     // [slot][curve][bin]
    ctx: &ModuleContext,
    suppression_out: &mut [f32],
    num_bins: usize,
)
```

---

## 5. Per-Curve Tilt and Offset

Every curve in every slot has its own tilt and offset, stored in `slot_curve_meta[slot][curve]`.

A single shared function applies the transform:

```rust
/// Apply tilt and offset to a pre-computed gain curve.
///
/// `tilt` (range approx −1.0..+1.0): tilts the curve so low bins are
///   attenuated and high bins boosted, or vice versa. Applied as a linear
///   ramp multiplied onto the gain values.
///
/// `offset` (range approx −1.0..+1.0): shifts all gain values uniformly
///   (additive in linear gain space, equivalent to global gain offset).
///
/// Called once per slot×curve after reading from the triple buffer.
/// Input: linear gain multipliers (1.0 = neutral). Output: modified in-place.
pub fn apply_curve_transform(gains: &mut [f32], tilt: f32, offset: f32) {
    let n = gains.len();
    for (k, g) in gains.iter_mut().enumerate() {
        let t = tilt * (k as f32 / n as f32 - 0.5);  // −tilt/2 .. +tilt/2
        *g = (*g + offset) * (1.0 + t);
        *g = g.max(0.0);  // clamp to non-negative
    }
}
```

The function is defined once in `src/dsp/modules/mod.rs` (or a shared `src/dsp/util.rs`).
No module file reimplements it. The UI exposes tilt and offset as `DragValue` widgets in the
curve editing panel, one pair per selected curve.

---

## 6. Preset Architecture

**File:** `src/presets.rs`

All modular state is fully serializable via `serde` from day one. Presets are typed Rust
functions that return a fully-populated `PluginState` struct. If the data model changes, the
compiler breaks the preset functions immediately — no silent format mismatches, no JSON files
to hunt down.

```rust
/// The complete serializable state of the plugin.
/// This is what nih-plug persists to the DAW project and what presets are made of.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PluginState {
    pub slot_module_types: [ModuleType; 9],
    pub slot_names:        [[u8; 32]; 9],
    pub slot_targets:      [FxChannelTarget; 9],
    pub slot_sidechain:    [u8; 9],
    pub slot_gain_mode:    [GainMode; 9],
    pub slot_curve_nodes:  [[[CurveNode; NUM_NODES]; 7]; 9],
    pub slot_curve_meta:   [[(f32, f32); 7]; 9],
    pub route:             RouteMatrix,
    // global scalars are stored as standard nih-plug params; not duplicated here
}
```

**Factory presets (small initial set):**

```rust
pub fn preset_default() -> PluginState { /* Slot 0 = Dynamics, Slot 1 = Dynamics, Slot 2 = Gain, all → Master */ }
pub fn preset_transient_sculptor() -> PluginState { /* Dyn → T/S → [Frz(S), Gn(T)] → M */ }
pub fn preset_spectral_width() -> PluginState { /* Dyn(Mid) + Dyn(Side) → M/S → M */ }
pub fn preset_phase_sculptor() -> PluginState { /* Dyn → Smear → Contrast → M */ }
pub fn preset_freeze_pad() -> PluginState { /* Frz (long) → Gn → M */ }
```

CLAP factory preset discovery is implemented via `ClapPlugin::clap_preset_discovery_factories()`.
Each `preset_*()` function is called at discovery time and serialized to JSON on the fly —
no embedded JSON blobs, no build-time generation step.

**Versioning:** No schema version field during development. Add a `schema_version: u32` field
and a `migrate()` function before the 1.0 release when the format stabilises. Breaking changes
to `PluginState` during development are intentional and expected; factory presets are updated
to match each time.

---

## 7. UI Changes

### Removed
- `EffectMode` button strip (Bypass / Freeze / PhaseRand / SpectralContrast) — gone entirely
- Fixed tabs DYNAMICS / EFFECTS / HARMONIC — gone; replaced by matrix selection
- Freeze-specific curve buttons in top bar — replaced by adaptive buttons
- Phase-specific controls — absorbed into PhaseSmear module slot editing

### Module assignment popup
- **Empty cell (diagonal):** left-click opens popup
- **Populated cell (diagonal):** left-click selects for editing; right-click opens popup with "Remove module" option
- Popup is an egui `Area` anchored near the clicked cell
- Lists all assignable types with colour swatch; "Remove module" at bottom (absent on Master)
- T/S Split is greyed-out and non-clickable when 2 are already active
- On assignment: `slot_module_types[i]` updated, `FxMatrix` slot replaced with `create_module(ty, ...)`,
  curve nodes reset to type defaults, `RouteMatrix` virtual rows updated if T/S Split

### Adaptive curve selector buttons (top bar)
- Buttons derive from `module_spec(slot_module_types[editing_slot]).curve_labels`
- Active button uses `color_lit` for the editing slot's type; inactive buttons use `color_dim`
- If `num_curves == 0` (Master, Harmonic): no curve buttons shown; top bar shows only range controls
- Tilt and offset `DragValue` widgets appear below the curve editor for the selected curve

### Matrix cell display
- Name truncated to fit cell width (~10 characters); full name shown as egui hover tooltip
- Cell background: `color_lit` when selected, `color_dim` when populated-but-not-selected
- Master cell (row/col 8): always white/bright (`#cccccc`), label "OUT", no popup
- T/S Split rows display the module on a full-height row; the two virtual rows (Transient,
  Sustained) follow at half-height each, with orange/blue left borders

### Graph header
- Format: `"Editing: {name} — {target}"` where name is the slot's user name
- Name is **inline-editable**: clicking the name activates a `TextEdit` widget, saved on Enter
  or focus loss, limited to 32 bytes UTF-8
- Disambiguation number appended automatically in the display name when multiple slots share a
  type ("Dynamics", "Dynamics 2", "Dynamics 3") — the stored name is separate and user-controlled

### Sidechain assignment
- Per-slot sidechain selector shown in the slot editing strip (below the curve area):
  small buttons "SC1" "SC2" "SC3" "SC4" "Self", selecting which aux input this slot uses
- "Self" = 255 = fall back to main input when aux is disconnected (current behaviour)

### GainMode selector
- Shown in the slot editing strip when the selected slot is a Gain module
- Three buttons: "Add" / "Subtract" / "Pull"
- Updates `slot_gain_mode[editing_slot]` via the params mutex

---

## 8. Implementation Plans

### Plan D1 — Foundation (target: same behaviour, new architecture)

| # | Task |
|---|------|
| 1 | `src/dsp/modules/mod.rs`: SpectralModule trait, ModuleType, ModuleSpec, ModuleContext, RouteMatrix, apply_curve_transform, create_module stub |
| 2 | Module files: dynamics.rs, freeze.rs, phase_smear.rs, contrast.rs, gain.rs, ts_split.rs, harmonic.rs, master.rs (DSP migrated from pipeline.rs; Contrast from EffectMode) |
| 3 | `src/params.rs`: add all per-slot fields + slot_curve_meta + slot_gain_mode; remove old fields |
| 4 | `src/bridge.rs`: 9×7 curve channels; 4 sidechain_active atomics; remove pending_engine |
| 5 | `src/lib.rs`: 4 aux sidechain inputs in AUDIO_IO_LAYOUTS; wire fft_size through initialize |
| 6 | `src/dsp/pipeline.rs`: slot_curve_cache with tilt/offset application; 4 sidechain paths; shrink STFT closure |
| 7 | `src/dsp/fx_matrix.rs`: Box<dyn SpectralModule>, RouteMatrix, virtual rows for T/S Split, Master at slot 8, updated process_hop |
| 8 | `src/presets.rs`: PluginState struct, preset_default(), preset_transient_sculptor(), and 3 more; wire to CLAP factory preset API |

End state: plugin behaviour identical to today plus new module types; all tests pass.

### Plan D2 — UX + new modules

| # | Task |
|---|------|
| 1 | Module assignment popup (egui Area); right-click on populated cell; T/S Split cap enforcement |
| 2 | Adaptive curve selector buttons in top bar; remove fixed tabs and EffectMode strip |
| 3 | Matrix cell truncation + hover tooltip; inline name edit in graph header |
| 4 | Sidechain assignment strip (SC1–SC4/Self buttons per slot) |
| 5 | Per-curve tilt/offset DragValue widgets in curve editing panel |
| 6 | GainMode selector (Add/Subtract/Pull) in slot editing strip |
| 7 | Half-height virtual rows for T/S Split in matrix UI; orange/blue left borders |
| 8 | M/S module DSP (ported from spectral2) + curve editor wiring |

---

## 9. What Is Not In Scope

- Variable FFT per slot (Plan B covers plugin-wide variable FFT)
- Harmonic module DSP (placeholder only in D1/D2)
- More than 4 sidechain inputs
- Reordering slots (matrix position is fixed; user labels slots by name)
- Schema versioning / preset migration (added before 1.0 release)
- More than 2 simultaneous T/S Split modules
