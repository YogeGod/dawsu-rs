//! Windows WASAPI `IAudioClient3` low-latency event-driven shared-mode output.
//!
//! Falls back silently to `cpal` at the caller if the active endpoint or driver does not expose
//! a 32-bit float mix format or low-latency shared stream initialization.

use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc,
};
use std::time::Duration;
use windows::Win32::{
    Devices::FunctionDiscovery::PKEY_Device_FriendlyName,
    Foundation::{CloseHandle, HANDLE, RPC_E_CHANGED_MODE, S_FALSE, S_OK, WAIT_OBJECT_0, WAIT_TIMEOUT},
    Media::{
        Audio::{
            AUDCLNT_E_DEVICE_INVALIDATED, AUDCLNT_STREAMFLAGS_EVENTCALLBACK, DEVICE_STATE_ACTIVE,
            IAudioClient3, IAudioRenderClient, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator,
            WAVEFORMATEX, WAVEFORMATEXTENSIBLE, eConsole, eRender,
        },
        KernelStreaming::WAVE_FORMAT_EXTENSIBLE,
        Multimedia::{KSDATAFORMAT_SUBTYPE_IEEE_FLOAT, WAVE_FORMAT_IEEE_FLOAT},
    },
    System::{
        Com::{
            CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
            CoUninitialize, STGM_READ, StructuredStorage::PropVariantClear,
        },
        Threading::{CreateEventW, WaitForSingleObject},
        Variant::VT_LPWSTR,
    },
};
use windows::core::{HSTRING, PWSTR};

use crate::{AudioTelemetry, NativeStream, StreamRender};

pub(super) struct WasapiLowLatencyOpen<T> {
    pub stream: NativeStream,
    pub sample_rate: u32,
    pub device_id: Option<String>,
    pub payload: T,
}

struct ComApartment(bool);

impl ComApartment {
    fn init() -> Result<Self, String> {
        // SAFETY: Initializing MTA COM apartment for this dedicated worker thread.
        let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        if hr == S_OK || hr == S_FALSE {
            Ok(Self(true))
        } else if hr == RPC_E_CHANGED_MODE {
            Ok(Self(false))
        } else {
            Err(format!("CoInitializeEx failed: 0x{:08x}", hr.0 as u32))
        }
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        if self.0 {
            // SAFETY: Paired with successful `CoInitializeEx` on this same thread, after all COM
            // interfaces created in `open_on_worker` have dropped.
            unsafe { CoUninitialize() };
        }
    }
}

struct EventHandle(HANDLE);

impl EventHandle {
    fn create() -> Result<Self, String> {
        // SAFETY: Auto-reset unnamed Win32 event with default security.
        unsafe { CreateEventW(None, false, false, None) }
            .map(Self)
            .map_err(|e| format!("WASAPI CreateEventW failed: {e}"))
    }
}

impl Drop for EventHandle {
    fn drop(&mut self) {
        // SAFETY: Owned handle released after `IAudioClient3` stops/drops on worker exit.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

struct MixFormatPtr(*mut WAVEFORMATEX);

impl Drop for MixFormatPtr {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: Pointer allocated by `IAudioClient3::GetMixFormat`.
            unsafe { CoTaskMemFree(Some(self.0.cast())) };
        }
    }
}

fn endpoint_id_string(device: &IMMDevice) -> Result<String, String> {
    // SAFETY: `IMMDevice::GetId` returns a COM task-allocated UTF-16 string.
    let wide = unsafe { device.GetId() }.map_err(|e| format!("IMMDevice::GetId failed: {e}"))?;
    struct PwstrGuard(PWSTR);
    impl Drop for PwstrGuard {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: Allocated by `IMMDevice::GetId`.
                unsafe { CoTaskMemFree(Some(self.0.0.cast())) };
            }
        }
    }
    let guard = PwstrGuard(wide);
    // SAFETY: Valid COM-allocated NUL-terminated UTF-16 string while `guard` is alive.
    unsafe { guard.0.to_string() }.map_err(|e| format!("Invalid WASAPI endpoint ID: {e}"))
}

