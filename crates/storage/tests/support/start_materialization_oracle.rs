use nebula_core::{
    ExecutablePlanRevisionId, ExecutionContractBundleId, ExecutionId, OrgId, PluginSetId,
    WorkerFlavorRevisionId, WorkflowId, WorkflowVersionId, WorkspaceId,
};
use nebula_storage_port::dto::{
    ContractBundleRecord, ControlCommand, ControlMsg, MaterializedStart, NewExecution,
    PlanFlavorRevisionIds, PlanFlavorRevisionRecord, RevisionRecordBytes, StartKey,
    WorkerFlavorRevisionRecord,
};
use nebula_storage_port::store::{
    ControlQueue, ExecutionStore, StartAcceptanceStore, StartContractIdentity, StartFingerprint,
    StartMaterialization, StartMaterializationError, StartRevisionRejection,
};
use nebula_storage_port::{
    BeginDrainOutcome, ExecutionReferenceTransition, PlanFlavorCatalogAdmin,
    PlanFlavorCatalogWriter, PlanFlavorRevisionTarget, Scope, TransitionBatch,
};

#[derive(Clone)]
struct Fixture {
    scope: Scope,
    execution_id: String,
    workflow_id: String,
    state: serde_json::Value,
    command: ControlMsg,
    bundle: ContractBundleRecord,
    pair: PlanFlavorRevisionRecord,
}
impl Fixture {
    fn new() -> Self {
        Self::with_plan_seed(51)
    }
    fn with_plan_seed(seed: u8) -> Self {
        let org = OrgId::new();
        let workspace = WorkspaceId::new();
        let scope = Scope::new(workspace.to_string(), org.to_string());
        let execution = ExecutionId::new();
        let workflow = WorkflowId::new();
        let revision = WorkflowVersionId::new();
        let plan = ExecutablePlanRevisionId::from_bytes([seed; 32]);
        let flavor = WorkerFlavorRevisionId::from_bytes([seed + 1; 32]);
        let plugins = PluginSetId::from_bytes([53; 32]);
        let identity = StartContractIdentity::new(
            ExecutionContractBundleId::new(),
            PlanFlavorRevisionIds::new(plan, flavor),
        );
        let bundle = ContractBundleRecord::v1_json(identity,serde_json::to_vec(&serde_json::json!({
            "bundle_id":identity.bundle_id(),"org_id":org,"workspace_id":workspace,"executable_plan_revision_id":plan,"plugin_set_id":plugins,
            "revisions":{"workflow":revision,"worker_flavor":flavor},"private":"secret-canary"
        })).unwrap()).unwrap();
        let state = serde_json::json!({"execution_id":execution,"workflow_id":workflow,"workflow_version_number":1,"executable_plan_revision_id":plan,"worker_flavor_revision_id":flavor,
            "status":"created","version":0,"node_states":{},"created_at":"2026-09-06T00:00:00Z","updated_at":"2026-09-06T00:00:00Z","started_at":null,"completed_at":null,
            "total_output_bytes":0,"total_retries":0,"terminated_by":null,"workflow_input":{"private":"secret-canary"}});
        let command = ControlMsg {
            id: execution.as_bytes(),
            execution_id: execution.to_string(),
            command: ControlCommand::Start,
            scope: scope.clone(),
            w3c_traceparent: None,
            reclaim_count: 0,
            resume_target: None,
        };
        let pair = PlanFlavorRevisionRecord::graph_v1_json(plan,RevisionRecordBytes::try_from_vec(serde_json::to_vec(&serde_json::json!({
            "claimed_id":plan,"workflow_version_id":revision,"worker_flavor_revision_id":flavor,"plugin_set_id":plugins,"manifest":{"workflow_id":workflow}
        })).unwrap()).unwrap(),WorkerFlavorRevisionRecord::v1_json(flavor,RevisionRecordBytes::try_from_vec(b"{}".to_vec()).unwrap()));
        Self {
            scope,
            execution_id: execution.to_string(),
            workflow_id: workflow.to_string(),
            state,
            command,
            bundle,
            pair,
        }
    }
    fn fresh_execution(&self) -> Self {
        let mut next = self.clone();
        let execution = ExecutionId::new();
        next.execution_id = execution.to_string();
        next.command.execution_id = execution.to_string();
        next.command.id = execution.as_bytes();
        next.state["execution_id"] = serde_json::json!(execution);
        let identity = StartContractIdentity::new(
            ExecutionContractBundleId::new(),
            self.bundle.identity().revisions(),
        );
        let mut body: serde_json::Value = serde_json::from_slice(self.bundle.bytes()).unwrap();
        body["bundle_id"] = serde_json::json!(identity.bundle_id());
        next.bundle =
            ContractBundleRecord::v1_json(identity, serde_json::to_vec(&body).unwrap()).unwrap();
        next
    }
    fn start<'a>(&'a self, key: Option<StartKey<'a>>) -> MaterializedStart<'a> {
        MaterializedStart::new(
            &self.scope,
            key,
            &self.execution_id,
            NewExecution::new(&self.workflow_id, &self.state),
            &self.command,
            &self.bundle,
        )
    }
    fn trigger_start<'a>(
        &'a self,
        key: nebula_storage_port::dto::TriggerStartKey<'a>,
    ) -> MaterializedStart<'a> {
        MaterializedStart::for_trigger(
            &self.scope,
            key,
            &self.execution_id,
            NewExecution::new(&self.workflow_id, &self.state),
            &self.command,
            &self.bundle,
        )
    }
}

