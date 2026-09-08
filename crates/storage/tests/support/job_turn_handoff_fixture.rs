use nebula_core::{
    ExecutablePlanRevisionId, ExecutionContractBundleId, ExecutionId, OrgId, PluginKey,
    PluginSetId, WorkerFlavorRevisionId, WorkflowId, WorkflowVersionId, WorkspaceId,
};
use nebula_storage_port::{
    Scope,
    dto::{
        ContractBundleRecord, ControlCommand, ControlMsg, JobDispatchMsg, MaterializedStart,
        NewExecution, PlanFlavorRevisionIds, PlanFlavorRevisionRecord, RevisionRecordBytes,
        WorkerFlavorRevisionRecord,
    },
    store::{
        ExecutionStore, ExecutionTurnHandoff, JobDispatchQueue, PlanFlavorCatalogWriter,
        StartAcceptanceStore, StartContractIdentity, StartMaterialization, TurnAcceptance,
        TurnHandoff, TurnRecovery,
    },
};
use std::time::Duration;

pub(super) const LIVE_FLAVOR: WorkerFlavorRevisionId =
    WorkerFlavorRevisionId::from_bytes([0x11; 32]);
pub(super) const OTHER_FLAVOR: WorkerFlavorRevisionId =
    WorkerFlavorRevisionId::from_bytes([0x22; 32]);

pub(super) struct TurnHandoffPorts<'a> {
    pub(super) store: &'a dyn ExecutionStore,
    pub(super) queue: &'a dyn JobDispatchQueue,
    pub(super) handoff: &'a dyn ExecutionTurnHandoff,
    pub(super) recovery: &'a dyn TurnRecovery,
    pub(super) catalog: &'a dyn PlanFlavorCatalogWriter,
    pub(super) starts: &'a dyn StartAcceptanceStore,
}

pub(super) async fn materialize_execution(
    catalog: &dyn PlanFlavorCatalogWriter,
    starts: &dyn StartAcceptanceStore,
    execution_id: &str,
    scope: &Scope,
) {
    let execution = execution_id
        .parse::<ExecutionId>()
        .expect("handoff fixture execution id must be typed");
    let workflow = WorkflowId::new();
    let workflow_revision = WorkflowVersionId::new();
    let plan = ExecutablePlanRevisionId::from_bytes(std::array::from_fn(|index| {
        execution.as_bytes()[index % 16]
    }));
    let plugin_set = PluginSetId::from_bytes([0x33; 32]);
    let pair = PlanFlavorRevisionRecord::graph_v1_json(
        plan,
        RevisionRecordBytes::try_from_vec(
            serde_json::to_vec(&serde_json::json!({
                "claimed_id": plan,
                "workflow_version_id": workflow_revision,
                "worker_flavor_revision_id": LIVE_FLAVOR,
                "plugin_set_id": plugin_set,
                "manifest": { "workflow_id": workflow }
            }))
            .expect("revision fixture must serialize"),
        )
        .expect("revision fixture must be non-empty"),
        WorkerFlavorRevisionRecord::v1_json(
            LIVE_FLAVOR,
            RevisionRecordBytes::try_from_vec(b"{}".to_vec())
                .expect("flavor fixture must be non-empty"),
        ),
    );
    catalog
        .insert(&pair)
        .await
        .expect("exact plan and flavor must install");

    let identity = StartContractIdentity::new(
        ExecutionContractBundleId::new(),
        PlanFlavorRevisionIds::new(plan, LIVE_FLAVOR),
    );
    let workspace = scope
        .workspace_id
        .parse::<WorkspaceId>()
        .expect("handoff fixture workspace id must be typed");
    let org = scope
        .org_id
        .parse::<OrgId>()
        .expect("handoff fixture org id must be typed");
    let bundle = ContractBundleRecord::v1_json(
        identity,
        serde_json::to_vec(&serde_json::json!({
            "bundle_id": identity.bundle_id(),
            "org_id": org,
            "workspace_id": workspace,
            "executable_plan_revision_id": plan,
            "plugin_set_id": plugin_set,
            "revisions": {
                "workflow": workflow_revision,
                "worker_flavor": LIVE_FLAVOR
            }
        }))
        .expect("bundle fixture must serialize"),
    )
    .expect("bundle fixture must be bounded");
    let state = serde_json::json!({
        "execution_id": execution,
        "workflow_id": workflow,
        "workflow_version_number": 1,
        "executable_plan_revision_id": plan,
        "worker_flavor_revision_id": LIVE_FLAVOR,
        "status": "created",
        "version": 0,
        "node_states": {},
        "created_at": "2026-09-06T00:00:00Z",
        "updated_at": "2026-09-06T00:00:00Z",
        "started_at": null,
        "completed_at": null,
        "total_output_bytes": 0,
        "total_retries": 0,
        "terminated_by": null
    });
    let command = ControlMsg {
        id: execution.as_bytes(),
        execution_id: execution_id.to_owned(),
        command: ControlCommand::Start,
        scope: scope.clone(),
        w3c_traceparent: None,
        reclaim_count: 0,
        resume_target: None,
    };
    let workflow_id = workflow.to_string();
    assert!(matches!(
        starts
            .materialize_start(&MaterializedStart::new(
                scope,
                None,
                execution_id,
                NewExecution::new(&workflow_id, &state),
                &command,
                &bundle,
            ))
            .await
            .expect("start fixture must materialize"),
        StartMaterialization::Accepted { .. }
    ));
}

