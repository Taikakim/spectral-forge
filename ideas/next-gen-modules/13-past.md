# Past Module — Audit

**Existing spec:** `docs/superpowers/specs/2026-04-21-past-module.md`
**Status:** DEFERRED, depends on BinPhysics + History Buffer infra.
**Source brainstorm:** original brainstorm idea #16 (Granular Spectral
Freezing), #20 (Decay Sorter), #15 (Spectral Convolution with Internal
Feedback), #3 (Tape Print-Through from circuit list), Kim's note about
"a local history buffer for, say 30 seconds."

## What the spec covers

4 sub-effects: `Granular Window`, `Decay Sorter`, `Spectral Convolution`,
`Tape Print-Through`. Includes the History Buffer infrastructure
description. Curves: AMOUNT, TIME, THRESHOLD, SPREAD, MIX. Reads
BinPhysics `crystallization` (Granular Window) and `flux` (Spectral
Convolution).

## Brainstorm cross-reference

The Past spec did good coverage of the explicit history-related ideas in
the brainstorm. But the brainstorm is small here — Kim's prompt to a
prior AI was "Give me 20 more history-based ones that employ feedback,
hysteresis etc, take inspiration from circuit modeling," which yielded
the 20 circuit-modeled ideas that mostly went into Circuit / Modulate /
Past (Tape Print-Through). So the *direct* Past brainstorm is just 4-5
ideas, all covered.

What's missing is *more* history-based ideas the brainstorm didn't
generate but are obvious follow-ups now that the History Buffer exists.

## Gap details

### a) Tape Print-Through: split out to Future

The spec puts Tape Print-Through in Past, but it's a **write-ahead**
effect — it leaks current magnitude into a *future* buffer that bleeds
back later. That's not Past (read-only history). It's Future.

**Recommendation:** move Tape Print-Through to the Future module
proposal in `14-future.md`. The Past module shrinks to 3 sub-effects;
Future picks it up. This makes both modules conceptually clean:

- **Past** = read-only access to the History Buffer.
- **Future** = own write-ahead buffers and predictive effects.

Tape Print-Through *also* leaks ghost into adjacent bins, which is a
PCB-Crosstalk-like effect — could equally fit in Circuit. But its
defining property is the *time-shift* (5% leaks 500ms later), which is
Future's territory.

### b) Decay Sorter variants

The existing Decay Sorter sorts bins by 20-dB decay time. Kim asked
"What are left and right here?" — fair, the spec is unclear.

**Clarification:** "shift to the left/right" in a 1-D spectrum means
"to lower/higher frequency bins." So bins that ring out longest get
*remapped* to the lowest available output bin slot, bins that decay
fastest go to the highest. The output spectrum is reordered by decay
time, not pitch.

This is one of three sort dimensions worth offering as variants:

| Variant | Sort key | Audible result |
|---|---|---|
| Decay | seconds-to-fall-20-dB | Long-ringing → bass; transient → treble |
| IF stability | per-hop frequency variance over N frames | Stable partials → low; modulating → high |
| Energy area | total energy integrated over N frames | High area → low; spiky → high |

Each is cheap once you have N frames of history. The main cost is the
sort itself — limit to top 256 bins by amplitude as the spec already
suggests.

**Sub-effect refinement.** Add a `SortKey` enum (`Decay`, `Stability`,
`Area`) to the existing Decay Sorter mode. The user picks one; the
machine sorts.

### c) Reverse playback

**Concept.** Play back the History Buffer in reverse — start at frame
(now - N) and read forward to (now). Combined with overlap-add this
gives a smoothly time-reversed spectrum.

**Sub-effect proposal: `Reverse` mode**
- Reads: HistoryBuffer at `(write_pos - read_offset)`, where
  `read_offset` is incremented backward each hop.
- Writes: bin magnitudes + phases from the reversed read.
- State: per-mode `read_phase` (f32, current read offset in frames).
- Curves: AMOUNT (wet/dry), TIME (length of the reverse window in
  beats / seconds), THRESHOLD (only reverse bins above this magnitude
  — keeps quiet bins normal), MIX.
- CPU: light (just buffer indexing and copy).

A scratchy / glitchy version of the famous Buchla 251e tape head reverse
trick, but per-bin selectable.

### d) Time stretch via variable read rate

**Concept.** Read the HistoryBuffer at a non-1.0 rate. 0.5× = half-
speed playback (twice as long). 2.0× = double-speed.

**Sub-effect proposal: `Stretch` mode**
- Reads: HistoryBuffer with fractional read position, linear-interp
  between adjacent frames.
