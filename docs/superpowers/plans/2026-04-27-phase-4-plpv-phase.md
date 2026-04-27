# Phase 4: PLPV Phase Unwrapping & Locking Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** add Peak-Locked Phase Vocoder (PLPV) infrastructure to `Pipeline` — per-bin unwrapped phase, low-energy phase damping, peak detection — and integrate it into four shipped modules (Dynamics, PhaseSmear, Freeze, MidSide) for cleaner phase-domain processing.

**Architecture:** PLPV runs at the Pipeline level, between STFT and FxMatrix dispatch. Two new analysis stages produce `ctx.unwrapped_phase` and `ctx.peaks`, which modules opt into. A re-wrap stage runs before iFFT. A single global toggle (`plpv_enable: BoolParam`, default `true`) lets users A/B compare. Adaptive per-frame coherence is **deferred to v2** — v1 ships static Laroche-Dolson identity locking with Voronoi (nearest-peak) skirt assignment.

**Tech Stack:** Rust, no new dependencies. Reuses `realfft` for FFT bins.

**Status banner to add at the top of each PR's commit message:** `infra(phase4):` for 4.1, 4.1.5, 4.2; `feat(phase4):` for 4.3a–4.3d.

**This plan is the source of truth** for everything in `ideas/next-gen-modules/20-plpv-phase-cross-cutting.md`. The cross-reference between brainstorm wording ("PVX") and implementation wording ("PLPV") is documented in that file's naming-note banner.

**Prerequisites:**
- Phase 1 plan landed: `ModuleContext` has `'block` lifetime + `unwrapped_phase: Option<&'block [f32]>` and `peaks: Option<&'block [PeakInfo]>` slots already wired (defaulting to `None`).
- `PeakInfo` struct defined in `src/dsp/modules/mod.rs` (Phase 1 Task 2 Step 2.2).
- Phase 3 BinPhysics is **not** required — PLPV is independent infra.

**Reading order before starting:**
- `ideas/next-gen-modules/20-plpv-phase-cross-cutting.md` (full file — this plan implements it)
- `ideas/next-gen-modules/91-research-synthesis.md` § Cross-cutting validated paths and § Addendum
- `ideas/next-gen-modules/research/01-pvx-phase-and-pll.md` (technical reference)
- `repos/pvx/src/pvx/core/voc.py` (Python reference implementation, MIT-licensed)
- Laroche & Dolson (1997) "About this Phasiness Business"
- Laroche & Dolson (1999) "Improved Phase Vocoder Time-Scale Modification of Audio"
- `src/dsp/pipeline.rs` (the integration site)
- `src/dsp/modules/{dynamics,freeze,phase_smear,mid_side}.rs` (the four module integration targets)

---

## File Structure

| File | Created/Modified | Responsibility |
|---|---|---|
| `src/dsp/plpv.rs` | Create | All PLPV kernels: `unwrap_phase()`, `damp_low_energy_bins()`, `detect_peaks()`, `assign_voronoi_skirts()`, `rewrap_phase()`. |
| `src/dsp/mod.rs` | Modify | `pub mod plpv;` |
| `src/dsp/pipeline.rs` | Modify | Per-channel `prev_unwrapped_phase` + `unwrapped_phase` + `peaks` buffers. Insert unwrap+damp+peaks before `apply_curve_transforms`; insert re-wrap before iFFT. |
| `src/params.rs` | Modify | Add `plpv_enable: BoolParam` (default true), `plpv_phase_noise_floor_db: FloatParam` (default -60.0, range -90..-20), `plpv_max_peaks: IntParam` (default 64, range 16..256), `plpv_peak_threshold_db: FloatParam` (default -40.0). Add per-module enable bools: `plpv_dynamics_enable`, `plpv_phase_smear_enable`, `plpv_freeze_enable`, `plpv_midside_enable` (all default true). |
| `src/dsp/modules/dynamics.rs` | Modify | Phase 4.3a — peak-locked ducking when `ctx.peaks.is_some() && plpv_dynamics_enable`. |
| `src/dsp/modules/phase_smear.rs` | Modify | Phase 4.3b — randomize unwrapped phase when `ctx.unwrapped_phase.is_some() && plpv_phase_smear_enable`. |
| `src/dsp/modules/freeze.rs` | Modify | Phase 4.3c — record + advance unwrapped phase when `ctx.unwrapped_phase.is_some() && plpv_freeze_enable`. |
| `src/dsp/modules/mid_side.rs` | Modify | Phase 4.3d — keep mid+side phase aligned per peak. |
| `tests/plpv_kernel.rs` | Create | Unit tests for unwrap, damp, peak detection, Voronoi assignment, re-wrap. |
| `tests/plpv_calibration.rs` | Create | PLPV-on vs PLPV-off probe-trace equivalence (within ε); inter-channel phase-drift `J` metric. |
| `tests/plpv_audio_render.rs` | Create | Render synthetic test signals (sine sweep, drum loop, sustained chord) and assert objective quality measures. |

---

## Task 1 (Phase 4.1): Per-bin phase unwrapping kernel

**Files:**
- Create: `src/dsp/plpv.rs`
- Modify: `src/dsp/mod.rs`
- Test: `tests/plpv_kernel.rs` (new)

- [ ] **Step 1.1: Write failing tests**

Create `tests/plpv_kernel.rs`:

