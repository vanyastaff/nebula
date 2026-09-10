//! Shared behavioral oracle for durable shared-resource fanout stores.

use nebula_storage_port::dto::{
    AcceptResourceEventOutcome, AcceptResourceEventRequest, AcknowledgeResourceHandoffOutcome,
    AcquireResourceSourceLeaseOutcome, AcquireResourceSourceLeaseRequest,
    ClaimResourceDeliveriesRequest, ClaimResourceHandoffsRequest, CompleteResourceDeliveryOutcome,
    CompleteResourceDeliveryRequest, EventEnvelope, EventOccurrenceKey, EventOccurrenceNamespace,
    HeartbeatResourceDeliveryRequest, HeartbeatResourceHandoffRequest,
    PutResourceSubscriptionOutcome, PutResourceSubscriptionRequest, ReconciliationCursor,
    ReleaseResourceDeliveryRequest, ReleaseResourceSourceLeaseRequest,
    ResolveSharedResourceOutcome, ResolveSharedResourceRequest, ResourceCompatibilityVersion,
    ResourceConfigurationIdentity, ResourceConsumerIdentity, ResourceConsumerKind,
    ResourceDeliveryCompletion, ResourceDeliveryId, ResourceEventState,
    ResourceHandoffClaimRequest, ResourceKind, ResourceLeaseHolder, ResourceLeaseTtl,
    ResourcePageSize, ResourceSlotIdentity, ResourceSubscriptionState, ResourceSubscriptionVersion,
    SharedResourceId, SharedResourceIdentity, TerminalDeliveryIneligibility,
    TransitionResourceSubscriptionRequest,
};
use nebula_storage_port::store::{
    ResourceEventFanoutStore, ResourceExecutionHandoffStore, ResourceSourceLeaseStore,
    ResourceSubscriptionStore, SharedResourceStore,
};
use nebula_storage_port::{Scope, StorageError};
use std::assert_matches;
use std::sync::Arc;
use std::time::Duration;

/// Backend-owned deterministic expiry control used by the conformance target.
#[async_trait::async_trait]
pub(crate) trait ResourceFanoutExpiryControl {
    async fn expire_source_for_test(&self, scope: &Scope, resource_id: SharedResourceId);
    async fn expire_delivery_for_test(&self, scope: &Scope, delivery_id: ResourceDeliveryId);
}

pub(crate) trait ResourceRuntimeUnderTest:
    SharedResourceStore
    + ResourceSubscriptionStore
    + ResourceSourceLeaseStore
    + ResourceEventFanoutStore
    + ResourceExecutionHandoffStore
    + Clone
    + Send
    + Sync
    + 'static
{
}

impl<T> ResourceRuntimeUnderTest for T where
    T: SharedResourceStore
        + ResourceSubscriptionStore
        + ResourceSourceLeaseStore
        + ResourceEventFanoutStore
        + ResourceExecutionHandoffStore
        + Clone
        + Send
        + Sync
        + 'static
{
}

fn scope() -> Scope {
    Scope::new("resource-ws", "resource-org")
}

fn other_scope() -> Scope {
    Scope::new("resource-ws-other", "resource-org-other")
}

fn identity(configuration: &[u8], slot: &[u8]) -> SharedResourceIdentity {
    SharedResourceIdentity::new(
        ResourceKind::new("telegram.bot").expect("valid kind"),
        ResourceCompatibilityVersion::new(1),
        ResourceConfigurationIdentity::try_from_vec(configuration.to_vec())
            .expect("valid configuration identity"),
        ResourceSlotIdentity::try_from_vec(slot.to_vec()).expect("valid slot identity"),
    )
}

fn consumer_identity(seed: u8) -> ResourceConsumerIdentity {
    ResourceConsumerIdentity::try_from_vec(vec![seed; 16]).expect("valid consumer identity")
}

fn holder(name: &str) -> ResourceLeaseHolder {
    ResourceLeaseHolder::new(name).expect("valid holder")
}

fn ttl(seconds: u64) -> ResourceLeaseTtl {
    ResourceLeaseTtl::new(Duration::from_secs(seconds)).expect("valid TTL")
}

async fn resolve(
    store: &impl SharedResourceStore,
    target_scope: Scope,
    resource_identity: SharedResourceIdentity,
) -> ResolveSharedResourceOutcome {
    store
        .resolve(ResolveSharedResourceRequest::new(
            target_scope,
            resource_identity,
        ))
        .await
        .expect("resource resolves")
}

fn resolved_id(outcome: &ResolveSharedResourceOutcome) -> SharedResourceId {
    match outcome {
        ResolveSharedResourceOutcome::Created(record)
        | ResolveSharedResourceOutcome::Existing(record) => record.id(),
    }
}

async fn put_subscription(
    store: &impl ResourceSubscriptionStore,
    resource_id: SharedResourceId,
    seed: u8,
) -> PutResourceSubscriptionOutcome {
    store
        .put(PutResourceSubscriptionRequest::new(
            scope(),
            resource_id,
            ResourceConsumerKind::new("workflow-trigger").expect("valid consumer kind"),
            consumer_identity(seed),
        ))
        .await
        .expect("subscription stores")
}

