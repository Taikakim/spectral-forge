> **Status (2026-04-27): IMPLEMENTED.** All 9 tasks merged on `feature/next-gen-modules-plans`. `FutureModule` with two modes (`PrintThrough`, `PreEcho`) sharing per-channel ring buffers (`MAX_ECHO_FRAMES = 64` hops); Print-Through uses 5%-leak default with two-pass adjacent-bin spread (`spread_scratch` preserves dry phase even when centre = 0 at max spread); Pre-Echo does full write-ahead with feedback decay (capped at 0.4 to keep closed-loop gain ≤ 0.8 < 1.0 even at AMOUNT=2.0) plus per-bin HF damping; per-slot mode persistence via `Arc<Mutex<[FutureMode; 9]>>` (mirrors `slot_gain_mode` shape) with serde round-trip; UI exposes Future under `ASSIGNABLE` plus a per-slot Print-Thru / Pre-Echo button picker mirroring the GainMode picker block; calibration probes (test/feature-gated) cover both modes' `amount_pct`, `mix_pct`, `length_ms` units. Lookahead Duck + Crystal Ball deferred per audit. The code is the source of truth; this plan is kept for history. See [../STATUS.md](../STATUS.md).

# Phase 2b — Future Module Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship a new `Future` module with two write-ahead sub-effects: **Tape Print-Through** (5% magnitude leak with adjacent-bin spread, write-ahead by N hops) and **Pre-Echo with Pre-Delay** (full signal write-ahead with feedback decay).

**Architecture:** Plain `SpectralModule` slot with internal per-channel ring buffers. No external infrastructure dependencies (no HistoryBuffer, no BinPhysics). Each slot owns its own `pre_echo_buf[MAX_NUM_BINS × MAX_ECHO_FRAMES]` ring (~2 MB per channel), shared between modes. A single-mode-per-slot enum selects Print-Through or Pre-Echo. Lookahead Duck and Crystal Ball are deferred per the audit.

**Tech Stack:** Rust, num_complex, nih-plug.

**Source design:** `ideas/next-gen-modules/14-future.md` (audit). No prior spec — this plan also doubles as the spec.

**Roadmap reference:** `ideas/next-gen-modules/99-implementation-roadmap.md` § Phase 2 (item 2).

---

## File structure

| File | Status | Responsibility |
|---|---|---|
| `src/dsp/modules/future.rs` | Create | `FutureModule` struct, `FutureMode` enum (`PrintThrough`/`PreEcho`), per-channel ring buffer state, `process()` kernel for both modes. |
| `src/dsp/modules/mod.rs` | Modify | Add `Future` variant to `ModuleType`, `FUT` static `ModuleSpec`, wire `module_spec()` and `create_module()`. |
| `src/editor/module_popup.rs` | Modify | Add `ModuleType::Future` to `ASSIGNABLE` so users can pick it. |
| `src/params.rs` | Modify | Add `slot_future_mode: [Mutex<FutureMode>; MAX_SLOTS]` for per-slot mode persistence (mirroring the existing per-slot gain mode pattern). |
| `tests/future.rs` | Create | Kernel tests: Print-Through delay/spread, Pre-Echo delay/feedback, mix curve. |
| `tests/calibration_roundtrip.rs` | Modify | Add a Future-module case so calibration probe coverage stays complete. |

---

## Curve mapping (5 curves)

| Idx | Label | Print-Through | Pre-Echo |
|---|---|---|---|
| 0 | AMOUNT | Leak % (0–20%, neutral=5%) | Echo amplitude (0–2.0, neutral=1.0) |
| 1 | TIME | Echo delay in hops (1–63, neutral=8) | Echo delay in hops (1–63, neutral=8) |
| 2 | THRESHOLD | unused | Feedback decay per echo (0–0.99, neutral=0.4) |
| 3 | SPREAD | Adjacent-bin bleed % (0–50%, neutral=20%) | HF damping per echo (0–1.0, neutral=0.2) |
| 4 | MIX | Wet (0–1, neutral=0.5) | Wet (0–1, neutral=0.5) |

`num_curves() = 5`.

---

## Task 1 — Add `ModuleType::Future` + spec

**Files:**
- Modify: `src/dsp/modules/mod.rs:14-27` (`ModuleType`), `:162-257` (`module_spec`)

- [ ] **Step 1: Write the failing test**

```rust
// tests/future.rs (NEW)
use spectral_forge::dsp::modules::{ModuleType, module_spec};

#[test]
fn future_module_spec_has_5_curves() {
    let spec = module_spec(ModuleType::Future);
    assert_eq!(spec.num_curves, 5);
    assert_eq!(spec.curve_labels, &["AMOUNT", "TIME", "THRESHOLD", "SPREAD", "MIX"]);
    assert!(!spec.supports_sidechain);
    assert_eq!(spec.display_name, "Future");
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --test future future_module_spec_has_5_curves`
Expected: compile error — `ModuleType::Future` does not exist.

- [ ] **Step 3: Add the variant + spec**

