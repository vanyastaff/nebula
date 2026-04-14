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
//! # Key rotation
//!
//! Multiple keys can be registered via [`EncryptionLayer::with_keys`]. On every
//! read the layer inspects `EncryptedData::key_id`:
//!
//! - If `key_id` matches `current_key_id`, decrypt normally.
//! - If `key_id` differs from `current_key_id`, decrypt with the old key and **re-encrypt with the
//!   current key** before returning — lazy rotation.

use std::{collections::HashMap, sync::Arc};

use crate::{
    crypto::{self, EncryptionKey},
    store::{CredentialStore, PutMode, StoreError, StoredCredential},
};

/// Wraps a store with AES-256-GCM encryption on the `data` field.
///
/// Supports multi-key operation for transparent key rotation: data encrypted
/// with an old key is automatically re-encrypted with the current key on read.
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_credential::{EncryptionLayer, InMemoryStore, EncryptionKey};
/// use std::sync::Arc;
///
/// // Single-key mode
/// let key = Arc::new(EncryptionKey::from_bytes([0x42; 32]));
/// let store = EncryptionLayer::new(InMemoryStore::new(), key);
///
/// // Multi-key mode (key rotation)
/// let old_key = Arc::new(EncryptionKey::from_bytes([0x01; 32]));
/// let new_key = Arc::new(EncryptionKey::from_bytes([0x02; 32]));
/// let store = EncryptionLayer::with_keys(
///     InMemoryStore::new(),
///     "new-key".to_string(),
///     vec![
///         ("old-key".to_string(), old_key),
///         ("new-key".to_string(), new_key),
///     ],
/// );
/// ```
pub struct EncryptionLayer<S> {
    inner: S,
    current_key_id: String,
    keys: HashMap<String, Arc<EncryptionKey>>,
}

impl<S> EncryptionLayer<S> {
    /// Create a new single-key encryption layer.
    ///
    /// The key is registered as `"default"` and used for all new writes.
    /// An empty-string alias is also registered for migration compatibility
    /// with pre-rotation data.
    pub fn new(inner: S, key: Arc<EncryptionKey>) -> Self {
        let mut keys = HashMap::new();
        keys.insert("default".into(), key.clone());
        keys.insert(String::new(), key); // alias for pre-rotation data
        Self {
            inner,
            current_key_id: "default".into(),
            keys,
        }
    }

    /// Create an encryption layer with multiple keys for rotation support.
    ///
    /// `current_key_id` must be present in `keys`; it is used for all new
    /// writes. Other keys are used only for decrypting legacy data and are
    /// replaced on read (lazy rotation).
    ///
    /// # Panics
    ///
    /// Panics if `current_key_id` is not present in `keys`.
    pub fn with_keys(
        inner: S,
        current_key_id: impl Into<String>,
        keys: Vec<(String, Arc<EncryptionKey>)>,
    ) -> Self {
        let current_key_id = current_key_id.into();
        let keys: HashMap<String, Arc<EncryptionKey>> = keys.into_iter().collect();
        assert!(
            keys.contains_key(&current_key_id),
            "current_key_id '{current_key_id}' must be present in the keys map"
        );
        Self {
            inner,
            current_key_id,
            keys,
        }
    }

    fn current_key(&self) -> Result<&EncryptionKey, StoreError> {
        self.keys
            .get(&self.current_key_id)
            .map(|k| k.as_ref())
            .ok_or_else(|| {
                StoreError::Backend(
                    format!("current encryption key '{}' not found", self.current_key_id).into(),
                )
            })
    }

