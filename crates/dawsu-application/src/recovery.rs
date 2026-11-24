//! Working hits and lanes that can be reopened without compiling or writing the difficulty.

use super::{Document, compile_snapshot};
use dawsu_editor::EditHistory;
use dawsu_foundation::{
    CustomIndex, Hitsound, SampleIdentity, SampleSet, SliderSound, SongTime, Sound,
    digest::sha256_hex, safe_write,
};
use dawsu_hitsounding::{Hitsounds, MomentId};
use serde::{Deserialize, Serialize, de};
use std::{
    fs::{self, File},
    io::{BufReader, ErrorKind},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

const VERSION: u32 = 1;
static NEXT_COPY: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryOffer {
    pub id: String,
    pub mapset: String,
    pub difficulty: String,
    pub created_ms: u64,
}

pub(super) struct Checkpoint {
    revision: u64,
    pending_revision: u64,
    mapset: String,
    difficulty: String,
    source: PathBuf,
    digest: String,
    files: Vec<String>,
    last_save: Option<LastSave>,
    working: WorkingSnapshot,
    pending: Vec<crate::pending::PendingSample>,
}

#[derive(Serialize, Deserialize)]
pub(super) struct Record<T> {
    version: u32,
    offer: RecoveryOffer,
    checkpoint: T,
}

pub(super) type RecoveryCopy = Record<CheckpointData>;

#[derive(Serialize, Deserialize)]
pub(super) struct CheckpointData {
    source: PathBuf,
    digest: String,
    files: Vec<String>,
    last_save: Option<LastSave>,
    working: Working,
    /// The pending files the session kept, by name and checksum. Older builds ignore the list;
    /// copies from earlier builds have none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pending: Vec<crate::pending::PendingSample>,
}

#[derive(Serialize)]
struct CheckpointDataRef<'a> {
    source: &'a PathBuf,
    digest: &'a str,
    files: &'a [String],
    last_save: Option<&'a LastSave>,
    working: Working,
    #[serde(skip_serializing_if = "<[_]>::is_empty")]
    pending: &'a [crate::pending::PendingSample],
}

struct WorkingSnapshot {
    hits: Hitsounds,
    row_order: Vec<SampleIdentity>,
    dirty: bool,
    hs_mode: bool,
}

#[derive(Serialize, Deserialize)]
struct LastSave {
    // Save can add lines or objects. Undo still uses the file the session opened.
    original: Option<Vec<u8>>,
    hits: WorkingHits,
    hs_mode: bool,
    // The hits as drawn at that Save, when it left moments out: Undo returns to them, so a Save
    // after restoring keeps them. Copies from earlier builds have none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    drawn: Option<WorkingHits>,
}

#[derive(Serialize, Deserialize)]
struct Working {
    hits: WorkingHits,
    row_order: Vec<Sample>,
    dirty: bool,
    hs_mode: bool,
}

#[derive(Serialize, Deserialize)]
struct WorkingHits {
    lanes: Vec<Sample>,
    moments: Vec<WorkingMoment>,
    // Sound set links, by moment index. Copies from earlier builds have none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    placements: Vec<WorkingPlacement>,
}

#[derive(Serialize, Deserialize)]
struct WorkingPlacement {
    id: u64,
    set_id: String,
    set_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    accepted_digest: Option<String>,
    moment: usize,
    gain: i32,
    applied: Vec<WorkingApplied>,
}

#[derive(Serialize, Deserialize)]
struct WorkingApplied {
    member_id: String,
    lane: Sample,
    volume: i32,
    added: bool,
}

#[derive(Serialize, Deserialize)]
struct WorkingMoment {
    // The regular JSON float reader can round a fractional millisecond differently.
    time_bits: u64,
    volume: i32,
    hits: Vec<Sample>,
    /// Per-hit volumes aligned with `hits`, omitted when every hit matches `volume`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    hit_volumes: Vec<i32>,
}

