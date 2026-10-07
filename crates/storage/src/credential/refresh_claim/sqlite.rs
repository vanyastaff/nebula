//! SQLite-backed `RefreshClaimRepo` impl.
//!
//! Single-replica desktop mode + multi-process tests. CAS via
//! `INSERT ... ON CONFLICT DO UPDATE WHERE` to mirror Postgres
//! `INSERT ... ON CONFLICT ... WHERE` pattern.
//!
//! # Timestamp encoding
//!
//! Instants (`acquired_at`, `expires_at`, `detected_at`, `adjudicated_at`)
//! are `INTEGER` microseconds since the Unix epoch, per the database
//! standard; integer ordering is unambiguous for the `expires_at < now`
//! predicate used by `try_claim`, `heartbeat`, and `reclaim_stuck`.
//!
//! # Ownership
//!
//! Claims and incidents belong to their credential (migration 0070) and are
//! filed under its workspace's `(org_id, workspace_id)`. A claim requires a
//! credential that exists and is not archived, and an archived credential's
//! claim cannot cross the provider boundary.

use std::time::Duration;

use chrono::{DateTime, Utc};
use nebula_core::CredentialId;
use nebula_storage_port::{
    CredentialAdmissionEpoch, CredentialMaterialEpoch, CredentialOwner, CredentialSelector,
    CredentialVersion, Scope,
};
use sqlx::SqlitePool;
use uuid::Uuid;

use super::{
    ClaimAttempt, ClaimToken, CredentialIncidentRef, CredentialOperationDecision,
    CredentialOperationIntent, CredentialOperationKind, ExpiredClaim, HeartbeatError,
    ReauthEscalation, RefreshAdjudication, RefreshClaim,
    RefreshClaimAdjudicationError as RepoAdjudicationError, RefreshClaimAdjudicator,
    RefreshClaimReclaimer, RefreshClaimRepo, ReplicaId, RepoError, RevokeOutcomeDecision,
    SentinelEscalationPolicy, SqlxClaimResultExt, adjudicate_against_recorded_resolution,
    adjudication_evidence_digest, validate_adjudication_evidence,
};

const SQLITE_NOW_US_SQL: &str = concat!("SELECT ", sqlite_now_us!());

const COUNT_SENTINEL_EVENTS_SQL: &str = "SELECT COUNT(*) \
     FROM credential_refresh_incidents \
     WHERE org_id = ?1 AND workspace_id = ?2 AND credential_id = ?3 \
       AND detected_at > ?4 AND operation_kind = ?5";

/// The poisoned-claim predicate, character for character the one `try_claim`
/// answers `OutcomeUnknown` with: an expired `sentinel = 1` row. The incident
/// table is deliberately not consulted — the sweep writes it later, so keying
/// on it would answer `NotPoisoned` for genuine replay-denied credentials
/// during the expiry-to-sweep window.
///
/// `?4` is the same clock source `try_claim` compares against, so a claim the
/// adapter just refused egress on is always adjudicable.
///
/// The select list must stay in this order: the caller decodes it into
/// `(claim_id, crashed_holder, generation)`, and a `claim_id`/holder swap is
/// invisible to both the compiler and the driver, so only the oracle cases
/// catch it. Change the list and the decode together.
const POISONED_CLAIM_SQL: &str = "SELECT claim_id, holder_replica_id, generation, operation_kind, observed_material_epoch \
     FROM credential_refresh_claims \
     WHERE org_id = ?1 AND workspace_id = ?2 AND credential_id = ?3 \
       AND expires_at < ?4 AND sentinel = 1";

const CLEAR_POISONED_CLAIM_SQL: &str = "DELETE FROM credential_refresh_claims \
     WHERE org_id = ?1 AND workspace_id = ?2 AND credential_id = ?3 \
       AND claim_id = ?4 AND generation = ?5";

/// Create the incident from the claim row's own identity when the sweep has not
/// run yet, or record the resolution on the incident it wrote.
///
/// Only the resolution columns are written on conflict: `detected_at`,
/// `crashed_holder` and `generation` stay as first accounted so neither the
/// sentinel window nor the incident's provenance can be rewritten by a later
/// adjudication.
const RECORD_RESOLUTION_SQL: &str = "INSERT INTO credential_refresh_incidents \
     (org_id, workspace_id, credential_id, claim_id, detected_at, crashed_holder, generation, \
      adjudicated_at, adjudication_decision, adjudication_evidence, \
      adjudication_evidence_digest, operation_kind, observed_material_epoch) \
     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?5, ?8, ?9, ?10, ?11, ?12) \
     ON CONFLICT(claim_id) DO UPDATE SET \
         adjudicated_at = excluded.adjudicated_at, \
         adjudication_decision = excluded.adjudication_decision, \
         adjudication_evidence = excluded.adjudication_evidence, \
         adjudication_evidence_digest = excluded.adjudication_evidence_digest, \
         operation_kind = excluded.operation_kind \
     WHERE credential_refresh_incidents.adjudicated_at IS NULL \
     RETURNING claim_id";

/// The resolution recorded on the named incident, if it has one.
///
/// Owner- and credential-bound, so an incident identity from another tenant or
/// credential finds nothing. Read before the poisoned claim: a retry of a
/// decision whose incident is already resolved must be answered from that
/// incident, never fall through to whatever claim is poisoned now.
const INCIDENT_RESOLUTION_SQL: &str = "SELECT adjudication_evidence_digest, adjudication_decision, operation_kind \
     FROM credential_refresh_incidents \
     WHERE org_id = ?1 AND workspace_id = ?2 AND credential_id = ?3 AND claim_id = ?4 \
       AND adjudicated_at IS NOT NULL";

