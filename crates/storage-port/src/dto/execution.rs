//! Execution row DTO.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::execution_listing::{ExecutionListingStatus, MicrosInstant};
use crate::Scope;

/// Parameters for inserting a new execution row inside a compose transaction.
///
/// The execution id and tenant scope travel on the enclosing
/// [`crate::dto::MaterializedStart`] — single source of truth in the compose
/// method; this struct carries only the fields that differ.
///
/// Construct via [`NewExecution::new`]; struct literal syntax is
/// unavailable from external crates (`#[non_exhaustive]`).
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct NewExecution<'a> {
    /// Owning workflow id (opaque string form).
    pub workflow_id: &'a str,
    /// Initial execution state blob.
    pub initial_state: &'a serde_json::Value,
}

impl<'a> NewExecution<'a> {
    /// Construct a new-execution parameter set.
    pub fn new(workflow_id: &'a str, initial_state: &'a serde_json::Value) -> Self {
        Self {
            workflow_id,
            initial_state,
        }
    }
}

/// One execution row as the port exposes it.
///
/// `state` is opaque `serde_json::Value` by design: the port never
/// interprets execution state — the execution FSM lives in
/// `nebula-execution`. `fencing` is the lease generation that last wrote the
/// row (`None` before any lease is acquired).
// guard-justified: `state` is `serde_json::Value`, which is not `Eq`
// (it can hold a float). `Eq` is therefore not derivable; the clippy
// hint is a false positive for any DTO carrying an opaque JSON payload.
#[expect(clippy::derive_partial_eq_without_eq)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExecutionRecord {
    /// Execution id (opaque string form).
    pub id: String,
    /// Owning workflow id (opaque string form).
    pub workflow_id: String,
    /// Tenant scope this row belongs to.
    pub scope: Scope,
    /// Optimistic-CAS version.
    pub version: u64,
    /// Execution status from the listing projection of the latest snapshot.
    pub status: ExecutionListingStatus,
    /// Opaque execution state blob.
    pub state: serde_json::Value,
    /// Replica currently holding the lease, if any.
    pub lease_holder: Option<String>,
    /// Lease fencing generation that last wrote the row, if any.
    pub fencing: Option<u64>,
    /// Creation instant.
    pub created_at: DateTime<Utc>,
    /// Last state change.
    pub updated_at: DateTime<Utc>,
}

/// An execution whose earliest durable timer is due, as listed by
/// [`crate::store::ExecutionStore::list_due_timers`].
#[derive(Debug, Clone, PartialEq)]
pub struct DueTimer {
    /// The listed timer instant (the row's `next_wake_at`).
    pub next_wake_at: MicrosInstant,
    /// The execution row.
    pub record: ExecutionRecord,
}

impl DueTimer {
    /// Position after this row, to continue the listing from.
    #[must_use]
    pub fn cursor(&self) -> DueTimerCursor {
        DueTimerCursor {
            next_wake_at: self.next_wake_at,
            id: self.record.id.clone(),
        }
    }
}

/// Keyset position in a due-timer listing, ordered by `(next_wake_at, id)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DueTimerCursor {
    next_wake_at: MicrosInstant,
    id: String,
}

impl DueTimerCursor {
    /// The timer instant of the last listed row.
    #[must_use]
    pub const fn next_wake_at(&self) -> MicrosInstant {
        self.next_wake_at
    }

    /// The execution id of the last listed row.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }
}
