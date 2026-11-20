//! Classic kick and snare detection, a port of 3.1.4's `DrumOnsets` (plain signal processing, no
//! trained model): STFT, harmonic/percussive separation by median filtering, band-limited log
//! spectral flux, adaptive peak picking and band-ratio rules. Kick uses 30–120 Hz. A snare needs its
//! body (150–400 Hz) and wire noise (1.5–5 kHz) to rise together, and is dropped when a coinciding
//! kick explains it.
//!
//! [`analyse`] does the expensive part once; [`pick`] re-picks at another sensitivity without a new
//! STFT. The arithmetic is 3.1.4's step for step, so its results are reproduced (invariant 9).
//! Work is split over threads by frame or bin; each value is computed by one thread, so the result
//! does not depend on the thread count.

use dawsu_foundation::fft::fft;
use std::f64::consts::PI;
use std::sync::atomic::{AtomicUsize, Ordering};

pub const WINDOW: usize = 2048;
const MEDIAN_HALF: usize = 8; // 17-point median filters
const COMPRESSION: f64 = 3.0; // ln(1 + γ·|X|)
const PEAK_HALF: usize = 3; // ±30 ms local maximum
const MEAN_BEFORE: usize = 10;
const MEAN_AFTER: usize = 3;
const DELTA: f64 = 0.12;
const FLOOR: f64 = 0.3;
const MIN_GAP_MS: f64 = 60.0;
const NORM_PERCENTILE: f64 = 0.9;
/// Centred frames see an attack a little before it lands; measured on annotated recordings.
const BIAS_MS: f64 = 4.0;
const SNARE_UNDER_KICK: f64 = 0.75;
const STRONG_SNARE: f64 = 1.0;
const BODY_OVER_KICK: f64 = 1.3;
const HAT_OVER_KICK: f64 = 0.5;

/// The run was cancelled; nothing was produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cancelled;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Drum {
    Kick,
    Snare,
}

/// A detected hit. `time_ms` is from the start of the analysed audio. `score` is the peak height of
/// the normalised curve (about 1 for a typical strong hit), rounded to 4 decimals: a relative
/// strength, not a probability. `frame` identifies the detection across re-picks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Onset {
    pub drum: Drum,
    pub frame: usize,
    pub time_ms: f64,
    pub score: f64,
}

/// The normalised onset curves of one analysis, one value per hop.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Curves {
    pub hop: usize,
    pub hop_ms: f64,
    pub kick: Vec<f64>,
    pub body: Vec<f64>,
    pub noise: Vec<f64>,
    pub hat: Vec<f64>,
    pub snare: Vec<f64>,
}

/// .NET's `Math.Round(value, 4)` (half to even after scaling).
fn round4(value: f64) -> f64 {
    (value * 10_000.0).round_ties_even() / 10_000.0
}

struct Layout {
    hop: usize,
    hop_ms: f64,
    bins: usize,
    frames: usize,
    rate: u32,
}

impl Layout {
    fn new(samples: usize, rate: u32) -> Self {
        let hop = (rate as usize / 100).max(1);
        let mut layout = Layout { hop, hop_ms: hop as f64 * 1000.0 / f64::from(rate), bins: 0, frames: samples / hop + 1, rate };
        layout.bins = layout.bin(11_000.0) + 2;
        layout
    }
    fn bin(&self, hz: f64) -> usize {
        ((hz * WINDOW as f64 / f64::from(self.rate)).round_ties_even() as i64).clamp(1, WINDOW as i64 / 2) as usize
    }
}

/// The Hann window and its sum, as 3.1.4 computes them.
pub fn hann() -> (Vec<f64>, f64) {
    let mut sum = 0.0;
    let window = (0..WINDOW).map(|i| {
        let value = 0.5 - 0.5 * (2.0 * PI * i as f64 / WINDOW as f64).cos();
        sum += value;
        value
    }).collect();
    (window, sum)
}

/// Fills `out`, `row` values per index, on several threads: each thread writes its own run of
/// rows in place, so nothing is copied afterwards.
fn fill_rows<S>(
    out: &mut [f32],
    row: usize,
    cancel: &(dyn Fn() -> bool + Sync),
    progress: &mut dyn FnMut(usize),
    init: impl Fn() -> S + Sync,
    work: impl Fn(usize, &mut S, &mut [f32]) + Sync,
) -> Result<(), Cancelled> {
    let count = out.len() / row.max(1);
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).clamp(1, count.max(1));
    let chunk = count.div_ceil(threads).max(1);
    let done = AtomicUsize::new(0);
    let finished: Vec<bool> = std::thread::scope(|scope| {
        let handles: Vec<_> = out.chunks_mut(chunk * row).enumerate().map(|(part, rows)| {
            let (init, work, done) = (&init, &work, &done);
            scope.spawn(move || {
                let mut state = init();
                for (offset, values) in rows.chunks_mut(row).enumerate() {
                    if cancel() {
                        return false;
                    }
                    work(part * chunk + offset, &mut state, values);
                    done.fetch_add(1, Ordering::Relaxed);
                }
                true
            })
        }).collect();
        while !handles.iter().all(|h| h.is_finished()) {
            progress(done.load(Ordering::Relaxed));
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        progress(done.load(Ordering::Relaxed));
        handles.into_iter().map(|h| h.join().expect("analysis thread panicked")).collect()
    });
    if finished.contains(&false) || cancel() {
        return Err(Cancelled);
    }
    Ok(())
}

