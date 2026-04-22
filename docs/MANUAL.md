# Spectral Forge — User Manual

Spectral Forge is a spectral compressor and modular effects processor for Linux/Bitwig Studio.
It suppresses resonances, tames harsh frequencies, and controls the spectral balance of a mix —
similar in concept to Soothe2 — but built around a familiar parametric EQ-style drawing interface
and a modular slot-based routing system.

---

# Section 1 — Feature Reference

## Installation

1. Build the plugin:
   ```
   cargo run --package xtask -- bundle spectral_forge --release
   ```
2. Copy the bundle to Bitwig's CLAP path:
   ```
   cp target/bundled/spectral_forge.clap ~/.clap/
   ```
3. Restart Bitwig and rescan plugins. The plugin appears as **Spectral Forge** under CLAP.

> The plugin reports its current FFT size as latency to the host. Bitwig compensates
> automatically in timeline playback. The default FFT size is 2048 samples.

---

## The Concept

Where a regular compressor tracks the overall signal level, Spectral Forge compresses
independently across up to **8193 frequency bins** depending on the chosen FFT size
(default 1025 bins at FFT 2048). Each bin has its own envelope follower, gain computer,
and optional makeup stage.

The 7 **parameter curves** (threshold, ratio, attack, release, knee, makeup, mix) let you
sculpt how compression behaves across the frequency spectrum. A flat curve applies the same
value everywhere. A bell dip in the threshold curve means compression engages at a lower level
in that frequency range — so narrow resonances get caught without touching the rest of the signal.

Processing is organised into **slots**: up to 9 independently typed processing modules, routed
through a matrix. The Dynamics slot does the spectral compression; other slots add effects like
freeze, phase smear, gain shaping, mid/side processing, and more.

---

## Curve Editor

### Curve selector (top bar)

Seven buttons select which parameter curve is shown in the editor for the currently active slot.
Each slot has its own independent set of curves — switching slots also switches to that slot's curves.

| Button    | What it controls                                |
|-----------|-------------------------------------------------|
| THRESHOLD | Level (dBFS) at which compression begins        |
| RATIO     | Compression ratio (1:1 = off, 20:1 = limiting)  |
| ATTACK    | How fast gain reduction engages (ms)            |
| RELEASE   | How fast gain reduction releases (ms)           |
| KNEE      | Soft-knee width (0 = hard knee, wide = gentle)  |
| MAKEUP    | Per-bin makeup gain (Gain module only)          |
| MIX       | Dry/wet blend per bin (1.0 = fully wet)         |

Which buttons are active depends on the selected slot's module type. A Dynamics slot shows
THRESHOLD, RATIO, ATTACK, RELEASE, KNEE, and MIX. A Gain slot shows GAIN and SC SMOOTH.
Unavailable curves are greyed out.

### Editing curves

The curve editor shows a parametric EQ-style magnitude response that sets how the selected
parameter varies across frequency.

- **Neutral position** for all curves is the flat line through the centre: this applies the
  same value everywhere (set by the global sliders in the control strip).
- **Pulling a node down** in THRESHOLD lowers the threshold — more compression in that band.
- **Pulling a node down** in RATIO lowers the ratio — less compression (more gentle) in that band.
- **Pulling a node up** in MAKEUP (Gain slot) adds positive makeup gain to that band.

**Node interaction:**
| Action                                   | Effect                                |
|------------------------------------------|---------------------------------------|
| Drag node                                | Move frequency and gain               |
| Scroll wheel over node                   | Coarse Q (bandwidth) adjustment       |
| Hold both mouse buttons + drag up/down   | Smooth Q adjustment (500 px = full)   |
| Double-click node                        | Reset node to default position        |

Nodes at the far left and right are **shelves**; the four inner nodes are **bells** (Gaussian
shape in log-frequency space).

---

## Background Display

- **Spectrum gradient** — pre-FX signal (teal line) and post-FX signal (pink line) with a filled
  gradient between them showing the amount of processing.
- **Suppression stalactites** — per-bin gain reduction applied by the compressor, dropping
  downward from the top. Longer stalactites = more compression in that bin.

---

## Dynamics Control Strip

