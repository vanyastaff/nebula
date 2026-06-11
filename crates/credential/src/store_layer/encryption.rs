//! Encryption layer — encrypts data before storage, decrypts after retrieval.
//!
//! Wraps any [`CredentialStore`] implementation and applies AES-256-GCM
//! encryption to the [`StoredCredential::data`] field. The credential ID is
//! bound as Additional Authenticated Data (AAD), preventing record-swapping
//! attacks where encrypted data from one credential is copied to another.
//!
//! AAD validation is mandatory — data encrypted without AAD (or with a
//! mismatched credential ID) is rejected with a hard error. There is no
//! legacy fallback path. This invariant is preserved by construction: the
//! [`nebula_crypto::Cipher`] trait exposes no no-AAD encrypt method (SEC-11).
//!
//! # Key source: [`KeyProvider`]
//!
//! The current encryption key is supplied by an
//! [`Arc<dyn KeyProvider>`](super::key_provider::KeyProvider), **not** an `Arc<EncryptionKey>`
//! directly. Composition roots choose the provider (env var, file, KMS, …)
//! at wiring time.
//!
//! # Cipher port (ADR-0092)
//!
//! The concrete AES-256-GCM algorithm is now injected via
//! [`Arc<dyn nebula_crypto::Cipher>`]. The default is
//! [`AesGcmCipher`](nebula_crypto::AesGcmCipher), which delegates to the
//! same free functions as before — zero behaviour change. Inject a different
//! impl via [`EncryptionLayer::with_cipher`] for algorithm-agility or testing.
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

use nebula_crypto::{AesGcmCipher, Cipher, EncryptedData, EncryptionKey};

use crate::{CredentialStore, PutMode, StoreError, StoredCredential};

use super::key_provider::KeyProvider;

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
/// use nebula_credential::store_layer::{EncryptionLayer, EnvKeyProvider};
/// use std::sync::Arc;
///
/// // Production: read the key from NEBULA_CRED_MASTER_KEY.
/// let provider = Arc::new(EnvKeyProvider::from_env()?);
/// let backend = /* any CredentialStore impl */;
/// let store = EncryptionLayer::new(backend, provider);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub struct EncryptionLayer<S> {
    inner: S,
    key_provider: Arc<dyn KeyProvider>,
    legacy_keys: HashMap<String, Arc<EncryptionKey>>,
    cipher: Arc<dyn Cipher>,
}

impl<S> EncryptionLayer<S> {
    /// Create an encryption layer whose current key is sourced from
    /// `key_provider`. Uses [`AesGcmCipher`] as the default cipher.
    ///
    /// # Legacy `""` records
    ///
    /// Earlier builds of this layer aliased the key under the empty string
    /// `""` so that legacy envelopes with `key_id: ""` would silently decrypt
    /// with the current key. That alias was removed (GitHub issue #281):
    /// silent cross-identity decryption breaks the key-rotation invariant
    /// (PRODUCT_CANON §4.2 / §12.5) and makes `key_id`-based audit provenance
    /// unreliable. Deployments that still hold `""` envelopes must register
    /// the empty alias explicitly via [`with_legacy_keys`](Self::with_legacy_keys).
    pub fn new(inner: S, key_provider: Arc<dyn KeyProvider>) -> Self {
        Self {
            inner,
            key_provider,
            legacy_keys: HashMap::new(),
            cipher: Arc::new(AesGcmCipher),
        }
    }

    /// Create an encryption layer with additional decrypt-only keys for
    /// rotation support. Uses [`AesGcmCipher`] as the default cipher.
    ///
    /// `key_provider` supplies the current encrypt/decrypt key; `legacy_keys`
    /// contains historical keys that remain valid for reads. On read of a
    /// record whose envelope `key_id` matches a legacy entry, the layer
    /// decrypts with the legacy key and re-encrypts with the current key
    /// (lazy rotation).
    pub fn with_legacy_keys(
        inner: S,
        key_provider: Arc<dyn KeyProvider>,
        legacy_keys: Vec<(String, Arc<EncryptionKey>)>,
    ) -> Self {
        Self {
            inner,
            key_provider,
            legacy_keys: legacy_keys.into_iter().collect(),
            cipher: Arc::new(AesGcmCipher),
        }
    }

