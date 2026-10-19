//! Node-result conformance for the in-memory reference model.
//!
//! The in-memory adapter is never a deployment target; running the shared
//! oracle against it keeps the reference model and the two SQL deployment
//! backends answering identically.

#[macro_use]
#[path = "support/node_result_oracle.rs"]
#[expect(
    dead_code,
    reason = "the in-memory store is composed apart from the execution store and does not \
              check a row's execution, so the relational cases do not run against it"
)]
mod oracle;

use nebula_storage::{InMemoryExecutionStore, InMemoryNodeResultStore};

async fn store() -> Option<(InMemoryNodeResultStore, InMemoryExecutionStore)> {
    Some((
        InMemoryNodeResultStore::new(),
        InMemoryExecutionStore::new(),
    ))
}

node_result_conformance_suite!(store());
