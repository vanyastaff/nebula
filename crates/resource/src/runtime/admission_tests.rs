use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use super::{
    AdmissionCell, AdmissionGeneration, CloseCause, ReopenTransition, SuspendTransition,
    SuspensionFloor, UseMark,
};
use crate::{CredentialObservedAt, error::CredentialUnavailableReason};

fn current(cell: &AdmissionCell) -> Arc<AdmissionGeneration> {
    cell.current()
        .expect("an open cell has a current generation")
}

/// A denial without any use revision: the pre-revision behaviour.
const NO_FLOOR: SuspensionFloor = SuspensionFloor::unwitnessed(None);

/// An observation without a use revision, from a caller that knows no
/// installed projection: the ticket-only rule.
fn legacy_reopen(cell: &AdmissionCell, slot: &str, ticket: u64) -> ReopenTransition {
    cell.reopen(slot, ticket, CredentialObservedAt::new(1), None)
}

const fn at(material_epoch: u64, admission_epoch: u64) -> CredentialObservedAt {
    CredentialObservedAt::new(material_epoch).with_admission_epoch(admission_epoch)
}

const fn mark(material_epoch: u64, admission_epoch: u64) -> UseMark {
    UseMark::new(material_epoch, admission_epoch)
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
        cell.suspend("db", REAUTH, NO_FLOOR),
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
    cell.suspend("db", REAUTH, NO_FLOOR);
    assert!(cell.publish().is_none());
    assert!(cell.current().is_none());
}

