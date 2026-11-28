//! Restores editable gains after a Save that encoded them in native fields or a mix.
use super::{Document, sample_path};
use crate::{mixes::StagedMix, pending::{Pending, listing}, project::{lane_identity, lane_key}};
use dawsu_foundation::{SampleIdentity, digest::sha256_hex};
use dawsu_hitsounding::{Decompiled, Hitsounds, Moment, MomentId, Place, decompile};
use dawsu_osu_format::{LineId, OsuFile};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, path::Path};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct Revision {
    map_digest: String,
    moments: Vec<AuthoredMoment>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct AuthoredMoment {
    object: usize,
    node: Option<usize>,
    time_bits: u64,
    default_volume: i32,
    sources: Vec<Voice>,
    output: Vec<Voice>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Voice {
    lane: String,
    volume: i32,
    digest: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(super) struct Recovery {
    current: Option<Revision>,
    pending: Option<Revision>,
    before: Option<String>,
}

impl Recovery {
    fn for_digest(&self, digest: &str) -> Option<&Revision> {
        self.pending.as_ref()
            .filter(|revision| revision.map_digest == digest && self.before.as_deref() != Some(digest))
            .or_else(|| self.current.as_ref().filter(|revision| revision.map_digest == digest))
    }

    /// True when `revision` is already the record for `bytes`, so writing it again changes nothing.
    pub(super) fn is_current(&self, bytes: &[u8], revision: &Revision) -> bool {
        self.for_digest(&sha256_hex(bytes)) == Some(revision)
    }

    pub(super) fn has_authored_for(&self, bytes: &[u8]) -> bool {
        self.for_digest(&sha256_hex(bytes)).is_some_and(|revision| !revision.moments.is_empty())
    }

    pub(super) fn stage(&mut self, before: &[u8], after: Revision) {
        let digest = sha256_hex(before);
        if let Some(current) = self.for_digest(&digest) {
            self.current = Some(current.clone());
        }
        self.before = Some(digest);
        self.pending = Some(after);
    }

    pub(super) fn promote(&mut self, written: &[u8]) {
        let digest = sha256_hex(written);
        if self.pending.as_ref().is_some_and(|pending| pending.map_digest == digest) {
            self.current = self.pending.take();
            self.before = None;
        }
    }
}

fn location(place: Place) -> Option<(LineId, Option<usize>)> {
    match place {
        Place::Circle(id) | Place::SpinnerEnd(id) => Some((id.line(), None)),
        Place::SliderNode { object, node } => Some((object.line(), Some(node))),
        Place::NoObject => None,
    }
}

struct Assets<'a> {
    root: &'a Path,
    pending: &'a Pending,
    defaults: &'a Path,
    default_files: Vec<String>,
    staged: &'a [StagedMix],
    digests: BTreeMap<SampleIdentity, String>,
}

impl<'a> Assets<'a> {
    fn new(root: &'a Path, pending: &'a Pending, defaults: &'a Path, staged: &'a [StagedMix]) -> Self {
        Self { root, pending, defaults, default_files: crate::pending::sample_folder(defaults), staged,
            digests: BTreeMap::new() }
    }

    fn voices(&mut self, hits: &Hitsounds, moment: &Moment) -> Result<Vec<Voice>, String> {
        moment.hits().map(|sample| {
            let digest = if let Some(digest) = self.digests.get(sample) {
                digest.clone()
            } else {
                let lane = hits.lane(sample).ok_or("Missing sample lane in gain recovery")?;
                let digest = if let dawsu_hitsounding::LaneSource::File(name) = &lane.source
                    && let Some(staged) = self.staged.iter().find(|mix| mix.path.file_name().is_some_and(|base| base == name.as_str()))
                {
                    staged.digest()
                } else {
                    let path = sample_path(lane, self.root, self.pending, self.defaults, &self.default_files)?;
                    sha256_hex(&fs::read(&path).map_err(|error| format!("Reading {}: {error}", path.display()))?)
                };
                self.digests.insert(sample.clone(), digest.clone());
                digest
            };
            Ok(Voice { lane: lane_key(sample), volume: moment.hit_volume(sample), digest })
        }).collect()
    }
}

pub(super) type Locations = std::collections::BTreeSet<(LineId, Option<usize>)>;

pub(super) struct Captured {
    voices: BTreeMap<(LineId, Option<usize>), (i32, Vec<Voice>)>,
    mixed: Locations,
}

pub(super) struct RevisionInput<'a> {
    pub hits: &'a Hitsounds,
    pub file: &'a OsuFile,
    pub root: &'a Path,
    pub pending: &'a Pending,
    pub defaults: &'a Path,
    pub staged: &'a [StagedMix],
}

impl Captured {
    /// The moments whose gains a Save must record: those with their own hit volumes, and each
    /// sound set placement's moment that this Save writes as a mix (`mixed`), so the set reopens
    /// with its sounds (M4). A set written natively needs no record.
    pub(super) fn read(hits: &Hitsounds, root: &Path, pending: &Pending, defaults: &Path, mixed: &Locations) -> Result<Self, String> {
        Self::read_with_hs(hits, root, pending, defaults, mixed, None)
    }

    /// Native `.osu` encodes any uniform-volume moment cleanly regardless of `moment.volume()`,
    /// so gain recovery is only needed when hits in the same moment play at differing volumes.
    fn has_differing_hit_volumes(moment: &dawsu_hitsounding::Moment) -> bool {
        let mut vols = moment.hits().map(|lane| moment.hit_volume(lane));
        vols.next().is_some_and(|first| vols.any(|v| v != first))
    }

    pub(super) fn read_with_hs(
        hits: &Hitsounds,
        root: &Path,
        pending: &Pending,
        defaults: &Path,
        mixed: &Locations,
        hs_circles: Option<(&OsuFile, &BTreeMap<i64, LineId>)>,
    ) -> Result<Self, String> {
        let mut assets = Assets::new(root, pending, defaults, &[]);
        let mut voices = BTreeMap::new();
        let placed: std::collections::BTreeSet<_> = hits.placements().iter().map(|p| p.moment.0).collect();
        for (i, moment) in hits.moments().iter().enumerate() {
            let loc = location(moment.place()).or_else(|| {
                let (base, circles) = hs_circles?;
                circles
                    .get(&dawsu_hitsounding::hs::hs_group_time(base, moment.time()))
                    .map(|line| (*line, None))
            });
            let Some(location) = loc else { continue };
            if Self::has_differing_hit_volumes(moment) || placed.contains(&i) && mixed.contains(&location) {
                voices.insert(location, (moment.volume(), assets.voices(hits, moment)?));
            }
        }
        Ok(Self { voices, mixed: mixed.clone() })
    }

    pub(super) fn needs_recovery(&self) -> bool {
        !self.voices.is_empty()
    }

    pub(super) fn revision(
        self, input: &RevisionInput<'_>, cancel: &dyn Fn() -> bool,
    ) -> Result<Revision, String> {
        let RevisionInput { hits, file, root, pending, defaults, staged } = *input;
        let map_digest = sha256_hex(&file.to_bytes());
        if self.voices.is_empty() {
            return Ok(Revision { map_digest, moments: Vec::new() });
        }
        // Re-fingerprint source assets after mixing so a stale input cannot certify a saved mix.
        if self.voices != Self::read_with_hs(hits, root, pending, defaults, &self.mixed, None)?.voices
            && !self.voices.is_empty()
        {
            // HS-mode Place::NoObject entries are validated via their captured circle locations.
            let non_hs = Self::read(hits, root, pending, defaults, &self.mixed)?.voices;
            if non_hs.iter().any(|(k, v)| self.voices.get(k) != Some(v)) {
                return Err("A source sample changed while preparing gain recovery".into());
            }
        }
        let mut listing = listing(root, pending)?;
        listing.extend(staged.iter().filter_map(|mix| mix.path.file_name()?.to_str().map(str::to_owned)));
        let Decompiled::Done(decoded) = decompile(file, &listing, cancel, &mut |_, _| {}) else {
            return Err("Could not verify the output for gain recovery".into());
        };
        let ordinal: BTreeMap<_, _> = file.hit_objects().iter().enumerate().map(|(i, object)| (object.line(), i)).collect();
        let mut assets = Assets::new(root, pending, defaults, staged);
        let mut moments = Vec::with_capacity(self.voices.len());
        for moment in decoded.moments() {
            let Some((object, node)) = location(moment.place()) else { continue };
            let Some((default_volume, sources)) = self.voices.get(&(object, node)) else { continue };
            if cancel() {
                return Err("Save cancelled".into());
            }
            let object = *ordinal.get(&object).ok_or("Saved hit has no object ordinal")?;
            moments.push(AuthoredMoment {
                object, node, time_bits: moment.time().ms().to_bits(), default_volume: *default_volume,
                sources: sources.clone(), output: assets.voices(&decoded, moment)?,
            });
        }
        if moments.len() != self.voices.len() {
            return Err("Some edited gains have no matching output moment".into());
        }
        Ok(Revision { map_digest, moments })
    }
}

/// The moments of `file` that play one of the mixes this Save renders (`staged`).
/// Pending files the Save writes are not mixes, so `staged` holds only the mixes.
pub(super) fn mixed_locations(file: &OsuFile, root: &Path, pending: &Pending, staged: &[StagedMix], cancel: &dyn Fn() -> bool) -> Result<Locations, String> {
    let names: Vec<String> = staged.iter().filter_map(|mix| mix.path.file_name()?.to_str().map(str::to_owned)).collect();
    if names.is_empty() {
        return Ok(Locations::new());
    }
    let mut listing = listing(root, pending)?;
    listing.extend(names.iter().cloned());
    let Decompiled::Done(decoded) = decompile(file, &listing, cancel, &mut |_, _| {}) else {
        return Err("Could not read back the written difficulty".into());
    };
    // Save's merged files are numbered samples, so a hit plays one when its lane reads that file.
    let plays_staged = |lane: &SampleIdentity| match lane {
        SampleIdentity::File(name) => names.contains(name),
        _ => decoded.lane(lane).is_some_and(|lane| matches!(&lane.source, dawsu_hitsounding::LaneSource::File(name) if names.contains(name))),
    };
    Ok(decoded.moments().iter().filter(|moment| moment.hits().any(plays_staged))
        .filter_map(|moment| location(moment.place())).collect())
}

pub(super) fn restore(document: &mut Document, recovery: &Recovery, defaults: &Path) -> Result<(), String> {
    let digest = sha256_hex(document.file.original_bytes());
    let revision = match recovery.for_digest(&digest) {
        Some(revision) => revision,
        None if recovery.current.iter().chain(recovery.pending.iter()).any(|r| !r.moments.is_empty()) =>
            return Err("The difficulty changed outside dawsu!".into()),
        None => return Ok(()),
    };
    if revision.moments.is_empty() {
        return Ok(());
    }
    let objects = document.file.hit_objects();
    let root = document.path.parent().ok_or("Missing mapset folder")?;
    let original = document.composer.hitsounds();
    let by_location: BTreeMap<_, _> = original.moments().iter().enumerate()
        .filter_map(|(index, moment)| Some((location(moment.place())?, MomentId(index)))).collect();
    let mut restored = original.clone();
    let mut output_assets = Assets::new(root, &document.pending, defaults, &[]);
    let mut source_assets = Assets::new(root, &document.pending, defaults, &[]);

    for saved in &revision.moments {
        let object = objects.get(saved.object).ok_or("Saved hit object is missing")?;
        let id = *by_location.get(&(object.line(), saved.node)).ok_or("Saved hit point is missing")?;
        let moment = original.moment(id);
        if moment.time().ms().to_bits() != saved.time_bits
            || output_assets.voices(original, moment)? != saved.output
        {
            return Err("A rendered sample or its hit point changed".into());
        }
        let sources = saved.sources.iter().map(|voice| {
            let lane = lane_identity(&voice.lane).ok_or("Invalid stored sample identity")?;
            Ok((lane, voice.volume))
        }).collect::<Result<Vec<_>, String>>()?;
        restored.restore_moment_hits(id, saved.default_volume, sources);
        if source_assets.voices(&restored, restored.moment(id))? != saved.sources {
            return Err("An original sample changed".into());
        }
    }
    document.composer.history.update_external(|hits| *hits = restored.clone());
    document.composer.lanes = restored.lanes().iter().map(|lane| lane.identity.clone()).collect();
    Ok(())
}
