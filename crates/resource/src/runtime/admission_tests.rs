use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use super::{AdmissionCell, AdmissionGeneration, CloseCause, ReopenTransition, SuspendTransition};
use crate::error::CredentialUnavailableReason;

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

const REAUTH: CredentialUnavailableReason = CredentialUnavailableReason::ReauthRequired;
const BLOCKED: CredentialUnavailableReason = CredentialUnavailableReason::OperationBlocked;

#[test]
fn suspend_closes_the_current_and_older_benign_generations() {
    let cell = AdmissionCell::default();
    let first = current(&cell);
    let second = cell.publish().expect("an open cell publishes");
    assert_eq!(
        cell.suspend("db", REAUTH),
        SuspendTransition::Suspended {
            closed_through: second.seq()
        }
    );
    assert!(second.is_closed());
    assert!(
        first.is_closed(),
        "suspension closes the whole span, not only the current generation"
    );
    assert!(cell.current().is_none());
    assert!(cell.is_suspended());
    assert!(!cell.is_retired());
}

#[test]
fn publish_while_suspended_publishes_nothing() {
    let cell = AdmissionCell::default();
    cell.suspend("db", REAUTH);
    assert!(cell.publish().is_none());
    assert!(cell.current().is_none());
}

#[test]
fn reopen_with_the_matching_ticket_publishes_a_fresh_generation() {
    let cell = AdmissionCell::default();
    let held = current(&cell);
    cell.suspend("db", BLOCKED);
    let ticket = cell.gate_epoch();
    let ReopenTransition::Reopened { seq } = cell.reopen("db", ticket) else {
        panic!("the matching ticket reopens the row");
    };
    let fresh = current(&cell);
    assert_eq!(fresh.seq(), seq);
    assert!(seq > held.seq());
    assert!(!fresh.is_closed());
    assert!(
        held.is_closed(),
        "a closed generation stays closed on reopen"
    );
    assert!(!cell.is_suspended());
    assert!(cell.suspension().is_none());
    // Benign publication resumes and leaves the reopened generation open.
    let next = cell.publish().expect("a reopened cell publishes");
    assert!(!fresh.is_closed());
    assert!(next.seq() > fresh.seq());
}

#[test]
fn a_second_suspension_closes_only_its_own_span() {
    let cell = AdmissionCell::default();
    cell.suspend("db", REAUTH);
    let ticket = cell.gate_epoch();
    cell.reopen("db", ticket);
    let reopened = current(&cell);
    cell.suspend("db", BLOCKED);
    assert!(reopened.is_closed());
    assert_eq!(
        reopened.close_cause(),
        Some(CloseCause::Credential(BLOCKED)),
        "the new span records its own cause"
    );
}

#[test]
fn a_stale_ticket_is_superseded() {
    let cell = AdmissionCell::default();
    let stale = cell.gate_epoch();
    cell.suspend("db", REAUTH);
    assert_eq!(cell.reopen("db", stale), ReopenTransition::Superseded);
    let captured = cell.gate_epoch();
    // A deny landing after the ticket was captured wins.
    assert_eq!(cell.suspend("db", REAUTH), SuspendTransition::Updated);
    assert_eq!(cell.reopen("db", captured), ReopenTransition::Superseded);
    assert!(cell.is_suspended());
}

#[test]
fn two_suspended_slots_need_both_to_reopen() {
    let cell = AdmissionCell::default();
    cell.suspend("db", REAUTH);
    assert_eq!(cell.suspend("cache", BLOCKED), SuspendTransition::Updated);
    let suspension = cell.suspension().expect("suspended");
    assert_eq!(suspension.slots().len(), 2);
    assert_eq!(suspension.reason_for("cache"), Some(BLOCKED));
    assert_eq!(suspension.reason(), REAUTH);
    let ticket = cell.gate_epoch();
    assert_eq!(cell.reopen("db", ticket), ReopenTransition::StillSuspended);
    assert!(cell.current().is_none());
    assert!(matches!(
        cell.reopen("cache", ticket),
        ReopenTransition::Reopened { .. }
    ));
    assert!(cell.current().is_some());
}

#[test]
fn reopen_without_a_suspension_is_not_suspended() {
    let cell = AdmissionCell::default();
    let before = current(&cell);
    assert_eq!(
        cell.reopen("db", cell.gate_epoch()),
        ReopenTransition::NotSuspended
    );
    assert_eq!(current(&cell).seq(), before.seq(), "nothing is published");
}

#[test]
fn retire_after_suspend_closes_the_fresh_span_too() {
    let cell = AdmissionCell::default();
    let held = current(&cell);
    cell.suspend("db", REAUTH);
    cell.retire();
    assert!(held.is_closed());
    assert!(cell.is_retired());
    assert_eq!(cell.suspend("db", REAUTH), SuspendTransition::Retired);
    assert_eq!(
        cell.reopen("db", cell.gate_epoch()),
        ReopenTransition::Retired
    );
    assert!(cell.current().is_none());
}

#[test]
fn close_cause_is_visible_from_a_held_generation() {
    let cell = AdmissionCell::default();
    let held = current(&cell);
    assert_eq!(held.close_cause(), None);
    cell.suspend("db", REAUTH);
    assert_eq!(held.close_cause(), Some(CloseCause::Credential(REAUTH)));
    // A later reason on another slot does not rewrite the closed span.
    cell.suspend("cache", BLOCKED);
    assert_eq!(held.close_cause(), Some(CloseCause::Credential(REAUTH)));
}

#[test]
fn retirement_alone_leaves_no_close_cause() {
    let cell = AdmissionCell::default();
    let held = current(&cell);
    cell.retire();
    assert!(held.is_closed());
    assert_eq!(held.close_cause(), None);
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

#[test]
fn snapshot_while_suspended_is_the_closed_sentinel() {
    let cell = AdmissionCell::default();
    cell.suspend("db", REAUTH);
    let sentinel = cell.snapshot();
    assert!(sentinel.is_closed());
    assert_eq!(sentinel.seq(), 0);
    assert_eq!(sentinel.close_cause(), None);
}