| Control      | Range        | Description                                                  |
|--------------|--------------|--------------------------------------------------------------|
| IN           | ±18 dB       | Input gain before STFT (smoothed)                            |
| OUT          | ±18 dB       | Output gain after STFT (smoothed)                            |
| MIX          | 0–100 %      | Global dry/wet                                               |
| SC           | ±18 dB       | Sidechain input gain                                         |
| **Dynamics** |              |                                                              |
| Atk          | 0.5–200 ms   | Global attack time (scaled per band by Freq curve)           |
| Rel          | 1–500 ms     | Global release time (scaled per band by Freq curve)          |
| Freq         | 0–1          | Frequency-dependent time scaling strength                    |
| Sens         | 0–1          | Sensitivity — how selectively peaks are targeted             |
| Width        | 0–0.5 st     | Gain-reduction smoothing radius (semitones)                  |
| **Threshold**|              |                                                              |
| Th Off       | ±40 dB       | Uniform vertical shift of the entire threshold curve         |
| Tilt         | ±6 dB/oct    | Spectral tilt of the threshold, pivoting at 1 kHz            |
| AUTO MK      | on/off       | Auto makeup gain — long-term GR compensation per bin         |
| DELTA        | on/off       | Delta monitor — hear only what is being removed              |

### Attack and Release

The time constants determine how quickly gain reduction follows level changes in each bin.

Global **Atk** and **Rel** set the baseline times. The **ATTACK** and **RELEASE** curves then
multiply those times per frequency bin — pulling a node up slows the time in that band, pulling
down speeds it up. The **Freq** knob adds an additional automatic scaling: at Freq=1, low
frequencies get proportionally slower times than high frequencies (matching the longer periods of
low-frequency content). At Freq=0, all bins use the global time unchanged.

Practical starting points:
- **Resonance control:** Atk 1–10 ms, Rel 50–150 ms. Fast enough to catch peaks, slow enough not to pump.
- **Spectral glue:** Atk 20–50 ms, Rel 100–300 ms. Slower times let more transient energy through.
- **De-essing:** Atk 1–3 ms, Rel 50 ms. Sibilants are short; release needs to reset between words.

### Sensitivity

Blends between absolute compression (0) and context-relative compression (1), where relative mode
only compresses bins that stick out above their local spectral neighbourhood.

Internally, the engine tracks a smoothed spectral envelope: a rolling average of each bin's local
spectral "floor" across its immediate neighbours. Sensitivity then raises each bin's effective
threshold by `sensitivity × max(0, envelope_db − threshold_db)` — meaning the threshold can only
go up, never down. At sensitivity=0, every bin above the drawn threshold compresses regardless of
context. At sensitivity=1, a bin sitting at the same level as its neighbours has its threshold
raised to match — it won't compress unless it sticks out above the local floor.

Practical settings:
- **0.0** — pure absolute compression; the spectral shape of the material doesn't influence which bins are hit. Use for transparent full-spectrum compression or bus glue.
- **0.3–0.5** — partial selectivity; broadband content is compressed somewhat less than tonal peaks. A good starting point for resonance control on most material.
- **0.8–1.0** — surgical/Soothe-like; only bins that genuinely protrude above their neighbours trigger gain reduction. The spectral tilt and overall level are mostly preserved; individual resonances and harsh peaks are caught.

### Width

Blurs the computed gain reduction across a musical interval around each bin, preventing
per-bin artifacts at the cost of frequency precision.

After the gain reduction for each bin is computed, Width averages each bin's GR with its
neighbours over a window of ±Width semitones in log-frequency space. Because this window is
defined in semitones, it covers the same musical interval at every frequency — half a semitone at
100 Hz spans a different number of bins than at 5 kHz, but it represents the same musical distance
at both. At Width=0, each bin retains its individual GR value (maximum precision). Wider values
produce smoother, more "multiband-like" transitions.

Width and Sensitivity are applied sequentially: Sensitivity shapes which bins get gain reduction,
then Width blurs that shape across frequency. Setting high sensitivity + narrow width targets
individual resonances precisely. High sensitivity + wide width catches the same resonances but
spreads the GR into neighbouring bins — useful when you want a resonance suppressed but the
transition to be less obvious.

