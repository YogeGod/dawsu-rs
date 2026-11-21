//! Cleans up the sample files dawsu! wrote into a mapset that nothing plays any more (Yoge,
//! 2026-11-20). Only files on the ledger are touched, and only while their bytes are still the
//! ones dawsu! wrote. A file a hit of the open difficulty still plays leaves the mapset and
//! becomes a pending file again; any other goes to `data_root/removed-samples`. Mix files from
//! builds before the ledger (`dawsu-mix-…`) are cleaned up the same way.

use super::{Application, layers, mixes, pending, project, recovery, safe_child};
use dawsu_foundation::{SampleIdentity, digest::sha256_hex, safe_write};
use dawsu_hitsounding::LaneSource;
use dawsu_hitsounding::rules::read_sample_file_name;
use dawsu_osu_format::list_folder;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Test hook: runs with each file's path once its bytes were checked, before it moves.
#[cfg(test)]
pub(crate) static BEFORE_MOVE: std::sync::Mutex<Option<fn(&Path)>> = std::sync::Mutex::new(None);

#[cfg(test)]
fn before_move(path: &Path) {
    let hook = *BEFORE_MOVE.lock().unwrap();
    if let Some(hook) = hook {
        hook(path);
    }
}
#[cfg(not(test))]
fn before_move(_: &Path) {}

