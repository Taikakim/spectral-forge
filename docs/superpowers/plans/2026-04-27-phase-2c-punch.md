# Phase 2c — Punch Module Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship a new `Punch` module that uses a sidechain peak detector to carve "holes" in the input spectrum at sidechain peak frequencies; neighbour bins fill the hole via amplitude boost (mandatory) and optional pitch drift. Two modes: **Direct** (loud-sidechain → carve here) and **Inverse** (quiet-sidechain → carve here). Self-Punch is deferred per the audit.

**Architecture:** Plain `SpectralModule` slot with sidechain. Owns its own peak-detection and per-bin state arrays (~66 KB per channel). No external infra dependencies beyond the existing sidechain plumbing and `ModuleContext.sample_rate`. Mode (`Direct`/`Inverse`) is per-slot; both modes share the same kernel with one inversion. The amp-fill smoothing follower (`τ = 5 ms`) and healing follower (default `τ = 150 ms`) are 1-pole; the pitch-drift uses cached `exp(jΔφ)` per active drift site, capped at `|d| ≤ 0.5` bins per the audit's research findings.

**Tech Stack:** Rust, num_complex, nih-plug.

**Source design:** `ideas/next-gen-modules/19-punch.md` (audit + ratified research findings). No prior spec — this plan also doubles as the spec.

**Roadmap reference:** `ideas/next-gen-modules/99-implementation-roadmap.md` § Phase 2 (item 3).

**Depends on:** Phase 1 (`ModuleSpec.wants_sidechain` field).

---

## File structure

| File | Status | Responsibility |
|---|---|---|
| `src/dsp/modules/punch.rs` | Create | `PunchModule`, `PunchMode` enum, peak detection, carve+fill kernel, per-bin state. |
| `src/dsp/modules/mod.rs` | Modify | Add `Punch` to `ModuleType`, `PUNCH` static `ModuleSpec` with `wants_sidechain: true`. |
| `src/editor/module_popup.rs` | Modify | Add `ModuleType::Punch` to `ASSIGNABLE`. |
| `src/params.rs` | Modify | Add `slot_punch_mode: [Mutex<PunchMode>; MAX_SLOTS]` (mirrors `slot_gain_mode`). |
| `tests/punch.rs` | Create | Peak detector unit tests, carve depth, fill smoothing, healing time-constant, pitch-drift cap. |
| `tests/calibration_roundtrip.rs` | Modify | Punch case for probe coverage. |

---

## Curve mapping (6 curves)

| Idx | Label | Direct mode | Inverse mode |
|---|---|---|---|
| 0 | AMOUNT | Carve depth (0–1, neutral=0.5) | Carve depth (same) |
| 1 | WIDTH | Neighbour-bin width (1–16, neutral=4) | (same) |
| 2 | FILL_MODE | Pitch-fill drift toward hole (0–0.5 bins, neutral=0) | (same) |
| 3 | AMP_FILL | Neighbour amp boost (0–2.0, neutral=1.0 → unity) | (same) |
| 4 | HEAL | Healing time-constant (20–2000 ms, neutral=150 ms) | (same) |
| 5 | MIX | Wet (0–1, neutral=0.5) | (same) |

`num_curves() = 6`. Inversion only flips the sidechain-peak → carve-here mapping; all other curves behave identically.

---

## Task 1 — Add `ModuleType::Punch` + spec with `wants_sidechain: true`

**Files:**
- Modify: `src/dsp/modules/mod.rs`

**Pre-req check:** Phase 1 (`ModuleSpec.wants_sidechain`) must be landed. Verify with:

```bash
rg "wants_sidechain" src/dsp/modules/mod.rs -n
```

Expected: at least one match in the `ModuleSpec` struct definition.

If not landed yet, **stop**: this plan is blocked on Phase 1.

- [ ] **Step 1: Write the failing test**

```rust
// tests/punch.rs (NEW)
use spectral_forge::dsp::modules::{ModuleType, module_spec};

#[test]
fn punch_module_spec() {
    let spec = module_spec(ModuleType::Punch);
    assert_eq!(spec.num_curves, 6);
    assert_eq!(spec.curve_labels, &["AMOUNT", "WIDTH", "FILL_MODE", "AMP_FILL", "HEAL", "MIX"]);
    assert!(spec.supports_sidechain);
    assert!(spec.wants_sidechain);
    assert_eq!(spec.display_name, "Punch");
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --test punch punch_module_spec`
Expected: compile error — `ModuleType::Punch` does not exist.

- [ ] **Step 3: Add the variant + spec**

Edit `src/dsp/modules/mod.rs`:

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
    Punch,         // NEW
    Master,
}
```

In `module_spec()`:

```rust
static PUNCH: ModuleSpec = ModuleSpec {
    display_name: "Punch",
    color_lit: Color32::from_rgb(0xe0, 0x70, 0x60),
    color_dim: Color32::from_rgb(0x48, 0x20, 0x20),
    num_curves: 6,
    curve_labels: &["AMOUNT", "WIDTH", "FILL_MODE", "AMP_FILL", "HEAL", "MIX"],
    supports_sidechain: true,
    wants_sidechain:    true,  // ← NEW field from Phase 1
};
// ...
ModuleType::Punch                  => &PUNCH,
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --test punch punch_module_spec`
Expected: 1 passed.

- [ ] **Step 5: Commit**

```bash
git add src/dsp/modules/mod.rs tests/punch.rs
git commit -m "feat(punch): ModuleType::Punch + ModuleSpec wants_sidechain"
```

---

## Task 2 — Skeleton `PunchModule` + `PunchMode` enum

**Files:**
- Create: `src/dsp/modules/punch.rs`
- Modify: `src/dsp/modules/mod.rs` (`pub mod punch;`, `create_module` arm)

- [ ] **Step 1: Write the failing test**

```rust
// tests/punch.rs — append
use spectral_forge::dsp::modules::punch::{PunchModule, PunchMode};

