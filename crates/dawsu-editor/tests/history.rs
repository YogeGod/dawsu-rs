//! Edit history: Responsibilities 1, 2, 13 and 13a; invariants 1-5 and 11(b).

use dawsu_editor::{EditError, EditHistory};

type Doc = Vec<i32>;

fn push(h: &mut EditHistory<Doc>, label: &str, value: i32) {
    h.apply(label, true, |d: &mut Doc| -> Result<(), ()> {
        d.push(value);
        Ok(())
    })
    .unwrap();
}

#[test]
fn r1_a_drag_is_visible_while_it_runs_and_one_undo_step_when_it_ends() {
    let mut h = EditHistory::new(vec![0]);
    h.begin("Move");
    for x in 1..=3 {
        h.edit(true, |d: &mut Doc| -> Result<(), ()> {
            d[0] = x;
            Ok(())
        })
        .unwrap();
        assert_eq!(h.document()[0], x, "the drag preview is live");
    }
    h.commit();
    assert_eq!(h.steps(), vec!["Move"]);
    h.undo();
    assert_eq!(h.document(), &vec![0]);
}

#[test]
fn r1_i2_a_cancelled_drag_is_fully_rolled_back_and_leaves_the_history_unchanged() {
    let mut h = EditHistory::new(vec![0]);
    push(&mut h, "Insert", 7);
    let version_steps = h.steps().len();
    h.begin("Move");
    h.edit(true, |d: &mut Doc| -> Result<(), ()> {
        d[0] = 99;
        Ok(())
    })
    .unwrap();
    h.cancel();
    assert_eq!(h.document(), &vec![0, 7]);
    assert_eq!(h.steps().len(), version_steps);
    assert!(!h.is_transaction_open());
}

#[test]
fn i2_a_failing_change_rolls_back_the_whole_transaction() {
    let mut h = EditHistory::new(vec![1]);
    h.begin("Paste");
    h.edit(true, |d: &mut Doc| -> Result<(), String> {
        d.push(2);
        Ok(())
    })
    .unwrap();
    let err = h.edit(true, |d: &mut Doc| -> Result<(), String> {
        d.push(3);
        Err("no room".into())
    });
    assert_eq!(err, Err(EditError::Failed("no room".to_owned())));
    assert_eq!(h.document(), &vec![1]);
    assert!(h.steps().is_empty());
    assert!(!h.is_transaction_open());
    assert!(!h.is_dirty());
}

#[test]
fn i1_the_document_changes_only_inside_a_transaction() {
    let mut h = EditHistory::new(vec![1]);
    let refused = h.edit(true, |d: &mut Doc| -> Result<(), ()> {
        d.clear();
        Ok(())
    });
    assert_eq!(refused, Err(EditError::NoOpenTransaction));
    assert_eq!(h.document(), &vec![1]);
    // `document()` hands out a shared reference only; there is no other way in (checked by the compiler).
}

#[test]
fn r2_undo_redo_and_jumping_to_any_step() {
    let mut h = EditHistory::new(Vec::new());
    push(&mut h, "A", 1);
    push(&mut h, "B", 2);
    push(&mut h, "C", 3);
    assert!(h.undo());
    assert_eq!(h.document(), &vec![1, 2]);
    assert_eq!(
        h.steps(),
        vec!["A", "B", "C"],
        "undone steps stay in the recent edits list"
    );
    assert!(h.redo());
    assert_eq!(h.document(), &vec![1, 2, 3]);
    h.go_to(1);
    assert_eq!(h.document(), &vec![1]);
    assert_eq!(h.position(), 1);
    h.go_to(99);
    assert_eq!(h.document(), &vec![1, 2, 3], "go_to is clamped");
    h.go_to(0);
    assert!(h.document().is_empty());
    assert!(!h.can_undo());
}

#[test]
fn r2_a_view_only_change_does_not_make_the_document_dirty() {
    let mut h = EditHistory::new(vec![1, 2]);
    h.apply("Lane order", false, |d: &mut Doc| -> Result<(), ()> {
        d.reverse();
        Ok(())
    })
    .unwrap();
    assert!(!h.is_dirty());
    assert!(h.can_undo(), "it is still an undo step");
    push(&mut h, "Insert", 3);
    assert!(h.is_dirty());
}

