//! The queryable listing projection of an execution snapshot.
//!
//! Stored in dedicated columns next to the opaque `state` blob, so history can
//! be filtered and ordered without the port ever parsing the state. Three
//! value types carry its invariants so no backend has to re-check them:
//!
//! - [`ExecutionListingStatus`] — the closed status set and its stored names;
//! - [`ExecutionStatusSet`] — a `Copy` set of statuses (a status filter);
//! - [`MicrosInstant`] — an instant at the microsecond precision every backend
//!   stores, so all three compare, order and round-trip identically.

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Lifecycle status of an execution as the listing projection stores it.
///
/// A port-local mirror of `nebula_execution::ExecutionStatus` (the port
/// depends only on `nebula-core`): the engine maps one onto the other with an
/// exhaustive `match`, so a new execution status cannot reach storage
/// unmapped. The stored and wire name ([`Self::as_str`], `Display`,
/// `FromStr`, serde) is the snake_case name, identical to the execution
/// crate's serde form.
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

    /// `true` once the execution can no longer change status.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Cancelled | Self::TimedOut
        )
    }

    /// This status's bit in an [`ExecutionStatusSet`].
    const fn bit(self) -> u8 {
        match self {
            Self::Created => 1 << 0,
            Self::Running => 1 << 1,
            Self::Paused => 1 << 2,
            Self::Cancelling => 1 << 3,
            Self::Completed => 1 << 4,
            Self::Failed => 1 << 5,
            Self::Cancelled => 1 << 6,
            Self::TimedOut => 1 << 7,
        }
    }
}

impl fmt::Display for ExecutionListingStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A name outside the closed [`ExecutionListingStatus`] set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("unknown execution status")]
pub struct UnknownExecutionStatus;

impl FromStr for ExecutionListingStatus {
    type Err = UnknownExecutionStatus;

    fn from_str(name: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|status| status.as_str() == name)
            .ok_or(UnknownExecutionStatus)
    }
}

/// A set of [`ExecutionListingStatus`] values — one bit per status, `Copy`.
///
/// History filters by membership, so "no filter" is [`Self::ALL`] rather than
/// an empty set with a special meaning.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct ExecutionStatusSet(u8);

impl ExecutionStatusSet {
    /// No status.
    pub const EMPTY: Self = Self(0);
    /// Every status.
    pub const ALL: Self = {
        let mut set = Self::EMPTY;
        let mut index = 0;
        while index < ExecutionListingStatus::ALL.len() {
            set = set.with(ExecutionListingStatus::ALL[index]);
            index += 1;
        }
        set
    };
    /// The non-terminal statuses: what "running" means for a listing.
    pub const ACTIVE: Self = Self::EMPTY
        .with(ExecutionListingStatus::Created)
        .with(ExecutionListingStatus::Running)
        .with(ExecutionListingStatus::Paused)
        .with(ExecutionListingStatus::Cancelling);

    /// This set plus `status`.
    #[must_use]
    pub const fn with(self, status: ExecutionListingStatus) -> Self {
        Self(self.0 | status.bit())
    }

    /// `true` when `status` is a member.
    #[must_use]
    pub const fn contains(self, status: ExecutionListingStatus) -> bool {
        self.0 & status.bit() != 0
    }

    /// `true` when no status is a member.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// `true` when every status is a member — a filter that admits all rows.
    #[must_use]
    pub const fn is_all(self) -> bool {
        self.0 == Self::ALL.0
    }

    /// Members in lifecycle order.
    pub fn iter(self) -> impl Iterator<Item = ExecutionListingStatus> {
        ExecutionListingStatus::ALL
            .into_iter()
            .filter(move |status| self.contains(*status))
    }
}

impl Default for ExecutionStatusSet {
    /// Every status: the unfiltered history.
    fn default() -> Self {
        Self::ALL
    }
}

impl FromIterator<ExecutionListingStatus> for ExecutionStatusSet {
    fn from_iter<I: IntoIterator<Item = ExecutionListingStatus>>(statuses: I) -> Self {
        statuses.into_iter().fold(Self::EMPTY, Self::with)
    }
}

