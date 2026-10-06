//! PostgreSQL implementations of the Plane-A account repositories, plus the
//! identity-secret rotation migrator.
//!
//! Each module implements one repository trait of [`crate::auth`]. Errors go
//! through [`crate::sql_error::storage_error_for`], so a unique violation is a
//! [`crate::StorageError::Duplicate`] naming the entity and constraint.
//!
//! Tests are gated behind `cfg(all(test, feature = "postgres"))` and are
//! skipped when `DATABASE_URL` is not set.

mod external_identity;
mod identity_secret;
mod mfa_enrollment;
mod oauth_login;
mod oauth_state;
mod pat;
mod session;
pub(crate) mod user;
mod verification_token;

pub use external_identity::PgExternalIdentityRepo;
pub use identity_secret::{
    IdentitySecretMigrationError, IdentitySecretMigrationReport, IdentitySecretRejectionReason,
    PgIdentitySecretMigrator,
};
pub use mfa_enrollment::PgMfaEnrollmentRepo;
pub use oauth_login::PgOAuthLoginFinalizer;
pub use oauth_state::PgOAuthStateRepo;
pub use pat::PgPatRepo;
pub use session::PgSessionRepo;
pub use user::PgUserRepo;
pub use verification_token::PgVerificationTokenRepo;
