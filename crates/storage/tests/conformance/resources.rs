//! Resource baseline references: what the relational schema proves
//! about stored resources, their runtime status and shared resources.
//!
//! A stored resource and a shared resource belong to their workspace; a
//! status snapshot belongs to its stored resource; subscriptions, source
//! leases, events, deliveries and execution handoffs belong to their shared
//! resource. Each is purged with its owner through `ON DELETE CASCADE`.
//! Writes beneath a soft-deletable parent check it is live in the same
//! transaction, and an archived resource has no live status. These are
//! references between aggregates, so they run on SQL only
//! (`relational_matrix!`).

use std::sync::Arc;
use std::time::Duration;

use nebula_storage_port::dto::{
    AcceptResourceEventOutcome, AcceptResourceEventRequest, AcquireResourceSourceLeaseOutcome,
    AcquireResourceSourceLeaseRequest, ClaimResourceDeliveriesRequest,
    ClaimResourceRuntimeWorkRequest, CompleteResourceDeliveryRequest, EventEnvelope,
    EventOccurrenceKey, EventOccurrenceNamespace, PutResourceSubscriptionRequest,
    ResolveSharedResourceOutcome, ResolveSharedResourceRequest, ResourceCompatibilityVersion,
    ResourceConfigurationIdentity, ResourceConsumerIdentity, ResourceConsumerKind,
    ResourceDeliveryCompletion, ResourceKind, ResourceLeaseHolder, ResourceLeaseTtl,
    ResourcePageSize, ResourceRow, ResourceSlotIdentity, ResourceStatusPhase,
    ResourceStatusSnapshot, SharedResourceId, SharedResourceIdentity, StatusWorkerId,
};
use nebula_storage_port::store::{
    ResourceEventFanoutStore, ResourceExecutionHandoffStore, ResourceRuntimeRecovery,
    ResourceSourceLeaseStore, ResourceStatusStore, ResourceStore, ResourceSubscriptionStore,
    SharedResourceStore,
};
use nebula_storage_port::{Scope, StorageError};

use super::{Backend, scope_a, scope_b};

/// Every role of the shared-resource runtime, behind one handle.
pub(crate) trait SharedResourceRuntime:
    SharedResourceStore
    + ResourceSubscriptionStore
    + ResourceSourceLeaseStore
    + ResourceEventFanoutStore
    + ResourceExecutionHandoffStore
    + ResourceRuntimeRecovery
    + Send
    + Sync
{
}

impl<T> SharedResourceRuntime for T where
    T: SharedResourceStore
        + ResourceSubscriptionStore
        + ResourceSourceLeaseStore
        + ResourceEventFanoutStore
        + ResourceExecutionHandoffStore
        + ResourceRuntimeRecovery
        + Send
        + Sync
{
}

fn resource_row(scope: &Scope, id: &str) -> ResourceRow {
    ResourceRow {
        id: id.into(),
        workspace_id: scope.workspace_id.clone(),
        slug: id.into(),
        display_name: "Fixture resource".into(),
        kind: "fixture".into(),
        config: serde_json::json!({}),
        credential_bindings: std::collections::BTreeMap::new(),
        topology: None,
        resilience_override: None,
        created_at: "2026-01-01T00:00:00.123456Z".into(),
        created_by: "fixture".into(),
        version: 1,
        deleted_at: None,
    }
}

fn snapshot(resource_id: &str) -> ResourceStatusSnapshot {
    ResourceStatusSnapshot {
        resource_id: resource_id.into(),
        phase: ResourceStatusPhase::Ready,
        healthy: true,
        accepting: true,
        row_version: 1,
    }
}

fn identity() -> SharedResourceIdentity {
    SharedResourceIdentity::new(
        ResourceKind::new("fixture.resource").expect("valid kind"),
        ResourceCompatibilityVersion::new(1),
        ResourceConfigurationIdentity::try_from_vec(b"configuration".to_vec())
            .expect("valid configuration"),
        ResourceSlotIdentity::try_from_vec(b"slot".to_vec()).expect("valid slot"),
    )
}

fn holder(name: &str) -> ResourceLeaseHolder {
    ResourceLeaseHolder::new(name).expect("valid holder")
}

fn ttl() -> ResourceLeaseTtl {
    ResourceLeaseTtl::new(Duration::from_secs(30)).expect("valid TTL")
}

fn batch() -> ResourcePageSize {
    ResourcePageSize::new(16).expect("valid batch")
}

fn is_not_found<T: std::fmt::Debug>(result: &Result<T, StorageError>, entity: &str) -> bool {
    matches!(result, Err(StorageError::NotFound { entity: found, .. }) if *found == entity)
}

async fn resources(backend: &dyn Backend) -> Arc<dyn ResourceStore> {
    backend
        .resource_store()
        .await
        .unwrap_or_else(|| panic!("[{}] a relational backend has resources", backend.name()))
}

