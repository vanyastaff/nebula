//! SQLite-backed `RefreshClaimRepo` impl.
//!
//! Single-replica desktop mode + multi-process tests. CAS via
//! `INSERT ... ON CONFLICT DO UPDATE WHERE` to mirror Postgres
//! `INSERT ... ON CONFLICT ... WHERE` pattern.
//!
//! # Timestamp encoding
//!
//! Timestamp columns (`acquired_at`, `expires_at`, `detected_at`) are
//! stored as `INTEGER` milliseconds-since-UNIX-epoch, not RFC-3339 text.
//! Lexicographic comparison of `chrono::DateTime::to_rfc3339()` output is
//! fragile: the fractional-second suffix is conditional (only emitted when
//! non-zero), and the timezone form can vary (`+00:00` vs `Z`) across
//! chrono versions or mixed inserts. Integer ordering is unambiguous for
//! the `expires_at < now` predicate used by `try_claim`, `heartbeat`, and
//! `reclaim_stuck`. Postgres uses native `TIMESTAMPTZ`, which is also
//! naturally typed.

use std::time::Duration;

use chrono::{DateTime, TimeZone, Utc};
use nebula_core::CredentialId;
use nebula_storage_port::{
    CredentialMaterialEpoch, CredentialOwner, CredentialSelector, CredentialVersion,
};
use sqlx::SqlitePool;
use uuid::Uuid;

use super::{
    ClaimAttempt, ClaimToken, CredentialOperationDecision, CredentialOperationIntent,
    CredentialOperationKind, ExpiredClaim, HeartbeatError, ReauthEscalation, RefreshAdjudication,
    RefreshClaim, RefreshClaimAdjudicationError as RepoAdjudicationError, RefreshClaimAdjudicator,
    RefreshClaimReclaimer, RefreshClaimRepo, ReplicaId, RepoError, RevokeOutcomeDecision,
    SentinelEscalationPolicy, SqlxClaimResultExt, adjudicate_against_recorded_resolution,
    adjudication_evidence_digest, validate_adjudication_evidence,
};

const SQLITE_NOW_MS_SQL: &str = "SELECT unixepoch('now') * 1000 \
     + CAST(substr(strftime('%f', 'now'), 4, 3) AS INTEGER)";

const COUNT_SENTINEL_EVENTS_SQL: &str = "SELECT COUNT(*) \
     FROM credential_sentinel_events \
     WHERE owner_id = ?1 AND credential_id = ?2 \
       AND detected_at > ?3 AND operation_kind = ?4";

/// The poisoned-claim predicate, character for character the one `try_claim`
/// answers `OutcomeUnknown` with: an expired `sentinel = 1` row. The incident
/// table is deliberately not consulted — the sweep writes it later, so keying
/// on it would answer `NotPoisoned` for genuine replay-denied credentials
/// during the expiry-to-sweep window.
///
/// `?2` is the same clock source `try_claim` compares against, so a claim the
/// adapter just refused egress on is always adjudicable.
///
/// The select list must stay in this order: the caller decodes it into
/// `(claim_id, crashed_holder, generation)`, and a `claim_id`/holder swap is
/// invisible to both the compiler and the driver, so only the oracle cases
/// catch it. Change the list and the decode together.
const POISONED_CLAIM_SQL: &str = "SELECT claim_id, holder_replica_id, generation, operation_kind, observed_material_epoch \
     FROM credential_refresh_claims \
     WHERE owner_id = ?1 AND credential_id = ?2 AND expires_at < ?3 AND sentinel = 1";

const CLEAR_POISONED_CLAIM_SQL: &str = "DELETE FROM credential_refresh_claims \
     WHERE owner_id = ?1 AND credential_id = ?2 AND claim_id = ?3 AND generation = ?4";

