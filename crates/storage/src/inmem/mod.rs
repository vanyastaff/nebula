//! In-memory adapter — the `nebula-storage-port` implementation for tests,
//! local single-process runs, and the loom probe.
//!
//! Each store is one `parking_lot::Mutex`-guarded map. `commit` performs the
//! whole §12.2 triple (CAS + lease fencing + state + outbox + journal) under
//! a single lock, so it behaviourally models the single-writer contract the
//! conformance suite asserts. The scope predicate is enforced exactly as the
//! SQL backends enforce `WHERE workspace_id = ? AND org_id = ?`, so
//! cross-tenant denial is proven uniformly across backends.

mod execution;

pub use execution::{InMemoryExecutionStore, InMemoryIdempotencyGuard};
