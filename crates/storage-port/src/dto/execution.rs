//! Execution row DTO, its listing projection, and the history query.
use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::Scope;

/// Lifecycle status of an execution as the listing projection stores it.
///
/// A port-local mirror of `nebula_execution::ExecutionStatus` (the port
/// cannot depend on the execution crate): the engine maps one onto the other
/// with an exhaustive `match`, so a new execution status cannot reach storage
/// unmapped. The wire form is the snake_case name, identical to the
/// execution crate's serde form; it is what the `status` column stores.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionListingStatus {
    /// Created but not yet started.
    Created,
    /// Actively running nodes.
    Running,
    /// Paused by the user or the system.
    Paused,
    /// Cancellation requested; active nodes are draining.
    Cancelling,
    /// Every node completed successfully.
    Completed,
    /// A node failed and the execution could not continue.
    Failed,
    /// Cancelled after the cancellation was fully processed.
    Cancelled,
    /// The wall-clock budget ran out.
    TimedOut,
}

impl ExecutionListingStatus {
    /// Every status, in lifecycle order.
    pub const ALL: [Self; 8] = [
        Self::Created,
        Self::Running,
        Self::Paused,
        Self::Cancelling,
        Self::Completed,
        Self::Failed,
        Self::Cancelled,
        Self::TimedOut,
    ];

    /// The stored and wire name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Running => "running",
            Self::Paused => "paused",
            Self::Cancelling => "cancelling",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::TimedOut => "timed_out",
        }
    }

    /// Parse a stored name. Returns `None` for anything outside the closed set.
    #[must_use]
    pub fn from_stored(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|status| status.as_str() == value)
    }

    /// `true` once the execution can no longer change status.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Cancelled | Self::TimedOut
        )
    }

    /// The non-terminal statuses: what "running" means for a listing.
    #[must_use]
    pub fn active() -> BTreeSet<Self> {
        Self::ALL
            .into_iter()
            .filter(|status| !status.is_terminal())
            .collect()
    }
}

/// The queryable projection of an execution state snapshot.
///
/// Stored in dedicated columns next to the opaque `state` blob, so history
/// can be filtered and ordered without the port ever parsing the state. It is
/// supplied together with every snapshot (see
/// [`crate::TransitionBatchBuilder::state`]); the two cannot drift apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExecutionListing {
    status: ExecutionListingStatus,
    started_at: Option<DateTime<Utc>>,
    finished_at: Option<DateTime<Utc>>,
}

impl ExecutionListing {
    /// The listing of a freshly created execution.
    pub const CREATED: Self = Self {
        status: ExecutionListingStatus::Created,
        started_at: None,
        finished_at: None,
    };

    /// Project a snapshot's status and lifecycle timestamps.
    #[must_use]
    pub const fn new(
        status: ExecutionListingStatus,
        started_at: Option<DateTime<Utc>>,
        finished_at: Option<DateTime<Utc>>,
    ) -> Self {
        Self {
            status,
            started_at,
            finished_at,
        }
    }

    /// Execution status.
    #[must_use]
    pub const fn status(&self) -> ExecutionListingStatus {
        self.status
    }

    /// When the execution started running, if it has.
    #[must_use]
    pub const fn started_at(&self) -> Option<DateTime<Utc>> {
        self.started_at
    }

    /// When the execution reached a terminal status, if it has.
    #[must_use]
    pub const fn finished_at(&self) -> Option<DateTime<Utc>> {
        self.finished_at
    }
}

/// Parameters for inserting a new execution row inside a compose transaction.
///
/// The execution id is taken from `JobDispatchMsg::execution_id` and the
/// tenant scope from `JobDispatchMsg::scope` — single source of truth in the
/// compose method; this struct carries only the fields that differ.
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
    /// Creation timestamp (RFC 3339).
    pub created_at: String,
    /// Last-update timestamp (RFC 3339).
    pub updated_at: String,
}

/// Bounded number of executions returned by one history page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ExecutionHistoryPageSize(u8);

impl Default for ExecutionHistoryPageSize {
    fn default() -> Self {
        Self(20)
    }
}

impl ExecutionHistoryPageSize {
    /// Largest admitted page.
    pub const MAX: u8 = 100;

    /// Construct a non-zero bounded page size.
    ///
    /// # Errors
    /// Returns [`ExecutionHistoryPageSizeError`] outside `1..=100`.
    pub const fn new(value: u8) -> Result<Self, ExecutionHistoryPageSizeError> {
        if value == 0 || value > Self::MAX {
            return Err(ExecutionHistoryPageSizeError);
        }
        Ok(Self(value))
    }

    /// The admitted page size.
    #[must_use]
    pub const fn get(self) -> u8 {
        self.0
    }
}

/// Execution history page size lies outside `1..=100`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("execution history page size is outside 1..=100")]
pub struct ExecutionHistoryPageSizeError;

/// Stable keyset position in the history order (`created_at DESC, id DESC`).
///
/// Backends store `created_at` truncated to microseconds, so the cursor taken
/// from a returned [`ExecutionSummary`] matches the stored key exactly.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExecutionHistoryCursor {
    created_at: DateTime<Utc>,
    id: String,
}

impl ExecutionHistoryCursor {
    /// Position just after `(created_at, id)`.
    #[must_use]
    pub fn new(created_at: DateTime<Utc>, id: impl Into<String>) -> Self {
        Self {
            created_at,
            id: id.into(),
        }
    }

    /// Creation instant of the last execution already returned.
    #[must_use]
    pub const fn created_at(&self) -> DateTime<Utc> {
        self.created_at
    }

    /// Id of the last execution already returned.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }
}

