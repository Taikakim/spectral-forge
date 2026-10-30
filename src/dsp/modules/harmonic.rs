//! Harmonic module — three tools for a sound's harmonic structure, one slot.
//!
//! - **HpSplit**   — harmonic/percussive separation by causal median filtering
//!   (after Fitzgerald 2010): a per-bin median over recent frames estimates the
//!   steady (harmonic) part, a median across neighbouring bins in the current
//!   frame estimates the broadband (percussive) part. The soft masks feed two
//!   virtual matrix rows (H and P) and a BALANCE-weighted main output.
//! - **PartialEq** — gain by partial number rather than frequency, using the
//!   Pipeline's harmonic-group detection: odd/even balance and a tilt over
//!   harmonic number, applied wherever the note sits.
//! - **Exciter**   — adds 2nd–5th harmonics of each bin above a relative
//!   threshold, phase-coherent (harmonic h gets h·φ), from a snapshot of the
//!   input so nothing feeds back.
//!
//! Curves (shared, renamed per mode by `active_layout`):
//! 0 AMOUNT (0..1) · 1 BALANCE (bipolar) · 2 TILT (bipolar) · 3 REACH (0..1) · 4 MIX.
//! Bipolar curves treat gain 1.0 as neutral. At default curves HpSplit and
//! PartialEq are exact passthrough.

use num_complex::Complex;
use serde::{Deserialize, Serialize};

use crate::params::{FxChannelTarget, StereoLink};
use super::{ModuleContext, ModuleType, SpectralModule};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum HarmonicMode {
    #[default]
    HpSplit,
    PartialEq,
    Exciter,
}

impl HarmonicMode {
    pub fn label(self) -> &'static str {
        match self {
            HarmonicMode::HpSplit   => "H/P Split",
            HarmonicMode::PartialEq => "Partial EQ",
            HarmonicMode::Exciter   => "Exciter",
        }
    }
}

/// Longest time-median window (frames). REACH = 1 uses all of it.
const HP_MAX_TIME: usize = 17;
/// Widest frequency-median half-width (bins). REACH = 1 uses all of it.
const HP_MAX_FREQ_HALF: usize = 8;
/// Below this, both median estimates count as silence and the mask is 50/50.
const HP_SILENCE: f32 = 1e-12;

/// Ceiling on any Partial EQ gain (+12 dB) so TILT on high partials can't spike.
const PEQ_MAX_GAIN: f32 = 4.0;
/// REACH = 1 widens each partial's gain to ±this many bins.
const PEQ_MAX_SKIRT: f32 = 4.0;

/// Generated-harmonic level at AMOUNT = 1, relative to the source bin (−12 dB).
const EXC_MAX_LEVEL: f32 = 0.25;
/// Threshold at REACH = 1, dB relative to the frame's loudest bin.
const EXC_MIN_THRESH_DB: f32 = -60.0;
/// Highest harmonic the exciter writes.
const EXC_MAX_HARMONIC: usize = 5;

/// Bipolar curve value: gain 1.0 → 0, 0 → −1, 2 → +1.
#[inline]
fn bipolar(g: f32) -> f32 { (g - 1.0).clamp(-1.0, 1.0) }

#[inline]
fn median_in_place(buf: &mut [f32]) -> f32 {
    let mid = buf.len() / 2;
    let (_, m, _) = buf.select_nth_unstable_by(mid, |a, b| a.total_cmp(b));
    *m
}

pub struct HarmonicModule {
    mode:        HarmonicMode,
    sample_rate: f32,
    fft_size:    usize,
    num_bins:    usize,
    /// Per-channel ring of the last `HP_MAX_TIME` magnitude frames, flattened
    /// `[frame * num_bins + k]`.
    hp_hist:   [Vec<f32>; 2],
    hp_pos:    [usize; 2],
    hp_filled: [usize; 2],
    /// Current-frame magnitudes (scratch).
    mags:      Vec<f32>,
    /// Virtual outputs for HpSplit: harmonic and percussive streams.
    h_out:     Vec<Complex<f32>>,
    p_out:     Vec<Complex<f32>>,
    /// PartialEq per-bin gain scratch; Exciter input snapshot.
    gains:     Vec<f32>,
    src:       Vec<Complex<f32>>,
    #[cfg(any(test, feature = "probe"))]
    last_probe: crate::dsp::modules::ProbeSnapshot,
}

