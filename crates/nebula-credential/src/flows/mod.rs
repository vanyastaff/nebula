//! Credential flow implementations

pub mod api_key;
pub mod basic_auth;
pub mod bearer_token;
pub mod oauth2;
pub mod password;

// Re-exports for convenience
pub use api_key::{ApiKeyCredential, ApiKeyFlow, ApiKeyInput, ApiKeyState};
pub use basic_auth::{BasicAuthCredential, BasicAuthFlow, BasicAuthInput, BasicAuthState};
pub use bearer_token::{
    BearerTokenCredential, BearerTokenFlow, BearerTokenInput, BearerTokenState,
};
pub use password::{PasswordCredential, PasswordFlow, PasswordInput, PasswordState};
