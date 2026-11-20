//! Drum hit detection (F027), the backend of the drum detection window: the whole song or a drum
//! stem, kick and snare lanes, a review of every detected hit, and Apply as one undo step.
//!
//! The analysis runs on its own thread with its own cancel flag, result channel and progress; it
//! never sets `busy`. Its curves are kept, keyed by the document's song load (the analysis stamp),
//! so sensitivity, snap distance, grid, lanes and the stem's offset only re-plan. Apply checks a
//! review stamp (history, tempo, grid, options, offsets) against the one the mapper saw, and
//! refuses on any difference. Listen plays one in-memory clip, the source on the left and cues on
//! the right, and stops whenever the result is dropped.

use crate::{Application, project};
use dawsu_audio::AudioClip;
use dawsu_editor_composer::{BatchCounts, BatchHit, BatchOptions, BatchRefusal, Lands, Policy};
use dawsu_foundation::SampleIdentity;
use dawsu_graph_audio::classic::{self, Curves, Drum, GridSnap, Onset};
use dawsu_hitsounding::Place;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};

const LABEL: &str = "Drum hit detection";
/// The longest "Snap to objects within" distance the window accepts.
pub const MAX_OBJECT_MS: f64 = 100.0;
/// The beat grids the window offers.
pub const GRIDS: [u32; 8] = [1, 2, 3, 4, 6, 8, 12, 16];

/// The kick and snare lanes last used for this difficulty, saved in the project. Optional, so
/// older projects load and older builds ignore it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DrumLanes {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kick: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snare: Option<String>,
}

/// What is analysed: the difficulty's song, or a drum stem file.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum DrumSource {
    Song,
    Stem(PathBuf),
}

/// The review's options. Changing any of them only re-plans; nothing is analysed again.
#[derive(Debug, Clone, PartialEq)]
pub struct DrumOptions {
    pub kick: Option<SampleIdentity>,
    pub snare: Option<SampleIdentity>,
    /// 0 strict to 100 loose; 50 is 3.1.4's middle (threshold scale 1.5 of 2.0 to 1.0).
    pub sensitivity: u8,
    /// How far a hit may be from an object and still land on it (3.1.4: 30 ms).
    pub object_ms: f64,
    /// The beat grid's divisor; `None` follows the editor's snap.
    pub grid: Option<u32>,
    /// Where the drum stem starts in the song, in ms. Not used for the song itself.
    pub stem_offset: f64,
}

impl Default for DrumOptions {
    fn default() -> Self {
        Self { kick: None, snare: None, sensitivity: 50, object_ms: 30.0, grid: None, stem_offset: 0.0 }
    }
}

/// The threshold scale for a sensitivity, linear from 2.0 (0) to 1.0 (100).
fn scale(sensitivity: u8) -> f64 {
    2.0 - f64::from(sensitivity.min(100)) / 100.0
}

/// Which song load the curves came from: a counter bumped each time a difficulty is loaded.
type AnalysisStamp = u64;

/// A finished analysis: its curves and the audio they came from (for Listen).
struct Analysed {
    curves: Curves,
    audio: Arc<AudioClip>,
}

enum Outcome {
    Done(Analysed),
    Cancelled,
    Failed(String),
}

struct Job {
    result: mpsc::Receiver<Outcome>,
    cancel: Arc<AtomicBool>,
    progress: Arc<Mutex<(usize, usize)>>,
    stamp: AnalysisStamp,
    source: DrumSource,
}

struct Ready {
    curves: Arc<Curves>,
    audio: Arc<AudioClip>,
    stamp: AnalysisStamp,
    source: DrumSource,
    review: Option<Review>,
}

/// One planned review, for the review stamp `stamp`.
#[derive(Clone)]
struct Review {
    stamp: u64,
    rows: Vec<DrumRow>,
    counts: BatchCounts,
    unhides: Vec<String>,
}

#[derive(Default)]
pub(crate) struct DrumDetect {
    job: Option<Job>,
    ready: Option<Ready>,
    song: AnalysisStamp,
    /// This session's options; `None` until the window sets them (the saved lanes fill the default).
    options: Option<DrumOptions>,
    /// Why the last analysis failed.
    error: Option<String>,
}

impl DrumDetect {
    /// A difficulty was loaded: anything analysed or chosen for the old one no longer applies.
    pub(crate) fn song_loaded(&mut self) {
        self.song += 1;
        self.options = None;
        self.clear();
    }

    /// Stops a running analysis and forgets the result. Nothing is left behind.
    fn clear(&mut self) {
        if let Some(job) = self.job.take() {
            job.cancel.store(true, Ordering::Relaxed);
        }
        self.ready = None;
        self.error = None;
    }
}

/// The object a hit lands on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectKind {
    Circle,
    SliderHead,
    SliderRepeat,
    SliderEnd,
    SpinnerEnd,
}

/// Where one detected hit goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowLands {
    Object(ObjectKind),
    /// On a grid line with no object.
    Grid,
    /// Too far from every object and grid line: left out unless the mapper checks it.
    Flagged,
    /// Its lane already has a hit there.
    Duplicate,
    /// No lane is picked for its drum.
    NoLane,
}

/// One detected hit in the review.
#[derive(Debug, Clone, PartialEq)]
pub struct DrumRow {
    /// Stable across re-plans: the drum and its detection frame.
    pub id: String,
    pub drum: Drum,
    /// Song time in ms where it lands, or where it was heard (lag taken off) when it lands nowhere.
    pub time: f64,
    /// Its score relative to the run's strongest hit, 0 to 1.
    pub strength: f64,
    pub lands: RowLands,
}

