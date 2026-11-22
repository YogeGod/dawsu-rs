//! The batch step (composer TDD R23): many detected hits as one undo step. The 3.1.4 parity cases
//! come from `dawsu-graph-audio`'s classic fixture (`create.json`).

use dawsu_editor::tempo::{TempoMap, TimingSection};
use dawsu_editor_composer::{
    BatchHit, BatchOptions, BatchRefusal, Composer, HitKey, Lands, PatternStampOptions, Policy,
    quantize_stamp_volume,
};
use dawsu_foundation::{SampleIdentity, SongTime};
use dawsu_graph_audio::classic::{grid_shift, on_grid};
use dawsu_hitsounding::{Decompiled, Place, decompile};
use dawsu_osu_format::OsuFile;
use serde_json::Value;

fn lane(name: &str) -> SampleIdentity {
    SampleIdentity::File(format!("{name}.wav"))
}

fn composer_for(osu: &str) -> Composer {
    let file = OsuFile::read(osu.as_bytes().to_vec()).file;
    let files = vec!["kick.wav".to_owned(), "snare.wav".to_owned(), "tom.wav".to_owned()];
    let Decompiled::Done(mut hits) = decompile(&file, &files, &|| false, &mut |_, _| {}) else { panic!("fixture map") };
    hits.add_folder_samples(&files);
    let mut composer = Composer::new(hits);
    composer.tempo = TempoMap::new(vec![TimingSection { beat_length_ms: 500.0, ..TimingSection::DEFAULT }]);
    composer
}

const TWO_CIRCLES: &str = "osu file format v14\n[General]\nSampleSet:Soft\nMode:0\n[Metadata]\nVersion:Test\n[TimingPoints]\n0,500,4,2,0,45,1,0\n[HitObjects]\n100,100,1000,1,0,0:0:0:0:\n100,100,1500,1,0,0:0:0:0:\n";

fn composer() -> Composer {
    composer_for(TWO_CIRCLES)
}

/// 120 BPM from 0 ms, 3.1.4's test grid.
fn grid(time: f64, divisor: u32) -> (f64, f64) {
    let step = 500.0 / f64::from(divisor);
    ((time / step).round_ties_even() * step, step)
}

fn hit(name: &str, time: f64, strength: f64) -> BatchHit {
    BatchHit { time, lane: lane(name), strength, grid: on_grid(time, &grid, &[4], 0.0) }
}

fn hits_on(c: &Composer, name: &str) -> Vec<(f64, bool)> {
    let mut found: Vec<_> = c.hitsounds().moments().iter().filter(|m| m.has_hit(&lane(name)))
        .map(|m| (m.time().ms(), m.place() != Place::NoObject)).collect();
    found.sort_by(|a, b| a.0.total_cmp(&b.0));
    found
}

// ---- 3.1.4 parity ----

fn number(value: &Value) -> f64 {
    match value {
        Value::String(s) if s == "inf" => f64::INFINITY,
        Value::String(s) if s == "-inf" => f64::NEG_INFINITY,
        Value::String(_) => f64::NAN,
        other => other.as_f64().unwrap(),
    }
}

