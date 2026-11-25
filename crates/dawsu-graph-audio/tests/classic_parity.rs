//! Classic drum detection against 3.1.4 (invariant 9). The fixture's provenance is in
//! `tests/fixtures/classic-3.1.4/PROVENANCE.md`. Window, twiddles, FFT, spectrograms and curves are compared bit for
//! bit (a difference there is a maths-library difference, reported, never loosened); detected hits
//! to 4 decimals.

use dawsu_foundation::digest::sha256_hex;
use dawsu_foundation::fft::{fft, twiddles};
use dawsu_graph_audio::classic::{self, Drum};
use serde_json::Value;
use std::path::PathBuf;

const RATE: u32 = 44_100;

fn fixture(name: &str) -> PathBuf {
    std::env::var_os("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")))
        .join("tests/fixtures/classic-3.1.4")
        .join(name)
}

fn json(name: &str) -> Value {
    serde_json::from_slice(&std::fs::read(fixture(name)).unwrap()).unwrap()
}

fn pattern() -> Vec<f32> {
    std::fs::read(fixture("pattern.f32")).unwrap().as_chunks::<4>().0
        .iter().copied().map(f32::from_le_bytes).collect()
}

fn f64s(name: &str) -> Vec<f64> {
    std::fs::read(fixture(name)).unwrap().as_chunks::<8>().0
        .iter().copied().map(f64::from_le_bytes).collect()
}

fn bits(value: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(value.as_str().unwrap(), 16).unwrap())
}

fn bit_list(value: &Value) -> Vec<f64> {
    value.as_array().unwrap().iter().map(bits).collect()
}

fn never() -> bool {
    false
}

fn curves() -> classic::Curves {
    classic::analyse(&pattern(), RATE, &never, &mut |_, _| {}).unwrap()
}

#[test]
fn hann_window_and_its_sum_match_bit_for_bit() {
    let fixture = json("window.json");
    let (window, sum) = classic::hann();
    assert_eq!(window.len(), classic::WINDOW);
    for (i, (ours, theirs)) in window.iter().zip(bit_list(&fixture["hann"])).enumerate() {
        assert_eq!(ours.to_bits(), theirs.to_bits(), "Hann value {i}: maths-library difference");
    }
    assert_eq!(sum.to_bits(), bits(&fixture["sum"]).to_bits());
}

#[test]
fn fft_twiddles_match_bit_for_bit() {
    for stage in json("window.json")["twiddles"].as_array().unwrap() {
        let len = stage["len"].as_u64().unwrap() as usize;
        let expected: Vec<_> = stage["sequence"].as_array().unwrap().iter()
            .map(|pair| (bits(&pair[0]), bits(&pair[1]))).collect();
        let ours: Vec<_> = twiddles(len).collect();
        assert_eq!(ours.len(), expected.len());
        for (k, (a, b)) in ours.iter().zip(&expected).enumerate() {
            assert!(a.0.to_bits() == b.0.to_bits() && a.1.to_bits() == b.1.to_bits(),
                "stage {len} twiddle {k}: maths-library difference");
        }
    }
}

#[test]
fn one_windowed_frame_and_its_fft_match_bit_for_bit() {
    let fixture = json("fft-frame50.json");
    let audio = pattern();
    let peak = audio.iter().fold(0f32, |p, v| p.max(v.abs()));
    let x: Vec<f32> = audio.iter().map(|v| v / peak).collect();
    let (hann, _) = classic::hann();
    let start = 50 * 441 - classic::WINDOW as isize / 2;
    let mut re: Vec<f64> = (0..classic::WINDOW).map(|i| {
        let j = start + i as isize;
        if j >= 0 && (j as usize) < x.len() { f64::from(x[j as usize]) * hann[i] } else { 0.0 }
    }).collect();
    assert_eq!(re.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
        bit_list(&fixture["input_re"]).iter().map(|v| v.to_bits()).collect::<Vec<_>>());
    let mut im = vec![0.0; classic::WINDOW];
    fft(&mut re, &mut im);
    for (name, ours, theirs) in [("re", &re, bit_list(&fixture["re"])), ("im", &im, bit_list(&fixture["im"]))] {
        for (k, (a, b)) in ours.iter().zip(&theirs).enumerate() {
            assert_eq!(a.to_bits(), b.to_bits(), "FFT {name}[{k}] differs");
        }
    }
}

