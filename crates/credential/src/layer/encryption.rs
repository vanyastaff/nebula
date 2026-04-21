//! Encryption layer -- encrypts data before storage, decrypts after retrieval.
//!
//! Wraps any [`CredentialStore`] implementation and applies AES-256-GCM
//! encryption to the [`StoredCredential::data`] field. The credential ID is
//! bound as Additional Authenticated Data (AAD), preventing record-swapping
//! attacks where encrypted data from one credential is copied to another.
//!
//! AAD validation is mandatory — data encrypted without AAD (or with a
//! mismatched credential ID) is rejected with a hard error. There is no
//! legacy fallback path.
//!
//! # Key source: [`KeyProvider`]
//!
//! The current encryption key is supplied by an
//! [`Arc<dyn KeyProvider>`](super::KeyProvider), **not** an `Arc<EncryptionKey>`
//! directly. Composition roots choose the provider (env var, file, KMS, …)
//! at wiring time — see
//! [ADR-0023](../../../../docs/adr/0023-keyprovider-trait.md) for the seam
//! and ADR-0020 §3 for why this gate exists.
//!
//! # Key rotation
//!
//! On every read the layer inspects `EncryptedData::key_id`:
//!
//! - If `key_id` matches `self.key_provider.version()`, decrypt with the provider's current key.
//! - If `key_id` differs, look it up in the optional `legacy_keys` map — decrypt with the legacy
//!   key and re-encrypt with the current key before returning (lazy rotation). `legacy_keys` is
//!   populated via [`EncryptionLayer::with_legacy_keys`] when the operator is migrating off an
//!   older key.

use std::{collections::HashMap, sync::Arc};

use crate::{
    EncryptedData, EncryptionKey, decrypt_with_aad, encrypt_with_key_id,
    layer::key_provider::KeyProvider,
    store::{CredentialStore, PutMode, StoreError, StoredCredential},
};

/// Wraps a store with AES-256-GCM encryption on the `data` field.
///
/// The current key is supplied by the configured [`KeyProvider`]. Records
/// encrypted with an older key may optionally be decrypted via `legacy_keys`
/// (populated by [`Self::with_legacy_keys`]); they are then re-encrypted
/// with the current key on read (lazy rotation).
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_credential::{EncryptionLayer, EnvKeyProvider, InMemoryStore};
/// use std::sync::Arc;
///
/// // Production: read the key from NEBULA_CRED_MASTER_KEY.
/// let provider = Arc::new(EnvKeyProvider::from_env()?);
/// let store = EncryptionLayer::new(InMemoryStore::new(), provider);
/// # Ok::<(), Box<dyn std::error::Error>>(())
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
    /// ```rust,ignore
    /// EncryptionLayer::with_legacy_keys(
    ///     inner,
    ///     Arc::new(EnvKeyProvider::from_env()?), // provider drives "default"
    ///     vec![(String::new(), legacy_key)],     // explicit, audit-visible
    /// );
    /// ```
    ///
    /// The lazy rotation path (see module docs) will then re-encrypt any
    /// `""` record with the provider's current version on next read.
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
    /// decrypts with the legacy key and re-encrypts with the current key
    /// (lazy rotation).
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

    fn current_key(&self) -> Result<Arc<EncryptionKey>, StoreError> {
        self.key_provider
            .current_key()
            .map_err(|e| StoreError::Backend(Box::new(e)))
    }

    fn current_key_id(&self) -> &str {
        self.key_provider.version()
    }

    fn legacy_key(&self, key_id: &str) -> Result<Arc<EncryptionKey>, StoreError> {
        self.legacy_keys.get(key_id).cloned().ok_or_else(|| {
            StoreError::Backend(
                format!("encryption key '{key_id}' not found — cannot decrypt").into(),
            )
        })
    }
}