/// Create the incident from the claim row's own identity when the sweep has not
/// run yet, or record the resolution on the incident it wrote.
///
/// Only the resolution columns are written on conflict: `detected_at`,
/// `crashed_holder` and `generation` stay as first accounted so neither the
/// sentinel window nor the incident's provenance can be rewritten by a later
/// adjudication.
const RECORD_RESOLUTION_SQL: &str = "INSERT INTO credential_sentinel_events \
     (owner_id, credential_id, claim_id, detected_at, crashed_holder, generation, \
      adjudicated_at, adjudication_decision, adjudication_evidence, \
      adjudication_evidence_digest, operation_kind, observed_material_epoch) \
     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?4, ?7, ?8, ?9, ?10, ?11) \
     ON CONFLICT(claim_id) WHERE claim_id IS NOT NULL DO UPDATE SET \
         adjudicated_at = excluded.adjudicated_at, \
         adjudication_decision = excluded.adjudication_decision, \
         adjudication_evidence = excluded.adjudication_evidence, \
         adjudication_evidence_digest = excluded.adjudication_evidence_digest, \
         operation_kind = excluded.operation_kind \
     WHERE credential_sentinel_events.adjudicated_at IS NULL \
     RETURNING claim_id";

/// The newest resolution on record for a credential.
///
/// The no-poison refusal names this pair as the one on record: the resolved-set
/// comparison has no single incident to point at, so the newest resolution is
/// the honest "what is on record now" answer a conflicted caller needs. It is
/// read inside the adjudication transaction, so the pair is the one the
/// refusal actually refused against.
const NEWEST_RESOLVED_PAIR_SQL: &str = "SELECT adjudication_evidence_digest, adjudication_decision, operation_kind \
     FROM credential_sentinel_events \
     WHERE owner_id = ?1 AND credential_id = ?2 AND adjudicated_at IS NOT NULL \
     ORDER BY adjudicated_at DESC, id DESC \
     LIMIT 1";

/// Does this credential's resolved set already carry this exact
/// `(evidence digest, decision)` pair?
///
/// The pair is the only identity the request carries, so an exact match over
/// the resolved set is how a recommit is attributed to the incident it
/// describes. Keyed on `credential_id` because a resolved incident outlives its
/// claim row, which is what keeps a pre-0039 incident with a NULL `claim_id`
/// adjudicable.
///
/// A replay of a *superseded* pair is accepted by design, and is not a gap: the
/// check is read-only and answers with the request's own values, and "this pair
/// is on record for this credential" is the honest answer to what a caller
/// re-committing it is asking. There is no unique incident for the request to
/// name — a credential may hold several resolved incidents — so evidence, not
/// incident identity, has to be the anchor; an incident id on the request would
/// be surface no caller can supply.
const RESOLVED_INCIDENT_MATCH_SQL: &str = "SELECT EXISTS ( \
         SELECT 1 FROM credential_sentinel_events \
         WHERE owner_id = ?1 AND credential_id = ?2 \
           AND adjudicated_at IS NOT NULL \
           AND adjudication_evidence_digest = ?3 \
           AND adjudication_decision = ?4 AND operation_kind = ?5 \
     )";

/// The resolution recorded on the incident a claim row owns.
///
/// Keyed on the claim UUID rather than the credential: the poisoned branch has
/// already located this claim's incident, so the comparison stays on this
/// lifecycle instead of reaching for whichever resolved incident is newest.
const CLAIM_INCIDENT_RESOLUTION_SQL: &str = "SELECT adjudication_evidence_digest, adjudication_decision, operation_kind \
     FROM credential_sentinel_events \
     WHERE claim_id = ?1";

/// Does `claim_id` have an incident whose provider outcome is still unknown?
const UNRESOLVED_INCIDENT_SQL: &str = "SELECT EXISTS ( \
         SELECT 1 FROM credential_sentinel_events \
         WHERE claim_id = ?1 AND adjudicated_at IS NULL \
     )";

/// SQLite-backed `RefreshClaimRepo`.
#[derive(Clone, Debug)]
pub struct SqliteRefreshClaimRepo {
    pool: SqlitePool,
}

