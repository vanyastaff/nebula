//! Shared-resource fanout conformance for the SQLite adapter.

#![cfg(feature = "sqlite")]

#[macro_use]
#[path = "support/resource_fanout_oracle.rs"]
mod oracle;

use nebula_storage::sqlite::{SqliteResourceRuntime, init_schema};
use nebula_storage_port::dto::{
    AcceptResourceEventOutcome, AcceptResourceEventRequest, AcquireResourceSourceLeaseOutcome,
    AcquireResourceSourceLeaseRequest, ClaimResourceDeliveriesRequest,
    CompleteResourceDeliveryRequest, EventEnvelope, EventOccurrenceKey, EventOccurrenceNamespace,
    PutResourceSubscriptionRequest, ResolveSharedResourceOutcome, ResolveSharedResourceRequest,
    ResourceCompatibilityVersion, ResourceConfigurationIdentity, ResourceConsumerIdentity,
    ResourceConsumerKind, ResourceDeliveryCompletion, ResourceDeliveryId, ResourceKind,
    ResourceLeaseHolder, ResourceLeaseTtl, ResourcePageSize, ResourceSlotIdentity,
    SharedResourceId, SharedResourceIdentity,
};
use nebula_storage_port::store::{
    ResourceEventFanoutStore, ResourceSourceLeaseStore, ResourceSubscriptionStore,
    SharedResourceStore,
};
use nebula_storage_port::{Scope, StorageError};
use sqlx::SqlitePool;
use sqlx::sqlite::SqlitePoolOptions;

struct SqliteExpiryControl {
    pool: SqlitePool,
}

#[async_trait::async_trait]
impl oracle::ResourceFanoutExpiryControl for SqliteExpiryControl {
    async fn expire_source_for_test(&self, scope: &Scope, resource_id: SharedResourceId) {
        sqlx::query("UPDATE port_resource_source_leases SET expires_at_ms = CAST((julianday('now') - 2440587.5) * 86400000.0 AS INTEGER) - 1 WHERE workspace_id = ? AND org_id = ? AND resource_id = ?")
            .bind(&scope.workspace_id).bind(&scope.org_id).bind(resource_id.into_bytes().as_slice())
            .execute(&self.pool)
            .await
            .expect("source backdate succeeds");
    }

    async fn expire_delivery_for_test(&self, scope: &Scope, delivery_id: ResourceDeliveryId) {
        sqlx::query("UPDATE port_resource_deliveries SET claim_expires_at_ms = CAST((julianday('now') - 2440587.5) * 86400000.0 AS INTEGER) - 1 WHERE workspace_id = ? AND org_id = ? AND id = ?")
            .bind(&scope.workspace_id).bind(&scope.org_id).bind(delivery_id.into_bytes().as_slice())
            .execute(&self.pool)
            .await
            .expect("delivery backdate succeeds");
    }
}

async fn pool() -> SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("SQLite pool connects");
    init_schema(&pool).await.expect("schema initializes");
    pool
}

async fn runtime() -> (SqliteResourceRuntime, SqliteExpiryControl) {
    let pool = pool().await;
    (
        SqliteResourceRuntime::new(pool.clone()),
        SqliteExpiryControl { pool },
    )
}

resource_fanout_conformance_suite!(runtime());

