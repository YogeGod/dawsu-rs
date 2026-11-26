//! Cross-platform real-time audio thread priority & EcoQoS promotion helpers.

use std::marker::PhantomData;

/// RAII thread-priority guard held by a dedicated audio worker thread on its owning OS thread.
pub(crate) struct RtThreadGuard {
    #[cfg(windows)]
    mmcss_handle: Option<windows::Win32::Foundation::HANDLE>,
    _own_thread_only: PhantomData<*mut ()>,
}

/// Promotes the current dedicated audio worker thread (`MMCSS "Pro Audio"` + EcoQoS off on Windows,
/// `QOS_CLASS_USER_INTERACTIVE` on macOS, `SCHED_FIFO` best-effort on Linux).
pub(crate) fn promote_current_thread() -> RtThreadGuard {
    #[cfg(windows)]
    {
        use std::ffi::c_void;
        use windows::Win32::System::Threading::{
            AvSetMmThreadCharacteristicsW, GetCurrentThread, SetThreadInformation,
            THREAD_POWER_THROTTLING_CURRENT_VERSION, THREAD_POWER_THROTTLING_EXECUTION_SPEED,
            THREAD_POWER_THROTTLING_STATE, ThreadPowerThrottling,
        };
        use windows::core::w;

        let throttling = THREAD_POWER_THROTTLING_STATE {
            Version: THREAD_POWER_THROTTLING_CURRENT_VERSION,
            ControlMask: THREAD_POWER_THROTTLING_EXECUTION_SPEED,
            StateMask: 0,
        };
        // SAFETY: `throttling` is valid for the duration of `SetThreadInformation` on the
        // current pseudo-handle and opts the thread out of EcoQoS efficiency cores.
        let _ = unsafe {
            SetThreadInformation(
                GetCurrentThread(),
                ThreadPowerThrottling,
                (&raw const throttling).cast::<c_void>(),
                size_of::<THREAD_POWER_THROTTLING_STATE>() as u32,
            )
        };

        let mut task_index = 0u32;
        // SAFETY: `"Pro Audio"` is a NUL-terminated static UTF-16 task name; `task_index` is valid.
        let mmcss_handle = unsafe { AvSetMmThreadCharacteristicsW(w!("Pro Audio"), &mut task_index) }.ok();
        RtThreadGuard {
            mmcss_handle,
            _own_thread_only: PhantomData,
        }
    }

    #[cfg(target_os = "macos")]
    {
        promote_macos_user_interactive();
        RtThreadGuard {
            _own_thread_only: PhantomData,
        }
    }

    #[cfg(target_os = "linux")]
    {
        promote_linux_fifo();
        RtThreadGuard {
            _own_thread_only: PhantomData,
        }
    }

    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    {
        RtThreadGuard {
            _own_thread_only: PhantomData,
        }
    }
}

/// Promotes a foreign callback thread (such as a `cpal` fallback stream callback) once without
/// retaining a cross-thread teardown handle.
pub(crate) fn promote_callback_thread() {
    #[cfg(windows)]
    {
        use std::ffi::c_void;
        use windows::Win32::System::Threading::{
            GetCurrentThread, SetThreadInformation, SetThreadPriority,
            THREAD_POWER_THROTTLING_CURRENT_VERSION, THREAD_POWER_THROTTLING_EXECUTION_SPEED,
            THREAD_POWER_THROTTLING_STATE, THREAD_PRIORITY_TIME_CRITICAL, ThreadPowerThrottling,
        };

        let throttling = THREAD_POWER_THROTTLING_STATE {
            Version: THREAD_POWER_THROTTLING_CURRENT_VERSION,
            ControlMask: THREAD_POWER_THROTTLING_EXECUTION_SPEED,
            StateMask: 0,
        };
        // SAFETY: Current-thread pseudo-handle; no handle escapes this function.
        unsafe {
            let thread = GetCurrentThread();
            let _ = SetThreadInformation(
                thread,
                ThreadPowerThrottling,
                (&raw const throttling).cast::<c_void>(),
                size_of::<THREAD_POWER_THROTTLING_STATE>() as u32,
            );
            let _ = SetThreadPriority(thread, THREAD_PRIORITY_TIME_CRITICAL);
        }
    }

    #[cfg(target_os = "macos")]
    promote_macos_user_interactive();

    #[cfg(target_os = "linux")]
    promote_linux_fifo();
}

impl Drop for RtThreadGuard {
    fn drop(&mut self) {
        #[cfg(windows)]
        if let Some(handle) = self.mmcss_handle.take() {
            // SAFETY: `!Send` guarantees this drops on the same thread that registered MMCSS.
            let _ = unsafe {
                windows::Win32::System::Threading::AvRevertMmThreadCharacteristics(handle)
            };
        }
    }
}

#[cfg(target_os = "macos")]
fn promote_macos_user_interactive() {
    const QOS_CLASS_USER_INTERACTIVE: u32 = 0x21;
    unsafe extern "C" {
        fn pthread_set_qos_class_self_np(qos_class: u32, relative_priority: i32) -> i32;
    }
    // SAFETY: Standard libSystem QoS hint on the calling thread.
    let _ = unsafe { pthread_set_qos_class_self_np(QOS_CLASS_USER_INTERACTIVE, 0) };
}

#[cfg(target_os = "linux")]
fn promote_linux_fifo() {
    #[repr(C)]
    struct SchedParam {
        sched_priority: i32,
    }
    const SCHED_FIFO: i32 = 1;
    unsafe extern "C" {
        fn sched_setscheduler(pid: i32, policy: i32, param: *const SchedParam) -> i32;
    }
    let param = SchedParam { sched_priority: 20 };
    // SAFETY: Best-effort real-time priority request (`pid == 0` = caller thread); EPERM is ignored.
    let _ = unsafe { sched_setscheduler(0, SCHED_FIFO, &raw const param) };
}
