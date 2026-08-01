//! PostgreSQL exact plan/flavor catalog over ordered migration 0041.
//!
//! Every operation runs inside one transaction that locks the rows it decides
//! on with `FOR UPDATE`, so the plan row, the flavor row, and the
//! authoritative reference rows are read and written in a single linearized
//! operation across processes. The pair is therefore inserted atomically, and a
//! drain or a guarded delete rechecks its blockers under the same lock that
//! applies the lifecycle change.
//!
//! Exact load never selects a latest or closest revision, follows a redirect,
//! or recompiles a plan: it reads the requested plan identity, rejects a plan
//! pinned to a different worker flavor, and composes the pair from the pinned
//! flavor row.
//!
//! Driver detail and record bytes never cross the port boundary. A failure
//! before commit is [`RevisionCatalogError::Unavailable`] (the operation
//! definitely did not commit); a failed commit is
//! [`RevisionCatalogError::OutcomeUnknown`], because a lost commit
//! acknowledgement leaves the caller unable to prove whether the write landed.

use nebula_core::{ExecutablePlanRevisionId, WorkerFlavorRevisionId};
use nebula_storage_port::{
    BeginDrainOutcome, ExecutablePlanRecordFormat, PlanFlavorCatalog, PlanFlavorCatalogAdmin,
    PlanFlavorCatalogWriter, PlanFlavorRevisionIds, PlanFlavorRevisionRecord,
    PlanFlavorRevisionTarget, RevisionCatalogError, RevisionInsertOutcome, RevisionReferenceCounts,
};
use sqlx::{PgPool, Postgres, Row, Transaction};

use nebula_metrics::revision_catalog_operation;

use crate::revision_catalog::{
    ArtifactLifecycle, EXECUTABLE_PLAN_GRAPH_V1_JSON, RevisionCatalogMetrics, StoredExecutablePlan,
    StoredWorkerFlavor, WORKER_FLAVOR_V1_JSON, compose_pair, decode_executable_plan_row,
    decode_worker_flavor_row, delete_label, deleted_for, drain_label, draining_for,
    flavor_records_match, insert_label, load_label, plan_content_matches, unavailable_for,
    validate_pair_recorded_form,
};

/// PostgreSQL-backed exact executable-plan and worker-flavor catalog.
///
/// Wrap a pool whose schema was installed via [`super::init_schema`].
#[derive(Clone, Debug)]
pub struct PgPlanFlavorCatalog {
    pool: PgPool,
    metrics: RevisionCatalogMetrics,
}

impl PgPlanFlavorCatalog {
    /// Wrap an existing pool and bind catalog counters to a shared registry.
    ///
    /// The caller installs the port schema (see [`super::init_schema`]). The
    /// registry is required rather than optional so a composition root cannot
    /// wire a deployment backend whose conflict and unknown-outcome counts are
    /// invisible to a scraper.
    #[must_use]
    pub fn new(pool: PgPool, metrics: &nebula_metrics::MetricsRegistry) -> Self {
        Self {
            pool,
            metrics: RevisionCatalogMetrics::new(metrics, "postgres"),
        }
    }

    /// Open the transaction every catalog operation runs in.
    async fn begin(&self) -> Result<Transaction<'_, Postgres>, RevisionCatalogError> {
        self.pool.begin().await.map_err(driver_did_not_commit)
    }
}

/// A driver failure reached before commit definitely did not commit.
fn driver_did_not_commit(_error: sqlx::Error) -> RevisionCatalogError {
    RevisionCatalogError::Unavailable
}

/// A failed commit leaves the caller unable to prove whether the write landed.
fn commit_outcome_unknown(_error: sqlx::Error) -> RevisionCatalogError {
    RevisionCatalogError::OutcomeUnknown
}

/// Whether a row read should take a row lock for a decision that will write.
///
/// Exact load decides nothing it then writes, so it reads without blocking a
/// concurrent drain; every mutating path locks the rows it decides on so a
/// racing writer cannot invalidate the decision between the read and the write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RowLock {
    /// Read the row without taking a lock.
    Shared,
    /// Take the row's exclusive lock for the rest of the transaction.
    Exclusive,
}

