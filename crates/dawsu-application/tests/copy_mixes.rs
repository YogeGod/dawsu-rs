use dawsu_application::{Document, output};
use dawsu_foundation::SampleIdentity;
use dawsu_hitsounding::copy::{CopyMode, CopyOptions};
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
    let opts = CopyOptions { mode: CopyMode::OverwriteOnlyDefined, ..CopyOptions::default() };
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
