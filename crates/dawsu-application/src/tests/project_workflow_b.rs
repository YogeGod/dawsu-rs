use super::*;
use dawsu_test_support::Scratch;

const TONE: &[u8] = b"RIFF\x26\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data\x02\0\0\0\0\x40";

fn hitnormal(index: u32) -> SampleIdentity {
    SampleIdentity::Standard { set: SampleSet::Normal, sound: Sound::Hit(Hitsound::Normal), index: dawsu_foundation::CustomIndex::new(index) }
}

/// The project fixture with two new hitnormal layers, 2 and 3, both pending.
fn two_layers(scratch: &Scratch, project: bool) -> (Application, PathBuf, PathBuf) {
    two_layers_in(scratch, project, None)
}

/// With `data_root`, the session keeps its data apart from the shared headless folder.
fn two_layers_in(scratch: &Scratch, project: bool, data_root: Option<PathBuf>) -> (Application, PathBuf, PathBuf) {
    let folder = scratch.path("mapset");
    let defaults = scratch.path("skin");
    for directory in [&folder, &defaults] {
        std::fs::create_dir_all(directory).unwrap();
    }
    let map = folder.join("test.osu");
    let text = include_str!("../../tests/project-fixture.osu").replace("Title:", "Artist:Artist
Creator:Mapper
Title:");
    std::fs::write(&map, text).unwrap();
    for name in ["normal-hitnormal.wav", "normal-hitclap.wav"] {
        std::fs::write(defaults.join(name), TONE).unwrap();
    }
    let mut app = Application::headless();
    if let Some(data_root) = data_root {
        app.data_root = data_root;
    }
    if project {
        app.project_root = Some(scratch.path("projects"));
    }
    app.preferences.default_samples = defaults;
    open(&mut app, map.clone());
    app.active_lane = app.document().unwrap().composer.lanes.iter().position(|lane| lane == &hitnormal(0)).unwrap();
    app.command("addNumberedLayer");
    app.command("addNumberedLayer");
    assert!(app.error.is_none(), "{:?}", app.error);
    let pending = &app.document().unwrap().pending;
    assert!(pending.get("normal-hitnormal2.wav").is_some() && pending.get("normal-hitnormal3.wav").is_some());
    (app, map, folder)
}

#[allow(clippy::disallowed_methods)]
fn mapset(folder: &Path) -> Vec<String> {
    dawsu_osu_format::list_folder(folder).unwrap()
}

/// Layer 2 alone plays on the circle; layer 3 stays unused.
fn use_layer_2(app: &mut Application) {
    app.document.as_mut().unwrap().composer.apply("Use layer 2", |hits| {
        let circle = dawsu_hitsounding::MomentId(0);
        for lane in hits.moment(circle).hits().cloned().collect::<Vec<_>>() {
            hits.remove_hit(circle, &lane);
        }
        hits.add_hit(circle, hitnormal(2));
    });
}

#[test]
fn save_writes_only_the_pending_files_the_written_difficulty_plays() {
    for hs in [false, true] {
        let scratch = Scratch::new(if hs { "pending-save-hs" } else { "pending-save" });
        let (mut app, map, folder) = two_layers(&scratch, true);
        app.set_hs_mode(hs);
        use_layer_2(&mut app);
        let before = mapset(&folder);
        save_through_summary(&mut app, scratch.path("backups"));
        let after = mapset(&folder);
        assert_eq!(after.iter().filter(|name| !before.contains(name)).collect::<Vec<_>>(), ["normal-hitnormal2.wav"], "hs {hs}");
        assert_eq!(std::fs::read(folder.join("normal-hitnormal2.wav")).unwrap(), TONE);
        let document = app.document().unwrap();
        assert!(document.pending.get("normal-hitnormal2.wav").is_none());
        assert!(document.pending.get("normal-hitnormal3.wav").is_some(), "the unused layer stays pending");
        assert!(document.files.iter().any(|name| name == "normal-hitnormal2.wav"));
        assert!(stored(&app).pending_samples.is_none(), "nothing the project needs is left pending");
        let reopened = reopen(&app, &map);
        assert!(reopened.document().unwrap().composer.hitsounds().moments()[0].has_hit(&hitnormal(2)));
        assert_eq!(reopened.clip_for_lane(&hitnormal(2)).unwrap().samples, [0.5]);
    }
}

#[test]
fn a_mix_source_layer_stays_out_of_the_mapset_and_comes_back_on_reopen() {
    let scratch = Scratch::new("pending-mix-source");
    let (mut app, map, folder) = two_layers(&scratch, true);
    // The skin hitnormal and layer 2 at another volume: Save merges them into one hitnormal file.
    app.document.as_mut().unwrap().composer.apply("Mix", |hits| {
        let normal = dawsu_hitsounding::MomentId(0);
        hits.add_hit(normal, hitnormal(2));
        hits.set_hit_volume(normal, &hitnormal(2), 40);
    });
    let before = mapset(&folder);
    save_through_summary(&mut app, scratch.path("backups"));
    let added: Vec<_> = mapset(&folder).into_iter().filter(|name| !before.contains(name)).collect();
    assert!(!added.iter().any(|name| name.starts_with("normal-hitnormal2") || name.starts_with("normal-hitnormal3")), "{added:?}");
    assert!(!added.is_empty(), "the merged files are written");
    let kept = stored(&app).pending_samples.expect("the project lists the mix source");
    assert_eq!(kept.iter().map(|file| file.name.as_str()).collect::<Vec<_>>(), ["normal-hitnormal2.wav"]);
    let mut reopened = Application::headless();
    reopened.project_root = app.project_root.clone();
    reopened.preferences.default_samples = app.preferences.default_samples.clone();
    open(&mut reopened, map);
    // The skin folder holds only the two sounds the map plays.
    let warnings: Vec<_> = reopened.warnings.iter().filter(|w| !w.starts_with("Sample unavailable")).collect();
    assert!(warnings.is_empty(), "{warnings:?}");
    assert!(reopened.document().unwrap().composer.hitsounds().moments()[0].has_hit(&hitnormal(2)));
    assert_eq!(reopened.clip_for_lane(&hitnormal(2)).unwrap().samples, [0.5]);
}

#[test]
fn a_loose_hit_on_a_layer_keeps_the_layer_pending_in_the_project() {
    let scratch = Scratch::new("pending-loose");
    let (mut app, map, folder) = two_layers(&scratch, true);
    // A hit with no object: the project keeps it, the `.osu` never plays it.
    app.document.as_mut().unwrap().composer.add(5000.0, hitnormal(2), 100, 0.0);
    let before = mapset(&folder);
    save_through_summary(&mut app, scratch.path("backups"));
    assert_eq!(mapset(&folder).len(), before.len());
    let kept = stored(&app).pending_samples.expect("the project lists the loose hit's layer");
    assert_eq!(kept.iter().map(|file| file.name.as_str()).collect::<Vec<_>>(), ["normal-hitnormal2.wav"]);
    let reopened = reopen(&app, &map);
    let hits = reopened.document().unwrap().composer.hitsounds();
    assert!(hits.moments().iter().any(|moment| moment.has_hit(&hitnormal(2))));
    assert_eq!(reopened.clip_for_lane(&hitnormal(2)).unwrap().samples, [0.5]);
}

#[test]
fn a_left_out_moment_keeps_its_layer_pending_and_reopen_brings_it_back() {
    let scratch = Scratch::new("pending-left-out");
    let (mut app, map) = open_left_out_fixture(&scratch, true);
    let defaults = scratch.path("skin");
    std::fs::create_dir_all(&defaults).unwrap();
    std::fs::write(defaults.join("normal-hitnormal.wav"), TONE).unwrap();
    app.preferences.default_samples = defaults;
    app.active_lane = app.document().unwrap().composer.lanes.iter().position(|lane| lane == &hitnormal(0)).unwrap();
    app.command("addNumberedLayer");
    assert!(app.error.is_none(), "{:?}", app.error);
    // The layer goes on the slider tail, which Save leaves out (F052).
    app.document.as_mut().unwrap().composer.apply("Layer on the tail", |hits| {
        let tail = tail_id(hits);
        hits.add_hit(tail, hitnormal(2));
    });
    let folder = map.parent().unwrap().to_path_buf();
    let before = mapset(&folder);
    save_through_summary(&mut app, scratch.path("backups"));
    assert!(!app.document().unwrap().left_out.is_empty());
    assert_eq!(mapset(&folder), before, "the left-out layer is not written");
    let kept = stored(&app).pending_samples.expect("the project lists the left-out layer");
    assert_eq!(kept.iter().map(|file| file.name.as_str()).collect::<Vec<_>>(), ["normal-hitnormal2.wav"]);
    let reopened = reopen(&app, &map);
    let hits = reopened.document().unwrap().composer.hitsounds();
    assert!(hits.moment(tail_id(hits)).has_hit(&hitnormal(2)));
    assert_eq!(reopened.clip_for_lane(&hitnormal(2)).unwrap().samples, [0.5]);
}

#[test]
fn a_file_another_program_put_under_a_pending_name_refuses_save_and_names_it() {
    let scratch = Scratch::new("pending-clash");
    let (mut app, map, folder) = two_layers(&scratch, true);
    use_layer_2(&mut app);
    std::fs::write(folder.join("normal-hitnormal2.wav"), b"another program's file").unwrap();
    let before = (mapset(&folder), std::fs::read(&map).unwrap());
    app.save_with_backups(scratch.path("backups"));
    wait(&mut app);
    let error = app.error.clone().expect("Save refuses");
    assert!(error.contains("normal-hitnormal2.wav"), "{error}");
    assert_eq!((mapset(&folder), std::fs::read(&map).unwrap()), before);
    assert_eq!(std::fs::read(folder.join("normal-hitnormal2.wav")).unwrap(), b"another program's file");
}

#[test]
fn generate_and_copy_write_the_pending_files_their_output_plays() {
    let scratch = Scratch::new("pending-output");
    let (mut app, map, folder) = two_layers(&scratch, false);
    use_layer_2(&mut app);
    app.generate();
    wait(&mut app);
    assert!(app.error.is_none(), "{:?}", app.error);
    assert!(!folder.join("normal-hitnormal2.wav").exists(), "nothing is written before the review is applied");
    app.apply_output_in(scratch.path("backups"));
    wait(&mut app);
    assert!(app.error.is_none(), "{:?}", app.error);
    assert_eq!(std::fs::read(folder.join("normal-hitnormal2.wav")).unwrap(), TONE);
    assert!(!folder.join("normal-hitnormal3.wav").exists(), "the unused layer stays pending");
    assert!(app.document().unwrap().pending.get("normal-hitnormal2.wav").is_none());
    assert!(app.document().unwrap().files.iter().any(|name| name == "normal-hitnormal2.wav"));

    // Copy onto a sibling difficulty with layer 3 on its circle.
    let sibling = folder.join("other.osu");
    std::fs::write(&sibling, include_bytes!("../../tests/project-fixture.osu")).unwrap();
    app.document.as_mut().unwrap().composer.apply("Use layer 3", |hits| {
        hits.add_hit(dawsu_hitsounding::MomentId(0), hitnormal(3));
        hits.remove_hit(dawsu_hitsounding::MomentId(0), &hitnormal(2));
    });
    app.copy_hitsounds(vec![sibling.canonicalize().unwrap()], Default::default());
    wait(&mut app);
    assert!(app.error.is_none(), "{:?}", app.error);
    app.apply_output_in(scratch.path("backups"));
    wait(&mut app);
    assert!(app.error.is_none(), "{:?}", app.error);
    assert_eq!(std::fs::read(folder.join("normal-hitnormal3.wav")).unwrap(), TONE);
    let _ = map;
}

#[test]
fn a_restored_recovery_copy_plays_its_pending_layer_and_saves_it() {
    let scratch = Scratch::new("pending-recovery");
    let (mut app, _, folder) = two_layers(&scratch, true);
    use_layer_2(&mut app);
    app.poll();
    let offer = app.write_recovery_copy().unwrap().unwrap();
    let mut restored = Application::create(Preferences::default(), false, None, None, false, app.data_root.clone());
    restored.project_root = app.project_root.clone();
    restored.restore_recovery(&offer.id).unwrap();
    wait(&mut restored);
    assert!(restored.error.is_none(), "{:?}", restored.error);
    assert!(restored.document().unwrap().composer.hitsounds().moments()[0].has_hit(&hitnormal(2)));
    assert_eq!(restored.clip_for_lane(&hitnormal(2)).unwrap().samples, [0.5]);
    save_through_summary(&mut restored, scratch.path("backups"));
    assert_eq!(std::fs::read(folder.join("normal-hitnormal2.wav")).unwrap(), TONE);
}

#[test]
fn save_refuses_a_name_dawsu_claimed_whose_sound_is_missing() {
    let scratch = Scratch::new("pending-lost");
    let (mut app, map, folder) = two_layers(&scratch, true);
    use_layer_2(&mut app);
    // The editor lost the pending file but its claim is still there: both sides play the
    // skin's sound, which is not what the mapper drew.
    let document = app.document.as_mut().unwrap();
    let root = document.path.parent().unwrap().to_path_buf();
    document.pending = pending::Pending::for_mapset(&app.data_root, &root);
    let before = (mapset(&folder), std::fs::read(&map).unwrap());
    app.save_with_backups(scratch.path("backups"));
    wait(&mut app);
    let error = app.error.clone().expect("Save refuses");
    assert!(error.contains("normal-hitnormal2"), "{error}");
    assert_eq!((mapset(&folder), std::fs::read(&map).unwrap()), before);
}

#[test]
fn another_windows_claim_on_a_name_this_difficulty_looks_up_refuses_save() {
    let scratch = Scratch::new("pending-other-claim");
    let folder = scratch.path("mapset");
    std::fs::create_dir_all(&folder).unwrap();
    let map = folder.join("test.osu");
    // Custom index 1: the circle looks up normal-hitnormal.wav in the mapset first.
    std::fs::write(&map, include_str!("../../tests/project-fixture.osu").replace("0,500,4,1,0,100,1,0", "0,500,4,1,1,100,1,0")).unwrap();
    let mut app = Application::headless();
    app.project_root = Some(scratch.path("projects"));
    open(&mut app, map.clone());
    // Another window made a new sample under that name; it isn't in the mapset yet.
    let mut other = pending::Pending::for_mapset(&app.data_root, &folder.canonicalize().unwrap());
    other.add("normal-hitnormal.wav", TONE).unwrap();
    // Once that window saves it, this difficulty would play it: Save refuses and names it.
    app.document.as_mut().unwrap().composer.apply("Edit", |hits| hits.set_volume(dawsu_hitsounding::MomentId(0), 80));
    let before = std::fs::read(&map).unwrap();
    app.save_with_backups(scratch.path("backups"));
    wait(&mut app);
    let error = app.error.clone().expect("refused");
    assert!(error.contains("normal-hitnormal.wav"), "{error}");
    assert_eq!(std::fs::read(&map).unwrap(), before);
    other.release("normal-hitnormal.wav");
}

fn claims(app: &Application, folder: &Path) -> Vec<String> {
    let dir = app.data_root.join("pending-names").join(pending::mapset_key(&folder.canonicalize().unwrap()));
    let mut names: Vec<String> = std::fs::read_dir(dir).map(|entries| entries.flatten()
        .filter(|entry| entry.path().is_file())
        .map(|entry| entry.file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default();
    names.sort();
    names
}

#[test]
fn closing_forgets_the_pending_files_nothing_needs_and_frees_their_numbers() {
    let scratch = Scratch::new("pending-close");
    let (mut app, map, folder) = two_layers(&scratch, true);
    // Layer 2 plays a loose hit the project keeps; layer 3 is never used.
    app.document.as_mut().unwrap().composer.add(5000.0, hitnormal(2), 100, 0.0);
    save_through_summary(&mut app, scratch.path("backups"));
    assert_eq!(claims(&app, &folder), ["normal-hitnormal2.wav", "normal-hitnormal3.wav"]);
    app.close_document();
    assert_eq!(claims(&app, &folder), ["normal-hitnormal2.wav"]);
    // Reopening keeps layer 2 and gives the next new layer the freed number 3.
    let mut reopened = reopen(&app, &map);
    reopened.preferences.default_samples = app.preferences.default_samples.clone();
    reopened.active_lane = reopened.document().unwrap().composer.lanes.iter().position(|lane| lane == &hitnormal(0)).unwrap();
    reopened.command("addNumberedLayer");
    assert!(reopened.error.is_none(), "{:?}", reopened.error);
    assert!(reopened.document().unwrap().pending.get("normal-hitnormal3.wav").is_some());
}

#[test]
fn closing_without_waiting_also_frees_the_numbers_nothing_needs() {
    let scratch = Scratch::new("pending-close-nonblocking");
    let (mut app, map, folder) = two_layers(&scratch, true);
    app.document.as_mut().unwrap().composer.add(5000.0, hitnormal(2), 100, 0.0);
    save_through_summary(&mut app, scratch.path("backups"));
    assert_eq!(claims(&app, &folder), ["normal-hitnormal2.wav", "normal-hitnormal3.wav"]);
    app.close_document_nonblocking();
    assert_eq!(claims(&app, &folder), ["normal-hitnormal2.wav"]);
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while app.has_active_background_jobs() {
        app.poll();
        assert!(std::time::Instant::now() < deadline, "detached cleanup did not finish");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(claims(&app, &folder), ["normal-hitnormal2.wav"], "after the detached cleanup");
    // Reopening keeps layer 2 and gives the next new layer the freed number 3.
    let mut reopened = reopen(&app, &map);
    reopened.preferences.default_samples = app.preferences.default_samples.clone();
    reopened.active_lane = reopened.document().unwrap().composer.lanes.iter().position(|lane| lane == &hitnormal(0)).unwrap();
    reopened.command("addNumberedLayer");
    assert!(reopened.error.is_none(), "{:?}", reopened.error);
    assert!(reopened.document().unwrap().pending.get("normal-hitnormal3.wav").is_some());
}

#[test]
fn opening_releases_claims_a_closed_or_crashed_session_left_behind() {
    let scratch = Scratch::new("pending-stale-claims");
    let (app, map, folder) = two_layers(&scratch, true);
    let claimed = claims(&app, &folder);
    assert_eq!(claimed.len(), 2);
    // The session ends without a close (a crash): its claims stay until the next open.
    let data_root = app.data_root.clone();
    let project_root = app.project_root.clone();
    drop(app);
    let mut other = Application::create(Preferences::default(), false, None, None, false, data_root.clone());
    other.project_root = project_root.clone();
    let live = pending::Presence::enter(&data_root, &folder.canonicalize().unwrap()).unwrap();
    open(&mut other, map.clone());
    assert_eq!(claims(&other, &folder), claimed, "another live session may still use them");
    drop(live);
    other.close_document();
    let mut next = Application::create(Preferences::default(), false, None, None, false, data_root);
    next.project_root = project_root;
    open(&mut next, map);
    assert!(claims(&next, &folder).is_empty(), "{:?}", claims(&next, &folder));
}

#[test]
fn the_project_lists_kept_pending_files_before_the_osu_is_written() {
    let scratch = Scratch::new("pending-staged-record");
    let (mut app, _, _) = two_layers(&scratch, true);
    use_layer_2(&mut app);
    app.document.as_mut().unwrap().composer.add(5000.0, hitnormal(3), 100, 0.0);
    app.save_session().unwrap();
    let document = app.document().unwrap();
    let mut project = app.project.clone().unwrap();
    let plan = prepare_save(
        PrepareSaveInput {
            file: document.compile_base.clone(),
            hits: document.composer.hitsounds(),
            written: &document.written,
            expected: document.file.original_bytes().to_vec(),
            path: &document.path,
            backups: scratch.path("backups"),
            hs: false,
            owned: &BTreeSet::new(),
            defaults: Path::new(""),
            pending: &document.pending,
            project: Some(&project),
            review: false,
            clean_redundant_greenlines: false,
        },
        &|| false,
        &mut Vec::new(),
        &mut |_, _| {},
    ).unwrap();
    // The `.osu` write fails, as a crash right after the project write would leave it.
    std::fs::write(scratch.path("backups"), b"not a folder").unwrap();
    assert!(plan.apply(Some(&mut project), &|| false).is_err());
    let stored = project::Project::load(app.project_root.as_ref().unwrap(), None, &document.path).unwrap().settings;
    let kept = stored.pending_samples.expect("listed before the .osu write");
    assert!(kept.iter().any(|file| file.name == "normal-hitnormal3.wav"));
}

#[test]
fn a_file_another_program_writes_under_a_pending_name_keeps_the_layers_sound() {
    let scratch = Scratch::new("pending-watch-clash");
    let (mut app, _, folder) = two_layers(&scratch, false);
    let mut louder = TONE.to_vec();
    *louder.last_mut().unwrap() = 0x60;
    std::fs::write(folder.join("normal-hitnormal3.wav"), &louder).unwrap();
    let deadline = Instant::now() + Duration::from_millis(2500);
    while Instant::now() < deadline {
        app.poll();
        std::thread::sleep(Duration::from_millis(20));
    }
    let document = app.document().unwrap();
    assert!(!document.files.iter().any(|name| name == "normal-hitnormal3.wav"), "still a pending file");
    assert_eq!(app.clip_for_lane(&hitnormal(3)).unwrap().samples, [0.5], "the layer keeps its own sound");
    assert!(app.status.contains("normal-hitnormal3.wav"), "{}", app.status);
    // Reported once: the watcher stops reading and decoding the file every second.
    app.status.clear();
    let deadline = Instant::now() + Duration::from_millis(2500);
    while Instant::now() < deadline {
        app.poll();
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!app.status.contains("normal-hitnormal3.wav"), "{}", app.status);
}

fn ledger(app: &Application) -> Vec<String> {
    app.document().unwrap().pending.ledger().unwrap().entries().unwrap().into_iter().map(|file| file.name).collect()
}

#[test]
fn the_ledger_lists_each_sample_file_a_save_creates_and_drops_a_taken_back_one() {
    let scratch = Scratch::new("pending-ledger");
    let (mut app, _, _) = two_layers(&scratch, true);
    use_layer_2(&mut app);
    std::fs::write(scratch.path("backups"), b"not a folder").unwrap();
    app.save_with_backups(scratch.path("backups"));
    wait(&mut app);
    assert!(app.error.take().is_some());
    assert!(ledger(&app).is_empty(), "a taken-back file is not listed");
    std::fs::remove_file(scratch.path("backups")).unwrap();
    save_through_summary(&mut app, scratch.path("backups"));
    let entries = app.document().unwrap().pending.ledger().unwrap().entries().unwrap();
    assert_eq!(entries, [pending::PendingSample { name: "normal-hitnormal2.wav".into(), sha256: dawsu_foundation::digest::sha256_hex(TONE) }]);
}

#[test]
fn vst_drum_render_stages_avoids_existing_mapset_file_switches_lanes_in_one_undo_step_and_saves_cleanly() {
    struct NullBlockProcessor;
    impl drum_vst::DrumBlockProcessor for NullBlockProcessor {
        fn process_block(
            &mut self,
            _sample_rate: f64,
            _events: &[dawsu_audio_instruments::ShmEvent],
            left: &mut [f32],
            right: &mut [f32],
        ) -> bool {
            left.fill(0.0);
            right.fill(0.0);
            true
        }
    }

    let scratch = Scratch::new("vst-render-staging");
    let mapset_dir = scratch.path("mapset");
    std::fs::create_dir_all(&mapset_dir).unwrap();
    let osu = mapset_dir.join("Map.osu");
    std::fs::write(
        &osu,
        b"osu file format v14\n[General]\nMode:0\n[TimingPoints]\n0,500,4,1,0,100,1,0\n[HitObjects]\n256,192,1000,1,0,0:0:0:73:\n256,192,1500,1,0,0:0:0:73:\n",
    )
    .unwrap();
    // Pre-place vst-kick-v73.wav with known bytes in the mapset folder.
    let existing_bytes = b"KNOWN_MAPSET_WAV_BYTES_DO_NOT_OVERWRITE".to_vec();
    let existing_path = mapset_dir.join("vst-kick-v73.wav");
    std::fs::write(&existing_path, &existing_bytes).unwrap();

    let mut app = Application::headless_with_projects(scratch.path("data"));
    open(&mut app, osu.clone());

    // Move the two moments onto a lane whose name sanitizes to "kick" at volume 73
    // (producing "vst-kick-v73.wav").
    let kick_lane = SampleIdentity::File("kick".into());
    let normal_lane = SampleIdentity::Standard {
        set: SampleSet::Normal,
        sound: Sound::Hit(Hitsound::Normal),
        index: dawsu_foundation::CustomIndex::new(0),
    };
    {
        let doc = app.document.as_mut().unwrap();
        doc.composer.apply("Use kick lane", |hits| {
            for m in [dawsu_hitsounding::MomentId(0), dawsu_hitsounding::MomentId(1)] {
                hits.remove_hit(m, &normal_lane);
                hits.add_hit_with_volume(m, kick_lane.clone(), 73);
            }
        });
    }
    let undo_steps_before = app.document().unwrap().composer.history.position();

    let (mut handle, _) = drum_vst::DrumVstLiveHandle::new(
        "vst3-1".into(),
        "TestDrums".into(),
        "TestDrums.vst3".into(),
        None,
        true,
        Box::new(NullBlockProcessor),
    );
    handle.set_routed_notes(HashMap::from([(kick_lane.clone(), 36)]));
    app.drum_vst = Some(handle);

    let expected_staging_dir = app
        .data_root
        .join("renders")
        .join(pending::mapset_key(&mapset_dir.canonicalize().unwrap()));
    let mut observed_staging_dir = None;

    let (plan, outputs) = app
        .render_drum_vst_to_wavs_with(VelocityExportMode::default(), |_vst3, _id, plan, staging_dir| {
            observed_staging_dir = Some(staging_dir.to_path_buf());
            vst3::execute_vst3_drum_export_with(plan, staging_dir, 44_100, |_events, raw_path, _opts| {
                let mut raw = Vec::new();
                for sample in [0.5_f32, -0.5, 0.25, -0.25] {
                    raw.extend_from_slice(&sample.to_le_bytes());
                }
                std::fs::write(raw_path, raw).unwrap();
                Ok(false)
            })
            .map_err(|e| e.to_string())
        })
        .unwrap();

    assert_eq!(observed_staging_dir.as_deref(), Some(expected_staging_dir.as_path()));
    assert_eq!(plan.jobs.len(), 1);
    let free_name = plan.jobs[0].filename.clone();
    assert_ne!(free_name, "vst-kick-v73.wav");
    assert_eq!(free_name, "vst-kick-v73-2.wav");
    assert_eq!(outputs.len(), 1);
    assert!(outputs[0].path.starts_with(&expected_staging_dir));

    // Mapset file must be byte-identical and NO new file written to mapset before Save!
    assert_eq!(std::fs::read(&existing_path).unwrap(), existing_bytes);
    assert_eq!(mapset(&mapset_dir), vec!["Map.osu", "vst-kick-v73.wav"]);

    // Rendered WAV is pending, playable in editor, and routed hits switched in ONE Undo step.
    let rendered_lane = SampleIdentity::File(free_name.clone());
    {
        let doc = app.document().unwrap();
        assert!(doc.pending.get(&free_name).is_some());
        assert!(!doc.files.iter().any(|f| f == &free_name));
        assert!(app.clip_for_lane(&rendered_lane).is_some());
        assert_eq!(doc.composer.history.position(), undo_steps_before + 1);
        let m0 = doc.composer.hitsounds().moment(dawsu_hitsounding::MomentId(0));
        let m1 = doc.composer.hitsounds().moment(dawsu_hitsounding::MomentId(1));
        assert!(!m0.has_hit(&kick_lane) && m0.has_hit(&rendered_lane));
        assert!(!m1.has_hit(&kick_lane) && m1.has_hit(&rendered_lane));
    }

    // Single Undo restores all routed hits back onto the original lane.
    assert!(app.document.as_mut().unwrap().composer.history.undo());
    {
        let doc = app.document().unwrap();
        let m0 = doc.composer.hitsounds().moment(dawsu_hitsounding::MomentId(0));
        let m1 = doc.composer.hitsounds().moment(dawsu_hitsounding::MomentId(1));
        assert!(m0.has_hit(&kick_lane) && !m0.has_hit(&rendered_lane));
        assert!(m1.has_hit(&kick_lane) && !m1.has_hit(&rendered_lane));
    }

    // Redo + Save writes vst-kick-v73-2.wav into the mapset and tracks it on the ledger.
    assert!(app.document.as_mut().unwrap().composer.history.redo());
    save_through_summary(&mut app, scratch.path("backups"));
    assert_eq!(std::fs::read(&existing_path).unwrap(), existing_bytes);
    assert!(mapset_dir.join(&free_name).is_file());
    assert!(ledger(&app).iter().any(|name| name == &free_name));
}

fn back_to_skin(app: &mut Application) {
    app.document.as_mut().unwrap().composer.apply("Back to the skin", |hits| {
        let circle = dawsu_hitsounding::MomentId(0);
        hits.remove_hit(circle, &hitnormal(2));
        hits.add_hit(circle, hitnormal(0));
    });
}

/// Layer 2 joins the skin's hitnormal at another volume: the `.osu` plays a mix, and the
/// layer's file, which a hit still plays, leaves the mapset at the next Save.
fn mix_with_skin(app: &mut Application) {
    app.document.as_mut().unwrap().composer.apply("Mix", |hits| {
        hits.add_hit(dawsu_hitsounding::MomentId(0), hitnormal(0));
        hits.set_hit_volume(dawsu_hitsounding::MomentId(0), &hitnormal(2), 40);
    });
}

#[test]
fn a_file_changed_after_its_check_stays_in_the_mapset_as_changed() {
    let scratch = Scratch::new("sweep-changed-before-move");
    let (mut app, _, folder) = two_layers(&scratch, true);
    use_layer_2(&mut app);
    save_through_summary(&mut app, scratch.path("backups"));
    fn edit(path: &Path) {
        if path.to_string_lossy().contains("sweep-changed-before-move") {
            std::fs::write(path, b"edited while dawsu! checked it").unwrap();
        }
    }
    mix_with_skin(&mut app);
    let _turn = sweep::HOOK_TURN.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    *sweep::BEFORE_MOVE.lock().unwrap() = Some(edit);
    save_through_summary(&mut app, scratch.path("backups"));
    *sweep::BEFORE_MOVE.lock().unwrap() = None;
    assert_eq!(std::fs::read(folder.join("normal-hitnormal2.wav")).unwrap(), b"edited while dawsu! checked it");
    let names: Vec<_> = mapset(&folder);
    assert!(names.iter().all(|name| !name.contains("dawsu-moving")), "{names:?}");
    assert!(!ledger(&app).iter().any(|name| name == "normal-hitnormal2.wav"), "no longer dawsu!'s file");
}

#[test]
fn a_file_a_stopped_cleanup_left_aside_comes_back_under_its_name() {
    let scratch = Scratch::new("sweep-stopped-move");
    let (mut app, _, folder) = two_layers(&scratch, true);
    use_layer_2(&mut app);
    save_through_summary(&mut app, scratch.path("backups"));
    std::fs::rename(folder.join("normal-hitnormal2.wav"), folder.join(".normal-hitnormal2.wav.dawsu-moving")).unwrap();
    assert!(mapset(&folder).iter().any(|name| name.ends_with(".dawsu-moving")), "the listing shows dot-names");
    app.close_document();
    assert_eq!(std::fs::read(folder.join("normal-hitnormal2.wav")).unwrap(), TONE);
    assert!(!mapset(&folder).iter().any(|name| name.ends_with(".dawsu-moving")));
}

#[test]
fn a_file_only_a_lane_names_goes_to_removed_samples_not_pending() {
    let scratch = Scratch::new("sweep-lane-only");
    let (mut app, map, folder) = two_layers_in(&scratch, true, Some(scratch.path("data")));
    let mut bytes = TONE.to_vec();
    *bytes.last_mut().unwrap() = 0x61;
    std::fs::write(folder.join("normal-hitclap7.wav"), &bytes).unwrap();
    let sha256 = dawsu_foundation::digest::sha256_hex(&bytes);
    app.document().unwrap().pending.ledger().unwrap()
        .add(&[pending::PendingSample { name: "normal-hitclap7.wav".into(), sha256: sha256.clone() }]).unwrap();
    // Closed while another session holds the mapset, so nothing is cleaned up yet.
    let other = pending::Presence::enter(&app.data_root, &folder.canonicalize().unwrap()).unwrap();
    app.close_document();
    drop(other);
    open(&mut app, map);
    assert!(app.document().unwrap().composer.hitsounds().lanes().iter()
        .any(|lane| matches!(&lane.source, dawsu_hitsounding::LaneSource::File(name) if name == "normal-hitclap7.wav")), "it has a lane");
    save_through_summary(&mut app, scratch.path("backups"));
    assert!(!folder.join("normal-hitclap7.wav").exists());
    assert_eq!(std::fs::read(app.data_root.join("removed-samples").join(format!("{sha256}.wav"))).unwrap(), bytes);
    assert!(app.document().unwrap().pending.get("normal-hitclap7.wav").is_none(), "not a pending file");
    assert!(!app.document().unwrap().pending.claimed_names().contains("normal-hitclap7.wav"), "no claim");
    assert!(!app.data_root.join("pending-samples").join(format!("{sha256}.wav")).exists());
}

#[test]
fn a_file_a_hit_plays_comes_back_when_the_cleanup_stops_before_the_project_lists_it() {
    let scratch = Scratch::new("sweep-stopped-before-listing");
    let (mut app, map, folder) = two_layers_in(&scratch, true, Some(scratch.path("data")));
    use_layer_2(&mut app);
    save_through_summary(&mut app, scratch.path("backups"));
    let other = pending::Presence::enter(&app.data_root, &folder.canonicalize().unwrap()).unwrap();
    app.document.as_mut().unwrap().composer.apply("Mix", |hits| {
        hits.add_hit(dawsu_hitsounding::MomentId(0), hitnormal(0));
        hits.set_hit_volume(dawsu_hitsounding::MomentId(0), &hitnormal(2), 40);
    });
    save_through_summary(&mut app, scratch.path("backups"));
    drop(other);
    assert!(folder.join("normal-hitnormal2.wav").exists(), "kept while the other session was open");
    // The job runs, and dawsu! stops before the editor thread applies what it did.
    app.start_sweep(false);
    let _ = app.sweep.take().unwrap().recv().unwrap();
    assert!(!folder.join("normal-hitnormal2.wav").exists(), "moved aside, waiting for the project");
    let (data_root, project_root) = (app.data_root.clone(), app.project_root.clone());
    drop(app);
    let mut second = Application::headless();
    second.data_root = data_root;
    second.project_root = project_root;
    second.preferences.default_samples = scratch.path("skin");
    open(&mut second, map);
    assert_eq!(std::fs::read(folder.join("normal-hitnormal2.wav")).unwrap(), TONE, "back under its name");
    assert!(ledger(&second).iter().any(|name| name == "normal-hitnormal2.wav"), "still dawsu!'s");
    assert_eq!(second.document().unwrap().composer.hitsounds().lane(&hitnormal(2)).map(|lane| &lane.source),
        Some(&dawsu_hitsounding::LaneSource::File("normal-hitnormal2.wav".into())), "its lane plays the file, not the skin");
    second.document.as_mut().unwrap().composer.apply("Mix", |hits| {
        hits.add_hit(dawsu_hitsounding::MomentId(0), hitnormal(2));
        hits.set_hit_volume(dawsu_hitsounding::MomentId(0), &hitnormal(2), 40);
    });
    save_through_summary(&mut second, scratch.path("backups"));
    let document = second.document().unwrap();
    assert!(folder.join("normal-hitnormal2.wav").exists() || document.pending.get("normal-hitnormal2.wav").is_some(),
        "a hit plays it, so it never goes to removed-samples");
}

#[test]
fn a_file_only_undo_plays_stays_in_the_mapset_and_goes_to_removed_samples_at_close() {
    let scratch = Scratch::new("sweep-undo-only");
    let (mut app, _, folder) = two_layers_in(&scratch, true, Some(scratch.path("data")));
    use_layer_2(&mut app);
    save_through_summary(&mut app, scratch.path("backups"));
    back_to_skin(&mut app);
    save_through_summary(&mut app, scratch.path("backups"));
    assert_eq!(std::fs::read(folder.join("normal-hitnormal2.wav")).unwrap(), TONE, "Undo can still play it");
    let copy = format!("{}.wav", dawsu_foundation::digest::sha256_hex(TONE));
    assert!(app.document().unwrap().pending.get("normal-hitnormal2.wav").is_none(), "it never left for the data folder");
    let (data_root, project_root) = (app.data_root.clone(), app.project_root.clone());
    app.save_session().unwrap();
    app.close_document();
    assert!(!folder.join("normal-hitnormal2.wav").exists());
    pending::prune_kept_copies(&data_root, project_root.as_deref(), Duration::ZERO);
    assert_eq!(std::fs::read(data_root.join("removed-samples").join(&copy)).unwrap(), TONE, "kept, never deleted");
}

#[test]
fn a_file_left_aside_by_a_crash_is_never_replaced_by_the_next_move() {
    let scratch = Scratch::new("sweep-aside-taken");
    let (mut app, _, folder) = two_layers(&scratch, true);
    use_layer_2(&mut app);
    save_through_summary(&mut app, scratch.path("backups"));
    let aside = folder.join(".normal-hitnormal2.wav.dawsu-moving");
    std::fs::write(&aside, b"bytes a crash left aside").unwrap();
    back_to_skin(&mut app);
    save_through_summary(&mut app, scratch.path("backups"));
    assert_eq!(std::fs::read(&aside).unwrap(), b"bytes a crash left aside");
    assert_eq!(std::fs::read(folder.join("normal-hitnormal2.wav")).unwrap(), TONE, "it can't move aside, so it stays");
}

#[test]
fn a_damaged_copy_in_removed_samples_is_replaced_before_the_file_leaves() {
    let scratch = Scratch::new("sweep-damaged-trash");
    let (mut app, _, folder) = two_layers_in(&scratch, true, Some(scratch.path("data")));
    use_layer_2(&mut app);
    save_through_summary(&mut app, scratch.path("backups"));
    let other = pending::Presence::enter(&app.data_root, &folder.canonicalize().unwrap()).unwrap();
    back_to_skin(&mut app);
    save_through_summary(&mut app, scratch.path("backups"));
    drop(other);
    let kept = app.data_root.join("removed-samples").join(format!("{}.wav", dawsu_foundation::digest::sha256_hex(TONE)));
    std::fs::create_dir_all(kept.parent().unwrap()).unwrap();
    std::fs::write(&kept, b"damaged").unwrap();
    app.close_document();
    assert!(!folder.join("normal-hitnormal2.wav").exists());
    assert_eq!(std::fs::read(&kept).unwrap(), TONE, "removed-samples holds the file's bytes");
    let damaged: Vec<_> = std::fs::read_dir(kept.parent().unwrap()).unwrap().flatten()
        .filter(|e| e.file_name().to_string_lossy().contains(".damaged-")).map(|e| std::fs::read(e.path()).unwrap()).collect();
    assert_eq!(damaged, [b"damaged".to_vec()], "the damaged copy is kept aside, not overwritten");
}

#[test]
fn a_file_with_no_hits_left_goes_to_removed_samples_at_close() {
    let scratch = Scratch::new("sweep-trash-at-close");
    let (mut app, _, folder) = two_layers_in(&scratch, true, Some(scratch.path("data")));
    use_layer_2(&mut app);
    save_through_summary(&mut app, scratch.path("backups"));
    // Another session keeps the file in the mapset through this Save.
    let other = pending::Presence::enter(&app.data_root, &folder.canonicalize().unwrap()).unwrap();
    back_to_skin(&mut app);
    save_through_summary(&mut app, scratch.path("backups"));
    assert!(folder.join("normal-hitnormal2.wav").exists());
    drop(other);
    let data_root = app.data_root.clone();
    app.close_document();
    let kept = data_root.join("removed-samples").join(format!("{}.wav", dawsu_foundation::digest::sha256_hex(TONE)));
    assert_eq!(std::fs::read(kept).unwrap(), TONE);
    assert!(!folder.join("normal-hitnormal2.wav").exists());
}

#[test]
fn a_difficulty_that_cant_be_read_stops_the_cleanup_and_says_so() {
    let scratch = Scratch::new("sweep-unreadable");
    let (mut app, _, folder) = two_layers(&scratch, true);
    use_layer_2(&mut app);
    save_through_summary(&mut app, scratch.path("backups"));
    std::fs::write(folder.join("broken.osu"), "osu file format v14\n[TimingPoints]\nnot a timing point\n").unwrap();
    back_to_skin(&mut app);
    save_through_summary(&mut app, scratch.path("backups"));
    assert!(folder.join("normal-hitnormal2.wav").exists());
    assert!(app.status.contains("No unused sample files were removed") && app.status.contains("broken.osu"), "{}", app.status);
}

#[test]
fn the_project_lists_a_kept_mix_source_even_when_nothing_moved() {
    let scratch = Scratch::new("sweep-lists-needed");
    let (mut app, _, folder) = two_layers(&scratch, true);
    use_layer_2(&mut app);
    save_through_summary(&mut app, scratch.path("backups"));
    app.document.as_mut().unwrap().composer.apply("Mix", |hits| {
        hits.add_hit(dawsu_hitsounding::MomentId(0), hitnormal(0));
        hits.set_hit_volume(dawsu_hitsounding::MomentId(0), &hitnormal(2), 40);
    });
    save_through_summary(&mut app, scratch.path("backups"));
    assert!(!folder.join("normal-hitnormal2.wav").exists(), "a mix plays it now");
    // Read through the open document's path: open stores it in its full form, which on
    // Windows can differ from `map` (a short 8.3 temp folder name such as RUNNER~1).
    let listed = |app: &Application| stored(app).pending_samples.unwrap_or_default()
        .into_iter().map(|file| file.name).collect::<Vec<_>>();
    assert_eq!(listed(&app), ["normal-hitnormal2.wav"]);
    // The list is lost; the next Save with nothing to move lists it again.
    let mut project = app.project.clone().unwrap();
    let mut settings = project.settings.clone();
    settings.pending_samples = None;
    project.save(settings).unwrap();
    app.project = Some(project);
    app.document.as_mut().unwrap().composer.apply("Edit", |hits| hits.set_volume(dawsu_hitsounding::MomentId(0), 90));
    save_through_summary(&mut app, scratch.path("backups"));
    assert_eq!(listed(&app), ["normal-hitnormal2.wav"]);
}

#[test]
fn two_windows_writing_the_ledger_at_once_keep_each_others_entries() {
    let scratch = Scratch::new("ledger-two-windows");
    let (data, mapset) = (scratch.path("data"), scratch.path("mapset"));
    let threads: Vec<_> = ["a", "b"].into_iter().map(|window| {
        let ledger = pending::Pending::for_mapset(&data, &mapset).ledger().unwrap();
        std::thread::spawn(move || {
            for n in 0..40 {
                let name = format!("{window}-{n}.wav");
                ledger.add(&[pending::PendingSample { name: name.clone(), sha256: "0".repeat(64) }]).unwrap();
                if n % 4 == 3 {
                    ledger.remove(&[name]).unwrap();
                }
            }
        })
    }).collect();
    for thread in threads {
        thread.join().unwrap();
    }
    let ledger = pending::Pending::for_mapset(&data, &mapset).ledger().unwrap();
    assert_eq!(ledger.entries().unwrap().len(), 60);
}

#[test]
fn a_session_lock_still_being_made_is_left_alone() {
    let scratch = Scratch::new("presence-being-made");
    let (data, mapset) = (scratch.path("data"), scratch.path("mapset"));
    let me = pending::Presence::enter(&data, &mapset).unwrap();
    // Another window's lock file before it locked it: that window is live, and its file stays.
    let folder = data.join("sessions").join(pending::mapset_key(&mapset));
    let making = folder.join("99999-0.lock.tmp");
    std::fs::write(&making, b"").unwrap();
    assert!(me.others_live());
    assert!(making.exists());
    // One left by a crash a while ago is removed and doesn't count.
    std::fs::File::options().write(true).open(&making).unwrap()
        .set_modified(std::time::SystemTime::now() - Duration::from_secs(120)).unwrap();
    assert!(!me.others_live());
    assert!(!making.exists());
    let other = pending::Presence::enter(&data, &mapset).unwrap();
    assert!(me.others_live());
    let names: Vec<_> = std::fs::read_dir(&folder).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    assert!(names.iter().filter(|name| name.ends_with(".lock")).count() == 2, "{names:?}");
    drop(other);
}

#[test]
fn a_session_that_cannot_be_recorded_does_not_open() {
    let scratch = Scratch::new("presence-fails");
    let folder = scratch.path("mapset");
    std::fs::create_dir_all(&folder).unwrap();
    let map = folder.join("test.osu");
    std::fs::write(&map, include_str!("../../tests/project-fixture.osu")).unwrap();
    let mut app = Application::headless();
    app.data_root = scratch.path("data");
    std::fs::create_dir_all(&app.data_root).unwrap();
    // A file where the sessions folder belongs.
    std::fs::write(app.data_root.join("sessions"), b"").unwrap();
    app.open_now(OpenSource::Map(map));
    wait(&mut app);
    assert!(app.document().is_none());
    assert!(app.error.as_deref().is_some_and(|error| error.contains("session")), "{:?}", app.error);
}

#[test]
fn kept_copies_no_record_names_are_pruned() {
    let scratch = Scratch::new("prune-kept-copies");
    let (data, projects) = (scratch.path("data"), scratch.path("projects"));
    let samples = data.join("pending-samples");
    std::fs::create_dir_all(&samples).unwrap();
    let sha = |n: u8| format!("{n:064x}");
    for n in 1..=6 {
        std::fs::write(samples.join(format!("{}.wav", sha(n))), [n]).unwrap();
    }
    let write = |path: PathBuf, text: String| {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    };
    write(projects.join("p.json"), format!("{{\"settings\":{{\"pending_samples\":[{{\"name\":\"a.wav\",\"sha256\":\"{}\"}}]}}}}", sha(1)));
    write(data.join("recovery").join("c.json"), format!("{{\"pending\":[{{\"sha256\":\"{}\"}}]}}", sha(2)));
    write(data.join("recovery").join("archive").join("old.json"), format!("{{\"pending\":[{{\"sha256\":\"{}\"}}]}}", sha(3)));
    write(data.join("created-samples").join("k.json"), format!("{{\"files\":[{{\"sha256\":\"{}\"}}]}}", sha(4)));
    write(data.join("pending-names").join("k").join("normal-hitnormal2.wav"), sha(5));
    let removed = pending::prune_kept_copies(&data, Some(&projects), Duration::ZERO);
    assert_eq!(removed, 1);
    for n in 1..=5 {
        assert!(samples.join(format!("{}.wav", sha(n))).exists(), "{n} is named by a record");
    }
    assert!(!samples.join(format!("{}.wav", sha(6))).exists());
    assert_eq!(std::fs::read(data.join("removed-samples").join(format!("{}.wav", sha(6)))).unwrap(), [6], "kept in removed-samples");
    // Without a project folder nothing is known to be unnamed.
    std::fs::write(samples.join(format!("{}.wav", sha(7))), [7]).unwrap();
    assert_eq!(pending::prune_kept_copies(&data, None, Duration::ZERO), 0);
    // A copy younger than the age limit may belong to a window still making its claim.
    assert_eq!(pending::prune_kept_copies(&data, Some(&projects), Duration::from_secs(3600)), 0);
}

#[test]
fn removed_samples_keep_thirty_days_and_a_size_limit_oldest_first() {
    let scratch = Scratch::new("cap-removed-samples");
    let data = scratch.path("data");
    let folder = data.join("removed-samples");
    std::fs::create_dir_all(&folder).unwrap();
    let day = Duration::from_secs(24 * 3600);
    let now = std::time::SystemTime::now();
    for (name, age, size) in [("old.wav", 31, 10), ("a.wav", 3, 40), ("b.wav", 2, 40), ("c.wav", 1, 40)] {
        let path = folder.join(name);
        std::fs::write(&path, vec![0; size]).unwrap();
        std::fs::File::options().write(true).open(&path).unwrap().set_modified(now - day * age).unwrap();
    }
    assert_eq!(pending::cap_removed_samples(&data, day * 30, 100), 2);
    let mut left: Vec<_> = std::fs::read_dir(&folder).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    left.sort();
    assert_eq!(left, ["b.wav", "c.wav"]);
}

#[test]
fn a_reused_copy_in_removed_samples_counts_from_its_latest_removal() {
    let scratch = Scratch::new("removed-reuse-date");
    let (mut app, _, folder) = two_layers_in(&scratch, true, Some(scratch.path("data")));
    let kept = app.data_root.join("removed-samples").join(format!("{}.wav", dawsu_foundation::digest::sha256_hex(TONE)));
    std::fs::create_dir_all(kept.parent().unwrap()).unwrap();
    std::fs::write(&kept, TONE).unwrap();
    std::fs::File::options().write(true).open(&kept).unwrap()
        .set_modified(std::time::SystemTime::now() - Duration::from_secs(31 * 24 * 3600)).unwrap();
    use_layer_2(&mut app);
    save_through_summary(&mut app, scratch.path("backups"));
    let other = pending::Presence::enter(&app.data_root, &folder.canonicalize().unwrap()).unwrap();
    back_to_skin(&mut app);
    save_through_summary(&mut app, scratch.path("backups"));
    drop(other);
    let data_root = app.data_root.clone();
    app.close_document();
    wait(&mut app);
    assert!(!folder.join("normal-hitnormal2.wav").exists());
    pending::cap_removed_samples(&data_root, Duration::from_secs(30 * 24 * 3600), u64::MAX);
    assert_eq!(std::fs::read(&kept).unwrap(), TONE, "removed today, kept for 30 days from today");
}

#[test]
fn adding_a_kept_copy_that_is_already_there_refreshes_its_date() {
    let scratch = Scratch::new("pending-copy-date");
    let mut pending = pending::Pending::for_mapset(&scratch.path("data"), &scratch.path("mapset"));
    let copy = scratch.path("data").join("pending-samples").join(format!("{}.wav", dawsu_foundation::digest::sha256_hex(TONE)));
    std::fs::create_dir_all(copy.parent().unwrap()).unwrap();
    std::fs::write(&copy, TONE).unwrap();
    std::fs::File::options().write(true).open(&copy).unwrap()
        .set_modified(std::time::SystemTime::now() - Duration::from_secs(7200)).unwrap();
    pending.add("normal-hitnormal2.wav", TONE).unwrap();
    let age = std::fs::metadata(&copy).unwrap().modified().unwrap().elapsed().unwrap();
    assert!(age < Duration::from_secs(60), "{age:?}");
}

#[test]
fn a_ledger_that_cant_be_read_stops_the_cleanup_and_says_so() {
    let scratch = Scratch::new("ledger-unreadable");
    let (mut app, _, folder) = two_layers_in(&scratch, true, Some(scratch.path("data")));
    use_layer_2(&mut app);
    save_through_summary(&mut app, scratch.path("backups"));
    let path = app.data_root.join("created-samples").join(format!("{}.json", pending::mapset_key(&folder.canonicalize().unwrap())));
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    app.close_document();
    wait(&mut app);
    assert!(folder.join("normal-hitnormal2.wav").exists() || app.status.contains("ledger"), "{}", app.status);
    let ledger = pending::Pending::for_mapset(&scratch.path("data"), &folder.canonicalize().unwrap()).ledger().unwrap();
    assert!(ledger.entries().is_err());
    assert!(ledger.add(&[pending::PendingSample { name: "x.wav".into(), sha256: "0".repeat(64) }]).is_err());
}

#[test]
fn a_second_corrupt_ledger_is_kept_aside_beside_the_first() {
    let scratch = Scratch::new("ledger-corrupt-twice");
    let (app, _, _) = two_layers(&scratch, true);
    let ledger = app.document().unwrap().pending.ledger().unwrap();
    let path = app.data_root.join("created-samples").join(format!("{}.json", pending::mapset_key(app.document().unwrap().path.parent().unwrap())));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    for text in [&b"{ first"[..], b"{ second"] {
        std::fs::write(&path, text).unwrap();
        assert!(ledger.entries().unwrap().is_empty());
    }
    let key = pending::mapset_key(app.document().unwrap().path.parent().unwrap());
    let mut aside: Vec<_> = std::fs::read_dir(path.parent().unwrap()).unwrap().flatten()
        .filter(|e| { let name = e.file_name().to_string_lossy().into_owned(); name.starts_with(&key) && name.contains("corrupt") }).map(|e| std::fs::read(e.path()).unwrap()).collect();
    aside.sort();
    assert_eq!(aside, [b"{ first".to_vec(), b"{ second".to_vec()]);
}

#[test]
fn an_old_mix_no_ledger_lists_is_cleaned_up_when_nothing_plays_it() {
    let mix = |tag: &str| {
        let bytes = format!("old mix {tag}").into_bytes();
        (format!("dawsu-mix-{}-1.wav", dawsu_foundation::digest::sha256_hex(&bytes)), bytes)
    };
    let scratch = Scratch::new("sweep-old-mixes");
    let (mut app, _, folder) = two_layers(&scratch, true);
    let (unused, unused_bytes) = mix("unused");
    let (other, other_bytes) = mix("other difficulty");
    let (mania, mania_bytes) = mix("mania");
    let (story, story_bytes) = mix("storyboard");
    let (copied, copied_bytes) = mix("recovery copy");
    let (changed, _) = mix("changed");
    for (name, bytes) in [(&unused, &unused_bytes), (&other, &other_bytes), (&mania, &mania_bytes), (&story, &story_bytes), (&copied, &copied_bytes)] {
        std::fs::write(folder.join(name), bytes).unwrap();
    }
    std::fs::write(folder.join(&changed), b"edited by the mapper").unwrap();
    // An old numbered file no ledger lists stays, played or not.
    std::fs::write(folder.join("normal-hitnormal7.wav"), TONE).unwrap();
    let fixture = include_str!("../../tests/project-fixture.osu");
    std::fs::write(folder.join("other.osu"), fixture.replace("Version:Hard", "Version:Other").replace("0:0:0:0:", &format!("0:0:0:0:{other}"))).unwrap();
    std::fs::write(folder.join("mania.osu"), fixture.replace("Mode:0", "Mode:3").replace("Version:Hard", "Version:Mania").replace("0:0:0:0:", &format!("0:0:0:0:{mania}"))).unwrap();
    std::fs::write(folder.join("Artist - Title (Mapper).osb"), format!("[Events]
Sample,0,0,\"{story}\",100
")).unwrap();
    // A stopped session's recovery copy lists it as one of its pending files.
    let mut crashed = Application::headless();
    crashed.project_root = Some(scratch.path("projects"));
    open(&mut crashed, folder.join("other.osu"));
    crashed.document.as_mut().unwrap().pending.add(&copied, &copied_bytes).unwrap();
    crashed.document.as_mut().unwrap().composer.apply("Edit", |hits| hits.set_volume(dawsu_hitsounding::MomentId(0), 80));
    crashed.poll();
    crashed.write_recovery_copy().unwrap().unwrap();
    drop(crashed);

    // While another session has the mapset open, nothing leaves.
    let live = pending::Presence::enter(&app.data_root, &folder.canonicalize().unwrap()).unwrap();
    use_layer_2(&mut app);
    save_through_summary(&mut app, scratch.path("backups"));
    assert!(folder.join(&unused).exists(), "another session is open");
    drop(live);

    app.document.as_mut().unwrap().composer.apply("Louder", |hits| hits.set_volume(dawsu_hitsounding::MomentId(0), 90));
    save_through_summary(&mut app, scratch.path("backups"));
    assert!(!folder.join(&unused).exists(), "the unused old mix left the mapset");
    let kept = app.data_root.join("removed-samples").join(format!("{}.wav", dawsu_foundation::digest::sha256_hex(&unused_bytes)));
    assert_eq!(std::fs::read(kept).unwrap(), unused_bytes, "kept in removed-samples");
    for (name, why) in [(&other, "another difficulty plays it"), (&mania, "a mania difficulty plays it"), (&story, "the storyboard plays it"),
        (&copied, "a recovery copy lists it"), (&changed, "its bytes are not the ones its name gives")]
    {
        assert!(folder.join(name).exists(), "{why}");
    }
    assert_eq!(std::fs::read(folder.join("normal-hitnormal7.wav")).unwrap(), TONE, "old numbered files stay");
    assert!(ledger(&app).iter().all(|name| !name.starts_with("dawsu-mix-")), "{:?}", ledger(&app));
}

#[test]
fn a_file_a_crashed_sessions_recovery_copy_plays_stays_in_the_mapset() {
    let scratch = Scratch::new("sweep-recovery-copy");
    let (mut crashed, map, folder) = two_layers(&scratch, true);
    use_layer_2(&mut crashed);
    save_through_summary(&mut crashed, scratch.path("backups"));
    crashed.document.as_mut().unwrap().composer.apply("Edit", |hits| hits.set_volume(dawsu_hitsounding::MomentId(0), 80));
    crashed.poll();
    crashed.write_recovery_copy().unwrap().unwrap();
    drop(crashed);
    let mut second = Application::headless();
    second.project_root = Some(scratch.path("projects"));
    open(&mut second, map);
    back_to_skin(&mut second);
    save_through_summary(&mut second, scratch.path("backups"));
    assert_eq!(std::fs::read(folder.join("normal-hitnormal2.wav")).unwrap(), TONE, "the recovery copy still plays it");
}

#[test]
fn a_recovery_copy_that_cant_be_read_stops_the_cleanup_and_is_named() {
    let scratch = Scratch::new("sweep-unreadable-copy");
    let (mut app, _, folder) = two_layers_in(&scratch, true, Some(scratch.path("data")));
    use_layer_2(&mut app);
    save_through_summary(&mut app, scratch.path("backups"));
    std::fs::create_dir_all(app.data_root.join("recovery")).unwrap();
    std::fs::write(app.data_root.join("recovery").join("broken-copy.json"), b"not json").unwrap();
    back_to_skin(&mut app);
    save_through_summary(&mut app, scratch.path("backups"));
    assert_eq!(std::fs::read(folder.join("normal-hitnormal2.wav")).unwrap(), TONE, "the copy could play it");
    assert!(app.status.contains("No unused sample files were removed") && app.status.contains("broken-copy.json"), "{}", app.status);
}

/// A file dawsu! wrote that nothing plays, on the ledger: the next cleanup takes it out.
fn unused_file(app: &Application, folder: &Path) -> (Vec<u8>, String) {
    let mut bytes = TONE.to_vec();
    *bytes.last_mut().unwrap() = 0x61;
    std::fs::write(folder.join("normal-hitclap7.wav"), &bytes).unwrap();
    let sha256 = dawsu_foundation::digest::sha256_hex(&bytes);
    app.document().unwrap().pending.ledger().unwrap()
        .add(&[pending::PendingSample { name: "normal-hitclap7.wav".into(), sha256: sha256.clone() }]).unwrap();
    (bytes, sha256)
}

#[test]
fn a_cleanup_stops_when_a_difficulty_changes_while_it_runs() {
    let scratch = Scratch::new("sweep-map-changed");
    let (mut app, _, folder) = two_layers_in(&scratch, true, Some(scratch.path("data")));
    std::fs::write(folder.join("other.osu"), include_str!("../../tests/project-fixture.osu").replace("Version:Hard", "Version:Other")).unwrap();
    unused_file(&app, &folder);
    fn edit_other(path: &Path) {
        if path.to_string_lossy().contains("sweep-map-changed") {
            let other = path.with_file_name("other.osu");
            let text = std::fs::read_to_string(&other).unwrap();
            std::fs::write(&other, text + "\n").unwrap();
        }
    }
    app.document.as_mut().unwrap().composer.apply("Edit", |hits| hits.set_volume(dawsu_hitsounding::MomentId(0), 80));
    let _turn = sweep::HOOK_TURN.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    *sweep::BEFORE_MOVE.lock().unwrap() = Some(edit_other);
    save_through_summary(&mut app, scratch.path("backups"));
    *sweep::BEFORE_MOVE.lock().unwrap() = None;
    assert!(folder.join("normal-hitclap7.wav").exists(), "other.osu changed, so nothing moves");
    assert!(ledger(&app).iter().any(|name| name == "normal-hitclap7.wav"), "still dawsu!'s, tried again later");
    app.close_document();
    assert!(!folder.join("normal-hitclap7.wav").exists(), "the next cleanup takes it out");
}

#[test]
fn a_file_only_a_recovery_copys_lane_names_leaves_the_mapset() {
    let scratch = Scratch::new("sweep-recovery-lane-only");
    let (mut crashed, map, folder) = two_layers_in(&scratch, true, Some(scratch.path("data")));
    let (bytes, sha256) = unused_file(&crashed, &folder);
    // Reopened while another session holds the mapset, so the file gets a lane and stays.
    let other = pending::Presence::enter(&crashed.data_root, &folder.canonicalize().unwrap()).unwrap();
    crashed.close_document();
    open(&mut crashed, map.clone());
    drop(other);
    assert!(crashed.document().unwrap().composer.hitsounds().lanes().iter()
        .any(|lane| matches!(&lane.source, dawsu_hitsounding::LaneSource::File(name) if name == "normal-hitclap7.wav")), "it has a lane");
    crashed.document.as_mut().unwrap().composer.apply("Edit", |hits| hits.set_volume(dawsu_hitsounding::MomentId(0), 80));
    crashed.poll();
    crashed.write_recovery_copy().unwrap().unwrap();
    let (data_root, project_root) = (crashed.data_root.clone(), crashed.project_root.clone());
    drop(crashed);
    let mut second = Application::headless();
    second.data_root = data_root;
    second.project_root = project_root;
    second.preferences.default_samples = scratch.path("skin");
    open(&mut second, map);
    second.document.as_mut().unwrap().composer.apply("Edit", |hits| hits.set_volume(dawsu_hitsounding::MomentId(0), 70));
    // The copy is still there when the cleanup runs, and its lane names the file.
    let copies: Vec<String> = std::fs::read_dir(second.data_root.join("recovery")).unwrap().flatten()
        .filter(|entry| entry.path().is_file()).map(|entry| std::fs::read_to_string(entry.path()).unwrap()).collect();
    assert!(copies.iter().any(|copy| copy.contains("normal-hitclap7.wav")), "the copy names the file");
    save_through_summary(&mut second, scratch.path("backups"));
    assert!(!folder.join("normal-hitclap7.wav").exists(), "a lane with no hit keeps nothing, in a copy too");
    assert_eq!(std::fs::read(second.data_root.join("removed-samples").join(format!("{sha256}.wav"))).unwrap(), bytes);
}

#[test]
fn a_recovery_copy_that_cant_be_read_frees_no_claim() {
    let scratch = Scratch::new("claims-unreadable-copy");
    let (app, map, folder) = two_layers_in(&scratch, true, Some(scratch.path("data")));
    let claimed = claims(&app, &folder);
    assert_eq!(claimed.len(), 2);
    let (data_root, project_root) = (app.data_root.clone(), app.project_root.clone());
    std::fs::create_dir_all(data_root.join("recovery")).unwrap();
    std::fs::write(data_root.join("recovery").join("broken-copy.json"), b"not json").unwrap();
    // A crash: the claims stay, and the next open can't tell whether the copy needs them.
    drop(app);
    let mut next = Application::create(Preferences::default(), false, None, None, false, data_root.clone());
    next.project_root = project_root.clone();
    open(&mut next, map.clone());
    assert_eq!(claims(&next, &folder), claimed, "at open");
    // Its own unused layer 3 keeps its claim at close too.
    next.close_document();
    let closing = Scratch::new("claims-unreadable-copy-close");
    let (mut third, _, folder) = two_layers_in(&closing, true, Some(closing.path("data")));
    std::fs::create_dir_all(third.data_root.join("recovery")).unwrap();
    std::fs::write(third.data_root.join("recovery").join("broken-copy.json"), b"not json").unwrap();
    third.close_document();
    assert_eq!(claims(&third, &folder), ["normal-hitnormal2.wav", "normal-hitnormal3.wav"], "at close");
}

#[test]
fn close_waits_at_most_the_limit_and_the_cleanup_keeps_the_mapset_marked_open() {
    let scratch = Scratch::new("sweep-slow-close");
    let (mut app, _, folder) = two_layers_in(&scratch, true, Some(scratch.path("data")));
    unused_file(&app, &folder);
    fn slow(path: &Path) {
        if path.to_string_lossy().contains("sweep-slow-close") {
            std::thread::sleep(Duration::from_millis(1500));
        }
    }
    app.sweep_wait = Duration::from_millis(100);
    let _turn = sweep::HOOK_TURN.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    *sweep::BEFORE_MOVE.lock().unwrap() = Some(slow);
    let started = Instant::now();
    app.close_document();
    assert!(started.elapsed() < Duration::from_millis(1000), "{:?}", started.elapsed());
    assert!(app.status.contains("Unused sample cleanup is still finishing in the background"), "{}", app.status);
    let other = pending::Presence::enter(&app.data_root, &folder.canonicalize().unwrap()).unwrap();
    assert!(other.others_live(), "the cleanup keeps the mapset marked open");
    drop(other);
    let again = Instant::now();
    app.poll_sweep(true);
    assert!(again.elapsed() < Duration::from_millis(50), "a job already given up on is not waited for again: {:?}", again.elapsed());
    finish_sweep(&mut app);
    *sweep::BEFORE_MOVE.lock().unwrap() = None;
    assert!(!folder.join("normal-hitclap7.wav").exists(), "it still finishes");
    assert!(!app.status.contains("still finishing"), "{}", app.status);
    let probe = pending::Presence::enter(&app.data_root, &folder.canonicalize().unwrap()).unwrap();
    assert!(!probe.others_live(), "and the mapset is free again");
}

#[test]
fn a_file_waiting_to_leave_stays_when_open_stops_waiting_for_the_cleanup() {
    let scratch = Scratch::new("sweep-slow-open");
    let (mut app, map, folder) = two_layers_in(&scratch, true, Some(scratch.path("data")));
    use_layer_2(&mut app);
    save_through_summary(&mut app, scratch.path("backups"));
    fn slow(path: &Path) {
        if path.to_string_lossy().contains("sweep-slow-open") {
            std::thread::sleep(Duration::from_millis(1000));
        }
    }
    // A mix plays layer 2 now, so this Save's cleanup moves it out; the map opens right after.
    mix_with_skin(&mut app);
    app.request_open(map);
    app.sweep_wait = Duration::from_millis(100);
    let _turn = sweep::HOOK_TURN.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    *sweep::BEFORE_MOVE.lock().unwrap() = Some(slow);
    let opens = app.opens;
    save_through_summary(&mut app, scratch.path("backups"));
    assert_eq!(app.opens, opens + 1, "opened while the cleanup still ran");
    assert!(app.sweep.is_some(), "the job is still running");
    assert!(app.status.contains("Unused sample cleanup is still finishing in the background"), "{}", app.status);
    finish_sweep(&mut app);
    *sweep::BEFORE_MOVE.lock().unwrap() = None;
    assert_eq!(std::fs::read(folder.join("normal-hitnormal2.wav")).unwrap(), TONE, "back under its name");
    assert!(!mapset(&folder).iter().any(|name| name.ends_with(".dawsu-moving")));
    assert!(ledger(&app).iter().any(|name| name == "normal-hitnormal2.wav"), "still dawsu!'s; the next Save tries again");
    assert!(!app.status.contains("still finishing"), "{}", app.status);
}

/// Polls until the cleanup job is done.
fn finish_sweep(app: &mut Application) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while app.sweep.is_some() {
        app.poll();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn when_the_next_map_fails_to_open_the_cleanup_finishes_for_the_map_still_open() {
    let scratch = Scratch::new("sweep-open-fails");
    let (mut app, _, folder) = two_layers_in(&scratch, true, Some(scratch.path("data")));
    use_layer_2(&mut app);
    save_through_summary(&mut app, scratch.path("backups"));
    fn slow(path: &Path) {
        if path.to_string_lossy().contains("sweep-open-fails") {
            std::thread::sleep(Duration::from_millis(500));
        }
    }
    mix_with_skin(&mut app);
    app.request_open(scratch.path("missing").join("gone.osu"));
    app.sweep_wait = Duration::from_millis(100);
    let _turn = sweep::HOOK_TURN.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    *sweep::BEFORE_MOVE.lock().unwrap() = Some(slow);
    app.save_with_backups(scratch.path("backups"));
    wait(&mut app);
    assert!(app.error.take().is_some(), "the open failed");
    finish_sweep(&mut app);
    *sweep::BEFORE_MOVE.lock().unwrap() = None;
    assert!(!folder.join("normal-hitnormal2.wav").exists(), "the same map is still open, so the file leaves as planned");
    assert!(app.document().unwrap().pending.get("normal-hitnormal2.wav").is_some(), "as a pending file");
}

#[test]
fn a_cleanup_stops_when_another_session_opens_the_mapset_while_it_runs() {
    static OTHER: std::sync::Mutex<Option<pending::Presence>> = std::sync::Mutex::new(None);
    fn open_elsewhere(path: &Path) {
        if path.to_string_lossy().contains("sweep-other-opens") {
            let mapset = path.parent().unwrap().canonicalize().unwrap();
            let data_root = mapset.parent().unwrap().join("data");
            *OTHER.lock().unwrap() = Some(pending::Presence::enter(&data_root, &mapset).unwrap());
        }
    }
    let scratch = Scratch::new("sweep-other-opens");
    let (mut app, _, folder) = two_layers_in(&scratch, true, Some(scratch.path("data")));
    unused_file(&app, &folder);
    app.document.as_mut().unwrap().composer.apply("Edit", |hits| hits.set_volume(dawsu_hitsounding::MomentId(0), 80));
    let _turn = sweep::HOOK_TURN.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    *sweep::BEFORE_MOVE.lock().unwrap() = Some(open_elsewhere);
    save_through_summary(&mut app, scratch.path("backups"));
    *sweep::BEFORE_MOVE.lock().unwrap() = None;
    assert!(OTHER.lock().unwrap().is_some(), "the hook ran");
    assert!(folder.join("normal-hitclap7.wav").exists(), "another session opened the mapset, so nothing moves");
    *OTHER.lock().unwrap() = None;
    app.close_document();
    assert!(!folder.join("normal-hitclap7.wav").exists(), "the next cleanup takes it out");
}

#[test]
fn an_old_mix_nothing_plays_goes_to_dawsus_removed_samples_folder() {
    let scratch = Scratch::new("sweep-trash");
    let (mut app, _, folder) = two_layers(&scratch, true);
    app.document.as_mut().unwrap().composer.apply("Mix", |hits| {
        hits.add_hit(dawsu_hitsounding::MomentId(0), hitnormal(2));
        hits.set_hit_volume(dawsu_hitsounding::MomentId(0), &hitnormal(2), 40);
    });
    let before = mapset(&folder);
    save_through_summary(&mut app, scratch.path("backups"));
    let first: Vec<_> = mapset(&folder).into_iter().filter(|name| !before.contains(name)).collect();
    assert!(!first.is_empty());
    let bytes: Vec<_> = first.iter().map(|name| std::fs::read(folder.join(name)).unwrap()).collect();
    app.document.as_mut().unwrap().composer.apply("Louder", |hits| hits.set_hit_volume(dawsu_hitsounding::MomentId(0), &hitnormal(2), 70));
    save_through_summary(&mut app, scratch.path("backups"));
    for (name, bytes) in first.iter().zip(&bytes) {
        assert!(!folder.join(name).exists(), "{name} left the mapset");
        let kept = app.data_root.join("removed-samples").join(format!("{}.wav", dawsu_foundation::digest::sha256_hex(bytes)));
        assert_eq!(&std::fs::read(kept).unwrap(), bytes, "{name} is kept in removed-samples");
    }
}

#[test]
fn nothing_is_cleaned_up_while_another_session_has_the_mapset_open() {
    let scratch = Scratch::new("sweep-live");
    let (mut app, _, folder) = two_layers(&scratch, true);
    use_layer_2(&mut app);
    save_through_summary(&mut app, scratch.path("backups"));
    let other = pending::Presence::enter(&app.data_root, &folder.canonicalize().unwrap()).unwrap();
    back_to_skin(&mut app);
    save_through_summary(&mut app, scratch.path("backups"));
    assert!(folder.join("normal-hitnormal2.wav").exists());
    drop(other);
}

#[test]
fn a_file_changed_before_the_cleanup_is_left_alone_and_no_longer_dawsus() {
    let scratch = Scratch::new("sweep-changed");
    let (mut app, _, folder) = two_layers(&scratch, true);
    use_layer_2(&mut app);
    save_through_summary(&mut app, scratch.path("backups"));
    std::fs::write(folder.join("normal-hitnormal2.wav"), b"edited by the mapper").unwrap();
    back_to_skin(&mut app);
    save_through_summary(&mut app, scratch.path("backups"));
    assert_eq!(std::fs::read(folder.join("normal-hitnormal2.wav")).unwrap(), b"edited by the mapper");
    assert!(ledger(&app).is_empty());
}

#[test]
fn a_save_with_nothing_to_save_and_a_save_of_an_earlier_revision_both_clean_up() {
    let scratch = Scratch::new("sweep-nothing-and-earlier");
    let (mut app, _, folder) = two_layers(&scratch, true);
    use_layer_2(&mut app);
    save_through_summary(&mut app, scratch.path("backups"));
    save_through_summary(&mut app, scratch.path("backups"));
    assert!(folder.join("normal-hitnormal2.wav").exists(), "still played");
    mix_with_skin(&mut app);
    app.save_with_backups(scratch.path("backups"));
    // A newer edit arrives while the Save runs.
    app.document.as_mut().unwrap().composer.apply("Newer", |hits| hits.set_volume(dawsu_hitsounding::MomentId(0), 90));
    wait(&mut app);
    assert!(app.error.is_none(), "{:?}", app.error);
    assert!(app.status.starts_with("Saved the earlier revision"), "{}", app.status);
    assert!(!folder.join("normal-hitnormal2.wav").exists());
    assert_eq!(app.clip_for_lane(&hitnormal(2)).unwrap().samples, [0.5], "still playable from the data folder");
}

#[test]
fn a_corrupt_ledger_is_kept_aside_and_a_new_one_started() {
    let scratch = Scratch::new("ledger-corrupt");
    let (mut app, _, _) = two_layers(&scratch, true);
    let path = app.data_root.join("created-samples").join(format!("{}.json", pending::mapset_key(app.document().unwrap().path.parent().unwrap())));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, b"{ not json").unwrap();
    use_layer_2(&mut app);
    save_through_summary(&mut app, scratch.path("backups"));
    assert_eq!(ledger(&app), ["normal-hitnormal2.wav"]);
    let key = pending::mapset_key(app.document().unwrap().path.parent().unwrap());
    let aside: Vec<_> = std::fs::read_dir(path.parent().unwrap()).unwrap().flatten()
        .filter(|e| { let name = e.file_name().to_string_lossy().into_owned(); name.starts_with(&key) && name.contains("corrupt") }).collect();
    assert_eq!(aside.len(), 1);
    assert_eq!(std::fs::read(aside[0].path()).unwrap(), b"{ not json");
}

#[test]
fn without_a_project_save_refuses_to_leave_a_new_sample_out() {
    let scratch = Scratch::new("pending-kept-no-project");
    let (mut app, map, folder) = two_layers(&scratch, false);
    app.document.as_mut().unwrap().composer.apply("Mix", |hits| {
        hits.add_hit(dawsu_hitsounding::MomentId(0), hitnormal(2));
        hits.set_hit_volume(dawsu_hitsounding::MomentId(0), &hitnormal(2), 40);
    });
    let before = (mapset(&folder), std::fs::read(&map).unwrap());
    app.save_with_backups(scratch.path("backups"));
    wait(&mut app);
    let error = app.error.clone().expect("refused");
    assert!(error.contains("normal-hitnormal2.wav"), "{error}");
    assert_eq!((mapset(&folder), std::fs::read(&map).unwrap()), before);
}

#[test]
fn a_pending_index_another_difficulty_now_uses_refuses_save() {
    let scratch = Scratch::new("pending-index-taken");
    let (mut app, map, folder) = two_layers(&scratch, true);
    use_layer_2(&mut app);
    std::fs::write(folder.join("other.osu"), include_str!("../../tests/project-fixture.osu").replace("0,500,4,1,0,100,1,0", "0,500,4,1,2,100,1,0")).unwrap();
    let before = std::fs::read(&map).unwrap();
    app.save_with_backups(scratch.path("backups"));
    wait(&mut app);
    let error = app.error.clone().expect("refused");
    assert!(error.contains("2") && error.contains("other.osu"), "{error}");
    assert_eq!(std::fs::read(&map).unwrap(), before);
    assert!(!folder.join("normal-hitnormal2.wav").exists());
}

#[test]
fn a_pending_name_must_be_a_bare_file_name() {
    let scratch = Scratch::new("pending-bare-names");
    let mut pending = pending::Pending::for_mapset(&scratch.path("data"), &scratch.path("mapset"));
    for name in ["../x.wav", "a/b.wav", "a\\b.wav", "C:x.wav", "..", ""] {
        assert!(pending.add(name, TONE).is_err(), "{name}");
        assert!(pending.restore(name, &dawsu_foundation::digest::sha256_hex(TONE)).is_err(), "{name}");
    }
    assert!(pending.add("normal-hitnormal2.wav", TONE).is_ok());
}

#[test]
fn slider_tick_and_whistle_lanes_on_pending_files_are_kept() {
    let scratch = Scratch::new("pending-slider-lanes");
    let map = b"osu file format v14\n[General]\nMode:0\n[Difficulty]\nSliderMultiplier:1.4\nSliderTickRate:2\n[TimingPoints]\n0,500,4,1,2,70,1,0\n[HitObjects]\n100,100,1000,2,2,L|300:100,1,280\n";
    let file = OsuFile::read(map.to_vec()).file;
    let mut pending = pending::Pending::for_mapset(&scratch.path("data"), &scratch.path("mapset"));
    pending.add("normal-slidertick2.wav", TONE).unwrap();
    pending.add("normal-sliderwhistle2.wav", TONE).unwrap();
    let folder = vec!["normal-slidertick2.wav".to_string(), "normal-sliderwhistle2.wav".to_string()];
    let Decompiled::Done(hits) = decompile(&file, &folder, &|| false, &mut |_, _| {}) else { panic!() };
    let needed: Vec<_> = pending::needed(&hits, &pending, &[]).into_iter().map(|file| file.name).collect();
    assert_eq!(needed, ["normal-slidertick2.wav", "normal-sliderwhistle2.wav"]);
}

#[test]
fn a_sample_file_dawsu_made_that_nothing_plays_leaves_the_mapset_and_others_stay() {
    let scratch = Scratch::new("pending-sweep");
    let (mut app, map, folder) = two_layers(&scratch, true);
    // Files the sweep must leave alone: one dawsu! never made, one with a dawsu!-style name
    // it didn't make, one a mania difficulty plays and one a storyboard plays (both listed
    // as dawsu!'s).
    let canaries = [("keep.wav", b"mapper".to_vec()), ("normal-hitnormal5.wav", b"older".to_vec()),
        ("soft-hitclap6.wav", TONE.to_vec()), ("sb.wav", TONE.to_vec())];
    for (name, bytes) in &canaries {
        std::fs::write(folder.join(name), bytes).unwrap();
    }
    std::fs::write(folder.join("mania.osu"), "osu file format v14\n[General]\nMode:3\n[TimingPoints]\n0,500,4,2,6,100,1,0\n[HitObjects]\n64,192,1000,1,8,0:0:0:0:\n").unwrap();
    std::fs::write(folder.join("story.osb"), "[Events]\nSample,0,0,\"sb.wav\",100\n").unwrap();
    app.document().unwrap().pending.ledger().unwrap().add(&[
        pending::PendingSample { name: "soft-hitclap6.wav".into(), sha256: dawsu_foundation::digest::sha256_hex(TONE) },
        pending::PendingSample { name: "sb.wav".into(), sha256: dawsu_foundation::digest::sha256_hex(TONE) },
    ]).unwrap();
    use_layer_2(&mut app);
    save_through_summary(&mut app, scratch.path("backups"));
    assert!(folder.join("normal-hitnormal2.wav").exists());
    // The layer's hit goes, but an undo state still plays its file: it stays in the mapset
    // until close (Yoge, 2026-11-21).
    app.document.as_mut().unwrap().composer.apply("Back to the skin", |hits| {
        let circle = dawsu_hitsounding::MomentId(0);
        hits.remove_hit(circle, &hitnormal(2));
        hits.add_hit(circle, hitnormal(0));
    });
    save_through_summary(&mut app, scratch.path("backups"));
    assert_eq!(std::fs::read(folder.join("normal-hitnormal2.wav")).unwrap(), TONE, "an undo state plays it");
    assert!(!app.status.contains("Removed"), "{}", app.status);
    assert!(app.document().unwrap().pending.get("normal-hitnormal2.wav").is_none(), "it never went to pending-samples");
    // Undo brings the hit back, playing from the mapset, and Save keeps it.
    app.command("undo");
    assert_eq!(app.clip_for_lane(&hitnormal(2)).unwrap().samples, [0.5]);
    save_through_summary(&mut app, scratch.path("backups"));
    assert_eq!(std::fs::read(folder.join("normal-hitnormal2.wav")).unwrap(), TONE);
    app.command("redo");
    save_through_summary(&mut app, scratch.path("backups"));
    assert_eq!(std::fs::read(folder.join("normal-hitnormal2.wav")).unwrap(), TONE, "Undo can still bring it back");
    // Close is strict: the undo history goes, so the file goes to removed-samples.
    app.close_document();
    assert!(!folder.join("normal-hitnormal2.wav").exists());
    let kept = app.data_root.join("removed-samples").join(format!("{}.wav", dawsu_foundation::digest::sha256_hex(TONE)));
    assert_eq!(std::fs::read(kept).unwrap(), TONE, "kept in removed-samples");
    for (name, bytes) in &canaries {
        assert_eq!(&std::fs::read(folder.join(name)).unwrap(), bytes, "{name}");
    }
    let _ = map;
}

#[test]
fn a_failed_output_write_removes_the_pending_files_it_wrote() {
    let scratch = Scratch::new("pending-output-fails");
    let (mut app, _, folder) = two_layers(&scratch, false);
    use_layer_2(&mut app);
    app.generate();
    wait(&mut app);
    std::fs::write(scratch.path("backups"), b"not a folder").unwrap();
    // The generated difficulty's name is taken just before the write, so it fails.
    let target = app.output_review.as_ref().unwrap().files[0].path.clone();
    std::fs::write(&target, b"taken").unwrap();
    app.apply_output_in(scratch.path("backups"));
    wait(&mut app);
    assert!(app.error.is_some());
    assert!(!folder.join("normal-hitnormal2.wav").exists());
    assert!(app.document().unwrap().pending.get("normal-hitnormal2.wav").is_some());
}

#[test]
fn a_failed_osu_write_removes_the_pending_files_it_wrote() {
    let scratch = Scratch::new("pending-write-fails");
    let (mut app, map, folder) = two_layers(&scratch, true);
    use_layer_2(&mut app);
    let before = (mapset(&folder), std::fs::read(&map).unwrap());
    // The backup folder is a file, so the `.osu` write fails after the samples are written.
    std::fs::write(scratch.path("backups"), b"not a folder").unwrap();
    app.save_with_backups(scratch.path("backups"));
    wait(&mut app);
    assert!(app.error.is_some());
    assert_eq!((mapset(&folder), std::fs::read(&map).unwrap()), before);
    assert!(app.document().unwrap().pending.get("normal-hitnormal2.wav").is_some());
}

#[test]
fn a_save_without_a_project_writes_the_pending_file_it_plays() {
    let scratch = Scratch::new("pending-no-project");
    let (mut app, _, folder) = two_layers(&scratch, false);
    use_layer_2(&mut app);
    save_through_summary(&mut app, scratch.path("backups"));
    assert_eq!(std::fs::read(folder.join("normal-hitnormal2.wav")).unwrap(), TONE);
    assert!(!folder.join("normal-hitnormal3.wav").exists());
}

#[test]
fn the_written_file_check_refuses_a_pending_file_the_save_missed() {
    let scratch = Scratch::new("pending-guard");
    let (mut app, _, folder) = two_layers(&scratch, false);
    use_layer_2(&mut app);
    let document = app.document().unwrap();
    let out = compile_snapshot(document, document.composer.hitsounds(), false, false, &|| false).unwrap().0;
    let editor = pending::listing(&folder, &document.pending).unwrap();
    let used = pending::used(&out, &folder, &document.pending, &[]).unwrap();
    assert_eq!(used.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>(), ["normal-hitnormal2.wav"]);
    assert!(pending::check_written(&out, &folder, &["normal-hitnormal2.wav".into()], &editor, &document.pending).is_ok());
    let error = pending::check_written(&out, &folder, &[], &editor, &document.pending).unwrap_err();
    assert!(error.contains("normal-hitnormal2.wav"), "{error}");
}

#[test]
fn sample_added_to_open_map_is_playable_without_losing_edits_or_redo() {
    let scratch = Scratch::new("sample-watch");
    let map = scratch.path("test.osu");
    let original = include_bytes!("../../tests/project-fixture.osu");
    std::fs::write(&map, original).unwrap();
    let mut app = Application::headless();
    open(&mut app, map.clone());

    let clap = SampleIdentity::Standard {
        set: SampleSet::Normal,
        sound: Sound::Hit(Hitsound::Clap),
        index: dawsu_foundation::CustomIndex::new(0),
    };
    {
        let composer = &mut app.document.as_mut().unwrap().composer;
        composer.add(1500.0, clap.clone(), 80, 0.0);
        composer.add(2000.0, clap, 80, 0.0);
        composer.undo();
    }
    let name = "soft-hitfinish2.wav";
    let wave = b"RIFF\x26\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data\x02\0\0\0\0\x40";
    std::fs::write(scratch.path(name), wave).unwrap();
    let lane = dawsu_hitsounding::rules::read_sample_file_name(name).unwrap();
    let deadline = Instant::now() + Duration::from_secs(4);
    while app.clip_for_lane(&lane).is_none() {
        app.poll();
        assert!(Instant::now() < deadline, "new sample did not become playable");
        std::thread::sleep(Duration::from_millis(10));
    }
    let document = app.document().unwrap();
    assert!(document.files.iter().any(|file| file == name));
    assert!(document.composer.lanes.contains(&lane));
    assert!(document.composer.dirty());
    assert!(document.composer.history.can_redo());
    assert_eq!(document.composer.history.steps(), ["Draw hit", "Draw hit"]);

    let composer = &mut app.document.as_mut().unwrap().composer;
    composer.redo();
    assert!(composer.hitsounds().lane(&lane).is_some());
    assert!(composer.hitsounds().moments().iter().any(|m| m.time().ms() == 2000.0));
    composer.add(1000.0, lane, 100, 0.0);
    let document = app.document().unwrap();
    let cues = app.scheduled_sounds(document);
    assert!(cues.iter().any(|cue| cue.at_ms == 1000.0 && cue.clip.samples == [0.5]));
    assert_eq!(std::fs::read(map).unwrap(), original);
}

#[test]
fn repeating_an_endpoint_shortcut_seeks_to_the_song_boundary() {
    use dawsu_editor::keys::{Key, Mods};
    let scratch = Scratch::new("endpoint-shortcuts");
    let map = scratch.path("Hard.osu");
    let original = b"osu file format v14\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n256,192,2000,1,0,0:0:0:0:\n";
    std::fs::write(&map, original).unwrap();
    let mut app = Application::headless();
    open(&mut app, map.clone());
    let press = |app: &mut Application, key| app.key(KeyChord::new(key, Mods::NONE));
    press(&mut app, Key::Z);
    assert_eq!(app.position_ms(), 1000.0);
    press(&mut app, Key::Z);
    assert_eq!(app.position_ms(), 0.0);
    press(&mut app, Key::V);
    assert_eq!(app.position_ms(), 2000.0);
    press(&mut app, Key::V);
    assert_eq!(app.position_ms(), 4000.0);
    assert!(!app.is_dirty());

    let lane = SampleIdentity::Standard {
        set: SampleSet::Normal, sound: Sound::Hit(Hitsound::Clap),
        index: dawsu_foundation::CustomIndex::new(0),
    };
    let composer = &mut app.document.as_mut().unwrap().composer;
    composer.add(800.0, lane.clone(), 40, 0.0);
    composer.add(2200.0, lane, 60, 0.0);
    press(&mut app, Key::Z);
    assert_eq!(app.position_ms(), 800.0);
    app.last_endpoint_jump = Some((true, Instant::now() - Duration::from_millis(401)));
    press(&mut app, Key::Z);
    assert_eq!(app.position_ms(), 800.0);
    press(&mut app, Key::V);
    assert_eq!(app.position_ms(), 2200.0, "changing direction starts a fresh jump");
    assert_eq!(std::fs::read(map).unwrap(), original);
}

#[test]
fn grid_alignment_shortcuts_shift_the_whole_selection_in_one_undo_step() {
    use dawsu_editor::keys::{Key, Mods};
    use dawsu_editor::tempo::SnapMode;

    let scratch = Scratch::new("grid-alignment");
    let map = scratch.path("Hard.osu");
    let original = b"osu file format v14\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n1150,360,4,1,0,100,1,0\n\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n256,192,1200,1,0,0:0:0:0:\n256,192,1318,1,0,0:0:0:0:\n";
    std::fs::write(&map, original).unwrap();
    for (mode, next, previous) in [
        (SnapMode::Grid, [1150.0, 1320.0], [1125.0, 1295.0]),
        (SnapMode::GridRelative, [1150.0, 1320.0], [1125.0, 1295.0]),
        (SnapMode::Objects, [1200.0, 1370.0], [1000.0, 1170.0]),
    ] {
        let mut app = Application::headless();
        app.choose_key_preset("DAW");
        open(&mut app, map.clone());
        let lane = SampleIdentity::Standard {
            set: SampleSet::Normal, sound: Sound::Hit(Hitsound::Clap),
            index: dawsu_foundation::CustomIndex::new(0),
        };
        let composer = &mut app.document.as_mut().unwrap().composer;
        composer.snap_mode = mode;
        composer.spacing = SnapSpacing::new(1, 4).unwrap();
        composer.add(1300.0, lane.clone(), 70, 0.0);
        composer.add(1130.0, lane.clone(), 40, 0.0);
        let moments = composer.hitsounds().moments();
        let selected: Vec<_> = moments.iter().enumerate()
            .filter(|(_, moment)| moment.has_hit(&lane))
            .map(|(moment, _)| HitKey { moment, lane: lane.clone() }).collect();
        composer.selected.replace(selected);
        let state = |app: &Application| {
            let mut values: Vec<_> = app.document().unwrap().composer.hitsounds().moments().iter()
                .filter(|moment| moment.has_hit(&lane))
                .map(|moment| (moment.time().ms(), moment.hit_volume(&lane))).collect();
            values.sort_by(|a, b| a.0.total_cmp(&b.0));
            values
        };
        let expected = |times: [f64; 2]| vec![(times[0], 40), (times[1], 70)];
        app.seek(500.0);
        app.key(KeyChord::new(Key::Space, Mods::ALT));
        assert_eq!(app.position_ms(), 1130.0);
        let mods = Mods::CTRL.union(Mods::ALT);
        app.key(KeyChord::new(Key::Right, mods));
        assert_eq!(state(&app), expected(next), "{mode:?}");
        app.key(KeyChord::new(Key::Left, mods));
        assert_eq!(state(&app), expected(previous), "{mode:?}");
        if mode == SnapMode::Objects {
            app.key(KeyChord::new(Key::Left, mods));
            assert_eq!(state(&app), expected(previous), "there is no earlier object");
        }
        app.command("undo");
        assert_eq!(state(&app), expected(next));
        app.command("undo");
        assert_eq!(state(&app), expected([1130.0, 1300.0]));
        assert_eq!(std::fs::read(&map).unwrap(), original);
    }
}

#[test]
fn transport_plays_numbered_map_additions_alongside_default_hitnormal() {
    let dir = Scratch::new("addition-playback");
    let folder = dir.path("mapset");
    let defaults = dir.path("skin");
    for path in [&folder, &defaults] {
        std::fs::create_dir(path).unwrap();
    }
    let map = folder.join("test.osu");
    let fixture = include_str!("../../tests/project-fixture.osu")
        .replace("256,192,1000,1,8,0:0:0:0:", "256,192,1000,1,14,1:2:2:50:");
    std::fs::write(&map, &fixture).unwrap();
    let header = b"RIFF\x26\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data\x02\0\0\0";
    for (path, amplitude) in [
        (defaults.join("normal-hitnormal.wav"), 4096i16),
        (folder.join("soft-hitwhistle2.wav"), 8192),
        (folder.join("soft-hitfinish2.wav"), 12288),
        (folder.join("soft-hitclap2.wav"), 16384),
    ] {
        let mut wav = header.to_vec();
        wav.extend(amplitude.to_le_bytes());
        std::fs::write(path, wav).unwrap();
    }
    let loaded = load(&map, &defaults, &std::env::temp_dir().join("dawsu-headless"), &AtomicBool::new(false), None, None, None).unwrap();
    let mut app = Application::headless();
    app.document = Some(loaded.document);
    app.samples = loaded.samples;
    let mix = |app: &Application| {
        let mut mixer = dawsu_audio::Mixer::new(48_000);
        mixer.song = app.song.clone();
        mixer.song_gain = app.playback_song_gain();
        mixer.cues = app.scheduled_sounds(app.document().unwrap());
        mixer.seek(1000.0);
        mixer.playing = true;
        let mut frame = [0.0];
        mixer.render(&mut frame, 1);
        (mixer.cues.len(), frame[0])
    };
    assert_eq!(mix(&app), (4, 0.625));
    let clap = SampleIdentity::Standard {
        set: SampleSet::Soft,
        sound: Sound::Hit(Hitsound::Clap),
        index: dawsu_foundation::CustomIndex::new(2),
    };
    let clip = app.clip_for_lane(&clap).unwrap();
    assert_eq!(clip.samples[0], 0.5);
    app.document.as_mut().unwrap().composer.listen_muted.push(HitKey {
        moment: 0,
        lane: clap.clone(),
    });
    assert_eq!(mix(&app), (3, 0.375));

    app.document.as_mut().unwrap().composer.listen_muted.clear();
    app.song = Some(Arc::new(AudioClip {
        samples: vec![0.25; 48_002],
        channels: 1,
        sample_rate: 48_000,
    }));
    app.preferences.song_gain = 1.0;
    assert_eq!(mix(&app), (4, 0.875));
    app.set_song_solo(true);
    assert_eq!(mix(&app), (0, 0.25));
    app.solo(clap.clone());
    assert_eq!(mix(&app), (1, 0.5));
    app.preferences.song_muted = true;
    assert_eq!(mix(&app), (1, 0.25), "explicit song mute takes priority");
    app.preferences.song_muted = false;
    app.set_song_solo(false);
    assert_eq!(mix(&app), (1, 0.25), "lane solo suppresses the unsoloed song");
    app.solo(clap);
    assert_eq!(mix(&app), (4, 0.875));
    assert!(!app.is_dirty());
    assert_eq!(std::fs::read(map).unwrap(), fixture.as_bytes());
}

#[test]
fn time_signatures_change_editor_bars_without_resnapping_or_writing_map_bytes() {
    let scratch = Scratch::new("editor-meter");
    let map = scratch.path("Hard.osu");
    let sessions = scratch.path("sessions");
    let original = b"osu file format v14\r\n\r\n[General]\r\nMode:0\r\n\r\n[TimingPoints]\r\n0,500,4,1,1,100,1,0\r\n4000,250,4,1,1,100,1,0\r\n\r\n[HitObjects]\r\n256,192,2000,1,0,0:0:0:0:\r\n";
    std::fs::write(&map, original).unwrap();
    let mut app = Application::headless();
    app.project_root = Some(sessions.clone());
    open(&mut app, map.clone());
    app.set_time_signatures(vec![EditorTimeSignature { bar: 2, numerator: 7, denominator: 8 }]).unwrap();
    let document = app.document().unwrap();
    let point = SongTime::from_ms(3750.0);
    let (bar, meter) = document.editor_bar_at(point);
    assert_eq!((bar.bar, bar.beat, meter), (3, 1, (7, 8)));
    let (bar_at_red, red_meter) = document.editor_bar_at(SongTime::from_ms(4000.0));
    assert_eq!((bar_at_red.bar, bar_at_red.beat, red_meter), (4, 1, (7, 8)));
    assert_eq!(document.composer.tempo.snap(point, SnapSpacing::new(1, 1).unwrap()).ms(), 4000.0);
    assert!(!app.is_dirty());
    app.save_session().unwrap();
    let mut reopened = Application::headless();
    reopened.project_root = Some(sessions);
    open(&mut reopened, map.clone());
    assert_eq!(reopened.document().unwrap().editor_bar_at(point), (bar, meter));
    assert_eq!(std::fs::read(&map).unwrap(), original);
}

#[test]
fn hs_mode_is_restored_per_difficulty_without_writing_map_bytes() {
    let fixture = Scratch::new("hs-mode-state");
    let a = fixture.path("Hard.osu");
    let b = fixture.path("Easy.osu");
    let original = include_bytes!("../../tests/project-fixture.osu");
    for path in [&a, &b] {
        std::fs::write(path, original).unwrap();
    }
    let mut app = Application::headless();
    app.project_root = Some(fixture.path("sessions"));
    open(&mut app, a.clone());
    app.set_hs_mode(true);
    assert!(!app.is_dirty());
    open(&mut app, b.clone());
    assert!(!app.document().unwrap().hs_mode);
    open(&mut app, a.clone());
    assert!(app.document().unwrap().hs_mode);
    app.set_hs_mode(false);
    app.save_session().unwrap();
    let mut reopened = Application::headless();
    reopened.project_root = app.project_root.clone();
    open(&mut reopened, a.clone());
    assert!(!reopened.document().unwrap().hs_mode);
    for path in [a, b] {
        assert_eq!(std::fs::read(path).unwrap(), original);
    }
}

#[test]
fn new_hitsound_difficulty_opens_with_hs_mode_enabled_by_default() {
    let fixture = Scratch::new("new-diff-hs-mode");
    let source_path = fixture.path("Artist - Title [Hard].osu");
    let original = include_bytes!("../../tests/project-fixture.osu");
    std::fs::write(&source_path, original).unwrap();
    let sessions = fixture.path("sessions");

    let mut app = Application::headless();
    app.project_root = Some(sessions.clone());
    open(&mut app, source_path);
    assert!(!app.document().unwrap().hs_mode);

    app.new_diff("Hitsounds".into());
    let wait = |app: &mut Application| {
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.busy {
            app.poll();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(app.error.is_none(), "{:?}", app.error);
    };
    wait(&mut app);
    let plan = app.output_review.as_ref().expect("new_diff prepares write plan");
    let created = plan.files[0].path.clone();
    app.apply_output();
    wait(&mut app);
    assert!(created.exists());

    let mut reopened = Application::headless();
    reopened.project_root = Some(sessions);
    open(&mut reopened, created);
    assert!(
        reopened.document().unwrap().hs_mode,
        "new hitsound difficulty should open with hitsounding mode enabled by default"
    );
}

#[test]
fn reopened_hs_ownership_removes_only_created_circles_and_survives_undo() {
    let scratch = Scratch::new("hs-circle-ownership");
    let folder = scratch.path("mapset");
    std::fs::create_dir(&folder).unwrap();
    let map = folder.join("test.osu");
    let original = include_bytes!("../../tests/project-fixture.osu");
    std::fs::write(&map, original).unwrap();
    let lane = SampleIdentity::File("voice.wav".into());
    let wav = b"RIFF\x26\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data\x02\0\0\0\0\x40";
    std::fs::write(folder.join("voice.wav"), wav).unwrap();

    let launch = || {
        let mut app = Application::headless();
        app.project_root = Some(scratch.path("projects"));
        open(&mut app, map.clone());
        app
    };
    let wait = |app: &mut Application| {
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.busy {
            app.poll();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(app.error.is_none(), "{:?}", app.error);
    };
    let save = |app: &mut Application| {
        app.save_with_backups(scratch.path("backups"));
        wait(app);
    };
    let mut app = launch();
    app.set_hs_mode(true);
    // It rounds to the original, also centered circle's file time.
    app.document.as_mut().unwrap().composer.add(1000.25, lane.clone(), 67, 0.0);
    save(&mut app);
    let saved = std::fs::read(&map).unwrap();
    assert_eq!(app.document().unwrap().file.hit_objects().len(), 2);

    let erase = |app: &mut Application| {
        let composer = &mut app.document.as_mut().unwrap().composer;
        let id = composer.hitsounds().moments().iter().position(|m| m.has_hit(&lane)).unwrap();
        composer.apply("Erase owned hit", |hits| hits.remove_hit(dawsu_hitsounding::MomentId(id), &lane));
    };
    let mut reopened = launch();
    erase(&mut reopened);
    let doc = reopened.document().unwrap();
    let (reviewed, problems, _) = compile_for_save(
        &doc.compile_base, doc.composer.hitsounds(), true, &doc.owned_hs_circles, true, &|| false, &mut |_, _| {},
    ).unwrap();
    assert!(problems.is_empty());
    assert_eq!(reviewed.to_bytes(), original);
    assert_eq!(std::fs::read(&map).unwrap(), saved);
    save(&mut reopened);
    assert_eq!(std::fs::read(&map).unwrap(), original);
    reopened.command("undo");
    save(&mut reopened);
    assert_eq!(std::fs::read(&map).unwrap(), saved);

    let mut external = saved.clone();
    external.extend(b"// This map was edited elsewhere\n");
    std::fs::write(&map, &external).unwrap();
    let mut stale = launch();
    erase(&mut stale);
    stale.save_with_backups(scratch.path("backups"));
    let deadline = Instant::now() + Duration::from_secs(10);
    while stale.busy {
        stale.poll();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    // The circle is not provably HS mode's, and the compiler cannot encode an empty gameplay
    // circle: Save leaves it out with its old hit, lists it, and writes the rest.
    assert!(stale.error.is_none(), "{:?}", stale.error);
    assert!(stale.save_problem_review());
    let problems = stale.output_problems();
    assert_eq!(problems.len(), 1);
    assert!(!problems[0].message.contains("sample mix"), "{}", problems[0].message);
    stale.answer_save_review(true);
    wait(&mut stale);
    assert!(stale.error.is_none(), "{:?}", stale.error);
    assert!(!stale.is_dirty());
    assert_eq!(std::fs::read(&map).unwrap(), external);
    assert_eq!(stale.document().unwrap().file.hit_objects().len(), 2);
    assert_eq!(std::fs::read(folder.join("voice.wav")).unwrap(), wav);
}

#[test]
fn hs_save_mix_reopens_without_duplicate_project_hits() {
    let scratch = Scratch::new("hs-save-mix");
    let folder = scratch.path("mapset");
    std::fs::create_dir(&folder).unwrap();
    let map = folder.join("a.osu");
    let original = include_bytes!("../../tests/project-fixture.osu");
    std::fs::write(&map, original).unwrap();
    let wave_header = b"RIFF\x28\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data\x04\0\0\0";
    for (name, frames) in [
        ("normal-hitnormal2.wav", [8192i16, 0]),
        ("soft-hitclap3.wav", [0, 8192i16]),
    ] {
        let mut wav = wave_header.to_vec();
        wav.extend(frames.iter().flat_map(|value| value.to_le_bytes()));
        std::fs::write(folder.join(name), wav).unwrap();
    }
    let mut app = Application::headless();
    app.project_root = Some(scratch.path("projects"));
    open(&mut app, map.clone());
    let composer = &mut app.document.as_mut().unwrap().composer;
    // Distinct unanchored moments round to one file time, with two different gains.
    composer.add(10_000.0, SampleIdentity::File("normal-hitnormal2.wav".into()), 80, 0.0);
    composer.add(10_000.25, SampleIdentity::File("soft-hitclap3.wav".into()), 40, 0.0);
    // Save the loose hits once so the HS save must clear the now-materialised project rows.
    let save = |app: &mut Application| {
        app.save_with_backups(scratch.path("backups"));
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.busy {
            app.poll();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(app.error.is_none(), "{:?}", app.error);
    };
    save(&mut app);
    assert_eq!(std::fs::read(&map).unwrap(), original);
    app.set_hs_mode(true);
    save(&mut app);
    assert!(!app.is_dirty());
    let saved = std::fs::read(&map).unwrap();
    assert!(saved.starts_with(original));
    let restart = |app: &mut Application| {
        app.project_root = Some(scratch.path("projects"));
        open(app, map.clone());
    };
    let mut reopened = Application::headless();
    restart(&mut reopened);
    let document = reopened.document().unwrap();
    let hits = document.composer.hitsounds();
    assert_eq!(project_hits(document).len(), 0);
    let [moment] = hits.moments().iter()
        .filter(|moment| moment.time().ms() >= 10_000.0)
        .collect::<Vec<_>>()[..] else {
        panic!("expected exactly one saved circle");
    };
    // Both sample files play as the hitnormal, so Save mixes them into one hitnormal file at
    // the louder one's volume.
    let [SampleIdentity::Standard { set: SampleSet::Normal, sound: Sound::Hit(Hitsound::Normal), index }] = moment.hits().collect::<Vec<_>>()[..] else {
        panic!("expected one hitnormal");
    };
    assert_eq!(moment.volume(), 80);
    let mix = dawsu_audio::decode(&folder.join(format!("normal-hitnormal{}.wav", index.get()))).unwrap();
    assert_eq!(mix.samples.len(), 4);
    for (actual, expected) in mix.samples.iter().zip([0.25, 0.25, 0.125, 0.125]) {
        assert!((actual - expected).abs() < 0.0001, "{actual} vs {expected}");
    }
    save(&mut app);
    save(&mut reopened);
    assert_eq!(std::fs::read(&map).unwrap(), saved);
    assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 4);
}


#[test]
fn loose_lanes_reopen_as_one_moment_beside_an_unrelated_object() {
    let dir = Scratch::new("grouped-project-hits");
    let path = dir.path("map.osu");
    std::fs::write(&path, include_bytes!("../../tests/project-fixture.osu")).unwrap();
    let mut doc = Document::read(&path).unwrap();
    let rows = ["Skin|soft-hitclap", "Skin|soft-hitwhistle"].map(|name| project::ProjectHit {
        lane: name.into(),
        time: 1000.0,
        volume: Some(75),
        group: None,
        default_volume: None,
    });
    restore_project_hits(&mut doc.composer, &rows).unwrap();
    let grouped = project_hits(&doc);
    assert_eq!(grouped.len(), 2);
    assert_eq!(grouped[0].group, grouped[1].group);
    let mut reopened = Document::read(&path).unwrap();
    restore_project_hits(&mut reopened.composer, &grouped).unwrap();
    let loose: Vec<_> = reopened
        .composer
        .hitsounds()
        .moments()
        .iter()
        .filter(|m| m.place() == dawsu_hitsounding::Place::NoObject)
        .collect();
    assert_eq!(loose.len(), 1);
    assert_eq!(loose[0].hits().count(), 2);
}

#[test]
fn legacy_clap_survives_edit_save_reopen_and_deletion() {
    let dir = Scratch::new("legacy-clap");
    let map = dir.path("a.osu");
    let original = include_bytes!("../../tests/project-fixture.osu");
    std::fs::write(&map, original).unwrap();
    let path = map.canonicalize().unwrap();
    let legacy = dir.path("legacy");
    let root = dir.path("sessions");
    std::fs::create_dir(&legacy).unwrap();
    let key = path
        .to_string_lossy()
        .trim_start_matches(r"\\?\")
        .to_lowercase();
    let hash = dawsu_foundation::digest::sha256_hex(key.as_bytes());
    let source = legacy.join(format!("A-{}.json", hash[..12].to_uppercase()));
    let bytes =
        br#"{"Version":3,"Unanchored":[{"Lane":"Skin|soft-hitclap","Time":250,"Volume":63}]}"#;
    std::fs::write(&source, bytes).unwrap();
    let load_map = || {
        load(
            &path,
            dir.dir(),
            &root,
            &AtomicBool::new(false),
            Some(&root),
            Some(&legacy),
            None,
        )
        .unwrap()
    };
    let mut app = Application::headless();
    let loaded = load_map();
    app.project = loaded.project;
    app.document = Some(loaded.document);
    let hit = project_hits(app.document().unwrap()).pop().unwrap();
    assert_eq!(
        (hit.lane.as_str(), hit.time, hit.volume),
        ("Skin|soft-hitclap", 250.0, Some(63))
    );
    assert!(!app.is_dirty());
    let c = &mut app.document.as_mut().unwrap().composer;
    let last = c.hitsounds().moments().len() - 1;
    let lane = c.hitsounds().moments()[last].hits().next().unwrap().clone();
    c.selected.replace([HitKey { moment: last, lane }]);
    app.selection_volume(77, false);
    app.save();
    let wait = |app: &mut Application| {
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.busy {
            app.poll();
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert!(app.error.is_none(), "{:?}", app.error);
    };
    wait(&mut app);
    assert_eq!(project_hits(&load_map().document)[0].volume, Some(77));
    app.document.as_mut().unwrap().composer.remove_selected();
    app.save();
    wait(&mut app);
    assert!(project_hits(&load_map().document).is_empty());
    assert_eq!(std::fs::read(source).unwrap(), bytes);
    assert_eq!(std::fs::read(map).unwrap(), original);
}

#[test]
fn session_conflict_keeps_the_outgoing_map_until_explicit_revert() {
    let fixture = Scratch::new("switch-conflict");
    let map = fixture.path("a.osu");
    let other = fixture.path("b.osu");
    let root = fixture.path("sessions");
    for path in [&map, &other] {
        std::fs::write(path, include_bytes!("../../tests/project-fixture.osu")).unwrap();
    }
    let mut app = Application::headless();
    app.project_root = Some(root.clone());
    open(&mut app, map.clone());
    app.song_offset = 5.0;
    app.save_session().unwrap();
    let mut external =
        project::Project::load(&root, None, &map.canonicalize().unwrap()).unwrap();
    external
        .save(project::SessionSettings {
            song_offset: 15.0,
            ..external.settings.clone()
        })
        .unwrap();
    app.song_offset = 10.0;
    app.request_open(other);
    assert!(!app.busy);
    assert_eq!(app.document().unwrap().path, map.canonicalize().unwrap());
    assert_eq!(app.song_offset, 10.0);
    assert!(app.error.take().unwrap().contains("changed on disk"));
    app.revert_requested = true;
    app.confirm_revert();
    let deadline = Instant::now() + Duration::from_secs(10);
    while app.busy {
        app.poll();
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    assert_eq!(app.song_offset, 15.0);
}

#[test]
fn difficulty_switch_protects_inspector_edits() {
    let fixture = Scratch::new("switch-inspector");
    let a = fixture.path("a.osu");
    let b = fixture.path("b.osu");
    for path in [&a, &b] {
        std::fs::write(path, include_bytes!("../../tests/project-fixture.osu")).unwrap();
    }
    let mut app = Application::headless();
    open(&mut app, a);
    app.document.as_mut().unwrap().composer.select_all();
    app.request_open(b);
    assert!(app.busy);
    app.selection_volume(7, false);
    assert!(!app.is_dirty(), "the inspector edited an outgoing document");
}

#[test]
fn reverting_reloads_disk_and_requires_an_explicit_confirm() {
    let fixture = Scratch::new("revert-workflow");
    let path = fixture.path("map.osu");
    let data = include_bytes!("../../tests/project-fixture.osu");
    std::fs::write(&path, data).unwrap();
    let mut app = Application::headless();
    open(&mut app, path.clone());
    app.document.as_mut().unwrap().composer.select_all();
    app.command("volumeDown");
    assert!(app.is_dirty());
    app.command("revert");
    assert!(app.revert_requested);
    assert!(app.is_dirty());
    app.confirm_revert();
    let deadline = Instant::now() + Duration::from_secs(10);
    while app.busy {
        app.poll();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(app.error.is_none(), "{:?}", app.error);
    assert!(!app.is_dirty());
    assert_eq!(std::fs::read(&path).unwrap(), data);
}

#[test]
fn reopening_a_difficulty_restores_its_session_without_changing_the_map() {
    let fixture = Scratch::new("project-workflow");
    let map = fixture.path("map.osu");
    let other = fixture.path("other.osu");
    let data = include_bytes!("../../tests/project-fixture.osu");
    std::fs::write(&map, data).unwrap();
    std::fs::write(&other, data).unwrap();
    let mut app = Application::headless();
    app.project_root = Some(fixture.path("application-data"));
    open(&mut app, map.clone());
    app.song_offset = -42.5;
    app.locator_left = 100.0;
    app.locator_right = 900.0;
    app.cycle = true;
    app.zoom = 0.35;
    app.set_view_start(125.0);
    open(&mut app, other);
    assert_eq!(app.song_offset, DEFAULT_SONG_OFFSET_MS);
    open(&mut app, map.clone());
    assert_eq!(app.song_offset, -42.5);
    assert_eq!(
        (app.locator_left, app.locator_right, app.cycle),
        (100.0, 900.0, true)
    );
    assert_eq!(app.zoom, 0.35);
    assert_eq!(app.document().unwrap().composer.view_start, 125.0);
    assert_eq!(std::fs::read(&map).unwrap(), data);
}

#[test]
fn protect_hitnormals_is_saved_like_the_other_preferences() {
    let fixture = Scratch::new("protect-normals-preference");
    let data = fixture.path("data");
    let mut app = Application::create(Preferences::default(), false, None, None, true, data.clone());
    app.set_normal_protection(true);
    assert!(app.error.is_none(), "{:?}", app.error);
    let (saved, _) = preferences::load(&data.join("preferences.json"), None);
    assert!(saved.protect_hitnormals);
}

#[test]
fn a_snap_spacing_osu_cannot_use_is_reported_and_changes_nothing() {
    let fixture = Scratch::new("mixed-snap");
    let map = fixture.path("map.osu");
    std::fs::write(&map, include_bytes!("../../tests/project-fixture.osu")).unwrap();
    let mut app = Application::headless();
    open(&mut app, map);
    let spacing = |app: &Application| {
        let composer = &app.document().unwrap().composer;
        (composer.snap_enabled, composer.spacing.beats(), composer.spacing.divisions())
    };
    let before = spacing(&app);
    app.set_snap_options(!before.0, 2, 3, dawsu_editor::tempo::SnapMode::Grid);
    assert_eq!(
        app.error.take().as_deref(),
        Some("Snap 2/3 is not available. Use a fraction of a beat, like 1/4, or whole beats, like 2/1.")
    );
    assert_eq!(spacing(&app), before);
    app.set_snap_options(true, 1, 3, dawsu_editor::tempo::SnapMode::Grid);
    assert!(app.error.is_none(), "{:?}", app.error);
    assert_eq!(spacing(&app), (true, 1, 3));
}

/// P and Alt+P (Cubase's Locators to Selection and Loop Selection) were in the key list but
/// did nothing: no command ran for them.
#[test]
fn locators_follow_the_selected_hits_or_the_range() {
    let fixture = Scratch::new("locators-to-selection");
    let path = fixture.path("map.osu");
    let map = b"osu file format v14\n[General]\nMode:0\n[TimingPoints]\n0,500,4,1,0,100,1,0\n[HitObjects]\n256,192,1000,1,8,0:0:0:0:\n256,192,2500,1,8,0:0:0:0:\n";
    std::fs::write(&path, map).unwrap();
    let mut app = Application::headless();
    app.project_root = Some(fixture.path("sessions"));
    open(&mut app, path.clone());
    app.locator_left = 100.0;
    app.locator_right = 200.0;
    let locators = |app: &Application| (app.locator_left, app.locator_right, app.cycle);

    // Nothing selected and no range: the locators stay where they are.
    app.command("locatorsToSelection");
    assert!(app.error.is_none(), "{:?}", app.error);
    assert_eq!(locators(&app), (100.0, 200.0, false));

    let clap = |moment: usize| {
        let moments = app.document().unwrap().composer.hitsounds().moments();
        let lane = moments[moment].hits().find(|lane| matches!(lane,
            SampleIdentity::Standard { sound: Sound::Hit(Hitsound::Clap), .. })).unwrap().clone();
        HitKey { moment, lane }
    };
    let (first, second) = (clap(0), clap(1));
    app.select_hit(first.clone(), false);
    app.select_hit(second, true);
    app.command("locatorsToSelection");
    assert_eq!(locators(&app), (1000.0, 2500.0, false), "the span of the selected hits");

    // One hit has no length: R goes to the next grid line after it.
    app.select_hit(first, false);
    app.command("locatorsToSelection");
    let composer = &app.document().unwrap().composer;
    let next = composer.tempo.grid_step(SongTime::from_ms(1000.0), 1, composer.spacing).ms();
    assert!(next > 1000.0);
    assert_eq!(locators(&app), (1000.0, next, false));

    // A range left from the Range tool does not beat the hits selected since.
    app.document.as_mut().unwrap().composer.range = Some(dawsu_editor::tools::TimeRange {
        from: SongTime::from_ms(1750.0), to: SongTime::from_ms(1250.0), from_row: 0, to_row: 1,
    });
    app.command("locatorsToSelection");
    assert_eq!(locators(&app), (1000.0, next, false));
    // With the Range tool in hand, its range wins.
    app.command("toolRange");
    app.command("locatorsToSelection");
    assert_eq!(locators(&app), (1250.0, 1750.0, false));
    app.command("toolSelect");

    // Alt+P: the same, with Cycle on and playback from L.
    app.document.as_mut().unwrap().composer.range = None;
    app.command("playLocatorRange");
    assert!(app.error.is_none(), "{:?}", app.error);
    assert_eq!(locators(&app), (1000.0, next, true));
    assert_eq!(app.playback_start, 1000.0);
    assert_eq!(std::fs::read(&path).unwrap(), map, "the map file is untouched");
}

#[test]
fn fit_zoom_of_a_30_minute_song_survives_zoom_out_and_reopening() {
    let fixture = Scratch::new("fit-zoom");
    let map = fixture.path("map.osu");
    let other = fixture.path("other.osu");
    let data = include_bytes!("../../tests/project-fixture.osu");
    std::fs::write(&map, data).unwrap();
    std::fs::write(&other, data).unwrap();
    let mut app = Application::headless();
    app.project_root = Some(fixture.path("application-data"));
    open(&mut app, map.clone());
    // Fit shows a 30-minute song in a 1000 px timeline.
    let fit = 1000.0 / (30.0 * 60_000.0);
    app.zoom = fit;
    app.command("zoomOut");
    assert!(app.zoom <= fit, "Zoom out zoomed in to {}", app.zoom);
    app.zoom = fit;
    open(&mut app, other);
    open(&mut app, map);
    assert_eq!(app.zoom, fit);
}

#[test]
fn opening_map_with_missing_drum_vst_keeps_unavailable_note_in_status() {
    let fixture = Scratch::new("missing-drum-vst-status");
    let map = fixture.path("map.osu");
    let sessions = fixture.path("sessions");
    let missing_vst = fixture.path("missing-drums.vst3");
    std::fs::write(&map, include_bytes!("../../tests/project-fixture.osu")).unwrap();

    let mut project = project::Project::load(&sessions, None, &map).unwrap();
    let mut settings = project.settings.clone();
    settings.drum_vst = Some(project::DrumVstSession {
        plugin_path: missing_vst.to_string_lossy().into_owned(),
        plugin_uid: None,
        enabled: true,
        state_b64: None,
    });
    project.save(settings).unwrap();

    let mut app = Application::headless();
    app.project_root = Some(sessions);
    open(&mut app, map);

    assert!(
        app.status.contains("Drum VST3 unavailable"),
        "expected Drum VST3 unavailable note in status, got: {:?}",
        app.status
    );
    assert!(
        app.status.starts_with("Difficulty loaded"),
        "expected base status before note, got: {:?}",
        app.status
    );
}

#[test]
fn gameplay_overlay_sorts_out_of_order_hs_no_object_moments_by_time() {
    let fixture = Scratch::new("gameplay-overlay-sorted");
    let map = fixture.path("map.osu");
    std::fs::write(
        &map,
        b"osu file format v14\n[General]\nMode:0\n[TimingPoints]\n0,500,4,1,0,100,1,0\n[HitObjects]\n256,192,5000,1,0,0:0:0:0:\n",
    )
    .unwrap();
    let mut app = Application::headless();
    open(&mut app, map);
    app.set_hs_mode(true);
    app.document
        .as_mut()
        .unwrap()
        .composer
        .add(500.0, skin(SampleSet::Normal, Hitsound::Clap), 80, 0.0);
    let overlay = presentation::gameplay_overlay(app.document().unwrap());
    let times: Vec<f64> = overlay.points.iter().map(|p| p.1).collect();
    assert_eq!(times, vec![500.0, 5000.0]);
}

#[test]
fn plan_audition_burst_deduplicates_resolved_samples_caps_and_normalizes_gain() {
    let fixture = Scratch::new("audition-burst");
    let map = fixture.path("burst.osu");
    std::fs::write(
        &map,
        b"osu file format v14\n[General]\nMode:0\n[TimingPoints]\n0,500,4,1,0,100,1,0\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n",
    )
    .unwrap();
    let mut app = Application::headless();
    open(&mut app, map);
    let clap = skin(SampleSet::Normal, Hitsound::Clap);

    // 1. 100 duplicate requests collapse to 1 voice at gain 1.0.
    let dup = vec![clap; 100];
    let plan = app.plan_audition_burst(&dup);
    assert_eq!(plan.len(), 1);
    assert!((plan[0].1 - 1.0).abs() < 1e-5);

    // 2. Two skin lanes resolving to the same `plays_as()` collapse to 1 voice at gain 1.0.
    let doc = app.document.as_mut().unwrap();
    let normal0 = skin(SampleSet::Normal, Hitsound::Normal);
    let normal2 = SampleIdentity::Standard {
        set: SampleSet::Normal,
        sound: Sound::Hit(Hitsound::Normal),
        index: dawsu_foundation::CustomIndex::new(2),
    };
    doc.composer.add(1000.0, normal2.clone(), 80, 5.0);
    let shared_plan = app.plan_audition_burst(&[normal0, normal2]);
    assert_eq!(shared_plan.len(), 1);
    assert!((shared_plan[0].1 - 1.0).abs() < 1e-5);

    // 3. 12 distinct lanes cap at 4 voices with 1/sqrt(4) = 0.5 gain each.
    let many: Vec<_> = (1..=12)
        .map(|i| SampleIdentity::File(format!("s{i}.wav")))
        .collect();
    let capped = app.plan_audition_burst(&many);
    assert_eq!(capped.len(), 4);
    for (_, gain) in capped {
        assert!((gain - 0.5).abs() < 1e-5, "expected gain 0.5, got {gain}");
    }
}

#[test]
fn moving_multiple_hits_between_lanes_respects_visual_order_in_samples_and_drums_view() {
    use dawsu_editor::keys::{Key, KeyChord, Mods};
    use dawsu_editor::tools::PointerInput;

    let fixture = Scratch::new("move-lanes-visual");
    let map = fixture.path("map.osu");
    std::fs::write(
        &map,
        b"osu file format v14\n[General]\nMode:0\n[TimingPoints]\n0,500,4,1,0,100,1,0\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n",
    )
    .unwrap();
    let mut app = Application::headless();
    open(&mut app, map);
    let doc_path = app.document().unwrap().path.clone();
    let lanes = app.document().unwrap().composer.lanes.clone();
    assert!(lanes.len() >= 4);
    let lane3 = lanes[3].clone();
    let lane1 = lanes[1].clone();
    let lane2 = lanes[2].clone();
    {
        let doc = app.document.as_mut().unwrap();
        doc.composer.add(1000.0, lane3.clone(), 80, 5.0);
        doc.composer.add(1500.0, lane3.clone(), 80, 5.0);
        doc.composer.selected.replace([
            HitKey { moment: 0, lane: lane3.clone() },
            HitKey { moment: 1, lane: lane3.clone() },
        ]);
    }
    let vis = [3usize, 1, 0, 2];

    // 1. Samples view mouse drag down from lane 3 to empty visual neighbor lane 1 (raw delta -2, visual delta +1).
    app.pointer_in_view(PointerPhase::Down, PointerInput::at(1000.0, 3), None, Some(&vis));
    app.pointer_in_view(PointerPhase::Move, PointerInput::at(1000.0, 1), None, Some(&vis));
    app.pointer_in_view(PointerPhase::Up, PointerInput::at(1000.0, 1), None, Some(&vis));
    {
        let doc = app.document().unwrap();
        assert_eq!(doc.composer.selected.items().len(), 2);
        assert!(doc.composer.selected.items().iter().all(|k| k.lane == lane1));
        assert!(doc.composer.hitsounds().moments()[0].has_hit(&lane1));
        assert!(doc.composer.hitsounds().moments()[1].has_hit(&lane1));
    }

    // 2. Samples view navDown with key_with_lane_view moves those 2 hits cleanly onto lane 0 (index 2 in vis).
    let lane0 = lanes[0].clone();
    app.key_with_lane_view(KeyChord::new(Key::Down, Mods::NONE), doc_path.clone(), vis.to_vec(), 5000.0);
    {
        let doc = app.document().unwrap();
        assert_eq!(doc.composer.selected.items().len(), 2);
        assert!(doc.composer.selected.items().iter().all(|k| k.lane == lane0));
    }

    // 3. Drums view navDown moves selected hits across drum groups via DrumRowTarget.
    app.set_drum_row_groups(Some(crate::drum_view::DrumRowGroups {
        groups: vec![vec![0], vec![1, 2]],
        group_of_lane: std::collections::HashMap::from([(0, 0), (1, 1), (2, 1)]),
    }));
    app.key_with_lane_view(KeyChord::new(Key::Down, Mods::NONE), doc_path, vec![0, 1], 5000.0);
    let doc = app.document().unwrap();
    assert_eq!(doc.composer.selected.items().len(), 2);
    assert!(
        doc.composer
            .selected
            .items()
            .iter()
            .all(|k| k.lane == lane1 || k.lane == lane2),
        "Drums view navDown should move selected hits to the next visible drum group"
    );
}