impl<const N: usize> From<[ExecutionListingStatus; N]> for ExecutionStatusSet {
    fn from(statuses: [ExecutionListingStatus; N]) -> Self {
        statuses.into_iter().collect()
    }
}

impl fmt::Debug for ExecutionStatusSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_set().entries(self.iter()).finish()
    }
}

/// An instant at microsecond precision — the precision every execution
/// backend stores (PostgreSQL `timestamptz`, the integer `created_at_us` sort
/// key, the in-memory reference adapter).
///
/// Carrying listing and history instants in this type makes every backend
/// compare, order and round-trip them identically: a nanosecond reading can
/// only enter through [`Self::floor`] or [`Self::ceil`], which name the
/// rounding the caller means.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MicrosInstant(DateTime<Utc>);

impl MicrosInstant {
    /// The current instant, truncated.
    #[must_use]
    pub fn now() -> Self {
        Self::floor(Utc::now())
    }

    /// Round `instant` down to whole microseconds.
    #[must_use]
    pub fn floor(instant: DateTime<Utc>) -> Self {
        // Every `DateTime<Utc>` has an in-range microsecond count, so the
        // conversion back cannot fail; `instant` is the unreachable fallback.
        Self(DateTime::from_timestamp_micros(instant.timestamp_micros()).unwrap_or(instant))
    }

    /// Round `instant` up to whole microseconds (identity when it has none).
    #[must_use]
    pub fn ceil(instant: DateTime<Utc>) -> Self {
        let floor = Self::floor(instant);
        if floor.0 == instant {
            return floor;
        }
        floor
            .0
            .checked_add_signed(chrono::Duration::microseconds(1))
            .map_or(floor, Self)
    }

    /// The instant `micros` microseconds after the Unix epoch, if in range.
    #[must_use]
    pub fn from_micros(micros: i64) -> Option<Self> {
        DateTime::from_timestamp_micros(micros).map(Self)
    }

    /// Microseconds since the Unix epoch — the stored sort key.
    #[must_use]
    pub fn as_micros(self) -> i64 {
        self.0.timestamp_micros()
    }

    /// The instant as a `DateTime`.
    #[must_use]
    pub const fn to_datetime(self) -> DateTime<Utc> {
        self.0
    }
}

/// The queryable projection of an execution state snapshot.
///
/// Supplied together with every snapshot (see
/// [`crate::TransitionBatchBuilder::state`]), so the two cannot drift apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExecutionListing {
    status: ExecutionListingStatus,
    started_at: Option<MicrosInstant>,
    finished_at: Option<MicrosInstant>,
}

impl ExecutionListing {
    /// The listing of a freshly created execution.
    pub const CREATED: Self = Self {
        status: ExecutionListingStatus::Created,
        started_at: None,
        finished_at: None,
    };

    /// Project a snapshot's status and lifecycle timestamps.
    ///
    /// Timestamps are truncated to the stored precision, and `finished_at` is
    /// kept only for a terminal status: a snapshot that can still change never
    /// claims to be finished, whatever the caller passed.
    #[must_use]
    pub fn new(
        status: ExecutionListingStatus,
        started_at: Option<DateTime<Utc>>,
        finished_at: Option<DateTime<Utc>>,
    ) -> Self {
        Self {
            status,
            started_at: started_at.map(MicrosInstant::floor),
            finished_at: finished_at
                .filter(|_| status.is_terminal())
                .map(MicrosInstant::floor),
        }
    }

    /// Execution status.
    #[must_use]
    pub const fn status(&self) -> ExecutionListingStatus {
        self.status
    }

    /// When the execution started running, if it has.
    #[must_use]
    pub const fn started_at(&self) -> Option<MicrosInstant> {
        self.started_at
    }

    /// When the execution reached a terminal status, if it has.
    #[must_use]
    pub const fn finished_at(&self) -> Option<MicrosInstant> {
        self.finished_at
    }
}
