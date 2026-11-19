use dawsu_application::Application;
use dawsu_editor::keys::{Key, KeyChord, Mods};

#[test]
fn keyboard_and_help_share_the_catalogue_and_missing_documents_disable_edits() {
    let mut app = Application::headless();
    assert!(
        app.help()
            .iter()
            .any(|g| g.entries.iter().any(|e| e.description == "Undo"))
    );
    app.key(KeyChord::new(Key::O, Mods::CTRL));
    assert_eq!(app.requests.len(), 1);
    app.command("undo");
    assert!(!app.is_dirty());
}

#[test]
fn help_lists_overview_wheel_and_lane_navigation_gestures() {
    let app = Application::headless();
    let help = app.help();
    for (group, terms) in [
        ("Timeline", ["overview", "wheel without modifiers", "sideways wheel"].as_slice()),
        ("Lanes", ["sample-set folder header", "collapsed hit preview"].as_slice()),
    ] {
        for term in terms {
            assert!(
                help.iter().any(|section| section.title.contains(group)
                    && section.entries.iter().any(|entry| entry.keys.to_lowercase().contains(term))),
                "missing {group} gesture: {term}"
            );
        }
    }
}

#[test]
fn legacy_map_and_filter_shortcuts_request_existing_editor_controls() {
    let scratch = dawsu_test_support::Scratch::new("legacy-window-keys");
    let map = scratch.path("Test.osu");
    std::fs::write(&map, include_bytes!("project-fixture.osu")).unwrap();
    let mut app = Application::headless();
    app.request_open(map);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while app.busy {
        app.poll();
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(app.document().is_some());
    for (key, mods, request) in [
        (Key::F, Mods::CTRL, "filter"),
        (Key::N, Mods::CTRL, "newDiff"),
        (Key::O, Mods::CTRL | Mods::SHIFT, "openFolder"),
    ] {
        app.key(KeyChord::new(key, mods));
        assert_eq!(app.requested_edit(), Some(request));
        app.cancel_timeline_edit();
    }
}

#[test]
fn imported_legacy_archive_library_and_export_keys_route_to_their_actions() {
    let scratch = dawsu_test_support::Scratch::new("legacy-output-keys");
    let keys = scratch.path("Legacy.dawsukeys");
    std::fs::write(
        &keys,
        br#"{"version":1,"keys":{"importOsz":["None+F6"],"openHitsoundFolder":["None+F7"],"reviewOutput":["None+F8"]}}"#,
    ).unwrap();
    let mut app = Application::headless();
    app.import_shortcuts(&keys).unwrap();
    for (key, request) in [(Key::F6, "importOsz"), (Key::F7, "openHitsoundFolder")] {
        app.key(KeyChord::new(key, Mods::NONE));
        assert_eq!(app.requested_edit(), Some(request));
        app.cancel_timeline_edit();
    }
    app.key(KeyChord::new(Key::F8, Mods::NONE));
    assert!(app.requested_edit().is_none(), "export settings need an open difficulty");

    let map = scratch.path("Test.osu");
    std::fs::write(&map, include_bytes!("project-fixture.osu")).unwrap();
    app.request_open(map);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while app.busy {
        app.poll();
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(app.document().is_some());
    app.key(KeyChord::new(Key::F8, Mods::NONE));
    assert_eq!(app.requested_edit(), Some("reviewOutput"));
}