#[test]
fn classic_options_reproduce_3_1_4_auto_fill() {
    let path = std::env::var_os("CARGO_MANIFEST_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")))
        .join("../dawsu-graph-audio/tests/fixtures/classic-3.1.4/create.json");
    let fixture: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let mut c = composer_for(fixture["maps"][case["map"].as_str().unwrap()].as_str().unwrap());
        for (index, step) in case["steps"].as_array().unwrap().iter().enumerate() {
            let label = format!("{name} step {index}");
            let divisors: Option<Vec<u32>> = step["divisors"].as_array()
                .map(|d| d.iter().map(|v| v.as_u64().unwrap() as u32).collect());
            let divisors = divisors.filter(|d| !d.is_empty());
            let (from, to) = (number(&step["from"]), number(&step["to"]));
            let candidates: Vec<(String, f64, f64)> = step["candidates"].as_array().unwrap().iter()
                .map(|c| (c["drum"].as_str().unwrap().to_owned(), number(&c["time_ms"]), c["score"].as_f64().unwrap()))
                .collect();
            // 3.1.4 measured the lag over every finite candidate in the range, any drum.
            let shift = divisors.as_ref().map_or(0.0, |d| grid_shift(
                candidates.iter().map(|c| c.1).filter(|t| t.is_finite() && *t >= from && *t < to), &grid, d[0]));
            let snare = step["snare_lane"].as_str().unwrap();
            let hits: Vec<BatchHit> = candidates.iter().filter(|c| c.1.is_finite() && c.2.is_finite() && c.1 >= 0.0)
                .filter_map(|(drum, time, score)| {
                    let lane = match drum.as_str() {
                        "kick" => lane("kick"),
                        "snare" if snare == "snare" => lane("snare"),
                        "snare" if snare == "kick" => lane("kick"),
                        _ => return None,
                    };
                    let grid = divisors.as_ref().and_then(|d| on_grid(*time, &grid, d, shift));
                    Some(BatchHit { time: *time, lane, strength: *score, grid })
                }).collect();
            let options = BatchOptions {
                object_ms: fixture["object_tolerance_ms"].as_f64().unwrap(),
                dedupe_ms: fixture["anchor_tolerance_ms"].as_f64().unwrap(),
                shift_ms: shift,
                range: Some((from, to)),
                policy: Policy::Classic {
                    filter_grid: divisors.is_some(),
                    include_unmatched: step["include_unmatched"].as_bool().unwrap(),
                },
            };
            let plan = c.plan_batch(&hits, &options);
            assert_eq!(plan.counts.unmatched, step["unmatched"].as_u64().unwrap() as usize, "{label} unmatched");
            assert_eq!(plan.counts.duplicates, step["duplicates"].as_u64().unwrap() as usize, "{label} duplicates");
            assert_eq!(plan.counts.off_grid, step["off_grid"].as_u64().unwrap() as usize, "{label} off grid");
            assert_eq!(format!("{shift:.4}"), format!("{:.4}", step["grid_shift_ms"].as_f64().unwrap()), "{label} lag");
            let mut ours: Vec<(String, String, bool)> = plan.rows.iter().filter_map(|row| {
                let anchored = match row.lands { Lands::Object(_) => true, Lands::Loose => false, _ => return None };
                let lane = if hits[row.hit].lane == lane("kick") { "kick" } else { "snare" };
                Some((lane.to_owned(), format!("{:.4}", row.time), anchored))
            }).collect();
            // In HS mode 3.1.4 gave an unmatched hit a key for the hitsounding circle Save would write
            // (`HsHit`, a negative object index), so it reads as anchored. Here the hit sits on a
            // no-object moment that Save turns into that circle, so only lane and time are compared.
            let hs = case["hs_mode"].as_bool().unwrap();
            let mut theirs: Vec<(String, String, bool)> = step["hits"].as_array().unwrap().iter().map(|h| (
                h["lane"].as_str().unwrap().to_owned(),
                format!("{:.4}", h["time_ms"].as_f64().unwrap()),
                h["anchored"].as_bool().unwrap() && !hs,
            )).collect();
            ours.sort();
            theirs.sort();
            assert_eq!(ours, theirs, "{label} hits");
            if step["apply"].as_bool().unwrap() {
                let steps = undo_steps(&mut c);
                c.add_batch("Drum hit detection", &hits, &options).unwrap();
                assert_eq!(undo_steps(&mut c) - steps, step["undo_steps_added"].as_u64().unwrap() as usize, "{label} undo");
            }
        }
    }
}

/// Counts undo steps without changing the document.
fn undo_steps(c: &mut Composer) -> usize {
    let mut n = 0;
    while c.history.undo() {
        n += 1;
    }
    for _ in 0..n {
        c.history.redo();
    }
    n
}

// ---- Yoge's default: object, else grid line, else flagged ----

#[test]
fn applying_is_one_undo_step_and_selects_the_added_hits() {
    let mut c = composer();
    let before = c.hitsounds().clone();
    let report = c.add_batch("Drum hit detection", &[hit("kick", 1010.0, 0.9), hit("snare", 1490.0, 0.8),
        hit("kick", 1260.0, 0.7)], &BatchOptions::default()).unwrap();
    assert_eq!((report.counts.added, report.counts.on_objects, report.counts.loose), (3, 2, 1));
    assert_eq!(hits_on(&c, "kick"), vec![(1000.0, true), (1250.0, false)]);
    assert_eq!(hits_on(&c, "snare"), vec![(1500.0, true)]);
    assert_eq!(c.selected.items().len(), 3);
    assert_eq!(undo_steps(&mut c), 1);
    c.undo();
    assert_eq!(c.hitsounds(), &before);
}

