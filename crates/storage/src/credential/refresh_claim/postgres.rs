//! Postgres-backed `RefreshClaimRepo` impl.
//!
//! Multi-replica production target. Atomic CAS via
//! `INSERT ... ON CONFLICT (credential_id) DO UPDATE WHERE
//! credential_refresh_claims.expires_at < CURRENT_TIMESTAMP
//! AND sentinel = Normal`
//! pattern, mirroring control-queue claim acquisition.
//!
//! PostgreSQL is the lease-clock authority: acquisition, heartbeat,
//! sentinel admission, and reclaim all compare against the database clock.

use std::time::Duration;

use chrono::{DateTime, Utc};
use nebula_core::CredentialId;
use sqlx::PgPool;
use uuid::Uuid;

use super::{
    ClaimAttempt, ClaimToken, ExpiredClaim, HeartbeatError, RefreshClaim, RefreshClaimRepo,
    ReplicaId, RepoError, SqlxClaimResultExt,
};

const TRY_CLAIM_SQL: &str = "INSERT INTO credential_refresh_claims \
     (credential_id, claim_id, generation, holder_replica_id, \
      acquired_at, expires_at, sentinel) \
     VALUES ( \
         $1, $2, 0, $3, CURRENT_TIMESTAMP, \
         CURRENT_TIMESTAMP + ($4 * INTERVAL '1 microsecond'), 0 \
     ) \
     ON CONFLICT (credential_id) DO UPDATE \
     SET claim_id = EXCLUDED.claim_id, \
         generation = credential_refresh_claims.generation + 1, \
         holder_replica_id = EXCLUDED.holder_replica_id, \
         acquired_at = EXCLUDED.acquired_at, \
         expires_at = EXCLUDED.expires_at, \
         sentinel = 0 \
     WHERE credential_refresh_claims.expires_at < CURRENT_TIMESTAMP \
       AND credential_refresh_claims.sentinel = 0 \
     RETURNING claim_id, generation, acquired_at, expires_at";

const HEARTBEAT_SQL: &str = "UPDATE credential_refresh_claims \
     SET expires_at = CURRENT_TIMESTAMP + ($1 * INTERVAL '1 microsecond') \
     WHERE claim_id = $2 \
       AND generation = $3 \
       AND expires_at > CURRENT_TIMESTAMP";

const RECLAIM_SELECT_SQL: &str = "SELECT \
         credential_id, claim_id, holder_replica_id, generation, sentinel \
     FROM credential_refresh_claims AS claim \
     WHERE expires_at < CURRENT_TIMESTAMP \
       AND ( \
           sentinel = 0 \
           OR ( \
               sentinel = 1 \
               AND NOT EXISTS ( \
                   SELECT 1 FROM credential_sentinel_events AS event \
                   WHERE event.credential_id = claim.credential_id \
                     AND event.claim_id = claim.claim_id \
               ) \
           ) \
       ) \
     FOR UPDATE SKIP LOCKED";

const COUNT_SENTINEL_EVENTS_SQL: &str = "SELECT COUNT(*) \
     FROM credential_sentinel_events \
     WHERE credential_id = $1 \
       AND detected_at > CURRENT_TIMESTAMP - ($2 * INTERVAL '1 microsecond')";

/// Postgres-backed `RefreshClaimRepo`.
#[derive(Clone, Debug)]
pub struct PgRefreshClaimRepo {
    pool: PgPool,
}

