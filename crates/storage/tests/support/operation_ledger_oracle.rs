//! One shared acceptance oracle for the durable operation ledger.
//!
//! The in-memory reference model, SQLite, and PostgreSQL implement the same two
//! roles, so they must answer every ledger question identically. This module
//! owns those answers once; each backend's test file supplies a ledger and runs
//! the same cases against it.
//!
//! Cases are keyed by a per-process namespace and a per-case seed so they can
//! share one durable store: spinning up an isolated PostgreSQL schema and
//! replaying the whole ordered migration catalog per case would cost minutes,
//! and a backend whose store outlives the run would otherwise meet the previous
//! run's slots.

use nebula_storage_port::store::{OperationLedger, OperationLedgerAdjudicator};
use nebula_storage_port::{
    AttemptGeneration, DestinationCapability, EffectSlotBinding, EffectSlotId, KnownOutcome,
    OperationLedgerError, OperationState, PrepareOutcome, RequestFingerprint, Scope,
};

/// Both ledger roles one adapter offers together.
///
/// Production wiring hands them out separately — an effect caller never
/// receives the adjudicator — but a conformance run needs both to drive an
/// operation through its whole lifecycle.
pub(crate) trait LedgerUnderTest: OperationLedger + OperationLedgerAdjudicator {}

impl<T> LedgerUnderTest for T where T: OperationLedger + OperationLedgerAdjudicator {}

/// Per-process namespace folded into every execution identity.
///
/// A backend whose durable store outlives the test run would otherwise meet the
/// previous run's slots on the second run, so a `Prepared` case would report
/// `Replayed`.
static NAMESPACE: std::sync::LazyLock<String> =
    std::sync::LazyLock::new(|| uuid::Uuid::new_v4().simple().to_string());

pub(crate) fn scope() -> Scope {
    Scope::new("ws-ledger", "org-ledger")
}

/// A second tenant, used to prove one tenant cannot reach another's slots.
pub(crate) fn other_scope() -> Scope {
    Scope::new("ws-ledger-other", "org-ledger-other")
}

fn execution_id(seed: u8) -> String {
    format!("exe-{}-{seed:02x}", *NAMESPACE)
}

fn fingerprint(byte: u8) -> RequestFingerprint {
    RequestFingerprint::new(1, [byte; 32])
}

/// Build a binding for `seed`'s slot.
fn binding<'a>(
    scope: &'a Scope,
    execution: &'a str,
    occurrence: &'a str,
    generation: u64,
    digest: u8,
    destination: DestinationCapability,
) -> EffectSlotBinding<'a> {
    EffectSlotBinding {
        scope,
        execution_id: execution,
        node_key: "charge",
        occurrence,
        attempt_generation: AttemptGeneration::new(generation),
        fingerprint: fingerprint(digest),
        destination,
    }
}

async fn prepare_fresh(ledger: &impl LedgerUnderTest, seed: u8) -> (String, EffectSlotId) {
    let scope = scope();
    let execution = execution_id(seed);
    let outcome = ledger
        .prepare(&binding(
            &scope,
            &execution,
            "once",
            1,
            0x11,
            DestinationCapability::StableKey,
        ))
        .await
        .expect("a fresh slot prepares");
    assert!(
        matches!(outcome, PrepareOutcome::Prepared(_)),
        "a slot the ledger has never seen must report Prepared, got {outcome:?}"
    );
    (execution, outcome.operation().slot_id())
}

/// A fresh slot prepares once and reads back with the identity it minted.
pub(crate) async fn prepare_mints_an_identity_and_reads_back(
    ledger: &impl LedgerUnderTest,
    seed: u8,
) {
    let (_execution, slot_id) = prepare_fresh(ledger, seed).await;
    let record = ledger
        .read_exact(&scope(), slot_id)
        .await
        .expect("a prepared slot reads back");

    assert_eq!(record.operation().slot_id(), slot_id);
    assert_eq!(record.state(), OperationState::Prepared);
    assert_eq!(record.fingerprint(), fingerprint(0x11));
    assert_eq!(
        record.operation().destination(),
        DestinationCapability::StableKey,
        "the guarantee recorded at prepare time is what the record carries"
    );
}

