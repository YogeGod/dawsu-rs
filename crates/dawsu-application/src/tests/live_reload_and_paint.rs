#[cfg(test)]
mod live_osu_reload_tests {
    use crate::*;
    use dawsu_test_support::Scratch;
    use std::time::Duration;

    const BASE_MAP: &str = concat!(
        "osu file format v14\n",
        "[General]\n",
        "Mode:0\n",
        "SampleSet:Normal\n",
        "[Metadata]\n",
        "Version:Hard\n",
        "[TimingPoints]\n",
        "0,500,4,1,0,100,1,0\n",
        "[HitObjects]\n",
        "256,192,1000,1,0,0:0:0:0:\n",
    );

    fn wait_busy(app: &mut Application) {
        for _ in 0..2000 {
            app.poll();
            if !app.busy {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("timed out waiting for background job");
    }

    fn make_app(scratch: &Scratch) -> Application {
        Application::create(
            Preferences::default(),
            false,
            None,
            None,
            false,
            scratch.path("data"),
        )
    }

    #[test]
    fn clean_document_auto_reloads_on_external_osu_edit_and_preserves_view_and_playhead() {
        let scratch = Scratch::new("live-osu-reload-clean");
        let path = scratch.path("Artist - Song (Mapper) [Hard].osu");
        std::fs::write(&path, BASE_MAP).unwrap();

        let mut app = make_app(&scratch);
        app.open_now(OpenSource::Map(path.clone()));
        wait_busy(&mut app);

        assert_eq!(app.document().unwrap().composer.hitsounds().moments().len(), 1);
        assert!(!app.is_dirty());

        // Set custom view and playhead state before external edit.
        app.zoom = 0.42;
        app.row_height = 44.0;
        app.set_view_start(250.0);
        app.seek(600.0);
        app.osu_watch
            .set_timings(Duration::ZERO, Duration::from_millis(15));

        // Simulate osu! saving a new hitobject at 2000 ms.
        let updated = format!("{BASE_MAP}300,200,2000,1,0,0:0:0:0:\n");
        std::fs::write(&path, &updated).unwrap();

        // First poll starts debounce; should not reload immediately.
        app.poll();
        assert!(!app.busy, "watcher should debounce before reloading");
        assert!(!app.external_osu_changed);

        // Wait past debounce window, then poll -> triggers clean auto-reload.
        std::thread::sleep(Duration::from_millis(25));
        app.poll();
        assert!(app.busy, "clean document should automatically start reloading");
        wait_busy(&mut app);

        assert!(!app.external_osu_changed);
        assert_eq!(app.status, "Reloaded changes from disk");
        let doc = app.document().unwrap();
        assert_eq!(doc.composer.hitsounds().moments().len(), 2);
        assert!((app.zoom - 0.42).abs() < 1e-6);
        assert!((app.row_height - 44.0).abs() < 1e-4);
        assert!((doc.composer.view_start - 250.0).abs() < 1e-6);
        assert!((app.position_ms() - 600.0).abs() < 1e-6);
    }

    #[test]
    fn dirty_document_keeps_edits_on_external_osu_change_and_supports_dismiss_and_reload() {
        let scratch = Scratch::new("live-osu-reload-dirty");
        let path = scratch.path("Artist - Song (Mapper) [Hard].osu");
        std::fs::write(&path, BASE_MAP).unwrap();

        let mut app = make_app(&scratch);
        app.open_now(OpenSource::Map(path.clone()));
        wait_busy(&mut app);

        // Make an unsaved hitsound edit (add Clap at moment 0).
        let clap = SampleIdentity::Standard {
            set: SampleSet::Normal,
            sound: Sound::Hit(Hitsound::Clap),
            index: dawsu_foundation::CustomIndex::new(0),
        };
        app.document
            .as_mut()
            .unwrap()
            .composer
            .history
            .apply("Add hit", true, |hits| {
                hits.add_hit(dawsu_hitsounding::MomentId(0), clap.clone());
                Ok::<_, ()>(())
            })
            .unwrap();
        assert!(app.is_dirty());

        app.osu_watch.set_timings(Duration::ZERO, Duration::ZERO);

        // External edit #1: add hitobject at 2000 ms.
        let updated1 = format!("{BASE_MAP}300,200,2000,1,0,0:0:0:0:\n");
        std::fs::write(&path, &updated1).unwrap();
        app.poll();

        assert!(!app.busy, "dirty document must NOT auto-reload");
        assert!(app.external_osu_changed);
        assert!(app.is_dirty(), "unsaved edits must remain intact");
        assert_eq!(
            app.document().unwrap().composer.hitsounds().moments().len(),
            1,
            "external file must not overwrite dirty document"
        );
        assert!(
            app.document()
                .unwrap()
                .composer
                .hitsounds()
                .moments()[0]
                .has_hit(&clap),
            "mapper's unsaved Clap hit must still be present"
        );

        // Choose "Keep my edits": clears notice and does not re-trigger for the same bytes.
        app.dismiss_external_osu_change();
        assert!(!app.external_osu_changed);
        assert!(app.is_dirty());
        app.poll();
        assert!(
            !app.external_osu_changed,
            "dismissed external edit must not re-alert on subsequent polls"
        );

        // External edit #2: add hitobject at 3000 ms -> alerts again.
        let updated2 = format!("{updated1}320,210,3000,1,0,0:0:0:0:\n");
        std::fs::write(&path, &updated2).unwrap();
        app.poll();
        assert!(app.external_osu_changed);
        assert!(!app.busy);

        // Choose "Reload from disk": reloads cleanly and clears dirty state.
        app.reload_from_disk();
        assert!(app.busy);
        wait_busy(&mut app);

        assert!(!app.external_osu_changed);
        assert!(!app.is_dirty());
        assert_eq!(app.status, "Reloaded changes from disk");
        assert_eq!(app.document().unwrap().composer.hitsounds().moments().len(), 3);
    }

    #[test]
    fn saving_from_dawsu_does_not_trigger_external_osu_change() {
        let scratch = Scratch::new("live-osu-reload-own-save");
        let path = scratch.path("Artist - Song (Mapper) [Hard].osu");
        std::fs::write(&path, BASE_MAP).unwrap();

        let mut app = make_app(&scratch);
        app.open_now(OpenSource::Map(path.clone()));
        wait_busy(&mut app);

        let clap = SampleIdentity::Standard {
            set: SampleSet::Normal,
            sound: Sound::Hit(Hitsound::Clap),
            index: dawsu_foundation::CustomIndex::new(0),
        };
        app.document
            .as_mut()
            .unwrap()
            .composer
            .history
            .apply("Add hit", true, |hits| {
                hits.add_hit(dawsu_hitsounding::MomentId(0), clap);
                Ok::<_, ()>(())
            })
            .unwrap();
        assert!(app.is_dirty());

        app.osu_watch.set_timings(Duration::ZERO, Duration::ZERO);
        app.save();
        wait_busy(&mut app);

        assert!(!app.is_dirty());
        assert!(!app.external_osu_changed);
        assert_eq!(app.status, "Saved with backup");

        // Poll repeatedly after our own save: must NOT trigger reload or external_osu_changed.
        for _ in 0..5 {
            app.poll();
            assert!(!app.busy);
            assert!(!app.external_osu_changed);
            assert_eq!(app.status, "Saved with backup");
        }
    }
}

#[cfg(test)]
mod draw_paint_stroke_tests {
    use crate::*;
    use dawsu_test_support::Scratch;

    #[test]
    fn paint_hits_on_lane_places_snapped_steps_skips_occupied_respects_volume_and_undoes_in_one_step() {
        let scratch = Scratch::new("paint-draw-stroke");
        let path = scratch.path("test.osu");
        // 120 BPM (500 ms per beat) => 1/4 beat step = 125 ms.
        // Existing circle at 1000 ms (normal-hitnormal at vol 100) and another at 1500 ms.
        std::fs::write(
            &path,
            "osu file format v14\n[General]\nMode:0\n[TimingPoints]\n0,500,4,1,0,100,1,0\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n256,192,1500,1,0,0:0:0:0:\n",
        )
        .unwrap();
        let mut app = Application::headless();
        app.request_open(path);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while app.busy {
            app.poll();
            assert!(std::time::Instant::now() < deadline, "open job timed out");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(app.error.is_none(), "{:?}", app.error);

        let clap = SampleIdentity::Standard {
            set: SampleSet::Normal,
            sound: Sound::Hit(Hitsound::Clap),
            index: dawsu_foundation::CustomIndex::new(0),
        };
        // Ensure clap lane exists and pre-place one clap at 1250 ms at volume 90.
        let clap_row = {
            let c = &mut app.document.as_mut().unwrap().composer;
            c.add(1250.0, clap.clone(), 90, 5.0);
            if !c.lanes.contains(&clap) {
                c.lanes.push(clap.clone());
            }
            c.lanes.iter().position(|l| l == &clap).unwrap()
        };
        let history_before = app.document().unwrap().composer.history.position();

        // Set 1/4 beat snap in Grid mode and insert_volume = 65.
        app.set_snap_options(true, 1, 4, dawsu_editor::tempo::SnapMode::Grid);
        app.set_insert_volume(65);

        // Paint from 1000 ms to 1500 ms => steps 1000, 1125, 1250 (occupied), 1375, 1500.
        let placed = app.paint_hits_on_lane(clap_row, 1000.0, 1500.0);
        assert_eq!(placed, 4, "should place 4 new hits and skip occupied 1250 ms");

        let c = &app.document().unwrap().composer;
        assert_eq!(
            c.history.position(),
            history_before + 1,
            "entire painted stroke must be a single undo step"
        );
        let mut clap_hits: Vec<(f64, i32)> = c
            .hitsounds()
            .moments()
            .iter()
            .filter(|m| m.has_hit(&clap))
            .map(|m| (m.time().ms(), m.hit_volume(&clap)))
            .collect();
        clap_hits.sort_by(|a, b| a.0.total_cmp(&b.0));
        assert_eq!(
            clap_hits,
            vec![
                (1000.0, 65),
                (1125.0, 65),
                (1250.0, 90), // pre-existing hit kept its volume
                (1375.0, 65),
                (1500.0, 65),
            ]
        );

        // Single undo removes all 4 painted hits and leaves only the pre-existing 1250 ms hit.
        app.document.as_mut().unwrap().composer.undo();
        let after_undo: Vec<(f64, i32)> = app
            .document()
            .unwrap()
            .composer
            .hitsounds()
            .moments()
            .iter()
            .filter(|m| m.has_hit(&clap))
            .map(|m| (m.time().ms(), m.hit_volume(&clap)))
            .collect();
        assert_eq!(after_undo, vec![(1250.0, 90)]);

        // Objects snap mode paints only at object times (1000 ms and 1500 ms).
        app.set_snap_options(true, 1, 4, dawsu_editor::tempo::SnapMode::Objects);
        app.set_insert_volume(72);
        let placed_obj = app.paint_hits_on_lane(clap_row, 990.0, 1510.0);
        assert_eq!(placed_obj, 2);
        let mut obj_hits: Vec<(f64, i32)> = app
            .document()
            .unwrap()
            .composer
            .hitsounds()
            .moments()
            .iter()
            .filter(|m| m.has_hit(&clap))
            .map(|m| (m.time().ms(), m.hit_volume(&clap)))
            .collect();
        obj_hits.sort_by(|a, b| a.0.total_cmp(&b.0));
        assert_eq!(obj_hits, vec![(1000.0, 72), (1250.0, 90), (1500.0, 72)]);

        // Undo objects stroke, then verify reverse drag and 512-step cap.
        app.document.as_mut().unwrap().composer.undo();
        app.set_snap_options(true, 1, 4, dawsu_editor::tempo::SnapMode::GridRelative);
        app.set_insert_volume(55);
        // Dragging backwards across 100,000 ms (800 steps) must be capped at 512 steps.
        let placed_capped = app.paint_hits_on_lane(clap_row, 100_000.0, 0.0);
        assert!(
            placed_capped <= dawsu_editor_composer::MAX_PAINT_STROKE_STEPS,
            "placed {placed_capped} hits, exceeding cap"
        );
        assert_eq!(placed_capped, dawsu_editor_composer::MAX_PAINT_STROKE_STEPS);
    }
}