#[test]
fn i3_undo_redo_and_jumps_do_nothing_while_a_transaction_is_open() {
    let mut h = EditHistory::new(Vec::new());
    push(&mut h, "A", 1);
    push(&mut h, "B", 2);
    h.undo();
    h.begin("Move");
    assert!(!h.undo());
    assert!(!h.redo());
    h.go_to(0);
    assert_eq!(h.document(), &vec![1]);
    h.cancel();
}

#[test]
fn i4_a_new_change_after_an_undo_clears_the_redo_list() {
    let mut h = EditHistory::new(Vec::new());
    push(&mut h, "A", 1);
    push(&mut h, "B", 2);
    h.undo();
    push(&mut h, "C", 3);
    assert!(!h.can_redo());
    assert_eq!(h.steps(), vec!["A", "C"]);
}

#[test]
fn i5_clean_after_save_and_again_after_undo_and_redo_back_to_it() {
    let mut h = EditHistory::new(Vec::new());
    push(&mut h, "A", 1);
    assert!(h.is_dirty());
    h.mark_clean();
    assert!(!h.is_dirty());
    h.undo();
    assert!(h.is_dirty());
    h.redo();
    assert!(!h.is_dirty());
    push(&mut h, "B", 2);
    h.undo();
    assert!(!h.is_dirty());
}

#[test]
fn i5_the_saved_point_becomes_unreachable_after_a_new_step_past_an_undo() {
    let mut h = EditHistory::new(Vec::new());
    push(&mut h, "A", 1);
    push(&mut h, "B", 2);
    h.mark_clean();
    h.undo();
    push(&mut h, "C", 3);
    assert!(h.is_dirty());
    h.undo();
    h.undo();
    assert!(h.is_dirty(), "no position equals the saved file any more");
}

#[test]
fn r13_undo_still_reaches_steps_made_before_a_save() {
    let mut h = EditHistory::new(Vec::new());
    push(&mut h, "A", 1);
    push(&mut h, "B", 2);
    h.mark_clean();
    assert_eq!(h.steps(), vec!["A", "B"]);
    h.undo();
    h.undo();
    assert!(h.document().is_empty());
}

#[test]
fn r13a_steps_are_whole_document_copies_that_restore_exactly() {
    // A document with ids that last across saves, as the in-memory file keeps them.
    let mut h = EditHistory::new(vec![(10u32, "line a"), (11, "line b")]);
    h.apply("Edit", true, |d: &mut Vec<(u32, &str)>| -> Result<(), ()> {
        d[1].1 = "line b changed";
        d.push((12, "line c"));
        Ok(())
    })
    .unwrap();
    h.mark_clean();
    h.undo();
    assert_eq!(h.document(), &vec![(10, "line a"), (11, "line b")]);
    h.redo();
    assert_eq!(
        h.document(),
        &vec![(10, "line a"), (11, "line b changed"), (12, "line c")]
    );
}

#[test]
fn nested_begin_joins_the_open_transaction_and_an_empty_one_adds_no_step() {
    let mut h = EditHistory::new(Vec::new());
    h.begin("Outer");
    h.begin("Inner");
    h.edit(true, |d: &mut Doc| -> Result<(), ()> {
        d.push(1);
        Ok(())
    })
    .unwrap();
    h.commit();
    assert!(
        h.is_transaction_open(),
        "the inner commit only leaves the inner begin"
    );
    h.commit();
    assert_eq!(h.steps(), vec!["Outer"]);
    h.begin("Nothing");
    h.commit();
    assert_eq!(h.steps(), vec!["Outer"]);
}

#[test]
fn i11b_a_background_job_result_comes_back_as_one_undo_step() {
    let mut h = EditHistory::new(vec![0]);
    let job_result = vec![1, 2, 3, 4]; // computed elsewhere from a snapshot
    h.apply("Generate", true, |d: &mut Doc| -> Result<(), ()> {
        *d = job_result;
        Ok(())
    })
    .unwrap();
    assert_eq!(h.steps(), vec!["Generate"]);
    h.undo();
    assert_eq!(h.document(), &vec![0]);
}

#[test]
fn the_version_bumps_on_every_change() {
    let mut h = EditHistory::new(Vec::new());
    let v0 = h.version();
    push(&mut h, "A", 1);
    let v1 = h.version();
    assert!(v1 > v0);
    h.undo();
    assert!(h.version() > v1);
}