/// Does `claim_id` have an incident whose provider outcome is still unknown?
const UNRESOLVED_INCIDENT_SQL: &str = "SELECT EXISTS ( \
         SELECT 1 FROM credential_refresh_incidents \
         WHERE claim_id = ?1 AND adjudicated_at IS NULL \
     )";

/// SQLite-backed `RefreshClaimRepo`.
#[derive(Clone, Debug)]
pub struct SqliteRefreshClaimRepo {
    pool: SqlitePool,
}

impl SqliteRefreshClaimRepo {
    /// Wrap an existing pool. Caller is responsible for running migrations
    /// through 0061.
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

fn parse_credential_id(s: &str) -> Result<CredentialId, RepoError> {
    s.parse::<CredentialId>()
        .map_err(|_| RepoError::InvalidState)
}

/// Convert a microsecond-since-epoch column back to a `DateTime<Utc>`.
///
/// The inverse of `DateTime::timestamp_micros()`. An out-of-range value
/// indicates table corruption (we never write such values), surfaced as
/// `InvalidState`.
fn micros_to_utc(us: i64) -> Result<DateTime<Utc>, RepoError> {
    DateTime::from_timestamp_micros(us).ok_or(RepoError::InvalidState)
}

/// The owner partition of a row filed under `(org_id, workspace_id)`.
fn row_owner(org_id: String, workspace_id: String) -> CredentialOwner {
    CredentialOwner::from_scope(&Scope::new(workspace_id, org_id))
}

#[async_trait::async_trait]
impl RefreshClaimRepo for SqliteRefreshClaimRepo {
    async fn try_claim(
        &self,
        selector: &CredentialSelector,
        holder: &ReplicaId,
        ttl: Duration,
        intent: CredentialOperationIntent,
    ) -> Result<ClaimAttempt, RepoError> {
        let now = Utc::now();
        let new_claim_id = Uuid::new_v4();
        let new_expires =
            now + chrono::Duration::from_std(ttl).map_err(|_| RepoError::InvalidState)?;
        let cid_str = selector.credential_id().to_string();
        let scope = selector
            .owner()
            .scope()
            .ok_or(RepoError::AggregateUnavailable)?;
        let holder_str = holder.as_str();
        let now_us = now.timestamp_micros();
        let exp_us = new_expires.timestamp_micros();
        let claim_id_str = new_claim_id.to_string();
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await.store_err()?;
        // A claim belongs to a credential that exists and is not archived;
        // the write lock serializes this check with a concurrent archive. The
        // admission epoch a won revoke claim must advance is read with it.
        let aggregate: Option<(i64, String, i64)> = sqlx::query_as(
            "SELECT material_epoch, record_state, admission_epoch FROM credentials \
             WHERE org_id = ?1 AND workspace_id = ?2 AND id = ?3 AND deleted_at IS NULL",
        )
        .bind(&scope.org_id)
        .bind(&scope.workspace_id)
        .bind(&cid_str)
        .fetch_optional(&mut *transaction)
        .await
        .store_err()?;
        let Some((actual, state, admission)) = aggregate else {
            return Err(RepoError::AggregateUnavailable);
        };
        let revoke_admission = if let CredentialOperationIntent::Revoke { material_epoch } = intent
        {
            if state != "live" {
                return Err(RepoError::AggregateUnavailable);
            }
            let actual =
                CredentialMaterialEpoch::try_from(actual).map_err(|_| RepoError::InvalidState)?;
            if actual != material_epoch {
                return Err(RepoError::MaterialEpochConflict {
                    expected: material_epoch,
                    actual,
                });
            }
            Some((
                material_epoch,
                CredentialAdmissionEpoch::try_from(admission)
                    .map_err(|_| RepoError::InvalidState)?,
            ))
        } else {
            None
        };

        // Atomic CAS via UPSERT with conditional UPDATE clause. Mirrors the
        // Postgres `INSERT ... ON CONFLICT DO UPDATE WHERE expires_at < ...
        // AND sentinel = 0` pattern (control-queue + refresh-claim CAS).
        // Expired in-flight rows remain intact until `reclaim_stuck` returns
        // their evidence to one sweeper. Requires SQLite 3.35+ for `RETURNING`.
        //
        // Win path: the row we wrote (or overwrote in place) comes back via
        // RETURNING. Lose path: the WHERE clause filtered the UPDATE, no
        // row is returned, and we fetch the existing row's `expires_at` for
        // the caller's backoff hint.
        let row: Option<(String, i64, i64, i64)> = sqlx::query_as(
            "INSERT INTO credential_refresh_claims \
             (org_id, workspace_id, credential_id, claim_id, generation, holder_replica_id, \
              acquired_at, expires_at, sentinel, operation_kind, observed_material_epoch) \
             VALUES (?1, ?9, ?2, ?3, 0, ?4, ?5, ?6, 0, ?7, ?8) \
             ON CONFLICT(org_id, workspace_id, credential_id) DO UPDATE SET \
                 claim_id = excluded.claim_id, \
                 generation = credential_refresh_claims.generation + 1, \
                 holder_replica_id = excluded.holder_replica_id, \
                 acquired_at = excluded.acquired_at, \
                 expires_at = excluded.expires_at, \
                 sentinel = 0, operation_kind = excluded.operation_kind, \
                 observed_material_epoch = excluded.observed_material_epoch \
             WHERE credential_refresh_claims.expires_at < ?5 \
               AND credential_refresh_claims.sentinel = 0 \
             RETURNING claim_id, generation, acquired_at, expires_at",
        )
        .bind(&scope.org_id)
        .bind(&cid_str)
        .bind(&claim_id_str)
        .bind(holder_str)
        .bind(now_us)
        .bind(exp_us)
        .bind(intent.kind().as_str())
        .bind(intent.material_epoch().map(CredentialMaterialEpoch::get))
        .bind(&scope.workspace_id)
        .fetch_optional(&mut *transaction)
        .await
        .store_err()?;

        if let Some((claim_id_str, generation, acquired_us, expires_us)) = row {
            // A won revoke closes credential use in the same transaction.
            if let Some((material_epoch, admission)) = revoke_admission {
                let next = admission
                    .next()
                    .map_err(|_| RepoError::AdmissionEpochExhausted)?;
                advance_admission_epoch(
                    &mut transaction,
                    &scope,
                    &cid_str,
                    admission,
                    next,
                    Some(material_epoch),
                )
                .await?;
            }
            let acquired_at = micros_to_utc(acquired_us)?;
            let expires_at = micros_to_utc(expires_us)?;
            let claim_id = claim_id_str
                .parse::<Uuid>()
                .map_err(|_| RepoError::InvalidState)?;
            let generation = u64::try_from(generation).map_err(|_| RepoError::InvalidState)?;
            let acquired = ClaimAttempt::Acquired(RefreshClaim {
                selector: selector.clone(),
                token: ClaimToken {
                    selector: selector.clone(),
                    claim_id,
                    generation,
                },
                acquired_at,
                expires_at,
            });
            transaction.commit().await.store_err()?;
            return Ok(acquired);
        }

        // CAS lost — fetch existing row's expires_at for the backoff hint.
        // If the row vanished between the failed UPSERT and this SELECT
        // (release / reclaim_stuck happened in between), surface as
        // `Contended { existing_expires_at: now }`: the caller backs off the
        // standard jitter delay and retries. Returning `InvalidState` here
        // would surface a transient race as a hard error.
        let existing: Option<(i64, i64, String)> = sqlx::query_as(
            "SELECT expires_at, sentinel, operation_kind \
             FROM credential_refresh_claims \
             WHERE org_id = ?1 AND workspace_id = ?2 AND credential_id = ?3",
        )
        .bind(&scope.org_id)
        .bind(&scope.workspace_id)
        .bind(&cid_str)
        .fetch_optional(&mut *transaction)
        .await
        .store_err()?;

        let attempt = match existing {
            Some((exp_us, 1, kind)) if exp_us < now_us => Ok(ClaimAttempt::OutcomeUnknown {
                expired_at: micros_to_utc(exp_us)?,
                operation: CredentialOperationKind::from_wire(&kind)
                    .ok_or(RepoError::InvalidState)?,
            }),
            Some((exp_us, 0 | 1, _)) => Ok(ClaimAttempt::Contended {
                existing_expires_at: micros_to_utc(exp_us)?,
            }),
            Some((_, _, _)) => Err(RepoError::InvalidState),
            None => Ok(ClaimAttempt::Contended {
                existing_expires_at: now,
            }),
        };
        transaction.commit().await.store_err()?;
        attempt
    }

