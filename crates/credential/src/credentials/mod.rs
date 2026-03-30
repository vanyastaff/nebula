//! Built-in credential type implementations.
//!
//! Each type implements [`Credential`](crate::credential_trait::Credential) using
//! the v2 unified trait. Static credentials (API key, basic auth, database) use
//! [`identity_state!`](crate::identity_state) so that `State = Scheme`.

pub mod api_key;
pub mod basic_auth;
pub mod database;
pub mod header_auth;
pub mod oauth2;
pub mod oauth2_flow;

pub use api_key::ApiKeyCredential;
pub use basic_auth::BasicAuthCredential;
pub use database::DatabaseCredential;
pub use header_auth::HeaderAuthCredential;
pub use oauth2::{OAuth2Credential, OAuth2Pending, OAuth2State};

// ── identity_state! invocations ─────────────────────────────────────────
//
// For static credentials, State = Scheme. These macro calls implement
// `CredentialStateV2` for each scheme type so they can be stored directly.

use crate::identity_state;
use crate::scheme::{BasicAuth, BearerToken, DatabaseAuth, HeaderAuth};

identity_state!(BearerToken, "bearer", 1);
identity_state!(BasicAuth, "basic_auth", 1);
identity_state!(DatabaseAuth, "database_auth", 1);
identity_state!(HeaderAuth, "header", 1);
