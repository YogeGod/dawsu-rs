use crate::{Application, Preferences, commands};
use dawsu_foundation::safe_write::{self, BackupPolicy, WriteOutcome};
use dawsu_editor::{
    commands::{CommandDefinition, KeyMap, KeyPreset},
    keys::{Key, KeyCombination, Mods},
};
use serde::{
    Deserialize, Deserializer, Serialize,
    de::{MapAccess, Visitor},
};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::path::Path;

pub(super) type Overrides = BTreeMap<String, Vec<String>>;
pub(super) type Presets = BTreeMap<String, Overrides>;
pub(super) type LegacyOverrides = BTreeMap<String, Ordered<Value>>;

// Key file conflict precedence is defined by object order. serde_json::Value would sort its keys.
#[derive(Default)]
pub(super) struct Ordered<V>(Vec<(String, V)>);

impl<'de, V: Deserialize<'de>> Deserialize<'de> for Ordered<V> {
    fn deserialize<D: Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        struct OrderedVisitor<V>(std::marker::PhantomData<V>);
        impl<'de, V: Deserialize<'de>> Visitor<'de> for OrderedVisitor<V> {
            type Value = Ordered<V>;
            fn expecting(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
                output.write_str("an object of command IDs")
            }
            fn visit_map<M: MapAccess<'de>>(self, mut access: M) -> Result<Self::Value, M::Error> {
                let mut entries = Vec::new();
                while let Some(entry) = access.next_entry()? {
                    entries.push(entry);
                }
                Ok(Ordered(entries))
            }
        }
        de.deserialize_map(OrderedVisitor(std::marker::PhantomData))
    }
}

fn preset(name: &str) -> KeyPreset {
    if name == "osu!" {
        KeyPreset::Osu
    } else {
        KeyPreset::DawLike
    }
}

fn stored(key: KeyCombination) -> String {
    let mut modifiers = Vec::new();
    if key.mods.ctrl() {
        modifiers.push("Control");
    }
    if key.mods.shift() {
        modifiers.push("Shift");
    }
    if key.mods.alt() {
        modifiers.push("Alt");
    }
    if modifiers.is_empty() {
        modifiers.push("None");
    }
    let name = match key.key {
        Key::Backspace => "Back",
        Key::Comma => "OemComma",
        Key::Period => "OemPeriod",
        Key::Minus => "OemMinus",
        Key::Equals => "OemPlus",
        Key::Slash => "OemQuestion",
        Key::Backslash => "OemPipe",
        Key::Semicolon => "OemSemicolon",
        Key::Quote => "OemQuotes",
        Key::OpenBracket => "OemOpenBrackets",
        Key::CloseBracket => "OemCloseBrackets",
        Key::Grave => "OemTilde",
        Key::NumPadDivide => "Divide",
        Key::NumPadMultiply => "Multiply",
        Key::NumPadSubtract => "Subtract",
        Key::NumPadAdd => "Add",
        Key::NumPadDecimal => "Decimal",
        other => return format!("{}+{other:?}", modifiers.join(",")),
    };
    format!("{}+{name}", modifiers.join(","))
}

