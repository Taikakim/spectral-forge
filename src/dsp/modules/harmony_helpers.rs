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