fn subscription_record(
    outcome: &PutResourceSubscriptionOutcome,
) -> &nebula_storage_port::dto::ResourceSubscriptionRecord {
    match outcome {
        PutResourceSubscriptionOutcome::Created(record)
        | PutResourceSubscriptionOutcome::Existing(record) => record,
    }
}

async fn acquire_source(
    store: &impl ResourceSourceLeaseStore,
    resource_id: SharedResourceId,
    holder_name: &str,
    lease_ttl: ResourceLeaseTtl,
) -> nebula_storage_port::dto::ResourceSourceLease {
    let outcome = store
        .acquire(AcquireResourceSourceLeaseRequest::new(
            scope(),
            resource_id,
            holder(holder_name),
            lease_ttl,
        ))
        .await
        .expect("source acquire succeeds");
    match outcome {
        AcquireResourceSourceLeaseOutcome::Acquired(lease) => lease,
        AcquireResourceSourceLeaseOutcome::Contended { .. } => {
            panic!("fresh or expired source lease must be acquired")
        },
    }
}

fn event_request(
    resource_id: SharedResourceId,
    token: nebula_storage_port::dto::ResourceSourceLeaseToken,
    occurrence: &[u8],
    schema_version: u32,
    payload: &[u8],
) -> AcceptResourceEventRequest {
    AcceptResourceEventRequest::new(
        scope(),
        resource_id,
        token,
        EventOccurrenceNamespace::new("telegram.update").expect("valid namespace"),
        EventOccurrenceKey::try_from_vec(occurrence.to_vec()).expect("valid occurrence"),
        EventEnvelope::try_from_vec(schema_version, payload.to_vec()).expect("valid envelope"),
    )
}

pub(crate) async fn exact_identity_and_scope_isolation(store: impl ResourceRuntimeUnderTest) {
    let first = resolve(&store, scope(), identity(b"config-a", b"slot-a")).await;
    let replay = resolve(&store, scope(), identity(b"config-a", b"slot-a")).await;
    let colliding_identity = identity(b"config-b", b"slot-a");
    let colliding_exact_other = resolve(&store, scope(), colliding_identity).await;
    let other_tenant = resolve(&store, other_scope(), identity(b"config-a", b"slot-a")).await;

    std::assert_matches!(first, ResolveSharedResourceOutcome::Created(_));
    std::assert_matches!(replay, ResolveSharedResourceOutcome::Existing(_));
    assert_eq!(resolved_id(&first), resolved_id(&replay));
    assert_ne!(resolved_id(&first), resolved_id(&colliding_exact_other));
    assert_ne!(resolved_id(&first), resolved_id(&other_tenant));
    assert_eq!(
        SharedResourceStore::get(&store, &other_scope(), resolved_id(&first))
            .await
            .expect("isolated read succeeds"),
        None
    );
}

pub(crate) async fn subscription_put_and_cas_states(store: impl ResourceRuntimeUnderTest) {
    let resource_id = resolved_id(&resolve(&store, scope(), identity(b"sub", b"slot")).await);
    let created = put_subscription(&store, resource_id, 1).await;
    let existing = put_subscription(&store, resource_id, 1).await;
    std::assert_matches!(created, PutResourceSubscriptionOutcome::Created(_));
    std::assert_matches!(existing, PutResourceSubscriptionOutcome::Existing(_));
    assert_eq!(
        subscription_record(&created),
        subscription_record(&existing)
    );
    assert_eq!(
        subscription_record(&created).state(),
        ResourceSubscriptionState::Active
    );
    assert_eq!(
        store
            .count_active_for_resource(&scope(), resource_id)
            .await
            .expect("active count"),
        1
    );
    let active = store
        .list_active_for_resource(
            &scope(),
            resource_id,
            None,
            ResourcePageSize::new(1).expect("page size"),
        )
        .await
        .expect("active page");
    assert_eq!(
        active.subscriptions(),
        std::slice::from_ref(subscription_record(&created))
    );
    let active_cursor = active.next_cursor().expect("full active page has cursor");
    let second = put_subscription(&store, resource_id, 2).await;
    store
        .put(PutResourceSubscriptionRequest::new(
            other_scope(),
            resource_id,
            ResourceConsumerKind::new("workflow-trigger").expect("valid consumer kind"),
            consumer_identity(9),
        ))
        .await
        .expect_err("foreign-scope resource is not visible");
    let third = put_subscription(&store, resource_id, 3).await;
    let remaining = store
        .list_active_for_resource(
            &scope(),
            resource_id,
            Some(active_cursor),
            ResourcePageSize::new(10).expect("page size"),
        )
        .await
        .expect("exclusive active page");
    assert_eq!(
        remaining
            .subscriptions()
            .iter()
            .map(nebula_storage_port::dto::ResourceSubscriptionRecord::id)
            .collect::<Vec<_>>(),
        vec![
            subscription_record(&second).id(),
            subscription_record(&third).id()
        ]
    );
    assert_eq!(
        store
            .count_active_for_resource(&scope(), resource_id)
            .await
            .expect("active count"),
        3
    );

    let subscription_id = subscription_record(&created).id();
    let disabled = store
        .transition(TransitionResourceSubscriptionRequest::new(
            scope(),
            subscription_id,
            ResourceSubscriptionVersion::new(1),
            ResourceSubscriptionState::Disabled,
        ))
        .await
        .expect("matching CAS transitions");
    assert_eq!(disabled.state(), ResourceSubscriptionState::Disabled);
    assert_eq!(disabled.version(), ResourceSubscriptionVersion::new(2));
    let repeated = store
        .transition(TransitionResourceSubscriptionRequest::new(
            scope(),
            subscription_id,
            ResourceSubscriptionVersion::new(1),
            ResourceSubscriptionState::Disabled,
        ))
        .await
        .expect("exact transition retry is idempotent");
    assert_eq!(repeated, disabled);
    assert_eq!(
        store
            .count_active_for_resource(&scope(), resource_id)
            .await
            .expect("disabled count"),
        2
    );
    let stale = store
        .transition(TransitionResourceSubscriptionRequest::new(
            scope(),
            subscription_id,
            ResourceSubscriptionVersion::new(1),
            ResourceSubscriptionState::Active,
        ))
        .await;
    std::assert_matches!(stale, Err(StorageError::Conflict { .. }));
    let tombstoned = store
        .transition(TransitionResourceSubscriptionRequest::new(
            scope(),
            subscription_id,
            ResourceSubscriptionVersion::new(2),
            ResourceSubscriptionState::Tombstoned,
        ))
        .await
        .expect("subscription tombstones");
    assert_eq!(tombstoned.state(), ResourceSubscriptionState::Tombstoned);
    let resurrection = store
        .transition(TransitionResourceSubscriptionRequest::new(
            scope(),
            subscription_id,
            tombstoned.version(),
            ResourceSubscriptionState::Active,
        ))
        .await;
    std::assert_matches!(resurrection, Err(StorageError::Conflict { .. }));
    let reconciliation = ResourceSubscriptionStore::list_for_reconciliation(
        &store,
        &scope(),
        None,
        ResourcePageSize::new(1).expect("page size"),
    )
    .await
    .expect("reconciliation page");
    assert_eq!(reconciliation.subscriptions(), &[tombstoned]);
}

