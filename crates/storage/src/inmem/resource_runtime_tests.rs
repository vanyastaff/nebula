use std::assert_matches;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use nebula_core::accessor::Clock;
use nebula_storage_port::dto::{
    AcceptResourceEventRequest, AcquireResourceSourceLeaseOutcome,
    AcquireResourceSourceLeaseRequest, ClaimResourceDeliveriesRequest, EventEnvelope,
    EventOccurrenceKey, EventOccurrenceNamespace, HeartbeatResourceSourceLeaseRequest,
    PutResourceSubscriptionRequest, ResolveSharedResourceOutcome, ResolveSharedResourceRequest,
    ResourceCompatibilityVersion, ResourceConfigurationIdentity, ResourceConsumerIdentity,
    ResourceConsumerKind, ResourceDeliveryId, ResourceEventId, ResourceKind,
    ResourceLeaseGeneration, ResourceLeaseHolder, ResourceLeaseTtl, ResourcePageSize,
    ResourceSlotIdentity, ResourceSubscriptionId, SharedResourceIdentity,
};
use nebula_storage_port::store::{
    ResourceEventFanoutStore, ResourceSourceLeaseStore, ResourceSubscriptionStore,
    SharedResourceStore,
};
use nebula_storage_port::{Scope, StorageError};

use super::{DeliveryRow, InMemoryResourceRuntime};

#[derive(Debug)]
struct ContendedClock {
    timestamp_seconds: AtomicI64,
    reads: AtomicUsize,
    monotonic_origin: Instant,
}

impl ContendedClock {
    fn new(timestamp_seconds: i64) -> Self {
        Self {
            timestamp_seconds: AtomicI64::new(timestamp_seconds),
            reads: AtomicUsize::new(0),
            monotonic_origin: Instant::now(),
        }
    }

    fn advance(&self, seconds: i64) {
        self.timestamp_seconds.fetch_add(seconds, Ordering::SeqCst);
    }
}