impl PgRefreshClaimRepo {
    /// Wrap an existing pool. Caller is responsible for running migrations
    /// through 0039.
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn parse_credential_id(s: &str) -> Result<CredentialId, RepoError> {
    s.parse::<CredentialId>()
        .map_err(|_| RepoError::InvalidState)
}

#[async_trait::async_trait]
impl RefreshClaimRepo for PgRefreshClaimRepo {
    async fn try_claim(
        &self,
        credential_id: &CredentialId,
        holder: &ReplicaId,
        ttl: Duration,
    ) -> Result<ClaimAttempt, RepoError> {
        let new_claim_id = Uuid::new_v4();
        let ttl_micros = i64::try_from(ttl.as_micros()).map_err(|_| RepoError::InvalidState)?;
        let cid_str = credential_id.to_string();

        // Atomic CAS: INSERT, or UPDATE only an expired Normal row. An
        // expired in-flight row remains intact until `reclaim_stuck` returns
        // its sentinel evidence to exactly one sweeper. Returns the row we
        // wrote (or overwrote) when we won; returns nothing when the
        // predicate filtered the UPDATE.
        let row: Option<(Uuid, i64, DateTime<Utc>, DateTime<Utc>)> = sqlx::query_as(TRY_CLAIM_SQL)
            .bind(&cid_str)
            .bind(new_claim_id)
            .bind(holder.as_str())
            .bind(ttl_micros)
            .fetch_optional(&self.pool)
            .await
            .store_err()?;

        if let Some((claim_id, generation, acquired, expires)) = row {
            let generation = u64::try_from(generation).map_err(|_| RepoError::InvalidState)?;
            return Ok(ClaimAttempt::Acquired(RefreshClaim {
                credential_id: *credential_id,
                token: ClaimToken {
                    claim_id,
                    generation,
                },
                acquired_at: acquired,
                expires_at: expires,
            }));
        }

        // CAS lost — fetch existing row's expires_at for backoff timing.
        // If the row vanished between the failed UPSERT and this SELECT
        // (release / reclaim_stuck happened in between), surface as
        // `Contended { existing_expires_at: now }`: the caller backs off the
        // standard jitter delay and retries. Returning `InvalidState` here
        // would surface a transient race as a hard error.
        let existing: Option<(DateTime<Utc>, i16, bool)> = sqlx::query_as(
            "SELECT expires_at, sentinel, expires_at < CURRENT_TIMESTAMP AS expired \
             FROM credential_refresh_claims \
             WHERE credential_id = $1",
        )
        .bind(&cid_str)
        .fetch_optional(&self.pool)
        .await
        .store_err()?;

        match existing {
            Some((exp, 1, true)) => Ok(ClaimAttempt::OutcomeUnknown { expired_at: exp }),
            Some((exp, 0 | 1, _)) => Ok(ClaimAttempt::Contended {
                existing_expires_at: exp,
            }),
            Some((_, _, _)) => Err(RepoError::InvalidState),
            None => Ok(ClaimAttempt::Contended {
                existing_expires_at: Utc::now(),
            }),
        }
    }

    async fn heartbeat(&self, token: &ClaimToken, ttl: Duration) -> Result<(), HeartbeatError> {
        let ttl_micros = i64::try_from(ttl.as_micros())
            .map_err(|_| HeartbeatError::Repo(RepoError::InvalidState))?;
        let generation = i64::try_from(token.generation)
            .map_err(|_| HeartbeatError::Repo(RepoError::InvalidState))?;

        let rows = sqlx::query(HEARTBEAT_SQL)
            .bind(ttl_micros)
            .bind(token.claim_id)
            .bind(generation)
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
        sqlx::query(
            "DELETE FROM credential_refresh_claims \
             WHERE claim_id = $1 AND generation = $2",
        )
        .bind(token.claim_id)
        .bind(generation)
        .execute(&self.pool)
        .await
        .store_err()?;
        Ok(())
    }

