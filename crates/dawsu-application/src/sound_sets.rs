//! Placing a sound set from a `.dawsuset` file (F056-F058, slice A of the sound-sets plan). The set
//! is copied into the hitsound library's "Sound sets" folder first, its samples are copied into the
//! mapset as one snapshot (all files or none), and its hits are placed at the snapped playhead as
//! one undo step, linked to the set.

use super::{Application, layers, safe_child};
use dawsu_audio::AudioClip;
use dawsu_editor::tools::ToolTarget;
use dawsu_editor_composer::SetHitnormal;
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
    let size = std::fs::metadata(path).map_err(|e| format!("Opening {name}: {e}"))?.len();
    if size > MAX_SET_BYTES {
        return Err(format!("{name} is over 96 MB."));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("Reading {name}: {e}"))?;
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
    fn write(&self) -> Result<(), String> {
        match self {
            LibraryCopy::There => Ok(()),
            LibraryCopy::New(target, bytes) => {
                let folder = target.parent().expect("a library file has a folder");
                std::fs::create_dir_all(folder).map_err(|e| format!("Creating the library's {LIBRARY_FOLDER} folder: {e}"))?;
                safe_write::create(target, bytes).map_err(|e| format!("Copying the set into the library: {e}"))
            }
        }
    }
}

/// Each library set file's id, by path, modified time and size, so placing reads a set file only
/// when it is new or changed (M10).
type IdCache = std::collections::HashMap<PathBuf, (std::time::SystemTime, u64, Option<String>)>;
static LIBRARY_IDS: std::sync::Mutex<Option<IdCache>> = std::sync::Mutex::new(None);

fn library_id(path: &Path, metadata: &std::fs::Metadata) -> Option<String> {
    let modified = metadata.modified().ok()?;
    let mut cache = LIBRARY_IDS.lock().unwrap_or_else(|e| e.into_inner());
    let cache = cache.get_or_insert_with(Default::default);
    if let Some((time, size, id)) = cache.get(path)
        && *time == modified && *size == metadata.len()
    {
        return id.clone();
    }
    let id = read_set_file(path).ok().map(|(_, set)| set.id);
    cache.insert(path.to_path_buf(), (modified, metadata.len(), id.clone()));
    id
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
            if library_id(&path, &metadata).as_deref() == Some(set.id.as_str()) {
                return if std::fs::read(&path).is_ok_and(|other| other == bytes) {
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
        let at = self.position_ms();
        let composer = &self.document.as_ref().expect("open document").composer;
        let time = composer.snap(SongTime::from_ms(at)).ms();
        if composer.is_set_placed_at(time, &set.id, 5.0) {
            self.status = format!("\"{}\" is already placed here.", set.name);
            return Ok(());
        }
        let library_copy = library_target(&library, file, &bytes, &set)?;
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
            let (mut used, _) = layers::reservations(&root, &files)?;
            let document = self.document.as_ref().expect("open document");
            let model = document.composer.hitsounds();
            used.extend(document.composer.lanes.iter().chain(model.lanes().iter().map(|l| &l.identity)).filter_map(|lane| match lane {
                SampleIdentity::Standard { index, .. } if index.get() > 1 => Some(index.get()),
                _ => None,
            }));
            used.extend(record.indexes());
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
                library_copy.write()?;
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
            library_copy.write()?;
            match write_snapshot(&root, &mut record, entry, &writes) {
                Ok(clips) => break (snapshot, clips, extra, hits, hitnormal),
                Err(_) if attempt == 1 && writes.iter().any(|(name, _)| root.join(name).exists()) => continue,
                Err(error) => return Err(error),
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
            accepted_digest: p.accepted_digest.clone(),
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
                    hits.moments().iter().position(|m| location(m.place()) == Some((line, saved.node)) && holds(m, &applied))
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
}
