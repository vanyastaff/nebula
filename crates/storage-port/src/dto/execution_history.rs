//! Execution history: the query, its keyset cursor, and the page a backend
//! returns for [`crate::store::ExecutionStore::list_history`].
//!
//! History is ordered newest first by `(created_at, id)` — `created_at` at
//! microsecond precision ([`MicrosInstant`]), `id` compared by bytes. A page is
//! fetched one row past its size ([`ExecutionHistoryQuery::fetch_limit`]) and
//! assembled by [`ExecutionHistoryPage::from_overfetched`], so every backend
//! derives `next_cursor` the same way.

use chrono::{DateTime, Utc};

use super::execution_listing::{ExecutionListingStatus, ExecutionStatusSet, MicrosInstant};

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
    pub const MAX: Self = Self(100);

    /// Construct a non-zero bounded page size.
    ///
    /// # Errors
    /// Returns [`ExecutionHistoryPageSizeError`] outside `1..=100`.
    pub const fn new(value: u8) -> Result<Self, ExecutionHistoryPageSizeError> {
        if value == 0 || value > Self::MAX.0 {
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

/// Stable keyset position: just after `(created_at, id)` in history order.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExecutionHistoryCursor {
    created_at: MicrosInstant,
    id: String,
}

impl ExecutionHistoryCursor {
    /// Position just after the execution `id` created at `created_at`.
    #[must_use]
    pub fn new(created_at: MicrosInstant, id: impl Into<String>) -> Self {
        Self {
            created_at,
            id: id.into(),
        }
    }

    /// Creation instant of the last execution already returned.
    #[must_use]
    pub const fn created_at(&self) -> MicrosInstant {
        self.created_at
    }

    /// Id of the last execution already returned.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The keyset key this cursor stands on.
    fn key(&self) -> (MicrosInstant, &str) {
        (self.created_at, &self.id)
    }
}

/// Filters and position for [`crate::store::ExecutionStore::list_history`].
///
/// The creation range is half-open: `created_after` is inclusive,
/// `created_before` exclusive. [`Self::admits`] is the exact predicate every
/// backend implements.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExecutionHistoryQuery {
    workflow_id: Option<String>,
    statuses: ExecutionStatusSet,
    created_after: Option<MicrosInstant>,
    created_before: Option<MicrosInstant>,
    page_size: ExecutionHistoryPageSize,
    cursor: Option<ExecutionHistoryCursor>,
}

impl ExecutionHistoryQuery {
    /// Every execution in scope, first page, default page size.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Only executions of this workflow.
    #[must_use]
    pub fn with_workflow(mut self, workflow_id: impl Into<String>) -> Self {
        self.workflow_id = Some(workflow_id.into());
        self
    }

    /// Only executions whose status is in `statuses`.
    #[must_use]
    pub const fn with_statuses(mut self, statuses: ExecutionStatusSet) -> Self {
        self.statuses = statuses;
        self
    }

    /// Only executions created at or after `instant`.
    ///
    /// Stored keys are whole microseconds, so the bound is rounded up: a
    /// microsecond key is `>= ceil(bound)` exactly when it is `>= bound`.
    #[must_use]
    pub fn with_created_after(mut self, instant: DateTime<Utc>) -> Self {
        self.created_after = Some(MicrosInstant::ceil(instant));
        self
    }

    /// Only executions created strictly before `instant`.
    ///
    /// Rounded up for the same reason: a microsecond key is `< ceil(bound)`
    /// exactly when it is `< bound`.
    #[must_use]
    pub fn with_created_before(mut self, instant: DateTime<Utc>) -> Self {
        self.created_before = Some(MicrosInstant::ceil(instant));
        self
    }

    /// Page size.
    #[must_use]
    pub const fn with_page_size(mut self, page_size: ExecutionHistoryPageSize) -> Self {
        self.page_size = page_size;
        self
    }

    /// Continue after a cursor from a previous page.
    #[must_use]
    pub fn with_cursor(mut self, cursor: ExecutionHistoryCursor) -> Self {
        self.cursor = Some(cursor);
        self
    }

    /// Workflow filter.
    #[must_use]
    pub fn workflow_id(&self) -> Option<&str> {
        self.workflow_id.as_deref()
    }

    /// Admitted statuses ([`ExecutionStatusSet::ALL`] when unfiltered).
    #[must_use]
    pub const fn statuses(&self) -> ExecutionStatusSet {
        self.statuses
    }

    /// Inclusive lower creation bound.
    #[must_use]
    pub const fn created_after(&self) -> Option<MicrosInstant> {
        self.created_after
    }

    /// Exclusive upper creation bound.
    #[must_use]
    pub const fn created_before(&self) -> Option<MicrosInstant> {
        self.created_before
    }

    /// Requested page size.
    #[must_use]
    pub const fn page_size(&self) -> ExecutionHistoryPageSize {
        self.page_size
    }

    /// Rows a backend fetches for one page: the page plus one row that only
    /// proves another page exists.
    #[must_use]
    pub fn fetch_limit(&self) -> u16 {
        u16::from(self.page_size.get()) + 1
    }

    /// Keyset position, `None` for the first page.
    #[must_use]
    pub const fn cursor(&self) -> Option<&ExecutionHistoryCursor> {
        self.cursor.as_ref()
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
            && self.statuses.contains(summary.status)
            && self
                .created_after
                .is_none_or(|bound| summary.created_at >= bound)
            && self
                .created_before
                .is_none_or(|bound| summary.created_at < bound)
            && self
                .cursor
                .as_ref()
                .is_none_or(|cursor| summary.key() < cursor.key())
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
    /// Creation instant.
    pub created_at: MicrosInstant,
    /// When the execution started running, if it has.
    pub started_at: Option<MicrosInstant>,
    /// When the execution reached a terminal status, if it has.
    pub finished_at: Option<MicrosInstant>,
    /// Last state change.
    pub updated_at: MicrosInstant,
}

impl ExecutionSummary {
    /// Cursor positioned just after this execution.
    #[must_use]
    pub fn cursor(&self) -> ExecutionHistoryCursor {
        ExecutionHistoryCursor::new(self.created_at, self.id.clone())
    }

    /// This execution's position in history order (newest first means a
    /// larger key comes first).
    #[must_use]
    pub fn key(&self) -> (MicrosInstant, &str) {
        (self.created_at, &self.id)
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
    /// A page with no rows and no successor.
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            items: Vec::new(),
            next_cursor: None,
        }
    }

    /// Build a page from up to [`ExecutionHistoryQuery::fetch_limit`] rows in
    /// history order. The extra row only proves another page exists; it is
    /// not returned.
    #[must_use]
    pub fn from_overfetched(
        mut rows: Vec<ExecutionSummary>,
        query: &ExecutionHistoryQuery,
    ) -> Self {
        let size = usize::from(query.page_size().get());
        let has_more = rows.len() > size;
        rows.truncate(size);
        let next_cursor = rows
            .last()
            .filter(|_| has_more)
            .map(ExecutionSummary::cursor);
        Self {
            items: rows,
            next_cursor,
        }
    }
}
