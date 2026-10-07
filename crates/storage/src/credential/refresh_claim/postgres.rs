//! Postgres-backed `RefreshClaimRepo` impl.
//!
//! Multi-replica production target. Atomic CAS via
//! `INSERT ... ON CONFLICT (org_id, workspace_id, credential_id) DO UPDATE
//! WHERE credential_refresh_claims.expires_at < CURRENT_TIMESTAMP
//! AND NOT sentinel`
//! pattern, mirroring control-queue claim acquisition.
//!
//! PostgreSQL is the lease-clock authority: acquisition, heartbeat,
//! sentinel admission, and reclaim all compare against the database clock.
//!
//! Claims and incidents belong to their credential and are
//! filed under its workspace's `(org_id, workspace_id)`. A claim requires a
//! credential that exists and is not archived (share-locked after the claim
//! row, the order every claim/credential transaction uses), and an archived
//! credential's claim cannot cross the provider boundary.

use std::time::Duration;

use chrono::{DateTime, Utc};
use nebula_core::CredentialId;
use nebula_storage_port::{
    CredentialAdmissionEpoch, CredentialMaterialEpoch, CredentialOwner, CredentialSelector,
    CredentialVersion, Scope,
};
use sqlx::PgPool;
use uuid::Uuid;

use crate::sql_error::is_foreign_key_violation;

use super::{
    ClaimAttempt, ClaimToken, CredentialIncidentRef, CredentialOperationDecision,
    CredentialOperationIntent, CredentialOperationKind, ExpiredClaim, HeartbeatError,
    ReauthEscalation, RefreshAdjudication, RefreshClaim,
    RefreshClaimAdjudicationError as RepoAdjudicationError, RefreshClaimAdjudicator,
    RefreshClaimReclaimer, RefreshClaimRepo, ReplicaId, RepoError, RevokeOutcomeDecision,
    SentinelEscalationPolicy, SqlxClaimResultExt, adjudicate_against_recorded_resolution,
    adjudication_evidence_digest, validate_adjudication_evidence,
};

const TRY_CLAIM_SQL: &str = "INSERT INTO credential_refresh_claims \
     (org_id, workspace_id, credential_id, claim_id, generation, holder_replica_id, \
      acquired_at, expires_at, sentinel, operation_kind, observed_material_epoch) \
     VALUES ( \
         $1, $8, $2, $3, 0, $4, CURRENT_TIMESTAMP, \
         CURRENT_TIMESTAMP + ($5 * INTERVAL '1 microsecond'), FALSE, $6, $7 \
     ) \
     ON CONFLICT (org_id, workspace_id, credential_id) DO UPDATE \
     SET claim_id = EXCLUDED.claim_id, \
         generation = credential_refresh_claims.generation + 1, \
         holder_replica_id = EXCLUDED.holder_replica_id, \
         acquired_at = EXCLUDED.acquired_at, \
         expires_at = EXCLUDED.expires_at, \
         sentinel = FALSE, operation_kind = EXCLUDED.operation_kind, \
         observed_material_epoch = EXCLUDED.observed_material_epoch \
     WHERE credential_refresh_claims.expires_at < CURRENT_TIMESTAMP \
       AND NOT credential_refresh_claims.sentinel \
     RETURNING claim_id, generation, acquired_at, expires_at";

/// The claimed credential is live: it exists (the foreign key) and is not
/// archived. Share-locked after the claim row so a concurrent archive
/// serializes with the claim.
const CLAIMED_CREDENTIAL_LIVE_SQL: &str = "SELECT deleted_at IS NULL FROM credentials \
     WHERE org_id = $1 AND workspace_id = $2 AND id = $3 \
     FOR SHARE";

/// A won revoke claim closes credential use: advance the live credential's
/// admission epoch, fenced by the material epoch the revoke observed. The
/// range guard makes an exhausted epoch match no row instead of overflowing.
/// Neither `version` nor `updated_at` moves.
const REVOKE_ADMISSION_SQL: &str = "UPDATE credentials \
     SET admission_epoch = admission_epoch + 1 \
     WHERE org_id = $1 AND workspace_id = $4 AND id = $2 AND record_state = 'live' \
       AND deleted_at IS NULL \
       AND material_epoch = $3 \
       AND admission_epoch < 9223372036854775807 \
     RETURNING admission_epoch";

