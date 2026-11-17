use dawsu_foundation::{CustomIndex, SampleIdentity, SampleSet};
use dawsu_foundation::{
    digest::sha256_hex,
    safe_write::{self, BackupPolicy, WriteOutcome},
};
use dawsu_hitsounding::rules::read_sample_file_name;
use dawsu_osu_format::check_map_path;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::{
    fs::{self, File},
    io::{self, Read},
    path::{Path, PathBuf},
};

const VERSION: u32 = 1;
const MAX_PROJECT_BYTES: u64 = 192 * 1024 * 1024;
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bookmarks: Option<Vec<ProjectBookmark>>,
    pub unanchored: Option<Vec<ProjectHit>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lane_layout: Option<LaneLayout>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lane_slots: Option<BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hs_mode: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_signatures: Option<Vec<super::EditorTimeSignature>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub export_settings: Option<super::ExportSettings>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LaneLayout {
    pub folder_order: Vec<String>,
    pub collapsed_folders: BTreeSet<String>,
    pub collapsed_families: BTreeSet<String>,
}

const SAMPLE_FOLDERS: [&str; 3] = ["normal", "soft", "drum"];

impl LaneLayout {
    pub(crate) fn normalise(&mut self) {
        let mut seen = BTreeSet::new();
        self.folder_order
            .retain(|group| SAMPLE_FOLDERS.contains(&group.as_str()) && seen.insert(group.clone()));
        self.folder_order.extend(
            SAMPLE_FOLDERS
                .into_iter()
                .filter(|group| !seen.contains(*group))
                .map(str::to_string),
        );
        self.collapsed_folders
            .retain(|group| SAMPLE_FOLDERS.contains(&group.as_str()));
        self.collapsed_families = self
            .collapsed_families
            .iter()
            .filter_map(|family| {
                lane_identity(&format!("Skin|{family}")).map(|lane| super::lane_name(&lane))
            })
            .collect();
    }
}

pub(crate) fn lane_key(lane: &SampleIdentity) -> String {
    match lane {
        SampleIdentity::File(name) => format!("File|{name}"),
        SampleIdentity::Standard { index, .. } if index.get() == 0 => {
            format!("Skin|{}", super::lane_name(lane))
        }
        _ => format!("Standard|{}.wav", super::lane_name(lane)),
    }
}

