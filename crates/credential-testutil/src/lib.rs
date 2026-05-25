//! Internal test utilities for `nebula-credential` and downstream
//! consumers. Crate is `publish = false`; do not consume from production
//! code.

#![forbid(unsafe_code)]

pub mod pending_store_memory;
pub mod store_memory;

pub use pending_store_memory::InMemoryPendingStore;
pub use store_memory::InMemoryStore;

/// Convenience helper: build a fully wired in-memory `(Store, PendingStore)` pair.
#[must_use]
pub fn in_memory_pair() -> (InMemoryStore, InMemoryPendingStore) {
    (InMemoryStore::new(), InMemoryPendingStore::new())
}