#[tokio::test]
async fn digest_collision_and_generation_overflow_use_test_owned_pool() {
    let pool = pool().await;
    let store = SqliteResourceRuntime::new(pool.clone());
    let scope = test_scope();
    let first_id = match store
        .resolve(ResolveSharedResourceRequest::new(
            scope.clone(),
            test_identity(),
        ))
        .await
        .expect("first resource resolves")
    {
        ResolveSharedResourceOutcome::Created(record)
        | ResolveSharedResourceOutcome::Existing(record) => record.id(),
    };
    let colliding_identity = SharedResourceIdentity::new(
        ResourceKind::new("test.resource").expect("valid kind"),
        ResourceCompatibilityVersion::new(1),
        ResourceConfigurationIdentity::try_from_vec(b"different-configuration".to_vec())
            .expect("valid configuration"),
        ResourceSlotIdentity::try_from_vec(b"slot".to_vec()).expect("valid slot"),
    );
    sqlx::query("UPDATE port_shared_resources SET identity_digest = ? WHERE workspace_id = ? AND org_id = ? AND id = ?")
        .bind(colliding_identity.digest().as_slice()).bind(&scope.workspace_id).bind(&scope.org_id)
        .bind(first_id.into_bytes().as_slice()).execute(&pool).await.expect("digest collision installs");
    let second_id = match store
        .resolve(ResolveSharedResourceRequest::new(
            scope.clone(),
            colliding_identity,
        ))
        .await
        .expect("colliding exact resource resolves")
    {
        ResolveSharedResourceOutcome::Created(record)
        | ResolveSharedResourceOutcome::Existing(record) => record.id(),
    };
    assert_ne!(first_id, second_id);

    let source = match store
        .acquire(AcquireResourceSourceLeaseRequest::new(
            scope.clone(),
            first_id,
            ResourceLeaseHolder::new("source").expect("valid holder"),
            ResourceLeaseTtl::new(std::time::Duration::from_secs(1)).expect("valid TTL"),
        ))
        .await
        .expect("source acquires")
    {
        AcquireResourceSourceLeaseOutcome::Acquired(lease) => lease,
        AcquireResourceSourceLeaseOutcome::Contended { .. } => panic!("fresh source acquires"),
    };
    store
        .put(PutResourceSubscriptionRequest::new(
            scope.clone(),
            first_id,
            ResourceConsumerKind::new("overflow-consumer").expect("valid kind"),
            ResourceConsumerIdentity::try_from_vec(b"overflow-consumer".to_vec())
                .expect("valid identity"),
        ))
        .await
        .expect("subscription stores");
    store
        .accept(AcceptResourceEventRequest::new(
            scope.clone(),
            first_id,
            source.token().clone(),
            EventOccurrenceNamespace::new("overflow.event").expect("valid namespace"),
            EventOccurrenceKey::try_from_vec(b"overflow-event".to_vec()).expect("valid key"),
            EventEnvelope::try_from_vec(1, b"payload".to_vec()).expect("valid envelope"),
        ))
        .await
        .expect("event accepts");
    let delivery = store
        .claim_deliveries(ClaimResourceDeliveriesRequest::new(
            scope.clone(),
            ResourceLeaseHolder::new("fanout").expect("valid holder"),
            ResourceLeaseTtl::new(std::time::Duration::from_secs(1)).expect("valid TTL"),
            ResourcePageSize::new(1).expect("valid batch"),
        ))
        .await
        .expect("delivery claims")
        .pop()
        .expect("delivery exists");
    sqlx::query("UPDATE port_resource_deliveries SET claim_generation = ?, claim_expires_at_ms = 0 WHERE workspace_id = ? AND org_id = ? AND id = ?")
        .bind(u64::MAX.to_be_bytes().as_slice()).bind(&scope.workspace_id).bind(&scope.org_id)
        .bind(delivery.id().into_bytes().as_slice()).execute(&pool).await.expect("delivery generation seeds");
    std::assert_matches!(
        store
            .claim_deliveries(ClaimResourceDeliveriesRequest::new(
                scope.clone(),
                ResourceLeaseHolder::new("fanout-next").expect("valid holder"),
                ResourceLeaseTtl::new(std::time::Duration::from_secs(1)).expect("valid TTL"),
                ResourcePageSize::new(1).expect("valid batch"),
            ))
            .await,
        Err(StorageError::Internal(_))
    );
    sqlx::query("UPDATE port_resource_source_leases SET generation = ?, expires_at_ms = 0 WHERE workspace_id = ? AND org_id = ? AND resource_id = ?")
        .bind(u64::MAX.to_be_bytes().as_slice()).bind(&scope.workspace_id).bind(&scope.org_id)
        .bind(first_id.into_bytes().as_slice()).execute(&pool).await.expect("generation seeds");
    std::assert_matches!(
        store
            .acquire(AcquireResourceSourceLeaseRequest::new(
                scope.clone(),
                first_id,
                ResourceLeaseHolder::new("source-next").expect("valid holder"),
                ResourceLeaseTtl::new(std::time::Duration::from_secs(1)).expect("valid TTL"),
            ))
            .await,
        Err(StorageError::Internal(_))
    );
    let generation: Vec<u8> = sqlx::query_scalar("SELECT generation FROM port_resource_source_leases WHERE workspace_id = ? AND org_id = ? AND resource_id = ?")
        .bind(&scope.workspace_id).bind(&scope.org_id).bind(first_id.into_bytes().as_slice())
        .fetch_one(&pool).await.expect("generation reads");
    assert_eq!(generation, u64::MAX.to_be_bytes());
    assert_eq!(source.token().generation().get(), 1);
}

