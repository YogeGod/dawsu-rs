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

/// The open mapset's pending files. Cheap to clone: a Save job takes a snapshot.
#[derive(Clone, Debug, Default)]
pub(crate) struct Pending {
    files: Arc<BTreeMap<String, PendingFile>>,
    store: Option<Store>,
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
        }
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

    /// The names claimed for this mapset on disk, by this or another window, lowercased.
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
        std::fs::create_dir_all(&store.samples).map_err(|e| format!("Could not keep {name}: {e}"))?;
        // The bytes are kept by their checksum, so another window may have kept them first.
        let kept = || std::fs::read(&path).is_ok_and(|stored| stored == bytes);
        if !kept() && let Err(error) = safe_write::create(&path, bytes) && !kept() {
            return Err(format!("Could not keep {name}: {error}"));
        }
        std::fs::create_dir_all(&store.claims).map_err(|e| format!("Could not keep {name}: {e}"))?;
        let claim = store.claims.join(name.to_ascii_lowercase());
        safe_write::create(&claim, sha256.as_bytes())
            .map_err(|_| format!("{name} was just taken by another dawsu! window. Try again."))?;
        let file = PendingFile { name: name.to_string(), sha256, path };
        Arc::make_mut(&mut self.files).insert(name.to_ascii_lowercase(), file.clone());
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
        Ok(file)
    }

    /// Takes in a pending file a cleanup job kept, with its claim already made.
    pub(crate) fn adopt(&mut self, file: PendingFile) {
        Arc::make_mut(&mut self.files).insert(file.name.to_ascii_lowercase(), file);
    }

    /// Drops a pending file and its claim: a Save wrote it into the mapset, or keeping it failed.
    pub(crate) fn release(&mut self, name: &str) {
        if Arc::make_mut(&mut self.files).remove(&name.to_ascii_lowercase()).is_some()
            && let Some(store) = &self.store
        {
            let _ = std::fs::remove_file(store.claims.join(name.to_ascii_lowercase()));
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
    let indexes: BTreeMap<u32, &String> = writes.iter().filter_map(|name| match read_sample_file_name(name)? {
        SampleIdentity::Standard { index, .. } => Some((index.get(), name)),
        SampleIdentity::File(_) => None,
    }).collect();
    if indexes.is_empty() {
        return Ok(());
    }
    let own = saved.file_name().map(|name| name.to_string_lossy().to_ascii_lowercase());
    for other in list_folder(root).map_err(|e| e.to_string())? {
        if !Path::new(&other).extension().is_some_and(|e| e.eq_ignore_ascii_case("osu")) || Some(other.to_ascii_lowercase()) == own {
            continue;
        }
        let (used, _) = crate::layers::reservations(root, std::slice::from_ref(&other))?;
        if let Some((index, name)) = indexes.iter().find(|(index, _)| used.contains(index)) {
            return Err(format!("{name} uses sample index {index}, which {other} now uses too. Nothing was saved."));
        }
    }
    Ok(())
}

/// The pending files the drawn hits play that this Save does not write.
pub(crate) fn needed(hits: &Hitsounds, pending: &Pending, written: &[String]) -> Vec<PendingSample> {
    let written: BTreeSet<String> = written.iter().map(|n| n.to_ascii_lowercase()).collect();
    let lanes = hits.moments().iter().flat_map(|moment| moment.hits().cloned())
        .chain(hits.bodies().iter().flat_map(|body| [body.slide().clone(), body.whistle_lane(), body.tick().clone()]));
    let mut needed = BTreeMap::new();
    for lane in lanes {
        if let Some(LaneSource::File(name)) = hits.lane(&lane).map(|lane| &lane.source)
            && let Some(file) = pending.get(name)
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

static NEXT_PRESENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

impl Presence {
    pub(crate) fn enter(data_root: &Path, mapset: &Path) -> Result<Self, String> {
        let folder = data_root.join("sessions").join(mapset_key(mapset));
        std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
        let n = NEXT_PRESENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = folder.join(format!("{}-{n}.lock", std::process::id()));
        let file = std::fs::File::create(&path).map_err(|e| e.to_string())?;
        file.try_lock().map_err(|e| e.to_string())?;
        Ok(Self { file: Some(file), path })
    }

    /// Whether another session holds this mapset. Lock files of ended sessions are removed.
    pub(crate) fn others_live(&self) -> bool {
        let Some(folder) = self.path.parent() else { return false };
        let Ok(entries) = std::fs::read_dir(folder) else { return false };
        let mut live = false;
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if path == self.path {
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
    /// The listed files. A ledger that can't be read is kept aside, next to it with `corrupt` in
    /// its name, and a new one is started: no file it listed is cleaned up.
    pub(crate) fn entries(&self) -> Vec<PendingSample> {
        let Ok(bytes) = std::fs::read(&self.path) else { return Vec::new() };
        match serde_json::from_slice::<LedgerFile>(&bytes) {
            Ok(file) => file.files,
            Err(_) => {
                let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis());
                let aside = self.path.with_extension(format!("corrupt-{stamp}-{}.json", std::process::id()));
                let _ = std::fs::rename(&self.path, aside);
                Vec::new()
            }
        }
    }

    fn write(&self, files: Vec<PendingSample>) -> Result<(), String> {
        let folder = self.path.parent().ok_or("The ledger has no folder.")?;
        std::fs::create_dir_all(folder).map_err(|e| format!("Recording new sample files: {e}"))?;
        let bytes = serde_json::to_vec_pretty(&LedgerFile { files }).map_err(|e| e.to_string())?;
        let temp = self.path.with_extension(format!("{}.tmp", std::process::id()));
        std::fs::write(&temp, bytes).map_err(|e| format!("Recording new sample files: {e}"))?;
        std::fs::rename(&temp, &self.path).map_err(|e| {
            let _ = std::fs::remove_file(&temp);
            format!("Recording new sample files: {e}")
        })
    }

    /// Lists files before they are created.
    pub(crate) fn add(&self, files: &[PendingSample]) -> Result<(), String> {
        if files.is_empty() {
            return Ok(());
        }
        let mut entries = self.entries();
        for file in files {
            entries.retain(|entry| !entry.name.eq_ignore_ascii_case(&file.name));
            entries.push(file.clone());
        }
        self.write(entries)
    }

    /// Drops entries: their files were taken back, moved out or changed by someone else.
    pub(crate) fn remove(&self, names: &[String]) {
        let mut entries = self.entries();
        let before = entries.len();
        entries.retain(|entry| !names.iter().any(|name| name.eq_ignore_ascii_case(&entry.name)));
        if entries.len() != before {
            let _ = self.write(entries);
        }
    }
}
