//! SQLite-backed `RefreshClaimRepo` impl.
//!
//! Single-replica desktop mode + multi-process tests. CAS via
//! `INSERT ... ON CONFLICT DO UPDATE WHERE` to mirror Postgres
//! `INSERT ... ON CONFLICT ... WHERE` pattern.

use std::time::Duration;

use chrono::{DateTime, Utc};
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

fn parse_iso(s: &str) -> Result<DateTime<Utc>, RepoError> {
    s.parse::<DateTime<Utc>>()
        .map_err(|e| RepoError::InvalidState(format!("bad timestamp `{s}`: {e}")))
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
        let now_iso = now.to_rfc3339();
        let exp_iso = new_expires.to_rfc3339();
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
        let row: Option<(String, i64, String, String)> = sqlx::query_as(
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
        .bind(&now_iso)
        .bind(&exp_iso)
        .fetch_optional(&self.pool)
        .await?;

        if let Some((claim_id_str, generation, acquired_at_str, expires_at_str)) = row {
            let acquired_at = parse_iso(&acquired_at_str)?;
            let expires_at = parse_iso(&expires_at_str)?;
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
        let existing: Option<(String,)> = sqlx::query_as(
            "SELECT expires_at FROM credential_refresh_claims WHERE credential_id = ?1",
        )
        .bind(&cid_str)
        .fetch_optional(&self.pool)
        .await?;

        match existing {
            Some((exp_str,)) => Ok(ClaimAttempt::Contended {
                existing_expires_at: parse_iso(&exp_str)?,
            }),
            None => Err(RepoError::InvalidState(
                "CAS lost but no existing row visible".into(),
            )),
        }
    }

    async fn heartbeat(&self, token: &ClaimToken) -> Result<(), HeartbeatError> {
        let now = Utc::now();
        let now_iso = now.to_rfc3339();
        let extension = (now + chrono::Duration::seconds(30)).to_rfc3339();
        let claim_id_str = token.claim_id.to_string();

        let rows = sqlx::query(
            "UPDATE credential_refresh_claims \
             SET expires_at = ?1 \
             WHERE claim_id = ?2 AND generation = ?3 AND expires_at > ?4",
        )
        .bind(&extension)
        .bind(&claim_id_str)
        .bind(token.generation as i64)
        .bind(&now_iso)
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
        sqlx::query(
            "UPDATE credential_refresh_claims \
             SET sentinel = 1 \
             WHERE claim_id = ?1 AND generation = ?2",
        )
        .bind(&claim_id_str)
        .bind(token.generation as i64)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn reclaim_stuck(&self) -> Result<Vec<ReclaimedClaim>, RepoError> {
        let now = Utc::now();
        let now_iso = now.to_rfc3339();

        // Two-phase for SQLite (no RETURNING with DELETE everywhere):
        // 1. SELECT expired rows
        // 2. DELETE them
        let stuck: Vec<(String, String, i64, i64)> = sqlx::query_as(
            "SELECT credential_id, holder_replica_id, generation, sentinel \
             FROM credential_refresh_claims \
             WHERE expires_at < ?1",
        )
        .bind(&now_iso)
        .fetch_all(&self.pool)
        .await?;

        sqlx::query("DELETE FROM credential_refresh_claims WHERE expires_at < ?1")
            .bind(&now_iso)
            .execute(&self.pool)
            .await?;

        let mut out = Vec::with_capacity(stuck.len());
        for (cid, holder, generation, sentinel_raw) in stuck {
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
}