pub(super) async fn assert_mismatched_flavor_refuses_the_turn(
    ports: TurnHandoffPorts<'_>,
    execution_id: &str,
    scope: &Scope,
) {
    materialize_execution(ports.catalog, ports.starts, execution_id, scope).await;
    let plugin = execution_id
        .replace('-', "")
        .parse::<PluginKey>()
        .expect("typed execution id must form a valid fixture plugin key");
    let message = JobDispatchMsg::new(
        *uuid::Uuid::new_v4().as_bytes(),
        execution_id,
        ControlCommand::Start,
        scope.clone(),
        serde_json::json!({}),
        None::<String>,
        plugin.clone(),
        vec![plugin.clone()],
        None::<String>,
        0,
        OTHER_FLAVOR,
    );
    ports
        .queue
        .enqueue(&message)
        .await
        .expect("mismatched job enqueues");
    let claims = ports
        .queue
        .claim_pending(&[0x44; 16], 1, &[plugin], OTHER_FLAVOR)
        .await
        .expect("mismatched job is claimable by its queued flavor");
    assert_eq!(
        claims.len(),
        1,
        "the mismatch fixture owns exactly one claim"
    );
    let claim = claims[0].token;

    assert_eq!(
        ports
            .handoff
            .accept_turn(
                &TurnHandoff::for_claim(scope, execution_id, claim, OTHER_FLAVOR)
                    .lease_to("wrong-flavor-worker", Duration::from_secs(30)),
            )
            .await
            .expect("a revision mismatch is a typed refusal"),
        TurnAcceptance::ClaimSuperseded
    );

    let candidates = ports
        .recovery
        .list_recoverable_turns(LIVE_FLAVOR, None, 16)
        .await
        .expect("recovery marker projection remains readable");
    assert!(
        candidates.turns().is_empty(),
        "a refused handoff must not create an acceptance marker"
    );
    ports
        .queue
        .mark_dispatched(&claim)
        .await
        .expect("a refused handoff must leave the queue claim non-terminal");
    ports
        .store
        .acquire_lease(
            scope,
            execution_id,
            "right-flavor-worker",
            Duration::from_secs(30),
        )
        .await
        .expect("the execution remains readable")
        .expect("a refused handoff must leave the execution unleased");
}
