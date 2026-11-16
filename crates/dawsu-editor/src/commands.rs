//! The command catalogue (Responsibilities 7-9, 11, 12; invariants 6-10, 12).
//!
//! A command is declared once, as a `CommandDefinition` beside the feature that owns it. Features
//! bind handlers through a `CommandRegistry`; the catalogue built from it is the only list help,
//! Preferences > Keys and menus read. Handlers are plain `fn` pointers that receive the
//! application context when they run, so they cannot hold on to a document (invariant 9 of the
//! design details; Responsibility 9). Lookups go through the definitions, never through a table
//! from id strings to areas or handlers (invariant 9).

use crate::keys::{KeyChord, KeyCombination};
use std::fmt;

/// Which editor a command area belongs to. Global commands run whichever editor has focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EditorScope {
    Global,
    Hitsound,
    Keysound,
    Nodal,
}

impl EditorScope {
    /// True when commands of the two scopes can be active at the same time.
    pub fn overlaps(self, other: EditorScope) -> bool {
        self == other || self == EditorScope::Global || other == EditorScope::Global
    }
}

/// A group of commands in F1 help and Preferences > Keys. `name` keeps the old display text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CommandArea {
    pub name: &'static str,
    pub scope: EditorScope,
    pub order: u16,
}

impl CommandArea {
    /// The help group of this area's mouse actions, shown right after the area's keys.
    pub fn mouse_group(self) -> String {
        format!("{} (mouse)", self.name)
    }
}

/// The areas, with the 3.1.x names and order (`Areas` in the old `EditorCommands.cs`).
pub mod areas {
    use super::{CommandArea, EditorScope};

    const fn area(name: &'static str, scope: EditorScope, order: u16) -> CommandArea {
        CommandArea { name, scope, order }
    }

    pub const HS_MAP_FILES: CommandArea = area("Hitsound – Map files", EditorScope::Hitsound, 0);
    pub const HS_OSU_CONNECTION: CommandArea =
        area("Hitsound – osu! connection", EditorScope::Hitsound, 1);
    pub const HS_LANES: CommandArea = area("Hitsound – Lanes", EditorScope::Hitsound, 2);
    pub const HS_HIT_EDITING: CommandArea =
        area("Hitsound – Hit editing", EditorScope::Hitsound, 3);
    pub const HS_TIMELINE: CommandArea =
        area("Hitsound – Timeline navigation", EditorScope::Hitsound, 4);
    pub const HS_VOLUME: CommandArea = area("Hitsound – Volume control", EditorScope::Hitsound, 5);
    pub const HS_MAP_OUTPUT: CommandArea = area("Hitsound – Map output", EditorScope::Hitsound, 6);
    pub const HS_PLAYBACK: CommandArea = area("Hitsound – Playback", EditorScope::Hitsound, 7);
    pub const KS_NOTE_EDITING: CommandArea =
        area("Keysound – Note editing", EditorScope::Keysound, 8);
    pub const KS_NOTE_INPUT: CommandArea = area("Keysound – Note input", EditorScope::Keysound, 9);
    pub const KS_SYNC: CommandArea = area("Keysound – Gameplay sync", EditorScope::Keysound, 10);
    pub const KS_OUTPUT: CommandArea = area("Keysound – Output", EditorScope::Keysound, 11);
    pub const KS_PLAYBACK: CommandArea = area("Keysound – Playback", EditorScope::Keysound, 12);
    pub const PREFERENCES: CommandArea = area("Preferences", EditorScope::Global, 13);
    pub const HELP: CommandArea = area("Help and updates", EditorScope::Global, 14);

    /// Every known area, in help order.
    pub const ALL: [CommandArea; 15] = [
        HS_MAP_FILES,
        HS_OSU_CONNECTION,
        HS_LANES,
        HS_HIT_EDITING,
        HS_TIMELINE,
        HS_VOLUME,
        HS_MAP_OUTPUT,
        HS_PLAYBACK,
        KS_NOTE_EDITING,
        KS_NOTE_INPUT,
        KS_SYNC,
        KS_OUTPUT,
        KS_PLAYBACK,
        PREFERENCES,
        HELP,
    ];
}

/// The key presets (`dawsu-application` keeps them and builds the key map; the definitions carry
/// each preset's keys).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyPreset {
    /// The built-in keys; a Cubase key file adds to them in `dawsu-application`.
    DawLike,
    /// osu!'s editor keys where they differ.
    Osu,
}