fn test_scope() -> Scope {
    Scope::new("sqlite-resource-ws", "sqlite-resource-org")
}

fn test_identity() -> SharedResourceIdentity {
    SharedResourceIdentity::new(
        ResourceKind::new("test.resource").expect("valid kind"),
        ResourceCompatibilityVersion::new(1),
        ResourceConfigurationIdentity::try_from_vec(b"configuration".to_vec())
            .expect("valid configuration"),
        ResourceSlotIdentity::try_from_vec(b"slot".to_vec()).expect("valid slot"),
    )
}

async fn pending_delivery(
    pool: &SqlitePool,
    store: &SqliteResourceRuntime,
    occurrence: &[u8],
) -> (Scope, ResourceDeliveryId) {
    let scope = test_scope();
    let resource_id = match store
        .resolve(ResolveSharedResourceRequest::new(
            scope.clone(),
            test_identity(),
        ))
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
            ResourceConsumerKind::new("test-consumer").expect("valid kind"),
            ResourceConsumerIdentity::try_from_vec(occurrence.to_vec()).expect("valid identity"),
        ))
        .await
        .expect("subscription stores");
    let source = match store
        .acquire(AcquireResourceSourceLeaseRequest::new(
            scope.clone(),
            resource_id,
            ResourceLeaseHolder::new("source").expect("valid holder"),
            ResourceLeaseTtl::new(std::time::Duration::from_secs(30)).expect("valid TTL"),
        ))
        .await
        .expect("source acquires")
    {
        AcquireResourceSourceLeaseOutcome::Acquired(lease) => lease,
        AcquireResourceSourceLeaseOutcome::Contended { .. } => panic!("fresh source must acquire"),
    };
    store
        .accept(AcceptResourceEventRequest::new(
            scope.clone(),
            resource_id,
            source.token().clone(),
            EventOccurrenceNamespace::new("test.event").expect("valid namespace"),
            EventOccurrenceKey::try_from_vec(occurrence.to_vec()).expect("valid key"),
            EventEnvelope::try_from_vec(1, b"payload".to_vec()).expect("valid envelope"),
        ))
        .await
        .expect("event accepts");
    let id: Vec<u8> = sqlx::query_scalar("SELECT id FROM port_resource_deliveries WHERE workspace_id = ? AND org_id = ? ORDER BY sequence LIMIT 1").bind(&scope.workspace_id).bind(&scope.org_id).fetch_one(pool).await.expect("delivery id reads");
    (
        scope,
        ResourceDeliveryId::from_bytes(id.try_into().expect("16-byte id")),
    )
}

#[tokio::test]
async fn claim_failure_rolls_back_and_retries_cleanly() {
    let pool = pool().await;
    let store = SqliteResourceRuntime::new(pool.clone());
    let (scope, delivery_id) = pending_delivery(&pool, &store, b"claim-rollback").await;
    sqlx::query("CREATE TRIGGER resource_claim_abort BEFORE UPDATE OF claim_id ON port_resource_deliveries WHEN NEW.claim_id IS NOT NULL BEGIN SELECT RAISE(ABORT, 'injected claim failure'); END").execute(&pool).await.expect("trigger installs");
    let request = ClaimResourceDeliveriesRequest::new(
        scope.clone(),
        ResourceLeaseHolder::new("fanout").expect("valid holder"),
        ResourceLeaseTtl::new(std::time::Duration::from_secs(30)).expect("valid TTL"),
        ResourcePageSize::new(1).expect("valid batch"),
    );
    std::assert_matches!(
        store.claim_deliveries(request.clone()).await,
        Err(StorageError::Internal(_))
    );
    let claim_id: Option<Vec<u8>> =
        sqlx::query_scalar("SELECT claim_id FROM port_resource_deliveries WHERE id = ?")
            .bind(delivery_id.into_bytes().as_slice())
            .fetch_one(&pool)
            .await
            .expect("claim reads");
    assert_eq!(claim_id, None);
    sqlx::query("DROP TRIGGER resource_claim_abort")
        .execute(&pool)
        .await
        .expect("trigger removes");
    assert_eq!(
        store
            .claim_deliveries(request)
            .await
            .expect("retry claims")
            .len(),
        1
    );
}

