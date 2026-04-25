> **Status (2026-04-24): DEFERRED.** Depends on BinPhysics + instantaneous-frequency infrastructure (both DEFERRED). Source of truth: [../STATUS.md](../STATUS.md).

# Modulate Module — Design Spec

**Status:** Planned  
**Plan:** (to be written — Plan 7)  
**Depends on:** BinPhysics Infrastructure (Plan 1), Instantaneous Frequency infrastructure (from Plan 6)

## What it is

A spectral module for phase-based and frequency-space modulation — phase phaser, gravity phaser, FM networks, ring modulation matrix, and PLL-based effects. Primarily works in the phase domain rather than magnitude.

## Sub-effects

### Phase Phaser
Instead of targeting amplitude, targets phase. The graph sets per-bin phase rotation in radians (±π). Unlike PhaseSmear (which is random/stochastic), Phase Phaser is deterministic — the graph draws an exact phase tilt applied to every bin every hop. Creates comb-filter effects, phase cancellation patterns, and stereo widening without changing magnitude. Does not use BinPhysics.

### Gravity Phaser
Sweepable gravity nodes in frequency space. Each node exerts a phase-randomization force on nearby bins. The graph sets the force distribution across the spectrum. Sliders (not curves) set Q (node width) and intensity. Switches: invert every peak, invert every other peak. Node spacing presets: linear, logarithmic, Fibonacci. Nodes can receive position from sidechain peak detection. Reads: `phase_momentum` (accumulated phase velocity). Writes: `phase_momentum`.

### FM Network — Partial Web
Finds the N loudest partials using IF tracking. Each partial can frequency-modulate up to M neighbors within a distance set by one graph and at a depth set by another graph. Creates inharmonic, bell-like, metallic spectra without adding new bins — only shifts existing ones. The graph for distance sets how far each partial looks for a modulation partner. Does not use BinPhysics.

### RM/FM Matrix — Bin-to-Sidechain
Every bin of the input signal ring-modulates (point-wise multiplies) the corresponding bin of the sidechain signal. Creates vocoder-like, robotic textures. Has an amplitude falloff curve for higher bins (prevents wall-of-noise). Buchla-style amplitude envelopes (triggered by sidechain peaks) gate the effect to avoid constant noise. Does not use BinPhysics.

### PLL Tear — Phase-Lock Loss
Every bin contains a primitive phase-locked loop tracking the bin's phase from the previous frame. If the input frequency glides too fast (synth sweep, detected via IF delta), the PLL loses lock and emits chaotic sub-octave phase noise until the frequency stabilizes and relocks. State: `pll_phase[N]`, `pll_freq[N]` inside module struct. Reads: `phase_momentum` (to detect fast phase changes). Writes: `phase_momentum` (accumulated oscillation).

### Bin Swapper — Spectral Scramble
Divides the spectrum into bands (graph-defined). Routes magnitude of Band A to the frequencies of Band B and vice versa. Phase continuity is maintained by applying the target bin's existing phase to the swapped magnitude (avoids ring-modulation artifacts). Scramble can be rhythmically triggered (requires BPM sync). Does not use BinPhysics.

## Curves (5 total)

| Idx | Label | Used by |
|---|---|---|
| 0 | AMOUNT | All modes — depth/strength |
| 1 | REACH | FM Network (modulation distance), Gravity Phaser (node width), RM (falloff) |
| 2 | RATE | PLL (lock speed), Phase Phaser (rotation speed for animated phasing) |
| 3 | THRESHOLD | PLL (min phase delta for tear), FM Network (min amplitude for partial detection) |
| 4 | MIX | All modes |

## Implementation notes

- All phase operations must preserve DC (k=0) and Nyquist (k=last) as real values — same constraint as PhaseSmear
- FM Network: partial detection shares the IF infrastructure from the Harmony module — both can call the same `compute_instantaneous_freq()` utility
- RM Matrix: requires a sidechain signal at the slot's assigned SC input — falls back to self-sidechain if none assigned
- PLL state arrays allocated at `reset()`: `pll_phase: vec![0.0; MAX_NUM_BINS]`, `pll_freq: vec![0.0; MAX_NUM_BINS]`
- Gravity Phaser with BPM sync for node sweep is a Phase 2 feature — initially nodes are static or manually set
