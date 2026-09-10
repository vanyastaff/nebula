//! PostgreSQL persistence for the shared-resource runtime aggregate.

use std::str::FromStr as _;

use chrono::{DateTime, Utc};
use nebula_storage_port::dto::{
    AcceptResourceEventOutcome, AcceptResourceEventRequest, AcknowledgeResourceHandoffOutcome,
    AcquireResourceSourceLeaseOutcome, AcquireResourceSourceLeaseRequest,
    ClaimResourceDeliveriesRequest, ClaimResourceHandoffsRequest, ClaimResourceRuntimeWorkRequest,
    ClaimedResourceDelivery, ClaimedResourceHandoff, CompleteResourceDeliveryOutcome,
    CompleteResourceDeliveryRequest, EventEnvelope, EventOccurrenceKey, EventOccurrenceNamespace,
    HeartbeatResourceDeliveryRequest, HeartbeatResourceHandoffRequest,
    HeartbeatResourceSourceLeaseRequest, PutResourceSubscriptionOutcome,
    PutResourceSubscriptionRequest, ReconciliationCursor, ReleaseResourceDeliveryRequest,
    ReleaseResourceSourceLeaseRequest, ResolveSharedResourceOutcome, ResolveSharedResourceRequest,
    ResourceCompatibilityVersion, ResourceConfigurationIdentity, ResourceConsumerIdentity,
    ResourceConsumerKind, ResourceDeliveryClaimToken, ResourceDeliveryCompletion,
    ResourceDeliveryId, ResourceEventAcceptance, ResourceEventId, ResourceEventRecord,
    ResourceEventState, ResourceHandoffClaimRequest, ResourceHandoffClaimToken, ResourceKind,
    ResourceLeaseGeneration, ResourceLeaseHolder, ResourceLeaseTtl, ResourcePageSize,
    ResourceSlotIdentity, ResourceSourceLease, ResourceSourceLeaseToken, ResourceSubscriptionId,
    ResourceSubscriptionPage, ResourceSubscriptionRecord, ResourceSubscriptionState,
    ResourceSubscriptionVersion, ScopedClaimedResourceDelivery, ScopedClaimedResourceHandoff,
    SharedResourceId, SharedResourceIdentity, SharedResourcePage, SharedResourceRecord,
    TerminalDeliveryIneligibility, TransitionResourceSubscriptionRequest,
};
use nebula_storage_port::store::{
    ResourceEventFanoutStore, ResourceExecutionHandoffStore, ResourceRuntimeRecovery,
    ResourceSourceLeaseStore, ResourceSubscriptionStore, SharedResourceStore,
};
use nebula_storage_port::{Scope, StorageError};
use sqlx::postgres::PgRow;
use sqlx::{PgPool, Postgres, Row as _, Transaction};
use uuid::Uuid;

const NOW_MS_SQL: &str = "SELECT (EXTRACT(EPOCH FROM clock_timestamp()) * 1000)::BIGINT";

/// PostgreSQL implementation of all shared-resource runtime persistence roles.
#[derive(Clone, Debug)]
pub struct PgResourceRuntime {
    pool: PgPool,
}

impl PgResourceRuntime {
    /// Wrap a pool initialized through [`super::init_schema`].
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    async fn begin_write(&self) -> Result<Transaction<'_, Postgres>, StorageError> {
        self.pool.begin().await.map_err(unavailable)
    }
}

fn unavailable(error: sqlx::Error) -> StorageError {
    match error {
        sqlx::Error::Configuration(_) => {
            StorageError::Configuration("resource runtime backend is misconfigured".to_owned())
        },
        sqlx::Error::Database(database) => {
            database_failure(database.kind(), database.code().as_deref())
        },
        sqlx::Error::Io(_)
        | sqlx::Error::Tls(_)
        | sqlx::Error::PoolTimedOut
        | sqlx::Error::PoolClosed
        | sqlx::Error::WorkerCrashed
        | sqlx::Error::BeginFailed => {
            StorageError::Connection("resource runtime backend unavailable".to_owned())
        },
        _ => corrupt(),
    }
}

fn database_failure(kind: sqlx::error::ErrorKind, code: Option<&str>) -> StorageError {
    if code.is_some_and(|value| value.starts_with("08") || value == "57014") {
        return StorageError::Connection("resource runtime backend unavailable".to_owned());
    }
    if kind == sqlx::error::ErrorKind::UniqueViolation {
        return StorageError::Conflict {
            entity: "resource runtime",
            id: "[opaque]".to_owned(),
            expected: 0,
            actual: 0,
        };
    }
    corrupt()
}

fn foreign_key_or_unavailable(error: sqlx::Error, parent: &'static str) -> StorageError {
    match &error {
        sqlx::Error::Database(database)
            if database.kind() == sqlx::error::ErrorKind::ForeignKeyViolation =>
        {
            missing(parent)
        },
        _ => unavailable(error),
    }
}

fn commit_unknown(_error: sqlx::Error) -> StorageError {
    StorageError::AcknowledgementUnknown {
        operation: "resource runtime mutation",
    }
}

fn corrupt() -> StorageError {
    StorageError::Internal("resource runtime persisted value is invalid".to_owned())
}

fn fenced(entity: &'static str) -> StorageError {
    tracing::warn!(storage.outcome = "fenced", storage.entity = entity);
    StorageError::FencedOut {
        entity,
        id: "[opaque]".to_owned(),
    }
}

fn missing(entity: &'static str) -> StorageError {
    StorageError::not_found(entity, "[opaque]")
}

fn id16(bytes: Vec<u8>) -> Result<[u8; 16], StorageError> {
    bytes.try_into().map_err(|_| corrupt())
}

fn decode_u64(bytes: Vec<u8>) -> Result<u64, StorageError> {
    Ok(u64::from_be_bytes(bytes.try_into().map_err(|_| corrupt())?))
}

fn expiry(ms: i64) -> Result<DateTime<Utc>, StorageError> {
    DateTime::from_timestamp_millis(ms).ok_or_else(corrupt)
}

fn ttl_ms(ttl: ResourceLeaseTtl) -> Result<i64, StorageError> {
    i64::try_from(ttl.get().as_millis()).map_err(|_| corrupt())
}

async fn now_ms(transaction: &mut Transaction<'_, Postgres>) -> Result<i64, StorageError> {
    sqlx::query_scalar(NOW_MS_SQL)
        .fetch_one(&mut **transaction)
        .await
        .map_err(unavailable)
}

async fn lock_scoped_key(
    transaction: &mut Transaction<'_, Postgres>,
    scope: &Scope,
    key: &[u8],
) -> Result<(), StorageError> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1 || ':' || $2 || ':' || encode($3, 'hex'), 0))")
        .bind(&scope.workspace_id)
        .bind(&scope.org_id)
        .bind(key)
        .execute(&mut **transaction)
        .await
        .map_err(unavailable)?;
    Ok(())
}

fn decode_resource(row: &PgRow) -> Result<SharedResourceRecord, StorageError> {
    let kind = ResourceKind::new(row.try_get::<String, _>("kind").map_err(unavailable)?)
        .map_err(|_| corrupt())?;
    let version = u32::try_from(
        row.try_get::<i64, _>("compatibility_version")
            .map_err(unavailable)?,
    )
    .map_err(|_| corrupt())?;
    let configuration = ResourceConfigurationIdentity::try_from_vec(
        row.try_get("configuration_identity").map_err(unavailable)?,
    )
    .map_err(|_| corrupt())?;
    let slot =
        ResourceSlotIdentity::try_from_vec(row.try_get("slot_identity").map_err(unavailable)?)
            .map_err(|_| corrupt())?;
    let sequence = u64::try_from(row.try_get::<i64, _>("sequence").map_err(unavailable)?)
        .map_err(|_| corrupt())?;
    Ok(SharedResourceRecord::new(
        SharedResourceId::from_bytes(id16(row.try_get("id").map_err(unavailable)?)?),
        SharedResourceIdentity::new(
            kind,
            ResourceCompatibilityVersion::new(version),
            configuration,
            slot,
        ),
        sequence,
    ))
}

fn decode_subscription(row: &PgRow) -> Result<ResourceSubscriptionRecord, StorageError> {
    Ok(ResourceSubscriptionRecord::new(
        ResourceSubscriptionId::from_bytes(id16(row.try_get("id").map_err(unavailable)?)?),
        SharedResourceId::from_bytes(id16(row.try_get("resource_id").map_err(unavailable)?)?),
        ResourceConsumerKind::new(
            row.try_get::<String, _>("consumer_kind")
                .map_err(unavailable)?,
        )
        .map_err(|_| corrupt())?,
        ResourceConsumerIdentity::try_from_vec(
            row.try_get("consumer_identity").map_err(unavailable)?,
        )
        .map_err(|_| corrupt())?,
        ResourceSubscriptionState::from_str(
            &row.try_get::<String, _>("state").map_err(unavailable)?,
        )
        .map_err(|_| corrupt())?,
        ResourceSubscriptionVersion::new(decode_u64(row.try_get("version").map_err(unavailable)?)?),
        u64::try_from(row.try_get::<i64, _>("sequence").map_err(unavailable)?)
            .map_err(|_| corrupt())?,
    ))
}

fn decode_subscription_page(
    rows: &[PgRow],
    page_size: ResourcePageSize,
) -> Result<ResourceSubscriptionPage, StorageError> {
    let subscriptions = rows
        .iter()
        .map(decode_subscription)
        .collect::<Result<Vec<_>, _>>()?;
    let next_cursor = (subscriptions.len() == usize::from(page_size.get()))
        .then(|| subscriptions.last())
        .flatten()
        .map(|record| ReconciliationCursor::from_sequence(record.reconciliation_sequence()));
    Ok(ResourceSubscriptionPage::new(subscriptions, next_cursor))
}

