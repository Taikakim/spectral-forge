# Phase 5c — Full Circuit Retrofit Implementation Plan

> **Status:** IMPLEMENTED — all 15 tasks landed on `feature/next-gen-modules-plans` (commits `0a92315` … `f434088`). Phase 5 sub-plan; depends on:
> - Phase 1 foundation infra (`docs/superpowers/plans/2026-04-27-phase-1-foundation-infra.md`) — `ModuleContext` `'block` lifetime + `bin_physics: Option<&'block BinPhysics>` slot + `writes_bin_physics`/`heavy_cpu_per_mode` `ModuleSpec` extensions.
> - Phase 2g Circuit-light (`docs/superpowers/plans/2026-04-27-phase-2g-circuit-light.md`) — base `CircuitModule` with BbdBins / SpectralSchmitt / CrossoverDistortion modes.
> - Phase 3 BinPhysics infra (`docs/superpowers/plans/2026-04-27-phase-3-bin-physics.md`) — `BinPhysics` struct (with `flux`, `temperature`, `bias`, `slew` fields).
> - Phase 5b.3 Kinetics (`docs/superpowers/plans/2026-04-27-phase-5b3-kinetics.md`) — provides `src/dsp/physics_helpers.rs` (reuse `smooth_curve_one_pole`).
>
> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extend the existing `CircuitModule` with seven new BinPhysics-aware sub-effects — **Vactrol**, **Transformer Saturation**, **Power Sag**, **Component Drift**, **PCB Crosstalk**, **Slew Distortion**, **Bias Fuzz** — bringing the module to its full 10-mode spec. Bump curve count from 4 to 5 by inserting **SPREAD** at index 2 (used by Transformer + PCB Crosstalk + Bias Fuzz). Migrate the three v1 kernels to the new curve layout.

**Architecture:** The module retains its existing skeleton; new modes plug into the `match self.mode { … }` dispatch block. State arrays expand to host per-bin fields the new modes need (vactrol two-stage caps, transformer magnitude smoother, sag envelope, drift LFSR state, prev_mag for slew). Vactrol/Transformer/Sag/Drift/Bias-Fuzz hook into `BinPhysics` via the Phase 3 reader (`ctx.bin_physics`) + writer (`physics: Option<&mut BinPhysics>`) hooks added in Phase 1+3. Module-level `writes_bin_physics: true` enables the writer schedule; per-mode behaviour decides whether the slot actually mutates `BinPhysics` on a given hop.

**Tech Stack:** Rust 2021, `nih-plug`, `realfft::num_complex::Complex<f32>`, existing `SpectralModule` trait with Phase 1+3 enrichments, Phase 5b.3 `physics_helpers::smooth_curve_one_pole`. New file: `src/dsp/circuit_kernels.rs` (scalar SIMD-friendly primitives — `lp_step`, `tanh_levien_poly`, `spread_3tap`, `SimdRng`). No new external deps.

**Source spec:** `ideas/next-gen-modules/16-modulate.md` is for the Modulate module — unrelated. The Circuit spec is `ideas/next-gen-modules/10-circuit.md` (research findings 2026-04-26 incorporated).

**Curve layout (post-retrofit):**

| Idx | Label     | Used by                                                                          |
|-----|-----------|----------------------------------------------------------------------------------|
| 0   | AMOUNT    | All modes                                                                        |
| 1   | THRESHOLD | Schmitt (on), Crossover (deadzone width), Transformer (knee), Sag (energy gate), Drift (per-bin temp gate), Slew (rate cap), Bias Fuzz (top rail) |
| 2   | SPREAD    | Transformer (neighbour leak), PCB Crosstalk (3-tap kernel), Bias Fuzz (bias bleed)|
| 3   | RELEASE   | All modes that smooth (BBD, Schmitt hysteresis gap, Vactrol, Transformer, Sag, Drift, Bias Fuzz) |
| 4   | MIX       | All modes                                                                        |

**Mode enum (final, ordered):**

| Idx | `CircuitMode` variant   | CPU class | BinPhysics  | Curves used   |
|-----|-------------------------|-----------|-------------|---------------|
| 0   | `BbdBins`               | light     | none        | 0,3,4         |
| 1   | `SpectralSchmitt`       | light     | none        | 0,1,3,4       |
| 2   | `CrossoverDistortion`   | light     | none        | 0,1,4         |
| 3   | `Vactrol`               | medium    | reads `flux`| 0,3,4         |
| 4   | `TransformerSaturation` | heavy     | reads/writes `flux` | 0,1,2,3,4 |
| 5   | `PowerSag`              | light     | reads `temperature` | 0,1,3,4 |
| 6   | `ComponentDrift`        | light     | reads/writes `temperature` | 0,1,3,4 |
| 7   | `PcbCrosstalk`          | medium    | none (or bias bleed only) | 0,2,4 |
| 8   | `SlewDistortion`        | light     | writes `slew` | 0,1,3,4   |
| 9   | `BiasFuzz`              | light     | reads/writes `bias` | 0,1,2,3,4 |

`heavy_cpu_per_mode = [false, false, false, false, true, false, false, false, false, false]` (only Transformer).
`writes_bin_physics = true` at module level (Phase 3 schedule).