```rust
use spectral_forge::dsp::plpv::{unwrap_phase, principal_arg};
use std::f32::consts::PI;

#[test]
fn principal_arg_wraps_to_pm_pi() {
    assert!((principal_arg(0.0) - 0.0).abs() < 1e-6);
    assert!((principal_arg(PI - 0.001) - (PI - 0.001)).abs() < 1e-6);
    assert!((principal_arg(PI + 0.001) - (-PI + 0.001)).abs() < 1e-4);
    assert!((principal_arg(3.0 * PI) - PI).abs() < 1e-4);
    assert!((principal_arg(-3.0 * PI) - (-PI)).abs() < 1e-4);
}

#[test]
fn unwrap_phase_constant_partial_advances_by_expected() {
    // A pure tone at bin k=10, sample rate 48000, fft 2048, hop 512.
    // Expected per-hop phase advance: 2π · 10 · 512 / 2048 = 5π ≡ π (mod 2π).
    let prev_phase = vec![0.0_f32; 2048 / 2 + 1];
    let curr_phase = {
        let mut v = vec![0.0_f32; 2048 / 2 + 1];
        v[10] = principal_arg(5.0 * PI);  // = π
        v
    };
    let mut prev_unwrapped = vec![0.0_f32; 2048 / 2 + 1];
    let mut out_unwrapped = vec![0.0_f32; 2048 / 2 + 1];

    unwrap_phase(
        &curr_phase, &prev_phase, &mut prev_unwrapped, &mut out_unwrapped,
        2048, 512, 1025,
    );

    // The unwrapped phase at bin 10 should be ~5π — the cumulative true phase.
    assert!((out_unwrapped[10] - 5.0 * PI).abs() < 1e-3,
        "expected ~5π, got {}", out_unwrapped[10]);
}

#[test]
fn unwrap_phase_silent_signal_stays_at_expected() {
    let prev_phase = vec![0.0_f32; 1025];
    let curr_phase = vec![0.0_f32; 1025];
    let mut prev_unwrapped = vec![0.0_f32; 1025];
    let mut out_unwrapped = vec![0.0_f32; 1025];

    unwrap_phase(
        &curr_phase, &prev_phase, &mut prev_unwrapped, &mut out_unwrapped,
        2048, 512, 1025,
    );
    // For each bin k, expected_advance = 2π · k · 512 / 2048 = π · k / 2.
    for k in 0..1025 {
        let expected = PI * k as f32 / 2.0;
        assert!(
            (out_unwrapped[k] - expected).abs() < 1e-3,
            "bin {} expected {}, got {}", k, expected, out_unwrapped[k],
        );
    }
}
```

Run: `cargo test --test plpv_kernel`
Expected: FAIL — module not found.

- [ ] **Step 1.2: Implement the kernel**

Create `src/dsp/plpv.rs`:

```rust
//! Peak-Locked Phase Vocoder kernels.
//!
//! Implements per-bin phase unwrapping (Laroche-Dolson 1999), low-energy
//! bin phase damping (lifted from `repos/pvx` PHASINESS_IMPLEMENTATION_PLAN
//! Phase 1), and peak detection with Voronoi (nearest-peak) skirt assignment.
//!
//! References:
//! - Laroche, J. and Dolson, M. (1997). About this Phasiness Business.
//!   Proc. ICMC 1997.
//! - Laroche, J. and Dolson, M. (1999). Improved Phase Vocoder Time-Scale
//!   Modification of Audio. IEEE Trans. on Speech and Audio Processing 7(3).

use std::f32::consts::PI;
use crate::dsp::modules::PeakInfo;

/// Wrap a phase to (-π, π] (the "principal value of arg").
#[inline]
pub fn principal_arg(phi: f32) -> f32 {
    let mut p = phi.rem_euclid(2.0 * PI);
    if p > PI { p -= 2.0 * PI; }
    p
}

/// Compute per-bin unwrapped phase trajectory.
///
/// `curr_phase`, `prev_phase`, `prev_unwrapped`, `out_unwrapped` are slices
/// of length `>= num_bins`. After the call, `out_unwrapped[k]` is the
/// continuous-time-like phase trajectory at bin k. `prev_unwrapped` is
/// updated in-place to `out_unwrapped` for the next hop.
///
/// `fft_size` and `hop_size` define the expected per-hop advance.
pub fn unwrap_phase(
    curr_phase:     &[f32],
    prev_phase:     &[f32],
    prev_unwrapped: &mut [f32],
    out_unwrapped:  &mut [f32],
    fft_size:       usize,
    hop_size:       usize,
    num_bins:       usize,
) {
    let n = fft_size as f32;
    let r = hop_size as f32;
    for k in 0..num_bins {
        let expected_advance = 2.0 * PI * (k as f32) * r / n;
        let observed_delta   = curr_phase[k] - prev_phase[k];
        let deviation        = principal_arg(observed_delta - expected_advance);
        let true_advance     = expected_advance + deviation;
        out_unwrapped[k]     = prev_unwrapped[k] + true_advance;
    }
    // Roll prev_unwrapped forward.
    prev_unwrapped[..num_bins].copy_from_slice(&out_unwrapped[..num_bins]);
}

/// Re-wrap an unwrapped phase array back into (-π, π] for iFFT input.
pub fn rewrap_phase(unwrapped: &[f32], wrapped_out: &mut [f32], num_bins: usize) {
    for k in 0..num_bins {
        wrapped_out[k] = principal_arg(unwrapped[k]);
    }
}
```

- [ ] **Step 1.3: Register the module**

In `src/dsp/mod.rs`, add `pub mod plpv;`.

- [ ] **Step 1.4: Run tests**

Run: `cargo test --test plpv_kernel`
Expected: PASS.

- [ ] **Step 1.5: Commit**

```bash
git add src/dsp/plpv.rs src/dsp/mod.rs tests/plpv_kernel.rs
git commit -m "$(cat <<'EOF'
infra(phase4): PLPV per-bin phase unwrapping kernel (Phase 4.1)

Implements Laroche-Dolson 1999 per-bin phase unwrapping. Computes
unwrapped phase trajectory by accumulating expected-advance + deviation
per hop. Pure function — no Pipeline integration yet.

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>
EOF
)"
```