pub(crate) async fn event_snapshot_replay_conflict_and_zero_subscriber(
    store: impl ResourceRuntimeUnderTest,
) {
    let resource_id = resolved_id(&resolve(&store, scope(), identity(b"events", b"slot")).await);
    put_subscription(&store, resource_id, 1).await;
    put_subscription(&store, resource_id, 2).await;
    let source = acquire_source(&store, resource_id, "source-a", ttl(30)).await;
    let accepted = store
        .accept(event_request(
            resource_id,
            source.token().clone(),
            b"event-1",
            1,
            b"payload-a",
        ))
        .await
        .expect("event accepts");
    let (event_id, delivery_count) = match accepted {
        AcceptResourceEventOutcome::Accepted {
            event_id,
            delivery_count,
        } => (event_id, delivery_count),
        other => panic!("fresh event must accept, got {other:?}"),
    };
    assert_eq!(delivery_count, 2);
    let event = store
        .get_event(&scope(), event_id)
        .await
        .expect("event read")
        .expect("event exists");
    assert_eq!(event.source_generation(), source.token().generation());
    assert!(event.accepted_at().timestamp_millis() > 0);

    store
        .release(ReleaseResourceSourceLeaseRequest::new(
            scope(),
            resource_id,
            source.token().clone(),
        ))
        .await
        .expect("source releases");
    let newer_source = acquire_source(&store, resource_id, "source-b", ttl(30)).await;
    assert!(newer_source.token().generation() > source.token().generation());
    let replay = store
        .accept(event_request(
            resource_id,
            newer_source.token().clone(),
            b"event-1",
            1,
            b"payload-a",
        ))
        .await
        .expect("event replays");
    assert_eq!(replay, AcceptResourceEventOutcome::Replayed { event_id });
    let replayed_event = store
        .get_event(&scope(), event_id)
        .await
        .expect("replayed event reads")
        .expect("replayed event exists");
    assert_eq!(replayed_event.accepted_at(), event.accepted_at());
    assert_eq!(
        replayed_event.source_generation(),
        source.token().generation()
    );

    let conflict = store
        .accept(event_request(
            resource_id,
            newer_source.token().clone(),
            b"event-1",
            1,
            b"payload-b",
        ))
        .await
        .expect("conflict is typed outcome");
    assert_eq!(conflict, AcceptResourceEventOutcome::Conflict { event_id });
    let replay_deliveries = store
        .claim_deliveries(ClaimResourceDeliveriesRequest::new(
            scope(),
            holder("replay-count"),
            ttl(30),
            ResourcePageSize::new(10).expect("batch"),
        ))
        .await
        .expect("replay deliveries claim");
    assert_eq!(replay_deliveries.len(), 2);
    for delivery in replay_deliveries {
        store
            .release_delivery(ReleaseResourceDeliveryRequest::new(
                scope(),
                delivery.id(),
                delivery.token().clone(),
            ))
            .await
            .expect("delivery releases");
    }

    let empty_resource =
        resolved_id(&resolve(&store, scope(), identity(b"empty-events", b"slot")).await);
    let empty_source = acquire_source(&store, empty_resource, "source-empty", ttl(30)).await;
    let empty_event = store
        .accept(event_request(
            empty_resource,
            empty_source.token().clone(),
            b"event-empty",
            1,
            b"payload",
        ))
        .await
        .expect("zero-subscriber event accepts");
    let empty_event_id = match empty_event {
        AcceptResourceEventOutcome::Accepted {
            event_id,
            delivery_count: 0,
        } => event_id,
        other => panic!("zero-subscriber event must accept complete, got {other:?}"),
    };
    assert_eq!(
        store
            .get_event(&scope(), empty_event_id)
            .await
            .expect("event read succeeds")
            .expect("event exists")
            .state(),
        ResourceEventState::Complete
    );
    assert!(
        store
            .claim_handoffs(ClaimResourceHandoffsRequest::new(
                scope(),
                holder("engine"),
                ttl(30),
                ResourcePageSize::new(1).expect("batch"),
            ))
            .await
            .expect("ineligible completion leaves no handoff")
            .is_empty()
    );
}