impl HarmonicModule {
    pub fn new() -> Self {
        Self {
            mode:        HarmonicMode::default(),
            sample_rate: 48_000.0,
            fft_size:    2048,
            num_bins:    0,
            hp_hist:   [Vec::new(), Vec::new()],
            hp_pos:    [0; 2],
            hp_filled: [0; 2],
            mags:      Vec::new(),
            h_out:     Vec::new(),
            p_out:     Vec::new(),
            gains:     Vec::new(),
            src:       Vec::new(),
            #[cfg(any(test, feature = "probe"))]
            last_probe: Default::default(),
        }
    }

    pub fn set_mode(&mut self, mode: HarmonicMode) {
        if mode != self.mode {
            self.clear_state();
            self.mode = mode;
        }
    }

    pub fn mode(&self) -> HarmonicMode { self.mode }

    // ── H/P Split ─────────────────────────────────────────────────────────

    fn process_hp_split(&mut self, ch: usize, bins: &mut [Complex<f32>], curves: &[&[f32]]) {
        let n = bins.len();
        let nb = self.num_bins;

        for k in 0..n { self.mags[k] = bins[k].norm(); }

        // Push this frame into the ring.
        let pos = self.hp_pos[ch];
        self.hp_hist[ch][pos * nb..pos * nb + n].copy_from_slice(&self.mags[..n]);
        self.hp_pos[ch] = (pos + 1) % HP_MAX_TIME;
        self.hp_filled[ch] = (self.hp_filled[ch] + 1).min(HP_MAX_TIME);
        let filled = self.hp_filled[ch];

        let mut tbuf = [0.0f32; HP_MAX_TIME];
        let mut fbuf = [0.0f32; 2 * HP_MAX_FREQ_HALF + 1];

        for k in 0..n {
            let reach = curves[3][k].clamp(0.0, 1.0);
            let sep   = curves[0][k].clamp(0.0, 1.0);
            let bal   = bipolar(curves[1][k]);
            let mix   = curves[4][k].clamp(0.0, 1.0);

            // Time median over the newest `len` frames (odd length, ≤ filled).
            let want = 3 + 2 * (reach * ((HP_MAX_TIME - 3) / 2) as f32).round() as usize;
            let len  = want.min(filled).max(1);
            for i in 0..len {
                let f = (pos + HP_MAX_TIME - i) % HP_MAX_TIME;
                tbuf[i] = self.hp_hist[ch][f * nb + k];
            }
            let harm = median_in_place(&mut tbuf[..len]);

            // Frequency median over ±half bins in the current frame.
            let half = 1 + (reach * (HP_MAX_FREQ_HALF - 1) as f32).round() as usize;
            let lo = k.saturating_sub(half);
            let hi = (k + half).min(n - 1);
            let cnt = hi - lo + 1;
            fbuf[..cnt].copy_from_slice(&self.mags[lo..=hi]);
            let perc = median_in_place(&mut fbuf[..cnt]);

            // Wiener-style soft mask, then SEPARATE pulls it toward 50/50.
            let h2 = harm * harm;
            let p2 = perc * perc;
            let mh_full = if h2 + p2 > HP_SILENCE { h2 / (h2 + p2) } else { 0.5 };
            let mh = 0.5 + (mh_full - 0.5) * sep;

            let x = bins[k];
            self.h_out[k] = x * mh;
            self.p_out[k] = x * (1.0 - mh);

            // BALANCE: 0 keeps both (exact passthrough), −1 keeps only H, +1 only P.
            // Written as 1 + deltas so neutral BALANCE is exactly 1.0.
            let g_h = (1.0 - bal).min(1.0);
            let g_p = (1.0 + bal).min(1.0);
            let wet_gain = 1.0 + mh * (g_h - 1.0) + (1.0 - mh) * (g_p - 1.0);
            bins[k] = x * (1.0 - mix + mix * wet_gain);
        }
    }

    // ── Partial EQ ────────────────────────────────────────────────────────