#[test]
fn punch_mode_default_is_direct() {
    assert_eq!(PunchMode::default(), PunchMode::Direct);
}

#[test]
fn punch_module_no_sidechain_is_passthrough() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::{SpectralModule, ModuleContext};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let mut m = PunchModule::new();
    m.reset(48000.0, 1024);
    let mut bins = vec![Complex::new(0.5, 0.1); 513];
    let original = bins.clone();
    let curves_storage: Vec<Vec<f32>> = (0..6).map(|_| vec![1.0f32; 513]).collect();
    let curves: Vec<&[f32]> = curves_storage.iter().map(|v| v.as_slice()).collect();
    let mut supp = vec![0.0f32; 513];
    let ctx = ModuleContext {
        sample_rate: 48000.0, fft_size: 1024, num_bins: 513,
        attack_ms: 10.0, release_ms: 100.0, sensitivity: 0.5,
        suppression_width: 1.0, auto_makeup: false, delta_monitor: false,
    };
    // No sidechain → no carve → output ≈ input
    m.process(0, StereoLink::Linked, FxChannelTarget::All,
        &mut bins, None, &curves, &mut supp, &ctx);
    for (a, b) in bins.iter().zip(original.iter()) {
        assert!((a.re - b.re).abs() < 1e-4 && (a.im - b.im).abs() < 1e-4,
            "no-sidechain should be transparent");
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --test punch punch_mode_default_is_direct`
Expected: compile error — `PunchModule` does not exist.

- [ ] **Step 3: Create the skeleton**

```rust
// src/dsp/modules/punch.rs (NEW)
use num_complex::Complex;
use serde::{Deserialize, Serialize};
use crate::params::{FxChannelTarget, StereoLink};
use super::{ModuleContext, ModuleType, SpectralModule};

pub const MAX_PEAKS: usize = 32;
pub const MAX_DRIFT_SITES: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum PunchMode {
    #[default]
    Direct,
    Inverse,
}

impl PunchMode {
    pub fn label(self) -> &'static str {
        match self {
            PunchMode::Direct  => "Direct",
            PunchMode::Inverse => "Inverse",
        }
    }
}

pub struct PunchModule {
    mode:        PunchMode,
    fft_size:    usize,
    sample_rate: f32,
    /// Per-channel × per-bin state.
    /// `current_carve_depth[ch][k]`: smoothed depth applied this hop (0 = no carve, 1 = full mute).
    pub current_carve_depth: [Vec<f32>; 2],
    /// Per-channel × per-bin pitch-drift accumulator (in fractional bins).
    pub drift_accum:         [Vec<f32>; 2],
    /// Scratch buffer for sidechain peak magnitudes; `peak_bin[i]` is the bin index of the i-th peak.
    peak_bin:                [u32; MAX_PEAKS],
    peak_count:              usize,
    #[cfg(any(test, feature = "probe"))]
    last_probe:              crate::dsp::modules::ProbeSnapshot,
}

impl PunchModule {
    pub fn new() -> Self {
        Self {
            mode:        PunchMode::default(),
            fft_size:    2048,
            sample_rate: 44100.0,
            current_carve_depth: [Vec::new(), Vec::new()],
            drift_accum:         [Vec::new(), Vec::new()],
            peak_bin:            [0u32; MAX_PEAKS],
            peak_count:          0,
            #[cfg(any(test, feature = "probe"))]
            last_probe: Default::default(),
        }
    }

    pub fn set_mode(&mut self, mode: PunchMode) { self.mode = mode; }
    pub fn mode(&self) -> PunchMode { self.mode }
}

impl Default for PunchModule {
    fn default() -> Self { Self::new() }
}

impl SpectralModule for PunchModule {
    fn reset(&mut self, sample_rate: f32, fft_size: usize) {
        self.sample_rate = sample_rate;
        self.fft_size    = fft_size;
        let n = fft_size / 2 + 1;
        for ch in 0..2 {
            self.current_carve_depth[ch] = vec![0.0; n];
            self.drift_accum[ch]         = vec![0.0; n];
        }
        self.peak_count = 0;
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
        // Stub. Tasks 3-6 implement peak detection, carve, fill, healing.
        suppression_out.fill(0.0);
        let _ = bins;
    }

    fn module_type(&self) -> ModuleType { ModuleType::Punch }
    fn num_curves(&self) -> usize { 6 }

    #[cfg(any(test, feature = "probe"))]
    fn last_probe(&self) -> crate::dsp::modules::ProbeSnapshot { self.last_probe }
}
```

Edit `src/dsp/modules/mod.rs`:

```rust
pub mod punch;
// ...
ModuleType::Punch                  => Box::new(punch::PunchModule::new()),
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --test punch`
Expected: 3 passed.

- [ ] **Step 5: Commit**

```bash
git add src/dsp/modules/punch.rs src/dsp/modules/mod.rs tests/punch.rs
git commit -m "feat(punch): PunchModule skeleton + PunchMode enum"
```

---

## Task 3 — Sidechain peak detection

**Files:**
- Modify: `src/dsp/modules/punch.rs`
- Test: `tests/punch.rs`

- [ ] **Step 1: Write the failing test**

```rust
// tests/punch.rs — append
#[test]
fn detect_peaks_finds_local_maxima_above_threshold() {
    use spectral_forge::dsp::modules::punch::detect_peaks;

    let mut sc = vec![0.0f32; 64];
    sc[10] = 0.9;
    sc[20] = 0.5;
    sc[30] = 0.95;
    sc[40] = 0.1; // below threshold
    let mut peaks = [0u32; 32];
    let count = detect_peaks(&sc, &mut peaks, 0.3, 8); // threshold 0.3, min_dist 8

    assert!(count >= 3);
    let bins: std::collections::HashSet<u32> = peaks[..count].iter().copied().collect();
    assert!(bins.contains(&10));
    assert!(bins.contains(&20));
    assert!(bins.contains(&30));
    assert!(!bins.contains(&40));
}