    /// Override the cipher implementation used for encrypt/decrypt.
    ///
    /// The default is [`AesGcmCipher`]. Pass a custom impl here for
    /// algorithm-agility (ChaCha20-Poly1305, HSM-backed) or test injection.
    /// The SEC-11 no-AAD invariant is preserved by construction: the
    /// [`Cipher`] trait exposes no no-AAD encrypt method.
    #[must_use]
    pub fn with_cipher(mut self, cipher: Arc<dyn Cipher>) -> Self {
        self.cipher = cipher;
        self
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
        // Restore original plaintext instead of decrypting again.
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
        let encrypted = self
            .cipher
            .encrypt_with_key_id(&key, current_version, plaintext, id.as_bytes())
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
            let plaintext = self
                .cipher
                .decrypt_with_aad(&key, &encrypted, id.as_bytes())
                .map_err(|e| StoreError::Backend(Box::new(e)))?;
            return Ok((plaintext, None));
        }

        // Data encrypted with an older key — decrypt with legacy key, re-encrypt.
        let old_key = self.legacy_key(&encrypted.key_id)?;
        let plaintext = self
            .cipher
            .decrypt_with_aad(&old_key, &encrypted, id.as_bytes())
            .map_err(|e| StoreError::Backend(Box::new(e)))?;
        let re_encrypted = self.encrypt_data(&plaintext, id)?;
        Ok((plaintext, Some(re_encrypted)))
    }
}

// ============================================================================
// Tests — use an in-memory CredentialStore double (no sqlx dependency).
// ============================================================================

#[cfg(test)]
mod tests {
    use nebula_credential_macros as _; // keep macro support wired

    use nebula_crypto::{EncryptionKey, encrypt_with_key_id};

    use crate::{
        AuthStyle, PutMode, SecretString,
        credentials::oauth2::OAuth2State,
        store::test_helpers::make_credential,
        store_layer::{InMemoryCredentialStore, key_provider::StaticKeyProvider},
    };

    use super::*;

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

    fn make_store(provider: Arc<dyn KeyProvider>) -> EncryptionLayer<InMemoryCredentialStore> {
        EncryptionLayer::new(InMemoryCredentialStore::new(), provider)
    }

    fn make_store_with_legacy(
        provider: Arc<dyn KeyProvider>,
        legacy: Vec<(String, Arc<EncryptionKey>)>,
    ) -> EncryptionLayer<InMemoryCredentialStore> {
        EncryptionLayer::with_legacy_keys(InMemoryCredentialStore::new(), provider, legacy)
    }

    // =========================================================================
    // Single-key round-trip / AAD / rotation tests
    // =========================================================================

    #[tokio::test]
    async fn round_trip_encrypts_and_decrypts() -> Result<(), StoreError> {
        let store = make_store(default_provider());
        let cred = make_credential("enc-1", b"super-secret");

        let stored = store.put(cred, PutMode::CreateOnly).await.unwrap();
        assert_eq!(stored.data, b"super-secret");

        let fetched = store.get("enc-1").await.unwrap();
        assert_eq!(fetched.data, b"super-secret");
        Ok(())
    }

    #[tokio::test]
    async fn data_is_encrypted_at_rest() -> Result<(), StoreError> {
        let inner = InMemoryCredentialStore::new();
        let store = EncryptionLayer::new(inner.clone(), default_provider());

        let cred = make_credential("enc-2", b"plaintext-secret");
        store.put(cred, PutMode::CreateOnly).await.unwrap();

        // Read directly from inner store — data should NOT be plaintext.
        let raw = inner.get("enc-2").await.unwrap();
        assert_ne!(raw.data, b"plaintext-secret");
        Ok(())
    }

    /// Integration-style check: an OAuth2 credential blob must not be stored as raw JSON
    /// strings in the backend row (at-rest encryption regression).
    #[tokio::test]
    async fn oauth2_state_secrets_not_plaintext_in_inner_store() -> Result<(), StoreError> {
        const PLAINTEXT_ACCESS: &str = "nebula-integration-plaintext-access-token-zz";
        const PLAINTEXT_REFRESH: &str = "nebula-integration-plaintext-refresh-zz";

        let inner = InMemoryCredentialStore::new();
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
        let data = serde_json::to_vec(&state).expect("serialize OAuth2 state");
        let cred = make_credential("enc-oauth2-state", &data);
        store.put(cred, PutMode::CreateOnly).await.unwrap();

        let raw = inner.get("enc-oauth2-state").await.unwrap();
        let lossy = String::from_utf8_lossy(&raw.data);
        let stored_len = raw.data.len();
        assert!(
            !lossy.contains(PLAINTEXT_ACCESS) && !lossy.contains(PLAINTEXT_REFRESH),
            "inner row must not contain discoverable credential secrets (stored bytes: {stored_len})"
        );
        Ok(())
    }