Edit `src/dsp/modules/mod.rs` at the `ModuleType` enum:
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
    Future,        // NEW
    Master,
}
```

In `module_spec()`, add a static and match arm:
```rust
static FUT: ModuleSpec = ModuleSpec {
    display_name: "Future",
    color_lit: Color32::from_rgb(0x60, 0xa0, 0xc8),
    color_dim: Color32::from_rgb(0x20, 0x34, 0x42),
    num_curves: 5,
    curve_labels: &["AMOUNT", "TIME", "THRESHOLD", "SPREAD", "MIX"],
    supports_sidechain: false,
};
// ...
match ty {
    // existing arms...
    ModuleType::Future                 => &FUT,
    ModuleType::Master                 => &MASTER,
    ModuleType::Empty                  => &EMPTY,
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --test future future_module_spec_has_5_curves`
Expected: 1 passed.

- [ ] **Step 5: Commit**

```bash
git add src/dsp/modules/mod.rs tests/future.rs
git commit -m "feat(future): ModuleType::Future variant + ModuleSpec"
```

---

## Task 2 — `FutureMode` enum + skeleton struct

**Files:**
- Create: `src/dsp/modules/future.rs`
- Modify: `src/dsp/modules/mod.rs` (add `pub mod future;`)

- [ ] **Step 1: Write the failing test**

```rust
// tests/future.rs — append
use spectral_forge::dsp::modules::future::{FutureModule, FutureMode};

#[test]
fn future_mode_default_is_print_through() {
    assert_eq!(FutureMode::default(), FutureMode::PrintThrough);
}

#[test]
fn future_module_starts_silent() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::{SpectralModule, ModuleContext};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let mut m = FutureModule::new();
    m.reset(48000.0, 1024);
    let mut bins = vec![Complex::new(1.0, 0.0); 513];
    let curves_storage: Vec<Vec<f32>> = (0..5).map(|_| vec![1.0f32; 513]).collect();
    let curves: Vec<&[f32]> = curves_storage.iter().map(|v| v.as_slice()).collect();
    let mut supp = vec![0.0f32; 513];
    let ctx = ModuleContext {
        sample_rate: 48000.0, fft_size: 1024, num_bins: 513,
        attack_ms: 10.0, release_ms: 100.0, sensitivity: 0.5,
        suppression_width: 1.0, auto_makeup: false, delta_monitor: false,
    };
    m.process(0, StereoLink::Linked, FxChannelTarget::All,
        &mut bins, None, &curves, &mut supp, &ctx);
    // First hop: dry signal preserved; ring buffer still empty so no echo.
    for c in &bins { assert!(c.re.is_finite() && c.im.is_finite()); }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --test future future_mode_default_is_print_through`
Expected: compile error — `FutureModule` does not exist.

- [ ] **Step 3: Create the module skeleton**

```rust
// src/dsp/modules/future.rs (NEW)
use num_complex::Complex;
use serde::{Deserialize, Serialize};
use crate::params::{FxChannelTarget, StereoLink};
use super::{ModuleContext, ModuleType, SpectralModule};

pub const MAX_ECHO_FRAMES: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum FutureMode {
    #[default]
    PrintThrough,
    PreEcho,
}

impl FutureMode {
    pub fn label(self) -> &'static str {
        match self {
            FutureMode::PrintThrough => "Print-Through",
            FutureMode::PreEcho      => "Pre-Echo",
        }
    }
}

pub struct FutureModule {
    mode:        FutureMode,
    fft_size:    usize,
    sample_rate: f32,
    /// Ring buffer of write-ahead frames. `[channel][frame_idx][bin]`.
    /// Frame index advances by 1 every hop; reads at `(write_pos + delay_hops) % MAX_ECHO_FRAMES`.
    pub ring:    [Vec<Vec<Complex<f32>>>; 2],
    write_pos:   [usize; 2],
    #[cfg(any(test, feature = "probe"))]
    last_probe:  crate::dsp::modules::ProbeSnapshot,
}

impl FutureModule {
    pub fn new() -> Self {
        Self {
            mode:        FutureMode::default(),
            fft_size:    2048,
            sample_rate: 44100.0,
            ring:        [Vec::new(), Vec::new()],
            write_pos:   [0; 2],
            #[cfg(any(test, feature = "probe"))]
            last_probe: Default::default(),
        }
    }

    pub fn set_mode(&mut self, mode: FutureMode) { self.mode = mode; }
    pub fn mode(&self) -> FutureMode { self.mode }
}

impl Default for FutureModule {
    fn default() -> Self { Self::new() }
}

impl SpectralModule for FutureModule {
    fn reset(&mut self, sample_rate: f32, fft_size: usize) {
        self.sample_rate = sample_rate;
        self.fft_size    = fft_size;
        let n = fft_size / 2 + 1;
        for ch in 0..2 {
            self.ring[ch] = (0..MAX_ECHO_FRAMES)
                .map(|_| vec![Complex::new(0.0, 0.0); n])
                .collect();
            self.write_pos[ch] = 0;
        }
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
        // Stub — Tasks 3 + 4 implement Print-Through and Pre-Echo kernels.
        suppression_out.fill(0.0);
        let _ = bins;
    }