- Writes: bin magnitudes + phases from the stretched read.
- State: `read_phase` (f64 for precision).
- Curves: AMOUNT (wet/dry), TIME (rate, log-scaled around 1.0),
  THRESHOLD, MIX.
- CPU: light.

Phase coherence at non-unity rate is the tricky bit — naive read gives
phase glitches. Either accept glitches as the "stretched tape" sound,
or use phase-vocoder-style rotation by `2π × bin_freq × delta_t`.

### e) Bin aging — dim with each retrieval

**Concept.** Each time a bin is read from history, it dims slightly in
the buffer. Repeated reads exhaust a bin's history.

**Useful?** Doesn't give a strong audible behaviour on its own — but
combined with feedback (Past output routed back to its own input via
the matrix) creates self-decaying loops. Worth noting but not a sub-
effect; instead, add a "FADE_ON_READ" toggle to Granular Window.

### f) Self-convolution clarifications

The spec's Spectral Convolution sub-effect uses "point-wise multiply in
the frequency domain" rather than full convolution. This is correct for
"self-resonance" (it boosts bins that have been loud) but doesn't match
the brainstorm wording "convolve the spectrum with a delayed version of
itself." Convolution in the frequency domain = multiplication in the
time domain. Multiplication in the frequency domain = convolution in
the time domain. So the spec's choice is *time-domain convolution* —
which actually adds reverb/density.

**Recommendation:** keep the spec's algorithm (it's the more useful
one), but note in the spec that "spectral convolution" here means the
time-domain operation realized via frequency-domain multiplication.
The user-facing label can stay "Convolution."

## History Buffer details — spec gaps

### Memory budget revisit

Spec says 256 frames default ≈ 3 s at default FFT. At hop=128 this is
0.74 s — too short for the "30 second" feel Kim mentioned.

**Recommendation:** make `MAX_HISTORY_FRAMES` configurable per-plugin-
instance, default 4 s in seconds (computed at init from sample_rate
and current hop). Document that increasing it costs ~280 KB per second
per bin-bytes. Do not allow the user to allocate it *during* audio —
only at preset-load time.

### Stereo handling

Spec says "implicitly assumes the history is only of the main signal."
For Independent stereo we need either:
- Per-channel history (doubles memory to ~34 MB at 256 frames)
- Mono-summed history (same memory; loses stereo)

**Recommendation:** per-channel history. Memory is fine on desktop. UX:
"Past is per-channel" matches what users expect from a "past" module.

### Sidechain history

Sidechains have their own STFTs (`sc_stfts[0..4]`). Should Past be able
to read *sidechain* history? Probably yes for advanced workflows
(rhythmic sidechain pulled from past). Add `HistorySource { Main,
Sidechain(0..3) }` per Past slot. Memory grows accordingly — only
allocate sidechain-history buffers if any slot subscribes.

### Pre-computed summary stats

`01-global-infrastructure.md` §2 raised the question: should
HistoryBuffer expose per-bin summary arrays (decay time, RMS envelope,
stability score) computed once and shared?

For Past specifically, **yes** — Decay Sorter and the new SortKey
variants all need this. Compute summary stats lazily on first request
each block. Cache for the rest of the block. Cost: O(N × num_bins) per
unique stat per block, where N is the analysis window.

## Curve set

Spec has 5: AMOUNT, TIME, THRESHOLD, SPREAD, MIX. With Reverse and
Stretch added, still fits. Tape Print-Through moves to Future, freeing
THRESHOLD to be more uniformly meaningful across remaining modes.

| Idx | Label | Used by |
|---|---|---|
| 0 | AMOUNT | All modes |
| 1 | TIME | Granular (scan offset), Convolution (delay), Reverse (window length), Stretch (rate) |
| 2 | THRESHOLD | Decay Sorter (min amp), Convolution (eligibility), Reverse (min mag) |
| 3 | SPREAD | Granular (band width), Stretch (smoothing) |
| 4 | MIX | All modes |

## CPU class

Decay Sorter is the heavy mode (sort each hop). Other modes are O(N)
buffer reads. **`heavy_cpu = true`** for the module overall (the Sort
mode dominates), but per-mode heavy flag (per `12-kinetics.md`
recommendation) would mark only Decay Sorter heavy.

## BinPhysics interactions

