use super::{Application, Document, SampleSet, sample_path};
use dawsu_hitsounding::Place;

impl Document {
    /// A single slider selected by any of its hit points.
    pub fn selected_slider_body_index(&self) -> Option<usize> {
        let hits = self.composer.hitsounds();
        let mut selected = self.composer.selected.items().iter();
        let object_of = |moment: usize| match hits.moments().get(moment)?.place() {
            Place::SliderNode { object, .. } => Some(object),
            _ => None,
        };
        let slider = object_of(selected.next()?.moment)?;
        if selected.any(|hit| object_of(hit.moment) != Some(slider)) {
            return None;
        }
        hits.bodies()
            .iter()
            .position(|body| body.object() == slider)
    }
}

impl Application {
    pub fn edit_slider_body(
        &mut self,
        index: usize,
        set: Option<SampleSet>,
        whistle_set: Option<Option<SampleSet>>,
        whistle: bool,
        volume: i32,
    ) -> Result<(), String> {
        if self.busy || self.is_stroke_open() {
            return Err("Finish the current edit before changing a slider body".into());
        }
        let doc = self.document.as_mut().ok_or("Open a difficulty first")?;
        if doc.selected_slider_body_index() != Some(index) {
            return Err("Select a hit point on the slider to change its body sounds".into());
        }
        let body = &doc.composer.hitsounds().bodies()[index];
        // A changed volume is never under 5% (R15); a quieter one the map already has may stay.
        let floor = dawsu_hitsounding::rules::SAVED_VOLUME_FLOOR;
        let volume = if volume == body.volume() { volume } else { volume.clamp(floor, 100) };
        let changed_set = set.filter(|&set| body.set().is_some_and(|before| before != set));
        let changed_volume = body.volume() != volume;
        let changed_whistle = body.whistle() != whistle;
        let changed_whistle_set = whistle_set.filter(|requested| {
            *requested != body.whistle_set() || requested.is_none() && changed_set.is_some()
        });
        if changed_set.is_none()
            && changed_whistle_set.is_none()
            && !changed_volume
            && !changed_whistle
        {
            return Ok(());
        }
        doc.composer.apply("Slider body sounds", |hits| {
            if let Some(set) = changed_set {
                hits.set_body_set(index, set);
            }
            if let Some(set) = changed_whistle_set {
                hits.set_body_whistle_set(index, set);
            }
            if changed_whistle {
                hits.set_body_whistle(index, whistle);
            }
            if changed_volume {
                hits.set_body_volume(index, volume);
                // The head's hits move with the body and stop at 5% too.
                let object = hits.bodies()[index].object();
                let head = hits.moments().iter()
                    .position(|m| m.place() == Place::SliderNode { object, node: 0 });
                if let Some(head) = head {
                    let id = dawsu_hitsounding::MomentId(head);
                    let quiet: Vec<_> = hits.moment(id).hits()
                        .filter(|lane| hits.moment(id).hit_volume(lane) < floor)
                        .cloned().collect();
                    for lane in quiet {
                        hits.set_hit_volume(id, &lane, floor);
                    }
                }
            }
        });

        let hits = doc.composer.history.document();
        let body = &hits.bodies()[index];
        let playable_lanes = [
            Some(body.slide().clone()),
            body.whistle().then(|| body.whistle_lane()),
            (!body.ticks().is_empty()).then(|| body.tick().clone()),
        ];
        let folder = doc.path.parent().ok_or("Missing mapset folder")?;
        let defaults = &self.preferences.default_samples;
        let defaults_list = crate::pending::sample_folder(defaults);
        for lane in playable_lanes
            .into_iter()
            .flatten()
            .filter_map(|id| hits.lane(&id))
        {
            if !doc.composer.lanes.contains(&lane.identity) {
                doc.composer.lanes.push(lane.identity.clone());
            }
            let source = lane.plays_as();
            if self.samples.contains_key(&source) {
                continue;
            }
            match sample_path(lane, folder, &doc.pending, defaults, &defaults_list)
                .and_then(|path| dawsu_audio::decode(&path).map_err(|err| err.to_string()))
            {
                Ok(clip) => {
                    self.samples.insert(source, clip);
                }
                Err(error) if !self.warnings.contains(&error) => self.warnings.push(error),
                Err(_) => {}
            }
        }
        self.sync_audio();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawsu_test_support::Scratch;
    use std::{
        fs,
        time::{Duration, Instant},
    };

    #[test]
    fn a_slider_body_volume_under_5_percent_becomes_5() {
        let fixture = Scratch::new("slider-body-volume-floor");
        let path = fixture.path("Slider.osu");
        let map = concat!(
            "osu file format v14\n\n[Difficulty]\nSliderMultiplier:1.4\nSliderTickRate:1\n\n",
            "[TimingPoints]\n0,500,4,2,1,60,1,0\n\n[HitObjects]\n",
            "100,100,1000,2,0,L|300:100,2,200,0|0|0,0:0|0:0|0:0,0:0:0:0:\n",
        );
        fs::write(&path, map).unwrap();
        let mut app = Application::headless();
        app.preferences.default_samples = fixture.path("defaults");
        app.request_open(path);
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.busy {
            app.poll();
            assert!(Instant::now() < deadline, "slider fixture did not open");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(app.error.is_none(), "{:?}", app.error);
        app.command("selectAll");
        let whistle = app.document().unwrap().composer.hitsounds().bodies()[0].whistle();
        app.edit_slider_body(0, None, None, whistle, 2).unwrap();

        let hits = app.document().unwrap().composer.hitsounds();
        assert_eq!(hits.bodies()[0].volume(), 5, "typing 2 gives 5");
        let object = hits.bodies()[0].object();
        let head = hits.moments().iter()
            .find(|m| m.place() == Place::SliderNode { object, node: 0 })
            .expect("the slider has a head");
        assert_eq!(head.volume(), 5, "the head moves with the body and stops at 5");
        assert!(head.hits().all(|lane| head.hit_volume(lane) >= 5));
    }

    #[test]
    fn edited_slider_cues_load_and_play_new_slide_whistle_and_tick_samples() {
        let fixture = Scratch::new("slider-preview-cues");
        let path = fixture.path("Slider.osu");
        let map = concat!(
            "osu file format v14\n\n[Difficulty]\nSliderMultiplier:1.4\nSliderTickRate:1\n\n",
            "[TimingPoints]\n0,500,4,2,1,60,1,0\n\n[HitObjects]\n",
            "100,100,1000,2,0,L|300:100,2,200,0|0|0,0:0|0:0|0:0,0:0:0:0:\n",
        );
        fs::write(&path, map).unwrap();
        let header = b"RIFF\x26\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data\x02\0\0\0";
        for (name, amplitude) in [
            ("soft-sliderslide.wav", 4096i16),
            ("soft-slidertick.wav", 4096),
            ("drum-sliderslide.wav", 8192),
            ("soft-sliderwhistle.wav", 16384),
            ("normal-sliderwhistle.wav", 4096),
            ("drum-sliderwhistle.wav", 20480),
            ("drum-slidertick.wav", 24576),
        ] {
            let mut wave = header.to_vec();
            wave.extend(amplitude.to_le_bytes());
            fs::write(fixture.path(name), wave).unwrap();
        }
        let mut app = Application::headless();
        app.preferences.default_samples = fixture.path("defaults");
        app.request_open(path);
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.busy {
            app.poll();
            assert!(Instant::now() < deadline, "slider fixture did not open");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(app.error.is_none(), "{:?}", app.error);
        app.command("selectAll");
        app.edit_slider_body(0, Some(SampleSet::Drum), None, true, 80)
            .unwrap();

        let doc = app.document().unwrap();
        let end = doc.composer.hitsounds().bodies()[0].end().ms();
        let cues = app.scheduled_sounds(doc);
        for amplitude in [0.25, 0.5] {
            assert!(
                cues.iter().any(|cue| cue.at_ms == 1000.0
                    && cue.end_ms == Some(end)
                    && cue.gain == 0.8
                    && cue.clip.samples == [amplitude]),
                "missing {amplitude} loop"
            );
        }
        assert!(
            cues.iter().any(|cue| cue.at_ms > 1000.0
                && cue.end_ms.is_none()
                && cue.clip.samples == [0.75]),
            "missing changed tick"
        );

        for (choice, amplitude) in [(Some(SampleSet::Normal), 0.125), (None, 0.625)] {
            app.edit_slider_body(0, None, Some(choice), true, 80)
                .unwrap();
            let doc = app.document().unwrap();
            assert_eq!(doc.composer.hitsounds().bodies()[0].whistle_set(), choice);
            assert!(
                app.scheduled_sounds(doc)
                    .iter()
                    .any(|cue| cue.at_ms == 1000.0
                        && cue.end_ms == Some(end)
                        && cue.gain == 0.8
                        && cue.clip.samples == [amplitude])
            );
        }
        app.command("undo");
        let choice = app.document().unwrap().composer.hitsounds().bodies()[0].whistle_set();
        assert_eq!(choice, Some(SampleSet::Normal));
    }
}
