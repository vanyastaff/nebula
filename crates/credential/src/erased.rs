//! Dyn-erasure bridge for the two RPITIT credential storage ports.
//!
//! [`CredentialStore`] and [`PendingStateStore`] both use return-position
//! `impl Future` in trait position (RPITIT); [`PendingStateStore`]'s
//! non-`delete` methods are additionally generic over `<P: PendingState>`.
//! Neither is object-safe, so neither can be stored behind `dyn`. The
//! [`CredentialService`](../../nebula_credential_runtime/index.html) facade
//! must be **non-generic** (ADR-0088 D4) so a durable backend can be swapped
//! in without re-monomorphizing every consumer; that requires erasing both
//! ports to `dyn`.
//!
//! This module provides the hand-rolled boxed-future bridge that does it,
//! mirroring the `ProviderFuture` idiom in
//! [`ProviderFuture`](crate::ProviderFuture) — no `async_trait`, no
//! `bon` (records the ADR-0088 D4 bon-deviation):
//!
//! - [`DynCredentialStore`] — object-safe boxed-future mirror of
//!   [`CredentialStore`]; a blanket impl gives every `CredentialStore` a
//!   `DynCredentialStore`, and [`ErasedCredentialStore`] wraps an
//!   `Arc<dyn DynCredentialStore>` back into a concrete `CredentialStore`.
//! - [`DynPendingStateStore`] — object-safe **byte-core** mirror of
//!   [`PendingStateStore`]: the `<P: PendingState>` generic is erased to
//!   `Vec<u8>` (serialize on put, deserialize on get/consume/get_bound). A
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

use crate::pending_store::{PendingStateStore, PendingStoreError};
use crate::store::{CredentialStore, PutMode, StoreError, StoredCredential};
use crate::{PendingState, PendingToken};

/// Boxed, `Send` future used by both bridge traits. `'a` ties the future
/// to the borrow of `&self` (and any borrowed arguments) so the returned
/// future may borrow from the store for its whole lifetime.
type BoxFut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

// ── Credential store bridge ──────────────────────────────────────────────

/// Object-safe boxed-future mirror of [`CredentialStore`].
///
/// Each method ties `&self` and every borrowed argument to a single
/// lifetime `'a`, so the returned `BoxFut` may borrow from the store. A
/// blanket impl (`impl<T: CredentialStore> DynCredentialStore for T`) makes
/// every `CredentialStore` usable as `dyn DynCredentialStore`, and
/// [`ErasedCredentialStore`] erases the backend at the resolver→store
/// boundary so the facade can be non-generic.
///
/// The five methods mirror [`CredentialStore`] one-for-one; see that trait
/// for the per-method contract and error semantics.
pub trait DynCredentialStore: Send + Sync {
    /// Boxed-future mirror of [`CredentialStore::get`].
    fn get<'a>(&'a self, id: &'a str) -> BoxFut<'a, Result<StoredCredential, StoreError>>;

    /// Boxed-future mirror of [`CredentialStore::put`].
    fn put(
        &self,
        credential: StoredCredential,
        mode: PutMode,
    ) -> BoxFut<'_, Result<StoredCredential, StoreError>>;

    /// Boxed-future mirror of [`CredentialStore::delete`].
    fn delete<'a>(&'a self, id: &'a str) -> BoxFut<'a, Result<(), StoreError>>;

    /// Boxed-future mirror of [`CredentialStore::list`].
    fn list<'a>(
        &'a self,
        state_kind: Option<&'a str>,
    ) -> BoxFut<'a, Result<Vec<String>, StoreError>>;

    /// Boxed-future mirror of [`CredentialStore::exists`].
    fn exists<'a>(&'a self, id: &'a str) -> BoxFut<'a, Result<bool, StoreError>>;
}

impl<T: CredentialStore> DynCredentialStore for T {
    fn get<'a>(&'a self, id: &'a str) -> BoxFut<'a, Result<StoredCredential, StoreError>> {
        Box::pin(CredentialStore::get(self, id))
    }

    fn put(
        &self,
        credential: StoredCredential,
        mode: PutMode,
    ) -> BoxFut<'_, Result<StoredCredential, StoreError>> {
        Box::pin(CredentialStore::put(self, credential, mode))
    }

    fn delete<'a>(&'a self, id: &'a str) -> BoxFut<'a, Result<(), StoreError>> {
        Box::pin(CredentialStore::delete(self, id))
    }

    fn list<'a>(
        &'a self,
        state_kind: Option<&'a str>,
    ) -> BoxFut<'a, Result<Vec<String>, StoreError>> {
        Box::pin(CredentialStore::list(self, state_kind))
    }

    fn exists<'a>(&'a self, id: &'a str) -> BoxFut<'a, Result<bool, StoreError>> {
        Box::pin(CredentialStore::exists(self, id))
    }
}

