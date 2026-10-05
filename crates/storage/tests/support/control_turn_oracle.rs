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
        .new_state(state.clone())
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
            matches!(reply, Flavor::FlavorMismatch { expected, actual: got, .. } if expected == seed.flavor && got == actual)
        );
    }
    let different = ControlFlavorRefusal::new(
        &claim,
        &seed.execution,
        WorkerFlavorRevisionId::from_bytes([0x8a; 32]),
    );
    assert!(
        matches!(ports.handoff.record_control_flavor_refusal(&different).await.unwrap(),
        Flavor::FlavorMismatch { expected, actual: got, observation_acknowledgement: Ack::AlreadyRecorded, .. } if expected == seed.flavor && got == actual),
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
        expected: seed.flavor,
        actual: WorkerFlavorRevisionId::from_bytes([0x89; 32]),
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
        .new_state(state.clone())
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
        Admission::Recorded { backend } => backend,
        other => panic!("actual owner refused admission observation: {other:?}"),
    };
    assert_eq!(
        ports
            .execution
            .record_execution_admission_refusal(&admission)
            .await
            .unwrap(),
        Admission::AlreadyRecorded { backend }
    );
    // A sibling checkpoint advances the aggregate version between the throttle
    // and its observation. The refusal never changes the aggregate, so the
    // newer version must not stop it from being recorded.
    let sibling = TransitionBatch::builder()
        .scope(seed.scope.clone())
        .execution_id(&seed.execution)
        .expected_version(persisted.version)
        .fencing(fence)
        .new_state(persisted.state.clone())
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
            Admission::Recorded { backend }
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
#[derive(Debug, Clone, Copy)]
pub(super) enum ObservationFault {
    /// The receipt insert fails inside the refusal transaction.
    FailObservationWrite,
    /// The journal insert fails after the receipt was already written in the
    /// same transaction, so the receipt must roll back with it.
    FailJournalWrite,
    /// The refusal transaction's commit fails, so its acknowledgement is lost.
    LoseCommitAcknowledgement,
    /// Remove every installed fault.
    Clear,
}

