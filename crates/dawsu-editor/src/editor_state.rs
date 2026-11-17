//! One editor's edit history and selection, kept in step (Responsibilities 1-3): undo, redo and
//! jumps in the history drop items that no longer exist from the selection, so no composer has to
//! remember to.

use std::hash::Hash;

use crate::history::EditHistory;
use crate::selection::Selection;

/// A document whose items a selection can point at. The composer's document implements it.
pub trait ItemDocument<T> {
    fn contains_item(&self, item: &T) -> bool;
    /// Every item, for Select all.
    fn all_items(&self) -> Vec<T>;
}

pub struct EditorState<D: Clone, T: Clone + Eq + Hash> {
    history: EditHistory<D>,
    selection: Selection<T>,
}

impl<D: Clone + ItemDocument<T>, T: Clone + Eq + Hash> EditorState<D, T> {
    pub fn new(doc: D) -> Self {
        EditorState {
            history: EditHistory::new(doc),
            selection: Selection::new(),
        }
    }

    pub fn history(&self) -> &EditHistory<D> {
        &self.history
    }

    /// For transactions (begin, edit, commit, cancel). Undo, redo and jumps go through this type
    /// instead, so the selection follows them.
    pub fn history_mut(&mut self) -> &mut EditHistory<D> {
        &mut self.history
    }

    pub fn selection(&self) -> &Selection<T> {
        &self.selection
    }

    pub fn selection_mut(&mut self) -> &mut Selection<T> {
        &mut self.selection
    }

    pub fn undo(&mut self) -> bool {
        let moved = self.history.undo();
        if moved {
            self.drop_missing();
        }
        moved
    }

    pub fn redo(&mut self) -> bool {
        let moved = self.history.redo();
        if moved {
            self.drop_missing();
        }
        moved
    }

    /// Jumps to a position in the "recent edits" list (see `EditHistory::go_to`).
    pub fn go_to(&mut self, position: usize) {
        self.history.go_to(position);
        self.drop_missing();
    }

    pub fn select_all(&mut self) {
        let items = self.history.document().all_items();
        self.selection.replace(items);
    }

    pub fn deselect(&mut self) {
        self.selection.clear();
    }

    fn drop_missing(&mut self) {
        let doc = self.history.document();
        self.selection.retain(|i| doc.contains_item(i));
    }
}
