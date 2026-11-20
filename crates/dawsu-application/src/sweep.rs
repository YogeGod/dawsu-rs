//! Cleans up the sample files dawsu! wrote into a mapset that nothing plays any more (Yoge,
//! 2026-11-20). Only files on the ledger are touched, and only while their bytes are still the
//! ones dawsu! wrote. A file a hit of the open difficulty still plays leaves the mapset and
//! becomes a pending file again; any other goes to `data_root/removed-samples`. Mix files from
//! builds before the ledger (`dawsu-mix-…`) are cleaned up the same way.

use super::{Application, layers, mixes, pending, project, recovery, safe_child};
use dawsu_foundation::{SampleIdentity, digest::sha256_hex, safe_write};
use dawsu_hitsounding::LaneSource;
use dawsu_hitsounding::rules::read_sample_file_name;
use dawsu_osu_format::{OsuFile, list_folder};
use std::collections::BTreeSet;
use std::path::Path;

/// What the mapset itself plays: the names its standard difficulties and storyboards play or
/// name, and the indexes a taiko, catch or mania difficulty may use, read conservatively.
#[allow(clippy::disallowed_methods)]
fn folder_uses(root: &Path) -> Result<(BTreeSet<String>, BTreeSet<u32>), String> {
    let files = list_folder(root).map_err(|e| e.to_string())?;
    let (_, mut used) = layers::reservations(root, &files)?;
    let mut other_modes = Vec::new();
    for name in files.iter().filter(|name| Path::new(name).extension().is_some_and(|e| e.eq_ignore_ascii_case("osu"))) {
        let bytes = std::fs::read(safe_child(root, name)?).map_err(|e| e.to_string())?;
        let file = OsuFile::read(bytes).file;
        let mode = file.general("Mode").and_then(|mode| mode.trim().parse::<i32>().ok()).unwrap_or(0);
        if mode == 0 {
            used.extend(pending::played_names(&file, &files).into_iter().map(|name| name.to_ascii_lowercase()));
        } else {
            other_modes.push(name.clone());
        }
    }
    let (indexes, _) = layers::reservations(root, &other_modes)?;
    Ok((used, indexes))
}

/// The mix files in the mapset that no ledger lists, from builds before the ledger (Yoge,
/// 2026-11-20). Only dawsu! names files `dawsu-mix-<sha256 of the bytes>-<n>.wav`, so the name
/// carries the checksum the cleanup checks the bytes against.
#[allow(clippy::disallowed_methods)]
fn old_mixes(root: &Path) -> Vec<pending::PendingSample> {
    list_folder(root).unwrap_or_default().into_iter().filter_map(|name| {
        let lower = name.to_ascii_lowercase();
        let rest = lower.strip_prefix(mixes::PREFIX)?.strip_suffix(".wav")?;
        let (sha256, n) = rest.split_once('-')?;
        (sha256.len() == 64 && sha256.bytes().all(|b| b.is_ascii_hexdigit()) && !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
            .then(|| pending::PendingSample { sha256: sha256.to_string(), name })
    }).collect()
}

/// The raw text of every project of the mapset's difficulties but `except`, lowercased: a file
/// any of them names stays where it is.
fn project_texts(project_root: Option<&Path>, root: &Path, except: Option<&Path>) -> Vec<String> {
    let Some(project_root) = project_root else { return Vec::new() };
    let Ok(entries) = std::fs::read_dir(root) else { return Vec::new() };
    entries.filter_map(Result::ok).map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|e| e.eq_ignore_ascii_case("osu")) && Some(path.as_path()) != except)
        .filter_map(|path| project::Project::load(project_root, None, &path).ok())
        .filter_map(|project| serde_json::to_string(&project.settings).ok())
        .map(|text| text.to_ascii_lowercase())
        .collect()
}