#[test]
fn detect_peaks_enforces_minimum_distance() {
    use spectral_forge::dsp::modules::punch::detect_peaks;

    let mut sc = vec![0.0f32; 64];
    sc[10] = 0.5;
    sc[12] = 0.6; // too close to bin 10 — should be suppressed if 10 wins
    sc[30] = 0.7;
    let mut peaks = [0u32; 32];
    let count = detect_peaks(&sc, &mut peaks, 0.3, 8);

    let bins: std::collections::HashSet<u32> = peaks[..count].iter().copied().collect();
    // Highest in the cluster wins (bin 12), and bin 10 is excluded by the min_dist veto.
    assert!(bins.contains(&12));
    assert!(!bins.contains(&10));
    assert!(bins.contains(&30));
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --test punch detect_peaks_`
Expected: compile error — `detect_peaks` does not exist.

- [ ] **Step 3: Implement `detect_peaks`**

Append to `src/dsp/modules/punch.rs`:

```rust
/// Detect up to `out.len()` local maxima in `mag`, above `threshold`, separated by
/// at least `min_dist` bins. Returns the number of peaks written.
/// Greedy: for each local max above threshold, sort by magnitude desc and skip any
/// that fall within `min_dist` of an already-accepted higher peak.
pub fn detect_peaks(mag: &[f32], out: &mut [u32], threshold: f32, min_dist: usize) -> usize {
    let n = mag.len();
    if n < 3 || out.is_empty() { return 0; }
    // First pass: collect (mag, bin) candidates that are local maxima above threshold.
    // Use a small fixed-size scratch to avoid allocations on the audio thread.
    let mut cand_count = 0usize;
    let mut cand_mag: [f32; 256] = [0.0; 256];
    let mut cand_bin: [u32; 256] = [0; 256];
    for k in 1..n - 1 {
        let m = mag[k];
        if m < threshold { continue; }
        if m > mag[k - 1] && m >= mag[k + 1] {
            if cand_count < cand_mag.len() {
                cand_mag[cand_count] = m;
                cand_bin[cand_count] = k as u32;
                cand_count += 1;
            }
        }
    }
    // Second pass: greedy selection by descending magnitude, enforcing min_dist.
    // Insertion sort by descending magnitude.
    for i in 1..cand_count {
        let mi = cand_mag[i];
        let bi = cand_bin[i];
        let mut j = i;
        while j > 0 && cand_mag[j - 1] < mi {
            cand_mag[j] = cand_mag[j - 1];
            cand_bin[j] = cand_bin[j - 1];
            j -= 1;
        }
        cand_mag[j] = mi;
        cand_bin[j] = bi;
    }
    let mut written = 0usize;
    for i in 0..cand_count {
        if written >= out.len() { break; }
        let b = cand_bin[i];
        let mut ok = true;
        for j in 0..written {
            if (out[j] as i64 - b as i64).unsigned_abs() < min_dist as u64 {
                ok = false;
                break;
            }
        }
        if ok {
            out[written] = b;
            written += 1;
        }
    }
    written
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --test punch detect_peaks_`
Expected: 2 passed.

- [ ] **Step 5: Commit**

```bash
git add src/dsp/modules/punch.rs tests/punch.rs
git commit -m "feat(punch): per-hop sidechain peak detector"
```

---

## Task 4 — Carve kernel (Direct + Inverse) without fill or healing

**Files:**
- Modify: `src/dsp/modules/punch.rs` (`process()` body)
- Test: `tests/punch.rs`

- [ ] **Step 1: Write the failing tests**

```rust
// tests/punch.rs — append
#[test]
fn direct_punch_carves_at_sidechain_peaks() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::{SpectralModule, ModuleContext};
    use spectral_forge::dsp::modules::punch::{PunchModule, PunchMode};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let mut m = PunchModule::new();
    m.set_mode(PunchMode::Direct);
    m.reset(48000.0, 1024);

    let mut sc = vec![0.0f32; 513];
    sc[100] = 0.9; // strong peak at bin 100

    // AMOUNT=2.0 (full carve), WIDTH=1.0 (4 bins each side), FILL_MODE=1.0 (no pitch),
    // AMP_FILL=1.0 (unity, no boost), HEAL=0.13 (≈20ms, snappy), MIX=2.0 (full wet)
    let amount = vec![2.0f32; 513];
    let width  = vec![1.0f32; 513];
    let fillm  = vec![1.0f32; 513];
    let ampfl  = vec![1.0f32; 513];
    let heal   = vec![0.13f32; 513];
    let mix    = vec![2.0f32; 513];
    let curves: Vec<&[f32]> = vec![&amount, &width, &fillm, &ampfl, &heal, &mix];

    let mut bins = vec![Complex::new(1.0, 0.0); 513];
    let mut supp = vec![0.0f32; 513];
    let ctx = ModuleContext {
        sample_rate: 48000.0, fft_size: 1024, num_bins: 513,
        attack_ms: 10.0, release_ms: 100.0, sensitivity: 0.5,
        suppression_width: 1.0, auto_makeup: false, delta_monitor: false,
    };
    m.process(0, StereoLink::Linked, FxChannelTarget::All,
        &mut bins, Some(&sc), &curves, &mut supp, &ctx);

    // Bin 100 should be heavily attenuated by the carve.
    assert!(bins[100].norm() < 0.5,
        "direct punch should carve bin 100; got {}", bins[100].norm());
    // Far-away bin 200 should be untouched.
    assert!((bins[200].norm() - 1.0).abs() < 0.1,
        "far-away bin should be untouched; got {}", bins[200].norm());
}

#[test]
fn inverse_punch_carves_where_sidechain_quiet() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::{SpectralModule, ModuleContext};
    use spectral_forge::dsp::modules::punch::{PunchModule, PunchMode};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let mut m = PunchModule::new();
    m.set_mode(PunchMode::Inverse);
    m.reset(48000.0, 1024);

    // Build a sidechain with one peak at bin 100; inverse mode means bin 100 is "safe"
    // and other bins (where SC is quiet) get carved. To make this testable, push
    // SC magnitude high at bin 100 and low elsewhere. Inverse → carve at bin 200, not 100.
    let mut sc = vec![0.05f32; 513];
    sc[100] = 0.9;

    let amount = vec![2.0f32; 513];
    let width  = vec![1.0f32; 513];
    let fillm  = vec![1.0f32; 513];
    let ampfl  = vec![1.0f32; 513];
    let heal   = vec![0.13f32; 513];
    let mix    = vec![2.0f32; 513];
    let curves: Vec<&[f32]> = vec![&amount, &width, &fillm, &ampfl, &heal, &mix];

    let mut bins = vec![Complex::new(1.0, 0.0); 513];
    let mut supp = vec![0.0f32; 513];
    let ctx = ModuleContext {
        sample_rate: 48000.0, fft_size: 1024, num_bins: 513,
        attack_ms: 10.0, release_ms: 100.0, sensitivity: 0.5,
        suppression_width: 1.0, auto_makeup: false, delta_monitor: false,
    };
    m.process(0, StereoLink::Linked, FxChannelTarget::All,
        &mut bins, Some(&sc), &curves, &mut supp, &ctx);

    // Bin 100 (sidechain peak) should be preserved in Inverse mode.
    assert!(bins[100].norm() > 0.7,
        "inverse punch should preserve bin where SC is loud; got {}", bins[100].norm());
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --test punch direct_punch_ inverse_punch_`
Expected: failures — kernel is a stub.

- [ ] **Step 3: Implement the carve kernel**

Replace the stub `process()` body:

```rust
fn process(
    &mut self,
    channel: usize,
    _stereo_link: StereoLink,
    _target: FxChannelTarget,
    bins: &mut [Complex<f32>],
    sidechain: Option<&[f32]>,
    curves: &[&[f32]],
    suppression_out: &mut [f32],
    ctx: &ModuleContext,
) {
    let ch = channel.min(1);
    let n  = bins.len();

    // Resize state if num_bins changed (e.g. FFT-size change).
    if self.current_carve_depth[ch].len() != n {
        self.current_carve_depth[ch].resize(n, 0.0);
    }
    if self.drift_accum[ch].len() != n {
        self.drift_accum[ch].resize(n, 0.0);
    }

    let probe_k = n / 2;

    let amount_curve = curves.get(0).copied().unwrap_or(&[][..]);
    let width_curve  = curves.get(1).copied().unwrap_or(&[][..]);
    let fillm_curve  = curves.get(2).copied().unwrap_or(&[][..]);
    let ampfl_curve  = curves.get(3).copied().unwrap_or(&[][..]);
    let heal_curve   = curves.get(4).copied().unwrap_or(&[][..]);
    let mix_curve    = curves.get(5).copied().unwrap_or(&[][..]);

    #[cfg(any(test, feature = "probe"))]
    let mut probe_amount_pct = 0.0f32;
    #[cfg(any(test, feature = "probe"))]
    let mut probe_mix_pct    = 0.0f32;

    // ── Detect peaks in the sidechain (or skip if absent) ────────────────────
    self.peak_count = 0;
    if let Some(sc) = sidechain {
        // Read bin-probe_k values for the slot-wide peak-detection params.
        let amount_g = amount_curve.get(probe_k).copied().unwrap_or(1.0);
        let threshold = 0.05_f32 + (1.0 - amount_g.clamp(0.0, 2.0) / 2.0) * 0.25; // 0.05..0.30
        let width_g  = width_curve.get(probe_k).copied().unwrap_or(1.0).clamp(0.0, 2.0);
        let min_dist = ((width_g * 4.0).round() as usize).max(2); // min separation

        match self.mode {
            PunchMode::Direct  => {
                self.peak_count = detect_peaks(&sc[..n.min(sc.len())],
                    &mut self.peak_bin, threshold, min_dist);
            }
            PunchMode::Inverse => {
                // Inverse: build inverted-magnitude scratch in drift_accum (re-use; we'll overwrite below).
                let scratch = &mut self.current_carve_depth[ch];
                for k in 0..n.min(sc.len()) {
                    scratch[k] = (1.0 - sc[k]).max(0.0);
                }
                self.peak_count = detect_peaks(scratch, &mut self.peak_bin, threshold, min_dist);
                // Reset scratch we just used (don't carry the inverted-SC values into the carve loop).
                for k in 0..n { scratch[k] = self.current_carve_depth[ch][k] * 0.0; } // = 0
                // (above reset is intentional: we're about to recompute current_carve_depth from scratch
                //  via the smoothing follower)
            }
        }
    }

    // ── Build carve-target depth per bin ─────────────────────────────────────
    // For each peak, set carve_target around it within ±width bins.
    let width_g  = width_curve.get(probe_k).copied().unwrap_or(1.0).clamp(0.0, 2.0);
    let half_w   = ((width_g * 4.0).round() as usize).max(1).min(16);
    // Target buffer: stack-stored max 16 bins each side per peak — use a small scratch
    // We will compute target on-the-fly per bin via min-distance to any peak.

    // ── Apply carve, amp-fill, healing follower, and mix ─────────────────────
    let hop_dt = ctx.fft_size as f32 / ctx.sample_rate / 4.0; // OVERLAP=4

    // Smoothing α for the depth follower (~5 ms attack/release):
    let smooth_a = (-hop_dt / 0.005).exp();

    for k in 0..n {
        let amount_g = amount_curve.get(k).copied().unwrap_or(1.0).clamp(0.0, 2.0);
        let depth    = (amount_g * 0.5).clamp(0.0, 1.0); // neutral=0.5
        let ampfl_g  = ampfl_curve.get(k).copied().unwrap_or(1.0).clamp(0.0, 4.0);
        let amp_fill = ampfl_g.clamp(0.0, 4.0); // neutral=1.0
        let heal_g   = heal_curve.get(k).copied().unwrap_or(1.0).clamp(0.05, 2.0);
        let heal_ms  = (heal_g * 150.0).clamp(20.0, 2000.0);
        let mix_g    = mix_curve.get(k).copied().unwrap_or(1.0).clamp(0.0, 2.0);
        let mix      = (mix_g * 0.5).clamp(0.0, 1.0);

        // Determine target carve depth: 1.0 if k is at a peak, fading to 0 within half_w.
        let mut target = 0.0f32;
        for i in 0..self.peak_count {
            let pk = self.peak_bin[i] as i64;
            let dist = (k as i64 - pk).unsigned_abs() as usize;
            if dist <= half_w {
                let weight = 1.0 - (dist as f32) / ((half_w + 1) as f32);
                let t = depth * weight;
                if t > target { target = t; }
            }
        }

        // Apply attack/release follower with healing time-constant on release.
        let release_a = (-hop_dt / (heal_ms * 0.001)).exp();
        let prev = self.current_carve_depth[ch][k];
        let cur  = if target > prev {
            // attack: snap fast (5 ms)
            smooth_a * prev + (1.0 - smooth_a) * target
        } else {
            // release: heal slow
            release_a * prev + (1.0 - release_a) * target
        };
        self.current_carve_depth[ch][k] = cur;

        // Compute neighbour amp-fill: bins near a peak (but not at it) get boosted.
        // For simplicity, every bin within half_w gets amp_fill scaling weighted by carve depth.
        let mut neighbour_boost = 1.0f32;
        for i in 0..self.peak_count {
            let pk = self.peak_bin[i] as i64;
            let dist = (k as i64 - pk).unsigned_abs() as usize;
            if dist > 0 && dist <= half_w {
                let w = 1.0 - (dist as f32) / ((half_w + 1) as f32);
                neighbour_boost = neighbour_boost.max(1.0 + (amp_fill - 1.0) * w);
            }
        }

        let dry = bins[k];
        let carved = dry * (1.0 - cur) * neighbour_boost;
        let wet    = carved;
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

    suppression_out.fill(0.0);
    // Pitch-fill (FILL_MODE curve > 0) implemented in Task 5.
    let _ = fillm_curve;

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

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --test punch direct_punch_ inverse_punch_`
Expected: 2 passed.

Run: `cargo test`
Expected: full suite green.

- [ ] **Step 5: Commit**

```bash
git add src/dsp/modules/punch.rs
git commit -m "feat(punch): carve kernel + amp-fill + healing follower (Direct + Inverse)"
```

---

## Task 5 — Pitch-fill (sub-bin drift toward hole)

**Files:**
- Modify: `src/dsp/modules/punch.rs` (extend `process()`)
- Test: `tests/punch.rs`

- [ ] **Step 1: Write the failing test**

```rust
// tests/punch.rs — append
#[test]
fn pitch_fill_caps_drift_at_half_bin() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::{SpectralModule, ModuleContext};
    use spectral_forge::dsp::modules::punch::{PunchModule, PunchMode};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let mut m = PunchModule::new();
    m.set_mode(PunchMode::Direct);
    m.reset(48000.0, 1024);

    let mut sc = vec![0.0f32; 513];
    sc[100] = 0.9;

    let amount = vec![2.0f32; 513];
    let width  = vec![1.0f32; 513];
    let fillm  = vec![2.0f32; 513];   // max pitch fill
    let ampfl  = vec![1.0f32; 513];
    let heal   = vec![0.13f32; 513];
    let mix    = vec![2.0f32; 513];
    let curves: Vec<&[f32]> = vec![&amount, &width, &fillm, &ampfl, &heal, &mix];

    // Run 50 hops with the same input — pitch drift should accumulate but cap at 0.5 bins.
    let mut bins;
    let mut supp = vec![0.0f32; 513];
    let ctx = ModuleContext {
        sample_rate: 48000.0, fft_size: 1024, num_bins: 513,
        attack_ms: 10.0, release_ms: 100.0, sensitivity: 0.5,
        suppression_width: 1.0, auto_makeup: false, delta_monitor: false,
    };
    for _ in 0..50 {
        bins = vec![Complex::new(1.0, 0.0); 513];
        m.process(0, StereoLink::Linked, FxChannelTarget::All,
            &mut bins, Some(&sc), &curves, &mut supp, &ctx);
    }
    // Inspect the per-channel drift_accum array (test-only public).
    for k in 0..513 {
        assert!(m.drift_accum[0][k].abs() <= 0.5 + 1e-4,
            "drift at bin {} = {} exceeded 0.5", k, m.drift_accum[0][k]);
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --test punch pitch_fill_caps_drift_at_half_bin`
Expected: failure — drift accumulates without cap (or stays at zero — Task 4 doesn't touch drift).

- [ ] **Step 3: Implement pitch-fill**

In `src/dsp/modules/punch.rs`, modify `process()` to update `drift_accum` and apply phase rotation.

Add inside the existing `for k in 0..n` loop, after the `self.current_carve_depth[ch][k] = cur;` line:

```rust
        // ── Pitch-fill: drift neighbour bins toward the nearest peak ────────
        let fillm_g = fillm_curve.get(k).copied().unwrap_or(1.0).clamp(0.0, 2.0);
        let target_drift = if fillm_g > 1e-3 {
            let mut best: Option<(usize, i64)> = None; // (peak_index, signed distance)
            for i in 0..self.peak_count {
                let pk = self.peak_bin[i] as i64;
                let signed = pk - k as i64; // positive: peak is above us → drift up
                let dist = signed.unsigned_abs() as usize;
                if dist > 0 && dist <= half_w {
                    if best.map(|(_, d)| (d.unsigned_abs() as usize) > dist).unwrap_or(true) {
                        best = Some((i, signed));
                    }
                }
            }
            if let Some((_, signed)) = best {
                // Drift fraction: scale by FILL_MODE (0..1 maps to 0..0.5 bin cap).
                let direction = (signed as f32).signum();
                direction * (fillm_g * 0.25).clamp(0.0, 0.5)
            } else { 0.0 }
        } else { 0.0 };

        // Slew-rate limit drift to 2 cents per hop equivalent.
        // 2 cents at bin k corresponds to ~0.001 bin per hop; we use a fixed 0.005 bin/hop limit
        // (overshoot OK; the cap below clamps to 0.5).
        let prev_drift = self.drift_accum[ch][k];
        let drift_step = (target_drift - prev_drift).clamp(-0.005, 0.005);
        let new_drift  = (prev_drift + drift_step).clamp(-0.5, 0.5);
        self.drift_accum[ch][k] = new_drift;

        // Apply phase rotation: Δφ per hop = 2π × Δf × hop / sample_rate.
        // A drift of `d` bins shifts frequency by d × (sample_rate / fft_size).
        // Per-hop phase rotation = 2π × d × hop / fft_size = 2π × d / 4 = (π/2)·d at OVERLAP=4.
        if new_drift.abs() > 1e-6 {
            let dphi = std::f32::consts::FRAC_PI_2 * new_drift;
            let (s, c) = dphi.sin_cos();
            let rot = Complex::new(c, s);
            bins[k] = bins[k] * rot;
        }
```

(Place this block **after** the carved/wet/mix assignment so drift modulates the final mix.)

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --test punch pitch_fill_caps_drift_at_half_bin`
Expected: pass.

Run: `cargo test`
Expected: green.

- [ ] **Step 5: Commit**

```bash
git add src/dsp/modules/punch.rs tests/punch.rs
git commit -m "feat(punch): sub-bin pitch-fill drift with 0.5-bin cap"
```

---

## Task 6 — Healing time-constant test (regression-grade)

**Files:**
- Test: `tests/punch.rs`

- [ ] **Step 1: Write the test**

```rust
// tests/punch.rs — append
#[test]
fn healing_follows_chosen_time_constant() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::{SpectralModule, ModuleContext};
    use spectral_forge::dsp::modules::punch::{PunchModule, PunchMode};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let mut m = PunchModule::new();
    m.set_mode(PunchMode::Direct);
    m.reset(48000.0, 1024);

    let amount = vec![2.0f32; 513];
    let width  = vec![1.0f32; 513];
    let fillm  = vec![1.0f32; 513];
    let ampfl  = vec![1.0f32; 513];
    let heal   = vec![1.0f32; 513];   // neutral = 150 ms
    let mix    = vec![2.0f32; 513];
    let curves: Vec<&[f32]> = vec![&amount, &width, &fillm, &ampfl, &heal, &mix];
    let ctx = ModuleContext {
        sample_rate: 48000.0, fft_size: 1024, num_bins: 513,
        attack_ms: 10.0, release_ms: 100.0, sensitivity: 0.5,
        suppression_width: 1.0, auto_makeup: false, delta_monitor: false,
    };

    // Phase 1: drive with a strong sidechain for 30 hops to engage carve.
    let mut sc = vec![0.0f32; 513]; sc[100] = 0.9;
    let mut supp = vec![0.0f32; 513];
    for _ in 0..30 {
        let mut bins = vec![Complex::new(1.0, 0.0); 513];
        m.process(0, StereoLink::Linked, FxChannelTarget::All,
            &mut bins, Some(&sc), &curves, &mut supp, &ctx);
    }
    let depth_engaged = m.current_carve_depth[0][100];
    assert!(depth_engaged > 0.3, "carve should be engaged; got {}", depth_engaged);

    // Phase 2: silence the sidechain, run for ~150 ms (~14 hops at 48k/256-hop) and check
    // that depth has decayed by ~63% (one time constant).
    let sc_silent = vec![0.0f32; 513];
    let hops_per_150ms = (0.150 / (1024.0 / 48000.0 / 4.0)) as usize; // ≈ 28 hops at OVERLAP=4
    for _ in 0..hops_per_150ms {
        let mut bins = vec![Complex::new(1.0, 0.0); 513];
        m.process(0, StereoLink::Linked, FxChannelTarget::All,
            &mut bins, Some(&sc_silent), &curves, &mut supp, &ctx);
    }
    let depth_decayed = m.current_carve_depth[0][100];
    let ratio = depth_decayed / depth_engaged;
    assert!(ratio < 0.5,
        "after one τ the depth should be < 0.5× initial; got ratio {}", ratio);
}
```

- [ ] **Step 2: Run**

Run: `cargo test --test punch healing_follows_chosen_time_constant`
Expected: pass.

- [ ] **Step 3: Commit**

```bash
git add tests/punch.rs
git commit -m "test(punch): healing follower decays per chosen time-constant"
```

---

## Task 7 — Per-slot mode persistence + dispatch

**Files:**
- Modify: `src/params.rs`
- Modify: `src/dsp/modules/mod.rs` (add `set_punch_mode` to trait)
- Modify: `src/dsp/fx_matrix.rs`
- Modify: `src/dsp/pipeline.rs`

- [ ] **Step 1: Mirror the `slot_future_mode` pattern**

(If Phase 2b shipped, the pattern is established. Follow it identically for `slot_punch_mode`.)

In `src/params.rs`, add alongside `slot_future_mode`:

```rust
use crate::dsp::modules::punch::PunchMode;
// ...
#[persist = "slot_punch_mode"]
pub slot_punch_mode: [Mutex<PunchMode>; MAX_SLOTS],
```

Initialize in Default impl:
```rust
slot_punch_mode: std::array::from_fn(|_| Mutex::new(PunchMode::default())),
```

- [ ] **Step 2: Add trait + override**

In `src/dsp/modules/mod.rs` `SpectralModule` trait:
```rust
fn set_punch_mode(&mut self, _: crate::dsp::modules::punch::PunchMode) {}
```

In `src/dsp/modules/punch.rs`:
```rust
fn set_punch_mode(&mut self, mode: PunchMode) { self.set_mode(mode); }
```

- [ ] **Step 3: Add `FxMatrix::set_punch_modes`**

```rust
pub fn set_punch_modes(&mut self, modes: &[crate::dsp::modules::punch::PunchMode; 9]) {
    for s in 0..MAX_SLOTS {
        if let Some(ref mut m) = self.slots[s] {
            m.set_punch_mode(modes[s]);
        }
    }
}
```

- [ ] **Step 4: Wire in `pipeline.rs`**

Alongside the future-modes call:
```rust
let punch_modes: [PunchMode; 9] = std::array::from_fn(|i| *params.slot_punch_mode[i].lock());
self.fx_matrix.set_punch_modes(&punch_modes);
```

Add the import:
```rust
use crate::dsp::modules::punch::PunchMode;
```

- [ ] **Step 5: Verify**

Run: `cargo build && cargo test`
Expected: green.

- [ ] **Step 6: Commit**

```bash
git add src/params.rs src/dsp/modules/mod.rs src/dsp/modules/punch.rs src/dsp/fx_matrix.rs src/dsp/pipeline.rs
git commit -m "feat(punch): per-slot PunchMode persistence + dispatch"
```

---

## Task 8 — Add Punch to ASSIGNABLE + per-slot mode picker UI

**Files:**
- Modify: `src/editor/module_popup.rs`
- Modify: `src/editor_ui.rs`

- [ ] **Step 1: ASSIGNABLE**

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
    ModuleType::Punch,
];
```

- [ ] **Step 2: Mode picker**

Mirror the FutureMode picker block:

```rust
if matches!(slot_module_types[s], ModuleType::Punch) {
    let mut current = *params.slot_punch_mode[s].lock();
    let prev = current;
    egui::ComboBox::from_id_source(("punch_mode", s))
        .selected_text(current.label())
        .show_ui(ui, |ui| {
            for mode in [PunchMode::Direct, PunchMode::Inverse] {
                if ui.selectable_label(current == mode, mode.label()).clicked() {
                    current = mode;
                }
            }
        });
    if current != prev {
        *params.slot_punch_mode[s].lock() = current;
    }
}
```

Add the import:
```rust
use crate::dsp::modules::punch::PunchMode;
```

- [ ] **Step 3: Wants-sidechain auto-route**

(Phase 1 plan added the `wants_sidechain` field. The hookup that auto-routes a default sidechain on first assignment lives in `module_popup.rs`. If Phase 1 also wired the auto-route, no change is needed here. Otherwise, add this in `module_popup.rs` `clicked()` handler for the new module:)

```rust
// On first assignment to a wants_sidechain module, auto-route Sc(0) if none routed yet.
if module_spec(ty).wants_sidechain {
    let mut sc = params.slot_sidechain[slot].lock();
    if matches!(*sc, ScChannel::None) {
        *sc = ScChannel::Sc0;
    }
}
```

- [ ] **Step 4: Smoke test in Bitwig**

```bash
cargo run --package xtask -- bundle spectral_forge --release
cp target/bundled/spectral_forge.clap ~/.clap/
```

Open Bitwig, set up Punch with a kick on the sidechain, verify the carve.

- [ ] **Step 5: Commit**

```bash
git add src/editor/module_popup.rs src/editor_ui.rs
git commit -m "feat(punch): assignable module + per-slot mode picker + auto-sidechain"
```

---

## Task 9 — Calibration round-trip coverage

**Files:**
- Modify: `tests/calibration_roundtrip.rs`

- [ ] **Step 1: Add a Punch test**

Following the existing per-module pattern:

```rust
#[test]
fn punch_amount_probe_matches_curve() {
    use num_complex::Complex;
    use spectral_forge::dsp::modules::{SpectralModule, ModuleContext};
    use spectral_forge::dsp::modules::punch::{PunchModule, PunchMode};
    use spectral_forge::params::{FxChannelTarget, StereoLink};

    let mut m = PunchModule::new();
    m.set_mode(PunchMode::Direct);
    m.reset(48000.0, 1024);
    let mut sc = vec![0.0f32; 513];
    sc[100] = 0.9;
    let amount = vec![2.0f32; 513];   // probe_k = 256 → 2.0 → depth 1.0 → 100%
    let other  = vec![1.0f32; 513];
    let mix    = vec![2.0f32; 513];
    let curves: Vec<&[f32]> = vec![&amount, &other, &other, &other, &other, &mix];
    let mut bins = vec![Complex::new(1.0, 0.0); 513];
    let mut supp = vec![0.0f32; 513];
    let ctx = ModuleContext {
        sample_rate: 48000.0, fft_size: 1024, num_bins: 513,
        attack_ms: 10.0, release_ms: 100.0, sensitivity: 0.5,
        suppression_width: 1.0, auto_makeup: false, delta_monitor: false,
    };
    m.process(0, StereoLink::Linked, FxChannelTarget::All,
        &mut bins, Some(&sc), &curves, &mut supp, &ctx);
    let probe = m.last_probe();
    assert!((probe.amount_pct.unwrap_or(0.0) - 100.0).abs() < 1.0);
    assert!((probe.mix_pct.unwrap_or(0.0)    - 100.0).abs() < 1.0);
}
```

- [ ] **Step 2: Run**

Run: `cargo test --test calibration_roundtrip punch_`
Expected: pass.

- [ ] **Step 3: Commit**

```bash
git add tests/calibration_roundtrip.rs
git commit -m "test(punch): calibration round-trip probes"
```

---

## Task 10 — Status banner + STATUS.md

**Files:**
- Modify: `docs/superpowers/STATUS.md`

- [ ] **Step 1: Update STATUS.md**

```markdown
- **Punch module** — IMPLEMENTED 2026-04-27 by `docs/superpowers/plans/2026-04-27-phase-2c-punch.md`. Sub-effects: Direct Punch, Inverse Punch (Self-Punch deferred per audit). Sidechain peak detector + carve-and-fill kernel. Default fill: amplitude with τ=150 ms healing; pitch-fill optional, capped at 0.5 bins.
```

- [ ] **Step 2: Smoke listen**

Test on bass + kick (Direct mode) and lead vocal + sibilance (Inverse mode).

- [ ] **Step 3: Commit**

```bash
git add docs/superpowers/STATUS.md
git commit -m "docs: Punch module IMPLEMENTED status entry"
```

---

## Risk register

1. **Peak detector quality.** The naive local-max-with-threshold approach catches lots of false peaks on noisy sources. Mitigation: the `min_dist` cluster-suppression (Task 3) keeps peaks separated. If users complain, swap in the same peak-detection algorithm Phase 4.2 uses (PLPV peaks).

2. **Inverse mode reuses `current_carve_depth` as scratch.** This is a clear hack — the `for k in 0..n` reset inside the Inverse branch zeros it back. If a future change adds another consumer of `current_carve_depth` between detection and the carve loop, the hack breaks. Mitigation: at the cost of a small extra alloc on `reset()`, a dedicated `inv_scratch` field would be cleaner. Track as v2.

3. **Pitch-fill drift never returns to zero on its own.** Per the audit recommendation #9, drift should slow-drift back to zero on release. The current implementation lets it sit at the engaged drift value. Mitigation (out of scope for v1): when the carve depth follower releases below a threshold, also slew `drift_accum` back to zero. Defer to v2.

4. **Sub-bin pitch drift uses fixed `(π/2)·d` rotation per hop.** This assumes `OVERLAP = 4`. If the OLA overlap ever changes, this constant must change. Hard-code a `const PITCH_ROT_PER_BIN_AT_OL4: f32 = std::f32::consts::FRAC_PI_2;` and a debug_assert to catch overlap changes.

5. **Self-Punch deferred.** Useful for de-essing but lands in v2. The audit explicitly defers it.

6. **`wants_sidechain` auto-route depends on Phase 1.** If Phase 1's auto-route is missing, users assigning Punch will see no effect until they manually wire a sidechain. The fallback in Task 8 Step 3 covers this case directly.

---

## Self-review checklist

- [x] Every task has complete code; no placeholders.
- [x] Tests precede implementation.
- [x] Spec coverage:
  - Direct Punch (Task 4) + Inverse Punch (Task 4)
  - Carve depth via AMOUNT curve (Task 4)
  - Width via WIDTH curve (Task 4)
  - Amp-fill via AMP_FILL curve (Task 4)
  - Healing via HEAL curve, 1-pole follower (Task 4 + Task 6 regression test)
  - Pitch-fill via FILL_MODE curve, 0.5-bin cap (Task 5)
  - Mix via MIX curve (Task 4)
  - Self-Punch deferred per audit ✓
- [x] Names consistent: `PunchModule`, `PunchMode`, `MAX_PEAKS`, `MAX_DRIFT_SITES`, `slot_punch_mode`.
- [x] `wants_sidechain` field used (Phase 1 dependency).
- [x] No allocation in `process()` after `reset()`. The `resize()` calls inside `process()` are conditional on FFT-size mismatch — they guard against missed `reset()` calls and only fire if state size is wrong.

---

## Execution handoff

**Plan complete and saved to `docs/superpowers/plans/2026-04-27-phase-2c-punch.md`.**

Two execution options:

1. **Subagent-Driven (recommended)** — dispatch a fresh subagent per task.
2. **Inline Execution** — execute tasks in this session.

This is one of seven Phase 2 plans. Companions: 2a (Matrix Amp Nodes), 2b (Future), 2d (Rhythm), 2e (Geometry-light), 2f (Modulate-light), 2g (Circuit-light).