fn spectrogram(x: &[f32], layout: &Layout, cancel: &(dyn Fn() -> bool + Sync), progress: &mut dyn FnMut(usize))
    -> Result<Vec<f32>, Cancelled> {
    let (window, sum) = hann();
    let bins = layout.bins;
    let mut mag = vec![0f32; layout.frames * bins];
    fill_rows(&mut mag, bins, cancel, progress,
        || (vec![0.0; WINDOW], vec![0.0; WINDOW]),
        |t, (re, im): &mut (Vec<f64>, Vec<f64>), out| {
            let start = (t * layout.hop) as isize - WINDOW as isize / 2;
            for i in 0..WINDOW {
                let j = start + i as isize;
                re[i] = if j >= 0 && (j as usize) < x.len() { f64::from(x[j as usize]) * window[i] } else { 0.0 };
                im[i] = 0.0;
            }
            fft(re, im);
            for (k, value) in out.iter_mut().enumerate() {
                *value = ((re[k] * re[k] + im[k] * im[k]).sqrt() * 2.0 / sum) as f32;
            }
        })?;
    Ok(mag)
}

fn median(values: &mut [f32]) -> f32 {
    values.sort_unstable_by(f32::total_cmp);
    values.get(values.len() / 2).copied().unwrap_or(0.0)
}

/// Soft-masked percussive part: a median across time keeps sustained (harmonic) partials, a median
/// across frequency keeps broadband (percussive) frames. Both medians are taken frame by frame, so
/// no harmonic spectrogram is kept.
fn percussive(mag: &[f32], layout: &Layout, cancel: &(dyn Fn() -> bool + Sync), progress: &mut dyn FnMut(usize))
    -> Result<Vec<f32>, Cancelled> {
    let (frames, bins) = (layout.frames, layout.bins);
    let window = |centre: usize, len: usize| centre.saturating_sub(MEDIAN_HALF)..(centre + MEDIAN_HALF + 1).min(len);
    let mut perc = vec![0f32; mag.len()];
    fill_rows(&mut perc, bins, cancel, progress, || Vec::with_capacity(2 * MEDIAN_HALF + 1), |t, buf: &mut Vec<f32>, out| {
        for (k, value) in out.iter_mut().enumerate() {
            buf.clear();
            buf.extend(window(t, frames).map(|tt| mag[tt * bins + k]));
            let h = f64::from(median(buf));
            buf.clear();
            buf.extend(window(k, bins).map(|kk| mag[t * bins + kk]));
            let p = f64::from(median(buf));
            let (p2, h2) = (p * p, h * h);
            *value = (f64::from(mag[t * bins + k]) * if p2 + h2 > 1e-20 { p2 / (p2 + h2) } else { 0.0 }) as f32;
        }
    })?;
    Ok(perc)
}

/// Positive log-magnitude increase per band, against a 3-bin maximum of the previous frame so that
/// small pitch wobbles do not count as onsets. One pass for all bands, keeping two frames of
/// `ln(1 + 3·|X|)` at a time.
fn fluxes(perc: &[f32], layout: &Layout, bands: &[(usize, usize)]) -> Vec<Vec<f64>> {
    let (frames, bins) = (layout.frames, layout.bins);
    let bands: Vec<(usize, usize)> = bands.iter().map(|&(lo, hi)| (lo.max(1), hi.min(bins - 2))).collect();
    let mut out = vec![vec![0.0; frames]; bands.len()];
    let log_row = |t: usize, row: &mut Vec<f64>| {
        row.clear();
        row.extend(perc[t * bins..(t + 1) * bins].iter().map(|&v| (1.0 + COMPRESSION * f64::from(v)).ln()));
    };
    let (mut previous, mut current) = (Vec::with_capacity(bins), Vec::with_capacity(bins));
    log_row(0, &mut previous);
    for t in 1..frames {
        log_row(t, &mut current);
        for (flux, &(lo, hi)) in out.iter_mut().zip(&bands) {
            if hi < lo {
                continue;
            }
            let mut s = 0.0;
            for k in lo..=hi {
                let prev = previous[k].max(previous[k - 1].max(previous[k + 1]));
                s += (current[k] - prev).max(0.0);
            }
            flux[t] = s / (hi - lo + 1) as f64;
        }
        std::mem::swap(&mut previous, &mut current);
    }
    out
}