pub(super) async fn trigger_replay(
    starts: &dyn StartAcceptanceStore,
    executions: &dyn ExecutionStore,
    queue: &dyn ControlQueue,
    writer: &dyn PlanFlavorCatalogWriter,
    admin: &dyn PlanFlavorCatalogAdmin,
    legacy: &dyn nebula_storage_port::store::TriggerDedupInbox,
    jobs: &dyn nebula_storage_port::store::JobDispatchQueue,
) {
    use nebula_storage_port::dto::{DispatchKind, TriggerDedupRow, TriggerStartKey};
    let fixture = Fixture::with_plan_seed(61);
    writer.insert(&fixture.pair).await.unwrap();
    let key = TriggerStartKey::new("trigger", "event");
    assert!(!format!("{key:?}").contains("event"));
    assert!(matches!(
        starts
            .materialize_start(&fixture.trigger_start(key))
            .await
            .unwrap(),
        StartMaterialization::Accepted { .. }
    ));
    assert_eq!(
        starts
            .lookup_trigger_start(&fixture.scope, &key)
            .await
            .unwrap(),
        Some(fixture.execution_id.clone())
    );
    let mut second = fixture.fresh_execution();
    second.state["workflow_input"] = serde_json::json!({"changed":"payload"});
    second.bundle =
        ContractBundleRecord::v1_json(second.bundle.identity(), b"{}".to_vec()).unwrap();
    assert_eq!(
        starts
            .materialize_start(&second.trigger_start(key))
            .await
            .unwrap(),
        StartMaterialization::Replayed {
            execution_id: fixture.execution_id.clone()
        }
    );
    assert!(
        executions
            .get(&fixture.scope, &second.execution_id)
            .await
            .unwrap()
            .is_none()
    );

    let caller = fixture.fresh_execution();
    let caller_key = StartKey::new("event", StartFingerprint::new(2, [91; 32]));
    assert!(matches!(
        starts
            .materialize_start(&caller.start(Some(caller_key)))
            .await
            .unwrap(),
        StartMaterialization::Accepted { .. }
    ));
    assert_eq!(
        starts
            .lookup_start(&fixture.scope, "event")
            .await
            .unwrap()
            .unwrap()
            .execution_id(),
        caller.execution_id
    );
    assert_eq!(
        starts
            .lookup_trigger_start(&fixture.scope, &key)
            .await
            .unwrap(),
        Some(fixture.execution_id.clone())
    );
    assert!(matches!(
        starts.materialize_start(&fixture.start(None)).await,
        Err(StartMaterializationError::MaterializationConflict)
    ));
    let different_key = TriggerStartKey::new("trigger", "different-event");
    assert!(matches!(
        starts
            .materialize_start(&fixture.trigger_start(different_key))
            .await,
        Err(StartMaterializationError::MaterializationConflict)
    ));
    assert!(
        starts
            .lookup_trigger_start(&fixture.scope, &different_key)
            .await
            .unwrap()
            .is_none()
    );

    let mut foreign = fixture.fresh_execution();
    foreign.scope = Scope::new(WorkspaceId::new().to_string(), OrgId::new().to_string());
    foreign.command.scope = foreign.scope.clone();
    let mut body: serde_json::Value = serde_json::from_slice(foreign.bundle.bytes()).unwrap();
    body["org_id"] = serde_json::json!(foreign.scope.org_id);
    body["workspace_id"] = serde_json::json!(foreign.scope.workspace_id);
    foreign.bundle = ContractBundleRecord::v1_json(
        foreign.bundle.identity(),
        serde_json::to_vec(&body).unwrap(),
    )
    .unwrap();
    assert!(
        starts
            .lookup_trigger_start(&foreign.scope, &key)
            .await
            .unwrap()
            .is_none()
    );
    assert!(matches!(
        starts
            .materialize_start(&foreign.trigger_start(key))
            .await
            .unwrap(),
        StartMaterialization::Accepted { .. }
    ));
    assert_eq!(
        starts
            .lookup_trigger_start(&foreign.scope, &key)
            .await
            .unwrap(),
        Some(foreign.execution_id.clone())
    );

    // Legacy first: replay preserves its execution, without synthesizing a bundle.
    let legacy_first = fixture.fresh_execution();
    let legacy_key = TriggerStartKey::new("trigger", "legacy-first");
    let legacy_row = TriggerDedupRow::new(
        legacy_key.trigger_id(),
        legacy_key.event_id(),
        fixture.scope.clone(),
        "2026-09-07T00:00:00Z",
    );
    let legacy_job = trigger_job(&legacy_first, legacy_key.event_id());
    let legacy_execution = NewExecution::new(&fixture.workflow_id, &legacy_first.state);
    assert_eq!(
        legacy
            .claim_and_materialize_start(Some(&legacy_row), &legacy_job, &legacy_execution)
            .await
            .unwrap()
            .kind,
        DispatchKind::Dispatched
    );
    assert_eq!(
        starts
            .materialize_start(&second.trigger_start(legacy_key))
            .await
            .unwrap(),
        StartMaterialization::Replayed {
            execution_id: legacy_first.execution_id.clone()
        }
    );
    assert!(
        starts
            .read_contract_bundle(&fixture.scope, &legacy_first.execution_id)
            .await
            .unwrap()
            .is_none()
    );
    // New owner first: the old ingress must not enqueue a second Start job.
    let row = TriggerDedupRow::new(
        key.trigger_id(),
        key.event_id(),
        fixture.scope.clone(),
        "2026-09-07T00:00:00Z",
    );
    let duplicate = legacy
        .claim_and_materialize_start(Some(&row), &legacy_job, &legacy_execution)
        .await
        .unwrap();
    assert_eq!(duplicate.kind, DispatchKind::Duplicate);
    assert_eq!(duplicate.execution_id, fixture.execution_id);

    let old_racer = fixture.fresh_execution();
    let new_racer = fixture.fresh_execution();
    let race_key = TriggerStartKey::new("trigger", "mixed-race");
    let race_row = TriggerDedupRow::new(
        race_key.trigger_id(),
        race_key.event_id(),
        fixture.scope.clone(),
        "2026-09-07T00:00:00Z",
    );
    let race_job = trigger_job(&old_racer, race_key.event_id());
    let old_execution = NewExecution::new(&fixture.workflow_id, &old_racer.state);
    let new_start = new_racer.trigger_start(race_key);
    let (old_result, new_result) = tokio::join!(
        legacy.claim_and_materialize_start(Some(&race_row), &race_job, &old_execution),
        starts.materialize_start(&new_start)
    );
    let old_result = old_result.unwrap();
    let new_result = new_result.unwrap();
    let legacy_won = match new_result {
        StartMaterialization::Accepted { execution_id } => {
            assert_eq!(old_result.kind, DispatchKind::Duplicate);
            assert_eq!(old_result.execution_id, execution_id);
            false
        },
        StartMaterialization::Replayed { execution_id } => {
            assert_eq!(old_result.kind, DispatchKind::Dispatched);
            assert_eq!(old_result.execution_id, execution_id);
            true
        },
        other => panic!("unexpected trigger race result {other:?}"),
    };
    let loser = if legacy_won {
        &new_racer.execution_id
    } else {
        &old_racer.execution_id
    };
    assert!(
        executions
            .get(&fixture.scope, loser)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        starts
            .lookup_trigger_start(&fixture.scope, &race_key)
            .await
            .unwrap(),
        Some(old_result.execution_id)
    );

    admin
        .begin_drain(PlanFlavorRevisionTarget::ExecutablePlan(
            fixture.pair.ids().plan(),
        ))
        .await
        .unwrap();
    let drained = fixture.fresh_execution();
    let drain_key = TriggerStartKey::new("trigger", "drained");
    assert!(matches!(
        starts
            .materialize_start(&drained.trigger_start(drain_key))
            .await
            .unwrap(),
        StartMaterialization::RevisionRejected(_)
    ));
    assert!(
        starts
            .lookup_trigger_start(&fixture.scope, &drain_key)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        executions
            .get(&fixture.scope, &drained.execution_id)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        starts
            .materialize_start(&second.trigger_start(key))
            .await
            .unwrap(),
        StartMaterialization::Replayed {
            execution_id: fixture.execution_id.clone()
        }
    );

    let claims = queue.claim_pending(&[71; 16], 100).await.unwrap();
    assert_eq!(
        claims
            .iter()
            .filter(|claim| claim.msg.execution_id == fixture.execution_id)
            .count(),
        1
    );
    assert!(
        !claims
            .iter()
            .any(|claim| claim.msg.execution_id == legacy_first.execution_id)
    );
    let plugin = nebula_core::PluginKey::new("core").unwrap();
    let job_claims = jobs
        .claim_pending(
            &[71; 16],
            100,
            &[plugin],
            fixture.pair.ids().worker_flavor(),
        )
        .await
        .unwrap();
    assert_eq!(job_claims.len(), if legacy_won { 2 } else { 1 });
    assert!(
        !job_claims
            .iter()
            .any(|claim| claim.msg.execution_id == fixture.execution_id
                || claim.msg.execution_id == caller.execution_id
                || claim.msg.execution_id == foreign.execution_id)
    );
}

