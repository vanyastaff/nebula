//! Dyn-erasure bridge for the generic pending-state store.
//!
//! Credential persistence is directly object-safe in `nebula-storage-port`;
//! there is deliberately no parallel mirror or wrapper here. Pending-state
//! operations remain generic over `<P: PendingState>`, so this module retains
//! only their byte-core erasure:
//!
//! - [`DynPendingStateStore`] is the object-safe **byte-core** mirror of
//!   [`PendingStateStore`]: the `<P: PendingState>` generic is erased to
//!   `Zeroizing<Vec<u8>>` (serialize on put, deserialize on
//!   get/consume/get_bound). A
//!   blanket impl gives every `DynPendingStateStore` a typed
//!   [`PendingStateStore`], and [`ErasedPendingStore`] wraps an
//!   `Arc<dyn DynPendingStateStore>` back into a concrete
//!   [`PendingStateStore`].
//!
//! # Coherence
//!
//! After this module lands, **no type implements [`PendingStateStore`]
//! directly** except the blanket here and [`ErasedPendingStore`]: backend
//! pending stores implement [`DynPendingStateStore`] and acquire
//! [`PendingStateStore`] via the blanket. This keeps the blanket
//! unambiguous.
//!
//! Do **not** add the symmetric `impl<T: PendingStateStore> DynPendingStateStore
//! for T`: it would make `ErasedPendingStore` (which has an explicit
//! [`PendingStateStore`] impl) transitively a [`DynPendingStateStore`], so the
//! blanket above would then also cover it and collide with the explicit impl
//! (E0119). The one-directional bridge (`DynPendingStateStore` → typed
//! [`PendingStateStore`]) is load-bearing for coherence.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use zeroize::Zeroizing;

use crate::pending_store::{PendingStateStore, PendingStoreError};
use crate::{PendingState, PendingToken};

/// Boxed, `Send` future used by both bridge traits. `'a` ties the future
/// to the borrow of `&self` (and any borrowed arguments) so the returned
/// future may borrow from the store for its whole lifetime.
type BoxFut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

// ── Pending store bridge ─────────────────────────────────────────────────

/// Object-safe **byte-core** mirror of [`PendingStateStore`].
///
/// [`PendingStateStore`]'s `put`/`get`/`get_bound`/`consume` are generic
/// over `<P: PendingState>`, which makes the trait non-object-safe. This
/// bridge erases the generic to `Zeroizing<Vec<u8>>`: the typed [`PendingStateStore`]
/// blanket below serializes on `put` and deserializes on the read paths,
/// so a `dyn DynPendingStateStore` carries exactly the same serde round-trip
/// the original direct impls did. `delete` carries no type parameter and is
/// mirrored unchanged.
///
/// Implementors store the raw bytes plus the binding tuple
/// (`credential_kind`, `owner_id`, `session_id`) and the absolute expiry
/// (`Utc::now() + expires_in`), and enforce TTL eviction + 4-dimensional
/// token binding on the read paths — see
/// [`PendingStateStore`] for the security contract.
pub trait DynPendingStateStore: Send + Sync {
    /// Byte-core mirror of [`PendingStateStore::put`]. `data` is the
    /// already-serialized pending state; `expires_in` is the type's
    /// [`PendingState::expires_in`] TTL (the store computes the absolute
    /// expiry as `Utc::now() + expires_in`).
    fn put_serialized<'a>(
        &'a self,
        credential_kind: &'a str,
        owner_id: &'a str,
        session_id: &'a str,
        data: Zeroizing<Vec<u8>>,
        expires_in: Duration,
    ) -> BoxFut<'a, Result<PendingToken, PendingStoreError>>;

    /// Byte-core mirror of [`PendingStateStore::get`]. Returns the stored
    /// serialized bytes without consuming or binding-checking.
    fn get_serialized<'a>(
        &'a self,
        token: &'a PendingToken,
    ) -> BoxFut<'a, Result<Zeroizing<Vec<u8>>, PendingStoreError>>;

    /// Byte-core mirror of [`PendingStateStore::get_bound`]. Returns the
    /// stored serialized bytes after validating the 3 binding dimensions
    /// (without consuming).
    fn get_bound_serialized<'a>(
        &'a self,
        credential_kind: &'a str,
        token: &'a PendingToken,
        owner_id: &'a str,
        session_id: &'a str,
    ) -> BoxFut<'a, Result<Zeroizing<Vec<u8>>, PendingStoreError>>;

    /// Byte-core mirror of [`PendingStateStore::consume`]. Validates all 4
    /// dimensions then atomically reads-and-deletes, returning the stored
    /// serialized bytes.
    fn consume_serialized<'a>(
        &'a self,
        credential_kind: &'a str,
        token: &'a PendingToken,
        owner_id: &'a str,
        session_id: &'a str,
    ) -> BoxFut<'a, Result<Zeroizing<Vec<u8>>, PendingStoreError>>;

    /// Mirror of [`PendingStateStore::delete`] (no type parameter).
    fn delete<'a>(&'a self, token: &'a PendingToken) -> BoxFut<'a, Result<(), PendingStoreError>>;
}

