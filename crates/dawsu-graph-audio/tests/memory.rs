//! Peak memory of one analysis of a 5-minute 44.1 kHz song. Its own test binary, so the counting
//! allocator sees only this test.

use dawsu_graph_audio::classic;
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

struct Counting;
static CURRENT: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let now = CURRENT.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
        PEAK.fetch_max(now, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        CURRENT.fetch_sub(layout.size(), Ordering::Relaxed);
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

#[test]
fn a_five_minute_song_needs_about_two_spectrograms_beside_its_audio() {
    let rate = 44_100;
    let mono: Vec<f32> = (0..rate * 300).map(|i| {
        let t = (i % (rate / 2)) as f32 / rate as f32;
        (t * 900.0).sin() * (-t * 30.0).exp()
    }).collect();
    let before = CURRENT.load(Ordering::Relaxed);
    PEAK.store(before, Ordering::Relaxed);
    let curves = classic::analyse(&mono, rate as u32, &|| false, &mut |_, _| {}).unwrap();
    let extra = PEAK.load(Ordering::Relaxed) - before;
    let spectrogram = curves.kick.len() * 513 * 4;
    println!("peak beside the input: {:.1} MB (one spectrogram {:.1} MB)", extra as f64 / 1e6, spectrogram as f64 / 1e6);
    // The normalised copy of the audio plus the magnitude spectrogram, or both spectrograms.
    assert!(extra <= (mono.len() * 4).max(spectrogram) + spectrogram + 8_000_000, "peak {extra} bytes");
}