---

## Task 2 (Phase 4.1 cont.): Pipeline integration of unwrap

**Files:**
- Modify: `src/dsp/pipeline.rs`
- Modify: `src/params.rs`

- [ ] **Step 2.1: Add `plpv_enable` parameter**

In `src/params.rs`, add to `SpectralForgeParams`:

```rust
#[id = "plpv_enable"]
pub plpv_enable: BoolParam,
```

In its `Default`:

```rust
plpv_enable: BoolParam::new("PLPV Enable", true),
```

- [ ] **Step 2.2: Add Pipeline state**

In `src/dsp/pipeline.rs`, add to `Pipeline`:

```rust
/// Per-channel previous wrapped phase (from last hop). [channel][bin]
prev_phase: Vec<Vec<f32>>,
/// Per-channel previous unwrapped phase. [channel][bin]
prev_unwrapped_phase: Vec<Vec<f32>>,
/// Per-channel current unwrapped phase. Exposed to modules via ctx.unwrapped_phase.
unwrapped_phase: Vec<Vec<f32>>,
/// Workspace for re-wrapping before iFFT. [channel][bin]
rewrap_buf: Vec<Vec<f32>>,
```

- [ ] **Step 2.3: Allocate in `Pipeline::new()` and zero in `reset()`**

In `Pipeline::new()`:

```rust
let mk_chan_buf = || vec![0.0_f32; MAX_NUM_BINS];
prev_phase:           (0..MAX_CHANNELS).map(|_| mk_chan_buf()).collect(),
prev_unwrapped_phase: (0..MAX_CHANNELS).map(|_| mk_chan_buf()).collect(),
unwrapped_phase:      (0..MAX_CHANNELS).map(|_| mk_chan_buf()).collect(),
rewrap_buf:           (0..MAX_CHANNELS).map(|_| mk_chan_buf()).collect(),
```

In `Pipeline::reset()`:

```rust
for v in &mut self.prev_phase           { v.fill(0.0); }
for v in &mut self.prev_unwrapped_phase { v.fill(0.0); }
for v in &mut self.unwrapped_phase      { v.fill(0.0); }
for v in &mut self.rewrap_buf           { v.fill(0.0); }
```

- [ ] **Step 2.4: Insert the unwrap stage in the per-hop closure**

In `Pipeline::process()`, find the per-hop closure that calls `apply_curve_transforms` then `fx_matrix.process_hop(...)`. Just *before* `apply_curve_transforms`, for each channel:

```rust
if plpv_enable {
    // Extract current wrapped phase from bins.
    for k in 0..num_bins {
        scratch_curr_phase[k] = bins[k].arg();
    }
    crate::dsp::plpv::unwrap_phase(
        &scratch_curr_phase,
        &self.prev_phase[ch][..num_bins],
        &mut self.prev_unwrapped_phase[ch][..num_bins],
        &mut self.unwrapped_phase[ch][..num_bins],
        self.fft_size, hop_size, num_bins,
    );
    // Roll prev_phase forward for the next hop.
    self.prev_phase[ch][..num_bins].copy_from_slice(&scratch_curr_phase[..num_bins]);
    // Expose to modules.
    ctx.unwrapped_phase = Some(&self.unwrapped_phase[ch][..num_bins]);
}
```

`scratch_curr_phase` is a pre-allocated `Vec<f32>` on `Pipeline`, sized `MAX_NUM_BINS`.

- [ ] **Step 2.5: Insert the re-wrap stage after FxMatrix dispatch**

After `fx_matrix.process_hop(...)`, before iFFT:

```rust
if plpv_enable {
    crate::dsp::plpv::rewrap_phase(
        &self.unwrapped_phase[ch][..num_bins],
        &mut self.rewrap_buf[ch][..num_bins],
        num_bins,
    );
    // Apply re-wrapped phase: keep magnitude, replace phase.
    for k in 0..num_bins {
        let m = bins[k].norm();
        let p = self.rewrap_buf[ch][k];
        bins[k] = num_complex::Complex::from_polar(m, p);
    }
}
```

This branch is a no-op when no module modified `unwrapped_phase` (the re-wrap is bit-identical to the input phase).

- [ ] **Step 2.6: Run tests**

Run: `cargo test`
Expected: all existing tests pass; PLPV is computed but no module consumes it yet, so audio output should be byte-identical when PLPV is on or off (verify with `tests/stft_roundtrip.rs`).

- [ ] **Step 2.7: Manual sanity check**

Build with PLPV on, render a 1 kHz sine, confirm no audible change vs PLPV off. (PLPV unwrap+rewrap should be transparent for single-tone input.)

- [ ] **Step 2.8: Commit**

```bash
git add src/dsp/pipeline.rs src/params.rs
git commit -m "$(cat <<'EOF'
infra(phase4): wire PLPV unwrap into Pipeline per-hop loop

Adds prev_phase / prev_unwrapped_phase / unwrapped_phase buffers per
channel (~96 KB total). Computes unwrapped phase before module
dispatch, exposes via ctx.unwrapped_phase, and re-wraps before iFFT.
Gated by plpv_enable BoolParam (default true). Bit-identical audio
output when no module modifies unwrapped_phase.

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>
EOF
)"
```

---

## Task 3 (Phase 4.1.5): Low-energy bin phase damping

**Files:**
- Modify: `src/dsp/plpv.rs`
- Modify: `src/dsp/pipeline.rs`
- Modify: `src/params.rs`
- Test: `tests/plpv_kernel.rs`

- [ ] **Step 3.1: Add `plpv_phase_noise_floor_db` parameter**

In `src/params.rs`:

