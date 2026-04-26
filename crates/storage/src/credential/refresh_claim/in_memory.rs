//! In-memory `RefreshClaimRepo` impl for tests + desktop-mode fallback.
//!
//! Single-process scope — no cross-replica coordination. CAS uses a
//! `parking_lot::Mutex` over `HashMap<CredentialId, ClaimRow>`.

use std::{collections::HashMap, sync::Arc, time::Duration};

use chrono::{DateTime, Utc};
use nebula_core::CredentialId;
use parking_lot::Mutex;
use uuid::Uuid;

use super::{
    ClaimAttempt, ClaimToken, HeartbeatError, ReclaimedClaim, RefreshClaim, RefreshClaimRepo,
    ReplicaId, RepoError, SentinelState,
};

#[derive(Clone, Debug)]
struct ClaimRow {
    claim_id: Uuid,
    generation: u64,
    holder: ReplicaId,
    #[allow(dead_code, reason = "kept for future event/metric emission")]
    acquired_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    sentinel: SentinelState,
}

/// In-memory `RefreshClaimRepo`. Cheap to clone (Arc-backed inner).
#[derive(Clone, Default)]
pub struct InMemoryRefreshClaimRepo {
    inner: Arc<Mutex<HashMap<CredentialId, ClaimRow>>>,
}

impl InMemoryRefreshClaimRepo {
    /// Create a fresh, empty repo.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl std::fmt::Debug for InMemoryRefreshClaimRepo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InMemoryRefreshClaimRepo")
            .field("entries", &self.inner.lock().len())
            .finish()
    }
}

#[async_trait::async_trait]
impl RefreshClaimRepo for InMemoryRefreshClaimRepo {
    async fn try_claim(
        &self,
        credential_id: &CredentialId,
        holder: &ReplicaId,
        ttl: Duration,
    ) -> Result<ClaimAttempt, RepoError> {
        let now = Utc::now();
        let mut guard = self.inner.lock();

        if let Some(existing) = guard.get(credential_id)
            && existing.expires_at > now
        {
            return Ok(ClaimAttempt::Contended {
                existing_expires_at: existing.expires_at,
            });
        }
        // No row OR existing expired — claim wins. Generation bumps if
        // we're overwriting.

        let claim_id = Uuid::new_v4();
        let generation = guard.get(credential_id).map_or(0, |row| row.generation + 1);
        let acquired_at = now;
        let expires_at = now
            + chrono::Duration::from_std(ttl)
                .map_err(|e| RepoError::InvalidState(format!("invalid ttl: {e}")))?;

        let row = ClaimRow {
            claim_id,
            generation,
            holder: holder.clone(),
            acquired_at,
            expires_at,
            sentinel: SentinelState::Normal,
        };
        guard.insert(*credential_id, row);

        Ok(ClaimAttempt::Acquired(RefreshClaim {
            credential_id: *credential_id,
            token: ClaimToken {
                claim_id,
                generation,
            },
            acquired_at,
            expires_at,
        }))
    }

    async fn heartbeat(&self, token: &ClaimToken, ttl: Duration) -> Result<(), HeartbeatError> {
        let now = Utc::now();
        let extension = chrono::Duration::from_std(ttl).map_err(|e| {
            HeartbeatError::Repo(RepoError::InvalidState(format!("invalid ttl: {e}")))
        })?;
        let mut guard = self.inner.lock();

        let row = guard
            .values_mut()
            .find(|r| r.claim_id == token.claim_id && r.generation == token.generation);
        match row {
            Some(r) if r.expires_at > now => {
                // Extend by `ttl` past now. Caller (RefreshCoordinator) must
                // pass the configured `claim_ttl` so the §3.5 invariants hold.
                r.expires_at = now + extension;
                Ok(())
            },
            _ => Err(HeartbeatError::ClaimLost),
        }
    }

    async fn release(&self, token: ClaimToken) -> Result<(), RepoError> {
        let mut guard = self.inner.lock();
        guard.retain(|_, row| {
            !(row.claim_id == token.claim_id && row.generation == token.generation)
        });
        Ok(())
    }

    async fn mark_sentinel(&self, token: &ClaimToken) -> Result<(), RepoError> {
        let mut guard = self.inner.lock();
        let row = guard
            .values_mut()
            .find(|r| r.claim_id == token.claim_id && r.generation == token.generation);
        if let Some(r) = row {
            r.sentinel = SentinelState::RefreshInFlight;
        }
        Ok(())
    }

    async fn reclaim_stuck(&self) -> Result<Vec<ReclaimedClaim>, RepoError> {
        let now = Utc::now();
        let mut guard = self.inner.lock();
        let mut out = Vec::new();

        let stuck: Vec<CredentialId> = guard
            .iter()
            .filter(|(_, r)| r.expires_at < now)
            .map(|(k, _)| *k)
            .collect();

        for cid in stuck {
            if let Some(row) = guard.remove(&cid) {
                out.push(ReclaimedClaim {
                    credential_id: cid,
                    previous_holder: row.holder.clone(),
                    previous_generation: row.generation,
                    sentinel: row.sentinel,
                });
            }
        }

        Ok(out)
    }
}
