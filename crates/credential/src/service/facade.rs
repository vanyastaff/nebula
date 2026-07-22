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

use std::{fmt, sync::Arc, time::Duration};

use serde::Serialize;
use serde_json::Value;

use crate::resolve::InteractionRequest;
use crate::runtime::{CredentialResolver, LeaseLifecycle};
use crate::store::{StoreError, StoredCredential};
use crate::{
    AuthPattern, CredentialContext, CredentialDisplay, CredentialRegistry, DynCredentialStore,
    ErasedCredentialStore, ErasedPendingStore,
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

/// Metadata key holding the facade-owned [`CredentialDisplay`] sub-object
/// (a sibling to [`OWNER_ID_KEY`]). Single-writer: only the facade reads or
/// writes it, so the multi-writer shape conflict that affected the api's old
/// top-level metadata layout cannot recur.
const DISPLAY_KEY: &str = "display";

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

impl fmt::Debug for Acquisition {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Complete { .. } => formatter.debug_struct("Complete").finish(),
            Self::Pending { .. } => formatter
                .debug_struct("Pending")
                .field("token", &"[REDACTED]")
                .field("interaction", &"[REDACTED]")
                .finish(),
            Self::Retry { after } => formatter
                .debug_struct("Retry")
                .field("after", after)
                .finish(),
        }
    }
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
    /// [`CredentialGuard`](crate::CredentialGuard) for action slot consumption.
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
    #[expect(clippy::too_many_arguments)]
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
        // Tie the resolver's source gate to the configured source at the single
        // construction point: a service built with an external (unwired) source
        // CANNOT hold a resolver that still reads local bytes. This makes the
        // direct-resolver paths (`scheme_factory` → `resolve_with_refresh`, which
        // bypass the facade's per-call source check) fail-closed by construction,
        // so the gate cannot drift from the source a future code path forgets.
        let resolver = resolver.gate_external_source(matches!(source, StateSource::External(_)));
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
    pub(crate) fn ensure_local_source(&self) -> Result<(), CredentialServiceError> {
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

    // ── Internal helpers ─────────────────────────────────────────────

    /// Load a row and assert it belongs to `scope`, mapping both "absent"
    /// and "other tenant" to [`CredentialServiceError::NotFound`].
    pub(crate) async fn load_owned(
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
    pub(crate) fn head_from(stored: &StoredCredential) -> CredentialHead {
        CredentialHead::from_stored(stored, Self::display_from(stored))
    }

    /// Whether `stored` is owned by `scope`. A row missing the
    /// `owner_id` stamp is treated as foreign (fail-closed).
    pub(crate) fn owner_matches(stored: &StoredCredential, scope: &TenantScope) -> bool {
        stored
            .metadata
            .get(OWNER_ID_KEY)
            .and_then(Value::as_str)
            .is_some_and(|o| o == scope.owner_id())
    }

    /// Write `display` into `metadata[DISPLAY_KEY]`, or remove the key when
    /// `display` is empty so an empty default leaves no residue. Sole writer
    /// of the reserved key (sibling to `owner_id`).
    pub(crate) fn set_display(
        metadata: &mut serde_json::Map<String, Value>,
        display: &CredentialDisplay,
    ) {
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
    /// [`CredentialContext::for_owner`] assembles exactly this shape (default
    /// credential/resource accessors + an `owner_id` override). First-party
    /// credential types resolve from their typed properties and ignore the
    /// context accessors, so the defaults are correct here. A production
    /// context wired with real accessors (for plugin credentials that consult
    /// them) is a follow-up; routing every call through this one helper keeps
    /// that migration to a single site.
    ///
    /// When the scope carries a session it is threaded onto the context
    /// via `with_session_id`: the engine's `execute_continue` (and the
    /// `PendingStateStore` `(kind, owner, session, token)` binding) reads
    /// `ctx.session_id()`, so without this the interactive paths would
    /// always fail `MissingSessionId`. CRUD passes a session-less scope
    /// and the accessors ignore the (absent) session.
    pub(crate) fn owner_context(scope: &TenantScope) -> CredentialContext {
        let ctx = CredentialContext::for_owner(scope.owner_id());
        match scope.session_id() {
            Some(session) => ctx.with_session_id(session),
            None => ctx,
        }
    }

    /// Map a [`StoreError`] into a [`CredentialServiceError`] without ever
    /// embedding secret material (store errors carry ids/versions only).
    pub(crate) fn map_store_err(err: StoreError) -> CredentialServiceError {
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

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET_CANARY: &str = "credential-acquisition-secret-NEVER-DEBUG-31af";

    #[test]
    fn acquisition_debug_redacts_complete_head_and_pending_payload() {
        let now = chrono::Utc::now();
        let complete = Acquisition::Complete {
            head: CredentialHead {
                id: SECRET_CANARY.to_owned(),
                credential_key: "api_key".to_owned(),
                version: 1,
                created_at: now,
                updated_at: now,
                expires_at: None,
                last_validated_at: Some(now),
                reauth_required: false,
                display: CredentialDisplay {
                    display_name: Some(SECRET_CANARY.to_owned()),
                    ..CredentialDisplay::default()
                },
            },
        };
        let pending = Acquisition::Pending {
            token: SECRET_CANARY.to_owned(),
            interaction: InteractionRequest::Redirect {
                url: format!("https://provider.example/?state={SECRET_CANARY}"),
            },
        };

        for debug in [format!("{complete:?}"), format!("{pending:?}")] {
            assert!(
                !debug.contains(SECRET_CANARY),
                "acquisition Debug must not expose heads, tokens, or interactions: {debug}"
            );
        }
    }
}
