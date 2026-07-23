//! Encryption layer -- encrypts data before storage, decrypts after retrieval.
//!
//! Wraps any [`CredentialPersistence`] implementation and applies AES-256-GCM
//! encryption to the live [`StoredCredential`] payload. The credential ID is
//! bound as Additional Authenticated Data (AAD), preventing record-swapping
//! attacks where encrypted data from one credential is copied to another.
//! Structural tombstones carry no data and bypass cryptography entirely.
//!
//! AAD validation is mandatory — data encrypted without AAD (or with a
//! mismatched credential ID) is rejected with a hard error. There is no
//! legacy fallback path.
//!
//! # Key source: [`KeyProvider`]
//!
//! The current encryption key is supplied by an
//! [`Arc<dyn KeyProvider>`](super::super::KeyProvider), **not** an `Arc<EncryptionKey>`
//! directly. Composition roots choose the provider (env var, file, KMS, …)
//! at wiring time — see `crates/storage/README.md` and
//! `docs/INTEGRATION_MODEL.md` for the key-provider seam.
//!
//! # Key rotation
//!
//! On every read the layer inspects `EncryptedData::key_id`:
//!
//! - If `key_id` matches the provider's atomic current snapshot, decrypt with that key.
//! - If `key_id` differs, look it up in the optional `legacy_keys` map and decrypt with that key.
//!   Reads never rewrite durable state. The next real mutation encrypts with the current key and
//!   advances the record version exactly once. `legacy_keys` is populated via
//!   [`EncryptionLayer::with_legacy_keys`] while an operator is migrating off an older key.

use std::{collections::HashMap, fmt, sync::Arc};

use async_trait::async_trait;
use nebula_crypto::{EncryptedData, EncryptionKey, decrypt_with_aad, encrypt_with_key_id};
use nebula_storage_port::{
    CredentialCommit, CredentialCreate, CredentialOwner, CredentialPersistence,
    CredentialPersistenceError, CredentialReplacement, CredentialSelector, CredentialTombstone,
    SecretBytes, StoredCredential, StoredCredentialHead, StoredLiveCredential,
};

use super::super::key_provider::{KeyProvider, KeySnapshot};

/// Wraps a store with AES-256-GCM encryption on the `data` field.
///
/// The current key is supplied by the configured [`KeyProvider`]. Records
/// encrypted with an older key may optionally be decrypted via `legacy_keys`
/// (populated by [`Self::with_legacy_keys`]). Reads are side-effect free;
/// subsequent writes always use the current key.
///
/// # Examples
///
/// Requires the `sqlite` feature; the async `connect` and the env-var key
/// read make this `no_run` (it still type-checks the real API):
///
/// ```rust,no_run
/// # #[cfg(feature = "sqlite")]
/// # async fn doc() -> Result<(), Box<dyn std::error::Error>> {
/// use std::sync::Arc;
///
/// use nebula_storage::credential::{EncryptionLayer, EnvKeyProvider, SqliteCredentialPersistence};
///
/// // Production: read the key from NEBULA_CRED_MASTER_KEY.
/// let provider = Arc::new(EnvKeyProvider::from_env()?);
/// let backend = SqliteCredentialPersistence::connect("sqlite://creds.db").await?;
/// let store = EncryptionLayer::new(backend, provider);
/// # let _ = store;
/// # Ok(())
/// # }
/// ```
pub struct EncryptionLayer<S> {
    inner: S,
    key_provider: Arc<dyn KeyProvider>,
    legacy_keys: HashMap<String, Arc<EncryptionKey>>,
}