pub(crate) async fn source_token_liveness_and_exact_expiry(
    store: impl ResourceRuntimeUnderTest,
    control: &impl ResourceFanoutExpiryControl,
) {
    let resource_id = resolved_id(&resolve(&store, scope(), identity(b"lease", b"slot")).await);
    let original = acquire_source(&store, resource_id, "source-a", ttl(10)).await;
    control.expire_source_for_test(&scope(), resource_id).await;
    let takeover = acquire_source(&store, resource_id, "source-b", ttl(10)).await;
    assert!(takeover.token().generation() > original.token().generation());

    for token in [original.token().clone(), takeover.token().clone()] {
        let result = if token == *takeover.token() {
            control.expire_source_for_test(&scope(), resource_id).await;
            store
                .accept(event_request(resource_id, token, b"expired", 1, b"payload"))
                .await
        } else {
            store
                .accept(event_request(resource_id, token, b"stale", 1, b"payload"))
                .await
        };
        std::assert_matches!(result, Err(StorageError::FencedOut { .. }));
    }
}

pub(crate) async fn source_release_preserves_generation(store: impl ResourceRuntimeUnderTest) {
    let resource_id =
        resolved_id(&resolve(&store, scope(), identity(b"release-generation", b"slot")).await);
    let first = acquire_source(&store, resource_id, "source-a", ttl(30)).await;
    store
        .release(ReleaseResourceSourceLeaseRequest::new(
            scope(),
            resource_id,
            first.token().clone(),
        ))
        .await
        .expect("first source releases");
    let second = acquire_source(&store, resource_id, "source-b", ttl(30)).await;
    assert!(second.token().generation() > first.token().generation());

    store
        .release(ReleaseResourceSourceLeaseRequest::new(
            scope(),
            resource_id,
            first.token().clone(),
        ))
        .await
        .expect("stale release is idempotent");
    let contended = store
        .acquire(AcquireResourceSourceLeaseRequest::new(
            scope(),
            resource_id,
            holder("source-c"),
            ttl(30),
        ))
        .await
        .expect("live source reports contention");
    std::assert_matches!(
        contended,
        AcquireResourceSourceLeaseOutcome::Contended { .. }
    );
}

pub(crate) async fn concurrent_sibling_completions_terminalize_parent(
    store: impl ResourceRuntimeUnderTest,
) {
    let resource_id =
        resolved_id(&resolve(&store, scope(), identity(b"sibling-complete", b"slot")).await);
    put_subscription(&store, resource_id, 31).await;
    put_subscription(&store, resource_id, 32).await;
    let source = acquire_source(&store, resource_id, "source", ttl(30)).await;
    let event_id = match store
        .accept(event_request(
            resource_id,
            source.token().clone(),
            b"sibling-event",
            1,
            b"payload",
        ))
        .await
        .expect("event accepts")
    {
        AcceptResourceEventOutcome::Accepted { event_id, .. } => event_id,
        other => panic!("fresh event must accept, got {other:?}"),
    };
    let deliveries = store
        .claim_deliveries(ClaimResourceDeliveriesRequest::new(
            scope(),
            holder("fanout"),
            ttl(30),
            ResourcePageSize::new(2).expect("batch"),
        ))
        .await
        .expect("two deliveries claim");
    assert_eq!(deliveries.len(), 2);
    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let mut tasks = Vec::with_capacity(2);
    for delivery in deliveries {
        let task_store = store.clone();
        let task_barrier = Arc::clone(&barrier);
        tasks.push(tokio::spawn(async move {
            task_barrier.wait().await;
            task_store
                .complete_delivery(CompleteResourceDeliveryRequest::new(
                    scope(),
                    delivery.id(),
                    delivery.token().clone(),
                    ResourceDeliveryCompletion::Delivered,
                ))
                .await
                .expect("sibling completion succeeds")
        }));
    }
    let mut terminalizing_completions = 0;
    for task in tasks {
        if task.await.expect("completion task joins")
            == (CompleteResourceDeliveryOutcome::Completed {
                completion: ResourceDeliveryCompletion::Delivered,
                event_became_terminal: true,
            })
        {
            terminalizing_completions += 1;
        }
    }
    assert_eq!(terminalizing_completions, 1);
    assert_eq!(
        store
            .get_event(&scope(), event_id)
            .await
            .expect("event read")
            .expect("event exists")
            .state(),
        ResourceEventState::Complete
    );
}

