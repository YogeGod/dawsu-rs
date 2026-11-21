//! One edit history per document (Responsibilities 1, 2, 13, 13a; invariants 1-5).
//!
//! The history owns the document and gives the editor mutable access for edits inside a
//! transaction (invariant 1). Undo steps are whole-document copies (Responsibility 13a,
//! Yoge 2026-11-15): undo or redo swaps the step's other-side snapshot with the current
//! document. External metadata can be refreshed across all snapshots without adding an edit.


use std::fmt;

/// A failed edit inside a transaction. The document is back as it was before the transaction began.
#[derive(Debug, Clone, PartialEq)]
pub enum EditError<E> {
    /// `edit` was called with no transaction open.
    NoOpenTransaction,
    /// The change itself failed; the whole transaction was rolled back (invariant 2).
    Failed(E),
}

impl<E: fmt::Display> fmt::Display for EditError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EditError::NoOpenTransaction => write!(f, "no edit transaction is open"),
            EditError::Failed(e) => write!(f, "the edit failed and was rolled back: {e}"),
        }
    }
}

impl<E: fmt::Debug + fmt::Display> std::error::Error for EditError<E> {}

struct Step<D> {
    label: String,
    affects_save: bool,
    /// The document on the other side of this step: before it while the step is done, after it
    /// while the step is undone.
    other: D,
}

struct OpenTransaction<D> {
    label: String,
    before: D,
    affects_save: bool,
    edited: bool,
    depth: u32,
}

/// The edit history of one document.
pub struct EditHistory<D: Clone> {
    doc: D,
    done: Vec<Step<D>>,
    /// Undone steps, the most recently undone last.
    undone: Vec<Step<D>>,
    /// The position at the last save; `None` once that point can no longer be reached.
    clean: Option<usize>,
    open: Option<OpenTransaction<D>>,
    version: u64,
}

impl<D: Clone> EditHistory<D> {
    /// A history over a freshly opened document, which counts as saved.
    pub fn new(doc: D) -> Self {
        EditHistory {
            doc,
            done: Vec::new(),
            undone: Vec::new(),
            clean: Some(0),
            open: None,
            version: 0,
        }
    }

    pub fn document(&self) -> &D {
        &self.doc
    }

    /// Bumps on every change of the document or the history; views compare it each frame.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// Opens a transaction, or joins the open one (a nested begin keeps the outer label).
    pub fn begin(&mut self, label: &str) {
        match &mut self.open {
            Some(open) => open.depth += 1,
            None => {
                self.open = Some(OpenTransaction {
                    label: label.to_owned(),
                    before: self.doc.clone(),
                    affects_save: false,
                    edited: false,
                    depth: 1,
                })
            }
        }
    }

    pub fn is_transaction_open(&self) -> bool {
        self.open.is_some()
    }

    /// Changes the document inside the open transaction. The change is visible at once (a drag's
    /// live preview). If `change` fails, the whole transaction is rolled back and closed.
    /// `affects_save` is false for view-only changes such as lane order.
    pub fn edit<T, E>(
        &mut self,
        affects_save: bool,
        change: impl FnOnce(&mut D) -> Result<T, E>,
    ) -> Result<T, EditError<E>> {
        let Some(open) = &mut self.open else {
            return Err(EditError::NoOpenTransaction);
        };
        open.edited = true;
        open.affects_save |= affects_save;
        self.version += 1;
        match change(&mut self.doc) {
            Ok(value) => Ok(value),
            Err(e) => {
                let open = self.open.take().expect("checked above");
                self.doc = open.before;
                Err(EditError::Failed(e))
            }
        }
    }

    /// One change as its own undo step: begin, edit, commit.
    pub fn apply<T, E>(
        &mut self,
        label: &str,
        affects_save: bool,
        change: impl FnOnce(&mut D) -> Result<T, E>,
    ) -> Result<T, EditError<E>> {
        self.begin(label);
        let value = self.edit(affects_save, change)?;
        self.commit();
        Ok(value)
    }

