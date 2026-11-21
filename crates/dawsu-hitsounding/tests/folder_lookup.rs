//! Opening a map whose folder holds many files (a storyboard, a big custom sample set) stays
//! quick: each hit sample is found in the listing without lowering every listed name again.

use dawsu_hitsounding::{decompile, Decompiled, LaneSource};
use dawsu_osu_format::OsuFile;
use std::time::{Duration, Instant};

/// A difficulty with `objects` circles, each with a whistle and a custom index from 1 to 20, on
/// the soft set.
fn map(objects: usize) -> OsuFile {
    let mut text = String::from(
        "osu file format v14\n\n[General]\nAudioFilename: audio.mp3\nMode: 0\n\n[Difficulty]\nSliderMultiplier:1.4\nSliderTickRate:1\n\n[TimingPoints]\n0,500,4,2,1,60,1,0\n\n[HitObjects]\n",
    );
    for i in 0..objects {
        text += &format!("256,192,{},1,2,2:0:{}:0:\n", 1000 + i * 100, i % 20 + 1);
    }
    OsuFile::read(text.into_bytes()).file
}

/// The soft samples for indexes 1 to 10 only, in mixed case, after `padding` storyboard files:
/// indexes 11 to 20 find no file and fall back to the skin, so their lookups read every name.
fn listing(padding: usize) -> Vec<String> {
    let mut files: Vec<String> = (1..=10)
        .flat_map(|i| [format!("Soft-HitNormal{i}.WAV"), format!("soft-hitwhistle{i}.wav")])
        .chain((0..padding).map(|i| format!("Storyboard/frame{i:05}.png")))
        .chain(["audio.mp3".to_string()])
        .collect();
    files.sort();
    files
}

#[test]
fn a_folder_of_many_files_opens_quickly_and_finds_the_same_samples() {
    let file = map(2000);
    let files = listing(2000);
    let start = Instant::now();
    let hits = match decompile(&file, &files, &|| false, &mut |_, _| {}) {
        Decompiled::Done(hits) => hits,
        other => panic!("{other:?}"),
    };
    let took = start.elapsed();
    // Index 11 finds no file and plays the skin's sample.
    let eleventh = hits.moments().iter().find(|m| m.time().ms() == 2000.0).expect("the circle at 2000 ms");
    assert!(eleventh.hits().all(|lane| matches!(hits.lane(lane).map(|l| &l.source), Some(LaneSource::Skin))), "{eleventh:?}");
    // The listed names, in their own case, play.
    let sources: Vec<_> = hits.lanes().iter().filter_map(|lane| match &lane.source {
        LaneSource::File(name) => Some(name.as_str()),
        LaneSource::Skin => None,
    }).collect();
    assert!(sources.contains(&"Soft-HitNormal3.WAV"), "{sources:?}");
    assert!(sources.contains(&"soft-hitwhistle10.wav"), "{sources:?}");
    assert!(!sources.iter().any(|name| name.contains("11")), "{sources:?}");
    // Before the index, this took 5 s or more in a debug build; now it takes milliseconds.
    assert!(took < Duration::from_secs(2), "decompile took {took:?}");
}