#[test]
fn a_batch_that_adds_nothing_adds_no_undo_step() {
    let mut c = composer();
    let report = c.add_batch("Drum hit detection", &[], &BatchOptions::default()).unwrap();
    assert_eq!(report.counts.added, 0);
    assert!(!c.history.can_undo());
    c.add_batch("Drum hit detection", &[hit("kick", 1000.0, 1.0)], &BatchOptions::default()).unwrap();
    let version = c.history.version();
    let again = c.add_batch("Drum hit detection", &[hit("kick", 1000.0, 1.0)], &BatchOptions::default()).unwrap();
    assert_eq!((again.counts.added, again.counts.duplicates), (0, 1));
    assert_eq!(c.history.version(), version);
    assert_eq!(undo_steps(&mut c), 1);
}

#[test]
fn refused_while_an_undo_step_is_open() {
    let mut c = composer();
    c.history.begin("Drag");
    let version = c.history.version();
    assert_eq!(c.add_batch("Drum hit detection", &[hit("kick", 1000.0, 1.0)], &BatchOptions::default()).err(),
        Some(BatchRefusal::StepOpen));
    assert_eq!(c.history.version(), version);
    assert!(hits_on(&c, "kick").is_empty());
}

#[test]
fn refused_when_a_target_lane_no_longer_exists() {
    let mut c = composer();
    let gone = BatchHit { lane: SampleIdentity::File("gone.wav".into()), ..hit("kick", 1000.0, 1.0) };
    assert_eq!(c.add_batch("Drum hit detection", &[hit("kick", 1000.0, 1.0), gone.clone()], &BatchOptions::default()).err(),
        Some(BatchRefusal::MissingLane(gone.lane.clone())));
    assert!(c.hitsounds().lane(&gone.lane).is_none());
    assert!(hits_on(&c, "kick").is_empty());
    assert!(!c.history.can_undo());
}

#[test]
fn hits_attach_only_to_object_moments() {
    let mut c = composer();
    // A tom hit with no object at 1200 makes a no-object moment there.
    c.add_batch("Drum hit detection", &[hit("tom", 1250.0, 1.0)], &BatchOptions::default()).unwrap();
    // A kick 10 ms from that moment is within the object distance but must not attach to it as an object.
    let plan = c.plan_batch(&[hit("kick", 1240.0, 1.0)], &BatchOptions::default());
    assert!(matches!(plan.rows[0].lands, Lands::Loose));
    assert_eq!(plan.rows[0].time, 1250.0);
}

#[test]
fn loose_hits_share_no_object_moments() {
    let mut c = composer();
    c.add_batch("Drum hit detection", &[hit("kick", 1252.0, 0.9), hit("snare", 1248.0, 0.8)], &BatchOptions::default())
        .unwrap();
    let loose: Vec<_> = c.hitsounds().moments().iter().filter(|m| m.place() == Place::NoObject).collect();
    assert_eq!(loose.len(), 1);
    assert_eq!(loose[0].time().ms(), 1250.0);
    assert_eq!(loose[0].volume(), c.insert_volume);
    // A later batch reuses the moment made earlier.
    c.add_batch("Drum hit detection", &[hit("tom", 1251.0, 1.0)], &BatchOptions::default()).unwrap();
    assert_eq!(c.hitsounds().moments().iter().filter(|m| m.place() == Place::NoObject).count(), 1);
}

#[test]
fn hits_too_far_from_the_grid_are_flagged_not_moved() {
    let c = composer();
    // A 1/4 step is 125 ms, so the distance is min(20 ms, 41.7 ms) = 20 ms.
    let plan = c.plan_batch(&[hit("kick", 1146.0, 1.0), hit("snare", 1144.0, 1.0)], &BatchOptions::default());
    assert!(matches!(plan.rows[0].lands, Lands::Flagged));
    assert_eq!(plan.rows[0].time, 1146.0);
    assert!(matches!(plan.rows[1].lands, Lands::Loose));
    assert_eq!(plan.rows[1].time, 1125.0);
    assert_eq!((plan.counts.flagged, plan.counts.added), (1, 1));
    // At 1/16 the step is 31.25 ms, so the distance narrows to about 10.4 ms.
    let fine = BatchHit { grid: on_grid(1262.0, &grid, &[16], 0.0), ..hit("kick", 1262.0, 1.0) };
    assert!(matches!(c.plan_batch(&[fine], &BatchOptions::default()).rows[0].lands, Lands::Flagged));
}

