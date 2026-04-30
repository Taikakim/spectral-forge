use num_complex::Complex;

/// Maximum number of groups returned per hop. 4 covers most musical material
/// (a chord triad + bass = 4 voices).
pub const MAX_GROUPS: usize = 4;

/// Maximum number of harmonics tracked per group.
pub const MAX_HARMONICS_PER_GROUP: usize = 16;

/// Maximum number of candidate peaks scanned each hop (top-K by magnitude).
pub const MAX_PEAK_CANDIDATES: usize = 32;

/// Tolerance in cents (1/100 of a semitone) for matching a peak to expected harmonic positions.
/// 25 cents = quarter-tone; chosen as a balance between strict tracking and natural inharmonicity.
pub const HARMONIC_TOLERANCE_CENTS: f32 = 25.0;

const FREQ_LOW_HZ: f32 = 60.0;
const FREQ_HIGH_HZ: f32 = 5000.0;

/// One detected harmonic group. Storage is fixed-size to avoid heap.
#[derive(Clone, Copy, Debug, Default)]
pub struct HarmonicGroup {
    pub fundamental_hz:  f32,
    pub harmonic_count:  u8,
    /// Bin indices of the harmonics (positions 0..harmonic_count are valid).
    pub harmonic_bins:   [u16; MAX_HARMONICS_PER_GROUP],
    /// Sum of magnitudes across detected harmonics (group "mass").
    pub total_magnitude: f32,
}

/// Scratch + output for harmonic-group detection. One instance per channel in Pipeline.
pub struct HarmonicGroupBuf {
    /// Output groups (only `groups_len` are valid).
    groups:     [HarmonicGroup; MAX_GROUPS],
    groups_len: usize,
    /// Scratch: top-K candidate peaks, sorted by magnitude descending.
    candidates: [(f32, usize, f32); MAX_PEAK_CANDIDATES], // (mag, bin_idx, freq_hz)
    /// Per-bin "claimed by group" flag — prevents double-counting across groups.
    claimed:    Vec<bool>,
}

impl HarmonicGroupBuf {
    pub fn new() -> Self {
        Self {
            groups:     [HarmonicGroup::default(); MAX_GROUPS],
            groups_len: 0,
            candidates: [(0.0, 0, 0.0); MAX_PEAK_CANDIDATES],
            claimed:    vec![false; crate::dsp::pipeline::MAX_NUM_BINS],
        }
    }

    /// Detect up to MAX_GROUPS harmonic clusters from `bins` + `ifreq`.
    /// Caller invariant: both slices length ≥ fft_size/2 + 1; `ifreq` from Phase 6.1.
    pub fn detect(
        &mut self,
        bins:        &[Complex<f32>],
        ifreq:       &[f32],
        sample_rate: f32,
        fft_size:    usize,
    ) {
        let num_bins = fft_size / 2 + 1;
        debug_assert!(bins.len() >= num_bins);
        debug_assert!(ifreq.len() >= num_bins);

        // Reset state.
        self.groups_len = 0;
        for c in self.claimed.iter_mut().take(num_bins) { *c = false; }

        // Step 1: top-K peaks by magnitude. We use a simple scan-and-replace into a small
        // sorted array — O(N × MAX_PEAK_CANDIDATES) but the inner loop is tiny.
        let mut count = 0usize;
        let mut min_idx = 0usize;
        let mut min_mag = f32::INFINITY;
        for k in 1..num_bins {
            let f = ifreq[k];
            if !(f >= FREQ_LOW_HZ && f <= FREQ_HIGH_HZ) { continue; }
            let mag = bins[k].norm();
            if mag <= 1e-7 { continue; }
            if count < MAX_PEAK_CANDIDATES {
                self.candidates[count] = (mag, k, f);
                count += 1;
                if count == MAX_PEAK_CANDIDATES {
                    // Find min for replacement scheme.
                    min_idx = 0; min_mag = self.candidates[0].0;
                    for (i, &(m, _, _)) in self.candidates.iter().enumerate() {
                        if m < min_mag { min_mag = m; min_idx = i; }
                    }
                }
            } else if mag > min_mag {
                self.candidates[min_idx] = (mag, k, f);
                // Recompute min.
                min_idx = 0; min_mag = self.candidates[0].0;
                for (i, &(m, _, _)) in self.candidates.iter().enumerate() {
                    if m < min_mag { min_mag = m; min_idx = i; }
                }
            }
        }
        if count == 0 { return; }

        // Sort candidates descending by magnitude (insertion sort — count ≤ 32).
        {
            let cands = &mut self.candidates[..count];
            for i in 1..cands.len() {
                let mut j = i;
                while j > 0 && cands[j].0 > cands[j-1].0 {
                    cands.swap(j, j-1);
                    j -= 1;
                }
            }
        } // mutable borrow of self.candidates ends here

        // Step 2: greedy group assignment. For each unassigned candidate (highest mag first):
        // treat as fundamental, sweep its harmonics, mark them claimed, build a group.
        let tol_ratio = (2.0_f32).powf(HARMONIC_TOLERANCE_CENTS / 1200.0); // upper bound multiplier
        let inv_tol_ratio = 1.0 / tol_ratio;
        for ci in 0..count {
            if self.groups_len >= MAX_GROUPS { break; }
            let (mag0, bin0, f0) = self.candidates[ci];
            if self.claimed[bin0] { continue; }

            let mut g = HarmonicGroup::default();
            g.fundamental_hz   = f0;
            g.harmonic_bins[0] = bin0 as u16;
            g.harmonic_count   = 1;
            g.total_magnitude  = mag0;
            self.claimed[bin0] = true;

            // Sweep n = 2..=MAX_HARMONICS_PER_GROUP.
            for n in 2..=MAX_HARMONICS_PER_GROUP as u32 {
                let target_hz = f0 * n as f32;
                if target_hz > FREQ_HIGH_HZ { break; }
                let lo_hz = target_hz * inv_tol_ratio;
                let hi_hz = target_hz * tol_ratio;

                // Find the highest-magnitude unclaimed candidate in [lo_hz, hi_hz].
                let mut best_idx = usize::MAX;
                let mut best_mag = 0.0_f32;
                for cj in (ci+1)..count {
                    let (mag, bin, f) = self.candidates[cj];
                    if self.claimed[bin] { continue; }
                    if f >= lo_hz && f <= hi_hz && mag > best_mag {
                        best_mag = mag;
                        best_idx = cj;
                    }
                }
                if best_idx != usize::MAX {
                    let (mag, bin, _) = self.candidates[best_idx];
                    let h = g.harmonic_count as usize;
                    g.harmonic_bins[h]   = bin as u16;
                    g.harmonic_count    += 1;
                    g.total_magnitude   += mag;
                    self.claimed[bin]    = true;
                }
            }

            // Reject groups that found <2 harmonics — likely a stray peak, not a voice.
            if g.harmonic_count >= 2 {
                self.groups[self.groups_len] = g;
                self.groups_len += 1;
            }
        }
    }

    /// Borrow the detected groups (length 0..=MAX_GROUPS).
    pub fn groups(&self) -> &[HarmonicGroup] {
        &self.groups[..self.groups_len]
    }
}

impl Default for HarmonicGroupBuf {
    fn default() -> Self { Self::new() }
}
