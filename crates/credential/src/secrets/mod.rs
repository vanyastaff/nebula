//! §12.5 primitives — AES-256-GCM, Argon2id KDF, PKCE, zeroizing secret wrappers.
//!
//! Canon-level secret-handling primitives. Every plaintext buffer here is
//! wrapped in `Zeroizing<T>` or a zeroize-on-drop newtype.
//!
//! # Canonical import paths
//!
//! This submodule is `pub` for escape hatches. Prefer flat root re-exports:
//! `use nebula_credential::{SecretString, CredentialGuard, encrypt_with_aad, decrypt};`.
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

// SEC-11 (security hardening 2026-04-27 Stage 1): bare `encrypt` was
// removed from the public surface (renamed to `pub(crate) encrypt_no_aad`,
// reachable only inside `nebula-credential`). Plugins and external callers
// must use `encrypt_with_aad` or `encrypt_with_key_id` — the AAD-mandatory
// public path. The `decrypt` partner stays public; callers can decrypt
// rotation-era envelopes that legitimately have the legacy shape.
pub use crypto::{
    EncryptedData, EncryptionKey, decrypt, decrypt_with_aad, encrypt_with_aad, encrypt_with_key_id,
    generate_code_challenge, generate_pkce_verifier, generate_random_state, serde_base64,
};
pub use guard::CredentialGuard;
pub use redacted::RedactedSecret;
pub use scheme_guard::{SchemeFactory, SchemeGuard};
pub use secret_string::SecretString;
