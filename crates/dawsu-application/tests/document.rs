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
    // Save merges per role: the hitnormal and the clap each get their own file at one new index.
    let mut hits: Vec<_> = moment.hits().cloned().collect();
    hits.sort();
    assert_eq!(hits, [std_lane(SampleSet::Normal, Hitsound::Normal, 4), std_lane(SampleSet::Soft, Hitsound::Clap, 4)]);
    assert_eq!(moment.volume(), 50);
    assert_eq!(dawsu_audio::decode(&root.join("normal-hitnormal4.wav")).unwrap().sample_rate, 48000);
    assert_frames(&root.join("normal-hitnormal4.wav"), &[0.25, 0.0, 0.125]);
    assert_frames(&root.join("soft-hitclap4.wav"), &[0.0, 0.25]);
    let bytes = std::fs::read(&path).unwrap();
    let expected = original.replace(
        "256,192,1000,1,0,1:0:2:50:",
        "256,192,1000,1,8,1:2:4:50:",
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
    assert_eq!(initial_files(), 5);
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
    // Save merges per role: the tail keeps a hitnormal and gets its clap back as a clap.
    assert_eq!(tail.volume(), 50);
    let mut hits: Vec<_> = tail.hits().cloned().collect();
    hits.sort();
    assert_eq!(hits, [std_lane(SampleSet::Normal, Hitsound::Normal, 5), std_lane(SampleSet::Soft, Hitsound::Clap, 5)]);
    assert_frames(&generated, &[0.25, 0.0, 0.125]);
    assert_frames(&root.join("soft-hitclap5.wav"), &[0.0, 0.25]);
    let text = String::from_utf8(new_bytes.clone()).unwrap();
    let slider = "256,192,1000,2,0,L|396:192,1,140,0|8,1:0|1:2,1:0:0:0:";
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
    assert_eq!(std::fs::read_dir(root).unwrap().count(), 6);
}

fn std_lane(set: SampleSet, sound: Hitsound, index: u32) -> SampleIdentity {
    SampleIdentity::Standard { set, sound: Sound::Hit(sound), index: CustomIndex::new(index) }
}

fn file_names(root: &std::path::Path) -> Vec<String> {
    let mut names: Vec<_> = std::fs::read_dir(root).unwrap().map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    names
}

fn assert_frames(path: &std::path::Path, expected: &[f32]) {
    let clip = dawsu_audio::decode(path).unwrap();
    let actual: Vec<f32> = clip.samples.chunks_exact(clip.channels).map(|frame| frame[0]).collect();
    assert_eq!(actual.len(), expected.len(), "{}: {actual:?}", path.display());
    for (a, e) in actual.iter().zip(expected) {
        assert!((a - e).abs() < 0.0001, "{}: {actual:?} vs {expected:?}", path.display());
    }
}

/// A mapset whose difficulty has `objects` and the timing line `timing`, beside 48 kHz mono samples.
fn mapset(scratch: &Scratch, objects: &str, timing: &str, samples: &[(&str, &[i16])]) -> (std::path::PathBuf, std::path::PathBuf) {
    let root = scratch.path("mapset");
    std::fs::create_dir(&root).unwrap();
    let path = root.join("test.osu");
    let text = String::from_utf8(MAP.to_vec()).unwrap()
        .replace("0,500,4,1,0,100,1,0", timing)
        .replace("256,192,1000,1,0,0:0:0:0:\r\n", objects);
    std::fs::write(&path, text).unwrap();
    for (name, data) in samples {
        sample(&root.join(name), data);
    }
    (root, path)
}

fn moment_at(d: &Document, time: f64) -> dawsu_hitsounding::MomentId {
    dawsu_hitsounding::MomentId(d.composer.hitsounds().moments().iter().position(|m| m.time().ms() == time).unwrap())
}

fn set_hits(d: &mut Document, time: f64, volume: i32, hits: Vec<(SampleIdentity, i32)>) {
    let id = moment_at(d, time);
    d.composer.apply("Set hits", |h| h.set_moment_state(id, volume, hits));
}

