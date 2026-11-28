use super::*;
use dawsu_test_support::Scratch;
use std::time::{Duration, Instant};


#[test]
fn selected_lane_volume_and_shortcut_leave_other_voices_at_the_moment() {
    let fixture = Scratch::new("selected-lane-volume");
    let path = fixture.path("map.osu");
    let map = b"osu file format v14\n[General]\nMode:0\n[TimingPoints]\n0,500,4,1,1,70,1,0\n[HitObjects]\n256,192,1000,1,8,1:1:1:70:\n128,192,1000,1,0,1:1:1:70:\n";
    std::fs::write(&path, map).unwrap();
    let wav_head = b"RIFF\x26\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data\x02\0\0\0";
    for (name, sample) in [("normal-hitnormal.wav", 8192_i16), ("normal-hitclap.wav", 16384)] {
        let mut wav = wav_head.to_vec();
        wav.extend(sample.to_le_bytes());
        std::fs::write(fixture.path(name), wav).unwrap();
    }
    let mut app = Application::headless();
    app.project_root = Some(fixture.path("sessions"));
    open(&mut app, path.clone());
    let lanes: Vec<_> = app.document().unwrap().composer.hitsounds().moments()[0].hits().cloned().collect();
    let normal = lanes.iter().find(|lane| matches!(lane,
        SampleIdentity::Standard { sound: Sound::Hit(Hitsound::Normal), .. })).unwrap().clone();
    let clap = lanes.iter().find(|lane| *lane != &normal).unwrap().clone();
    app.select_hit(HitKey { moment: 0, lane: clap.clone() }, false);
    app.set_selection_lane_scope(Some(BTreeSet::from([normal.clone()]))).unwrap();
    app.selection_volume(30, false);
    app.command("volumeUp");
    let gains = |app: &Application| {
        let moment = &app.document().unwrap().composer.hitsounds().moments()[0];
        (moment.hit_volume(&normal), moment.hit_volume(&clap))
    };
    assert_eq!(gains(&app), (35, 70));
    use dawsu_editor::keys::{Key, Mods};
    let fine = Mods::ALT.union(Mods::SHIFT);
    app.key(KeyChord::new(Key::Up, fine));
    assert_eq!(gains(&app), (36, 70));
    app.key(KeyChord::new(Key::Down, fine));
    assert_eq!(gains(&app), (35, 70));
    app.command("undo");
    assert_eq!(gains(&app), (36, 70));
    app.command("undo");
    assert_eq!(gains(&app), (35, 70));
    app.command("undo");
    assert_eq!(gains(&app), (30, 70));
    app.command("undo");
    assert_eq!(gains(&app), (70, 70));
    app.command("redo");
    app.command("redo");
    let save = |app: &mut Application| {
        app.save_with_backups(fixture.path("backups"));
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.busy {
            app.poll();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(app.error.is_none(), "{:?}", app.error);
    };
    save(&mut app);
    let written = std::fs::read(&path).unwrap();
    assert_ne!(written, map);
    assert!(written.ends_with(b"128,192,1000,1,0,1:1:1:70:\n"));
    let decoded = Document::read(&path).unwrap();
    // Save merges per role at the clap's volume: the hitnormal file holds it at half.
    let saved = &decoded.composer.hitsounds().moments()[0];
    assert_eq!(saved.volume(), 70);
    assert!(saved.has_hit(&SampleIdentity::Standard { set: SampleSet::Normal, sound: Sound::Hit(Hitsound::Normal), index: dawsu_foundation::CustomIndex::new(2) }));
    let pcm = dawsu_audio::decode(&fixture.path("normal-hitnormal2.wav")).unwrap();
    assert!((pcm.samples[0] - 0.25 * 0.5).abs() < 0.0001);
    let pcm = dawsu_audio::decode(&fixture.path("normal-hitclap2.wav")).unwrap();
    assert!((pcm.samples[0] - 0.5).abs() < 0.0001);
    let mut reopened = Application::headless();
    reopened.project_root = app.project_root.clone();
    open(&mut reopened, path.clone());
    assert_eq!(gains(&reopened), (35, 70));
    save(&mut reopened);
    assert_eq!(std::fs::read(&path).unwrap(), written);

    // A pending recovery record must stay inactive if the map write fails.
    reopened.select_hit(HitKey { moment: 0, lane: clap.clone() }, false);
    reopened.set_selection_lane_scope(Some(BTreeSet::from([normal.clone()]))).unwrap();
    reopened.selection_volume(55, false);
    let blocked_backup = fixture.path("blocked");
    std::fs::write(&blocked_backup, b"not a backup directory").unwrap();
    reopened.save_with_backups(blocked_backup);
    let deadline = Instant::now() + Duration::from_secs(10);
    while reopened.busy {
        reopened.poll();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(reopened.error.is_some());
    assert!(reopened.is_dirty());
    assert_eq!(std::fs::read(&path).unwrap(), written);

    let mut retry = Application::headless();
    retry.project_root = app.project_root.clone();
    open(&mut retry, path.clone());
    assert_eq!(gains(&retry), (35, 70));

    let original_sample = fixture.path("normal-hitclap.wav");
    let mut changed = std::fs::read(&original_sample).unwrap();
    changed[44] ^= 1;
    std::fs::write(&original_sample, changed).unwrap();
    let mut external = Application::headless();
    external.project_root = app.project_root.clone();
    open(&mut external, path.clone());
    assert!(external.warnings.iter().any(|warning| warning.contains("An original sample changed")));
    assert!(!external.document().unwrap().composer.hitsounds().moments()[0].has_hit(&clap));
    assert_eq!(std::fs::read(path).unwrap(), written);
}





/// HS mode: the circle needs a sample mix of two files, which Save renders; the slider's tail
/// asks for a soft and a drum clap, which no object line can hold.
fn mix_and_unwritable(scratch: &Scratch, second: &str) -> (Document, Hitsounds, project::Project) {
    let folder = scratch.path("mapset");
    std::fs::create_dir(&folder).unwrap();
    let map = folder.join("a.osu");
    std::fs::write(&map, SLIDER_MAP).unwrap();
    wav(&folder.join("a.wav"), 8192);
    wav(&folder.join("b.wav"), 4096);
    // A file in the folder that is not audio, for the test whose mix must fail.
    if second == "broken.wav" {
        std::fs::write(folder.join(second), b"not audio").unwrap();
    }
    let document = Document::read(&map).unwrap();
    let mut hits = document.composer.hitsounds().clone();
    hits.set_moment_state(dawsu_hitsounding::MomentId(0), 80, [
        (SampleIdentity::File("a.wav".into()), 80), (SampleIdentity::File(second.into()), 40),
    ]);
    let tail = hits.moments().iter().position(|m| matches!(m.place(), dawsu_hitsounding::Place::SliderNode { node: 1, .. })).unwrap();
    hits.add_hit(dawsu_hitsounding::MomentId(tail), skin(SampleSet::Soft, Hitsound::Clap));
    hits.add_hit(dawsu_hitsounding::MomentId(tail), skin(SampleSet::Drum, Hitsound::Clap));
    let project = project::Project::load(&scratch.path("projects"), None, &document.path).unwrap();
    (document, hits, project)
}

fn prepare(scratch: &Scratch, document: &Document, hits: &Hitsounds, project: Option<&project::Project>, diagnostics: &mut Vec<OutputProblem>) -> Result<PreparedSave, String> {
    prepare_save(
        PrepareSaveInput {
            file: document.compile_base.clone(),
            hits,
            written: &document.written,
            expected: document.file.original_bytes().to_vec(),
            path: &document.path,
            backups: scratch.path("backups"),
            hs: true,
            owned: &BTreeSet::new(),
            defaults: Path::new(""),
            pending: &document.pending,
            project,
            review: false,
            clean_redundant_greenlines: false,
        },
        &|| false,
        diagnostics,
        &mut |_, _| {},
    )
}

#[test]
fn save_renders_the_mixes_it_can_and_lists_only_the_parts_it_leaves_out() {
    let scratch = Scratch::new("mix-beside-left-out");
    let (document, hits, project) = mix_and_unwritable(&scratch, "b.wav");
    let mut diagnostics = Vec::new();
    let plan = prepare(&scratch, &document, &hits, Some(&project), &mut diagnostics).unwrap();
    assert_eq!(diagnostics.iter().map(|p| p.time).collect::<Vec<_>>(), [2500.0], "the rendered mix is not a problem");
    assert_eq!(plan.left_out.len(), 1);
    assert_eq!(plan.mix_files.staged.len(), 1, "the circle's mix is rendered");
}

#[test]
fn hs_save_merges_a_circle_per_role_and_leaves_a_slider_head_out() {
    let scratch = Scratch::new("hs-merge-roles");
    let folder = scratch.path("mapset");
    std::fs::create_dir(&folder).unwrap();
    let map = folder.join("a.osu");
    std::fs::write(&map, SLIDER_MAP).unwrap();
    wav(&folder.join("normal-hitnormal3.wav"), 8192);
    wav(&folder.join("drum-hitfinish3.wav"), 4096);
    wav(&folder.join("soft-hitfinish4.wav"), 16384);
    let document = Document::read(&map).unwrap();
    let mut hits = document.composer.hitsounds().clone();
    let numbered = |set, sound, index| SampleIdentity::Standard { set, sound: Sound::Hit(sound), index: dawsu_foundation::CustomIndex::new(index) };
    hits.set_moment_state(dawsu_hitsounding::MomentId(0), 80, [
        (numbered(SampleSet::Normal, Hitsound::Normal, 3), 80),
        (numbered(SampleSet::Drum, Hitsound::Finish, 3), 80),
        (numbered(SampleSet::Soft, Hitsound::Finish, 4), 40),
    ]);
    // On objects only, a slider head cannot take a new index, so it stays left out.
    let head = hits.moments().iter().position(|m| matches!(m.place(), dawsu_hitsounding::Place::SliderNode { node: 0, .. })).unwrap();
    hits.set_moment_state(dawsu_hitsounding::MomentId(head), 70, [
        (skin(SampleSet::Normal, Hitsound::Normal), 70), (skin(SampleSet::Drum, Hitsound::Finish), 70), (skin(SampleSet::Soft, Hitsound::Finish), 35),
    ]);
    let project = project::Project::load(&scratch.path("projects"), None, &document.path).unwrap();
    let mut diagnostics = Vec::new();
    let plan = prepare(&scratch, &document, &hits, Some(&project), &mut diagnostics).unwrap();
    let names: Vec<_> = plan.mix_files.staged.iter().map(|mix| mix.path.file_name().unwrap().to_string_lossy().into_owned()).collect();
    assert_eq!(names, ["normal-hitnormal2.wav", "drum-hitfinish2.wav"]);
    assert!((plan.mix_files.staged[0].clip().samples[0] - 0.25).abs() < 0.0001);
    assert!((plan.mix_files.staged[1].clip().samples[0] - (0.125 + 0.5 * 0.5)).abs() < 0.0001);
    let text = String::from_utf8(plan.out.to_bytes()).unwrap();
    assert!(text.contains("256,192,1000,1,4,0:3:2:80:\r\n"), "{text}");
    assert_eq!(plan.left_out.len(), 1);
    assert_eq!(diagnostics.iter().map(|p| p.time).collect::<Vec<_>>(), [2000.0]);
}

#[test]
fn a_slider_sound_on_a_hit_is_left_out_and_save_writes_the_rest() {
    let scratch = Scratch::new("slider-sound-left-out");
    let folder = scratch.path("mapset");
    std::fs::create_dir(&folder).unwrap();
    let map = folder.join("a.osu");
    std::fs::write(&map, SLIDER_MAP).unwrap();
    let document = Document::read(&map).unwrap();
    let mut hits = document.composer.hitsounds().clone();
    let slide = SampleIdentity::Standard { set: SampleSet::Soft, sound: Sound::Slider(SliderSound::Slide), index: dawsu_foundation::CustomIndex::new(0) };
    hits.add_hit(dawsu_hitsounding::MomentId(0), slide);
    let tail = tail_id(&hits);
    hits.add_hit(tail, skin(SampleSet::Normal, Hitsound::Whistle));
    let project = project::Project::load(&scratch.path("projects"), None, &document.path).unwrap();
    let mut diagnostics = Vec::new();
    let plan = prepare(&scratch, &document, &hits, Some(&project), &mut diagnostics).unwrap();
    // The tail's whistle is written; the circle keeps its old line and is listed.
    let text = String::from_utf8(plan.out.to_bytes()).unwrap();
    assert!(text.contains("256,192,1000,1,0,0:0:0:0:\r\n"), "{text}");
    assert!(text.contains("100,100,2000,2,0,L|200:100,1,140,0|2"), "{text}");
    assert_eq!(plan.left_out.len(), 1);
    assert!(plan.mix_files.staged.is_empty());
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].time, 1000.0);
    assert_eq!(diagnostics[0].message, "At 1000 ms a slider sound (soft-sliderslide) is on a hit. osu! plays slider sounds only on a slider body. Remove it from the hit.");
}

