//! Dispatch baseline references: what the relational schema proves
//! about queue rows, triggers and webhook activations.
//!
//! A queue row belongs to its execution, a trigger to its workflow and an
//! activation to its trigger and workflow; each is purged with its owner
//! through `ON DELETE CASCADE`. Writes beneath a soft-deletable parent check
//! it is live in the same transaction. These are references between
//! aggregates, so they run on SQL only (`relational_matrix!`).

use nebula_core::{PluginKey, WorkerFlavorRevisionId};
use nebula_storage_port::dto::{ControlCommand, ControlMsg, TriggerRow, WebhookActivationRecord};
use nebula_storage_port::store::TriggerStore;
use nebula_storage_port::{Scope, StorageError};

use super::{Backend, make_job, scope_a, scope_b, seed_execution, seed_scope_and_workflow};

fn control_msg(id: u8, scope: &Scope, execution_id: &str) -> ControlMsg {
    ControlMsg {
        id: [id; 16],
        execution_id: execution_id.into(),
        command: ControlCommand::Cancel,
        scope: scope.clone(),
        w3c_traceparent: None,
        reclaim_count: 0,
        resume_target: None,
    }
}

fn trigger_row(id: &str, workflow_id: &str) -> TriggerRow {
    TriggerRow {
        id: id.into(),
        workspace_id: scope_a().workspace_id,
        workflow_id: workflow_id.into(),
        slug: id.into(),
        display_name: "Fixture trigger".into(),
        kind: "webhook".into(),
        config: serde_json::json!({}),
        state: "active".into(),
        run_as: None,
        webhook_path: None,
        created_at: "2026-01-01T00:00:00.123456Z".into(),
        created_by: "fixture".into(),
        version: 1,
        deleted_at: None,
    }
}

async fn triggers(backend: &dyn Backend) -> std::sync::Arc<dyn TriggerStore> {
    backend
        .trigger_store()
        .await
        .unwrap_or_else(|| panic!("[{}] a relational backend has triggers", backend.name()))
}

fn is_not_found(result: &Result<(), StorageError>, entity: &str) -> bool {
    matches!(result, Err(StorageError::NotFound { entity: found, .. }) if *found == entity)
}

/// Enqueueing on either queue for an execution its tenant does not hold is
/// `NotFound { entity: "execution" }` and leaves nothing claimable.
pub(crate) async fn assert_queue_rows_require_their_execution(backend: &dyn Backend) {
    let control = backend.control_queue().await;
    let jobs = backend.job_dispatch_queue().await;

    let missing = control
        .enqueue(&control_msg(0xE1, &scope_a(), "exe_missing"))
        .await;
    assert!(
        is_not_found(&missing, "execution"),
        "[{}] a control row for a missing execution must be NotFound, got {missing:?}",
        backend.name()
    );
    let job = make_job(0xE2, "plugin.orphan", &["plugin.orphan"]);
    let missing_job = jobs.enqueue(&job).await;
    assert!(
        is_not_found(&missing_job, "execution"),
        "[{}] a job for a missing execution must be NotFound, got {missing_job:?}",
        backend.name()
    );

    // The execution exists, but in another tenant: the row names its tenant.
    seed_execution(backend, &scope_a(), "exe_tenant_bound").await;
    let foreign = control
        .enqueue(&control_msg(0xE3, &scope_b(), "exe_tenant_bound"))
        .await;
    assert!(
        is_not_found(&foreign, "execution"),
        "[{}] a control row naming another tenant's execution must be NotFound, got {foreign:?}",
        backend.name()
    );

    assert!(
        control
            .claim_pending(&[1; 16], 16)
            .await
            .expect("claim control rows")
            .is_empty(),
        "[{}] a rejected enqueue must leave no control row",
        backend.name()
    );
    assert!(
        jobs.claim_pending(
            &[1; 16],
            16,
            &["plugin.orphan".parse::<PluginKey>().expect("plugin key")],
            WorkerFlavorRevisionId::from_bytes([0x11; 32]),
        )
        .await
        .expect("claim jobs")
        .is_empty(),
        "[{}] a rejected enqueue must leave no job",
        backend.name()
    );
}

