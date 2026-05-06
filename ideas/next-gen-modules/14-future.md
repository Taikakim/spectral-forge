# Future Module — NEW Module Proposal

**Existing spec:** none — this file is the first proposal.
**Status:** RESEARCH — not yet specced.
**Source brainstorm:** Tape Print-Through (#3 of circuit list), Kim's
own naming hint: "could all effects like this be bundled under the
'future' module (hey I like the modules: 'Past', 'Future', 'Life',
'Kinetics', etc. not even as gimmicky, since it would push the users
further from the 'oh a delay mmh'-reaction)."

## Why Future is its own module

The original Past module spec has Tape Print-Through, which is a
**write-ahead** effect — it writes 5% of a bin's current magnitude into
a buffer that will be read 500 ms in the future. That's the inverse
direction from Past, which only *reads* the History Buffer.

Conflating read-history and write-future inside Past muddies the model:

- **Past** = read-only access to the History Buffer (shared, owned by
  Pipeline).
- **Future** = own write-ahead buffers and predictive effects that can
  exist without the History Buffer.

Splitting them gives:

1. Past stays purely read-side, simple to implement once the buffer
   exists.
2. Future has its own internal write-ahead buffers and is independent
   of the History Buffer infrastructure.
3. Users see the symmetry — "Past" and "Future" as a matched pair next
   to each other in the module list.
4. Future can ship before the HistoryBuffer infrastructure if needed
   (it doesn't depend on it).

## Proposed sub-effects

### a) Tape Print-Through (relocated from Past)

**Concept (per existing Past spec).** Take 5% of a bin's current
magnitude and write it to a delay buffer. The ghost magnitude bleeds
into the same bin N hops later, and into adjacent bins N±1 (capacitive-
coupling style). Creates pre-echo/post-echo spectral smearing that
responds physically to input volume.

- State: `pre_echo_buf[MAX_NUM_BINS × MAX_ECHO_FRAMES]` ring buffer
  (~2 MB at 64 frames). Per-channel.
- Curves: AMOUNT (leak %), TIME (echo delay), SPREAD (adjacent bleed),
  MIX.
- CPU: light.

### b) Look-Ahead Duck

**Concept.** Process a peek of the *future* signal (which we have, since
the OLA introduces FFT_SIZE samples of look-ahead anyway) to apply a
duck *before* the loud event arrives. This is what lookahead-limiters
do at the time-domain level; we do it per bin in the spectrum.

The current shipped Dynamics module does not do this. It reacts to the
*current* hop's bin magnitudes. With FFT_SIZE samples of inherent
look-ahead already in our latency budget, we can preview the upcoming
hop and pre-emptively duck.

- Reads: HistoryBuffer at `(write_pos + lookahead_frames)` … wait. The
  HistoryBuffer is *past*. The lookahead is built into the OLA: when
  the user submits a buffer, the next hop's FFT covers samples that
  haven't yet been output. The Pipeline already knows them.
- **Implementation:** Future module stores its own one-hop-ahead bin
  snapshot, computed during the *next* hop's STFT but *before* the
  current hop's FxMatrix runs. This requires a small Pipeline
  reordering: STFT next-hop → run Future module on this-hop with
  next-hop preview → continue normal FxMatrix.

This is intrusive enough to the Pipeline that it deserves its own
plan. **Question:** is the audible benefit big enough to warrant the
Pipeline change? For percussive sources, lookahead ducking is a huge
win. For pads, near-zero. Worth scoping.

### c) Predicted-Spectrum Interpolation

**Concept.** Predict the next hop's spectrum from the last N hops
(linear extrapolation of magnitude per bin), and blend the prediction
with the actual current bin. Adds anticipation — the spectrum is "what
it's about to be" rather than "what it currently is."

- State: short ring of last 4 hops' bin magnitudes (~130 KB per
  channel).
- Curves: AMOUNT (blend factor), TIME (number of hops back to use for
  extrapolation), THRESHOLD (only predict bins above this magnitude),
  MIX.
- CPU: light.

Most useful for soft-tracking effects (vocoders, sidechains) where
slight anticipation makes the output feel "tight." Equivalent to a
subtle phase shift in the time domain but spectrum-shaped.

### d) Pre-Echo with Arbitrary Pre-Delay

**Concept.** Generic version of Tape Print-Through with user-controllable
pre-delay length and feedback. Distinct from Print-Through because:

- Print-Through is a small (5%) leak with adjacent-bin spread, mimicking
  tape physics.
- Pre-Echo is the *full* signal output ahead of itself, with feedback,
  potentially BPM-synced.

Implementation is the same write-ahead buffer plus a feedback path. UI:
user explicitly knows it's a pre-echo, not a "print-through."

- State: same buffer as Print-Through (could share).
- Curves: AMOUNT (echo amplitude), TIME (delay), THRESHOLD (feedback
  decay), SPREAD (per-echo high-frequency damping), MIX.
- CPU: light.

### e) Crystal Ball — full one-frame look-ahead delay

**Concept.** The simplest Future effect: output the spectrum of the
*next* hop, not the current one. Equivalent to a one-FFT-hop pre-echo
of the entire signal at 100% wet.

Probably useless on its own (it just adds latency for no obvious
gain), but useful as a building block: combined with Dynamics it gives
true look-ahead spectral compression.

**Recommendation:** don't ship as a sub-effect. Mention as a primitive
that other modules could opt into via a `lookahead: bool` ModuleSpec
flag. Scope creep — defer.

## Proposed curve set

| Idx | Label | Used by |
|---|---|---|
| 0 | AMOUNT | All modes — dry/wet on the predicted/echoed signal |
| 1 | TIME | Print-Through (echo delay), Lookahead Duck (lookahead amount), Prediction (history depth), Pre-Echo (delay length) |
| 2 | THRESHOLD | Lookahead Duck (peak detection), Prediction (mag floor), Pre-Echo (feedback decay) |
| 3 | SPREAD | Print-Through (adjacent bleed), Pre-Echo (HF damping) |
| 4 | MIX | All modes |

5 curves. `num_curves() = 5`.

## Architecture fit

### Plain SpectralModule slot, no special infra needed

Future does NOT need:
- The shared HistoryBuffer (it owns its own write-ahead buffers).
- BinPhysics (it doesn't write physics state).
- IF / chromagram / cepstrum (no pitch math).
- MIDI (no note tracking).

It DOES need (Lookahead Duck only):
- A small Pipeline reordering for one-hop-ahead access.

This makes Future the **easiest** of the new modules to ship — most
sub-effects are just per-bin ring buffers.

### Why the user-facing name "Future" matters

Same reason Kim wanted the existing module names — "Future" lands as a
concept, "Pre-Echo Module" lands as a tool. Users will:

- Combine "Past" (slot 1) → "Future" (slot 2) and feel the symmetry.
- Be willing to try unfamiliar workflows because the name suggests
  exploration.
- Map their producer-mental-model ("I want this to react before the
  beat") to "Future" without searching docs.

The naming is design work, not flavour.

## CPU class

Light overall. Pre-echo / Print-Through buffers are the main RAM
consumer (~2 MB per slot per channel). Tag `heavy_cpu = false`.

## Calibration probe set

- `probe_amount_pct`
- `probe_time_seconds` (echo delay or lookahead amount)
- `probe_active_mode_idx`
- `probe_buffer_fill_pct` (how much of the write-ahead buffer is in use)

## RESEARCH PROMPT — Predictive spectral extrapolation accuracy

```
Topic: Per-bin predictive extrapolation of spectral magnitudes for
real-time anticipation effects.

Context: We have a sequence of STFT frames (one per hop, hop=128–512
samples). We want to predict frame N+1's magnitudes from frames
N-K…N for various K (3, 4, 8). Goal: prediction good enough that
mixing the predicted frame in at 30% gives perceptual "tightness"
without sounding broken.

Specific questions:
1. Linear extrapolation of magnitude in dB vs. linear: which is more
   audibly forgiving when the prediction is wrong (and it will be wrong)?
2. Per-bin AR(K) with simple Burg/Yule-Walker fit each block: too
   expensive at 8193 bins?
3. Phase prediction is hard (phase is wrapped). Just keep current
   phase and only predict magnitude?
4. When the input is steady (sustained chord), prediction is trivially
   accurate. When it's transient (drum hit), prediction is catastrophic.
   Is there a simple "prediction confidence" per bin we can compute
   cheaply, and downgrade to dry signal when confidence is low?

Deliverable: a Rust kernel with confidence weighting + audio examples
showing the failure mode (mispredicting a transient).
```

## Open questions

1. **Move Tape Print-Through here from Past:** confirm with Past audit.
2. **Lookahead Duck Pipeline reorder:** worth the intrusion, or skip
   that sub-effect for v1?
3. **Prediction "confidence" weighting:** see RESEARCH PROMPT.
4. **Crystal Ball as a sub-effect or ModuleSpec flag:** the flag option
   keeps Future's UI clean.
5. **Naming:** "Future" — confirm. Alt: "Anticipate," "Pre," "Foretell."
   "Future" is best per the analogy with Past.

## Research findings (2026-04-26)

Predicted-Spectrum Interpolation is covered by
`research/05-time-manipulation.md` (Topic B). Validated decisions:

1. **Predicted-Spectrum Interpolation = linear-in-dB prediction +
   flux-and-IF-variance confidence gating + reused phase.**
   ~80 lines of code, <1% of one core at 8193 bins / hop=256 / 4×
   independent stereo. Ship as the v1 sub-effect.
2. **Reuse spectral-flux infrastructure** that Dynamics needs anyway
   for the confidence weighting — no new per-bin state if shared
   (BinPhysics already has `flux` per the Past audit).
3. **Defer AR-Burg** prediction. Only revisit if listening tests show
   users can tell the difference. Don't pay the ~3% CPU and the
   implementation complexity speculatively.
4. **Defer neural prediction.** Wrong tool for one-frame look-ahead.
   No codec is built for our "frame N+1 from N" semantics; CNN
   inference at 8193 bins burns >30% of a core.
5. **DEAD ENDS:**
   - **Janssen 2.0** time-frequency AR audio inpainting — 200× slower
     than real-time on a modern desktop. The whole class of "fancy
     AR-in-STFT" methods is offline-only.
   - **Yule-Walker AR fitting** — declared actively dangerous by the
     Wharton tech note (poorly conditioned, can give unstable poles
     for short windows). Burg is the right AR estimator if AR is
     ever used.
6. **Document the failure mode in the UI.** When the
   confidence-weighted blend drops below 0.2, surface a small
   "transient detected — drying out" indicator near the slot. Helps
   producers understand why the effect "disappears" on drum hits.
7. **Shared infrastructure with Past and Punch** — same `PhaseRotator`
   helper + `if_offset[k]` cache in `ModuleContext` (see `13-past.md`
   Research findings).