#[test]
fn before_the_first_red_line_nothing_is_flagged() {
    let mut c = composer();
    c.tempo = TempoMap::new(vec![TimingSection { start: SongTime::from_ms(2000.0), beat_length_ms: 500.0, ..TimingSection::DEFAULT }]);
    let (snapped, step) = c.grid_line(1237.0, 4);
    assert!(step.is_infinite());
    let early = BatchHit { grid: on_grid(1237.0, &|t, d| c.grid_line(t, d), &[4], 0.0), ..hit("kick", 1237.0, 1.0) };
    let plan = c.plan_batch(&[early], &BatchOptions::default());
    assert!(matches!(plan.rows[0].lands, Lands::Loose));
    assert_eq!(plan.rows[0].time, snapped);
}

#[test]
fn the_editor_grid_follows_the_tempo_and_snap_beats() {
    let c = composer();
    assert_eq!(c.grid_line(1140.0, 4), (1125.0, 125.0));
    let (triplet, step) = c.grid_line(1140.0, 3);
    assert!((triplet - 1166.667).abs() < 0.001 && (step - 500.0 / 3.0).abs() < 1e-9);
}

#[test]
fn a_constant_lag_moves_hits_onto_their_lines_and_objects() {
    let c = composer();
    let late: Vec<f64> = (0..12).map(|i| 2000.0 + f64::from(i) * 125.0 + 28.0).chain([1028.0]).collect();
    let shift = grid_shift(late.iter().copied(), &grid, 4);
    assert!((shift - 28.0).abs() < 1e-9);
    let hits: Vec<_> = late.iter().map(|&t| BatchHit { grid: on_grid(t, &grid, &[4], shift), ..hit("kick", t, 1.0) }).collect();
    let plan = c.plan_batch(&hits, &BatchOptions { shift_ms: shift, ..BatchOptions::default() });
    assert_eq!(plan.counts.flagged, 0);
    assert!(matches!(plan.rows[12].lands, Lands::Object(_)));
    assert_eq!(plan.rows[12].time, 1000.0);
    assert!(plan.rows[..12].iter().enumerate().all(|(i, r)| r.time == 2000.0 + i as f64 * 125.0));
}

#[test]
fn stronger_hits_win_and_existing_hits_count_as_duplicates() {
    let mut c = composer();
    c.add(1000.0, lane("kick"), 80, 5.0);
    let plan = c.plan_batch(&[hit("kick", 1005.0, 0.5), hit("snare", 1255.0, 0.4), hit("snare", 1247.0, 0.9)],
        &BatchOptions::default());
    assert_eq!(plan.counts.duplicates, 2);
    assert!(matches!(plan.rows[2].lands, Lands::Loose));
    assert!(matches!(plan.rows[1].lands, Lands::Duplicate));
}

#[test]
fn the_range_includes_its_start_and_excludes_its_end() {
    let mut c = composer();
    let options = BatchOptions { range: Some((1010.0, 1510.0)), ..BatchOptions::default() };
    let plan = c.plan_batch(&[hit("kick", 1001.0, 1.0), hit("snare", 1490.0, 1.0), hit("kick", 1510.0, 1.0),
        hit("kick", 1010.0, 0.5)], &options);
    assert!(matches!(plan.rows[0].lands, Lands::Skipped));
    assert!(matches!(plan.rows[1].lands, Lands::Object(_)));
    assert!(matches!(plan.rows[2].lands, Lands::Skipped));
    // The object at 1000 is outside the range and so is its grid line: the hit is flagged, never placed.
    assert!(matches!(plan.rows[3].lands, Lands::Flagged));
    assert_eq!(plan.rows[3].time, 1010.0);
    let before = c.hitsounds().moments().len();
    c.add_batch("Drum hit detection", &[hit("kick", 1010.0, 0.5), hit("snare", 1260.0, 1.0)], &options).unwrap();
    let added: Vec<f64> = c.hitsounds().moments()[before..].iter().map(|m| m.time().ms()).collect();
    assert_eq!(added, vec![1250.0]);
    assert!(c.hitsounds().moments().iter().filter(|m| m.has_hit(&lane("kick")) || m.has_hit(&lane("snare")))
        .all(|m| (1010.0..1510.0).contains(&m.time().ms())));
}

