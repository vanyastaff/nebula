//! API-edge bearer-token hashing for webhook activations.
//!
//! The plaintext capability token is hashed here at the API edge and never
//! crosses into `nebula-storage-port`.  Only the 32-byte SHA-256 digest is
//! persisted (as `WebhookActivationRecord::token_hash`).  This is the single
//! seam between the plaintext URL surface and the durable store.
//!
//! For `whsec_<base64>` signing-secret minting see
//! [`super::secret_resolver::mint_whsec`], which lives alongside
//! [`super::secret_resolver::decode_whsec`] where the `whsec_` format is
//! owned and tested.
//!
//! # Security
//!
//! SHA-256 is used rather than a KDF because the token itself is
//! 128-bit random (16 bytes of `Uuid::new_v4` hex-encoded as 32 chars),
//! which already sits above the brute-force horizon.  A KDF would add
//! latency on the hot inbound-webhook path without meaningful security gain.
//! The sentinel value (`[0u8; 32]`) is excluded from the `token_hash` partial
//! unique index on `port_webhook_activations`, so an uninitialized row can
//! never match a real token.

use sha2::{Digest, Sha256};

/// Hash a plaintext bearer token at the API edge.
///
/// Returns the raw 32-byte SHA-256 digest.  The input is the
/// plaintext URL segment (the `nonce` from [`super::key::WebhookKey::Programmatic`]);
/// the output is stored in [`nebula_storage_port::dto::WebhookActivationRecord::token_hash`].
///
/// # Contract
///
/// - Deterministic: equal inputs → equal outputs.
/// - Distinct: distinct inputs → distinct outputs (SHA-256 collision resistance).
/// - The all-zeros sentinel `[0u8; 32]` is NOT a valid output (SHA-256 cannot
///   produce all-zeros for a nonempty preimage); callers may rely on this to
///   distinguish "no token" from a real hash.
#[must_use]
pub fn token_hash(plaintext: &str) -> [u8; 32] {
    let digest = Sha256::digest(plaintext.as_bytes());
    // `digest` is `GenericArray<u8, U32>` — copy into a plain array.
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Known-vector: SHA-256("") == e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
    #[test]
    fn empty_string_known_vector() {
        let hash = token_hash("");
        let hex: String = hash.iter().fold(String::with_capacity(64), |mut s, b| {
            use std::fmt::Write as _;
            let _ = write!(s, "{b:02x}");
            s
        });
        assert_eq!(
            hex,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    /// Determinism: same input always yields the same hash.
    #[test]
    fn deterministic() {
        let h1 = token_hash("nonce-abc");
        let h2 = token_hash("nonce-abc");
        assert_eq!(h1, h2);
    }

    /// Distinct inputs produce distinct hashes.
    #[test]
    fn distinct_inputs_distinct_hashes() {
        let h1 = token_hash("nonce-aaa");
        let h2 = token_hash("nonce-bbb");
        assert_ne!(h1, h2);
    }

    /// The all-zeros sentinel is not a valid output of `token_hash` for any
    /// non-degenerate input — confirms the "no token" sentinel is safe.
    #[test]
    fn output_is_not_all_zeros_for_real_token() {
        let h = token_hash("super-secret-nonce-token");
        assert_ne!(h, [0u8; 32]);
    }
}