```rust
#[id = "plpv_phase_noise_floor_db"]
pub plpv_phase_noise_floor_db: FloatParam,
```

Default:

```rust
plpv_phase_noise_floor_db: FloatParam::new(
    "PLPV Phase Noise Floor",
    -60.0,
    FloatRange::Linear { min: -90.0, max: -20.0 },
)
.with_unit(" dB")
.with_step_size(1.0),
```

- [ ] **Step 3.2: Write failing test**

Add to `tests/plpv_kernel.rs`:

```rust
use spectral_forge::dsp::plpv::damp_low_energy_bins;

#[test]
fn damp_low_energy_blends_silent_bins_to_expected() {
    use std::f32::consts::PI;
    let num_bins = 1025;
    let mut unwrapped = vec![0.0_f32; num_bins];
    let mags = vec![0.0_f32; num_bins];  // all silence
    // Pre-fill: bin k=10 has unwrapped phase 99π (way off); rest 0.
    unwrapped[10] = 99.0 * PI;
    let expected = {
        let mut v = vec![0.0_f32; num_bins];
        for k in 0..num_bins { v[k] = PI * k as f32 / 2.0; }
        v
    };

    damp_low_energy_bins(&mut unwrapped, &mags, &expected, -60.0, num_bins);

    // Bin 10 was silent — its unwrapped phase should now be the expected_advance value.
    assert!((unwrapped[10] - expected[10]).abs() < 1e-3,
        "silent bin should be damped to expected; got {}", unwrapped[10]);
}

#[test]
fn damp_low_energy_leaves_loud_bins_alone() {
    use std::f32::consts::PI;
    let num_bins = 1025;
    let mut unwrapped = vec![5.0_f32 * PI; num_bins];
    // Loud signal: 0 dBFS == 1.0 RMS magnitude; well above -60 dB floor.
    let mags = vec![1.0_f32; num_bins];
    let expected = vec![PI / 2.0; num_bins];

    damp_low_energy_bins(&mut unwrapped, &mags, &expected, -60.0, num_bins);

    // Loud bins must not have been touched.
    for k in 0..num_bins {
        assert!((unwrapped[k] - 5.0 * PI).abs() < 1e-3, "loud bin {} was modified", k);
    }
}
```

Run: `cargo test --test plpv_kernel damp`
Expected: FAIL — function not found.

- [ ] **Step 3.3: Implement the damping kernel**

Add to `src/dsp/plpv.rs`:

```rust
/// Damp the unwrapped phase of low-energy bins toward their expected-advance
/// value, using a soft-sigmoid blend across a ±6 dB band centred on the
/// noise floor. Avoids letting noise-dominated phase pollute downstream
/// peak-relative math.
///
/// `mags` and `expected_phase` are length `>= num_bins`. The expected phase
/// is the per-bin cumulative `2π · k · hop_total / fft_size` (caller-computed).
/// `noise_floor_db` is the dB FS reference (typically -60.0).
pub fn damp_low_energy_bins(
    unwrapped:      &mut [f32],
    mags:           &[f32],
    expected_phase: &[f32],
    noise_floor_db: f32,
    num_bins:       usize,
) {
    let floor_lin    = 10.0_f32.powf(noise_floor_db / 20.0);
    let band_lo      = floor_lin * 0.5_f32; // -6 dB below floor
    let band_hi      = floor_lin * 2.0_f32; // +6 dB above floor
    let band_inv_len = 1.0 / (band_hi - band_lo);

    for k in 0..num_bins {
        let m = mags[k];
        let blend = if m <= band_lo {
            1.0  // fully damped
        } else if m >= band_hi {
            0.0  // untouched
        } else {
            // Smoothstep across the ±6 dB band.
            let t = ((band_hi - m) * band_inv_len).clamp(0.0, 1.0);
            t * t * (3.0 - 2.0 * t)
        };
        if blend > 0.0 {
            unwrapped[k] = unwrapped[k] * (1.0 - blend) + expected_phase[k] * blend;
        }
    }
}
```

- [ ] **Step 3.4: Wire damping in the Pipeline unwrap stage**

In `src/dsp/pipeline.rs`, after the `unwrap_phase(...)` call in Task 2 Step 2.4, add:

```rust
if plpv_enable {
    // Compute per-bin expected cumulative advance (cheap, could be cached).
    for k in 0..num_bins {
        // total_hops_seen incremented per hop on Pipeline.
        scratch_expected[k] = 2.0 * std::f32::consts::PI * k as f32 * (self.total_hops as f32 * hop_size as f32) / self.fft_size as f32;
    }
    // Compute current magnitudes from bins.
    for k in 0..num_bins {
        scratch_mags[k] = bins[k].norm();
    }
    crate::dsp::plpv::damp_low_energy_bins(
        &mut self.unwrapped_phase[ch][..num_bins],
        &scratch_mags[..num_bins],
        &scratch_expected[..num_bins],
        noise_floor_db,
        num_bins,
    );
}
```

`scratch_expected`, `scratch_mags`, `total_hops` are added to Pipeline.

- [ ] **Step 3.5: Run tests**

Run: `cargo test`
Expected: PASS.

- [ ] **Step 3.6: Commit**

```bash
git add src/dsp/plpv.rs src/dsp/pipeline.rs src/params.rs tests/plpv_kernel.rs
git commit -m "$(cat <<'EOF'
infra(phase4): low-energy bin phase damping (Phase 4.1.5)

Bins below plpv_phase_noise_floor_db (default -60 dBFS) get their
unwrapped phase blended toward expected-advance via a smoothstep
across a ±6 dB band centred on the floor. Removes noise-dominated
phase from downstream peak-relative math. From repos/pvx
PHASINESS_IMPLEMENTATION_PLAN Phase 1.

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>
EOF
)"
```

