//! Render the compiler's one-shot mix plans and stage their files until Apply or Save.

use crate::{ExportSettings, export_settings, lane_name, sample_path};
use crate::pending::{Pending, listing};
use dawsu_audio::{AudioClip, Mixer, ScheduledSound, decode_with_cancel};
use dawsu_foundation::{
    CustomIndex, Hitsound, SampleIdentity, SampleSet, Sound, digest::sha256_hex, safe_write,
};
use dawsu_hitsounding::{
    Compiled, Decompiled, Hitsounds, LaneSource, MomentId, ObjectId, Place, Placement, ProblemKind,
    SaveProblem, compile, compile::{RoleFile, RoleMerge, merge_roles}, decompile,
};
use dawsu_osu_format::{OsuFile, list_folder};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const SAMPLE_RATE: u32 = 48_000;
const CHANNELS: usize = 2;
const PREFIX: &str = "dawsu-mix-";

pub(crate) fn can_render(problem: &SaveProblem, placement: Placement) -> bool {
    (matches!(problem.place, Place::Circle(_) | Place::SpinnerEnd(_))
        || placement == Placement::GreenLines && matches!(problem.place, Place::SliderNode { .. }))
        && matches!(&problem.kind, ProblemKind::NeedsSampleMix(layers) if !layers.is_empty())
}

fn mix_place(place: Place) -> Option<(ObjectId, Option<usize>)> {
    match place {
        Place::Circle(id) | Place::SpinnerEnd(id) => Some((id, None)),
        Place::SliderNode { object, node } => Some((object, Some(node))),
        Place::NoObject => None,
    }
}

#[derive(Default)]
pub(crate) struct MixFiles {
    pub(super) staged: Vec<StagedMix>,
    pub(super) combinations: Vec<export_settings::ExportCombination>,
}

#[derive(Clone)]
pub(super) struct StagedMix {
    pub(super) path: PathBuf,
    wav: Vec<u8>,
    reuse: bool,
    index: Option<u32>,
}

impl StagedMix {
    pub(super) fn is_reused(&self) -> bool {
        self.reuse
    }

    pub(super) fn digest(&self) -> String {
        sha256_hex(&self.wav)
    }

    pub(super) fn new_generated<'a>(
        root: &Path,
        wav: Vec<u8>,
        mut existing: impl Iterator<Item = &'a String>,
        names: &mut BTreeSet<String>,
    ) -> Result<(String, Self), String> {
        let stem = format!("{PREFIX}{}-", sha256_hex(&wav));
        let reusable =
            existing.find(|name| name.starts_with(&stem) && has_contents(root, name, &wav));
        let name = match reusable {
            Some(name) => name.clone(),
            None => reserve_name(&stem, names)?,
        };
        let mix = Self {
            path: root.join(&name),
            wav,
            reuse: reusable.is_some(),
            index: None,
        };
        Ok((name, mix))
    }

    /// A pending file a Save writes as it is, checked by its name when published. A file another
    /// program put under the same name is used when it holds the same bytes; other bytes make
    /// the publish refuse and name it.
    pub(super) fn pending(root: &Path, name: &str, bytes: Vec<u8>) -> Self {
        let path = root.join(name);
        let reuse = fs::read(&path).is_ok_and(|on_disk| on_disk == bytes);
        Self { path, wav: bytes, reuse, index: None }
    }

    pub(super) fn clip(&self) -> AudioClip {
        let samples = self.wav[44..]
            .chunks_exact(2)
            .map(|frame| f32::from(i16::from_le_bytes([frame[0], frame[1]])) / 32768.0)
            .collect();
        AudioClip {
            samples,
            channels: CHANNELS,
            sample_rate: SAMPLE_RATE,
        }
    }
}

pub(super) fn stage_in_file(
    source: &Hitsounds,
    file: OsuFile,
    problems: &[SaveProblem],
    root: &Path,
    defaults: &Path,
    expected: &[u8],
    placement: Placement,
    already_staged: &[StagedMix],
    settings: Option<&ExportSettings>,
    previous_mixes: &[String],
    start_index: u32,
    pending: &Pending,
    cancel: &dyn Fn() -> bool,
) -> Result<(OsuFile, MixFiles), String> {
    let listing = listing(root, pending)?;
    let mut hits = match decompile(&file, &listing, cancel, &mut |_, _| {}) {
        Decompiled::Done(hits) => hits,
        Decompiled::Cancelled => return Err("Sample mix preparation cancelled".into()),
        Decompiled::UnsupportedMode { .. } => return Err("Cannot mix this difficulty".into()),
    };
    let mut files = MixFiles::default();
    if let Some(settings) = settings {
        files.prepare_export(
            source,
            &mut hits,
            problems,
            root,
            defaults,
            settings,
            previous_mixes,
            pending,
            cancel,
        )?;
    } else {
        files.prepare(
            source,
            &mut hits,
            problems,
            root,
            defaults,
            expected,
            already_staged,
            start_index,
            pending,
            cancel,
        )?;
    }
    match compile(&file, &hits, placement, cancel, &mut |_, _| {}) {
        Compiled::Done { file, problems } if problems.is_empty() => Ok((file, files)),
        Compiled::Done { problems, .. } => Err(format!(
            "Could not make the sample mix at {:.0} ms: {}",
            problems[0].time.ms(),
            crate::problem_description(&problems[0].kind, problems[0].time.ms()),
        )),
        Compiled::Cancelled => Err("Sample mix preparation cancelled".into()),
    }
}