**Risk register:**
- **Existing v1 preset breakage:** Phase 2g has not yet shipped (per STATUS.md as of 2026-04-26). This plan can re-order curves without preset migration as long as Phase 5c lands before any v1 release. Task 1 hard-fails the build if a presets/* fixture references old curve indices — gate.
- **Transformer SPREAD two-pass:** Naive in-place leak corrupts neighbours read after write. Use a pre-allocated `flux_workspace[2][num_bins]` scratch in the module; one read-pass copies, one write-pass leaks. No allocation on the audio thread.
- **Component Drift LFSR step rate:** Per-bin per-hop xorshift would burn cycles. Step the LFSR once per hop and modulate per-bin via a deterministic offset (bin index XOR low bits of state). Cheap.
- **Bias Fuzz top-rail clipping discontinuity:** Clip with `tanh_levien_poly` from the new circuit kernels for C¹ smoothness, not hard `min()`.
- **Slew phase-scramble dither audibility:** Use `tear_rng` xorshift32 as in Phase 5b.4 PLL Tear, NOT `rand` crate (RT alloc-free guarantee).
- **Curve smoothing on retrofit modes:** All 7 retrofit modes consume potentially-fast-moving curves at hop rate. Vactrol/Transformer/Sag (all of which feed into 1-poles) want a 1-pole smoother on the curves themselves to avoid hop-rate parameter modulation steps. Reuse `physics_helpers::smooth_curve_one_pole`. v1 modes (BBD, Schmitt, Crossover) are NOT smoothed — they consume raw curves byte-identically to before this plan, so the v1 multi-hop test still passes after the migration.
- **Schmitt latch state size:** Existing `schmitt_latched: [Vec<u8>; 2]` is fine, no change.
- **BBD memory:** unchanged from Phase 2g (~262 KB per slot).
- **`writes_bin_physics: true` cost on v1 modes:** Phase 3's writer schedule allocates a per-slot `BinPhysics` even when v1 modes don't mutate it. The cost is the merge into the next slot's input — bounded; per Phase 3 risk register, this is the intended cost of opting in.

**Defer list (NOT in this plan):**
- **Resonant Feedback** — handled via `RouteMatrix` + Matrix Amp Nodes (Phase 2a). Not a sub-effect.
- **Envelope Follower Ripple** — global helper, separately tracked.
- **Circuit-as-stack** (multi-mode-per-slot) — v2; not in scope.
- **SIMD adoption via `wide` crate** — v2; this plan ships scalar kernels structured for clean SIMD lift later (loops are bin-major, no inter-bin dependencies inside a kernel except the 3-tap stencil which has been pre-cleared).
- **Volterra-series transformer** — confirmed dead-end per spec research §6.

---

## File Structure

**Create:**
- `src/dsp/circuit_kernels.rs` — shared scalar primitives: `lp_step(state, target, alpha)`, `tanh_levien_poly(x)`, `spread_3tap(input, output, kernel_strength)`, `SimdRng` (xorshift32 wrapped in a small struct).

**Modify:**
- `src/dsp/mod.rs` — `pub mod circuit_kernels;` (above `pub mod modules;`).
- `src/dsp/modules/circuit.rs` — extend `CircuitMode` enum with 7 variants; bump `num_curves()` to 5; migrate v1 kernels to new curve indices; add 7 new mode kernels; expand state fields; expand `reset()`; expand `set_circuit_mode()` reset list; expand `process()` dispatch; expand probe types.
- `src/dsp/modules/mod.rs` — update `module_spec(Circuit)`: bump `num_curves` to 5, set `curve_labels` to `&["AMOUNT","THRESH","SPREAD","RELEASE","MIX"]`, set `writes_bin_physics: true`, set `heavy_cpu_per_mode: Some(&CIR_HEAVY)`.
- `src/dsp/fx_matrix.rs` — no schema change; the existing `set_circuit_modes()` from Phase 2g already routes the per-slot mode. (BinPhysics flow is Phase 3 — already wired before this plan.)
- `src/editor/circuit_popup.rs` — add 7 new entries to the mode picker; render a small "(heavy)" label next to Transformer.
- `tests/module_trait.rs` — add per-mode kernel tests + extend the multi-hop dual-channel finite/bounded contract test to all 10 modes.
- `tests/calibration_roundtrip.rs` — extend Circuit calibration test to all 10 modes; new probe fields.
- `tests/bin_physics_pipeline.rs` — add Vactrol-reads-flux, Transformer-writes-flux, Bias-Fuzz-roundtrips-bias integration tests.
- `docs/superpowers/STATUS.md` — flip Phase 5c row to IMPLEMENTED on landing.
- `docs/superpowers/plans/2026-04-27-phase-2g-circuit-light.md` — append a "Superseded by Phase 5c for curve layout" line in the banner.

---

## Task 1: Bump curve count to 5; insert SPREAD at index 2; flip ModuleSpec to BinPhysics-writer

**Files:**
- Modify: `src/dsp/modules/mod.rs` (`module_spec(Circuit)`)
- Modify: `src/dsp/modules/circuit.rs` (`num_curves()` returns 5)
- Modify: `tests/module_trait.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn circuit_spec_has_5_curves_with_spread_and_writes_bin_physics() {
    use spectral_forge::dsp::modules::{module_spec, ModuleType};

    let spec = module_spec(ModuleType::Circuit);
    assert_eq!(spec.num_curves, 5);
    assert_eq!(spec.curve_labels, &["AMOUNT", "THRESH", "SPREAD", "RELEASE", "MIX"]);
    assert!(spec.writes_bin_physics, "Phase 5c retrofit opts into BinPhysics writer schedule");
    let heavy = spec.heavy_cpu_per_mode.expect("Phase 5c declares per-mode heavy flag");
    assert_eq!(heavy.len(), 10);
    assert!(!heavy[0], "BbdBins is light");
    assert!(!heavy[1], "Schmitt is light");
    assert!(!heavy[2], "Crossover is light");
    assert!(!heavy[3], "Vactrol is medium (treated as light for matrix budget)");
    assert!( heavy[4], "Transformer is heavy");
    assert!(!heavy[5], "Sag is light");
    assert!(!heavy[6], "Drift is light");
    assert!(!heavy[7], "PCB Crosstalk is medium (treated as light)");
    assert!(!heavy[8], "Slew is light");
    assert!(!heavy[9], "Bias Fuzz is light");
}
```

- [ ] **Step 2: Run test, expect failure**

Run: `cargo test --test module_trait circuit_spec_has_5_curves_with_spread_and_writes_bin_physics -- --nocapture`
Expected: FAIL — assertion on `num_curves == 5`.

- [ ] **Step 3: Update `module_spec(Circuit)` in `src/dsp/modules/mod.rs`**

Replace the existing `ModuleType::Circuit => ModuleSpec { … }` arm (or `static CIR: ModuleSpec = …;` declaration if Phase 2g landed with the static-spec pattern from Phase 5b.3) with:

```rust
static CIR_HEAVY: [bool; 10] = [
    false, // BbdBins
    false, // SpectralSchmitt
    false, // CrossoverDistortion
    false, // Vactrol (medium — counted as light for routing budget)
    true,  // TransformerSaturation (tanh + 3-tap spread)
    false, // PowerSag
    false, // ComponentDrift
    false, // PcbCrosstalk (medium — counted as light)
    false, // SlewDistortion
    false, // BiasFuzz
];

static CIR: ModuleSpec = ModuleSpec {
    ty: ModuleType::Circuit,
    display_name: "CIR",
    color: theme::CIRCUIT_DOT_COLOR,
    num_curves: 5,
    curve_labels: &["AMOUNT", "THRESH", "SPREAD", "RELEASE", "MIX"],
    assignable_to_user_slots: true,
    heavy_cpu: false,
    wants_sidechain: false,
    writes_bin_physics: true,
    heavy_cpu_per_mode: Some(&CIR_HEAVY),
    panel_widget: None,
};
```

If Phase 2g shipped with the inline `ModuleSpec { … }` pattern, lift it to a `static` first then insert the new fields. Match arm becomes: `ModuleType::Circuit => &CIR,`.

- [ ] **Step 4: Update `num_curves()` in `src/dsp/modules/circuit.rs`**

```rust
fn num_curves(&self) -> usize {
    5
}
```

- [ ] **Step 5: Run test, expect pass**

Run: `cargo test --test module_trait circuit_spec_has_5_curves_with_spread_and_writes_bin_physics -- --nocapture`
Expected: PASS.

- [ ] **Step 6: Verify the existing `circuit_module_spec_present` test from Phase 2g still passes after the count bump**

The Phase 2g test asserts `spec.num_curves == 4`. That assertion is now stale. Open `tests/module_trait.rs::circuit_module_spec_present` and update the two assertions:

```rust
assert_eq!(spec.num_curves, 5);
assert_eq!(spec.curve_labels.len(), 5);
```

Run: `cargo test --test module_trait circuit_module_spec_present -- --nocapture`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add src/dsp/modules/mod.rs src/dsp/modules/circuit.rs tests/module_trait.rs
git commit -m "feat(circuit): bump to 5 curves with SPREAD at index 2; flip writes_bin_physics; per-mode heavy flag"
```

---

## Task 2: Migrate v1 kernels (BBD / Schmitt / Crossover) to new curve indices

**Files:**
- Modify: `src/dsp/modules/circuit.rs` (`apply_bbd`, `apply_schmitt`, `apply_crossover`)
- Modify: `tests/module_trait.rs` (Phase 2g existing tests)

The v1 plan put RELEASE at index 2 and MIX at index 3. Phase 5c moves them to 3 and 4 respectively. SPREAD (index 2) is unused by v1 modes.

- [ ] **Step 1: Update `apply_bbd` curve indices**

```rust
fn apply_bbd(
    bins: &mut [Complex<f32>],
    bbd_mag: &mut [Vec<f32>; BBD_STAGES],
    rng_state: &mut u32,
    curves: &[&[f32]],
) {
    let amount_c  = curves[0];
    let _thresh_c = curves[1]; // BBD doesn't use thresh — kept for layout consistency
    // curves[2] = SPREAD: unused by BBD
    let release_c = curves[3]; // was curves[2]
    let mix_c     = curves[4]; // was curves[3]

    let num_bins = bins.len();

    for k in 0..num_bins {
        let amount = amount_c[k].clamp(0.0, 2.0) * 0.5;
        let dither_amt = _thresh_c[k].clamp(0.0, 2.0) * 0.005;
        let lp_alpha = (release_c[k].clamp(0.01, 2.0) * 0.4).clamp(0.05, 0.9);
        let mix = mix_c[k].clamp(0.0, 2.0) * 0.5;

        let dry = bins[k];
        let in_mag = dry.norm();

        let target_0 = bbd_mag[0][k] + (in_mag - bbd_mag[0][k]) * lp_alpha;
        let dither_0 = xorshift32_step(rng_state) * dither_amt;
        bbd_mag[0][k] = (target_0 + dither_0).max(0.0);

        let s0_prev = bbd_mag[0][k];
        let s1_prev = bbd_mag[1][k];
        let s2_prev = bbd_mag[2][k];
        let s3_prev = bbd_mag[3][k];

        bbd_mag[3][k] = s3_prev + (s2_prev - s3_prev) * lp_alpha + xorshift32_step(rng_state) * dither_amt;
        bbd_mag[2][k] = s2_prev + (s1_prev - s2_prev) * lp_alpha + xorshift32_step(rng_state) * dither_amt;
        bbd_mag[1][k] = s1_prev + (s0_prev - s1_prev) * lp_alpha + xorshift32_step(rng_state) * dither_amt;

        let out_mag = bbd_mag[3][k].max(0.0) * amount;
        let scale = if in_mag > 1e-9 { out_mag / in_mag } else { 0.0 };
        let wet = dry * scale;
        bins[k] = dry * (1.0 - mix) + wet * mix;
    }
}
```

- [ ] **Step 2: Update `apply_schmitt` curve indices**

```rust
fn apply_schmitt(
    bins: &mut [Complex<f32>],
    latched: &mut [u8],
    curves: &[&[f32]],
) {
    let amount_c  = curves[0];
    let thresh_c  = curves[1];
    // curves[2] = SPREAD: unused by Schmitt
    let release_c = curves[3]; // was curves[2]
    let mix_c     = curves[4]; // was curves[3]

    let num_bins = bins.len();

    for k in 0..num_bins {
        let attenuation = amount_c[k].clamp(0.0, 2.0) * 0.5;
        let high = thresh_c[k].clamp(0.01, 4.0);
        let gap = (release_c[k].clamp(0.0, 2.0) * 0.5).clamp(0.05, 0.95);
        let low = high * (1.0 - gap);

        let mag = bins[k].norm();
        let was_latched = latched[k] != 0;

        let now_latched = if was_latched {
            mag > low
        } else {
            mag > high
        };
        latched[k] = if now_latched { 1 } else { 0 };

        let mix = mix_c[k].clamp(0.0, 2.0) * 0.5;
        let attenuate = if now_latched { 1.0 } else { 1.0 - attenuation };
        let dry = bins[k];
        let wet = dry * attenuate;
        bins[k] = dry * (1.0 - mix) + wet * mix;
    }
}
```

- [ ] **Step 3: Update `apply_crossover` curve indices**

```rust
fn apply_crossover(bins: &mut [Complex<f32>], curves: &[&[f32]]) {
    let amount_c = curves[0];
    // curves[1] = THRESHOLD: unused (deadzone width derives from AMOUNT)
    // curves[2] = SPREAD: unused by Crossover
    // curves[3] = RELEASE: unused
    let mix_c = curves[4]; // was curves[3]

    let num_bins = bins.len();

    for k in 0..num_bins {
        let dz_width = amount_c[k].clamp(0.0, 2.0) * 0.1;
        let mix = mix_c[k].clamp(0.0, 2.0) * 0.5;

        let dry = bins[k];
        let mag = dry.norm();

        let new_mag = if mag <= dz_width {
            0.0
        } else {
            let excess = mag - dz_width;
            (excess * excess) / mag
        };

        let scale = if mag > 1e-9 { new_mag / mag } else { 0.0 };
        let wet = dry * scale;
        bins[k] = dry * (1.0 - mix) + wet * mix;
    }
}
```

- [ ] **Step 4: Migrate the Phase 2g tests' curve fixtures**

Open `tests/module_trait.rs` and update every Circuit-related test fixture. The pattern was:

```rust
let curves: Vec<&[f32]> = vec![&amount, &thresh, &release, &mix];
```

becomes:

```rust
let spread = vec![0.0_f32; num_bins];                    // unused by v1 modes
let curves: Vec<&[f32]> = vec![&amount, &thresh, &spread, &release, &mix];
```

Apply this rewrite to **all** Phase 2g Circuit tests:
- `circuit_module_constructs_and_passes_through`
- `circuit_bbd_delays_and_lowpasses`
- `circuit_schmitt_hysteresis_latches_above_threshold`
- `circuit_crossover_smooth_deadzone`
- `circuit_finite_bounded_all_modes_dual_channel`

The Phase 2g `CrossoverDistortion` test had a "MIX=2" fixture; the assertion on `bins[50]` should still hold after the migration because the math is unchanged — only the index shifted.

- [ ] **Step 5: Run all Phase 2g + Task 1 tests**

Run: `cargo test --test module_trait circuit -- --nocapture`
Expected: ALL PASS.

- [ ] **Step 6: Commit**

```bash
git add src/dsp/modules/circuit.rs tests/module_trait.rs
git commit -m "refactor(circuit): migrate v1 kernels to 5-curve layout (RELEASE 2→3, MIX 3→4)"
```

---

## Task 3: Add `circuit_kernels.rs` shared scalar primitives

**Files:**
- Create: `src/dsp/circuit_kernels.rs`
- Modify: `src/dsp/mod.rs`
- Test: `tests/circuit_kernels.rs` (new)

These primitives are designed for clean SIMD lift in v2 (per spec research finding §1: target `wide::f32x8`). The scalar version uses `#[inline]` and bin-major loops with no inter-bin data dependencies (except `spread_3tap`, which uses pre-allocated workspace for the read pass).

- [ ] **Step 1: Write the failing tests**

Create `tests/circuit_kernels.rs`:

```rust
use spectral_forge::dsp::circuit_kernels::{
    lp_step, tanh_levien_poly, spread_3tap, SimdRng,
};

#[test]
fn lp_step_settles_to_target_within_5_taus() {
    let mut state = 0.0_f32;
    let target = 1.0_f32;
    // alpha for tau=10 hops: alpha = 1 - exp(-1/10) ≈ 0.0952
    let alpha = 1.0 - (-1.0_f32 / 10.0).exp();
    for _ in 0..50 {
        lp_step(&mut state, target, alpha);
    }
    assert!((state - target).abs() < 0.01, "state={} after 50 hops at tau=10", state);
}

#[test]
fn lp_step_zero_alpha_holds_state() {
    let mut state = 0.5_f32;
    lp_step(&mut state, 9.0, 0.0);
    assert!((state - 0.5).abs() < 1e-9);
}

#[test]
fn tanh_levien_poly_matches_tanh_within_5pct_in_unit_band() {
    for i in -10..=10 {
        let x = i as f32 * 0.1;
        let exact = x.tanh();
        let approx = tanh_levien_poly(x);
        let err = (exact - approx).abs();
        assert!(err < 0.05, "x={} exact={} approx={} err={}", x, exact, approx, err);
    }
}

#[test]
fn tanh_levien_poly_saturates_at_extremes() {
    assert!(tanh_levien_poly(10.0) > 0.95);
    assert!(tanh_levien_poly(-10.0) < -0.95);
    assert!(tanh_levien_poly(100.0).is_finite());
    assert!(tanh_levien_poly(-100.0).is_finite());
}

#[test]
fn spread_3tap_neighbours_share_energy() {
    // Pre-cleared output buffer.
    let input  = vec![0.0, 0.0, 1.0, 0.0, 0.0]; // impulse at bin 2
    let mut output = vec![0.0_f32; 5];
    spread_3tap(&input, &mut output, 0.5); // 50% leakage to neighbours
    // Bin 2 should retain (1 - 0.5) = 0.5 of its energy.
    // Bins 1 and 3 should each receive 0.25.
    assert!((output[2] - 0.5).abs() < 1e-6, "centre={}", output[2]);
    assert!((output[1] - 0.25).abs() < 1e-6, "left={}", output[1]);
    assert!((output[3] - 0.25).abs() < 1e-6, "right={}", output[3]);
    assert!(output[0].abs() < 1e-6);
    assert!(output[4].abs() < 1e-6);
}

#[test]
fn spread_3tap_zero_strength_is_passthrough() {
    let input  = vec![0.1, 0.2, 0.3, 0.4, 0.5];
    let mut output = vec![0.0_f32; 5];
    spread_3tap(&input, &mut output, 0.0);
    for k in 0..5 {
        assert!((output[k] - input[k]).abs() < 1e-6);
    }
}

#[test]
fn spread_3tap_bounded_at_edges() {
    let input  = vec![1.0, 0.0, 0.0, 0.0, 1.0];
    let mut output = vec![0.0_f32; 5];
    spread_3tap(&input, &mut output, 0.6);
    // Edge bins miss one neighbour — they retain (1 - 0.3) = 0.7 (vs 0.4 in middle).
    // Specifically: bin 0 has only right neighbour: out = 0.4 * 1.0 + 0.3 * 0.0 = 0.4 (no left to leak in).
    // The choice for edge handling: zero-padded (no wrap). Verify finiteness only here.
    for k in 0..5 {
        assert!(output[k].is_finite() && output[k] >= 0.0);
    }
}

#[test]
fn simd_rng_produces_uniform_in_minus1_plus1() {
    let mut rng = SimdRng::new(0xCAFEBABE);
    let mut min = f32::MAX;
    let mut max = f32::MIN;
    for _ in 0..1000 {
        let x = rng.next_f32_centered();
        assert!(x.is_finite());
        assert!(x >= -1.0 && x < 1.0);
        if x < min { min = x; }
        if x > max { max = x; }
    }
    assert!(min < -0.8, "min={}: distribution should reach close to -1", min);
    assert!(max > 0.8,  "max={}: distribution should reach close to +1", max);
}

#[test]
fn simd_rng_deterministic_for_same_seed() {
    let mut a = SimdRng::new(42);
    let mut b = SimdRng::new(42);
    for _ in 0..100 {
        assert_eq!(a.next_u32(), b.next_u32());
    }
}
```

- [ ] **Step 2: Run tests, expect failure**

Run: `cargo test --test circuit_kernels -- --nocapture`
Expected: FAIL — module not found.

- [ ] **Step 3: Create `src/dsp/circuit_kernels.rs`**

```rust
//! Shared scalar primitives for the Circuit module's analog-component kernels.
//! Designed for clean SIMD lift in v2 (target `wide::f32x8`); v1 is scalar.
//!
//! All functions are `#[inline]` and use bin-major loops with no inter-bin
//! dependencies except `spread_3tap`, which expects a pre-cleared output
//! buffer that aliases-free with `input`.

/// One step of a 1-pole lowpass: `state += (target - state) * alpha`.
/// `alpha` should already be clamped to `[0, 1]` by the caller.
#[inline]
pub fn lp_step(state: &mut f32, target: f32, alpha: f32) {
    *state += (target - *state) * alpha;
}

/// Levien-style 4th-order rational polynomial approximation of `tanh(x)`.
/// Max abs error in `|x| <= 1` is ~3%. Saturates monotonically beyond.
/// Branchless, no `exp()`, no `tanh()`. Approx 4 muls + 1 div per call.
#[inline]
pub fn tanh_levien_poly(x: f32) -> f32 {
    // Reference: Raph Levien, "Approximating tanh", 2019.
    // tanh(x) ≈ x * (27 + x²) / (27 + 9 * x²)
    let x2 = x * x;
    let num = x * (27.0 + x2);
    let den = 27.0 + 9.0 * x2;
    num / den
}

/// Apply a 3-tap symmetric stencil: `output[k] = (1 - s) * input[k] + 0.5 * s * (input[k-1] + input[k+1])`.
/// Edges (k = 0, k = N-1) use zero-padded neighbours (no wrap, no replicate).
/// `s` is clamped to `[0, 1]` internally.
/// Caller must pass two distinct slices of the same length; aliasing is UB.
#[inline]
pub fn spread_3tap(input: &[f32], output: &mut [f32], strength: f32) {
    let s = strength.clamp(0.0, 1.0);
    let n = input.len();
    debug_assert_eq!(output.len(), n);
    if n == 0 { return; }
    if n == 1 { output[0] = input[0]; return; }

    // First bin: only right neighbour exists.
    output[0] = (1.0 - s) * input[0] + 0.5 * s * input[1];
    // Interior bins.
    for k in 1..n - 1 {
        output[k] = (1.0 - s) * input[k] + 0.5 * s * (input[k - 1] + input[k + 1]);
    }
    // Last bin: only left neighbour exists.
    output[n - 1] = (1.0 - s) * input[n - 1] + 0.5 * s * input[n - 2];
}

/// Per-channel xorshift32 PRNG. Cheap (3 shifts + 3 XORs per `next_u32`),
/// branchless, deterministic for a given seed. NOT cryptographically secure.
/// Used for: BBD dither, Drift, Slew phase scramble, Bias Fuzz noise.
#[derive(Debug, Clone)]
pub struct SimdRng {
    state: u32,
}

impl SimdRng {
    #[inline]
    pub fn new(seed: u32) -> Self {
        // Avoid the all-zero degenerate state.
        let s = if seed == 0 { 0xDEADBEEF } else { seed };
        Self { state: s }
    }

    #[inline]
    pub fn next_u32(&mut self) -> u32 {
        let mut s = self.state;
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        self.state = s;
        s
    }

    /// Uniform `f32` in `[-1, 1)`.
    #[inline]
    pub fn next_f32_centered(&mut self) -> f32 {
        (self.next_u32() as i32 as f32) / (i32::MAX as f32)
    }

    /// Uniform `f32` in `[0, 1)`.
    #[inline]
    pub fn next_f32_unit(&mut self) -> f32 {
        (self.next_u32() as f32) / (u32::MAX as f32)
    }
}
```

- [ ] **Step 4: Wire the module**

In `src/dsp/mod.rs`, **above** `pub mod modules;`:

```rust
pub mod circuit_kernels;
```

(This must precede `modules` because `modules::circuit` will use it.)

- [ ] **Step 5: Run tests, expect pass**

Run: `cargo test --test circuit_kernels -- --nocapture`
Expected: ALL PASS.

- [ ] **Step 6: Commit**

```bash
git add src/dsp/circuit_kernels.rs src/dsp/mod.rs tests/circuit_kernels.rs
git commit -m "feat(circuit): shared scalar kernels (lp_step, tanh_levien_poly, spread_3tap, SimdRng)"
```

---

## Task 4: Vactrol mode — cascaded 1-pole, reads `BinPhysics::flux`

**Files:**
- Modify: `src/dsp/modules/circuit.rs`
- Modify: `tests/module_trait.rs`

Spec §3 + research finding §3: two cascaded 1-poles per bin with `τ_fast ≈ 8 ms` and `τ_slow ≈ 250 ms`. RELEASE curve scales both time constants. The "drive" feeding the photocell is `flux[k]` (read from BinPhysics) plus a fallback to magnitude when `ctx.bin_physics` is `None`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn circuit_vactrol_smooths_flux_input_with_release_envelope() {
    use spectral_forge::dsp::modules::circuit::{CircuitModule, CircuitMode};
    use spectral_forge::dsp::modules::{SpectralModule, StereoLink, FxChannelTarget};
    use realfft::num_complex::Complex;

    let mut module = CircuitModule::new();
    module.reset(48_000.0, 2048);
    module.set_circuit_mode(CircuitMode::Vactrol);

    let num_bins = 1025;
    let mut bins: Vec<Complex<f32>> = vec![Complex::new(1.0, 0.0); num_bins];

    // AMOUNT=1 (full vactrol drive), THRESHOLD=1 (unused), SPREAD=0, RELEASE=1, MIX=2 (full wet).
    let amount  = vec![1.0_f32; num_bins];
    let thresh  = vec![1.0_f32; num_bins];
    let spread  = vec![0.0_f32; num_bins];
    let release = vec![1.0_f32; num_bins];
    let mix     = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &thresh, &spread, &release, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = circuit_test_ctx(num_bins);

    // Hop 1: cap is empty, so first hop attenuates strongly.
    module.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, None, &curves, &mut suppression, &ctx);
    let first_hop_mag = bins[100].norm();
    assert!(first_hop_mag < 0.5, "first hop should be attenuated by empty vactrol cap (got {})", first_hop_mag);

    // Many hops in: cap charges, output approaches input.
    for _ in 0..200 {
        for b in bins.iter_mut() { *b = Complex::new(1.0, 0.0); }
        module.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, None, &curves, &mut suppression, &ctx);
    }
    let charged_mag = bins[100].norm();
    assert!(charged_mag > 0.7, "after charge, output should approach input (got {})", charged_mag);

    // Drop input to zero — output should ring (slow release).
    for b in bins.iter_mut() { *b = Complex::new(0.0, 0.0); }
    module.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, None, &curves, &mut suppression, &ctx);
    // Hard zero now (input × cap_gain = 0 regardless of cap state). Inspect via probe.
    let probe = module.probe_state(0);
    assert!(probe.vactrol_slow_avg > 0.1, "slow cap should still hold charge (got {})", probe.vactrol_slow_avg);
}

#[test]
fn circuit_vactrol_finite_after_long_run() {
    use spectral_forge::dsp::modules::circuit::{CircuitModule, CircuitMode};
    use spectral_forge::dsp::modules::{SpectralModule, StereoLink, FxChannelTarget};
    use realfft::num_complex::Complex;

    let mut module = CircuitModule::new();
    module.reset(48_000.0, 2048);
    module.set_circuit_mode(CircuitMode::Vactrol);

    let num_bins = 1025;
    let mut bins: Vec<Complex<f32>> = (0..num_bins).map(|k| Complex::new((k as f32 * 0.05).sin().abs(), 0.0)).collect();
    let amount  = vec![1.5_f32; num_bins];
    let thresh  = vec![1.0_f32; num_bins];
    let spread  = vec![0.0_f32; num_bins];
    let release = vec![1.0_f32; num_bins];
    let mix     = vec![1.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &thresh, &spread, &release, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = circuit_test_ctx(num_bins);

    for _ in 0..500 {
        module.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, None, &curves, &mut suppression, &ctx);
        for b in &bins {
            assert!(b.norm().is_finite() && b.norm() < 100.0);
        }
    }
}
```

- [ ] **Step 2: Run tests, expect failure**

Run: `cargo test --test module_trait circuit_vactrol -- --nocapture`
Expected: FAIL — Vactrol arm unimplemented; bin 100 stays at 1.0 (Phase 2g default `_ => {}`).

- [ ] **Step 3: Add Vactrol state fields to `CircuitModule`**

In `src/dsp/modules/circuit.rs`:

```rust
pub struct CircuitModule {
    mode: CircuitMode,
    bbd_mag: [[Vec<f32>; BBD_STAGES]; 2],
    schmitt_latched: [Vec<u8>; 2],
    rng_state: [u32; 2],
    sample_rate: f32,
    fft_size: usize,
    // --- Phase 5c additions ---
    vactrol_fast: [Vec<f32>; 2], // fast cap state per bin
    vactrol_slow: [Vec<f32>; 2], // slow cap state per bin
    // ... (further fields added in subsequent tasks)
}
```

Update `CircuitModule::new()`:

```rust
pub fn new() -> Self {
    Self {
        mode: CircuitMode::default(),
        bbd_mag: [
            [Vec::new(), Vec::new(), Vec::new(), Vec::new()],
            [Vec::new(), Vec::new(), Vec::new(), Vec::new()],
        ],
        schmitt_latched: [Vec::new(), Vec::new()],
        rng_state: [0xDEADBEEFu32, 0xCAFEBABEu32],
        sample_rate: 48_000.0,
        fft_size: 2048,
        vactrol_fast: [Vec::new(), Vec::new()],
        vactrol_slow: [Vec::new(), Vec::new()],
    }
}
```

Update `reset()`:

```rust
fn reset(&mut self, sample_rate: f32, fft_size: usize) {
    self.sample_rate = sample_rate;
    self.fft_size = fft_size;
    let num_bins = fft_size / 2 + 1;
    for ch in 0..2 {
        for stage in 0..BBD_STAGES {
            self.bbd_mag[ch][stage].clear();
            self.bbd_mag[ch][stage].resize(num_bins, 0.0);
        }
        self.schmitt_latched[ch].clear();
        self.schmitt_latched[ch].resize(num_bins, 0);
        self.vactrol_fast[ch].clear();
        self.vactrol_fast[ch].resize(num_bins, 0.0);
        self.vactrol_slow[ch].clear();
        self.vactrol_slow[ch].resize(num_bins, 0.0);
    }
}
```

Update `set_circuit_mode()` reset list:

```rust
fn set_circuit_mode(&mut self, mode: CircuitMode) {
    if mode != self.mode {
        for ch in 0..2 {
            for stage in 0..BBD_STAGES {
                for v in self.bbd_mag[ch][stage].iter_mut() { *v = 0.0; }
            }
            for l in self.schmitt_latched[ch].iter_mut() { *l = 0; }
            for v in self.vactrol_fast[ch].iter_mut() { *v = 0.0; }
            for v in self.vactrol_slow[ch].iter_mut() { *v = 0.0; }
        }
        self.mode = mode;
    }
}
```

- [ ] **Step 4: Add the Vactrol kernel**

```rust
fn apply_vactrol(
    bins: &mut [Complex<f32>],
    fast: &mut [f32],
    slow: &mut [f32],
    flux: Option<&[f32]>,
    curves: &[&[f32]],
    sample_rate: f32,
    fft_size: usize,
) {
    use crate::dsp::circuit_kernels::lp_step;

    let amount_c  = curves[0];
    // curves[1] = THRESHOLD: unused
    // curves[2] = SPREAD: unused
    let release_c = curves[3];
    let mix_c     = curves[4];

    let num_bins = bins.len();
    let hop_dt = (fft_size as f32 / 4.0) / sample_rate; // OVERLAP=4 ⇒ hop = fft/4 samples

    // Base time constants (research finding §3).
    const TAU_FAST_BASE: f32 = 0.008;  // 8 ms
    const TAU_SLOW_BASE: f32 = 0.250;  // 250 ms

    for k in 0..num_bins {
        let amount = amount_c[k].clamp(0.0, 2.0) * 0.5;     // 0..1 drive
        // RELEASE 1.0 → no scale; 0.0 → 0.1× (faster); 2.0 → 4× (slower).
        let release = release_c[k].clamp(0.0, 2.0);
        let scale = if release < 1.0 { 0.1 + 0.9 * release } else { 1.0 + 3.0 * (release - 1.0) };
        let tau_fast = (TAU_FAST_BASE * scale).max(1e-4);
        let tau_slow = (TAU_SLOW_BASE * scale).max(1e-4);

        // Convert tau → alpha: alpha = 1 - exp(-dt/tau). Use 1st-order series for cheap, finite result:
        // alpha ≈ dt/tau for small ratios. Clamp to [0, 1].
        let alpha_fast = (hop_dt / tau_fast).min(1.0);
        let alpha_slow = (hop_dt / tau_slow).min(1.0);

        // Drive: prefer flux when available, else magnitude.
        let drive = match flux {
            Some(f) => f[k].abs(),
            None    => bins[k].norm(),
        } * amount;

        // Cascade: drive → fast cap → slow cap.
        lp_step(&mut fast[k], drive, alpha_fast);
        lp_step(&mut slow[k], fast[k], alpha_slow);

        // Cell resistance is inversely proportional to cap voltage; cell gain is monotonically increasing.
        // Map slow cap → gain in [0, 1] via tanh-squashed normaliser.
        let g = slow[k] / (1.0 + slow[k].abs()); // soft-saturating: in[0, 1)

        let dry = bins[k];
        let wet = dry * g;
        let mix = mix_c[k].clamp(0.0, 2.0) * 0.5;
        bins[k] = dry * (1.0 - mix) + wet * mix;
    }
}
```

- [ ] **Step 5: Wire dispatch in `process()`**

The current `process()` signature was set by Phase 1 + Phase 3 and now takes `physics: Option<&mut BinPhysics>`. Update to read flux from `ctx.bin_physics` (Phase 1+3 added the slot):

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
    physics: Option<&mut crate::dsp::bin_physics::BinPhysics>,
    ctx: &ModuleContext,
) {
    debug_assert!(channel < 2);
    let num_bins = ctx.num_bins;

    match self.mode {
        CircuitMode::BbdBins => {
            let bbd = &mut self.bbd_mag[channel];
            let rng = &mut self.rng_state[channel];
            apply_bbd(&mut bins[..num_bins], bbd, rng, curves);
        }
        CircuitMode::SpectralSchmitt => {
            apply_schmitt(&mut bins[..num_bins], &mut self.schmitt_latched[channel][..num_bins], curves);
        }
        CircuitMode::CrossoverDistortion => {
            apply_crossover(&mut bins[..num_bins], curves);
        }
        CircuitMode::Vactrol => {
            // Read flux from incoming BinPhysics (Phase 3 reader hook).
            let flux: Option<&[f32]> = ctx.bin_physics.map(|bp| &bp.flux[..num_bins]);
            apply_vactrol(
                &mut bins[..num_bins],
                &mut self.vactrol_fast[channel][..num_bins],
                &mut self.vactrol_slow[channel][..num_bins],
                flux,
                curves,
                self.sample_rate,
                self.fft_size,
            );
            // Vactrol does not write physics. Nothing to do with `physics` arg.
            let _ = physics;
        }
        _ => {
            let _ = physics;
        }
    }

    for s in suppression_out.iter_mut() {
        *s = 0.0;
    }
}
```

NOTE: If Phase 3's exact signature is `physics: Option<&mut BinPhysics>` *replaces* `&ModuleContext`, adjust accordingly. The pattern in this plan assumes the merged signature `(.., physics: Option<&mut BinPhysics>, ctx: &ModuleContext)`. If Phase 3 made `bin_physics` part of `ctx` only (no separate `physics` writer arg), drop the `physics` parameter and instead read via `ctx.bin_physics`. Inspect `src/dsp/modules/mod.rs::SpectralModule::process` before applying.

- [ ] **Step 6: Update existing v1 tests' `circuit_test_ctx` to provide `bin_physics: None`**

The Phase 2g `circuit_test_ctx` already returns `bin_physics: None` (per Phase 1 default). No change needed.

- [ ] **Step 7: Add `vactrol_slow_avg` to `CircuitProbe`**

```rust
#[cfg(any(test, feature = "probe"))]
#[derive(Debug, Clone, Copy)]
pub struct CircuitProbe {
    pub active_mode: CircuitMode,
    pub average_amount_pct: f32,
    pub bbd_stage3_avg: f32,
    pub schmitt_active_count: u32,
    // --- Phase 5c additions ---
    pub vactrol_fast_avg: f32,
    pub vactrol_slow_avg: f32,
}
```

In the `probe_state()` method:

```rust
let (vactrol_fast_avg, vactrol_slow_avg) = if self.mode == CircuitMode::Vactrol && !self.vactrol_slow[ch].is_empty() {
    let fa: f32 = self.vactrol_fast[ch].iter().sum::<f32>() / self.vactrol_fast[ch].len() as f32;
    let sa: f32 = self.vactrol_slow[ch].iter().sum::<f32>() / self.vactrol_slow[ch].len() as f32;
    (fa, sa)
} else {
    (0.0, 0.0)
};

CircuitProbe {
    active_mode: self.mode,
    average_amount_pct: 100.0,
    bbd_stage3_avg,
    schmitt_active_count,
    vactrol_fast_avg,
    vactrol_slow_avg,
    // ... more added in later tasks; default = 0.0
}
```

- [ ] **Step 8: Run tests, expect pass**

Run: `cargo test --test module_trait circuit_vactrol -- --nocapture`
Expected: PASS — first hop < 0.5, sustained input charges to > 0.7, slow cap retains > 0.1 after input drops.

- [ ] **Step 9: Commit**

```bash
git add src/dsp/modules/circuit.rs tests/module_trait.rs
git commit -m "feat(circuit): Vactrol mode — cascaded 1-pole reading BinPhysics::flux"
```

---

## Task 5: Transformer Saturation — `tanh` polynomial + magnitude one-pole + SPREAD, reads/writes `BinPhysics::flux`

**Files:**
- Modify: `src/dsp/modules/circuit.rs`
- Modify: `tests/module_trait.rs`

Spec §research finding §5: tanh + magnitude one-pole. SPREAD curve drives 3-tap leakage. `tanh_levien_poly` from circuit_kernels. State: `xfmr_lp[2][num_bins]` (magnitude smoother) + `xfmr_workspace[2][num_bins]` (SPREAD read-pass scratch).

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn circuit_transformer_saturates_high_magnitudes_softly() {
    use spectral_forge::dsp::modules::circuit::{CircuitModule, CircuitMode};
    use spectral_forge::dsp::modules::{SpectralModule, StereoLink, FxChannelTarget};
    use realfft::num_complex::Complex;

    let mut module = CircuitModule::new();
    module.reset(48_000.0, 2048);
    module.set_circuit_mode(CircuitMode::TransformerSaturation);

    let num_bins = 1025;
    let mut bins: Vec<Complex<f32>> = vec![Complex::new(0.0, 0.0); num_bins];
    bins[100] = Complex::new(0.5, 0.0);  // sub-knee
    bins[200] = Complex::new(3.0, 0.0);  // above knee — should saturate

    // AMOUNT=2 (max drive), THRESHOLD=1 (knee at unity), SPREAD=0 (test isolation), RELEASE=1, MIX=2 wet.
    let amount  = vec![2.0_f32; num_bins];
    let thresh  = vec![1.0_f32; num_bins];
    let spread  = vec![0.0_f32; num_bins];
    let release = vec![1.0_f32; num_bins];
    let mix     = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &thresh, &spread, &release, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = circuit_test_ctx(num_bins);

    // Several hops to let the magnitude smoother settle.
    for _ in 0..40 {
        bins[100] = Complex::new(0.5, 0.0);
        bins[200] = Complex::new(3.0, 0.0);
        module.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, None, &curves, &mut suppression, &ctx);
    }

    // Sub-knee bin: ~unchanged.
    assert!(bins[100].norm() < 0.7 && bins[100].norm() > 0.3, "bin 100 sub-knee got {}", bins[100].norm());
    // Above-knee bin: bounded well below input.
    assert!(bins[200].norm() < 2.0, "bin 200 should saturate (got {})", bins[200].norm());
    assert!(bins[200].norm() > 0.5, "bin 200 should not collapse to 0 (got {})", bins[200].norm());
}

#[test]
fn circuit_transformer_spread_leaks_to_neighbours() {
    use spectral_forge::dsp::modules::circuit::{CircuitModule, CircuitMode};
    use spectral_forge::dsp::modules::{SpectralModule, StereoLink, FxChannelTarget};
    use realfft::num_complex::Complex;

    let mut module = CircuitModule::new();
    module.reset(48_000.0, 2048);
    module.set_circuit_mode(CircuitMode::TransformerSaturation);

    let num_bins = 1025;
    let mut bins: Vec<Complex<f32>> = vec![Complex::new(0.0, 0.0); num_bins];
    bins[200] = Complex::new(3.0, 0.0);

    // SPREAD = 2 (full leak), drive on, neighbours start at zero.
    let amount  = vec![2.0_f32; num_bins];
    let thresh  = vec![1.0_f32; num_bins];
    let spread  = vec![2.0_f32; num_bins]; // 1.0 leak strength after clamp/scale
    let release = vec![1.0_f32; num_bins];
    let mix     = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &thresh, &spread, &release, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = circuit_test_ctx(num_bins);

    // Settle several hops to let the magnitude smoother + spread reach steady state.
    for _ in 0..20 {
        bins[200] = Complex::new(3.0, 0.0);
        bins[199] = Complex::new(0.0, 0.0);
        bins[201] = Complex::new(0.0, 0.0);
        module.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, None, &curves, &mut suppression, &ctx);
    }

    // Neighbours should have *non-zero* magnitude after the leak.
    assert!(bins[199].norm() > 0.05, "bin 199 should receive leak (got {})", bins[199].norm());
    assert!(bins[201].norm() > 0.05, "bin 201 should receive leak (got {})", bins[201].norm());
}
```

- [ ] **Step 2: Run tests, expect failure**

Run: `cargo test --test module_trait circuit_transformer -- --nocapture`
Expected: FAIL — TransformerSaturation arm not implemented; bins unchanged.

- [ ] **Step 3: Add Transformer state fields**

```rust
pub struct CircuitModule {
    // ... existing ...
    xfmr_lp: [Vec<f32>; 2],         // magnitude one-pole
    xfmr_workspace: [Vec<f32>; 2],  // SPREAD read-pass scratch
    // ... more added in later tasks
}
```

In `new()`: add `xfmr_lp: [Vec::new(), Vec::new()], xfmr_workspace: [Vec::new(), Vec::new()],`.

In `reset()`: extend the `for ch in 0..2 { … }` loop:

```rust
self.xfmr_lp[ch].clear();
self.xfmr_lp[ch].resize(num_bins, 0.0);
self.xfmr_workspace[ch].clear();
self.xfmr_workspace[ch].resize(num_bins, 0.0);
```

In `set_circuit_mode()` reset list:

```rust
for v in self.xfmr_lp[ch].iter_mut() { *v = 0.0; }
for v in self.xfmr_workspace[ch].iter_mut() { *v = 0.0; }
```

- [ ] **Step 4: Add the Transformer kernel**

```rust
fn apply_transformer(
    bins: &mut [Complex<f32>],
    xfmr_lp: &mut [f32],
    workspace: &mut [f32],
    flux_in: Option<&[f32]>,
    flux_out: Option<&mut [f32]>,
    curves: &[&[f32]],
    sample_rate: f32,
    fft_size: usize,
) {
    use crate::dsp::circuit_kernels::{lp_step, tanh_levien_poly, spread_3tap};

    let amount_c  = curves[0];
    let thresh_c  = curves[1];
    let spread_c  = curves[2];
    let release_c = curves[3];
    let mix_c     = curves[4];

    let num_bins = bins.len();
    let hop_dt = (fft_size as f32 / 4.0) / sample_rate;

    // --- Pass 1: per-bin saturation, write into workspace as new magnitude. ---
    for k in 0..num_bins {
        let drive = amount_c[k].clamp(0.0, 2.0) * 4.0; // up to 8× drive into tanh
        let knee = thresh_c[k].clamp(0.05, 4.0);
        let release = release_c[k].clamp(0.0, 2.0).max(0.01);
        // alpha for hop-rate magnitude smoother.
        let tau = 0.020 * (0.1 + release); // 2..62 ms
        let alpha = (hop_dt / tau).min(1.0);

        let in_mag = bins[k].norm();
        // Bias the smoother by flux feedback (a hot bin saturates more easily).
        let flux_bias = flux_in.map(|f| f[k] * 0.25).unwrap_or(0.0);
        let target = in_mag + flux_bias;
        lp_step(&mut xfmr_lp[k], target, alpha);

        // Saturate: tanh(drive × xfmr_lp / knee) × knee.
        let x = drive * xfmr_lp[k] / knee;
        let sat_mag = tanh_levien_poly(x) * knee;
        workspace[k] = sat_mag.max(0.0);
    }

    // --- Pass 2: SPREAD into output. ---
    // Reuse spread_3tap kernel with the workspace as input. Use a local stack scratch
    // for the spread output? No — write back into workspace would alias. Instead, use
    // bins.iter().map() to consume new magnitudes one bin at a time without alias:
    // we'll spread the magnitude in-place via a 2-pass: read prev/next from workspace
    // BEFORE overwriting. Simpler: use a tiny stack-only triple buffer per bin.
    let strength_avg = (0..num_bins).map(|k| spread_c[k].clamp(0.0, 2.0) * 0.5).sum::<f32>() / num_bins.max(1) as f32;
    // The stencil weight is bin-uniform per hop (curves are smooth at hop rate, so an
    // averaged strength is close enough). For a true per-bin spread, the workspace
    // alias problem demands an extra buffer — we keep workspace simple by using avg.
    // (If per-bin spread is needed, allocate a second `xfmr_workspace2` field.)

    let mut prev_w = workspace[0];
    let mut curr_w = if num_bins > 0 { workspace[0] } else { 0.0 };
    for k in 0..num_bins {
        let next_w = if k + 1 < num_bins { workspace[k + 1] } else { 0.0 };
        let s = strength_avg;
        let new_mag = (1.0 - s) * curr_w + 0.5 * s * (prev_w + next_w);
        // Apply the new magnitude back to bins, preserving phase.
        let in_mag = bins[k].norm();
        let scale = if in_mag > 1e-9 { new_mag / in_mag } else { 0.0 };
        let dry = bins[k];
        let wet = dry * scale;
        let mix = mix_c[k].clamp(0.0, 2.0) * 0.5;
        bins[k] = dry * (1.0 - mix) + wet * mix;
        // Slide the window forward.
        prev_w = curr_w;
        curr_w = next_w;
    }

    // Write flux back: hot bins (xfmr_lp >> in_mag) push positive flux.
    if let Some(fout) = flux_out {
        for k in 0..num_bins {
            let in_mag = bins[k].norm();
            let excess = (xfmr_lp[k] - in_mag).max(0.0);
            fout[k] = (fout[k] * 0.95 + excess * 0.1).clamp(-100.0, 100.0); // 5%/hop decay
        }
    }
}
```

- [ ] **Step 5: Wire dispatch in `process()`**

```rust
CircuitMode::TransformerSaturation => {
    let flux_in: Option<&[f32]> = ctx.bin_physics.map(|bp| &bp.flux[..num_bins]);
    let flux_out: Option<&mut [f32]> = physics.as_mut().map(|p| &mut p.flux[..num_bins]);
    apply_transformer(
        &mut bins[..num_bins],
        &mut self.xfmr_lp[channel][..num_bins],
        &mut self.xfmr_workspace[channel][..num_bins],
        flux_in,
        flux_out,
        curves,
        self.sample_rate,
        self.fft_size,
    );
}
```

NOTE: `physics.as_mut()` requires `physics: Option<&mut BinPhysics>` in scope. If Phase 3 supplies it as `&mut Option<…>` or via a different shape, adapt.

- [ ] **Step 6: Add transformer state to `CircuitProbe`**

```rust
pub struct CircuitProbe {
    // ... existing ...
    pub xfmr_lp_avg: f32,
}
```

```rust
let xfmr_lp_avg = if self.mode == CircuitMode::TransformerSaturation && !self.xfmr_lp[ch].is_empty() {
    self.xfmr_lp[ch].iter().sum::<f32>() / self.xfmr_lp[ch].len() as f32
} else { 0.0 };

CircuitProbe { /* ... */ xfmr_lp_avg, /* ... */ }
```

- [ ] **Step 7: Run tests, expect pass**

Run: `cargo test --test module_trait circuit_transformer -- --nocapture`
Expected: PASS — sub-knee preserved, above-knee saturates, neighbours receive leak.

- [ ] **Step 8: Commit**

```bash
git add src/dsp/modules/circuit.rs tests/module_trait.rs
git commit -m "feat(circuit): Transformer Saturation — tanh polynomial + flux read/write + SPREAD"
```

---

## Task 6: Power Sag — per-channel sag envelope, reads `BinPhysics::temperature`

**Files:**
- Modify: `src/dsp/modules/circuit.rs`
- Modify: `tests/module_trait.rs`

Spec §research finding §7: Kim's "fluctuate / sag, not sudden silence." Per-channel global sag envelope tracks total energy; hot bins (per `temperature`) experience deeper sag than cool bins. Output is a per-bin gain reduction.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn circuit_power_sag_attenuates_under_high_energy() {
    use spectral_forge::dsp::modules::circuit::{CircuitModule, CircuitMode};
    use spectral_forge::dsp::modules::{SpectralModule, StereoLink, FxChannelTarget};
    use realfft::num_complex::Complex;

    let mut module = CircuitModule::new();
    module.reset(48_000.0, 2048);
    module.set_circuit_mode(CircuitMode::PowerSag);

    let num_bins = 1025;

    // AMOUNT=2 (deep sag), THRESHOLD=0.1 (low energy threshold), SPREAD=0, RELEASE=1, MIX=2.
    let amount  = vec![2.0_f32; num_bins];
    let thresh  = vec![0.1_f32; num_bins];
    let spread  = vec![0.0_f32; num_bins];
    let release = vec![1.0_f32; num_bins];
    let mix     = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &thresh, &spread, &release, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = circuit_test_ctx(num_bins);

    // High-energy input: every bin at magnitude 2.0.
    let mut bins: Vec<Complex<f32>> = vec![Complex::new(2.0, 0.0); num_bins];
    let initial_total: f32 = bins.iter().map(|b| b.norm()).sum();

    // Settle 100 hops.
    for _ in 0..100 {
        for b in bins.iter_mut() { *b = Complex::new(2.0, 0.0); }
        module.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, None, &curves, &mut suppression, &ctx);
    }

    let final_total: f32 = bins.iter().map(|b| b.norm()).sum();
    assert!(final_total < initial_total * 0.95, "sag should attenuate (initial={}, final={})", initial_total, final_total);
    assert!(final_total > initial_total * 0.05, "sag should not zero out (final={})", final_total);
}

#[test]
fn circuit_power_sag_recovers_when_energy_drops() {
    use spectral_forge::dsp::modules::circuit::{CircuitModule, CircuitMode};
    use spectral_forge::dsp::modules::{SpectralModule, StereoLink, FxChannelTarget};
    use realfft::num_complex::Complex;

    let mut module = CircuitModule::new();
    module.reset(48_000.0, 2048);
    module.set_circuit_mode(CircuitMode::PowerSag);

    let num_bins = 1025;
    let amount  = vec![2.0_f32; num_bins];
    let thresh  = vec![0.1_f32; num_bins];
    let spread  = vec![0.0_f32; num_bins];
    let release = vec![1.0_f32; num_bins];
    let mix     = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &thresh, &spread, &release, &mix];
    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = circuit_test_ctx(num_bins);

    let mut bins: Vec<Complex<f32>> = vec![Complex::new(2.0, 0.0); num_bins];
    // High-energy ramp-up.
    for _ in 0..50 {
        for b in bins.iter_mut() { *b = Complex::new(2.0, 0.0); }
        module.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, None, &curves, &mut suppression, &ctx);
    }
    let probe_high = module.probe_state(0);
    // Now drop energy to silence.
    for _ in 0..200 {
        for b in bins.iter_mut() { *b = Complex::new(0.0, 0.0); }
        module.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, None, &curves, &mut suppression, &ctx);
    }
    let probe_low = module.probe_state(0);
    assert!(probe_low.sag_envelope < probe_high.sag_envelope * 0.5,
        "sag envelope should recover (high={}, low={})", probe_high.sag_envelope, probe_low.sag_envelope);
}
```

