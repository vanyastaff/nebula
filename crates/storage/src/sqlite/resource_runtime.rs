//! SQLite persistence for the shared-resource runtime aggregate, over
//! `shared_resources`, `resource_subscriptions`, `resource_source_leases`,
//! `resource_events`, `resource_deliveries` and `resource_execution_handoffs`.
//!
//! Every mutation runs under `BEGIN IMMEDIATE`, SQLite's single writer. A
//! shared resource belongs to its workspace: resolving one checks the
//! workspace is live inside that transaction (`NotFound` when it or its org is
//! missing or archived), and everything beneath a shared resource cascades
//! from it. Instants are INTEGER microseconds read from SQLite's clock;
//! fencing generations and subscription versions are non-negative INTEGERs.

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
use sqlx::sqlite::SqliteRow;
use sqlx::{Row as _, Sqlite, SqlitePool, Transaction};
use uuid::Uuid;

use crate::resource_runtime_error::{
    commit_unknown, corrupt_record, deadline_overflow, foreign_key_or_statement_error,
    generation_exhausted, invalid_computed_count, statement_error,
};

const NOW_MICROS_SQL: &str =
    "SELECT CAST((julianday('now') - 2440587.5) * 86400000000.0 AS INTEGER)";

/// SQLite implementation of all shared-resource runtime persistence roles.
#[derive(Clone, Debug)]
pub struct SqliteResourceRuntime {
    pool: SqlitePool,
}

impl SqliteResourceRuntime {
    /// Wrap a pool initialized through [`super::init_schema`].
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    async fn begin_write(&self) -> Result<Transaction<'_, Sqlite>, StorageError> {
        self.pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(statement_error)
    }
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
    bytes.try_into().map_err(|_| corrupt_record())
}

/// A stored generation or version (a non-negative INTEGER).
fn decode_counter(value: i64) -> Result<u64, StorageError> {
    u64::try_from(value).map_err(|_| corrupt_record())
}

/// A generation or version as the INTEGER it is stored as; past `i64::MAX`
/// the counter is exhausted.
fn encode_counter(value: u64) -> Result<i64, StorageError> {
    i64::try_from(value).map_err(|_| generation_exhausted())
}

/// The generation after `current`, within the stored range.
fn next_counter(current: u64) -> Result<u64, StorageError> {
    let next = current.checked_add(1).ok_or_else(generation_exhausted)?;
    encode_counter(next)?;
    Ok(next)
}

/// A stored instant (INTEGER microseconds since the Unix epoch).
fn instant(micros: i64) -> Result<DateTime<Utc>, StorageError> {
    DateTime::from_timestamp_micros(micros).ok_or_else(corrupt_record)
}

/// `ttl` past `now`, both in microseconds.
fn expiry_after(now: i64, ttl: ResourceLeaseTtl) -> Result<i64, StorageError> {
    let ttl = i64::try_from(ttl.get().as_micros()).map_err(|_| deadline_overflow())?;
    now.checked_add(ttl).ok_or_else(deadline_overflow)
}

/// SQLite's clock, in microseconds.
async fn now_micros(transaction: &mut Transaction<'_, Sqlite>) -> Result<i64, StorageError> {
    sqlx::query_scalar(NOW_MICROS_SQL)
        .fetch_one(&mut **transaction)
        .await
        .map_err(statement_error)
}

fn decode_resource(row: &SqliteRow) -> Result<SharedResourceRecord, StorageError> {
    let kind = ResourceKind::new(row.try_get::<String, _>("kind").map_err(statement_error)?)
        .map_err(|_| corrupt_record())?;
    let version = u32::try_from(
        row.try_get::<i64, _>("compatibility_version")
            .map_err(statement_error)?,
    )
    .map_err(|_| corrupt_record())?;
    let configuration = ResourceConfigurationIdentity::try_from_vec(
        row.try_get("configuration_identity")
            .map_err(statement_error)?,
    )
    .map_err(|_| corrupt_record())?;
    let slot =
        ResourceSlotIdentity::try_from_vec(row.try_get("slot_identity").map_err(statement_error)?)
            .map_err(|_| corrupt_record())?;
    let sequence = u64::try_from(row.try_get::<i64, _>("sequence").map_err(statement_error)?)
        .map_err(|_| corrupt_record())?;
    Ok(SharedResourceRecord::new(
        SharedResourceId::from_bytes(id16(row.try_get("id").map_err(statement_error)?)?),
        SharedResourceIdentity::new(
            kind,
            ResourceCompatibilityVersion::new(version),
            configuration,
            slot,
        ),
        sequence,
    ))
}

fn decode_subscription(row: &SqliteRow) -> Result<ResourceSubscriptionRecord, StorageError> {
    Ok(ResourceSubscriptionRecord::new(
        ResourceSubscriptionId::from_bytes(id16(row.try_get("id").map_err(statement_error)?)?),
        SharedResourceId::from_bytes(id16(row.try_get("resource_id").map_err(statement_error)?)?),
        ResourceConsumerKind::new(
            row.try_get::<String, _>("consumer_kind")
                .map_err(statement_error)?,
        )
        .map_err(|_| corrupt_record())?,
        ResourceConsumerIdentity::try_from_vec(
            row.try_get("consumer_identity").map_err(statement_error)?,
        )
        .map_err(|_| corrupt_record())?,
        ResourceSubscriptionState::from_str(
            &row.try_get::<String, _>("state").map_err(statement_error)?,
        )
        .map_err(|_| corrupt_record())?,
        ResourceSubscriptionVersion::new(decode_counter(
            row.try_get("version").map_err(statement_error)?,
        )?),
        u64::try_from(row.try_get::<i64, _>("sequence").map_err(statement_error)?)
            .map_err(|_| corrupt_record())?,
    ))
}