#[async_trait::async_trait]
impl SharedResourceStore for PgResourceRuntime {
    #[tracing::instrument(skip_all, fields(storage.role = "shared_resource", storage.operation = "resolve"))]
    async fn resolve(
        &self,
        request: ResolveSharedResourceRequest,
    ) -> Result<ResolveSharedResourceOutcome, StorageError> {
        let mut transaction = self.begin_write().await?;
        lock_scoped_key(
            &mut transaction,
            request.scope(),
            request.identity().digest(),
        )
        .await?;
        let rows = sqlx::query("SELECT sequence, id, kind, compatibility_version, configuration_identity, slot_identity FROM port_shared_resources WHERE workspace_id = $1 AND org_id = $2 AND identity_digest = $3 FOR UPDATE")
            .bind(&request.scope().workspace_id).bind(&request.scope().org_id).bind(request.identity().digest().as_slice())
            .fetch_all(&mut *transaction).await.map_err(unavailable)?;
        for row in rows {
            let record = decode_resource(&row)?;
            if record.identity() == request.identity() {
                transaction.commit().await.map_err(commit_unknown)?;
                return Ok(ResolveSharedResourceOutcome::Existing(record));
            }
        }
        let id = SharedResourceId::from_bytes(*Uuid::new_v4().as_bytes());
        sqlx::query("INSERT INTO port_shared_resources (id, workspace_id, org_id, kind, compatibility_version, configuration_identity, slot_identity, identity_digest) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)")
            .bind(id.into_bytes().as_slice()).bind(&request.scope().workspace_id).bind(&request.scope().org_id)
            .bind(request.identity().kind().as_str()).bind(i64::from(request.identity().compatibility_version().get()))
            .bind(request.identity().configuration_bytes()).bind(request.identity().slot_bytes()).bind(request.identity().digest().as_slice())
            .execute(&mut *transaction).await.map_err(unavailable)?;
        let sequence: i64 = sqlx::query_scalar("SELECT sequence FROM port_shared_resources WHERE workspace_id = $1 AND org_id = $2 AND id = $3")
            .bind(&request.scope().workspace_id).bind(&request.scope().org_id).bind(id.into_bytes().as_slice())
            .fetch_one(&mut *transaction).await.map_err(unavailable)?;
        let record = SharedResourceRecord::new(
            id,
            request.identity().clone(),
            u64::try_from(sequence).map_err(|_| corrupt())?,
        );
        transaction.commit().await.map_err(commit_unknown)?;
        Ok(ResolveSharedResourceOutcome::Created(record))
    }

    async fn get(
        &self,
        scope: &Scope,
        resource_id: SharedResourceId,
    ) -> Result<Option<SharedResourceRecord>, StorageError> {
        sqlx::query("SELECT sequence, id, kind, compatibility_version, configuration_identity, slot_identity FROM port_shared_resources WHERE workspace_id = $1 AND org_id = $2 AND id = $3")
            .bind(&scope.workspace_id).bind(&scope.org_id).bind(resource_id.into_bytes().as_slice())
            .fetch_optional(&self.pool).await.map_err(unavailable)?.map(|row| decode_resource(&row)).transpose()
    }

    async fn list_for_reconciliation(
        &self,
        scope: &Scope,
        after: Option<ReconciliationCursor>,
        page_size: ResourcePageSize,
    ) -> Result<SharedResourcePage, StorageError> {
        let after = after.map_or(0, ReconciliationCursor::sequence);
        let after = i64::try_from(after).unwrap_or(i64::MAX);
        let rows = sqlx::query("SELECT sequence, id, kind, compatibility_version, configuration_identity, slot_identity FROM port_shared_resources WHERE workspace_id = $1 AND org_id = $2 AND sequence > $3 ORDER BY sequence LIMIT $4")
            .bind(&scope.workspace_id).bind(&scope.org_id).bind(after).bind(i64::from(page_size.get()))
            .fetch_all(&self.pool).await.map_err(unavailable)?;
        let resources = rows
            .iter()
            .map(decode_resource)
            .collect::<Result<Vec<_>, _>>()?;
        let next_cursor = if resources.len() == usize::from(page_size.get()) {
            resources
                .last()
                .map(|record| ReconciliationCursor::from_sequence(record.reconciliation_sequence()))
        } else {
            None
        };
        Ok(SharedResourcePage::new(resources, next_cursor))
    }
}

#[async_trait::async_trait]
impl ResourceSubscriptionStore for PgResourceRuntime {
    #[tracing::instrument(skip_all, fields(storage.role = "resource_subscription", storage.operation = "put", resource_id = ?request.resource_id()))]
    async fn put(
        &self,
        request: PutResourceSubscriptionRequest,
    ) -> Result<PutResourceSubscriptionOutcome, StorageError> {
        let mut transaction = self.begin_write().await?;
        lock_scoped_key(
            &mut transaction,
            request.scope(),
            &request.resource_id().into_bytes(),
        )
        .await?;
        let existing = sqlx::query("SELECT sequence, id, resource_id, consumer_kind, consumer_identity, state, version FROM port_resource_subscriptions WHERE workspace_id = $1 AND org_id = $2 AND resource_id = $3 AND consumer_kind = $4 AND consumer_identity = $5 FOR UPDATE")
            .bind(&request.scope().workspace_id).bind(&request.scope().org_id)
            .bind(request.resource_id().into_bytes().as_slice()).bind(request.consumer_kind().as_str())
            .bind(request.consumer_identity_bytes()).fetch_optional(&mut *transaction).await.map_err(unavailable)?;
        if let Some(row) = existing {
            let record = decode_subscription(&row)?;
            transaction.commit().await.map_err(commit_unknown)?;
            return Ok(PutResourceSubscriptionOutcome::Existing(record));
        }
        let id = ResourceSubscriptionId::from_bytes(*Uuid::new_v4().as_bytes());
        let version = ResourceSubscriptionVersion::new(1);
        sqlx::query("INSERT INTO port_resource_subscriptions (id, workspace_id, org_id, resource_id, consumer_kind, consumer_identity, state, version) VALUES ($1, $2, $3, $4, $5, $6, 'active', $7)")
            .bind(id.into_bytes().as_slice()).bind(&request.scope().workspace_id).bind(&request.scope().org_id)
            .bind(request.resource_id().into_bytes().as_slice()).bind(request.consumer_kind().as_str())
            .bind(request.consumer_identity_bytes()).bind(version.get().to_be_bytes().as_slice())
            .execute(&mut *transaction).await.map_err(|error| foreign_key_or_unavailable(error, "shared resource"))?;
        let sequence: i64 = sqlx::query_scalar("SELECT sequence FROM port_resource_subscriptions WHERE workspace_id = $1 AND org_id = $2 AND id = $3")
            .bind(&request.scope().workspace_id).bind(&request.scope().org_id).bind(id.into_bytes().as_slice())
            .fetch_one(&mut *transaction).await.map_err(unavailable)?;
        let record = ResourceSubscriptionRecord::new(
            id,
            request.resource_id(),
            request.consumer_kind().clone(),
            ResourceConsumerIdentity::try_from_vec(request.consumer_identity_bytes().to_vec())
                .map_err(|_| corrupt())?,
            ResourceSubscriptionState::Active,
            version,
            u64::try_from(sequence).map_err(|_| corrupt())?,
        );
        transaction.commit().await.map_err(commit_unknown)?;
        Ok(PutResourceSubscriptionOutcome::Created(record))
    }

    async fn get(
        &self,
        scope: &Scope,
        subscription_id: ResourceSubscriptionId,
    ) -> Result<Option<ResourceSubscriptionRecord>, StorageError> {
        sqlx::query("SELECT sequence, id, resource_id, consumer_kind, consumer_identity, state, version FROM port_resource_subscriptions WHERE workspace_id = $1 AND org_id = $2 AND id = $3")
            .bind(&scope.workspace_id).bind(&scope.org_id).bind(subscription_id.into_bytes().as_slice())
            .fetch_optional(&self.pool).await.map_err(unavailable)?.map(|row| decode_subscription(&row)).transpose()
    }

    async fn list_active_for_resource(
        &self,
        scope: &Scope,
        resource_id: SharedResourceId,
        after: Option<ReconciliationCursor>,
        page_size: ResourcePageSize,
    ) -> Result<ResourceSubscriptionPage, StorageError> {
        let after =
            i64::try_from(after.map_or(0, ReconciliationCursor::sequence)).unwrap_or(i64::MAX);
        let rows = sqlx::query("SELECT sequence, id, resource_id, consumer_kind, consumer_identity, state, version FROM port_resource_subscriptions WHERE workspace_id = $1 AND org_id = $2 AND resource_id = $3 AND state = 'active' AND sequence > $4 ORDER BY sequence LIMIT $5")
            .bind(&scope.workspace_id).bind(&scope.org_id).bind(resource_id.into_bytes().as_slice()).bind(after).bind(i64::from(page_size.get()))
            .fetch_all(&self.pool).await.map_err(unavailable)?;
        decode_subscription_page(&rows, page_size)
    }

    async fn list_for_reconciliation(
        &self,
        scope: &Scope,
        after: Option<ReconciliationCursor>,
        page_size: ResourcePageSize,
    ) -> Result<ResourceSubscriptionPage, StorageError> {
        let after =
            i64::try_from(after.map_or(0, ReconciliationCursor::sequence)).unwrap_or(i64::MAX);
        let rows = sqlx::query("SELECT sequence, id, resource_id, consumer_kind, consumer_identity, state, version FROM port_resource_subscriptions WHERE workspace_id = $1 AND org_id = $2 AND sequence > $3 ORDER BY sequence LIMIT $4")
            .bind(&scope.workspace_id).bind(&scope.org_id).bind(after).bind(i64::from(page_size.get()))
            .fetch_all(&self.pool).await.map_err(unavailable)?;
        decode_subscription_page(&rows, page_size)
    }