    async fn heartbeat(&self, token: &ClaimToken, ttl: Duration) -> Result<(), HeartbeatError> {
        let now = Utc::now();
        let now_us = now.timestamp_micros();
        let extension_us = (now
            + chrono::Duration::from_std(ttl)
                .map_err(|_| HeartbeatError::Repo(RepoError::InvalidState))?)
        .timestamp_micros();
        let claim_id_str = token.claim_id.to_string();
        let generation = i64::try_from(token.generation)
            .map_err(|_| HeartbeatError::Repo(RepoError::InvalidState))?;
        let Some(scope) = token.selector.owner().scope() else {
            return Err(HeartbeatError::ClaimLost);
        };

        let rows = sqlx::query(
            "UPDATE credential_refresh_claims \
             SET expires_at = ?1 \
             WHERE org_id = ?2 AND workspace_id = ?7 AND credential_id = ?3 \
               AND claim_id = ?4 AND generation = ?5 AND expires_at > ?6",
        )
        .bind(extension_us)
        .bind(&scope.org_id)
        .bind(token.selector.credential_id().to_string())
        .bind(&claim_id_str)
        .bind(generation)
        .bind(now_us)
        .bind(&scope.workspace_id)
        .execute(&self.pool)
        .await
        .store_err()?
        .rows_affected();

        if rows == 0 {
            return Err(HeartbeatError::ClaimLost);
        }
        Ok(())
    }