#[test]
fn a_loose_hit_reuses_the_nearest_no_object_moment() {
    let mut c = composer();
    let at = |time: f64, line: f64| BatchHit { grid: Some(line), ..hit("tom", time, 1.0) };
    c.add_batch("Drum hit detection", &[at(1248.9, 1248.9), at(1251.1, 1251.1)], &BatchOptions::default()).unwrap();
    let plan = c.plan_batch(&[BatchHit { grid: Some(1250.5), ..hit("kick", 1250.5, 1.0) }], &BatchOptions::default());
    assert!(matches!(plan.rows[0].lands, Lands::Loose));
    assert_eq!(plan.rows[0].time, 1251.1);
}

#[test]
fn applying_shows_a_hidden_target_lane() {
    let mut c = composer();
    c.history.apply("Hide", false, |h| { h.set_lane_hidden(&lane("kick"), true); Ok::<_, ()>(()) }).unwrap();
    c.add_batch("Drum hit detection", &[hit("kick", 1000.0, 1.0)], &BatchOptions::default()).unwrap();
    assert!(!c.hitsounds().is_lane_hidden(&lane("kick")));
}

// ---- Pattern stamp ----

fn select_hits(c: &mut Composer, targets: &[(&str, f64)]) {
    let keys: Vec<HitKey> = targets
        .iter()
        .map(|&(name, time)| {
            let l = lane(name);
            let moment = c
                .hitsounds()
                .moments()
                .iter()
                .position(|m| (m.time().ms() - time).abs() < 0.5 && m.has_hit(&l))
                .expect("selected hit exists");
            HitKey { moment, lane: l }
        })
        .collect();
    c.selected.replace(keys);
}

#[test]
fn pattern_stamp_repeats_two_lane_kick_snare_four_times_with_inferred_bar() {
    // 120 BPM, 4/4 => bar length = 2000 ms.
    let mut c = composer();
    c.add(1000.0, lane("kick"), 80, 1.0);
    c.add(1500.0, lane("snare"), 90, 1.0);
    select_hits(&mut c, &[("kick", 1000.0), ("snare", 1500.0)]);
    assert_eq!(c.pattern_stamp_step(), Some(2000.0));

    let added = c.pattern_stamp(&PatternStampOptions {
        repeat_count: 4,
        step_ms: 0.0, // infer 1-bar = 2000 ms
        snap_to_objects: false,
        object_leniency_ms: 15.0,
        quantize_volumes: false,
        clear_existing: false,
    });
    assert_eq!(added, 8);
    assert_eq!(
        hits_on(&c, "kick").into_iter().map(|(t, _)| t).collect::<Vec<_>>(),
        vec![1000.0, 3000.0, 5000.0, 7000.0, 9000.0]
    );
    assert_eq!(
        hits_on(&c, "snare").into_iter().map(|(t, _)| t).collect::<Vec<_>>(),
        vec![1500.0, 3500.0, 5500.0, 7500.0, 9500.0]
    );
    assert_eq!(c.selected.items().len(), 8);
}

