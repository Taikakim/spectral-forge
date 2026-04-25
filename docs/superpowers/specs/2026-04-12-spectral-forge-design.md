> **Status (2026-04-24): IMPLEMENTED — historical.** Foundational design. Partially superseded by the modular architecture spec (2026-04-19). Source of truth: the code + [../STATUS.md](../STATUS.md).

# Design: Spectral Forge — Modular Spectral Compressor CLAP Plugin

**Date:** 2026-04-12  
**Status:** Approved  
**Target:** Linux (EndeavourOS / Arch), CLAP only, Bitwig primary host  
**Language:** Rust (nih-plug), Python 3.12 (uv, tooling/analysis only)  
**GPU:** ROCm/HIP — future exploration, not in scope for v1

---

## 1. Purpose & Design Philosophy

A real-time **spectral compressor** — like oeksound Soothe2 in spirit, but with a different architecture. Audio is decomposed via STFT into frequency bins; each bin is independently compressed using per-bin parameter values read from a set of user-drawn EQ-style curves. The processed spectrum is resynthesised via overlap-add.

The user controls the compressor by drawing curves for each parameter (threshold, ratio, attack, release, knee, makeup, mix) using a 6-band parametric EQ editor. One curve is displayed at a time; buttons at the top of the GUI switch which parameter is being edited. This lets a producer set a gentle threshold across the mids but a more aggressive one in the problem frequencies — with a single curve gesture.

The architecture serves two audiences simultaneously:

1. **A human experimenter (Kim)** who invents new spectral effects by swapping DSP engines without touching GUI or transport plumbing.
2. **An agentic AI coding assistant** that must implement, debug, and extend any single layer without loading the entire codebase into context.

### Core Invariants

| ID | Invariant |
|----|-----------|
| **I-1** | The audio thread (`process()`) never allocates heap memory, never locks a mutex, never performs I/O. |
| **I-2** | The GUI thread never writes to the audio buffer or touches STFT state. |
| **I-3** | No file in `src/` exceeds 500 lines. If it does, split before adding features. |
| **I-4** | `SpectralEngine` is the only interface between the STFT pipeline and the effect algorithm. No engine reaches outside its own module. |
| **I-5** | `CurveNode` fields are stored normalised (`x`, `q` ∈ [0.0, 1.0]; `y` ∈ [−1.0, +1.0]). Physical units are computed only inside `editor/curve.rs` at biquad evaluation time. `BinParams` slices contain pre-computed physical values ready for the engine to use. |
| **I-6** | All cross-thread data transfer uses `triple_buffer` (latest-frame semantics) or `Arc<Atomic*>` (scalars). No `crossbeam-channel`, no `std::sync::Mutex` on the audio path. |
| **I-7** | Engine selection changes are applied exclusively at STFT hop boundaries, never mid-frame. |
| **I-8** | The relative threshold mode uses median-envelope estimation across neighboring bins. It does not use highpass filtering of the magnitude spectrum, Hilbert transform, or minimum-phase convolution — those are the subject of oeksound patents (US10587238, US20240194218). Our architecture (STFT → per-bin gain → ISTFT) is independently derived from standard spectral analysis prior art. |

---

## 2. The Effect: Per-Bin Spectral Compression

Each STFT bin is an independent compressor. The pipeline per bin per hop:

```
1. Detect level:
   absolute mode: level = bin_magnitude
   relative mode: level = bin_magnitude / local_median_envelope[bin]
                  (local_median_envelope = median of neighbouring bins, smoothed over time)

2. Optional detrend (relative mode):
   Before computing local_median, subtract a heavily-smoothed spectral tilt
   (2-octave Gaussian blur across bins) → detection is consistent regardless of
   how bright/dark the material is.

3. Gain computer (standard compressor math):
   overshoot  = level_db - threshold[bin]
   gain_db    = soft_knee(overshoot, knee[bin], ratio[bin])
   → gain_db is ≤ 0 (always attenuation, never boost)

4. Temporal smoothing of gain_db:
   effective_attack[bin]  = attack_ms  * freq_scale_factor(bin_frequency)
   effective_release[bin] = release_ms * freq_scale_factor(bin_frequency)
   smoothed_gain[bin] is envelope-followed using effective_attack/release

5. Bin-linking (smoothness):
   smoothed_gain = weighted average across neighbouring bins
   (prevents narrow notching artifacts from independent per-bin gain)

6. Apply:
   if sidechain active: gain is driven by sidechain magnitude, applied to main signal
   output_bin = main_bin * linear_gain(smoothed_gain[bin]) * makeup[bin]
   output_bin = lerp(main_bin, output_bin, mix[bin])   ← per-bin dry/wet

7. Write to suppression_data[bin] = |smoothed_gain[bin]| for GUI stalactite display
```

