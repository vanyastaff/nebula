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
    ClaimAttempt, ClaimToken, HeartbeatError, ReclaimedClaim, RefreshClaim, RefreshClaimRepo,
    ReplicaId, RepoError, SentinelState,
};

/// SQLite-backed `RefreshClaimRepo`.
#[derive(Clone, Debug)]
pub struct SqliteRefreshClaimRepo {
    pool: SqlitePool,
}

impl SqliteRefreshClaimRepo {
    /// Wrap an existing pool. Caller is responsible for running migrations
    /// 0022/0023.
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

fn parse_credential_id(s: &str) -> Result<CredentialId, RepoError> {
    s.parse::<CredentialId>()
        .map_err(|e| RepoError::InvalidState(format!("bad credential_id `{s}`: {e}")))
}

/// Convert a millisecond-since-epoch column back to a `DateTime<Utc>`.
///
/// SQLite stores timestamps as `INTEGER` per migration 0022/0023; this is the
/// inverse of `DateTime::timestamp_millis()`. An out-of-range value indicates
/// table corruption (we never write such values), surfaced as `InvalidState`.
fn millis_to_utc(ms: i64) -> Result<DateTime<Utc>, RepoError> {
    Utc.timestamp_millis_opt(ms)
        .single()
        .ok_or_else(|| RepoError::InvalidState(format!("timestamp millis out of range: {ms}")))
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
        let new_expires = now
            + chrono::Duration::from_std(ttl)
                .map_err(|e| RepoError::InvalidState(format!("invalid ttl: {e}")))?;
        let cid_str = credential_id.to_string();
        let holder_str = holder.as_str();
        let now_ms = now.timestamp_millis();
        let exp_ms = new_expires.timestamp_millis();
        let claim_id_str = new_claim_id.to_string();

        // Atomic CAS via UPSERT with conditional UPDATE clause. Mirrors the
        // Postgres `INSERT ... ON CONFLICT DO UPDATE WHERE expires_at < ...`
        // pattern (ADR-0008 + ADR-0041). Requires SQLite 3.35+ for
        // `RETURNING`.
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
             RETURNING claim_id, generation, acquired_at, expires_at",
        )
        .bind(&cid_str)
        .bind(&claim_id_str)
        .bind(holder_str)
        .bind(now_ms)
        .bind(exp_ms)
        .fetch_optional(&self.pool)
        .await?;