/// One command. Its id is a public format: `.dawsukeys` files and `preferences.json` store it.
#[derive(Debug, Clone, PartialEq)]
pub struct CommandDefinition {
    pub id: String,
    pub description: String,
    pub default_keys: Vec<KeyCombination>,
    /// `None`: the same as `default_keys`.
    pub osu_keys: Option<Vec<KeyCombination>>,
    /// The matching Cubase command, such as "Edit|Undo".
    pub cubase: Option<String>,
    /// Old ids read from 3.1.x key files and preferences.
    pub legacy_ids: Vec<String>,
}

impl CommandDefinition {
    pub fn new(id: impl Into<String>, description: impl Into<String>) -> CommandDefinition {
        CommandDefinition {
            id: id.into(),
            description: description.into(),
            default_keys: Vec::new(),
            osu_keys: None,
            cubase: None,
            legacy_ids: Vec::new(),
        }
    }

    pub fn keys(mut self, keys: &[KeyCombination]) -> Self {
        self.default_keys = keys.to_vec();
        self
    }

    pub fn osu(mut self, keys: &[KeyCombination]) -> Self {
        self.osu_keys = Some(keys.to_vec());
        self
    }

    pub fn cubase(mut self, command: &str) -> Self {
        self.cubase = Some(command.to_owned());
        self
    }

    pub fn legacy(mut self, ids: &[&str]) -> Self {
        self.legacy_ids = ids.iter().map(|s| (*s).to_owned()).collect();
        self
    }

    pub fn keys_for(&self, preset: KeyPreset) -> &[KeyCombination] {
        match (preset, &self.osu_keys) {
            (KeyPreset::Osu, Some(keys)) => keys,
            _ => &self.default_keys,
        }
    }

    /// True when `id` is this command's current id or one of its legacy ids.
    pub fn answers_to(&self, id: &str) -> bool {
        self.id == id || self.legacy_ids.iter().any(|l| l == id)
    }
}

/// A numbered family declared once (CS-17): ids `prefix` + `from..=to`, one handler taking the number.
#[derive(Debug, Clone, Copy)]
pub struct CommandFamily {
    pub prefix: &'static str,
    pub from: u32,
    pub to: u32,
    pub define: fn(u32) -> CommandDefinition,
}

impl CommandFamily {
    pub fn members(&self) -> Vec<(u32, CommandDefinition)> {
        (self.from..=self.to)
            .map(|n| (n, (self.define)(n)))
            .collect()
    }
}

/// A mouse action, declared once beside the tool that owns it; help lists it under "(mouse)".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MouseAction {
    pub gesture: &'static str,
    pub description: &'static str,
}

/// Runs a command against the application context, found when it runs.
pub type RunFn<C> = fn(&mut C) -> Result<(), String>;
/// False when the command cannot run now (no document of its kind open, its long work running).
pub type CanRunFn<C> = fn(&C) -> bool;
pub type FamilyRunFn<C> = fn(&mut C, u32) -> Result<(), String>;
pub type FamilyCanRunFn<C> = fn(&C, u32) -> bool;

enum Handler<C> {
    Plain {
        run: RunFn<C>,
        can_run: Option<CanRunFn<C>>,
    },
    Family {
        run: FamilyRunFn<C>,
        can_run: Option<FamilyCanRunFn<C>>,
        value: u32,
    },
}

impl<C> Clone for Handler<C> {
    fn clone(&self) -> Self {
        match self {
            Handler::Plain { run, can_run } => Handler::Plain {
                run: *run,
                can_run: *can_run,
            },
            Handler::Family {
                run,
                can_run,
                value,
            } => Handler::Family {
                run: *run,
                can_run: *can_run,
                value: *value,
            },
        }
    }
}

struct Binding<C> {
    definition: usize,
    area: CommandArea,
    handler: Handler<C>,
}

/// Collects definitions, handlers and mouse actions from every feature at startup.
pub struct CommandRegistry<C> {
    definitions: Vec<CommandDefinition>,
    bindings: Vec<Binding<C>>,
    mouse: Vec<(CommandArea, MouseAction)>,
    known_areas: Vec<CommandArea>,
}

/// A feature that declares and binds commands. Lives for the whole app run; finds documents only
/// when a handler runs.
pub trait CommandContributor<C> {
    fn register(&self, registry: &mut CommandRegistry<C>);
}

impl<C> Default for CommandRegistry<C> {
    fn default() -> Self {
        CommandRegistry {
            definitions: Vec::new(),
            bindings: Vec::new(),
            mouse: Vec::new(),
            known_areas: areas::ALL.to_vec(),
        }
    }
}

impl<C> CommandRegistry<C> {
    pub fn new() -> Self {
        Self::default()
    }

