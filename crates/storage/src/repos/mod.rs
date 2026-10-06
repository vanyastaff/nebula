//! Repository traits for Plane-A account persistence — the surface that is
//! not part of the `nebula-storage-port` contract.
//!
//! - **Accounts and sign-in** — `UserRepo`, `SessionRepo`, `PatRepo`,
//!   `VerificationTokenRepo`, `ExternalIdentityRepo`, `OAuthStateRepo`,
//!   `MfaEnrollmentRepo`, and the atomic OAuth login finalizer types.
//! - **API idempotency cache** — `IdempotencyStoreRepo` /
//!   `InMemoryIdempotencyStoreRepo`, consumed by the API idempotency
//!   middleware.
//!
//! PostgreSQL implementations live in `crate::pg` (behind the `postgres`
//! feature).
//!
//! ## Conventions
//!
//! - Traits accept **raw byte slices** for IDs; callers encode their
//!   domain newtypes.
//! - Return types are row structs from [`crate::rows`].
//! - All errors funnel through [`crate::StorageError`].
mod idempotency;
mod mfa_enrollment;
mod oauth_login;
mod user;

pub use idempotency::{CachedRecord, IdempotencyStoreRepo, InMemoryIdempotencyStoreRepo};
pub use mfa_enrollment::{MfaEnrollmentCandidate, MfaEnrollmentInstallOutcome, MfaEnrollmentRepo};
pub use oauth_login::{
    OAuthLoginFinalizeCommand, OAuthLoginFinalizeOutcome, OAuthLoginFinalized,
    OAuthLoginMfaChallengeDraft, OAuthLoginSessionDraft, OAuthLoginUserDraft,
};
pub use user::{
    ExternalIdentityRepo, OAUTH_STATE_CAPACITY, OAuthStateAdmission, OAuthStateRepo, PatRepo,
    SessionRepo, UserRepo, VerificationTokenRepo,
};