Reads: `crystallization` (Granular Window blends freeze depth), `flux`
(Convolution gates eligibility).
Writes: nothing — Past is mostly a *read* of stored audio, not a
modifier of physics state. Adding writes (e.g. "playing back history
sets `temperature`") is possible but obscures the conceptual model.

## Calibration probe set

- `probe_amount_pct`
- `probe_time_seconds` (read offset for Granular, delay for Convolution,
  window length for Reverse, rate for Stretch)
- `probe_active_mode_idx`
- `probe_history_frames_used` (how many frames the current mode needed)
- `probe_sort_key_idx` (Decay Sorter only)

## RESEARCH PROMPT — phase coherence in stretched STFT playback

```
Topic: Phase-coherent playback of a frequency-domain history buffer
at variable read rates.

Context: We have a rolling buffer of complex STFT frames (8193 bins,
hop=128 to 1024 depending on user setting). We want to play back at
arbitrary rate (0.25× to 4× the recording rate) without phase glitches
that would manifest as audible artifacts in overlap-add reconstruction.

Goal: Identify the cheapest correct phase-rotation scheme for variable-
rate STFT playback. Naive linear interpolation of complex bins fails;
classic phase-vocoder phase rotation works but is expensive.

Specific questions:
1. Is "phase rotation by 2π × bin_freq × time_offset" sufficient when
   reading from history at fractional frame positions? What about
   bins where the actual partial drifts off bin-center (use IF)?
2. For 0.25× / 0.5× / 2× / 4× (integer + simple ratios), can we
   precompute phase-rotation LUTs?
3. Phase-locking neighbouring bins (per Laroche/Dolson) costs O(N)
   per hop. Is it worth it for our use case where the user might
   intentionally want some warble?

Deliverable: A Rust algorithm + reference implementation comparison
(naive lerp vs phase-vocoder vs phase-locked vocoder) with audio
examples and CPU costs at 8193 bins.
```

## Open questions

1. **Move Tape Print-Through to Future:** confirm.
2. **Per-channel vs mono history buffer:** per-channel.
3. **Sidechain history support:** ship now or v2?
4. **HistoryBuffer summary stats lazy-computed and cached:** approve?
5. **MAX_HISTORY_FRAMES configurable:** approve, default 4 s.
6. **Sort key variants** (decay/stability/area): all three or just
   decay for v1?
7. **Phase-coherence math for Stretch:** see RESEARCH PROMPT.

## Research findings (2026-04-26)

Phase-coherent stretch is covered by `research/05-time-manipulation.md`
(Topic A). Validated decisions:

1. **Do not wrap Rubber Band** (GPL contagion) **or Signalsmith
   Stretch** (would force a second STFT inside our existing one).
   Build a minimal in-house **Stretch** kernel using the existing
   STFT and a fractional read position into the HistoryBuffer.
2. **Phase coherence = Puckette/lamination-style** (no peak detection)
   for v1. Reads from a fractional history-frame position; the phase
   anchor advances by per-bin instantaneous frequency `ω_inst · Hs`.
   Ships at ~80 lines of code, <1% of one core for 8193 bins. Cost
   ≈ 12 ops + 1 sin/cos per bin per hop (replace sin_cos with the
   shared `PhaseRotator` LUT for production).
3. **Laroche-Dolson rigid peak-locking** is the v2 quality upgrade
   if listening tests show producers want it. The peak detector is
   already shared infrastructure (per `20-plpv-phase-cross-cutting.md`),
   so the upgrade path is well-understood.
4. **PVDR (Phase Vocoder Done Right)** is the objective SOTA per
   Pruša & Holighaus 2017-2022 (arXiv:2202.07382). CPU is acceptable,
   but the heap-driven phase integration is ~500 LOC of careful index
   arithmetic. Tag as v2 quality upgrade if Stretch artefacts surface
   in user testing.
5. **Shared infrastructure with Future and Punch:**
   - `PhaseRotator` helper in `src/dsp/utils.rs` or new
     `src/dsp/phase.rs` — handles `Complex<f32> × (freq_offset,
     time_delta)` with a 1024-entry sin/cos LUT. ~6 muls + 3 adds
     per call.
   - `if_offset[k]` cache in `ModuleContext`, filled once per hop in
     `Pipeline::process()` after the analysis FFT, before FxMatrix
     dispatch. Shape `[channel][bin]`, ~65 KB total. Fill cost
     ~16k subtract+wrap ops per hop, ~0.3% of one core. Cache is
     "as of the analysis FFT, before any FxMatrix processing" —
     modules that care about *post-processing* IF must compute it
     themselves.
6. **Stereo locking:** in Independent stereo, both channels read the
   *same* fractional history offset (mono lag/lead, stereo content),
   not independent offsets per channel. Avoids stereo flam.
7. **Unified buffer ring** (one big `Complex<f32>` ring with read
   offsets in *both* directions) shared by Past and Future is
   architecturally cleaner; defer the decision until both modules are
   unblocked to avoid creating a Past↔Future shipping dependency.