#[tokio::test]
async fn handoff_failure_rolls_back_completion_and_retries_cleanly() {
    let pool = pool().await;
    let store = SqliteResourceRuntime::new(pool.clone());
    let (scope, _) = pending_delivery(&pool, &store, b"handoff-rollback").await;
    let delivery = store
        .claim_deliveries(ClaimResourceDeliveriesRequest::new(
            scope.clone(),
            ResourceLeaseHolder::new("fanout").expect("valid holder"),
            ResourceLeaseTtl::new(std::time::Duration::from_secs(30)).expect("valid TTL"),
            ResourcePageSize::new(1).expect("valid batch"),
        ))
        .await
        .expect("delivery claims")
        .pop()
        .expect("delivery exists");
    sqlx::query("CREATE TRIGGER resource_handoff_abort BEFORE INSERT ON port_resource_execution_handoffs BEGIN SELECT RAISE(ABORT, 'injected handoff failure'); END").execute(&pool).await.expect("trigger installs");
    let request = CompleteResourceDeliveryRequest::new(
        scope.clone(),
        delivery.id(),
        delivery.token().clone(),
        ResourceDeliveryCompletion::Delivered,
    );
    std::assert_matches!(
        store.complete_delivery(request.clone()).await,
        Err(StorageError::Internal(_))
    );
    let status: String =
        sqlx::query_scalar("SELECT status FROM port_resource_deliveries WHERE id = ?")
            .bind(delivery.id().into_bytes().as_slice())
            .fetch_one(&pool)
            .await
            .expect("status reads");
    let handoffs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM port_resource_execution_handoffs")
        .fetch_one(&pool)
        .await
        .expect("handoff count reads");
    assert_eq!((status.as_str(), handoffs), ("pending", 0));
    sqlx::query("DROP TRIGGER resource_handoff_abort")
        .execute(&pool)
        .await
        .expect("trigger removes");
    std::assert_matches!(
        store.complete_delivery(request).await,
        Ok(
            nebula_storage_port::dto::CompleteResourceDeliveryOutcome::Completed {
                completion: ResourceDeliveryCompletion::Delivered,
                event_became_terminal: true
            }
        )
    );
    let handoffs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM port_resource_execution_handoffs")
        .fetch_one(&pool)
        .await
        .expect("handoff count reads");
    assert_eq!(handoffs, 1);
}

#[tokio::test]
async fn delivery_insert_failure_rolls_back_event_and_retries_cleanly() {
    let pool = pool().await;
    let store = SqliteResourceRuntime::new(pool.clone());
    let scope = test_scope();
    let resource_id = match store
        .resolve(ResolveSharedResourceRequest::new(
            scope.clone(),
            test_identity(),
        ))
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
            ResourceConsumerKind::new("test-consumer").expect("valid consumer kind"),
            ResourceConsumerIdentity::try_from_vec(b"consumer".to_vec())
                .expect("valid consumer identity"),
        ))
        .await
        .expect("subscription stores");
    let source = match store
        .acquire(AcquireResourceSourceLeaseRequest::new(
            scope.clone(),
            resource_id,
            ResourceLeaseHolder::new("source").expect("valid holder"),
            ResourceLeaseTtl::new(std::time::Duration::from_secs(30)).expect("valid TTL"),
        ))
        .await
        .expect("source lease acquires")
    {
        AcquireResourceSourceLeaseOutcome::Acquired(lease) => lease,
        AcquireResourceSourceLeaseOutcome::Contended { .. } => {
            panic!("fresh source lease must acquire")
        },
    };
    let request = AcceptResourceEventRequest::new(
        scope.clone(),
        resource_id,
        source.token().clone(),
        EventOccurrenceNamespace::new("test.event").expect("valid namespace"),
        EventOccurrenceKey::try_from_vec(b"rollback-event".to_vec()).expect("valid key"),
        EventEnvelope::try_from_vec(1, b"payload".to_vec()).expect("valid envelope"),
    );

    sqlx::query("CREATE TRIGGER resource_delivery_abort BEFORE INSERT ON port_resource_deliveries BEGIN SELECT RAISE(ABORT, 'injected delivery failure'); END")
        .execute(&pool).await.expect("failure trigger installs");
    std::assert_matches!(
        store.accept(request.clone()).await,
        Err(StorageError::Internal(_))
    );
    let event_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM port_resource_events WHERE workspace_id = ? AND org_id = ?",
    )
    .bind(&scope.workspace_id)
    .bind(&scope.org_id)
    .fetch_one(&pool)
    .await
    .expect("event count reads");
    let delivery_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM port_resource_deliveries WHERE workspace_id = ? AND org_id = ?",
    )
    .bind(&scope.workspace_id)
    .bind(&scope.org_id)
    .fetch_one(&pool)
    .await
    .expect("delivery count reads");
    assert_eq!((event_count, delivery_count), (0, 0));

    sqlx::query("DROP TRIGGER resource_delivery_abort")
        .execute(&pool)
        .await
        .expect("failure trigger removes");
    std::assert_matches!(
        store.accept(request).await.expect("retry accepts"),
        AcceptResourceEventOutcome::Accepted {
            delivery_count: 1,
            ..
        }
    );
}