/// Scales a curve so that a typical strong hit (the 90th percentile of its local maxima) is 1.
fn normalise(o: Vec<f64>) -> Vec<f64> {
    // Most local maxima are tiny wiggles; only those within 25 dB of the loudest count.
    let mut maxima: Vec<f64> = (1..o.len().saturating_sub(1))
        .filter(|&t| o[t] > 0.0 && o[t] >= o[t - 1] && o[t] > o[t + 1])
        .map(|t| o[t]).collect();
    if maxima.is_empty() {
        return o;
    }
    let top = maxima.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    maxima.retain(|&v| v >= top * 0.056);
    maxima.sort_by(f64::total_cmp);
    let index = ((maxima.len() - 1) as f64).min((maxima.len() as f64 * NORM_PERCENTILE).floor()) as usize;
    let scale = maxima[index];
    if scale <= 1e-12 {
        return o;
    }
    o.into_iter().map(|v| v / scale).collect()
}

/// The two spectrograms (magnitude and percussive part) of the peak-normalised audio, for parity
/// checks. Empty when the audio is too short or silent.
#[doc(hidden)]
pub fn spectra(mono: &[f32], rate: u32) -> (Vec<f32>, Vec<f32>) {
    let Some((x, layout)) = prepare(mono, rate) else { return (Vec::new(), Vec::new()) };
    let never = || false;
    let mag = spectrogram(&x, &layout, &never, &mut |_| {}).expect("not cancelled");
    let perc = percussive(&mag, &layout, &never, &mut |_| {}).expect("not cancelled");
    (mag, perc)
}

fn prepare(mono: &[f32], rate: u32) -> Option<(Vec<f32>, Layout)> {
    if rate == 0 {
        return None;
    }
    let layout = Layout::new(mono.len(), rate);
    if mono.len() < layout.hop * 4 {
        return None;
    }
    // Peak-normalise so the log compression behaves the same for quiet and loud songs.
    let peak = mono.iter().fold(0f32, |p, v| p.max(v.abs()));
    if peak < 1e-6 {
        return None;
    }
    Some((mono.iter().map(|v| v / peak).collect(), layout))
}

/// Analyses mono audio at its own sample rate. Audio too short or silent gives empty curves.
/// `progress` gets (done, total) work units from the calling thread.
pub fn analyse(
    mono: &[f32],
    rate: u32,
    cancel: &(dyn Fn() -> bool + Sync),
    progress: &mut dyn FnMut(usize, usize),
) -> Result<Curves, Cancelled> {
    let Some((x, layout)) = prepare(mono, rate) else {
        return if cancel() { Err(Cancelled) } else { Ok(Curves::default()) };
    };
    let total = 2 * layout.frames;
    let mag = spectrogram(&x, &layout, cancel, &mut |done| progress(done, total))?;
    drop(x);
    let perc = percussive(&mag, &layout, cancel, &mut |done| progress(layout.frames + done, total))?;
    drop(mag);
    let bands = [(30.0, 120.0), (150.0, 400.0), (1500.0, 5000.0), (7000.0, 11_000.0)]
        .map(|(lo, hi)| (layout.bin(lo), layout.bin(hi)));
    let mut curves = fluxes(&perc, &layout, &bands).into_iter().map(normalise);
    drop(perc);
    let mut next = || curves.next().expect("four bands");
    let (kick, body, noise, hat) = (next(), next(), next(), next());
    let snare = normalise(body.iter().zip(&noise).map(|(b, n)| (b * n).sqrt()).collect());
    if cancel() {
        return Err(Cancelled);
    }
    progress(total, total);
    Ok(Curves { hop: layout.hop, hop_ms: layout.hop_ms, kick, body, noise, hat, snare })
}

fn peaks(o: &[f64], scale: f64, hop_ms: f64) -> Vec<(usize, f64)> {
    let (delta, floor) = (DELTA * scale, FLOOR * scale);
    let mut picked = Vec::new();
    for t in 1..o.len().saturating_sub(1) {
        let v = o[t];
        if v < floor {
            continue;
        }
        let range = t.saturating_sub(PEAK_HALF)..=(t + PEAK_HALF).min(o.len() - 1);
        if range.clone().any(|u| u != t && (o[u] > v || (o[u] == v && u < t))) {
            continue;
        }
        let around = t.saturating_sub(MEAN_BEFORE)..=(t + MEAN_AFTER).min(o.len() - 1);
        let (mut mean, mut n) = (0.0, 0);
        for u in around {
            mean += o[u];
            n += 1;
        }
        if v < mean / f64::from(n) + delta {
            continue;
        }
        picked.push((t, v));
    }
    // Keep the stronger of two peaks closer than a plausible repeat of the same drum.
    let gap = (MIN_GAP_MS / hop_ms).ceil() as usize;
    picked.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    let mut kept: Vec<(usize, f64)> = Vec::new();
    for p in picked {
        if kept.iter().all(|k| k.0.abs_diff(p.0) >= gap) {
            kept.push(p);
        }
    }
    kept.sort_by_key(|p| p.0);
    kept
}

