//! The hitsound rules' invariants (TDD hitsound rules, "Invariants"). Invariant 2 is tested in
//! `rules.rs` (played and saved volumes), 5(a) and 5(b) there too, 7(a) in `oracle.rs`; invariant 3
//! is a save's, tested with compile.

use dawsu_hitsounding::rules::{answer, Answer, ObjectAnswer, Plays, Sample};
use dawsu_osu_format::OsuFile;
use dawsu_test_support::save_pack_files;

fn map(timing: &str, objects: &str) -> OsuFile {
    let text = format!(
        "osu file format v14\r\n\r\n[General]\r\nSampleSet: Soft\r\n\r\n[Difficulty]\r\nSliderMultiplier:1.4\r\nSliderTickRate:1\r\n\r\n[TimingPoints]\r\n{timing}\r\n\r\n[HitObjects]\r\n{objects}\r\n"
    );
    OsuFile::read(text.into_bytes()).file
}

fn objects(file: &OsuFile, folder: &[String]) -> Vec<ObjectAnswer> {
    match answer(file, folder) {
        Answer::Objects(o) => o,
        other => panic!("expected objects, got {other:?}"),
    }
}

fn every_sample(o: &ObjectAnswer) -> Vec<&Sample> {
    let mut all: Vec<&Sample> = o.samples.iter().collect();
    if let Some(s) = &o.slider {
        all.extend(s.parts.iter().flat_map(|p| p.samples.iter()));
        all.extend(s.tail_samples.iter());
        all.extend(s.sliding_samples.iter());
    }
    all
}

#[test]
fn invariant_4_sample_set_0_plays_normal_and_a_missing_field_plays_the_map_default() {
    let zero = map("0,500,4,0,0,100,1,0", "256,192,100,1,0,0:0:0:0:");
    assert_eq!(objects(&zero, &[])[0].samples[0].lazer.bank, "normal");
    let missing = map("0,500,4", "256,192,100,1,0,0:0:0:0:");
    assert_eq!(objects(&missing, &[])[0].samples[0].lazer.bank, "soft", "the map's [General] SampleSet");
}

#[test]
fn invariant_5c_a_resolved_beatmap_file_is_always_one_in_the_folder() {
    let folder: Vec<String> = ["normal", "soft", "drum"]
        .iter()
        .flat_map(|set| ["hitnormal", "hitwhistle", "hitfinish", "hitclap", "sliderslide", "sliderwhistle", "slidertick"].map(move |s| format!("{set}-{s}.wav")))
        .chain(["hitnormal.ogg".to_string(), "soft-hitclap2.mp3".to_string()])
        .collect();
    let mut found = 0;
    for (name, path) in save_pack_files() {
        let file = OsuFile::read(std::fs::read(&path).unwrap()).file;
        let Answer::Objects(objs) = answer(&file, &folder) else { continue };
        for o in &objs {
            for s in every_sample(o) {
                if let Plays::BeatmapFile(f) = &s.plays {
                    assert!(folder.contains(f), "{name}: {f} is not in the folder");
                    found += 1;
                }
            }
        }
    }
    assert!(found > 0, "some save-pack sample must resolve to a folder file");
}

#[test]
fn invariant_8_the_rules_hold_no_state_so_threads_share_them() {
    let bytes = dawsu_test_support::save_pack_bytes("owner/catalyst-impetus-std.osu");
    let file = OsuFile::read(bytes).file;
    let here = answer(&file, &[]);
    let there = std::thread::scope(|s| s.spawn(|| answer(&file, &[])).join().unwrap());
    assert_eq!(here, there);
    assert_eq!(answer(&file, &[]), here, "a second call gives the same answer");
}

#[test]
fn invariant_9a_rules_never_fail_on_a_value_a_map_can_hold() {
    let timing = "0,NaN,4,2,0,100,1,0\r\n-5,0,4,99,-7,-40,1,0\r\n10,-0.0001,4,2,0,500,0,0\r\n20,1e300,4,2,0,100,1,0";
    let objects_text = [
        "256,192,100,1,255,9:9:-4:-30:",
        "256,192,200,2,0,L|256:192,0,0",
        "256,192,300,2,0,P|1:1|2:2,1,-50",
        "256,192,400,2,0,B,1,100",
        "256,192,500,2,0,C|300:300,10,100,1|2|3,0:0|9:9|-1:-1,0:0:0:0:",
        "999999,-999999,600,6,0,P|256:192|256:192,1,1e9",
        "256,192,700,12,0,650,0:0:0:0:",
        "256,192,800,128,0,900:0:0:0:0:",
    ]
    .join("\r\n");
    let file = map(timing, &objects_text);
    let _ = answer(&file, &["x.wav".into()]);
}

#[test]
fn invariant_9b_an_out_of_range_sample_set_gives_lazer_s_answer() {
    // An object's own out-of-range set plays Normal (ConvertHitObjectParser.cs:204-210).
    let file = map("0,500,4,2,0,100,1,0", "256,192,100,1,8,7:9:0:0:");
    for s in &objects(&file, &[])[0].samples {
        assert_eq!(s.lazer.bank, "normal", "{s:?}");
    }
    // A timing line's out-of-range set is kept as its number, which an object with set 0 takes:
    // lazer then looks up "7-hitnormal", then "hitnormal" (LegacyBeatmapDecoder.cs:502-528,
    // LegacySkin.cs:610-634). The code follows lazer (ADR 0008, invariant 7a), as
    // invariant 9(b) says.
    let file = map("0,500,4,7,0,100,1,0", "256,192,100,1,0,0:0:0:0:");
    let s = &objects(&file, &[])[0].samples[0];
    assert_eq!(s.lazer.bank, "7");
    assert_eq!(s.lookup_names.first().map(String::as_str), Some("Gameplay/7-hitnormal"));
}

#[test]
fn invariant_9c_a_negative_index_counts_as_0() {
    let folder = vec!["soft-hitclap.wav".to_string(), "soft-hitnormal.wav".to_string()];
    let file = map("0,500,4,2,-2,100,1,0", "256,192,100,1,8,0:0:-1:0:");
    for s in &objects(&file, &folder)[0].samples {
        assert!(matches!(s.plays, Plays::Skin { .. }), "{s:?}");
        assert_eq!(s.lazer.suffix, None);
        assert!(!s.lazer.use_beatmap_samples);
    }
}

#[test]
fn invariant_9d_volumes_are_clamped_to_the_played_range() {
    let loud = map("0,500,4,2,0,150,1,0", "256,192,100,1,0,0:0:0:0:");
    assert_eq!(objects(&loud, &[])[0].samples[0].played_volume, 100);
    let quiet = map("0,500,4,2,0,100,1,0", "256,192,100,1,0,0:0:0:3:");
    let s = &objects(&quiet, &[])[0].samples[0];
    assert_eq!((s.lazer.volume, s.played_volume), (3, 5), "lazer keeps 3; it plays at 5");
}
