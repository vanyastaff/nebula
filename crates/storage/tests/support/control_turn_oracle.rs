use super::*;
use nebula_storage_port::{
    FencingToken, TransitionBatch,
    dto::ResumeTarget,
    store::{
        ControlObservationAcknowledgement as Ack, ControlTurnCommand, ControlTurnCommit,
        ControlTurnCommitOutcome as Outcome, ControlTurnTransition,
    },
};

pub(super) async fn command(
    ports: &Ports,
    seed: &Seed,
    kind: ControlCommand,
    target: Option<ResumeTarget>,
) -> ControlClaimToken {
    let msg = ControlMsg {
        id: ExecutionId::new().as_bytes(),
        execution_id: seed.execution.clone(),
        scope: seed.scope.clone(),
        command: kind,
        resume_target: target,
        w3c_traceparent: None,
        reclaim_count: 0,
    };
    ports.queue.enqueue(&msg).await.unwrap();
    let rows = ports
        .queue
        .claim_pending_for_flavor(&[0x81; 16], 1, seed.flavor)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].msg.id, msg.id);
    rows[0].token.clone()
}

pub(super) async fn run(ports: &Ports) {
    let seed = seed(ports).await;
    ports.queue.mark_completed(&seed.claim).await.unwrap();
    let fence = ports
        .execution
        .acquire_lease(
            &seed.scope,
            &seed.execution,
            "resume-owner",
            Duration::from_secs(30),
        )
        .await
        .unwrap()
        .unwrap();
    let target = ResumeTarget::Webhook {
        callback_id: "private-target-canary".into(),
    };
    let claim = command(ports, &seed, ControlCommand::Resume, Some(target.clone())).await;
    let original = ports
        .execution
        .get(&seed.scope, &seed.execution)
        .await
        .unwrap()
        .unwrap();
    let mut state = original.state.clone();
    state["arm_marker"] = serde_json::json!("durable existing arm");
    let outbox = ControlMsg {
        id: ExecutionId::new().as_bytes(),
        execution_id: seed.execution.clone(),
        scope: seed.scope.clone(),
        command: ControlCommand::Cancel,
        resume_target: None,
        w3c_traceparent: None,
        reclaim_count: 0,
    };
    let batch = TransitionBatch::builder()
        .scope(seed.scope.clone())
        .execution_id(&seed.execution)
        .expected_version(original.version)
        .fencing(fence)
        .state(
            state.clone(),
            nebula_storage_port::ExecutionListing::CREATED,
        )
        .outbox(vec![outbox.clone()])
        .build()
        .unwrap();
    let mut request = ControlTurnCommit::new(
        claim.clone(),
        seed.flavor,
        ControlTurnCommand::Resume {
            target: Some(target.clone()),
        },
        ControlTurnTransition::Checkpoint(&batch),
    );
    assert!(!format!("{request:?}").contains("private-target-canary"));
    request = request.with_command(ControlTurnCommand::Restart);
    assert_eq!(
        ports.handoff.commit_control_turn(&request).await.unwrap(),
        Outcome::ClaimSuperseded
    );
    request = request.with_command(ControlTurnCommand::Resume { target: None });
    assert_eq!(
        ports.handoff.commit_control_turn(&request).await.unwrap(),
        Outcome::ClaimSuperseded
    );
    request = request.with_command(ControlTurnCommand::Resume {
        target: Some(target),
    });
    request =
        request.with_worker_flavor_revision_id(WorkerFlavorRevisionId::from_bytes([0x89; 32]));
    use nebula_storage_port::store::{ControlFlavorRefusal, ControlFlavorRefusalOutcome as Flavor};
    let same = ControlFlavorRefusal::new(&claim, &seed.execution, seed.flavor);
    assert_eq!(
        ports
            .handoff
            .record_control_flavor_refusal(&same)
            .await
            .unwrap(),
        Flavor::NoMismatch
    );
    let actual = WorkerFlavorRevisionId::from_bytes([0x89; 32]);
    let preflight = ControlFlavorRefusal::new(&claim, &seed.execution, actual);
    let (first, second) = tokio::join!(
        ports.handoff.record_control_flavor_refusal(&preflight),
        ports.handoff.record_control_flavor_refusal(&preflight),
    );
    let replies = [first.unwrap(), second.unwrap()];
    assert_eq!(
        replies
            .iter()
            .filter(|reply| matches!(
                reply,
                Flavor::FlavorMismatch {
                    observation_acknowledgement: Ack::Recorded,
                    ..
                }
            ))
            .count(),
        1
    );
    assert_eq!(
        replies
            .iter()
            .filter(|reply| matches!(
                reply,
                Flavor::FlavorMismatch {
                    observation_acknowledgement: Ack::AlreadyRecorded,
                    ..
                }
            ))
            .count(),
        1
    );
    for reply in replies {
        assert!(
            matches!(reply, Flavor::FlavorMismatch { snapshot: Some(snapshot), .. } if snapshot.expected == seed.flavor && snapshot.actual == actual)
        );
    }
    let different = ControlFlavorRefusal::new(
        &claim,
        &seed.execution,
        WorkerFlavorRevisionId::from_bytes([0x8a; 32]),
    );
    assert!(
        matches!(ports.handoff.record_control_flavor_refusal(&different).await.unwrap(),
        Flavor::FlavorMismatch { snapshot: Some(snapshot), observation_acknowledgement: Ack::AlreadyRecorded, .. } if snapshot.expected == seed.flavor && snapshot.actual == actual),
        "dedup acknowledges original immutable snapshot, never the retry's changed runtime"
    );
    assert_eq!(
        ports
            .execution
            .get(&seed.scope, &seed.execution)
            .await
            .unwrap()
            .unwrap(),
        original,
        "preflight refusal never acquires or changes the owner lease, state or version"
    );
    let foreign_claim = ControlClaimToken::new(
        *claim.row_id(),
        claim.generation(),
        Scope::new(WorkspaceId::new().to_string(), OrgId::new().to_string()),
    );
    assert_eq!(
        ports
            .handoff
            .record_control_flavor_refusal(&ControlFlavorRefusal::new(
                &foreign_claim,
                &seed.execution,
                actual
            ))
            .await
            .unwrap(),
        Flavor::ClaimSuperseded
    );
    let mismatch = Outcome::FlavorMismatch {
        snapshot: Some(nebula_storage_port::store::FlavorMismatchSnapshot {
            expected: seed.flavor,
            actual: WorkerFlavorRevisionId::from_bytes([0x89; 32]),
        }),
        observation_acknowledgement: Ack::AlreadyRecorded,
    };
    let (first, second) = tokio::join!(
        ports.handoff.commit_control_turn(&request),
        ports.handoff.commit_control_turn(&request),
    );
    assert_eq!(first.unwrap(), mismatch);
    assert_eq!(second.unwrap(), mismatch);
    assert_eq!(
        ports.handoff.commit_control_turn(&request).await.unwrap(),
        mismatch
    );
    request =
        request.with_worker_flavor_revision_id(WorkerFlavorRevisionId::from_bytes([0x8b; 32]));
    assert_eq!(
        ports.handoff.commit_control_turn(&request).await.unwrap(),
        mismatch,
        "commit refusal replay returns original immutable receipt diagnostics"
    );
    let observations = refusal_observations(ports, &seed).await;
    assert_eq!(
        observations.len(),
        1,
        "concurrent and repeated mismatch appends one owner observation"
    );
    assert!(
        matches!(observations[0].reason(), nebula_execution::ExecutionControlReason::ExactFlavorMismatch { expected, actual }
        if *expected == seed.flavor && *actual == WorkerFlavorRevisionId::from_bytes([0x89; 32]))
    );
    request = request.with_worker_flavor_revision_id(seed.flavor);
    let foreign = Scope::new(WorkspaceId::new().to_string(), OrgId::new().to_string());
    request = request.with_transition(ControlTurnTransition::Unchanged {
        scope: &foreign,
        execution_id: &seed.execution,
        expected_version: original.version,
        fence,
    });
    assert_eq!(
        ports.handoff.commit_control_turn(&request).await.unwrap(),
        Outcome::ClaimSuperseded
    );
    assert_eq!(
        refusal_observations(ports, &seed).await.len(),
        1,
        "foreign scope cannot append to the real execution journal"
    );
    request = request.with_transition(ControlTurnTransition::Unchanged {
        scope: &seed.scope,
        execution_id: &seed.execution,
        expected_version: original.version + 1,
        fence,
    });
    assert_eq!(
        ports.handoff.commit_control_turn(&request).await.unwrap(),
        Outcome::VersionConflict {
            actual: original.version,
            observation_acknowledgement: Ack::Recorded,
        }
    );
    assert_eq!(
        ports.handoff.commit_control_turn(&request).await.unwrap(),
        Outcome::VersionConflict {
            actual: original.version,
            observation_acknowledgement: Ack::AlreadyRecorded
        }
    );
    let deferred = control_observations(ports, &seed)
        .await
        .into_iter()
        .filter(|entry| entry.outcome() == nebula_execution::ExecutionControlOutcome::Deferred)
        .count();
    assert_eq!(
        deferred, 1,
        "CAS refusal replay acknowledges one immutable durable decision"
    );
    request = request.with_transition(ControlTurnTransition::Unchanged {
        scope: &seed.scope,
        execution_id: &seed.execution,
        expected_version: original.version,
        fence: FencingToken::from_generation(fence.generation() + 1),
    });
    assert_eq!(
        ports.handoff.commit_control_turn(&request).await.unwrap(),
        Outcome::FencedOut {
            observation_acknowledgement: Ack::Recorded
        }
    );
    let (first, second) = tokio::join!(
        ports.handoff.commit_control_turn(&request),
        ports.handoff.commit_control_turn(&request),
    );
    assert_eq!(
        first.unwrap(),
        Outcome::FencedOut {
            observation_acknowledgement: Ack::AlreadyRecorded
        }
    );
    assert_eq!(
        second.unwrap(),
        Outcome::FencedOut {
            observation_acknowledgement: Ack::AlreadyRecorded
        }
    );
    let observations = refusal_observations(ports, &seed).await;
    assert_eq!(
        observations.len(),
        2,
        "one mismatch and one fenced observation, never repeated appends"
    );
    let fenced = observations
        .iter()
        .find(|o| o.outcome() == nebula_execution::ExecutionControlOutcome::Fenced)
        .unwrap();
    assert!(
        matches!(fenced.reason(), nebula_execution::ExecutionControlReason::LeaseFenced {
        attempted_execution_lease_generation, current_execution_lease_generation
    } if *attempted_execution_lease_generation == fence.generation() + 1 && *current_execution_lease_generation == fence.generation())
    );
    assert!(
        matches!(fenced.source(), nebula_execution::ExecutionControlSource::ControlQueue { row_id, queue_claim_generation }
        if row_id == claim.row_id() && *queue_claim_generation == claim.generation().get())
    );

    assert_eq!(
        ports
            .execution
            .get(&seed.scope, &seed.execution)
            .await
            .unwrap()
            .unwrap(),
        original
    );
    assert!(
        ports
            .recovery
            .list_recoverable_turns(seed.flavor, None, 256)
            .await
            .unwrap()
            .next_cursor()
            .is_none(),
        "rejections must not invent markers"
    );
    request = request.with_transition(ControlTurnTransition::Checkpoint(&batch));
    let mut collision = outbox.clone();
    collision.id = *seed.claim.row_id();
    let rejected_batch = TransitionBatch::builder()
        .scope(seed.scope.clone())
        .execution_id(&seed.execution)
        .expected_version(original.version)
        .fencing(fence)
        .state(
            state.clone(),
            nebula_storage_port::ExecutionListing::CREATED,
        )
        .outbox(vec![collision])
        .build()
        .unwrap();
    request = request.with_transition(ControlTurnTransition::Checkpoint(&rejected_batch));
    assert!(ports.handoff.commit_control_turn(&request).await.is_err());
    assert_eq!(
        ports
            .execution
            .get(&seed.scope, &seed.execution)
            .await
            .unwrap()
            .unwrap(),
        original,
        "outbox failure must roll back state and CAS"
    );
    request = request.with_transition(ControlTurnTransition::Checkpoint(&batch));
    let (first, second) = tokio::join!(
        ports.handoff.commit_control_turn(&request),
        ports.handoff.commit_control_turn(&request)
    );
    let outcomes = [first.unwrap(), second.unwrap()];
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(outcome, Outcome::Accepted { .. }))
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| **outcome == Outcome::ClaimSuperseded)
            .count(),
        1
    );
    let persisted = ports
        .execution
        .get(&seed.scope, &seed.execution)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(persisted.state, state);
    assert_eq!(persisted.version, original.version + 1);
    assert_eq!(
        persisted.fencing, original.fencing,
        "live owner fence must not be replaced"
    );
    assert_eq!(persisted.lease_holder, original.lease_holder);
    let observed = control_observations(ports, &seed).await;
    let accepted: Vec<_> = observed
        .iter()
        .filter(|entry| entry.outcome() == nebula_execution::ExecutionControlOutcome::Accepted)
        .collect();
    assert_eq!(
        accepted.len(),
        1,
        "exactly one Resume commit records success atomically"
    );
    assert!(
        matches!(accepted[0].source(),nebula_execution::ExecutionControlSource::ControlQueue { row_id,queue_claim_generation } if row_id==claim.row_id() && *queue_claim_generation==claim.generation().get())
    );
    let node_a = nebula_core::node_key!("admission_a");
    let node_b = nebula_core::node_key!("admission_b");
    let admission = nebula_storage_port::store::ExecutionAdmissionRefusal::new(
        &seed.scope,
        &seed.execution,
        fence,
        &node_a,
        0,
    );
    use nebula_storage_port::store::ExecutionAdmissionRefusalOutcome as Admission;
    let first = ports
        .execution
        .record_execution_admission_refusal(&admission)
        .await
        .unwrap();
    let backend = match first {
        Admission::Attributed {
            backend,
            observation_acknowledgement: Ack::Recorded,
        } => backend,
        other => panic!("actual owner refused admission observation: {other:?}"),
    };
    assert_eq!(
        ports
            .execution
            .record_execution_admission_refusal(&admission)
            .await
            .unwrap(),
        Admission::Attributed {
            backend,
            observation_acknowledgement: Ack::AlreadyRecorded,
        }
    );
    // A sibling checkpoint advances the aggregate version between the throttle
    // and its observation. The refusal never changes the aggregate, so the
    // newer version must not stop it from being recorded.
    let sibling = TransitionBatch::builder()
        .scope(seed.scope.clone())
        .execution_id(&seed.execution)
        .expected_version(persisted.version)
        .fencing(fence)
        .state(
            persisted.state.clone(),
            nebula_storage_port::ExecutionListing::CREATED,
        )
        .build()
        .unwrap();
    assert!(matches!(
        ports.execution.commit(sibling).await.unwrap(),
        nebula_storage_port::TransitionOutcome::Applied { .. }
    ));
    // Observations below must leave this sibling-committed aggregate as is.
    let persisted = ports
        .execution
        .get(&seed.scope, &seed.execution)
        .await
        .unwrap()
        .unwrap();
    for (node, attempt) in [(&node_b, 0), (&node_a, 1)] {
        let next = nebula_storage_port::store::ExecutionAdmissionRefusal::new(
            &seed.scope,
            &seed.execution,
            fence,
            node,
            attempt,
        );
        assert_eq!(
            ports
                .execution
                .record_execution_admission_refusal(&next)
                .await
                .unwrap(),
            Admission::Attributed {
                backend,
                observation_acknowledgement: Ack::Recorded,
            }
        );
    }
    let denied = nebula_storage_port::store::ExecutionAdmissionRefusal::new(
        &foreign,
        &seed.execution,
        fence,
        &node_a,
        2,
    );
    assert_eq!(
        ports
            .execution
            .record_execution_admission_refusal(&denied)
            .await
            .unwrap(),
        Admission::FencedOut
    );
    let stale = nebula_storage_port::store::ExecutionAdmissionRefusal::new(
        &seed.scope,
        &seed.execution,
        FencingToken::from_generation(fence.generation() + 1),
        &node_a,
        2,
    );
    assert_eq!(
        ports
            .execution
            .record_execution_admission_refusal(&stale)
            .await
            .unwrap(),
        Admission::FencedOut
    );
    let observations = control_observations(ports, &seed).await;
    let throttled: Vec<_> = observations
        .iter()
        .filter(|observation| {
            observation.outcome() == nebula_execution::ExecutionControlOutcome::Throttled
        })
        .collect();
    assert_eq!(
        throttled.len(),
        3,
        "different node-attempts record, repeats and denied owners never append"
    );
    assert!(throttled.iter().all(|observation| matches!(observation.source(), nebula_execution::ExecutionControlSource::AcceptedTurn {
        source_kind: nebula_execution::ExecutionControlQueueKind::ControlQueue,
        source_row_id,
        accepted_execution_lease_generation,
    } if source_row_id == claim.row_id() && *accepted_execution_lease_generation == fence.generation())));
    assert!(
        throttled
            .iter()
            .all(|observation| observation.attempt().is_some())
    );
    assert_eq!(
        ports
            .execution
            .get(&seed.scope, &seed.execution)
            .await
            .unwrap()
            .unwrap(),
        persisted,
        "owner observation never changes aggregate state, lease or version"
    );
    assert!(matches!(
        ports.queue.mark_completed(&claim).await,
        Err(StorageError::FencedOut { .. })
    ));
    let appended = ports
        .queue
        .claim_pending_for_flavor(&[0x81; 16], 4, seed.flavor)
        .await
        .unwrap();
    assert_eq!(appended.len(), 1);
    assert_eq!(appended[0].msg, outbox);
    ports
        .queue
        .mark_completed(&appended[0].token)
        .await
        .unwrap();
    ports
        .execution
        .release_lease(&seed.scope, &seed.execution, fence)
        .await
        .unwrap();
    let recovered = ports
        .recovery
        .list_recoverable_turns(seed.flavor, None, 256)
        .await
        .unwrap();
    assert_eq!(recovered.turns().len(), 1);
    assert_eq!(
        recovered.turns()[0].accepted_fencing_generation(),
        fence.generation()
    );

    // Unchanged re-drive records acceptance without a fabricated checkpoint.
    for kind in [ControlCommand::Restart, ControlCommand::Resume] {
        let claim = command(ports, &seed, kind, None).await;
        let fence = ports
            .execution
            .acquire_lease(
                &seed.scope,
                &seed.execution,
                "redrive-owner",
                Duration::from_secs(30),
            )
            .await
            .unwrap()
            .unwrap();
        let snapshot = ports
            .execution
            .get(&seed.scope, &seed.execution)
            .await
            .unwrap()
            .unwrap();
        let request = ControlTurnCommit::new(
            claim,
            seed.flavor,
            if kind == ControlCommand::Restart {
                ControlTurnCommand::Restart
            } else {
                ControlTurnCommand::Resume { target: None }
            },
            ControlTurnTransition::Unchanged {
                scope: &seed.scope,
                execution_id: &seed.execution,
                expected_version: snapshot.version,
                fence,
            },
        );
        assert_eq!(
            ports.handoff.commit_control_turn(&request).await.unwrap(),
            Outcome::Accepted {
                fence,
                new_version: snapshot.version
            }
        );
        assert_eq!(
            ports
                .execution
                .get(&seed.scope, &seed.execution)
                .await
                .unwrap()
                .unwrap(),
            snapshot
        );
        ports
            .execution
            .release_lease(&seed.scope, &seed.execution, fence)
            .await
            .unwrap();
    }

    // A released/expired owner's still-current numeric generation is insufficient.
    let claim = command(ports, &seed, ControlCommand::Restart, None).await;
    let fence = ports
        .execution
        .acquire_lease(
            &seed.scope,
            &seed.execution,
            "short-owner",
            Duration::from_secs(1),
        )
        .await
        .unwrap()
        .unwrap();
    let request = ControlTurnCommit::new(
        claim.clone(),
        seed.flavor,
        ControlTurnCommand::Restart,
        ControlTurnTransition::Unchanged {
            scope: &seed.scope,
            execution_id: &seed.execution,
            expected_version: persisted.version,
            fence,
        },
    );
    tokio::time::sleep(Duration::from_millis(1100)).await;
    assert_eq!(
        ports.handoff.commit_control_turn(&request).await.unwrap(),
        Outcome::FencedOut {
            observation_acknowledgement: Ack::Recorded
        }
    );
    ports
        .execution
        .release_lease(&seed.scope, &seed.execution, fence)
        .await
        .unwrap();
    assert_eq!(
        ports.handoff.commit_control_turn(&request).await.unwrap(),
        Outcome::FencedOut {
            observation_acknowledgement: Ack::AlreadyRecorded
        }
    );
    assert_eq!(
        ports
            .queue
            .reclaim_stuck(Duration::ZERO, 8)
            .await
            .unwrap()
            .reclaimed,
        1
    );
    let replacement = ports
        .queue
        .claim_pending_for_flavor(&[0x82; 16], 1, seed.flavor)
        .await
        .unwrap();
    assert_eq!(replacement.len(), 1);
    assert!(replacement[0].token.generation().get() > claim.generation().get());
    let expected = Outcome::ClaimFenced {
        attempted_queue_claim_generation: claim.generation().get(),
        current_queue_claim_generation: replacement[0].token.generation().get(),
        observation_acknowledgement: Ack::AlreadyRecorded,
    };
    assert_eq!(
        ports.handoff.commit_control_turn(&request).await.unwrap(),
        expected
    );
    assert_eq!(
        ports.handoff.commit_control_turn(&request).await.unwrap(),
        expected
    );
    let stale = ControlFlavorRefusal::new(
        &claim,
        &seed.execution,
        WorkerFlavorRevisionId::from_bytes([0xa4; 32]),
    );
    assert_eq!(
        ports
            .handoff
            .record_control_flavor_refusal(&stale)
            .await
            .unwrap(),
        nebula_storage_port::store::ControlFlavorRefusalOutcome::ClaimFenced {
            attempted_queue_claim_generation: claim.generation().get(),
            current_queue_claim_generation: replacement[0].token.generation().get(),
            observation_acknowledgement: Ack::AlreadyRecorded,
        }
    );
    // An earlier phase fenced a different delivery of this execution; only
    // this claim's source may hold exactly one Fenced decision.
    let this_claim = nebula_execution::ExecutionControlSource::ControlQueue {
        row_id: *claim.row_id(),
        queue_claim_generation: claim.generation().get(),
    };
    assert_eq!(
        control_observations(ports, &seed)
            .await
            .into_iter()
            .filter(
                |entry| entry.outcome() == nebula_execution::ExecutionControlOutcome::Fenced
                    && entry.source() == &this_claim
            )
            .count(),
        1,
        "stale queue claim repeats acknowledge the original Fenced receipt without another durable decision"
    );
    ports
        .queue
        .mark_completed(&replacement[0].token)
        .await
        .unwrap();
}