/// Crossing the provider boundary closes credential use for either operation
/// kind. Same shape as [`REVOKE_ADMISSION_SQL`] without the material fence.
const SENTINEL_ADMISSION_SQL: &str = "UPDATE credentials \
     SET admission_epoch = admission_epoch + 1 \
     WHERE org_id = $1 AND workspace_id = $3 AND id = $2 AND record_state = 'live' \
       AND admission_epoch < 9223372036854775807 \
     RETURNING admission_epoch";

const HEARTBEAT_SQL: &str = "UPDATE credential_refresh_claims \
     SET expires_at = CURRENT_TIMESTAMP + ($1 * INTERVAL '1 microsecond') \
     WHERE org_id = $2 AND workspace_id = $6 AND credential_id = $3 AND claim_id = $4 \
       AND generation = $5 \
       AND expires_at > CURRENT_TIMESTAMP";

const RECLAIM_SELECT_SQL: &str = "SELECT \
         org_id, workspace_id, credential_id, claim_id, holder_replica_id, generation, \
         sentinel, operation_kind, observed_material_epoch \
     FROM credential_refresh_claims AS claim \
     WHERE expires_at < CURRENT_TIMESTAMP \
       AND ( \
           NOT sentinel \
           OR NOT EXISTS ( \
               SELECT 1 FROM credential_refresh_incidents AS incident \
               WHERE incident.claim_id = claim.claim_id \
           ) \
       ) \
     FOR UPDATE SKIP LOCKED";

const COUNT_SENTINEL_EVENTS_SQL: &str = "SELECT COUNT(*) \
     FROM credential_refresh_incidents \
     WHERE org_id = $1 AND workspace_id = $5 AND credential_id = $2 \
       AND detected_at > clock_timestamp() - ($3 * INTERVAL '1 microsecond') \
       AND operation_kind = $4";

/// The poisoned-claim predicate, the same one `try_claim` answers
/// `OutcomeUnknown` with: an expired sentinel row compared against the
/// PostgreSQL clock. The incident table is deliberately not consulted — the
/// sweep writes it later, so keying on it would answer `NotPoisoned` for
/// genuine replay-denied credentials during the expiry-to-sweep window.
const POISONED_CLAIM_SQL: &str = "SELECT claim_id, holder_replica_id, generation, operation_kind, observed_material_epoch \
     FROM credential_refresh_claims \
     WHERE org_id = $1 AND workspace_id = $3 AND credential_id = $2 \
       AND expires_at < CURRENT_TIMESTAMP \
       AND sentinel \
     FOR UPDATE";

const CLEAR_POISONED_CLAIM_SQL: &str = "DELETE FROM credential_refresh_claims \
     WHERE org_id = $1 AND workspace_id = $5 AND credential_id = $2 \
       AND claim_id = $3 AND generation = $4";

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
     VALUES ($1, $11, $2, $3, clock_timestamp(), $4, $5, clock_timestamp(), $6, $7, $8, $9, $10) \
     ON CONFLICT (claim_id) DO UPDATE SET \
         adjudicated_at = EXCLUDED.adjudicated_at, \
         adjudication_decision = EXCLUDED.adjudication_decision, \
         adjudication_evidence = EXCLUDED.adjudication_evidence, \
         adjudication_evidence_digest = EXCLUDED.adjudication_evidence_digest, \
         operation_kind = EXCLUDED.operation_kind \
     WHERE credential_refresh_incidents.adjudicated_at IS NULL \
     RETURNING claim_id";

/// The resolution recorded on the named incident, if it has one.
///
/// Owner- and credential-bound, so an incident identity from another tenant or
/// credential finds nothing. A retry of a decision whose incident is already
/// resolved must be answered from that incident, never fall through to
/// whatever claim is poisoned now.
const INCIDENT_RESOLUTION_SQL: &str = "SELECT adjudication_evidence_digest, adjudication_decision, operation_kind \
     FROM credential_refresh_incidents \
     WHERE org_id = $1 AND workspace_id = $4 AND credential_id = $2 AND claim_id = $3 \
       AND adjudicated_at IS NOT NULL";