### Frequency Scaling for Attack/Release

```
freq_scale_factor(f) = (f_ref / f) ^ alpha
```

- `f_ref = 1000 Hz`, `alpha` = "Freq Scale" knob [0.0, 1.0]
- At `alpha=1, attack=2ms`: 100 Hz → ~20ms, 10 kHz → ~0.2ms
- Perceptually correct; also reduces CPU convergence time for high bins

### Gain Computer: Soft Knee

```
if overshoot <= -knee/2:        gr = 0
elif overshoot <= +knee/2:      gr = overshoot^2 / (2 * knee)   (quadratic knee)
else:                           gr = (overshoot) / ratio
```

`knee[bin]` is in dB, comes from the knee curve. Lower frequencies → wider knee.

---

## 3. Directory Structure

```
spectral_forge/
├── Cargo.toml
├── xtask/src/main.rs
├── src/
│   ├── lib.rs                          # Plugin struct, Params, CLAP export. ≤200 lines.
│   ├── params.rs                       # Global params: mix, gains, engine, sc_gain,
│   │                                   # attack_ms, release_ms, freq_scale, lookahead,
│   │                                   # stereo_link, auto_makeup, threshold_mode.
│   ├── bridge.rs                       # SharedState: all triple_buffers + atomics.
│   ├── editor.rs                       # egui top-level layout + parameter selector buttons.
│   ├── editor/
│   │   ├── mod.rs
│   │   ├── curve.rs                    # EQ node model, biquad response, curve widget.
│   │   ├── spectrum_display.rs         # Background magnitude bars (audio→GUI).
│   │   ├── suppression_display.rs      # Stalactite suppression bars (audio→GUI).
│   │   └── theme.rs                    # All visual constants: colours, stroke widths,
│   │                                   # fonts. Single file to modify for reskins.
│   └── dsp/
│       ├── mod.rs
│       ├── guard.rs                    # NaN/Inf clamp, denormal flush, init guard.
│       ├── pipeline.rs                 # StftHelper, sidechain path, engine dispatch.
│       └── engines/
│           ├── mod.rs                  # SpectralEngine trait, BinParams, EngineSelection.
│           ├── spectral_compressor.rs  # Default engine: per-bin compressor (this doc §2).
│           └── README.md               # Agent instructions for adding a new engine.
├── tests/
│   ├── stft_roundtrip.rs
│   ├── curve_sampling.rs
│   └── engine_contract.rs
└── docs/superpowers/specs/
    └── 2026-04-12-spectral-forge-design.md
```

### File Ownership Rules (agent-facing)

| File | Owner Concern | May Read | May Write |
|------|--------------|----------|-----------|
| `lib.rs` | Plugin lifecycle, CLAP export | any | only `lib.rs` |
| `params.rs` | Global parameter declarations | any | only `params.rs` |
| `bridge.rs` | Shared state types | `params.rs` | only `bridge.rs` |
| `editor.rs` | Top-level GUI layout, param selector | `params.rs`, `bridge.rs` | only `editor.rs` |
| `editor/curve.rs` | EQ node model, biquad computation, widget | `bridge.rs` | only `curve.rs` |
| `editor/spectrum_display.rs` | Background spectrum bars | `bridge.rs` | only this file |
| `editor/suppression_display.rs` | Stalactite suppression bars | `bridge.rs` | only this file |
| `editor/theme.rs` | Visual constants only | nothing | only `theme.rs` |
| `dsp/guard.rs` | Input sanitisation | nothing | only `guard.rs` |
| `dsp/pipeline.rs` | STFT, sidechain path, engine dispatch | `bridge.rs`, `engines/mod.rs` | only `pipeline.rs` |
| `dsp/engines/mod.rs` | Trait + registry + BinParams | nothing | only `mod.rs` |
| `dsp/engines/*.rs` | Individual engine | `engines/mod.rs` | only its own file |

