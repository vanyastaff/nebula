//! Shared-resource fanout conformance for the PostgreSQL adapter.

#![cfg(feature = "postgres")]

#[macro_use]
#[path = "support/resource_fanout_oracle.rs"]
mod oracle;

use std::str::FromStr as _;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use nebula_storage::postgres::{PgResourceRuntime, init_schema};
use nebula_storage_port::dto::{
    AcceptResourceEventOutcome, AcceptResourceEventRequest, AcquireResourceSourceLeaseOutcome,
    AcquireResourceSourceLeaseRequest, ClaimResourceDeliveriesRequest,
    CompleteResourceDeliveryOutcome, CompleteResourceDeliveryRequest, EventEnvelope,
    EventOccurrenceKey, EventOccurrenceNamespace, HeartbeatResourceSourceLeaseRequest,
    PutResourceSubscriptionRequest, ResolveSharedResourceOutcome, ResolveSharedResourceRequest,
    ResourceCompatibilityVersion, ResourceConfigurationIdentity, ResourceConsumerIdentity,
    ResourceConsumerKind, ResourceDeliveryCompletion, ResourceDeliveryId, ResourceEventState,
    ResourceKind, ResourceLeaseHolder, ResourceLeaseTtl, ResourcePageSize, ResourceSlotIdentity,
    SharedResourceId, SharedResourceIdentity,
};
use nebula_storage_port::store::{
    ResourceEventFanoutStore, ResourceSourceLeaseStore, ResourceSubscriptionStore,
    SharedResourceStore,
};
use nebula_storage_port::{Scope, StorageError};
use sqlx::PgPool;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};

struct PgExpiryControl {
    pool: PgPool,
}

#[async_trait::async_trait]
impl oracle::ResourceFanoutExpiryControl for PgExpiryControl {
    async fn expire_source_for_test(&self, scope: &Scope, resource_id: SharedResourceId) {
        sqlx::query("UPDATE port_resource_source_leases SET expires_at_ms = (EXTRACT(EPOCH FROM clock_timestamp()) * 1000)::BIGINT - 1 WHERE workspace_id = $1 AND org_id = $2 AND resource_id = $3")
            .bind(&scope.workspace_id).bind(&scope.org_id).bind(resource_id.into_bytes().as_slice())
            .execute(&self.pool)
            .await
            .expect("source backdate succeeds");
    }
    async fn expire_delivery_for_test(&self, scope: &Scope, delivery_id: ResourceDeliveryId) {
        sqlx::query("UPDATE port_resource_deliveries SET claim_expires_at_ms = (EXTRACT(EPOCH FROM clock_timestamp()) * 1000)::BIGINT - 1 WHERE workspace_id = $1 AND org_id = $2 AND id = $3")
            .bind(&scope.workspace_id).bind(&scope.org_id).bind(delivery_id.into_bytes().as_slice())
            .execute(&self.pool)
            .await
            .expect("delivery backdate succeeds");
    }
}

fn unique_schema() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("nebula_resource_runtime_{}_{nanos}", std::process::id())
}

async fn runtime() -> Option<(PgResourceRuntime, PgExpiryControl)> {
    let url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(std::env::VarError::NotPresent) => {
            assert_ne!(
                std::env::var("NEBULA_REQUIRE_POSTGRES").as_deref(),
                Ok("1"),
                "DATABASE_URL must be set when NEBULA_REQUIRE_POSTGRES=1"
            );
            return None;
        },
        Err(error) => panic!("DATABASE_URL is set but invalid: {error}"),
    };
    let admin = PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .expect("connect to DATABASE_URL");
    let schema = unique_schema();
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
        .execute(&admin)
        .await
        .expect("create isolated schema");
    let options = PgConnectOptions::from_str(&url)
        .expect("parse DATABASE_URL")
        .options([("search_path", schema.as_str())]);
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .connect_with(options)
        .await
        .expect("connect to isolated schema");
    init_schema(&pool)
        .await
        .expect("initialize isolated schema");
    Some((
        PgResourceRuntime::new(pool.clone()),
        PgExpiryControl { pool },
    ))
}

optional_resource_fanout_conformance_suite!(runtime());

