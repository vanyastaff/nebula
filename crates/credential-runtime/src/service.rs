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
use std::time::Duration;

use nebula_credential::pending_store::PendingStateStore;
use nebula_credential::resolve::{InteractionRequest, TestResult, UserInput};
use nebula_credential::store::{CredentialStore, PutMode, StoreError, StoredCredential};
use nebula_credential::{
    AuthPattern, CredentialContext, CredentialId, CredentialRecord, CredentialRegistry,
    CredentialSnapshot, PendingToken,
};
use nebula_engine::credential::{CredentialResolver, LeaseLifecycle};
use nebula_resilience::CallError;
use nebula_resilience::retry::{BackoffConfig, RetryConfig, retry_with};
use nebula_schema::FieldValues;
use nebula_storage::credential::{AuditLayer, CacheLayer, EncryptionLayer};
use serde::Serialize;
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

/// Outcome of [`CredentialService::test`] — a secret-free health-probe
/// summary. `message` carries only the provider's failure reason (never
/// secret material); `Debug` is derived because no field can hold a
/// secret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TestReport {
    /// `true` when the provider accepted the credential.
    pub ok: bool,
    /// Provider-supplied failure reason when `ok` is `false`.
    pub message: Option<String>,
}

/// Outcome of [`CredentialService::resolve`] /
/// [`CredentialService::continue_resolve`]. Secret-free: the `Complete`
/// arm carries the redacting [`CredentialSnapshot`]; the `Pending` arm
/// carries the opaque token string + the UI instruction.
#[derive(Debug)]
#[non_exhaustive]
pub enum Acquisition {
    /// Resolved synchronously and persisted.
    Complete {
        /// Secret-free snapshot of the just-persisted credential.
        snapshot: CredentialSnapshot,
    },
    /// Interactive acquisition kicked off; resume via
    /// [`continue_resolve`](CredentialService::continue_resolve) with
    /// `token`.
    Pending {
        /// Opaque pending-acquisition token (round-trips as a string).
        token: String,
        /// What the UI must show / do to complete the flow.
        interaction: InteractionRequest,
    },
    /// Framework asked to poll the continuation again after `after`.
    Retry {
        /// Delay before the next continuation poll.
        after: Duration,
    },
}

/// Capability surface of a credential type, sourced from the dispatch
/// table (closure presence), not self-attested metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct TypeCapabilities {
    /// Type implements `Refreshable`.
    pub refreshable: bool,
    /// Type implements `Testable`.
    pub testable: bool,
    /// Type implements `Revocable`.
    pub revocable: bool,
}

