//! Verifies that the real-time `StreamRender::fill` callback path performs zero Rust heap
//! allocations or deallocations across 1,000 audio buffers exercising transport, seek, cycle,
//! pitch-preserving 2.0x speed stretching, auditions, Listen, and live VST3 swaps.
//!
//! Boundary: Rust's `#[global_allocator]` intercepts all Rust heap allocations/deallocations.
//! C++ `signalsmith-stretch` STL buffers are pre-warmed off-thread in `SongStretcher::new` and
//! capped at `2.0x` playback speed (verified separately by source audit).

use dawsu_audio::{AudioClip, LiveAudioSource, RtBenchHarness, ScheduledSound};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

struct CountingAllocator;

static COUNT_ACTIVE: AtomicBool = AtomicBool::new(false);
static ALLOC_COUNT: AtomicUsize = AtomicUsize::new(0);
static DEALLOC_COUNT: AtomicUsize = AtomicUsize::new(0);

#[global_allocator]
static GLOBAL: CountingAllocator = CountingAllocator;

// SAFETY: Delegates all allocations directly to `System` while atomically counting allocations
// on test callbacks when `COUNT_ACTIVE` is enabled.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if COUNT_ACTIVE.load(Ordering::Relaxed) {
            ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if COUNT_ACTIVE.load(Ordering::Relaxed) {
            DEALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.dealloc(ptr, layout) };
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if COUNT_ACTIVE.load(Ordering::Relaxed) {
            ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if COUNT_ACTIVE.load(Ordering::Relaxed) {
            ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
            DEALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

struct SilentLiveSource;

impl LiveAudioSource for SilentLiveSource {
    fn process_block(
        &mut self,
        left: &mut [f32],
        right: &mut [f32],
        _sample_rate: u32,
        _is_playing: bool,
    ) {
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            *l = 0.05;
            *r = -0.05;
        }
    }
}

fn sine_clip(sample_rate: u32, frames: usize, channels: usize, freq_hz: f32) -> Arc<AudioClip> {
    let mut samples = Vec::with_capacity(frames * channels);
    for i in 0..frames {
        let s = (2.0 * std::f32::consts::PI * freq_hz * i as f32 / sample_rate as f32).sin() * 0.25;
        for _ in 0..channels {
            samples.push(s);
        }
    }
    Arc::new(AudioClip {
        samples,
        channels,
        sample_rate,
    })
}

#[test]
fn audio_callback_performs_zero_rust_allocations_over_1000_buffers() {
    let sample_rate = 48_000;
    let channels = 2;
    let song = sine_clip(sample_rate, sample_rate as usize * 8, channels, 440.0);
    let cue_clip = sine_clip(sample_rate, 1024, 1, 880.0);
    let listen_clip = sine_clip(sample_rate, 2048, 2, 220.0);
    let cues: Vec<ScheduledSound> = (0..64)
        .map(|i| ScheduledSound {
            at_ms: i as f64 * 100.0,
            end_ms: None,
            clip: cue_clip.clone(),
            gain: 0.7,
        })
        .collect();

    // Off-thread setup: build schedule + live source outside the RT callback measurement.
    let mut harness = RtBenchHarness::new(sample_rate, channels);
    let prebuilt_load = harness.prepare_load(Some(song.clone()), cues.clone(), 2.0);
    let mut live_a: Option<Box<dyn LiveAudioSource>> = Some(Box::new(SilentLiveSource));
    let mut out = vec![0.0_f32; 512 * channels];

    // Push initial state and live source before measuring, then push further pre-built
    // RT-safe commands mid-run without allocating inside the measured section.
    harness.push_prepared_load(prebuilt_load);
    harness.set_live_source(live_a.take());
    harness.play(true);
    harness.cycle(Some((100.0, 2500.0)));
    harness.keep_pitch(true);

    // Pre-allocate second Load slot & schedule before entering RT measurement.
    let prebuilt_reload = harness.prepare_load(Some(song), cues, 1.5);
    let mut reload_opt = Some(prebuilt_reload);
    let audition_clip = Some(cue_clip);
    let listen_opt = Some(listen_clip);

    // Single warm-up buffer to ensure OS page commit before strict measurement.
    harness.fill(&mut out);

    ALLOC_COUNT.store(0, Ordering::SeqCst);
    DEALLOC_COUNT.store(0, Ordering::SeqCst);
    COUNT_ACTIVE.store(true, Ordering::SeqCst);

    for i in 0..1000 {
        match i {
            100 => harness.seek(500.0),
            200 => {
                if let Some(clip) = audition_clip.clone() {
                    harness.audition(clip, 0.8);
                }
            }
            300 => {
                if let Some(clip) = listen_opt.clone() {
                    harness.listen(Some(clip));
                }
            }
            400 => harness.keep_pitch(false),
            500 => {
                if let Some(load) = reload_opt.take() {
                    harness.push_prepared_load(load);
                }
            }
            600 => harness.set_live_source(None),
            750 => harness.keep_pitch(true),
            900 => harness.play(false),
            _ => {}
        }
        harness.fill(&mut out);
    }

    COUNT_ACTIVE.store(false, Ordering::SeqCst);
    let allocs = ALLOC_COUNT.load(Ordering::SeqCst);
    let deallocs = DEALLOC_COUNT.load(Ordering::SeqCst);

    // Non-RT retirement happens cleanly outside the RT callback window.
    harness.drain_retired();

    assert_eq!(allocs, 0, "expected 0 Rust heap allocations inside RT callback, observed {allocs}");
    assert_eq!(deallocs, 0, "expected 0 Rust heap deallocations inside RT callback, observed {deallocs}");
}