impl Clock for ContendedClock {
    fn now(&self) -> DateTime<Utc> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        DateTime::from_timestamp(self.timestamp_seconds.load(Ordering::SeqCst), 0)
            .expect("test timestamp is valid")
    }

    fn monotonic(&self) -> Instant {
        self.monotonic_origin
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn heartbeat_samples_clock_only_after_waiting_for_state_lock() {
    let clock = Arc::new(ContendedClock::new(1_800_000_000));
    let store = InMemoryResourceRuntime::with_clock(clock.clone());
    let scope = Scope::new("clock-workspace", "clock-org");
    let identity = SharedResourceIdentity::new(
        ResourceKind::new("clock.resource").expect("valid kind"),
        ResourceCompatibilityVersion::new(1),
        ResourceConfigurationIdentity::try_from_vec(b"configuration".to_vec())
            .expect("valid configuration"),
        ResourceSlotIdentity::try_from_vec(Vec::new()).expect("valid slot"),
    );
    let resource_id = match store
        .resolve(ResolveSharedResourceRequest::new(scope.clone(), identity))
        .await
        .expect("resource resolves")
    {
        ResolveSharedResourceOutcome::Created(record)
        | ResolveSharedResourceOutcome::Existing(record) => record.id(),
    };
    let lease = match store
        .acquire(AcquireResourceSourceLeaseRequest::new(
            scope.clone(),
            resource_id,
            ResourceLeaseHolder::new("source").expect("valid holder"),
            ResourceLeaseTtl::new(Duration::from_secs(1)).expect("valid TTL"),
        ))
        .await
        .expect("source acquires")
    {
        AcquireResourceSourceLeaseOutcome::Acquired(lease) => lease,
        AcquireResourceSourceLeaseOutcome::Contended { .. } => {
            panic!("fresh source must acquire")
        },
    };
    let reads_before_contention = clock.reads.load(Ordering::SeqCst);
    let state_guard = store.inner.lock();
    let start = Arc::new(Barrier::new(2));
    let task_start = start.clone();
    let task_store = store.clone();
    let handle = tokio::runtime::Handle::current();
    let request = HeartbeatResourceSourceLeaseRequest::new(
        scope,
        resource_id,
        lease.token().clone(),
        ResourceLeaseTtl::new(Duration::from_secs(1)).expect("valid TTL"),
    );
    let heartbeat = std::thread::spawn(move || {
        task_start.wait();
        handle.block_on(task_store.heartbeat(request))
    });
    start.wait();
    for _ in 0..1_000 {
        std::thread::yield_now();
    }
    assert_eq!(clock.reads.load(Ordering::SeqCst), reads_before_contention);
    clock.advance(2);
    drop(state_guard);
    assert_matches!(
        heartbeat.join().expect("heartbeat thread joins"),
        Err(StorageError::FencedOut { .. })
    );
}

#[tokio::test]
async fn forced_digest_collision_still_uses_exact_identity() {
    let store = InMemoryResourceRuntime::new();
    store.inner.lock().force_identity_digest_collision = true;
    let scope = Scope::new("collision-workspace", "collision-org");
    let make_identity = |configuration: &[u8]| {
        SharedResourceIdentity::new(
            ResourceKind::new("collision.resource").expect("valid kind"),
            ResourceCompatibilityVersion::new(1),
            ResourceConfigurationIdentity::try_from_vec(configuration.to_vec())
                .expect("valid configuration"),
            ResourceSlotIdentity::try_from_vec(Vec::new()).expect("valid slot"),
        )
    };
    let first = store
        .resolve(ResolveSharedResourceRequest::new(
            scope.clone(),
            make_identity(b"first"),
        ))
        .await
        .expect("first identity resolves");
    let second = store
        .resolve(ResolveSharedResourceRequest::new(
            scope.clone(),
            make_identity(b"second"),
        ))
        .await
        .expect("colliding exact identity resolves");
    let replay = store
        .resolve(ResolveSharedResourceRequest::new(
            scope,
            make_identity(b"first"),
        ))
        .await
        .expect("first identity replays");
    let id = |outcome: ResolveSharedResourceOutcome| match outcome {
        ResolveSharedResourceOutcome::Created(record)
        | ResolveSharedResourceOutcome::Existing(record) => record.id(),
    };
    let first_id = id(first);
    assert_ne!(first_id, id(second));
    assert_eq!(first_id, id(replay));
}

#[tokio::test]
async fn source_and_delivery_generation_overflow_leave_state_unchanged() {
    let clock = Arc::new(ContendedClock::new(1_800_000_000));
    let store = InMemoryResourceRuntime::with_clock(clock);
    let scope = Scope::new("overflow-workspace", "overflow-org");
    let identity = SharedResourceIdentity::new(
        ResourceKind::new("overflow.resource").expect("valid kind"),
        ResourceCompatibilityVersion::new(1),
        ResourceConfigurationIdentity::try_from_vec(b"configuration".to_vec())
            .expect("valid configuration"),
        ResourceSlotIdentity::try_from_vec(Vec::new()).expect("valid slot"),
    );
    let resource_id = match store
        .resolve(ResolveSharedResourceRequest::new(scope.clone(), identity))
        .await
        .expect("resource resolves")
    {
        ResolveSharedResourceOutcome::Created(record)
        | ResolveSharedResourceOutcome::Existing(record) => record.id(),
    };
    store
        .acquire(AcquireResourceSourceLeaseRequest::new(
            scope.clone(),
            resource_id,
            ResourceLeaseHolder::new("source").expect("valid holder"),
            ResourceLeaseTtl::new(Duration::from_secs(1)).expect("valid TTL"),
        ))
        .await
        .expect("source acquires");
    {
        let mut state = store.inner.lock();
        let lease = state
            .source_leases
            .get_mut(&(scope.clone(), resource_id))
            .expect("source row exists");
        lease.token = nebula_storage_port::dto::ResourceSourceLeaseToken::new(
            lease.token.claim_id(),
            ResourceLeaseGeneration::new(u64::MAX),
        );
        lease.expires_at =
            DateTime::from_timestamp(1_700_000_000, 0).expect("test timestamp is valid");
    }
    assert_matches!(
        store
            .acquire(AcquireResourceSourceLeaseRequest::new(
                scope.clone(),
                resource_id,
                ResourceLeaseHolder::new("source-next").expect("valid holder"),
                ResourceLeaseTtl::new(Duration::from_secs(1)).expect("valid TTL"),
            ))
            .await,
        Err(StorageError::Internal(_))
    );
    assert_eq!(
        store
            .inner
            .lock()
            .source_leases
            .get(&(scope.clone(), resource_id))
            .expect("source row remains")
            .token
            .generation(),
        ResourceLeaseGeneration::new(u64::MAX)
    );

    let delivery_id = ResourceDeliveryId::from_bytes([7; 16]);
    store.inner.lock().deliveries.insert(
        (scope.clone(), delivery_id),
        DeliveryRow {
            sequence: 1,
            id: delivery_id,
            event_id: ResourceEventId::from_bytes([8; 16]),
            subscription_id: ResourceSubscriptionId::from_bytes([9; 16]),
            envelope: EventEnvelope::try_from_vec(1, b"payload".to_vec()).expect("valid envelope"),
            last_generation: ResourceLeaseGeneration::new(u64::MAX),
            claim: None,
            terminal: None,
        },
    );
    assert_matches!(
        store
            .claim_deliveries(ClaimResourceDeliveriesRequest::new(
                scope.clone(),
                ResourceLeaseHolder::new("fanout").expect("valid holder"),
                ResourceLeaseTtl::new(Duration::from_secs(1)).expect("valid TTL"),
                ResourcePageSize::new(1).expect("valid batch"),
            ))
            .await,
        Err(StorageError::Internal(_))
    );
    let state = store.inner.lock();
    let delivery = state
        .deliveries
        .get(&(scope, delivery_id))
        .expect("delivery row remains");
    assert_eq!(
        delivery.last_generation,
        ResourceLeaseGeneration::new(u64::MAX)
    );
    assert!(delivery.claim.is_none());
}

#[tokio::test]
async fn accept_sequence_exhaustion_leaves_no_partial_event_or_delivery() {
    let store = InMemoryResourceRuntime::new();
    let scope = Scope::new("accept-overflow-workspace", "accept-overflow-org");
    let identity = SharedResourceIdentity::new(
        ResourceKind::new("accept-overflow.resource").expect("valid kind"),
        ResourceCompatibilityVersion::new(1),
        ResourceConfigurationIdentity::try_from_vec(b"configuration".to_vec())
            .expect("valid configuration"),
        ResourceSlotIdentity::try_from_vec(Vec::new()).expect("valid slot"),
    );
    let resource_id = match store
        .resolve(ResolveSharedResourceRequest::new(scope.clone(), identity))
        .await
        .expect("resource resolves")
    {
        ResolveSharedResourceOutcome::Created(record)
        | ResolveSharedResourceOutcome::Existing(record) => record.id(),
    };
    store
        .put(PutResourceSubscriptionRequest::new(
            scope.clone(),
            resource_id,
            ResourceConsumerKind::new("workflow-trigger").expect("valid consumer kind"),
            ResourceConsumerIdentity::try_from_vec(b"consumer".to_vec())
                .expect("valid consumer identity"),
        ))
        .await
        .expect("subscription stores");
    let lease = match store
        .acquire(AcquireResourceSourceLeaseRequest::new(
            scope.clone(),
            resource_id,
            ResourceLeaseHolder::new("source").expect("valid holder"),
            ResourceLeaseTtl::new(Duration::from_secs(30)).expect("valid TTL"),
        ))
        .await
        .expect("source acquires")
    {
        AcquireResourceSourceLeaseOutcome::Acquired(lease) => lease,
        AcquireResourceSourceLeaseOutcome::Contended { .. } => {
            panic!("fresh source must acquire")
        },
    };
    {
        let mut state = store.inner.lock();
        state.next_delivery_sequence = u64::MAX;
    }

    let result = store
        .accept(AcceptResourceEventRequest::new(
            scope,
            resource_id,
            lease.token().clone(),
            EventOccurrenceNamespace::new("overflow").expect("valid namespace"),
            EventOccurrenceKey::try_from_vec(b"occurrence".to_vec()).expect("valid occurrence"),
            EventEnvelope::try_from_vec(1, b"payload".to_vec()).expect("valid envelope"),
        ))
        .await;

    assert_matches!(result, Err(StorageError::Internal(_)));
    let state = store.inner.lock();
    assert!(state.events.is_empty());
    assert!(state.event_occurrences.is_empty());
    assert!(state.deliveries.is_empty());
    assert_eq!(state.next_delivery_sequence, u64::MAX);
}
