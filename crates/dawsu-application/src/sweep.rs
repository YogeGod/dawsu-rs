//! Cleans up the sample files dawsu! wrote into a mapset that nothing plays any more (Yoge,
//! 2026-11-20). Only files on the ledger are touched, and only while their bytes are still the
//! ones dawsu! wrote. A file a hit of the open difficulty still plays leaves the mapset and
//! becomes a pending file again; one only an undo state plays stays until close; any other goes
//! to `data_root/removed-samples`. Mix files from
//! builds before the ledger (`dawsu-mix-…`) are cleaned up the same way.

use super::{Application, layers, mixes, pending, project, recovery, safe_child};
use dawsu_foundation::{SampleIdentity, digest::sha256_hex};
use dawsu_hitsounding::rules::read_sample_file_name;
use dawsu_osu_format::list_folder;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

/// The status words once close or open stopped waiting for a cleanup job.
pub(crate) const STILL_RUNNING: &str = " Unused sample cleanup is still finishing in the background.";

/// Test hook: runs with each file's path once its bytes were checked, before it moves.
#[cfg(test)]
pub(crate) static BEFORE_MOVE: std::sync::Mutex<Option<fn(&Path)>> = std::sync::Mutex::new(None);
/// Held by a test while it sets `BEFORE_MOVE`, so tests running at once never swap hooks.
#[cfg(test)]
pub(crate) static HOOK_TURN: std::sync::Mutex<()> = std::sync::Mutex::new(());

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

/// The mapset's difficulties and storyboards with their sizes and change times. A change while a
/// cleanup runs stops it (Yoge, 2026-11-21).
fn map_stamps(root: &Path) -> Vec<(PathBuf, u64, Option<SystemTime>)> {
    let Ok(entries) = std::fs::read_dir(root) else { return Vec::new() };
    let mut stamps: Vec<_> = entries.filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|e| e.eq_ignore_ascii_case("osu") || e.eq_ignore_ascii_case("osb")))
        .map(|entry| {
            let metadata = entry.metadata().ok();
            (entry.path(), metadata.as_ref().map_or(0, std::fs::Metadata::len), metadata.and_then(|m| m.modified().ok()))
        })
        .collect();
    stamps.sort();
    stamps
}

/// The raw text of the project of the difficulty at `path`, lowercased.
fn project_text(project_root: &Path, path: &Path) -> Option<String> {
    let project = project::Project::load(project_root, None, path).ok()?;
    serde_json::to_string(&project.settings).ok().map(|text| text.to_ascii_lowercase())
}

/// The raw text of every project of the mapset's difficulties but `except`, lowercased: a file
/// any of them names stays where it is.
fn project_texts(project_root: Option<&Path>, root: &Path, except: Option<&Path>) -> Vec<String> {
    let Some(project_root) = project_root else { return Vec::new() };
    let Ok(entries) = std::fs::read_dir(root) else { return Vec::new() };
    entries.filter_map(Result::ok).map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|e| e.eq_ignore_ascii_case("osu")) && Some(path.as_path()) != except)
        .filter_map(|path| project_text(project_root, &path))
        .collect()
}

