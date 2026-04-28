# Global Infrastructure for Next-Gen Modules

**Created:** 2026-04-26
**Status:** RESEARCH

> **§ 2 (History Buffer) status:** IMPLEMENTED — Phase 5b.1
> (`docs/superpowers/plans/2026-04-27-phase-5b1-history-buffer.md`).
> Other sections remain RESEARCH until their own phase ships.

This file collects every cross-cutting capability that more than one of the
next-gen modules needs, so the trade-offs get argued once instead of in each
module spec.

For every entry, the answer to "which existing spec covers this?" is given,
followed by what is *missing* from that spec or what changes once the new
modules pile on.

---

## 1. BinPhysics — per-bin persistent state across slots

**Existing spec:** `docs/superpowers/specs/2026-04-21-bin-physics-infrastructure.md`
(DEFERRED). Defines fields `velocity`, `mass`, `temperature`, `flux`,
`displacement`, `crystallization`, `phase_momentum`, plus mixing rules at
RouteMatrix sends.

### Why it is the linchpin

Six of the eight DEFERRED module specs (Circuit / Life / Kinetics / Past /
Modulate / Harmony) explicitly depend on it. Almost every analog-modeled,
physics-modeled, and history-modeled idea in the brainstorm needs at least
one of these fields. Without BinPhysics, every module ends up re-implementing
its own per-bin scratch state, and the user-facing claim "bins carry their
history through the chain" stops being true.

### What may need to change vs. the existing spec

The existing spec lists 7 fields. The newly-pulled ideas suggest at least
four more:

| New field | First needed by | Default | Meaning |
|---|---|---|---|
| `slew` | Circuit (slew-rate distortion), Life (oobleck) | 0.0 | Magnitude rate-of-change *limit*, not the rate itself. Modules can read this to know "this bin can only move this fast right now." |
| `bias` | Circuit (asymmetric-bias fuzz), DC offset detection | 0.0 | Time-averaged DC offset of the bin's complex value. |
| `decay_estimate` | Past (decay sorter), any "how long has this bin been ringing" question | 0.0 | Frames-to-fall-20-dB estimate, lazily updated. |
| `lock_target_freq` | Modulate (PLL), Kinetics (orbital), Harmony (chordification) | bin center freq | The frequency this bin is currently being pulled toward, so two modules don't fight. |

These are proposals — adding fields is cheap (the existing spec rule 3 says
adding a field only requires touching FxMatrix init/reset and the
modules that *use* the field).

### Ordering question

The existing BinPhysics spec mixes physics at RouteMatrix sends with
amplitude-weighted averaging. **Question:** is amplitude-weighting always
the right semantic? For `mass`, "the heavier of two parents wins" might be
more physical than "weighted average." For `crystallization`, "max" might
make more sense (crystallization is harder to break than to form). I think
the right answer is per-field merge rules, declared next to the field
definition. Worth a paragraph in the BinPhysics spec rewrite.

---

## 2. History Buffer — read-only rolling spectrum

**Existing spec:** `docs/superpowers/specs/2026-04-21-past-module.md`
section "History Buffer infrastructure (prerequisite)".

### What the existing spec says

A `HistoryBuffer` lives in `Pipeline`, written each hop after STFT, passed
read-only into `FxMatrix::process_hop()`. Initial size 256 frames (~3 s at
the default FFT). Max 30 s would be ~169 MB and is rejected for now.

### What the new ideas push on it

- **Future module** (proposed in this folder) needs *write-ahead* buffers,
  not history. Those are the module's own internal scratch — they should
  not live in `HistoryBuffer`. The existing Past spec already says
  Tape Print-Through uses its own buffer, not the shared one. Confirmed
  pattern: shared = read-only past; per-module = own writes.
