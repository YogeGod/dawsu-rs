//! The selection of one editor (Responsibility 3). Selection changes are not undo steps.

use std::collections::HashSet;
use std::hash::Hash;

#[derive(Debug, Clone)]
pub struct Selection<T: Clone + Eq + Hash> {
    items: HashSet<T>,
    /// Selection order, so a removed primary falls back to the last item still selected.
    order: Vec<T>,
    primary: Option<T>,
    version: u64,
}

impl<T: Clone + Eq + Hash> Default for Selection<T> {
    fn default() -> Self {
        Selection {
            items: HashSet::new(),
            order: Vec::new(),
            primary: None,
            version: 0,
        }
    }
}

impl<T: Clone + Eq + Hash> Selection<T> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn items(&self) -> &HashSet<T> {
        &self.items
    }

    /// The last clicked item: the anchor for Shift ranges.
    pub fn primary(&self) -> Option<&T> {
        self.primary.as_ref()
    }

    /// Bumps on every change; views compare it.
    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn contains(&self, item: &T) -> bool {
        self.items.contains(item)
    }

    /// Selects exactly `items`; the first becomes the primary.
    pub fn replace(&mut self, items: impl IntoIterator<Item = T>) {
        self.items.clear();
        self.order.clear();
        self.primary = None;
        for item in items {
            if self.primary.is_none() {
                self.primary = Some(item.clone());
            }
            if self.items.insert(item.clone()) {
                self.order.push(item);
            }
        }
        self.version += 1;
    }

    /// Adds an item and makes it the primary.
    pub fn add(&mut self, item: T) {
        if self.items.insert(item.clone()) {
            self.order.push(item.clone());
        }
        self.primary = Some(item);
        self.version += 1;
    }

    pub fn remove(&mut self, item: &T) {
        if self.items.remove(item) {
            self.order.retain(|i| i != item);
            self.fix_primary();
            self.version += 1;
        }
    }

    pub fn toggle(&mut self, item: T) {
        if self.items.contains(&item) {
            self.remove(&item)
        } else {
            self.add(item)
        }
    }

    pub fn clear(&mut self) {
        if !self.items.is_empty() || self.primary.is_some() {
            self.items.clear();
            self.order.clear();
            self.primary = None;
            self.version += 1;
        }
    }

    /// Drops items that no longer exist, for example after an undo or redo removed them.
    pub fn retain(&mut self, still_exists: impl Fn(&T) -> bool) {
        let before = self.items.len();
        self.items.retain(|i| still_exists(i));
        self.order.retain(|i| still_exists(i));
        self.fix_primary();
        if self.items.len() != before {
            self.version += 1;
        }
    }

    /// The items in selection order and the primary, to put back with `restore`.
    pub fn snapshot(&self) -> (Vec<T>, Option<T>) {
        (self.order.clone(), self.primary.clone())
    }

    /// Puts back a `snapshot` (a cancelled stroke).
    pub fn restore(&mut self, (order, primary): (Vec<T>, Option<T>)) {
        self.items = order.iter().cloned().collect();
        self.order = order;
        self.primary = primary;
        self.version += 1;
    }

    /// A primary that left the selection passes to the last item still selected.
    fn fix_primary(&mut self) {
        if self
            .primary
            .as_ref()
            .is_some_and(|p| !self.items.contains(p))
        {
            self.primary = self.order.last().cloned();
        }
    }
}
