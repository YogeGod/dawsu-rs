//! The selection of one editor (Responsibility 3). Selection changes are not undo steps.

use std::collections::HashSet;
use std::hash::Hash;

#[derive(Debug, Clone)]
pub struct Selection<T: Clone + Eq + Hash> {
    items: HashSet<T>,
    primary: Option<T>,
    version: u64,
}

impl<T: Clone + Eq + Hash> Default for Selection<T> {
    fn default() -> Self {
        Selection {
            items: HashSet::new(),
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
        self.primary = None;
        for item in items {
            if self.primary.is_none() {
                self.primary = Some(item.clone());
            }
            self.items.insert(item);
        }
        self.version += 1;
    }

    /// Adds an item and makes it the primary.
    pub fn add(&mut self, item: T) {
        self.items.insert(item.clone());
        self.primary = Some(item);
        self.version += 1;
    }

    pub fn remove(&mut self, item: &T) {
        if self.items.remove(item) {
            if self.primary.as_ref() == Some(item) {
                self.primary = None;
            }
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
            self.primary = None;
            self.version += 1;
        }
    }

    /// Drops items that no longer exist, for example after an undo or redo removed them.
    pub fn retain(&mut self, still_exists: impl Fn(&T) -> bool) {
        let before = self.items.len();
        self.items.retain(|i| still_exists(i));
        if self
            .primary
            .as_ref()
            .is_some_and(|p| !self.items.contains(p))
        {
            self.primary = None;
        }
        if self.items.len() != before {
            self.version += 1;
        }
    }
}