Practical settings:
- **0.0 st** — bin-exact GR; can sound phasey or grainy on pitched material. Use when you need to kill a single narrow resonance and artefacts aren't audible.
- **0.05–0.1 st** — a small musical blur that removes per-bin graininess while keeping frequency precision high. Good default for most resonance work.
- **0.2–0.3 st** — noticeable smoothing; adjacent semitones get similar treatment. Suitable for de-essing and taming frequency-range harshness.
- **0.4–0.5 st** — broad smoothing approaching a half-semitone band. Useful for spectral glue and bus compression where precision is less important than smoothness.

### Threshold Offset and Tilt

**Th Off** shifts the entire threshold curve up or down without changing its shape — useful for
quickly adjusting how much material is caught once the curve shape is dialled in.

**Tilt** rotates the threshold around 1 kHz. Positive values raise the threshold toward high
frequencies (compress treble less and low-mids more); negative values do the opposite. This is
a faster alternative to manually drawing a sloped threshold curve when you want frequency-
proportional compression.

### Auto Makeup

Applies long-term per-bin gain compensation equal to the average gain reduction being applied.
The averaging window is approximately 1 second, so transient peaks don't affect the compensation.
The result is that the overall spectral shape and perceived loudness remain roughly constant even
as compression settings change. Enable it for transparent processing where you don't want to
re-balance levels manually.

### Delta Monitor

Routes only the portion of the signal being removed (input minus output) to the output. Use this
to hear what the plugin is doing in isolation: if you hear mostly resonance and harshness, the
settings are working as intended. If you hear pitched content or musical detail, the threshold is
probably too low or the ratio too high.

---

## Threshold Modes

| Mode     | Behaviour                                                                        |
|----------|----------------------------------------------------------------------------------|
| Absolute | Threshold is a fixed dBFS level. Works like a normal compressor per bin.        |
| Relative | Detection normalised against the local spectral envelope. Only bins that stick out above their neighbours trigger compression — the spectral shape is preserved while resonances are caught. |

**Relative mode** is the "Soothe-like" mode: it leaves the spectral tilt alone and only
compresses peaks relative to the local context. The **Sens** knob blends between absolute
(0.0) and fully relative (1.0). In Absolute mode the Sens knob still functions — it is not
locked to 0 — giving fine control over the blend in either mode.

---

## Stereo Link Modes

| Mode        | Behaviour                                                                 |
|-------------|---------------------------------------------------------------------------|
| Linked      | Both channels share the slot chain — same processing applied to L and R   |
| Independent | L and R run through the same slots independently; each has its own state  |
| MidSide     | Encode L/R to M/S before processing, decode after — compress mid and side separately |

**MidSide** is useful for controlling low-end weight (mid) and stereo width (side) independently.
In MidSide mode, individual slots can be targeted to Mid only, Side only, or both via their
channel target setting.

---

## Sidechain

Spectral Forge accepts one stereo sidechain (SC) input. In Bitwig, route any track's output to the plugin's sidechain input as usual.

### Which modules use the sidechain?

| Module          | SC role                                                                  |
|-----------------|--------------------------------------------------------------------------|
| **Dynamics**    | External detector for per-bin gain reduction.                            |
| **Gain**        | All four modes use SC — see Gain Modes below. Pull and Match apply a per-bin peak-hold; Add and Subtract combine SC with the GAIN curve instantaneously. |
| **Phase Smear** | Modulates per-bin smear amount by SC magnitude, smoothed by PEAK HOLD curve. |
| **Freeze**      | Gates the freeze threshold; louder SC raises effective threshold.        |

Other modules (Contrast, Mid/Side, T/S Split, Harmonic) do not use the sidechain and show no SC controls.

### Per-module SC controls

Each SC-aware module panel carries:

- **SC gain** (−∞ to +18 dB) — level applied to the SC signal for *this slot only*. −∞ disables SC for the slot.
- **SC source** — which channel of the stereo SC signal the slot keys off:

| Choice    | Behaviour                                                                  |
|-----------|----------------------------------------------------------------------------|
| **Follow**    | Routes the SC channel matching whatever the slot is currently processing. See table below. |
| **L+R**       | Sum of SC left and right.                                                  |
| **L**         | SC left channel only.                                                      |
| **R**         | SC right channel only.                                                     |
| **M**         | Mid (L+R)/√2 of the SC.                                                    |
| **S**         | Side (L−R)/√2 of the SC.                                                   |

### Follow semantics

