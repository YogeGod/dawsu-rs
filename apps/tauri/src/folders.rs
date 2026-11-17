use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Folder {
    Map,
    Library,
    Project,
}

impl Folder {
    pub(super) fn resolve(
        self, map: Option<&Path>, library: Option<&Path>, data: &Path,
    ) -> Result<PathBuf, String> {
        match self {
            Self::Map => map
                .and_then(Path::parent)
                .map(Path::to_path_buf)
                .ok_or_else(|| "Open a difficulty first.".into()),
            Self::Project => Ok(data.join("projects")),
            Self::Library => library.map(Path::to_path_buf)
                .ok_or_else(|| "Choose a hitsound library folder first.".into()),
        }
    }

    pub(super) fn open(self, folder: &Path) -> Result<(), String> {
        if matches!(self, Self::Project) {
            std::fs::create_dir_all(folder).map_err(|error| {
                format!("Creating project folder {}: {error}", folder.display())
            })?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_targets_are_derived_from_editor_state() {
        let map = Path::new("Songs/Mapset/Hard.osu");
        let data = Path::new("EditorData");
        assert_eq!(
            Folder::Map.resolve(Some(map), None, data).unwrap(),
            Path::new("Songs/Mapset")
        );
        assert!(Folder::Map.resolve(None, None, data).is_err());
        assert!(Folder::Library.resolve(None, None, data).is_err());
        let library = Path::new("SampleLibrary");
        assert_eq!(Folder::Library.resolve(None, Some(library), data).unwrap(), library);
        assert_eq!(
            Folder::Project.resolve(Some(map), None, data).unwrap(),
            Path::new("EditorData/projects")
        );
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