impl MixFiles {
    fn prepare_export(
        &mut self,
        source: &Hitsounds,
        target: &mut Hitsounds,
        problems: &[SaveProblem],
        root: &Path,
        defaults: &Path,
        settings: &ExportSettings,
        previous_mixes: &[String],
        pending: &Pending,
        cancel: &dyn Fn() -> bool,
    ) -> Result<(), String> {
        settings.validate()?;
        let rules = export_settings::Rules::new(settings);
        let defaults_list = list_folder(defaults).unwrap_or_default();
        let moments: HashMap<_, _> = target
            .moments()
            .iter()
            .enumerate()
            .filter_map(|(index, moment)| Some((mix_place(moment.place())?, MomentId(index))))
            .collect();
        let mut indexes = IndexedMixes::read(root, &OsuFile::read(Vec::new()).file, &[], 2, pending)?;
        let previous_mixes = previous_mixes
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        let mut inputs = HashMap::<SampleIdentity, (String, Arc<AudioClip>)>::new();
        let mut rendered = HashMap::new();
        let mut described = BTreeSet::new();
        for problem in problems {
            if cancel() {
                return Err("Generate cancelled during sample mixing.".into());
            }
            let ProblemKind::NeedsSampleMix(layers) = &problem.kind else {
                return Err("Expected a Generate sample mix plan.".into());
            };
            let moment = mix_place(problem.place)
                .and_then(|place| moments.get(&place))
                .copied()
                .ok_or("Sample mix has no matching Generate circle.")?;
            let highest = layers
                .iter()
                .map(|(_, volume)| (*volume).max(8))
                .max()
                .unwrap_or(100);
            let mut labels = Vec::with_capacity(layers.len());
            let mut identities = Vec::with_capacity(layers.len());
            for (identity, volume) in layers {
                identities.push(identity.clone());
                if !inputs.contains_key(identity) {
                    let lane = source.lane(identity).ok_or("Sample mix lane is missing.")?;
                    let name = lane_name(identity);
                    let path = sample_path(lane, root, pending, defaults, &defaults_list)
                        .map_err(|error| format!("Could not mix {name}: {error}"))?;
                    let input =
                        fs::read(&path).map_err(|error| format!("Reading {name}: {error}"))?;
                    let fingerprint = sha256_hex(&input).to_ascii_uppercase();
                    let clip = decode_with_cancel(&path, cancel)
                        .map_err(|error| format!("Could not mix {name}: {error}"))?;
                    if fs::read(&path).map_err(|error| format!("Checking {name}: {error}"))?
                        != input
                    {
                        return Err(format!("{name} changed during Generate. Review again."));
                    }
                    let (kind, name) = match &lane.source {
                        LaneSource::File(name) => ("BeatmapFile", name.clone()),
                        LaneSource::Skin => ("Skin", name),
                    };
                    let label = format!(
                        "hit:{kind}:{}@{fingerprint}",
                        name.replace('\\', "/").to_lowercase()
                    );
                    inputs.insert(identity.clone(), (label, clip));
                }
                let label = &inputs[identity].0;
                labels.push(if settings.pair_samples {
                    format!(
                        "{label}@balance:{}",
                        export_settings::balance((*volume).max(8), highest)
                    )
                } else {
                    label.clone()
                });
            }
            labels.sort();
            let (rule, keep) = rules.resolve(problem.time.ms(), &labels);
            if keep {
                return Err(format!(
                    "The preset requests original files at {:.0} ms, where the sounds need mixing.",
                    problem.time.ms()
                ));
            }
            let destination = rule.unwrap_or_else(|| settings.priority(&identities));
            let rule_key = export_settings::legacy_key(&labels);
            if described.insert((rule_key.clone(), destination)) {
                self.combinations.push(export_settings::ExportCombination {
                    key: rule_key,
                    time: problem.time.ms(),
                    layers: layers
                        .iter()
                        .map(|(lane, volume)| format!("{} ({volume}%)", lane_name(lane)))
                        .collect(),
                    destination: destination.into(),
                });
            }
            let key = (destination, layers.as_slice());
            let outputs = if let Some(outputs) = rendered.get(&key) {
                outputs
            } else {
                let (bank, fused_role) = export_settings::slot(destination)?;
                let normal_bank = if settings.pair_samples {
                    identities
                        .iter()
                        .find_map(|identity| match identity {
                            SampleIdentity::Standard {
                                set,
                                sound: Sound::Hit(Hitsound::Normal),
                                ..
                            } => Some(*set),
                            _ => None,
                        })
                        .unwrap_or(bank)
                } else {
                    bank
                };
                let mut components = BTreeMap::<Hitsound, Vec<ScheduledSound>>::new();
                components.entry(Hitsound::Normal).or_default();
                for (lane, volume) in layers {
                    let role = if settings.pair_samples {
                        match lane {
                            SampleIdentity::Standard {
                                sound: Sound::Hit(role),
                                ..
                            } => *role,
                            _ => Hitsound::Normal,
                        }
                    } else {
                        fused_role
                    };
                    components.entry(role).or_default().push(ScheduledSound {
                        at_ms: 0.0,
                        end_ms: None,
                        clip: inputs[lane].1.clone(),
                        gain: *volume as f32 / 100.0,
                    });
                }
                let roles = components
                    .into_iter()
                    .map(|(role, cues)| {
                        let bank = if role == Hitsound::Normal {
                            normal_bank
                        } else {
                            bank
                        };
                        (bank, role, cues)
                    })
                    .collect();
                let waves = render_roles(roles, cancel)?;
                let reused = reusable_export_index(root, &waves, &previous_mixes);
                let index = match reused {
                    Some(index) => index,
                    None => indexes.reserve()?,
                };
                let outputs: Vec<_> = waves
                    .iter()
                    .map(|(bank, role, _)| SampleIdentity::Standard {
                        set: *bank,
                        sound: Sound::Hit(*role),
                        index: CustomIndex::new(index),
                    })
                    .collect();
                self.stage_roles(
                    root,
                    waves.into_iter().map(|(bank, role, wav)| (bank, role, "wav".to_string(), wav)),
                    index,
                    reused.is_some(),
                );
                &*rendered.entry(key).or_insert(outputs)
            };
            replace_moment(target, moment, outputs.iter().cloned());
        }
        Ok(())
    }