/// Picks kick and snare hits at a threshold scale (3.1.4's 2.0 strict to 1.0 loose). A scale that
/// is not a positive number gives none.
pub fn pick(curves: &Curves, scale: f64) -> Vec<Onset> {
    if !scale.is_finite() || scale <= 0.0 || curves.kick.is_empty() {
        return Vec::new();
    }
    let frames = curves.kick.len();
    let near = |curve: &[f64], frame: usize| {
        (frame.saturating_sub(2)..=(frame + 2).min(frames - 1)).fold(0.0, |o: f64, u| o.max(curve[u]))
    };
    // A kick's beater click leaks into the snare bands. Drop a snare candidate when it is much
    // weaker than a coinciding kick (unless strong in its own right), or when its body barely
    // exceeds the kick and it has little of the wires' high noise.
    let kick_bleed = |(frame, score): (usize, f64)| {
        let k = near(&curves.kick, frame);
        if score < STRONG_SNARE && score < SNARE_UNDER_KICK * k {
            return true;
        }
        near(&curves.body, frame) < BODY_OVER_KICK * k && near(&curves.hat, frame) < HAT_OVER_KICK * k
    };
    let onset = |drum, (frame, score): (usize, f64)| Onset {
        drum,
        frame,
        time_ms: (frame as f64 * curves.hop_ms + BIAS_MS).max(0.0),
        score: round4(score),
    };
    let mut result: Vec<Onset> = peaks(&curves.kick, scale, curves.hop_ms).into_iter().map(|p| onset(Drum::Kick, p)).collect();
    result.extend(peaks(&curves.snare, scale, curves.hop_ms).into_iter().filter(|&p| !kick_bleed(p)).map(|p| onset(Drum::Snare, p)));
    result.sort_by(|a, b| a.time_ms.total_cmp(&b.time_ms).then(a.drum.cmp(&b.drum)));
    result
}

/// The map's beat grid: the nearest line for a beat divisor and that division's length in ms
/// (infinite where the map has no red line, so nothing there counts as off the grid).
pub type GridSnap<'a> = &'a dyn Fn(f64, u32) -> (f64, f64);

/// How far a detection may sit from a grid line and still count as on it; narrowed to a third of
/// the step for fast tempos or fine divisors.
pub const GRID_TOLERANCE_MS: f64 = 20.0;
/// Largest constant offset between the audio and the map's timing that is compensated.
pub const MAX_GRID_SHIFT_MS: f64 = 50.0;

fn median_f64(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    let n = values.len();
    if n % 2 == 1 { values[n / 2] } else { (values[n / 2 - 1] + values[n / 2]) / 2.0 }
}

/// The constant lag of detections behind the grid (decoder delay, the mapper's offset): the median
/// of the deltas, then twice more over those within 25 ms of it. 0 with fewer than 8 deltas or a lag
/// beyond [`MAX_GRID_SHIFT_MS`].
pub fn grid_shift(times: impl IntoIterator<Item = f64>, snap: GridSnap, divisor: u32) -> f64 {
    let deltas: Vec<f64> = times.into_iter().filter_map(|t| {
        let (snapped, step) = snap(t, divisor);
        (step.is_finite() && step > 0.0).then_some(t - snapped)
    }).collect();
    if deltas.len() < 8 {
        return 0.0;
    }
    let mut shift = median_f64(deltas.clone());
    for _ in 0..2 {
        let near: Vec<f64> = deltas.iter().copied().filter(|x| (x - shift).abs() <= 25.0).collect();
        if near.len() < 8 {
            break;
        }
        shift = median_f64(near);
    }
    if shift.abs() <= MAX_GRID_SHIFT_MS { shift } else { 0.0 }
}

/// The nearest line of any allowed division within `min(20 ms, step / 3)` of `time - shift`, or
/// `None` when the detection is off the grid.
pub fn on_grid(time: f64, snap: GridSnap, divisors: &[u32], shift: f64) -> Option<f64> {
    let time = time - shift;
    let mut best: Option<f64> = None;
    for &divisor in divisors {
        let (snapped, step) = snap(time, divisor);
        let tolerance = if step.is_finite() && step > 0.0 { GRID_TOLERANCE_MS.min(step / 3.0) } else { f64::INFINITY };
        if (snapped - time).abs() <= tolerance && best.is_none_or(|b| (snapped - time).abs() < (b - time).abs()) {
            best = Some(snapped);
        }
    }
    best
}
