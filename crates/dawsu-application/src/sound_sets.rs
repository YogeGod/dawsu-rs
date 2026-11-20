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
}
