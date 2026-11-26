//! Cross-platform shared-memory audio/MIDI exchange with `dawsu_vst3_host`.
//!
//! Mirrors the Win32 `Local\<name>`, `Local\<name>-req`, and `Local\<name>-done` objects
//! expected by `native/vst3-host/host.cpp` (Protocol v4).

use crate::host::{
    HostError, MAX_EVENTS, MAX_FRAMES, PROTOCOL_MAGIC, PROTOCOL_VERSION, SHM_FLAG_BYPASSED,
    SHM_FLAG_HAS_AUDIO_IN, ShmBlock, ShmEvent, ShmProcessContext,
};

#[cfg(windows)]
mod imp {
    use super::*;
    use std::ffi::c_void;
    use std::ptr::null_mut;
    use std::sync::atomic::Ordering;
    use std::time::{Duration, Instant};

    type Handle = *mut c_void;
    const INVALID_HANDLE_VALUE: Handle = -1isize as Handle;
    const PAGE_READWRITE: u32 = 0x04;
    const FILE_MAP_ALL_ACCESS: u32 = 0x000F_001F;
    const DEFAULT_SPIN_US: u64 = 100;

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
        in_flight_seq: u32,
        late_blocks: u64,
        max_spin_us: u64,
        timeout_override_ms: Option<u32>,
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
            let effective_sr = if sample_rate > 0.0 && sample_rate.is_finite() {
                sample_rate
            } else {
                44_100.0
            };
            unsafe {
                std::ptr::write_bytes(block, 0, 1);
                (*block).magic = PROTOCOL_MAGIC;
                (*block).version = PROTOCOL_VERSION;
                (*block).context = ShmProcessContext {
                    sample_rate: effective_sr,
                    ..ShmProcessContext::default()
                };
            }
            std::sync::atomic::fence(Ordering::Release);

