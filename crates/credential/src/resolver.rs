//! Runtime credential resolution.
//!
//! Loads stored credentials, deserializes state, and projects to
//! [`AuthScheme`](nebula_core::AuthScheme) via the
//! [`Credential::project()`](crate::credential_v2::Credential::project) pipeline.

use std::sync::Arc;

use crate::credential_state::CredentialStateV2;
use crate::credential_v2::Credential;
use crate::handle_v2::CredentialHandle;
use crate::store_v2::{CredentialStoreV2, StoreError};

/// Resolves credentials from storage into typed [`CredentialHandle`]s.
///
/// The resolver loads a [`StoredCredential`](crate::store_v2::StoredCredential),
/// verifies the `state_kind` matches the expected credential type,
/// deserializes the state, and projects it to the [`AuthScheme`](nebula_core::AuthScheme).
///
/// # Examples
///
/// ```ignore
/// use nebula_credential::{CredentialResolver, InMemoryStore};
/// use nebula_credential::credentials::ApiKeyCredential;
///
/// let store = Arc::new(InMemoryStore::new());
/// let resolver = CredentialResolver::new(store);
///
/// let handle = resolver.resolve::<ApiKeyCredential>("my-api-key").await?;
/// let token = handle.snapshot();
/// ```
pub struct CredentialResolver<S: CredentialStoreV2> {
    store: Arc<S>,
}

impl<S: CredentialStoreV2> CredentialResolver<S> {
    /// Creates a new resolver backed by the given store.
    pub fn new(store: Arc<S>) -> Self {
        Self { store }
    }

    /// Resolves a credential by ID into a typed handle.
    ///
    /// Loads the stored credential, deserializes the state, and
    /// projects it to the [`AuthScheme`](nebula_core::AuthScheme) via
    /// [`Credential::project()`](crate::credential_v2::Credential::project).
    ///
    /// # Errors
    ///
    /// Returns [`ResolveError::Store`] if the credential is not found
    /// or the store is unavailable.
    ///
    /// Returns [`ResolveError::KindMismatch`] if the stored `state_kind`
    /// does not match `C::State::KIND`.
    ///
    /// Returns [`ResolveError::Deserialize`] if the stored bytes cannot
    /// be deserialized into `C::State`.
    pub async fn resolve<C>(
        &self,
        credential_id: &str,
    ) -> Result<CredentialHandle<C::Scheme>, ResolveError>
    where
        C: Credential,
    {
        let stored = self
            .store
            .get(credential_id)
            .await
            .map_err(ResolveError::Store)?;

        // Verify state kind matches
        let expected_kind = <C::State as CredentialStateV2>::KIND;
        if stored.state_kind != expected_kind {
            return Err(ResolveError::KindMismatch {
                credential_id: credential_id.to_string(),
                expected: expected_kind.to_string(),
                actual: stored.state_kind,
            });
        }

        // Deserialize state
        let state: C::State =
            serde_json::from_slice(&stored.data).map_err(|e| ResolveError::Deserialize {
                credential_id: credential_id.to_string(),
                reason: e.to_string(),
            })?;

        // Project to scheme
        let scheme = C::project(&state);

        Ok(CredentialHandle::new(scheme, credential_id))
    }
}

/// Error during credential resolution.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ResolveError {
    /// Credential not found or store error.
    #[error("store error: {0}")]
    Store(#[from] StoreError),

    /// State kind mismatch between stored and expected.
    #[error("credential {credential_id}: expected kind {expected}, found {actual}")]
    KindMismatch {
        /// The credential ID.
        credential_id: String,
        /// The expected `state_kind`.
        expected: String,
        /// The actual `state_kind` found in storage.
        actual: String,
    },

    /// Failed to deserialize stored state.
    #[error("credential {credential_id}: deserialize failed: {reason}")]
    Deserialize {
        /// The credential ID.
        credential_id: String,
        /// The deserialization error message.
        reason: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credentials::ApiKeyCredential;
    use crate::store_memory::InMemoryStore;
    use crate::store_v2::{PutMode, StoredCredential};

    #[tokio::test]
    async fn resolve_api_key_credential() {
        let store = Arc::new(InMemoryStore::new());

        // Construct raw JSON directly because SecretString serializes
        // as "[REDACTED]" — the real store holds encrypted raw values.
        let data = br#"{"token":"test-api-key"}"#.to_vec();

        let cred = StoredCredential {
            id: "my-api-key".into(),
            data,
            state_kind: "bearer".into(),
            state_version: 1,
            version: 0,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            expires_at: None,
            metadata: Default::default(),
        };
        store.put(cred, PutMode::CreateOnly).await.unwrap();

        let resolver = CredentialResolver::new(store);
        let handle = resolver
            .resolve::<ApiKeyCredential>("my-api-key")
            .await
            .unwrap();

        let snapshot = handle.snapshot();
        let value = snapshot.expose().expose_secret(|s| s.to_owned());
        assert_eq!(value, "test-api-key");
        assert_eq!(handle.credential_id(), "my-api-key");
    }

    #[tokio::test]
    async fn resolve_not_found() {
        let store = Arc::new(InMemoryStore::new());
        let resolver = CredentialResolver::new(store);

        let result = resolver.resolve::<ApiKeyCredential>("nonexistent").await;
        assert!(matches!(
            result,
            Err(ResolveError::Store(StoreError::NotFound { .. }))
        ));
    }

    #[tokio::test]
    async fn resolve_kind_mismatch() {
        let store = Arc::new(InMemoryStore::new());

        let cred = StoredCredential {
            id: "wrong-kind".into(),
            data: b"{}".to_vec(),
            state_kind: "database_auth".into(),
            state_version: 1,
            version: 0,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            expires_at: None,
            metadata: Default::default(),
        };
        store.put(cred, PutMode::CreateOnly).await.unwrap();

        let resolver = CredentialResolver::new(store);
        let result = resolver.resolve::<ApiKeyCredential>("wrong-kind").await;
        assert!(matches!(result, Err(ResolveError::KindMismatch { .. })));
    }

    #[tokio::test]
    async fn resolve_deserialize_failure() {
        let store = Arc::new(InMemoryStore::new());

        let cred = StoredCredential {
            id: "bad-data".into(),
            data: b"not-valid-json".to_vec(),
            state_kind: "bearer".into(),
            state_version: 1,
            version: 0,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            expires_at: None,
            metadata: Default::default(),
        };
        store.put(cred, PutMode::CreateOnly).await.unwrap();

        let resolver = CredentialResolver::new(store);
        let result = resolver.resolve::<ApiKeyCredential>("bad-data").await;
        assert!(matches!(result, Err(ResolveError::Deserialize { .. })));
    }
}
