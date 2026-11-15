//! New hitsound diff and Generate hitsound diff (design doc "Generate and Copy hitsounds",
//! Responsibilities 1-5 and 11-12; the Generate and Copy plan, Tasks 2 and 4).
//!
//! A plan is a changed copy of the source difficulty: it starts from the source's bytes (ADR 0028
//! Decision 3) and keeps its line endings, BOM and format version. Because of that it must be
//! written only as a new file (`OsuFile::save_new`): written over the source's path, the one
//! write's changed-on-disk check would pass and the source would be replaced. Planning opens and
//! writes no file (invariant 1).

use dawsu_osu_format::OsuFile;

/// The outcome of New hitsound diff.
#[derive(Debug, Clone)]
pub enum NewDiff {
    /// The planned difficulty and the file name it is written under, in the source's folder.
    Planned { file: OsuFile, file_name: String },
    BlankName,
    /// Another difficulty in the folder has this Version (compared ignoring case).
    VersionTaken,
    /// A file with this name is in the folder (compared ignoring case, as Windows does).
    NameTaken,
}

/// Plans a new difficulty from `source`: every hit object removed, `Version:` set to `version`
/// (trimmed) and `BeatmapID:` to 0, every other line kept. `folder` lists the folder's `.osu`
/// files with their Versions, which the caller reads.
pub fn new_hitsound_diff(source: &OsuFile, source_file_name: &str, version: &str, folder: &[(String, Option<String>)]) -> NewDiff {
    let version = version.trim();
    if version.is_empty() {
        return NewDiff::BlankName;
    }
    let file_name = file_name_for(source, source_file_name, version);
    if folder.iter().any(|(name, _)| name.to_lowercase() == file_name.to_lowercase()) {
        return NewDiff::NameTaken;
    }
    if folder.iter().any(|(_, v)| v.as_deref().is_some_and(|v| v.to_lowercase() == version.to_lowercase())) {
        return NewDiff::VersionTaken;
    }
    NewDiff::Planned { file: empty_copy(source, version), file_name }
}

/// The source with no hit objects, the given Version and `BeatmapID:0`.
fn empty_copy(source: &OsuFile, version: &str) -> OsuFile {
    let mut file = source.clone();
    file.remove_all_hit_objects();
    // Neither can be refused: the text holds no line break, and a line that is not UTF-8 keeps
    // its bytes (it stays a difference the change summary shows).
    let _ = file.set_value("Metadata", "Version", version);
    let _ = file.set_value("Metadata", "BeatmapID", "0");
    file
}

/// `Artist - Title (Creator) [Version].osu`, or, without an artist or title, the source's file
/// name with its `[...]` part replaced (old app `HitsoundDiff.FileNameFor`).
pub fn file_name_for(source: &OsuFile, source_file_name: &str, version: &str) -> String {
    let meta = |key: &str| source.value("Metadata", key).unwrap_or_default();
    let (artist, title) = (meta("Artist"), meta("Title"));
    let name = if !artist.is_empty() && !title.is_empty() {
        format!("{artist} - {title} ({}) [{version}].osu", meta("Creator"))
    } else {
        match (source_file_name.rfind('['), source_file_name.ends_with("].osu")) {
            (Some(open), true) => format!("{}[{version}].osu", &source_file_name[..open]),
            _ => source_file_name.to_string(),
        }
    };
    // One fixed set on every system: the characters Windows does not allow in a file name, and
    // control characters (the old app used the running system's set, which differs on Linux).
    name.chars().map(|c| if c.is_control() || "<>:\"/\\|?*".contains(c) { '_' } else { c }).collect()
}
