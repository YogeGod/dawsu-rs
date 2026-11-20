//! Drum hit detection (F027), first slice: the whole song, the map's own song, kick and snare
//! lanes picked by the mapper, Apply as one undo step.
//!
//! The analysis runs on its own thread with its own cancel flag, result channel and progress, so
//! it never sets `busy` and never blocks the editor. Its curves are kept, keyed by the song they
//! came from (the analysis stamp); edits only re-plan. Apply checks a review stamp (history,
//! tempo, grid, lanes, offset) against the one the mapper saw, and refuses on any difference.

use crate::{Application, project};
use dawsu_audio::AudioClip;
use dawsu_editor_composer::{BatchCounts, BatchHit, BatchOptions, BatchRefusal, Policy};
use dawsu_foundation::SampleIdentity;
use dawsu_graph_audio::classic::{self, Curves, Drum};
use serde::{Deserialize, Serialize};
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};

/// 3.1.4's middle sensitivity (scale 1.5 of 2.0 strict to 1.0 loose).
const SCALE: f64 = 1.5;
const LABEL: &str = "Drum hit detection";

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

/// Which audio the curves came from.
#[derive(Clone, Copy, PartialEq, Eq)]
struct AnalysisStamp {
    song: usize,
    samples: usize,
}

impl AnalysisStamp {
    fn of(song: &Arc<AudioClip>) -> Self {
        Self { song: Arc::as_ptr(song) as usize, samples: song.samples.len() }
    }
}

struct Job {
    result: mpsc::Receiver<Result<Curves, classic::Cancelled>>,
    cancel: Arc<AtomicBool>,
    progress: Arc<Mutex<(usize, usize)>>,
    stamp: AnalysisStamp,
    lanes: (Option<SampleIdentity>, Option<SampleIdentity>),
}

struct Ready {
    curves: Arc<Curves>,
    stamp: AnalysisStamp,
    lanes: (Option<SampleIdentity>, Option<SampleIdentity>),
    /// The review stamp the counts below were planned for.
    review: u64,
    counts: BatchCounts,
    kicks: usize,
    snares: usize,
}

#[derive(Default)]
pub(crate) struct DrumDetect {
    job: Option<Job>,
    ready: Option<Ready>,
}

impl DrumDetect {
    /// Stops a running analysis and forgets the result. Nothing is left behind.
    pub(crate) fn clear(&mut self) {
        if let Some(job) = self.job.take() {
            job.cancel.store(true, Ordering::Relaxed);
        }
        self.ready = None;
    }
}

/// What the editor shows about drum hit detection.
#[derive(Debug, Clone, PartialEq)]
pub enum DrumDetectState {
    Idle,
    Running { done: usize, total: usize },
    Ready { review: u64, counts: BatchCounts, kicks: usize, snares: usize },
}

#[derive(Debug, Clone, PartialEq)]
pub struct DrumDetectView {
    pub state: DrumDetectState,
    /// Lanes that can take the hits, with their names.
    pub lanes: Vec<(SampleIdentity, String)>,
    /// The lanes last used for this difficulty.
    pub kick: Option<SampleIdentity>,
    pub snare: Option<SampleIdentity>,
}