fn sorted_hits(d: &Document, time: f64) -> Vec<SampleIdentity> {
    let mut hits: Vec<_> = d.composer.hitsounds().moment(moment_at(d, time)).hits().cloned().collect();
    hits.sort();
    hits
}

/// Reopens the saved difficulty and saves it again: nothing changes.
fn resave_unchanged(path: &std::path::Path, backups: std::path::PathBuf) {
    let bytes = std::fs::read(path).unwrap();
    save_document(&Document::read(path).unwrap(), backups).unwrap();
    assert_eq!(std::fs::read(path).unwrap(), bytes);
}

#[test]
fn save_keeps_the_hitnormal_and_merges_the_finishes_into_one_file() {
    let scratch = Scratch::new("application-save-roles");
    let (root, path) = mapset(&scratch, "256,192,1000,1,0,1:0:2:80:\r\n", "0,500,4,1,0,100,1,0", &[
        ("normal-hitnormal2.wav", &[8192, 0, 4096]),
        ("drum-hitfinish3.wav", &[0, 8192]),
        ("soft-hitfinish4.wav", &[4096]),
    ]);
    let before = file_names(&root);
    let mut d = Document::read(&path).unwrap();
    set_hits(&mut d, 1000.0, 80, vec![
        (std_lane(SampleSet::Normal, Hitsound::Normal, 2), 80),
        (std_lane(SampleSet::Drum, Hitsound::Finish, 3), 80),
        (std_lane(SampleSet::Soft, Hitsound::Finish, 4), 80),
    ]);
    let saved = save_document(&d, scratch.path("backups")).unwrap();
    assert!(saved.problems.is_empty());
    let after = file_names(&root);
    eprintln!("before Save: {before:?}\nafter Save:  {after:?}\n{}", std::fs::read_to_string(&path).unwrap());
    assert!(after.iter().all(|name| !name.starts_with("dawsu-mix-")), "{after:?}");
    // The hitnormal is kept as it is. Both finishes go into one finish file; at one volume each
    // set is used once, so soft names it.
    assert_eq!(after, ["drum-hitfinish3.wav", "normal-hitnormal2.wav", "normal-hitnormal5.wav", "soft-hitfinish4.wav", "soft-hitfinish5.wav", "test.osu"]);
    assert_frames(&root.join("normal-hitnormal5.wav"), &[0.25, 0.0, 0.125]);
    assert_frames(&root.join("soft-hitfinish5.wav"), &[0.125, 0.25]);
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("256,192,1000,1,4,1:2:5:80:\r\n"), "{text}");
    let reopened = Document::read(&path).unwrap();
    assert_eq!(sorted_hits(&reopened, 1000.0), [std_lane(SampleSet::Normal, Hitsound::Normal, 5), std_lane(SampleSet::Soft, Hitsound::Finish, 5)]);
    assert_eq!(reopened.composer.hitsounds().moment(moment_at(&reopened, 1000.0)).volume(), 80);
    resave_unchanged(&path, scratch.path("backups"));
    // Drawing the same sounds again and saving reuses index 5: same line, no new files.
    let bytes = std::fs::read(&path).unwrap();
    let mut again = Document::read(&path).unwrap();
    set_hits(&mut again, 1000.0, 80, vec![
        (std_lane(SampleSet::Normal, Hitsound::Normal, 2), 80),
        (std_lane(SampleSet::Drum, Hitsound::Finish, 3), 80),
        (std_lane(SampleSet::Soft, Hitsound::Finish, 4), 80),
    ]);
    save_document(&again, scratch.path("backups")).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert_eq!(file_names(&root), after);
}

