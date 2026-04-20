# Spectral Forge

A spectral compressor and effects processor for Linux, implemented as a CLAP plugin. Designed for Bitwig Studio.

Patent-safe design — does not use the Hilbert/convolution approach from oeksound's patents.

Very early test version. The dynamics section is more or less functional, the multi fx part is under heavy testing. Much of the developement happens in Main, so expect things to be unexpectedly broken or unfinished at the current stage.

---

<img width="900" height="600" alt="Screenshot_20260415_204922" src="https://github.com/user-attachments/assets/79a325fa-cece-4aeb-8674-90fbd9dad162" />

## Building and installing

**Requirements:** Rust stable toolchain, Cargo, `clap-validator` optional for testing.

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

### Top bar

Seven curve selector buttons (**THRESHOLD / RATIO / ATTACK / RELEASE / KNEE / MAKEUP / MIX**) choose which parameter curve is active for editing. All seven curves are always drawn; the selected one is highlighted and interactive.

To the right: three tab buttons — **DYNAMICS**, **EFFECTS**, **HARMONIC** — switch the lower portion of the interface.

Further right: **Floor** and **Ceil** drag-values set the dBFS range of the spectrum display. **Falloff** sets the peak-hold decay time in ms.

### Curve display

The large centre area shows:

- **Background grid** — vertical Hz lines at standard intervals; horizontal reference lines whose values depend on which curve is selected.
- **Spectrum gradient** — the pre-FX signal (teal line) and post-FX signal (pink line) with a filled gradient between them showing the amount of processing.
- **Response curves** — seven coloured polylines showing the current value of each parameter across the frequency range. Attack and release curves are drawn as dashed lines to distinguish them from the others.
- **Node handles** — only for the selected curve. Circles for bell-type nodes; right-pointing triangles (▶) for the low-shelf node; left-pointing triangles (◀) for the high-shelf node.

### Node interaction

| Action | Effect |
|--------|--------|
| Drag node | Move frequency and gain |
| Scroll wheel over node | Coarse Q (bandwidth) adjustment |
| Hold both mouse buttons + drag up/down | Smooth Q adjustment (500 px = full range) |
| Double-click node | Reset node to default position |

### Dynamics tab — control strip

| Control | Range | Description |
|---------|-------|-------------|
| IN | ±18 dB | Input gain |
| OUT | ±18 dB | Output gain |
| MIX | 0–100 % | Global dry/wet |
| SC | ±18 dB | Sidechain input gain |
| **Dynamics group** | | |
| Atk | 0.5–200 ms | Global attack time (scaled per band by Freq) |
| Rel | 1–500 ms | Global release time (scaled per band by Freq) |
| Freq | 0–1 | Frequency-dependent time scaling strength |
| Sens | 0–1 | Sensitivity — how selectively peaks are targeted |
| Width | 0–0.5 st | Gain-reduction mask blur radius (semitones) |
| **Threshold shaping** | | |
| Th Off | ±40 dB | Uniform vertical shift of the entire threshold curve |
| Tilt | ±6 dB/oct | Spectral tilt of the threshold, pivoting at 1 kHz |
| AUTO MK | on/off | Auto makeup gain — long-term GR compensation |
| DELTA | on/off | Delta monitor — hear only what is being removed |

**Tilt** rotates the threshold curve around 1 kHz. Positive values raise the threshold toward high frequencies (compress treble less); negative values lower it (compress treble more). **Th Off** shifts the whole curve up or down without changing its shape.

### Effects tab

Select an effects mode:

| Mode | Description |
|------|-------------|
| BYPASS | No effect processing |
| FREEZE | Spectral freeze — holds the current FFT frame indefinitely |
| PHASE | Phase randomiser — randomises per-bin phase each hop |
| CONTRAST | Spectral contrast enhancer — boosts peaks, cuts valleys |

When **PHASE** is active: an **Amount** slider controls how much phase rotation is applied per hop (0 = none, 1 = full ±π randomisation).

When **CONTRAST** is active: a **Depth** slider (−12 to +12 dB) controls the enhancement strength. Negative values flatten the spectrum toward its local mean; positive values expand peaks away from it. The **Ratio** and other curves continue to apply per-frequency modulation on top of the global Depth setting.

---

## Sidechain

Route a sidechain signal into the plugin's auxiliary input. Bitwig: enable the plugin's sidechain input in the track header, then route a source to it.

When a sidechain signal is present it drives the gain-reduction decisions instead of the main signal. **SC** adjusts the sidechain level. Sidechain attack/release times are set separately from the main signal times.

---

## Running tests

```bash
cargo test            # all tests
cargo test engine     # engine contract tests only
cargo test stft       # STFT roundtrip test only
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
                                                                       

