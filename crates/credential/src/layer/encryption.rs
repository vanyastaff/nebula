//! Encryption layer -- encrypts data before storage, decrypts after retrieval.
//!
//! Wraps any [`CredentialStore`] implementation and applies AES-256-GCM
//! encryption to the [`StoredCredential::data`] field. The credential ID is
//! bound as Additional Authenticated Data (AAD), preventing record-swapping
//! attacks where encrypted data from one credential is copied to another.
//!
//! For backward compatibility, decryption falls back to no-AAD mode if
//! AAD-based decryption fails, allowing legacy data to be read transparently.
//! New writes always use AAD binding.

use std::sync::Arc;

use crate::credential_store::{CredentialStore, PutMode, StoreError, StoredCredential};
use crate::utils::crypto::{self, EncryptionKey};

/// Wraps a store with AES-256-GCM encryption on the `data` field.
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_credential::{EncryptionLayer, InMemoryStore, EncryptionKey};
/// use std::sync::Arc;
///
/// let key = Arc::new(EncryptionKey::from_bytes([0x42; 32]));
/// let store = EncryptionLayer::new(InMemoryStore::new(), key);
/// ```
pub struct EncryptionLayer<S> {
    inner: S,
    key: Arc<EncryptionKey>,
}

impl<S> EncryptionLayer<S> {
    /// Create a new encryption layer wrapping the given store.
    pub fn new(inner: S, key: Arc<EncryptionKey>) -> Self {
        Self { inner, key }
    }
}

impl<S: CredentialStore> CredentialStore for EncryptionLayer<S> {
    async fn get(&self, id: &str) -> Result<StoredCredential, StoreError> {
        let mut credential = self.inner.get(id).await?;
        credential.data = decrypt_data(&self.key, &credential.data, id)?;
        Ok(credential)
    }

    async fn put(
        &self,
        mut credential: StoredCredential,
        mode: PutMode,
    ) -> Result<StoredCredential, StoreError> {
        let id = credential.id.clone();
        credential.data = encrypt_data(&self.key, &credential.data, &id)?;
        let mut stored = self.inner.put(credential, mode).await?;
        // Return with plaintext data so callers see what they stored
        stored.data = decrypt_data(&self.key, &stored.data, &id)?;
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

/// Encrypt plaintext data with the credential ID as AAD, serializing the
/// [`EncryptedData`](crypto::EncryptedData) envelope to bytes.
///
/// The credential ID is bound as Additional Authenticated Data (AAD),
/// preventing record-swapping attacks where encrypted data from one
/// credential is copied to another.
fn encrypt_data(key: &EncryptionKey, plaintext: &[u8], id: &str) -> Result<Vec<u8>, StoreError> {
    let encrypted = crypto::encrypt_with_aad(key, plaintext, id.as_bytes())
        .map_err(|e| StoreError::Backend(Box::new(e)))?;
    serde_json::to_vec(&encrypted).map_err(|e| StoreError::Backend(Box::new(e)))
}

/// Deserialize an [`EncryptedData`](crypto::EncryptedData) envelope and decrypt.
///
/// Tries decryption with AAD (current format) first, then falls back to
/// no-AAD decryption for backward compatibility with data encrypted before
/// AAD binding was introduced.
fn decrypt_data(key: &EncryptionKey, ciphertext: &[u8], id: &str) -> Result<Vec<u8>, StoreError> {
    let encrypted: crypto::EncryptedData =
        serde_json::from_slice(ciphertext).map_err(|e| StoreError::Backend(Box::new(e)))?;

    // Try with AAD first (current format)
    if let Ok(data) = crypto::decrypt_with_aad(key, &encrypted, id.as_bytes()) {
        return Ok(data);
    }

    // Fall back to no-AAD decryption (legacy format)
    crypto::decrypt(key, &encrypted).map_err(|e| StoreError::Backend(Box::new(e)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credential_store::PutMode;
    use crate::store_memory::InMemoryStore;

    fn test_key() -> Arc<EncryptionKey> {
        Arc::new(EncryptionKey::from_bytes([0x42; 32]))
    }

    fn make_credential(id: &str, data: &[u8]) -> StoredCredential {
        StoredCredential {
            id: id.into(),
            data: data.to_vec(),
            state_kind: "test".into(),
            state_version: 1,
            version: 0,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            expires_at: None,
            metadata: Default::default(),
        }
    }

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
    async fn legacy_data_without_aad_still_readable() {
        let inner = InMemoryStore::new();
        let key = test_key();

        // Simulate legacy write: encrypt without AAD and store directly
        let plaintext = b"legacy-secret";
        let encrypted =
            crate::utils::crypto::encrypt(&key, plaintext).unwrap();
        let encrypted_bytes = serde_json::to_vec(&encrypted).unwrap();

        let cred = StoredCredential {
            id: "legacy-1".into(),
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

        // Read through encryption layer -- should fall back to no-AAD decrypt
        let store = EncryptionLayer::new(inner, key);
        let fetched = store.get("legacy-1").await.unwrap();
        assert_eq!(fetched.data, b"legacy-secret");
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
}