fn parse(text: &str) -> Option<KeyCombination> {
    use Key::*;
    let (modifiers, name) = text.rsplit_once('+')?;
    let mut mods = Mods::NONE;
    for modifier in modifiers.split(',') {
        mods = mods.union(match modifier.trim() {
            "Control" => Mods::CTRL,
            "Shift" => Mods::SHIFT,
            "Alt" => Mods::ALT,
            "None" => Mods::NONE,
            _ => return None,
        });
    }
    macro_rules! named {
        ($($key:ident),+ $(,)?) => { match name {
            $( stringify!($key) => $key, )+
            "Return" => Enter,
            "Back" => Backspace,
            "Prior" => PageUp,
            "Next" => PageDown,
            "OemComma" => Comma,
            "OemPeriod" => Period,
            "OemMinus" => Minus,
            "OemPlus" => Equals,
            "OemQuestion" | "Oem2" => Slash,
            "OemPipe" | "Oem5" | "OemBackslash" | "Oem102" => Backslash,
            "OemSemicolon" | "Oem1" => Semicolon,
            "OemQuotes" | "Oem7" => Quote,
            "OemOpenBrackets" | "Oem4" => OpenBracket,
            "OemCloseBrackets" | "Oem6" => CloseBracket,
            "OemTilde" | "Oem3" => Grave,
            "Divide" => NumPadDivide,
            "Multiply" => NumPadMultiply,
            "Subtract" => NumPadSubtract,
            "Add" => NumPadAdd,
            "Decimal" => NumPadDecimal,
            _ => return None,
        } }
    }
    let key = named!(
        A,
        B,
        C,
        D,
        E,
        F,
        G,
        H,
        I,
        J,
        K,
        L,
        M,
        N,
        O,
        P,
        Q,
        R,
        S,
        T,
        U,
        V,
        W,
        X,
        Y,
        Z,
        D0,
        D1,
        D2,
        D3,
        D4,
        D5,
        D6,
        D7,
        D8,
        D9,
        NumPad0,
        NumPad1,
        NumPad2,
        NumPad3,
        NumPad4,
        NumPad5,
        NumPad6,
        NumPad7,
        NumPad8,
        NumPad9,
        NumPadDivide,
        NumPadMultiply,
        NumPadSubtract,
        NumPadAdd,
        NumPadDecimal,
        NumPadEnter,
        F1,
        F2,
        F3,
        F4,
        F5,
        F6,
        F7,
        F8,
        F9,
        F10,
        F11,
        F12,
        Space,
        Enter,
        Tab,
        Escape,
        Backspace,
        Delete,
        Insert,
        Home,
        End,
        PageUp,
        PageDown,
        Left,
        Right,
        Up,
        Down,
        Comma,
        Period,
        Minus,
        Equals,
        Slash,
        Backslash,
        Semicolon,
        Quote,
        OpenBracket,
        CloseBracket,
        Grave,
    );
    Some(KeyCombination::new(key, mods))
}

pub(super) fn keymap(definitions: &[CommandDefinition], preferences: &Preferences) -> KeyMap {
    let mut map = KeyMap::from_preset(definitions, preset(&preferences.key_preset));
    if let Some(overrides) = preferences.key_overrides.get(&preferences.key_preset) {
        for (id, keys) in overrides {
            if let Some(definition) = definitions
                .iter()
                .find(|definition| definition.answers_to(id))
            {
                map.change(
                    &definition.id,
                    &keys.iter().filter_map(|key| parse(key)).collect::<Vec<_>>(),
                );
            }
        }
    }
    map
}

fn changes(definitions: &[CommandDefinition], base: &KeyMap, map: &KeyMap) -> Overrides {
    definitions
        .iter()
        .filter_map(|definition| {
            let id = &definition.id;
            let current = map.keys_of(id);
            let original = base.keys_of(id);
            (current.len() != original.len() || current.iter().any(|key| !original.contains(key)))
                .then(|| (id.clone(), current.iter().copied().map(stored).collect()))
        })
        .collect()
}

pub(super) fn migrate(legacy: LegacyOverrides) -> Presets {
    if legacy.is_empty() {
        return Presets::new();
    }
    let catalogue = commands::catalogue();
    let definitions = catalogue.definitions();
    legacy
        .into_iter()
        .filter_map(|(name, entries)| {
            let name = match name.as_str() {
                "Cubase" => "DAW",
                "Osu" => "osu!",
                _ => return None,
            };
            let base = KeyMap::from_preset(definitions, preset(name));
            let mut map = base.clone();
            for (id, keys) in entries.0 {
                if let Some(definition) = definitions
                    .iter()
                    .find(|definition| definition.answers_to(&id))
                {
                    let mut seen = HashSet::new();
                    let keys = keys
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|item| parse(item.as_str()?))
                        .filter(|key| seen.insert(*key))
                        .collect::<Vec<_>>();
                    map.change(&definition.id, &keys);
                }
            }
            Some((name.to_owned(), changes(definitions, &base, &map)))
        })
        .collect()
}

#[derive(Deserialize)]
struct KeyFile {
    version: u32,
    keys: Ordered<Value>,
}

#[derive(Default)]
struct ImportSummary {
    imported: usize,
    unknown: usize,
    duplicate: usize,
    unreadable: usize,
}

impl ImportSummary {
    fn message(&self) -> String {
        let mut skipped = Vec::new();
        for (count, label) in [
            (self.unknown, "unknown command"),
            (self.duplicate, "duplicate key"),
            (self.unreadable, "unreadable key"),
        ] {
            if count > 0 {
                skipped.push(format!(
                    "{count} {label}{}",
                    if count == 1 { "" } else { "s" }
                ));
            }
        }
        let imported = format!(
            "Imported {} command{}",
            self.imported,
            if self.imported == 1 { "" } else { "s" }
        );
        if skipped.is_empty() {
            format!("{imported}.")
        } else {
            format!("{imported}; {} skipped.", skipped.join(", "))
        }
    }
}

