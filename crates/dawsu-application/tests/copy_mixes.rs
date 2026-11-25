use dawsu_application::{Document, output};
use dawsu_foundation::SampleIdentity;
use dawsu_hitsounding::copy::{CopyCollisionPolicy, CopyMode, CopyOptions};
use dawsu_test_support::Scratch;
use std::{fs, path::Path};

const HEADER: &str = "osu file format v14\n[General]\nMode:0\n[Metadata]\nArtist:Artist\nTitle:Title\nCreator:Mapper\nVersion:Test\n[TimingPoints]\n0,500,4,1,0,100,1,0\n[HitObjects]\n";
const SOURCE_OBJECTS: &str =
    "256,192,1000,1,0,0:0:0:40:one.wav\n300,192,1000,1,0,0:0:0:80:two.wav\n";
const TARGET_OBJECTS: &str = "100,100,1000,1,0,0:0:0:0:\n";

fn write_tone(path: &Path, value: i16) -> Vec<u8> {
    let mut wav = b"RIFF\x28\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data\x04\0\0\0".to_vec();
    for _ in 0..2 {
        wav.extend(value.to_le_bytes());
    }
    fs::write(path, &wav).unwrap();
    wav
}

#[test]
fn timed_copy_reviews_tick_and_slide_samples_and_reapplies_without_duplicates() {
    let scratch = Scratch::new("copy-timed-samples");
    let source_path = scratch.path("source.osu");
    let target = scratch.path("target.osu");
    let bytes = format!("{HEADER}256,192,1503,1,0,0:0:0:80:one.wav\n256,192,1750,1,0,0:0:0:80:one.wav\n");
    fs::write(&source_path, &bytes).unwrap();
    let slider = "256,192,1000,2,0,L|536:192,1,280,0|0,0:0|0:0,0:0:0:0:";
    let original = format!("{HEADER}{slider}\n[Events]\n// preserve this line\n");
    fs::write(&target, &original).unwrap();
    let tone = write_tone(&scratch.path("one.wav"), 8192);
    let source = output::Source::from_document(&Document::read(&source_path).unwrap());
    let options = CopyOptions {
        mode: CopyMode::OverwriteOnlyDefined, copy_to_slider_ticks: true,
        copy_to_slider_slides: true, ..CopyOptions::default()
    };
    let review = || output::copy(&source, std::slice::from_ref(&target), &options, &|| false).unwrap();
    let plan = review();
    let paths: Vec<_> = plan.mix_paths().collect();
    assert_eq!(paths.len(), 1);
    assert!(!paths[0].exists());
    assert_eq!(fs::read_to_string(&target).unwrap(), original);
    assert!(plan.details.iter().any(|detail| detail.contains("1500") && detail.contains("tick")));
    assert!(plan.details.iter().any(|detail| detail.contains("1750") && detail.contains("slide")));
    plan.apply(&scratch.path("backups"), &|| false).unwrap();
    let written = fs::read_to_string(&target).unwrap();
    assert!(written.contains(slider) && written.contains("// preserve this line"));
    let events = Document::read(&target).unwrap().file.storyboard_samples();
    assert_eq!(events.iter().map(|event| event.time).collect::<Vec<_>>(), [1500.0, 1750.0]);
    assert_eq!(events[0].path, events[1].path);
    let clip = dawsu_audio::decode(paths[0]).unwrap();
    assert!((clip.samples[0] * events[0].volume / 100.0 - 0.2).abs() < 0.0001);
    review().apply(&scratch.path("backups"), &|| false).unwrap();
    assert_eq!(fs::read_to_string(&target).unwrap(), written);
    assert_eq!(fs::read_to_string(source_path).unwrap(), bytes);
    assert_eq!(fs::read(scratch.path("one.wav")).unwrap(), tone);
}