impl Application {
    /// Cleans up the ledger's files nothing in the mapset plays. With `closing`, the open
    /// difficulty's hits no longer count: only the saved files and the projects do. Returns how
    /// many files left the mapset. Does nothing while another dawsu! session has the mapset open.
    pub(crate) fn sweep_unused_samples(&mut self, closing: bool) -> usize {
        let project_root = self.project_root.clone();
        let data_root = self.data_root.clone();
        let Some(document) = self.document.as_mut() else { return 0 };
        let Some(ledger) = document.pending.ledger() else { return 0 };
        let Some(root) = document.path.parent().map(Path::to_path_buf) else { return 0 };
        if document.presence.as_ref().is_none_or(|presence| presence.others_live()) {
            return 0;
        }
        let mut entries = ledger.entries();
        let listed: BTreeSet<String> = entries.iter().map(|entry| entry.name.to_ascii_lowercase()).collect();
        entries.extend(old_mixes(&root).into_iter().filter(|mix| !listed.contains(&mix.name.to_ascii_lowercase())));
        if entries.is_empty() {
            return 0;
        }
        // A difficulty that can't be read leaves everything where it is.
        let Ok((used, other_mode_indexes)) = folder_uses(&root) else { return 0 };
        let except = (!closing).then(|| document.path.clone());
        let projects = project_texts(project_root.as_deref(), &root, except.as_deref());
        let copies = recovery::copies_play(&data_root, &root);
        let hits = document.composer.hitsounds().clone();
        let mut dropped = Vec::new();
        let mut moved = 0;
        for entry in entries {
            let name = entry.name.clone();
            let lower = name.to_ascii_lowercase();
            let Ok(path) = safe_child(&root, &name) else {
                // Gone from the mapset: nothing to clean up.
                dropped.push(name);
                continue;
            };
            let Ok(bytes) = std::fs::read(&path) else { continue };
            if sha256_hex(&bytes) != entry.sha256 {
                // Changed by someone else: not dawsu!'s file any more.
                dropped.push(name);
                continue;
            }
            let index = match read_sample_file_name(&name) {
                Some(SampleIdentity::Standard { index, .. }) => Some(index.get()),
                _ => None,
            };
            let stem = lower.rsplit_once('.').map_or(lower.as_str(), |(stem, _)| stem).to_string();
            if used.contains(&lower)
                || index.is_some_and(|index| other_mode_indexes.contains(&index))
                || projects.iter().any(|text| text.contains(&stem))
                || in_a_copy(&copies, &name)
            {
                continue;
            }
            let played_by_a_lane = !closing && hits.lanes().iter()
                .any(|lane| matches!(&lane.source, LaneSource::File(file) if file.eq_ignore_ascii_case(&name)));
            if played_by_a_lane {
                // A hit of the open difficulty still plays it: it waits in the data folder.
                if document.pending.add(&name, &bytes).is_err() {
                    continue;
                }
                if std::fs::remove_file(&path).is_err() {
                    document.pending.release(&name);
                    continue;
                }
                document.files.retain(|file| !file.eq_ignore_ascii_case(&name));
            } else {
                let extension = Path::new(&name).extension().and_then(|e| e.to_str()).unwrap_or("wav").to_ascii_lowercase();
                let folder = data_root.join("removed-samples");
                let target = folder.join(format!("{}.{extension}", entry.sha256));
                if std::fs::create_dir_all(&folder).is_err()
                    || !target.exists() && safe_write::create(&target, &bytes).is_err()
                    || std::fs::remove_file(&path).is_err()
                {
                    // Locked or unwritable: tried again after the next Save or at close.
                    continue;
                }
                document.files.retain(|file| !file.eq_ignore_ascii_case(&name));
                document.pending.release(&name);
            }
            dropped.push(name);
            moved += 1;
        }
        ledger.remove(&dropped);
        // A file a kept hit plays is listed in the project, so reopening plays it.
        if moved > 0 && !closing
            && let Some(project) = self.project.as_mut()
        {
            let document = self.document.as_ref().expect("open document");
            let needed = pending::needed(document.composer.hitsounds(), &document.pending, &[]);
            if !needed.is_empty() {
                let mut settings = project.settings.clone();
                let listed = settings.pending_samples.get_or_insert_with(Vec::new);
                for file in needed {
                    if !listed.contains(&file) {
                        listed.push(file);
                    }
                }
                if let Err(error) = project.save(settings) {
                    self.error = Some(format!("Unused sample files left the mapset, but the project could not list them: {error}"));
                }
            }
        }
        moved
    }
}

/// Whether a recovery copy plays or lists the file `name`.
fn in_a_copy(copies: &[SampleIdentity], name: &str) -> bool {
    let identity = read_sample_file_name(name);
    copies.iter().any(|sample| match sample {
        SampleIdentity::File(file) => file.eq_ignore_ascii_case(name),
        standard => identity.as_ref() == Some(standard),
    })
}

/// The words for the status line.
pub(crate) fn removed_status(count: usize) -> Option<String> {
    (count > 0).then(|| format!("Removed {count} unused sample file{}.", if count == 1 { "" } else { "s" }))
}