/// What a cleanup job needs, taken from the open document when it starts.
pub(crate) struct Request {
    closing: bool,
    root: PathBuf,
    data_root: PathBuf,
    project_root: Option<PathBuf>,
    /// The open difficulty. Its project does not count while it stays open; at close it counts,
    /// but not for the files only an undo state plays.
    open: PathBuf,
    ledger: pending::Ledger,
    pending: pending::Pending,
    /// The files the open difficulty's hits play, lowercased. Empty when closing.
    hit_files: BTreeSet<String>,
    /// The files only an undo or redo state's hits play, lowercased: they stay in the mapset so
    /// Undo still plays them, and leave at close, when the undo history goes (Yoge, 2026-11-21).
    undo_files: BTreeSet<String>,
    /// Whether the open difficulty has a project to list a pending file in.
    can_list: bool,
    /// The session's hold on the mapset: the job keeps it marked open until it ends, and stops
    /// once another session opens it.
    presence: Arc<pending::Presence>,
    /// `Application::opens` when the job started.
    opens: u64,
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
    /// `Application::opens` when the job started: the document it serves.
    opens: u64,
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
        removed: Vec::new(), problem: None, keep: None, opens: request.opens,
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
        let stamps = map_stamps(&root);
        match folder_uses(&root).and_then(|uses| Ok((uses, recovery::copies_play(&request.data_root, &root)?))) {
            // A difficulty or recovery copy that can't be read leaves everything where it is.
            Err(error) => outcome.problem = Some(format!("No unused sample files were removed. {error}")),
            Ok(((used, other_mode_indexes), copies)) => {
                let project_root = request.project_root.as_deref();
                let keep = Keep {
                    used,
                    other_mode_indexes,
                    projects: project_texts(project_root, &root, Some(&request.open)),
                    own_project: project_root.filter(|_| request.closing).and_then(|project_root| project_text(project_root, &request.open)),
                    copies,
                    stamps,
                };
                let mut dropped = Vec::new();
                for entry in entries {
                    match clean_up(&mut request, &mut outcome, &entry, &keep) {
                        Step::Drop => dropped.push(entry.name.clone()),
                        Step::Kept => {}
                        // The rest stay; the next Save tries again.
                        Step::Stop => break,
                    }
                }
                if let Err(error) = request.ledger.remove(&dropped) {
                    outcome.problem = Some(format!("Unused sample files were cleaned up, but their list could not be updated. {error}"));
                }
            }
        }
    }
    if request.closing {
        outcome.keep = super::recorded_pending(request.project_root.as_deref(), &request.data_root, &root);
    }
    outcome.pending = request.pending;
    outcome
}

/// What keeps a file in the mapset.
struct Keep {
    used: BTreeSet<String>,
    other_mode_indexes: BTreeSet<u32>,
    /// The other difficulties' projects.
    projects: Vec<String>,
    /// At close, the open difficulty's project.
    own_project: Option<String>,
    copies: Vec<SampleIdentity>,
    /// `map_stamps` when the difficulties were read.
    stamps: Vec<(PathBuf, u64, Option<SystemTime>)>,
}

enum Step {
    Kept,
    /// Another session opened the mapset or a difficulty changed: this file and the rest stay.
    Stop,
    /// The ledger entry goes: the file left the mapset, is gone, or isn't dawsu!'s any more.
    Drop,
}

/// Shared retention decision used by both `clean_up()` and read-only `plan_sweep()`.
fn is_kept_by_rules(
    name: &str,
    lower: &str,
    used: &BTreeSet<String>,
    other_mode_indexes: &BTreeSet<u32>,
    projects: &[String],
    own_project: Option<&str>,
    copies: &[SampleIdentity],
    undo_only: bool,
    closing: bool,
) -> bool {
    let index = match read_sample_file_name(name) {
        Some(SampleIdentity::Standard { index, .. }) => Some(index.get()),
        _ => None,
    };
    let stem = lower.rsplit_once('.').map_or(lower, |(stem, _)| stem);
    used.contains(lower)
        || index.is_some_and(|index| other_mode_indexes.contains(&index))
        || projects.iter().any(|text| text.contains(stem))
        || (!undo_only && own_project.is_some_and(|text| text.contains(stem)))
        || in_a_copy(copies, name)
        || (undo_only && !closing)
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
    // While open, Undo may still play it. At close the open project may still name it (a replaced
    // lane's slot), but only the undo history played it.
    let undo_only = request.undo_files.contains(&lower);
    if is_kept_by_rules(
        name,
        &lower,
        &keep.used,
        &keep.other_mode_indexes,
        &keep.projects,
        keep.own_project.as_deref(),
        &keep.copies,
        undo_only,
        request.closing,
    ) {
        return Step::Kept;
    }
    // A hit of the open difficulty still plays it: it can wait outside the mapset only where a
    // project lists it.
    let to_pending = request.hit_files.contains(&lower);
    if to_pending && !request.can_list {
        return Step::Kept;
    }
    before_move(&path);
    // What the job read may be out of date now.
    if request.presence.others_live() || map_stamps(&request.root) != keep.stamps {
        return Step::Stop;
    }
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

/// One removable ledger or legacy-mix file reported by a read-only `plan_sweep()` dry-run.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnusedSampleCandidate {
    pub name: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub reason: String,
}

