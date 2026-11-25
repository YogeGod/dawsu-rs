use super::{Application, Document, safe_child, sample_watch::file_stamp};
use dawsu_foundation::SampleIdentity;
use dawsu_hitsounding::rules::read_sample_file_name;
use dawsu_osu_format::list_folder;
use std::{
    collections::{BTreeMap, HashSet},
    path::Path,
};

impl Application {
    pub fn lane_add_candidates(&self) -> Vec<SampleIdentity> {
        let Some(document) = self.document() else {
            return Vec::new();
        };
        let composer = &document.composer;
        let model = composer.hitsounds();
        composer
            .lanes
            .iter()
            .filter(|lane| {
                model.is_lane_hidden(lane) && !model.lane_slots().any(|(_, slot)| slot == *lane)
            })
            .cloned()
            .collect()
    }

    pub fn scan_lane_add_candidates(
        &self,
        expected_map: &Path,
    ) -> Result<Vec<SampleIdentity>, String> {
        let document = self.lane_edit_document(expected_map)?;
        let mut candidates = self.lane_add_candidates();
        candidates.extend(fresh_lane_files(document)?.into_keys());
        Ok(candidates)
    }

    fn lane_edit_document(&self, expected_map: &Path) -> Result<&Document, String> {
        if self.busy || self.is_stroke_open() {
            return Err("Finish the current operation before editing lanes".into());
        }
        self.document()
            .filter(|doc| doc.path == expected_map)
            .ok_or_else(|| "The difficulty changed while choosing a lane".into())
    }

    pub fn show_lane(&mut self, expected_map: &Path, lane: &SampleIdentity) -> Result<(), String> {
        let doc = self.lane_edit_document(expected_map)?;
        if doc.composer.hitsounds().lane(lane).is_none() {
            let (_, name) = fresh_lane_files(doc)?
                .remove(lane)
                .ok_or("This lane is no longer available")?;
            let root = doc.path.parent().ok_or("Missing mapset folder")?;
            let path = safe_child(root, &name)?;
            let before = file_stamp(&path).ok_or("This sample could not be read")?;
            let clip = dawsu_audio::decode(&path)?;
            if file_stamp(&path) != Some(before) {
                return Err("The sample changed while it was being read. Choose it again.".into());
            }
            self.register_added_samples(vec![(name, clip)]);
            // Keep the catalog entry and cached audio after Undo, but return it to the Add list.
            let doc = self.document.as_mut().expect("open map was just validated");
            doc.composer
                .history
                .update_external(|hits| hits.set_lane_hidden(lane, true));
        }
        let doc = self.document.as_mut().expect("open map was just validated");
        doc.composer.show_lane(lane);
        let hits = doc.composer.hitsounds();
        let slot = hits.lane_slot(lane).unwrap_or(lane).clone();
        self.restore_active_lane(Some(slot));
        Ok(())
    }