    #[tokio::test]
    async fn passthrough_operations() -> Result<(), StoreError> {
        let store = make_store(default_provider());

        let cred = make_credential("enc-3", b"data");
        store.put(cred, PutMode::CreateOnly).await.unwrap();

        assert!(store.exists("enc-3").await.unwrap());
        assert!(!store.exists("missing").await.unwrap());

        let ids = store.list(None).await.unwrap();
        assert_eq!(ids, vec!["enc-3"]);

        store.delete("enc-3").await.unwrap();
        assert!(!store.exists("enc-3").await.unwrap());
        Ok(())
    }

    #[tokio::test]
    async fn aad_prevents_record_swapping() -> Result<(), StoreError> {
        let inner = InMemoryCredentialStore::new();
        let store = EncryptionLayer::new(inner.clone(), default_provider());

        let cred = make_credential("cred-1", b"secret-data");
        store.put(cred, PutMode::CreateOnly).await.unwrap();

        // Read raw encrypted data from inner store and insert it under a different ID.
        let raw = inner.get("cred-1").await.unwrap();
        let swapped = StoredCredential {
            id: "cred-2".into(),
            ..raw
        };
        inner.put(swapped, PutMode::CreateOnly).await.unwrap();

        // Reading cred-2 through the encryption layer must fail because
        // the AAD (credential ID) doesn't match.
        let err = store.get("cred-2").await.unwrap_err();
        assert!(matches!(err, StoreError::Backend(_)));
        Ok(())
    }

    /// SEC-11: data encrypted with an empty AAD must be rejected at the layer
    /// boundary even when the `key_id` matches.
    #[tokio::test]
    async fn rejects_data_without_aad() -> Result<(), StoreError> {
        let inner = InMemoryCredentialStore::new();
        let key = EncryptionKey::from_bytes([0x42; 32]);

        // Construct a legacy-shaped envelope: encrypted with the *current*
        // provider's key_id ("default") but with an EMPTY AAD.
        let envelope = encrypt_with_key_id(&key, "default", b"legacy-secret", b"")
            .expect("encrypt with empty AAD should succeed at the crypto layer");
        let encrypted_bytes = serde_json::to_vec(&envelope).unwrap();

        let cred = StoredCredential {
            id: "legacy-1".into(),
            name: None,
            credential_key: "test_credential".into(),
            data: encrypted_bytes,
            state_kind: "test".into(),
            state_version: 1,
            version: 0,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            expires_at: None,
            reauth_required: false,
            metadata: Default::default(),
        };
        inner.put(cred, PutMode::CreateOnly).await.unwrap();

        // Reading through the encryption layer must fail with an AAD
        // mismatch: the envelope was sealed with empty AAD but the layer
        // unconditionally binds `credential_id` as AAD on decrypt.
        let store = EncryptionLayer::new(inner, default_provider());
        let err = store.get("legacy-1").await.unwrap_err();
        assert!(matches!(err, StoreError::Backend(_)));
        Ok(())
    }

    #[tokio::test]
    async fn wrong_key_fails_decryption() -> Result<(), StoreError> {
        let inner = InMemoryCredentialStore::new();
        let provider1 = static_provider_with_version([0x01; 32], "default");
        let provider2 = static_provider_with_version([0x02; 32], "default");

        let store1 = EncryptionLayer::new(inner.clone(), provider1);
        let cred = make_credential("enc-4", b"secret");
        store1.put(cred, PutMode::CreateOnly).await.unwrap();

        let store2 = EncryptionLayer::new(inner, provider2);
        let err = store2.get("enc-4").await.unwrap_err();
        assert!(matches!(err, StoreError::Backend(_)));
        Ok(())
    }

    // =========================================================================
    // Multi-key / key rotation tests
    // =========================================================================

    #[tokio::test]
    async fn single_key_mode_stores_key_id() -> Result<(), StoreError> {
        let inner = InMemoryCredentialStore::new();
        let store = EncryptionLayer::new(inner.clone(), default_provider());

        let cred = make_credential("key-id-check", b"secret");
        store.put(cred, PutMode::CreateOnly).await.unwrap();

        // Inspect the raw bytes stored — should contain "default" as key_id.
        let raw = inner.get("key-id-check").await.unwrap();
        let envelope: EncryptedData = serde_json::from_slice(&raw.data).unwrap();
        assert_eq!(envelope.key_id, "default");
        Ok(())
    }

