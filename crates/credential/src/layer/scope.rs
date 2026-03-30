//! Multi-tenant scope isolation layer.
//!
//! Outermost storage layer — checked before any data access.
//! Rejects cross-tenant credential access based on `metadata["owner_id"]`
//! vs the current scope from [`ScopeResolver`].
//!
//! # Security model
//!
//! - Scope mismatches return [`StoreError::NotFound`], never a distinct
//!   "permission denied" — callers cannot probe credential existence
//!   across tenants.
//! - A `None` return from [`ScopeResolver::current_owner`] represents
//!   admin / global access and bypasses all scope checks.
//!
//! # Limitations
//!
//! [`list`](CredentialStore::list) and [`exists`](CredentialStore::exists)
//! pass through to the inner store without scope filtering. Full tenant
//! isolation for these operations requires backend-level support
//! (e.g. a per-tenant namespace or query filter).

use std::sync::Arc;

use serde_json::Value;

use crate::credential_store::{CredentialStore, PutMode, StoreError, StoredCredential};

/// The metadata key used to store the owner identifier.
const OWNER_KEY: &str = "owner_id";

/// Resolves the current caller's scope for access control.
///
/// Implementations typically extract the owner identity from
/// a request-scoped context (e.g. JWT claims, session state).
///
/// # Examples
///
/// ```
/// use nebula_credential::layer::scope::ScopeResolver;
///
/// struct StaticScope(Option<String>);
///
/// impl ScopeResolver for StaticScope {
///     fn current_owner(&self) -> Option<&str> {
///         self.0.as_deref()
///     }
/// }
/// ```
pub trait ScopeResolver: Send + Sync {
    /// Returns the owner ID for the current request context.
    ///
    /// Returns `None` for admin / global access, which bypasses
    /// all scope checks.
    fn current_owner(&self) -> Option<&str>;
}

/// Multi-tenant isolation layer.
///
/// Wraps a [`CredentialStore`] and validates that the caller owns
/// the credential before allowing access. Sits outermost in the
/// layer stack:
///
/// ```text
/// Request → ScopeLayer → AuditLayer → EncryptionLayer → CacheLayer → Backend
/// ```
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_credential::{InMemoryStore, layer::scope::{ScopeLayer, ScopeResolver}};
/// use std::sync::Arc;
///
/// struct TenantScope(String);
/// impl ScopeResolver for TenantScope {
///     fn current_owner(&self) -> Option<&str> { Some(&self.0) }
/// }
///
/// let store = ScopeLayer::new(
///     InMemoryStore::new(),
///     Arc::new(TenantScope("tenant-1".into())),
/// );
/// ```
pub struct ScopeLayer<S> {
    inner: S,
    resolver: Arc<dyn ScopeResolver>,
}

impl<S> ScopeLayer<S> {
    /// Create a new scope isolation layer wrapping the given store.
    pub fn new(inner: S, resolver: Arc<dyn ScopeResolver>) -> Self {
        Self { inner, resolver }
    }
}

impl<S: CredentialStore> CredentialStore for ScopeLayer<S> {
    /// Retrieve a credential, verifying the caller owns it.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::NotFound`] if the credential does not exist
    /// **or** if the caller's scope does not match the stored `owner_id`.
    async fn get(&self, id: &str) -> Result<StoredCredential, StoreError> {
        let credential = self.inner.get(id).await?;
        verify_owner(&self.resolver, id, &credential)?;
        Ok(credential)
    }

    /// Store or update a credential with scope enforcement.
    ///
    /// For new credentials ([`PutMode::CreateOnly`]), the caller's owner ID
    /// is injected into metadata automatically.
    ///
    /// For updates ([`PutMode::Overwrite`] and [`PutMode::CompareAndSwap`]),
    /// the existing credential's owner is verified before allowing the write.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::NotFound`] if the caller's scope does not match
    /// the existing credential's `owner_id` on update operations.
    ///
    /// Returns [`StoreError::AlreadyExists`] or [`StoreError::VersionConflict`]
    /// from the inner store as appropriate.
    async fn put(
        &self,
        mut credential: StoredCredential,
        mode: PutMode,
    ) -> Result<StoredCredential, StoreError> {
        if let Some(owner) = self.resolver.current_owner() {
            match mode {
                PutMode::CreateOnly => {
                    credential
                        .metadata
                        .insert(OWNER_KEY.to_owned(), Value::String(owner.to_owned()));
                }
                PutMode::Overwrite | PutMode::CompareAndSwap { .. } => {
                    let existing = self.inner.get(&credential.id).await?;
                    verify_owner(&self.resolver, &credential.id, &existing)?;
                    credential
                        .metadata
                        .insert(OWNER_KEY.to_owned(), Value::String(owner.to_owned()));
                }
            }
        }
        self.inner.put(credential, mode).await
    }