async fn control_observations(
    ports: &Ports,
    seed: &Seed,
) -> Vec<nebula_execution::ExecutionControlObservationV1> {
    ports
        .journal
        .get_journal(&seed.scope, &seed.execution)
        .await
        .unwrap()
        .into_iter()
        .filter_map(|row| {
            match serde_json::from_value::<nebula_execution::JournalEntry>(row.payload).unwrap() {
                nebula_execution::JournalEntry::ControlObserved { observation, .. } => {
                    Some(observation)
                },
                _ => None,
            }
        })
        .collect()
}

async fn refusal_observations(
    ports: &Ports,
    seed: &Seed,
) -> Vec<nebula_execution::ExecutionControlObservationV1> {
    control_observations(ports, seed)
        .await
        .into_iter()
        .filter(|observation| {
            matches!(
                observation.outcome(),
                nebula_execution::ExecutionControlOutcome::Fenced
                    | nebula_execution::ExecutionControlOutcome::FlavorMismatch
            )
        })
        .collect()
}

/// Faults a backend test installs around a decided refusal's observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ObservationFault {
    /// The receipt insert fails inside the refusal transaction.
    FailReceiptWrite,
    /// The journal insert fails after the receipt was already written in the
    /// same transaction, so the receipt must roll back with it.
    FailJournalWrite,
    /// The refusal transaction's commit fails, so its acknowledgement is lost.
    LoseCommitAcknowledgement,
    /// Existing flavor-mismatch receipts keep their rows but their snapshot
    /// can no longer be decoded. Irreversible; installed last.
    CorruptReceiptSnapshot,
    /// Remove every reversible fault.
    Clear,
}