impl<S: CredentialStore> CredentialStore for EncryptionLayer<S> {
    async fn get(&self, id: &str) -> Result<StoredCredential, StoreError> {
        let mut credential = self.inner.get(id).await?;
        let (mut plaintext, rotated) = self.decrypt_possibly_rotating(&credential.data, id)?;
        credential.data = std::mem::take(&mut *plaintext);

        if let Some(re_encrypted) = rotated {
            // Use CAS to avoid clobbering concurrent updates. If the record
            // changed since we read it, skip — rotation will happen on next read.
            let updated = StoredCredential {
                data: re_encrypted,
                ..credential.clone()
            };
            match self
                .inner
                .put(
                    updated,
                    PutMode::CompareAndSwap {
                        expected_version: credential.version,
                    },
                )
                .await
            {
                Ok(stored_after_rotation) => {
                    // The CAS write bumped `version` (and `updated_at`) to the
                    // post-rotation row. Propagate that to the caller so its
                    // own subsequent CAS updates target the fresh row rather
                    // than phantom-conflicting against our lazy re-encrypt
                    // write (GitHub issue #282).
                    credential.version = stored_after_rotation.version;
                    credential.updated_at = stored_after_rotation.updated_at;
                },
                Err(StoreError::VersionConflict { .. }) => {},
                Err(other) => return Err(other),
            }
        }

        Ok(credential)
    }

    async fn put(
        &self,
        mut credential: StoredCredential,
        mode: PutMode,
    ) -> Result<StoredCredential, StoreError> {
        let id = credential.id.clone();
        let plaintext_data = credential.data.clone();
        credential.data = self.encrypt_data(&plaintext_data, &id)?;
        let mut stored = self.inner.put(credential, mode).await?;
        // Restore original plaintext instead of decrypting again
        stored.data = plaintext_data;
        Ok(stored)
    }

    async fn delete(&self, id: &str) -> Result<(), StoreError> {
        self.inner.delete(id).await
    }

    async fn list(&self, state_kind: Option<&str>) -> Result<Vec<String>, StoreError> {
        self.inner.list(state_kind).await
    }

    async fn exists(&self, id: &str) -> Result<bool, StoreError> {
        self.inner.exists(id).await
    }
}

impl<S> EncryptionLayer<S> {
    /// Encrypt `plaintext` with the current key, serializing the envelope to bytes.
    fn encrypt_data(&self, plaintext: &[u8], id: &str) -> Result<Vec<u8>, StoreError> {
        let key = self.current_key()?;
        let current_version = self.current_key_id();
        let encrypted = encrypt_with_key_id(&key, current_version, plaintext, id.as_bytes())
            .map_err(|e| StoreError::Backend(Box::new(e)))?;
        serde_json::to_vec(&encrypted).map_err(|e| StoreError::Backend(Box::new(e)))
    }

