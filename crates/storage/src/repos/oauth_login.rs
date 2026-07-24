//! Atomic persistence contract for Plane-A OAuth login completion.

use chrono::{DateTime, Utc};

use crate::rows::UserRow;

/// All storage inputs needed to converge one OAuth login into a local
/// user, a stable external-identity link, and exactly one authority
/// artifact: a browser session or a local MFA challenge.
///
/// The command intentionally contains no provider access token or
/// authorization code. Network exchange and identity verification must
/// finish before the storage transaction starts.
pub struct OAuthLoginFinalizeCommand {
    /// Low-cardinality provider key, for example `google` or `github`.
    pub provider: String,
    /// Opaque stable subject asserted by the provider.
    pub subject: String,
    /// Provider-attested email, normalized by the caller. `None` is
    /// sufficient only when `(provider, subject)` is already linked.
    /// An email collision never authorizes linking to an existing user.
    pub verified_email: Option<String>,
    /// Candidate local user used only when neither an identity link nor
    /// an active user with `verified_email` exists.
    pub candidate_user: OAuthLoginUserDraft,
    /// Browser session to persist when the canonical user does not require MFA.
    pub session: OAuthLoginSessionDraft,
    /// One-time MFA challenge material to persist instead of the session
    /// when the canonical linked user has Nebula MFA enabled.
    pub mfa_challenge: OAuthLoginMfaChallengeDraft,
}

impl std::fmt::Debug for OAuthLoginFinalizeCommand {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OAuthLoginFinalizeCommand")
            .field("provider", &self.provider)
            .field("subject", &"[redacted]")
            .field(
                "verified_email",
                &self.verified_email.as_ref().map(|_| "[redacted]"),
            )
            .field("candidate_user", &"[redacted]")
            .field("session", &"[redacted]")
            .field("mfa_challenge", &"[redacted]")
            .finish()
    }
}

/// Candidate OAuth-only user fields whose values are chosen outside
/// storage. Security-sensitive defaults such as `password_hash = NULL`
/// and `email_verified_at` are owned by the finalizer.
pub struct OAuthLoginUserDraft {
    /// Candidate `usr_` identifier in raw storage bytes.
    pub id: Vec<u8>,
    /// Display name for a newly-created account.
    pub display_name: String,
    /// Optional provider avatar for a newly-created account.
    pub avatar_url: Option<String>,
    /// Creation and email-verification timestamp.
    pub created_at: DateTime<Utc>,
}

impl std::fmt::Debug for OAuthLoginUserDraft {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OAuthLoginUserDraft")
            .field("id", &"[redacted]")
            .field("display_name", &"[redacted]")
            .field(
                "avatar_url",
                &self.avatar_url.as_ref().map(|_| "[redacted]"),
            )
            .field("created_at", &self.created_at)
            .finish()
    }
}

/// New browser-session fields independent of the eventual canonical
/// user. The finalizer supplies `user_id` and always stores the session
/// as initially unrevoked.
pub struct OAuthLoginSessionDraft {
    /// Opaque bearer returned exactly once after the transaction commits.
    /// Persistence derives and stores only its domain-separated digest.
    pub token: Vec<u8>,
    /// Session creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Initial activity timestamp.
    pub last_active_at: DateTime<Utc>,
    /// Absolute session expiration timestamp.
    pub expires_at: DateTime<Utc>,
    /// Optional client IP address.
    pub ip_address: Option<String>,
    /// Optional client user-agent string.
    pub user_agent: Option<String>,
}

impl std::fmt::Debug for OAuthLoginSessionDraft {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OAuthLoginSessionDraft")
            .field("token", &"[redacted]")
            .field("created_at", &self.created_at)
            .field("last_active_at", &self.last_active_at)
            .field("expires_at", &self.expires_at)
            .field(
                "ip_address",
                &self.ip_address.as_ref().map(|_| "[redacted]"),
            )
            .field(
                "user_agent",
                &self.user_agent.as_ref().map(|_| "[redacted]"),
            )
            .finish()
    }
}

/// Hashed one-time challenge material independent of the canonical user.
///
/// The API retains the random plaintext and passes only its SHA-256 digest
/// into storage. The finalizer supplies `user_id`, fixes the token kind to
/// `mfa_challenge`, and stores this draft only when local MFA gates the login.
pub struct OAuthLoginMfaChallengeDraft {
    /// SHA-256 digest of the opaque plaintext challenge.
    pub token_hash: [u8; 32],
    /// Challenge creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Absolute challenge expiration timestamp.
    pub expires_at: DateTime<Utc>,
}

impl std::fmt::Debug for OAuthLoginMfaChallengeDraft {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OAuthLoginMfaChallengeDraft")
            .field("token_hash", &"[redacted]")
            .field("created_at", &self.created_at)
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

/// Successfully persisted canonical user plus the one-time session bearer.
pub struct OAuthLoginFinalized {
    /// Canonical active local user selected by the stable identity link
    /// or created for an otherwise-unclaimed verified email.
    pub user: UserRow,
    /// Plaintext bearer returned only after its digest is committed. The
    /// finalizer never reads this value back from storage.
    pub session_token: Vec<u8>,
    /// Absolute expiration of the committed session.
    pub session_expires_at: DateTime<Utc>,
}

impl std::fmt::Debug for OAuthLoginFinalized {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OAuthLoginFinalized")
            .field("user", &"[redacted]")
            .field("session_token", &"[redacted]")
            .field("session_expires_at", &self.session_expires_at)
            .finish()
    }
}

