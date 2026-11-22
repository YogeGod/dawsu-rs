use dawsu_application::{Document, drum_classify, save_document};
use dawsu_foundation::{CustomIndex, Hitsound, SampleIdentity, SampleSet, Sound};
use dawsu_hitsounding::LaneSource;
use dawsu_hitsounding::model::Lane;
use dawsu_test_support::Scratch;
use std::fs;
use std::path::{Path, PathBuf};

const CORPUS_DIR: &str = r"E:\GitHub\dawsu-hitsounding-editor\corpus";

fn corpus_osu_files() -> Vec<PathBuf> {
    let root = Path::new(CORPUS_DIR);
    if !root.is_dir() {
        return Vec::new();
    }
    let mut files = Vec::new();
    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("osu") {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

fn clap_normal() -> SampleIdentity {
    SampleIdentity::Standard {
        set: SampleSet::Normal,
        sound: Sound::Hit(Hitsound::Clap),
        index: CustomIndex::new(0),
    }
}

fn soak_one_map(source: &Path, idx: usize) -> Result<(), String> {
    let scratch = Scratch::new(&format!("corpus-soak-{idx}"));
    let copy = scratch.path("map.osu");
    let backups = scratch.path("backups");
    let original_bytes = fs::read(source).map_err(|e| format!("read failed: {e}"))?;
    fs::write(&copy, &original_bytes).map_err(|e| format!("write copy failed: {e}"))?;

    let mut doc = match Document::read(&copy) {
        Ok(d) => d,
        Err(e) => {
            // Non-standard modes or intentionally malformed corpus files may be rejected cleanly.
            return Err(format!("Document::read rejected cleanly: {e}"));
        }
    };

    // 1. No-op save must leave the .osu file byte-identical.
    let noop_result = save_document(&doc, backups.clone())
        .map_err(|e| format!("no-op save failed: {e}"))?;
    let after_noop = fs::read(&copy).map_err(|e| format!("read after no-op failed: {e}"))?;
    if after_noop != original_bytes {
        return Err(format!(
            "no-op save modified bytes ({} -> {})",
            original_bytes.len(),
            after_noop.len()
        ));
    }
    doc.file = noop_result.file;

    // 2. Edit -> Undo -> Redo -> Save -> Reopen round-trip.
    let first_moment = doc.composer.hitsounds().moments().first();
    let target_time = first_moment.map(|m| m.time().ms()).unwrap_or(1000.0);

    // Pick a standard hitsound not already on the first moment, matching its set/index if any.
    let existing_set = first_moment
        .and_then(|m| {
            m.hits().find_map(|id| match id {
                SampleIdentity::Standard { set, .. } => Some(*set),
                _ => None,
            })
        })
        .unwrap_or(SampleSet::Normal);
    let existing_index = first_moment
        .and_then(|m| {
            m.hits().find_map(|id| match id {
                SampleIdentity::Standard { index, .. } => Some(*index),
                _ => None,
            })
        })
        .unwrap_or_else(|| CustomIndex::new(0));

    let candidate_sound = [Hitsound::Clap, Hitsound::Whistle, Hitsound::Finish, Hitsound::Normal]
        .into_iter()
        .find(|hs| {
            !first_moment.is_some_and(|m| {
                m.hits().any(|id| {
                    matches!(
                        id,
                        SampleIdentity::Standard { sound: Sound::Hit(s), .. } if *s == *hs
                    )
                })
            })
        })
        .unwrap_or(Hitsound::Clap);

    let identity = SampleIdentity::Standard {
        set: existing_set,
        sound: Sound::Hit(candidate_sound),
        index: existing_index,
    };

    doc.composer.add(target_time, identity.clone(), 75, 5.0);
    doc.composer.undo();
    doc.composer.redo();

    let edit_report = match save_document(&doc, backups) {
        Ok(report) => report,
        Err(e) if e.starts_with("Save needs review:") => {
            return Err(format!("Save needs review (expected safety refusal): {e}"));
        }
        Err(e) => return Err(format!("edit save failed: {e}")),
    };
    let reopened =
        Document::read(&copy).map_err(|e| format!("reopen after save failed: {e}"))?;
    if edit_report.problems.is_empty() {
        let has_hit_after = reopened
            .composer
            .hitsounds()
            .moments()
            .iter()
            .find(|m| (m.time().ms() - target_time).abs() < 1.0)
            .is_some_and(|m| {
                m.hits().any(|id| {
                    matches!(
                        id,
                        SampleIdentity::Standard { sound: Sound::Hit(s), .. } if *s == candidate_sound
                    )
                })
            });

        if !has_hit_after {
            return Err(format!(
                "round-trip hit missing at {target_time}ms for {identity:?} (problems={:?})",
                edit_report.problems
            ));
        }
    }

    Ok(())
}

#[test]
fn corpus_sample_soak_25_maps() {
    let files = corpus_osu_files();
    if files.is_empty() {
        eprintln!("Skipping corpus_sample_soak_25_maps: corpus not present at {CORPUS_DIR}");
        return;
    }
    let step = (files.len() / 25).max(1);
    let mut tested = 0usize;
    let mut rejected = 0usize;
    let mut failures = Vec::new();

    for (idx, path) in files.iter().step_by(step).take(25).enumerate() {
        tested += 1;
        match soak_one_map(path, idx) {
            Ok(()) => {}
            Err(msg) if msg.starts_with("Document::read rejected cleanly:") => {
                rejected += 1;
            }
            Err(msg) => failures.push(format!("{}: {msg}", path.display())),
        }
    }
    eprintln!(
        "corpus_sample_soak_25_maps: tested={tested}, passed={}, rejected={rejected}, failed={}",
        tested - rejected - failures.len(),
        failures.len()
    );
    assert!(
        failures.is_empty(),
        "Corpus sample soak failures:\n{}",
        failures.join("\n")
    );
}

#[test]
#[ignore]
fn corpus_full_soak_all_maps() {
    let files = corpus_osu_files();
    if files.is_empty() {
        eprintln!("Skipping corpus_full_soak_all_maps: corpus not present at {CORPUS_DIR}");
        return;
    }
    let mut tested = 0usize;
    let mut rejected = 0usize;
    let mut review_refused = 0usize;
    let mut failures = Vec::new();

    for (idx, path) in files.iter().enumerate() {
        tested += 1;
        match soak_one_map(path, idx) {
            Ok(()) => {}
            Err(msg) if msg.starts_with("Document::read rejected cleanly:") => {
                rejected += 1;
            }
            Err(msg) if msg.starts_with("Save needs review (expected safety refusal):") => {
                review_refused += 1;
            }
            Err(msg) => failures.push(format!("{}: {msg}", path.display())),
        }
    }
    eprintln!(
        "corpus_full_soak_all_maps: tested={tested}, passed={}, rejected={rejected}, review_refused={review_refused}, failed={}",
        tested - rejected - review_refused - failures.len(),
        failures.len()
    );
    assert!(
        failures.is_empty(),
        "Corpus full soak failures ({}/{}):\n{}",
        failures.len(),
        tested,
        failures.iter().take(25).cloned().collect::<Vec<_>>().join("\n")
    );
}

#[test]
fn drum_classifier_handles_lanes_without_panic() {
    let lanes = vec![
        Lane {
            identity: SampleIdentity::File("custom-kick.wav".into()),
            source: LaneSource::File("custom-kick.wav".into()),
        },
        Lane {
            identity: clap_normal(),
            source: LaneSource::Skin,
        },
    ];
    let kit = drum_classify::auto_detect_drum_map(&lanes, |_| None);
    kit.validate().unwrap();
    assert!(!kit.rows.is_empty());
}
