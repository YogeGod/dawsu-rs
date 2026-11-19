use dawsu_application::{Document, output};
use dawsu_test_support::Scratch;

const MAP:&[u8]=b"osu file format v14\n[General]\nMode:0\n[Metadata]\nArtist:Artist\nTitle:Title\nCreator:Mapper\nVersion:Hard\n[Difficulty]\nSliderMultiplier:1.4\nSliderTickRate:1\n[TimingPoints]\n0,500,4,1,0,100,1,0\n[HitObjects]\n256,192,1000,1,8,0:0:0:0:\n";
#[test]
fn generate_review_lists_the_output_and_writes_a_new_reopenable_difficulty() {
    let scratch = Scratch::new("output-generate");
    let path = scratch.path("source.osu");
    std::fs::write(&path, MAP).unwrap();
    let source = Document::read(&path).unwrap();
    let plan = output::generate(&output::Source::from_document(&source), &|| false).unwrap();
    assert_eq!(plan.files.len(), 1);
    assert_ne!(plan.files[0].path, path);
    let target = plan.files[0].path.clone();
    plan.apply(&scratch.path("backups"), &|| false).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), MAP);
    assert_eq!(
        Document::read(&target)
            .unwrap()
            .file
            .value("Metadata", "Version")
            .as_deref(),
        Some("Hitsounds")
    );
}
#[test]
fn copy_refuses_source_duplicate_and_foreign_targets_before_any_write() {
    let scratch = Scratch::new("output-copy");
    let path = scratch.path("source.osu");
    let target = scratch.path("target.osu");
    std::fs::write(&path, MAP).unwrap();
    std::fs::write(&target, MAP).unwrap();
    let source = output::Source::from_document(&Document::read(&path).unwrap());
    assert!(output::copy(&source, &[path.clone()], &Default::default(), &|| false).is_err());
    assert!(
        output::copy(
            &source,
            &[target.clone(), target.clone()],
            &Default::default(),
            &|| false
        )
        .is_err()
    );
    let foreign = Scratch::new("output-foreign");
    let other = foreign.path("other.osu");
    std::fs::write(&other, MAP).unwrap();
    assert!(output::copy(&source, &[other], &Default::default(), &|| false).is_err());
    assert_eq!(std::fs::read(&target).unwrap(), MAP);
}
#[test]
fn changed_target_invalidates_the_review_without_overwriting_it() {
    let scratch = Scratch::new("output-changed");
    let path = scratch.path("source.osu");
    let target = scratch.path("target.osu");
    std::fs::write(&path, MAP).unwrap();
    std::fs::write(&target, MAP).unwrap();
    let source = output::Source::from_document(&Document::read(&path).unwrap());
    let plan = output::copy(&source, &[target.clone()], &Default::default(), &|| false).unwrap();
    std::fs::write(&target, b"external edits").unwrap();
    assert!(plan.apply(&scratch.path("backups"), &|| false).is_err());
    assert_eq!(std::fs::read(&target).unwrap(), b"external edits");
}
#[test]
fn cancellation_after_one_copy_target_restores_it_but_preserves_a_newer_external_edit() {
    use std::cell::Cell;
    let scratch = Scratch::new("output-copy-recovery");
    let source_path = scratch.path("source.osu");
    let first = scratch.path("first.osu");
    let second = scratch.path("second.osu");
    let plain = String::from_utf8(MAP.to_vec())
        .unwrap()
        .replace("1000,1,8,", "1000,1,0,");
    std::fs::write(&source_path, MAP).unwrap();
    std::fs::write(&first, &plain).unwrap();
    std::fs::write(&second, &plain).unwrap();
    let source = output::Source::from_document(&Document::read(&source_path).unwrap());
    let plan = output::copy(
        &source,
        &[first.clone(), second.clone()],
        &Default::default(),
        &|| false,
    )
    .unwrap();
    let calls = Cell::new(0);
    let result = plan.apply(&scratch.path("backups"), &|| {
        let call = calls.get();
        calls.set(call + 1);
        call >= 1
    });
    assert!(result.is_err());
    assert_eq!(std::fs::read(&first).unwrap(), plain.as_bytes());
    assert_eq!(std::fs::read(&second).unwrap(), plain.as_bytes());
    let calls = Cell::new(0);
    let result = plan.apply(&scratch.path("backups"), &|| {
        let call = calls.get();
        calls.set(call + 1);
        if call == 1 {
            std::fs::write(&first, b"newer external edit").unwrap();
        }
        call >= 1
    });
    assert!(result.unwrap_err().contains("were kept"));
    assert_eq!(std::fs::read(&first).unwrap(), b"newer external edit");
    assert_eq!(std::fs::read(&second).unwrap(), plain.as_bytes());
}
#[test]
fn new_difficulty_and_generate_refusals_are_worded_for_mappers() {
    let scratch = Scratch::new("output-plain-refusals");
    let path = scratch.path("source.osu");
    std::fs::write(&path, MAP).unwrap();
    let taken_file = String::from_utf8(MAP.to_vec()).unwrap().replace("Version:Hard", "Version:Insane");
    std::fs::write(scratch.path("Artist - Title (Mapper) [Taken].osu"), taken_file).unwrap();
    let source = output::Source::from_document(&Document::read(&path).unwrap());
    let refusal = |result: Result<output::WritePlan, String>| result.err().expect("a refusal");
    assert_eq!(refusal(output::new_diff(&source, " ")), "Type a name for the new difficulty.");
    assert_eq!(
        refusal(output::new_diff(&source, "hard")),
        "A difficulty with that name already exists in this mapset. Choose another name."
    );
    assert_eq!(
        refusal(output::new_diff(&source, "Taken")),
        "A difficulty file with that name is already in the mapset folder. Choose another name."
    );
    let hitsounds = String::from_utf8(MAP.to_vec()).unwrap().replace("Version:Hard", "Version:Hitsounds");
    std::fs::write(scratch.path("other.osu"), hitsounds).unwrap();
    assert_eq!(
        refusal(output::generate(&source, &|| false)),
        "A difficulty in this mapset is already called \"Hitsounds\". Rename it, then generate again."
    );
}
