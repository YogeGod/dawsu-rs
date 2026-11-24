use crate::sample_watch::file_stamp;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime};

const DEFAULT_CHECK_INTERVAL: Duration = Duration::from_millis(250);
const DEFAULT_DEBOUNCE: Duration = Duration::from_millis(300);

/// Lightweight, cross-platform debounced watcher for the currently open `.osu` file.
///
/// Uses metadata `(size, mtime)` polling on `Application::poll()` so it adds zero OS
/// handle overhead or extra crates, waits for the file stamp to stay stable across the
/// debounce window (so osu!'s write/rename finishes cleanly), and compares bytes against
/// the last bytes `dawsu!` loaded or saved so our own saves never false-trigger.
pub(crate) struct OsuWatch {
    check_interval: Duration,
    debounce: Duration,
    next_check: Instant,
    last_stamp: Option<(u64, SystemTime)>,
    pending_since: Option<(Instant, (u64, SystemTime))>,
    dismissed_bytes: Option<Vec<u8>>,
}

impl Default for OsuWatch {
    fn default() -> Self {
        Self {
            check_interval: DEFAULT_CHECK_INTERVAL,
            debounce: DEFAULT_DEBOUNCE,
            next_check: Instant::now() + DEFAULT_CHECK_INTERVAL,
            last_stamp: None,
            pending_since: None,
            dismissed_bytes: None,
        }
    }
}

impl OsuWatch {
    #[cfg(test)]
    pub(crate) fn set_timings(&mut self, check_interval: Duration, debounce: Duration) {
        self.check_interval = check_interval;
        self.debounce = debounce;
        self.next_check = Instant::now();
    }

    pub(crate) fn clear(&mut self) {
        self.last_stamp = None;
        self.pending_since = None;
        self.dismissed_bytes = None;
        self.next_check = Instant::now() + self.check_interval;
    }

    pub(crate) fn reset_for(&mut self, path: &Path) {
        self.last_stamp = file_stamp(path);
        self.pending_since = None;
        self.dismissed_bytes = None;
        self.next_check = Instant::now() + self.check_interval;
    }

    /// Remember the on-disk bytes the mapper chose to ignore ("Keep my edits") so we don't
    /// keep alerting for the same write, while still alerting if osu! writes a newer change.
    pub(crate) fn dismiss(&mut self, path: &Path) {
        self.last_stamp = file_stamp(path);
        self.pending_since = None;
        self.dismissed_bytes = std::fs::read(path).ok();
        self.next_check = Instant::now() + self.check_interval;
    }

    /// Returns `true` once a settled external change is detected whose bytes differ from
    /// `expected_bytes` (the bytes `dawsu!` last loaded or saved) and from any dismissed write.
    pub(crate) fn poll(&mut self, path: &Path, expected_bytes: &[u8]) -> bool {
        let now = Instant::now();
        if now < self.next_check {
            return false;
        }
        self.next_check = now + self.check_interval;

        let Some(current_stamp) = file_stamp(path) else {
            // File may be mid-rename (e.g. osu! temp-file swap); wait for next tick.
            return false;
        };

        if let Some((since, pending_stamp)) = self.pending_since {
            if current_stamp != pending_stamp {
                // Still changing on disk — restart the debounce timer.
                self.pending_since = Some((now, current_stamp));
                return false;
            }
            if now.duration_since(since) < self.debounce {
                return false;
            }
            self.pending_since = None;
            self.last_stamp = Some(current_stamp);
            return self.bytes_changed(path, expected_bytes);
        }

        if self.last_stamp.is_none() {
            self.last_stamp = Some(current_stamp);
            return false;
        }

        if Some(current_stamp) != self.last_stamp {
            if self.debounce.is_zero() {
                self.last_stamp = Some(current_stamp);
                return self.bytes_changed(path, expected_bytes);
            }
            self.pending_since = Some((now, current_stamp));
        }
        false
    }

    fn bytes_changed(&self, path: &Path, expected_bytes: &[u8]) -> bool {
        let Ok(disk_bytes) = std::fs::read(path) else {
            return false;
        };
        if disk_bytes == expected_bytes {
            return false;
        }
        if self.dismissed_bytes.as_deref() == Some(disk_bytes.as_slice()) {
            return false;
        }
        true
    }
}
