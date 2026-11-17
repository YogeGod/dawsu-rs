//! Responsibility 3: items removed by undo or redo drop out of the selection, through
//! `EditorState`, and Select all and Deselect.

use dawsu_editor::{EditorState, ItemDocument};

#[derive(Clone, Debug, PartialEq)]
struct Doc(Vec<u32>);

impl ItemDocument<u32> for Doc {
    fn contains_item(&self, item: &u32) -> bool {
        self.0.contains(item)
    }
    fn all_items(&self) -> Vec<u32> {
        self.0.clone()
    }
}

fn insert(state: &mut EditorState<Doc, u32>, item: u32) {
    state
        .history_mut()
        .apply("Insert", true, |d: &mut Doc| -> Result<(), ()> {
            d.0.push(item);
            Ok(())
        })
        .unwrap();
}

#[test]
fn undo_drops_an_item_it_removed_from_the_selection() {
    let mut s = EditorState::new(Doc(vec![1]));
    insert(&mut s, 2);
    s.selection_mut().replace([1, 2]);
    assert!(s.undo());
    assert!(s.selection().contains(&1));
    assert!(!s.selection().contains(&2));
    assert_eq!(s.selection().primary(), Some(&1));
}

#[test]
fn redo_and_go_to_drop_removed_items_too() {
    let mut s = EditorState::new(Doc(vec![]));
    insert(&mut s, 1);
    s.history_mut()
        .apply("Delete", true, |d: &mut Doc| -> Result<(), ()> {
            d.0.clear();
            Ok(())
        })
        .unwrap();
    s.undo();
    s.selection_mut().replace([1]);
    assert!(s.redo());
    assert!(s.selection().is_empty());

    s.undo();
    s.selection_mut().replace([1]);
    s.go_to(0);
    assert!(s.selection().is_empty());
}

#[test]
fn undo_with_nothing_to_undo_leaves_the_selection_alone() {
    let mut s = EditorState::new(Doc(vec![1]));
    s.selection_mut().replace([1]);
    let v = s.selection().version();
    assert!(!s.undo());
    assert_eq!(s.selection().version(), v);
}

#[test]
fn select_all_and_deselect() {
    let mut s = EditorState::new(Doc(vec![3, 4, 5]));
    s.select_all();
    assert_eq!(s.selection().len(), 3);
    assert_eq!(s.selection().primary(), Some(&3));
    s.deselect();
    assert!(s.selection().is_empty());
}
