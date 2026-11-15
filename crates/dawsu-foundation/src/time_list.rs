//! A list kept in time order that finds the last item at or before a time
//! (Responsibility 4), as timing points, hits and notes all need.

use crate::SongTime;

/// Something that sits at a song time.
pub trait Timed {
    fn time(&self) -> SongTime;
}

/// Items in time order; items at the same time keep the order they were given in.
#[derive(Debug, Clone, PartialEq)]
pub struct TimeOrderedList<T> {
    items: Vec<T>,
}

impl<T: Timed> TimeOrderedList<T> {
    /// Sorts the items by time; a stable sort keeps equal times in the given order.
    pub fn new(mut items: Vec<T>) -> TimeOrderedList<T> {
        items.sort_by(|a, b| a.time().ms().total_cmp(&b.time().ms()));
        TimeOrderedList { items }
    }

    /// The last item at or before `time`, or `None` when every item is later (TD-33).
    /// Of several items at the same time, the last one wins.
    pub fn last_at_or_before(&self, time: SongTime) -> Option<&T> {
        let after = self.items.partition_point(|item| item.time().ms().total_cmp(&time.ms()).is_le());
        after.checked_sub(1).map(|i| &self.items[i])
    }

    pub fn items(&self) -> &[T] {
        &self.items
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq)]
    struct Point(f64, &'static str);

    impl Timed for Point {
        fn time(&self) -> SongTime {
            SongTime::from_ms(self.0)
        }
    }

    fn list() -> TimeOrderedList<Point> {
        TimeOrderedList::new(vec![Point(2000.0, "b"), Point(1000.0, "a"), Point(2000.0, "c"), Point(3000.0, "d")])
    }

    #[test]
    fn nothing_before_the_first_item() {
        assert_eq!(list().last_at_or_before(SongTime::from_ms(999.0)), None);
    }

    #[test]
    fn an_item_exactly_at_the_time_counts() {
        assert_eq!(list().last_at_or_before(SongTime::from_ms(1000.0)).unwrap().1, "a");
    }

    #[test]
    fn of_two_items_at_one_time_the_later_given_wins() {
        assert_eq!(list().last_at_or_before(SongTime::from_ms(2500.0)).unwrap().1, "c");
    }

    #[test]
    fn after_the_last_item_the_last_item() {
        assert_eq!(list().last_at_or_before(SongTime::from_ms(9999.0)).unwrap().1, "d");
    }

    #[test]
    fn an_empty_list_finds_nothing() {
        let empty: TimeOrderedList<Point> = TimeOrderedList::new(vec![]);
        assert!(empty.is_empty());
        assert_eq!(empty.last_at_or_before(SongTime::from_ms(0.0)), None);
    }
}
