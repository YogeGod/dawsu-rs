//! Sample files dawsu! made that the mapset does not hold yet (pending files). A new layer, a
//! replaced lane sample or a placed sound set keeps its bytes in dawsu!'s data folder, at
//! `pending-samples/<sha256>.<extension>`, and claims its name under
//! `pending-names/<mapset key>/` so no other allocation, here or in another window, takes it.
//! A Save writes a pending file into the mapset only when the written `.osu` plays it.

use dawsu_foundation::{digest::sha256_hex, safe_write};
use dawsu_hitsounding::rules::read_sample_file_name;
use dawsu_foundation::SampleIdentity;
use dawsu_hitsounding::rules::{Answer, Plays, Sample, answer};
use dawsu_hitsounding::{Hitsounds, LaneSource};
use dawsu_osu_format::{OsuFile, list_folder};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PendingFile {
    /// The name the mapset would hold the file under.
    pub name: String,
    pub sha256: String,
    /// Where the bytes are kept until a Save writes them.
    pub path: PathBuf,
}

#[derive(Clone, Debug)]
struct Store {
    samples: PathBuf,
    claims: PathBuf,
    ledger: PathBuf,
}

static NEXT_REVISION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn next_revision() -> u64 {
    NEXT_REVISION.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// The open mapset's pending files. Cheap to clone: a Save job takes a snapshot.
#[derive(Clone, Debug, Default)]
pub(crate) struct Pending {
    files: Arc<BTreeMap<String, PendingFile>>,
    store: Option<Store>,
    revision: u64,
}

/// The key that names a mapset in dawsu!'s data folder, as the sound set snapshot record names it.
pub(crate) fn mapset_key(mapset: &Path) -> String {
    sha256_hex(mapset.to_string_lossy().as_bytes())
}

impl Pending {
    pub(crate) fn for_mapset(data_root: &Path, mapset: &Path) -> Self {
        Self {
            files: Arc::default(),
            store: Some(Store {
                samples: data_root.join("pending-samples"),
                claims: data_root.join("pending-names").join(mapset_key(mapset)),
                ledger: data_root.join("created-samples").join(format!("{}.json", mapset_key(mapset))),
            }),
            revision: next_revision(),
        }
    }

    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    pub(crate) fn get(&self, name: &str) -> Option<&PendingFile> {
        let name = name.replace('\\', "/");
        self.files.get(&name.to_ascii_lowercase())
    }

    /// The ledger of the sample files dawsu! wrote into this mapset.
    pub(crate) fn ledger(&self) -> Option<Ledger> {
        self.store.as_ref().map(|store| Ledger { path: store.ledger.clone() })
    }

    pub(crate) fn is_empty(&self) -> bool {

        self.files.is_empty()
    }

    pub(crate) fn files(&self) -> impl Iterator<Item = &PendingFile> {
        self.files.values()
    }

    /// Every name a new file must not take: this document's pending names and the names any
    /// dawsu! window claimed for this mapset, lowercased.
    pub(crate) fn taken_names(&self) -> BTreeSet<String> {
        let mut names: BTreeSet<String> = self.files.keys().cloned().collect();
        if let Some(store) = &self.store
            && let Ok(entries) = std::fs::read_dir(&store.claims)
        {
            names.extend(entries.filter_map(Result::ok).map(|entry| entry.file_name().to_string_lossy().to_ascii_lowercase()));
        }
        names
    }

    /// The names claimed for this mapset on disk, by this or another window, lowercased. Another
    /// window's claim counts too: once that window saves its file, a difficulty that looks the
    /// name up starts playing it.
    pub(crate) fn claimed_names(&self) -> BTreeSet<String> {
        let Some(store) = &self.store else { return BTreeSet::new() };
        std::fs::read_dir(&store.claims).map(|entries| entries.filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().to_ascii_lowercase()).collect()).unwrap_or_default()
    }

    /// The custom indexes of the taken names.
    pub(crate) fn taken_indexes(&self) -> BTreeSet<u32> {
        self.taken_names().iter().filter_map(|name| match read_sample_file_name(name)? {
            SampleIdentity::Standard { index, .. } => Some(index.get()),
            SampleIdentity::File(_) => None,
        }).collect()
    }

    /// Keeps `bytes` as the pending file `name`. The name is claimed atomically: a name another
    /// window claimed first is refused.
    pub(crate) fn add(&mut self, name: &str, bytes: &[u8]) -> Result<PendingFile, String> {
        bare_name(name)?;
        let store = self.store.as_ref().ok_or("The open map has no data folder for new samples.")?;
        let sha256 = sha256_hex(bytes);
        let extension = Path::new(name).extension().and_then(|e| e.to_str()).unwrap_or("wav").to_ascii_lowercase();
        let path = store.samples.join(format!("{sha256}.{extension}"));
        // The claim comes first: a kept copy is never there without a claim that names it.
        std::fs::create_dir_all(&store.claims).map_err(|e| format!("Could not keep {name}: {e}"))?;
        let claim = store.claims.join(name.to_ascii_lowercase());
        safe_write::create(&claim, sha256.as_bytes())
            .map_err(|_| format!("{name} was just taken by another dawsu! window. Try again."))?;
        let kept = || std::fs::read(&path).is_ok_and(|stored| stored == bytes);
        let _prune_lock = store.samples.parent().and_then(|root| removed_samples_lock(root).ok());
        let stored = std::fs::create_dir_all(&store.samples).map_err(|e| e.to_string()).and_then(|()| {
            if kept() {
                // Kept by an earlier sample with the same bytes: it counts from today.
                touch(&path);
                return Ok(());
            }
            match safe_write::create(&path, bytes) {
                Err(_) if kept() => Ok(()),
                result => result.map_err(|e| e.to_string()),
            }
        });
        if let Err(error) = stored {
            let _ = std::fs::remove_file(&claim);
            return Err(format!("Could not keep {name}: {error}"));
        }
        let file = PendingFile { name: name.to_string(), sha256, path };
        Arc::make_mut(&mut self.files).insert(name.to_ascii_lowercase(), file.clone());
        self.revision = next_revision();
        Ok(file)
    }

    /// Takes back a pending file a project listed, when its kept bytes still match. Its claim is
    /// kept or made again.
    pub(crate) fn restore(&mut self, name: &str, sha256: &str) -> Result<PendingFile, String> {
        bare_name(name)?;
        if sha256.len() != 64 || !sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(format!("The kept copy of {name} has no valid checksum."));
        }
        let store = self.store.as_ref().ok_or("The open map has no data folder for new samples.")?;
        let extension = Path::new(name).extension().and_then(|e| e.to_str()).unwrap_or("wav").to_ascii_lowercase();
        let path = store.samples.join(format!("{sha256}.{extension}"));
        let bytes = std::fs::read(&path).map_err(|_| format!("The kept copy of {name} is missing."))?;
        if !sha256_hex(&bytes).eq_ignore_ascii_case(sha256) {
            return Err(format!("The kept copy of {name} changed."));
        }
        std::fs::create_dir_all(&store.claims).map_err(|e| e.to_string())?;
        let claim = store.claims.join(name.to_ascii_lowercase());
        match std::fs::read(&claim) {
            Ok(owner) if owner == sha256.as_bytes() => {}
            Ok(_) => return Err(format!("{name} is now claimed by another sample.")),
            Err(_) => safe_write::create(&claim, sha256.as_bytes()).map_err(|_| format!("{name} is now claimed by another sample."))?,
        }
        let file = PendingFile { name: name.to_string(), sha256: sha256.to_string(), path };
        Arc::make_mut(&mut self.files).insert(name.to_ascii_lowercase(), file.clone());
        self.revision = next_revision();
        Ok(file)
    }

    /// Takes in a pending file a cleanup job kept, with its claim already made.
    pub(crate) fn adopt(&mut self, file: PendingFile) {
        Arc::make_mut(&mut self.files).insert(file.name.to_ascii_lowercase(), file);
        self.revision = next_revision();
    }

    /// Drops a pending file and its claim only while it holds the bytes `sha256` names.
    pub(crate) fn release_matching(&mut self, name: &str, sha256: &str) {
        if self.get(name).is_some_and(|file| file.sha256.eq_ignore_ascii_case(sha256)) {
            self.release(name);
        }
    }

    /// Drops a pending file and its claim: a Save wrote it into the mapset, or keeping it failed.
    pub(crate) fn release(&mut self, name: &str) {
        if Arc::make_mut(&mut self.files).remove(&name.to_ascii_lowercase()).is_some() {
            self.revision = next_revision();
            if let Some(store) = &self.store {
                let _ = std::fs::remove_file(store.claims.join(name.to_ascii_lowercase()));
            }
        }
    }
}

