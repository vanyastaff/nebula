//! Credential state trait for stored credential data.
//!
//! [`CredentialState`] represents what gets persisted
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
/// credential secrecy invariant (§15.4 amendment, Tech Spec).
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

/// Compile-time identity of a credential state type's serde wire shape.
///
/// Implemented by `#[derive(StateWireFingerprint)]`; `SCHEMA_FINGERPRINT` is
/// an FNV-1a 64 hash over the wire-shape projection (field names as
/// authored, Option-ness, declaration order, type tokens as written — never
/// authored annotations, labels, descriptions, or values). See the derive
/// documentation for the exact projection contract.
///
/// The fingerprint rides in the persisted-state envelope (ADR-0107 Seam 2)
/// so a reader can fail closed before deserialization when the stored wire
/// shape is not the shape this build understands.
pub trait StateWireFingerprint: CredentialState {
    /// FNV-1a 64 over the serde wire-shape projection of this state type.
    const SCHEMA_FINGERPRINT: u64;
}

/// Opt-in macro: make an `AuthScheme` also usable as `CredentialState`.
///
/// For static credentials where stored state = consumer-facing auth
/// (e.g., API key, bot token), the state and scheme are the same type.
///
/// # Examples
///
/// ```
/// use nebula_credential::{CredentialState, identity_state};
/// use serde::{Deserialize, Serialize};
/// use zeroize::ZeroizeOnDrop;
///
/// // A small auth-scheme-like type whose stored state == consumer-facing
/// // material. `CredentialState` requires `Serialize + DeserializeOwned +
/// // ZeroizeOnDrop`, all derivable here.
/// #[derive(Serialize, Deserialize, ZeroizeOnDrop)]
/// struct ApiKey {
///     key: String,
/// }
///
/// // Now `ApiKey` is usable as a `CredentialState` (state == scheme).
/// identity_state!(ApiKey, "api_key_demo", 1);
///
/// assert_eq!(<ApiKey as CredentialState>::KIND, "api_key_demo");
/// assert_eq!(<ApiKey as CredentialState>::VERSION, 1);
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