fn trigger_job(fixture: &Fixture, event: &str) -> nebula_storage_port::dto::JobDispatchMsg {
    let plugin = nebula_core::PluginKey::new("core").unwrap();
    nebula_storage_port::dto::JobDispatchMsg::new(
        fixture.command.id,
        &fixture.execution_id,
        ControlCommand::Start,
        fixture.scope.clone(),
        serde_json::json!({}),
        Some(event),
        plugin.clone(),
        vec![plugin],
        None::<String>,
        0,
        fixture.pair.ids().worker_flavor(),
    )
}

pub(super) struct RunEvidence {
    #[cfg_attr(
        not(any(feature = "sqlite", feature = "postgres")),
        expect(
            dead_code,
            reason = "only deployment-backend cases inspect the stored bundle"
        )
    )]
    pub(super) stored: nebula_storage_port::dto::StoredContractBundle,
    pub(super) observations: serde_json::Value,
}

pub(super) async fn run(
    starts: &dyn StartAcceptanceStore,
    executions: &dyn ExecutionStore,
    queue: &dyn ControlQueue,
    writer: &dyn PlanFlavorCatalogWriter,
    admin: &dyn PlanFlavorCatalogAdmin,
) -> RunEvidence {
    let fixture = Fixture::new();
    writer.insert(&fixture.pair).await.unwrap();
    assert!(matches!(
        starts
            .materialize_start(&fixture.start(None))
            .await
            .unwrap(),
        StartMaterialization::Accepted { .. }
    ));
    assert!(matches!(
        starts
            .materialize_start(&fixture.start(None))
            .await
            .unwrap(),
        StartMaterialization::Replayed { .. }
    ));
    let stored = starts
        .read_contract_bundle(&fixture.scope, &fixture.execution_id)
        .await
        .unwrap()
        .unwrap();
    let initial_execution = executions
        .get(&fixture.scope, &fixture.execution_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.record(), &fixture.bundle);
    assert!(!format!("{stored:?}").contains("secret-canary"));
    let mut different_attempt = fixture.clone();
    different_attempt.command.id = [99; 16];
    assert!(matches!(
        starts
            .materialize_start(&different_attempt.start(None))
            .await,
        Err(StartMaterializationError::MaterializationConflict)
    ));

    let first = fixture.fresh_execution();
    let second = fixture.fresh_execution();
    let key = StartKey::new("concurrent", StartFingerprint::new(2, [17; 32]));
    let first_start = first.start(Some(key));
    let second_start = second.start(Some(key));
    let (first_result, second_result) = tokio::join!(
        starts.materialize_start(&first_start),
        starts.materialize_start(&second_start)
    );
    let first_result = first_result.unwrap();
    let second_result = second_result.unwrap();
    let winner = match (&first_result, &second_result) {
        (
            StartMaterialization::Accepted { execution_id },
            StartMaterialization::Replayed {
                execution_id: replayed,
            },
        )
        | (
            StartMaterialization::Replayed {
                execution_id: replayed,
            },
            StartMaterialization::Accepted { execution_id },
        ) => {
            assert_eq!(execution_id, replayed);
            execution_id.clone()
        },
        outcomes => panic!("expected one acceptance and one replay, got {outcomes:?}"),
    };
    let reservation = starts
        .lookup_start(&fixture.scope, "concurrent")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reservation.execution_id(), winner);
    assert_eq!(reservation.fingerprint(), key.fingerprint());
    let mut malformed_retry = fixture.fresh_execution();
    malformed_retry.bundle =
        ContractBundleRecord::v1_json(malformed_retry.bundle.identity(), b"{}".to_vec()).unwrap();
    assert_eq!(
        starts
            .materialize_start(&malformed_retry.start(Some(key)))
            .await
            .unwrap(),
        StartMaterialization::Replayed {
            execution_id: winner.clone()
        }
    );
    assert_eq!(
        starts
            .materialize_start(&malformed_retry.start(Some(StartKey::new(
                "concurrent",
                StartFingerprint::new(1, [17; 32])
            ))))
            .await
            .unwrap(),
        StartMaterialization::FingerprintMismatch
    );
    assert!(
        executions
            .get(&fixture.scope, &malformed_retry.execution_id)
            .await
            .unwrap()
            .is_none()
    );

    rejected_envelopes(starts, executions, &fixture).await;
    for collision in [true, false] {
        let mut candidate = fixture.fresh_execution();
        if collision {
            candidate.command.id = fixture.command.id;
        } else {
            candidate.bundle = fixture.bundle.clone();
        }
        assert!(matches!(
            starts
                .materialize_start(
                    &candidate.start(Some(StartKey::new("collision", key.fingerprint())))
                )
                .await,
            Err(StartMaterializationError::MaterializationConflict)
        ));
        assert!(
            starts
                .lookup_start(&fixture.scope, "collision")
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            executions
                .get(&fixture.scope, &candidate.execution_id)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            starts
                .read_contract_bundle(&fixture.scope, &candidate.execution_id)
                .await
                .unwrap()
                .is_none()
        );
    }
    let racer = fixture.fresh_execution();
    let racer_start = racer.start(Some(StartKey::new("drain-race", key.fingerprint())));
    let target = PlanFlavorRevisionTarget::ExecutablePlan(fixture.pair.ids().plan());
    let (drained, raced) = tokio::join!(
        admin.begin_drain(target),
        starts.materialize_start(&racer_start)
    );
    drained.unwrap();
    let accepted_count = match raced.unwrap() {
        StartMaterialization::Accepted { .. } => 3,
        StartMaterialization::RevisionRejected(_) => {
            assert!(
                starts
                    .lookup_start(&fixture.scope, "drain-race")
                    .await
                    .unwrap()
                    .is_none()
            );
            assert!(
                executions
                    .get(&fixture.scope, &racer.execution_id)
                    .await
                    .unwrap()
                    .is_none()
            );
            2
        },
        outcome => panic!("unexpected drain race outcome {outcome:?}"),
    };
    assert!(matches!(
        starts
            .materialize_start(&fixture.start(None))
            .await
            .unwrap(),
        StartMaterialization::Replayed { .. }
    ));
    assert_eq!(
        starts
            .materialize_start(&malformed_retry.start(Some(key)))
            .await
            .unwrap(),
        StartMaterialization::Replayed {
            execution_id: winner.clone()
        }
    );
    let counts = match admin.begin_drain(target).await.unwrap() {
        BeginDrainOutcome::Started(counts) | BeginDrainOutcome::AlreadyDraining(counts) => counts,
    };
    assert_eq!(counts.live_executions(), accepted_count);
    let lease = executions
        .acquire_lease(
            &fixture.scope,
            &fixture.execution_id,
            "terminal",
            std::time::Duration::from_secs(30),
        )
        .await
        .unwrap()
        .unwrap();
    let batch = TransitionBatch::builder()
        .scope(fixture.scope.clone())
        .execution_id(fixture.execution_id.clone())
        .expected_version(0)
        .fencing(lease)
        .new_state(serde_json::json!({"status":"Completed"}))
        .reference_transition(ExecutionReferenceTransition::ReleaseLive)
        .build()
        .unwrap();
    executions.commit(batch).await.unwrap();
    assert_eq!(
        starts
            .read_contract_bundle(&fixture.scope, &fixture.execution_id)
            .await
            .unwrap()
            .unwrap(),
        stored
    );
    assert!(matches!(
        starts
            .materialize_start(&fixture.start(None))
            .await
            .unwrap(),
        StartMaterialization::Replayed { .. }
    ));
    let counts = match admin.begin_drain(target).await.unwrap() {
        BeginDrainOutcome::Started(counts) | BeginDrainOutcome::AlreadyDraining(counts) => counts,
    };
    assert_eq!(counts.live_executions(), accepted_count - 1);
    let claims = queue.claim_pending(&[1; 16], 100).await.unwrap();
    assert_eq!(claims.len() as u64, accepted_count);
    assert_eq!(
        claims
            .iter()
            .filter(|claim| claim.msg.execution_id == fixture.execution_id)
            .count(),
        1
    );
    let foreign = Scope::new(WorkspaceId::new().to_string(), OrgId::new().to_string());
    assert!(
        starts
            .read_contract_bundle(&foreign, &fixture.execution_id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        starts
            .lookup_start(&foreign, "concurrent")
            .await
            .unwrap()
            .is_none()
    );
    let exact_route = exact_control_claim(starts, executions, queue, writer, admin).await;
    let identity = stored.record().identity();
    RunEvidence {
        stored,
        observations: serde_json::json!({
            "materialized_execution_id": fixture.execution_id,
            "bundle_revision": identity.bundle_id().to_string(),
            "plan_revision": identity.revisions().plan().to_string(),
            "flavor_revision": identity.revisions().worker_flavor().to_string(),
            "execution_state_plan_revision": initial_execution.state["executable_plan_revision_id"],
            "execution_state_flavor_revision": initial_execution.state["worker_flavor_revision_id"],
            "initial_execution_version": initial_execution.version,
            "keyed_winner_execution_id": reservation.execution_id(),
            "keyed_replay_execution_id": winner,
            "durable_drive_identities": 1,
            "fingerprint_mismatch_durable_delta": 0,
            "conflict_durable_delta": 0,
            "drain_race_accepted_count": accepted_count,
            "live_references_after_terminal": accepted_count - 1,
            "foreign_scope_bundle_visible": false,
            "exact_route": exact_route
        }),
    }
}

