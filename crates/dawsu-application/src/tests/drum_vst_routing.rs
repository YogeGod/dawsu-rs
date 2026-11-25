#[cfg(test)]
mod drum_vst_routing_tests {
    use crate::*;
    use dawsu_test_support::Scratch;

    #[test]
    fn toggling_drum_vst_lane_bypasses_midi_and_schedules_sample_cue() {
        struct SilentDrumProcessor;
        impl drum_vst::DrumBlockProcessor for SilentDrumProcessor {
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

        let scratch = Scratch::new("vst-lane-bypass");
        let map = scratch.path("Map.osu");
        std::fs::write(
            &map,
            b"osu file format v14\n[General]\nMode:0\n[TimingPoints]\n0,500,4,1,0,100,1,0\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n256,192,1500,1,8,0:0:0:0:\n",
        )
        .unwrap();

        let mut app = Application::headless();
        app.request_open(map);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while app.busy {
            app.poll();
            assert!(std::time::Instant::now() < deadline, "open job timed out");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(app.error.is_none(), "{:?}", app.error);

        let normal = SampleIdentity::Standard {
            set: SampleSet::Normal,
            sound: Sound::Hit(Hitsound::Normal),
            index: dawsu_foundation::CustomIndex::new(0),
        };
        let clap = SampleIdentity::Standard {
            set: SampleSet::Normal,
            sound: Sound::Hit(Hitsound::Clap),
            index: dawsu_foundation::CustomIndex::new(0),
        };
        let normal_clip = Arc::new(AudioClip { samples: vec![0.4], channels: 1, sample_rate: 48_000 });
        let clap_clip = Arc::new(AudioClip { samples: vec![0.8], channels: 1, sample_rate: 48_000 });
        app.samples.insert(normal.clone(), normal_clip);
        app.samples.insert(clap.clone(), clap_clip);

        let (mut handle, _source) = drum_vst::DrumVstLiveHandle::new(
            "vst-test".into(),
            "Test Drum Kit".into(),
            "test-kit.vst3".into(),
            None,
            true,
            Box::new(SilentDrumProcessor),
        );
        handle.set_routed_notes(HashMap::from([(normal.clone(), 36), (clap.clone(), 38)]));
        app.drum_vst = Some(handle);

        // Both lanes routed to VST -> all 3 cues (Normal@1000, Normal@1500, Clap@1500) become MIDI cues.
        let (sounds, midi) = app.scheduled_audio_and_midi(app.document().unwrap());
        assert!(sounds.is_empty());
        assert_eq!(midi.len(), 3);
        assert!(midi.iter().any(|c| c.at_ms == 1000.0 && c.note == 36));
        assert!(midi.iter().any(|c| c.at_ms == 1500.0 && c.note == 36));
        assert!(midi.iter().any(|c| c.at_ms == 1500.0 && c.note == 38));

        // Bypass the Clap lane via toggle_drum_vst_lane -> Clap plays its WAV sample, Normal stays on MIDI.
        let clap_row = app
            .document()
            .unwrap()
            .composer
            .lanes
            .iter()
            .position(|l| l == &clap)
            .unwrap();
        assert!(!app.toggle_drum_vst_lane(clap_row).unwrap());
        assert_eq!(app.drum_vst.as_ref().unwrap().routed_note_for(&clap), None);
        assert_eq!(app.drum_vst.as_ref().unwrap().routed_note_for(&normal), Some(36));

        let (sounds, midi) = app.scheduled_audio_and_midi(app.document().unwrap());
        assert_eq!(sounds.len(), 1);
        assert_eq!(sounds[0].at_ms, 1500.0);
        assert_eq!(sounds[0].clip.samples, [0.8]);
        assert_eq!(midi.len(), 2);
        assert!(midi.iter().all(|c| c.note == 36));

        // Toggle Clap back ON -> all 3 cues return to MIDI.
        assert!(app.toggle_drum_vst_lane(clap_row).unwrap());
        let (sounds, midi) = app.scheduled_audio_and_midi(app.document().unwrap());
        assert!(sounds.is_empty());
        assert_eq!(midi.len(), 3);
    }

    #[test]
    fn save_readback_verification_catches_tampered_output_without_overwriting_disk() {
        const TEST_MAP: &[u8] = b"osu file format v14\r\n\r\n[General]\r\nSampleSet: Normal\r\n\r\n[Difficulty]\r\nSliderMultiplier:1.4\r\nSliderTickRate:1\r\n\r\n[TimingPoints]\r\n0,500,4,1,0,70,1,0\r\n\r\n[HitObjects]\r\n256,192,1000,1,0,0:0:0:0:\r\n100,100,2000,2,0,L|200:100,1,140\r\n";
        let scratch = Scratch::new("save-readback-verify");
        let folder = scratch.path("mapset");
        std::fs::create_dir(&folder).unwrap();
        let map = folder.join("a.osu");
        std::fs::write(&map, TEST_MAP).unwrap();
        let original_bytes = std::fs::read(&map).unwrap();

        let document = Document::read(&map).unwrap();
        let mut hits = document.composer.hitsounds().clone();
        hits.add_hit(
            dawsu_hitsounding::MomentId(0),
            SampleIdentity::Standard {
                set: SampleSet::Normal,
                sound: Sound::Hit(Hitsound::Clap),
                index: dawsu_foundation::CustomIndex::new(0),
            },
        );
        hits.set_volume(dawsu_hitsounding::MomentId(0), 65);

        let mut diagnostics = Vec::new();
        let mut plan = prepare_save(
            PrepareSaveInput {
                file: document.compile_base.clone(),
                hits: &hits,
                written: &document.written,
                expected: document.file.original_bytes().to_vec(),
                path: &document.path,
                backups: scratch.path("backups"),
                hs: false,
                owned: &BTreeSet::new(),
                defaults: Path::new(""),
                pending: &document.pending,
                project: None,
                review: false,
                clean_redundant_greenlines: false,
            },
            &|| false,
            &mut diagnostics,
            &mut |_, _| {},
        )
        .expect("valid compiled output should pass read-back verification");

        // Deliberately tamper with the compiled output's hitsound field before applying to disk.
        let compiled_text = String::from_utf8(plan.out.to_bytes()).unwrap();
        let tampered_text = compiled_text.replace("256,192,1000,1,8,", "256,192,1000,1,4,");
        assert_ne!(compiled_text, tampered_text);
        plan.out = OsuFile::read(tampered_text.into_bytes()).file;

        let err = plan
            .apply(None, &|| false)
            .err()
            .expect("tampered output must be rejected before writing");
        assert!(
            err.contains("1000 ms"),
            "error should report the mismatched timestamp: {err}"
        );
        assert!(
            err.contains("normal-hitclap") && err.contains("normal-hitfinish"),
            "error should describe expected vs actual samples: {err}"
        );
        assert_eq!(
            std::fs::read(&map).unwrap(),
            original_bytes,
            "original .osu file on disk must remain untouched when verification fails"
        );
    }
}