#[tokio::test]
async fn digest_collision_and_generation_overflow_use_test_owned_pool() {
    let Some((admin, pool, schema)) = isolated_pool().await else {
        return;
    };
    let store = PgResourceRuntime::new(pool.clone());
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
    sqlx::query("UPDATE port_shared_resources SET identity_digest = $1 WHERE workspace_id = $2 AND org_id = $3 AND id = $4")
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
    sqlx::query("UPDATE port_resource_deliveries SET claim_generation = $1, claim_expires_at_ms = 0 WHERE workspace_id = $2 AND org_id = $3 AND id = $4")
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
    sqlx::query("UPDATE port_resource_source_leases SET generation = $1, expires_at_ms = 0 WHERE workspace_id = $2 AND org_id = $3 AND resource_id = $4")
        .bind(u64::MAX.to_be_bytes().as_slice()).bind(&scope.workspace_id).bind(&scope.org_id)
        .bind(first_id.into_bytes().as_slice()).execute(&pool).await.expect("source generation seeds");
    std::assert_matches!(
        store
            .acquire(AcquireResourceSourceLeaseRequest::new(
                scope,
                first_id,
                ResourceLeaseHolder::new("source-next").expect("valid holder"),
                ResourceLeaseTtl::new(std::time::Duration::from_secs(1)).expect("valid TTL"),
            ))
            .await,
        Err(StorageError::Internal(_))
    );
    cleanup(admin, pool, schema).await;
}

async fn isolated_pool() -> Option<(PgPool, PgPool, String)> {
    let url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(std::env::VarError::NotPresent) => {
            assert_ne!(
                std::env::var("NEBULA_REQUIRE_POSTGRES").as_deref(),
                Ok("1"),
                "DATABASE_URL must be set when NEBULA_REQUIRE_POSTGRES=1"
            );
            return None;
        },
        Err(error) => panic!("DATABASE_URL is set but invalid: {error}"),
    };
    let admin = PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .expect("connect to DATABASE_URL");
    let schema = unique_schema();
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
        .execute(&admin)
        .await
        .expect("create isolated schema");
    let options = PgConnectOptions::from_str(&url)
        .expect("parse DATABASE_URL")
        .options([("search_path", schema.as_str())]);
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .connect_with(options)
        .await
        .expect("connect to isolated schema");
    init_schema(&pool)
        .await
        .expect("initialize isolated schema");
    Some((admin, pool, schema))
}