impl<S> EncryptionLayer<S> {
    /// Create an encryption layer whose current key is sourced from
    /// `key_provider`.
    ///
    /// # Legacy `""` records
    ///
    /// Earlier builds of this layer aliased the key under the empty string
    /// `""` so that legacy envelopes with `key_id: ""` would silently decrypt
    /// with the current key. That alias was removed (GitHub issue #281):
    /// silent cross-identity decryption breaks the key-rotation invariant
    /// (PRODUCT_CANON §4.2 / §12.5) and makes `key_id`-based audit provenance
    /// unreliable. Deployments that still hold `""` envelopes must register
    /// the empty alias explicitly via [`with_legacy_keys`](Self::with_legacy_keys) —
    /// e.g.
    ///
    /// ```rust,no_run
    /// # #[cfg(feature = "sqlite")]
    /// # async fn doc() -> Result<(), Box<dyn std::error::Error>> {
    /// use std::sync::Arc;
    ///
    /// use nebula_crypto::EncryptionKey;
    /// use nebula_storage::credential::{EncryptionLayer, EnvKeyProvider, SqliteCredentialPersistence};
    ///
    /// let inner = SqliteCredentialPersistence::connect("sqlite://creds.db").await?;
    /// let legacy_key = Arc::new(EncryptionKey::from_bytes([0x42; 32]));
    /// EncryptionLayer::with_legacy_keys(
    ///     inner,
    ///     Arc::new(EnvKeyProvider::from_env()?), // provider drives the current key
    ///     vec![(String::new(), legacy_key)],     // explicit, audit-visible
    /// );
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// A subsequent real mutation will re-encrypt any successfully read
    /// `""` record with the provider's current version.
    pub fn new(inner: S, key_provider: Arc<dyn KeyProvider>) -> Self {
        Self {
            inner,
            key_provider,
            legacy_keys: HashMap::new(),
        }
    }

    /// Create an encryption layer with additional decrypt-only keys for
    /// rotation support.
    ///
    /// `key_provider` supplies the current encrypt/decrypt key; `legacy_keys`
    /// contains historical keys that remain valid for reads. On read of a
    /// record whose envelope `key_id` matches a legacy entry, the layer
    /// decrypts with the legacy key without rewriting it. The next real
    /// mutation encrypts with the current key.
    ///
    /// Legacy entries do not include the current key — that is always the
    /// provider's concern.
    pub fn with_legacy_keys(
        inner: S,
        key_provider: Arc<dyn KeyProvider>,
        legacy_keys: Vec<(String, Arc<EncryptionKey>)>,
    ) -> Self {
        Self {
            inner,
            key_provider,
            legacy_keys: legacy_keys.into_iter().collect(),
        }
    }

    fn current_snapshot(&self) -> Result<KeySnapshot, CredentialPersistenceError> {
        self.key_provider
            .current()
            .map_err(|_| CredentialPersistenceError::Unavailable)
    }

    fn legacy_key(&self, key_id: &str) -> Result<Arc<EncryptionKey>, CredentialPersistenceError> {
        self.legacy_keys
            .get(key_id)
            .cloned()
            .ok_or(CredentialPersistenceError::CorruptRecord)
    }
}

