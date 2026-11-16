//! The command catalogue and key resolution: Responsibilities 7-12; invariants 6-10, 11(c) and 12.

use dawsu_editor::commands::{
    CatalogProblem, CommandArea, CommandCatalog, CommandContributor, CommandDefinition,
    CommandRegistry, EditorScope, KeyMap, KeyOutcome, KeyPreset, RunOutcome, areas,
};
use dawsu_editor::editing_commands as ec;
use dawsu_editor::keys::{Key, KeyChord, KeyCombination, Mods};
use dawsu_editor::tools::ToolKind;

/// A fake application context: at most one open Hitsound document, found when a handler runs.
#[derive(Default)]
struct App {
    hitsound: Option<Vec<String>>,
    long_work_running: bool,
    snap_division: Option<u32>,
    ran: Vec<&'static str>,
}

fn has_doc(app: &App) -> bool {
    app.hitsound.is_some()
}

fn undo(app: &mut App) -> Result<(), String> {
    app.hitsound
        .as_mut()
        .ok_or("no document")?
        .push("undo".into());
    app.ran.push("undo");
    Ok(())
}

fn nothing(_: &mut App) -> Result<(), String> {
    Ok(())
}

fn snap_to(app: &mut App, n: u32) -> Result<(), String> {
    app.snap_division = Some(n);
    Ok(())
}

fn doc_open_for(app: &App, _n: u32) -> bool {
    has_doc(app)
}

/// Binds the kernel's commands under the same areas 3.1.x listed them in (`EditorCommands` and the
/// Keysound `Shared` table), in both editors.
struct KernelCommands;

impl CommandContributor<App> for KernelCommands {
    fn register(&self, r: &mut CommandRegistry<App>) {
        use areas::*;
        let edit = [
            ec::undo(),
            ec::redo(),
            ec::copy(),
            ec::cut(),
            ec::paste(),
            ec::duplicate(),
            ec::delete(),
            ec::select_all(),
            ec::deselect(),
        ];
        for d in &edit {
            let run = if d.id == "undo" { undo } else { nothing };
            r.bind(d, HS_HIT_EDITING, run, Some(has_doc));
            r.bind(d, KS_NOTE_EDITING, nothing, None);
        }
        for d in [ec::snap_toggle(), ec::zoom_in(), ec::zoom_out()] {
            r.bind(&d, HS_TIMELINE, nothing, None);
            r.bind(&d, KS_NOTE_EDITING, nothing, None);
        }
        r.bind_family(&ec::SNAP_TO, HS_TIMELINE, snap_to, Some(doc_open_for));
        r.bind_family(&ec::SNAP_TO, KS_NOTE_EDITING, snap_to, None);
        for d in [
            ec::seek_back(),
            ec::seek_forward(),
            ec::seek_back_far(),
            ec::seek_forward_far(),
            ec::cycle(),
            ec::locators_to_selection(),
            ec::play_locator_range(),
            ec::jump_left(),
            ec::jump_right(),
            ec::set_left(),
            ec::set_right(),
            ec::add_bookmark(),
            ec::remove_bookmark(),
            ec::previous_bookmark(),
            ec::next_bookmark(),
        ] {
            r.bind(&d, HS_TIMELINE, nothing, None);
            r.bind(&d, KS_PLAYBACK, nothing, None);
        }
        for kind in ToolKind::ALL {
            let area = if kind == ToolKind::Ramp {
                HS_VOLUME
            } else {
                HS_HIT_EDITING
            };
            r.bind(&kind.command(), area, nothing, None);
            r.bind(&kind.command(), KS_NOTE_EDITING, nothing, None);
            r.mouse(area, kind.mouse_actions());
        }
    }
}

fn kernel_catalog() -> CommandCatalog<App> {
    let mut r = CommandRegistry::new();
    r.contribute(&KernelCommands);
    CommandCatalog::build(r).unwrap_or_else(|p| {
        panic!(
            "{}",
            p.iter()
                .map(|x| x.to_string())
                .collect::<Vec<_>>()
                .join("\n")
        )
    })
}

fn press(key: Key, mods: Mods) -> KeyChord {
    KeyChord::new(key, mods)
}

