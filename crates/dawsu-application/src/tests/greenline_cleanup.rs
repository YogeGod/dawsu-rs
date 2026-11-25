#[cfg(test)]
mod greenline_cleanup_tests {
    use crate::*;
    use dawsu_test_support::Scratch;

    fn wait_busy(app: &mut Application) {
        for _ in 0..2000 {
            app.poll();
            if !app.busy {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("timed out waiting for background job");
    }

    #[test]
    fn save_leaves_redundant_greenlines_untouched_by_default_and_cleans_when_enabled() {
        let scratch = Scratch::new("greenline-cleanup-save");
        let path = scratch.path("Artist - Song (Mapper) [Hard].osu");
        let original = concat!(
            "osu file format v14\n",
            "[General]\n",
            "Mode:0\n",
            "SampleSet:Normal\n",
            "[Metadata]\n",
            "Version:Hard\n",
            "[TimingPoints]\n",
            "0,500,4,1,0,100,1,0\n",
            "500,-100,4,1,0,100,0,0\n",
            "1000,-100,4,1,0,60,0,0\n",
            "1500,-100,4,1,0,60,0,0\n",
            "2000,-50,4,1,0,60,0,0\n",
            "[HitObjects]\n",
            "256,192,1000,1,0,0:0:0:0:\n",
        );
        std::fs::write(&path, original).unwrap();
        let backups = scratch.path("backups");

        // Default (OFF): untouched .osu lines stay 100% byte-identical.
        let doc = Document::read(&path).unwrap();
        let report_off = save_document(&doc, backups.clone()).unwrap();
        assert_eq!(report_off.cleaned_greenlines, 0);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);

        // Enabled (ON): removes the 2 redundant no-op greenlines (500ms and 1500ms) while
        // preserving the red line (0ms) and real volume/SV changes (1000ms and 2000ms).
        let report_on = save_document_with_cleanup(&doc, backups, true).unwrap();
        assert_eq!(report_on.cleaned_greenlines, 2);
        let after = std::fs::read_to_string(&path).unwrap();
        assert!(after.contains("0,500,4,1,0,100,1,0\n"));
        assert!(!after.contains("500,-100,4,1,0,100,0,0\n"));
        assert!(after.contains("1000,-100,4,1,0,60,0,0\n"));
        assert!(!after.contains("1500,-100,4,1,0,60,0,0\n"));
        assert!(after.contains("2000,-50,4,1,0,60,0,0\n"));
    }

    #[test]
    fn application_save_respects_clean_redundant_greenlines_preference() {
        let scratch = Scratch::new("greenline-cleanup-app-pref");
        let path = scratch.path("Artist - Song (Mapper) [Hard].osu");
        let original = concat!(
            "osu file format v14\n",
            "[General]\n",
            "Mode:0\n",
            "SampleSet:Normal\n",
            "[Metadata]\n",
            "Version:Hard\n",
            "[TimingPoints]\n",
            "0,500,4,1,0,100,1,0\n",
            "500,-100,4,1,0,100,0,0\n",
            "1000,-100,4,1,0,75,0,0\n",
            "[HitObjects]\n",
            "256,192,1000,1,0,0:0:0:0:\n",
        );
        std::fs::write(&path, original).unwrap();

        let mut app = Application::create(
            Preferences::default(),
            false,
            None,
            None,
            false,
            scratch.path("data"),
        );
        assert!(!app.preferences.clean_redundant_greenlines);
        app.open_now(OpenSource::Map(path.clone()));
        wait_busy(&mut app);

        // Save with preference OFF -> byte-identical.
        app.save();
        wait_busy(&mut app);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        assert_eq!(app.status, "Saved with backup");

        // Enable preference and Save -> removes the redundant 500ms greenline.
        app.preferences.clean_redundant_greenlines = true;
        app.save();
        wait_busy(&mut app);
        let after = std::fs::read_to_string(&path).unwrap();
        assert!(!after.contains("500,-100,4,1,0,100,0,0\n"));
        assert!(after.contains("1000,-100,4,1,0,75,0,0\n"));
        assert_eq!(app.status, "Saved with backup. Removed 1 redundant greenline.");
    }

    #[test]
    fn write_plan_clean_redundant_greenlines_preserves_real_changes() {
        let scratch = Scratch::new("greenline-cleanup-export");
        let source_path = scratch.path("Artist - Song (Mapper) [Source].osu");
        let target_path = scratch.path("Artist - Song (Mapper) [Target].osu");
        let map_text = concat!(
            "osu file format v14\n",
            "[General]\n",
            "Mode:0\n",
            "SampleSet:Normal\n",
            "[Metadata]\n",
            "Version:Target\n",
            "[TimingPoints]\n",
            "0,500,4,1,0,100,1,0\n",
            "250,-100,4,1,0,100,0,0\n",
            "500,-50,4,1,0,100,0,0\n",
            "[HitObjects]\n",
            "256,192,1000,1,0,0:0:0:0:\n",
        );
        std::fs::write(&source_path, map_text).unwrap();
        std::fs::write(&target_path, map_text).unwrap();
        let doc = Document::read(&source_path).unwrap();
        let source = output::Source::from_document(&doc);
        let mut plan = output::copy(
            &source,
            &[target_path],
            &dawsu_hitsounding::copy::CopyOptions::default(),
            &|| false,
        ).unwrap();
        let before = String::from_utf8(plan.files[0].file.to_bytes()).unwrap();
        assert!(before.contains("250,-100,4,1,0,100,0,0\n"));
        assert_eq!(plan.clean_redundant_greenlines(), 1);
        let after = String::from_utf8(plan.files[0].file.to_bytes()).unwrap();
        assert!(!after.contains("250,-100,4,1,0,100,0,0\n"));
        assert!(after.contains("500,-50,4,1,0,100,0,0\n"));
        assert!(plan.details.iter().any(|d| d.contains("Removed 1 redundant greenline.")));
    }

    #[test]
    fn import_mania_hitsound_diff_adds_lanes_places_hits_and_undoes_in_one_step() {
        let scratch = Scratch::new("mania-hs-import");
        let std_path = scratch.path("Artist - Song (Mapper) [Hard].osu");
        let mania_path = scratch.path("Artist - Song (Mapper) [Hitsounds].osu");
        let non_mania_path = scratch.path("Artist - Song (Mapper) [Other].osu");

        let std_map = concat!(
            "osu file format v14\n",
            "[General]\n",
            "Mode: 0\n",
            "SampleSet: Normal\n",
            "[Metadata]\n",
            "Version: Hard\n",
            "[TimingPoints]\n",
            "0,500,4,1,0,100,1,0\n",
            "[HitObjects]\n",
            "256,192,1000,1,0,0:0:0:0:\n",
        );
        let mania_map = concat!(
            "osu file format v14\n",
            "[General]\n",
            "Mode: 3\n",
            "SampleSet: Normal\n",
            "[TimingPoints]\n",
            "0,500,4,2,2,70,1,0\n",
            "[HitObjects]\n",
            "64,192,1002,1,8,0:0:0:85:\n",
            "192,192,1002,1,0,0:0:0:60:kick.wav\n",
            "320,192,2000,1,0,0:0:0:0:snare.wav\n",
        );
        std::fs::write(&std_path, std_map).unwrap();
        std::fs::write(&mania_path, mania_map).unwrap();
        std::fs::write(&non_mania_path, std_map).unwrap();

        let mut app = Application::create(
            Preferences::default(),
            false,
            None,
            None,
            false,
            scratch.path("data"),
        );
        app.open_now(OpenSource::Map(std_path));
        wait_busy(&mut app);

        let before_moments = app.document().unwrap().composer.hitsounds().moments().to_vec();
        let before_lanes = app.document().unwrap().composer.hitsounds().lanes().to_vec();

        // Refuses non-mania diff cleanly without modifying document.
        let err = app.import_mania_hitsound_diff(&non_mania_path).unwrap_err();
        assert!(err.contains("Mode: 3"), "expected mania error, got: {err}");
        assert_eq!(app.document().unwrap().composer.hitsounds().moments(), before_moments.as_slice());

        // Imports mania diff: snaps 1002 ms onto existing 1000 ms object, creates 2000 ms loose moment.
        let report = app.import_mania_hitsound_diff(&mania_path).unwrap();
        assert!(report.hits_imported >= 3);
        assert!(report.lanes_added >= 2);
        assert!(report.created_moments >= 1);
        assert!(app.status.contains("Mania hitsound diff"));

        let hs = app.document().unwrap().composer.hitsounds();
        let kick = SampleIdentity::File("kick.wav".into());
        let snare = SampleIdentity::File("snare.wav".into());
        let soft_clap_2 = SampleIdentity::Standard {
            set: SampleSet::Soft,
            sound: dawsu_foundation::Sound::Hit(dawsu_foundation::Hitsound::Clap),
            index: dawsu_foundation::CustomIndex::new(2),
        };
        assert!(hs.lane(&kick).is_some());
        assert!(hs.lane(&snare).is_some());
        assert!(hs.lane(&soft_clap_2).is_some());

        let m1000 = hs.moments().iter().find(|m| (m.time().ms() - 1000.0).abs() < 0.1).unwrap();
        assert!(m1000.has_hit(&kick));
        assert_eq!(m1000.hit_volume(&kick), 60);
        assert!(m1000.has_hit(&soft_clap_2));
        assert_eq!(m1000.hit_volume(&soft_clap_2), 85);

        let m2000 = hs.moments().iter().find(|m| (m.time().ms() - 2000.0).abs() < 0.1).unwrap();
        assert_eq!(m2000.place(), dawsu_hitsounding::Place::NoObject);
        assert!(m2000.has_hit(&snare));
        assert_eq!(m2000.hit_volume(&snare), 70);

        // Single undo() reverts everything cleanly.
        app.document.as_mut().unwrap().composer.undo();
        assert_eq!(app.document().unwrap().composer.hitsounds().moments(), before_moments.as_slice());
        assert_eq!(app.document().unwrap().composer.hitsounds().lanes(), before_lanes.as_slice());
    }
}
