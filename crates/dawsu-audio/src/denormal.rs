//! Scoped floating-point denormal (FTZ/DAZ) guard for real-time audio callbacks.

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[inline]
unsafe fn read_mxcsr() -> u32 {
    let mut mxcsr = 0u32;
    // SAFETY: Caller ensures SSE2 is present; `stmxcsr` stores 32-bit MXCSR into stack slot.
    unsafe {
        core::arch::asm!(
            "stmxcsr [{ptr}]",
            ptr = in(reg) &mut mxcsr,
            options(nostack, preserves_flags),
        );
    }
    mxcsr
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[inline]
unsafe fn write_mxcsr(mxcsr: u32) {
    // SAFETY: Caller ensures SSE2 is present; `ldmxcsr` loads 32-bit MXCSR from stack slot.
    unsafe {
        core::arch::asm!(
            "ldmxcsr [{ptr}]",
            ptr = in(reg) &mxcsr,
            options(readonly, nostack, preserves_flags),
        );
    }
}

/// Enables Flush-To-Zero (FTZ) and Denormals-Are-Zero (DAZ) while alive, restoring the thread's
/// previous floating-point control register on drop.
pub struct DenormalGuard {
    #[cfg(target_arch = "x86_64")]
    prev_mxcsr: u32,
    #[cfg(target_arch = "x86")]
    prev_mxcsr: Option<u32>,
    #[cfg(target_arch = "aarch64")]
    prev_fpcr: u64,
}

impl DenormalGuard {
    /// Activates hardware denormal flushing on the current thread and returns an RAII guard.
    #[inline]
    pub fn enter() -> Self {
        #[cfg(target_arch = "x86_64")]
        {
            const FTZ_DAZ_MASK: u32 = 0x8040;
            // SAFETY: SSE2 / MXCSR are baseline on all x86_64 targets.
            let prev_mxcsr = unsafe {
                let prev = read_mxcsr();
                write_mxcsr(prev | FTZ_DAZ_MASK);
                prev
            };
            Self { prev_mxcsr }
        }
        #[cfg(target_arch = "x86")]
        {
            const FTZ_DAZ_MASK: u32 = 0x8040;
            let prev_mxcsr = if std::arch::is_x86_feature_detected!("sse2") {
                // SAFETY: Feature detection confirmed SSE2 MXCSR support.
                unsafe {
                    let prev = read_mxcsr();
                    write_mxcsr(prev | FTZ_DAZ_MASK);
                    Some(prev)
                }
            } else {
                None
            };
            Self { prev_mxcsr }
        }
        #[cfg(target_arch = "aarch64")]
        {
            const FZ_BIT: u64 = 1 << 24;
            let mut prev_fpcr: u64;
            // SAFETY: Reading and writing FPCR bit 24 (FZ) affects only current-thread FP flags.
            unsafe {
                core::arch::asm!("mrs {}, fpcr", out(reg) prev_fpcr, options(nomem, nostack, preserves_flags));
                core::arch::asm!("msr fpcr, {}", in(reg) (prev_fpcr | FZ_BIT), options(nomem, nostack, preserves_flags));
            }
            Self { prev_fpcr }
        }
        #[cfg(not(any(target_arch = "x86_64", target_arch = "x86", target_arch = "aarch64")))]
        {
            Self {}
        }
    }
}

impl Drop for DenormalGuard {
    #[inline]
    fn drop(&mut self) {
        #[cfg(target_arch = "x86_64")]
        {
            // SAFETY: Restores the MXCSR value saved in `enter()` on the same thread.
            unsafe { write_mxcsr(self.prev_mxcsr) };
        }
        #[cfg(target_arch = "x86")]
        {
            if let Some(prev) = self.prev_mxcsr {
                // SAFETY: Restores the MXCSR value saved in `enter()` on the same thread.
                unsafe { write_mxcsr(prev) };
            }
        }
        #[cfg(target_arch = "aarch64")]
        {
            // SAFETY: Restores the FPCR value saved in `enter()` on the same thread.
            unsafe {
                core::arch::asm!("msr fpcr, {}", in(reg) self.prev_fpcr, options(nomem, nostack, preserves_flags));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn denormal_guard_flushes_subnormals_and_restores_register() {
        #[cfg(target_arch = "x86_64")]
        let before = unsafe { read_mxcsr() };

        {
            let _guard = DenormalGuard::enter();
            let tiny = std::hint::black_box(f32::from_bits(1));
            assert_eq!(tiny + std::hint::black_box(0.0_f32), 0.0_f32);
            assert_eq!(tiny * std::hint::black_box(1.0_f32), 0.0_f32);
        }

        #[cfg(target_arch = "x86_64")]
        {
            let after = unsafe { read_mxcsr() };
            assert_eq!(after, before);
        }
    }
}