/// The mix fails on a file that is in the folder but is not audio. A file that is not in the
/// folder no longer fails it: osu! plays the normal hitnormal in its place (`Lane::plays_as`),
/// and dawsu! ships that sound.
#[test]
fn a_failed_mix_render_stops_the_whole_save() {
    let scratch = Scratch::new("mix-render-fails");
    let (document, hits, project) = mix_and_unwritable(&scratch, "broken.wav");
    let mut diagnostics = Vec::new();
    let error = prepare(&scratch, &document, &hits, Some(&project), &mut diagnostics).err().unwrap();
    assert!(error.contains("Could not mix"), "{error}");
    assert_eq!(diagnostics.iter().map(|p| p.time).collect::<Vec<_>>(), [1000.0]);
    assert!(diagnostics[0].message.contains("Could not mix"));
}


/// HS mode with a soft and a drum clap on the slider's tail, which no object line can hold, and
/// a finish on the circle, which Save can write.


#[test]
fn ctrl_s_with_left_out_moments_shows_the_summary_and_its_save_writes_the_rest() {
    let scratch = Scratch::new("save-leaves-out");
    let (mut app, map) = open_left_out_fixture(&scratch, true);
    app.save_with_backups(scratch.path("backups"));
    wait(&mut app);
    assert!(app.error.is_none(), "{:?}", app.error);
    assert!(app.save_problem_review(), "Ctrl+S shows the Save hitsounds summary first");
    assert!(app.save_review_files().is_some(), "the summary's Save writes the reviewed plan");
    assert_eq!(app.output_problems().iter().map(|p| p.time).collect::<Vec<_>>(), [2500.0]);
    assert_eq!(std::fs::read(&map).unwrap(), SLIDER_MAP, "nothing is written before the summary's Save");
    app.answer_save_review(true);
    wait(&mut app);
    assert!(app.error.is_none(), "{:?}", app.error);
    let saved = std::fs::read(&map).unwrap();
    assert_eq!(line(&saved, "100,100,2000,"), line(SLIDER_MAP, "100,100,2000,"), "the slider keeps its line");
    assert_ne!(line(&saved, "256,192,1000,"), line(SLIDER_MAP, "256,192,1000,"), "the circle's finish is written");
    assert!(!app.is_dirty());
    assert_eq!(app.output_problems().iter().map(|p| p.time).collect::<Vec<_>>(), [2500.0], "the left-out moment stays listed");
    assert!(!app.save_problem_review());
}

#[test]
fn written_follows_a_save_even_with_newer_edits_open_and_reproduces_the_file() {
    let scratch = Scratch::new("written-newer-revision");
    let (mut app, map) = open_left_out_fixture(&scratch, true);
    app.save_with_backups(scratch.path("backups"));
    wait(&mut app);
    app.answer_save_review(true);
    assert!(app.busy);
    // A newer edit lands while the reviewed plan is written.
    app.document.as_mut().unwrap().composer.apply("Newer edit", |hits| hits.set_volume(dawsu_hitsounding::MomentId(0), 55));
    wait(&mut app);
    assert!(app.error.is_none(), "{:?}", app.error);
    assert!(app.status.contains("earlier revision"), "{}", app.status);
    let doc = app.document().unwrap();
    let (again, problems, _) = compile_for_save(&doc.compile_base, &doc.written, true, &BTreeSet::new(), true, &|| false, &mut |_, _| {}).unwrap();
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(again.to_bytes(), std::fs::read(&map).unwrap(), "the hits as last written reproduce the file");
    assert!(!doc.written.moment(tail_id(&doc.written)).is_changed(), "the left-out tail is written as opened");
}

#[test]
fn a_node_written_by_one_save_keeps_those_fields_when_the_next_save_leaves_it_out() {
    let scratch = Scratch::new("node-keeps-written");
    let (mut app, map) = open_left_out_fixture(&scratch, true);
    app.document.as_mut().unwrap().composer.apply("Drum clap only later", |hits| {
        hits.remove_hit(tail_id(hits), &skin(SampleSet::Drum, Hitsound::Clap));
    });
    save_through_summary(&mut app, scratch.path("backups"));
    let first = std::fs::read(&map).unwrap();
    assert_ne!(line(&first, "100,100,2000,"), line(SLIDER_MAP, "100,100,2000,"), "save 1 writes the soft clap");
    app.document.as_mut().unwrap().composer.apply("Drum clap", |hits| {
        hits.add_hit(tail_id(hits), skin(SampleSet::Drum, Hitsound::Clap));
    });
    save_through_summary(&mut app, scratch.path("backups"));
    assert_eq!(app.output_problems().len(), 1);
    assert_eq!(line(&std::fs::read(&map).unwrap(), "100,100,2000,"), line(&first, "100,100,2000,"));
}

/// Under a timing line with custom index 2, a circle cannot ask for the skin's index 0.
const INDEXED_MAP: &[u8] = b"osu file format v14\r\n\r\n[General]\r\nSampleSet: Soft\r\n\r\n[TimingPoints]\r\n0,500,4,2,2,70,1,0\r\n\r\n[HitObjects]\r\n256,192,1000,1,0,0:0:0:0:\r\n";

fn record_json(app: &Application) -> serde_json::Value {
    serde_json::to_value(stored(app).unwritten).unwrap()
}


fn tail_hits(app: &Application) -> Vec<(SampleIdentity, i32)> {
    let hits = app.document().unwrap().composer.hitsounds();
    let tail = hits.moment(tail_id(hits));
    tail.hits().map(|lane| (lane.clone(), tail.hit_volume(lane))).collect()
}

#[test]
fn left_out_moments_are_kept_in_the_project_through_session_writes_and_come_back_on_reopen() {
    let scratch = Scratch::new("record-kept");
    let (mut app, map) = open_left_out_fixture(&scratch, true);
    let other = map.with_file_name("b.osu");
    std::fs::write(&other, SLIDER_MAP).unwrap();
    let drawn = tail_hits(&app);
    save_through_summary(&mut app, scratch.path("backups"));
    let record = record_json(&app);
    assert_eq!(record["current"]["moments"].as_array().map(Vec::len), Some(1), "{record}");
    assert_eq!(record["current"]["moments"][0]["object"], 1);
    assert_eq!(record["current"]["moments"][0]["node"], 1);
    // C1: session writes and a difficulty switch keep the record.
    app.zoom = 0.5;
    app.save_session().unwrap();
    open(&mut app, other.clone());
    open(&mut app, map.clone());
    assert_eq!(tail_hits(&app), drawn, "the drawn hits are back on the tail");
    let hits = app.document().unwrap().composer.hitsounds();
    assert!(hits.moment(tail_id(hits)).is_changed(), "the tail still differs from the file");
    assert!(!app.is_dirty());
    assert_eq!(app.output_problems().iter().map(|p| p.time).collect::<Vec<_>>(), [2500.0], "listed before any Save");
    let reopened = reopen(&app, &map);
    assert_eq!(tail_hits(&reopened), drawn);
    assert_eq!(reopened.output_problems().iter().map(|p| p.time).collect::<Vec<_>>(), [2500.0]);
    assert_eq!(std::fs::read(&other).unwrap(), SLIDER_MAP);
}

#[test]
fn a_digest_mismatch_puts_nothing_back_keeps_the_record_and_names_the_times() {
    let scratch = Scratch::new("record-mismatch");
    let (mut app, map) = open_left_out_fixture(&scratch, true);
    save_through_summary(&mut app, scratch.path("backups"));
    let record = record_json(&app);
    let mut edited = std::fs::read(&map).unwrap();
    edited.extend(b"// edited in another program\r\n");
    std::fs::write(&map, &edited).unwrap();
    let reopened = reopen(&app, &map);
    assert!(reopened.warnings.iter().any(|w| w.contains("changed outside dawsu!") && w.contains("00:02.500")), "{:?}", reopened.warnings);
    assert!(!tail_hits(&reopened).iter().any(|(lane, _)| *lane == skin(SampleSet::Drum, Hitsound::Clap)));
    assert_eq!(record_json(&reopened), record, "the record is kept until the next Save");
}

#[test]
fn a_record_that_fails_its_checks_puts_nothing_back_warns_and_is_kept() {
    let scratch = Scratch::new("record-invalid");
    let (mut app, map) = open_left_out_fixture(&scratch, true);
    save_through_summary(&mut app, scratch.path("backups"));
    let path = app.project.as_ref().unwrap().path().to_path_buf();
    let reopened = reopen(&app, &map);
    let written = {
        let doc = reopened.document().unwrap();
        let tail = doc.written.moment(tail_id(&doc.written));
        tail.hits().map(|lane| (lane.clone(), tail.hit_volume(lane))).collect::<Vec<_>>()
    };
    assert_ne!(tail_hits(&reopened), written, "the untouched record puts the drawn hits back");
    for (field, value, reason) in [
        ("/settings/unwritten/current/moments/0/hits/0/volume", serde_json::json!(150), "volume 150 is out of range"),
        ("/settings/unwritten/current/moments/0/object", serde_json::json!(99), "an object is missing"),
        ("/settings/unwritten/current/moments/0/time_bits", serde_json::json!(1), "a moment moved"),
        ("/settings/unwritten/current/moments/0/hits/0/lane", serde_json::json!("Nope|x"), "unknown lane"),
    ] {
        let mut file: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let original = file.clone();
        *file.pointer_mut(field).unwrap() = value;
        std::fs::write(&path, serde_json::to_vec(&file).unwrap()).unwrap();
        let tampered = std::fs::read(&path).unwrap();
        let reopened = reopen(&app, &map);
        assert!(reopened.warnings.iter().any(|w| w.contains("could not be put back") && w.contains(reason)), "{reason}: {:?}", reopened.warnings);
        assert_eq!(tail_hits(&reopened), written, "{reason}: nothing is put back");
        assert!(reopened.output_problems().is_empty(), "{reason}: nothing is listed");
        assert_eq!(std::fs::read(&path).unwrap(), tampered, "{reason}: the record is kept");
        std::fs::write(&path, serde_json::to_vec(&original).unwrap()).unwrap();
    }
}

#[test]
fn a_failed_record_write_writes_no_osu() {
    let scratch = Scratch::new("record-stage-fails");
    let (mut app, map) = open_left_out_fixture(&scratch, true);
    app.save_session().unwrap();
    // The project's backup folder can't be made, so the staged record can't be written.
    std::fs::write(scratch.path("projects").join("backups"), b"not a folder").unwrap();
    app.save_with_backups(scratch.path("backups"));
    wait(&mut app);
    assert!(app.save_problem_review());
    app.answer_save_review(true);
    wait(&mut app);
    assert!(app.error.is_some());
    assert_eq!(std::fs::read(&map).unwrap(), SLIDER_MAP);
    assert!(app.is_dirty());
}

#[test]
fn a_failed_record_update_after_the_write_names_the_left_out_moments() {
    let scratch = Scratch::new("record-promote-fails");
    let (mut app, map) = open_left_out_fixture(&scratch, true);
    let drawn = tail_hits(&app);
    // No project file yet: the record is staged by creating it, and only the update after the
    // `.osu` write needs the (blocked) backup folder.
    std::fs::create_dir_all(scratch.path("projects")).unwrap();
    std::fs::write(scratch.path("projects").join("backups"), b"not a folder").unwrap();
    app.save_with_backups(scratch.path("backups"));
    wait(&mut app);
    app.answer_save_review(true);
    wait(&mut app);
    assert_ne!(std::fs::read(&map).unwrap(), SLIDER_MAP, "the difficulty is written");
    assert!(app.error.is_some());
    assert_eq!(app.status, "Difficulty saved. Left-out moments could not be recorded.");
    assert_eq!(tail_hits(&reopen(&app, &map)), drawn, "the staged record still puts them back");
}

#[test]
fn a_crash_after_the_osu_write_restores_the_staged_record_on_reopen() {
    let scratch = Scratch::new("record-crash");
    let (mut app, map) = open_left_out_fixture(&scratch, true);
    let drawn = tail_hits(&app);
    save_through_summary(&mut app, scratch.path("backups"));
    // As if dawsu! stopped between the `.osu` write and the promote.
    let path = app.project.as_ref().unwrap().path().to_path_buf();
    let mut file: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let current = file["settings"]["unwritten"]["current"].take();
    file["settings"]["unwritten"] = serde_json::json!({
        "current": null, "pending": current, "before": dawsu_foundation::digest::sha256_hex(SLIDER_MAP),
    });
    std::fs::write(&path, serde_json::to_vec(&file).unwrap()).unwrap();
    assert_eq!(tail_hits(&reopen(&app, &map)), drawn);
}

#[test]
fn fixing_the_moment_and_saving_clears_the_record() {
    let scratch = Scratch::new("record-cleared");
    let (mut app, map) = open_left_out_fixture(&scratch, true);
    save_through_summary(&mut app, scratch.path("backups"));
    app.document.as_mut().unwrap().composer.apply("Fix", |hits| {
        hits.remove_hit(tail_id(hits), &skin(SampleSet::Drum, Hitsound::Clap));
    });
    app.save_with_backups(scratch.path("backups"));
    wait(&mut app);
    assert!(!app.save_problem_review(), "nothing is left out, so Save writes straight away");
    assert!(app.error.is_none(), "{:?}", app.error);
    assert_eq!(record_json(&app)["current"]["moments"], serde_json::json!([]));
    assert_ne!(line(&std::fs::read(&map).unwrap(), "100,100,2000,"), line(SLIDER_MAP, "100,100,2000,"));
    assert!(app.output_problems().is_empty());
}