pub(crate) async fn delivery_claim_lifecycle(
    store: impl ResourceRuntimeUnderTest,
    control: &impl ResourceFanoutExpiryControl,
) {
    let resource_id = resolved_id(&resolve(&store, scope(), identity(b"delivery", b"slot")).await);
    put_subscription(&store, resource_id, 1).await;
    let source = acquire_source(&store, resource_id, "source", ttl(30)).await;
    let event_id = match store
        .accept(event_request(
            resource_id,
            source.token().clone(),
            b"delivery-event",
            1,
            b"payload",
        ))
        .await
        .expect("event accepts")
    {
        AcceptResourceEventOutcome::Accepted { event_id, .. } => event_id,
        other => panic!("fresh event must accept, got {other:?}"),
    };
    let first = store
        .claim_deliveries(ClaimResourceDeliveriesRequest::new(
            scope(),
            holder("fanout-a"),
            ttl(5),
            ResourcePageSize::new(1).expect("batch"),
        ))
        .await
        .expect("delivery claims")
        .pop()
        .expect("one delivery");
    let heartbeat = store
        .heartbeat_delivery(HeartbeatResourceDeliveryRequest::new(
            scope(),
            first.id(),
            first.token().clone(),
            ttl(10),
        ))
        .await
        .expect("live exact claim heartbeats");
    control.expire_delivery_for_test(&scope(), first.id()).await;
    let takeover = store
        .claim_deliveries(ClaimResourceDeliveriesRequest::new(
            scope(),
            holder("fanout-b"),
            ttl(10),
            ResourcePageSize::new(1).expect("batch"),
        ))
        .await
        .expect("expired claim is reclaimed")
        .pop()
        .expect("one reclaimed delivery");
    assert!(takeover.token().generation() > heartbeat.token().generation());
    let stale_completion = store
        .complete_delivery(CompleteResourceDeliveryRequest::new(
            scope(),
            first.id(),
            first.token().clone(),
            ResourceDeliveryCompletion::Delivered,
        ))
        .await;
    std::assert_matches!(stale_completion, Err(StorageError::FencedOut { .. }));
    store
        .release_delivery(ReleaseResourceDeliveryRequest::new(
            scope(),
            takeover.id(),
            takeover.token().clone(),
        ))
        .await
        .expect("exact claim releases");
    let reclaimed = store
        .claim_deliveries(ClaimResourceDeliveriesRequest::new(
            scope(),
            holder("fanout-c"),
            ttl(10),
            ResourcePageSize::new(1).expect("batch"),
        ))
        .await
        .expect("released delivery reclaims")
        .pop()
        .expect("released delivery available");
    assert_eq!(reclaimed.id(), first.id());
    let completed = store
        .complete_delivery(CompleteResourceDeliveryRequest::new(
            scope(),
            reclaimed.id(),
            reclaimed.token().clone(),
            ResourceDeliveryCompletion::Ineligible(
                TerminalDeliveryIneligibility::ConsumerUnavailable,
            ),
        ))
        .await
        .expect("current claim completes");
    assert_eq!(
        completed,
        CompleteResourceDeliveryOutcome::Completed {
            completion: ResourceDeliveryCompletion::Ineligible(
                TerminalDeliveryIneligibility::ConsumerUnavailable,
            ),
            event_became_terminal: true
        }
    );
    assert_eq!(
        store
            .get_event(&scope(), event_id)
            .await
            .expect("event read")
            .expect("event exists")
            .state(),
        ResourceEventState::Complete
    );
    assert!(
        store
            .claim_handoffs(ClaimResourceHandoffsRequest::new(
                scope(),
                holder("engine-ineligible"),
                ttl(30),
                ResourcePageSize::new(1).expect("batch"),
            ))
            .await
            .expect("ineligible completion creates no handoff")
            .is_empty()
    );
}