    /// Decrypt `ciphertext`, returning `(plaintext, Some(re_encrypted_bytes))` when
    /// the data was stored under an old key and must be lazily rotated, or
    /// `(plaintext, None)` when no rotation is needed.
    ///
    /// AAD (credential ID) is always enforced — data without AAD or with a
    /// mismatched ID is rejected.
    #[expect(
        clippy::type_complexity,
        reason = "return type is (plaintext, optional re-encryption bytes); a type alias would obscure meaning without reducing complexity — function is private"
    )]
    // Reason: The return type is (plaintext, optional re-encryption bytes) — a type alias
    // here would obscure the meaning without reducing complexity. The function is private.
    fn decrypt_possibly_rotating(
        &self,
        ciphertext: &[u8],
        id: &str,
    ) -> Result<(zeroize::Zeroizing<Vec<u8>>, Option<Vec<u8>>), StoreError> {
        let encrypted: EncryptedData =
            serde_json::from_slice(ciphertext).map_err(|e| StoreError::Backend(Box::new(e)))?;

        let current_version = self.current_key_id();

        // Data encrypted with the current key — normal path.
        if encrypted.key_id == current_version {
            let key = self.current_key()?;
            let plaintext = decrypt_with_aad(&key, &encrypted, id.as_bytes())
                .map_err(|e| StoreError::Backend(Box::new(e)))?;
            return Ok((plaintext, None));
        }

        // Data encrypted with an older key — decrypt with legacy key, re-encrypt.
        let old_key = self.legacy_key(&encrypted.key_id)?;
        let plaintext = decrypt_with_aad(&old_key, &encrypted, id.as_bytes())
            .map_err(|e| StoreError::Backend(Box::new(e)))?;
        let re_encrypted = self.encrypt_data(&plaintext, id)?;
        Ok((plaintext, Some(re_encrypted)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        encrypt,
        layer::key_provider::StaticKeyProvider,
        store::{PutMode, test_helpers::make_credential},
        store_memory::InMemoryStore,
    };

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
    async fn round_trip_encrypts_and_decrypts() {
        let store = EncryptionLayer::new(InMemoryStore::new(), default_provider());
        let cred = make_credential("enc-1", b"super-secret");

        let stored = store.put(cred, PutMode::CreateOnly).await.unwrap();
        assert_eq!(stored.data, b"super-secret");

        let fetched = store.get("enc-1").await.unwrap();
        assert_eq!(fetched.data, b"super-secret");
    }

    #[tokio::test]
    async fn data_is_encrypted_at_rest() {
        let inner = InMemoryStore::new();
        let store = EncryptionLayer::new(inner.clone(), default_provider());

        let cred = make_credential("enc-2", b"plaintext-secret");
        store.put(cred, PutMode::CreateOnly).await.unwrap();

        // Read directly from inner store — data should NOT be plaintext
        let raw = inner.get("enc-2").await.unwrap();
        assert_ne!(raw.data, b"plaintext-secret");
    }

    #[tokio::test]
    async fn passthrough_operations() {
        let store = EncryptionLayer::new(InMemoryStore::new(), default_provider());

        let cred = make_credential("enc-3", b"data");
        store.put(cred, PutMode::CreateOnly).await.unwrap();

        assert!(store.exists("enc-3").await.unwrap());
        assert!(!store.exists("missing").await.unwrap());

        let ids = store.list(None).await.unwrap();
        assert_eq!(ids, vec!["enc-3"]);

        store.delete("enc-3").await.unwrap();
        assert!(!store.exists("enc-3").await.unwrap());
    }

    #[tokio::test]
    async fn aad_prevents_record_swapping() {
        let inner = InMemoryStore::new();
        let store = EncryptionLayer::new(inner.clone(), default_provider());

        let cred = make_credential("cred-1", b"secret-data");
        store.put(cred, PutMode::CreateOnly).await.unwrap();

        // Read raw encrypted data from inner store and insert it under a different ID
        let raw = inner.get("cred-1").await.unwrap();
        let swapped = StoredCredential {
            id: "cred-2".into(),
            ..raw
        };
        inner.put(swapped, PutMode::CreateOnly).await.unwrap();

        // Reading cred-2 through the encryption layer should fail because
        // the AAD (credential ID) doesn't match
        let err = store.get("cred-2").await.unwrap_err();
        assert!(matches!(err, StoreError::Backend(_)));
    }

    #[tokio::test]
    async fn rejects_data_without_aad() {
        let inner = InMemoryStore::new();
        let key = Arc::new(EncryptionKey::from_bytes([0x42; 32]));

        // Simulate legacy write: encrypt without AAD and store directly
        let plaintext = b"legacy-secret";
        let encrypted = encrypt(&key, plaintext).unwrap();
        let encrypted_bytes = serde_json::to_vec(&encrypted).unwrap();

        let cred = StoredCredential {
            id: "legacy-1".into(),
            credential_key: "test_credential".into(),
            data: encrypted_bytes,
            state_kind: "test".into(),
            state_version: 1,
            version: 0,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            expires_at: None,
            metadata: Default::default(),
        };
        inner.put(cred, PutMode::CreateOnly).await.unwrap();

        // Reading through the encryption layer must fail: AAD is mandatory.
        // Data encrypted without AAD is unreadable — no legacy fallback.
        let store = EncryptionLayer::new(inner, default_provider());
        let err = store.get("legacy-1").await.unwrap_err();
        assert!(matches!(err, StoreError::Backend(_)));
    }

    #[tokio::test]
    async fn wrong_key_fails_decryption() {
        let inner = InMemoryStore::new();
        let provider1 = static_provider_with_version([0x01; 32], "default");
        let provider2 = static_provider_with_version([0x02; 32], "default");

        let store1 = EncryptionLayer::new(inner.clone(), provider1);
        let cred = make_credential("enc-4", b"secret");
        store1.put(cred, PutMode::CreateOnly).await.unwrap();

        let store2 = EncryptionLayer::new(inner, provider2);
        let err = store2.get("enc-4").await.unwrap_err();
        assert!(matches!(err, StoreError::Backend(_)));
    }

    // =========================================================================
    // Multi-key / key rotation tests
    // =========================================================================

    #[tokio::test]
    async fn single_key_mode_stores_key_id() {
        let inner = InMemoryStore::new();
        let store = EncryptionLayer::new(inner.clone(), default_provider());

        let cred = make_credential("key-id-check", b"secret");
        store.put(cred, PutMode::CreateOnly).await.unwrap();

        // Inspect the raw bytes stored — should contain "default" as key_id
        let raw = inner.get("key-id-check").await.unwrap();
        let envelope: EncryptedData = serde_json::from_slice(&raw.data).unwrap();
        assert_eq!(envelope.key_id, "default");
    }

    #[tokio::test]
    async fn multi_key_round_trip() {
        let key1 = Arc::new(EncryptionKey::from_bytes([0x01; 32]));
        let provider = Arc::new(StaticKeyProvider::with_version(
            Arc::new(EncryptionKey::from_bytes([0x02; 32])),
            "key-2",
        )) as Arc<dyn KeyProvider>;
        let store = EncryptionLayer::with_legacy_keys(
            InMemoryStore::new(),
            provider,
            vec![("key-1".to_string(), key1)],
        );

        let cred = make_credential("mk-1", b"multi-key-secret");
        store.put(cred, PutMode::CreateOnly).await.unwrap();

        let fetched = store.get("mk-1").await.unwrap();
        assert_eq!(fetched.data, b"multi-key-secret");
    }

    #[tokio::test]
    async fn decrypt_with_old_key_succeeds() {
        let inner = InMemoryStore::new();
        let key1_bytes = [0x01; 32];
        let key2_bytes = [0x02; 32];

        // Write with old key (key-1 is current)
        let store_old = EncryptionLayer::new(
            inner.clone(),
            static_provider_with_version(key1_bytes, "key-1"),
        );
        let cred = make_credential("rotate-1", b"old-key-data");
        store_old.put(cred, PutMode::CreateOnly).await.unwrap();

        // Now rotate: key-2 is current, key-1 available as a legacy decrypt-only key
        let store_new = EncryptionLayer::with_legacy_keys(
            inner.clone(),
            static_provider_with_version(key2_bytes, "key-2"),
            vec![(
                "key-1".to_string(),
                Arc::new(EncryptionKey::from_bytes(key1_bytes)),
            )],
        );

        let fetched = store_new.get("rotate-1").await.unwrap();
        assert_eq!(fetched.data, b"old-key-data");
    }

    /// Regression for GitHub issue #282: `get()` on a record that triggers
    /// lazy re-encryption used to return the `StoredCredential` with the
    /// pre-CAS `version`. Downstream callers then hit a phantom
    /// [`StoreError::VersionConflict`] on their own CAS update against the
    /// row we just bumped. The returned struct must carry the post-rotation
    /// `version` (and `updated_at`) so downstream CAS targets the fresh row.
    #[tokio::test]
    async fn lazy_reencryption_returns_post_rotation_version() {
        let inner = InMemoryStore::new();
        let key1_bytes = [0x01; 32];
        let key2_bytes = [0x02; 32];

        let store_old = EncryptionLayer::new(
            inner.clone(),
            static_provider_with_version(key1_bytes, "key-1"),
        );
        let cred = make_credential("rotate-version-1", b"needs-rotation");
        let pre_rotation = store_old.put(cred, PutMode::CreateOnly).await.unwrap();
        let version_before_rotation = pre_rotation.version;

        // Read through new layer — triggers lazy rotation + CAS write.
        let store_new = EncryptionLayer::with_legacy_keys(
            inner.clone(),
            static_provider_with_version(key2_bytes, "key-2"),
            vec![(
                "key-1".to_string(),
                Arc::new(EncryptionKey::from_bytes(key1_bytes)),
            )],
        );
        let fetched = store_new.get("rotate-version-1").await.unwrap();

        let current_raw = inner.get("rotate-version-1").await.unwrap();
        assert_eq!(
            fetched.version, current_raw.version,
            "returned version must match persisted post-rotation row"
        );
        assert_eq!(
            fetched.updated_at, current_raw.updated_at,
            "returned updated_at must match persisted post-rotation row"
        );
        assert!(
            fetched.version > version_before_rotation,
            "returned version must be bumped past pre-rotation value"
        );
    }

    #[tokio::test]
    async fn lazy_reencryption_on_read_when_key_id_differs() {
        let inner = InMemoryStore::new();
        let key1_bytes = [0x01; 32];
        let key2_bytes = [0x02; 32];

        // Write with key-1
        let store_old = EncryptionLayer::new(
            inner.clone(),
            static_provider_with_version(key1_bytes, "key-1"),
        );
        let cred = make_credential("lazy-1", b"will-be-rotated");
        store_old.put(cred, PutMode::CreateOnly).await.unwrap();

        // Read through new layer — triggers lazy rotation
        let store_new = EncryptionLayer::with_legacy_keys(
            inner.clone(),
            static_provider_with_version(key2_bytes, "key-2"),
            vec![(
                "key-1".to_string(),
                Arc::new(EncryptionKey::from_bytes(key1_bytes)),
            )],
        );
        let fetched = store_new.get("lazy-1").await.unwrap();
        assert_eq!(fetched.data, b"will-be-rotated");

        // Verify the data was re-encrypted with key-2 in the backing store
        let raw = inner.get("lazy-1").await.unwrap();
        let envelope: EncryptedData = serde_json::from_slice(&raw.data).unwrap();
        assert_eq!(envelope.key_id, "key-2");
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
    async fn new_does_not_silently_decrypt_empty_key_id_envelopes() {
        let inner = InMemoryStore::new();
        let key_bytes = [0x42; 32];
        let key = Arc::new(EncryptionKey::from_bytes(key_bytes));

        // Encrypt normally under "default", then mutate key_id to "" to
        // simulate a legacy pre-guard envelope persisted by an older build.
        let plaintext = b"legacy-record";
        let mut legacy_envelope =
            encrypt_with_key_id(&key, "default", plaintext, b"legacy-1").unwrap();
        legacy_envelope.key_id = String::new();
        let envelope_bytes = serde_json::to_vec(&legacy_envelope).unwrap();

        let cred = StoredCredential {
            id: "legacy-1".into(),
            credential_key: "test_credential".into(),
            data: envelope_bytes,
            state_kind: "test".into(),
            state_version: 1,
            version: 0,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            expires_at: None,
            metadata: Default::default(),
        };
        inner.put(cred, PutMode::CreateOnly).await.unwrap();

        // `new(_, provider)` must refuse to decrypt the `""`-tagged record —
        // the empty alias no longer maps to the default key.
        let store = EncryptionLayer::new(
            inner.clone(),
            static_provider_with_version(key_bytes, "default"),
        );
        let err = store.get("legacy-1").await.unwrap_err();
        assert!(
            matches!(&err, StoreError::Backend(_)),
            "expected a Backend error for unknown key_id, got {err:?}",
        );

        // Explicit opt-in via `with_legacy_keys` still works — the migration
        // path documented on `new()` succeeds.
        let store_with_legacy = EncryptionLayer::with_legacy_keys(
            inner,
            static_provider_with_version(key_bytes, "default"),
            vec![(String::new(), Arc::clone(&key))],
        );
        let fetched = store_with_legacy.get("legacy-1").await.unwrap();
        assert_eq!(fetched.data, plaintext);
    }
}
