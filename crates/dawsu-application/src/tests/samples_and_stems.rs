#[cfg(test)]
mod sample_file_clip {
    use crate::*;
    use dawsu_test_support::Scratch;

    fn wave(frames: u16) -> Vec<u8> {
        let data = frames as u32 * 2;
        let mut bytes = b"RIFF".to_vec();
        bytes.extend((36 + data).to_le_bytes());
        bytes.extend(b"WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data");
        bytes.extend(data.to_le_bytes());
        bytes.extend((0..frames).flat_map(|i| (i as i16 * 100).to_le_bytes()));
        bytes
    }

    #[test]
    fn a_library_sample_is_decoded_once_for_its_audition_and_its_outline_until_the_file_changes() {
        let scratch = Scratch::new("sample-file-clip");
        let path = scratch.path("clap.wav");
        std::fs::write(&path, wave(4)).unwrap();
        let mut app = Application::headless();
        app.audition_sample_file(&path).unwrap();
        let first = app.sample_file_clip(&path).unwrap();
        let (ms, _) = app.sample_file_outline(&path, 4).unwrap();
        assert!(Arc::ptr_eq(&first, &app.sample_file_clip(&path).unwrap()), "the outline reused the audition's clip");
        assert!((ms - 4000.0 / 48000.0).abs() < 1e-9);
        std::fs::write(&path, wave(8)).unwrap();
        let (ms, _) = app.sample_file_outline(&path, 4).unwrap();
        assert!((ms - 8000.0 / 48000.0).abs() < 1e-9, "a changed file is decoded again");
    }
}

#[cfg(test)]
mod local_default_samples {
    use crate::*;
    use dawsu_test_support::Scratch;

    #[test]
    fn starting_from_a_data_folder_takes_default_sounds_from_its_preferences_only() {
        let scratch = Scratch::new("data-root-defaults");
        let app = Application::with_data_root(scratch.path("data"));
        assert!(
            app.preferences.default_samples.as_os_str().is_empty(),
            "no folder outside the data folder: {}",
            app.preferences.default_samples.display()
        );
    }

    #[test]
    fn the_desktop_start_uses_the_local_default_sounds_only_when_none_are_saved() {
        let scratch = Scratch::new("local-defaults");
        let local = scratch.path("local");
        std::fs::create_dir(&local).unwrap();

        let mut app = Application::headless();
        app.use_local_default_samples(&scratch.path("missing"));
        assert!(app.preferences.default_samples.as_os_str().is_empty(), "a missing folder is not used");

        app.use_local_default_samples(&local);
        assert_eq!(app.preferences.default_samples, local);

        let saved = scratch.path("saved");
        app.preferences.default_samples = saved.clone();
        app.use_local_default_samples(&local);
        assert_eq!(app.preferences.default_samples, saved, "a saved folder is kept");
    }
}

#[cfg(test)]
mod shipped_default_sounds {
    use crate::*;
    use dawsu_foundation::CustomIndex;
    use dawsu_test_support::Scratch;

    fn skin(set: SampleSet, sound: Sound, index: u32) -> Lane {
        Lane { identity: SampleIdentity::Standard { set, sound, index: CustomIndex::new(index) }, source: LaneSource::Skin }
    }

    fn path_for(lane: &Lane, defaults: &Path) -> Result<PathBuf, String> {
        let folder = defaults.parent().unwrap();
        sample_path(lane, folder, &pending::Pending::default(), defaults, &pending::sample_folder(defaults))
    }

    #[test]
    fn a_default_sound_the_folder_lacks_plays_osus_own_shipped_sound() {
        let scratch = Scratch::new("shipped-defaults");
        let defaults = scratch.path("defaults");
        std::fs::create_dir(&defaults).unwrap();
        for (lane, name) in [
            (skin(SampleSet::Soft, Sound::Slider(SliderSound::Slide), 0), "soft-sliderslide.wav"),
            (skin(SampleSet::Drum, Sound::Slider(SliderSound::Tick), 3), "drum-slidertick.wav"),
            (skin(SampleSet::Normal, Sound::Slider(SliderSound::Whistle), 0), "normal-sliderwhistle.wav"),
            (skin(SampleSet::Normal, Sound::Hit(Hitsound::Clap), 0), "normal-hitclap.wav"),
        ] {
            let path = path_for(&lane, &defaults).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(path, default_sounds::path(name).unwrap(), "{name}");
            dawsu_audio::decode(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
        }
        // No folder chosen at all: still osu!'s sound.
        let none = scratch.path("not-chosen");
        let slide = skin(SampleSet::Soft, Sound::Slider(SliderSound::Slide), 0);
        assert_eq!(path_for(&slide, &none).unwrap(), default_sounds::path("soft-sliderslide.wav").unwrap());
    }

    #[test]
    fn the_chosen_default_sounds_folder_comes_before_the_shipped_sound() {
        let scratch = Scratch::new("shipped-defaults-order");
        let defaults = scratch.path("defaults");
        std::fs::create_dir(&defaults).unwrap();
        std::fs::write(defaults.join("Soft-SliderSlide.ogg"), b"OggS").unwrap();
        std::fs::write(defaults.join("drum-hitnormal.wav"), b"RIFF").unwrap();
        let slide = skin(SampleSet::Soft, Sound::Slider(SliderSound::Slide), 0);
        assert_eq!(path_for(&slide, &defaults).unwrap(), defaults.join("Soft-SliderSlide.ogg").canonicalize().unwrap());
        let kick = skin(SampleSet::Drum, Sound::Hit(Hitsound::Normal), 0);
        assert_eq!(path_for(&kick, &defaults).unwrap(), defaults.join("drum-hitnormal.wav").canonicalize().unwrap());
    }
}

#[cfg(test)]
mod drum_stem_monitor {
    use crate::*;
    use dawsu_test_support::Scratch;
    use std::time::{Duration, Instant};