/// Secret-free descriptor of a registered credential type for discovery
/// UIs / pickers. Projected from [`CredentialMetadata`] +
/// [`CredentialDispatch`](crate::dispatch::CredentialDispatch).
///
/// [`CredentialMetadata`]: nebula_credential::CredentialMetadata
#[derive(Debug, Clone, Serialize)]
pub struct CredentialTypeInfo {
    /// `Credential::KEY` (normalized type key).
    pub key: String,
    /// Human-readable type name.
    pub name: String,
    /// Human-readable type description.
    pub description: String,
    /// Authentication-pattern classification.
    pub pattern: AuthPattern,
    /// Which lifecycle capabilities the type supports.
    pub capabilities: TypeCapabilities,
}

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

    /// Active dynamic-lease count — a test-only smoke accessor. Gated
    /// `cfg(any(test, feature = "test-util"))` so it is **not** part of
    /// the stable public surface of this security-critical facade
    /// (lease-count introspection is a test affordance, not an API).
    #[cfg(any(test, feature = "test-util"))]
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
        let ctx = Self::owner_context(scope);

        let resolved = self
            .ops
            .resolve(credential_key, &values, &ctx, &self.pending)
            .await?;

        let snapshot = self
            .persist_resolved(scope, credential_key, id, resolved)
            .await?;

        self.observer.on_resolve(&id);
        tracing::info!(
            credential.key = credential_key,
            credential.id = %id,
            "credential created"
        );

        Ok(snapshot)
    }

    /// Persist a freshly-resolved credential under `id` scoped to
    /// `scope`, returning the secret-free snapshot projected from the
    /// just-resolved bytes (no decrypt round-trip). Shared by [`create`]
    /// and the synchronous-`Complete` arm of [`resolve`].
    ///
    /// [`create`]: Self::create
    /// [`resolve`]: Self::resolve
    async fn persist_resolved(
        &self,
        scope: &TenantScope,
        credential_key: &str,
        id: CredentialId,
        resolved: crate::ops::ResolvedState,
    ) -> Result<CredentialSnapshot, CredentialServiceError> {
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
    /// # Performance contract
    ///
    /// This is **O(N) in the global credential count**, not in the
    /// caller's tenant size: it enumerates every stored id and does one
    /// `get` (one decrypt) per row to read the `owner_id` stamp, because
    /// the build-once layered stack omits the storage `ScopeLayer` and
    /// tenancy is enforced at the operation level. That is acceptable for
    /// the non-durable in-memory backend (the only backend that ships
    /// with this facade today). Owner-scoped listing for the durable
    /// backends belongs in the **store layer** (an indexed,
    /// owner-filtered query), not a facade-side scan — a conscious
    /// deferral, not an oversight.
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

        let ctx = Self::owner_context(scope);
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

    // ── Capability operations (test / refresh / revoke) ──────────────

    /// Run the credential type's provider health probe.
    ///
    /// Owner-checked first (a cross-tenant id is
    /// [`NotFound`](CredentialServiceError::NotFound), never a capability
    /// leak). If the type is not testable the call fails with
    /// [`CapabilityUnsupported`](CredentialServiceError::CapabilityUnsupported)
    /// **before** any decrypt — a static type cannot be probed.
    ///
    /// # Errors
    ///
    /// - [`CredentialServiceError::NotFound`] — absent or cross-tenant id.
    /// - [`CredentialServiceError::CapabilityUnsupported`] — type is not `Testable`.
    /// - [`CredentialServiceError::Provider`] — the probe itself failed.
    pub async fn test(
        &self,
        scope: &TenantScope,
        id: &str,
    ) -> Result<TestReport, CredentialServiceError> {
        let stored = self.load_owned(scope, id).await?;
        if !self.dispatch.is_testable(&stored.credential_key) {
            return Err(CredentialServiceError::CapabilityUnsupported {
                capability: "test".to_owned(),
                key: stored.credential_key.clone(),
            });
        }
        let ctx = Self::owner_context(scope);
        let result = self
            .ops
            .test(&stored.credential_key, &stored.data, &ctx)
            .await?;
        let report = match result {
            TestResult::Success => TestReport {
                ok: true,
                message: None,
            },
            TestResult::Failed { reason } => TestReport {
                ok: false,
                message: Some(reason),
            },
            // `TestResult` is `#[non_exhaustive]`; an unrecognized future
            // variant is not provably a success — report not-ok so a new
            // outcome never silently presents as a passing probe.
            other => TestReport {
                ok: false,
                message: Some(format!("unrecognized test outcome: {other:?}")),
            },
        };
        tracing::info!(credential.id = %id, ok = report.ok, "credential tested");
        Ok(report)
    }

    /// Force-refresh the credential's stored state and re-persist it.
    ///
    /// Owner-checked first. The refresh runs through
    /// [`nebula_resilience::retry_with`] (3 attempts, exponential
    /// backoff). If this caller performed the refresh the resulting state
    /// is written back under compare-and-swap on the version observed at
    /// load; a concurrent refresh/update wins and this attempt fails
    /// explicitly with [`CredentialServiceError::VersionConflict`] —
    /// canon §13.2: refresh must never silently strand a concurrent
    /// write. If another replica coalesced the refresh
    /// (`RefreshOutcome::CoalescedByOtherReplica`) the write is **skipped
    /// entirely** and the now-fresher state is re-read from the store
    /// instead of clobbering it with the un-mutated local copy. On
    /// success (either path) [`CredentialObserver::on_refresh`] fires and
    /// the fresh secret-free snapshot is returned.
    ///
    /// # Errors
    ///
    /// - [`CredentialServiceError::NotFound`] — absent or cross-tenant id.
    /// - [`CredentialServiceError::CapabilityUnsupported`] — type is not `Refreshable`.
    /// - [`CredentialServiceError::Provider`] — refresh failed after retries.
    /// - [`CredentialServiceError::VersionConflict`] — a concurrent write landed first.
    /// - [`CredentialServiceError::Store`] — re-persist failed.
    pub async fn refresh(
        &self,
        scope: &TenantScope,
        id: &str,
    ) -> Result<CredentialSnapshot, CredentialServiceError> {
        let stored = self.load_owned(scope, id).await?;
        if !self.dispatch.is_refreshable(&stored.credential_key) {
            return Err(CredentialServiceError::CapabilityUnsupported {
                capability: "refresh".to_owned(),
                key: stored.credential_key.clone(),
            });
        }
        let ctx = Self::owner_context(scope);

        let config = RetryConfig::<CredentialServiceError>::new(3)
            .map_err(|e| CredentialServiceError::Internal(format!("retry config invalid: {e}")))?
            .backoff(BackoffConfig::Exponential {
                base: Duration::from_millis(200),
                multiplier: 2.0,
                max: Duration::from_secs(5),
            });

        let outcome = retry_with(config, || async {
            self.ops
                .refresh(&stored.credential_key, &stored.data, &ctx)
                .await
        })
        .await
        .map_err(|call_err| match call_err {
            CallError::Operation(e) | CallError::RetriesExhausted { last: e, .. } => e,
            other => {
                CredentialServiceError::Provider(format!("credential refresh failed: {other}"))
            },
        })?;

        let refreshed = match outcome {
            crate::ops::RefreshOutcomeKind::Rewrote(bytes) => bytes,
            // Another replica already refreshed and persisted fresher
            // state. Re-writing the un-mutated local copy here would
            // either spuriously `VersionConflict` or clobber that fresher
            // state (canon §13.2). Skip the write entirely and return the
            // store's current (post-coalesce) snapshot.
            crate::ops::RefreshOutcomeKind::CoalescedReRead => {
                let credential_id = CredentialId::parse(&stored.id).map_err(|e| {
                    CredentialServiceError::Internal(format!(
                        "stored credential id unparsable: {e}"
                    ))
                })?;
                self.observer.on_refresh(&credential_id);
                tracing::info!(
                    credential.id = %id,
                    "credential refresh coalesced by another replica; re-reading without re-writing"
                );
                return self.snapshot_from_store(scope, id).await;
            },
        };

        let now = chrono::Utc::now();
        let state_kind = stored.state_kind.clone();
        let state_version = stored.state_version;
        let stored_next = StoredCredential {
            id: stored.id.clone(),
            credential_key: stored.credential_key.clone(),
            data: refreshed.to_vec(),
            state_kind,
            state_version,
            version: stored.version,
            created_at: stored.created_at,
            updated_at: now,
            expires_at: stored.expires_at,
            reauth_required: false,
            metadata: stored.metadata.clone(),
        };
        // Re-persist under compare-and-swap on the version observed at
        // load. A concurrent refresh/update that landed in between wins
        // and this attempt fails *explicitly* with `VersionConflict`
        // (canon §13.2: refresh must never silently strand a concurrent
        // write; failure is explicit). Blind `Overwrite` here would
        // last-writer-wins and clobber the racing write.
        self.store
            .put(
                stored_next,
                PutMode::CompareAndSwap {
                    expected_version: stored.version,
                },
            )
            .await
            .map_err(Self::map_store_err)?;

        let credential_id = CredentialId::parse(&stored.id).map_err(|e| {
            CredentialServiceError::Internal(format!("stored credential id unparsable: {e}"))
        })?;
        self.observer.on_refresh(&credential_id);
        tracing::info!(credential.id = %id, "credential refreshed");
        self.snapshot_from_store(scope, id).await
    }

    /// Revoke the credential at the provider, release any leases, and
    /// delete the stored row.
    ///
    /// Owner-checked first. The provider-side revoke runs the type's
    /// `Revocable::revoke`; lease release is best-effort (a failure is
    /// logged, not propagated — the credential is still revoked); the
    /// stored row is then deleted per the revoke contract. On success
    /// [`CredentialObserver::on_revoke`] fires.
    ///
    /// # Errors
    ///
    /// - [`CredentialServiceError::NotFound`] — absent or cross-tenant id.
    /// - [`CredentialServiceError::CapabilityUnsupported`] — type is not `Revocable`.
    /// - [`CredentialServiceError::Provider`] — the provider revoke failed.
    /// - [`CredentialServiceError::Store`] — deleting the row failed.
    pub async fn revoke(
        &self,
        scope: &TenantScope,
        id: &str,
    ) -> Result<(), CredentialServiceError> {
        let stored = self.load_owned(scope, id).await?;
        if !self.dispatch.is_revocable(&stored.credential_key) {
            return Err(CredentialServiceError::CapabilityUnsupported {
                capability: "revoke".to_owned(),
                key: stored.credential_key.clone(),
            });
        }
        let ctx = Self::owner_context(scope);
        self.ops
            .revoke(&stored.credential_key, &stored.data, &ctx)
            .await?;

        let credential_id = CredentialId::parse(&stored.id).map_err(|e| {
            CredentialServiceError::Internal(format!("stored credential id unparsable: {e}"))
        })?;

        // Best-effort lease release: a credential whose provider-side
        // secret is revoked must not keep dynamic leases alive, but a
        // lease-subsystem hiccup must not block the revoke itself (the
        // secret is already dead at the provider).
        let released = self.lease.revoke_for_credential(credential_id).await;
        if released > 0 {
            tracing::info!(
                credential.id = %id,
                released,
                "released dynamic leases for revoked credential"
            );
        }

        // Delete the stored row per the revoke contract — a revoked
        // credential is gone, not a stale row.
        self.store.delete(id).await.map_err(Self::map_store_err)?;

        self.observer.on_revoke(&credential_id);
        tracing::info!(credential.id = %id, "credential revoked");
        Ok(())
    }

    // ── Acquisition (resolve / continue) ─────────────────────────────

    /// Acquire a credential of `credential_key` from `props`, persisting
    /// it on synchronous completion or surfacing an interaction token for
    /// interactive flows.
    ///
    /// Validation is the canonical credential pipeline (the `$expr`
    /// refusal point, canon §12.5). A `Complete` resolution is persisted
    /// through the same path as [`create`](Self::create) and returned as
    /// [`Acquisition::Complete`]; a `Pending` kickoff returns
    /// [`Acquisition::Pending`] with the opaque token + UI instruction.
    ///
    /// # Errors
    ///
    /// - [`CredentialServiceError::TypeUnknown`] — key not registered.
    /// - [`CredentialServiceError::ValidationFailed`] — schema / typed-deserialize / resolve.
    /// - [`CredentialServiceError::SessionRequired`] — the resolution
    ///   went `Pending` (interactive kickoff) but `scope` carries no
    ///   session, so the issued token could never be redeemed.
    /// - [`CredentialServiceError::Store`] — persistence failure on the `Complete` path.
    pub async fn resolve(
        &self,
        scope: &TenantScope,
        credential_key: &str,
        props: Value,
    ) -> Result<Acquisition, CredentialServiceError> {
        if !self.dispatch.contains(credential_key) {
            return Err(CredentialServiceError::TypeUnknown {
                key: credential_key.to_owned(),
            });
        }
        self.ops.validate(credential_key, &props)?;
        let values = FieldValues::from_json(props).map_err(|e| {
            CredentialServiceError::ValidationFailed {
                reason: format!("property ingest failed: {e}"),
            }
        })?;
        let ctx = Self::owner_context(scope);
        let outcome = self
            .ops
            .acquire(credential_key, &values, &ctx, &self.pending)
            .await?;
        self.finish_acquire(scope, credential_key, outcome).await
    }

    /// Continue an interactive acquisition with the user's input.
    ///
    /// Threads the service's pending store through the engine's
    /// `execute_continue` for the concrete interactive type. The three
    /// first-party builtins are non-interactive, so no continuation
    /// closure is registered for them and this returns
    /// [`CapabilityUnsupported`](CredentialServiceError::CapabilityUnsupported)
    /// (or [`TypeUnknown`](CredentialServiceError::TypeUnknown) for an
    /// unregistered key).
    ///
    /// # Errors
    ///
    /// - [`CredentialServiceError::TypeUnknown`] — key not registered.
    /// - [`CredentialServiceError::SessionRequired`] — `scope` carries no
    ///   session; the pending-store binding makes a continuation
    ///   structurally impossible without one.
    /// - [`CredentialServiceError::CapabilityUnsupported`] — type is not `Interactive`.
    /// - [`CredentialServiceError::ValidationFailed`] — continuation failed.
    /// - [`CredentialServiceError::Store`] — persistence failure on the `Complete` path.
    pub async fn continue_resolve(
        &self,
        scope: &TenantScope,
        credential_key: &str,
        pending_token: &str,
        user_input: UserInput,
    ) -> Result<Acquisition, CredentialServiceError> {
        if !self.dispatch.contains(credential_key) {
            return Err(CredentialServiceError::TypeUnknown {
                key: credential_key.to_owned(),
            });
        }
        // A continuation is structurally dead without a session: the
        // engine's `execute_continue` requires `ctx.session_id()` and the
        // `PendingStateStore` binds the pending on
        // `(kind, owner, session, token)`. Surface that explicitly here
        // rather than letting it collapse into a misleading
        // `ValidationFailed` deep inside the executor.
        if scope.session_id().is_none() {
            return Err(CredentialServiceError::SessionRequired {
                capability: "continue",
            });
        }
        // `PendingToken` has no public string constructor; its
        // documented wire form is a bare JSON string (see its
        // serde round-trip contract), so reconstruct the client-returned
        // token through serde — the only public inbound path.
        let token: PendingToken = serde_json::from_value(Value::String(pending_token.to_owned()))
            .map_err(|_| CredentialServiceError::ValidationFailed {
            reason: "malformed pending acquisition token".to_owned(),
        })?;
        let ctx = Self::owner_context(scope);
        let outcome = self
            .ops
            .continue_resolve(credential_key, &token, &user_input, &ctx, &self.pending)
            .await?;
        self.finish_acquire(scope, credential_key, outcome).await
    }

    /// Map an [`AcquireOutcome`] into the public [`Acquisition`]:
    /// `Complete` is persisted (shared create path); `Pending`/`Retry`
    /// surface the token + interaction without persisting.
    async fn finish_acquire(
        &self,
        scope: &TenantScope,
        credential_key: &str,
        outcome: crate::ops::AcquireOutcome,
    ) -> Result<Acquisition, CredentialServiceError> {
        match outcome {
            crate::ops::AcquireOutcome::Complete(resolved) => {
                let id = CredentialId::new();
                let snapshot = self
                    .persist_resolved(scope, credential_key, id, resolved)
                    .await?;
                self.observer.on_resolve(&id);
                tracing::info!(
                    credential.key = credential_key,
                    credential.id = %id,
                    "credential acquired"
                );
                Ok(Acquisition::Complete { snapshot })
            },
            crate::ops::AcquireOutcome::Pending { token, interaction } => {
                // The interaction can only be completed through
                // `continue_resolve`, which the engine binds on
                // `(kind, owner, session, token)`. Without a session on
                // the scope the issued token is unusable, so refuse the
                // kickoff explicitly instead of handing back a token that
                // can never be redeemed.
                if scope.session_id().is_none() {
                    return Err(CredentialServiceError::SessionRequired {
                        capability: "resolve",
                    });
                }
                Ok(Acquisition::Pending {
                    token: token.as_str().to_owned(),
                    interaction,
                })
            },
            crate::ops::AcquireOutcome::Retry { after } => Ok(Acquisition::Retry { after }),
        }
    }

    // ── Type discovery ───────────────────────────────────────────────

    /// List every registered credential type as a secret-free
    /// descriptor. Capability flags come from the
    /// [`CredentialDispatch`](crate::dispatch::CredentialDispatch) table
    /// (closure presence), not self-attested metadata.
    #[must_use]
    pub fn list_types(&self) -> Vec<CredentialTypeInfo> {
        self.registry
            .iter_compatible(nebula_credential::Capabilities::empty())
            .filter_map(|(key, _caps)| self.type_info(key))
            .collect()
    }

    /// Project a single credential type's descriptor, or `None` when the
    /// key is not registered.
    #[must_use]
    pub fn get_type(&self, key: &str) -> Option<CredentialTypeInfo> {
        if !self.registry.contains(key) {
            return None;
        }
        self.type_info(key)
    }

    /// Build a [`CredentialTypeInfo`] from the registry metadata +
    /// dispatch capability flags. Returns `None` if the registry has no
    /// instance for `key` (cannot project metadata).
    fn type_info(&self, key: &str) -> Option<CredentialTypeInfo> {
        let metadata = self.registry.resolve_any(key)?.metadata();
        Some(CredentialTypeInfo {
            key: metadata.base.key.as_str().to_owned(),
            name: metadata.base.name.clone(),
            description: metadata.base.description.clone(),
            pattern: metadata.pattern,
            capabilities: TypeCapabilities {
                refreshable: self.dispatch.is_refreshable(key),
                testable: self.dispatch.is_testable(key),
                revocable: self.dispatch.is_revocable(key),
            },
        })
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
    /// pipeline needs.
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
    ///
    /// When the scope carries a session it is threaded onto the context
    /// via `with_session_id`: the engine's `execute_continue` (and the
    /// `PendingStateStore` `(kind, owner, session, token)` binding) reads
    /// `ctx.session_id()`, so without this the interactive paths would
    /// always fail `MissingSessionId`. CRUD passes a session-less scope
    /// and the accessors ignore the (absent) session.
    fn owner_context(scope: &TenantScope) -> CredentialContext {
        let ctx = CredentialContext::for_test(scope.owner_id());
        match scope.session_id() {
            Some(session) => ctx.with_session_id(session),
            None => ctx,
        }
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

#[cfg(any(test, feature = "test-util"))]
pub mod test_support {
    //! In-memory `CredentialService` wiring for unit tests, the
    //! adversarial integration suite, and Plan-3 consumers. One call
    //! assembles a `StaticKeyProvider`, an `InMemoryStore`, an
    //! `InMemoryPendingStore`, an `AuditSink` (no-op by default), the
    //! three first-party builtins registered into the
    //! registry/dispatch/ops, and a `NoopObserver`.
    //!
    //! # ADR-0023
    //!
    //! This module is gated `cfg(any(test, feature = "test-util"))`. The
    //! `test-util` feature MUST NOT be enabled in a release/production
    //! build: it forwards the storage in-memory test backends and wires
    //! a `StaticKeyProvider` (fixed key) over a non-durable
    //! `InMemoryStore`. `unwrap`/`expect` below is acceptable —
    //! `test_support` is test-support code, never a release path.

    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use nebula_core::accessor::MetricsEmitter;
    use nebula_credential::provider::LeaseEvent;
    use nebula_credential::store::StoreError;
    use nebula_credential::{
        CredentialEvent, CredentialId, CredentialRegistry, EncryptionKey, InMemoryPendingStore,
    };
    use nebula_credential_builtin::{
        BearerTokenCredential, SharedKeyCredential, SigningKeyCredential, register_builtins,
    };
    use nebula_engine::credential::LeaseLifecycleConfig;
    use nebula_eventbus::EventBus;
    use nebula_storage::credential::{
        AuditEvent, AuditSink, CacheConfig, InMemoryStore, StaticKeyProvider,
    };
    use tokio_util::sync::CancellationToken;

    use super::CredentialService;
    use crate::builder::CredentialServiceBuilder;
    use crate::dispatch::CredentialDispatch;
    use crate::observer::{CredentialObserver, NoopObserver};
    use crate::ops::{
        DispatchOps, register_all_builtin_ops, register_refreshable_ops, register_runtime_ops,
    };
    use crate::test_fixtures::RefreshableFixtureCredential;

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
    /// wired through registry + dispatch + ops, accepting an arbitrary
    /// [`AuditSink`], **and** return a `Clone` of the raw `InMemoryStore`
    /// that shares the service's backing map.
    ///
    /// The raw handle is a structural read-back seam for the audit
    /// fail-closed invariant (spec §6 #8): with a refusing sink every
    /// store op through the layered stack also fails closed, so the row
    /// cannot be observed through the facade — the raw handle bypasses
    /// the poisoned `AuditLayer` to prove the write did not partially
    /// land. The secure layered composition is unchanged; only the audit
    /// sink varies and the inner store is observable for assertions.
    pub fn service_and_raw_store_with_audit_sink(
        audit_sink: Arc<dyn AuditSink>,
    ) -> (
        CredentialService<InMemoryStore, InMemoryPendingStore>,
        InMemoryStore,
    ) {
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

        // All three builtins are static (no capability impls), so only
        // the base ops are registered — no `register_*_ops` capability
        // call. Closure absence is "capability not supported".
        let mut ops = DispatchOps::<InMemoryStore, InMemoryPendingStore>::new();
        register_all_builtin_ops::<InMemoryStore, InMemoryPendingStore>(&mut ops)
            .expect("builtin ops");

        // `InMemoryStore` is `Arc<RwLock<..>>`-backed: the clone shares
        // the same map the layered stack writes through.
        let raw_store = InMemoryStore::new();
        let key = Arc::new(EncryptionKey::from_bytes([0x42; 32]));
        let svc = CredentialServiceBuilder::new(
            raw_store.clone(),
            Arc::new(StaticKeyProvider::new(key)),
            audit_sink,
            CacheConfig::default(),
            InMemoryPendingStore::new(),
            Arc::new(registry),
            Arc::new(dispatch),
            Arc::new(ops),
            Arc::new(NoopObserver),
            LeaseLifecycleConfig::default(),
            CancellationToken::new(),
        )
        .build();
        (svc, raw_store)
    }

    /// Build an in-memory service with an arbitrary [`AuditSink`],
    /// discarding the raw-store handle. Convenience over
    /// [`service_and_raw_store_with_audit_sink`] for tests that do not
    /// need to inspect the inner store.
    pub fn service_with_audit_sink(
        audit_sink: Arc<dyn AuditSink>,
    ) -> CredentialService<InMemoryStore, InMemoryPendingStore> {
        service_and_raw_store_with_audit_sink(audit_sink).0
    }

    /// Build an in-memory service with a no-op audit sink — the default
    /// fixture for tests / Plan-3 wiring.
    pub fn in_memory_service() -> CredentialService<InMemoryStore, InMemoryPendingStore> {
        service_with_audit_sink(Arc::new(NoopAuditSink))
    }

    /// Observer that counts `on_refresh` invocations so a test can prove
    /// the facade fired the refresh hook on the success path. Event/lease
    /// buses are inert (a real `CredentialResolver` still needs a bus, so
    /// one is provided, but nothing subscribes).
    #[derive(Debug, Default)]
    struct CountingObserver {
        refreshes: Arc<AtomicUsize>,
    }

    impl CredentialObserver for CountingObserver {
        fn event_bus(&self) -> Arc<EventBus<CredentialEvent>> {
            Arc::new(EventBus::new(1))
        }
        fn lease_bus(&self) -> Option<Arc<EventBus<LeaseEvent>>> {
            None
        }
        fn metrics(&self) -> Option<Arc<dyn MetricsEmitter>> {
            None
        }
        fn on_resolve(&self, _credential_id: &CredentialId) {}
        fn on_refresh(&self, _credential_id: &CredentialId) {
            self.refreshes.fetch_add(1, Ordering::SeqCst);
        }
        fn on_revoke(&self, _credential_id: &CredentialId) {}
    }

    /// Build an in-memory service with the three static builtins **plus**
    /// the [`RefreshableFixtureCredential`] wired through registry +
    /// dispatch (`mark_refreshable`) + ops (`register_runtime_ops` then
    /// `register_refreshable_ops`). Returns the service alongside an
    /// `Arc<AtomicUsize>` that counts `on_refresh` calls, so a test can
    /// prove the success path fired the observer hook.
    ///
    /// The static builtins cannot exercise any positive capability path
    /// (refresh CAS + retry, the coalesced re-read branch, the §13.2
    /// version-conflict branch); this fixture-enabled variant does.
    pub fn in_memory_service_with_fixtures() -> (
        CredentialService<InMemoryStore, InMemoryPendingStore>,
        Arc<AtomicUsize>,
    ) {
        let mut registry = CredentialRegistry::new();
        register_builtins(&mut registry).expect("register_builtins");
        registry
            .register(RefreshableFixtureCredential, "nebula-credential-runtime")
            .expect("register fixture");

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
        dispatch
            .register::<RefreshableFixtureCredential>()
            .expect("dispatch fixture");
        // Closure presence *is* the capability; the dispatch flag mirrors
        // it so `is_refreshable` agrees with the registered ops.
        dispatch.mark_refreshable::<RefreshableFixtureCredential>();

        let mut ops = DispatchOps::<InMemoryStore, InMemoryPendingStore>::new();
        register_all_builtin_ops::<InMemoryStore, InMemoryPendingStore>(&mut ops)
            .expect("builtin ops");
        // Base ops first, then the refresh capability closure (the
        // ordering `register_refreshable_ops` enforces via
        // `DispatchError::BaseOpsMissing`).
        register_runtime_ops::<RefreshableFixtureCredential, InMemoryStore, InMemoryPendingStore>(
            &mut ops,
        )
        .expect("fixture base ops");
        register_refreshable_ops::<
            RefreshableFixtureCredential,
            InMemoryStore,
            InMemoryPendingStore,
        >(&mut ops)
        .expect("fixture refreshable ops");

        let refreshes = Arc::new(AtomicUsize::new(0));
        let observer = CountingObserver {
            refreshes: Arc::clone(&refreshes),
        };

        let raw_store = InMemoryStore::new();
        let key = Arc::new(EncryptionKey::from_bytes([0x42; 32]));
        let svc = CredentialServiceBuilder::new(
            raw_store,
            Arc::new(StaticKeyProvider::new(key)),
            Arc::new(NoopAuditSink),
            CacheConfig::default(),
            InMemoryPendingStore::new(),
            Arc::new(registry),
            Arc::new(dispatch),
            Arc::new(ops),
            Arc::new(observer),
            LeaseLifecycleConfig::default(),
            CancellationToken::new(),
        )
        .build();
        (svc, refreshes)
    }
}

#[cfg(test)]
mod tests {
    use super::Acquisition;
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

    /// Abuse #4: a static credential type advertises no capability, so
    /// `test` / `refresh` / `revoke` are refused with
    /// `CapabilityUnsupported` (closure-absence = capability-absence).
    #[tokio::test]
    async fn static_type_capability_ops_are_unsupported() {
        let svc = in_memory_service();
        let scope = TenantScope::new("org1", "ws1");
        svc.create(
            &scope,
            "bearer_token",
            serde_json::json!({ "token": "sk-cap" }),
        )
        .await
        .expect("create ok");
        let id = svc.list(&scope).await.expect("list")[0].clone();

        let test_err = svc.test(&scope, &id).await.expect_err("not testable");
        assert!(matches!(
            test_err,
            CredentialServiceError::CapabilityUnsupported { ref capability, .. }
                if capability == "test"
        ));
        let refresh_err = svc.refresh(&scope, &id).await.expect_err("not refreshable");
        assert!(matches!(
            refresh_err,
            CredentialServiceError::CapabilityUnsupported { ref capability, .. }
                if capability == "refresh"
        ));
        let revoke_err = svc.revoke(&scope, &id).await.expect_err("not revocable");
        assert!(matches!(
            revoke_err,
            CredentialServiceError::CapabilityUnsupported { ref capability, .. }
                if capability == "revoke"
        ));
    }

    #[tokio::test]
    async fn resolve_complete_persists_and_is_gettable() {
        let svc = in_memory_service();
        let scope = TenantScope::new("org1", "ws1");
        let acq = svc
            .resolve(
                &scope,
                "bearer_token",
                serde_json::json!({ "token": "sk-acquired" }),
            )
            .await
            .expect("resolve ok");
        let snapshot = match acq {
            Acquisition::Complete { snapshot } => snapshot,
            other => panic!("expected Complete, got {other:?}"),
        };
        assert_eq!(snapshot.kind(), "bearer_token");

        // The acquired credential is now a normal stored credential.
        let ids = svc.list(&scope).await.expect("list ok");
        assert_eq!(ids.len(), 1);
        let got = svc.get(&scope, &ids[0]).await.expect("get ok");
        assert_eq!(got.kind(), "bearer_token");
        assert!(!format!("{got:?}").contains("sk-acquired"));
    }

    #[tokio::test]
    async fn list_types_contains_builtins_with_no_capabilities() {
        let svc = in_memory_service();
        let types = svc.list_types();
        let keys: Vec<&str> = types.iter().map(|t| t.key.as_str()).collect();
        for expected in ["bearer_token", "shared_key", "signing_key"] {
            assert!(keys.contains(&expected), "missing {expected} in {keys:?}");
        }
        for info in &types {
            assert!(
                !info.capabilities.refreshable
                    && !info.capabilities.testable
                    && !info.capabilities.revocable,
                "static builtin {} must advertise no capabilities",
                info.key
            );
        }
        // get_type agrees with list_types for a known key, None otherwise.
        let one = svc.get_type("bearer_token").expect("known type");
        assert_eq!(one.key, "bearer_token");
        assert!(svc.get_type("no_such_type").is_none());
    }

    #[tokio::test]
    async fn cross_tenant_capability_ops_are_not_found() {
        let svc = in_memory_service();
        let owner = TenantScope::new("org1", "ws1");
        svc.create(
            &owner,
            "bearer_token",
            serde_json::json!({ "token": "sk-xt" }),
        )
        .await
        .expect("create ok");
        let id = svc.list(&owner).await.expect("list")[0].clone();

        let other = TenantScope::new("org9", "ws9");
        // A cross-tenant id is reported as missing on every new op —
        // never a capability leak (the owner check runs before the
        // capability gate).
        assert!(matches!(
            svc.test(&other, &id).await.expect_err("denied"),
            CredentialServiceError::NotFound { .. }
        ));
        assert!(matches!(
            svc.refresh(&other, &id).await.expect_err("denied"),
            CredentialServiceError::NotFound { .. }
        ));
        assert!(matches!(
            svc.revoke(&other, &id).await.expect_err("denied"),
            CredentialServiceError::NotFound { .. }
        ));
    }

    #[tokio::test]
    async fn continue_resolve_on_non_interactive_is_unsupported() {
        let svc = in_memory_service();
        // A session is present so the path reaches the capability gate
        // (the session check runs first; see the dedicated test below).
        let scope = TenantScope::new("org1", "ws1").with_session("sess-1");
        // bearer_token is non-interactive: no continuation closure is
        // registered, so the continuation path refuses with a clear
        // capability error rather than a confusing pending-store miss.
        let err = svc
            .continue_resolve(
                &scope,
                "bearer_token",
                "irrelevant-token",
                nebula_credential::resolve::UserInput::Poll,
            )
            .await
            .expect_err("non-interactive");
        assert!(matches!(
            err,
            CredentialServiceError::CapabilityUnsupported { ref capability, .. }
                if capability == "interactive"
        ));
    }

    /// Without a session, `continue_resolve` must fail `SessionRequired`
    /// *before* the dispatch/capability gate: the pending-store
    /// `(kind, owner, session, token)` binding makes a continuation
    /// structurally impossible without one, and the bare session-less
    /// path would otherwise collapse into a silent `ValidationFailed`
    /// inside the engine's `execute_continue`.
    #[tokio::test]
    async fn continue_resolve_without_session_is_session_required() {
        let svc = in_memory_service();
        let scope = TenantScope::new("org1", "ws1"); // no .with_session
        let err = svc
            .continue_resolve(
                &scope,
                "bearer_token",
                "irrelevant-token",
                nebula_credential::resolve::UserInput::Poll,
            )
            .await
            .expect_err("no session");
        assert!(
            matches!(
                err,
                CredentialServiceError::SessionRequired { capability } if capability == "continue"
            ),
            "expected SessionRequired(continue), got {err:?}"
        );
    }

    /// `owner_context` threads the scope's session onto the
    /// `CredentialContext` (so the engine's `execute_continue`, which
    /// reads `ctx.session_id()`, is not structurally dead), and leaves it
    /// `None` for a session-less CRUD scope.
    #[test]
    fn owner_context_threads_session_when_scope_carries_one() {
        use super::CredentialService;
        use nebula_credential::InMemoryPendingStore;
        use nebula_storage::credential::InMemoryStore;

        type Svc = CredentialService<InMemoryStore, InMemoryPendingStore>;

        let with = TenantScope::new("org1", "ws1").with_session("sess-xyz");
        let ctx = Svc::owner_context(&with);
        assert_eq!(ctx.session_id(), Some("sess-xyz"));
        assert_eq!(ctx.owner_id(), "org1/ws1");

        let without = TenantScope::new("org1", "ws1");
        let ctx_none = Svc::owner_context(&without);
        assert_eq!(ctx_none.session_id(), None);
    }

    /// Positive refresh path on the refreshable fixture: `refresh()`
    /// succeeds, the *mutated* (token-rotated, counter-bumped) state is
    /// re-persisted under CAS, and the `on_refresh` observer hook fires
    /// exactly once. The static builtins cannot reach this path (none is
    /// `Refreshable`), so the fixture is the only coverage for it.
    #[tokio::test]
    async fn fixture_refresh_succeeds_repersists_and_fires_on_refresh() {
        let (svc, refreshes) = super::test_support::in_memory_service_with_fixtures();
        let scope = TenantScope::new("org1", "ws1");

        svc.create(
            &scope,
            "refreshable_fixture",
            serde_json::json!({ "token": "tok-base" }),
        )
        .await
        .expect("create fixture ok");
        let id = svc.list(&scope).await.expect("list")[0].clone();

        assert_eq!(refreshes.load(std::sync::atomic::Ordering::SeqCst), 0);
        let snap = svc.refresh(&scope, &id).await.expect("refresh ok");
        assert_eq!(snap.kind(), "refreshable_fixture");
        // on_refresh fired exactly once on the success path.
        assert_eq!(refreshes.load(std::sync::atomic::Ordering::SeqCst), 1);

        // The re-persisted state is the *mutated* one: a second refresh
        // observes the first rotation (counter is 2 → token `tok-base-r2`),
        // proving the CAS write stored the refreshed bytes, not the
        // pre-refresh copy.
        let snap2 = svc.refresh(&scope, &id).await.expect("second refresh ok");
        assert_eq!(snap2.kind(), "refreshable_fixture");
        assert_eq!(refreshes.load(std::sync::atomic::Ordering::SeqCst), 2);
        // Still retrievable and correctly projected post-refresh.
        let got = svc.get(&scope, &id).await.expect("get ok");
        assert_eq!(got.kind(), "refreshable_fixture");
    }

    /// A concurrent version bump landing between refresh's internal load
    /// and its compare-and-swap re-persist makes `refresh()` fail
    /// *explicitly* with `VersionConflict` rather than silently
    /// clobbering the racing write (the canon §13.2 contract).
    ///
    /// Determinism: the fixture's `refresh` parks on a 2-party rendezvous
    /// barrier *after* mutating its local state but *before* the service
    /// performs the CAS. The concurrent writer lands a successful
    /// `update` (bumping the stored version out from under the version
    /// `refresh` loaded) and only then releases the barrier, so the CAS
    /// is guaranteed to observe a stale version on every run.
    #[tokio::test]
    async fn fixture_refresh_loses_cas_to_concurrent_write_is_version_conflict() {
        use crate::test_fixtures::set_refresh_rendezvous;
        use std::sync::Arc;

        let (svc, _refreshes) = super::test_support::in_memory_service_with_fixtures();
        let scope = TenantScope::new("org1", "ws1");

        svc.create(
            &scope,
            "refreshable_fixture",
            serde_json::json!({ "token": "tok-race" }),
        )
        .await
        .expect("create ok");
        let id = svc.list(&scope).await.expect("list")[0].clone();

        // 2 parties: the fixture's parked `refresh` and the writer below.
        let barrier = Arc::new(tokio::sync::Barrier::new(2));
        set_refresh_rendezvous(Some(Arc::clone(&barrier)));

        let svc_ref = &svc;
        let scope_ref = &scope;
        let id_ref = &id;
        let (refresh_res, upd_res) =
            tokio::join!(async { svc_ref.refresh(scope_ref, id_ref).await }, async {
                // The credential is created at stored version 1. Bump it
                // (1 -> 2) while `refresh` is parked at the rendezvous,
                // then release the barrier so refresh proceeds to its CAS
                // on the now-stale version it loaded (1).
                let r = svc_ref
                    .update(
                        scope_ref,
                        id_ref,
                        serde_json::json!({ "token": "tok-race-concurrent" }),
                        1,
                    )
                    .await;
                barrier.wait().await;
                r
            });
        set_refresh_rendezvous(None);

        upd_res.expect("concurrent update (version 1 -> 2) must succeed");
        let err = refresh_res.expect_err("refresh lost the CAS race");
        assert!(
            matches!(err, CredentialServiceError::VersionConflict { .. }),
            "a refresh that lost the CAS race must be VersionConflict, got {err:?}"
        );
    }
}