#[tokio::test]
async fn oversized_in_memory_start_leaves_no_revision_reference() {
    const EXECUTION_STATE_LIMIT: usize = 64 * 1024 * 1024;

    let executions = nebula_storage::InMemoryExecutionStore::new();
    let starts = nebula_storage::inmem::InMemoryStartAcceptanceStore::new(&executions);
    let catalog = executions.plan_flavor_catalog();
    let mut fixture = Fixture::new();
    catalog.insert(&fixture.pair).await.unwrap();
    fixture.state["workflow_input"] =
        serde_json::json!({"padding": "x".repeat(EXECUTION_STATE_LIMIT)});

    assert!(matches!(
        starts.materialize_start(&fixture.start(None)).await,
        Err(StartMaterializationError::Storage(
            nebula_storage_port::StorageError::Serialization(_)
        ))
    ));
    assert!(
        executions
            .get(&fixture.scope, &fixture.execution_id)
            .await
            .unwrap()
            .is_none()
    );

    let blockers = match catalog
        .begin_drain(PlanFlavorRevisionTarget::ExecutablePlan(
            fixture.pair.ids().plan(),
        ))
        .await
        .unwrap()
    {
        BeginDrainOutcome::Started(blockers) | BeginDrainOutcome::AlreadyDraining(blockers) => {
            blockers
        },
    };
    assert_eq!(blockers.live_executions(), 0);
}

