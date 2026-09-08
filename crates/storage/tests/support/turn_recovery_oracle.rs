use super::*;
use nebula_storage_port::store::{RecoverableTurn, RecoveryTurnAcceptance, RecoveryTurnHandoff};

fn request<'a>(seed: &'a Seed, candidate: &RecoverableTurn) -> RecoveryTurnHandoff<'a> {
    RecoveryTurnHandoff::for_candidate(
        &seed.scope,
        &seed.execution,
        seed.flavor,
        candidate.accepted_fencing_generation(),
    )
    .at_version(0)
    .lease_to("recovery-owner", Duration::from_secs(30))
}

async fn duplicate(ports: &Ports, original: &Seed) -> Seed {
    let execution = ExecutionId::new();
    let stored = ports
        .execution
        .get(&original.scope, &original.execution)
        .await
        .unwrap()
        .unwrap();
    let mut state = stored.state;
    state["execution_id"] = serde_json::json!(execution);
    let stored = ports
        .starts
        .read_contract_bundle(&original.scope, &original.execution)
        .await
        .unwrap()
        .unwrap();
    let identity = StartContractIdentity::new(
        ExecutionContractBundleId::new(),
        stored.record().identity().revisions(),
    );
    let mut body: serde_json::Value = serde_json::from_slice(stored.record().bytes()).unwrap();
    body["bundle_id"] = serde_json::json!(identity.bundle_id());
    let bundle =
        ContractBundleRecord::v1_json(identity, serde_json::to_vec(&body).unwrap()).unwrap();
    let command = ControlMsg {
        id: execution.as_bytes(),
        execution_id: execution.to_string(),
        command: ControlCommand::Start,
        scope: original.scope.clone(),
        w3c_traceparent: None,
        reclaim_count: 0,
        resume_target: None,
    };
    ports
        .starts
        .materialize_start(&MaterializedStart::new(
            &original.scope,
            None,
            &execution.to_string(),
            NewExecution::new(state["workflow_id"].as_str().unwrap(), &state),
            &command,
            &bundle,
        ))
        .await
        .unwrap();
    let claim = ports
        .queue
        .claim_pending_for_flavor(&[0x72; 16], 1, original.flavor)
        .await
        .unwrap()[0]
        .token;
    Seed {
        scope: original.scope.clone(),
        execution: execution.to_string(),
        flavor: original.flavor,
        claim,
    }
}