fn friendly_name(device: &IMMDevice) -> Option<String> {
    // SAFETY: Opening read-only device property store for `PKEY_Device_FriendlyName`.
    let store = unsafe { device.OpenPropertyStore(STGM_READ) }.ok()?;
    let mut prop = unsafe { store.GetValue(&PKEY_Device_FriendlyName) }.ok()?;
    struct PropGuard<'a>(&'a mut windows::Win32::System::Com::StructuredStorage::PROPVARIANT);
    impl Drop for PropGuard<'_> {
        fn drop(&mut self) {
            // SAFETY: Clears the `PROPVARIANT` returned by `IPropertyStore::GetValue`.
            let _ = unsafe { PropVariantClear(self.0) };
        }
    }
    let guard = PropGuard(&mut prop);
    // SAFETY: Checked `vt == VT_LPWSTR` and non-null `pwszVal` before reading UTF-16 string.
    unsafe {
        if guard.0.Anonymous.Anonymous.vt == VT_LPWSTR {
            let ptr = guard.0.Anonymous.Anonymous.Anonymous.pwszVal;
            if !ptr.is_null() {
                return ptr.to_string().ok().filter(|s| !s.is_empty());
            }
        }
    }
    None
}

fn resolve_device(
    enumerator: &IMMDeviceEnumerator,
    device_id: Option<&str>,
    device_name: Option<&str>,
) -> Result<IMMDevice, String> {
    if let Some(raw_id) = device_id {
        let want_id = match raw_id.parse::<cpal::DeviceId>() {
            Ok(parsed) => {
                if parsed.host() != cpal::HostId::Wasapi {
                    return Err("Selected output device is not a WASAPI endpoint".into());
                }
                parsed.id().to_owned()
            }
            Err(_) => raw_id.strip_prefix("wasapi:").unwrap_or(raw_id).to_owned(),
        };
        let wide_id = HSTRING::from(want_id.as_str());
        if let Ok(dev) = unsafe { enumerator.GetDevice(&wide_id) }
            && unsafe { dev.GetState() }.ok() == Some(DEVICE_STATE_ACTIVE)
        {
            return Ok(dev);
        }
        let collection = unsafe { enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) }
            .map_err(|e| format!("EnumAudioEndpoints failed: {e}"))?;
        let count = unsafe { collection.GetCount() }.unwrap_or(0);
        for index in 0..count {
            if let Ok(dev) = unsafe { collection.Item(index) } {
                let id_matches = endpoint_id_string(&dev)
                    .is_ok_and(|ep| ep.eq_ignore_ascii_case(&want_id));
                let name_matches = friendly_name(&dev)
                    .is_some_and(|name| name.eq_ignore_ascii_case(&want_id));
                if id_matches || name_matches {
                    return Ok(dev);
                }
            }
        }
        return Err("Selected WASAPI output device is unavailable".into());
    }

    if let Some(want_name) = device_name {
        let collection = unsafe { enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) }
            .map_err(|e| format!("EnumAudioEndpoints failed: {e}"))?;
        let count = unsafe { collection.GetCount() }.unwrap_or(0);
        for index in 0..count {
            if let Ok(dev) = unsafe { collection.Item(index) }
                && friendly_name(&dev).is_some_and(|name| name == want_name)
            {
                return Ok(dev);
            }
        }
        return Err("Selected WASAPI output device is unavailable".into());
    }

    unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eConsole) }
        .map_err(|e| format!("GetDefaultAudioEndpoint failed: {e}"))
}

fn align_period_frames(target: u32, fund: u32, min_p: u32, max_p: u32) -> u32 {
    let fund = fund.max(1);
    let min_p = min_p.max(1);
    let max_p = max_p.max(min_p);
    let aligned = target.max(min_p).div_ceil(fund).saturating_mul(fund);
    aligned.clamp(min_p, max_p)
}