impl Application {
    pub fn shortcuts(&self) -> Vec<(&str, &str, Vec<String>)> {
        self.catalogue
            .definitions()
            .iter()
            .map(|definition| {
                let keys = self
                    .keymap
                    .keys_of(&definition.id)
                    .iter()
                    .map(ToString::to_string)
                    .collect();
                (
                    definition.id.as_str(),
                    definition.description.as_str(),
                    keys,
                )
            })
            .collect()
    }

    pub fn change_shortcut(&mut self, id: &str, key: Option<KeyCombination>) -> Result<(), String> {
        let id = self
            .catalogue
            .definitions()
            .iter()
            .find(|definition| definition.answers_to(id))
            .ok_or("This command is unavailable.")?
            .id
            .clone();
        let mut keys = Vec::new();
        let mut status = format!("Cleared the shortcuts for {id}.");
        if let Some(key) = key {
            keys.extend_from_slice(self.keymap.keys_of(&id));
            if keys.contains(&key) {
                return Ok(());
            }
            status = match self.keymap.command_for(key) {
                Some(previous) => format!("Moved {key} from {previous} to {id}."),
                None => format!("Assigned {key} to {id}."),
            };
            keys.push(key);
        }
        let mut map = self.keymap.clone();
        map.change(&id, &keys);
        self.install_shortcuts(map, status);
        Ok(())
    }

    /// Resets all shortcuts in the active preset back to their default bindings.
    pub fn reset_shortcuts(&mut self) {
        let definitions = self.catalogue.definitions();
        let name = &self.preferences.key_preset;
        let map = KeyMap::from_preset(definitions, preset(name));
        let status = format!("Reset shortcuts for the {name} preset.");
        self.install_shortcuts(map, status);
    }

    pub fn import_shortcuts(&mut self, path: &Path) -> Result<(), String> {
        let bytes = std::fs::read(path).map_err(|error| format!("Opening key file: {error}"))?;
        let file: KeyFile = serde_json::from_slice(crate::preferences::without_bom(&bytes))
            .map_err(|_| "This isn't a readable .dawsukeys file.")?;
        if file.version != 1 {
            return Err(format!(
                "This editor reads .dawsukeys version 1. The file is version {}.",
                file.version
            ));
        }

        let definitions = self.catalogue.definitions();
        let mut map = KeyMap::from_preset(definitions, preset(&self.preferences.key_preset));
        let mut summary = ImportSummary::default();
        let mut commands = HashSet::new();
        let mut used = HashSet::new();
        for (id, stored) in file.keys.0 {
            let Some(definition) = definitions
                .iter()
                .find(|definition| definition.answers_to(&id))
            else {
                summary.unknown += 1;
                continue;
            };
            if !commands.insert(&definition.id) {
                summary.unknown += 1;
                continue;
            }
            summary.imported += 1;
            let mut keys = Vec::new();
            for entry in stored.as_array().into_iter().flatten() {
                let Some(key) = entry.as_str().and_then(parse) else {
                    summary.unreadable += 1;
                    continue;
                };
                if used.insert(key) {
                    keys.push(key);
                } else if !keys.contains(&key) {
                    summary.duplicate += 1;
                }
            }
            map.change(&definition.id, &keys);
        }
        self.install_shortcuts(map, summary.message());
        Ok(())
    }