/// A refusal is decided before its observation is written: a failed
/// observation write or a lost commit acknowledgement must still return the
/// definite refusal on every refusal branch of both owner seams (control turn
/// commit and flavor preflight), and leave aggregate and claim untouched.
pub(super) async fn refusals_survive_observation_faults(
    ports: &Ports,
    inject: impl AsyncFn(ObservationFault),
) {
    use nebula_storage_port::store::{ControlFlavorRefusal, ControlFlavorRefusalOutcome as Flavor};
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
    let other_flavor = WorkerFlavorRevisionId::from_bytes([0xb7; 32]);
    let claim = command(ports, &seed, ControlCommand::Restart, None).await;
    let current = ports
        .execution
        .get(&seed.scope, &seed.execution)
        .await
        .unwrap()
        .unwrap();
    let journal_before = control_observations(ports, &seed).await.len();
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
    let stale_version = commit_with(&claim, seed.flavor, current.version + 7, fence);
    let stale_fence = commit_with(
        &claim,
        seed.flavor,
        current.version,
        FencingToken::from_generation(fence.generation() + 1),
    );
    let wrong_flavor = commit_with(&claim, other_flavor, current.version, fence);
    let preflight = ControlFlavorRefusal::new(&claim, &seed.execution, other_flavor);
    let decide = async |request: &ControlTurnCommit<'_>| {
        ports
            .handoff
            .commit_control_turn(request)
            .await
            .expect("an observation fault must never replace the refusal")
    };
    let preflight_decision = async |request: &ControlFlavorRefusal<'_>| {
        ports
            .handoff
            .record_control_flavor_refusal(request)
            .await
            .expect("an observation fault must never replace the flavor refusal")
    };
    let mismatch = |acknowledgement: Ack| Outcome::FlavorMismatch {
        expected: seed.flavor,
        actual: other_flavor,
        observation_acknowledgement: acknowledgement,
    };
    let preflight_mismatch = |acknowledgement: Ack| Flavor::FlavorMismatch {
        expected: seed.flavor,
        actual: other_flavor,
        backend: ports.handoff.backend_kind(),
        observation_acknowledgement: acknowledgement,
    };

    // The receipt insert fails: every branch keeps its decision, unrecorded.
    inject(ObservationFault::FailObservationWrite).await;
    assert_eq!(
        decide(&stale_version).await,
        Outcome::VersionConflict {
            actual: current.version,
            observation_acknowledgement: Ack::Unrecorded,
        }
    );
    assert_eq!(decide(&wrong_flavor).await, mismatch(Ack::Unrecorded));
    assert_eq!(
        preflight_decision(&preflight).await,
        preflight_mismatch(Ack::Unrecorded)
    );
    inject(ObservationFault::Clear).await;

    // The receipt lands, then the journal insert fails: both roll back, so a
    // clean retry records the decision as new rather than as a replay.
    inject(ObservationFault::FailJournalWrite).await;
    assert_eq!(
        decide(&stale_version).await,
        Outcome::VersionConflict {
            actual: current.version,
            observation_acknowledgement: Ack::Unrecorded,
        }
    );
    inject(ObservationFault::Clear).await;
    assert_eq!(
        control_observations(ports, &seed).await.len(),
        journal_before,
        "nothing from a failed observation is journaled"
    );
    assert_eq!(
        decide(&stale_version).await,
        Outcome::VersionConflict {
            actual: current.version,
            observation_acknowledgement: Ack::Recorded,
        },
        "the receipt written before the failed journal insert was rolled back"
    );

    // The commit acknowledgement is lost: each decision stands, unknown.
    inject(ObservationFault::LoseCommitAcknowledgement).await;
    assert_eq!(
        decide(&stale_fence).await,
        Outcome::FencedOut {
            observation_acknowledgement: Ack::Unknown,
        }
    );
    assert_eq!(decide(&wrong_flavor).await, mismatch(Ack::Unknown));
    assert_eq!(
        preflight_decision(&preflight).await,
        preflight_mismatch(Ack::Unknown)
    );
    inject(ObservationFault::Clear).await;

    // A redelivery advances the claim: the stale delivery is fenced on both
    // seams whatever happens to the observation.
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
    let current_claim = replacement[0].token.generation().get();
    let fenced = |acknowledgement: Ack| Outcome::ClaimFenced {
        attempted_queue_claim_generation: claim.generation().get(),
        current_queue_claim_generation: current_claim,
        observation_acknowledgement: acknowledgement,
    };
    let preflight_fenced = |acknowledgement: Ack| Flavor::ClaimFenced {
        attempted_queue_claim_generation: claim.generation().get(),
        current_queue_claim_generation: current_claim,
        observation_acknowledgement: acknowledgement,
    };
    let stale_claim = commit_with(&claim, seed.flavor, current.version, fence);
    inject(ObservationFault::FailObservationWrite).await;
    assert_eq!(decide(&stale_claim).await, fenced(Ack::Unrecorded));
    assert_eq!(
        preflight_decision(&preflight).await,
        preflight_fenced(Ack::Unrecorded)
    );
    inject(ObservationFault::Clear).await;
    inject(ObservationFault::LoseCommitAcknowledgement).await;
    assert_eq!(decide(&stale_claim).await, fenced(Ack::Unknown));
    assert_eq!(
        preflight_decision(&preflight).await,
        preflight_fenced(Ack::Unknown)
    );
    inject(ObservationFault::Clear).await;

    assert_eq!(
        ports
            .execution
            .get(&seed.scope, &seed.execution)
            .await
            .unwrap()
            .unwrap()
            .version,
        current.version,
        "refusals never change the aggregate"
    );
    assert!(
        matches!(
            decide(&commit_with(
                &replacement[0].token,
                seed.flavor,
                current.version,
                fence
            ))
            .await,
            Outcome::Accepted { .. }
        ),
        "the live owner can still commit the current delivery"
    );
}