#[test]
fn r7_a_command_is_declared_once_with_id_description_keys_osu_keys_and_cubase_command() {
    let undo = ec::undo();
    assert_eq!(undo.id, "undo");
    assert_eq!(undo.description, "Undo");
    assert_eq!(
        undo.default_keys,
        vec![KeyCombination::new(Key::Z, Mods::CTRL)]
    );
    assert_eq!(undo.cubase.as_deref(), Some("Edit|Undo"));
    let seek = ec::seek_back();
    assert!(seek.default_keys.is_empty());
    assert_eq!(
        seek.keys_for(KeyPreset::Osu),
        &[KeyCombination::plain(Key::Left)]
    );
    // Declared once even though two areas bind it.
    let catalog = kernel_catalog();
    assert_eq!(
        catalog
            .definitions()
            .iter()
            .filter(|d| d.id == "undo")
            .count(),
        1
    );
    assert_eq!(
        catalog
            .bindings()
            .iter()
            .filter(|(d, _)| d.id == "undo")
            .count(),
        2
    );
}

#[test]
fn r7_the_snap_family_expands_to_exactly_snap1_to_snap9_with_one_handler() {
    let ids: Vec<String> = ec::SNAP_TO
        .members()
        .into_iter()
        .map(|(_, d)| d.id)
        .collect();
    assert_eq!(
        ids,
        [
            "snap1", "snap2", "snap3", "snap4", "snap5", "snap6", "snap7", "snap8", "snap9"
        ]
    );
    let catalog = kernel_catalog();
    let mut app = App {
        hitsound: Some(Vec::new()),
        ..App::default()
    };
    assert_eq!(
        catalog.run(&mut app, "snap7", EditorScope::Hitsound),
        RunOutcome::Ran
    );
    assert_eq!(
        app.snap_division,
        Some(7),
        "the handler got the number, not the id"
    );
    let keys = KeyMap::from_preset(catalog.definitions(), KeyPreset::DawLike);
    catalog.handle_key(
        &mut app,
        &keys,
        press(Key::D5, Mods::SHIFT),
        EditorScope::Hitsound,
    );
    assert_eq!(app.snap_division, Some(5));
}

#[test]
fn r7_each_mouse_action_is_listed_once_as_mouse_in_its_area() {
    let catalog = kernel_catalog();
    let keys = KeyMap::from_preset(catalog.definitions(), KeyPreset::DawLike);
    let help = catalog.help(&keys);
    let mouse = help
        .iter()
        .find(|g| g.title == "Hitsound – Hit editing (mouse)")
        .expect("mouse group");
    assert!(
        mouse
            .entries
            .iter()
            .any(|e| e.keys == "Right-click in the grid" && e.description == "Pick a tool")
    );
    let hs = help
        .iter()
        .position(|g| g.title == "Hitsound – Hit editing")
        .unwrap();
    assert_eq!(
        help[hs + 1].title,
        "Hitsound – Hit editing (mouse)",
        "mouse actions right after the area's keys"
    );
}

#[test]
fn r8_help_is_built_from_the_catalogue_by_area_with_the_old_names() {
    let catalog = kernel_catalog();
    let keys = KeyMap::from_preset(catalog.definitions(), KeyPreset::DawLike);
    let help = catalog.help(&keys);
    let titles: Vec<&str> = help.iter().map(|g| g.title.as_str()).collect();
    assert_eq!(
        titles,
        [
            "Hitsound – Hit editing",
            "Hitsound – Hit editing (mouse)",
            "Hitsound – Timeline navigation",
            "Hitsound – Volume control",
            "Hitsound – Volume control (mouse)",
            "Keysound – Note editing",
            "Keysound – Playback",
        ]
    );
    let editing = &help[0];
    let redo = editing
        .entries
        .iter()
        .find(|e| e.description == "Redo")
        .unwrap();
    assert_eq!(redo.keys, "Ctrl+Y, Ctrl+Shift+Z");
    let ks = help
        .iter()
        .find(|g| g.title == "Keysound – Note editing")
        .unwrap();
    assert!(
        ks.entries.iter().any(|e| e.description == "Undo"),
        "a shared command shows under both areas without a table"
    );
}

#[test]
fn r9_a_handler_finds_the_open_document_when_it_runs_and_cannot_run_without_one() {
    let catalog = kernel_catalog();
    let mut app = App::default();
    assert_eq!(
        catalog.run(&mut app, "undo", EditorScope::Hitsound),
        RunOutcome::CannotRun
    );
    assert!(!catalog.can_run(&app, "undo", EditorScope::Hitsound));
    app.hitsound = Some(Vec::new());
    assert_eq!(
        catalog.run(&mut app, "undo", EditorScope::Hitsound),
        RunOutcome::Ran
    );
    // A new document opened later is the one the same handler changes.
    app.hitsound = Some(vec!["other".into()]);
    catalog.run(&mut app, "undo", EditorScope::Hitsound);
    assert_eq!(
        app.hitsound.as_deref(),
        Some(&["other".to_owned(), "undo".to_owned()][..])
    );
}