impl<S> fmt::Debug for EncryptionLayer<S> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EncryptionLayer")
            .field("legacy_key_count", &self.legacy_keys.len())
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl<S: CredentialPersistence> CredentialPersistence for EncryptionLayer<S> {
    async fn get(
        &self,
        selector: &CredentialSelector,
    ) -> Result<StoredCredential, CredentialPersistenceError> {
        match self.inner.get(selector).await? {
            StoredCredential::Live(record) => {
                let credential_id = record.credential_id();
                let plaintext = self.decrypt_data(record.data(), credential_id)?;
                StoredLiveCredential::new(
                    credential_id,
                    record.name().map(str::to_owned),
                    record.credential_key().to_owned(),
                    SecretBytes::from(plaintext),
                    record.state_kind().to_owned(),
                    record.state_version(),
                    record.version(),
                    record.created_at(),
                    record.updated_at(),
                    record.expires_at(),
                    record.reauth_required(),
                    record.metadata().clone(),
                )
                .map(StoredCredential::Live)
            },
            tombstone @ StoredCredential::Tombstoned(_) => Ok(tombstone),
        }
    }

    async fn get_head(
        &self,
        selector: &CredentialSelector,
    ) -> Result<StoredCredentialHead, CredentialPersistenceError> {
        // The projection has no data field, so this path neither selects nor
        // decrypts credential material.
        self.inner.get_head(selector).await
    }

    async fn create(
        &self,
        selector: &CredentialSelector,
        create: CredentialCreate,
    ) -> Result<CredentialCommit, CredentialPersistenceError> {
        let encrypted = self.encrypt_data(create.data(), selector.credential_id())?;
        self.inner
            .create(
                selector,
                CredentialCreate::new(
                    create.credential_key().to_owned(),
                    SecretBytes::new(encrypted),
                    create.state_kind().to_owned(),
                    create.state_version(),
                    create.name().map(str::to_owned),
                    create.expires_at(),
                    create.reauth_required(),
                    create.metadata().clone(),
                ),
            )
            .await
    }

    async fn replace(
        &self,
        selector: &CredentialSelector,
        replacement: CredentialReplacement,
    ) -> Result<CredentialCommit, CredentialPersistenceError> {
        let encrypted = self.encrypt_data(replacement.data(), selector.credential_id())?;
        self.inner
            .replace(
                selector,
                CredentialReplacement::new(
                    replacement.expected_version(),
                    SecretBytes::new(encrypted),
                    replacement.state_kind().to_owned(),
                    replacement.state_version(),
                    replacement.name().map(str::to_owned),
                    replacement.expires_at(),
                    replacement.reauth_required(),
                    replacement.metadata().clone(),
                ),
            )
            .await
    }

    async fn tombstone(
        &self,
        selector: &CredentialSelector,
        tombstone: CredentialTombstone,
    ) -> Result<CredentialCommit, CredentialPersistenceError> {
        self.inner.tombstone(selector, tombstone).await
    }

    async fn list(
        &self,
        owner: &CredentialOwner,
        state_kind: Option<&str>,
    ) -> Result<Vec<nebula_core::CredentialId>, CredentialPersistenceError> {
        self.inner.list(owner, state_kind).await
    }

    async fn list_heads(
        &self,
        owner: &CredentialOwner,
        state_kind: Option<&str>,
    ) -> Result<Vec<StoredCredentialHead>, CredentialPersistenceError> {
        self.inner.list_heads(owner, state_kind).await
    }

    async fn exists(
        &self,
        selector: &CredentialSelector,
    ) -> Result<bool, CredentialPersistenceError> {
        self.inner.exists(selector).await
    }
}

impl<S> EncryptionLayer<S> {
    /// Encrypt `plaintext` with the current key, serializing the envelope to bytes.
    fn encrypt_data(
        &self,
        plaintext: &[u8],
        credential_id: nebula_core::CredentialId,
    ) -> Result<Vec<u8>, CredentialPersistenceError> {
        let current = self.current_snapshot()?;
        let aad = credential_id.to_string();
        let encrypted =
            encrypt_with_key_id(current.key(), current.key_id(), plaintext, aad.as_bytes())
                .map_err(|_| CredentialPersistenceError::Unavailable)?;
        serde_json::to_vec(&encrypted).map_err(|_| CredentialPersistenceError::Unavailable)
    }

    /// Decrypt `ciphertext` with the current or explicitly configured legacy key.
    ///
    /// AAD (credential ID) is always enforced — data without AAD or with a
    /// mismatched ID is rejected.
    fn decrypt_data(
        &self,
        ciphertext: &[u8],
        credential_id: nebula_core::CredentialId,
    ) -> Result<zeroize::Zeroizing<Vec<u8>>, CredentialPersistenceError> {
        let encrypted: EncryptedData = serde_json::from_slice(ciphertext)
            .map_err(|_| CredentialPersistenceError::CorruptRecord)?;

        let current = self.current_snapshot()?;
        let aad = credential_id.to_string();

        // Data encrypted with the current key — normal path.
        if encrypted.key_id == current.key_id() {
            let plaintext = decrypt_with_aad(current.key(), &encrypted, aad.as_bytes())
                .map_err(|_| CredentialPersistenceError::CorruptRecord)?;
            return Ok(plaintext);
        }

        // Data encrypted with an older key is readable during the migration
        // window, but a read must never become a hidden durable write.
        let old_key = self.legacy_key(&encrypted.key_id)?;
        decrypt_with_aad(&old_key, &encrypted, aad.as_bytes())
            .map_err(|_| CredentialPersistenceError::CorruptRecord)
    }
}