/// Does `claim_id` have an incident whose provider outcome is still unknown?
const UNRESOLVED_INCIDENT_SQL: &str = "SELECT EXISTS ( \
         SELECT 1 FROM credential_refresh_incidents \
         WHERE claim_id = $1 AND adjudicated_at IS NULL \
     )";

/// Postgres-backed `RefreshClaimRepo`.
#[derive(Clone, Debug)]
pub struct PgRefreshClaimRepo {
    pool: PgPool,
}

impl PgRefreshClaimRepo {
    /// Wrap an existing pool. Caller is responsible for admitting the current
    /// deployment catalog before use.
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn parse_credential_id(s: &str) -> Result<CredentialId, RepoError> {
    s.parse::<CredentialId>()
        .map_err(|_| RepoError::InvalidState)
}

/// The owner partition of a row filed under `(org_id, workspace_id)`.
fn row_owner(org_id: String, workspace_id: String) -> CredentialOwner {
    CredentialOwner::from_scope(&Scope::new(workspace_id, org_id))
}

#[async_trait::async_trait]
impl RefreshClaimRepo for PgRefreshClaimRepo {
    async fn try_claim(
        &self,
        selector: &CredentialSelector,
        holder: &ReplicaId,
        ttl: Duration,
        intent: CredentialOperationIntent,
    ) -> Result<ClaimAttempt, RepoError> {
        let new_claim_id = Uuid::new_v4();
        let ttl_micros = i64::try_from(ttl.as_micros()).map_err(|_| RepoError::InvalidState)?;
        let cid_str = selector.credential_id().to_string();
        let scope = selector
            .owner()
            .scope()
            .ok_or(RepoError::AggregateUnavailable)?;
        let mut transaction = self.pool.begin().await.store_err()?;
        // Atomic CAS: INSERT, or UPDATE only an expired Normal row. An
        // expired in-flight row remains intact until `reclaim_stuck` returns
        // its sentinel evidence to exactly one sweeper. Returns the row we
        // wrote (or overwrote) when we won; returns nothing when the
        // predicate filtered the UPDATE. A credential that does not exist
        // fails the foreign key.
        let row: Result<Option<(Uuid, i64, DateTime<Utc>, DateTime<Utc>)>, sqlx::Error> =
            sqlx::query_as(TRY_CLAIM_SQL)
                .bind(&scope.org_id)
                .bind(&cid_str)
                .bind(new_claim_id)
                .bind(holder.as_str())
                .bind(ttl_micros)
                .bind(intent.kind().as_str())
                .bind(intent.material_epoch().map(CredentialMaterialEpoch::get))
                .bind(&scope.workspace_id)
                .fetch_optional(&mut *transaction)
                .await;
        let row = match row {
            Ok(row) => row,
            Err(error) if is_foreign_key_violation(&error) => {
                return Err(RepoError::AggregateUnavailable);
            },
            Err(_) => return Err(RepoError::Storage),
        };
        // The claim row is locked first, then the credential: an archived
        // credential is not claimable.
        let live: Option<(bool,)> = sqlx::query_as(CLAIMED_CREDENTIAL_LIVE_SQL)
            .bind(&scope.org_id)
            .bind(&scope.workspace_id)
            .bind(&cid_str)
            .fetch_optional(&mut *transaction)
            .await
            .store_err()?;
        if !matches!(live, Some((true,))) {
            return Err(RepoError::AggregateUnavailable);
        }

        if let Some((claim_id, generation, acquired, expires)) = row {
            if let CredentialOperationIntent::Revoke { material_epoch } = intent {
                // A won revoke closes credential use: the fenced admission
                // bump is the aggregate lock, taken after the claim row as in
                // every claim/credential transaction. Any failure below drops
                // the transaction, so the claim is not acquired either.
                let advanced: Option<(i64,)> = sqlx::query_as(REVOKE_ADMISSION_SQL)
                    .bind(&scope.org_id)
                    .bind(&cid_str)
                    .bind(material_epoch.get())
                    .bind(&scope.workspace_id)
                    .fetch_optional(&mut *transaction)
                    .await
                    .store_err()?;
                if advanced.is_none() {
                    let aggregate: Option<(i64, String, i64)> = sqlx::query_as(
                        "SELECT material_epoch, record_state, admission_epoch FROM credentials \
                         WHERE org_id = $1 AND workspace_id = $2 AND id = $3",
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
                    if state != "live" {
                        return Err(RepoError::AggregateUnavailable);
                    }
                    let actual = CredentialMaterialEpoch::try_from(actual)
                        .map_err(|_| RepoError::InvalidState)?;
                    if actual != material_epoch {
                        return Err(RepoError::MaterialEpochConflict {
                            expected: material_epoch,
                            actual,
                        });
                    }
                    if admission == CredentialAdmissionEpoch::MAX.get() {
                        return Err(RepoError::AdmissionEpochExhausted);
                    }
                    return Err(RepoError::InvalidState);
                }
            }
            let generation = u64::try_from(generation).map_err(|_| RepoError::InvalidState)?;
            let acquired = ClaimAttempt::Acquired(RefreshClaim {
                selector: selector.clone(),
                token: ClaimToken {
                    selector: selector.clone(),
                    claim_id,
                    generation,
                },
                acquired_at: acquired,
                expires_at: expires,
            });
            transaction.commit().await.store_err()?;
            return Ok(acquired);
        }

        // CAS lost — fetch existing row's expires_at for backoff timing.
        // If the row vanished between the failed UPSERT and this SELECT
        // (release / reclaim_stuck happened in between), surface as
        // `Contended { existing_expires_at: now }`: the caller backs off the
        // standard jitter delay and retries. Returning `InvalidState` here
        // would surface a transient race as a hard error.
        let existing: Option<(DateTime<Utc>, bool, bool, String)> = sqlx::query_as(
            "SELECT expires_at, sentinel, expires_at < CURRENT_TIMESTAMP AS expired, operation_kind \
             FROM credential_refresh_claims \
             WHERE org_id = $1 AND workspace_id = $2 AND credential_id = $3",
        )
        .bind(&scope.org_id)
        .bind(&scope.workspace_id)
        .bind(&cid_str)
        .fetch_optional(&mut *transaction)
        .await
        .store_err()?;

        let attempt = match existing {
            Some((exp, true, true, kind)) => Ok(ClaimAttempt::OutcomeUnknown {
                expired_at: exp,
                operation: CredentialOperationKind::from_wire(&kind)
                    .ok_or(RepoError::InvalidState)?,
            }),
            Some((exp, _, _, _)) => Ok(ClaimAttempt::Contended {
                existing_expires_at: exp,
            }),
            None => Ok(ClaimAttempt::Contended {
                existing_expires_at: Utc::now(),
            }),
        };
        transaction.commit().await.store_err()?;
        attempt
    }

    async fn heartbeat(&self, token: &ClaimToken, ttl: Duration) -> Result<(), HeartbeatError> {
        let ttl_micros = i64::try_from(ttl.as_micros())
            .map_err(|_| HeartbeatError::Repo(RepoError::InvalidState))?;
        let generation = i64::try_from(token.generation)
            .map_err(|_| HeartbeatError::Repo(RepoError::InvalidState))?;
        let Some(scope) = token.selector.owner().scope() else {
            return Err(HeartbeatError::ClaimLost);
        };

        let rows = sqlx::query(HEARTBEAT_SQL)
            .bind(ttl_micros)
            .bind(&scope.org_id)
            .bind(token.selector.credential_id().to_string())
            .bind(token.claim_id)
            .bind(generation)
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
             WHERE org_id = $1 AND workspace_id = $5 AND credential_id = $2 \
               AND claim_id = $3 AND generation = $4 \
               AND NOT EXISTS ( \
                   SELECT 1 FROM credential_refresh_incidents AS incident \
                   WHERE incident.claim_id = credential_refresh_claims.claim_id \
                     AND incident.adjudicated_at IS NULL \
               )",
        )
        .bind(&scope.org_id)
        .bind(token.selector.credential_id().to_string())
        .bind(token.claim_id)
        .bind(generation)
        .bind(&scope.workspace_id)
        .execute(&self.pool)
        .await
        .store_err()?
        .rows_affected();

        if rows == 0 && self.has_unresolved_incident(token.claim_id).await? {
            return Err(RepoError::ReleaseRefused);
        }
        Ok(())
    }