#[test]
fn r10_a_key_chord_carries_its_physical_position_and_commands_match_the_key() {
    let catalog = kernel_catalog();
    let keys = KeyMap::from_preset(catalog.definitions(), KeyPreset::DawLike);
    // An AZERTY "Z" sits where US has W: the command follows the key, live input can use the position.
    let azerty_z = KeyChord {
        key: Key::Z,
        physical: Key::W,
        mods: Mods::CTRL,
    };
    assert_eq!(azerty_z.physical, Key::W);
    assert_eq!(
        catalog
            .resolve(&keys, azerty_z, EditorScope::Hitsound)
            .map(|d| d.id.as_str()),
        Some("undo")
    );
}

#[test]
fn r11_every_3_1_x_id_resolves_to_a_kernel_command_or_is_left_to_its_owner() {
    let text = include_str!("fixtures/command-ids-3.1.x.txt");
    let ids: Vec<&str> = text
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .collect();
    assert_eq!(
        ids.len(),
        97,
        "88 definitions in EditorCommands, with snap1..snap9 expanded"
    );
    let catalog = kernel_catalog();
    // Declared beside their own features in later crates; the full zero-unknown check runs where
    // every contributor exists (dawsu-application).
    const ELSEWHERE: &[&str] = &[
        // dawsu-application: files, the osu! link, playback, preferences, help, live input.
        "open",
        "openFromOsu",
        "newDiff",
        "importOsz",
        "openFolder",
        "save",
        "revert",
        "openHitsoundFolder",
        "reviewOutput",
        "preferences",
        "help",
        "inspector",
        "play",
        "start",
        "stop",
        "ks.record",
        "ks.typing",
        // dawsu-editor-composer: Hitsound hit editing and lanes, Keysound note editing.
        "first",
        "last",
        "jumpToSelection",
        "selectPrevious",
        "selectNext",
        "addPrevious",
        "addNext",
        "navUp",
        "navDown",
        "navAddUp",
        "navAddDown",
        "nudgeLeft",
        "nudgeRight",
        "rowsOut",
        "rowsIn",
        "volumeUp",
        "volumeDown",
        "volumeUpFine",
        "volumeDownFine",
        "ks.startLeft",
        "ks.endRight",
        "ks.startRight",
        "ks.endLeft",
        "gridLeft",
        "gridRight",
        "laneUp",
        "laneDown",
        "quantize",
        "quantizeDialog",
        "repeat",
        "editRange",
        "scaleByVolume",
        "filter",
        "ks.toolSplit",
        "ks.toolGlue",
        "ks.sync",
        "ks.generate",
    ];
    let unknown = catalog.unknown_ids(ids.iter().copied());
    let mut left: Vec<&str> = ELSEWHERE.to_vec();
    left.sort();
    let mut unknown_sorted: Vec<&str> = unknown.iter().map(String::as_str).collect();
    unknown_sorted.sort();
    assert_eq!(
        unknown_sorted, left,
        "every 3.1.x id the kernel owns resolves; the rest are named owners'"
    );
    assert_eq!(
        ids.len() - ELSEWHERE.len(),
        catalog.definitions().len(),
        "the kernel declares nothing 3.1.x did not ship"
    );
}

#[test]
fn r11_a_renamed_command_keeps_answering_to_its_old_id() {
    let mut r = CommandRegistry::<App>::new();
    let renamed = CommandDefinition::new("history.undo", "Undo").legacy(&["undo"]);
    r.bind(&renamed, areas::HS_HIT_EDITING, undo, Some(has_doc));
    let catalog = CommandCatalog::build(r).unwrap();
    assert_eq!(
        catalog.find("undo").map(|d| d.id.as_str()),
        Some("history.undo")
    );
    assert!(catalog.unknown_ids(["undo", "history.undo"]).is_empty());
    let mut app = App {
        hitsound: Some(Vec::new()),
        ..App::default()
    };
    assert_eq!(
        catalog.run(&mut app, "undo", EditorScope::Hitsound),
        RunOutcome::Ran
    );
}

