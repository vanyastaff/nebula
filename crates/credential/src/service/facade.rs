//! `CredentialService` — the sole public entry to the credential
//! management bounded context. **Non-generic** (ADR-0088 D4): the raw
//! backend and pending store are erased to `Arc<dyn DynCredentialStore>` /
//! [`ErasedPendingStore`] at
//! construction, so a durable backend can be swapped in without
//! re-monomorphizing every consumer. Both ports are RPITIT (and the
//! pending port additionally generic per call), so the erasure is the
//! hand-rolled boxed-future bridge in `nebula_credential::erased`. All
//! invariant-bearing composition is crate-private: the only constructor
//! path is the api-layer credential builder, whose `build()` wraps the raw
//! backend in the layered store so an unencrypted/mis-composed service is
//! unrepresentable.
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

use nebula_resilience::CallError;
use nebula_resilience::retry::{BackoffConfig, RetryConfig, retry_with};
use nebula_schema::FieldValues;
use serde::Serialize;
use serde_json::Value;
use tokio_util::sync::CancellationToken;
use zeroize::Zeroize;

use crate::resolve::{InteractionRequest, TestResult, UserInput};
use crate::runtime::{CredentialResolver, LeaseLifecycle};
use crate::store::{PutMode, StoreError, StoredCredential};
use crate::{
    AuthPattern, Credential, CredentialContext, CredentialDisplay, CredentialGuard, CredentialId,
    CredentialLifecycle, CredentialRegistry, DynCredentialStore, ErasedCredentialStore,
    ErasedPendingStore, PendingToken, Refreshable, SchemeFactory,
};

use super::error::CredentialServiceError;
use super::head::CredentialHead;
use super::observer::CredentialObserver;
use super::ops::DispatchOps;
use super::scope::TenantScope;
use super::state_source::StateSource;

// Metadata key the facade stamps with the owning tenant, read on every
// get/list/update/delete to enforce tenant isolation. Aliased from the single
// source of truth in `store` so the facade write-stamp and the runtime
// resolver's load-time owner check can never disagree on the key.
use crate::store::OWNER_ID_METADATA_KEY as OWNER_ID_KEY;

// Reserved metadata key holding the last time the material was validated against
// its provider. `create` / re-resolve stamp it; the re-validation floor measures
// from it, NOT `updated_at` (which a display-only edit bumps). Aliased from the
// single source of truth in `store`.
use crate::store::LAST_VALIDATED_AT_METADATA_KEY;
// Reserved metadata key holding the revoke tombstone epoch. `revoke` stamps it
// (zeroizing the secret) instead of deleting the row; read paths treat a
// stamped row as gone and `validate_credential_binding` rejects it with a typed
// `CredentialTombstoned`. Aliased from the single source of truth in `store`.
use crate::store::REVOKED_AT_METADATA_KEY;

/// Metadata key holding the facade-owned [`CredentialDisplay`] sub-object
/// (a sibling to [`OWNER_ID_KEY`]). Single-writer: only the facade reads or
/// writes it, so the multi-writer shape conflict that affected the api's old
/// top-level metadata layout cannot recur.
const DISPLAY_KEY: &str = "display";

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

/// Outcome of [`CredentialService::refresh`]. `refreshed` distinguishes a
/// real provider refresh from the fallback-on-interrupt path that served
/// the still-valid stored material after a transient provider failure —
/// a management caller asking "did the refresh happen?" must not be told
/// `yes` when the provider was unreachable.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct RefreshReport {
    /// Secret-free head of the row after the call (post-refresh on the
    /// success path; the pre-call head on the fallback path).
    pub head: CredentialHead,
    /// `true` iff the provider refresh ran and the rotated state was
    /// persisted (or coalesced by another replica). `false` when the
    /// fallback served the existing non-expired material instead.
    pub refreshed: bool,
}