---

## 4. Data Model

### 4.1 CurveNode (per-parameter, 6 per curve, 7 curves total)

```rust
// src/editor/curve.rs

pub enum BandType { LowShelf, Bell, HighShelf }

#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub struct CurveNode {
    pub x: f32,   // [0.0, 1.0] — log-frequency: 0.0 = 20 Hz, 1.0 = 20 kHz
    pub y: f32,   // [−1.0, +1.0] — gain: 0.0 = neutral, ±1.0 = max effect
    pub q: f32,   // [0.0, 1.0] — normalised octave-bandwidth: 0.1–4 oct (log-scaled)
}

pub const NUM_NODES: usize = 6;

// Band type is derived from index, never stored.
pub fn band_type_for(index: usize) -> BandType {
    match index { 0 => BandType::LowShelf, 5 => BandType::HighShelf, _ => BandType::Bell }
}
```

Nodes are **freely positionable** — no crossing constraints. Multiple nodes at similar frequencies combine additively in dB space (product of linear gains).

### 4.2 Parameter Curves

Seven independent curve sets, each with 6 `CurveNode`s:

| Curve | y=−1 meaning | y=0 meaning | y=+1 meaning |
|-------|-------------|-------------|-------------|
| **Threshold** | Min threshold (most sensitive) | Default | Max threshold (least sensitive) |
| **Ratio** | Min ratio (gentle) | Default | Max ratio (limiting) |
| **Attack** | Min attack (fastest) | Default | Max attack (slowest) |
| **Release** | Min release (fastest) | Default | Max release (slowest) |
| **Knee** | Hardest knee | Default | Widest/softest knee |
| **Makeup** | Max cut | Unity | Max boost |
| **Mix** | Fully dry | Default wet | Fully wet |

Physical unit conversions happen only in `curve.rs` at biquad evaluation time. The `BinParams` slices the engine receives contain physical values (dB, ms, etc.) ready for use.

### 4.3 Biquad Response Computation

Same for all curves. Runs on the GUI thread on any node change. Pure function.

```
For each node i:
  freq_hz  = 20.0 * 1000.0^x
  gain_db  = y * 18.0                           (±18 dB range)
  bw_oct   = 0.1 * 40.0^q                       (0.1–4 octaves)
  Q_linear = 1.0 / (2.0 * sinh(ln(2)/2 * bw_oct))   (RBJ octave-bandwidth)

  Compute RBJ biquad coefficients for band_type_for(i)
  Evaluate |H(e^jω)| at each of num_bins bin frequencies

combined_response[k] = product of all 6 magnitude responses at bin k
```

GUI shows bandwidth as "0.5 oct", "2 oct" — not Q. Each curve maps its combined response to a physical range at read time in `pipeline.rs`.

### 4.4 Cross-Thread Bridge

```rust
// src/bridge.rs

pub struct SharedState {
    pub num_bins: usize,                          // fft_size / 2 + 1, runtime

    // 7 curve channels: GUI → Audio (one per parameter)
    // Each carries physical values ready for BinParams
    pub curve_tx: [Arc<Mutex<TbInput<Vec<f32>>>>; 7],
    pub curve_rx: [TbOutput<Vec<f32>>; 7],

    // Audio → GUI: magnitude spectrum (for background bars)
    pub spectrum_tx: TbInput<Vec<f32>>,
    pub spectrum_rx: Arc<Mutex<TbOutput<Vec<f32>>>>,

    // Audio → GUI: suppression amount per bin (for stalactite bars)
    pub suppression_tx: TbInput<Vec<f32>>,
    pub suppression_rx: Arc<Mutex<TbOutput<Vec<f32>>>>,

    // Scalars
    pub sample_rate:      Arc<AtomicF32>,   // written at initialize(), read by GUI
    pub pending_engine:   Arc<AtomicU8>,    // S/H engine switch at hop boundary
    pub sidechain_active: Arc<AtomicBool>,  // mirrors host sidechain port state
}
```

