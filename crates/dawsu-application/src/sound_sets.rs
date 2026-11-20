//! Placing a sound set from a `.dawsuset` file (F056-F058, slice A of the sound-sets plan). The set
//! is copied into the hitsound library's "Sound sets" folder first, its samples are copied into the
//! mapset as one snapshot (all files or none), and its hits are placed at the snapped playhead as
//! one undo step, linked to the set.

use super::{Application, layers, safe_child};
use dawsu_audio::AudioClip;
use dawsu_editor::tools::ToolTarget;
use dawsu_editor_composer::{MOMENT_HITNORMAL, SetHitnormal};
use dawsu_foundation::{SampleIdentity, SongTime, digest::sha256_hex, safe_write};
use dawsu_hitsounding::sound_sets::{SetSource, SoundSet, placement_hits, plan_snapshot, read_dawsuset};
use dawsu_osu_format::list_folder;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The folder in the hitsound library that holds sound set files.
pub(crate) const LIBRARY_FOLDER: &str = "Sound sets";
const MAX_SET_BYTES: u64 = 96 * 1024 * 1024;
const MAX_RECORD_BYTES: u64 = 16 * 1024 * 1024;

/// Reads a `.dawsuset` file, checking its size before reading it.
fn read_set_file(path: &Path) -> Result<(Vec<u8>, SoundSet), String> {
    let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
    let size = std::fs::metadata(path).map_err(|_| format!("Could not read {name}. Check that another program is not using it."))?.len();
    if size > MAX_SET_BYTES {
        return Err(format!("{name} is over 96 MB."));
    }
    let bytes = std::fs::read(path).map_err(|_| format!("Could not read {name}. Check that another program is not using it."))?;
    let text = std::str::from_utf8(&bytes).map_err(|_| format!("{name} is not a sound set file."))?;
    let set = read_dawsuset(text).map_err(|e| format!("{name}: {e}"))?;
    Ok((bytes, set))
}

/// Where the picked set goes in the library, worked out without writing anything.
enum LibraryCopy {
    /// The set is already there, or was picked from there.
    There,
    /// A new file to create with these bytes.
    New(PathBuf, Vec<u8>),
}

impl LibraryCopy {
    /// Writes the new library file, if any, and returns it so a failed placement can take it back.
    fn write(&self) -> Result<Option<PathBuf>, String> {
        match self {
            LibraryCopy::There => Ok(None),
            LibraryCopy::New(target, bytes) => {
                let folder = target.parent().expect("a library file has a folder");
                std::fs::create_dir_all(folder).map_err(|e| format!("Creating the library's {LIBRARY_FOLDER} folder: {e}"))?;
                safe_write::create(target, bytes).map_err(|e| format!("Copying the set into the library: {e}"))?;
                index_written(target, bytes);
                Ok(Some(target.clone()))
            }
        }
    }
}

/// What the library list shows of a set file: its id, name and number of sounds.
#[derive(Clone, Debug, PartialEq)]
struct CachedSet {
    id: String,
    name: String,
    sounds: usize,
    /// The file's checksum, which names this version of the set.
    digest: String,
    /// The checksum of its sounds (`audio_checksum`): a new one needs new mapset files.
    audio: String,
}

/// A file's modified time and size: a cached read stays good while both are unchanged. A file
/// whose time can't be read is stamped without one and recorded as failed, so it isn't read
/// again on every list.
type Stamp = (Option<std::time::SystemTime>, u64);

/// What reading the library's set files found, by path (M10). Only sets that were read are kept;
/// a file that could not be read is noted with why and read again on the next list, since it may
/// only have been locked for a moment.
#[derive(Default)]
struct LibraryIndex {
    read: std::collections::HashMap<PathBuf, (Stamp, CachedSet)>,
    failed: std::collections::HashMap<PathBuf, (Stamp, String)>,
    /// Libraries whose files a background thread is reading now.
    reading: std::collections::HashSet<PathBuf>,
}
static LIBRARY_INDEX: std::sync::Mutex<Option<LibraryIndex>> = std::sync::Mutex::new(None);

fn with_index<T>(f: impl FnOnce(&mut LibraryIndex) -> T) -> T {
    let mut index = LIBRARY_INDEX.lock().unwrap_or_else(|e| e.into_inner());
    f(index.get_or_insert_with(Default::default))
}

fn stamp(metadata: &std::fs::Metadata) -> Option<Stamp> {
    stamp_of(metadata.modified(), metadata.len())
}

fn stamp_of(modified: std::io::Result<std::time::SystemTime>, len: u64) -> Option<Stamp> {
    Some((modified.ok(), len))
}

fn file_label(path: &Path) -> String {
    path.file_name().unwrap_or_default().to_string_lossy().into_owned()
}

/// Reads one library set file, or takes it from the index when its time and size are unchanged.
fn library_entry(path: &Path, metadata: &std::fs::Metadata) -> Result<CachedSet, String> {
    let stamp = stamp(metadata).ok_or_else(|| format!("Could not read {}. Check that another program is not using it.", file_label(path)))?;
    if let Some(entry) = with_index(|index| index.read.get(path).filter(|(s, _)| *s == stamp).map(|(_, e)| e.clone())) {
        return Ok(entry);
    }
    let result = if stamp.0.is_none() {
        Err(format!("Could not read {}. Check that another program is not using it.", file_label(path)))
    } else {
        read_set_file(path)
    }.map(|(bytes, set)| CachedSet { digest: sha256_hex(&bytes), audio: audio_checksum(&set), id: set.id, name: set.name, sounds: set.members.len() });
    with_index(|index| match &result {
        Ok(entry) => {
            index.read.insert(path.to_path_buf(), (stamp, entry.clone()));
            index.failed.remove(path);
        }
        Err(why) => {
            index.read.remove(path);
            index.failed.insert(path.to_path_buf(), (stamp, why.clone()));
        }
    });
    result
}

/// Puts a set file dawsu! just wrote into the index, so the next list or placement needn't wait
/// for it to be read.
fn index_written(path: &Path, bytes: &[u8]) {
    let (Ok(metadata), Ok(text)) = (std::fs::metadata(path), std::str::from_utf8(bytes)) else { return };
    let (Some(stamp), Ok(set)) = (stamp(&metadata), read_dawsuset(text)) else { return };
    if stamp.0.is_some() {
        let entry = CachedSet { digest: sha256_hex(bytes), audio: audio_checksum(&set), id: set.id, name: set.name, sounds: set.members.len() };
        with_index(|index| index.read.insert(path.to_path_buf(), (stamp, entry)));
    }
}

#[cfg(test)]
fn library_id(path: &Path, metadata: &std::fs::Metadata) -> Option<String> {
    library_entry(path, metadata).ok().map(|entry| entry.id)
}

/// The `.dawsuset` files in the library's "Sound sets" folder (none when it doesn't exist yet).
/// The index forgets the files of that folder that are gone.
fn set_files(library: &Path) -> Vec<(PathBuf, std::fs::Metadata)> {
    let folder = library.join(LIBRARY_FOLDER);
    let files: Vec<_> = std::fs::read_dir(&folder).map(|entries| entries.flatten().filter_map(|entry| {
        let path = entry.path();
        let metadata = entry.metadata().ok()?;
        (metadata.is_file() && path.extension().is_some_and(|e| e.eq_ignore_ascii_case("dawsuset"))).then_some((path, metadata))
    }).collect()).unwrap_or_default();
    let listed: std::collections::HashSet<&PathBuf> = files.iter().map(|(p, _)| p).collect();
    with_index(|index| {
        let gone = |path: &PathBuf| path.parent() == Some(folder.as_path()) && !listed.contains(path);
        index.read.retain(|path, _| !gone(path));
        index.failed.retain(|path, _| !gone(path));
    });
    files
}

/// Every set file in the library, read now on this thread: the sets, and the names of files that
/// could not be read. Saving and placing use it.
fn library_sets(library: &Path) -> (Vec<(PathBuf, CachedSet)>, Vec<String>) {
    let mut sets = Vec::new();
    let mut unreadable = Vec::new();
    for (path, metadata) in set_files(library) {
        match library_entry(&path, &metadata) {
            Ok(entry) => sets.push((path, entry)),
            Err(_) => unreadable.push(file_label(&path)),
        }
    }
    (sets, unreadable)
}

/// The library list without reading any set file on this thread: sets already read come from the
/// index, and files that are new, changed or failed last time are read on a background thread.
/// `reading` is true while a file has not been tried at its current time and size.
fn library_listing(library: &Path) -> (Vec<(PathBuf, CachedSet)>, Vec<String>, bool) {
    let files = set_files(library);
    let mut sets = Vec::new();
    let mut unreadable = Vec::new();
    let mut to_read = Vec::new();
    let mut reading = false;
    with_index(|index| {
        for (path, metadata) in &files {
            let stamp = stamp(metadata);
            match index.read.get(path) {
                Some((s, entry)) if Some(*s) == stamp => sets.push((path.clone(), entry.clone())),
                _ => {
                    match index.failed.get(path) {
                        Some((s, _)) if Some(*s) == stamp => unreadable.push(file_label(path)),
                        _ => reading = true,
                    }
                    to_read.push(path.clone());
                }
            }
        }
        if !to_read.is_empty() && index.reading.insert(library.to_path_buf()) {
            let library = library.to_path_buf();
            std::thread::spawn(move || {
                for path in to_read {
                    if let Ok(metadata) = std::fs::metadata(&path) {
                        let _ = library_entry(&path, &metadata);
                    }
                }
                with_index(|index| index.reading.remove(&library));
            });
        }
    });
    (sets, unreadable, reading)
}

/// A set in the library list.
#[derive(Clone, Debug, PartialEq)]
pub struct LibrarySet {
    pub id: String,
    pub name: String,
    pub sounds: usize,
    /// The open difficulty has a placement of this set.
    pub in_map: bool,
    /// More than one library file holds this id, so it can't be placed (M10).
    pub duplicate: bool,
    /// The open difficulty has a placement of this set but no library file holds it: listed by
    /// its id, and it can't be placed.
    pub missing: bool,
}

/// The library list: this map's sets first, then the rest, each by name, and the files that could
/// not be read.
#[derive(Clone, Debug, PartialEq)]
pub struct SoundSetLibrary {
    /// False when no hitsound library folder is chosen.
    pub configured: bool,
    pub sets: Vec<LibrarySet>,
    /// File names of set files that could not be read.
    pub unreadable: Vec<String>,
    /// Some files are still being read, so sets may be missing.
    pub reading: bool,
}

/// Why saving a set was refused. `NameTaken` asks for another name.
#[derive(Clone, Debug, PartialEq)]
pub enum SaveSetError {
    NameTaken(String),
    Refused(String),
}

/// The old app's library file name (`MainWindow.SoundSets.cs` 144-147): the set's name with the
/// characters no Windows file name can hold as `_` (`SafeFile.Name`, on every platform), at most
/// 80 characters, then `-<id>`. The id keeps names apart and away from Windows' reserved names.
fn set_file_name(name: &str, id: &str) -> String {
    let label: String = name.chars().map(|c| if c.is_control() || r#"<>:"/\|?*"#.contains(c) { '_' } else { c }).take(80).collect();
    format!("{label}-{id}.dawsuset")
}

/// A new GUID-shaped id (version 4 form). dawsu-application has no GUID crate and adding `uuid`
/// would be a new dependency, so the id is a SHA-256 of the time, a counter, a hash from std's
/// randomly seeded `RandomState` and `seed`.
fn new_id(seed: &str) -> String {
    use std::hash::{BuildHasher, Hasher};
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let count = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let random = std::collections::hash_map::RandomState::new().build_hasher().finish();
    let time = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |t| t.as_nanos());
    let hex: Vec<char> = sha256_hex(format!("{seed}|{time}|{count}|{random}").as_bytes()).chars().collect();
    let variant = ['8', '9', 'a', 'b'][hex[16].to_digit(16).unwrap_or(0) as usize % 4];
    let part = |range: std::ops::Range<usize>| hex[range].iter().collect::<String>();
    format!("{}-{}-4{}-{variant}{}-{}", part(0..8), part(8..12), part(13..16), part(17..20), part(20..32))
}

/// Where the picked set file goes in the library's "Sound sets" folder. A library file with the
/// same set id and the same bytes is reused; one with the same id and other bytes is never
/// overwritten, so placing is refused. A name already taken gets " (2)" and so on.
fn library_target(library: &Path, source: &Path, bytes: &[u8], set: &SoundSet) -> Result<LibraryCopy, String> {
    let folder = library.join(LIBRARY_FOLDER);
    if let Ok(canonical) = folder.canonicalize() {
        if source.canonicalize().is_ok_and(|s| s.parent() == Some(canonical.as_path())) {
            return Ok(LibraryCopy::There);
        }
        for entry in std::fs::read_dir(&canonical).map_err(|e| e.to_string())?.flatten() {
            let path = entry.path();
            let Ok(metadata) = entry.metadata() else { continue };
            if !path.extension().is_some_and(|e| e.eq_ignore_ascii_case("dawsuset")) || !metadata.is_file() {
                continue;
            }
            let Ok(entry_set) = library_entry(&path, &metadata) else { continue };
            if entry_set.id == set.id {
                return if entry_set.digest == sha256_hex(bytes) {
                    Ok(LibraryCopy::There)
                } else {
                    Err(format!("The library already has another version of \"{}\" ({}). Place it from there, or remove one of them.", set.name, entry.file_name().to_string_lossy()))
                };
            }
        }
    }
    let stem = source.file_stem().unwrap_or_default().to_string_lossy().into_owned();
    (1..1000)
        .map(|n| folder.join(if n == 1 { format!("{stem}.dawsuset") } else { format!("{stem} ({n}).dawsuset") }))
        .find(|target| !target.exists())
        .map(|target| LibraryCopy::New(target, bytes.to_vec()))
        .ok_or_else(|| "No free file name in the library's Sound sets folder.".into())
}

/// The snapshots made in one mapset, so placing a set again reuses its files.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct SnapshotRecord {
    entries: Vec<SnapshotEntry>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct SnapshotEntry {
    set_id: String,
    /// The checksum of the set's sounds (ids, roles and audio), which names this version.
    audio: String,
    files: Vec<SnapshotFile>,
    /// False while the files are being written: a crash leaves the entry pending, never reused.
    done: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct SnapshotFile {
    member_id: String,
    name: String,
    sha256: String,
}

fn audio_checksum(set: &SoundSet) -> String {
    let mut bytes = Vec::new();
    for m in &set.members {
        for part in [m.id.as_bytes(), m.slot.as_bytes(), &(m.audio.len() as u64).to_le_bytes(), &m.audio] {
            bytes.extend_from_slice(part);
        }
    }
    sha256_hex(&bytes)
}

struct RecordFile {
    path: PathBuf,
    read: Option<Vec<u8>>,
    record: SnapshotRecord,
}

impl RecordFile {
    fn open(data_root: &Path, mapset: &Path) -> Result<Self, String> {
        let key = sha256_hex(mapset.to_string_lossy().as_bytes());
        let path = data_root.join("sound-set-snapshots").join(format!("{key}.json"));
        let read = match std::fs::metadata(&path) {
            Ok(meta) if meta.len() > MAX_RECORD_BYTES => return Err("The sound set snapshot record is too large.".into()),
            Ok(_) => Some(std::fs::read(&path).map_err(|e| format!("Reading the sound set snapshot record: {e}"))?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.to_string()),
        };
        let record = match &read {
            Some(bytes) => serde_json::from_slice(bytes).map_err(|e| format!("Reading the sound set snapshot record: {e}"))?,
            None => SnapshotRecord::default(),
        };
        Ok(Self { path, read, record })
    }

    fn write(&mut self) -> Result<(), String> {
        let bytes = serde_json::to_vec_pretty(&self.record).map_err(|e| e.to_string())?;
        let folder = self.path.parent().expect("record has a folder");
        std::fs::create_dir_all(folder).map_err(|e| e.to_string())?;
        match &self.read {
            Some(read) => match safe_write::replace(&self.path, read, &bytes, &safe_write::BackupPolicy::new(folder.join("backups"), 5)).map_err(|e| e.to_string())? {
                safe_write::WriteOutcome::Saved { .. } => {}
                safe_write::WriteOutcome::ChangedOnDisk => return Err("The sound set snapshot record changed on disk. Try again.".into()),
            },
            None => safe_write::create(&self.path, &bytes).map_err(|e| e.to_string())?,
        }
        self.read = Some(bytes);
        Ok(())
    }

    /// The last finished snapshot of this version of the set, as the set on its mapset files.
    fn previous(&self, set: &SoundSet, audio: &str) -> Option<SoundSet> {
        let entry = self.record.entries.iter().rev().find(|e| e.done && e.set_id == set.id && e.audio == audio)?;
        let members = set.members.iter().map(|m| {
            let file = entry.files.iter().find(|f| f.member_id == m.id)?;
            let mut m = m.clone();
            m.lane = dawsu_hitsounding::sound_sets::SetLane { source: SetSource::File, name: file.name.clone(), missing: false, kept: Default::default() };
            Some(m)
        }).collect::<Option<Vec<_>>>()?;
        Some(SoundSet { members, ..set.clone() })
    }

    fn indexes(&self) -> impl Iterator<Item = u32> + '_ {
        self.record.entries.iter().flat_map(|e| e.files.iter()).filter_map(|f| match dawsu_hitsounding::rules::read_sample_file_name(&f.name) {
            Some(SampleIdentity::Standard { index, .. }) => Some(index.get()),
            _ => None,
        })
    }
}

/// Writes a snapshot's files, all or none: a pending record entry first, then each file as a new
/// file, then each decoded; on any failure the files this attempt created are deleted and the
/// entry dropped. Returns the written files with their decoded audio.
fn write_snapshot(root: &Path, record: &mut RecordFile, entry: SnapshotEntry, files: &[(String, Vec<u8>)]) -> Result<Vec<(String, Arc<AudioClip>)>, String> {
    let before = record.record.clone();
    record.record.entries.retain(|e| !(e.set_id == entry.set_id && e.audio == entry.audio));
    record.record.entries.push(entry);
    record.write()?;
    let mut created = Vec::new();
    let result = (|| {
        let mut clips = Vec::new();
        for (name, bytes) in files {
            let path = root.join(name);
            safe_write::create(&path, bytes).map_err(|e| format!("Could not add {name}: {e}"))?;
            created.push(path.clone());
            clips.push((name.clone(), dawsu_audio::decode(&path).map_err(|e| format!("{name}: {e}"))?));
        }
        Ok(clips)
    })();
    match result {
        Ok(clips) => {
            record.record.entries.last_mut().expect("the entry was pushed").done = true;
            if let Err(error) = record.write() {
                for path in &created {
                    let _ = std::fs::remove_file(path);
                }
                return Err(error);
            }
            Ok(clips)
        }
        Err(error) => {
            for path in &created {
                let _ = std::fs::remove_file(path);
            }
            record.record = before;
            let _ = record.write();
            Err(error)
        }
    }
}

impl Application {
    /// The custom indexes a new snapshot must not take: those any difficulty or storyboard of the
    /// mapset uses, the open document's lanes and every snapshot the record holds.
    fn used_indexes(&self, root: &Path, files: &[String], record: &RecordFile) -> Result<std::collections::BTreeSet<u32>, String> {
        let (mut used, _) = layers::reservations(root, files)?;
        let document = self.document.as_ref().expect("open document");
        let model = document.composer.hitsounds();
        used.extend(document.composer.lanes.iter().chain(model.lanes().iter().map(|l| &l.identity)).filter_map(|lane| match lane {
            SampleIdentity::Standard { index, .. } if index.get() > 1 => Some(index.get()),
            _ => None,
        }));
        used.extend(record.indexes());
        Ok(used)
    }