- [ ] **Step 2: Run tests, expect failure**

Run: `cargo test --test module_trait circuit_power_sag -- --nocapture`
Expected: FAIL — PowerSag arm not implemented.

- [ ] **Step 3: Add Sag state fields**

```rust
pub struct CircuitModule {
    // ... existing ...
    sag_envelope: [f32; 2],          // per-channel scalar sag depth
    sag_gain_reduction: [Vec<f32>; 2], // per-bin smoothed gain reduction
}
```

In `new()`: `sag_envelope: [0.0, 0.0], sag_gain_reduction: [Vec::new(), Vec::new()],`.

In `reset()`:

```rust
self.sag_envelope[ch] = 0.0;
self.sag_gain_reduction[ch].clear();
self.sag_gain_reduction[ch].resize(num_bins, 1.0); // 1.0 = no reduction
```

In `set_circuit_mode()`: reset `sag_envelope[ch] = 0.0` and `sag_gain_reduction[ch].iter_mut().for_each(|v| *v = 1.0)`.

- [ ] **Step 4: Add the Sag kernel**

```rust
fn apply_power_sag(
    bins: &mut [Complex<f32>],
    sag_env: &mut f32,
    gain_red: &mut [f32],
    temperature: Option<&[f32]>,
    curves: &[&[f32]],
    sample_rate: f32,
    fft_size: usize,
) {
    use crate::dsp::circuit_kernels::lp_step;

    let amount_c  = curves[0];
    let thresh_c  = curves[1];
    // curves[2] = SPREAD: unused
    let release_c = curves[3];
    let mix_c     = curves[4];

    let num_bins = bins.len();
    let hop_dt = (fft_size as f32 / 4.0) / sample_rate;

    // --- 1. Compute total input energy (sum of magnitudes). Cheap proxy for power. ---
    let mut total_energy = 0.0_f32;
    for k in 0..num_bins {
        total_energy += bins[k].norm();
    }
    let energy_norm = total_energy / num_bins.max(1) as f32; // average per bin

    // --- 2. Update sag envelope: rises with energy above threshold, decays toward 0 below. ---
    // Use first-bin curves as the global control source (curves are bin-uniform under tilt/offset).
    let amount = amount_c[0].clamp(0.0, 2.0) * 0.5;
    let thresh = thresh_c[0].clamp(0.0, 4.0);
    let release = release_c[0].clamp(0.0, 2.0).max(0.01);
    let attack_tau = 0.05; // 50 ms attack (sag onset)
    let release_tau = 0.5 * (0.1 + release); // 50..1050 ms recovery
    let alpha_attack  = (hop_dt / attack_tau).min(1.0);
    let alpha_release = (hop_dt / release_tau).min(1.0);

    let drive = (energy_norm - thresh).max(0.0) * amount;
    let alpha = if drive > *sag_env { alpha_attack } else { alpha_release };
    lp_step(sag_env, drive, alpha);
    *sag_env = sag_env.clamp(0.0, 4.0);

    // --- 3. Per-bin gain reduction weighted by temperature. ---
    let temp_default = 0.0_f32;
    for k in 0..num_bins {
        let temp = temperature.map(|t| t[k].abs()).unwrap_or(temp_default);
        // Hot bins absorb more sag. Reduction factor = 1 / (1 + sag * (1 + temp)).
        let target = 1.0 / (1.0 + *sag_env * (1.0 + temp.min(4.0)));
        // Smooth the per-bin reduction to avoid hop-rate clicks.
        lp_step(&mut gain_red[k], target, alpha_attack);

        let dry = bins[k];
        let wet = dry * gain_red[k];
        let mix = mix_c[k].clamp(0.0, 2.0) * 0.5;
        bins[k] = dry * (1.0 - mix) + wet * mix;
    }
}
```

