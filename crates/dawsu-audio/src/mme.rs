//! Windows MME playback using a three-buffer WinMM queue.

use crate::{NATIVE_SAMPLE_RATE, NativeStream, OutputDevice};
use std::cell::UnsafeCell;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc,
};
use std::time::{Duration, Instant};
use windows::Win32::{
    Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT},
    Media::{
        Audio::{
            CALLBACK_EVENT, HWAVEOUT, WAVE_FORMAT_PCM, WAVE_MAPPER, WAVEFORMATEX, WAVEHDR,
            WAVEOUTCAPSW, WHDR_DONE, WHDR_INQUEUE, waveOutClose, waveOutGetDevCapsW,
            waveOutGetErrorTextW, waveOutGetNumDevs, waveOutMessage, waveOutOpen,
            waveOutPrepareHeader, waveOutReset, waveOutUnprepareHeader, waveOutWrite,
        },
        Multimedia::{DRV_QUERYFUNCTIONINSTANCEID, DRV_QUERYFUNCTIONINSTANCEIDSIZE},
    },
    System::Threading::{CreateEventW, WaitForSingleObject},
};
use windows::core::PSTR;

const CHANNELS: usize = 2;
const BUFFER_COUNT: usize = 3;
const HEADER_BYTES: u32 = size_of::<WAVEHDR>() as u32;
const POLL_MS: u32 = 25;

struct BufferPlan {
    block_ms: u32,
}

impl BufferPlan {
    fn new(buffer_ms: Option<u32>) -> Self {
        Self {
            block_ms: (buffer_ms.unwrap_or(30) / BUFFER_COUNT as u32).max(20),
        }
    }

    fn effective_ms(&self) -> u32 {
        self.block_ms * BUFFER_COUNT as u32
    }

    fn samples(&self) -> usize {
        (NATIVE_SAMPLE_RATE as usize * self.block_ms as usize / 1000) * CHANNELS
    }
}

fn completed_in_order(
    next: usize,
    done: [bool; BUFFER_COUNT],
) -> impl ExactSizeIterator<Item = usize> {
    let count = (0..BUFFER_COUNT)
        .take_while(|offset| done[(next + offset) % BUFFER_COUNT])
        .count();
    (0..count).map(move |offset| (next + offset) % BUFFER_COUNT)
}

fn device_entries() -> impl Iterator<Item = (u32, OutputDevice)> {
    // SAFETY: The WinMM count query has no arguments or retained pointers.
    let count = unsafe { waveOutGetNumDevs() };
    (0..count).filter_map(|index| {
        let mut capabilities = WAVEOUTCAPSW::default();
        // SAFETY: The native function writes one capabilities struct during this call.
        let status = unsafe {
            waveOutGetDevCapsW(
                index as usize,
                &mut capabilities,
                size_of::<WAVEOUTCAPSW>() as u32,
            )
        };
        if status != 0 {
            return None;
        }
        let id = endpoint_id(index)?;
        let name = capabilities.szPname;
        Some((
            index,
            OutputDevice {
                id,
                name: wide_string(&name),
            },
        ))
    })
}

fn wide_string(units: &[u16]) -> String {
    String::from_utf16_lossy(units.split(|&unit| unit == 0).next().unwrap_or_default())
}

fn endpoint_id(index: u32) -> Option<String> {
    // These two WinMM messages accept an enumeration index cast to HWAVEOUT.
    let device_id = Some(HWAVEOUT(index as usize as *mut _));
    let mut bytes = 0u32;
    // SAFETY: The first call writes a byte count. The second gets an initialized aligned buffer
    // of exactly that size. Neither API retains these caller-owned addresses.
    unsafe {
        if waveOutMessage(
            device_id,
            DRV_QUERYFUNCTIONINSTANCEIDSIZE,
            (&mut bytes as *mut u32) as usize,
            0,
        ) != 0
            || !(2..=65_536).contains(&bytes)
            || !bytes.is_multiple_of(2)
        {
            return None;
        }
        let mut name = vec![0u16; (bytes / 2) as usize];
        if waveOutMessage(
            device_id,
            DRV_QUERYFUNCTIONINSTANCEID,
            name.as_mut_ptr() as usize,
            bytes as usize,
        ) != 0
        {
            return None;
        }
        let id = wide_string(&name);
        (!id.is_empty()).then_some(id)
    }
}

