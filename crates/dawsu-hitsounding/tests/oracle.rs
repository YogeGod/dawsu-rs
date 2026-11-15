//! The hitsound rules' one answer compared with lazer's own results (Task 6, ADR 0036, TS-13).
//! Times must be equal where no circle maths is involved, and within 0.01 ms for sliders with a
//! perfect-circle curve. Spinner bonus ticks are not compared: they are spin rewards, not parts
//! of an object that a map's hitsounds set (ADR 0036 lists the spinner's end).

use dawsu_hitsounding::rules::{answer, Answer, Kind, ObjectAnswer, PartKind, Sample};
use dawsu_osu_format::OsuFile;
use dawsu_test_support::serde_json::Value;
use dawsu_test_support::{oracle_case_files, oracle_result, oracle_results_dir, save_pack_files};
use std::path::{Path, PathBuf};

const TOLERANCE_MS: f64 = 0.01;

fn number(v: &Value) -> f64 {
    match v {
        Value::Number(n) => n.as_f64().unwrap(),
        Value::String(s) if s == "Infinity" => f64::INFINITY,
        Value::String(s) if s == "-Infinity" => f64::NEG_INFINITY,
        Value::String(s) if s == "NaN" => f64::NAN,
        other => panic!("not a number: {other}"),
    }
}

struct Check<'a> {
    file: &'a str,
    mismatches: Vec<String>,
}