impl Application {
    /// Starts analysing the song for kick and snare hits. Refused without a difficulty, its song,
    /// or a lane to put hits on; a run already going is replaced.
    pub fn start_drum_detection(&mut self, kick: Option<SampleIdentity>, snare: Option<SampleIdentity>) -> Result<(), String> {
        if self.busy || self.is_stroke_open() {
            return Err("Wait for the current job before detecting drum hits.".into());
        }
        let document = self.document.as_ref().ok_or("Open a difficulty first.")?;
        if kick.is_none() && snare.is_none() {
            return Err("Pick kick and snare lanes first.".into());
        }
        if [&kick, &snare].into_iter().flatten()
            .any(|lane| slider_body(lane) || document.composer.hitsounds().lane(lane).is_none()) {
            return Err("A chosen lane no longer exists. Pick the lanes again.".into());
        }
        let song = self.song.clone().ok_or("This difficulty's song could not be loaded.")?;
        self.drum.clear();
        let cancel = Arc::new(AtomicBool::new(false));
        let progress = Arc::new(Mutex::new((0, 1)));
        let (send, result) = mpsc::channel();
        let stamp = AnalysisStamp::of(&song);
        {
            let (cancel, progress) = (cancel.clone(), progress.clone());
            std::thread::spawn(move || {
                let mono = mono_mix(&song);
                let stop = || cancel.load(Ordering::Relaxed);
                let curves = classic::analyse(&mono, song.sample_rate, &stop, &mut |done, total| {
                    if let Ok(mut current) = progress.lock() {
                        *current = (done, total);
                    }
                });
                let _ = send.send(curves);
            });
        }
        self.drum.job = Some(Job { result, cancel, progress, stamp, lanes: (kick, snare) });
        self.status = "Detecting drum hits…".into();
        Ok(())
    }

    pub fn cancel_drum_detection(&mut self) {
        if self.drum.job.is_some() || self.drum.ready.is_some() {
            self.drum.clear();
            self.status = "Drum hit detection cancelled. No hits were added.".into();
        }
    }

    /// Called from `poll`: collects a finished analysis and re-plans after edits.
    pub(crate) fn poll_drum_detection(&mut self) {
        let current = self.song.as_ref().map(AnalysisStamp::of);
        if let Some(job) = &self.drum.job {
            if current != Some(job.stamp) {
                self.drum.clear();
                return;
            }
            let done = match job.result.try_recv() {
                Ok(Ok(curves)) => Some(Some(curves)),
                Ok(Err(classic::Cancelled)) | Err(mpsc::TryRecvError::Disconnected) => Some(None),
                Err(mpsc::TryRecvError::Empty) => None,
            };
            match done {
                Some(Some(curves)) => {
                    let job = self.drum.job.take().expect("checked");
                    self.drum.ready = Some(Ready {
                        curves: Arc::new(curves), stamp: job.stamp, lanes: job.lanes,
                        review: u64::MAX, counts: BatchCounts::default(), kicks: 0, snares: 0,
                    });
                }
                Some(None) => {
                    self.drum.job = None;
                }
                None => {
                    if let Ok((done, total)) = job.progress.lock().map(|p| *p) {
                        self.status = format!("Detecting drum hits… {}%", done * 100 / total.max(1));
                    }
                }
            }
        }
        if let Some(ready) = &self.drum.ready {
            if current != Some(ready.stamp) || self.document.is_none() {
                self.drum.clear();
                return;
            }
            let review = self.drum_review_stamp(&ready.lanes);
            if review != ready.review {
                let first = ready.review == u64::MAX;
                let (hits, options) = self.drum_plan(&ready.curves, &ready.lanes);
                let counts = self.document.as_ref().expect("checked").composer.plan_batch(&hits, &options).counts;
                let kicks = hits.iter().filter(|h| Some(&h.lane) == ready.lanes.0.as_ref()).count();
                let ready = self.drum.ready.as_mut().expect("checked");
                ready.review = review;
                ready.counts = counts;
                ready.kicks = kicks;
                ready.snares = hits.len() - kicks;
                if first {
                    self.status = format!("Found {} kick and {} snare hits. Review and apply.", ready.kicks, ready.snares);
                }
            }
        }
    }