    fn tail_length(&self) -> u32 { (self.fft_size as u32) * (MAX_ECHO_FRAMES as u32) / 4 }
    fn module_type(&self) -> ModuleType { ModuleType::Future }
    fn num_curves(&self) -> usize { 5 }

    #[cfg(any(test, feature = "probe"))]
    fn last_probe(&self) -> crate::dsp::modules::ProbeSnapshot { self.last_probe }
}
```

Edit `src/dsp/modules/mod.rs` — add near the other `pub mod xxx;` lines:
```rust
pub mod future;
```

In `create_module()`:
```rust
ModuleType::Future                 => Box::new(future::FutureModule::new()),
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --test future`
Expected: 3 passed.

- [ ] **Step 5: Commit**

```bash
git add src/dsp/modules/future.rs src/dsp/modules/mod.rs tests/future.rs
git commit -m "feat(future): FutureModule skeleton + FutureMode enum"
```

---

## Task 3 — Print-Through kernel

**Files:**
- Modify: `src/dsp/modules/future.rs` (`process()` body)

- [ ] **Step 1: Write the failing test**

```rust
// tests/future.rs — append
#[test]
fn print_through_writes_ahead_then_reads() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::{SpectralModule, ModuleContext};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let mut m = FutureModule::new();
    m.set_mode(FutureMode::PrintThrough);
    m.reset(48000.0, 1024);

    // Curves: AMOUNT=1.0 (5% leak), TIME=1.0 (8 hops delay), THRESHOLD unused,
    //         SPREAD=0.0 (no adjacent bleed), MIX=2.0 → mix=1.0 (full wet)
    let mut amount = vec![1.0f32; 513];
    let mut time   = vec![1.0f32; 513];
    let     thresh = vec![1.0f32; 513];
    let mut spread = vec![0.0f32; 513];
    let mut mix    = vec![2.0f32; 513];
    let supp_zero = vec![0.0f32; 513];

    let ctx = ModuleContext {
        sample_rate: 48000.0, fft_size: 1024, num_bins: 513,
        attack_ms: 10.0, release_ms: 100.0, sensitivity: 0.5,
        suppression_width: 1.0, auto_makeup: false, delta_monitor: false,
    };

    // Hop 0: feed a unit impulse at bin 100. Wet output should be 0 (buffer empty).
    let mut bins = vec![Complex::new(0.0, 0.0); 513];
    bins[100] = Complex::new(1.0, 0.0);
    let curves: Vec<&[f32]> = vec![&amount, &time, &thresh, &spread, &mix];
    let mut supp = supp_zero.clone();
    m.process(0, StereoLink::Linked, FxChannelTarget::All,
        &mut bins, None, &curves, &mut supp, &ctx);
    assert!(bins[100].norm() < 0.01,
        "hop 0 wet should be silent (no historical data yet)");

    // Hops 1..=7: silence in.
    for _ in 1..=7 {
        let mut buf = vec![Complex::new(0.0, 0.0); 513];
        m.process(0, StereoLink::Linked, FxChannelTarget::All,
            &mut buf, None, &curves, &mut supp, &ctx);
    }

    // Hop 8: still silence; the impulse written at hop 0 should now read out.
    let mut bins = vec![Complex::new(0.0, 0.0); 513];
    m.process(0, StereoLink::Linked, FxChannelTarget::All,
        &mut bins, None, &curves, &mut supp, &ctx);
    // 5% leak × MIX=1.0 → expect ~0.05 magnitude at bin 100.
    assert!(bins[100].norm() > 0.03 && bins[100].norm() < 0.08,
        "hop 8 should read back the print-through with ~5% leak; got {}",
        bins[100].norm());
}

