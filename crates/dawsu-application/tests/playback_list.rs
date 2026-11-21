//! The list of every sound playback would play, worked out without an audio device. The robot
//! mapper's silent sound check (tools/compare-314.mjs) compares it with what osu!lazer plays.

use dawsu_application::{Application, PlannedSource};
use dawsu_test_support::Scratch;
use std::path::Path;
use std::time::{Duration, Instant};

/// A short silent 16-bit mono WAV, enough for a sample the editor decodes.
fn write_wav(path: &Path) {
    let frames = 441u32;
    let data = frames * 2;
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&44_100u32.to_le_bytes());
    out.extend_from_slice(&88_200u32.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data.to_le_bytes());
    out.resize(out.len() + data as usize, 0);
    std::fs::write(path, out).unwrap();
}

#[test]
fn playback_list_names_each_sound_its_file_or_skin_sound_and_volume() {
    let scratch = Scratch::new("playback-list");
    write_wav(&scratch.path("song.wav"));
    write_wav(&scratch.path("clap-custom.wav"));
    let osu = scratch.path("Map.osu");
    std::fs::write(&osu, "\
osu file format v14
[General]
AudioFilename: song.wav
SampleSet: Soft
Mode: 0

[Metadata]
Title: PlaybackList
Version: Hard

[TimingPoints]
0,500,4,2,0,45,1,0

[HitObjects]
100,100,0,1,2,0:0:0:0:
200,100,500,1,0,0:0:0:0:clap-custom.wav
300,100,1000,1,0,0:0:0:0:missing.wav
").unwrap();
    let mut app = Application::headless_with_projects(scratch.path("data"));
    assert!(app.playback_list().is_empty(), "no map open, nothing plays");
    app.request_open(osu);
    let deadline = Instant::now() + Duration::from_secs(15);
    while app.busy || app.document().is_none() {
        app.poll();
        assert!(Instant::now() < deadline, "the map did not open");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(app.error.is_none(), "{:?}", app.error);

    let list = app.playback_list();
    let at = |ms: f64| list.iter().filter(|s| s.at_ms == ms).map(|s| (s.plays.clone(), s.volume)).collect::<Vec<_>>();
    let skin = |name: &str| PlannedSource::Skin(name.into());
    // A whistled circle on a soft line at 45%: the hitnormal and the whistle, from the skin.
    let mut first = at(0.0);
    first.sort_by_key(|(plays, _)| format!("{plays:?}"));
    assert_eq!(first, vec![(skin("soft-hitnormal"), 45), (skin("soft-hitwhistle"), 45)]);
    // The circle's own file, which the folder has, and loaded.
    assert_eq!(at(500.0), vec![(PlannedSource::File("clap-custom.wav".into()), 45)]);
    assert!(list.iter().find(|s| s.at_ms == 500.0).unwrap().loaded, "clap-custom.wav was not loaded");
    // A file the folder lacks: lazer goes on to the normal hitnormal, which the skin answers.
    assert_eq!(at(1000.0), vec![(skin("normal-hitnormal"), 45)]);
    assert!(list.iter().all(|s| s.end_ms.is_none()), "circles play no looping sounds");
}