/// A pending file's name is one file in the mapset folder itself: no folder, drive or `..`.
fn bare_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\', ':']) || name.chars().any(char::is_control) {
        return Err(format!("{name:?} is not a sample file name in the mapset folder."));
    }
    Ok(())
}

/// A sample folder's files (the default samples or a library folder), empty when it can't be read.
#[allow(clippy::disallowed_methods)]
pub(crate) fn sample_folder(dir: &Path) -> Vec<String> {
    list_folder(dir).unwrap_or_default()
}

/// The mapset's files as the editor sees them: the files on disk and the pending files.
#[allow(clippy::disallowed_methods)]
pub(crate) fn listing(root: &Path, pending: &Pending) -> Result<Vec<String>, String> {
    let mut files = list_folder(root).map_err(|error| error.to_string())?;
    let on_disk: BTreeSet<String> = files.iter().map(|name| name.to_ascii_lowercase()).collect();
    files.extend(pending.files().filter(|file| !on_disk.contains(&file.name.to_ascii_lowercase())).map(|file| file.name.clone()));
    files.sort();
    Ok(files)
}

/// A pending file a project still needs after a Save: the drawn hits play it, but the written
/// `.osu` does not (a mix source, a left-out moment, a loose hit). Older builds ignore the list.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct PendingSample {
    pub name: String,
    pub sha256: String,
}

