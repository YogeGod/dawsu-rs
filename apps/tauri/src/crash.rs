use std::{
    any::Any,
    backtrace::Backtrace,
    cell::{Cell, RefCell},
    ffi::OsString,
    fs::{self, OpenOptions},
    io::Write,
    panic::{self, PanicHookInfo},
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

/// Rotate `crash.log` to `crash.log.1` once appending would exceed 512 KiB.
const MAX_CRASH_LOG_BYTES: u64 = 512 * 1024;
const MAX_DIALOG_MESSAGE_CHARS: usize = 300;

static LOG_LOCK: Mutex<()> = Mutex::new(());
static DIALOG_SHOWN: AtomicBool = AtomicBool::new(false);

thread_local! {
    static IN_PANIC_HOOK: Cell<bool> = const { Cell::new(false) };
    /// Above zero while `guarded` runs on this thread: `guard` reports the panic itself.
    static GUARDED_DEPTH: Cell<u32> = const { Cell::new(0) };
    /// The crash report the last panic on this thread saved.
    static LAST_SAVED_LOG_PATH: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

struct GuardedScope;

impl Drop for GuardedScope {
    fn drop(&mut self) {
        GUARDED_DEPTH.with(|depth| depth.set(depth.get().saturating_sub(1)));
    }
}

/// Runs `f` as an operation whose panic the caller catches and shows: the panic hook still
/// writes `crash.log` and stderr, but shows no native dialog.
pub(crate) fn guarded<T>(f: impl FnOnce() -> T) -> T {
    GUARDED_DEPTH.with(|depth| depth.set(depth.get() + 1));
    let _scope = GuardedScope;
    f()
}

fn is_guarded() -> bool {
    GUARDED_DEPTH.with(|depth| depth.get() > 0)
}

fn remember_saved_log_path(path: Option<PathBuf>) {
    LAST_SAVED_LOG_PATH.with(|last| *last.borrow_mut() = path);
}

/// The crash report of the last panic on this thread, else where reports are written.
pub(crate) fn crash_log_path() -> PathBuf {
    LAST_SAVED_LOG_PATH
        .with(|last| last.borrow().clone())
        .unwrap_or_else(default_crash_log_path)
}

struct ReentrancyGuard;

impl ReentrancyGuard {
    fn enter() -> Option<Self> {
        IN_PANIC_HOOK.with(|active| {
            if active.replace(true) {
                None
            } else {
                Some(Self)
            }
        })
    }
}

impl Drop for ReentrancyGuard {
    fn drop(&mut self) {
        IN_PANIC_HOOK.with(|active| active.set(false));
    }
}

/// Install the process-wide panic hook that writes to `<data_dir>/crash.log`,
/// preserves stderr output, and shows a native error dialog in interactive runs.
pub(crate) fn install() {
    let previous = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        let Some(_guard) = ReentrancyGuard::enter() else {
            previous(info);
            return;
        };

        let message = payload_message(info.payload());
        let saved_path = record_panic(info, message);
        remember_saved_log_path(saved_path.clone());

        // Preserve default stderr panic output for terminal runs and test harnesses.
        previous(info);

        // A guarded panic is shown by the editor page; the one-shot dialog stays for others.
        if should_show_dialog() && !DIALOG_SHOWN.swap(true, Ordering::SeqCst) {
            let _ = panic::catch_unwind(|| {
                show_crash_dialog(saved_path.as_deref(), message);
            });
        }
    }));
}

fn record_panic(info: &PanicHookInfo<'_>, message: &str) -> Option<PathBuf> {
    let timestamp = format_utc_timestamp(SystemTime::now());
    let version = env!("CARGO_PKG_VERSION");
    let thread_name = std::thread::current()
        .name()
        .unwrap_or("<unnamed>")
        .to_string();
    let location = info
        .location()
        .map(|loc| format!("{}:{}:{}", loc.file(), loc.line(), loc.column()));
    let backtrace = Backtrace::force_capture().to_string();

    let entry = format_crash_entry(
        &timestamp,
        version,
        &thread_name,
        message,
        location.as_deref(),
        &backtrace,
    );

    let log_path = default_crash_log_path();
    let _lock = LOG_LOCK.lock().unwrap_or_else(|err| err.into_inner());
    match write_crash_log_with_limit(&log_path, &entry, MAX_CRASH_LOG_BYTES) {
        Ok(()) => Some(log_path),
        Err(error) => {
            let _ = writeln!(
                std::io::stderr(),
                "dawsu!: failed to write crash log at {}: {error}",
                log_path.display()
            );
            None
        }
    }
}