pub(crate) fn lane_identity(key: &str) -> Option<SampleIdentity> {
    let (kind, name) = key.split_once('|')?;
    match kind {
        "Skin" => match read_sample_file_name(&format!("{name}.wav"))? {
            SampleIdentity::Standard { set, sound, .. } => Some(SampleIdentity::Standard {
                set,
                sound,
                index: CustomIndex::new(0),
            }),
            _ => None,
        },
        "BeatmapFile" if check_map_path(name).is_ok() => {
            Some(read_sample_file_name(name).unwrap_or_else(|| SampleIdentity::File(name.into())))
        }
        "File" if check_map_path(name).is_ok() => Some(SampleIdentity::File(name.into())),
        "Standard" if check_map_path(name).is_ok() => read_sample_file_name(name),
        _ => None,
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct ProjectBookmark {
    pub time: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub(crate) struct ProjectHit {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<usize>,
    pub lane: String,
    pub time: f64,
    pub volume: Option<i32>,
}

#[derive(Serialize, Deserialize)]
struct FileData {
    version: u32,
    settings: SessionSettings,
}

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
struct LegacyProject {
    version: u32,
    lane_order: Vec<String>,
    collapsed_families: Vec<String>,
    lane_slots: Option<BTreeMap<String, String>>,
    unanchored: Vec<ProjectHit>,
    locator_left: Option<f64>,
    locator_right: Option<f64>,
    cycle: bool,
    song_offset: f64,
    hs_mode: Option<LegacyHsMode>,
    time_signatures: Option<Vec<LegacyTimeSignature>>,
    export_settings: Option<serde_json::Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct LegacyHsMode {
    enabled: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct LegacyTimeSignature {
    bar: i32,
    numerator: i32,
    denominator: i32,
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
            let mut settings = data.settings;
            if let Some(root) = legacy_root
                && (settings.unanchored.is_none()
                    || settings.lane_layout.is_none()
                    || settings.lane_slots.is_none()
                    || settings.hs_mode.is_none()
                    || settings.time_signatures.is_none()
                    || settings.export_settings.is_none())
            {
                let legacy = legacy_settings(root, map);
                // The older sidecar was already optional once a native project saved its hits.
                // A malformed copy should not block opening the native project for new view metadata.
                let legacy = if settings.unanchored.is_none() {
                    legacy?
                } else {
                    legacy.ok().flatten()
                };
                if let Some(legacy) = legacy {
                    settings.unanchored = settings.unanchored.or(legacy.unanchored);
                    settings.lane_layout = settings.lane_layout.or(legacy.lane_layout);
                    settings.lane_slots = settings.lane_slots.or(legacy.lane_slots);
                    settings.hs_mode = settings.hs_mode.or(legacy.hs_mode);
                    settings.time_signatures = settings.time_signatures.or(legacy.time_signatures);
                    settings.export_settings = settings.export_settings.or(legacy.export_settings);
                }
            }
            settings
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
        if bytes.len() as u64 > MAX_PROJECT_BYTES {
            return Err("Session exceeds the supported size limit".into());
        }
        let folder = self.path.parent().expect("project path has a folder");
        fs::create_dir_all(folder).map_err(|e| e.to_string())?;
        if let Some(expected) = &self.expected {
            match safe_write::replace(&self.path, expected, &bytes, &BackupPolicy::new(folder.join("backups"), 5))
                .map_err(|e| e.to_string())? {
                WriteOutcome::Saved { .. } => {},
                WriteOutcome::ChangedOnDisk => return Err("The session file changed on disk. Use Revert to last saved to discard local changes and load the latest settings.".into()),
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

pub(super) fn path_digest(path: &Path) -> String {
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
    let mut layout = LaneLayout::default();
    if legacy.version >= 3 {
        for key in &legacy.lane_order {
            let slot = legacy
                .lane_slots
                .as_ref()
                .and_then(|slots| slots.get(key))
                .unwrap_or(key);
            if let Some(SampleIdentity::Standard { set, .. }) = lane_identity(slot) {
                layout.folder_order.push(
                    match set {
                        SampleSet::Normal => "normal",
                        SampleSet::Soft => "soft",
                        SampleSet::Drum => "drum",
                    }
                    .into(),
                );
            }
        }
    }
    layout.collapsed_families = legacy
        .collapsed_families
        .iter()
        .filter_map(|family| {
            let (set, sound) = family.split_once(':')?;
            Some(format!(
                "{}-{}",
                set.to_ascii_lowercase(),
                sound.to_ascii_lowercase()
            ))
        })
        .collect();
    let lane_layout = if layout == LaneLayout::default() {
        None
    } else {
        layout.normalise();
        Some(layout)
    };
    Ok(Some(SessionSettings {
        unanchored: Some(legacy.unanchored),
        lane_layout,
        lane_slots: legacy.lane_slots,
        hs_mode: legacy.hs_mode.map(|state| state.enabled),
        export_settings: legacy.export_settings
            .and_then(|value| serde_json::from_value::<super::ExportSettings>(value).ok())
            .filter(|settings| settings.validate().is_ok()),
        time_signatures: legacy.time_signatures.map(|signatures| signatures.into_iter().filter_map(|signature| {
            let signature = super::EditorTimeSignature {
                bar: signature.bar.try_into().ok()?,
                numerator: signature.numerator.try_into().ok()?,
                denominator: signature.denominator.try_into().ok()?,
            };
            signature.valid().then_some(signature)
        }).collect()),
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
        let contents = br#"{"Version":3,"LocatorLeft":21,"LocatorRight":400,"SongOffset":-3.5,"Cycle":true,"Unanchored":[{"Lane":"Skin|soft-hitclap","Time":250}],"LaneOrder":["BeatmapFile|soft-hitclap2.wav","Skin|normal-hitnormal"],"CollapsedFamilies":["Soft:hitclap"],"LaneSlots":{"BeatmapFile|replacement.wav":"BeatmapFile|soft-hitclap2.wav"},"HsMode":{"Enabled":true},"TimeSignatures":[{"Bar":2,"Numerator":7,"Denominator":8}]}"#;
        fs::write(&legacy, contents).unwrap();
        let mut project = Project::load(&project_root, Some(&legacy_root), map).unwrap();
        assert_eq!(project.settings.song_offset, -3.5);
        assert_eq!(project.settings.hs_mode, Some(true));
        assert_eq!(project.settings.time_signatures.as_deref(), Some(&[super::super::EditorTimeSignature { bar: 2, numerator: 7, denominator: 8 }][..]));
        let layout = project.settings.lane_layout.as_ref().unwrap();
        assert_eq!(layout.folder_order, ["soft", "normal", "drum"]);
        assert!(layout.collapsed_families.contains("soft-hitclap"));
        assert_eq!(
            project
                .settings
                .lane_slots
                .as_ref()
                .unwrap()
                .get("BeatmapFile|replacement.wav")
                .unwrap(),
            "BeatmapFile|soft-hitclap2.wav",
        );
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