    /// A registry that knows other areas than `areas::ALL` (tests).
    pub fn with_areas(known_areas: &[CommandArea]) -> Self {
        CommandRegistry {
            known_areas: known_areas.to_vec(),
            ..Self::default()
        }
    }

    /// Declares a command. Declaring the same definition again is harmless; a different definition
    /// with the same id is kept, so the catalogue check reports the duplicate.
    pub fn declare(&mut self, definition: &CommandDefinition) -> usize {
        if let Some(i) = self.definitions.iter().position(|d| d == definition) {
            return i;
        }
        self.definitions.push(definition.clone());
        self.definitions.len() - 1
    }

    pub fn bind(
        &mut self,
        definition: &CommandDefinition,
        area: CommandArea,
        run: RunFn<C>,
        can_run: Option<CanRunFn<C>>,
    ) {
        let index = self.declare(definition);
        self.bindings.push(Binding {
            definition: index,
            area,
            handler: Handler::Plain { run, can_run },
        });
    }

    pub fn bind_family(
        &mut self,
        family: &CommandFamily,
        area: CommandArea,
        run: FamilyRunFn<C>,
        can_run: Option<FamilyCanRunFn<C>>,
    ) {
        for (value, definition) in family.members() {
            let index = self.declare(&definition);
            self.bindings.push(Binding {
                definition: index,
                area,
                handler: Handler::Family {
                    run,
                    can_run,
                    value,
                },
            });
        }
    }

    pub fn mouse(&mut self, area: CommandArea, actions: &[MouseAction]) {
        self.mouse.extend(actions.iter().map(|a| (area, *a)));
    }

    pub fn contribute(&mut self, contributor: &dyn CommandContributor<C>) {
        contributor.register(self);
    }
}

/// What the startup check found wrong (invariants 6, 7 and 8). Each one fails the build's tests.
#[derive(Debug, Clone, PartialEq)]
pub enum CatalogProblem {
    /// Two commands answer to the same id (current or legacy).
    DuplicateId(String),
    /// A declared command no area binds.
    Unbound(String),
    /// A binding names an area that is not known.
    UnknownArea { command: String, area: &'static str },
    /// One key runs two commands that can be active in the same scope.
    KeyClash {
        preset: KeyPreset,
        key: KeyCombination,
        first: String,
        second: String,
    },
}

impl fmt::Display for CatalogProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CatalogProblem::DuplicateId(id) => write!(f, "the command id {id:?} is used twice"),
            CatalogProblem::Unbound(id) => {
                write!(f, "the command {id:?} has no handler in any area")
            }
            CatalogProblem::UnknownArea { command, area } => write!(
                f,
                "the command {command:?} is bound to the unknown area {area:?}"
            ),
            CatalogProblem::KeyClash {
                preset,
                key,
                first,
                second,
            } => {
                write!(
                    f,
                    "{key} runs both {first:?} and {second:?} in the {preset:?} preset"
                )
            }
        }
    }
}

/// What running a command did.
#[derive(Debug, Clone, PartialEq)]
pub enum RunOutcome {
    Ran,
    /// Its `can_run` said no (no document of its kind open, or its long work still running).
    CannotRun,
    /// The handler failed; reported with the command id (invariant 12(a)). Input handling goes on.
    Failed {
        id: String,
        message: String,
    },
    /// No binding in the focused scope or Global.
    NotInScope,
    /// No command answers to this id.
    Unknown,
}

/// What a key press did.
#[derive(Debug, Clone, PartialEq)]
pub enum KeyOutcome {
    /// No command in the focused scope or Global handles the key: it goes back to the UI (invariant 10).
    Unhandled(KeyChord),
    Command {
        id: String,
        outcome: RunOutcome,
    },
}

/// One line of help: the keys in force (or the mouse gesture) and the description.
#[derive(Debug, Clone, PartialEq)]
pub struct HelpEntry {
    pub keys: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HelpGroup {
    pub title: String,
    pub entries: Vec<HelpEntry>,
}

/// The keys in force: each command id with its key combinations, each combination on one command.
/// `dawsu-application` builds it from a preset, a Cubase file and the user's key changes.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct KeyMap {
    keys: Vec<(String, Vec<KeyCombination>)>,
}

impl KeyMap {
    pub fn new() -> KeyMap {
        KeyMap::default()
    }

    /// The preset's keys of every definition, in definition order; a key already taken stays with
    /// the first command (the old `Keymap.Build`'s `Take`).
    pub fn from_preset(definitions: &[CommandDefinition], preset: KeyPreset) -> KeyMap {
        let mut map = KeyMap::new();
        for d in definitions {
            map.take(&d.id, d.keys_for(preset));
        }
        map
    }

