//! Storage-boundary oracle for atomic Control Start ownership transfer.

use nebula_core::{
    ExecutablePlanRevisionId, ExecutionContractBundleId, ExecutionId, OrgId, PluginSetId,
    WorkerFlavorRevisionId, WorkflowId, WorkflowVersionId, WorkspaceId,
};
use nebula_storage_port::{
    Scope, StorageError,
    dto::{
        ContractBundleRecord, ControlCommand, ControlMsg, MaterializedStart, NewExecution,
        PlanFlavorRevisionIds, PlanFlavorRevisionRecord, RevisionRecordBytes,
        WorkerFlavorRevisionRecord,
    },
    store::{
        ControlClaimToken, ControlQueue, ControlStartAcceptance, ControlStartHandoff,
        ExecutionStore, ExecutionTurnHandoff, PlanFlavorCatalogAdmin, PlanFlavorCatalogWriter,
        StartAcceptanceStore, StartContractIdentity, StartMaterialization, TurnRecovery,
    },
};
use std::{sync::Arc, time::Duration};

#[path = "support/turn_recovery_oracle.rs"]
mod recovery_oracle;

#[path = "support/control_turn_oracle.rs"]
mod control_turn_oracle;

struct Ports {
    jobs: Arc<dyn nebula_storage_port::store::JobDispatchQueue>,
    execution: Arc<dyn ExecutionStore>,
    queue: Arc<dyn ControlQueue>,
    handoff: Arc<dyn ExecutionTurnHandoff>,
    recovery: Arc<dyn TurnRecovery>,
    starts: Arc<dyn StartAcceptanceStore>,
    catalog: Arc<dyn PlanFlavorCatalogWriter>,
    admin: Arc<dyn PlanFlavorCatalogAdmin>,
}

struct Seed {
    scope: Scope,
    execution: String,
    flavor: WorkerFlavorRevisionId,
    claim: ControlClaimToken,
}
impl Seed {
    fn request(&self) -> ControlStartHandoff<'_> {
        ControlStartHandoff::for_claim(
            &self.scope,
            &self.execution,
            self.claim.clone(),
            self.flavor,
        )
        .at_version(0)
        .lease_to("accepted-worker", Duration::from_secs(30))
    }
}

async fn seed(ports: &Ports) -> Seed {
    let execution = ExecutionId::new();
    let workflow = WorkflowId::new();
    let revision = WorkflowVersionId::new();
    let plan = ExecutablePlanRevisionId::from_bytes(std::array::from_fn(|index| {
        execution.as_bytes()[index % 16]
    }));
    let flavor = WorkerFlavorRevisionId::from_bytes(std::array::from_fn(|index| {
        workflow.as_bytes()[index % 16]
    }));
    let plugins = PluginSetId::from_bytes([0x63; 32]);
    let org = OrgId::new();
    let workspace = WorkspaceId::new();
    let scope = Scope::new(workspace.to_string(), org.to_string());
    let pair = PlanFlavorRevisionRecord::graph_v1_json(
        plan,
        RevisionRecordBytes::try_from_vec(
            serde_json::to_vec(&serde_json::json!({
                "claimed_id":plan,"workflow_version_id":revision,"worker_flavor_revision_id":flavor,
                "plugin_set_id":plugins,"manifest":{"workflow_id":workflow}
            }))
            .unwrap(),
        )
        .unwrap(),
        WorkerFlavorRevisionRecord::v1_json(
            flavor,
            RevisionRecordBytes::try_from_vec(b"{}".to_vec()).unwrap(),
        ),
    );
    ports.catalog.insert(&pair).await.unwrap();
    let identity = StartContractIdentity::new(
        ExecutionContractBundleId::new(),
        PlanFlavorRevisionIds::new(plan, flavor),
    );
    let bundle = ContractBundleRecord::v1_json(
        identity,
        serde_json::to_vec(&serde_json::json!({
            "bundle_id":identity.bundle_id(),"org_id":org,"workspace_id":workspace,
            "executable_plan_revision_id":plan,"plugin_set_id":plugins,
            "revisions":{"workflow":revision,"worker_flavor":flavor}
        }))
        .unwrap(),
    )
    .unwrap();
    let state = serde_json::json!({"execution_id":execution,"workflow_id":workflow,
        "workflow_version_number":1,"executable_plan_revision_id":plan,"worker_flavor_revision_id":flavor,
        "status":"created","version":0,"node_states":{},"created_at":"2026-09-06T00:00:00Z",
        "updated_at":"2026-09-06T00:00:00Z","started_at":null,"completed_at":null,
        "total_output_bytes":0,"total_retries":0,"terminated_by":null});
    let command = ControlMsg {
        id: execution.as_bytes(),
        execution_id: execution.to_string(),
        command: ControlCommand::Start,
        scope: scope.clone(),
        w3c_traceparent: None,
        reclaim_count: 0,
        resume_target: None,
    };
    assert!(matches!(
        ports
            .starts
            .materialize_start(&MaterializedStart::new(
                &scope,
                None,
                &execution.to_string(),
                NewExecution::new(&workflow.to_string(), &state),
                &command,
                &bundle
            ))
            .await
            .unwrap(),
        StartMaterialization::Accepted { .. }
    ));
    let claims = ports
        .queue
        .claim_pending_for_flavor(&[0x63; 16], 1, flavor)
        .await
        .unwrap();
    assert_eq!(claims.len(), 1);
    Seed {
        scope,
        execution: execution.to_string(),
        flavor,
        claim: claims[0].token.clone(),
    }
}