fn decode_subscription_page(
    rows: &[SqliteRow],
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
impl SharedResourceStore for SqliteResourceRuntime {
    #[tracing::instrument(skip_all, fields(storage.role = "shared_resource", storage.operation = "resolve"))]
    async fn resolve(
        &self,
        request: ResolveSharedResourceRequest,
    ) -> Result<ResolveSharedResourceOutcome, StorageError> {
        let mut transaction = self.begin_write().await?;
        super::identity::ensure_live_workspace(&mut transaction, request.scope()).await?;
        let rows = sqlx::query("SELECT sequence, id, kind, compatibility_version, configuration_identity, slot_identity FROM shared_resources WHERE workspace_id = ? AND org_id = ? AND identity_digest = ?")
            .bind(&request.scope().workspace_id).bind(&request.scope().org_id).bind(request.identity().digest().as_slice())
            .fetch_all(&mut *transaction).await.map_err(statement_error)?;
        for row in rows {
            let record = decode_resource(&row)?;
            if record.identity() == request.identity() {
                transaction.commit().await.map_err(commit_unknown)?;
                return Ok(ResolveSharedResourceOutcome::Existing(record));
            }
        }
        let id = SharedResourceId::from_bytes(*Uuid::new_v4().as_bytes());
        let sequence: i64 = sqlx::query_scalar("INSERT INTO shared_resources (org_id, workspace_id, id, kind, compatibility_version, configuration_identity, slot_identity, identity_digest) VALUES (?, ?, ?, ?, ?, ?, ?, ?) RETURNING sequence")
            .bind(&request.scope().org_id).bind(&request.scope().workspace_id).bind(id.into_bytes().as_slice())
            .bind(request.identity().kind().as_str()).bind(i64::from(request.identity().compatibility_version().get()))
            .bind(request.identity().configuration_bytes()).bind(request.identity().slot_bytes()).bind(request.identity().digest().as_slice())
            .fetch_one(&mut *transaction).await.map_err(|error| foreign_key_or_statement_error(error, "workspace"))?;
        let record = SharedResourceRecord::new(
            id,
            request.identity().clone(),
            u64::try_from(sequence).map_err(|_| corrupt_record())?,
        );
        transaction.commit().await.map_err(commit_unknown)?;
        Ok(ResolveSharedResourceOutcome::Created(record))
    }

    async fn get(
        &self,
        scope: &Scope,
        resource_id: SharedResourceId,
    ) -> Result<Option<SharedResourceRecord>, StorageError> {
        sqlx::query("SELECT sequence, id, kind, compatibility_version, configuration_identity, slot_identity FROM shared_resources WHERE workspace_id = ? AND org_id = ? AND id = ?")
            .bind(&scope.workspace_id).bind(&scope.org_id).bind(resource_id.into_bytes().as_slice())
            .fetch_optional(&self.pool).await.map_err(statement_error)?.map(|row| decode_resource(&row)).transpose()
    }

    async fn list_for_reconciliation(
        &self,
        scope: &Scope,
        after: Option<ReconciliationCursor>,
        page_size: ResourcePageSize,
    ) -> Result<SharedResourcePage, StorageError> {
        let after = after.map_or(0, ReconciliationCursor::sequence);
        let after = i64::try_from(after).unwrap_or(i64::MAX);
        let rows = sqlx::query("SELECT sequence, id, kind, compatibility_version, configuration_identity, slot_identity FROM shared_resources WHERE workspace_id = ? AND org_id = ? AND sequence > ? ORDER BY sequence LIMIT ?")
            .bind(&scope.workspace_id).bind(&scope.org_id).bind(after).bind(i64::from(page_size.get()))
            .fetch_all(&self.pool).await.map_err(statement_error)?;
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
impl ResourceSubscriptionStore for SqliteResourceRuntime {
    #[tracing::instrument(skip_all, fields(storage.role = "resource_subscription", storage.operation = "put", resource_id = ?request.resource_id()))]
    async fn put(
        &self,
        request: PutResourceSubscriptionRequest,
    ) -> Result<PutResourceSubscriptionOutcome, StorageError> {
        let mut transaction = self.begin_write().await?;
        let existing = sqlx::query("SELECT sequence, id, resource_id, consumer_kind, consumer_identity, state, version FROM resource_subscriptions WHERE workspace_id = ? AND org_id = ? AND resource_id = ? AND consumer_kind = ? AND consumer_identity = ?")
            .bind(&request.scope().workspace_id).bind(&request.scope().org_id)
            .bind(request.resource_id().into_bytes().as_slice()).bind(request.consumer_kind().as_str())
            .bind(request.consumer_identity_bytes()).fetch_optional(&mut *transaction).await.map_err(statement_error)?;
        if let Some(row) = existing {
            let record = decode_subscription(&row)?;
            transaction.commit().await.map_err(commit_unknown)?;
            return Ok(PutResourceSubscriptionOutcome::Existing(record));
        }
        let id = ResourceSubscriptionId::from_bytes(*Uuid::new_v4().as_bytes());
        let version = ResourceSubscriptionVersion::new(1);
        let sequence: i64 = sqlx::query_scalar("INSERT INTO resource_subscriptions (org_id, workspace_id, resource_id, id, consumer_kind, consumer_identity, state, version) VALUES (?, ?, ?, ?, ?, ?, 'active', ?) RETURNING sequence")
            .bind(&request.scope().org_id).bind(&request.scope().workspace_id)
            .bind(request.resource_id().into_bytes().as_slice()).bind(id.into_bytes().as_slice()).bind(request.consumer_kind().as_str())
            .bind(request.consumer_identity_bytes()).bind(encode_counter(version.get())?)
            .fetch_one(&mut *transaction).await.map_err(|error| foreign_key_or_statement_error(error, "shared resource"))?;
        let record = ResourceSubscriptionRecord::new(
            id,
            request.resource_id(),
            request.consumer_kind().clone(),
            ResourceConsumerIdentity::try_from_vec(request.consumer_identity_bytes().to_vec())
                .map_err(|_| corrupt_record())?,
            ResourceSubscriptionState::Active,
            version,
            u64::try_from(sequence).map_err(|_| corrupt_record())?,
        );
        transaction.commit().await.map_err(commit_unknown)?;
        Ok(PutResourceSubscriptionOutcome::Created(record))
    }

    async fn get(
        &self,
        scope: &Scope,
        subscription_id: ResourceSubscriptionId,
    ) -> Result<Option<ResourceSubscriptionRecord>, StorageError> {
        sqlx::query("SELECT sequence, id, resource_id, consumer_kind, consumer_identity, state, version FROM resource_subscriptions WHERE workspace_id = ? AND org_id = ? AND id = ?")
            .bind(&scope.workspace_id).bind(&scope.org_id).bind(subscription_id.into_bytes().as_slice())
            .fetch_optional(&self.pool).await.map_err(statement_error)?.map(|row| decode_subscription(&row)).transpose()
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
        let rows = sqlx::query("SELECT sequence, id, resource_id, consumer_kind, consumer_identity, state, version FROM resource_subscriptions WHERE workspace_id = ? AND org_id = ? AND resource_id = ? AND state = 'active' AND sequence > ? ORDER BY sequence LIMIT ?")
            .bind(&scope.workspace_id).bind(&scope.org_id).bind(resource_id.into_bytes().as_slice()).bind(after).bind(i64::from(page_size.get()))
            .fetch_all(&self.pool).await.map_err(statement_error)?;
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
        let rows = sqlx::query("SELECT sequence, id, resource_id, consumer_kind, consumer_identity, state, version FROM resource_subscriptions WHERE workspace_id = ? AND org_id = ? AND sequence > ? ORDER BY sequence LIMIT ?")
            .bind(&scope.workspace_id).bind(&scope.org_id).bind(after).bind(i64::from(page_size.get()))
            .fetch_all(&self.pool).await.map_err(statement_error)?;
        decode_subscription_page(&rows, page_size)
    }

    async fn count_active_for_resource(
        &self,
        scope: &Scope,
        resource_id: SharedResourceId,
    ) -> Result<u64, StorageError> {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM resource_subscriptions WHERE workspace_id = ? AND org_id = ? AND resource_id = ? AND state = 'active'")
            .bind(&scope.workspace_id).bind(&scope.org_id).bind(resource_id.into_bytes().as_slice())
            .fetch_one(&self.pool).await.map_err(statement_error)?;
        u64::try_from(count).map_err(|_| invalid_computed_count())
    }

    #[tracing::instrument(skip_all, fields(storage.role = "resource_subscription", storage.operation = "transition", subscription_id = ?request.subscription_id()))]
    async fn transition(
        &self,
        request: TransitionResourceSubscriptionRequest,
    ) -> Result<ResourceSubscriptionRecord, StorageError> {
        let mut transaction = self.begin_write().await?;
        let row = sqlx::query("SELECT sequence, id, resource_id, consumer_kind, consumer_identity, state, version FROM resource_subscriptions WHERE workspace_id = ? AND org_id = ? AND id = ?")
            .bind(&request.scope().workspace_id).bind(&request.scope().org_id).bind(request.subscription_id().into_bytes().as_slice())
            .fetch_optional(&mut *transaction).await.map_err(statement_error)?.ok_or_else(|| missing("resource subscription"))?;
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
        let next = next_counter(current.version().get())?;
        sqlx::query("UPDATE resource_subscriptions SET state = ?, version = ? WHERE workspace_id = ? AND org_id = ? AND id = ?")
            .bind(request.target_state().as_str()).bind(encode_counter(next)?)
            .bind(&request.scope().workspace_id).bind(&request.scope().org_id).bind(request.subscription_id().into_bytes().as_slice())
            .execute(&mut *transaction).await.map_err(statement_error)?;
        let updated = ResourceSubscriptionRecord::new(
            current.id(),
            current.resource_id(),
            current.consumer_kind().clone(),
            ResourceConsumerIdentity::try_from_vec(current.consumer_identity_bytes().to_vec())
                .map_err(|_| corrupt_record())?,
            request.target_state(),
            ResourceSubscriptionVersion::new(next),
            current.reconciliation_sequence(),
        );
        transaction.commit().await.map_err(commit_unknown)?;
        Ok(updated)
    }
}

fn decode_source_lease(
    row: &SqliteRow,
    resource_id: SharedResourceId,
) -> Result<ResourceSourceLease, StorageError> {
    let generation = ResourceLeaseGeneration::new(decode_counter(
        row.try_get("generation").map_err(statement_error)?,
    )?);
    Ok(ResourceSourceLease::new(
        resource_id,
        ResourceLeaseHolder::new(
            row.try_get::<String, _>("holder")
                .map_err(statement_error)?,
        )
        .map_err(|_| corrupt_record())?,
        ResourceSourceLeaseToken::from_claim_bytes(
            id16(row.try_get("claim_id").map_err(statement_error)?)?,
            generation,
        ),
        instant(row.try_get("expires_at").map_err(statement_error)?)?,
    ))
}

#[async_trait::async_trait]
impl ResourceSourceLeaseStore for SqliteResourceRuntime {
    #[tracing::instrument(skip_all, fields(storage.role = "resource_source_lease", storage.operation = "acquire", resource_id = ?request.resource_id()))]
    async fn acquire(
        &self,
        request: AcquireResourceSourceLeaseRequest,
    ) -> Result<AcquireResourceSourceLeaseOutcome, StorageError> {
        let mut transaction = self.begin_write().await?;
        let now = now_micros(&mut transaction).await?;
        let existing = sqlx::query("SELECT holder, claim_id, generation, expires_at FROM resource_source_leases WHERE workspace_id = ? AND org_id = ? AND resource_id = ?")
            .bind(&request.scope().workspace_id).bind(&request.scope().org_id).bind(request.resource_id().into_bytes().as_slice())
            .fetch_optional(&mut *transaction).await.map_err(statement_error)?;
        let generation = if let Some(row) = &existing {
            let current = decode_source_lease(row, request.resource_id())?;
            if instant(now)? < current.expires_at() {
                transaction.commit().await.map_err(commit_unknown)?;
                return Ok(AcquireResourceSourceLeaseOutcome::Contended {
                    expires_at: current.expires_at(),
                });
            }
            ResourceLeaseGeneration::new(next_counter(current.token().generation().get())?)
        } else {
            ResourceLeaseGeneration::new(1)
        };
        let expires_at = expiry_after(now, request.ttl())?;
        let claim_id = Uuid::new_v4();
        sqlx::query("INSERT INTO resource_source_leases (org_id, workspace_id, resource_id, holder, claim_id, generation, expires_at) VALUES (?, ?, ?, ?, ?, ?, ?) ON CONFLICT (org_id, workspace_id, resource_id) DO UPDATE SET holder = excluded.holder, claim_id = excluded.claim_id, generation = excluded.generation, expires_at = excluded.expires_at")
            .bind(&request.scope().org_id).bind(&request.scope().workspace_id).bind(request.resource_id().into_bytes().as_slice())
            .bind(request.holder().as_str()).bind(claim_id.as_bytes().as_slice()).bind(encode_counter(generation.get())?)
            .bind(expires_at).execute(&mut *transaction).await.map_err(|error| foreign_key_or_statement_error(error, "shared resource"))?;
        let lease = ResourceSourceLease::new(
            request.resource_id(),
            request.holder().clone(),
            ResourceSourceLeaseToken::new(claim_id, generation),
            instant(expires_at)?,
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
        let now = now_micros(&mut transaction).await?;
        let expires_at = expiry_after(now, request.ttl())?;
        let result = sqlx::query("UPDATE resource_source_leases SET expires_at = ? WHERE workspace_id = ? AND org_id = ? AND resource_id = ? AND claim_id = ? AND generation = ? AND expires_at > ?")
            .bind(expires_at).bind(&request.scope().workspace_id).bind(&request.scope().org_id)
            .bind(request.resource_id().into_bytes().as_slice()).bind(request.token().claim_id().as_bytes().as_slice())
            .bind(encode_counter(request.token().generation().get())?).bind(now)
            .execute(&mut *transaction).await.map_err(statement_error)?;
        if result.rows_affected() != 1 {
            return Err(fenced("resource source lease"));
        }
        let row = sqlx::query("SELECT holder, claim_id, generation, expires_at FROM resource_source_leases WHERE workspace_id = ? AND org_id = ? AND resource_id = ?")
            .bind(&request.scope().workspace_id).bind(&request.scope().org_id).bind(request.resource_id().into_bytes().as_slice())
            .fetch_one(&mut *transaction).await.map_err(statement_error)?;
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
        sqlx::query("UPDATE resource_source_leases SET expires_at = 0 WHERE workspace_id = ? AND org_id = ? AND resource_id = ? AND claim_id = ? AND generation = ?")
            .bind(&request.scope().workspace_id).bind(&request.scope().org_id).bind(request.resource_id().into_bytes().as_slice())
            .bind(request.token().claim_id().as_bytes().as_slice()).bind(encode_counter(request.token().generation().get())?)
            .execute(&mut *transaction).await.map_err(statement_error)?;
        transaction.commit().await.map_err(commit_unknown)?;
        Ok(())
    }
}

fn decode_envelope(row: &SqliteRow) -> Result<EventEnvelope, StorageError> {
    EventEnvelope::try_from_vec(
        u32::try_from(
            row.try_get::<i64, _>("schema_version")
                .map_err(statement_error)?,
        )
        .map_err(|_| corrupt_record())?,
        row.try_get("canonical_payload").map_err(statement_error)?,
    )
    .map_err(|_| corrupt_record())
}

fn decode_event(row: &SqliteRow) -> Result<ResourceEventRecord, StorageError> {
    Ok(ResourceEventRecord::new(
        ResourceEventId::from_bytes(id16(row.try_get("id").map_err(statement_error)?)?),
        SharedResourceId::from_bytes(id16(row.try_get("resource_id").map_err(statement_error)?)?),
        EventOccurrenceNamespace::new(
            row.try_get::<String, _>("occurrence_namespace")
                .map_err(statement_error)?,
        )
        .map_err(|_| corrupt_record())?,
        EventOccurrenceKey::try_from_vec(row.try_get("occurrence_key").map_err(statement_error)?)
            .map_err(|_| corrupt_record())?,
        decode_envelope(row)?,
        ResourceEventAcceptance::new(
            instant(row.try_get("accepted_at").map_err(statement_error)?)?,
            ResourceLeaseGeneration::new(decode_counter(
                row.try_get("source_generation").map_err(statement_error)?,
            )?),
        ),
        ResourceEventState::from_str(&row.try_get::<String, _>("state").map_err(statement_error)?)
            .map_err(|_| corrupt_record())?,
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
            TerminalDeliveryIneligibility::from_str(reason.ok_or_else(corrupt_record)?)
                .map_err(|_| corrupt_record())?,
        ))),
        _ => Err(corrupt_record()),
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
        _ => Err(corrupt_record()),
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

fn decode_claimed_delivery(row: &SqliteRow) -> Result<ClaimedResourceDelivery, StorageError> {
    let generation = ResourceLeaseGeneration::new(decode_counter(
        row.try_get("claim_generation").map_err(statement_error)?,
    )?);
    Ok(ClaimedResourceDelivery::new(
        ResourceDeliveryId::from_bytes(id16(row.try_get("id").map_err(statement_error)?)?),
        ResourceEventId::from_bytes(id16(row.try_get("event_id").map_err(statement_error)?)?),
        ResourceSubscriptionId::from_bytes(id16(
            row.try_get("subscription_id").map_err(statement_error)?,
        )?),
        decode_envelope(row)?,
        ResourceDeliveryClaimToken::from_claim_bytes(
            id16(row.try_get("claim_id").map_err(statement_error)?)?,
            generation,
        ),
    ))
}

fn decode_claimed_handoff(row: &SqliteRow) -> Result<ClaimedResourceHandoff, StorageError> {
    Ok(ClaimedResourceHandoff::new(
        ResourceDeliveryId::from_bytes(id16(row.try_get("delivery_id").map_err(statement_error)?)?),
        ResourceEventId::from_bytes(id16(row.try_get("event_id").map_err(statement_error)?)?),
        ResourceSubscriptionId::from_bytes(id16(
            row.try_get("subscription_id").map_err(statement_error)?,
        )?),
        decode_envelope(row)?,
        ResourceHandoffClaimToken::from_claim_bytes(
            id16(row.try_get("claim_id").map_err(statement_error)?)?,
            ResourceLeaseGeneration::new(decode_counter(
                row.try_get("claim_generation").map_err(statement_error)?,
            )?),
        ),
    ))
}

async fn ensure_handoff(
    transaction: &mut Transaction<'_, Sqlite>,
    scope: &Scope,
    delivery_id: ResourceDeliveryId,
    resource_id: SharedResourceId,
    event_id: ResourceEventId,
    subscription_id: ResourceSubscriptionId,
) -> Result<(), StorageError> {
    sqlx::query("INSERT INTO resource_execution_handoffs (org_id, workspace_id, resource_id, delivery_id, event_id, subscription_id, status, claim_generation) VALUES (?, ?, ?, ?, ?, ?, 'pending', 0) ON CONFLICT (org_id, workspace_id, delivery_id) DO NOTHING")
        .bind(&scope.org_id).bind(&scope.workspace_id).bind(resource_id.into_bytes().as_slice())
        .bind(delivery_id.into_bytes().as_slice()).bind(event_id.into_bytes().as_slice())
        .bind(subscription_id.into_bytes().as_slice())
        .execute(&mut **transaction).await.map_err(statement_error)?;
    let exact: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM resource_execution_handoffs WHERE workspace_id = ? AND org_id = ? AND resource_id = ? AND delivery_id = ? AND event_id = ? AND subscription_id = ?")
        .bind(&scope.workspace_id).bind(&scope.org_id).bind(resource_id.into_bytes().as_slice())
        .bind(delivery_id.into_bytes().as_slice()).bind(event_id.into_bytes().as_slice())
        .bind(subscription_id.into_bytes().as_slice()).fetch_one(&mut **transaction).await.map_err(statement_error)?;
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
impl ResourceEventFanoutStore for SqliteResourceRuntime {
    #[tracing::instrument(skip_all, fields(storage.role = "resource_event_fanout", storage.operation = "accept", resource_id = ?request.resource_id(), source_generation = request.source_token().generation().get()))]
    async fn accept(
        &self,
        request: AcceptResourceEventRequest,
    ) -> Result<AcceptResourceEventOutcome, StorageError> {
        let mut transaction = self.begin_write().await?;
        let now = now_micros(&mut transaction).await?;
        let live_source: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM resource_source_leases WHERE workspace_id = ? AND org_id = ? AND resource_id = ? AND claim_id = ? AND generation = ? AND expires_at > ?")
            .bind(&request.scope().workspace_id).bind(&request.scope().org_id).bind(request.resource_id().into_bytes().as_slice())
            .bind(request.source_token().claim_id().as_bytes().as_slice()).bind(encode_counter(request.source_token().generation().get())?).bind(now)
            .fetch_one(&mut *transaction).await.map_err(statement_error)?;
        if live_source != 1 {
            return Err(fenced("resource source lease"));
        }

        let existing = sqlx::query("SELECT id, resource_id, occurrence_namespace, occurrence_key, schema_version, canonical_payload, accepted_at, source_generation, state FROM resource_events WHERE workspace_id = ? AND org_id = ? AND resource_id = ? AND occurrence_namespace = ? AND occurrence_key = ?")
            .bind(&request.scope().workspace_id).bind(&request.scope().org_id).bind(request.resource_id().into_bytes().as_slice())
            .bind(request.namespace().as_str()).bind(request.occurrence_key_bytes())
            .fetch_optional(&mut *transaction).await.map_err(statement_error)?;
        if let Some(row) = existing {
            let event = decode_event(&row)?;
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

        let event_id = ResourceEventId::from_bytes(*Uuid::new_v4().as_bytes());
        sqlx::query("INSERT INTO resource_events (org_id, workspace_id, resource_id, id, occurrence_namespace, occurrence_key, schema_version, canonical_payload, envelope_digest, accepted_at, source_generation, state) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'pending')")
            .bind(&request.scope().org_id).bind(&request.scope().workspace_id).bind(request.resource_id().into_bytes().as_slice())
            .bind(event_id.into_bytes().as_slice()).bind(request.namespace().as_str()).bind(request.occurrence_key_bytes())
            .bind(i64::from(request.envelope().schema_version())).bind(request.envelope().canonical_payload())
            .bind(request.envelope().digest().as_slice()).bind(now)
            .bind(encode_counter(request.source_token().generation().get())?)
            .execute(&mut *transaction).await.map_err(statement_error)?;
        let inserted = sqlx::query("INSERT INTO resource_deliveries (org_id, workspace_id, resource_id, id, event_id, subscription_id, status, claim_generation) SELECT org_id, workspace_id, resource_id, randomblob(16), ?, id, 'pending', 0 FROM resource_subscriptions WHERE workspace_id = ? AND org_id = ? AND resource_id = ? AND state = 'active'")
            .bind(event_id.into_bytes().as_slice())
            .bind(&request.scope().workspace_id).bind(&request.scope().org_id)
            .bind(request.resource_id().into_bytes().as_slice())
            .execute(&mut *transaction).await.map_err(statement_error)?;
        let delivery_count =
            u32::try_from(inserted.rows_affected()).map_err(|_| invalid_computed_count())?;
        if delivery_count == 0 {
            sqlx::query("UPDATE resource_events SET state = 'complete' WHERE workspace_id = ? AND org_id = ? AND id = ?")
                .bind(&request.scope().workspace_id).bind(&request.scope().org_id)
                .bind(event_id.into_bytes().as_slice()).execute(&mut *transaction).await.map_err(statement_error)?;
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
        sqlx::query("SELECT id, resource_id, occurrence_namespace, occurrence_key, schema_version, canonical_payload, accepted_at, source_generation, state FROM resource_events WHERE workspace_id = ? AND org_id = ? AND id = ?")
            .bind(&scope.workspace_id).bind(&scope.org_id).bind(event_id.into_bytes().as_slice())
            .fetch_optional(&self.pool).await.map_err(statement_error)?.map(|row| decode_event(&row)).transpose()
    }

    #[tracing::instrument(skip_all, fields(storage.role = "resource_event_fanout", storage.operation = "claim_deliveries"))]
    async fn claim_deliveries(
        &self,
        request: ClaimResourceDeliveriesRequest,
    ) -> Result<Vec<ClaimedResourceDelivery>, StorageError> {
        let mut transaction = self.begin_write().await?;
        let now = now_micros(&mut transaction).await?;
        let rows = sqlx::query("SELECT d.id, d.event_id, d.subscription_id, d.claim_generation, e.schema_version, e.canonical_payload FROM resource_deliveries d JOIN resource_events e ON e.workspace_id = d.workspace_id AND e.org_id = d.org_id AND e.id = d.event_id WHERE d.workspace_id = ? AND d.org_id = ? AND d.status = 'pending' AND (d.claim_id IS NULL OR d.claim_expires_at <= ?) ORDER BY d.sequence LIMIT ?")
            .bind(&request.scope().workspace_id).bind(&request.scope().org_id).bind(now).bind(i64::from(request.batch_size().get()))
            .fetch_all(&mut *transaction).await.map_err(statement_error)?;
        let mut prepared = Vec::with_capacity(rows.len());
        for row in rows {
            let next = next_counter(decode_counter(
                row.try_get("claim_generation").map_err(statement_error)?,
            )?)?;
            prepared.push((row, next, Uuid::new_v4()));
        }
        let expires_at = expiry_after(now, request.ttl())?;
        let mut claimed = Vec::with_capacity(prepared.len());
        for (row, generation, claim_id) in prepared {
            let delivery_id =
                ResourceDeliveryId::from_bytes(id16(row.try_get("id").map_err(statement_error)?)?);
            sqlx::query("UPDATE resource_deliveries SET claim_holder = ?, claim_id = ?, claim_generation = ?, claim_expires_at = ? WHERE workspace_id = ? AND org_id = ? AND id = ?")
                .bind(request.holder().as_str()).bind(claim_id.as_bytes().as_slice()).bind(encode_counter(generation)?).bind(expires_at)
                .bind(&request.scope().workspace_id).bind(&request.scope().org_id).bind(delivery_id.into_bytes().as_slice())
                .execute(&mut *transaction).await.map_err(statement_error)?;
            claimed.push(ClaimedResourceDelivery::new(
                delivery_id,
                ResourceEventId::from_bytes(id16(
                    row.try_get("event_id").map_err(statement_error)?,
                )?),
                ResourceSubscriptionId::from_bytes(id16(
                    row.try_get("subscription_id").map_err(statement_error)?,
                )?),
                decode_envelope(&row)?,
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
        let now = now_micros(&mut transaction).await?;
        let expires_at = expiry_after(now, request.ttl())?;
        let updated = sqlx::query("UPDATE resource_deliveries SET claim_expires_at = ? WHERE workspace_id = ? AND org_id = ? AND id = ? AND status = 'pending' AND claim_id = ? AND claim_generation = ? AND claim_expires_at > ?")
            .bind(expires_at).bind(&request.scope().workspace_id).bind(&request.scope().org_id).bind(request.delivery_id().into_bytes().as_slice())
            .bind(request.token().claim_id().as_bytes().as_slice()).bind(encode_counter(request.token().generation().get())?).bind(now)
            .execute(&mut *transaction).await.map_err(statement_error)?;
        if updated.rows_affected() != 1 {
            return Err(fenced("resource delivery"));
        }
        let row = sqlx::query("SELECT d.id, d.event_id, d.subscription_id, d.claim_id, d.claim_generation, e.schema_version, e.canonical_payload FROM resource_deliveries d JOIN resource_events e ON e.workspace_id = d.workspace_id AND e.org_id = d.org_id AND e.id = d.event_id WHERE d.workspace_id = ? AND d.org_id = ? AND d.id = ?")
            .bind(&request.scope().workspace_id).bind(&request.scope().org_id).bind(request.delivery_id().into_bytes().as_slice())
            .fetch_one(&mut *transaction).await.map_err(statement_error)?;
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
        sqlx::query("UPDATE resource_deliveries SET claim_holder = NULL, claim_id = NULL, claim_expires_at = NULL WHERE workspace_id = ? AND org_id = ? AND id = ? AND status = 'pending' AND claim_id = ? AND claim_generation = ?")
            .bind(&request.scope().workspace_id).bind(&request.scope().org_id).bind(request.delivery_id().into_bytes().as_slice())
            .bind(request.token().claim_id().as_bytes().as_slice()).bind(encode_counter(request.token().generation().get())?)
            .execute(&mut *transaction).await.map_err(statement_error)?;
        transaction.commit().await.map_err(commit_unknown)?;
        Ok(())
    }

    #[tracing::instrument(skip_all, fields(storage.role = "resource_event_fanout", storage.operation = "complete_delivery", delivery_id = ?request.delivery_id(), generation = request.token().generation().get()))]
    async fn complete_delivery(
        &self,
        request: CompleteResourceDeliveryRequest,
    ) -> Result<CompleteResourceDeliveryOutcome, StorageError> {
        let mut transaction = self.begin_write().await?;
        let now = now_micros(&mut transaction).await?;
        let row = sqlx::query("SELECT resource_id, event_id, subscription_id, status, terminal_reason, requested_status, requested_terminal_reason, claim_id, claim_generation, claim_expires_at, terminal_claim_id, terminal_claim_generation FROM resource_deliveries WHERE workspace_id = ? AND org_id = ? AND id = ?")
            .bind(&request.scope().workspace_id).bind(&request.scope().org_id).bind(request.delivery_id().into_bytes().as_slice())
            .fetch_optional(&mut *transaction).await.map_err(statement_error)?.ok_or_else(|| fenced("resource delivery"))?;
        let event_id =
            ResourceEventId::from_bytes(id16(row.try_get("event_id").map_err(statement_error)?)?);
        let resource_id = SharedResourceId::from_bytes(id16(
            row.try_get("resource_id").map_err(statement_error)?,
        )?);
        let subscription_id = ResourceSubscriptionId::from_bytes(id16(
            row.try_get("subscription_id").map_err(statement_error)?,
        )?);
        let subscription_state: String = sqlx::query_scalar("SELECT state FROM resource_subscriptions WHERE workspace_id = ? AND org_id = ? AND resource_id = ? AND id = ?")
            .bind(&request.scope().workspace_id).bind(&request.scope().org_id).bind(resource_id.into_bytes().as_slice()).bind(subscription_id.into_bytes().as_slice())
            .fetch_one(&mut *transaction).await.map_err(statement_error)?;
        let effective_completion = effective_completion(request.completion(), &subscription_state)?;
        let status: String = row.try_get("status").map_err(statement_error)?;
        let reason: Option<String> = row.try_get("terminal_reason").map_err(statement_error)?;
        if let Some(completion) = persisted_completion(&status, reason.as_deref())? {
            let requested_status: Option<String> =
                row.try_get("requested_status").map_err(statement_error)?;
            let requested_reason: Option<String> = row
                .try_get("requested_terminal_reason")
                .map_err(statement_error)?;
            let requested_completion = persisted_completion(
                requested_status.as_deref().ok_or_else(corrupt_record)?,
                requested_reason.as_deref(),
            )?
            .ok_or_else(corrupt_record)?;
            let terminal_claim_id = id16(
                row.try_get::<Option<Vec<u8>>, _>("terminal_claim_id")
                    .map_err(statement_error)?
                    .ok_or_else(corrupt_record)?,
            )?;
            let terminal_generation = decode_counter(
                row.try_get::<Option<i64>, _>("terminal_claim_generation")
                    .map_err(statement_error)?
                    .ok_or_else(corrupt_record)?,
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
            let event_state: String = sqlx::query_scalar("SELECT state FROM resource_events WHERE workspace_id = ? AND org_id = ? AND id = ?")
                .bind(&request.scope().workspace_id).bind(&request.scope().org_id).bind(event_id.into_bytes().as_slice())
                .fetch_one(&mut *transaction).await.map_err(statement_error)?;
            transaction.commit().await.map_err(commit_unknown)?;
            let outcome = CompleteResourceDeliveryOutcome::AlreadyCompleted {
                completion,
                event_is_terminal: event_state == "complete",
            };
            trace_completion_outcome(outcome);
            return Ok(outcome);
        }
        let claim_id = id16(
            row.try_get::<Option<Vec<u8>>, _>("claim_id")
                .map_err(statement_error)?
                .ok_or_else(|| fenced("resource delivery"))?,
        )?;
        let claim_generation = decode_counter(
            row.try_get::<i64, _>("claim_generation")
                .map_err(statement_error)?,
        )?;
        let claim_expires_at: Option<i64> =
            row.try_get("claim_expires_at").map_err(statement_error)?;
        if claim_id != *request.token().claim_id().as_bytes()
            || claim_generation != request.token().generation().get()
            || claim_expires_at.is_none_or(|deadline| now >= deadline)
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
        sqlx::query("UPDATE resource_deliveries SET status = ?, terminal_reason = ?, requested_status = ?, requested_terminal_reason = ?, terminal_claim_id = claim_id, terminal_claim_generation = claim_generation, claim_holder = NULL, claim_id = NULL, claim_expires_at = NULL WHERE workspace_id = ? AND org_id = ? AND id = ?")
            .bind(next_status).bind(terminal_reason).bind(requested_status).bind(requested_terminal_reason).bind(&request.scope().workspace_id).bind(&request.scope().org_id).bind(request.delivery_id().into_bytes().as_slice())
            .execute(&mut *transaction).await.map_err(statement_error)?;
        let has_pending: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM resource_deliveries WHERE workspace_id = ? AND org_id = ? AND event_id = ? AND status = 'pending')")
            .bind(&request.scope().workspace_id).bind(&request.scope().org_id).bind(event_id.into_bytes().as_slice())
            .fetch_one(&mut *transaction).await.map_err(statement_error)?;
        let event_became_terminal = !has_pending;
        if event_became_terminal {
            sqlx::query("UPDATE resource_events SET state = 'complete' WHERE workspace_id = ? AND org_id = ? AND id = ?")
                .bind(&request.scope().workspace_id).bind(&request.scope().org_id).bind(event_id.into_bytes().as_slice())
                .execute(&mut *transaction).await.map_err(statement_error)?;
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
impl ResourceExecutionHandoffStore for SqliteResourceRuntime {
    #[tracing::instrument(skip_all, fields(storage.role = "resource_execution_handoff", storage.operation = "claim"))]
    async fn claim_handoffs(
        &self,
        request: ClaimResourceHandoffsRequest,
    ) -> Result<Vec<ClaimedResourceHandoff>, StorageError> {
        let mut transaction = self.begin_write().await?;
        let now = now_micros(&mut transaction).await?;
        let rows = sqlx::query("SELECT h.delivery_id, h.event_id, h.subscription_id, h.claim_generation, e.schema_version, e.canonical_payload FROM resource_execution_handoffs h JOIN resource_events e ON e.workspace_id = h.workspace_id AND e.org_id = h.org_id AND e.resource_id = h.resource_id AND e.id = h.event_id WHERE h.workspace_id = ? AND h.org_id = ? AND h.status = 'pending' AND (h.claim_id IS NULL OR h.claim_expires_at <= ?) ORDER BY h.sequence LIMIT ?")
            .bind(&request.scope().workspace_id).bind(&request.scope().org_id).bind(now).bind(i64::from(request.batch_size().get()))
            .fetch_all(&mut *transaction).await.map_err(statement_error)?;
        let mut prepared = Vec::with_capacity(rows.len());
        for row in rows {
            let generation = next_counter(decode_counter(
                row.try_get("claim_generation").map_err(statement_error)?,
            )?)?;
            prepared.push((row, generation, Uuid::new_v4()));
        }
        let expires_at = expiry_after(now, request.ttl())?;
        let mut claims = Vec::with_capacity(prepared.len());
        for (row, generation, claim_id) in prepared {
            let delivery_id = ResourceDeliveryId::from_bytes(id16(
                row.try_get("delivery_id").map_err(statement_error)?,
            )?);
            sqlx::query("UPDATE resource_execution_handoffs SET claim_holder = ?, claim_id = ?, claim_generation = ?, claim_expires_at = ? WHERE workspace_id = ? AND org_id = ? AND delivery_id = ?")
                .bind(request.holder().as_str()).bind(claim_id.as_bytes().as_slice()).bind(encode_counter(generation)?).bind(expires_at)
                .bind(&request.scope().workspace_id).bind(&request.scope().org_id).bind(delivery_id.into_bytes().as_slice())
                .execute(&mut *transaction).await.map_err(statement_error)?;
            claims.push(ClaimedResourceHandoff::new(
                delivery_id,
                ResourceEventId::from_bytes(id16(
                    row.try_get("event_id").map_err(statement_error)?,
                )?),
                ResourceSubscriptionId::from_bytes(id16(
                    row.try_get("subscription_id").map_err(statement_error)?,
                )?),
                decode_envelope(&row)?,
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
        let now = now_micros(&mut transaction).await?;
        let expires_at = expiry_after(now, request.ttl())?;
        let updated = sqlx::query("UPDATE resource_execution_handoffs SET claim_expires_at = ? WHERE workspace_id = ? AND org_id = ? AND delivery_id = ? AND status = 'pending' AND claim_id = ? AND claim_generation = ? AND claim_expires_at > ?")
            .bind(expires_at).bind(&request.scope().workspace_id).bind(&request.scope().org_id).bind(request.delivery_id().into_bytes().as_slice())
            .bind(request.token().claim_id().as_bytes().as_slice()).bind(encode_counter(request.token().generation().get())?).bind(now)
            .execute(&mut *transaction).await.map_err(statement_error)?;
        if updated.rows_affected() != 1 {
            return Err(fenced("resource execution handoff"));
        }
        let row = sqlx::query("SELECT h.delivery_id, h.event_id, h.subscription_id, h.claim_id, h.claim_generation, e.schema_version, e.canonical_payload FROM resource_execution_handoffs h JOIN resource_events e ON e.workspace_id = h.workspace_id AND e.org_id = h.org_id AND e.resource_id = h.resource_id AND e.id = h.event_id WHERE h.workspace_id = ? AND h.org_id = ? AND h.delivery_id = ?")
            .bind(&request.scope().workspace_id).bind(&request.scope().org_id).bind(request.delivery_id().into_bytes().as_slice())
            .fetch_one(&mut *transaction).await.map_err(statement_error)?;
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
        sqlx::query("UPDATE resource_execution_handoffs SET claim_holder = NULL, claim_id = NULL, claim_expires_at = NULL WHERE workspace_id = ? AND org_id = ? AND delivery_id = ? AND status = 'pending' AND claim_id = ? AND claim_generation = ?")
            .bind(&request.scope().workspace_id).bind(&request.scope().org_id).bind(request.delivery_id().into_bytes().as_slice())
            .bind(request.token().claim_id().as_bytes().as_slice()).bind(encode_counter(request.token().generation().get())?)
            .execute(&mut *transaction).await.map_err(statement_error)?;
        transaction.commit().await.map_err(commit_unknown)?;
        Ok(())
    }

    #[tracing::instrument(skip_all, fields(storage.role = "resource_execution_handoff", storage.operation = "acknowledge", delivery_id = ?request.delivery_id(), generation = request.token().generation().get()))]
    async fn acknowledge_handoff(
        &self,
        request: ResourceHandoffClaimRequest,
    ) -> Result<AcknowledgeResourceHandoffOutcome, StorageError> {
        let mut transaction = self.begin_write().await?;
        let now = now_micros(&mut transaction).await?;
        let row = sqlx::query("SELECT status, claim_id, claim_generation, claim_expires_at, terminal_claim_id, terminal_claim_generation FROM resource_execution_handoffs WHERE workspace_id = ? AND org_id = ? AND delivery_id = ?")
            .bind(&request.scope().workspace_id).bind(&request.scope().org_id).bind(request.delivery_id().into_bytes().as_slice())
            .fetch_optional(&mut *transaction).await.map_err(statement_error)?.ok_or_else(|| fenced("resource execution handoff"))?;
        let status: String = row.try_get("status").map_err(statement_error)?;
        if status == "acknowledged" {
            let terminal_id = id16(
                row.try_get::<Option<Vec<u8>>, _>("terminal_claim_id")
                    .map_err(statement_error)?
                    .ok_or_else(corrupt_record)?,
            )?;
            let terminal_generation = decode_counter(
                row.try_get::<Option<i64>, _>("terminal_claim_generation")
                    .map_err(statement_error)?
                    .ok_or_else(corrupt_record)?,
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
                .map_err(statement_error)?
                .ok_or_else(|| fenced("resource execution handoff"))?,
        )?;
        let generation = decode_counter(row.try_get("claim_generation").map_err(statement_error)?)?;
        let deadline: Option<i64> = row.try_get("claim_expires_at").map_err(statement_error)?;
        if claim_id != *request.token().claim_id().as_bytes()
            || generation != request.token().generation().get()
            || deadline.is_none_or(|value| now >= value)
        {
            return Err(fenced("resource execution handoff"));
        }
        sqlx::query("UPDATE resource_execution_handoffs SET status = 'acknowledged', terminal_claim_id = claim_id, terminal_claim_generation = claim_generation, claim_holder = NULL, claim_id = NULL, claim_expires_at = NULL WHERE workspace_id = ? AND org_id = ? AND delivery_id = ?")
            .bind(&request.scope().workspace_id).bind(&request.scope().org_id).bind(request.delivery_id().into_bytes().as_slice()).execute(&mut *transaction).await.map_err(statement_error)?;
        transaction.commit().await.map_err(commit_unknown)?;
        tracing::debug!(storage.outcome = "acknowledged");
        Ok(AcknowledgeResourceHandoffOutcome::Acknowledged)
    }
}

#[async_trait::async_trait]
impl ResourceRuntimeRecovery for SqliteResourceRuntime {
    #[tracing::instrument(skip_all, fields(storage.role = "resource_runtime_recovery", storage.operation = "claim_deliveries"))]
    async fn claim_deliveries_globally(
        &self,
        request: ClaimResourceRuntimeWorkRequest,
    ) -> Result<Vec<ScopedClaimedResourceDelivery>, StorageError> {
        let mut transaction = self.begin_write().await?;
        let now = now_micros(&mut transaction).await?;
        let expires_at = expiry_after(now, request.ttl())?;
        let rows = sqlx::query("SELECT d.workspace_id, d.org_id, d.id, d.event_id, d.subscription_id, d.claim_generation, e.schema_version, e.canonical_payload FROM resource_deliveries d JOIN resource_events e ON e.workspace_id = d.workspace_id AND e.org_id = d.org_id AND e.id = d.event_id WHERE d.status = 'pending' AND (d.claim_id IS NULL OR d.claim_expires_at <= ?) ORDER BY d.sequence LIMIT ?")
            .bind(now).bind(i64::from(request.batch_size().get()))
            .fetch_all(&mut *transaction).await.map_err(statement_error)?;
        let mut claimed = Vec::with_capacity(rows.len());
        for row in rows {
            let scope = Scope::new(
                row.try_get::<String, _>("workspace_id")
                    .map_err(statement_error)?,
                row.try_get::<String, _>("org_id")
                    .map_err(statement_error)?,
            );
            let delivery_id =
                ResourceDeliveryId::from_bytes(id16(row.try_get("id").map_err(statement_error)?)?);
            let generation = next_counter(decode_counter(
                row.try_get("claim_generation").map_err(statement_error)?,
            )?)?;
            let claim_id = Uuid::new_v4();
            sqlx::query("UPDATE resource_deliveries SET claim_holder = ?, claim_id = ?, claim_generation = ?, claim_expires_at = ? WHERE workspace_id = ? AND org_id = ? AND id = ?")
                .bind(request.holder().as_str()).bind(claim_id.as_bytes().as_slice()).bind(encode_counter(generation)?).bind(expires_at)
                .bind(&scope.workspace_id).bind(&scope.org_id).bind(delivery_id.into_bytes().as_slice())
                .execute(&mut *transaction).await.map_err(statement_error)?;
            claimed.push(ScopedClaimedResourceDelivery::new(
                scope,
                ClaimedResourceDelivery::new(
                    delivery_id,
                    ResourceEventId::from_bytes(id16(
                        row.try_get("event_id").map_err(statement_error)?,
                    )?),
                    ResourceSubscriptionId::from_bytes(id16(
                        row.try_get("subscription_id").map_err(statement_error)?,
                    )?),
                    decode_envelope(&row)?,
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
        let mut transaction = self.begin_write().await?;
        let now = now_micros(&mut transaction).await?;
        let expires_at = expiry_after(now, request.ttl())?;
        let rows = sqlx::query("SELECT h.workspace_id, h.org_id, h.delivery_id, h.event_id, h.subscription_id, h.claim_generation, e.schema_version, e.canonical_payload FROM resource_execution_handoffs h JOIN resource_events e ON e.workspace_id = h.workspace_id AND e.org_id = h.org_id AND e.resource_id = h.resource_id AND e.id = h.event_id WHERE h.status = 'pending' AND (h.claim_id IS NULL OR h.claim_expires_at <= ?) ORDER BY h.sequence LIMIT ?")
            .bind(now).bind(i64::from(request.batch_size().get()))
            .fetch_all(&mut *transaction).await.map_err(statement_error)?;
        let mut claimed = Vec::with_capacity(rows.len());
        for row in rows {
            let scope = Scope::new(
                row.try_get::<String, _>("workspace_id")
                    .map_err(statement_error)?,
                row.try_get::<String, _>("org_id")
                    .map_err(statement_error)?,
            );
            let delivery_id = ResourceDeliveryId::from_bytes(id16(
                row.try_get("delivery_id").map_err(statement_error)?,
            )?);
            let generation = next_counter(decode_counter(
                row.try_get("claim_generation").map_err(statement_error)?,
            )?)?;
            let claim_id = Uuid::new_v4();
            sqlx::query("UPDATE resource_execution_handoffs SET claim_holder = ?, claim_id = ?, claim_generation = ?, claim_expires_at = ? WHERE workspace_id = ? AND org_id = ? AND delivery_id = ?")
                .bind(request.holder().as_str()).bind(claim_id.as_bytes().as_slice()).bind(encode_counter(generation)?).bind(expires_at)
                .bind(&scope.workspace_id).bind(&scope.org_id).bind(delivery_id.into_bytes().as_slice())
                .execute(&mut *transaction).await.map_err(statement_error)?;
            claimed.push(ScopedClaimedResourceHandoff::new(
                scope,
                ClaimedResourceHandoff::new(
                    delivery_id,
                    ResourceEventId::from_bytes(id16(
                        row.try_get("event_id").map_err(statement_error)?,
                    )?),
                    ResourceSubscriptionId::from_bytes(id16(
                        row.try_get("subscription_id").map_err(statement_error)?,
                    )?),
                    decode_envelope(&row)?,
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

    use super::next_counter;
    use nebula_storage_port::StorageError;

    #[test]
    fn a_counter_past_the_stored_range_is_exhausted() {
        assert_eq!(next_counter(1).ok(), Some(2));
        let last = u64::try_from(i64::MAX).expect("i64::MAX fits u64");
        assert_matches!(next_counter(last), Err(StorageError::Internal(_)));
    }
}
