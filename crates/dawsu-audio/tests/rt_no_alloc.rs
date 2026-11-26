//! Verifies that the real-time `StreamRender::fill` callback path performs zero Rust heap
//! allocations or deallocations across 1,000 audio buffers exercising transport, seek, cycle,
//! pitch-preserving 2.0x speed stretching, auditions, Listen, and live VST3 swaps.
//!
//! Boundary: Rust's `#[global_allocator]` intercepts all Rust heap allocations/deallocations.
//! C++ `signalsmith-stretch` STL buffers are pre-warmed off-thread in `SongStretcher::new` and
//! capped at `2.0x` playback speed (verified separately by source audit).

use dawsu_audio::{
    AudioClip, AudioEffectProcessor, FxInsertSpec, FxStripSpec, LiveAudioSource, MixerFxGraph,
    RtBenchHarness, STRIP_DRY, STRIP_HITSOUNDS, STRIP_LANE_BASE, STRIP_MASTER, STRIP_SONG,
    ScheduledSound,
};
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
            strip_id: 0,
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
            150 => harness.set_spectrum_tap_enabled(true),
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
            700 => harness.set_spectrum_tap_enabled(false),
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

struct RtFxDelayPlugin {
    latency: u32,
    latency_dirty: Arc<AtomicBool>,
}

impl AudioEffectProcessor for RtFxDelayPlugin {
    fn process_block(
        &mut self,
        left: &mut [f32],
        right: &mut [f32],
        _sample_rate: u32,
        _is_playing: bool,
        _position_ms: f64,
        _speed: f64,
        bypassed: bool,
    ) {
        if !bypassed {
            for (l, r) in left.iter_mut().zip(right.iter_mut()) {
                *l *= 0.9;
                *r *= 0.9;
            }
        }
    }

    fn latency_samples(&self) -> u32 {
        self.latency
    }

    fn take_latency_changed(&mut self) -> bool {
        if self.latency_dirty.swap(false, Ordering::AcqRel) {
            self.latency = if self.latency == 512 { 256 } else { 512 };
            true
        } else {
            false
        }
    }
}

#[test]
fn audio_callback_with_fx_graph_and_dynamic_pdc_performs_zero_rust_allocations() {
    let sample_rate = 48_000;
    let channels = 2;
    let song = sine_clip(sample_rate, sample_rate as usize * 4, channels, 440.0);
    let cue_clip = sine_clip(sample_rate, 1024, 1, 880.0);
    let cues: Vec<ScheduledSound> = (0..32)
        .map(|i| ScheduledSound {
            at_ms: i as f64 * 100.0,
            end_ms: None,
            clip: cue_clip.clone(),
            gain: 0.7,
            strip_id: if i % 2 == 0 { STRIP_LANE_BASE } else { STRIP_DRY },
        })
        .collect();

    let latency_dirty = Arc::new(AtomicBool::new(false));
    let graph = MixerFxGraph::new(
        sample_rate,
        vec![
            FxStripSpec {
                id: STRIP_DRY,
                inserts: vec![],
                outputs: vec![(STRIP_HITSOUNDS, 1.0)],
            },
            FxStripSpec {
                id: STRIP_LANE_BASE,
                inserts: vec![FxInsertSpec::new(Box::new(RtFxDelayPlugin {
                    latency: 512,
                    latency_dirty: Arc::clone(&latency_dirty),
                }))],
                outputs: vec![(STRIP_HITSOUNDS, 1.0)],
            },
            FxStripSpec {
                id: STRIP_HITSOUNDS,
                inserts: vec![],
                outputs: vec![(STRIP_MASTER, 1.0)],
            },
            FxStripSpec {
                id: STRIP_SONG,
                inserts: vec![],
                outputs: vec![(STRIP_MASTER, 1.0)],
            },
            FxStripSpec {
                id: STRIP_MASTER,
                inserts: vec![],
                outputs: vec![],
            },
        ],
    )
    .unwrap();

    let mut harness = RtBenchHarness::new(sample_rate, channels);
    let load = harness.prepare_load(Some(song), cues, 1.0);
    harness.push_prepared_load(load);
    harness.set_fx_graph(Some(Box::new(graph)));
    harness.play(true);
    let mut out = vec![0.0_f32; 512 * channels];
    harness.fill(&mut out);

    ALLOC_COUNT.store(0, Ordering::SeqCst);
    DEALLOC_COUNT.store(0, Ordering::SeqCst);
    COUNT_ACTIVE.store(true, Ordering::SeqCst);

    for i in 0..500 {
        if i == 250 {
            latency_dirty.store(true, Ordering::Release);
        }
        harness.fill(&mut out);
    }

    COUNT_ACTIVE.store(false, Ordering::SeqCst);
    let allocs = ALLOC_COUNT.load(Ordering::SeqCst);
    let deallocs = DEALLOC_COUNT.load(Ordering::SeqCst);
    harness.drain_retired();

    assert_eq!(harness.pdc_latency_samples(), 256);
    assert_eq!(allocs, 0, "expected 0 allocs with MixerFxGraph + kLatencyChanged, got {allocs}");
    assert_eq!(deallocs, 0, "expected 0 deallocs with MixerFxGraph + kLatencyChanged, got {deallocs}");
}
