//! Selection (Responsibility 3) and clipboard (Responsibility 3a).

use dawsu_editor::{Clipboard, EditHistory, Selection};
use dawsu_foundation::SongTime;

fn t(ms: f64) -> SongTime {
    SongTime::from_ms(ms)
}

#[test]
fn r3_the_selection_keeps_a_primary_item_and_bumps_its_version() {
    let mut s = Selection::new();
    let v0 = s.version();
    s.add(1);
    s.add(2);
    assert_eq!(s.primary(), Some(&2));
    s.toggle(2);
    assert!(!s.contains(&2));
    assert_eq!(s.primary(), Some(&1), "the primary passes to the last item still selected");
    s.toggle(3);
    assert!(s.contains(&3) && s.contains(&1));
    s.replace([7, 8]);
    assert_eq!(s.len(), 2);
    assert_eq!(s.primary(), Some(&7));
    assert!(s.version() > v0);
    s.clear();
    assert!(s.is_empty());
}

#[test]
fn r3_items_removed_by_undo_drop_out_of_the_selection_which_is_no_undo_step() {
    let mut h = EditHistory::new(vec![1, 2]);
    let mut s = Selection::new();
    h.apply("Insert", true, |d: &mut Vec<i32>| -> Result<(), ()> {
        d.push(3);
        Ok(())
    })
    .unwrap();
    s.replace([2, 3]);
    assert_eq!(h.steps().len(), 1, "selecting made no undo step");
    h.undo();
    let doc = h.document().clone();
    s.retain(|i| doc.contains(i));
    assert!(s.contains(&2));
    assert!(!s.contains(&3));
    assert_eq!(s.primary(), Some(&2));
}

#[test]
fn r3_retain_passes_a_removed_primary_on_and_clears_it_when_nothing_is_left() {
    let mut s = Selection::new();
    s.add(1);
    s.add(2);
    s.retain(|i| *i != 2);
    assert_eq!(s.primary(), Some(&1));
    s.retain(|_| false);
    assert_eq!(s.primary(), None);
}

#[test]
fn r3a_the_clipboard_keeps_times_relative_to_the_first_item_and_pastes_at_the_playhead() {
    let mut c = Clipboard::new();
    c.copy([(t(1500.0), "b"), (t(1000.0), "a"), (t(1750.0), "c")]);
    assert_eq!(c.span_ms(), 750.0);
    let pasted = c.paste_at(t(4000.0));
    assert_eq!(
        pasted,
        vec![(t(4000.0), "a"), (t(4500.0), "b"), (t(4750.0), "c")]
    );
}

#[test]
fn r3a_copying_nothing_keeps_the_clipboard() {
    let mut c = Clipboard::new();
    c.copy([(t(10.0), 1)]);
    c.copy(Vec::<(SongTime, i32)>::new());
    assert!(!c.is_empty());
    assert_eq!(c.paste_at(t(0.0)), vec![(t(0.0), 1)]);
}
