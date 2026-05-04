> **Status (2026-05-04): DRAFT — pending implementation plan + landing PR.** First in the per-module UX overhaul series for the new (Phase 2/5/6) modules. Source of truth for Past's UI surface; the shipped Phase 5b2 DSP stays intact except for the small additions in §6. Consumes the `CurveLayout` infrastructure declared in [`2026-04-23-ui-parameter-spec-design.md` §8](2026-04-23-ui-parameter-spec-design.md#8-per-mode-curvelayout--active-curves-label-overrides-help-text).

# Past Module — UX Design Spec

## Purpose

The Phase 5b2 plan landed Past's DSP and probe surface but left every mode showing the legacy 5-curve strip (AMOUNT, TIME, THRESHOLD, SPREAD, MIX) regardless of whether the DSP read each curve per-bin or just averaged it across the spectrum. Two modes (Reverse, Stretch) silently averaged TIME and SPREAD curves to derive a single scalar — a control surface that's a curve in shape but a slider in semantics, leaving the user drawing a graph that gets collapsed into one number.

This spec re-architects Past's UI so that:

1. Each mode shows only the curves the DSP reads per-bin (driven by `CurveLayout::active`).
2. Mode-specific scalar controls (Stretch rate, Reverse window length, DecaySorter low_k floor) live as proper sliders, not as averaged-curve hacks.
3. A module-wide soft-clip toggle prevents Convolution's multiplicative output from feeding huge magnitudes to downstream slots.
4. A help-box explains the module's purpose and the active curve's meaning so users aren't lost when staring at a graph called "Smear" with no context.

This is one of six per-module UX specs (Past, Geometry, Circuit, Life, Kinetics, Harmony) the dev branch needs. Past is the template; the others mimic the same structure once it's settled.

---

## 1. Per-mode CurveLayout

The Past `ModuleSpec` gains `active_layout: Some(past::active_layout)`. The `active_layout(mode: u8) -> CurveLayout` function in `src/dsp/modules/past.rs` matches on the current `PastMode` value and returns one of the five layouts below.

### 1.1 Granular

Active: **AMOUNT · Age · THRESHOLD · Smear · MIX** (5 of 5).

| Curve idx | Visible label | What it controls |
|---|---|---|
| 0 | AMOUNT | Per-bin replacement strength: 0 = current only, 1 = historical only. Adds with upstream BinPhysics `crystallization` if any upstream slot writes it; clamp at 1.0 after add. |
| 1 | **Age** (override of TIME) | Per-bin lookback into history. 0 = current frame, 1 = oldest available frame. |
| 2 | THRESHOLD | Per-bin gate on the current-frame magnitude. Bins below the gate pass through unchanged (no historical replacement). |
| 3 | **Smear** (override of SPREAD) | Per-bin toggle (>0.5 enables) for 3-bin frequency smear of the historical read. Smooths bin-leakage on narrow partials. |
| 4 | MIX | Per-bin wet/dry. |

### 1.2 DecaySorter

Active: **AMOUNT · THRESHOLD · MIX** (3 of 5).

| Curve idx | Visible label | What it controls |
|---|---|---|
| 0 | AMOUNT | Per-bin gain applied to the rearranged output. |
| 2 | THRESHOLD | Per-bin floor below which bins are excluded from the sort entirely. |
| 4 | MIX | Per-bin wet/dry of sorted output vs. original. |

TIME (1) and SPREAD (3) hidden — DSP never reads them. SortKey (Decay/Stability/Area) stays in the popup since it's a coarse mode-shape choice, not a real-time tweak. The scalar Floor slider (§2) replaces the hardcoded `low_k = 10`.

### 1.3 Convolution

Active: **AMOUNT · Delay · THRESHOLD · MIX** (4 of 5).