/// Every sample of every object of `file`, in lazer's order.
fn samples(file: &OsuFile, folder: &[String]) -> Vec<Sample> {
    let Answer::Objects(objects) = answer(file, folder) else { return Vec::new() };
    let mut out = Vec::new();
    for object in objects {
        out.extend(object.samples);
        if let Some(slider) = object.slider {
            for part in slider.parts {
                out.extend(part.samples);
            }
            out.extend(slider.tail_samples.into_iter().chain(slider.sliding_samples));
            out.push(slider.whistle_sample);
        }
    }
    out
}

/// The folder files the samples of `file` play.
pub(crate) fn played_names(file: &OsuFile, folder: &[String]) -> BTreeSet<String> {
    plays(file, folder).into_iter().filter_map(|plays| match plays {
        Plays::BeatmapFile(name) => Some(name),
        Plays::Skin { .. } => None,
    }).collect()
}

/// What plays for every sample of every object of `file`, in lazer's order.
fn plays(file: &OsuFile, folder: &[String]) -> Vec<Plays> {
    samples(file, folder).into_iter().map(|sample| sample.plays).collect()
}

/// The pending files the written `file` plays, with their bytes. `planned` are the names the
/// same Save writes besides them (its mixes).
pub(crate) fn used(file: &OsuFile, root: &Path, pending: &Pending, planned: &[String]) -> Result<Vec<(String, Vec<u8>)>, String> {
    if pending.is_empty() {
        return Ok(Vec::new());
    }
    let mut folder = listing(root, pending)?;
    folder.extend(planned.iter().cloned());
    let names: BTreeSet<String> = plays(file, &folder).into_iter().filter_map(|plays| match plays {
        Plays::BeatmapFile(name) => Some(name),
        Plays::Skin { .. } => None,
    }).collect();
    let mut used = Vec::new();
    for name in names {
        let Some(file) = pending.get(&name) else { continue };
        let bytes = std::fs::read(&file.path).map_err(|_| format!("The kept copy of {} is missing. Nothing was saved.", file.name))?;
        if sha256_hex(&bytes) != file.sha256 {
            return Err(format!("The kept copy of {} changed. Nothing was saved.", file.name));
        }
        used.push((file.name.clone(), bytes));
    }
    Ok(used)
}

