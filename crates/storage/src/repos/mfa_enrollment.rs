//! Pending Plane-A MFA enrollment persistence.
//!
//! A candidate is deliberately separate from `users.mfa_secret_envelope`: starting a
//! replacement enrollment must never weaken or overwrite an already active
//! factor. The storage representation calls the candidate bytes an envelope
//! so the identity cipher can replace today's encoding without changing the
//! repository contract.

use std::future::Future;

use chrono::{DateTime, Utc};

use crate::StorageError;

/// One expiring MFA candidate owned by a user.
///
/// The value is move-only and its [`Debug`](std::fmt::Debug) implementation
/// redacts every authority-bearing field.
pub struct MfaEnrollmentCandidate {
    enrollment_id: [u8; 32],
    user_id: Vec<u8>,
    secret_envelope: Vec<u8>,
    created_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
}

impl MfaEnrollmentCandidate {
    /// Build a validated pending enrollment candidate.
    pub fn new(
        enrollment_id: [u8; 32],
        user_id: Vec<u8>,
        secret_envelope: Vec<u8>,
        created_at: DateTime<Utc>,
        expires_at: DateTime<Utc>,
    ) -> Result<Self, StorageError> {
        if enrollment_id == [0; 32] {
            return Err(StorageError::Configuration(
                "MFA enrollment id must be non-zero".to_owned(),
            ));
        }
        if user_id.is_empty() {
            return Err(StorageError::Configuration(
                "MFA enrollment user id must not be empty".to_owned(),
            ));
        }
        if secret_envelope.is_empty() {
            return Err(StorageError::Configuration(
                "MFA enrollment secret envelope must not be empty".to_owned(),
            ));
        }
        if created_at >= expires_at {
            return Err(StorageError::Configuration(
                "MFA enrollment expiry must follow creation".to_owned(),
            ));
        }
        Ok(Self {
            enrollment_id,
            user_id,
            secret_envelope,
            created_at,
            expires_at,
        })
    }

    /// Opaque identifier used to bind confirmation to the candidate that was
    /// actually verified.
    #[must_use]
    pub const fn enrollment_id(&self) -> &[u8; 32] {
        &self.enrollment_id
    }

    /// Raw storage identifier of the owning user.
    #[must_use]
    pub fn user_id(&self) -> &[u8] {
        &self.user_id
    }

    /// Opaque bytes consumed by the identity-layer cipher/codec.
    #[must_use]
    pub fn secret_envelope(&self) -> &[u8] {
        &self.secret_envelope
    }

    /// Candidate creation time.
    #[must_use]
    pub const fn created_at(&self) -> DateTime<Utc> {
        self.created_at
    }

    /// Candidate expiry time.
    #[must_use]
    pub const fn expires_at(&self) -> DateTime<Utc> {
        self.expires_at
    }
}

impl std::fmt::Debug for MfaEnrollmentCandidate {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MfaEnrollmentCandidate")
            .field("enrollment_id", &"[redacted]")
            .field("user_id", &"[redacted]")
            .field("secret_envelope", &"[redacted]")
            .field("created_at", &self.created_at)
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

/// Outcome of attempting to atomically install one verified candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use = "MFA enrollment installation outcome must be handled"]
#[non_exhaustive]
pub enum MfaEnrollmentInstallOutcome {
    /// The exact live candidate was consumed and became the active factor.
    Installed,
    /// The candidate was absent, expired, replaced, or already consumed.
    CandidateUnavailable,
}

/// Persistence boundary for pending MFA enrollment.
pub trait MfaEnrollmentRepo: Send + Sync {
    /// Replace only the user's pending candidate; active MFA state is never
    /// modified by this operation.
    fn replace_candidate(
        &self,
        candidate: &MfaEnrollmentCandidate,
    ) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Load the user's current live candidate without consuming it.
    fn get_live_candidate(
        &self,
        user_id: &[u8],
    ) -> impl Future<Output = Result<Option<MfaEnrollmentCandidate>, StorageError>> + Send;

    /// Atomically consume the exact live candidate and install its secret
    /// envelope as the user's active factor.
    fn install_candidate(
        &self,
        user_id: &[u8],
        enrollment_id: &[u8; 32],
    ) -> impl Future<Output = Result<MfaEnrollmentInstallOutcome, StorageError>> + Send;
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, Utc};

    use super::MfaEnrollmentCandidate;

    static_assertions::assert_not_impl_any!(MfaEnrollmentCandidate: Clone);

    #[test]
    fn debug_redacts_candidate_authority() {
        const CANARY: &str = "MFA_ENROLLMENT_CANDIDATE_CANARY-8b91";
        let now = Utc::now();
        let candidate = MfaEnrollmentCandidate::new(
            [7; 32],
            CANARY.as_bytes().to_vec(),
            CANARY.as_bytes().to_vec(),
            now,
            now + Duration::minutes(10),
        )
        .expect("valid candidate");

        let debug = format!("{candidate:?}");
        assert!(!debug.contains(CANARY));
        assert!(debug.contains("[redacted]"));
    }
}
