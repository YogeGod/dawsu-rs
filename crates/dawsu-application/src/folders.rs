use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Folder {
    Map,
    Library,
    Project,
    CrashLogs,
    /// The folder the last hitsound-only Export wrote.
    LastExport,
}

impl Folder {
    pub fn resolve(
        self,
        map: Option<&Path>,
        library: Option<&Path>,
        data: &Path,
        last_export: Option<&Path>,
    ) -> Result<PathBuf, String> {
        match self {
            Self::Map => map
                .and_then(Path::parent)
                .map(Path::to_path_buf)
                .ok_or_else(|| "Open a difficulty first.".into()),
            Self::Project => Ok(data.join("projects")),
            Self::Library => library
                .map(Path::to_path_buf)
                .ok_or_else(|| "Choose a hitsound library folder first.".into()),
            Self::CrashLogs => Ok(data.to_path_buf()),
            Self::LastExport => last_export
                .map(Path::to_path_buf)
                .ok_or_else(|| "Export a hitsound-only folder first.".into()),
        }
    }

    pub fn open(self, folder: &Path) -> Result<(), String> {
        if let Some(label) = match self {
            Self::Project => Some("project folder"),
            Self::CrashLogs => Some("crash log folder"),
            Self::Map | Self::Library | Self::LastExport => None,
        } {
            std::fs::create_dir_all(folder)
                .map_err(|error| format!("Creating {label} {}: {error}", folder.display()))?;
        }
        if !folder.is_dir() {
            return Err(format!("Folder is unavailable: {}", folder.display()));
        }

        #[cfg(target_os = "windows")]
        let (program, argument) = ("explorer.exe", explorer_path(folder));
        #[cfg(target_os = "macos")]
        let (program, argument) = ("open", folder.into());
        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        let (program, argument) = ("xdg-open", folder.into());

        Command::new(program)
            .arg(argument)
            .spawn()
            .map(|_| ())
            .map_err(|error| format!("Opening folder {}: {error}", folder.display()))
    }
}

#[cfg(target_os = "windows")]
fn explorer_path(folder: &Path) -> PathBuf {
    let Some(path) = folder.to_str() else {
        return folder.into();
    };
    if let Some(unc) = path.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{unc}").into();
    }
    // Document::read canonicalizes to a verbatim drive path; Explorer expects a regular drive path.
    if let Some(drive) = path.strip_prefix(r"\\?\")
        && drive.as_bytes().get(1) == Some(&b':')
    {
        return drive.into();
    }
    folder.into()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HelpLink {
    Wiki,
    Faq,
    BugReport,
}

impl HelpLink {
    pub const fn url(self) -> &'static str {
        match self {
            Self::Wiki => "https://github.com/YogeGod/dawsu-rs/wiki",
            Self::Faq => "https://github.com/YogeGod/dawsu-rs/blob/main/docs/FAQ.md",
            Self::BugReport => "https://github.com/YogeGod/dawsu-rs/issues/new",
        }
    }
}

