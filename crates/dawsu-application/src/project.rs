use dawsu_foundation::{
    digest::sha256_hex,
    safe_write::{self, BackupPolicy, WriteOutcome},
};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::{self, Read},
    path::{Path, PathBuf},
};

const VERSION: u32 = 1;
const MAX_PROJECT_BYTES: u64 = 64 * 1024;
const MAX_LEGACY_BYTES: u64 = 192 * 1024 * 1024;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct SessionSettings {
    pub song_offset: f64,
    pub locator_left: f64,
    pub locator_right: Option<f64>,
    pub cycle: bool,
    pub zoom: Option<f64>,
    pub row_height: Option<f32>,
    pub view_start: f64,
}

#[derive(Serialize, Deserialize)]
struct FileData {
    version: u32,
    settings: SessionSettings,
}

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
struct LegacyProject {
    locator_left: Option<f64>,
    locator_right: Option<f64>,
    cycle: bool,
    song_offset: f64,
}

pub(crate) struct Project {
    path: PathBuf,
    expected: Option<Vec<u8>>,
    pub settings: SessionSettings,
}

impl Project {
    pub fn load(root: &Path, legacy_root: Option<&Path>, map: &Path) -> Result<Self, String> {
        let path = root.join(format!("{}.json", path_digest(map)));
        let expected = read_bounded(&path, MAX_PROJECT_BYTES)?;
        let settings = if let Some(bytes) = &expected {
            let data: FileData = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
            if data.version != VERSION {
                return Err(format!("Unsupported session file version {}", data.version));
            }
            data.settings
        } else if let Some(root) = legacy_root {
            legacy_settings(root, map)?.unwrap_or_default()
        } else {
            SessionSettings::default()
        };
        Ok(Self {
            path,
            expected,
            settings,
        })
    }

    pub fn save(&mut self, settings: SessionSettings) -> Result<(), String> {
        if self.settings == settings {
            return Ok(());
        }
        let bytes = serde_json::to_vec_pretty(&FileData {
            version: VERSION,
            settings: settings.clone(),
        })
        .map_err(|e| e.to_string())?;
        let folder = self.path.parent().expect("project path has a folder");
        fs::create_dir_all(folder).map_err(|e| e.to_string())?;
        if let Some(expected) = &self.expected {
            match safe_write::replace(&self.path, expected, &bytes, &BackupPolicy::new(folder.join("backups"), 5))
                .map_err(|e| e.to_string())? {
                WriteOutcome::Saved { .. } => {},
                WriteOutcome::ChangedOnDisk => return Err("The session file changed on disk. Reopen the difficulty to load the latest settings.".into()),
            }
        } else {
            safe_write::create(&self.path, &bytes).map_err(|e| e.to_string())?;
        }
        self.expected = Some(bytes);
        self.settings = settings;
        Ok(())
    }
}

fn read_bounded(path: &Path, limit: u64) -> Result<Option<Vec<u8>>, String> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > limit {
        return Err(format!(
            "Session file exceeds the size limit: {}",
            path.display()
        ));
    }
    Ok(Some(bytes))
}

fn legacy_path_key(path: &Path) -> String {
    let path = path.to_string_lossy();
    let normalised = if let Some(unc) = path.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{unc}")
    } else {
        path.strip_prefix(r"\\?\").unwrap_or(&path).to_owned()
    };
    normalised.to_lowercase()
}

fn path_digest(path: &Path) -> String {
    if cfg!(windows) {
        sha256_hex(legacy_path_key(path).as_bytes())
    } else {
        sha256_hex(path.as_os_str().as_encoded_bytes())
    }
}

fn legacy_settings(root: &Path, map: &Path) -> Result<Option<SessionSettings>, String> {
    let digest = sha256_hex(legacy_path_key(map).as_bytes());
    let suffix = format!("-{}.json", digest[..12].to_uppercase());
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    let mut found = None;
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        if entry.file_name().to_string_lossy().ends_with(&suffix) {
            if found.is_some() {
                return Err("More than one legacy project matches this difficulty".into());
            }
            found = Some(entry.path());
        }
    }
    let Some(path) = found else {
        return Ok(None);
    };
    let Some(bytes) = read_bounded(&path, MAX_LEGACY_BYTES)? else {
        return Ok(None);
    };
    let legacy: LegacyProject = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    Ok(Some(SessionSettings {
        song_offset: legacy.song_offset,
        locator_left: legacy.locator_left.unwrap_or_default(),
        locator_right: legacy.locator_right,
        cycle: legacy.cycle,
        ..SessionSettings::default()
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawsu_test_support::Scratch;

    #[test]
    fn legacy_session_is_read_then_saved_separately() {
        let scratch = Scratch::new("legacy-project");
        let legacy_root = scratch.path("legacy");
        let project_root = scratch.path("current");
        let map = Path::new(r"\\?\E:\Maps\Song\Hard.osu");
        let expected_key = r"e:\maps\song\hard.osu";
        let hash = sha256_hex(expected_key.as_bytes());
        let legacy = legacy_root.join(format!("Hard-{}.json", hash[..12].to_uppercase()));
        fs::create_dir(&legacy_root).unwrap();
        let contents = br#"{"Version":3,"LocatorLeft":21,"LocatorRight":400,"SongOffset":-3.5,"Cycle":true,"Unanchored":[{"Lane":"Skin|soft-hitclap","Time":250}]}"#;
        fs::write(&legacy, contents).unwrap();
        let mut project = Project::load(&project_root, Some(&legacy_root), map).unwrap();
        assert_eq!(project.settings.song_offset, -3.5);
        assert_eq!(project.settings.locator_left, 21.0);
        assert_eq!(project.settings.locator_right, Some(400.0));
        assert!(project.settings.cycle);
        let settings = SessionSettings {
            song_offset: 9.5,
            ..project.settings.clone()
        };
        project.save(settings.clone()).unwrap();
        assert_eq!(fs::read(legacy).unwrap(), contents);
        assert_eq!(
            Project::load(&project_root, Some(&legacy_root), map)
                .unwrap()
                .settings,
            settings
        );
    }

    #[test]
    fn stale_sessions_cannot_overwrite_current_settings() {
        let scratch = Scratch::new("project-conflict");
        let map = scratch.path("map.osu");
        let mut first = Project::load(scratch.dir(), None, &map).unwrap();
        let mut stale_create = Project::load(scratch.dir(), None, &map).unwrap();
        first
            .save(SessionSettings {
                song_offset: 5.0,
                ..SessionSettings::default()
            })
            .unwrap();
        assert!(
            stale_create
                .save(SessionSettings {
                    song_offset: 10.0,
                    ..SessionSettings::default()
                })
                .is_err()
        );
        let mut stale_update = Project::load(scratch.dir(), None, &map).unwrap();
        first
            .save(SessionSettings {
                song_offset: 15.0,
                ..SessionSettings::default()
            })
            .unwrap();
        assert!(
            stale_update
                .save(SessionSettings {
                    song_offset: -5.0,
                    ..SessionSettings::default()
                })
                .is_err()
        );
        assert_eq!(
            Project::load(scratch.dir(), None, &map)
                .unwrap()
                .settings
                .song_offset,
            15.0
        );
    }
}