    async fn mark_sentinel(&self, token: &ClaimToken) -> Result<(), RepoError> {
        let generation = i64::try_from(token.generation).map_err(|_| RepoError::InvalidState)?;
        // Mirrors heartbeat's claim-validity check: zero rows affected means
        // the claim is absent, superseded, or expired. Returning Ok here
        // would authorize provider egress after the holder's TTL elapsed.
        // `CURRENT_TIMESTAMP` is evaluated by Postgres in the same transaction,
        // begun immediately before, so connection-pool wait time cannot stale
        // a caller-bound timestamp.
        //
        // Crossing the provider boundary closes credential use, so the
        // sentinel and the admission epoch commit together. The claim row is
        // locked before the credential row, the order every claim/credential
        // transaction uses. An archived credential's claim never authorizes
        // provider egress.
        let scope = token
            .selector
            .owner()
            .scope()
            .ok_or(RepoError::InvalidState)?;
        let cid_str = token.selector.credential_id().to_string();
        let mut transaction = self.pool.begin().await.store_err()?;
        let rows = sqlx::query(
            "UPDATE credential_refresh_claims \
             SET sentinel = TRUE \
             WHERE org_id = $1 AND workspace_id = $5 AND credential_id = $2 AND claim_id = $3 \
               AND generation = $4 \
               AND expires_at > CURRENT_TIMESTAMP",
        )
        .bind(&scope.org_id)
        .bind(&cid_str)
        .bind(token.claim_id)
        .bind(generation)
        .bind(&scope.workspace_id)
        .execute(&mut *transaction)
        .await
        .store_err()?
        .rows_affected();

        if rows == 0 {
            return Err(RepoError::InvalidState);
        }
        let live: Option<(bool,)> = sqlx::query_as(CLAIMED_CREDENTIAL_LIVE_SQL)
            .bind(&scope.org_id)
            .bind(&scope.workspace_id)
            .bind(&cid_str)
            .fetch_optional(&mut *transaction)
            .await
            .store_err()?;
        if !matches!(live, Some((true,))) {
            return Err(RepoError::InvalidState);
        }
        let advanced: Option<(i64,)> = sqlx::query_as(SENTINEL_ADMISSION_SQL)
            .bind(&scope.org_id)
            .bind(&cid_str)
            .bind(&scope.workspace_id)
            .fetch_optional(&mut *transaction)
            .await
            .store_err()?;
        if advanced.is_none() {
            // No live row matched: either the credential is terminal — no use
            // is left to close — or its epoch is exhausted.
            let live: Option<(i64,)> = sqlx::query_as(
                "SELECT admission_epoch FROM credentials \
                 WHERE org_id = $1 AND workspace_id = $2 AND id = $3 AND record_state = 'live'",
            )
            .bind(&scope.org_id)
            .bind(&scope.workspace_id)
            .bind(&cid_str)
            .fetch_optional(&mut *transaction)
            .await
            .store_err()?;
            match live {
                None => {},
                Some((admission,)) if admission == CredentialAdmissionEpoch::MAX.get() => {
                    return Err(RepoError::AdmissionEpochExhausted);
                },
                Some(_) => return Err(RepoError::InvalidState),
            }
        }
        transaction.commit().await.store_err()?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl RefreshClaimReclaimer for PgRefreshClaimRepo {
    async fn reclaim_stuck(
        &self,
        policy: SentinelEscalationPolicy,
    ) -> Result<Vec<ExpiredClaim>, RepoError> {
        let mut transaction = self.pool.begin().await.store_err()?;
        // Row locks serialize evidence existence-check + insert; the incident
        // key (the claim id) is the final corruption/race guard.
        // `SKIP LOCKED` lets concurrent sweepers process disjoint rows.
        let rows: Vec<(
            String,
            String,
            String,
            Uuid,
            String,
            i64,
            bool,
            String,
            Option<i64>,
        )> = sqlx::query_as(RECLAIM_SELECT_SQL)
            .fetch_all(&mut *transaction)
            .await
            .store_err()?;
        let window_micros =
            i64::try_from(policy.window().as_micros()).map_err(|_| RepoError::InvalidState)?;

        let mut out = Vec::with_capacity(rows.len());
        for (
            org_id,
            workspace_id,
            cid,
            claim_id,
            holder,
            generation,
            sentinel,
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
            if !sentinel {
                // An expired claim that never crossed the provider boundary
                // carries no evidence: delete it.
                let deleted = sqlx::query(
                    "DELETE FROM credential_refresh_claims \
                         WHERE org_id = $1 AND workspace_id = $5 AND credential_id = $2 \
                           AND claim_id = $3 AND generation = $4",
                )
                .bind(&org_id)
                .bind(&cid)
                .bind(claim_id)
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
                continue;
            }
            sqlx::query(
                        "INSERT INTO credential_refresh_incidents \
                         (org_id, workspace_id, credential_id, claim_id, detected_at, crashed_holder, \
                          generation, operation_kind, observed_material_epoch) \
                         VALUES ($1, $8, $2, $3, clock_timestamp(), $4, $5, $6, $7)",
                    )
                    .bind(&org_id)
                    .bind(&cid)
                    .bind(claim_id)
                    .bind(&holder)
                    .bind(generation)
                    .bind(operation.as_str())
                    .bind(observed_epoch)
                    .bind(&workspace_id)
                    .execute(&mut *transaction)
                    .await
                    .store_err()?;
            let (count,): (i64,) = sqlx::query_as(COUNT_SENTINEL_EVENTS_SQL)
                .bind(&org_id)
                .bind(&cid)
                .bind(window_micros)
                .bind(operation.as_str())
                .bind(&workspace_id)
                .fetch_one(&mut *transaction)
                .await
                .store_err()?;
            let event_count = u32::try_from(count).unwrap_or(u32::MAX);
            let escalation = if operation == CredentialOperationKind::Refresh
                && event_count >= policy.threshold()
            {
                let aggregate: Option<(i64, i64, bool, String, i64, bool)> = sqlx::query_as(
                    "SELECT version, material_epoch, reauth_required, record_state, \
                                        admission_epoch, deleted_at IS NOT NULL \
                                 FROM credentials \
                                 WHERE org_id = $1 AND workspace_id = $2 AND id = $3 FOR UPDATE",
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
                        if reauth_required {
                            ReauthEscalation::ReauthRequired {
                                changed: false,
                                version,
                                material_epoch: epoch,
                            }
                        } else {
                            let next_version =
                                version.next_live().map_err(|_| RepoError::InvalidState)?;
                            let next_epoch = epoch.next().map_err(|_| RepoError::InvalidState)?;
                            let next_admission = CredentialAdmissionEpoch::try_from(admission)
                                .map_err(|_| RepoError::InvalidState)?
                                .next()
                                .map_err(|_| RepoError::AdmissionEpochExhausted)?;
                            let updated: Option<(i64, i64)> = sqlx::query_as(
                                "UPDATE credentials SET reauth_required = TRUE, \
                                             version = $3, material_epoch = $4, \
                                             admission_epoch = $5, \
                                             updated_at = clock_timestamp(), \
                                             refresh_retry_mode = NULL, \
                                             refresh_retry_not_before = NULL, \
                                             refresh_retry_phase = NULL, \
                                             refresh_retry_kind = NULL, \
                                             refresh_retry_diagnostic_code = NULL \
                                         WHERE org_id = $1 AND workspace_id = $6 AND id = $2 \
                                           AND record_state = 'live' AND reauth_required = FALSE \
                                         RETURNING version, material_epoch",
                            )
                            .bind(&org_id)
                            .bind(&cid)
                            .bind(next_version.get())
                            .bind(next_epoch.get())
                            .bind(next_admission.get())
                            .bind(&workspace_id)
                            .fetch_optional(&mut *transaction)
                            .await
                            .store_err()?;
                            if updated.is_none() {
                                return Err(RepoError::InvalidState);
                            }
                            ReauthEscalation::ReauthRequired {
                                changed: true,
                                version: next_version,
                                material_epoch: next_epoch,
                            }
                        }
                    },
                    Some((_, _, _, state, _, false)) if state == "tombstoned" => {
                        ReauthEscalation::AggregateTerminal
                    },
                    Some(_) => return Err(RepoError::InvalidState),
                    None => return Err(RepoError::InvalidState),
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
        }

        transaction.commit().await.store_err()?;
        Ok(out)
    }
}

impl PgRefreshClaimRepo {
    /// Does an incident for `claim_id` still lack a recorded provider outcome?
    async fn has_unresolved_incident(&self, claim_id: Uuid) -> Result<bool, RepoError> {
        let (exists,): (bool,) = sqlx::query_as(UNRESOLVED_INCIDENT_SQL)
            .bind(claim_id)
            .fetch_one(&self.pool)
            .await
            .store_err()?;
        Ok(exists)
    }
}

#[async_trait::async_trait]
impl RefreshClaimAdjudicator for PgRefreshClaimRepo {
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

        // One transaction: the poison is cleared and its resolution recorded
        // together, or neither is. The `FOR UPDATE` on the poison lookup
        // serializes two concurrent adjudications of the same credential, so
        // exactly one of them finds the row.
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(|_| RepoAdjudicationError::Storage)?;

        let poisoned: Option<(Uuid, String, i64, String, Option<i64>)> =
            sqlx::query_as(POISONED_CLAIM_SQL)
                .bind(&scope.org_id)
                .bind(&cid_str)
                .bind(&scope.workspace_id)
                .fetch_optional(&mut *transaction)
                .await
                .map_err(|_| RepoAdjudicationError::Storage)?;

        // An archived credential is unusable, its incidents included. Read
        // after the claim lock, the order every claim/credential transaction
        // uses.
        let archived: Option<(bool,)> = sqlx::query_as(
            "SELECT deleted_at IS NOT NULL FROM credentials \
             WHERE org_id = $1 AND workspace_id = $2 AND id = $3 FOR SHARE",
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

        // The named incident's own resolution answers first. It is read after
        // the `FOR UPDATE` above, so a concurrent adjudication of the same
        // incident that committed while this one waited on the lock is seen
        // here and answered as the idempotent recommit it is.
        let recorded: Option<(Vec<u8>, Option<String>, String)> =
            sqlx::query_as(INCIDENT_RESOLUTION_SQL)
                .bind(&scope.org_id)
                .bind(&cid_str)
                .bind(incident.as_uuid())
                .bind(&scope.workspace_id)
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

        let Some((claim_id, crashed_holder, generation, operation_raw, observed_epoch)) = poisoned
        else {
            return Err(RepoAdjudicationError::NotPoisoned);
        };
        // A different poisoned claim is a different incident: the decision
        // was established for the named one and says nothing about this one.
        if claim_id != incident.as_uuid() {
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
                     WHERE org_id = $1 AND workspace_id = $2 AND id = $3 AND deleted_at IS NULL \
                     FOR UPDATE",
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
                            "WITH mutation_clock AS MATERIALIZED (SELECT clock_timestamp() AS now) \
                             UPDATE credentials SET name = NULL, data = ''::bytea, \
                             version = $3, updated_at = mutation_clock.now, expires_at = NULL, \
                             reauth_required = FALSE, metadata = '{}', record_state = 'tombstoned', \
                             tombstoned_at = mutation_clock.now, refresh_retry_mode = NULL, \
                             refresh_retry_not_before = NULL, refresh_retry_phase = NULL, \
                             refresh_retry_kind = NULL, refresh_retry_diagnostic_code = NULL \
                             FROM mutation_clock \
                             WHERE org_id = $1 AND workspace_id = $5 AND id = $2 \
                               AND record_state = 'live' AND version = $4",
                        )
                        .bind(&scope.org_id)
                        .bind(&cid_str)
                        .bind(next.get())
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
                .bind(&cid_str)
                .bind(claim_id)
                .bind(generation)
                .bind(&scope.workspace_id)
                .execute(&mut *transaction)
                .await
                .map_err(|_| RepoAdjudicationError::Storage)?
                .rows_affected();
            if cleared != 1 {
                return Err(RepoAdjudicationError::Storage);
            }

            let recorded: Option<(Uuid,)> = sqlx::query_as(RECORD_RESOLUTION_SQL)
                .bind(&scope.org_id)
                .bind(&cid_str)
                .bind(claim_id)
                .bind(&crashed_holder)
                .bind(generation)
                .bind(decision.as_str())
                .bind(evidence)
                .bind(digest.as_slice())
                .bind(operation.as_str())
                .bind(observed_epoch)
                .bind(&scope.workspace_id)
                .fetch_optional(&mut *transaction)
                .await
                .map_err(|_| RepoAdjudicationError::Storage)?;

            // `RETURNING` yields a row only when this call recorded the
            // resolution. The named incident had no resolution when this
            // transaction read it under the claim lock, and the claim row
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
    use super::{
        COUNT_SENTINEL_EVENTS_SQL, HEARTBEAT_SQL, INCIDENT_RESOLUTION_SQL, POISONED_CLAIM_SQL,
        RECLAIM_SELECT_SQL, TRY_CLAIM_SQL,
    };

    #[test]
    fn postgres_is_the_lease_clock_authority() {
        assert!(
            TRY_CLAIM_SQL.contains(
                "VALUES ( \
         $1, $8, $2, $3, 0, $4, CURRENT_TIMESTAMP"
            ),
            "acquisition time must come from PostgreSQL"
        );
        assert!(
            TRY_CLAIM_SQL.contains("expires_at < CURRENT_TIMESTAMP"),
            "takeover must compare expiry with the PostgreSQL clock"
        );
        assert!(
            TRY_CLAIM_SQL.contains("RETURNING claim_id, generation, acquired_at, expires_at"),
            "the caller must receive the database-authored lease timestamps"
        );
        assert!(
            HEARTBEAT_SQL.contains("expires_at > CURRENT_TIMESTAMP"),
            "heartbeat admission must use the PostgreSQL clock"
        );
        assert!(
            RECLAIM_SELECT_SQL.contains("expires_at < CURRENT_TIMESTAMP"),
            "reclaim eligibility must use the PostgreSQL clock"
        );
        assert!(
            COUNT_SENTINEL_EVENTS_SQL
                .contains("detected_at > clock_timestamp() - ($3 * INTERVAL '1 microsecond')"),
            "sentinel windows must be derived from the PostgreSQL clock"
        );
    }

    #[test]
    fn reclaim_query_excludes_accounted_poison_before_row_locking() {
        let evidence_filter = RECLAIM_SELECT_SQL
            .find("OR NOT EXISTS")
            .expect("reclaim query must exclude already-accounted poison");
        let row_lock = RECLAIM_SELECT_SQL
            .find("FOR UPDATE SKIP LOCKED")
            .expect("reclaim query must lock only selected work");

        assert!(
            evidence_filter < row_lock,
            "accounted poison must be filtered before rows are locked"
        );
        assert!(
            RECLAIM_SELECT_SQL.contains("WHERE incident.claim_id = claim.claim_id"),
            "incident identity must use the globally unique claim UUID"
        );
    }

    #[test]
    fn poison_detection_is_keyed_on_the_claim_row() {
        assert!(
            POISONED_CLAIM_SQL.contains("FROM credential_refresh_claims"),
            "the poisoned claim row is the poison"
        );
        assert!(
            POISONED_CLAIM_SQL.contains("expires_at < CURRENT_TIMESTAMP"),
            "poison must be compared against the same clock `try_claim` refuses on"
        );
        assert!(POISONED_CLAIM_SQL.contains("AND sentinel"));
        assert!(
            POISONED_CLAIM_SQL.contains("FOR UPDATE"),
            "the poison lookup must serialize concurrent adjudications"
        );
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
            "org_id = $1 AND workspace_id = $4 AND credential_id = $2 AND claim_id = $3"
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
