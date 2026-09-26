use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use super::{AdmissionCell, AdmissionGeneration};

fn current(cell: &AdmissionCell) -> Arc<AdmissionGeneration> {
    cell.current()
        .expect("an open cell has a current generation")
}

#[test]
fn new_cell_publishes_sequence_one() {
    let cell = AdmissionCell::new(CancellationToken::new());
    let first = current(&cell);
    assert_eq!(first.seq(), 1);
    assert!(!first.is_closed());
    assert!(!cell.is_retired());
}

#[test]
fn publish_is_monotone_and_leaves_the_predecessor_open() {
    let cell = AdmissionCell::default();
    let first = current(&cell);
    let second = cell.publish().expect("an open cell publishes");
    let third = cell.publish().expect("an open cell publishes");
    assert!(first.seq() < second.seq() && second.seq() < third.seq());
    assert_eq!(current(&cell).seq(), third.seq());
    assert!(
        !first.is_closed(),
        "a benign publish never closes a predecessor"
    );
    assert!(!second.is_closed());
}

#[test]
fn close_current_closes_only_the_current_generation() {
    let cell = AdmissionCell::default();
    let first = current(&cell);
    let second = cell.publish().expect("an open cell publishes");
    assert_eq!(cell.close_current(), Some(second.seq()));
    assert!(second.is_closed());
    assert!(!first.is_closed(), "an older benign generation stays open");
    assert!(cell.current().is_none());
    assert!(!cell.is_retired());
    assert_eq!(cell.close_current(), None, "nothing left to close");
}

#[test]
fn close_then_publish_opens_a_fresh_generation() {
    let cell = AdmissionCell::default();
    // A lease holds G1 before the successor exists.
    let held = current(&cell);
    assert_eq!(cell.close_current(), Some(held.seq()));
    let successor = cell
        .publish()
        .expect("a closed-but-not-retired cell publishes");
    assert!(!successor.is_closed());
    assert!(
        held.is_closed(),
        "the held G1 reads closed after G2 is published"
    );
    assert!(successor.seq() > held.seq());
    assert_eq!(current(&cell).seq(), successor.seq());
    assert!(held.token().is_cancelled());
}

#[test]
fn retire_closes_held_and_current_generations() {
    let cell = AdmissionCell::default();
    let held = current(&cell);
    let second = cell.publish().expect("an open cell publishes");
    cell.retire();
    assert!(
        held.is_closed(),
        "retirement reaches generations held by leases"
    );
    assert!(second.is_closed());
    assert!(cell.current().is_none());
    assert!(cell.is_retired());
}

#[test]
fn publish_after_retire_publishes_nothing() {
    let cell = AdmissionCell::default();
    cell.retire();
    assert!(cell.publish().is_none());
    assert!(cell.current().is_none());
}

#[test]
fn cancelled_parent_leaves_no_current_generation() {
    let manager = CancellationToken::new();
    let cell = AdmissionCell::new(manager.child_token());
    let held = current(&cell);
    manager.cancel();
    assert!(cell.current().is_none());
    assert!(cell.is_retired());
    assert!(held.is_closed());
    assert!(cell.publish().is_none());
}

#[test]
fn retire_is_idempotent() {
    let cell = AdmissionCell::default();
    cell.retire();
    cell.retire();
    assert!(cell.is_retired());
    assert!(cell.current().is_none());
}

#[test]
fn snapshot_is_closed_when_nothing_is_admitted() {
    let cell = AdmissionCell::default();
    assert!(!cell.snapshot().is_closed());
    cell.retire();
    assert!(cell.snapshot().is_closed());
}
