//! The kernel's own key values (Responsibility 10). The UI layer maps its toolkit's keys onto them.

use std::fmt;

/// A key. Names follow what is printed on a US keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Key {
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
}

impl Key {
    /// The digit key 0-9 above the letters.
    pub fn digit(n: u32) -> Option<Key> {
        const DIGITS: [Key; 10] = [
            Key::D0,
            Key::D1,
            Key::D2,
            Key::D3,
            Key::D4,
            Key::D5,
            Key::D6,
            Key::D7,
            Key::D8,
            Key::D9,
        ];
        DIGITS.get(n as usize).copied()
    }

    /// The key as printed on the keyboard: "1" not "D1", "," not "Comma" (the old `EditorCommand.Name`).
    pub fn name(self) -> String {
        use Key::*;
        let fixed = match self {
            NumPadDivide => "Numpad /",
            NumPadMultiply => "Numpad *",
            NumPadSubtract => "Numpad -",
            NumPadAdd => "Numpad +",
            NumPadDecimal => "Numpad .",
            NumPadEnter => "Numpad Enter",
            Period => ".",
            Minus => "-",
            Equals => "=",
            Slash => "/",
            Backslash => "\\",
            Comma => ",",
            Semicolon => ";",
            Quote => "'",
            OpenBracket => "[",
            CloseBracket => "]",
            Grave => "`",
            Escape => "Esc",
            Delete => "Del",
            _ => "",
        };
        if !fixed.is_empty() {
            return fixed.to_owned();
        }
        let debug = format!("{self:?}");
        if let Some(n) = debug.strip_prefix("NumPad") {
            return format!("Numpad {n}");
        }
        if debug.len() == 2 && debug.starts_with('D') {
            return debug[1..].to_owned();
        }
        debug
    }
}

/// Modifier keys, as a small bit set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, PartialOrd, Ord)]
pub struct Mods(u8);

impl Mods {
    pub const NONE: Mods = Mods(0);
    pub const CTRL: Mods = Mods(1);
    pub const SHIFT: Mods = Mods(2);
    pub const ALT: Mods = Mods(4);

    pub const fn union(self, other: Mods) -> Mods {
        Mods(self.0 | other.0)
    }

    pub const fn contains(self, other: Mods) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn ctrl(self) -> bool {
        self.contains(Mods::CTRL)
    }

    pub const fn shift(self) -> bool {
        self.contains(Mods::SHIFT)
    }

    pub const fn alt(self) -> bool {
        self.contains(Mods::ALT)
    }
}

impl std::ops::BitOr for Mods {
    type Output = Mods;
    fn bitor(self, rhs: Mods) -> Mods {
        self.union(rhs)
    }
}

/// A key with its modifiers, as a preset or a key change names it. Commands are found by these.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct KeyCombination {
    pub key: Key,
    pub mods: Mods,
}

impl KeyCombination {
    pub const fn new(key: Key, mods: Mods) -> KeyCombination {
        KeyCombination { key, mods }
    }

    pub const fn plain(key: Key) -> KeyCombination {
        KeyCombination {
            key,
            mods: Mods::NONE,
        }
    }
}

impl fmt::Display for KeyCombination {
    /// "Ctrl+Shift+Z", the order the old help used.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.mods.ctrl() {
            f.write_str("Ctrl+")?;
        }
        if self.mods.shift() {
            f.write_str("Shift+")?;
        }
        if self.mods.alt() {
            f.write_str("Alt+")?;
        }
        f.write_str(&self.key.name())
    }
}

/// A key press as the UI passes it in: the key the layout produced, the key at that physical
/// position on a US layout, and the modifiers (ADR 0012: the live-input mode plays by position).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyChord {
    pub key: Key,
    pub physical: Key,
    pub mods: Mods,
}

impl KeyChord {
    /// A press where the key and its physical position agree (a US layout).
    pub const fn new(key: Key, mods: Mods) -> KeyChord {
        KeyChord {
            key,
            physical: key,
            mods,
        }
    }

    /// What commands are matched by: the key and modifiers, not the physical position.
    pub const fn combination(self) -> KeyCombination {
        KeyCombination {
            key: self.key,
            mods: self.mods,
        }
    }
}