#[test]
fn spectrograms_match_bit_for_bit() {
    let meta = json("meta.json");
    let (magnitude, percussive) = classic::spectra(&pattern(), RATE);
    let bytes = |v: &[f32]| v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<_>>();
    assert_eq!(sha256_hex(&bytes(&magnitude)), meta["magnitude_sha256"].as_str().unwrap());
    assert_eq!(sha256_hex(&bytes(&percussive)), meta["percussive_sha256"].as_str().unwrap());
}

#[test]
fn normalised_curves_match_bit_for_bit() {
    let curves = curves();
    assert_eq!(curves.hop, 441);
    for (name, ours) in [("kick", &curves.kick), ("body", &curves.body), ("noise", &curves.noise),
        ("hat", &curves.hat), ("snare", &curves.snare)] {
        let theirs = f64s(&format!("curve-{name}.f64"));
        assert_eq!(ours.len(), theirs.len(), "{name} length");
        for (t, (a, b)) in ours.iter().zip(&theirs).enumerate() {
            assert_eq!(a.to_bits(), b.to_bits(), "{name} curve frame {t}: {a} vs {b}");
        }
    }
}

#[test]
fn detected_hits_match_3_1_4_at_each_sensitivity() {
    let curves = curves();
    let fixture = json("onsets.json");
    for (scale, key) in [(2.0, "2.00"), (1.5, "1.50"), (1.25, "1.25"), (1.0, "1.00")] {
        let expected = fixture[key].as_array().unwrap();
        let ours = classic::pick(&curves, scale);
        assert_eq!(ours.len(), expected.len(), "scale {scale}");
        for (a, b) in ours.iter().zip(expected) {
            let drum = match a.drum { Drum::Kick => "kick", Drum::Snare => "snare" };
            assert_eq!(drum, b["drum"].as_str().unwrap(), "scale {scale}");
            assert_eq!(format!("{:.4}", a.time_ms), format!("{:.4}", b["time_ms"].as_f64().unwrap()), "scale {scale}");
            assert_eq!(format!("{:.4}", a.score), format!("{:.4}", b["score"].as_f64().unwrap()), "scale {scale}");
        }
    }
}

#[test]
fn results_do_not_depend_on_overall_level() {
    let quiet: Vec<f32> = pattern().iter().map(|v| v * 0.05).collect();
    let loud = classic::pick(&curves(), 1.25);
    let soft = classic::pick(&classic::analyse(&quiet, RATE, &never, &mut |_, _| {}).unwrap(), 1.25);
    let key = |o: &classic::Onset| (o.drum, o.time_ms.to_bits());
    assert_eq!(loud.iter().map(key).collect::<Vec<_>>(), soft.iter().map(key).collect::<Vec<_>>());
}

#[test]
fn silence_and_tiny_input_give_no_hits() {
    for audio in [vec![0f32; RATE as usize * 3], vec![0f32; 100]] {
        let curves = classic::analyse(&audio, RATE, &never, &mut |_, _| {}).unwrap();
        assert!(classic::pick(&curves, 1.0).is_empty());
    }
    assert!(classic::pick(&curves(), 0.0).is_empty());
    assert!(classic::pick(&curves(), f64::NAN).is_empty());
}

#[test]
fn cancelling_stops_analysis_and_returns_nothing() {
    let cancel = || true;
    assert!(classic::analyse(&pattern(), RATE, &cancel, &mut |_, _| {}).is_err());
}

#[test]
fn progress_reaches_its_total() {
    let mut last = (0, 0);
    classic::analyse(&pattern(), RATE, &never, &mut |done, total| last = (done, total)).unwrap();
    assert!(last.1 > 0);
    assert_eq!(last.0, last.1);
}
