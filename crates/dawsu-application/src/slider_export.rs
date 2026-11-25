//! Optional timed slider audio for Generate. Existing hitsound circles keep their own samples.

use crate::{
    mixes::{self, StagedMix},
    output::Source,
    sample_path,
};
use dawsu_audio::{AudioClip, ScheduledSound, decode_with_cancel};
use dawsu_foundation::SampleIdentity;
use dawsu_hitsounding::{
    Compiled, Decompiled, MomentId, Place, Placement, compile, decompile,
    lazer::control_points::offset_for, rules::played_volume,
};
use dawsu_osu_format::OsuFile;
use std::{
    collections::{BTreeMap, HashMap},
    path::Path,
    sync::Arc,
};

// The optional integer holds the exact bit representation of a loop duration in milliseconds.
type Layer = (SampleIdentity, i32, Option<u64>);

pub(super) struct StageInput<'a> {
    pub source: &'a Source,
    pub root: &'a Path,
    pub defaults: &'a Path,
    pub previous: &'a [u8],
    pub already_staged: &'a [StagedMix],
    pub format: mixes::MixFormat,
}

pub(super) fn stage(
    input: &StageInput<'_>,
    file: &mut OsuFile,
    cancel: &dyn Fn() -> bool,
) -> Result<Vec<StagedMix>, String> {
    let StageInput { source, root, defaults, previous, already_staged, format } = *input;
    let offset = offset_for(file.format_version());
    let mut groups = BTreeMap::<i64, Vec<Layer>>::new();
    for body in source.hits.bodies() {
        if cancel() {
            return Err("Slider audio preparation cancelled".into());
        }
        let file_time = |ms: f64| (ms - offset).round_ties_even() as i64;
        let volume = played_volume(body.volume());
        let duration = body.end().ms() - body.start().ms();
        if duration > 0.0 {
            let layers = groups.entry(file_time(body.start().ms())).or_default();
            layers.push((body.slide().clone(), volume, Some(duration.to_bits())));
            if body.whistle() {
                layers.push((body.whistle_lane(), volume, Some(duration.to_bits())));
            }
        }
        for tick in body.ticks() {
            groups.entry(file_time(tick.ms())).or_default().push((
                body.tick().clone(),
                volume,
                None,
            ));
        }
    }
    if groups.is_empty() {
        return Ok(Vec::new());
    }

    let previous_file = OsuFile::read(previous.to_vec()).file;
    let existing: Vec<_> = previous_file
        .hit_objects()
        .iter()
        .filter_map(|obj| obj.hit_sample()?.filename.clone())
        .collect();
    let pending = &source.pending;
    let listing = crate::pending::listing(root, pending)?;
    let defaults_list = crate::pending::sample_folder(defaults);
    let mut clips = HashMap::<SampleIdentity, Arc<AudioClip>>::new();
    let mut rendered = BTreeMap::<Vec<Layer>, String>::new();
    let mut staged = Vec::new();
    let mut names = mixes::reserved_mix_names(root, already_staged, pending)?;

    // Keep existing object lines verbatim. On a tie write the separate passive circle first.
    let mut objects: Vec<_> = file
        .hit_objects()
        .iter()
        .map(|obj| {
            let line = file
                .line_text(obj.line())
                .ok_or("Generated circle text is unreadable")?;
            Ok((obj.time() as i64, None, line))
        })
        .collect::<Result<_, String>>()?;
    for (time, mut layers) in groups {
        if cancel() {
            return Err("Slider audio preparation cancelled".into());
        }
        layers.sort_unstable();
        let name = if let Some(name) = rendered.get(&layers) {
            name.clone()
        } else {
            let mut cues = Vec::with_capacity(layers.len());
            for (sample, volume, duration) in &layers {
                let lane = source
                    .hits
                    .lane(sample)
                    .ok_or("Slider audio lane is no longer available")?;
                let source = lane.plays_as();
                let clip = match clips.entry(source) {
                    std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
                    std::collections::hash_map::Entry::Vacant(entry) => {
                        let path = sample_path(lane, root, pending, defaults, &defaults_list)?;

                        let clip = decode_with_cancel(&path, cancel).map_err(|error| {
                            format!("Opening slider audio {}: {error}", path.display())
                        })?;
                        entry.insert(clip)
                    }
                };
                cues.push(ScheduledSound {
                    at_ms: 0.0,
                    end_ms: duration.map(f64::from_bits),
                    gain: *volume as f32 / 100.0,
                    clip: clip.clone(),
                });
            }
            let bytes = mixes::render_format(cues, format, cancel)?;
            let (name, mix) = StagedMix::new_generated(root, bytes, format, existing.iter(), &mut names)?;
            staged.push(mix);
            rendered.insert(layers, name.clone());
            name
        };
        objects.push((time, Some(name), format!("256,192,{time},1,0,0:0:0:0:")));
    }
    objects.sort_by_key(|(time, sample, _)| (*time, sample.is_none()));
    let mut trial = file.clone();
    trial.remove_all_hit_objects();
    trial.append_hit_objects(
        &objects
            .iter()
            .map(|(_, _, line)| line.clone())
            .collect::<Vec<_>>(),
    );
    let mut hits = match decompile(&trial, &listing, cancel, &mut |_, _| {}) {
        Decompiled::Done(hits) => hits,
        Decompiled::Cancelled => return Err("Slider audio preparation cancelled".into()),
        Decompiled::UnsupportedMode { .. } => {
            return Err("Cannot export slider audio for this mode".into());
        }
    };
    if hits.moments().len() != objects.len() {
        return Err("Generated circle order changed while preparing slider audio".into());
    }
    for (index, (time, name, _)) in objects.into_iter().enumerate() {
        let Some(name) = name else { continue };
        let id = MomentId(index);
        let moment = hits.moment(id);
        if !matches!(moment.place(), Place::Circle(_)) || moment.time().ms() != time as f64 + offset
        {
            return Err("Generated circle time changed while preparing slider audio".into());
        }
        for hit in moment.hits().cloned().collect::<Vec<_>>() {
            hits.remove_hit(id, &hit);
        }
        hits.add_hit(id, SampleIdentity::File(name));
        // The mixer has already applied the loop and tick gains.
        hits.set_volume(id, 100);
    }
    match compile(&trial, &hits, Placement::OnObjects, cancel, &mut |_, _| {}) {
        Compiled::Done {
            file: compiled,
            problems,
        } if problems.is_empty() => {
            *file = compiled;
            Ok(staged)
        }
        Compiled::Done { problems, .. } => Err(format!(
            "Slider audio could not be placed on {} circles",
            problems.len()
        )),
        Compiled::Cancelled => Err("Slider audio preparation cancelled".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Document, output};
    use dawsu_test_support::Scratch;
    use std::fs;

    #[test]
    fn opt_in_stages_finite_slide_whistle_and_tick_audio_beside_native_hits() {
        let scratch = Scratch::new("generate-slider-audio");
        let root = scratch.path(".");
        let path = root.join("Example [Hard].osu");
        let original = concat!(
            "osu file format v14\r\n\r\n[General]\r\nMode:0\r\n\r\n",
            "[Metadata]\r\nVersion:Hard\r\nBeatmapID:7\r\nSource:fixture\r\n\r\n",
            "[Difficulty]\r\nSliderMultiplier:1.4\r\nSliderTickRate:1\r\n\r\n",
            "[TimingPoints]\r\n0,500,4,2,1,100,1,0\r\n\r\n[HitObjects]\r\n",
            "100,100,1000,2,2,L|300:100,1,200,0|0,0:0|0:0,0:0:0:0:\r\n",
        );
        fs::write(&path, original).unwrap();
        let wav = b"RIFF\x26\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data\x02\0\0\0";
        for (name, amplitude) in [
            ("soft-sliderslide.wav", 4096i16),
            ("soft-sliderwhistle.wav", 8192),
            ("soft-slidertick.wav", 16384),
        ] {
            let mut bytes = wav.to_vec();
            bytes.extend(amplitude.to_le_bytes());
            fs::write(root.join(name), bytes).unwrap();
        }

        let source = Source::from_document(&Document::read(&path).unwrap());
        let plan = output::generate(&source, &|| false).unwrap();
        assert_eq!(plan.mix_paths().count(), 0);
        let plain = &plan.files[0].file;
        let native = plain
            .hit_objects()
            .iter()
            .map(|object| plain.line_text(object.line()).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(native.len(), 2);
        let mut file = plain.clone();
        let defaults = scratch.path("defaults");
        let assets = stage(
            &StageInput {
                source: &source,
                root: &root,
                defaults: &defaults,
                previous: &[],
                already_staged: &[],
                format: mixes::MixFormat::Wav16,
            },
            &mut file,
            &|| false,
        )
        .unwrap();
        assert_eq!(assets.len(), 2);
        assert!(assets.iter().all(|asset| !asset.path.exists()));
        let body = &source.hits.bodies()[0];
        assert_eq!(body.ticks().len(), 1);
        let loop_clip = assets[0].clip();
        let frames = ((body.end().ms() - body.start().ms()) * 48.0).ceil() as usize;
        assert_eq!(loop_clip.samples.len(), frames * 2);
        assert!(loop_clip.samples.iter().all(|&sample| sample == 0.375));
        assert_eq!(assets[1].clip().samples, [0.5, 0.5]);

        let reopened = OsuFile::read(file.to_bytes()).file;
        let objects = reopened.hit_objects();
        assert_eq!(objects.len(), 4);
        assert_eq!(objects[0].time(), objects[1].time());
        assert_eq!(
            objects[0].hit_sample().unwrap().filename.as_deref(),
            assets[0].path.file_name().unwrap().to_str()
        );
        assert_eq!(
            objects[2].hit_sample().unwrap().filename.as_deref(),
            assets[1].path.file_name().unwrap().to_str()
        );
        assert_eq!(reopened.line_text(objects[1].line()).unwrap(), native[0]);
        assert_eq!(reopened.line_text(objects[3].line()).unwrap(), native[1]);

        let unchanged_sections = |bytes: Vec<u8>| {
            String::from_utf8(bytes)
                .unwrap()
                .split("[HitObjects]")
                .next()
                .unwrap()
                .to_owned()
        };
        assert_eq!(
            unchanged_sections(reopened.to_bytes()),
            unchanged_sections(plain.to_bytes())
        );
        assert_eq!(fs::read(&path).unwrap(), original.as_bytes());
        mixes::publish(&assets, None, &|| false).unwrap().keep();
        let mut repeat = plain.clone();
        let prev_bytes = file.to_bytes();
        let reused = stage(
            &StageInput {
                source: &source,
                root: &root,
                defaults: &defaults,
                previous: &prev_bytes,
                already_staged: &[],
                format: mixes::MixFormat::Wav16,
            },
            &mut repeat,
            &|| false,
        )
        .unwrap();
        assert_eq!(repeat.to_bytes(), file.to_bytes());
        assert_eq!(reused[0].path, assets[0].path);
        mixes::publish(&reused, None, &|| false).unwrap().keep();
    }
}
