//! Time utilities for credentials

use std::time::{SystemTime, UNIX_EPOCH};

/// Get current Unix timestamp in seconds
#[must_use]
pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("System time before Unix epoch")
        .as_secs()
}

/// Convert `SystemTime` to Unix timestamp
#[must_use]
pub fn to_unix_timestamp(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .expect("System time before Unix epoch")
        .as_secs()
}

/// Convert Unix timestamp to `SystemTime`
#[must_use]
pub fn from_unix_timestamp(timestamp: u64) -> SystemTime {
    UNIX_EPOCH + std::time::Duration::from_secs(timestamp)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unix_now() {
        let now = unix_now();
        assert!(now > 1_600_000_000); // After Sep 2020
    }

    #[test]
    fn test_round_trip() {
        let original = SystemTime::now();
        let timestamp = to_unix_timestamp(original);
        let restored = from_unix_timestamp(timestamp);

        // Should be within 1 second (due to precision loss)
        let diff = restored
            .duration_since(original)
            .unwrap_or_else(|e| e.duration())
            .as_secs();
        assert!(diff <= 1);
    }
}