    fn process_partial_eq(&mut self, bins: &mut [Complex<f32>], curves: &[&[f32]], ctx: &ModuleContext<'_>) {
        let n = bins.len();
        let groups = match ctx.harmonic_groups {
            Some(g) if !g.is_empty() => g,
            _ => return, // no detected voices → passthrough
        };
        let bin_hz = self.sample_rate / self.fft_size as f32;
        self.gains[..n].fill(1.0);

        for g in groups {
            if g.fundamental_hz <= 0.0 { continue; }
            for i in 1..g.harmonic_count as usize {
                let b = g.harmonic_bins[i] as usize;
                if b >= n { continue; }
                let h = ((b as f32 * bin_hz) / g.fundamental_hz).round().max(1.0);
                if h < 2.0 { continue; }

                let amount = curves[0][b].clamp(0.0, 1.0);
                let oe     = bipolar(curves[1][b]);
                let tilt   = bipolar(curves[2][b]);
                let even   = (h as u32) % 2 == 0;
                let g_oe   = if even { 1.0 + oe } else { 1.0 - oe };
                let g_tilt = h.powf(tilt);
                let gain   = (g_oe * g_tilt).clamp(0.0, PEQ_MAX_GAIN);
                let gain   = 1.0 + (gain - 1.0) * amount;

                let skirt = (curves[3][b].clamp(0.0, 1.0) * PEQ_MAX_SKIRT).round() as usize;
                let lo = b.saturating_sub(skirt);
                let hi = (b + skirt).min(n - 1);
                for k in lo..=hi { self.gains[k] = gain; }
            }
        }

        for k in 0..n {
            let mix = curves[4][k].clamp(0.0, 1.0);
            bins[k] *= 1.0 - mix + mix * self.gains[k];
        }
    }

    // ── Exciter ───────────────────────────────────────────────────────────

    fn process_exciter(&mut self, bins: &mut [Complex<f32>], curves: &[&[f32]]) {
        let n = bins.len();
        if n < 4 { return; }
        self.src[..n].copy_from_slice(bins);

        let mut peak = 0.0f32;
        for k in 0..n { peak = peak.max(self.src[k].norm()); }
        if peak <= 0.0 { return; }

        let last = n - 1; // Nyquist must stay real — never written.
        for k in 1..n {
            if 2 * k >= last { break; }
            let x = self.src[k];
            let m = x.norm();
            if m <= 0.0 { continue; }

            let sens = curves[3][k].clamp(0.0, 1.0);
            let thresh = peak * 10f32.powf(EXC_MIN_THRESH_DB * sens / 20.0);
            if m < thresh { continue; }

            let drive = curves[0][k].clamp(0.0, 1.0) * EXC_MAX_LEVEL;
            let mix   = curves[4][k].clamp(0.0, 1.0);
            if drive * mix <= 0.0 { continue; }
            let oe    = bipolar(curves[1][k]);
            let tilt  = bipolar(curves[2][k]);
            let w_even = 0.5 * (1.0 + oe);
            let w_odd  = 0.5 * (1.0 - oe);
            let phi = x.arg();

            for h in 2..=EXC_MAX_HARMONIC {
                let j = h * k;
                if j >= last { break; }
                let w = if h % 2 == 0 { w_even } else { w_odd };
                // Natural 1/(h−1) falloff; TILT +1 flattens it, −1 squares it.
                let falloff = (1.0 / (h - 1) as f32).powf(1.0 - tilt);
                let level = drive * w * falloff * m * mix;
                if level > 0.0 {
                    bins[j] += Complex::from_polar(level, h as f32 * phi);
                }
            }
        }
    }
}

impl Default for HarmonicModule {
    fn default() -> Self { Self::new() }
}

impl SpectralModule for HarmonicModule {
    fn reset(&mut self, sample_rate: f32, fft_size: usize) {
        self.sample_rate = sample_rate;
        self.fft_size    = fft_size;
        self.num_bins    = fft_size / 2 + 1;
        let nb = self.num_bins;
        for ch in 0..2 {
            self.hp_hist[ch].clear();
            self.hp_hist[ch].resize(HP_MAX_TIME * nb, 0.0);
        }
        self.mags.clear();
        self.mags.resize(nb, 0.0);
        self.h_out.clear();
        self.h_out.resize(nb, Complex::new(0.0, 0.0));
        self.p_out.clear();
        self.p_out.resize(nb, Complex::new(0.0, 0.0));
        self.gains.clear();
        self.gains.resize(nb, 1.0);
        self.src.clear();
        self.src.resize(nb, Complex::new(0.0, 0.0));
        self.hp_pos    = [0; 2];
        self.hp_filled = [0; 2];
    }