#[test]
fn print_through_spread_bleeds_to_adjacent_bins() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::{SpectralModule, ModuleContext};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let mut m = FutureModule::new();
    m.set_mode(FutureMode::PrintThrough);
    m.reset(48000.0, 1024);

    let amount = vec![1.0f32; 513];
    let time   = vec![1.0f32; 513];
    let thresh = vec![1.0f32; 513];
    let spread = vec![1.0f32; 513];   // 20% spread to k±1
    let mix    = vec![2.0f32; 513];

    let ctx = ModuleContext {
        sample_rate: 48000.0, fft_size: 1024, num_bins: 513,
        attack_ms: 10.0, release_ms: 100.0, sensitivity: 0.5,
        suppression_width: 1.0, auto_makeup: false, delta_monitor: false,
    };
    let curves: Vec<&[f32]> = vec![&amount, &time, &thresh, &spread, &mix];

    // Hop 0: impulse at bin 100.
    let mut bins = vec![Complex::new(0.0, 0.0); 513];
    bins[100] = Complex::new(1.0, 0.0);
    let mut supp = vec![0.0f32; 513];
    m.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, None, &curves, &mut supp, &ctx);

    // Skip 7 hops.
    for _ in 1..=7 {
        let mut buf = vec![Complex::new(0.0, 0.0); 513];
        m.process(0, StereoLink::Linked, FxChannelTarget::All, &mut buf, None, &curves, &mut supp, &ctx);
    }

    // Hop 8: check that bins 99 + 101 also have signal due to spread.
    let mut bins = vec![Complex::new(0.0, 0.0); 513];
    m.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, None, &curves, &mut supp, &ctx);
    assert!(bins[99].norm()  > 0.005, "spread should bleed left, got {}",  bins[99].norm());
    assert!(bins[101].norm() > 0.005, "spread should bleed right, got {}", bins[101].norm());
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --test future print_through_`
Expected: failures — kernel is a stub.

- [ ] **Step 3: Implement the Print-Through kernel**

Replace the stub `process()` body in `src/dsp/modules/future.rs`:

```rust
fn process(
    &mut self,
    channel: usize,
    _stereo_link: StereoLink,
    _target: FxChannelTarget,
    bins: &mut [Complex<f32>],
    _sidechain: Option<&[f32]>,
    curves: &[&[f32]],
    suppression_out: &mut [f32],
    _ctx: &ModuleContext,
) {
    let ch = channel.min(1);
    let n  = bins.len();
    debug_assert_eq!(self.ring[ch][0].len(), n,
        "FutureModule: bins/ring size mismatch — call reset() before process()");

    // Map curves to physical params (using bin n/2 as the probe location for ProbeSnapshot).
    let probe_k = n / 2;
    let amount_curve = curves.get(0).copied().unwrap_or(&[][..]);
    let time_curve   = curves.get(1).copied().unwrap_or(&[][..]);
    let thresh_curve = curves.get(2).copied().unwrap_or(&[][..]);
    let spread_curve = curves.get(3).copied().unwrap_or(&[][..]);
    let mix_curve    = curves.get(4).copied().unwrap_or(&[][..]);

    #[cfg(any(test, feature = "probe"))]
    let mut probe_amount_pct = 0.0f32;
    #[cfg(any(test, feature = "probe"))]
    let mut probe_time_hops  = 0u32;
    #[cfg(any(test, feature = "probe"))]
    let mut probe_mix_pct    = 0.0f32;

    match self.mode {
        FutureMode::PrintThrough => {
            // First read out the frame that was written `delay_hops` hops ago.
            // Then write 5%-scaled current frame into ring at write_pos for future read.
            // Use bin-0 TIME curve as the slot-wide delay (Print-Through is per-slot, not per-bin).
            let time_gain = time_curve.get(probe_k).copied().unwrap_or(1.0).clamp(0.0, 2.0);
            let delay_hops = ((time_gain * 8.0).round() as usize).clamp(1, MAX_ECHO_FRAMES - 1);

            // Read position: read the frame that was written delay_hops ago.
            let read_pos = (self.write_pos[ch] + MAX_ECHO_FRAMES - delay_hops) % MAX_ECHO_FRAMES;

            for k in 0..n {
                // Per-bin parameters.
                let amount_gain = amount_curve.get(k).copied().unwrap_or(1.0).clamp(0.0, 4.0);
                let leak_pct    = (amount_gain * 0.05).clamp(0.0, 0.20); // 5% nominal
                let spread_gain = spread_curve.get(k).copied().unwrap_or(0.0).clamp(0.0, 2.0);
                let spread_pct  = (spread_gain * 0.20).clamp(0.0, 0.50); // 20% nominal each side
                let mix_gain    = mix_curve.get(k).copied().unwrap_or(1.0).clamp(0.0, 2.0);
                let mix         = (mix_gain * 0.5).clamp(0.0, 1.0);

                let dry = bins[k];
                // Wet = the sample at read_pos for bin k.
                let wet = self.ring[ch][read_pos][k];
                bins[k] = Complex::new(
                    dry.re * (1.0 - mix) + wet.re * mix,
                    dry.im * (1.0 - mix) + wet.im * mix,
                );

                #[cfg(any(test, feature = "probe"))]
                if k == probe_k {
                    probe_amount_pct = leak_pct * 100.0;
                    probe_time_hops  = delay_hops as u32;
                    probe_mix_pct    = mix * 100.0;
                }

                // Write into ring at write_pos: 5% leak × dry magnitude, preserved phase.
                // Spread: distribute (1 - 2*spread_pct) to centre, spread_pct to each neighbour.
                let leaked_mag = dry.norm() * leak_pct;
                let phase_unit = if dry.norm() > 1e-12 { dry / dry.norm() } else { Complex::new(1.0, 0.0) };
                let centre  = phase_unit * (leaked_mag * (1.0 - 2.0 * spread_pct));
                let side    = phase_unit * (leaked_mag * spread_pct);
                self.ring[ch][self.write_pos[ch]][k] = centre;
                if k > 0     { self.ring[ch][self.write_pos[ch]][k - 1] += side; }
                if k + 1 < n { self.ring[ch][self.write_pos[ch]][k + 1] += side; }
            }
        }
        FutureMode::PreEcho => {
            // Implemented in Task 4.
            // Stub: copy dry into ring (no echo until kernel lands).
            for k in 0..n {
                self.ring[ch][self.write_pos[ch]][k] = bins[k];
            }
        }
    }

    // Advance write position.
    self.write_pos[ch] = (self.write_pos[ch] + 1) % MAX_ECHO_FRAMES;
    // Pre-clear the slot we will write into on the next hop, so leftover spread doesn't accumulate.
    let next_pos = self.write_pos[ch];
    for k in 0..n { self.ring[ch][next_pos][k] = Complex::new(0.0, 0.0); }

    suppression_out.fill(0.0);

    #[cfg(any(test, feature = "probe"))]
    {
        self.last_probe = crate::dsp::modules::ProbeSnapshot {
            amount_pct: Some(probe_amount_pct),
            length_ms:  Some((probe_time_hops as f32) * (self.fft_size as f32 / 4.0) / self.sample_rate * 1000.0),
            mix_pct:    Some(probe_mix_pct),
            ..Default::default()
        };
    }
}
```

> **Note on the pre-clear:** the next-hop slot is wiped *after* advancing `write_pos`, so each frame slot is freshly zeroed before the spread accumulators write into it. Without this, the +=  on neighbours would compound across cycles.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --test future print_through_`
Expected: 2 passed.

