//! The map preview's playfield: circle size, approach time and stacking as osu!lazer works them
//! out at 131fe72 (`OsuHitObject.cs`, `OsuBeatmapProcessor.cs`). Expected heights are worked by hand
//! from lazer's algorithm on small synthetic maps.

use dawsu_hitsounding::lazer::vector::Vector2;
use dawsu_hitsounding::playfield::{playfield, Playfield};
use dawsu_hitsounding::rules::Kind;
use dawsu_osu_format::OsuFile;

fn map(version: u32, difficulty: &str, objects: &str) -> Playfield {
    let text = format!(
        "osu file format v{version}\n\n[General]\nMode: 0\nStackLeniency: 0.7\n\n[Difficulty]\n{difficulty}SliderMultiplier:1\nSliderTickRate:1\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n{objects}"
    );
    playfield(&OsuFile::read(text.into_bytes()).file).expect("an osu! map")
}

fn heights(p: &Playfield) -> Vec<i32> {
    p.objects.iter().map(|o| o.stack_height).collect()
}

#[test]
fn circle_size_and_approach_rate_set_radius_preempt_and_fade_in() {
    let p = map(14, "CircleSize:4\nOverallDifficulty:8\nApproachRate:9\n", "100,100,1000,1,0\n");
    assert!((p.radius - 64.0 * 0.570_233_7).abs() < 1e-4);
    assert_eq!(p.preempt, 600.0);
    assert_eq!(p.fade_in, 400.0);
}

#[test]
fn without_an_approach_rate_the_overall_difficulty_counts() {
    let p = map(14, "CircleSize:4\nOverallDifficulty:10\n", "100,100,1000,1,0\n");
    assert_eq!(p.preempt, 450.0);
    // TimeFadeIn = 400 × min(1, 450 / 450).
    assert_eq!(p.fade_in, 400.0);
}

#[test]
fn circles_on_one_spot_stack_up_and_left_from_the_last() {
    // Preempt 1200 at AR 5, threshold 1200 × 0.7 = 840 ms.
    let p = map(14, "ApproachRate:5\n", "200,200,1000,1,0\n200,200,1100,1,0\n201,201,1200,1,0\n");
    assert_eq!(heights(&p), vec![2, 1, 0]);
    let offset = p.objects[0].stack_offset(p.scale);
    assert_eq!(offset, Vector2::new(2.0 * p.scale * -6.4, 2.0 * p.scale * -6.4));
}

#[test]
fn circles_too_far_apart_in_time_do_not_stack() {
    let p = map(14, "ApproachRate:5\n", "200,200,1000,1,0\n200,200,1900,1,0\n");
    assert_eq!(heights(&p), vec![0, 0]);
}

#[test]
fn circles_on_a_slider_end_stack_down_and_right() {
    // A 100 px slider from 100,100 to 200,100; 1 beat at 500 ms (velocity 100 × 1 / 500 per ms).
    let p = map(14, "ApproachRate:5\n", "100,100,1000,2,0,L|200:100,1,100\n200,100,1600,1,0\n200,100,1700,1,0\n");
    assert_eq!(p.objects[0].kind, Kind::Slider);
    assert_eq!(p.objects[0].end_time, 1500.0);
    assert_eq!(p.objects[0].end_position(), Vector2::new(200.0, 100.0));
    assert_eq!(heights(&p), vec![0, -1, -2]);
}

#[test]
fn a_slider_with_two_spans_ends_where_it_started() {
    let p = map(14, "ApproachRate:5\n", "100,100,1000,2,0,L|200:100,2,100\n");
    let slider = &p.objects[0];
    assert_eq!(slider.span_count, 2);
    assert_eq!(slider.end_position(), Vector2::new(100.0, 100.0));
    assert_eq!(slider.curve_position_at(0.25), Vector2::new(50.0, 0.0));
    assert_eq!(slider.curve_position_at(0.75), Vector2::new(50.0, 0.0));
}

#[test]
fn spinners_never_stack() {
    let p = map(14, "ApproachRate:5\n", "256,192,1000,12,0,1500\n256,192,1600,1,0\n");
    assert_eq!(heights(&p), vec![0, 0]);
}

#[test]
fn maps_before_version_6_use_the_old_stacking() {
    // applyStackingOld counts up from the first object instead.
    let p = map(5, "ApproachRate:5\n", "200,200,1000,1,0\n200,200,1100,1,0\n");
    assert_eq!(heights(&p), vec![1, 0]);
}

#[test]
fn taiko_maps_have_no_playfield() {
    let text = "osu file format v14\n\n[General]\nMode: 1\n\n[HitObjects]\n100,100,1000,1,0\n";
    assert!(playfield(&OsuFile::read(text.as_bytes().to_vec()).file).is_none());
}

#[test]
fn every_save_pack_map_has_a_playfield_with_every_object() {
    use dawsu_hitsounding::rules::{answer, Answer};
    let mut checked = 0;
    for (name, path) in dawsu_test_support::save_pack_files() {
        let file = OsuFile::read(std::fs::read(&path).unwrap()).file;
        let Answer::Objects(objects) = answer(&file, &[]) else { continue };
        let field = playfield(&file).unwrap_or_else(|| panic!("{name}: no playfield"));
        assert_eq!(field.objects.len(), objects.len(), "{name}");
        for object in &field.objects {
            assert!(object.stack_offset(field.scale).x.is_finite(), "{name}");
            if let Some(path) = &object.path {
                assert!(path.cumulative.len() >= path.points.len(), "{name}: a length for every path point");
                assert!(object.end_position().x.is_finite(), "{name}");
            }
        }
        checked += 1;
    }
    assert!(checked > 0);
}

#[test]
fn extreme_times_wrap_as_lazer_does_instead_of_stopping() {
    // lazer subtracts the times as C# ints, which wrap; heights [1, 0] as in a release build.
    let p = map(14, "ApproachRate:5\n", "200,200,-2147483647,1,0\n200,200,2147483647,1,0\n");
    assert_eq!(heights(&p), vec![1, 0]);
}

#[test]
fn spinners_sit_in_the_playfield_centre() {
    let p = map(14, "ApproachRate:5\n", "10,20,1000,12,0,2000\n");
    assert_eq!(p.objects[0].position, Vector2::new(256.0, 192.0));
}

#[test]
fn sliders_carry_their_tick_times() {
    // 200 px at velocity 100 px per 500 ms beat, tick rate 1: one tick a beat, at 1500; the tail at 2000.
    let p = map(14, "ApproachRate:5\n", "100,100,1000,2,0,L|300:100,1,200\n");
    assert_eq!(p.objects[0].ticks, vec![1500.0]);
    let p = map(14, "ApproachRate:5\n", "100,100,1000,1,0\n");
    assert!(p.objects[0].ticks.is_empty());
}