#[test]
fn saving_twice_with_a_left_out_moment_changes_neither_file() {
    let scratch = Scratch::new("record-twice");
    let (mut app, map) = open_left_out_fixture(&scratch, true);
    save_through_summary(&mut app, scratch.path("backups"));
    let project = app.project.as_ref().unwrap().path().to_path_buf();
    let (osu, json) = (std::fs::read(&map).unwrap(), std::fs::read(&project).unwrap());
    save_through_summary(&mut app, scratch.path("backups"));
    assert_eq!(std::fs::read(&map).unwrap(), osu);
    assert_eq!(std::fs::read(&project).unwrap(), json);
    let mut reopened = reopen(&app, &map);
    save_through_summary(&mut reopened, scratch.path("backups"));
    assert_eq!(std::fs::read(&map).unwrap(), osu);
    assert_eq!(std::fs::read(&project).unwrap(), json);
}

#[test]
fn a_body_volume_left_out_in_hs_mode_comes_back_with_its_head() {
    let scratch = Scratch::new("record-body");
    let folder = scratch.path("mapset");
    std::fs::create_dir(&folder).unwrap();
    let map = folder.join("a.osu");
    std::fs::write(&map, SLIDER_MAP).unwrap();
    let mut app = Application::headless();
    app.project_root = Some(scratch.path("projects"));
    open(&mut app, map.clone());
    app.set_hs_mode(true);
    app.document.as_mut().unwrap().composer.apply("Body volume", |hits| hits.set_body_volume(0, 40));
    save_through_summary(&mut app, scratch.path("backups"));
    assert_eq!(std::fs::read(&map).unwrap(), SLIDER_MAP, "HS mode can't add the green line a body volume needs");
    assert_eq!(app.output_problems().iter().map(|p| p.time).collect::<Vec<_>>(), [2000.0, 2000.0], "the body and its head are both listed");
    assert!(app.status.contains("Saved with backup. 2 parts left out."), "{}", app.status);
    let messages: Vec<_> = app.output_problems().into_iter().map(|p| p.message).collect();
    assert_eq!(messages.iter().filter(|m| m.contains("slider body volume")).count(), 1, "the body's row differs from its head's: {messages:?}");
    let record = record_json(&app);
    assert_eq!(record["current"]["bodies"][0]["volume"], 40, "{record}");
    let reopened = reopen(&app, &map);
    let hits = reopened.document().unwrap().composer.hitsounds();
    assert_eq!(hits.bodies()[0].volume(), 40);
    let head = hits.moments().iter().find(|m| matches!(m.place(), dawsu_hitsounding::Place::SliderNode { node: 0, .. })).unwrap();
    assert_eq!(head.volume(), 40);
    let mut edited = std::fs::read(&map).unwrap();
    edited.extend(b"// edited in another program\r\n");
    std::fs::write(&map, &edited).unwrap();
    let changed = reopen(&app, &map);
    assert!(changed.warnings.iter().any(|w| w.contains("00:02.000") && w.contains("slider body volume")), "{:?}", changed.warnings);
}

#[test]
fn per_hit_volumes_of_a_left_out_node_survive_reopen_and_the_next_save() {
    let scratch = Scratch::new("record-gains");
    let folder = scratch.path("mapset");
    std::fs::create_dir(&folder).unwrap();
    let map = folder.join("a.osu");
    std::fs::write(&map, SLIDER_MAP).unwrap();
    let mut app = Application::headless();
    app.project_root = Some(scratch.path("projects"));
    open(&mut app, map.clone());
    app.set_hs_mode(true);
    app.document.as_mut().unwrap().composer.apply("Two volumes", |hits| {
        let tail = tail_id(hits);
        hits.add_hit(tail, skin(SampleSet::Soft, Hitsound::Clap));
        hits.set_hit_volume(tail, &skin(SampleSet::Soft, Hitsound::Clap), 40);
    });
    let drawn = tail_hits(&app);
    save_through_summary(&mut app, scratch.path("backups"));
    let mut reopened = reopen(&app, &map);
    assert_eq!(tail_hits(&reopened), drawn);
    let hits = reopened.document().unwrap().composer.hitsounds();
    assert!(hits.moment(tail_id(hits)).is_changed(), "gain recovery must not make the drawn gains the baseline");
    assert_eq!(reopened.output_problems().len(), 1, "still listed after reopen");
    save_through_summary(&mut reopened, scratch.path("backups"));
    assert_eq!(record_json(&reopened)["current"]["moments"].as_array().map(Vec::len), Some(1));
    assert_eq!(std::fs::read(&map).unwrap(), SLIDER_MAP);
}

#[test]
fn undo_after_restoring_a_recovery_copy_keeps_loose_hits() {
    let scratch = Scratch::new("recovery-undo-loose");
    let folder = scratch.path("mapset");
    std::fs::create_dir(&folder).unwrap();
    let map = folder.join("a.osu");
    std::fs::write(&map, SLIDER_MAP).unwrap();
    let mut app = Application::headless();
    app.project_root = Some(scratch.path("projects"));
    app.data_root = scratch.path("data");
    open(&mut app, map.clone());
    app.document.as_mut().unwrap().composer.add(3000.0, skin(SampleSet::Soft, Hitsound::Clap), 80, 0.0);
    save_through_summary(&mut app, scratch.path("backups"));
    app.document.as_mut().unwrap().composer.apply("Later edit", |hits| hits.set_volume(dawsu_hitsounding::MomentId(0), 33));
    app.poll();
    let offer = app.write_recovery_copy().unwrap().unwrap();
    let mut restored = Application::create(Preferences::default(), false, None, None, false, scratch.path("data"));
    restored.project_root = app.project_root.clone();
    restored.restore_recovery(&offer.id).unwrap();
    wait(&mut restored);
    assert!(restored.error.is_none(), "{:?}", restored.error);
    restored.command("undo");
    let loose = |app: &Application| app.document().unwrap().composer.hitsounds().moments().iter()
        .filter(|m| m.place() == dawsu_hitsounding::Place::NoObject && m.hits().next().is_some()).count();
    assert_eq!(loose(&restored), 1, "the last Save's loose hit is still drawn after undo");
    assert!(!restored.is_dirty());
    let doc = restored.document().unwrap();
    assert!(doc.written.moments().iter().all(|m| m.place() != dawsu_hitsounding::Place::NoObject || m.hits().next().is_none()),
        "loose hits never count as written outside HS mode");
}

#[test]
fn a_body_volume_left_out_in_normal_mode_keeps_the_head_written_beside_it() {
    let scratch = Scratch::new("record-body-green-lines");
    let folder = scratch.path("mapset");
    std::fs::create_dir(&folder).unwrap();
    let map = folder.join("a.osu");
    std::fs::write(&map, SLIDER_MAP).unwrap();
    let mut app = Application::headless();
    app.project_root = Some(scratch.path("projects"));
    open(&mut app, map.clone());
    app.document.as_mut().unwrap().composer.apply("Body 40, head 60", |hits| {
        hits.set_body_volume(0, 40);
        let head = hits.moments().iter().position(|m| matches!(m.place(), dawsu_hitsounding::Place::SliderNode { node: 0, .. })).unwrap();
        hits.set_volume(dawsu_hitsounding::MomentId(head), 60);
    });
    save_through_summary(&mut app, scratch.path("backups"));
    let saved = std::fs::read(&map).unwrap();
    let text = String::from_utf8_lossy(&saved);
    assert!(text.contains(",60,0,0\r\n"), "the head's green line is written:\n{text}");
    assert_eq!(line(&saved, "100,100,2000,"), line(SLIDER_MAP, "100,100,2000,"));
    let record = record_json(&app);
    assert_eq!(record["current"]["bodies"].as_array().map(Vec::len), Some(1), "{record}");
    assert_eq!(record["current"]["moments"], serde_json::json!([]), "{record}");
    assert_eq!(app.output_problems().len(), 1);
    let project = app.project.as_ref().unwrap().path().to_path_buf();
    let json = std::fs::read(&project).unwrap();
    let mut reopened = reopen(&app, &map);
    let hits = reopened.document().unwrap().composer.hitsounds();
    assert_eq!(hits.bodies()[0].volume(), 40);
    let head = hits.moments().iter().find(|m| matches!(m.place(), dawsu_hitsounding::Place::SliderNode { node: 0, .. })).unwrap();
    assert_eq!(head.volume(), 60);
    assert_eq!(reopened.output_problems().len(), 1, "listed again on reopen");
    save_through_summary(&mut reopened, scratch.path("backups"));
    assert_eq!(std::fs::read(&map).unwrap(), saved);
    assert_eq!(std::fs::read(&project).unwrap(), json);
}

#[test]
fn a_recovery_copy_taken_after_a_partial_save_restores() {
    let scratch = Scratch::new("recovery-after-partial-save");
    let (mut app, _map) = open_left_out_fixture(&scratch, true);
    app.data_root = scratch.path("data");
    let drawn = tail_hits(&app);
    save_through_summary(&mut app, scratch.path("backups"));
    app.document.as_mut().unwrap().composer.apply("Later edit", |hits| hits.set_volume(dawsu_hitsounding::MomentId(0), 33));
    app.poll();
    let offer = app.write_recovery_copy().unwrap().unwrap();
    let mut restored = Application::create(Preferences::default(), false, None, None, false, scratch.path("data"));
    restored.project_root = app.project_root.clone();
    restored.restore_recovery(&offer.id).unwrap();
    wait(&mut restored);
    assert!(restored.error.is_none(), "{:?}", restored.error);
    let hits = restored.document().unwrap().composer.hitsounds();
    assert_eq!(hits.moment(dawsu_hitsounding::MomentId(0)).volume(), 33);
    assert_eq!(tail_hits(&restored), drawn);
    assert!(restored.is_dirty());
    let doc = restored.document().unwrap();
    assert!(!doc.written.moment(tail_id(&doc.written)).is_changed(), "what was last written comes from the copy's last Save");
}

/// Saves with the tail left out, edits on, and restores a recovery copy taken then.
fn restore_after_partial_save(scratch: &Scratch) -> (Application, Vec<(SampleIdentity, i32)>) {
    let (mut app, _map) = open_left_out_fixture(scratch, true);
    app.data_root = scratch.path("data");
    let drawn = tail_hits(&app);
    save_through_summary(&mut app, scratch.path("backups"));
    app.document.as_mut().unwrap().composer.apply("Later edit", |hits| hits.set_volume(dawsu_hitsounding::MomentId(0), 33));
    app.poll();
    let offer = app.write_recovery_copy().unwrap().unwrap();
    let mut restored = Application::create(Preferences::default(), false, None, None, false, scratch.path("data"));
    restored.project_root = app.project_root.clone();
    restored.restore_recovery(&offer.id).unwrap();
    wait(&mut restored);
    assert!(restored.error.is_none(), "{:?}", restored.error);
    (restored, drawn)
}

#[test]
fn undo_then_save_after_restoring_a_recovery_copy_keeps_the_left_out_hits() {
    let scratch = Scratch::new("recovery-undo-left-out");
    let (mut restored, drawn) = restore_after_partial_save(&scratch);
    restored.command("undo");
    assert_eq!(tail_hits(&restored), drawn, "Undo returns to the hits as drawn at the last Save");
    assert!(!restored.is_dirty());
    save_through_summary(&mut restored, scratch.path("backups"));
    assert_eq!(record_json(&restored)["current"]["moments"].as_array().map(Vec::len), Some(1), "the left-out tail is still kept");
    let map = restored.document().unwrap().path.clone();
    assert_eq!(tail_hits(&reopen(&restored, &map)), drawn);
}

#[test]
fn a_restored_recovery_copy_lists_left_out_moments_straight_away() {
    let scratch = Scratch::new("recovery-lists-left-out");
    let (restored, _) = restore_after_partial_save(&scratch);
    assert_eq!(restored.output_problems().iter().map(|p| p.time).collect::<Vec<_>>(), [2500.0]);
}

#[test]
fn an_old_recovery_copy_without_drawn_hits_still_restores() {
    let scratch = Scratch::new("recovery-old-format");
    let (mut app, _map) = open_left_out_fixture(&scratch, true);
    app.data_root = scratch.path("data");
    save_through_summary(&mut app, scratch.path("backups"));
    app.document.as_mut().unwrap().composer.apply("Later edit", |hits| hits.set_volume(dawsu_hitsounding::MomentId(0), 33));
    app.poll();
    let offer = app.write_recovery_copy().unwrap().unwrap();
    // A copy an earlier build wrote has no drawn hits for its last Save.
    let path = scratch.path("data").join("recovery").join(&offer.id);
    let mut copy: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    copy["checkpoint"]["last_save"].as_object_mut().unwrap().remove("drawn");
    std::fs::write(&path, serde_json::to_vec(&copy).unwrap()).unwrap();
    let mut restored = Application::create(Preferences::default(), false, None, None, false, scratch.path("data"));
    restored.project_root = app.project_root.clone();
    restored.restore_recovery(&offer.id).unwrap();
    wait(&mut restored);
    assert!(restored.error.is_none(), "{:?}", restored.error);
    assert_eq!(restored.document().unwrap().composer.hitsounds().moment(dawsu_hitsounding::MomentId(0)).volume(), 33);
}