async fn cleanup(admin: PgPool, pool: PgPool, schema: String) {
    pool.close().await;
    sqlx::query(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
        .execute(&admin)
        .await
        .expect("drop isolated schema");
    admin.close().await;
}

fn test_scope() -> Scope {
    Scope::new("pg-resource-ws", "pg-resource-org")
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

fn pseudorandom_bytes(byte_count: usize, mut state: u64) -> Vec<u8> {
    std::iter::repeat_with(|| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state.to_le_bytes()[0]
    })
    .take(byte_count)
    .collect()
}

async fn pending_delivery(
    pool: &PgPool,
    store: &PgResourceRuntime,
    occurrence: &[u8],
) -> (Scope, ResourceDeliveryId) {
    let scope = test_scope();
    let resource_id = resolve_test_resource(store, &scope).await;
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
    let id: Vec<u8> = sqlx::query_scalar("SELECT id FROM port_resource_deliveries WHERE workspace_id = $1 AND org_id = $2 ORDER BY sequence LIMIT 1").bind(&scope.workspace_id).bind(&scope.org_id).fetch_one(pool).await.expect("delivery id reads");
    (
        scope,
        ResourceDeliveryId::from_bytes(id.try_into().expect("16-byte id")),
    )
}

#[tokio::test]
async fn claim_failure_rolls_back_and_retries_cleanly() {
    let Some((admin, pool, schema)) = isolated_pool().await else {
        return;
    };
    let store = PgResourceRuntime::new(pool.clone());
    let (scope, delivery_id) = pending_delivery(&pool, &store, b"claim-rollback").await;
    sqlx::query("CREATE FUNCTION abort_resource_claim() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'injected claim failure'; END $$").execute(&pool).await.expect("function installs");
    sqlx::query("CREATE TRIGGER resource_claim_abort BEFORE UPDATE OF claim_id ON port_resource_deliveries FOR EACH ROW WHEN (NEW.claim_id IS NOT NULL) EXECUTE FUNCTION abort_resource_claim()").execute(&pool).await.expect("trigger installs");
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
        sqlx::query_scalar("SELECT claim_id FROM port_resource_deliveries WHERE id = $1")
            .bind(delivery_id.into_bytes().as_slice())
            .fetch_one(&pool)
            .await
            .expect("claim reads");
    assert_eq!(claim_id, None);
    sqlx::query("DROP TRIGGER resource_claim_abort ON port_resource_deliveries")
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
    cleanup(admin, pool, schema).await;
}

#[tokio::test]
async fn handoff_failure_rolls_back_completion_and_retries_cleanly() {
    let Some((admin, pool, schema)) = isolated_pool().await else {
        return;
    };
    let store = PgResourceRuntime::new(pool.clone());
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
    sqlx::query("CREATE FUNCTION abort_resource_handoff() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'injected handoff failure'; END $$").execute(&pool).await.expect("function installs");
    sqlx::query("CREATE TRIGGER resource_handoff_abort BEFORE INSERT ON port_resource_execution_handoffs FOR EACH ROW EXECUTE FUNCTION abort_resource_handoff()").execute(&pool).await.expect("trigger installs");
    let request = CompleteResourceDeliveryRequest::new(
        scope,
        delivery.id(),
        delivery.token().clone(),
        ResourceDeliveryCompletion::Delivered,
    );
    std::assert_matches!(
        store.complete_delivery(request.clone()).await,
        Err(StorageError::Internal(_))
    );
    let status: String =
        sqlx::query_scalar("SELECT status FROM port_resource_deliveries WHERE id = $1")
            .bind(delivery.id().into_bytes().as_slice())
            .fetch_one(&pool)
            .await
            .expect("status reads");
    let handoffs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM port_resource_execution_handoffs")
        .fetch_one(&pool)
        .await
        .expect("handoff count reads");
    assert_eq!((status.as_str(), handoffs), ("pending", 0));
    sqlx::query("DROP TRIGGER resource_handoff_abort ON port_resource_execution_handoffs")
        .execute(&pool)
        .await
        .expect("trigger removes");
    std::assert_matches!(
        store.complete_delivery(request).await,
        Ok(CompleteResourceDeliveryOutcome::Completed {
            completion: ResourceDeliveryCompletion::Delivered,
            event_became_terminal: true
        })
    );
    let handoffs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM port_resource_execution_handoffs")
        .fetch_one(&pool)
        .await
        .expect("handoff count reads");
    assert_eq!(handoffs, 1);
    cleanup(admin, pool, schema).await;
}

async fn wait_for_blocked_backends(pool: &PgPool, blocker_pid: i32, expected: i64) {
    for _ in 0..10_000 {
        let blocked: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM pg_stat_activity WHERE $1 = ANY(pg_blocking_pids(pid))",
        )
        .bind(blocker_pid)
        .fetch_one(pool)
        .await
        .expect("blocked backend count reads");
        if blocked >= expected {
            return;
        }
        tokio::task::yield_now().await;
    }
    panic!("expected {expected} backend operations to block on the fixture transaction");
}

async fn resolve_test_resource(store: &PgResourceRuntime, scope: &Scope) -> SharedResourceId {
    match store
        .resolve(ResolveSharedResourceRequest::new(
            scope.clone(),
            test_identity(),
        ))
        .await
        .expect("resource resolves")
    {
        ResolveSharedResourceOutcome::Created(record)
        | ResolveSharedResourceOutcome::Existing(record) => record.id(),
    }
}