/// The written-file check (M1): `file` must play the same sounds from the mapset as it will be
/// after the Save (the files on disk plus `writes`) as the editor plays them (`editor`). A
/// difference names the first file the mapset would miss.
/// A sample that plays the skin's sound on both sides while dawsu! claimed one of its file names
/// for a sample it made is refused too: that file was lost.
#[allow(clippy::disallowed_methods)]
pub(crate) fn check_written(file: &OsuFile, root: &Path, writes: &[String], editor: &[String], pending: &Pending) -> Result<(), String> {
    let mut after = list_folder(root).map_err(|e| e.to_string())?;
    after.extend(writes.iter().cloned());
    let written_samples = samples(file, &after);
    let claimed = pending.claimed_names();
    for sample in written_samples.iter().filter(|sample| matches!(sample.plays, Plays::Skin { .. }) && sample.lazer.use_beatmap_samples) {
        for name in sample.lazer.legacy_lookup_names(true) {
            let lost = ["", ".wav", ".mp3", ".ogg"].iter().map(|ext| format!("{name}{ext}"))
                .find(|candidate| claimed.contains(&candidate.to_ascii_lowercase()));
            if let Some(lost) = lost {
                return Err(format!("dawsu! made {lost} for this map, but its sound is missing. Nothing was saved."));
            }
        }
    }
    let (written, drawn) = (written_samples.into_iter().map(|sample| sample.plays).collect::<Vec<_>>(), plays(file, editor));
    match written.iter().zip(&drawn).find(|(written, drawn)| written != drawn) {
        None if written.len() == drawn.len() => Ok(()),
        found => {
            let name = match found.map(|(_, drawn)| drawn) {
                Some(Plays::BeatmapFile(name)) => name.clone(),
                _ => "a sample file".into(),
            };
            Err(format!("Save would leave {name} out of the mapset while the difficulty plays it. Nothing was saved."))
        }
    }
}

/// The refusal for a new sample a Save would leave out of the mapset with no project to keep it.
pub(crate) fn left_out_without_project(name: &str) -> String {
    format!("Save would leave {name} out of the mapset, and without a writable editor project nothing would keep it. Nothing was saved.")
}

/// A new sample file a Save writes must not take a custom index another difficulty of the mapset
/// now uses: that difficulty would start playing it.
#[allow(clippy::disallowed_methods)]
pub(crate) fn check_indexes_free(root: &Path, saved: &Path, writes: &[String]) -> Result<(), String> {
    check_indexes_free_except(root, &[saved], writes)
}

pub(crate) fn check_indexes_free_except(root: &Path, skip: &[&Path], writes: &[String]) -> Result<(), String> {
    let indexes: BTreeMap<u32, &String> = writes.iter().filter_map(|name| match read_sample_file_name(name)? {
        SampleIdentity::Standard { index, .. } => Some((index.get(), name)),
        SampleIdentity::File(_) => None,
    }).collect();
    if indexes.is_empty() {
        return Ok(());
    }
    let skipped: BTreeSet<String> = skip
        .iter()
        .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()))
        .collect();
    for other in list_folder(root).map_err(|e| e.to_string())? {
        if !Path::new(&other).extension().is_some_and(|e| e.eq_ignore_ascii_case("osu"))
            || skipped.contains(&other.to_ascii_lowercase())
        {
            continue;
        }
        let (used, _) = crate::layers::reservations(root, std::slice::from_ref(&other))?;
        if let Some((index, name)) = indexes.iter().find(|(index, _)| used.contains(index)) {
            return Err(format!("{name} uses sample index {index}, which {other} now uses too. Nothing was saved."));
        }
    }
    Ok(())
}

