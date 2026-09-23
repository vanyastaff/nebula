use super::*;
use chrono::{TimeZone, Utc};
use sqlx::{PgPool, Row};

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
        let row = sqlx::query("SELECT credential_kind, owner_id, session_id, state_encrypted, (EXTRACT(EPOCH FROM expires_at) * 1000)::BIGINT, expires_at <= clock_timestamp() FROM credential_pending_states WHERE token_digest = $1 FOR UPDATE")
            .bind(digest.as_slice()).fetch_optional(&mut *transaction).await.map_err(|_| backend(DurablePendingError::Unavailable))?
            .ok_or(PendingStoreError::NotFound)?;
        let row = PendingRow {
            credential_kind: row
                .try_get(0)
                .map_err(|_| backend(DurablePendingError::CorruptRecord))?,
            owner_id: row
                .try_get(1)
                .map_err(|_| backend(DurablePendingError::CorruptRecord))?,
            session_id: row
                .try_get(2)
                .map_err(|_| backend(DurablePendingError::CorruptRecord))?,
            state_encrypted: row
                .try_get(3)
                .map_err(|_| backend(DurablePendingError::CorruptRecord))?,
            expires_at_ms: row
                .try_get(4)
                .map_err(|_| backend(DurablePendingError::CorruptRecord))?,
            expired: row
                .try_get(5)
                .map_err(|_| backend(DurablePendingError::CorruptRecord))?,
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
            let now: chrono::DateTime<Utc> = sqlx::query_scalar("SELECT clock_timestamp()")
                .fetch_one(&self.pool)
                .await
                .map_err(|_| backend(DurablePendingError::Unavailable))?;
            let now_ms = now.timestamp_millis();
            let expires_ms = expiry_from(now_ms, ttl)?;
            let expires = Utc
                .timestamp_millis_opt(expires_ms)
                .single()
                .ok_or_else(|| backend(DurablePendingError::Unavailable))?;
            sqlx::query(
                "DELETE FROM credential_pending_states WHERE expires_at <= clock_timestamp()",
            )
            .execute(&self.pool)
            .await
            .map_err(|_| backend(DurablePendingError::Unavailable))?;
            for _ in 0..INSERT_ATTEMPTS {
                let token = PendingToken::generate();
                let digest = token_digest(&token);
                let aad =
                    pending_aad(&digest, kind, owner, session, expires_ms).map_err(backend)?;
                let encrypted = self.cipher.encrypt(&data, &aad).map_err(backend)?;
                let inserted = sqlx::query("INSERT INTO credential_pending_states (token_digest, credential_kind, owner_id, session_id, state_encrypted, created_at, expires_at) VALUES ($1,$2,$3,$4,$5,$6,$7) ON CONFLICT (token_digest) DO NOTHING").bind(digest.as_slice()).bind(kind).bind(owner).bind(session).bind(encrypted).bind(now).bind(expires).execute(&self.pool).await.map_err(|_| backend(DurablePendingError::Unavailable))?;
                if inserted.rows_affected() == 1 {
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