#[cfg(all(test, feature = "sqlite"))]
mod tests {
    use nebula_core::CredentialId;
    use nebula_credential::{AuthStyle, SecretString, credentials::oauth2::OAuth2State};
    use nebula_storage_port::{
        CredentialOwner, CredentialSelector, CredentialTombstone, CredentialVersion,
        StoredCredential, StoredLiveCredential,
    };

    use crate::credential::test_support::{make_credential, make_replacement};
    use nebula_crypto::encrypt_with_key_id;

    use super::{
        super::super::{key_provider::StaticKeyProvider, sqlite::SqliteCredentialPersistence},
        *,
    };

    fn owner() -> CredentialOwner {
        CredentialOwner::from_canonical("test-owner")
    }

    fn selector(id: CredentialId) -> CredentialSelector {
        CredentialSelector::new(owner(), id)
    }

    fn version(value: i64) -> CredentialVersion {
        CredentialVersion::try_from(value).expect("test version must be valid")
    }

    fn into_live(record: StoredCredential) -> StoredLiveCredential {
        let StoredCredential::Live(record) = record else {
            panic!("test fixture must remain live");
        };
        record
    }

    fn static_provider_with_version(
        bytes: [u8; 32],
        version: &'static str,
    ) -> Arc<dyn KeyProvider> {
        Arc::new(StaticKeyProvider::with_version(
            Arc::new(EncryptionKey::from_bytes(bytes)),
            version,
        )) as Arc<dyn KeyProvider>
    }

    fn default_provider() -> Arc<dyn KeyProvider> {
        static_provider_with_version([0x42; 32], "default")
    }

    // =========================================================================
    // Single-key round-trip / AAD / rotation tests (preserved from the
    // pre-provider shape; the switch from `Arc<EncryptionKey>` to
    // `Arc<dyn KeyProvider>` is transparent to every invariant below.)
    // =========================================================================

    #[tokio::test]
    async fn round_trip_encrypts_and_decrypts() -> Result<(), CredentialPersistenceError> {
        let store = EncryptionLayer::new(
            SqliteCredentialPersistence::connect_memory().await?,
            default_provider(),
        );
        let selector = selector(CredentialId::new());
        store
            .create(&selector, make_credential(b"super-secret"))
            .await?;

        let fetched = into_live(store.get(&selector).await?);
        assert_eq!(fetched.data().as_ref(), b"super-secret");
        Ok(())
    }

    #[tokio::test]
    async fn data_is_encrypted_at_rest() -> Result<(), CredentialPersistenceError> {
        let inner = SqliteCredentialPersistence::connect_memory().await?;
        let store = EncryptionLayer::new(inner.clone(), default_provider());

        let selector = selector(CredentialId::new());
        store
            .create(&selector, make_credential(b"plaintext-secret"))
            .await?;

        // Read directly from inner store — data should NOT be plaintext
        let raw = into_live(inner.get(&selector).await?);
        assert_ne!(raw.data().as_ref(), b"plaintext-secret");
        Ok(())
    }