/// The files the drawn hits play, lowercased: those of moments (sound set placements included)
/// and slider bodies. A lane no hit uses plays nothing.
pub(crate) fn hit_files(hits: &Hitsounds) -> BTreeSet<String> {
    let lanes = hits.moments().iter().flat_map(|moment| moment.hits().cloned())
        .chain(hits.bodies().iter().flat_map(|body| [body.slide().clone(), body.whistle_lane(), body.tick().clone()]));
    lanes.filter_map(|lane| match hits.lane(&lane).map(|lane| &lane.source) {
        Some(LaneSource::File(name)) => Some(name.to_ascii_lowercase()),
        _ => None,
    }).collect()
}

/// The pending files the drawn hits play that this Save does not write.
pub(crate) fn needed(hits: &Hitsounds, pending: &Pending, written: &[String]) -> Vec<PendingSample> {
    let written: BTreeSet<String> = written.iter().map(|n| n.to_ascii_lowercase()).collect();
    let mut needed = BTreeMap::new();
    for name in hit_files(hits) {
        if let Some(file) = pending.get(&name)
            && !written.contains(&file.name.to_ascii_lowercase())
        {
            needed.insert(file.name.to_ascii_lowercase(), PendingSample { name: file.name.clone(), sha256: file.sha256.clone() });
        }
    }
    needed.into_values().collect()
}

impl Pending {
    /// Releases the claims for this mapset that no record keeps (`keep`, lowercased names) and
    /// this document does not hold: those a closed or crashed session left behind.
    pub(crate) fn release_stale(&self, keep: &BTreeSet<String>) {
        let Some(store) = &self.store else { return };
        let Ok(entries) = std::fs::read_dir(&store.claims) else { return };
        for entry in entries.filter_map(Result::ok) {
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            if !keep.contains(&name) && !self.files.contains_key(&name) && entry.path().is_file() {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }

    /// Forgets every pending file `keep` (lowercased names) does not list, releasing its claim.
    pub(crate) fn forget_all_but(&mut self, keep: &BTreeSet<String>) {
        let names: Vec<String> = self.files.keys().filter(|name| !keep.contains(*name)).cloned().collect();
        for name in names {
            self.release(&name);
        }
    }
}

/// An open session on a mapset, held as a locked file under `data_root/sessions/<mapset key>/`.
/// The lock goes when the session ends, a crash included.
#[derive(Debug)]
pub(crate) struct Presence {
    file: Option<std::fs::File>,
    path: PathBuf,
}

const MAKING: &str = ".tmp";

static NEXT_PRESENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

impl Presence {
    pub(crate) fn enter(data_root: &Path, mapset: &Path) -> Result<Self, String> {
        let folder = data_root.join("sessions").join(mapset_key(mapset));
        std::fs::create_dir_all(&folder).map_err(|e| format!("{}: {e}", folder.display()))?;
        let n = NEXT_PRESENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = folder.join(format!("{}-{n}.lock", std::process::id()));
        // Made and locked under a temporary name, then renamed: another window never finds an
        // unlocked lock file of a live session and removes it as ended.
        let making = path.with_extension(format!("lock{MAKING}"));
        let file = std::fs::File::create(&making).map_err(|e| format!("{}: {e}", making.display()))?;
        let locked = file.try_lock().map_err(|e| format!("Locking {}: {e}", making.display()))
            .and_then(|()| safe_write::rename_waiting(&making, &path).map_err(|e| e.to_string()));
        if let Err(error) = locked {
            drop(file);
            let _ = std::fs::remove_file(&making);
            return Err(error);
        }
        Ok(Self { file: Some(file), path })
    }

    /// Whether another session holds this mapset. Lock files of ended sessions are removed. One
    /// still being made counts as live unless it is a minute old, when it is removed.
    pub(crate) fn others_live(&self) -> bool {
        let Some(folder) = self.path.parent() else { return false };
        let Ok(entries) = std::fs::read_dir(folder) else { return false };
        let mut live = false;
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if path == self.path {
                continue;
            }
            if path.to_string_lossy().ends_with(MAKING) {
                let old = entry.metadata().and_then(|m| m.modified()).ok()
                    .and_then(|modified| modified.elapsed().ok())
                    .is_some_and(|age| age > std::time::Duration::from_secs(60));
                if old {
                    let _ = std::fs::remove_file(&path);
                } else {
                    live = true;
                }
                continue;
            }
            let Ok(file) = std::fs::OpenOptions::new().read(true).write(true).open(&path) else { continue };
            match file.try_lock() {
                Ok(()) => {
                    drop(file);
                    let _ = std::fs::remove_file(&path);
                }
                Err(_) => live = true,
            }
        }
        live
    }
}

impl Drop for Presence {
    fn drop(&mut self) {
        drop(self.file.take());
        let _ = std::fs::remove_file(&self.path);
    }
}

/// The sample files dawsu! wrote into one mapset, by name and checksum, kept at
/// `data_root/created-samples/<mapset key>.json`. Only files on this list are ever cleaned up.
#[derive(Clone, Debug)]
pub(crate) struct Ledger {
    path: PathBuf,
}

#[derive(Default, serde::Serialize, serde::Deserialize)]
struct LedgerFile {
    files: Vec<PendingSample>,
}

impl Ledger {
    /// The listed files, read under the ledger's lock. A ledger that isn't there lists nothing;
    /// one that can't be read is an error. A corrupt one is kept aside (see `read`).
    pub(crate) fn entries(&self) -> Result<Vec<PendingSample>, String> {
        self.locked(|| self.read())
    }