    /// The one hitnormal on `moment` that no placement added, as its bank, file extension and
    /// bytes, for D6b. None when the moment is new or holds no such hitnormal, or more than one
    /// (that moment saves as a mix anyway). A sound whose file can't be read is skipped with a
    /// note for the status line: the moment then saves as a mix.
    fn moment_hitnormal(&self, root: &Path, moment: Option<dawsu_hitsounding::MomentId>) -> (Option<(String, String, Vec<u8>)>, Option<String>) {
        let Some(moment) = moment else { return (None, None) };
        let hits = self.document.as_ref().expect("open document").composer.hitsounds();
        let placed = hits.placed_lanes(moment);
        let normals: Vec<_> = hits.moment(moment).hits().filter(|lane| !placed.contains(*lane) && matches!(lane, SampleIdentity::Standard { sound: dawsu_foundation::Sound::Hit(dawsu_foundation::Hitsound::Normal), .. })).collect();
        let [lane] = normals.as_slice() else { return (None, None) };
        let SampleIdentity::Standard { set, .. } = lane else { return (None, None) };
        let Some(model) = hits.lane(lane) else { return (None, None) };
        let defaults = &self.preferences.default_samples;
        let defaults_list = list_folder(defaults).unwrap_or_default();
        let skipped = || Some(format!("The {} under it wasn't found, so that moment saves as a mix.", crate::lane_name(lane)));
        let Ok(path) = super::sample_path(model, root, defaults, &defaults_list) else { return (None, skipped()) };
        let Some(extension) = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase)
            .filter(|e| matches!(e.as_str(), "wav" | "ogg" | "mp3")) else { return (None, skipped()) };
        let Ok(bytes) = std::fs::read(&path) else { return (None, skipped()) };
        let bank = match set {
            dawsu_foundation::SampleSet::Normal => "normal",
            dawsu_foundation::SampleSet::Soft => "soft",
            dawsu_foundation::SampleSet::Drum => "drum",
        };
        (Some((bank.to_string(), extension, bytes)), None)
    }

    /// Edit > Place sound set: copies the picked `.dawsuset` into the library, copies its samples
    /// into the mapset (or reuses an earlier snapshot of the same version), and places it at the
    /// snapped playhead as one undo step, linked to the set. Every check runs before the first
    /// write, so a refusal leaves no file behind.
    pub fn place_sound_set_file(&mut self, map: &Path, file: &Path) -> Result<(), String> {
        if self.busy || self.is_stroke_open() {
            return Err("Finish the current operation before placing a sound set.".into());
        }
        let document = self.document.as_ref().ok_or("Open a difficulty first.")?;
        if document.path != map {
            return Err("The difficulty changed while choosing a sound set.".into());
        }
        let library = self.preferences.hitsound_library.clone().ok_or("Choose a hitsound library folder first: sound sets are kept there.")?;
        let (bytes, set) = read_set_file(file)?;
        // M10: a set whose id more than one library file holds can't be placed, picked from the
        // list or browsed to.
        let (listed, _, reading) = library_listing(&library);
        if reading {
            return Err(READING_TRY_AGAIN.into());
        }
        let copies = listed.iter().filter(|(_, listed)| listed.id == set.id).count();
        if copies > 1 {
            return Err(duplicate_refusal(copies, &set.name));
        }
        let at = self.position_ms();
        let composer = &self.document.as_ref().expect("open document").composer;
        let time = composer.snap(SongTime::from_ms(at)).ms();
        if composer.is_set_placed_at(time, &set.id, 5.0) {
            self.status = format!("\"{}\" is already placed here.", set.name);
            return Ok(());
        }
        // Written once, just before the first mapset write; taken back if that write fails.
        let mut library_copy = Some(library_target(&library, file, &bytes, &set)?);
        let mut library_created = None;
        let set_digest = sha256_hex(&bytes);
        let root = document.path.parent().ok_or("Missing mapset folder")?.to_path_buf();
        let mut record = RecordFile::open(&self.data_root, &root)?;
        let audio = audio_checksum(&set);
        let mut previous = record.previous(&set, &audio);
        // D6b: a set with only additions takes the moment's own hitnormal along to its index.
        let set_has_hitnormal = set.members.iter().any(|m| m.slot.ends_with("-hitnormal"));
        let (own_hitnormal, note) = if set_has_hitnormal {
            (None, None)
        } else {
            self.moment_hitnormal(&root, composer.moment_at(time, 5.0))
        };
        // A name another program takes between planning and writing is planned again once.
        let mut attempt = 0;
        let (snapshot, clips, extra, hits, hitnormal) = loop {
            attempt += 1;
            let files = list_folder(&root).map_err(|e| e.to_string())?;
            let used = self.used_indexes(&root, &files, &record)?;
            let existing = |name: &str| safe_child(&root, name).ok().and_then(|p| std::fs::read(p).ok());
            let snapshot = plan_snapshot(&set, &files, &used, previous.as_ref(), &existing).map_err(|e| format!("{}: {e}", set.name))?;
            let mut writes = snapshot.files.clone();
            let mut extra = None;
            if let Some((bank, extension, bytes)) = &own_hitnormal {
                let index = match dawsu_hitsounding::rules::read_sample_file_name(&snapshot.set.members[0].lane.name) {
                    Some(SampleIdentity::Standard { index, .. }) => index.get(),
                    _ => return Err("The set's files have no sample index.".into()),
                };
                let name = format!("{bank}-hitnormal{index}.{extension}");
                match existing(&name) {
                    // Reused by its checksum.
                    Some(on_disk) if sha256_hex(&on_disk) == sha256_hex(bytes) => {}
                    // A different file holds the name: never overwritten, the whole set is copied
                    // again at a new index.
                    Some(_) if snapshot.files.is_empty() && previous.is_some() => {
                        previous = None;
                        continue;
                    }
                    _ => writes.push((name.clone(), bytes.clone())),
                }
                extra = Some(name);
            }
            // The hits and the hitnormal rule are worked out before anything is written.
            let hits = placement_hits(&snapshot.set, 100).into_iter().zip(&snapshot.set.members).map(|((key, volume), m)| {
                let lane = crate::project::lane_identity(&key).ok_or_else(|| format!("{} is not a sample file name", m.lane.name))?;
                Ok((m.id.clone(), lane, volume))
            }).collect::<Result<Vec<_>, String>>()?;
            let hitnormal = match (&extra, set_has_hitnormal) {
                (_, true) => SetHitnormal::ReplaceWithSet,
                (Some(name), false) => SetHitnormal::Reindex(dawsu_hitsounding::rules::read_sample_file_name(name).ok_or("The copied hitnormal has no sample name.")?),
                (None, false) => SetHitnormal::Keep,
            };
            if writes.is_empty() {
                if let Some(copy) = library_copy.take() {
                    copy.write()?;
                }
                break (snapshot, Vec::new(), extra, hits, hitnormal);
            }
            let entry = SnapshotEntry {
                set_id: set.id.clone(),
                audio: audio.clone(),
                files: snapshot.set.members.iter().map(|m| SnapshotFile { member_id: m.id.clone(), name: m.lane.name.clone(), sha256: sha256_hex(&m.audio) })
                    .chain(extra.iter().map(|name| SnapshotFile { member_id: "moment-hitnormal".into(), name: name.clone(), sha256: sha256_hex(&own_hitnormal.as_ref().expect("an extra file has bytes").2) }))
                    .collect(),
                done: false,
            };
            let clash = writes.iter().any(|(name, _)| root.join(name).exists());
            if clash && attempt == 1 {
                continue;
            }
            if let Some(copy) = library_copy.take() {
                library_created = copy.write()?;
            }
            match write_snapshot(&root, &mut record, entry, &writes) {
                Ok(clips) => break (snapshot, clips, extra, hits, hitnormal),
                Err(_) if attempt == 1 && writes.iter().any(|(name, _)| root.join(name).exists()) => continue,
                Err(error) => {
                    if let Some(path) = &library_created {
                        let _ = std::fs::remove_file(path);
                    }
                    return Err(error);
                }
            }
        };
        // Reused files may have no decoded audio yet.
        let written = clips.len();
        let mut additions = clips;
        for name in snapshot.set.members.iter().map(|m| &m.lane.name).chain(extra.as_ref()) {
            if !additions.iter().any(|(n, _)| n == name) && !self.samples.contains_key(&SampleIdentity::File(name.clone())) {
                let path = safe_child(&root, name)?;
                additions.push((name.clone(), dawsu_audio::decode(&path).map_err(|e| format!("{name}: {e}"))?));
            }
        }
        self.register_added_samples(additions);
        let composer = &mut self.document.as_mut().expect("open document").composer;
        composer.place_sound_set(time, &set.id, &set_digest, 100, &hits, hitnormal, 5.0)
            .ok_or("The set was placed here while its files were copied.")?;
        composer.audition_requests.extend(hits.iter().map(|(_, lane, _)| lane.clone()));
        let copied = if written == 0 { String::new() } else { format!(", {written} files copied") };
        self.status = format!("Placed \"{}\"{copied}.", set.name);
        if let Some(note) = note {
            self.status = format!("{} {note}", self.status);
        }
        self.sync_audio();
        Ok(())
    }

    /// Edit > Place sound set: the library's sets, this map's first, and the files that could not
    /// be read. Reads no set file on this thread (see `library_listing`). An error names a library
    /// folder that is gone; a library with no "Sound sets" folder yet is an empty list.
    pub fn sound_set_library(&self) -> Result<SoundSetLibrary, String> {
        let Some(library) = &self.preferences.hitsound_library else {
            return Ok(SoundSetLibrary { configured: false, sets: Vec::new(), unreadable: Vec::new(), reading: false });
        };
        if !library.is_dir() {
            return Err(format!("Hitsound library folder not found: {}", library.display()));
        }
        let used: std::collections::HashSet<&str> = self.document.as_ref().map(|d| {
            let hits = d.composer.hitsounds();
            hits.placements().iter().filter(|p| hits.is_placement_live(p)).map(|p| p.set_id.as_str()).collect()
        }).unwrap_or_default();
        let (found, mut unreadable, reading) = library_listing(library);
        let mut sets: Vec<LibrarySet> = Vec::new();
        for (_, entry) in found {
            if let Some(listed) = sets.iter_mut().find(|s| s.id == entry.id) {
                listed.duplicate = true;
                continue;
            }
            sets.push(LibrarySet { in_map: used.contains(entry.id.as_str()), duplicate: false, missing: false, id: entry.id, name: entry.name, sounds: entry.sounds });
        }
        // A set this map uses is called missing only when every library file was read.
        if !reading && unreadable.is_empty() {
            for id in used {
                if !sets.iter().any(|s| s.id == id) {
                    sets.push(LibrarySet { id: id.to_string(), name: String::new(), sounds: 0, in_map: true, duplicate: false, missing: true });
                }
            }
        }
        sets.sort_by(|a, b| b.in_map.cmp(&a.in_map).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())).then_with(|| a.id.cmp(&b.id)));
        unreadable.sort_by_key(|name| name.to_lowercase());
        Ok(SoundSetLibrary { configured: true, sets, unreadable, reading })
    }

    /// Places the library set with this id, as `place_sound_set_file` does. A set whose id more
    /// than one library file holds is refused (M10).
    pub fn place_library_sound_set(&mut self, map: &Path, id: &str) -> Result<(), String> {
        if self.busy || self.is_stroke_open() {
            return Err("Finish the current operation before placing a sound set.".into());
        }
        let library = self.preferences.hitsound_library.clone().ok_or("Choose a hitsound library folder first: sound sets are kept there.")?;
        let (listed, _, reading) = library_listing(&library);
        if reading {
            return Err(READING_TRY_AGAIN.into());
        }
        let found: Vec<_> = listed.into_iter().filter(|(_, set)| set.id == id).collect();
        match found.as_slice() {
            [] => Err("That sound set is no longer in the library.".into()),
            [(path, _)] => self.place_sound_set_file(map, path),
            [(_, set), ..] => Err(duplicate_refusal(found.len(), &set.name)),
        }
    }

    /// Edit > Save selection as sound set: a new `.dawsuset` in the library's "Sound sets" folder
    /// with each selected hit's sample audio, role and volume (the old app's capture rules). The
    /// hits stay ordinary hits, so no undo step is made. A sound set name the library already has
    /// asks for another; no file is ever overwritten.
    pub fn save_sound_set_from_selection(&mut self, map: &Path, name: &str) -> Result<(), SaveSetError> {
        let refused = |why: String| SaveSetError::Refused(why);
        if self.busy || self.is_stroke_open() {
            return Err(refused("Finish the current operation before saving a sound set.".into()));
        }
        let document = self.document.as_ref().ok_or_else(|| refused("Open a difficulty first.".into()))?;
        if document.path != map {
            return Err(refused("The difficulty changed while naming the sound set.".into()));
        }
        let library = self.preferences.hitsound_library.clone().ok_or_else(|| refused("Choose a hitsound library folder first: sound sets are kept there.".into()))?;
        if !library.is_dir() {
            return Err(refused(format!("Hitsound library folder not found: {}", library.display())));
        }
        let name = name.trim();
        if name.is_empty() {
            return Err(refused("Name the sound set.".into()));
        }
        if name.encode_utf16().count() > 128 {
            return Err(refused("A sound set name can have at most 128 characters.".into()));
        }
        let hits = document.composer.hitsounds();
        let mut keys: Vec<_> = document.composer.selected.items().iter().collect();
        if keys.is_empty() {
            return Err(refused("Select the sounds to save as a sound set.".into()));
        }
        keys.sort_by(|a, b| (a.moment, &a.lane).cmp(&(b.moment, &b.lane)));
        let times: std::collections::BTreeSet<u64> = keys.iter().map(|k| hits.moment(dawsu_hitsounding::MomentId(k.moment)).time().ms().to_bits()).collect();
        if times.len() != 1 {
            return Err(refused("Select sounds at one time to save a sound set.".into()));
        }
        // Two objects at one time can hold the same sound: it goes in the set once. A silent hit
        // (volume 0) is left out: a set member at 0 would be a silent sound.
        let mut lanes: Vec<(&SampleIdentity, i32)> = Vec::new();
        let mut silent = 0;
        for key in keys {
            let volume = hits.moment(dawsu_hitsounding::MomentId(key.moment)).hit_volume(&key.lane);
            if volume == 0 {
                silent += 1;
                continue;
            }
            match lanes.iter().find(|(lane, _)| *lane == &key.lane) {
                Some((_, v)) if *v == volume => {}
                Some(_) => return Err(refused(format!("{} is selected twice at this time.", crate::lane_name(&key.lane)))),
                None => lanes.push((&key.lane, volume)),
            }
        }
        if lanes.is_empty() {
            return Err(refused("Every selected sound is silent. Select sounds you can hear.".into()));
        }
        if lanes.len() > 128 {
            return Err(refused("A sound set holds at most 128 sounds.".into()));
        }
        let lower = name.to_lowercase();
        if library_sets(&library).0.iter().any(|(_, s)| s.name.trim().to_lowercase() == lower) {
            return Err(SaveSetError::NameTaken(format!("The library already has a sound set named \"{name}\". Choose another name.")));
        }
        let root = document.path.parent().ok_or_else(|| refused("The difficulty's folder was not found.".into()))?;
        let defaults = &self.preferences.default_samples;
        let defaults_list = list_folder(defaults).unwrap_or_default();
        let mut captured = Vec::new();
        let mut total = 0u64;
        for (identity, volume) in lanes {
            let lane_name = crate::lane_name(identity);
            let unusable = |why: &str| refused(format!("{lane_name} {why}, so it can't go in a sound set."));
            let unreadable = || refused(format!("Could not read {lane_name}. Check that another program is not using it."));
            let model = hits.lane(identity).ok_or_else(|| unusable("has no lane"))?;
            // `sample_path` decides what plays: the folder's file, or the skin's sound.
            let path = super::sample_path(model, root, defaults, &defaults_list).map_err(|_| unusable("wasn't found"))?;
            let (source, file) = match &model.source {
                dawsu_hitsounding::LaneSource::File(file) => (SetSource::File, file.clone()),
                dawsu_hitsounding::LaneSource::Skin => (SetSource::Skin, crate::lane_name(&model.plays_as())),
            };
            if source == SetSource::Skin && !path.extension().is_some_and(|e| e.eq_ignore_ascii_case("wav")) {
                return Err(unusable("is not a .wav file in the default samples"));
            }
            let size = std::fs::metadata(&path).map_err(|_| unreadable())?.len();
            if size > 32 * 1024 * 1024 {
                return Err(unusable("is over 32 MB"));
            }
            total += size;
            if total > 64 * 1024 * 1024 {
                return Err(refused("The selected sounds are over 64 MB together, so they can't be one sound set.".into()));
            }
            let audio = std::fs::read(&path).map_err(|_| unreadable())?;
            let lane = dawsu_hitsounding::sound_sets::SetLane { source, name: file, missing: false, kept: Default::default() };
            captured.push(dawsu_hitsounding::sound_sets::CapturedHit {
                // The old app's key of the captured lane (`ProjectStore.KeyOf`).
                original_key: lane.key(),
                lane,
                volume,
                audio,
            });
        }
        let set = dawsu_hitsounding::sound_sets::capture(&mut || new_id(name), name, captured).map_err(|e| refused(format!("{name}: {e}")))?;
        let text = dawsu_hitsounding::sound_sets::write_dawsuset(&set).map_err(|e| refused(format!("{name}: {e}")))?;
        let folder = library.join(LIBRARY_FOLDER);
        let file_name = set_file_name(name, &set.id);
        std::fs::create_dir_all(&folder).map_err(|_| refused(format!("Could not create the library's {LIBRARY_FOLDER} folder. Check that the library folder can be written to.")))?;
        if let Err(error) = safe_write::create(&folder.join(&file_name), text.as_bytes()) {
            return Err(refused(save_failure(name, &error)));
        }
        index_written(&folder.join(&file_name), text.as_bytes());
        self.status = format!("Saved sound set \"{name}\".");
        if silent > 0 {
            self.status += &format!(" Left out {silent} silent {}.", if silent == 1 { "sound" } else { "sounds" });
        }
        Ok(())
    }
}

/// The message for a failed library write. Only a hard link the drive can't make (FAT and exFAT:
/// Windows' ERROR_INVALID_FUNCTION, raw OS error 1, or `Unsupported`) asks for another drive;
/// any other failure names the set.
fn save_failure(name: &str, error: &dawsu_foundation::Error) -> String {
    let dawsu_foundation::Error::Io { step, source, .. } = error;
    let unsupported = source.kind() == std::io::ErrorKind::Unsupported || (cfg!(windows) && source.raw_os_error() == Some(1));
    if *step == "creating the file" && unsupported {
        "This drive can't hold the library. Choose a library on an NTFS drive.".into()
    } else {
        format!("Could not save {name} to the library. Check that another program is not using it.")
    }
}

/// The refusal for a set id that more than one library file holds (M10).
fn duplicate_refusal(copies: usize, name: &str) -> String {
    if copies == 2 {
        format!("Two library files contain \"{name}\". Remove one to place it.")
    } else {
        format!("{copies} library files contain \"{name}\". Remove all but one to place it.")
    }
}

// ---------------------------------------------------------------- linked placements (step 3)

/// Where a placed set stands in the library index, for its placements.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum InLibrary {
    /// One library file holds the set: its path, name, version (the file's checksum) and the
    /// checksum of its sounds.
    Found { path: PathBuf, name: String, digest: String, audio: String },
    /// Every library file was read and none holds it: its placements are frozen.
    Missing,
    /// The set may still be there: no library is chosen, a file could not be read or is being
    /// read, or two files hold the id. Frozen too, with why.
    Unsure(String),
}

const READING: &str = "Still reading the sound set library.";
const READING_TRY_AGAIN: &str = "Still reading the sound set library. Try again.";