    /// Adds an external or mapset audio file as a visible lane without writing to the mapset
    /// folder until a hit on that lane is saved.
    ///
    /// - If `source` already lives inside the open mapset folder and matches an existing lane,
    ///   unhides that lane (1 Undo step, 0 duplicate files).
    /// - Otherwise decodes & validates the audio, stages its bytes under a collision-safe
    ///   non-standard filename in `document.pending`, registers the lane, and reveals it in
    ///   1 Undo step (`dirty = false`).
    pub fn add_lane_from_file(
        &mut self,
        expected_map: &Path,
        source: &Path,
    ) -> Result<SampleIdentity, String> {
        let doc = self.lane_edit_document(expected_map)?;
        let root = doc
            .path
            .parent()
            .ok_or("Missing mapset folder")?
            .canonicalize()
            .map_err(|e| format!("Opening mapset folder: {e}"))?;
        let canonical = source
            .canonicalize()
            .map_err(|e| format!("Opening {}: {e}", source.display()))?;
        let extension = canonical
            .extension()
            .and_then(|ext| ext.to_str())
            .map(str::to_ascii_lowercase)
            .filter(|ext| matches!(ext.as_str(), "wav" | "ogg" | "mp3"))
            .ok_or("Choose a WAV, OGG, or MP3 sample.")?;

        // 1. If the file is already inside the mapset folder, reuse its existing lane or register
        //    the on-disk file directly without duplicating bytes into pending.
        if canonical.starts_with(&root) {
            let hits = doc.composer.hitsounds();
            let existing = hits
                .lanes()
                .iter()
                .find(|lane| match &lane.source {
                    dawsu_hitsounding::LaneSource::File(name) => {
                        safe_child(&root, name).ok().as_ref() == Some(&canonical)
                    }
                    dawsu_hitsounding::LaneSource::Skin => false,
                })
                .map(|lane| lane.identity.clone());
            if let Some(identity) = existing {
                let doc = self.document.as_mut().expect("open map was just validated");
                doc.composer.show_lane(&identity);
                let slot = doc
                    .composer
                    .hitsounds()
                    .lane_slot(&identity)
                    .unwrap_or(&identity)
                    .clone();
                self.restore_active_lane(Some(slot));
                self.status = format!(
                    "Added {}",
                    canonical.file_name().unwrap_or_default().to_string_lossy()
                );
                return Ok(identity);
            }
            if let Ok(rel) = canonical.strip_prefix(&root) {
                let rel_name = rel.to_string_lossy().replace('\\', "/");
                let before = file_stamp(&canonical).ok_or("This sample could not be read")?;
                let clip = dawsu_audio::decode(&canonical)?;
                if file_stamp(&canonical) != Some(before) {
                    return Err("The sample changed while it was being read. Choose it again.".into());
                }
                let identity = read_sample_file_name(&rel_name)
                    .unwrap_or_else(|| SampleIdentity::File(rel_name.clone()));
                self.register_samples(vec![(rel_name, clip)], false);
                let doc = self.document.as_mut().expect("open map was just validated");
                doc.composer
                    .history
                    .update_external(|hits| hits.set_lane_hidden(&identity, true));
                doc.composer.show_lane(&identity);
                self.restore_active_lane(Some(identity.clone()));
                self.status = format!(
                    "Added {}",
                    canonical.file_name().unwrap_or_default().to_string_lossy()
                );
                return Ok(identity);
            }
        }

        // 2. External file: validate, decode, stage in pending under a safe non-standard name.
        let bytes = std::fs::read(&canonical).map_err(|e| format!("Reading sample: {e}"))?;
        let clip = dawsu_audio::decode(&canonical)?;
        if !std::fs::read(&canonical).is_ok_and(|current| current == bytes) {
            return Err("The sample changed while it was being read. Choose it again.".into());
        }
        let original_stem = canonical
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("sample")
            .trim()
            .to_string();
        let (name, used_hash_fallback) =
            super::layers::create_custom_file_name(doc, &original_stem, &extension, &bytes)?;
        let identity = SampleIdentity::File(name.clone());

        self.document
            .as_mut()
            .expect("open map was just validated")
            .pending
            .add(&name, &bytes)?;
        self.register_samples(vec![(name.clone(), clip)], false);

        let doc = self.document.as_mut().expect("open map was just validated");
        if used_hash_fallback && !original_stem.is_empty() {
            let mut layout = doc.lane_layout().cloned().unwrap_or_default();
            layout
                .lane_names
                .insert(super::project::lane_key(&identity), original_stem.clone());
            layout.normalise();
            doc.lane_layout = Some(layout);
        }
        doc.composer
            .history
            .update_external(|hits| hits.set_lane_hidden(&identity, true));
        doc.composer.show_lane(&identity);
        self.restore_active_lane(Some(identity.clone()));
        self.status = format!("Added {original_stem}.{extension}");
        Ok(identity)
    }