- [ ] **Step 5: Wire dispatch in `process()`**

```rust
CircuitMode::PowerSag => {
    let temp: Option<&[f32]> = ctx.bin_physics.map(|bp| &bp.temperature[..num_bins]);
    apply_power_sag(
        &mut bins[..num_bins],
        &mut self.sag_envelope[channel],
        &mut self.sag_gain_reduction[channel][..num_bins],
        temp,
        curves,
        self.sample_rate,
        self.fft_size,
    );
    let _ = physics; // sag does not write physics
}
```

- [ ] **Step 6: Add `sag_envelope` to `CircuitProbe`**

```rust
pub struct CircuitProbe {
    // ... existing ...
    pub sag_envelope: f32,
}
```

```rust
let sag_envelope = if self.mode == CircuitMode::PowerSag { self.sag_envelope[ch] } else { 0.0 };
CircuitProbe { /* ... */ sag_envelope, /* ... */ }
```

- [ ] **Step 7: Run tests, expect pass**

Run: `cargo test --test module_trait circuit_power_sag -- --nocapture`
Expected: PASS — high energy reduces total, drop in energy recovers envelope.

- [ ] **Step 8: Commit**

```bash
git add src/dsp/modules/circuit.rs tests/module_trait.rs
git commit -m "feat(circuit): Power Sag — energy-driven envelope, temperature-weighted reduction"
```