    /// Stages the role files at `index`.
    fn stage_roles(&mut self, root: &Path, files: impl IntoIterator<Item = RoleOutput>, index: u32, reuse: bool) {
        for (bank, role, extension, bytes) in files {
            let name = format!("{}{index}.{extension}", export_settings::slot_name(bank, role));
            self.staged.push(StagedMix {
                path: root.join(name),
                wav: bytes,
                reuse,
                index: Some(index),
            });
        }
    }

    /// Save's sample mixes (Yoge 2026-11-20): a moment osu! cannot hold is merged per role at one
    /// shared index, so its hitnormal stays a hitnormal and only sounds of one role share a file.
    /// The object plays at its loudest sound's volume, or louder when a file would clip, and each
    /// file holds its sounds at their volume over the object's.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare_by_role(
        &mut self,
        source: &Hitsounds,
        target: &mut Hitsounds,
        problems: &[SaveProblem],
        root: &Path,
        defaults: &Path,
        expected: &[u8],
        pending: &Pending,
        cancel: &dyn Fn() -> bool,
    ) -> Result<(), String> {
        let assets = Assets { source, root, pending, defaults, defaults_list: list_folder(defaults).unwrap_or_default() };
        let expected = OsuFile::read(expected.to_vec()).file;
        let moments: HashMap<_, _> = target
            .moments()
            .iter()
            .enumerate()
            .filter_map(|(index, moment)| Some((mix_place(moment.place())?, MomentId(index))))
            .collect();
        let mut indexes = IndexedMixes::read(root, &expected, &[], 2, pending)?;
        // A circle holds its index on its own line, so an earlier Save's files are found there too.
        indexes.reuse.extend(
            expected
                .hit_objects()
                .iter()
                .filter_map(|object| object.hit_sample())
                .map(|sample| sample.index)
                .filter(|index| *index > 1)
                .map(|index| index as u32),
        );
        let mut sections = None;
        let mut clips = HashMap::new();
        let mut by_layers = HashMap::<&[(SampleIdentity, i32)], (u32, i32)>::new();
        let mut by_files = HashMap::<String, u32>::new();
        for problem in problems {
            let moment = *mix_place(problem.place)
                .and_then(|id| moments.get(&id))
                .ok_or("Sample mix has no matching hit")?;
            if cancel() {
                return Err("Sample mix preparation cancelled".into());
            }
            let ProblemKind::NeedsSampleMix(layers) = &problem.kind else {
                return Err("Expected a sample mix plan".into());
            };
            let merge = merge_roles(layers);
            let (index, volume) = match by_layers.get(layers.as_slice()) {
                Some(found) => *found,
                None => {
                    let (volume, files) = assets.render_merge(&merge, target, &mut clips, cancel)?;
                    let key: String = files
                        .iter()
                        .map(|(set, role, extension, bytes)| format!("{}.{extension}={};", export_settings::slot_name(*set, *role), sha256_hex(bytes)))
                        .collect();
                    let index = match by_files.get(&key) {
                        Some(index) => *index,
                        None => {
                            let (index, reuse) = indexes.allocate_roles(root, pending, &files)?;
                            self.stage_roles(root, files, index, reuse);
                            by_files.insert(key, index);
                            index
                        }
                    };
                    by_layers.insert(layers.as_slice(), (index, volume));
                    (index, volume)
                }
            };
            // The problem moment kept its old line, so its hitnormal shows the object's normal set.
            // A sample file has no set: its object's hitnormal then comes from the timing line.
            let object_normal = match target.moment(moment).hits().find_map(|lane| match lane {
                SampleIdentity::Standard { set, sound: Sound::Hit(Hitsound::Normal), .. } => Some(*set),
                _ => None,
            }) {
                Some(set) => set,
                None => timing_set(&expected, root, pending, &mut sections, problem.place)?,
            };
            replace_moment(target, moment, merge.hits(index, object_normal));
            target.set_volume(moment, volume);
        }
        Ok(())
    }

    pub(crate) fn prepare(
        &mut self,
        source: &Hitsounds,
        target: &mut Hitsounds,
        problems: &[SaveProblem],
        root: &Path,
        defaults: &Path,
        expected: &[u8],
        already_staged: &[StagedMix],
        start_index: u32,
        pending: &Pending,
        cancel: &dyn Fn() -> bool,
    ) -> Result<(), String> {
        let defaults_list = list_folder(defaults).unwrap_or_default();
        let expected = OsuFile::read(expected.to_vec()).file;
        let existing: BTreeSet<_> = expected
            .hit_objects()
            .into_iter()
            .filter_map(|object| object.hit_sample()?.filename.clone())
            .filter(|name| name.starts_with(PREFIX) && name.ends_with(".wav"))
            .collect();
        let moments: HashMap<_, _> = target
            .moments()
            .iter()
            .enumerate()
            .filter_map(|(index, moment)| Some((mix_place(moment.place())?, MomentId(index))))
            .collect();
        let mut indexes = if problems
            .iter()
            .any(|problem| matches!(problem.place, Place::SliderNode { .. }))
        {
            Some(IndexedMixes::read(root, &expected, already_staged, start_index, pending)?)
        } else {
            None
        };
        let mut names = if problems.iter().any(|problem| !matches!(problem.place, Place::SliderNode { .. })) {
            reserved_mix_names(root, already_staged, pending)?
        } else {
            BTreeSet::new()
        };
        let mut clips = HashMap::new();
        let mut rendered = HashMap::<(bool, &[(SampleIdentity, i32)]), SampleIdentity>::new();

        for problem in problems {
            let moment = *mix_place(problem.place)
                .and_then(|id| moments.get(&id))
                .ok_or("Sample mix has no matching hit")?;
            if cancel() {
                return Err("Sample mix preparation cancelled".into());
            }
            let ProblemKind::NeedsSampleMix(layers) = &problem.kind else {
                return Err("Expected a sample mix plan".into());
            };
            let slider = matches!(problem.place, Place::SliderNode { .. });
            let key = (slider, layers.as_slice());
            let sample = if let Some(sample) = rendered.get(&key) {
                sample.clone()
            } else {
                let wav = render_layers(source, Some(target), layers, root, pending, defaults, &defaults_list, &mut clips, cancel)?;
                let (sample, staged) = if let Some(indexes) = indexes.as_mut().filter(|_| slider) {
                    let (index, reuse) = indexes.allocate(root, &wav)?;
                    let identity = SampleIdentity::Standard {
                        set: SampleSet::Normal,
                        sound: Sound::Hit(Hitsound::Normal),
                        index: CustomIndex::new(index),
                    };
                    (
                        identity,
                        StagedMix {
                            path: root.join(indexed_name(index)),
                            wav,
                            reuse,
                            index: Some(index),
                        },
                    )
                } else {
                    let (name, staged) = StagedMix::new_generated(root, wav, existing.iter(), &mut names)?;
                    (SampleIdentity::File(name), staged)
                };
                self.staged.push(staged);
                rendered.insert(key, sample.clone());
                sample
            };
            replace_moment(target, moment, [sample]);
        }
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
fn render_layers(
    source: &Hitsounds, target: Option<&Hitsounds>, layers: &[(SampleIdentity, i32)],
    root: &Path, pending: &Pending, defaults: &Path, defaults_list: &[String],
    clips: &mut HashMap<SampleIdentity, Arc<AudioClip>>, cancel: &dyn Fn() -> bool,
) -> Result<Vec<u8>, String> {
    let mut cues = Vec::with_capacity(layers.len());
    for (identity, volume) in layers {
        cues.push(ScheduledSound {
            at_ms: 0.0,
            end_ms: None,
            clip: load_clip(source, target, identity, root, pending, defaults, defaults_list, clips, cancel)?,
            gain: *volume as f32 / 100.0,
        });
    }
    render(cues, cancel)
}

#[allow(clippy::too_many_arguments)]
fn load_clip(
    source: &Hitsounds, target: Option<&Hitsounds>, identity: &SampleIdentity,
    root: &Path, pending: &Pending, defaults: &Path, defaults_list: &[String],
    clips: &mut HashMap<SampleIdentity, Arc<AudioClip>>, cancel: &dyn Fn() -> bool,
) -> Result<Arc<AudioClip>, String> {
    if !clips.contains_key(identity) {
        let lane = source
            .lane(identity)
            .or_else(|| target.and_then(|target| target.lane(identity)))
            .ok_or("Sample mix lane is missing")?;
        let path =
            sample_path(lane, root, pending, defaults, defaults_list).map_err(|error| {
                format!("Could not mix {}: {error}", lane_name(identity))
            })?;
        let clip = decode_with_cancel(&path, cancel).map_err(|error| {
            format!("Could not mix {}: {error}", lane_name(identity))
        })?;
        clips.insert(identity.clone(), clip);
    }
    Ok(clips[identity].clone())
}

type RoleWaves = Vec<(SampleSet, Hitsound, Vec<u8>)>;

/// One WAV per role, each sound at its cue's gain. A role with no sound is written silent: a
/// missing numbered hitnormal would fall back to the skin's plain one (lazer 131fe72:
/// LegacyBeatmapSkin.cs:22, LegacySkin.cs:621-624, SkinManager.cs:314). Export's paired samples
/// and Save's merge per role both write these.
fn render_roles(
    roles: Vec<(SampleSet, Hitsound, Vec<ScheduledSound>)>,
    cancel: &dyn Fn() -> bool,
) -> Result<RoleWaves, String> {
    let mut waves = Vec::with_capacity(roles.len());
    for (bank, role, mut cues) in roles {
        if cues.is_empty() {
            cues.push(ScheduledSound {
                at_ms: 0.0,
                end_ms: None,
                gain: 1.0,
                clip: Arc::new(AudioClip {
                    samples: vec![0.0; CHANNELS],
                    channels: CHANNELS,
                    sample_rate: SAMPLE_RATE,
                }),
            });
        }
        let wav = render(cues, cancel)?;
        waves.push((bank, role, wav));
    }
    Ok(waves)
}

pub(super) fn stage_timed_copy(
    source: &Hitsounds, file: &mut OsuFile, path: &Path, defaults: &Path,
    placements: &[dawsu_hitsounding::copy::SliderCopyPlacement],
    staged: &mut Vec<StagedMix>, details: &mut Vec<String>, pending: &Pending, cancel: &dyn Fn() -> bool,
) -> Result<(), String> {
    if placements.is_empty() {
        return Ok(());
    }
    let root = path.parent().ok_or("Missing mapset folder")?;
    let defaults_list = list_folder(defaults).unwrap_or_default();
    let mut names = reserved_mix_names(root, staged, pending)?;
    let events = file.storyboard_samples();
    let existing: Vec<_> = events.iter().map(|event| event.path.clone()).collect();
    let mut clips = HashMap::new();
    let mut rendered = BTreeMap::<&[(SampleIdentity, i32)], String>::new();
    for placement in placements {
        if cancel() {
            return Err("Copy cancelled during timed sample review.".into());
        }
        let layers = placement.layers.as_slice();
        let name = if let Some(name) = rendered.get(layers) {
            name.clone()
        } else {
            let wav = render_layers(source, None, layers, root, pending, defaults, &defaults_list, &mut clips, cancel)?;
            let name = if let Some(previous) = staged.iter().find(|mix| mix.index.is_none() && mix.wav == wav) {
                previous.path.file_name().ok_or("Missing staged filename")?.to_string_lossy().into_owned()
            } else {
                let (name, mix) = StagedMix::new_generated(root, wav, existing.iter(), &mut names)?;
                staged.push(mix);
                name
            };
            rendered.insert(layers, name.clone());
            name
        };
        // Storyboard events use song time directly, including for pre-v5 difficulties.
        let time = placement.time.ms();
        let role = match placement.role {
            dawsu_foundation::SliderSound::Tick => "tick",
            dawsu_foundation::SliderSound::Slide => "slide",
            _ => return Err("Unsupported timed slider sound.".into()),
        };
        if !events.iter().any(|event| event.time == time && event.path == name && event.volume == placement.volume as f32
            && (event.layer == "0" || event.layer.eq_ignore_ascii_case("Background"))) {
            file.insert_storyboard_sample(&format!("Sample,{time},0,\"{name}\",{}", placement.volume));
        }
        details.push(format!("{} · {time} ms · {role} · timed storyboard sample {name}",
            path.file_name().unwrap_or_default().to_string_lossy()));
    }
    Ok(())
}

/// One file Save writes: its set, role, extension and bytes.
type RoleOutput = (SampleSet, Hitsound, String, Vec<u8>);

struct Assets<'a> {
    source: &'a Hitsounds,
    root: &'a Path,
    pending: &'a Pending,
    defaults: &'a Path,
    defaults_list: Vec<String>,
}