pub(super) fn devices() -> Result<Vec<OutputDevice>, String> {
    Ok(device_entries().map(|(_, device)| device).collect())
}

pub(super) fn open(
    device: Option<&str>,
    buffer_ms: Option<u32>,
    mut render: impl FnMut(&mut [i16]) + Send + 'static,
    errors: Arc<AtomicU64>,
) -> Result<NativeStream, String> {
    if let Some(ms) = buffer_ms {
        crate::validate_buffer_ms(crate::OutputBackend::Mme, ms)?;
    }
    let (index, name) = match device {
        Some(id) => device_entries()
            .find(|(_, entry)| entry.id.eq_ignore_ascii_case(id))
            .map(|(index, entry)| (index, entry.name))
            .ok_or_else(|| "Selected MME output device is unavailable".to_owned())?,
        None => (WAVE_MAPPER, "System default".to_owned()),
    };
    let plan = BufferPlan::new(buffer_ms);
    let buffer_ms = plan.effective_ms();
    let stop = Arc::new(AtomicBool::new(false));
    let worker_stop = stop.clone();
    let (ready, opened) = mpsc::sync_channel(1);
    let thread = std::thread::Builder::new()
        .name("dawsu-mme".into())
        .spawn(move || {
            let mut device = match WaveOut::open(index, plan.samples()) {
                Ok(device) => device,
                Err(error) => {
                    let _ = ready.send(Err(error));
                    return;
                }
            };
            if ready.send(Ok(())).is_err() {
                return;
            }
            let _rt = crate::rt_thread::promote_current_thread();
            if device
                .run(&worker_stop, &mut render, &errors, buffer_ms)
                .is_err()
            {
                errors.fetch_add(1, Ordering::Relaxed);
            }
            if !device.close() {
                errors.fetch_add(1, Ordering::Relaxed);
            }
        })
        .map_err(|error| format!("Starting the MME output worker failed: {error}"))?;

    match opened.recv_timeout(Duration::from_secs(5)) {
        Ok(Ok(())) => Ok(NativeStream {
            name,
            buffer_ms,
            stop,
            thread: Some(thread),
        }),
        Err(mpsc::RecvTimeoutError::Timeout) => {
            // A driver call can be blocked. The worker will clean up on return when it
            // finds the ready receiver closed, so the caller must not join it here.
            stop.store(true, Ordering::Release);
            Err("Timed out opening the MME output device".into())
        }
        result => {
            let _ = thread.join();
            Err(result
                .ok()
                .and_then(Result::err)
                .unwrap_or_else(|| "The MME output worker exited before opening the device".into()))
        }
    }
}

struct CompletionEvent(HANDLE);

impl CompletionEvent {
    fn new() -> Result<Self, String> {
        // SAFETY: An unnamed event with default security has no borrowed parameters.
        unsafe { CreateEventW(None, false, false, None) }
            .map(Self)
            .map_err(|error| format!("Creating the MME completion event failed: {error}"))
    }
}

