//! Shared helpers for `nebula-storage` integration tests.
//!
//! This module is not a test binary — it is included via `mod common;` in each
//! integration test file that needs it. Cargo compiles it as part of those
//! binaries only; it is never compiled standalone.

use nebula_credential::StoredCredential;

/// Build a minimal [`StoredCredential`] for use in integration tests.
pub fn make_credential(id: &str, data: &[u8]) -> StoredCredential {
    StoredCredential {
        id: id.into(),
        name: None,
        credential_key: "test_credential".into(),
        data: data.to_vec(),
        state_kind: "test".into(),
        state_version: 1,
        version: 0,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        expires_at: None,
        reauth_required: false,
        metadata: Default::default(),
    }
}