| Curve idx | Visible label | What it controls |
|---|---|---|
| 0 | AMOUNT | Per-bin convolution strength. Multiplied by upstream BinPhysics `flux` if present (gate by recent change). |
| 1 | **Delay** (override of TIME) | Per-bin delay into history. Each bin can read at a different age — low bins lag, high bins recent, etc. |
| 2 | THRESHOLD | Per-bin gate on the current-frame magnitude. |
| 4 | MIX | Per-bin wet/dry. |

SPREAD (3) hidden.

### 1.4 Reverse

Active: **AMOUNT · THRESHOLD · MIX** (3 of 5).

| Curve idx | Visible label | What it controls |
|---|---|---|
| 0 | AMOUNT | Per-bin keep amount during the reverse read. |
| 2 | THRESHOLD | Per-bin gate. |
| 4 | MIX | Per-bin wet/dry. |

TIME (1) hidden — replaced by the **Window** scalar (§2). SPREAD (3) hidden.

### 1.5 Stretch

Active: **AMOUNT · MIX** (2 of 5).

| Curve idx | Visible label | What it controls |
|---|---|---|
| 0 | AMOUNT | Per-bin keep during the stretched read. |
| 4 | MIX | Per-bin wet/dry. |

TIME (1) hidden — replaced by the **Rate** scalar. SPREAD (3) hidden — replaced by the **Dither** scalar. THRESHOLD (2) hidden (DSP doesn't gate Stretch today).

---

## 2. Mode-specific scalar controls

Rendered in the slot's `panel_widget` strip (analogous to Dynamics' Atk/Rel/Sens/Width row). Visible only when the relevant mode is active.

| Mode | Control | Range / unit | Default | DSP wiring |
|---|---|---|---|---|
| DecaySorter | **Floor** | 20 Hz – 2 kHz, log | 230 Hz (= bin 10 at fft 2048 / 48 kHz) | Replaces hardcoded `low_k = 10`. Converted to bin index at `Pipeline::process` time using current `sample_rate` and `fft_size`; the kernel reads it as `low_k`. |
| Reverse | **Window** | 0.05 s – `total_history_seconds`, linear | 1.0 s, clamped to history length | Replaces averaging the TIME curve. Converted to frame count `window` at `Pipeline::process` time; passed through `apply_reverse` as the existing `window` argument. |
| Stretch | **Rate** | 0.25× – 4×, log around 1.0× | 1.0× | Replaces averaging the TIME curve. Drives `stretch_read_phase` advance directly (no curve averaging). |
| Stretch | **Dither** | 0% – 100%, linear | 0% | Replaces averaging the SPREAD curve. Drives the xorshift32 dither applied to the read phase to mask phase glitches at non-integer rates. |

Each control is a new automatable `FloatParam` per slot. Naming convention:

```
s{s}_past_floor_hz             FloatRange::Skewed { min: 20.0, max: 2000.0, factor: skew(-2.0) }
s{s}_past_reverse_window_s     FloatRange::Linear { min: 0.05, max: 30.0 }
s{s}_past_stretch_rate         FloatRange::Skewed { min: 0.25, max: 4.0, factor: skew(0.0) }  // log-around-1.0
s{s}_past_stretch_dither       FloatRange::Linear { min: 0.0,  max: 1.0  }
```

Smoothing: `Logarithmic(50.0)` for Floor and Rate (zipper-free continuous tuning across log space, 50 ms time-constant matches existing per-slot smoothers), `Linear(50.0)` for Window (rare adjust, matches existing Linear smoothers), `Linear(20.0)` for Dither (faster — Dither is a quality control the user often rides into the mix).

The 30-second upper bound on `Window` is the spec ceiling; the slider clamps at runtime to `min(30.0, history.capacity_frames() × hop_seconds)` so the user can never request a window longer than the actual buffer. The clamp re-runs whenever fft_size or sample_rate changes. If the user-set value exceeds the runtime ceiling (e.g. after switching to a smaller FFT), it's clamped silently — no notification — and the displayed value updates next frame.

---

## 3. Module-wide controls