async fn accepted(ports: &Ports, seed: &Seed) {
    let before = ports
        .execution
        .get(&seed.scope, &seed.execution)
        .await
        .unwrap()
        .unwrap();
    let decision = ports
        .handoff
        .accept_control_start(&seed.request())
        .await
        .unwrap();
    let ControlStartAcceptance::Accepted { fence } = decision else {
        panic!("expected owned turn: {decision:?}")
    };
    let after = ports
        .execution
        .get(&seed.scope, &seed.execution)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.state, before.state);
    assert_eq!(after.version, before.version);
    assert_eq!(after.lease_holder.as_deref(), Some("accepted-worker"));
    assert_eq!(after.fencing, Some(fence.generation()));
    assert!(
        ports
            .execution
            .renew_lease(&seed.scope, &seed.execution, fence, Duration::from_secs(30))
            .await
            .unwrap()
    );
    assert!(matches!(
        ports.queue.mark_completed(&seed.claim).await,
        Err(StorageError::FencedOut { .. })
    ));
    assert_eq!(
        ports
            .queue
            .reclaim_stuck(Duration::ZERO, 8)
            .await
            .unwrap()
            .reclaimed,
        0
    );
    assert_eq!(
        ports
            .handoff
            .accept_control_start(&seed.request())
            .await
            .unwrap(),
        ControlStartAcceptance::ClaimSuperseded
    );
}

