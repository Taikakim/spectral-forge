//! Phase rotation helper used by Past Stretch and (later) Future / Punch.
//!
//! Encapsulates the operation: rotate a complex bin by `2π · freq_offset ·
//! time_delta` radians. Uses a 1024-entry sin/cos LUT to avoid trig calls in
//! the hot path. Cost per call: ~6 muls + 3 adds + 2 LUT loads.

use num_complex::Complex;

const LUT_SIZE: usize = 1024;
const LUT_SIZE_F: f32 = LUT_SIZE as f32;
const TAU: f32 = std::f32::consts::TAU;

pub struct PhaseRotator {
    sin_lut: [f32; LUT_SIZE],
    cos_lut: [f32; LUT_SIZE],
}

impl PhaseRotator {
    pub fn new() -> Self {
        let mut sin_lut = [0.0_f32; LUT_SIZE];
        let mut cos_lut = [0.0_f32; LUT_SIZE];
        for i in 0..LUT_SIZE {
            let theta = (i as f32 / LUT_SIZE_F) * TAU;
            sin_lut[i] = theta.sin();
            cos_lut[i] = theta.cos();
        }
        Self { sin_lut, cos_lut }
    }

    /// Rotate `c` by `2π · freq_offset · time_delta` radians.
    /// `freq_offset` is in cycles-per-hop (or any unit such that
    /// `freq_offset · time_delta` is in cycles). The product is wrapped into
    /// [0, 1) before LUT lookup.
    #[inline]
    pub fn rotate(&self, c: Complex<f32>, freq_offset: f32, time_delta: f32) -> Complex<f32> {
        let cycles = freq_offset * time_delta;
        // Wrap to [0, 1). Take fract; if negative, add 1.
        let frac = cycles - cycles.floor();
        let idx = (frac * LUT_SIZE_F) as usize % LUT_SIZE;
        let cs = self.cos_lut[idx];
        let sn = self.sin_lut[idx];
        Complex::new(
            c.re * cs - c.im * sn,
            c.re * sn + c.im * cs,
        )
    }
}

impl Default for PhaseRotator {
    fn default() -> Self { Self::new() }
}
