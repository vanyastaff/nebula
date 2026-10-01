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

use nebula_storage_port::store::{ExecutionStore, OperationLedger, OperationLedgerAdjudicator};
use nebula_storage_port::{
    AttemptGeneration, DestinationCapability, EffectSlotBinding, EffectSlotId, KnownOutcome,
    OperationLedgerError, OperationState, PrepareOutcome, RequestFingerprint, Scope,
};
use nebula_storage_port::{EffectOccurrenceKey, FencingToken};

/// Both ledger roles one adapter offers together.
///
/// Production wiring hands them out separately — an effect caller never
/// receives the adjudicator — but a conformance run needs both to drive an
/// operation through its whole lifecycle.
///
/// `Clone + 'static` lets a case wrap the adapter in the tenancy decorator,
/// which owns its inner ledger as `Arc<dyn OperationLedger>`.
pub(crate) trait LedgerUnderTest:
    OperationLedger + OperationLedgerAdjudicator + LedgerAssertions + Clone + 'static
{
}

impl<T> LedgerUnderTest for T where T: OperationLedger + OperationLedgerAdjudicator + Clone + 'static
{}

use nebula_core::OperationCallId;
use nebula_storage_port::dto::{
    FrozenOutcomeEvidence, OperationAdvance, OperationCommand, OutcomeEvidenceSource,
    PreparedEffectContract, PreparedEffectPolicy,
};

pub(crate) fn contract(capability: DestinationCapability) -> &'static PreparedEffectContract {
    static STABLE: std::sync::LazyLock<PreparedEffectContract> = std::sync::LazyLock::new(|| {
        PreparedEffectContract::new(
            fingerprint(0x71),
            PreparedEffectPolicy::builder(DestinationCapability::StableKey)
                .maximum_invocations(3)
                .maximum_queries(3)
                .recovery_window(std::time::Duration::from_mins(1))
                .stable_key_window(std::time::Duration::from_mins(1))
                .build()
                .unwrap(),
        )
        .unwrap()
    });
    static OPAQUE: std::sync::LazyLock<PreparedEffectContract> = std::sync::LazyLock::new(|| {
        PreparedEffectContract::new(
            fingerprint(0x72),
            PreparedEffectPolicy::builder(DestinationCapability::Opaque)
                .maximum_invocations(3)
                .maximum_queries(0)
                .recovery_window(std::time::Duration::from_mins(1))
                .build()
                .unwrap(),
        )
        .unwrap()
    });
    static RECONCILABLE: std::sync::LazyLock<PreparedEffectContract> =
        std::sync::LazyLock::new(|| {
            PreparedEffectContract::new(
                fingerprint(0x73),
                PreparedEffectPolicy::builder(DestinationCapability::Reconcilable)
                    .maximum_invocations(3)
                    .maximum_queries(3)
                    .recovery_window(std::time::Duration::from_mins(1))
                    .build()
                    .unwrap(),
            )
            .unwrap()
        });
    match capability {
        DestinationCapability::StableKey => &STABLE,
        DestinationCapability::Reconcilable => &RECONCILABLE,
        _ => &OPAQUE,
    }
}

#[async_trait::async_trait]
pub(crate) trait LedgerAssertions: OperationLedger + OperationLedgerAdjudicator {
    async fn record_known_outcome(
        &self,
        scope: &Scope,
        slot: EffectSlotId,
        fencing: FencingToken,
        outcome: KnownOutcome,
    ) -> Result<(), OperationLedgerError> {
        let stored = self.read_exact(scope, slot).await?;
        let protocol = stored
            .protocol()
            .ok_or(OperationLedgerError::ProtocolConflict)?;
        if outcome == KnownOutcome::OutcomeUnknown {
            self.advance(
                scope,
                slot,
                fencing,
                &OperationCommand::MarkUnknown {
                    expected_revision: protocol.revision(),
                },
            )
            .await?;
            return Ok(());
        }
        let call = if let Some(call) = protocol.invocation() {
            call
        } else {
            let granted = self
                .advance(
                    scope,
                    slot,
                    fencing,
                    &OperationCommand::GrantInvocation {
                        expected_revision: protocol.revision(),
                    },
                )
                .await?;
            match granted {
                OperationAdvance::Granted { call, .. } => call,
                _ => return Err(OperationLedgerError::ProtocolConflict),
            }
        };
        let evidence = FrozenOutcomeEvidence::v1_json(
            OutcomeEvidenceSource::Invocation(call),
            outcome,
            serde_json::to_vec(&serde_json::json!({"outcome":outcome})).unwrap(),
        )
        .unwrap();
        self.advance(
            scope,
            slot,
            fencing,
            &OperationCommand::RecordOutcome(evidence),
        )
        .await?;
        Ok(())
    }

    async fn adjudicate_known(
        &self,
        scope: &Scope,
        slot: EffectSlotId,
        outcome: KnownOutcome,
        audit: &str,
    ) -> Result<(), OperationLedgerError> {
        let evidence = FrozenOutcomeEvidence::v1_json(
            OutcomeEvidenceSource::Adjudication(OperationCallId::from_bytes(*slot.as_bytes())),
            outcome,
            serde_json::to_vec(&serde_json::json!({"outcome":outcome,"output_unavailable":true}))
                .unwrap(),
        )?;
        self.adjudicate(scope, slot, &evidence, audit).await
    }
}
impl<T: OperationLedger + OperationLedgerAdjudicator> LedgerAssertions for T {}

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
        contract: contract(destination),
        provider_key: None,
        concurrent_floor: None,
    }
}

pub(crate) async fn prepare_fresh(
    ledger: &impl LedgerUnderTest,
    executions: &dyn ExecutionStore,
    seed: u8,
) -> (String, EffectSlotId, FencingToken) {
    let scope = scope();
    let execution = execution_id(seed);
    let fencing = create_leased_execution(executions, &scope, &execution).await;
    let outcome = ledger
        .prepare(
            &binding(
                &scope,
                &execution,
                "once",
                1,
                0x11,
                DestinationCapability::StableKey,
            ),
            fencing,
        )
        .await
        .expect("a fresh slot prepares");
    assert!(
        matches!(outcome, PrepareOutcome::Prepared(_)),
        "a slot the ledger has never seen must report Prepared, got {outcome:?}"
    );
    (execution, outcome.operation().slot_id(), fencing)
}

