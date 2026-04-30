// src/dsp/chromagram.rs

use num_complex::Complex;

/// 12 = number of pitch classes (C, C#, D, ..., B).
pub const NUM_PITCH_CLASSES: usize = 12;

/// Lower frequency cutoff: anything below this Hz is ignored (sub-bass / DC noise).
pub const FREQ_LOW_HZ: f32 = 80.0;

/// Upper frequency cutoff: anything above this Hz is ignored (cymbal / hiss noise).
pub const FREQ_HIGH_HZ: f32 = 5000.0;

/// MIDI note number for A4 (440 Hz reference).
const MIDI_A4: f32 = 69.0;

/// Compute IF-refined pitch-class profile from FFT bins.
///
/// `bins`  — slice of FFT output, length ≥ fft_size/2 + 1.
/// `ifreq` — optional IF in Hz per bin (Phase 6.1). When `None`, falls back to bin-center
///           frequencies. IF gives sub-bin accuracy and tunes the chromagram correctly even
///           when the fundamental sits between bin centers.
/// `out`   — 12-element output array; overwritten. Sums to 1.0 on non-silent input, all 0 on silent.
///
/// No allocation. ~6 FLOPs + 1 log2 per qualifying bin.
pub fn compute_chromagram(
    bins:        &[Complex<f32>],
    ifreq:       Option<&[f32]>,
    sample_rate: f32,
    fft_size:    usize,
    out:         &mut [f32; NUM_PITCH_CLASSES],
) {
    let num_bins = fft_size / 2 + 1;
    debug_assert!(bins.len() >= num_bins);
    if let Some(f) = ifreq { debug_assert!(f.len() >= num_bins); }

    *out = [0.0; NUM_PITCH_CLASSES];
    let bin_freq_hz = sample_rate / fft_size as f32;

    for k in 1..num_bins {                          // skip DC
        let mag = bins[k].norm();
        if mag <= 1e-7 { continue; }                // skip silent bins
        let freq_hz = match ifreq {
            Some(f) => f[k],
            None    => k as f32 * bin_freq_hz,
        };
        if !(freq_hz >= FREQ_LOW_HZ && freq_hz <= FREQ_HIGH_HZ) { continue; }
        let midi = 12.0 * (freq_hz / 440.0).log2() + MIDI_A4;
        let pc   = (midi.round() as i32).rem_euclid(NUM_PITCH_CLASSES as i32) as usize;
        out[pc] += mag;
    }

    let sum: f32 = out.iter().sum();
    if sum > 1e-7 {
        for v in out.iter_mut() { *v /= sum; }
    }
}