    /// Adds the reviewed hits as one undo step. `review` is the stamp the mapper saw; any edit,
    /// tempo, grid, lane or offset change since then refuses, and the review shows new counts.
    pub fn apply_drum_detection(&mut self, review: u64) -> Result<(), String> {
        if self.busy || self.is_stroke_open() {
            return Err("Wait for the current job before applying drum hits.".into());
        }
        let ready = self.drum.ready.as_ref().ok_or("Detect drum hits first.")?;
        if self.song.as_ref().map(AnalysisStamp::of) != Some(ready.stamp) {
            self.drum.clear();
            return Err("The song changed. Detect drum hits again.".into());
        }
        if self.drum_review_stamp(&ready.lanes) != review {
            return Err("The map changed since the review. Check the new counts, then apply.".into());
        }
        let (curves, lanes) = (ready.curves.clone(), ready.lanes.clone());
        let (hits, options) = self.drum_plan(&curves, &lanes);
        let document = self.document.as_mut().ok_or("Open a difficulty first.")?;
        let report = document.composer.add_batch(LABEL, &hits, &options).map_err(|refusal| match refusal {
            BatchRefusal::StepOpen => "Finish the current edit before applying drum hits.".to_owned(),
            BatchRefusal::MissingLane(_) => "A chosen lane no longer exists. Pick the lanes again.".to_owned(),
        })?;
        document.drum_lanes = Some(DrumLanes {
            kick: lanes.0.as_ref().map(project::lane_key),
            snare: lanes.1.as_ref().map(project::lane_key),
        });
        self.drum.clear();
        self.refresh_recovery_edit();
        let c = report.counts;
        let mut status = format!("Added {} drum hits: {} on objects, {} on grid lines.", c.added, c.on_objects, c.loose);
        if c.flagged > 0 {
            status.push_str(&format!(" {} too far from the grid left out.", c.flagged));
        }
        if c.duplicates > 0 {
            status.push_str(&format!(" {} already there.", c.duplicates));
        }
        self.status = status;
        Ok(())
    }

    pub fn drum_detection(&self) -> DrumDetectView {
        let state = match (&self.drum.job, &self.drum.ready) {
            (Some(job), _) => {
                let (done, total) = job.progress.lock().map(|p| *p).unwrap_or((0, 1));
                DrumDetectState::Running { done, total }
            }
            (None, Some(ready)) if ready.review != u64::MAX => DrumDetectState::Ready {
                review: ready.review, counts: ready.counts, kicks: ready.kicks, snares: ready.snares,
            },
            _ => DrumDetectState::Idle,
        };
        let document = self.document.as_ref();
        let lanes = document.map(|d| d.composer.hitsounds().lanes().iter()
            .filter(|lane| !slider_body(&lane.identity))
            .map(|lane| (lane.identity.clone(), crate::lane_name(&lane.identity))).collect()).unwrap_or_default();
        let saved = |key: Option<&String>| key.and_then(|key| project::lane_identity(key))
            .filter(|lane| !slider_body(lane))
            .filter(|lane| document.is_some_and(|d| d.composer.hitsounds().lane(lane).is_some()));
        let mapping = document.and_then(|d| d.drum_lanes.as_ref());
        DrumDetectView {
            state,
            lanes,
            kick: saved(mapping.and_then(|m| m.kick.as_ref())),
            snare: saved(mapping.and_then(|m| m.snare.as_ref())),
        }
    }

    /// The detected hits as batch hits in song time with their grid lines, and the batch options
    /// with the measured lag. The grid is the editor's snap division, also when snap is off.
    fn drum_plan(&self, curves: &Curves, lanes: &(Option<SampleIdentity>, Option<SampleIdentity>)) -> (Vec<BatchHit>, BatchOptions) {
        let mut options = BatchOptions { policy: Policy::Snap, ..BatchOptions::default() };
        let Some(document) = self.document.as_ref() else { return (Vec::new(), options) };
        let composer = &document.composer;
        let divisor = composer.spacing.divisions();
        let grid = |time: f64, divisor: u32| composer.grid_line(time, divisor);
        let onsets: Vec<_> = classic::pick(curves, SCALE).into_iter().filter_map(|onset| {
            let lane = match onset.drum { Drum::Kick => lanes.0.clone(), Drum::Snare => lanes.1.clone() }?;
            Some((lane, onset.time_ms + self.song_offset, onset.score))
        }).collect();
        let shift = classic::grid_shift(onsets.iter().map(|o| o.1), &grid, divisor);
        let hits = onsets.into_iter().map(|(lane, time, strength)| BatchHit {
            time, lane, strength, grid: classic::on_grid(time, &grid, &[divisor], shift),
        }).collect();
        options.shift_ms = shift;
        (hits, options)
    }