    /// Reads the ledger; the caller holds the lock. A corrupt ledger is kept aside, next to it
    /// with `corrupt` in its name and never over an earlier aside, and a new one starts. When no
    /// aside copy could be kept, reading fails and the ledger stays as it is.
    fn read(&self) -> Result<Vec<PendingSample>, String> {
        let bytes = match std::fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(format!("Reading the list of sample files dawsu! made ({}): {error}", self.path.display())),
        };
        if let Ok(file) = serde_json::from_slice::<LedgerFile>(&bytes) {
            return Ok(file.files);
        }
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis());
        let kept = (0..1000).find_map(|k| {
            let aside = self.path.with_extension(format!("corrupt-{stamp}-{}-{k}.json", std::process::id()));
            if aside.exists() {
                return None;
            }
            // A link where the file system has them, a copy where it doesn't.
            let linked = std::fs::hard_link(&self.path, &aside).is_ok()
                || safe_write::create(&aside, &bytes).is_ok();
            linked.then_some(aside)
        });
        let Some(aside) = kept else {
            return Err(format!("The list of sample files dawsu! made ({}) is damaged and could not be kept aside.", self.path.display()));
        };
        if std::fs::read(&aside).ok().as_deref() != Some(bytes.as_slice()) {
            return Err(format!("The damaged list of sample files dawsu! made could not be kept aside as {}.", aside.display()));
        }
        std::fs::remove_file(&self.path).map_err(|e| format!("Starting a new list of sample files dawsu! made: {e}"))?;
        Ok(Vec::new())
    }

    /// Writes the ledger like any file only dawsu! writes: temporary file, then rename, with the
    /// same waits as other replaces when another program holds the file for a moment.
    fn write(&self, files: Vec<PendingSample>) -> Result<(), String> {
        let bytes = serde_json::to_vec_pretty(&LedgerFile { files }).map_err(|e| e.to_string())?;
        safe_write::write_new(&self.path, &bytes).map_err(|e| format!("Recording new sample files: {e}"))
    }

    /// Runs `change` while holding the ledger's lock file, so two windows never drop each
    /// other's entries.
    fn locked<T>(&self, change: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
        let folder = self.path.parent().ok_or("The ledger has no folder.")?;
        std::fs::create_dir_all(folder).map_err(|e| format!("Recording new sample files: {e}"))?;
        let lock_path = self.path.with_extension("lock");
        let lock = std::fs::OpenOptions::new().create(true).truncate(false).read(true).write(true)
            .open(&lock_path).map_err(|e| format!("Opening {}: {e}", lock_path.display()))?;
        lock.lock().map_err(|e| format!("Locking {}: {e}", lock_path.display()))?;
        let result = change();
        let _ = lock.unlock();
        result
    }

    /// Changes the entries in one locked read-modify-write.
    pub(crate) fn update(&self, change: impl FnOnce(&mut Vec<PendingSample>)) -> Result<(), String> {
        self.locked(|| {
            let mut entries = self.read()?;
            let before = entries.clone();
            change(&mut entries);
            if entries != before { self.write(entries) } else { Ok(()) }
        })
    }

    /// Lists files before they are created. Returns the entries it replaced, by name.
    pub(crate) fn add(&self, files: &[PendingSample]) -> Result<Vec<PendingSample>, String> {
        let mut replaced = Vec::new();
        if files.is_empty() {
            return Ok(replaced);
        }
        self.update(|entries| {
            let mut added: Vec<&str> = Vec::new();
            for file in files {
                if let Some(at) = entries.iter().position(|entry| entry.name.eq_ignore_ascii_case(&file.name)) {
                    let earlier = entries.remove(at);
                    // A name listed twice in one call replaces only its own first entry.
                    if !added.iter().any(|name| name.eq_ignore_ascii_case(&file.name)) {
                        replaced.push(earlier);
                    }
                }
                entries.push(file.clone());
                added.push(&file.name);
            }
        })?;
        Ok(replaced)
    }

    /// Drops entries: their files were taken back, moved out or changed by someone else.
    pub(crate) fn remove(&self, names: &[String]) -> Result<(), String> {
        if names.is_empty() {
            return Ok(());
        }
        self.update(|entries| entries.retain(|entry| !names.iter().any(|name| name.eq_ignore_ascii_case(&entry.name))))
    }
}