    async fn count_active_for_resource(
        &self,
        scope: &Scope,
        resource_id: SharedResourceId,
    ) -> Result<u64, StorageError> {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM port_resource_subscriptions WHERE workspace_id = $1 AND org_id = $2 AND resource_id = $3 AND state = 'active'")
            .bind(&scope.workspace_id).bind(&scope.org_id).bind(resource_id.into_bytes().as_slice())
            .fetch_one(&self.pool).await.map_err(unavailable)?;
        u64::try_from(count).map_err(|_| corrupt())
    }

    #[tracing::instrument(skip_all, fields(storage.role = "resource_subscription", storage.operation = "transition", subscription_id = ?request.subscription_id()))]
    async fn transition(
        &self,
        request: TransitionResourceSubscriptionRequest,
    ) -> Result<ResourceSubscriptionRecord, StorageError> {
        let mut transaction = self.begin_write().await?;
        let row = sqlx::query("SELECT sequence, id, resource_id, consumer_kind, consumer_identity, state, version FROM port_resource_subscriptions WHERE workspace_id = $1 AND org_id = $2 AND id = $3 FOR UPDATE")
            .bind(&request.scope().workspace_id).bind(&request.scope().org_id).bind(request.subscription_id().into_bytes().as_slice())
            .fetch_optional(&mut *transaction).await.map_err(unavailable)?.ok_or_else(|| missing("resource subscription"))?;
        let current = decode_subscription(&row)?;
        if current.version() != request.expected_version() {
            if request.expected_version().get().checked_add(1) == Some(current.version().get())
                && current.state() == request.target_state()
            {
                transaction.commit().await.map_err(commit_unknown)?;
                return Ok(current);
            }
            return Err(StorageError::Conflict {
                entity: "resource subscription",
                id: "[opaque]".to_owned(),
                expected: request.expected_version().get(),
                actual: current.version().get(),
            });
        }
        if current.state() == ResourceSubscriptionState::Tombstoned {
            return Err(StorageError::Conflict {
                entity: "resource subscription",
                id: "[opaque]".to_owned(),
                expected: request.expected_version().get(),
                actual: current.version().get(),
            });
        }
        let next = current.version().get().checked_add(1).ok_or_else(corrupt)?;
        sqlx::query("UPDATE port_resource_subscriptions SET state = $1, version = $2 WHERE workspace_id = $3 AND org_id = $4 AND id = $5")
            .bind(request.target_state().as_str()).bind(next.to_be_bytes().as_slice())
            .bind(&request.scope().workspace_id).bind(&request.scope().org_id).bind(request.subscription_id().into_bytes().as_slice())
            .execute(&mut *transaction).await.map_err(unavailable)?;
        let updated = ResourceSubscriptionRecord::new(
            current.id(),
            current.resource_id(),
            current.consumer_kind().clone(),
            ResourceConsumerIdentity::try_from_vec(current.consumer_identity_bytes().to_vec())
                .map_err(|_| corrupt())?,
            request.target_state(),
            ResourceSubscriptionVersion::new(next),
            current.reconciliation_sequence(),
        );
        transaction.commit().await.map_err(commit_unknown)?;
        Ok(updated)
    }
}

fn decode_source_lease(
    row: &PgRow,
    resource_id: SharedResourceId,
) -> Result<ResourceSourceLease, StorageError> {
    let generation =
        ResourceLeaseGeneration::new(decode_u64(row.try_get("generation").map_err(unavailable)?)?);
    Ok(ResourceSourceLease::new(
        resource_id,
        ResourceLeaseHolder::new(row.try_get::<String, _>("holder").map_err(unavailable)?)
            .map_err(|_| corrupt())?,
        ResourceSourceLeaseToken::from_claim_bytes(
            id16(row.try_get("claim_id").map_err(unavailable)?)?,
            generation,
        ),
        expiry(row.try_get("expires_at_ms").map_err(unavailable)?)?,
    ))
}

#[async_trait::async_trait]
impl ResourceSourceLeaseStore for PgResourceRuntime {
    #[tracing::instrument(skip_all, fields(storage.role = "resource_source_lease", storage.operation = "acquire", resource_id = ?request.resource_id()))]
    async fn acquire(
        &self,
        request: AcquireResourceSourceLeaseRequest,
    ) -> Result<AcquireResourceSourceLeaseOutcome, StorageError> {
        let mut transaction = self.begin_write().await?;
        lock_scoped_key(
            &mut transaction,
            request.scope(),
            &request.resource_id().into_bytes(),
        )
        .await?;
        let existing = sqlx::query("SELECT holder, claim_id, generation, expires_at_ms FROM port_resource_source_leases WHERE workspace_id = $1 AND org_id = $2 AND resource_id = $3 FOR UPDATE")
            .bind(&request.scope().workspace_id).bind(&request.scope().org_id).bind(request.resource_id().into_bytes().as_slice())
            .fetch_optional(&mut *transaction).await.map_err(unavailable)?;
        let now = now_ms(&mut transaction).await?;
        let generation =
            if let Some(row) = &existing {
                let current = decode_source_lease(row, request.resource_id())?;
                if now < current.expires_at().timestamp_millis() {
                    transaction.commit().await.map_err(commit_unknown)?;
                    return Ok(AcquireResourceSourceLeaseOutcome::Contended {
                        expires_at: current.expires_at(),
                    });
                }
                current.token().generation().checked_next().map_err(|_| {
                    StorageError::Internal("resource generation exhausted".to_owned())
                })?
            } else {
                ResourceLeaseGeneration::new(1)
            };
        let expires_at_ms = now
            .checked_add(ttl_ms(request.ttl())?)
            .ok_or_else(corrupt)?;
        let claim_id = Uuid::new_v4();
        sqlx::query("INSERT INTO port_resource_source_leases (workspace_id, org_id, resource_id, holder, claim_id, generation, expires_at_ms) VALUES ($1, $2, $3, $4, $5, $6, $7) ON CONFLICT (workspace_id, org_id, resource_id) DO UPDATE SET holder = excluded.holder, claim_id = excluded.claim_id, generation = excluded.generation, expires_at_ms = excluded.expires_at_ms")
            .bind(&request.scope().workspace_id).bind(&request.scope().org_id).bind(request.resource_id().into_bytes().as_slice())
            .bind(request.holder().as_str()).bind(claim_id.as_bytes().as_slice()).bind(generation.get().to_be_bytes().as_slice())
            .bind(expires_at_ms).execute(&mut *transaction).await.map_err(|error| foreign_key_or_unavailable(error, "shared resource"))?;
        let lease = ResourceSourceLease::new(
            request.resource_id(),
            request.holder().clone(),
            ResourceSourceLeaseToken::new(claim_id, generation),
            expiry(expires_at_ms)?,
        );
        transaction.commit().await.map_err(commit_unknown)?;
        Ok(AcquireResourceSourceLeaseOutcome::Acquired(lease))
    }

    #[tracing::instrument(skip_all, fields(storage.role = "resource_source_lease", storage.operation = "heartbeat", resource_id = ?request.resource_id(), generation = request.token().generation().get()))]
    async fn heartbeat(
        &self,
        request: HeartbeatResourceSourceLeaseRequest,
    ) -> Result<ResourceSourceLease, StorageError> {
        let mut transaction = self.begin_write().await?;
        let row = sqlx::query("SELECT holder, claim_id, generation, expires_at_ms FROM port_resource_source_leases WHERE workspace_id = $1 AND org_id = $2 AND resource_id = $3 FOR UPDATE")
            .bind(&request.scope().workspace_id).bind(&request.scope().org_id).bind(request.resource_id().into_bytes().as_slice())
            .fetch_optional(&mut *transaction).await.map_err(unavailable)?.ok_or_else(|| fenced("resource source lease"))?;
        let now = now_ms(&mut transaction).await?;
        let current = decode_source_lease(&row, request.resource_id())?;
        if current.token() != request.token() || now >= current.expires_at().timestamp_millis() {
            return Err(fenced("resource source lease"));
        }
        let expires_at_ms = now
            .checked_add(ttl_ms(request.ttl())?)
            .ok_or_else(corrupt)?;
        let result = sqlx::query("UPDATE port_resource_source_leases SET expires_at_ms = $1 WHERE workspace_id = $2 AND org_id = $3 AND resource_id = $4 AND claim_id = $5 AND generation = $6")
            .bind(expires_at_ms).bind(&request.scope().workspace_id).bind(&request.scope().org_id)
            .bind(request.resource_id().into_bytes().as_slice()).bind(request.token().claim_id().as_bytes().as_slice())
            .bind(request.token().generation().get().to_be_bytes().as_slice())
            .execute(&mut *transaction).await.map_err(unavailable)?;
        if result.rows_affected() != 1 {
            return Err(fenced("resource source lease"));
        }
        let row = sqlx::query("SELECT holder, claim_id, generation, expires_at_ms FROM port_resource_source_leases WHERE workspace_id = $1 AND org_id = $2 AND resource_id = $3")
            .bind(&request.scope().workspace_id).bind(&request.scope().org_id).bind(request.resource_id().into_bytes().as_slice())
            .fetch_one(&mut *transaction).await.map_err(unavailable)?;
        let lease = decode_source_lease(&row, request.resource_id())?;
        transaction.commit().await.map_err(commit_unknown)?;
        Ok(lease)
    }

