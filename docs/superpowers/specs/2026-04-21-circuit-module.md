> **Status (2026-04-24): DEFERRED.** Depends on BinPhysics infrastructure (also DEFERRED). Not yet implemented. Source of truth: [../STATUS.md](../STATUS.md).

# Circuit Module — Design Spec

**Status:** Planned  
**Plan:** (to be written — Plan 5)  
**Depends on:** BinPhysics Infrastructure (Plan 1)

## What it is

A spectral module modeling cheap 1970s analog hardware components — per bin, as if each FFT bin were built from failing capacitors, saturating transformers, and non-linear photoresistors. All effects are implemented as SIMD-friendly 1-pole IIR operations on flat float arrays. Reads and writes `flux` and `temperature` in BinPhysics; other modules later in the chain see that history.

## Sub-effects

### Vactrol — Opto-Isolator Smoothing
Models a Buchla low-pass gate per bin. Attack: instant (LED lights immediately). Release: multi-stage non-linear decay (photoresistor slowly regaining resistance). High-frequency bins get slower vactrols — drum loops become liquid, ringing "bong" sounds. State: `vactrol_level[N]` (f32 per bin) inside module struct. Reads: magnitude. Writes: output magnitude.

### Schmitt — Hysteresis Gate
Two thresholds per bin: `on_threshold` and `off_threshold`. Bin stays muted until magnitude crosses on_threshold, then stays open until it drops below off_threshold. Great for extracting punchy transients with long natural reverb tails. State: `latch[N]` (bool per bin). Does not use BinPhysics fields.

### Transformer — Flux Saturation
Bins have a `flux` variable (from BinPhysics). Sustained high amplitude saturates the virtual transformer core — compressing magnitude and introducing slight phase rotation. After signal stops, the core slowly demagnetizes. Consecutive loud hits sound progressively darker. Reads/writes: `physics.flux`.

### BBD — Bucket-Brigade Delay
A delay line for bin magnitudes. Each "bucket" (delay stage) applies a 1-pole lowpass filter and adds a tiny amount of noise. High feedback produces the classic dark, murky, self-oscillating wash of a Boss DM-2, applied per-frequency. State: `bucket[N][stages]` (f32 ring buffer per bin) inside module struct. Note: per-bin delay buffers must be pre-allocated at module creation (permit_alloc) — not on the audio thread.

### Power Sag — Global Starvation
A global feedback loop. Total RMS of the spectrum is computed. If a massive sub-bass hit occurs, the virtual power supply "sags" — globally reducing available headroom for all high-frequency bins. Creates a pumping, breathing modulation mimicking a dying amplifier. Does not use BinPhysics. Output-only magnitude scaling.

### Component Drift — Temperature-Dependent Offset
At module init, each bin receives a static random micro-offset to its amplitude (seeded from bin index for reproducibility). A very slow global LFO (modeling room temperature) causes these offsets to drift over minutes. Reads/writes: `physics.temperature` (accumulated heat → drift rate). Writes bin magnitude with offset.

### PCB Crosstalk — Capacitive Coupling
A percentage of a bin's magnitude bleeds into adjacent bins, but only when the phase difference between them is high. Mimics capacitive coupling. Reads bin phases; writes adjacent bin magnitudes. No BinPhysics fields required.

### Slew Distortion — Op-Amp Overload
If a bin's magnitude tries to change more than a maximum rate between hops, the virtual op-amp "struggles" — it restricts the change and injects phase-scramble (noise) proportional to the excess slew. Reads: previous magnitudes (module-internal). Writes: output magnitude + phase scramble.

## Curves (4 total)

| Idx | Label | Used by |
|---|---|---|
| 0 | AMOUNT | All modes — depth of effect |
| 1 | THRESHOLD | Schmitt (on_threshold), Transformer, Slew |
| 2 | RELEASE / DECAY | Vactrol, Transformer, Flux |
| 3 | MIX | All modes |

## Implementation notes

- All states (vactrol_level, latch, bucket buffers) live inside the module struct as pre-allocated `Vec<f32>` or `Vec<bool>`, sized at `reset()` time — no audio-thread allocation
- BBD: number of stages is a fixed const (e.g. 4) — variable stages would require dynamic allocation
- Non-linear curves (vactrol decay, transformer saturation) implemented as polynomial approximations — no `exp()` or `sin()` in inner loops
- `flux` in BinPhysics persists across slots: a Transformer instance in slot 1 saturates flux, and a downstream Kinetics or Life module in slot 5 can respond to it
