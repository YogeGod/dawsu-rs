//! The first M2 slice (Task 8, plan section 3): one copied difficulty through read, decompile,
//! cue planning, one edit, save and reopen.
//!
//! Set `DAWSU_KEEP_SLICE=1` to keep the saved copy in `target/slice-output/` for the manual lazer
//! oracle run (see the slice's result note).

use dawsu_foundation::safe_write::BackupPolicy;
use dawsu_foundation::{CustomIndex, Hitsound, SampleIdentity, Sound};
use dawsu_hitsounding::{compile, cue_list, decompile, Compiled, Decompiled, Hitsounds, MomentId, Place, Placement};
use dawsu_osu_format::{list_folder, LineEnding, OsuFile, SaveOutcome};
use dawsu_test_support::{save_pack_bytes, sha256_hex, workspace_root, Scratch};
use std::path::{Path, PathBuf};

const MAP: &str = "owner/catalyst-impetus-std.osu";
/// The edit target, fixed by its line: a circle with a whistle and no clap.
const TARGET: &str = "218,18,7386,1,2,2:0:0:0:";
const EDITED: &str = "218,18,7386,1,10,2:0:0:0:";

fn decompiled(file: &OsuFile, folder: &[String]) -> Hitsounds {
    match decompile(file, folder, &|| false, &mut |_, _| {}) {
        Decompiled::Done(h) => h,
        other => panic!("{other:?}"),
    }
}

/// Every file under `dir`, relative, sorted.
fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.push(p.strip_prefix(dir).unwrap().to_path_buf());
            }
        }
    }
    out.sort();
    out
}

/// The file's lines with their endings, as bytes.
fn lines_of(bytes: &[u8]) -> Vec<&[u8]> {
    bytes.split_inclusive(|b| *b == b'\n').collect()
}

/// Steps 1-5: copy, read, decompile, plan cues and add the clap. Returns the scratch folder,
/// the read file and the edited hitsounds.
fn prepare(name: &str) -> (Scratch, PathBuf, OsuFile, Hitsounds, MomentId) {
    // 1. Copy the save-pack file into a fresh scratch folder (AR-18).
    let scratch = Scratch::new(name);
    let path = scratch.path("catalyst-impetus-std.osu");
    std::fs::write(&path, save_pack_bytes(MAP)).unwrap();

    // 2. Read.
    let read = OsuFile::read(std::fs::read(&path).unwrap());
    assert!(read.warnings.is_empty());
    let file = read.file;
    assert!(!file.has_bom());
    assert!(file.lines().all(|l| l.ending() == LineEnding::CrLf), "the slice map is CRLF throughout");
    let target = file.hit_objects().into_iter().find(|o| file.line_text(o.line()).as_deref() == Some(TARGET)).expect("the target line");

    // 3. Decompile with the folder's file list.
    let folder = list_folder(scratch.dir()).unwrap();
    let mut h = decompiled(&file, &folder);
    assert_eq!(h.moments().len(), 1531 - 648 + h.moments().iter().filter(|m| matches!(m.place(), Place::SliderNode { .. })).count());

    // 4. Plan cues.
    let cues = cue_list(&h, &[], &[]);
    assert!(cues.windows(2).all(|w| w[0].time() <= w[1].time()));
    assert!(cues.iter().all(|c| (0.05..=1.0).contains(&c.gain())));

    // 5. Edit: the target circle's moment, after checking the preconditions.
    let at = h.moments().iter().position(|m| matches!(m.place(), Place::Circle(id) if id == object_id(&h, target.time()))).unwrap();
    let m = &h.moments()[at];
    assert!(m.volume() >= 5, "volume {}", m.volume());
    assert!(!m.hits().any(|l| matches!(l, SampleIdentity::Standard { sound: Sound::Hit(Hitsound::Clap), .. })), "no clap yet");
    let indexes: Vec<u32> = m.hits().filter_map(|l| match l {
        SampleIdentity::Standard { index, .. } => Some(index.get()),
        _ => None,
    }).collect();
    assert!(indexes.windows(2).all(|w| w[0] == w[1]), "a single custom index: {indexes:?}");
    let (set, index) = m.hits().find_map(|l| match l {
        SampleIdentity::Standard { set, sound: Sound::Hit(Hitsound::Whistle), index } => Some((*set, *index)),
        _ => None,
    }).expect("the whistle gives the addition set");
    h.add_hit(MomentId(at), SampleIdentity::Standard { set, sound: Sound::Hit(Hitsound::Clap), index: CustomIndex::new(index.get()) });
    (scratch, path, file, h, MomentId(at))
}

fn object_id(h: &Hitsounds, time: f64) -> dawsu_hitsounding::ObjectId {
    h.moments().iter().find_map(|m| match m.place() {
        Place::Circle(id) if m.time().ms() == time => Some(id),
        _ => None,
    }).unwrap()
}