impl DrumRow {
    /// Hits on an object or a grid line start checked; flagged ones start unchecked.
    pub fn checked_by_default(&self) -> bool {
        matches!(self.lands, RowLands::Object(_) | RowLands::Grid)
    }
    /// Duplicates and hits without a lane can't be added.
    pub fn can_check(&self) -> bool {
        !matches!(self.lands, RowLands::Duplicate | RowLands::NoLane)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum DrumDetectState {
    Idle,
    Running { done: usize, total: usize },
    /// `counts` are for the hits checked by default; `unhides` names the hidden lanes they show.
    Ready { review: u64, rows: Vec<DrumRow>, counts: BatchCounts, unhides: Vec<String> },
}

#[derive(Debug, Clone, PartialEq)]
pub struct DrumDetectView {
    pub state: DrumDetectState,
    /// The source being analysed or reviewed.
    pub source: Option<DrumSource>,
    /// Why the last analysis failed.
    pub error: Option<String>,
    pub options: DrumOptions,
    /// The editor's snap divisor, the grid when the window picks none.
    pub editor_grid: u32,
    pub has_song: bool,
    /// Lanes that can take the hits, with their names and whether each is hidden.
    pub lanes: Vec<(SampleIdentity, String, bool)>,
    pub listening: bool,
}

/// The detected hits planned against the map.
struct Planned {
    onsets: Vec<Onset>,
    /// Per onset, its batch hit (`None` when its drum has no lane).
    hit_of: Vec<Option<usize>>,
    hits: Vec<BatchHit>,
    options: BatchOptions,
    /// Song time minus analysed audio time.
    offset: f64,
}

impl Application {
    /// The options the window shows: this session's, or the defaults with the saved lanes.
    pub fn drum_options(&self) -> DrumOptions {
        if let Some(options) = &self.drum.options {
            return options.clone();
        }
        let document = self.document.as_ref();
        let saved = |key: Option<&String>| key.and_then(|key| project::lane_identity(key))
            .filter(|lane| !slider_body(lane))
            .filter(|lane| document.is_some_and(|d| d.composer.hitsounds().lane(lane).is_some()));
        let mapping = document.and_then(|d| d.drum_lanes.as_ref());
        DrumOptions {
            kick: saved(mapping.and_then(|m| m.kick.as_ref())),
            snare: saved(mapping.and_then(|m| m.snare.as_ref())),
            ..DrumOptions::default()
        }
    }

    /// Sets the review's options; the review re-plans on the next poll.
    pub fn set_drum_options(&mut self, options: DrumOptions) -> Result<(), String> {
        let document = self.document.as_ref().ok_or("Open a difficulty first.")?;
        if options.kick.is_some() && options.kick == options.snare {
            return Err("Pick different lanes for kicks and snares.".into());
        }
        if [&options.kick, &options.snare].into_iter().flatten()
            .any(|lane| slider_body(lane) || document.composer.hitsounds().lane(lane).is_none()) {
            return Err("A chosen lane no longer exists. Pick the lanes again.".into());
        }
        if options.sensitivity > 100 {
            return Err("Sensitivity goes from 0 to 100.".into());
        }
        if !(0.0..=MAX_OBJECT_MS).contains(&options.object_ms) {
            return Err(format!("Snap to objects within 0 to {MAX_OBJECT_MS} ms."));
        }
        if options.grid.is_some_and(|grid| !GRIDS.contains(&grid)) {
            return Err("Pick a beat grid from the list.".into());
        }
        if !options.stem_offset.is_finite() {
            return Err("Enter the stem's offset in ms.".into());
        }
        self.drum.options = Some(options);
        Ok(())
    }

    /// Starts analysing the song or a drum stem for kick and snare hits. Refused without a
    /// difficulty, or without its song when the song is the source; a run already going is replaced.
    pub fn start_drum_detection(&mut self, source: DrumSource) -> Result<(), String> {
        if self.busy || self.is_stroke_open() {
            return Err("Wait for the current job before detecting drum hits.".into());
        }
        if self.document.is_none() {
            return Err("Open a difficulty first.".into());
        }
        let song = match &source {
            DrumSource::Song => Some(self.song.clone().ok_or("This difficulty's song could not be loaded. Pick a drum stem.")?),
            DrumSource::Stem(_) => None,
        };
        self.clear_drum_detection();
        let cancel = Arc::new(AtomicBool::new(false));
        let progress = Arc::new(Mutex::new((0, 1)));
        let (send, result) = mpsc::channel();
        {
            let (cancel, progress, source) = (cancel.clone(), progress.clone(), source.clone());
            std::thread::spawn(move || {
                let stop = || cancel.load(Ordering::Relaxed);
                let audio = match (song, &source) {
                    (Some(song), _) => song,
                    (None, DrumSource::Stem(path)) => match dawsu_audio::decode_with_cancel(path, &stop) {
                        Ok(audio) => audio,
                        Err(_) if stop() => return,
                        Err(error) => {
                            let _ = send.send(Outcome::Failed(format!("The drum stem could not be read. {error}")));
                            return;
                        }
                    },
                    (None, DrumSource::Song) => unreachable!("the song was checked above"),
                };
                let mono = mono_mix(&audio);
                let outcome = match classic::analyse(&mono, audio.sample_rate, &stop, &mut |done, total| {
                    if let Ok(mut current) = progress.lock() {
                        *current = (done, total);
                    }
                }) {
                    Ok(curves) => {
                        drop(mono);
                        Outcome::Done(Analysed { curves, audio })
                    }
                    Err(classic::Cancelled) => Outcome::Cancelled,
                };
                let _ = send.send(outcome);
            });
        }
        self.drum.job = Some(Job { result, cancel, progress, stamp: self.drum.song, source });
        Ok(())
    }

    /// Stops the run and Listen, and forgets the result.
    pub fn cancel_drum_detection(&mut self) {
        self.clear_drum_detection();
    }

    /// Everything drum hit detection holds goes: the run, its result and Listen.
    pub(crate) fn clear_drum_detection(&mut self) {
        self.drum.clear();
        self.stop_drum_listen();
    }

    /// Called from `poll`: collects a finished analysis and re-plans after edits or new options.
    pub(crate) fn poll_drum_detection(&mut self) {
        if self.document.is_none() {
            if self.drum.job.is_some() || self.drum.ready.is_some() {
                self.clear_drum_detection();
            }
            return;
        }
        if let Some(job) = &self.drum.job {
            if job.stamp != self.drum.song {
                self.clear_drum_detection();
                return;
            }
            let outcome = match job.result.try_recv() {
                Ok(outcome) => outcome,
                Err(mpsc::TryRecvError::Disconnected) => Outcome::Cancelled,
                Err(mpsc::TryRecvError::Empty) => return,
            };
            let job = self.drum.job.take().expect("checked");
            match outcome {
                Outcome::Done(analysed) => {
                    self.drum.ready = Some(Ready {
                        curves: Arc::new(analysed.curves), audio: analysed.audio,
                        stamp: job.stamp, source: job.source, review: None,
                    });
                }
                Outcome::Failed(error) => self.drum.error = Some(error),
                Outcome::Cancelled => {}
            }
        }
        let Some(ready) = &self.drum.ready else { return };
        if ready.stamp != self.drum.song {
            self.clear_drum_detection();
            return;
        }
        let options = self.drum_options();
        let stamp = self.drum_review_stamp(ready, &options);
        if ready.review.as_ref().is_some_and(|review| review.stamp == stamp) {
            return;
        }
        let review = self.drum_review(ready, &options, stamp);
        self.drum.ready.as_mut().expect("checked").review = Some(review);
    }

    /// Adds the checked hits (`ticked`, by row id) as one undo step. `review` is the stamp the
    /// mapper saw; any edit or change of options since then refuses, and the review re-plans.
    pub fn apply_drum_detection(&mut self, review: u64, ticked: &[String]) -> Result<(), String> {
        if self.busy || self.is_stroke_open() {
            return Err("Wait for the current job before applying drum hits.".into());
        }
        let ready = self.drum.ready.as_ref().ok_or("Detect drum hits first.")?;
        if ready.stamp != self.drum.song {
            self.clear_drum_detection();
            return Err("The song changed. Detect drum hits again.".into());
        }
        let options = self.drum_options();
        if self.drum_review_stamp(ready, &options) != review {
            return Err("The map changed since the review. Check the new list, then apply.".into());
        }
        let planned = self.drum_plan(&ready.curves, &ready.source, &options);
        let document = self.document.as_ref().ok_or("Open a difficulty first.")?;
        let rows = review_rows(document.composer.hitsounds(), &planned,
            &document.composer.plan_batch(&planned.hits, &planned.options));
        let checked: Vec<&DrumRow> = rows.iter().filter(|row| row.can_check() && ticked.contains(&row.id)).collect();
        if checked.is_empty() {
            return Err("Check at least one hit to add.".into());
        }
        // A checked flagged hit goes in where it was heard, its lag taken off.
        let hits: Vec<BatchHit> = checked.iter().map(|row| {
            let index = planned.onsets.iter().position(|onset| onset_id(onset) == row.id).expect("a row per onset");
            let mut hit = planned.hits[planned.hit_of[index].expect("a checkable row has a lane")].clone();
            if row.lands == RowLands::Flagged {
                hit.grid = Some(row.time);
            }
            hit
        }).collect();
        let document = self.document.as_mut().expect("checked");
        let plan = document.composer.plan_batch(&hits, &planned.options);
        let unhides = shown_lanes(document.composer.hitsounds(), &hits, &plan);
        let report = document.composer.add_batch(LABEL, &hits, &planned.options).map_err(|refusal| match refusal {
            BatchRefusal::StepOpen => "Finish the current edit before applying drum hits.".to_owned(),
            BatchRefusal::MissingLane(_) => "A chosen lane no longer exists. Pick the lanes again.".to_owned(),
        })?;
        document.drum_lanes = Some(DrumLanes {
            kick: options.kick.as_ref().map(project::lane_key),
            snare: options.snare.as_ref().map(project::lane_key),
        });
        self.clear_drum_detection();
        self.refresh_recovery_edit();
        let c = report.counts;
        let mut status = format!("Added {} drum hits: {} on objects, {} on grid lines.", c.added, c.on_objects, c.loose);
        if c.duplicates > 0 {
            status.push_str(&format!(" {} already there.", c.duplicates));
        }
        for name in unhides {
            status.push_str(&format!(" Shows hidden lane: {name}."));
        }
        self.status = status;
        Ok(())
    }

    /// Plays the analysed audio from `from` (song time, ms) on the left and a cue at each checked
    /// hit on the right, in place of anything Listen played before. The song pauses first.
    pub fn listen_drum_detection(&mut self, from: f64, ticked: &[String]) -> Result<(), String> {
        let ready = self.drum.ready.as_ref().ok_or("Detect drum hits first.")?;
        let review = ready.review.as_ref().ok_or("Detect drum hits first.")?;
        let audio = self.audio.as_ref().ok_or("Audio output is unavailable.")?;
        let cues: Vec<(Drum, f64)> = review.rows.iter()
            .filter(|row| row.can_check() && ticked.contains(&row.id))
            .map(|row| (row.drum, row.time)).collect();
        let options = self.drum_options();
        let clip = listen_clip(&ready.audio, self.song_offset + source_offset(&ready.source, &options), from.max(0.0), &cues);
        audio.play(false);
        audio.listen(Some(Arc::new(clip)));
        Ok(())
    }

    pub fn stop_drum_listen(&mut self) {
        if let Some(audio) = &self.audio {
            audio.listen(None);
        }
    }

    pub fn drum_detection(&self) -> DrumDetectView {
        let state = match (&self.drum.job, &self.drum.ready) {
            (Some(job), _) => {
                let (done, total) = job.progress.lock().map(|p| *p).unwrap_or((0, 1));
                DrumDetectState::Running { done, total }
            }
            (None, Some(Ready { review: Some(review), .. })) => DrumDetectState::Ready {
                review: review.stamp, rows: review.rows.clone(), counts: review.counts, unhides: review.unhides.clone(),
            },
            _ => DrumDetectState::Idle,
        };
        let source = self.drum.job.as_ref().map(|job| job.source.clone())
            .or_else(|| self.drum.ready.as_ref().map(|ready| ready.source.clone()));
        let document = self.document.as_ref();
        let lanes = document.map(|d| d.composer.hitsounds().lanes().iter()
            .filter(|lane| !slider_body(&lane.identity))
            .map(|lane| (lane.identity.clone(), crate::lane_name(&lane.identity), d.composer.hitsounds().is_lane_hidden(&lane.identity)))
            .collect()).unwrap_or_default();
        DrumDetectView {
            state,
            source,
            error: self.drum.error.clone(),
            options: self.drum_options(),
            editor_grid: document.map_or(4, |d| d.composer.spacing.divisions()),
            has_song: self.song.is_some(),
            lanes,
            listening: self.audio.as_ref().is_some_and(dawsu_audio::AudioEngine::is_listening),
        }
    }

    fn drum_review(&self, ready: &Ready, options: &DrumOptions, stamp: u64) -> Review {
        let planned = self.drum_plan(&ready.curves, &ready.source, options);
        let composer = &self.document.as_ref().expect("a document").composer;
        let plan = composer.plan_batch(&planned.hits, &planned.options);
        let rows = review_rows(composer.hitsounds(), &planned, &plan);
        // The counts and hidden lanes for the hits checked by default.
        let placed = |row: &&DrumRow| row.checked_by_default();
        let mut counts = BatchCounts::default();
        for row in rows.iter().filter(placed) {
            counts.added += 1;
            match row.lands {
                RowLands::Object(_) => counts.on_objects += 1,
                _ => counts.loose += 1,
            }
        }
        counts.flagged = rows.iter().filter(|row| row.lands == RowLands::Flagged).count();
        counts.duplicates = rows.iter().filter(|row| row.lands == RowLands::Duplicate).count();
        let unhides = shown_lanes(composer.hitsounds(), &planned.hits, &plan);
        Review { stamp, rows, counts, unhides }
    }

    /// The detected hits as batch hits in song time with their grid lines, and the batch options
    /// with the measured lag.
    fn drum_plan(&self, curves: &Curves, source: &DrumSource, options: &DrumOptions) -> Planned {
        let mut batch = BatchOptions { policy: Policy::Snap, object_ms: options.object_ms, ..BatchOptions::default() };
        let offset = self.song_offset + source_offset(source, options);
        let onsets = classic::pick(curves, scale(options.sensitivity));
        let Some(document) = self.document.as_ref() else {
            return Planned { hit_of: vec![None; onsets.len()], onsets, hits: Vec::new(), options: batch, offset };
        };
        let composer = &document.composer;
        let divisor = options.grid.unwrap_or_else(|| composer.spacing.divisions());
        let grid = |time: f64, divisor: u32| composer.grid_line(time, divisor);
        let (placed, shift) = batch_hits(&onsets, offset, &(options.kick.clone(), options.snare.clone()), &grid, divisor);
        let mut hit_of = vec![None; onsets.len()];
        let hits = placed.into_iter().enumerate().map(|(n, (onset, hit))| {
            hit_of[onset] = Some(n);
            hit
        }).collect();
        batch.shift_ms = shift;
        Planned { onsets, hit_of, hits, options: batch, offset }
    }

    fn drum_review_stamp(&self, ready: &Ready, options: &DrumOptions) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        ready.stamp.hash(&mut hasher);
        ready.source.hash(&mut hasher);
        if let Some(document) = &self.document {
            let composer = &document.composer;
            document.path.hash(&mut hasher);
            composer.history.version().hash(&mut hasher);
            for section in composer.tempo.sections() {
                (section.start.ms().to_bits(), section.beat_length_ms.to_bits()).hash(&mut hasher);
            }
            (composer.spacing.beats(), composer.spacing.divisions(), composer.insert_volume).hash(&mut hasher);
        }
        (&options.kick, &options.snare, options.sensitivity, options.grid).hash(&mut hasher);
        (options.object_ms.to_bits(), source_offset(&ready.source, options).to_bits()).hash(&mut hasher);
        self.song_offset.to_bits().hash(&mut hasher);
        hasher.finish()
    }
}

/// How far the source's own audio sits from the song's, in ms.
fn source_offset(source: &DrumSource, options: &DrumOptions) -> f64 {
    match source {
        DrumSource::Song => 0.0,
        DrumSource::Stem(_) => options.stem_offset,
    }
}

fn onset_id(onset: &Onset) -> String {
    let drum = match onset.drum { Drum::Kick => "k", Drum::Snare => "s" };
    format!("{drum}{}", onset.frame)
}

/// One row per detected hit, in time order.
fn review_rows(model: &dawsu_hitsounding::Hitsounds, planned: &Planned, plan: &dawsu_editor_composer::BatchPlan) -> Vec<DrumRow> {
    let strongest = planned.onsets.iter().map(|onset| onset.score).fold(0.0, f64::max);
    let mut last_node: HashMap<_, usize> = HashMap::new();
    for moment in model.moments() {
        if let Place::SliderNode { object, node } = moment.place() {
            let last = last_node.entry(object).or_insert(0);
            *last = (*last).max(node);
        }
    }
    let heard = |onset: &Onset| onset.time_ms + planned.offset - planned.options.shift_ms;
    let mut rows: Vec<DrumRow> = planned.onsets.iter().zip(&planned.hit_of).filter_map(|(onset, hit)| {
        let (time, lands) = match hit.map(|h| plan.rows[h]) {
            None => (heard(onset), RowLands::NoLane),
            Some(row) => match row.lands {
                Lands::Object(id) => {
                    let kind = match model.moment(id).place() {
                        Place::Circle(_) | Place::NoObject => ObjectKind::Circle,
                        Place::SliderNode { node: 0, .. } => ObjectKind::SliderHead,
                        Place::SliderNode { object, node } if last_node.get(&object) == Some(&node) => ObjectKind::SliderEnd,
                        Place::SliderNode { .. } => ObjectKind::SliderRepeat,
                        Place::SpinnerEnd(_) => ObjectKind::SpinnerEnd,
                    };
                    (row.time, RowLands::Object(kind))
                }
                Lands::Loose => (row.time, RowLands::Grid),
                Lands::Duplicate => (row.time, RowLands::Duplicate),
                Lands::Flagged | Lands::OffGrid | Lands::Unmatched => (heard(onset), RowLands::Flagged),
                // Before the song's start: nothing can go there.
                Lands::Skipped => return None,
            },
        };
        let strength = if strongest > 0.0 { onset.score / strongest } else { 0.0 };
        Some(DrumRow { id: onset_id(onset), drum: onset.drum, time, strength, lands })
    }).collect();
    rows.sort_by(|a, b| a.time.total_cmp(&b.time).then(a.drum.cmp(&b.drum)));
    rows
}

/// Detected hits in song time on their lanes, each with the index of its onset and its grid line.
/// The lag is measured over every detected hit, also those whose drum has no lane, as 3.1.4 did.
fn batch_hits(onsets: &[Onset], offset: f64, lanes: &(Option<SampleIdentity>, Option<SampleIdentity>), grid: GridSnap, divisor: u32)
    -> (Vec<(usize, BatchHit)>, f64) {
    let shift = classic::grid_shift(onsets.iter().map(|o| o.time_ms + offset), grid, divisor);
    let hits = onsets.iter().enumerate().filter_map(|(index, onset)| {
        let lane = match onset.drum { Drum::Kick => lanes.0.clone(), Drum::Snare => lanes.1.clone() }?;
        let time = onset.time_ms + offset;
        Some((index, BatchHit { time, lane, strength: onset.score, grid: classic::on_grid(time, grid, &[divisor], shift) }))
    }).collect();
    (hits, shift)
}

/// The names of hidden lanes that the placed hits would show again.
fn shown_lanes(model: &dawsu_hitsounding::Hitsounds, hits: &[BatchHit], plan: &dawsu_editor_composer::BatchPlan) -> Vec<String> {
    let mut lanes: Vec<&SampleIdentity> = plan.rows.iter()
        .filter(|row| matches!(row.lands, Lands::Object(_) | Lands::Loose))
        .map(|row| &hits[row.hit].lane).filter(|lane| model.is_lane_hidden(lane)).collect();
    lanes.sort();
    lanes.dedup();
    lanes.into_iter().map(crate::lane_name).collect()
}

/// Slider body lanes play along a slider, not at a moment; drum hits never go there.
fn slider_body(lane: &SampleIdentity) -> bool {
    matches!(lane, SampleIdentity::Standard { sound: dawsu_foundation::Sound::Slider(_), .. })
}

/// The audio as mono, the channels averaged (3.1.4 used (left + right) / 2). Mono is borrowed.
fn mono_mix(clip: &AudioClip) -> Cow<'_, [f32]> {
    match clip.channels.max(1) {
        1 => Cow::Borrowed(&clip.samples),
        2 => Cow::Owned(clip.samples.chunks_exact(2).map(|f| (f[0] + f[1]) * 0.5).collect()),
        channels => Cow::Owned(clip.samples.chunks_exact(channels).map(|f| f.iter().sum::<f32>() / channels as f32).collect()),
    }
}