/// The same slot with the same request replays the original operation identity.
///
/// This is the property the whole protocol rests on: a restarted worker must
/// reach the provider under one identity, not mint a second one.
pub(crate) async fn same_slot_same_request_reuses_the_operation_identity(
    ledger: &impl LedgerUnderTest,
    seed: u8,
) {
    let scope = scope();
    let (execution, slot_id) = prepare_fresh(ledger, seed).await;
    let first = ledger
        .read_exact(&scope, slot_id)
        .await
        .expect("the prepared slot reads back")
        .operation()
        .operation_id();

    // A later attempt, from a different destination view, re-preparing the
    // same slot.
    let replayed = ledger
        .prepare(&binding(
            &scope,
            &execution,
            "once",
            9,
            0x11,
            DestinationCapability::Opaque,
        ))
        .await
        .expect("the same request on the same slot replays");

    assert!(
        matches!(replayed, PrepareOutcome::Replayed(_)),
        "a slot the ledger already holds must report Replayed, got {replayed:?}"
    );
    assert_eq!(
        replayed.operation().operation_id(),
        first,
        "every retry of one effect slot must reach the provider under one identity"
    );
    assert_eq!(
        replayed.operation().destination(),
        DestinationCapability::StableKey,
        "a destination guarantee that changed since the prepare must not apply retroactively"
    );
}

/// The same slot with a different request fails closed and writes nothing.
pub(crate) async fn same_slot_different_request_is_a_mismatch_with_no_delta(
    ledger: &impl LedgerUnderTest,
    seed: u8,
) {
    let scope = scope();
    let (execution, slot_id) = prepare_fresh(ledger, seed).await;
    let before = ledger
        .read_exact(&scope, slot_id)
        .await
        .expect("the prepared slot reads back");

    assert_eq!(
        ledger
            .prepare(&binding(
                &scope,
                &execution,
                "once",
                1,
                0x99,
                DestinationCapability::StableKey,
            ))
            .await,
        Err(OperationLedgerError::OperationMismatch { slot_id }),
        "reusing a slot for a different request would give two effects one identity"
    );
    assert_eq!(
        ledger
            .read_exact(&scope, slot_id)
            .await
            .expect("the slot still reads back"),
        before,
        "a rejected prepare must leave no durable delta"
    );
}

/// Two occurrences from one node are two slots even with identical payloads.
pub(crate) async fn identical_payloads_on_distinct_occurrences_stay_distinct(
    ledger: &impl LedgerUnderTest,
    seed: u8,
) {
    let scope = scope();
    let execution = execution_id(seed);
    let first = ledger
        .prepare(&binding(
            &scope,
            &execution,
            "first",
            1,
            0x11,
            DestinationCapability::StableKey,
        ))
        .await
        .expect("the first occurrence prepares");
    let second = ledger
        .prepare(&binding(
            &scope,
            &execution,
            "second",
            1,
            0x11,
            DestinationCapability::StableKey,
        ))
        .await
        .expect("the second occurrence prepares");

    assert!(matches!(second, PrepareOutcome::Prepared(_)));
    assert_ne!(
        first.operation().slot_id(),
        second.operation().slot_id(),
        "charging a card twice is a legitimate program; payload equality must not merge them"
    );
    assert_ne!(
        first.operation().operation_id(),
        second.operation().operation_id(),
        "two intended effects must not share one provider-visible identity"
    );
}