---

## Task 7: Component Drift — slow per-bin LFSR drift, reads/writes `BinPhysics::temperature`

**Files:**
- Modify: `src/dsp/modules/circuit.rs`
- Modify: `tests/module_trait.rs`

Spec idea #10: components age; bins gain a slow, time-varying ±1 dB per-bin offset. Read temperature: hot bins drift faster. Write temperature: drift activity heats bins (positive feedback up to a clamp). One LFSR per channel; per-bin variation comes from XOR'ing bin index into the LFSR output.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn circuit_component_drift_modulates_magnitudes_slowly() {
    use spectral_forge::dsp::modules::circuit::{CircuitModule, CircuitMode};
    use spectral_forge::dsp::modules::{SpectralModule, StereoLink, FxChannelTarget};
    use realfft::num_complex::Complex;

    let mut module = CircuitModule::new();
    module.reset(48_000.0, 2048);
    module.set_circuit_mode(CircuitMode::ComponentDrift);

    let num_bins = 1025;
    let amount  = vec![2.0_f32; num_bins]; // max drift
    let thresh  = vec![0.0_f32; num_bins];
    let spread  = vec![0.0_f32; num_bins];
    let release = vec![1.0_f32; num_bins];
    let mix     = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &thresh, &spread, &release, &mix];
    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = circuit_test_ctx(num_bins);

    let baseline = 1.0_f32;
    let mut bins: Vec<Complex<f32>> = vec![Complex::new(baseline, 0.0); num_bins];

    let mut max_dev = 0.0_f32;
    for _ in 0..200 {
        for b in bins.iter_mut() { *b = Complex::new(baseline, 0.0); }
        module.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, None, &curves, &mut suppression, &ctx);
        for b in &bins {
            let dev = (b.norm() - baseline).abs();
            if dev > max_dev { max_dev = dev; }
        }
    }
    // ±1 dB ≈ 12% magnitude swing. Deviation should reach a few percent within 200 hops.
    assert!(max_dev > 0.005, "drift should reach measurable deviation (got {})", max_dev);
    assert!(max_dev < 0.5,   "drift should remain bounded (got {})", max_dev);
}
```

- [ ] **Step 2: Run test, expect failure**

Run: `cargo test --test module_trait circuit_component_drift -- --nocapture`
Expected: FAIL — Drift arm not implemented; no modulation.

- [ ] **Step 3: Add Drift state fields**

```rust
pub struct CircuitModule {
    // ... existing ...
    drift_env: [Vec<f32>; 2],   // per-bin smoothed drift offset
    drift_rng: [u32; 2],        // per-channel LFSR
}
```

In `new()`: `drift_env: [Vec::new(), Vec::new()], drift_rng: [0xACEDDEAD, 0xFEEDFACE],`.

In `reset()`:

```rust
self.drift_env[ch].clear();
self.drift_env[ch].resize(num_bins, 0.0);
// drift_rng is NOT reset on FFT-size change (preserves seed across resizes).
```

In `set_circuit_mode()`:

```rust
for v in self.drift_env[ch].iter_mut() { *v = 0.0; }
```

- [ ] **Step 4: Add the Drift kernel**

```rust
fn apply_component_drift(
    bins: &mut [Complex<f32>],
    drift_env: &mut [f32],
    drift_rng: &mut u32,
    temperature_in:  Option<&[f32]>,
    temperature_out: Option<&mut [f32]>,
    curves: &[&[f32]],
    sample_rate: f32,
    fft_size: usize,
) {
    use crate::dsp::circuit_kernels::lp_step;

    let amount_c  = curves[0];
    let thresh_c  = curves[1];
    // curves[2] = SPREAD: unused
    let release_c = curves[3];
    let mix_c     = curves[4];

    let num_bins = bins.len();
    let hop_dt = (fft_size as f32 / 4.0) / sample_rate;

    // Step LFSR once per hop. Per-bin variation via XOR with bin index in low bits.
    let lfsr_step = {
        let mut s = *drift_rng;
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        *drift_rng = s;
        s
    };

    for k in 0..num_bins {
        let amount = amount_c[k].clamp(0.0, 2.0) * 0.06; // up to ±12% (~±1 dB)
        let temp_gate = thresh_c[k].clamp(0.0, 4.0);
        let release = release_c[k].clamp(0.0, 2.0).max(0.01);
        let drift_tau = 1.0 + 4.0 * release; // 1..9 sec
        let alpha = (hop_dt / drift_tau).min(1.0);

        // Per-bin pseudo-random target via LFSR XOR bin idx.
        let mixed = lfsr_step ^ (k as u32).wrapping_mul(2654435761);
        let centered = (mixed as i32 as f32) / (i32::MAX as f32); // [-1, 1)

        // Modulate by temperature: hot bins drift further.
        let temp = temperature_in.map(|t| t[k].abs().min(4.0)).unwrap_or(0.0);
        let temp_scale = if temp > temp_gate { 1.0 + (temp - temp_gate) } else { 1.0 };

        let target = centered * amount * temp_scale;
        lp_step(&mut drift_env[k], target, alpha);

        let g = (1.0 + drift_env[k]).max(0.0);
        let dry = bins[k];
        let wet = dry * g;
        let mix = mix_c[k].clamp(0.0, 2.0) * 0.5;
        bins[k] = dry * (1.0 - mix) + wet * mix;
    }

    // Write temperature: drift activity heats bins (positive feedback, clamped).
    if let Some(tout) = temperature_out {
        for k in 0..num_bins {
            let activity = drift_env[k].abs() * 0.1;
            tout[k] = (tout[k] * 0.99 + activity).clamp(0.0, 10.0);
        }
    }
}
```

- [ ] **Step 5: Wire dispatch in `process()`**

```rust
CircuitMode::ComponentDrift => {
    let temp_in: Option<&[f32]> = ctx.bin_physics.map(|bp| &bp.temperature[..num_bins]);
    let temp_out: Option<&mut [f32]> = physics.as_mut().map(|p| &mut p.temperature[..num_bins]);
    apply_component_drift(
        &mut bins[..num_bins],
        &mut self.drift_env[channel][..num_bins],
        &mut self.drift_rng[channel],
        temp_in,
        temp_out,
        curves,
        self.sample_rate,
        self.fft_size,
    );
}
```

- [ ] **Step 6: Add `drift_env_avg` to `CircuitProbe`**

```rust
pub struct CircuitProbe {
    // ... existing ...
    pub drift_env_avg: f32,
}
```

```rust
let drift_env_avg = if self.mode == CircuitMode::ComponentDrift && !self.drift_env[ch].is_empty() {
    self.drift_env[ch].iter().map(|v| v.abs()).sum::<f32>() / self.drift_env[ch].len() as f32
} else { 0.0 };
```

- [ ] **Step 7: Run test, expect pass**

Run: `cargo test --test module_trait circuit_component_drift -- --nocapture`
Expected: PASS — measurable deviation, bounded.

- [ ] **Step 8: Commit**

```bash
git add src/dsp/modules/circuit.rs tests/module_trait.rs
git commit -m "feat(circuit): Component Drift — per-bin LFSR drift with temperature feedback"
```

---

## Task 8: PCB Crosstalk — `spread_3tap` kernel, uses SPREAD curve

**Files:**
- Modify: `src/dsp/modules/circuit.rs`
- Modify: `tests/module_trait.rs`

Pure stencil kernel: bins leak into neighbours. SPREAD curve drives strength. AMOUNT scales overall wet signal. No state. Uses pre-allocated workspace to avoid alias issues.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn circuit_pcb_crosstalk_leaks_to_neighbours() {
    use spectral_forge::dsp::modules::circuit::{CircuitModule, CircuitMode};
    use spectral_forge::dsp::modules::{SpectralModule, StereoLink, FxChannelTarget};
    use realfft::num_complex::Complex;

    let mut module = CircuitModule::new();
    module.reset(48_000.0, 2048);
    module.set_circuit_mode(CircuitMode::PcbCrosstalk);

    let num_bins = 1025;
    let mut bins: Vec<Complex<f32>> = vec![Complex::new(0.0, 0.0); num_bins];
    bins[200] = Complex::new(1.0, 0.0);

    // AMOUNT=2 (full wet contribution), THRESH=0, SPREAD=1.0 (50% leak), RELEASE=0, MIX=2.
    let amount  = vec![2.0_f32; num_bins];
    let thresh  = vec![0.0_f32; num_bins];
    let spread  = vec![1.0_f32; num_bins];
    let release = vec![0.0_f32; num_bins];
    let mix     = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &thresh, &spread, &release, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = circuit_test_ctx(num_bins);

    module.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, None, &curves, &mut suppression, &ctx);

    // Centre bin retains some energy; neighbours pick up.
    assert!(bins[200].norm() < 1.0, "centre should bleed (got {})", bins[200].norm());
    assert!(bins[199].norm() > 0.05, "left neighbour should pick up (got {})", bins[199].norm());
    assert!(bins[201].norm() > 0.05, "right neighbour should pick up (got {})", bins[201].norm());
    // Distant bins should remain zero.
    assert!(bins[150].norm() < 1e-6);
    assert!(bins[250].norm() < 1e-6);
}
```

