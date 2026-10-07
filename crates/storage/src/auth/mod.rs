//! Plane-A account persistence — users, sessions, personal access tokens,
//! OAuth state and external identities, MFA enrollment, email verification
//! — kept outside the `nebula-storage-port` contract by design.
//!
//! Repository traits and row types live here; backend implementations live in
//! `postgres` and `sqlite`, enabled by their corresponding Cargo features.
//! [`AuthPersistence`] binds the repository roles to one deployment pool and
//! identity codec. Its factories run no schema or key admission themselves.
//!
//! ## Conventions
//!
//! - Traits accept **raw byte slices** for IDs; callers encode their domain
//!   newtypes.
//! - Return types are the row structs of this module.
//! - All errors are [`crate::StorageError`].

mod account_lifecycle;
pub mod identity_secret;
mod mfa_enrollment;
mod oauth_login;
mod persistence;
/// PostgreSQL implementations of the account repositories.
#[cfg(feature = "postgres")]
pub mod postgres;
mod repos;
/// SQLite implementations of the account repositories.
#[cfg(feature = "sqlite")]
pub mod sqlite;
// Row structs are plain data containers whose fields mirror SQL columns.
#[expect(
    missing_docs,
    reason = "row structs mirror SQL columns; per-field docs add noise without value"
)]
mod rows;
pub mod session_token;

pub use account_lifecycle::{AccountLifecycle, AccountTokenOutcome, PasswordRegistration};
pub use mfa_enrollment::{MfaEnrollmentCandidate, MfaEnrollmentInstallOutcome, MfaEnrollmentRepo};
pub use oauth_login::{
    OAuthLoginFinalizeCommand, OAuthLoginFinalizeOutcome, OAuthLoginFinalized, OAuthLoginFinalizer,
    OAuthLoginMfaChallengeDraft, OAuthLoginSessionDraft, OAuthLoginUserDraft,
};
pub use persistence::AuthPersistence;
pub use repos::{
    ExternalIdentityRepo, OAUTH_STATE_CAPACITY, OAuthStateAdmission, OAuthStateRepo, PatRepo,
    SessionRepo, UserRepo, VerificationTokenRepo,
};
pub use rows::{
    OAuthStateRow, PersonalAccessTokenRow, SessionDraft, SessionRow, UserRow, VerificationTokenRow,
};
