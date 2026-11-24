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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
    pub hidden_lanes: Option<BTreeSet<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hs_mode: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hs_circle_ownership: Option<HsCircleOwnership>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gain_recovery: Option<super::gain_recovery::Recovery>,
    /// The moments the last Save left out of the `.osu` (F052). Older builds ignore the field.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unwritten: Option<super::unwritten::Unwritten>,
    /// The sound set links of the last Save. Older builds ignore the field: the links are lost, the hits stay.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sound_sets: Option<super::sound_sets::Links>,
    /// The pending files the last Save left out of the mapset that its hits still play (mix
    /// sources, left-out moments, loose hits). Older builds ignore the field: those lanes play the
    /// skin's sound.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending_samples: Option<Vec<super::pending::PendingSample>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_signatures: Option<Vec<super::EditorTimeSignature>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub export_settings: Option<super::ExportSettings>,
    /// The kick and snare lanes drum hit detection last used. Older builds ignore the field.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub drum_lanes: Option<super::DrumLanes>,
    /// What the Export window last chose for this map. Older builds ignore the field.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub export_choices: Option<super::ExportChoices>,
    /// Output slot -> hitsound lane bindings for the M4 node graph. Older builds ignore the field.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_graph_bindings: Option<BTreeMap<String, String>>,
    /// Per-map Drum VST3 instrument selection, toggle, and saved preset state. Older builds ignore it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub drum_vst: Option<DrumVstSession>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DrumVstSession {
    pub plugin_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_uid: Option<String>,
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_b64: Option<String>,
}

