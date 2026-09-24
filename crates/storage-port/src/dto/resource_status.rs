//! Cross-process resource runtime status.
//!
//! A resource instance lives in the `Manager` of the worker that activated
//! it; the API that reports its status runs in another process. Workers
//! publish a per-row snapshot and renew a liveness heartbeat; readers only
//! trust snapshots whose worker heartbeat has not expired, so a crashed
//! worker's status disappears on its own instead of reporting a stale
//! `ready` forever. Snapshots carry lifecycle state only: never config or
//! credential material.

use std::fmt;

/// Stable identity of a publishing worker process.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StatusWorkerId(String);

/// Longest accepted [`StatusWorkerId`] in bytes.
pub const MAX_STATUS_WORKER_ID_BYTES: usize = 128;

impl StatusWorkerId {
    /// Validates a worker identity: 1..=128 bytes, printable ASCII.
    ///
    /// # Errors
    ///
    /// [`ResourceStatusValueError::WorkerId`] when empty, too long or not
    /// printable ASCII.
    pub fn new(value: impl Into<String>) -> Result<Self, ResourceStatusValueError> {
        let value = value.into();
        let valid = !value.is_empty()
            && value.len() <= MAX_STATUS_WORKER_ID_BYTES
            && value.bytes().all(|byte| byte.is_ascii_graphic());
        if valid {
            Ok(Self(value))
        } else {
            Err(ResourceStatusValueError::WorkerId)
        }
    }

    /// The identity as stored.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for StatusWorkerId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("StatusWorkerId")
            .field(&self.0)
            .finish()
    }
}

/// Closed lifecycle vocabulary persisted for a resource row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ResourceStatusPhase {
    /// Runtime is being constructed.
    Initializing,
    /// Healthy and serving.
    Ready,
    /// Reloading; may still accept.
    Reloading,
    /// Draining in-flight work.
    Draining,
    /// Shutting down.
    ShuttingDown,
    /// Failed.
    Failed,
    /// A phase the publishing worker's build does not name.
    Unknown,
}

impl ResourceStatusPhase {
    /// Every phase, in persisted-token order.
    pub const ALL: [Self; 7] = [
        Self::Initializing,
        Self::Ready,
        Self::Reloading,
        Self::Draining,
        Self::ShuttingDown,
        Self::Failed,
        Self::Unknown,
    ];

    /// The persisted token.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Initializing => "initializing",
            Self::Ready => "ready",
            Self::Reloading => "reloading",
            Self::Draining => "draining",
            Self::ShuttingDown => "shutting_down",
            Self::Failed => "failed",
            Self::Unknown => "unknown",
        }
    }

    /// Parses a persisted token.
    ///
    /// # Errors
    ///
    /// [`ResourceStatusValueError::Phase`] for an unrecognised token.
    pub fn parse(token: &str) -> Result<Self, ResourceStatusValueError> {
        Self::ALL
            .into_iter()
            .find(|phase| phase.as_str() == token)
            .ok_or(ResourceStatusValueError::Phase)
    }
}

/// One worker's view of one resource row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceStatusSnapshot {
    /// Stored resource row id (`res_…`).
    pub resource_id: String,
    /// Lifecycle phase.
    pub phase: ResourceStatusPhase,
    /// Serving healthily.
    pub healthy: bool,
    /// Accepting new acquires.
    pub accepting: bool,
    /// Stored row version the worker activated.
    pub row_version: u64,
}

/// A snapshot published by a worker whose heartbeat is still live.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveResourceStatus {
    /// The publishing worker.
    pub worker_id: StatusWorkerId,
    /// Its view of the row.
    pub snapshot: ResourceStatusSnapshot,
}

/// A resource-status value failed validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ResourceStatusValueError {
    /// Worker id empty, longer than 128 bytes, or not printable ASCII.
    #[error("resource status worker id must be 1..=128 printable ASCII bytes")]
    WorkerId,
    /// Phase token outside the closed vocabulary.
    #[error("resource status phase is not a known token")]
    Phase,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phase_tokens_round_trip_and_reject_unknown_tokens() {
        for phase in ResourceStatusPhase::ALL {
            assert_eq!(ResourceStatusPhase::parse(phase.as_str()), Ok(phase));
        }
        assert_eq!(
            ResourceStatusPhase::parse("Ready"),
            Err(ResourceStatusValueError::Phase)
        );
    }

    #[test]
    fn worker_id_is_bounded_printable_ascii() {
        assert!(StatusWorkerId::new("worker:0a1b").is_ok());
        assert!(StatusWorkerId::new("").is_err());
        assert!(StatusWorkerId::new("has space").is_err());
        assert!(StatusWorkerId::new("x".repeat(MAX_STATUS_WORKER_ID_BYTES + 1)).is_err());
    }
}
