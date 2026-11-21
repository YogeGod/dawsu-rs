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
        let name = super::lane_name(hits.lane_slot(lane).unwrap_or(lane));
        let count = hits.moments().iter().filter(|moment| moment.has_hit(lane)).count();
        let doc = self.document.as_mut().expect("open map was just validated");
        doc.composer.delete_lane(lane)?;
        self.restore_active_lane(slot);
        self.status = match count {
            0 => format!("Removed {name}."),
            1 => format!("Removed {name} and 1 hit."),
            _ => format!("Removed {name} and {count} hits."),
        };
        Ok(())
    }
}

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
