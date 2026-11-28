use crate::*;
use dawsu_test_support::Scratch;
use std::time::{Duration, Instant};

mod async_song_decode;
mod autosave;
mod drum_vst_routing;
mod greenline_cleanup;
mod live_reload_and_paint;
mod project_workflow_a;
mod project_workflow_b;
mod samples_and_stems;

fn open(app: &mut Application, path: PathBuf) {
    app.request_open(path);
    let deadline = Instant::now() + Duration::from_secs(10);
    while app.busy {
        app.poll();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(app.error.is_none(), "{:?}", app.error);
}

const SLIDER_MAP: &[u8] = b"osu file format v14\r\n\r\n[General]\r\nSampleSet: Normal\r\n\r\n[Difficulty]\r\nSliderMultiplier:1.4\r\nSliderTickRate:1\r\n\r\n[TimingPoints]\r\n0,500,4,1,0,70,1,0\r\n\r\n[HitObjects]\r\n256,192,1000,1,0,0:0:0:0:\r\n100,100,2000,2,0,L|200:100,1,140\r\n";

/// [`SLIDER_MAP`] with a green line 3 ms after the slider's tail, giving the same values as the red
/// line. It sits inside the 6 ms a green line for the tail would need (ADR 0042 point 4), so Save
/// cannot give the tail an index or volume of its own and leaves such a tail out, in HS mode too.
const LEFT_OUT_MAP: &[u8] = b"osu file format v14\r\n\r\n[General]\r\nSampleSet: Normal\r\n\r\n[Difficulty]\r\nSliderMultiplier:1.4\r\nSliderTickRate:1\r\n\r\n[TimingPoints]\r\n0,500,4,1,0,70,1,0\r\n2503,-100,4,1,0,70,0,0\r\n\r\n[HitObjects]\r\n256,192,1000,1,0,0:0:0:0:\r\n100,100,2000,2,0,L|200:100,1,140\r\n";

/// [`SLIDER_MAP`] with a green line 3 ms after the slider's start, giving the same values as the red
/// line. A slider body volume needs a green line from the start to just after the body's lookup
/// (ADR 0042 point 6); this one sits inside it, so Save leaves the body and its head out.
const LEFT_OUT_HEAD_MAP: &[u8] = b"osu file format v14\r\n\r\n[General]\r\nSampleSet: Normal\r\n\r\n[Difficulty]\r\nSliderMultiplier:1.4\r\nSliderTickRate:1\r\n\r\n[TimingPoints]\r\n0,500,4,1,0,70,1,0\r\n2003,-100,4,1,0,70,0,0\r\n\r\n[HitObjects]\r\n256,192,1000,1,0,0:0:0:0:\r\n100,100,2000,2,0,L|200:100,1,140\r\n";

fn wav(path: &Path, frame: i16) {
    let mut wav = b"RIFF\x26\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data\x02\0\0\0".to_vec();
    wav.extend(frame.to_le_bytes());
    std::fs::write(path, wav).unwrap();
}

fn skin(set: SampleSet, sound: Hitsound) -> SampleIdentity {
    SampleIdentity::Standard { set, sound: Sound::Hit(sound), index: dawsu_foundation::CustomIndex::new(0) }
}

fn wait(app: &mut Application) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while app.busy {
        app.poll();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Saves like Ctrl+S, confirming the Save hitsounds summary when Save leaves moments out.
fn save_through_summary(app: &mut Application, backups: PathBuf) {
    app.save_with_backups(backups);
    wait(app);
    if app.save_problem_review() {
        app.answer_save_review(true);
        wait(app);
    }
    assert!(app.error.is_none(), "{:?}", app.error);
}

fn tail_id(hits: &Hitsounds) -> dawsu_hitsounding::MomentId {
    dawsu_hitsounding::MomentId(hits.moments().iter().position(|m| matches!(m.place(), dawsu_hitsounding::Place::SliderNode { node: 1, .. })).unwrap())
}

fn open_left_out_fixture(scratch: &Scratch, project: bool) -> (Application, PathBuf) {
    let folder = scratch.path("mapset");
    std::fs::create_dir_all(&folder).unwrap();
    let map = folder.join("a.osu");
    std::fs::write(&map, LEFT_OUT_MAP).unwrap();
    let mut app = Application::headless();
    if project {
        app.project_root = Some(scratch.path("projects"));
    }
    open(&mut app, map.clone());
    app.set_hs_mode(true);
    app.document.as_mut().unwrap().composer.apply("Draw claps", |hits| {
        let tail = tail_id(hits);
        hits.add_hit(tail, skin(SampleSet::Soft, Hitsound::Clap));
        hits.add_hit(tail, skin(SampleSet::Drum, Hitsound::Clap));
        hits.add_hit(dawsu_hitsounding::MomentId(0), skin(SampleSet::Normal, Hitsound::Finish));
    });
    (app, map)
}

fn line(bytes: &[u8], start: &str) -> String {
    String::from_utf8_lossy(bytes).lines().find(|line| line.starts_with(start)).unwrap().to_string()
}

/// The project as it is on disk now.
fn stored(app: &Application) -> project::SessionSettings {
    let path = &app.document().unwrap().path;
    project::Project::load(app.project_root.as_ref().unwrap(), None, path).unwrap().settings
}

fn reopen(app: &Application, map: &Path) -> Application {
    let mut reopened = Application::headless();
    reopened.project_root = app.project_root.clone();
    open(&mut reopened, map.to_path_buf());
    reopened
}

#[test]
fn asio_audio_output_without_a_driver_is_refused_and_keeps_the_old_engine() {
    let mut app = Application::headless();
    let before = app.preferences.clone();
    for device in [None, Some(String::new()), Some("   ".to_string())] {
        let result = app.set_output_settings(OutputBackend::Asio, device.clone(), None);
        assert_eq!(result, Err("Choose an ASIO driver first.".to_string()), "device {device:?}");
        assert!(app.audio.is_none(), "the old engine stays in place for {device:?}");
        assert_eq!(app.preferences.audio_backend, before.audio_backend);
        assert_eq!(app.preferences.audio_device_id, before.audio_device_id);
    }
}
