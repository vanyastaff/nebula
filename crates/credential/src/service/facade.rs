//! `CredentialService` — the technical semantic service behind the
//! authority-bound controller and runtime consumers. **Non-generic** (ADR-0088 D4): the raw
//! backend and pending store are erased to `Arc<dyn CredentialPersistence>` /
//! [`ErasedPendingStore`] at
//! construction, so a durable backend can be swapped in without
//! re-monomorphizing every consumer. Credential persistence is directly
//! object-safe; only the generic pending port uses the boxed-future bridge in
//! `nebula_credential::erased`. The first-party production composition root
//! lives in `apps/server` and supplies the secure layered store.
//! `from_secure_parts` remains a public, doc-hidden technical seam for trusted
//! workspace application/test composition; it is not exposed by the supported
//! API/SDK and its caller must preserve the layering invariant.
//!
//! ## Tenant isolation
//!
//! Tenancy is enforced by mandatory owner-bound persistence inputs. Every
//! single-row operation derives a [`CredentialSelector`](crate::CredentialSelector) from
//! `TenantScope`;
//! list takes the mandatory owner. Backends include owner in the query/CAS
//! predicate, so a credential in another tenant is indistinguishable from a
//! missing one and no post-read authority check is required. The metadata
//! owner stamp is retained only as a compatibility/integrity check.
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
use crate::{
    AuthPattern, CredentialAlreadyExistsKey, CredentialContext, CredentialDisplay, CredentialId,
    CredentialPersistence, CredentialPersistenceError, CredentialRegistry, ErasedPendingStore,
    StoredCredential, StoredCredentialHead, StoredLiveCredential,
};

use super::error::CredentialServiceError;
use super::head::CredentialHead;
use super::observer::CredentialObserver;
use super::ops::DispatchOps;
use super::scope::TenantScope;
use super::state_source::StateSource;

/// Metadata key holding the facade-owned [`CredentialDisplay`] sub-object
/// Single-writer: only the facade reads or writes it, so the multi-writer shape
/// conflict that affected the api's old top-level metadata layout cannot recur.
const DISPLAY_KEY: &str = "display";

/// Outcome of [`CredentialService::refresh`]. `refreshed` distinguishes a
/// real provider refresh from the pre-dispatch fallback path that served the
/// still-valid stored material after coordination failed before provider I/O.
/// Errors returned after entering an erased integration are outcome-unknown
/// and never take this fallback.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ManagementRefreshReport {
    /// Secret-free head of the row after the call (post-refresh on the
    /// success path; the pre-call head on the fallback path).
    pub head: CredentialHead,
    /// `true` iff the provider refresh ran and the rotated state was
    /// persisted (or coalesced by another replica). `false` when the
    /// fallback served existing non-expired material after a failure proven to
    /// occur before provider dispatch.
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

/// Technical semantic service for the credential bounded context.
///
/// Supported authenticated HTTP management enters through
/// [`CredentialController`](super::CredentialController), which authorizes
/// before calling this service. Runtime/webhook technical consumers also use
/// selected service methods directly; K3 will make the controller plus
/// operation ledger the sole semantic management writer. First-party secure
/// production construction belongs to the deployment application; API is a
/// transport boundary, not a composition root.
pub struct CredentialService {
    pub(crate) store: Arc<dyn CredentialPersistence>,
    /// Engine resolver wired through the layered store stack (erased at the
    /// store→resolver boundary). Used by
    /// [`resolve_for_slot`](Self::resolve_for_slot) to produce a typed
    /// [`CredentialGuard`](crate::CredentialGuard) for action slot consumption.
    pub(crate) resolver: CredentialResolver<dyn CredentialPersistence>,
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
    /// Trusted composition constructor. The layered store MUST already be the
    /// secure `Audit(Encryption(raw))` stack; production assembly belongs to
    /// the deployment application, while API-local callers are test factories.
    // `#[doc(hidden)]`: `pub` only so trusted in-workspace application/test
    // composition can supply the complete collaborator set. It is NOT a
    // supported surface — `nebula-sdk` never re-exports it, and dependency
    // wrappers limit direct use to technical boundaries.
    #[doc(hidden)]
    // guard-justified: from_secure_parts mirrors the mandatory collaborators
    // the builder composes; bundling them into a struct would just move the
    // arity to that struct's literal at the single call site.
    #[expect(clippy::too_many_arguments)]
    pub fn from_secure_parts(
        store: Arc<dyn CredentialPersistence>,
        resolver: CredentialResolver<dyn CredentialPersistence>,
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
    ) -> Result<StoredLiveCredential, CredentialServiceError> {
        let credential_id = CredentialId::parse(id)
            .map_err(|_| CredentialServiceError::NotFound { id: id.to_owned() })?;
        let stored = match self.store.get(&scope.selector(credential_id)).await {
            Ok(stored) => stored,
            Err(CredentialPersistenceError::NotFound) => {
                return Err(CredentialServiceError::NotFound { id: id.to_owned() });
            },
            Err(error) => return Err(Self::map_store_err_for(id, error)),
        };
        match stored {
            StoredCredential::Live(live) => Ok(live),
            StoredCredential::Tombstoned(_) => {
                // The trusted binding path may inspect a physical tombstone,
                // while every management operation treats it as absent.
                Err(CredentialServiceError::NotFound { id: id.to_owned() })
            },
        }
    }