/// What the mapset itself plays: the names its standard difficulties and storyboards play or
/// name, and the indexes a taiko, catch or mania difficulty may use, read conservatively. Each
/// difficulty and storyboard is read once.
#[allow(clippy::disallowed_methods)]
fn folder_uses(root: &Path) -> Result<(BTreeSet<String>, BTreeSet<u32>), String> {
    let files = list_folder(root).map_err(|e| e.to_string())?;
    let read = layers::read_beatmap_files(root, &files)?;
    let (_, mut used) = layers::reservations_in(read.iter().map(|(name, read)| (name.as_str(), read)))?;
    let is_difficulty = |name: &str| Path::new(name).extension().is_some_and(|e| e.eq_ignore_ascii_case("osu"));
    let mut other_modes = Vec::new();
    for (name, read) in read.iter().filter(|(name, _)| is_difficulty(name)) {
        let mode = read.file.general("Mode").and_then(|mode| mode.trim().parse::<i32>().ok()).unwrap_or(0);
        if mode == 0 {
            used.extend(pending::played_names(&read.file, &files).into_iter().map(|name| name.to_ascii_lowercase()));
        } else {
            other_modes.push((name.as_str(), read));
        }
    }
    let (indexes, _) = layers::reservations_in(other_modes)?;
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

/// What a cleanup job needs, taken from the open document when it starts.
pub(crate) struct Request {
    closing: bool,
    root: PathBuf,
    data_root: PathBuf,
    project_root: Option<PathBuf>,
    /// The open difficulty, whose project does not count while it stays open.
    except: Option<PathBuf>,
    ledger: pending::Ledger,
    pending: pending::Pending,
    /// The files the open difficulty's lanes play, lowercased. Empty when closing.
    lane_files: BTreeSet<String>,
}

/// What a cleanup job did.
pub(crate) struct Outcome {
    closing: bool,
    root: PathBuf,
    /// Files that left the mapset and wait in the data folder as pending files.
    to_pending: Vec<pending::PendingFile>,
    /// Files that left the mapset for `removed-samples`.
    removed: Vec<String>,
    /// Why nothing was cleaned up.
    problem: Option<String>,
    /// When closing: the pending names a record still lists.
    keep: Option<BTreeSet<String>>,
}

const MOVING: &str = ".dawsu-moving";

/// The name a file has while it moves out of the mapset.
fn moving_name(path: &Path) -> Option<PathBuf> {
    let name = path.file_name()?.to_string_lossy();
    Some(path.with_file_name(format!(".{name}{MOVING}")))
}

/// Puts a moving file back under its name, never over a file that took the name meanwhile.
fn put_back(temp: &Path, path: &Path) -> bool {
    std::fs::hard_link(temp, path).is_ok() && std::fs::remove_file(temp).is_ok()
}

/// Files a stopped cleanup left under a moving name go back under their own.
#[allow(clippy::disallowed_methods)]
fn put_back_stopped_moves(root: &Path) {
    for name in list_folder(root).unwrap_or_default() {
        let Some(original) = name.strip_prefix('.').and_then(|rest| rest.strip_suffix(MOVING)) else { continue };
        if let Ok(temp) = safe_child(root, &name)
            && !temp.with_file_name(original).exists()
        {
            put_back(&temp, &temp.with_file_name(original));
        }
    }
}

/// Keeps `bytes` in `removed-samples` under their checksum. A copy there with other bytes is replaced.
fn keep_removed(data_root: &Path, name: &str, sha256: &str, bytes: &[u8]) -> bool {
    let extension = Path::new(name).extension().and_then(|e| e.to_str()).unwrap_or("wav").to_ascii_lowercase();
    let folder = data_root.join("removed-samples");
    let target = folder.join(format!("{sha256}.{extension}"));
    if std::fs::create_dir_all(&folder).is_err() {
        return false;
    }
    match std::fs::read(&target) {
        Ok(kept) if kept == bytes => true,
        Ok(_) => safe_write::write_new(&target, bytes).is_ok(),
        Err(_) => safe_write::create(&target, bytes).is_ok() || std::fs::read(&target).is_ok_and(|kept| kept == bytes),
    }
}

/// Runs a cleanup. Called off the editor thread.
pub(crate) fn run(mut request: Request) -> Outcome {
    let mut outcome = Outcome {
        closing: request.closing, root: request.root.clone(), to_pending: Vec::new(), removed: Vec::new(), problem: None, keep: None,
    };
    let root = request.root.clone();
    put_back_stopped_moves(&root);
    let mut entries = request.ledger.entries();
    let listed: BTreeSet<String> = entries.iter().map(|entry| entry.name.to_ascii_lowercase()).collect();
    entries.extend(old_mixes(&root).into_iter().filter(|mix| !listed.contains(&mix.name.to_ascii_lowercase())));
    if !entries.is_empty() {
        match folder_uses(&root) {
            // A difficulty that can't be read leaves everything where it is.
            Err(error) => outcome.problem = Some(format!("No unused sample files were removed. {error}")),
            Ok((used, other_mode_indexes)) => {
                let keep = Keep {
                    used,
                    other_mode_indexes,
                    projects: project_texts(request.project_root.as_deref(), &root, request.except.as_deref()),
                    copies: recovery::copies_play(&request.data_root, &root),
                };
                let mut dropped = Vec::new();
                for entry in entries {
                    if let Step::Drop = clean_up(&mut request, &mut outcome, &entry, &keep) {
                        dropped.push(entry.name.clone());
                    }
                }
                request.ledger.remove(&dropped);
            }
        }
    }
    if request.closing {
        outcome.keep = Some(super::recorded_pending(request.project_root.as_deref(), &request.data_root, &root));
    }
    outcome
}

/// What keeps a file in the mapset.
struct Keep {
    used: BTreeSet<String>,
    other_mode_indexes: BTreeSet<u32>,
    projects: Vec<String>,
    copies: Vec<SampleIdentity>,
}

enum Step {
    Kept,
    /// The ledger entry goes: the file left the mapset, is gone, or isn't dawsu!'s any more.
    Drop,
}

fn clean_up(request: &mut Request, outcome: &mut Outcome, entry: &pending::PendingSample, keep: &Keep) -> Step {
    let name = entry.name.as_str();
    let lower = name.to_ascii_lowercase();
    let Ok(path) = safe_child(&request.root, name) else {
        // Gone from the mapset: nothing to clean up.
        return Step::Drop;
    };
    let Ok(bytes) = std::fs::read(&path) else { return Step::Kept };
    if sha256_hex(&bytes) != entry.sha256 {
        // Changed by someone else: not dawsu!'s file any more.
        return Step::Drop;
    }
    let index = match read_sample_file_name(name) {
        Some(SampleIdentity::Standard { index, .. }) => Some(index.get()),
        _ => None,
    };
    let stem = lower.rsplit_once('.').map_or(lower.as_str(), |(stem, _)| stem);
    if keep.used.contains(&lower)
        || index.is_some_and(|index| keep.other_mode_indexes.contains(&index))
        || keep.projects.iter().any(|text| text.contains(stem))
        || in_a_copy(&keep.copies, name)
    {
        return Step::Kept;
    }
    before_move(&path);
    // Moved aside first, so no program writes it under its name while it leaves; then checked again.
    let Some(temp) = moving_name(&path) else { return Step::Kept };
    if std::fs::rename(&path, &temp).is_err() {
        // Locked: tried again after the next Save or at close.
        return Step::Kept;
    }
    let changed = || std::fs::read(&temp).map_or(true, |now| now != bytes);
    if changed() {
        put_back(&temp, &path);
        return Step::Drop;
    }
    let to_pending = request.lane_files.contains(&lower);
    let kept = if to_pending { request.pending.add(name, &bytes).ok() } else { None };
    let stored = if to_pending { kept.is_some() } else { keep_removed(&request.data_root, name, &entry.sha256, &bytes) };
    let changed_meanwhile = stored && changed();
    if !stored || changed_meanwhile || std::fs::remove_file(&temp).is_err() {
        if kept.is_some() {
            request.pending.release(name);
        }
        put_back(&temp, &path);
        return if changed_meanwhile { Step::Drop } else { Step::Kept };
    }
    match kept {
        // A lane of the open difficulty still plays it: it waits in the data folder.
        Some(file) => outcome.to_pending.push(file),
        None => outcome.removed.push(name.to_string()),
    }
    Step::Drop
}

impl Application {
    /// Starts cleaning up the ledger's files nothing in the mapset plays, in a job. With
    /// `closing`, the open difficulty's hits no longer count: only the saved files and the
    /// projects do. Does nothing while another dawsu! session has the mapset open.
    pub(crate) fn start_sweep(&mut self, closing: bool) {
        let Some(document) = self.document.as_ref() else { return };
        let Some(ledger) = document.pending.ledger() else { return };
        let Some(root) = document.path.parent().map(Path::to_path_buf) else { return };
        if document.presence.as_ref().is_none_or(|presence| presence.others_live()) {
            return;
        }
        let lane_files = if closing {
            BTreeSet::new()
        } else {
            document.composer.hitsounds().lanes().iter().filter_map(|lane| match &lane.source {
                LaneSource::File(file) => Some(file.to_ascii_lowercase()),
                _ => None,
            }).collect()
        };
        let request = Request {
            closing, root, data_root: self.data_root.clone(), project_root: self.project_root.clone(),
            except: (!closing).then(|| document.path.clone()), ledger, pending: document.pending.clone(), lane_files,
        };
        let (send, receive) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = send.send(run(request));
        });
        self.sweep = Some(receive);
        self.busy = true;
    }

    /// Applies the cleanup job once it finished; `wait` blocks until it does. Returns, when a
    /// job finished, the pending names a record still lists (closing only).
    pub(crate) fn poll_sweep(&mut self, wait: bool) -> Option<Option<BTreeSet<String>>> {
        let receive = self.sweep.as_ref()?;
        let outcome = if wait {
            receive.recv().ok()
        } else {
            match receive.try_recv() {
                Ok(outcome) => Some(outcome),
                Err(std::sync::mpsc::TryRecvError::Empty) => return None,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => None,
            }
        };
        self.sweep = None;
        self.busy = false;
        let Some(outcome) = outcome else {
            self.error = Some("The unused sample cleanup stopped unexpectedly. No file was removed by it.".into());
            return Some(None);
        };
        Some(self.apply_sweep(outcome))
    }

    fn apply_sweep(&mut self, outcome: Outcome) -> Option<BTreeSet<String>> {
        let document = self.document.as_mut()?;
        if document.path.parent() != Some(outcome.root.as_path()) {
            return outcome.keep;
        }
        let moved = outcome.to_pending.len() + outcome.removed.len();
        for file in outcome.to_pending {
            document.files.retain(|name| !name.eq_ignore_ascii_case(&file.name));
            document.pending.adopt(file);
        }
        for name in &outcome.removed {
            document.files.retain(|file| !file.eq_ignore_ascii_case(name));
            document.pending.release(name);
        }
        if let Some(problem) = &outcome.problem {
            self.status = format!("{}. {problem}", self.status.trim_end_matches('.'));
        } else if let Some(removed) = removed_status(moved) {
            self.status = format!("{}. {removed}", self.status.trim_end_matches('.'));
        }
        // A pending file a kept hit plays is listed in the project, so reopening plays it.
        if !outcome.closing
            && let Some(project) = self.project.as_mut()
        {
            let document = self.document.as_ref().expect("open document");
            let needed = pending::needed(document.composer.hitsounds(), &document.pending, &[]);
            let listed = project.settings.pending_samples.clone().unwrap_or_default();
            let missing: Vec<_> = needed.into_iter().filter(|file| !listed.contains(file)).collect();
            if !missing.is_empty() {
                let mut settings = project.settings.clone();
                settings.pending_samples.get_or_insert_with(Vec::new).extend(missing);
                if let Err(error) = project.save(settings) {
                    self.error = Some(format!("Unused sample files left the mapset, but the project could not list them: {error}"));
                }
            }
        }
        outcome.keep
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