pub(crate) async fn delivered_completion_rechecks_subscription(
    store: impl ResourceRuntimeUnderTest,
) {
    let resource_id =
        resolved_id(&resolve(&store, scope(), identity(b"eligibility-race", b"slot")).await);
    let subscription = put_subscription(&store, resource_id, 41).await;
    let subscription_id = subscription_record(&subscription).id();
    let source = acquire_source(&store, resource_id, "source", ttl(30)).await;
    store
        .accept(event_request(
            resource_id,
            source.token().clone(),
            b"eligibility-event",
            1,
            b"payload",
        ))
        .await
        .expect("event accepts");
    let delivery = store
        .claim_deliveries(ClaimResourceDeliveriesRequest::new(
            scope(),
            holder("fanout"),
            ttl(30),
            ResourcePageSize::new(1).expect("batch"),
        ))
        .await
        .expect("delivery claims")
        .pop()
        .expect("delivery exists");
    store
        .transition(TransitionResourceSubscriptionRequest::new(
            scope(),
            subscription_id,
            ResourceSubscriptionVersion::new(1),
            ResourceSubscriptionState::Disabled,
        ))
        .await
        .expect("subscription disables");
    let outcome = store
        .complete_delivery(CompleteResourceDeliveryRequest::new(
            scope(),
            delivery.id(),
            delivery.token().clone(),
            ResourceDeliveryCompletion::Delivered,
        ))
        .await
        .expect("completion persists current eligibility");
    assert_eq!(
        outcome,
        CompleteResourceDeliveryOutcome::Completed {
            completion: ResourceDeliveryCompletion::Ineligible(
                TerminalDeliveryIneligibility::SubscriptionDisabled,
            ),
            event_became_terminal: true,
        }
    );
    store
        .transition(TransitionResourceSubscriptionRequest::new(
            scope(),
            subscription_id,
            ResourceSubscriptionVersion::new(2),
            ResourceSubscriptionState::Active,
        ))
        .await
        .expect("subscription re-enables after terminal delivery");
    assert_eq!(
        store
            .complete_delivery(CompleteResourceDeliveryRequest::new(
                scope(),
                delivery.id(),
                delivery.token().clone(),
                ResourceDeliveryCompletion::Delivered,
            ))
            .await
            .expect("identical requested completion replays"),
        CompleteResourceDeliveryOutcome::AlreadyCompleted {
            completion: ResourceDeliveryCompletion::Ineligible(
                TerminalDeliveryIneligibility::SubscriptionDisabled,
            ),
            event_is_terminal: true,
        }
    );
    assert_matches!(
        store
            .complete_delivery(CompleteResourceDeliveryRequest::new(
                scope(),
                delivery.id(),
                delivery.token().clone(),
                ResourceDeliveryCompletion::Ineligible(
                    TerminalDeliveryIneligibility::ConsumerUnavailable,
                ),
            ))
            .await,
        Err(StorageError::FencedOut { .. })
    );
    assert!(
        store
            .claim_handoffs(ClaimResourceHandoffsRequest::new(
                scope(),
                holder("engine"),
                ttl(30),
                ResourcePageSize::new(1).expect("batch"),
            ))
            .await
            .expect("handoff scan succeeds")
            .is_empty()
    );
}

pub(crate) async fn delivered_completion_creates_exact_recoverable_handoff(
    store: impl ResourceRuntimeUnderTest,
) {
    let resource_id = resolved_id(&resolve(&store, scope(), identity(b"handoff", b"slot")).await);
    put_subscription(&store, resource_id, 7).await;
    let source = acquire_source(&store, resource_id, "source", ttl(30)).await;
    store
        .accept(event_request(
            resource_id,
            source.token().clone(),
            b"handoff-event",
            1,
            b"payload",
        ))
        .await
        .expect("event accepts");
    let delivery = store
        .claim_deliveries(ClaimResourceDeliveriesRequest::new(
            scope(),
            holder("fanout"),
            ttl(30),
            ResourcePageSize::new(1).expect("batch"),
        ))
        .await
        .expect("delivery claims")
        .pop()
        .expect("delivery exists");
    store
        .complete_delivery(CompleteResourceDeliveryRequest::new(
            scope(),
            delivery.id(),
            delivery.token().clone(),
            ResourceDeliveryCompletion::Delivered,
        ))
        .await
        .expect("delivery completes");

    let first = store
        .claim_handoffs(ClaimResourceHandoffsRequest::new(
            scope(),
            holder("engine-a"),
            ttl(30),
            ResourcePageSize::new(1).expect("batch"),
        ))
        .await
        .expect("handoff claims")
        .pop()
        .expect("handoff exists");
    assert_eq!(first.delivery_id(), delivery.id());
    let heartbeat = store
        .heartbeat_handoff(HeartbeatResourceHandoffRequest::new(
            ResourceHandoffClaimRequest::new(scope(), first.delivery_id(), first.token().clone()),
            ttl(30),
        ))
        .await
        .expect("handoff heartbeats");
    store
        .release_handoff(ResourceHandoffClaimRequest::new(
            scope(),
            heartbeat.delivery_id(),
            heartbeat.token().clone(),
        ))
        .await
        .expect("handoff releases");
    let reclaimed = store
        .claim_handoffs(ClaimResourceHandoffsRequest::new(
            scope(),
            holder("engine-b"),
            ttl(30),
            ResourcePageSize::new(1).expect("batch"),
        ))
        .await
        .expect("handoff reclaims")
        .pop()
        .expect("released handoff exists");
    assert!(reclaimed.token().generation() > first.token().generation());
    let stale = store
        .acknowledge_handoff(ResourceHandoffClaimRequest::new(
            scope(),
            first.delivery_id(),
            first.token().clone(),
        ))
        .await;
    std::assert_matches!(stale, Err(StorageError::FencedOut { .. }));
    let acknowledgement = ResourceHandoffClaimRequest::new(
        scope(),
        reclaimed.delivery_id(),
        reclaimed.token().clone(),
    );
    assert_eq!(
        store
            .acknowledge_handoff(acknowledgement.clone())
            .await
            .expect("handoff acknowledges"),
        AcknowledgeResourceHandoffOutcome::Acknowledged
    );
    assert_eq!(
        store
            .acknowledge_handoff(acknowledgement)
            .await
            .expect("acknowledgement replays"),
        AcknowledgeResourceHandoffOutcome::AlreadyAcknowledged
    );
}