    /// Gives `id` the combinations no command has yet, if `id` has no keys yet.
    pub fn take(&mut self, id: &str, combinations: &[KeyCombination]) {
        if self.keys.iter().any(|(i, k)| i == id && !k.is_empty()) {
            return;
        }
        let free: Vec<KeyCombination> = combinations
            .iter()
            .copied()
            .filter(|c| self.command_for(*c).is_none())
            .collect();
        if !free.is_empty() {
            self.keys.retain(|(i, _)| i != id);
            self.keys.push((id.to_owned(), free));
        }
    }

    /// A user's key change: `id` gets exactly `combinations`, taken away from any other command.
    pub fn change(&mut self, id: &str, combinations: &[KeyCombination]) {
        for (_, keys) in &mut self.keys {
            keys.retain(|k| !combinations.contains(k));
        }
        self.keys.retain(|(i, _)| i != id);
        self.keys.push((id.to_owned(), combinations.to_vec()));
    }

    pub fn command_for(&self, combination: KeyCombination) -> Option<&str> {
        self.keys
            .iter()
            .find(|(_, k)| k.contains(&combination))
            .map(|(i, _)| i.as_str())
    }

    pub fn keys_of(&self, id: &str) -> &[KeyCombination] {
        self.keys
            .iter()
            .find(|(i, _)| i == id)
            .map_or(&[], |(_, k)| k)
    }
}

/// Every command, its bindings and mouse actions, checked at startup.
pub struct CommandCatalog<C> {
    definitions: Vec<CommandDefinition>,
    bindings: Vec<Binding<C>>,
    mouse: Vec<(CommandArea, MouseAction)>,
}

impl<C> CommandCatalog<C> {
    /// Builds the catalogue and runs every startup check (invariants 6, 7, 8 and 12(c)).
    pub fn build(registry: CommandRegistry<C>) -> Result<CommandCatalog<C>, Vec<CatalogProblem>> {
        let mut problems = Vec::new();
        let defs = &registry.definitions;

        let mut seen: Vec<&str> = Vec::new();
        for d in defs {
            for id in std::iter::once(&d.id).chain(d.legacy_ids.iter()) {
                if seen.contains(&id.as_str()) {
                    problems.push(CatalogProblem::DuplicateId(id.clone()));
                } else {
                    seen.push(id);
                }
            }
        }
        for (i, d) in defs.iter().enumerate() {
            if !registry.bindings.iter().any(|b| b.definition == i) {
                problems.push(CatalogProblem::Unbound(d.id.clone()));
            }
        }
        for b in &registry.bindings {
            if !registry.known_areas.contains(&b.area) {
                problems.push(CatalogProblem::UnknownArea {
                    command: defs[b.definition].id.clone(),
                    area: b.area.name,
                });
            }
        }
        let scopes = |i: usize| {
            registry
                .bindings
                .iter()
                .filter(move |b| b.definition == i)
                .map(|b| b.area.scope)
        };
        for preset in [KeyPreset::DawLike, KeyPreset::Osu] {
            for i in 0..defs.len() {
                for j in i + 1..defs.len() {
                    let can_meet = scopes(i).any(|a| scopes(j).any(|b| a.overlaps(b)));
                    if !can_meet {
                        continue;
                    }
                    for key in defs[i].keys_for(preset) {
                        if defs[j].keys_for(preset).contains(key) {
                            problems.push(CatalogProblem::KeyClash {
                                preset,
                                key: *key,
                                first: defs[i].id.clone(),
                                second: defs[j].id.clone(),
                            });
                        }
                    }
                }
            }
        }

        if problems.is_empty() {
            Ok(CommandCatalog {
                definitions: registry.definitions,
                bindings: registry.bindings,
                mouse: registry.mouse,
            })
        } else {
            Err(problems)
        }
    }

    pub fn definitions(&self) -> &[CommandDefinition] {
        &self.definitions
    }

    /// Every (command, area) pair: what F1 help, Preferences > Keys and menus list (Responsibility 8).
    pub fn bindings(&self) -> Vec<(&CommandDefinition, CommandArea)> {
        self.bindings
            .iter()
            .map(|b| (&self.definitions[b.definition], b.area))
            .collect()
    }

    /// The command answering to `id`, current or legacy (Responsibility 11).
    pub fn find(&self, id: &str) -> Option<&CommandDefinition> {
        self.definitions.iter().find(|d| d.answers_to(id))
    }

