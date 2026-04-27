use num_complex::Complex;
use serde::{Deserialize, Serialize};
use crate::params::{FxChannelTarget, StereoLink};
use super::{ModuleContext, ModuleType, SpectralModule};

pub const MAX_PEAKS:        usize = 32;
pub const MAX_DRIFT_SITES:  usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum PunchMode {
    #[default]
    Direct,
    Inverse,
}

impl PunchMode {
    pub fn label(self) -> &'static str {
        match self {
            PunchMode::Direct  => "Direct",
            PunchMode::Inverse => "Inverse",
        }
    }
}

pub struct PunchModule {
    mode:        PunchMode,
    fft_size:    usize,
    sample_rate: f32,
    /// Smoothed carve depth applied this hop (0 = no carve, 1 = full mute), per channel × per bin.
    /// Allocated in `reset()`. Tasks 2c.4-2c.6 read/write this.
    current_carve_depth: [Vec<f32>; 2],
    /// Sub-bin pitch-drift accumulator (in fractional bins), per channel × per bin.
    /// Allocated in `reset()`. Task 2c.5 populates.
    drift_accum:         [Vec<f32>; 2],
    /// Inverted-SC scratch for `PunchMode::Inverse` peak detection. One Vec per channel,
    /// sized in `reset()`. Contents are transient per process() call.
    peak_scratch:        [Vec<f32>; 2],
    /// Sidechain peak indices detected this hop (Task 2c.3 fills).
    peak_bin:            [u32; MAX_PEAKS],
    peak_count:          usize,
    #[cfg(any(test, feature = "probe"))]
    last_probe:          crate::dsp::modules::ProbeSnapshot,
}

impl PunchModule {
    pub fn new() -> Self {
        Self {
            mode:        PunchMode::default(),
            fft_size:    2048,
            sample_rate: 44100.0,
            current_carve_depth: [Vec::new(), Vec::new()],
            drift_accum:         [Vec::new(), Vec::new()],
            peak_scratch:        [Vec::new(), Vec::new()],
            peak_bin:            [0u32; MAX_PEAKS],
            peak_count:          0,
            #[cfg(any(test, feature = "probe"))]
            last_probe:          Default::default(),
        }
    }

    pub fn set_mode(&mut self, mode: PunchMode) { self.mode = mode; }
    pub fn mode(&self) -> PunchMode { self.mode }

    pub fn drift_accum_slice(&self, ch: usize) -> &[f32] { &self.drift_accum[ch] }
    pub fn current_carve_depth_slice(&self, ch: usize) -> &[f32] { &self.current_carve_depth[ch] }
}

impl Default for PunchModule {
    fn default() -> Self { Self::new() }
}

impl SpectralModule for PunchModule {
    fn reset(&mut self, sample_rate: f32, fft_size: usize) {
        self.sample_rate = sample_rate;
        self.fft_size    = fft_size;
        let n = fft_size / 2 + 1;
        for ch in 0..2 {
            self.current_carve_depth[ch] = vec![0.0; n];
            self.drift_accum[ch]         = vec![0.0; n];
            self.peak_scratch[ch]        = vec![0.0; n];
        }
        self.peak_count = 0;
    }

    fn clear_state(&mut self) {
        for ch in 0..2 {
            self.current_carve_depth[ch].fill(0.0);
            self.drift_accum[ch].fill(0.0);
            self.peak_scratch[ch].fill(0.0);
        }
        self.peak_count = 0;
    }

