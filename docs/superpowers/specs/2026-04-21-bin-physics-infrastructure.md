> **Status (2026-04-28): SUPERSEDED by [`../plans/2026-04-27-phase-3-bin-physics.md`](../plans/2026-04-27-phase-3-bin-physics.md).** The Phase 3 plan ships the implementation under a refined architecture (per-field `MergeRule`, opt-in writer/reader split, auto-velocity). This spec is preserved for history; do not follow as written. Source of truth: [../STATUS.md](../STATUS.md).

# BinPhysics Infrastructure — Design Spec

**Status:** Planned  
**Plan:** `docs/superpowers/plans/2026-04-21-bin-physics-infrastructure.md`

## What it is

A per-bin physical state system that travels through `FxMatrix` alongside audio bins. Every bin carries a small set of physical properties that persist across slots — so a spring module in slot 6 can see the momentum a kinetics module gave bins in slot 1.

## Fields

| Field | Default | Meaning |
|---|---|---|
| `velocity` | 0.0 (auto) | Magnitude rate-of-change between hops. Auto-computed by FxMatrix from signal, not by modules. |
| `mass` | 1.0 | Inertia — how hard it is to redirect a bin. High mass = slow response. |
| `temperature` | 0.0 | Thermal energy. Increases with sustained high amplitude; drives saturation, drift, expansion. |
| `flux` | 0.0 | Magnetic saturation state. Builds up over time; causes compression and phase rotation. |
| `displacement` | 0.0 | Deviation from spectral rest position. Used by spring and gravity modules. |
| `crystallization` | 0.0 | Phase coherence level. 0 = chaotic, 1 = fully locked in integer ratios. |
| `phase_momentum` | 0.0 | Angular phase velocity. Used by orbital gravity and phase phaser modules. |

## Core rules

1. `velocity` only: auto-computed by FxMatrix each hop from hop-to-hop magnitude delta of the assembled slot input. Modules don't write velocity directly (they write `mass`, `displacement`, etc. and let velocity arise naturally).
2. All other fields: inert until a module writes them. Modules that don't use a field simply leave it unchanged.
3. Adding a new field to `BinPhysics` only requires: adding the field, updating FxMatrix init/reset, and updating modules that USE the new field. All other modules compile unchanged.
4. No allocation on the audio thread — all `Vec<f32>` pre-allocated at `MAX_NUM_BINS = 8193` in `FxMatrix::new()`.

## Routing / mixing

When multiple upstream slots send to the same destination (via RouteMatrix), FxMatrix assembles the destination's input physics using amplitude-weighted averaging — identical to how complex bin values are summed. This preserves proportionality: a send at 0.5 amplitude contributes half the weight to the mixed physics.

## How to add new global properties in the future

1. Add `pub new_property: Vec<f32>` to `BinPhysics` struct in `src/dsp/bin_physics.rs`
2. Initialize in `BinPhysics::new()`: `new_property: vec![DEFAULT; MAX_NUM_BINS]`
3. Reset in `reset_active()`: `self.new_property[..num_bins].fill(DEFAULT)`
4. Mix in `mix_from()`: add the weighted average line
5. Copy in `copy_active_to()`: add the copy_from_slice line
6. FxMatrix picks it up automatically (no changes needed there)
7. Write a module that uses it — all other modules are untouched
