//! Render the compiler's one-shot mix plans and stage their files until Apply or Save.

use crate::{ExportSettings, export_settings, lane_name, sample_path};
use dawsu_audio::{AudioClip, Mixer, ScheduledSound, decode_with_cancel};
use dawsu_foundation::{
    CustomIndex, Hitsound, SampleIdentity, SampleSet, Sound, digest::sha256_hex, safe_write,
};
use dawsu_hitsounding::{
    Compiled, Decompiled, Hitsounds, LaneSource, MomentId, ObjectId, Place, Placement, ProblemKind,
    SaveProblem, compile, decompile,
};
use dawsu_osu_format::{OsuFile, list_folder};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const SAMPLE_RATE: u32 = 48_000;
const CHANNELS: usize = 2;
const PREFIX: &str = "dawsu-mix-";
static NEXT: AtomicU64 = AtomicU64::new(0);

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

pub(super) struct StagedMix {
    pub(super) path: PathBuf,
    wav: Vec<u8>,
    reuse: bool,
    index: Option<u32>,
}

impl StagedMix {
    pub(super) fn new_generated<'a>(
        root: &Path,
        wav: Vec<u8>,
        mut existing: impl Iterator<Item = &'a String>,
    ) -> Result<(String, Self), String> {
        let stem = format!("{PREFIX}{}-", sha256_hex(&wav));
        let reusable =
            existing.find(|name| name.starts_with(&stem) && has_contents(root, name, &wav));
        let name = match reusable {
            Some(name) => name.clone(),
            None => new_name(&stem)?,
        };
        let mix = Self {
            path: root.join(&name),
            wav,
            reuse: reusable.is_some(),
            index: None,
        };
        Ok((name, mix))
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
    cancel: &dyn Fn() -> bool,
) -> Result<(OsuFile, MixFiles), String> {
    let listing = list_folder(root).map_err(|error| error.to_string())?;
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
            cancel,
        )?;
    }
    match compile(&file, &hits, placement, cancel, &mut |_, _| {}) {
        Compiled::Done { file, problems } if problems.is_empty() => Ok((file, files)),
        Compiled::Done { problems, .. } => Err(format!(
            "Could not encode sample mix at {:.0} ms: {:?}",
            problems[0].time.ms(),
            problems[0].kind,
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
        let mut indexes = IndexedMixes::read(root, &OsuFile::read(Vec::new()).file, &[])?;
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
                    let path = sample_path(lane, root, defaults, &defaults_list)
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
                let mut waves = Vec::with_capacity(components.len());
                for (role, mut cues) in components {
                    let bank = if role == Hitsound::Normal {
                        normal_bank
                    } else {
                        bank
                    };
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
                let reused = reusable_export_index(root, &waves, &previous_mixes);
                let index = match reused {
                    Some(index) => index,
                    None => indexes.reserve()?,
                };
                let mut outputs = Vec::with_capacity(waves.len());
                for (bank, role, wav) in waves {
                    let name = format!("{}{index}.wav", export_settings::slot_name(bank, role));
                    self.staged.push(StagedMix {
                        path: root.join(name),
                        wav,
                        reuse: reused.is_some(),
                        index: Some(index),
                    });
                    outputs.push(SampleIdentity::Standard {
                        set: bank,
                        sound: Sound::Hit(role),
                        index: CustomIndex::new(index),
                    });
                }
                &*rendered.entry(key).or_insert(outputs)
            };
            replace_moment(target, moment, outputs.iter().cloned());
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
            Some(IndexedMixes::read(root, &expected, already_staged)?)
        } else {
            None
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
                let mut cues = Vec::with_capacity(layers.len());
                for (identity, volume) in layers {
                    if !clips.contains_key(identity) {
                        let lane = source
                            .lane(identity)
                            .or_else(|| target.lane(identity))
                            .ok_or("Sample mix lane is missing")?;
                        let path =
                            sample_path(lane, root, defaults, &defaults_list).map_err(|error| {
                                format!("Could not mix {}: {error}", lane_name(identity))
                            })?;
                        let clip = decode_with_cancel(&path, cancel).map_err(|error| {
                            format!("Could not mix {}: {error}", lane_name(identity))
                        })?;
                        clips.insert(identity.clone(), clip);
                    }
                    cues.push(ScheduledSound {
                        at_ms: 0.0,
                        end_ms: None,
                        clip: clips[identity].clone(),
                        gain: *volume as f32 / 100.0,
                    });
                }
                let wav = render(cues, cancel)?;
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
                    let (name, staged) = StagedMix::new_generated(root, wav, existing.iter())?;
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

fn occupied_indexes(root: &Path) -> Result<BTreeSet<u32>, String> {
    use dawsu_hitsounding::rules::read_sample_file_name;
    let files = list_folder(root).map_err(|error| error.to_string())?;
    let (mut reserved, _) = crate::layers::reservations(root, &files)?;
    reserved.extend(files.iter().filter_map(|name| {
        match read_sample_file_name(name.rsplit(['/', '\\']).next()?)? {
            SampleIdentity::Standard { index, .. } => Some(index.get()),
            SampleIdentity::File(_) => None,
        }
    }));
    Ok(reserved)
}

impl IndexedMixes {
    fn read(root: &Path, expected: &OsuFile, already_staged: &[StagedMix]) -> Result<Self, String> {
        let mut reserved = occupied_indexes(root)?;
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
            next: 2,
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
}

pub(super) fn publish<'a>(
    staged: &'a [StagedMix],
    cancel: &dyn Fn() -> bool,
) -> Result<PublishedMixes<'a>, String> {
    let mut indexed = staged
        .iter()
        .filter(|mix| !mix.reuse && mix.index.is_some())
        .peekable();
    if let Some(first) = indexed.peek() {
        let root = first.path.parent().ok_or("Missing mapset folder")?;
        let reserved = occupied_indexes(root)?;
        if let Some(index) = indexed
            .filter_map(|mix| mix.index)
            .find(|index| reserved.contains(index))
        {
            return Err(format!(
                "Sample index {index} is now used in this mapset. Review again before applying."
            ));
        }
    }
    let mut publication = PublishedMixes {
        staged,
        created: Vec::new(),
    };
    for (index, mix) in staged.iter().enumerate() {
        if cancel() {
            return Err("Sample mix publishing cancelled".into());
        }
        if mix.reuse {
            if !fs::read(&mix.path).is_ok_and(|bytes| bytes == mix.wav) {
                return Err(format!(
                    "{} changed since sample review",
                    mix.path.display()
                ));
            }
        } else {
            safe_write::create(&mix.path, &mix.wav).map_err(|error| error.to_string())?;
            publication.created.push(index);
        }
    }
    Ok(publication)
}

impl PublishedMixes<'_> {
    pub(super) fn keep(mut self) {
        self.created.clear();
    }
}

impl Drop for PublishedMixes<'_> {
    fn drop(&mut self) {
        for index in &self.created {
            let mix = &self.staged[*index];
            if fs::read(&mix.path).is_ok_and(|bytes| bytes == mix.wav) {
                let _ = fs::remove_file(&mix.path);
            }
        }
    }
}

fn new_name(stem: &str) -> Result<String, String> {
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    Ok(format!(
        "{stem}{time:x}-{:x}-{id:x}.wav",
        std::process::id()
    ))
}

pub(super) fn render(
    cues: Vec<ScheduledSound>,
    cancel: &dyn Fn() -> bool,
) -> Result<Vec<u8>, String> {
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
    let data_size = frames
        .checked_mul(CHANNELS * 2)
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
    wav.extend(1u16.to_le_bytes()); // PCM
    wav.extend((CHANNELS as u16).to_le_bytes());
    wav.extend(SAMPLE_RATE.to_le_bytes());
    wav.extend((SAMPLE_RATE * CHANNELS as u32 * 2).to_le_bytes());
    wav.extend((CHANNELS as u16 * 2).to_le_bytes());
    wav.extend(16u16.to_le_bytes());
    wav.extend(b"data");
    wav.extend(data_size.to_le_bytes());

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
        for value in samples {
            wav.extend(((*value * 32768.0).round() as i16).to_le_bytes());
        }
    }
    Ok(wav)
}

#[cfg(test)]
mod tests {
    use super::*;

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