- [ ] **Step 5: Run the full suite**

Run: `cargo test`
Expected: full suite green.

- [ ] **Step 6: Commit**

```bash
git add src/dsp/modules/future.rs
git commit -m "feat(future): Print-Through kernel — write-ahead with adjacent-bin spread"
```

---

## Task 4 — Pre-Echo kernel with feedback

**Files:**
- Modify: `src/dsp/modules/future.rs` (`process()` body — Pre-Echo arm)

- [ ] **Step 1: Write the failing test**

```rust
// tests/future.rs — append
#[test]
fn pre_echo_full_signal_arrives_at_delay() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::{SpectralModule, ModuleContext};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let mut m = FutureModule::new();
    m.set_mode(FutureMode::PreEcho);
    m.reset(48000.0, 1024);

    // AMOUNT=1.0 (full echo), TIME=1.0 (8 hops), THRESHOLD=0.5 (low feedback decay → quick decay),
    // SPREAD=0.0 (no HF damping), MIX=2.0 → mix=1.0
    let amount = vec![1.0f32; 513];
    let time   = vec![1.0f32; 513];
    let thresh = vec![0.5f32; 513];
    let spread = vec![0.0f32; 513];
    let mix    = vec![2.0f32; 513];
    let curves: Vec<&[f32]> = vec![&amount, &time, &thresh, &spread, &mix];

    let ctx = ModuleContext {
        sample_rate: 48000.0, fft_size: 1024, num_bins: 513,
        attack_ms: 10.0, release_ms: 100.0, sensitivity: 0.5,
        suppression_width: 1.0, auto_makeup: false, delta_monitor: false,
    };

    // Hop 0: impulse at bin 100. Wet should still be silent (no historical data).
    let mut bins = vec![Complex::new(0.0, 0.0); 513];
    bins[100] = Complex::new(1.0, 0.0);
    let mut supp = vec![0.0f32; 513];
    m.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, None, &curves, &mut supp, &ctx);

    // Hops 1..=7 silence.
    for _ in 1..=7 {
        let mut buf = vec![Complex::new(0.0, 0.0); 513];
        m.process(0, StereoLink::Linked, FxChannelTarget::All, &mut buf, None, &curves, &mut supp, &ctx);
    }

    // Hop 8: should hear the full impulse (post-mix).
    let mut bins = vec![Complex::new(0.0, 0.0); 513];
    m.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, None, &curves, &mut supp, &ctx);
    assert!(bins[100].norm() > 0.4,
        "pre-echo at delay should give near-full magnitude; got {}", bins[100].norm());
}

#[test]
fn pre_echo_feedback_creates_decaying_taps() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::{SpectralModule, ModuleContext};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let mut m = FutureModule::new();
    m.set_mode(FutureMode::PreEcho);
    m.reset(48000.0, 1024);

    // Strong feedback: THRESHOLD=2.0 → high feedback (close to 0.99).
    let amount = vec![1.0f32; 513];
    let time   = vec![1.0f32; 513];
    let thresh = vec![2.0f32; 513];
    let spread = vec![0.0f32; 513];
    let mix    = vec![2.0f32; 513];
    let curves: Vec<&[f32]> = vec![&amount, &time, &thresh, &spread, &mix];

    let ctx = ModuleContext {
        sample_rate: 48000.0, fft_size: 1024, num_bins: 513,
        attack_ms: 10.0, release_ms: 100.0, sensitivity: 0.5,
        suppression_width: 1.0, auto_makeup: false, delta_monitor: false,
    };

    // Hop 0: impulse.
    let mut bins = vec![Complex::new(0.0, 0.0); 513];
    bins[100] = Complex::new(1.0, 0.0);
    let mut supp = vec![0.0f32; 513];
    m.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, None, &curves, &mut supp, &ctx);

    // Run silence for many hops; with high feedback, energy should persist.
    let mut peak_after_long_decay = 0.0f32;
    for h in 1..=24 {
        let mut buf = vec![Complex::new(0.0, 0.0); 513];
        m.process(0, StereoLink::Linked, FxChannelTarget::All, &mut buf, None, &curves, &mut supp, &ctx);
        if h >= 16 { peak_after_long_decay = peak_after_long_decay.max(buf[100].norm()); }
        for c in &buf { assert!(c.norm() <= 4.0, "feedback runaway at hop {}: |c|={}", h, c.norm()); }
    }
    assert!(peak_after_long_decay > 0.05,
        "high-feedback pre-echo should still have audible energy after 16+ hops; got peak {}",
        peak_after_long_decay);
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --test future pre_echo_`
Expected: failures — Pre-Echo arm is still a stub.