/// The faults every refusal kind must survive, with the acknowledgement the
/// still-definite decision must then carry.
const OBSERVATION_FAULTS: [(ObservationFault, Ack); 3] = [
    (ObservationFault::FailReceiptWrite, Ack::Unrecorded),
    (ObservationFault::FailJournalWrite, Ack::Unrecorded),
    (ObservationFault::LoseCommitAcknowledgement, Ack::Unknown),
];

/// Every refusal an owner seam decides before observing it.
#[derive(Debug, Clone, Copy)]
enum RefusalKind {
    /// Commit seam: the live owner's checkpoint version is stale.
    VersionConflict,
    /// Commit seam: the execution lease generation is stale.
    FencedOut,
    /// Commit seam: the delivery's queue claim was superseded.
    CommitClaimFenced,
    /// Commit seam: the delivery addresses an execution of another flavor.
    CommitFlavorMismatch,
    /// Flavor preflight seam: the delivery's queue claim was superseded.
    PreflightClaimFenced,
    /// Flavor preflight seam: the runtime is of another flavor.
    PreflightFlavorMismatch,
}

impl RefusalKind {
    const ALL: [Self; 6] = [
        Self::VersionConflict,
        Self::FencedOut,
        Self::CommitClaimFenced,
        Self::PreflightClaimFenced,
        // Flavor kinds run last: their deliveries stay claimed for the
        // unreadable-snapshot retry, and no later kind reclaims them.
        Self::CommitFlavorMismatch,
        Self::PreflightFlavorMismatch,
    ];