impl Default for SessionSettings {
    fn default() -> Self {
        Self {
            song_offset: crate::DEFAULT_SONG_OFFSET_MS,
            locator_left: 0.0,
            locator_right: None,
            cycle: false,
            zoom: None,
            row_height: None,
            view_start: 0.0,
            bookmarks: None,
            unanchored: None,
            lane_layout: None,
            lane_slots: None,
            hidden_lanes: None,
            hs_mode: None,
            hs_circle_ownership: None,
            gain_recovery: None,
            unwritten: None,
            sound_sets: None,
            pending_samples: None,
            time_signatures: None,
            export_settings: None,
            drum_lanes: None,
            export_choices: None,
            node_graph_bindings: None,
            drum_vst: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct HsCircleOwnership {
    map_digest: String,
    object_ordinals: Vec<usize>,
}

impl HsCircleOwnership {
    pub(super) fn capture(
        file: &dawsu_osu_format::OsuFile,
        bytes: &[u8],
        owned: &BTreeSet<dawsu_osu_format::LineId>,
    ) -> Option<Self> {
        let object_ordinals = file.hit_objects().iter().enumerate()
            .filter_map(|(ordinal, object)| (owned.contains(&object.line()) && owned_circle(object)).then_some(ordinal))
            .collect::<Vec<_>>();
        (!object_ordinals.is_empty()).then(|| Self { map_digest: sha256_hex(bytes), object_ordinals })
    }

    pub(super) fn verified_lines(&self, file: &dawsu_osu_format::OsuFile) -> BTreeSet<dawsu_osu_format::LineId> {
        if !self.map_digest.eq_ignore_ascii_case(&sha256_hex(file.original_bytes())) {
            return BTreeSet::new();
        }
        let objects = file.hit_objects();
        self.object_ordinals.iter()
            .map(|ordinal| objects.get(*ordinal).filter(|object| owned_circle(object)).map(|object| object.line()))
            .collect::<Option<_>>().unwrap_or_default()
    }
}

fn owned_circle(object: &dawsu_osu_format::HitObject) -> bool {
    matches!(object.kind(), dawsu_osu_format::ObjectKind::Circle)
        && object.x() == 256.0 && object.y() == 192.0
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LaneLayout {
    pub folder_order: Vec<String>,
    pub collapsed_folders: BTreeSet<String>,
    pub collapsed_families: BTreeSet<String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub lane_colours: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub lane_names: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub lane_order: Vec<String>,
}

const SAMPLE_FOLDERS: [&str; 3] = ["normal", "soft", "drum"];

impl LaneLayout {
    pub(crate) fn normalise(&mut self) {
        self.lane_colours.retain(|key, colour| {
            lane_identity(key).is_some() && colour.len() == 7 && colour.starts_with('#')
                && colour.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit)
        });
        self.lane_names = std::mem::take(&mut self.lane_names).into_iter()
            .filter_map(|(key, name)| {
                let lane = lane_identity(&key)?;
                let name = name.trim();
                (!name.is_empty() && name.chars().count() <= 120 && !name.chars().any(char::is_control)
                    && name != super::lane_name(&lane)).then(|| (lane_key(&lane), name.to_owned()))
            })
            .collect();
        let mut ordered = BTreeSet::new();
        self.lane_order = self.lane_order.iter().filter_map(|key| {
            let key = lane_key(&lane_identity(key)?);
            ordered.insert(key.clone()).then_some(key)
        }).collect();
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

pub fn lane_key(lane: &SampleIdentity) -> String {
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_volume: Option<i32>,
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
    lane_names: BTreeMap<String, String>,
    collapsed_families: Vec<String>,
    lane_slots: Option<BTreeMap<String, String>>,
    hidden_lanes: Option<BTreeSet<String>>,
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
    file_hash: Option<String>,
    moments: Option<Vec<LegacyHsMoment>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct LegacyHsMoment {
    object_index: Option<i64>,
    node: Option<i64>,
    #[serde(default)]
    owned: bool,
}

impl LegacyHsMode {
    fn ownership(&self) -> Option<HsCircleOwnership> {
        let object_ordinals = self.moments.as_ref()?.iter()
            .filter(|moment| moment.owned && moment.node == Some(0))
            .filter_map(|moment| usize::try_from(moment.object_index?).ok())
            .collect::<Vec<_>>();
        if object_ordinals.is_empty() {
            return None;
        }
        Some(HsCircleOwnership { map_digest: self.file_hash.clone()?, object_ordinals })
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct LegacyTimeSignature {
    bar: i32,
    numerator: i32,
    denominator: i32,
}

#[derive(Clone)]
pub(crate) struct Project {
    path: PathBuf,
    expected: Option<Vec<u8>>,
    pub settings: SessionSettings,
    /// Sound set links the last open could not put back; each Save writes them again (M3).
    pub kept_links: Vec<super::sound_sets::PlacementRecord>,
    /// Library versions the mapper chose Keep for this session, by set id; each Save writes
    /// them on the set's placements without an edit to the document.
    pub accepted_versions: std::collections::BTreeMap<String, String>,
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
                    || settings.hidden_lanes.is_none()
                    || settings.hs_mode.is_none()
                    || settings.hs_circle_ownership.is_none()
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
                    settings.hidden_lanes = settings.hidden_lanes.or(legacy.hidden_lanes);
                    settings.hs_mode = settings.hs_mode.or(legacy.hs_mode);
                    settings.hs_circle_ownership = settings.hs_circle_ownership.or(legacy.hs_circle_ownership);
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
            kept_links: Vec::new(),
            accepted_versions: Default::default(),
        })
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    pub(super) fn check_current(&self) -> Result<(), String> {
        if read_bounded(&self.path, MAX_PROJECT_BYTES)? != self.expected {
            return Err("The session file changed on disk. Reopen the latest session before saving.".into());
        }
        Ok(())
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
        let slot_for = |key: &String| legacy.lane_slots.as_ref()
            .and_then(|slots| slots.get(key)).unwrap_or(key).clone();
        layout.lane_names = legacy.lane_names.iter()
            .map(|(key, name)| (slot_for(key), name.clone())).collect();
        layout.lane_order = legacy.lane_order.iter().map(slot_for).collect();
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
        hidden_lanes: legacy.hidden_lanes,
        hs_circle_ownership: legacy.hs_mode.as_ref().and_then(LegacyHsMode::ownership),
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
        let contents = br#"{"Version":3,"LocatorLeft":21,"LocatorRight":400,"SongOffset":-3.5,"Cycle":true,"Unanchored":[{"Lane":"Skin|soft-hitclap","Time":250}],"LaneOrder":["BeatmapFile|soft-hitclap2.wav","Skin|normal-hitnormal"],"CollapsedFamilies":["Soft:hitclap"],"HiddenLanes":["BeatmapFile|replacement.wav"],"LaneSlots":{"BeatmapFile|replacement.wav":"BeatmapFile|soft-hitclap2.wav"},"HsMode":{"Enabled":true},"TimeSignatures":[{"Bar":2,"Numerator":7,"Denominator":8}]}"#;
        fs::write(&legacy, contents).unwrap();
        let mut project = Project::load(&project_root, Some(&legacy_root), map).unwrap();
        assert_eq!(project.settings.song_offset, -3.5);
        assert_eq!(project.settings.hs_mode, Some(true));
        assert_eq!(project.settings.time_signatures.as_deref(), Some(&[super::super::EditorTimeSignature { bar: 2, numerator: 7, denominator: 8 }][..]));
        let layout = project.settings.lane_layout.as_ref().unwrap();
        assert_eq!(layout.folder_order, ["soft", "normal", "drum"]);
        assert!(layout.collapsed_families.contains("soft-hitclap"));
        assert_eq!(project.settings.hidden_lanes.as_ref().unwrap(), &BTreeSet::from(["BeatmapFile|replacement.wav".to_owned()]));
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
    fn legacy_hs_ownership_requires_the_exact_digest_and_valid_circle_ordinals() {
        use dawsu_osu_format::OsuFile;
        let bytes = b"osu file format v14\n[HitObjects]\n256,192,1000,1,8,0:0:0:0:\n256,192,1000,1,0,0:0:0:67:voice.wav\n128,192,2000,1,0,0:0:0:0:\n";
        let file = OsuFile::read(bytes.to_vec()).file;
        let legacy: LegacyHsMode = serde_json::from_value(serde_json::json!({
            "Enabled": true, "FileHash": sha256_hex(bytes).to_uppercase(),
            "Moments": [
                {"ObjectIndex":0,"Node":0,"Owned":false},
                {"ObjectIndex":1,"Node":0,"Owned":true},
                {"ObjectIndex":2,"Node":1,"Owned":true},
                {"ObjectIndex":-1,"Node":0,"Owned":true}
            ]
        })).unwrap();
        let ownership = legacy.ownership().unwrap();
        let objects = file.hit_objects();
        assert_eq!(ownership.verified_lines(&file), BTreeSet::from([objects[1].line()]));
        let mut mismatch = bytes.to_vec();
        mismatch.extend_from_slice(b"// same objects, different exact bytes\n");
        let changed = OsuFile::read(mismatch).file;
        assert!(ownership.verified_lines(&changed).is_empty());

        for ordinal in [2, 3] {
            let invalid = HsCircleOwnership {
                map_digest: sha256_hex(bytes),
                object_ordinals: vec![1, ordinal],
            };
            assert!(invalid.verified_lines(&file).is_empty());
        }
    }

    #[test]
    fn a_project_file_without_left_out_moments_loads_and_the_record_round_trips() {
        let scratch = Scratch::new("project-unwritten");
        let map = scratch.path("map.osu");
        let path = scratch.dir().join(format!("{}.json", path_digest(&map)));
        // A project an older build wrote has no `unwritten` field.
        fs::write(&path, br#"{"version":1,"settings":{"song_offset":3.0,"unanchored":null}}"#).unwrap();
        let mut project = Project::load(scratch.dir(), None, &map).unwrap();
        assert_eq!(project.settings.song_offset, 3.0);
        assert!(project.settings.unwritten.is_none());
        project.save(SessionSettings { song_offset: 4.0, ..project.settings.clone() }).unwrap();
        assert!(!String::from_utf8_lossy(&fs::read(&path).unwrap()).contains("unwritten"), "no record, no field");
        let unwritten: super::super::unwritten::Unwritten = serde_json::from_value(serde_json::json!({
            "current": {"map_digest": "abc", "moments": [{"object": 1, "node": 1, "time_bits": 0, "default_volume": 80,
                "hits": [{"lane": "Skin|soft-hitclap", "volume": 80}, {"lane": "Skin|drum-hitclap", "volume": 80}]}]},
            "pending": null, "before": null,
        })).unwrap();
        project.save(SessionSettings { unwritten: Some(unwritten.clone()), ..project.settings.clone() }).unwrap();
        let reloaded = Project::load(scratch.dir(), None, &map).unwrap();
        assert_eq!(reloaded.settings.unwritten, Some(unwritten));
        let file: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(file["version"], 1);
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

#[cfg(test)]
mod lane_layout_tests {
    use super::LaneLayout;
    use serde_json::json;

    #[test]
    fn lane_names_and_order_survive_layout_round_trip() {
        let mut layout: LaneLayout = serde_json::from_value(json!({
            "lane_names": {"Skin|normal-hitclap": "  Snare  ", "not a lane": "Invalid"},
            "lane_order": ["Skin|normal-hitclap", "Skin|normal-hitnormal", "Skin|normal-hitclap", "bad"]
        })).unwrap();
        layout.normalise();
        let result = serde_json::to_value(layout).unwrap();
        assert_eq!(result["lane_names"], json!({"Skin|normal-hitclap": "Snare"}));
        assert_eq!(result["lane_order"], json!(["Skin|normal-hitclap", "Skin|normal-hitnormal"]));
    }
}