#[test]
fn r12_a_key_resolves_in_the_focused_editor_first_then_global() {
    let mut r = CommandRegistry::<App>::new();
    let both = CommandDefinition::new("both", "x").keys(&[KeyCombination::plain(Key::F5)]);
    fn hs(app: &mut App) -> Result<(), String> {
        app.ran.push("hs");
        Ok(())
    }
    fn global(app: &mut App) -> Result<(), String> {
        app.ran.push("global");
        Ok(())
    }
    r.bind(&both, areas::HS_HIT_EDITING, hs, None);
    r.bind(&both, areas::PREFERENCES, global, None);
    let catalog = CommandCatalog::build(r).unwrap();
    let keys = KeyMap::from_preset(catalog.definitions(), KeyPreset::DawLike);
    let mut app = App::default();
    catalog.handle_key(
        &mut app,
        &keys,
        press(Key::F5, Mods::NONE),
        EditorScope::Hitsound,
    );
    catalog.handle_key(
        &mut app,
        &keys,
        press(Key::F5, Mods::NONE),
        EditorScope::Keysound,
    );
    assert_eq!(app.ran, ["hs", "global"]);
}

#[test]
fn i10_a_key_no_command_handles_goes_back_to_the_ui() {
    let catalog = kernel_catalog();
    let keys = KeyMap::from_preset(catalog.definitions(), KeyPreset::DawLike);
    let mut app = App::default();
    let q = press(Key::Q, Mods::NONE);
    assert_eq!(
        catalog.handle_key(&mut app, &keys, q, EditorScope::Hitsound),
        KeyOutcome::Unhandled(q)
    );
    // Bound only in editors, so the node editor does not take Ctrl+Z.
    let ctrl_z = press(Key::Z, Mods::CTRL);
    assert_eq!(
        catalog.handle_key(&mut app, &keys, ctrl_z, EditorScope::Nodal),
        KeyOutcome::Unhandled(ctrl_z)
    );
}

#[test]
fn i6_every_id_is_unique_across_current_and_legacy_ids() {
    let mut r = CommandRegistry::<App>::new();
    r.bind(
        &CommandDefinition::new("a", "A"),
        areas::HS_LANES,
        nothing,
        None,
    );
    r.bind(
        &CommandDefinition::new("b", "B").legacy(&["a"]),
        areas::HS_LANES,
        nothing,
        None,
    );
    r.bind(
        &CommandDefinition::new("a", "another A"),
        areas::HS_LANES,
        nothing,
        None,
    );
    let problems = CommandCatalog::build(r).err().unwrap();
    assert_eq!(
        problems
            .iter()
            .filter(|p| **p == CatalogProblem::DuplicateId("a".into()))
            .count(),
        2
    );
}

#[test]
fn i7_every_command_is_bound_and_every_area_is_known() {
    let mut r = CommandRegistry::<App>::new();
    r.declare(&CommandDefinition::new("orphan", "never bound"));
    let made_up = CommandArea {
        name: "Hitsound – Nowhere",
        scope: EditorScope::Hitsound,
        order: 99,
    };
    r.bind(
        &CommandDefinition::new("lost", "bound to an unknown area"),
        made_up,
        nothing,
        None,
    );
    let problems = CommandCatalog::build(r).err().unwrap();
    assert!(problems.contains(&CatalogProblem::Unbound("orphan".into())));
    assert!(problems.contains(&CatalogProblem::UnknownArea {
        command: "lost".into(),
        area: "Hitsound – Nowhere"
    }));
}

#[test]
fn i8_a_key_clash_in_one_scope_fails_and_different_editors_may_share_a_key() {
    let k = [KeyCombination::plain(Key::F7)];
    let mut r = CommandRegistry::<App>::new();
    r.bind(
        &CommandDefinition::new("hs", "x").keys(&k),
        areas::HS_LANES,
        nothing,
        None,
    );
    r.bind(
        &CommandDefinition::new("ks", "y").keys(&k),
        areas::KS_OUTPUT,
        nothing,
        None,
    );
    assert!(
        CommandCatalog::build(r).is_ok(),
        "Hitsound and Keysound are never focused together"
    );
    let mut r = CommandRegistry::<App>::new();
    r.bind(
        &CommandDefinition::new("hs", "x").keys(&k),
        areas::HS_LANES,
        nothing,
        None,
    );
    r.bind(
        &CommandDefinition::new("global", "y").keys(&k),
        areas::HELP,
        nothing,
        None,
    );
    let problems = CommandCatalog::build(r).err().unwrap();
    assert!(matches!(
        &problems[0],
        CatalogProblem::KeyClash {
            preset: KeyPreset::DawLike,
            ..
        }
    ));
}

#[test]
fn i8_the_kernel_commands_have_no_key_clash_in_either_preset() {
    // kernel_catalog() panics on any problem, including clashes in the DAW-like and osu! presets.
    let catalog = kernel_catalog();
    let osu = KeyMap::from_preset(catalog.definitions(), KeyPreset::Osu);
    assert_eq!(
        osu.command_for(KeyCombination::plain(Key::D2)),
        Some("toolDraw"),
        "2 is the placing tool in osu!"
    );
    assert!(osu.keys_of("toolGrab").is_empty());
    assert_eq!(
        osu.command_for(KeyCombination::new(Key::Left, Mods::SHIFT)),
        Some("seekBackFar")
    );
}