| Stereo Link    | Follow resolves to                                                        |
|----------------|---------------------------------------------------------------------------|
| Linked         | L+R                                                                       |
| Independent    | Channel-paired: main L → SC L, main R → SC R                              |
| Mid/Side       | Target-paired: Mid-target slot → SC M, Side-target slot → SC S, All-target slot → L+R |

To duck the mids by the sides of the SC input: route a stereo SC, target the slot to Mid, set SC source to **S**.

### SC level indicator

The small yellow bar in the top bar (right of Falloff) lights up when the plugin is receiving audio on the SC input. If the bar stays dim while playing your project, the SC isn't reaching the plugin — check your host routing.

### Gain modes

The Gain module has four modes, selectable via the Mode row beneath the curve area. All four use the SC input; what differs is how it's combined with the drawn GAIN curve and the main signal.

| Mode         | Formula (per bin)                                    | Temporal behaviour         |
|--------------|------------------------------------------------------|----------------------------|
| **Add**      | `bins *= g + sc`                                     | Instantaneous, no envelope |
| **Subtract** | `bins *= max(g - sc, 0)`                             | Instantaneous, no envelope |
| **Pull**     | `target_mag = main_mag * g + sc_env * (1 - g)`       | Per-bin peak-hold on SC    |
| **Match**    | `bins *= g + (1 - g) * ERB_smoothed(sc_env / main)`  | Per-bin peak-hold on SC    |

`g` is the per-bin value of the GAIN curve; `sc` is the gained SC magnitude after per-module SC gain and channel selection; `sc_env` is that SC run through a per-bin peak follower whose release time is set by the PEAK HOLD curve.

**Add / Subtract** — `g` is a dB gain (neutral = 0 dB). Draw above neutral to boost, below to cut; SC is added (Add) or subtracted (Subtract) on top.

**Pull** — `g` is a wet/dry morph clamped to `[0, 1]`. At `g = 1` the main signal passes through; at `g = 0` the main's magnitude is replaced with the peak-held SC magnitude bin-for-bin. Pull is a magnitude-swap / cross-synthesis tool — harmonic peaks in the main get flattened to the SC's per-bin shape. Great for sound-design morphs, not for timbre matching.

**Match** — `g` is a wet/dry mix clamped to `[0, 1]`, same meaning as Pull. At `g = 0` the module applies a smooth per-bin EQ curve derived from the ratio of ERB-smoothed SC to ERB-smoothed main (log-domain, re-exponentiated). Unlike Pull, the main's narrow harmonic peaks are preserved — Match only shifts the broad spectral balance toward the SC. Boost/cut is clamped to ±12 dB.

### GAIN / MIX curve (context-dependent)

The first curve retitles itself by mode — both on the tab button and the "Editing: …" header — so the label matches what the curve actually controls:

- **Add / Subtract** → the curve is labelled **GAIN** and the hover tooltip shows dB values.
- **Pull / Match** → the curve is labelled **MIX**, because `g` is a clamped `[0, 1]` wet/dry, not a dB gain. The tooltip shows the effective mix percentage (`X% dry · Y% pull` or `match`). Draw above neutral = 100% dry (clamped); draw below neutral to apply the effect.

The second curve — **PEAK HOLD** — sets per-bin peak-hold time (1 ms to ~500 ms, log). It is live in **Pull** and **Match** (which both run the SC through the per-bin peak follower) and grayed/disabled in Add and Subtract. Longer hold prevents pumping on percussive SC material; shorter hold tracks detail.

### Live SC envelope overlay

When a Gain slot is selected for editing, a thin darker line is drawn behind every curve showing the live per-bin SC magnitude the module is currently seeing (post per-module SC gain, pre peak-hold). The overlay uses the same dBFS reference as the pre/post spectrum display, so a unit sine on the SC sits at roughly 0 dB — matching main and SC on the graph now means matching their actual levels. This is the signal Add/Subtract directly combine with the GAIN curve, and the input to the peak follower in Pull/Match.

### Layout stability

The per-module SC strip (SC gain · source selector) only has meaning for SC-aware modules (Dynamics, Freeze, Phase Smear, Gain). On other modules the strip is rendered invisibly so the tilt/offset row below keeps its vertical position; switching between modules does not shift surrounding controls.

---

## Slot Routing Matrix

The routing matrix shows all 9 slots. Slot 8 is always the **Master** output.

