//! Postgres-backed `RefreshClaimRepo` impl.
//!
//! Multi-replica production target. Atomic CAS via
//! `INSERT ... ON CONFLICT (credential_id) DO UPDATE WHERE
//! credential_refresh_claims.expires_at < EXCLUDED.expires_at`
//! pattern, mirroring control-queue claim acquisition (ADR-0008).

use std::time::Duration;

use chrono::{DateTime, Utc};
use nebula_core::CredentialId;
use sqlx::PgPool;
use uuid::Uuid;

use super::{
    ClaimAttempt, ClaimToken, HeartbeatError, ReclaimedClaim, RefreshClaim, RefreshClaimRepo,
    ReplicaId, RepoError, SentinelState,
};

/// Postgres-backed `RefreshClaimRepo`.
#[derive(Clone, Debug)]
pub struct PgRefreshClaimRepo {
    pool: PgPool,
}

impl PgRefreshClaimRepo {
    /// Wrap an existing pool. Caller is responsible for running migrations
    /// 0022/0023.
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn parse_credential_id(s: &str) -> Result<CredentialId, RepoError> {
    s.parse::<CredentialId>()
        .map_err(|e| RepoError::InvalidState(format!("bad credential_id `{s}`: {e}")))
}

#[async_trait::async_trait]
impl RefreshClaimRepo for PgRefreshClaimRepo {
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

        // Atomic CAS: INSERT or UPDATE only if the existing row is expired.
        // Returns the row we wrote (or overwrote) when we won; returns
        // nothing when we lost (the WHERE clause filtered the UPDATE).
        let row: Option<(Uuid, i64, DateTime<Utc>, DateTime<Utc>)> = sqlx::query_as(
            "INSERT INTO credential_refresh_claims \
             (credential_id, claim_id, generation, holder_replica_id, \
              acquired_at, expires_at, sentinel) \
             VALUES ($1, $2, 0, $3, $4, $5, 0) \
             ON CONFLICT (credential_id) DO UPDATE \
             SET claim_id = EXCLUDED.claim_id, \
                 generation = credential_refresh_claims.generation + 1, \
                 holder_replica_id = EXCLUDED.holder_replica_id, \
                 acquired_at = EXCLUDED.acquired_at, \
                 expires_at = EXCLUDED.expires_at, \
                 sentinel = 0 \
             WHERE credential_refresh_claims.expires_at < $4 \
             RETURNING claim_id, generation, acquired_at, expires_at",
        )
        .bind(&cid_str)
        .bind(new_claim_id)
        .bind(holder.as_str())
        .bind(now)
        .bind(new_expires)
        .fetch_optional(&self.pool)
        .await?;

        if let Some((claim_id, generation, acquired, expires)) = row {
            return Ok(ClaimAttempt::Acquired(RefreshClaim {
                credential_id: *credential_id,
                token: ClaimToken {
                    claim_id,
                    generation: generation as u64,
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
        let existing: Option<(DateTime<Utc>,)> = sqlx::query_as(
            "SELECT expires_at FROM credential_refresh_claims WHERE credential_id = $1",
        )
        .bind(&cid_str)
        .fetch_optional(&self.pool)
        .await?;

        match existing {
            Some((exp,)) => Ok(ClaimAttempt::Contended {
                existing_expires_at: exp,
            }),
            None => Ok(ClaimAttempt::Contended {
                existing_expires_at: now,
            }),
        }
    }

    async fn heartbeat(&self, token: &ClaimToken, ttl: Duration) -> Result<(), HeartbeatError> {
        let now = Utc::now();
        let extension = now
            + chrono::Duration::from_std(ttl).map_err(|e| {
                HeartbeatError::Repo(RepoError::InvalidState(format!("invalid ttl: {e}")))
            })?;

        let rows = sqlx::query(
            "UPDATE credential_refresh_claims \
             SET expires_at = $1 \
             WHERE claim_id = $2 AND generation = $3 AND expires_at > $4",
        )
        .bind(extension)
        .bind(token.claim_id)
        .bind(token.generation as i64)
        .bind(now)
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
        sqlx::query(
            "DELETE FROM credential_refresh_claims \
             WHERE claim_id = $1 AND generation = $2",
        )
        .bind(token.claim_id)
        .bind(token.generation as i64)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn mark_sentinel(&self, token: &ClaimToken) -> Result<(), RepoError> {
        sqlx::query(
            "UPDATE credential_refresh_claims \
             SET sentinel = 1 \
             WHERE claim_id = $1 AND generation = $2",
        )
        .bind(token.claim_id)
        .bind(token.generation as i64)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn reclaim_stuck(&self) -> Result<Vec<ReclaimedClaim>, RepoError> {
        let now = Utc::now();
        let rows: Vec<(String, String, i64, i16)> = sqlx::query_as(
            "DELETE FROM credential_refresh_claims \
             WHERE expires_at < $1 \
             RETURNING credential_id, holder_replica_id, generation, sentinel",
        )
        .bind(now)
        .fetch_all(&self.pool)
        .await?;

        let mut out = Vec::with_capacity(rows.len());
        for (cid, holder, generation, sentinel_raw) in rows {
            out.push(ReclaimedClaim {
                credential_id: parse_credential_id(&cid)?,
                previous_holder: ReplicaId::new(holder),
                previous_generation: generation as u64,
                sentinel: if sentinel_raw == 1 {
                    SentinelState::RefreshInFlight
                } else {
                    SentinelState::Normal
                },
            });
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
        sqlx::query(
            "INSERT INTO credential_sentinel_events \
             (credential_id, detected_at, crashed_holder, generation) \
             VALUES ($1, $2, $3, $4)",
        )
        .bind(&cid_str)
        .bind(Utc::now())
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
        let (count,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM credential_sentinel_events \
             WHERE credential_id = $1 AND detected_at > $2",
        )
        .bind(&cid_str)
        .bind(window_start)
        .fetch_one(&self.pool)
        .await?;
        Ok(u32::try_from(count).unwrap_or(u32::MAX))
    }
}
