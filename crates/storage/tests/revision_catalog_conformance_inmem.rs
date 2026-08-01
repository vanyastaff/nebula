//! Exact plan/flavor catalog conformance for the in-memory reference model.
//!
//! The in-memory adapter is the reference/conformance backend — it is never a
//! deployment target. Running the shared oracle against it keeps the reference
//! model and the two SQL deployment backends answering identically; a
//! divergence shows up as the same named case failing on one of the three.

#[macro_use]
#[path = "support/revision_catalog_oracle.rs"]
mod oracle;

use nebula_storage::inmem::{InMemoryExecutionStore, InMemoryPlanFlavorCatalog};

/// A catalog over its own empty execution store.
///
/// The catalog holds the store's shared state, so the store value itself does
/// not need to outlive this call.
async fn catalog() -> Option<InMemoryPlanFlavorCatalog> {
    Some(InMemoryPlanFlavorCatalog::new(
        &InMemoryExecutionStore::new(),
    ))
}

revision_catalog_conformance_suite!(catalog());
