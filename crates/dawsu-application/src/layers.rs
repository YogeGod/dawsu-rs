use crate::{Document, lane_name, safe_child};
use dawsu_audio::{AudioClip, decode};
use dawsu_foundation::{CustomIndex, SampleIdentity};
use dawsu_hitsounding::rules::read_sample_file_name;
use dawsu_osu_format::{HitSample, ObjectKind, OsuFile, find_case_insensitive};
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;

pub(super) fn create(
    document: &Document,
    defaults: &Path,
    lane: &SampleIdentity,
) -> Result<(String, Vec<u8>, Arc<AudioClip>), String> {
    let SampleIdentity::Standard { set, sound, .. } = *lane else {
        return Err("Choose a sample family to add a numbered layer.".into());
    };
    let family = SampleIdentity::Standard {
        set,
        sound,
        index: CustomIndex::new(0),
    };
    let stem = lane_name(&family);
    let default_name = format!("{stem}.wav");
    let defaults_list = crate::pending::sample_folder(defaults);
    let default = find_case_insensitive(&defaults_list, &default_name)
        .ok_or_else(|| format!("No default WAV for {stem} in the configured sample folder."))?;
    let source = safe_child(defaults, default)?;
    let clip = decode(&source)?;
    let bytes =
        std::fs::read(&source).map_err(|error| format!("Reading {default_name}: {error}"))?;

    let name = create_backing(document, &family, "wav", &bytes)?;
    Ok((name, bytes, clip))
}

/// The name a new sample file for `lane` takes. Nothing is written: the caller keeps the bytes as
/// a pending file, which claims the name. Names on disk, names any difficulty or storyboard uses
/// and pending names are all taken.
pub(super) fn create_backing(
    document: &Document,
    lane: &SampleIdentity,
    extension: &str,
    bytes: &[u8],
) -> Result<String, String> {
    let root = document.path.parent().ok_or("The map folder is unavailable.")?;
    let files = crate::pending::listing(root, &document.pending)?;
    let (mut indexes, mut references) = reservations(root, &files)?;
    indexes.extend(document.pending.taken_indexes());
    references.extend(document.pending.taken_names());
    let name = match *lane {
        SampleIdentity::Standard { set, sound, .. } => next_numbered(document, &files, &indexes, set, sound, extension)?,
        SampleIdentity::File(_) => {
            let hash = dawsu_foundation::digest::sha256_hex(bytes);
            (1..=u32::MAX).map(|i| format!("dawsu-sample-{}-{i}.{extension}", &hash[..12]))
                .find(|name| !references.contains(name) && find_case_insensitive(&files, name).is_none())
                .ok_or("No safe sample name is available in this mapset.")?
        }
    };
    Ok(name)
}

fn next_numbered(
    document: &Document,
    files: &[String],
    reserved: &BTreeSet<u32>,
    set: dawsu_foundation::SampleSet,
    sound: dawsu_foundation::Sound,
    extension: &str,
) -> Result<String, String> {
    let stem = lane_name(&SampleIdentity::Standard { set, sound, index: CustomIndex::new(0) });
    let file_lanes = files.iter().filter_map(|name| named_sample(name));
    let lanes = document.composer.lanes.iter().cloned().chain(file_lanes);
    let mut family_highest = 1;
    let mut file_highest = 1;
    for lane in lanes {
        if let SampleIdentity::Standard {
            set: other_set,
            sound: other_sound,
            index,
        } = lane
        {
            file_highest = file_highest.max(index.get());
            if other_set == set && other_sound == sound {
                family_highest = family_highest.max(index.get());
            }
        }
    }

    let next_index = |current: u32| {
        current
            .checked_add(1)
            .filter(|index| *index <= i32::MAX as u32)
            .ok_or("No safe sample index is available in this mapset.")
    };
    let preferred = next_index(family_highest)?;
    let index = if reserved.contains(&preferred) {
        next_index(file_highest.max(reserved.last().copied().unwrap_or(1)))?
    } else {
        preferred
    };
    Ok(format!("{stem}{index}.{extension}"))
}

fn named_sample(name: &str) -> Option<SampleIdentity> {
    read_sample_file_name(name.rsplit(['/', '\\']).next()?)
}

#[derive(Default)]
struct Reservations {
    indexes: BTreeSet<u32>,
    names: BTreeSet<String>,
}

impl Reservations {
    fn remember_name(&mut self, name: &str) {
        let filename = name.rsplit(['/', '\\']).next().unwrap_or(name);
        self.names.insert(filename.to_ascii_lowercase());
        if let Some(SampleIdentity::Standard { index, .. }) = named_sample(filename) {
            self.indexes.insert(index.get());
        }
    }

    fn remember_sample(&mut self, sample: &HitSample) {
        if sample.index > 0 {
            self.indexes.insert(sample.index as u32);
        }
        if let Some(name) = &sample.filename {
            self.remember_name(name);
        }
    }
}

pub(super) fn reservations(root: &Path, files: &[String]) -> Result<(BTreeSet<u32>, BTreeSet<String>), String> {
    let mut reserved = Reservations::default();
    let mut difficulties = Vec::new();
    let mut storyboards = Vec::new();
    for name in files {
        let Some(ext) = Path::new(name).extension() else {
            continue;
        };
        let difficulty = ext.eq_ignore_ascii_case("osu");
        if !difficulty && !ext.eq_ignore_ascii_case("osb") {
            continue;
        }
        let path = safe_child(root, name)?;
        let bytes = std::fs::read(path)
            .map_err(|error| format!("Checking sample indexes in {name}: {error}"))?;
        let read = OsuFile::read(bytes);
        if let Some(warning) = read.warnings.iter().find(|warning| {
            matches!(
                warning.section.as_str(),
                "TimingPoints" | "HitObjects" | "Events"
            )
        }) {
            return Err(format!(
                "Cannot check sample indexes in {name} at line {}.",
                warning.line_number
            ));
        }
        let file = read.file;
        if difficulty {
            reserved.indexes.extend(
                file.timing_points()
                    .iter()
                    .filter_map(|point| point.sample_index())
                    .filter(|index| *index > 0)
                    .map(|index| index as u32),
            );
            for object in file.hit_objects() {
                if let Some(sample) = object.hit_sample() {
                    reserved.remember_sample(sample);
                }
                if let ObjectKind::Slider(slider) = object.kind()
                    && let Some(edges) = &slider.edge_sets
                {
                    for edge in edges.iter().flatten() {
                        reserved.remember_sample(edge);
                    }
                }
            }
        }
        for sample in file.storyboard_samples() {
            reserved.remember_name(&sample.path);
        }
        if difficulty {
            difficulties.push(file);
        } else {
            storyboards.push(file);
        }
    }
    for storyboard in storyboards {
        for file in &difficulties {
            for sample in storyboard.storyboard_samples_after(file) {
                reserved.remember_name(&sample.path);
            }
        }
    }
    Ok((reserved.indexes, reserved.names))
}