        if let Some((claim_id_str, generation, acquired_ms, expires_ms)) = row {
            let acquired_at = millis_to_utc(acquired_ms)?;
            let expires_at = millis_to_utc(expires_ms)?;
            let claim_id = claim_id_str
                .parse::<Uuid>()
                .map_err(|e| RepoError::InvalidState(format!("bad claim_id: {e}")))?;
            return Ok(ClaimAttempt::Acquired(RefreshClaim {
                credential_id: *credential_id,
                token: ClaimToken {
                    claim_id,
                    generation: generation as u64,
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
        let existing: Option<(i64,)> = sqlx::query_as(
            "SELECT expires_at FROM credential_refresh_claims WHERE credential_id = ?1",
        )
        .bind(&cid_str)
        .fetch_optional(&self.pool)
        .await?;

        match existing {
            Some((exp_ms,)) => Ok(ClaimAttempt::Contended {
                existing_expires_at: millis_to_utc(exp_ms)?,
            }),
            None => Ok(ClaimAttempt::Contended {
                existing_expires_at: now,
            }),
        }
    }

    async fn heartbeat(&self, token: &ClaimToken, ttl: Duration) -> Result<(), HeartbeatError> {
        let now = Utc::now();
        let now_ms = now.timestamp_millis();
        let extension_ms = (now
            + chrono::Duration::from_std(ttl).map_err(|e| {
                HeartbeatError::Repo(RepoError::InvalidState(format!("invalid ttl: {e}")))
            })?)
        .timestamp_millis();
        let claim_id_str = token.claim_id.to_string();

        let rows = sqlx::query(
            "UPDATE credential_refresh_claims \
             SET expires_at = ?1 \
             WHERE claim_id = ?2 AND generation = ?3 AND expires_at > ?4",
        )
        .bind(extension_ms)
        .bind(&claim_id_str)
        .bind(token.generation as i64)
        .bind(now_ms)
        .execute(&self.pool)
        .await
        .map_err(RepoError::from)?
        .rows_affected();

        if rows == 0 {
            return Err(HeartbeatError::ClaimLost);
        }
        Ok(())
    }

    async fn release(&self, token: ClaimToken) -> Result<(), RepoError> {
        let claim_id_str = token.claim_id.to_string();
        sqlx::query(
            "DELETE FROM credential_refresh_claims \
             WHERE claim_id = ?1 AND generation = ?2",
        )
        .bind(&claim_id_str)
        .bind(token.generation as i64)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn mark_sentinel(&self, token: &ClaimToken) -> Result<(), RepoError> {
        let claim_id_str = token.claim_id.to_string();
        // Mirrors heartbeat's claim-loss check: zero rows affected means
        // the claim row was reclaimed (different generation or deleted)
        // and another replica owns the credential. Returning Ok here would
        // let the holder proceed to the IdP POST while another replica
        // already owns the row.
        let rows = sqlx::query(
            "UPDATE credential_refresh_claims \
             SET sentinel = 1 \
             WHERE claim_id = ?1 AND generation = ?2",
        )
        .bind(&claim_id_str)
        .bind(token.generation as i64)
        .execute(&self.pool)
        .await?
        .rows_affected();

        if rows == 0 {
            return Err(RepoError::InvalidState(
                "mark_sentinel: claim lost — token no longer owns the row".to_string(),
            ));
        }
        Ok(())
    }

    async fn reclaim_stuck(&self) -> Result<Vec<ReclaimedClaim>, RepoError> {
        let now = Utc::now();
        let now_ms = now.timestamp_millis();

        // Atomic reclaim via `DELETE ... RETURNING` (SQLite 3.35+, same
        // version that enables UPSERT RETURNING in `try_claim`). Two
        // sweepers running concurrently each get a disjoint subset of the
        // expired rows — no row is observed by both, which preserves the
        // §3.4 sentinel-event invariant (one event per stuck refresh,
        // never double-counted toward the N=3 ReauthRequired threshold).
        let rows: Vec<(String, String, i64, i64)> = sqlx::query_as(
            "DELETE FROM credential_refresh_claims \
             WHERE expires_at < ?1 \
             RETURNING credential_id, holder_replica_id, generation, sentinel",
        )
        .bind(now_ms)
        .fetch_all(&self.pool)
        .await?;

        // The DELETE has already committed; short-circuiting on the first
        // bad row would silently abandon every other already-deleted row,
        // including ones with `sentinel = 1` that the engine reclaim sweep
        // must turn into `credential_sentinel_events` inserts. Skip the bad
        // row with a `warn!` and continue draining survivors.
        let mut out = Vec::with_capacity(rows.len());
        for (cid, holder, generation, sentinel_raw) in rows {
            match parse_credential_id(&cid) {
                Ok(credential_id) => {
                    out.push(ReclaimedClaim {
                        credential_id,
                        previous_holder: ReplicaId::new(holder),
                        previous_generation: generation as u64,
                        sentinel: if sentinel_raw == 1 {
                            SentinelState::RefreshInFlight
                        } else {
                            SentinelState::Normal
                        },
                    });
                },
                Err(error) => {
                    tracing::warn!(
                        cid,
                        %error,
                        "reclaim_stuck: skipping deleted row with unparsable credential_id"
                    );
                },
            }
        }

        Ok(out)
    }

    async fn record_sentinel_event(
        &self,
        credential_id: &CredentialId,
        crashed_holder: &ReplicaId,
        generation: u64,
    ) -> Result<(), RepoError> {
        let cid_str = credential_id.to_string();
        let now_ms = Utc::now().timestamp_millis();
        sqlx::query(
            "INSERT INTO credential_sentinel_events \
             (credential_id, detected_at, crashed_holder, generation) \
             VALUES (?1, ?2, ?3, ?4)",
        )
        .bind(&cid_str)
        .bind(now_ms)
        .bind(crashed_holder.as_str())
        .bind(generation as i64)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn count_sentinel_events_in_window(
        &self,
        credential_id: &CredentialId,
        window_start: DateTime<Utc>,
    ) -> Result<u32, RepoError> {
        let cid_str = credential_id.to_string();
        let window_ms = window_start.timestamp_millis();
        let (count,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM credential_sentinel_events \
             WHERE credential_id = ?1 AND detected_at > ?2",
        )
        .bind(&cid_str)
        .bind(window_ms)
        .fetch_one(&self.pool)
        .await?;
        Ok(u32::try_from(count).unwrap_or(u32::MAX))
    }
}
