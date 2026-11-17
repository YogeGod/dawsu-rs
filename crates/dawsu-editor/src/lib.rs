//! The editing kernel shared by the Hitsound composer, the Keysound composer and the node editor
//! (TDD: editor core): edit history with transactions, selection, the clipboard, timeline maths and
//! snap, locators and bookmarks, the mouse tools as state machines, and the command catalogue with
//! key resolution.
//!
//! It knows nothing about hitsounds, notes, audio, osu! files or the UI toolkit (Non-goals). It runs
//! on the UI's update thread only (invariant 11(a)): none of its types lock.

pub mod clipboard;
pub mod commands;
pub mod editing_commands;
pub mod editor_state;
pub mod history;
pub mod keys;
pub mod selection;
pub mod tempo;
pub mod time_axis;
pub mod tools;
pub mod transport;
pub mod volume_ramp;

pub use clipboard::Clipboard;
pub use editor_state::{EditorState, ItemDocument};
pub use history::{EditError, EditHistory};
pub use selection::Selection;