#[test]
fn save_mixes_two_hitnormals_into_one_hitnormal_file() {
    let scratch = Scratch::new("application-save-two-normals");
    let (root, path) = mapset(&scratch, "256,192,1000,1,0,1:0:2:80:\r\n", "0,500,4,1,0,100,1,0", &[
        ("normal-hitnormal2.wav", &[8192, 0]),
        ("soft-hitnormal3.wav", &[0, 8192]),
    ]);
    let mut d = Document::read(&path).unwrap();
    set_hits(&mut d, 1000.0, 80, vec![
        (std_lane(SampleSet::Normal, Hitsound::Normal, 2), 80),
        (std_lane(SampleSet::Soft, Hitsound::Normal, 3), 80),
    ]);
    save_document(&d, scratch.path("backups")).unwrap();
    assert_eq!(file_names(&root), ["normal-hitnormal2.wav", "soft-hitnormal3.wav", "soft-hitnormal4.wav", "test.osu"]);
    assert_frames(&root.join("soft-hitnormal4.wav"), &[0.25, 0.25]);
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("256,192,1000,1,0,2:0:4:80:\r\n"), "{text}");
    let reopened = Document::read(&path).unwrap();
    assert_eq!(sorted_hits(&reopened, 1000.0), [std_lane(SampleSet::Soft, Hitsound::Normal, 4)]);
    resave_unchanged(&path, scratch.path("backups"));
}

#[test]
fn moments_with_the_same_files_share_one_index() {
    let scratch = Scratch::new("application-save-role-reuse");
    let objects = "256,192,1000,1,0,1:0:2:80:\r\n256,192,2000,1,0,1:0:2:100:\r\n256,192,3000,1,0,1:0:2:80:\r\n256,192,4000,1,0,1:0:2:80:\r\n";
    let (root, path) = mapset(&scratch, objects, "0,500,4,1,0,100,1,0", &[
        ("normal-hitnormal2.wav", &[8192]),
        ("drum-hitfinish3.wav", &[16384]),
        ("drum-hitwhistle3.wav", &[4096]),
    ]);
    let mut d = Document::read(&path).unwrap();
    let normal = std_lane(SampleSet::Normal, Hitsound::Normal, 2);
    let finish = std_lane(SampleSet::Drum, Hitsound::Finish, 3);
    // 1000 and 2000 differ in volume but play the same files.
    set_hits(&mut d, 1000.0, 80, vec![(normal.clone(), 80), (finish.clone(), 80)]);
    set_hits(&mut d, 2000.0, 100, vec![(normal.clone(), 100), (finish.clone(), 100)]);
    set_hits(&mut d, 3000.0, 80, vec![(normal.clone(), 80), (std_lane(SampleSet::Drum, Hitsound::Whistle, 3), 80)]);
    // A moment osu! can hold is written on its object, with no file.
    set_hits(&mut d, 4000.0, 80, vec![(normal.clone(), 80), (std_lane(SampleSet::Drum, Hitsound::Finish, 2), 80)]);
    save_document(&d, scratch.path("backups")).unwrap();
    assert_eq!(file_names(&root), [
        "drum-hitfinish3.wav", "drum-hitfinish4.wav", "drum-hitwhistle3.wav", "drum-hitwhistle5.wav",
        "normal-hitnormal2.wav", "normal-hitnormal4.wav", "normal-hitnormal5.wav", "test.osu",
    ]);
    let text = std::fs::read_to_string(&path).unwrap();
    for line in ["256,192,1000,1,4,1:3:4:80:", "256,192,2000,1,4,1:3:4:100:", "256,192,3000,1,2,1:3:5:80:", "256,192,4000,1,4,1:3:2:80:"] {
        assert!(text.contains(&format!("{line}\r\n")), "{line} in {text}");
    }
    assert_frames(&root.join("drum-hitfinish4.wav"), &[0.5]);

    // A later Save of the same sounds reuses the files already in the mapset.
    let mut reopened = Document::read(&path).unwrap();
    set_hits(&mut reopened, 4000.0, 80, vec![(normal.clone(), 80), (finish.clone(), 80)]);
    save_document(&reopened, scratch.path("backups")).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("256,192,4000,1,4,1:3:4:80:\r\n"), "{text}");
    assert_eq!(file_names(&root).len(), 8);
}

