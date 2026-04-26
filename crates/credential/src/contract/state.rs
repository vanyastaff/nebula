//! Credential state trait for stored credential data.
//!
//! [`CredentialState`](CredentialState) represents what gets persisted
//! in encrypted storage. It may contain refresh internals
//! (`refresh_token`, `client_secret`) that are NOT exposed to resource
//! consumers -- those see only the [`AuthScheme`].
//!
//! [`AuthScheme`]: crate::AuthScheme

use serde::{Serialize, de::DeserializeOwned};
use zeroize::ZeroizeOnDrop;

/// Trait for credential state types stored in encrypted storage (v2).
///
/// The `project()` method on `Credential` extracts an [`AuthScheme`]
/// from this state for consumer use.
///
/// `ZeroizeOnDrop` is mandatory — credential state contains decrypted
/// secret material at runtime; deterministic plaintext drop is a
/// §12.5 invariant (§15.4 amendment, Tech Spec).
///
/// [`AuthScheme`]: crate::AuthScheme
pub trait CredentialState:
    Serialize + DeserializeOwned + Send + Sync + ZeroizeOnDrop + 'static
{
    /// Unique identifier for this state type (e.g., `"oauth2_state"`).
    const KIND: &'static str;
    /// Schema version for migration support.
    const VERSION: u32;

    /// When this credential expires, if applicable.
    fn expires_at(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        None
    }
}

/// Opt-in macro: make an `AuthScheme` also usable as `CredentialState`.
///
/// For static credentials where stored state = consumer-facing auth
/// (e.g., API key, bot token), the state and scheme are the same type.
///
/// # Examples
///
/// ```ignore
/// identity_state!(SecretToken, "secret_token", 1);
/// // Now SecretToken can be used as both AuthScheme and CredentialState
/// ```
#[macro_export]
macro_rules! identity_state {
    ($ty:ty, $kind:expr, $version:expr) => {
        impl $crate::CredentialState for $ty {
            const KIND: &'static str = $kind;
            const VERSION: u32 = $version;
        }
    };
}
