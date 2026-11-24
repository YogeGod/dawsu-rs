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
use dawsu_osu_format::OsuFile;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::num::{NonZeroU8, NonZeroU32};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use vorbis_rs::{VorbisBitrateManagementStrategy, VorbisEncoderBuilder};

const SAMPLE_RATE: u32 = 48_000;
const CHANNELS: usize = 2;
pub(super) const PREFIX: &str = "dawsu-mix-";

/// Audio format for hitsound mix files written during Export.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MixFormat {
    #[default]
    Wav16,
    Wav32f,
    OggVorbis,
}

impl MixFormat {
    /// File extension without leading dot (`"wav"` or `"ogg"`).
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Wav16 | Self::Wav32f => "wav",
            Self::OggVorbis => "ogg",
        }
    }
}

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
    pub(super) index_schema: BTreeMap<String, u32>,
}

#[derive(Clone)]
pub(super) struct StagedMix {
    pub(super) path: PathBuf,
    wav: Vec<u8>,
    reuse: bool,
    index: Option<u32>,
    trimmed_leading_silence_ms: f64,
}

impl StagedMix {
    pub(super) fn is_reused(&self) -> bool {
        self.reuse
    }

    pub(super) fn digest(&self) -> String {
        sha256_hex(&self.wav)
    }

    pub(super) fn wav_bytes(&self) -> &[u8] {
        &self.wav
    }

    pub(super) fn trimmed_leading_silence_ms(&self) -> f64 {
        self.trimmed_leading_silence_ms
    }

    pub(super) fn new_generated<'a>(
        root: &Path,
        wav: Vec<u8>,
        format: MixFormat,
        mut existing: impl Iterator<Item = &'a String>,
        names: &mut BTreeSet<String>,
    ) -> Result<(String, Self), String> {
        Self::new_generated_with_trim(root, wav, 0.0, format, &mut existing, names)
    }

    pub(super) fn new_generated_with_trim<'a>(
        root: &Path,
        wav: Vec<u8>,
        trimmed_leading_silence_ms: f64,
        format: MixFormat,
        mut existing: impl Iterator<Item = &'a String>,
        names: &mut BTreeSet<String>,
    ) -> Result<(String, Self), String> {
        let ext = format.extension();
        let suffix = format!(".{ext}");
        let stem = format!("{PREFIX}{}-", sha256_hex(&wav));
        let reusable = existing.find(|name| {
            name.starts_with(&stem) && name.ends_with(&suffix) && has_contents(root, name, &wav)
        });
        let name = match reusable {
            Some(name) => name.clone(),
            None => reserve_name(&stem, ext, names)?,
        };
        let mix = Self {
            path: root.join(&name),
            wav,
            reuse: reusable.is_some(),
            index: None,
            trimmed_leading_silence_ms,
        };
        Ok((name, mix))
    }

    /// A pending file a Save writes as it is, checked by its name when published. A file another
    /// program put under the same name is used when it holds the same bytes; other bytes make
    /// the publish refuse and name it.
    pub(super) fn pending(root: &Path, name: &str, bytes: Vec<u8>) -> Self {
        let path = root.join(name);
        let reuse = fs::read(&path).is_ok_and(|on_disk| on_disk == bytes);
        Self { path, wav: bytes, reuse, index: None, trimmed_leading_silence_ms: 0.0 }
    }

    pub(super) fn clip(&self) -> AudioClip {
        if self.wav.len() >= 44 && &self.wav[0..4] == b"RIFF" && &self.wav[8..12] == b"WAVE" {
            let format_tag = u16::from_le_bytes([self.wav[20], self.wav[21]]);
            if format_tag == 1 {
                let samples = self.wav[44..]
                    .chunks_exact(2)
                    .map(|frame| f32::from(i16::from_le_bytes([frame[0], frame[1]])) / 32768.0)
                    .collect();
                return AudioClip {
                    samples,
                    channels: CHANNELS,
                    sample_rate: SAMPLE_RATE,
                };
            }
            if format_tag == 3 {
                let samples = self.wav[44..]
                    .chunks_exact(4)
                    .map(|frame| f32::from_le_bytes([frame[0], frame[1], frame[2], frame[3]]))
                    .collect();
                return AudioClip {
                    samples,
                    channels: CHANNELS,
                    sample_rate: SAMPLE_RATE,
                };
            }
        }
        let ext = self.path.extension().and_then(|e| e.to_str()).or_else(|| {
            self.wav.starts_with(b"OggS").then_some("ogg")
        });
        dawsu_audio::decode_bytes(self.wav.clone(), ext)
            .map(|clip| (*clip).clone())
            .unwrap_or(AudioClip {
                samples: Vec::new(),
                channels: CHANNELS,
                sample_rate: SAMPLE_RATE,
            })
    }
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
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
    format: MixFormat,
    pending: &Pending,
    cancel: &dyn Fn() -> bool,
) -> Result<(OsuFile, MixFiles), String> {
    stage_in_file_with_options(
        source,
        file,
        problems,
        root,
        defaults,
        expected,
        placement,
        already_staged,
        settings,
        previous_mixes,
        start_index,
        format,
        pending,
        false,
        &BTreeMap::new(),
        cancel,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn stage_in_file_with_options(
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
    format: MixFormat,
    pending: &Pending,
    auto_fix: bool,
    index_schema: &BTreeMap<String, u32>,
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
            already_staged,
            start_index,
            format,
            previous_mixes,
            pending,
            auto_fix,
            index_schema,
            cancel,
        )?;
    } else {
        files.prepare_with_options(
            source,
            &mut hits,
            problems,
            root,
            defaults,
            expected,
            already_staged,
            start_index,
            format,
            pending,
            auto_fix,
            index_schema,
            cancel,
        )?;
    }
    match compile(&file, &hits, placement, cancel, &mut |_, _| {}) {
        Compiled::Done { file, problems } if problems.is_empty() => {
            let mut after_listing = listing;
            after_listing.extend(
                files
                    .staged
                    .iter()
                    .filter_map(|mix| mix.path.file_name()?.to_str().map(str::to_owned)),
            );
            after_listing.sort();
            after_listing.dedup();
            dawsu_hitsounding::verify_compiled_osu(&file.to_bytes(), &hits, &after_listing)
                .map_err(|mismatch| mismatch.to_string())?;
            Ok((file, files))
        }
        Compiled::Done { problems, .. } => Err(format!(
            "Could not make the sample mix at {:.0} ms: {}",
            problems[0].time.ms(),
            crate::problem_description(&problems[0].kind, problems[0].time.ms()),
        )),
        Compiled::Cancelled => Err("Sample mix preparation cancelled".into()),
    }
}

