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

    /// SQLite's clock in milliseconds: the precision pending expiry is sealed
    /// with. Stored instants are microseconds, always whole milliseconds here.
    async fn now_ms(&self) -> Result<i64, PendingStoreError> {
        sqlx::query_scalar(concat!("SELECT ", sqlite_now_us!(), " / 1000"))
            .fetch_one(&self.pool)
            .await
            .map_err(|_| backend(DurablePendingError::Unavailable))
    }

    async fn load(
        transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        digest: &[u8; 32],
    ) -> Result<Option<PendingRow>, PendingStoreError> {
        let row = sqlx::query(concat!(
            "SELECT credential_kind, org_id, workspace_id, session_id, state_encrypted, \
             expires_at, expires_at <= ",
            sqlite_now_us!(),
            " AS expired FROM credential_pending_states WHERE token_digest = ?1"
        ))
        .bind(digest.as_slice())
        .fetch_optional(&mut **transaction)
        .await
        .map_err(|_| backend(DurablePendingError::Unavailable))?;
        let corrupt = |_| backend(DurablePendingError::CorruptRecord);
        row.map(|row| {
            let expires_at_us: i64 = row.try_get(5).map_err(corrupt)?;
            if expires_at_us % 1000 != 0 {
                return Err(backend(DurablePendingError::CorruptRecord));
            }
            Ok(PendingRow {
                credential_kind: row.try_get(0).map_err(corrupt)?,
                owner_id: row_owner_id(
                    row.try_get(1).map_err(corrupt)?,
                    row.try_get(2).map_err(corrupt)?,
                ),
                session_id: row.try_get(3).map_err(corrupt)?,
                state_encrypted: row.try_get(4).map_err(corrupt)?,
                expires_at_ms: expires_at_us / 1000,
                expired: row.try_get::<i64, _>(6).map_err(corrupt)? != 0,
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
            // Pending state belongs to the workspace its credential will.
            let scope = owner_scope(owner)?;
            let now = self.now_ms().await?;
            let expires = expiry_from(now, ttl)?;
            let unavailable = |_| backend(DurablePendingError::Unavailable);
            let (created_at_us, expires_at_us) = now
                .checked_mul(1000)
                .zip(expires.checked_mul(1000))
                .ok_or_else(|| backend(DurablePendingError::Unavailable))?;
            sqlx::query(concat!(
                "DELETE FROM credential_pending_states WHERE expires_at <= ",
                sqlite_now_us!()
            ))
            .execute(&self.pool)
            .await
            .map_err(unavailable)?;
            let mut connection = self.pool.acquire().await.map_err(unavailable)?;
            // `BEGIN IMMEDIATE` serializes the live-workspace check with a
            // concurrent archive; the foreign key proves existence only.
            let mut transaction = connection
                .begin_with("BEGIN IMMEDIATE")
                .await
                .map_err(unavailable)?;
            let workspace: Option<(i64,)> = sqlx::query_as(
                "SELECT 1 FROM workspaces w JOIN orgs o ON o.id = w.org_id \
                 WHERE w.org_id = ?1 AND w.id = ?2 \
                   AND w.deleted_at IS NULL AND o.deleted_at IS NULL",
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
                let aad = pending_aad(&digest, kind, owner, session, expires).map_err(backend)?;
                let encrypted = self.cipher.encrypt(&data, &aad).map_err(backend)?;
                let result = sqlx::query(
                    "INSERT INTO credential_pending_states \
                     (org_id, workspace_id, token_digest, credential_kind, session_id, \
                      state_encrypted, created_at, expires_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) \
                     ON CONFLICT(token_digest) DO NOTHING",
                )
                .bind(&scope.org_id)
                .bind(&scope.workspace_id)
                .bind(digest.as_slice())
                .bind(kind)
                .bind(session)
                .bind(encrypted)
                .bind(created_at_us)
                .bind(expires_at_us)
                .execute(&mut *transaction)
                .await
                .map_err(|error| {
                    if crate::sql_error::is_foreign_key_violation(&error) {
                        PendingStoreError::NotFound
                    } else {
                        backend(DurablePendingError::Unavailable)
                    }
                })?;
                if result.rows_affected() == 1 {
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
    use nebula_storage_port::Scope;

    use crate::credential::{SqliteCredentialPersistence, key_provider::StaticKeyProvider};

    /// The canonical owner key of workspace `ws-a` in `org-a`.
    const OWNER_A: &str = "5\u{1e}org-a\u{1e}ws-a";

    fn provider(byte: u8, version: &'static str) -> Arc<dyn KeyProvider> {
        Arc::new(StaticKeyProvider::with_version(
            Arc::new(EncryptionKey::from_bytes([byte; 32])),
            version,
        ))
    }

    /// Provision the workspace [`OWNER_A`] names: pending state belongs to it.
    async fn provision_owner_a(persistence: &SqliteCredentialPersistence) {
        crate::credential::test_owner::provision(&persistence.tenant_provisioning_store(), &["a"])
            .await;
    }

    async fn store() -> SqlitePendingStateStore {
        let persistence = SqliteCredentialPersistence::connect_memory()
            .await
            .expect("admit SQLite pending-state schema");
        provision_owner_a(&persistence).await;
        persistence.pending_state_store(provider(7, "pending-test-v1"), Vec::new())
    }

    #[tokio::test]
    async fn pending_state_is_filed_under_its_owners_workspace() {
        assert_eq!(
            Scope::new("ws-a", "org-a").credential_owner_id(),
            OWNER_A,
            "the fixture owner is the canonical key of its workspace"
        );
        let store = store().await;
        assert!(matches!(
            store
                .put_serialized(
                    "oauth2",
                    "owner-without-workspace",
                    "session-a",
                    Zeroizing::new(b"secret".to_vec()),
                    Duration::from_mins(1),
                )
                .await,
            Err(PendingStoreError::NotFound)
        ));
        let unprovisioned = Scope::new("ws-missing", "org-missing").credential_owner_id();
        assert!(matches!(
            store
                .put_serialized(
                    "oauth2",
                    &unprovisioned,
                    "session-a",
                    Zeroizing::new(b"secret".to_vec()),
                    Duration::from_mins(1),
                )
                .await,
            Err(PendingStoreError::NotFound)
        ));
        let token = store
            .put_serialized(
                "oauth2",
                OWNER_A,
                "session-a",
                Zeroizing::new(b"secret".to_vec()),
                Duration::from_mins(1),
            )
            .await
            .expect("put pending state");
        let tenant: (String, String) =
            sqlx::query_as("SELECT org_id, workspace_id FROM credential_pending_states")
                .fetch_one(&store.pool)
                .await
                .expect("inspect the owning workspace");
        assert_eq!(tenant, ("org-a".to_owned(), "ws-a".to_owned()));

        sqlx::query("UPDATE workspaces SET deleted_at = 1 WHERE id = 'ws-a'")
            .execute(&store.pool)
            .await
            .expect("archive the workspace");
        assert!(matches!(
            store
                .put_serialized(
                    "oauth2",
                    OWNER_A,
                    "session-a",
                    Zeroizing::new(b"secret".to_vec()),
                    Duration::from_mins(1),
                )
                .await,
            Err(PendingStoreError::NotFound)
        ));

        sqlx::query("DELETE FROM workspaces WHERE id = 'ws-a'")
            .execute(&store.pool)
            .await
            .expect("purge the workspace");
        assert!(matches!(
            store.get_serialized(&token).await,
            Err(PendingStoreError::NotFound)
        ));
    }

    #[tokio::test]
    async fn round_trip_persists_only_digest_and_ciphertext() {
        const CANARY: &[u8] = b"pending-secret-canary";
        let store = store().await;
        let token = store
            .put_serialized(
                "oauth2",
                OWNER_A,
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
            .get_bound_serialized("oauth2", &token, OWNER_A, "session-a")
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
                OWNER_A,
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
                .consume_serialized("oauth2", &token, OWNER_A, "session-a")
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
                OWNER_A,
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
                .consume_serialized("oauth2", &left_token, OWNER_A, "session-a")
                .await
        });
        let right_store = Arc::clone(&store);
        let right = tokio::spawn(async move {
            right_store
                .consume_serialized("oauth2", &token, OWNER_A, "session-a")
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
                OWNER_A,
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
                .consume_serialized("oauth2", &token, OWNER_A, "session-a")
                .await,
            Err(PendingStoreError::Backend(_))
        ));
        assert!(
            good.get_bound_serialized("oauth2", &token, OWNER_A, "session-a")
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
                OWNER_A,
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
    async fn ttl_boundary_matches_the_reference_adapter() {
        let store = store().await;
        let zero = store
            .put_serialized(
                "oauth2",
                OWNER_A,
                "session-a",
                Zeroizing::new(b"secret".to_vec()),
                Duration::ZERO,
            )
            .await
            .expect("zero TTL is admitted as immediately expired state");
        assert!(matches!(
            store.get_serialized(&zero).await,
            Err(PendingStoreError::Expired)
        ));

        let oversized = store
            .put_serialized(
                "oauth2",
                OWNER_A,
                "session-a",
                Zeroizing::new(b"secret".to_vec()),
                Duration::from_mins(10) + Duration::from_nanos(1),
            )
            .await;
        assert!(matches!(
            oversized,
            Err(PendingStoreError::ValidationFailed { .. })
        ));
    }

    #[tokio::test]
    async fn explicit_legacy_key_reads_without_rewriting() {
        let persistence = SqliteCredentialPersistence::connect_memory()
            .await
            .expect("admit SQLite pending-state schema");
        provision_owner_a(&persistence).await;
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
                OWNER_A,
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
            .get_bound_serialized("oauth2", &token, OWNER_A, "session-a")
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
        provision_owner_a(&first).await;
        let first_store = first.pending_state_store(provider(7, "pending-test-v1"), Vec::new());
        let token = first_store
            .put_serialized(
                "oauth2",
                OWNER_A,
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
            .consume_serialized("oauth2", &token, OWNER_A, "session-a")
            .await
            .expect("consume after restart");
        assert_eq!(&*restored, b"restart-secret");
    }
}