#[test]
fn a_recovery_copy_preserves_per_hit_volumes() {
    let scratch = Scratch::new("recovery-per-hit-volumes");
    let (mut app, _map) = open_left_out_fixture(&scratch, true);
    app.data_root = scratch.path("data");
    let clap = skin(SampleSet::Normal, Hitsound::Clap);
    app.document.as_mut().unwrap().composer.apply("Quiet clap", |hits| {
        let m = dawsu_hitsounding::MomentId(0);
        hits.set_volume(m, 100);
        hits.add_hit(m, clap.clone());
        hits.set_hit_volume(m, &clap, 30);
    });
    app.poll();
    let offer = app.write_recovery_copy().unwrap().unwrap();
    let mut restored = Application::create(Preferences::default(), false, None, None, false, scratch.path("data"));
    restored.project_root = app.project_root.clone();
    restored.restore_recovery(&offer.id).unwrap();
    wait(&mut restored);
    assert!(restored.error.is_none(), "{:?}", restored.error);
    let moment = restored.document().unwrap().composer.hitsounds().moment(dawsu_hitsounding::MomentId(0));
    assert_eq!(moment.volume(), 100);
    assert_eq!(moment.hit_volume(&clap), 30);
}

#[test]
fn an_unrelated_edit_keeps_a_left_out_moment_listed_and_editing_it_drops_it() {
    let scratch = Scratch::new("left-out-listing");
    let (mut app, _map) = open_left_out_fixture(&scratch, true);
    save_through_summary(&mut app, scratch.path("backups"));
    assert_eq!(app.output_problems().len(), 1);
    app.document.as_mut().unwrap().composer.apply("Unrelated", |hits| hits.set_volume(dawsu_hitsounding::MomentId(0), 30));
    assert_eq!(app.output_problems().len(), 1, "an unrelated edit keeps it listed");
    app.document.as_mut().unwrap().composer.apply("Edit the tail", |hits| {
        let tail = tail_id(hits);
        hits.set_hit_volume(tail, &skin(SampleSet::Drum, Hitsound::Clap), 20);
    });
    assert!(app.output_problems().is_empty(), "editing the moment drops it until the next Save");
    app.document.as_mut().unwrap().composer.undo();
    assert_eq!(app.output_problems().len(), 1, "undo brings back what was left out");
    app.set_hs_mode(false);
    assert!(app.output_problems().is_empty(), "the list belongs to the HS mode it was saved in");
}

#[test]
fn a_generate_review_keeps_the_left_out_moments_listed() {
    let scratch = Scratch::new("left-out-generate");
    let (mut app, _map) = open_left_out_fixture(&scratch, true);
    save_through_summary(&mut app, scratch.path("backups"));
    app.generate();
    wait(&mut app);
    assert!(app.output_problems().iter().any(|p| p.time == 2500.0), "the badge stays");
}

#[test]
fn an_hs_group_never_written_makes_no_circle_and_comes_back_as_loose_hits() {
    let scratch = Scratch::new("record-loose-group");
    let folder = scratch.path("mapset");
    std::fs::create_dir(&folder).unwrap();
    let map = folder.join("a.osu");
    let other = folder.join("b.osu");
    std::fs::write(&map, INDEXED_MAP).unwrap();
    std::fs::write(&other, INDEXED_MAP).unwrap();
    let mut app = Application::headless();
    app.project_root = Some(scratch.path("projects"));
    open(&mut app, map.clone());
    app.set_hs_mode(true);
    let composer = &mut app.document.as_mut().unwrap().composer;
    composer.add(3000.0, skin(SampleSet::Soft, Hitsound::Normal), 80, 0.0);
    composer.add(3000.0, skin(SampleSet::Soft, Hitsound::Clap), 80, 0.0);
    save_through_summary(&mut app, scratch.path("backups"));
    assert_eq!(std::fs::read(&map).unwrap(), INDEXED_MAP, "no 256,192 placeholder circle");
    assert_eq!(stored(&app).unanchored.map(|rows| rows.len()), Some(2));
    app.zoom = 0.5;
    app.save_session().unwrap();
    open(&mut app, other);
    open(&mut app, map.clone());
    let reopened = reopen(&app, &map);
    for app in [&app, &reopened] {
        let loose: Vec<_> = app.document().unwrap().composer.hitsounds().moments().iter()
            .filter(|m| m.place() == dawsu_hitsounding::Place::NoObject).collect();
        assert_eq!(loose.len(), 1);
        assert_eq!(loose[0].hits().count(), 2);
        assert_eq!(app.output_problems().iter().map(|p| p.time).collect::<Vec<_>>(), [3000.0]);
    }
}

#[test]
fn an_hs_group_written_before_and_left_out_later_keeps_one_circle() {
    let scratch = Scratch::new("record-written-group");
    let folder = scratch.path("mapset");
    std::fs::create_dir(&folder).unwrap();
    let map = folder.join("a.osu");
    std::fs::write(&map, INDEXED_MAP).unwrap();
    wav(&folder.join("voice.wav"), 8192);
    let mut app = Application::headless();
    app.project_root = Some(scratch.path("projects"));
    open(&mut app, map.clone());
    app.set_hs_mode(true);
    app.document.as_mut().unwrap().composer.add(3000.0, SampleIdentity::File("voice.wav".into()), 80, 0.0);
    save_through_summary(&mut app, scratch.path("backups"));
    let first = std::fs::read(&map).unwrap();
    assert_eq!(app.document().unwrap().file.hit_objects().len(), 2);
    app.document.as_mut().unwrap().composer.add(3000.0, skin(SampleSet::Soft, Hitsound::Clap), 80, 0.0);
    save_through_summary(&mut app, scratch.path("backups"));
    assert_eq!(std::fs::read(&map).unwrap(), first, "the circle keeps its last written hits");
    assert_eq!(record_json(&app)["current"]["moments"][0]["object"], 1);
    let mut reopened = reopen(&app, &map);
    let doc = reopened.document().unwrap();
    assert_eq!(doc.file.hit_objects().len(), 2);
    let circle = doc.composer.hitsounds().moments().iter().find(|m| m.time().ms() == 3000.0).unwrap();
    assert_eq!(circle.hits().count(), 2, "the drawn hits are back on the one circle");
    assert_eq!(project_hits(doc).len(), 0, "and not as loose hits too");
    save_through_summary(&mut reopened, scratch.path("backups"));
    assert_eq!(std::fs::read(&map).unwrap(), first);
}

#[test]
fn loose_hits_saved_outside_hs_mode_never_count_as_a_written_circle() {
    let scratch = Scratch::new("loose-never-a-circle");
    let folder = scratch.path("mapset");
    std::fs::create_dir(&folder).unwrap();
    let map = folder.join("a.osu");
    std::fs::write(&map, INDEXED_MAP).unwrap();
    wav(&folder.join("voice.wav"), 8192);
    let mut app = Application::headless();
    app.project_root = Some(scratch.path("projects"));
    open(&mut app, map.clone());
    app.document.as_mut().unwrap().composer.add(3000.0, SampleIdentity::File("voice.wav".into()), 80, 0.0);
    save_through_summary(&mut app, scratch.path("backups"));
    assert_eq!(std::fs::read(&map).unwrap(), INDEXED_MAP, "outside HS mode a loose hit stays in the project");
    app.set_hs_mode(true);
    app.document.as_mut().unwrap().composer.add(3000.0, skin(SampleSet::Soft, Hitsound::Clap), 80, 0.0);
    save_through_summary(&mut app, scratch.path("backups"));
    assert_eq!(std::fs::read(&map).unwrap(), INDEXED_MAP, "the group was never a circle, so none is made");
    assert_eq!(app.output_problems().len(), 1);
}

#[test]
fn cancel_or_show_in_the_summary_drops_the_pending_save() {
    let scratch = Scratch::new("summary-cancel");
    let (mut app, map) = open_left_out_fixture(&scratch, true);
    app.review_save();
    wait(&mut app);
    assert!(app.save_problem_review());
    assert!(app.save_review_files().is_some());
    app.dismiss_save_problem_review();
    assert!(!app.save_problem_review());
    assert!(app.save_review_files().is_none(), "Review Save must not pop up straight after");
    app.answer_save_review(true);
    wait(&mut app);
    assert_eq!(std::fs::read(&map).unwrap(), SLIDER_MAP);
    assert!(app.is_dirty());
}

#[test]
fn without_a_project_save_still_refuses_to_leave_moments_out() {
    let scratch = Scratch::new("no-project-refuses");
    let (mut app, map) = open_left_out_fixture(&scratch, false);
    app.save_with_backups(scratch.path("backups"));
    wait(&mut app);
    assert!(app.error.as_deref().is_some_and(|e| e.contains("cannot be written")), "{:?}", app.error);
    assert_eq!(std::fs::read(&map).unwrap(), SLIDER_MAP);
    assert!(app.is_dirty());
}

#[test]
fn a_pending_open_save_shows_the_summary_first_and_opens_after_its_save() {
    let scratch = Scratch::new("pending-open-summary");
    let (mut app, map) = open_left_out_fixture(&scratch, true);
    let other = map.with_file_name("b.osu");
    std::fs::write(&other, SLIDER_MAP).unwrap();
    app.request_open(other.clone());
    assert!(app.pending_open.is_some());
    app.save();
    wait(&mut app);
    assert!(app.error.is_none() && app.is_dirty() && app.save_problem_review(), "{:?}", app.error);
    assert_eq!(std::fs::read(&map).unwrap(), SLIDER_MAP, "nothing is written before the summary's Save");
    assert!(app.pending_open.is_some());
    app.answer_save_review(true);
    wait(&mut app);
    assert!(app.error.is_none(), "{:?}", app.error);
    assert_eq!(app.document().unwrap().path, other.canonicalize().unwrap(), "the pending difficulty opens");
}

#[test]
fn editing_a_left_out_hs_group_drops_it_from_the_list_and_undo_brings_it_back() {
    let scratch = Scratch::new("left-out-group-listing");
    let folder = scratch.path("mapset");
    std::fs::create_dir(&folder).unwrap();
    let map = folder.join("a.osu");
    std::fs::write(&map, INDEXED_MAP).unwrap();
    let mut app = Application::headless();
    app.project_root = Some(scratch.path("projects"));
    open(&mut app, map.clone());
    app.set_hs_mode(true);
    let composer = &mut app.document.as_mut().unwrap().composer;
    composer.add(3000.0, skin(SampleSet::Soft, Hitsound::Normal), 80, 0.0);
    composer.add(3000.0, skin(SampleSet::Soft, Hitsound::Clap), 80, 0.0);
    save_through_summary(&mut app, scratch.path("backups"));
    assert_eq!(app.output_problems().iter().map(|p| p.time).collect::<Vec<_>>(), [3000.0]);
    app.document.as_mut().unwrap().composer.add(3000.0, skin(SampleSet::Soft, Hitsound::Finish), 80, 0.0);
    assert!(app.output_problems().is_empty(), "the group no longer holds what Save left out");
    app.document.as_mut().unwrap().composer.undo();
    assert_eq!(app.output_problems().iter().map(|p| p.time).collect::<Vec<_>>(), [3000.0]);
}

#[test]
fn a_left_out_tail_beside_a_written_head_green_line_comes_back_on_reopen() {
    let scratch = Scratch::new("left-out-tail-green-line");
    let folder = scratch.path("mapset");
    std::fs::create_dir(&folder).unwrap();
    let map = folder.join("a.osu");
    std::fs::write(&map, SLIDER_MAP).unwrap();
    let mut app = Application::headless();
    app.project_root = Some(scratch.path("projects"));
    open(&mut app, map.clone());
    app.document.as_mut().unwrap().composer.apply("Head 60, empty tail", |hits| {
        let head = hits.moments().iter().position(|m| matches!(m.place(), dawsu_hitsounding::Place::SliderNode { node: 0, .. })).unwrap();
        hits.set_volume(dawsu_hitsounding::MomentId(head), 60);
        let tail = tail_id(hits);
        let lanes: Vec<_> = hits.moment(tail).hits().cloned().collect();
        for lane in lanes {
            hits.remove_hit(tail, &lane);
        }
    });
    let drawn = tail_hits(&app);
    save_through_summary(&mut app, scratch.path("backups"));
    let saved = std::fs::read(&map).unwrap();
    assert_ne!(saved, SLIDER_MAP, "the head's green line is written");
    assert_eq!(app.output_problems().iter().map(|p| p.time).collect::<Vec<_>>(), [2500.0]);
    let reopened = reopen(&app, &map);
    assert!(!reopened.warnings.iter().any(|w| w.contains("left out")), "{:?}", reopened.warnings);
    assert_eq!(tail_hits(&reopened), drawn);
    assert_eq!(reopened.output_problems().iter().map(|p| p.time).collect::<Vec<_>>(), [2500.0]);
}