#[tokio::test]
async fn source_heartbeat_waiting_past_expiry_is_fenced() {
    let Some((admin, pool, schema)) = isolated_pool().await else {
        return;
    };
    let store = PgResourceRuntime::new(pool.clone());
    let scope = test_scope();
    let resource_id = resolve_test_resource(&store, &scope).await;
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
        AcquireResourceSourceLeaseOutcome::Contended { .. } => panic!("fresh lease must acquire"),
    };

    let mut blocker = pool.begin().await.expect("blocker transaction begins");
    let blocker_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *blocker)
        .await
        .expect("blocker pid reads");
    sqlx::query("SELECT resource_id FROM port_resource_source_leases WHERE workspace_id = $1 AND org_id = $2 AND resource_id = $3 FOR UPDATE")
        .bind(&scope.workspace_id).bind(&scope.org_id).bind(resource_id.into_bytes().as_slice())
        .fetch_one(&mut *blocker).await.expect("source row locks");

    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let task_store = store.clone();
    let task_scope = scope.clone();
    let task_token = source.token().clone();
    let task_barrier = Arc::clone(&barrier);
    let heartbeat = tokio::spawn(async move {
        task_barrier.wait().await;
        task_store
            .heartbeat(HeartbeatResourceSourceLeaseRequest::new(
                task_scope,
                resource_id,
                task_token,
                ResourceLeaseTtl::new(std::time::Duration::from_secs(30)).expect("valid TTL"),
            ))
            .await
    });
    barrier.wait().await;
    wait_for_blocked_backends(&pool, blocker_pid, 1).await;
    sqlx::query("UPDATE port_resource_source_leases SET expires_at_ms = (EXTRACT(EPOCH FROM clock_timestamp()) * 1000)::BIGINT - 1 WHERE workspace_id = $1 AND org_id = $2 AND resource_id = $3")
        .bind(&scope.workspace_id).bind(&scope.org_id).bind(resource_id.into_bytes().as_slice())
        .execute(&mut *blocker).await.expect("source expires while heartbeat waits");
    blocker.commit().await.expect("blocker commits");
    std::assert_matches!(
        heartbeat.await.expect("heartbeat task joins"),
        Err(StorageError::FencedOut { .. })
    );
    cleanup(admin, pool, schema).await;
}

#[tokio::test]
async fn sibling_completions_wait_on_parent_and_terminalize_it() {
    let Some((admin, pool, schema)) = isolated_pool().await else {
        return;
    };
    let store = PgResourceRuntime::new(pool.clone());
    let scope = test_scope();
    let resource_id = resolve_test_resource(&store, &scope).await;
    for identity in [b"consumer-a".as_slice(), b"consumer-b".as_slice()] {
        store
            .put(PutResourceSubscriptionRequest::new(
                scope.clone(),
                resource_id,
                ResourceConsumerKind::new("test-consumer").expect("valid kind"),
                ResourceConsumerIdentity::try_from_vec(identity.to_vec()).expect("valid identity"),
            ))
            .await
            .expect("subscription stores");
    }
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
        AcquireResourceSourceLeaseOutcome::Contended { .. } => panic!("fresh lease must acquire"),
    };
    let event_id = match store
        .accept(AcceptResourceEventRequest::new(
            scope.clone(),
            resource_id,
            source.token().clone(),
            EventOccurrenceNamespace::new("test.event").expect("valid namespace"),
            EventOccurrenceKey::try_from_vec(b"sibling-lock".to_vec()).expect("valid key"),
            EventEnvelope::try_from_vec(1, b"payload".to_vec()).expect("valid envelope"),
        ))
        .await
        .expect("event accepts")
    {
        AcceptResourceEventOutcome::Accepted { event_id, .. } => event_id,
        other => panic!("fresh event must accept, got {other:?}"),
    };
    let deliveries = store
        .claim_deliveries(ClaimResourceDeliveriesRequest::new(
            scope.clone(),
            ResourceLeaseHolder::new("fanout").expect("valid holder"),
            ResourceLeaseTtl::new(std::time::Duration::from_secs(30)).expect("valid TTL"),
            ResourcePageSize::new(2).expect("valid batch"),
        ))
        .await
        .expect("deliveries claim");
    assert_eq!(deliveries.len(), 2);

    let mut blocker = pool.begin().await.expect("blocker transaction begins");
    let blocker_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *blocker)
        .await
        .expect("blocker pid reads");
    sqlx::query("SELECT id FROM port_resource_events WHERE workspace_id = $1 AND org_id = $2 AND id = $3 FOR UPDATE")
        .bind(&scope.workspace_id).bind(&scope.org_id).bind(event_id.into_bytes().as_slice())
        .fetch_one(&mut *blocker).await.expect("parent event locks");
    let barrier = Arc::new(tokio::sync::Barrier::new(3));
    let mut tasks = Vec::with_capacity(2);
    for delivery in deliveries {
        let task_store = store.clone();
        let task_scope = scope.clone();
        let task_barrier = Arc::clone(&barrier);
        tasks.push(tokio::spawn(async move {
            task_barrier.wait().await;
            task_store
                .complete_delivery(CompleteResourceDeliveryRequest::new(
                    task_scope,
                    delivery.id(),
                    delivery.token().clone(),
                    ResourceDeliveryCompletion::Delivered,
                ))
                .await
        }));
    }
    barrier.wait().await;
    // PostgreSQL may queue the second waiter behind the first tuple-lock waiter;
    // observing one blocked completion proves the parent row is the serialization point.
    wait_for_blocked_backends(&pool, blocker_pid, 1).await;
    blocker.commit().await.expect("parent lock releases");
    let mut terminalized = 0;
    for task in tasks {
        if task
            .await
            .expect("completion task joins")
            .expect("completion succeeds")
            == (CompleteResourceDeliveryOutcome::Completed {
                completion: ResourceDeliveryCompletion::Delivered,
                event_became_terminal: true,
            })
        {
            terminalized += 1;
        }
    }
    assert_eq!(terminalized, 1);
    assert_eq!(
        store
            .get_event(&scope, event_id)
            .await
            .expect("event reads")
            .expect("event exists")
            .state(),
        ResourceEventState::Complete
    );
    cleanup(admin, pool, schema).await;
}