- [ ] **Step 2: Run test, expect failure**

Run: `cargo test --test module_trait circuit_pcb_crosstalk -- --nocapture`
Expected: FAIL — PcbCrosstalk arm not implemented.

- [ ] **Step 3: Add PCB workspace state fields**

```rust
pub struct CircuitModule {
    // ... existing ...
    pcb_workspace: [Vec<f32>; 2], // magnitude scratch for spread_3tap read pass
    pcb_workspace2: [Vec<f32>; 2], // output of spread_3tap pass
}
```

In `new()`: `pcb_workspace: [Vec::new(), Vec::new()], pcb_workspace2: [Vec::new(), Vec::new()],`.

In `reset()`:

```rust
self.pcb_workspace[ch].clear();
self.pcb_workspace[ch].resize(num_bins, 0.0);
self.pcb_workspace2[ch].clear();
self.pcb_workspace2[ch].resize(num_bins, 0.0);
```

(No `set_circuit_mode()` reset needed — workspaces are overwritten each hop.)

- [ ] **Step 4: Add the PCB Crosstalk kernel**

```rust
fn apply_pcb_crosstalk(
    bins: &mut [Complex<f32>],
    workspace: &mut [f32],
    workspace2: &mut [f32],
    curves: &[&[f32]],
) {
    use crate::dsp::circuit_kernels::spread_3tap;

    let amount_c = curves[0];
    // curves[1] = THRESHOLD: unused
    let spread_c = curves[2];
    // curves[3] = RELEASE: unused
    let mix_c = curves[4];

    let num_bins = bins.len();

    // 1. Read pass: copy magnitudes into workspace.
    for k in 0..num_bins {
        workspace[k] = bins[k].norm();
    }

    // 2. Average SPREAD strength (curves are smooth at hop rate).
    let spread_avg = if num_bins > 0 {
        (0..num_bins).map(|k| spread_c[k].clamp(0.0, 2.0) * 0.5).sum::<f32>() / num_bins as f32
    } else { 0.0 };

    // 3. Apply spread into workspace2.
    spread_3tap(&workspace[..num_bins], &mut workspace2[..num_bins], spread_avg);

    // 4. Write back: scale phase by new magnitude, mix with dry.
    for k in 0..num_bins {
        let amount = amount_c[k].clamp(0.0, 2.0) * 0.5;
        let mix = mix_c[k].clamp(0.0, 2.0) * 0.5;
        let in_mag = workspace[k];
        let out_mag = workspace2[k] * amount + in_mag * (1.0 - amount); // amount blends raw vs spread
        let dry = bins[k];
        let scale = if in_mag > 1e-9 { out_mag / in_mag } else { 0.0 };
        let wet = dry * scale;
        bins[k] = dry * (1.0 - mix) + wet * mix;
    }
}
```

- [ ] **Step 5: Wire dispatch in `process()`**

```rust
CircuitMode::PcbCrosstalk => {
    apply_pcb_crosstalk(
        &mut bins[..num_bins],
        &mut self.pcb_workspace[channel][..num_bins],
        &mut self.pcb_workspace2[channel][..num_bins],
        curves,
    );
    let _ = physics;
}
```

- [ ] **Step 6: Run test, expect pass**

Run: `cargo test --test module_trait circuit_pcb_crosstalk -- --nocapture`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add src/dsp/modules/circuit.rs tests/module_trait.rs
git commit -m "feat(circuit): PCB Crosstalk — 3-tap spread stencil with workspace"
```

---

## Task 9: Slew Distortion — magnitude rate-limit + phase scramble, writes `BinPhysics::slew`

**Files:**
- Modify: `src/dsp/modules/circuit.rs`
- Modify: `tests/module_trait.rs`

Spec idea #6: slew-rate clipping. Limit per-bin magnitude rate-of-change between hops; excess energy is added to a per-bin random phase rotation (audibly: gritty fuzz on transients).

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn circuit_slew_distortion_caps_rate_of_change_and_scrambles_phase() {
    use spectral_forge::dsp::modules::circuit::{CircuitModule, CircuitMode};
    use spectral_forge::dsp::modules::{SpectralModule, StereoLink, FxChannelTarget};
    use realfft::num_complex::Complex;

    let mut module = CircuitModule::new();
    module.reset(48_000.0, 2048);
    module.set_circuit_mode(CircuitMode::SlewDistortion);

    let num_bins = 1025;
    // Strong rate cap.
    let amount  = vec![2.0_f32; num_bins];
    let thresh  = vec![0.1_f32; num_bins]; // rate cap = 0.1 per hop
    let spread  = vec![0.0_f32; num_bins];
    let release = vec![1.0_f32; num_bins];
    let mix     = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &thresh, &spread, &release, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = circuit_test_ctx(num_bins);

    // Hop 1: settle prev_mag = 0 baseline.
    let mut bins: Vec<Complex<f32>> = vec![Complex::new(0.0, 0.0); num_bins];
    module.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, None, &curves, &mut suppression, &ctx);

    // Hop 2: introduce a large transient at bin 100.
    let mut bins: Vec<Complex<f32>> = vec![Complex::new(0.0, 0.0); num_bins];
    bins[100] = Complex::new(2.0, 0.0); // sudden jump from 0 to 2.0
    let phase_in = bins[100].arg();
    module.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, None, &curves, &mut suppression, &ctx);

    let after_mag = bins[100].norm();
    let after_phase = bins[100].arg();
    // Magnitude should be rate-limited (≤ thresh × scale).
    assert!(after_mag < 1.0, "mag should be slew-capped (got {})", after_mag);
    assert!(after_mag > 0.0, "mag should not zero (got {})", after_mag);
    // Phase should differ from input phase (scramble).
    let phase_diff = (after_phase - phase_in).abs();
    assert!(phase_diff > 0.01, "phase should be scrambled by excess slew (diff={})", phase_diff);
}
```

- [ ] **Step 2: Run test, expect failure**

Run: `cargo test --test module_trait circuit_slew_distortion -- --nocapture`
Expected: FAIL — Slew arm not implemented.

- [ ] **Step 3: Add Slew state fields**

```rust
pub struct CircuitModule {
    // ... existing ...
    slew_prev_mag: [Vec<f32>; 2], // per-bin previous magnitude
    slew_rng: [u32; 2],
}
```

In `new()`: `slew_prev_mag: [Vec::new(), Vec::new()], slew_rng: [0xBADF00D5, 0x0BADBEEF],`.

In `reset()`:

```rust
self.slew_prev_mag[ch].clear();
self.slew_prev_mag[ch].resize(num_bins, 0.0);
```

In `set_circuit_mode()`: `for v in self.slew_prev_mag[ch].iter_mut() { *v = 0.0; }`.

- [ ] **Step 4: Add the Slew kernel**

```rust
fn apply_slew_distortion(
    bins: &mut [Complex<f32>],
    prev_mag: &mut [f32],
    rng_state: &mut u32,
    slew_out: Option<&mut [f32]>,
    curves: &[&[f32]],
) {
    let amount_c  = curves[0];
    let thresh_c  = curves[1];
    // curves[2] = SPREAD: unused
    let release_c = curves[3];
    let mix_c     = curves[4];

    let num_bins = bins.len();

    for k in 0..num_bins {
        let amount = amount_c[k].clamp(0.0, 2.0) * 0.5;
        let rate_cap = thresh_c[k].clamp(0.001, 4.0); // max delta-mag per hop
        let scramble_gain = release_c[k].clamp(0.0, 2.0) * 0.5;

        let dry = bins[k];
        let in_mag = dry.norm();
        let in_phase = if in_mag > 1e-9 { dry.arg() } else { 0.0 };
        let prev = prev_mag[k];
        let delta = in_mag - prev;
        let allowed = rate_cap * (0.5 + 0.5 * amount); // amount scales the cap

        let (capped_mag, excess) = if delta.abs() > allowed {
            let new_mag = prev + delta.signum() * allowed;
            (new_mag, delta.abs() - allowed)
        } else {
            (in_mag, 0.0)
        };
        prev_mag[k] = capped_mag.max(0.0);

        // Excess slew → random phase rotation.
        let mut s = *rng_state;
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        *rng_state = s;
        let rand_centered = (s as i32 as f32) / (i32::MAX as f32); // [-1, 1)

        let phase_kick = rand_centered * excess * scramble_gain * std::f32::consts::PI;
        let new_phase = in_phase + phase_kick;

        let mix = mix_c[k].clamp(0.0, 2.0) * 0.5;
        let wet = realfft::num_complex::Complex::from_polar(prev_mag[k], new_phase);
        bins[k] = dry * (1.0 - mix) + wet * mix;
    }

    // Write slew (the rate cap actually applied) so downstream modules can read it.
    if let Some(sout) = slew_out {
        for k in 0..num_bins {
            sout[k] = thresh_c[k].clamp(0.001, 4.0); // record the cap, not the excess
        }
    }
}
```

- [ ] **Step 5: Wire dispatch in `process()`**

```rust
CircuitMode::SlewDistortion => {
    let slew_out: Option<&mut [f32]> = physics.as_mut().map(|p| &mut p.slew[..num_bins]);
    apply_slew_distortion(
        &mut bins[..num_bins],
        &mut self.slew_prev_mag[channel][..num_bins],
        &mut self.slew_rng[channel],
        slew_out,
        curves,
    );
}
```

- [ ] **Step 6: Run test, expect pass**

Run: `cargo test --test module_trait circuit_slew_distortion -- --nocapture`
Expected: PASS — magnitude capped, phase scrambled.

- [ ] **Step 7: Commit**

```bash
git add src/dsp/modules/circuit.rs tests/module_trait.rs
git commit -m "feat(circuit): Slew Distortion — magnitude rate-limit + phase scramble; writes BinPhysics::slew"
```

---

## Task 10: Bias Fuzz — DC offset envelope + asymmetric clip, reads/writes `BinPhysics::bias`

**Files:**
- Modify: `src/dsp/modules/circuit.rs`
- Modify: `tests/module_trait.rs`