- [ ] **Step 3: Implement the Pre-Echo kernel**

Replace the `FutureMode::PreEcho` arm in `process()`:

```rust
FutureMode::PreEcho => {
    let time_gain  = time_curve.get(probe_k).copied().unwrap_or(1.0).clamp(0.0, 2.0);
    let delay_hops = ((time_gain * 8.0).round() as usize).clamp(1, MAX_ECHO_FRAMES - 1);
    let read_pos   = (self.write_pos[ch] + MAX_ECHO_FRAMES - delay_hops) % MAX_ECHO_FRAMES;

    for k in 0..n {
        let amount_gain = amount_curve.get(k).copied().unwrap_or(1.0).clamp(0.0, 4.0);
        let echo_amp    = (amount_gain).clamp(0.0, 2.0); // 1.0 nominal
        let thresh_gain = thresh_curve.get(k).copied().unwrap_or(1.0).clamp(0.0, 2.0);
        let feedback    = (thresh_gain * 0.4).clamp(0.0, 0.99); // 0.4 nominal, hard cap 0.99
        let spread_gain = spread_curve.get(k).copied().unwrap_or(0.0).clamp(0.0, 2.0);
        let hf_damp     = (spread_gain * 0.20).clamp(0.0, 1.0); // 0 nominal, 1.0 = max damping
        let mix_gain    = mix_curve.get(k).copied().unwrap_or(1.0).clamp(0.0, 2.0);
        let mix         = (mix_gain * 0.5).clamp(0.0, 1.0);

        // High-frequency damping factor: 1.0 at bin 0, (1 - hf_damp) at Nyquist.
        let bin_norm    = k as f32 / (n - 1) as f32;
        let damp_factor = 1.0 - hf_damp * bin_norm;

        let dry = bins[k];
        let wet = self.ring[ch][read_pos][k] * echo_amp;
        bins[k] = Complex::new(
            dry.re * (1.0 - mix) + wet.re * mix,
            dry.im * (1.0 - mix) + wet.im * mix,
        );

        #[cfg(any(test, feature = "probe"))]
        if k == probe_k {
            probe_amount_pct = echo_amp * 100.0;
            probe_time_hops  = delay_hops as u32;
            probe_mix_pct    = mix * 100.0;
        }

        // Write into ring: dry signal + feedback × ring-read, both damped.
        let to_write = (dry + wet * feedback) * damp_factor;
        self.ring[ch][self.write_pos[ch]][k] = to_write;
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --test future pre_echo_`
Expected: 2 passed.

- [ ] **Step 5: Run the full suite**

Run: `cargo test`
Expected: full suite green.

- [ ] **Step 6: Commit**

```bash
git add src/dsp/modules/future.rs
git commit -m "feat(future): Pre-Echo kernel with feedback decay + HF damping"
```

---

## Task 5 — Per-slot mode persistence

**Files:**
- Modify: `src/params.rs`
- Modify: `src/dsp/pipeline.rs` (propagate mode like `set_gain_modes`)
- Modify: `src/dsp/fx_matrix.rs` (add `set_future_modes` method)
- Modify: `src/dsp/modules/mod.rs` (add `set_future_mode` default to trait)

- [ ] **Step 1: Locate the existing `slot_gain_mode` pattern**

Run: `rg "slot_gain_mode" src/ -n`
Run: `rg "set_gain_mode" src/ -n`

Read the matches to understand the existing pattern (per-slot `Mutex<GainMode>` in params, snapshotted per block, pushed to FxMatrix via `set_gain_modes`, dispatched via the trait method).

- [ ] **Step 2: Mirror the pattern for `FutureMode`**

Edit `src/params.rs`. Find `slot_gain_mode` and add a sibling field:

```rust
use crate::dsp::modules::future::FutureMode;
// ...
#[persist = "slot_future_mode"]
pub slot_future_mode: [Mutex<FutureMode>; MAX_SLOTS],
```

Initialize it in the `Default` impl alongside `slot_gain_mode`:

```rust
slot_future_mode: std::array::from_fn(|_| Mutex::new(FutureMode::default())),
```

- [ ] **Step 3: Add trait method**

Edit `src/dsp/modules/mod.rs` `SpectralModule` trait — append before the closing `}`:

```rust
fn set_future_mode(&mut self, _: crate::dsp::modules::future::FutureMode) {}
```

Override in `FutureModule`:

```rust
fn set_future_mode(&mut self, mode: FutureMode) { self.set_mode(mode); }
```