#[test]
fn a_slider_head_merges_per_role_through_a_green_line() {
    let scratch = Scratch::new("application-save-role-slider");
    let slider = "256,192,1000,2,0,L|396:192,1,140,0|0,1:0|1:0,1:0:0:0:\r\n";
    let (root, path) = mapset(&scratch, slider, "0,500,4,1,2,50,1,0", &[
        ("normal-hitnormal2.wav", &[8192]),
        ("soft-hitclap3.wav", &[4096]),
    ]);
    let mut d = Document::read(&path).unwrap();
    set_hits(&mut d, 1000.0, 50, vec![
        (std_lane(SampleSet::Normal, Hitsound::Normal, 2), 50),
        (std_lane(SampleSet::Soft, Hitsound::Clap, 3), 50),
    ]);
    let saved = save_document(&d, scratch.path("backups")).unwrap();
    assert!(saved.problems.is_empty());
    assert_eq!(file_names(&root), ["normal-hitnormal2.wav", "normal-hitnormal4.wav", "soft-hitclap3.wav", "soft-hitclap4.wav", "test.osu"]);
    assert_frames(&root.join("normal-hitnormal4.wav"), &[0.25]);
    assert_frames(&root.join("soft-hitclap4.wav"), &[0.125]);
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("256,192,1000,2,0,L|396:192,1,140,8|0,1:2|1:0,1:0:0:0:\r\n"), "{text}");
    assert!(text.contains("1000,-100,4,1,4,50,0,0\r\n") && text.contains("1006,-100,4,1,2,50,0,0\r\n"), "{text}");
    let reopened = Document::read(&path).unwrap();
    assert_eq!(sorted_hits(&reopened, 1000.0), [std_lane(SampleSet::Normal, Hitsound::Normal, 4), std_lane(SampleSet::Soft, Hitsound::Clap, 4)]);
    resave_unchanged(&path, scratch.path("backups"));
}

#[test]
fn a_clap_only_merge_writes_no_hitnormal_and_avoids_an_index_with_one() {
    let scratch = Scratch::new("application-save-no-hitnormal");
    let objects = "256,192,1000,1,0,1:0:0:80:\r\n256,192,2000,1,0,1:0:0:80:\r\n";
    let (root, path) = mapset(&scratch, objects, "0,500,4,1,0,100,1,0", &[
        ("drum-hitclap3.wav", &[8192]),
        ("soft-hitclap4.wav", &[4096]),
    ]);
    let claps = || vec![(std_lane(SampleSet::Drum, Hitsound::Clap, 3), 80), (std_lane(SampleSet::Soft, Hitsound::Clap, 4), 80)];
    let mut d = Document::read(&path).unwrap();
    set_hits(&mut d, 1000.0, 80, claps());
    save_document(&d, scratch.path("backups")).unwrap();
    // No hitnormal file: the skin plays the hitnormal, in the normal set the circle had.
    assert_eq!(file_names(&root), ["drum-hitclap3.wav", "soft-hitclap2.wav", "soft-hitclap4.wav", "test.osu"]);
    assert_frames(&root.join("soft-hitclap2.wav"), &[0.375]);
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("256,192,1000,1,8,1:2:2:80:\r\n"), "{text}");

    // Another moment's soft-hitnormal2 now sits at index 2, so the same claps can't reuse it:
    // osu! would play that file instead of the skin's hitnormal.
    sample(&root.join("soft-hitnormal2.wav"), &[100]);
    let mut d = Document::read(&path).unwrap();
    set_hits(&mut d, 2000.0, 80, claps());
    save_document(&d, scratch.path("backups")).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("256,192,2000,1,8,1:2:5:80:\r\n"), "{text}");
    assert_eq!(file_names(&root), ["drum-hitclap3.wav", "soft-hitclap2.wav", "soft-hitclap4.wav", "soft-hitclap5.wav", "soft-hitnormal2.wav", "test.osu"]);
}