/// Advisory-lock namespace for an executable-plan identity.
const PLAN_IDENTITY_LOCK_SALT: i64 = 0x504c_414e_5f52_4556;

/// Advisory-lock namespace for a worker-flavor identity.
const FLAVOR_IDENTITY_LOCK_SALT: i64 = 0x464c_4156_5f52_4556;

/// Serialize concurrent operations against one immutable identity, including
/// the identity that does not exist yet.
///
/// `SELECT … FOR UPDATE` cannot lock a row that is absent, so two processes
/// installing the same new revision would both decide "absent" and one would
/// lose on the unique index — turning an idempotent install into a driver
/// error. A transaction-scoped advisory lock keyed on the identity closes that
/// gap. Revision identifiers are content digests, so the leading eight bytes
/// are well distributed; a key collision between unrelated identities costs
/// only serialization, never correctness.
async fn lock_identity_key(
    tx: &mut Transaction<'_, Postgres>,
    salt: i64,
    id_bytes: &[u8; 32],
) -> Result<(), RevisionCatalogError> {
    let mut leading = [0_u8; 8];
    leading.copy_from_slice(&id_bytes[..8]);
    let key = i64::from_be_bytes(leading) ^ salt;
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(key)
        .execute(&mut **tx)
        .await
        .map(|_locked| ())
        .map_err(driver_did_not_commit)
}

/// Read one durable executable-plan row inside the caller's transaction.
async fn load_plan_row(
    tx: &mut Transaction<'_, Postgres>,
    plan_id: ExecutablePlanRevisionId,
    lock: RowLock,
) -> Result<Option<StoredExecutablePlan>, RevisionCatalogError> {
    let statement = match lock {
        RowLock::Shared => {
            "SELECT worker_flavor_id, record_format, lifecycle, record_bytes \
             FROM port_executable_plan_revisions \
             WHERE executable_plan_id = $1"
        },
        RowLock::Exclusive => {
            "SELECT worker_flavor_id, record_format, lifecycle, record_bytes \
             FROM port_executable_plan_revisions \
             WHERE executable_plan_id = $1 FOR UPDATE"
        },
    };
    let row = sqlx::query(statement)
        .bind(plan_id.as_bytes().as_slice())
        .fetch_optional(&mut **tx)
        .await
        .map_err(driver_did_not_commit)?;

    let Some(row) = row else {
        return Ok(None);
    };
    let target = PlanFlavorRevisionTarget::ExecutablePlan(plan_id);
    let corrupt = |_column: sqlx::Error| RevisionCatalogError::CorruptRecord { target };
    let worker_flavor_id: Vec<u8> = row.try_get("worker_flavor_id").map_err(corrupt)?;
    let record_format: String = row.try_get("record_format").map_err(corrupt)?;
    let lifecycle: String = row.try_get("lifecycle").map_err(corrupt)?;
    let record_bytes: Option<Vec<u8>> = row.try_get("record_bytes").map_err(corrupt)?;

    decode_executable_plan_row(
        plan_id,
        worker_flavor_id,
        &lifecycle,
        &record_format,
        record_bytes,
    )
    .map(Some)
}

/// Read one durable worker-flavor row inside the caller's transaction.
async fn load_flavor_row(
    tx: &mut Transaction<'_, Postgres>,
    worker_flavor_id: WorkerFlavorRevisionId,
    lock: RowLock,
) -> Result<Option<StoredWorkerFlavor>, RevisionCatalogError> {
    let statement = match lock {
        RowLock::Shared => {
            "SELECT record_format, lifecycle, record_bytes \
             FROM port_worker_flavor_revisions WHERE worker_flavor_id = $1"
        },
        RowLock::Exclusive => {
            "SELECT record_format, lifecycle, record_bytes \
             FROM port_worker_flavor_revisions WHERE worker_flavor_id = $1 FOR UPDATE"
        },
    };
    let row = sqlx::query(statement)
        .bind(worker_flavor_id.as_bytes().as_slice())
        .fetch_optional(&mut **tx)
        .await
        .map_err(driver_did_not_commit)?;

    let Some(row) = row else {
        return Ok(None);
    };
    let target = PlanFlavorRevisionTarget::WorkerFlavor(worker_flavor_id);
    let corrupt = |_column: sqlx::Error| RevisionCatalogError::CorruptRecord { target };
    let record_format: String = row.try_get("record_format").map_err(corrupt)?;
    let lifecycle: String = row.try_get("lifecycle").map_err(corrupt)?;
    let record_bytes: Option<Vec<u8>> = row.try_get("record_bytes").map_err(corrupt)?;

    decode_worker_flavor_row(worker_flavor_id, &lifecycle, &record_format, record_bytes).map(Some)
}

