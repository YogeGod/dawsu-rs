//! Cleans up the sample files dawsu! wrote into a mapset that nothing plays any more (Yoge,
//! 2026-11-20). Only files on the ledger are touched, and only while their bytes are still the
//! ones dawsu! wrote. A file a hit of the open difficulty still plays leaves the mapset and
//! becomes a pending file again; any other goes to `data_root/removed-samples`. Mix files from
//! builds before the ledger (`dawsu-mix-…`) are cleaned up the same way.

use super::{Application, layers, mixes, pending, project, recovery, safe_child};
use dawsu_foundation::{SampleIdentity, digest::sha256_hex};
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
    /// The files the open difficulty's hits play, lowercased. Empty when closing.
    hit_files: BTreeSet<String>,
    /// The files only an undo or redo state's hits play, lowercased: they wait in the data folder
    /// so Undo still plays them, and no project lists them. Empty when closing.
    undo_files: BTreeSet<String>,
    /// Whether the open difficulty has a project to list a pending file in.
    can_list: bool,
}

/// A file a hit still plays, kept as a pending file and moved aside. It leaves the mapset only
/// once the project lists it, on the editor thread; until then a crash leaves it aside, and the
/// next cleanup or open puts it back.
pub(crate) struct Moving {
    file: pending::PendingFile,
    temp: PathBuf,
    path: PathBuf,
}

/// What a cleanup job did.
pub(crate) struct Outcome {
    closing: bool,
    root: PathBuf,
    /// Files waiting to leave the mapset as pending files.
    to_pending: Vec<Moving>,
    /// The job's pending files, whose claims a file that goes back gives up.
    pending: pending::Pending,
    /// Files that left the mapset for `removed-samples`.
    removed: Vec<pending::PendingSample>,
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
    match std::fs::hard_link(temp, path) {
        Ok(()) => std::fs::remove_file(temp).is_ok(),
        // A drive with no hard links (FAT32, exFAT): renamed back while the name is free.
        Err(error) if error.kind() != std::io::ErrorKind::AlreadyExists && std::fs::symlink_metadata(path).is_err() => {
            std::fs::rename(temp, path).is_ok()
        }
        Err(_) => false,
    }
}

/// Files a stopped cleanup left under a moving name go back under their own. Returns their names.
#[allow(clippy::disallowed_methods)]
pub(crate) fn put_back_stopped_moves(root: &Path) -> Vec<String> {
    let mut back = Vec::new();
    for name in list_folder(root).unwrap_or_default() {
        let Some(original) = name.strip_prefix('.').and_then(|rest| rest.strip_suffix(MOVING)) else { continue };
        if let Ok(temp) = safe_child(root, &name)
            && !temp.with_file_name(original).exists()
            && put_back(&temp, &temp.with_file_name(original))
        {
            back.push(original.to_string());
        }
    }
    back
}

/// Keeps `bytes` in `removed-samples`, under its lock.
fn keep_removed(data_root: &Path, name: &str, sha256: &str, bytes: &[u8]) -> bool {
    let Ok(_lock) = pending::removed_samples_lock(data_root) else { return false };
    pending::keep_removed(data_root, name, sha256, bytes).is_ok()
}

/// Runs a cleanup. Called off the editor thread.
pub(crate) fn run(mut request: Request) -> Outcome {
    let mut outcome = Outcome {
        closing: request.closing, root: request.root.clone(), to_pending: Vec::new(), pending: pending::Pending::default(),
        removed: Vec::new(), problem: None, keep: None,
    };
    let root = request.root.clone();
    put_back_stopped_moves(&root);
    let mut entries = match request.ledger.entries() {
        Ok(entries) => entries,
        Err(error) => {
            outcome.problem = Some(format!("No unused sample files were removed. {error}"));
            Vec::new()
        }
    };
    let listed: BTreeSet<String> = entries.iter().map(|entry| entry.name.to_ascii_lowercase()).collect();
    entries.extend(old_mixes(&root).into_iter().filter(|mix| !listed.contains(&mix.name.to_ascii_lowercase())));
    if !entries.is_empty() {
        match folder_uses(&root).and_then(|uses| Ok((uses, recovery::copies_play(&request.data_root, &root)?))) {
            // A difficulty or recovery copy that can't be read leaves everything where it is.
            Err(error) => outcome.problem = Some(format!("No unused sample files were removed. {error}")),
            Ok(((used, other_mode_indexes), copies)) => {
                let keep = Keep {
                    used,
                    other_mode_indexes,
                    projects: project_texts(request.project_root.as_deref(), &root, request.except.as_deref()),
                    copies,
                };
                let mut dropped = Vec::new();
                for entry in entries {
                    if let Step::Drop = clean_up(&mut request, &mut outcome, &entry, &keep) {
                        dropped.push(entry.name.clone());
                    }
                }
                if let Err(error) = request.ledger.remove(&dropped) {
                    outcome.problem = Some(format!("Unused sample files were cleaned up, but their list could not be updated. {error}"));
                }
            }
        }
    }
    if request.closing {
        outcome.keep = Some(super::recorded_pending(request.project_root.as_deref(), &request.data_root, &root));
    }
    outcome.pending = request.pending;
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
    // A hit of the open difficulty still plays it: it can wait outside the mapset only where a
    // project lists it.
    let hit_plays = request.hit_files.contains(&lower);
    if hit_plays && !request.can_list {
        return Step::Kept;
    }
    let to_pending = hit_plays || request.undo_files.contains(&lower);
    before_move(&path);
    // Moved aside first, so no program writes it under its name while it leaves; then checked
    // again. A file a stopped cleanup left under the moving name is never moved over.
    let Some(temp) = moving_name(&path) else { return Step::Kept };
    if std::fs::symlink_metadata(&temp).is_ok() || std::fs::rename(&path, &temp).is_err() {
        // Locked: tried again after the next Save or at close.
        return Step::Kept;
    }
    // Unchanged, changed (not dawsu!'s any more), or unreadable for now (still dawsu!'s).
    let unchanged = || std::fs::read(&temp).ok().map(|now| now == bytes);
    let back = |now: Option<bool>| {
        put_back(&temp, &path);
        if now == Some(false) { Step::Drop } else { Step::Kept }
    };
    let now = unchanged();
    if now != Some(true) {
        return back(now);
    }
    if to_pending {
        let Ok(file) = request.pending.add(name, &bytes) else { return back(None) };
        let now = unchanged();
        if now != Some(true) {
            request.pending.release(name);
            return back(now);
        }
        // Its ledger entry goes once it left the mapset.
        outcome.to_pending.push(Moving { file, temp, path });
        return Step::Kept;
    }
    let now = keep_removed(&request.data_root, name, &entry.sha256, &bytes).then(unchanged).flatten();
    if now != Some(true) || std::fs::remove_file(&temp).is_err() {
        return back(now);
    }
    outcome.removed.push(entry.clone());
    Step::Drop
}