/// An open-time question: placements that use an older version of a library set (M9).
#[derive(Clone, Debug, PartialEq)]
pub struct SetUpdatePrompt {
    pub set_id: String,
    pub name: String,
    pub digest: String,
    pub placements: usize,
    /// Sounds edited by hand that stay on their old files, because Update copies the set to
    /// new files.
    pub edited_keep_files: usize,
}

/// What the Sound set placement dialog shows of one placement.
#[derive(Clone, Debug, PartialEq)]
pub struct PlacementView {
    pub id: u64,
    pub set_id: String,
    /// The set's name; None when the library index doesn't hold it.
    pub name: Option<String>,
    pub edited: usize,
    pub sounds: usize,
    pub gain: i32,
    /// Why the set's volume can't be set, if it can't.
    pub frozen: Option<String>,
    /// The library has a newer version than this placement plays.
    pub newer: bool,
}

/// The sets with these ids in the library index. Reads no set file on this thread: the index
/// reads new and changed files in the background (M10).
fn look_up(library: Option<&Path>, ids: &[String]) -> Vec<(String, InLibrary)> {
    let unsure = |why: String| ids.iter().map(|id| (id.clone(), InLibrary::Unsure(why.clone()))).collect();
    let Some(library) = library else { return unsure("Choose the hitsound library folder.".into()) };
    if !library.is_dir() {
        return unsure(format!("Hitsound library folder not found: {}", library.display()));
    }
    let (sets, unreadable, reading) = library_listing(library);
    ids.iter().map(|id| {
        let found: Vec<_> = sets.iter().filter(|(_, s)| &s.id == id).collect();
        let state = match found.as_slice() {
            // Another copy may still be being read (M10 duplicates).
            _ if reading => InLibrary::Unsure(READING.into()),
            [] if unreadable.is_empty() => InLibrary::Missing,
            [] => InLibrary::Unsure(format!("Could not read {}.", unreadable.join(", "))),
            [(path, set)] => InLibrary::Found { path: path.clone(), name: set.name.clone(), digest: set.digest.clone(), audio: set.audio.clone() },
            [(_, set), ..] => InLibrary::Unsure(duplicate_refusal(found.len(), &set.name)),
        };
        (id.clone(), state)
    }).collect()
}

/// The custom index a folder file's name holds, whatever its extension.
fn folder_index(name: &str) -> Option<u32> {
    let stem = name.rsplit_once('.').map_or(name, |(s, _)| s);
    match dawsu_hitsounding::rules::read_sample_file_name(&format!("{stem}.wav"))? {
        SampleIdentity::Standard { index, .. } => Some(index.get()),
        SampleIdentity::File(_) => None,
    }
}

fn is_hitnormal(lane: &SampleIdentity) -> bool {
    matches!(lane, SampleIdentity::Standard { sound: dawsu_foundation::Sound::Hit(dawsu_foundation::Hitsound::Normal), .. })
}

/// A moved moment hitnormal (D6b) to carry to a new index: its lane, bank, extension and bytes.
type MovedHitnormal = (SampleIdentity, String, String, Vec<u8>);

/// The hits one placement plays from `set` (on its mapset files) at `gain`, with its moved
/// hitnormal on `moved` lanes when the set has no hitnormal of its own.
fn placement_member_hits(set: &SoundSet, gain: i32, placement: &dawsu_hitsounding::sound_sets::SoundSetPlacement, moved: &std::collections::BTreeMap<SampleIdentity, SampleIdentity>) -> Result<Vec<(String, SampleIdentity, i32)>, String> {
    let mut hits = member_hits(set, gain)?;
    if !hits.iter().any(|(_, lane, _)| is_hitnormal(lane))
        && let Some(a) = placement.applied.iter().find(|a| a.member_id == MOMENT_HITNORMAL)
    {
        hits.push((MOMENT_HITNORMAL.to_string(), moved.get(&a.lane).cloned().unwrap_or_else(|| a.lane.clone()), a.volume));
    }
    Ok(hits)
}

impl Application {
    fn placed_set_ids(&self) -> Vec<String> {
        let Some(document) = self.document.as_ref() else { return Vec::new() };
        let hits = document.composer.hitsounds();
        let mut ids: Vec<String> = hits.placements().iter().filter(|p| hits.is_placement_live(p)).map(|p| p.set_id.clone()).collect();
        ids.sort();
        ids.dedup();
        ids
    }

    /// On open: one question per set whose library version is newer than its placements' and
    /// than the version the mapper kept them on (M9). Nothing is written. While the library
    /// index is still being read, the check waits for `poll`.
    pub(crate) fn ask_about_set_versions(&mut self) {
        self.sound_set_prompts.clear();
        self.sound_set_check_pending = false;
        let ids = self.placed_set_ids();
        if ids.is_empty() {
            return;
        }
        let found = look_up(self.preferences.hitsound_library.as_deref(), &ids);
        if found.iter().any(|(_, state)| matches!(state, InLibrary::Unsure(why) if why == READING)) {
            self.sound_set_check_pending = true;
            return;
        }
        let document = self.document.as_ref().expect("open document");
        let record = document.path.parent().and_then(|root| RecordFile::open(&self.data_root, root).ok());
        let hits = document.composer.hitsounds();
        let accepted = self.project.as_ref().map(|p| &p.accepted_versions);
        for (id, state) in found {
            let InLibrary::Found { name, digest, audio, .. } = state else { continue };
            if accepted.is_some_and(|a| a.get(&id) == Some(&digest)) {
                continue;
            }
            let placements: Vec<_> = hits.placements().iter()
                .filter(|p| p.set_id == id && hits.is_placement_live(p) && p.set_digest != digest && p.accepted_digest.as_deref() != Some(digest.as_str()))
                .collect();
            if placements.is_empty() {
                continue;
            }
            let new_files = !record.as_ref().is_some_and(|r| r.record.entries.iter().any(|e| e.done && e.set_id == id && e.audio == audio));
            let edited_keep_files = if new_files {
                placements.iter().map(|p| p.applied.iter().filter(|a| hits.is_member_edited(p, a)).count()).sum()
            } else {
                0
            };
            self.sound_set_prompts.push(SetUpdatePrompt { set_id: id, name, digest, placements: placements.len(), edited_keep_files });
        }
    }

    /// Runs the open-time check once the library index has read every set file.
    pub(crate) fn check_set_versions_when_read(&mut self) {
        if self.sound_set_check_pending && !self.busy && self.document.is_some() {
            self.ask_about_set_versions();
        }
    }

    /// The open-time questions still waiting for an answer.
    pub fn sound_set_prompts(&self) -> &[SetUpdatePrompt] {
        &self.sound_set_prompts
    }

    /// Which open the questions belong to: a question closed without an answer comes back only
    /// when this changes.
    pub fn sound_set_prompt_open(&self) -> u64 {
        self.opens
    }

    /// Answers the open-time question about `set_id`: Update plays the library's version on
    /// every placement as one undo step; Keep notes the version in the project, with no edit to
    /// the document, so only a newer one asks again, and writes no mapset file.
    pub fn answer_sound_set_prompt(&mut self, set_id: &str, update: bool) -> Result<(), String> {
        let Some(index) = self.sound_set_prompts.iter().position(|p| p.set_id == set_id) else { return Ok(()) };
        if self.busy || self.is_stroke_open() {
            return Err("Finish the current operation before updating a sound set.".into());
        }
        if update {
            self.update_sound_set(set_id)?;
            self.sound_set_prompts.retain(|p| p.set_id != set_id);
            return Ok(());
        }
        let prompt = self.sound_set_prompts.remove(index);
        let accept = |records: &mut Vec<PlacementRecord>| {
            for r in records.iter_mut().filter(|r| r.set_id == prompt.set_id) {
                r.accepted_digest = Some(prompt.digest.clone());
            }
        };
        if let (Some(mut settings), Some(_)) = (self.session_settings(None, None, None), self.project.as_ref()) {
            if let Some(links) = settings.sound_sets.as_mut() {
                for record in links.current.iter_mut().chain(links.pending.iter_mut()) {
                    accept(&mut record.placements);
                }
            }
            let project = self.project.as_mut().expect("a project");
            accept(&mut project.kept_links);
            project.accepted_versions.insert(prompt.set_id.clone(), prompt.digest.clone());
            project.save(settings).map_err(|e| format!("Saving session settings: {e}"))?;
        }
        self.status = format!("Kept this version of \"{}\".", prompt.name);
        Ok(())
    }

    /// The set as its one library file holds it now, with its version, or why its placements
    /// are frozen. Reads that file: only an edit that plays the set's sounds asks for it.
    fn library_version(&self, set_id: &str) -> Result<(SoundSet, String), String> {
        match look_up(self.preferences.hitsound_library.as_deref(), &[set_id.to_string()]).pop().map(|(_, s)| s) {
            Some(InLibrary::Found { path, .. }) => {
                let (bytes, set) = read_set_file(&path)?;
                Ok((set, sha256_hex(&bytes)))
            }
            Some(InLibrary::Unsure(why)) => Err(why),
            _ => Err("Not in library".into()),
        }
    }

    /// The moved hitnormals (D6b) of every live placement of `set_id`, read from the mapset.
    fn moved_hitnormals(&self, set_id: &str) -> Result<Vec<MovedHitnormal>, String> {
        let document = self.document.as_ref().ok_or("Open a difficulty first.")?;
        let root = document.path.parent().ok_or("Missing mapset folder")?;
        let hits = document.composer.hitsounds();
        let mut moved: Vec<MovedHitnormal> = Vec::new();
        for p in hits.placements().iter().filter(|p| p.set_id == set_id && hits.is_placement_live(p)) {
            let Some(a) = p.applied.iter().find(|a| a.member_id == MOMENT_HITNORMAL) else { continue };
            if moved.iter().any(|m| m.0 == a.lane) {
                continue;
            }
            let (SampleIdentity::Standard { set, .. }, Some(dawsu_hitsounding::LaneSource::File(name))) = (&a.lane, hits.lane(&a.lane).map(|l| &l.source)) else { continue };
            let extension = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
            let bytes = std::fs::read(safe_child(root, name)?).map_err(|_| format!("Could not read {name}. Check that another program is not using it."))?;
            let bank = match set {
                dawsu_foundation::SampleSet::Normal => "normal",
                dawsu_foundation::SampleSet::Soft => "soft",
                dawsu_foundation::SampleSet::Drum => "drum",
            };
            moved.push((a.lane.clone(), bank.to_string(), extension, bytes));
        }
        Ok(moved)
    }

    /// The set on its files in this mapset, with each moved hitnormal's lane at the set's index:
    /// the last snapshot of this version when its files are unchanged, else a new snapshot at a
    /// new index with the moved hitnormals, all files or none (M6, M7), through the same
    /// name-clash check as placing. With `write` false, None when files would be written. Also
    /// returns whether files were written.
    #[allow(clippy::type_complexity)]
    fn snapshot_set(&mut self, set: &SoundSet, write: bool, moved: &[MovedHitnormal]) -> Result<Option<(SoundSet, std::collections::BTreeMap<SampleIdentity, SampleIdentity>, bool)>, String> {
        let root = self.document.as_ref().ok_or("Open a difficulty first.")?.path.parent().ok_or("Missing mapset folder")?.to_path_buf();
        let mut record = RecordFile::open(&self.data_root, &root)?;
        let audio = audio_checksum(set);
        let mut previous = record.previous(set, &audio);
        let mut attempt = 0;
        let (snapshot, lanes, extra, clips) = loop {
            attempt += 1;
            let files = list_folder(&root).map_err(|e| e.to_string())?;
            let used = self.used_indexes(&root, &files, &record)?;
            let existing = |name: &str| safe_child(&root, name).ok().and_then(|p| std::fs::read(p).ok());
            let snapshot = plan_snapshot(set, &files, &used, previous.as_ref(), &existing).map_err(|e| format!("{}: {e}", set.name))?;
            // The hits are worked out before anything is written.
            member_hits(&snapshot.set, 100)?;
            let index = match snapshot.set.members.first().and_then(|m| dawsu_hitsounding::rules::read_sample_file_name(&m.lane.name)) {
                Some(SampleIdentity::Standard { index, .. }) => index.get(),
                _ => return Err("The set's files have no sample index.".into()),
            };
            let mut writes = snapshot.files.clone();
            let mut lanes = std::collections::BTreeMap::new();
            let mut extra: Vec<(String, &Vec<u8>)> = Vec::new();
            let mut clash = false;
            // Indexes a moved hitnormal that can't share the set's file name may take.
            let mut occupied = used.clone();
            occupied.extend(files.iter().filter_map(|f| folder_index(f)));
            occupied.extend(snapshot.set.members.iter().filter_map(|m| folder_index(&m.lane.name)));
            for (lane, bank, extension, bytes) in moved {
                let mut at = index;
                let name = loop {
                    let name = format!("{bank}-hitnormal{at}.{extension}");
                    match (extra.iter().find(|(n, _)| *n == name), existing(&name)) {
                        // Another moved hitnormal with the same sound: one file.
                        (Some((_, planned)), _) if *planned == bytes => break name,
                        // Already in the mapset with this sound.
                        (None, Some(on_disk)) if sha256_hex(&on_disk) == sha256_hex(bytes) => break name,
                        // Never overwritten: a reused snapshot is copied again whole at a new index.
                        (None, Some(_)) if at == index && previous.is_some() => {
                            clash = true;
                            break name;
                        }
                        (None, None) => {
                            writes.push((name.clone(), bytes.clone()));
                            break name;
                        }
                        // Taken by another sound: this hitnormal gets its own index.
                        _ => {
                            at = (2u32..).find(|i| !occupied.contains(i)).expect("an index is free");
                            occupied.insert(at);
                        }
                    }
                };
                let target = dawsu_hitsounding::rules::read_sample_file_name(&name).ok_or("The moved hitnormal has no sample name.")?;
                lanes.insert(lane.clone(), target);
                if !extra.iter().any(|(n, _)| *n == name) {
                    extra.push((name, bytes));
                }
            }
            if clash && previous.is_some() {
                previous = None;
                continue;
            }
            if writes.is_empty() {
                break (snapshot, lanes, extra, Vec::new());
            }
            if !write {
                return Ok(None);
            }
            let entry = SnapshotEntry {
                set_id: set.id.clone(),
                audio: audio.clone(),
                files: snapshot.set.members.iter().map(|m| SnapshotFile { member_id: m.id.clone(), name: m.lane.name.clone(), sha256: sha256_hex(&m.audio) })
                    .chain(extra.iter().map(|(name, bytes)| SnapshotFile { member_id: MOMENT_HITNORMAL.into(), name: name.clone(), sha256: sha256_hex(bytes.as_slice()) }))
                    .collect(),
                done: false,
            };
            // A name another program takes between planning and writing is planned again once.
            if attempt == 1 && writes.iter().any(|(name, _)| root.join(name).exists()) {
                continue;
            }
            match write_snapshot(&root, &mut record, entry, &writes) {
                Ok(clips) => break (snapshot, lanes, extra, clips),
                Err(_) if attempt == 1 && writes.iter().any(|(name, _)| root.join(name).exists()) => continue,
                Err(error) => return Err(error),
            }
        };
        let wrote = !clips.is_empty();
        let mut additions = clips;
        for name in snapshot.set.members.iter().map(|m| &m.lane.name).chain(extra.iter().map(|(name, _)| name)) {
            if !additions.iter().any(|(n, _)| n == name) && !self.samples.contains_key(&SampleIdentity::File(name.clone())) {
                let path = safe_child(&root, name)?;
                additions.push((name.clone(), dawsu_audio::decode(&path).map_err(|e| format!("{name}: {e}"))?));
            }
        }
        self.register_added_samples(additions);
        Ok(Some((snapshot.set, lanes, wrote)))
    }

    /// Plays the library's version of `set_id` on its placements as one undo step named `label`.
    /// With `only` (a placement and its new gain), only that placement changes, unless the set's
    /// files had to be copied again: then every placement moves to them (M7).
    /// Returns the set and, for the status line, the moments where the mapper's own hitnormal
    /// kept the set's new one out. Placements kept on an older version stay as they are when
    /// only one placement's volume changes.
    fn rerender_set(&mut self, set_id: &str, label: impl FnOnce(&str) -> String, only: Option<(u64, i32)>) -> Result<(SoundSet, String), String> {
        let (set, digest) = self.library_version(set_id)?;
        let placements: Vec<_> = {
            let hits = self.document.as_ref().ok_or("Open a difficulty first.")?.composer.hitsounds();
            hits.placements().iter().filter(|p| p.set_id == set_id && hits.is_placement_live(p)).cloned().collect()
        };
        if placements.is_empty() {
            return Ok((set, String::new()));
        }
        let set_has_hitnormal = set.members.iter().any(|m| m.slot.ends_with("-hitnormal"));
        let moved = if set_has_hitnormal { Vec::new() } else { self.moved_hitnormals(set_id)? };
        let (snapshot, lanes, wrote) = self.snapshot_set(&set, true, &moved)?.expect("a written snapshot");
        let updates = placements.iter()
            .filter(|p| match only {
                None => true,
                Some((id, _)) => p.id == id || wrote && p.set_digest == digest,
            })
            .map(|p| {
                let gain = only.filter(|(id, _)| *id == p.id).map_or(p.gain, |(_, gain)| gain);
                Ok((p.id, digest.clone(), gain, placement_member_hits(&snapshot, gain, p, &lanes)?))
            })
            .collect::<Result<Vec<_>, String>>()?;
        let composer = &mut self.document.as_mut().expect("open document").composer;
        let kept = composer.update_placements(&label(&set.name), &updates);
        self.sync_audio();
        let note = if kept.is_empty() { String::new() } else {
            format!(" Kept your own hitnormal at {}.", kept.iter().map(|ms| clock(*ms)).collect::<Vec<_>>().join(", "))
        };
        Ok((set, note))
    }

    /// Updates every placement of `set_id` in the open difficulty to the library's version as
    /// one undo step, "Update sound set <name>" (M5: the hits play the mapset copies).
    pub fn update_sound_set(&mut self, set_id: &str) -> Result<(), String> {
        if self.busy || self.is_stroke_open() {
            return Err("Finish the current operation before updating a sound set.".into());
        }
        let count = self.document.as_ref().map_or(0, |d| {
            let hits = d.composer.hitsounds();
            hits.placements().iter().filter(|p| p.set_id == set_id && hits.is_placement_live(p)).count()
        });
        let (set, note) = self.rerender_set(set_id, |name| format!("Update sound set {name}"), None)?;
        self.status = format!("Updated {count} {} of \"{}\".{note}", if count == 1 { "placement" } else { "placements" }, set.name);
        Ok(())
    }

    fn placement(&self, id: u64) -> Result<dawsu_hitsounding::sound_sets::SoundSetPlacement, String> {
        let hits = self.document.as_ref().ok_or("Open a difficulty first.")?.composer.hitsounds();
        hits.placements().iter().find(|p| p.id == id && hits.is_placement_live(p)).cloned().ok_or_else(|| "That sound set placement is gone.".into())
    }

    /// The Sound set placement dialog's view of one placement, from the library index.
    pub fn sound_set_placement(&self, id: u64) -> Result<PlacementView, String> {
        let p = self.placement(id)?;
        let (edited, sounds) = self.document.as_ref().expect("open document").composer.placement_edits(id).unwrap_or_default();
        let state = look_up(self.preferences.hitsound_library.as_deref(), &[p.set_id.clone()]).pop().map(|(_, s)| s);
        let (name, frozen, newer) = match state {
            Some(InLibrary::Found { name, digest, .. }) if digest == p.set_digest => (Some(name), None, false),
            Some(InLibrary::Found { name, .. }) => (Some(name), Some("The library has another version of this set.".to_string()), true),
            Some(InLibrary::Unsure(why)) => (None, Some(why), false),
            _ => (None, Some("Not in library".to_string()), false),
        };
        Ok(PlacementView { id, set_id: p.set_id, name, edited, sounds, gain: p.gain, frozen, newer })
    }

