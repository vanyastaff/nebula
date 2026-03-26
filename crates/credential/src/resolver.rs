//! Runtime credential resolution.
//!
//! Loads stored credentials, deserializes state, and projects to
//! [`AuthScheme`](nebula_core::AuthScheme) via the
//! [`Credential::project()`](crate::credential_trait::Credential::project) pipeline.
//!
//! For refreshable credentials, use [`CredentialResolver::resolve_with_refresh()`]
//! which coordinates refresh via [`RefreshCoordinator`] to prevent
//! thundering herd.

use std::sync::Arc;

use crate::core::CredentialContext;
use crate::credential_handle::CredentialHandle;
use crate::credential_state::CredentialStateV2;
use crate::credential_store::{CredentialStore, PutMode, StoreError, StoredCredential};
use crate::credential_trait::Credential;
use crate::refresh::{RefreshAttempt, RefreshCoordinator};
use crate::resolve::RefreshOutcome;

/// Resolves credentials from storage into typed [`CredentialHandle`]s.
///
/// The resolver loads a [`StoredCredential`](crate::credential_store::StoredCredential),
/// verifies the `state_kind` matches the expected credential type,
/// deserializes the state, and projects it to the [`AuthScheme`](nebula_core::AuthScheme).
///
/// For refreshable credentials, [`resolve_with_refresh()`](Self::resolve_with_refresh)
/// coordinates concurrent refresh attempts through the embedded
/// [`RefreshCoordinator`].
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
pub struct CredentialResolver<S: CredentialStore> {
    store: Arc<S>,
    refresh_coordinator: RefreshCoordinator,
}

impl<S: CredentialStore> CredentialResolver<S> {
    /// Creates a new resolver backed by the given store.
    pub fn new(store: Arc<S>) -> Self {
        Self {
            store,
            refresh_coordinator: RefreshCoordinator::new(),
        }
    }

