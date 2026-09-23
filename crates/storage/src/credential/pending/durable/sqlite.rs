use super::*;
use sqlx::{Connection, Row, SqlitePool};

/// SQLite-backed encrypted pending state store.
pub struct SqlitePendingStateStore {
    pool: SqlitePool,
    cipher: PendingCipher,
}

impl SqlitePendingStateStore {
    pub(crate) fn new(
        pool: SqlitePool,
        key_provider: Arc<dyn KeyProvider>,
        legacy_keys: Vec<(String, Arc<EncryptionKey>)>,
    ) -> Self {
        Self {
            pool,
            cipher: PendingCipher::new(key_provider, legacy_keys),
        }
    }

    async fn now_ms(&self) -> Result<i64, PendingStoreError> {
        sqlx::query_scalar(
            "SELECT (CAST(strftime('%s', 'now') AS INTEGER) * 1000 + \
             CAST(substr(strftime('%f', 'now'), 4, 3) AS INTEGER))",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(|_| backend(DurablePendingError::Unavailable))
    }

    async fn load(
        transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        digest: &[u8; 32],
    ) -> Result<Option<PendingRow>, PendingStoreError> {
        let row = sqlx::query(
            "SELECT credential_kind, owner_id, session_id, state_encrypted, expires_at, \
             expires_at <= (CAST(strftime('%s', 'now') AS INTEGER) * 1000 + \
             CAST(substr(strftime('%f', 'now'), 4, 3) AS INTEGER)) AS expired \
             FROM credential_pending_states WHERE token_digest = ?1",
        )
        .bind(digest.as_slice())
        .fetch_optional(&mut **transaction)
        .await
        .map_err(|_| backend(DurablePendingError::Unavailable))?;
        row.map(|row| {
            Ok(PendingRow {
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
                    .try_get::<i64, _>(5)
                    .map_err(|_| backend(DurablePendingError::CorruptRecord))?
                    != 0,
            })
        })
        .transpose()
    }

    #[tracing::instrument(
        name = "credential.pending.sqlite.read",
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
        let mut connection = self
            .pool
            .acquire()
            .await
            .map_err(|_| backend(DurablePendingError::Unavailable))?;
        let mut transaction = connection
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(|_| backend(DurablePendingError::Unavailable))?;
        let row = Self::load(&mut transaction, &digest)
            .await?
            .ok_or(PendingStoreError::NotFound)?;
        if row.expired {
            sqlx::query("DELETE FROM credential_pending_states WHERE token_digest = ?1")
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
            let result =
                sqlx::query("DELETE FROM credential_pending_states WHERE token_digest = ?1")
                    .bind(digest.as_slice())
                    .execute(&mut *transaction)
                    .await
                    .map_err(|_| backend(DurablePendingError::Unavailable))?;
            if result.rows_affected() != 1 {
                return Err(PendingStoreError::NotFound);
            }
        }
        transaction
            .commit()
            .await
            .map_err(|_| backend(DurablePendingError::Unavailable))?;
        Ok(plaintext)
    }
}

impl std::fmt::Debug for SqlitePendingStateStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SqlitePendingStateStore")
    }
}