/// Project the authoritative reference rows that block `target`'s deletion.
///
/// Counts are always derived from reference rows; the schema carries no
/// mutable counter that could drift from them. A rollback window whose
/// deadline has arrived no longer blocks, so equality with `now_ms` is
/// expired.
async fn reference_counts(
    tx: &mut Transaction<'_, Postgres>,
    target: PlanFlavorRevisionTarget,
    now_ms: i64,
) -> Result<RevisionReferenceCounts, RevisionCatalogError> {
    let query = match target {
        PlanFlavorRevisionTarget::ExecutablePlan(plan_id) => sqlx::query(
            "SELECT \
             COUNT(*) FILTER (WHERE reference_state = 'live') AS live_executions, \
             COUNT(*) FILTER (WHERE reference_state = 'rollback' AND retain_until_ms > $1) \
                 AS rollback_windows \
             FROM port_execution_revision_refs WHERE executable_plan_id = $2",
        )
        .bind(now_ms)
        .bind(plan_id.as_bytes().as_slice()),
        PlanFlavorRevisionTarget::WorkerFlavor(worker_flavor_id) => sqlx::query(
            "SELECT \
             COUNT(*) FILTER (WHERE reference_state = 'live') AS live_executions, \
             COUNT(*) FILTER (WHERE reference_state = 'rollback' AND retain_until_ms > $1) \
                 AS rollback_windows \
             FROM port_execution_revision_refs WHERE worker_flavor_id = $2",
        )
        .bind(now_ms)
        .bind(worker_flavor_id.as_bytes().as_slice()),
    };

    let row = query
        .fetch_one(&mut **tx)
        .await
        .map_err(driver_did_not_commit)?;
    let corrupt = |_column: sqlx::Error| RevisionCatalogError::CorruptRecord { target };
    let live_executions: i64 = row.try_get("live_executions").map_err(corrupt)?;
    let rollback_windows: i64 = row.try_get("rollback_windows").map_err(corrupt)?;

    Ok(RevisionReferenceCounts::new(
        u64::try_from(live_executions).unwrap_or_default(),
        u64::try_from(rollback_windows).unwrap_or_default(),
    ))
}

/// One tombstone-aware lifecycle probe taken under the row's exclusive lock.
struct LockedIdentity {
    lifecycle: ArtifactLifecycle,
    payload_is_cleared: bool,
}

/// Read the durable lifecycle of one immutable identity under its row lock.
async fn lock_identity(
    tx: &mut Transaction<'_, Postgres>,
    target: PlanFlavorRevisionTarget,
) -> Result<Option<LockedIdentity>, RevisionCatalogError> {
    let query = match target {
        PlanFlavorRevisionTarget::ExecutablePlan(plan_id) => sqlx::query(
            "SELECT lifecycle, record_bytes IS NULL AS cleared \
             FROM port_executable_plan_revisions \
             WHERE executable_plan_id = $1 FOR UPDATE",
        )
        .bind(plan_id.as_bytes().as_slice()),
        PlanFlavorRevisionTarget::WorkerFlavor(worker_flavor_id) => sqlx::query(
            "SELECT lifecycle, record_bytes IS NULL AS cleared \
             FROM port_worker_flavor_revisions \
             WHERE worker_flavor_id = $1 FOR UPDATE",
        )
        .bind(worker_flavor_id.as_bytes().as_slice()),
    };

    let Some(row) = query
        .fetch_optional(&mut **tx)
        .await
        .map_err(driver_did_not_commit)?
    else {
        return Ok(None);
    };
    let corrupt = |_column: sqlx::Error| RevisionCatalogError::CorruptRecord { target };
    let lifecycle: String = row.try_get("lifecycle").map_err(corrupt)?;
    let payload_is_cleared: bool = row.try_get("cleared").map_err(corrupt)?;
    let lifecycle = ArtifactLifecycle::from_text(&lifecycle)
        .ok_or(RevisionCatalogError::CorruptRecord { target })?;

    Ok(Some(LockedIdentity {
        lifecycle,
        payload_is_cleared,
    }))
}