fn default_crash_log_path() -> PathBuf {
    dawsu_application::data_folder().join("crash.log")
}

fn payload_message(payload: &(dyn Any + Send)) -> &str {
    if let Some(s) = payload.downcast_ref::<&str>() {
        s
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.as_str()
    } else {
        "<non-string panic payload>"
    }
}

pub(crate) fn format_utc_timestamp(now: SystemTime) -> String {
    let Ok(duration) = now.duration_since(UNIX_EPOCH) else {
        return "1970-01-01T00:00:00Z".to_string();
    };
    let secs = duration.as_secs();
    let days = (secs / 86_400) as i64;
    let secs_of_day = secs % 86_400;
    let hour = secs_of_day / 3_600;
    let minute = (secs_of_day % 3_600) / 60;
    let second = secs_of_day % 60;

    // Howard Hinnant's civil-from-days algorithm (days since 1970-01-01).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };

    format!("{year:04}-{m:02}-{d:02}T{hour:02}:{minute:02}:{second:02}Z")
}

pub(crate) fn format_crash_entry(
    timestamp: &str,
    version: &str,
    thread: &str,
    message: &str,
    location: Option<&str>,
    backtrace: &str,
) -> String {
    let location = location.unwrap_or("<unknown>");
    let backtrace = backtrace.trim_end();
    format!(
        "=== dawsu! crash report ===\n\
         timestamp: {timestamp}\n\
         version:   {version}\n\
         thread:    {thread}\n\
         location:  {location}\n\
         message:   {message}\n\
         backtrace:\n\
         {backtrace}\n\n"
    )
}

fn rotated_backup_path(log_path: &Path) -> PathBuf {
    let mut name = log_path
        .file_name()
        .map(OsString::from)
        .unwrap_or_else(|| OsString::from("crash.log"));
    name.push(".1");
    log_path.with_file_name(name)
}

pub(crate) fn write_crash_log_with_limit(
    log_path: &Path,
    entry: &str,
    max_bytes: u64,
) -> std::io::Result<()> {
    if let Some(parent) = log_path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }

    let existing_size = fs::metadata(log_path).map(|meta| meta.len()).unwrap_or(0);
    let mut truncate_in_place = false;

    if existing_size > 0 && existing_size.saturating_add(entry.len() as u64) > max_bytes {
        let backup = rotated_backup_path(log_path);
        let _ = fs::remove_file(&backup);
        if fs::rename(log_path, &backup).is_err() {
            truncate_in_place = true;
        }
    }

    let mut file = if truncate_in_place {
        OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(log_path)?
    } else {
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_path)?
    };

    file.write_all(entry.as_bytes())?;
    file.flush()
}

fn is_truthy_env(value: Option<OsString>) -> bool {
    let Some(raw) = value else {
        return false;
    };
    let text = raw.to_string_lossy();
    let trimmed = text.trim();
    !trimmed.is_empty()
        && !trimmed.eq_ignore_ascii_case("0")
        && !trimmed.eq_ignore_ascii_case("false")
}

fn should_show_dialog() -> bool {
    if cfg!(test) {
        return false;
    }
    should_show_dialog_for(is_guarded(), |key| std::env::var_os(key))
}

/// Whether a panic shows the native dialog: never inside `guarded`, else as the environment says.
fn should_show_dialog_for<F>(guarded: bool, get_env: F) -> bool
where
    F: Fn(&str) -> Option<OsString>,
{
    !guarded && should_show_dialog_from_env(get_env)
}