/// Every file under `folder`, any depth.
fn files_under(folder: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut todo = vec![folder.to_path_buf()];
    while let Some(folder) = todo.pop() {
        let Ok(entries) = std::fs::read_dir(&folder) else { continue };
        for entry in entries.filter_map(Result::ok) {
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => todo.push(entry.path()),
                Ok(kind) if kind.is_file() => out.push(entry.path()),
                _ => {}
            }
        }
    }
    out
}

fn age(path: &Path) -> Option<std::time::Duration> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()?.elapsed().ok()
}

/// Gives a file today's date.
pub(crate) fn touch(path: &Path) {
    if let Ok(file) = std::fs::File::options().write(true).open(path) {
        let _ = file.set_modified(std::time::SystemTime::now());
    }
}

/// Holds `removed-samples`' lock: keeping a file there and the size cap never overlap.
pub(crate) fn removed_samples_lock(data_root: &Path) -> Result<std::fs::File, String> {
    let folder = data_root.join("removed-samples");
    std::fs::create_dir_all(&folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    let path = data_root.join("removed-samples.lock");
    let lock = std::fs::OpenOptions::new().create(true).truncate(false).read(true).write(true)
        .open(&path).map_err(|e| format!("Opening {}: {e}", path.display()))?;
    lock.lock().map_err(|e| format!("Locking {}: {e}", path.display()))?;
    Ok(lock)
}

/// Keeps `bytes` in `removed-samples` under their checksum, dated today, and returns where. A
/// copy already there with the same bytes is reused and dated today, so the 30 days count from
/// this removal. One with other bytes is damaged: it is renamed `.damaged-<stamp>`, never
/// overwritten. The caller holds `removed_samples_lock`.
pub(crate) fn keep_removed(data_root: &Path, name: &str, sha256: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    let extension = Path::new(name).extension().and_then(|e| e.to_str()).unwrap_or("wav").to_ascii_lowercase();
    let target = data_root.join("removed-samples").join(format!("{sha256}.{extension}"));
    match std::fs::read(&target) {
        Ok(kept) if kept == bytes => {
            touch(&target);
            return Ok(target);
        }
        Ok(_) => {
            let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis());
            let damaged = target.with_extension(format!("{extension}.damaged-{stamp}"));
            if damaged.exists() {
                return Err(format!("{} is damaged and could not be kept aside.", target.display()));
            }
            safe_write::rename_waiting(&target, &damaged).map_err(|e| e.to_string())?;
        }
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(format!("{}: {error}", target.display())),
        Err(_) => {}
    }
    match safe_write::create(&target, bytes) {
        Ok(()) => Ok(target),
        Err(_) if std::fs::read(&target).is_ok_and(|kept| kept == bytes) => Ok(target),
        Err(error) => Err(error.to_string()),
    }
}