    #[tracing::instrument(skip_all, fields(storage.role = "resource_source_lease", storage.operation = "release", resource_id = ?request.resource_id(), generation = request.token().generation().get()))]
    async fn release(
        &self,
        request: ReleaseResourceSourceLeaseRequest,
    ) -> Result<(), StorageError> {
        let mut transaction = self.begin_write().await?;
        sqlx::query("UPDATE port_resource_source_leases SET expires_at_ms = 0 WHERE workspace_id = $1 AND org_id = $2 AND resource_id = $3 AND claim_id = $4 AND generation = $5")
            .bind(&request.scope().workspace_id).bind(&request.scope().org_id).bind(request.resource_id().into_bytes().as_slice())
            .bind(request.token().claim_id().as_bytes().as_slice()).bind(request.token().generation().get().to_be_bytes().as_slice())
            .execute(&mut *transaction).await.map_err(unavailable)?;
        transaction.commit().await.map_err(commit_unknown)?;
        Ok(())
    }
}

fn decode_event(row: &PgRow) -> Result<ResourceEventRecord, StorageError> {
    Ok(ResourceEventRecord::new(
        ResourceEventId::from_bytes(id16(row.try_get("id").map_err(unavailable)?)?),
        SharedResourceId::from_bytes(id16(row.try_get("resource_id").map_err(unavailable)?)?),
        EventOccurrenceNamespace::new(
            row.try_get::<String, _>("occurrence_namespace")
                .map_err(unavailable)?,
        )
        .map_err(|_| corrupt())?,
        EventOccurrenceKey::try_from_vec(row.try_get("occurrence_key").map_err(unavailable)?)
            .map_err(|_| corrupt())?,
        EventEnvelope::try_from_vec(
            u32::try_from(
                row.try_get::<i64, _>("schema_version")
                    .map_err(unavailable)?,
            )
            .map_err(|_| corrupt())?,
            row.try_get("canonical_payload").map_err(unavailable)?,
        )
        .map_err(|_| corrupt())?,
        ResourceEventAcceptance::new(
            expiry(row.try_get("accepted_at_ms").map_err(unavailable)?)?,
            ResourceLeaseGeneration::new(decode_u64(
                row.try_get("source_generation").map_err(unavailable)?,
            )?),
        ),
        ResourceEventState::from_str(&row.try_get::<String, _>("state").map_err(unavailable)?)
            .map_err(|_| corrupt())?,
    ))
}

fn completion_columns(
    completion: ResourceDeliveryCompletion,
) -> (&'static str, Option<&'static str>) {
    match completion {
        ResourceDeliveryCompletion::Delivered => ("delivered", None),
        ResourceDeliveryCompletion::Ineligible(reason) => ("ineligible", Some(reason.as_str())),
    }
}

fn persisted_completion(
    status: &str,
    reason: Option<&str>,
) -> Result<Option<ResourceDeliveryCompletion>, StorageError> {
    match status {
        "pending" => Ok(None),
        "delivered" if reason.is_none() => Ok(Some(ResourceDeliveryCompletion::Delivered)),
        "ineligible" => Ok(Some(ResourceDeliveryCompletion::Ineligible(
            TerminalDeliveryIneligibility::from_str(reason.ok_or_else(corrupt)?)
                .map_err(|_| corrupt())?,
        ))),
        _ => Err(corrupt()),
    }
}

fn effective_completion(
    requested: ResourceDeliveryCompletion,
    subscription_state: &str,
) -> Result<ResourceDeliveryCompletion, StorageError> {
    if requested != ResourceDeliveryCompletion::Delivered {
        return Ok(requested);
    }
    match subscription_state {
        "active" => Ok(ResourceDeliveryCompletion::Delivered),
        "disabled" => Ok(ResourceDeliveryCompletion::Ineligible(
            TerminalDeliveryIneligibility::SubscriptionDisabled,
        )),
        "tombstoned" => Ok(ResourceDeliveryCompletion::Ineligible(
            TerminalDeliveryIneligibility::SubscriptionTombstoned,
        )),
        _ => Err(corrupt()),
    }
}

fn trace_acceptance_outcome(outcome: &AcceptResourceEventOutcome) {
    match outcome {
        AcceptResourceEventOutcome::Accepted {
            event_id,
            delivery_count,
        } => tracing::debug!(storage.outcome = "accepted", ?event_id, delivery_count),
        AcceptResourceEventOutcome::Replayed { event_id } => {
            tracing::debug!(storage.outcome = "replayed", ?event_id);
        },
        AcceptResourceEventOutcome::Conflict { event_id } => {
            tracing::warn!(storage.outcome = "conflict", ?event_id);
        },
    }
}

fn trace_completion_outcome(outcome: CompleteResourceDeliveryOutcome) {
    let (outcome_name, completion) = match outcome {
        CompleteResourceDeliveryOutcome::Completed { completion, .. } => ("completed", completion),
        CompleteResourceDeliveryOutcome::AlreadyCompleted { completion, .. } => {
            ("completion_replayed", completion)
        },
    };
    let terminal = match completion {
        ResourceDeliveryCompletion::Delivered => "delivered",
        ResourceDeliveryCompletion::Ineligible(reason) => reason.as_str(),
    };
    tracing::debug!(storage.outcome = outcome_name, terminal);
}

fn decode_claimed_delivery(row: &PgRow) -> Result<ClaimedResourceDelivery, StorageError> {
    let generation = ResourceLeaseGeneration::new(decode_u64(
        row.try_get("claim_generation").map_err(unavailable)?,
    )?);
    Ok(ClaimedResourceDelivery::new(
        ResourceDeliveryId::from_bytes(id16(row.try_get("id").map_err(unavailable)?)?),
        ResourceEventId::from_bytes(id16(row.try_get("event_id").map_err(unavailable)?)?),
        ResourceSubscriptionId::from_bytes(id16(
            row.try_get("subscription_id").map_err(unavailable)?,
        )?),
        EventEnvelope::try_from_vec(
            u32::try_from(
                row.try_get::<i64, _>("schema_version")
                    .map_err(unavailable)?,
            )
            .map_err(|_| corrupt())?,
            row.try_get("canonical_payload").map_err(unavailable)?,
        )
        .map_err(|_| corrupt())?,
        ResourceDeliveryClaimToken::from_claim_bytes(
            id16(row.try_get("claim_id").map_err(unavailable)?)?,
            generation,
        ),
    ))
}

fn decode_claimed_handoff(row: &PgRow) -> Result<ClaimedResourceHandoff, StorageError> {
    Ok(ClaimedResourceHandoff::new(
        ResourceDeliveryId::from_bytes(id16(row.try_get("delivery_id").map_err(unavailable)?)?),
        ResourceEventId::from_bytes(id16(row.try_get("event_id").map_err(unavailable)?)?),
        ResourceSubscriptionId::from_bytes(id16(
            row.try_get("subscription_id").map_err(unavailable)?,
        )?),
        EventEnvelope::try_from_vec(
            u32::try_from(
                row.try_get::<i64, _>("schema_version")
                    .map_err(unavailable)?,
            )
            .map_err(|_| corrupt())?,
            row.try_get("canonical_payload").map_err(unavailable)?,
        )
        .map_err(|_| corrupt())?,
        ResourceHandoffClaimToken::from_claim_bytes(
            id16(row.try_get("claim_id").map_err(unavailable)?)?,
            ResourceLeaseGeneration::new(decode_u64(
                row.try_get("claim_generation").map_err(unavailable)?,
            )?),
        ),
    ))
}

async fn ensure_handoff(
    transaction: &mut Transaction<'_, Postgres>,
    scope: &Scope,
    delivery_id: ResourceDeliveryId,
    resource_id: SharedResourceId,
    event_id: ResourceEventId,
    subscription_id: ResourceSubscriptionId,
) -> Result<(), StorageError> {
    sqlx::query("INSERT INTO port_resource_execution_handoffs (workspace_id, org_id, resource_id, delivery_id, event_id, subscription_id, status, claim_generation) VALUES ($1, $2, $3, $4, $5, $6, 'pending', $7) ON CONFLICT (workspace_id, org_id, delivery_id) DO NOTHING")
        .bind(&scope.workspace_id).bind(&scope.org_id).bind(resource_id.into_bytes().as_slice())
        .bind(delivery_id.into_bytes().as_slice()).bind(event_id.into_bytes().as_slice())
        .bind(subscription_id.into_bytes().as_slice()).bind(0_u64.to_be_bytes().as_slice())
        .execute(&mut **transaction).await.map_err(unavailable)?;
    let exact: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM port_resource_execution_handoffs WHERE workspace_id = $1 AND org_id = $2 AND resource_id = $3 AND delivery_id = $4 AND event_id = $5 AND subscription_id = $6")
        .bind(&scope.workspace_id).bind(&scope.org_id).bind(resource_id.into_bytes().as_slice())
        .bind(delivery_id.into_bytes().as_slice()).bind(event_id.into_bytes().as_slice())
        .bind(subscription_id.into_bytes().as_slice()).fetch_one(&mut **transaction).await.map_err(unavailable)?;
    if exact == 1 {
        tracing::debug!(
            storage.outcome = "handoff_confirmed",
            delivery_id = ?delivery_id,
        );
        Ok(())
    } else {
        Err(StorageError::Conflict {
            entity: "resource execution handoff",
            id: "[opaque]".to_owned(),
            expected: 1,
            actual: 0,
        })
    }
}