    async fn release(&self, token: ClaimToken) -> Result<(), RepoError> {
        let claim_id_str = token.claim_id.to_string();
        let generation = i64::try_from(token.generation).map_err(|_| RepoError::InvalidState)?;
        // An incident with no recorded provider outcome is unresolved poison:
        // it outlives its claim row until `adjudicate` decides it, so the
        // predicate retains the row and the caller learns why. An absent claim,
        // a superseded generation, or an already-reconciled incident all keep
        // release idempotent.
        let Some(scope) = token.selector.owner().scope() else {
            return Ok(());
        };
        let rows = sqlx::query(
            "DELETE FROM credential_refresh_claims \
             WHERE org_id = ?1 AND workspace_id = ?5 AND credential_id = ?2 \
               AND claim_id = ?3 AND generation = ?4 \
               AND NOT EXISTS ( \
                   SELECT 1 FROM credential_refresh_incidents AS incident \
                   WHERE incident.claim_id = credential_refresh_claims.claim_id \
                     AND incident.adjudicated_at IS NULL \
               )",
        )
        .bind(&scope.org_id)
        .bind(token.selector.credential_id().to_string())
        .bind(&claim_id_str)
        .bind(generation)
        .bind(&scope.workspace_id)
        .execute(&self.pool)
        .await
        .store_err()?
        .rows_affected();

        if rows == 0 && self.has_unresolved_incident(&claim_id_str).await? {
            return Err(RepoError::ReleaseRefused);
        }
        Ok(())
    }