/// Immutable inputs captured on the editor thread so the dry-run SHA-256 scan can run off-thread.
pub struct SweepDryRunSnapshot {
    root: PathBuf,
    open_path: PathBuf,
    data_root: PathBuf,
    project_root: Option<PathBuf>,
    ledger: pending::Ledger,
    hit_files: BTreeSet<String>,
    undo_files: BTreeSet<String>,
    presence: Option<std::sync::Arc<pending::Presence>>,
}

/// Runs the read-only dry-run SHA-256 scan off the editor thread using `snapshot`.
pub fn run_plan_sweep(snapshot: SweepDryRunSnapshot) -> Result<Vec<UnusedSampleCandidate>, String> {
    if snapshot
        .presence
        .as_deref()
        .is_some_and(pending::Presence::others_live)
    {
        return Ok(Vec::new());
    }
    let mut entries = snapshot.ledger.entries().map_err(|e| e.to_string())?;
    let listed: BTreeSet<String> = entries
        .iter()
        .map(|e| e.name.to_ascii_lowercase())
        .collect();
    entries.extend(
        old_mixes(&snapshot.root)
            .into_iter()
            .filter(|m| !listed.contains(&m.name.to_ascii_lowercase())),
    );
    if entries.is_empty() {
        return Ok(Vec::new());
    }
    let (used, other_mode_indexes) = folder_uses(&snapshot.root)?;
    let copies = recovery::copies_play(&snapshot.data_root, &snapshot.root)?;
    let projects = project_texts(
        snapshot.project_root.as_deref(),
        &snapshot.root,
        Some(&snapshot.open_path),
    );
    let mut candidates = Vec::new();
    for entry in entries {
        let lower = entry.name.to_ascii_lowercase();
        let Ok(path) = safe_child(&snapshot.root, &entry.name) else {
            continue;
        };
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        if sha256_hex(&bytes) != entry.sha256 {
            continue;
        }
        let undo_only = snapshot.undo_files.contains(&lower);
        if snapshot.hit_files.contains(&lower)
            || is_kept_by_rules(
                &entry.name,
                &lower,
                &used,
                &other_mode_indexes,
                &projects,
                None,
                &copies,
                undo_only,
                false,
            )
        {
            continue;
        }
        let reason = if lower.starts_with(mixes::PREFIX) {
            "Unused dawsu! mix file".to_string()
        } else {
            "Unused dawsu!-generated sample".to_string()
        };
        candidates.push(UnusedSampleCandidate {
            name: entry.name,
            size_bytes: bytes.len() as u64,
            sha256: entry.sha256,
            reason,
        });
    }
    candidates.sort_by(|a, b| a.name.to_ascii_lowercase().cmp(&b.name.to_ascii_lowercase()));
    Ok(candidates)
}

impl Application {
    /// Captures a lightweight snapshot so `run_plan_sweep` can scan files off the editor thread.
    pub fn plan_sweep_snapshot(&self) -> Option<SweepDryRunSnapshot> {
        let document = self.document.as_ref()?;
        let ledger = document.pending.ledger()?;
        let root = document.path.parent()?.to_path_buf();
        let hit_files = pending::hit_files(document.composer.hitsounds());
        let undo_files: BTreeSet<String> = document
            .composer
            .history
            .states()
            .flat_map(pending::hit_files)
            .filter(|name| !hit_files.contains(name))
            .collect();
        Some(SweepDryRunSnapshot {
            root,
            open_path: document.path.clone(),
            data_root: self.data_root.clone(),
            project_root: self.project_root.clone(),
            ledger,
            hit_files,
            undo_files,
            presence: document.presence.clone(),
        })
    }