async fn exact_control_claim(
    starts: &dyn StartAcceptanceStore,
    executions: &dyn ExecutionStore,
    queue: &dyn ControlQueue,
    writer: &dyn PlanFlavorCatalogWriter,
    admin: &dyn PlanFlavorCatalogAdmin,
) -> serde_json::Value {
    let mut wrong = flavor_fixture(70);
    wrong.command.id = [0; 16];
    let mut matching = flavor_fixture(72);
    matching.command.id = [0; 16];
    matching.command.id[15] = 4;
    for fixture in [&wrong, &matching] {
        writer.insert(&fixture.pair).await.unwrap();
        starts
            .materialize_start(&fixture.start(None))
            .await
            .unwrap();
    }
    let mut foreign = matching.command.clone();
    foreign.id[15] = 2;
    foreign.scope = wrong.scope.clone();
    queue.enqueue(&foreign).await.unwrap();
    let mut unpinned = matching.command.clone();
    unpinned.id[15] = 3;
    unpinned.execution_id = ExecutionId::new().to_string();
    executions
        .create(
            &unpinned.scope,
            &unpinned.execution_id,
            &matching.workflow_id,
            serde_json::json!({"status":"created"}),
        )
        .await
        .unwrap();
    queue.enqueue(&unpinned).await.unwrap();
    let before = executions
        .get(&matching.scope, &matching.execution_id)
        .await
        .unwrap()
        .unwrap();
    let claimed = queue
        .claim_pending_for_flavor(&[90; 16], 1, matching.pair.ids().worker_flavor())
        .await
        .unwrap();
    assert_eq!(claimed.len(), 1);
    assert_eq!(
        claimed[0].msg.id, matching.command.id,
        "wrong-flavor/unpinned/foreign-scope heads must not consume the batch limit"
    );
    assert_eq!(
        executions
            .get(&matching.scope, &matching.execution_id)
            .await
            .unwrap()
            .unwrap(),
        before
    );
    queue.mark_completed(&claimed[0].token).await.unwrap();
    // Existing references remain routable while their catalog is draining.
    admin
        .begin_drain(PlanFlavorRevisionTarget::WorkerFlavor(
            matching.pair.ids().worker_flavor(),
        ))
        .await
        .unwrap();
    let mut repeated = matching.command.clone();
    repeated.id[15] = 5;
    queue.enqueue(&repeated).await.unwrap();
    let claims = queue
        .claim_pending_for_flavor(&[91; 16], 1, matching.pair.ids().worker_flavor())
        .await
        .unwrap();
    assert_eq!(claims.len(), 1);
    assert_eq!(claims[0].msg.id, repeated.id);
    queue.mark_completed(&claims[0].token).await.unwrap();
    assert!(
        queue
            .claim_pending_for_flavor(&[92; 16], 1, matching.pair.ids().worker_flavor())
            .await
            .unwrap()
            .is_empty()
    );
    let wrong_claim = queue
        .claim_pending_for_flavor(&[93; 16], 1, wrong.pair.ids().worker_flavor())
        .await
        .unwrap();
    assert_eq!(wrong_claim.len(), 1);
    assert_eq!(wrong_claim[0].msg.id, wrong.command.id);
    assert_eq!(
        wrong_claim[0].token.generation().get(),
        1,
        "incompatible workers must not claim and release the head"
    );
    queue.mark_completed(&wrong_claim[0].token).await.unwrap();
    let lease = executions
        .acquire_lease(
            &matching.scope,
            &matching.execution_id,
            "terminal",
            std::time::Duration::from_secs(30),
        )
        .await
        .unwrap()
        .unwrap();
    executions
        .commit(
            TransitionBatch::builder()
                .scope(matching.scope.clone())
                .execution_id(matching.execution_id.clone())
                .expected_version(0)
                .fencing(lease)
                .new_state(serde_json::json!({"status":"Completed"}))
                .reference_transition(ExecutionReferenceTransition::ReleaseLive)
                .build()
                .unwrap(),
        )
        .await
        .unwrap();
    repeated.id[15] = 6;
    queue.enqueue(&repeated).await.unwrap();
    let (left, right) = tokio::join!(
        queue.claim_pending_for_flavor(&[94; 16], 1, matching.pair.ids().worker_flavor()),
        queue.claim_pending_for_flavor(&[95; 16], 1, matching.pair.ids().worker_flavor()),
    );
    let terminal_duplicate_claim = left.unwrap().into_iter().chain(right.unwrap()).next();
    assert!(
        terminal_duplicate_claim.is_none(),
        "released references must reject new terminal duplicate delivery"
    );
    let untouched = queue.claim_pending(&[96; 16], 2).await.unwrap();
    assert_eq!(
        untouched
            .iter()
            .map(|claim| claim.msg.id)
            .collect::<std::collections::BTreeSet<_>>(),
        [foreign.id, unpinned.id].into_iter().collect()
    );
    assert!(
        untouched
            .iter()
            .all(|claim| claim.token.generation().get() == 1)
    );

    let missing = flavor_fixture(74);
    let missing_outcome = starts.materialize_start(&missing.start(None)).await;
    let Ok(StartMaterialization::RevisionRejected(missing_rejection)) = missing_outcome else {
        panic!("a missing exact revision must be rejected before materialization");
    };
    assert_eq!(missing_rejection, StartRevisionRejection::PlanUnavailable);
    let missing_execution_visible = executions
        .get(&missing.scope, &missing.execution_id)
        .await
        .unwrap()
        .is_some();
    let missing_bundle_visible = starts
        .read_contract_bundle(&missing.scope, &missing.execution_id)
        .await
        .unwrap()
        .is_some();

    let draining = flavor_fixture(76);
    writer.insert(&draining.pair).await.unwrap();
    admin
        .begin_drain(PlanFlavorRevisionTarget::WorkerFlavor(
            draining.pair.ids().worker_flavor(),
        ))
        .await
        .unwrap();
    let draining_outcome = starts.materialize_start(&draining.start(None)).await;
    let Ok(StartMaterialization::RevisionRejected(draining_rejection)) = draining_outcome else {
        panic!("a draining exact revision must be rejected before materialization");
    };
    assert_eq!(draining_rejection, StartRevisionRejection::PairNotAdmitted);
    let draining_execution_visible = executions
        .get(&draining.scope, &draining.execution_id)
        .await
        .unwrap()
        .is_some();
    let draining_bundle_visible = starts
        .read_contract_bundle(&draining.scope, &draining.execution_id)
        .await
        .unwrap()
        .is_some();

    serde_json::json!({
        "required_flavor_revision": matching.pair.ids().worker_flavor().to_string(),
        "claimed_execution_id": matching.execution_id,
        "claimed_flavor_revision": matching.pair.ids().worker_flavor().to_string(),
        "wrong_flavor_execution_id": wrong.execution_id,
        "wrong_flavor_first_generation": wrong_claim[0].token.generation().get(),
        "unscoped_or_unpinned_rows_left_untouched": untouched.len(),
        "missing_revision_outcome": <&'static str>::from(missing_rejection),
        "missing_revision_execution_visible": missing_execution_visible,
        "missing_revision_bundle_visible": missing_bundle_visible,
        "draining_revision_outcome": <&'static str>::from(draining_rejection),
        "draining_revision_execution_visible": draining_execution_visible,
        "draining_revision_bundle_visible": draining_bundle_visible
    })
}

