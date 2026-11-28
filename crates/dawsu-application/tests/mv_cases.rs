//! Writes `before/` + `after/` mapset folders for the Mapset Verifier oracle
//! (`node tools/mv-oracle/run.mjs`). Each case copies an input set twice and runs one dawsu!
//! write path on the `after` copy. Inputs are never touched in place.
//!
//! Run: `cargo test -p dawsu-application --test mv_cases -- --ignored --nocapture`
//! Env: `MV_CASES_DIR` (default `<repo>/target/mv-oracle/cases`), `MV_SONGS_DIR`
//! (default `%LOCALAPPDATA%/osu!/Songs`, read-only), `MV_SONGS_LIMIT` (default 20).

use dawsu_application::{Document, output, save_document};
use dawsu_foundation::{CustomIndex, Hitsound, SampleIdentity, SampleSet, Sound};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Hits within this distance count as the same moment across difficulties (MV's own window).
const SAME_MOMENT_MS: f64 = 2.0;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

fn osu_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(dir)
        .map(|rd| rd.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    files.retain(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("osu")));
    files.sort();
    files
}

fn short_name(name: &str) -> String {
    name.chars().filter(|c| c.is_ascii_alphanumeric()).take(24).collect()
}

/// Every input set as its own folder: each fixture `.osu` alone, the demo set, and Songs copies.
fn inputs(staging: &Path) -> Vec<(String, PathBuf)> {
    let mut sets = Vec::new();
    let fixtures = repo().join("tests/fixtures");
    for sub in ["save-pack/owner", "save-pack/synthetic", "oracle-cases"] {
        for osu in osu_files(&fixtures.join(sub)) {
            let stem = osu.file_stem().unwrap().to_string_lossy().to_string();
            let dir = staging.join(format!("fx-{}", short_name(&stem)));
            fs::create_dir_all(&dir).unwrap();
            fs::copy(&osu, dir.join(osu.file_name().unwrap())).unwrap();
            sets.push((format!("fx-{}", short_name(&stem)), dir));
        }
    }
    let demo = staging.join("demo");
    let made = Command::new("node")
        .arg(repo().join("tools/make-demo-mapset.mjs"))
        .arg(&demo)
        .status()
        .is_ok_and(|s| s.success());
    if made {
        sets.push(("demo".into(), demo));
    } else {
        eprintln!("mv_cases: node tools/make-demo-mapset.mjs failed; demo set skipped");
    }
    let songs = std::env::var_os("MV_SONGS_DIR").map(PathBuf::from).or_else(|| {
        std::env::var_os("LOCALAPPDATA").map(|p| PathBuf::from(p).join("osu!").join("Songs"))
    });
    let limit: usize = std::env::var("MV_SONGS_LIMIT").ok().and_then(|v| v.parse().ok()).unwrap_or(20);
    if let Some(songs) = songs.filter(|p| p.is_dir()) {
        let mut dirs: Vec<PathBuf> = fs::read_dir(&songs).unwrap().flatten().map(|e| e.path()).collect();
        dirs.retain(|d| d.is_dir() && !osu_files(d).is_empty());
        dirs.sort();
        for (i, d) in dirs.into_iter().take(limit).enumerate() {
            let name = format!("song{i:02}-{}", short_name(&d.file_name().unwrap().to_string_lossy()));
            sets.push((name, d));
        }
    }
    sets
}

/// Saves every difficulty without edits.
fn op_save(after: &Path, backups: &Path) -> Result<(), String> {
    for osu in osu_files(after) {
        let doc = Document::read(&osu)?;
        save_document(&doc, backups.to_path_buf())?;
    }
    Ok(())
}

