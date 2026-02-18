//! Built-in credential protocols for reuse across plugins.
//!
//! A [`CredentialProtocol`] is a static building block that defines:
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
//! [`CredentialProtocol`]: crate::traits::CredentialProtocol
//! [`CredentialState`]: crate::core::CredentialState

pub mod api_key;

pub use api_key::{ApiKeyProtocol, ApiKeyState};