impl SqliteRefreshClaimRepo {
    /// Wrap an existing pool. Caller is responsible for running migrations
    /// through 0057.
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

fn parse_credential_id(s: &str) -> Result<CredentialId, RepoError> {
    s.parse::<CredentialId>()
        .map_err(|_| RepoError::InvalidState)
}

/// Convert a millisecond-since-epoch column back to a `DateTime<Utc>`.
///
/// SQLite stores timestamps as `INTEGER` per migration 0022/0023; this is the
/// inverse of `DateTime::timestamp_millis()`. An out-of-range value indicates
/// table corruption (we never write such values), surfaced as `InvalidState`.
fn millis_to_utc(ms: i64) -> Result<DateTime<Utc>, RepoError> {
    Utc.timestamp_millis_opt(ms)
        .single()
        .ok_or(RepoError::InvalidState)
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
        let owner = selector.owner().as_str();
        let holder_str = holder.as_str();
        let now_ms = now.timestamp_millis();
        let exp_ms = new_expires.timestamp_millis();
        let claim_id_str = new_claim_id.to_string();
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await.store_err()?;
        if let CredentialOperationIntent::Revoke { material_epoch } = intent {
            let aggregate: Option<(i64, String)> = sqlx::query_as(
                "SELECT material_epoch, record_state FROM credentials \
                 WHERE owner_id = ?1 AND id = ?2",
            )
            .bind(owner)
            .bind(&cid_str)
            .fetch_optional(&mut *transaction)
            .await
            .store_err()?;
            let Some((actual, state)) = aggregate else {
                return Err(RepoError::AggregateUnavailable);
            };
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
        }

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
             (owner_id, credential_id, claim_id, generation, holder_replica_id, \
              acquired_at, expires_at, sentinel, operation_kind, observed_material_epoch) \
             VALUES (?1, ?2, ?3, 0, ?4, ?5, ?6, 0, ?7, ?8) \
             ON CONFLICT(owner_id, credential_id) DO UPDATE SET \
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
        .bind(owner)
        .bind(&cid_str)
        .bind(&claim_id_str)
        .bind(holder_str)
        .bind(now_ms)
        .bind(exp_ms)
        .bind(intent.kind().as_str())
        .bind(intent.material_epoch().map(CredentialMaterialEpoch::get))
        .fetch_optional(&mut *transaction)
        .await
        .store_err()?;

        if let Some((claim_id_str, generation, acquired_ms, expires_ms)) = row {
            let acquired_at = millis_to_utc(acquired_ms)?;
            let expires_at = millis_to_utc(expires_ms)?;
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
             WHERE owner_id = ?1 AND credential_id = ?2",
        )
        .bind(owner)
        .bind(&cid_str)
        .fetch_optional(&mut *transaction)
        .await
        .store_err()?;

        let attempt = match existing {
            Some((exp_ms, 1, kind)) if exp_ms < now_ms => Ok(ClaimAttempt::OutcomeUnknown {
                expired_at: millis_to_utc(exp_ms)?,
                operation: CredentialOperationKind::from_wire(&kind)
                    .ok_or(RepoError::InvalidState)?,
            }),
            Some((exp_ms, 0 | 1, _)) => Ok(ClaimAttempt::Contended {
                existing_expires_at: millis_to_utc(exp_ms)?,
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
        let now_ms = now.timestamp_millis();
        let extension_ms = (now
            + chrono::Duration::from_std(ttl)
                .map_err(|_| HeartbeatError::Repo(RepoError::InvalidState))?)
        .timestamp_millis();
        let claim_id_str = token.claim_id.to_string();
        let generation = i64::try_from(token.generation)
            .map_err(|_| HeartbeatError::Repo(RepoError::InvalidState))?;

        let rows = sqlx::query(
            "UPDATE credential_refresh_claims \
             SET expires_at = ?1 \
             WHERE owner_id = ?2 AND credential_id = ?3 \
               AND claim_id = ?4 AND generation = ?5 AND expires_at > ?6",
        )
        .bind(extension_ms)
        .bind(token.selector.owner().as_str())
        .bind(token.selector.credential_id().to_string())
        .bind(&claim_id_str)
        .bind(generation)
        .bind(now_ms)
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
        let rows = sqlx::query(
            "DELETE FROM credential_refresh_claims \
             WHERE owner_id = ?1 AND credential_id = ?2 \
               AND claim_id = ?3 AND generation = ?4 \
               AND NOT EXISTS ( \
                   SELECT 1 FROM credential_sentinel_events AS event \
                   WHERE event.claim_id = credential_refresh_claims.claim_id \
                     AND event.adjudicated_at IS NULL \
               )",
        )
        .bind(token.selector.owner().as_str())
        .bind(token.selector.credential_id().to_string())
        .bind(&claim_id_str)
        .bind(generation)
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
        let rows = sqlx::query(
            "UPDATE credential_refresh_claims \
             SET sentinel = 1 \
             WHERE owner_id = ?1 AND credential_id = ?2 AND claim_id = ?3 \
               AND generation = ?4 \
               AND expires_at > ( \
                   unixepoch('now') * 1000 \
                   + CAST(substr(strftime('%f', 'now'), 4, 3) AS INTEGER) \
               )",
        )
        .bind(token.selector.owner().as_str())
        .bind(token.selector.credential_id().to_string())
        .bind(&claim_id_str)
        .bind(generation)
        .execute(&self.pool)
        .await
        .store_err()?
        .rows_affected();

        if rows == 0 {
            return Err(RepoError::InvalidState);
        }
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
        let (now_ms,): (i64,) = sqlx::query_as(SQLITE_NOW_MS_SQL)
            .fetch_one(&mut *transaction)
            .await
            .store_err()?;
        let rows: Vec<(String, String, String, String, i64, i64, String, Option<i64>)> = sqlx::query_as(
            "SELECT owner_id, credential_id, claim_id, holder_replica_id, generation, sentinel, operation_kind, observed_material_epoch \
             FROM credential_refresh_claims AS claim \
             WHERE expires_at < ?1 \
               AND ( \
                   sentinel = 0 \
                   OR ( \
                       sentinel = 1 \
                       AND NOT EXISTS ( \
                           SELECT 1 FROM credential_sentinel_events AS event \
                           WHERE event.owner_id = claim.owner_id \
                             AND event.credential_id = claim.credential_id \
                             AND event.claim_id = claim.claim_id \
                       ) \
                   ) \
               )",
        )
        .bind(now_ms)
        .fetch_all(&mut *transaction)
        .await
        .store_err()?;

        let mut out = Vec::with_capacity(rows.len());
        let window_ms =
            i64::try_from(policy.window().as_millis()).map_err(|_| RepoError::InvalidState)?;
        for (
            owner,
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
                CredentialOwner::from_canonical(owner.clone()),
                credential_id,
            );
            let previous_generation =
                u64::try_from(generation).map_err(|_| RepoError::InvalidState)?;
            match sentinel_raw {
                0 => {
                    let deleted = sqlx::query(
                        "DELETE FROM credential_refresh_claims \
                         WHERE owner_id = ?1 AND credential_id = ?2 \
                           AND claim_id = ?3 AND generation = ?4",
                    )
                    .bind(&owner)
                    .bind(&cid)
                    .bind(&claim_id)
                    .bind(generation)
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
                        "INSERT INTO credential_sentinel_events \
                         (owner_id, credential_id, claim_id, detected_at, crashed_holder, generation, operation_kind, observed_material_epoch) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    )
                    .bind(&owner)
                    .bind(&cid)
                    .bind(&claim_id)
                    .bind(now_ms)
                    .bind(&holder)
                    .bind(generation)
                    .bind(operation.as_str())
                    .bind(observed_epoch)
                    .execute(&mut *transaction)
                    .await
                    .store_err()?;
                    let (count,): (i64,) = sqlx::query_as(COUNT_SENTINEL_EVENTS_SQL)
                        .bind(&owner)
                        .bind(&cid)
                        .bind(now_ms.saturating_sub(window_ms))
                        .bind(operation.as_str())
                        .fetch_one(&mut *transaction)
                        .await
                        .store_err()?;
                    let event_count = u32::try_from(count).unwrap_or(u32::MAX);
                    let escalation = if operation == CredentialOperationKind::Refresh
                        && event_count >= policy.threshold()
                    {
                        let aggregate: Option<(i64, i64, i64, String)> = sqlx::query_as(
                            "SELECT version, material_epoch, reauth_required, record_state \
                             FROM credentials WHERE owner_id = ?1 AND id = ?2",
                        )
                        .bind(&owner)
                        .bind(&cid)
                        .fetch_optional(&mut *transaction)
                        .await
                        .store_err()?;
                        match aggregate {
                            Some((version, epoch, reauth_required, state)) if state == "live" => {
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
                                        let updated = sqlx::query(
                                            "UPDATE credentials SET reauth_required = 1, \
                                                 version = ?3, material_epoch = ?4, updated_at = ?5, \
                                                 refresh_retry_mode = NULL, refresh_retry_not_before = NULL, \
                                                 refresh_retry_phase = NULL, refresh_retry_kind = NULL, \
                                                 refresh_retry_diagnostic_code = NULL \
                                             WHERE owner_id = ?1 AND id = ?2 \
                                               AND record_state = 'live' AND reauth_required = 0",
                                        )
                                        .bind(&owner)
                                        .bind(&cid)
                                        .bind(next_version.get())
                                        .bind(next_epoch.get())
                                        .bind(now_ms)
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
                            Some((_, _, _, state)) if state == "tombstoned" => {
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
        decision: CredentialOperationDecision,
        evidence: &str,
    ) -> Result<RefreshAdjudication, RepoAdjudicationError> {
        validate_adjudication_evidence(evidence)?;
        let digest = adjudication_evidence_digest(evidence);
        let cid_str = selector.credential_id().to_string();
        let owner = selector.owner().as_str();
        let now_ms = Utc::now().timestamp_millis();

        // One transaction: the poison is cleared and its resolution recorded
        // together, or neither is. `BEGIN IMMEDIATE` also takes the write lock
        // before the poison lookup, so two concurrent adjudications of the same
        // credential cannot both find the row.
        let mut transaction = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(|_| RepoAdjudicationError::Storage)?;

        let poisoned: Option<(String, String, i64, String, Option<i64>)> =
            sqlx::query_as(POISONED_CLAIM_SQL)
                .bind(owner)
                .bind(&cid_str)
                .bind(now_ms)
                .fetch_optional(&mut *transaction)
                .await
                .map_err(|_| RepoAdjudicationError::Storage)?;

        let adjudication = if let Some((
            claim_id,
            crashed_holder,
            generation,
            operation_raw,
            observed_epoch,
        )) = poisoned
        {
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
                     WHERE owner_id = ?1 AND id = ?2",
                )
                .bind(owner)
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
                             WHERE owner_id = ?1 AND id = ?2 AND record_state = 'live' AND version = ?5",
                        )
                        .bind(owner)
                        .bind(&cid_str)
                        .bind(next.get())
                        .bind(now_ms)
                        .bind(version.get())
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
                .bind(owner)
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
                .bind(owner)
                .bind(&cid_str)
                .bind(&claim_id)
                .bind(now_ms)
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

            if recorded.is_some() {
                // `RETURNING` yields a row only when this call recorded the
                // resolution.
                RefreshAdjudication::new(decision, true, digest)
            } else {
                // The incident already carried a resolution, and the
                // `adjudicated_at IS NULL` guard on the upsert above is what
                // filtered this call's write out. The comparison is keyed on
                // `claim_id`, on which the partial unique index admits at most
                // one incident row (`0039_credentials_owner_and_record_state.sql:14-16`),
                // so the row read is this claim's own incident rather than
                // whichever resolved incident is newest.
                //
                // The `Storage` arm below is unreachable through the port, and
                // that is why no case can fail on it: reaching this state means
                // a live poisoned row for a claim whose incident already
                // carries a resolution, but the `cleared != 1` check makes the
                // claim-row DELETE a precondition of the resolution write, and
                // both `try_claim` and `reclaim_stuck` mint fresh claim ids, so
                // no row can be re-created for this claim. The agreement is
                // closed for the day it becomes reachable.
                let Some((recorded_digest, recorded_decision, recorded_operation)) = self
                    .claim_incident_resolution(&mut transaction, &claim_id)
                    .await?
                else {
                    return Err(RepoAdjudicationError::Storage);
                };
                adjudicate_against_recorded_resolution(
                    &recorded_digest,
                    recorded_decision.as_deref(),
                    CredentialOperationKind::from_wire(&recorded_operation)
                        .ok_or(RepoAdjudicationError::Storage)?,
                    &digest,
                    decision,
                )?
            }
        } else {
            // Nothing is poisoned. The credential's resolved set is the only
            // identity available — there is no claim row, and the request
            // carries none — so the rule is an exact match over what the
            // credential has already decided: this pair on record is the
            // idempotent recommit, a set without it contradicts every decision
            // on record, and an empty set has nothing to adjudicate.
            //
            // The newest resolution is read in the same pass so a refusal can
            // name the pair on record: a set has no single incident to point
            // at, and "the newest resolution" is the honest answer to what a
            // conflicted caller's evidence disagreed with.
            let Some((newest_digest, newest_decision, newest_operation)) = self
                .newest_resolved_pair(&mut transaction, selector)
                .await?
            else {
                return Err(RepoAdjudicationError::NotPoisoned);
            };
            match adjudicate_against_recorded_resolution(
                &newest_digest,
                newest_decision.as_deref(),
                CredentialOperationKind::from_wire(&newest_operation)
                    .ok_or(RepoAdjudicationError::Storage)?,
                &digest,
                decision,
            ) {
                Ok(recommit) => recommit,
                Err(conflict @ RepoAdjudicationError::EvidenceConflict { .. }) => {
                    // Not the newest: still a no-op when an older resolution
                    // already recorded this exact pair (a superseded replay).
                    if self
                        .has_matching_resolution(&mut transaction, selector, &digest, decision)
                        .await?
                    {
                        RefreshAdjudication::new(decision, false, digest)
                    } else {
                        return Err(conflict);
                    }
                },
                Err(other) => return Err(other),
            }
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

impl SqliteRefreshClaimRepo {
    /// The newest resolution on record for this credential, if any.
    ///
    /// Decoding is left to the shared comparison
    /// (`adjudicate_against_recorded_resolution`), so a corrupted decision
    /// spelling or non-32-byte digest fails closed there as `Storage` rather
    /// than being re-classified here.
    async fn newest_resolved_pair(
        &self,
        transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        selector: &CredentialSelector,
    ) -> Result<Option<(Vec<u8>, Option<String>, String)>, RepoAdjudicationError> {
        sqlx::query_as(NEWEST_RESOLVED_PAIR_SQL)
            .bind(selector.owner().as_str())
            .bind(selector.credential_id().to_string())
            .fetch_optional(&mut **transaction)
            .await
            .map_err(|_| RepoAdjudicationError::Storage)
    }

    /// Does this credential's resolved set carry this exact
    /// `(digest, decision)` pair?
    async fn has_matching_resolution(
        &self,
        transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        selector: &CredentialSelector,
        digest: &[u8; 32],
        decision: CredentialOperationDecision,
    ) -> Result<bool, RepoAdjudicationError> {
        let (exists,): (i64,) = sqlx::query_as(RESOLVED_INCIDENT_MATCH_SQL)
            .bind(selector.owner().as_str())
            .bind(selector.credential_id().to_string())
            .bind(digest.as_slice())
            .bind(decision.as_str())
            .bind(decision.kind().as_str())
            .fetch_one(&mut **transaction)
            .await
            .map_err(|_| RepoAdjudicationError::Storage)?;
        Ok(exists != 0)
    }

    /// The resolution recorded on `claim_id`'s incident, if it has one.
    async fn claim_incident_resolution(
        &self,
        transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        claim_id: &str,
    ) -> Result<Option<(Vec<u8>, Option<String>, String)>, RepoAdjudicationError> {
        sqlx::query_as(CLAIM_INCIDENT_RESOLUTION_SQL)
            .bind(claim_id)
            .fetch_optional(&mut **transaction)
            .await
            .map_err(|_| RepoAdjudicationError::Storage)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use nebula_storage_port::{
        CredentialCreate, CredentialMaterialEpoch, CredentialOperationDecision,
        CredentialOperationIntent, CredentialOwner, CredentialPersistence, CredentialSelector,
        RefreshClaimAdjudicator, RefreshClaimStore, RevokeOutcomeDecision, SecretBytes,
        StoredCredential,
    };

    use crate::credential::SqliteCredentialPersistence;

    use super::{
        CLAIM_INCIDENT_RESOLUTION_SQL, COUNT_SENTINEL_EVENTS_SQL, NEWEST_RESOLVED_PAIR_SQL,
        POISONED_CLAIM_SQL, RESOLVED_INCIDENT_MATCH_SQL, SQLITE_NOW_MS_SQL,
    };

    #[tokio::test]
    async fn revoke_applied_tombstones_and_resolves_in_one_transaction() {
        let store = SqliteCredentialPersistence::connect_memory()
            .await
            .expect("ready credential store");
        let selector = CredentialSelector::new(
            CredentialOwner::from_canonical("revoke-adjudication-owner"),
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
        let repo = store.refresh_claim_repo();
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
        sqlx::query(
            "UPDATE credential_refresh_claims SET expires_at = 0 \
             WHERE owner_id = ?1 AND credential_id = ?2",
        )
        .bind(selector.owner().as_str())
        .bind(selector.credential_id().to_string())
        .execute(&repo.pool)
        .await
        .expect("expire claim");

        let result = repo
            .adjudicate(
                &selector,
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
             FROM credential_sentinel_events WHERE owner_id = ?1 AND credential_id = ?2",
        )
        .bind(selector.owner().as_str())
        .bind(selector.credential_id().to_string())
        .fetch_one(&repo.pool)
        .await
        .expect("incident provenance");
        assert_eq!(provenance, ("revoke".to_owned(), Some(1)));
    }

    #[test]
    fn sentinel_window_uses_the_sqlite_clock() {
        assert!(
            SQLITE_NOW_MS_SQL.contains("unixepoch('now') * 1000"),
            "sentinel windows must be derived from SQLite's clock"
        );
        assert!(
            COUNT_SENTINEL_EVENTS_SQL.contains("detected_at > ?3"),
            "the count must use the transaction-derived absolute window boundary"
        );
    }

    #[test]
    fn poison_detection_is_keyed_on_the_claim_row() {
        assert!(
            POISONED_CLAIM_SQL.contains("FROM credential_refresh_claims"),
            "the poisoned claim row is the poison"
        );
        assert!(POISONED_CLAIM_SQL.contains("expires_at < ?3"));
        assert!(POISONED_CLAIM_SQL.contains("sentinel = 1"));
        assert!(
            !POISONED_CLAIM_SQL.contains("credential_sentinel_events"),
            "the incident is the poison's accounting and the sweep writes it later"
        );
    }

    /// The no-poison rule is an exact match over the credential's resolved set.
    ///
    /// Bind order is the silent half: a swap between `?3` and `?4` compiles and
    /// turns every genuine recommit into `EvidenceConflict`, and dropping either
    /// column would accept a pair the credential never recorded. The behavioural
    /// oracle is the conformance case
    /// `recommit_of_an_older_resolution_is_a_no_op_not_a_conflict`; this pins
    /// the shape that case reads.
    #[test]
    fn resolved_set_lookup_matches_the_whole_recorded_pair() {
        let query = RESOLVED_INCIDENT_MATCH_SQL;
        assert!(
            query.contains("owner_id = ?1 AND credential_id = ?2"),
            "the resolved set is the credential's own"
        );
        assert!(
            query.contains("adjudicated_at IS NOT NULL"),
            "an incident with no resolution is not a candidate"
        );
        assert!(
            !query.contains("ORDER BY"),
            "an exact match is not a newest-of query"
        );
        let digest = RESOLVED_INCIDENT_MATCH_SQL
            .find("adjudication_evidence_digest = ?3")
            .expect("the request's digest is compared");
        let decision = RESOLVED_INCIDENT_MATCH_SQL
            .find("adjudication_decision = ?4")
            .expect("the request's decision is compared");
        assert!(
            digest < decision,
            "the bind order is (digest, decision); the statement must agree"
        );
    }

    /// `newest_resolved_pair` decodes into `(digest, decision)`, keyed on the
    /// credential, newest first.
    ///
    /// A swapped select list compiles and only fails at the driver boundary,
    /// exactly as with `claim_incident_resolution`; a dropped `ORDER BY` would
    /// let whichever row the engine lands on speak as "the pair on record" in
    /// a refusal.
    #[test]
    fn newest_resolved_pair_reads_the_digest_before_the_decision_newest_first() {
        let digest = NEWEST_RESOLVED_PAIR_SQL
            .find("adjudication_evidence_digest")
            .expect("the resolution digest is read");
        let decision = NEWEST_RESOLVED_PAIR_SQL
            .find("adjudication_decision")
            .expect("the resolution decision is read");
        assert!(
            digest < decision,
            "newest_resolved_pair decodes (digest, decision); the select list must agree"
        );
        assert!(
            NEWEST_RESOLVED_PAIR_SQL.contains("WHERE owner_id = ?1 AND credential_id = ?2"),
            "the newest resolution is the credential's own"
        );
        assert!(
            NEWEST_RESOLVED_PAIR_SQL.contains("adjudicated_at IS NOT NULL"),
            "an incident with no resolution is not a candidate"
        );
        assert!(
            NEWEST_RESOLVED_PAIR_SQL.contains("ORDER BY adjudicated_at DESC"),
            "a refusal must name the newest resolution, not a random one"
        );
    }

    /// `claim_incident_resolution` decodes into `(digest, decision)`, keyed on
    /// the claim's own incident.
    ///
    /// A swapped select list compiles and only fails at the driver boundary —
    /// as `Storage` for a `TEXT`-into-`BLOB` decode, or as a silent
    /// `EvidenceConflict` when the digest happens to be valid UTF-8. This pins
    /// the select-list order only; the decode it must agree with is exercised
    /// by the oracle cases, not here.
    #[test]
    fn claim_incident_resolution_reads_the_digest_before_the_decision() {
        let digest = CLAIM_INCIDENT_RESOLUTION_SQL
            .find("adjudication_evidence_digest")
            .expect("the resolution digest is read");
        let decision = CLAIM_INCIDENT_RESOLUTION_SQL
            .find("adjudication_decision")
            .expect("the resolution decision is read");
        assert!(
            digest < decision,
            "claim_incident_resolution decodes (digest, decision); the select list must agree"
        );
        assert!(
            CLAIM_INCIDENT_RESOLUTION_SQL.contains("WHERE claim_id = ?1"),
            "the comparison is this claim's own incident, not the credential's newest"
        );
    }
}
