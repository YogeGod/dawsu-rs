//! The commands the kernel's own features own: history, clipboard and selection, snap, zoom,
//! grid seeking, locators and bookmarks. Ids, descriptions, keys and Cubase commands are the
//! 3.1.x ones (`EditorCommands.cs`, `KeyBindings.cs` at `c81774e`). The tool commands are in
//! `tools`. The composers and `dawsu-application` declare their own commands beside their features
//! and bind every handler.

use crate::commands::{CommandDefinition, CommandFamily};
use crate::keys::{Key, KeyCombination as K, Mods};

const CTRL: Mods = Mods::CTRL;
const SHIFT: Mods = Mods::SHIFT;
const ALT: Mods = Mods::ALT;

fn def(id: &str, description: &str) -> CommandDefinition {
    CommandDefinition::new(id, description)
}

// History (Responsibilities 1-2).
pub fn undo() -> CommandDefinition {
    def("undo", "Undo")
        .keys(&[K::new(Key::Z, CTRL)])
        .cubase("Edit|Undo")
}
pub fn redo() -> CommandDefinition {
    def("redo", "Redo")
        .keys(&[K::new(Key::Y, CTRL), K::new(Key::Z, CTRL | SHIFT)])
        .cubase("Edit|Redo")
}

// Clipboard and selection (Responsibilities 3, 3a).
pub fn copy() -> CommandDefinition {
    def("copy", "Copy")
        .keys(&[K::new(Key::C, CTRL)])
        .cubase("Edit|Copy")
}
pub fn cut() -> CommandDefinition {
    def("cut", "Cut")
        .keys(&[K::new(Key::X, CTRL)])
        .cubase("Edit|Cut")
}
pub fn paste() -> CommandDefinition {
    def("paste", "Paste at the playhead")
        .keys(&[K::new(Key::V, CTRL)])
        .cubase("Edit|Paste")
}
pub fn duplicate() -> CommandDefinition {
    def("duplicate", "Duplicate the selection after itself")
        .keys(&[K::new(Key::D, CTRL)])
        .cubase("Edit|Duplicate")
}
pub fn delete() -> CommandDefinition {
    def("delete", "Delete the selection")
        .keys(&[K::plain(Key::Delete), K::plain(Key::Backspace)])
        .cubase("Edit|Delete")
}
pub fn select_all() -> CommandDefinition {
    def("selectAll", "Select all hits")
        .keys(&[K::new(Key::A, CTRL)])
        .cubase("Edit|Select All")
}
pub fn deselect() -> CommandDefinition {
    def("deselect", "Select nothing")
        .keys(&[K::new(Key::A, CTRL | SHIFT), K::plain(Key::Escape)])
        .cubase("Edit|Select None")
}

// Snap (Responsibilities 4, 4a).
pub fn snap_toggle() -> CommandDefinition {
    def("snap", "Snapping on / off")
        .keys(&[K::plain(Key::J)])
        .cubase("Edit|Snap On/Off")
}

fn snap_to(n: u32) -> CommandDefinition {
    let key = Key::digit(n).expect("snap family runs 1 to 9");
    def(&format!("snap{n}"), &format!("Snap to 1/{n}")).keys(&[K::new(key, SHIFT)])
}

/// "snap1" to "snap9": snap to 1/N, one handler taking N (CS-17).
pub const SNAP_TO: CommandFamily = CommandFamily {
    prefix: "snap",
    from: 1,
    to: 9,
    define: snap_to,
};