Curve index mapping: `[0=threshold, 1=ratio, 2=attack, 3=release, 4=knee, 5=makeup, 6=mix]`

---

## 5. SpectralEngine Trait

```rust
// src/dsp/engines/mod.rs

/// Per-bin parameter values, pre-computed from curves by pipeline.rs.
/// Physical units — engines use these directly without further conversion.
pub struct BinParams<'a> {
    pub threshold_db: &'a [f32],   // compression threshold in dBFS per bin
    pub ratio:        &'a [f32],   // compression ratio (1.0 = no compression)
    pub attack_ms:    &'a [f32],   // already freq-scaled by pipeline
    pub release_ms:   &'a [f32],   // already freq-scaled by pipeline
    pub knee_db:      &'a [f32],   // soft knee width in dB
    pub makeup_db:    &'a [f32],   // makeup gain in dB
    pub mix:          &'a [f32],   // dry/wet per bin [0.0, 1.0]
}

pub trait SpectralEngine: Send {
    fn reset(&mut self, sample_rate: f32, fft_size: usize);

    fn process_bins(
        &mut self,
        bins: &mut [Complex<f32>],           // main signal — modify in place
        sidechain: Option<&[f32]>,           // sidechain magnitude envelope (pre-smoothed)
        params: &BinParams,
        sample_rate: f32,
        suppression_out: &mut [f32],         // write |gain_reduction_db| per bin for GUI
    );

    fn tail_length(&self, fft_size: usize) -> TailLength {
        TailLength::Finite(fft_size)
    }

    fn name(&self) -> &'static str;
}
```

---

## 6. Global Parameters (CLAP / Bitwig)

These are DAW-automatable. Curve nodes are persisted state, not automation lanes.

| Parameter | Type | Range | Smoothing | `MODULATABLE` | Notes |
|-----------|------|-------|-----------|---------------|-------|
| `mix` | float | 0–1 | Linear 10 ms | yes | Master dry/wet |
| `input_gain` | float | ±18 dB | Log 20 ms | yes | Pre-FFT drive |
| `output_gain` | float | ±18 dB | Log 20 ms | yes | Post-ISTFT trim |
| `attack_ms` | float | 0.5–200 ms | Log 20 ms | yes | Global attack (freq-scaled per bin) |
| `release_ms` | float | 1–500 ms | Log 20 ms | yes | Global release (freq-scaled per bin) |
| `freq_scale` | float | 0–1 | Linear 50 ms | yes | Frequency-dependent timing alpha |
| `sc_gain` | float | ±18 dB | Log 20 ms | yes | Sidechain input sensitivity |
| `sc_attack_ms` | float | 0.5–100 ms | Log 20 ms | yes | Sidechain envelope attack |
| `sc_release_ms` | float | 1–300 ms | Log 20 ms | yes | Sidechain envelope release |
| `lookahead_ms` | float | 0–10 ms | — | no | Adds to reported latency; stepped feel |
| `stereo_link` | enum | LR / Linked / MS | — | no | S/H at hop boundary |
| `threshold_mode` | enum | Absolute / Relative | — | no | S/H at hop boundary |
| `auto_makeup` | bool | off/on | — | no | Compensate avg gain reduction |
| `delta_monitor` | bool | off/on | — | no | Output only what's removed |
| `engine` | enum | SpectralCompressor / … | — | no | S/H at hop boundary |

Stepped enums and booleans: `CLAP_PARAM_IS_STEPPED`, not modulatable. Bitwig won't audio-rate modulate them.

Persisted (not automation):
```rust
#[persist = "curve_nodes"]   // [[CurveNode; 6]; 7] — all 7 parameter curves
#[persist = "active_curve"]  // usize — which curve the GUI is showing
#[persist = "editor_state"]  // Arc<EguiState>
```

---

