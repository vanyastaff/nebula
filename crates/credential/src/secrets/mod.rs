//! §12.5 primitives — AES-256-GCM, Argon2id KDF, PKCE, zeroizing secret wrappers.
//!
//! Canon-level secret-handling primitives. Every plaintext buffer here is
//! wrapped in `Zeroizing<T>` or a zeroize-on-drop newtype.
//!
//! # Canonical import paths
//!
//! This submodule is `pub` for escape hatches. Prefer flat root re-exports:
//! `use nebula_credential::{SecretString, CredentialGuard, encrypt, decrypt};`.
//!
//! The `serde_secret` submodule is `pub` and re-exported at the crate root
//! specifically so serde attribute paths
//! (`#[serde(with = "nebula_credential::serde_secret")]` and
//! `#[serde(with = "nebula_credential::serde_secret::option")]`) continue to
//! resolve.
//!
//! A companion `serde_base64` module (re-exported from `crypto`) provides
//! `#[serde(with = "…")]` helpers for binary ciphertext fields that should
//! round-trip as base64 strings rather than byte arrays. Import via
//! `use nebula_credential::secrets::serde_base64;` (no root re-export —
//! the module is reached through `secrets`).

mod crypto;
mod guard;
pub mod redacted;
mod scheme_guard;
mod secret_string;
pub mod serde_secret;

pub use crypto::{
    EncryptedData, EncryptionKey, decrypt, decrypt_with_aad, encrypt, encrypt_with_aad,
    encrypt_with_key_id, generate_code_challenge, generate_pkce_verifier, generate_random_state,
    serde_base64,
};
pub use guard::CredentialGuard;
pub use redacted::RedactedSecret;
#[allow(deprecated)]
pub use scheme_guard::OnCredentialRefresh;
pub use scheme_guard::{SchemeFactory, SchemeGuard};
pub use secret_string::SecretString;
