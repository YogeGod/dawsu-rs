//! Decompile (hitsound domain Responsibilities 7 and 8): the rules' one answer grouped into lanes,
//! hits and moments. Blocking, with a cancel check and a progress report (ADR 0035).

use crate::model::{Hitsounds, Lane, LaneSource, Moment, ObjectId, Place, SliderBody};
use crate::rules::{self, Answer, Kind, PartKind, Plays, Sample};
use dawsu_foundation::{CustomIndex, Hitsound, SampleIdentity, SampleSet, SliderSound, Sound, SongTime};
use dawsu_osu_format::OsuFile;
use std::collections::{BTreeMap, BTreeSet};

/// The outcome of decompile.
#[derive(Debug, Clone, PartialEq)]
pub enum Decompiled {
    Done(Hitsounds),
    /// A taiko, catch or mania difficulty, as the rules answer it.
    UnsupportedMode { mode: i32 },
    /// The cancel check asked to stop; nothing is kept.
    Cancelled,
}

/// Decompiles `file`, given the names of the files in its folder (`dawsu_osu_format::list_folder`).
/// `cancel` is looked at between hit objects; `progress` gets (objects done, objects in total).
pub fn decompile(file: &OsuFile, folder_files: &[String], cancel: &dyn Fn() -> bool, progress: &mut dyn FnMut(usize, usize)) -> Decompiled {
    if cancel() {
        return Decompiled::Cancelled;
    }
    // The rules' answer takes no stop check of its own (an open item in todo.md section 3,
    // "Errors subsections"), so the checks are made while its objects are grouped.
    let objects = match rules::answer(file, folder_files) {
        Answer::Objects(objects) => objects,
        Answer::UnsupportedMode { mode } => return Decompiled::UnsupportedMode { mode },
    };

    let mut lanes: BTreeMap<SampleIdentity, LaneSource> = BTreeMap::new();
    for set in [SampleSet::Normal, SampleSet::Soft, SampleSet::Drum] {
        for sound in [Hitsound::Normal, Hitsound::Whistle, Hitsound::Finish, Hitsound::Clap] {
            lanes.insert(SampleIdentity::Standard { set, sound: Sound::Hit(sound), index: CustomIndex::new(0) }, LaneSource::Skin);
        }
    }
    // Sample files: every .wav, .mp3 and .ogg file, sub-folders included, except the song
    // (hitsound domain Responsibility 8).
    let song = file.general("AudioFilename").map(|s| s.to_lowercase());
    for name in folder_files {
        let lower = name.to_lowercase();
        let is_sample = [".wav", ".mp3", ".ogg"].iter().any(|e| lower.ends_with(e));
        if is_sample && Some(&lower) != song.as_ref() {
            let identity = rules::read_sample_file_name(name).unwrap_or_else(|| SampleIdentity::File(name.clone()));
            lanes.entry(identity).or_insert_with(|| LaneSource::File(name.clone()));
        }
    }

    let mut moments = Vec::new();
    let mut bodies = Vec::new();
    let total = objects.len();
    for (done, o) in objects.iter().enumerate() {
        if cancel() {
            return Decompiled::Cancelled;
        }
        let id = ObjectId(o.line);
        match o.kind {
            Kind::Circle => push_moment(&mut lanes, &mut moments, o.start_time, Place::Circle(id), &o.samples),
            Kind::Spinner => push_moment(&mut lanes, &mut moments, o.end_time.unwrap_or(o.start_time), Place::SpinnerEnd(id), &o.samples),
            Kind::Slider => {
                let Some(s) = &o.slider else { continue };
                let mut node = 0;
                let mut ticks = Vec::new();
                let mut tail = o.end_time.unwrap_or(o.start_time);
                for p in &s.parts {
                    match p.kind {
                        PartKind::Head => push_moment(&mut lanes, &mut moments, p.time, Place::SliderNode { object: id, node: 0 }, &p.samples),
                        PartKind::Repeat => {
                            node += 1;
                            push_moment(&mut lanes, &mut moments, p.time, Place::SliderNode { object: id, node }, &p.samples);
                        }
                        PartKind::Tick => ticks.push(SongTime::from_ms(p.time)),
                        PartKind::Tail => tail = p.time,
                    }
                }
                push_moment(&mut lanes, &mut moments, tail, Place::SliderNode { object: id, node: s.repeat_count + 1 }, &s.tail_samples);

                let slide = s.sliding_samples.iter().find(|x| x.lazer.name == "sliderslide");
                let whistle = s.sliding_samples.iter().any(|x| x.lazer.name == "sliderwhistle");
                for x in &s.sliding_samples {
                    lanes.insert(identity(x), source(&x.plays));
                }
                let tick_sample = s.parts.iter().find(|p| p.kind == PartKind::Tick).and_then(|p| p.samples.first());
                if let Some(t) = tick_sample {
                    lanes.insert(identity(t), source(&t.plays));
                }
                if let Some(slide) = slide {
                    let slide_id = identity(slide);
                    let tick = tick_sample.map(identity).unwrap_or_else(|| crate::model::with_sound(&slide_id, Sound::Slider(SliderSound::Tick)));
                    bodies.push(SliderBody {
                        object: id,
                        start: SongTime::from_ms(o.start_time),
                        end: SongTime::from_ms(o.end_time.unwrap_or(o.start_time)),
                        volume: slide.lazer.volume,
                        slide: slide_id,
                        whistle,
                        decompiled_whistle: whistle,
                        ticks,
                        tick,
                    });
                }
            }
        }
        progress(done + 1, total);
    }
    // Moments in time order, ties in object order (the rules' order).
    moments.sort_by(|a, b| a.time.ms().total_cmp(&b.time.ms()));
    let lanes = lanes.into_iter().map(|(identity, source)| Lane { identity, source }).collect();
    Decompiled::Done(Hitsounds { lanes, moments, bodies, folder_files: folder_files.to_vec() })
}

