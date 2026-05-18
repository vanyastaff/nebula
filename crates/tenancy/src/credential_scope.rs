//! Multi-tenant scope isolation layer for
//! [`CredentialStore`](nebula_credential::CredentialStore).
//!
//! Re-homed from `nebula_storage::credential::layer::scope` (spec §8): the
//! credential scope **policy** belongs in the tenancy security boundary,
//! not in the storage adapter. Shape preserved verbatim — this is the
//! same `ScopeLayer` / `ScopeResolver` the credential stack has always
//! used; `nebula_storage::credential` now re-exports these under their
//! historical names so every consumer compiles unchanged across the move.
//!
//! The credential-specific `EncryptionLayer` / `CacheLayer` / `AuditLayer`
//! stay in `nebula-storage` and re-compose **on top** of this layer; their
//! fail-closed audit + zeroize-on-drop invariants are unaffected
//! by the move (the layer order — `ScopeLayer → AuditLayer →
//! EncryptionLayer → CacheLayer → Backend` — is preserved at the
//! composition root) and remain regression-tested in
//! `crates/storage/tests/credential_*`.
//!
//! # Security model
//!
//! - Scope mismatches return [`StoreError::NotFound`], never a distinct
//!   "permission denied" — callers cannot probe credential existence
//!   across tenants.
//! - A `None` return from [`ScopeResolver::current_owner`] represents
//!   admin / global access and bypasses all scope checks.
//!
//! # Scope enforcement
//!
//! All [`CredentialStore`] operations — including [`list`](CredentialStore::list)
//! and [`exists`](CredentialStore::exists) — are filtered by owner. Tenant
//! callers only see their own credentials; admin callers see all.

use std::sync::Arc;

use nebula_credential::{CredentialStore, PutMode, ScopeResolver, StoreError, StoredCredential};
use serde_json::Value;

// The `ScopeResolver` trait is the credential-store scope abstraction; it
// lives in `nebula-credential` (the contract crate) so downward consumers
// name it without an upward `→ nebula-tenancy` dependency (spec §3
// data-vs-policy split). This crate owns only the concrete scoping
// **policy** (`ScopeLayer`) that `impl`s the credential-store contract on
// top of that resolver — re-exported as `CredentialScopeResolver` /
// `CredentialScopeLayer` from `nebula_tenancy` (see `lib.rs`).

/// The metadata key used to store the owner identifier.
const OWNER_KEY: &str = "owner_id";

/// Multi-tenant credential isolation layer.
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
/// use nebula_tenancy::{CredentialScopeLayer, CredentialScopeResolver};
/// use nebula_credential::InMemoryStore;
/// use std::sync::Arc;
///
/// struct TenantScope(String);
/// impl CredentialScopeResolver for TenantScope {
/// fn current_owner(&self) -> Option<&str> { Some(&self.0) }
/// }
///
/// let store = CredentialScopeLayer::new(
/// InMemoryStore::new(),
/// Arc::new(TenantScope("tenant-1".into())),
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
                },
                PutMode::Overwrite | PutMode::CompareAndSwap { .. } => {
                    let existing = self.inner.get(&credential.id).await?;
                    verify_owner(&self.resolver, &credential.id, &existing)?;
                    credential
                        .metadata
                        .insert(OWNER_KEY.to_owned(), Value::String(owner.to_owned()));
                },
                // `PutMode` is `#[non_exhaustive]`. Future modes default to the
                // strictest path (owner verification + stamp) until explicitly
                // routed here.
                _ => {
                    let existing = self.inner.get(&credential.id).await?;
                    verify_owner(&self.resolver, &credential.id, &existing)?;
                    credential
                        .metadata
                        .insert(OWNER_KEY.to_owned(), Value::String(owner.to_owned()));
                },
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

    /// List credential IDs visible to the current scope.
    ///
    /// For tenant callers, filters results to only credentials owned by
    /// the caller (matching `metadata["owner_id"]`). Admin callers
    /// (`current_owner() == None`) see all credentials.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Backend`] on underlying storage failures.
    async fn list(&self, state_kind: Option<&str>) -> Result<Vec<String>, StoreError> {
        let ids = self.inner.list(state_kind).await?;
        let Some(caller_owner) = self.resolver.current_owner() else {
            return Ok(ids); // Admin bypass
        };
        let mut owned = Vec::with_capacity(ids.len());
        for id in &ids {
            if let Ok(cred) = self.inner.get(id).await
                && matches!(
                    cred.metadata.get(OWNER_KEY),
                    Some(Value::String(owner)) if owner == caller_owner
                )
            {
                owned.push(id.clone());
            }
        }
        Ok(owned)
    }

    /// Check if a credential exists and is visible to the current scope.
    ///
    /// For tenant callers, delegates to [`get`](Self::get) so that scope
    /// enforcement applies — a credential owned by another tenant returns
    /// `false`, not leaking its existence. Admin callers bypass scope checks.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Backend`] on underlying storage failures.
    async fn exists(&self, id: &str) -> Result<bool, StoreError> {
        let Some(_) = self.resolver.current_owner() else {
            return self.inner.exists(id).await; // Admin bypass
        };
        match self.get(id).await {
            Ok(_) => Ok(true),
            Err(StoreError::NotFound { .. }) => Ok(false),
            Err(e) => Err(e),
        }
    }
}

