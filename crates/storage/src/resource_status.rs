//! Backend-independent resource-status decisions.
//!
//! Every `ResourceStatusStore` adapter — the in-memory reference model,
//! SQLite, and PostgreSQL — routes heartbeat TTL clipping, the prune horizon,
//! and the persisted-value conversions through this module, so the three
//! backends cannot disagree about when a worker is dead or what a stored row
//! means. Row plumbing and the clock stay in each adapter.

use std::time::Duration;

use nebula_storage_port::StorageError;
#[cfg(any(feature = "sqlite", feature = "postgres"))]
use nebula_storage_port::dto::{
    LiveResourceStatus, ResourceStatusPhase, ResourceStatusSnapshot, StatusWorkerId,
};

/// Longest heartbeat TTL a store honours; a longer one is clipped so a
/// mistaken caller cannot keep a crashed worker's status live for years.
pub(crate) const MAX_HEARTBEAT_TTL: Duration = Duration::from_hours(24);

/// How long past its expiry a heartbeat, and every snapshot its worker
/// published, is kept before a later heartbeat prunes it. Expired rows are
/// already invisible to reads; this only bounds table growth.
pub(crate) const HEARTBEAT_RETENTION_MS: i64 = 60 * 60 * 1000;

/// The heartbeat TTL in store milliseconds, clipped to [`MAX_HEARTBEAT_TTL`].
pub(crate) fn heartbeat_ttl_ms(ttl: Duration) -> i64 {
    let clipped = ttl.min(MAX_HEARTBEAT_TTL).as_millis();
    i64::try_from(clipped).unwrap_or(i64::MAX)
}

/// A published row version as the signed integer both SQL dialects store.
pub(crate) fn row_version_to_stored(row_version: u64) -> Result<i64, StorageError> {
    i64::try_from(row_version).map_err(|_| {
        StorageError::Serialization("resource status row version exceeds i64::MAX".to_owned())
    })
}

/// Rebuilds one live snapshot from its persisted columns, rejecting any value
/// the migration constraints should have made impossible.
#[cfg(any(feature = "sqlite", feature = "postgres"))]
pub(crate) fn decode_live(
    resource_id: &str,
    worker_id: String,
    phase: &str,
    healthy: bool,
    accepting: bool,
    row_version: i64,
) -> Result<LiveResourceStatus, StorageError> {
    let worker_id = StatusWorkerId::new(worker_id)
        .map_err(|error| StorageError::Serialization(format!("resource status: {error}")))?;
    let phase = ResourceStatusPhase::parse(phase)
        .map_err(|error| StorageError::Serialization(format!("resource status: {error}")))?;
    let row_version = u64::try_from(row_version).map_err(|_| {
        StorageError::Serialization("resource status row version is negative".to_owned())
    })?;
    Ok(LiveResourceStatus {
        worker_id,
        snapshot: ResourceStatusSnapshot {
            resource_id: resource_id.to_owned(),
            phase,
            healthy,
            accepting,
            row_version,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heartbeat_ttl_is_clipped_and_never_overflows() {
        assert_eq!(heartbeat_ttl_ms(Duration::from_millis(50)), 50);
        assert_eq!(heartbeat_ttl_ms(Duration::ZERO), 0);
        assert_eq!(
            heartbeat_ttl_ms(Duration::MAX),
            i64::try_from(MAX_HEARTBEAT_TTL.as_millis()).expect("24h fits i64")
        );
    }

    #[test]
    fn row_version_rejects_values_outside_the_stored_range() {
        let max = u64::try_from(i64::MAX).expect("i64::MAX fits u64");
        assert_eq!(row_version_to_stored(max).ok(), Some(i64::MAX));
        assert!(row_version_to_stored(u64::MAX).is_err());
    }
}
