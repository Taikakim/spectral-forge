// src/dsp/guard.rs

/// Clamp NaN and Inf to 0.0 before FFT.
pub fn sanitize(buf: &mut [f32]) {
    for s in buf.iter_mut() {
        if !s.is_finite() {
            *s = 0.0;
        }
    }
}

/// Set FTZ (bit 15) and DAZ (bit 6) in MXCSR to flush denormals to zero.
/// No-op on other architectures.
pub fn flush_denormals() {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        let mut mxcsr: u32 = 0;
        core::arch::asm!(
            "stmxcsr [{0}]",
            in(reg) &mut mxcsr,
            options(nostack),
        );
        mxcsr |= 0x8040; // FTZ (bit 15) | DAZ (bit 6)
        core::arch::asm!(
            "ldmxcsr [{0}]",
            in(reg) &mxcsr,
            options(nostack),
        );
    }
}

/// Returns false if SharedState is not yet initialised.
/// Guards against buggy hosts calling process() before initialize().
pub fn is_ready<T>(state: &Option<T>) -> bool {
    state.is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_clamps_nan() {
        let mut buf = [f32::NAN, 1.0, f32::INFINITY, -f32::INFINITY, 0.5];
        sanitize(&mut buf);
        assert_eq!(buf, [0.0, 1.0, 0.0, 0.0, 0.5]);
    }

    #[test]
    fn sanitize_passes_finite() {
        let mut buf = [0.0f32, 0.5, -0.5, 1.0, -1.0];
        let original = buf;
        sanitize(&mut buf);
        assert_eq!(buf, original);
    }

    #[test]
    fn is_ready_none() {
        let s: Option<u8> = None;
        assert!(!is_ready(&s));
    }

    #[test]
    fn is_ready_some() {
        let s: Option<u8> = Some(1);
        assert!(is_ready(&s));
    }

    #[test]
    #[cfg(target_arch = "x86_64")]
    fn flush_denormals_sets_ftz_daz() {
        flush_denormals();
        let mut mxcsr: u32 = 0;
        unsafe {
            core::arch::asm!("stmxcsr [{0}]", in(reg) &mut mxcsr, options(nostack));
        }
        assert!(mxcsr & 0x8000 != 0, "FTZ bit not set");
        assert!(mxcsr & 0x0040 != 0, "DAZ bit not set");
    }
}
