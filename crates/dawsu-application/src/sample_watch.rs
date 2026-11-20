use crate::{Document, safe_child};
use dawsu_audio::{AudioClip, decode_with_cancel};
use dawsu_osu_format::list_folder;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

const SCAN_INTERVAL: Duration = Duration::from_secs(1);
type AddedSamples = Vec<(String, Arc<AudioClip>)>;

pub(crate) struct Watch {
    next_scan: Instant,
    scan: Option<Scan>,
}

impl Default for Watch {
    fn default() -> Self {
        Self {
            next_scan: Instant::now() + SCAN_INTERVAL,
            scan: None,
        }
    }
}

struct Scan {
    result: mpsc::Receiver<AddedSamples>,
    cancel: Arc<AtomicBool>,
}

impl Drop for Scan {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

impl Watch {
    pub(crate) fn poll(&mut self, document: &Document) -> AddedSamples {
        if let Some(scan) = &self.scan {
            return match scan.result.try_recv() {
                Err(mpsc::TryRecvError::Empty) => Vec::new(),
                result => {
                    self.scan = None;
                    self.next_scan = Instant::now() + SCAN_INTERVAL;
                    result.unwrap_or_default()
                }
            };
        }
        if Instant::now() >= self.next_scan
            && let Some(folder) = document.path.parent()
        {
            self.scan = Some(start_scan(
                folder.to_owned(),
                document.files.clone(),
                document.file.general("AudioFilename"),
            ));
        }
        Vec::new()
    }
}

/// Looks for files new on disk, so it reads the folder alone.
#[allow(clippy::disallowed_methods)]
fn start_scan(folder: PathBuf, known: Vec<String>, song: Option<String>) -> Scan {
    let (send, result) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = cancel.clone();
    std::thread::spawn(move || {
        let stopped = || worker_cancel.load(Ordering::Relaxed);
        let known: HashSet<_> = known.iter().map(String::as_str).collect();
        let song = song.map(|name| name.replace('\\', "/"));
        let added = list_folder(&folder)
            .unwrap_or_default()
            .into_iter()
            .take_while(|_| !stopped())
            .filter(|name| !known.contains(name.as_str()))
            .filter(|name| {
                !song
                    .as_ref()
                    .is_some_and(|song| song.eq_ignore_ascii_case(name))
            })
            .filter(|name| {
                Path::new(name).extension().is_some_and(|ext| {
                    ["wav", "ogg", "mp3"]
                        .iter()
                        .any(|audio| ext.eq_ignore_ascii_case(audio))
                })
            })
            .filter_map(|name| {
                let path = safe_child(&folder, &name).ok()?;
                let before = file_stamp(&path)?;
                let clip = decode_with_cancel(&path, &stopped).ok()?;
                // A copy that is still in progress gets another chance on the next scan.
                (file_stamp(&path)? == before).then_some((name, clip))
            })
            .collect();
        let _ = send.send(added);
    });
    Scan { result, cancel }
}

pub(super) fn file_stamp(path: &Path) -> Option<(u64, std::time::SystemTime)> {
    let metadata = std::fs::metadata(path).ok()?;
    Some((metadata.len(), metadata.modified().ok()?))
}
