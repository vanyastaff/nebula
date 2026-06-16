//! Trigger-dedup inbox row DTO.
//!
//! `TriggerDedupRow` is the guard inserted atomically with the `Start` job
//! to enforce first-writer-wins trigger fan-out.  `event_id` is required and
//! must be a source-natural idempotency key — never a freshly minted ULID.
use crate::Scope;
use serde::{Deserialize, Serialize};

/// One trigger-dedup guard row.
///
/// The uniqueness constraint is `PRIMARY KEY(workspace_id, org_id, trigger_id,
/// event_id)` — scoped per tenant, so two tenants sharing the same
/// `(trigger_id, event_id)` pair are never deduplicated against each other.  A
/// second delivery of the same event from the same trigger *within one tenant*
/// finds the row already present and is discarded (`DispatchOutcome::Duplicate`).
///
/// Construct via [`TriggerDedupRow::new`]; struct literal syntax is
/// unavailable from external crates (`#[non_exhaustive]`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct TriggerDedupRow {
    /// The trigger that fired (scoped to `scope`).
    pub trigger_id: String,
    /// Source-natural idempotency key for the event.
    ///
    /// Required (`String`, not `Option`) — a `TriggerDedupRow` is only
    /// created when dedup is desired.  Callers that want unconditional
    /// dispatch pass `None` as the `row` argument to
    /// `TriggerDedupInbox::claim_and_enqueue_start`.
    pub event_id: String,
    /// Tenant scope.
    pub scope: Scope,
    /// The execution that was started as a result of this trigger event.
    pub execution_id: String,
    /// Wall-clock creation time (RFC 3339).
    pub created_at: String,
}

impl TriggerDedupRow {
    /// Construct a trigger-dedup guard row.
    pub fn new(
        trigger_id: impl Into<String>,
        event_id: impl Into<String>,
        scope: Scope,
        execution_id: impl Into<String>,
        created_at: impl Into<String>,
    ) -> Self {
        Self {
            trigger_id: trigger_id.into(),
            event_id: event_id.into(),
            scope,
            execution_id: execution_id.into(),
            created_at: created_at.into(),
        }
    }
}