async fn insert_locked(
    tx: &mut Transaction<'_, Postgres>,
    record: &PlanFlavorRevisionRecord,
) -> Result<RevisionInsertOutcome, RevisionCatalogError> {
    validate_pair_recorded_form(record)?;

    let ids = record.ids();
    let plan_target = PlanFlavorRevisionTarget::ExecutablePlan(ids.plan());
    let flavor_target = PlanFlavorRevisionTarget::WorkerFlavor(ids.worker_flavor());

    // Lock the flavor before the plan, matching the write order the foreign
    // key forces, so two concurrent inserts of the same pair cannot deadlock by
    // taking the two identities in opposite orders.
    lock_identity_key(
        tx,
        FLAVOR_IDENTITY_LOCK_SALT,
        ids.worker_flavor().as_bytes(),
    )
    .await?;
    lock_identity_key(tx, PLAN_IDENTITY_LOCK_SALT, ids.plan().as_bytes()).await?;

    // Read plan-then-flavor, matching the order the rejections are decided in,
    // so a database holding two corrupt rows names the same identity here as
    // the SQLite adapter and the in-memory reference model do.
    let stored_plan = load_plan_row(tx, ids.plan(), RowLock::Exclusive).await?;
    let stored_flavor = load_flavor_row(tx, ids.worker_flavor(), RowLock::Exclusive).await?;

    let existing_plan_matches = match stored_plan.as_ref() {
        None => false,
        Some(plan) if plan.lifecycle == ArtifactLifecycle::Deleted => {
            return Err(deleted_for(plan_target));
        },
        Some(plan) => {
            let stored_bytes =
                plan.plan_bytes
                    .as_ref()
                    .ok_or(RevisionCatalogError::CorruptRecord {
                        target: plan_target,
                    })?;
            if plan_content_matches(
                PlanFlavorRevisionIds::new(ids.plan(), plan.worker_flavor_id),
                ExecutablePlanRecordFormat::GraphV1Json,
                stored_bytes.as_bytes(),
                record,
            ) {
                true
            } else {
                return Err(RevisionCatalogError::ContentConflict {
                    target: plan_target,
                });
            }
        },
    };

    let existing_flavor_matches = match stored_flavor.as_ref() {
        None => false,
        Some(flavor) if flavor.lifecycle == ArtifactLifecycle::Deleted => {
            return Err(deleted_for(flavor_target));
        },
        Some(flavor) => {
            let stored_record =
                flavor
                    .record
                    .as_ref()
                    .ok_or(RevisionCatalogError::CorruptRecord {
                        target: flavor_target,
                    })?;
            if flavor_records_match(stored_record, record.worker_flavor()) {
                true
            } else {
                return Err(RevisionCatalogError::ContentConflict {
                    target: flavor_target,
                });
            }
        },
    };

    // A plan whose pinned flavor row is absent is durable corruption, not an
    // insert this call may heal by writing the flavor half underneath it.
    if existing_plan_matches && !existing_flavor_matches {
        return Err(RevisionCatalogError::CorruptRecord {
            target: flavor_target,
        });
    }

    // Draining is a property of the stored artifact, not of whether this
    // caller happens to have inserted this pair before, so an idempotent retry
    // learns that the revision is being retired.
    if stored_plan
        .as_ref()
        .is_some_and(|plan| plan.lifecycle == ArtifactLifecycle::Draining)
    {
        return Err(draining_for(plan_target));
    }
    if stored_flavor
        .as_ref()
        .is_some_and(|flavor| flavor.lifecycle == ArtifactLifecycle::Draining)
    {
        return Err(draining_for(flavor_target));
    }

    if existing_plan_matches && existing_flavor_matches {
        return Ok(RevisionInsertOutcome::AlreadyPresent);
    }

    // The plan row's foreign key requires its flavor row to exist first.
    if !existing_flavor_matches {
        sqlx::query(
            "INSERT INTO port_worker_flavor_revisions \
             (worker_flavor_id, record_format, lifecycle, record_bytes) \
             VALUES ($1, $2, $3, $4)",
        )
        .bind(ids.worker_flavor().as_bytes().as_slice())
        .bind(WORKER_FLAVOR_V1_JSON)
        .bind(ArtifactLifecycle::Active.as_str())
        .bind(record.worker_flavor().bytes())
        .execute(&mut **tx)
        .await
        .map_err(driver_did_not_commit)?;
    }
    if !existing_plan_matches {
        sqlx::query(
            "INSERT INTO port_executable_plan_revisions \
             (executable_plan_id, worker_flavor_id, record_format, lifecycle, record_bytes) \
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(ids.plan().as_bytes().as_slice())
        .bind(ids.worker_flavor().as_bytes().as_slice())
        .bind(EXECUTABLE_PLAN_GRAPH_V1_JSON)
        .bind(ArtifactLifecycle::Active.as_str())
        .bind(record.plan_bytes())
        .execute(&mut **tx)
        .await
        .map_err(driver_did_not_commit)?;
    }

    Ok(RevisionInsertOutcome::Inserted)
}

async fn load_exact_locked(
    tx: &mut Transaction<'_, Postgres>,
    ids: PlanFlavorRevisionIds,
) -> Result<PlanFlavorRevisionRecord, RevisionCatalogError> {
    let plan_target = PlanFlavorRevisionTarget::ExecutablePlan(ids.plan());

    let plan = load_plan_row(tx, ids.plan(), RowLock::Shared)
        .await?
        .ok_or_else(|| unavailable_for(plan_target))?;
    if plan.lifecycle == ArtifactLifecycle::Deleted {
        return Err(deleted_for(plan_target));
    }
    if plan.worker_flavor_id != ids.worker_flavor() {
        return Err(RevisionCatalogError::PlanFlavorMismatch {
            requested: ids,
            stored_worker_flavor_id: plan.worker_flavor_id,
        });
    }

    let flavor_target = PlanFlavorRevisionTarget::WorkerFlavor(plan.worker_flavor_id);
    let flavor = load_flavor_row(tx, plan.worker_flavor_id, RowLock::Shared)
        .await?
        .ok_or_else(|| unavailable_for(flavor_target))?;
    if flavor.lifecycle == ArtifactLifecycle::Deleted {
        return Err(deleted_for(flavor_target));
    }

    compose_pair(ids.plan(), &plan, &flavor)
}

async fn begin_drain_locked(
    tx: &mut Transaction<'_, Postgres>,
    target: PlanFlavorRevisionTarget,
    now_ms: i64,
) -> Result<BeginDrainOutcome, RevisionCatalogError> {
    let identity = lock_identity(tx, target)
        .await?
        .ok_or_else(|| unavailable_for(target))?;

    match identity.lifecycle {
        ArtifactLifecycle::Active => {
            let updated = match target {
                PlanFlavorRevisionTarget::ExecutablePlan(plan_id) => sqlx::query(
                    "UPDATE port_executable_plan_revisions SET lifecycle = 'draining' \
                     WHERE executable_plan_id = $1 AND lifecycle = 'active'",
                )
                .bind(plan_id.as_bytes().as_slice()),
                PlanFlavorRevisionTarget::WorkerFlavor(worker_flavor_id) => sqlx::query(
                    "UPDATE port_worker_flavor_revisions SET lifecycle = 'draining' \
                     WHERE worker_flavor_id = $1 AND lifecycle = 'active'",
                )
                .bind(worker_flavor_id.as_bytes().as_slice()),
            }
            .execute(&mut **tx)
            .await
            .map_err(driver_did_not_commit)?
            .rows_affected();
            if updated != 1 {
                // The row was read as Active under its exclusive lock, so no
                // concurrent writer can have moved it.
                return Err(RevisionCatalogError::CorruptRecord { target });
            }
            Ok(BeginDrainOutcome::Started(
                reference_counts(tx, target, now_ms).await?,
            ))
        },
        ArtifactLifecycle::Draining => Ok(BeginDrainOutcome::AlreadyDraining(
            reference_counts(tx, target, now_ms).await?,
        )),
        ArtifactLifecycle::Deleted => Err(deleted_for(target)),
    }
}

async fn delete_drained_locked(
    tx: &mut Transaction<'_, Postgres>,
    target: PlanFlavorRevisionTarget,
    now_ms: i64,
) -> Result<(), RevisionCatalogError> {
    let identity = lock_identity(tx, target)
        .await?
        .ok_or_else(|| unavailable_for(target))?;

    match identity.lifecycle {
        ArtifactLifecycle::Active => return Err(RevisionCatalogError::DrainRequired { target }),
        ArtifactLifecycle::Deleted => {
            // Repeating a delete against its own tombstone succeeds, so a
            // caller whose acknowledgement was lost can reconcile the commit.
            return if identity.payload_is_cleared {
                Ok(())
            } else {
                Err(RevisionCatalogError::CorruptRecord { target })
            };
        },
        ArtifactLifecycle::Draining => {},
    }

    let references = reference_counts(tx, target, now_ms).await?;
    if !references.is_empty() {
        return Err(RevisionCatalogError::Referenced { target, references });
    }

    match target {
        PlanFlavorRevisionTarget::ExecutablePlan(plan_id) => {
            sqlx::query(
                "UPDATE port_executable_plan_revisions \
                 SET lifecycle = 'deleted', record_bytes = NULL \
                 WHERE executable_plan_id = $1",
            )
            .bind(plan_id.as_bytes().as_slice())
            .execute(&mut **tx)
            .await
            .map_err(driver_did_not_commit)?;
        },
        PlanFlavorRevisionTarget::WorkerFlavor(worker_flavor_id) => {
            let dependent_plans: i64 = sqlx::query(
                "SELECT COUNT(*) AS dependent_plans FROM port_executable_plan_revisions \
                 WHERE worker_flavor_id = $1 AND lifecycle <> 'deleted'",
            )
            .bind(worker_flavor_id.as_bytes().as_slice())
            .fetch_one(&mut **tx)
            .await
            .map_err(driver_did_not_commit)?
            .try_get("dependent_plans")
            .map_err(|_column| RevisionCatalogError::CorruptRecord { target })?;
            if dependent_plans != 0 {
                return Err(RevisionCatalogError::DependentPlans {
                    worker_flavor_id,
                    dependent_plans: u64::try_from(dependent_plans).unwrap_or_default(),
                });
            }
            sqlx::query(
                "UPDATE port_worker_flavor_revisions \
                 SET lifecycle = 'deleted', record_bytes = NULL \
                 WHERE worker_flavor_id = $1",
            )
            .bind(worker_flavor_id.as_bytes().as_slice())
            .execute(&mut **tx)
            .await
            .map_err(driver_did_not_commit)?;
        },
    }

    Ok(())
}

#[async_trait::async_trait]
impl PlanFlavorCatalog for PgPlanFlavorCatalog {
    #[tracing::instrument(
        level = "debug",
        name = "revision_catalog.load_exact",
        skip(self),
        fields(
            backend = "postgres",
            plan_revision_id = %ids.plan(),
            worker_flavor_revision_id = %ids.worker_flavor(),
            outcome = tracing::field::Empty,
        )
    )]
    async fn load_exact(
        &self,
        ids: PlanFlavorRevisionIds,
    ) -> Result<PlanFlavorRevisionRecord, RevisionCatalogError> {
        // The transaction is opened *inside* the observed body. Returning `?`
        // from an unreachable pool would otherwise leave the span's `outcome`
        // empty and emit no event at all, so the one failure that means "this
        // backend is down" was the one failure nothing recorded.
        let result = async {
            let mut tx = self.begin().await?;
            let loaded = load_exact_locked(&mut tx, ids).await;
            // Nothing was written on this path; the commit only releases the
            // read transaction, so failing to release cannot make the outcome
            // unknown.
            drop(tx.commit().await);
            loaded
        }
        .await;

        let outcome = load_label(&result);
        tracing::Span::current().record("outcome", outcome);
        self.metrics
            .record(revision_catalog_operation::LOAD_EXACT, outcome);
        tracing::debug!(
            target: "nebula_storage::postgres",
            outcome,
            "exact plan/flavor catalog load"
        );
        result
    }
}