#[async_trait::async_trait]
impl ResourceEventFanoutStore for PgResourceRuntime {
    #[tracing::instrument(skip_all, fields(storage.role = "resource_event_fanout", storage.operation = "accept", resource_id = ?request.resource_id(), source_generation = request.source_token().generation().get()))]
    async fn accept(
        &self,
        request: AcceptResourceEventRequest,
    ) -> Result<AcceptResourceEventOutcome, StorageError> {
        let mut transaction = self.begin_write().await?;
        let occurrence_digest = request.occurrence_digest();
        lock_scoped_key(&mut transaction, request.scope(), &occurrence_digest).await?;
        let live_source = sqlx::query("SELECT expires_at_ms FROM port_resource_source_leases WHERE workspace_id = $1 AND org_id = $2 AND resource_id = $3 AND claim_id = $4 AND generation = $5 FOR UPDATE")
            .bind(&request.scope().workspace_id).bind(&request.scope().org_id).bind(request.resource_id().into_bytes().as_slice())
            .bind(request.source_token().claim_id().as_bytes().as_slice()).bind(request.source_token().generation().get().to_be_bytes().as_slice())
            .fetch_optional(&mut *transaction).await.map_err(unavailable)?;
        let now = now_ms(&mut transaction).await?;
        let is_live = live_source
            .as_ref()
            .and_then(|row| row.try_get::<i64, _>("expires_at_ms").ok())
            .is_some_and(|expires_at_ms| now < expires_at_ms);
        if !is_live {
            return Err(fenced("resource source lease"));
        }

        let rows = sqlx::query("SELECT id, resource_id, occurrence_namespace, occurrence_key, schema_version, canonical_payload, accepted_at_ms, source_generation, state FROM port_resource_events WHERE workspace_id = $1 AND org_id = $2 AND resource_id = $3 AND occurrence_digest = $4 FOR UPDATE")
            .bind(&request.scope().workspace_id).bind(&request.scope().org_id).bind(request.resource_id().into_bytes().as_slice())
            .bind(occurrence_digest.as_slice()).fetch_all(&mut *transaction).await.map_err(unavailable)?;
        for row in rows {
            let event = decode_event(&row)?;
            if event.namespace() == request.namespace()
                && event.occurrence_key_bytes() == request.occurrence_key_bytes()
            {
                let outcome = if event.envelope() == request.envelope() {
                    AcceptResourceEventOutcome::Replayed {
                        event_id: event.id(),
                    }
                } else {
                    AcceptResourceEventOutcome::Conflict {
                        event_id: event.id(),
                    }
                };
                transaction.commit().await.map_err(commit_unknown)?;
                trace_acceptance_outcome(&outcome);
                return Ok(outcome);
            }
        }

        let event_id = ResourceEventId::from_bytes(*Uuid::new_v4().as_bytes());
        sqlx::query("INSERT INTO port_resource_events (id, workspace_id, org_id, resource_id, occurrence_namespace, occurrence_key, occurrence_digest, schema_version, canonical_payload, envelope_digest, accepted_at_ms, source_generation, state) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, 'pending')")
            .bind(event_id.into_bytes().as_slice()).bind(&request.scope().workspace_id).bind(&request.scope().org_id)
            .bind(request.resource_id().into_bytes().as_slice()).bind(request.namespace().as_str()).bind(request.occurrence_key_bytes())
            .bind(occurrence_digest.as_slice()).bind(i64::from(request.envelope().schema_version())).bind(request.envelope().canonical_payload())
            .bind(request.envelope().digest().as_slice()).bind(now)
            .bind(request.source_token().generation().get().to_be_bytes().as_slice())
            .execute(&mut *transaction).await.map_err(unavailable)?;
        let inserted = sqlx::query("INSERT INTO port_resource_deliveries (id, workspace_id, org_id, resource_id, event_id, subscription_id, status, claim_generation) SELECT uuid_send(gen_random_uuid()), workspace_id, org_id, resource_id, $1, id, 'pending', $2 FROM port_resource_subscriptions WHERE workspace_id = $3 AND org_id = $4 AND resource_id = $5 AND state = 'active'")
            .bind(event_id.into_bytes().as_slice()).bind(0_u64.to_be_bytes().as_slice())
            .bind(&request.scope().workspace_id).bind(&request.scope().org_id)
            .bind(request.resource_id().into_bytes().as_slice())
            .execute(&mut *transaction).await.map_err(unavailable)?;
        let delivery_count = u32::try_from(inserted.rows_affected()).map_err(|_| corrupt())?;
        if delivery_count == 0 {
            sqlx::query("UPDATE port_resource_events SET state = 'complete' WHERE workspace_id = $1 AND org_id = $2 AND id = $3")
                .bind(&request.scope().workspace_id).bind(&request.scope().org_id)
                .bind(event_id.into_bytes().as_slice()).execute(&mut *transaction).await.map_err(unavailable)?;
        }
        transaction.commit().await.map_err(commit_unknown)?;
        let outcome = AcceptResourceEventOutcome::Accepted {
            event_id,
            delivery_count,
        };
        trace_acceptance_outcome(&outcome);
        Ok(outcome)
    }

    async fn get_event(
        &self,
        scope: &Scope,
        event_id: ResourceEventId,
    ) -> Result<Option<ResourceEventRecord>, StorageError> {
        sqlx::query("SELECT id, resource_id, occurrence_namespace, occurrence_key, schema_version, canonical_payload, accepted_at_ms, source_generation, state FROM port_resource_events WHERE workspace_id = $1 AND org_id = $2 AND id = $3")
            .bind(&scope.workspace_id).bind(&scope.org_id).bind(event_id.into_bytes().as_slice())
            .fetch_optional(&self.pool).await.map_err(unavailable)?.map(|row| decode_event(&row)).transpose()
    }

    #[tracing::instrument(skip_all, fields(storage.role = "resource_event_fanout", storage.operation = "claim_deliveries"))]
    async fn claim_deliveries(
        &self,
        request: ClaimResourceDeliveriesRequest,
    ) -> Result<Vec<ClaimedResourceDelivery>, StorageError> {
        let mut transaction = self.begin_write().await?;
        let rows = sqlx::query("SELECT d.id, d.event_id, d.subscription_id, d.claim_generation, e.schema_version, e.canonical_payload FROM port_resource_deliveries d JOIN port_resource_events e ON e.workspace_id = d.workspace_id AND e.org_id = d.org_id AND e.id = d.event_id WHERE d.workspace_id = $1 AND d.org_id = $2 AND d.status = 'pending' AND (d.claim_id IS NULL OR d.claim_expires_at_ms <= (EXTRACT(EPOCH FROM clock_timestamp()) * 1000)::BIGINT) ORDER BY d.sequence LIMIT $3 FOR UPDATE OF d SKIP LOCKED")
            .bind(&request.scope().workspace_id).bind(&request.scope().org_id).bind(i64::from(request.batch_size().get()))
            .fetch_all(&mut *transaction).await.map_err(unavailable)?;
        let now = now_ms(&mut transaction).await?;
        let mut prepared = Vec::with_capacity(rows.len());
        for row in rows {
            let current = decode_u64(row.try_get("claim_generation").map_err(unavailable)?)?;
            let next = current.checked_add(1).ok_or_else(|| {
                StorageError::Internal("resource generation exhausted".to_owned())
            })?;
            prepared.push((row, next, Uuid::new_v4()));
        }
        let expires_at_ms = now
            .checked_add(ttl_ms(request.ttl())?)
            .ok_or_else(corrupt)?;
        let mut claimed = Vec::with_capacity(prepared.len());
        for (row, generation, claim_id) in prepared {
            let delivery_id =
                ResourceDeliveryId::from_bytes(id16(row.try_get("id").map_err(unavailable)?)?);
            sqlx::query("UPDATE port_resource_deliveries SET claim_holder = $1, claim_id = $2, claim_generation = $3, claim_expires_at_ms = $4 WHERE workspace_id = $5 AND org_id = $6 AND id = $7")
                .bind(request.holder().as_str()).bind(claim_id.as_bytes().as_slice()).bind(generation.to_be_bytes().as_slice()).bind(expires_at_ms)
                .bind(&request.scope().workspace_id).bind(&request.scope().org_id).bind(delivery_id.into_bytes().as_slice())
                .execute(&mut *transaction).await.map_err(unavailable)?;
            claimed.push(ClaimedResourceDelivery::new(
                delivery_id,
                ResourceEventId::from_bytes(id16(row.try_get("event_id").map_err(unavailable)?)?),
                ResourceSubscriptionId::from_bytes(id16(
                    row.try_get("subscription_id").map_err(unavailable)?,
                )?),
                EventEnvelope::try_from_vec(
                    u32::try_from(
                        row.try_get::<i64, _>("schema_version")
                            .map_err(unavailable)?,
                    )
                    .map_err(|_| corrupt())?,
                    row.try_get("canonical_payload").map_err(unavailable)?,
                )
                .map_err(|_| corrupt())?,
                ResourceDeliveryClaimToken::new(claim_id, ResourceLeaseGeneration::new(generation)),
            ));
        }
        transaction.commit().await.map_err(commit_unknown)?;
        tracing::debug!(storage.outcome = "claimed", claim_count = claimed.len());
        Ok(claimed)
    }