#[test]
fn mixed_copy_reviews_without_writes_and_rolls_back_unfinished_targets() {
    let scratch = Scratch::new("mixed-copy-review");
    let mapset = scratch.path("mapset");
    fs::create_dir(&mapset).unwrap();
    let source_path = mapset.join("source.osu");
    let first = mapset.join("first.osu");
    let second = mapset.join("second.osu");
    let source_bytes = format!("{HEADER}{SOURCE_OBJECTS}");
    let original = format!("{HEADER}{TARGET_OBJECTS}");
    fs::write(&source_path, &source_bytes).unwrap();
    for path in [&first, &second] {
        fs::write(path, &original).unwrap();
    }
    let one = write_tone(&mapset.join("one.wav"), 8192);
    let two = write_tone(&mapset.join("two.wav"), 16384);

    let document = Document::read(&source_path).unwrap();
    let source = output::Source::from_document(&document);
    let plan = output::copy(
        &source,
        &[first.clone(), second.clone()],
        &Default::default(),
        &|| false,
    )
    .unwrap_or_else(|error| panic!("{error}"));
    let waves: Vec<_> = plan.mix_paths().collect();
    assert!(!waves.is_empty());
    let no_output_wavs = || waves.iter().all(|wave| !wave.exists());
    let targets_unchanged = || {
        assert_eq!(fs::read_to_string(&first).unwrap(), original);
        assert_eq!(fs::read_to_string(&second).unwrap(), original);
    };
    let backups = scratch.path("backups");
    assert!(no_output_wavs());
    targets_unchanged();
    assert!(!backups.exists());

    fs::write(&second, "External editor changed this map").unwrap();
    let error = plan.apply(&backups, &|| false).unwrap_err();
    assert!(error.contains("changed since this review"), "{error}");
    assert!(no_output_wavs());
    assert!(!backups.exists());
    assert_eq!(fs::read_to_string(&first).unwrap(), original);
    assert_eq!(
        fs::read_to_string(&second).unwrap(),
        "External editor changed this map"
    );
    fs::write(&second, &original).unwrap();

    let error = plan
        .apply(&backups, &|| {
            fs::read_to_string(&first).unwrap() != original
        })
        .unwrap_err();
    assert!(error.contains("Previous targets were restored"), "{error}");
    targets_unchanged();
    assert!(no_output_wavs());

    assert_eq!(plan.apply(&backups, &|| false).unwrap(), 2);
    for path in [&first, &second] {
        let target = Document::read(path).unwrap();
        let moment = &target.composer.hitsounds().moments()[0];
        assert_eq!(moment.volume(), 100);
        let samples: Vec<_> = moment.hits().collect();
        let [SampleIdentity::File(name)] = samples.as_slice() else {
            panic!("expected the staged filename, got {samples:?}");
        };
        let clip = dawsu_audio::decode(&mapset.join(name)).unwrap();
        assert_eq!(clip.channels, 2);
        assert_eq!(clip.sample_rate, 48_000);
        // The 40% sample at 0.25 and 80% at 0.5 are baked into the mix.
        assert_eq!(clip.samples, [0.5; 4]);
    }
    assert_eq!(fs::read_to_string(source_path).unwrap(), source_bytes);
    assert_eq!(fs::read(mapset.join("one.wav")).unwrap(), one);
    assert_eq!(fs::read(mapset.join("two.wav")).unwrap(), two);
}

