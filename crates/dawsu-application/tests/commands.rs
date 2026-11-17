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
