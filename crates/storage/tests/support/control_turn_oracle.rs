use super::*;
use nebula_storage_port::{
    FencingToken, TransitionBatch,
    dto::ResumeTarget,
    store::{
        ControlTurnCommand, ControlTurnCommit, ControlTurnCommitOutcome as Outcome,
        ControlTurnTransition,
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
    assert_eq!(
        ports.handoff.commit_control_turn(&request).await.unwrap(),
        Outcome::ClaimSuperseded
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
    request = request.with_transition(ControlTurnTransition::Unchanged {
        scope: &seed.scope,
        execution_id: &seed.execution,
        expected_version: original.version + 1,
        fence,
    });
    assert_eq!(
        ports.handoff.commit_control_turn(&request).await.unwrap(),
        Outcome::VersionConflict {
            actual: original.version
        }
    );
    request = request.with_transition(ControlTurnTransition::Unchanged {
        scope: &seed.scope,
        execution_id: &seed.execution,
        expected_version: original.version,
        fence: FencingToken::from_generation(fence.generation() + 1),
    });
    assert_eq!(
        ports.handoff.commit_control_turn(&request).await.unwrap(),
        Outcome::FencedOut
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
        Outcome::FencedOut
    );
    ports
        .execution
        .release_lease(&seed.scope, &seed.execution, fence)
        .await
        .unwrap();
    assert_eq!(
        ports.handoff.commit_control_turn(&request).await.unwrap(),
        Outcome::FencedOut
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
    assert_eq!(
        ports.handoff.commit_control_turn(&request).await.unwrap(),
        Outcome::ClaimSuperseded
    );
    ports
        .queue
        .mark_completed(&replacement[0].token)
        .await
        .unwrap();
}
