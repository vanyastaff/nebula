//! Credential provider trait for resource operations.
//!
//! Resource implementations can use
//! the [`CredentialProvider`] passed via [`Context`](crate::Context) to fetch
//! secrets at instance-creation time, keeping credentials fresh.

use std::fmt;

use crate::error::Error;

/// A string that redacts its contents in Debug and Display.
#[derive(Clone)]
pub struct SecureString {
    inner: String,
}

impl SecureString {
    /// Create a new secure string.
    pub fn new(value: impl Into<String>) -> Self {
        Self {
            inner: value.into(),
        }
    }

    /// Access the underlying value.
    pub fn expose(&self) -> &str {
        &self.inner
    }
}

impl fmt::Debug for SecureString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecureString(***)")
    }
}

impl fmt::Display for SecureString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("***")
    }
}

/// Provider trait for injecting credential resolution into resource operations.
///
/// Resource implementations typically call this via the
/// [`Context`](crate::Context) helper:
///
/// ```ignore
/// if let Some(creds) = ctx.credentials() {
///     let password = creds.get("db_password").await?;
///     // use `password.expose()` to access the underlying value
/// }
/// ```
///
/// Returns a boxed future so the trait is dyn-compatible and can be stored
/// as `Arc<dyn CredentialProvider>` inside [`Context`](crate::Context).
pub trait CredentialProvider: Send + Sync {
    /// Retrieve a credential value by key.
    fn get(
        &self,
        key: &str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<SecureString, Error>> + Send + '_>>;
}