    #[tracing::instrument(skip_all, fields(storage.role = "resource_event_fanout", storage.operation = "heartbeat_delivery", delivery_id = ?request.delivery_id(), generation = request.token().generation().get()))]
    async fn heartbeat_delivery(
        &self,
        request: HeartbeatResourceDeliveryRequest,
    ) -> Result<ClaimedResourceDelivery, StorageError> {
        let mut transaction = self.begin_write().await?;
        let row = sqlx::query("SELECT d.id, d.event_id, d.subscription_id, d.claim_id, d.claim_generation, d.claim_expires_at_ms, e.schema_version, e.canonical_payload FROM port_resource_deliveries d JOIN port_resource_events e ON e.workspace_id = d.workspace_id AND e.org_id = d.org_id AND e.id = d.event_id WHERE d.workspace_id = $1 AND d.org_id = $2 AND d.id = $3 AND d.status = 'pending' AND d.claim_id = $4 AND d.claim_generation = $5 FOR UPDATE OF d")
            .bind(&request.scope().workspace_id).bind(&request.scope().org_id).bind(request.delivery_id().into_bytes().as_slice())
            .bind(request.token().claim_id().as_bytes().as_slice()).bind(request.token().generation().get().to_be_bytes().as_slice())
            .fetch_optional(&mut *transaction).await.map_err(unavailable)?.ok_or_else(|| fenced("resource delivery"))?;
        let now = now_ms(&mut transaction).await?;
        let current_expiry: i64 = row.try_get("claim_expires_at_ms").map_err(unavailable)?;
        if now >= current_expiry {
            return Err(fenced("resource delivery"));
        }
        let expires_at_ms = now
            .checked_add(ttl_ms(request.ttl())?)
            .ok_or_else(corrupt)?;
        let updated = sqlx::query("UPDATE port_resource_deliveries SET claim_expires_at_ms = $1 WHERE workspace_id = $2 AND org_id = $3 AND id = $4 AND status = 'pending' AND claim_id = $5 AND claim_generation = $6")
            .bind(expires_at_ms).bind(&request.scope().workspace_id).bind(&request.scope().org_id).bind(request.delivery_id().into_bytes().as_slice())
            .bind(request.token().claim_id().as_bytes().as_slice()).bind(request.token().generation().get().to_be_bytes().as_slice())
            .execute(&mut *transaction).await.map_err(unavailable)?;
        if updated.rows_affected() != 1 {
            return Err(fenced("resource delivery"));
        }
        let row = sqlx::query("SELECT d.id, d.event_id, d.subscription_id, d.claim_id, d.claim_generation, e.schema_version, e.canonical_payload FROM port_resource_deliveries d JOIN port_resource_events e ON e.workspace_id = d.workspace_id AND e.org_id = d.org_id AND e.id = d.event_id WHERE d.workspace_id = $1 AND d.org_id = $2 AND d.id = $3")
            .bind(&request.scope().workspace_id).bind(&request.scope().org_id).bind(request.delivery_id().into_bytes().as_slice())
            .fetch_one(&mut *transaction).await.map_err(unavailable)?;
        let claim = decode_claimed_delivery(&row)?;
        transaction.commit().await.map_err(commit_unknown)?;
        Ok(claim)
    }

    #[tracing::instrument(skip_all, fields(storage.role = "resource_event_fanout", storage.operation = "release_delivery", delivery_id = ?request.delivery_id(), generation = request.token().generation().get()))]
    async fn release_delivery(
        &self,
        request: ReleaseResourceDeliveryRequest,
    ) -> Result<(), StorageError> {
        let mut transaction = self.begin_write().await?;
        sqlx::query("UPDATE port_resource_deliveries SET claim_holder = NULL, claim_id = NULL, claim_expires_at_ms = NULL WHERE workspace_id = $1 AND org_id = $2 AND id = $3 AND status = 'pending' AND claim_id = $4 AND claim_generation = $5")
            .bind(&request.scope().workspace_id).bind(&request.scope().org_id).bind(request.delivery_id().into_bytes().as_slice())
            .bind(request.token().claim_id().as_bytes().as_slice()).bind(request.token().generation().get().to_be_bytes().as_slice())
            .execute(&mut *transaction).await.map_err(unavailable)?;
        transaction.commit().await.map_err(commit_unknown)?;
        Ok(())
    }

    #[tracing::instrument(skip_all, fields(storage.role = "resource_event_fanout", storage.operation = "complete_delivery", delivery_id = ?request.delivery_id(), generation = request.token().generation().get()))]
    async fn complete_delivery(
        &self,
        request: CompleteResourceDeliveryRequest,
    ) -> Result<CompleteResourceDeliveryOutcome, StorageError> {
        let mut transaction = self.begin_write().await?;
        let row = sqlx::query("SELECT resource_id, event_id, subscription_id, status, terminal_reason, requested_status, requested_terminal_reason, claim_id, claim_generation, claim_expires_at_ms, terminal_claim_id, terminal_claim_generation FROM port_resource_deliveries WHERE workspace_id = $1 AND org_id = $2 AND id = $3 FOR UPDATE")
            .bind(&request.scope().workspace_id).bind(&request.scope().org_id).bind(request.delivery_id().into_bytes().as_slice())
            .fetch_optional(&mut *transaction).await.map_err(unavailable)?.ok_or_else(|| fenced("resource delivery"))?;
        let event_id =
            ResourceEventId::from_bytes(id16(row.try_get("event_id").map_err(unavailable)?)?);
        let resource_id =
            SharedResourceId::from_bytes(id16(row.try_get("resource_id").map_err(unavailable)?)?);
        let subscription_id = ResourceSubscriptionId::from_bytes(id16(
            row.try_get("subscription_id").map_err(unavailable)?,
        )?);
        let event_state: String = sqlx::query_scalar("SELECT state FROM port_resource_events WHERE workspace_id = $1 AND org_id = $2 AND id = $3 FOR UPDATE")
            .bind(&request.scope().workspace_id).bind(&request.scope().org_id).bind(event_id.into_bytes().as_slice())
            .fetch_one(&mut *transaction).await.map_err(unavailable)?;
        let subscription_state: String = sqlx::query_scalar("SELECT state FROM port_resource_subscriptions WHERE workspace_id = $1 AND org_id = $2 AND resource_id = $3 AND id = $4 FOR UPDATE")
            .bind(&request.scope().workspace_id).bind(&request.scope().org_id).bind(resource_id.into_bytes().as_slice()).bind(subscription_id.into_bytes().as_slice())
            .fetch_one(&mut *transaction).await.map_err(unavailable)?;
        let effective_completion = effective_completion(request.completion(), &subscription_state)?;
        let status: String = row.try_get("status").map_err(unavailable)?;
        let reason: Option<String> = row.try_get("terminal_reason").map_err(unavailable)?;
        if let Some(completion) = persisted_completion(&status, reason.as_deref())? {
            let requested_status: Option<String> =
                row.try_get("requested_status").map_err(unavailable)?;
            let requested_reason: Option<String> = row
                .try_get("requested_terminal_reason")
                .map_err(unavailable)?;
            let requested_completion = persisted_completion(
                requested_status.as_deref().ok_or_else(corrupt)?,
                requested_reason.as_deref(),
            )?
            .ok_or_else(corrupt)?;
            let terminal_claim_id = id16(
                row.try_get::<Option<Vec<u8>>, _>("terminal_claim_id")
                    .map_err(unavailable)?
                    .ok_or_else(corrupt)?,
            )?;
            let terminal_generation = decode_u64(
                row.try_get::<Option<Vec<u8>>, _>("terminal_claim_generation")
                    .map_err(unavailable)?
                    .ok_or_else(corrupt)?,
            )?;
            if requested_completion != request.completion()
                || terminal_claim_id != *request.token().claim_id().as_bytes()
                || terminal_generation != request.token().generation().get()
            {
                return Err(fenced("resource delivery"));
            }
            if completion == ResourceDeliveryCompletion::Delivered {
                ensure_handoff(
                    &mut transaction,
                    request.scope(),
                    request.delivery_id(),
                    resource_id,
                    event_id,
                    subscription_id,
                )
                .await?;
            }
            transaction.commit().await.map_err(commit_unknown)?;
            let outcome = CompleteResourceDeliveryOutcome::AlreadyCompleted {
                completion,
                event_is_terminal: event_state == "complete",
            };
            trace_completion_outcome(outcome);
            return Ok(outcome);
        }
        let now = now_ms(&mut transaction).await?;
        let claim_id = id16(
            row.try_get::<Option<Vec<u8>>, _>("claim_id")
                .map_err(unavailable)?
                .ok_or_else(|| fenced("resource delivery"))?,
        )?;
        let claim_generation = decode_u64(
            row.try_get::<Vec<u8>, _>("claim_generation")
                .map_err(unavailable)?,
        )?;
        let claim_expires_at_ms: Option<i64> =
            row.try_get("claim_expires_at_ms").map_err(unavailable)?;
        if claim_id != *request.token().claim_id().as_bytes()
            || claim_generation != request.token().generation().get()
            || claim_expires_at_ms.is_none_or(|deadline| now >= deadline)
        {
            return Err(fenced("resource delivery"));
        }
        let (next_status, terminal_reason) = completion_columns(effective_completion);
        let (requested_status, requested_terminal_reason) =
            completion_columns(request.completion());
        if effective_completion == ResourceDeliveryCompletion::Delivered {
            ensure_handoff(
                &mut transaction,
                request.scope(),
                request.delivery_id(),
                resource_id,
                event_id,
                subscription_id,
            )
            .await?;
        }
        sqlx::query("UPDATE port_resource_deliveries SET status = $1, terminal_reason = $2, requested_status = $3, requested_terminal_reason = $4, terminal_claim_id = claim_id, terminal_claim_generation = claim_generation, claim_holder = NULL, claim_id = NULL, claim_expires_at_ms = NULL WHERE workspace_id = $5 AND org_id = $6 AND id = $7")
            .bind(next_status).bind(terminal_reason).bind(requested_status).bind(requested_terminal_reason).bind(&request.scope().workspace_id).bind(&request.scope().org_id).bind(request.delivery_id().into_bytes().as_slice())
            .execute(&mut *transaction).await.map_err(unavailable)?;
        let has_pending: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM port_resource_deliveries WHERE workspace_id = $1 AND org_id = $2 AND event_id = $3 AND status = 'pending')")
            .bind(&request.scope().workspace_id).bind(&request.scope().org_id).bind(event_id.into_bytes().as_slice())
            .fetch_one(&mut *transaction).await.map_err(unavailable)?;
        let event_became_terminal = !has_pending;
        if event_became_terminal {
            sqlx::query("UPDATE port_resource_events SET state = 'complete' WHERE workspace_id = $1 AND org_id = $2 AND id = $3")
                .bind(&request.scope().workspace_id).bind(&request.scope().org_id).bind(event_id.into_bytes().as_slice())
                .execute(&mut *transaction).await.map_err(unavailable)?;
        }
        transaction.commit().await.map_err(commit_unknown)?;
        let outcome = CompleteResourceDeliveryOutcome::Completed {
            completion: effective_completion,
            event_became_terminal,
        };
        trace_completion_outcome(outcome);
        Ok(outcome)
    }
}