impl MixFiles {
    #[allow(clippy::too_many_arguments)]
    fn prepare_export(
        &mut self,
        source: &Hitsounds,
        target: &mut Hitsounds,
        problems: &[SaveProblem],
        root: &Path,
        defaults: &Path,
        settings: &ExportSettings,
        already_staged: &[StagedMix],
        start_index: u32,
        format: MixFormat,
        // The target's reuse set: mix files it already references, reused when their sound matches.
        previous_mixes: &[String],
        pending: &Pending,
        auto_fix: bool,
        index_schema: &BTreeMap<String, u32>,
        cancel: &dyn Fn() -> bool,
    ) -> Result<(), String> {
        settings.validate()?;
        let rules = export_settings::Rules::new(settings);
        let defaults_list = crate::pending::sample_folder(defaults);
        let moments: HashMap<_, _> = target
            .moments()
            .iter()
            .enumerate()
            .filter_map(|(index, moment)| Some((mix_place(moment.place())?, MomentId(index))))
            .collect();
        let mut indexes = IndexedMixes::read_with_schema(
            root,
            &OsuFile::read(Vec::new()).file,
            already_staged,
            start_index,
            pending,
            index_schema.clone(),
        )?;
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
            fn hitnormal<'a>(mut hits: impl Iterator<Item = &'a SampleIdentity>) -> Option<SampleSet> {
                hits.find_map(|identity| match identity {
                    SampleIdentity::Standard {
                        set,
                        sound: Sound::Hit(Hitsound::Normal),
                        ..
                    } => Some(*set),
                    _ => None,
                })
            }
            // Without a hitnormal sound the skin's hitnormal plays in the circle's own normal set,
            // as Save keeps it, so circles of different sets do not share a result.
            let object_normal = (settings.pair_samples && hitnormal(identities.iter()).is_none())
                .then(|| hitnormal(target.moment(moment).hits()))
                .flatten();
            let key = (destination, layers.as_slice(), object_normal);
            let outputs = if let Some(outputs) = rendered.get(&key) {
                outputs
            } else {
                let (bank, fused_role) = export_settings::slot(destination)?;
                let normal_bank = if settings.pair_samples {
                    hitnormal(identities.iter()).or(object_normal).unwrap_or(bank)
                } else {
                    bank
                };
                let mut components = BTreeMap::<Hitsound, Vec<ScheduledSound>>::new();
                components.entry(Hitsound::Normal).or_default();
                let has_normal_layer = settings.pair_samples
                    && layers.iter().any(|(lane, _)| {
                        !matches!(
                            lane,
                            SampleIdentity::Standard {
                                sound: Sound::Hit(
                                    Hitsound::Whistle | Hitsound::Finish | Hitsound::Clap
                                ),
                                ..
                            }
                        )
                    });
                let object_volume = if settings.pair_samples && !has_normal_layer {
                    layers
                        .iter()
                        .map(|(_, v)| *v)
                        .max()
                        .unwrap_or(100)
                        .clamp(5, 100)
                } else {
                    100
                };
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
                        gain: *volume as f32 / object_volume as f32,
                    });
                }
                let roles: Vec<_> = components
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
                let slots: Vec<_> = roles.iter().map(|(bank, role, _)| (*bank, *role)).collect();
                // Paired samples write no hitnormal file for a moment without one: the object keeps
                // its normal set and the player's skin plays it, as Save does (Yoge 2026-11-20).
                let roles = roles
                    .into_iter()
                    .filter(|(_, _, cues)| !(settings.pair_samples && cues.is_empty()))
                    .collect();
                let waves = render_roles(roles, format, auto_fix, cancel)?;
                let wave_bytes: Vec<(SampleSet, Hitsound, Vec<u8>)> = waves
                    .iter()
                    .map(|(bank, role, rendered)| (*bank, *role, rendered.bytes.clone()))
                    .collect();
                let (index, reused) =
                    indexes.allocate_export(root, pending, &wave_bytes, format, &previous_mixes)?;
                let outputs: Vec<_> = slots
                    .iter()
                    .map(|(bank, role)| SampleIdentity::Standard {
                        set: *bank,
                        sound: Sound::Hit(*role),
                        index: CustomIndex::new(index),
                    })
                    .collect();
                let ext = format.extension();
                self.stage_roles(
                    root,
                    waves.into_iter().map(|(bank, role, rendered)| {
                        (bank, role, ext.to_string(), rendered.bytes, rendered.trimmed_leading_silence_ms)
                    }),
                    index,
                    reused,
                );
                &*rendered.entry(key).or_insert((outputs, object_volume))
            };
            replace_moment_with_volume(target, moment, outputs.0.iter().cloned(), outputs.1);
        }
        self.index_schema = indexes.into_schema();
        Ok(())
    }

    /// Stages the role files at `index`.
    fn stage_roles(
        &mut self,
        root: &Path,
        files: impl IntoIterator<Item = (SampleSet, Hitsound, String, Vec<u8>, f64)>,
        index: u32,
        reuse: bool,
    ) {
        for (bank, role, extension, bytes, trimmed_leading_silence_ms) in files {
            let name = format!("{}{index}.{extension}", export_settings::slot_name(bank, role));
            self.staged.push(StagedMix {
                path: root.join(name),
                wav: bytes,
                reuse,
                index: Some(index),
                trimmed_leading_silence_ms,
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
        let assets = Assets { source, root, pending, defaults, defaults_list: crate::pending::sample_folder(defaults) };
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
                            self.stage_roles(
                                root,
                                files.into_iter().map(|(set, role, ext, bytes)| (set, role, ext, bytes, 0.0)),
                                index,
                                reuse,
                            );
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

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare_with_options(
        &mut self,
        source: &Hitsounds,
        target: &mut Hitsounds,
        problems: &[SaveProblem],
        root: &Path,
        defaults: &Path,
        expected: &[u8],
        already_staged: &[StagedMix],
        start_index: u32,
        format: MixFormat,
        pending: &Pending,
        auto_fix: bool,
        index_schema: &BTreeMap<String, u32>,
        cancel: &dyn Fn() -> bool,
    ) -> Result<(), String> {
        let defaults_list = crate::pending::sample_folder(defaults);
        let expected = OsuFile::read(expected.to_vec()).file;
        let suffix = format!(".{}", format.extension());
        let existing: BTreeSet<_> = expected
            .hit_objects()
            .into_iter()
            .filter_map(|object| object.hit_sample()?.filename.clone())
            .filter(|name| name.starts_with(PREFIX) && name.ends_with(&suffix))
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
            Some(IndexedMixes::read_with_schema(
                root,
                &expected,
                already_staged,
                start_index,
                pending,
                index_schema.clone(),
            )?)
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
                let rendered_mix = render_layers(source, Some(target), layers, root, pending, defaults, &defaults_list, &mut clips, format, auto_fix, cancel)?;
                let (sample, staged) = if let Some(indexes) = indexes.as_mut().filter(|_| slider) {
                    let (index, reuse) = indexes.allocate(root, &rendered_mix.bytes, format)?;
                    let identity = SampleIdentity::Standard {
                        set: SampleSet::Normal,
                        sound: Sound::Hit(Hitsound::Normal),
                        index: CustomIndex::new(index),
                    };
                    (
                        identity,
                        StagedMix {
                            path: root.join(indexed_name(index, format)),
                            wav: rendered_mix.bytes,
                            reuse,
                            index: Some(index),
                            trimmed_leading_silence_ms: rendered_mix.trimmed_leading_silence_ms,
                        },
                    )
                } else {
                    let (name, staged) = StagedMix::new_generated_with_trim(
                        root,
                        rendered_mix.bytes,
                        rendered_mix.trimmed_leading_silence_ms,
                        format,
                        existing.iter(),
                        &mut names,
                    )?;
                    (SampleIdentity::File(name), staged)
                };
                self.staged.push(staged);
                rendered.insert(key, sample.clone());
                sample
            };
            replace_moment(target, moment, [sample]);
        }
        if let Some(indexes) = indexes {
            self.index_schema = indexes.into_schema();
        }
        Ok(())
    }
}

struct RenderedMix {
    bytes: Vec<u8>,
    trimmed_leading_silence_ms: f64,
}

#[allow(clippy::too_many_arguments)]
fn render_layers(
    source: &Hitsounds, target: Option<&Hitsounds>, layers: &[(SampleIdentity, i32)],
    root: &Path, pending: &Pending, defaults: &Path, defaults_list: &[String],
    clips: &mut HashMap<SampleIdentity, Arc<AudioClip>>, format: MixFormat, auto_fix: bool, cancel: &dyn Fn() -> bool,
) -> Result<RenderedMix, String> {
    let mut cues = Vec::with_capacity(layers.len());
    for (identity, volume) in layers {
        cues.push(ScheduledSound {
            at_ms: 0.0,
            end_ms: None,
            clip: load_clip(source, target, identity, root, pending, defaults, defaults_list, clips, cancel)?,
            gain: *volume as f32 / 100.0,
        });
    }
    render_with_options(cues, format, auto_fix, cancel)
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

type RenderedRoleWaves = Vec<(SampleSet, Hitsound, RenderedMix)>;

/// One WAV per role, each sound at its cue's gain. A role with no sound is written silent: a
/// missing numbered hitnormal would fall back to the skin's plain one (lazer 131fe72:
/// LegacyBeatmapSkin.cs:22, LegacySkin.cs:621-624, SkinManager.cs:314). Export writes these;
/// with paired samples it passes no empty role, since the skin plays a missing hitnormal.
fn render_roles(
    roles: Vec<(SampleSet, Hitsound, Vec<ScheduledSound>)>,
    format: MixFormat,
    auto_fix: bool,
    cancel: &dyn Fn() -> bool,
) -> Result<RenderedRoleWaves, String> {
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
        let rendered = render_with_options(cues, format, auto_fix, cancel)?;
        waves.push((bank, role, rendered));
    }
    Ok(waves)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn stage_timed_copy_with_options(
    source: &Hitsounds, file: &mut OsuFile, path: &Path, defaults: &Path,
    placements: &[dawsu_hitsounding::copy::SliderCopyPlacement],
    staged: &mut Vec<StagedMix>, details: &mut Vec<String>, pending: &Pending,
    format: MixFormat, auto_fix: bool, cancel: &dyn Fn() -> bool,
    // Mixes before this index belong to an earlier part of the write (Generate): their names stay
    // reserved, but a target never points at them.
    own_from: usize,
) -> Result<(), String> {
    if placements.is_empty() {
        return Ok(());
    }
    let root = path.parent().ok_or("Missing mapset folder")?;
    let defaults_list = crate::pending::sample_folder(defaults);
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
            let rendered_mix = render_layers(source, None, layers, root, pending, defaults, &defaults_list, &mut clips, format, auto_fix, cancel)?;
            let name = if let Some(previous) = staged[own_from..].iter().find(|mix| mix.index.is_none() && mix.wav == rendered_mix.bytes) {
                previous.path.file_name().ok_or("Missing staged filename")?.to_string_lossy().into_owned()
            } else {
                let (name, mix) = StagedMix::new_generated_with_trim(
                    root,
                    rendered_mix.bytes,
                    rendered_mix.trimmed_leading_silence_ms,
                    format,
                    existing.iter(),
                    &mut names,
                )?;
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
    pending: &Pending,
    waves: &[(SampleSet, Hitsound, Vec<u8>)],
    format: MixFormat,
    previous: &BTreeSet<&str>,
) -> Result<Option<u32>, String> {
    let Some((bank, role, _)) = waves.first() else { return Ok(None) };
    let prefix = export_settings::slot_name(*bank, *role);
    let suffix = format!(".{}", format.extension());
    let hitnormals = hitnormal_indexes(root, pending, waves.iter().any(|(_, role, _)| *role == Hitsound::Normal))?;
    Ok(previous
        .iter()
        .filter_map(|name| {
            name.strip_prefix(prefix)?
                .strip_suffix(&suffix)?
                .parse::<u32>()
                .ok()
        })
        .find(|index| {
            *index > 1
                && *index <= i32::MAX as u32
                && !hitnormals.contains(index)
                && waves.iter().all(|(bank, role, wav)| {
                    let name = format!("{}{index}{suffix}", export_settings::slot_name(*bank, *role));
                    previous.contains(name.as_str()) && has_contents(root, &name, wav)
                })
        }))
}

fn replace_moment(
    target: &mut Hitsounds,
    moment: MomentId,
    samples: impl IntoIterator<Item = SampleIdentity>,
) {
    replace_moment_with_volume(target, moment, samples, 100);
}

fn replace_moment_with_volume(
    target: &mut Hitsounds,
    moment: MomentId,
    samples: impl IntoIterator<Item = SampleIdentity>,
    volume: i32,
) {
    for identity in target.moment(moment).hits().cloned().collect::<Vec<_>>() {
        target.remove_hit(moment, &identity);
    }
    for sample in samples {
        target.add_hit(moment, sample);
    }
    // The layer volumes are baked relative to `volume`.
    target.set_volume(moment, volume);
}

fn role_waves_digest(waves: &[(SampleSet, Hitsound, Vec<u8>)], format: MixFormat) -> String {
    let mut data = Vec::new();
    for (bank, role, bytes) in waves {
        data.extend_from_slice(export_settings::slot_name(*bank, *role).as_bytes());
        data.push(b'.');
        data.extend_from_slice(format.extension().as_bytes());
        data.push(b':');
        data.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
        data.extend_from_slice(bytes);
    }
    sha256_hex(&data)
}

struct IndexedMixes {
    reserved: BTreeSet<u32>,
    next: u32,
    reuse: BTreeSet<u32>,
    /// Persistent map of mix content SHA-256 hex -> preferred custom index.
    schema: BTreeMap<String, u32>,
    /// Indices claimed by entries in `schema` so newly inserted mixes skip over them.
    schema_indices: BTreeSet<u32>,
}

fn indexed_name(index: u32, format: MixFormat) -> String {
    format!("normal-hitnormal{index}.{}", format.extension())
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

/// The indexes with a hitnormal file of any set, which files without a hitnormal must not reuse:
/// osu! would play that file instead of the skin's (lazer 131fe72 LegacySkin.cs:609-633).
/// Empty when `has_hitnormal`, since the written hitnormal replaces it.
fn hitnormal_indexes(root: &Path, pending: &Pending, has_hitnormal: bool) -> Result<BTreeSet<u32>, String> {
    if has_hitnormal {
        return Ok(BTreeSet::new());
    }
    Ok(listing(root, pending)?.iter().filter_map(|name| {
        match dawsu_hitsounding::rules::read_sample_file_name(name.rsplit(['/', '\\']).next()?)? {
            SampleIdentity::Standard { sound: Sound::Hit(Hitsound::Normal), index, .. } => Some(index.get()),
            _ => None,
        }
    }).collect())
}

impl IndexedMixes {
    fn read(root: &Path, expected: &OsuFile, already_staged: &[StagedMix], start_index: u32, pending: &Pending) -> Result<Self, String> {
        Self::read_with_schema(root, expected, already_staged, start_index, pending, BTreeMap::new())
    }

    fn read_with_schema(
        root: &Path,
        expected: &OsuFile,
        already_staged: &[StagedMix],
        start_index: u32,
        pending: &Pending,
        schema: BTreeMap<String, u32>,
    ) -> Result<Self, String> {
        let mut reserved = occupied_indexes(root, pending)?;
        reserved.extend(already_staged.iter().filter_map(|mix| mix.index));
        let reuse = expected
            .timing_points()
            .iter()
            .filter_map(|point| point.sample_index())
            .filter(|index| *index > 1)
            .map(|index| index as u32)
            .collect();
        let schema_indices = schema.values().copied().filter(|&index| index > 1 && index <= i32::MAX as u32).collect();
        Ok(Self {
            reserved,
            next: start_index,
            reuse,
            schema,
            schema_indices,
        })
    }

    fn into_schema(self) -> BTreeMap<String, u32> {
        self.schema
    }

    fn allocate_export(
        &mut self,
        root: &Path,
        pending: &Pending,
        waves: &[(SampleSet, Hitsound, Vec<u8>)],
        format: MixFormat,
        previous: &BTreeSet<&str>,
    ) -> Result<(u32, bool), String> {
        let digest = role_waves_digest(waves, format);
        if let Some(reused) = reusable_export_index(root, pending, waves, format, previous)? {
            self.reserved.insert(reused);
            self.schema.insert(digest, reused);
            self.schema_indices.insert(reused);
            return Ok((reused, true));
        }
        if let Some(&preferred) = self.schema.get(&digest)
            && preferred > 1
            && preferred <= i32::MAX as u32
        {
            let suffix = format!(".{}", format.extension());
            let hitnormals = hitnormal_indexes(root, pending, waves.iter().any(|(_, role, _)| *role == Hitsound::Normal))?;
            if !hitnormals.contains(&preferred)
                && waves.iter().all(|(bank, role, wav)| {
                    let name = format!("{}{preferred}{suffix}", export_settings::slot_name(*bank, *role));
                    has_contents(root, &name, wav)
                })
            {
                self.reserved.insert(preferred);
                return Ok((preferred, true));
            }
            if !self.reserved.contains(&preferred) {
                self.reserved.insert(preferred);
                return Ok((preferred, false));
            }
        }
        let index = self.reserve_unclaimed()?;
        self.schema.insert(digest, index);
        self.schema_indices.insert(index);
        Ok((index, false))
    }

    fn allocate(&mut self, root: &Path, wav: &[u8], format: MixFormat) -> Result<(u32, bool), String> {
        let digest = sha256_hex(wav);
        // A referenced mix belongs to the version we read. Revalidate it when publishing.
        if let Some(&existing) = self
            .reuse
            .iter()
            .find(|index| has_contents(root, &indexed_name(**index, format), wav))
        {
            self.reserved.insert(existing);
            self.schema.insert(digest, existing);
            self.schema_indices.insert(existing);
            return Ok((existing, true));
        }
        if let Some(&preferred) = self.schema.get(&digest)
            && preferred > 1
            && preferred <= i32::MAX as u32
        {
            if has_contents(root, &indexed_name(preferred, format), wav) {
                self.reserved.insert(preferred);
                return Ok((preferred, true));
            }
            if !self.reserved.contains(&preferred) {
                self.reserved.insert(preferred);
                return Ok((preferred, false));
            }
        }
        let index = self.reserve_unclaimed()?;
        self.schema.insert(digest, index);
        self.schema_indices.insert(index);
        Ok((index, false))
    }

    /// An index whose files already hold exactly these role files, or a free one. Without a
    /// hitnormal file the index must have no hitnormal file of any set, or osu! plays that
    /// instead of the skin's (lazer 131fe72 LegacySkin.cs:609-633). A free index has no file at all.
    fn allocate_roles(&mut self, root: &Path, pending: &Pending, files: &[RoleOutput]) -> Result<(u32, bool), String> {
        let hitnormals = hitnormal_indexes(root, pending, files.iter().any(|(_, role, _, _)| *role == Hitsound::Normal))?;
        if let Some(&existing) = self.reuse.iter().find(|index| {
            !hitnormals.contains(index)
                && files.iter().all(|(set, role, extension, bytes)| {
                    has_contents(root, &format!("{}{index}.{extension}", export_settings::slot_name(*set, *role)), bytes)
                })
        }) {
            return Ok((existing, true));
        }
        Ok((self.reserve()?, false))
    }

    /// Reserves the next free index, skipping both occupied indices and indices reserved by
    /// `schema_indices` for other known mixes. Falls back to `reserve()` if needed.
    fn reserve_unclaimed(&mut self) -> Result<u32, String> {
        let mut candidate = self.next;
        while candidate <= i32::MAX as u32
            && (self.reserved.contains(&candidate) || self.schema_indices.contains(&candidate))
        {
            candidate += 1;
        }
        if candidate <= i32::MAX as u32 {
            self.reserved.insert(candidate);
            self.next = candidate + 1;
            return Ok(candidate);
        }
        self.reserve()
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
    staged: Vec<&'a StagedMix>,
    created: Vec<usize>,
    /// The ledger, the entries this publish added, and the earlier entries those replaced:
    /// a rollback drops its own and puts the earlier ones back.
    ledger: Option<(crate::pending::Ledger, Vec<String>, Vec<crate::pending::PendingSample>)>,
}

/// Writes the staged files, all or none. With a `ledger`, every file this publish creates is listed
/// there before it is created, so dawsu! knows it made the file.
pub(super) fn publish<'a>(
    staged: impl IntoIterator<Item = &'a StagedMix>,
    ledger: Option<&crate::pending::Ledger>,
    cancel: &dyn Fn() -> bool,
) -> Result<PublishedMixes<'a>, String> {
    let staged: Vec<&'a StagedMix> = staged.into_iter().collect();
    // One index holds one sound per file: a file staged twice in a plan must be the same
    // reused WAV, never two different sounds or two new writes.
    let mut files = HashMap::<String, &StagedMix>::new();
    for mix in staged.iter().filter(|mix| mix.index.is_some()) {
        let name = mix.path.to_string_lossy().to_lowercase();
        if let Some(earlier) = files.insert(name, mix)
            && (earlier.wav != mix.wav || !(earlier.reuse && mix.reuse))
        {
            return Err(format!(
                "Sample index {} would hold two different sounds. Review again before applying.",
                mix.index.unwrap_or_default()
            ));
        }
    }
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
            let replaced = ledger.add(&entries)?;
            Some((ledger.clone(), entries.into_iter().map(|entry| entry.name).collect(), replaced))
        }
        None => None,
    };
    let mut publication = PublishedMixes {
        staged: staged.clone(),
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
        // Files that stay because they couldn't be removed (a lock), unless confirmed changed:
        // their ledger entries stay, so a later cleanup tries again.
        let mut still_ours = Vec::new();
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
                // Only a file confirmed changed leaves the ledger; one that can't be read stays listed.
                let changed = fs::read(&mix.path).is_ok_and(|bytes| bytes != mix.wav);
                if !changed && let Some(name) = mix.path.file_name() {
                    still_ours.push(name.to_string_lossy().into_owned());
                }
            }
        }
        // A file removed, never made, or changed by someone else leaves the ledger.
        if let Some((ledger, names, replaced)) = self.ledger.take() {
            let gone: Vec<String> = names.into_iter()
                .filter(|name| !still_ours.iter().any(|ours| ours.eq_ignore_ascii_case(name))).collect();
            let _ = ledger.update(|entries| {
                entries.retain(|entry| !gone.iter().any(|name| name.eq_ignore_ascii_case(&entry.name)));
                entries.extend(replaced.into_iter().filter(|earlier| gone.iter().any(|name| name.eq_ignore_ascii_case(&earlier.name))));
            });
        }
        kept
    }
}