#[test]
fn reopen_with_the_matching_ticket_publishes_a_fresh_generation() {
    let cell = AdmissionCell::default();
    let held = current(&cell);
    cell.suspend("db", BLOCKED, NO_FLOOR);
    let ticket = cell.gate_epoch();
    let ReopenTransition::Reopened { seq } = legacy_reopen(&cell, "db", ticket) else {
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
    cell.suspend("db", REAUTH, NO_FLOOR);
    let ticket = cell.gate_epoch();
    legacy_reopen(&cell, "db", ticket);
    let reopened = current(&cell);
    cell.suspend("db", BLOCKED, NO_FLOOR);
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
    cell.suspend("db", REAUTH, NO_FLOOR);
    assert_eq!(
        legacy_reopen(&cell, "db", stale),
        ReopenTransition::Superseded
    );
    let captured = cell.gate_epoch();
    // A deny landing after the ticket was captured wins.
    assert_eq!(
        cell.suspend("db", REAUTH, NO_FLOOR),
        SuspendTransition::Updated
    );
    assert_eq!(
        legacy_reopen(&cell, "db", captured),
        ReopenTransition::Superseded
    );
    assert!(cell.is_suspended());
}

#[test]
fn two_suspended_slots_need_both_to_reopen() {
    let cell = AdmissionCell::default();
    cell.suspend("db", REAUTH, NO_FLOOR);
    assert_eq!(
        cell.suspend("cache", BLOCKED, NO_FLOOR),
        SuspendTransition::Updated
    );
    let suspension = cell.suspension().expect("suspended");
    assert_eq!(suspension.slots().len(), 2);
    assert_eq!(suspension.reason_for("cache"), Some(BLOCKED));
    assert_eq!(suspension.reason(), REAUTH);
    let ticket = cell.gate_epoch();
    assert_eq!(
        legacy_reopen(&cell, "db", ticket),
        ReopenTransition::StillSuspended
    );
    assert!(cell.current().is_none());
    assert!(matches!(
        legacy_reopen(&cell, "cache", ticket),
        ReopenTransition::Reopened { .. }
    ));
    assert!(cell.current().is_some());
}

#[test]
fn reopen_without_a_suspension_is_not_suspended() {
    let cell = AdmissionCell::default();
    let before = current(&cell);
    assert_eq!(
        legacy_reopen(&cell, "db", cell.gate_epoch()),
        ReopenTransition::NotSuspended
    );
    assert_eq!(current(&cell).seq(), before.seq(), "nothing is published");
}

#[test]
fn retire_after_suspend_closes_the_fresh_span_too() {
    let cell = AdmissionCell::default();
    let held = current(&cell);
    cell.suspend("db", REAUTH, NO_FLOOR);
    cell.retire();
    assert!(held.is_closed());
    assert!(cell.is_retired());
    assert_eq!(
        cell.suspend("db", REAUTH, NO_FLOOR),
        SuspendTransition::Retired
    );
    assert_eq!(
        legacy_reopen(&cell, "db", cell.gate_epoch()),
        ReopenTransition::Retired
    );
    assert!(cell.current().is_none());
}

#[test]
fn close_cause_is_visible_from_a_held_generation() {
    let cell = AdmissionCell::default();
    let held = current(&cell);
    assert_eq!(held.close_cause(), None);
    cell.suspend("db", REAUTH, NO_FLOOR);
    assert_eq!(held.close_cause(), Some(CloseCause::Credential(REAUTH)));
    // A later reason on another slot does not rewrite the closed span.
    cell.suspend("cache", BLOCKED, NO_FLOOR);
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
    cell.suspend("db", REAUTH, NO_FLOOR);
    let sentinel = cell.snapshot();
    assert!(sentinel.is_closed());
    assert_eq!(sentinel.seq(), 0);
    assert_eq!(sentinel.close_cause(), None);
}

// Use revisions (admission epochs).

#[test]
fn a_witnessed_floor_needs_a_strictly_newer_use_revision() {
    let cell = AdmissionCell::default();
    let installed = Some(mark(1, 5));
    cell.suspend("db", REAUTH, SuspensionFloor::witnessed(mark(1, 5)));
    let ticket = cell.gate_epoch();
    // An `Available` read at the revision the denial was read at, or older,
    // cannot clear it: the backend advances the revision when it clears.
    for stale in [at(1, 5), at(1, 4)] {
        assert_eq!(
            cell.reopen("db", ticket, stale, installed),
            ReopenTransition::StaleObservation
        );
        assert!(cell.is_suspended());
        assert_eq!(cell.gate_epoch(), ticket, "a refusal mutates nothing");
    }
    assert!(matches!(
        cell.reopen("db", ticket, at(1, 6), installed),
        ReopenTransition::Reopened { .. }
    ));
    assert!(!cell.is_suspended());
    assert_eq!(cell.admitted("db", installed), Some(mark(1, 6)));
}

#[test]
fn a_material_advance_clears_any_floor_but_only_through_its_install() {
    let cell = AdmissionCell::default();
    cell.suspend("db", REAUTH, SuspensionFloor::witnessed(mark(1, 9)));
    let ticket = cell.gate_epoch();
    // The slot still holds material 1: newer material must be installed.
    assert_eq!(
        cell.reopen("db", ticket, at(2, 1), Some(mark(1, 9))),
        ReopenTransition::StaleObservation
    );
    // Once installed, material 2 at any revision beats a floor at material 1.
    assert!(matches!(
        cell.reopen("db", ticket, at(2, 1), Some(mark(2, 1))),
        ReopenTransition::Reopened { .. }
    ));
}

#[test]
fn an_unwitnessed_floor_accepts_the_admitted_revision_but_not_an_older_one() {
    let cell = AdmissionCell::default();
    let installed = Some(mark(1, 5));
    cell.suspend("db", BLOCKED, SuspensionFloor::unwitnessed(installed));
    let ticket = cell.gate_epoch();
    assert_eq!(
        cell.reopen("db", ticket, at(1, 4), installed),
        ReopenTransition::StaleObservation
    );
    assert!(matches!(
        cell.reopen("db", ticket, at(1, 5), installed),
        ReopenTransition::Reopened { .. }
    ));
}

#[test]
fn an_observation_without_a_revision_follows_the_ticket_rule() {
    let cell = AdmissionCell::default();
    cell.suspend("db", REAUTH, SuspensionFloor::witnessed(mark(1, 5)));
    let ticket = cell.gate_epoch();
    assert!(matches!(
        cell.reopen("db", ticket, CredentialObservedAt::new(1), Some(mark(1, 5))),
        ReopenTransition::Reopened { .. }
    ));
}

#[test]
fn a_stale_ticket_is_superseded_before_the_revision_is_compared() {
    let cell = AdmissionCell::default();
    let stale = cell.gate_epoch();
    cell.suspend("db", REAUTH, SuspensionFloor::witnessed(mark(1, 5)));
    assert_eq!(
        cell.reopen("db", stale, at(1, 6), Some(mark(1, 5))),
        ReopenTransition::Superseded
    );
    assert!(cell.is_suspended());
}

#[test]
fn a_higher_revision_on_an_admitting_row_readmits_without_closing() {
    let cell = AdmissionCell::default();
    let installed = Some(mark(1, 5));
    let held = current(&cell);
    let ticket = cell.gate_epoch();
    let ReopenTransition::Readmitted { seq } = cell.reopen("db", ticket, at(1, 6), installed)
    else {
        panic!("a missed denial interval readmits");
    };
    assert!(seq > held.seq());
    assert_eq!(current(&cell).seq(), seq);
    assert!(
        !held.is_closed(),
        "only an observed block closes admitted work"
    );
    assert_eq!(cell.gate_epoch(), ticket, "readmission is not a suspension");
    assert!(!cell.is_suspended());
    // The same revision again adds nothing; an older one is stale.
    assert_eq!(
        cell.reopen("db", ticket, at(1, 6), installed),
        ReopenTransition::NotSuspended
    );
    assert_eq!(
        cell.reopen("db", ticket, at(1, 5), installed),
        ReopenTransition::StaleObservation
    );
    assert_eq!(current(&cell).seq(), seq, "nothing else was published");
}

#[test]
fn the_installed_revision_is_already_admitted() {
    let cell = AdmissionCell::default();
    let before = current(&cell);
    assert_eq!(
        cell.reopen("db", cell.gate_epoch(), at(1, 5), Some(mark(1, 5))),
        ReopenTransition::NotSuspended
    );
    assert_eq!(current(&cell).seq(), before.seq());
    // Without an installed projection there is nothing to compare against:
    // the revision is only recorded.
    assert_eq!(
        cell.reopen("cache", cell.gate_epoch(), at(3, 2), None),
        ReopenTransition::NotSuspended
    );
    assert_eq!(cell.admitted("cache", None), Some(mark(3, 2)));
    assert_eq!(current(&cell).seq(), before.seq());
}

#[test]
fn each_slot_keeps_its_own_floor() {
    let cell = AdmissionCell::default();
    cell.suspend("db", REAUTH, SuspensionFloor::witnessed(mark(1, 5)));
    cell.suspend("cache", REAUTH, SuspensionFloor::witnessed(mark(1, 3)));
    let ticket = cell.gate_epoch();
    assert_eq!(
        cell.reopen("db", ticket, at(1, 6), Some(mark(1, 5))),
        ReopenTransition::StillSuspended
    );
    assert_eq!(
        cell.reopen("cache", ticket, at(1, 3), Some(mark(1, 3))),
        ReopenTransition::StaleObservation
    );
    assert!(matches!(
        cell.reopen("cache", ticket, at(1, 4), Some(mark(1, 3))),
        ReopenTransition::Reopened { .. }
    ));
}

#[test]
fn a_repeated_suspension_keeps_the_higher_floor() {
    let cell = AdmissionCell::default();
    let installed = Some(mark(1, 3));
    cell.suspend("db", REAUTH, SuspensionFloor::witnessed(mark(1, 5)));
    // A late, older denial read cannot lower the floor.
    assert_eq!(
        cell.suspend("db", BLOCKED, SuspensionFloor::witnessed(mark(1, 3))),
        SuspendTransition::Updated
    );
    let ticket = cell.gate_epoch();
    assert_eq!(
        cell.reopen("db", ticket, at(1, 5), installed),
        ReopenTransition::StaleObservation
    );
    assert_eq!(
        cell.suspension().expect("suspended").reason_for("db"),
        Some(BLOCKED),
        "the latest reason is recorded"
    );
    // An unwitnessed denial at the same mark does not relax a witnessed one.
    cell.suspend(
        "db",
        BLOCKED,
        SuspensionFloor::unwitnessed(Some(mark(1, 5))),
    );
    let ticket = cell.gate_epoch();
    assert_eq!(
        cell.reopen("db", ticket, at(1, 5), installed),
        ReopenTransition::StaleObservation
    );
    assert!(matches!(
        cell.reopen("db", ticket, at(1, 6), installed),
        ReopenTransition::Reopened { .. }
    ));
}

#[test]
fn a_retired_row_neither_reopens_nor_readmits() {
    let cell = AdmissionCell::default();
    cell.retire();
    assert_eq!(
        cell.reopen("db", cell.gate_epoch(), at(1, 9), Some(mark(1, 1))),
        ReopenTransition::Retired
    );
    assert!(cell.current().is_none());
}
