//! Plane-A account persistence — users, sessions, personal access tokens,
//! OAuth state and external identities, MFA enrollment, email verification
//! — kept outside the `nebula-storage-port` contract by design.
//!
//! Repository traits and row types live here; the PostgreSQL implementations
//! live in [`postgres`] (feature `postgres`).
//!
//! ## Conventions
//!
//! - Traits accept **raw byte slices** for IDs; callers encode their domain
//!   newtypes.
//! - Return types are the row structs of this module.
//! - All errors are [`crate::StorageError`].

pub mod identity_secret;
mod mfa_enrollment;
mod oauth_login;
/// PostgreSQL implementations of the account repositories.
#[cfg(feature = "postgres")]
pub mod postgres;
mod repos;
// Row structs are plain data containers whose fields mirror SQL columns.
#[expect(
    missing_docs,
    reason = "row structs mirror SQL columns; per-field docs add noise without value"
)]
mod rows;
pub mod session_token;

pub use mfa_enrollment::{MfaEnrollmentCandidate, MfaEnrollmentInstallOutcome, MfaEnrollmentRepo};
pub use oauth_login::{
    OAuthLoginFinalizeCommand, OAuthLoginFinalizeOutcome, OAuthLoginFinalized,
    OAuthLoginMfaChallengeDraft, OAuthLoginSessionDraft, OAuthLoginUserDraft,
};
pub use repos::{
    ExternalIdentityRepo, OAUTH_STATE_CAPACITY, OAuthStateAdmission, OAuthStateRepo, PatRepo,
    SessionRepo, UserRepo, VerificationTokenRepo,
};
pub use rows::{
    OAuthStateRow, PersonalAccessTokenRow, SessionDraft, SessionRow, UserRow, VerificationTokenRow,
};
