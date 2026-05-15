//! `CredentialService<B, PS>` — the sole public entry to the credential
//! management bounded context. Generic over the raw backend `B` and
//! pending store `PS` (both RPITIT non-object-safe; the params live only
//! on the struct, never in operation signatures). All invariant-bearing
//! composition is crate-private: the only constructor path is
//! [`CredentialServiceBuilder`](crate::builder::CredentialServiceBuilder),
//! whose `build()` wraps the raw backend in the layered store so an
//! unencrypted/mis-composed service is unrepresentable.

use std::sync::Arc;

use nebula_credential::CredentialRegistry;
use nebula_credential::pending_store::PendingStateStore;
use nebula_credential::store::CredentialStore;
use nebula_engine::credential::{CredentialResolver, LeaseLifecycle};
use nebula_storage::credential::{AuditLayer, CacheLayer, EncryptionLayer};

use crate::dispatch::CredentialDispatch;
use crate::observer::CredentialObserver;
use crate::state_source::StateSource;

/// Crate-private layered store stack composed once at `build()`:
/// `Audit(Cache(Encryption(raw)))`. `Encryption` is adjacent to the raw
/// backend so persisted bytes are always ciphertext (spec §6 #7).
pub(crate) type LayeredStore<B> = AuditLayer<CacheLayer<EncryptionLayer<B>>>;

/// Sole public entry to the credential management bounded context.
///
/// Constructed only via
/// [`CredentialServiceBuilder`](crate::builder::CredentialServiceBuilder).
// Fields are read by the credential operations (create / get / list /
// update / delete / test / refresh / revoke / resolve / discovery),
// which are implemented in sibling modules of this crate.
#[allow(dead_code)]
pub struct CredentialService<B: CredentialStore, PS: PendingStateStore> {
    pub(crate) store: Arc<LayeredStore<B>>,
    pub(crate) resolver: CredentialResolver<LayeredStore<B>>,
    pub(crate) lease: LeaseLifecycle,
    pub(crate) pending: PS,
    pub(crate) registry: Arc<CredentialRegistry>,
    pub(crate) dispatch: Arc<CredentialDispatch>,
    pub(crate) observer: Arc<dyn CredentialObserver>,
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
            observer,
            source,
        }
    }

    /// Active dynamic-lease count (smoke accessor; operations land in
    /// later increments).
    pub async fn active_lease_count(&self) -> usize {
        self.lease.active_lease_count().await
    }
}