/// Purging an execution purges its control rows and jobs; purging a
/// workflow purges its executions and, through them, their rows.
pub(crate) async fn assert_queue_rows_cascade_with_their_execution(backend: &dyn Backend) {
    let control = backend.control_queue().await;
    let jobs = backend.job_dispatch_queue().await;
    let executions = backend.execution_store().await;
    let s = scope_a();
    let plugins = ["plugin.purged".parse::<PluginKey>().expect("plugin key")];
    let flavor = WorkerFlavorRevisionId::from_bytes([0x11; 32]);

    seed_execution(backend, &s, "exe_purged").await;
    control
        .enqueue(&control_msg(0xE4, &s, "exe_purged"))
        .await
        .expect("enqueue the control row");
    let mut job = make_job(0xE5, "plugin.purged", &["plugin.purged"]);
    job.execution_id = "exe_purged".into();
    jobs.enqueue(&job).await.expect("enqueue the job");

    assert!(
        backend.purge("executions", &s, "exe_purged").await,
        "[{}] the execution must purge",
        backend.name()
    );
    assert!(
        control
            .claim_pending(&[2; 16], 16)
            .await
            .expect("claim control rows")
            .is_empty(),
        "[{}] an execution's control rows are purged with it",
        backend.name()
    );
    assert!(
        jobs.claim_pending(&[2; 16], 16, &plugins, flavor)
            .await
            .expect("claim jobs")
            .is_empty(),
        "[{}] an execution's jobs are purged with it",
        backend.name()
    );

    // The workflow owns its executions, and they own their rows.
    seed_execution(backend, &s, "exe_workflow_purged").await;
    control
        .enqueue(&control_msg(0xE6, &s, "exe_workflow_purged"))
        .await
        .expect("enqueue the control row");
    assert!(
        backend.purge("workflows", &s, "wf_queued").await,
        "[{}] the workflow must purge",
        backend.name()
    );
    assert!(
        executions
            .get(&s, "exe_workflow_purged")
            .await
            .expect("read the execution")
            .is_none(),
        "[{}] a workflow's executions are purged with it",
        backend.name()
    );
    assert!(
        control
            .claim_pending(&[3; 16], 16)
            .await
            .expect("claim control rows")
            .is_empty(),
        "[{}] a purged workflow's control rows are purged with it",
        backend.name()
    );
}

/// Whether the `hook-live` activation is found by slug, by token and in the
/// bootstrap listing — all three routing reads agree.
async fn assert_routes(
    backend: &dyn Backend,
    webhooks: &dyn nebula_storage_port::store::WebhookActivationStore,
    s: &Scope,
    expected: bool,
) {
    let by_slug = webhooks
        .resolve(s, "hook-live")
        .await
        .expect("resolve by slug")
        .is_some();
    let by_token = webhooks
        .resolve_by_token(&[0x5a; 32])
        .await
        .expect("resolve by token")
        .is_some();
    let listed = webhooks
        .list_all_active()
        .await
        .expect("list active")
        .iter()
        .any(|record| record.slug == "hook-live");
    assert_eq!(
        (by_slug, by_token, listed),
        (expected, expected, expected),
        "[{}] an activation routes exactly while its trigger is live",
        backend.name()
    );
}