fn flavor_fixture(seed: u8) -> Fixture {
    let mut fixture = Fixture::new();
    let ids = PlanFlavorRevisionIds::new(
        ExecutablePlanRevisionId::from_bytes([seed; 32]),
        WorkerFlavorRevisionId::from_bytes([seed + 1; 32]),
    );
    let identity = StartContractIdentity::new(ExecutionContractBundleId::new(), ids);
    let mut bundle: serde_json::Value = serde_json::from_slice(fixture.bundle.bytes()).unwrap();
    bundle["bundle_id"] = serde_json::json!(identity.bundle_id());
    bundle["executable_plan_revision_id"] = serde_json::json!(ids.plan());
    bundle["revisions"]["worker_flavor"] = serde_json::json!(ids.worker_flavor());
    fixture.bundle =
        ContractBundleRecord::v1_json(identity, serde_json::to_vec(&bundle).unwrap()).unwrap();
    fixture.state["executable_plan_revision_id"] = serde_json::json!(ids.plan());
    fixture.state["worker_flavor_revision_id"] = serde_json::json!(ids.worker_flavor());
    let mut plan: serde_json::Value = serde_json::from_slice(fixture.pair.plan_bytes()).unwrap();
    plan["claimed_id"] = serde_json::json!(ids.plan());
    plan["worker_flavor_revision_id"] = serde_json::json!(ids.worker_flavor());
    fixture.pair = PlanFlavorRevisionRecord::graph_v1_json(
        ids.plan(),
        RevisionRecordBytes::try_from_vec(serde_json::to_vec(&plan).unwrap()).unwrap(),
        WorkerFlavorRevisionRecord::v1_json(
            ids.worker_flavor(),
            RevisionRecordBytes::try_from_vec(b"{}".to_vec()).unwrap(),
        ),
    );
    fixture
}