// Serde adapters keep the recovery file's representation private to Application.
#[derive(Serialize, Deserialize)]
struct Sample(#[serde(with = "SampleValue")] SampleIdentity);

#[derive(Serialize, Deserialize)]
#[serde(remote = "SampleIdentity")]
enum SampleValue {
    Standard {
        #[serde(with = "SetValue")]
        set: SampleSet,
        #[serde(with = "SoundValue")]
        sound: Sound,
        #[serde(with = "IndexValue")]
        index: CustomIndex,
    },
    File(String),
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "SampleSet")]
enum SetValue {
    Normal,
    Soft,
    Drum,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "Sound")]
enum SoundValue {
    Hit(#[serde(with = "HitsoundValue")] Hitsound),
    Slider(#[serde(with = "SliderSoundValue")] SliderSound),
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "Hitsound")]
enum HitsoundValue {
    Normal,
    Whistle,
    Finish,
    Clap,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "SliderSound")]
enum SliderSoundValue {
    Slide,
    Whistle,
    Tick,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "CustomIndex")]
struct IndexValue(#[serde(getter = "custom_index_value")] u32);

fn custom_index_value(index: &CustomIndex) -> u32 {
    index.get()
}

impl From<IndexValue> for CustomIndex {
    fn from(value: IndexValue) -> Self {
        Self::new(value.0)
    }
}

fn pending_list(document: &Document) -> Vec<crate::pending::PendingSample> {
    document.pending.files().map(|file| crate::pending::PendingSample { name: file.name.clone(), sha256: file.sha256.clone() }).collect()
}

impl Checkpoint {

    pub(super) fn new(document: &Document) -> Self {
        let metadata = |key: &str, fallback: &str| {
            document
                .file
                .value("Metadata", key)
                .filter(|name| !name.trim().is_empty())
                .unwrap_or_else(|| fallback.to_owned())
        };
        Self {
            revision: document.composer.history.version(),
            pending_revision: document.pending.revision(),
            mapset: metadata("Title", "Mapset"),
            difficulty: metadata("Version", "Difficulty"),
            source: document.path.clone(),
            digest: sha256_hex(document.file.original_bytes()),
            files: document.files.clone(),
            last_save: None,
            working: WorkingSnapshot::capture(document),
            pending: pending_list(document),
        }
    }

    pub(super) fn after_save(document: &Document, hits: &Hitsounds, drawn: &Hitsounds, hs_mode: bool) -> Self {
        let mut checkpoint = Self::new(document);
        let original = document.compile_base.original_bytes();
        checkpoint.last_save = Some(LastSave {
            original: (original != document.file.original_bytes()).then(|| original.to_vec()),
            hits: WorkingHits::capture(hits),
            hs_mode,
            drawn: (drawn != hits).then(|| WorkingHits::capture(drawn)),
        });
        checkpoint
    }

    pub(super) fn refresh(&mut self, document: &Document) {
        if self.source != document.path {
            return;
        }
        if self.pending_revision != document.pending.revision() {
            self.pending = pending_list(document);
            self.pending_revision = document.pending.revision();
        }
        let history = &document.composer.history;
        if !history.is_transaction_open()
            && (self.revision != history.version() || self.working.hs_mode != document.hs_mode)
        {
            self.working = WorkingSnapshot::capture(document);
            self.revision = history.version();
        }
    }

    pub(super) fn write(&self, root: &Path) -> Result<RecoveryOffer, String> {
        let created_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_millis() as u64;
        let id = format!(
            "{} - {} - {created_ms}-{}-{}.json",
            filename_part(&self.mapset),
            filename_part(&self.difficulty),
            std::process::id(),
            NEXT_COPY.fetch_add(1, Ordering::Relaxed)
        );
        let checkpoint = CheckpointDataRef {
            source: &self.source,
            digest: &self.digest,
            files: &self.files,
            last_save: self.last_save.as_ref(),
            working: self.working.to_working(),
            pending: &self.pending,
        };
        let record = Record {
            version: VERSION,
            offer: RecoveryOffer {
                id,
                mapset: self.mapset.clone(),
                difficulty: self.difficulty.clone(),
                created_ms,
            },
            checkpoint,
        };
        let directory = root.join("recovery");
        fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
        let bytes = serde_json::to_vec(&record).map_err(|error| error.to_string())?;
        safe_write::create(&directory.join(&record.offer.id), &bytes)
            .map_err(|error| error.to_string())?;
        Ok(record.offer)
    }
}

impl WorkingSnapshot {
    fn capture(document: &Document) -> Self {
        Self {
            hits: document.composer.hitsounds().clone(),
            row_order: document.composer.lanes.clone(),
            dirty: document.composer.dirty(),
            hs_mode: document.hs_mode,
        }
    }

    fn to_working(&self) -> Working {
        Working {
            hits: WorkingHits::capture(&self.hits),
            row_order: self.row_order.iter().cloned().map(Sample).collect(),
            dirty: self.dirty,
            hs_mode: self.hs_mode,
        }
    }
}

impl WorkingHits {
    fn capture(hits: &Hitsounds) -> Self {
        Self {
            lanes: hits
                .lanes()
                .iter()
                .map(|lane| Sample(lane.identity.clone()))
                .collect(),
            moments: hits
                .moments()
                .iter()
                .map(|moment| {
                    let volume = moment.volume();
                    let volumes: Vec<i32> = moment
                        .hits()
                        .map(|lane| moment.hit_volume(lane))
                        .collect();
                    let hit_volumes = if volumes.iter().any(|&v| v != volume) {
                        volumes
                    } else {
                        Vec::new()
                    };
                    WorkingMoment {
                        time_bits: moment.time().ms().to_bits(),
                        volume,
                        hits: moment.hits().cloned().map(Sample).collect(),
                        hit_volumes,
                    }
                })
                .collect(),
            placements: hits
                .placements()
                .iter()
                .map(|p| WorkingPlacement {
                    id: p.id,
                    set_id: p.set_id.clone(),
                    set_digest: p.set_digest.clone(),
                    accepted_digest: p.accepted_digest.clone(),
                    moment: p.moment.0,
                    gain: p.gain,
                    applied: p.applied.iter().map(|a| WorkingApplied {
                        member_id: a.member_id.clone(), lane: Sample(a.lane.clone()), volume: a.volume, added: a.added,
                    }).collect(),
                })
                .collect(),
        }
    }

    fn apply(&self, mut base: Hitsounds, cancel: &AtomicBool) -> Result<Hitsounds, String> {
        let anchored = base.moments().len();
        if self.moments.len() < anchored {
            return Err("The recovery copy is missing moments from the difficulty.".into());
        }
        for (index, saved) in self.moments.iter().enumerate() {
            if cancel.load(Ordering::Relaxed) {
                return Err("Restore cancelled.".into());
            }
            let time_ms = f64::from_bits(saved.time_bits);
            if !time_ms.is_finite() {
                return Err("Invalid moment time in the recovery copy.".into());
            }
            let id = if index < anchored {
                let id = MomentId(index);
                let current = base.moment(id);
                if current.time().ms().to_bits() != saved.time_bits {
                    return Err("The recovery copy's moments do not match the difficulty.".into());
                }
                let previous: Vec<_> = current.hits().cloned().collect();
                for lane in previous {
                    base.remove_hit(id, &lane);
                }
                base.set_volume(id, saved.volume);
                id
            } else {
                base.add_moment(SongTime::from_ms(time_ms), saved.volume)
            };
            for (hit_index, Sample(lane)) in saved.hits.iter().enumerate() {
                base.add_hit(id, lane.clone());
                if let Some(&vol) = saved.hit_volumes.get(hit_index)
                    && vol != saved.volume
                {
                    base.set_hit_volume(id, lane, vol);
                }
            }
        }
        // A lane can be empty after its last hit was deleted. Resolve it the same way
        // the editor did when it first added the hit.
        for Sample(lane) in &self.lanes {
            if base.lane(lane).is_none() {
                if base.moments().is_empty() {
                    return Err("Invalid empty lane in the recovery copy.".into());
                }
                base.add_hit(MomentId(0), lane.clone());
                base.remove_hit(MomentId(0), lane);
            }
        }
        base.restore_placements(self.placements.iter().map(|p| dawsu_hitsounding::sound_sets::SoundSetPlacement {
            id: p.id,
            set_id: p.set_id.clone(),
            set_digest: p.set_digest.clone(),
            accepted_digest: p.accepted_digest.clone(),
            moment: MomentId(p.moment),
            gain: p.gain,
            applied: p.applied.iter().map(|a| dawsu_hitsounding::sound_sets::Applied {
                member_id: a.member_id.clone(), lane: a.lane.0.clone(), volume: a.volume, added: a.added,
            }).collect(),
        }).collect());
        Ok(base)
    }
}

impl RecoveryCopy {
    pub(super) fn source(&self) -> &Path {
        &self.checkpoint.source
    }

    pub(super) fn files(&self) -> &[String] {
        &self.checkpoint.files
    }

    pub(super) fn pending(&self) -> &[crate::pending::PendingSample] {
        &self.checkpoint.pending
    }

    pub(super) fn original(&self) -> Option<&[u8]> {
        self.checkpoint.last_save.as_ref()?.original.as_deref()
    }

    pub(super) fn verify_source(&self, path: &Path, bytes: &[u8]) -> Result<(), String> {
        if path != self.source() {
            return Err(
                "The difficulty has moved. Put it back at its original path and retry.".into(),
            );
        }
        if sha256_hex(bytes) != self.checkpoint.digest {
            return Err("The difficulty changed on disk. The recovery copy was kept.".into());
        }
        Ok(())
    }

    pub(super) fn apply(
        &self,
        document: &mut Document,
        loose_hits_saved_in_project: bool,
        cancel: &AtomicBool,
    ) -> Result<(), String> {
        let working = &self.checkpoint.working;
        let base = document.composer.hitsounds();
        let restored = working.hits.apply(base.clone(), cancel)?;
        let saved = if let Some(last_save) = &self.checkpoint.last_save {
            let hits = last_save.hits.apply(base.clone(), cancel)?;
            let (file, problems) = compile_snapshot(
                document,
                &hits,
                last_save.hs_mode,
                loose_hits_saved_in_project,
                &|| cancel.load(Ordering::Relaxed),
            )?;
            if !problems.is_empty() || file.to_bytes() != document.file.original_bytes() {
                return Err(
                    "The recovery copy could not reproduce the last saved difficulty.".into(),
                );
            }
            // The hits the copy's last Save wrote are what a part Save leaves out goes back to.
            document.written = super::last_written(&hits, last_save.hs_mode);
            match &last_save.drawn {
                Some(drawn) => drawn.apply(base.clone(), cancel)?,
                None => hits,
            }
        } else {
            document.written = base.clone();
            base.clone()
        };

        if working.dirty {
            document.composer.history = EditHistory::new(saved);
            document
                .composer
                .apply("Restore recovery copy", |hits| *hits = restored);
        } else {
            document.composer.history = EditHistory::new(restored);
        }
        document.composer.lanes = working.row_order.iter().map(|s| s.0.clone()).collect();
        document.hs_mode = working.hs_mode;
        Ok(())
    }

    pub(super) fn resume(self, checkpoint: &mut Checkpoint) -> String {
        checkpoint.last_save = self.checkpoint.last_save;
        self.offer.id
    }
}

fn filename_part(value: &str) -> String {
    let safe: String = value
        .chars()
        .take(24)
        .map(|c| {
            if c.is_alphanumeric() || c == ' ' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let safe = safe.trim_matches([' ', '-']);
    if safe.is_empty() {
        "untitled".into()
    } else {
        safe.into()
    }
}

fn valid_id(id: &str) -> bool {
    id.strip_suffix(".json").is_some_and(|name| {
        !name.is_empty()
            && name.len() <= 250
            && name
                .chars()
                .all(|c| c.is_alphanumeric() || c == ' ' || c == '-')
    })
}

pub(super) fn pending_path(root: &Path, id: &str) -> Result<PathBuf, String> {
    if !valid_id(id) {
        return Err("Invalid recovery copy identifier.".into());
    }
    Ok(root.join("recovery").join(id))
}

pub(super) fn archive(root: &Path, id: &str) -> Result<(), String> {
    let source = pending_path(root, id)?;
    let directory = root.join("recovery").join("archive");
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let destination = directory.join(id);
    if destination
        .try_exists()
        .map_err(|error| error.to_string())?
    {
        return Err("The archive already contains a copy with this name.".into());
    }
    fs::rename(source, destination).map_err(|error| error.to_string())
}

fn read_record<T: de::DeserializeOwned>(path: &Path) -> Result<Record<T>, String> {
    let file = File::open(path).map_err(|error| format!("Opening the recovery copy: {error}"))?;
    let record: Record<T> = serde_json::from_reader(BufReader::new(file))
        .map_err(|error| format!("Reading the recovery copy: {error}"))?;
    if record.version != VERSION {
        return Err(format!(
            "Unsupported recovery copy version {}.",
            record.version
        ));
    }
    if path.file_name().and_then(|name| name.to_str()) != Some(record.offer.id.as_str()) {
        return Err("The recovery copy has the wrong identifier.".into());
    }
    Ok(record)
}

pub(super) fn read_copy(path: &Path) -> Result<RecoveryCopy, String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if !metadata.is_file() {
        return Err("The recovery copy is not a regular file.".into());
    }
    let copy: RecoveryCopy = read_record(path)?;
    let source = copy.source();
    if !source.is_absolute()
        || !source
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("osu"))
    {
        return Err("Invalid source path in the recovery copy.".into());
    }
    Ok(copy)
}

pub(super) fn offers(root: &Path) -> Result<Vec<RecoveryOffer>, String> {
    let entries = match fs::read_dir(root.join("recovery")) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.to_string()),
    };
    let mut offers = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| error.to_string())?;
        if entry.file_name().to_str().is_some_and(valid_id)
            && entry.file_type().is_ok_and(|kind| kind.is_file())
            && let Ok(record) = read_record::<de::IgnoredAny>(&entry.path())
        {
            offers.push(record.offer);
        }
    }
    offers.sort_by(|left, right| {
        right
            .created_ms
            .cmp(&left.created_ms)
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(offers)
}

impl WorkingHits {
    /// The samples its hits play. Its lanes are left out on purpose: a lane with no hit keeps no
    /// file in the mapset, in a recovery copy as in the open difficulty (Yoge, 2026-11-21).
    fn samples(&self) -> impl Iterator<Item = &SampleIdentity> {
        self.moments.iter().flat_map(|moment| &moment.hits)
            .chain(self.placements.iter().flat_map(|placement| placement.applied.iter().map(|applied| &applied.lane)))
            .map(|sample| &sample.0)
    }
}

/// The stopped sessions' recovery copies of a difficulty in `mapset`. A copy that can't be read
/// could belong to any mapset: it is an error that names the copy.
pub(super) fn mapset_copies(data_root: &Path, mapset: &Path) -> Result<Vec<RecoveryCopy>, String> {
    let Ok(entries) = fs::read_dir(data_root.join("recovery")) else { return Ok(Vec::new()) };
    let same = |folder: &Path| folder == mapset
        || folder.canonicalize().ok().zip(mapset.canonicalize().ok()).is_some_and(|(a, b)| a == b);
    let mut out = Vec::new();
    for entry in entries.filter_map(Result::ok) {
        if !entry.file_name().to_str().is_some_and(valid_id) {
            continue;
        }
        let copy = read_copy(&entry.path())
            .map_err(|_| format!("The recovery copy {} could not be read.", entry.path().display()))?;
        if copy.source().parent().is_some_and(same) {
            out.push(copy);
        }
    }
    Ok(out)
}

/// Every sample a stopped session's recovery copy of a difficulty in `mapset` plays or lists:
/// its hits, those of its last Save and its pending files. A sample file it names stays in the mapset, so restoring
/// the copy still plays it. A copy that can't be read is an error that names the copy.
pub(super) fn copies_play(data_root: &Path, mapset: &Path) -> Result<Vec<SampleIdentity>, String> {
    let mut out = Vec::new();
    for copy in mapset_copies(data_root, mapset)? {
        let data = &copy.checkpoint;
        let saved = data.last_save.iter().flat_map(|save| save.hits.samples().chain(save.drawn.iter().flat_map(WorkingHits::samples)));
        out.extend(data.working.hits.samples().chain(saved).cloned());
        out.extend(data.pending.iter().map(|file| SampleIdentity::File(file.name.clone())));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pending::Pending;
    use dawsu_test_support::Scratch;

    const TEST_OSU: &[u8] = include_bytes!("../tests/project-fixture.osu");

    fn eager_record_bytes(document: &Document, checkpoint: &Checkpoint, offer: &RecoveryOffer) -> Vec<u8> {
        let eager = Record {
            version: VERSION,
            offer: offer.clone(),
            checkpoint: CheckpointData {
                source: document.path.clone(),
                digest: sha256_hex(document.file.original_bytes()),
                files: document.files.clone(),
                last_save: checkpoint.last_save.as_ref().map(|save| LastSave {
                    original: save.original.clone(),
                    hits: WorkingHits {
                        lanes: save.hits.lanes.iter().map(|s| Sample(s.0.clone())).collect(),
                        moments: save.hits.moments.iter().map(|m| WorkingMoment {
                            time_bits: m.time_bits,
                            volume: m.volume,
                            hits: m.hits.iter().map(|s| Sample(s.0.clone())).collect(),
                            hit_volumes: m.hit_volumes.clone(),
                        }).collect(),
                        placements: save.hits.placements.iter().map(|p| WorkingPlacement {
                            id: p.id,
                            set_id: p.set_id.clone(),
                            set_digest: p.set_digest.clone(),
                            accepted_digest: p.accepted_digest.clone(),
                            moment: p.moment,
                            gain: p.gain,
                            applied: p.applied.iter().map(|a| WorkingApplied {
                                member_id: a.member_id.clone(),
                                lane: Sample(a.lane.0.clone()),
                                volume: a.volume,
                                added: a.added,
                            }).collect(),
                        }).collect(),
                    },
                    hs_mode: save.hs_mode,
                    drawn: save.drawn.as_ref().map(|d| WorkingHits {
                        lanes: d.lanes.iter().map(|s| Sample(s.0.clone())).collect(),
                        moments: d.moments.iter().map(|m| WorkingMoment {
                            time_bits: m.time_bits,
                            volume: m.volume,
                            hits: m.hits.iter().map(|s| Sample(s.0.clone())).collect(),
                            hit_volumes: m.hit_volumes.clone(),
                        }).collect(),
                        placements: d.placements.iter().map(|p| WorkingPlacement {
                            id: p.id,
                            set_id: p.set_id.clone(),
                            set_digest: p.set_digest.clone(),
                            accepted_digest: p.accepted_digest.clone(),
                            moment: p.moment,
                            gain: p.gain,
                            applied: p.applied.iter().map(|a| WorkingApplied {
                                member_id: a.member_id.clone(),
                                lane: Sample(a.lane.0.clone()),
                                volume: a.volume,
                                added: a.added,
                            }).collect(),
                        }).collect(),
                    }),
                }),
                working: Working {
                    hits: WorkingHits::capture(document.composer.hitsounds()),
                    row_order: document.composer.lanes.iter().cloned().map(Sample).collect(),
                    dirty: document.composer.dirty(),
                    hs_mode: document.hs_mode,
                },
                pending: pending_list(document),
            },
        };
        serde_json::to_vec(&eager).unwrap()
    }

    #[test]
    fn lazy_checkpoint_write_is_byte_identical_to_eager_checkpoint_data() {
        let scratch = Scratch::new("lazy-recovery-json");
        let mapset = scratch.path("mapset");
        let data_root = scratch.path("data");
        fs::create_dir_all(&mapset).unwrap();
        let osu_path = mapset.join("map.osu");
        fs::write(&osu_path, TEST_OSU).unwrap();

        let mut document = Document::read(&osu_path).unwrap();
        document.pending = Pending::for_mapset(&data_root, &mapset);
        let mut checkpoint = Checkpoint::new(&document);

        let offer = checkpoint.write(&data_root).unwrap();
        let written = fs::read(pending_path(&data_root, &offer.id).unwrap()).unwrap();
        assert_eq!(written, eager_record_bytes(&document, &checkpoint, &offer));

        // Edit hits, reorder lanes, toggle hs_mode, and add a pending file.
        document.composer.apply("Add hit and override volume", |hits| {
            let id = hits.add_moment(SongTime::from_ms(1250.5), 75);
            let lane = SampleIdentity::File("snare.wav".into());
            hits.add_hit(id, lane.clone());
            hits.set_hit_volume(id, &lane, 55);
        });
        document.composer.lanes.reverse();
        document.hs_mode = true;
        document.pending.add("snare.wav", b"RIFFdemo").unwrap();
        checkpoint.refresh(&document);

        let offer = checkpoint.write(&data_root).unwrap();
        let written = fs::read(pending_path(&data_root, &offer.id).unwrap()).unwrap();
        assert_eq!(written, eager_record_bytes(&document, &checkpoint, &offer));

        // Also check after_save with drawn != compiled and after releasing pending.
        let compiled = document.composer.hitsounds().clone();
        let mut drawn = compiled.clone();
        drawn.set_volume(MomentId(0), 40);
        let mut after_save = Checkpoint::after_save(&document, &compiled, &drawn, document.hs_mode);
        document.pending.release("snare.wav");
        after_save.refresh(&document);

        let offer = after_save.write(&data_root).unwrap();
        let written = fs::read(pending_path(&data_root, &offer.id).unwrap()).unwrap();
        assert_eq!(written, eager_record_bytes(&document, &after_save, &offer));
    }

    #[test]
    fn refresh_skips_open_transactions_and_tracks_pending_revision() {
        let scratch = Scratch::new("lazy-recovery-refresh");
        let mapset = scratch.path("mapset");
        let data_root = scratch.path("data");
        fs::create_dir_all(&mapset).unwrap();
        let osu_path = mapset.join("map.osu");
        fs::write(&osu_path, TEST_OSU).unwrap();

        let mut document = Document::read(&osu_path).unwrap();
        document.pending = Pending::for_mapset(&data_root, &mapset);
        let mut checkpoint = Checkpoint::new(&document);
        let initial_rev = checkpoint.revision;
        let initial_pending_rev = checkpoint.pending_revision;

        // No changes: refresh is a no-op.
        checkpoint.refresh(&document);
        assert_eq!(checkpoint.revision, initial_rev);
        assert_eq!(checkpoint.pending_revision, initial_pending_rev);

        // Open transaction: edits must NOT update the checkpoint snapshot.
        document.composer.history.begin("Open stroke");
        let _ = document.composer.history.edit(true, |hits| {
            hits.set_volume(MomentId(0), 33);
            Ok::<_, std::convert::Infallible>(())
        });
        checkpoint.refresh(&document);
        assert_eq!(checkpoint.revision, initial_rev);
        assert_eq!(checkpoint.working.hits.moments()[0].volume(), 100);

        // Commit transaction: checkpoint updates on refresh.
        document.composer.history.commit();
        checkpoint.refresh(&document);
        assert_eq!(checkpoint.revision, document.composer.history.version());
        assert_eq!(checkpoint.working.hits.moments()[0].volume(), 33);

        // Pending add updates pending_revision and pending list; release updates it again.
        document.pending.add("kick.wav", b"RIFFkick").unwrap();
        checkpoint.refresh(&document);
        assert_eq!(checkpoint.pending_revision, document.pending.revision());
        assert_eq!(checkpoint.pending.len(), 1);
        assert_eq!(checkpoint.pending[0].name, "kick.wav");

        document.pending.release("kick.wav");
        checkpoint.refresh(&document);
        assert_eq!(checkpoint.pending_revision, document.pending.revision());
        assert!(checkpoint.pending.is_empty());
    }
}