    /// Set volume %: plays the placement's set at `gain` as one undo step. Sounds edited by hand
    /// keep their hits. Needs the set's version in the library.
    pub fn set_sound_set_placement_gain(&mut self, id: u64, gain: i32) -> Result<(), String> {
        if self.busy || self.is_stroke_open() {
            return Err("Finish the current operation before changing a sound set.".into());
        }
        if gain < dawsu_hitsounding::rules::MINIMUM_PLAYED_VOLUME {
            return Err(format!("Volume must be at least {}%.", dawsu_hitsounding::rules::MINIMUM_PLAYED_VOLUME));
        }
        if gain > 100 {
            return Err("Volume can be at most 100%.".into());
        }
        let p = self.placement(id)?;
        let (set, digest) = self.library_version(&p.set_id)?;
        if digest != p.set_digest {
            return Err(format!("The library has another version of \"{}\". Update the set first.", set.name));
        }
        let (set, note) = self.rerender_set(&p.set_id, |_| "Set sound set volume".into(), Some((id, gain)))?;
        self.status = format!("{} set to {gain}%.{note}", set.name);
        Ok(())
    }

    /// Reset edits: puts the sounds edited by hand back as placed, one undo step. With the set's
    /// version in the library and its files in the mapset, they follow its values.
    pub fn reset_sound_set_placement(&mut self, id: u64) -> Result<(), String> {
        if self.busy || self.is_stroke_open() {
            return Err("Finish the current operation before changing a sound set.".into());
        }
        let p = self.placement(id)?;
        let current = match self.library_version(&p.set_id) {
            Ok((set, digest)) if digest == p.set_digest => {
                let moved = if set.members.iter().any(|m| m.slot.ends_with("-hitnormal")) { Vec::new() } else { self.moved_hitnormals(&p.set_id)? };
                match self.snapshot_set(&set, false, &moved)? {
                    Some((snapshot, lanes, _)) => Some((digest, placement_member_hits(&snapshot, p.gain, &p, &lanes)?)),
                    None => None,
                }
            }
            _ => None,
        };
        let composer = &mut self.document.as_mut().expect("open document").composer;
        composer.reset_placement(id, current.as_ref().map(|(d, h)| (d.as_str(), h.as_slice())));
        self.status = "Sound set edits reset.".into();
        self.sync_audio();
        Ok(())
    }

    /// Unlink: removes the placement's link as one undo step; its hits stay.
    pub fn unlink_sound_set_placement(&mut self, id: u64) -> Result<(), String> {
        if self.busy || self.is_stroke_open() {
            return Err("Finish the current operation before changing a sound set.".into());
        }
        self.placement(id)?;
        self.document.as_mut().expect("open document").composer.unlink_placement(id);
        self.status = "Sound set unlinked. Its hits stay.".into();
        Ok(())
    }
}

/// A snapshot set's hits at `gain`: member id, lane and volume.
fn member_hits(set: &SoundSet, gain: i32) -> Result<Vec<(String, SampleIdentity, i32)>, String> {
    placement_hits(set, gain).into_iter().zip(&set.members).map(|((key, volume), m)| {
        let lane = crate::project::lane_identity(&key).ok_or_else(|| format!("{} is not a sample file name", m.lane.name))?;
        Ok((m.id.clone(), lane, volume))
    }).collect()
}

// ---------------------------------------------------------------- the links in the project

