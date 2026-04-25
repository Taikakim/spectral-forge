> **Status (2026-04-24): DEFERRED.** Depends on BinPhysics + host-BPM-sync infrastructure (both DEFERRED). Source of truth: [../STATUS.md](../STATUS.md).

# Rhythm Module — Design Spec

**Status:** Planned  
**Plan:** (to be written — Plan 8)  
**Depends on:** BinPhysics Infrastructure (Plan 1), Host BPM sync infrastructure

## What it is

A spectral module that applies Euclidean rhythmic masking, spectral arpeggiation, and bin-specific swing to different frequency bands — turning spectral energy into structural rhythm synced to the host's BPM.

## Host BPM sync infrastructure (prerequisite)

Before this module, the plugin needs a reliable way to read host transport:
- `nih_plug` exposes `ProcessContext::transport()` which gives `tempo`, `pos_beats`, `is_playing`
- Pipeline passes a `transport_snap` (beat position at the start of each block) into FxMatrix via `ModuleContext`
- `ModuleContext` gains two new fields: `bpm: f32` and `beat_position: f64`
- This is a one-block-latency approximation (good enough for rhythmic gating)

## Sub-effects

### Euclidean Rhythms
Spectrum is divided into zones by the graph. Each zone gets an independent Euclidean rhythm — a maximally even distribution of N beats across M steps. Example: bass bins get a 3-against-8 pattern (three beats evenly distributed across 8 sixteenth notes), mid bins get 5-against-8. When a zone's current step is "off," its bins are gated (attenuated toward zero). The graph's Y axis at each frequency sets the step count N (quantized to 1–8). A global step length control sets M (1, 2, 4, 8 steps; default 8). A broadband noise input becomes a polyrhythmic texture.

### Spectral Arpeggiator
Finds the 4–8 loudest spectral peaks each hop using IF tracking. Instead of sounding them simultaneously, sequences them one-by-one in BPM-synced steps. The graph maps frequency → position in the arpeggio sequence (earlier/later in the step). A simple step-sequencer UI (not graph-based, a row of 8 toggle buttons per voice) lets the user decide which voices play on which steps. Between a voice's "on" steps, its bins are attenuated smoothly (use existing Freeze module's portamento math).

### Bin Swing — Spectral Groove
Delays the OLA (overlap-add) reconstruction phase of specific bins on off-beats by a few milliseconds (1–20ms), controlled by the graph. Low-frequency bins stay quantized to the grid; high-frequency bins swing proportionally. Achieves physical groove strictly in the top end of the spectrum. Implementation: per-bin delay buffer (ring buffer) with variable read-head position. Read-head is advanced by the swing amount on off-beats, reset on beats.

### Rhythmic Phase Reset — Laser
On BPM-synced triggers (beat, half-beat, or graph-defined subdivision), forces the phase of all active bins to 0 (or a graph-defined target phase). Smeared or chaotic reverb tails snap into hyper-sharp constructive interference exactly on the beat. Can also be triggered by note input from sidechain transient detection (T/S Split output). Does not use BinPhysics.

## Curves (4 total)

| Idx | Label | Used by |
|---|---|---|
| 0 | AMOUNT | Euclidean (density), Arpeggiator (velocity envelope), Swing (depth), Phase Reset (strength) |
| 1 | DIVISION | Euclidean (step count N per band), Swing (off-beat amount) |
| 2 | ATTACK / FADE | Euclidean/Arpeggiator gate attack/release shape |
| 3 | MIX | All modes |

## Implementation notes

- Euclidean rhythm generation: classic Bjorklund algorithm — pre-compute the pattern once when parameters change, cache as `[bool; 16]`
- Swing delay buffers: pre-allocated at `reset()` — ring buffer sized for max swing (20ms × sample_rate / hop_size hops). One buffer per bin would be expensive; instead, apply swing as a phase offset in the OLA accumulation (requires pipeline-level cooperation, to be designed in Plan 8)
- BPM sync: `beat_position` from `ModuleContext` — compare fractional beat position to detect beat crossings. Store `prev_beat_position` in module struct.
- Arpeggiator step sequencer state: 8 `bool` values per "voice" (up to 8 voices) stored in module struct, exposed via a small custom UI widget (not the main graph editor)
