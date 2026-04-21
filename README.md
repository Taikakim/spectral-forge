# Spectral Forge

A spectral compressor and modular effects processor for Linux/Windows, implemented as a CLAP plugin. Designed for Bitwig Studio.

Patent-safe design — does not use the Hilbert/convolution approach from oeksound's patents.

The dynamics section is functional. The modular multi-fx routing system is under active development — expect things to be unfinished or experimental on the main branch. If you just want to use this, stick to the releases.

---

<img width="898" height="454" alt="Screenshot_20260421_115617" src="https://github.com/user-attachments/assets/12213421-60ca-4178-ade3-b7bba5575f09" />

## Building and installing

**Requirements:** Rust stable toolchain, Cargo. `clap-validator` is optional for testing.

```bash
# Debug build
cargo build

# Release build
cargo build --release

# Bundle as .clap
cargo run --package xtask -- bundle spectral_forge --release

# Install to Bitwig's default search path
cp target/bundled/spectral_forge.clap ~/.clap/
```

After installing, rescan plugins in Bitwig (or restart). The plugin appears as **Spectral Forge** under CLAP instruments/effects.

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

Adaptive curve selector buttons show the curves available for the **currently selected slot**. A Dynamics slot shows THRESHOLD / RATIO / ATTACK / RELEASE / KNEE / MIX; a Gain slot shows GAIN / SC SMOOTH; a Freeze slot shows LENGTH / THRESHOLD / PORTAMENTO / RESISTANCE; and so on. Clicking a button makes that curve active for editing.

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

**SC assignment (per slot):** SC1 / SC2 / SC3 / SC4 / Self buttons assign the current slot's sidechain input. Active inputs light up green.

**GainMode (Gain slots only):** Add / Subtract / Pull buttons appear when a Gain module is selected.

**Global row:**

| Control | Range | Description |
|---------|-------|-------------|
| IN | ±18 dB | Input gain |
| OUT | ±18 dB | Output gain |
| MIX | 0–100 % | Global dry/wet |
| SC | ±18 dB | Sidechain input gain |
| AUTO MK | on/off | Auto makeup gain — long-term GR compensation |
| DELTA | on/off | Delta monitor — hear only what is being removed |

**Dynamics + per-curve row:**

| Control | Range | Description |
|---------|-------|-------------|
| Atk | 0.5–200 ms | Global attack time base |
| Rel | 1–500 ms | Global release time base |
| Sens | 0–1 | Sensitivity — how selectively peaks are targeted |
| Width | 0–0.5 st | Gain-reduction mask blur radius (semitones) |
| Offset | ±3 | Additive offset for the active curve |
| Tilt | ±3 | Spectral tilt for the active curve, pivoting at 1 kHz |

**DELTA** is the fastest way to verify the plugin is targeting what you intend — it outputs the removed signal.

### Routing matrix

Below the curve editor, the slot routing matrix shows up to 9 processing slots:

- **Diagonal cells** — the module assigned to each slot. Click to select that slot for curve editing.
- **Off-diagonal cells** — send amplitudes between slots. Default routing is serial: slot 0 → 1 → 2 → Master (slot 8).
- Right-click a diagonal cell to change the module type.

Available module types: **Dynamics** (spectral compressor), **Freeze**, **Phase Smear**, **Contrast**, **Gain**, **Mid/Side**, **T/S Split**, **Harmonic**.

### FFT Size

Selectable from 512 to 16384. Larger sizes give better frequency resolution and higher latency. The default 2048 gives ~46 ms latency at 44.1 kHz and ~21 Hz per bin.

---

## Sidechain

Up to 4 auxiliary sidechain inputs are supported. Route any source to an aux input (Bitwig: enable aux inputs in the track header). Each slot can be independently assigned to a sidechain input via the routing matrix.

---

## Running tests

```bash
cargo test            # all tests (28 total across 5 test files)
cargo test engine     # engine contract tests only
cargo test stft       # STFT roundtrip test only
cargo test module     # SpectralModule trait compliance tests
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
                                                                       