| Control | Default | DSP wiring |
|---|---|---|
| **Soft Clip** | ON | Module-wide `BoolParam` per slot: `s{s}_past_soft_clip`. When ON, after the active mode's kernel writes `bins[k]`, applies a per-bin radial soft-clip toward magnitude 4.0 (≈ +12 dBFS): `bins[k] *= K / (K + |bins[k]|)` with `K = 4.0`. When OFF, bins pass through unchanged — for users who want the raw multiplicative output of Convolution piped into a downstream limiter or saturator. |

The toggle is rendered in `panel_widget` to the left of the mode-specific scalars (so it stays in the same X position regardless of mode). Implementation lives in `apply_soft_clip` next to the mode kernels in `past.rs`; called once at the end of `process()` after the matched mode has run.

The `K = 4.0` threshold is fixed (not user-adjustable). Rationale: 4.0 ≈ +12 dBFS is well above the magnitudes of typical audio signals after Hann² OLA normalisation (peak unit-sine bins land around 0.33), so the soft-clip is inert until something explodes in the multiplicative kernels. A user-adjustable threshold would invite confusion (it's not a musical control, it's a safety net). If a future user testing pass shows the threshold should change, edit the constant.

---

## 4. Help-box content

Help text for the module is supplied via `CurveLayout::help_for(curve_idx)` and `CurveLayout::mode_overview`. All strings are `&'static str` declared inline in `past.rs` (no allocation, no per-frame formatting).

### 4.1 Module overview (when Past slot is selected, no curve in focus)

Used as fallback when `mode_overview` is `None`; Past sets it `None` and relies on this single string for all five modes. (Per-mode overviews can be added later if user testing shows mode-specific guidance is needed.)

> **Past** — Read-only access to a rolling buffer of recent spectral history.
> Pick a mode (right-click the slot) to choose how the buffer is replayed:
> Granular freezes selected bins by age, DecaySorter rearranges bins by how
> long they ring, Convolution blends current with delayed self, Reverse plays
> the buffer backward, Stretch plays it at variable speed.
>
> Per-channel history. Reads BinPhysics `crystallization` and `flux` when an
> upstream slot writes them. Soft-Clip on by default — turn off if you're
> deliberately routing huge magnitudes into a downstream module.

### 4.2 Per-curve help text

Granular (mode = 0):
- `AMOUNT` (0): "How much of the historical bin replaces the current bin. 0 = current only, 1 = historical only. Adds with upstream BinPhysics `crystallization`."
- `Age` (1): "Per-bin lookback into history. 0 = now, 1 = oldest available frame."
- `THRESHOLD` (2): "Per-bin gate. Bins whose current magnitude falls below the threshold pass through unchanged."
- `Smear` (3): "Toggle (>0.5) per-bin 3-bin frequency smear of the historical read. Smooths bin-leakage across narrow partials."
- `MIX` (4): "Per-bin wet/dry."

DecaySorter (mode = 1):
- `AMOUNT` (0): "Per-bin output gain on the rearranged signal."
- `THRESHOLD` (2): "Per-bin floor — bins below this magnitude are excluded from sorting."
- `MIX` (4): "Per-bin wet/dry of sorted output vs. original."

Convolution (mode = 2):
- `AMOUNT` (0): "Per-bin convolution strength. Multiplied by upstream BinPhysics `flux` if present (gates by recent change)."
- `Delay` (1): "Per-bin delay into history. Low bins can sample old, high bins recent, or any other shape."
- `THRESHOLD` (2): "Per-bin gate on the current frame's magnitude."
- `MIX` (4): "Per-bin wet/dry."

Reverse (mode = 3):
- `AMOUNT` (0): "Per-bin keep during the reverse read."
- `THRESHOLD` (2): "Per-bin gate."
- `MIX` (4): "Per-bin wet/dry."

Stretch (mode = 4):
- `AMOUNT` (0): "Per-bin keep during the stretched read."
- `MIX` (4): "Per-bin wet/dry."

