//! Test-only `CredentialServiceBuilder`.
//!
//! The first-party production composition lives in `apps/server`. This helper
//! remains inside the `test-util` surface so API integration tests can compose
//! the real storage, resolver, and lease machinery without copying setup.
//!
//! Panel-refined shape (spec §5): every mandatory collaborator is a
//! by-value argument to [`CredentialServiceBuilder::new`], so omitting
//! one is a compile error; optional collaborators are chained setters;
//! [`build`](CredentialServiceBuilder::build) is infallible and holds
//! **no `Option`/`unwrap`** for mandatory state. `build()` performs the
//! secure composition `Audit(Encryption(raw))` +
//! the engine resolver + lease lifecycle — so an unencrypted or
//! mis-composed service cannot be constructed.

use std::sync::Arc;

use crate::ports::reqwest_transport::ReqwestRefreshTransport;
use nebula_credential::runtime::{CredentialResolver, LeaseLifecycle, LeaseLifecycleConfig};
use nebula_credential::{
    Capabilities, CredentialObserver, CredentialRegistry, CredentialService,
    CredentialServiceError, DispatchOps, ErasedPendingStore, StateSource,
};
use nebula_engine::credential::default_in_memory_coordinator;
use nebula_storage::credential::{AuditLayer, AuditSink, EncryptionLayer, KeyProvider};
use nebula_storage_port::CredentialPersistence;
use tokio_util::sync::CancellationToken;

/// Builder for [`CredentialService`]. Construct via [`Self::new`] (all
/// mandatory collaborators), chain optional setters, then [`build`].
///
/// [`build`]: Self::build
pub(crate) struct CredentialServiceBuilder<B: CredentialPersistence + 'static> {
    raw_store: B,
    key_provider: Arc<dyn KeyProvider>,
    audit_sink: Arc<dyn AuditSink>,
    pending_store: ErasedPendingStore,
    registry: Arc<CredentialRegistry>,
    ops: Arc<DispatchOps<ErasedPendingStore>>,
    observer: Arc<dyn CredentialObserver>,
    lease_config: LeaseLifecycleConfig,
    shutdown: CancellationToken,
    external: StateSource,
}

impl<B: CredentialPersistence + 'static> CredentialServiceBuilder<B> {
    /// Provide every mandatory collaborator. Omitting any is a compile
    /// error (the secure-construction guarantee, no runtime check).
    // guard-justified: the nine mandatory collaborators are the secure-construction
    // contract; bundling them into a params struct just moves the arity to that
    // struct's single literal at the call site.
    #[expect(clippy::too_many_arguments)]
    pub(crate) fn new(
        raw_store: B,
        key_provider: Arc<dyn KeyProvider>,
        audit_sink: Arc<dyn AuditSink>,
        pending_store: ErasedPendingStore,
        registry: Arc<CredentialRegistry>,
        ops: Arc<DispatchOps<ErasedPendingStore>>,
        observer: Arc<dyn CredentialObserver>,
        lease_config: LeaseLifecycleConfig,
        shutdown: CancellationToken,
    ) -> Self {
        Self {
            raw_store,
            key_provider,
            audit_sink,
            pending_store,
            registry,
            ops,
            observer,
            lease_config,
            shutdown,
            external: StateSource::LocalEncrypted,
        }
    }

    /// Configure an external provider chain as the credential
    /// [`StateSource`] instead of the local encrypted store.
    ///
    /// **Not yet wired.** This records the provider on the service, but
    /// the resolution path that routes through an external chain is the
    /// external provider bridge external-source bridge, which is out of this
    /// crate's current scope (see the credential-runtime subsystem spec §8).
    /// Until that lands, a service built with an external source rejects
    /// every secret-resolving call (`create` / `resolve` /
    /// `continue_resolve`) with
    /// [`CredentialServiceError::ExternalSourceNotWired`]
    /// rather than silently resolving from the local store (which would
    /// hand back material from the wrong source). The default
    /// [`StateSource::LocalEncrypted`] is fully functional.
    #[must_use = "builder methods must be chained or built"]
    pub(crate) fn external_providers(
        mut self,
        provider: Arc<dyn nebula_credential::provider::ExternalProvider>,
    ) -> Self {
        self.external = StateSource::External(provider);
        self
    }

    /// Compose the secure layered store + engine resolver + lease lifecycle
    /// and return the service.
    ///
    /// Runs a startup invariant first: every capability the registry
    /// advertises (in the four ops-modeled capabilities) must have a
    /// registered operation closure, so discovery and dispatch cannot
    /// advertise a capability that would fail at first call.
    ///
    /// # Errors
    ///
    /// [`CredentialServiceError::CapabilityWithoutOps`] when a registered
    /// credential type advertises `refresh` / `test` / `revoke` /
    /// `interactive` but its matching `register_*_ops` call was skipped at
    /// the composition root.
    pub(crate) fn build(self) -> Result<CredentialService, CredentialServiceError> {
        // registry-advertised capabilities ⊆ ops-registered closures, per
        // credential key. DYNAMIC is a lease concern with no ops closure, so
        // the subset is scoped to the four ops-modeled capabilities.
        let ops_modeled = Capabilities::REFRESHABLE
            | Capabilities::TESTABLE
            | Capabilities::REVOCABLE
            | Capabilities::INTERACTIVE;
        for key in self.registry.iter_keys() {
            let advertised = self
                .registry
                .capabilities_of(key)
                .unwrap_or_default()
                .intersection(ops_modeled);
            let missing = advertised.difference(self.ops.capabilities_of(key));
            if !missing.is_empty() {
                return Err(CredentialServiceError::CapabilityWithoutOps {
                    capability: first_missing_capability(missing).to_owned(),
                    key: key.to_owned(),
                });
            }
        }

        let encrypted = EncryptionLayer::new(self.raw_store, self.key_provider);
        let persistence: Arc<dyn CredentialPersistence> = Arc::new(encrypted);
        let layered = AuditLayer::new(Arc::clone(&persistence), self.audit_sink);
        let store: Arc<dyn CredentialPersistence> = Arc::new(layered);
        let refresh_coordinator = Arc::new(
            default_in_memory_coordinator()
                .map_err(|e| CredentialServiceError::Internal(e.to_string()))?,
        );
        let transport = Arc::new(ReqwestRefreshTransport);
        let resolver = CredentialResolver::with_dependencies(
            Arc::clone(&store),
            refresh_coordinator,
            transport,
        )
        .with_event_bus(self.observer.event_bus());
        let lease = LeaseLifecycle::spawn(
            self.lease_config,
            self.observer.lease_bus(),
            self.observer.metrics(),
            self.shutdown,
        );
        Ok(CredentialService::from_secure_parts(
            store,
            resolver,
            lease,
            self.pending_store,
            self.registry,
            self.ops,
            self.observer,
            self.external,
        ))
    }
}

/// Name the first ops-modeled capability present in `missing`, for the
/// [`CredentialServiceError::CapabilityWithoutOps`] message. `missing` is
/// always non-empty at the call site.
fn first_missing_capability(missing: Capabilities) -> &'static str {
    if missing.contains(Capabilities::REFRESHABLE) {
        "refresh"
    } else if missing.contains(Capabilities::TESTABLE) {
        "test"
    } else if missing.contains(Capabilities::REVOCABLE) {
        "revoke"
    } else if missing.contains(Capabilities::INTERACTIVE) {
        "interactive"
    } else {
        "unknown"
    }
}