## 7. GUI Design

### Layout

```
┌─────────────────────────────────────────────────────┐
│ [THRESHOLD] [RATIO] [ATTACK] [RELEASE] [KNEE] [MAKEUP] [MIX] │  ← parameter selector
├─────────────────────────────────────────────────────┤
│                                                     │
│  [suppression stalactites — downward bars from top] │
│                                                     │
│  [active parameter curve — EQ-style line]           │
│                                                     │
│  [spectrum bars — background, bottom-up]            │
│                                                     │
├─────────────────────────────────────────────────────┤
│ INPUT ──── ATTACK ─── RELEASE ─── FREQ SCALE ───── │
│ THRESH MODE [ABS/REL] ── STEREO [L/R|LINK|MS] ──── │
│ LOOKAHEAD ── AUTO MAKEUP ── DELTA ── OUTPUT ─ MIX  │
└─────────────────────────────────────────────────────┘
```

### Curve Interaction

| Action | Behaviour |
|--------|-----------|
| Horizontal drag | Adjust node frequency. Nodes freely positionable — no crossing constraint. |
| Vertical drag | Adjust y (gain/depth). Clamped to [−1.0, +1.0]. |
| Scroll wheel | Adjust bandwidth q. Clamped to [0.0, 1.0]. |
| Double-click node | Reset to DEFAULT_NODES entry for this curve. |
| Right-click node | Context menu: "Reset band" / "Reset all". |

### Visual Theme: 80s Vector / Elite

Inspired by Elite (David Braben, 1984) and Bitwig's minimalism. All visual constants live exclusively in `editor/theme.rs` — reskin by editing one file.

```rust
// editor/theme.rs (the only file that defines colours)
pub const BG:              Color32 = Color32::from_rgb(0x12, 0x12, 0x14);  // very dark grey
pub const GRID_LINE:       Color32 = Color32::from_rgb(0x1a, 0x2a, 0x28);  // dim teal grid
pub const BORDER:          Color32 = Color32::from_rgb(0x00, 0xcc, 0xbb);  // turquoise borders/dividers
pub const CURVE_LINE:      Color32 = Color32::from_rgb(0x00, 0xff, 0xdd);  // bright turquoise curve
pub const NODE_FILL:       Color32 = Color32::from_rgb(0x00, 0xcc, 0xbb);
pub const LABEL:           Color32 = Color32::from_rgb(0x88, 0xdd, 0xcc);  // muted turquoise text
pub const BTN_ACTIVE:      Color32 = Color32::from_rgb(0x00, 0xcc, 0xbb);
pub const BTN_INACTIVE:    Color32 = Color32::from_rgb(0x2a, 0x3a, 0x38);

// Spectrum bars: gradient dark-blue → blue → green → yellow → red
// Indexed by normalised magnitude [0.0, 1.0]
pub fn spectrum_color(magnitude_norm: f32) -> Color32 { ... }
// Suppression stalactites: same gradient, indexed by suppression depth
pub fn suppression_color(depth_norm: f32) -> Color32 { ... }

pub const STROKE_THIN:  f32 = 1.0;
pub const STROKE_CURVE: f32 = 1.5;
pub const NODE_RADIUS:  f32 = 5.0;
```

**Stalactite display:** suppression bars hang downward from the top edge of the curve area, amplitude = gain reduction at that bin, colour-coded by depth. Same colour gradient as spectrum bars.

---

## 8. STFT Pipeline

### Parameters

| Parameter | Value | Rationale |
|-----------|-------|-----------|
| FFT size | 2048 | ~46 ms frame, 1025 bins. Good spectral resolution for compression. |
| Window | Hann | COLA at 75% overlap. |
| Overlap | 75% (hop = 512) | Perfect reconstruction. |
| Latency | 2048 + lookahead samples | Reported to host exactly. |

### Pipeline Flow