    fn clear_state(&mut self) {
        for ch in 0..2 { self.hp_hist[ch].fill(0.0); }
        self.hp_pos    = [0; 2];
        self.hp_filled = [0; 2];
        self.h_out.fill(Complex::new(0.0, 0.0));
        self.p_out.fill(Complex::new(0.0, 0.0));
    }

    fn process(
        &mut self,
        channel: usize,
        _stereo_link: StereoLink,
        _target: FxChannelTarget,
        bins: &mut [Complex<f32>],
        _sidechain: Option<&[f32]>,
        curves: &[&[f32]],
        suppression_out: &mut [f32],
        _physics: Option<&mut crate::dsp::bin_physics::BinPhysics>,
        ctx: &ModuleContext<'_>,
    ) {
        suppression_out.fill(0.0);
        let n = bins.len();
        if n == 0 || n > self.num_bins || curves.len() < 5 { return; }
        let ch = channel.min(1);

        match self.mode {
            HarmonicMode::HpSplit   => self.process_hp_split(ch, bins, curves),
            HarmonicMode::PartialEq => self.process_partial_eq(bins, curves, ctx),
            HarmonicMode::Exciter   => self.process_exciter(bins, curves),
        }

        #[cfg(any(test, feature = "probe"))]
        {
            let k = n / 2;
            self.last_probe = crate::dsp::modules::ProbeSnapshot {
                amount_pct: Some(curves[0][k].clamp(0.0, 1.0) * 100.0),
                mix_pct:    Some(curves[4][k].clamp(0.0, 1.0) * 100.0),
                ..Default::default()
            };
        }
    }

    fn tail_length(&self) -> u32 {
        // HpSplit's time median looks back HP_MAX_TIME hops.
        (HP_MAX_TIME * self.fft_size / 4) as u32
    }

    fn module_type(&self) -> ModuleType { ModuleType::Harmonic }
    fn num_curves(&self) -> usize { 5 }
    fn num_outputs(&self) -> Option<usize> {
        if self.mode == HarmonicMode::HpSplit { Some(2) } else { None }
    }

    fn set_harmonic_mode(&mut self, mode: HarmonicMode) { self.set_mode(mode); }

    fn virtual_outputs(&self) -> Option<[&[Complex<f32>]; 2]> {
        if self.mode == HarmonicMode::HpSplit {
            Some([&self.h_out, &self.p_out])
        } else {
            None
        }
    }

    #[cfg(any(test, feature = "probe"))]
    fn last_probe(&self) -> crate::dsp::modules::ProbeSnapshot { self.last_probe }
}

/// Per-mode `CurveLayout` for Harmonic.
///
/// Curves: 0=AMOUNT, 1=BALANCE, 2=TILT, 3=REACH, 4=MIX.
/// - HpSplit:   reads 0, 1, 3, 4 — TILT(2) unused.
/// - PartialEq: reads all five.
/// - Exciter:   reads all five.
pub fn active_layout(mode_byte: u8) -> super::CurveLayout {
    match mode_byte {
        0 => super::CurveLayout {
            active:          &[0, 1, 3, 4],
            label_overrides: &[(0, "SEPARATE"), (1, "H/P BAL"), (3, "SIZE")],
            help_for:        |_| "",
            mode_overview:   Some("H/P Split: separates steady (harmonic) from broadband (percussive) energy. The H and P streams appear as virtual rows in the matrix; H/P BAL weights them in this slot's own output."),
        },
        1 => super::CurveLayout {
            active:          &[0, 1, 2, 3, 4],
            label_overrides: &[(1, "ODD/EVEN"), (3, "WIDTH")],
            help_for:        |_| "",
            mode_overview:   Some("Partial EQ: gain by partial number (2nd, 3rd, …) of each detected note, wherever it sits in frequency. Non-harmonic bins pass untouched."),
        },
        _ => super::CurveLayout {
            active:          &[0, 1, 2, 3, 4],
            label_overrides: &[(0, "DRIVE"), (1, "ODD/EVEN"), (2, "BRIGHT"), (3, "SENS")],
            help_for:        |_| "",
            mode_overview:   Some("Exciter: adds phase-coherent 2nd–5th harmonics above each loud bin. DRIVE sets their level (up to −12 dB re source)."),
        },
    }
}