async fn statuses(backend: &dyn Backend) -> Arc<dyn ResourceStatusStore> {
    backend.resource_status_store().await.unwrap_or_else(|| {
        panic!(
            "[{}] a relational backend has resource status",
            backend.name()
        )
    })
}

async fn runtime(backend: &dyn Backend) -> Arc<dyn SharedResourceRuntime> {
    backend.resource_runtime().await.unwrap_or_else(|| {
        panic!(
            "[{}] a relational backend has a shared-resource runtime",
            backend.name()
        )
    })
}

async fn resolve(
    runtime: &dyn SharedResourceRuntime,
    scope: &Scope,
) -> Result<SharedResourceId, StorageError> {
    runtime
        .resolve(ResolveSharedResourceRequest::new(scope.clone(), identity()))
        .await
        .map(|outcome| match outcome {
            ResolveSharedResourceOutcome::Created(record)
            | ResolveSharedResourceOutcome::Existing(record) => record.id(),
        })
}

/// A stored resource and a shared resource need their live workspace: a
/// missing or archived one is `NotFound { entity: "workspace" }`, and a stored
/// resource round-trips at microsecond precision.
pub(crate) async fn assert_resources_require_a_live_workspace(backend: &dyn Backend) {
    let resources = resources(backend).await;
    let runtime = runtime(backend).await;
    let unprovisioned = Scope::new("ws_unprovisioned", "org_unprovisioned");

    let orphan = resources
        .create(&unprovisioned, resource_row(&unprovisioned, "res_orphan"))
        .await;
    assert!(
        is_not_found(&orphan, "workspace"),
        "[{}] a resource in a missing workspace must be NotFound, got {orphan:?}",
        backend.name()
    );
    let orphan_shared = resolve(runtime.as_ref(), &unprovisioned).await;
    assert!(
        is_not_found(&orphan_shared, "workspace"),
        "[{}] a shared resource in a missing workspace must be NotFound, got {orphan_shared:?}",
        backend.name()
    );

    let s = scope_a();
    let live = resource_row(&s, "res_live");
    resources
        .create(&s, live.clone())
        .await
        .expect("a resource in a live workspace");
    assert_eq!(
        resources
            .get(&s, "res_live")
            .await
            .expect("read the resource"),
        Some(live),
        "[{}] a resource round-trips, instants at microsecond precision",
        backend.name()
    );
    resolve(runtime.as_ref(), &s)
        .await
        .expect("a shared resource in a live workspace");

    let archived = scope_b();
    assert!(
        backend.retire_workspace(&archived, false).await,
        "[{}] the workspace must archive",
        backend.name()
    );
    let in_archived = resources
        .create(&archived, resource_row(&archived, "res_archived"))
        .await;
    assert!(
        is_not_found(&in_archived, "workspace"),
        "[{}] a resource in an archived workspace must be NotFound, got {in_archived:?}",
        backend.name()
    );
    let shared_in_archived = resolve(runtime.as_ref(), &archived).await;
    assert!(
        is_not_found(&shared_in_archived, "workspace"),
        "[{}] a shared resource in an archived workspace must be NotFound, got \
         {shared_in_archived:?}",
        backend.name()
    );
}

/// A status snapshot needs its live resource: publishing for a missing or
/// archived resource is `NotFound { entity: "resource" }`, and archiving a
/// resource hides the status already published for it.
pub(crate) async fn assert_resource_status_needs_a_live_resource(backend: &dyn Backend) {
    let resources = resources(backend).await;
    let statuses = statuses(backend).await;
    let s = scope_a();
    let worker = StatusWorkerId::new("worker-relational").expect("valid worker id");
    statuses
        .heartbeat(&worker, Duration::from_mins(1))
        .await
        .expect("heartbeat");

    let missing = statuses
        .publish(&s, &worker, &snapshot("res_missing"))
        .await;
    assert!(
        is_not_found(&missing, "resource"),
        "[{}] status of a missing resource must be NotFound, got {missing:?}",
        backend.name()
    );

    resources
        .create(&s, resource_row(&s, "res_status"))
        .await
        .expect("create the resource");
    statuses
        .publish(&s, &worker, &snapshot("res_status"))
        .await
        .expect("status of a live resource");
    assert_eq!(
        statuses
            .live_for(&s, "res_status")
            .await
            .expect("read live status")
            .len(),
        1,
        "[{}] a live resource's status is visible",
        backend.name()
    );

    resources
        .soft_delete(&s, "res_status")
        .await
        .expect("archive the resource");
    assert!(
        statuses
            .live_for(&s, "res_status")
            .await
            .expect("read live status")
            .is_empty(),
        "[{}] an archived resource has no live status",
        backend.name()
    );
    let archived = statuses.publish(&s, &worker, &snapshot("res_status")).await;
    assert!(
        is_not_found(&archived, "resource"),
        "[{}] status of an archived resource must be NotFound, got {archived:?}",
        backend.name()
    );
}

