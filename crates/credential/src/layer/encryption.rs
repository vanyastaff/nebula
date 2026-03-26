//! Encryption layer -- encrypts data before storage, decrypts after retrieval.
//!
//! Wraps any [`CredentialStoreV2`] implementation and applies AES-256-GCM
//! encryption to the [`StoredCredential::data`] field using the existing
//! [`encrypt`](crate::utils::crypto::encrypt) / [`decrypt`](crate::utils::crypto::decrypt)
//! functions. Non-data fields (metadata, version, etc.) pass through unchanged.

use std::sync::Arc;

use crate::store_v2::{CredentialStoreV2, PutMode, StoreError, StoredCredential};
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

impl<S: CredentialStoreV2> CredentialStoreV2 for EncryptionLayer<S> {
    async fn get(&self, id: &str) -> Result<StoredCredential, StoreError> {
        let mut credential = self.inner.get(id).await?;
        credential.data = decrypt_data(&self.key, &credential.data)?;
        Ok(credential)
    }

    async fn put(
        &self,
        mut credential: StoredCredential,
        mode: PutMode,
    ) -> Result<StoredCredential, StoreError> {
        credential.data = encrypt_data(&self.key, &credential.data)?;
        let mut stored = self.inner.put(credential, mode).await?;
        // Return with plaintext data so callers see what they stored
        stored.data = decrypt_data(&self.key, &stored.data)?;
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

/// Encrypt plaintext data, serializing the [`EncryptedData`] envelope to bytes.
fn encrypt_data(key: &EncryptionKey, plaintext: &[u8]) -> Result<Vec<u8>, StoreError> {
    let encrypted =
        crypto::encrypt(key, plaintext).map_err(|e| StoreError::Backend(Box::new(e)))?;
    serde_json::to_vec(&encrypted).map_err(|e| StoreError::Backend(Box::new(e)))
}

/// Deserialize an [`EncryptedData`] envelope and decrypt.
fn decrypt_data(key: &EncryptionKey, ciphertext: &[u8]) -> Result<Vec<u8>, StoreError> {
    let encrypted: crypto::EncryptedData =
        serde_json::from_slice(ciphertext).map_err(|e| StoreError::Backend(Box::new(e)))?;
    crypto::decrypt(key, &encrypted).map_err(|e| StoreError::Backend(Box::new(e)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store_memory::InMemoryStore;
    use crate::store_v2::PutMode;

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
