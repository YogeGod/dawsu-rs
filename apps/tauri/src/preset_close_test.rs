use super::{Action, Request, ask, editor_thread};
use std::{env, fs, process::Command};

#[test]
fn closing_before_a_preset_choice_preserves_onboarding() {
    const CHILD_PROFILE: &str = "DAWSU_PRESET_CLOSE_TEST_PROFILE";
    if let Some(profile) = env::var_os(CHILD_PROFILE) {
        assert_eq!(env::var_os("LOCALAPPDATA").as_ref(), Some(&profile));
        let preferences = dawsu_application::data_folder().join("preferences.json");
        let profile = std::path::PathBuf::from(profile);
        let initial = profile.join("launch.osu");
        fs::write(&initial, b"osu file format v14\n\n[Metadata]\nTitle:Launch test\n\n[HitObjects]\n256,192,0,1,0,0:0:0:0:\n").unwrap();
        let first = editor_thread(Some(initial.clone()));
        let startup = ask(&first, |tx| Request::View(false, None, false, tx)).unwrap();
        assert_eq!(startup["firstLaunch"], true);
        assert_eq!(startup["busy"], false);
        assert_eq!(startup["hasDocument"], false);
        ask(&first, |tx| Request::Close(false, tx))
            .unwrap()
            .unwrap();
        assert!(!preferences.exists());
        drop(first);

        let second = editor_thread(Some(initial));
        let chosen = ask(&second, |tx| {
            Request::View(
                false,
                Some(Action::KeyPreset {
                    name: "osu!".into(),
                }),
                false,
                tx,
            )
        })
        .unwrap();
        assert_eq!(chosen["firstLaunch"], false);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let view = ask(&second, |tx| Request::View(false, None, false, tx)).unwrap();
            if view["hasDocument"] == true {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "{view}");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        ask(&second, |tx| Request::Close(false, tx))
            .unwrap()
            .unwrap();
        let songs = profile.join("Songs");
        fs::create_dir(&songs).unwrap();
        ask(&second, |tx| Request::SetSongsDirectory(songs.clone(), tx))
            .unwrap()
            .unwrap();
        let mut persisted: serde_json::Value =
            serde_json::from_slice(&fs::read(&preferences).unwrap()).unwrap();
        persisted["master_gain"] = serde_json::json!(0.05);
        fs::write(&preferences, serde_json::to_vec(&persisted).unwrap()).unwrap();
        let reopened = editor_thread(None);
        let view = ask(&reopened, |tx| Request::View(false, None, false, tx)).unwrap();
        assert!((view["main"].as_f64().unwrap() - 0.2).abs() < 1e-6);
        assert_eq!(ask(&reopened, Request::SongsDirectory).unwrap(), Some(songs));

        let settings: serde_json::Value =
            serde_json::from_slice(&fs::read(preferences).unwrap()).unwrap();
        assert_eq!(settings["key_preset"], "osu!");
        return;
    }

    // Isolate app-data writes without changing the environment of parallel tests.
    let profile = env::temp_dir().join(format!("dawsu-tauri-preset-{}", std::process::id()));
    fs::create_dir(&profile).unwrap();
    let output = Command::new(env::current_exe().unwrap())
        .args([
            "--exact",
            "preset_close_test::closing_before_a_preset_choice_preserves_onboarding",
        ])
        .env(CHILD_PROFILE, &profile)
        .env("LOCALAPPDATA", &profile)
        .output()
        .unwrap();
    fs::remove_dir_all(profile).unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
}