    async fn mark_sentinel(&self, token: &ClaimToken) -> Result<(), RepoError> {
        let generation = i64::try_from(token.generation).map_err(|_| RepoError::InvalidState)?;
        // Mirrors heartbeat's claim-validity check: zero rows affected means
        // the claim is absent, superseded, or expired. Returning Ok here
        // would authorize provider egress after the holder's TTL elapsed.
        // `CURRENT_TIMESTAMP` is evaluated by Postgres in the same statement,
        // so connection-pool wait time cannot stale a caller-bound timestamp.
        let rows = sqlx::query(
            "UPDATE credential_refresh_claims \
             SET sentinel = 1 \
             WHERE claim_id = $1 \
               AND generation = $2 \
               AND expires_at > CURRENT_TIMESTAMP",
        )
        .bind(token.claim_id)
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

    async fn reclaim_stuck(&self) -> Result<Vec<ExpiredClaim>, RepoError> {
        let mut transaction = self.pool.begin().await.store_err()?;
        // Row locks serialize evidence existence-check + insert; the global
        // partial unique claim-id index is the final corruption/race guard.
        // `SKIP LOCKED` lets concurrent sweepers process disjoint rows.
        let rows: Vec<(String, Uuid, String, i64, i16)> = sqlx::query_as(RECLAIM_SELECT_SQL)
            .fetch_all(&mut *transaction)
            .await
            .store_err()?;

        let mut out = Vec::with_capacity(rows.len());
        for (cid, claim_id, holder, generation, sentinel_raw) in rows {
            let credential_id = parse_credential_id(&cid)?;
            let previous_generation =
                u64::try_from(generation).map_err(|_| RepoError::InvalidState)?;
            match sentinel_raw {
                0 => {
                    let deleted = sqlx::query(
                        "DELETE FROM credential_refresh_claims \
                         WHERE credential_id = $1 AND claim_id = $2 AND generation = $3",
                    )
                    .bind(&cid)
                    .bind(claim_id)
                    .bind(generation)
                    .execute(&mut *transaction)
                    .await
                    .store_err()?
                    .rows_affected();
                    if deleted != 1 {
                        return Err(RepoError::InvalidState);
                    }
                    out.push(ExpiredClaim::ReclaimedNormal {
                        credential_id,
                        previous_holder: ReplicaId::new(holder),
                        previous_generation,
                    });
                },
                1 => {
                    sqlx::query(
                        "INSERT INTO credential_sentinel_events \
                         (credential_id, claim_id, detected_at, crashed_holder, generation) \
                         VALUES ($1, $2, CURRENT_TIMESTAMP, $3, $4)",
                    )
                    .bind(&cid)
                    .bind(claim_id)
                    .bind(&holder)
                    .bind(generation)
                    .execute(&mut *transaction)
                    .await
                    .store_err()?;
                    out.push(ExpiredClaim::OutcomeUnknownAccounted {
                        credential_id,
                        previous_holder: ReplicaId::new(holder),
                        previous_generation,
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

    async fn count_sentinel_events_in_window(
        &self,
        credential_id: &CredentialId,
        window: Duration,
    ) -> Result<u32, RepoError> {
        let cid_str = credential_id.to_string();
        let window_micros =
            i64::try_from(window.as_micros()).map_err(|_| RepoError::InvalidState)?;
        let (count,): (i64,) = sqlx::query_as(COUNT_SENTINEL_EVENTS_SQL)
            .bind(&cid_str)
            .bind(window_micros)
            .fetch_one(&self.pool)
            .await
            .store_err()?;
        Ok(u32::try_from(count).unwrap_or(u32::MAX))
    }
}

#[cfg(test)]
mod tests {
    use super::{COUNT_SENTINEL_EVENTS_SQL, HEARTBEAT_SQL, RECLAIM_SELECT_SQL, TRY_CLAIM_SQL};

    #[test]
    fn postgres_is_the_lease_clock_authority() {
        assert!(
            TRY_CLAIM_SQL.contains(
                "VALUES ( \
         $1, $2, 0, $3, CURRENT_TIMESTAMP"
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
                .contains("detected_at > CURRENT_TIMESTAMP - ($2 * INTERVAL '1 microsecond')"),
            "sentinel windows must be derived from the PostgreSQL clock"
        );
    }

    #[test]
    fn reclaim_query_excludes_accounted_poison_before_row_locking() {
        let evidence_filter = RECLAIM_SELECT_SQL
            .find("AND NOT EXISTS")
            .expect("reclaim query must exclude already-accounted poison");
        let row_lock = RECLAIM_SELECT_SQL
            .find("FOR UPDATE SKIP LOCKED")
            .expect("reclaim query must lock only selected work");

        assert!(
            evidence_filter < row_lock,
            "accounted poison must be filtered before rows are locked"
        );
        assert!(
            RECLAIM_SELECT_SQL.contains(
                "event.credential_id = claim.credential_id \
                     AND event.claim_id = claim.claim_id"
            ),
            "incident identity must use the globally unique claim UUID"
        );
    }
}
