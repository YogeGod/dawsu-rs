use crate::*;
use dawsu_test_support::Scratch;
use std::time::{Duration, Instant};

fn test_wav(frames: u32) -> Vec<u8> {
    let data = frames * 2;
    let mut wav = b"RIFF".to_vec();
    wav.extend((36 + data).to_le_bytes());
    wav.extend(b"WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data");
    wav.extend(data.to_le_bytes());
    for i in 0..frames {
        let sample = (((i % 100) as f32 / 100.0) * 16_000.0) as i16;
        wav.extend(sample.to_le_bytes());
    }
    wav
}

fn wait_open(app: &mut Application) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while app.busy {
        app.poll();
        assert!(Instant::now() < deadline, "open job timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(app.error.is_none(), "{:?}", app.error);
}

fn wait_song(app: &mut Application) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while app.song_loading {
        app.poll();
        assert!(Instant::now() < deadline, "song decode job timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn document_is_open_and_editable_before_song_ready_and_pending_play_starts_on_arrival() {
    let scratch = Scratch::new("async-song-ready");
    let map = scratch.path("Map.osu");
    std::fs::write(
        &map,
        b"osu file format v14\n[General]\nMode:0\nAudioFilename: song.wav\n[TimingPoints]\n0,500,4,1,1,100,1,0\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n",
    )
    .unwrap();
    // 4 seconds of audio so song duration (4000 ms) exceeds document duration (3000 ms).
    std::fs::write(scratch.path("song.wav"), test_wav(48_000 * 4)).unwrap();

    let hold = Arc::new(AtomicBool::new(true));
    let mut app = Application::headless();
    app.song_decode_hold = Some(hold.clone());
    app.request_open(map);
    wait_open(&mut app);

    assert!(app.document().is_some());
    assert!(!app.busy);
    assert!(app.song_loading);
    assert!(app.song.is_none());
    assert!(app.waveform.is_empty());
    assert_eq!(app.duration_ms(), 3000.0);

    // Edits work immediately while the song is still decoding.
    let hit_lane = app
        .document()
        .unwrap()
        .composer
        .hitsounds()
        .moments()[0]
        .hits()
        .next()
        .unwrap()
        .clone();
    app.select_hit(HitKey { moment: 0, lane: hit_lane }, false);
    app.selection_volume(55, false);
    assert!(app.is_dirty());
    assert_eq!(
        app.document().unwrap().composer.hitsounds().moments()[0].volume(),
        55
    );

    // Pressing Play while song_loading queues playback rather than starting out of sync.
    app.play_pause_from(Some(250.0));
    assert!(app.pending_play_on_song);
    assert_eq!(app.started_play_count, 0);
    assert_eq!(app.status, "Loading song…");
    assert_eq!(app.position_ms(), 250.0);

    let rev_before = app.waveform_revision();
    hold.store(false, Ordering::Relaxed);
    wait_song(&mut app);

    assert!(!app.song_loading);
    assert!(app.song.is_some());
    assert!(!app.waveform.is_empty());
    assert!(app.waveform_revision() > rev_before);
    assert!(!app.pending_play_on_song);
    assert_eq!(app.started_play_count, 1);
    assert!((app.duration_ms() - 4000.0).abs() < 1.0);
    assert!((app.locator_right - 4000.0).abs() < 1.0);
    assert!(app.is_dirty());
}

#[test]
fn opening_a_second_map_cancels_first_decode_and_drops_late_song_ready() {
    let scratch = Scratch::new("async-song-generation");
    let first = scratch.path("First.osu");
    let second = scratch.path("Second.osu");
    std::fs::write(
        &first,
        b"osu file format v14\n[General]\nMode:0\nAudioFilename: song1.wav\n[TimingPoints]\n0,500,4,1,1,100,1,0\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n",
    )
    .unwrap();
    std::fs::write(
        &second,
        b"osu file format v14\n[General]\nMode:0\nAudioFilename: song2.wav\n[TimingPoints]\n0,500,4,1,1,100,1,0\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n",
    )
    .unwrap();
    std::fs::write(scratch.path("song1.wav"), test_wav(48_000)).unwrap();
    std::fs::write(scratch.path("song2.wav"), test_wav(48_000 * 2)).unwrap();

    let hold = Arc::new(AtomicBool::new(true));
    let mut app = Application::headless();
    app.song_decode_hold = Some(hold.clone());
    app.request_open(first);
    wait_open(&mut app);
    let gen1 = app.document_generation();
    let first_cancel = app.song_cancel.clone();
    assert!(!first_cancel.load(Ordering::Relaxed));

    // Release hold as we switch to the second difficulty.
    hold.store(false, Ordering::Relaxed);
    app.request_open(second);
    assert!(first_cancel.load(Ordering::Relaxed), "switching maps cancels old song decode");
    wait_open(&mut app);
    wait_song(&mut app);

    let gen2 = app.document_generation();
    assert_ne!(gen1, gen2);
    let second_wf_len = app.waveform.len();
    let second_rev = app.waveform_revision();
    assert!((app.duration_ms() - 2000.0).abs() < 1.0);

    // Inject a stale SongReady from the first generation and ensure it is dropped cleanly.
    app.pending_play_on_song = false;
    let stale_clip = Arc::new(AudioClip {
        samples: vec![0.5; 48_000 * 5],
        channels: 1,
        sample_rate: 48_000,
    });
    app.apply_song_job_result(JobResult::SongReady {
        generation: gen1,
        song: Some(stale_clip.clone()),
        waveform: vec![WaveBin { peak: 0.9, low: 0.1, mid: 0.2, high: 0.3, amp_l: 0.9, amp_r: 0.9 }; 3],
        warning: Some("stale warning".into()),
    });
    assert_eq!(app.waveform.len(), second_wf_len);
    assert_eq!(app.waveform_revision(), second_rev);
    assert!(!Arc::ptr_eq(app.song.as_ref().unwrap(), &stale_clip));
    assert!(app.warnings.is_empty());
}

#[test]
fn play_from_while_song_loading_waits_and_stop_clears_pending_play() {
    let scratch = Scratch::new("async-song-stop");
    let map = scratch.path("Map.osu");
    std::fs::write(
        &map,
        b"osu file format v14\n[General]\nMode:0\nAudioFilename: song.wav\n[TimingPoints]\n0,500,4,1,1,100,1,0\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n",
    )
    .unwrap();
    std::fs::write(scratch.path("song.wav"), test_wav(48_000 * 2)).unwrap();

    let hold = Arc::new(AtomicBool::new(true));
    let mut app = Application::headless();
    app.song_decode_hold = Some(hold.clone());
    app.request_open(map);
    wait_open(&mut app);

    app.play_from(400.0);
    assert!(app.pending_play_on_song);
    assert_eq!(app.status, "Loading song…");
    app.stop();
    assert!(!app.pending_play_on_song);

    hold.store(false, Ordering::Relaxed);
    wait_song(&mut app);
    assert_eq!(app.started_play_count, 0);
    assert_eq!(app.status, "Difficulty loaded");
}

#[test]
fn failed_drum_vst_load_and_restore_preserve_saved_session_and_preset() {
    let scratch = Scratch::new("vst-session-safety");
    let map = scratch.path("Map.osu");
    let root = scratch.path("sessions");
    std::fs::write(&map, include_bytes!("../../tests/project-fixture.osu")).unwrap();
    let canonical = map.canonicalize().unwrap();

    let saved_vst = project::DrumVstSession {
        plugin_path: "missing.vst3".into(),
        plugin_uid: Some("uid-1".into()),
        enabled: true,
        state_b64: Some("abc".into()),
    };
    let mut proj = project::Project::load(&root, None, &canonical).unwrap();
    proj.save(project::SessionSettings {
        drum_vst: Some(saved_vst.clone()),
        ..proj.settings.clone()
    })
    .unwrap();

    let mut app = Application::headless();
    app.project_root = Some(root.clone());
    app.request_open(map.clone());
    wait_open(&mut app);
    assert!(app.drum_vst.is_none());
    assert_eq!(
        project::Project::load(&root, None, &canonical)
            .unwrap()
            .settings
            .drum_vst,
        Some(saved_vst.clone())
    );

    // A failing replacement load must not wipe the saved Drum VST3 choice or preset.
    assert!(app.load_drum_vst(PathBuf::from("other-missing.vst3"), None).is_err());
    assert_eq!(
        project::Project::load(&root, None, &canonical)
            .unwrap()
            .settings
            .drum_vst,
        Some(saved_vst.clone())
    );

    // Saving other session changes while the Drum VST3 is unavailable keeps it intact.
    app.save_session().unwrap();
    assert_eq!(
        project::Project::load(&root, None, &canonical)
            .unwrap()
            .settings
            .drum_vst,
        Some(saved_vst.clone())
    );

    // Explicit unload still clears the saved Drum VST3 session.
    app.unload_drum_vst();
    assert!(
        project::Project::load(&root, None, &canonical)
            .unwrap()
            .settings
            .drum_vst
            .is_none()
    );
}

fn stored(app: &Application) -> project::SessionSettings {
    let path = &app.document().unwrap().path;
    project::Project::load(app.project_root.as_ref().unwrap(), None, path)
        .unwrap()
        .settings
}

fn wait_drum_vst(app: &mut Application) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while app.drum_vst_loading {
        app.poll();
        assert!(Instant::now() < deadline, "drum VST3 restore job timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn opening_map_with_saved_drum_vst_is_nonblocking_and_preserves_session_on_error() {
    let scratch = Scratch::new("async-drum-vst-open");
    let projects = scratch.path("projects");
    let map = scratch.path("Map.osu");
    std::fs::write(
        &map,
        b"osu file format v14\n[General]\nMode:0\n[TimingPoints]\n0,500,4,1,1,100,1,0\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n",
    )
    .unwrap();
    let plugin_path = scratch.path("DummyKit.vst3");
    std::fs::write(&plugin_path, b"dummy vst3").unwrap();

    let saved_vst = project::DrumVstSession {
        plugin_path: plugin_path.to_string_lossy().into_owned(),
        plugin_uid: Some("uid-kit-1".into()),
        enabled: true,
        state_b64: Some("AQID".into()),
    };
    let mut proj = project::Project::load(&projects, None, &map).unwrap();
    let mut settings = proj.settings.clone();
    settings.drum_vst = Some(saved_vst.clone());
    proj.save(settings).unwrap();

    let hold = Arc::new(AtomicBool::new(true));
    let mut app = Application::headless();
    app.project_root = Some(projects.clone());
    app.vst3 = vst3::Vst3Manager::with_search_dirs(Vec::new())
        .with_host_executable(scratch.path("missing-dawsu-vst3-host.exe"));
    app.drum_vst_restore_hold = Some(hold.clone());
    app.request_open(map.clone());
    wait_open(&mut app);

    // Map opens immediately and is editable while the sidecar restore is still running.
    assert!(app.document().is_some());
    assert!(!app.busy);
    assert!(app.drum_vst_loading);
    assert!(app.drum_vst.is_none());
    assert!(
        app.status.contains("Loading drum kit…"),
        "unexpected status while loading drum kit: {}",
        app.status
    );

    let hit_lane = app
        .document()
        .unwrap()
        .composer
        .hitsounds()
        .moments()[0]
        .hits()
        .next()
        .unwrap()
        .clone();
    app.select_hit(HitKey { moment: 0, lane: hit_lane }, false);
    app.selection_volume(60, false);
    assert!(app.is_dirty());

    // Saving while loading must not lose the saved Drum VST3 session.
    app.save_session().unwrap();
    assert_eq!(stored(&app).drum_vst.as_ref(), Some(&saved_vst));

    // Let the background restore finish (fails cleanly because sidecar isn't built in unit test).
    hold.store(false, Ordering::Relaxed);
    wait_drum_vst(&mut app);
    assert!(!app.drum_vst_loading);
    assert!(
        app.status.contains("Drum VST3 unavailable:"),
        "unexpected status after unavailable VST3: {}",
        app.status
    );
    app.save_session().unwrap();
    assert_eq!(stored(&app).drum_vst.as_ref(), Some(&saved_vst));

    // Missing plugin path reports unavailable immediately without spawning a background thread.
    std::fs::remove_file(&plugin_path).unwrap();
    let mut reopened = Application::headless();
    reopened.project_root = Some(projects);
    reopened.request_open(map);
    wait_open(&mut reopened);
    assert!(!reopened.drum_vst_loading);
    assert!(
        reopened.status.contains("Drum VST3 unavailable:"),
        "unexpected status for missing plugin file: {}",
        reopened.status
    );
    reopened.save_session().unwrap();
    assert_eq!(stored(&reopened).drum_vst.as_ref(), Some(&saved_vst));
}

#[test]
fn opening_second_map_cancels_first_drum_vst_restore_and_drops_stale_ready() {
    let scratch = Scratch::new("async-drum-vst-gen");
    let projects = scratch.path("projects");
    let first = scratch.path("First.osu");
    let second = scratch.path("Second.osu");
    std::fs::write(
        &first,
        b"osu file format v14\n[General]\nMode:0\n[TimingPoints]\n0,500,4,1,1,100,1,0\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n",
    )
    .unwrap();
    std::fs::write(
        &second,
        b"osu file format v14\n[General]\nMode:0\n[TimingPoints]\n0,500,4,1,1,100,1,0\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n",
    )
    .unwrap();
    let plugin_path = scratch.path("DummyKit.vst3");
    std::fs::write(&plugin_path, b"dummy vst3").unwrap();

    let saved_vst = project::DrumVstSession {
        plugin_path: plugin_path.to_string_lossy().into_owned(),
        plugin_uid: Some("uid-kit-1".into()),
        enabled: true,
        state_b64: None,
    };
    let mut proj = project::Project::load(&projects, None, &first).unwrap();
    let mut settings = proj.settings.clone();
    settings.drum_vst = Some(saved_vst.clone());
    proj.save(settings).unwrap();

    let hold = Arc::new(AtomicBool::new(true));
    let mut app = Application::headless();
    app.project_root = Some(projects);
    app.vst3 = vst3::Vst3Manager::with_search_dirs(Vec::new())
        .with_host_executable(scratch.path("missing-dawsu-vst3-host.exe"));
    app.drum_vst_restore_hold = Some(hold.clone());

    app.request_open(first);
    wait_open(&mut app);
    let gen1 = app.document_generation();
    let first_cancel = app.drum_vst_cancel.clone();
    assert!(app.drum_vst_loading);
    assert!(!first_cancel.load(Ordering::Relaxed));

    // Switch to the second map (which has no saved VST3).
    hold.store(false, Ordering::Relaxed);
    app.request_open(second);
    assert!(
        first_cancel.load(Ordering::Relaxed),
        "switching maps cancels old drum VST3 restore"
    );
    wait_open(&mut app);
    wait_drum_vst(&mut app);

    let gen2 = app.document_generation();
    assert_ne!(gen1, gen2);
    assert!(!app.drum_vst_loading);
    assert!(app.drum_vst.is_none());
    let status_before = app.status.clone();

    // Inject a stale DrumVstReady for gen1 and verify it is ignored cleanly.
    app.apply_drum_vst_job_result(JobResult::DrumVstReady {
        generation: gen1,
        saved: saved_vst,
        result: Err("stale vst error".into()),
    });
    assert_eq!(app.status, status_before);
    assert!(!app.drum_vst_loading);
    assert!(app.drum_vst.is_none());
    assert!(app.drum_vst_saved.is_none());
}
