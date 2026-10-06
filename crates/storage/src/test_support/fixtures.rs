//! Factory functions for the Plane-A auth row tests.

use chrono::Utc;

use crate::rows::UserRow;

/// Generate a pseudo-unique 16-byte ID for tests.
///
/// Uses nanosecond timestamp mixed with an atomic counter,
/// producing IDs that are unique across calls within a process.
pub fn random_id() -> Vec<u8> {
    use std::{
        sync::atomic::{AtomicU64, Ordering},
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    // A pre-epoch wall clock (broken environment) would make
    // `duration_since(UNIX_EPOCH)` fail; degrade to zero instead of panicking.
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_nanos();
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);

    let mut bytes = [0u8; 16];
    bytes[..8].copy_from_slice(&nanos.to_le_bytes()[..8]);
    bytes[8..16].copy_from_slice(&seq.to_le_bytes());
    bytes.to_vec()
}

/// Create a test [`UserRow`] with the given email and generated defaults.
pub fn test_user(email: &str) -> UserRow {
    UserRow {
        id: random_id(),
        email: email.to_lowercase(),
        email_verified_at: None,
        display_name: email.split('@').next().unwrap_or("test-user").to_string(),
        avatar_url: None,
        password_hash: None,
        created_at: Utc::now(),
        last_login_at: None,
        locked_until: None,
        failed_login_count: 0,
        mfa_enabled: false,
        mfa_secret_envelope: None,
        version: 0,
        deleted_at: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_ids_are_16_bytes_and_unique() {
        let a = random_id();
        assert_eq!(a.len(), 16);
        assert_ne!(a, random_id());
    }

    #[test]
    fn test_user_defaults() {
        let user = test_user("alice@example.com");
        assert_eq!(user.email, "alice@example.com");
        assert_eq!(user.display_name, "alice");
        assert_eq!(user.version, 0);
        assert!(!user.mfa_enabled);
        assert!(user.deleted_at.is_none());
    }
}