    #[tokio::test]
    async fn multi_key_round_trip() -> Result<(), StoreError> {
        let key1 = Arc::new(EncryptionKey::from_bytes([0x01; 32]));
        let provider = Arc::new(StaticKeyProvider::with_version(
            Arc::new(EncryptionKey::from_bytes([0x02; 32])),
            "key-2",
        )) as Arc<dyn KeyProvider>;
        let store = EncryptionLayer::with_legacy_keys(
            InMemoryCredentialStore::new(),
            provider,
            vec![("key-1".to_string(), key1)],
        );

        let cred = make_credential("mk-1", b"multi-key-secret");
        store.put(cred, PutMode::CreateOnly).await.unwrap();

        let fetched = store.get("mk-1").await.unwrap();
        assert_eq!(fetched.data, b"multi-key-secret");
        Ok(())
    }

    #[tokio::test]
    async fn decrypt_with_old_key_succeeds() -> Result<(), StoreError> {
        let inner = InMemoryCredentialStore::new();
        let key1_bytes = [0x01; 32];
        let key2_bytes = [0x02; 32];

        // Write with old key (key-1 is current).
        let store_old = EncryptionLayer::new(
            inner.clone(),
            static_provider_with_version(key1_bytes, "key-1"),
        );
        let cred = make_credential("rotate-1", b"old-key-data");
        store_old.put(cred, PutMode::CreateOnly).await.unwrap();

        // Now rotate: key-2 is current, key-1 available as a legacy decrypt-only key.
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
        Ok(())
    }

    /// Regression for GitHub issue #282: `get()` on a record that triggers
    /// lazy re-encryption used to return the `StoredCredential` with the
    /// pre-CAS `version`. Downstream callers then hit a phantom
    /// [`StoreError::VersionConflict`] on their own CAS update against the
    /// row we just bumped. The returned struct must carry the post-rotation
    /// `version` (and `updated_at`) so downstream CAS targets the fresh row.
    #[tokio::test]
    async fn lazy_reencryption_returns_post_rotation_version() -> Result<(), StoreError> {
        let inner = InMemoryCredentialStore::new();
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
        Ok(())
    }

    #[tokio::test]
    async fn lazy_reencryption_on_read_when_key_id_differs() -> Result<(), StoreError> {
        let inner = InMemoryCredentialStore::new();
        let key1_bytes = [0x01; 32];
        let key2_bytes = [0x02; 32];

        // Write with key-1.
        let store_old = EncryptionLayer::new(
            inner.clone(),
            static_provider_with_version(key1_bytes, "key-1"),
        );
        let cred = make_credential("lazy-1", b"will-be-rotated");
        store_old.put(cred, PutMode::CreateOnly).await.unwrap();

        // Read through new layer — triggers lazy rotation.
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

        // Verify the data was re-encrypted with key-2 in the backing store.
        let raw = inner.get("lazy-1").await.unwrap();
        let envelope: EncryptedData = serde_json::from_slice(&raw.data).unwrap();
        assert_eq!(envelope.key_id, "key-2");
        Ok(())
    }

    /// Regression for GitHub issue #281: `new()` no longer aliases the key
    /// under `""`, so legacy envelopes with `key_id: ""` cannot silently
    /// decrypt with the current key. Operators who still hold such records
    /// must register the alias explicitly via `with_legacy_keys`.
    #[tokio::test]
    async fn new_does_not_silently_decrypt_empty_key_id_envelopes() -> Result<(), StoreError> {
        let inner = InMemoryCredentialStore::new();
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
            name: None,
            credential_key: "test_credential".into(),
            data: envelope_bytes,
            state_kind: "test".into(),
            state_version: 1,
            version: 0,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            expires_at: None,
            reauth_required: false,
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
        let store_with_legacy = make_store_with_legacy(
            static_provider_with_version(key_bytes, "default"),
            vec![(String::new(), Arc::clone(&key))],
        );
        // Re-insert the record into the fresh inner store.
        let mut legacy_envelope2 =
            encrypt_with_key_id(&key, "default", plaintext, b"legacy-1").unwrap();
        legacy_envelope2.key_id = String::new();
        let envelope_bytes2 = serde_json::to_vec(&legacy_envelope2).unwrap();
        let cred2 = StoredCredential {
            id: "legacy-1".into(),
            name: None,
            credential_key: "test_credential".into(),
            data: envelope_bytes2,
            state_kind: "test".into(),
            state_version: 1,
            version: 0,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            expires_at: None,
            reauth_required: false,
            metadata: Default::default(),
        };
        store_with_legacy
            .inner
            .put(cred2, PutMode::CreateOnly)
            .await
            .unwrap();
        let fetched = store_with_legacy.get("legacy-1").await.unwrap();
        assert_eq!(fetched.data, plaintext);
        Ok(())
    }
}
