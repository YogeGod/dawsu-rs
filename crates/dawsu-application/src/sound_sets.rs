//! Placing a sound set from a `.dawsuset` file (F056-F058, slice A of the sound-sets plan). The set
//! is copied into the hitsound library's "Sound sets" folder first, its samples are copied into the
//! mapset as one snapshot (all files or none), and its hits are placed at the snapped playhead as
//! one undo step, linked to the set.

use super::{Application, layers, safe_child};
use dawsu_audio::AudioClip;
use dawsu_editor::tools::ToolTarget;
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

/// Puts the picked set file in the library's "Sound sets" folder and returns the library copy. A
/// library file with the same set id and the same bytes is reused; one with the same id and other
/// bytes is never overwritten, so placing is refused. A name already taken gets " (2)" and so on.
fn add_to_library(library: &Path, source: &Path, bytes: &[u8], set: &SoundSet) -> Result<PathBuf, String> {
    let folder = library.join(LIBRARY_FOLDER);
    std::fs::create_dir_all(&folder).map_err(|e| format!("Creating the library's {LIBRARY_FOLDER} folder: {e}"))?;
    let folder = folder.canonicalize().map_err(|e| e.to_string())?;
    if source.canonicalize().is_ok_and(|s| s.parent() == Some(folder.as_path())) {
        return Ok(source.to_path_buf());
    }
    for entry in std::fs::read_dir(&folder).map_err(|e| e.to_string())?.flatten() {
        let path = entry.path();
        if !path.extension().is_some_and(|e| e.eq_ignore_ascii_case("dawsuset")) || !entry.file_type().is_ok_and(|t| t.is_file()) {
            continue;
        }
        let Ok((other, found)) = read_set_file(&path) else { continue };
        if found.id == set.id {
            return if other == bytes {
                Ok(path)
            } else {
                Err(format!("The library already has another version of \"{}\" ({}). Place it from there, or remove one of them.", set.name, entry.file_name().to_string_lossy()))
            };
        }
    }
    let stem = source.file_stem().unwrap_or_default().to_string_lossy().into_owned();
    for n in 1..1000 {
        let name = if n == 1 { format!("{stem}.dawsuset") } else { format!("{stem} ({n}).dawsuset") };
        let target = folder.join(&name);
        if target.exists() {
            continue;
        }
        return match safe_write::create(&target, bytes) {
            Ok(()) => Ok(target),
            Err(dawsu_foundation::Error::Io { source, .. }) if source.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => Err(format!("Copying the set into the library: {e}")),
        };
    }
    Err("No free file name in the library's Sound sets folder.".into())
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
        let key = sha256_hex(mapset.to_string_lossy().to_lowercase().as_bytes());
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
    /// Edit > Place sound set: copies the picked `.dawsuset` into the library, copies its samples
    /// into the mapset (or reuses an earlier snapshot of the same version), and places it at the
    /// snapped playhead as one undo step, linked to the set.
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
        add_to_library(&library, file, &bytes, &set)?;
        let set_digest = sha256_hex(&bytes);
        let root = document.path.parent().ok_or("Missing mapset folder")?.to_path_buf();
        let mut record = RecordFile::open(&self.data_root, &root)?;
        let audio = audio_checksum(&set);
        let previous = record.previous(&set, &audio);
        // A name another program takes between planning and writing is planned again once.
        let mut attempt = 0;
        let (snapshot, clips) = loop {
            attempt += 1;
            let files = list_folder(&root).map_err(|e| e.to_string())?;
            let (mut used, _) = layers::reservations(&root, &files)?;
            let document = self.document.as_ref().expect("open document");
            let hits = document.composer.hitsounds();
            used.extend(document.composer.lanes.iter().chain(hits.lanes().iter().map(|l| &l.identity)).filter_map(|lane| match lane {
                SampleIdentity::Standard { index, .. } if index.get() > 1 => Some(index.get()),
                _ => None,
            }));
            used.extend(record.indexes());
            let existing = |name: &str| safe_child(&root, name).ok().and_then(|p| std::fs::read(p).ok());
            let snapshot = plan_snapshot(&set, &files, &used, previous.as_ref(), &existing).map_err(|e| format!("{}: {e}", set.name))?;
            if snapshot.files.is_empty() {
                break (snapshot, Vec::new());
            }
            let entry = SnapshotEntry {
                set_id: set.id.clone(),
                audio: audio.clone(),
                files: snapshot.set.members.iter().map(|m| SnapshotFile { member_id: m.id.clone(), name: m.lane.name.clone(), sha256: sha256_hex(&m.audio) }).collect(),
                done: false,
            };
            let clash = snapshot.files.iter().any(|(name, _)| root.join(name).exists());
            if clash && attempt == 1 {
                continue;
            }
            match write_snapshot(&root, &mut record, entry, &snapshot.files) {
                Ok(clips) => break (snapshot, clips),
                Err(_) if attempt == 1 && snapshot.files.iter().any(|(name, _)| root.join(name).exists()) => continue,
                Err(error) => return Err(error),
            }
        };
        // Reused files may have no decoded audio yet.
        let mut additions = clips;
        for m in &snapshot.set.members {
            let name = &m.lane.name;
            if !additions.iter().any(|(n, _)| n == name) && !self.samples.contains_key(&SampleIdentity::File(name.clone())) {
                let path = safe_child(&root, name)?;
                additions.push((name.clone(), dawsu_audio::decode(&path).map_err(|e| format!("{name}: {e}"))?));
            }
        }
        self.register_added_samples(additions);
        let hits = placement_hits(&snapshot.set, 100).into_iter().zip(&snapshot.set.members).map(|((key, volume), m)| {
            let lane = crate::project::lane_identity(&key).ok_or_else(|| format!("{} is not a sample file name", m.lane.name))?;
            Ok((m.id.clone(), lane, volume))
        }).collect::<Result<Vec<_>, String>>()?;
        let at = self.position_ms();
        let composer = &mut self.document.as_mut().expect("open document").composer;
        let time = composer.snap(SongTime::from_ms(at)).ms();
        if composer.place_sound_set(time, &set.id, &set_digest, 100, &hits, 5.0).is_none() {
            self.status = format!("\"{}\" is already placed here.", set.name);
            return Ok(());
        }
        composer.audition_requests.extend(hits.iter().map(|(_, lane, _)| lane.clone()));
        let copied = if snapshot.files.is_empty() { String::new() } else { format!(", {} files copied", snapshot.files.len()) };
        self.status = format!("Placed \"{}\"{copied}.", set.name);
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
struct PlacementRecord {
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
    circles: &std::collections::BTreeMap<i64, dawsu_osu_format::LineId>, hs: bool,
) -> LinkRecord {
    let ordinals: std::collections::HashMap<_, _> = out.hit_objects().iter().enumerate().map(|(i, o)| (o.line(), i)).collect();
    let placements = drawn.placements().iter().map(|p| {
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
    }).collect();
    LinkRecord { map_digest: sha256_hex(&out.to_bytes()), placements }
}

/// Puts the links of the record for the opened file back, last on open, without an undo step. A
/// placement whose moment or hits can't be found is not linked and named in a warning; the project
/// keeps the record until the next Save.
pub(crate) fn restore_links(document: &mut super::Document, links: &Links) -> Result<(), String> {
    let digest = sha256_hex(document.file.original_bytes());
    let Some(record) = links.for_digest(&digest) else {
        return if links.is_empty() {
            Ok(())
        } else {
            Err("The difficulty changed outside dawsu!, so its sound set links were not restored. The hits stay; the project keeps the links until the next Save.".into())
        };
    };
    let objects = document.file.hit_objects();
    let hits = document.composer.hitsounds();
    let mut restored = Vec::new();
    let mut missing = Vec::new();
    for saved in &record.placements {
        let time = f64::from_bits(saved.time_bits);
        let applied = saved.applied.iter().map(|a| Some(dawsu_hitsounding::sound_sets::Applied {
            member_id: a.member_id.clone(), lane: crate::project::lane_identity(&a.lane)?, volume: a.volume, added: a.added,
        })).collect::<Option<Vec<_>>>();
        let holds = |m: &dawsu_hitsounding::Moment, applied: &[dawsu_hitsounding::sound_sets::Applied]| applied.iter().filter(|a| a.added).all(|a| m.has_hit(&a.lane));
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
            Some(dawsu_hitsounding::sound_sets::SoundSetPlacement {
                id: saved.id, set_id: saved.set_id.clone(), set_digest: saved.set_digest.clone(), accepted_digest: saved.accepted_digest.clone(),
                moment: dawsu_hitsounding::MomentId(moment), gain: saved.gain, applied,
            })
        });
        match found {
            Some(placement) => restored.push(placement),
            None => missing.push(clock(time)),
        }
    }
    document.composer.history.update_external(|hits| hits.restore_placements(restored.clone()));
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!("Link not restored for the sound set placed at {}: its hits changed. The hits stay; the project keeps the link until the next Save.", missing.join(", ")))
    }
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
        let members = frames.iter().enumerate().map(|(i, (lane, frame))| SetMember {
            id: format!("aaaaaaaa-0000-4000-8000-{i:012}"),
            original_key: format!("BeatmapFile|{lane}"),
            lane: SetLane { source: SetSource::File, name: lane.to_string(), missing: false, kept: Kept::default() },
            slot: lane.trim_end_matches(".wav").to_string(),
            volume: 60,
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
        assert_eq!(hits.moment(p.moment).hits().count(), 3, "the circle's own hitnormal stays beside the set's two sounds");
        assert!(app.is_dirty());
        // Playback plays both copied sounds.
        let sounds = app.scheduled_sounds(app.document().unwrap());
        assert_eq!(sounds.iter().filter(|s| s.at_ms == 1000.0).count(), 3);
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
        // Two hitnormals saved as one rendered mix; without its gain recovery the mix is what reopens.
        assert_eq!(hits.moment(dawsu_hitsounding::MomentId(0)).hits().count(), 1, "the written hits stay");
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
    fn gain_recovery_keeps_a_placed_moment_even_when_every_hit_plays_at_one_volume() {
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
        let captured = crate::gain_recovery::Captured::read(hits, map.parent().unwrap(), &defaults).unwrap();
        assert!(captured.needs_recovery());
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
}