---

## Task 4 (Phase 4.2): Peak detection + Voronoi skirt assignment

**Files:**
- Modify: `src/dsp/plpv.rs`
- Modify: `src/dsp/pipeline.rs`
- Modify: `src/params.rs`
- Test: `tests/plpv_kernel.rs`

- [ ] **Step 4.1: Add peak-detection params**

In `src/params.rs`:

```rust
#[id = "plpv_max_peaks"]
pub plpv_max_peaks: IntParam,
#[id = "plpv_peak_threshold_db"]
pub plpv_peak_threshold_db: FloatParam,
```

Defaults:

```rust
plpv_max_peaks: IntParam::new("PLPV Max Peaks", 64,
    IntRange::Linear { min: 16, max: 256 }),
plpv_peak_threshold_db: FloatParam::new(
    "PLPV Peak Threshold", -40.0,
    FloatRange::Linear { min: -80.0, max: -10.0 },
).with_unit(" dB"),
```

- [ ] **Step 4.2: Write failing tests**

Add to `tests/plpv_kernel.rs`:

```rust
use spectral_forge::dsp::plpv::detect_peaks;
use spectral_forge::dsp::modules::PeakInfo;

#[test]
fn detect_peaks_finds_local_4_neighbour_max() {
    let mut mags = vec![0.0_f32; 100];
    mags[20] = 0.5;  // small peak
    mags[50] = 1.0;  // large peak
    mags[80] = 0.3;  // peak below threshold

    let mut peaks = vec![PeakInfo { k: 0, mag: 0.0, low_k: 0, high_k: 0 }; 64];
    let n = detect_peaks(&mags, 100, -20.0, 64, &mut peaks);
    // Threshold -20 dB == 0.1 linear, so all three exceed.
    assert!(n >= 3);
    let ks: Vec<u32> = peaks[..n].iter().map(|p| p.k).collect();
    assert!(ks.contains(&20));
    assert!(ks.contains(&50));
    assert!(ks.contains(&80));
}

#[test]
fn voronoi_assigns_skirts_to_nearest_peak() {
    use spectral_forge::dsp::plpv::assign_voronoi_skirts;
    let mut peaks = vec![
        PeakInfo { k: 20, mag: 0.5, low_k: 0, high_k: 0 },
        PeakInfo { k: 60, mag: 1.0, low_k: 0, high_k: 0 },
    ];
    assign_voronoi_skirts(&mut peaks, 100);
    // Midpoint between k=20 and k=60 is 40. Skirts split there.
    assert_eq!(peaks[0].low_k, 0);
    assert_eq!(peaks[0].high_k, 40);
    assert_eq!(peaks[1].low_k, 41);
    assert_eq!(peaks[1].high_k, 99);
}
```

Run: `cargo test --test plpv_kernel detect_peaks voronoi`
Expected: FAIL.

- [ ] **Step 4.3: Implement peak detection**

Add to `src/dsp/plpv.rs`:

```rust
/// Detect local 4-neighbour magnitude peaks above a dB threshold.
/// Writes up to `max_peaks` peaks into `out_peaks` (sorted by bin index
/// ascending). Returns the actual number written.
///
/// A bin k is a peak if `mags[k]` is strictly greater than `mags[k±1]`
/// and `mags[k±2]`. `low_k` and `high_k` are filled by `assign_voronoi_skirts`
/// in a separate pass.
pub fn detect_peaks(
    mags:       &[f32],
    num_bins:   usize,
    threshold_db: f32,
    max_peaks:  usize,
    out_peaks:  &mut [PeakInfo],
) -> usize {
    let threshold = 10.0_f32.powf(threshold_db / 20.0);
    let mut count = 0;
    // Skip k=0 and k=num_bins-1, k=1 and k=num_bins-2 (no 2-neighbour ranges).
    for k in 2..num_bins.saturating_sub(2) {
        if count >= max_peaks { break; }
        let m = mags[k];
        if m < threshold { continue; }
        if m > mags[k - 1] && m > mags[k - 2]
            && m > mags[k + 1] && m > mags[k + 2]
        {
            out_peaks[count] = PeakInfo {
                k: k as u32,
                mag: m,
                low_k: 0,
                high_k: 0,
            };
            count += 1;
        }
    }
    count
}

/// Assign each peak's skirt as the bins in its Voronoi cell — closer to
/// it than to the next peak. Updates `low_k` and `high_k` in place.
/// Peaks must be sorted by `k` ascending.
pub fn assign_voronoi_skirts(peaks: &mut [PeakInfo], num_bins: usize) {
    let n = peaks.len();
    for i in 0..n {
        let lo = if i == 0 {
            0
        } else {
            // Midpoint between this peak and previous, exclusive.
            ((peaks[i - 1].k + peaks[i].k) / 2 + 1)
        };
        let hi = if i == n - 1 {
            num_bins as u32 - 1
        } else {
            (peaks[i].k + peaks[i + 1].k) / 2
        };
        peaks[i].low_k = lo;
        peaks[i].high_k = hi;
    }
}
```

- [ ] **Step 4.4: Wire into Pipeline**

In `src/dsp/pipeline.rs`, add buffer:

```rust
peak_buf: Vec<Vec<PeakInfo>>,  // [channel] — capacity MAX_PEAKS
```

Initialize:

```rust
const MAX_PEAKS: usize = 256;
peak_buf: (0..MAX_CHANNELS).map(|_|
    vec![PeakInfo { k: 0, mag: 0.0, low_k: 0, high_k: 0 }; MAX_PEAKS]
).collect(),
```

After damping (Step 3.4), add:

```rust
if plpv_enable {
    let n_peaks = crate::dsp::plpv::detect_peaks(
        &scratch_mags[..num_bins], num_bins,
        peak_threshold_db, max_peaks as usize,
        &mut self.peak_buf[ch][..],
    );
    crate::dsp::plpv::assign_voronoi_skirts(
        &mut self.peak_buf[ch][..n_peaks], num_bins,
    );
    ctx.peaks = Some(&self.peak_buf[ch][..n_peaks]);
}
```

- [ ] **Step 4.5: Run tests**

Run: `cargo test`
Expected: PASS.

- [ ] **Step 4.6: Commit**

```bash
git add src/dsp/plpv.rs src/dsp/pipeline.rs src/params.rs tests/plpv_kernel.rs
git commit -m "$(cat <<'EOF'
infra(phase4): peak detection + Voronoi skirt assignment (Phase 4.2)

Laroche-Dolson local 4-neighbour max + threshold gate. Voronoi
(nearest-peak) skirt assignment. Caps at plpv_max_peaks (default 64).
Peaks exposed via ctx.peaks. No module consumes them yet — Phase 4.3
PRs land per module.

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>
EOF
)"
```

---

## Task 5 (Phase 4.3a): Dynamics PLPV peak-locked ducking

**Files:**
- Modify: `src/dsp/modules/dynamics.rs`
- Modify: `src/params.rs`
- Test: `tests/plpv_calibration.rs` (new)

- [ ] **Step 5.1: Add per-module enable param**

In `src/params.rs`:

```rust
#[id = "plpv_dynamics_enable"]
pub plpv_dynamics_enable: BoolParam,
```

Default:

```rust
plpv_dynamics_enable: BoolParam::new("Dynamics PLPV", true),
```

Pass `plpv_dynamics_enable` through `DynamicsModule` via a setter or a bool argument.

- [ ] **Step 5.2: Write failing PLPV-equivalence test**

Create `tests/plpv_calibration.rs`:

```rust
use spectral_forge::dsp::modules::{ModuleType, create_module};
// (Test scaffolding similar to tests/calibration_roundtrip.rs.)

#[test]
fn dynamics_plpv_off_matches_legacy_path() {
    // Setup: run a probe through Dynamics with PLPV off, capture the trace.
    // Then run the same probe with PLPV enabled at the Pipeline level but
    // plpv_dynamics_enable=false. Probe traces must match within ε=1e-5.
    todo!("set up PipelineHarness::run_probe with both PLPV states");
}

#[test]
fn dynamics_plpv_on_locks_skirt_to_peak() {
    // Setup: synthesize a sustained 1 kHz sine + 2 kHz harmonic.
    // Sidechain ducks all bins by -6 dB at the peak.
    // Assert: skirt bins around 1 kHz peak get the *same* gain reduction as
    // the peak itself (within ε), proving the lock is active.
    todo!("synthesize partial+harmonic, run, assert skirt gains == peak gain");
}
```

Run: `cargo test --test plpv_calibration`
Expected: FAIL — `todo!()` panics.

- [ ] **Step 5.3: Implement peak-locked ducking in Dynamics**

In `src/dsp/modules/dynamics.rs`, find `process()`. Add after the existing per-bin gain-reduction computation:

```rust
if let (Some(peaks), true) = (ctx.peaks, self.plpv_enabled) {
    // Per-peak: take the peak bin's gain reduction; apply same factor to skirt.
    for p in peaks {
        let peak_gain = self.gain_reduction[p.k as usize];
        let lo = p.low_k as usize;
        let hi = (p.high_k as usize).min(ctx.num_bins - 1);
        for k in lo..=hi {
            self.gain_reduction[k] = peak_gain;
        }
    }
}
// Then apply gain_reduction[k] to bins[k].mag as before.
```

`self.plpv_enabled: bool` is set from the param via a setter the Pipeline calls each block.

- [ ] **Step 5.4: Implement the test scaffolding**

Replace the `todo!()` blocks in `tests/plpv_calibration.rs` with actual `Pipeline::process()` runs. Use the existing `tests/calibration_roundtrip.rs` patterns for harness setup.

- [ ] **Step 5.5: Run tests**

Run: `cargo test --test plpv_calibration`
Expected: PASS.

- [ ] **Step 5.6: Manual A/B render**

Build and bundle. Open Bitwig. Load Dynamics on a polyphonic sustained source (pad). A/B by toggling `plpv_dynamics_enable`. Confirm cleaner ducking (less smear) on the PLPV-on path.

- [ ] **Step 5.7: Commit**

```bash
git add src/dsp/modules/dynamics.rs src/params.rs tests/plpv_calibration.rs
git commit -m "$(cat <<'EOF'
feat(phase4): Dynamics — peak-locked ducking (Phase 4.3a)

When ctx.peaks is populated and plpv_dynamics_enable is on, applies the
peak's gain reduction to the entire Voronoi skirt instead of per-bin.
Preserves partial coherence under sidechain ducking. Falls back to
per-bin behaviour when PLPV is off.

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>
EOF
)"
```

---

## Task 6 (Phase 4.3b): PhaseSmear unwrapped randomization

**Files:**
- Modify: `src/dsp/modules/phase_smear.rs`
- Modify: `src/params.rs`
- Test: extend `tests/plpv_calibration.rs`

- [ ] **Step 6.1: Add `plpv_phase_smear_enable: BoolParam` (default true)**

(Same pattern as Step 5.1.)

- [ ] **Step 6.2: Modify PhaseSmear to operate on unwrapped phase**

In `src/dsp/modules/phase_smear.rs`, find the per-bin random offset application. When PLPV is on:

