//! Factory functions for the Plane-A auth row tests.

use chrono::Utc;

use crate::auth::UserRow;

/// Generate a 16-byte ID for fixtures shared by independent test processes.
pub fn random_id() -> Vec<u8> {
    uuid::Uuid::new_v4().as_bytes().to_vec()
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
