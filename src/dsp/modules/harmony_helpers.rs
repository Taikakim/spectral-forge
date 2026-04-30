//! Shared helpers for the Harmony module.
//!
//! Pure functions, no state, no allocation in the hot path.

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PeakRecord {
    pub bin: usize,
    pub mag: f32,
}

/// First 32 positive zeros of the Bessel function J_0.
/// Source: NIST DLMF Table 10.21.iii.
pub const BESSEL_J0_ZEROS: [f32; 32] = [
     2.4048256, 5.5200781, 8.6537280, 11.7915344, 14.9309177,
    18.0710640, 21.2116366, 24.3524715, 27.4934791, 30.6346065,
    33.7758202, 36.9170984, 40.0584258, 43.1997917, 46.3411884,
    49.4826099, 52.6240518, 55.7655108, 58.9069839, 62.0484692,
    65.1899648, 68.3314693, 71.4729816, 74.6145006, 77.7560256,
    80.8975559, 84.0390908, 87.1806298, 90.3221726, 93.4637186,
    96.6052677, 99.7468195,
];

/// First 32 primes.
pub const SMALL_PRIMES: [u32; 32] = [
     2,  3,  5,  7, 11, 13, 17, 19, 23, 29,
    31, 37, 41, 43, 47, 53, 59, 61, 67, 71,
    73, 79, 83, 89, 97,101,103,107,109,113,
   127,131,
];

/// Find local maxima in `mag` above `threshold`, keeping the top `out.len()`
/// by magnitude. Output is sorted descending by magnitude. Unused slots are
/// zeroed. Returns the number of valid entries written.
///
/// A bin `k` is a local maximum iff `mag[k] > mag[k-1] && mag[k] > mag[k+1]`.
/// Boundary bins (0 and last) are never considered peaks.
///
/// O(N + N·log K). No allocation: the caller owns the output slice.
pub fn find_top_k_peaks(mag: &[f32], threshold: f32, out: &mut [PeakRecord]) -> usize {
    for slot in out.iter_mut() { *slot = PeakRecord::default(); }
    if mag.len() < 3 || out.is_empty() { return 0; }

    let k_max = out.len();
    let mut filled: usize = 0;

    for k in 1..mag.len() - 1 {
        let m = mag[k];
        if m < threshold { continue; }
        if !(m > mag[k - 1] && m > mag[k + 1]) { continue; }

        // Decide insertion position: linear scan is fine for K ≤ 32.
        let mut insert_at = filled;
        for j in 0..filled {
            if m > out[j].mag { insert_at = j; break; }
        }
        if insert_at >= k_max { continue; }

        // Shift right, drop the last entry if we were already full.
        let end = filled.min(k_max - 1);
        for j in (insert_at..end).rev() { out[j + 1] = out[j]; }
        out[insert_at] = PeakRecord { bin: k, mag: m };
        if filled < k_max { filled += 1; }
    }
    filled
}

/// 24-element chord template bank: 12 major + 12 minor.
/// Each template is a 12-element [0/1] pitch-class profile.
/// Index 0..=11: major (C, C#, …, B). Index 12..=23: minor (Cm, C#m, …, Bm).
pub const CHORD_TEMPLATES_24: [[f32; 12]; 24] = [
    // Major
    /* C  */ [1.,0.,0.,0.,1.,0.,0.,1.,0.,0.,0.,0.],
    /* C# */ [0.,1.,0.,0.,0.,1.,0.,0.,1.,0.,0.,0.],
    /* D  */ [0.,0.,1.,0.,0.,0.,1.,0.,0.,1.,0.,0.],
    /* D# */ [0.,0.,0.,1.,0.,0.,0.,1.,0.,0.,1.,0.],
    /* E  */ [0.,0.,0.,0.,1.,0.,0.,0.,1.,0.,0.,1.],
    /* F  */ [1.,0.,0.,0.,0.,1.,0.,0.,0.,1.,0.,0.],
    /* F# */ [0.,1.,0.,0.,0.,0.,1.,0.,0.,0.,1.,0.],
    /* G  */ [0.,0.,1.,0.,0.,0.,0.,1.,0.,0.,0.,1.],
    /* G# */ [1.,0.,0.,1.,0.,0.,0.,0.,1.,0.,0.,0.],
    /* A  */ [0.,1.,0.,0.,1.,0.,0.,0.,0.,1.,0.,0.],
    /* A# */ [0.,0.,1.,0.,0.,1.,0.,0.,0.,0.,1.,0.],
    /* B  */ [0.,0.,0.,1.,0.,0.,1.,0.,0.,0.,0.,1.],
    // Minor
    /* Cm */  [1.,0.,0.,1.,0.,0.,0.,1.,0.,0.,0.,0.],
    /* C#m */ [0.,1.,0.,0.,1.,0.,0.,0.,1.,0.,0.,0.],
    /* Dm */  [0.,0.,1.,0.,0.,1.,0.,0.,0.,1.,0.,0.],
    /* D#m */ [0.,0.,0.,1.,0.,0.,1.,0.,0.,0.,1.,0.],
    /* Em */  [0.,0.,0.,0.,1.,0.,0.,1.,0.,0.,0.,1.],
    /* Fm */  [1.,0.,0.,0.,0.,1.,0.,0.,1.,0.,0.,0.],
    /* F#m */ [0.,1.,0.,0.,0.,0.,1.,0.,0.,1.,0.,0.],
    /* Gm */  [0.,0.,1.,0.,0.,0.,0.,1.,0.,0.,1.,0.],
    /* G#m */ [0.,0.,0.,1.,0.,0.,0.,0.,1.,0.,0.,1.],
    /* Am */  [1.,0.,0.,0.,1.,0.,0.,0.,0.,1.,0.,0.],
    /* A#m */ [0.,1.,0.,0.,0.,1.,0.,0.,0.,0.,1.,0.],
    /* Bm */  [0.,0.,1.,0.,0.,0.,1.,0.,0.,0.,0.,1.],
];

/// Cosine similarity between two equal-length f32 vectors. Returns 0.0 for empty
/// or zero-norm inputs to avoid NaN.
#[inline]
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let mut dot = 0.0_f32;
    let mut a2 = 0.0_f32;
    let mut b2 = 0.0_f32;
    for i in 0..a.len().min(b.len()) {
        dot += a[i] * b[i];
        a2  += a[i] * a[i];
        b2  += b[i] * b[i];
    }
    let denom = (a2 * b2).sqrt();
    if denom < 1e-12 { 0.0 } else { dot / denom }
}

/// Find the chord template best matching `chromagram`. Returns (index, score).
pub fn best_chord_template(chromagram: &[f32; 12]) -> (usize, f32) {
    let mut best_i = 0_usize;
    let mut best_s = -1.0_f32;
    for (i, tmpl) in CHORD_TEMPLATES_24.iter().enumerate() {
        let s = cosine_similarity(chromagram, tmpl);
        if s > best_s { best_s = s; best_i = i; }
    }
    (best_i, best_s)
}