/// Typed [`PendingStateStore`] for every [`DynPendingStateStore`].
///
/// Serializes on `put` (`serde_json::to_vec` + [`PendingState::expires_in`])
/// and deserializes on the read paths (`serde_json::from_slice`); serde
/// failures map to [`PendingStoreError::Backend`]. This is the **sole**
/// `PendingStateStore` impl in the workspace other than
/// [`ErasedPendingStore`] — backend stores implement
/// [`DynPendingStateStore`] and inherit [`PendingStateStore`] here.
impl<T: DynPendingStateStore + ?Sized> PendingStateStore for T {
    async fn put<P: PendingState>(
        &self,
        credential_kind: &str,
        owner_id: &str,
        session_id: &str,
        pending: P,
    ) -> Result<PendingToken, PendingStoreError> {
        // Pending interactive state (PKCE verifier, partial OAuth2 secrets) is
        // serialized in cleartext only into this zeroizing buffer. The current
        // first-party adapter is ephemeral in-memory and writes no disk; a
        // future durable adapter must provide encryption at rest explicitly.
        // Outside this scope the state's secret fields redact.
        let data = Zeroizing::new(
            crate::serde_secret::expose_for_serialization(|| serde_json::to_vec(&pending))
                .map_err(|e| PendingStoreError::Backend(Box::new(e)))?,
        );
        let expires_in = pending.expires_in();
        self.put_serialized(credential_kind, owner_id, session_id, data, expires_in)
            .await
    }

    async fn get<P: PendingState>(&self, token: &PendingToken) -> Result<P, PendingStoreError> {
        let data = self.get_serialized(token).await?;
        serde_json::from_slice(&data).map_err(|e| PendingStoreError::Backend(Box::new(e)))
    }

    async fn get_bound<P: PendingState>(
        &self,
        credential_kind: &str,
        token: &PendingToken,
        owner_id: &str,
        session_id: &str,
    ) -> Result<P, PendingStoreError> {
        let data = self
            .get_bound_serialized(credential_kind, token, owner_id, session_id)
            .await?;
        serde_json::from_slice(&data).map_err(|e| PendingStoreError::Backend(Box::new(e)))
    }

    async fn consume<P: PendingState>(
        &self,
        credential_kind: &str,
        token: &PendingToken,
        owner_id: &str,
        session_id: &str,
    ) -> Result<P, PendingStoreError> {
        let data = self
            .consume_serialized(credential_kind, token, owner_id, session_id)
            .await?;
        serde_json::from_slice(&data).map_err(|e| PendingStoreError::Backend(Box::new(e)))
    }

    async fn delete(&self, token: &PendingToken) -> Result<(), PendingStoreError> {
        DynPendingStateStore::delete(self, token).await
    }
}

/// Concrete, `Clone`-able [`PendingStateStore`] over an erased backend.
///
/// Wraps an `Arc<dyn DynPendingStateStore>` and forwards the typed
/// generic methods to the blanket impl on `dyn DynPendingStateStore`
/// (which performs the serde round-trip). This is the fixed `PS` type the
/// facade and `DispatchOps` are monomorphized over, so the pending backend
/// can be swapped without re-typing any consumer.
#[derive(Clone)]
pub struct ErasedPendingStore(Arc<dyn DynPendingStateStore>);

impl std::fmt::Debug for ErasedPendingStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ErasedPendingStore").finish_non_exhaustive()
    }
}

impl ErasedPendingStore {
    /// Wrap an erased pending backend.
    #[must_use]
    pub fn new(inner: Arc<dyn DynPendingStateStore>) -> Self {
        Self(inner)
    }
}

impl PendingStateStore for ErasedPendingStore {
    // `async fn` so the impl's opaque return types capture the borrows;
    // each forwards to the blanket `PendingStateStore` impl on
    // `dyn DynPendingStateStore` (`*self.0`), which does the serde
    // round-trip.
    async fn put<P: PendingState>(
        &self,
        credential_kind: &str,
        owner_id: &str,
        session_id: &str,
        pending: P,
    ) -> Result<PendingToken, PendingStoreError> {
        (*self.0)
            .put(credential_kind, owner_id, session_id, pending)
            .await
    }

    async fn get<P: PendingState>(&self, token: &PendingToken) -> Result<P, PendingStoreError> {
        (*self.0).get(token).await
    }

    async fn get_bound<P: PendingState>(
        &self,
        credential_kind: &str,
        token: &PendingToken,
        owner_id: &str,
        session_id: &str,
    ) -> Result<P, PendingStoreError> {
        (*self.0)
            .get_bound(credential_kind, token, owner_id, session_id)
            .await
    }

    async fn consume<P: PendingState>(
        &self,
        credential_kind: &str,
        token: &PendingToken,
        owner_id: &str,
        session_id: &str,
    ) -> Result<P, PendingStoreError> {
        (*self.0)
            .consume(credential_kind, token, owner_id, session_id)
            .await
    }

    async fn delete(&self, token: &PendingToken) -> Result<(), PendingStoreError> {
        PendingStateStore::delete(&*self.0, token).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Compile-time object-safety probe for the one remaining erasure bridge.
    // Credential persistence has its own direct dyn probe in storage-port.
    fn _assert_object_safe(_: Arc<dyn DynPendingStateStore>) {}

    #[test]
    fn bridge_traits_are_object_safe() {
        // Compiling `_assert_object_safe`'s parameter above is the proof.
    }
}