/// Check that the credential's `owner_id` matches the resolver's current owner.
///
/// Returns `Ok(())` if:
/// - The resolver returns `None` (admin/global scope — bypass), or
/// - The credential's `metadata["owner_id"]` matches the resolver's owner.
///
/// Returns `StoreError::NotFound` if:
/// - The owner does not match, or
/// - The credential has no `owner_id` in metadata (fail-closed: only admin can access
///   unscoped/legacy credentials).
///
/// Returns `StoreError::NotFound` on rejection to avoid leaking existence.
fn verify_owner(
    resolver: &Arc<dyn ScopeResolver>,
    id: &str,
    credential: &StoredCredential,
) -> Result<(), StoreError> {
    let Some(caller_owner) = resolver.current_owner() else {
        // Admin / global access — bypass scope check.
        return Ok(());
    };

    match credential.metadata.get(OWNER_KEY) {
        Some(Value::String(stored_owner)) if stored_owner == caller_owner => Ok(()),
        Some(Value::String(_)) => Err(StoreError::NotFound { id: id.to_owned() }),
        _ => {
            // No owner_id or wrong type → only admin can access.
            // Non-admin callers get NotFound (fail-closed).
            Err(StoreError::NotFound { id: id.to_owned() })
        },
    }
}

// The in-memory `CredentialStore` used as the inner store is behind
// `nebula-credential`'s `test-util` feature; the dev-dependency enables
// it for test builds, so a plain `#[cfg(test)]` gate suffices.
#[cfg(test)]
mod tests {
    use nebula_credential::{InMemoryStore, PutMode};

    use super::*;

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
            credential_key: "test_credential".into(),
            data: b"test-data".to_vec(),
            state_kind: "test".into(),
            state_version: 1,
            version: 0,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            expires_at: None,
            reauth_required: false,
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
        store.put(cred, PutMode::CreateOnly).await.unwrap();