#[tokio::test]
async fn subscription_insert_maps_only_foreign_keys_to_not_found() {
    let pool = pool().await;
    let store = SqliteResourceRuntime::new(pool.clone());
    let scope = test_scope();
    let missing_request = PutResourceSubscriptionRequest::new(
        scope.clone(),
        SharedResourceId::from_bytes([91; 16]),
        ResourceConsumerKind::new("test-consumer").expect("valid consumer kind"),
        ResourceConsumerIdentity::try_from_vec(b"missing-parent".to_vec())
            .expect("valid consumer identity"),
    );
    std::assert_matches!(
        store.put(missing_request).await,
        Err(StorageError::NotFound { .. })
    );

    let resource_id = match store
        .resolve(ResolveSharedResourceRequest::new(
            scope.clone(),
            test_identity(),
        ))
        .await
        .expect("resource resolves")
    {
        ResolveSharedResourceOutcome::Created(record)
        | ResolveSharedResourceOutcome::Existing(record) => record.id(),
    };
    sqlx::query("CREATE TRIGGER resource_subscription_abort BEFORE INSERT ON port_resource_subscriptions BEGIN SELECT RAISE(ABORT, 'injected non-FK failure'); END")
        .execute(&pool)
        .await
        .expect("failure trigger installs");
    let unavailable_request = PutResourceSubscriptionRequest::new(
        scope,
        resource_id,
        ResourceConsumerKind::new("test-consumer").expect("valid consumer kind"),
        ResourceConsumerIdentity::try_from_vec(b"non-fk".to_vec())
            .expect("valid consumer identity"),
    );
    std::assert_matches!(
        store.put(unavailable_request).await,
        Err(StorageError::Internal(_))
    );
}

