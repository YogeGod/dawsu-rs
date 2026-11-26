//! DirectSound's secondary buffer is a PCM ring owned by one worker.
use std::{
    ffi::c_void,
    ptr, slice,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

use windows::{
    Win32::{
        Foundation::{CloseHandle, E_FAIL, HANDLE, WAIT_OBJECT_0},
        Media::Audio::{
            DirectSound::{
                DSBCAPS_GETCURRENTPOSITION2, DSBCAPS_GLOBALFOCUS, DSBPLAY_LOOPING, DSBUFFERDESC,
                DSSCL_NORMAL, DirectSoundCreate, DirectSoundEnumerateW, IDirectSound,
                IDirectSoundBuffer,
            },
            WAVE_FORMAT_PCM, WAVEFORMATEX,
        },
        System::Threading::{
            CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, CreateWaitableTimerExW,
            SYNCHRONIZATION_SYNCHRONIZE, SetWaitableTimer, TIMER_MODIFY_STATE, WaitForSingleObject,
        },
        UI::WindowsAndMessaging::GetDesktopWindow,
    },
    core::{BOOL, Error, GUID, PCWSTR, Result as WinResult},
};

use super::{NATIVE_SAMPLE_RATE, NativeStream, OutputDevice, validate_buffer_ms};

const CHANNELS: usize = 2;
const FRAME_BYTES: u32 = (CHANNELS * size_of::<i16>()) as u32;
const RING_FRAMES: u32 = NATIVE_SAMPLE_RATE;
const RECOVERY_TIMEOUT: Duration = Duration::from_secs(1);
const WRITE_GUARD_FRAMES: u32 = NATIVE_SAMPLE_RATE * 2 / 1000;
const DEFAULT_BUFFER_MS: u32 = 40;

struct Device {
    guid: GUID,
    info: OutputDevice,
}

unsafe extern "system" fn enumerate(
    guid: *mut GUID,
    description: PCWSTR,
    _module: PCWSTR,
    context: *mut c_void,
) -> BOOL {
    if !guid.is_null() && !context.is_null() {
        // DirectSound calls us synchronously with the vector and device descriptors still live.
        let (devices, guid) = unsafe { (&mut *context.cast::<Vec<Device>>(), *guid) };
        let name = if description.is_null() {
            "DirectSound output".into()
        } else {
            String::from_utf16_lossy(unsafe { description.as_wide() })
        };
        devices.push(Device {
            guid,
            info: OutputDevice {
                id: format!("{:032x}", guid.to_u128()),
                name,
            },
        });
    }
    true.into()
}

fn enumerate_devices() -> Result<Vec<Device>, String> {
    let mut devices = Vec::new();
    unsafe { DirectSoundEnumerateW(Some(enumerate), Some(ptr::from_mut(&mut devices).cast())) }
        .map_err(|e| format!("Could not list DirectSound outputs: {e}"))?;
    Ok(devices)
}

pub(super) fn devices() -> Result<Vec<OutputDevice>, String> {
    Ok(enumerate_devices()?
        .into_iter()
        .map(|device| device.info)
        .collect())
}

pub(super) fn open(
    device: Option<&str>,
    buffer_ms: Option<u32>,
    render: impl FnMut(&mut [i16]) + Send + 'static,
    errors: Arc<AtomicU64>,
) -> Result<NativeStream, String> {
    let buffer_ms = buffer_ms.unwrap_or(DEFAULT_BUFFER_MS);
    validate_buffer_ms(crate::OutputBackend::DirectSound, buffer_ms)?;
    let (guid, name) = match device {
        Some(id) => {
            let device = enumerate_devices()?
                .into_iter()
                .find(|device| device.info.id == id)
                .ok_or("Selected DirectSound output is unavailable")?;
            (Some(device.guid), device.info.name)
        }
        None => (None, "System default".into()),
    };
    let stop = Arc::new(AtomicBool::new(false));
    let worker_stop = Arc::clone(&stop);
    let (ready, started) = mpsc::sync_channel(1);
    let handle = thread::Builder::new()
        .name("dawsu-directsound".into())
        .spawn(move || {
            let mut scratch = vec![0i16; RING_FRAMES as usize * CHANNELS];
            let buffer = match open_buffer(guid.as_ref(), &mut scratch) {
                Ok(buffer) => buffer,
                Err(error) => {
                    let _ = ready.send(Err(format!("Could not start DirectSound: {error}")));
                    return;
                }
            };
            if ready.send(Ok(())).is_ok() {
                let _rt = crate::rt_thread::promote_current_thread();
                run(
                    &buffer.buffer,
                    &mut scratch,
                    buffer_ms,
                    render,
                    &worker_stop,
                    &errors,
                );
            }
        })
        .map_err(|error| format!("Could not start DirectSound worker: {error}"))?;
    match started.recv_timeout(Duration::from_secs(5)) {
        Ok(Ok(())) => Ok(NativeStream {
            name,
            buffer_ms,
            stop,
            thread: Some(handle),
        }),
        Ok(Err(error)) => {
            let _ = handle.join();
            Err(error)
        }
        Err(_) => {
            // A driver that has not returned from startup does not block the editor.
            stop.store(true, Ordering::Release);
            Err("The DirectSound output did not start.".into())
        }
    }
}

struct SoundBuffer {
    buffer: IDirectSoundBuffer,
    _device: IDirectSound,
}

impl Drop for SoundBuffer {
    fn drop(&mut self) {
        let _ = unsafe { self.buffer.Stop() };
    }
}

fn open_buffer(guid: Option<&GUID>, scratch: &mut [i16]) -> WinResult<SoundBuffer> {
    let mut device = None;
    // All DirectSound interfaces and the sound buffer stay on their creating worker.
    unsafe { DirectSoundCreate(guid.map(ptr::from_ref), &mut device, None)? };
    let device = device.ok_or_else(|| Error::from_hresult(E_FAIL))?;
    // A secondary buffer with global focus works without tying output to an editor HWND.
    unsafe { device.SetCooperativeLevel(GetDesktopWindow(), DSSCL_NORMAL)? };
    let mut format = WAVEFORMATEX {
        wFormatTag: WAVE_FORMAT_PCM as u16,
        nChannels: CHANNELS as u16,
        nSamplesPerSec: NATIVE_SAMPLE_RATE,
        nAvgBytesPerSec: NATIVE_SAMPLE_RATE * FRAME_BYTES,
        nBlockAlign: FRAME_BYTES as u16,
        wBitsPerSample: 16,
        cbSize: 0,
    };
    let desc = DSBUFFERDESC {
        dwSize: size_of::<DSBUFFERDESC>() as u32,
        dwFlags: DSBCAPS_GLOBALFOCUS | DSBCAPS_GETCURRENTPOSITION2,
        dwBufferBytes: RING_FRAMES * FRAME_BYTES,
        lpwfxFormat: &mut format,
        ..Default::default()
    };
    let mut buffer = None;
    unsafe { device.CreateSoundBuffer(&desc, &mut buffer, None)? };
    let buffer = buffer.ok_or_else(|| Error::from_hresult(E_FAIL))?;
    start_silent(&buffer, scratch)?;
    Ok(SoundBuffer {
        buffer,
        _device: device,
    })
}

fn start_silent(buffer: &IDirectSoundBuffer, scratch: &mut [i16]) -> WinResult<()> {
    write(buffer, 0, scratch, |samples| samples.fill(0))?;
    unsafe {
        buffer.SetCurrentPosition(0)?;
        buffer.Play(0, 0, DSBPLAY_LOOPING)
    }
}

fn run(
    buffer: &IDirectSoundBuffer,
    scratch: &mut [i16],
    buffer_ms: u32,
    mut render: impl FnMut(&mut [i16]),
    stop: &AtomicBool,
    errors: &AtomicU64,
) {
    let timer = PollTimer::new();
    let poll_ms = (buffer_ms / 4).clamp(2, 10);
    let queued = NATIVE_SAMPLE_RATE * buffer_ms / 1000;
    let mut next = None;
    let mut reset = false;
    let mut failed_since = None;
    let mut last = Instant::now();
    while !stop.load(Ordering::Acquire) {
        let delayed = last.elapsed() >= Duration::from_secs(1);
        last = Instant::now();
        if delayed {
            errors.fetch_add(1, Ordering::Relaxed);
        }
        let restoring = reset || delayed;
        let healthy = if restoring {
            next = None;
            // Restore also succeeds when the buffer has not been lost.
            let _ = unsafe { buffer.Stop() };
            unsafe { buffer.Restore() }
                .and_then(|()| start_silent(buffer, scratch))
                .is_ok()
        } else {
            pump(buffer, scratch, queued, &mut next, &mut render).unwrap_or(false)
        };
        reset = !healthy;
        if reset {
            errors.fetch_add(1, Ordering::Relaxed);
        }
        // Restoring the silent ring does not establish that the mixer can feed it again.
        if recovery_stalled(&mut failed_since, healthy && !restoring, Instant::now()) {
            break;
        }
        timer.wait(poll_ms);
    }
}

fn recovery_stalled(failed_since: &mut Option<Instant>, pumped: bool, now: Instant) -> bool {
    if pumped {
        *failed_since = None;
        return false;
    }
    now.duration_since(*failed_since.get_or_insert(now)) >= RECOVERY_TIMEOUT
}

fn pump(
    buffer: &IDirectSoundBuffer,
    scratch: &mut [i16],
    queued: u32,
    next: &mut Option<u32>,
    render: impl FnOnce(&mut [i16]),
) -> WinResult<bool> {
    let (mut play, mut writer) = (0, 0);
    unsafe { buffer.GetCurrentPosition(Some(&mut play), Some(&mut writer))? };
    let Some((frame, frames)) = refill(
        RING_FRAMES,
        play / FRAME_BYTES,
        write_frame(writer),
        *next,
        queued,
    ) else {
        return Ok(false);
    };
    if frames > 0 {
        write(
            buffer,
            frame,
            &mut scratch[..frames as usize * CHANNELS],
            render,
        )?;
        *next = Some((frame + frames) % RING_FRAMES);
    }
    Ok(true)
}

fn write_frame(writer: u32) -> u32 {
    (writer.div_ceil(FRAME_BYTES) + WRITE_GUARD_FRAMES) % RING_FRAMES
}

fn refill(
    capacity: u32,
    play: u32,
    safe: u32,
    next: Option<u32>,
    target: u32,
) -> Option<(u32, u32)> {
    let distance = |from, to| (to + capacity - from) % capacity;
    let available = capacity - distance(play, safe);
    let offset = next.unwrap_or(safe);
    let queued = distance(safe, offset);
    // The region from play up to safe belongs to the device.
    if target >= available || queued >= available {
        return None;
    }
    Some((offset, target.saturating_sub(queued)))
}

fn write(
    buffer: &IDirectSoundBuffer,
    frame: u32,
    scratch: &mut [i16],
    render: impl FnOnce(&mut [i16]),
) -> WinResult<()> {
    let (mut first, mut second) = (ptr::null_mut::<c_void>(), ptr::null_mut::<c_void>());
    let (mut first_bytes, mut second_bytes) = (0, 0);
    let bytes = size_of_val(scratch) as u32;
    // The driver may split a write at the end of the ring. Both regions are
    // valid until Unlock, and neither exceeds the size we requested.
    unsafe {
        buffer.Lock(
            frame * FRAME_BYTES,
            bytes,
            &mut first,
            &mut first_bytes,
            Some(&mut second),
            Some(&mut second_bytes),
            0,
        )?;
        let valid = first_bytes.checked_add(second_bytes) == Some(bytes)
            && !first.is_null()
            && (second_bytes == 0 || !second.is_null());
        if valid {
            render(scratch);
            let source = slice::from_raw_parts(scratch.as_ptr().cast::<u8>(), bytes as usize);
            let (a, b) = source.split_at(first_bytes as usize);
            ptr::copy_nonoverlapping(a.as_ptr(), first.cast(), a.len());
            if !b.is_empty() {
                ptr::copy_nonoverlapping(b.as_ptr(), second.cast(), b.len());
            }
        }
        buffer.Unlock(
            first,
            first_bytes,
            (!second.is_null()).then_some(second.cast_const()),
            second_bytes,
        )?;
        if !valid {
            return Err(Error::from_hresult(E_FAIL));
        }
    }
    Ok(())
}

struct PollTimer(Option<HANDLE>);

impl PollTimer {
    fn new() -> Self {
        let access = (SYNCHRONIZATION_SYNCHRONIZE | TIMER_MODIFY_STATE).0;
        // High-resolution wait avoids changing the Windows timer resolution for the process.
        Self(
            unsafe {
                CreateWaitableTimerExW(
                    None,
                    PCWSTR::null(),
                    CREATE_WAITABLE_TIMER_HIGH_RESOLUTION,
                    access,
                )
            }
            .ok(),
        )
    }

    fn wait(&self, millis: u32) {
        if let Some(timer) = self.0 {
            let due_100ns = -i64::from(millis) * 10_000;
            unsafe {
                if SetWaitableTimer(timer, &due_100ns, 0, None, None, false).is_ok()
                    && WaitForSingleObject(timer, 20) == WAIT_OBJECT_0
                {
                    return;
                }
            }
        }
        thread::sleep(Duration::from_millis(u64::from(millis)));
    }
}

impl Drop for PollTimer {
    fn drop(&mut self) {
        if let Some(timer) = self.0 {
            let _ = unsafe { CloseHandle(timer) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stalled_playback_exits_after_a_second_and_a_working_pump_restarts_the_timeout() {
        let start = Instant::now();
        let at = |ms| start + Duration::from_millis(ms);
        let mut failed_since = None;
        assert!(!recovery_stalled(&mut failed_since, false, at(0)));
        assert!(!recovery_stalled(&mut failed_since, false, at(999)));
        assert!(recovery_stalled(&mut failed_since, false, at(1000)));

        let mut failed_since = None;
        assert!(!recovery_stalled(&mut failed_since, false, at(1100)));
        assert!(!recovery_stalled(&mut failed_since, true, at(1600)));
        assert!(failed_since.is_none());
        assert!(!recovery_stalled(&mut failed_since, false, at(1800)));
        assert!(!recovery_stalled(&mut failed_since, false, at(2799)));
        assert!(recovery_stalled(&mut failed_since, false, at(2800)));
    }

    #[test]
    fn queues_only_the_requested_frames_across_the_ring_boundary() {
        assert_eq!(refill(1000, 730, 880, None, 200), Some((880, 200)));
        assert_eq!(refill(1000, 792, 942, Some(80), 200), Some((80, 62)));
        assert_eq!(refill(1000, 792, 942, Some(142), 200), Some((142, 0)));
    }

    #[test]
    fn refuses_an_underrun_or_a_queue_that_reaches_the_play_cursor() {
        assert_eq!(refill(1000, 100, 200, Some(199), 300), None);
        assert_eq!(refill(1000, 100, 200, Some(100), 300), None);
        assert_eq!(refill(1000, 20, 760, None, 500), None);
    }

    #[test]
    fn rounds_the_write_cursor_away_from_the_protected_region() {
        assert_eq!(write_frame(1), WRITE_GUARD_FRAMES + 1);
        assert_eq!(
            write_frame(RING_FRAMES * FRAME_BYTES - 1),
            WRITE_GUARD_FRAMES
        );
    }

    #[test]
    fn device_ids_distinguish_equal_names_and_omit_the_default_alias() {
        let mut devices = Vec::<Device>::new();
        let name = [u16::from(b'A'), 0];
        let mut first = GUID::from_u128(1);
        let mut second = GUID::from_u128(2);
        let context = ptr::from_mut(&mut devices).cast();
        // Feed the same in-memory descriptors DirectSound would pass to the callback.
        for guid in [
            ptr::null_mut(),
            ptr::from_mut(&mut first),
            ptr::from_mut(&mut second),
        ] {
            assert!(unsafe {
                enumerate(
                    guid,
                    PCWSTR::from_raw(name.as_ptr()),
                    PCWSTR::null(),
                    context,
                )
                .as_bool()
            });
        }
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].info.name, devices[1].info.name);
        assert_eq!(devices[0].info.id, "00000000000000000000000000000001");
        assert_ne!(devices[0].info.id, devices[1].info.id);
    }
}