```
Host audio buffer (main + optional sidechain)
  ↓
dsp::guard::flush_denormals()
dsp::guard::sanitize(main_buffer)
dsp::guard::sanitize(sidechain_buffer)   ← if sidechain active
  ↓
[Sidechain path, if active]:
  StftHelper (sidechain) → forward FFT → magnitude per bin
  → envelope-follow with sc_attack_ms / sc_release_ms (freq-scaled)
  → sidechain_envelope: Vec<f32>

[Main path]:
StftHelper::process_overlap_add(main_buffer, overlap=4, |channel, scratch| {
  [1] Forward realfft: 2048 real → 1025 Complex<f32>
  [2] Magnitudes → spectrum_tx (triple_buffer, GUI background bars)
  [3] Check pending_engine / pending_mode → apply at hop boundary (I-7)
  [4] Read all 7 curve_rx channels → build BinParams (apply freq_scale to attack/release)
  [5] engine.process_bins(&mut bins, sc_envelope, &params, sr, &mut suppression)
  [6] Apply stereo link if enabled (use max/RMS gain reduction across channels)
  [7] Apply lookahead delay if > 0
  [8] suppression → suppression_tx (triple_buffer, GUI stalactites)
  [9] Inverse realfft → 2048 real, normalise
})
  ↓
Apply delta_monitor: if active, output = dry - wet
Apply auto_makeup: if active, compensate long-term average gain reduction
  ↓
Host receives audio
```

---

## 9. Guard Layer (`dsp/guard.rs`)

```rust
pub fn flush_denormals() { /* FTZ/DAZ bits, x86_64 only */ }
pub fn sanitize(buf: &mut [f32]) { for s in buf { if !s.is_finite() { *s = 0.0 } } }
pub fn is_ready(state: &Option<SharedState>) -> bool { state.is_some() }
```

---

## 10. Input Safety & Crash Prevention

| Pitfall | Mitigation |
|---------|-----------|
| Allocation on audio thread | All buffers pre-allocated in `initialize()`. `assert_process_allocs` feature enforces this in dev. |
| Denormal floats | `guard::flush_denormals()` at process() entry. |
| NaN/Inf in FFT | `guard::sanitize()` before STFT. |
| Mutex on audio path | Only triple_buffer + atomics on audio path (I-6). |
| SR/buffer-size change | `SharedState` rebuilt in `initialize()`. Engines call `reset()`. |
| `process()` before `initialize()` | `guard::is_ready()` early-returns silently. |
| Mid-frame engine/mode switch | Atomics checked at hop boundary only (I-7). |
| Latency mismatch | `Plugin::latency()` returns `FFT_SIZE + lookahead_samples` exactly. |
| Sidechain port absent | `sidechain_active` AtomicBool; main signal used for detection if false. |
| FPU exception traps | FTZ/DAZ prevent SIGFPE from denormals. |

---

## 11. Dependency Stack

```toml
[dependencies]
nih_plug       = { git = "https://github.com/robbert-vdh/nih-plug.git",
                   features = ["assert_process_allocs"] }
nih_plug_egui  = { git = "https://github.com/robbert-vdh/nih-plug.git" }
realfft        = "3"
triple_buffer  = "9"
parking_lot    = "0.12"   # Mutex for GUI-side triple_buffer access only
num-complex    = "0.4"
# AtomicF32: re-exported by nih_plug. AtomicU8/AtomicBool: std.

[dev-dependencies]
approx = "0.5"

[profile.release]
lto = "thin"
opt-level = 3
strip = "symbols"
```