    fn key_for_id(&self, key_id: &str) -> Result<&EncryptionKey, StoreError> {
        self.keys.get(key_id).map(|k| k.as_ref()).ok_or_else(|| {
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
                Ok(_) | Err(StoreError::VersionConflict { .. }) => {},
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
        let encrypted =
            crypto::encrypt_with_key_id(key, &self.current_key_id, plaintext, id.as_bytes())
                .map_err(|e| StoreError::Backend(Box::new(e)))?;
        serde_json::to_vec(&encrypted).map_err(|e| StoreError::Backend(Box::new(e)))
    }

    /// Decrypt `ciphertext`, returning `(plaintext, Some(re_encrypted_bytes))` when
    /// the data was stored under an old key and must be lazily rotated, or
    /// `(plaintext, None)` when no rotation is needed.
    ///
    /// AAD (credential ID) is always enforced — data without AAD or with a
    /// mismatched ID is rejected.
    #[allow(clippy::type_complexity)]
    // Reason: The return type is (plaintext, optional re-encryption bytes) — a type alias
    // here would obscure the meaning without reducing complexity. The function is private.
    fn decrypt_possibly_rotating(
        &self,
        ciphertext: &[u8],
        id: &str,
    ) -> Result<(zeroize::Zeroizing<Vec<u8>>, Option<Vec<u8>>), StoreError> {
        let encrypted: crypto::EncryptedData =
            serde_json::from_slice(ciphertext).map_err(|e| StoreError::Backend(Box::new(e)))?;

        // Data encrypted with the current key — normal path.
        if encrypted.key_id == self.current_key_id {
            let key = self.current_key()?;
            let plaintext = crypto::decrypt_with_aad(key, &encrypted, id.as_bytes())
                .map_err(|e| StoreError::Backend(Box::new(e)))?;
            return Ok((plaintext, None));
        }

        // Data encrypted with an older key — decrypt with old key, re-encrypt.
        let old_key = self.key_for_id(&encrypted.key_id)?;
        let plaintext = crypto::decrypt_with_aad(old_key, &encrypted, id.as_bytes())
            .map_err(|e| StoreError::Backend(Box::new(e)))?;
        let re_encrypted = self.encrypt_data(&plaintext, id)?;
        Ok((plaintext, Some(re_encrypted)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{store::PutMode, store_memory::InMemoryStore};

    fn test_key() -> Arc<EncryptionKey> {
        Arc::new(EncryptionKey::from_bytes([0x42; 32]))
    }

    use crate::store::test_helpers::make_credential;

    // =========================================================================
    // Existing single-key tests (preserved)
    // =========================================================================

    #[tokio::test]
    async fn round_trip_encrypts_and_decrypts() {
        let store = EncryptionLayer::new(InMemoryStore::new(), test_key());
        let cred = make_credential("enc-1", b"super-secret");

        let stored = store.put(cred, PutMode::CreateOnly).await.unwrap();
        assert_eq!(stored.data, b"super-secret");

        let fetched = store.get("enc-1").await.unwrap();
        assert_eq!(fetched.data, b"super-secret");
    }

    #[tokio::test]
    async fn data_is_encrypted_at_rest() {
        let inner = InMemoryStore::new();
        let store = EncryptionLayer::new(inner.clone(), test_key());

        let cred = make_credential("enc-2", b"plaintext-secret");
        store.put(cred, PutMode::CreateOnly).await.unwrap();

        // Read directly from inner store — data should NOT be plaintext
        let raw = inner.get("enc-2").await.unwrap();
        assert_ne!(raw.data, b"plaintext-secret");
    }

    #[tokio::test]
    async fn passthrough_operations() {
        let store = EncryptionLayer::new(InMemoryStore::new(), test_key());

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
        let key = test_key();
        let store = EncryptionLayer::new(inner.clone(), key);

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
        let key = test_key();

        // Simulate legacy write: encrypt without AAD and store directly
        let plaintext = b"legacy-secret";
        let encrypted = crate::crypto::encrypt(&key, plaintext).unwrap();
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
        let store = EncryptionLayer::new(inner, key);
        let err = store.get("legacy-1").await.unwrap_err();
        assert!(matches!(err, StoreError::Backend(_)));
    }

    #[tokio::test]
    async fn wrong_key_fails_decryption() {
        let inner = InMemoryStore::new();
        let key1 = Arc::new(EncryptionKey::from_bytes([0x01; 32]));
        let key2 = Arc::new(EncryptionKey::from_bytes([0x02; 32]));

        let store1 = EncryptionLayer::new(inner.clone(), key1);
        let cred = make_credential("enc-4", b"secret");
        store1.put(cred, PutMode::CreateOnly).await.unwrap();

        let store2 = EncryptionLayer::new(inner, key2);
        let err = store2.get("enc-4").await.unwrap_err();
        assert!(matches!(err, StoreError::Backend(_)));
    }

    // =========================================================================
    // New multi-key / key rotation tests
    // =========================================================================

    #[tokio::test]
    async fn single_key_mode_stores_key_id() {
        let inner = InMemoryStore::new();
        let store = EncryptionLayer::new(inner.clone(), test_key());

        let cred = make_credential("key-id-check", b"secret");
        store.put(cred, PutMode::CreateOnly).await.unwrap();

        // Inspect the raw bytes stored — should contain "default" as key_id
        let raw = inner.get("key-id-check").await.unwrap();
        let envelope: crate::crypto::EncryptedData = serde_json::from_slice(&raw.data).unwrap();
        assert_eq!(envelope.key_id, "default");
    }

    #[tokio::test]
    async fn multi_key_round_trip() {
        let key1 = Arc::new(EncryptionKey::from_bytes([0x01; 32]));
        let key2 = Arc::new(EncryptionKey::from_bytes([0x02; 32]));
        let store = EncryptionLayer::with_keys(
            InMemoryStore::new(),
            "key-2".to_string(),
            vec![("key-1".to_string(), key1), ("key-2".to_string(), key2)],
        );

        let cred = make_credential("mk-1", b"multi-key-secret");
        store.put(cred, PutMode::CreateOnly).await.unwrap();

        let fetched = store.get("mk-1").await.unwrap();
        assert_eq!(fetched.data, b"multi-key-secret");
    }

    #[tokio::test]
    async fn decrypt_with_old_key_succeeds() {
        let inner = InMemoryStore::new();
        let key1 = Arc::new(EncryptionKey::from_bytes([0x01; 32]));
        let key2 = Arc::new(EncryptionKey::from_bytes([0x02; 32]));

        // Write with old key (key-1 is current)
        let store_old = EncryptionLayer::with_keys(
            inner.clone(),
            "key-1".to_string(),
            vec![("key-1".to_string(), key1.clone())],
        );
        let cred = make_credential("rotate-1", b"old-key-data");
        store_old.put(cred, PutMode::CreateOnly).await.unwrap();

        // Now rotate: key-2 is current, key-1 still available for decryption
        let store_new = EncryptionLayer::with_keys(
            inner.clone(),
            "key-2".to_string(),
            vec![("key-1".to_string(), key1), ("key-2".to_string(), key2)],
        );

        let fetched = store_new.get("rotate-1").await.unwrap();
        assert_eq!(fetched.data, b"old-key-data");
    }

    #[tokio::test]
    async fn lazy_reencryption_on_read_when_key_id_differs() {
        let inner = InMemoryStore::new();
        let key1 = Arc::new(EncryptionKey::from_bytes([0x01; 32]));
        let key2 = Arc::new(EncryptionKey::from_bytes([0x02; 32]));

        // Write with key-1
        let store_old = EncryptionLayer::with_keys(
            inner.clone(),
            "key-1".to_string(),
            vec![("key-1".to_string(), key1.clone())],
        );
        let cred = make_credential("lazy-1", b"will-be-rotated");
        store_old.put(cred, PutMode::CreateOnly).await.unwrap();

        // Read through new layer — triggers lazy rotation
        let store_new = EncryptionLayer::with_keys(
            inner.clone(),
            "key-2".to_string(),
            vec![("key-1".to_string(), key1), ("key-2".to_string(), key2)],
        );
        let fetched = store_new.get("lazy-1").await.unwrap();
        assert_eq!(fetched.data, b"will-be-rotated");

        // Verify the data was re-encrypted with key-2 in the backing store
        let raw = inner.get("lazy-1").await.unwrap();
        let envelope: crate::crypto::EncryptedData = serde_json::from_slice(&raw.data).unwrap();
        assert_eq!(envelope.key_id, "key-2");
    }
}
