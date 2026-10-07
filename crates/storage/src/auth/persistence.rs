//! Coherent account repositories for one deployment database and key authority.

use std::sync::Arc;

use super::{
    AccountLifecycle, MfaEnrollmentRepo, OAuthLoginFinalizer, OAuthStateRepo, PatRepo, SessionRepo,
    UserRepo, VerificationTokenRepo, identity_secret::IdentitySecretCodec,
};

/// Account persistence assembled from one deployment pool and identity codec.
///
/// Fields cannot be independently replaced with repositories from another
/// database or key authority. Read-only accessors expose the existing operation
/// contracts without another layer of async forwarding.
///
/// Construction is not a readiness check: the application must admit the schema
/// and complete identity-secret convergence before exposing authentication.
pub struct AuthPersistence {
    users: Box<dyn UserRepo>,
    sessions: Box<dyn SessionRepo>,
    pats: Box<dyn PatRepo>,
    verification_tokens: Box<dyn VerificationTokenRepo>,
    mfa_enrollments: Box<dyn MfaEnrollmentRepo>,
    oauth_states: Box<dyn OAuthStateRepo>,
    accounts: Box<dyn AccountLifecycle>,
    oauth_login: Box<dyn OAuthLoginFinalizer>,
    identity_secrets: Arc<IdentitySecretCodec>,
}

impl AuthPersistence {
    /// Assemble SQLite account persistence on the supplied deployment pool.
    /// The application must first admit its schema and identity secrets through
    /// [`super::sqlite::admit_identity_secrets`]. Construction performs no I/O.
    #[cfg(feature = "sqlite")]
    #[must_use]
    pub fn sqlite(
        deployment: &crate::sqlite::DeploymentPool,
        identity_secrets: Arc<IdentitySecretCodec>,
    ) -> Self {
        use super::sqlite::{
            SqliteAccountLifecycle, SqliteMfaEnrollmentRepo, SqliteOAuthLoginFinalizer,
            SqliteOAuthStateRepo, SqlitePatRepo, SqliteSessionRepo, SqliteUserRepo,
            SqliteVerificationTokenRepo,
        };
        let pool = deployment.pool();
        Self {
            users: Box::new(SqliteUserRepo::new(pool.clone())),
            sessions: Box::new(SqliteSessionRepo::new(pool.clone())),
            pats: Box::new(SqlitePatRepo::new(pool.clone())),
            verification_tokens: Box::new(SqliteVerificationTokenRepo::new(pool.clone())),
            mfa_enrollments: Box::new(SqliteMfaEnrollmentRepo::new(
                pool.clone(),
                Arc::clone(&identity_secrets),
            )),
            oauth_states: Box::new(SqliteOAuthStateRepo::new(deployment)),
            accounts: Box::new(SqliteAccountLifecycle::new(pool.clone())),
            oauth_login: Box::new(SqliteOAuthLoginFinalizer::new(pool.clone())),
            identity_secrets,
        }
    }

    /// Assemble PostgreSQL account persistence on the supplied deployment pool.
    /// No connection, schema admission, key migration or environment lookup is
    /// performed here; these remain explicit deployment startup stages.
    #[cfg(feature = "postgres")]
    #[must_use]
    pub fn postgres(pool: sqlx::PgPool, identity_secrets: Arc<IdentitySecretCodec>) -> Self {
        use super::postgres::{
            PgAccountLifecycle, PgMfaEnrollmentRepo, PgOAuthLoginFinalizer, PgOAuthStateRepo,
            PgPatRepo, PgSessionRepo, PgUserRepo, PgVerificationTokenRepo,
        };

        Self {
            users: Box::new(PgUserRepo::new(pool.clone())),
            sessions: Box::new(PgSessionRepo::new(pool.clone())),
            pats: Box::new(PgPatRepo::new(pool.clone())),
            verification_tokens: Box::new(PgVerificationTokenRepo::new(pool.clone())),
            mfa_enrollments: Box::new(PgMfaEnrollmentRepo::new(
                pool.clone(),
                Arc::clone(&identity_secrets),
            )),
            oauth_states: Box::new(PgOAuthStateRepo::new(pool.clone())),
            accounts: Box::new(PgAccountLifecycle::new(pool.clone())),
            oauth_login: Box::new(PgOAuthLoginFinalizer::new(pool)),
            identity_secrets,
        }
    }

    /// Live user lookup, versioned updates and authentication bookkeeping.
    #[must_use]
    pub fn users(&self) -> &dyn UserRepo {
        self.users.as_ref()
    }

    /// Browser session digests and their lifecycle.
    #[must_use]
    pub fn sessions(&self) -> &dyn SessionRepo {
        self.sessions.as_ref()
    }

    /// Principal-owned personal access tokens.
    #[must_use]
    pub fn pats(&self) -> &dyn PatRepo {
        self.pats.as_ref()
    }

    /// One-time verification and local MFA challenge tokens.
    #[must_use]
    pub fn verification_tokens(&self) -> &dyn VerificationTokenRepo {
        self.verification_tokens.as_ref()
    }

    /// Pending MFA candidates and their atomic installation.
    #[must_use]
    pub fn mfa_enrollments(&self) -> &dyn MfaEnrollmentRepo {
        self.mfa_enrollments.as_ref()
    }

    /// Bounded, provider-qualified OAuth state admission and consumption.
    #[must_use]
    pub fn oauth_states(&self) -> &dyn OAuthStateRepo {
        self.oauth_states.as_ref()
    }

    /// Atomic registration and account-token transitions.
    #[must_use]
    pub fn accounts(&self) -> &dyn AccountLifecycle {
        self.accounts.as_ref()
    }

    /// Atomic OAuth account/link/session or MFA-challenge finalization.
    #[must_use]
    pub fn oauth_login(&self) -> &dyn OAuthLoginFinalizer {
        self.oauth_login.as_ref()
    }

    /// The exact codec used by this set's MFA persistence owner.
    #[must_use]
    pub fn identity_secrets(&self) -> &IdentitySecretCodec {
        &self.identity_secrets
    }
}