fn should_show_dialog_from_env<F>(get_env: F) -> bool
where
    F: Fn(&str) -> Option<OsString>,
{
    if is_truthy_env(get_env("DAWSU_NO_CRASH_DIALOG"))
        || is_truthy_env(get_env("DAWSU_SMOKE_WINDOW"))
        || is_truthy_env(get_env("CI"))
        || get_env("DAWSU_RECOVERY_SMOKE_DATA").is_some_and(|v| !v.is_empty())
        || get_env("RUST_TEST_THREADS").is_some_and(|v| !v.is_empty())
        || get_env("NEXTEST").is_some_and(|v| !v.is_empty())
    {
        return false;
    }

    if let Some(args) = get_env("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS")
        && args.to_string_lossy().contains("--remote-debugging-port")
    {
        return false;
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let has_display = get_env("DISPLAY").is_some_and(|v| !v.is_empty())
            || get_env("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty());
        if !has_display {
            return false;
        }
    }

    true
}

fn truncate_summary(message: &str) -> String {
    let mut chars = message.chars();
    let truncated: String = chars.by_ref().take(MAX_DIALOG_MESSAGE_CHARS).collect();
    if chars.next().is_some() {
        format!("{truncated}...")
    } else {
        truncated
    }
}

fn show_crash_dialog(saved_path: Option<&Path>, message: &str) {
    let summary = truncate_summary(message);
    let description = match saved_path {
        Some(path) => format!(
            "dawsu! ran into an unexpected error and needs to close.\n\n\
             A crash report was saved to:\n{}\n\n\
             Error: {}",
            path.display(),
            summary
        ),
        None => format!(
            "dawsu! ran into an unexpected error and needs to close.\n\n\
             Error: {}",
            summary
        ),
    };

    rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title("dawsu! crashed unexpectedly")
        .set_description(&description)
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::time::Duration;

    #[test]
    fn formats_utc_timestamp_for_epoch_and_leap_day() {
        assert_eq!(format_utc_timestamp(UNIX_EPOCH), "1970-01-01T00:00:00Z");
        // 2024-02-29T12:34:56Z = 1709210096 seconds since Unix epoch.
        let leap = UNIX_EPOCH + Duration::from_secs(1_709_210_096);
        assert_eq!(format_utc_timestamp(leap), "2024-02-29T12:34:56Z");
    }

    #[test]
    fn formats_crash_entry_with_fields_and_fallback_location() {
        let entry = format_crash_entry(
            "2026-11-24T20:00:00Z",
            "0.1.0",
            "editor-worker",
            "index out of bounds",
            Some("apps/tauri/src/main.rs:42:5"),
            "   0: dawsu_tauri::boom\n   1: std::panicking::begin_panic\n\n",
        );

        assert!(entry.contains("=== dawsu! crash report ==="));
        assert!(entry.contains("timestamp: 2026-11-24T20:00:00Z"));
        assert!(entry.contains("version:   0.1.0"));
        assert!(entry.contains("thread:    editor-worker"));
        assert!(entry.contains("location:  apps/tauri/src/main.rs:42:5"));
        assert!(entry.contains("message:   index out of bounds"));
        assert!(entry.contains("0: dawsu_tauri::boom"));
        assert!(entry.ends_with("\n\n"));

        let without_loc = format_crash_entry(
            "2026-11-24T20:00:00Z",
            "0.1.0",
            "<unnamed>",
            "boom",
            None,
            "disabled backtrace",
        );
        assert!(without_loc.contains("location:  <unknown>"));
    }

    #[test]
    fn writes_and_rotates_crash_log_in_temp_dir() {
        let dir = std::env::temp_dir().join(format!(
            "dawsu-crash-log-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let log_path = dir.join("nested").join("crash.log");
        let backup_path = dir.join("nested").join("crash.log.1");

        let first = format_crash_entry(
            "2026-11-24T20:00:01Z",
            "0.1.0",
            "main",
            "first panic",
            Some("a.rs:1:1"),
            "frame0",
        );
        let second = format_crash_entry(
            "2026-11-24T20:00:02Z",
            "0.1.0",
            "audio",
            "second panic",
            Some("b.rs:2:2"),
            "frame1",
        );
        let third = format_crash_entry(
            "2026-11-24T20:00:03Z",
            "0.1.0",
            "editor",
            "third panic",
            Some("c.rs:3:3"),
            "frame2",
        );

        // Allow first + second in the active log, then trigger rotation on third.
        let limit = (first.len() + second.len() + 10) as u64;

        write_crash_log_with_limit(&log_path, &first, limit).expect("write first entry");
        write_crash_log_with_limit(&log_path, &second, limit).expect("write second entry");

        let before_rotate = fs::read_to_string(&log_path).expect("read active log");
        assert!(before_rotate.contains("first panic"));
        assert!(before_rotate.contains("second panic"));
        assert!(!backup_path.exists());

        write_crash_log_with_limit(&log_path, &third, limit).expect("rotate and write third entry");

        let after_rotate = fs::read_to_string(&log_path).expect("read rotated active log");
        let rotated_backup = fs::read_to_string(&backup_path).expect("read backup log");
        assert!(!after_rotate.contains("first panic"));
        assert!(after_rotate.contains("third panic"));
        assert!(rotated_backup.contains("first panic"));
        assert!(rotated_backup.contains("second panic"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn guarded_marks_the_thread_until_it_returns_or_unwinds() {
        assert!(!is_guarded());
        let inside = guarded(|| {
            assert!(guarded(is_guarded), "nested calls stay guarded");
            is_guarded()
        });
        assert!(inside);
        assert!(!is_guarded(), "cleared after a normal return");
        let unwound = std::panic::catch_unwind(|| guarded(|| panic!("guarded test panic")));
        assert!(unwound.is_err());
        assert!(!is_guarded(), "cleared after an unwind");
    }

    #[test]
    fn a_guarded_panic_shows_no_native_dialog() {
        let interactive = |key: &str| (key == "DISPLAY").then(|| OsString::from(":0"));
        assert!(should_show_dialog_for(false, interactive));
        assert!(!should_show_dialog_for(true, interactive));
        assert!(!should_show_dialog_for(false, |key: &str| (key == "CI").then(|| OsString::from("1"))));
    }

    #[test]
    fn crash_log_path_names_the_report_a_panic_on_this_thread_saved() {
        let saved = std::thread::spawn(|| {
            assert_eq!(crash_log_path(), default_crash_log_path());
            remember_saved_log_path(Some(PathBuf::from("E:/elsewhere/crash.log")));
            crash_log_path()
        })
        .join()
        .unwrap();
        assert_eq!(saved, PathBuf::from("E:/elsewhere/crash.log"));
        assert!(crash_log_path().ends_with("crash.log"));
    }

    #[test]
    fn suppresses_dialog_in_headless_and_automated_envs() {
        let env_from = |pairs: &[(&str, &str)]| {
            let map: HashMap<String, OsString> = pairs
                .iter()
                .map(|(k, v)| ((*k).to_string(), OsString::from(v)))
                .collect();
            move |key: &str| map.get(key).cloned()
        };

        assert!(!should_show_dialog(), "tests must never pop a crash dialog");
        assert!(!should_show_dialog_from_env(env_from(&[(
            "DAWSU_NO_CRASH_DIALOG",
            "1"
        )])));
        assert!(!should_show_dialog_from_env(env_from(&[(
            "DAWSU_SMOKE_WINDOW",
            "1"
        )])));
        assert!(!should_show_dialog_from_env(env_from(&[("CI", "true")])));
        assert!(!should_show_dialog_from_env(env_from(&[(
            "DAWSU_RECOVERY_SMOKE_DATA",
            "/tmp/smoke"
        )])));
        assert!(!should_show_dialog_from_env(env_from(&[(
            "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS",
            "--remote-debugging-port=9222"
        )])));

        // Interactive env (with DISPLAY set so this assertion also holds on Linux).
        assert!(should_show_dialog_from_env(env_from(&[
            ("DAWSU_NO_CRASH_DIALOG", "0"),
            ("DISPLAY", ":0"),
        ])));
    }
}