#[tokio::test]
async fn migration_enforces_resource_bounds_states_and_tenant_foreign_keys() {
    let pool = pool().await;
    let resource_id = [1_u8; 16];
    let digest = [2_u8; 32];
    sqlx::query("INSERT INTO port_shared_resources (id, workspace_id, org_id, kind, compatibility_version, configuration_identity, slot_identity, identity_digest) VALUES (?, 'ws', 'org', ?, 1, ?, ?, ?)")
        .bind(resource_id.as_slice()).bind("k".repeat(128)).bind(vec![3_u8; 65_536])
        .bind(vec![4_u8; 65_536]).bind(digest.as_slice()).execute(&pool).await
        .expect("maximum resource bounds are accepted");

    for invalid_insert in [
        sqlx::query("INSERT INTO port_shared_resources (id, workspace_id, org_id, kind, compatibility_version, configuration_identity, slot_identity, identity_digest) VALUES (?, 'ws', 'org', '', 1, X'01', X'', ?)")
            .bind([5_u8; 16].as_slice()).bind(digest.as_slice()).execute(&pool).await,
        sqlx::query("INSERT INTO port_shared_resources (id, workspace_id, org_id, kind, compatibility_version, configuration_identity, slot_identity, identity_digest) VALUES (?, 'ws', 'org', 'kind', 1, X'', X'', ?)")
            .bind([6_u8; 16].as_slice()).bind(digest.as_slice()).execute(&pool).await,
    ] {
        assert!(invalid_insert.is_err());
    }

    let missing_resource = [9_u8; 16];
    let invalid_state = sqlx::query("INSERT INTO port_resource_subscriptions (id, workspace_id, org_id, resource_id, consumer_kind, consumer_identity, state, version) VALUES (?, 'ws', 'org', ?, 'consumer', X'01', 'unknown', ?)")
        .bind([7_u8; 16].as_slice()).bind(resource_id.as_slice()).bind(1_u64.to_be_bytes().as_slice())
        .execute(&pool).await;
    let foreign_scope = sqlx::query("INSERT INTO port_resource_subscriptions (id, workspace_id, org_id, resource_id, consumer_kind, consumer_identity, state, version) VALUES (?, 'other-ws', 'org', ?, 'consumer', X'01', 'active', ?)")
        .bind([8_u8; 16].as_slice()).bind(resource_id.as_slice()).bind(1_u64.to_be_bytes().as_slice())
        .execute(&pool).await;
    let missing_parent = sqlx::query("INSERT INTO port_resource_subscriptions (id, workspace_id, org_id, resource_id, consumer_kind, consumer_identity, state, version) VALUES (?, 'ws', 'org', ?, 'consumer', X'01', 'active', ?)")
        .bind([10_u8; 16].as_slice()).bind(missing_resource.as_slice()).bind(1_u64.to_be_bytes().as_slice())
        .execute(&pool).await;
    assert!(invalid_state.is_err());
    assert!(foreign_scope.is_err());
    assert!(missing_parent.is_err());

    let second_resource = [11_u8; 16];
    sqlx::query("INSERT INTO port_shared_resources (id, workspace_id, org_id, kind, compatibility_version, configuration_identity, slot_identity, identity_digest) VALUES (?, 'ws', 'org', 'other', 1, X'02', X'', ?)")
        .bind(second_resource.as_slice()).bind([12_u8; 32].as_slice()).execute(&pool).await.expect("second resource inserts");
    sqlx::query("INSERT INTO port_resource_subscriptions (id, workspace_id, org_id, resource_id, consumer_kind, consumer_identity, state, version) VALUES (?, 'ws', 'org', ?, 'consumer', X'02', 'active', ?)")
        .bind([13_u8; 16].as_slice()).bind(second_resource.as_slice()).bind(1_u64.to_be_bytes().as_slice()).execute(&pool).await.expect("second subscription inserts");
    sqlx::query("INSERT INTO port_resource_events (id, workspace_id, org_id, resource_id, occurrence_namespace, occurrence_key, occurrence_digest, schema_version, canonical_payload, envelope_digest, accepted_at_ms, source_generation, state) VALUES (?, 'ws', 'org', ?, 'event', X'01', ?, 1, X'01', ?, 1, ?, 'pending')")
        .bind([14_u8; 16].as_slice()).bind(resource_id.as_slice()).bind([15_u8; 32].as_slice()).bind([16_u8; 32].as_slice()).bind(1_u64.to_be_bytes().as_slice()).execute(&pool).await.expect("event inserts");
    let cross_resource_delivery = sqlx::query("INSERT INTO port_resource_deliveries (id, workspace_id, org_id, resource_id, event_id, subscription_id, status, claim_generation) VALUES (?, 'ws', 'org', ?, ?, ?, 'pending', ?)")
        .bind([17_u8; 16].as_slice()).bind(resource_id.as_slice()).bind([14_u8; 16].as_slice()).bind([13_u8; 16].as_slice()).bind(0_u64.to_be_bytes().as_slice()).execute(&pool).await;
    assert!(cross_resource_delivery.is_err());
    let claim_index: String = sqlx::query_scalar("SELECT sql FROM sqlite_master WHERE type = 'index' AND name = 'port_resource_deliveries_claimable'").fetch_one(&pool).await.expect("claim index reads");
    assert!(claim_index.contains("workspace_id, org_id, sequence, claim_expires_at_ms"));
    let pending_index: String = sqlx::query_scalar("SELECT sql FROM sqlite_master WHERE type = 'index' AND name = 'port_resource_deliveries_event_pending'").fetch_one(&pool).await.expect("event pending index reads");
    assert!(pending_index.contains("workspace_id, org_id, event_id"));
    assert!(pending_index.contains("WHERE status = 'pending'"));
}