#[async_trait::async_trait]
impl ResourceExecutionHandoffStore for PgResourceRuntime {
    #[tracing::instrument(skip_all, fields(storage.role = "resource_execution_handoff", storage.operation = "claim"))]
    async fn claim_handoffs(
        &self,
        request: ClaimResourceHandoffsRequest,
    ) -> Result<Vec<ClaimedResourceHandoff>, StorageError> {
        let mut transaction = self.begin_write().await?;
        let rows = sqlx::query("SELECT h.delivery_id, h.event_id, h.subscription_id, h.claim_generation, e.schema_version, e.canonical_payload FROM port_resource_execution_handoffs h JOIN port_resource_events e ON e.workspace_id = h.workspace_id AND e.org_id = h.org_id AND e.resource_id = h.resource_id AND e.id = h.event_id WHERE h.workspace_id = $1 AND h.org_id = $2 AND h.status = 'pending' AND (h.claim_id IS NULL OR h.claim_expires_at_ms <= (EXTRACT(EPOCH FROM clock_timestamp()) * 1000)::BIGINT) ORDER BY h.sequence LIMIT $3 FOR UPDATE OF h SKIP LOCKED")
            .bind(&request.scope().workspace_id).bind(&request.scope().org_id).bind(i64::from(request.batch_size().get()))
            .fetch_all(&mut *transaction).await.map_err(unavailable)?;
        let now = now_ms(&mut transaction).await?;
        let mut prepared = Vec::with_capacity(rows.len());
        for row in rows {
            let generation = decode_u64(row.try_get("claim_generation").map_err(unavailable)?)?
                .checked_add(1)
                .ok_or_else(|| {
                    StorageError::Internal("resource generation exhausted".to_owned())
                })?;
            prepared.push((row, generation, Uuid::new_v4()));
        }
        let expires_at_ms = now
            .checked_add(ttl_ms(request.ttl())?)
            .ok_or_else(corrupt)?;
        let mut claims = Vec::with_capacity(prepared.len());
        for (row, generation, claim_id) in prepared {
            let delivery_id = ResourceDeliveryId::from_bytes(id16(
                row.try_get("delivery_id").map_err(unavailable)?,
            )?);
            sqlx::query("UPDATE port_resource_execution_handoffs SET claim_holder = $1, claim_id = $2, claim_generation = $3, claim_expires_at_ms = $4 WHERE workspace_id = $5 AND org_id = $6 AND delivery_id = $7")
                .bind(request.holder().as_str()).bind(claim_id.as_bytes().as_slice()).bind(generation.to_be_bytes().as_slice()).bind(expires_at_ms)
                .bind(&request.scope().workspace_id).bind(&request.scope().org_id).bind(delivery_id.into_bytes().as_slice())
                .execute(&mut *transaction).await.map_err(unavailable)?;
            claims.push(ClaimedResourceHandoff::new(
                delivery_id,
                ResourceEventId::from_bytes(id16(row.try_get("event_id").map_err(unavailable)?)?),
                ResourceSubscriptionId::from_bytes(id16(
                    row.try_get("subscription_id").map_err(unavailable)?,
                )?),
                EventEnvelope::try_from_vec(
                    u32::try_from(
                        row.try_get::<i64, _>("schema_version")
                            .map_err(unavailable)?,
                    )
                    .map_err(|_| corrupt())?,
                    row.try_get("canonical_payload").map_err(unavailable)?,
                )
                .map_err(|_| corrupt())?,
                ResourceHandoffClaimToken::new(claim_id, ResourceLeaseGeneration::new(generation)),
            ));
        }
        transaction.commit().await.map_err(commit_unknown)?;
        tracing::debug!(storage.outcome = "claimed", claim_count = claims.len());
        Ok(claims)
    }

    #[tracing::instrument(skip_all, fields(storage.role = "resource_execution_handoff", storage.operation = "heartbeat", delivery_id = ?request.delivery_id(), generation = request.token().generation().get()))]
    async fn heartbeat_handoff(
        &self,
        request: HeartbeatResourceHandoffRequest,
    ) -> Result<ClaimedResourceHandoff, StorageError> {
        let mut transaction = self.begin_write().await?;
        let row = sqlx::query("SELECT h.delivery_id, h.event_id, h.subscription_id, h.claim_id, h.claim_generation, h.claim_expires_at_ms, e.schema_version, e.canonical_payload FROM port_resource_execution_handoffs h JOIN port_resource_events e ON e.workspace_id = h.workspace_id AND e.org_id = h.org_id AND e.resource_id = h.resource_id AND e.id = h.event_id WHERE h.workspace_id = $1 AND h.org_id = $2 AND h.delivery_id = $3 AND h.status = 'pending' AND h.claim_id = $4 AND h.claim_generation = $5 FOR UPDATE OF h")
            .bind(&request.scope().workspace_id).bind(&request.scope().org_id).bind(request.delivery_id().into_bytes().as_slice())
            .bind(request.token().claim_id().as_bytes().as_slice()).bind(request.token().generation().get().to_be_bytes().as_slice())
            .fetch_optional(&mut *transaction).await.map_err(unavailable)?.ok_or_else(|| fenced("resource execution handoff"))?;
        let now = now_ms(&mut transaction).await?;
        let deadline: i64 = row.try_get("claim_expires_at_ms").map_err(unavailable)?;
        if now >= deadline {
            return Err(fenced("resource execution handoff"));
        }
        let expires_at_ms = now
            .checked_add(ttl_ms(request.ttl())?)
            .ok_or_else(corrupt)?;
        sqlx::query("UPDATE port_resource_execution_handoffs SET claim_expires_at_ms = $1 WHERE workspace_id = $2 AND org_id = $3 AND delivery_id = $4 AND claim_id = $5 AND claim_generation = $6")
            .bind(expires_at_ms).bind(&request.scope().workspace_id).bind(&request.scope().org_id).bind(request.delivery_id().into_bytes().as_slice())
            .bind(request.token().claim_id().as_bytes().as_slice()).bind(request.token().generation().get().to_be_bytes().as_slice())
            .execute(&mut *transaction).await.map_err(unavailable)?;
        let claim = decode_claimed_handoff(&row)?;
        transaction.commit().await.map_err(commit_unknown)?;
        Ok(claim)
    }

    #[tracing::instrument(skip_all, fields(storage.role = "resource_execution_handoff", storage.operation = "release", delivery_id = ?request.delivery_id(), generation = request.token().generation().get()))]
    async fn release_handoff(
        &self,
        request: ResourceHandoffClaimRequest,
    ) -> Result<(), StorageError> {
        let mut transaction = self.begin_write().await?;
        sqlx::query("UPDATE port_resource_execution_handoffs SET claim_holder = NULL, claim_id = NULL, claim_expires_at_ms = NULL WHERE workspace_id = $1 AND org_id = $2 AND delivery_id = $3 AND status = 'pending' AND claim_id = $4 AND claim_generation = $5")
            .bind(&request.scope().workspace_id).bind(&request.scope().org_id).bind(request.delivery_id().into_bytes().as_slice())
            .bind(request.token().claim_id().as_bytes().as_slice()).bind(request.token().generation().get().to_be_bytes().as_slice())
            .execute(&mut *transaction).await.map_err(unavailable)?;
        transaction.commit().await.map_err(commit_unknown)?;
        Ok(())
    }

    #[tracing::instrument(skip_all, fields(storage.role = "resource_execution_handoff", storage.operation = "acknowledge", delivery_id = ?request.delivery_id(), generation = request.token().generation().get()))]
    async fn acknowledge_handoff(
        &self,
        request: ResourceHandoffClaimRequest,
    ) -> Result<AcknowledgeResourceHandoffOutcome, StorageError> {
        let mut transaction = self.begin_write().await?;
        let row = sqlx::query("SELECT status, claim_id, claim_generation, claim_expires_at_ms, terminal_claim_id, terminal_claim_generation FROM port_resource_execution_handoffs WHERE workspace_id = $1 AND org_id = $2 AND delivery_id = $3 FOR UPDATE")
            .bind(&request.scope().workspace_id).bind(&request.scope().org_id).bind(request.delivery_id().into_bytes().as_slice())
            .fetch_optional(&mut *transaction).await.map_err(unavailable)?.ok_or_else(|| fenced("resource execution handoff"))?;
        let now = now_ms(&mut transaction).await?;
        let status: String = row.try_get("status").map_err(unavailable)?;
        if status == "acknowledged" {
            let terminal_id = id16(
                row.try_get::<Option<Vec<u8>>, _>("terminal_claim_id")
                    .map_err(unavailable)?
                    .ok_or_else(corrupt)?,
            )?;
            let terminal_generation = decode_u64(
                row.try_get::<Option<Vec<u8>>, _>("terminal_claim_generation")
                    .map_err(unavailable)?
                    .ok_or_else(corrupt)?,
            )?;
            if terminal_id == *request.token().claim_id().as_bytes()
                && terminal_generation == request.token().generation().get()
            {
                transaction.commit().await.map_err(commit_unknown)?;
                tracing::debug!(storage.outcome = "already_acknowledged");
                return Ok(AcknowledgeResourceHandoffOutcome::AlreadyAcknowledged);
            }
            return Err(fenced("resource execution handoff"));
        }
        let claim_id = id16(
            row.try_get::<Option<Vec<u8>>, _>("claim_id")
                .map_err(unavailable)?
                .ok_or_else(|| fenced("resource execution handoff"))?,
        )?;
        let generation = decode_u64(row.try_get("claim_generation").map_err(unavailable)?)?;
        let deadline: Option<i64> = row.try_get("claim_expires_at_ms").map_err(unavailable)?;
        if claim_id != *request.token().claim_id().as_bytes()
            || generation != request.token().generation().get()
            || deadline.is_none_or(|value| now >= value)
        {
            return Err(fenced("resource execution handoff"));
        }
        sqlx::query("UPDATE port_resource_execution_handoffs SET status = 'acknowledged', terminal_claim_id = claim_id, terminal_claim_generation = claim_generation, claim_holder = NULL, claim_id = NULL, claim_expires_at_ms = NULL WHERE workspace_id = $1 AND org_id = $2 AND delivery_id = $3")
            .bind(&request.scope().workspace_id).bind(&request.scope().org_id).bind(request.delivery_id().into_bytes().as_slice()).execute(&mut *transaction).await.map_err(unavailable)?;
        transaction.commit().await.map_err(commit_unknown)?;
        tracing::debug!(storage.outcome = "acknowledged");
        Ok(AcknowledgeResourceHandoffOutcome::Acknowledged)
    }
}