#[test]
fn an_hs_circle_made_before_a_left_out_tail_shifts_its_record_ordinal_and_reopen_matches() {
    let scratch = Scratch::new("left-out-ordinal-shift");
    let (mut app, map) = open_left_out_fixture(&scratch, true);
    app.document.as_mut().unwrap().composer.add(500.0, skin(SampleSet::Normal, Hitsound::Normal), 80, 0.0);
    let drawn = tail_hits(&app);
    save_through_summary(&mut app, scratch.path("backups"));
    assert_eq!(app.document().unwrap().file.hit_objects().len(), 3);
    assert_eq!(record_json(&app)["current"]["moments"][0]["object"], 2);
    let mut reopened = reopen(&app, &map);
    assert!(!reopened.warnings.iter().any(|w| w.contains("left out")), "{:?}", reopened.warnings);
    assert_eq!(tail_hits(&reopened), drawn);
    assert_eq!(reopened.output_problems().iter().map(|p| p.time).collect::<Vec<_>>(), [2500.0]);
    let before = std::fs::read(&map).unwrap();
    save_through_summary(&mut reopened, scratch.path("backups"));
    assert_eq!(std::fs::read(&map).unwrap(), before);
}

#[test]
fn an_emptied_circle_is_left_out_and_comes_back_empty_on_reopen() {
    let scratch = Scratch::new("left-out-empty-circle");
    let folder = scratch.path("mapset");
    std::fs::create_dir(&folder).unwrap();
    let map = folder.join("a.osu");
    std::fs::write(&map, SLIDER_MAP).unwrap();
    let mut app = Application::headless();
    app.project_root = Some(scratch.path("projects"));
    open(&mut app, map.clone());
    app.document.as_mut().unwrap().composer.apply("Empty the circle", |hits| {
        let lanes: Vec<_> = hits.moment(dawsu_hitsounding::MomentId(0)).hits().cloned().collect();
        for lane in lanes {
            hits.remove_hit(dawsu_hitsounding::MomentId(0), &lane);
        }
    });
    save_through_summary(&mut app, scratch.path("backups"));
    assert_eq!(std::fs::read(&map).unwrap(), SLIDER_MAP);
    assert_eq!(app.output_problems().len(), 1);
    let reopened = reopen(&app, &map);
    assert!(!reopened.warnings.iter().any(|w| w.contains("left out")), "{:?}", reopened.warnings);
    assert_eq!(reopened.document().unwrap().composer.hitsounds().moment(dawsu_hitsounding::MomentId(0)).hits().count(), 0);
    assert_eq!(reopened.output_problems().len(), 1);
}

#[test]
fn save_and_save_check_publish_object_progress() {
    let fixture = Scratch::new("save-progress");
    let original = b"osu file format v14\n[General]\nMode:0\n[HitObjects]\n256,192,1000,1,0,0:0:0:50:\n256,192,2000,1,0,0:0:0:60:\n";
    for check_only in [true, false] {
        let path = fixture.path(if check_only { "checked.osu" } else { "saved.osu" });
        std::fs::write(&path, original).unwrap();
        let mut app = Application::headless();
        open(&mut app, path.clone());
        app.document.as_mut().unwrap().composer.apply("Change both objects", |hits| {
            hits.set_volume(dawsu_hitsounding::MomentId(0), 75);
            hits.set_volume(dawsu_hitsounding::MomentId(1), 80);
        });
        if check_only {
            app.review_save();
        } else {
            app.save_with_backups(fixture.path("backups"));
        }
        assert!(app.busy);
        let progress = app.job_progress.as_ref().expect("Save should track object progress");
        let result = app.jobs.as_ref().unwrap().recv_timeout(Duration::from_secs(3)).unwrap();
        assert!(matches!(result, JobResult::SaveCheck { result: Ok(_), .. } | JobResult::Save { result: Ok(_), .. }));
        assert_eq!(*progress.current.lock().unwrap(), Some((2, 2)));
        assert_eq!(std::fs::read(path).unwrap() == original, check_only);
    }
}

#[test]
fn review_progress_reaches_status_while_worker_is_active() {
    let fixture = Scratch::new("review-progress");
    let path = fixture.path("map.osu");
    std::fs::write(&path, b"osu file format v14\n[General]\nMode:0\n[HitObjects]\n").unwrap();
    let mut app = Application::headless();
    open(&mut app, path);
    let (release, wait) = mpsc::channel();
    let (reported, ready) = mpsc::channel();
    app.review_job(move |_, _, _, progress| {
        progress(2, 5);
        reported.send(()).unwrap();
        wait.recv().unwrap();
        Err("test complete".into())
    });
    ready.recv_timeout(Duration::from_secs(3)).unwrap();
    app.poll();
    assert!(app.busy);
    assert!(app.status.contains("2 / 5"), "{}", app.status);
    release.send(()).unwrap();
}