/// Adds the same clap at the first object of the first difficulty in every difficulty that has
/// an object there, at that moment's own volume and set, so the edit itself stays rankable.
fn op_edit(after: &Path, backups: &Path) -> Result<(), String> {
    let files = osu_files(after);
    let first = Document::read(files.first().ok_or("no .osu")?)?;
    let moment = first.composer.hitsounds().moments().first().ok_or("no hit moments")?;
    let t = moment.time().ms();
    let (set, index) = moment
        .hits()
        .find_map(|id| match id {
            SampleIdentity::Standard { set, index, .. } => Some((*set, *index)),
            _ => None,
        })
        .unwrap_or((SampleSet::Normal, CustomIndex::new(0)));
    let clap = SampleIdentity::Standard { set, sound: Sound::Hit(Hitsound::Clap), index };
    let mut edited = 0;
    for osu in &files {
        let mut doc = Document::read(osu)?;
        let Some(m) = doc
            .composer
            .hitsounds()
            .moments()
            .iter()
            .find(|m| (m.time().ms() - t).abs() <= SAME_MOMENT_MS)
        else {
            continue;
        };
        if m.has_hit(&clap) {
            continue;
        }
        let (time, volume) = (m.time().ms(), m.volume());
        doc.composer.add(time, clap.clone(), volume, SAME_MOMENT_MS);
        save_document(&doc, backups.to_path_buf())?;
        edited += 1;
    }
    if edited == 0 {
        return Err("every difficulty already has a clap there".into());
    }
    Ok(())
}

/// Decision D2: make a `[Hitsounds]` difficulty, copy it to every other difficulty, delete it.
fn op_hs_spread(after: &Path, backups: &Path) -> Result<(), String> {
    let files = osu_files(after);
    let source_doc = Document::read(files.first().ok_or("no .osu")?)?;
    let plan = output::generate(&output::Source::from_document(&source_doc), &|| false)?;
    let hs_path = plan.files.first().ok_or("generate planned no file")?.path.clone();
    plan.apply(backups, &|| false)?;
    let targets: Vec<PathBuf> = osu_files(after).into_iter().filter(|p| *p != hs_path).collect();
    if !targets.is_empty() {
        let hs_doc = Document::read(&hs_path)?;
        let copy = output::copy(&output::Source::from_document(&hs_doc), &targets, &Default::default(), &|| false)?;
        copy.apply(backups, &|| false)?;
    }
    fs::remove_file(&hs_path).map_err(|e| format!("delete [Hitsounds] diff: {e}"))?;
    Ok(())
}

type Op = fn(&Path, &Path) -> Result<(), String>;

#[test]
#[ignore = "writes MV oracle cases; run with --ignored, then node tools/mv-oracle/run.mjs"]
fn write_mv_oracle_cases() {
    let cases_dir = std::env::var_os("MV_CASES_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo().join("target/mv-oracle/cases"));
    let work = cases_dir.parent().unwrap().join("work");
    let _ = fs::remove_dir_all(&cases_dir);
    let _ = fs::remove_dir_all(&work);
    let staging = work.join("inputs");
    fs::create_dir_all(&staging).unwrap();

    let ops: [(&str, Op); 3] = [("save", op_save), ("edit", op_edit), ("hsspread", op_hs_spread)];
    let mut manifest = Vec::new();
    for (input, src) in inputs(&staging) {
        for (op_name, op) in ops {
            let name = format!("{op_name}-{input}");
            let case = cases_dir.join(&name);
            let after = case.join("after");
            if let Err(e) = copy_dir(&src, &case.join("before")).and_then(|()| copy_dir(&src, &after)) {
                manifest.push(format!("REFUSED {name}: copy failed: {e}"));
                let _ = fs::remove_dir_all(&case);
                continue;
            }
            // Backups live outside the mapset so they never show up as unused files.
            match op(&after, &work.join("backups").join(&name)) {
                Ok(()) => manifest.push(format!("WROTE {name}")),
                Err(e) => {
                    manifest.push(format!("REFUSED {name}: {}", e.lines().next().unwrap_or("")));
                    // A half-made case would compare equal and pass falsely, so drop it.
                    let _ = fs::remove_dir_all(&case);
                }
            }
        }
    }
    let wrote = manifest.iter().filter(|l| l.starts_with("WROTE")).count();
    fs::write(cases_dir.join("manifest.txt"), manifest.join("\n") + "\n").unwrap();
    eprintln!("{}", manifest.join("\n"));
    eprintln!("mv_cases: wrote {wrote} of {} cases into {}", manifest.len(), cases_dir.display());
    assert!(wrote > 0, "no MV oracle case was written");
}