/// A superseded attempt cannot decide the current attempt's outcome.
pub(crate) async fn a_stale_attempt_cannot_commit_an_outcome(
    ledger: &impl LedgerUnderTest,
    seed: u8,
) {
    let scope = scope();
    let execution = execution_id(seed);
    let prepared = ledger
        .prepare(&binding(
            &scope,
            &execution,
            "once",
            5,
            0x11,
            DestinationCapability::StableKey,
        ))
        .await
        .expect("a fresh slot prepares");
    let slot_id = prepared.operation().slot_id();

    assert_eq!(
        ledger
            .commit_outcome(
                &scope,
                slot_id,
                AttemptGeneration::new(4),
                KnownOutcome::Succeeded
            )
            .await,
        Err(OperationLedgerError::StaleFence {
            slot_id,
            current: AttemptGeneration::new(5),
        })
    );
    assert_eq!(
        ledger
            .read_exact(&scope, slot_id)
            .await
            .expect("the slot reads back")
            .state(),
        OperationState::Prepared,
        "a refused commit must not move the record"
    );
}

/// Recommitting the same outcome is idempotent; a different one is refused.
///
/// The idempotent path is what lets a caller whose acknowledgement was lost
/// recommit the same frozen evidence without inventing a second answer.
pub(crate) async fn a_lost_acknowledgement_recommits_but_a_second_answer_is_refused(
    ledger: &impl LedgerUnderTest,
    seed: u8,
) {
    let scope = scope();
    let (_execution, slot_id) = prepare_fresh(ledger, seed).await;
    let generation = AttemptGeneration::new(1);

    assert_eq!(
        ledger
            .commit_outcome(&scope, slot_id, generation, KnownOutcome::Succeeded)
            .await,
        Ok(())
    );
    assert_eq!(
        ledger
            .commit_outcome(&scope, slot_id, generation, KnownOutcome::Succeeded)
            .await,
        Ok(()),
        "a lost acknowledgement is reconciled by recommitting the same evidence"
    );
    assert_eq!(
        ledger
            .commit_outcome(&scope, slot_id, generation, KnownOutcome::Failed)
            .await,
        Err(OperationLedgerError::OutcomeAlreadyRecorded {
            slot_id,
            recorded: OperationState::Succeeded,
        }),
        "the ledger must never hold two answers for one effect"
    );
}

/// Adjudication resolves an unknown outcome and nothing else.
pub(crate) async fn adjudication_resolves_only_an_unknown_outcome(
    ledger: &impl LedgerUnderTest,
    seed: u8,
) {
    let scope = scope();
    let (_execution, slot_id) = prepare_fresh(ledger, seed).await;
    let generation = AttemptGeneration::new(1);

    ledger
        .commit_outcome(&scope, slot_id, generation, KnownOutcome::OutcomeUnknown)
        .await
        .expect("an ambiguous boundary records OutcomeUnknown");

    assert_eq!(
        ledger
            .adjudicate(&scope, slot_id, KnownOutcome::Succeeded, "   ")
            .await,
        Err(OperationLedgerError::CorruptRecord { slot_id }),
        "an adjudication without a reason is not reviewable"
    );
    assert_eq!(
        ledger
            .adjudicate(
                &scope,
                slot_id,
                KnownOutcome::Succeeded,
                "provider support confirmed the charge landed"
            )
            .await,
        Ok(())
    );
    assert_eq!(
        ledger
            .read_exact(&scope, slot_id)
            .await
            .expect("the slot reads back")
            .state(),
        OperationState::Succeeded
    );
    assert_eq!(
        ledger
            .adjudicate(&scope, slot_id, KnownOutcome::Failed, "changed my mind")
            .await,
        Err(OperationLedgerError::OutcomeAlreadyRecorded {
            slot_id,
            recorded: OperationState::Succeeded,
        }),
        "adjudication never overrules an answer that is already determined"
    );
}