/// The sound set links of the last Save, kept in dawsu!'s project (`SessionSettings.sound_sets`;
/// older builds drop the field, so the links are lost but the hits stay). Like the left-out
/// moments, a record is staged before the `.osu` write and promoted after it, and names each
/// placement's moment by its object's ordinal in the written file.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Links {
    current: Option<LinkRecord>,
    pending: Option<LinkRecord>,
    before: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct LinkRecord {
    map_digest: String,
    placements: Vec<PlacementRecord>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct PlacementRecord {
    id: u64,
    set_id: String,
    set_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    accepted_digest: Option<String>,
    gain: i32,
    /// The moment's object by its ordinal in the written file, and its slider node; none for a
    /// moment with no object, found again by its time and hits.
    object: Option<usize>,
    node: Option<usize>,
    time_bits: u64,
    applied: Vec<AppliedRecord>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct AppliedRecord {
    member_id: String,
    lane: String,
    volume: i32,
    added: bool,
}

impl Links {
    fn for_digest(&self, digest: &str) -> Option<&LinkRecord> {
        self.pending.as_ref()
            .filter(|record| record.map_digest == digest && self.before.as_deref() != Some(digest))
            .or_else(|| self.current.as_ref().filter(|record| record.map_digest == digest))
    }

    /// True when no revision holds a link.
    pub(crate) fn is_empty(&self) -> bool {
        self.current.iter().chain(self.pending.iter()).all(|record| record.placements.is_empty())
    }

    /// Before writing `before`'s replacement: the record for the new file waits until it is written.
    pub(crate) fn stage(&mut self, before: &[u8], after: LinkRecord) {
        let digest = sha256_hex(before);
        if let Some(current) = self.for_digest(&digest) {
            self.current = Some(current.clone());
        }
        self.before = Some(digest);
        self.pending = Some(after);
    }

    /// After the write: the staged record is the current one when it names the written bytes.
    pub(crate) fn promote(&mut self, written: &[u8]) {
        let digest = sha256_hex(written);
        if self.pending.as_ref().is_some_and(|pending| pending.map_digest == digest) {
            self.current = self.pending.take();
            self.before = None;
        }
    }

    /// When the map bytes are unchanged there is nothing to wait for.
    pub(crate) fn set_current(&mut self, record: LinkRecord) {
        self.current = Some(record);
        self.pending = None;
        self.before = None;
    }
}

impl LinkRecord {
    pub(crate) fn is_empty(&self) -> bool {
        self.placements.is_empty()
    }
}

fn location(place: dawsu_hitsounding::Place) -> Option<(dawsu_osu_format::LineId, Option<usize>)> {
    use dawsu_hitsounding::Place;
    match place {
        Place::Circle(id) | Place::SpinnerEnd(id) => Some((id.line(), None)),
        Place::SliderNode { object, node } => Some((object.line(), Some(node))),
        Place::NoObject => None,
    }
}

/// The record of `drawn`'s placements for `out`, the file this Save writes from `base`. In HS mode
/// a moment with no object is bound to the circle its group became (`circles`, by group time).
pub(crate) fn build_links(
    drawn: &dawsu_hitsounding::Hitsounds, base: &dawsu_osu_format::OsuFile, out: &dawsu_osu_format::OsuFile,
    circles: &std::collections::BTreeMap<i64, dawsu_osu_format::LineId>, hs: bool, kept: &[PlacementRecord],
    accepted: &std::collections::BTreeMap<String, String>,
) -> LinkRecord {
    let ordinals: std::collections::HashMap<_, _> = out.hit_objects().iter().enumerate().map(|(i, o)| (o.line(), i)).collect();
    // A placement whose added hits were all deleted links nothing any more.
    let live = |p: &&dawsu_hitsounding::sound_sets::SoundSetPlacement| {
        let added: Vec<_> = p.applied.iter().filter(|a| a.added).collect();
        added.is_empty() || added.iter().any(|a| drawn.moment(p.moment).has_hit(&a.lane))
    };
    let placements = drawn.placements().iter().filter(live).map(|p| {
        let moment = drawn.moment(p.moment);
        let bound = location(moment.place())
            .or_else(|| if hs { circles.get(&dawsu_hitsounding::hs::hs_group_time(base, moment.time())).map(|line| (*line, None)) } else { None })
            .and_then(|(line, node)| Some((*ordinals.get(&line)?, node)));
        PlacementRecord {
            id: p.id,
            set_id: p.set_id.clone(),
            set_digest: p.set_digest.clone(),
            // Keep this session, unless the placement has since been updated to that version.
            accepted_digest: accepted.get(&p.set_id).filter(|d| **d != p.set_digest).cloned().or_else(|| p.accepted_digest.clone()),
            gain: p.gain,
            object: bound.map(|b| b.0),
            node: bound.and_then(|b| b.1),
            time_bits: moment.time().ms().to_bits(),
            applied: p.applied.iter().map(|a| AppliedRecord { member_id: a.member_id.clone(), lane: crate::project::lane_key(&a.lane), volume: a.volume, added: a.added }).collect(),
        }
    }).chain(kept.iter().cloned()).collect();
    LinkRecord { map_digest: sha256_hex(&out.to_bytes()), placements }
}

/// Puts the links of the record for the opened file back, last on open, without an undo step. A
/// placement on an object is found by the object's ordinal; one with no object by its time. Either
/// way one of the hits it added must still be there. Returns the placements that could not be put
/// back, which the project keeps and the next Save writes again (M3), and a warning naming them.
pub(crate) fn restore_links(document: &mut super::Document, links: &Links) -> (Vec<PlacementRecord>, Option<String>) {
    let digest = sha256_hex(document.file.original_bytes());
    let Some(record) = links.for_digest(&digest) else {
        let kept = links.current.as_ref().or(links.pending.as_ref()).map(|r| r.placements.clone()).unwrap_or_default();
        let warning = (!kept.is_empty()).then(|| "The difficulty changed outside dawsu!, so its sound set links were not restored. The hits stay; the project keeps the links.".to_string());
        return (kept, warning);
    };
    let objects = document.file.hit_objects();
    let hits = document.composer.hitsounds();
    let mut restored: Vec<dawsu_hitsounding::sound_sets::SoundSetPlacement> = Vec::new();
    let mut kept = Vec::new();
    let mut missing = Vec::new();
    for saved in &record.placements {
        let time = f64::from_bits(saved.time_bits);
        let applied = saved.applied.iter().map(|a| Some(dawsu_hitsounding::sound_sets::Applied {
            member_id: a.member_id.clone(), lane: crate::project::lane_identity(&a.lane)?, volume: a.volume, added: a.added,
        })).collect::<Option<Vec<_>>>();
        let holds = |m: &dawsu_hitsounding::Moment, applied: &[dawsu_hitsounding::sound_sets::Applied]| {
            let mut added = applied.iter().filter(|a| a.added).peekable();
            added.peek().is_none() || added.any(|a| m.has_hit(&a.lane))
        };
        let found = applied.and_then(|applied| {
            let moment = match saved.object {
                Some(ordinal) => {
                    let line = objects.get(ordinal)?.line();
                    // A kept record's ordinal may name another object now, so the time must match too.
                    hits.moments().iter().position(|m| location(m.place()) == Some((line, saved.node)) && (m.time().ms() - time).abs() < 1.0 && holds(m, &applied))
                }
                None => hits.moments().iter().enumerate()
                    .filter(|(_, m)| (m.time().ms() - time).abs() < 1.0 && holds(m, &applied))
                    .min_by(|(_, a), (_, b)| (a.time().ms() - time).abs().total_cmp(&(b.time().ms() - time).abs()))
                    .map(|(i, _)| i),
            }?;
            // Records kept from an earlier open may share an id with a newer placement.
            let id = if restored.iter().any(|p| p.id == saved.id) { restored.iter().map(|p| p.id).max().unwrap_or(0) + 1 } else { saved.id };
            Some(dawsu_hitsounding::sound_sets::SoundSetPlacement {
                id, set_id: saved.set_id.clone(), set_digest: saved.set_digest.clone(), accepted_digest: saved.accepted_digest.clone(),
                moment: dawsu_hitsounding::MomentId(moment), gain: saved.gain, applied,
            })
        });
        match found {
            Some(placement) => restored.push(placement),
            None => {
                missing.push(clock(time));
                kept.push(saved.clone());
            }
        }
    }
    document.composer.history.update_external(|hits| hits.restore_placements(restored.clone()));
    let warning = (!missing.is_empty()).then(|| format!("Link not restored for the sound set placed at {}: its hits changed. The hits stay; the project keeps the link.", missing.join(", ")));
    (kept, warning)
}

fn clock(ms: f64) -> String {
    let rounded = ms.round().max(0.0) as u64;
    format!("{:02}:{:02}.{:03}", rounded / 60_000, rounded % 60_000 / 1000, rounded % 1000)
}


#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use dawsu_hitsounding::sound_sets::{Kept, SetLane, SetMember, write_dawsuset};
    use dawsu_test_support::Scratch;
    use std::time::{Duration, Instant};

    pub(crate) const SET_ID: &str = "9e7b1d18-3186-4778-9a6a-b6baef95cbe7";
    pub(crate) const MAP: &[u8] = b"osu file format v14\n[General]\nMode:0\n[TimingPoints]\n0,500,4,1,0,70,1,0\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n";

    pub(crate) fn wav(frame: i16) -> Vec<u8> {
        let mut wav = b"RIFF\x26\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data\x02\0\0\0".to_vec();
        wav.extend(frame.to_le_bytes());
        wav
    }

    pub(crate) fn set_text(name: &str, frames: &[(&str, i16)]) -> String {
        set_text_at(name, frames, 60)
    }

    pub(crate) fn set_text_at(name: &str, frames: &[(&str, i16)], volume: i32) -> String {
        let members = frames.iter().enumerate().map(|(i, (lane, frame))| SetMember {
            id: format!("aaaaaaaa-0000-4000-8000-{i:012}"),
            original_key: format!("BeatmapFile|{lane}"),
            lane: SetLane { source: SetSource::File, name: lane.to_string(), missing: false, kept: Kept::default() },
            slot: lane.trim_end_matches(".wav").to_string(),
            volume,
            audio: wav(*frame),
            kept: Kept::default(),
        }).collect();
        write_dawsuset(&SoundSet { id: SET_ID.into(), name: name.into(), members, library_id: None, kept: Kept::default() }).unwrap()
    }

    pub(crate) struct Fixture {
        pub scratch: Scratch,
        pub map: PathBuf,
        pub set: PathBuf,
    }

    pub(crate) fn fixture(name: &str) -> Fixture {
        let scratch = Scratch::new(name);
        let mapset = scratch.path("mapset");
        std::fs::create_dir(&mapset).unwrap();
        let map = mapset.join("a.osu");
        std::fs::write(&map, MAP).unwrap();
        let set = scratch.path("Kick and clap.dawsuset");
        std::fs::write(&set, set_text("Kick and clap", &[("drum-hitnormal.wav", 8000), ("drum-hitclap.wav", 4000)])).unwrap();
        Fixture { scratch, map, set }
    }

    pub(crate) fn app(f: &Fixture) -> Application {
        let mut app = Application::with_data_root(f.scratch.path("data"));
        app.error = None;
        // No audio device: the playhead is then exactly where a test puts it.
        app.audio = None;
        app.project_root = Some(f.scratch.path("projects"));
        app.preferences.hitsound_library = Some(f.scratch.path("library"));
        app
    }

    fn app_for(f: &Fixture) -> Application {
        app(f)
    }

    pub(crate) fn wait(app: &mut Application) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.busy {
            app.poll();
            assert!(Instant::now() < deadline, "{}", app.status);
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Opens the map and returns the path the document uses, which commands name.
    pub(crate) fn open(app: &mut Application, map: &Path) -> PathBuf {
        app.request_open(map.to_path_buf());
        wait(app);
        assert!(app.document().is_some(), "{:?}", app.error);
        app.document().unwrap().path.clone()
    }

    fn mapset_files(f: &Fixture) -> Vec<String> {
        let mut files = list_folder(&f.map.with_file_name("")).unwrap();
        files.sort();
        files
    }

    #[test]
    fn placing_copies_the_set_into_the_library_and_its_samples_into_the_mapset_at_one_index() {
        let f = fixture("place-set");
        let mut app = app(&f);
        let map = open(&mut app, &f.map);
        app.seek(1000.0);
        app.place_sound_set_file(&map, &f.set).unwrap();
        assert_eq!(mapset_files(&f), ["a.osu", "drum-hitclap2.wav", "drum-hitnormal2.wav"]);
        assert_eq!(std::fs::read(f.map.with_file_name("drum-hitnormal2.wav")).unwrap(), wav(8000));
        let library = f.scratch.path("library").join(LIBRARY_FOLDER).join("Kick and clap.dawsuset");
        assert_eq!(std::fs::read(&library).unwrap(), std::fs::read(&f.set).unwrap());
        let hits = app.document().unwrap().composer.hitsounds();
        let p = &hits.placements()[0];
        assert_eq!((p.moment.0, p.set_id.as_str()), (0, SET_ID), "on the circle at the playhead");
        assert_eq!(p.set_digest, sha256_hex(&std::fs::read(&f.set).unwrap()));
        assert!(p.applied.iter().all(|a| a.added && a.volume == 60));
        assert_eq!(hits.moment(p.moment).hits().map(crate::lane_name).collect::<Vec<_>>(), ["drum-hitnormal2", "drum-hitclap2"], "the set's hitnormal replaces the circle's (D6)");
        assert!(app.is_dirty());
        // Playback plays both copied sounds.
        let sounds = app.scheduled_sounds(app.document().unwrap());
        assert_eq!(sounds.iter().filter(|s| s.at_ms == 1000.0).count(), 2);
        // Undo leaves the copied files (D5); placing again reuses them.
        app.command("undo");
        assert!(app.document().unwrap().composer.hitsounds().placements().is_empty());
        app.place_sound_set_file(&map, &f.set).unwrap();
        assert_eq!(mapset_files(&f), ["a.osu", "drum-hitclap2.wav", "drum-hitnormal2.wav"], "no new file");
        assert_eq!(app.document().unwrap().composer.hitsounds().placements().len(), 1);
    }

    #[test]
    fn a_changed_snapshot_file_is_never_overwritten_and_the_set_is_copied_again() {
        let f = fixture("place-set-changed");
        let mut app = app(&f);
        let map = open(&mut app, &f.map);
        app.seek(1000.0);
        app.place_sound_set_file(&map, &f.set).unwrap();
        std::fs::write(f.map.with_file_name("drum-hitclap2.wav"), wav(1)).unwrap();
        app.seek(1500.0);
        app.place_sound_set_file(&map, &f.set).unwrap();
        assert_eq!(std::fs::read(f.map.with_file_name("drum-hitclap2.wav")).unwrap(), wav(1));
        assert_eq!(mapset_files(&f), ["a.osu", "drum-hitclap2.wav", "drum-hitclap3.wav", "drum-hitnormal2.wav", "drum-hitnormal3.wav"]);
    }

    #[test]
    fn a_failed_copy_leaves_no_file_and_no_record_entry() {
        let f = fixture("place-set-fails");
        // The second member's audio does not decode, so the first file is taken back.
        std::fs::write(&f.set, set_text("Broken", &[("drum-hitnormal.wav", 8000), ("drum-hitclap.wav", 4000)]).replace(&dawsu_hitsounding::drums::base64_encode(&wav(4000)), &dawsu_hitsounding::drums::base64_encode(b"not audio"))).unwrap();
        let mut app = app(&f);
        let map = open(&mut app, &f.map);
        let version = app.document().unwrap().composer.history.version();
        assert!(app.place_sound_set_file(&map, &f.set).is_err());
        assert_eq!(mapset_files(&f), ["a.osu"]);
        assert_eq!(app.document().unwrap().composer.history.version(), version, "nothing placed");
        let records: Vec<_> = std::fs::read_dir(f.scratch.path("data").join("sound-set-snapshots")).unwrap().flatten().filter(|e| e.path().extension().is_some_and(|x| x == "json")).collect();
        for entry in records {
            let r: SnapshotRecord = serde_json::from_slice(&std::fs::read(entry.path()).unwrap()).unwrap();
            assert!(r.entries.is_empty(), "{r:?}");
        }
        let library = f.scratch.path("library").join(LIBRARY_FOLDER);
        assert!(std::fs::read_dir(&library).map_or(true, |mut entries| entries.next().is_none()), "no set left in the library");
    }

    #[test]
    fn without_a_library_placing_asks_for_one_and_a_different_version_in_the_library_is_not_overwritten() {
        let f = fixture("place-set-library");
        let mut app = app(&f);
        let map = open(&mut app, &f.map);
        app.preferences.hitsound_library = None;
        assert!(app.place_sound_set_file(&map, &f.set).unwrap_err().contains("hitsound library"));
        app.preferences.hitsound_library = Some(f.scratch.path("library"));
        let folder = f.scratch.path("library").join(LIBRARY_FOLDER);
        std::fs::create_dir_all(&folder).unwrap();
        let other = set_text("Kick and clap", &[("drum-hitnormal.wav", 1)]);
        std::fs::write(folder.join("older.dawsuset"), &other).unwrap();
        listed(&app);
        assert!(app.place_sound_set_file(&map, &f.set).unwrap_err().contains("another version"));
        assert_eq!(std::fs::read_to_string(folder.join("older.dawsuset")).unwrap(), other);
        assert_eq!(mapset_files(&f), ["a.osu"]);
    }

    // ------------------------------------------------------------ links through Save and reopen

    pub(crate) fn save(app: &mut Application, f: &Fixture) {
        app.save_with_backups(f.scratch.path("backups"));
        wait(app);
        if app.save_problem_review() {
            app.answer_save_review(true);
            wait(app);
        }
        assert!(app.error.is_none(), "{:?}", app.error);
    }

    fn reopen(f: &Fixture) -> Application {
        let mut app = app(f);
        open(&mut app, &f.map);
        app
    }

    fn links(app: &Application) -> Vec<(usize, String)> {
        app.document().unwrap().composer.hitsounds().placements().iter().map(|p| (p.moment.0, p.set_id.clone())).collect()
    }

    fn project_bytes(f: &Fixture) -> Vec<Vec<u8>> {
        let mut files: Vec<_> = walk(&f.scratch.path("projects"));
        files.sort();
        files.into_iter().filter(|p| !p.to_string_lossy().contains("backups")).map(|p| std::fs::read(p).unwrap()).collect()
    }

    fn walk(dir: &Path) -> Vec<PathBuf> {
        std::fs::read_dir(dir).map(|entries| entries.flatten().flat_map(|e| if e.path().is_dir() { walk(&e.path()) } else { vec![e.path()] }).collect()).unwrap_or_default()
    }

    #[test]
    fn a_placement_survives_save_a_view_change_another_save_and_reopen() {
        let f = fixture("links-reopen");
        let mut app = app(&f);
        let map = open(&mut app, &f.map);
        app.seek(1000.0);
        app.place_sound_set_file(&map, &f.set).unwrap();
        save(&mut app, &f);
        // M1: a session write and a second Save keep the link written by the first.
        app.zoom = 0.5;
        app.save_session().unwrap();
        save(&mut app, &f);
        let (osu, project) = (std::fs::read(&f.map).unwrap(), project_bytes(&f));
        save(&mut app, &f);
        assert_eq!((std::fs::read(&f.map).unwrap(), project_bytes(&f)), (osu, project), "saving again changes neither file");
        let reopened = reopen(&f);
        assert_eq!(links(&reopened), [(0, SET_ID.to_string())]);
        assert!(reopened.warnings.is_empty(), "{:?}", reopened.warnings);
        assert!(!reopened.is_dirty(), "restoring the link is no edit");
        let p = &reopened.document().unwrap().composer.hitsounds().placements()[0];
        assert_eq!(p.applied.iter().map(|a| (crate::lane_name(&a.lane), a.volume, a.added)).collect::<Vec<_>>(),
            [("drum-hitnormal2".to_string(), 60, true), ("drum-hitclap2".to_string(), 60, true)]);
    }

    #[test]
    fn a_placement_with_no_object_comes_back_from_the_project_and_in_hs_mode_from_its_circle() {
        for hs in [false, true] {
            let f = fixture(if hs { "links-loose-hs" } else { "links-loose" });
            let mut app = app(&f);
            let map = open(&mut app, &f.map);
            app.set_hs_mode(hs);
            app.seek(1500.0);
            app.place_sound_set_file(&map, &f.set).unwrap();
            save(&mut app, &f);
            if hs {
                // The set saves natively: one circle at the set's index, no rendered mix.
                let text = String::from_utf8(std::fs::read(&f.map).unwrap()).unwrap();
                let circle = text.lines().find(|l| l.starts_with("256,192,1500,")).expect("the circle the set became");
                assert_eq!(circle, "256,192,1500,1,8,3:0:2:60:", "drum hitnormal and clap at index 2, volume 60");
                assert_eq!(mapset_files(&f), ["a.osu", "drum-hitclap2.wav", "drum-hitnormal2.wav"]);
            }
            let reopened = reopen(&f);
            let hits = reopened.document().unwrap().composer.hitsounds();
            assert_eq!(hits.placements().len(), 1, "hs {hs}: {:?}", reopened.warnings);
            assert_eq!(hits.moment(hits.placements()[0].moment).time().ms(), 1500.0);
            assert!(reopened.warnings.is_empty(), "{:?}", reopened.warnings);
        }
    }

    #[test]
    fn unlinking_with_the_map_unchanged_still_writes_the_project() {
        let f = fixture("links-unlink");
        let mut app = app(&f);
        let map = open(&mut app, &f.map);
        app.seek(1000.0);
        app.place_sound_set_file(&map, &f.set).unwrap();
        save(&mut app, &f);
        let osu = std::fs::read(&f.map).unwrap();
        let id = app.document().unwrap().composer.hitsounds().placements()[0].id;
        app.document.as_mut().unwrap().composer.unlink_placement(id);
        save(&mut app, &f);
        assert_eq!(std::fs::read(&f.map).unwrap(), osu);
        assert!(links(&reopen(&f)).is_empty());
    }

    #[test]
    fn a_map_changed_outside_dawsu_keeps_its_hits_drops_the_link_warns_and_keeps_the_record() {
        let f = fixture("links-changed");
        let mut app = app(&f);
        let map = open(&mut app, &f.map);
        app.seek(1000.0);
        app.place_sound_set_file(&map, &f.set).unwrap();
        save(&mut app, &f);
        let mut edited = std::fs::read(&f.map).unwrap();
        edited.extend(b"// edited in another program\n");
        std::fs::write(&f.map, &edited).unwrap();
        let reopened = reopen(&f);
        assert!(links(&reopened).is_empty());
        assert!(reopened.warnings.iter().any(|w| w.contains("sound set links were not restored")), "{:?}", reopened.warnings);
        let hits = reopened.document().unwrap().composer.hitsounds();
        // The set's hitnormal replaced the circle's (D6), so the set saved natively.
        assert_eq!(hits.moment(dawsu_hitsounding::MomentId(0)).hits().map(crate::lane_name).collect::<Vec<_>>(), ["drum-hitnormal2", "drum-hitclap2"], "the written hits stay");
        assert!(!reopened.project.as_ref().unwrap().settings.sound_sets.as_ref().unwrap().is_empty(), "the record is kept");
    }

    #[test]
    fn an_old_project_without_the_field_loads() {
        let settings: crate::project::SessionSettings = serde_json::from_str(r#"{"song_offset":0,"zoom":0.3}"#).unwrap();
        assert!(settings.sound_sets.is_none());
        let json = serde_json::to_value(&crate::project::SessionSettings::default()).unwrap();
        assert!(json.get("sound_sets").is_none(), "no field is written without links");
    }

    #[test]
    fn without_a_project_save_refuses_rather_than_drop_the_link() {
        let f = fixture("links-no-project");
        let mut app = app(&f);
        let map = open(&mut app, &f.map);
        app.seek(1000.0);
        app.place_sound_set_file(&map, &f.set).unwrap();
        app.project = None;
        app.save_with_backups(f.scratch.path("backups"));
        wait(&mut app);
        assert!(app.error.as_deref().is_some_and(|e| e.contains("editor project")), "{:?}", app.error);
        assert_eq!(std::fs::read(&f.map).unwrap(), MAP);
    }

    #[test]
    fn gain_recovery_keeps_a_placed_moment_at_one_volume_only_when_the_save_mixes_it() {
        let f = fixture("links-gain-recovery");
        let mut app = app(&f);
        let map = open(&mut app, &f.map);
        app.seek(1000.0);
        app.place_sound_set_file(&map, &f.set).unwrap();
        let composer = &mut app.document.as_mut().unwrap().composer;
        let moment = composer.hitsounds().placements()[0].moment;
        composer.apply("One volume", |h| h.set_volume(moment, 60));
        let hits = composer.hitsounds();
        assert!(!hits.moment(moment).has_hit_volume_overrides());
        let defaults = f.scratch.path("defaults");
        std::fs::create_dir(&defaults).unwrap();
        std::fs::write(defaults.join("normal-hitnormal.wav"), wav(100)).unwrap();
        let circle = match hits.moment(moment).place() {
            dawsu_hitsounding::Place::Circle(id) => (id.line(), None),
            other => panic!("{other:?}"),
        };
        let read = |mixed: &crate::gain_recovery::Locations| crate::gain_recovery::Captured::read(hits, map.parent().unwrap(), &defaults, mixed).unwrap().needs_recovery();
        assert!(read(&[circle].into()), "a placed moment the Save mixes");
        assert!(!read(&Default::default()), "a placed moment written natively needs no record");
    }

    #[test]
    fn a_recovery_copy_carries_the_links_of_the_working_hits_and_of_the_last_save() {
        let f = fixture("links-recovery");
        let mut app = app(&f);
        let map = open(&mut app, &f.map);
        app.seek(1500.0);
        app.place_sound_set_file(&map, &f.set).unwrap();
        save(&mut app, &f);
        app.seek(2000.0);
        app.place_sound_set_file(&map, &f.set).unwrap();
        app.poll();
        let offer = app.write_recovery_copy().unwrap().unwrap();
        let mut restored = Application::create(crate::Preferences::default(), false, None, None, false, f.scratch.path("data"));
        restored.project_root = app.project_root.clone();
        restored.restore_recovery(&offer.id).unwrap();
        wait(&mut restored);
        assert!(restored.error.is_none(), "{:?}", restored.error);
        let times = |app: &Application| {
            let hits = app.document().unwrap().composer.hitsounds();
            hits.placements().iter().map(|p| hits.moment(p.moment).time().ms()).collect::<Vec<_>>()
        };
        assert_eq!(times(&restored), [1500.0, 2000.0]);
        restored.command("undo");
        assert_eq!(times(&restored), [1500.0], "undo returns to the last Save, with its link");
    }

    #[test]
    fn a_placement_whose_hits_were_all_deleted_is_not_recorded_and_reopens_without_a_warning() {
        let f = fixture("links-deleted");
        let mut app = app(&f);
        let map = open(&mut app, &f.map);
        app.seek(1000.0);
        app.place_sound_set_file(&map, &f.set).unwrap();
        app.document.as_mut().unwrap().composer.remove_selected();
        assert_eq!(app.document().unwrap().composer.hitsounds().placements().len(), 1, "the link outlives its hits until Save");
        save(&mut app, &f);
        let reopened = reopen(&f);
        assert!(links(&reopened).is_empty());
        assert!(reopened.warnings.is_empty(), "{:?}", reopened.warnings);
    }

    #[test]
    fn a_partly_deleted_set_on_an_object_keeps_its_link() {
        let f = fixture("links-partly-deleted");
        let mut app = app(&f);
        let map = open(&mut app, &f.map);
        app.seek(1000.0);
        app.place_sound_set_file(&map, &f.set).unwrap();
        let composer = &mut app.document.as_mut().unwrap().composer;
        let clap = composer.hitsounds().placements()[0].applied[1].lane.clone();
        composer.selected.replace([dawsu_editor_composer::HitKey { moment: 0, lane: clap }]);
        composer.remove_selected();
        save(&mut app, &f);
        let reopened = reopen(&f);
        assert_eq!(links(&reopened), [(0, SET_ID.to_string())]);
        assert!(reopened.warnings.is_empty(), "{:?}", reopened.warnings);
    }

    // ------------------------------------------------------------ D6b: additions only

    /// A soft map with a soft hitnormal file in the mapset, circles at 1000 and 1500, and a set
    /// holding only a drum clap at the map's volume.
    fn additions_fixture(name: &str) -> Fixture {
        let scratch = Scratch::new(name);
        let mapset = scratch.path("mapset");
        std::fs::create_dir(&mapset).unwrap();
        let map = mapset.join("a.osu");
        std::fs::write(&map, b"osu file format v14\n[General]\nMode:0\n[TimingPoints]\n0,500,4,2,1,70,1,0\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n256,192,1500,1,0,0:0:0:0:\n").unwrap();
        std::fs::write(mapset.join("soft-hitnormal.wav"), wav(500)).unwrap();
        let set = scratch.path("Clap.dawsuset");
        std::fs::write(&set, set_text_at("Clap", &[("drum-hitclap.wav", 4000)], 70)).unwrap();
        Fixture { scratch, map, set }
    }

    #[test]
    fn d6b_an_additions_only_set_copies_the_moments_hitnormal_to_its_index_and_saves_natively() {
        for hs in [false, true] {
            let f = additions_fixture(if hs { "d6b-hs" } else { "d6b" });
            let mut app = app(&f);
            let map = open(&mut app, &f.map);
            app.set_hs_mode(hs);
            app.seek(1000.0);
            app.place_sound_set_file(&map, &f.set).unwrap();
            assert_eq!(mapset_files(&f), ["a.osu", "drum-hitclap2.wav", "soft-hitnormal.wav", "soft-hitnormal2.wav"]);
            assert_eq!(std::fs::read(f.map.with_file_name("soft-hitnormal2.wav")).unwrap(), wav(500), "the moment's own hitnormal sound");
            let hits = app.document().unwrap().composer.hitsounds();
            let moment = hits.moment(dawsu_hitsounding::MomentId(0));
            assert_eq!(moment.hits().map(crate::lane_name).collect::<Vec<_>>(), ["soft-hitnormal2", "drum-hitclap2"]);
            assert_eq!(moment.hit_volume(&dawsu_hitsounding::rules::read_sample_file_name("soft-hitnormal2.wav").unwrap()), 70);
            app.command("undo");
            assert!(app.document().unwrap().composer.hitsounds().moment(dawsu_hitsounding::MomentId(0)).hits().map(crate::lane_name).eq(["soft-hitnormal"]), "one undo step");
            app.command("redo");
            save(&mut app, &f);
            assert_eq!(mapset_files(&f), ["a.osu", "drum-hitclap2.wav", "soft-hitnormal.wav", "soft-hitnormal2.wav"], "hs {hs}: no rendered mix");
            let written = crate::Document::read(&f.map).unwrap();
            let decoded = written.composer.hitsounds().moment(dawsu_hitsounding::MomentId(0));
            assert_eq!(decoded.hits().map(crate::lane_name).collect::<Vec<_>>(), ["soft-hitnormal2", "drum-hitclap2"], "hs {hs}: plain osu! hitsounds");
            assert_eq!(links(&reopen(&f)), [(0, SET_ID.to_string())]);
        }
    }

    #[test]
    fn d6b_the_copied_hitnormal_is_reused_by_its_checksum() {
        let f = additions_fixture("d6b-reuse");
        let mut app = app(&f);
        let map = open(&mut app, &f.map);
        app.seek(1000.0);
        app.place_sound_set_file(&map, &f.set).unwrap();
        app.seek(1500.0);
        app.place_sound_set_file(&map, &f.set).unwrap();
        assert_eq!(mapset_files(&f), ["a.osu", "drum-hitclap2.wav", "soft-hitnormal.wav", "soft-hitnormal2.wav"], "no new file");
        assert!(app.document().unwrap().composer.hitsounds().moment(dawsu_hitsounding::MomentId(1)).hits().map(crate::lane_name).eq(["soft-hitnormal2", "drum-hitclap2"]));
        // A changed copy is never overwritten: the whole set goes to a new index.
        std::fs::write(f.map.with_file_name("soft-hitnormal2.wav"), wav(1)).unwrap();
        app.command("undo");
        app.place_sound_set_file(&map, &f.set).unwrap();
        assert_eq!(std::fs::read(f.map.with_file_name("soft-hitnormal2.wav")).unwrap(), wav(1));
        assert_eq!(mapset_files(&f), ["a.osu", "drum-hitclap2.wav", "drum-hitclap3.wav", "soft-hitnormal.wav", "soft-hitnormal2.wav", "soft-hitnormal3.wav"]);
    }

    #[test]
    fn d6b_a_moment_with_no_hitnormal_copies_nothing_extra() {
        let f = additions_fixture("d6b-none");
        let mut app = app(&f);
        let map = open(&mut app, &f.map);
        app.seek(2000.0);
        app.place_sound_set_file(&map, &f.set).unwrap();
        assert_eq!(mapset_files(&f), ["a.osu", "drum-hitclap2.wav", "soft-hitnormal.wav"]);
    }

    // ------------------------------------------------------------ checks before writes

    #[test]
    fn a_refused_placement_leaves_no_file_behind() {
        let f = fixture("place-refused");
        let mut app = app(&f);
        let map = open(&mut app, &f.map);
        app.seek(1000.0);
        app.place_sound_set_file(&map, &f.set).unwrap();
        // A changed copy would make a new snapshot; placing on the same moment is refused first.
        std::fs::write(f.map.with_file_name("drum-hitnormal2.wav"), wav(1)).unwrap();
        app.place_sound_set_file(&map, &f.set).unwrap();
        assert!(app.status.contains("already placed"), "{}", app.status);
        assert_eq!(mapset_files(&f), ["a.osu", "drum-hitclap2.wav", "drum-hitnormal2.wav"]);
        // A refusal before any write also leaves the library alone.
        let g = fixture("place-refused-library");
        let mut other = app_for(&g);
        let map = open(&mut other, &g.map);
        std::fs::write(&g.set, b"{}").unwrap();
        assert!(other.place_sound_set_file(&map, &g.set).is_err());
        assert!(!g.scratch.path("library").join(LIBRARY_FOLDER).exists());
        assert_eq!(mapset_files(&g), ["a.osu"]);
    }

    #[test]
    fn a_skin_hitnormal_with_no_default_file_is_left_as_it_is_and_named_in_the_status() {
        let f = fixture("d6b-skin-missing");
        std::fs::write(&f.set, set_text("Clap", &[("drum-hitclap.wav", 4000)])).unwrap();
        let mut app = app(&f);
        app.preferences.default_samples = f.scratch.path("no-defaults");
        let map = open(&mut app, &f.map);
        app.seek(1000.0);
        app.place_sound_set_file(&map, &f.set).unwrap();
        assert_eq!(mapset_files(&f), ["a.osu", "drum-hitclap2.wav"]);
        assert!(app.status.contains("saves as a mix"), "{}", app.status);
        assert_eq!(app.document().unwrap().composer.hitsounds().placements().len(), 1);
    }

    #[test]
    fn a_set_file_over_96_mb_is_refused_before_it_is_read() {
        let f = fixture("set-too-large");
        let mut app = app(&f);
        let map = open(&mut app, &f.map);
        let big = f.scratch.path("big.dawsuset");
        std::fs::File::create(&big).unwrap().set_len(MAX_SET_BYTES + 1).unwrap();
        assert!(app.place_sound_set_file(&map, &big).unwrap_err().contains("over 96 MB"));
    }

    #[test]
    fn a_library_set_file_is_read_again_only_when_its_time_or_size_changes() {
        let f = fixture("library-cache");
        let path = f.scratch.path("cached.dawsuset");
        let text = set_text("Kick", &[("drum-hitnormal.wav", 1)]);
        std::fs::write(&path, &text).unwrap();
        let metadata = std::fs::metadata(&path).unwrap();
        assert_eq!(library_id(&path, &metadata).as_deref(), Some(SET_ID));
        // Same size and time, other bytes: the cached id is used, the file is not read again.
        std::fs::write(&path, "x".repeat(text.len())).unwrap();
        std::fs::File::options().write(true).open(&path).unwrap().set_modified(metadata.modified().unwrap()).unwrap();
        assert_eq!(library_id(&path, &std::fs::metadata(&path).unwrap()).as_deref(), Some(SET_ID));
        std::fs::write(&path, "x").unwrap();
        assert_eq!(library_id(&path, &std::fs::metadata(&path).unwrap()), None, "a changed size reads it again");
    }

    // ------------------------------------------------------------ slice 2: save selection, library list

    /// A map whose circle at 1000 plays drum hitnormal and clap at index 2, volume 50, with both
    /// files in the mapset; a soft whistle from the skin is added at volume 80, and the three hits
    /// are selected. The skin's sounds are in `defaults`.
    fn selection_fixture(name: &str) -> (Fixture, Application, PathBuf) {
        let f = fixture(name);
        std::fs::write(&f.map, b"osu file format v14\n[General]\nMode:0\n[TimingPoints]\n0,500,4,1,0,70,1,0\n[HitObjects]\n256,192,1000,1,8,3:0:2:50:\n256,192,1500,1,0,0:0:0:0:\n").unwrap();
        std::fs::write(f.map.with_file_name("drum-hitnormal2.wav"), wav(8000)).unwrap();
        std::fs::write(f.map.with_file_name("drum-hitclap2.wav"), wav(4000)).unwrap();
        let defaults = f.scratch.path("defaults");
        std::fs::create_dir(&defaults).unwrap();
        std::fs::write(defaults.join("soft-hitwhistle.wav"), wav(300)).unwrap();
        std::fs::create_dir(f.scratch.path("library")).unwrap();
        let mut app = app(&f);
        app.preferences.default_samples = defaults;
        let map = open(&mut app, &f.map);
        let whistle = skin_lane("soft-hitwhistle");
        let composer = &mut app.document.as_mut().unwrap().composer;
        composer.apply("Whistle", |h| h.add_hit_with_volume(dawsu_hitsounding::MomentId(0), whistle.clone(), 80));
        let keys: Vec<_> = composer.hitsounds().moment(dawsu_hitsounding::MomentId(0)).hits().map(|lane| dawsu_editor_composer::HitKey { moment: 0, lane: lane.clone() }).collect();
        assert_eq!(keys.len(), 3);
        composer.selected.replace(keys);
        (f, app, map)
    }

    fn skin_lane(name: &str) -> SampleIdentity {
        match dawsu_hitsounding::rules::read_sample_file_name(&format!("{name}.wav")).unwrap() {
            SampleIdentity::Standard { set, sound, .. } => SampleIdentity::Standard { set, sound, index: dawsu_foundation::CustomIndex::new(0) },
            other => other,
        }
    }

    /// The library set named `name`.
    fn saved(f: &Fixture, name: &str) -> SoundSet {
        library_files(f).into_iter()
            .map(|file| read_dawsuset(&std::fs::read_to_string(f.scratch.path("library").join(LIBRARY_FOLDER).join(file)).unwrap()).unwrap())
            .find(|set| set.name == name).unwrap_or_else(|| panic!("no set named {name}"))
    }

    fn library_files(f: &Fixture) -> Vec<String> {
        let mut files = list_folder(&f.scratch.path("library").join(LIBRARY_FOLDER)).unwrap_or_default();
        files.sort();
        files
    }

    #[test]
    fn saving_the_selection_writes_a_set_with_each_hits_sample_role_and_volume_and_changes_no_hit() {
        let (f, mut app, map) = selection_fixture("save-set");
        let version = app.document().unwrap().composer.history.version();
        let dirty = app.is_dirty();
        app.save_sound_set_from_selection(&map, "  Kick: clap/whistle ").unwrap();
        let set = saved(&f, "Kick: clap/whistle");
        assert_eq!(library_files(&f), [format!("Kick_ clap_whistle-{}.dawsuset", set.id)], "characters a file name can't hold become _");
        assert_eq!(set.name, "Kick: clap/whistle");
        let mut members: Vec<_> = set.members.iter().map(|m| (m.slot.as_str(), m.volume, m.audio.clone(), m.original_key.as_str(), m.lane.key())).collect();
        members.sort();
        assert_eq!(members, [
            ("drum-hitclap", 50, wav(4000), "BeatmapFile|drum-hitclap2.wav", "BeatmapFile|drum-hitclap2.wav".to_string()),
            ("drum-hitnormal", 50, wav(8000), "BeatmapFile|drum-hitnormal2.wav", "BeatmapFile|drum-hitnormal2.wav".to_string()),
            ("soft-hitwhistle", 80, wav(300), "Skin|soft-hitwhistle", "Skin|soft-hitwhistle".to_string()),
        ]);
        // The hits stay ordinary hits: no undo step, no link, nothing to save.
        let document = app.document().unwrap();
        assert_eq!(document.composer.history.version(), version);
        assert!(document.composer.hitsounds().placements().is_empty());
        assert_eq!(app.is_dirty(), dirty);
        assert!(app.status.contains("Kick: clap/whistle"), "{}", app.status);
        // Two sets saved from the same hits get their own ids.
        app.save_sound_set_from_selection(&map, "Again").unwrap();
        let again = saved(&f, "Again");
        assert_ne!(again.id, set.id);
        assert!(again.members.iter().all(|m| set.members.iter().all(|n| n.id != m.id)));
    }

    #[test]
    fn a_name_the_library_already_has_is_refused_and_the_file_is_never_overwritten() {
        let (f, mut app, map) = selection_fixture("save-set-clash");
        app.save_sound_set_from_selection(&map, "Kick").unwrap();
        let path = f.scratch.path("library").join(LIBRARY_FOLDER).join(&library_files(&f)[0]);
        let first = std::fs::read(&path).unwrap();
        for name in ["Kick", "kick", " KICK "] {
            assert!(matches!(app.save_sound_set_from_selection(&map, name), Err(SaveSetError::NameTaken(_))), "{name}");
        }
        assert_eq!(std::fs::read(&path).unwrap(), first);
        // A set of that name in a file of another name counts too.
        std::fs::rename(&path, path.with_file_name("old.dawsuset")).unwrap();
        assert!(matches!(app.save_sound_set_from_selection(&map, "Kick"), Err(SaveSetError::NameTaken(_))));
        assert_eq!(library_files(&f), ["old.dawsuset"]);
    }

    #[test]
    fn saving_needs_a_library_and_sounds_at_one_time_that_can_be_read() {
        let (f, mut app, map) = selection_fixture("save-set-refused");
        let refused = |app: &mut Application| match app.save_sound_set_from_selection(&map, "Set") {
            Err(SaveSetError::Refused(why)) => why,
            other => panic!("{other:?}"),
        };
        app.preferences.hitsound_library = None;
        assert!(refused(&mut app).contains("hitsound library"));
        app.preferences.hitsound_library = Some(f.scratch.path("gone"));
        assert_eq!(refused(&mut app), format!("Hitsound library folder not found: {}", f.scratch.path("gone").display()));
        app.preferences.hitsound_library = Some(f.scratch.path("library"));
        app.busy = true;
        assert!(refused(&mut app).contains("Finish"));
        app.busy = false;
        let composer = &mut app.document.as_mut().unwrap().composer;
        let mut keys: Vec<_> = composer.selected.items().iter().cloned().collect();
        composer.apply("Clap at 1500", |h| h.add_hit(dawsu_hitsounding::MomentId(1), skin_lane("soft-hitclap")));
        keys.push(dawsu_editor_composer::HitKey { moment: 1, lane: skin_lane("soft-hitclap") });
        composer.selected.replace(keys);
        assert!(refused(&mut app).contains("one time"));
        app.document.as_mut().unwrap().composer.selected.replace([]);
        assert!(refused(&mut app).contains("Select"));
        // A skin sound the default samples don't have.
        app.document.as_mut().unwrap().composer.selected.replace([dawsu_editor_composer::HitKey { moment: 1, lane: skin_lane("soft-hitclap") }]);
        assert!(refused(&mut app).contains("soft-hitclap"));
        // A numbered sound whose file is missing plays the skin's sound: saved as that when the
        // default samples have it, else refused by name.
        std::fs::remove_file(f.map.with_file_name("drum-hitclap2.wav")).unwrap();
        let mut app = super::tests::app(&f);
        app.preferences.default_samples = f.scratch.path("defaults");
        let map = open(&mut app, &f.map);
        let composer = &mut app.document.as_mut().unwrap().composer;
        let clap = composer.hitsounds().moment(dawsu_hitsounding::MomentId(0)).hits().find(|l| crate::lane_name(l) == "drum-hitclap2").unwrap().clone();
        composer.selected.replace([dawsu_editor_composer::HitKey { moment: 0, lane: clap }]);
        match app.save_sound_set_from_selection(&map, "Set") {
            Err(SaveSetError::Refused(why)) => assert_eq!(why, "drum-hitclap2 wasn't found, so it can't go in a sound set."),
            other => panic!("{other:?}"),
        }
        assert!(library_files(&f).is_empty());
        std::fs::write(f.scratch.path("defaults").join("drum-hitclap.wav"), wav(11)).unwrap();
        app.save_sound_set_from_selection(&map, "Set").unwrap();
        let set = saved(&f, "Set");
        assert_eq!(set.members.iter().map(|m| (m.lane.key(), m.audio.clone())).collect::<Vec<_>>(), [("Skin|drum-hitclap".to_string(), wav(11))]);
    }

    #[test]
    fn a_skin_sound_that_is_not_a_wav_file_is_refused() {
        let (f, mut app, map) = selection_fixture("save-set-ogg-skin");
        let defaults = f.scratch.path("defaults");
        std::fs::remove_file(defaults.join("soft-hitwhistle.wav")).unwrap();
        std::fs::write(defaults.join("soft-hitwhistle.ogg"), b"OggS").unwrap();
        match app.save_sound_set_from_selection(&map, "Set") {
            Err(SaveSetError::Refused(why)) => assert!(why.contains("soft-hitwhistle") && why.contains(".wav"), "{why}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_sound_at_index_1_with_no_file_is_the_skins_sound_and_can_be_saved() {
        let f = fixture("save-set-index-1");
        std::fs::write(&f.map, b"osu file format v14
[General]
Mode:0
[TimingPoints]
0,500,4,1,1,70,1,0
[HitObjects]
256,192,1000,1,0,0:0:0:0:
").unwrap();
        let defaults = f.scratch.path("defaults");
        std::fs::create_dir(&defaults).unwrap();
        std::fs::write(defaults.join("normal-hitnormal.wav"), wav(700)).unwrap();
        std::fs::create_dir(f.scratch.path("library")).unwrap();
        let mut app = app(&f);
        app.preferences.default_samples = defaults;
        let map = open(&mut app, &f.map);
        let composer = &mut app.document.as_mut().unwrap().composer;
        let lane = composer.hitsounds().moment(dawsu_hitsounding::MomentId(0)).hits().next().unwrap().clone();
        assert!(matches!(lane, SampleIdentity::Standard { index, .. } if index.get() == 1), "{lane:?}");
        composer.selected.replace([dawsu_editor_composer::HitKey { moment: 0, lane }]);
        app.save_sound_set_from_selection(&map, "Plain").unwrap();
        let set = saved(&f, "Plain");
        assert_eq!(set.members.iter().map(|m| (m.lane.key(), m.original_key.as_str(), m.audio.clone())).collect::<Vec<_>>(),
            [("Skin|normal-hitnormal".to_string(), "Skin|normal-hitnormal", wav(700))]);
    }

    fn write_library_set(f: &Fixture, file: &str, id: &str, name: &str, sounds: usize) {
        let folder = f.scratch.path("library").join(LIBRARY_FOLDER);
        std::fs::create_dir_all(&folder).unwrap();
        let frames: Vec<_> = [("drum-hitnormal.wav", 1), ("drum-hitclap.wav", 2), ("drum-hitfinish.wav", 3)].into_iter().take(sounds).collect();
        std::fs::write(folder.join(file), set_text(name, &frames).replace(SET_ID, id)).unwrap();
    }

    const OTHER_ID: &str = "1c0ffee0-0000-4000-8000-000000000001";
    const THIRD_ID: &str = "1c0ffee0-0000-4000-8000-000000000002";

    #[test]
    fn the_library_lists_this_maps_sets_first_then_the_rest_by_name_and_flags_duplicate_ids() {
        let f = fixture("library-list");
        let mut app = app(&f);
        let map = open(&mut app, &f.map);
        app.preferences.hitsound_library = None;
        assert!(!app.sound_set_library().unwrap().configured);
        app.preferences.hitsound_library = Some(f.scratch.path("library"));
        assert!(app.sound_set_library().unwrap_err().starts_with("Hitsound library folder not found: "), "the library folder is gone");
        std::fs::create_dir(f.scratch.path("library")).unwrap();
        assert_eq!(listed(&app).sets, [], "no Sound sets folder yet is an empty list");
        write_library_set(&f, "zz kick.dawsuset", SET_ID, "Kick and clap", 2);
        write_library_set(&f, "b.dawsuset", OTHER_ID, "Bell", 1);
        write_library_set(&f, "c.dawsuset", THIRD_ID, "Crash", 3);
        write_library_set(&f, "c copy.dawsuset", THIRD_ID, "Crash", 3);
        std::fs::write(f.scratch.path("library").join(LIBRARY_FOLDER).join("broken.dawsuset"), "{}").unwrap();
        app.seek(1000.0);
        listed(&app);
        app.place_library_sound_set(&map, SET_ID).unwrap();
        let library = listed(&app);
        assert_eq!(library.unreadable, ["broken.dawsuset"], "a file that can't be read is listed by name");
        let sets: Vec<_> = library.sets.into_iter().map(|s| (s.name, s.sounds, s.in_map, s.duplicate)).collect();
        assert_eq!(sets, [
            ("Kick and clap".to_string(), 2, true, false),
            ("Bell".to_string(), 1, false, false),
            ("Crash".to_string(), 3, false, true),
        ], "a duplicate id is listed once, with a warning");
        assert_eq!(library_files(&f).len(), 5, "placing from the library copies nothing into it");
        // A duplicate id can't be placed.
        app.seek(2000.0);
        let refused = app.place_library_sound_set(&map, THIRD_ID).unwrap_err();
        assert!(refused.contains("Crash"), "{refused}");
        assert_eq!(app.document().unwrap().composer.hitsounds().placements().len(), 1);
        assert!(app.place_library_sound_set(&map, "1c0ffee0-0000-4000-8000-000000000009").is_err(), "an id no file holds");
        // Browsing to either file, or to an outside copy, is refused too.
        let folder = f.scratch.path("library").join(LIBRARY_FOLDER);
        let outside = f.scratch.path("outside.dawsuset");
        std::fs::copy(folder.join("c.dawsuset"), &outside).unwrap();
        for file in [folder.join("c.dawsuset"), outside] {
            assert!(app.place_sound_set_file(&map, &file).unwrap_err().contains("Two library files contain \"Crash\""), "{file:?}");
        }
        assert_eq!(app.document().unwrap().composer.hitsounds().placements().len(), 1);
    }

    /// The library list once the background reads are done.
    fn listed(app: &Application) -> SoundSetLibrary {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let library = app.sound_set_library().unwrap();
            if !library.reading {
                return library;
            }
            assert!(Instant::now() < deadline, "the library is still being read");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn the_list_reads_no_file_itself_and_reads_a_failed_file_again() {
        let f = fixture("library-background");
        std::fs::create_dir_all(f.scratch.path("library").join(LIBRARY_FOLDER)).unwrap();
        write_library_set(&f, "a.dawsuset", SET_ID, "Kick and clap", 2);
        let broken = f.scratch.path("library").join(LIBRARY_FOLDER).join("later.dawsuset");
        // Broken and fixed contents of the same size, written with the same modified time.
        let fixed = set_text("Later", &[("drum-hitclap.wav", 9)]).replace(SET_ID, OTHER_ID);
        std::fs::write(&broken, format!("{{{}", " ".repeat(fixed.len() - 1))).unwrap();
        let modified = std::fs::metadata(&broken).unwrap().modified().unwrap();
        let mut app = app(&f);
        open(&mut app, &f.map);
        let first = app.sound_set_library().unwrap();
        assert!(first.reading && first.sets.is_empty(), "a cold list returns at once and reads in the background: {first:?}");
        let library = listed(&app);
        assert_eq!((library.sets.len(), library.unreadable.clone()), (1, vec!["later.dawsuset".to_string()]));
        // The broken file is fixed with the same time and size: it is read again, not remembered.
        std::fs::write(&broken, &fixed).unwrap();
        assert_eq!(std::fs::metadata(&broken).unwrap().len() as usize, fixed.len());
        std::fs::File::options().write(true).open(&broken).unwrap().set_modified(modified).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while listed(&app).sets.len() < 2 {
            assert!(Instant::now() < deadline, "the fixed file is read again");
            std::thread::sleep(Duration::from_millis(5));
        }
        // A deleted file leaves the index.
        std::fs::remove_file(&broken).unwrap();
        assert_eq!(listed(&app).sets.len(), 1);
        assert!(with_index(|index| !index.read.contains_key(&broken)));
    }

    #[test]
    fn only_a_hard_link_the_drive_cant_make_asks_for_another_drive() {
        let io = |step: &'static str, source: std::io::Error| dawsu_foundation::Error::Io { step, path: PathBuf::from("Kick.dawsuset"), source };
        let drive = "This drive can't hold the library. Choose a library on an NTFS drive.";
        let plain = "Could not save Kick to the library. Check that another program is not using it.";
        assert_eq!(save_failure("Kick", &io("creating the file", std::io::Error::from(std::io::ErrorKind::Unsupported))), drive);
        let invalid_function = save_failure("Kick", &io("creating the file", std::io::Error::from_raw_os_error(1)));
        assert_eq!(invalid_function, if cfg!(windows) { drive } else { plain }, "ERROR_INVALID_FUNCTION is Windows' code");
        for source in [
            std::io::Error::from(std::io::ErrorKind::PermissionDenied),
            std::io::Error::from_raw_os_error(5),
            std::io::Error::from_raw_os_error(112),
            std::io::Error::from(std::io::ErrorKind::AlreadyExists),
        ] {
            assert_eq!(save_failure("Kick", &io("creating the file", source)), plain);
        }
        assert_eq!(save_failure("Kick", &io("writing the temporary file", std::io::Error::from(std::io::ErrorKind::Unsupported))), plain, "only the link step");
    }

    #[test]
    fn a_library_file_name_is_the_old_apps_label_and_id() {
        let id = "1c0ffee0-0000-4000-8000-000000000001";
        for (name, file) in [
            ("CON", "CON-"), ("com1", "com1-"), ("COM\u{b9}", "COM\u{b9}-"), ("Kick.", "Kick.-"), ("Kick  ", "Kick  -"),
            ("a/b:c*d?", "a_b_c_d_-"),
        ] {
            assert_eq!(set_file_name(name, id), format!("{file}{id}.dawsuset"), "{name}");
        }
        let long_a = format!("{}a", "x".repeat(80));
        let long_b = format!("{}b", "x".repeat(80));
        let other = "1c0ffee0-0000-4000-8000-000000000002";
        assert_eq!(set_file_name(&long_a, id), format!("{}-{id}.dawsuset", "x".repeat(80)), "cut to 80 characters");
        assert_ne!(set_file_name(&long_a, id), set_file_name(&long_b, other), "two sets sharing the first 80 characters get their own files");
    }

    #[test]
    fn a_sound_selected_twice_at_one_time_goes_in_once_or_is_refused_at_two_volumes() {
        let (f, _, _) = selection_fixture("save-set-twice");
        std::fs::write(&f.map, b"osu file format v14\n[General]\nMode:0\n[TimingPoints]\n0,500,4,1,0,70,1,0\n[HitObjects]\n256,192,1000,1,8,3:0:2:50:\n100,100,1000,1,8,3:0:2:50:\n").unwrap();
        let mut app = super::tests::app(&f);
        let map = open(&mut app, &f.map);
        assert_eq!(app.document().unwrap().composer.hitsounds().moment(dawsu_hitsounding::MomentId(1)).time().ms(), 1000.0);
        let composer = &mut app.document.as_mut().unwrap().composer;
        let keys: Vec<_> = [0, 1].into_iter().flat_map(|m| composer.hitsounds().moment(dawsu_hitsounding::MomentId(m)).hits().map(move |lane| dawsu_editor_composer::HitKey { moment: m, lane: lane.clone() }).collect::<Vec<_>>()).collect();
        assert_eq!(keys.len(), 4);
        composer.selected.replace(keys);
        app.save_sound_set_from_selection(&map, "Twice").unwrap();
        let file = library_files(&f).into_iter().find(|n| n.starts_with("Twice-")).unwrap();
        let set = read_dawsuset(&std::fs::read_to_string(f.scratch.path("library").join(LIBRARY_FOLDER).join(file)).unwrap()).unwrap();
        assert_eq!(set.members.len(), 2, "each sound once");
        let composer = &mut app.document.as_mut().unwrap().composer;
        let clap = composer.hitsounds().moment(dawsu_hitsounding::MomentId(1)).hits().find(|l| crate::lane_name(l) == "drum-hitclap2").unwrap().clone();
        composer.apply("Quieter", |h| h.set_hit_volume(dawsu_hitsounding::MomentId(1), &clap, 20));
        match app.save_sound_set_from_selection(&map, "Twice again") {
            Err(SaveSetError::Refused(why)) => assert_eq!(why, "drum-hitclap2 is selected twice at this time."),
            other => panic!("{other:?}"),
        }
    }

    // ------------------------------------------------------------ links: mixes, kept links, left-out moments

    fn set_with_volumes(name: &str, members: &[(&str, i16, i32)]) -> String {
        let members = members.iter().enumerate().map(|(i, (lane, frame, volume))| SetMember {
            id: format!("aaaaaaaa-0000-4000-8000-{i:012}"),
            original_key: format!("BeatmapFile|{lane}"),
            lane: SetLane { source: SetSource::File, name: lane.to_string(), missing: false, kept: Kept::default() },
            slot: lane.trim_end_matches(".wav").to_string(),
            volume: *volume,
            audio: wav(*frame),
            kept: Kept::default(),
        }).collect();
        write_dawsuset(&SoundSet { id: SET_ID.into(), name: name.into(), members, library_id: None, kept: Kept::default() }).unwrap()
    }

    fn sounds_at(app: &Application, moment: usize) -> Vec<(String, i32)> {
        let m = app.document().unwrap().composer.hitsounds().moment(dawsu_hitsounding::MomentId(moment));
        m.hits().map(|l| (crate::lane_name(l), m.hit_volume(l))).collect()
    }

    #[test]
    fn a_set_saved_as_a_mix_reopens_with_its_sounds_and_a_link_whose_hits_changed_is_kept_until_they_come_back() {
        let f = fixture("links-mixed");
        std::fs::write(&f.set, set_with_volumes("Kick and clap", &[("drum-hitnormal.wav", 8000, 60), ("drum-hitclap.wav", 4000, 30)])).unwrap();
        let mut app = app(&f);
        let map = open(&mut app, &f.map);
        app.seek(1000.0);
        app.place_sound_set_file(&map, &f.set).unwrap();
        save(&mut app, &f);
        assert_eq!(mapset_files(&f).len(), 4, "the two copies and one rendered mix: {:?}", mapset_files(&f));
        // Saving again with nothing changed writes no project record and keeps no new backup.
        let (project, backups) = (project_bytes(&f), walk(&f.scratch.path("projects")).len());
        save(&mut app, &f);
        assert_eq!((project_bytes(&f), walk(&f.scratch.path("projects")).len()), (project, backups));
        let reopened = reopen(&f);
        assert_eq!(links(&reopened), [(0, SET_ID.to_string())]);
        assert_eq!(sounds_at(&reopened, 0), [("drum-hitnormal2".to_string(), 60), ("drum-hitclap2".to_string(), 30)]);
        // A copied sample changed: gain recovery can't put the sounds back, so neither can the link.
        let clap = f.map.with_file_name("drum-hitclap2.wav");
        std::fs::write(&clap, wav(1)).unwrap();
        let mut changed = reopen(&f);
        assert!(links(&changed).is_empty());
        assert!(changed.warnings.iter().any(|w| w.contains("Link not restored")), "{:?}", changed.warnings);
        // The next Save keeps the link it could not restore (M3).
        save(&mut changed, &f);
        let record = serde_json::to_value(changed.project.as_ref().unwrap().settings.sound_sets.as_ref().unwrap()).unwrap();
        assert_eq!(record["current"]["placements"].as_array().map(Vec::len), Some(1), "{record}");
        assert!(reopen(&f).warnings.iter().any(|w| w.contains("Link not restored")), "still named on the next open");
    }

    #[test]
    fn a_set_written_natively_saves_no_gain_recovery() {
        let f = fixture("links-native-no-recovery");
        // At the map's own volume, so no hit has a volume of its own either.
        std::fs::write(&f.set, set_text_at("Kick and clap", &[("drum-hitnormal.wav", 8000), ("drum-hitclap.wav", 4000)], 70)).unwrap();
        let mut app = app(&f);
        let map = open(&mut app, &f.map);
        app.seek(1000.0);
        app.place_sound_set_file(&map, &f.set).unwrap();
        save(&mut app, &f);
        let bytes = std::fs::read(&f.map).unwrap();
        let settings = &app.project.as_ref().unwrap().settings;
        assert!(!settings.gain_recovery.as_ref().is_some_and(|r| r.has_authored_for(&bytes)), "nothing to recover");
        assert!(settings.sound_sets.as_ref().is_some_and(|l| !l.is_empty()));
    }

    #[test]
    fn without_a_project_a_loose_placement_is_refused_by_name() {
        let f = fixture("links-no-project-loose");
        let mut app = app(&f);
        let map = open(&mut app, &f.map);
        app.seek(1500.0);
        app.place_sound_set_file(&map, &f.set).unwrap();
        app.project = None;
        app.save_with_backups(f.scratch.path("backups"));
        wait(&mut app);
        assert!(app.error.as_deref().is_some_and(|e| e.contains("Sound set links need a writable editor project")), "{:?}", app.error);
        assert_eq!(std::fs::read(&f.map).unwrap(), MAP);
    }

    const SLIDER_MAP: &[u8] = b"osu file format v14\r\n\r\n[General]\r\nSampleSet: Normal\r\n\r\n[Difficulty]\r\nSliderMultiplier:1.4\r\nSliderTickRate:1\r\n\r\n[TimingPoints]\r\n0,500,4,1,0,70,1,0\r\n\r\n[HitObjects]\r\n256,192,1000,1,0,0:0:0:0:\r\n100,100,2000,2,0,L|200:100,1,140\r\n";

    /// HS mode; a set placed on a slider tail that also has a soft clap, which no tail can hold
    /// beside the set's drum sounds, so Save leaves the tail out (F052).
    fn left_out_placement(name: &str) -> (Fixture, Application, Vec<(String, i32)>) {
        let f = fixture(name);
        std::fs::write(&f.map, SLIDER_MAP).unwrap();
        let mut app = app(&f);
        let map = open(&mut app, &f.map);
        app.set_hs_mode(true);
        let tail = app.document().unwrap().composer.hitsounds().moments().iter()
            .position(|m| matches!(m.place(), dawsu_hitsounding::Place::SliderNode { node: 1, .. })).unwrap();
        let time = app.document().unwrap().composer.hitsounds().moment(dawsu_hitsounding::MomentId(tail)).time().ms();
        app.seek(time);
        app.place_sound_set_file(&map, &f.set).unwrap();
        assert_eq!(app.document().unwrap().composer.hitsounds().placements()[0].moment.0, tail);
        app.document.as_mut().unwrap().composer.apply("Soft clap", |h| {
            h.add_hit(dawsu_hitsounding::MomentId(tail), dawsu_hitsounding::rules::read_sample_file_name("soft-hitclap.wav").map(|l| match l {
                SampleIdentity::Standard { set, sound, .. } => SampleIdentity::Standard { set, sound, index: dawsu_foundation::CustomIndex::new(0) },
                other => other,
            }).unwrap());
        });
        let drawn = sounds_at(&app, tail);
        save(&mut app, &f);
        assert!(!app.document().unwrap().left_out.is_empty(), "the tail is left out");
        (f, app, drawn)
    }

    #[test]
    fn a_placement_on_a_moment_save_leaves_out_comes_back_with_the_moment() {
        let (f, app, drawn) = left_out_placement("links-left-out");
        let tail = app.document().unwrap().composer.hitsounds().placements()[0].moment.0;
        let reopened = reopen(&f);
        assert_eq!(sounds_at(&reopened, tail), drawn);
        assert_eq!(links(&reopened), [(tail, SET_ID.to_string())], "{:?}", reopened.warnings);
    }

    #[test]
    fn a_recovery_copy_after_a_save_that_left_the_placement_out_restores_the_drawn_link() {
        let (f, mut app, drawn) = left_out_placement("links-left-out-recovery");
        let tail = app.document().unwrap().composer.hitsounds().placements()[0].moment.0;
        app.document.as_mut().unwrap().composer.apply("Later edit", |h| h.set_volume(dawsu_hitsounding::MomentId(0), 33));
        app.poll();
        let offer = app.write_recovery_copy().unwrap().unwrap();
        let copy: serde_json::Value = serde_json::from_slice(&std::fs::read(crate::recovery::pending_path(&f.scratch.path("data"), &offer.id).unwrap()).unwrap()).unwrap();
        assert!(copy["checkpoint"]["last_save"]["drawn"]["placements"].as_array().is_some_and(|p| p.len() == 1), "the drawn hits carry the link");
        let mut restored = Application::with_data_root(f.scratch.path("data"));
        restored.error = None;
        restored.audio = None;
        restored.project_root = app.project_root.clone();
        restored.restore_recovery(&offer.id).unwrap();
        wait(&mut restored);
        assert!(restored.error.is_none(), "{:?}", restored.error);
        restored.command("undo");
        // A recovery copy keeps no volumes of single hits, so only the sounds are compared.
        let names = |sounds: Vec<(String, i32)>| sounds.into_iter().map(|s| s.0).collect::<Vec<_>>();
        assert_eq!(names(sounds_at(&restored, tail)), names(drawn), "undo returns to the hits as drawn at that Save");
        assert_eq!(links(&restored), [(tail, SET_ID.to_string())]);
    }

    #[test]
    fn a_failed_record_write_names_every_record_it_held() {
        assert_eq!(crate::unfinished_record(true, false, false), "Gain recovery could not be finalized.");
        assert_eq!(crate::unfinished_record(false, true, false), "Left-out moments could not be recorded.");
        assert_eq!(crate::unfinished_record(true, true, false), "Gain recovery and left-out moments could not be recorded.");
        assert_eq!(crate::unfinished_record(false, false, true), "Sound set links could not be recorded.");
        assert_eq!(crate::unfinished_record(true, true, true), "Gain recovery, left-out moments and sound set links could not be recorded.");
    }

    #[test]
    fn a_set_mixed_at_one_volume_reopens_with_its_sounds_and_link() {
        let f = fixture("links-mixed-one-volume");
        // Claps from two banks at the map's volume: one object can't play both, so Save mixes them.
        std::fs::write(&f.set, set_text_at("Claps", &[("drum-hitclap.wav", 4000), ("soft-hitclap.wav", 3000)], 70)).unwrap();
        let mut app = app(&f);
        let map = open(&mut app, &f.map);
        app.seek(1000.0);
        app.place_sound_set_file(&map, &f.set).unwrap();
        let placed = sounds_at(&app, 0);
        assert!(!app.document().unwrap().composer.hitsounds().moment(dawsu_hitsounding::MomentId(0)).has_hit_volume_overrides());
        let before = mapset_files(&f);
        save(&mut app, &f);
        assert_eq!(mapset_files(&f).len(), before.len() + 1, "one rendered mix: {:?}", mapset_files(&f));
        let reopened = reopen(&f);
        assert_eq!(sounds_at(&reopened, 0), placed, "{:?}", reopened.warnings);
        assert_eq!(links(&reopened), [(0, SET_ID.to_string())]);
    }

    #[test]
    fn a_kept_link_never_lands_on_another_object_that_now_has_its_ordinal() {
        let f = fixture("links-kept-ordinal");
        let mut app = app(&f);
        let map = open(&mut app, &f.map);
        app.seek(1000.0);
        app.place_sound_set_file(&map, &f.set).unwrap();
        save(&mut app, &f);
        // Another program puts a circle playing the set's sounds before it.
        let text = String::from_utf8(std::fs::read(&f.map).unwrap()).unwrap();
        let edited = text.replacen("[HitObjects]
", "[HitObjects]
256,192,500,1,8,3:0:2:60:
", 1);
        assert_ne!(edited, text);
        std::fs::write(&f.map, edited).unwrap();
        let mut changed = reopen(&f);
        assert!(links(&changed).is_empty());
        changed.document.as_mut().unwrap().composer.apply("Edit", |h| h.set_volume(dawsu_hitsounding::MomentId(0), 59));
        save(&mut changed, &f);
        let again = reopen(&f);
        assert!(links(&again).iter().all(|(moment, _)| again.document().unwrap().composer.hitsounds().moment(dawsu_hitsounding::MomentId(*moment)).time().ms() == 1000.0), "{:?}", links(&again));
    }

    // ------------------------------------------------------------ step 3: update, keep, gain, reset, unlink

    fn library_set(f: &Fixture) -> PathBuf {
        f.scratch.path("library").join(LIBRARY_FOLDER).join("Kick and clap.dawsuset")
    }

    /// Places the fixture's set on the circle, saves, and changes the library's copy to `text`.
    fn placed_then_changed(name: &str, text: Option<String>) -> Fixture {
        let f = fixture(name);
        let mut app = app(&f);
        let map = open(&mut app, &f.map);
        app.seek(1000.0);
        app.place_sound_set_file(&map, &f.set).unwrap();
        save(&mut app, &f);
        if let Some(text) = text {
            std::fs::write(library_set(&f), text).unwrap();
        }
        f
    }

    fn louder() -> String {
        set_text_at("Kick and clap", &[("drum-hitnormal.wav", 8000), ("drum-hitclap.wav", 4000)], 80)
    }

    /// Waits for the open-time check, which waits for the library index's background read.
    fn settle(app: &mut Application) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.sound_set_check_pending {
            app.poll();
            assert!(Instant::now() < deadline, "the library is still being read");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn prompt(app: &mut Application) -> Option<(String, usize)> {
        settle(app);
        app.sound_set_prompts().first().map(|p| (p.name.clone(), p.placements))
    }

    #[test]
    fn an_older_version_asks_on_open_and_update_re_renders_every_placement_as_one_undo_step() {
        let f = placed_then_changed("update-volumes", Some(louder()));
        let files = mapset_files(&f);
        let mut app = reopen(&f);
        assert_eq!(prompt(&mut app), Some(("Kick and clap".to_string(), 1)));
        assert_eq!(sounds_at(&app, 0).iter().map(|s| s.1).collect::<Vec<_>>(), [60, 60], "nothing changes before Update");
        let at = app.document().unwrap().composer.history.position();
        app.answer_sound_set_prompt(SET_ID, true).unwrap();
        assert!(app.sound_set_prompts().is_empty());
        assert_eq!(sounds_at(&app, 0).iter().map(|s| s.1).collect::<Vec<_>>(), [80, 80]);
        let composer = &app.document().unwrap().composer;
        assert_eq!(composer.history.position(), at + 1);
        assert_eq!(composer.history.steps().last().copied(), Some("Update sound set Kick and clap"));
        assert_eq!(mapset_files(&f), files, "only volumes changed: the snapshot is reused");
        app.command("undo");
        assert_eq!(sounds_at(&app, 0).iter().map(|s| s.1).collect::<Vec<_>>(), [60, 60], "undo takes the update back");
        app.command("redo");
        save(&mut app, &f);
        let mut reopened = reopen(&f);
        assert_eq!(prompt(&mut reopened), None, "the saved placement uses the new version");
        assert_eq!(sounds_at(&reopened, 0).iter().map(|s| s.1).collect::<Vec<_>>(), [80, 80]);
        let digest = sha256_hex(&std::fs::read(library_set(&f)).unwrap());
        assert_eq!(reopened.document().unwrap().composer.hitsounds().placements()[0].set_digest, digest);
    }

    #[test]
    fn keep_then_reopen_asks_again_only_for_a_newer_version() {
        let f = placed_then_changed("update-keep", Some(louder()));
        let (files, osu) = (mapset_files(&f), std::fs::read(&f.map).unwrap());
        let mut app = reopen(&f);
        assert!(prompt(&mut app).is_some());
        app.answer_sound_set_prompt(SET_ID, false).unwrap();
        assert!(app.sound_set_prompts().is_empty());
        assert!(!app.is_dirty(), "Keep is no edit");
        assert_eq!((mapset_files(&f), std::fs::read(&f.map).unwrap()), (files.clone(), osu), "Keep writes no mapset file");
        assert_eq!(sounds_at(&app, 0).iter().map(|s| s.1).collect::<Vec<_>>(), [60, 60]);
        drop(app);
        let mut again = reopen(&f);
        assert_eq!(prompt(&mut again), None, "the kept version doesn't ask again");
        // A Save that rewrites the difficulty keeps the choice too.
        let osu = std::fs::read(&f.map).unwrap();
        again.document.as_mut().unwrap().composer.apply("Edit", |h| h.set_volume(dawsu_hitsounding::MomentId(0), 59));
        save(&mut again, &f);
        assert_ne!(std::fs::read(&f.map).unwrap(), osu);
        assert_eq!(prompt(&mut reopen(&f)), None);
        std::fs::write(library_set(&f), set_text_at("Kick and clap", &[("drum-hitnormal.wav", 8000), ("drum-hitclap.wav", 4000)], 90)).unwrap();
        assert_eq!(prompt(&mut reopen(&f)), Some(("Kick and clap".to_string(), 1)), "a newer version asks again");
    }

    #[test]
    fn changed_audio_gets_a_new_index_and_no_file_is_overwritten() {
        let f = placed_then_changed("update-audio", Some(set_text("Kick and clap", &[("drum-hitnormal.wav", 9000), ("drum-hitclap.wav", 4000)])));
        let mut app = reopen(&f);
        assert_eq!(prompt(&mut app), Some(("Kick and clap".to_string(), 1)));
        assert_eq!(mapset_files(&f), ["a.osu", "drum-hitclap2.wav", "drum-hitnormal2.wav"], "nothing is written before Update");
        app.answer_sound_set_prompt(SET_ID, true).unwrap();
        assert_eq!(mapset_files(&f), ["a.osu", "drum-hitclap2.wav", "drum-hitclap3.wav", "drum-hitnormal2.wav", "drum-hitnormal3.wav"], "the whole set at a new index");
        assert_eq!(std::fs::read(f.map.with_file_name("drum-hitnormal2.wav")).unwrap(), wav(8000), "the old file stays as it was");
        assert_eq!(std::fs::read(f.map.with_file_name("drum-hitnormal3.wav")).unwrap(), wav(9000));
        assert_eq!(sounds_at(&app, 0).iter().map(|s| s.0.as_str()).collect::<Vec<_>>(), ["drum-hitnormal3", "drum-hitclap3"], "the hits moved to the new files");
        // Playback finds the new files.
        let sounds = app.scheduled_sounds(app.document().unwrap());
        assert_eq!(sounds.iter().filter(|s| s.at_ms == 1000.0).count(), 2);
        save(&mut app, &f);
        let mut reopened = reopen(&f);
        assert_eq!(prompt(&mut reopened), None);
        assert_eq!(links(&reopened), [(0, SET_ID.to_string())]);
    }

    #[test]
    fn a_set_not_in_the_library_is_frozen_listed_by_id_and_can_still_be_unlinked() {
        let f = placed_then_changed("update-missing", None);
        std::fs::remove_file(library_set(&f)).unwrap();
        let mut app = reopen(&f);
        assert_eq!(prompt(&mut app), None, "a missing set never asks");
        let id = app.document().unwrap().composer.hitsounds().placements()[0].id;
        let view = app.sound_set_placement(id).unwrap();
        assert_eq!((view.name.as_deref(), view.set_id.as_str(), view.frozen.as_deref(), view.edited, view.sounds), (None, SET_ID, Some("Not in library"), 0, 2));
        let version = app.document().unwrap().composer.history.version();
        assert!(app.set_sound_set_placement_gain(id, 50).unwrap_err().contains("Not in library"));
        assert!(app.update_sound_set(SET_ID).is_err());
        assert_eq!(app.document().unwrap().composer.history.version(), version, "frozen: nothing changes");
        let library = listed(&app);
        assert_eq!(library.sets.iter().map(|s| (s.id.as_str(), s.name.as_str(), s.in_map, s.missing)).collect::<Vec<_>>(), [(SET_ID, "", true, true)], "listed in this map by its id, never as its name");
        app.unlink_sound_set_placement(id).unwrap();
        assert!(links(&app).is_empty());
        assert_eq!(sounds_at(&app, 0).len(), 2, "the hits stay");
    }

    #[test]
    fn set_volume_and_reset_edits_each_take_one_undo_step_and_keep_through_save() {
        let f = placed_then_changed("placement-gain", None);
        let mut app = reopen(&f);
        let id = app.document().unwrap().composer.hitsounds().placements()[0].id;
        assert_eq!(app.sound_set_placement(id).unwrap().frozen, None);
        app.set_sound_set_placement_gain(id, 50).unwrap();
        assert_eq!(sounds_at(&app, 0).iter().map(|s| s.1).collect::<Vec<_>>(), [30, 30]);
        let clap = app.document().unwrap().composer.hitsounds().placements()[0].applied[1].lane.clone();
        app.document.as_mut().unwrap().composer.apply("Volume", |h| h.set_hit_volume(dawsu_hitsounding::MomentId(0), &clap, 90));
        let view = app.sound_set_placement(id).unwrap();
        assert_eq!((view.edited, view.sounds, view.gain), (1, 2, 50));
        let at = app.document().unwrap().composer.history.position();
        app.reset_sound_set_placement(id).unwrap();
        assert_eq!(app.document().unwrap().composer.history.position(), at + 1);
        assert_eq!(sounds_at(&app, 0).iter().map(|s| s.1).collect::<Vec<_>>(), [30, 30]);
        save(&mut app, &f);
        let reopened = reopen(&f);
        let p = &reopened.document().unwrap().composer.hitsounds().placements()[0];
        assert_eq!((p.gain, p.applied.iter().map(|a| a.volume).collect::<Vec<_>>()), (50, vec![30, 30]));
    }

    #[test]
    fn a_set_saved_as_a_mix_reopens_unedited_and_updates() {
        let f = fixture("update-mixed");
        std::fs::write(&f.set, set_with_volumes("Kick and clap", &[("drum-hitnormal.wav", 8000, 60), ("drum-hitclap.wav", 4000, 30)])).unwrap();
        let mut app = app(&f);
        let map = open(&mut app, &f.map);
        app.seek(1000.0);
        app.place_sound_set_file(&map, &f.set).unwrap();
        save(&mut app, &f);
        std::fs::write(library_set(&f), set_with_volumes("Kick and clap", &[("drum-hitnormal.wav", 8000, 70), ("drum-hitclap.wav", 4000, 40)])).unwrap();
        let mut app = reopen(&f);
        let id = app.document().unwrap().composer.hitsounds().placements()[0].id;
        assert_eq!(app.sound_set_placement(id).unwrap().edited, 0, "the mix reopens as placed");
        assert_eq!(prompt(&mut app), Some(("Kick and clap".to_string(), 1)));
        app.answer_sound_set_prompt(SET_ID, true).unwrap();
        assert_eq!(sounds_at(&app, 0).iter().map(|s| s.1).collect::<Vec<_>>(), [70, 40]);
    }

    // ------------------------------------------------------------ step 3 review fixes

    fn lane_of(name: &str) -> SampleIdentity {
        dawsu_hitsounding::rules::read_sample_file_name(name).unwrap()
    }

    #[test]
    fn d6b_the_moved_hitnormal_is_recorded_survives_a_recovery_copy_and_moves_with_an_update_to_a_new_index() {
        let f = additions_fixture("d6b-update");
        let mut app = app(&f);
        let map = open(&mut app, &f.map);
        app.seek(1000.0);
        app.place_sound_set_file(&map, &f.set).unwrap();
        let p = app.document().unwrap().composer.hitsounds().placements()[0].clone();
        assert!(p.applied.iter().any(|a| a.member_id == MOMENT_HITNORMAL && a.lane == lane_of("soft-hitnormal2.wav") && a.added), "{:?}", p.applied);
        // A recovery copy carries the recorded hitnormal.
        app.poll();
        let offer = app.write_recovery_copy().unwrap().unwrap();
        let mut restored = Application::create(crate::Preferences::default(), false, None, None, false, f.scratch.path("data"));
        restored.project_root = app.project_root.clone();
        restored.restore_recovery(&offer.id).unwrap();
        wait(&mut restored);
        assert_eq!(restored.document().unwrap().composer.hitsounds().placements()[0].applied, p.applied);
        // New audio in the library: the set and its moved hitnormal go to index 3 together.
        save(&mut app, &f);
        std::fs::write(f.scratch.path("library").join(LIBRARY_FOLDER).join("Clap.dawsuset"), set_text_at("Clap", &[("drum-hitclap.wav", 4100)], 70)).unwrap();
        let mut app = reopen(&f);
        assert_eq!(prompt(&mut app), Some(("Clap".to_string(), 1)));
        let at = app.document().unwrap().composer.history.position();
        app.answer_sound_set_prompt(SET_ID, true).unwrap();
        assert_eq!(app.document().unwrap().composer.history.position(), at + 1, "one undo step");
        assert_eq!(mapset_files(&f), ["a.osu", "drum-hitclap2.wav", "drum-hitclap3.wav", "soft-hitnormal.wav", "soft-hitnormal2.wav", "soft-hitnormal3.wav"]);
        assert_eq!(std::fs::read(f.map.with_file_name("soft-hitnormal3.wav")).unwrap(), wav(500));
        assert_eq!(sounds_at(&app, 0), [("soft-hitnormal3".to_string(), 70), ("drum-hitclap3".to_string(), 70)], "the object still saves as plain hitsounds");
    }

    #[test]
    fn a_volume_below_5_percent_is_refused() {
        let f = placed_then_changed("gain-floor", None);
        let mut app = reopen(&f);
        settle(&mut app);
        let id = app.document().unwrap().composer.hitsounds().placements()[0].id;
        let version = app.document().unwrap().composer.history.version();
        assert_eq!(app.set_sound_set_placement_gain(id, 4).unwrap_err(), "Volume must be at least 5%.");
        assert_eq!(app.document().unwrap().composer.history.version(), version);
        app.set_sound_set_placement_gain(id, 5).unwrap();
        assert_eq!(sounds_at(&app, 0).iter().map(|s| s.1).collect::<Vec<_>>(), [5, 5], "60% of 5 is 3, played at 5");
        assert_eq!(app.status, "Kick and clap set to 5%.");
    }

    #[test]
    fn keep_is_no_edit_and_no_new_recovery_revision() {
        let f = placed_then_changed("keep-quiet", Some(louder()));
        let mut app = reopen(&f);
        assert!(prompt(&mut app).is_some());
        app.poll();
        let version = app.document().unwrap().composer.history.version();
        app.answer_sound_set_prompt(SET_ID, false).unwrap();
        app.poll();
        assert_eq!(app.document().unwrap().composer.history.version(), version, "Keep leaves the document alone");
        assert!(!app.is_dirty());
    }

    #[test]
    fn the_question_stays_until_update_succeeds() {
        let f = placed_then_changed("update-fails", Some(louder()));
        let mut app = reopen(&f);
        assert!(prompt(&mut app).is_some());
        std::fs::write(library_set(&f), "not a set").unwrap();
        assert!(app.answer_sound_set_prompt(SET_ID, true).is_err());
        assert_eq!(app.sound_set_prompts().len(), 1, "still asked");
    }

    #[test]
    fn the_question_names_edited_sounds_that_keep_their_own_files() {
        let f = placed_then_changed("update-edited", None);
        let mut app = reopen(&f);
        let clap = lane_of("drum-hitclap2.wav");
        app.document.as_mut().unwrap().composer.apply("Volume", |h| h.set_hit_volume(dawsu_hitsounding::MomentId(0), &clap, 20));
        save(&mut app, &f);
        std::fs::write(library_set(&f), louder()).unwrap();
        let mut volumes = reopen(&f);
        settle(&mut volumes);
        assert_eq!(volumes.sound_set_prompts()[0].edited_keep_files, 0, "the same files: nothing to say");
        std::fs::write(library_set(&f), set_text("Kick and clap", &[("drum-hitnormal.wav", 9000), ("drum-hitclap.wav", 4000)])).unwrap();
        let mut audio = reopen(&f);
        settle(&mut audio);
        assert_eq!(audio.sound_set_prompts()[0].edited_keep_files, 1);
    }

    #[test]
    fn set_volume_that_copies_the_set_again_moves_every_placement() {
        let f = fixture("gain-moves-all");
        let mut app = app(&f);
        let map = open(&mut app, &f.map);
        app.seek(1000.0);
        app.place_sound_set_file(&map, &f.set).unwrap();
        app.seek(1500.0);
        app.place_sound_set_file(&map, &f.set).unwrap();
        // The snapshot's clap file is changed by another program, so it can't be used.
        std::fs::write(f.map.with_file_name("drum-hitclap2.wav"), wav(1)).unwrap();
        let id = app.document().unwrap().composer.hitsounds().placements()[0].id;
        settle(&mut app);
        let at = app.document().unwrap().composer.history.position();
        app.set_sound_set_placement_gain(id, 50).unwrap();
        assert_eq!(app.document().unwrap().composer.history.position(), at + 1);
        let hits = app.document().unwrap().composer.hitsounds();
        for p in hits.placements() {
            assert!(hits.moment(p.moment).has_hit(&lane_of("drum-hitclap3.wav")), "every placement plays the new files");
        }
        assert_eq!(hits.placements().iter().map(|p| p.gain).collect::<Vec<_>>(), [50, 100]);
    }

    #[test]
    fn a_set_is_called_missing_only_when_it_is() {
        let f = placed_then_changed("not-missing", None);
        let mut app = reopen(&f);
        settle(&mut app);
        let id = app.document().unwrap().composer.hitsounds().placements()[0].id;
        app.preferences.hitsound_library = None;
        assert_eq!(app.sound_set_placement(id).unwrap().frozen.as_deref(), Some("Choose the hitsound library folder."));
        app.preferences.hitsound_library = Some(f.scratch.path("library"));
        std::fs::write(f.scratch.path("library").join(LIBRARY_FOLDER).join("broken.dawsuset"), "{}").unwrap();
        std::fs::remove_file(library_set(&f)).unwrap();
        listed(&app);
        assert_eq!(app.sound_set_placement(id).unwrap().frozen.as_deref(), Some("Could not read broken.dawsuset."));
    }

    // ------------------------------------------------------------ step 3 re-review fixes

    fn clap_library_file(f: &Fixture) -> PathBuf {
        f.scratch.path("library").join(LIBRARY_FOLDER).join("Clap.dawsuset")
    }

    #[test]
    fn an_update_keeps_a_hand_edited_moved_hitnormal_and_says_so() {
        let f = additions_fixture("update-own-hitnormal");
        let mut app = app(&f);
        let map = open(&mut app, &f.map);
        app.seek(1000.0);
        app.place_sound_set_file(&map, &f.set).unwrap();
        let moved = lane_of("soft-hitnormal2.wav");
        app.document.as_mut().unwrap().composer.apply("Volume", |h| h.set_hit_volume(dawsu_hitsounding::MomentId(0), &moved, 40));
        std::fs::write(clap_library_file(&f), set_text_at("Clap", &[("drum-hitnormal.wav", 8000), ("drum-hitclap.wav", 4000)], 70)).unwrap();
        listed(&app);
        app.update_sound_set(SET_ID).unwrap();
        let sounds = sounds_at(&app, 0);
        assert!(sounds.contains(&("soft-hitnormal2".to_string(), 40)), "{sounds:?}");
        assert!(!sounds.iter().any(|s| s.0.starts_with("drum-hitnormal")), "{sounds:?}");
        assert!(app.status.contains("Kept your own hitnormal at 00:01.000."), "{}", app.status);
    }

    #[test]
    fn an_update_gives_moved_hitnormals_with_different_sounds_in_one_bank_their_own_files() {
        let f = additions_fixture("update-two-hitnormals");
        std::fs::write(&f.map, b"osu file format v14\n[General]\nMode:0\n[TimingPoints]\n0,500,4,2,1,70,1,0\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n256,192,1500,1,0,0:0:5:0:\n").unwrap();
        std::fs::write(f.map.with_file_name("soft-hitnormal5.wav"), wav(600)).unwrap();
        let mut app = app(&f);
        let map = open(&mut app, &f.map);
        app.seek(1000.0);
        app.place_sound_set_file(&map, &f.set).unwrap();
        app.seek(1500.0);
        listed(&app);
        app.place_sound_set_file(&map, &f.set).unwrap();
        assert_eq!(app.document().unwrap().composer.hitsounds().placements().len(), 2);
        std::fs::write(clap_library_file(&f), set_text_at("Clap", &[("drum-hitclap.wav", 4100)], 70)).unwrap();
        listed(&app);
        app.update_sound_set(SET_ID).unwrap();
        let hits = app.document().unwrap().composer.hitsounds();
        let normals: Vec<String> = hits.placements().iter().map(|p| {
            let a = p.applied.iter().find(|a| a.member_id == MOMENT_HITNORMAL).unwrap();
            assert!(hits.moment(p.moment).has_hit(&a.lane));
            crate::lane_name(&a.lane)
        }).collect();
        assert_ne!(normals[0], normals[1], "{normals:?}");
        for (name, frame) in normals.iter().zip([500, 600]) {
            assert_eq!(std::fs::read(f.map.with_file_name(format!("{name}.wav"))).unwrap(), wav(frame), "{name}");
        }
    }

    #[test]
    fn saving_a_selection_leaves_silent_sounds_out() {
        let (f, mut app, map) = selection_fixture("save-set-silent");
        let whistle = skin_lane("soft-hitwhistle");
        app.document.as_mut().unwrap().composer.apply("Silence", |h| h.set_hit_volume(dawsu_hitsounding::MomentId(0), &whistle, 0));
        app.save_sound_set_from_selection(&map, "Quiet").unwrap();
        assert_eq!(saved(&f, "Quiet").members.len(), 2);
        assert!(app.status.ends_with("Left out 1 silent sound."), "{}", app.status);
        let composer = &mut app.document.as_mut().unwrap().composer;
        composer.apply("Silence all", |h| h.set_volume(dawsu_hitsounding::MomentId(0), 0));
        assert!(matches!(app.save_sound_set_from_selection(&map, "Nothing"), Err(SaveSetError::Refused(why)) if why.contains("silent")));
    }

    #[test]
    fn the_question_key_changes_on_open_but_not_on_save() {
        let f = fixture("prompt-open-key");
        let mut app = app(&f);
        let map = open(&mut app, &f.map);
        let first = app.sound_set_prompt_open();
        app.seek(1000.0);
        app.place_sound_set_file(&map, &f.set).unwrap();
        save(&mut app, &f);
        assert_eq!(app.sound_set_prompt_open(), first, "Save is no new open");
        open(&mut app, &f.map);
        assert_ne!(app.sound_set_prompt_open(), first);
    }

    #[test]
    fn placing_waits_for_the_library_to_be_read() {
        let f = fixture("place-reading");
        let mut app = app(&f);
        let map = open(&mut app, &f.map);
        let folder = f.scratch.path("library").join(LIBRARY_FOLDER);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::copy(&f.set, folder.join("Kick and clap.dawsuset")).unwrap();
        app.seek(1000.0);
        assert_eq!(app.place_library_sound_set(&map, SET_ID).unwrap_err(), "Still reading the sound set library. Try again.");
        listed(&app);
        app.place_library_sound_set(&map, SET_ID).unwrap();
    }

    #[test]
    fn a_file_with_no_modified_time_has_a_stamp_so_it_is_not_read_again_and_again() {
        let error = || Err(std::io::Error::other("no time"));
        assert!(stamp_of(error(), 10).is_some(), "a stamp the index can match");
        assert_eq!(stamp_of(error(), 10), stamp_of(error(), 10));
        assert_ne!(stamp_of(error(), 10), stamp_of(Ok(std::time::UNIX_EPOCH), 10));
    }

    #[test]
    fn a_second_copy_still_being_read_keeps_the_set_unsure() {
        let f = placed_then_changed("duplicate-reading", None);
        let mut app = reopen(&f);
        settle(&mut app);
        let id = app.document().unwrap().composer.hitsounds().placements()[0].id;
        assert_eq!(app.sound_set_placement(id).unwrap().frozen, None);
        std::fs::copy(library_set(&f), f.scratch.path("library").join(LIBRARY_FOLDER).join("copy.dawsuset")).unwrap();
        assert_eq!(app.sound_set_placement(id).unwrap().frozen.as_deref(), Some("Still reading the sound set library."));
    }

    #[test]
    fn set_volume_that_copies_the_set_again_leaves_placements_kept_on_an_older_version() {
        let f = fixture("gain-leaves-kept");
        let mut app = app(&f);
        let map = open(&mut app, &f.map);
        app.seek(1000.0);
        app.place_sound_set_file(&map, &f.set).unwrap();
        app.seek(1500.0);
        app.place_sound_set_file(&map, &f.set).unwrap();
        let placements = app.document().unwrap().composer.hitsounds().placements().to_vec();
        let kept = placements[1].clone();
        let hits: Vec<_> = kept.applied.iter().map(|a| (a.member_id.clone(), a.lane.clone(), a.volume)).collect();
        app.document.as_mut().unwrap().composer.apply("Older", |h| {
            h.update_placement(kept.id, "an older version", kept.gain, &hits);
        });
        std::fs::write(f.map.with_file_name("drum-hitclap2.wav"), wav(1)).unwrap();
        settle(&mut app);
        app.set_sound_set_placement_gain(placements[0].id, 50).unwrap();
        let hits = app.document().unwrap().composer.hitsounds();
        let older = hits.placements().iter().find(|p| p.id == kept.id).unwrap();
        assert!(hits.moment(older.moment).has_hit(&lane_of("drum-hitclap2.wav")), "the kept placement stays on its files");
        assert_eq!(older.set_digest, "an older version");
    }
}