// Zoom and grid seeking (Responsibility 4).
pub fn zoom_in() -> CommandDefinition {
    def("zoomIn", "Zoom in")
        .keys(&[K::plain(Key::H)])
        .cubase("Zoom|Zoom In")
}
pub fn zoom_out() -> CommandDefinition {
    def("zoomOut", "Zoom out")
        .keys(&[K::plain(Key::G)])
        .cubase("Zoom|Zoom Out")
}
pub fn seek_back() -> CommandDefinition {
    def(
        "seekBack",
        "Playhead to the previous grid line (osu! keys: Left, wheel up)",
    )
    .osu(&[K::plain(Key::Left)])
}
pub fn seek_forward() -> CommandDefinition {
    def(
        "seekForward",
        "Playhead to the next grid line (osu! keys: Right, wheel down)",
    )
    .osu(&[K::plain(Key::Right)])
}
pub fn seek_back_far() -> CommandDefinition {
    def(
        "seekBackFar",
        "Playhead back four grid lines (osu! keys: Shift+Left, Shift+wheel up)",
    )
    .osu(&[K::new(Key::Left, SHIFT)])
}
pub fn seek_forward_far() -> CommandDefinition {
    def(
        "seekForwardFar",
        "Playhead forward four grid lines (osu! keys: Shift+Right, Shift+wheel down)",
    )
    .osu(&[K::new(Key::Right, SHIFT)])
}

// Locators (Responsibility 5).
pub fn cycle() -> CommandDefinition {
    def("cycle", "Cycle between the L and R locators")
        .keys(&[
            K::plain(Key::L),
            K::plain(Key::NumPadDivide),
            K::plain(Key::Slash),
        ])
        .cubase("Transport|Cycle")
}
pub fn locators_to_selection() -> CommandDefinition {
    def(
        "locatorsToSelection",
        "Locators to the selected hits or the Range tool's range",
    )
    .keys(&[K::plain(Key::P)])
    .cubase("Transport|Locators to Selection")
}
pub fn play_locator_range() -> CommandDefinition {
    def(
        "playLocatorRange",
        "Locators to the selection, cycle on, play from L",
    )
    .keys(&[K::new(Key::P, ALT)])
    .cubase("Transport|Loop Selection")
}
pub fn jump_left() -> CommandDefinition {
    def("jumpLeft", "Jump to the left locator")
        .keys(&[K::plain(Key::NumPad1)])
        .cubase("Transport|To Left Locator")
}
pub fn jump_right() -> CommandDefinition {
    def("jumpRight", "Jump to the right locator")
        .keys(&[K::plain(Key::NumPad2)])
        .cubase("Transport|To Right Locator")
}
pub fn set_left() -> CommandDefinition {
    def("setLeft", "Set the left locator at the playhead")
        .keys(&[K::new(Key::NumPad1, CTRL)])
        .cubase("Transport|Set Left Locator")
}
pub fn set_right() -> CommandDefinition {
    def("setRight", "Set the right locator at the playhead")
        .keys(&[K::new(Key::NumPad2, CTRL)])
        .cubase("Transport|Set Right Locator")
}

// Bookmarks (Responsibility 5).
pub fn add_bookmark() -> CommandDefinition {
    def("addBookmark", "Add a bookmark at the playhead")
        .keys(&[K::new(Key::B, CTRL)])
        .cubase("Transport|Insert Marker")
}
pub fn remove_bookmark() -> CommandDefinition {
    def("removeBookmark", "Remove the bookmark at the playhead")
        .keys(&[K::new(Key::B, CTRL | SHIFT)])
}
pub fn previous_bookmark() -> CommandDefinition {
    def("previousBookmark", "Previous bookmark")
        .keys(&[K::plain(Key::OpenBracket)])
        .cubase("Transport|Locate Previous Marker")
}
pub fn next_bookmark() -> CommandDefinition {
    def("nextBookmark", "Next bookmark")
        .keys(&[K::plain(Key::CloseBracket)])
        .cubase("Transport|Locate Next Marker")
}

/// Every plain command declared here (the snap family and the tool commands are separate).
pub fn all() -> Vec<CommandDefinition> {
    vec![
        undo(),
        redo(),
        copy(),
        cut(),
        paste(),
        duplicate(),
        delete(),
        select_all(),
        deselect(),
        snap_toggle(),
        zoom_in(),
        zoom_out(),
        seek_back(),
        seek_forward(),
        seek_back_far(),
        seek_forward_far(),
        cycle(),
        locators_to_selection(),
        play_locator_range(),
        jump_left(),
        jump_right(),
        set_left(),
        set_right(),
        add_bookmark(),
        remove_bookmark(),
        previous_bookmark(),
        next_bookmark(),
    ]
}