/// A trigger needs its live workflow, and an activation its live trigger and
/// workflow: a missing or soft-deleted parent is `NotFound`. Trigger instants
/// round-trip at microsecond precision.
pub(crate) async fn assert_dispatch_writes_require_live_parents(backend: &dyn Backend) {
    let triggers = triggers(backend).await;
    let webhooks = backend.webhook_store().await;
    let workflows = backend.workflow_store().await;
    let s = scope_a();

    let orphan = triggers
        .create(&s, trigger_row("trg_orphan", "wf_missing"))
        .await;
    assert!(
        is_not_found(&orphan, "workflow"),
        "[{}] a trigger of a missing workflow must be NotFound, got {orphan:?}",
        backend.name()
    );

    seed_scope_and_workflow(backend, &s, "wf_live").await;
    let live = trigger_row("trg_live", "wf_live");
    triggers
        .create(&s, live.clone())
        .await
        .expect("a trigger of a live workflow");
    assert_eq!(
        triggers
            .get(&s, "trg_live")
            .await
            .expect("read the trigger"),
        Some(live),
        "[{}] a trigger round-trips, instants at microsecond precision",
        backend.name()
    );

    let mut activation = WebhookActivationRecord::new("node_live", s.clone(), "hook-live", true);
    activation.spec_trigger_id = Some("trg_missing".into());
    let no_trigger = webhooks.upsert(&s, activation.clone()).await;
    assert!(
        is_not_found(&no_trigger, "trigger"),
        "[{}] an activation of a missing trigger must be NotFound, got {no_trigger:?}",
        backend.name()
    );
    activation.spec_trigger_id = Some("trg_live".into());
    activation.workflow_id = Some("wf_missing".into());
    let no_workflow = webhooks.upsert(&s, activation.clone()).await;
    assert!(
        is_not_found(&no_workflow, "workflow"),
        "[{}] an activation into a missing workflow must be NotFound, got {no_workflow:?}",
        backend.name()
    );
    activation.workflow_id = Some("wf_live".into());
    activation.token_hash = [0x5a; 32];
    webhooks
        .upsert(&s, activation.clone())
        .await
        .expect("an activation of a live trigger and workflow");
    assert_routes(backend, webhooks.as_ref(), &s, true).await;

    triggers
        .soft_delete(&s, "trg_live")
        .await
        .expect("delete the trigger");
    assert_routes(backend, webhooks.as_ref(), &s, false).await;
    let deleted_trigger = webhooks.upsert(&s, activation).await;
    assert!(
        is_not_found(&deleted_trigger, "trigger"),
        "[{}] an activation of a deleted trigger must be NotFound, got {deleted_trigger:?}",
        backend.name()
    );

    workflows
        .soft_delete(&s, "wf_live")
        .await
        .expect("archive the workflow");
    let archived = triggers
        .create(&s, trigger_row("trg_archived", "wf_live"))
        .await;
    assert!(
        is_not_found(&archived, "workflow"),
        "[{}] a trigger of an archived workflow must be NotFound, got {archived:?}",
        backend.name()
    );
}

/// Purging a workflow purges its triggers and their activations.
pub(crate) async fn assert_triggers_cascade_with_their_workflow(backend: &dyn Backend) {
    let triggers = triggers(backend).await;
    let webhooks = backend.webhook_store().await;
    let s = scope_a();
    let token = [0x5A; 32];

    seed_scope_and_workflow(backend, &s, "wf_trigger_purged").await;
    triggers
        .create(&s, trigger_row("trg_purged", "wf_trigger_purged"))
        .await
        .expect("create the trigger");
    let mut activation =
        WebhookActivationRecord::new("node_purged", s.clone(), "hook-purged", true);
    activation.spec_trigger_id = Some("trg_purged".into());
    activation.workflow_id = Some("wf_trigger_purged".into());
    activation.token_hash = token;
    webhooks
        .upsert(&s, activation)
        .await
        .expect("activate the trigger");

    assert!(
        backend.purge("workflows", &s, "wf_trigger_purged").await,
        "[{}] the workflow must purge",
        backend.name()
    );
    assert!(
        triggers
            .get(&s, "trg_purged")
            .await
            .expect("read the trigger")
            .is_none(),
        "[{}] a workflow's triggers are purged with it",
        backend.name()
    );
    assert!(
        webhooks
            .resolve(&s, "hook-purged")
            .await
            .expect("resolve the slug")
            .is_none()
            && webhooks
                .resolve_by_token(&token)
                .await
                .expect("resolve the token")
                .is_none(),
        "[{}] a trigger's activations are purged with it",
        backend.name()
    );
}