impl Assets<'_> {
    /// The object's volume and the files for one merge. A file with one sound at the object's
    /// volume is that sound's file as it is; the others are rendered. When a mix would clip,
    /// the object's volume rises (up to 100) and the files are turned down to match, so it plays
    /// as loud as the editor does. Only a mix past full scale at 100 still clips, as osu! would.
    fn render_merge(
        &self, merge: &RoleMerge, target: &Hitsounds,
        clips: &mut HashMap<SampleIdentity, Arc<AudioClip>>, cancel: &dyn Fn() -> bool,
    ) -> Result<(i32, Vec<RoleOutput>), String> {
        let loudest = merge.volume;
        let cues = |file: &RoleFile, volume: f32, clips: &mut HashMap<SampleIdentity, Arc<AudioClip>>| {
            file.layers.iter().map(|(identity, v)| Ok(ScheduledSound {
                at_ms: 0.0,
                end_ms: None,
                clip: load_clip(self.source, Some(target), identity, self.root, self.pending, self.defaults, &self.defaults_list, clips, cancel)?,
                gain: *v as f32 / volume,
            })).collect::<Result<Vec<_>, String>>()
        };
        // Peaks are measured at a gain that cannot reach full scale, since the mixer clamps.
        let mut peak = 0.0_f32;
        for file in merge.files.iter().filter(|file| file.layers.len() > 1) {
            let down = (loudest * file.layers.len() as i32) as f32;
            let samples = mix(cues(file, down, clips)?, cancel)?;
            let file_peak = samples.iter().fold(0.0_f32, |p, s| p.max(s.abs())) * file.layers.len() as f32;
            peak = peak.max(file_peak);
        }
        let volume = if peak > 1.0 { ((loudest as f32 * peak - 1e-3).ceil() as i32).clamp(loudest, 100) } else { loudest };
        let mut files = Vec::with_capacity(merge.files.len());
        for file in &merge.files {
            let output = match file.layers.as_slice() {
                [(identity, v)] if *v == volume => {
                    let lane = self.source.lane(identity).or_else(|| target.lane(identity)).ok_or("Sample mix lane is missing")?;
                    let path = sample_path(lane, self.root, self.pending, self.defaults, &self.defaults_list)
                        .map_err(|error| format!("Could not mix {}: {error}", lane_name(identity)))?;
                    let extension = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_else(|| "wav".into());
                    let bytes = fs::read(&path).map_err(|error| format!("Reading {}: {error}", lane_name(identity)))?;
                    (file.set, file.role, extension, bytes)
                }
                _ => (file.set, file.role, "wav".to_string(), render(cues(file, volume as f32, clips)?, cancel)?),
            };
            files.push(output);
        }
        Ok((volume, files))
    }
}