    #[tokio::test]
    async fn management_heads_do_not_read_or_decrypt_material()
    -> Result<(), CredentialPersistenceError> {
        let inner = SqliteCredentialPersistence::connect_memory().await?;
        let credential_id = CredentialId::new();
        let selector = selector(credential_id);
        inner
            .create(&selector, make_credential(b"not-an-encryption-envelope"))
            .await?;
        let store = EncryptionLayer::new(inner, default_provider());

        let head = store.get_head(&selector).await?;
        assert_eq!(head.credential_id(), credential_id);
        let heads = store.list_heads(&owner(), None).await?;
        assert_eq!(heads.len(), 1);
        assert_eq!(heads[0].credential_id(), credential_id);

        let error = store
            .get(&selector)
            .await
            .expect_err("full material read must still reject invalid ciphertext");
        assert_eq!(error, CredentialPersistenceError::CorruptRecord);
        Ok(())
    }

    /// Integration-style check: an OAuth2 credential blob must not be stored as raw JSON
    /// strings in the backend row (at-rest encryption regression).
    #[tokio::test]
    async fn oauth2_state_secrets_not_plaintext_in_inner_store()
    -> Result<(), CredentialPersistenceError> {
        const PLAINTEXT_ACCESS: &str = "nebula-integration-plaintext-access-token-zz";
        const PLAINTEXT_REFRESH: &str = "nebula-integration-plaintext-refresh-zz";

        let inner = SqliteCredentialPersistence::connect_memory().await?;
        let store = EncryptionLayer::new(inner.clone(), default_provider());

        let state = OAuth2State {
            access_token: SecretString::new(PLAINTEXT_ACCESS),
            token_type: "Bearer".to_owned(),
            refresh_token: Some(SecretString::new(PLAINTEXT_REFRESH)),
            expires_at: None,
            scopes: vec!["s1".to_owned()],
            client_id: SecretString::new("c"),
            client_secret: SecretString::new("s"),
            token_url: "https://example.invalid/token".to_owned(),
            auth_style: AuthStyle::Header,
        };
        // Mirror the production seal path: `OAuth2State`'s secret fields emit
        // cleartext only inside the `expose_for_serialization` storage scope.
        // Serializing through it feeds REAL plaintext into the encryption layer,
        // so the "not plaintext in the inner row" assertion below stays
        // meaningful instead of trivially passing on a redacted blob.
        let data = nebula_credential::serde_secret::expose_for_serialization(|| {
            serde_json::to_vec(&state)
        })
        .expect("serialize OAuth2 state");
        let selector = selector(CredentialId::new());
        store.create(&selector, make_credential(&data)).await?;

        let raw = into_live(inner.get(&selector).await?);
        let lossy = String::from_utf8_lossy(raw.data());
        let stored_len = raw.data().len();
        assert!(
            !lossy.contains(PLAINTEXT_ACCESS) && !lossy.contains(PLAINTEXT_REFRESH),
            "inner row must not contain discoverable credential secrets (stored bytes: {stored_len})"
        );
        Ok(())
    }

    #[tokio::test]
    async fn passthrough_operations() -> Result<(), CredentialPersistenceError> {
        let store = EncryptionLayer::new(
            SqliteCredentialPersistence::connect_memory().await?,
            default_provider(),
        );

        let credential_id = CredentialId::new();
        let primary_selector = selector(credential_id);
        let created = store
            .create(&primary_selector, make_credential(b"data"))
            .await?;

        assert!(store.exists(&primary_selector).await?);
        assert!(!store.exists(&selector(CredentialId::new())).await?);

        let ids = store.list(&owner(), None).await?;
        assert_eq!(ids, vec![credential_id]);

        store
            .tombstone(
                &primary_selector,
                CredentialTombstone::new(created.version()),
            )
            .await?;
        assert!(!store.exists(&primary_selector).await?);
        assert!(matches!(
            store.get(&primary_selector).await?,
            StoredCredential::Tombstoned(_)
        ));
        Ok(())
    }