#[tokio::test]
async fn delivery_insert_failure_rolls_back_event_and_retries_cleanly() {
    let Some((admin, pool, schema)) = isolated_pool().await else {
        return;
    };
    let store = PgResourceRuntime::new(pool.clone());
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
            ResourceConsumerIdentity::try_from_vec(b"consumer".to_vec()).expect("valid identity"),
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
        AcquireResourceSourceLeaseOutcome::Contended { .. } => panic!("fresh lease must acquire"),
    };
    let request = AcceptResourceEventRequest::new(
        scope.clone(),
        resource_id,
        source.token().clone(),
        EventOccurrenceNamespace::new("test.event").expect("valid namespace"),
        EventOccurrenceKey::try_from_vec(b"rollback-event".to_vec()).expect("valid key"),
        EventEnvelope::try_from_vec(1, b"payload".to_vec()).expect("valid envelope"),
    );
    sqlx::query("CREATE FUNCTION abort_resource_delivery() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'injected delivery failure'; END $$").execute(&pool).await.expect("function installs");
    sqlx::query("CREATE TRIGGER resource_delivery_abort BEFORE INSERT ON port_resource_deliveries FOR EACH ROW EXECUTE FUNCTION abort_resource_delivery()").execute(&pool).await.expect("trigger installs");
    std::assert_matches!(
        store.accept(request.clone()).await,
        Err(StorageError::Internal(_))
    );
    let events: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM port_resource_events WHERE workspace_id = $1 AND org_id = $2",
    )
    .bind(&scope.workspace_id)
    .bind(&scope.org_id)
    .fetch_one(&pool)
    .await
    .expect("event count");
    let deliveries: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM port_resource_deliveries WHERE workspace_id = $1 AND org_id = $2",
    )
    .bind(&scope.workspace_id)
    .bind(&scope.org_id)
    .fetch_one(&pool)
    .await
    .expect("delivery count");
    assert_eq!((events, deliveries), (0, 0));
    sqlx::query("DROP TRIGGER resource_delivery_abort ON port_resource_deliveries")
        .execute(&pool)
        .await
        .expect("trigger removes");
    std::assert_matches!(
        store.accept(request).await.expect("retry accepts"),
        AcceptResourceEventOutcome::Accepted {
            delivery_count: 1,
            ..
        }
    );
    cleanup(admin, pool, schema).await;
}

#[tokio::test]
async fn subscription_insert_maps_only_foreign_keys_to_not_found() {
    let Some((admin, pool, schema)) = isolated_pool().await else {
        return;
    };
    let store = PgResourceRuntime::new(pool.clone());
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
    sqlx::query("CREATE FUNCTION abort_resource_subscription() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'injected non-FK failure'; END $$")
        .execute(&pool).await.expect("failure function installs");
    sqlx::query("CREATE TRIGGER resource_subscription_abort BEFORE INSERT ON port_resource_subscriptions FOR EACH ROW EXECUTE FUNCTION abort_resource_subscription()")
        .execute(&pool).await.expect("failure trigger installs");
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
    cleanup(admin, pool, schema).await;
}

