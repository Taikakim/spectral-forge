# Spectral Forge

0.15 is now merged to the Master branch. An extensive update with dozens of new processors, modular routing, presets, host automation, a new more modular codebase, etc. I will tweak the things before a release since some of the new processors still lack DSP, and none of them have really had their parameter ranges and DSP tuned for usability and interesting outcomes. Work with the missing DSP for some of the new modules will go on in parallel in the dev branch.

A spectral compressor and modular effects processor for Linux/Windows, implemented as a CLAP and VST3 plugin. Designed for Bitwig Studio.

Patent-safe design — does not use the Hilbert/convolution approach from oeksound's patents.

The dynamics section is functional. The modular multi-fx routing system is under active development — expect things to be unfinished or experimental on the main branch. If you just want to use this, stick to the releases.

Contributors and AI assistants: the design specs and implementation plans under `docs/superpowers/` include historical, in-progress, and deferred work. Check [`docs/superpowers/STATUS.md`](docs/superpowers/STATUS.md) before following any of them — several are SUPERSEDED.

---

<img width="898" height="454" alt="Screenshot_20260421_115617" src="https://github.com/user-attachments/assets/12213421-60ca-4178-ade3-b7bba5575f09" />

## Building and installing

**Requirements:** Rust stable toolchain, Cargo. `clap-validator` is optional for testing.

```bash
# Debug build
cargo build

# Release build
cargo build --release

# Bundle as .clap and .vst3
cargo run --package xtask -- bundle spectral_forge --release

# Install to Bitwig's default search path
cp target/bundled/spectral_forge.clap ~/.clap/
```

After installing, rescan plugins in Bitwig (or restart). The plugin appears as **Spectral Forge** under CLAP effects (or VST3, if you installed the `.vst3` bundle instead).

---

## Quick start

1. Insert Spectral Forge on any audio track.
2. Play audio through it — the spectrum display will show your signal in real time.
3. The threshold curve (selected by default) controls where compression begins. Drag nodes to shape the threshold across the frequency range.
4. Increase the **Ratio** curve to set compression depth per frequency band.
5. Use **Attack** and **Release** curves to control how fast each band responds.

---

## Interface overview

### Top bar (row 1 — curve selectors)

Adaptive curve selector buttons show the curves available for the **currently selected slot**. A Dynamics slot shows THRESHOLD / RATIO / ATTACK / RELEASE / KNEE / MIX; a Gain slot shows GAIN / PEAK HOLD; a Freeze slot shows LENGTH / THRESHOLD / PORTAMENTO / RESISTANCE / MIX; and so on. Clicking a button makes that curve active for editing.

To the right: **Floor** and **Ceil** drag-values set the dBFS range of the spectrum display. **Falloff** sets the peak-hold decay time in ms.

### Top bar (row 2 — FFT and scale)

**FFT** buttons (512 / 1k / 2k / 4k / 8k / 16k) set the FFT window size, trading frequency resolution against latency. **Scale** buttons (1× – 2×) set the UI zoom.

### Curve display

The large centre area shows:

- **Spectrum gradient** — the pre-FX signal (teal line) and post-FX signal (pink line) with a filled gradient between them showing the amount of processing.
- **Response curves** — coloured polylines for each of the current slot's curves. The active curve is brighter; inactive ones are dimmed.
- **Node handles** — only for the selected curve. Circles for bell-type nodes; triangles (▶ / ◀) for shelf nodes.
- **Graph header** — top-left overlay reads "Editing: {slot name} — {channel target}". Click it to rename the slot.

### Node interaction

| Action | Effect |
|--------|--------|
| Drag node | Move frequency and gain |
| Scroll wheel over node | Coarse Q (bandwidth) adjustment |
| Hold both mouse buttons + drag up/down | Smooth Q adjustment (500 px = full range) |
| Double-click node | Reset node to default position |

### Bottom strip — control knobs

**SC strip (per slot, sidechain-aware modules only):** an SC gain value (−90 to +18 dB; −90 dB is shown as −∞ and switches the sidechain off for that slot) and a Source selector (Follow / L+R / L / R / M / S) set how much of the stereo sidechain, and which part of it, the current slot keys off.

**GainMode (Gain slots only):** Add / Subtract / Pull / Match buttons appear when a Gain module is selected. All four combine the slot's sidechain with the main signal:

- **Add** / **Subtract** — the GAIN curve is a per-bin dB gain; the sidechain magnitude is added to it (Add) or subtracted from it (Subtract), instantaneously.
- **Pull** — the GAIN curve (relabelled MIX) morphs each bin's magnitude toward the peak-held sidechain magnitude (cross-synthesis). The PEAK HOLD curve sets the hold time.
- **Match** — the GAIN curve (relabelled MIX) blends in a smooth EQ that shifts the main signal's broad spectral balance toward the sidechain's, while keeping the main signal's harmonic detail. Boost/cut is limited to ±12 dB; the PEAK HOLD curve sets the hold time.