    /// Delete a credential, verifying the caller owns it first.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::NotFound`] if the credential does not exist
    /// **or** if the caller's scope does not match the stored `owner_id`.
    async fn delete(&self, id: &str) -> Result<(), StoreError> {
        let credential = self.inner.get(id).await?;
        verify_owner(&self.resolver, id, &credential)?;
        self.inner.delete(id).await
    }

    /// List credential IDs (pass-through — no scope filtering).
    ///
    /// # Limitations
    ///
    /// This method does **not** filter by owner. Full tenant isolation
    /// on `list` requires backend-level query support. Callers should
    /// treat returned IDs as candidates and use [`get`](Self::get) for
    /// access-checked retrieval.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Backend`] on underlying storage failures.
    async fn list(&self, state_kind: Option<&str>) -> Result<Vec<String>, StoreError> {
        self.inner.list(state_kind).await
    }

    /// Check if a credential exists (pass-through — no scope filtering).
    ///
    /// # Limitations
    ///
    /// This method does **not** check ownership. Use [`get`](Self::get)
    /// for scope-checked access.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Backend`] on underlying storage failures.
    async fn exists(&self, id: &str) -> Result<bool, StoreError> {
        self.inner.exists(id).await
    }
}

/// Check that the credential's `owner_id` matches the resolver's current owner.
///
/// Returns `Ok(())` if:
/// - The resolver returns `None` (admin/global scope — bypass), or
/// - The credential's `metadata["owner_id"]` matches the resolver's owner, or
/// - The credential has no `owner_id` in metadata (unscoped legacy credential).
///
/// Returns `StoreError::NotFound` on mismatch to avoid leaking existence.
fn verify_owner(
    resolver: &Arc<dyn ScopeResolver>,
    id: &str,
    credential: &StoredCredential,
) -> Result<(), StoreError> {
    let Some(caller_owner) = resolver.current_owner() else {
        // Admin / global access — bypass scope check.
        return Ok(());
    };

    if let Some(Value::String(stored_owner)) = credential.metadata.get(OWNER_KEY)
        && stored_owner != caller_owner
    {
        return Err(StoreError::NotFound { id: id.to_owned() });
    }
    // No owner_id in metadata → unscoped credential, allow access.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credential_store::PutMode;
    use crate::store_memory::InMemoryStore;

    struct FixedScope(Option<String>);

    impl ScopeResolver for FixedScope {
        fn current_owner(&self) -> Option<&str> {
            self.0.as_deref()
        }
    }

    fn scoped_store(owner: Option<&str>) -> ScopeLayer<InMemoryStore> {
        ScopeLayer::new(
            InMemoryStore::new(),
            Arc::new(FixedScope(owner.map(ToOwned::to_owned))),
        )
    }

    fn make_credential(id: &str) -> StoredCredential {
        StoredCredential {
            id: id.into(),
            data: b"test-data".to_vec(),
            state_kind: "test".into(),
            state_version: 1,
            version: 0,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            expires_at: None,
            metadata: Default::default(),
        }
    }

    fn make_credential_with_owner(id: &str, owner: &str) -> StoredCredential {
        let mut cred = make_credential(id);
        cred.metadata
            .insert(OWNER_KEY.to_owned(), Value::String(owner.to_owned()));
        cred
    }

    // ── get ─────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn get_succeeds_for_matching_owner() {
        let store = scoped_store(Some("tenant-1"));
        let cred = make_credential("cred-1");
        // put injects owner_id automatically
        store.put(cred, PutMode::CreateOnly).await.unwrap();

        let fetched = store.get("cred-1").await.unwrap();
        assert_eq!(
            fetched.metadata.get(OWNER_KEY),
            Some(&Value::String("tenant-1".into()))
        );
    }

    #[tokio::test]
    async fn get_rejects_wrong_owner() {
        // Store as tenant-1 via inner store directly
        let inner = InMemoryStore::new();
        let cred = make_credential_with_owner("cred-2", "tenant-1");
        inner.put(cred, PutMode::CreateOnly).await.unwrap();

        // Access as tenant-2
        let store = ScopeLayer::new(
            inner,
            Arc::new(FixedScope(Some("tenant-2".to_owned()))),
        );
        let err = store.get("cred-2").await.unwrap_err();
        assert!(matches!(err, StoreError::NotFound { .. }));
    }

    #[tokio::test]
    async fn get_succeeds_for_admin_scope() {
        let inner = InMemoryStore::new();
        let cred = make_credential_with_owner("cred-3", "tenant-1");
        inner.put(cred, PutMode::CreateOnly).await.unwrap();

        // Admin scope (None) bypasses checks
        let store = ScopeLayer::new(inner, Arc::new(FixedScope(None)));
        let fetched = store.get("cred-3").await.unwrap();
        assert_eq!(fetched.id, "cred-3");
    }

    #[tokio::test]
    async fn get_allows_unscoped_credential() {
        let inner = InMemoryStore::new();
        // Credential without owner_id in metadata
        let cred = make_credential("cred-unscoped");
        inner.put(cred, PutMode::CreateOnly).await.unwrap();

        let store = ScopeLayer::new(
            inner,
            Arc::new(FixedScope(Some("any-tenant".to_owned()))),
        );
        let fetched = store.get("cred-unscoped").await.unwrap();
        assert_eq!(fetched.id, "cred-unscoped");
    }

    // ── put ─────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn put_injects_owner_into_metadata() {
        let store = scoped_store(Some("tenant-1"));
        let cred = make_credential("cred-4");
        let stored = store.put(cred, PutMode::CreateOnly).await.unwrap();

        assert_eq!(
            stored.metadata.get(OWNER_KEY),
            Some(&Value::String("tenant-1".into()))
        );
    }

    #[tokio::test]
    async fn put_overwrite_rejects_wrong_owner() {
        let inner = InMemoryStore::new();
        let cred = make_credential_with_owner("cred-5", "tenant-1");
        inner.put(cred, PutMode::CreateOnly).await.unwrap();

        let store = ScopeLayer::new(
            inner,
            Arc::new(FixedScope(Some("tenant-2".to_owned()))),
        );
        let update = make_credential("cred-5");
        let err = store.put(update, PutMode::Overwrite).await.unwrap_err();
        assert!(matches!(err, StoreError::NotFound { .. }));
    }

    #[tokio::test]
    async fn put_overwrite_succeeds_for_matching_owner() {
        let inner = InMemoryStore::new();
        let cred = make_credential_with_owner("cred-6", "tenant-1");
        inner.put(cred, PutMode::CreateOnly).await.unwrap();

        let store = ScopeLayer::new(
            inner,
            Arc::new(FixedScope(Some("tenant-1".to_owned()))),
        );
        let mut update = make_credential("cred-6");
        update.data = b"updated-data".to_vec();
        let stored = store.put(update, PutMode::Overwrite).await.unwrap();
        assert_eq!(stored.data, b"updated-data");
    }

    #[tokio::test]
    async fn put_admin_skips_scope_injection() {
        let store = scoped_store(None);
        let cred = make_credential("cred-admin");
        let stored = store.put(cred, PutMode::CreateOnly).await.unwrap();

        // Admin scope: no owner_id injected
        assert!(stored.metadata.get(OWNER_KEY).is_none());
    }

    // ── delete ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn delete_rejects_wrong_owner() {
        let inner = InMemoryStore::new();
        let cred = make_credential_with_owner("cred-7", "tenant-1");
        inner.put(cred, PutMode::CreateOnly).await.unwrap();

        let store = ScopeLayer::new(
            inner,
            Arc::new(FixedScope(Some("tenant-2".to_owned()))),
        );
        let err = store.delete("cred-7").await.unwrap_err();
        assert!(matches!(err, StoreError::NotFound { .. }));
    }

    #[tokio::test]
    async fn delete_succeeds_for_matching_owner() {
        let store = scoped_store(Some("tenant-1"));
        let cred = make_credential("cred-8");
        store.put(cred, PutMode::CreateOnly).await.unwrap();

        store.delete("cred-8").await.unwrap();
        let err = store.get("cred-8").await.unwrap_err();
        assert!(matches!(err, StoreError::NotFound { .. }));
    }

    #[tokio::test]
    async fn delete_succeeds_for_admin_scope() {
        let inner = InMemoryStore::new();
        let cred = make_credential_with_owner("cred-9", "tenant-1");
        inner.put(cred, PutMode::CreateOnly).await.unwrap();

        let store = ScopeLayer::new(inner, Arc::new(FixedScope(None)));
        store.delete("cred-9").await.unwrap();
    }
}
