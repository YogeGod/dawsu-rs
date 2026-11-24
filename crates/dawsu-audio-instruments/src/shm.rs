//! Cross-platform shared-memory audio/MIDI exchange with `dawsu_vst3_host`.
//!
//! Mirrors the Win32 `Local\<name>`, `Local\<name>-req`, and `Local\<name>-done` objects
//! expected by `native/vst3-host/host.cpp` (Protocol v3).

use crate::host::{HostError, MAX_EVENTS, MAX_FRAMES, PROTOCOL_MAGIC, PROTOCOL_VERSION, ShmBlock, ShmEvent};

#[cfg(windows)]
mod imp {
    use super::*;
    use std::ffi::c_void;
    use std::ptr::null_mut;
    use std::sync::atomic::{Ordering, fence};

    type Handle = *mut c_void;
    const INVALID_HANDLE_VALUE: Handle = -1isize as Handle;
    const PAGE_READWRITE: u32 = 0x04;
    const FILE_MAP_ALL_ACCESS: u32 = 0x000F_001F;
    const WAIT_OBJECT_0: u32 = 0x0000_0000;
    const BLOCK_TIMEOUT_MS: u32 = 15;

    unsafe extern "system" {
        fn CreateFileMappingW(
            h_file: Handle,
            lp_attributes: *mut c_void,
            fl_protect: u32,
            dw_max_size_high: u32,
            dw_max_size_low: u32,
            lp_name: *const u16,
        ) -> Handle;
        fn MapViewOfFile(
            h_file_mapping: Handle,
            dw_desired_access: u32,
            dw_file_offset_high: u32,
            dw_file_offset_low: u32,
            dw_number_of_bytes_to_map: usize,
        ) -> *mut c_void;
        fn UnmapViewOfFile(lp_base_address: *const c_void) -> i32;
        fn CreateEventW(
            lp_event_attributes: *mut c_void,
            b_manual_reset: i32,
            b_initial_state: i32,
            lp_name: *const u16,
        ) -> Handle;
        fn SetEvent(h_event: Handle) -> i32;
        fn WaitForSingleObject(h_handle: Handle, dw_milliseconds: u32) -> u32;
        fn CloseHandle(h_object: Handle) -> i32;
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub struct ShmChannel {
        mapping: Handle,
        req_event: Handle,
        done_event: Handle,
        block: *mut ShmBlock,
        seq: u32,
    }

    // Exclusive ownership of the mapped view and Win32 event handles; safe to move to the audio thread.
    unsafe impl Send for ShmChannel {}

    impl ShmChannel {
        pub fn create(name: &str, sample_rate: f64) -> Result<Self, HostError> {
            let size = std::mem::size_of::<ShmBlock>();
            let size_high = ((size as u64) >> 32) as u32;
            let size_low = size as u32;
            let map_name = wide(&format!("Local\\{name}"));
            let req_name = wide(&format!("Local\\{name}-req"));
            let done_name = wide(&format!("Local\\{name}-done"));

            let mapping = unsafe {
                CreateFileMappingW(
                    INVALID_HANDLE_VALUE,
                    null_mut(),
                    PAGE_READWRITE,
                    size_high,
                    size_low,
                    map_name.as_ptr(),
                )
            };
            if mapping.is_null() {
                return Err(HostError::SpawnFailed(format!(
                    "CreateFileMappingW failed for {name}"
                )));
            }

            let view = unsafe { MapViewOfFile(mapping, FILE_MAP_ALL_ACCESS, 0, 0, size) };
            if view.is_null() {
                unsafe { CloseHandle(mapping) };
                return Err(HostError::SpawnFailed(format!(
                    "MapViewOfFile failed for {name}"
                )));
            }

            let req_event = unsafe { CreateEventW(null_mut(), 0, 0, req_name.as_ptr()) };
            let done_event = unsafe { CreateEventW(null_mut(), 0, 0, done_name.as_ptr()) };
            if req_event.is_null() || done_event.is_null() {
                if !req_event.is_null() {
                    unsafe { CloseHandle(req_event) };
                }
                if !done_event.is_null() {
                    unsafe { CloseHandle(done_event) };
                }
                unsafe {
                    UnmapViewOfFile(view);
                    CloseHandle(mapping);
                }
                return Err(HostError::SpawnFailed(format!(
                    "CreateEventW failed for {name}"
                )));
            }

            let block = view.cast::<ShmBlock>();
            unsafe {
                std::ptr::write_bytes(block, 0, 1);
                (*block).magic = PROTOCOL_MAGIC;
                (*block).version = PROTOCOL_VERSION;
                (*block).sample_rate = if sample_rate > 0.0 { sample_rate } else { 44_100.0 };
            }
            fence(Ordering::Release);

            Ok(Self {
                mapping,
                req_event,
                done_event,
                block,
                seq: 0,
            })
        }

        /// Processes one stereo block through the live VST3 sidecar.
        /// Returns `true` if rendered cleanly, `false` on timeout or plugin error (fills silence).
        pub fn process_block(
            &mut self,
            sample_rate: f64,
            events: &[ShmEvent],
            left: &mut [f32],
            right: &mut [f32],
        ) -> bool {
            let frames = left.len().min(right.len()).min(MAX_FRAMES);
            if frames == 0 {
                return true;
            }
            let event_count = events.len().min(MAX_EVENTS);
            self.seq = self.seq.wrapping_add(1).max(1);
            let target_seq = self.seq;

            unsafe {
                let shm = &mut *self.block;
                shm.frames = frames as i32;
                shm.event_count = event_count as i32;
                shm.has_audio_in = 0;
                shm.sample_rate = if sample_rate > 0.0 { sample_rate } else { 44_100.0 };
                for (i, ev) in events.iter().take(event_count).enumerate() {
                    shm.events[i] = *ev;
                }
                fence(Ordering::Release);
                shm.request_seq = target_seq;
                SetEvent(self.req_event);
            }

            let wait = unsafe { WaitForSingleObject(self.done_event, BLOCK_TIMEOUT_MS) };
            if wait != WAIT_OBJECT_0 {
                left[..frames].fill(0.0);
                right[..frames].fill(0.0);
                return false;
            }

            fence(Ordering::Acquire);
            unsafe {
                let shm = &*self.block;
                if shm.done_seq != target_seq || shm.status != 0 {
                    left[..frames].fill(0.0);
                    right[..frames].fill(0.0);
                    return false;
                }
                left[..frames].copy_from_slice(&shm.left[..frames]);
                right[..frames].copy_from_slice(&shm.right[..frames]);
            }
            true
        }
    }

    impl Drop for ShmChannel {
        fn drop(&mut self) {
            unsafe {
                if !self.block.is_null() {
                    UnmapViewOfFile(self.block.cast());
                }
                if !self.req_event.is_null() {
                    CloseHandle(self.req_event);
                }
                if !self.done_event.is_null() {
                    CloseHandle(self.done_event);
                }
                if !self.mapping.is_null() {
                    CloseHandle(self.mapping);
                }
            }
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;

    pub struct ShmChannel;

    impl ShmChannel {
        pub fn create(_name: &str, _sample_rate: f64) -> Result<Self, HostError> {
            Err(HostError::SpawnFailed(
                "VST3 shared-memory host is not yet supported on this platform".to_string(),
            ))
        }

        pub fn process_block(
            &mut self,
            _sample_rate: f64,
            _events: &[ShmEvent],
            left: &mut [f32],
            right: &mut [f32],
        ) -> bool {
            left.fill(0.0);
            right.fill(0.0);
            false
        }
    }
}

pub use imp::ShmChannel;