struct WorkerReady<T> {
    name: String,
    buffer_ms: u32,
    sample_rate: u32,
    device_id: String,
    payload: T,
}

pub(super) fn open<T: Send + 'static>(
    device_name: Option<&str>,
    device_id: Option<&str>,
    buffer_ms: Option<u32>,
    errors: Arc<AtomicU64>,
    telemetry: Arc<AudioTelemetry>,
    build_render: impl FnOnce(u32, usize) -> (StreamRender, T) + Send + 'static,
) -> Result<WasapiLowLatencyOpen<T>, String> {
    let stop = Arc::new(AtomicBool::new(false));
    let worker_stop = Arc::clone(&stop);
    let owned_name = device_name.map(str::to_owned);
    let owned_id = device_id.map(str::to_owned);
    let (ready_tx, ready_rx) = mpsc::sync_channel(1);

    let thread = std::thread::Builder::new()
        .name("dawsu-wasapi-rt".into())
        .spawn(move || {
            let _com = match ComApartment::init() {
                Ok(com) => com,
                Err(err) => {
                    let _ = ready_tx.send(Err(err));
                    return;
                }
            };
            run_on_worker(
                owned_name.as_deref(),
                owned_id.as_deref(),
                buffer_ms,
                errors,
                telemetry,
                &worker_stop,
                build_render,
                ready_tx,
            );
        })
        .map_err(|e| format!("Starting WASAPI RT worker failed: {e}"))?;

    match ready_rx.recv_timeout(Duration::from_secs(5)) {
        Ok(Ok(ready)) => Ok(WasapiLowLatencyOpen {
            stream: NativeStream {
                name: ready.name,
                buffer_ms: ready.buffer_ms,
                stop,
                thread: Some(thread),
            },
            sample_rate: ready.sample_rate,
            device_id: Some(ready.device_id),
            payload: ready.payload,
        }),
        Ok(Err(err)) => {
            stop.store(true, Ordering::Release);
            let _ = thread.join();
            Err(err)
        }
        Err(_) => {
            stop.store(true, Ordering::Release);
            Err("Timed out initializing WASAPI low-latency stream".into())
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn run_on_worker<T: Send + 'static>(
    device_name: Option<&str>,
    device_id: Option<&str>,
    buffer_ms: Option<u32>,
    errors: Arc<AtomicU64>,
    telemetry: Arc<AudioTelemetry>,
    stop: &AtomicBool,
    build_render: impl FnOnce(u32, usize) -> (StreamRender, T),
    ready_tx: mpsc::SyncSender<Result<WorkerReady<T>, String>>,
) {
    // SAFETY: MTA COM apartment was initialized cleanly by `ComApartment::init()` in caller scope.
    let enumerator: IMMDeviceEnumerator =
        match unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) } {
            Ok(e) => e,
            Err(e) => {
                let _ = ready_tx.send(Err(format!("CoCreateInstance(MMDeviceEnumerator) failed: {e}")));
                return;
            }
        };

    let device = match resolve_device(&enumerator, device_id, device_name) {
        Ok(d) => d,
        Err(e) => {
            let _ = ready_tx.send(Err(e));
            return;
        }
    };

    let ep_id = match endpoint_id_string(&device) {
        Ok(id) => id,
        Err(e) => {
            let _ = ready_tx.send(Err(e));
            return;
        }
    };
    let canonical_id = cpal::DeviceId::new(cpal::HostId::Wasapi, &ep_id).to_string();
    let name = friendly_name(&device).unwrap_or_else(|| "Audio output".into());

    // SAFETY: Activating `IAudioClient3` on a verified active render endpoint.
    let audio_client: IAudioClient3 = match unsafe { device.Activate(CLSCTX_ALL, None) } {
        Ok(c) => c,
        Err(e) => {
            let _ = ready_tx.send(Err(format!("Activate(IAudioClient3) failed: {e}")));
            return;
        }
    };

    let fmt_ptr = match unsafe { audio_client.GetMixFormat() } {
        Ok(ptr) if !ptr.is_null() => ptr,
        Ok(_) => {
            let _ = ready_tx.send(Err("WASAPI GetMixFormat returned null".into()));
            return;
        }
        Err(e) => {
            let _ = ready_tx.send(Err(format!("WASAPI GetMixFormat failed: {e}")));
            return;
        }
    };
    let _fmt_guard = MixFormatPtr(fmt_ptr);

    // SAFETY: `fmt_ptr` is non-null and valid until `_fmt_guard` drops; `WAVEFORMATEX` is packed(1),
    // so read unaligned before inspecting fields.
    let fmt = unsafe { fmt_ptr.read_unaligned() };
    let is_float32 = if fmt.wFormatTag == WAVE_FORMAT_IEEE_FLOAT as u16 {
        fmt.wBitsPerSample == 32
    } else if fmt.wFormatTag == WAVE_FORMAT_EXTENSIBLE as u16 && fmt.cbSize >= 22 {
        // SAFETY: `cbSize >= 22` guarantees a full `WAVEFORMATEXTENSIBLE` layout.
        let subformat = unsafe {
            let ext_ptr = fmt_ptr.cast::<WAVEFORMATEXTENSIBLE>();
            (&raw const (*ext_ptr).SubFormat).read_unaligned()
        };
        fmt.wBitsPerSample == 32 && subformat == KSDATAFORMAT_SUBTYPE_IEEE_FLOAT
    } else {
        false
    };
    let sample_rate = fmt.nSamplesPerSec;
    let channels = fmt.nChannels as usize;
    if !is_float32 || sample_rate == 0 || channels == 0 {
        let _ = ready_tx.send(Err("WASAPI endpoint mix format is not 32-bit float PCM".into()));
        return;
    }

    let (mut default_p, mut fund_p, mut min_p, mut max_p) = (0u32, 0u32, 0u32, 0u32);
    if let Err(e) = unsafe {
        audio_client.GetSharedModeEnginePeriod(fmt_ptr, &mut default_p, &mut fund_p, &mut min_p, &mut max_p)
    } {
        let _ = ready_tx.send(Err(format!("GetSharedModeEnginePeriod failed: {e}")));
        return;
    }

    let period_frames = match buffer_ms {
        Some(ms) => {
            let req = ((u64::from(ms) * u64::from(sample_rate) + 500) / 1000).max(1) as u32;
            if req > max_p.max(min_p).max(1) {
                let _ = ready_tx.send(Err(format!(
                    "Requested {ms} ms ({req} frames) exceeds shared WaveRT max period ({max_p} frames)"
                )));
                return;
            }
            align_period_frames(req, fund_p, min_p, max_p)
        }
        None => align_period_frames(min_p, fund_p, min_p, max_p),
    };

    if let Err(e) = unsafe {
        audio_client.InitializeSharedAudioStream(
            AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
            period_frames,
            fmt_ptr,
            None,
        )
    } {
        let _ = ready_tx.send(Err(format!("InitializeSharedAudioStream failed: {e}")));
        return;
    }

    let event = match EventHandle::create() {
        Ok(ev) => ev,
        Err(e) => {
            let _ = ready_tx.send(Err(e));
            return;
        }
    };
    if let Err(e) = unsafe { audio_client.SetEventHandle(event.0) } {
        let _ = ready_tx.send(Err(format!("SetEventHandle failed: {e}")));
        return;
    }
    let buffer_frames = match unsafe { audio_client.GetBufferSize() } {
        Ok(size) if size > 0 => size,
        Ok(_) => {
            let _ = ready_tx.send(Err("WASAPI GetBufferSize returned 0".into()));
            return;
        }
        Err(e) => {
            let _ = ready_tx.send(Err(format!("GetBufferSize failed: {e}")));
            return;
        }
    };
    let render_client: IAudioRenderClient = match unsafe { audio_client.GetService() } {
        Ok(r) => r,
        Err(e) => {
            let _ = ready_tx.send(Err(format!("GetService(IAudioRenderClient) failed: {e}")));
            return;
        }
    };

    // Prime the shared endpoint buffer with initial silence before calling Start().
    if let Ok(ptr) = unsafe { render_client.GetBuffer(buffer_frames) } {
        if !ptr.is_null() {
            // SAFETY: `GetBuffer(buffer_frames)` grants write access to `buffer_frames * channels` floats.
            unsafe {
                std::ptr::write_bytes(ptr.cast::<f32>(), 0, buffer_frames as usize * channels);
            }
        }
        if let Err(e) = unsafe { render_client.ReleaseBuffer(buffer_frames, 0) } {
            let _ = ready_tx.send(Err(format!("Initial silent ReleaseBuffer failed: {e}")));
            return;
        }
    }

    let (mut render, payload) = build_render(sample_rate, channels);
    let _rt = crate::rt_thread::promote_current_thread();

    if let Err(e) = unsafe { audio_client.Start() } {
        let _ = ready_tx.send(Err(format!("IAudioClient3::Start failed: {e}")));
        return;
    }

    let effective_ms =
        ((u64::from(period_frames) * 1000 + u64::from(sample_rate) / 2) / u64::from(sample_rate))
            .max(1) as u32;
    telemetry
        .observed_buffer_frames
        .store(period_frames, Ordering::Release);

    if ready_tx
        .send(Ok(WorkerReady {
            name,
            buffer_ms: effective_ms,
            sample_rate,
            device_id: canonical_id,
            payload,
        }))
        .is_err()
    {
        let _ = unsafe { audio_client.Stop() };
        return;
    }

    while !stop.load(Ordering::Acquire) {
        // SAFETY: `event.0` is an auto-reset Win32 handle owned by this thread; 200 ms timeout bounds stop latency.
        let waited = unsafe { WaitForSingleObject(event.0, 200) };
        if stop.load(Ordering::Acquire) {
            break;
        }
        if waited != WAIT_OBJECT_0 && waited != WAIT_TIMEOUT {
            errors.fetch_add(1, Ordering::Relaxed);
            break;
        }

        let padding = match unsafe { audio_client.GetCurrentPadding() } {
            Ok(p) => p,
            Err(err) => {
                if err.code() == AUDCLNT_E_DEVICE_INVALIDATED || !stop.load(Ordering::Acquire) {
                    errors.fetch_add(1, Ordering::Relaxed);
                }
                break;
            }
        };
        let avail = buffer_frames.saturating_sub(padding);
        if avail == 0 {
            continue;
        }
        match unsafe { render_client.GetBuffer(avail) } {
            Ok(ptr) if !ptr.is_null() => {
                let sample_count = avail as usize * channels;
                // SAFETY: `GetBuffer(avail)` returns exclusive writable memory for `sample_count`
                // 32-bit float samples until `ReleaseBuffer(avail, 0)`.
                let slice =
                    unsafe { std::slice::from_raw_parts_mut(ptr.cast::<f32>(), sample_count) };
                render.fill(slice);
                if let Err(err) = unsafe { render_client.ReleaseBuffer(avail, 0) } {
                    if err.code() == AUDCLNT_E_DEVICE_INVALIDATED || !stop.load(Ordering::Acquire) {
                        errors.fetch_add(1, Ordering::Relaxed);
                    }
                    break;
                }
            }
            _ => {
                errors.fetch_add(1, Ordering::Relaxed);
                break;
            }
        }
    }

    let _ = unsafe { audio_client.Stop() };
    telemetry.playing.store(false, Ordering::Release);
}