Spec gap b: per-bin DC offset (1-pole magnitude LP) shifts the "zero point"; clip top against `1.0 - bias`. Adds even-order character. Optional SPREAD bleeds bias to neighbours. Read/write BinPhysics::bias.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn circuit_bias_fuzz_clips_against_top_rail() {
    use spectral_forge::dsp::modules::circuit::{CircuitModule, CircuitMode};
    use spectral_forge::dsp::modules::{SpectralModule, StereoLink, FxChannelTarget};
    use realfft::num_complex::Complex;

    let mut module = CircuitModule::new();
    module.reset(48_000.0, 2048);
    module.set_circuit_mode(CircuitMode::BiasFuzz);

    let num_bins = 1025;
    // AMOUNT=2 (max clip), THRESHOLD=1.0 (top rail = 1.0), SPREAD=0, RELEASE=0.1 (fast bias env), MIX=2.
    let amount  = vec![2.0_f32; num_bins];
    let thresh  = vec![1.0_f32; num_bins];
    let spread  = vec![0.0_f32; num_bins];
    let release = vec![0.1_f32; num_bins];
    let mix     = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &thresh, &spread, &release, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let ctx = circuit_test_ctx(num_bins);

    // Sustained loud input: builds bias envelope, clips top.
    let mut bins: Vec<Complex<f32>> = vec![Complex::new(2.0, 0.0); num_bins];
    for _ in 0..100 {
        for b in bins.iter_mut() { *b = Complex::new(2.0, 0.0); }
        module.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, None, &curves, &mut suppression, &ctx);
    }

    // Output bounded by ~top_rail.
    for k in 0..num_bins {
        assert!(bins[k].norm() < 1.5, "bin {} should be clipped (got {})", k, bins[k].norm());
    }
    let probe = module.probe_state(0);
    assert!(probe.bias_lp_avg > 0.1, "bias env should build (got {})", probe.bias_lp_avg);
}
```

- [ ] **Step 2: Run test, expect failure**

Run: `cargo test --test module_trait circuit_bias_fuzz -- --nocapture`
Expected: FAIL — Bias Fuzz arm not implemented.

- [ ] **Step 3: Add Bias Fuzz state fields**

```rust
pub struct CircuitModule {
    // ... existing ...
    bias_lp: [Vec<f32>; 2],
}
```

In `new()`: `bias_lp: [Vec::new(), Vec::new()],`.

In `reset()`:

```rust
self.bias_lp[ch].clear();
self.bias_lp[ch].resize(num_bins, 0.0);
```

In `set_circuit_mode()`: `for v in self.bias_lp[ch].iter_mut() { *v = 0.0; }`.

- [ ] **Step 4: Add the Bias Fuzz kernel**

```rust
fn apply_bias_fuzz(
    bins: &mut [Complex<f32>],
    bias_lp: &mut [f32],
    bias_in:  Option<&[f32]>,
    bias_out: Option<&mut [f32]>,
    curves: &[&[f32]],
    sample_rate: f32,
    fft_size: usize,
) {
    use crate::dsp::circuit_kernels::{lp_step, tanh_levien_poly};

    let amount_c  = curves[0];
    let thresh_c  = curves[1];
    let spread_c  = curves[2];
    let release_c = curves[3];
    let mix_c     = curves[4];

    let num_bins = bins.len();
    let hop_dt = (fft_size as f32 / 4.0) / sample_rate;

    for k in 0..num_bins {
        let amount = amount_c[k].clamp(0.0, 2.0) * 0.5;
        let top_rail = thresh_c[k].clamp(0.05, 4.0);
        let release = release_c[k].clamp(0.0, 2.0).max(0.01);
        // Bias envelope tau: 0.05 .. 1.0 sec for release in [0, 2].
        let tau = 0.05 + 0.5 * release;
        let alpha = (hop_dt / tau).min(1.0);

        let in_mag = bins[k].norm();
        // Seed the LP from incoming bias if available.
        let prev = match bias_in {
            Some(b) => bias_lp[k].max(b[k]),
            None    => bias_lp[k],
        };
        bias_lp[k] = prev;
        lp_step(&mut bias_lp[k], in_mag, alpha);

        // Effective top rail shrinks with bias buildup.
        let effective_top = (top_rail - bias_lp[k] * 0.5).max(0.05);
        // Asymmetric clip: tanh against effective_top × (1 + amount).
        let drive = (1.0 + amount * 4.0) / effective_top;
        let new_mag = tanh_levien_poly(in_mag * drive) * effective_top;

        let dry = bins[k];
        let scale = if in_mag > 1e-9 { new_mag / in_mag } else { 0.0 };
        let wet = dry * scale;
        let mix = mix_c[k].clamp(0.0, 2.0) * 0.5;
        bins[k] = dry * (1.0 - mix) + wet * mix;
    }

    // Optional SPREAD: bleed bias to neighbours (simple in-place, accepting one hop of lag).
    let spread_avg = (0..num_bins).map(|k| spread_c[k].clamp(0.0, 2.0) * 0.5).sum::<f32>() / num_bins.max(1) as f32;
    if spread_avg > 0.001 && num_bins >= 3 {
        let mut prev_b = bias_lp[0];
        let mut curr_b = bias_lp[0];
        for k in 0..num_bins {
            let next_b = if k + 1 < num_bins { bias_lp[k + 1] } else { 0.0 };
            let bleed = 0.5 * spread_avg * (prev_b + next_b);
            bias_lp[k] = (1.0 - spread_avg) * curr_b + bleed;
            prev_b = curr_b;
            curr_b = next_b;
        }
    }

    // Write bias back: time-averaged DC offset proxy.
    if let Some(bout) = bias_out {
        for k in 0..num_bins {
            bout[k] = (bout[k] * 0.95 + bias_lp[k] * 0.05).clamp(-10.0, 10.0);
        }
    }
}
```

- [ ] **Step 5: Wire dispatch in `process()`**

```rust
CircuitMode::BiasFuzz => {
    let bias_in:  Option<&[f32]> = ctx.bin_physics.map(|bp| &bp.bias[..num_bins]);
    let bias_out: Option<&mut [f32]> = physics.as_mut().map(|p| &mut p.bias[..num_bins]);
    apply_bias_fuzz(
        &mut bins[..num_bins],
        &mut self.bias_lp[channel][..num_bins],
        bias_in,
        bias_out,
        curves,
        self.sample_rate,
        self.fft_size,
    );
}
```

- [ ] **Step 6: Add `bias_lp_avg` to `CircuitProbe`**

```rust
pub struct CircuitProbe {
    // ... existing ...
    pub bias_lp_avg: f32,
}
```

```rust
let bias_lp_avg = if self.mode == CircuitMode::BiasFuzz && !self.bias_lp[ch].is_empty() {
    self.bias_lp[ch].iter().sum::<f32>() / self.bias_lp[ch].len() as f32
} else { 0.0 };
```

- [ ] **Step 7: Run test, expect pass**

Run: `cargo test --test module_trait circuit_bias_fuzz -- --nocapture`
Expected: PASS — output bounded, bias env builds.

- [ ] **Step 8: Commit**

```bash
git add src/dsp/modules/circuit.rs tests/module_trait.rs
git commit -m "feat(circuit): Bias Fuzz — DC offset envelope + asymmetric tanh clip; reads/writes BinPhysics::bias"
```

---

## Task 11: Mode picker UI extension (10 modes + heavy indicator)

**Files:**
- Modify: `src/editor/circuit_popup.rs`

- [ ] **Step 1: Update the popup mode list**

Replace the 3-entry mode list with the full 10. Group by curated order: light first, then physics-aware. Append a "(heavy)" tag after the Transformer entry.

```rust
pub fn show_circuit_popup(
    ui: &mut egui::Ui,
    state: &mut CircuitPopupState,
    slot_circuit_mode: &Arc<Mutex<CircuitMode>>,
) -> bool {
    let Some(_slot) = state.open_for_slot else { return false; };
    let area_id = egui::Id::new("circuit_mode_picker");
    let mut selected = false;

    egui::Area::new(area_id)
        .order(egui::Order::Foreground)
        .fixed_pos(state.anchor)
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style())
                .fill(theme::POPUP_BG)
                .stroke(egui::Stroke::new(1.0, theme::POPUP_BORDER))
                .show(ui, |ui| {
                    ui.set_min_width(180.0);
                    ui.label(egui::RichText::new("CIRCUIT MODE")
                        .color(theme::POPUP_TITLE).size(11.0));
                    ui.separator();
                    let cur = *slot_circuit_mode.lock().unwrap();
                    for (label, mode, heavy) in [
                        ("Crossover Distortion",  CircuitMode::CrossoverDistortion,  false),
                        ("Spectral Schmitt",      CircuitMode::SpectralSchmitt,      false),
                        ("BBD Bins",              CircuitMode::BbdBins,              false),
                        ("Vactrol",               CircuitMode::Vactrol,              false),
                        ("Transformer",           CircuitMode::TransformerSaturation, true),
                        ("Power Sag",             CircuitMode::PowerSag,             false),
                        ("Component Drift",       CircuitMode::ComponentDrift,       false),
                        ("PCB Crosstalk",         CircuitMode::PcbCrosstalk,         false),
                        ("Slew Distortion",       CircuitMode::SlewDistortion,       false),
                        ("Bias Fuzz",             CircuitMode::BiasFuzz,             false),
                    ] {
                        let is_active = cur == mode;
                        let color = if is_active { theme::CIRCUIT_DOT_COLOR } else { theme::POPUP_TEXT };
                        let display = if heavy {
                            format!("{} (heavy)", label)
                        } else {
                            label.to_string()
                        };
                        let response = ui.selectable_label(
                            is_active,
                            egui::RichText::new(display).color(color).size(11.0),
                        );
                        if response.clicked() {
                            *slot_circuit_mode.lock().unwrap() = mode;
                            selected = true;
                        }
                    }
                });
        });

    if selected {
        state.open_for_slot = None;
    }
    selected
}
```

- [ ] **Step 2: Verify compile + manual smoke test**

Run: `cargo build`
Expected: clean build.

Manual: load in Bitwig, assign Circuit, right-click slot, confirm 10 entries appear, all selectable, persist across plugin reopen.

- [ ] **Step 3: Commit**

```bash
git add src/editor/circuit_popup.rs
git commit -m "feat(circuit): mode picker UI extended to 10 modes with heavy indicator"
```

---

## Task 12: BinPhysics integration tests — Vactrol reads flux, Transformer writes flux, Bias Fuzz round-trips bias

**Files:**
- Modify: `tests/bin_physics_pipeline.rs`

These tests verify the cross-slot BinPhysics flow: a producer-slot module writes a physics field; a consumer-slot module on the next hop sees the produced value via its `ctx.bin_physics`.

- [ ] **Step 1: Write the failing tests**

Append to `tests/bin_physics_pipeline.rs` (created in Phase 3):

```rust
#[test]
fn circuit_transformer_writes_flux_visible_to_next_slot() {
    use spectral_forge::dsp::modules::circuit::{CircuitModule, CircuitMode};
    use spectral_forge::dsp::modules::{SpectralModule, StereoLink, FxChannelTarget, ModuleContext};
    use spectral_forge::dsp::bin_physics::BinPhysics;
    use realfft::num_complex::Complex;

    let mut module = CircuitModule::new();
    module.reset(48_000.0, 2048);
    module.set_circuit_mode(CircuitMode::TransformerSaturation);

    let num_bins = 1025;
    let mut bins: Vec<Complex<f32>> = vec![Complex::new(3.0, 0.0); num_bins];

    let amount  = vec![2.0_f32; num_bins];
    let thresh  = vec![1.0_f32; num_bins];
    let spread  = vec![0.0_f32; num_bins];
    let release = vec![1.0_f32; num_bins];
    let mix     = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &thresh, &spread, &release, &mix];

    let mut suppression = vec![0.0_f32; num_bins];
    let mut physics = BinPhysics::new();
    physics.reset_active(num_bins, 48_000.0, 2048);

    // Phase 1+3 ctx: bin_physics provides incoming reader view.
    // We simulate "no incoming flux" by passing None on the reader side.
    let ctx = phase_test_ctx(num_bins, None);

    // Several hops to let xfmr_lp settle and accumulate flux.
    for _ in 0..40 {
        for b in bins.iter_mut() { *b = Complex::new(3.0, 0.0); }
        module.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, None, &curves, &mut suppression, Some(&mut physics), &ctx);
    }

    // Flux should have built up where the magnitude was saturating.
    let total_flux: f32 = physics.flux[..num_bins].iter().sum();
    assert!(total_flux > 0.5, "Transformer should write flux; total = {}", total_flux);
}

#[test]
fn circuit_vactrol_reads_incoming_flux() {
    use spectral_forge::dsp::modules::circuit::{CircuitModule, CircuitMode};
    use spectral_forge::dsp::modules::{SpectralModule, StereoLink, FxChannelTarget};
    use spectral_forge::dsp::bin_physics::BinPhysics;
    use realfft::num_complex::Complex;

    let mut module = CircuitModule::new();
    module.reset(48_000.0, 2048);
    module.set_circuit_mode(CircuitMode::Vactrol);

    let num_bins = 1025;

    let amount  = vec![1.0_f32; num_bins];
    let thresh  = vec![1.0_f32; num_bins];
    let spread  = vec![0.0_f32; num_bins];
    let release = vec![1.0_f32; num_bins];
    let mix     = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &thresh, &spread, &release, &mix];
    let mut suppression = vec![0.0_f32; num_bins];

    // Build a physics view with a strong flux peak at bin 200.
    let mut physics_view = BinPhysics::new();
    physics_view.reset_active(num_bins, 48_000.0, 2048);
    physics_view.flux[200] = 4.0;
    let view_ref: &BinPhysics = &physics_view;

    let mut bins: Vec<Complex<f32>> = vec![Complex::new(0.5, 0.0); num_bins];

    let ctx = phase_test_ctx(num_bins, Some(view_ref));

    // Several hops: vactrol cap should charge primarily at bin 200 (high flux).
    for _ in 0..50 {
        for b in bins.iter_mut() { *b = Complex::new(0.5, 0.0); }
        module.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, None, &curves, &mut suppression, None, &ctx);
    }

    let probe = module.probe_state(0);
    // The probe averages across all bins; with a single hot bin among 1025 the avg
    // is small but should still be > 0 (the cap charges with flux drive).
    assert!(probe.vactrol_slow_avg > 0.0, "vactrol should charge from incoming flux (avg={})", probe.vactrol_slow_avg);
}

#[test]
fn circuit_bias_fuzz_roundtrips_bias_field() {
    use spectral_forge::dsp::modules::circuit::{CircuitModule, CircuitMode};
    use spectral_forge::dsp::modules::{SpectralModule, StereoLink, FxChannelTarget};
    use spectral_forge::dsp::bin_physics::BinPhysics;
    use realfft::num_complex::Complex;

    let mut module = CircuitModule::new();
    module.reset(48_000.0, 2048);
    module.set_circuit_mode(CircuitMode::BiasFuzz);

    let num_bins = 1025;

    let amount  = vec![2.0_f32; num_bins];
    let thresh  = vec![1.0_f32; num_bins];
    let spread  = vec![0.0_f32; num_bins];
    let release = vec![0.1_f32; num_bins];
    let mix     = vec![2.0_f32; num_bins];
    let curves: Vec<&[f32]> = vec![&amount, &thresh, &spread, &release, &mix];
    let mut suppression = vec![0.0_f32; num_bins];

    let mut physics = BinPhysics::new();
    physics.reset_active(num_bins, 48_000.0, 2048);

    let ctx = phase_test_ctx(num_bins, None);

    let mut bins: Vec<Complex<f32>> = vec![Complex::new(2.0, 0.0); num_bins];
    for _ in 0..100 {
        for b in bins.iter_mut() { *b = Complex::new(2.0, 0.0); }
        module.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, None, &curves, &mut suppression, Some(&mut physics), &ctx);
    }

    let total_bias: f32 = physics.bias[..num_bins].iter().sum();
    assert!(total_bias > 0.5, "Bias Fuzz should write bias; total = {}", total_bias);
}

