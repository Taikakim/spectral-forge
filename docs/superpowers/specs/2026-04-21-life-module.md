> **Status (2026-04-28): IMPLEMENTED** — implementation plan `2026-04-27-phase-5a-life.md` landed in commits `33ecea8`..`085a49d`. The plan extended this design with four additional gap modes (Yield, Capillary, Sandpaper, Brownian) per the audit at `ideas/next-gen-modules/11-life.md`. Source of truth for runtime behaviour: source code (`src/dsp/modules/life.rs`). Source of truth for status: [../STATUS.md](../STATUS.md).

# Life Module — Design Spec

**Status:** Planned  
**Plan:** (to be written — Plan 3)  
**Depends on:** BinPhysics Infrastructure (Plan 1)

## What it is

A spectral module where energy obeys biological and fluid physics — it diffuses, coalesces, crystallizes, and conserves. The module has a sub-effect selector (mode switch) and 4–5 curves controlling the selected effect.

## Sub-effects (AmpMode-style selector)

### Viscosity — Diffusion
Energy spreads laterally from loud bins into quieter adjacent bins, weighted by a viscosity curve. "Honey" zones move slowly; "Water" zones spread fast. Energy is approximately conserved — diffusion takes from the source and gives to the destination. Reads/writes: `displacement` (rate of spread), `temperature` (heat accelerates diffusion).

### Surface Tension — Coalescence
Adjacent small peaks "attract" each other and clump into a single larger peak, reducing spectral "surface area." Acts as a noise-reduction tool: broadband noise → discrete bubbling sine tones. Writes: `crystallization` (as bins settle into stable peaks).

### Crystallization — Phase Lattice Snapping
Sustained bins slowly align to integer harmonic ratios and lock phases together. Incoming transients melt the crystal back to chaotic audio. Reads: `crystallization` (current lock level). Writes: increases `crystallization` when sustained, resets it on transients. Closely related to Freeze module but physically motivated.

### Archimedes — Volume-Conserving Ducking
Total spectral energy is treated like liquid volume in a tank. When a loud element occupies a large fraction of the "tank," quieter bins are physically displaced (amplitude reduced) to make room. Has an "escape valve" parameter that allows some overflow. Reads: global RMS energy. Does not use BinPhysics fields directly.

### Non-Newtonian — Oobleck Effect
Responds to the *rate* of amplitude change rather than amplitude itself. Slow changes (pads) pass freely. Fast changes (drum transients) cause the medium to solidify — hard-limiting the transient, then melting immediately. Reads: `velocity` (auto-computed by FxMatrix). Writes: `displacement`.

### Stiction — Grain/Halt Gate
Bins require a minimum accumulated force to break free from zero (static friction). Once moving, they slide freely (kinetic friction). Reverb tails don't fade smoothly — they grind to a halt in blocky, textured steps. Reads: `velocity`. Writes: `displacement`.

## Curves (5 total, active set depends on mode)

| Idx | Label | Used by |
|---|---|---|
| 0 | AMOUNT | All modes — primary depth/strength |
| 1 | THRESHOLD | Viscosity, Stiction, Non-Newtonian |
| 2 | SPEED | Viscosity, Crystallization — rate of change |
| 3 | REACH | Viscosity, Surface Tension — how far energy spreads |
| 4 | MIX | All modes — wet/dry blend |

## Implementation notes

- All effects are SIMD-friendly: operate on flat `[f32]` slices, adjacent-bin reads are sequential
- Viscosity and Surface Tension need a temporary read buffer (copy of input magnitudes before diffusion) to avoid order-of-processing artifacts — FxMatrix's existing `mix_buf` can serve or a local `scratch: Vec<f32>` pre-allocated in the module struct
- No allocation on audio thread

## Follow-ups (post Phase 5a)

- **Freeze reads `BinPhysics.crystallization`** — Phase 5a Life writes
  `crystallization[k]` from the Crystallization mode but Freeze does not yet
  read it. Add a small follow-up PR to make Freeze accumulate faster on
  bins where `crystallization > 0` (per audit § Crystallization scope vs
  Freeze module). This is intentionally NOT in Phase 5a's scope to keep
  the plan focused on the Life module itself.
- **Multi-mode-per-slot stacking** — v2 enhancement (audit § Module ordering).
- **Cepstral envelope baseline for Capillary** — v2 enhancement (audit
  research finding 7).