async fn rejected_envelopes(
    starts: &dyn StartAcceptanceStore,
    executions: &dyn ExecutionStore,
    fixture: &Fixture,
) {
    for (field, value) in [
        ("status", serde_json::json!("running")),
        ("version", serde_json::json!(1)),
        ("total_retries", serde_json::json!(1)),
        ("total_output_bytes", serde_json::json!(1)),
        ("lease_holder", serde_json::json!("worker")),
        (
            "lease_expires_at",
            serde_json::json!("2026-09-06T01:00:00Z"),
        ),
        ("terminated_by", serde_json::json!("caller")),
        ("completed_at", serde_json::json!("2026-09-06T01:00:00Z")),
        ("started_at", serde_json::json!("2026-09-06T01:00:00Z")),
        ("node_states", serde_json::json!({"node":{}})),
        ("execution_id", serde_json::json!(ExecutionId::new())),
    ] {
        let mut candidate = fixture.fresh_execution();
        candidate.state[field] = value;
        assert!(matches!(
            starts
                .materialize_start(&candidate.start(Some(StartKey::new(
                    "invalid",
                    StartFingerprint::new(2, [5; 32])
                ))))
                .await,
            Err(StartMaterializationError::InvalidEnvelope)
        ));
        assert!(
            starts
                .lookup_start(&fixture.scope, "invalid")
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            executions
                .get(&fixture.scope, &candidate.execution_id)
                .await
                .unwrap()
                .is_none()
        );
    }
    let mut wrong_tenant = fixture.fresh_execution();
    let mut body: serde_json::Value = serde_json::from_slice(wrong_tenant.bundle.bytes()).unwrap();
    body["org_id"] = serde_json::json!(OrgId::new());
    wrong_tenant.bundle = ContractBundleRecord::v1_json(
        wrong_tenant.bundle.identity(),
        serde_json::to_vec(&body).unwrap(),
    )
    .unwrap();
    assert!(matches!(
        starts.materialize_start(&wrong_tenant.start(None)).await,
        Err(StartMaterializationError::InvalidEnvelope)
    ));
    assert!(
        executions
            .get(&fixture.scope, &wrong_tenant.execution_id)
            .await
            .unwrap()
            .is_none()
    );
    for pointer in ["/revisions/workflow", "/plugin_set_id"] {
        let mut candidate = fixture.fresh_execution();
        let mut body: serde_json::Value = serde_json::from_slice(candidate.bundle.bytes()).unwrap();
        *body.pointer_mut(pointer).unwrap() = if pointer == "/revisions/workflow" {
            serde_json::json!(WorkflowVersionId::new())
        } else {
            serde_json::json!(PluginSetId::from_bytes([99; 32]))
        };
        candidate.bundle = ContractBundleRecord::v1_json(
            candidate.bundle.identity(),
            serde_json::to_vec(&body).unwrap(),
        )
        .unwrap();
        assert!(matches!(
            starts
                .materialize_start(&candidate.start(Some(StartKey::new(
                    "catalog-mismatch",
                    StartFingerprint::new(2, [5; 32])
                ))))
                .await,
            Err(StartMaterializationError::InvalidEnvelope)
        ));
        assert!(
            starts
                .lookup_start(&fixture.scope, "catalog-mismatch")
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            executions
                .get(&fixture.scope, &candidate.execution_id)
                .await
                .unwrap()
                .is_none()
        );
    }
}