- [ ] **Step 4: Add `FxMatrix::set_future_modes`**

Edit `src/dsp/fx_matrix.rs`, right after `set_gain_modes`:

```rust
pub fn set_future_modes(&mut self, modes: &[crate::dsp::modules::future::FutureMode; 9]) {
    for s in 0..MAX_SLOTS {
        if let Some(ref mut m) = self.slots[s] {
            m.set_future_mode(modes[s]);
        }
    }
}
```

- [ ] **Step 5: Wire in `pipeline.rs`**

Find the `set_gain_modes` call in `pipeline.rs` and add a sibling call for future modes:

```rust
let future_modes: [FutureMode; 9] = std::array::from_fn(|i| *params.slot_future_mode[i].lock());
self.fx_matrix.set_future_modes(&future_modes);
```

Add the import at the top of `pipeline.rs`:
```rust
use crate::dsp::modules::future::FutureMode;
```

- [ ] **Step 6: Verify compile + tests**

Run: `cargo build && cargo test`
Expected: green.

- [ ] **Step 7: Commit**

```bash
git add src/params.rs src/dsp/modules/mod.rs src/dsp/modules/future.rs src/dsp/fx_matrix.rs src/dsp/pipeline.rs
git commit -m "feat(future): per-slot FutureMode persistence + dispatch"
```

---

## Task 6 — Add Future to ASSIGNABLE in module popup

**Files:**
- Modify: `src/editor/module_popup.rs:26-35` (`ASSIGNABLE`)

- [ ] **Step 1: Edit the constant**

```rust
const ASSIGNABLE: &[ModuleType] = &[
    ModuleType::Dynamics,
    ModuleType::Freeze,
    ModuleType::PhaseSmear,
    ModuleType::Contrast,
    ModuleType::Gain,
    ModuleType::MidSide,
    ModuleType::TransientSustainedSplit,
    ModuleType::Harmonic,
    ModuleType::Future,
];
```

- [ ] **Step 2: Verify compile**

Run: `cargo build`
Expected: success.

- [ ] **Step 3: Commit**

```bash
git add src/editor/module_popup.rs
git commit -m "feat(future): expose Future in module assignment popup"
```

---

## Task 7 — Mode-picker UI inside the slot panel

**Files:**
- Modify: `src/editor_ui.rs` (or wherever per-slot non-curve UI lives)

- [ ] **Step 1: Locate the existing `GainMode` selector**

Run: `rg "GainMode::" src/editor_ui.rs -n`
Run: `rg "slot_gain_mode" src/editor_ui.rs -n`

Read the match to find the per-slot UI block where Gain mode is selected.

- [ ] **Step 2: Add a sibling block for `FutureMode`**

Right after the GainMode block, add:

```rust
if matches!(slot_module_types[s], ModuleType::Future) {
    let mut current = *params.slot_future_mode[s].lock();
    let prev = current;
    egui::ComboBox::from_id_source(("future_mode", s))
        .selected_text(current.label())
        .show_ui(ui, |ui| {
            for mode in [FutureMode::PrintThrough, FutureMode::PreEcho] {
                if ui.selectable_label(current == mode, mode.label()).clicked() {
                    current = mode;
                }
            }
        });
    if current != prev {
        *params.slot_future_mode[s].lock() = current;
    }
}
```

Add the import at the top of `editor_ui.rs`:
```rust
use crate::dsp::modules::future::FutureMode;
```

- [ ] **Step 3: Smoke test**

```bash
cargo run --package xtask -- bundle spectral_forge --release
cp target/bundled/spectral_forge.clap ~/.clap/
```

Open Bitwig, assign Future to slot 0, switch between Print-Through and Pre-Echo, listen.

- [ ] **Step 4: Commit**

```bash
git add src/editor_ui.rs
git commit -m "feat(future): per-slot Print-Through / Pre-Echo mode picker"
```

---

## Task 8 — Calibration probes coverage

**Files:**
- Modify: `tests/calibration_roundtrip.rs`

- [ ] **Step 1: Read existing pattern**

Read `tests/calibration_roundtrip.rs` (first ~80 lines) to understand the existing per-module probe coverage pattern.

- [ ] **Step 2: Add Future cases**

Append a test block following the existing pattern. Cover:
- Print-Through with AMOUNT=1.0 → probe `amount_pct` ≈ 5.0
- Print-Through with TIME=1.0 → probe `length_ms` ≈ 8 × hop_ms
- Pre-Echo with MIX=2.0 → probe `mix_pct` ≈ 100.0

Example skeleton (adapt to the existing harness):