        let fetched = store.get("cred-1").await.unwrap();
        assert_eq!(
            fetched.metadata.get(OWNER_KEY),
            Some(&Value::String("tenant-1".into()))
        );
    }

    #[tokio::test]
    async fn get_rejects_wrong_owner() {
        let inner = InMemoryStore::new();
        let cred = make_credential_with_owner("cred-2", "tenant-1");
        inner.put(cred, PutMode::CreateOnly).await.unwrap();

        let store = ScopeLayer::new(inner, Arc::new(FixedScope(Some("tenant-2".to_owned()))));
        let err = store.get("cred-2").await.unwrap_err();
        assert!(matches!(err, StoreError::NotFound { .. }));
    }

    #[tokio::test]
    async fn get_succeeds_for_admin_scope() {
        let inner = InMemoryStore::new();
        let cred = make_credential_with_owner("cred-3", "tenant-1");
        inner.put(cred, PutMode::CreateOnly).await.unwrap();

        let store = ScopeLayer::new(inner, Arc::new(FixedScope(None)));
        let fetched = store.get("cred-3").await.unwrap();
        assert_eq!(fetched.id, "cred-3");
    }

    #[tokio::test]
    async fn get_rejects_unscoped_credential_for_non_admin() {
        let inner = InMemoryStore::new();
        let cred = make_credential("cred-unscoped");
        inner.put(cred, PutMode::CreateOnly).await.unwrap();

        let store = ScopeLayer::new(inner, Arc::new(FixedScope(Some("any-tenant".to_owned()))));
        let err = store.get("cred-unscoped").await.unwrap_err();
        assert!(matches!(err, StoreError::NotFound { .. }));
    }

    #[tokio::test]
    async fn get_allows_unscoped_credential_for_admin() {
        let inner = InMemoryStore::new();
        let cred = make_credential("cred-unscoped-admin");
        inner.put(cred, PutMode::CreateOnly).await.unwrap();

        let store = ScopeLayer::new(inner, Arc::new(FixedScope(None)));
        let fetched = store.get("cred-unscoped-admin").await.unwrap();
        assert_eq!(fetched.id, "cred-unscoped-admin");
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

        let store = ScopeLayer::new(inner, Arc::new(FixedScope(Some("tenant-2".to_owned()))));
        let update = make_credential("cred-5");
        let err = store.put(update, PutMode::Overwrite).await.unwrap_err();
        assert!(matches!(err, StoreError::NotFound { .. }));
    }

    #[tokio::test]
    async fn put_overwrite_succeeds_for_matching_owner() {
        let inner = InMemoryStore::new();
        let cred = make_credential_with_owner("cred-6", "tenant-1");
        inner.put(cred, PutMode::CreateOnly).await.unwrap();

        let store = ScopeLayer::new(inner, Arc::new(FixedScope(Some("tenant-1".to_owned()))));
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

        assert!(stored.metadata.get(OWNER_KEY).is_none());
    }

    // ── delete ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn delete_rejects_wrong_owner() {
        let inner = InMemoryStore::new();
        let cred = make_credential_with_owner("cred-7", "tenant-1");
        inner.put(cred, PutMode::CreateOnly).await.unwrap();

        let store = ScopeLayer::new(inner, Arc::new(FixedScope(Some("tenant-2".to_owned()))));
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

    // ── list ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn list_filters_by_owner() {
        let inner = InMemoryStore::new();
        let cred_a = make_credential_with_owner("cred-a", "tenant-a");
        let cred_b = make_credential_with_owner("cred-b", "tenant-b");
        inner.put(cred_a, PutMode::CreateOnly).await.unwrap();
        inner.put(cred_b, PutMode::CreateOnly).await.unwrap();

        let store = ScopeLayer::new(inner, Arc::new(FixedScope(Some("tenant-a".to_owned()))));
        let ids = store.list(None).await.unwrap();
        assert_eq!(ids, vec!["cred-a".to_owned()]);
    }

    #[tokio::test]
    async fn list_admin_sees_all() {
        let inner = InMemoryStore::new();
        let cred_a = make_credential_with_owner("cred-a2", "tenant-a");
        let cred_b = make_credential_with_owner("cred-b2", "tenant-b");
        inner.put(cred_a, PutMode::CreateOnly).await.unwrap();
        inner.put(cred_b, PutMode::CreateOnly).await.unwrap();

        let store = ScopeLayer::new(inner, Arc::new(FixedScope(None)));
        let mut ids = store.list(None).await.unwrap();
        ids.sort();
        assert_eq!(ids, vec!["cred-a2".to_owned(), "cred-b2".to_owned()]);
    }

    // ── exists ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn exists_respects_scope() {
        let inner = InMemoryStore::new();
        let cred = make_credential_with_owner("cred-other", "tenant-b");
        inner.put(cred, PutMode::CreateOnly).await.unwrap();

        let store = ScopeLayer::new(inner, Arc::new(FixedScope(Some("tenant-a".to_owned()))));
        assert!(!store.exists("cred-other").await.unwrap());
    }

    #[tokio::test]
    async fn exists_true_for_own_credential() {
        let inner = InMemoryStore::new();
        let cred = make_credential_with_owner("cred-mine", "tenant-a");
        inner.put(cred, PutMode::CreateOnly).await.unwrap();

        let store = ScopeLayer::new(inner, Arc::new(FixedScope(Some("tenant-a".to_owned()))));
        assert!(store.exists("cred-mine").await.unwrap());
    }

    #[tokio::test]
    async fn exists_admin_sees_all() {
        let inner = InMemoryStore::new();
        let cred = make_credential_with_owner("cred-any", "tenant-x");
        inner.put(cred, PutMode::CreateOnly).await.unwrap();

        let store = ScopeLayer::new(inner, Arc::new(FixedScope(None)));
        assert!(store.exists("cred-any").await.unwrap());
    }

    #[tokio::test]
    async fn exists_returns_false_for_nonexistent() {
        let store = scoped_store(Some("tenant-a"));
        assert!(!store.exists("no-such-cred").await.unwrap());
    }

    // ── cross-tenant credential isolation (spec §6.1 abuse case 4) ───────

    /// Tenant B cannot read, overwrite, delete, or even confirm the
    /// existence of a credential tenant A created — every cross-tenant
    /// path returns `NotFound`, never the row, never a distinct error
    /// (the same existence-non-disclosure rule the storage decorators
    /// enforce). This is the credential-layer analogue of the
    /// `cross_tenant_denial` execution-store suite and closes abuse
    /// case 4's cross-tenant half.
    #[tokio::test]
    async fn cross_tenant_credential_access_is_denied_uniformly() {
        let inner = InMemoryStore::new();
        // Tenant A creates a credential through its own scoped layer
        // (owner_id is injected automatically).
        let store_a = ScopeLayer::new(
            inner.clone(),
            Arc::new(FixedScope(Some("tenant-a".to_owned()))),
        );
        store_a
            .put(make_credential("shared-id"), PutMode::CreateOnly)
            .await
            .unwrap();

        // Tenant B, sharing the SAME backend, attacks every entry point.
        let store_b = ScopeLayer::new(inner, Arc::new(FixedScope(Some("tenant-b".to_owned()))));

        let read = store_b.get("shared-id").await;
        assert!(
            matches!(read, Err(StoreError::NotFound { .. })),
            "cross-tenant get must be NotFound, got {read:?}"
        );

        let exists = store_b.exists("shared-id").await.unwrap();
        assert!(!exists, "cross-tenant exists must be false (no oracle)");

        let mut hijack = make_credential("shared-id");
        hijack.data = b"hijacked".to_vec();
        let overwrite = store_b.put(hijack, PutMode::Overwrite).await;
        assert!(
            matches!(overwrite, Err(StoreError::NotFound { .. })),
            "cross-tenant overwrite must be NotFound, got {overwrite:?}"
        );

        let del = store_b.delete("shared-id").await;
        assert!(
            matches!(del, Err(StoreError::NotFound { .. })),
            "cross-tenant delete must be NotFound, got {del:?}"
        );

        // Tenant A's credential is intact and untouched.
        let a_view = store_a.get("shared-id").await.unwrap();
        assert_eq!(
            a_view.data, b"test-data",
            "victim credential must be unmodified by tenant B"
        );

        // A different tenant's id-space never resolves to A's row.
        let b_owned = store_b.list(None).await.unwrap();
        assert!(
            b_owned.is_empty(),
            "tenant B must not see tenant A's credential in list"
        );
    }
}