/// Moves the kept copies in `pending-samples` that no project, recovery copy (archived ones
/// included), ledger, claim or sound set record names, and that are older than `min_age`, to
/// `removed-samples`: a younger one may belong to a window still making its claim. A copy is
/// removed from `pending-samples` only once `removed-samples` holds the same bytes. A record
/// names a copy by its checksum anywhere in its text. Without a project folder nothing moves.
/// Returns how many moved.
pub(crate) fn prune_kept_copies(data_root: &Path, project_root: Option<&Path>, min_age: std::time::Duration) -> usize {
    let Some(project_root) = project_root else { return 0 };
    let Ok(_lock) = removed_samples_lock(data_root) else { return 0 };
    let copies: Vec<PathBuf> = files_under(&data_root.join("pending-samples")).into_iter()
        .filter(|path| age(path).is_some_and(|age| age >= min_age)).collect();
    if copies.is_empty() {
        return 0;
    }
    let mut named = String::new();
    for folder in [
        project_root.to_path_buf(), data_root.join("recovery"), data_root.join("created-samples"),
        data_root.join("pending-names"), data_root.join("sound-set-snapshots"), data_root.join("kept-samples"),
    ] {
        for record in files_under(&folder) {
            // A record that can't be read could name anything: nothing moves.
            let Ok(bytes) = std::fs::read(&record) else { return 0 };
            named.push_str(&String::from_utf8_lossy(&bytes).to_ascii_lowercase());
            named.push('\n');
        }
    }
    let mut moved = 0;
    for copy in copies {
        let Some(sha256) = copy.file_stem().map(|stem| stem.to_string_lossy().to_ascii_lowercase()) else { continue };
        if sha256.len() != 64 || named.contains(&sha256) {
            continue;
        }
        let Ok(bytes) = std::fs::read(&copy) else { continue };
        let name = copy.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        let Ok(target) = keep_removed(data_root, &name, &sha256, &bytes) else { continue };
        if std::fs::read(&target).is_ok_and(|kept| kept == bytes) && std::fs::remove_file(&copy).is_ok() {
            moved += 1;
        }
    }
    moved
}

/// Keeps `removed-samples` within `max_age` and `max_bytes`, removing the oldest files first,
/// under its lock. The only place dawsu! deletes a sample outright. Returns how many went.
pub(crate) fn cap_removed_samples(data_root: &Path, max_age: std::time::Duration, max_bytes: u64) -> usize {
    let Ok(_lock) = removed_samples_lock(data_root) else { return 0 };
    let mut files: Vec<(std::time::Duration, u64, PathBuf)> = files_under(&data_root.join("removed-samples")).into_iter()
        .filter_map(|path| {
            let size = std::fs::metadata(&path).ok()?.len();
            Some((age(&path).unwrap_or_default(), size, path))
        }).collect();
    // Oldest first.
    files.sort_by_key(|file| std::cmp::Reverse(file.0));
    let mut total: u64 = files.iter().map(|(_, size, _)| size).sum();
    let mut removed = 0;
    for (age, size, path) in files {
        if (age > max_age || total > max_bytes) && std::fs::remove_file(&path).is_ok() {
            total -= size;
            removed += 1;
        }
    }
    removed
}

/// How long and how much `removed-samples` keeps.
pub(crate) const REMOVED_SAMPLES_DAYS: u64 = 30;
pub(crate) const REMOVED_SAMPLES_BYTES: u64 = 1024 * 1024 * 1024;
/// A kept copy younger than this is never pruned.
pub(crate) const PRUNE_MIN_AGE: std::time::Duration = std::time::Duration::from_secs(3600);