    /// Unhides all currently hidden Index-0 default osu! skin lanes in 1 Undo step without
    /// marking the document dirty. Pushes no Undo step when all 12 are already visible.
    pub fn show_default_skin_lanes(&mut self, expected_map: &Path) -> Result<usize, String> {
        let doc = self.lane_edit_document(expected_map)?;
        let skin_lanes: Vec<SampleIdentity> = doc
            .composer
            .hitsounds()
            .lanes()
            .iter()
            .filter(|lane| {
                matches!(
                    lane.identity,
                    SampleIdentity::Standard {
                        sound: dawsu_foundation::Sound::Hit(_),
                        index,
                        ..
                    } if index.get() == 0
                )
            })
            .map(|lane| lane.identity.clone())
            .collect();
        let doc = self.document.as_mut().expect("open map was just validated");
        let shown = doc
            .composer
            .show_lanes("Show 12 default lanes", &skin_lanes);
        if shown > 0 {
            self.status = format!("Shown {shown} default skin lanes.");
        }
        Ok(shown)
    }

    pub fn delete_lane(
        &mut self,
        expected_map: &Path,
        lane: &SampleIdentity,
        expected_revision: u64,
    ) -> Result<(), String> {
        let slot = self.active_lane_slot();
        let doc = self.lane_edit_document(expected_map)?;
        if doc.composer.history.version() != expected_revision {
            return Err("The hits changed before deletion. Review the lane again".into());
        }
        let hits = doc.composer.hitsounds();
        let name = lane_label(doc, hits.lane_slot(lane).unwrap_or(lane));
        let count = hits.moments().iter().filter(|moment| moment.has_hit(lane)).count();
        let links = super::sound_sets::live_placements(hits);
        let doc = self.document.as_mut().expect("open map was just validated");
        doc.composer.delete_lane(lane)?;
        let broken = links - super::sound_sets::live_placements(doc.composer.hitsounds());
        self.restore_active_lane(slot);
        let mut status = match count {
            0 => format!("Removed {name}."),
            1 => format!("Removed {name} and 1 hit."),
            _ => format!("Removed {name} and {count} hits."),
        };
        // A sound set placement whose added hits were all on removed lanes links nothing any more.
        match broken {
            0 => {}
            1 => status.push_str(" 1 sound set link no longer applies."),
            _ => status.push_str(&format!(" {broken} sound set links no longer apply.")),
        }
        if count > 0 || broken > 0 {
            status.push_str(" Undo restores them.");
        }
        self.status = status;
        Ok(())
    }