    #[tokio::test]
    async fn aad_prevents_record_swapping() -> Result<(), CredentialPersistenceError> {
        let inner = SqliteCredentialPersistence::connect_memory().await?;
        let store = EncryptionLayer::new(inner.clone(), default_provider());

        let first = selector(CredentialId::new());
        store
            .create(&first, make_credential(b"secret-data"))
            .await?;

        // Read raw encrypted data from inner store and insert it under a different ID
        let raw = into_live(inner.get(&first).await?);
        let second = selector(CredentialId::new());
        inner
            .create(&second, make_credential(raw.data().as_ref()))
            .await?;

        // Reading cred-2 through the encryption layer should fail because
        // the AAD (credential ID) doesn't match
        let err = store.get(&second).await.unwrap_err();
        assert_eq!(err, CredentialPersistenceError::CorruptRecord);
        Ok(())
    }

    #[tokio::test]
    async fn rejects_data_without_aad() -> Result<(), CredentialPersistenceError> {
        let inner = SqliteCredentialPersistence::connect_memory().await?;
        let key = EncryptionKey::from_bytes([0x42; 32]);

        // Construct a legacy-shaped envelope: encrypted with the *current*
        // provider's key_id ("default" — found via `default_provider()`)
        // but with an EMPTY AAD. The encryption layer will:
        //   1. Look up the key under "default" — succeeds (matches provider).
        //   2. Decrypt with credential_id ("legacy-1") as AAD — fails because the envelope was
        //      sealed with empty AAD.
        // This is the AAD-mandatory rejection path; no legacy fallback.
        //
        // SEC-11 (security hardening 2026-04-27 Stage 1) removed the bare
        // `crypto::encrypt(key, plaintext)` helper from public surface.
        // `encrypt_with_key_id(key, "default", plaintext, b"")` is the
        // public AAD-aware alternative that produces the same legacy shape
        // (valid key_id + empty AAD) without exposing a no-AAD shortcut.
        let envelope = encrypt_with_key_id(&key, "default", b"legacy-secret", b"")
            .expect("encrypt with empty AAD should succeed at the crypto layer");
        let encrypted_bytes = serde_json::to_vec(&envelope).unwrap();

        let selector = selector(CredentialId::new());
        inner
            .create(&selector, make_credential(&encrypted_bytes))
            .await?;

        // Reading through the encryption layer must fail with an AAD
        // mismatch: the envelope was sealed with empty AAD but the layer
        // unconditionally binds `credential_id` as AAD on decrypt.
        let store = EncryptionLayer::new(inner, default_provider());
        let err = store.get(&selector).await.unwrap_err();
        assert_eq!(err, CredentialPersistenceError::CorruptRecord);
        Ok(())
    }

    #[tokio::test]
    async fn wrong_key_fails_decryption() -> Result<(), CredentialPersistenceError> {
        let inner = SqliteCredentialPersistence::connect_memory().await?;
        let provider1 = static_provider_with_version([0x01; 32], "default");
        let provider2 = static_provider_with_version([0x02; 32], "default");

        let store1 = EncryptionLayer::new(inner.clone(), provider1);
        let selector = selector(CredentialId::new());
        store1.create(&selector, make_credential(b"secret")).await?;

        let store2 = EncryptionLayer::new(inner, provider2);
        let err = store2.get(&selector).await.unwrap_err();
        assert_eq!(err, CredentialPersistenceError::CorruptRecord);
        Ok(())
    }

    // =========================================================================
    // Multi-key / key rotation tests
    // =========================================================================

    #[tokio::test]
    async fn single_key_mode_stores_key_id() -> Result<(), CredentialPersistenceError> {
        let inner = SqliteCredentialPersistence::connect_memory().await?;
        let store = EncryptionLayer::new(inner.clone(), default_provider());

        let selector = selector(CredentialId::new());
        store.create(&selector, make_credential(b"secret")).await?;

        // Inspect the raw bytes stored — should contain "default" as key_id
        let raw = into_live(inner.get(&selector).await?);
        let envelope: EncryptedData = serde_json::from_slice(raw.data()).unwrap();
        assert_eq!(envelope.key_id, "default");
        Ok(())
    }

