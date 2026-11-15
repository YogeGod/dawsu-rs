//! Green-line clean-up (hitsound domain design doc Responsibility 21; the last-four-parts plan,
//! part A). Old test names are from the old app's `GreenLineCleanupTests.cs` at c81774e.

use dawsu_hitsounding::cleanup::clean_up_green_lines;
use dawsu_hitsounding::rules::answer;
use dawsu_osu_format::OsuFile;

fn read(bytes: &[u8]) -> OsuFile {
    OsuFile::read(bytes.to_vec()).file
}

fn map(set: &str, timing: &str) -> OsuFile {
    read(format!("osu file format v14\r\n\r\n[General]\r\nSampleSet: {set}\r\n\r\n[Difficulty]\r\nSliderMultiplier:1.4\r\nSliderTickRate:1\r\n\r\n[TimingPoints]\r\n{timing}\r\n\r\n[HitObjects]\r\n256,192,250,1,0,0:0:0:0:\r\n256,192,750,1,0,0:0:0:0:\r\n100,100,1250,2,0,L|200:100,1,140\r\n256,192,1750,1,0,0:0:0:0:\r\n").as_bytes())
}

fn timing_times(file: &OsuFile) -> Vec<f64> {
    file.timing_points().iter().map(|t| t.time()).collect()
}

/// Kiai sections as lazer reads them: per time the last line wins, then each change.
fn kiai(file: &OsuFile) -> Vec<(f64, bool)> {
    let mut at: Vec<(f64, bool)> = Vec::new();
    for t in file.timing_points() {
        let on = t.effects().unwrap_or(0) & 1 != 0;
        match at.last_mut() {
            Some(last) if last.0 == t.time() => last.1 = on,
            _ => at.push((t.time(), on)),
        }
    }
    at.dedup_by(|b, a| a.1 == b.1);
    at
}

#[test]
fn repeated_green_lines_are_removed() {
    let file = map("Normal", "0,500,4,1,0,100,1,0\r\n500,-100,4,1,0,100,0,0\r\n1000,-50,4,1,0,100,0,0\r\n1500,-50,4,1,0,100,0,0");
    let c = clean_up_green_lines(&file);
    assert_eq!(c.removed, 2);
    assert_eq!(timing_times(&c.file), [0.0, 1000.0]);
}

#[test]
fn a_change_of_set_index_volume_velocity_or_kiai_keeps_the_line() {
    for green in ["500,-100,4,2,0,100,0,0", "500,-100,4,1,1,100,0,0", "500,-100,4,1,0,90,0,0", "500,-80,4,1,0,100,0,0", "500,-100,4,1,0,100,0,1"] {
        let file = map("Normal", &format!("0,500,4,1,0,100,1,0\r\n{green}"));
        assert_eq!(clean_up_green_lines(&file).removed, 0, "{green}");
    }
}

#[test]
fn set_0_is_not_the_same_as_no_set_field() {
    // lazer reads set 0 as normal and a missing set as [General] SampleSet, here soft.
    let file = map("Soft", "0,500,4\r\n500,-100,4,0,0,100,0,0");
    assert_eq!(clean_up_green_lines(&file).removed, 0);
    let file = map("Normal", "0,500,4,1,0,100,1,0\r\n500,-100,4,0,0,100,0,0");
    assert_eq!(clean_up_green_lines(&file).removed, 1, "with a normal default, set 0 changes nothing");
}

#[test]
fn red_lines_and_unreadable_lines_stay() {
    let file = map("Normal", "0,500,4,1,0,100,1,0\r\n0,500,4,1,0,100,1,0\r\n500,x,4,1,0,100,0,0\r\n1000,-100,4,1,0,100,0,0");
    let c = clean_up_green_lines(&file);
    assert_eq!(c.removed, 1, "only the green line at 1000");
    assert_eq!(c.file.lines().count(), file.lines().count() - 1);
}

#[test]
fn a_map_with_nothing_to_remove_stays_unedited() {
    let file = map("Normal", "0,500,4,1,0,100,1,0\r\n1000,-50,4,1,0,100,0,0");
    let c = clean_up_green_lines(&file);
    assert_eq!((c.removed, c.file.to_bytes()), (0, file.to_bytes()));
}

/// Every part's time, samples and volume, and the kiai sections, are the same after clean-up.
fn plays_the_same(name: &str, file: &OsuFile) -> usize {
    let c = clean_up_green_lines(file);
    let after = read(&c.file.to_bytes());
    if let (dawsu_hitsounding::rules::Answer::Objects(a), dawsu_hitsounding::rules::Answer::Objects(b)) = (answer(&c.file, &[]), answer(file, &[])) {
        assert_eq!(a.len(), b.len(), "{name}");
        for (x, y) in a.iter().zip(&b) {
            if x != y {
                let (sx, sy) = (format!("{x:?}"), format!("{y:?}"));
                let at = sx.bytes().zip(sy.bytes()).position(|(p, q)| p != q).unwrap_or(0);
                panic!("{name} at {}: after …{}… before …{}…", y.start_time, &sx[at.saturating_sub(150)..(at + 150).min(sx.len())], &sy[at.saturating_sub(150)..(at + 150).min(sy.len())]);
            }
        }
    }
    assert_eq!(kiai(&after), kiai(file), "{name}: kiai");
    assert_eq!(clean_up_green_lines(&after).removed, 0, "{name}: a second clean-up removes nothing");
    c.removed
}

#[test]
fn every_save_pack_map_plays_the_same_after_clean_up() {
    let mut removed = 0;
    for (name, path) in dawsu_test_support::save_pack_files() {
        removed += plays_the_same(&name, &read(&std::fs::read(&path).unwrap()));
    }
    eprintln!("save pack: {removed} green lines removed");
}

/// The local corpus (not committed): set `DAWSU_CORPUS` to its folder.
#[test]
fn the_local_corpus_plays_the_same_after_clean_up() {
    let Ok(corpus) = std::env::var("DAWSU_CORPUS") else {
        eprintln!("skipped: DAWSU_CORPUS is not set");
        return;
    };
    let (mut maps, mut removed, mut lines) = (0, 0, 0);
    let mut paths: Vec<_> = std::fs::read_dir(&corpus).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|e| e == "osu")).collect();
    paths.sort();
    for path in paths {
        let file = read(&std::fs::read(&path).unwrap());
        lines += file.timing_points().len();
        removed += plays_the_same(&path.file_name().unwrap().to_string_lossy(), &file);
        maps += 1;
    }
    eprintln!("corpus: {maps} maps, {removed} of {lines} timing lines removed");
    assert!(removed > 0);
}