    fn process(
        &mut self,
        channel: usize,
        _stereo_link: StereoLink,
        _target: FxChannelTarget,
        bins: &mut [Complex<f32>],
        sidechain: Option<&[f32]>,
        curves: &[&[f32]],
        suppression_out: &mut [f32],
        _physics: Option<&mut crate::dsp::bin_physics::BinPhysics>,
        ctx: &ModuleContext<'_>,
    ) {
        let ch = channel.min(1);
        let n  = bins.len();

        // Resize state if num_bins changed (e.g. FFT-size change).
        if self.current_carve_depth[ch].len() != n {
            self.current_carve_depth[ch].resize(n, 0.0);
        }
        if self.drift_accum[ch].len() != n {
            self.drift_accum[ch].resize(n, 0.0);
        }
        if self.peak_scratch[ch].len() != n {
            self.peak_scratch[ch].resize(n, 0.0);
        }

        let probe_k = n / 2;

        let amount_curve = curves.get(0).copied().unwrap_or(&[][..]);
        let width_curve  = curves.get(1).copied().unwrap_or(&[][..]);
        let fillm_curve  = curves.get(2).copied().unwrap_or(&[][..]); // Task 2c.5
        let ampfl_curve  = curves.get(3).copied().unwrap_or(&[][..]);
        let heal_curve   = curves.get(4).copied().unwrap_or(&[][..]);
        let mix_curve    = curves.get(5).copied().unwrap_or(&[][..]);

        #[cfg(any(test, feature = "probe"))]
        let mut probe_amount_pct = 0.0f32;
        #[cfg(any(test, feature = "probe"))]
        let mut probe_mix_pct    = 0.0f32;

        // ── Detect peaks in the (possibly inverted) sidechain ─────────────────────
        self.peak_count = 0;
        if let Some(sc) = sidechain {
            // Slot-wide peak-detection params read at probe_k.
            let amount_g = amount_curve.get(probe_k).copied().unwrap_or(1.0).clamp(0.0, 2.0);
            let threshold = 0.05_f32 + (1.0 - amount_g / 2.0) * 0.25; // 0.05..0.30
            let width_g   = width_curve.get(probe_k).copied().unwrap_or(1.0).clamp(0.0, 2.0);
            let min_dist  = ((width_g * 4.0).round() as usize).max(2);

            let nn = n.min(sc.len());
            let peak_source: &[f32] = match self.mode {
                PunchMode::Direct  => &sc[..nn],
                PunchMode::Inverse => {
                    // Build inverted SC into peak_scratch (a dedicated buffer — does NOT
                    // touch current_carve_depth, which holds the smoothing follower state).
                    for k in 0..nn { self.peak_scratch[ch][k] = (1.0 - sc[k]).max(0.0); }
                    &self.peak_scratch[ch][..nn]
                }
            };
            self.peak_count = detect_peaks(peak_source, &mut self.peak_bin, threshold, min_dist);
        }

        // ── Apply carve, amp-fill, healing follower, and mix ─────────────────────
        let width_g = width_curve.get(probe_k).copied().unwrap_or(1.0).clamp(0.0, 2.0);
        let half_w  = ((width_g * 4.0).round() as usize).max(1).min(16);

        let hop_dt   = ctx.fft_size as f32 / ctx.sample_rate / 4.0; // OVERLAP=4
        let smooth_a = (-hop_dt / 0.005).exp(); // 5 ms attack

        for k in 0..n {
            let amount_g = amount_curve.get(k).copied().unwrap_or(1.0).clamp(0.0, 2.0);
            let depth    = (amount_g * 0.5).clamp(0.0, 1.0); // neutral=0.5
            let ampfl_g  = ampfl_curve.get(k).copied().unwrap_or(1.0).clamp(0.0, 4.0);
            let amp_fill = ampfl_g; // neutral=1.0
            let heal_g   = heal_curve.get(k).copied().unwrap_or(1.0).clamp(0.05, 2.0);
            let heal_ms  = (heal_g * 150.0).clamp(20.0, 2000.0);
            let mix_g    = mix_curve.get(k).copied().unwrap_or(1.0).clamp(0.0, 2.0);
            let mix      = (mix_g * 0.5).clamp(0.0, 1.0);

            // Per-bin carve target: max over peaks of (depth × triangle weight in [0, half_w]).
            let mut target = 0.0f32;
            for i in 0..self.peak_count {
                let pk = self.peak_bin[i] as i64;
                let dist = (k as i64 - pk).unsigned_abs() as usize;
                if dist <= half_w {
                    let weight = 1.0 - (dist as f32) / ((half_w + 1) as f32);
                    let t = depth * weight;
                    if t > target { target = t; }
                }
            }

            // Follower: 5 ms attack, HEAL release.
            let release_a = (-hop_dt / (heal_ms * 0.001)).exp();
            let prev = self.current_carve_depth[ch][k];
            let cur  = if target > prev {
                smooth_a * prev + (1.0 - smooth_a) * target
            } else {
                release_a * prev + (1.0 - release_a) * target
            };
            self.current_carve_depth[ch][k] = cur;

            // Neighbour amp-fill: bins NEAR a peak (not at it) get boosted.
            let mut neighbour_boost = 1.0f32;
            for i in 0..self.peak_count {
                let pk = self.peak_bin[i] as i64;
                let dist = (k as i64 - pk).unsigned_abs() as usize;
                if dist > 0 && dist <= half_w {
                    let w = 1.0 - (dist as f32) / ((half_w + 1) as f32);
                    neighbour_boost = neighbour_boost.max(1.0 + (amp_fill - 1.0) * w);
                }
            }

            let dry    = bins[k];
            let carved = dry * (1.0 - cur) * neighbour_boost;
            let wet    = carved;
            bins[k] = Complex::new(
                dry.re * (1.0 - mix) + wet.re * mix,
                dry.im * (1.0 - mix) + wet.im * mix,
            );

            // ── Pitch-fill: drift neighbour bins toward the nearest peak ────────
            let fillm_g = fillm_curve.get(k).copied().unwrap_or(1.0).clamp(0.0, 2.0);
            let target_drift = if fillm_g > 1e-3 {
                let mut best: Option<(usize, i64)> = None; // (peak_index, signed distance)
                for i in 0..self.peak_count {
                    let pk = self.peak_bin[i] as i64;
                    let signed = pk - k as i64; // positive: peak is above us → drift up
                    let dist = signed.unsigned_abs() as usize;
                    if dist > 0 && dist <= half_w {
                        if best.map(|(_, d)| (d.unsigned_abs() as usize) > dist).unwrap_or(true) {
                            best = Some((i, signed));
                        }
                    }
                }
                if let Some((_, signed)) = best {
                    // Drift fraction: scale by FILL_MODE (0..1 maps to 0..0.5 bin cap).
                    let direction = (signed as f32).signum();
                    direction * (fillm_g * 0.25).clamp(0.0, 0.5)
                } else { 0.0 }
            } else { 0.0 };

            // Slew-rate limit drift to 0.005 bin/hop, then clamp to ±0.5 bins.
            let prev_drift = self.drift_accum[ch][k];
            let drift_step = (target_drift - prev_drift).clamp(-0.005, 0.005);
            let new_drift  = (prev_drift + drift_step).clamp(-0.5, 0.5);
            self.drift_accum[ch][k] = new_drift;

            // Apply phase rotation: per-hop Δφ = (π/2) × drift_bins (assumes OVERLAP=4).
            if new_drift.abs() > 1e-6 {
                let dphi = std::f32::consts::FRAC_PI_2 * new_drift;
                let (s, c) = dphi.sin_cos();
                let rot = Complex::new(c, s);
                bins[k] = bins[k] * rot;
            }

            #[cfg(any(test, feature = "probe"))]
            if k == probe_k {
                probe_amount_pct = depth * 100.0;
                probe_mix_pct    = mix * 100.0;
            }
        }

        suppression_out.fill(0.0);

        #[cfg(any(test, feature = "probe"))]
        {
            self.last_probe = crate::dsp::modules::ProbeSnapshot {
                amount_pct: Some(probe_amount_pct),
                mix_pct:    Some(probe_mix_pct),
                ..Default::default()
            };
        }
    }