/// Filters and position for [`crate::store::ExecutionStore::list_history`].
///
/// Results are ordered newest first (`created_at DESC, id DESC`). The creation
/// range is half-open: `created_after` is inclusive, `created_before`
/// exclusive. An empty status set means every status.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExecutionHistoryQuery {
    workflow_id: Option<String>,
    statuses: BTreeSet<ExecutionListingStatus>,
    created_after: Option<DateTime<Utc>>,
    created_before: Option<DateTime<Utc>>,
    page_size: ExecutionHistoryPageSize,
    after: Option<ExecutionHistoryCursor>,
}

impl ExecutionHistoryQuery {
    /// Every execution in scope, first page, default page size.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Only executions of this workflow.
    #[must_use]
    pub fn workflow(mut self, workflow_id: impl Into<String>) -> Self {
        self.workflow_id = Some(workflow_id.into());
        self
    }

    /// Only executions whose status is in `statuses` (empty = all).
    #[must_use]
    pub fn statuses(mut self, statuses: BTreeSet<ExecutionListingStatus>) -> Self {
        self.statuses = statuses;
        self
    }

    /// Only executions created at or after `instant`.
    #[must_use]
    pub const fn created_after(mut self, instant: DateTime<Utc>) -> Self {
        self.created_after = Some(instant);
        self
    }

    /// Only executions created strictly before `instant`.
    #[must_use]
    pub const fn created_before(mut self, instant: DateTime<Utc>) -> Self {
        self.created_before = Some(instant);
        self
    }

    /// Page size.
    #[must_use]
    pub const fn page_size(mut self, page_size: ExecutionHistoryPageSize) -> Self {
        self.page_size = page_size;
        self
    }

    /// Continue after a cursor from a previous page.
    #[must_use]
    pub fn after(mut self, cursor: ExecutionHistoryCursor) -> Self {
        self.after = Some(cursor);
        self
    }

    /// Workflow filter.
    #[must_use]
    pub fn workflow_id(&self) -> Option<&str> {
        self.workflow_id.as_deref()
    }

    /// Status filter (empty = all).
    #[must_use]
    pub const fn status_filter(&self) -> &BTreeSet<ExecutionListingStatus> {
        &self.statuses
    }

    /// Inclusive lower creation bound.
    #[must_use]
    pub const fn created_after_bound(&self) -> Option<DateTime<Utc>> {
        self.created_after
    }

    /// Exclusive upper creation bound.
    #[must_use]
    pub const fn created_before_bound(&self) -> Option<DateTime<Utc>> {
        self.created_before
    }

    /// Requested page size.
    #[must_use]
    pub const fn limit(&self) -> ExecutionHistoryPageSize {
        self.page_size
    }

    /// Keyset position, `None` for the first page.
    #[must_use]
    pub const fn cursor(&self) -> Option<&ExecutionHistoryCursor> {
        self.after.as_ref()
    }

    /// `true` when `summary` belongs in the result set.
    ///
    /// The reference predicate: SQL backends express the same conditions in
    /// their `WHERE` clause, the in-memory adapter calls this directly, and
    /// conformance holds every backend to it.
    #[must_use]
    pub fn admits(&self, summary: &ExecutionSummary) -> bool {
        self.workflow_id
            .as_deref()
            .is_none_or(|workflow| workflow == summary.workflow_id)
            && (self.statuses.is_empty() || self.statuses.contains(&summary.status))
            && self
                .created_after
                .is_none_or(|bound| summary.created_at >= bound)
            && self
                .created_before
                .is_none_or(|bound| summary.created_at < bound)
            && self.after.as_ref().is_none_or(|cursor| {
                (summary.created_at, summary.id.as_str())
                    < (cursor.created_at, cursor.id.as_str())
            })
    }
}

/// One execution in a history page: the listing projection plus identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionSummary {
    /// Execution id (opaque string form).
    pub id: String,
    /// Owning workflow id (opaque string form).
    pub workflow_id: String,
    /// Status of the latest snapshot.
    pub status: ExecutionListingStatus,
    /// Creation instant, truncated to microseconds.
    pub created_at: DateTime<Utc>,
    /// When the execution started running, if it has.
    pub started_at: Option<DateTime<Utc>>,
    /// When the execution reached a terminal status, if it has.
    pub finished_at: Option<DateTime<Utc>>,
    /// Last state change.
    pub updated_at: DateTime<Utc>,
}

impl ExecutionSummary {
    /// Cursor positioned just after this execution.
    #[must_use]
    pub fn cursor(&self) -> ExecutionHistoryCursor {
        ExecutionHistoryCursor::new(self.created_at, self.id.clone())
    }
}

/// One page of execution history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionHistoryPage {
    /// Executions, newest first.
    pub items: Vec<ExecutionSummary>,
    /// Cursor for the next page; `None` when this page is the last.
    pub next_cursor: Option<ExecutionHistoryCursor>,
}

impl ExecutionHistoryPage {
    /// Build a page from up to `page_size + 1` ordered rows. The extra row
    /// only proves that another page exists; it is not returned.
    #[must_use]
    pub fn from_overfetched(
        mut rows: Vec<ExecutionSummary>,
        page_size: ExecutionHistoryPageSize,
    ) -> Self {
        let limit = usize::from(page_size.get());
        let has_more = rows.len() > limit;
        rows.truncate(limit);
        let next_cursor = if has_more {
            rows.last().map(ExecutionSummary::cursor)
        } else {
            None
        };
        Self {
            items: rows,
            next_cursor,
        }
    }
}

/// Truncate an instant to the microsecond precision every backend stores.
#[must_use]
pub fn truncate_to_micros(instant: DateTime<Utc>) -> DateTime<Utc> {
    DateTime::from_timestamp_micros(instant.timestamp_micros()).unwrap_or(instant)
}
