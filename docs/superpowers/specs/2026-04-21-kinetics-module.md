> **Status (2026-04-24): DEFERRED.** Depends on BinPhysics infrastructure (DEFERRED). Source of truth: [../STATUS.md](../STATUS.md).

# Kinetics Module — Design Spec

**Status:** Planned  
**Plan:** (to be written — Plan 4)  
**Depends on:** BinPhysics Infrastructure (Plan 1)

## What it is

A spectral module that applies physical forces to bins — springs, gravity wells, inertial mass, magnetic attraction/repulsion. Bins carry momentum and mass through the FxMatrix; Kinetics reads and writes those fields. Multiple Kinetics instances in a chain accumulate forces on the same bins.

## Sub-effects

### Hooke — Spring Connections
Adjacent bins are connected by virtual springs. A loud transient in bin N pulls bins N±1, N±2 upward with spring force proportional to displacement. Stiffness curve controls spring constant per frequency band. Also supports harmonic springs: bin F is spring-connected to 2F, 3F, 4F. Reads: `velocity`, `displacement`, `mass`. Writes: `displacement`, `velocity`.

### Gravity Well
One or more frequency-space gravity wells attract bins toward a center frequency. The curve sets well positions and strength across the spectrum. Can be set to repulsion instead of attraction (switch). Wells can track MIDI or sidechain peaks. Reads: `displacement`, `velocity`, `mass`. Writes: `displacement`, `velocity`.

### Inertial Mass
Assigns variable mass per frequency band from the graph. High-mass bins (bass) are slow to accelerate (attack) and hard to stop (release). Creates a deeply physical, sluggish compressor that feels like moving machinery. Writes: `mass` (the primary purpose of this sub-effect).

### Orbital Phase
A massive spectral peak captures smaller nearby peaks. The phase of lighter peaks is forced to orbit the phase of the heavy peak at a rate proportional to distance. Creates physically modeled vibrato and spatial width. Reads: `mass`, `phase_momentum`. Writes: `phase_momentum`.

### Ferromagnetism — Phase Alignment
Loud bins act as magnetic sources that pull the phase of nearby quieter bins into alignment. Turns blurry, unpitched noise into laser-focused, phase-locked harmonic tones. Reads: `flux`, `mass`. Writes: `flux`.

### Thermal Expansion — Detuning
A frequency band that absorbs sustained RMS energy heats up (writes `temperature`) and physically detuning-expands — bins drift outward in frequency. Cools and contracts when signal stops. Reads/writes: `temperature`. Achieves detuning via phase accumulation (instantaneous frequency shift).

## Curves (5 total)

| Idx | Label | Used by |
|---|---|---|
| 0 | STRENGTH | All modes — force magnitude |
| 1 | MASS | Inertial Mass, Gravity, Springs |
| 2 | REACH | Gravity, Springs, Orbital, Ferromagnetism |
| 3 | DAMPING | Springs, Orbital — energy loss rate |
| 4 | MIX | All modes |

## Implementation notes

- Spring connections require reading the neighboring bin's displacement before updating — process in two passes (read pass, write pass) or use a scratch buffer to avoid order artifacts
- Gravity and orbital effects require knowing the position of the "massive" peak each hop — find the loudest peak in a frequency region before the per-bin loop
- `velocity` written by this module is additive to the auto-computed velocity from FxMatrix — modules add their forces, FxMatrix baseline provides the starting momentum
- SIMD-friendly: all operations are per-bin scalar or adjacent-bin reads (sequential memory)