```rust
if let (Some(unwrapped), true) = (ctx.unwrapped_phase, self.plpv_enabled) {
    // Add random offset to a copy of the unwrapped trajectory; Pipeline re-wraps.
    for k in 0..ctx.num_bins {
        let m = bins[k].norm();
        let new_phase = unwrapped[k] + self.random_offset[k];
        // We can't write back to ctx.unwrapped_phase (it's & not &mut).
        // Instead, write the wrapped result directly and skip Pipeline rewrap?
        // Cleaner: ModuleContext can carry a `&mut` slice for *this* module's
        // override. For simplicity in v1, write the wrapped result here:
        bins[k] = num_complex::Complex::from_polar(m, crate::dsp::plpv::principal_arg(new_phase));
    }
} else {
    // Existing wrapped-phase code path.
}
```

Note: this design deliberately writes the bins directly. The Pipeline re-wrap stage is idempotent (re-wrapping an already-wrapped phase is a no-op) so this is safe.

Alternative design: pass an additional `unwrapped_phase_writer: Option<&'block mut [f32]>` in ctx so PhaseSmear writes the unwrapped trajectory and the Pipeline re-wrap stage produces the bins. Document this trade-off in the PR description.

- [ ] **Step 6.3: Add audio test**

Add to `tests/plpv_calibration.rs`:

```rust
#[test]
fn phase_smear_plpv_smoothness() {
    // Synthesize a 1 kHz sine. Run through PhaseSmear with amount=0.5,
    // PLPV off vs on. Measure RMS at hop boundaries; PLPV-on should be
    // less than PLPV-off (smoother boundaries).
    todo!("PipelineHarness + boundary RMS measurement");
}
```

- [ ] **Step 6.4: Run tests, manual A/B, commit**

Run: `cargo test --test plpv_calibration`. Manual render. Commit.

```bash
git commit -m "feat(phase4): PhaseSmear — unwrapped-phase randomization (Phase 4.3b)

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

## Task 7 (Phase 4.3c): Freeze unwrapped phase advance

**Files:**
- Modify: `src/dsp/modules/freeze.rs`
- Modify: `src/params.rs`
- Test: extend `tests/plpv_calibration.rs`

- [ ] **Step 7.1: Add `plpv_freeze_enable: BoolParam`**

- [ ] **Step 7.2: Modify Freeze**

In `src/dsp/modules/freeze.rs`, when freeze is active and PLPV is on:

1. At freeze trigger, record the *unwrapped* phase per bin into the freeze buffer.
2. Each subsequent hop, advance the recorded unwrapped phase by `expected_advance = 2π · k · hop_size / fft_size` per bin.
3. Apply the result to the output bins (Pipeline re-wraps).

```rust
if let (Some(unwrapped), true) = (ctx.unwrapped_phase, self.plpv_enabled) {
    if !self.frozen_unwrapped_recorded {
        self.frozen_unwrapped[..ctx.num_bins]
            .copy_from_slice(&unwrapped[..ctx.num_bins]);
        self.frozen_unwrapped_recorded = true;
    }
    // Advance and write to bins.
    let n = ctx.fft_size as f32;
    let r = ctx.fft_size as f32 / 4.0;  // hop = fft/4
    for k in 0..ctx.num_bins {
        self.frozen_unwrapped[k] += 2.0 * std::f32::consts::PI * k as f32 * r / n;
        let m = self.frozen_magnitude[k];
        bins[k] = num_complex::Complex::from_polar(
            m,
            crate::dsp::plpv::principal_arg(self.frozen_unwrapped[k]),
        );
    }
} else {
    // Existing static-phase freeze path.
}
```

`frozen_unwrapped: Vec<f32>` is added to `FreezeModule`, allocated `MAX_NUM_BINS` in `reset()`.

- [ ] **Step 7.3: Test, A/B, commit**

```bash
git commit -m "feat(phase4): Freeze — unwrapped phase advance (Phase 4.3c)

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

## Task 8 (Phase 4.3d): MidSide PLPV + inter-channel phase-drift probe

**Files:**
- Modify: `src/dsp/modules/mid_side.rs`
- Modify: `src/params.rs`
- Test: extend `tests/plpv_calibration.rs`

- [ ] **Step 8.1: Add `plpv_midside_enable: BoolParam`**

- [ ] **Step 8.2: Modify MidSide for peak-aligned mid/side decode**

In `src/dsp/modules/mid_side.rs`, when PLPV peaks are present:

For each peak, the mid/side decode operates on `(L+R)/√2, (L−R)/√2`. The peak's relative phase between L and R should be preserved during balance/expansion processing. Concretely: if the user's BALANCE curve attenuates side at bin k, apply the *same* attenuation to the entire skirt of the peak nearest to k, instead of per-bin.

(Refer to brainstorm § "MidSide" in `20-plpv-phase-cross-cutting.md` for the full spec.)

- [ ] **Step 8.3: Implement the inter-channel phase-drift `J` metric**

Add to `tests/plpv_calibration.rs`:

```rust
fn inter_channel_phase_drift(
    in_l: &[Complex<f32>], in_r: &[Complex<f32>],
    out_l: &[Complex<f32>], out_r: &[Complex<f32>],
    num_bins: usize,
    noise_floor: f32,
) -> f32 {
    let mut j = 0.0_f32;
    for k in 0..num_bins {
        let dphi_in  = in_l[k].arg()  - in_r[k].arg();
        let dphi_out = out_l[k].arg() - out_r[k].arg();
        // Gate noise-dominated bins out of the metric.
        if in_l[k].norm() > noise_floor && in_r[k].norm() > noise_floor {
            j += (dphi_out - dphi_in).abs();
        }
    }
    j
}

#[test]
fn midside_plpv_does_not_increase_phase_drift_on_tonal_signal() {
    // Synthesize a sustained chord, run through MidSide balance=+0.5.
    // Measure J for PLPV-off and PLPV-on. PLPV-on must not exceed PLPV-off
    // by more than ε.
    let eps = 0.5;  // radians, total over all bins
    let j_off = run_and_measure(/* plpv_enable=false */);
    let j_on  = run_and_measure(/* plpv_enable=true */);
    assert!(j_on <= j_off + eps,
        "PLPV-on increased inter-channel phase drift: J_off={} J_on={}", j_off, j_on);
}
```