/// Outcome of [`CredentialService::resolve`] /
/// [`CredentialService::continue_resolve`]. Secret-free: the `Complete`
/// arm carries the management-plane [`CredentialHead`] (id + row
/// metadata, no state bytes); the `Pending` arm carries the opaque token
/// string + the UI instruction.
#[derive(Debug)]
#[non_exhaustive]
pub enum Acquisition {
    /// Resolved synchronously and persisted.
    Complete {
        /// Secret-free head of the just-persisted credential.
        head: CredentialHead,
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

/// Capability surface of a credential type, sourced from the
/// [`CredentialRegistry`] `Capabilities` bitflag (computed from sub-trait
/// membership at registration), not self-attested metadata.
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
/// UIs / pickers. Projected from [`CredentialMetadata`] + the
/// [`CredentialRegistry`] capability bitflag.
///
/// [`CredentialMetadata`]: crate::CredentialMetadata
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
/// Constructed only via the api-layer credential builder.
pub struct CredentialService {
    pub(crate) store: Arc<dyn DynCredentialStore>,
    /// Un-audited handle over the same cache+encryption core as `store`.
    /// Used ONLY by [`list`](Self::list)'s owner-filter scan: enumerating
    /// rows to discard the foreign ones is not an access, so it must not
    /// mint per-credential audit `Get` events against other tenants' ids.
    /// Every real per-id operation goes through the audited `store`.
    pub(crate) scan_store: ErasedCredentialStore,
    /// Engine resolver wired through the layered store stack (erased at the
    /// store→resolver boundary). Used by
    /// [`resolve_for_slot`](Self::resolve_for_slot) to produce a typed
    /// [`CredentialGuard`] for action slot consumption.
    pub(crate) resolver: CredentialResolver<ErasedCredentialStore>,
    pub(crate) lease: LeaseLifecycle,
    pub(crate) pending: ErasedPendingStore,
    pub(crate) registry: Arc<CredentialRegistry>,
    pub(crate) ops: Arc<DispatchOps<ErasedPendingStore>>,
    pub(crate) observer: Arc<dyn CredentialObserver>,
    // Read by `ensure_local_source` on every secret-resolving entry
    // point. `External` is configurable but its resolution wiring (the
    // external provider bridge bridge) is not implemented here yet, so it fails
    // typed rather than silently resolving from the local store.
    pub(crate) source: StateSource,
}

impl CredentialService {
    /// Composition-root constructor — the only caller is `nebula-api`'s
    /// credential builder. The layered store MUST already be the secure
    /// `Audit(Cache(Encryption(raw)))` stack; assemble it via the api
    /// builder, never by hand. Bypassing the builder forfeits the
    /// encryption-at-rest guarantee.
    // `#[doc(hidden)]`: `pub` only so the `nebula-api` composition-root builder
    // (a different crate) can call it after composing the decorator stack. It is
    // NOT supported surface — `nebula-sdk` never re-exports it, and the
    // `deny.toml` wrappers allowlist limits who may depend on `nebula-credential`
    // to the trusted in-workspace composition roots, so no external crate reaches it.
    #[doc(hidden)]
    // guard-justified: from_secure_parts mirrors the eight mandatory collaborators
    // the builder composes; bundling them into a struct would just move the
    // arity to that struct's literal at the single call site.
    #[allow(clippy::too_many_arguments)]
    pub fn from_secure_parts(
        store: Arc<dyn DynCredentialStore>,
        scan_store: ErasedCredentialStore,
        resolver: CredentialResolver<ErasedCredentialStore>,
        lease: LeaseLifecycle,
        pending: ErasedPendingStore,
        registry: Arc<CredentialRegistry>,
        ops: Arc<DispatchOps<ErasedPendingStore>>,
        observer: Arc<dyn CredentialObserver>,
        source: StateSource,
    ) -> Self {
        Self {
            store,
            scan_store,
            resolver,
            lease,
            pending,
            registry,
            ops,
            observer,
            source,
        }
    }

    /// Expose the composed layered store (`Audit(Cache(Encryption(raw)))`,
    /// erased) as a shared arc for callers that need raw access to the
    /// encrypted store without going through type dispatch.
    ///
    /// The api layer uses this to replace
    /// the tenancy `CredentialScopeLayer<InMemoryStore>` with the
    /// service's encryption + audit + cache stack while still managing
    /// api-level metadata (`name` / `description` / `tags`) directly in
    /// [`StoredCredential::metadata`]. The concrete layer stack is erased
    /// behind [`DynCredentialStore`] so the backend stays swappable.
    pub fn credential_store_handle(&self) -> Arc<dyn DynCredentialStore> {
        Arc::clone(&self.store)
    }

    /// Guard the resolution path against a configured-but-unwired
    /// external [`StateSource`].
    ///
    /// The api-layer credential builder's `external_providers`
    /// records an [`StateSource::External`] but the resolution wiring that
    /// would route through the provider chain is the external provider bridge
    /// bridge, not yet implemented in this crate. Without this guard a
    /// caller that configured Vault would silently get material resolved
    /// from the *local* store — a wrong-source security hazard. Every
    /// secret-resolving entry point (`create` / `resolve` /
    /// `continue_resolve`) calls this first so the misconfiguration fails
    /// loud and typed instead of resolving from the wrong place.
    /// `LocalEncrypted` (the default) is the success path.
    fn ensure_local_source(&self) -> Result<(), CredentialServiceError> {
        match &self.source {
            StateSource::LocalEncrypted => Ok(()),
            StateSource::External(provider) => {
                Err(CredentialServiceError::ExternalSourceNotWired {
                    provider: provider.provider_name().to_owned(),
                })
            },
        }
    }

    /// Active dynamic-lease count — a test-only smoke accessor. Gated
    /// `#[cfg(test)]` so it is **not** part of the stable public surface
    /// of this security-critical facade (lease-count introspection is a
    /// test affordance, not an API).
    #[cfg(test)]
    pub async fn active_lease_count(&self) -> usize {
        self.lease.active_lease_count().await
    }

    // ── CRUD operations ──────────────────────────────────────────────

    /// Create a credential: validate `props` against the type's schema,
    /// resolve it to encrypted state, and persist it scoped to `scope`.
    ///
    /// The validation pipeline is the canonical credential pipeline
    /// (credential secrecy): `schema_of::<Properties>().validate(FieldValues)`
    /// then a typed `serde_json::from_value` round-trip — a `{"$expr": ..}`
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
        display: CredentialDisplay,
    ) -> Result<CredentialHead, CredentialServiceError> {
        // Fail loud if an external source was configured but its
        // resolution wiring is not implemented yet — never silently
        // resolve from the local store under a Vault-configured service.
        self.ensure_local_source()?;
        // The type must be registered (TypeUnknown closes the abuse where
        // an unregistered key reaches resolution).
        if !self.registry.contains(credential_key) {
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

        let head = self
            .persist_resolved(scope, credential_key, id, resolved, display)
            .await?;

        self.observer.on_resolve(&id);
        tracing::info!(
            credential.key = credential_key,
            credential.id = %id,
            "credential created"
        );

        Ok(head)
    }

    /// Persist a freshly-resolved credential under `id` scoped to
    /// `scope`, returning the secret-free [`CredentialHead`] of the
    /// just-persisted row (never the state bytes). Shared by [`create`]
    /// and the synchronous-`Complete` arm of [`resolve`].
    ///
    /// [`create`]: Self::create
    /// [`resolve`]: Self::resolve
    async fn persist_resolved(
        &self,
        scope: &TenantScope,
        credential_key: &str,
        id: CredentialId,
        resolved: super::ops::ResolvedState,
        display: CredentialDisplay,
    ) -> Result<CredentialHead, CredentialServiceError> {
        let mut metadata = serde_json::Map::new();
        metadata.insert(
            OWNER_ID_KEY.to_owned(),
            Value::String(scope.owner_id().to_owned()),
        );
        Self::set_display(&mut metadata, &display);

        let now = chrono::Utc::now();
        // Creation resolved the credential against its provider → stamp the
        // validation time so the mandatory re-validation floor measures from a
        // real validation, not from a later display edit.
        metadata.insert(
            LAST_VALIDATED_AT_METADATA_KEY.to_owned(),
            Value::String(now.to_rfc3339()),
        );
        let stored = StoredCredential {
            id: id.to_string(),
            name: None,
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

        // The store returns the persisted row (with its post-put version),
        // which is the authoritative source for the returned head — the
        // CAS token must reflect what a subsequent `update` has to match.
        let persisted = self
            .store
            .put(stored, PutMode::CreateOnly)
            .await
            .map_err(Self::map_store_err)?;

        Ok(CredentialHead::from_stored(&persisted, display))
    }

    /// Fetch a credential's secret-free [`CredentialHead`], scoped to
    /// `scope`. Never deserializes the state bytes, so a row that is not
    /// yet resolvable (e.g. an interactive flow awaiting authorization,
    /// `reauth_required = true`) still reads back as a valid head.
    ///
    /// # Errors
    ///
    /// [`CredentialServiceError::NotFound`] if the id is absent **or**
    /// belongs to another tenant (no cross-tenant existence leak).
    pub async fn get(
        &self,
        scope: &TenantScope,
        id: &str,
    ) -> Result<CredentialHead, CredentialServiceError> {
        let stored = self.load_owned(scope, id).await?;
        Ok(Self::head_from(&stored))
    }

    /// List the secret-free heads of every credential visible to `scope`
    /// (rows whose stored `owner_id` matches).
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
    pub async fn list(
        &self,
        scope: &TenantScope,
    ) -> Result<Vec<CredentialHead>, CredentialServiceError> {
        // The id enumeration goes through the audited store (one `List`
        // audit event); the per-row owner-filter reads go through the
        // un-audited `scan_store` so foreign rows — fetched only to be
        // discarded — never mint audit `Get` events against other
        // tenants' ids (the audit trail must record accesses, not scans).
        let ids = self.store.list(None).await.map_err(Self::map_store_err)?;
        let mut visible = Vec::new();
        for id in ids {
            match self.scan_store.get(&id).await {
                Ok(stored) => {
                    // Skip foreign rows (owner filter) and revoked rows: a
                    // tombstone is a retired credential, not a listable one.
                    if Self::owner_matches(&stored, scope) && !stored.is_tombstoned() {
                        visible.push(Self::head_from(&stored));
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

    /// Update a credential's stored state and/or display metadata.
    ///
    /// `props = Some(..)` re-runs the canonical validate→resolve pipeline
    /// for the row's (unchanged) credential type and replaces the stored
    /// state; `props = None` keeps the existing state bytes untouched and
    /// rewrites only the display metadata — a rename/re-tag never
    /// re-resolves or re-encrypts material.
    ///
    /// `display` is the **full replacement** value; callers that want
    /// field-wise merge semantics read the current head first and merge
    /// before calling.
    ///
    /// `expected_version = Some(v)` engages compare-and-swap on the
    /// caller's version (a mismatch surfaces as
    /// [`CredentialServiceError::VersionConflict`]); `None` CASes on the
    /// version this call just loaded, so a concurrent write landing
    /// between the load and the put surfaces as `VersionConflict` instead
    /// of silently rolling the row — including its secret state and any
    /// concurrently-rotated tokens — back to the loaded copy. There is no
    /// blind-overwrite path.
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
        props: Option<Value>,
        expected_version: Option<u64>,
        display: CredentialDisplay,
    ) -> Result<CredentialHead, CredentialServiceError> {
        // Owner check first: a cross-tenant id is reported as missing,
        // never as a version conflict (no existence leak).
        let existing = self.load_owned(scope, id).await?;

        // Re-resolve only when new properties were supplied; a
        // display-only update carries the existing state through.
        let resolved = match props {
            Some(props) => {
                self.ops.validate(&existing.credential_key, &props)?;
                let values = FieldValues::from_json(props).map_err(|e| {
                    CredentialServiceError::ValidationFailed {
                        reason: format!("property ingest failed: {e}"),
                    }
                })?;
                let ctx = Self::owner_context(scope);
                Some(
                    self.ops
                        .resolve(&existing.credential_key, &values, &ctx, &self.pending)
                        .await?,
                )
            },
            None => None,
        };

        let mut metadata = existing.metadata.clone();
        metadata.insert(
            OWNER_ID_KEY.to_owned(),
            Value::String(scope.owner_id().to_owned()),
        );
        Self::set_display(&mut metadata, &display);

        let now = chrono::Utc::now();
        let stored = match resolved {
            // Props supplied ⇒ re-resolved against the provider ⇒ stamp the
            // validation time. A display-only edit (the `None` arm) preserves the
            // existing stamp and bumps only `updated_at`, so it cannot postpone
            // the re-validation floor.
            Some(resolved) => {
                metadata.insert(
                    LAST_VALIDATED_AT_METADATA_KEY.to_owned(),
                    Value::String(now.to_rfc3339()),
                );
                StoredCredential {
                    id: existing.id.clone(),
                    name: existing.name.clone(),
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
                }
            },
            None => StoredCredential {
                updated_at: now,
                metadata,
                ..existing.clone()
            },
        };

        // No blind-overwrite path: when the caller supplied no version,
        // CAS on the version loaded above. A display-only rename racing a
        // token refresh must conflict, never silently restore the stale
        // secret bytes captured at load time.
        let mode = PutMode::CompareAndSwap {
            expected_version: expected_version.unwrap_or(existing.version),
        };

        let persisted = self
            .store
            .put(stored, mode)
            .await
            .map_err(Self::map_store_err)?;

        tracing::info!(credential.id = %id, "credential updated");
        Ok(Self::head_from(&persisted))
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
        if !self.registry.is_testable(&stored.credential_key) {
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
            // `TestResult` is exhaustively matched here (this crate defines it).
            // Adding a variant is a compile error at this arm, forcing a
            // deliberate decision rather than silently presenting as a pass.
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
    /// concurrent-refresh contract: refresh must never silently strand a concurrent
    /// write. If another replica coalesced the refresh
    /// (`RefreshOutcome::CoalescedByOtherReplica`) the write is **skipped
    /// entirely** and the now-fresher state is re-read from the store
    /// instead of clobbering it with the un-mutated local copy. On
    /// success (either path) [`CredentialObserver::on_refresh`] fires and
    /// the fresh secret-free [`CredentialHead`] is returned.
    ///
    /// ## Fallback-on-interrupt
    ///
    /// If the provider call fails with a **transient** error
    /// ([`CredentialServiceError::TransientProvider`]) AND the currently
    /// stored material is still non-expired, the cached head is
    /// returned instead of propagating the error. This protects in-flight
    /// executions from transient provider 5xx / network blips without
    /// papering over real expiry. Terminal failures (token expired / revoked /
    /// authentication) always propagate regardless of cached state.
    ///
    /// This matches the `aws-credential-types` `fallback_on_interrupt` pattern.
    ///
    /// # Errors
    ///
    /// - [`CredentialServiceError::NotFound`] — absent or cross-tenant id.
    /// - [`CredentialServiceError::CapabilityUnsupported`] — type is not `Refreshable`.
    /// - [`CredentialServiceError::Provider`] — refresh failed after retries (terminal).
    /// - [`CredentialServiceError::TransientProvider`] — transient failure AND stored material
    ///   is expired (no valid fallback available).
    /// - [`CredentialServiceError::VersionConflict`] — a concurrent write landed first.
    /// - [`CredentialServiceError::Store`] — re-persist failed.
    pub async fn refresh(
        &self,
        scope: &TenantScope,
        id: &str,
    ) -> Result<RefreshReport, CredentialServiceError> {
        // Read the current head before attempting refresh. On a transient
        // provider failure we fall back to this if the material is still
        // non-expired — avoids propagating blips to the caller. The
        // report's `refreshed: false` keeps the fallback honest for
        // management callers.
        let cached = self.get(scope, id).await?;

        match self.refresh_inner(scope, id).await {
            Ok(head) => Ok(RefreshReport {
                head,
                refreshed: true,
            }),
            Err(ref e) if Self::is_transient_failure(e) && !cached.is_expired() => {
                tracing::warn!(
                    credential.id = %id,
                    error = %e,
                    "credential refresh failed transiently; stored material still non-expired"
                );
                Ok(RefreshReport {
                    head: cached,
                    refreshed: false,
                })
            },
            Err(e) => Err(e),
        }
    }

    /// Inner refresh: actual provider call + CAS-persist. The public
    /// [`refresh`](Self::refresh) wrapper applies the fallback-on-interrupt
    /// logic around this method.
    async fn refresh_inner(
        &self,
        scope: &TenantScope,
        id: &str,
    ) -> Result<CredentialHead, CredentialServiceError> {
        let stored = self.load_owned(scope, id).await?;
        if !self.registry.is_refreshable(&stored.credential_key) {
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

        let (refreshed, refreshed_expires_at) = match outcome {
            super::ops::RefreshOutcomeKind::Rewrote { data, expires_at } => (data, expires_at),
            // Another replica already refreshed and persisted fresher
            // state. Re-writing the un-mutated local copy here would
            // either spuriously `VersionConflict` or clobber that fresher
            // state (concurrent-refresh contract). Skip the write entirely and return the
            // store's current (post-coalesce) head.
            super::ops::RefreshOutcomeKind::CoalescedReRead => {
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
                return self.get(scope, id).await;
            },
        };

        let now = chrono::Utc::now();
        let state_kind = stored.state_kind.clone();
        let state_version = stored.state_version;
        let stored_next = StoredCredential {
            id: stored.id.clone(),
            name: stored.name.clone(),
            credential_key: stored.credential_key.clone(),
            data: refreshed.to_vec(),
            state_kind,
            state_version,
            version: stored.version,
            created_at: stored.created_at,
            updated_at: now,
            // The refresh closure read this off the *refreshed* state
            // (`CredentialState::expires_at()`), not the pre-refresh row:
            // a token rotation typically produces a new expiry, so reusing
            // `stored.expires_at` would persist a stale (possibly
            // already-elapsed) expiry against fresh credential bytes.
            expires_at: refreshed_expires_at,
            reauth_required: false,
            metadata: stored.metadata.clone(),
        };
        // Re-persist under compare-and-swap on the version observed at
        // load. A concurrent refresh/update that landed in between wins
        // and this attempt fails *explicitly* with `VersionConflict`
        // (concurrent-refresh contract: refresh must never silently strand a concurrent
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
        self.get(scope, id).await
    }

    /// True iff this error is a transient refresh/provider failure that the
    /// fallback-on-interrupt path can swallow when cached material is still
    /// non-expired.
    ///
    /// Only [`CredentialServiceError::TransientProvider`] qualifies — this
    /// variant is emitted exclusively by the refresh ops closure for the
    /// transient `CredentialError` kinds (`RefreshFailed(TransientNetwork |
    /// ProviderUnavailable)` / `Provider(Network | RateLimit | ServerError)`).
    /// Terminal failures use [`CredentialServiceError::Provider`] and are
    /// excluded here so the fallback never swallows real expiry or auth errors.
    #[inline]
    fn is_transient_failure(e: &CredentialServiceError) -> bool {
        matches!(e, CredentialServiceError::TransientProvider(_))
    }

    /// Revoke the credential at the provider, release any leases, and write a
    /// revoke **tombstone** over the stored row (it is not deleted).
    ///
    /// Owner-checked first. The provider-side revoke runs the type's
    /// `Revocable::revoke`; lease release is best-effort (a failure is
    /// logged, not propagated — the credential is still revoked); the stored
    /// row is then CAS-overwritten with a tombstone epoch and empty secret
    /// bytes. On success [`CredentialObserver::on_revoke`] fires.
    ///
    /// The row is tombstoned rather than deleted so the id cannot be
    /// resurrected and a slot binding still pointing at it resolves to a typed
    /// [`CredentialTombstoned`](super::binding::ValidatedCredentialBindingError::CredentialTombstoned)
    /// rather than a bare `NotFound`. Every management read
    /// ([`get`](Self::get)/[`list`](Self::list)/[`update`](Self::update)/
    /// [`refresh`](Self::refresh)) then treats the row as gone, so a second
    /// revoke of the same id returns
    /// [`NotFound`](CredentialServiceError::NotFound) (idempotent from the
    /// caller's view).
    ///
    /// `Revocable::revoke` receives `&mut state` and may mutate it. That
    /// mutation is intentionally **not** re-persisted: the tombstone drops the
    /// secret bytes, so there is no live state to write back — unlike
    /// [`refresh`](Self::refresh), which keeps the row and CAS-persists its
    /// mutated state.
    ///
    /// # Errors
    ///
    /// - [`CredentialServiceError::NotFound`] — absent, cross-tenant, or already-revoked id.
    /// - [`CredentialServiceError::CapabilityUnsupported`] — type is not `Revocable`.
    /// - [`CredentialServiceError::Provider`] — the provider revoke failed.
    /// - [`CredentialServiceError::VersionConflict`] — a concurrent write raced the revoke.
    /// - [`CredentialServiceError::Store`] — persisting the tombstone failed.
    pub async fn revoke(
        &self,
        scope: &TenantScope,
        id: &str,
    ) -> Result<(), CredentialServiceError> {
        let stored = self.load_owned(scope, id).await?;
        if !self.registry.is_revocable(&stored.credential_key) {
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

        // Write a tombstone instead of deleting the row. A revoked credential
        // must not be resurrectable under the same id, and a workflow slot
        // binding that still points at it must surface a typed
        // `CredentialTombstoned` (via `validate_credential_binding`) rather than
        // a bare `NotFound`. The secret bytes are dropped — a revoked secret has
        // no reason to persist at rest. CAS on the version loaded above so a
        // rotation/update racing this revoke conflicts instead of silently
        // clobbering (or resurrecting) the row.
        let now = chrono::Utc::now();
        let expected_version = stored.version;
        let mut metadata = stored.metadata;
        metadata.insert(
            REVOKED_AT_METADATA_KEY.to_owned(),
            Value::String(now.to_rfc3339()),
        );
        let tombstoned = StoredCredential {
            data: Vec::new(),
            updated_at: now,
            metadata,
            ..stored
        };
        self.store
            .put(tombstoned, PutMode::CompareAndSwap { expected_version })
            .await
            .map_err(Self::map_store_err)?;

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
    /// refusal point, credential secrecy). A `Complete` resolution is persisted
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
        self.ensure_local_source()?;
        if !self.registry.contains(credential_key) {
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
        self.ensure_local_source()?;
        if !self.registry.contains(credential_key) {
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
        outcome: super::ops::AcquireOutcome,
    ) -> Result<Acquisition, CredentialServiceError> {
        match outcome {
            super::ops::AcquireOutcome::Complete(resolved) => {
                let id = CredentialId::new();
                // Acquisition carries no caller-supplied display metadata
                // (the interactive/resolve flow names nothing); a later
                // `update` can attach it.
                let head = self
                    .persist_resolved(
                        scope,
                        credential_key,
                        id,
                        resolved,
                        CredentialDisplay::default(),
                    )
                    .await?;
                self.observer.on_resolve(&id);
                tracing::info!(
                    credential.key = credential_key,
                    credential.id = %id,
                    "credential acquired"
                );
                Ok(Acquisition::Complete { head })
            },
            super::ops::AcquireOutcome::Pending { token, interaction } => {
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
            super::ops::AcquireOutcome::Retry { after } => Ok(Acquisition::Retry { after }),
        }
    }

    // ── Type discovery ───────────────────────────────────────────────

    /// List every registered credential type as a secret-free
    /// descriptor. Capability flags come from the [`CredentialRegistry`]
    /// bitflag (computed from sub-trait membership at registration), not
    /// self-attested metadata.
    #[must_use]
    pub fn list_types(&self) -> Vec<CredentialTypeInfo> {
        self.registry
            .iter_compatible(crate::Capabilities::empty())
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
    /// capability bitflag. Returns `None` if the registry has no
    /// instance for `key` (cannot project metadata).
    fn type_info(&self, key: &str) -> Option<CredentialTypeInfo> {
        let metadata = self.registry.resolve_any(key)?.metadata();
        Some(CredentialTypeInfo {
            key: metadata.base.key.as_str().to_owned(),
            name: metadata.base.name.clone(),
            description: metadata.base.description.clone(),
            pattern: metadata.pattern,
            capabilities: TypeCapabilities {
                refreshable: self.registry.is_refreshable(key),
                testable: self.registry.is_testable(key),
                revocable: self.registry.is_revocable(key),
            },
        })
    }

    // ── Binding validation ───────────────────────────────────────────

    /// Validate a workflow `slot_bindings` reference against the caller's
    /// tenant scope, returning a typed
    /// [`ValidatedCredentialBinding`](crate::ValidatedCredentialBinding) that
    /// engine execution consumes.
    ///
    /// This is the **only construction path** for
    /// `ValidatedCredentialBinding`. Its `pub(crate)` constructor is
    /// unreachable from outside `nebula-credential`, so engine code
    /// that consumes the handle has a structural proof that the scope-check
    /// already ran.
    ///
    /// # Cross-tenant behaviour
    ///
    /// Unlike every other read operation in this service (which maps
    /// cross-tenant ids to [`CredentialServiceError::NotFound`] to prevent
    /// existence leaks), `validate_credential_binding` intentionally reads
    /// the foreign row's `owner_id` so it can emit a structured
    /// [`ScopeMismatch`](crate::ValidatedCredentialBindingError::ScopeMismatch)
    /// error rather than a misleading `NotFound`. Workflow authors debugging
    /// a misconfigured binding need to know the mismatch occurred; they are
    /// not adversarial tenants probing for existence.
    ///
    /// The raw read path (`store_load_raw`) bypasses the `owner_id`
    /// existence-hiding gate used by `load_owned`.
    ///
    /// # Errors
    ///
    /// - [`crate::ValidatedCredentialBindingError::NotFound`] — id absent from the store.
    /// - [`crate::ValidatedCredentialBindingError::ScopeMismatch`] — id exists but
    ///   belongs to a different tenant.
    /// - [`crate::ValidatedCredentialBindingError::Io`] — underlying store error.
    pub async fn validate_credential_binding(
        &self,
        scope: &TenantScope,
        id: &str,
    ) -> Result<
        super::binding::ValidatedCredentialBinding,
        super::binding::ValidatedCredentialBindingError,
    > {
        let stored = self
            .store_load_raw(id)
            .await
            .map_err(super::binding::ValidatedCredentialBindingError::Io)?
            .ok_or_else(
                || super::binding::ValidatedCredentialBindingError::NotFound { id: id.to_owned() },
            )?;

        let owner = stored
            .metadata
            .get(OWNER_ID_KEY)
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if owner != scope.owner_id() {
            return Err(
                super::binding::ValidatedCredentialBindingError::ScopeMismatch {
                    id: id.to_owned(),
                    requested: scope.owner_id().to_owned(),
                    actual: owner.to_owned(),
                },
            );
        }

        // Reject a revoked credential here — before any binding (and thus any
        // guard) is produced — with a typed `CredentialTombstoned` rather than
        // a bare `NotFound`, so the caller learns the slot stopped resolving
        // because the credential was revoked. The check is owner-gated above,
        // so it never reveals another tenant's revoke status. No reverse
        // `references()` index is consulted: the tombstone travels with the row.
        if stored.is_tombstoned() {
            return Err(
                super::binding::ValidatedCredentialBindingError::CredentialTombstoned {
                    id: id.to_owned(),
                    revoked_at: stored.revoked_at(),
                },
            );
        }

        Ok(super::binding::ValidatedCredentialBinding::new(
            id.to_owned(),
            super::binding::TenantFingerprint::from_scope(scope),
        ))
    }

    /// Production execution-time resolver. Consumes a tenant-validated
    /// binding (from [`validate_credential_binding`]) and produces a typed
    /// [`CredentialGuard<C::Scheme>`] for an action slot.
    ///
    /// # Hot path
    ///
    /// Called once per action node per execution. The engine resolver
    /// (`CredentialResolver::resolve`) goes through the full layered-store
    /// stack (`Audit(Cache(Encryption(raw)))`) composed at `build()` —
    /// the `EncryptionLayer` decrypts on every miss, `CacheLayer` coalesces
    /// warm-cache hits to avoid repeated decrypt, and `AuditLayer` records
    /// each access. Target p99 ≤ 1ms on warm cache.
    ///
    /// # Cancellation
    ///
    /// `cancel` is observed via [`CancellationToken::run_until_cancelled`]
    /// wrapping the entire resolver delegation. On cancellation, returns
    /// [`CredentialServiceError::Cancelled`] without partial state.
    ///
    /// # Defence in depth
    ///
    /// Re-checks the binding's tenant fingerprint against `scope` even
    /// though [`validate_credential_binding`] already enforced it at
    /// construction — type-safe consumption with a runtime sanity arm that
    /// fires if a binding is presented against the wrong scope.
    ///
    /// # Errors
    ///
    /// - [`CredentialServiceError::ScopeViolation`] — binding's tenant
    ///   fingerprint does not match `scope`.
    /// - [`CredentialServiceError::Cancelled`] — `cancel` fired.
    /// - [`CredentialServiceError::NotFound`] — credential absent from store.
    /// - [`CredentialServiceError::Internal`] — resolver error (kind
    ///   mismatch, deserialisation failure, or store error).
    ///
    /// [`validate_credential_binding`]: Self::validate_credential_binding
    pub async fn resolve_for_slot<C>(
        &self,
        scope: &TenantScope,
        binding: &super::binding::ValidatedCredentialBinding,
        cancel: CancellationToken,
    ) -> Result<CredentialGuard<C::Scheme>, CredentialServiceError>
    where
        C: Credential,
        C::Scheme: Zeroize + Clone,
    {
        // 0. Source gate: a service configured with an `External` state source
        //    has no local-decrypt resolution path wired, so resolving a slot
        //    against it must fail with `ExternalSourceNotWired` rather than read
        //    local bytes. This guard is present on every other secret-resolving
        //    entry point; resolve_for_slot — the moat path — must not be the one
        //    that skips it.
        self.ensure_local_source()?;

        // 1. Defence-in-depth fingerprint check: even though
        //    `validate_credential_binding` enforced the scope at
        //    construction, re-verify here so mismatched bindings fail
        //    loudly at the consume site.
        let expected_fp = super::binding::TenantFingerprint::from_scope(scope);
        if binding.fingerprint() != &expected_fp {
            return Err(CredentialServiceError::ScopeViolation {
                requested: scope.owner_id().to_string(),
            });
        }

        // 2. Delegate to engine resolver, wrapped in cancellation. The
        //    resolver goes through the full layered store stack
        //    (EncryptionLayer → CacheLayer → AuditLayer) composed at
        //    `build()`, so the EncryptionLayer is not bypassed.
        let credential_id = binding.credential_id();
        // Resolve through the binding's owner-scoped key: the resolver re-checks
        // the stored row's owner at load, so a cross-tenant id fails closed
        // (`NotFound`) by construction rather than relying on the fingerprint
        // check above alone.
        let key = binding.owner_scoped_key();
        let scheme = cancel
            .run_until_cancelled(async {
                let handle = self.resolver.resolve_scoped::<C>(&key).await.map_err(|e| {
                    // Preserve the documented `NotFound` contract for
                    // resolver lookup misses. The resolver wraps store
                    // errors in `ResolveError::Store(StoreError::NotFound)`
                    // — surface that as `CredentialServiceError::NotFound`
                    // so callers can branch on it. Other resolver errors
                    // collapse to `Internal` with the underlying message.
                    use crate::runtime::ResolveError;
                    use crate::store::StoreError;
                    match e {
                        ResolveError::Store(StoreError::NotFound { id }) => {
                            CredentialServiceError::NotFound { id }
                        },
                        other => CredentialServiceError::Internal(other.to_string()),
                    }
                })?;

                // Extract the owned scheme from the snapshot `Arc`. The
                // resolver caches live handles, so `try_unwrap` succeeds when
                // this is the only outstanding snapshot; otherwise clone.
                let arc = handle.snapshot();
                let owned = Arc::try_unwrap(arc).unwrap_or_else(|arc| (*arc).clone());
                Ok::<_, CredentialServiceError>(owned)
            })
            .await
            .ok_or(CredentialServiceError::Cancelled)??;

        tracing::debug!(
            credential.id = credential_id,
            "credential resolved for slot"
        );
        Ok(CredentialGuard::new(scheme))
    }

    /// Per-request scheme re-acquisition for long-lived resources (§15.7).
    ///
    /// Stash the returned [`SchemeFactory`] on the resource instance at
    /// `create` and call [`SchemeFactory::acquire`] once per outbound
    /// request instead of retaining a [`CredentialGuard`] across spawn
    /// boundaries (which is forbidden — see SEC-05).
    pub fn scheme_factory<C>(&self, credential_id: &str, ctx: CredentialContext) -> SchemeFactory<C>
    where
        C: Refreshable + CredentialLifecycle,
        C::Scheme: Zeroize + Clone + Send + Sync + 'static,
    {
        self.resolver.scheme_factory(credential_id, ctx)
    }

    /// Load the raw stored credential row **without** applying the
    /// `owner_id` existence-hiding gate that `load_owned` enforces.
    ///
    /// `pub(crate)` — callers outside this crate cannot bypass the tenant
    /// isolation enforced by the public operations. The only in-crate
    /// caller today is `validate_credential_binding`, which needs to read
    /// the foreign `owner_id` to emit a structured `ScopeMismatch` rather
    /// than a misleading `NotFound`.
    pub(crate) async fn store_load_raw(
        &self,
        id: &str,
    ) -> Result<Option<StoredCredential>, CredentialServiceError> {
        match self.store.get(id).await {
            Ok(stored) => Ok(Some(stored)),
            Err(StoreError::NotFound { .. }) => Ok(None),
            Err(e) => Err(Self::map_store_err(e)),
        }
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
        if stored.is_tombstoned() {
            // A revoked credential reads back as gone to every management path
            // (get / update / test / refresh) and to a repeat revoke, so the
            // surviving row is a non-resurrectable tombstone, not a live
            // credential. The slot-binding path surfaces the typed "revoked"
            // signal separately in `validate_credential_binding`; management
            // ops see `NotFound` so a revoked id behaves as absent.
            return Err(CredentialServiceError::NotFound { id: id.to_owned() });
        }
        Ok(stored)
    }

    /// Project a stored row to its secret-free [`CredentialHead`],
    /// attaching the facade-owned display sub-object. Never touches
    /// `stored.data`.
    fn head_from(stored: &StoredCredential) -> CredentialHead {
        CredentialHead::from_stored(stored, Self::display_from(stored))
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

    /// Write `display` into `metadata[DISPLAY_KEY]`, or remove the key when
    /// `display` is empty so an empty default leaves no residue. Sole writer
    /// of the reserved key (sibling to `owner_id`).
    fn set_display(metadata: &mut serde_json::Map<String, Value>, display: &CredentialDisplay) {
        if display.is_empty() {
            metadata.remove(DISPLAY_KEY);
            return;
        }
        // `CredentialDisplay` is plain `Option<String>` / `BTreeMap` fields,
        // so serialization to a JSON object cannot fail; on the impossible
        // error drop the key rather than persist a corrupt entry.
        match serde_json::to_value(display) {
            Ok(v) => {
                metadata.insert(DISPLAY_KEY.to_owned(), v);
            },
            Err(_) => {
                metadata.remove(DISPLAY_KEY);
            },
        }
    }

    /// Read the facade-owned display sub-object back from a stored row. A
    /// missing or malformed entry yields the empty default — display metadata
    /// is non-critical and must never fail a credential read.
    fn display_from(stored: &StoredCredential) -> CredentialDisplay {
        stored
            .metadata
            .get(DISPLAY_KEY)
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default()
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
        }
    }
}
