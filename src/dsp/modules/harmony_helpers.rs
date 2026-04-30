//! Shared helpers for the Harmony module.
//!
//! Pure functions, no state, no allocation in the hot path.

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PeakRecord {
    pub bin: usize,
    pub mag: f32,
}

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