fn push_moment(lanes: &mut BTreeMap<SampleIdentity, LaneSource>, moments: &mut Vec<Moment>, time: f64, place: Place, samples: &[Sample]) {
    let hits: BTreeSet<SampleIdentity> = samples
        .iter()
        .map(|s| {
            let identity = identity(s);
            lanes.insert(identity.clone(), source(&s.plays));
            identity
        })
        .collect();
    // All samples at one node share the object's bank volume in lazer (invariant 2).
    let volume = samples.first().map_or(100, |s| s.lazer.volume);
    moments.push(Moment { time: SongTime::from_ms(time), place, volume, hits: hits.clone(), decompiled: Some((hits, volume)) });
}

fn source(plays: &Plays) -> LaneSource {
    match plays {
        Plays::BeatmapFile(name) => LaneSource::File(name.clone()),
        Plays::Skin { .. } => LaneSource::Skin,
    }
}

/// The lane a sample sounds on.
pub(crate) fn identity(s: &Sample) -> SampleIdentity {
    let l = &s.lazer;
    if let Some(f) = &l.filename {
        return SampleIdentity::File(f.clone());
    }
    let sound = match l.name.as_str() {
        "hitwhistle" => Sound::Hit(Hitsound::Whistle),
        "hitfinish" => Sound::Hit(Hitsound::Finish),
        "hitclap" => Sound::Hit(Hitsound::Clap),
        "sliderslide" => Sound::Slider(SliderSound::Slide),
        "sliderwhistle" => Sound::Slider(SliderSound::Whistle),
        "slidertick" => Sound::Slider(SliderSound::Tick),
        _ => Sound::Hit(Hitsound::Normal),
    };
    let set = match l.bank.as_str() {
        "soft" => SampleSet::Soft,
        "drum" => SampleSet::Drum,
        // A timing line's out-of-range set keeps its number as lazer's bank name (rules invariant
        // 9(b)). The model's three sets have no place for it, so its hits sit on the Normal lanes
        // (todo.md section 4, "odd sets in the model").
        _ => SampleSet::Normal,
    };
    SampleIdentity::Standard { set, sound, index: CustomIndex::new(l.custom_sample_bank().max(0) as u32) }
}