pub fn validate_external_url(url: &str) -> Result<(), String> {
    let rest = url
        .strip_prefix("https://")
        .ok_or_else(|| format!("External links must use https://: {url}"))?;
    if rest.is_empty() || url.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err(format!("Invalid external link URL: {url}"));
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn to_wide_null(value: &str) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    std::ffi::OsStr::new(value)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

/// Opens a validated `https://` URL in the OS default browser.
///
/// # Platform Contract (Windows)
/// On Windows, `ShellExecuteW` delegates to Shell handlers that expect STA COM initialization, so
/// callers must invoke this synchronously on the shell's main UI thread (STA).
pub fn open_external_url(url: &str) -> Result<(), String> {
    validate_external_url(url)?;

    #[cfg(target_os = "windows")]
    {
        #[link(name = "shell32")]
        unsafe extern "system" {
            fn ShellExecuteW(
                hwnd: *mut core::ffi::c_void,
                lp_operation: *const u16,
                lp_file: *const u16,
                lp_parameters: *const u16,
                lp_directory: *const u16,
                n_show_cmd: i32,
            ) -> isize;
        }
        const SW_SHOWNORMAL: i32 = 1;
        let verb = to_wide_null("open");
        let target = to_wide_null(url);
        let result = unsafe {
            ShellExecuteW(
                core::ptr::null_mut(),
                verb.as_ptr(),
                target.as_ptr(),
                core::ptr::null(),
                core::ptr::null(),
                SW_SHOWNORMAL,
            )
        };
        if result <= 32 {
            return Err(format!("Opening {url} failed with ShellExecuteW code {result}."));
        }
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg(url)
            .spawn()
            .map(|_| ())
            .map_err(|error| format!("Opening {url}: {error}"))
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        Command::new("xdg-open")
            .arg(url)
            .spawn()
            .map(|_| ())
            .map_err(|error| format!("Opening {url}: {error}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_targets_are_derived_from_editor_state() {
        let map = Path::new("Songs/Mapset/Hard.osu");
        let data = Path::new("EditorData");
        assert_eq!(
            Folder::Map.resolve(Some(map), None, data, None).unwrap(),
            Path::new("Songs/Mapset")
        );
        assert!(Folder::Map.resolve(None, None, data, None).is_err());
        assert!(Folder::Library.resolve(None, None, data, None).is_err());
        let library = Path::new("SampleLibrary");
        assert_eq!(
            Folder::Library.resolve(None, Some(library), data, None).unwrap(),
            library
        );
        assert_eq!(
            Folder::Project.resolve(Some(map), None, data, None).unwrap(),
            Path::new("EditorData/projects")
        );
        assert_eq!(Folder::CrashLogs.resolve(None, None, data, None).unwrap(), data);
    }

    #[test]
    fn folder_last_export_is_the_last_hitsound_only_folder_and_is_never_created() {
        let data = Path::new("EditorData");
        let out = Path::new("Exports/Mapset-hitsound-only");
        assert_eq!(
            serde_json::from_str::<Folder>("\"last_export\"").unwrap(),
            Folder::LastExport
        );
        assert_eq!(
            Folder::LastExport.resolve(None, None, data, None),
            Err("Export a hitsound-only folder first.".into())
        );
        assert_eq!(
            Folder::LastExport
                .resolve(None, None, data, Some(out))
                .unwrap(),
            out
        );
        let gone = std::env::temp_dir().join(format!(
            "dawsu-folders-last-export-gone-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&gone);
        assert!(Folder::LastExport.open(&gone).is_err());
        assert!(!gone.exists(), "a moved or deleted export folder is not made again");
    }

    #[test]
    fn help_links_are_allowlisted_and_non_https_urls_are_rejected() {
        assert_eq!(
            serde_json::from_str::<HelpLink>("\"wiki\"").unwrap().url(),
            "https://github.com/YogeGod/dawsu-rs/wiki"
        );
        assert_eq!(
            serde_json::from_str::<HelpLink>("\"faq\"").unwrap().url(),
            "https://github.com/YogeGod/dawsu-rs/blob/main/docs/FAQ.md"
        );
        assert_eq!(
            serde_json::from_str::<HelpLink>("\"bug_report\"")
                .unwrap()
                .url(),
            "https://github.com/YogeGod/dawsu-rs/issues/new"
        );
        for link in [HelpLink::Wiki, HelpLink::Faq, HelpLink::BugReport] {
            assert!(validate_external_url(link.url()).is_ok());
        }
        for rejected in [
            "http://github.com/YogeGod/dawsu-rs/wiki",
            "file:///C:/Windows/System32/cmd.exe",
            "javascript:alert(1)",
            "https://",
            "https://github.com/YogeGod/dawsu-rs/wiki with space",
            "https://github.com/YogeGod/dawsu-rs/wiki\r\n",
        ] {
            assert!(
                open_external_url(rejected).is_err(),
                "expected rejection for {rejected}"
            );
        }
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn explorer_uses_non_verbatim_drive_and_unc_paths() {
        for (raw, expected) in [
            (r"\\?\E:\Map Set", r"E:\Map Set"),
            (r"\\?\UNC\server\share\Map Set", r"\\server\share\Map Set"),
            (r"E:\Map Set", r"E:\Map Set"),
        ] {
            assert_eq!(explorer_path(Path::new(raw)), Path::new(expected));
        }
    }
}