    /// Ends the innermost begin. The outermost commit turns the transaction into one undo step;
    /// a transaction that changed nothing adds no step. A new step clears the redo list (invariant 4).
    pub fn commit(&mut self) {
        let Some(open) = &mut self.open else { return };
        if open.depth > 1 {
            open.depth -= 1;
            return;
        }
        let open = self.open.take().expect("checked above");
        if !open.edited {
            return;
        }
        if self.clean.is_some_and(|c| c > self.done.len()) {
            self.clean = None;
        }
        self.undone.clear();
        self.done.push(Step {
            label: open.label,
            affects_save: open.affects_save,
            other: open.before,
        });
        self.version += 1;
    }

    /// Rolls the whole open transaction back (Esc, focus lost): the document and the history are
    /// exactly as before `begin` (invariant 2).
    pub fn cancel(&mut self) {
        if let Some(open) = self.open.take()
            && open.edited {
                self.doc = open.before;
                self.version += 1;
            }
    }

    pub fn can_undo(&self) -> bool {
        self.open.is_none() && !self.done.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        self.open.is_none() && !self.undone.is_empty()
    }

    /// Does nothing while a transaction is open (invariant 3).
    pub fn undo(&mut self) -> bool {
        if !self.can_undo() {
            return false;
        }
        let mut step = self.done.pop().expect("checked");
        std::mem::swap(&mut self.doc, &mut step.other);
        self.undone.push(step);
        self.version += 1;
        true
    }

    /// Does nothing while a transaction is open (invariant 3).
    pub fn redo(&mut self) -> bool {
        if !self.can_redo() {
            return false;
        }
        let mut step = self.undone.pop().expect("checked");
        std::mem::swap(&mut self.doc, &mut step.other);
        self.done.push(step);
        self.version += 1;
        true
    }

    /// Jumps to a position in the "recent edits" list (0 = before the first step), clamped.
    /// Does nothing while a transaction is open (invariant 3).
    pub fn go_to(&mut self, position: usize) {
        if self.open.is_some() {
            return;
        }
        let target = position.min(self.done.len() + self.undone.len());
        while self.done.len() > target {
            self.undo();
        }
        while self.done.len() < target {
            self.redo();
        }
    }

    /// Refreshes external metadata in the current document and all retained snapshots.
    /// The change must preserve undoable and saved document content.
    pub fn update_external(&mut self, update: impl Fn(&mut D)) {
        update(&mut self.doc);
        for step in self.done.iter_mut().chain(&mut self.undone) {
            update(&mut step.other);
        }
        if let Some(open) = &mut self.open {
            update(&mut open.before);
        }
        self.version += 1;
    }

    /// Every state undo or redo can reach, the current one first, then the done and undone
    /// steps' and an open transaction's. Read-only.
    pub fn states(&self) -> impl Iterator<Item = &D> {
        std::iter::once(&self.doc)
            .chain(self.done.iter().chain(&self.undone).map(|step| &step.other))
            .chain(self.open.iter().map(|open| &open.before))
    }

    /// The number of done steps.
    pub fn position(&self) -> usize {
        self.done.len()
    }

    /// Step labels for "recent edits": the done ones, then the undone ones, oldest first.
    pub fn steps(&self) -> Vec<&str> {
        self.done
            .iter()
            .chain(self.undone.iter().rev())
            .map(|s| s.label.as_str())
            .collect()
    }

    /// After a successful save. The steps stay, so undo still reaches edits made before the save
    /// (Responsibility 13).
    pub fn mark_clean(&mut self) {
        self.clean = Some(self.done.len());
        self.version += 1;
    }

    /// True only when a step that affects the saved file lies between the last save and now
    /// (Responsibility 2, invariant 5). An open transaction that changed the file counts too.
    pub fn is_dirty(&self) -> bool {
        if self.open.as_ref().is_some_and(|o| o.affects_save) {
            return true;
        }
        let Some(clean) = self.clean else { return true };
        let now = self.done.len();
        (clean.min(now)..clean.max(now)).any(|i| self.step_at(i).affects_save)
    }

    /// Step `i` of the whole list (done, then undone oldest first).
    fn step_at(&self, i: usize) -> &Step<D> {
        if i < self.done.len() {
            &self.done[i]
        } else {
            &self.undone[self.undone.len() - 1 - (i - self.done.len())]
        }
    }
}