#[test]
fn indexed_slider_tail_copy_checks_sibling_reservations_and_plans_unique_files() {
    let scratch = Scratch::new("indexed-copy-review");
    let mapset = scratch.path("mapset");
    fs::create_dir(&mapset).unwrap();
    let source_path = mapset.join("source.osu");
    let source_bytes = format!("{HEADER}{}", SOURCE_OBJECTS.replace("1000", "1500"));
    fs::write(&source_path, &source_bytes).unwrap();
    write_tone(&mapset.join("one.wav"), 8192);
    write_tone(&mapset.join("two.wav"), 16384);
    let original = format!("{HEADER}256,192,1000,2,0,L|396:192,1,140,0|0,1:0|1:0,1:0:0:0:\n");
    let targets = [mapset.join("first.osu"), mapset.join("second.osu")];
    for target in &targets {
        fs::write(target, &original).unwrap();
    }
    let source = output::Source::from_document(&Document::read(&source_path).unwrap());
    let default = CopyOptions { mode: CopyMode::OverwriteOnlyDefined, ..CopyOptions::default() };
    let default_plan = output::copy(&source, &targets, &default, &|| false).unwrap();
    assert_eq!(default.start_index, 100);
    assert_eq!(default_plan.mix_paths().next().unwrap().file_name().unwrap(), "normal-hitnormal100.wav");
    let opts = CopyOptions { start_index: 2, ..default };
    let plan = output::copy(&source, &targets, &opts, &|| false).unwrap();
    let paths = plan.mix_paths().collect::<Vec<_>>();
    assert_eq!(paths.iter().map(|path| path.file_name().unwrap()).collect::<Vec<_>>(),
        ["normal-hitnormal2.wav", "normal-hitnormal3.wav"]);
    assert!(paths.iter().all(|path| !path.exists()));
    let sibling = mapset.join("sibling.osu");
    let reservation = format!("{HEADER}100,100,3000,1,0,1:0:2:100:\n");
    fs::write(&sibling, &reservation).unwrap();
    let backup = scratch.path("backups");
    let error = plan.apply(&backup, &|| false).unwrap_err();
    assert!(error.contains("Sample index 2 is now used"), "{error}");
    assert!(paths.iter().all(|path| !path.exists()));
    assert!(!backup.exists());
    for target in &targets {
        assert_eq!(fs::read_to_string(target).unwrap(), original);
    }
    let plan = output::copy(&source, &targets, &opts, &|| false).unwrap();
    assert_eq!(plan.apply(&backup, &|| false).unwrap(), 2);
    for (target, index) in targets.iter().zip([3, 4]) {
        let document = Document::read(target).unwrap();
        let tail = document.composer.hitsounds().moments().iter().find(|moment| moment.time().ms() == 1500.0).unwrap();
        let [SampleIdentity::Standard { index: actual, .. }] = &tail.hits().collect::<Vec<_>>()[..] else {
            panic!("expected an indexed Tail mix");
        };
        assert_eq!(actual.get(), index);
        assert_eq!(dawsu_audio::decode(&mapset.join(format!("normal-hitnormal{index}.wav"))).unwrap().samples, [0.5; 4]);
    }
    assert_eq!(fs::read_to_string(&source_path).unwrap(), source_bytes);
    assert_eq!(fs::read_to_string(&sibling).unwrap(), reservation);
}


