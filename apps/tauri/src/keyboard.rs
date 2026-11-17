use dawsu_editor::keys::{Key, KeyChord, Mods};

pub(super) fn chord(key: &str, code: &str, ctrl: bool, shift: bool, alt: bool) -> Option<KeyChord> {
    let numpad = code.strip_prefix("Numpad").and_then(numpad_key);
    let physical = numpad.or_else(|| {
        let label = code
            .strip_prefix("Key")
            .or_else(|| code.strip_prefix("Digit"))
            .unwrap_or(code);
        key_value(label)
    });
    // A shifted digit can produce a symbol that the catalogue doesn't name.
    // Keep the letter produced by the layout when the event does carry one.
    let key = numpad.or_else(|| key_value(key)).or(physical)?;
    let mut mods = Mods::NONE;
    for (pressed, flag) in [(ctrl, Mods::CTRL), (shift, Mods::SHIFT), (alt, Mods::ALT)] {
        if pressed {
            mods = mods.union(flag);
        }
    }
    Some(KeyChord {
        key,
        physical: physical.unwrap_or(key),
        mods,
    })
}

fn key_value(value: &str) -> Option<Key> {
    use Key::*;
    const LETTERS: [Key; 26] = [
        A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z,
    ];
    if let [byte] = value.as_bytes() {
        let byte = byte.to_ascii_uppercase();
        if byte.is_ascii_uppercase() {
            return Some(LETTERS[usize::from(byte - b'A')]);
        }
        if byte.is_ascii_digit() {
            return Key::digit(u32::from(byte - b'0'));
        }
    }
    Some(match value {
        "ArrowLeft" => Left,
        "ArrowRight" => Right,
        "ArrowUp" => Up,
        "ArrowDown" => Down,
        " " | "Space" => Space,
        "Enter" => Enter,
        "Escape" => Escape,
        "Backspace" => Backspace,
        "Delete" => Delete,
        "Home" => Home,
        "End" => End,
        "PageUp" => PageUp,
        "PageDown" => PageDown,
        "Insert" => Insert,
        "Tab" => Tab,
        "F1" => F1,
        "F2" => F2,
        "F3" => F3,
        "F4" => F4,
        "F5" => F5,
        "F6" => F6,
        "F7" => F7,
        "F8" => F8,
        "F9" => F9,
        "F10" => F10,
        "F11" => F11,
        "F12" => F12,
        "[" | "BracketLeft" => OpenBracket,
        "]" | "BracketRight" => CloseBracket,
        "/" | "Slash" => Slash,
        "." | "Period" => Period,
        "," | "Comma" => Comma,
        "-" | "Minus" => Minus,
        "=" | "Equal" => Equals,
        "\\" | "Backslash" => Backslash,
        ";" | "Semicolon" => Semicolon,
        "'" | "Quote" => Quote,
        "\x60" | "Backquote" => Grave,
        _ => return None,
    })
}

fn numpad_key(value: &str) -> Option<Key> {
    use Key::*;
    Some(match value {
        "0" => NumPad0,
        "1" => NumPad1,
        "2" => NumPad2,
        "3" => NumPad3,
        "4" => NumPad4,
        "5" => NumPad5,
        "6" => NumPad6,
        "7" => NumPad7,
        "8" => NumPad8,
        "9" => NumPad9,
        "Divide" => NumPadDivide,
        "Multiply" => NumPadMultiply,
        "Subtract" => NumPadSubtract,
        "Add" => NumPadAdd,
        "Decimal" => NumPadDecimal,
        "Enter" => NumPadEnter,
        _ => return None,
    })
}