#[tokio::test]
async fn migration_enforces_bounds_states_and_tenant_foreign_keys() {
    let Some((admin, pool, schema)) = isolated_pool().await else {
        return;
    };
    let resource_id = [1_u8; 16];
    let digest = [2_u8; 32];
    sqlx::query("INSERT INTO port_shared_resources (id, workspace_id, org_id, kind, compatibility_version, configuration_identity, slot_identity, identity_digest) VALUES ($1, 'ws', 'org', $2, 1, $3, $4, $5)")
        .bind(resource_id.as_slice()).bind("k".repeat(128)).bind(vec![3_u8; 65_536]).bind(vec![4_u8; 65_536]).bind(digest.as_slice())
        .execute(&pool).await.expect("maximum bounds accepted");
    let empty_kind = sqlx::query("INSERT INTO port_shared_resources (id, workspace_id, org_id, kind, compatibility_version, configuration_identity, slot_identity, identity_digest) VALUES ($1, 'ws', 'org', '', 1, '\\x01', '\\x', $2)")
        .bind([5_u8; 16].as_slice()).bind(digest.as_slice()).execute(&pool).await;
    let invalid_state = sqlx::query("INSERT INTO port_resource_subscriptions (id, workspace_id, org_id, resource_id, consumer_kind, consumer_identity, state, version) VALUES ($1, 'ws', 'org', $2, 'consumer', '\\x01', 'unknown', $3)")
        .bind([6_u8; 16].as_slice()).bind(resource_id.as_slice()).bind(1_u64.to_be_bytes().as_slice()).execute(&pool).await;
    let foreign_scope = sqlx::query("INSERT INTO port_resource_subscriptions (id, workspace_id, org_id, resource_id, consumer_kind, consumer_identity, state, version) VALUES ($1, 'other', 'org', $2, 'consumer', '\\x01', 'active', $3)")
        .bind([7_u8; 16].as_slice()).bind(resource_id.as_slice()).bind(1_u64.to_be_bytes().as_slice()).execute(&pool).await;
    assert!(empty_kind.is_err());
    assert!(invalid_state.is_err());
    assert!(foreign_scope.is_err());
    let second_resource = [11_u8; 16];
    sqlx::query("INSERT INTO port_shared_resources (id, workspace_id, org_id, kind, compatibility_version, configuration_identity, slot_identity, identity_digest) VALUES ($1, 'ws', 'org', 'other', 1, '\\x02', '\\x', $2)")
        .bind(second_resource.as_slice()).bind([12_u8; 32].as_slice()).execute(&pool).await.expect("second resource inserts");
    sqlx::query("INSERT INTO port_resource_subscriptions (id, workspace_id, org_id, resource_id, consumer_kind, consumer_identity, state, version) VALUES ($1, 'ws', 'org', $2, 'consumer', '\\x02', 'active', $3)")
        .bind([13_u8; 16].as_slice()).bind(second_resource.as_slice()).bind(1_u64.to_be_bytes().as_slice()).execute(&pool).await.expect("second subscription inserts");
    sqlx::query("INSERT INTO port_resource_events (id, workspace_id, org_id, resource_id, occurrence_namespace, occurrence_key, occurrence_digest, schema_version, canonical_payload, envelope_digest, accepted_at_ms, source_generation, state) VALUES ($1, 'ws', 'org', $2, 'event', '\\x01', $3, 1, '\\x01', $4, 1, $5, 'pending')")
        .bind([14_u8; 16].as_slice()).bind(resource_id.as_slice()).bind([15_u8; 32].as_slice()).bind([16_u8; 32].as_slice()).bind(1_u64.to_be_bytes().as_slice()).execute(&pool).await.expect("event inserts");
    let cross_resource_delivery = sqlx::query("INSERT INTO port_resource_deliveries (id, workspace_id, org_id, resource_id, event_id, subscription_id, status, claim_generation) VALUES ($1, 'ws', 'org', $2, $3, $4, 'pending', $5)")
        .bind([17_u8; 16].as_slice()).bind(resource_id.as_slice()).bind([14_u8; 16].as_slice()).bind([13_u8; 16].as_slice()).bind(0_u64.to_be_bytes().as_slice()).execute(&pool).await;
    assert!(cross_resource_delivery.is_err());
    let index_definition: String = sqlx::query_scalar("SELECT indexdef FROM pg_indexes WHERE schemaname = current_schema() AND indexname = 'port_resource_deliveries_claimable'").fetch_one(&pool).await.expect("claim index reads");
    assert!(index_definition.contains("workspace_id, org_id, sequence, claim_expires_at_ms"));
    let pending_index: String = sqlx::query_scalar("SELECT indexdef FROM pg_indexes WHERE schemaname = current_schema() AND indexname = 'port_resource_deliveries_event_pending'").fetch_one(&pool).await.expect("event pending index reads");
    assert!(pending_index.contains("workspace_id, org_id, event_id"));
    assert!(pending_index.contains("WHERE (status = 'pending'::text)"));
    let index_definitions: Vec<String> =
        sqlx::query_scalar("SELECT indexdef FROM pg_indexes WHERE schemaname = current_schema()")
            .fetch_all(&pool)
            .await
            .expect("index definitions read");
    assert!(
        index_definitions
            .iter()
            .all(|definition| !definition.contains("configuration_identity"))
    );
    assert!(
        index_definitions
            .iter()
            .all(|definition| !definition.contains("occurrence_key"))
    );
    cleanup(admin, pool, schema).await;
}