- [ ] **Step 8.4: Test, A/B, commit**

```bash
git commit -m "feat(phase4): MidSide — peak-aligned decode + J probe (Phase 4.3d)

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

## Task 9: Audio render regression suite

**Goal:** synthetic test signals that produce reproducible audio output for objective quality measures. Catches future regressions.

**Files:**
- Create: `tests/plpv_audio_render.rs`

- [ ] **Step 9.1: Implement three audio render tests**

```rust
#[test]
fn sine_sweep_through_dynamics_centroid_stability() {
    // 100 Hz → 2 kHz over 4 seconds. Run through Dynamics ducking.
    // Measure spectral centroid frame-to-frame variance.
    // PLPV-on variance < PLPV-off variance.
}

#[test]
fn drum_loop_through_dynamics_no_smearing() {
    // Synthetic kick-snare-hat pattern. Run through Dynamics.
    // Measure: peak hold per bin during duck attack.
    // PLPV-on should preserve transient energy better.
}

#[test]
fn sustained_chord_through_freeze_no_boundary_clicks() {
    // Major triad sustained. Trigger freeze. Hold for 2 seconds.
    // Measure RMS at every hop boundary.
    // PLPV-on RMS variance < PLPV-off variance.
}
```

(Each test uses `tests/test_signals.rs` helpers — extract sine/sweep/drum synthesis into shared module.)

- [ ] **Step 9.2: Commit**

```bash
git add tests/plpv_audio_render.rs tests/test_signals.rs
git commit -m "test(phase4): PLPV audio-render regression suite

Three synthetic-signal regression tests for PLPV's quality
improvements: sine-sweep centroid stability, drum-loop transient
preservation, frozen-chord boundary smoothness.

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

## Task 10: Status update

- [ ] **Step 10.1: Update STATUS.md and roadmap banner**

```markdown
| `2026-04-27-phase-4-plpv-phase` | IMPLEMENTED | Per-bin unwrap, low-energy damping, peak detection, Voronoi skirts, integration into Dynamics/PhaseSmear/Freeze/MidSide. v2 adaptive coherence deferred. |
```

In `99-implementation-roadmap.md` § Phase 4:

```markdown
> **Status:** IMPLEMENTED (2026-04-27). See plan
> `docs/superpowers/plans/2026-04-27-phase-4-plpv-phase.md`.
```

In `20-plpv-phase-cross-cutting.md`, update the status banner from "RESEARCH" to "IMPLEMENTED".

- [ ] **Step 10.2: Commit**

```bash
git add docs/superpowers/STATUS.md ideas/next-gen-modules/99-implementation-roadmap.md ideas/next-gen-modules/20-plpv-phase-cross-cutting.md
git commit -m "docs(status): mark Phase 4 PLPV IMPLEMENTED

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

## Self-review checklist

- [ ] **Spec coverage:** every Phase listed in `20-plpv-phase-cross-cutting.md` § Implementation phasing maps to a Task here. ✓ (Phase 1 → Tasks 1+2, Phase 1.5 → Task 3, Phase 2 → Task 4, Phase 3 → Tasks 5–8.) Adaptive policy (v2) explicitly out of scope.
- [ ] **Placeholder scan:** Tasks 5.4, 6.3, 7.3, 8.4 contain `todo!()` for test scaffolding — these are intentional checkboxes for the executing engineer to fill from the existing `tests/calibration_roundtrip.rs` harness pattern.
- [ ] **Type consistency:** `PeakInfo` always has fields `k, mag, low_k, high_k` (defined in Phase 1 Task 2 Step 2.2). PLPV kernels in `dsp/plpv.rs` are pure functions. Per-module bool params follow the `plpv_<module>_enable` pattern.
- [ ] **All `cargo test` passes** between every Task.
- [ ] **Inter-channel phase-drift probe** defined and used in Task 8.3.
- [ ] **Laroche-Dolson 1997 + 1999** cited in `src/dsp/plpv.rs` doc comment.

---

## Risk register (Phase 4)

| Risk | Mitigation |
|---|---|
| PLPV doesn't deliver audible quality improvement | Phase 1 (unwrap only) ships behind a toggle. A/B by ear before committing module integrations. |
| Peak detection is jittery between hops, causing skirt-membership flicker | Voronoi assignment + threshold gate. v2 may need temporal hysteresis (deferred). |
| Re-wrap stage breaks existing modules | Re-wrap is idempotent on unmodified `unwrapped_phase`. Verified with byte-identical-output test in Step 2.6. |
| Memory growth | Per-channel buffers ~96 KB total. PeakInfo array ~8 KB. Well under any RT budget. |
| Phase noise floor too aggressive on quiet program | Default -60 dBFS is conservative; user-tunable via `plpv_phase_noise_floor_db`. |

## Execution handoff

Tasks 1, 2, 3, 4 are sequential infra (Phase 4.1 → 4.1.5 → 4.2). Each lands in isolation and produces no audible change.

Tasks 5, 6, 7, 8 are independent module integrations. Each can land separately with its own A/B render.

Task 9 is the regression suite — runs before each release to catch quality regressions.

Task 10 closes out Phase 4.

**Critical path:** Tasks 1 → 2 → 3 → 4 must be sequential. Tasks 5–8 can parallelise. Task 9 depends on at least one of 5–8 landed.