See the Gain modes section of [docs/MANUAL.md](docs/MANUAL.md) for the per-bin formulas.

**Global row:**

| Control | Range | Description |
|---------|-------|-------------|
| IN | ±18 dB | Input gain |
| OUT | ±18 dB | Output gain |
| MIX | 0–100 % | Global dry/wet |
| AUTO MK | on/off | Auto makeup gain — long-term GR compensation |
| DELTA | on/off | Delta monitor — hear only what is being removed |
| CLIP | on/off | Master output clipper |
| THR | −24 to 0 dB | Master clipper threshold |

**Dynamics row (Dynamics and Contrast slots) + per-curve row (active curve of any slot):**

| Control | Range | Description |
|---------|-------|-------------|
| Atk | 0.5–200 ms | Global attack time base |
| Rel | 1–500 ms | Global release time base |
| Sens | 0–1 | Sensitivity — how selectively peaks are targeted |
| Width | 0–12 st | Gain-reduction blur radius, in semitones each side of every bin (12 st = one octave each side) |
| Offset | ±1 | Additive offset for the active curve (value shown in the curve's own units) |
| Tilt | ±1 | Spectral tilt for the active curve, pivoting at 1 kHz (±1 = ±4 dB/oct) |
| Curv | 0–1 | Curvature of the Tilt: 0 = straight slope; 1 = S-shaped slope, steepest in the mid-range and flattening toward the lowest and highest frequencies. No effect when Tilt is 0 |

**DELTA** is the fastest way to verify the plugin is targeting what you intend — it outputs the removed signal.

### Routing matrix

Below the curve editor, the slot routing matrix shows up to 9 processing slots:

- **Diagonal cells** — the module assigned to each slot. Click to select that slot for curve editing.
- **Off-diagonal cells** — send amplitudes between slots. Default routing is Dynamics (slot 0) → Gain (slot 1) → Master (slot 8).
- Right-click a diagonal cell to change the module type.

Available module types: **Dynamics** (spectral compressor), **Freeze**, **Phase Smear**, **Contrast**, **Gain**, **Mid/Side**, **T/S Split**, **Harmonic** (placeholder, no DSP yet), plus experimental modules added in 0.15: **Future**, **Punch**, **Rhythm**, **Geometry**, **Modulate**, **Circuit**, **PAST**, **KINETICS**, **Harmony**, **LIFE**. Slot 8 is always **Master**.

### FFT Size

Selectable from 512 to 16384. Larger sizes give better frequency resolution and higher latency. The default 2048 gives ~46 ms latency at 44.1 kHz and ~21 Hz per bin.

---

## Sidechain

Spectral Forge has one stereo sidechain input. Route any source to it in Bitwig as usual. Each sidechain-aware slot has its own SC gain and SC source (Follow / L+R / L / R / M / S), so different slots can key off different parts of the same sidechain signal.

---

## Running tests

```bash
cargo test                   # all default tests (68 files under tests/, plus unit tests)
cargo test engine            # engine contract tests only
cargo test stft              # STFT roundtrip test only
cargo test module            # SpectralModule trait compliance tests
cargo test --features probe  # also runs the 4 probe-gated test files (calibration, calibration_roundtrip, circuit, bin_physics_pipeline)
```

---

## Credits

Built on [nih-plug](https://github.com/robbert-vdh/nih-plug) (Robbert van der Helm), [realfft](https://github.com/HEnquist/realfft) (Henrik Enquist), [triple_buffer](https://github.com/HadrienG2/triple-buffer), and the [CLAP plugin standard](https://github.com/free-audio/clap) (Alexandre Bique et al.). Phase vocoder algorithm references from [pvx](https://github.com/TheColby/pvx) (Colby Leider). See [CREDITS.md](CREDITS.md) for full details.

## License
              
    The original source code in this repository is dedicated to the
    public domain under the                                        
    [Creative Commons Zero v1.0 Universal (CC0-1.0)]
    (https://creativecommons.org/publicdomain/zero/1.0/legalcode).
    To the extent possible under law, the author has waived all copyright                                         
    and related or neighbouring rights to this work.                     
                                                    
    ### Third-party components
                              
    The compiled plugin binary links against third-party libraries that
    retain their own licenses. Distributions of the compiled binary must
    preserve their notices:                                             
                           
    | Library | License |
    |---------|---------|
    | [nih-plug](https://github.com/robbert-vdh/nih-plug) | ISC |
    | [egui](https://github.com/emilk/egui) | MIT OR Apache-2.0 |
    | [realfft](https://github.com/HEnquist/realfft) | MIT |     
    | [triple_buffer](https://github.com/HadrienG2/triple-buffer) | LGPL-3.0 | (I need to check the exact requirements for this)
    | [parking_lot](https://github.com/Amanieu/parking_lot) | MIT OR Apache-2.0 |
                                                                                 
    Run `cargo license` in the repository for the full dependency list.
                                                                       