#[async_trait::async_trait]
impl PlanFlavorCatalogWriter for PgPlanFlavorCatalog {
    #[tracing::instrument(
        level = "debug",
        name = "revision_catalog.insert",
        skip(self, record),
        fields(
            backend = "postgres",
            plan_revision_id = %record.ids().plan(),
            worker_flavor_revision_id = %record.ids().worker_flavor(),
            outcome = tracing::field::Empty,
        )
    )]
    async fn insert(
        &self,
        record: &PlanFlavorRevisionRecord,
    ) -> Result<RevisionInsertOutcome, RevisionCatalogError> {
        let result = async {
            let mut tx = self.begin().await?;
            match insert_locked(&mut tx, record).await {
                Ok(outcome) => tx
                    .commit()
                    .await
                    .map_err(commit_outcome_unknown)
                    .map(|()| outcome),
                Err(rejection) => {
                    drop(tx.rollback().await);
                    Err(rejection)
                },
            }
        }
        .await;

        let outcome = insert_label(&result);
        tracing::Span::current().record("outcome", outcome);
        self.metrics
            .record(revision_catalog_operation::INSERT, outcome);
        tracing::debug!(
            target: "nebula_storage::postgres",
            outcome,
            "plan/flavor catalog insert"
        );
        result
    }
}