#[test]
fn one_copied_difficulty_goes_through_read_decompile_cues_edit_save_and_reopen() {
    let pack_before = sha256_hex(&save_pack_bytes(MAP));
    let (scratch, path, file, h, edited) = prepare("slice");
    let before = decompiled(&file, &list_folder(scratch.dir()).unwrap());

    // 6. Save: compile with green-line placement, then the one write with a backup.
    let Compiled::Done { file: out, problems } = compile(&file, &h, Placement::GreenLines, &|| false, &mut |_, _| {}) else { panic!("cancelled") };
    assert!(problems.is_empty(), "{problems:?}");
    let backups = scratch.path("backups");
    assert_eq!(out.save_over(&path, &BackupPolicy::new(backups.clone(), 20)).unwrap(), SaveOutcome::Saved { warnings: vec![] });

    // 7. Reopen and decompile.
    let saved = std::fs::read(&path).unwrap();
    let reopened = OsuFile::read(saved.clone()).file;
    let after = decompiled(&reopened, &list_folder(scratch.dir()).unwrap());

    // The edited moment has the clap; every other moment decompiles as before.
    assert_eq!(after.moments().len(), before.moments().len());
    for (i, (b, a)) in before.moments().iter().zip(after.moments()).enumerate() {
        assert_eq!(a.time(), b.time());
        if i == edited.0 {
            assert_eq!(a.hits().collect::<Vec<_>>(), h.moment(edited).hits().collect::<Vec<_>>());
            assert!(a.hits().any(|l| matches!(l, SampleIdentity::Standard { sound: Sound::Hit(Hitsound::Clap), .. })));
            assert_eq!(a.volume(), b.volume());
        } else {
            assert_eq!(a.hits().collect::<Vec<_>>(), b.hits().collect::<Vec<_>>(), "moment {i}");
            assert_eq!(a.volume(), b.volume(), "moment {i}");
        }
    }
    assert_eq!(after.bodies().len(), before.bodies().len());

    // Exactly one line differs, the edited object's, and only in its hitsound field.
    let original = save_pack_bytes(MAP);
    let (old_lines, new_lines) = (lines_of(&original), lines_of(&saved));
    assert_eq!(old_lines.len(), new_lines.len());
    let differing: Vec<usize> = (0..old_lines.len()).filter(|&i| old_lines[i] != new_lines[i]).collect();
    assert_eq!(differing.len(), 1, "{differing:?}");
    let i = differing[0];
    assert_eq!(old_lines[i], format!("{TARGET}\r\n").as_bytes());
    assert_eq!(new_lines[i], format!("{EDITED}\r\n").as_bytes());
    let (old_fields, new_fields): (Vec<&str>, Vec<&str>) = (TARGET.split(',').collect(), EDITED.split(',').collect());
    let changed: Vec<usize> = (0..old_fields.len()).filter(|&f| old_fields[f] != new_fields[f]).collect();
    assert_eq!(changed, vec![4], "only the hitsound field");

    // One backup, equal to the original.
    let in_backups = files_under(&backups);
    assert_eq!(in_backups.len(), 1, "{in_backups:?}");
    assert_eq!(std::fs::read(backups.join(&in_backups[0])).unwrap(), original);

    // The save-pack file is unchanged, and no temporary file is left.
    assert_eq!(sha256_hex(&save_pack_bytes(MAP)), pack_before);
    let left = files_under(scratch.dir());
    assert_eq!(left.len(), 2, "the map and its backup only: {left:?}");
    assert!(left.iter().all(|p| !p.to_string_lossy().ends_with(".tmp")));

    if std::env::var_os("DAWSU_KEEP_SLICE").is_some() {
        let keep = workspace_root().join("target").join("slice-output");
        let _ = std::fs::remove_dir_all(&keep);
        std::fs::create_dir_all(&keep).unwrap();
        std::fs::write(keep.join("catalyst-impetus-std.osu"), &saved).unwrap();
        eprintln!("kept the saved copy in {}", keep.display());
    }
}

#[test]
fn a_change_on_disk_between_read_and_save_is_kept_and_reported() {
    let (scratch, path, file, h, _) = prepare("slice-changed-on-disk");
    let Compiled::Done { file: out, problems } = compile(&file, &h, Placement::GreenLines, &|| false, &mut |_, _| {}) else { panic!("cancelled") };
    assert!(problems.is_empty());
    let mut outside = save_pack_bytes(MAP);
    outside.extend_from_slice(b"// changed outside dawsu!\r\n");
    std::fs::write(&path, &outside).unwrap();
    let backups = scratch.path("backups");
    assert_eq!(out.save_over(&path, &BackupPolicy::new(backups.clone(), 20)).unwrap(), SaveOutcome::ChangedOnDisk);
    assert_eq!(std::fs::read(&path).unwrap(), outside, "the outside change stays");
    assert_eq!(files_under(scratch.dir()), vec![PathBuf::from("catalyst-impetus-std.osu")], "no backup and no temporary file");
}