/// The normal set of the timing line `place`'s object reads.
fn timing_set(
    file: &OsuFile, root: &Path, pending: &Pending, sections: &mut Option<HashMap<dawsu_osu_format::LineId, Option<String>>>, place: Place,
) -> Result<SampleSet, String> {
    if sections.is_none() {
        let listing = listing(root, pending)?;
        *sections = Some(match dawsu_hitsounding::rules::answer(file, &listing) {
            dawsu_hitsounding::rules::Answer::Objects(list) => list.into_iter().map(|a| (a.line, a.sample_point.map(|p| p.bank))).collect(),
            dawsu_hitsounding::rules::Answer::UnsupportedMode { .. } => HashMap::new(),
        });
    }
    let line = mix_place(place).map(|(object, _)| object.line());
    let bank = line.and_then(|line| sections.as_ref()?.get(&line).cloned().flatten());
    Ok(match bank.as_deref() {
        Some("soft") => SampleSet::Soft,
        Some("drum") => SampleSet::Drum,
        _ => SampleSet::Normal,
    })
}

fn reusable_export_index(
    root: &Path,
    waves: &[(SampleSet, Hitsound, Vec<u8>)],
    previous: &BTreeSet<&str>,
) -> Option<u32> {
    let (bank, role, _) = waves.first()?;
    let prefix = export_settings::slot_name(*bank, *role);
    previous
        .iter()
        .filter_map(|name| {
            name.strip_prefix(prefix)?
                .strip_suffix(".wav")?
                .parse::<u32>()
                .ok()
        })
        .find(|index| {
            *index > 1
                && *index <= i32::MAX as u32
                && waves.iter().all(|(bank, role, wav)| {
                    let name = format!("{}{index}.wav", export_settings::slot_name(*bank, *role));
                    previous.contains(name.as_str()) && has_contents(root, &name, wav)
                })
        })
}