    /// Project a persistence head to the public management head without
    /// touching credential material.
    pub(crate) fn head_from_projection(stored: &StoredCredentialHead) -> CredentialHead {
        CredentialHead::from_stored(stored, Self::display_from_metadata(stored.metadata()))
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
    pub(crate) fn display_from_metadata(
        metadata: &serde_json::Map<String, Value>,
    ) -> CredentialDisplay {
        metadata
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
    /// When the scope carries a Plane-A authentication binding it is threaded
    /// onto the context's legacy `session_id` slot: the engine's
    /// `execute_continue` (and the
    /// `PendingStateStore` `(kind, owner, session, token)` binding) reads
    /// `ctx.session_id()`, so without this the interactive paths would
    /// always fail `MissingSessionId`. CRUD passes a binding-less scope
    /// and the accessors ignore the absent value.
    pub(crate) fn owner_context(scope: &TenantScope) -> CredentialContext {
        let ctx = CredentialContext::for_owner(scope.owner_id());
        match scope.authentication_binding() {
            Some(binding) => ctx.with_session_id(binding),
            None => ctx,
        }
    }

    /// Map a [`CredentialPersistenceError`] into a [`CredentialServiceError`] without ever
    /// forwarding backend- or audit-controlled diagnostic text.
    pub(crate) fn map_store_err_for(
        id: &str,
        err: CredentialPersistenceError,
    ) -> CredentialServiceError {
        match err {
            CredentialPersistenceError::NotFound => {
                CredentialServiceError::NotFound { id: id.to_owned() }
            },
            CredentialPersistenceError::VersionConflict { expected, actual } => {
                CredentialServiceError::VersionConflict {
                    id: id.to_owned(),
                    expected: expected.get() as u64,
                    actual: actual.get() as u64,
                }
            },
            CredentialPersistenceError::AlreadyExists {
                key: CredentialAlreadyExistsKey::Id,
            } => CredentialServiceError::IdAlreadyExists,
            CredentialPersistenceError::AlreadyExists {
                key: CredentialAlreadyExistsKey::Name,
            } => CredentialServiceError::NameAlreadyExists,
            CredentialPersistenceError::VersionExhausted
            | CredentialPersistenceError::MaterialEpochExhausted => {
                CredentialServiceError::VersionExhausted
            },
            CredentialPersistenceError::OutcomeUnknown => CredentialServiceError::OutcomeUnknown,
            CredentialPersistenceError::CorruptRecord => CredentialServiceError::Store,
            CredentialPersistenceError::Unavailable => {
                CredentialServiceError::PersistenceUnavailable
            },
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

    #[test]
    fn persistence_mapping_preserves_only_closed_context() {
        let mapped = CredentialService::map_store_err_for(
            "cred_test",
            CredentialPersistenceError::OutcomeUnknown,
        );
        assert!(matches!(&mapped, CredentialServiceError::OutcomeUnknown));
        assert!(!mapped.to_string().contains(SECRET_CANARY));
        assert!(!format!("{mapped:?}").contains(SECRET_CANARY));

        assert!(matches!(
            CredentialService::map_store_err_for(
                "cred_test",
                CredentialPersistenceError::AlreadyExists {
                    key: CredentialAlreadyExistsKey::Name,
                },
            ),
            CredentialServiceError::NameAlreadyExists
        ));
        assert!(matches!(
            CredentialService::map_store_err_for(
                "cred_test",
                CredentialPersistenceError::AlreadyExists {
                    key: CredentialAlreadyExistsKey::Id,
                },
            ),
            CredentialServiceError::IdAlreadyExists
        ));
        assert!(matches!(
            CredentialService::map_store_err_for(
                "cred_test",
                CredentialPersistenceError::VersionExhausted,
            ),
            CredentialServiceError::VersionExhausted
        ));
    }
}