    #[tokio::test]
    async fn multi_key_round_trip() -> Result<(), CredentialPersistenceError> {
        let key1 = Arc::new(EncryptionKey::from_bytes([0x01; 32]));
        let provider = Arc::new(StaticKeyProvider::with_version(
            Arc::new(EncryptionKey::from_bytes([0x02; 32])),
            "key-2",
        )) as Arc<dyn KeyProvider>;
        let store = EncryptionLayer::with_legacy_keys(
            SqliteCredentialPersistence::connect_memory().await?,
            provider,
            vec![("key-1".to_string(), key1)],
        );

        let selector = selector(CredentialId::new());
        store
            .create(&selector, make_credential(b"multi-key-secret"))
            .await?;

        let fetched = into_live(store.get(&selector).await?);
        assert_eq!(fetched.data().as_ref(), b"multi-key-secret");
        Ok(())
    }

    #[tokio::test]
    async fn decrypt_with_old_key_succeeds() -> Result<(), CredentialPersistenceError> {
        let inner = SqliteCredentialPersistence::connect_memory().await?;
        let key1_bytes = [0x01; 32];
        let key2_bytes = [0x02; 32];

        // Write with old key (key-1 is current)
        let store_old = EncryptionLayer::new(
            inner.clone(),
            static_provider_with_version(key1_bytes, "key-1"),
        );
        let selector = selector(CredentialId::new());
        store_old
            .create(&selector, make_credential(b"old-key-data"))
            .await?;

        // Now rotate: key-2 is current, key-1 available as a legacy decrypt-only key
        let store_new = EncryptionLayer::with_legacy_keys(
            inner.clone(),
            static_provider_with_version(key2_bytes, "key-2"),
            vec![(
                "key-1".to_string(),
                Arc::new(EncryptionKey::from_bytes(key1_bytes)),
            )],
        );

        let fetched = into_live(store_new.get(&selector).await?);
        assert_eq!(fetched.data().as_ref(), b"old-key-data");
        Ok(())
    }

    /// A legacy-key read is observational: it must not advance the durable
    /// version or mutate the encrypted envelope behind the caller's back.
    #[tokio::test]
    async fn legacy_key_read_preserves_version_and_envelope()
    -> Result<(), CredentialPersistenceError> {
        let inner = SqliteCredentialPersistence::connect_memory().await?;
        let key1_bytes = [0x01; 32];
        let key2_bytes = [0x02; 32];

        let store_old = EncryptionLayer::new(
            inner.clone(),
            static_provider_with_version(key1_bytes, "key-1"),
        );
        let selector = selector(CredentialId::new());
        let pre_rotation = store_old
            .create(&selector, make_credential(b"needs-rotation"))
            .await?;
        let version_before_rotation = pre_rotation.version();

        let raw_before = into_live(inner.get(&selector).await?);

        // Read through the new layer using the explicitly configured legacy key.
        let store_new = EncryptionLayer::with_legacy_keys(
            inner.clone(),
            static_provider_with_version(key2_bytes, "key-2"),
            vec![(
                "key-1".to_string(),
                Arc::new(EncryptionKey::from_bytes(key1_bytes)),
            )],
        );
        let fetched = into_live(store_new.get(&selector).await?);

        let raw_after = into_live(inner.get(&selector).await?);
        assert_eq!(fetched.data().as_ref(), b"needs-rotation");
        assert_eq!(
            fetched.version(),
            version_before_rotation,
            "a read must return the existing durable version"
        );
        assert_eq!(
            raw_after.version(),
            raw_before.version(),
            "a read must not advance the durable version"
        );
        assert_eq!(raw_after.updated_at(), raw_before.updated_at());
        assert_eq!(raw_after.data(), raw_before.data());
        Ok(())
    }