#[test]
fn copy_can_keep_original_hitnormals_for_silent_samples_and_piles() {
    let scratch = Scratch::new("copy-silent-normal");
    let source_path = scratch.path("source.osu");
    let target_path = scratch.path("target.osu");
    let source_header = HEADER.replace("0,500,4,1,0", "0,500,4,1,4");
    let source_map = format!(
        "{source_header}256,192,1000,1,0,1:0:0:33:blank.wav\n\
         256,192,1500,1,0,1:0:0:40:blank.wav\n\
         300,192,1500,1,0,1:0:0:80:other-blank.wav\n\
         256,192,3000,1,0,1:0:0:50:unreadable.wav\n\
         256,192,4000,1,0,1:0:0:40:blank.wav\n\
         300,192,4000,1,0,1:0:0:80:tone.wav\n"
    );
    fs::write(&source_path, &source_map).unwrap();
    let protected = [
        "256,192,1000,1,8,1:1:0:63:",
        "256,192,1500,1,8,1:1:0:72:",
        "256,192,2500,1,8,1:1:0:81:",
    ];
    let target_map = format!(
        "{HEADER}{}\n256,192,3000,1,8,1:1:0:61:\n256,192,4000,1,8,1:1:0:61:\n",
        protected.join("\n"),
    );
    fs::write(&target_path, &target_map).unwrap();
    for name in ["blank.wav", "other-blank.wav", "normal-hitnormal4.wav"] {
        write_tone(&scratch.path(name), 0);
    }
    let tone = write_tone(&scratch.path("tone.wav"), 16384);
    fs::write(scratch.path("unreadable.wav"), b"invalid audio").unwrap();
    let source = output::Source::from_document(&Document::read(&source_path).unwrap());
    let targets = [target_path.clone()];
    let default = output::copy(&source, &targets, &CopyOptions::default(), &|| false).unwrap();
    assert_eq!(default.mix_paths().count(), 2);
    let protected_options = CopyOptions { never_silent_hitnormals: true, ..CopyOptions::default() };
    let plan = output::copy(&source, &targets, &protected_options, &|| false).unwrap();
    assert!(plan.details.iter().any(|detail| detail.contains("3 hit points kept their own sound")));
    let [mix] = plan.mix_paths().collect::<Vec<_>>()[..] else {
        panic!("only the partially audible pile should still be mixed");
    };
    assert!(!mix.exists());
    assert_eq!(fs::read_to_string(&target_path).unwrap(), target_map);
    plan.apply(&scratch.path("backups"), &|| false).unwrap();
    let written = fs::read_to_string(&target_path).unwrap();
    for line in protected {
        assert!(written.lines().any(|candidate| candidate == line), "{line}");
    }
    assert!(written.contains("unreadable.wav"));
    assert_eq!(dawsu_audio::decode(mix).unwrap().samples, [13_107.0 / 32_768.0; 4]);
    assert_eq!(fs::read_to_string(&source_path).unwrap(), source_map);
    assert_eq!(fs::read(scratch.path("tone.wav")).unwrap(), tone);
}
#[test]
fn copy_review_names_the_winner_and_every_replaced_source_without_writing() {
    let scratch = Scratch::new("copy-collision-review");
    let source_path = scratch.path("Easy.osu");
    let target = scratch.path("Hard.osu");
    let source_bytes = format!("{HEADER}256,192,1998,1,4,0:0:0:60:\n256,192,1999,1,8,0:0:0:60:\n256,192,2002,1,2,0:0:0:60:\n");
    let original = format!("{HEADER}100,100,2000,1,0,0:0:0:60:\n");
    fs::write(&source_path, &source_bytes).unwrap();
    fs::write(&target, &original).unwrap();
    let source = output::Source::from_document(&Document::read(&source_path).unwrap());
    let options = CopyOptions { collision_policy: CopyCollisionPolicy::LaterWins, ..CopyOptions::default() };
    let plan = output::copy(&source, std::slice::from_ref(&target), &options, &|| false).unwrap();

    assert!(plan.details.iter().any(|detail| detail ==
        "Hard.osu · 2000 ms: source at 2002 ms won; replaced 1998 ms, 1999 ms."), "{:?}", plan.details);
    assert_eq!(fs::read_to_string(&target).unwrap(), original);
    assert_eq!(fs::read_to_string(&source_path).unwrap(), source_bytes);
    assert!(plan.mix_paths().next().is_none());
    assert!(plan.files[0].file.hit_objects()[0].hitsound() & 2 != 0, "the winning whistle reaches the target");
}

#[test]
fn copy_review_merges_nearby_source_hits_by_default() {
    let scratch = Scratch::new("copy-collision-merge-default");
    let source_path = scratch.path("Easy.osu");
    let target = scratch.path("Hard.osu");
    let source_bytes = format!("{HEADER}256,192,1999,1,0,0:0:0:80:one.wav\n256,192,2001,1,0,0:0:0:60:two.wav\n");
    let original = format!("{HEADER}100,100,2000,1,0,0:0:0:60:\n");
    fs::write(&source_path, &source_bytes).unwrap();
    fs::write(&target, &original).unwrap();
    write_tone(&scratch.path("one.wav"), 8192);
    write_tone(&scratch.path("two.wav"), 4096);
    let source = output::Source::from_document(&Document::read(&source_path).unwrap());
    let plan = output::copy(&source, std::slice::from_ref(&target), &CopyOptions::default(), &|| false).unwrap();

    assert_eq!(plan.targets[0].collisions, 0);
    assert!(plan.details.iter().any(|detail| detail == "Hard.osu: 1 hit point merged nearby source hits."), "{:?}", plan.details);
    assert!(plan.mix_paths().next().is_some(), "merged nearby hits stage a combined mix without dropping earlier hits");
    assert_eq!(fs::read_to_string(&target).unwrap(), original);
}