#[async_trait::async_trait]
impl PlanFlavorCatalogAdmin for PgPlanFlavorCatalog {
    #[tracing::instrument(
        level = "debug",
        name = "revision_catalog.begin_drain",
        skip(self),
        fields(backend = "postgres", target = ?target, outcome = tracing::field::Empty)
    )]
    async fn begin_drain(
        &self,
        target: PlanFlavorRevisionTarget,
    ) -> Result<BeginDrainOutcome, RevisionCatalogError> {
        let now_ms = chrono::Utc::now().timestamp_millis();
        let result = async {
            let mut tx = self.begin().await?;
            match begin_drain_locked(&mut tx, target, now_ms).await {
                Ok(outcome) => tx
                    .commit()
                    .await
                    .map_err(commit_outcome_unknown)
                    .map(|()| outcome),
                Err(rejection) => {
                    drop(tx.rollback().await);
                    Err(rejection)
                },
            }
        }
        .await;

        let outcome = drain_label(&result);
        tracing::Span::current().record("outcome", outcome);
        self.metrics
            .record(revision_catalog_operation::BEGIN_DRAIN, outcome);
        tracing::debug!(
            target: "nebula_storage::postgres",
            outcome,
            "plan/flavor catalog begin drain"
        );
        result
    }

    #[tracing::instrument(
        level = "debug",
        name = "revision_catalog.delete_drained",
        skip(self),
        fields(backend = "postgres", target = ?target, outcome = tracing::field::Empty)
    )]
    async fn delete_drained(
        &self,
        target: PlanFlavorRevisionTarget,
    ) -> Result<(), RevisionCatalogError> {
        let now_ms = chrono::Utc::now().timestamp_millis();
        let result = async {
            let mut tx = self.begin().await?;
            match delete_drained_locked(&mut tx, target, now_ms).await {
                Ok(()) => tx.commit().await.map_err(commit_outcome_unknown),
                Err(rejection) => {
                    drop(tx.rollback().await);
                    Err(rejection)
                },
            }
        }
        .await;

        let outcome = delete_label(&result);
        tracing::Span::current().record("outcome", outcome);
        self.metrics
            .record(revision_catalog_operation::DELETE_DRAINED, outcome);
        tracing::debug!(
            target: "nebula_storage::postgres",
            outcome,
            "plan/flavor catalog guarded delete"
        );
        result
    }
}