    /// Clones `lane`'s sample file to the next numbered variant (`name2.ext`, `name3.ext`, ...)
    /// in the mapset folder and adds it as a visible lane right after `lane`.
    pub fn clone_sample_variant(&mut self, lane: usize) -> Result<SampleIdentity, String> {
        if self.busy || self.is_stroke_open() {
            return Err("Finish the current operation before cloning a sample variant.".into());
        }
        let doc = self.document().ok_or("Open a difficulty first.")?;
        let expected_map = doc.path.clone();
        let source_identity = doc
            .composer
            .lanes
            .get(lane)
            .cloned()
            .ok_or("Choose a valid lane to clone.")?;
        if matches!(
            source_identity,
            SampleIdentity::Standard {
                sound: dawsu_foundation::Sound::Slider(_),
                ..
            }
        ) {
            return Err("Slider body samples cannot be cloned here.".into());
        }
        let hits = doc.composer.hitsounds();
        let slot = hits.lane_slot(&source_identity).unwrap_or(&source_identity);
        let source_name = match &hits
            .lane(slot)
            .or_else(|| hits.lane(&source_identity))
            .ok_or("This lane has no sample entry.")?
            .source
        {
            dawsu_hitsounding::LaneSource::File(name) => name.clone(),
            dawsu_hitsounding::LaneSource::Skin => {
                return Err("Only mapset sample files can be cloned as numbered variants.".into());
            }
        };
        let root = expected_map.parent().ok_or("Missing mapset folder")?;
        let source_path = if let Some(pending) = doc
            .pending
            .files()
            .find(|f| f.name.eq_ignore_ascii_case(&source_name))
        {
            pending.path.clone()
        } else {
            safe_child(root, &source_name)?
        };
        let bytes = std::fs::read(&source_path)
            .map_err(|e| format!("Reading {source_name}: {e}"))?;
        let clip = dawsu_audio::decode(&source_path)?;
        let ext = Path::new(&source_name)
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .filter(|e| matches!(e.as_str(), "wav" | "ogg" | "mp3"))
            .unwrap_or_else(|| "wav".to_string());
        let variant_name = super::layers::create_backing(doc, &source_identity, &ext, &bytes, false)?;
        let variant_identity = read_sample_file_name(&variant_name)
            .unwrap_or_else(|| SampleIdentity::File(variant_name.clone()));
        self.document
            .as_mut()
            .expect("open map was just validated")
            .pending
            .add(&variant_name, &bytes)?;
        self.register_samples(vec![(variant_name.clone(), clip)], false);
        let doc = self.document.as_mut().expect("open map was just validated");
        doc.composer
            .history
            .update_external(|hits| hits.set_lane_hidden(&variant_identity, true));
        doc.composer.show_lane(&variant_identity);
        if let Some(pos) = doc.composer.lanes.iter().position(|l| l == &variant_identity) {
            doc.composer.lanes.remove(pos);
        }
        let insert_at = (lane + 1).min(doc.composer.lanes.len());
        doc.composer.lanes.insert(insert_at, variant_identity.clone());
        self.active_lane = insert_at;
        self.status = format!("Cloned {source_name} as {variant_name}.");
        Ok(variant_identity)
    }

    /// Moves every second selected hit on `from_lane` (chronologically) onto `to_lane`
    /// as a single undoable edit.
    pub fn alternate_variants(&mut self, from_lane: usize, to_lane: usize) -> Result<usize, String> {
        if self.busy || self.is_stroke_open() {
            return Err("Finish the current operation before alternating variants.".into());
        }
        if from_lane == to_lane {
            return Err("Choose two different lanes to alternate.".into());
        }
        let doc = self.document.as_mut().ok_or("Open a difficulty first.")?;
        let from_id = doc
            .composer
            .lanes
            .get(from_lane)
            .cloned()
            .ok_or("Source lane is unavailable.")?;
        let to_id = doc
            .composer
            .lanes
            .get(to_lane)
            .cloned()
            .ok_or("Target variant lane is unavailable.")?;
        let moved = doc.composer.alternate_lane_hits(&from_id, &to_id);
        if moved == 0 {
            return Err("Select at least two hits on the source lane to alternate.".into());
        }
        let from_name = lane_label(doc, &from_id);
        let to_name = lane_label(doc, &to_id);
        self.status = format!("Alternated {moved} hits from {from_name} to {to_name}.");
        Ok(moved)
    }
}

/// Computes the next free numbered variant filename (e.g. `soft-hitwhistle.wav` ->
/// `soft-hitwhistle2.wav`, `soft-hitwhistle2.wav` -> `soft-hitwhistle3.wav`), checking
/// `existing_names` case-insensitively.
#[cfg(test)]
pub fn next_numbered_variant_name(existing_names: &[String], source_name: &str) -> String {
    let (stem, ext) = match source_name.rfind('.') {
        Some(dot) if dot > 0 => (&source_name[..dot], Some(&source_name[dot..])),
        _ => (source_name, None),
    };
    let digit_start = stem
        .char_indices()
        .rev()
        .take_while(|(_, ch)| ch.is_ascii_digit())
        .last()
        .map(|(idx, _)| idx)
        .unwrap_or(stem.len());
    let (base, mut next_num) = if digit_start < stem.len() && digit_start > 0 {
        let parsed = stem[digit_start..].parse::<u32>().unwrap_or(1);
        (&stem[..digit_start], parsed.saturating_add(1).max(2))
    } else {
        (stem, 2)
    };
    loop {
        let candidate = match ext {
            Some(ext) => format!("{base}{next_num}{ext}"),
            None => format!("{base}{next_num}"),
        };
        if !existing_names
            .iter()
            .any(|name| name.eq_ignore_ascii_case(&candidate))
        {
            return candidate;
        }
        next_num = next_num.saturating_add(1);
    }
}