#[test]
fn key_changes_take_their_keys_away_from_other_commands() {
    let catalog = kernel_catalog();
    let mut keys = KeyMap::from_preset(catalog.definitions(), KeyPreset::DawLike);
    keys.change("redo", &[KeyCombination::new(Key::Z, Mods::CTRL)]);
    assert_eq!(
        keys.command_for(KeyCombination::new(Key::Z, Mods::CTRL)),
        Some("redo")
    );
    assert!(keys.keys_of("undo").is_empty());
    keys.take(
        "undo",
        &[
            KeyCombination::new(Key::Z, Mods::CTRL),
            KeyCombination::new(Key::U, Mods::CTRL),
        ],
    );
    assert_eq!(
        keys.keys_of("undo"),
        &[KeyCombination::new(Key::U, Mods::CTRL)],
        "take only gets free keys"
    );
}

#[test]
fn i11c_a_command_whose_long_work_runs_cannot_start_again() {
    fn generate(app: &mut App) -> Result<(), String> {
        app.long_work_running = true;
        Ok(())
    }
    fn idle(app: &App) -> bool {
        !app.long_work_running
    }
    let mut r = CommandRegistry::<App>::new();
    r.bind(
        &CommandDefinition::new("generate", "Generate"),
        areas::HS_MAP_OUTPUT,
        generate,
        Some(idle),
    );
    let catalog = CommandCatalog::build(r).unwrap();
    let mut app = App::default();
    assert_eq!(
        catalog.run(&mut app, "generate", EditorScope::Hitsound),
        RunOutcome::Ran
    );
    assert_eq!(
        catalog.run(&mut app, "generate", EditorScope::Hitsound),
        RunOutcome::CannotRun
    );
}

#[test]
fn i12_a_failing_command_is_reported_with_its_id_and_input_handling_goes_on() {
    fn fails(_: &mut App) -> Result<(), String> {
        Err("disk full".into())
    }
    let mut r = CommandRegistry::<App>::new();
    r.bind(
        &CommandDefinition::new("save", "Save").keys(&[KeyCombination::new(Key::S, Mods::CTRL)]),
        areas::HS_MAP_OUTPUT,
        fails,
        None,
    );
    r.bind(&ec::undo(), areas::HS_HIT_EDITING, undo, Some(has_doc));
    let catalog = CommandCatalog::build(r).unwrap();
    let keys = KeyMap::from_preset(catalog.definitions(), KeyPreset::DawLike);
    let mut app = App {
        hitsound: Some(Vec::new()),
        ..App::default()
    };
    let outcome = catalog.handle_key(
        &mut app,
        &keys,
        press(Key::S, Mods::CTRL),
        EditorScope::Hitsound,
    );
    assert_eq!(
        outcome,
        KeyOutcome::Command {
            id: "save".into(),
            outcome: RunOutcome::Failed {
                id: "save".into(),
                message: "disk full".into()
            }
        }
    );
    catalog.handle_key(
        &mut app,
        &keys,
        press(Key::Z, Mods::CTRL),
        EditorScope::Hitsound,
    );
    assert_eq!(app.ran, ["undo"]);
}

#[test]
fn unknown_and_out_of_scope_commands_are_told_apart() {
    let catalog = kernel_catalog();
    let mut app = App::default();
    assert_eq!(
        catalog.run(&mut app, "no.such", EditorScope::Hitsound),
        RunOutcome::Unknown
    );
    assert_eq!(
        catalog.run(&mut app, "undo", EditorScope::Nodal),
        RunOutcome::NotInScope
    );
}

#[test]
fn key_names_print_as_on_the_keyboard() {
    assert_eq!(
        KeyCombination::new(Key::D1, Mods::SHIFT).to_string(),
        "Shift+1"
    );
    assert_eq!(
        KeyCombination::new(Key::Comma, Mods::CTRL).to_string(),
        "Ctrl+,"
    );
    assert_eq!(KeyCombination::plain(Key::NumPad2).to_string(), "Numpad 2");
    assert_eq!(
        KeyCombination::plain(Key::NumPadDivide).to_string(),
        "Numpad /"
    );
    assert_eq!(
        KeyCombination::new(Key::Z, Mods::CTRL | Mods::SHIFT).to_string(),
        "Ctrl+Shift+Z"
    );
}