`help_for` is a `match` on `(mode, curve_idx)` — both are u8; total 17 string literals. Curves not listed return `""`, which the help-box treats as "fall back to module overview."

---

## 5. `past_config()` — per-curve display calibration

The placeholder `default_config()` arm in `curve_config::curve_display_config` is replaced by `past_config(curve_idx, mode)`. Returns a `CurveDisplayConfig` per `(mode, curve_idx)` pair so that:

- The slider's `custom_formatter` shows correct physical units (e.g. "0.42" for AMOUNT, "1.5 s" for Age in Granular).
- `apply_curve_adjustments` (graph) and `apply_curve_transform` (audio) use the right `offset_fn` per curve.
- `display_curve_idx` for Past returns curve-specific physical scales rather than falling through to the Dynamics scales.

| Curve | y_label | y_min | y_max | y_log | y_natural | offset_fn | grid_lines |
|---|---|---|---|---|---|---|---|
| AMOUNT (all modes) | "%"     | 0.0  | 100.0 | false | 100.0 | `off_mix`        | [25, 50, 75, 100] |
| TIME / Age (Granular) | "s" | 0.0  | `total_history_seconds` | false | 0.0 | `off_amount_norm` (linear add, clamped to gain ∈ [0, 1]) | quartile of `total_history_seconds` |
| TIME / Delay (Convolution) | "s" | 0.0  | `total_history_seconds` | false | 0.0 | `off_amount_norm` | quartile of `total_history_seconds` |
| THRESHOLD (Granular, DecaySorter, Convolution, Reverse) | "dBFS" | -80.0 | 0.0 | false | -60.0 | `off_thresh` (existing) | [-60, -40, -20, -6] |
| SPREAD / Smear (Granular) | "%"     | 0.0  | 100.0 | false | 100.0 | `off_mix` | [25, 50, 75, 100] |
| MIX (all modes) | "%"     | 0.0  | 100.0 | false | 100.0 | `off_mix`        | [25, 50, 75, 100] |

Notes on the table:

- `y_natural` is the **physical** value (in `y_label` units) that a default-drawn curve (`y = 0` at every node → linear gain = 1.0) maps to. AMOUNT/MIX/Smear default to "full effect" (= 100 %); TIME/Age/Delay default to "no offset / current frame" but currently the default y values map to `gain = 1.0` which displays as `total_history_seconds` (oldest frame). See §7.2 for the calibration mismatch and Future work.
- `off_amount_norm(g, o)` is a new helper added to `curve_config.rs`: `(g + o).clamp(0.0, 1.0)`. Documents "linear add to gain, never below 0 or above 1." Used by Age and Delay where the gain is interpreted as a normalised fraction of `capacity_frames`.
- `off_mix` is asymmetric (no-op on positive offset) and is correct for AMOUNT / MIX / Smear because their `y_natural = y_max` — there's no headroom above the natural value. This is the spec-allowed use of an asymmetric offset_fn (UI spec §7 pitfall 1).

`display_curve_idx` for Past returns:

| Past curve (mode) | display_idx | Resulting `gain_to_display` formula |
|---|---|---|
| 0 AMOUNT (all modes) | 6  | `gain × 100`, clamp [0, 100] |
| 1 Age (Granular) / Delay (Convolution) | 13 (NEW) | `gain × total_history_seconds`, clamp [0, total_history_seconds] |
| 2 THRESHOLD (Granular, DecaySorter, Convolution, Reverse) | 9 | dBFS, clamp [-80, 0] (existing Freeze formula) |
| 3 Smear (Granular) | 6 | same as AMOUNT |
| 4 MIX (all modes) | 6 | same as AMOUNT |

`gain_to_display` adds **display index 13 = "seconds (history-relative)"**. Formula: `(gain * total_history_seconds).clamp(0.0, total_history_seconds)`. The function signature gains a `total_history_seconds: f32` parameter (always passed by Pipeline-side callers; legacy callers pass `0.0` and never hit index 13). This is a small DSP-touching change.

