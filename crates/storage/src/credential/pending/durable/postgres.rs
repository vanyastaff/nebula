use super::*;
use chrono::{TimeZone, Utc};
use sqlx::{PgPool, Row};

fn pending_timestamps(
    now: chrono::DateTime<Utc>,
    ttl: Duration,
) -> Result<(chrono::DateTime<Utc>, chrono::DateTime<Utc>), PendingStoreError> {
    let now_ms = now.timestamp_millis();
    let created_at = Utc
        .timestamp_millis_opt(now_ms)
        .single()
        .ok_or_else(|| backend(DurablePendingError::Unavailable))?;
    let expires_ms = expiry_from(now_ms, ttl)?;
    let expires_at = Utc
        .timestamp_millis_opt(expires_ms)
        .single()
        .ok_or_else(|| backend(DurablePendingError::Unavailable))?;
    Ok((created_at, expires_at))
}

/// PostgreSQL-backed encrypted pending state store.
pub struct PgPendingStateStore {
    pool: PgPool,
    cipher: PendingCipher,
}

impl PgPendingStateStore {
    pub(crate) fn new(
        pool: PgPool,
        key_provider: Arc<dyn KeyProvider>,
        legacy_keys: Vec<(String, Arc<EncryptionKey>)>,
    ) -> Self {
        Self {
            pool,
            cipher: PendingCipher::new(key_provider, legacy_keys),
        }
    }

    #[tracing::instrument(
        name = "credential.pending.postgres.read",
        level = "debug",
        skip_all,
        fields(consume)
    )]
    async fn read(
        &self,
        token: &PendingToken,
        binding: Option<(&str, &str, &str)>,
        consume: bool,
    ) -> Result<Zeroizing<Vec<u8>>, PendingStoreError> {
        let digest = token_digest(token);
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(|_| backend(DurablePendingError::Unavailable))?;
        let row = sqlx::query(
            "SELECT credential_kind, org_id, workspace_id, session_id, state_encrypted, \
             (EXTRACT(EPOCH FROM expires_at) * 1000)::BIGINT, expires_at <= clock_timestamp() \
             FROM credential_pending_states WHERE token_digest = $1 FOR UPDATE",
        )
        .bind(digest.as_slice())
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|_| backend(DurablePendingError::Unavailable))?
        .ok_or(PendingStoreError::NotFound)?;
        let corrupt = |_| backend(DurablePendingError::CorruptRecord);
        let row = PendingRow {
            credential_kind: row.try_get(0).map_err(corrupt)?,
            owner_id: row_owner_id(
                row.try_get(1).map_err(corrupt)?,
                row.try_get(2).map_err(corrupt)?,
            ),
            session_id: row.try_get(3).map_err(corrupt)?,
            state_encrypted: row.try_get(4).map_err(corrupt)?,
            expires_at_ms: row.try_get(5).map_err(corrupt)?,
            expired: row.try_get(6).map_err(corrupt)?,
        };
        if row.expired {
            sqlx::query("DELETE FROM credential_pending_states WHERE token_digest = $1")
                .bind(digest.as_slice())
                .execute(&mut *transaction)
                .await
                .map_err(|_| backend(DurablePendingError::Unavailable))?;
            transaction
                .commit()
                .await
                .map_err(|_| backend(DurablePendingError::Unavailable))?;
            return Err(PendingStoreError::Expired);
        }
        if let Some((kind, owner, session)) = binding
            && !binding_matches(&row, kind, owner, session)
        {
            return Err(PendingStoreError::ValidationFailed {
                reason: "token bindings do not match".to_owned(),
            });
        }
        let plaintext = row.decrypt(&self.cipher, &digest)?;
        if consume {
            sqlx::query("DELETE FROM credential_pending_states WHERE token_digest = $1")
                .bind(digest.as_slice())
                .execute(&mut *transaction)
                .await
                .map_err(|_| backend(DurablePendingError::Unavailable))?;
        }
        transaction
            .commit()
            .await
            .map_err(|_| backend(DurablePendingError::Unavailable))?;
        Ok(plaintext)
    }
}

impl std::fmt::Debug for PgPendingStateStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PgPendingStateStore")
    }
}