impl Drop for PublishedMixes<'_> {
    fn drop(&mut self) {
        self.rollback();
    }
}

pub(crate) fn reserved_mix_names(root: &Path, staged: &[StagedMix], pending: &Pending) -> Result<BTreeSet<String>, String> {
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

pub(crate) fn name_is_reserved(name: &str, names: &BTreeSet<String>) -> bool {
    let lower = name.to_ascii_lowercase();
    if names.contains(&lower) {
        return true;
    }
    let stem = lower
        .strip_suffix(".wav")
        .or_else(|| lower.strip_suffix(".ogg"))
        .or_else(|| lower.strip_suffix(".mp3"))
        .unwrap_or(&lower);
    ["wav", "ogg", "mp3"]
        .into_iter()
        .any(|extension| names.contains(&format!("{stem}.{extension}")))
}

fn reserve_name(stem: &str, extension: &str, names: &mut BTreeSet<String>) -> Result<String, String> {
    let name = (1..=u32::MAX)
        .map(|number| format!("{stem}{number}.{extension}"))
        .find(|name| !name_is_reserved(name, names))
        .ok_or("No safe sample filename is available for this mix.")?;
    names.insert(name.clone());
    Ok(name)
}

pub(super) fn render(
    cues: Vec<ScheduledSound>,
    cancel: &dyn Fn() -> bool,
) -> Result<Vec<u8>, String> {
    Ok(render_with_options(cues, MixFormat::Wav16, false, cancel)?.bytes)
}

pub(super) fn render_format(
    cues: Vec<ScheduledSound>,
    format: MixFormat,
    cancel: &dyn Fn() -> bool,
) -> Result<Vec<u8>, String> {
    Ok(render_with_options(cues, format, false, cancel)?.bytes)
}

fn render_with_options(
    cues: Vec<ScheduledSound>,
    format: MixFormat,
    auto_fix: bool,
    cancel: &dyn Fn() -> bool,
) -> Result<RenderedMix, String> {
    let mixed = mix(cues, cancel)?;
    if auto_fix && format == MixFormat::Wav16 {
        let fixed = crate::sample_checks::auto_fix_wav_pcm(&mixed, SAMPLE_RATE, CHANNELS)?;
        Ok(RenderedMix {
            bytes: fixed.wav,
            trimmed_leading_silence_ms: fixed.trimmed_ms,
        })
    } else {
        Ok(RenderedMix {
            bytes: encode_format(&mixed, format)?,
            trimmed_leading_silence_ms: 0.0,
        })
    }
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
    mixer.limiter_enabled = false;
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

fn encode_format(samples: &[f32], format: MixFormat) -> Result<Vec<u8>, String> {
    match format {
        MixFormat::Wav16 => encode_wav16(samples),
        MixFormat::Wav32f => encode_wav32f(samples),
        MixFormat::OggVorbis => encode_ogg_vorbis(samples),
    }
}

/// A 16-bit PCM WAV of interleaved samples at the mix sample rate.
fn encode_wav16(samples: &[f32]) -> Result<Vec<u8>, String> {
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

/// An IEEE 754 32-bit float WAV of interleaved samples at the mix sample rate.
fn encode_wav32f(samples: &[f32]) -> Result<Vec<u8>, String> {
    let data_size = samples
        .len()
        .checked_mul(4)
        .and_then(|size| u32::try_from(size).ok())
        .filter(|size| *size <= u32::MAX - 36)
        .ok_or("Sample mix is too long for a WAV file")?;
    let mut wav = Vec::new();
    wav.try_reserve_exact(44 + data_size as usize)
        .map_err(|error| error.to_string())?;
    wav.extend(b"RIFF");
    wav.extend((36 + data_size).to_le_bytes());
    wav.extend(b"WAVEfmt ");
    wav.extend(16u32.to_le_bytes());
    wav.extend(3u16.to_le_bytes()); // IEEE float
    wav.extend((CHANNELS as u16).to_le_bytes());
    wav.extend(SAMPLE_RATE.to_le_bytes());
    wav.extend((SAMPLE_RATE * CHANNELS as u32 * 4).to_le_bytes());
    wav.extend((CHANNELS as u16 * 4).to_le_bytes());
    wav.extend(32u16.to_le_bytes());
    wav.extend(b"data");
    wav.extend(data_size.to_le_bytes());
    for value in samples {
        wav.extend(value.to_le_bytes());
    }
    Ok(wav)
}

/// An Ogg Vorbis stream of interleaved samples at the mix sample rate.
/// Uses a fixed stream serial so identical mixes produce byte-identical files for reuse.
fn encode_ogg_vorbis(samples: &[f32]) -> Result<Vec<u8>, String> {
    const STREAM_SERIAL: i32 = 1;
    const QUALITY: f32 = 0.6;
    const BLOCK_FRAMES: usize = 1024;

    let sample_rate = NonZeroU32::new(SAMPLE_RATE).expect("SAMPLE_RATE > 0");
    let channels = NonZeroU8::new(CHANNELS as u8).expect("CHANNELS > 0");
    let mut encoder =
        VorbisEncoderBuilder::new_with_serial(sample_rate, channels, Vec::new(), STREAM_SERIAL)
            .bitrate_management_strategy(VorbisBitrateManagementStrategy::QualityVbr {
                target_quality: QUALITY,
            })
            .build()
            .map_err(|error| format!("Could not initialize Ogg Vorbis encoder: {error}"))?;

    if samples.is_empty() {
        encoder
            .encode_audio_block([&[0.0_f32][..], &[0.0_f32][..]])
            .map_err(|error| format!("Could not encode Ogg Vorbis audio: {error}"))?;
    } else {
        let mut left = Vec::with_capacity(BLOCK_FRAMES);
        let mut right = Vec::with_capacity(BLOCK_FRAMES);
        for chunk in samples.chunks(BLOCK_FRAMES * CHANNELS) {
            left.clear();
            right.clear();
            for frame in chunk.chunks_exact(CHANNELS) {
                left.push(frame[0]);
                right.push(frame[1]);
            }
            if !left.is_empty() {
                encoder
                    .encode_audio_block([left.as_slice(), right.as_slice()])
                    .map_err(|error| format!("Could not encode Ogg Vorbis audio: {error}"))?;
            }
        }
    }

    encoder
        .finish()
        .map_err(|error| format!("Could not finish Ogg Vorbis stream: {error}"))
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
            StagedMix { path: first.clone(), wav: vec![1], reuse: false, index: None, trimmed_leading_silence_ms: 0.0 },
            StagedMix { path: second.clone(), wav: vec![2], reuse: false, index: None, trimmed_leading_silence_ms: 0.0 },
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
    fn a_rollback_keeps_the_ledger_entry_of_a_file_it_could_not_remove() {
        let scratch = dawsu_test_support::Scratch::new("mix-rollback-locked");
        let root = scratch.path("mapset");
        fs::create_dir(&root).unwrap();
        let ledger = Pending::for_mapset(&scratch.path("data"), &root).ledger().unwrap();
        let staged = [
            StagedMix { path: root.join("dawsu-mix-a-1.wav"), wav: vec![1], reuse: false, index: None, trimmed_leading_silence_ms: 0.0 },
            StagedMix { path: root.join("dawsu-mix-b-1.wav"), wav: vec![2], reuse: false, index: None, trimmed_leading_silence_ms: 0.0 },
        ];
        let publication = publish(&staged, Some(&ledger), &|| false).unwrap();
        // One file can't be removed: it stays, and so does its entry, so a cleanup can retry it.
        // Windows: another program holds it open without sharing deletion. Unix: the folder is read-only.
        let locked = &staged[0].path;
        #[cfg(windows)]
        let holder = {
            use std::os::windows::fs::OpenOptionsExt;
            fs::OpenOptions::new().read(true).share_mode(1).open(locked).unwrap()
        };
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&root, fs::Permissions::from_mode(0o555)).unwrap();
        }
        drop(publication);
        #[cfg(windows)]
        drop(holder);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).unwrap();
        }
        assert!(locked.exists(), "locked stays");
        assert!(!staged[1].path.exists(), "the other goes");
        let names: Vec<_> = ledger.entries().unwrap().into_iter().map(|entry| entry.name).collect();
        assert_eq!(names, ["dawsu-mix-a-1.wav"]);
    }

    #[test]
    fn a_rollback_keeps_an_entry_the_ledger_had_before_the_publish() {
        let scratch = dawsu_test_support::Scratch::new("mix-rollback-earlier-entry");
        let root = scratch.path("mapset");
        fs::create_dir(&root).unwrap();
        let ledger = Pending::for_mapset(&scratch.path("data"), &root).ledger().unwrap();
        // The ledger already lists c.wav from an earlier write; this publish lists it again and
        // then fails on its second file.
        ledger.add(&[crate::pending::PendingSample { name: "c.wav".into(), sha256: sha256_hex(&[7]) }]).unwrap();
        let staged = [
            StagedMix { path: root.join("c.wav"), wav: vec![3], reuse: false, index: None, trimmed_leading_silence_ms: 0.0 },
            StagedMix { path: root.join("c.wav"), wav: vec![3], reuse: false, index: None, trimmed_leading_silence_ms: 0.0 },
        ];
        assert!(publish(&staged, Some(&ledger), &|| false).is_err());
        let entries: Vec<_> = ledger.entries().unwrap().into_iter().map(|entry| (entry.name, entry.sha256)).collect();
        assert_eq!(entries, [("c.wav".to_string(), sha256_hex(&[7]))], "the earlier entry is back as it was");
        assert!(!root.join("c.wav").exists());
    }

    #[test]
    fn publish_refuses_one_index_file_for_two_sounds() {
        let scratch = dawsu_test_support::Scratch::new("mix-index-twice");
        let path = scratch.path("normal-hitnormal2.wav");
        let mix = |wav: u8, reuse: bool| StagedMix { path: path.clone(), wav: vec![wav], reuse, index: Some(2), trimmed_leading_silence_ms: 0.0 };
        for staged in [[mix(1, false), mix(2, false)], [mix(1, false), mix(1, false)]] {
            let error = publish(&staged, None, &|| false).err().expect("refused");
            assert!(error.contains("Sample index 2 would hold two different sounds"), "{error}");
            assert!(!path.exists(), "nothing is written");
        }
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
            trimmed_leading_silence_ms: 0.0,
        };
        let preview = staged.clip();
        assert_eq!(preview.samples, [0.125, 0.125]);
        assert_eq!(preview.channels, CHANNELS);
        assert_eq!(preview.sample_rate, SAMPLE_RATE);
    }

    /// Export with pair samples for one circle at 1000 ms holding `layers`, in a folder with `files`.
    fn pair_export(
        scratch: &dawsu_test_support::Scratch, files: &[(&str, Vec<u8>)], layers: &[(SampleIdentity, i32)], previous: &[String],
    ) -> (OsuFile, Vec<StagedMix>) {
        let map = scratch.path("Artist - Song (Mapper) [Hard].osu");
        fs::write(&map, b"osu file format v14\n[General]\nMode:0\n[Metadata]\nVersion:Hard\n[TimingPoints]\n0,500,4,1,0,100,1,0\n[HitObjects]\n256,192,1000,1,0,1:0:0:50:\n").unwrap();
        for (name, wav) in files {
            fs::write(scratch.path(name), wav).unwrap();
        }
        let mut document = crate::Document::read(&map).unwrap();
        document.composer.apply("Hits", |h| h.set_moment_state(MomentId(0), 50, layers.iter().cloned()));
        let hits = document.composer.hitsounds();
        let dawsu_hitsounding::generate::GenerateOutcome::Planned { file, problems, .. } = dawsu_hitsounding::generate::generate(
            hits, &document.file, "source.osu", &[], &|| false, &mut |_, _| {},
        ) else {
            panic!("Generate planned nothing");
        };
        let settings = ExportSettings { pair_samples: true, ..ExportSettings::default() };
        let (file, mixes) = stage_in_file(
            hits, file, &problems, map.parent().unwrap(), Path::new(""), &[], Placement::OnObjects, &[], Some(&settings),
            previous, 2, MixFormat::Wav16, &Pending::default(), &|| false,
        ).unwrap();
        (file, mixes.staged)
    }

    fn tiny_wav(frame: i16) -> Vec<u8> {
        let mut wav = b"RIFF\x26\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data\x02\0\0\0".to_vec();
        wav.extend(frame.to_le_bytes());
        wav
    }

    fn clap(set: SampleSet, index: u32) -> SampleIdentity {
        SampleIdentity::Standard { set, sound: Sound::Hit(Hitsound::Clap), index: CustomIndex::new(index) }
    }

    fn staged_names(staged: &[StagedMix]) -> Vec<String> {
        staged.iter().map(|mix| mix.path.file_name().unwrap().to_string_lossy().into_owned()).collect()
    }

    /// Yoge 2026-11-20: with no hitnormal in the moment, Export writes no hitnormal file and the
    /// player's skin plays its own, as Save does.
    #[test]
    #[allow(clippy::disallowed_methods)]
    fn pair_samples_export_of_a_clap_only_moment_writes_only_its_clap() {
        let scratch = dawsu_test_support::Scratch::new("pair-clap-only");
        let claps = [(clap(SampleSet::Soft, 3), 50), (clap(SampleSet::Drum, 4), 40)];
        let (file, staged) = pair_export(
            &scratch, &[("soft-hitclap3.wav", tiny_wav(4096)), ("drum-hitclap4.wav", tiny_wav(8192))], &claps, &[],
        );
        let names = staged_names(&staged);
        assert_eq!(names.len(), 1, "{names:?}");
        assert!(names[0].ends_with("-hitclap2.wav"), "{names:?}");
        assert_eq!(file.hit_objects()[0].hit_sample().unwrap().index, 2);
        let hitnormal_at_2 = dawsu_osu_format::list_folder(&scratch.path("")).unwrap().iter().any(|name| name.ends_with("-hitnormal2.wav"));
        assert!(!hitnormal_at_2);
        // The skin's hitnormal plays in the circle's own normal set, as Save keeps it.
        let objects = file.hit_objects();
        let sample = objects[0].hit_sample().unwrap();
        assert!(matches!(sample.normal_set, 0 | 1), "{sample:?}");
    }

    /// A reusable index that has any set's hitnormal file is skipped, or osu! would play that file.
    #[test]
    fn pair_samples_export_skips_an_index_with_a_hitnormal_file() {
        let claps = [(clap(SampleSet::Soft, 3), 50), (clap(SampleSet::Drum, 4), 40)];
        let inputs = [("soft-hitclap3.wav", tiny_wav(4096)), ("drum-hitclap4.wav", tiny_wav(8192))];
        let first = dawsu_test_support::Scratch::new("pair-clap-render");
        let (_, staged) = pair_export(&first, &inputs, &claps, &[]);
        let earlier = staged_names(&staged)[0].clone();
        let mut files = inputs.to_vec();
        files.push((earlier.as_str(), staged[0].wav.clone()));
        let previous = [earlier.clone()];

        let reused = dawsu_test_support::Scratch::new("pair-clap-reuse");
        let (file, staged) = pair_export(&reused, &files, &claps, &previous);
        assert_eq!(file.hit_objects()[0].hit_sample().unwrap().index, 2, "the earlier clap is reused");
        assert!(staged[0].reuse);

        files.push(("soft-hitnormal2.wav", tiny_wav(1024)));
        let blocked = dawsu_test_support::Scratch::new("pair-clap-hitnormal");
        let (file, staged) = pair_export(&blocked, &files, &claps, &previous);
        assert_eq!(file.hit_objects()[0].hit_sample().unwrap().index, 5);
        assert_eq!(staged_names(&staged).len(), 1);
        assert!(!staged[0].reuse);
    }

    #[test]
    fn wav32f_round_trips_through_dawsu_audio_preserving_rate_channels_and_waveform() {
        let scratch = dawsu_test_support::Scratch::new("mix-wav32f-roundtrip");
        let frames = 256;
        let mut samples = Vec::with_capacity(frames * CHANNELS);
        for i in 0..frames {
            let t = i as f32 / SAMPLE_RATE as f32;
            samples.push((2.0 * std::f32::consts::PI * 440.0 * t).sin() * 0.75);
            samples.push((2.0 * std::f32::consts::PI * 880.0 * t).cos() * 0.5);
        }
        let encoded = encode_format(&samples, MixFormat::Wav32f).unwrap();
        assert_eq!(&encoded[0..4], b"RIFF");
        assert_eq!(&encoded[8..12], b"WAVE");
        assert_eq!(u16::from_le_bytes([encoded[20], encoded[21]]), 3); // IEEE float
        assert_eq!(u16::from_le_bytes([encoded[34], encoded[35]]), 32); // 32 bits/sample

        let path = scratch.path("float-mix.wav");
        fs::write(&path, &encoded).unwrap();
        let decoded = dawsu_audio::decode(&path).unwrap();
        assert_eq!(decoded.sample_rate, SAMPLE_RATE);
        assert_eq!(decoded.channels, CHANNELS);
        assert_eq!(decoded.samples.len(), samples.len());
        for (a, b) in decoded.samples.iter().zip(&samples) {
            assert!((a - b).abs() < 1e-6, "sample mismatch: {a} vs {b}");
        }

        let staged = StagedMix { path, wav: encoded, reuse: false, index: None, trimmed_leading_silence_ms: 0.0 };
        let preview = staged.clip();
        assert_eq!(preview.sample_rate, SAMPLE_RATE);
        assert_eq!(preview.channels, CHANNELS);
        assert_eq!(preview.samples, samples);
    }

    #[test]
    fn ogg_vorbis_round_trips_through_dawsu_audio_and_is_byte_deterministic() {
        let scratch = dawsu_test_support::Scratch::new("mix-ogg-roundtrip");
        let frames = 4096;
        let mut samples = Vec::with_capacity(frames * CHANNELS);
        for i in 0..frames {
            let t = i as f32 / SAMPLE_RATE as f32;
            let env = (1.0 - i as f32 / frames as f32).max(0.0);
            samples.push((2.0 * std::f32::consts::PI * 440.0 * t).sin() * 0.6 * env);
            samples.push((2.0 * std::f32::consts::PI * 660.0 * t).sin() * 0.5 * env);
        }
        let first = encode_format(&samples, MixFormat::OggVorbis).unwrap();
        let second = encode_format(&samples, MixFormat::OggVorbis).unwrap();
        assert_eq!(&first[0..4], b"OggS");
        assert_eq!(first, second, "identical PCM must produce byte-identical Ogg Vorbis output");

        let path = scratch.path("vorbis-mix.ogg");
        fs::write(&path, &first).unwrap();
        let decoded = dawsu_audio::decode(&path).unwrap();
        assert_eq!(decoded.sample_rate, SAMPLE_RATE);
        assert_eq!(decoded.channels, CHANNELS);
        assert!(!decoded.samples.is_empty());
        let peak = decoded.samples.iter().fold(0.0_f32, |p, s| p.max(s.abs()));
        assert!(peak > 0.3, "decoded Ogg should preserve audible waveform, peak={peak}");

        let staged = StagedMix { path, wav: first, reuse: false, index: None, trimmed_leading_silence_ms: 0.0 };
        let preview = staged.clip();
        assert_eq!(preview.sample_rate, SAMPLE_RATE);
        assert_eq!(preview.channels, CHANNELS);
        assert_eq!(preview.samples.len(), decoded.samples.len());
    }

    #[test]
    fn export_index_schema_keeps_unchanged_mix_indices_stable_across_reexports() {
        let scratch = dawsu_test_support::Scratch::new("mix-index-schema-stability");
        let root = scratch.path("mapset");
        fs::create_dir(&root).unwrap();
        let empty = OsuFile::read(Vec::new()).file;
        let pending = Pending::default();

        let mix_a = vec![1u8, 2, 3, 4];
        let mix_b = vec![5u8, 6, 7, 8];
        let mix_new_earlier = vec![9u8, 10, 11, 12];

        // First export: mix_a and mix_b get custom indices 100 and 101.
        let mut first = IndexedMixes::read_with_schema(&root, &empty, &[], 100, &pending, BTreeMap::new()).unwrap();
        let (idx_a_1, reuse_a_1) = first.allocate(&root, &mix_a, MixFormat::Wav16).unwrap();
        let (idx_b_1, reuse_b_1) = first.allocate(&root, &mix_b, MixFormat::Wav16).unwrap();
        assert_eq!((idx_a_1, reuse_a_1), (100, false));
        assert_eq!((idx_b_1, reuse_b_1), (101, false));
        let schema_after_first = first.into_schema();
        assert_eq!(schema_after_first.get(&sha256_hex(&mix_a)), Some(&100));
        assert_eq!(schema_after_first.get(&sha256_hex(&mix_b)), Some(&101));

        // Second export in a clean folder (or after cleanup): a brand-new mix is inserted EARLIER
        // in the timeline before mix_a and mix_b. With export_index_schema, mix_new_earlier skips
        // 100 and 101 (taking 102) so mix_a stays 100 and mix_b stays 101.
        let mut second = IndexedMixes::read_with_schema(&root, &empty, &[], 100, &pending, schema_after_first).unwrap();
        let (idx_new, _) = second.allocate(&root, &mix_new_earlier, MixFormat::Wav16).unwrap();
        let (idx_a_2, _) = second.allocate(&root, &mix_a, MixFormat::Wav16).unwrap();
        let (idx_b_2, _) = second.allocate(&root, &mix_b, MixFormat::Wav16).unwrap();
        assert_eq!(idx_new, 102, "new mix inserted earlier must skip schema-claimed indices");
        assert_eq!(idx_a_2, 100, "unchanged mix_a keeps custom index 100");
        assert_eq!(idx_b_2, 101, "unchanged mix_b keeps custom index 101");

        // Also verify export-role allocation (prepare_export path) preserves custom indices across sessions.
        let waves_old = vec![(SampleSet::Soft, Hitsound::Clap, mix_a.clone())];
        let waves_new = vec![(SampleSet::Soft, Hitsound::Clap, mix_new_earlier.clone())];
        let mut exp1 = IndexedMixes::read_with_schema(&root, &empty, &[], 2, &pending, BTreeMap::new()).unwrap();
        let (old_idx_1, _) = exp1.allocate_export(&root, &pending, &waves_old, MixFormat::Wav16, &BTreeSet::new()).unwrap();
        assert_eq!(old_idx_1, 2);
        let exp_schema = exp1.into_schema();

        let mut exp2 = IndexedMixes::read_with_schema(&root, &empty, &[], 2, &pending, exp_schema).unwrap();
        let (new_idx_2, _) = exp2.allocate_export(&root, &pending, &waves_new, MixFormat::Wav16, &BTreeSet::new()).unwrap();
        let (old_idx_2, _) = exp2.allocate_export(&root, &pending, &waves_old, MixFormat::Wav16, &BTreeSet::new()).unwrap();
        assert_eq!(new_idx_2, 3);
        assert_eq!(old_idx_2, 2);
    }
}
