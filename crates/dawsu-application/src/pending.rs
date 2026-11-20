//! Sample files dawsu! made that the mapset does not hold yet (pending files). A new layer, a
//! replaced lane sample or a placed sound set keeps its bytes in dawsu!'s data folder, at
//! `pending-samples/<sha256>.<extension>`, and claims its name under
//! `pending-names/<mapset key>/` so no other allocation, here or in another window, takes it.
//! A Save writes a pending file into the mapset only when the written `.osu` plays it.

use dawsu_foundation::{digest::sha256_hex, safe_write};
use dawsu_hitsounding::rules::read_sample_file_name;
use dawsu_foundation::SampleIdentity;
use dawsu_osu_format::list_folder;
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
            }),
        }
    }

    pub(crate) fn get(&self, name: &str) -> Option<&PendingFile> {
        let name = name.replace('\\', "/");
        self.files.get(&name.to_ascii_lowercase())
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
        let store = self.store.as_ref().ok_or("The open map has no data folder for new samples.")?;
        let sha256 = sha256_hex(bytes);
        let extension = Path::new(name).extension().and_then(|e| e.to_str()).unwrap_or("wav").to_ascii_lowercase();
        let path = store.samples.join(format!("{sha256}.{extension}"));
        std::fs::create_dir_all(&store.samples).map_err(|e| format!("Could not keep {name}: {e}"))?;
        match std::fs::read(&path) {
            Ok(stored) if stored == bytes => {}
            _ => {
                let _ = std::fs::remove_file(&path);
                safe_write::create(&path, bytes).map_err(|e| format!("Could not keep {name}: {e}"))?;
            }
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

    /// Drops a pending file a Save wrote into the mapset, and its claim.
    pub(crate) fn published(&mut self, name: &str) {
        if Arc::make_mut(&mut self.files).remove(&name.to_ascii_lowercase()).is_some()
            && let Some(store) = &self.store
        {
            let _ = std::fs::remove_file(store.claims.join(name.to_ascii_lowercase()));
        }
    }
}

/// The mapset's files as the editor sees them: the files on disk and the pending files.
pub(crate) fn listing(root: &Path, pending: &Pending) -> Result<Vec<String>, String> {
    let mut files = list_folder(root).map_err(|error| error.to_string())?;
    let on_disk: BTreeSet<String> = files.iter().map(|name| name.to_ascii_lowercase()).collect();
    files.extend(pending.files().filter(|file| !on_disk.contains(&file.name.to_ascii_lowercase())).map(|file| file.name.clone()));
    files.sort();
    Ok(files)
}
