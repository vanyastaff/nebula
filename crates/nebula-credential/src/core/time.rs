use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Convert SystemTime to Unix timestamp
pub fn to_unix_timestamp(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

/// Convert Unix timestamp to SystemTime
pub fn from_unix_timestamp(timestamp: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(timestamp)
}

/// Get current Unix timestamp
pub fn unix_now() -> u64 {
    to_unix_timestamp(SystemTime::now())
}
