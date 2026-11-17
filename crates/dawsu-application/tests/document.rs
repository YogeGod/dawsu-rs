use dawsu_application::{Document, save_document};
use dawsu_foundation::{CustomIndex, Hitsound, SampleIdentity, SampleSet, Sound};
use dawsu_test_support::Scratch;

const MAP: &[u8] = b"osu file format v14\r\n// untouched comment\r\n[General]\r\nMode:0\r\n[Metadata]\r\nTitle:Test\r\nVersion:Hard\r\n[Difficulty]\r\nSliderMultiplier:1.4\r\nSliderTickRate:1\r\n[TimingPoints]\r\n0,500,4,1,0,100,1,0\r\n[HitObjects]\r\n256,192,1000,1,0,0:0:0:0:\r\n";
fn clap() -> SampleIdentity {
    SampleIdentity::Standard {
        set: SampleSet::Normal,
        sound: Sound::Hit(Hitsound::Clap),
        index: CustomIndex::new(0),
    }
}
#[test]
fn copied_map_edit_save_reopen_keeps_untouched_bytes_and_history() {
    let scratch = Scratch::new("application-save");
    let path = scratch.path("test.osu");
    std::fs::write(&path, MAP).unwrap();
    let mut d = Document::read(&path).unwrap();
    d.composer.add(1000.0, clap(), 80, 5.0);
    let result = save_document(&d, scratch.path("backups")).unwrap();
    assert!(result.problems.is_empty());
    let bytes = std::fs::read(&path).unwrap();
    assert!(bytes.starts_with(b"osu file format v14\r\n// untouched comment\r\n"));
    let reopened = Document::read(&path).unwrap();
    assert!(reopened.composer.hitsounds().moments()[0].has_hit(&clap()));
    assert!(d.composer.history.can_undo());
}
#[test]
fn external_edit_is_preserved_and_not_reported_as_a_successful_save() {
    let scratch = Scratch::new("application-conflict");
    let path = scratch.path("test.osu");
    std::fs::write(&path, MAP).unwrap();
    let mut d = Document::read(&path).unwrap();
    d.composer.add(1000.0, clap(), 80, 5.0);
    std::fs::write(&path, b"external editor wrote this").unwrap();
    assert!(save_document(&d, scratch.path("backups")).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"external editor wrote this");
    assert!(d.composer.dirty());
}
#[test]
fn undo_after_save_can_be_saved_again_without_losing_the_undo() {
    let scratch = Scratch::new("application-save-undo-save");
    let path = scratch.path("test.osu");
    std::fs::write(&path, MAP).unwrap();
    let mut d = Document::read(&path).unwrap();
    d.composer.add(1000.0, clap(), 80, 5.0);
    let first = save_document(&d, scratch.path("backups")).unwrap();
    d.file = first.file;
    d.composer.mark_saved();
    d.composer.undo();
    save_document(&d, scratch.path("backups")).unwrap();
    let reopened = Document::read(&path).unwrap();
    assert!(!reopened.composer.hitsounds().moments()[0].has_hit(&clap()));
    assert_eq!(std::fs::read(&path).unwrap(), MAP);
}

fn sample(path: &std::path::Path, data: &[i16]) {
    let size = (data.len() * 2) as u32;
    let mut wav = b"RIFF".to_vec();
    wav.extend((36 + size).to_le_bytes());
    wav.extend(b"WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data");
    wav.extend(size.to_le_bytes());
    wav.extend(data.iter().flat_map(|value| value.to_le_bytes()));
    std::fs::write(path, wav).unwrap();
}