impl Drop for CompletionEvent {
    fn drop(&mut self) {
        // SAFETY: This event was created for this worker and is now detached from WinMM.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

// The generated WAVEHDR is packed. The native fields are naturally aligned on Windows when the
// enclosing header starts at an aligned address. UnsafeCell allows writes via WinMM's raw pointer.
#[repr(align(8))]
struct WaveHeader(UnsafeCell<WAVEHDR>);

struct Buffer {
    header: Box<WaveHeader>,
    samples: Box<[i16]>,
    prepared: bool,
}

impl Buffer {
    fn new(count: usize) -> Self {
        let mut samples = vec![0i16; count].into_boxed_slice();
        let header = WAVEHDR {
            lpData: PSTR(samples.as_mut_ptr().cast()),
            dwBufferLength: (count * size_of::<i16>()) as u32,
            ..WAVEHDR::default()
        };
        Self {
            header: Box::new(WaveHeader(UnsafeCell::new(header))),
            samples,
            prepared: false,
        }
    }

    fn ptr(&self) -> *mut WAVEHDR {
        self.header.0.get()
    }

    fn done(&self) -> bool {
        // SAFETY: The header remains allocated and 8-byte aligned while WinMM owns its flags.
        // WinMM signals the completion event after writing DONE and clearing INQUEUE.
        let flags = unsafe { (&raw const (*self.ptr()).dwFlags).read_volatile() };
        flags & (WHDR_DONE | WHDR_INQUEUE) == WHDR_DONE
    }
}

struct WaveOut {
    handle: HWAVEOUT,
    event: Option<CompletionEvent>,
    buffers: Vec<Buffer>,
}

impl WaveOut {
    fn open(index: u32, samples: usize) -> Result<Self, String> {
        let event = CompletionEvent::new()?;
        let format = WAVEFORMATEX {
            wFormatTag: WAVE_FORMAT_PCM as u16,
            nChannels: CHANNELS as u16,
            nSamplesPerSec: NATIVE_SAMPLE_RATE,
            nAvgBytesPerSec: NATIVE_SAMPLE_RATE * CHANNELS as u32 * size_of::<i16>() as u32,
            nBlockAlign: (CHANNELS * size_of::<i16>()) as u16,
            wBitsPerSample: 16,
            cbSize: 0,
        };
        let mut handle = HWAVEOUT::default();
        // SAFETY: The format is valid PCM. WinMM borrows it during open and may signal the event
        // until the device closes. WaveOut owns the event for that entire time.
        check("Opening MME output", unsafe {
            waveOutOpen(
                Some(&mut handle),
                index,
                &format,
                Some(event.0.0 as usize),
                None,
                CALLBACK_EVENT,
            )
        })?;
        let mut device = Self {
            handle,
            event: Some(event),
            buffers: (0..BUFFER_COUNT).map(|_| Buffer::new(samples)).collect(),
        };
        for buffer in &mut device.buffers {
            // SAFETY: Each prepared header and its backing samples stay at fixed addresses.
            check("Preparing the MME buffer", unsafe {
                waveOutPrepareHeader(handle, buffer.ptr(), HEADER_BYTES)
            })?;
            buffer.prepared = true;
        }
        for buffer in &device.buffers {
            // SAFETY: The prepared buffer contains initial silence. WinMM owns it until DONE.
            check("Priming the MME buffer", unsafe {
                waveOutWrite(handle, buffer.ptr(), HEADER_BYTES)
            })?;
        }
        Ok(device)
    }

    fn run(
        &mut self,
        stop: &AtomicBool,
        render: &mut impl FnMut(&mut [i16]),
        errors: &AtomicU64,
        effective_ms: u32,
    ) -> Result<(), String> {
        let mut next = 0;
        let mut last_completed = Instant::now();
        let max_stall = Duration::from_millis(u64::from((4 * effective_ms).max(1000)));
        let event = self.event.as_ref().expect("open MME completion event").0;
        while !stop.load(Ordering::Acquire) {
            // SAFETY: The event is owned by this worker. The timeout bounds shutdown latency.
            let waited = unsafe { WaitForSingleObject(event, POLL_MS) };
            if stop.load(Ordering::Acquire) {
                break;
            }
            if waited != WAIT_OBJECT_0 && waited != WAIT_TIMEOUT {
                return Err(format!(
                    "Waiting for MME playback failed (status {})",
                    waited.0
                ));
            }

            let done = std::array::from_fn(|i| self.buffers[i].done());
            let completed = completed_in_order(next, done);
            if completed.len() == 0 && last_completed.elapsed() >= max_stall {
                return Err("The MME device stopped returning audio buffers".into());
            }
            if completed.len() == BUFFER_COUNT {
                errors.fetch_add(1, Ordering::Relaxed);
            }
            for index in completed {
                let buffer = &mut self.buffers[index];
                // DONE confirms WinMM has returned the sample block to the producer.
                render(&mut buffer.samples);
                // SAFETY: The returned buffer still has a prepared, stable header.
                check("Submitting the MME buffer", unsafe {
                    waveOutWrite(self.handle, buffer.ptr(), HEADER_BYTES)
                })?;
                next = (index + 1) % BUFFER_COUNT;
                last_completed = Instant::now();
            }
        }
        Ok(())
    }

    fn close(&mut self) -> bool {
        if self.handle.is_invalid() {
            return true;
        }
        // SAFETY: This worker owns the open device and every header. Reset returns queued blocks
        // before unprepare. Unprepare or a successful close ends WinMM's ownership.
        unsafe {
            let _ = waveOutReset(self.handle);
            for buffer in &mut self.buffers {
                if buffer.prepared
                    && waveOutUnprepareHeader(self.handle, buffer.ptr(), HEADER_BYTES) == 0
                {
                    buffer.prepared = false;
                }
            }
            let closed = waveOutClose(self.handle) == 0;
            if !closed {
                // A driver that rejects reset and close may still own pointers or signal the event.
                // Retain the outstanding allocations for process teardown in that failure case.
                for buffer in self.buffers.drain(..).filter(|buffer| buffer.prepared) {
                    std::mem::forget(buffer);
                }
                std::mem::forget(self.event.take());
            }
            self.handle = HWAVEOUT::default();
            closed
        }
    }
}

impl Drop for WaveOut {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

fn check(operation: &str, status: u32) -> Result<(), String> {
    if status == 0 {
        return Ok(());
    }
    let mut text = [0u16; 256];
    // SAFETY: The bounded slice stays valid during the message lookup.
    if unsafe { waveOutGetErrorTextW(status, &mut text) } == 0 {
        Err(format!("{operation}: {}", wide_string(&text)))
    } else {
        Err(format!("{operation} failed with WinMM status {status}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requested_buffer_is_split_into_three_pcm16_blocks_with_a_20_ms_floor() {
        let short = BufferPlan::new(Some(10));
        assert_eq!(short.effective_ms(), 60);
        assert_eq!(short.samples(), 1920);

        let long = BufferPlan::new(Some(500));
        assert_eq!(long.effective_ms(), 498);
        assert_eq!(long.samples(), 15_936);
    }

    #[test]
    fn coalesced_notifications_refill_completed_buffers_in_submission_order() {
        let queue = completed_in_order(2, [true, false, true]);
        assert_eq!(queue.len(), 2);
        assert_eq!(queue.collect::<Vec<_>>(), [2, 0]);

        let blocked = completed_in_order(0, [false, true, false]);
        assert_eq!(blocked.len(), 0);
        assert_eq!(blocked.collect::<Vec<_>>(), []);
    }

    #[test]
    fn underrun_is_one_full_completion_batch() {
        assert_eq!(completed_in_order(1, [true; 3]).len(), 3);
    }

    #[test]
    fn a_driver_owned_buffer_is_not_ready_even_if_its_done_bit_is_set() {
        let buffer = Buffer::new(4);
        assert!(!buffer.done());
        // This buffer was never given to WinMM. Simulate the driver updating its header.
        unsafe {
            let flags = &raw mut (*buffer.ptr()).dwFlags;
            assert!(flags.is_aligned());
            flags.write_volatile(WHDR_DONE | WHDR_INQUEUE);
            assert!(!buffer.done());
            flags.write_volatile(WHDR_DONE);
            assert!(buffer.done());
        }
    }
}