/// Concrete, `Clone`-able [`CredentialStore`] over an erased backend.
///
/// Wraps an `Arc<dyn DynCredentialStore>` and re-implements
/// [`CredentialStore`] by forwarding to the boxed-future methods. This is
/// the type the facade's resolver is monomorphized over, so the backend can
/// be swapped (in-memory ↔ SQLite ↔ Postgres) without re-typing any
/// consumer.
#[derive(Clone)]
pub struct ErasedCredentialStore(Arc<dyn DynCredentialStore>);

impl std::fmt::Debug for ErasedCredentialStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ErasedCredentialStore")
            .finish_non_exhaustive()
    }
}

impl ErasedCredentialStore {
    /// Wrap an erased credential backend.
    #[must_use]
    pub fn new(inner: Arc<dyn DynCredentialStore>) -> Self {
        Self(inner)
    }
}

impl CredentialStore for ErasedCredentialStore {
    // `async fn` (not a bare forward returning the `BoxFut`) so the impl's
    // opaque return type captures the `&self`/arg borrows the boxed future
    // holds — a bare `self.0.get(id)` is E0700 (captured lifetime not in
    // bounds).
    async fn get(&self, id: &str) -> Result<StoredCredential, StoreError> {
        self.0.get(id).await
    }

    async fn put(
        &self,
        credential: StoredCredential,
        mode: PutMode,
    ) -> Result<StoredCredential, StoreError> {
        self.0.put(credential, mode).await
    }

    async fn delete(&self, id: &str) -> Result<(), StoreError> {
        self.0.delete(id).await
    }

    async fn list(&self, state_kind: Option<&str>) -> Result<Vec<String>, StoreError> {
        self.0.list(state_kind).await
    }

    async fn exists(&self, id: &str) -> Result<bool, StoreError> {
        self.0.exists(id).await
    }
}

// ── Pending store bridge ─────────────────────────────────────────────────

/// Object-safe **byte-core** mirror of [`PendingStateStore`].
///
/// [`PendingStateStore`]'s `put`/`get`/`get_bound`/`consume` are generic
/// over `<P: PendingState>`, which makes the trait non-object-safe. This
/// bridge erases the generic to `Vec<u8>`: the typed [`PendingStateStore`]
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
        data: Vec<u8>,
        expires_in: Duration,
    ) -> BoxFut<'a, Result<PendingToken, PendingStoreError>>;

    /// Byte-core mirror of [`PendingStateStore::get`]. Returns the stored
    /// serialized bytes without consuming or binding-checking.
    fn get_serialized<'a>(
        &'a self,
        token: &'a PendingToken,
    ) -> BoxFut<'a, Result<Vec<u8>, PendingStoreError>>;

    /// Byte-core mirror of [`PendingStateStore::get_bound`]. Returns the
    /// stored serialized bytes after validating the 3 binding dimensions
    /// (without consuming).
    fn get_bound_serialized<'a>(
        &'a self,
        credential_kind: &'a str,
        token: &'a PendingToken,
        owner_id: &'a str,
        session_id: &'a str,
    ) -> BoxFut<'a, Result<Vec<u8>, PendingStoreError>>;

    /// Byte-core mirror of [`PendingStateStore::consume`]. Validates all 4
    /// dimensions then atomically reads-and-deletes, returning the stored
    /// serialized bytes.
    fn consume_serialized<'a>(
        &'a self,
        credential_kind: &'a str,
        token: &'a PendingToken,
        owner_id: &'a str,
        session_id: &'a str,
    ) -> BoxFut<'a, Result<Vec<u8>, PendingStoreError>>;

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
        let data =
            serde_json::to_vec(&pending).map_err(|e| PendingStoreError::Backend(Box::new(e)))?;
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

    // Compile-time object-safety probe: naming `Arc<dyn …>` for both bridge
    // traits proves they are dyn-compatible — the contract the facade's
    // `store: Arc<dyn DynCredentialStore>` / `ErasedPendingStore(Arc<dyn …>)`
    // rely on. Never called; mirrors `crates/storage-port/tests/object_safe.rs`.
    fn _assert_object_safe(_a: Arc<dyn DynCredentialStore>, _b: Arc<dyn DynPendingStateStore>) {}

    #[test]
    fn bridge_traits_are_object_safe() {
        // Compiling `_assert_object_safe`'s `Arc<dyn …>` parameters above is
        // the proof; this test exists so the probe participates in the
        // test target.
    }
}