/// Purging a stored resource purges its status; purging a workspace purges
/// its stored resources and its shared resources with every subscription,
/// source lease, event, delivery and execution handoff beneath them.
pub(crate) async fn assert_resource_rows_cascade_with_their_owner(backend: &dyn Backend) {
    let resources = resources(backend).await;
    let statuses = statuses(backend).await;
    let runtime = runtime(backend).await;
    let s = scope_a();
    let worker = StatusWorkerId::new("worker-relational").expect("valid worker id");
    statuses
        .heartbeat(&worker, Duration::from_mins(1))
        .await
        .expect("heartbeat");

    resources
        .create(&s, resource_row(&s, "res_purged"))
        .await
        .expect("create the resource");
    statuses
        .publish(&s, &worker, &snapshot("res_purged"))
        .await
        .expect("publish its status");
    assert!(
        backend.purge("resources", &s, "res_purged").await,
        "[{}] the resource must purge",
        backend.name()
    );
    // The same id again: a snapshot that outlived its resource would show.
    resources
        .create(&s, resource_row(&s, "res_purged"))
        .await
        .expect("re-create the resource");
    assert!(
        statuses
            .live_for(&s, "res_purged")
            .await
            .expect("read live status")
            .is_empty(),
        "[{}] a resource's status is purged with it",
        backend.name()
    );

    // A shared resource with one handed-off delivery and one pending one.
    let resource_id = resolve(runtime.as_ref(), &s)
        .await
        .expect("resolve the shared resource");
    runtime
        .put(PutResourceSubscriptionRequest::new(
            s.clone(),
            resource_id,
            ResourceConsumerKind::new("workflow-trigger").expect("valid consumer kind"),
            ResourceConsumerIdentity::try_from_vec(b"consumer".to_vec())
                .expect("valid consumer identity"),
        ))
        .await
        .expect("subscribe");
    let source = match runtime
        .acquire(AcquireResourceSourceLeaseRequest::new(
            s.clone(),
            resource_id,
            holder("source"),
            ttl(),
        ))
        .await
        .expect("acquire the source")
    {
        AcquireResourceSourceLeaseOutcome::Acquired(lease) => lease,
        AcquireResourceSourceLeaseOutcome::Contended { .. } => {
            panic!("a fresh source lease must be acquired")
        },
    };
    let mut events = Vec::new();
    for occurrence in [b"handed-off".as_slice(), b"pending".as_slice()] {
        let outcome = runtime
            .accept(AcceptResourceEventRequest::new(
                s.clone(),
                resource_id,
                source.token().clone(),
                EventOccurrenceNamespace::new("fixture.event").expect("valid namespace"),
                EventOccurrenceKey::try_from_vec(occurrence.to_vec()).expect("valid key"),
                EventEnvelope::try_from_vec(1, b"payload".to_vec()).expect("valid envelope"),
            ))
            .await
            .expect("accept the event");
        let AcceptResourceEventOutcome::Accepted { event_id, .. } = outcome else {
            panic!("a fresh occurrence must be accepted, got {outcome:?}");
        };
        events.push(event_id);
        if events.len() == 1 {
            let delivery = runtime
                .claim_deliveries(ClaimResourceDeliveriesRequest::new(
                    s.clone(),
                    holder("fanout"),
                    ttl(),
                    batch(),
                ))
                .await
                .expect("claim the delivery")
                .pop()
                .expect("one delivery");
            runtime
                .complete_delivery(CompleteResourceDeliveryRequest::new(
                    s.clone(),
                    delivery.id(),
                    delivery.token().clone(),
                    ResourceDeliveryCompletion::Delivered,
                ))
                .await
                .expect("deliver it, handing it off");
        }
    }

    assert!(
        backend.retire_workspace(&s, true).await,
        "[{}] the workspace must purge",
        backend.name()
    );
    assert!(
        resources
            .get(&s, "res_purged")
            .await
            .expect("read the resource")
            .is_none(),
        "[{}] a workspace's resources are purged with it",
        backend.name()
    );
    assert!(
        SharedResourceStore::get(runtime.as_ref(), &s, resource_id)
            .await
            .expect("read the shared resource")
            .is_none(),
        "[{}] a workspace's shared resources are purged with it",
        backend.name()
    );
    for event_id in events {
        assert!(
            runtime
                .get_event(&s, event_id)
                .await
                .expect("read the event")
                .is_none(),
            "[{}] a shared resource's events are purged with it",
            backend.name()
        );
    }
    let recovery = ClaimResourceRuntimeWorkRequest::new(holder("recovery"), ttl(), batch());
    assert!(
        runtime
            .claim_deliveries_globally(recovery.clone())
            .await
            .expect("claim deliveries")
            .is_empty(),
        "[{}] a shared resource's deliveries are purged with it",
        backend.name()
    );
    assert!(
        runtime
            .claim_handoffs_globally(recovery)
            .await
            .expect("claim handoffs")
            .is_empty(),
        "[{}] a shared resource's execution handoffs are purged with it",
        backend.name()
    );
}