#[test]
fn regenerate_checks_owned_output_reuses_mix_and_backs_up_replacement() {
    let fixture = Scratch::new("checked-regenerate");
    let source = fixture.path("source.osu");
    let projects = fixture.path("sessions");
    let backups = fixture.path("backups");
    let body = b"osu file format v14\n[General]\nMode:0\n[Metadata]\nArtist:Artist\nTitle:Song\nCreator:Mapper\nVersion:Hard\n[TimingPoints]\n0,500,4,1,0,100,1,0\n[HitObjects]\n256,192,1000,1,0,0:0:0:50:one.wav\n300,192,1000,1,0,0:0:0:80:two.wav\n";
    std::fs::write(&source, body).unwrap();
    for (name, amplitude) in [("one.wav", 8192_i16), ("two.wav", 16384)] {
        let mut wav = b"RIFF\x26\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data\x02\0\0\0".to_vec();
        wav.extend(amplitude.to_le_bytes());
        std::fs::write(fixture.path(name), wav).unwrap();
    }
    let wait = |app: &mut Application| {
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.busy {
            app.poll();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    let mut app = Application::headless();
    app.project_root = Some(projects.clone());
    open(&mut app, source.clone());
    app.generate();
    wait(&mut app);
    let review = app.output_review.as_ref().unwrap();
    let target = review.files[0].path.clone();
    let mix = review.mix_paths().next().unwrap().to_path_buf();
    assert!(!target.exists() && !mix.exists());
    app.apply_output_in(backups.clone());
    wait(&mut app);
    assert!(app.error.is_none());
    let initial = std::fs::read(&target).unwrap();
    let rendered = std::fs::read(&mix).unwrap();
    let mut app = Application::headless();
    app.project_root = Some(projects);
    open(&mut app, source.clone());
    app.document.as_mut().unwrap().composer.add(1500.0, SampleIdentity::File("one.wav".into()), 60, 1.0);
    app.generate();
    wait(&mut app);
    let replacement = app.output_review.as_ref().unwrap_or_else(|| panic!("{:?}", app.error));
    assert_eq!(replacement.files[0].expected.as_deref(), Some(initial.as_slice()));
    assert_eq!(replacement.mix_paths().collect::<Vec<_>>(), [mix.as_path()]);
    assert_eq!(std::fs::read(&target).unwrap(), initial);
    app.apply_output_in(backups.clone());
    wait(&mut app);
    assert!(app.error.is_none(), "{:?}", app.error);
    assert_ne!(std::fs::read(&target).unwrap(), initial);
    assert_eq!(std::fs::read(&mix).unwrap(), rendered);
    let backup_dir = std::fs::read_dir(&backups).unwrap().next().unwrap().unwrap().path();
    let backup = std::fs::read_dir(backup_dir).unwrap().next().unwrap().unwrap().path();
    assert_eq!(std::fs::read(backup).unwrap(), initial);
    std::fs::write(&target, b"Another editor changed the output").unwrap();
    app.generate();
    wait(&mut app);
    assert!(app.output_review.is_none());
    assert!(app.error.as_deref().unwrap().contains("changed since Generate"));
    assert_eq!(std::fs::read(&target).unwrap(), b"Another editor changed the output");
    assert_eq!(std::fs::read(&source).unwrap(), body);
}

#[test]
fn generate_uses_legacy_rule_to_pair_incompatible_roles_after_reopen() {
    let fixture = Scratch::new("generate-paired-preset");
    let map = fixture.path("Artist - Song (Mapper) [Hard].osu");
    let body = b"osu file format v14\n[General]\nMode:0\n[Metadata]\nVersion:Hard\n[TimingPoints]\n0,500,4,1,0,100,1,0\n[HitObjects]\n256,192,1000,1,0,1:0:2:50:\n";
    std::fs::write(&map, body).unwrap();
    let mut layers = Vec::new();
    for (name, amplitude) in [("normal-hitnormal2.wav", 8192_i16), ("soft-hitclap3.wav", 4096), ("drum-hitfinish4.wav", 16384)] {
        let mut wav = b"RIFF\x26\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data\x02\0\0\0".to_vec();
        wav.extend(amplitude.to_le_bytes());
        std::fs::write(fixture.path(name), &wav).unwrap();
        let hash = dawsu_foundation::digest::sha256_hex(&wav).to_ascii_uppercase();
        layers.push(format!("hit:BeatmapFile:{name}@{hash}@balance:1"));
    }
    layers.sort();
    let key = serde_json::to_string(&layers).unwrap();
    let mut settings = ExportSettings { pair_samples: true, ..ExportSettings::default() };
    settings.rules.insert(key, "soft-hitwhistle".into());
    let path = fixture.path("merge-preset.json");
    let export = serde_json::to_vec_pretty(&settings).unwrap();
    std::fs::write(&path, &export).unwrap();
    let loaded = ExportSettings::read_file(&path).unwrap();
    assert_eq!(loaded, settings);
    let mut app = Application::headless();
    app.project_root = Some(fixture.path("projects"));
    open(&mut app, map.clone());
    app.set_export_settings(loaded).unwrap();
    app.save_session().unwrap();
    let mut app = Application::headless();
    app.project_root = Some(fixture.path("projects"));
    open(&mut app, map.clone());
    assert_eq!(app.document().unwrap().export_settings(), Some(&settings));
    let d = app.document.as_mut().unwrap();
    for (set, sound, index) in [(SampleSet::Soft, Hitsound::Clap, 3), (SampleSet::Drum, Hitsound::Finish, 4)] {
        d.composer.add(1000.0, SampleIdentity::Standard { set, sound: Sound::Hit(sound), index: dawsu_foundation::CustomIndex::new(index) }, 50, 1.0);
    }
    app.generate();
    let deadline = Instant::now() + Duration::from_secs(10);
    while app.busy {
        app.poll();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    let plan = app.output_review.as_ref().unwrap_or_else(|| panic!("{:?}", app.error));
    let names = plan.mix_paths().map(|path| path.file_name().unwrap().to_str().unwrap().to_owned()).collect::<Vec<_>>();
    assert_eq!(names, ["normal-hitnormal5.wav", "soft-hitfinish5.wav", "soft-hitclap5.wav"]);
    assert!(plan.mix_paths().all(|path| !path.exists()));
    assert!(!plan.files[0].path.exists());
    let target = plan.files[0].path.clone();
    app.apply_output_in(fixture.path("backups"));
    while app.busy {
        app.poll();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(app.error.is_none(), "{:?}", app.error);
    let reopened = Document::read(&target).unwrap();
    let moment = &reopened.composer.hitsounds().moments()[0];
    assert_eq!(moment.volume(), 100);
    assert_eq!(moment.hits().count(), 3);
    for (file, amplitude) in names.iter().zip([0.125, 0.25, 0.0625]) {
        assert_eq!(dawsu_audio::decode(&fixture.path(file)).unwrap().samples, [amplitude; 2]);
    }
    app.generate();
    while app.busy {
        app.poll();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    let replacement = app.output_review.as_ref().unwrap_or_else(|| panic!("{:?}", app.error));
    assert_eq!(replacement.mix_paths().map(|path| path.file_name().unwrap().to_str().unwrap()).collect::<Vec<_>>(), names);
    assert_eq!(std::fs::read(&map).unwrap(), body);
    assert_eq!(std::fs::read(&path).unwrap(), export);
}

#[test]
fn bookmark_commands_navigate_imported_and_new_bookmarks_per_difficulty() {
    let fixture = Scratch::new("bookmark-commands");
    let first = fixture.path("first.osu");
    let second = fixture.path("second.osu");
    let map = include_str!("../../tests/project-fixture.osu");
    let original = map.replace("[Metadata]", "[Editor]\nBookmarks: 100,500\n[Metadata]");
    std::fs::write(&first, &original).unwrap();
    std::fs::write(&second, map).unwrap();
    let mut app = Application::headless();
    app.project_root = Some(fixture.path("projects"));
    open(&mut app, first.clone());

    app.seek(0.0);
    app.command("nextBookmark");
    assert_eq!(app.position_ms(), 100.0, "next bookmark should navigate to the first map bookmark");
    // The Inspector's pick follows the step: the editor names the bookmark it jumped to, once.
    assert_eq!(app.bookmark_step.take(), Some(100.0));
    app.seek(1500.0);
    app.command("nextBookmark");
    assert_eq!(app.bookmark_step, None, "a step with no bookmark to go to names none");
    app.seek(750.0);
    app.command("addBookmark");
    app.command("previousBookmark");
    assert_eq!(app.position_ms(), 500.0);
    app.command("nextBookmark");
    assert_eq!(app.position_ms(), 750.0);
    app.rename_bookmark(750.0, "Chorus".into()).unwrap();
    app.seek(1250.0);
    app.add_bookmark(Some("Break".into())).unwrap();
    open(&mut app, second.clone());
    app.seek(0.0);
    app.command("nextBookmark");
    assert_eq!(app.position_ms(), 0.0);
    open(&mut app, first.clone());
    let marks = app.document().unwrap().bookmarks();
    assert_eq!(marks[2].label.as_deref(), Some("Chorus"));
    assert_eq!(marks[3].label.as_deref(), Some("Break"));
    app.seek(500.0);
    app.command("nextBookmark");
    assert_eq!(app.position_ms(), 750.0);
    app.rename_bookmark(750.0, " ".into()).unwrap();
    assert_eq!(app.document().unwrap().bookmarks()[2].label, None);
    app.command("removeBookmark");
    for time in [100.0, 500.0, 1250.0] {
        app.remove_bookmark(time).unwrap();
    }
    open(&mut app, first.clone());
    assert!(app.document().unwrap().bookmarks().is_empty(), "the saved empty list overrides imported marks");
    assert_eq!(std::fs::read(&first).unwrap(), original.as_bytes());
    assert_eq!(std::fs::read(&second).unwrap(), map.as_bytes());
}

#[test]
fn add_lane_registers_a_new_sample_before_the_watcher_runs() {
    let fixture = Scratch::new("add-lane-immediate");
    let path = fixture.path("first.osu");
    let map = b"osu file format v14\n[General]\nAudioFilename: song.wav\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n";
    let wav = b"RIFF\x26\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data\x02\0\0\0\0\x40";
    std::fs::write(&path, map).unwrap();
    let mut app = Application::headless();
    open(&mut app, path.clone());
    let path = path.canonicalize().unwrap();
    let lane = SampleIdentity::File("late.WAV".into());
    std::fs::write(fixture.path("late.WAV"), wav).unwrap();
    std::fs::write(fixture.path("song.wav"), wav).unwrap();
    assert!(!app.document().unwrap().composer.lanes.contains(&lane));
    assert!(app.scan_lane_add_candidates(&path).unwrap().contains(&lane));
    // No poll occurs between copy and explicit Add.
    app.show_lane(&path, &lane).unwrap();
    let doc = app.document().unwrap();
    assert_eq!(doc.composer.lanes[app.active_lane], lane);
    assert!(app.samples.contains_key(&lane));
    assert!(!app.is_dirty());
    assert!(!app.scan_lane_add_candidates(&path).unwrap().contains(&lane));
    app.command("undo");
    assert!(app.document().unwrap().composer.hitsounds().is_lane_hidden(&lane));
    assert!(app.scan_lane_add_candidates(&path).unwrap().contains(&lane));
    app.command("redo");
    assert!(!app.document().unwrap().composer.hitsounds().is_lane_hidden(&lane));
    assert_eq!(std::fs::read(&path).unwrap(), map);
    assert_eq!(std::fs::read(fixture.path("late.WAV")).unwrap(), wav);
}

#[test]
fn fresh_gameplay_diff_opens_with_objects_snap_and_used_lanes_only_and_persists_overrides() {
    let fixture = Scratch::new("task1-path3-defaults");
    let path = fixture.path("map.osu");
    let map = b"osu file format v14\n[TimingPoints]\n0,500,4,1,0,80,1,0\n[HitObjects]\n256,192,1000,1,8,0:0:0:0:\n";
    std::fs::write(&path, map).unwrap();
    let projects = fixture.path("projects");
    let mut app = Application::headless();
    app.project_root = Some(projects);
    open(&mut app, path.clone());

    let doc = app.document().unwrap();
    assert_eq!(doc.composer.snap_mode, dawsu_editor::tempo::SnapMode::Objects);
    let visible: Vec<_> = doc
        .composer
        .hitsounds()
        .lanes()
        .iter()
        .filter(|l| !doc.composer.hitsounds().is_lane_hidden(&l.identity))
        .map(|l| crate::lane_name(&l.identity))
        .collect();
    assert_eq!(visible, vec!["normal-hitnormal", "normal-hitclap"]);

    // Switch to Grid snap and unhide every lane, then save session and reopen:
    // both Grid snap and the empty hidden_lanes list must persist.
    let doc = app.document.as_mut().unwrap();
    doc.composer.snap_mode = dawsu_editor::tempo::SnapMode::Grid;
    doc.composer.history.update_external(|hits| {
        hits.restore_hidden_lanes(std::iter::empty());
    });
    app.save_session().unwrap();

    open(&mut app, path.clone());
    let doc = app.document().unwrap();
    assert_eq!(doc.composer.snap_mode, dawsu_editor::tempo::SnapMode::Grid);
    assert_eq!(doc.composer.hitsounds().hidden_lanes().count(), 0);
}

#[test]
fn live_reload_preserves_unsaved_snap_mode_and_unhidden_lanes() {
    let fixture = Scratch::new("task1-live-reload-preserve");
    let path = fixture.path("map.osu");
    let map_v1 = b"osu file format v14\n[TimingPoints]\n0,500,4,1,0,80,1,0\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n";
    let map_v2 = b"osu file format v14\n[TimingPoints]\n0,500,4,1,0,80,1,0\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n256,192,2000,1,0,0:0:0:0:\n";
    std::fs::write(&path, map_v1).unwrap();
    let projects = fixture.path("projects");
    let mut app = Application::headless();
    app.project_root = Some(projects);
    open(&mut app, path.clone());

    // Change snap_mode to Grid and unhide all lanes without saving the session first.
    {
        let doc = app.document.as_mut().unwrap();
        doc.composer.snap_mode = dawsu_editor::tempo::SnapMode::Grid;
        doc.composer.snap_enabled = false;
        doc.composer.history.update_external(|hits| {
            hits.restore_hidden_lanes(std::iter::empty());
        });
    }

    // External tool updates .osu on disk; trigger a live reload.
    std::fs::write(&path, map_v2).unwrap();
    app.reload_from_disk();
    wait(&mut app);

    let doc = app.document().unwrap();
    assert_eq!(doc.composer.hitsounds().moments().len(), 2);
    assert_eq!(doc.composer.snap_mode, dawsu_editor::tempo::SnapMode::Grid);
    assert!(!doc.composer.snap_enabled);
    assert_eq!(doc.composer.hitsounds().hidden_lanes().count(), 0);
}

#[test]
fn deleting_a_lane_preserves_its_file_and_supports_undo_reopen_and_add() {
    let fixture = Scratch::new("delete-lane");
    let path = fixture.path("first.osu");
    let other = fixture.path("second.osu");
    let sample = fixture.path("soft-hitclap.wav");
    let original = b"osu file format v14\r\n[General]\r\nMode:0\r\n[TimingPoints]\r\n0,500,4,2,1,75,1,0\r\n[HitObjects]\r\n256,192,1000,1,8,0:0:0:75:\r\n256,192,2000,1,0,0:0:0:50:\r\n";
    let wav = b"RIFF\x26\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data\x02\0\0\0\0\x40";
    std::fs::write(&sample, wav).unwrap();
    std::fs::write(&path, original).unwrap();
    std::fs::write(&other, original).unwrap();
    let mut app = Application::headless();
    app.project_root = Some(fixture.path("projects"));
    open(&mut app, path.clone());
    let path = path.canonicalize().unwrap();
    let lane = dawsu_hitsounding::rules::read_sample_file_name("soft-hitclap.wav").unwrap();
    let protected = app.document().unwrap().composer.hitsounds().moments()[1].hits().next().unwrap().clone();
    app.set_normal_protection(true);
    let revision = app.document().unwrap().composer.history.version();
    assert!(app.delete_lane(&path, &protected, revision).is_err());
    assert!(app.delete_lane(&path, &lane, revision + 1).is_err());
    assert!(!app.is_dirty());

    app.active_lane = app.document().unwrap().composer.lanes.iter().position(|row| row == &lane).unwrap();
    let hidden = |app: &Application| app.document().unwrap().composer.hitsounds().is_lane_hidden(&lane);
    let has_hit = |app: &Application| app.document().unwrap().composer.hitsounds().moments()[0].has_hit(&lane);
    app.delete_lane(&path, &lane, revision).unwrap();
    assert!(hidden(&app));
    assert!(!has_hit(&app));
    assert_ne!(app.document().unwrap().composer.lanes[app.active_lane], lane);
    assert_eq!(std::fs::read(&path).unwrap(), original);
    // Simulates a watcher payload that arrives after Delete, including after a later rescan.
    app.register_added_samples(vec![("soft-hitclap.wav".into(), dawsu_audio::decode(&sample).unwrap())]);
    assert!(hidden(&app));
    app.command("undo");
    assert!(!hidden(&app) && has_hit(&app));
    app.command("redo");
    assert!(hidden(&app) && !has_hit(&app));

    app.save_with_backups(fixture.path("backups"));
    let deadline = Instant::now() + Duration::from_secs(10);
    while app.busy {
        app.poll();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(app.error.is_none(), "{:?}", app.error);
    assert!(!app.is_dirty());
    open(&mut app, other.clone());
    assert!(!hidden(&app) && has_hit(&app));
    open(&mut app, path.clone());
    assert!(hidden(&app) && !has_hit(&app));
    assert!(app.lane_add_candidates().contains(&lane));
    app.show_lane(&path, &lane).unwrap();
    assert!(!hidden(&app) && !has_hit(&app));
    assert_eq!(app.document().unwrap().composer.lanes[app.active_lane], lane);
    assert!(!app.is_dirty(), "showing an empty lane never dirties the map");
    app.command("undo");
    assert!(hidden(&app));
    app.command("redo");
    assert!(!hidden(&app));
    assert_eq!(std::fs::read(sample).unwrap(), wav);
    assert_eq!(std::fs::read(other).unwrap(), original);
}

#[test]
fn lane_layout_is_read_per_difficulty_without_writing_the_map() {
    let fixture = Scratch::new("lane-layout");
    let path = fixture.path("first.osu");
    let other = fixture.path("second.osu");
    let original = include_bytes!("../../tests/project-fixture.osu");
    std::fs::write(&path, original).unwrap();
    std::fs::write(&other, original).unwrap();
    let projects = fixture.path("projects");
    let mut app = Application::headless();
    app.project_root = Some(projects.clone());
    let mut project = project::Project::load(&projects, None, &path.canonicalize().unwrap()).unwrap();
    let settings = project::SessionSettings {
        lane_layout: Some(project::LaneLayout {
            folder_order: vec!["drum".into(), "normal".into(), "soft".into()],
            collapsed_folders: ["normal".into()].into(),
            collapsed_families: ["soft-hitclap".into()].into(),
            lane_colours: Default::default(),
            ..Default::default()
        }),
        ..Default::default()
    };
    project.save(settings.clone()).unwrap();
    open(&mut app, path.clone());
    assert_eq!(app.document().unwrap().lane_layout(), settings.lane_layout.as_ref());
    let updated = LaneLayout {
        folder_order: vec!["soft".into(), "drum".into(), "normal".into()],
        collapsed_folders: ["drum".into()].into(),
        collapsed_families: ["normal-hitfinish".into()].into(),
        lane_colours: [("Skin|normal-hitclap".into(), "#e65420".into())].into(),
        lane_names: [("Skin|normal-hitclap".into(), "Backup snare".into())].into(),
        lane_order: vec!["Skin|normal-hitclap".into(), "Skin|normal-hitnormal".into()],
        plain_list: false,
    };
    app.set_lane_layout(updated.clone()).unwrap();
    app.save_session().unwrap();
    open(&mut app, other);
    assert!(app.document().unwrap().lane_layout().is_none());
    app.save_session().unwrap();
    open(&mut app, path.clone());
    assert_eq!(app.document().unwrap().lane_layout(), Some(&updated));
    assert_eq!(std::fs::read(path).unwrap().as_slice(), original);
}

#[test]
fn explicit_play_start_keeps_the_return_point_when_seeking() {
    let fixture = Scratch::new("play-start");
    let path = fixture.path("test.osu");
    std::fs::write(&path, include_bytes!("../../tests/project-fixture.osu")).unwrap();
    let mut app = Application::headless();
    app.document = Some(Document::read(&path).unwrap());
    app.seek(700.0);
    app.play_pause_from(Some(500.0));
    assert_eq!(app.position_ms(), 500.0);
    assert_eq!(app.return_position, 500.0);
    app.seek(850.0);
    assert_eq!(app.return_position, 500.0);
    app.play_pause_from(None);
    assert_eq!(app.return_position, 850.0);
}

#[test]
fn replacing_a_lane_keeps_the_slot_and_saves_a_playable_new_backing_sample() {
    let scratch = Scratch::new("lane-replacement");
    let folder = scratch.path("mapset");
    std::fs::create_dir(&folder).unwrap();
    let map = folder.join("test.osu");
    let sibling = folder.join("other.osu");
    let original_hit = "256,192,1000,1,0,1:1:1:67:";
    let unrelated_hit = "320,192,1250,1,8,2:2:0:91:";
    let source = include_str!("../../tests/project-fixture.osu")
        .replace("256,192,1000,1,8,0:0:0:0:", &format!("{original_hit}\n{unrelated_hit}"));
    let sibling_bytes = source.replace(original_hit, "256,192,1000,1,0,1:1:3:67:");
    std::fs::write(&map, &source).unwrap();
    std::fs::write(&sibling, &sibling_bytes).unwrap();
    let wav = b"RIFF\x26\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data\x02\0\0\0\x00\x40";
    let mut quieter = wav.to_vec();
    *quieter.last_mut().unwrap() = 0x20;
    let external = scratch.path("replacement.wav");
    let backing = folder.join("normal-hitnormal4.wav");
    std::fs::write(&external, &quieter).unwrap();
    for name in ["normal-hitnormal.wav", "normal-hitnormal2.wav"] {
        std::fs::write(folder.join(name), wav).unwrap();
    }

    let mut app = Application::headless();
    app.project_root = Some(scratch.path("projects"));
    open(&mut app, map.clone());
    let old = dawsu_hitsounding::rules::read_sample_file_name("normal-hitnormal.wav").unwrap();
    let new = dawsu_hitsounding::rules::read_sample_file_name("normal-hitnormal4.wav").unwrap();
    app.active_lane = app.document().unwrap().composer.lanes.iter().position(|lane| lane == &old).unwrap();
    let map_key = app.document().unwrap().path.clone();
    assert!(app.replace_lane_sample(&sibling, &old, &external).is_err());
    assert!(app.replace_lane_sample(&map_key, &new, &external).is_err());
    assert!(!backing.exists());
    app.replace_lane_sample(&map_key, &old, &external).unwrap();
    let composer = &app.document().unwrap().composer;
    assert_eq!(composer.lanes[app.active_lane], new);
    assert_eq!(composer.hitsounds().lane_slot(&new), Some(&old));
    assert!(composer.hitsounds().moments()[0].hits().any(|sample| sample == &new));
    assert_eq!(composer.hitsounds().moments()[0].volume(), 67);
    assert!(app.is_dirty());
    assert_eq!(app.clip_for_lane(&new).unwrap().samples[0], 0.25);
    // The new backing sample is a pending file until a Save plays it.
    assert!(!backing.exists());
    assert_eq!(app.document().unwrap().pending.get("normal-hitnormal4.wav").map(|file| std::fs::read(&file.path).unwrap()), Some(quieter.clone()));
    assert_eq!(std::fs::read(&map).unwrap(), source.as_bytes());

    app.command("undo");
    let composer = &app.document().unwrap().composer;
    assert_eq!(composer.lanes[app.active_lane], old);
    assert!(composer.hitsounds().moments()[0].hits().any(|sample| sample == &old));
    assert!(composer.hitsounds().lane_slot(&new).is_none());
    assert!(!app.is_dirty());
    app.command("redo");
    assert_eq!(app.document().unwrap().composer.lanes[app.active_lane], new);

    app.save_with_backups(scratch.path("backups"));
    let deadline = Instant::now() + Duration::from_secs(10);
    while app.busy {
        app.poll();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(app.error.is_none(), "{:?}", app.error);
    assert!(!app.is_dirty());
    let saved = std::fs::read(&map).unwrap();
    assert!(saved.windows(unrelated_hit.len()).any(|window| window == unrelated_hit.as_bytes()));
    assert_eq!(std::fs::read(&sibling).unwrap(), sibling_bytes.as_bytes());
    assert_eq!(std::fs::read(&external).unwrap(), quieter);
    assert_eq!(std::fs::read(folder.join("normal-hitnormal.wav")).unwrap(), wav);
    // The Save wrote the new backing sample because the difficulty plays it.
    assert_eq!(std::fs::read(&backing).unwrap(), quieter);


    let mut reopened = Application::headless();
    reopened.project_root = Some(scratch.path("projects"));
    open(&mut reopened, map);
    let hits = reopened.document().unwrap().composer.hitsounds();
    assert_eq!(hits.lane_slot(&new), Some(&old));
    assert!(hits.moments()[0].hits().any(|sample| sample == &new));
    assert_eq!(reopened.clip_for_lane(&new).unwrap().samples[0], 0.25);
}

#[test]
#[allow(clippy::disallowed_methods)]
fn add_numbered_layers_skips_reserved_indexes_and_preserves_mapset_files() {
    let scratch = Scratch::new("numbered-layers");
    let folder = scratch.path("mapset");
    let defaults = scratch.path("skin");
    for directory in [&folder, &defaults] {
        std::fs::create_dir(directory).unwrap();
    }
    let map = folder.join("test.osu");
    let sibling = folder.join("other.osu");
    let original = include_bytes!("../../tests/project-fixture.osu");
    let other = include_str!("../../tests/project-fixture.osu")
        .replace("256,192,1000,1,8,0:0:0:0:", "256,192,1000,1,8,1:2:3:50:");
    std::fs::write(&map, original).unwrap();
    std::fs::write(&sibling, &other).unwrap();
    let tone = b"RIFF\x26\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data\x02\0\0\0\0\x40";
    let occupied = folder.join("SOFT-HITCLAP2.wav");
    std::fs::write(&occupied, tone).unwrap();
    std::fs::write(folder.join("drum-hitfinish7.wav"), tone).unwrap();
    let default_file = defaults.join("soft-hitclap.wav");
    std::fs::write(&default_file, tone).unwrap();

    let mut app = Application::headless();
    app.preferences.default_samples = defaults;
    open(&mut app, map.clone());
    let family = SampleIdentity::Standard {
        set: SampleSet::Soft,
        sound: Sound::Hit(Hitsound::Clap),
        index: dawsu_foundation::CustomIndex::new(0),
    };
    app.active_lane = app.document().unwrap().composer.lanes.iter().position(|lane| lane == &family).unwrap();
    let before = dawsu_osu_format::list_folder(&folder).unwrap();
    app.command("addNumberedLayer");
    assert!(app.error.is_none(), "{:?}", app.error);
    // A new layer is a pending file: the mapset holds nothing new until a Save plays it.
    let first = folder.join("soft-hitclap8.wav");
    assert!(!first.exists());
    assert_eq!(app.document().unwrap().pending.get("soft-hitclap8.wav").map(|file| std::fs::read(&file.path).unwrap()), Some(tone.to_vec()));

    let mut quieter = tone.to_vec();
    *quieter.last_mut().unwrap() = 0x20;
    std::fs::write(&default_file, &quieter).unwrap();
    app.command("addNumberedLayer");
    assert!(app.error.is_none(), "{:?}", app.error);
    // The next layer takes the next index, though the first is not on disk.
    assert!(!folder.join("soft-hitclap9.wav").exists());
    assert!(app.document().unwrap().pending.get("soft-hitclap9.wav").is_some());
    assert_eq!(dawsu_osu_format::list_folder(&folder).unwrap(), before);
    assert_eq!(std::fs::read(&occupied).unwrap(), tone);
    assert_eq!(std::fs::read(&map).unwrap(), original);
    assert_eq!(std::fs::read_to_string(&sibling).unwrap(), other);
    assert!(!app.is_dirty());
    let first_lane = dawsu_hitsounding::rules::read_sample_file_name("soft-hitclap8.wav").unwrap();
    let second_lane = dawsu_hitsounding::rules::read_sample_file_name("soft-hitclap9.wav").unwrap();
    {
        let composer = &mut app.document.as_mut().unwrap().composer;
        composer.add(1000.0, first_lane.clone(), 100, 0.0);
        composer.add(1500.0, second_lane.clone(), 80, 0.0);
    }
    assert_eq!(app.clip_for_lane(&first_lane).unwrap().samples, [0.5]);
    assert_eq!(app.clip_for_lane(&second_lane).unwrap().samples, [0.25]);
    let cues = app.scheduled_sounds(app.document().unwrap());
    assert!(cues.iter().any(|c| c.at_ms == 1000.0 && c.clip.samples == [0.5]));
    assert!(cues.iter().any(|c| c.at_ms == 1500.0 && c.clip.samples == [0.25]));
}

#[test]
fn task2_path1_create_hitsound_diff_preserves_bytes_and_seeds_custom_only_project() {
    let scratch = Scratch::new("task2-path1-create");
    let folder = scratch.path("mapset");
    std::fs::create_dir_all(&folder).unwrap();
    let source_path = folder.join("Artist - Song (Mapper) [Hard].osu");
    let mut bytes = vec![0xEF, 0xBB, 0xBF]; // UTF-8 BOM
    bytes.extend_from_slice(
        concat!(
            "osu file format v14\r\n",
            "[General]\r\nMode: 0\r\n",
            "[Editor]\r\nBookmarks: 1000,2000\r\n",
            "[Metadata]\r\nTitle:Song\r\nArtist:Artist\r\nCreator:Mapper\r\nVersion:Hard\r\nBeatmapID:12345\r\n",
            "[TimingPoints]\r\n0,500,4,1,0,80,1,0\r\n",
            "[HitObjects]\r\n256,192,1000,1,0,0:0:0:0:\r\n",
        )
        .as_bytes(),
    );
    std::fs::write(&source_path, &bytes).unwrap();

    let mut app = Application::headless();
    app.project_root = Some(scratch.path("projects"));
    let created = app
        .create_hitsound_diff(&source_path, false, Some(crate::HsStarterMode::CustomOnly))
        .unwrap();
    let created_bytes = std::fs::read(&created).unwrap();
    assert!(created_bytes.starts_with(&[0xEF, 0xBB, 0xBF]), "BOM must be preserved");
    let text = String::from_utf8_lossy(&created_bytes);
    assert!(text.contains("\r\n"), "CRLF must be preserved");
    assert!(text.contains("Bookmarks: 1000,2000"), "Bookmarks must be preserved");
    assert!(text.contains("Version:Hitsounds\r\n"), "Version must be Hitsounds");
    assert!(text.contains("BeatmapID:0\r\n"), "BeatmapID must be reset to 0");

    open(&mut app, created.clone());
    let doc = app.document().unwrap();
    assert!(doc.hs_mode);
    assert_eq!(doc.starter_mode, Some(crate::HsStarterMode::CustomOnly));
    let visible_count = doc
        .composer
        .hitsounds()
        .lanes()
        .iter()
        .filter(|l| !doc.composer.hitsounds().is_lane_hidden(&l.identity))
        .count();
    assert_eq!(visible_count, 0, "Path 1 must open with 0 visible lanes");
    assert!(doc.lane_layout().is_some_and(|l| l.plain_list));
    assert_eq!(
        doc.export_choices().map(|c| c.addition_only_normal),
        Some(dawsu_hitsounding::index_packing::AdditionOnlyNormal::Silent)
    );

    // Calling reuse_existing=true returns the existing diff without overwriting its project.
    let reused = app
        .create_hitsound_diff(&source_path, true, Some(crate::HsStarterMode::Default12))
        .unwrap();
    assert_eq!(reused.canonicalize().unwrap(), created.canonicalize().unwrap());
    open(&mut app, reused);
    assert_eq!(
        app.document().unwrap().starter_mode,
        Some(crate::HsStarterMode::CustomOnly),
        "reuse_existing must not overwrite existing starter_mode"
    );
}

#[test]
#[allow(clippy::disallowed_methods)]
fn task2_add_lane_from_file_stages_safely_sanitizes_and_reuses_mapset_files() {
    let scratch = Scratch::new("task2-add-lane");
    let folder = scratch.path("mapset");
    std::fs::create_dir_all(&folder).unwrap();
    let map = folder.join("Artist - Song (Mapper) [Hard].osu");
    std::fs::write(
        &map,
        "osu file format v14\n[General]\nMode: 0\n[Metadata]\nTitle:Song\nArtist:Artist\nCreator:Mapper\nVersion:Hard\n[TimingPoints]\n0,500,4,1,0,80,1,0\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n",
    )
    .unwrap();
    let wav = b"RIFF\x26\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data\x02\0\0\0\x00\x40";
    let existing_in_map = folder.join("existing-kick.wav");
    std::fs::write(&existing_in_map, wav).unwrap();
    let ext_standard = scratch.path("normal-hitnormal.wav");
    let ext_comma = scratch.path("kick,1.wav");
    std::fs::write(&ext_standard, wav).unwrap();
    std::fs::write(&ext_comma, wav).unwrap();

    let mut app = Application::headless();
    app.project_root = Some(scratch.path("projects"));
    let created = app
        .create_hitsound_diff(&map, false, Some(crate::HsStarterMode::CustomOnly))
        .unwrap();
    open(&mut app, created.clone());
    let doc_path = app.document().unwrap().path.clone();
    let before_files = dawsu_osu_format::list_folder(&folder).unwrap();

    // Verify pipe `|` and `,` are both sanitized by create_custom_file_name (cross-platform).
    let (sanitized_pipe, _) =
        crate::layers::create_custom_file_name(app.document().unwrap(), "kick,pipe|1", "wav", wav)
            .unwrap();
    assert!(!sanitized_pipe.contains(',') && !sanitized_pipe.contains('|'));

    // External file named like a standard osu! sound must not hijack Standard index slot.
    let id1 = app.add_lane_from_file(&doc_path, &ext_standard).unwrap();
    assert!(matches!(id1, dawsu_foundation::SampleIdentity::File(_)));
    assert!(!app.is_dirty());
    assert_eq!(dawsu_osu_format::list_folder(&folder).unwrap(), before_files);

    // External file with `,` must be sanitized.
    let id2 = app.add_lane_from_file(&doc_path, &ext_comma).unwrap();
    let dawsu_foundation::SampleIdentity::File(ref name2) = id2 else {
        panic!("expected File lane");
    };
    assert!(!name2.contains(',') && !name2.contains('|'));
    assert!(!app.is_dirty());
    assert_eq!(dawsu_osu_format::list_folder(&folder).unwrap(), before_files);

    // Undo hides the added lane cleanly.
    app.command("undo");
    assert!(app.document().unwrap().composer.hitsounds().is_lane_hidden(&id2));

    // Adding a file already in the mapset unhides it with 0 duplicates.
    let id3 = app.add_lane_from_file(&doc_path, &existing_in_map).unwrap();
    assert_eq!(id3, dawsu_foundation::SampleIdentity::File("existing-kick.wav".into()));
    assert!(!app.document().unwrap().composer.hitsounds().is_lane_hidden(&id3));
    assert_eq!(dawsu_osu_format::list_folder(&folder).unwrap(), before_files);
}

#[test]
fn task2_show_default_skin_lanes_and_index1_upgrade_work() {
    let scratch = Scratch::new("task2-show-skin-index1");
    let folder = scratch.path("mapset");
    std::fs::create_dir_all(&folder).unwrap();
    let map = folder.join("Artist - Song (Mapper) [Hard].osu");
    std::fs::write(
        &map,
        "osu file format v14\n[General]\nMode: 0\n[Metadata]\nTitle:Song\nArtist:Artist\nCreator:Mapper\nVersion:Hard\n[TimingPoints]\n0,500,4,1,0,80,1,0\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n",
    )
    .unwrap();
    let wav = b"RIFF\x26\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data\x02\0\0\0\x00\x40";
    let ext = scratch.path("my-kick.wav");
    std::fs::write(&ext, wav).unwrap();

    let mut app = Application::headless();
    app.project_root = Some(scratch.path("projects"));
    let created = app
        .create_hitsound_diff(&map, false, Some(crate::HsStarterMode::CustomOnly))
        .unwrap();
    open(&mut app, created);
    let doc_path = app.document().unwrap().path.clone();

    let pos_before = app.document().unwrap().composer.history.position();
    let shown = app.show_default_skin_lanes(&doc_path).unwrap();
    assert_eq!(shown, 12);
    assert!(!app.is_dirty());
    assert_eq!(app.document().unwrap().composer.history.position(), pos_before + 1);
    // Calling again pushes no Undo step.
    assert_eq!(app.show_default_skin_lanes(&doc_path).unwrap(), 0);
    assert_eq!(app.document().unwrap().composer.history.position(), pos_before + 1);

    // Replacing Index-0 skin lane upgrades to Index 1 (`normal-hitnormal.wav`) when unreserved.
    let skin_normal = dawsu_foundation::SampleIdentity::Standard {
        set: dawsu_foundation::SampleSet::Normal,
        sound: dawsu_foundation::Sound::Hit(dawsu_foundation::Hitsound::Normal),
        index: dawsu_foundation::CustomIndex::new(0),
    };
    app.active_lane = app
        .document()
        .unwrap()
        .composer
        .lanes
        .iter()
        .position(|l| l == &skin_normal)
        .unwrap();
    app.replace_lane_sample(&doc_path, &skin_normal, &ext).unwrap();
    assert!(
        app.document().unwrap().pending.get("normal-hitnormal.wav").is_some(),
        "Replacing unreserved Index-0 skin lane should upgrade to Index 1"
    );
}

#[test]
fn task2_create_hitsound_diff_from_current_preserves_source_and_writes_hitsounds() {
    let scratch = Scratch::new("task2-from-current");
    let folder = scratch.path("mapset");
    std::fs::create_dir_all(&folder).unwrap();
    let map = folder.join("Artist - Song (Mapper) [Hard].osu");
    let original = concat!(
        "osu file format v14\r\n",
        "[General]\r\nMode: 0\r\n",
        "[Metadata]\r\nTitle:Song\r\nArtist:Artist\r\nCreator:Mapper\r\nVersion:Hard\r\n",
        "[TimingPoints]\r\n0,500,4,1,0,80,1,0\r\n",
        "[HitObjects]\r\n256,192,1000,1,8,0:0:0:0:\r\n",
    );
    std::fs::write(&map, original).unwrap();

    let mut app = Application::headless();
    app.project_root = Some(scratch.path("projects"));
    open(&mut app, map.clone());
    let created = app.create_hitsound_diff_from_current().unwrap();
    assert_eq!(std::fs::read(&map).unwrap(), original.as_bytes(), "Source diff must remain byte-identical");
    assert!(created.exists());
    open(&mut app, created);
    let doc = app.document().unwrap();
    assert!(doc.hs_mode);
    assert_eq!(doc.composer.hitsounds().moments().len(), 1);
}

#[test]
fn task2_addition_only_normal_silent_exports_custom_indexed_additions_without_error() {
    let scratch = Scratch::new("task2-silent-normal-custom-idx");
    let folder = scratch.path("mapset");
    std::fs::create_dir_all(&folder).unwrap();
    let wav = b"RIFF\x26\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data\x02\0\0\0\x00\x40";
    std::fs::write(folder.join("normal-hitclap2.wav"), wav).unwrap();
    let hs_path = folder.join("Artist - Song (Mapper) [Hitsounds].osu");
    let target_path = folder.join("Artist - Song (Mapper) [Hard].osu");
    std::fs::write(
        &hs_path,
        "osu file format v14\n[General]\nMode: 0\n[Metadata]\nTitle:Song\nArtist:Artist\nCreator:Mapper\nVersion:Hitsounds\n[TimingPoints]\n0,500,4,1,2,80,1,0\n[HitObjects]\n256,192,1000,1,8,0:0:2:0:\n256,192,2000,1,8,0:0:2:0:\n256,192,3000,1,2,0:0:0:0:\n",
    )
    .unwrap();
    std::fs::write(
        &target_path,
        "osu file format v14\n[General]\nMode: 0\n[Metadata]\nTitle:Song\nArtist:Artist\nCreator:Mapper\nVersion:Hard\n[TimingPoints]\n0,500,4,1,0,80,1,0\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n256,192,2000,2,0,L|356:192,1,100,0|0,0:0|0:0,0:0:0:0:\n256,192,3000,1,0,0:0:0:0:\n",
    )
    .unwrap();

    let mut app = Application::headless();
    app.project_root = Some(scratch.path("projects"));
    open(&mut app, hs_path);
    // Remove hitnormal from all moments in source so only additions remain (custom-indexed clap2
    // on circle + slider, plus skin whistle on the 3rd circle).
    let normal2 = dawsu_hitsounding::rules::read_sample_file_name("normal-hitnormal2.wav").unwrap();
    let normal0 = dawsu_foundation::SampleIdentity::Standard {
        set: dawsu_foundation::SampleSet::Normal,
        sound: dawsu_foundation::Sound::Hit(dawsu_foundation::Hitsound::Normal),
        index: dawsu_foundation::CustomIndex::new(0),
    };
    {
        let doc = app.document.as_mut().unwrap();
        doc.composer.apply("Remove normals", |hits| {
            for i in 0..hits.moments().len() {
                hits.remove_hit(dawsu_hitsounding::MomentId(i), &normal2);
                hits.remove_hit(dawsu_hitsounding::MomentId(i), &normal0);
            }
        });
    }
    let source = crate::output::Source::from_document(app.document().unwrap());
    let opts = crate::ExportOptions {
        base: false,
        targets: vec![target_path.clone()],
        addition_only_normal: dawsu_hitsounding::index_packing::AdditionOnlyNormal::Silent,
        ..Default::default()
    };
    let config = crate::output::ExportConfig {
        defaults: &app.preferences.default_samples,
        settings: None,
        projects: app.project_root.as_deref(),
        options: &opts,
    };
    let mut diagnostics = Vec::new();
    let plan = crate::output::export_report(
        &source,
        &config,
        &|| false,
        &mut diagnostics,
        &mut |_, _| {},
    )
    .expect("AdditionOnlyNormal::Silent must not fail with Sample mix lane is missing");
    assert_eq!(plan.files.len(), 1);
    let exported_osu = String::from_utf8_lossy(&plan.files[0].file.to_bytes()).into_owned();
    // Circle at 1000 ms and slider head at 2000 ms (custom-indexed clap2) and circle at 3000 ms
    // (skin whistle) must all export cleanly without falling back to default skin hitnormal.
    assert!(
        !plan.mixes.is_empty(),
        "Expected staged silent-normal mixes for addition-only moments.\nOSU:\n{exported_osu}"
    );
    assert!(
        plan.mixes.iter().all(|m| m.wav_bytes().starts_with(b"RIFF")),
        "Every staged mix must be a valid WAV"
    );
    assert!(
        exported_osu.contains("dawsu-mix-") && exported_osu.contains("2000,-100,4,1,"),
        "Expected circle custom mix and slider-edge indexed greenline in exported .osu:\n{exported_osu}"
    );
}

#[test]
fn recovery_copy_with_project_gain_recovery_preserves_working_edits_without_warning() {
    let fixture = Scratch::new("recovery-copy-gain-recovery");
    let path = fixture.path("map.osu");
    let map = b"osu file format v14\n[General]\nMode:0\n[TimingPoints]\n0,500,4,1,1,70,1,0\n[HitObjects]\n256,192,1000,1,8,1:1:1:70:\n128,192,1000,1,0,1:1:1:70:\n";
    std::fs::write(&path, map).unwrap();
    let wav_head = b"RIFF\x26\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data\x02\0\0\0";
    for (name, sample) in [("normal-hitnormal.wav", 8192_i16), ("normal-hitclap.wav", 16384)] {
        let mut wav = wav_head.to_vec();
        wav.extend(sample.to_le_bytes());
        std::fs::write(fixture.path(name), wav).unwrap();
    }

    let data = fixture.path("app-data");
    let sessions = fixture.path("sessions");
    let mut app = Application::with_data_root(data.clone());
    app.clear_error();
    app.audio = None;
    app.project_root = Some(sessions.clone());
    open(&mut app, path.clone());

    let lanes: Vec<_> = app.document().unwrap().composer.hitsounds().moments()[0].hits().cloned().collect();
    let normal = lanes.iter().find(|lane| matches!(lane,
        SampleIdentity::Standard { sound: Sound::Hit(Hitsound::Normal), .. })).unwrap().clone();
    let clap = lanes.iter().find(|lane| *lane != &normal).unwrap().clone();

    // Save once with mixed per-hit volumes so project.settings.gain_recovery is populated.
    app.select_hit(HitKey { moment: 0, lane: normal.clone() }, false);
    app.selection_volume(35, false);
    app.save_with_backups(fixture.path("backups"));
    let deadline = Instant::now() + Duration::from_secs(10);
    while app.busy {
        app.poll();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(app.error.is_none(), "{:?}", app.error);

    // Reopen cleanly with project gain_recovery active, then edit and capture a crash recovery copy.
    drop(app);
    let mut session2 = Application::with_data_root(data.clone());
    session2.clear_error();
    session2.audio = None;
    session2.project_root = Some(sessions.clone());
    open(&mut session2, path.clone());
    session2.select_hit(HitKey { moment: 0, lane: normal.clone() }, false);
    session2.selection_volume(90, false);
    session2.refresh_recovery_edit();
    let offer = session2.write_recovery_copy().unwrap().unwrap();
    drop(session2);

    let mut reopened = Application::with_data_root(data);
    reopened.clear_error();
    reopened.audio = None;
    reopened.project_root = Some(sessions);
    reopened.restore_recovery(&offer.id).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while reopened.busy {
        reopened.poll();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(reopened.error.is_none(), "{:?}", reopened.error);
    assert!(
        reopened.warnings().iter().all(|w| !w.contains("Editable hit gains could not be restored")),
        "unexpected gain recovery warning during crash recovery: {:?}",
        reopened.warnings()
    );
    let moment = &reopened.document().unwrap().composer.hitsounds().moments()[0];
    assert_eq!((moment.hit_volume(&normal), moment.hit_volume(&clap)), (90, 70));
}