pub(crate) async fn reconciliation_pages_are_stable_and_exclusive(
    store: impl ResourceRuntimeUnderTest,
) {
    let first = resolved_id(&resolve(&store, scope(), identity(b"page-1", b"slot")).await);
    let second = resolved_id(&resolve(&store, scope(), identity(b"page-2", b"slot")).await);
    resolve(&store, other_scope(), identity(b"other-page", b"slot")).await;
    let first_page = SharedResourceStore::list_for_reconciliation(
        &store,
        &scope(),
        None,
        ResourcePageSize::new(1).expect("page size"),
    )
    .await
    .expect("first page");
    assert_eq!(first_page.resources().len(), 1);
    assert_eq!(first_page.resources()[0].id(), first);
    let cursor = first_page.next_cursor().expect("full page has cursor");
    let inserted = resolved_id(&resolve(&store, scope(), identity(b"page-3", b"slot")).await);
    let second_page = SharedResourceStore::list_for_reconciliation(
        &store,
        &scope(),
        Some(cursor),
        ResourcePageSize::new(10).expect("page size"),
    )
    .await
    .expect("second page");
    assert_eq!(
        second_page
            .resources()
            .iter()
            .map(nebula_storage_port::dto::SharedResourceRecord::id)
            .collect::<Vec<_>>(),
        vec![second, inserted]
    );
    let empty = SharedResourceStore::list_for_reconciliation(
        &store,
        &scope(),
        Some(ReconciliationCursor::from_sequence(u64::MAX)),
        ResourcePageSize::new(10).expect("page size"),
    )
    .await
    .expect("empty page");
    assert!(empty.resources().is_empty());
    assert_eq!(empty.next_cursor(), None);
}

pub(crate) async fn barrier_contention_has_single_winners(store: impl ResourceRuntimeUnderTest) {
    let barrier = Arc::new(tokio::sync::Barrier::new(8));
    let mut tasks = Vec::new();
    for _ in 0..8 {
        let task_store = store.clone();
        let task_barrier = Arc::clone(&barrier);
        tasks.push(tokio::spawn(async move {
            task_barrier.wait().await;
            resolve(&task_store, scope(), identity(b"raced", b"slot")).await
        }));
    }
    let mut created = 0;
    let mut resource_id = None;
    for task in tasks {
        let outcome = task.await.expect("resolve task joins");
        created += usize::from(matches!(outcome, ResolveSharedResourceOutcome::Created(_)));
        resource_id = Some(resolved_id(&outcome));
    }
    assert_eq!(created, 1);
    let resource_id = resource_id.expect("resolve tasks produce id");

    let barrier = Arc::new(tokio::sync::Barrier::new(8));
    let mut tasks = Vec::new();
    for _ in 0..8 {
        let task_store = store.clone();
        let task_barrier = Arc::clone(&barrier);
        tasks.push(tokio::spawn(async move {
            task_barrier.wait().await;
            put_subscription(&task_store, resource_id, 9).await
        }));
    }
    let mut subscription_created = 0;
    for task in tasks {
        subscription_created += usize::from(matches!(
            task.await.expect("put task joins"),
            PutResourceSubscriptionOutcome::Created(_)
        ));
    }
    assert_eq!(subscription_created, 1);

    let source = acquire_source(&store, resource_id, "source", ttl(30)).await;
    let barrier = Arc::new(tokio::sync::Barrier::new(8));
    let mut tasks = Vec::new();
    for _ in 0..8 {
        let task_store = store.clone();
        let task_barrier = Arc::clone(&barrier);
        let token = source.token().clone();
        tasks.push(tokio::spawn(async move {
            task_barrier.wait().await;
            task_store
                .accept(event_request(
                    resource_id,
                    token,
                    b"raced-event",
                    1,
                    b"payload",
                ))
                .await
                .expect("event race returns typed outcome")
        }));
    }
    let mut accepted = 0;
    let mut event_id = None;
    for task in tasks {
        let outcome = task.await.expect("accept task joins");
        if let AcceptResourceEventOutcome::Accepted { event_id: id, .. } = outcome {
            accepted += 1;
            event_id = Some(id);
        }
    }
    assert_eq!(accepted, 1);
    assert!(event_id.is_some());

    let barrier = Arc::new(tokio::sync::Barrier::new(8));
    let mut tasks = Vec::new();
    for index in 0..8 {
        let task_store = store.clone();
        let task_barrier = Arc::clone(&barrier);
        tasks.push(tokio::spawn(async move {
            task_barrier.wait().await;
            task_store
                .claim_deliveries(ClaimResourceDeliveriesRequest::new(
                    scope(),
                    holder(&format!("fanout-{index}")),
                    ttl(10),
                    ResourcePageSize::new(1).expect("batch size"),
                ))
                .await
                .expect("claim race succeeds")
                .len()
        }));
    }
    let mut claimed = 0;
    for task in tasks {
        claimed += task.await.expect("claim task joins");
    }
    assert_eq!(claimed, 1);
}

pub(crate) fn opaque_debug_contract(store: &impl std::fmt::Debug) {
    let debug = format!("{store:?}");
    assert!(!debug.contains("configuration-secret"));
    assert!(!debug.contains("payload-secret"));
    assert!(!debug.contains("claim_id"));
}