/// One tenant cannot observe or mutate another's slot.
///
/// The denial is reported exactly as an absent slot: "exists, but not yours"
/// would turn a guessed identity into a cross-tenant existence oracle.
pub(crate) async fn a_foreign_tenant_cannot_observe_or_mutate_a_slot(
    ledger: &impl LedgerUnderTest,
    seed: u8,
) {
    let (_execution, slot_id) = prepare_fresh(ledger, seed).await;
    let intruder = other_scope();

    assert_eq!(
        ledger.read_exact(&intruder, slot_id).await,
        Err(OperationLedgerError::SlotUnprepared { slot_id }),
        "a foreign slot must be indistinguishable from an absent one"
    );
    assert_eq!(
        ledger
            .commit_outcome(
                &intruder,
                slot_id,
                AttemptGeneration::new(1),
                KnownOutcome::Succeeded
            )
            .await,
        Err(OperationLedgerError::SlotUnprepared { slot_id })
    );
    assert_eq!(
        ledger
            .read_exact(&scope(), slot_id)
            .await
            .expect("the owning tenant still reads it")
            .state(),
        OperationState::Prepared,
        "a refused cross-tenant write must change nothing"
    );
}

/// Reading a slot that was never prepared is not an error the caller can act on
/// by inventing one.
pub(crate) async fn an_unprepared_slot_reads_as_unprepared(
    ledger: &impl LedgerUnderTest,
    seed: u8,
) {
    let absent = EffectSlotId::from_storage_bytes([seed; 16]);
    assert_eq!(
        ledger.read_exact(&scope(), absent).await,
        Err(OperationLedgerError::SlotUnprepared { slot_id: absent })
    );
    assert_eq!(
        ledger
            .commit_outcome(
                &scope(),
                absent,
                AttemptGeneration::new(1),
                KnownOutcome::Succeeded
            )
            .await,
        Err(OperationLedgerError::SlotUnprepared { slot_id: absent })
    );
}

/// Generate one `#[tokio::test]` per shared case against `$ledger`.
///
/// `$ledger` is an async expression yielding `Option<impl LedgerUnderTest>`;
/// `None` means this backend is unreachable in the current environment and the
/// case reports that rather than asserting against a substitute.
///
/// The including file must declare this module as `oracle`.
#[macro_export]
macro_rules! operation_ledger_conformance_suite {
    ($ledger:expr) => {
        $crate::operation_ledger_case!(prepare_mints_an_identity_and_reads_back, 0x21, $ledger);
        $crate::operation_ledger_case!(
            same_slot_same_request_reuses_the_operation_identity,
            0x22,
            $ledger
        );
        $crate::operation_ledger_case!(
            same_slot_different_request_is_a_mismatch_with_no_delta,
            0x23,
            $ledger
        );
        $crate::operation_ledger_case!(
            identical_payloads_on_distinct_occurrences_stay_distinct,
            0x24,
            $ledger
        );
        $crate::operation_ledger_case!(a_stale_attempt_cannot_commit_an_outcome, 0x25, $ledger);
        $crate::operation_ledger_case!(
            a_lost_acknowledgement_recommits_but_a_second_answer_is_refused,
            0x26,
            $ledger
        );
        $crate::operation_ledger_case!(
            adjudication_resolves_only_an_unknown_outcome,
            0x27,
            $ledger
        );
        $crate::operation_ledger_case!(
            a_foreign_tenant_cannot_observe_or_mutate_a_slot,
            0x28,
            $ledger
        );
        $crate::operation_ledger_case!(an_unprepared_slot_reads_as_unprepared, 0x29, $ledger);
    };
}

/// Bind one shared case to a `#[tokio::test]` in the including backend file.
#[macro_export]
macro_rules! operation_ledger_case {
    ($case:ident, $seed:expr, $ledger:expr) => {
        #[tokio::test]
        async fn $case() {
            let Some(ledger) = $ledger.await else {
                eprintln!(concat!(
                    stringify!($case),
                    ": backend unreachable in this environment"
                ));
                return;
            };
            oracle::$case(&ledger, $seed).await;
        }
    };
}