/// Listen's clip, 3.1.4's `AuditionWav` in memory: from `from` (song time) to the end of `audio`,
/// whose first sample sits at song time `offset`. Left: the audio as mono. Right: a low thump at
/// each kick and a noise burst at each snare (not the mapper's samples).
fn listen_clip(audio: &AudioClip, offset: f64, from: f64, cues: &[(Drum, f64)]) -> AudioClip {
    let rate = f64::from(audio.sample_rate.max(1));
    let channels = audio.channels.max(1);
    let source_frames = audio.samples.len() / channels;
    let first = ((from - offset) * rate / 1000.0).round() as i64;
    let frames = (source_frames as i64 - first).max(0) as usize;
    let mut samples = vec![0.0_f32; frames * 2];
    for (i, frame) in samples.chunks_exact_mut(2).enumerate() {
        let at = first + i as i64;
        if at >= 0 && (at as usize) < source_frames {
            let start = at as usize * channels;
            frame[0] = audio.samples[start..start + channels].iter().sum::<f32>() / channels as f32;
        }
    }
    // xorshift, seeded like 3.1.4's Random(1): the same burst on every Listen.
    let mut seed: u32 = 1;
    let mut noise = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        f64::from(seed) / f64::from(u32::MAX) * 2.0 - 1.0
    };
    for &(drum, time) in cues {
        let at = ((time - from) * rate / 1000.0).round() as i64;
        let kick = drum == Drum::Kick;
        let length = (rate * if kick { 0.150 } else { 0.090 }) as i64;
        for i in 0..length {
            let index = at + i;
            if index < 0 {
                continue;
            }
            let Some(frame) = samples.get_mut(index as usize * 2..index as usize * 2 + 2) else { break };
            let t = i as f64 / rate;
            let value = if kick {
                (2.0 * std::f64::consts::PI * (55.0 * t + 60.0 * (1.0 - (-t * 30.0).exp()) / 30.0)).sin() * (-t * 25.0).exp()
            } else {
                (noise() * 0.7 + (2.0 * std::f64::consts::PI * 190.0 * t).sin() * 0.3) * (-t * 40.0).exp()
            };
            frame[1] += (value * 0.8) as f32;
        }
    }
    AudioClip { samples, channels: 2, sample_rate: audio.sample_rate }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawsu_foundation::{Hitsound, SampleSet, Sound};
    use dawsu_test_support::Scratch;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};

    fn lane(sound: Hitsound) -> SampleIdentity {
        SampleIdentity::Standard { set: SampleSet::Soft, sound: Sound::Hit(sound), index: dawsu_foundation::CustomIndex::new(0) }
    }

    fn pattern() -> AudioClip {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../dawsu-graph-audio/tests/fixtures/classic-3.1.4/pattern.f32");
        let samples = std::fs::read(path).unwrap().chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap())).collect();
        AudioClip { samples, channels: 1, sample_rate: 44_100 }
    }

    /// The pattern as a 16-bit mono WAV at half level (it peaks above 1.0), for a drum stem.
    fn write_stem(path: &Path) {
        let audio = pattern();
        let size = audio.samples.len() * 2;
        let mut wav = Vec::with_capacity(44 + size);
        wav.extend(b"RIFF");
        wav.extend(((36 + size) as u32).to_le_bytes());
        wav.extend(b"WAVEfmt ");
        wav.extend(16_u32.to_le_bytes());
        wav.extend(1_u16.to_le_bytes());
        wav.extend(1_u16.to_le_bytes());
        wav.extend(44_100_u32.to_le_bytes());
        wav.extend((44_100_u32 * 2).to_le_bytes());
        wav.extend(2_u16.to_le_bytes());
        wav.extend(16_u16.to_le_bytes());
        wav.extend(b"data");
        wav.extend((size as u32).to_le_bytes());
        for sample in audio.samples {
            wav.extend((((sample * 0.5).clamp(-1.0, 1.0) * 32767.0).round() as i16).to_le_bytes());
        }
        std::fs::write(path, wav).unwrap();
    }

    /// 120 BPM; objects on every kick (500 + 1000k ms); one whistle+clap circle at 0 so both lanes exist.
    fn map(fixture: &Scratch) -> PathBuf {
        map_with(fixture, "")
    }

    fn map_with(fixture: &Scratch, extra: &str) -> PathBuf {
        let mut body = String::from("osu file format v14\n[General]\nMode:0\nSampleSet:Soft\n[TimingPoints]\n0,500,4,2,0,100,1,0\n[HitObjects]\n256,192,0,1,10,0:0:0:0:\n");
        body.push_str(extra);
        for k in 0..16 {
            body.push_str(&format!("256,192,{},1,0,0:0:0:0:\n", 500 + 1000 * k));
        }
        let path = fixture.path("map.osu");
        std::fs::write(&path, body).unwrap();
        path
    }

    fn open(fixture: &Scratch) -> Application {
        open_map(fixture, map(fixture))
    }

    fn open_map(fixture: &Scratch, path: PathBuf) -> Application {
        let mut app = Application::headless();
        app.project_root = Some(fixture.path("projects"));
        app.request_open(path);
        wait(&mut app, |app| !app.busy);
        assert!(app.error.is_none(), "{:?}", app.error);
        app.song = Some(Arc::new(pattern()));
        app
    }

    fn wait(app: &mut Application, until: impl Fn(&Application) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(60);
        while !until(app) {
            app.poll();
            assert!(Instant::now() < deadline, "timed out");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn set_lanes(app: &mut Application, kick: Option<SampleIdentity>, snare: Option<SampleIdentity>) {
        let options = DrumOptions { kick, snare, ..app.drum_options() };
        app.set_drum_options(options).unwrap();
    }

    fn change(app: &mut Application, edit: impl FnOnce(&mut DrumOptions)) {
        let mut options = app.drum_options();
        edit(&mut options);
        app.set_drum_options(options).unwrap();
    }

    fn ready(app: &Application) -> Option<(u64, Vec<DrumRow>)> {
        match app.drum_detection().state {
            DrumDetectState::Ready { review, rows, .. } => Some((review, rows)),
            _ => None,
        }
    }

    fn detect(app: &mut Application) -> (u64, Vec<DrumRow>) {
        app.start_drum_detection(DrumSource::Song).unwrap();
        wait(app, |app| ready(app).is_some());
        ready(app).unwrap()
    }

    fn checked(rows: &[DrumRow]) -> Vec<String> {
        rows.iter().filter(|row| row.checked_by_default()).map(|row| row.id.clone()).collect()
    }

    fn undo_steps(app: &mut Application) -> usize {
        let history = &mut app.document.as_mut().unwrap().composer.history;
        let mut n = 0;
        while history.undo() {
            n += 1;
        }
        for _ in 0..n {
            history.redo();
        }
        n
    }

    #[test]
    fn detection_runs_on_its_own_thread_and_apply_is_one_undo_step() {
        let fixture = Scratch::new("drum-detect-apply");
        let mut app = open(&fixture);
        set_lanes(&mut app, Some(lane(Hitsound::Whistle)), Some(lane(Hitsound::Clap)));
        app.start_drum_detection(DrumSource::Song).unwrap();
        assert!(!app.busy);
        assert!(matches!(app.drum_detection().state, DrumDetectState::Running { .. }));
        wait(&mut app, |app| ready(app).is_some());
        let (review, rows) = ready(&app).unwrap();
        let kicks: Vec<&DrumRow> = rows.iter().filter(|row| row.drum == Drum::Kick).collect();
        let snares: Vec<&DrumRow> = rows.iter().filter(|row| row.drum == Drum::Snare).collect();
        assert_eq!((kicks.len(), snares.len()), (16, 16));
        // Kicks land on the objects; snares between them land on their 1/4 grid lines.
        assert!(kicks.iter().all(|row| row.lands == RowLands::Object(ObjectKind::Circle)));
        assert!(snares.iter().all(|row| row.lands == RowLands::Grid));
        assert!(rows.windows(2).all(|pair| pair[0].time <= pair[1].time), "in time order");
        assert!(rows.iter().all(|row| (0.0..=1.0).contains(&row.strength)));
        assert!(rows.iter().any(|row| row.strength == 1.0));
        let DrumDetectState::Ready { counts, .. } = app.drum_detection().state else { unreachable!() };
        assert_eq!((counts.added, counts.on_objects, counts.loose, counts.flagged), (32, 16, 16, 0));
        app.apply_drum_detection(review, &checked(&rows)).unwrap();
        assert!(app.status.starts_with("Added 32 drum hits: 16 on objects, 16 on grid lines."), "{}", app.status);
        assert_eq!(undo_steps(&mut app), 1);
        let hits = app.document().unwrap().composer.hitsounds();
        let snare_times: Vec<f64> = hits.moments().iter().filter(|m| m.has_hit(&lane(Hitsound::Clap)) && m.time().ms() > 0.0)
            .map(|m| m.time().ms()).collect();
        assert_eq!(snare_times.len(), 16);
        assert!(snare_times.iter().all(|t| (t - 1000.0) % 1000.0 == 0.0), "{snare_times:?}");
        assert!(matches!(app.drum_detection().state, DrumDetectState::Idle));
        assert_eq!(app.document().unwrap().composer.selected.items().len(), 32);
    }

    #[test]
    fn options_only_re_plan_and_each_one_changes_the_review_stamp() {
        let fixture = Scratch::new("drum-detect-options");
        let mut app = open(&fixture);
        set_lanes(&mut app, Some(lane(Hitsound::Whistle)), Some(lane(Hitsound::Clap)));
        let (mut review, rows) = detect(&mut app);
        let curves = app.drum.ready.as_ref().unwrap().curves.clone();
        let changes: [(&str, Box<dyn FnOnce(&mut DrumOptions)>); 5] = [
            ("sensitivity", Box::new(|o| o.sensitivity = 90)),
            ("snap to objects", Box::new(|o| o.object_ms = 10.0)),
            ("grid", Box::new(|o| o.grid = Some(2))),
            ("kick lane", Box::new(|o| o.kick = None)),
            ("snare lane", Box::new(|o| o.snare = Some(lane(Hitsound::Whistle)))),
        ];
        for (name, edit) in changes {
            change(&mut app, edit);
            app.poll();
            assert!(app.drum.job.is_none(), "{name} analysed again");
            assert!(Arc::ptr_eq(&curves, &app.drum.ready.as_ref().unwrap().curves), "{name}");
            let (fresh, _) = ready(&app).expect(name);
            assert_ne!(fresh, review, "{name}");
            assert!(app.apply_drum_detection(review, &checked(&rows)).unwrap_err().contains("changed since the review"), "{name}");
            review = fresh;
        }
        // A looser sensitivity finds at least the same hits, under the same ids.
        let (_, loose) = ready(&app).unwrap();
        change(&mut app, |o| o.sensitivity = 10);
        app.poll();
        let (_, strict) = ready(&app).unwrap();
        assert!(strict.iter().all(|row| loose.iter().any(|other| other.id == row.id)));
        // Without a kick lane, kicks show but can't be checked.
        assert!(loose.iter().filter(|row| row.drum == Drum::Kick).all(|row| row.lands == RowLands::NoLane && !row.can_check()));
        // An edit in the editor re-plans too, and a hit already there shows as a duplicate.
        let (stamp, _) = ready(&app).unwrap();
        let first = loose.iter().find(|row| row.drum == Drum::Snare).unwrap().time;
        app.document.as_mut().unwrap().composer.add(first, lane(Hitsound::Whistle), 100, 5.0);
        app.poll();
        let (after, rows) = ready(&app).unwrap();
        assert_ne!(after, stamp);
        assert!(rows.iter().any(|row| row.lands == RowLands::Duplicate && !row.can_check()));
    }

    #[test]
    fn only_checked_hits_are_added_and_a_checked_flagged_hit_goes_in_where_it_was_heard() {
        let fixture = Scratch::new("drum-detect-ticked");
        let mut app = open(&fixture);
        // Half a grid step late: every hit is too far from objects and grid lines, and the lag is
        // too large to be taken off.
        app.song_offset = 62.5;
        set_lanes(&mut app, Some(lane(Hitsound::Whistle)), Some(lane(Hitsound::Clap)));
        let (review, rows) = detect(&mut app);
        assert!(rows.iter().all(|row| row.lands == RowLands::Flagged && !row.checked_by_default() && row.can_check()), "{rows:?}");
        assert_eq!(app.apply_drum_detection(review, &checked(&rows)).unwrap_err(), "Check at least one hit to add.");
        let kick = rows.iter().find(|row| row.drum == Drum::Kick).unwrap();
        app.apply_drum_detection(review, &[kick.id.clone()]).unwrap();
        assert_eq!(undo_steps(&mut app), 1);
        let added: Vec<_> = app.document().unwrap().composer.hitsounds().moments().iter()
            .filter(|m| m.has_hit(&lane(Hitsound::Whistle)) && m.time().ms() > 0.0).collect();
        assert_eq!(added.len(), 1);
        assert!((added[0].time().ms() - kick.time).abs() < 1.0, "{} vs {}", added[0].time().ms(), kick.time);
        assert_eq!(added[0].place(), Place::NoObject);
    }

    #[test]
    fn a_drum_stem_is_analysed_at_its_offset_without_the_song() {
        let fixture = Scratch::new("drum-detect-stem");
        let mut app = open(&fixture);
        app.song = None;
        assert!(app.start_drum_detection(DrumSource::Song).unwrap_err().contains("Pick a drum stem"));
        let stem = fixture.path("drums.wav");
        write_stem(&stem);
        set_lanes(&mut app, Some(lane(Hitsound::Whistle)), Some(lane(Hitsound::Clap)));
        change(&mut app, |o| o.stem_offset = 1000.0);
        app.start_drum_detection(DrumSource::Stem(stem.clone())).unwrap();
        wait(&mut app, |app| ready(app).is_some());
        let view = app.drum_detection();
        assert_eq!(view.source, Some(DrumSource::Stem(stem)));
        let (_, rows) = ready(&app).unwrap();
        let kicks: Vec<&DrumRow> = rows.iter().filter(|row| row.drum == Drum::Kick).collect();
        // The stem's kicks at 500 + 1000k ms sound at 1500 + 1000k ms in the song: on the objects.
        assert_eq!(kicks.len(), 16);
        assert_eq!(kicks[0].time, 1500.0);
        assert!(kicks[..15].iter().all(|row| row.lands == RowLands::Object(ObjectKind::Circle)));
        // A file that is no audio reports why and leaves nothing running.
        let broken = fixture.path("broken.wav");
        std::fs::write(&broken, b"not audio").unwrap();
        app.start_drum_detection(DrumSource::Stem(broken)).unwrap();
        wait(&mut app, |app| !matches!(app.drum_detection().state, DrumDetectState::Running { .. }));
        let view = app.drum_detection();
        assert!(matches!(view.state, DrumDetectState::Idle));
        assert!(view.error.unwrap().starts_with("The drum stem could not be read."));
    }

    #[test]
    fn the_listen_clip_has_the_source_on_the_left_and_cues_on_the_right() {
        let audio = AudioClip { samples: vec![0.5; 2000], channels: 2, sample_rate: 1000 };
        // The audio starts at song time 100; Listen starts at 50; a kick at 300.
        let clip = listen_clip(&audio, 100.0, 50.0, &[(Drum::Kick, 300.0)]);
        assert_eq!((clip.channels, clip.sample_rate), (2, 1000));
        assert_eq!(clip.samples.len(), 1050 * 2);
        let left = |i: usize| clip.samples[i * 2];
        let right = |i: usize| clip.samples[i * 2 + 1];
        assert_eq!((left(49), left(50), left(1049)), (0.0, 0.5, 0.5));
        assert!((0..250).all(|i| right(i) == 0.0));
        assert!((251..300).any(|i| right(i).abs() > 0.1));
        // From past the end there is nothing to play.
        assert!(listen_clip(&audio, 100.0, 5000.0, &[]).samples.is_empty());
    }

    #[test]
    fn listen_needs_a_result_and_an_audio_output() {
        let fixture = Scratch::new("drum-detect-listen");
        let mut app = open(&fixture);
        assert_eq!(app.listen_drum_detection(0.0, &[]).unwrap_err(), "Detect drum hits first.");
        set_lanes(&mut app, Some(lane(Hitsound::Whistle)), None);
        let (_, rows) = detect(&mut app);
        assert_eq!(app.listen_drum_detection(0.0, &checked(&rows)).unwrap_err(), "Audio output is unavailable.");
        assert!(!app.drum_detection().listening);
    }

    #[test]
    fn cancel_and_reopen_leave_nothing_behind() {
        let fixture = Scratch::new("drum-detect-cancel");
        let mut app = open(&fixture);
        let version = app.document().unwrap().composer.history.version();
        app.start_drum_detection(DrumSource::Song).unwrap();
        app.cancel_drum_detection();
        assert!(matches!(app.drum_detection().state, DrumDetectState::Idle));
        for _ in 0..20 {
            app.poll();
        }
        assert!(matches!(app.drum_detection().state, DrumDetectState::Idle));
        assert_eq!(app.document().unwrap().composer.history.version(), version);
        // Reverting or opening another difficulty drops a finished result and the options too.
        change(&mut app, |o| o.sensitivity = 70);
        detect(&mut app);
        app.revert_requested = true;
        app.confirm_revert();
        assert!(matches!(app.drum_detection().state, DrumDetectState::Idle));
        wait(&mut app, |app| !app.busy);
        assert_eq!(app.drum_options().sensitivity, 50);
    }

    #[test]
    fn refused_without_a_difficulty_or_while_busy() {
        let fixture = Scratch::new("drum-detect-refused");
        assert!(Application::headless().start_drum_detection(DrumSource::Song).is_err());
        let mut app = open(&fixture);
        let gone = SampleIdentity::File("gone.wav".into());
        assert!(app.set_drum_options(DrumOptions { kick: Some(gone), ..DrumOptions::default() }).is_err());
        assert!(app.set_drum_options(DrumOptions { sensitivity: 101, ..DrumOptions::default() }).is_err());
        assert!(app.set_drum_options(DrumOptions { object_ms: 500.0, ..DrumOptions::default() }).is_err());
        assert!(app.set_drum_options(DrumOptions { grid: Some(5), ..DrumOptions::default() }).is_err());
        app.busy = true;
        assert!(app.start_drum_detection(DrumSource::Song).is_err());
        app.busy = false;
        assert!(app.apply_drum_detection(0, &[]).is_err());
    }

    #[test]
    fn kicks_and_snares_on_one_lane_are_refused() {
        let fixture = Scratch::new("drum-detect-same-lane");
        let mut app = open(&fixture);
        let same = DrumOptions { kick: Some(lane(Hitsound::Clap)), snare: Some(lane(Hitsound::Clap)), ..DrumOptions::default() };
        assert_eq!(app.set_drum_options(same).unwrap_err(), "Pick different lanes for kicks and snares.");
        assert_eq!(app.drum_options(), DrumOptions::default());
    }

    #[test]
    fn the_lag_is_measured_over_every_detected_hit_before_lanes_are_chosen() {
        // Ten snares and three kicks, all 28 ms late on a 120 BPM 1/4 grid. Only kicks have a lane.
        let late = |drum, i: usize, from: f64| classic::Onset { drum, frame: i, time_ms: from + i as f64 * 125.0 + 28.0, score: 1.0 };
        let onsets: Vec<_> = (0..10).map(|i| late(Drum::Snare, i, 1000.0)).chain((0..3).map(|i| late(Drum::Kick, i, 2500.0))).collect();
        let grid = |time: f64, divisor: u32| {
            let step = 500.0 / f64::from(divisor);
            ((time / step).round_ties_even() * step, step)
        };
        let (hits, shift) = batch_hits(&onsets, 0.0, &(Some(lane(Hitsound::Whistle)), None), &grid, 4);
        assert!((shift - 28.0).abs() < 1e-9);
        assert_eq!(hits.iter().map(|(_, h)| h.grid).collect::<Vec<_>>(), vec![Some(2500.0), Some(2625.0), Some(2750.0)]);
        assert_eq!(hits.iter().map(|(onset, _)| *onset).collect::<Vec<_>>(), vec![10, 11, 12]);
    }

    #[test]
    fn a_hidden_target_lane_is_named_in_the_review_and_the_status() {
        let fixture = Scratch::new("drum-detect-hidden");
        let mut app = open(&fixture);
        app.document.as_mut().unwrap().composer.history.update_external(|h| h.set_lane_hidden(&lane(Hitsound::Whistle), true));
        let name = crate::lane_name(&lane(Hitsound::Whistle));
        let view = app.drum_detection();
        assert!(view.lanes.iter().any(|(l, _, hidden)| *l == lane(Hitsound::Whistle) && *hidden));
        assert!(view.lanes.iter().any(|(l, _, hidden)| *l == lane(Hitsound::Clap) && !*hidden));
        set_lanes(&mut app, Some(lane(Hitsound::Whistle)), Some(lane(Hitsound::Clap)));
        let (review, rows) = detect(&mut app);
        let DrumDetectState::Ready { unhides, .. } = app.drum_detection().state else { unreachable!() };
        assert_eq!(unhides, vec![name.clone()]);
        app.apply_drum_detection(review, &checked(&rows)).unwrap();
        assert!(app.status.ends_with(&format!("Shows hidden lane: {name}.")), "{}", app.status);
    }

    #[test]
    fn a_newly_loaded_song_drops_the_result() {
        let fixture = Scratch::new("drum-detect-new-song");
        let mut app = open(&fixture);
        detect(&mut app);
        app.drum.song_loaded();
        app.poll();
        assert!(matches!(app.drum_detection().state, DrumDetectState::Idle));
    }

    #[test]
    fn file_and_numbered_lanes_round_trip_in_the_project() {
        let fixture = Scratch::new("drum-detect-lane-keys");
        let path = map_with(&fixture, "256,192,250,1,2,0:0:2:0:\n256,192,750,1,0,0:0:0:0:kick.wav\n");
        let mut app = open_map(&fixture, path.clone());
        let file = SampleIdentity::File("kick.wav".into());
        let numbered = SampleIdentity::Standard { set: SampleSet::Soft, sound: Sound::Hit(Hitsound::Whistle), index: dawsu_foundation::CustomIndex::new(2) };
        let hits = app.document().unwrap().composer.hitsounds();
        assert!(hits.lane(&file).is_some() && hits.lane(&numbered).is_some());
        set_lanes(&mut app, Some(file.clone()), Some(numbered.clone()));
        let (review, rows) = detect(&mut app);
        app.apply_drum_detection(review, &checked(&rows)).unwrap();
        app.save_session().unwrap();
        let options = open_map(&fixture, path).drum_options();
        assert_eq!((options.kick, options.snare), (Some(file), Some(numbered)));
    }

    #[test]
    fn slider_body_lanes_are_not_offered_or_accepted() {
        let fixture = Scratch::new("drum-detect-slider-lanes");
        let path = fixture.path("slider.osu");
        std::fs::write(&path, "osu file format v14
[General]
Mode:0
SampleSet:Soft
[Difficulty]
SliderMultiplier:1.4
SliderTickRate:1
[TimingPoints]
0,500,4,2,0,100,1,0
[HitObjects]
256,192,0,1,10,0:0:0:0:
256,192,500,2,0,L|356:192,1,100
").unwrap();
        let mut app = Application::headless();
        app.request_open(path);
        wait(&mut app, |app| !app.busy);
        app.song = Some(Arc::new(pattern()));
        let slide = SampleIdentity::Standard {
            set: SampleSet::Soft, sound: Sound::Slider(dawsu_foundation::SliderSound::Slide), index: dawsu_foundation::CustomIndex::new(0),
        };
        assert!(app.document().unwrap().composer.hitsounds().lane(&slide).is_some());
        let view = app.drum_detection();
        assert!(!view.lanes.is_empty());
        assert!(view.lanes.iter().all(|(lane, _, _)| !matches!(lane, SampleIdentity::Standard { sound: Sound::Slider(_), .. })));
        assert!(app.set_drum_options(DrumOptions { kick: Some(slide), ..DrumOptions::default() }).is_err());
        // Kicks near the slider's head and end name those ends.
        set_lanes(&mut app, Some(lane(Hitsound::Whistle)), None);
        change(&mut app, |o| o.object_ms = 100.0);
        let (_, rows) = detect(&mut app);
        let kinds: Vec<RowLands> = rows.iter().filter(|row| row.drum == Drum::Kick).map(|row| row.lands).collect();
        assert!(kinds.contains(&RowLands::Object(ObjectKind::SliderHead)), "{kinds:?}");
    }

    #[test]
    fn the_lane_choice_is_saved_in_the_project_and_old_projects_still_load() {
        let fixture = Scratch::new("drum-detect-lanes");
        let mut app = open(&fixture);
        assert_eq!((app.drum_options().kick, app.drum_options().snare), (None, None));
        set_lanes(&mut app, Some(lane(Hitsound::Whistle)), Some(lane(Hitsound::Clap)));
        let (review, rows) = detect(&mut app);
        app.apply_drum_detection(review, &checked(&rows)).unwrap();
        app.save_session().unwrap();
        let reopened = open(&fixture);
        let options = reopened.drum_options();
        assert_eq!((options.kick, options.snare), (Some(lane(Hitsound::Whistle)), Some(lane(Hitsound::Clap))));
        // A project written before the field existed loads without it, and none is written until used.
        let old: project::SessionSettings = serde_json::from_str(r#"{"song_offset":0.0,"view_start":0.0}"#).unwrap();
        assert_eq!(old.drum_lanes, None);
        assert!(!serde_json::to_string(&old).unwrap().contains("drum_lanes"));
    }
}