/// A lane's name as its header shows it: the mapper's own name, else the sound and its number
/// ("Finish 3"), else a map file's name. app.js's `laneSound` names lanes the same way.
fn lane_label(document: &Document, lane: &SampleIdentity) -> String {
    use dawsu_foundation::{Hitsound, SliderSound, Sound};
    if let Some(name) = document.lane_layout().and_then(|layout| layout.lane_names.get(&super::project::lane_key(lane))) {
        return name.clone();
    }
    match lane {
        SampleIdentity::File(name) => name.clone(),
        SampleIdentity::Standard { sound, index, .. } => {
            let sound = match sound {
                Sound::Hit(Hitsound::Normal) => "Normal",
                Sound::Hit(Hitsound::Whistle) => "Whistle",
                Sound::Hit(Hitsound::Finish) => "Finish",
                Sound::Hit(Hitsound::Clap) => "Clap",
                Sound::Slider(SliderSound::Slide) => "Slide",
                Sound::Slider(SliderSound::Whistle) => "Slide whistle",
                Sound::Slider(SliderSound::Tick) => "Tick",
            };
            if index.get() > 1 { format!("{sound} {}", index.get()) } else { sound.to_owned() }
        }
    }
}

/// Files on disk not yet lanes, so it reads the folder alone.
#[allow(clippy::disallowed_methods)]
fn fresh_lane_files(
    document: &Document,
) -> Result<BTreeMap<SampleIdentity, (usize, String)>, String> {
    let root = document.path.parent().ok_or("Missing mapset folder")?;
    let known: HashSet<_> = document.files.iter().map(String::as_str).collect();
    let song = document
        .file
        .general("AudioFilename")
        .map(|name| name.replace('\\', "/"));
    let mut candidates = BTreeMap::new();
    for name in list_folder(root).map_err(|error| error.to_string())? {
        if known.contains(name.as_str())
            || song
                .as_ref()
                .is_some_and(|song| song.eq_ignore_ascii_case(&name))
        {
            continue;
        }
        let Some(extension) = Path::new(&name).extension() else {
            continue;
        };
        let Some(rank) = ["wav", "mp3", "ogg"]
            .into_iter()
            .position(|kind| extension.eq_ignore_ascii_case(kind))
        else {
            continue;
        };
        let lane =
            read_sample_file_name(&name).unwrap_or_else(|| SampleIdentity::File(name.clone()));
        if document.composer.hitsounds().lane(&lane).is_some() || safe_child(root, &name).is_err() {
            continue;
        }
        let choice = candidates.entry(lane).or_insert((rank, name.clone()));
        if rank < choice.0 {
            *choice = (rank, name);
        }
    }
    Ok(candidates)
}

#[cfg(test)]
mod tests {
    use super::next_numbered_variant_name;

    #[test]
    fn numbered_variant_increments_and_avoids_case_insensitive_collisions() {
        let existing = vec![
            "soft-hitwhistle.wav".to_owned(),
            "SOFT-HITWHISTLE2.WAV".to_owned(),
        ];
        assert_eq!(
            next_numbered_variant_name(&existing, "soft-hitwhistle.wav"),
            "soft-hitwhistle3.wav"
        );
        assert_eq!(
            next_numbered_variant_name(&existing, "soft-hitwhistle2.wav"),
            "soft-hitwhistle3.wav"
        );
        assert_eq!(
            next_numbered_variant_name(&[], "drum-snare.wav"),
            "drum-snare2.wav"
        );
    }
}