#[async_trait::async_trait]
impl ResourceRuntimeRecovery for PgResourceRuntime {
    #[tracing::instrument(skip_all, fields(storage.role = "resource_runtime_recovery", storage.operation = "claim_deliveries"))]
    async fn claim_deliveries_globally(
        &self,
        request: ClaimResourceRuntimeWorkRequest,
    ) -> Result<Vec<ScopedClaimedResourceDelivery>, StorageError> {
        let mut transaction = self.pool.begin().await.map_err(unavailable)?;
        let rows = sqlx::query("SELECT d.workspace_id, d.org_id, d.id, d.event_id, d.subscription_id, d.claim_generation, e.schema_version, e.canonical_payload FROM port_resource_deliveries d JOIN port_resource_events e ON e.workspace_id = d.workspace_id AND e.org_id = d.org_id AND e.id = d.event_id WHERE d.status = 'pending' AND (d.claim_id IS NULL OR d.claim_expires_at_ms <= (EXTRACT(EPOCH FROM clock_timestamp()) * 1000)::BIGINT) ORDER BY d.sequence LIMIT $1 FOR UPDATE OF d SKIP LOCKED")
            .bind(i64::from(request.batch_size().get()))
            .fetch_all(&mut *transaction).await.map_err(unavailable)?;
        let mut claimed = Vec::with_capacity(rows.len());
        for row in rows {
            let scope = Scope::new(
                row.try_get::<String, _>("workspace_id")
                    .map_err(unavailable)?,
                row.try_get::<String, _>("org_id").map_err(unavailable)?,
            );
            let delivery_id =
                ResourceDeliveryId::from_bytes(id16(row.try_get("id").map_err(unavailable)?)?);
            let generation = decode_u64(row.try_get("claim_generation").map_err(unavailable)?)?
                .checked_add(1)
                .ok_or_else(|| {
                    StorageError::Internal("resource generation exhausted".to_owned())
                })?;
            let claim_id = Uuid::new_v4();
            sqlx::query("UPDATE port_resource_deliveries SET claim_holder = $1, claim_id = $2, claim_generation = $3, claim_expires_at_ms = (EXTRACT(EPOCH FROM clock_timestamp()) * 1000)::BIGINT + $4 WHERE workspace_id = $5 AND org_id = $6 AND id = $7")
                .bind(request.holder().as_str()).bind(claim_id.as_bytes().as_slice()).bind(generation.to_be_bytes().as_slice())
                .bind(ttl_ms(request.ttl())?).bind(&scope.workspace_id).bind(&scope.org_id).bind(delivery_id.into_bytes().as_slice())
                .execute(&mut *transaction).await.map_err(unavailable)?;
            claimed.push(ScopedClaimedResourceDelivery::new(
                scope,
                ClaimedResourceDelivery::new(
                    delivery_id,
                    ResourceEventId::from_bytes(id16(
                        row.try_get("event_id").map_err(unavailable)?,
                    )?),
                    ResourceSubscriptionId::from_bytes(id16(
                        row.try_get("subscription_id").map_err(unavailable)?,
                    )?),
                    EventEnvelope::try_from_vec(
                        u32::try_from(
                            row.try_get::<i64, _>("schema_version")
                                .map_err(unavailable)?,
                        )
                        .map_err(|_| corrupt())?,
                        row.try_get("canonical_payload").map_err(unavailable)?,
                    )
                    .map_err(|_| corrupt())?,
                    ResourceDeliveryClaimToken::new(
                        claim_id,
                        ResourceLeaseGeneration::new(generation),
                    ),
                ),
            ));
        }
        transaction.commit().await.map_err(commit_unknown)?;
        tracing::debug!(storage.outcome = "claimed", claim_count = claimed.len());
        Ok(claimed)
    }

    #[tracing::instrument(skip_all, fields(storage.role = "resource_runtime_recovery", storage.operation = "claim_handoffs"))]
    async fn claim_handoffs_globally(
        &self,
        request: ClaimResourceRuntimeWorkRequest,
    ) -> Result<Vec<ScopedClaimedResourceHandoff>, StorageError> {
        let mut transaction = self.pool.begin().await.map_err(unavailable)?;
        let rows = sqlx::query("SELECT h.workspace_id, h.org_id, h.delivery_id, h.event_id, h.subscription_id, h.claim_generation, e.schema_version, e.canonical_payload FROM port_resource_execution_handoffs h JOIN port_resource_events e ON e.workspace_id = h.workspace_id AND e.org_id = h.org_id AND e.resource_id = h.resource_id AND e.id = h.event_id WHERE h.status = 'pending' AND (h.claim_id IS NULL OR h.claim_expires_at_ms <= (EXTRACT(EPOCH FROM clock_timestamp()) * 1000)::BIGINT) ORDER BY h.sequence LIMIT $1 FOR UPDATE OF h SKIP LOCKED")
            .bind(i64::from(request.batch_size().get()))
            .fetch_all(&mut *transaction).await.map_err(unavailable)?;
        let mut claimed = Vec::with_capacity(rows.len());
        for row in rows {
            let scope = Scope::new(
                row.try_get::<String, _>("workspace_id")
                    .map_err(unavailable)?,
                row.try_get::<String, _>("org_id").map_err(unavailable)?,
            );
            let delivery_id = ResourceDeliveryId::from_bytes(id16(
                row.try_get("delivery_id").map_err(unavailable)?,
            )?);
            let generation = decode_u64(row.try_get("claim_generation").map_err(unavailable)?)?
                .checked_add(1)
                .ok_or_else(|| {
                    StorageError::Internal("resource generation exhausted".to_owned())
                })?;
            let claim_id = Uuid::new_v4();
            sqlx::query("UPDATE port_resource_execution_handoffs SET claim_holder = $1, claim_id = $2, claim_generation = $3, claim_expires_at_ms = (EXTRACT(EPOCH FROM clock_timestamp()) * 1000)::BIGINT + $4 WHERE workspace_id = $5 AND org_id = $6 AND delivery_id = $7")
                .bind(request.holder().as_str()).bind(claim_id.as_bytes().as_slice()).bind(generation.to_be_bytes().as_slice())
                .bind(ttl_ms(request.ttl())?).bind(&scope.workspace_id).bind(&scope.org_id).bind(delivery_id.into_bytes().as_slice())
                .execute(&mut *transaction).await.map_err(unavailable)?;
            claimed.push(ScopedClaimedResourceHandoff::new(
                scope,
                ClaimedResourceHandoff::new(
                    delivery_id,
                    ResourceEventId::from_bytes(id16(
                        row.try_get("event_id").map_err(unavailable)?,
                    )?),
                    ResourceSubscriptionId::from_bytes(id16(
                        row.try_get("subscription_id").map_err(unavailable)?,
                    )?),
                    EventEnvelope::try_from_vec(
                        u32::try_from(
                            row.try_get::<i64, _>("schema_version")
                                .map_err(unavailable)?,
                        )
                        .map_err(|_| corrupt())?,
                        row.try_get("canonical_payload").map_err(unavailable)?,
                    )
                    .map_err(|_| corrupt())?,
                    ResourceHandoffClaimToken::new(
                        claim_id,
                        ResourceLeaseGeneration::new(generation),
                    ),
                ),
            ));
        }
        transaction.commit().await.map_err(commit_unknown)?;
        tracing::debug!(storage.outcome = "claimed", claim_count = claimed.len());
        Ok(claimed)
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use super::database_failure;
    use nebula_storage_port::StorageError;

    #[test]
    fn database_sqlstates_have_closed_storage_classes() {
        assert_matches!(
            database_failure(sqlx::error::ErrorKind::Other, Some("08006")),
            StorageError::Connection(_)
        );
        assert_matches!(
            database_failure(sqlx::error::ErrorKind::Other, Some("57014")),
            StorageError::Connection(_)
        );
        assert_matches!(
            database_failure(sqlx::error::ErrorKind::UniqueViolation, Some("23505")),
            StorageError::Conflict { .. }
        );
        assert_matches!(
            database_failure(sqlx::error::ErrorKind::Other, Some("P0001")),
            StorageError::Internal(_)
        );
    }
}