impl DynPendingStateStore for SqlitePendingStateStore {
    #[tracing::instrument(name = "credential.pending.sqlite.put", level = "debug", skip_all)]
    fn put_serialized<'a>(
        &'a self,
        kind: &'a str,
        owner: &'a str,
        session: &'a str,
        data: Zeroizing<Vec<u8>>,
        ttl: Duration,
    ) -> Pin<Box<dyn Future<Output = Result<PendingToken, PendingStoreError>> + Send + 'a>> {
        Box::pin(async move {
            let now = self.now_ms().await?;
            let expires = expiry_from(now, ttl)?;
            sqlx::query(
                "DELETE FROM credential_pending_states WHERE expires_at <= \
                 (CAST(strftime('%s', 'now') AS INTEGER) * 1000 + \
                 CAST(substr(strftime('%f', 'now'), 4, 3) AS INTEGER))",
            )
            .execute(&self.pool)
            .await
            .map_err(|_| backend(DurablePendingError::Unavailable))?;
            for _ in 0..INSERT_ATTEMPTS {
                let token = PendingToken::generate();
                let digest = token_digest(&token);
                let aad = pending_aad(&digest, kind, owner, session, expires).map_err(backend)?;
                let encrypted = self.cipher.encrypt(&data, &aad).map_err(backend)?;
                let result = sqlx::query("INSERT INTO credential_pending_states (token_digest, credential_kind, owner_id, session_id, state_encrypted, created_at, expires_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) ON CONFLICT(token_digest) DO NOTHING")
                    .bind(digest.as_slice()).bind(kind).bind(owner).bind(session).bind(encrypted).bind(now).bind(expires)
                    .execute(&self.pool).await.map_err(|_| backend(DurablePendingError::Unavailable))?;
                if result.rows_affected() == 1 {
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
    #[tracing::instrument(name = "credential.pending.sqlite.delete", level = "debug", skip_all)]
    fn delete<'a>(
        &'a self,
        token: &'a PendingToken,
    ) -> Pin<Box<dyn Future<Output = Result<(), PendingStoreError>> + Send + 'a>> {
        Box::pin(async move {
            let digest = token_digest(token);
            sqlx::query("DELETE FROM credential_pending_states WHERE token_digest = ?1")
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
    use crate::credential::{SqliteCredentialPersistence, key_provider::StaticKeyProvider};

    fn provider(byte: u8, version: &'static str) -> Arc<dyn KeyProvider> {
        Arc::new(StaticKeyProvider::with_version(
            Arc::new(EncryptionKey::from_bytes([byte; 32])),
            version,
        ))
    }

    async fn store() -> SqlitePendingStateStore {
        let persistence = SqliteCredentialPersistence::connect_memory()
            .await
            .expect("admit SQLite pending-state schema");
        persistence.pending_state_store(provider(7, "pending-test-v1"), Vec::new())
    }

    #[tokio::test]
    async fn round_trip_persists_only_digest_and_ciphertext() {
        const CANARY: &[u8] = b"pending-secret-canary";
        let store = store().await;
        let token = store
            .put_serialized(
                "oauth2",
                "owner-a",
                "session-a",
                Zeroizing::new(CANARY.to_vec()),
                Duration::from_mins(1),
            )
            .await
            .expect("put pending state");

        let (digest, encrypted): (Vec<u8>, Vec<u8>) =
            sqlx::query_as("SELECT token_digest, state_encrypted FROM credential_pending_states")
                .fetch_one(&store.pool)
                .await
                .expect("inspect durable representation");
        assert_eq!(digest.len(), 32);
        assert_ne!(digest, token.as_str().as_bytes());
        assert!(
            !encrypted
                .windows(CANARY.len())
                .any(|window| window == CANARY)
        );

        let restored = store
            .get_bound_serialized("oauth2", &token, "owner-a", "session-a")
            .await
            .expect("read pending state");
        assert_eq!(&*restored, CANARY);
    }

    #[tokio::test]
    async fn binding_mismatch_does_not_consume() {
        let store = store().await;
        let token = store
            .put_serialized(
                "oauth2",
                "owner-a",
                "session-a",
                Zeroizing::new(b"secret".to_vec()),
                Duration::from_mins(1),
            )
            .await
            .expect("put pending state");
        let rejected = store
            .consume_serialized("oauth2", &token, "owner-b", "session-a")
            .await;
        assert!(matches!(
            rejected,
            Err(PendingStoreError::ValidationFailed { .. })
        ));
        assert!(
            store
                .consume_serialized("oauth2", &token, "owner-a", "session-a")
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn concurrent_consume_has_exactly_one_winner() {
        let store = Arc::new(store().await);
        let token = store
            .put_serialized(
                "oauth2",
                "owner-a",
                "session-a",
                Zeroizing::new(b"secret".to_vec()),
                Duration::from_mins(1),
            )
            .await
            .expect("put pending state");
        let left_store = Arc::clone(&store);
        let left_token = token.clone();
        let left = tokio::spawn(async move {
            left_store
                .consume_serialized("oauth2", &left_token, "owner-a", "session-a")
                .await
        });
        let right_store = Arc::clone(&store);
        let right = tokio::spawn(async move {
            right_store
                .consume_serialized("oauth2", &token, "owner-a", "session-a")
                .await
        });
        let results = [
            left.await.expect("left task"),
            right.await.expect("right task"),
        ];
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|result| matches!(result, Err(PendingStoreError::NotFound)))
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn wrong_key_and_tampering_fail_without_consuming() {
        let good = store().await;
        let token = good
            .put_serialized(
                "oauth2",
                "owner-a",
                "session-a",
                Zeroizing::new(b"secret".to_vec()),
                Duration::from_mins(1),
            )
            .await
            .expect("put pending state");
        let wrong = SqlitePendingStateStore::new(
            good.pool.clone(),
            provider(9, "pending-test-v2"),
            Vec::new(),
        );
        assert!(matches!(
            wrong
                .consume_serialized("oauth2", &token, "owner-a", "session-a")
                .await,
            Err(PendingStoreError::Backend(_))
        ));
        assert!(
            good.get_bound_serialized("oauth2", &token, "owner-a", "session-a")
                .await
                .is_ok()
        );

        sqlx::query("UPDATE credential_pending_states SET session_id = 'tampered-session'")
            .execute(&good.pool)
            .await
            .expect("tamper row metadata");
        assert!(matches!(
            good.get_serialized(&token).await,
            Err(PendingStoreError::Backend(_))
        ));
    }

    #[tokio::test]
    async fn backend_clock_expires_and_evicts_state() {
        let store = store().await;
        let token = store
            .put_serialized(
                "oauth2",
                "owner-a",
                "session-a",
                Zeroizing::new(b"secret".to_vec()),
                Duration::from_millis(5),
            )
            .await
            .expect("put pending state");
        tokio::time::sleep(Duration::from_millis(25)).await;
        assert!(matches!(
            store.get_serialized(&token).await,
            Err(PendingStoreError::Expired)
        ));
        assert!(matches!(
            store.get_serialized(&token).await,
            Err(PendingStoreError::NotFound)
        ));
    }

    #[tokio::test]
    async fn explicit_legacy_key_reads_without_rewriting() {
        let persistence = SqliteCredentialPersistence::connect_memory()
            .await
            .expect("admit SQLite pending-state schema");
        let old_key = Arc::new(EncryptionKey::from_bytes([7; 32]));
        let old = persistence.pending_state_store(
            Arc::new(StaticKeyProvider::with_version(
                Arc::clone(&old_key),
                "pending-test-v1",
            )),
            Vec::new(),
        );
        let token = old
            .put_serialized(
                "oauth2",
                "owner-a",
                "session-a",
                Zeroizing::new(b"legacy-secret".to_vec()),
                Duration::from_mins(1),
            )
            .await
            .expect("put with old key");
        let before: Vec<u8> =
            sqlx::query_scalar("SELECT state_encrypted FROM credential_pending_states")
                .fetch_one(&old.pool)
                .await
                .expect("read old envelope");

        let current = SqlitePendingStateStore::new(
            old.pool.clone(),
            provider(9, "pending-test-v2"),
            vec![("pending-test-v1".to_owned(), old_key)],
        );
        let restored = current
            .get_bound_serialized("oauth2", &token, "owner-a", "session-a")
            .await
            .expect("read with explicit legacy key");
        assert_eq!(&*restored, b"legacy-secret");
        let after: Vec<u8> =
            sqlx::query_scalar("SELECT state_encrypted FROM credential_pending_states")
                .fetch_one(&current.pool)
                .await
                .expect("read unchanged envelope");
        assert_eq!(before, after);
    }

    #[tokio::test]
    async fn survives_sqlite_reopen() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("pending.db");
        let url = format!("sqlite://{}", path.display());
        let first = SqliteCredentialPersistence::connect(&url)
            .await
            .expect("open first persistence");
        let first_store = first.pending_state_store(provider(7, "pending-test-v1"), Vec::new());
        let token = first_store
            .put_serialized(
                "oauth2",
                "owner-a",
                "session-a",
                Zeroizing::new(b"restart-secret".to_vec()),
                Duration::from_mins(1),
            )
            .await
            .expect("put before restart");
        drop(first_store);
        drop(first);

        let second = SqliteCredentialPersistence::connect(&url)
            .await
            .expect("reopen persistence");
        let second_store = second.pending_state_store(provider(7, "pending-test-v1"), Vec::new());
        let restored = second_store
            .consume_serialized("oauth2", &token, "owner-a", "session-a")
            .await
            .expect("consume after restart");
        assert_eq!(&*restored, b"restart-secret");
    }
}