fn replace_moment(
    target: &mut Hitsounds,
    moment: MomentId,
    samples: impl IntoIterator<Item = SampleIdentity>,
) {
    for identity in target.moment(moment).hits().cloned().collect::<Vec<_>>() {
        target.remove_hit(moment, &identity);
    }
    for sample in samples {
        target.add_hit(moment, sample);
    }
    // The layer volumes are already baked into the WAV.
    target.set_volume(moment, 100);
}

struct IndexedMixes {
    reserved: BTreeSet<u32>,
    next: u32,
    reuse: BTreeSet<u32>,
}

fn indexed_name(index: u32) -> String {
    format!("normal-hitnormal{index}.wav")
}

fn has_contents(root: &Path, name: &str, wav: &[u8]) -> bool {
    crate::safe_child(root, name)
        .ok()
        .and_then(|path| fs::read(path).ok())
        .is_some_and(|bytes| bytes == wav)
}

/// The custom indexes a new mix must not take: those the mapset's files and maps use, and those
/// of pending files and of names claimed for them.
fn occupied_indexes(root: &Path, pending: &Pending) -> Result<BTreeSet<u32>, String> {
    use dawsu_hitsounding::rules::read_sample_file_name;
    let files = listing(root, pending)?;
    let (mut reserved, _) = crate::layers::reservations(root, &files)?;
    reserved.extend(pending.taken_indexes());
    reserved.extend(files.iter().filter_map(|name| {
        match read_sample_file_name(name.rsplit(['/', '\\']).next()?)? {
            SampleIdentity::Standard { index, .. } => Some(index.get()),
            SampleIdentity::File(_) => None,
        }
    }));
    Ok(reserved)
}

impl IndexedMixes {
    fn read(root: &Path, expected: &OsuFile, already_staged: &[StagedMix], start_index: u32, pending: &Pending) -> Result<Self, String> {
        let mut reserved = occupied_indexes(root, pending)?;
        reserved.extend(already_staged.iter().filter_map(|mix| mix.index));
        let reuse = expected
            .timing_points()
            .iter()
            .filter_map(|point| point.sample_index())
            .filter(|index| *index > 1)
            .map(|index| index as u32)
            .collect();
        Ok(Self {
            reserved,
            next: start_index,
            reuse,
        })
    }

    fn allocate(&mut self, root: &Path, wav: &[u8]) -> Result<(u32, bool), String> {
        // A referenced WAV belongs to the version we read. Revalidate it when publishing.
        if let Some(&existing) = self
            .reuse
            .iter()
            .find(|index| has_contents(root, &indexed_name(**index), wav))
        {
            return Ok((existing, true));
        }
        Ok((self.reserve()?, false))
    }

    /// An index whose files already hold exactly these role files, or a free one. Without a
    /// hitnormal file the index must have no hitnormal file of any set, or osu! plays that
    /// instead of the skin's (lazer 131fe72 LegacySkin.cs:609-633). A free index has no file at all.
    fn allocate_roles(&mut self, root: &Path, pending: &Pending, files: &[RoleOutput]) -> Result<(u32, bool), String> {
        let has_hitnormal = files.iter().any(|(_, role, _, _)| *role == Hitsound::Normal);
        let hitnormals: BTreeSet<u32> = if has_hitnormal {
            BTreeSet::new()
        } else {
            listing(root, pending)?.iter().filter_map(|name| {
                match dawsu_hitsounding::rules::read_sample_file_name(name.rsplit(['/', '\\']).next()?)? {
                    SampleIdentity::Standard { sound: Sound::Hit(Hitsound::Normal), index, .. } => Some(index.get()),
                    _ => None,
                }
            }).collect()
        };
        let hitnormal_free = |index: u32| !hitnormals.contains(&index);
        if let Some(&existing) = self.reuse.iter().find(|index| {
            (has_hitnormal || hitnormal_free(**index))
                && files.iter().all(|(set, role, extension, bytes)| {
                    has_contents(root, &format!("{}{index}.{extension}", export_settings::slot_name(*set, *role)), bytes)
                })
        }) {
            return Ok((existing, true));
        }
        Ok((self.reserve()?, false))
    }

    fn reserve(&mut self) -> Result<u32, String> {
        while self.next <= i32::MAX as u32 && self.reserved.contains(&self.next) {
            self.next += 1;
        }
        if self.next > i32::MAX as u32 {
            return Err("No safe sample index is available for this mix.".into());
        }
        self.reserved.insert(self.next);
        Ok(self.next)
    }
}

pub(super) struct PublishedMixes<'a> {
    staged: &'a [StagedMix],
    created: Vec<usize>,
    /// The ledger and the entries this publish added, dropped again on rollback.
    ledger: Option<(crate::pending::Ledger, Vec<String>)>,
}