#[test]
fn pattern_stamp_snaps_to_objects_within_leniency_and_respects_toggle() {
    // Object at 3010 ms is within 15 ms of 3000 ms; object at 5025 ms is outside 15 ms of 5000 ms.
    let osu = "osu file format v14\n[General]\nSampleSet:Soft\nMode:0\n[Metadata]\nVersion:Test\n\
        [TimingPoints]\n0,500,4,2,0,45,1,0\n\
        [HitObjects]\n\
        100,100,1000,1,0,0:0:0:0:\n\
        100,100,3010,1,0,0:0:0:0:\n\
        100,100,5025,1,0,0:0:0:0:\n";
    let mut c = composer_for(osu);
    c.add(1000.0, lane("kick"), 80, 1.0);
    select_hits(&mut c, &[("kick", 1000.0)]);

    let added = c.pattern_stamp(&PatternStampOptions {
        repeat_count: 2,
        step_ms: 2000.0,
        snap_to_objects: true,
        object_leniency_ms: 15.0,
        quantize_volumes: false,
        clear_existing: false,
    });
    assert_eq!(added, 2);
    assert_eq!(
        hits_on(&c, "kick"),
        vec![(1000.0, true), (3010.0, true), (5000.0, false)]
    );

    // Undo and stamp with snap_to_objects = false: 3000 ms stays loose, does not snap to 3010 ms.
    c.undo();
    select_hits(&mut c, &[("kick", 1000.0)]);
    let added = c.pattern_stamp(&PatternStampOptions {
        repeat_count: 1,
        step_ms: 2000.0,
        snap_to_objects: false,
        object_leniency_ms: 15.0,
        quantize_volumes: false,
        clear_existing: false,
    });
    assert_eq!(added, 1);
    assert_eq!(hits_on(&c, "kick"), vec![(1000.0, true), (3000.0, false)]);
}

#[test]
fn pattern_stamp_quantizes_volumes_to_three_clean_levels() {
    assert_eq!(quantize_stamp_volume(20), 35);
    assert_eq!(quantize_stamp_volume(52), 35);
    assert_eq!(quantize_stamp_volume(53), 70);
    assert_eq!(quantize_stamp_volume(85), 70);
    assert_eq!(quantize_stamp_volume(86), 100);
    assert_eq!(quantize_stamp_volume(100), 100);

    let mut c = composer();
    c.add(1000.0, lane("kick"), 22, 1.0);
    c.add(1500.0, lane("snare"), 64, 1.0);
    c.add(1750.0, lane("tom"), 92, 1.0);
    // Existing object moments keep their timing-point volume (45%) on `add`, so set explicit per-hit volumes.
    c.apply("Set volumes", |h| {
        for (name, time, vol) in [("kick", 1000.0, 22), ("snare", 1500.0, 64), ("tom", 1750.0, 92)] {
            let l = lane(name);
            let id = h
                .moments()
                .iter()
                .position(|m| (m.time().ms() - time).abs() < 0.5 && m.has_hit(&l))
                .expect("hit exists");
            h.set_hit_volume(dawsu_hitsounding::MomentId(id), &l, vol);
        }
    });
    select_hits(&mut c, &[("kick", 1000.0), ("snare", 1500.0), ("tom", 1750.0)]);

    let added = c.pattern_stamp(&PatternStampOptions {
        repeat_count: 1,
        step_ms: 2000.0,
        snap_to_objects: false,
        object_leniency_ms: 15.0,
        quantize_volumes: true,
        clear_existing: false,
    });
    assert_eq!(added, 3);

    let volume_at = |name: &str, time: f64| {
        let l = lane(name);
        c.hitsounds()
            .moments()
            .iter()
            .find(|m| (m.time().ms() - time).abs() < 0.5 && m.has_hit(&l))
            .map(|m| m.hit_volume(&l))
    };
    assert_eq!(volume_at("kick", 3000.0), Some(35));
    assert_eq!(volume_at("snare", 3500.0), Some(70));
    assert_eq!(volume_at("tom", 3750.0), Some(100));
}

#[test]
fn pattern_stamp_rolls_back_in_a_single_undo_step() {
    let mut c = composer();
    c.add(1000.0, lane("kick"), 80, 1.0);
    c.add(1500.0, lane("snare"), 85, 1.0);
    select_hits(&mut c, &[("kick", 1000.0), ("snare", 1500.0)]);
    let steps_before = c.history.steps().len();

    let added = c.pattern_stamp(&PatternStampOptions {
        repeat_count: 3,
        step_ms: 2000.0,
        snap_to_objects: true,
        object_leniency_ms: 15.0,
        quantize_volumes: true,
        clear_existing: false,
    });
    assert_eq!(added, 6);
    assert_eq!(c.history.steps().len(), steps_before + 1);
    assert_eq!(c.history.steps().last().copied(), Some("Pattern stamp"));

    c.undo();
    assert_eq!(hits_on(&c, "kick"), vec![(1000.0, true)]);
    assert_eq!(hits_on(&c, "snare"), vec![(1500.0, true)]);
}
