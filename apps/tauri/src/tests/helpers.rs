use crate::Action;
use dawsu_application::Application;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub(super) struct Fixture(pub(super) PathBuf);

impl Fixture {
    pub(super) fn new(label: &str) -> Self {
        let folder =
            std::env::temp_dir().join(format!("dawsu-tauri-tests-{label}-{}", std::process::id()));
        fs::create_dir(&folder).unwrap();
        Self(folder)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub(super) fn wait_for_job(app: &mut Application) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while app.is_busy() {
        app.poll();
        assert!(Instant::now() < deadline, "editor job did not finish");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(app.error().is_none(), "{:?}", app.error());
}

/// A default sounds folder, outside any mapset folder, with the skin's twelve hitsounds, all
/// `wave`: a hit whose sound isn't in the mapset folder then plays the skin's sound, as on a
/// computer with default sounds.
pub(super) fn skin_sounds(label: &str, wave: &[u8]) -> Fixture {
    let folder = Fixture::new(label);
    for bank in ["normal", "soft", "drum"] {
        for sound in ["hitnormal", "hitwhistle", "hitfinish", "hitclap"] {
            fs::write(folder.0.join(format!("{bank}-{sound}.wav")), wave).unwrap();
        }
    }
    folder
}

pub(super) fn open_raw(app: &mut Application, path: &Path) {
    Action::Open { path: path.into() }.run(app);
    wait_for_job(app);
    let deadline = Instant::now() + Duration::from_secs(10);
    while app.is_song_loading() {
        app.poll();
        assert!(Instant::now() < deadline, "song decode job did not finish");
        std::thread::sleep(Duration::from_millis(5));
    }
}

pub(super) fn open(app: &mut Application, path: &Path) {
    open_raw(app, path);
    // Pre-existing editor tests expect Grid snap and all lanes visible on fresh open.
    app.reset_fresh_open_for_tests();
}

pub(super) fn isolated_save_fixture(label: &str, test: &str) -> Option<Fixture> {
    const PROFILE: &str = "DAWSU_TAURI_SAVE_TEST_PROFILE";
    let root = Fixture::new(label);
    if let Some(profile) = std::env::var_os(PROFILE) {
        assert_eq!(std::env::var_os("LOCALAPPDATA"), Some(profile));
        return Some(root);
    }
    // Give the test a private backup folder without changing other tests' environment.
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", test])
        .env(PROFILE, &root.0)
        .env("LOCALAPPDATA", &root.0)
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stdout));
    None
}

pub(super) fn finish(index: u32) -> dawsu_foundation::SampleIdentity {
    dawsu_foundation::SampleIdentity::Standard {
        set: dawsu_foundation::SampleSet::Normal,
        sound: dawsu_foundation::Sound::Hit(dawsu_foundation::Hitsound::Finish),
        index: dawsu_foundation::CustomIndex::new(index),
    }
}
