//! Built-in credential protocols for reuse across plugins.
//!
//! A [`StaticProtocol`] is a static building block that defines:
//! - A fixed set of parameters (e.g. `server`, `token`)
//! - A [`CredentialState`] type for those fields
//! - A default `build_state` implementation
//!
//! Plugin authors extend a protocol via:
//! ```ignore
//! #[derive(Credential)]
//! #[credential(key = "github-api", name = "GitHub API", extends = ApiKeyProtocol)]
//! pub struct GithubApi {
//!     #[param(name = "User", required)]
//!     pub user: String,
//! }
//! ```
//!
//! [`StaticProtocol`]: crate::traits::StaticProtocol
//! [`CredentialState`]: crate::core::CredentialState

pub mod api_key;
pub mod basic_auth;
pub mod database;
pub mod header_auth;
pub mod oauth2;

pub use api_key::{ApiKeyProtocol, ApiKeyState};
pub use basic_auth::{BasicAuthProtocol, BasicAuthState};
pub use database::{DatabaseProtocol, DatabaseState};
pub use header_auth::{HeaderAuthProtocol, HeaderAuthState};
pub use oauth2::{AuthStyle, GrantType, OAuth2Config, OAuth2ConfigBuilder, OAuth2State};