    fn module_type(&self) -> ModuleType { ModuleType::Punch }
    fn num_curves(&self) -> usize { 6 }
    fn set_punch_mode(&mut self, mode: PunchMode) { self.set_mode(mode); }

    #[cfg(any(test, feature = "probe"))]
    fn last_probe(&self) -> crate::dsp::modules::ProbeSnapshot { self.last_probe }
}

/// Detect up to `out.len()` local maxima in `mag`, above `threshold`, separated by
/// at least `min_dist` bins. Returns the number of peaks written.
///
/// Greedy: for each local max above threshold, sort by magnitude desc and skip any
/// that fall within `min_dist` of an already-accepted higher peak.
///
/// Audio-thread safe: uses fixed-size on-stack scratch (`[_; 256]`) — silently caps
/// at 256 candidates, far above any realistic local-max density.
pub fn detect_peaks(mag: &[f32], out: &mut [u32], threshold: f32, min_dist: usize) -> usize {
    let n = mag.len();
    if n < 3 || out.is_empty() { return 0; }

    // Pass 1: collect candidates (local maxima above threshold) into fixed-size scratch.
    let mut cand_count = 0usize;
    let mut cand_mag: [f32; 256] = [0.0; 256];
    let mut cand_bin: [u32; 256] = [0; 256];
    for k in 1..n - 1 {
        let m = mag[k];
        if m < threshold { continue; }
        if m > mag[k - 1] && m >= mag[k + 1] {
            if cand_count < cand_mag.len() {
                cand_mag[cand_count] = m;
                cand_bin[cand_count] = k as u32;
                cand_count += 1;
            }
        }
    }

    // Pass 2: insertion sort candidates by descending magnitude (in-place).
    for i in 1..cand_count {
        let mi = cand_mag[i];
        let bi = cand_bin[i];
        let mut j = i;
        while j > 0 && cand_mag[j - 1] < mi {
            cand_mag[j] = cand_mag[j - 1];
            cand_bin[j] = cand_bin[j - 1];
            j -= 1;
        }
        cand_mag[j] = mi;
        cand_bin[j] = bi;
    }

    // Pass 3: greedy write into `out`, enforcing min_dist between accepted peaks.
    let mut written = 0usize;
    for i in 0..cand_count {
        if written >= out.len() { break; }
        let b = cand_bin[i];
        let mut ok = true;
        for j in 0..written {
            if (out[j] as i64 - b as i64).unsigned_abs() < min_dist as u64 {
                ok = false;
                break;
            }
        }
        if ok {
            out[written] = b;
            written += 1;
        }
    }
    written
}
