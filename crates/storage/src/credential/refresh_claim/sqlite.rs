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
use sqlx::SqlitePool;
use uuid::Uuid;

use super::{
    ClaimAttempt, ClaimToken, ExpiredClaim, HeartbeatError, RefreshClaim, RefreshClaimRepo,
    ReplicaId, RepoError, SqlxClaimResultExt,
};

const COUNT_SENTINEL_EVENTS_SQL: &str = "SELECT COUNT(*) \
     FROM credential_sentinel_events \
     WHERE credential_id = ?1 \
       AND detected_at > ( \
           unixepoch('now') * 1000 \
           + CAST(substr(strftime('%f', 'now'), 4, 3) AS INTEGER) \
           - ?2 \
       )";

/// SQLite-backed `RefreshClaimRepo`.
#[derive(Clone, Debug)]
pub struct SqliteRefreshClaimRepo {
    pool: SqlitePool,
}

impl SqliteRefreshClaimRepo {
    /// Wrap an existing pool. Caller is responsible for running migrations
    /// through 0039.
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
        credential_id: &CredentialId,
        holder: &ReplicaId,
        ttl: Duration,
    ) -> Result<ClaimAttempt, RepoError> {
        let now = Utc::now();
        let new_claim_id = Uuid::new_v4();
        let new_expires =
            now + chrono::Duration::from_std(ttl).map_err(|_| RepoError::InvalidState)?;
        let cid_str = credential_id.to_string();
        let holder_str = holder.as_str();
        let now_ms = now.timestamp_millis();
        let exp_ms = new_expires.timestamp_millis();
        let claim_id_str = new_claim_id.to_string();

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
             (credential_id, claim_id, generation, holder_replica_id, \
              acquired_at, expires_at, sentinel) \
             VALUES (?1, ?2, 0, ?3, ?4, ?5, 0) \
             ON CONFLICT(credential_id) DO UPDATE SET \
                 claim_id = excluded.claim_id, \
                 generation = credential_refresh_claims.generation + 1, \
                 holder_replica_id = excluded.holder_replica_id, \
                 acquired_at = excluded.acquired_at, \
                 expires_at = excluded.expires_at, \
                 sentinel = 0 \
             WHERE credential_refresh_claims.expires_at < ?4 \
               AND credential_refresh_claims.sentinel = 0 \
             RETURNING claim_id, generation, acquired_at, expires_at",
        )
        .bind(&cid_str)
        .bind(&claim_id_str)
        .bind(holder_str)
        .bind(now_ms)
        .bind(exp_ms)
        .fetch_optional(&self.pool)
        .await
        .store_err()?;

        if let Some((claim_id_str, generation, acquired_ms, expires_ms)) = row {
            let acquired_at = millis_to_utc(acquired_ms)?;
            let expires_at = millis_to_utc(expires_ms)?;
            let claim_id = claim_id_str
                .parse::<Uuid>()
                .map_err(|_| RepoError::InvalidState)?;
            let generation = u64::try_from(generation).map_err(|_| RepoError::InvalidState)?;
            return Ok(ClaimAttempt::Acquired(RefreshClaim {
                credential_id: *credential_id,
                token: ClaimToken {
                    claim_id,
                    generation,
                },
                acquired_at,
                expires_at,
            }));
        }

        // CAS lost — fetch existing row's expires_at for the backoff hint.
        // If the row vanished between the failed UPSERT and this SELECT
        // (release / reclaim_stuck happened in between), surface as
        // `Contended { existing_expires_at: now }`: the caller backs off the
        // standard jitter delay and retries. Returning `InvalidState` here
        // would surface a transient race as a hard error.
        let existing: Option<(i64, i64)> = sqlx::query_as(
            "SELECT expires_at, sentinel \
             FROM credential_refresh_claims \
             WHERE credential_id = ?1",
        )
        .bind(&cid_str)
        .fetch_optional(&self.pool)
        .await
        .store_err()?;

        match existing {
            Some((exp_ms, 1)) if exp_ms < now_ms => Ok(ClaimAttempt::OutcomeUnknown {
                expired_at: millis_to_utc(exp_ms)?,
            }),
            Some((exp_ms, 0 | 1)) => Ok(ClaimAttempt::Contended {
                existing_expires_at: millis_to_utc(exp_ms)?,
            }),
            Some((_, _)) => Err(RepoError::InvalidState),
            None => Ok(ClaimAttempt::Contended {
                existing_expires_at: now,
            }),
        }
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
             WHERE claim_id = ?2 AND generation = ?3 AND expires_at > ?4",
        )
        .bind(extension_ms)
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
        sqlx::query(
            "DELETE FROM credential_refresh_claims \
             WHERE claim_id = ?1 AND generation = ?2",
        )
        .bind(&claim_id_str)
        .bind(generation)
        .execute(&self.pool)
        .await
        .store_err()?;
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
             WHERE claim_id = ?1 \
               AND generation = ?2 \
               AND expires_at > ( \
                   unixepoch('now') * 1000 \
                   + CAST(substr(strftime('%f', 'now'), 4, 3) AS INTEGER) \
               )",
        )
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

    async fn reclaim_stuck(&self) -> Result<Vec<ExpiredClaim>, RepoError> {
        let now = Utc::now();
        let now_ms = now.timestamp_millis();
        // `BEGIN IMMEDIATE` takes SQLite's write lock before reading expired
        // rows. That serializes existence-check + event insert for poisoned
        // rows and delete for Normal rows into one atomic boundary.
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await.store_err()?;
        let rows: Vec<(String, String, String, i64, i64)> = sqlx::query_as(
            "SELECT credential_id, claim_id, holder_replica_id, generation, sentinel \
             FROM credential_refresh_claims AS claim \
             WHERE expires_at < ?1 \
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
               )",
        )
        .bind(now_ms)
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
                         WHERE credential_id = ?1 AND claim_id = ?2 AND generation = ?3",
                    )
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
                        credential_id,
                        previous_holder: ReplicaId::new(holder),
                        previous_generation,
                    });
                },
                1 => {
                    sqlx::query(
                        "INSERT INTO credential_sentinel_events \
                         (credential_id, claim_id, detected_at, crashed_holder, generation) \
                         VALUES (?1, ?2, ?3, ?4, ?5)",
                    )
                    .bind(&cid)
                    .bind(&claim_id)
                    .bind(now_ms)
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
        let window_ms = i64::try_from(window.as_millis()).map_err(|_| RepoError::InvalidState)?;
        let (count,): (i64,) = sqlx::query_as(COUNT_SENTINEL_EVENTS_SQL)
            .bind(&cid_str)
            .bind(window_ms)
            .fetch_one(&self.pool)
            .await
            .store_err()?;
        Ok(u32::try_from(count).unwrap_or(u32::MAX))
    }
}

#[cfg(test)]
mod tests {
    use super::COUNT_SENTINEL_EVENTS_SQL;

    #[test]
    fn sentinel_window_uses_the_sqlite_clock() {
        assert!(
            COUNT_SENTINEL_EVENTS_SQL.contains("unixepoch('now') * 1000"),
            "sentinel windows must be derived from SQLite's clock"
        );
        assert!(
            COUNT_SENTINEL_EVENTS_SQL.contains("- ?2"),
            "the caller may provide only a duration"
        );
    }
}
