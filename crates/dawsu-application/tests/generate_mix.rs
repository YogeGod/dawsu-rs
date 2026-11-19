use dawsu_application::{Document, output};
use dawsu_foundation::SampleIdentity;
use dawsu_test_support::Scratch;
use std::fs;

const MAP: &[u8] = b"osu file format v14\n[General]\nMode:0\n[Metadata]\nArtist:Artist\nTitle:Title\nCreator:Mapper\nVersion:Hard\n[Difficulty]\nSliderMultiplier:1.4\nSliderTickRate:1\n[TimingPoints]\n0,500,4,1,0,100,1,0\n[HitObjects]\n256,192,1000,1,8,0:0:0:0:\n";

fn wave(amplitude: i16) -> Vec<u8> {
    let mut bytes = b"RIFF\x26\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data\x02\0\0\0".to_vec();
    bytes.extend(amplitude.to_le_bytes());
    bytes
}

#[test]
fn generate_stages_a_mix_without_writing_until_review_is_accepted() {
    let scratch = Scratch::new("generate-mixed-review");
    let path = scratch.path("source.osu");
    fs::write(&path, MAP).unwrap();
    let first = scratch.path("first.wav");
    let second = scratch.path("second.wav");
    fs::write(&first, wave(8192)).unwrap();
    fs::write(&second, wave(16384)).unwrap();
    let mut source = Document::read(&path).unwrap();
    source
        .composer
        .add(1900.1, SampleIdentity::File("first.wav".into()), 80, 0.0);
    source
        .composer
        .add(1900.2, SampleIdentity::File("second.wav".into()), 40, 0.0);

    let plan = output::generate(&output::Source::from_document(&source), &|| false)
        .unwrap_or_else(|error| panic!("Generate should prepare a mixed review: {error}"));
    let [target] = plan.files.as_slice() else {
        panic!("one generated difficulty expected")
    };
    let objects = target.file.hit_objects();
    assert_eq!(objects.len(), 2);
    assert_eq!(objects[0].hitsound(), 8);
    let merged = objects
        .iter()
        .find(|object| object.time() == 1900.0)
        .unwrap();
    let sample = merged.hit_sample().unwrap();
    let mix_name = sample
        .filename
        .as_deref()
        .expect("mixed circle needs a sample file");
    assert!(mix_name.ends_with(".wav"));
    assert_eq!(
        std::path::Path::new(mix_name)
            .file_name()
            .and_then(|name| name.to_str()),
        Some(mix_name)
    );
    let mix_path = target.path.with_file_name(mix_name);
    assert_eq!(plan.mix_paths().collect::<Vec<_>>(), [mix_path.as_path()]);
    assert!(!target.path.exists());
    assert!(!mix_path.exists());
    let repeat = output::generate(&output::Source::from_document(&source), &|| false).unwrap();
    assert_eq!(repeat.mix_paths().collect::<Vec<_>>(), [mix_path.as_path()]);
    assert_eq!(repeat.files[0].file.to_bytes(), target.file.to_bytes());
    assert!(!target.path.exists());
    assert!(!mix_path.exists());

    assert!(plan.apply(&scratch.path("backups"), &|| true).is_err());
    assert!(!target.path.exists());
    assert!(!mix_path.exists());

    // Once the WAV is on disk, cancellation before writing the difficulty must roll it back.
    assert!(
        plan.apply(&scratch.path("backups"), &|| mix_path.exists())
            .is_err()
    );
    assert!(!target.path.exists());
    assert!(!mix_path.exists());

    fs::write(&mix_path, b"existing sample").unwrap();
    assert!(plan.apply(&scratch.path("backups"), &|| false).is_err());
    assert_eq!(fs::read(&mix_path).unwrap(), b"existing sample");
    assert!(!target.path.exists());
    fs::remove_file(&mix_path).unwrap();

    // Case and format variants reserve the same sample stem on every platform.
    let other_format = mix_path.with_extension("OGG");
    fs::write(&other_format, b"unrelated sample").unwrap();
    assert!(plan.apply(&scratch.path("backups"), &|| false).is_err());
    let alternative = output::generate(&output::Source::from_document(&source), &|| false).unwrap();
    let new_path = alternative.mix_paths().next().unwrap();
    assert_ne!(new_path, mix_path);
    let again = output::generate(&output::Source::from_document(&source), &|| false).unwrap();
    assert_eq!(again.mix_paths().next(), Some(new_path));

    // An absent sample already named by a sibling map is reserved too.
    let sibling = scratch.path("sibling.osu");
    let sibling_map = std::str::from_utf8(MAP).unwrap()
        .replace("Version:Hard", "Version:Sibling")
        .replace("0:0:0:0:", &format!("0:0:0:0:{}", new_path.file_name().unwrap().to_string_lossy()));
    fs::write(&sibling, &sibling_map).unwrap();
    assert!(alternative.apply(&scratch.path("backups"), &|| false).is_err());
    let without_conflict = output::generate(&output::Source::from_document(&source), &|| false).unwrap();
    assert_ne!(without_conflict.mix_paths().next(), Some(new_path));
    assert!(!new_path.exists());
    assert_eq!(fs::read(&sibling).unwrap(), sibling_map.as_bytes());
    assert_eq!(fs::read(&other_format).unwrap(), b"unrelated sample");
    fs::remove_file(&sibling).unwrap();
    fs::remove_file(&other_format).unwrap();

    plan.apply(&scratch.path("backups"), &|| false).unwrap();
    let written = Document::read(&target.path).unwrap();
    assert_eq!(
        written.file.value("Metadata", "Version").as_deref(),
        Some("Hitsounds")
    );
    assert_eq!(fs::read(&target.path).unwrap(), target.file.to_bytes());
    let written_mix = written
        .composer
        .hitsounds()
        .moments()
        .iter()
        .find(|moment| moment.time().ms() == 1900.0)
        .unwrap();
    assert!(written_mix.has_hit(&SampleIdentity::File(mix_name.into())));
    assert_eq!(written_mix.volume(), 100);
    let rendered = dawsu_audio::decode(&mix_path).unwrap();
    assert_eq!(rendered.samples.len(), 2);
    assert!(
        rendered
            .samples
            .iter()
            .all(|value| (value - 0.4).abs() < 1.0 / 32768.0)
    );
    assert_eq!(fs::read(path).unwrap(), MAP);
    assert_eq!(fs::read(first).unwrap(), wave(8192));
    assert_eq!(fs::read(second).unwrap(), wave(16384));
}
