//! Shared scalar primitives for the Circuit module's analog-component kernels.
//! Designed for clean SIMD lift in v2 (target `wide::f32x8`); v1 is scalar.
//!
//! All functions are `#[inline]` and use bin-major loops with no inter-bin
//! dependencies except `spread_3tap`, which expects a pre-cleared output
//! buffer that aliases-free with `input`.

/// One step of a 1-pole lowpass: `state += (target - state) * alpha`.
/// `alpha` should already be clamped to `[0, 1]` by the caller.
#[inline]
pub fn lp_step(state: &mut f32, target: f32, alpha: f32) {
    *state += (target - *state) * alpha;
}

/// Levien-style 4th-order rational polynomial approximation of `tanh(x)`.
/// Max abs error in `|x| <= 1` is ~3%. Saturates monotonically beyond.
/// Branchless, no `exp()`, no `tanh()`. Approx 4 muls + 1 div per call.
///
/// Input is clamped to `[-3, 3]` before evaluation so that the approximation
/// saturates at exactly ±1.0 for large inputs. At x=±3 the rational form gives
/// ±3*(27+9)/(27+81) = ±36/36 = ±1.0, which is the correct saturation value.
/// Without the clamp the formula approaches x/9 for large |x| (unbounded).
#[inline]
pub fn tanh_levien_poly(x: f32) -> f32 {
    // Clamp to ensure proper saturation: at x=3, num/den = 3*36/108 = 1.0
    let x = x.clamp(-3.0, 3.0);
    // Reference: Raph Levien, "Approximating tanh", 2019.
    // tanh(x) ≈ x * (27 + x²) / (27 + 9 * x²)
    let x2 = x * x;
    let num = x * (27.0 + x2);
    let den = 27.0 + 9.0 * x2;
    num / den
}

/// Apply a 3-tap symmetric stencil: `output[k] = (1 - s) * input[k] + 0.5 * s * (input[k-1] + input[k+1])`.
/// Edges (k = 0, k = N-1) use zero-padded neighbours (no wrap, no replicate).
/// `s` is clamped to `[0, 1]` internally.
/// Caller must pass two distinct slices of the same length; aliasing is UB.
#[inline]
pub fn spread_3tap(input: &[f32], output: &mut [f32], strength: f32) {
    let s = strength.clamp(0.0, 1.0);
    let n = input.len();
    debug_assert_eq!(output.len(), n);
    if n == 0 { return; }
    if n == 1 { output[0] = input[0]; return; }

    // First bin: only right neighbour exists.
    output[0] = (1.0 - s) * input[0] + 0.5 * s * input[1];
    // Interior bins.
    for k in 1..n - 1 {
        output[k] = (1.0 - s) * input[k] + 0.5 * s * (input[k - 1] + input[k + 1]);
    }
    // Last bin: only left neighbour exists.
    output[n - 1] = (1.0 - s) * input[n - 1] + 0.5 * s * input[n - 2];
}

/// Per-channel xorshift32 PRNG. Cheap (3 shifts + 3 XORs per `next_u32`),
/// branchless, deterministic for a given seed. NOT cryptographically secure.
/// Used for: BBD dither, Drift, Slew phase scramble, Bias Fuzz noise.
#[derive(Debug, Clone)]
pub struct SimdRng {
    state: u32,
}

impl SimdRng {
    #[inline]
    pub fn new(seed: u32) -> Self {
        // Avoid the all-zero degenerate state.
        let s = if seed == 0 { 0xDEADBEEF } else { seed };
        Self { state: s }
    }

    #[inline]
    pub fn next_u32(&mut self) -> u32 {
        let mut s = self.state;
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        self.state = s;
        s
    }

    /// Uniform `f32` in `[-1, 1)`.
    ///
    /// Divides by 2^31 (= 2147483648.0) rather than `i32::MAX as f32`
    /// (= 2147483520.0, rounded) to ensure the lower bound -1.0 holds
    /// exactly. When the raw u32 maps to i32::MIN, the cast gives
    /// -2147483648.0; dividing by 2147483648.0 yields exactly -1.0,
    /// which satisfies `x >= -1.0`. With i32::MAX as divisor the same
    /// value would yield ≈ -1.0000001, failing the bound assertion.
    #[inline]
    pub fn next_f32_centered(&mut self) -> f32 {
        (self.next_u32() as i32 as f32) / ((1u32 << 31) as f32)
    }

    /// Uniform `f32` in `[0, 1)`.
    #[inline]
    pub fn next_f32_unit(&mut self) -> f32 {
        (self.next_u32() as f32) / (u32::MAX as f32)
    }
}