impl Check<'_> {
    fn time(&mut self, what: &str, ours: f64, lazer: f64, tolerant: bool) {
        let ok = if tolerant { (ours - lazer).abs() <= TOLERANCE_MS } else { ours == lazer || (ours.is_infinite() && ours == lazer) };
        if !ok {
            self.mismatches.push(format!("{}: {what}: ours {ours}, lazer {lazer}", self.file));
        }
    }

    fn eq<T: PartialEq + std::fmt::Debug>(&mut self, what: &str, ours: T, lazer: T) {
        if ours != lazer {
            self.mismatches.push(format!("{}: {what}: ours {ours:?}, lazer {lazer:?}", self.file));
        }
    }

    fn samples(&mut self, what: &str, ours: &[Sample], lazer: &Value) {
        let lazer = lazer.as_array().unwrap();
        if ours.len() != lazer.len() {
            self.mismatches.push(format!("{}: {what}: {} samples, lazer {}", self.file, ours.len(), lazer.len()));
            return;
        }
        for (i, (o, l)) in ours.iter().zip(lazer).enumerate() {
            let w = format!("{what} sample {i}");
            let s = &o.lazer;
            self.eq(&format!("{w} kind"), s.kind(), l["kind"].as_str().unwrap());
            self.eq(&format!("{w} name"), s.name.as_str(), l["name"].as_str().unwrap());
            self.eq(&format!("{w} bank"), s.bank.as_str(), l["bank"].as_str().unwrap());
            self.eq(&format!("{w} suffix"), s.suffix.as_deref(), l["suffix"].as_str());
            self.eq(&format!("{w} volume"), i64::from(s.volume), l["volume"].as_i64().unwrap());
            self.eq(&format!("{w} editor auto bank"), s.editor_auto_bank, l["editor_auto_bank"].as_bool().unwrap());
            self.eq(&format!("{w} uses beatmap samples"), s.use_beatmap_samples, l["use_beatmap_samples"].as_bool().unwrap());
            self.eq(&format!("{w} custom sample bank"), i64::from(s.custom_sample_bank()), l["custom_sample_bank"].as_i64().unwrap());
            self.eq(&format!("{w} layered"), s.is_layered, l["is_layered"].as_bool().unwrap());
            self.eq(&format!("{w} bank specified"), s.bank_specified, l["bank_specified"].as_bool().unwrap());
            self.eq(&format!("{w} file"), s.filename.as_deref(), l["filename"].as_str());
            let names: Vec<&str> = l["lookup_names"].as_array().unwrap().iter().map(|n| n.as_str().unwrap()).collect();
            self.eq(&format!("{w} lookup names"), o.lookup_names.iter().map(String::as_str).collect::<Vec<_>>(), names);
        }
    }

    fn object(&mut self, i: usize, ours: &ObjectAnswer, lazer: &Value, curve_is_circular: bool) {
        let w = format!("object {i} at {}", ours.start_time);
        let lazer_type = lazer["type"].as_str().unwrap();
        let expected = match ours.kind {
            Kind::Circle => "HitCircle",
            Kind::Slider => "Slider",
            Kind::Spinner => "Spinner",
        };
        self.eq(&format!("{w} type"), expected, lazer_type);
        self.time(&format!("{w} start"), ours.start_time, number(&lazer["start_time"]), false);
        if let Some(end) = ours.end_time {
            self.time(&format!("{w} end"), end, number(&lazer["end_time"]), curve_is_circular);
        }
        self.samples(&format!("{w} samples"), &ours.samples, &lazer["samples"]);

        let Some(slider) = &ours.slider else { return };
        let l = &lazer["slider"];
        self.time(&format!("{w} beat length"), slider.beat_length, number(&l["beat_length"]), false);
        self.time(&format!("{w} slider velocity"), slider.slider_velocity_multiplier, number(&l["slider_velocity_multiplier"]), false);
        self.time(&format!("{w} velocity"), slider.velocity, number(&l["velocity"]), false);
        self.time(&format!("{w} path distance"), slider.path_distance, number(&l["path_distance"]), curve_is_circular);
        self.time(&format!("{w} span duration"), slider.span_duration, number(&l["span_duration"]), curve_is_circular);
        self.time(&format!("{w} tick distance"), slider.tick_distance, number(&l["tick_distance"]), false);
        self.time(&format!("{w} tick distance multiplier"), slider.tick_distance_multiplier, number(&l["tick_distance_multiplier"]), false);
        self.eq(&format!("{w} generates ticks"), slider.generate_ticks, l["generate_ticks"].as_bool().unwrap());
        self.eq(&format!("{w} repeats"), slider.repeat_count as i64, l["repeat_count"].as_i64().unwrap());
        self.samples(&format!("{w} tail samples"), &slider.tail_samples, &lazer["tail_samples"]);
        self.samples(&format!("{w} sliding samples"), &slider.sliding_samples, &lazer["sliding_samples"]);

        let nested = lazer["nested"].as_array().unwrap();
        if slider.parts.len() != nested.len() {
            self.mismatches.push(format!("{}: {w}: {} parts, lazer {}", self.file, slider.parts.len(), nested.len()));
            return;
        }
        for (j, (p, n)) in slider.parts.iter().zip(nested).enumerate() {
            let pw = format!("{w} part {j}");
            let kind = match p.kind {
                PartKind::Head => "SliderHeadCircle",
                PartKind::Tick => "SliderTick",
                PartKind::Repeat => "SliderRepeat",
                PartKind::Tail => "SliderTailCircle",
            };
            self.eq(&format!("{pw} type"), kind, n["type"].as_str().unwrap());
            self.time(&format!("{pw} time"), p.time, number(&n["start_time"]), curve_is_circular);
            self.samples(&pw, &p.samples, &n["samples"]);
        }
    }
}

/// Compares one map with its oracle result; returns the mismatches (empty when all agree).
fn compare(name: &str, bytes: &[u8], lazer: &Value) -> Vec<String> {
    let file = OsuFile::read(bytes.to_vec()).file;
    let mut check = Check { file: name, mismatches: Vec::new() };
    match answer(&file, &[]) {
        Answer::UnsupportedMode { mode } => {
            check.eq("mode", i64::from(mode), lazer["header"]["mode"].as_i64().unwrap());
            check.eq("an unsupported mode is not osu!", lazer["header"]["ruleset"].as_str() == Some("osu"), false);
        }
        Answer::Objects(objects) => {
            let lazer_objects = lazer["hit_objects"].as_array().unwrap();
            if objects.len() != lazer_objects.len() {
                check.mismatches.push(format!("{name}: {} objects, lazer {}", objects.len(), lazer_objects.len()));
                return check.mismatches;
            }
            let curves = file.hit_objects();
            for (i, (o, l)) in objects.iter().zip(lazer_objects).enumerate() {
                let circular = curves.iter().find(|h| h.line() == o.line).and_then(|h| h.slider()).is_some_and(|s| s.curve.starts_with('P') || s.curve.contains("|P"));
                check.object(i, o, l, circular);
            }
        }
    }
    check.mismatches
}