- **Diagonal cells** show the module type for each slot. Click to select that slot for editing.
- **Off-diagonal cells** set the send amplitude from one slot to another (0.0 = no connection,
  1.0 = full). The default routing is serial: slot 0 → 1 → 2 → Master.
- **Lower triangle** (row > column): forward sends (current hop).
- **Upper triangle** (row < column): feedback sends (one-hop delayed).

Nothing is routed to Master unless explicitly connected. If no sends reach slot 8, the output is silence.

### Module types

| Type              | Description                                               |
|-------------------|-----------------------------------------------------------|
| Dynamics          | Spectral compressor/expander with 6 parameter curves      |
| Freeze            | Spectral freeze — holds the current FFT frame             |
| Phase Smear       | Per-bin phase randomisation                               |
| Contrast          | Spectral contrast enhancer — boosts peaks, cuts valleys   |
| Gain              | Per-bin gain shaping (Add / Subtract / Pull / Match modes) |
| Mid/Side          | M/S balance, expansion, phase decorrelation               |
| T/S Split         | Transient/Sustained spectral split                        |
| Harmonic          | Harmonic emphasis                                         |
| Master            | Terminal output slot (slot 8, always present)             |

---

## FFT Size

The FFT size sets the frequency resolution and the latency:

| FFT Size | Bins | Bin width @ 44.1 kHz | Latency   |
|----------|------|----------------------|-----------|
| 512      | 257  | ~86 Hz               | ~12 ms    |
| 1024     | 513  | ~43 Hz               | ~23 ms    |
| 2048     | 1025 | ~21 Hz               | ~46 ms    |
| 4096     | 2049 | ~11 Hz               | ~93 ms    |
| 8192     | 4097 | ~5 Hz                | ~186 ms   |
| 16384    | 8193 | ~3 Hz                | ~371 ms   |

Larger FFT sizes give better low-frequency resolution at the cost of higher latency.
For live performance use 512 or 1024; for mix work 2048–4096 is typical.

At 512 samples, bin width is ~86 Hz — two adjacent bins span most of a minor third at 200 Hz,
so narrow resonances in the low-mids may not be precisely resolvable. At 4096, the same region
has ~11 Hz bins, making it possible to isolate a single harmonic partial.

---

## Typical Workflows

### Tame a resonant instrument

1. Put Spectral Forge on the instrument channel.
2. Select **THRESHOLD**, pull the node near the resonance frequency **down** (lower threshold → more compression there).
3. Use **Relative** threshold mode so the plugin only responds to peaks relative to the instrument's own spectral shape.
4. Enable **DELTA** to hear what is being removed. You should hear the resonance in isolation. Disable DELTA when satisfied.

### De-ess a vocal

1. Put Spectral Forge on the vocal.
2. Select **THRESHOLD**, pull down in the 4–10 kHz region.
3. Set a fast **Atk** (1–5 ms) and medium **Rel** (50–100 ms).
4. Increase **Ratio** globally or boost it with the RATIO curve in the sibilance range.
5. Use **DELTA** to verify you're catching sibilants, not consonants.

### Spectral glue on a bus

1. Put Spectral Forge on a bus (drums, mix bus).
2. Leave all curves flat (neutral).
3. Use a gentle ratio (2:1–4:1) and moderate attack/release.
4. **Linked** mode for consistent stereo image; **MidSide** if you want to leave the stereo width alone.
5. Enable **AUTO MK** so average loudness is preserved.

### Frequency-targeted sidechain duck

1. Put Spectral Forge on a pad or synth.
2. Route the kick or bass to aux sidechain input 1.
3. Assign slot 0 (Dynamics) to sidechain 1 in the routing matrix.
4. Pull down **THRESHOLD** at the frequencies that clash (e.g. 60–200 Hz).
5. Keep other frequencies at neutral threshold — the duck only happens where the sidechain is loud.

### Mid/Side processing

1. Set **Stereo Link** to **MidSide**.
2. Add a Dynamics slot targeting **Mid** and a separate Dynamics slot targeting **Side**.
3. Shape the threshold curves independently for each component.

---

## Test Files (test_flac/)

Included test files for evaluating the plugin:

| File                                              | Contains                                   | What to test                          |
|---------------------------------------------------|--------------------------------------------|---------------------------------------|
| `breakbeat_4030hz_bell-curve-high-q_resonance`    | Sharp bell resonance at 4030 Hz            | Narrow threshold dip at 4 kHz        |
| `breakbeat_kick_resonance`                        | Kick with ring/resonance artefact          | Low-mid threshold dip, fast attack   |
| `breakbeat_sweep-high-q-200hz_to_4khz_resonance`  | Sweeping resonance 200 Hz → 4 kHz         | Relative mode tracking a moving peak |
| `chord-brillant-resonance-attack-decay-sweep`     | Chord with attack/decay resonance sweep    | Attack/release curve shaping         |
| `saw_filter_decay`                                | Sawtooth with filter decay                 | Temporal response of envelope follower |

Load into Bitwig, insert Spectral Forge, and enable **DELTA** while adjusting the threshold
curve to isolate the resonance you want to remove.

---

# Section 2 — Algorithm Reference

This section documents the DSP algorithms and the reasoning behind design choices.

---

## STFT Architecture

The plugin uses an overlap-add STFT with 75% overlap (4× overlap factor), a Hann analysis
window, and Hann² OLA normalisation. This gives perfect reconstruction of the unmodified signal
and keeps the STFT smearing artefacts low enough that per-bin processing sounds natural at typical
settings.

Hop size = FFT size / 4. The normalisation constant is `2 / (3 × FFT size)`, which corrects for
the Hann² power reduction in the OLA sum.

The STFT latency (FFT size samples) is reported to the host as plugin latency. Bitwig compensates
automatically in timeline playback by delaying other tracks. In live performance this latency is
unavoidable — choose the smallest FFT size that gives adequate frequency resolution for the task.

---

## Spectral Envelope Tracker

The sensitivity system requires a per-bin estimate of the local spectral "floor" — the level a
bin would have if the signal were purely broadband at that frequency.

Each hop, a 3-tap median of each bin and its two immediate neighbours is computed. This median is
then fed into a one-pole lowpass filter with a 50 ms time constant. The median step removes
short-duration peaks (transients, tonal spikes) before smoothing — without it, a single loud bin
would bias the envelope estimate upward and reduce sensitivity in its neighbourhood.

The 50 ms time constant is a deliberate choice: fast enough to track the overall spectral shape as
it evolves with the material (e.g. a chord change), but slow enough to ignore individual
transients. A faster envelope would be biased by the very peaks we want to detect; a slower one
would not adapt when the spectral context changes.

---

## Gain Computer

The gain computer maps the envelope follower output (in dBFS) to gain reduction (in dB) using a
standard soft-knee compressor characteristic:

- Below `threshold − knee/2`: no gain reduction.
- In the knee region `[threshold − knee/2, threshold + knee/2]`: quadratic interpolation between 0 and full-ratio GR.
- Above `threshold + knee/2`: `GR = (level − threshold) × (1 − 1/ratio)`.

The effective threshold passed to the gain computer is modified by the sensitivity system before
this calculation (see below).

---

## Sensitivity

Sensitivity raises the effective threshold per bin by an amount proportional to how much the
local spectral envelope exceeds the drawn threshold:

```
envelope_excess = max(0, envelope_db − threshold_db)
effective_threshold = threshold_db + sensitivity × envelope_excess
```

The `max(0, …)` clamp means the envelope can only raise the effective threshold, never lower it.
This prevents sensitivity from making compression more aggressive than the drawn threshold would
suggest — it only makes it more selective.

At sensitivity=1: if a bin's neighbourhood is already 10 dB above the threshold, that bin's
effective threshold is raised by 10 dB. The bin must be 10 dB above its neighbours (not just
10 dB above the threshold) to trigger compression.

At sensitivity=0: the envelope is ignored entirely and the drawn threshold is used directly.

This formulation is a continuous blend rather than a mode switch, so intermediate values are
useful — they partially account for local context without requiring bins to fully "stand out."

---

## Attack and Release — Frequency Scaling

Global attack and release times are modified per bin by two mechanisms:

**ATTACK/RELEASE curves:** Each bin's time = `global_time × curve_gain_at_bin`. Pulling the curve
above centre slows that band; pulling below speeds it up. The curve gain is a linear multiplier,
so the ATTACK curve at +18 dB (gain ≈ 8×) makes that band's attack ~8× slower than global.