Implementations not visible to Past (Granular Smear's "(>0.5 toggle)" semantic, etc.) are documented in the help text — the slider just shows 0–100 %; the user reads "Smear" in the label and the per-curve help says "toggle (>0.5)".

---

## 6. DSP changes

This spec is mostly UI re-architecture, but it requires four narrowly-scoped DSP edits:

1. **`apply_reverse`** drops the per-bin TIME averaging (`time.iter().take(n).copied().sum::<f32>() / n as f32 → window`) and reads the `s{s}_past_reverse_window_s` scalar instead. Conversion seconds → frames happens in `Pipeline::process()` and is passed in; the kernel signature changes from `(amount, time, threshold, mix, ctx)` to `(amount, threshold, mix, window_frames, ctx)`.

2. **`apply_stretch`** drops the per-bin TIME averaging and SPREAD averaging. Reads `s{s}_past_stretch_rate` and `s{s}_past_stretch_dither` scalars instead. Kernel signature changes from `(amount, time, spread, mix, ctx)` to `(amount, mix, rate, dither, ctx)`.

3. **`apply_soft_clip`** is a new helper called once at the end of `process()` after the matched mode kernel returns, gated on `s{s}_past_soft_clip`:

   ```rust
   fn apply_soft_clip(bins: &mut [Complex<f32>], num_bins: usize) {
       const K: f32 = 4.0;
       for k in 0..num_bins {
           let mag = bins[k].norm();
           if mag > 1e-9 {
               bins[k] *= K / (K + mag);
           }
       }
   }
   ```

4. **`ProbeSnapshot`** gains three new optional fields and one becomes mode-specific:
   - `past_reverse_window_s: Option<f32>` (Reverse only)
   - `past_stretch_rate: Option<f32>` (Stretch only)
   - `past_stretch_dither_pct: Option<f32>` (Stretch only)
   - Existing `past_time_seconds` becomes Granular/Convolution-specific (was previously populated for all modes via the averaged TIME curve).

The five new params (`floor_hz`, `reverse_window_s`, `stretch_rate`, `stretch_dither`, `soft_clip`) are added to the generated `params_gen.rs` via `build.rs` extensions: a per-slot family for each Past-specific param, alongside the existing curve / matrix / tilt / offset / curvature emitters.

---

## 7. Guardrails and known issues

### 7.1 Guardrails (enforced)

| Guard | Mechanism |
|---|---|
| `Floor` slider mapped to a bin index in `[1, num_bins - MAX_SORT_BINS]` | Bin-index mapping happens in `Pipeline::process` using current `sample_rate`/`fft_size`; result is clamped before passing to `apply_decay_sorter`. |
| `Window` clamped to `[1 hop, history.capacity_frames() × hop_seconds]` | Param range is wide (0.05–30 s); DSP-side conversion clamps to history reality. |
| `Rate` clamped to `[0.05, 4.0]` (UI displays 0.25× as the floor; 0.05 is the safety floor preventing pointer freeze) | Param range; documented in spec. |
| `Dither` clamped to `[0, 1]` | Param range. |
| **Soft Clip ON** prevents Convolution `bins[k] = bins[k] * frame[k] * amount` from exploding | Per-bin radial soft-clip applied after the mode kernel. |
| `default_config()`'s `offset_fn` is `off_identity` (un-calibrated). After this spec, every Past curve has a calibrated entry in `past_config()`. | Audited by `tests/calibration.rs::past_*` (extend to cover all five modes). |

### 7.2 Known issue NOT addressed by this spec — default curve y values mismatch some calibrations

A user-default curve has `y = 0` at every node, which `compute_curve_response` maps to `gain = 1.0` everywhere. For most curves that's the natural neutral, but several Past curves currently default to musically-inert states:

| Curve (mode) | Default-drawn → physical | Probably wanted by default |
|---|---|---|
| THRESHOLD (Granular, DecaySorter, Convolution, Reverse) | `gain = 1.0` → mag-squared gate at 1.0 — closes the gate for all typical FFT magnitudes | low or off (-60 dBFS) |
| Age (Granular) | `gain = 1.0` → read oldest frame in buffer | current frame (`gain = 0`) |
| Delay (Convolution) | `gain = 1.0` → read oldest frame | current frame |

Result: a freshly-instantiated Past slot with default settings produces no audible effect — the user must draw THRESHOLD down (and possibly Age/Delay down) before hearing anything. This is **not** a regression introduced by this spec; the gates were always too closed, just hidden behind un-calibrated display.

Two possible fixes (deferred — neither is in scope for this spec):

- **Per-module custom default node y values.** Extend `default_nodes_for_curve(curve_idx)` to `default_nodes_for_curve(module_type, curve_idx)`, letting Past override THRESHOLD's default y to e.g. -1.0 (gain ≈ 0.126, dBFS ≈ -18) and Age/Delay's defaults to a low value. Cleanest fix; no DSP changes needed.
- **Recalibrate the DSP thresholds.** Map the curve gain through a different function inside the DSP (e.g. THRESHOLD treats gain as already in dBFS-magnitude space). More invasive; touches every mode kernel.

Tracked in §8 Future work. The `default_config()` `off_identity` issue we just fixed is a separate concern; this is about the *default y values*, not the offset_fn.

This is a calibration mismatch between the curve y-value and the actual magnitude domain, **not** a regression introduced by this spec. Two possible fixes (deferred):

- **Recalibrate** the THRESHOLD curve so that `y_natural = -60 dBFS` (gain ≈ 0.001) maps the user's drawn curve to a sensible audible default. Requires changing default node y values for Past's THRESHOLD curve, OR mapping the curve through a dBFS-to-linear function inside the DSP.
- **Default the y nodes lower** for Past's THRESHOLD curve at module-instantiation time (similar to how `assign_module` already resets the curve nodes via `default_nodes_for_curve`). Cheaper change, narrower scope.

Tracked in §8 Future work; **not** in scope for this spec, which is a UI re-architecture.

---

## 8. Future work

- Source selector: main vs. aux sidechain history (deferred per Phase 5b2 spec — sidechain history buffers not allocated by default).
- Granular `FADE_ON_READ` toggle (memo §e — bin aging on retrieval).
- Stretch v2: Laroche–Dolson rigid peak-locking when artefacts surface in user testing (memo Research findings §3).
- DecaySorter performance: O(n × MAX_SORT_BINS) linear-min selection becomes a partial-heap before fft = 16384.
- Tempo-synced Reverse window length (depends on rhythm/sync infra already shipped — not blocked, just out of scope here).
- Per-mode overview text: currently a single module overview is reused for every mode; if user testing shows mode-specific guidance is desirable, populate each mode's `mode_overview`.
- THRESHOLD / Age / Delay default-y calibration fix (see §7.2).

---

## 9. Open questions

None at spec time — all decisions locked in via brainstorm of 2026-05-04. Outstanding ambiguities (TIME display formula, THRESHOLD audibility) are tagged Future work / known issue.

---

## 10. References

- [`2026-04-23-ui-parameter-spec-design.md`](2026-04-23-ui-parameter-spec-design.md) — UI parameter spec (foundation, addenda §7 + §8 add the `CurveLayout` infrastructure this spec consumes).
- [`2026-04-21-past-module.md`](2026-04-21-past-module.md) — original Past design spec; superseded in places by this UX overhaul.
- [`../plans/2026-04-27-phase-5b2-past.md`](../plans/2026-04-27-phase-5b2-past.md) — Phase 5b2 implementation plan; describes the DSP that this spec reframes.
- [`../../../ideas/next-gen-modules/13-past.md`](../../../ideas/next-gen-modules/13-past.md) — design memo & research synthesis the brainstorm drew from.
