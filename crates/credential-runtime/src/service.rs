//! `CredentialService<B, PS>` — the sole public entry to the credential
//! management bounded context. Generic over the raw backend `B` and
//! pending store `PS` (both RPITIT non-object-safe; the params live only
//! on the struct, never in operation signatures). All invariant-bearing
//! composition is crate-private: the only constructor path is
//! [`CredentialServiceBuilder`](crate::builder::CredentialServiceBuilder),
//! whose `build()` wraps the raw backend in the layered store so an
//! unencrypted/mis-composed service is unrepresentable.
//!
//! ## Tenant isolation
//!
//! Tenancy is enforced at the operation level (not via the storage
//! `ScopeLayer`, which the build-once stack omits): [`create`] persists
//! `StoredCredential.metadata["owner_id"] = scope.owner_id()`;
//! [`get`]/[`list`]/[`update`]/[`delete`] load then reject rows whose
//! `owner_id` differs with [`CredentialServiceError::NotFound`] — no
//! cross-tenant existence leak (a credential in another tenant is
//! indistinguishable from a missing one).
//!
//! [`create`]: CredentialService::create
//! [`get`]: CredentialService::get
//! [`list`]: CredentialService::list
//! [`update`]: CredentialService::update
//! [`delete`]: CredentialService::delete

use std::sync::Arc;

use nebula_credential::pending_store::PendingStateStore;
use nebula_credential::store::{CredentialStore, PutMode, StoreError, StoredCredential};
use nebula_credential::{
    CredentialContext, CredentialId, CredentialRecord, CredentialRegistry, CredentialSnapshot,
};
use nebula_engine::credential::{CredentialResolver, LeaseLifecycle};
use nebula_schema::FieldValues;
use nebula_storage::credential::{AuditLayer, CacheLayer, EncryptionLayer};
use serde_json::Value;

use crate::CredentialServiceError;
use crate::dispatch::CredentialDispatch;
use crate::observer::CredentialObserver;
use crate::ops::DispatchOps;
use crate::scope::TenantScope;
use crate::state_source::StateSource;

/// Metadata key the facade stamps with the owning tenant. Read on every
/// `get`/`list`/`update`/`delete` to enforce tenant isolation.
const OWNER_ID_KEY: &str = "owner_id";

/// Crate-private layered store stack composed once at `build()`:
/// `Audit(Cache(Encryption(raw)))`. `Encryption` is adjacent to the raw
/// backend so persisted bytes are always ciphertext (spec §6 #7).
pub(crate) type LayeredStore<B> = AuditLayer<CacheLayer<EncryptionLayer<B>>>;

/// Sole public entry to the credential management bounded context.
///
/// Constructed only via
/// [`CredentialServiceBuilder`](crate::builder::CredentialServiceBuilder).
pub struct CredentialService<B: CredentialStore, PS: PendingStateStore> {
    pub(crate) store: Arc<LayeredStore<B>>,
    // Consumed by the acquisition/refresh operations (resolve /
    // resolve_with_refresh) which land alongside the dispatch closures
    // in a later increment of this crate.
    #[allow(dead_code)]
    pub(crate) resolver: CredentialResolver<LayeredStore<B>>,
    pub(crate) lease: LeaseLifecycle,
    pub(crate) pending: PS,
    // Consumed by the type-discovery operations (list_types / get_type)
    // which project `CredentialMetadata` from the registry.
    #[allow(dead_code)]
    pub(crate) registry: Arc<CredentialRegistry>,
    pub(crate) dispatch: Arc<CredentialDispatch>,
    pub(crate) ops: Arc<DispatchOps<B, PS>>,
    pub(crate) observer: Arc<dyn CredentialObserver>,
    // Consumed by the acquisition path when state comes from an external
    // provider chain instead of the local encrypted store.
    #[allow(dead_code)]
    pub(crate) source: StateSource,
}

