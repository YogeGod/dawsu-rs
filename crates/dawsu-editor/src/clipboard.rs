//! Copied or cut items with their times relative to the first one (Responsibility 3a). Cut is a
//! copy plus a delete the editor runs through its edit history.

use dawsu_foundation::SongTime;

#[derive(Debug, Clone)]
pub struct Clipboard<T: Clone> {
    /// (offset from the first item in ms, item), in time order.
    entries: Vec<(f64, T)>,
}

impl<T: Clone> Default for Clipboard<T> {
    fn default() -> Self {
        Clipboard {
            entries: Vec::new(),
        }
    }
}

impl<T: Clone> Clipboard<T> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Replaces the clipboard with `items`. Copying nothing leaves the clipboard as it was.
    pub fn copy(&mut self, items: impl IntoIterator<Item = (SongTime, T)>) {
        let mut items: Vec<(SongTime, T)> = items.into_iter().collect();
        if items.is_empty() {
            return;
        }
        items.sort_by(|a, b| a.0.ms().total_cmp(&b.0.ms()));
        let first = items[0].0.ms();
        self.entries = items
            .into_iter()
            .map(|(t, item)| (t.ms() - first, item))
            .collect();
    }

    /// The items placed so the first lands at `at` (the playhead); the others keep their spacing.
    pub fn paste_at(&self, at: SongTime) -> Vec<(SongTime, T)> {
        self.entries
            .iter()
            .map(|(offset, item)| (SongTime::from_ms(at.ms() + offset), item.clone()))
            .collect()
    }

    /// The time from the first item to the last.
    pub fn span_ms(&self) -> f64 {
        self.entries.last().map_or(0.0, |(offset, _)| *offset)
    }
}
