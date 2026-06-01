//! credential secrecy primitives — PKCE/state helpers + zeroizing secret wrappers.
//!
//! AES-256-GCM + Argon2id KDF moved to `nebula-crypto` (ADR-0088); import
//! `EncryptedData` / `EncryptionKey` / `encrypt_with_aad` / `decrypt` / … from
//! there. Every plaintext buffer here is wrapped in `Zeroizing<T>` or a
//! zeroize-on-drop newtype.
//!
//! # Canonical import paths
//!
//! This submodule is `pub` for escape hatches. Prefer flat root re-exports:
//! `use nebula_credential::{SecretString, CredentialGuard};`.
//!
//! The `serde_secret` submodule is `pub` and re-exported at the crate root
//! specifically so serde attribute paths
//! (`#[serde(with = "nebula_credential::serde_secret")]` and
//! `#[serde(with = "nebula_credential::serde_secret::option")]`) continue to
//! resolve.
//!
//! A companion `serde_base64` module (re-exported from `crypto`) provides
//! `#[serde(with = "…")]` helpers for binary fields that should round-trip as
//! base64 strings. Import via `use nebula_credential::secrets::serde_base64;`.

mod crypto;
mod guard;
pub mod redacted;
mod scheme_guard;
mod secret_string;
pub mod serde_secret;

pub use crypto::{
    generate_code_challenge, generate_pkce_verifier, generate_random_state, serde_base64,
};
pub use guard::CredentialGuard;
pub use redacted::RedactedSecret;
pub use scheme_guard::{SchemeFactory, SchemeGuard};
pub use secret_string::{
    ExposeSecret, ExposeSecretMut, SecretBox, SecretString, secret_from_string,
};