fn assert_agrees(group: &str, files: Vec<(String, PathBuf)>, results: &Path) {
    let mut all = Vec::new();
    for (name, path) in &files {
        let bytes = std::fs::read(path).unwrap();
        let lazer = oracle_result(results, group, name, &bytes);
        all.extend(compare(name, &bytes, &lazer));
    }
    assert!(all.is_empty(), "{} mismatches with lazer, first ones:\n{}", all.len(), all.iter().take(25).cloned().collect::<Vec<_>>().join("\n"));
}

#[test]
fn every_save_pack_map_matches_lazer() {
    assert_agrees("save-pack", save_pack_files(), &oracle_results_dir());
}

#[test]
fn the_hand_made_slider_velocity_cases_match_lazer() {
    // before-first-line, at-red-and-green-pair, after-red-following-green, green-before-first-red
    assert_eq!(oracle_case_files().len(), 4);
    assert_agrees("oracle-cases", oracle_case_files(), &oracle_results_dir());
}

/// The local corpus (not committed, ADR 0026 point 2). Set `DAWSU_CORPUS` to its folder and run
/// the lazer oracle over it into `target/lazer-oracle-corpus/` first (see tools/lazer-oracle/README.md).
#[test]
fn the_local_corpus_matches_lazer() {
    let Ok(corpus) = std::env::var("DAWSU_CORPUS") else {
        eprintln!("skipped: DAWSU_CORPUS is not set");
        return;
    };
    let corpus = PathBuf::from(corpus);
    let group = corpus.file_name().unwrap().to_string_lossy().into_owned();
    let results = dawsu_test_support::workspace_root().join("target").join("lazer-oracle-corpus");
    if !results.join(&group).exists() {
        eprintln!("skipped: no oracle results in {}", results.join(&group).display());
        return;
    }
    let mut files: Vec<(String, PathBuf)> = std::fs::read_dir(&corpus)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "osu"))
        .map(|p| (p.file_name().unwrap().to_string_lossy().into_owned(), p))
        .collect();
    files.sort();
    // The 10 maps holding the 14 sliders whose span Spike B found wrong in the old port
    // (E:\dawsu-rewrite-spikes\spike-b-lazer\parity-report.txt, "SPAN" lines).
    for prefix in ["18fce1cc174e", "611fe5077d84", "77e2e03752af", "78cc704e998a", "7f39d930e1b4", "9eba7e89e9ea", "a5b10e7a1b3a", "a9ef204b763b", "b7214576e61e", "cd6cca0e871b"] {
        assert!(files.iter().any(|(n, _)| n.starts_with(prefix)), "the corpus lacks Spike B's map {prefix}");
    }
    let mut all = Vec::new();
    let mut maps_with_mismatches = 0;
    for (name, path) in &files {
        let bytes = std::fs::read(path).unwrap();
        let lazer = oracle_result(&results, &group, name, &bytes);
        let m = compare(name, &bytes, &lazer);
        if !m.is_empty() {
            maps_with_mismatches += 1;
        }
        all.extend(m);
    }
    eprintln!("corpus: {} maps compared, {maps_with_mismatches} with mismatches", files.len());
    assert!(all.is_empty(), "{} mismatches in {maps_with_mismatches} corpus maps, first ones:\n{}", all.len(), all.iter().take(25).cloned().collect::<Vec<_>>().join("\n"));
}
