//! Trigger-dedup inbox row DTO.
//!
//! `TriggerDedupRow` is the guard inserted atomically with the `Start` job
//! to enforce first-writer-wins trigger fan-out.  `event_id` is required and
//! must be a source-natural idempotency key — never a freshly minted ULID.
//!
//! The guarded execution id is NOT carried on this row; it is sourced from
//! `JobDispatchMsg::execution_id` (the `start` argument passed to
//! `TriggerDedupInbox::claim_and_materialize_start`) inside each backend's
//! atomic compose.  This avoids an API trap where a caller could pass a
//! different id on the row and have it silently ignored.
use crate::Scope;
use serde::{Deserialize, Serialize};

/// One trigger-dedup guard row.
///
/// The uniqueness constraint is `PRIMARY KEY(workspace_id, org_id, trigger_id,
/// event_id)` — scoped per tenant, so two tenants sharing the same
/// `(trigger_id, event_id)` pair are never deduplicated against each other.  A
/// second delivery of the same event from the same trigger *within one tenant*
/// finds the row already present and is discarded (`DispatchKind::Duplicate`).
///
/// The guarded execution id is sourced from `start.execution_id` inside the
/// atomic compose, not carried on this struct.
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
    /// `TriggerDedupInbox::claim_and_materialize_start`.
    pub event_id: String,
    /// Tenant scope.
    pub scope: Scope,
    /// Wall-clock creation time (RFC 3339).
    pub created_at: String,
}

impl TriggerDedupRow {
    /// Construct a trigger-dedup guard row.
    ///
    /// The execution id guarded by this row is supplied separately as
    /// `start.execution_id` in `claim_and_materialize_start` — it is not
    /// a parameter here.
    pub fn new(
        trigger_id: impl Into<String>,
        event_id: impl Into<String>,
        scope: Scope,
        created_at: impl Into<String>,
    ) -> Self {
        Self {
            trigger_id: trigger_id.into(),
            event_id: event_id.into(),
            scope,
            created_at: created_at.into(),
        }
    }
}