    /// The ids in `ids` no command answers to (a key file or preferences loaded from 3.1.x).
    pub fn unknown_ids<'a>(&self, ids: impl IntoIterator<Item = &'a str>) -> Vec<String> {
        ids.into_iter()
            .filter(|id| self.find(id).is_none())
            .map(str::to_owned)
            .collect()
    }

    /// The binding that handles command `id` in the focused scope, else in Global.
    fn binding_for(&self, id: &str, focused: EditorScope) -> Option<&Binding<C>> {
        let def = self.definitions.iter().position(|d| d.answers_to(id))?;
        let in_scope = |scope: EditorScope| {
            self.bindings
                .iter()
                .find(|b| b.definition == def && b.area.scope == scope)
        };
        in_scope(focused).or_else(|| in_scope(EditorScope::Global))
    }

    /// The command a key press runs in the focused scope (the focused editor first, then Global).
    pub fn resolve(
        &self,
        keys: &KeyMap,
        chord: KeyChord,
        focused: EditorScope,
    ) -> Option<&CommandDefinition> {
        let id = keys.command_for(chord.combination())?;
        self.binding_for(id, focused)
            .map(|b| &self.definitions[b.definition])
    }

    /// Whether command `id` could run now in the focused scope.
    pub fn can_run(&self, context: &C, id: &str, focused: EditorScope) -> bool {
        match self.binding_for(id, focused).map(|b| &b.handler) {
            Some(Handler::Plain { can_run, .. }) => can_run.is_none_or(|f| f(context)),
            Some(Handler::Family { can_run, value, .. }) => {
                can_run.is_none_or(|f| f(context, *value))
            }
            None => false,
        }
    }

    /// Runs command `id` (current or legacy) in the focused scope.
    pub fn run(&self, context: &mut C, id: &str, focused: EditorScope) -> RunOutcome {
        let Some(def) = self.find(id) else {
            return RunOutcome::Unknown;
        };
        let Some(binding) = self.binding_for(id, focused) else {
            return RunOutcome::NotInScope;
        };
        let result = match binding.handler.clone() {
            Handler::Plain { run, can_run } => {
                if can_run.is_some_and(|f| !f(context)) {
                    return RunOutcome::CannotRun;
                }
                run(context)
            }
            Handler::Family {
                run,
                can_run,
                value,
            } => {
                if can_run.is_some_and(|f| !f(context, value)) {
                    return RunOutcome::CannotRun;
                }
                run(context, value)
            }
        };
        match result {
            Ok(()) => RunOutcome::Ran,
            Err(message) => RunOutcome::Failed {
                id: def.id.clone(),
                message,
            },
        }
    }

    /// A key press: resolves it and runs the command, or hands the key back unhandled.
    pub fn handle_key(
        &self,
        context: &mut C,
        keys: &KeyMap,
        chord: KeyChord,
        focused: EditorScope,
    ) -> KeyOutcome {
        match self.resolve(keys, chord, focused) {
            None => KeyOutcome::Unhandled(chord),
            Some(def) => {
                let id = def.id.clone();
                let outcome = self.run(context, &id, focused);
                KeyOutcome::Command { id, outcome }
            }
        }
    }

    /// F1 help: areas in their order, each area's commands with the keys in force, then its mouse
    /// actions as "<area> (mouse)". Empty groups are left out.
    pub fn help(&self, keys: &KeyMap) -> Vec<HelpGroup> {
        let mut areas: Vec<CommandArea> = Vec::new();
        for area in self
            .bindings
            .iter()
            .map(|b| b.area)
            .chain(self.mouse.iter().map(|m| m.0))
        {
            if !areas.contains(&area) {
                areas.push(area);
            }
        }
        areas.sort_by_key(|a| a.order);
        let mut groups = Vec::new();
        for area in areas {
            let entries: Vec<HelpEntry> = self
                .bindings
                .iter()
                .filter(|b| b.area == area)
                .map(|b| &self.definitions[b.definition])
                .map(|d| HelpEntry {
                    keys: keys
                        .keys_of(&d.id)
                        .iter()
                        .map(|k| k.to_string())
                        .collect::<Vec<_>>()
                        .join(", "),
                    description: d.description.clone(),
                })
                .collect();
            if !entries.is_empty() {
                groups.push(HelpGroup {
                    title: area.name.to_owned(),
                    entries,
                });
            }
            let mouse: Vec<HelpEntry> = self
                .mouse
                .iter()
                .filter(|m| m.0 == area)
                .map(|m| HelpEntry {
                    keys: m.1.gesture.to_owned(),
                    description: m.1.description.to_owned(),
                })
                .collect();
            if !mouse.is_empty() {
                groups.push(HelpGroup {
                    title: area.mouse_group(),
                    entries: mouse,
                });
            }
        }
        groups
    }
}
