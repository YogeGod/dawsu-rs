//! Locators, the cycle switch and bookmarks as plain times, with no playback (Responsibility 5).
//! Starting, stopping and seeking audio belong to `dawsu-application`.

use dawsu_foundation::SongTime;

/// The left and right locators and the cycle switch.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Locators {
    pub left: Option<SongTime>,
    pub right: Option<SongTime>,
    pub cycle_on: bool,
}

impl Locators {
    pub fn set_left(&mut self, at: SongTime) {
        self.left = Some(at);
    }

    pub fn set_right(&mut self, at: SongTime) {
        self.right = Some(at);
    }

    pub fn toggle_cycle(&mut self) {
        self.cycle_on = !self.cycle_on;
    }

    /// Locators to a range (the selected hits, or the Range tool's range): left at its start, right at its end.
    pub fn to_range(&mut self, from: SongTime, to: SongTime) {
        let (a, b) = if to.ms() < from.ms() {
            (to, from)
        } else {
            (from, to)
        };
        self.left = Some(a);
        self.right = Some(b);
    }

    /// "Play locator range": locators to the range and cycle on. Returns where playback starts (L).
    pub fn loop_range(&mut self, from: SongTime, to: SongTime) -> SongTime {
        self.to_range(from, to);
        self.cycle_on = true;
        self.left.expect("just set")
    }

    /// True when the right locator lies before the left one; the old arrange view draws this as a
    /// different band. What playback does then is decided by `dawsu-application`, not here.
    pub fn is_reversed(&self) -> bool {
        matches!((self.left, self.right), (Some(l), Some(r)) if r.ms() < l.ms())
    }
}

/// One bookmark: a time and an optional label.
#[derive(Debug, Clone, PartialEq)]
pub struct Bookmark {
    pub time: SongTime,
    pub label: Option<String>,
}

/// Bookmarks sorted by time, at most one per time.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BookmarkList {
    items: Vec<Bookmark>,
}

/// How close (in ms) a time must be to a bookmark to count as "at" it.
pub const BOOKMARK_TOLERANCE_MS: f64 = 1.0;

impl BookmarkList {
    pub fn new(times: impl IntoIterator<Item = SongTime>) -> BookmarkList {
        let mut list = BookmarkList::default();
        for t in times {
            list.add(t, None);
        }
        list
    }

    pub fn items(&self) -> &[Bookmark] {
        &self.items
    }

    /// Adds a bookmark at `time`, or relabels the one already there. Returns false when nothing changed.
    pub fn add(&mut self, time: SongTime, label: Option<String>) -> bool {
        if let Some(existing) = self
            .items
            .iter_mut()
            .find(|b| (b.time.ms() - time.ms()).abs() < BOOKMARK_TOLERANCE_MS)
        {
            if label.is_some() && existing.label != label {
                existing.label = label;
                return true;
            }
            return false;
        }
        let at = self.items.partition_point(|b| b.time.ms() < time.ms());
        self.items.insert(at, Bookmark { time, label });
        true
    }

    /// Removes the bookmark at `time`. Returns false when there is none.
    pub fn remove_at(&mut self, time: SongTime) -> bool {
        let before = self.items.len();
        self.items
            .retain(|b| (b.time.ms() - time.ms()).abs() >= BOOKMARK_TOLERANCE_MS);
        self.items.len() != before
    }

    /// The last bookmark before `time`.
    pub fn previous(&self, time: SongTime) -> Option<SongTime> {
        self.items
            .iter()
            .rev()
            .find(|b| b.time.ms() < time.ms() - BOOKMARK_TOLERANCE_MS)
            .map(|b| b.time)
    }

    /// The first bookmark after `time`.
    pub fn next(&self, time: SongTime) -> Option<SongTime> {
        self.items
            .iter()
            .find(|b| b.time.ms() > time.ms() + BOOKMARK_TOLERANCE_MS)
            .map(|b| b.time)
    }
}