            Ok(Self {
                mapping,
                req_event,
                done_event,
                block,
                in_flight_seq: 0,
                late_blocks: 0,
                max_spin_us: DEFAULT_SPIN_US,
                timeout_override_ms: None,
            })
        }

        pub fn set_timeout_override_ms(&mut self, timeout_ms: Option<u32>) {
            self.timeout_override_ms = timeout_ms;
        }

        pub fn set_max_spin_us(&mut self, spin_us: u64) {
            self.max_spin_us = spin_us;
        }

        pub fn late_blocks(&self) -> u64 {
            self.late_blocks
        }

        pub fn latency_samples(&self) -> u32 {
            unsafe { (*self.block).latency_samples.load(Ordering::Acquire) }
        }

        pub fn tail_samples(&self) -> u32 {
            unsafe { (*self.block).tail_samples.load(Ordering::Acquire) }
        }

        pub fn take_restart_flags(&self) -> u32 {
            unsafe { (*self.block).restart_flags.swap(0, Ordering::AcqRel) }
        }

        /// Processes one stereo instrument block through the live VST3 sidecar.
        pub fn process_block(
            &mut self,
            sample_rate: f64,
            events: &[ShmEvent],
            left: &mut [f32],
            right: &mut [f32],
        ) -> bool {
            self.process_audio_block(sample_rate, None, events, None, None, left, right, false)
        }

        /// Processes one stereo block (instrument or Fx with stereo audio input + ProcessContext)
        /// through the live Protocol v4 VST3 sidecar using hybrid spin+OS wait and an atomic
        /// `in_flight_seq` guard preventing SHM clobbering across slow sidecar blocks.
        #[allow(clippy::too_many_arguments)]
        pub fn process_audio_block(
            &mut self,
            sample_rate: f64,
            context: Option<&ShmProcessContext>,
            events: &[ShmEvent],
            in_left: Option<&[f32]>,
            in_right: Option<&[f32]>,
            out_left: &mut [f32],
            out_right: &mut [f32],
            bypassed: bool,
        ) -> bool {
            let frames = out_left.len().min(out_right.len()).min(MAX_FRAMES);
            if frames == 0 {
                return true;
            }

            // Atomic `in_flight_seq` race guard: if a previous request has not yet completed
            // (`done_seq != in_flight_seq`), do NOT touch shared buffers or events!
            let shm_ref = unsafe { &*self.block };
            if self.in_flight_seq != 0
                && shm_ref.done_seq.load(Ordering::Acquire) != self.in_flight_seq
            {
                self.late_blocks += 1;
                out_left[..frames].fill(0.0);
                out_right[..frames].fill(0.0);
                return false;
            }

            let effective_sr = if sample_rate > 0.0 && sample_rate.is_finite() {
                sample_rate
            } else {
                44_100.0
            };
            let event_count = events.len().min(MAX_EVENTS);
            let target_seq = self.in_flight_seq.wrapping_add(1).max(1);
            self.in_flight_seq = target_seq;

            let mut flags = if bypassed { SHM_FLAG_BYPASSED } else { 0 };
            if in_left.is_some() {
                flags |= SHM_FLAG_HAS_AUDIO_IN;
            }

            unsafe {
                let shm = &mut *self.block;
                shm.frames = frames as i32;
                shm.event_count = event_count as i32;
                shm.flags.store(flags, Ordering::Relaxed);
                shm.context = match context {
                    Some(ctx) => {
                        let mut c = *ctx;
                        c.sample_rate = effective_sr;
                        c
                    }
                    None => ShmProcessContext {
                        sample_rate: effective_sr,
                        ..ShmProcessContext::default()
                    },
                };
                for (i, ev) in events.iter().take(event_count).enumerate() {
                    shm.events[i] = *ev;
                }
                if let Some(src_l) = in_left {
                    let copy_l = src_l.len().min(frames);
                    shm.in_left[..copy_l].copy_from_slice(&src_l[..copy_l]);
                    if copy_l < frames {
                        shm.in_left[copy_l..frames].fill(0.0);
                    }
                    if let Some(src_r) = in_right {
                        let copy_r = src_r.len().min(frames);
                        shm.in_right[..copy_r].copy_from_slice(&src_r[..copy_r]);
                        if copy_r < frames {
                            shm.in_right[copy_r..frames].fill(0.0);
                        }
                    } else {
                        shm.in_right[..frames].copy_from_slice(&shm.in_left[..frames]);
                    }
                }

                // Drain any stale completion signal from a prior late block before posting `target_seq`.
                WaitForSingleObject(self.done_event, 0);
                shm.request_seq.store(target_seq, Ordering::Release);
                SetEvent(self.req_event);
            }

            // Hybrid wait: brief user-space spin (<= 100 us) before bounded OS wait (< buffer duration).
            let mut completed = shm_ref.done_seq.load(Ordering::Acquire) == target_seq;
            if completed {
                unsafe { WaitForSingleObject(self.done_event, 0) };
            } else if self.max_spin_us > 0 {
                let spin_deadline = Instant::now() + Duration::from_micros(self.max_spin_us);
                while Instant::now() < spin_deadline {
                    if shm_ref.done_seq.load(Ordering::Acquire) == target_seq {
                        completed = true;
                        unsafe { WaitForSingleObject(self.done_event, 0) };
                        break;
                    }
                    std::hint::spin_loop();
                }
            }

            if !completed {
                // Keep wait strictly below buffer duration (or test override) to avoid stalling audio.
                let buffer_ms = (frames as f64 / effective_sr) * 1000.0;
                let max_wait_ms = self
                    .timeout_override_ms
                    .unwrap_or_else(|| (buffer_ms * 0.85).floor().clamp(1.0, 12.0) as u32);
                let deadline = Instant::now() + Duration::from_millis(u64::from(max_wait_ms));
                loop {
                    let remaining_ms = deadline
                        .saturating_duration_since(Instant::now())
                        .as_millis()
                        .min(u128::from(u32::MAX)) as u32;
                    if remaining_ms == 0 {
                        completed = shm_ref.done_seq.load(Ordering::Acquire) == target_seq;
                        break;
                    }
                    let _ = unsafe { WaitForSingleObject(self.done_event, remaining_ms) };
                    if shm_ref.done_seq.load(Ordering::Acquire) == target_seq {
                        completed = true;
                        break;
                    }
                }
            }

            if !completed {
                // Leave `self.in_flight_seq == target_seq` intact until sidecar finishes this seq.
                self.late_blocks += 1;
                out_left[..frames].fill(0.0);
                out_right[..frames].fill(0.0);
                return false;
            }

            unsafe {
                let shm = &*self.block;
                if shm.status.load(Ordering::Acquire) != 0 {
                    out_left[..frames].fill(0.0);
                    out_right[..frames].fill(0.0);
                    return false;
                }
                out_left[..frames].copy_from_slice(&shm.left[..frames]);
                out_right[..frames].copy_from_slice(&shm.right[..frames]);
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

        pub fn set_timeout_override_ms(&mut self, _timeout_ms: Option<u32>) {}
        pub fn set_max_spin_us(&mut self, _spin_us: u64) {}
        pub fn late_blocks(&self) -> u64 {
            0
        }
        pub fn latency_samples(&self) -> u32 {
            0
        }
        pub fn tail_samples(&self) -> u32 {
            0
        }
        pub fn take_restart_flags(&self) -> u32 {
            0
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

        #[allow(clippy::too_many_arguments)]
        pub fn process_audio_block(
            &mut self,
            _sample_rate: f64,
            _context: Option<&ShmProcessContext>,
            _events: &[ShmEvent],
            _in_left: Option<&[f32]>,
            _in_right: Option<&[f32]>,
            out_left: &mut [f32],
            out_right: &mut [f32],
            _bypassed: bool,
        ) -> bool {
            out_left.fill(0.0);
            out_right.fill(0.0);
            false
        }
    }
}

pub use imp::ShmChannel;

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use crate::host::{CTX_FLAG_PLAYING, RESTART_LATENCY_CHANGED};
    use std::ffi::c_void;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    type Handle = *mut c_void;
    const FILE_MAP_ALL_ACCESS: u32 = 0x000F_001F;
    const EVENT_MODIFY_STATE: u32 = 0x0002;
    const SYNCHRONIZE: u32 = 0x0010_0000;
    const WAIT_OBJECT_0: u32 = 0;

    unsafe extern "system" {
        fn OpenFileMappingW(dw_desired_access: u32, b_inherit_handle: i32, lp_name: *const u16) -> Handle;
        fn MapViewOfFile(
            h_file_mapping: Handle,
            dw_desired_access: u32,
            dw_file_offset_high: u32,
            dw_file_offset_low: u32,
            dw_number_of_bytes_to_map: usize,
        ) -> *mut c_void;
        fn UnmapViewOfFile(lp_base_address: *const c_void) -> i32;
        fn OpenEventW(dw_desired_access: u32, b_inherit_handle: i32, lp_name: *const u16) -> Handle;
        fn SetEvent(h_event: Handle) -> i32;
        fn WaitForSingleObject(h_handle: Handle, dw_milliseconds: u32) -> u32;
        fn CloseHandle(h_object: Handle) -> i32;
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    #[test]
    fn protocol_v4_shm_effect_round_trip_and_metadata() {
        let name = format!("dawsu_v4_fx_test_{}", std::process::id());
        let mut channel = ShmChannel::create(&name, 48_000.0).unwrap();
        channel.set_timeout_override_ms(Some(500));
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_bg = stop.clone();
        let name_bg = name.clone();

        let worker = std::thread::spawn(move || {
            let map_name = wide(&format!("Local\\{name_bg}"));
            let req_name = wide(&format!("Local\\{name_bg}-req"));
            let done_name = wide(&format!("Local\\{name_bg}-done"));
            unsafe {
                let mapping = OpenFileMappingW(FILE_MAP_ALL_ACCESS, 0, map_name.as_ptr());
                assert!(!mapping.is_null());
                let view = MapViewOfFile(
                    mapping,
                    FILE_MAP_ALL_ACCESS,
                    0,
                    0,
                    std::mem::size_of::<ShmBlock>(),
                );
                assert!(!view.is_null());
                let shm = &mut *view.cast::<ShmBlock>();
                let req = OpenEventW(SYNCHRONIZE | EVENT_MODIFY_STATE, 0, req_name.as_ptr());
                let done = OpenEventW(SYNCHRONIZE | EVENT_MODIFY_STATE, 0, done_name.as_ptr());
                assert!(!req.is_null() && !done.is_null());
                let _ = ready_tx.send(());

                while !stop_bg.load(Ordering::Relaxed) {
                    if WaitForSingleObject(req, 50) != WAIT_OBJECT_0 {
                        continue;
                    }
                    let req_seq = shm.request_seq.load(Ordering::Acquire);
                    assert_ne!(shm.flags.load(Ordering::Relaxed) & SHM_FLAG_HAS_AUDIO_IN, 0);
                    assert_eq!(shm.context.time_sig_num, 3);
                    assert_eq!(shm.context.tempo, 175.0);
                    let n = shm.frames as usize;
                    for i in 0..n {
                        shm.left[i] = shm.in_left[i] * 0.5;
                        shm.right[i] = shm.in_right[i] * 0.25;
                    }
                    shm.latency_samples.store(256, Ordering::Release);
                    shm.tail_samples.store(1024, Ordering::Release);
                    shm.restart_flags.fetch_or(RESTART_LATENCY_CHANGED, Ordering::AcqRel);
                    shm.status.store(0, Ordering::Release);
                    shm.done_seq.store(req_seq, Ordering::Release);
                    SetEvent(done);
                }
                CloseHandle(req);
                CloseHandle(done);
                UnmapViewOfFile(view);
                CloseHandle(mapping);
            }
        });

        let ctx = ShmProcessContext {
            state_flags: CTX_FLAG_PLAYING,
            time_sig_num: 3,
            time_sig_den: 4,
            tempo: 175.0,
            ..ShmProcessContext::default()
        };
        let in_l = [0.8_f32; 64];
        let in_r = [0.4_f32; 64];
        let mut out_l = [0.0_f32; 64];
        let mut out_r = [0.0_f32; 64];

        ready_rx.recv().unwrap();
        let ok = channel.process_audio_block(
            48_000.0,
            Some(&ctx),
            &[],
            Some(&in_l),
            Some(&in_r),
            &mut out_l,
            &mut out_r,
            false,
        );
        stop.store(true, Ordering::Relaxed);
        worker.join().unwrap();

        assert!(ok);
        assert!((out_l[0] - 0.4).abs() < 1e-6);
        assert!((out_r[0] - 0.1).abs() < 1e-6);
        assert_eq!(channel.latency_samples(), 256);
        assert_eq!(channel.tail_samples(), 1024);
        assert_eq!(channel.take_restart_flags(), RESTART_LATENCY_CHANGED);
        assert_eq!(channel.take_restart_flags(), 0);
        assert_eq!(channel.late_blocks(), 0);
    }

    #[test]
    fn slow_synthetic_sidecar_triggers_late_blocks_without_corrupting_shm_and_recovers() {
        let name = format!("dawsu_v4_late_test_{}", std::process::id());
        let mut channel = ShmChannel::create(&name, 48_000.0).unwrap();
        channel.set_max_spin_us(0);
        channel.set_timeout_override_ms(Some(2));
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_bg = stop.clone();
        let name_bg = name.clone();
        let clobbered = Arc::new(AtomicBool::new(false));
        let clobbered_bg = clobbered.clone();

        let worker = std::thread::spawn(move || {
            let map_name = wide(&format!("Local\\{name_bg}"));
            let req_name = wide(&format!("Local\\{name_bg}-req"));
            let done_name = wide(&format!("Local\\{name_bg}-done"));
            unsafe {
                let mapping = OpenFileMappingW(FILE_MAP_ALL_ACCESS, 0, map_name.as_ptr());
                let view = MapViewOfFile(
                    mapping,
                    FILE_MAP_ALL_ACCESS,
                    0,
                    0,
                    std::mem::size_of::<ShmBlock>(),
                );
                let shm = &mut *view.cast::<ShmBlock>();
                let req = OpenEventW(SYNCHRONIZE | EVENT_MODIFY_STATE, 0, req_name.as_ptr());
                let done = OpenEventW(SYNCHRONIZE | EVENT_MODIFY_STATE, 0, done_name.as_ptr());
                let _ = ready_tx.send(());

                while !stop_bg.load(Ordering::Relaxed) {
                    if WaitForSingleObject(req, 50) != WAIT_OBJECT_0 {
                        continue;
                    }
                    let req_seq = shm.request_seq.load(Ordering::Acquire);
                    let in0 = shm.in_left[0];
                    if req_seq == 1 {
                        // Simulate slow block longer than caller timeout; verify caller never clobbers SHM!
                        std::thread::sleep(std::time::Duration::from_millis(35));
                    }
                    if shm.request_seq.load(Ordering::Acquire) != req_seq
                        || (shm.in_left[0] - in0).abs() > 1e-6
                    {
                        clobbered_bg.store(true, Ordering::Relaxed);
                    }
                    let n = shm.frames as usize;
                    for i in 0..n {
                        shm.left[i] = shm.in_left[i] * 2.0;
                        shm.right[i] = shm.in_right[i] * 2.0;
                    }
                    shm.status.store(0, Ordering::Release);
                    shm.done_seq.store(req_seq, Ordering::Release);
                    SetEvent(done);
                }
                CloseHandle(req);
                CloseHandle(done);
                UnmapViewOfFile(view);
                CloseHandle(mapping);
            }
        });

        let in_slow = [0.25_f32; 64];
        let in_attempt_2 = [0.75_f32; 64];
        let mut out_l = [1.0_f32; 64];
        let mut out_r = [1.0_f32; 64];

        ready_rx.recv().unwrap();
        // Block 1 times out -> false + silence + late_blocks = 1.
        let r1 = channel.process_audio_block(
            48_000.0,
            None,
            &[],
            Some(&in_slow),
            Some(&in_slow),
            &mut out_l,
            &mut out_r,
            false,
        );
        assert!(!r1);
        assert_eq!(out_l[0], 0.0);
        assert_eq!(channel.late_blocks(), 1);

        // Immediate second call while seq 1 is still in flight: MUST NOT clobber SHM!
        let r2 = channel.process_audio_block(
            48_000.0,
            None,
            &[],
            Some(&in_attempt_2),
            Some(&in_attempt_2),
            &mut out_l,
            &mut out_r,
            false,
        );
        assert!(!r2);
        assert_eq!(channel.late_blocks(), 2);

        // Wait until worker finishes slow seq 1, then send next block with normal timeout.
        std::thread::sleep(std::time::Duration::from_millis(80));
        channel.set_timeout_override_ms(Some(500));
        let in_ok = [0.3_f32; 64];
        let r3 = channel.process_audio_block(
            48_000.0,
            None,
            &[],
            Some(&in_ok),
            Some(&in_ok),
            &mut out_l,
            &mut out_r,
            false,
        );
        stop.store(true, Ordering::Relaxed);
        worker.join().unwrap();

        assert!(!clobbered.load(Ordering::Relaxed), "SHM must never be overwritten while a seq is in flight");
        assert!(r3);
        assert!((out_l[0] - 0.6).abs() < 1e-6);
    }
}
