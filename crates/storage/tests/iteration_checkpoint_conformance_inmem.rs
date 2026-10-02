//! Iteration-checkpoint conformance for the in-memory reference model.
//!
//! The in-memory adapter is never a deployment target; running the shared
//! oracle against it keeps the reference model and the two SQL deployment
//! backends answering identically.

#[macro_use]
#[path = "support/iteration_checkpoint_oracle.rs"]
mod oracle;

use nebula_storage::{InMemoryCheckpointStore, InMemoryExecutionStore};

async fn store() -> Option<(InMemoryCheckpointStore, InMemoryExecutionStore)> {
    let execution = InMemoryExecutionStore::new();
    Some((InMemoryCheckpointStore::new(&execution), execution))
}

iteration_checkpoint_conformance_suite!(store());