    async fn mark_sentinel(&self, token: &ClaimToken) -> Result<(), RepoError> {
        let claim_id_str = token.claim_id.to_string();
        let generation = i64::try_from(token.generation).map_err(|_| RepoError::InvalidState)?;
        // Mirrors heartbeat's claim-validity check: zero rows affected means
        // the claim is absent, superseded, or expired. Returning Ok here
        // would authorize provider egress after the holder's TTL elapsed.
        // The expiry comparison uses SQLite's clock inside the UPDATE so
        // connection-pool wait time cannot stale a caller-bound timestamp.
        //
        // Crossing the provider boundary closes credential use, so the
        // sentinel and the admission epoch commit together. `BEGIN IMMEDIATE`
        // takes the write lock first; the claim row is written before the
        // credential row, the order every claim/credential transaction uses.
        // An archived credential's claim never authorizes provider egress.
        let scope = token
            .selector
            .owner()
            .scope()
            .ok_or(RepoError::InvalidState)?;
        let cid_str = token.selector.credential_id().to_string();
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await.store_err()?;
        let rows = sqlx::query(concat!(
            "UPDATE credential_refresh_claims \
             SET sentinel = 1 \
             WHERE org_id = ?1 AND workspace_id = ?5 AND credential_id = ?2 AND claim_id = ?3 \
               AND generation = ?4 \
               AND expires_at > ",
            sqlite_now_us!(),
            " AND EXISTS ( \
                   SELECT 1 FROM credentials AS c \
                   WHERE c.org_id = credential_refresh_claims.org_id \
                     AND c.workspace_id = credential_refresh_claims.workspace_id \
                     AND c.id = credential_refresh_claims.credential_id \
                     AND c.deleted_at IS NULL \
               )"
        ))
        .bind(&scope.org_id)
        .bind(&cid_str)
        .bind(&claim_id_str)
        .bind(generation)
        .bind(&scope.workspace_id)
        .execute(&mut *transaction)
        .await
        .store_err()?
        .rows_affected();

        if rows == 0 {
            return Err(RepoError::InvalidState);
        }
        let admission: Option<(i64,)> = sqlx::query_as(
            "SELECT admission_epoch FROM credentials \
             WHERE org_id = ?1 AND workspace_id = ?2 AND id = ?3 AND record_state = 'live'",
        )
        .bind(&scope.org_id)
        .bind(&scope.workspace_id)
        .bind(&cid_str)
        .fetch_optional(&mut *transaction)
        .await
        .store_err()?;
        // A terminal or absent credential has no use left to close.
        if let Some((admission,)) = admission {
            let admission = CredentialAdmissionEpoch::try_from(admission)
                .map_err(|_| RepoError::InvalidState)?;
            let next = admission
                .next()
                .map_err(|_| RepoError::AdmissionEpochExhausted)?;
            advance_admission_epoch(&mut transaction, &scope, &cid_str, admission, next, None)
                .await?;
        }
        transaction.commit().await.store_err()?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl RefreshClaimReclaimer for SqliteRefreshClaimRepo {
    async fn reclaim_stuck(
        &self,
        policy: SentinelEscalationPolicy,
    ) -> Result<Vec<ExpiredClaim>, RepoError> {
        // `BEGIN IMMEDIATE` takes SQLite's write lock before reading expired
        // rows. That serializes existence-check + event insert for poisoned
        // rows and delete for Normal rows into one atomic boundary.
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await.store_err()?;
        // Capture the transaction's effective time only after acquiring the
        // write lock. A pool or lock wait must not make a newly inserted
        // incident older than the rolling-window count that follows it.
        let (now_us,): (i64,) = sqlx::query_as(SQLITE_NOW_US_SQL)
            .fetch_one(&mut *transaction)
            .await
            .store_err()?;
        let rows: Vec<(
            String,
            String,
            String,
            String,
            String,
            i64,
            i64,
            String,
            Option<i64>,
        )> = sqlx::query_as(
            "SELECT org_id, workspace_id, credential_id, claim_id, holder_replica_id, generation, \
                    sentinel, operation_kind, observed_material_epoch \
             FROM credential_refresh_claims AS claim \
             WHERE expires_at < ?1 \
               AND ( \
                   sentinel = 0 \
                   OR ( \
                       sentinel = 1 \
                       AND NOT EXISTS ( \
                           SELECT 1 FROM credential_refresh_incidents AS incident \
                           WHERE incident.claim_id = claim.claim_id \
                       ) \
                   ) \
               )",
        )
        .bind(now_us)
        .fetch_all(&mut *transaction)
        .await
        .store_err()?;

        let mut out = Vec::with_capacity(rows.len());
        let window_us =
            i64::try_from(policy.window().as_micros()).map_err(|_| RepoError::InvalidState)?;
        for (
            org_id,
            workspace_id,
            cid,
            claim_id,
            holder,
            generation,
            sentinel_raw,
            operation_raw,
            observed_epoch,
        ) in rows
        {
            let operation = CredentialOperationKind::from_wire(&operation_raw)
                .ok_or(RepoError::InvalidState)?;
            let credential_id = parse_credential_id(&cid)?;
            let selector = CredentialSelector::new(
                row_owner(org_id.clone(), workspace_id.clone()),
                credential_id,
            );
            let previous_generation =
                u64::try_from(generation).map_err(|_| RepoError::InvalidState)?;
            match sentinel_raw {
                0 => {
                    let deleted = sqlx::query(
                        "DELETE FROM credential_refresh_claims \
                         WHERE org_id = ?1 AND workspace_id = ?5 AND credential_id = ?2 \
                           AND claim_id = ?3 AND generation = ?4",
                    )
                    .bind(&org_id)
                    .bind(&cid)
                    .bind(&claim_id)
                    .bind(generation)
                    .bind(&workspace_id)
                    .execute(&mut *transaction)
                    .await
                    .store_err()?
                    .rows_affected();
                    if deleted != 1 {
                        return Err(RepoError::InvalidState);
                    }
                    out.push(ExpiredClaim::ReclaimedNormal {
                        selector,
                        previous_holder: ReplicaId::new(holder),
                        previous_generation,
                    });
                },
                1 => {
                    sqlx::query(
                        "INSERT INTO credential_refresh_incidents \
                         (org_id, workspace_id, credential_id, claim_id, detected_at, crashed_holder, \
                          generation, operation_kind, observed_material_epoch) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    )
                    .bind(&org_id)
                    .bind(&workspace_id)
                    .bind(&cid)
                    .bind(&claim_id)
                    .bind(now_us)
                    .bind(&holder)
                    .bind(generation)
                    .bind(operation.as_str())
                    .bind(observed_epoch)
                    .execute(&mut *transaction)
                    .await
                    .store_err()?;
                    let (count,): (i64,) = sqlx::query_as(COUNT_SENTINEL_EVENTS_SQL)
                        .bind(&org_id)
                        .bind(&workspace_id)
                        .bind(&cid)
                        .bind(now_us.saturating_sub(window_us))
                        .bind(operation.as_str())
                        .fetch_one(&mut *transaction)
                        .await
                        .store_err()?;
                    let event_count = u32::try_from(count).unwrap_or(u32::MAX);
                    let escalation = if operation == CredentialOperationKind::Refresh
                        && event_count >= policy.threshold()
                    {
                        let aggregate: Option<(i64, i64, i64, String, i64, bool)> = sqlx::query_as(
                            "SELECT version, material_epoch, reauth_required, record_state, \
                                        admission_epoch, deleted_at IS NOT NULL \
                                 FROM credentials \
                                 WHERE org_id = ?1 AND workspace_id = ?2 AND id = ?3",
                        )
                        .bind(&org_id)
                        .bind(&workspace_id)
                        .bind(&cid)
                        .fetch_optional(&mut *transaction)
                        .await
                        .store_err()?;
                        match aggregate {
                            // An archived credential has no use left to close.
                            Some((_, _, _, _, _, true)) => ReauthEscalation::AggregateTerminal,
                            Some((version, epoch, reauth_required, state, admission, false))
                                if state == "live" =>
                            {
                                let version = CredentialVersion::try_from(version)
                                    .map_err(|_| RepoError::InvalidState)?;
                                let epoch = CredentialMaterialEpoch::try_from(epoch)
                                    .map_err(|_| RepoError::InvalidState)?;
                                match reauth_required {
                                    1 => ReauthEscalation::ReauthRequired {
                                        changed: false,
                                        version,
                                        material_epoch: epoch,
                                    },
                                    0 => {
                                        let next_version = version
                                            .next_live()
                                            .map_err(|_| RepoError::InvalidState)?;
                                        let next_epoch =
                                            epoch.next().map_err(|_| RepoError::InvalidState)?;
                                        let next_admission =
                                            CredentialAdmissionEpoch::try_from(admission)
                                                .map_err(|_| RepoError::InvalidState)?
                                                .next()
                                                .map_err(|_| RepoError::AdmissionEpochExhausted)?;
                                        let updated = sqlx::query(
                                            "UPDATE credentials SET reauth_required = 1, \
                                                 version = ?3, material_epoch = ?4, updated_at = ?5, \
                                                 admission_epoch = ?6, \
                                                 refresh_retry_mode = NULL, refresh_retry_not_before = NULL, \
                                                 refresh_retry_phase = NULL, refresh_retry_kind = NULL, \
                                                 refresh_retry_diagnostic_code = NULL \
                                             WHERE org_id = ?1 AND workspace_id = ?7 AND id = ?2 \
                                               AND record_state = 'live' AND reauth_required = 0",
                                        )
                                        .bind(&org_id)
                                        .bind(&cid)
                                        .bind(next_version.get())
                                        .bind(next_epoch.get())
                                        .bind(now_us)
                                        .bind(next_admission.get())
                                        .bind(&workspace_id)
                                        .execute(&mut *transaction)
                                        .await
                                        .store_err()?
                                        .rows_affected();
                                        if updated != 1 {
                                            return Err(RepoError::InvalidState);
                                        }
                                        ReauthEscalation::ReauthRequired {
                                            changed: true,
                                            version: next_version,
                                            material_epoch: next_epoch,
                                        }
                                    },
                                    _ => return Err(RepoError::InvalidState),
                                }
                            },
                            Some((_, _, _, state, _, false)) if state == "tombstoned" => {
                                ReauthEscalation::AggregateTerminal
                            },
                            Some(_) | None => return Err(RepoError::InvalidState),
                        }
                    } else {
                        ReauthEscalation::BelowThreshold
                    };
                    out.push(ExpiredClaim::OutcomeUnknownAccounted {
                        selector,
                        previous_holder: ReplicaId::new(holder),
                        previous_generation,
                        operation,
                        event_count,
                        escalation,
                    });
                },
                _ => {
                    return Err(RepoError::InvalidState);
                },
            }
        }

        transaction.commit().await.store_err()?;
        Ok(out)
    }
}

/// Advance a live credential's admission epoch from `current` to `next` inside
/// the caller's claim transaction, fenced by the material epoch when one is
/// given.
///
/// This write closes credential use and nothing else: it never moves the row
/// `version` or `updated_at`, so it cannot conflict with an ordinary
/// version-fenced replacement.
async fn advance_admission_epoch(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    scope: &Scope,
    credential_id: &str,
    current: CredentialAdmissionEpoch,
    next: CredentialAdmissionEpoch,
    material_epoch: Option<CredentialMaterialEpoch>,
) -> Result<(), RepoError> {
    let rows = sqlx::query(
        "UPDATE credentials SET admission_epoch = ?3 \
         WHERE org_id = ?1 AND workspace_id = ?6 AND id = ?2 AND record_state = 'live' \
           AND admission_epoch = ?4 AND (?5 IS NULL OR material_epoch = ?5)",
    )
    .bind(&scope.org_id)
    .bind(credential_id)
    .bind(next.get())
    .bind(current.get())
    .bind(material_epoch.map(CredentialMaterialEpoch::get))
    .bind(&scope.workspace_id)
    .execute(&mut **transaction)
    .await
    .store_err()?
    .rows_affected();
    if rows != 1 {
        return Err(RepoError::InvalidState);
    }
    Ok(())
}

impl SqliteRefreshClaimRepo {
    /// Does an incident for `claim_id` still lack a recorded provider outcome?
    async fn has_unresolved_incident(&self, claim_id: &str) -> Result<bool, RepoError> {
        let (exists,): (i64,) = sqlx::query_as(UNRESOLVED_INCIDENT_SQL)
            .bind(claim_id)
            .fetch_one(&self.pool)
            .await
            .store_err()?;
        Ok(exists != 0)
    }
}

#[async_trait::async_trait]
impl RefreshClaimAdjudicator for SqliteRefreshClaimRepo {
    async fn adjudicate(
        &self,
        selector: &CredentialSelector,
        incident: CredentialIncidentRef,
        decision: CredentialOperationDecision,
        evidence: &str,
    ) -> Result<RefreshAdjudication, RepoAdjudicationError> {
        validate_adjudication_evidence(evidence)?;
        let digest = adjudication_evidence_digest(evidence);
        let cid_str = selector.credential_id().to_string();
        let Some(scope) = selector.owner().scope() else {
            return Err(RepoAdjudicationError::NotPoisoned);
        };
        let incident_str = incident.as_uuid().to_string();
        let now_us = Utc::now().timestamp_micros();

        // One transaction: the poison is cleared and its resolution recorded
        // together, or neither is. `BEGIN IMMEDIATE` also takes the write lock
        // before the poison lookup, so two concurrent adjudications of the same
        // credential cannot both find the row.
        let mut transaction = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(|_| RepoAdjudicationError::Storage)?;

        // An archived credential is unusable, its incidents included.
        let archived: Option<(bool,)> = sqlx::query_as(
            "SELECT deleted_at IS NOT NULL FROM credentials \
             WHERE org_id = ?1 AND workspace_id = ?2 AND id = ?3",
        )
        .bind(&scope.org_id)
        .bind(&scope.workspace_id)
        .bind(&cid_str)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|_| RepoAdjudicationError::Storage)?;
        if matches!(archived, Some((true,))) {
            return Err(RepoAdjudicationError::AggregateUnavailable);
        }

        // The named incident's own resolution answers first. A retry after a
        // lost acknowledgement lands here, and must not fall through to a
        // newer poisoned claim that its decision never described.
        let recorded: Option<(Vec<u8>, Option<String>, String)> =
            sqlx::query_as(INCIDENT_RESOLUTION_SQL)
                .bind(&scope.org_id)
                .bind(&scope.workspace_id)
                .bind(&cid_str)
                .bind(&incident_str)
                .fetch_optional(&mut *transaction)
                .await
                .map_err(|_| RepoAdjudicationError::Storage)?;
        if let Some((recorded_digest, recorded_decision, recorded_operation)) = recorded {
            return adjudicate_against_recorded_resolution(
                &recorded_digest,
                recorded_decision.as_deref(),
                CredentialOperationKind::from_wire(&recorded_operation)
                    .ok_or(RepoAdjudicationError::Storage)?,
                &digest,
                decision,
            );
        }

        let poisoned: Option<(String, String, i64, String, Option<i64>)> =
            sqlx::query_as(POISONED_CLAIM_SQL)
                .bind(&scope.org_id)
                .bind(&scope.workspace_id)
                .bind(&cid_str)
                .bind(now_us)
                .fetch_optional(&mut *transaction)
                .await
                .map_err(|_| RepoAdjudicationError::Storage)?;

        let Some((claim_id, crashed_holder, generation, operation_raw, observed_epoch)) = poisoned
        else {
            return Err(RepoAdjudicationError::NotPoisoned);
        };
        // A different poisoned claim is a different incident: the decision
        // was established for the named one and says nothing about this one.
        if claim_id.parse::<Uuid>().ok() != Some(incident.as_uuid()) {
            return Err(RepoAdjudicationError::StaleIncident);
        }
        let adjudication = {
            let operation = CredentialOperationKind::from_wire(&operation_raw)
                .ok_or(RepoAdjudicationError::Storage)?;
            if operation != decision.kind() {
                return Err(RepoAdjudicationError::OperationMismatch {
                    recorded_operation: operation,
                });
            }
            if decision
                == CredentialOperationDecision::Revoke(RevokeOutcomeDecision::ProviderRevoked)
            {
                let expected_epoch = observed_epoch.ok_or(RepoAdjudicationError::Storage)?;
                let aggregate: Option<(i64, i64, String)> = sqlx::query_as(
                    "SELECT version, material_epoch, record_state FROM credentials \
                     WHERE org_id = ?1 AND workspace_id = ?2 AND id = ?3 AND deleted_at IS NULL",
                )
                .bind(&scope.org_id)
                .bind(&scope.workspace_id)
                .bind(&cid_str)
                .fetch_optional(&mut *transaction)
                .await
                .map_err(|_| RepoAdjudicationError::Storage)?;
                let Some((version, actual_epoch, state)) = aggregate else {
                    return Err(RepoAdjudicationError::Storage);
                };
                if actual_epoch != expected_epoch {
                    return Err(RepoAdjudicationError::MaterialEpochConflict);
                }
                match state.as_str() {
                    "live" => {
                        let version = CredentialVersion::try_from(version)
                            .map_err(|_| RepoAdjudicationError::Storage)?;
                        let next = version
                            .next_tombstone()
                            .map_err(|_| RepoAdjudicationError::Storage)?;
                        let rows = sqlx::query(
                            "UPDATE credentials SET name = NULL, data = zeroblob(0), \
                             version = ?3, updated_at = ?4, expires_at = NULL, \
                             reauth_required = 0, metadata = '{}', record_state = 'tombstoned', \
                             tombstoned_at = ?4, refresh_retry_mode = NULL, \
                             refresh_retry_not_before = NULL, refresh_retry_phase = NULL, \
                             refresh_retry_kind = NULL, refresh_retry_diagnostic_code = NULL \
                             WHERE org_id = ?1 AND workspace_id = ?6 AND id = ?2 \
                               AND record_state = 'live' AND version = ?5",
                        )
                        .bind(&scope.org_id)
                        .bind(&cid_str)
                        .bind(next.get())
                        .bind(now_us)
                        .bind(version.get())
                        .bind(&scope.workspace_id)
                        .execute(&mut *transaction)
                        .await
                        .map_err(|_| RepoAdjudicationError::Storage)?
                        .rows_affected();
                        if rows != 1 {
                            return Err(RepoAdjudicationError::Storage);
                        }
                    },
                    "tombstoned" => {},
                    _ => return Err(RepoAdjudicationError::Storage),
                }
            }
            let cleared = sqlx::query(CLEAR_POISONED_CLAIM_SQL)
                .bind(&scope.org_id)
                .bind(&scope.workspace_id)
                .bind(&cid_str)
                .bind(&claim_id)
                .bind(generation)
                .execute(&mut *transaction)
                .await
                .map_err(|_| RepoAdjudicationError::Storage)?
                .rows_affected();
            if cleared != 1 {
                return Err(RepoAdjudicationError::Storage);
            }

            let recorded: Option<(String,)> = sqlx::query_as(RECORD_RESOLUTION_SQL)
                .bind(&scope.org_id)
                .bind(&scope.workspace_id)
                .bind(&cid_str)
                .bind(&claim_id)
                .bind(now_us)
                .bind(&crashed_holder)
                .bind(generation)
                .bind(decision.as_str())
                .bind(evidence)
                .bind(digest.as_slice())
                .bind(operation.as_str())
                .bind(observed_epoch)
                .fetch_optional(&mut *transaction)
                .await
                .map_err(|_| RepoAdjudicationError::Storage)?;

            // `RETURNING` yields a row only when this call recorded the
            // resolution. The named incident had no resolution when this
            // transaction read it under the write lock, and the claim row
            // DELETE above is a precondition of the write, so an empty
            // `RETURNING` is a corrupted incident row, not a caller mistake.
            if recorded.is_none() {
                return Err(RepoAdjudicationError::Storage);
            }
            RefreshAdjudication::new(decision, true, digest)
        };

        // The decision is written but its acknowledgement is what we are
        // missing here: the caller may recommit the same evidence safely.
        transaction
            .commit()
            .await
            .map_err(|_| RepoAdjudicationError::AcknowledgementUnknown)?;
        Ok(adjudication)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use nebula_storage_port::{
        CredentialCreate, CredentialMaterialEpoch, CredentialOperationDecision,
        CredentialOperationIntent, CredentialPersistence, CredentialSelector,
        RefreshClaimAdjudicator, RefreshClaimStore, RevokeOutcomeDecision, SecretBytes,
        StoredCredential,
    };

    use crate::credential::test_owner::owner;

    use super::{
        COUNT_SENTINEL_EVENTS_SQL, INCIDENT_RESOLUTION_SQL, POISONED_CLAIM_SQL, SQLITE_NOW_US_SQL,
    };

    #[tokio::test]
    async fn revoke_applied_tombstones_and_resolves_in_one_transaction() {
        let store = crate::credential::test_owner::sqlite_store()
            .await
            .expect("ready credential store");
        let repo = store.refresh_claim_repo();
        let selector = CredentialSelector::new(
            owner("revoke-adjudication"),
            nebula_core::CredentialId::new(),
        );
        store
            .create(
                &selector,
                CredentialCreate::new(
                    "oauth".to_owned(),
                    SecretBytes::new(vec![7]),
                    "oauth".to_owned(),
                    1,
                    None,
                    None,
                    false,
                    Default::default(),
                ),
            )
            .await
            .expect("create credential");
        let claim = match repo
            .try_claim(
                &selector,
                &super::ReplicaId::new("revoke-holder"),
                Duration::from_secs(30),
                CredentialOperationIntent::Revoke {
                    material_epoch: CredentialMaterialEpoch::MIN,
                },
            )
            .await
            .expect("acquire revoke")
        {
            super::ClaimAttempt::Acquired(claim) => claim,
            other => panic!("unexpected claim attempt: {other:?}"),
        };
        repo.mark_sentinel(&claim.token)
            .await
            .expect("cross provider boundary");
        sqlx::query("UPDATE credential_refresh_claims SET expires_at = 0 WHERE credential_id = ?1")
            .bind(selector.credential_id().to_string())
            .execute(&repo.pool)
            .await
            .expect("expire claim");

        let result = repo
            .adjudicate(
                &selector,
                super::CredentialIncidentRef::from_uuid(claim.token.claim_id),
                CredentialOperationDecision::Revoke(RevokeOutcomeDecision::ProviderRevoked),
                "provider audit confirms revocation",
            )
            .await
            .expect("adjudicate revoke");
        assert!(result.changed);
        assert!(matches!(
            store.get(&selector).await.expect("physical tombstone"),
            StoredCredential::Tombstoned(_)
        ));
        let provenance: (String, Option<i64>) = sqlx::query_as(
            "SELECT operation_kind, observed_material_epoch \
             FROM credential_refresh_incidents WHERE credential_id = ?1",
        )
        .bind(selector.credential_id().to_string())
        .fetch_one(&repo.pool)
        .await
        .expect("incident provenance");
        assert_eq!(provenance, ("revoke".to_owned(), Some(1)));
    }

    #[test]
    fn sentinel_window_uses_the_sqlite_clock() {
        assert!(
            SQLITE_NOW_US_SQL.contains("strftime('%s', 'now') AS INTEGER) * 1000000"),
            "sentinel windows must be derived from SQLite's clock"
        );
        assert!(
            COUNT_SENTINEL_EVENTS_SQL.contains("detected_at > ?4"),
            "the count must use the transaction-derived absolute window boundary"
        );
    }

    #[test]
    fn poison_detection_is_keyed_on_the_claim_row() {
        assert!(
            POISONED_CLAIM_SQL.contains("FROM credential_refresh_claims"),
            "the poisoned claim row is the poison"
        );
        assert!(POISONED_CLAIM_SQL.contains("expires_at < ?4"));
        assert!(POISONED_CLAIM_SQL.contains("sentinel = 1"));
        assert!(
            !POISONED_CLAIM_SQL.contains("credential_refresh_incidents"),
            "the incident is the poison's accounting and the sweep writes it later"
        );
    }

    /// The named incident's resolution is read owner- and credential-bound,
    /// resolved rows only, digest before decision.
    ///
    /// A swapped select list compiles and only fails at the driver boundary;
    /// a dropped owner or credential predicate would let an incident identity
    /// from another tenant answer for this credential.
    #[test]
    fn incident_resolution_is_bound_to_the_owner_and_credential() {
        let query = INCIDENT_RESOLUTION_SQL;
        assert!(query.contains(
            "org_id = ?1 AND workspace_id = ?2 AND credential_id = ?3 AND claim_id = ?4"
        ));
        assert!(
            query.contains("adjudicated_at IS NOT NULL"),
            "an incident with no resolution is not a recorded answer"
        );
        let digest = query
            .find("adjudication_evidence_digest")
            .expect("the resolution digest is read");
        let decision = query
            .find("adjudication_decision")
            .expect("the resolution decision is read");
        assert!(
            digest < decision,
            "the caller decodes (digest, decision); the select list must agree"
        );
    }
}