/// Writes the staged files, all or none. With a `ledger`, every file this publish creates is listed
/// there before it is created, so dawsu! knows it made the file.
pub(super) fn publish<'a>(
    staged: &'a [StagedMix],
    ledger: Option<&crate::pending::Ledger>,
    cancel: &dyn Fn() -> bool,
) -> Result<PublishedMixes<'a>, String> {
    let mut indexed = staged
        .iter()
        .filter(|mix| !mix.reuse && mix.index.is_some())
        .peekable();
    if let Some(first) = indexed.peek() {
        let root = first.path.parent().ok_or("Missing mapset folder")?;
        // The files being published are not taken by themselves.
        let reserved = occupied_indexes(root, &Pending::default())?;
        if let Some(index) = indexed
            .filter_map(|mix| mix.index)
            .find(|index| reserved.contains(index))
        {
            return Err(format!(
                "Sample index {index} is now used in this mapset. Review again before applying."
            ));
        }
    }
    let mut direct = staged.iter().filter(|mix| !mix.reuse && mix.index.is_none()).peekable();
    if let Some(first) = direct.peek() {
        let root = first.path.parent().ok_or("Missing mapset folder")?;
        let reserved = reserved_mix_names(root, &[], &Pending::default())?;
        if let Some(collision) = direct.find(|mix| {
            mix.path.file_name().is_some_and(|name| name_is_reserved(&name.to_string_lossy(), &reserved))
        }) {
            return Err(format!("{} is now used in this mapset. Review again before applying.", collision.path.display()));
        }
    }
    let ledger = match ledger {
        Some(ledger) => {
            let entries: Vec<crate::pending::PendingSample> = staged.iter().filter(|mix| !mix.reuse)
                .filter_map(|mix| Some(crate::pending::PendingSample { name: mix.path.file_name()?.to_str()?.to_owned(), sha256: mix.digest() }))
                .collect();
            ledger.add(&entries)?;
            Some((ledger.clone(), entries.into_iter().map(|entry| entry.name).collect()))
        }
        None => None,
    };
    let mut publication = PublishedMixes {
        staged,
        created: Vec::new(),
        ledger,
    };
    for (index, mix) in staged.iter().enumerate() {
        if cancel() {
            return Err(publication.failed("Sample mix publishing cancelled".into()));
        }
        if mix.reuse {
            if !fs::read(&mix.path).is_ok_and(|bytes| bytes == mix.wav) {
                return Err(publication.failed(format!(
                    "{} changed since sample review",
                    mix.path.display()
                )));
            }
        } else {
            if let Err(error) = safe_write::create(&mix.path, &mix.wav) {
                return Err(publication.failed(error.to_string()));
            }
            publication.created.push(index);
        }
    }
    Ok(publication)
}

impl PublishedMixes<'_> {
    pub(super) fn keep(mut self) {
        self.created.clear();
        self.ledger = None;
    }

    pub(super) fn failed(mut self, error: String) -> String {
        let kept = self.rollback();
        if kept.is_empty() {
            error
        } else {
            format!("{error}. Sample files kept during rollback: {}", kept.join(", "))
        }
    }

    fn rollback(&mut self) -> Vec<String> {
        let mut kept = Vec::new();
        for index in self.created.drain(..).rev() {
            let mix = &self.staged[index];
            let result = match fs::read(&mix.path) {
                Ok(bytes) if bytes == mix.wav => fs::remove_file(&mix.path),
                Err(error) => Err(error),
                Ok(_) => {
                    kept.push(mix.path.display().to_string());
                    continue;
                }
            };
            if let Err(error) = result
                && error.kind() != std::io::ErrorKind::NotFound
            {
                kept.push(mix.path.display().to_string());
            }
        }
        // A file kept because someone changed it is not dawsu!'s any more.
        if let Some((ledger, names)) = self.ledger.take() {
            ledger.remove(&names);
        }
        kept
    }
}

impl Drop for PublishedMixes<'_> {
    fn drop(&mut self) {
        self.rollback();
    }
}

pub(super) fn reserved_mix_names(root: &Path, staged: &[StagedMix], pending: &Pending) -> Result<BTreeSet<String>, String> {
    let files = listing(root, pending)?;
    let (_, mut names) = crate::layers::reservations(root, &files)?;
    names.extend(pending.taken_names());

    // Directory entries also claim names, including case-only variants on case-sensitive systems.
    for entry in fs::read_dir(root).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        names.insert(entry.file_name().to_string_lossy().to_ascii_lowercase());
    }
    names.extend(staged.iter().filter_map(|mix| {
        mix.path.file_name().map(|name| name.to_string_lossy().to_ascii_lowercase())
    }));
    Ok(names)
}

fn name_is_reserved(name: &str, names: &BTreeSet<String>) -> bool {
    let stem = name.to_ascii_lowercase();
    let stem = stem.strip_suffix(".wav").unwrap_or(&stem);
    ["wav", "ogg", "mp3"].into_iter().any(|extension| names.contains(&format!("{stem}.{extension}")))
}

fn reserve_name(stem: &str, names: &mut BTreeSet<String>) -> Result<String, String> {
    let name = (1..=u32::MAX)
        .map(|number| format!("{stem}{number}.wav"))
        .find(|name| !name_is_reserved(name, names))
        .ok_or("No safe sample filename is available for this mix.")?;
    names.insert(name.clone());
    Ok(name)
}

pub(super) fn render(
    cues: Vec<ScheduledSound>,
    cancel: &dyn Fn() -> bool,
) -> Result<Vec<u8>, String> {
    encode(&mix(cues, cancel)?)
}