### System Dependencies (EndeavourOS)

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh   # Rust toolchain
sudo pacman -S libx11 libxcursor libxcb mesa pkg-config gcc
```

### Python (uv — tooling/analysis only)

```bash
uv init --no-readme
uv add numpy scipy soundfile
```

---

## 12. Testing Strategy

| Test | Validates |
|------|-----------|
| `stft_roundtrip.rs` | Identity engine: 440 Hz sine in, max sample error < 1e-4. |
| `curve_sampling.rs` | All nodes at y=0.0 → unity gains. Physical ranges correct. |
| `engine_contract.rs` | All-zero bins → all-zero out. No panic with any BinParams. `reset()` multiple times. |

Manual integration (Bitwig):
1. Pink noise → all curves flat → output matches input (gain reduction = 0 dB everywhere)
2. Audio-rate modulate `attack_ms` via Bitwig modulator → no clicks
3. Enable sidechain → stalactites appear only for bins where sidechain exceeds threshold
4. Toggle delta monitor → hear only what's being removed
5. Save session → reopen → all curves restored

---

## 13. Implementation Phases

| Phase | Goal | Key Files |
|-------|------|-----------|
| **1 — Skeleton** | CLAP loads in Bitwig, blank GUI, audio passthrough | `Cargo.toml`, `lib.rs`, `params.rs`, `editor.rs`, `bridge.rs` |
| **2 — STFT + Guard** | STFT↔ISTFT roundtrip, identity engine, guard layer | `dsp/guard.rs`, `dsp/pipeline.rs`, `dsp/engines/mod.rs`, `dsp/engines/spectral_compressor.rs` |
| **3 — Theme + Layout** | GUI skeleton with theme.rs, parameter selector buttons, empty curve area | `editor/theme.rs`, `editor.rs` |
| **4 — EQ Curve Editor** | 6-band EQ nodes, biquad response, all 7 curves, triple_buffer write | `editor/curve.rs`, `bridge.rs` |
| **5 — Spectrum Display** | Background magnitude bars + stalactite suppression bars | `editor/spectrum_display.rs`, `editor/suppression_display.rs`, `dsp/pipeline.rs` |
| **6 — Compressor Engine** | Full per-bin compression: threshold, ratio, knee, attack/release, freq-scaling, bin-linking | `dsp/engines/spectral_compressor.rs` |
| **7 — Connect Curves** | Curves drive engine in real time; BinParams wired through pipeline | `dsp/pipeline.rs` |
| **8 — Sidechain** | Sidechain STFT path, sc_attack/release, stalactites react to sidechain | `dsp/pipeline.rs`, `bridge.rs` |
| **9 — CLAP Polish** | Stereo link, M/S, lookahead, auto-makeup, delta monitor, relative mode + detrend, persist | `params.rs`, `dsp/pipeline.rs`, `lib.rs`, `editor.rs` |

---

## 14. Adding a New Engine (Agent Prompt Template)

```markdown
## Files you may read:
- src/dsp/engines/mod.rs                    (SpectralEngine trait + BinParams)
- src/dsp/engines/spectral_compressor.rs    (reference implementation)

## Files you will create:
- src/dsp/engines/{engine_name}.rs

## Files you will modify:
- src/dsp/engines/mod.rs  (pub mod + EngineSelection + create_engine match arm)

## DO NOT touch:
- lib.rs, editor.rs, editor/*, pipeline.rs, bridge.rs, params.rs, guard.rs

## Requirements:
1. impl SpectralEngine for YourEngine
2. process_bins() must not allocate, lock, or perform I/O
3. reset() must pre-allocate all internal state (envelope buffers, etc.)
4. Write gain reduction magnitude (dB) to suppression_out[i] for GUI stalactites
5. Override tail_length() if your engine has an extended tail
6. Run: cargo test --test engine_contract
```

---

## 15. Patent Safety Notes

The core oeksound Soothe patents (US10587238, US20240194218, US20190131951, US20260057866) cover:
- Highpass filtering of the magnitude spectrum → Hilbert transform → minimum phase convolution
- A specific dual shape+level combination formula `|H| = p * A^(B^q) * B`
- A filterbank (sub-band) architecture with per-band FFT

Our architecture avoids all of these:
- We use **STFT → per-bin gain multiplication → ISTFT** (standard spectral processing, prior art since the 1970s)
- Relative mode uses **median filtering across neighbouring bins** for envelope estimation (standard spectral analysis prior art)
- Detrending uses **Gaussian blur / moving average subtraction** (not their highpass-of-spectrum method)
- No Hilbert transform or minimum-phase filter involved anywhere

---

## 16. Future: ROCm / GPU Acceleration

Not in scope for v1. The `SpectralEngine` trait requires no changes — a GPU engine is just an engine. The per-bin compression loop is embarrassingly parallel and would map well to a HIP kernel. Kim runs at ≥24 ms, providing headroom for GPU round-trip latency.