    fn drum_review_stamp(&self, lanes: &(Option<SampleIdentity>, Option<SampleIdentity>)) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        if let Some(document) = &self.document {
            let composer = &document.composer;
            document.path.hash(&mut hasher);
            composer.history.version().hash(&mut hasher);
            for section in composer.tempo.sections() {
                (section.start.ms().to_bits(), section.beat_length_ms.to_bits()).hash(&mut hasher);
            }
            (composer.spacing.beats(), composer.spacing.divisions(), composer.insert_volume).hash(&mut hasher);
        }
        lanes.hash(&mut hasher);
        self.song_offset.to_bits().hash(&mut hasher);
        // u64::MAX marks "not planned yet".
        hasher.finish().min(u64::MAX - 1)
    }
}

/// Slider body lanes play along a slider, not at a moment; drum hits never go there.
fn slider_body(lane: &SampleIdentity) -> bool {
    matches!(lane, SampleIdentity::Standard { sound: dawsu_foundation::Sound::Slider(_), .. })
}

/// The song as mono, the channels averaged (3.1.4 used (left + right) / 2).
fn mono_mix(clip: &AudioClip) -> Vec<f32> {
    let channels = clip.channels.max(1);
    if channels == 1 {
        return clip.samples.clone();
    }
    if channels == 2 {
        return clip.samples.chunks_exact(2).map(|f| (f[0] + f[1]) * 0.5).collect();
    }
    clip.samples.chunks_exact(channels).map(|f| f.iter().sum::<f32>() / channels as f32).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawsu_foundation::{Hitsound, SampleSet, Sound};
    use dawsu_test_support::Scratch;
    use std::path::PathBuf;
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

    /// 120 BPM; objects on every kick (500 + 1000k ms); one whistle+clap circle at 0 so both lanes exist.
    fn map(fixture: &Scratch) -> PathBuf {
        let mut body = String::from("osu file format v14\n[General]\nMode:0\nSampleSet:Soft\n[TimingPoints]\n0,500,4,2,0,100,1,0\n[HitObjects]\n256,192,0,1,10,0:0:0:0:\n");
        for k in 0..16 {
            body.push_str(&format!("256,192,{},1,0,0:0:0:0:\n", 500 + 1000 * k));
        }
        let path = fixture.path("map.osu");
        std::fs::write(&path, body).unwrap();
        path
    }

    fn open(fixture: &Scratch) -> Application {
        let mut app = Application::headless();
        app.project_root = Some(fixture.path("projects"));
        app.request_open(map(fixture));
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

    fn ready(app: &Application) -> Option<u64> {
        match app.drum_detection().state {
            DrumDetectState::Ready { review, .. } => Some(review),
            _ => None,
        }
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
    fn detection_runs_beside_the_editor_and_apply_is_one_undo_step() {
        let fixture = Scratch::new("drum-detect-apply");
        let mut app = open(&fixture);
        app.start_drum_detection(Some(lane(Hitsound::Whistle)), Some(lane(Hitsound::Clap))).unwrap();
        assert!(!app.busy);
        wait(&mut app, |app| ready(app).is_some());
        let DrumDetectState::Ready { review, counts, kicks, snares } = app.drum_detection().state else { unreachable!() };
        assert_eq!((kicks, snares), (16, 16));
        // Kicks land on the objects; snares between them land on their 1/4 grid lines.
        assert_eq!((counts.added, counts.on_objects, counts.loose, counts.flagged), (32, 16, 16, 0));
        app.apply_drum_detection(review).unwrap();
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
    fn an_edit_after_the_review_refuses_apply_and_re_plans() {
        let fixture = Scratch::new("drum-detect-stale");
        let mut app = open(&fixture);
        app.start_drum_detection(Some(lane(Hitsound::Whistle)), None).unwrap();
        wait(&mut app, |app| ready(app).is_some());
        let review = ready(&app).unwrap();
        app.document.as_mut().unwrap().composer.add(500.0, lane(Hitsound::Whistle), 100, 5.0);
        assert!(app.apply_drum_detection(review).unwrap_err().contains("changed since the review"));
        app.poll();
        let DrumDetectState::Ready { review: fresh, counts, .. } = app.drum_detection().state else { panic!("re-planned") };
        assert_ne!(fresh, review);
        assert_eq!(counts.duplicates, 1);
        app.apply_drum_detection(fresh).unwrap();
        assert_eq!(undo_steps(&mut app), 2);
    }

    #[test]
    fn cancel_and_reopen_leave_nothing_behind() {
        let fixture = Scratch::new("drum-detect-cancel");
        let mut app = open(&fixture);
        let version = app.document().unwrap().composer.history.version();
        app.start_drum_detection(Some(lane(Hitsound::Whistle)), Some(lane(Hitsound::Clap))).unwrap();
        app.cancel_drum_detection();
        assert!(matches!(app.drum_detection().state, DrumDetectState::Idle));
        for _ in 0..20 {
            app.poll();
        }
        assert!(matches!(app.drum_detection().state, DrumDetectState::Idle));
        assert_eq!(app.document().unwrap().composer.history.version(), version);
        // Reverting or opening another difficulty drops a finished result too.
        app.start_drum_detection(Some(lane(Hitsound::Whistle)), None).unwrap();
        wait(&mut app, |app| ready(app).is_some());
        app.revert_requested = true;
        app.confirm_revert();
        assert!(matches!(app.drum_detection().state, DrumDetectState::Idle));
        wait(&mut app, |app| !app.busy);
    }

    #[test]
    fn refused_without_lanes_a_song_or_while_busy() {
        let fixture = Scratch::new("drum-detect-refused");
        let mut app = open(&fixture);
        assert_eq!(app.start_drum_detection(None, None).unwrap_err(), "Pick kick and snare lanes first.");
        let gone = SampleIdentity::File("gone.wav".into());
        assert!(app.start_drum_detection(Some(gone), None).is_err());
        app.busy = true;
        assert!(app.start_drum_detection(Some(lane(Hitsound::Whistle)), None).is_err());
        app.busy = false;
        app.song = None;
        assert!(app.start_drum_detection(Some(lane(Hitsound::Whistle)), None).is_err());
        assert!(app.apply_drum_detection(0).is_err());
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
256,192,1000,2,0,L|356:192,1,100
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
        assert!(view.lanes.iter().all(|(lane, _)| !matches!(lane, SampleIdentity::Standard { sound: Sound::Slider(_), .. })));
        assert!(app.start_drum_detection(Some(slide), None).is_err());
    }

    #[test]
    fn the_lane_choice_is_saved_in_the_project_and_old_projects_still_load() {
        let fixture = Scratch::new("drum-detect-lanes");
        let mut app = open(&fixture);
        assert_eq!((app.drum_detection().kick, app.drum_detection().snare), (None, None));
        app.start_drum_detection(Some(lane(Hitsound::Whistle)), Some(lane(Hitsound::Clap))).unwrap();
        wait(&mut app, |app| ready(app).is_some());
        app.apply_drum_detection(ready(&app).unwrap()).unwrap();
        app.save_session().unwrap();
        let reopened = open(&fixture);
        let view = reopened.drum_detection();
        assert_eq!((view.kick, view.snare), (Some(lane(Hitsound::Whistle)), Some(lane(Hitsound::Clap))));
        // A project written before the field existed loads without it, and none is written until used.
        let old: project::SessionSettings = serde_json::from_str(r#"{"song_offset":0.0,"view_start":0.0}"#).unwrap();
        assert_eq!(old.drum_lanes, None);
        assert!(!serde_json::to_string(&old).unwrap().contains("drum_lanes"));
    }
}