    /// Read-only dry-run listing ledger/mix files safe to quarantine to `removed-samples`.
    /// Honors `DECISIONS.md` (undo-reachable files, active hits, recovery copies, sibling projects,
    /// other-mode indexes, multi-session lock, and byte-hash ownership) without modifying files.
    pub fn plan_sweep(&self) -> Result<Vec<UnusedSampleCandidate>, String> {
        let Some(snapshot) = self.plan_sweep_snapshot() else {
            return Ok(Vec::new());
        };
        run_plan_sweep(snapshot)
    }

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
        let Some(presence) = document.presence.clone().filter(|presence| !presence.others_live()) else { return };
        let hit_files = pending::hit_files(document.composer.hitsounds());
        // Each state names its own lanes: an older state may play a replaced lane sample.
        let undo_files = document.composer.history.states().flat_map(pending::hit_files)
            .filter(|name| !hit_files.contains(name)).collect();
        let request = Request {
            closing, root, data_root: self.data_root.clone(), project_root: self.project_root.clone(),
            open: document.path.clone(), ledger, pending: document.pending.clone(),
            hit_files: if closing { BTreeSet::new() } else { hit_files }, undo_files, can_list: self.project.is_some(), presence,
            opens: self.opens,
        };
        let (send, receive) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = send.send(run(request));
        });
        self.sweep = Some(receive);
        self.busy = true;
    }

    /// Applies the cleanup job once it finished; `wait` waits up to `sweep_wait` for it. A job
    /// still running then goes on in the background. Returns, when a job finished, the pending
    /// names a record still lists (closing only).
    pub(crate) fn poll_sweep(&mut self, wait: bool) -> Option<Option<BTreeSet<String>>> {
        let receive = self.sweep.as_ref()?;
        // A job close or open already gave up waiting for is not waited for again.
        let outcome = if wait && !self.sweep_detached {
            match receive.recv_timeout(self.sweep_wait) {
                Ok(outcome) => Some(outcome),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    // Its document is about to go: the job no longer keeps `busy` set.
                    self.sweep_detached = true;
                    self.busy = false;
                    self.status = with_note(&self.status, STILL_RUNNING);
                    return None;
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => None,
            }
        } else {
            match receive.try_recv() {
                Ok(outcome) => Some(outcome),
                Err(std::sync::mpsc::TryRecvError::Empty) => return None,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => None,
            }
        };
        self.sweep = None;
        // A job close or open stopped waiting for no longer keeps `busy` set; another job may.
        let detached = std::mem::take(&mut self.sweep_detached);
        if detached {
            self.status = self.status.replace(STILL_RUNNING.trim_start(), "").trim_end().to_string();
        } else {
            self.busy = false;
        }
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
        let Outcome { closing, root, to_pending, mut pending, removed, problem, keep, opens } = outcome;
        let same_mapset = |document: &super::Document| document.path.parent() == Some(root.as_path());
        // A failed open keeps the document the job served; a new open or a close does not.
        let served = opens == self.opens;
        let Some(document) = self.document.as_mut().filter(|document| served && same_mapset(document)) else {
            // Its document is gone: the files waiting to leave stay, and the next Save tries again.
            for moving in &to_pending {
                pending.release(&moving.file.name);
                put_back(&moving.temp, &moving.path);
            }
            // The mapset, open again meanwhile, no longer has the files that went to removed-samples.
            if let Some(document) = self.document.as_mut().filter(|document| same_mapset(document)) {
                for file in &removed {
                    document.files.retain(|name| !name.eq_ignore_ascii_case(&file.name));
                }
            }
            if let Some(problem) = &problem {
                self.status = with_note(&self.status, problem);
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
        // Without a project the job moved nothing to pending.
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

/// `status` with `note` after it, as one more sentence.
pub(crate) fn with_note(status: &str, note: &str) -> String {
    let (status, note) = (status.trim_end_matches(['.', ' ']), note.trim_start());
    match status {
        "" => note.to_string(),
        _ if status.ends_with('…') => format!("{status} {note}"),
        _ => format!("{status}. {note}"),
    }
}

/// The words for the status line.
pub(crate) fn removed_status(count: usize) -> Option<String> {
    (count > 0).then(|| format!("Removed {count} unused sample file{}.", if count == 1 { "" } else { "s" }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cleanup_whose_map_closed_still_says_why_nothing_was_removed() {
        let mut app = Application::headless();
        app.sweep_detached = true;
        app.status = format!("Closed.{STILL_RUNNING}");
        let (send, receive) = std::sync::mpsc::channel();
        send.send(Outcome {
            closing: true, root: PathBuf::from("mapset"), to_pending: Vec::new(), pending: pending::Pending::default(),
            removed: Vec::new(), problem: Some("No unused sample files were removed. The recovery copy x.json could not be read.".into()),
            keep: None, opens: 0,
        }).unwrap();
        app.sweep = Some(receive);
        app.poll_sweep(false);
        assert!(app.status.contains("x.json could not be read"), "{}", app.status);
        assert!(!app.status.contains("still finishing"), "{}", app.status);
    }
}
