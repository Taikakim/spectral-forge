> **Status (2026-04-24): DEFERRED.** Depends on BinPhysics + history-buffer infrastructure (both DEFERRED). Source of truth: [../STATUS.md](../STATUS.md).

# Past Module — Design Spec

**Status:** Planned  
**Plan:** (to be written — Plan 9)  
**Depends on:** BinPhysics Infrastructure (Plan 1), History Buffer infrastructure

## What it is

A spectral module that reads from a rolling history buffer of past FFT frames — enabling granular spectral freezing of arbitrary time windows, decay-sorted spectral reconstruction, and self-convolution. The module is named "Past" because it literally accesses the audio's past.

## History Buffer infrastructure (prerequisite)

A shared rolling buffer of FFT magnitude+phase frames, maintained by `Pipeline`:

```rust
// src/dsp/history_buffer.rs (new)
pub struct HistoryBuffer {
    frames:   Vec<Vec<Complex<f32>>>,  // [MAX_HISTORY_FRAMES][MAX_NUM_BINS]
    write_pos: usize,
    pub num_frames: usize,
    pub num_bins:   usize,
}
```

- `MAX_HISTORY_FRAMES`: 30s × sample_rate / hop_size — at 44.1 kHz, hop=512, that's ~2580 frames. At 8193 bins × 8 bytes (Complex<f32>) × 2580 = ~169 MB. This is large. Options:
  - Store magnitudes only (f32) + phase only (f32) in separate arrays: same size
  - Reduce to 10 seconds: ~56 MB — more reasonable
  - Use `MAX_HISTORY_FRAMES = 512` (about 6 seconds at hop=512) initially and allow the user to configure via a plugin-level setting

- `Pipeline` writes to `HistoryBuffer` each hop after the STFT, before `FxMatrix::process_hop()`
- `Pipeline` passes `&HistoryBuffer` into `FxMatrix::process_hop()` as an optional argument, which passes it to modules that request it via `ModuleContext` (or a new `HistoryContext` struct)
- No audio-thread allocation — all buffer memory allocated at `Pipeline::initialize()`

Initial implementation: `MAX_HISTORY_FRAMES = 256` (≈3 seconds at default FFT size). Configurable later.

## Sub-effects

### Granular Window — Selective Time-Domain Freeze
Instead of freezing all bins, freeze only a graph-defined window of bins (e.g. 1–3 kHz). A secondary "scan position" parameter (BPM-syncable or LFO-driven) reads from a variable time offset in the history buffer for those frozen bins. Bass and treble continue in real-time; only the selected band is time-scanned. The graph controls which bins are frozen vs. live. Uses `crystallization` (from BinPhysics) to blend freeze depth per bin.

### Decay Sorter — Temporal Reconstruction
Analyzes the last N frames of the history buffer to compute each bin's decay time (how many frames it takes to drop 20 dB). Bins are then reconstructed ordered by their decay — long-ringing bins assigned to low-frequency output positions, short-lived bins assigned to high-frequency positions. This creates an alien reconstruction of audio based on temporal footprint rather than pitch. Output is the time-sorted spectrum, not the original.

### Spectral Convolution — Self-Resonance
Convolves the current spectrum with a delayed copy of itself from the history buffer. Only for bins above a graph-defined threshold. The convolution is point-wise multiplication in the frequency domain (not full convolution — that would change frequencies). Creates a blooming, self-resonating texture. The graph controls the delay offset (in frames/time) and the convolution depth. Reads `flux` (from BinPhysics) to gate which bins are eligible.

### Tape Print-Through — Ghost Pre-Echo
Takes 5% of a bin's current magnitude and writes it to a future position in a short write-ahead buffer. This ghost magnitude bleeds into the same bin N frames later, also leaking into adjacent bins. Creates pre-echo and post-echo spectral smearing that responds to volume. The delay time is set by a curve. Reads/writes a dedicated pre-echo buffer inside the module struct (NOT the main HistoryBuffer, since HistoryBuffer is read-only from the module's perspective).

## Curves (5 total)

| Idx | Label | Used by |
|---|---|---|
| 0 | AMOUNT | All modes — depth |
| 1 | TIME | Granular (scan offset), Convolution (delay frames), Print-Through (echo delay) |
| 2 | THRESHOLD | Decay Sorter (minimum amplitude to analyze), Convolution (bin eligibility) |
| 3 | SPREAD | Granular (width of frozen band around graph value), Print-Through (adjacent bleed) |
| 4 | MIX | All modes |

## Implementation notes

- HistoryBuffer is read-only from modules — Pipeline is the sole writer. This avoids concurrency issues.
- Granular scan: the "scan position" as a frame offset is computed from a secondary internal phasor or from `beat_position` if BPM sync is enabled. The result is `history_buffer.frame_at(offset)[k]` for frozen bins.
- Decay Sorter: analyzing N frames per hop is O(N×bins) — expensive. Limit analysis to every 4th hop (`frame_counter % 4 == 0`) and cache the sort order. Sort 8193 bins ≈ 8193 × log(8193) ≈ 105K comparisons — fast on modern CPU, especially if limited to the top 256 bins by amplitude.
- Self-convolution via point-wise multiply: `out[k] = bins[k] * history[frame][k].conj()` — not true convolution but perceptually similar for this use case
- Memory: module allocates Tape Print-Through's write-ahead buffer at `reset()` — `vec![0.0; MAX_NUM_BINS × MAX_ECHO_FRAMES]`, where `MAX_ECHO_FRAMES = 64` (≈750ms at hop=512)