/// Semantic result of atomically finalizing an OAuth login.
#[must_use]
#[non_exhaustive]
pub enum OAuthLoginFinalizeOutcome {
    /// User/link/session state converged and committed.
    Finalized(Box<OAuthLoginFinalized>),
    /// The canonical linked user requires Nebula MFA. The challenge hash was
    /// committed for that user in the same transaction and no session was
    /// created.
    MfaRequired,
    /// A first link cannot be established without a provider-attested
    /// email. An existing subject link would have succeeded without it.
    VerifiedEmailRequired,
    /// The verified email already belongs to a local account. Explicit
    /// authenticated account linking is required; this finalizer performs
    /// no writes and never auto-links by email.
    AccountLinkRequired,
    /// The stable external identity points at a local user that is not
    /// active. The finalizer fails closed instead of rebinding by email.
    LinkedUserUnavailable,
}

impl std::fmt::Debug for OAuthLoginFinalizeOutcome {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Finalized(_) => formatter
                .debug_tuple("Finalized")
                .field(&"[redacted]")
                .finish(),
            Self::MfaRequired => formatter.write_str("MfaRequired"),
            Self::VerifiedEmailRequired => formatter.write_str("VerifiedEmailRequired"),
            Self::AccountLinkRequired => formatter.write_str("AccountLinkRequired"),
            Self::LinkedUserUnavailable => formatter.write_str("LinkedUserUnavailable"),
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::{
        OAuthLoginFinalizeCommand, OAuthLoginFinalizeOutcome, OAuthLoginFinalized,
        OAuthLoginMfaChallengeDraft, OAuthLoginSessionDraft, OAuthLoginUserDraft,
    };
    use crate::rows::UserRow;

    #[test]
    fn command_debug_redacts_identity_email_and_session_material() {
        let now = Utc::now();
        let candidate_user = OAuthLoginUserDraft {
            id: b"USER_CANARY-b2bd".to_vec(),
            display_name: "DISPLAY_CANARY-f247".to_owned(),
            avatar_url: Some("https://example.test/AVATAR_CANARY-3552".to_owned()),
            created_at: now,
        };
        let session = OAuthLoginSessionDraft {
            token: b"SESSION_CANARY-83c0".to_vec(),
            created_at: now,
            last_active_at: now,
            expires_at: now,
            ip_address: Some("192.0.2.42".to_owned()),
            user_agent: Some("AGENT_CANARY-cdb5".to_owned()),
        };
        let mfa_challenge = OAuthLoginMfaChallengeDraft {
            token_hash: [0xA5; 32],
            created_at: now,
            expires_at: now,
        };
        let component_debug = format!("{candidate_user:?} {session:?} {mfa_challenge:?}");
        let command = OAuthLoginFinalizeCommand {
            provider: "google".to_owned(),
            subject: "SUBJECT_CANARY-5e77".to_owned(),
            verified_email: Some("EMAIL_CANARY-1399@example.test".to_owned()),
            candidate_user,
            session,
            mfa_challenge,
        };

        let debug = format!("{command:?}");
        assert!(debug.contains("google"));
        for canary in [
            "SUBJECT_CANARY-5e77",
            "EMAIL_CANARY-1399",
            "USER_CANARY-b2bd",
            "DISPLAY_CANARY-f247",
            "AVATAR_CANARY-3552",
            "SESSION_CANARY-83c0",
            "192.0.2.42",
            "AGENT_CANARY-cdb5",
            "a5a5a5a5",
            "165, 165, 165",
        ] {
            assert!(!debug.contains(canary), "Debug leaked {canary}");
            assert!(
                !component_debug.contains(canary),
                "component Debug leaked {canary}"
            );
        }

        let user = UserRow {
            id: b"FINAL_USER_CANARY".to_vec(),
            email: "FINAL_EMAIL_CANARY@example.test".to_owned(),
            email_verified_at: Some(now),
            display_name: "FINAL_DISPLAY_CANARY".to_owned(),
            avatar_url: None,
            password_hash: None,
            created_at: now,
            last_login_at: None,
            locked_until: None,
            failed_login_count: 0,
            mfa_enabled: false,
            mfa_secret_envelope: None,
            version: 0,
            deleted_at: None,
        };
        let finalized = OAuthLoginFinalized {
            user,
            session_token: b"FINAL_SESSION_CANARY".to_vec(),
            session_expires_at: now,
        };
        let finalized_debug = format!("{finalized:?}");
        assert!(!finalized_debug.contains("FINAL_USER_CANARY"));
        assert!(!finalized_debug.contains("FINAL_EMAIL_CANARY"));
        assert!(!finalized_debug.contains("FINAL_SESSION_CANARY"));
        let outcome_debug = format!(
            "{:?}",
            OAuthLoginFinalizeOutcome::Finalized(Box::new(finalized))
        );
        assert!(!outcome_debug.contains("FINAL_USER_CANARY"));
        assert!(!outcome_debug.contains("FINAL_EMAIL_CANARY"));
        assert!(!outcome_debug.contains("FINAL_SESSION_CANARY"));
    }
}