#[macro_export]
macro_rules! resource_fanout_conformance_suite {
    ($factory:expr) => {
        #[tokio::test]
        async fn exact_identity_and_scope_isolation() {
            let (store, _control) = $factory.await;
            oracle::exact_identity_and_scope_isolation(store).await;
        }

        #[tokio::test]
        async fn subscription_put_and_cas_states() {
            let (store, _control) = $factory.await;
            oracle::subscription_put_and_cas_states(store).await;
        }

        #[tokio::test]
        async fn event_snapshot_replay_conflict_and_zero_subscriber() {
            let (store, _control) = $factory.await;
            oracle::event_snapshot_replay_conflict_and_zero_subscriber(store).await;
        }

        #[tokio::test]
        async fn source_token_liveness_and_exact_expiry() {
            let (store, control) = $factory.await;
            oracle::source_token_liveness_and_exact_expiry(store, &control).await;
        }

        #[tokio::test]
        async fn source_release_preserves_generation() {
            let (store, _control) = $factory.await;
            oracle::source_release_preserves_generation(store).await;
        }

        #[tokio::test]
        async fn concurrent_sibling_completions_terminalize_parent() {
            let (store, _control) = $factory.await;
            oracle::concurrent_sibling_completions_terminalize_parent(store).await;
        }

        #[tokio::test]
        async fn delivery_claim_lifecycle() {
            let (store, control) = $factory.await;
            oracle::delivery_claim_lifecycle(store, &control).await;
        }

        #[tokio::test]
        async fn delivered_completion_creates_exact_recoverable_handoff() {
            let (store, _control) = $factory.await;
            oracle::delivered_completion_creates_exact_recoverable_handoff(store).await;
        }

        #[tokio::test]
        async fn delivered_completion_rechecks_subscription() {
            let (store, _control) = $factory.await;
            oracle::delivered_completion_rechecks_subscription(store).await;
        }

        #[tokio::test]
        async fn reconciliation_pages_are_stable_and_exclusive() {
            let (store, _control) = $factory.await;
            oracle::reconciliation_pages_are_stable_and_exclusive(store).await;
        }

        #[tokio::test]
        async fn barrier_contention_has_single_winners() {
            let (store, _control) = $factory.await;
            oracle::barrier_contention_has_single_winners(store).await;
        }

        #[tokio::test]
        async fn opaque_debug_contract() {
            let (store, _control) = $factory.await;
            oracle::opaque_debug_contract(&store);
        }
    };
}

#[macro_export]
macro_rules! optional_resource_fanout_conformance_suite {
    ($factory:expr) => {
        #[tokio::test]
        async fn exact_identity_and_scope_isolation() {
            let Some((store, _control)) = $factory.await else {
                return;
            };
            oracle::exact_identity_and_scope_isolation(store).await;
        }
        #[tokio::test]
        async fn subscription_put_and_cas_states() {
            let Some((store, _control)) = $factory.await else {
                return;
            };
            oracle::subscription_put_and_cas_states(store).await;
        }
        #[tokio::test]
        async fn event_snapshot_replay_conflict_and_zero_subscriber() {
            let Some((store, _control)) = $factory.await else {
                return;
            };
            oracle::event_snapshot_replay_conflict_and_zero_subscriber(store).await;
        }
        #[tokio::test]
        async fn source_token_liveness_and_exact_expiry() {
            let Some((store, control)) = $factory.await else {
                return;
            };
            oracle::source_token_liveness_and_exact_expiry(store, &control).await;
        }
        #[tokio::test]
        async fn source_release_preserves_generation() {
            let Some((store, _control)) = $factory.await else {
                return;
            };
            oracle::source_release_preserves_generation(store).await;
        }
        #[tokio::test]
        async fn concurrent_sibling_completions_terminalize_parent() {
            let Some((store, _control)) = $factory.await else {
                return;
            };
            oracle::concurrent_sibling_completions_terminalize_parent(store).await;
        }
        #[tokio::test]
        async fn delivery_claim_lifecycle() {
            let Some((store, control)) = $factory.await else {
                return;
            };
            oracle::delivery_claim_lifecycle(store, &control).await;
        }

        #[tokio::test]
        async fn delivered_completion_creates_exact_recoverable_handoff() {
            let Some((store, _control)) = $factory.await else {
                return;
            };
            oracle::delivered_completion_creates_exact_recoverable_handoff(store).await;
        }
        #[tokio::test]
        async fn delivered_completion_rechecks_subscription() {
            let Some((store, _control)) = $factory.await else {
                return;
            };
            oracle::delivered_completion_rechecks_subscription(store).await;
        }
        #[tokio::test]
        async fn reconciliation_pages_are_stable_and_exclusive() {
            let Some((store, _control)) = $factory.await else {
                return;
            };
            oracle::reconciliation_pages_are_stable_and_exclusive(store).await;
        }
        #[tokio::test]
        async fn barrier_contention_has_single_winners() {
            let Some((store, _control)) = $factory.await else {
                return;
            };
            oracle::barrier_contention_has_single_winners(store).await;
        }
        #[tokio::test]
        async fn opaque_debug_contract() {
            let Some((store, _control)) = $factory.await else {
                return;
            };
            oracle::opaque_debug_contract(&store);
        }
    };
}