pub(super) async fn run(ports: &Ports) {
    let original = seed(ports).await;
    // Ordinary historical leases on admitted rows do not establish acceptance.
    let technical = ports
        .execution
        .acquire_lease(
            &original.scope,
            &original.execution,
            "technical",
            Duration::from_secs(1),
        )
        .await
        .unwrap()
        .unwrap();
    ports
        .execution
        .release_lease(&original.scope, &original.execution, technical)
        .await
        .unwrap();
    assert!(
        ports
            .recovery
            .list_recoverable_turns(original.flavor, None, 256)
            .await
            .unwrap()
            .turns()
            .is_empty()
    );
    let mut handoff = original.request();
    handoff = handoff.with_lease_ttl(Duration::from_secs(1));
    let ControlStartAcceptance::Accepted { fence } =
        ports.handoff.accept_control_start(&handoff).await.unwrap()
    else {
        panic!("unowned current Start must be accepted")
    };
    let live = ports
        .recovery
        .list_recoverable_turns(original.flavor, None, 1)
        .await
        .unwrap();
    assert!(live.turns().is_empty());
    assert_eq!(live.next_cursor(), Some(original.execution.as_str()));
    assert!(
        ports
            .recovery
            .list_recoverable_turns(original.flavor, live.next_cursor(), 1)
            .await
            .unwrap()
            .next_cursor()
            .is_none()
    );
    tokio::time::sleep(Duration::from_millis(1_100)).await;
    let candidates = ports
        .recovery
        .list_recoverable_turns(original.flavor, None, 256)
        .await
        .unwrap();
    assert_eq!(candidates.turns().len(), 1);
    let candidate = &candidates.turns()[0];
    assert_eq!(candidate.accepted_fencing_generation(), fence.generation());
    let original_row = ports
        .execution
        .get(&original.scope, &original.execution)
        .await
        .unwrap()
        .unwrap();
    for mismatch in 0..4 {
        let mut recovery = request(&original, candidate);
        let foreign = Scope::new(WorkspaceId::new().to_string(), OrgId::new().to_string());
        match mismatch {
            0 => recovery = recovery.with_scope(&foreign),
            1 => {
                recovery = recovery
                    .with_worker_flavor_revision_id(WorkerFlavorRevisionId::from_bytes([0xFF; 32]));
            },
            2 => {
                let next_generation = recovery.expected_accepted_fencing_generation() + 1;
                recovery = recovery.with_expected_accepted_fencing_generation(next_generation);
            },
            3 => {
                let next_version = recovery.expected_execution_version() + 1;
                recovery = recovery.with_expected_execution_version(next_version);
            },
            _ => unreachable!(),
        }
        let decision = ports
            .recovery
            .accept_recovery_turn(&recovery)
            .await
            .unwrap();
        assert_eq!(
            decision,
            if mismatch == 3 {
                RecoveryTurnAcceptance::VersionConflict { actual: 0 }
            } else {
                RecoveryTurnAcceptance::CandidateSuperseded
            }
        );
        assert_eq!(
            ports
                .execution
                .get(&original.scope, &original.execution)
                .await
                .unwrap()
                .unwrap(),
            original_row
        );
    }
    let technical = ports
        .execution
        .acquire_lease(
            &original.scope,
            &original.execution,
            "later-technical-owner",
            Duration::from_secs(30),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        ports
            .recovery
            .accept_recovery_turn(&request(&original, candidate))
            .await
            .unwrap(),
        RecoveryTurnAcceptance::TurnHeldByAnotherOwner
    );
    ports
        .execution
        .release_lease(&original.scope, &original.execution, technical)
        .await
        .unwrap();
    let recovery = request(&original, candidate);
    let (left, right) = tokio::join!(
        ports.recovery.accept_recovery_turn(&recovery),
        ports.recovery.accept_recovery_turn(&recovery)
    );
    let outcomes = [left.unwrap(), right.unwrap()];
    let fences: Vec<_> = outcomes
        .iter()
        .filter_map(|outcome| {
            if let RecoveryTurnAcceptance::Accepted { fence } = outcome {
                Some(*fence)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(fences.len(), 1);
    assert!(fences[0].generation() > technical.generation());
    assert!(outcomes.contains(&RecoveryTurnAcceptance::CandidateSuperseded));
    assert_eq!(
        ports
            .recovery
            .accept_recovery_turn(&recovery)
            .await
            .unwrap(),
        RecoveryTurnAcceptance::CandidateSuperseded
    );
    ports
        .execution
        .release_lease(&original.scope, &original.execution, fences[0])
        .await
        .unwrap();
    let current = ports
        .recovery
        .list_recoverable_turns(original.flavor, None, 256)
        .await
        .unwrap();
    assert_eq!(
        current.turns()[0].accepted_fencing_generation(),
        fences[0].generation()
    );

    let later = duplicate(ports, &original).await;
    let ControlStartAcceptance::Accepted { fence: later_fence } = ports
        .handoff
        .accept_control_start(&later.request())
        .await
        .unwrap()
    else {
        panic!("second Start accepts")
    };
    // Exercise a page with one live prefix and a later expired accepted execution.
    let (prefix, suffix, prefix_fence, suffix_fence) = if original.execution < later.execution {
        (&original, &later, fences[0], later_fence)
    } else {
        (&later, &original, later_fence, fences[0])
    };
    ports
        .execution
        .release_lease(&suffix.scope, &suffix.execution, suffix_fence)
        .await
        .unwrap();
    let prefix_owner = ports
        .execution
        .acquire_lease(
            &prefix.scope,
            &prefix.execution,
            "page-prefix",
            Duration::from_secs(30),
        )
        .await
        .unwrap();
    assert!(prefix_owner.is_some() || prefix_fence == later_fence);
    let first = ports
        .recovery
        .list_recoverable_turns(original.flavor, None, 1)
        .await
        .unwrap();
    assert!(first.turns().is_empty());
    let second = ports
        .recovery
        .list_recoverable_turns(original.flavor, first.next_cursor(), 1)
        .await
        .unwrap();
    assert_eq!(second.turns().len(), 1);
    assert_eq!(second.turns()[0].execution_id(), suffix.execution);
    for limit in [0, 257] {
        assert!(
            ports
                .recovery
                .list_recoverable_turns(original.flavor, None, limit)
                .await
                .is_err()
        );
    }

    let job = seed(ports).await;
    ports.queue.mark_completed(&job.claim).await.unwrap();
    let plugin: nebula_core::PluginKey = "recovery".parse().unwrap();
    let message = nebula_storage_port::dto::JobDispatchMsg::new(
        ExecutionId::new().as_bytes(),
        job.execution.clone(),
        ControlCommand::Start,
        job.scope.clone(),
        serde_json::json!({}),
        None::<String>,
        plugin.clone(),
        vec![plugin.clone()],
        None::<String>,
        0,
        job.flavor,
    );
    ports.jobs.enqueue(&message).await.unwrap();
    let claim = ports
        .jobs
        .claim_pending(&[0x73; 16], 1, &[plugin], job.flavor)
        .await
        .unwrap()[0]
        .token;
    let nebula_storage_port::store::TurnAcceptance::Accepted { fence } = ports
        .handoff
        .accept_turn(
            &nebula_storage_port::store::TurnHandoff::for_claim(
                &job.scope,
                &job.execution,
                claim,
                job.flavor,
            )
            .lease_to("job-worker", Duration::from_secs(30)),
        )
        .await
        .unwrap()
    else {
        panic!("actual Job acceptance owns the turn")
    };
    ports
        .execution
        .release_lease(&job.scope, &job.execution, fence)
        .await
        .unwrap();
    let job_recovery = ports
        .recovery
        .list_recoverable_turns(job.flavor, None, 256)
        .await
        .unwrap();
    assert_eq!(job_recovery.turns().len(), 1);
    assert_eq!(job_recovery.turns()[0].execution_id(), job.execution);
}