/// A fresh slot prepares once and reads back with the identity it minted.
pub(crate) async fn prepare_mints_an_identity_and_reads_back(
    ledger: &impl LedgerUnderTest,
    executions: &dyn ExecutionStore,
    seed: u8,
) {
    let (execution, slot_id, _fencing) = prepare_fresh(ledger, executions, seed).await;
    let natural = ledger
        .read_occurrence(&EffectOccurrenceKey::new(
            &scope(),
            &execution,
            "charge",
            "once",
        ))
        .await
        .unwrap()
        .unwrap();
    let record = ledger
        .read_exact(&scope(), slot_id)
        .await
        .expect("a prepared slot reads back");

    assert_eq!(natural, record);
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
    executions: &dyn ExecutionStore,
    seed: u8,
) {
    let scope = scope();
    let (execution, slot_id, fencing) = prepare_fresh(ledger, executions, seed).await;
    let first = ledger
        .read_exact(&scope, slot_id)
        .await
        .expect("the prepared slot reads back")
        .operation()
        .operation_id();

    // A later runtime attempt retains the original complete destination binding.
    let replayed = ledger
        .prepare(
            &binding(
                &scope,
                &execution,
                "once",
                9,
                0x11,
                DestinationCapability::StableKey,
            ),
            fencing,
        )
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
    executions: &dyn ExecutionStore,
    seed: u8,
) {
    let scope = scope();
    let (execution, slot_id, fencing) = prepare_fresh(ledger, executions, seed).await;
    let before = ledger
        .read_exact(&scope, slot_id)
        .await
        .expect("the prepared slot reads back");

    assert_eq!(
        ledger
            .prepare(
                &binding(
                    &scope,
                    &execution,
                    "once",
                    1,
                    0x99,
                    DestinationCapability::StableKey,
                ),
                fencing
            )
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
    executions: &dyn ExecutionStore,
    seed: u8,
) {
    let scope = scope();
    let execution = execution_id(seed);
    let fencing = create_leased_execution(executions, &scope, &execution).await;
    let first = ledger
        .prepare(
            &binding(
                &scope,
                &execution,
                "first",
                1,
                0x11,
                DestinationCapability::StableKey,
            ),
            fencing,
        )
        .await
        .expect("the first occurrence prepares");
    let second = ledger
        .prepare(
            &binding(
                &scope,
                &execution,
                "second",
                1,
                0x11,
                DestinationCapability::StableKey,
            ),
            fencing,
        )
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
    executions: &dyn ExecutionStore,
    seed: u8,
) {
    let scope = scope();
    let execution = execution_id(seed);
    let fencing = create_leased_execution(executions, &scope, &execution).await;
    let prepared = ledger
        .prepare(
            &binding(
                &scope,
                &execution,
                "once",
                5,
                0x11,
                DestinationCapability::StableKey,
            ),
            fencing,
        )
        .await
        .expect("a fresh slot prepares");
    let slot_id = prepared.operation().slot_id();
    executions
        .release_lease(&scope, &execution, fencing)
        .await
        .unwrap();
    let replacement = executions
        .acquire_lease(
            &scope,
            &execution,
            "takeover",
            std::time::Duration::from_secs(30),
        )
        .await
        .unwrap()
        .unwrap();
    assert!(replacement.generation() > fencing.generation());

    assert_eq!(
        ledger
            .record_known_outcome(&scope, slot_id, fencing, KnownOutcome::Succeeded)
            .await,
        Err(OperationLedgerError::ExecutionLeaseRejected)
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
    executions: &dyn ExecutionStore,
    seed: u8,
) {
    let scope = scope();
    let (_execution, slot_id, fencing) = prepare_fresh(ledger, executions, seed).await;
    let generation = fencing;

    assert_eq!(
        ledger
            .record_known_outcome(&scope, slot_id, generation, KnownOutcome::Succeeded)
            .await,
        Ok(())
    );
    assert_eq!(
        ledger
            .record_known_outcome(&scope, slot_id, generation, KnownOutcome::Succeeded)
            .await,
        Ok(()),
        "a lost acknowledgement is reconciled by recommitting the same evidence"
    );
    assert_eq!(
        ledger
            .record_known_outcome(&scope, slot_id, generation, KnownOutcome::Failed)
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
    executions: &dyn ExecutionStore,
    seed: u8,
) {
    let scope = scope();
    let (_execution, slot_id, fencing) = prepare_fresh(ledger, executions, seed).await;
    let generation = fencing;

    ledger
        .record_known_outcome(&scope, slot_id, generation, KnownOutcome::OutcomeUnknown)
        .await
        .expect("an ambiguous boundary records OutcomeUnknown");

    assert_eq!(
        ledger
            .adjudicate_known(&scope, slot_id, KnownOutcome::Succeeded, "   ")
            .await,
        Err(OperationLedgerError::InvalidProtocol),
        "an adjudication without a reason is not reviewable"
    );
    assert_eq!(
        ledger
            .adjudicate_known(
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
            .adjudicate_known(&scope, slot_id, KnownOutcome::Failed, "changed my mind")
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
    executions: &dyn ExecutionStore,
    seed: u8,
) {
    let (_execution, slot_id, _fencing) = prepare_fresh(ledger, executions, seed).await;
    let intruder = other_scope();

    assert_eq!(
        ledger.read_exact(&intruder, slot_id).await,
        Err(OperationLedgerError::SlotUnprepared { slot_id }),
        "a foreign slot must be indistinguishable from an absent one"
    );
    assert_eq!(
        ledger
            .record_known_outcome(
                &intruder,
                slot_id,
                FencingToken::from_generation(1),
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
    executions: &dyn ExecutionStore,
    seed: u8,
) {
    let _ = executions;
    let absent = EffectSlotId::from_storage_bytes([seed; 16]);
    assert_eq!(
        ledger.read_exact(&scope(), absent).await,
        Err(OperationLedgerError::SlotUnprepared { slot_id: absent })
    );
    assert_eq!(
        ledger
            .record_known_outcome(
                &scope(),
                absent,
                FencingToken::from_generation(1),
                KnownOutcome::Succeeded
            )
            .await,
        Err(OperationLedgerError::SlotUnprepared { slot_id: absent })
    );
}

/// Generate one `#[tokio::test]` per shared case against `$ledger`.
///
/// `$ledger` is an async expression yielding `Option<impl LedgerUnderTest>`;
/// `None` is a hard failure naming the unreachable backend, never a silent pass
/// and never a skip: a case body that returns early is counted as a pass, so a
/// backend the case cannot run against has to fail loudly instead. A green run
/// therefore means every case ran against a live backend.
///
/// The including file must declare this module as `oracle`.
#[macro_export]
macro_rules! operation_ledger_conformance_suite {
    ($ledger:expr) => {
        $crate::operation_ledger_case!(
            bounded_permits_preserve_identity_and_frozen_evidence,
            0x60,
            $ledger
        );
        $crate::operation_ledger_case!(
            unknown_grant_ack_never_mints_replacement_authority,
            0x61,
            $ledger
        );
        $crate::operation_ledger_case!(
            opaque_ambiguity_and_expiry_forbid_second_effect,
            0x62,
            $ledger
        );
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
        $crate::operation_ledger_case!(attempt_provenance_is_never_truncated, 0x32, $ledger);
        $crate::operation_ledger_case!(execution_lease_is_authority_for_every_write, 0x30, $ledger);
        $crate::operation_ledger_case!(
            adjudication_and_outcome_serialize_one_answer,
            0x31,
            $ledger
        );
        $crate::operation_ledger_case!(
            many_occurrences_in_one_node_are_independent_listed_slots,
            0x40,
            $ledger
        );
        $crate::operation_ledger_case!(
            the_provider_key_is_durable_and_part_of_the_prepare_identity,
            0x41,
            $ledger
        );
        $crate::operation_ledger_case!(
            outstanding_residue_explained_ambiguous_by_a_later_attempt,
            0x42,
            $ledger
        );
        $crate::operation_ledger_case!(
            regrant_budget_is_cumulative_across_attempt_generations,
            0x43,
            $ledger
        );
        $crate::operation_ledger_case!(
            occurrence_labels_are_validated_before_durable_access,
            0x44,
            $ledger
        );
        $crate::operation_ledger_case!(a_scoped_ledger_lists_no_foreign_occurrences, 0x45, $ledger);
        $crate::operation_ledger_case!(
            the_budget_counts_only_possibly_crossed_calls,
            0x46,
            $ledger
        );
        $crate::operation_ledger_case!(
            a_never_crossed_slot_outlives_its_recovery_window,
            0x47,
            $ledger
        );
        $crate::operation_ledger_case!(
            a_never_crossed_stable_key_ignores_its_key_window,
            0x48,
            $ledger
        );
        $crate::operation_ledger_case!(
            the_concurrent_floor_is_durable_and_outside_the_prepare_identity,
            0x49,
            $ledger
        );
    };
}

pub(crate) async fn bounded_permits_preserve_identity_and_frozen_evidence(
    ledger: &impl LedgerUnderTest,
    executions: &dyn ExecutionStore,
    seed: u8,
) {
    use nebula_storage_port::dto::{EffectPhase, InvocationDisposition};
    let scope = scope();
    let (execution, slot, fence) = prepare_fresh(ledger, executions, seed).await;
    let initial = ledger.read_exact(&scope, slot).await.unwrap();
    let original_id = initial.operation().operation_id();
    let changed = binding(
        &scope,
        &execution,
        "once",
        1,
        0x11,
        DestinationCapability::Opaque,
    );
    assert!(matches!(
        ledger.prepare(&changed, fence).await,
        Err(OperationLedgerError::OperationMismatch { .. })
    ));
    assert_eq!(ledger.read_exact(&scope, slot).await.unwrap(), initial);
    let grant = OperationCommand::GrantInvocation {
        expected_revision: 0,
    };
    let (left, right) = tokio::join!(
        ledger.advance(&scope, slot, fence, &grant),
        ledger.advance(&scope, slot, fence, &grant)
    );
    assert_ne!(left.is_ok(), right.is_ok());
    let first = match left.or(right).unwrap() {
        OperationAdvance::Granted { call, .. } => call,
        _ => panic!("fresh grant"),
    };
    assert!(
        ledger
            .advance(
                &scope,
                slot,
                fence,
                &OperationCommand::GrantInvocation {
                    expected_revision: 1
                }
            )
            .await
            .is_err(),
        "outstanding call cannot be retried"
    );
    let before = OperationCommand::RecordDisposition {
        invocation: first,
        disposition: InvocationDisposition::BeforeBoundary,
    };
    ledger.advance(&scope, slot, fence, &before).await.unwrap();
    ledger.advance(&scope, slot, fence, &before).await.unwrap();
    let second = match ledger
        .advance(
            &scope,
            slot,
            fence,
            &OperationCommand::GrantInvocation {
                expected_revision: 2,
            },
        )
        .await
        .unwrap()
    {
        OperationAdvance::Granted { call, record, .. } => {
            assert_eq!(record.operation().operation_id(), original_id);
            call
        },
        _ => panic!("second bounded grant"),
    };
    assert!(
        ledger.advance(&scope, slot, fence, &before).await.is_err(),
        "delayed old disposition cannot reset a newer permit"
    );
    ledger
        .advance(
            &scope,
            slot,
            fence,
            &OperationCommand::RecordDisposition {
                invocation: second,
                disposition: InvocationDisposition::Ambiguous,
            },
        )
        .await
        .unwrap();
    let third = match ledger
        .advance(
            &scope,
            slot,
            fence,
            &OperationCommand::GrantInvocation {
                expected_revision: 4,
            },
        )
        .await
        .unwrap()
    {
        OperationAdvance::Granted { call, record, .. } => {
            assert_eq!(record.operation().operation_id(), original_id);
            call
        },
        _ => panic!("bounded stable recovery"),
    };
    ledger
        .advance(
            &scope,
            slot,
            fence,
            &OperationCommand::RecordDisposition {
                invocation: third,
                disposition: InvocationDisposition::Ambiguous,
            },
        )
        .await
        .unwrap();
    // The first call was proven not sent, so only two of the three permitted
    // calls are spent: one more may cross before the budget is exhausted.
    let fourth = granted_call(
        ledger
            .advance(
                &scope,
                slot,
                fence,
                &OperationCommand::GrantInvocation {
                    expected_revision: 6,
                },
            )
            .await
            .unwrap(),
    );
    ledger
        .advance(
            &scope,
            slot,
            fence,
            &OperationCommand::RecordDisposition {
                invocation: fourth,
                disposition: InvocationDisposition::Ambiguous,
            },
        )
        .await
        .unwrap();
    let exhausted = ledger
        .advance(
            &scope,
            slot,
            fence,
            &OperationCommand::GrantInvocation {
                expected_revision: 8,
            },
        )
        .await
        .unwrap();
    assert!(
        matches!(exhausted, OperationAdvance::Recorded(_)),
        "exhaustion grants no call"
    );
    let unknown = ledger.read_exact(&scope, slot).await.unwrap();
    assert_eq!(unknown.state(), OperationState::OutcomeUnknown);
    assert_eq!(
        unknown.protocol().unwrap().phase(),
        EffectPhase::OutcomeUnknown
    );
    assert_eq!(unknown.protocol().unwrap().invocations(), 4);
    assert_eq!(unknown.protocol().unwrap().not_crossed(), 1);
    assert!(
        ledger
            .advance(
                &scope,
                slot,
                fence,
                &OperationCommand::GrantInvocation {
                    expected_revision: 9
                }
            )
            .await
            .is_err()
    );
    let query = match ledger
        .advance(
            &scope,
            slot,
            fence,
            &OperationCommand::GrantReconciliation {
                expected_revision: 9,
            },
        )
        .await
        .unwrap()
    {
        OperationAdvance::ReconciliationGranted { call, .. } => call,
        _ => panic!("read-only reconciliation"),
    };
    ledger
        .advance(
            &scope,
            slot,
            fence,
            &OperationCommand::RecordReconciliationInconclusive { query },
        )
        .await
        .unwrap();
    assert!(
        ledger
            .advance(
                &scope,
                slot,
                fence,
                &OperationCommand::GrantInvocation {
                    expected_revision: 11
                }
            )
            .await
            .is_err(),
        "absence is not invocation authority"
    );
    let query = match ledger
        .advance(
            &scope,
            slot,
            fence,
            &OperationCommand::GrantReconciliation {
                expected_revision: 11,
            },
        )
        .await
        .unwrap()
    {
        OperationAdvance::ReconciliationGranted { call, .. } => call,
        _ => panic!("second bounded query"),
    };
    let frozen = FrozenOutcomeEvidence::v1_json(
        OutcomeEvidenceSource::Reconciliation(query),
        KnownOutcome::Succeeded,
        b"{\"result\":\"response-canary\"}".to_vec(),
    )
    .unwrap();
    let commit = OperationCommand::RecordOutcome(frozen.clone());
    ledger.advance(&scope, slot, fence, &commit).await.unwrap();
    let terminal = ledger.read_exact(&scope, slot).await.unwrap();
    ledger.advance(&scope, slot, fence, &commit).await.unwrap();
    assert_eq!(terminal, ledger.read_exact(&scope, slot).await.unwrap());
    assert_eq!(terminal.protocol().unwrap().evidence(), Some(&frozen));
    assert!(!format!("{terminal:?}").contains("response-canary"));
    let different = FrozenOutcomeEvidence::v1_json(
        OutcomeEvidenceSource::Reconciliation(query),
        KnownOutcome::Succeeded,
        b"{\"result\":\"changed\"}".to_vec(),
    )
    .unwrap();
    assert!(
        ledger
            .advance(
                &scope,
                slot,
                fence,
                &OperationCommand::RecordOutcome(different)
            )
            .await
            .is_err(),
        "same success enum is not exact evidence"
    );
    assert_eq!(terminal, ledger.read_exact(&scope, slot).await.unwrap());
}

pub(crate) async fn unknown_grant_ack_never_mints_replacement_authority(
    ledger: &impl LedgerUnderTest,
    executions: &dyn ExecutionStore,
    seed: u8,
) {
    let scope = scope();
    let (execution, slot, fence) = prepare_fresh(ledger, executions, seed).await;
    let natural = EffectOccurrenceKey::new(&scope, &execution, "charge", "once");
    assert_eq!(
        ledger
            .read_occurrence(&natural)
            .await
            .unwrap()
            .unwrap()
            .protocol()
            .unwrap()
            .invocations(),
        0
    );
    // Lose the fresh response; a stored grant projection cannot reconstruct egress authority.
    drop(
        ledger
            .advance(
                &scope,
                slot,
                fence,
                &OperationCommand::GrantInvocation {
                    expected_revision: 0,
                },
            )
            .await
            .unwrap(),
    );
    let durable = ledger.read_occurrence(&natural).await.unwrap().unwrap();
    let revision = durable.protocol().unwrap().revision();
    assert!(
        ledger
            .advance(
                &scope,
                slot,
                fence,
                &OperationCommand::GrantInvocation {
                    expected_revision: revision
                }
            )
            .await
            .is_err()
    );
    ledger
        .advance(
            &scope,
            slot,
            fence,
            &OperationCommand::MarkUnknown {
                expected_revision: revision,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        ledger.read_exact(&scope, slot).await.unwrap().state(),
        OperationState::OutcomeUnknown
    );
}

pub(crate) async fn opaque_ambiguity_and_expiry_forbid_second_effect(
    ledger: &impl LedgerUnderTest,
    executions: &dyn ExecutionStore,
    seed: u8,
) {
    use nebula_storage_port::dto::InvocationDisposition;
    let scope = scope();
    let execution = execution_id(seed);
    let fence = create_leased_execution(executions, &scope, &execution).await;
    for (occurrence, capability, window) in [
        ("opaque", DestinationCapability::Opaque, 60_000),
        ("expired", DestinationCapability::StableKey, 1),
    ] {
        let contract = PreparedEffectContract::new(
            fingerprint(0x77),
            {
                let builder = PreparedEffectPolicy::builder(capability)
                    .maximum_invocations(2)
                    .maximum_queries(0)
                    .recovery_window(std::time::Duration::from_mins(1));
                let builder = if capability == DestinationCapability::StableKey {
                    builder.stable_key_window(std::time::Duration::from_millis(window))
                } else {
                    builder
                };
                builder.build()
            }
            .unwrap(),
        )
        .unwrap();
        let mut binding = binding(&scope, &execution, occurrence, 1, 0x11, capability);
        binding.contract = &contract;
        let slot = ledger
            .prepare(&binding, fence)
            .await
            .unwrap()
            .operation()
            .slot_id();
        if capability == DestinationCapability::StableKey {
            // The key window binds only once a call may have crossed, so the
            // slot crosses ambiguously first and the window then expires.
            let call = granted_call(
                ledger
                    .advance(
                        &scope,
                        slot,
                        fence,
                        &OperationCommand::GrantInvocation {
                            expected_revision: 0,
                        },
                    )
                    .await
                    .unwrap(),
            );
            ledger
                .advance(
                    &scope,
                    slot,
                    fence,
                    &OperationCommand::RecordDisposition {
                        invocation: call,
                        disposition: InvocationDisposition::Ambiguous,
                    },
                )
                .await
                .unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            assert!(matches!(
                ledger
                    .advance(
                        &scope,
                        slot,
                        fence,
                        &OperationCommand::GrantInvocation {
                            expected_revision: 2
                        }
                    )
                    .await
                    .unwrap(),
                OperationAdvance::Recorded(_)
            ));
        } else {
            let call = match ledger
                .advance(
                    &scope,
                    slot,
                    fence,
                    &OperationCommand::GrantInvocation {
                        expected_revision: 0,
                    },
                )
                .await
                .unwrap()
            {
                OperationAdvance::Granted { call, .. } => call,
                _ => panic!("opaque initial grant"),
            };
            ledger
                .advance(
                    &scope,
                    slot,
                    fence,
                    &OperationCommand::RecordDisposition {
                        invocation: call,
                        disposition: InvocationDisposition::Ambiguous,
                    },
                )
                .await
                .unwrap();
        }
        let record = ledger.read_exact(&scope, slot).await.unwrap();
        assert_eq!(record.state(), OperationState::OutcomeUnknown);
        assert!(
            ledger
                .advance(
                    &scope,
                    slot,
                    fence,
                    &OperationCommand::GrantInvocation {
                        expected_revision: record.protocol().unwrap().revision()
                    }
                )
                .await
                .is_err()
        );
    }
}

/// Bind one shared case to a `#[tokio::test]` in the including backend file.
#[macro_export]
macro_rules! operation_ledger_case {
    ($case:ident, $seed:expr, $ledger:expr) => {
        #[tokio::test]
        async fn $case() {
            let Some((ledger, executions)) = $ledger.await else {
                panic!(concat!(
                    stringify!($case),
                    ": backend unreachable — the case cannot run and must fail rather than \
                     pass unchecked; reach the backend (set DATABASE_URL for postgres) or run \
                     without this feature"
                ));
            };
            oracle::$case(&ledger, &executions, $seed).await;
        }
    };
}

async fn create_leased_execution(
    executions: &dyn ExecutionStore,
    scope: &Scope,
    execution: &str,
) -> FencingToken {
    executions
        .create(
            scope,
            execution,
            "workflow",
            serde_json::json!({"status":"Created"}),
        )
        .await
        .unwrap();
    executions
        .acquire_lease(
            scope,
            execution,
            "runner",
            std::time::Duration::from_secs(30),
        )
        .await
        .unwrap()
        .unwrap()
}

pub(crate) async fn attempt_provenance_is_never_truncated(
    ledger: &impl LedgerUnderTest,
    executions: &dyn ExecutionStore,
    seed: u8,
) {
    let scope = scope();
    let execution = execution_id(seed);
    let fencing = create_leased_execution(executions, &scope, &execution).await;
    let boundary = u64::try_from(i64::MAX).unwrap();
    let valid = binding(
        &scope,
        &execution,
        "maximum",
        boundary,
        0x11,
        DestinationCapability::Opaque,
    );
    let prepared = ledger.prepare(&valid, fencing).await.unwrap().operation();
    assert_eq!(prepared.attempt_generation().get(), boundary);
    assert_eq!(
        ledger
            .read_occurrence(&valid.occurrence_key())
            .await
            .unwrap()
            .unwrap()
            .operation(),
        prepared
    );
    let invalid = binding(
        &scope,
        &execution,
        "overflow",
        boundary + 1,
        0x11,
        DestinationCapability::Opaque,
    );
    assert_eq!(
        ledger.prepare(&invalid, fencing).await,
        Err(OperationLedgerError::InvalidAttemptGeneration)
    );
    assert!(
        ledger
            .read_occurrence(&invalid.occurrence_key())
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        ledger
            .read_exact(&scope, prepared.slot_id())
            .await
            .unwrap()
            .operation(),
        prepared
    );
}

pub(crate) async fn execution_lease_is_authority_for_every_write(
    ledger: &impl LedgerUnderTest,
    executions: &dyn ExecutionStore,
    seed: u8,
) {
    let scope = scope();
    let execution = execution_id(seed);
    let request = binding(
        &scope,
        &execution,
        "once",
        1,
        0x11,
        DestinationCapability::Opaque,
    );
    let invented = FencingToken::from_generation(u64::MAX);
    assert_eq!(
        ledger.prepare(&request, invented).await,
        Err(OperationLedgerError::ExecutionLeaseRejected)
    );
    assert!(
        ledger
            .read_occurrence(&request.occurrence_key())
            .await
            .unwrap()
            .is_none()
    );
    let fencing = create_leased_execution(executions, &scope, &execution).await;
    assert_eq!(
        ledger.prepare(&request, invented).await,
        Err(OperationLedgerError::ExecutionLeaseRejected)
    );
    let foreign = other_scope();
    let foreign_request = binding(
        &foreign,
        &execution,
        "once",
        1,
        0x11,
        DestinationCapability::Opaque,
    );
    assert_eq!(
        ledger.prepare(&foreign_request, fencing).await,
        Err(OperationLedgerError::ExecutionLeaseRejected)
    );
    assert!(
        ledger
            .read_occurrence(&foreign_request.occurrence_key())
            .await
            .unwrap()
            .is_none()
    );
    // Discard the acknowledgement: reconciliation knows only the natural key.
    let _ = ledger.prepare(&request, fencing).await.unwrap();
    let prepared = ledger
        .read_occurrence(&request.occurrence_key())
        .await
        .unwrap()
        .unwrap();
    let slot = prepared.operation().slot_id();
    assert_eq!(
        ledger
            .record_known_outcome(&scope, slot, invented, KnownOutcome::Succeeded)
            .await,
        Err(OperationLedgerError::ExecutionLeaseRejected)
    );
    ledger
        .record_known_outcome(&scope, slot, fencing, KnownOutcome::Succeeded)
        .await
        .unwrap();
    executions
        .release_lease(&scope, &execution, fencing)
        .await
        .unwrap();
    assert_eq!(
        ledger
            .record_known_outcome(&scope, slot, fencing, KnownOutcome::Succeeded)
            .await,
        Err(OperationLedgerError::ExecutionLeaseRejected)
    );
    assert_eq!(
        ledger.prepare(&request, fencing).await,
        Err(OperationLedgerError::ExecutionLeaseRejected)
    );
    let short = executions
        .acquire_lease(
            &scope,
            &execution,
            "short",
            std::time::Duration::from_secs(1),
        )
        .await
        .unwrap()
        .unwrap();
    // All adapters clamp execution leases to at least one second.
    // Exercise expiry after that deadline, rather than a still-live lease.
    tokio::time::sleep(std::time::Duration::from_millis(1_100)).await;
    assert_eq!(
        ledger.prepare(&request, short).await,
        Err(OperationLedgerError::ExecutionLeaseRejected)
    );
    assert_eq!(
        ledger
            .record_known_outcome(&scope, slot, short, KnownOutcome::Succeeded)
            .await,
        Err(OperationLedgerError::ExecutionLeaseRejected)
    );
    let replacement = executions
        .acquire_lease(
            &scope,
            &execution,
            "replacement",
            std::time::Duration::from_secs(30),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        ledger
            .prepare(&request, replacement)
            .await
            .unwrap()
            .operation(),
        prepared.operation()
    );
    ledger
        .record_known_outcome(&scope, slot, replacement, KnownOutcome::Succeeded)
        .await
        .unwrap();
    assert_eq!(
        ledger.read_exact(&scope, slot).await.unwrap().state(),
        OperationState::Succeeded
    );
}

pub(crate) async fn adjudication_and_outcome_serialize_one_answer(
    ledger: &impl LedgerUnderTest,
    executions: &dyn ExecutionStore,
    seed: u8,
) {
    let scope = scope();
    let (_execution, slot, fencing) = prepare_fresh(ledger, executions, seed).await;
    ledger
        .advance(
            &scope,
            slot,
            fencing,
            &OperationCommand::GrantInvocation {
                expected_revision: 0,
            },
        )
        .await
        .unwrap();
    ledger
        .record_known_outcome(&scope, slot, fencing, KnownOutcome::OutcomeUnknown)
        .await
        .unwrap();
    let query = match ledger
        .advance(
            &scope,
            slot,
            fencing,
            &OperationCommand::GrantReconciliation {
                expected_revision: 2,
            },
        )
        .await
        .unwrap()
    {
        OperationAdvance::ReconciliationGranted { call, .. } => call,
        _ => panic!("fresh read-only grant required"),
    };
    let failure = OperationCommand::RecordOutcome(
        FrozenOutcomeEvidence::v1_json(
            OutcomeEvidenceSource::Reconciliation(query),
            KnownOutcome::Failed,
            b"{\"confirmed\":false}".to_vec(),
        )
        .unwrap(),
    );
    let (adjudication, competing) = tokio::join!(
        ledger.adjudicate_known(&scope, slot, KnownOutcome::Succeeded, "provider confirmed"),
        ledger.advance(&scope, slot, fencing, &failure)
    );
    assert_ne!(
        adjudication.is_ok(),
        competing.is_ok(),
        "exactly one authorized resolution wins"
    );
    assert_eq!(
        ledger.read_exact(&scope, slot).await.unwrap().state(),
        if adjudication.is_ok() {
            OperationState::Succeeded
        } else {
            OperationState::Failed
        }
    );
}

/// Release `fencing` and hand the execution to a later attempt, as crash
/// recovery does.
async fn take_over(
    executions: &dyn ExecutionStore,
    scope: &Scope,
    execution: &str,
    fencing: FencingToken,
) -> FencingToken {
    executions
        .release_lease(scope, execution, fencing)
        .await
        .unwrap();
    let next = executions
        .acquire_lease(
            scope,
            execution,
            "takeover",
            std::time::Duration::from_secs(30),
        )
        .await
        .unwrap()
        .unwrap();
    assert!(next.generation() > fencing.generation());
    next
}

fn granted_call(advance: OperationAdvance) -> OperationCallId {
    match advance {
        OperationAdvance::Granted { call, .. } => call,
        other => panic!("expected a fresh invocation grant, got {other:?}"),
    }
}

/// A contract with explicit limits, for the never-crossed cases.
fn limited_contract(
    capability: DestinationCapability,
    max_invocations: u32,
    recovery: std::time::Duration,
    stable: Option<std::time::Duration>,
) -> PreparedEffectContract {
    let builder = PreparedEffectPolicy::builder(capability)
        .maximum_invocations(max_invocations)
        .maximum_queries(0)
        .recovery_window(recovery);
    let builder = match stable {
        Some(window) => builder.stable_key_window(window),
        None => builder,
    };
    PreparedEffectContract::new(fingerprint(0x78), builder.build().unwrap()).unwrap()
}

/// Prepare `occurrence` of `execution` under `contract`.
async fn prepare_with(
    ledger: &impl LedgerUnderTest,
    scope: &Scope,
    execution: &str,
    occurrence: &str,
    contract: &PreparedEffectContract,
    fence: FencingToken,
) -> EffectSlotId {
    let capability = contract.policy().capability();
    let mut binding = binding(scope, execution, occurrence, 1, 0x11, capability);
    binding.contract = contract;
    ledger
        .prepare(&binding, fence)
        .await
        .unwrap()
        .operation()
        .slot_id()
}

/// Ask for an invocation permit at the slot's current revision.
async fn grant_now(
    ledger: &impl LedgerUnderTest,
    scope: &Scope,
    slot: EffectSlotId,
    fence: FencingToken,
) -> Result<OperationAdvance, OperationLedgerError> {
    let revision = ledger
        .read_exact(scope, slot)
        .await
        .unwrap()
        .protocol()
        .unwrap()
        .revision();
    ledger
        .advance(
            scope,
            slot,
            fence,
            &OperationCommand::GrantInvocation {
                expected_revision: revision,
            },
        )
        .await
}

/// Explain `call` with `disposition`.
async fn explain(
    ledger: &impl LedgerUnderTest,
    scope: &Scope,
    slot: EffectSlotId,
    fence: FencingToken,
    call: OperationCallId,
    disposition: nebula_storage_port::dto::InvocationDisposition,
) {
    ledger
        .advance(
            scope,
            slot,
            fence,
            &OperationCommand::RecordDisposition {
                invocation: call,
                disposition,
            },
        )
        .await
        .unwrap();
}

/// A call proven not to cross spends no invocation budget: a budget-one
/// slot survives any number of local refusals, and only a call that may have
/// crossed exhausts it.
pub(crate) async fn the_budget_counts_only_possibly_crossed_calls(
    ledger: &impl LedgerUnderTest,
    executions: &dyn ExecutionStore,
    seed: u8,
) {
    use nebula_storage_port::dto::{EffectPhase, InvocationDisposition};
    const REFUSALS: u32 = 5;
    let scope = scope();
    let execution = execution_id(seed);
    let fence = create_leased_execution(executions, &scope, &execution).await;

    for (occurrence, capability, stable) in [
        ("opaque", DestinationCapability::Opaque, None),
        (
            "stable",
            DestinationCapability::StableKey,
            Some(std::time::Duration::from_mins(1)),
        ),
    ] {
        let contract = limited_contract(capability, 1, std::time::Duration::from_mins(1), stable);
        let slot = prepare_with(ledger, &scope, &execution, occurrence, &contract, fence).await;
        for refusal in 1..=REFUSALS {
            let call = granted_call(grant_now(ledger, &scope, slot, fence).await.unwrap());
            explain(
                ledger,
                &scope,
                slot,
                fence,
                call,
                InvocationDisposition::BeforeBoundary,
            )
            .await;
            let protocol = ledger.read_exact(&scope, slot).await.unwrap();
            let protocol = protocol.protocol().unwrap();
            assert_eq!(protocol.phase(), EffectPhase::BeforeBoundary);
            assert_eq!(
                (protocol.invocations(), protocol.not_crossed()),
                (refusal, refusal)
            );
        }

        let crossing = granted_call(grant_now(ledger, &scope, slot, fence).await.unwrap());
        explain(
            ledger,
            &scope,
            slot,
            fence,
            crossing,
            InvocationDisposition::Ambiguous,
        )
        .await;
        if capability == DestinationCapability::StableKey {
            // Still inside its key window, but the one crossed call is the budget.
            assert!(matches!(
                grant_now(ledger, &scope, slot, fence).await.unwrap(),
                OperationAdvance::Recorded(_)
            ));
        }
        let record = ledger.read_exact(&scope, slot).await.unwrap();
        assert_eq!(
            record.state(),
            OperationState::OutcomeUnknown,
            "{occurrence}"
        );
        let protocol = record.protocol().unwrap();
        assert_eq!(protocol.phase(), EffectPhase::OutcomeUnknown);
        assert_eq!(protocol.invocations(), REFUSALS + 1);
        assert_eq!(protocol.crossed_invocations(), 1);
        assert!(grant_now(ledger, &scope, slot, fence).await.is_err());
    }
}

/// A slot that never crossed has sent nothing, so its recovery window never
/// expires it; the first ambiguous crossing still ends an opaque slot.
pub(crate) async fn a_never_crossed_slot_outlives_its_recovery_window(
    ledger: &impl LedgerUnderTest,
    executions: &dyn ExecutionStore,
    seed: u8,
) {
    use nebula_storage_port::dto::{EffectPhase, InvocationDisposition};
    let scope = scope();
    let execution = execution_id(seed);
    let fence = create_leased_execution(executions, &scope, &execution).await;
    let contract = limited_contract(
        DestinationCapability::Opaque,
        1,
        std::time::Duration::from_millis(1),
        None,
    );
    let slot = prepare_with(ledger, &scope, &execution, "late", &contract, fence).await;

    for _ in 0..2 {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        let call = granted_call(grant_now(ledger, &scope, slot, fence).await.unwrap());
        explain(
            ledger,
            &scope,
            slot,
            fence,
            call,
            InvocationDisposition::BeforeBoundary,
        )
        .await;
    }
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    let crossing = granted_call(grant_now(ledger, &scope, slot, fence).await.unwrap());
    explain(
        ledger,
        &scope,
        slot,
        fence,
        crossing,
        InvocationDisposition::Ambiguous,
    )
    .await;
    let record = ledger.read_exact(&scope, slot).await.unwrap();
    assert_eq!(record.state(), OperationState::OutcomeUnknown);
    assert_eq!(
        record.protocol().unwrap().phase(),
        EffectPhase::OutcomeUnknown
    );
    assert!(grant_now(ledger, &scope, slot, fence).await.is_err());
}

/// A stable key is not yet in use until a call may have crossed, so its window
/// is ignored before the first crossing and enforced after it.
pub(crate) async fn a_never_crossed_stable_key_ignores_its_key_window(
    ledger: &impl LedgerUnderTest,
    executions: &dyn ExecutionStore,
    seed: u8,
) {
    use nebula_storage_port::dto::{EffectPhase, InvocationDisposition};
    let scope = scope();
    let execution = execution_id(seed);
    let fence = create_leased_execution(executions, &scope, &execution).await;
    let contract = limited_contract(
        DestinationCapability::StableKey,
        3,
        std::time::Duration::from_mins(1),
        Some(std::time::Duration::from_millis(1)),
    );
    let slot = prepare_with(ledger, &scope, &execution, "late-key", &contract, fence).await;

    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    let call = granted_call(grant_now(ledger, &scope, slot, fence).await.unwrap());
    explain(
        ledger,
        &scope,
        slot,
        fence,
        call,
        InvocationDisposition::BeforeBoundary,
    )
    .await;
    let crossing = granted_call(grant_now(ledger, &scope, slot, fence).await.unwrap());
    explain(
        ledger,
        &scope,
        slot,
        fence,
        crossing,
        InvocationDisposition::Ambiguous,
    )
    .await;
    assert_eq!(
        ledger
            .read_exact(&scope, slot)
            .await
            .unwrap()
            .protocol()
            .unwrap()
            .phase(),
        EffectPhase::Ambiguous
    );

    // Budget remains, but the key window has passed since a call may have crossed.
    assert!(matches!(
        grant_now(ledger, &scope, slot, fence).await.unwrap(),
        OperationAdvance::Recorded(_)
    ));
    let record = ledger.read_exact(&scope, slot).await.unwrap();
    assert_eq!(record.state(), OperationState::OutcomeUnknown);
    assert_eq!(record.protocol().unwrap().crossed_invocations(), 1);
}

/// Many effects of one node are many independent slots, and a listing of the
/// node returns exactly them, in preparation order.
pub(crate) async fn many_occurrences_in_one_node_are_independent_listed_slots(
    ledger: &impl LedgerUnderTest,
    executions: &dyn ExecutionStore,
    seed: u8,
) {
    use nebula_storage_port::dto::{EffectPhase, InvocationDisposition};
    let scope = scope();
    let execution = execution_id(seed);
    let fence = create_leased_execution(executions, &scope, &execution).await;
    let labels = [
        "unit/v1/db.main/orders.insert/#1",
        "unit/v1/db.main/orders.insert/#2",
        "unit/v1/db.main/orders.insert/#3",
    ];
    let mut prepared = Vec::new();
    for label in labels {
        let outcome = ledger
            .prepare(
                &binding(
                    &scope,
                    &execution,
                    label,
                    1,
                    0x11,
                    DestinationCapability::StableKey,
                ),
                fence,
            )
            .await
            .unwrap();
        assert!(
            matches!(outcome, PrepareOutcome::Prepared(_)),
            "identical request bytes under a new label are a new slot, got {outcome:?}"
        );
        prepared.push(outcome.operation());
    }
    for (index, left) in prepared.iter().enumerate() {
        for right in &prepared[index + 1..] {
            assert_ne!(left.slot_id(), right.slot_id());
            assert_ne!(left.operation_id(), right.operation_id());
        }
    }

    // Slots that must not be listed: another node of the same execution, and
    // the same node label in another execution.
    let sibling = EffectSlotBinding {
        node_key: "refund",
        ..binding(
            &scope,
            &execution,
            labels[0],
            1,
            0x11,
            DestinationCapability::StableKey,
        )
    };
    ledger.prepare(&sibling, fence).await.unwrap();
    let other_execution = format!("{execution}-other");
    let other_fence = create_leased_execution(executions, &scope, &other_execution).await;
    ledger
        .prepare(
            &binding(
                &scope,
                &other_execution,
                labels[0],
                1,
                0x11,
                DestinationCapability::StableKey,
            ),
            other_fence,
        )
        .await
        .unwrap();

    // Phases and budgets are per slot: spend a permit on the middle one only.
    let middle = prepared[1].slot_id();
    let call = granted_call(
        ledger
            .advance(
                &scope,
                middle,
                fence,
                &OperationCommand::GrantInvocation {
                    expected_revision: 0,
                },
            )
            .await
            .unwrap(),
    );
    ledger
        .advance(
            &scope,
            middle,
            fence,
            &OperationCommand::RecordDisposition {
                invocation: call,
                disposition: InvocationDisposition::Ambiguous,
            },
        )
        .await
        .unwrap();

    let listed = ledger
        .read_occurrences(&scope, &execution, "charge")
        .await
        .unwrap();
    assert_eq!(
        listed
            .iter()
            .map(nebula_storage_port::EffectOccurrenceRecord::occurrence)
            .collect::<Vec<_>>(),
        labels,
        "every slot of the node, in preparation order, and nothing else"
    );
    for (listed, operation) in listed.iter().zip(&prepared) {
        assert_eq!(listed.record().operation(), *operation);
        assert_eq!(
            listed.record(),
            &ledger
                .read_exact(&scope, operation.slot_id())
                .await
                .unwrap()
        );
    }
    assert_eq!(
        listed
            .iter()
            .map(|occurrence| {
                let protocol = occurrence.record().protocol().unwrap();
                (protocol.phase(), protocol.invocations())
            })
            .collect::<Vec<_>>(),
        [
            (EffectPhase::Prepared, 0),
            (EffectPhase::Ambiguous, 1),
            (EffectPhase::Prepared, 0),
        ],
        "one slot's grant must not move or spend another slot's budget"
    );
    assert!(
        ledger
            .read_occurrences(&other_scope(), &execution, "charge")
            .await
            .unwrap()
            .is_empty(),
        "a foreign tenant must see no slot at all"
    );
    assert_eq!(
        ledger
            .read_occurrences(&scope, &execution, "refund")
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        ledger
            .read_occurrences(&scope, &other_execution, "charge")
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(
        ledger
            .read_occurrences(&scope, &execution, "absent")
            .await
            .unwrap()
            .is_empty()
    );
}

/// The provider idempotency key is recorded at preparation, read back on
/// replay, and part of the prepare identity — but never of the natural key.
pub(crate) async fn the_provider_key_is_durable_and_part_of_the_prepare_identity(
    ledger: &impl LedgerUnderTest,
    executions: &dyn ExecutionStore,
    seed: u8,
) {
    use nebula_storage_port::ProviderIdempotencyKey;
    let scope = scope();
    let execution = execution_id(seed);
    let fence = create_leased_execution(executions, &scope, &execution).await;
    let key = ProviderIdempotencyKey::new("47DEQpj8HBSa-_TImW-5JCeuQeRkm5NMpJWZG3hSuFU").unwrap();
    let other_key = ProviderIdempotencyKey::new("order-123").unwrap();
    let keyed = |occurrence: &'static str,
                 generation: u64,
                 digest: u8,
                 provider_key: Option<ProviderIdempotencyKey>| EffectSlotBinding {
        provider_key,
        ..binding(
            &scope,
            &execution,
            occurrence,
            generation,
            digest,
            DestinationCapability::StableKey,
        )
    };

    let prepared = ledger
        .prepare(&keyed("keyed", 1, 0x11, Some(key)), fence)
        .await
        .unwrap();
    assert!(matches!(prepared, PrepareOutcome::Prepared(_)));
    assert_eq!(prepared.operation().provider_key(), Some(key));
    let slot = prepared.operation().slot_id();
    let before = ledger.read_exact(&scope, slot).await.unwrap();
    assert_eq!(before.operation().provider_key(), Some(key));
    assert_eq!(before.protocol().unwrap().provider_key(), Some(key));
    assert_eq!(
        ledger
            .read_occurrence(&EffectOccurrenceKey::new(
                &scope, &execution, "charge", "keyed"
            ))
            .await
            .unwrap()
            .as_ref(),
        Some(&before)
    );

    // A retry reuses the key; the ledger hands back the recorded one.
    let replayed = ledger
        .prepare(&keyed("keyed", 2, 0x11, Some(key)), fence)
        .await
        .unwrap();
    assert_eq!(replayed, PrepareOutcome::Replayed(prepared.operation()));

    for (rejected, reason) in [
        (keyed("keyed", 1, 0x99, Some(key)), "a different request"),
        (keyed("keyed", 1, 0x11, Some(other_key)), "a different key"),
        (keyed("keyed", 1, 0x11, None), "a dropped key"),
    ] {
        assert_eq!(
            ledger.prepare(&rejected, fence).await,
            Err(OperationLedgerError::OperationMismatch { slot_id: slot }),
            "{reason} must fail closed"
        );
        assert_eq!(
            ledger.read_exact(&scope, slot).await.unwrap(),
            before,
            "{reason} must leave no durable delta"
        );
    }

    let keyless = ledger
        .prepare(&keyed("keyless", 1, 0x11, None), fence)
        .await
        .unwrap()
        .operation();
    assert_eq!(keyless.provider_key(), None);
    let keyless_before = ledger.read_exact(&scope, keyless.slot_id()).await.unwrap();
    assert_eq!(
        ledger
            .prepare(&keyed("keyless", 1, 0x11, Some(key)), fence)
            .await,
        Err(OperationLedgerError::OperationMismatch {
            slot_id: keyless.slot_id()
        }),
        "a slot prepared without a key cannot acquire one later"
    );
    assert_eq!(
        ledger.read_exact(&scope, keyless.slot_id()).await.unwrap(),
        keyless_before
    );

    // The key is not an address: another occurrence may carry the same key.
    let shared = ledger
        .prepare(&keyed("shared", 1, 0x11, Some(key)), fence)
        .await
        .unwrap();
    assert!(matches!(shared, PrepareOutcome::Prepared(_)));
    assert_ne!(shared.operation().slot_id(), slot);

    // Protocol transitions never rewrite the recorded key.
    let granted = ledger
        .advance(
            &scope,
            slot,
            fence,
            &OperationCommand::GrantInvocation {
                expected_revision: 0,
            },
        )
        .await
        .unwrap();
    let OperationAdvance::Granted { record, .. } = granted else {
        panic!("a fresh slot grants its first permit");
    };
    assert_eq!(record.operation().provider_key(), Some(key));
    let listed = ledger
        .read_occurrences(&scope, &execution, "charge")
        .await
        .unwrap();
    let keyed_listing = listed
        .iter()
        .find(|occurrence| occurrence.occurrence() == "keyed")
        .unwrap();
    assert_eq!(keyed_listing.record().operation().provider_key(), Some(key));
}

/// The owner's concurrency floor is recorded at the first preparation, read
/// back by every read and listing, kept across protocol transitions, and
/// never part of the prepare identity: a later prepare with another floor
/// replays the slot unchanged.
pub(crate) async fn the_concurrent_floor_is_durable_and_outside_the_prepare_identity(
    ledger: &impl LedgerUnderTest,
    executions: &dyn ExecutionStore,
    seed: u8,
) {
    let scope = scope();
    let execution = execution_id(seed);
    let fence = create_leased_execution(executions, &scope, &execution).await;
    let floored = |occurrence: &'static str, concurrent_floor: Option<u32>| EffectSlotBinding {
        concurrent_floor,
        ..binding(
            &scope,
            &execution,
            occurrence,
            1,
            0x11,
            DestinationCapability::StableKey,
        )
    };
    let floor = |record: &nebula_storage_port::dto::OperationRecord| {
        record.protocol().unwrap().concurrent_floor()
    };

    let prepared = ledger
        .prepare(&floored("floored", Some(3)), fence)
        .await
        .unwrap();
    let slot = prepared.operation().slot_id();
    let before = ledger.read_exact(&scope, slot).await.unwrap();
    assert_eq!(floor(&before), Some(3));

    for other in [Some(0), None] {
        assert_eq!(
            ledger.prepare(&floored("floored", other), fence).await,
            Ok(PrepareOutcome::Replayed(prepared.operation())),
            "{other:?}: not part of the prepare identity"
        );
        assert_eq!(ledger.read_exact(&scope, slot).await.unwrap(), before);
    }

    let granted = ledger
        .advance(
            &scope,
            slot,
            fence,
            &OperationCommand::GrantInvocation {
                expected_revision: 0,
            },
        )
        .await
        .unwrap();
    let OperationAdvance::Granted { record, .. } = granted else {
        panic!("a fresh slot grants its first permit");
    };
    assert_eq!(floor(&record), Some(3), "transitions keep the floor");

    let unfloored = ledger
        .prepare(&floored("unfloored", None), fence)
        .await
        .unwrap()
        .operation()
        .slot_id();
    let listed = ledger
        .read_occurrences(&scope, &execution, "charge")
        .await
        .unwrap();
    let listed_floor = |label: &str| {
        listed
            .iter()
            .find(|occurrence| occurrence.occurrence() == label)
            .map(|occurrence| floor(occurrence.record()))
    };
    assert_eq!(listed_floor("floored"), Some(Some(3)));
    assert_eq!(listed_floor("unfloored"), Some(None));
    assert_eq!(
        floor(&ledger.read_exact(&scope, unfloored).await.unwrap()),
        None
    );
}

/// A later attempt that finds its predecessor's call outstanding may explain
/// it as ambiguous: a stable-key slot stays re-grantable in its window, an
/// opaque one becomes `OutcomeUnknown`.
pub(crate) async fn outstanding_residue_explained_ambiguous_by_a_later_attempt(
    ledger: &impl LedgerUnderTest,
    executions: &dyn ExecutionStore,
    seed: u8,
) {
    use nebula_storage_port::dto::{EffectPhase, InvocationDisposition};
    let scope = scope();
    let execution = execution_id(seed);
    let first = create_leased_execution(executions, &scope, &execution).await;
    let mut residues = Vec::new();
    for (occurrence, capability) in [
        ("stable", DestinationCapability::StableKey),
        ("opaque", DestinationCapability::Opaque),
    ] {
        let prepared = ledger
            .prepare(
                &binding(&scope, &execution, occurrence, 1, 0x11, capability),
                first,
            )
            .await
            .unwrap()
            .operation();
        let call = granted_call(
            ledger
                .advance(
                    &scope,
                    prepared.slot_id(),
                    first,
                    &OperationCommand::GrantInvocation {
                        expected_revision: 0,
                    },
                )
                .await
                .unwrap(),
        );
        residues.push((occurrence, capability, prepared, call));
    }

    // The first attempt dies with both calls outstanding.
    let second = take_over(executions, &scope, &execution, first).await;
    for (occurrence, capability, prepared, call) in residues {
        let slot = prepared.slot_id();
        let resumed = ledger
            .prepare(
                &binding(&scope, &execution, occurrence, 2, 0x11, capability),
                second,
            )
            .await
            .unwrap();
        assert_eq!(
            resumed,
            PrepareOutcome::Replayed(prepared),
            "a later attempt inherits the identity and original generation"
        );
        assert_eq!(
            ledger
                .read_exact(&scope, slot)
                .await
                .unwrap()
                .protocol()
                .unwrap()
                .phase(),
            EffectPhase::InvocationOutstanding
        );
        let explain = OperationCommand::RecordDisposition {
            invocation: call,
            disposition: InvocationDisposition::Ambiguous,
        };
        assert!(
            matches!(
                ledger.advance(&scope, slot, first, &explain).await,
                Err(OperationLedgerError::ExecutionLeaseRejected)
            ),
            "the dead attempt can no longer explain its own call"
        );
        ledger
            .advance(&scope, slot, second, &explain)
            .await
            .unwrap();
        let explained = ledger.read_exact(&scope, slot).await.unwrap();
        let protocol = explained.protocol().unwrap();
        let regrant = ledger
            .advance(
                &scope,
                slot,
                second,
                &OperationCommand::GrantInvocation {
                    expected_revision: protocol.revision(),
                },
            )
            .await;
        if capability == DestinationCapability::StableKey {
            assert_eq!(explained.state(), OperationState::Prepared);
            assert_eq!(protocol.phase(), EffectPhase::Ambiguous);
            match regrant {
                Ok(OperationAdvance::Granted {
                    call: retry,
                    record,
                    ..
                }) => {
                    assert_ne!(retry, call);
                    assert_eq!(
                        record.operation().operation_id(),
                        prepared.operation_id(),
                        "stable-key recovery re-invokes under the same identity"
                    );
                    assert_eq!(record.protocol().unwrap().invocations(), 2);
                },
                other => panic!("stable-key ambiguity must be re-grantable, got {other:?}"),
            }
        } else {
            assert_eq!(explained.state(), OperationState::OutcomeUnknown);
            assert_eq!(protocol.phase(), EffectPhase::OutcomeUnknown);
            assert!(
                matches!(regrant, Err(OperationLedgerError::ProtocolConflict)),
                "opaque ambiguity forbids a second effect"
            );
            assert_eq!(
                ledger.read_exact(&scope, slot).await.unwrap().state(),
                OperationState::OutcomeUnknown
            );
        }
    }
}

/// A slot's invocation budget spans attempt generations: a later attempt
/// re-grants only what earlier attempts left unspent.
///
/// Only calls that may have crossed spend the budget, and an opaque slot ends
/// at its first possible crossing, so the budget is exercised on a stable-key
/// slot whose every attempt crosses ambiguously.
pub(crate) async fn regrant_budget_is_cumulative_across_attempt_generations(
    ledger: &impl LedgerUnderTest,
    executions: &dyn ExecutionStore,
    seed: u8,
) {
    use nebula_storage_port::dto::{EffectPhase, InvocationDisposition};
    let scope = scope();
    let execution = execution_id(seed);
    let mut fence = create_leased_execution(executions, &scope, &execution).await;
    let request = |generation| {
        binding(
            &scope,
            &execution,
            "budget",
            generation,
            0x11,
            DestinationCapability::StableKey,
        )
    };
    let prepared = ledger
        .prepare(&request(1), fence)
        .await
        .unwrap()
        .operation();
    let slot = prepared.slot_id();
    // The oracle's stable-key contract permits three invocations in total.
    for generation in 1..=3_u64 {
        if generation > 1 {
            fence = take_over(executions, &scope, &execution, fence).await;
            assert_eq!(
                ledger.prepare(&request(generation), fence).await,
                Ok(PrepareOutcome::Replayed(prepared))
            );
        }
        let revision = ledger
            .read_exact(&scope, slot)
            .await
            .unwrap()
            .protocol()
            .unwrap()
            .revision();
        let call = granted_call(
            ledger
                .advance(
                    &scope,
                    slot,
                    fence,
                    &OperationCommand::GrantInvocation {
                        expected_revision: revision,
                    },
                )
                .await
                .unwrap(),
        );
        ledger
            .advance(
                &scope,
                slot,
                fence,
                &OperationCommand::RecordDisposition {
                    invocation: call,
                    disposition: InvocationDisposition::Ambiguous,
                },
            )
            .await
            .unwrap();
        let protocol = ledger
            .read_exact(&scope, slot)
            .await
            .unwrap()
            .protocol()
            .unwrap()
            .clone();
        assert_eq!(protocol.phase(), EffectPhase::Ambiguous);
        assert_eq!(
            u64::from(protocol.crossed_invocations()),
            generation,
            "each attempt spends from the one slot budget"
        );
    }

    fence = take_over(executions, &scope, &execution, fence).await;
    assert_eq!(
        ledger.prepare(&request(4), fence).await,
        Ok(PrepareOutcome::Replayed(prepared))
    );
    let revision = ledger
        .read_exact(&scope, slot)
        .await
        .unwrap()
        .protocol()
        .unwrap()
        .revision();
    let exhausted = ledger
        .advance(
            &scope,
            slot,
            fence,
            &OperationCommand::GrantInvocation {
                expected_revision: revision,
            },
        )
        .await
        .unwrap();
    assert!(
        matches!(exhausted, OperationAdvance::Recorded(_)),
        "a later attempt generation does not get a fresh budget"
    );
    let record = ledger.read_exact(&scope, slot).await.unwrap();
    assert_eq!(record.state(), OperationState::OutcomeUnknown);
    assert_eq!(record.protocol().unwrap().invocations(), 3);
    assert_eq!(record.operation(), prepared);
}

/// Inadmissible occurrence labels are refused before any durable access, on
/// prepare and read alike; admissible ones prepare normally.
pub(crate) async fn occurrence_labels_are_validated_before_durable_access(
    ledger: &impl LedgerUnderTest,
    executions: &dyn ExecutionStore,
    seed: u8,
) {
    use nebula_storage_port::{MAX_OCCURRENCE_LABEL_BYTES, OccurrenceLabelViolation};
    let scope = scope();
    let execution = execution_id(seed);
    let fence = create_leased_execution(executions, &scope, &execution).await;
    let too_long = "a".repeat(MAX_OCCURRENCE_LABEL_BYTES + 1);
    let longest = "a".repeat(MAX_OCCURRENCE_LABEL_BYTES);
    for (label, violation) in [
        ("", OccurrenceLabelViolation::Empty),
        (too_long.as_str(), OccurrenceLabelViolation::TooLong),
        ("tab\there", OccurrenceLabelViolation::InvalidByte),
        ("line\nbreak", OccurrenceLabelViolation::InvalidByte),
        ("two words", OccurrenceLabelViolation::InvalidByte),
        ("ünit", OccurrenceLabelViolation::InvalidByte),
    ] {
        assert_eq!(
            ledger
                .prepare(
                    &binding(
                        &scope,
                        &execution,
                        label,
                        1,
                        0x11,
                        DestinationCapability::Opaque
                    ),
                    fence
                )
                .await,
            Err(OperationLedgerError::InvalidOccurrence { violation }),
            "{label:?}"
        );
        assert!(matches!(
            ledger
                .read_occurrence(&EffectOccurrenceKey::new(
                    &scope, &execution, "charge", label
                ))
                .await,
            Err(OperationLedgerError::InvalidOccurrence { violation: rejected }) if rejected == violation
        ));
    }
    let admitted = [
        longest.as_str(),
        "node-effect/v1",
        "unit/v1/db.main/orders.insert/#3",
    ];
    for label in admitted {
        assert!(matches!(
            ledger
                .prepare(
                    &binding(
                        &scope,
                        &execution,
                        label,
                        1,
                        0x11,
                        DestinationCapability::Opaque
                    ),
                    fence
                )
                .await,
            Ok(PrepareOutcome::Prepared(_))
        ));
    }
    let listed = ledger
        .read_occurrences(&scope, &execution, "charge")
        .await
        .unwrap();
    assert_eq!(
        listed
            .iter()
            .map(nebula_storage_port::EffectOccurrenceRecord::occurrence)
            .collect::<Vec<_>>(),
        admitted,
        "only admissible labels were ever stored"
    );
}

/// Behind the tenancy decorator a caller-supplied scope cannot widen the bound
/// tenant: a foreign-bound ledger lists nothing of this tenant's node.
pub(crate) async fn a_scoped_ledger_lists_no_foreign_occurrences(
    ledger: &impl LedgerUnderTest,
    executions: &dyn ExecutionStore,
    seed: u8,
) {
    use nebula_tenancy::ScopedOperationLedger;
    let scope = scope();
    let execution = execution_id(seed);
    let fence = create_leased_execution(executions, &scope, &execution).await;
    for label in ["first", "second"] {
        ledger
            .prepare(
                &binding(
                    &scope,
                    &execution,
                    label,
                    1,
                    0x11,
                    DestinationCapability::Opaque,
                ),
                fence,
            )
            .await
            .unwrap();
    }
    let inner: std::sync::Arc<dyn OperationLedger> = std::sync::Arc::new(ledger.clone());
    let foreign = ScopedOperationLedger::new(inner.clone(), other_scope());
    assert!(
        foreign
            .read_occurrences(&scope, &execution, "charge")
            .await
            .unwrap()
            .is_empty(),
        "a caller-supplied scope must not reach another tenant's slots"
    );
    let owner = ScopedOperationLedger::new(inner, scope.clone());
    assert_eq!(
        owner
            .read_occurrences(&other_scope(), &execution, "charge")
            .await
            .unwrap()
            .len(),
        2,
        "the bound tenant is substituted for the caller's scope"
    );
}