    /// Resolves a credential by ID into a typed handle.
    ///
    /// Loads the stored credential, deserializes the state, and
    /// projects it to the [`AuthScheme`](nebula_core::AuthScheme) via
    /// [`Credential::project()`](crate::credential_trait::Credential::project).
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
        let stored = self.load_and_verify::<C>(credential_id).await?;
        let state: C::State = self.deserialize::<C>(credential_id, &stored)?;
        let scheme = C::project(&state);
        Ok(CredentialHandle::new(scheme, credential_id))
    }

    /// Resolves a credential, refreshing it if expired.
    ///
    /// If the credential's state reports an expiration time
    /// ([`CredentialStateV2::expires_at()`]) in the past, the resolver
    /// coordinates a refresh through the embedded
    /// [`RefreshCoordinator`]. Only one caller performs the actual
    /// refresh; others wait and then re-read the updated state from
    /// the store.
    ///
    /// For non-expiring credentials this behaves identically to
    /// [`resolve()`](Self::resolve).
    ///
    /// # Errors
    ///
    /// Returns all errors from [`resolve()`](Self::resolve), plus:
    ///
    /// - [`ResolveError::Refresh`] if `Credential::refresh()` fails.
    /// - [`ResolveError::Store`] if the CAS write after refresh fails.
    /// - [`ResolveError::ReauthRequired`] if the refresh indicates
    ///   the credential needs full re-authentication.
    pub async fn resolve_with_refresh<C>(
        &self,
        credential_id: &str,
        ctx: &CredentialContext,
    ) -> Result<CredentialHandle<C::Scheme>, ResolveError>
    where
        C: Credential,
    {
        let stored = self.load_and_verify::<C>(credential_id).await?;
        let state: C::State = self.deserialize::<C>(credential_id, &stored)?;

        // Check expiration
        let needs_refresh = state
            .expires_at()
            .is_some_and(|exp| exp <= chrono::Utc::now());

        if !needs_refresh {
            let scheme = C::project(&state);
            return Ok(CredentialHandle::new(scheme, credential_id));
        }

        // Coordinate refresh -- only one caller does the work
        match self.refresh_coordinator.try_refresh(credential_id).await {
            RefreshAttempt::Winner => {
                let result = self
                    .perform_refresh::<C>(credential_id, state, stored, ctx)
                    .await;
                // Always complete to wake waiters, even on error
                self.refresh_coordinator.complete(credential_id).await;
                result
            }
            RefreshAttempt::Waiter(notify) => {
                notify.notified().await;
                // Re-read from store -- the winner updated it
                self.resolve::<C>(credential_id).await
            }
        }
    }

    /// Returns a reference to the embedded [`RefreshCoordinator`].
    ///
    /// Primarily useful for testing and diagnostics.
    pub fn refresh_coordinator(&self) -> &RefreshCoordinator {
        &self.refresh_coordinator
    }

    /// Loads and verifies a stored credential's `state_kind`.
    async fn load_and_verify<C>(
        &self,
        credential_id: &str,
    ) -> Result<StoredCredential, ResolveError>
    where
        C: Credential,
    {
        let stored = self
            .store
            .get(credential_id)
            .await
            .map_err(ResolveError::Store)?;

        let expected_kind = <C::State as CredentialStateV2>::KIND;
        if stored.state_kind != expected_kind {
            return Err(ResolveError::KindMismatch {
                credential_id: credential_id.to_string(),
                expected: expected_kind.to_string(),
                actual: stored.state_kind,
            });
        }

        Ok(stored)
    }

    /// Deserializes stored bytes into the credential state type.
    fn deserialize<C>(
        &self,
        credential_id: &str,
        stored: &StoredCredential,
    ) -> Result<C::State, ResolveError>
    where
        C: Credential,
    {
        serde_json::from_slice(&stored.data).map_err(|e| ResolveError::Deserialize {
            credential_id: credential_id.to_string(),
            reason: e.to_string(),
        })
    }

    /// Performs the actual refresh: calls `C::refresh()`, writes back
    /// to the store with CAS, and projects the result.
    async fn perform_refresh<C>(
        &self,
        credential_id: &str,
        mut state: C::State,
        stored: StoredCredential,
        ctx: &CredentialContext,
    ) -> Result<CredentialHandle<C::Scheme>, ResolveError>
    where
        C: Credential,
    {
        let outcome = C::refresh(&mut state, ctx)
            .await
            .map_err(|e| ResolveError::Refresh {
                credential_id: credential_id.to_string(),
                reason: e.to_string(),
            })?;

        match outcome {
            RefreshOutcome::Refreshed => {
                let data = serde_json::to_vec(&state).map_err(|e| ResolveError::Refresh {
                    credential_id: credential_id.to_string(),
                    reason: format!("failed to serialize refreshed state: {e}"),
                })?;

                let updated = StoredCredential {
                    data,
                    updated_at: chrono::Utc::now(),
                    expires_at: state.expires_at(),
                    ..stored
                };

                self.store
                    .put(
                        updated,
                        PutMode::CompareAndSwap {
                            expected_version: stored.version,
                        },
                    )
                    .await
                    .map_err(ResolveError::Store)?;

                let scheme = C::project(&state);
                Ok(CredentialHandle::new(scheme, credential_id))
            }
            RefreshOutcome::NotSupported => {
                // Not refreshable -- return current state anyway
                let scheme = C::project(&state);
                Ok(CredentialHandle::new(scheme, credential_id))
            }
            RefreshOutcome::ReauthRequired => Err(ResolveError::ReauthRequired {
                credential_id: credential_id.to_string(),
            }),
        }
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

    /// Refresh failed.
    #[error("credential {credential_id}: refresh failed: {reason}")]
    Refresh {
        /// The credential ID.
        credential_id: String,
        /// The refresh error message.
        reason: String,
    },

    /// Credential needs full re-authentication (refresh token expired).
    #[error("credential {credential_id}: re-authentication required")]
    ReauthRequired {
        /// The credential ID.
        credential_id: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credential_store::{PutMode, StoredCredential};
    use crate::credentials::ApiKeyCredential;
    use crate::store_memory::InMemoryStore;

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