    pub fn export_shortcuts(&mut self, path: &Path) -> Result<(), String> {
        #[derive(Serialize)]
        struct Export<'a> {
            version: u32,
            preset: &'a str,
            keys: BTreeMap<&'a str, Vec<String>>,
        }
        let keys = self
            .catalogue
            .definitions()
            .iter()
            .map(|definition| {
                (
                    definition.id.as_str(),
                    self.keymap
                        .keys_of(&definition.id)
                        .iter()
                        .copied()
                        .map(stored)
                        .collect(),
                )
            })
            .collect();
        let output = Export {
            version: 1,
            preset: if self.preferences.key_preset == "osu!" {
                "Osu"
            } else {
                "Cubase"
            },
            keys,
        };
        let bytes = serde_json::to_vec_pretty(&output).map_err(|error| error.to_string())?;
        let exporting = |error: dawsu_foundation::Error| format!("Exporting shortcuts: {error}");
        // The save dialog already asked before replacing a file; keep the old one as a
        // backup in the same folder and count as map saves.
        self.status = match std::fs::read(path) {
            Ok(old) => {
                let backups = BackupPolicy::new(self.data_root.join("backups"), 20);
                if safe_write::replace(path, &old, &bytes, &backups).map_err(exporting)?
                    == WriteOutcome::ChangedOnDisk
                {
                    return Err("The shortcut file changed while exporting. Nothing was written.".into());
                }
                format!("Exported shortcuts to {} and kept a backup of the old file", path.display())
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                safe_write::create(path, &bytes).map_err(exporting)?;
                format!("Exported shortcuts to {}", path.display())
            }
            Err(error) => return Err(format!("Exporting shortcuts: {error}")),
        };
        self.bump_view_revision();
        Ok(())
    }

    fn install_shortcuts(&mut self, map: KeyMap, status: String) {
        let definitions = self.catalogue.definitions();
        let name = &self.preferences.key_preset;
        let base = KeyMap::from_preset(definitions, preset(name));
        let overrides = changes(definitions, &base, &map);
        if overrides.is_empty() {
            self.preferences.key_overrides.remove(name);
        } else {
            self.preferences
                .key_overrides
                .insert(name.clone(), overrides);
        }
        self.keymap = map;
        self.status = status;
        self.save_preferences();
        self.bump_view_revision();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use dawsu_test_support::Scratch;

    #[test]
    fn shortcut_v1_uses_aliases_keeps_first_conflict_and_roundtrips_active_preset() {
        let fixture = Scratch::new("key-files");
        let input = fixture.path("copied-csharp.dawsukeys");
        let output = fixture.path("rust.dawsukeys");
        let bytes = br#"{
          "version":1, "preset":"Osu",
          "keys":{"undo":["Control,Shift+D1","None+F5"],"play":["Control,Shift+D1","unknown",4],
                  "moveLeft":["Alt+OemComma"],"future":["None+F8"]}
        }"#;
        std::fs::write(&input, bytes).unwrap();
        let mut app = Application::headless();
        app.import_shortcuts(&input).unwrap();
        let chord = KeyCombination::new(Key::D1, Mods::CTRL | Mods::SHIFT);
        assert_eq!(app.keymap.command_for(chord), Some("undo"));
        assert_eq!(
            app.keymap
                .command_for(KeyCombination::new(Key::Comma, Mods::ALT)),
            Some("nudgeLeft")
        );
        assert_eq!(
            app.keymap
                .command_for(KeyCombination::new(Key::Y, Mods::CTRL)),
            Some("redo")
        );
        assert!(app.keymap.keys_of("play").is_empty());
        assert!(
            app.status
                .contains("1 unknown command, 1 duplicate key, 2 unreadable keys")
        );
        assert_eq!(std::fs::read(&input).unwrap(), bytes);
        app.change_shortcut("play", Some(chord)).unwrap();
        assert_eq!(app.keymap.command_for(chord), Some("play"));
        assert_eq!(
            app.keymap.keys_of("undo"),
            &[KeyCombination::plain(Key::F5)]
        );

        app.choose_key_preset("osu!");
        assert_ne!(app.keymap.command_for(chord), Some("play"));
        app.choose_key_preset("DAW");
        assert_eq!(app.keymap.command_for(chord), Some("play"));

        app.export_shortcuts(&output).unwrap();
        let exported = std::fs::read(&output).unwrap();
        let value: Value = serde_json::from_slice(&exported).unwrap();
        assert_eq!(value["version"], 1);
        assert_eq!(value["preset"], "Cubase");
        assert_eq!(value["keys"]["play"][0], "Control,Shift+D1");
        // Exporting over the file (after the save dialog's Replace) replaces it and keeps
        // the old file as a backup, the way map saves do.
        app.data_root = fixture.path("data");
        app.change_shortcut("play", Some(KeyCombination::plain(Key::F8))).unwrap();
        app.export_shortcuts(&output).unwrap();
        let replaced: Value = serde_json::from_slice(&std::fs::read(&output).unwrap()).unwrap();
        assert_eq!(replaced["keys"]["play"][1], "None+F8");
        let folder = std::fs::read_dir(fixture.path("data").join("backups")).unwrap().next().unwrap().unwrap().path();
        let backup = std::fs::read_dir(folder).unwrap().next().unwrap().unwrap().path();
        assert_eq!(std::fs::read(backup).unwrap(), exported);
        let mut reopened = Application::headless();
        reopened.import_shortcuts(&output).unwrap();
        assert_eq!(reopened.shortcuts(), app.shortcuts());
        let unsupported = fixture.path("future.dawsukeys");
        std::fs::write(&unsupported, br#"{"version":2,"keys":{}}"#).unwrap();
        assert!(reopened.import_shortcuts(&unsupported).is_err());
        assert_eq!(reopened.shortcuts(), app.shortcuts());
    }
}
