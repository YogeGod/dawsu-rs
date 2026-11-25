use dawsu_application::{Document, output};
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

fn soak_export_one(source_osu: &Path, idx: usize) -> Result<(), String> {
    let scratch = Scratch::new(&format!("export-soak-{idx}"));
    let root = scratch.path("mapset");
    fs::create_dir_all(&root).map_err(|e| format!("mkdir failed: {e}"))?;
    let source_path = root.join("Artist - Song (Mapper) [Hard].osu");
    let target_path = root.join("Artist - Song (Mapper) [Insane].osu");
    let backups = scratch.path("backups");

    let original_bytes = fs::read(source_osu).map_err(|e| format!("read failed: {e}"))?;
    fs::write(&source_path, &original_bytes).map_err(|e| format!("write source failed: {e}"))?;
    fs::write(&target_path, &original_bytes).map_err(|e| format!("write target failed: {e}"))?;

    let doc = match Document::read(&source_path) {
        Ok(d) => d,
        Err(e) => return Err(format!("Document::read rejected cleanly: {e}")),
    };

    let source = output::Source::from_document(&doc);

    // 1. Generate [Hitsounds] difficulty and verify source stays byte-identical.
    let gen_plan = match output::generate(&source, &|| false) {
        Ok(plan) => plan,
        Err(e) => return Err(format!("output::generate refused cleanly: {e}")),
    };
    let generated_path = gen_plan.files[0].path.clone();
    gen_plan
        .apply(&backups, &|| false)
        .map_err(|e| format!("generate apply failed: {e}"))?;
    if fs::read(&source_path).map_err(|e| format!("re-read source failed: {e}"))? != original_bytes {
        return Err("generate modified source .osu".into());
    }
    Document::read(&generated_path).map_err(|e| format!("reopen generated failed: {e}"))?;

    // 2. Copy to target difficulty and verify target reopens cleanly while source stays byte-identical.
    let copy_plan = match output::copy(&source, std::slice::from_ref(&target_path), &Default::default(), &|| false) {
        Ok(plan) => plan,
        Err(e) => return Err(format!("output::copy refused cleanly: {e}")),
    };
    copy_plan
        .apply(&backups, &|| false)
        .map_err(|e| format!("copy apply failed: {e}"))?;
    if fs::read(&source_path).map_err(|e| format!("re-read source after copy failed: {e}"))? != original_bytes {
        return Err("copy modified source .osu".into());
    }
    Document::read(&target_path).map_err(|e| format!("reopen copied target failed: {e}"))?;

    Ok(())
}

#[test]
fn export_sample_soak_20_maps() {
    let files = corpus_osu_files();
    if files.is_empty() {
        eprintln!("Skipping export_sample_soak_20_maps: corpus not present at {CORPUS_DIR}");
        return;
    }
    let step = (files.len() / 20).max(1);
    let mut tested = 0usize;
    let mut rejected = 0usize;
    let mut refused = 0usize;
    let mut failures = Vec::new();

    for (idx, path) in files.iter().step_by(step).take(20).enumerate() {
        tested += 1;
        match soak_export_one(path, idx) {
            Ok(()) => {}
            Err(msg) if msg.starts_with("Document::read rejected cleanly:") => rejected += 1,
            Err(msg) if msg.contains("refused cleanly:") => refused += 1,
            Err(msg) => failures.push(format!("{}: {msg}", path.display())),
        }
    }
    eprintln!(
        "export_sample_soak_20_maps: tested={tested}, passed={}, rejected={rejected}, refused={refused}, failed={}",
        tested - rejected - refused - failures.len(),
        failures.len()
    );
    assert!(
        failures.is_empty(),
        "Export sample soak failures:\n{}",
        failures.join("\n")
    );
}

#[test]
#[ignore]
fn export_full_soak_all_maps() {
    let files = corpus_osu_files();
    if files.is_empty() {
        eprintln!("Skipping export_full_soak_all_maps: corpus not present at {CORPUS_DIR}");
        return;
    }
    let mut tested = 0usize;
    let mut rejected = 0usize;
    let mut refused = 0usize;
    let mut failures = Vec::new();

    for (idx, path) in files.iter().enumerate() {
        tested += 1;
        match soak_export_one(path, idx) {
            Ok(()) => {}
            Err(msg) if msg.starts_with("Document::read rejected cleanly:") => rejected += 1,
            Err(msg) if msg.contains("refused cleanly:") => refused += 1,
            Err(msg) => failures.push(format!("{}: {msg}", path.display())),
        }
    }
    eprintln!(
        "export_full_soak_all_maps: tested={tested}, passed={}, rejected={rejected}, refused={refused}, failed={}",
        tested - rejected - refused - failures.len(),
        failures.len()
    );
    assert!(
        failures.is_empty(),
        "Export full soak failures ({}/{}):\n{}",
        failures.len(),
        tested,
        failures.iter().take(25).cloned().collect::<Vec<_>>().join("\n")
    );
}