#[tokio::test]
async fn incompressible_max_identity_resolves_once_and_accepts_max_occurrence() {
    let Some((admin, pool, schema)) = isolated_pool().await else {
        return;
    };
    let store = PgResourceRuntime::new(pool.clone());
    let scope = test_scope();
    let identity = SharedResourceIdentity::new(
        ResourceKind::new("k".repeat(128)).expect("maximum kind"),
        ResourceCompatibilityVersion::new(u32::MAX),
        ResourceConfigurationIdentity::try_from_vec(pseudorandom_bytes(65_536, 0x1234_5678))
            .expect("maximum configuration"),
        ResourceSlotIdentity::try_from_vec(pseudorandom_bytes(65_536, 0x8765_4321))
            .expect("maximum slot"),
    );
    let barrier = Arc::new(tokio::sync::Barrier::new(8));
    let mut tasks = Vec::with_capacity(8);
    for _ in 0..8 {
        let task_store = store.clone();
        let task_scope = scope.clone();
        let task_identity = identity.clone();
        let task_barrier = barrier.clone();
        tasks.push(tokio::spawn(async move {
            task_barrier.wait().await;
            task_store
                .resolve(ResolveSharedResourceRequest::new(task_scope, task_identity))
                .await
                .expect("maximum identity resolves")
        }));
    }
    let mut created = 0;
    let mut resource_id = None;
    for task in tasks {
        let outcome = task.await.expect("resolve task joins");
        if let ResolveSharedResourceOutcome::Created(record) = &outcome {
            created += 1;
            resource_id = Some(record.id());
        } else if let ResolveSharedResourceOutcome::Existing(record) = &outcome {
            resource_id = Some(record.id());
        }
    }
    assert_eq!(created, 1);
    let resource_id = resource_id.expect("resolve returns resource id");
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
        AcquireResourceSourceLeaseOutcome::Contended { .. } => panic!("fresh source acquires"),
    };
    std::assert_matches!(
        store
            .accept(AcceptResourceEventRequest::new(
                scope,
                resource_id,
                source.token().clone(),
                EventOccurrenceNamespace::new("n".repeat(128)).expect("maximum namespace"),
                EventOccurrenceKey::try_from_vec(pseudorandom_bytes(1_024, 0xa5a5_5a5a))
                    .expect("maximum occurrence key"),
                EventEnvelope::try_from_vec(1, b"payload".to_vec()).expect("valid envelope"),
            ))
            .await,
        Ok(AcceptResourceEventOutcome::Accepted {
            delivery_count: 0,
            ..
        })
    );
    cleanup(admin, pool, schema).await;
}