- **Decay-sorter / history-feedback** ideas (#20 and the "20 more
  history-based" list in the ideas file) suggest *summary statistics over
  the buffer* are wanted by multiple modules — decay time per bin, RMS
  envelope per bin, bin-stability score. **Question:** should the
  HistoryBuffer expose pre-computed summary arrays (each ~MAX_NUM_BINS f32)
  to amortise the cost across modules, or is that premature? My instinct:
  yes, but lazy — compute on first read each block, not every hop.
- **Memory budget question.** 256 frames × 8193 bins × 8 bytes = ~17 MB
  for one channel. With Independent stereo, that doubles. With four aux
  sidechain STFTs, that x6. The existing spec implicitly assumes the
  history is only of the main signal — confirm that.

### Refactor implication

`Pipeline::process()` already has a tight ordering (STFT → FxMatrix →
delta monitor → publish). HistoryBuffer write goes between STFT and
FxMatrix — adds one `for k in 0..num_bins { history.frames[wp][k] = bins[k]; }`
loop per channel per hop. SIMD-friendly and trivially RT-safe.

---

## 3. Instantaneous Frequency (IF)

**Existing spec:** `docs/superpowers/specs/2026-04-21-harmony-module.md`
section "Instantaneous Frequency infrastructure (prerequisite)". Adds
`compute_instantaneous_freq()` to `src/dsp/utils.rs`.

### Who else needs it

- Harmony (all sub-effects) — original consumer
- Modulate (FM Network, PLL Tear) — explicitly named in the spec
- Past (decay sorter, optionally) — for "is this bin really ringing at its
  centre frequency, or is its IF drifting?"
- Kinetics (orbital phase) — needs to know the actual frequency of the
  loud peak that smaller peaks are orbiting
- Punch / new ideas — anything that talks about "the loudest stable
  partial" needs IF + an inter-hop stability filter

### What is missing from the spec

The existing spec gives the formula but does not say *where* the IF array
lives. It puts it inside the Harmony module struct. **Better:** put it in
`ModuleContext` (or a new `SpectralAnalysisContext`) so every module sees
the same array, and the cost is paid once per hop in `Pipeline` not once
per consuming module. This is the same pattern as `velocity` in
BinPhysics — a derived signal, not module state.

Cost: ~one `phase_delta` and one wrap per bin — cheap, SIMD-friendly.

---

## 4. Pitch & chord detection (Chromagram + harmony matrix)

**Existing spec:** `docs/superpowers/specs/2026-04-21-harmony-module.md` and
`docs/future-ideas/harmony effects.txt` (the architecture-recommendation
text).

### Confirmed plan from harmony effects.txt

- Chromagram via IF (no ML) — compress IF + magnitude into a 12-element
  pitch-class profile per hop.
- 12×12 harmony probability matrix in `src/dsp/harmony_weights.rs`,
  hardcoded music-theory weights as a starting point. Pluggable so a
  trained matrix from JSB Chorales / Nottingham can replace it later.

### Who else needs the chromagram

- Rhythm (the spectral-arpeggiator wants "loudest active partial")
- Modulate (Bin Swapper rhythmic trigger could be chord-change-driven)
- Geometry (a chord-driven Chladni pattern could be a sub-effect)

So the chromagram array (12 floats per hop) belongs in `ModuleContext`,
right next to IF. Compute cost is trivial (it is a sparse matrix multiply
on the magnitude array — see Method 2 in `harmony effects.txt`). The
matrix-multiplied chromagram itself is small enough that even
non-Harmony modules paying for it incurs almost zero cost.

### MIDI input: still open

Five+ ideas reference "tracks MIDI" or "use MIDI input" (Chordification,
Spectrum Stretch, Harmonic Companding, Rhythmic Phase Reset, gravity
wells). The current plugin is MIDI-less.

**This is the largest piece of new global infra.** It implies:

- Add MIDI event handling to `lib.rs` impl `Plugin::process()` —
  read `aux.midi.next_event()` and update a `held_notes: [bool; 128]`
  array in `SharedState`.
- Pass `held_notes` into `ModuleContext` or expose via a lock-free
  snapshot just like `route_matrix_snap`.
- Note: nih-plug already supports MIDI-only input on a CLAP plugin —
  it is a cargo-feature flag (`plugin = "..."` extras) plus declaring
  the port in `Plugin::midi_config()`. No DAW-side wiring beyond what
  the host already supplies.

**Question:** do we want polyphonic note-per-voice, or a flat
"these are the active pitch classes right now" 12-bool array?
For modules like Chordification the latter is sufficient and a
lot simpler. Per-voice (with per-voice ages, velocities, channel)
would only matter if we ever want a Spectral Arpeggiator that
distinguishes voices in chord. Recommendation: ship the 128-bool
held-notes + 12-bool pitch-class arrays first; revisit per-voice if
needed.

---

## 5. Host BPM sync

**Existing spec:** `docs/superpowers/specs/2026-04-21-rhythm-module.md`
section "Host BPM sync infrastructure (prerequisite)".

### What it adds to ModuleContext

`bpm: f32` and `beat_position: f64`. One-block-latency. Reads from
`ProcessContext::transport()` in nih-plug.

### Who else needs it

- Rhythm (entire module)
- Past (granular scan-position can be BPM-synced)
- Future (write-ahead delay length can be BPM-synced)
- Modulate (Bin Swapper rhythmic trigger; Phase Phaser animated rotation)
- Harmony (Shepard rotation in Formant Rotation — explicitly noted in spec)
- Kinetics (gravity-well sweep)
- Plus any "S/H Sync 1/16 Legato" via the Modulation Ring UI (see §8)

### Refactor implication

Trivial. `Pipeline::process()` already has access to `ProcessContext`.
Add two fields to `ModuleContext`. Document that
`beat_position` is the integer-and-fractional beat at the *start* of the
current block; modules wanting per-hop beat math compute
`beat_position + hop_index * hop_size / samples_per_beat`.

---

## 6. Cepstral analysis

**Currently:** not in any spec.

### What needs it

- Harmony / Formant Rotation — calculate the formant envelope before the
  pitch shift (existing spec: "calculates the spectral envelope (formants
  via cepstral smoothing)").
- Cepstral Liftering as its own sub-effect (idea #18). Kim's note: "need
  more info. Is this like DDSP?" — short answer: not really. DDSP is a
  trainable synth that *parameterises* a vocoder. Cepstral liftering is
  pure DSP — take the IFFT of the log-magnitude spectrum, edit the time-
  domain "cepstrum" (low quefrency = formants/timbre, high quefrency =
  pitch/harmonics), then re-FFT and exponentiate. The user can EQ the
  *envelope* of the sound without touching the *pitch*. Yes it's
  state-of-the-art for vowel morphing. No it does not need ML.

### Cost

One real-FFT and one real-IFFT per hop per channel, on a buffer the size
of the FFT. With `realfft` (already in deps), about as expensive as one
extra slot's worth of work. Acceptable for a single Harmony slot, not
acceptable to do automatically for every module.

**Recommendation:** lazy. Add a `cepstrum_buf: &[f32]` to
`ModuleContext` *only computed if any module declared
`needs_cepstrum: true` in its module spec.* Pipeline checks the
declaration once at slot-assignment time, not per block.

---

## 7. Kinetics as global function vs. as module

**Kim's note in the ideas file:**

> "About partial movement in general, quite many modules now have
> portamento. I'm thinking here could the plugin have just one 'kinetics'
> global function that could do basic linear and exponential portamento,
> quantised BPM-matched glissando, but also have feedback so movement has
> mass and hysteresis, resistance to change, etc?"

### My read

Two separate things are conflated. Both can be true at the same time:

1. **`BinPhysics.mass` / `velocity` / `displacement`** is the "global
   kinetics" function. It is the data substrate that every module shares,
   and the auto-velocity computation in FxMatrix is exactly the
   "feedback so movement has mass" Kim is describing.
2. **The Kinetics module** is the place where the user actively *shapes*
   that data — adds spring forces, sets per-band mass, opens gravity
   wells. It is one optional slot, not a global setting.

So the answer is: BinPhysics already *is* the global kinetics function in
disguise. The Kinetics module is the editing UI for it. No conflict, but
the BinPhysics spec rewrite should make this framing explicit, because
right now BinPhysics is presented as a passive store rather than a
physics simulator.

### What might still be missing globally

- **Per-bin portamento for the curve-derived multipliers themselves.**
  Today, when a curve changes (user drags a node), the new gain array is
  applied next block instantly. A "global kinetics" with mass would mean
  the multiplier glides toward its new target with mass-controlled inertia.
  This is independent of audio bin movement — it's parameter movement.
  **Question:** is this what Kim wants? It's a one-pole filter on
  `slot_curve_cache` updates. Cheap. Could be one global "kinetics smooth
  time" knob, or it could be Modulation-Ring per-curve. I think the
  Modulation Ring is the right answer (see §8).

---

## 8. Modulation Ring UI (S/H + Sync + Legato per control)

**Kim's note in the ideas file:** the "Modulation Ring Paradigm." Alt-click
or right-click any node or slider to reveal three tiny dots: [S/H], [Sync
1/16], [Legato], each toggleable.

### What this means concretely

For every automatable parameter (curve nodes, transforms, sliders), four
new sub-flags:

- **S/H** — sample-and-hold the current value at every BPM tick instead of
  reading the live value.
- **Sync 1/16** — quantise the modulation rate to 16th notes (default).
  User can change subdivision in a context menu.
- **Legato** — when the value changes, glide instead of jump.
- **Sensitivity** — how much "push" needs to accumulate before the held
  value updates (per Kim's "cumulative push" note). One-knob, ranged
  per parameter, hidden behind the ring.

### Refactor cost

Touches every parameter wrapper. Today the plugin has ~1341 generated
parameters (from the automation-presets work). Adding four flags per
parameter would be more than 5000 new fields in `Params`. That is too many.

**Suggestion:** make the Modulation Ring opt-in *per parameter category*,
not per parameter. e.g. all curve-node Y values can have the ring; the
matrix amount knobs cannot. The ring state is then one bitfield per
*category* + one global sensitivity, not per parameter. Cuts the surface
to ~30 ring-eligible categories.

**Question:** alternative — store ring state in a *separate* persisted
struct, not in nih-plug `Params`, so it does not pollute automation. The
ring itself is a hidden modulation source, not an automatable parameter.
This is closer to how Bitwig modulators work. Probably the right shape.

### Visual prior art

Bitwig modulator dots, Ableton "modulation router". The visual is solved.
The data model and the persistence are the work.

---

## 9. GPU / SIMD path (AVX-512, wgpu)

**Source:** the long architecture discussion at the bottom of
`ideas_for_the_wonderful_future.txt` (lines 281–335).

### Status

Not in any spec. Today the plugin uses the scalar `realfft` crate and
plain `for k in 0..num_bins` loops. `assert_process_allocs` enforces
RT-safety but does not enforce SIMD.

### The conclusion that file argues toward

For the heavy "circuit-modeled per-bin" effects (Circuit, Kinetics in
spring-network mode, Past with decay-sort), AVX-512 *with SoA layout* is
already enough to keep them under the OLA budget. wgpu compute shaders
become attractive if the user wants 8192 spring-coupled bins simulated
in parallel.

### What this means for our planning

- **No GPU required for shipping any of the next-gen modules.** The DSP
  fits in CPU SIMD.
- **Honour the SoA layout in `BinPhysics` from day one.** The existing
  spec already does this — `velocity: Vec<f32>`, not
  `Vec<BinPhysicsState>`. Stay disciplined.
- **Plan a `cargo feature = "wgpu-compute"` escape hatch.** Modules that
  *opt in* can dispatch their per-bin loop to a compute shader if the
  host supports it. The SpectralModule trait does not need to change —
  the module's `process()` would just route to a different kernel
  internally. Apple Silicon's unified-memory advantage (line 318) means
  Mac collaborators get the best version "for free" if/when this exists.
- **Always-bypassed heavy modules:** Kim's note "the heavier stuff can be
  always bypassed" — this is a UI affordance, not infra. Just a per-slot
  bypass with a "💀 EXPENSIVE" badge in the module popup.

### When to actually do GPU work

Not before the BinPhysics + module stack is shipping on CPU. GPU is an
optimisation answer to a problem we have not measured yet.

---

## 10. Reset-to-default / panic / recall

**Kim's note in the ideas file:** "Button to Reset plugin to default state."

Trivial. One button in the global header that calls
`Params::set_to_default()` for every parameter and clears every per-slot
state. One method per `SpectralModule` already exists: `reset(sample_rate,
fft_size)`. The Pipeline `reset()` already calls every module's `reset()`.
Wiring is already there.

Open question: do we want it to also reset the route matrix to serial
default, or only the curves and transforms? Probably yes to everything,
with a confirmation dialog.

---

## Summary table — what each next-gen module needs

| Module       | BinPhysics | History | IF | MIDI/Chroma | BPM | Cepstrum | Heavy? |
|--------------|:---:|:---:|:---:|:---:|:---:|:---:|:---:|
| Circuit      | yes | no  | no  | no  | no  | no  | yes  |
| Life         | yes | no  | no  | no  | no  | no  | med  |
| Kinetics     | yes | no  | yes | no  | yes | no  | yes  |
| Past         | yes | yes | yes | no  | yes | no  | yes  |
| Future (new) | yes | no¹ | yes | no  | yes | no  | med  |
| Harmony      | yes | no  | yes | yes | yes | yes | med  |
| Modulate     | yes | no  | yes | no  | yes | no  | med  |
| Rhythm       | no² | no  | yes | yes | yes | no  | low  |
| Geometry (new) | yes | yes³ | no | no  | no  | no  | yes  |

¹ Future has its *own* write-ahead buffer; it does not read the shared
HistoryBuffer.
² Rhythm could optionally use BinPhysics for sympathetic gating, but the
existing spec gets away without it.
³ Geometry uses HistoryBuffer for persistent-homology / structural
analysis; this is one of the strongest arguments for paying the
HistoryBuffer cost.

---

## Open infra questions Kim should answer first

1. **MIDI input:** ship it now (12-bool pitch-class array + 128-bool
   held-notes), or wait for the first module that *requires* it
   (Harmony / Chordification)? My vote: ship now, it's small.
2. **Cepstrum lazy compute:** OK to add `needs_cepstrum: bool` to
   `ModuleSpec`?
3. **Modulation Ring data model:** stored in `Params` (bloat) or in a
   separate `ModulationConfig` struct (cleaner, doesn't show in
   automation lanes)?
4. **HistoryBuffer:** 3 s default OK, or should it be larger from the
   start at the cost of ~17 MB?
5. **BinPhysics merge rules:** per-field override, or always
   amplitude-weighted as currently specced?
6. **Always-bypass heavy modules:** UI affordance only, or also a CPU
   guard (e.g. block enabling it if measured CPU > X)?