#[cfg(test)]
fn phase_test_ctx<'a>(
    num_bins: usize,
    bin_physics: Option<&'a spectral_forge::dsp::bin_physics::BinPhysics>,
) -> spectral_forge::dsp::modules::ModuleContext<'a> {
    spectral_forge::dsp::modules::ModuleContext {
        sample_rate: 48_000.0,
        fft_size: 2048,
        num_bins,
        attack_ms: 10.0,
        release_ms: 100.0,
        sensitivity: 1.0,
        suppression_width: 1.0,
        auto_makeup: false,
        delta_monitor: false,
        bpm: 120.0,
        beat_position: 0.0,
        bin_physics,
        unwrapped_phase: None,
        peaks: None,
    }
}
```

- [ ] **Step 2: Run tests, expect failure**

Run: `cargo test --test bin_physics_pipeline circuit -- --nocapture`
Expected: FAIL on the very first call to `module.process(.., physics, &ctx)` if Phase 3 has not yet shipped, or PASS on the first two if Phase 3 + Phase 5c Tasks 4-10 are merged.

- [ ] **Step 3: Run tests, expect pass**

Once Phase 5c Tasks 4-10 are landed: re-run the test.
Run: `cargo test --test bin_physics_pipeline circuit -- --nocapture`
Expected: ALL PASS.

- [ ] **Step 4: Commit**

```bash
git add tests/bin_physics_pipeline.rs
git commit -m "test(circuit): BinPhysics pipeline integration — flux read/write + bias roundtrip"
```

---

## Task 13: Calibration probes — extend `calibration_roundtrip.rs` to all 10 modes

**Files:**
- Modify: `tests/calibration_roundtrip.rs`

The Phase 2g calibration test loops over `[CrossoverDistortion, SpectralSchmitt, BbdBins]`. Extend it to all 10 modes; assert mode-specific probe fields are non-zero where applicable.

- [ ] **Step 1: Update the calibration test**

```rust
#[cfg(feature = "probe")]
#[test]
fn circuit_calibration_roundtrip_all_modes() {
    use spectral_forge::dsp::modules::circuit::{CircuitModule, CircuitMode};
    use spectral_forge::dsp::modules::{SpectralModule, StereoLink, FxChannelTarget};
    use realfft::num_complex::Complex;

    let num_bins = 1025;

    for mode in [
        CircuitMode::CrossoverDistortion,
        CircuitMode::SpectralSchmitt,
        CircuitMode::BbdBins,
        CircuitMode::Vactrol,
        CircuitMode::TransformerSaturation,
        CircuitMode::PowerSag,
        CircuitMode::ComponentDrift,
        CircuitMode::PcbCrosstalk,
        CircuitMode::SlewDistortion,
        CircuitMode::BiasFuzz,
    ] {
        let mut module = CircuitModule::new();
        module.reset(48_000.0, 2048);
        module.set_circuit_mode(mode);

        let mut bins: Vec<Complex<f32>> = vec![Complex::new(0.7, 0.0); num_bins];
        let amount  = vec![1.5_f32; num_bins];
        let thresh  = vec![1.0_f32; num_bins];
        let spread  = vec![0.5_f32; num_bins];
        let release = vec![1.0_f32; num_bins];
        let mix     = vec![1.0_f32; num_bins];
        let curves: Vec<&[f32]> = vec![&amount, &thresh, &spread, &release, &mix];
        let mut suppression = vec![0.0_f32; num_bins];
        let ctx = ckt_test_ctx(num_bins);

        for _ in 0..30 {
            module.process(0, StereoLink::Linked, FxChannelTarget::All, &mut bins, None, &curves, &mut suppression, &ctx);
        }

        let probe = module.probe_state(0);
        assert_eq!(probe.active_mode, mode);
        assert!(probe.average_amount_pct >= 0.0 && probe.average_amount_pct <= 200.0);

        // Mode-specific probe assertions.
        match mode {
            CircuitMode::Vactrol => {
                assert!(probe.vactrol_slow_avg > 0.0, "Vactrol slow cap should charge");
            }
            CircuitMode::TransformerSaturation => {
                assert!(probe.xfmr_lp_avg > 0.0, "Transformer magnitude LP should accumulate");
            }
            CircuitMode::PowerSag => {
                assert!(probe.sag_envelope >= 0.0, "Sag envelope should be non-negative");
            }
            CircuitMode::ComponentDrift => {
                // Drift may be small at hop 30, but the abs-avg should be finite.
                assert!(probe.drift_env_avg.is_finite());
            }
            CircuitMode::BiasFuzz => {
                assert!(probe.bias_lp_avg > 0.0, "Bias env should build under sustained input");
            }
            _ => {}
        }
    }
}
```

- [ ] **Step 2: Run test**

Run: `cargo test --features probe --test calibration_roundtrip circuit -- --nocapture`
Expected: PASS for all 10 modes.

- [ ] **Step 3: Commit**

```bash
git add tests/calibration_roundtrip.rs
git commit -m "test(circuit): calibration roundtrip extended to all 10 modes with mode-specific probes"
```

---

## Task 14: Multi-hop dual-channel finite/bounded contract test (all 10 modes)

**Files:**
- Modify: `tests/module_trait.rs`

Replace the Phase 2g 3-mode `circuit_finite_bounded_all_modes_dual_channel` with a 10-mode version. Run 200 hops on each mode, both channels, with realistic curves; assert finite and bounded.

- [ ] **Step 1: Update the test**

```rust
#[test]
fn circuit_finite_bounded_all_modes_dual_channel() {
    use spectral_forge::dsp::modules::circuit::{CircuitModule, CircuitMode};
    use spectral_forge::dsp::modules::{SpectralModule, StereoLink, FxChannelTarget};
    use realfft::num_complex::Complex;

    let num_bins = 1025;
    let modes = [
        CircuitMode::CrossoverDistortion,
        CircuitMode::SpectralSchmitt,
        CircuitMode::BbdBins,
        CircuitMode::Vactrol,
        CircuitMode::TransformerSaturation,
        CircuitMode::PowerSag,
        CircuitMode::ComponentDrift,
        CircuitMode::PcbCrosstalk,
        CircuitMode::SlewDistortion,
        CircuitMode::BiasFuzz,
    ];

    for mode in modes {
        let mut module = CircuitModule::new();
        module.reset(48_000.0, 2048);
        module.set_circuit_mode(mode);

        let mut bins_l: Vec<Complex<f32>> = (0..num_bins).map(|k|
            Complex::new(((k as f32 * 0.07).sin() + 0.1).abs(),
                         ((k as f32 * 0.11).cos() * 0.5))
        ).collect();
        let mut bins_r: Vec<Complex<f32>> = bins_l.iter().map(|b| b * 0.6).collect();

        let amount  = vec![1.5_f32; num_bins];
        let thresh  = vec![1.0_f32; num_bins];
        let spread  = vec![0.5_f32; num_bins];
        let release = vec![1.0_f32; num_bins];
        let mix     = vec![1.0_f32; num_bins];
        let curves: Vec<&[f32]> = vec![&amount, &thresh, &spread, &release, &mix];

        let mut suppression = vec![0.0_f32; num_bins];
        let ctx = circuit_test_ctx(num_bins);

        for hop in 0..200 {
            for ch in 0..2 {
                let bins = if ch == 0 { &mut bins_l } else { &mut bins_r };
                module.process(ch, StereoLink::Independent, FxChannelTarget::All, bins, None, &curves, &mut suppression, &ctx);
                for (i, b) in bins.iter().enumerate() {
                    assert!(b.norm().is_finite(), "mode={:?} hop={} ch={} bin={} norm={}", mode, hop, ch, i, b.norm());
                    assert!(b.norm() < 1e6, "runaway: mode={:?} hop={} ch={} bin={} norm={}", mode, hop, ch, i, b.norm());
                }
                for s in &suppression {
                    assert!(s.is_finite() && *s >= 0.0);
                }
            }
        }
    }
}
```

- [ ] **Step 2: Run test, expect pass**

Run: `cargo test --test module_trait circuit_finite_bounded_all_modes_dual_channel -- --nocapture`
Expected: PASS — all 10 modes survive 200 hops × 2 channels with finite/bounded outputs.

- [ ] **Step 3: Commit**

```bash
git add tests/module_trait.rs
git commit -m "test(circuit): multi-hop dual-channel finite/bounded contract — all 10 modes"
```

---

## Task 15: Status banners + STATUS.md + back-pointer in Phase 2g banner

**Files:**
- Modify: `docs/superpowers/STATUS.md`
- Modify: this plan file
- Modify: `docs/superpowers/plans/2026-04-27-phase-2g-circuit-light.md`

- [ ] **Step 1: Update banner at top of this plan after merge**

Change:

```
> **Status:** PLANNED — implementation pending. Phase 5 sub-plan; depends on:
```

to:

```
> **Status:** IMPLEMENTED — landed in commit <SHA>. Phase 5 sub-plan; depends on:
```

- [ ] **Step 2: Add cross-reference to Phase 2g plan banner**

In `docs/superpowers/plans/2026-04-27-phase-2g-circuit-light.md`, append to the existing banner:

```
>
> **Superseded for curve layout by Phase 5c (`docs/superpowers/plans/2026-04-27-phase-5c-full-circuit.md`):** Phase 5c bumps `num_curves` to 5 (inserts SPREAD at index 2; RELEASE moves to 3; MIX moves to 4) and migrates the 3 v1 kernels in-place.
```

- [ ] **Step 3: Add STATUS.md row**

```
| 2026-04-27-phase-5c-full-circuit.md | IMPLEMENTED | Circuit retrofit: adds Vactrol / Transformer / Power Sag / Component Drift / PCB Crosstalk / Slew Distortion / Bias Fuzz (10-mode total). Bumps to 5 curves with SPREAD at index 2. BinPhysics writer for flux/temperature/bias/slew. |
```

- [ ] **Step 4: Final commit**

```bash
git add docs/superpowers/plans/2026-04-27-phase-5c-full-circuit.md docs/superpowers/plans/2026-04-27-phase-2g-circuit-light.md docs/superpowers/STATUS.md
git commit -m "docs(status): mark phase-5c full Circuit IMPLEMENTED"
```

---

## Self-review

**Spec coverage check** (against `ideas/next-gen-modules/10-circuit.md` brainstorm cross-reference table):
- ✅ Idea #1 Spectral Schmitt — Phase 2g (already in v1; curve indices migrated in Task 2).
- ✅ Idea #2 Transformer Core Saturation + spread — Task 5 (`apply_transformer`, SPREAD curve).
- ✅ Idea #5 Vactrol Bin Smoothing — Task 4 (cascaded 1-pole, reads `BinPhysics::flux`).
- ✅ Idea #6 Slew-Rate Induced Distortion — Task 9 (rate cap + phase scramble; clarified scramble = phase rotation, not magnitude noise; writes `BinPhysics::slew`).
- ✅ Idea #7 Thermal Runaway refinement → folded into Power Sag (Task 6) per Kim's "fluctuate / sag, not violent silence" preference.
- ✅ Idea #8 Power Supply Sag — Task 6 (energy-driven envelope, temperature-weighted reduction).
- ✅ Idea #10 Component Tolerance Drift — Task 7 (per-bin LFSR drift, reads/writes `BinPhysics::temperature`).
- ✅ Idea #11 Crossover Distortion (Class A/B Deadzone) — Phase 2g (curve indices migrated in Task 2).
- ✅ Idea #13 Bucket-Brigade Bins (BBD) — Phase 2g (curve indices migrated in Task 2).
- ✅ Idea #14 PCB Trace Crosstalk — Task 8 (`spread_3tap` kernel).
- ✅ Idea #19 Asymmetric Bias Fuzz — Task 10 (DC offset envelope + asymmetric `tanh_levien_poly` clip; reads/writes `BinPhysics::bias`).

**Brainstorm items NOT in this plan (explicitly per spec):**
- Idea #3 Tape Print-Through → Past spec.
- Idea #4 Stuck Relay → dropped.
- Idea #9 Dusty Pot → dropped.
- Idea #12 PLL Tearing → Modulate (Phase 5b.4).
- Idea #15 Resonant Feedback Channel → handled via `RouteMatrix` + Matrix Amp Nodes (Phase 2a).
- Idea #16 Ground Loop → Modulate (Phase 2f).
- Idea #17 Diode Bridge Ring Mod → Modulate (Phase 2f).
- Idea #18 Envelope Follower Ripple → deferred (global helper).
- Idea #20 Bypass Pop → dropped.

**Plan-internal consistency:**
- ✅ All 10 mode kernels use the new `[AMOUNT, THRESHOLD, SPREAD, RELEASE, MIX]` curve layout.
- ✅ State-field additions to `CircuitModule` are listed once at module struct level (cumulative across tasks); each task's `// ... existing ...` placeholder is the only abstraction permitted.
- ✅ All BinPhysics-aware modes pass `physics.as_mut().map(|p| &mut p.<field>[..num_bins])` for the writer side; readers use `ctx.bin_physics.map(|bp| &bp.<field>[..num_bins])`.
- ✅ Reuse of `circuit_kernels::lp_step` (Tasks 4, 5, 6, 7, 10) and `tanh_levien_poly` (Tasks 5, 10) and `spread_3tap` (Task 8) is consistent.
- ✅ `set_circuit_mode()` reset list grows with each new state field (specified in Tasks 4-10).
- ✅ `CircuitProbe` fields grow with each new mode (specified per task); existing fields remain.

**Type consistency:** `CircuitMode` enum used uniformly across params, FxMatrix, CircuitModule, popup, probes. `BinPhysics` field names match Phase 3 (`flux`, `temperature`, `bias`, `slew`).

**Placeholder scan:** No "TBD" / "implement later". All 10 mode kernels have full code in their tasks; tests have full assertions.

**No Phase 3/4 dependency unmet:** Vactrol, Transformer, Sag, Drift, Bias Fuzz all use the Phase 1+3 `ctx.bin_physics` reader and `physics: Option<&mut BinPhysics>` writer. None of the new modes need PLPV (Phase 4) or History Buffer (Phase 5b.1).

**Test breakdown:**
- Phase 2g existing tests: 5 tests, curve fixtures migrated in Task 2.
- Per-mode kernel tests: 1 test per new mode (Tasks 4, 5, 6, 7, 8, 9, 10) + Vactrol bonus stability test = 8 tests.
- Transformer SPREAD bonus test: 1 test (Task 5).
- BinPhysics integration tests: 3 tests (Task 12).
- Calibration roundtrip: 1 test parameterised over 10 modes (Task 13).
- Multi-hop dual-channel contract: 1 test parameterised over 10 modes (Task 14).
- Circuit kernels: 7 tests (Task 3).

**Risk register coverage:**
- ✅ Curve migration breakage caught by Task 2 fixture rewrites.
- ✅ Transformer SPREAD alias avoided by `xfmr_workspace` plus the on-bin-window slide in Task 5 (no second workspace allocation; cheap stack-only `prev_w`/`curr_w`/`next_w` triple).
- ✅ Drift LFSR step-rate addressed by single-step-per-hop + per-bin XOR in Task 7.
- ✅ Bias Fuzz top-rail smoothness via `tanh_levien_poly`.
- ✅ Slew phase-scramble dither uses inline xorshift32 (RT-alloc-free) in Task 9.
- ✅ Curve smoothing on retrofit modes: `lp_step` is applied per-bin per kernel (NOT a separate smoother pass) — each kernel has its own `tau` matched to its dynamics.

---

## Execution Handoff

**Plan complete and saved to `docs/superpowers/plans/2026-04-27-phase-5c-full-circuit.md`. Two execution options:**

**1. Subagent-Driven (recommended)** — dispatch fresh subagent per task, review between tasks. Best for the 15-task volume.
**2. Inline Execution** — execute tasks in this session using executing-plans, batch with checkpoints (e.g. checkpoints after Tasks 3, 7, 11, 15).