#[test]
fn mixes_different_sets_and_indexes_when_saving_a_circle() {
    let scratch = Scratch::new("application-save-mix");
    let root = scratch.path("mapset");
    std::fs::create_dir(&root).unwrap();
    let path = root.join("test.osu");
    let backups = scratch.path("backups");
    std::fs::write(&path, MAP).unwrap();
    let normal = root.join("normal-hitnormal2.wav");
    let clap = root.join("soft-hitclap3.wav");
    sample(&normal, &[8192, 0, 4096]);
    sample(&clap, &[0, 8192]);
    // The circle already uses the numbered hitnormal. At 50%, both layers should be audible.
    let original = String::from_utf8(MAP.to_vec())
        .unwrap()
        .replace("0:0:0:0:", "1:0:2:50:");
    std::fs::write(&path, &original).unwrap();
    let mut d = Document::read(&path).unwrap();
    d.composer.add(
        1000.0,
        SampleIdentity::Standard {
            set: SampleSet::Soft,
            sound: Sound::Hit(Hitsound::Clap),
            index: CustomIndex::new(3),
        },
        50,
        5.0,
    );
    let initial_files = || std::fs::read_dir(&root).unwrap().count();
    assert_eq!(initial_files(), 3);
    std::fs::write(&path, "external edit").unwrap();
    assert!(
        save_document(&d, backups.clone())
            .err()
            .unwrap()
            .contains("changed on disk")
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "external edit");
    assert_eq!(initial_files(), 3);

    std::fs::write(&path, &original).unwrap();
    std::fs::write(&backups, "backup destination is blocked").unwrap();
    assert!(save_document(&d, backups.clone()).is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
    assert_eq!(initial_files(), 3);
    std::fs::remove_file(&backups).unwrap();

    let original_clap = std::fs::read(&clap).unwrap();
    let original_normal = std::fs::read(&normal).unwrap();
    std::fs::remove_file(&clap).unwrap();
    let missing = save_document(&d, backups.clone()).err().unwrap();
    assert!(missing.contains("soft-hitclap3"), "{missing}");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
    assert_eq!(initial_files(), 2);
    std::fs::write(&clap, &original_clap).unwrap();

    let saved = save_document(&d, backups.clone()).unwrap();
    assert!(saved.problems.is_empty());
    let reopened = Document::read(&path).unwrap();
    let moment = &reopened.composer.hitsounds().moments()[0];
    let [SampleIdentity::File(name)] = &moment.hits().collect::<Vec<_>>()[..] else {
        panic!("expected a single generated sample");
    };
    assert_eq!(moment.volume(), 100);
    let rendered = dawsu_audio::decode(&root.join(name)).unwrap();
    assert_eq!(rendered.sample_rate, 48000);
    for (frame, expected) in rendered
        .samples
        .chunks_exact(rendered.channels)
        .zip([0.125, 0.125, 0.0625])
    {
        for &channel in frame {
            assert!(
                (channel - expected).abs() < 0.0001,
                "{channel} vs {expected}"
            );
        }
    }
    assert_eq!(rendered.samples.len(), 3 * rendered.channels);
    let bytes = std::fs::read(&path).unwrap();
    let expected = original.replace(
        "256,192,1000,1,0,1:0:2:50:",
        &format!("256,192,1000,1,0,1:0:2:100:{name}"),
    );
    assert_eq!(bytes, expected.as_bytes());
    let backup_dir = std::fs::read_dir(&backups)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let backup = std::fs::read_dir(backup_dir)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert_eq!(std::fs::read_to_string(backup).unwrap(), original);
    d.file = saved.file;
    d.composer.mark_saved();
    save_document(&d, backups.clone()).unwrap();
    save_document(&reopened, backups).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert_eq!(initial_files(), 4);
    assert_eq!(std::fs::read(normal).unwrap(), original_normal);
    assert_eq!(std::fs::read(clap).unwrap(), original_clap);
}

#[test]
fn mixes_slider_tail_with_reserved_indexes_and_checked_green_lines() {
    let scratch = Scratch::new("application-slider-mix");
    let root = scratch.path("mapset");
    std::fs::create_dir(&root).unwrap();
    let path = root.join("Hard.osu");
    let sibling = root.join("Easy.osu");
    let backups = scratch.path("backups");
    let slider = "256,192,1000,2,0,L|396:192,1,140,0|0,1:0|1:0,1:0:0:0:";
    let neighbor = "256,192,2000,1,0,1:0:2:70:";
    let header = String::from_utf8(MAP.to_vec()).unwrap()
        .replace("0,500,4,1,0,100,1,0", "0,500,4,1,2,50,1,0")
        .replace("256,192,1000,1,0,0:0:0:0:\r\n", "");
    let old = format!("{header}{slider}\r\n{neighbor}\r\n");
    let sibling_bytes = format!("{header}256,192,3000,1,0,1:0:4:100:\r\n");
    std::fs::write(&path, &old).unwrap();
    std::fs::write(&sibling, &sibling_bytes).unwrap();
    let original_normal = root.join("normal-hitnormal2.wav");
    let original_clap = root.join("soft-hitclap3.wav");
    sample(&original_normal, &[8192, 0, 4096]);
    sample(&original_clap, &[0, 8192]);
    let normal_bytes = std::fs::read(&original_normal).unwrap();
    let clap_bytes = std::fs::read(&original_clap).unwrap();
    let generated = root.join("normal-hitnormal5.wav");
    let mut d = Document::read(&path).unwrap();
    let add = SampleIdentity::Standard {
        set: SampleSet::Soft,
        sound: Sound::Hit(Hitsound::Clap),
        index: CustomIndex::new(3),
    };
    d.composer.add(1500.0, add, 50, 1.0);
    std::fs::write(&backups, "cannot use this as a directory").unwrap();
    assert!(save_document(&d, backups.clone()).is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), old);
    assert!(!generated.exists());
    std::fs::remove_file(&backups).unwrap();
    let saved = save_document(&d, backups.clone()).unwrap();
    let new_bytes = std::fs::read(&path).unwrap();
    let reopened = Document::read(&path).unwrap();
    assert_eq!(reopened.file.line_text(reopened.file.hit_objects()[1].line()).as_deref(), Some(neighbor));
    let tail = reopened.composer.hitsounds().moments().iter().find(|m| m.time().ms() == 1500.0).unwrap();
    assert_eq!(tail.volume(), 100);
    assert_eq!(tail.hits().cloned().collect::<Vec<_>>(), [SampleIdentity::Standard {
        set: SampleSet::Normal,
        sound: Sound::Hit(Hitsound::Normal),
        index: CustomIndex::new(5),
    }]);
    let clip = dawsu_audio::decode(&generated).unwrap();
    assert_eq!(clip.samples, [0.125, 0.125, 0.125, 0.125, 0.0625, 0.0625]);
    let text = String::from_utf8(new_bytes.clone()).unwrap();
    assert!(text.contains(&format!("{slider}\r\n{neighbor}\r\n")), "{text}");
    assert!(text.contains("1500,") && text.contains("1506,"), "{text}");
    d.file = saved.file;
    d.composer.mark_saved();
    save_document(&d, backups.clone()).unwrap();
    save_document(&reopened, backups).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), new_bytes);
    assert_eq!(std::fs::read_to_string(&sibling).unwrap(), sibling_bytes);
    assert_eq!(std::fs::read(original_normal).unwrap(), normal_bytes);
    assert_eq!(std::fs::read(original_clap).unwrap(), clap_bytes);
    assert_eq!(std::fs::read_dir(root).unwrap().count(), 5);
}