impl Application {
    /// Starts cleaning up the ledger's files nothing in the mapset plays, in a job. With
    /// `closing`, the open difficulty's hits no longer count: only the saved files and the
    /// projects do. Does nothing while another dawsu! session has the mapset open.
    pub(crate) fn start_sweep(&mut self, closing: bool) {
        if self.sweep.is_some() {
            return;
        }
        let Some(document) = self.document.as_ref() else { return };
        let Some(ledger) = document.pending.ledger() else { return };
        let Some(root) = document.path.parent().map(Path::to_path_buf) else { return };
        if document.presence.as_ref().is_none_or(|presence| presence.others_live()) {
            return;
        }
        let (hit_files, undo_files) = if closing {
            Default::default()
        } else {
            let hit_files = pending::hit_files(document.composer.hitsounds());
            // Each state names its own lanes: an older state may play a replaced lane sample.
            let undo_files = document.composer.history.states().flat_map(pending::hit_files)
                .filter(|name| !hit_files.contains(name)).collect();
            (hit_files, undo_files)
        };
        let request = Request {
            closing, root, data_root: self.data_root.clone(), project_root: self.project_root.clone(),
            except: (!closing).then(|| document.path.clone()), ledger, pending: document.pending.clone(), hit_files,
            undo_files, can_list: self.project.is_some(),
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
            // The job is gone: a file it was moving goes back now, unless another session's may be running.
            if let Some(document) = &self.document
                && document.presence.as_ref().is_some_and(|presence| !presence.others_live())
                && let Some(root) = document.path.parent()
            {
                put_back_stopped_moves(root);
            }
            self.error = Some("The unused sample cleanup stopped unexpectedly. A file it removed is kept in removed-samples; one it was moving stays in the mapset.".into());
            return Some(None);
        };
        Some(self.apply_sweep(outcome))
    }

    fn apply_sweep(&mut self, outcome: Outcome) -> Option<BTreeSet<String>> {
        let Outcome { closing, root, to_pending, mut pending, removed, problem, keep } = outcome;
        let Some(document) = self.document.as_mut().filter(|document| document.path.parent() == Some(root.as_path())) else {
            // Another mapset is open now: the files waiting to leave stay.
            for moving in &to_pending {
                pending.release(&moving.file.name);
                put_back(&moving.temp, &moving.path);
            }
            return keep;
        };
        for file in &removed {
            document.files.retain(|name| !name.eq_ignore_ascii_case(&file.name));
            document.pending.release_matching(&file.name, &file.sha256);
        }
        for moving in &to_pending {
            document.pending.adopt(moving.file.clone());
        }
        // A pending file a kept hit plays is listed in the project, so reopening plays it. A file
        // waiting to leave the mapset leaves only once it is listed.
        let mut listed = true;
        if !closing
            && let Some(project) = self.project.as_mut()
        {
            let document = self.document.as_ref().expect("open document");
            let needed = pending::needed(document.composer.hitsounds(), &document.pending, &[]);
            let already = project.settings.pending_samples.clone().unwrap_or_default();
            let missing: Vec<_> = needed.into_iter().filter(|file| !already.contains(file)).collect();
            if !missing.is_empty() {
                let mut settings = project.settings.clone();
                settings.pending_samples.get_or_insert_with(Vec::new).extend(missing);
                if let Err(error) = project.save(settings) {
                    listed = false;
                    self.error = Some(format!("The project could not list the new samples its hits play, so they stay in the mapset: {error}"));
                }
            }
        }
        // Without a project the job moved only files an undo state plays, which no project lists.
        let document = self.document.as_mut().expect("open document");
        let mut left = Vec::new();
        for moving in to_pending {
            if listed && std::fs::remove_file(&moving.temp).is_ok() {
                document.files.retain(|name| !name.eq_ignore_ascii_case(&moving.file.name));
                left.push(moving.file.name);
            } else {
                document.pending.release_matching(&moving.file.name, &moving.file.sha256);
                put_back(&moving.temp, &moving.path);
            }
        }
        let mut problem = problem;
        if !left.is_empty()
            && let Some(Err(error)) = document.pending.ledger().map(|ledger| ledger.remove(&left))
        {
            problem.get_or_insert(format!("Unused sample files were cleaned up, but their list could not be updated. {error}"));
        }
        if let Some(problem) = &problem {
            self.status = format!("{}. {problem}", self.status.trim_end_matches('.'));
        } else if let Some(removed) = removed_status(left.len() + removed.len()) {
            self.status = format!("{}. {removed}", self.status.trim_end_matches('.'));
        }
        keep
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