    #[tokio::test]
    async fn real_update_after_legacy_read_rotates_exactly_once()
    -> Result<(), CredentialPersistenceError> {
        let inner = SqliteCredentialPersistence::connect_memory().await?;
        let key1_bytes = [0x01; 32];
        let key2_bytes = [0x02; 32];

        // Write with key-1
        let store_old = EncryptionLayer::new(
            inner.clone(),
            static_provider_with_version(key1_bytes, "key-1"),
        );
        let selector = selector(CredentialId::new());
        let originally_stored = store_old
            .create(&selector, make_credential(b"will-be-rotated"))
            .await?;

        // A read decrypts through key-1 but remains side-effect free.
        let store_new = EncryptionLayer::with_legacy_keys(
            inner.clone(),
            static_provider_with_version(key2_bytes, "key-2"),
            vec![(
                "key-1".to_string(),
                Arc::new(EncryptionKey::from_bytes(key1_bytes)),
            )],
        );
        let fetched = into_live(store_new.get(&selector).await?);
        assert_eq!(fetched.data().as_ref(), b"will-be-rotated");
        assert_eq!(fetched.version(), originally_stored.version());

        // The next semantic write rotates to the current key and consumes one
        // version, instead of a hidden read consuming a separate version.
        let updated = store_new
            .replace(
                &selector,
                make_replacement(originally_stored.version(), b"rotated"),
            )
            .await?;
        assert_eq!(updated.version(), version(2));

        // The one real mutation encrypted with key-2 in the backing store.
        let raw = into_live(inner.get(&selector).await?);
        let envelope: EncryptedData = serde_json::from_slice(raw.data()).unwrap();
        assert_eq!(envelope.key_id, "key-2");
        assert_eq!(raw.version(), updated.version());
        Ok(())
    }

    /// Regression for GitHub issue #281: `new()` no longer aliases the key
    /// under `""`, so legacy envelopes with `key_id: ""` cannot silently
    /// decrypt with the current key. Operators who still hold such records
    /// must register the alias explicitly via `with_legacy_keys`.
    ///
    /// The current `encrypt_with_key_id` refuses to produce new envelopes
    /// with an empty `key_id`, so this test mutates a legitimately-encrypted
    /// envelope to simulate a pre-guard legacy record.
    #[tokio::test]
    async fn new_does_not_silently_decrypt_empty_key_id_envelopes()
    -> Result<(), CredentialPersistenceError> {
        let inner = SqliteCredentialPersistence::connect_memory().await?;
        let key_bytes = [0x42; 32];
        let key = Arc::new(EncryptionKey::from_bytes(key_bytes));

        // Encrypt normally under "default", then mutate key_id to "" to
        // simulate a legacy pre-guard envelope persisted by an older build.
        let plaintext = b"legacy-record";
        let selector = selector(CredentialId::new());
        let aad = selector.credential_id().to_string();
        let mut legacy_envelope =
            encrypt_with_key_id(&key, "default", plaintext, aad.as_bytes()).unwrap();
        legacy_envelope.key_id = String::new();
        let envelope_bytes = serde_json::to_vec(&legacy_envelope).unwrap();

        inner
            .create(&selector, make_credential(&envelope_bytes))
            .await?;

        // `new(_, provider)` must refuse to decrypt the `""`-tagged record —
        // the empty alias no longer maps to the default key.
        let store = EncryptionLayer::new(
            inner.clone(),
            static_provider_with_version(key_bytes, "default"),
        );
        let err = store.get(&selector).await.unwrap_err();
        assert!(
            matches!(&err, CredentialPersistenceError::CorruptRecord),
            "expected a corruption error for unknown key_id, got {err:?}",
        );

        // Explicit opt-in via `with_legacy_keys` still works — the migration
        // path documented on `new()` succeeds.
        let store_with_legacy = EncryptionLayer::with_legacy_keys(
            inner,
            static_provider_with_version(key_bytes, "default"),
            vec![(String::new(), Arc::clone(&key))],
        );
        let fetched = into_live(store_with_legacy.get(&selector).await?);
        assert_eq!(fetched.data().as_ref(), plaintext);
        Ok(())
    }
}