impl<B: CredentialStore, PS: PendingStateStore> CredentialService<B, PS> {
    /// Crate-private assembly point — the builder's `build()` is the
    /// only caller. Not `pub`: external code cannot bypass the layered
    /// composition (compile-fail probe target, spec §6 #7).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn __from_parts(
        store: Arc<LayeredStore<B>>,
        resolver: CredentialResolver<LayeredStore<B>>,
        lease: LeaseLifecycle,
        pending: PS,
        registry: Arc<CredentialRegistry>,
        dispatch: Arc<CredentialDispatch>,
        ops: Arc<DispatchOps<B, PS>>,
        observer: Arc<dyn CredentialObserver>,
        source: StateSource,
    ) -> Self {
        Self {
            store,
            resolver,
            lease,
            pending,
            registry,
            dispatch,
            ops,
            observer,
            source,
        }
    }

    /// Active dynamic-lease count (smoke accessor).
    pub async fn active_lease_count(&self) -> usize {
        self.lease.active_lease_count().await
    }

    // ── CRUD operations ──────────────────────────────────────────────

    /// Create a credential: validate `props` against the type's schema,
    /// resolve it to encrypted state, and persist it scoped to `scope`.
    ///
    /// The validation pipeline is the canonical credential pipeline
    /// (canon §12.5): `properties_schema().validate(FieldValues)` then a
    /// typed `serde_json::from_value` round-trip — a `{"$expr": ..}`
    /// envelope survives schema validation but is refused by the typed
    /// deserialize, so secrets never depend on workflow state.
    ///
    /// # Errors
    ///
    /// - [`CredentialServiceError::TypeUnknown`] — no type registered under `credential_key`.
    /// - [`CredentialServiceError::ValidationFailed`] — schema or typed-deserialize rejection
    ///   (including `$expr` injection), or a resolve failure.
    /// - [`CredentialServiceError::Store`] — persistence failure (incl. fail-closed audit).
    pub async fn create(
        &self,
        scope: &TenantScope,
        credential_key: &str,
        props: Value,
    ) -> Result<CredentialSnapshot, CredentialServiceError> {
        // The type must be registered (TypeUnknown closes the abuse where
        // an unregistered key reaches resolution).
        if !self.dispatch.contains(credential_key) {
            return Err(CredentialServiceError::TypeUnknown {
                key: credential_key.to_owned(),
            });
        }

        // Canonical validation pipeline: schema validate + typed
        // deserialize (the `$expr` refusal point) without ever resolving
        // expressions. Monomorphised per type in the ops table.
        self.ops.validate(credential_key, &props)?;

        let values = FieldValues::from_json(props).map_err(|e| {
            CredentialServiceError::ValidationFailed {
                reason: format!("property ingest failed: {e}"),
            }
        })?;

        let id = CredentialId::new();
        let ctx = Self::owner_context(scope.owner_id());

        let resolved = self
            .ops
            .resolve(credential_key, &values, &ctx, &self.pending)
            .await?;

        let mut metadata = serde_json::Map::new();
        metadata.insert(
            OWNER_ID_KEY.to_owned(),
            Value::String(scope.owner_id().to_owned()),
        );

        // Project the response snapshot from the just-resolved state
        // bytes before they are moved into the stored row (avoids a
        // round-trip + decrypt; identical projection to `get`).
        let mut record = CredentialRecord::new();
        record.expires_at = resolved.expires_at;
        let snapshot = self.ops.snapshot(credential_key, &resolved.data, record)?;

        let now = chrono::Utc::now();
        let stored = StoredCredential {
            id: id.to_string(),
            credential_key: credential_key.to_owned(),
            data: resolved.data.to_vec(),
            state_kind: resolved.state_kind,
            state_version: resolved.state_version,
            version: 0,
            created_at: now,
            updated_at: now,
            expires_at: resolved.expires_at,
            reauth_required: false,
            metadata,
        };

        self.store
            .put(stored, PutMode::CreateOnly)
            .await
            .map_err(Self::map_store_err)?;

        self.observer.on_resolve(&id);
        tracing::info!(
            credential.key = credential_key,
            credential.id = %id,
            "credential created"
        );

        Ok(snapshot)
    }

    /// Fetch a credential's secret-free snapshot, scoped to `scope`.
    ///
    /// # Errors
    ///
    /// [`CredentialServiceError::NotFound`] if the id is absent **or**
    /// belongs to another tenant (no cross-tenant existence leak);
    /// [`CredentialServiceError::Internal`] on a decode failure.
    pub async fn get(
        &self,
        scope: &TenantScope,
        id: &str,
    ) -> Result<CredentialSnapshot, CredentialServiceError> {
        self.snapshot_from_store(scope, id).await
    }

    /// List credential ids visible to `scope` (rows whose stored
    /// `owner_id` matches).
    ///
    /// # Errors
    ///
    /// [`CredentialServiceError::Store`] on a backend failure.
    pub async fn list(&self, scope: &TenantScope) -> Result<Vec<String>, CredentialServiceError> {
        let ids = self.store.list(None).await.map_err(Self::map_store_err)?;
        let mut visible = Vec::new();
        for id in ids {
            match self.store.get(&id).await {
                Ok(stored) => {
                    if Self::owner_matches(&stored, scope) {
                        visible.push(id);
                    }
                },
                // A row that vanished between `list` and `get` is simply
                // not visible; a hard backend error propagates.
                Err(StoreError::NotFound { .. }) => {},
                Err(e) => return Err(Self::map_store_err(e)),
            }
        }
        Ok(visible)
    }

    /// Replace a credential's stored state via compare-and-swap.
    ///
    /// `expected_version` is the optimistic-concurrency precondition; a
    /// mismatch surfaces as [`CredentialServiceError::VersionConflict`].
    ///
    /// # Errors
    ///
    /// - [`CredentialServiceError::NotFound`] — absent or cross-tenant id.
    /// - [`CredentialServiceError::ValidationFailed`] — schema / typed-deserialize / resolve.
    /// - [`CredentialServiceError::VersionConflict`] — stale `expected_version`.
    /// - [`CredentialServiceError::Store`] — persistence failure.
    pub async fn update(
        &self,
        scope: &TenantScope,
        id: &str,
        props: Value,
        expected_version: u64,
    ) -> Result<CredentialSnapshot, CredentialServiceError> {
        // Owner check first: a cross-tenant id is reported as missing,
        // never as a version conflict (no existence leak).
        let existing = self.load_owned(scope, id).await?;

        self.ops.validate(&existing.credential_key, &props)?;
        let values = FieldValues::from_json(props).map_err(|e| {
            CredentialServiceError::ValidationFailed {
                reason: format!("property ingest failed: {e}"),
            }
        })?;

        let ctx = Self::owner_context(scope.owner_id());
        let resolved = self
            .ops
            .resolve(&existing.credential_key, &values, &ctx, &self.pending)
            .await?;

        let mut metadata = existing.metadata.clone();
        metadata.insert(
            OWNER_ID_KEY.to_owned(),
            Value::String(scope.owner_id().to_owned()),
        );

        let now = chrono::Utc::now();
        let stored = StoredCredential {
            id: existing.id.clone(),
            credential_key: existing.credential_key.clone(),
            data: resolved.data.to_vec(),
            state_kind: resolved.state_kind,
            state_version: resolved.state_version,
            version: existing.version,
            created_at: existing.created_at,
            updated_at: now,
            expires_at: resolved.expires_at,
            reauth_required: false,
            metadata,
        };

        self.store
            .put(stored, PutMode::CompareAndSwap { expected_version })
            .await
            .map_err(Self::map_store_err)?;

        tracing::info!(credential.id = %id, "credential updated");
        self.snapshot_from_store(scope, id).await
    }

    /// Delete a credential scoped to `scope`.
    ///
    /// # Errors
    ///
    /// [`CredentialServiceError::NotFound`] if absent or cross-tenant;
    /// [`CredentialServiceError::Store`] on a backend failure.
    pub async fn delete(
        &self,
        scope: &TenantScope,
        id: &str,
    ) -> Result<(), CredentialServiceError> {
        // Owner check: cross-tenant delete is indistinguishable from a
        // missing credential.
        let _existing = self.load_owned(scope, id).await?;
        self.store.delete(id).await.map_err(Self::map_store_err)?;
        tracing::info!(credential.id = %id, "credential deleted");
        Ok(())
    }

    // ── Internal helpers ─────────────────────────────────────────────

    /// Load a row and assert it belongs to `scope`, mapping both "absent"
    /// and "other tenant" to [`CredentialServiceError::NotFound`].
    async fn load_owned(
        &self,
        scope: &TenantScope,
        id: &str,
    ) -> Result<StoredCredential, CredentialServiceError> {
        let stored = match self.store.get(id).await {
            Ok(s) => s,
            Err(StoreError::NotFound { .. }) => {
                return Err(CredentialServiceError::NotFound { id: id.to_owned() });
            },
            Err(e) => return Err(Self::map_store_err(e)),
        };
        if !Self::owner_matches(&stored, scope) {
            // Deliberately the same error as a missing credential — a
            // caller cannot probe other tenants' ids.
            return Err(CredentialServiceError::NotFound { id: id.to_owned() });
        }
        Ok(stored)
    }

    /// Load + owner-check + project to a secret-free snapshot.
    async fn snapshot_from_store(
        &self,
        scope: &TenantScope,
        id: &str,
    ) -> Result<CredentialSnapshot, CredentialServiceError> {
        let stored = self.load_owned(scope, id).await?;
        let mut record = CredentialRecord::new();
        record.created_at = stored.created_at;
        record.last_modified = stored.updated_at;
        record.expires_at = stored.expires_at;
        self.ops
            .snapshot(&stored.credential_key, &stored.data, record)
    }

    /// Whether `stored` is owned by `scope`. A row missing the
    /// `owner_id` stamp is treated as foreign (fail-closed).
    fn owner_matches(stored: &StoredCredential, scope: &TenantScope) -> bool {
        stored
            .metadata
            .get(OWNER_ID_KEY)
            .and_then(Value::as_str)
            .is_some_and(|o| o == scope.owner_id())
    }

    /// Build the minimal owner-scoped [`CredentialContext`] the resolve
    /// pipeline needs for `create` / `update`.
    ///
    /// `CredentialContext::for_test` is the upstream `pub` constructor
    /// that assembles exactly this shape (default credential/resource
    /// accessors + an `owner_id` override); despite its name it is **not**
    /// test-gated. First-party credential types resolve from their typed
    /// properties and ignore the context accessors, so the defaults are
    /// correct here. A production context wired with real accessors (for
    /// plugin credentials that consult them) is a follow-up; routing
    /// every call through this one helper keeps that migration to a
    /// single site.
    fn owner_context(owner_id: &str) -> CredentialContext {
        CredentialContext::for_test(owner_id)
    }

    /// Map a [`StoreError`] into a [`CredentialServiceError`] without ever
    /// embedding secret material (store errors carry ids/versions only).
    fn map_store_err(err: StoreError) -> CredentialServiceError {
        match err {
            StoreError::NotFound { id } => CredentialServiceError::NotFound { id },
            StoreError::VersionConflict {
                id,
                expected,
                actual,
            } => CredentialServiceError::VersionConflict {
                id,
                expected,
                actual,
            },
            StoreError::AlreadyExists { id } => {
                CredentialServiceError::Store(format!("credential already exists: {id}"))
            },
            StoreError::AuditFailure(msg) => {
                CredentialServiceError::Store(format!("audit sink refused: {msg}"))
            },
            StoreError::Backend(e) => CredentialServiceError::Store(e.to_string()),
            // `StoreError` is `#[non_exhaustive]`; a future variant maps
            // to the generic store category until it earns a dedicated
            // `CredentialServiceError` arm. Never embeds secret data
            // (store errors carry ids/versions only).
            other => CredentialServiceError::Store(other.to_string()),
        }
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    //! One-call in-memory `CredentialService` for tests / Plan-3 wiring:
    //! `StaticKeyProvider` + `InMemoryStore` + `InMemoryPendingStore` +
    //! a no-op `AuditSink` + the three first-party builtins registered
    //! into the registry/dispatch/ops + `NoopObserver`.

    use std::sync::Arc;

    use nebula_credential::store::StoreError;
    use nebula_credential::{CredentialRegistry, EncryptionKey, InMemoryPendingStore};
    use nebula_credential_builtin::{
        BearerTokenCredential, SharedKeyCredential, SigningKeyCredential, register_builtins,
    };
    use nebula_engine::credential::LeaseLifecycleConfig;
    use nebula_storage::credential::{
        AuditEvent, AuditSink, CacheConfig, InMemoryStore, StaticKeyProvider,
    };
    use tokio_util::sync::CancellationToken;

    use super::CredentialService;
    use crate::builder::CredentialServiceBuilder;
    use crate::dispatch::CredentialDispatch;
    use crate::observer::NoopObserver;
    use crate::ops::{DispatchOps, register_runtime_ops};

    /// No-op audit sink — accepts every event (tests assert behavior via
    /// the store, not the audit trail).
    #[derive(Debug)]
    struct NoopAuditSink;

    impl AuditSink for NoopAuditSink {
        fn record(&self, _event: &AuditEvent) -> Result<(), StoreError> {
            Ok(())
        }
    }

    /// Build an in-memory service with the three first-party builtins
    /// wired through registry + dispatch + ops.
    pub(crate) fn in_memory_service() -> CredentialService<InMemoryStore, InMemoryPendingStore> {
        let mut registry = CredentialRegistry::new();
        register_builtins(&mut registry).expect("register_builtins");

        let mut dispatch = CredentialDispatch::new();
        dispatch
            .register::<BearerTokenCredential>()
            .expect("dispatch bearer");
        dispatch
            .register::<SharedKeyCredential>()
            .expect("dispatch shared");
        dispatch
            .register::<SigningKeyCredential>()
            .expect("dispatch signing");

        let mut ops = DispatchOps::<InMemoryStore, InMemoryPendingStore>::new();
        register_runtime_ops::<BearerTokenCredential, InMemoryStore, InMemoryPendingStore>(
            &mut ops,
        )
        .expect("ops bearer");
        register_runtime_ops::<SharedKeyCredential, InMemoryStore, InMemoryPendingStore>(&mut ops)
            .expect("ops shared");
        register_runtime_ops::<SigningKeyCredential, InMemoryStore, InMemoryPendingStore>(&mut ops)
            .expect("ops signing");

        let key = Arc::new(EncryptionKey::from_bytes([0x42; 32]));
        CredentialServiceBuilder::new(
            InMemoryStore::new(),
            Arc::new(StaticKeyProvider::new(key)),
            Arc::new(NoopAuditSink),
            CacheConfig::default(),
            InMemoryPendingStore::new(),
            Arc::new(registry),
            Arc::new(dispatch),
            Arc::new(ops),
            Arc::new(NoopObserver),
            LeaseLifecycleConfig::default(),
            CancellationToken::new(),
        )
        .build()
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::in_memory_service;
    use crate::CredentialServiceError;
    use crate::scope::TenantScope;

    #[tokio::test]
    async fn create_then_get_roundtrip_is_tenant_scoped() {
        let svc = in_memory_service();
        let scope = TenantScope::new("org1", "ws1");
        let snap = svc
            .create(
                &scope,
                "bearer_token",
                serde_json::json!({ "token": "sk-secret-1" }),
            )
            .await
            .expect("create ok");
        assert_eq!(snap.kind(), "bearer_token");

        // The id round-trips: get with the same scope returns the row.
        let ids = svc.list(&scope).await.expect("list ok");
        assert_eq!(ids.len(), 1);
        let got = svc.get(&scope, &ids[0]).await.expect("get ok");
        assert_eq!(got.kind(), "bearer_token");
        // Secret never appears in the snapshot's Debug.
        assert!(!format!("{got:?}").contains("sk-secret-1"));
    }

    #[tokio::test]
    async fn cross_tenant_get_returns_not_found() {
        let svc = in_memory_service();
        let owner = TenantScope::new("org1", "ws1");
        svc.create(
            &owner,
            "bearer_token",
            serde_json::json!({ "token": "sk-secret-2" }),
        )
        .await
        .expect("create ok");
        let ids = svc.list(&owner).await.expect("list ok");
        let id = &ids[0];

        // A different tenant cannot see the credential at all.
        let other = TenantScope::new("org1", "ws2");
        let err = svc.get(&other, id).await.expect_err("must be denied");
        assert!(matches!(err, CredentialServiceError::NotFound { .. }));
        // And it is invisible to the other tenant's list.
        assert!(svc.list(&other).await.expect("list ok").is_empty());
    }

    #[tokio::test]
    async fn expr_injection_in_props_is_rejected() {
        let svc = in_memory_service();
        let scope = TenantScope::new("org1", "ws1");
        let err = svc
            .create(
                &scope,
                "bearer_token",
                serde_json::json!({ "token": { "$expr": "{{ $execution.id }}" } }),
            )
            .await
            .expect_err("expr must be rejected");
        assert!(matches!(
            err,
            CredentialServiceError::ValidationFailed { .. }
        ));
    }

    #[tokio::test]
    async fn create_unknown_type_is_type_unknown() {
        let svc = in_memory_service();
        let scope = TenantScope::new("org1", "ws1");
        let err = svc
            .create(&scope, "no_such_type", serde_json::json!({}))
            .await
            .expect_err("unknown type");
        assert!(matches!(err, CredentialServiceError::TypeUnknown { .. }));
    }

    #[tokio::test]
    async fn update_with_stale_version_is_version_conflict() {
        let svc = in_memory_service();
        let scope = TenantScope::new("org1", "ws1");
        svc.create(
            &scope,
            "bearer_token",
            serde_json::json!({ "token": "sk-v1" }),
        )
        .await
        .expect("create ok");
        let id = svc.list(&scope).await.expect("list")[0].clone();

        // Stored version after CreateOnly is 1; a stale expected_version
        // of 99 must conflict.
        let err = svc
            .update(&scope, &id, serde_json::json!({ "token": "sk-v2" }), 99)
            .await
            .expect_err("stale version");
        assert!(matches!(
            err,
            CredentialServiceError::VersionConflict { .. }
        ));
    }

    #[tokio::test]
    async fn update_then_get_reflects_new_state_and_delete_removes() {
        let svc = in_memory_service();
        let scope = TenantScope::new("org1", "ws1");
        svc.create(
            &scope,
            "bearer_token",
            serde_json::json!({ "token": "sk-old" }),
        )
        .await
        .expect("create ok");
        let id = svc.list(&scope).await.expect("list")[0].clone();

        svc.update(&scope, &id, serde_json::json!({ "token": "sk-new" }), 1)
            .await
            .expect("update ok");
        // Still resolvable post-update.
        let got = svc.get(&scope, &id).await.expect("get ok");
        assert_eq!(got.kind(), "bearer_token");

        svc.delete(&scope, &id).await.expect("delete ok");
        let err = svc.get(&scope, &id).await.expect_err("gone");
        assert!(matches!(err, CredentialServiceError::NotFound { .. }));
    }

    #[tokio::test]
    async fn cross_tenant_delete_and_update_are_not_found() {
        let svc = in_memory_service();
        let owner = TenantScope::new("org1", "ws1");
        svc.create(
            &owner,
            "bearer_token",
            serde_json::json!({ "token": "sk-x" }),
        )
        .await
        .expect("create ok");
        let id = svc.list(&owner).await.expect("list")[0].clone();

        let other = TenantScope::new("org9", "ws9");
        assert!(matches!(
            svc.delete(&other, &id).await.expect_err("denied"),
            CredentialServiceError::NotFound { .. }
        ));
        assert!(matches!(
            svc.update(&other, &id, serde_json::json!({ "token": "z" }), 1)
                .await
                .expect_err("denied"),
            CredentialServiceError::NotFound { .. }
        ));
    }
}