async fn oracle(ports: Ports) {
    control_turn_oracle::run(&ports).await;
    recovery_oracle::run(&ports).await;
    let happy = seed(&ports).await;
    accepted(&ports, &happy).await;
    let invalid_number = seed(&ports).await;
    let original = ports
        .execution
        .get(&invalid_number.scope, &invalid_number.execution)
        .await
        .unwrap()
        .unwrap();
    for invalid_claim in [false, true] {
        let mut request = invalid_number.request();
        if invalid_claim {
            let claim = ControlClaimToken::new(
                *request.claim().row_id(),
                nebula_storage_port::store::ClaimGeneration::new(u64::MAX),
                request.claim().scope().clone(),
            );
            request = request.with_claim(claim);
        } else {
            request = request.with_expected_execution_version(u64::MAX);
        }
        assert!(matches!(
            ports.handoff.accept_control_start(&request).await,
            Err(StorageError::Internal(_))
        ));
        assert_eq!(
            ports
                .execution
                .get(&invalid_number.scope, &invalid_number.execution)
                .await
                .unwrap()
                .unwrap(),
            original
        );
    }
    accepted(&ports, &invalid_number).await;
    for mismatch in 0..4 {
        let seed = seed(&ports).await;
        let original = ports
            .execution
            .get(&seed.scope, &seed.execution)
            .await
            .unwrap()
            .unwrap();
        let mut request = seed.request();
        let foreign = Scope::new(WorkspaceId::new().to_string(), OrgId::new().to_string());
        let missing = ExecutionId::new().to_string();
        match mismatch {
            0 => request = request.with_scope(&foreign),
            1 => request = request.with_execution_id(&missing),
            2 => {
                request = request
                    .with_worker_flavor_revision_id(WorkerFlavorRevisionId::from_bytes([0xFF; 32]));
            },
            3 => request = request.with_expected_execution_version(1),
            _ => unreachable!(),
        }
        let decision = ports.handoff.accept_control_start(&request).await.unwrap();
        assert_eq!(
            decision,
            if mismatch == 3 {
                ControlStartAcceptance::VersionConflict { actual: 0 }
            } else {
                ControlStartAcceptance::ClaimSuperseded
            }
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
        accepted(&ports, &seed).await;
    }
    let held = seed(&ports).await;
    let fence = ports
        .execution
        .acquire_lease(
            &held.scope,
            &held.execution,
            "other-owner",
            Duration::from_secs(30),
        )
        .await
        .unwrap()
        .unwrap();
    let before = ports
        .execution
        .get(&held.scope, &held.execution)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        ports
            .handoff
            .accept_control_start(&held.request())
            .await
            .unwrap(),
        ControlStartAcceptance::TurnHeldByAnotherOwner
    );
    assert_eq!(
        ports
            .execution
            .get(&held.scope, &held.execution)
            .await
            .unwrap()
            .unwrap(),
        before
    );
    assert!(
        ports
            .execution
            .release_lease(&held.scope, &held.execution, fence)
            .await
            .unwrap()
    );
    accepted(&ports, &held).await;

    let mut stale = seed(&ports).await;
    // SQL reclaim compares millisecond timestamps strictly, so leave the claim tick.
    tokio::time::sleep(Duration::from_millis(3)).await;
    assert_eq!(
        ports
            .queue
            .reclaim_stuck(Duration::ZERO, 8)
            .await
            .unwrap()
            .reclaimed,
        1
    );
    let fresh = ports
        .queue
        .claim_pending_for_flavor(&[0x64; 16], 1, stale.flavor)
        .await
        .unwrap();
    assert_eq!(fresh.len(), 1);
    let before = ports
        .execution
        .get(&stale.scope, &stale.execution)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        ports
            .handoff
            .accept_control_start(&stale.request())
            .await
            .unwrap(),
        ControlStartAcceptance::ClaimSuperseded
    );
    assert_eq!(
        ports
            .execution
            .get(&stale.scope, &stale.execution)
            .await
            .unwrap()
            .unwrap(),
        before
    );
    stale.claim = fresh[0].token.clone();
    accepted(&ports, &stale).await;

    let racing = seed(&ports).await;
    let request = racing.request();
    let (left, right) = tokio::join!(
        ports.handoff.accept_control_start(&request),
        ports.handoff.accept_control_start(&request)
    );
    let outcomes = [left.unwrap(), right.unwrap()];
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(outcome, ControlStartAcceptance::Accepted { .. }))
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| **outcome == ControlStartAcceptance::ClaimSuperseded)
            .count(),
        1
    );

    let drained = seed(&ports).await;
    ports
        .admin
        .begin_drain(nebula_storage_port::PlanFlavorRevisionTarget::WorkerFlavor(
            drained.flavor,
        ))
        .await
        .unwrap();
    accepted(&ports, &drained).await;

    let mut wrong_command = seed(&ports).await;
    ports
        .queue
        .mark_completed(&wrong_command.claim)
        .await
        .unwrap();
    let command = ControlMsg {
        id: ExecutionId::new().as_bytes(),
        execution_id: wrong_command.execution.clone(),
        command: ControlCommand::Cancel,
        scope: wrong_command.scope.clone(),
        w3c_traceparent: None,
        reclaim_count: 0,
        resume_target: None,
    };
    ports.queue.enqueue(&command).await.unwrap();
    wrong_command.claim = ports
        .queue
        .claim_pending_for_flavor(&[0x65; 16], 1, wrong_command.flavor)
        .await
        .unwrap()[0]
        .token
        .clone();
    let before = ports
        .execution
        .get(&wrong_command.scope, &wrong_command.execution)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        ports
            .handoff
            .accept_control_start(&wrong_command.request())
            .await
            .unwrap(),
        ControlStartAcceptance::ClaimSuperseded
    );
    assert_eq!(
        ports
            .execution
            .get(&wrong_command.scope, &wrong_command.execution)
            .await
            .unwrap()
            .unwrap(),
        before
    );
    ports
        .queue
        .mark_completed(&wrong_command.claim)
        .await
        .unwrap();

    let execution = ExecutionId::new().to_string();
    let scope = Scope::new(WorkspaceId::new().to_string(), OrgId::new().to_string());
    ports
        .execution
        .create(
            &scope,
            &execution,
            &WorkflowId::new().to_string(),
            serde_json::json!({}),
        )
        .await
        .unwrap();
    let command = ControlMsg {
        id: ExecutionId::new().as_bytes(),
        execution_id: execution.clone(),
        command: ControlCommand::Start,
        scope: scope.clone(),
        w3c_traceparent: None,
        reclaim_count: 0,
        resume_target: None,
    };
    ports.queue.enqueue(&command).await.unwrap();
    let claim = ports.queue.claim_pending(&[0x66; 16], 1).await.unwrap()[0]
        .token
        .clone();
    let missing_reference = Seed {
        scope,
        execution,
        flavor: WorkerFlavorRevisionId::from_bytes([0x67; 32]),
        claim: claim.clone(),
    };
    let before = ports
        .execution
        .get(&missing_reference.scope, &missing_reference.execution)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        ports
            .handoff
            .accept_control_start(&missing_reference.request())
            .await
            .unwrap(),
        ControlStartAcceptance::ClaimSuperseded
    );
    assert_eq!(
        ports
            .execution
            .get(&missing_reference.scope, &missing_reference.execution)
            .await
            .unwrap()
            .unwrap(),
        before
    );
    ports.queue.mark_completed(&claim).await.unwrap();

    let reclaim = seed(&ports).await;
    tokio::time::sleep(Duration::from_millis(3)).await;
    let request = reclaim.request();
    let (handoff, sweep) = tokio::join!(
        ports.handoff.accept_control_start(&request),
        ports.queue.reclaim_stuck(Duration::ZERO, 8)
    );
    match handoff.unwrap() {
        ControlStartAcceptance::Accepted { .. } => assert_eq!(sweep.unwrap().reclaimed, 0),
        ControlStartAcceptance::ClaimSuperseded => {
            assert_eq!(sweep.unwrap().reclaimed, 1);
            let mut seed = reclaim;
            seed.claim = ports
                .queue
                .claim_pending_for_flavor(&[0x68; 16], 1, seed.flavor)
                .await
                .unwrap()[0]
                .token
                .clone();
            accepted(&ports, &seed).await;
        },
        outcome => panic!("handoff/reclaim must have exactly one winner: {outcome:?}"),
    }
}