impl DynPendingStateStore for PgPendingStateStore {
    #[tracing::instrument(name = "credential.pending.postgres.put", level = "debug", skip_all)]
    fn put_serialized<'a>(
        &'a self,
        kind: &'a str,
        owner: &'a str,
        session: &'a str,
        data: Zeroizing<Vec<u8>>,
        ttl: Duration,
    ) -> Pin<Box<dyn Future<Output = Result<PendingToken, PendingStoreError>> + Send + 'a>> {
        Box::pin(async move {
            // Pending state belongs to the workspace its credential will.
            let scope = owner_scope(owner)?;
            let now: chrono::DateTime<Utc> = sqlx::query_scalar("SELECT clock_timestamp()")
                .fetch_one(&self.pool)
                .await
                .map_err(|_| backend(DurablePendingError::Unavailable))?;
            let (created_at, expires) = pending_timestamps(now, ttl)?;
            let expires_ms = expires.timestamp_millis();
            sqlx::query(
                "DELETE FROM credential_pending_states WHERE expires_at <= clock_timestamp()",
            )
            .execute(&self.pool)
            .await
            .map_err(|_| backend(DurablePendingError::Unavailable))?;
            let unavailable = |_| backend(DurablePendingError::Unavailable);
            let mut transaction = self.pool.begin().await.map_err(unavailable)?;
            // The foreign key proves the workspace exists; the share lock
            // proves it is live and serializes with a concurrent archive.
            let workspace: Option<(String,)> = sqlx::query_as(
                "SELECT w.id FROM workspaces w JOIN orgs o ON o.id = w.org_id \
                 WHERE w.org_id = $1 AND w.id = $2 \
                   AND w.deleted_at IS NULL AND o.deleted_at IS NULL \
                 FOR SHARE OF w",
            )
            .bind(&scope.org_id)
            .bind(&scope.workspace_id)
            .fetch_optional(&mut *transaction)
            .await
            .map_err(unavailable)?;
            if workspace.is_none() {
                return Err(PendingStoreError::NotFound);
            }
            for _ in 0..INSERT_ATTEMPTS {
                let token = PendingToken::generate();
                let digest = token_digest(&token);
                let aad =
                    pending_aad(&digest, kind, owner, session, expires_ms).map_err(backend)?;
                let encrypted = self.cipher.encrypt(&data, &aad).map_err(backend)?;
                let inserted = sqlx::query(
                    "INSERT INTO credential_pending_states \
                     (org_id, workspace_id, token_digest, credential_kind, session_id, \
                      state_encrypted, created_at, expires_at) \
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
                     ON CONFLICT (token_digest) DO NOTHING",
                )
                .bind(&scope.org_id)
                .bind(&scope.workspace_id)
                .bind(digest.as_slice())
                .bind(kind)
                .bind(session)
                .bind(encrypted)
                .bind(created_at)
                .bind(expires)
                .execute(&mut *transaction)
                .await
                .map_err(|error| {
                    if crate::sql_error::is_foreign_key_violation(&error) {
                        PendingStoreError::NotFound
                    } else {
                        backend(DurablePendingError::Unavailable)
                    }
                })?;
                if inserted.rows_affected() == 1 {
                    transaction.commit().await.map_err(unavailable)?;
                    return Ok(token);
                }
            }
            Err(backend(DurablePendingError::Unavailable))
        })
    }
    fn get_serialized<'a>(
        &'a self,
        token: &'a PendingToken,
    ) -> Pin<Box<dyn Future<Output = Result<Zeroizing<Vec<u8>>, PendingStoreError>> + Send + 'a>>
    {
        Box::pin(self.read(token, None, false))
    }
    fn get_bound_serialized<'a>(
        &'a self,
        kind: &'a str,
        token: &'a PendingToken,
        owner: &'a str,
        session: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Zeroizing<Vec<u8>>, PendingStoreError>> + Send + 'a>>
    {
        Box::pin(self.read(token, Some((kind, owner, session)), false))
    }
    fn consume_serialized<'a>(
        &'a self,
        kind: &'a str,
        token: &'a PendingToken,
        owner: &'a str,
        session: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Zeroizing<Vec<u8>>, PendingStoreError>> + Send + 'a>>
    {
        Box::pin(self.read(token, Some((kind, owner, session)), true))
    }
    #[tracing::instrument(name = "credential.pending.postgres.delete", level = "debug", skip_all)]
    fn delete<'a>(
        &'a self,
        token: &'a PendingToken,
    ) -> Pin<Box<dyn Future<Output = Result<(), PendingStoreError>> + Send + 'a>> {
        Box::pin(async move {
            let digest = token_digest(token);
            sqlx::query("DELETE FROM credential_pending_states WHERE token_digest=$1")
                .bind(digest.as_slice())
                .execute(&self.pool)
                .await
                .map_err(|_| backend(DurablePendingError::Unavailable))?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_and_sub_millisecond_ttl_share_created_at_precision() {
        let now = Utc
            .timestamp_micros(1_700_000_000_123_456)
            .single()
            .expect("test timestamp must be valid");

        for ttl in [Duration::ZERO, Duration::from_nanos(999_999)] {
            let (created_at, expires_at) =
                pending_timestamps(now, ttl).expect("bounded timestamps must compute");
            assert_eq!(created_at, expires_at);
            assert_eq!(created_at.timestamp_subsec_nanos() % 1_000_000, 0);
        }
    }
}