    fn test_wav(frames: u32, freq_hz: f32) -> Vec<u8> {
        let data = frames * 2;
        let mut wav = b"RIFF".to_vec();
        wav.extend((36 + data).to_le_bytes());
        wav.extend(b"WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data");
        wav.extend(data.to_le_bytes());
        for i in 0..frames {
            let phase = 2.0 * std::f32::consts::PI * freq_hz * (i as f32 / 48_000.0);
            let sample = (phase.sin() * 18_000.0) as i16;
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

    fn wait_audio(app: &mut Application) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.song_loading || app.drum_stem_loading {
            app.poll();
            assert!(Instant::now() < deadline, "audio decode job timed out");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn headless_with_projects(data_root: PathBuf) -> Application {
        let mut app = Application::headless();
        app.project_root = Some(data_root.join("projects"));
        app.data_root = data_root;
        app
    }

    #[test]
    fn drum_stem_decodes_async_swaps_clip_and_coloured_waveform_and_persists_in_session() {
        let scratch = Scratch::new("drum-stem-monitor");
        let map = scratch.path("Map.osu");
        std::fs::write(
            &map,
            b"osu file format v14\n[General]\nMode:0\nAudioFilename: song.wav\n[TimingPoints]\n0,500,4,1,1,100,1,0\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n",
        )
        .unwrap();
        // Song: 1 second high-ish tone (2000 Hz); Stem: 2 seconds bass tone (90 Hz) so length & colour differ.
        std::fs::write(scratch.path("song.wav"), test_wav(48_000, 2000.0)).unwrap();
        let stem_path = scratch.path("drums.wav");
        std::fs::write(&stem_path, test_wav(48_000 * 2, 90.0)).unwrap();

        let data_root = scratch.path("data");
        let mut app = headless_with_projects(data_root.clone());
        app.request_open(map.clone());
        wait_open(&mut app);
        wait_audio(&mut app);

        assert!(!app.has_drum_stem());
        assert!(!app.is_monitoring_stem());
        let song_clip = app.active_song_clip().expect("song decoded");
        let song_wf_len = app.active_waveform().len();
        assert!(song_wf_len > 0);

        let rev_before = app.waveform_revision();
        app.load_drum_stem(stem_path.clone());
        assert!(app.drum_stem_loading);
        wait_audio(&mut app);

        assert!(app.has_drum_stem());
        assert!(app.is_monitoring_stem(), "explicit attach switches monitor to stem");
        assert_eq!(app.drum_stem_name().as_deref(), Some("drums.wav"));
        assert!(app.waveform_revision() > rev_before);

        let stem_clip = app.active_song_clip().expect("stem clip active");
        assert!(!Arc::ptr_eq(&song_clip, &stem_clip));
        let stem_wf = app.active_waveform();
        assert!(stem_wf.len() > song_wf_len, "2s stem waveform has more bins than 1s song");
        assert!(stem_wf.iter().any(|b| b.low > 0.0), "stem waveform carries frequency-coloured low band");

        // Toggle back to Song via T command / toggle_monitor_stem.
        let rev_stem = app.waveform_revision();
        assert!(!app.toggle_monitor_stem());
        assert!(!app.is_monitoring_stem());
        assert!(Arc::ptr_eq(&app.active_song_clip().unwrap(), &song_clip));
        assert_eq!(app.active_waveform().len(), song_wf_len);
        assert!(app.waveform_revision() > rev_stem);

        // Toggle via command catalogue ("toggleStemMonitor").
        app.command("toggleStemMonitor");
        assert!(app.is_monitoring_stem());
        assert!(Arc::ptr_eq(&app.active_song_clip().unwrap(), &stem_clip));

        // Verify SessionSettings persisted drum_stem_path and re-opening restores it off-thread.
        app.close_document();
        assert!(!app.has_drum_stem());
        assert!(!app.is_monitoring_stem());

        let mut reopened = headless_with_projects(data_root);
        reopened.request_open(map);
        wait_open(&mut reopened);
        wait_audio(&mut reopened);
        assert!(reopened.has_drum_stem(), "reopening map restores persisted drum_stem_path");
        assert!(!reopened.is_monitoring_stem(), "reopen starts monitoring song until toggled");
        assert_eq!(reopened.drum_stem_name().as_deref(), Some("drums.wav"));
        assert!(reopened.toggle_monitor_stem());
        assert!(reopened.is_monitoring_stem());

        // Clear removes stem and restores song monitoring.
        reopened.clear_drum_stem();
        assert!(!reopened.has_drum_stem());
        assert!(!reopened.is_monitoring_stem());
        assert!(reopened.drum_stem_path().is_none());
    }
}