    const fn stale_claim(self) -> bool {
        matches!(self, Self::CommitClaimFenced | Self::PreflightClaimFenced)
    }
}

/// A decided refusal reduced to what the matrix compares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Refused {
    acknowledgement: Ack,
    snapshot: Option<nebula_storage_port::store::FlavorMismatchSnapshot>,
}

/// A refusal is decided before its observation is written. For every refusal
/// kind on both owner seams, each observation fault must still return the
/// definite refusal and leave nothing journaled, after which a clean retry
/// records the decision as new and a second one acknowledges it. An existing
/// receipt whose snapshot cannot be read reports no snapshot, never the
/// retrying runtime's. A throttle attributed to a current owner survives the
/// same faults.
pub(super) async fn refusals_survive_observation_faults(
    ports: &Ports,
    inject: impl AsyncFn(ObservationFault),
) {
    use nebula_storage_port::store::{
        ControlFlavorRefusal, ControlFlavorRefusalOutcome as Flavor, ExecutionAdmissionRefusal,
        ExecutionAdmissionRefusalOutcome as Admission, FlavorMismatchSnapshot,
    };
    let seed = seed(ports).await;
    ports.queue.mark_completed(&seed.claim).await.unwrap();
    let fence = ports
        .execution
        .acquire_lease(
            &seed.scope,
            &seed.execution,
            "fault-owner",
            Duration::from_secs(30),
        )
        .await
        .unwrap()
        .unwrap();
    let current = ports
        .execution
        .get(&seed.scope, &seed.execution)
        .await
        .unwrap()
        .unwrap();
    let other_flavor = WorkerFlavorRevisionId::from_bytes([0xb7; 32]);
    let decided_mismatch = FlavorMismatchSnapshot {
        expected: seed.flavor,
        actual: other_flavor,
    };
    let commit_with = |claim: &ControlClaimToken,
                       flavor: WorkerFlavorRevisionId,
                       expected_version: u64,
                       fence: FencingToken| {
        ControlTurnCommit::new(
            claim.clone(),
            flavor,
            ControlTurnCommand::Restart,
            ControlTurnTransition::Unchanged {
                scope: &seed.scope,
                execution_id: &seed.execution,
                expected_version,
                fence,
            },
        )
    };
    // Decide one refusal of `kind` for `claim` with the runtime `flavor`,
    // asserting the decision itself and returning its acknowledgement.
    let refuse = async |kind: RefusalKind,
                        claim: &ControlClaimToken,
                        current_claim: u64,
                        flavor: WorkerFlavorRevisionId|
           -> Refused {
        let commit = async |request: &ControlTurnCommit<'_>| {
            ports
                .handoff
                .commit_control_turn(request)
                .await
                .expect("an observation fault must never replace the refusal")
        };
        let preflight = || async {
            ports
                .handoff
                .record_control_flavor_refusal(&ControlFlavorRefusal::new(
                    claim,
                    &seed.execution,
                    flavor,
                ))
                .await
                .expect("an observation fault must never replace the flavor refusal")
        };
        let attempted = claim.generation().get();
        let plain = |acknowledgement| Refused {
            acknowledgement,
            snapshot: None,
        };
        match kind {
            RefusalKind::VersionConflict => {
                match commit(&commit_with(claim, seed.flavor, current.version + 7, fence)).await {
                    Outcome::VersionConflict {
                        actual,
                        observation_acknowledgement,
                    } if actual == current.version => plain(observation_acknowledgement),
                    other => panic!("{kind:?} decided {other:?}"),
                }
            },
            RefusalKind::FencedOut => {
                let stale = FencingToken::from_generation(fence.generation() + 1);
                match commit(&commit_with(claim, seed.flavor, current.version, stale)).await {
                    Outcome::FencedOut {
                        observation_acknowledgement,
                    } => plain(observation_acknowledgement),
                    other => panic!("{kind:?} decided {other:?}"),
                }
            },
            RefusalKind::CommitClaimFenced => {
                match commit(&commit_with(claim, seed.flavor, current.version, fence)).await {
                    Outcome::ClaimFenced {
                        attempted_queue_claim_generation,
                        current_queue_claim_generation,
                        observation_acknowledgement,
                    } if attempted_queue_claim_generation == attempted
                        && current_queue_claim_generation == current_claim =>
                    {
                        plain(observation_acknowledgement)
                    },
                    other => panic!("{kind:?} decided {other:?}"),
                }
            },
            RefusalKind::CommitFlavorMismatch => {
                match commit(&commit_with(claim, flavor, current.version, fence)).await {
                    Outcome::FlavorMismatch {
                        snapshot,
                        observation_acknowledgement,
                    } => Refused {
                        acknowledgement: observation_acknowledgement,
                        snapshot,
                    },
                    other => panic!("{kind:?} decided {other:?}"),
                }
            },
            RefusalKind::PreflightClaimFenced => match preflight().await {
                Flavor::ClaimFenced {
                    attempted_queue_claim_generation,
                    current_queue_claim_generation,
                    observation_acknowledgement,
                } if attempted_queue_claim_generation == attempted
                    && current_queue_claim_generation == current_claim =>
                {
                    plain(observation_acknowledgement)
                },
                other => panic!("{kind:?} decided {other:?}"),
            },
            RefusalKind::PreflightFlavorMismatch => match preflight().await {
                Flavor::FlavorMismatch {
                    snapshot,
                    backend,
                    observation_acknowledgement,
                } => {
                    assert_eq!(backend, ports.handoff.backend_kind());
                    Refused {
                        acknowledgement: observation_acknowledgement,
                        snapshot,
                    }
                },
                other => panic!("{kind:?} decided {other:?}"),
            },
        }
    };

    let mut mismatch_claims = Vec::new();
    for kind in RefusalKind::ALL {
        // Each kind refuses its own delivery, so receipts never collide.
        let claim = command(ports, &seed, ControlCommand::Restart, None).await;
        let (refused, current_claim, finish) = if kind.stale_claim() {
            assert_eq!(
                ports
                    .queue
                    .reclaim_stuck(Duration::ZERO, 8)
                    .await
                    .unwrap()
                    .reclaimed,
                1
            );
            let replacement = ports
                .queue
                .claim_pending_for_flavor(&[0x83; 16], 1, seed.flavor)
                .await
                .unwrap();
            assert_eq!(replacement.len(), 1);
            let token = replacement[0].token.clone();
            (claim, token.generation().get(), token)
        } else {
            let generation = claim.generation().get();
            (claim.clone(), generation, claim)
        };
        let flavor = if matches!(
            kind,
            RefusalKind::CommitFlavorMismatch | RefusalKind::PreflightFlavorMismatch
        ) {
            other_flavor
        } else {
            seed.flavor
        };
        let expected_snapshot = (flavor == other_flavor).then_some(decided_mismatch);
        let journal_before = control_observations(ports, &seed).await.len();
        for (fault, acknowledgement) in OBSERVATION_FAULTS {
            inject(fault).await;
            assert_eq!(
                refuse(kind, &refused, current_claim, flavor).await,
                Refused {
                    acknowledgement,
                    snapshot: expected_snapshot,
                },
                "{kind:?} under {fault:?}"
            );
            inject(ObservationFault::Clear).await;
            assert_eq!(
                control_observations(ports, &seed).await.len(),
                journal_before,
                "{kind:?} under {fault:?} journaled nothing"
            );
        }
        assert_eq!(
            refuse(kind, &refused, current_claim, flavor).await,
            Refused {
                acknowledgement: Ack::Recorded,
                snapshot: expected_snapshot,
            },
            "{kind:?}: every faulted observation rolled back, so the clean retry is new"
        );
        assert_eq!(
            refuse(kind, &refused, current_claim, flavor).await,
            Refused {
                acknowledgement: Ack::AlreadyRecorded,
                snapshot: expected_snapshot,
            },
            "{kind:?}: the clean observation is durable"
        );
        assert_eq!(
            control_observations(ports, &seed).await.len(),
            journal_before + 1,
            "{kind:?} journaled exactly one observation"
        );
        if flavor == other_flavor {
            mismatch_claims.push((kind, refused, current_claim));
        } else {
            ports.queue.mark_completed(&finish).await.unwrap();
        }
    }

    // A throttle attributed to the current owner survives the same faults.
    let accepted = command(ports, &seed, ControlCommand::Restart, None).await;
    assert!(matches!(
        ports
            .handoff
            .commit_control_turn(&commit_with(&accepted, seed.flavor, current.version, fence))
            .await
            .unwrap(),
        Outcome::Accepted { .. }
    ));
    let node = nebula_core::node_key!("throttled_node");
    let throttle = ExecutionAdmissionRefusal::new(&seed.scope, &seed.execution, fence, &node, 0);
    let throttled = async || match ports
        .execution
        .record_execution_admission_refusal(&throttle)
        .await
        .expect("an observation fault must never replace the throttle")
    {
        Admission::Attributed {
            observation_acknowledgement,
            ..
        } => observation_acknowledgement,
        other => panic!("current owner's throttle was not attributed: {other:?}"),
    };
    let journal_before = control_observations(ports, &seed).await.len();
    for (fault, acknowledgement) in OBSERVATION_FAULTS {
        inject(fault).await;
        assert_eq!(
            throttled().await,
            acknowledgement,
            "throttle under {fault:?}"
        );
        inject(ObservationFault::Clear).await;
    }
    assert_eq!(
        control_observations(ports, &seed).await.len(),
        journal_before
    );
    assert_eq!(throttled().await, Ack::Recorded);
    assert_eq!(throttled().await, Ack::AlreadyRecorded);

    // A retry from a different runtime meets an existing receipt whose
    // snapshot cannot be read: it is reported as recorded with no snapshot,
    // never with the retry's own values.
    let changed_runtime = WorkerFlavorRevisionId::from_bytes([0xb8; 32]);
    inject(ObservationFault::CorruptReceiptSnapshot).await;
    for (kind, claim, current_claim) in mismatch_claims {
        assert_eq!(
            refuse(kind, &claim, current_claim, changed_runtime).await,
            Refused {
                acknowledgement: Ack::AlreadyRecorded,
                snapshot: None,
            },
            "{kind:?}: an unreadable receipt snapshot is reported as absent"
        );
    }
    assert_eq!(
        ports
            .execution
            .get(&seed.scope, &seed.execution)
            .await
            .unwrap()
            .unwrap()
            .version,
        current.version,
        "refusals and throttles never change the aggregate"
    );
}