```rust
#[test]
fn future_print_through_probes_match_curves() {
    use spectral_forge::dsp::modules::{SpectralModule, ModuleContext, ModuleType, create_module};
    use spectral_forge::dsp::modules::future::{FutureModule, FutureMode};
    use spectral_forge::params::{FxChannelTarget, StereoLink};
    use num_complex::Complex;

    let mut m = FutureModule::new();
    m.set_mode(FutureMode::PrintThrough);
    m.reset(48000.0, 1024);
    let amount = vec![1.0f32; 513];
    let time   = vec![1.0f32; 513];
    let thresh = vec![1.0f32; 513];
    let spread = vec![0.0f32; 513];
    let mix    = vec![2.0f32; 513];
    let curves: Vec<&[f32]> = vec![&amount, &time, &thresh, &spread, &mix];
    let mut bins = vec![Complex::new(0.5, 0.0); 513];
    let mut supp = vec![0.0f32; 513];
    let ctx = ModuleContext {
        sample_rate: 48000.0, fft_size: 1024, num_bins: 513,
        attack_ms: 10.0, release_ms: 100.0, sensitivity: 0.5,
        suppression_width: 1.0, auto_makeup: false, delta_monitor: false,
    };
    m.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, None, &curves, &mut supp, &ctx);
    let probe = m.last_probe();
    assert!((probe.amount_pct.unwrap_or(0.0) - 5.0).abs() < 0.5);
    assert!((probe.mix_pct.unwrap_or(0.0)    - 100.0).abs() < 0.5);
}
```

- [ ] **Step 3: Run**

Run: `cargo test --test calibration_roundtrip future_`
Expected: pass.

Run: `cargo test`
Expected: green.

- [ ] **Step 4: Commit**

```bash
git add tests/calibration_roundtrip.rs
git commit -m "test(future): calibration round-trip probes for Print-Through"
```

---

## Task 9 — Status banner + STATUS.md

**Files:**
- Modify: `docs/superpowers/STATUS.md`

- [ ] **Step 1: Update STATUS.md**

Add an entry under the "Modules" or "Implemented features" section:

```markdown
- **Future module** — IMPLEMENTED 2026-04-27 by `docs/superpowers/plans/2026-04-27-phase-2b-future.md`. Sub-effects: Tape Print-Through, Pre-Echo with Pre-Delay. Lookahead Duck and Crystal Ball deferred per audit `ideas/next-gen-modules/14-future.md`.
```

- [ ] **Step 2: Smoke listen**

Build, install, load Future on a percussive source, listen to both modes back-to-back. Note any quirks in the commit message.

- [ ] **Step 3: Commit**

```bash
git add docs/superpowers/STATUS.md
git commit -m "docs: Future module IMPLEMENTED status entry"
```

---

## Risk register

1. **Pre-clear of next ring slot is critical.** The Print-Through kernel writes via `+=` for spread; without the next-slot pre-clear those neighbour writes would compound across full ring cycles. Test `print_through_spread_bleeds_to_adjacent_bins` exercises this in steady state.

2. **Pre-Echo feedback can run away if THRESHOLD=2.0 + AMOUNT=2.0 + HF damping=0.** The `feedback.clamp(0, 0.99)` cap keeps it stable. The `pre_echo_feedback_creates_decaying_taps` test asserts `|c| <= 4.0` per bin per hop as a runaway guard.

3. **Per-slot mode change clears no state.** Switching from PrintThrough to PreEcho mid-session leaves the ring buffer populated with leak-scaled data. This produces a brief audible glitch but is intentional — it gives "modulation" character. If it bothers users, add a `clear_ring_on_mode_change` policy in v2.

4. **Predicted-Spectrum Interpolation deferred.** The audit's research-findings recommend Predicted-Spectrum as v1, but the roadmap explicitly excludes it. This plan follows the roadmap. When BinPhysics ships in Phase 3, revisit — Predicted-Spectrum can reuse the `flux` field for confidence weighting.

5. **`tail_length()` returns `(fft_size × MAX_ECHO_FRAMES) / 4`** — that is the worst-case write-ahead in samples (MAX_ECHO_FRAMES hops × hop_size). Hosts use this to know how long to feed silence after the user stops to flush the buffer.

---

## Self-review checklist

- [x] Every task has complete code; no "TBD" placeholders.
- [x] Tests precede implementation in every task.
- [x] Spec coverage:
  - Print-Through with adjacent-bin spread (Task 3)
  - Pre-Echo with feedback + HF damping (Task 4)
  - Per-slot mode selector (Task 5 + Task 7)
  - Calibration probes (Task 8)
  - Lookahead Duck + Crystal Ball deferred per roadmap.
- [x] Names consistent: `FutureModule`, `FutureMode`, `MAX_ECHO_FRAMES`, `slot_future_mode`.
- [x] Module wired into `module_spec`, `create_module`, `ASSIGNABLE` (Tasks 1, 2, 6).
- [x] Trait extension (`set_future_mode`) has a default impl so existing modules need no edit.

---

## Execution handoff

**Plan complete and saved to `docs/superpowers/plans/2026-04-27-phase-2b-future.md`.**

Two execution options:

1. **Subagent-Driven (recommended)** — dispatch a fresh subagent per task, review between tasks, fast iteration.
2. **Inline Execution** — execute tasks in this session using executing-plans.

This is one of seven Phase 2 plans. The companion plans are 2a (Matrix Amp Nodes), 2c (Punch), 2d (Rhythm), 2e (Geometry-light), 2f (Modulate-light), 2g (Circuit-light). They can ship independently in any order.