#[tokio::test]
async fn in_memory_control_start_handoff() {
    use nebula_storage::inmem::*;
    let execution = Arc::new(InMemoryExecutionStore::new());
    let catalog = Arc::new(execution.plan_flavor_catalog());
    oracle(Ports {
        jobs: Arc::new(InMemoryJobDispatchQueue::new(&execution)),
        queue: Arc::new(InMemoryControlQueue::new(&execution)),
        handoff: Arc::new(InMemoryTurnHandoff::new(&execution)),
        recovery: Arc::new(InMemoryTurnHandoff::new(&execution)),
        starts: Arc::new(InMemoryStartAcceptanceStore::new(&execution)),
        execution,
        catalog: catalog.clone(),
        admin: catalog,
    })
    .await;
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn sqlite_control_start_handoff() {
    use nebula_storage::sqlite::*;
    let directory = tempfile::tempdir().unwrap();
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(directory.path().join("handoff.db"))
        .create_if_missing(true)
        .busy_timeout(Duration::from_secs(10));
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(4)
        .connect_with(options)
        .await
        .unwrap();
    init_schema(&pool).await.unwrap();
    let catalog = Arc::new(SqlitePlanFlavorCatalog::new(
        pool.clone(),
        &nebula_metrics::MetricsRegistry::new(),
    ));
    oracle(Ports {
        jobs: Arc::new(SqliteJobDispatchQueue::new(pool.clone())),
        execution: Arc::new(SqliteExecutionStore::new(pool.clone())),
        queue: Arc::new(SqliteControlQueue::new(pool.clone())),
        handoff: Arc::new(SqliteTurnHandoff::new(pool.clone())),
        recovery: Arc::new(SqliteTurnHandoff::new(pool.clone())),
        starts: Arc::new(SqliteStartAcceptanceStore::new(pool)),
        catalog: catalog.clone(),
        admin: catalog,
    })
    .await;
}

#[cfg(feature = "postgres")]
#[path = "support/postgres_schema.rs"]
mod postgres_schema;

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_control_start_handoff() {
    use nebula_storage::postgres::*;
    let Ok(url) = std::env::var("DATABASE_URL") else {
        assert!(
            std::env::var_os("NEBULA_REQUIRE_POSTGRES").is_none(),
            "required PostgreSQL evidence needs DATABASE_URL"
        );
        return;
    };
    let pool = postgres_schema::connect_with_private_schema(&url, "control_start_handoff")
        .await
        .unwrap();
    init_schema(&pool).await.unwrap();
    let catalog = Arc::new(PgPlanFlavorCatalog::new(
        pool.clone(),
        &nebula_metrics::MetricsRegistry::new(),
    ));
    oracle(Ports {
        jobs: Arc::new(PgJobDispatchQueue::new(pool.clone())),
        execution: Arc::new(PgExecutionStore::new(pool.clone())),
        queue: Arc::new(PgControlQueue::new(pool.clone())),
        handoff: Arc::new(PgTurnHandoff::new(pool.clone())),
        recovery: Arc::new(PgTurnHandoff::new(pool.clone())),
        starts: Arc::new(PgStartAcceptanceStore::new(pool)),
        catalog: catalog.clone(),
        admin: catalog,
    })
    .await;
}
