//! `CredentialServiceBuilder` — the only construction path for
//! [`CredentialService`].
//!
//! Panel-refined shape (spec §5): every mandatory collaborator is a
//! by-value argument to [`CredentialServiceBuilder::new`], so omitting
//! one is a compile error; optional collaborators are chained setters;
//! [`build`](CredentialServiceBuilder::build) is infallible and holds
//! **no `Option`/`unwrap`** for mandatory state. `build()` performs the
//! crate-private secure composition `Audit(Cache(Encryption(raw)))` +
//! the engine resolver + lease lifecycle — so an unencrypted or
//! mis-composed service cannot be constructed.

use std::sync::Arc;

use nebula_credential::pending_store::PendingStateStore;
use nebula_credential::store::CredentialStore;
use nebula_credential::{Capabilities, CredentialRegistry};
use nebula_engine::credential::{
    CredentialResolver, LeaseLifecycle, LeaseLifecycleConfig, RefreshCoordinator,
};
use nebula_storage::credential::{
    AuditLayer, AuditSink, CacheConfig, CacheLayer, EncryptionLayer, KeyProvider,
};
use tokio_util::sync::CancellationToken;

use crate::error::CredentialServiceError;
use crate::observer::CredentialObserver;
use crate::ops::DispatchOps;
use crate::service::CredentialService;
use crate::state_source::StateSource;

/// Builder for [`CredentialService`]. Construct via [`Self::new`] (all
/// mandatory collaborators), chain optional setters, then [`build`].
///
/// [`build`]: Self::build
pub struct CredentialServiceBuilder<B: CredentialStore, PS: PendingStateStore> {
    raw_store: B,
    key_provider: Arc<dyn KeyProvider>,
    audit_sink: Arc<dyn AuditSink>,
    cache_config: CacheConfig,
    pending_store: PS,
    registry: Arc<CredentialRegistry>,
    ops: Arc<DispatchOps<PS>>,
    observer: Arc<dyn CredentialObserver>,
    lease_config: LeaseLifecycleConfig,
    shutdown: CancellationToken,
    refresh_coordinator: Option<Arc<RefreshCoordinator>>,
    external: StateSource,
}

impl<B: CredentialStore, PS: PendingStateStore> CredentialServiceBuilder<B, PS> {
    /// Provide every mandatory collaborator. Omitting any is a compile
    /// error (the secure-construction guarantee, no runtime check).
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        raw_store: B,
        key_provider: Arc<dyn KeyProvider>,
        audit_sink: Arc<dyn AuditSink>,
        cache_config: CacheConfig,
        pending_store: PS,
        registry: Arc<CredentialRegistry>,
        ops: Arc<DispatchOps<PS>>,
        observer: Arc<dyn CredentialObserver>,
        lease_config: LeaseLifecycleConfig,
        shutdown: CancellationToken,
    ) -> Self {
        Self {
            raw_store,
            key_provider,
            audit_sink,
            cache_config,
            pending_store,
            registry,
            ops,
            observer,
            lease_config,
            shutdown,
            refresh_coordinator: None,
            external: StateSource::LocalEncrypted,
        }
    }

    /// Override the default in-memory [`RefreshCoordinator`] with a
    /// production (durable claim-repo) one.
    #[must_use = "builder methods must be chained or built"]
    pub fn refresh_coordinator(mut self, rc: Arc<RefreshCoordinator>) -> Self {
        self.refresh_coordinator = Some(rc);
        self
    }

    /// Configure an external provider chain as the credential
    /// [`StateSource`] instead of the local encrypted store.
    ///
    /// **Not yet wired.** This records the provider on the service, but
    /// the resolution path that routes through an external chain is the
    /// external provider bridge external-source bridge, which is out of this
    /// crate's current scope (see  / the credential-runtime
    /// subsystem spec §8). Until that lands, a service built with an
    /// external source rejects every secret-resolving call
    /// (`create` / `resolve` / `continue_resolve`) with
    /// [`CredentialServiceError::ExternalSourceNotWired`](crate::CredentialServiceError::ExternalSourceNotWired)
    /// rather than silently resolving from the local store (which would
    /// hand back material from the wrong source). The default
    /// [`StateSource::LocalEncrypted`] is fully functional.
    #[must_use = "builder methods must be chained or built"]
    pub fn external_providers(
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
    pub fn build(self) -> Result<CredentialService<B, PS>, CredentialServiceError> {
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

        let store = AuditLayer::new(
            CacheLayer::new(
                EncryptionLayer::new(self.raw_store, self.key_provider),
                self.cache_config,
            ),
            self.audit_sink,
        );
        let store = Arc::new(store);
        let refresh_coordinator = self
            .refresh_coordinator
            .unwrap_or_else(|| Arc::new(RefreshCoordinator::new()));
        let resolver = CredentialResolver::new(Arc::clone(&store))
            .with_refresh_coordinator(refresh_coordinator)
            .with_event_bus(self.observer.event_bus());
        let lease = LeaseLifecycle::spawn(
            self.lease_config,
            self.observer.lease_bus(),
            self.observer.metrics(),
            self.shutdown,
        );
        Ok(CredentialService::__from_parts(
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