/// The cues mixed at the mix sample rate, interleaved, each value within full scale.
fn mix(cues: Vec<ScheduledSound>, cancel: &dyn Fn() -> bool) -> Result<Vec<f32>, String> {
    let at_frame = |ms: f64| (ms * f64::from(SAMPLE_RATE) / 1000.0).ceil() as u64;
    let mut frames = 0;
    for cue in &cues {
        let start = at_frame(cue.at_ms);
        let end = if let Some(end_ms) = cue.end_ms {
            if !end_ms.is_finite() || end_ms <= cue.at_ms {
                return Err("Sample loop must have a finite duration".into());
            }
            at_frame(end_ms)
        } else {
            let source_frames = (cue.clip.samples.len() / cue.clip.channels) as u64;
            let resampled =
                (source_frames * u64::from(SAMPLE_RATE)).div_ceil(u64::from(cue.clip.sample_rate));
            start.saturating_add(resampled)
        };
        frames = frames.max(end);
    }
    let frames = usize::try_from(frames).map_err(|_| "Sample mix is too long")?;
    // The WAV size limit is checked before mixing, so nothing too long is rendered.
    frames
        .checked_mul(CHANNELS * 2)
        .and_then(|size| u32::try_from(size).ok())
        .filter(|size| *size <= u32::MAX - 36)
        .ok_or("Sample mix is too long for a WAV file")?;
    let mut out = Vec::new();
    out.try_reserve_exact(frames * CHANNELS)
        .map_err(|error| error.to_string())?;
    let mut mixer = Mixer::new(SAMPLE_RATE);
    mixer.cues = cues;
    mixer.playing = true;
    let mut block = [0.0; 1024 * CHANNELS];
    for start in (0..frames).step_by(1024) {
        if cancel() {
            return Err("Save cancelled during sample mixing".into());
        }
        let samples = &mut block[..(frames - start).min(1024) * CHANNELS];
        mixer.render(samples, CHANNELS);
        out.extend_from_slice(samples);
    }
    Ok(out)
}

/// A 16-bit PCM WAV of interleaved samples at the mix sample rate.
fn encode(samples: &[f32]) -> Result<Vec<u8>, String> {
    let data_size = u32::try_from(samples.len() * 2).map_err(|_| "Sample mix is too long for a WAV file")?;
    let mut wav = Vec::new();
    wav.try_reserve_exact(44 + data_size as usize)
        .map_err(|error| error.to_string())?;
    wav.extend(b"RIFF");
    wav.extend((36 + data_size).to_le_bytes());
    wav.extend(b"WAVEfmt ");
    wav.extend(16u32.to_le_bytes());
    wav.extend(1u16.to_le_bytes()); // PCM
    wav.extend((CHANNELS as u16).to_le_bytes());
    wav.extend(SAMPLE_RATE.to_le_bytes());
    wav.extend((SAMPLE_RATE * CHANNELS as u32 * 2).to_le_bytes());
    wav.extend((CHANNELS as u16 * 2).to_le_bytes());
    wav.extend(16u16.to_le_bytes());
    wav.extend(b"data");
    wav.extend(data_size.to_le_bytes());
    for value in samples {
        wav.extend(((*value * 32768.0).round() as i16).to_le_bytes());
    }
    Ok(wav)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancelled_publish_reports_a_sample_it_could_not_remove() {
        let scratch = dawsu_test_support::Scratch::new("mix-rollback-report");
        let first = scratch.path("first.wav");
        let second = scratch.path("second.wav");
        let staged = [
            StagedMix { path: first.clone(), wav: vec![1], reuse: false, index: None },
            StagedMix { path: second.clone(), wav: vec![2], reuse: false, index: None },
        ];
        let result = publish(&staged, None, &|| {
            if !first.exists() {
                return false;
            }
            fs::write(&first, b"externally edited").unwrap();
            true
        });
        let error = result.err().expect("publishing was cancelled");
        assert!(error.contains("cancelled"), "{error}");
        assert!(error.contains("first.wav"), "{error}");
        assert_eq!(fs::read(first).unwrap(), b"externally edited");
        assert!(!second.exists());
    }

    #[test]
    fn a_mix_never_takes_a_pending_only_index_or_name() {
        let scratch = dawsu_test_support::Scratch::new("mix-skips-pending");
        let root = scratch.path("mapset");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("normal-hitnormal2.wav"), [0]).unwrap();
        let mut pending = Pending::for_mapset(&scratch.path("data"), &root);
        pending.add("soft-hitclap3.wav", &[1]).unwrap();
        // Another window's claim for the same mapset counts too.
        let mut other = Pending::for_mapset(&scratch.path("data"), &root);
        other.add("drum-hitfinish4.wav", &[2]).unwrap();
        let mut indexes = IndexedMixes::read(&root, &OsuFile::read(Vec::new()).file, &[], 2, &pending).unwrap();
        assert_eq!(indexes.reserve().unwrap(), 5);
        let names = reserved_mix_names(&root, &[], &pending).unwrap();
        assert!(names.contains("soft-hitclap3.wav") && names.contains("drum-hitfinish4.wav"));
        // Publishing checks the mapset alone, so a pending file never blocks its own write.
        assert!(!occupied_indexes(&root, &Pending::default()).unwrap().contains(&3));
    }

    #[test]
    fn staged_mix_preview_uses_the_same_pcm_without_opening_its_output_path() {
        let clip = AudioClip {
            samples: vec![0.25],
            channels: 1,
            sample_rate: SAMPLE_RATE,
        };
        let wav = render(
            vec![ScheduledSound {
                at_ms: 0.0,
                end_ms: None,
                clip: std::sync::Arc::new(clip),
                gain: 0.5,
            }],
            &|| false,
        )
        .unwrap();
        let staged = StagedMix {
            path: PathBuf::new(),
            wav,
            reuse: false,
            index: None,
        };
        let preview = staged.clip();
        assert_eq!(preview.samples, [0.125, 0.125]);
        assert_eq!(preview.channels, CHANNELS);
        assert_eq!(preview.sample_rate, SAMPLE_RATE);
    }
}
