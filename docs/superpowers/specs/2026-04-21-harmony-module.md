> **Status (2026-04-24): DEFERRED.** Depends on BinPhysics + instantaneous-frequency infrastructure (both DEFERRED). Source of truth: [../STATUS.md](../STATUS.md).

# Harmony Module — Design Spec

**Status:** Planned  
**Plan:** (to be written — Plan 6)  
**Depends on:** BinPhysics Infrastructure (Plan 1); Instantaneous Frequency infrastructure

## What it is

A spectral module that analyzes pitch content and applies musically intelligent transformations: scale quantization, subharmonic generation, formant-preserving pitch shift, and harmonic companding. Requires Instantaneous Frequency (IF) tracking for sub-bin pitch accuracy.

## Instantaneous Frequency infrastructure (prerequisite)

Before the Harmony module can be built, a shared DSP primitive is needed:

**`src/dsp/utils.rs`** — add `compute_instantaneous_freq()`:
```
phase_delta[k] = (current_phase[k] - prev_phase[k]) - expected_phase_advance[k]
wrap to (-π, π)
instantaneous_freq[k] = bin_center_freq[k] + phase_delta[k] * sample_rate / (2π * hop_size)
```

This gives the exact frequency of the dominant partial in each bin, even if E and F fall in the same bin. Called once per hop, result stored in a `[f32; MAX_NUM_BINS]` array inside the module struct.

## Sub-effects

### Chordification — Bin Scale Quantization
Each bin's instantaneous frequency is snapped toward the nearest frequency in the user-defined scale. The graph controls "magnetism" (snap strength) per frequency band. Bins between scale degrees are either muted, blended, or smeared depending on a mode switch. Scale source: MIDI input (tracked in real-time) or a built-in scale selector (chromatic, major, minor, pentatonic, etc.). Does not use BinPhysics fields.

### Undertone Generator — Spectral Subharmonics
Finds the N loudest stable partials (using IF tracking + inter-hop stability test). For each, synthesizes subharmonics at f/2, f/3, f/4. The graph draws the amplitude envelope of the undertone series. "Stable" = IF-tracked frequency change < 2 semitones/hop over 3 consecutive hops. Requires a scratch output buffer for synthesized subharmonic bins (added to existing bins, not replacing them).

### Harmonic Companding
Identifies the fundamental and first 2 harmonics per detected partial (requires IF + MIDI or pitch tracking). Heavily compresses those bins (flatten the body). Upward-expands bins at 3rd–16th harmonics. Result: incredibly aggressive and biting without adding distortion — purely magnitude scaling. Reads: `mass` (bins with high mass resist companding). Writes: output magnitudes.

### Formant Rotation — Shepard Tone
Pitch-shifts bins, but first calculates and stores the spectral envelope (formants via cepstral smoothing), applies the shift, then re-applies the stored envelope. Additionally allows harmonics to infinitely "rotate" upward through the static formant envelope — creating a rhythmic Shepard-tone illusion synced to host BPM. Requires BPM sync infrastructure (Plan 8) for timed rotation.

## Harmony probability matrix

For Chordification and any chord-intelligent processing: a 12×12 float matrix (hardcoded music theory weights) maps detected pitch classes to harmony output probabilities. The matrix is defined once in `src/dsp/harmony_weights.rs` and imported by any module needing it. No ML inference at runtime — just a matrix multiply on 12 floats.

```
[C detected] → [E: 0.8, G: 0.9, C#: -1.0, ...]
[C + E detected] → [G: 0.95, B: 0.6, Bb: 0.4, ...]
```

## Curves (5 total)

| Idx | Label | Used by |
|---|---|---|
| 0 | AMOUNT | All modes — snap strength / depth |
| 1 | THRESHOLD | Undertone, Companding — minimum magnitude for detection |
| 2 | SPREAD | Undertone (harmonic amplitude decay), Chordification (snap width) |
| 3 | STABILITY | Undertone — inter-hop stability required before treating as a partial |
| 4 | MIX | All modes |

## Implementation notes

- IF computation requires storing previous-hop phases — `prev_phase[MAX_NUM_BINS]` inside module struct, reset to zeros on `reset()`
- Undertone synthesis: new complex bins are created using the detected IF as frequency and a derived phase — synthesized bins are added to existing `bins[k]`, not replacing them
- Harmonic companding requires re-detecting fundamental each hop from IF results — expensive but O(N log N) with a simple peak-finding pass
- No audio-thread allocation; all scratch buffers pre-allocated at `reset()` via `permit_alloc` pattern (same as existing practice in pipeline.rs)