**Frequency scaling (Freq knob):** Adds an automatic adjustment proportional to the inverse of
frequency. The scaling factor for bin `k` at frequency `f` is `(1000 / f)^(Freq × 0.5)`. At
Freq=1 and 100 Hz, this gives roughly a 3× multiplier relative to 1 kHz. The choice of 0.5 as
the exponent limits the maximum range to avoid extreme values at very low frequencies. The pivot
at 1 kHz (1000 Hz) is arbitrary but keeps mid-range content unscaled, which matches the intuition
that 1 kHz is "normal."

Frequency scaling is applied after the curve multiplier, so both mechanisms combine multiplicatively.

---

## Gain Reduction Smoothing (Width)

After per-bin GR is computed, it is averaged across a log-frequency window of ±Width semitones.
This is implemented with a prefix sum for O(n) cost regardless of window size:

1. Build prefix sum of the GR array.
2. For each bin `k`, find `k_lo = floor(k / 2^(w/12))` and `k_hi = ceil(k × 2^(w/12))`.
3. The smoothed GR for bin `k` is the mean of `gr[k_lo..k_hi]` via a single prefix-sum range query.

The window boundaries `k / 2^(w/12)` and `k × 2^(w/12)` maintain a constant musical interval in
both directions from `k`. A 0.1 semitone window spans the same musical distance at 200 Hz and at
8 kHz, even though it covers a different number of FFT bins at each frequency. This is why Width
is expressed in semitones rather than Hz or bins.

The prefix-sum approach was chosen because the alternative (a convolution or running average) is
either O(n × window_size) or requires a variable-width kernel that can't be precomputed. The
prefix-sum runs in constant time per bin regardless of window size.

---

## Auto Makeup

Auto makeup tracks the long-term average smoothed gain reduction per bin with a ~1000 ms time
constant, then applies `−average_GR` as makeup gain. The long window means transient peaks don't
affect the compensation — only sustained compression contributes.

The compensation is applied to `smooth_buf × mix` (the GR actually reaching the output, not the
raw GR before mix), so the compensation is exact at any dry/wet setting.

---

## Per-Bin SIMD Gain Application

The final pass (applying smoothed GR, makeup, auto-makeup, and mix to each bin) is
runtime-dispatched via the `multiversion` crate:

- **AVX2 + FMA** (Haswell and later): 8 complex bins per SIMD instruction.
- **SSE4.1 fallback**: 4 bins per instruction.
- **Scalar fallback**: 1 bin per iteration.

The dispatch happens once at startup based on CPUID flags. No runtime branching inside the loop.

---

## FxMatrix and Slot Routing

`FxMatrix::process_hop` processes all 9 slots in forward order (0 → 8) each STFT hop. For each
slot `s`:

1. Build the slot's input by summing `send[src][s] × slot_out[src]` for all `src < s`.
2. Also accumulate from virtual rows (T/S Split outputs) for `src_slot < s`.
3. Call the module's `process()`.
4. Store the output in `slot_out[s]`.
5. If the module exposes virtual outputs (T/S Split), copy them into `virtual_out[v]`.

Slot 8 (Master) is handled as the final mix: all `send[src][8]` sums are accumulated into the
output buffer. If nothing routes to slot 8, the output is silence (no implicit passthrough).

Module types are synced from UI params at the top of each audio block via `sync_slot_types()`,
which uses `permit_alloc()` to allow allocation when a module type changes. This runs only when
the user changes a slot, not every hop.

---

## Real-Time Safety

- No allocation on the audio thread except inside `permit_alloc()` blocks (module type changes only).
- No locks on the audio thread. Curves are read via lock-free triple-buffer. Route matrix and slot params use `try_lock()` with a fallback that skips the update if the GUI holds the lock.
- No I/O on the audio thread.
- `flush_denormals()` sets FTZ+DAZ CPU flags each block to prevent denormal slowdowns in the envelope follower's one-pole filters.
- `assert_process_allocs` (nih-plug feature) will abort if the audio thread allocates unexpectedly.

---

## Known Limitations

- Linux and Windows, CLAP only. No macOS, VST3, or AU support (a collaborator provides signed Mac builds separately).
- The lookahead parameter is reserved for a future implementation; the STFT latency itself provides effective transient anticipation.
- The GUI requires OpenGL (egui via nih-plug-egui). Some headless environments won't open the editor.
- The T/S Split virtual row routing is implemented in the audio engine but not yet exposed in the routing matrix GUI.
